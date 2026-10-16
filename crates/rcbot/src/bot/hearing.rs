//! What the bot heard lately (port addition, design §4).
//!
//! Upstream turned an enemy's footsteps into a `LISTEN_TO_SOUND` task without an entity, which
//! failed at once (§5.58) after `AddPriorityTask` had made every later task forget its path, and
//! heard gunfire only by polling for `IN_ATTACK` within 750 units, then went to the shooter with a
//! "find the path and finish" task that ended as soon as the path was found. The port keeps the
//! sounds for a few seconds: where, when, how loud and from whom; the goal chooser investigates
//! them ([`super::goals`]).

use super::EntRef;
use crate::Vector;

/// Seconds a sound is remembered.
pub(crate) const HEARD_FOR: f32 = 8.0;
/// Sounds this near each other (or from the same player) are one.
const SAME_SOUND: f32 = 200.0;
/// Sounds kept.
const MAX_SOUNDS: usize = 8;
/// How loud footsteps are ([`Heard::loud`]).
pub(crate) const LOUD_STEPS: f32 = 1.0;
/// How loud gunfire is.
pub(crate) const LOUD_GUNFIRE: f32 = 2.0;
/// How far unsilenced gunfire is heard (upstream: 750, from any player firing).
pub(crate) const GUNFIRE_RANGE: f32 = 1500.0;
/// How far silenced gunfire is heard.
pub(crate) const SILENCED_RANGE: f32 = 350.0;
/// `TSA_SILENCER`: the attachment bit of a silencer.
pub(crate) const TSA_SILENCER: i32 = 1;

/// A sound the bot heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Heard {
    /// Where.
    pub origin: Vector,
    /// When (the last time, for a sound heard again).
    pub time: f32,
    /// How loud ([`LOUD_STEPS`], [`LOUD_GUNFIRE`]).
    pub loud: f32,
    /// Who made it.
    pub source: Option<EntRef>,
    /// The bot went to see.
    pub investigated: bool,
}

/// The sounds the bot heard in the last [`HEARD_FOR`] seconds.
#[derive(Clone, Debug, Default)]
pub(crate) struct Hearing {
    sounds: Vec<Heard>,
}

impl Hearing {
    /// Hears a sound: the same player's or one near an earlier one updates that one.
    pub fn add(&mut self, origin: Vector, now: f32, loud: f32, source: Option<EntRef>) {
        let same = |h: &Heard| (source.is_some() && h.source == source) || (h.origin - origin).length() < SAME_SOUND;
        if let Some(h) = self.sounds.iter_mut().find(|h| same(h)) {
            // Somewhere new: worth a look again.
            if (h.origin - origin).length() >= SAME_SOUND {
                h.investigated = false;
            }
            h.origin = origin;
            h.time = now;
            h.loud = h.loud.max(loud);
            return;
        }
        if self.sounds.len() >= MAX_SOUNDS {
            self.sounds.remove(0);
        }
        self.sounds.push(Heard { origin, time: now, loud, source, investigated: false });
    }

    /// Forgets sounds older than [`HEARD_FOR`].
    pub fn tick(&mut self, now: f32) {
        self.sounds.retain(|h| h.time <= now && now - h.time <= HEARD_FOR);
    }

    /// The sounds, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Heard> {
        self.sounds.iter()
    }

    /// The bot goes to see the sound at `origin`.
    pub fn investigate(&mut self, origin: Vector) {
        for h in self.sounds.iter_mut().filter(|h| (h.origin - origin).length() < SAME_SOUND) {
            h.investigated = true;
        }
    }

    /// The newest sound not investigated yet.
    pub fn newest_new(&self) -> Option<&Heard> {
        self.sounds.iter().filter(|h| !h.investigated).max_by(|a, b| a.time.total_cmp(&b.time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_merge_and_expire() {
        let mut h = Hearing::default();
        let p = Some(EntRef { index: 2, serial: 1 });
        h.add(Vector::ZERO, 1.0, LOUD_STEPS, p);
        h.add(Vector::new(50.0, 0.0, 0.0), 2.0, LOUD_GUNFIRE, p);
        h.add(Vector::new(1000.0, 0.0, 0.0), 3.0, LOUD_STEPS, None);
        assert_eq!(h.iter().count(), 2);
        let first = *h.iter().next().unwrap();
        assert_eq!((first.time, first.loud), (2.0, LOUD_GUNFIRE));
        h.investigate(Vector::new(1000.0, 0.0, 0.0));
        assert_eq!(h.newest_new().unwrap().origin.x, 50.0);
        // The same player somewhere else: new again.
        h.investigate(Vector::new(50.0, 0.0, 0.0));
        h.add(Vector::new(600.0, 0.0, 0.0), 4.0, LOUD_STEPS, p);
        assert_eq!(h.newest_new().unwrap().origin.x, 600.0);
        h.tick(4.0 + HEARD_FOR + 0.1);
        assert_eq!(h.iter().count(), 0);
    }
}
