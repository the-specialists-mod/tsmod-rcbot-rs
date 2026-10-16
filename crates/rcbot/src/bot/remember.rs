//! Where the bot last saw things (`bot.h` `CRememberPosition`, `CRememberPositions`).
//!
//! Port changes (design §4): positions are stamped with the time and forgotten after
//! [`REMEMBER_FOR`] seconds or when their player dies, at most [`MAX_REMEMBER_POSITIONS`] are kept,
//! and positions without an entity are the same position only when they are near each other
//! (upstream: they all compared equal, §5.53).

use super::EntRef;
use crate::Vector;

/// `MAX_REMEMBER_POSITIONS`.
pub const MAX_REMEMBER_POSITIONS: usize = 8;
/// `BOT_REMEMBER_LOST_ENEMY`.
pub const REMEMBER_LOST_ENEMY: i32 = 1 << 1;
/// Seconds a position is remembered (upstream: until pushed out by newer ones).
pub const REMEMBER_FOR: f32 = 15.0;
/// Positions without an entity nearer than this are the same position.
const SAME_PLACE: f32 = 64.0;

/// `CRememberPosition`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RememberPosition {
    /// `m_vOrigin`.
    pub origin: Vector,
    /// `m_pEntity`.
    pub entity: Option<EntRef>,
    /// `m_iFlags`.
    pub flags: i32,
    /// `m_vVisibleOrigin`: where the bot stood.
    pub visible_origin: Vector,
    /// When it was seen there (port addition).
    pub time: f32,
    /// The bot went after it already (port addition).
    pub hunted: bool,
}

/// `CRememberPositions`: oldest first.
#[derive(Clone, Debug, Default)]
pub struct RememberPositions {
    positions: Vec<RememberPosition>,
    /// The time new positions are stamped with ([`RememberPositions::tick`]).
    now: f32,
}

impl RememberPositions {
    /// `addPosition`: an earlier position of the same entity (or, without one, the same place) is
    /// replaced, and the oldest goes beyond [`MAX_REMEMBER_POSITIONS`] (upstream kept 9: it
    /// dropped one only once there were more than 8).
    pub fn add(&mut self, origin: Vector, entity: Option<EntRef>, flags: i32, visible_origin: Vector) {
        let p = RememberPosition { origin, entity, flags, visible_origin, time: self.now, hunted: false };
        let same = |x: &RememberPosition| match (x.entity, entity) {
            (None, None) => (x.origin - origin).length() < SAME_PLACE,
            (a, b) => a == b,
        };
        if let Some(i) = self.positions.iter().position(same) {
            self.positions.remove(i);
        } else if self.positions.len() >= MAX_REMEMBER_POSITIONS {
            self.positions.remove(0);
        }
        self.positions.push(p);
    }

    /// The clock moves on to `now`: positions older than [`REMEMBER_FOR`] are forgotten.
    pub fn tick(&mut self, now: f32) {
        if now < self.now {
            // A new map.
            self.positions.clear();
        }
        self.now = now;
        self.positions.retain(|p| now - p.time <= REMEMBER_FOR);
    }

    /// The bot goes after the position at `origin`.
    pub fn hunted(&mut self, origin: Vector) {
        for p in self.positions.iter_mut().filter(|p| p.origin == origin) {
            p.hunted = true;
        }
    }

    /// Forgets where `entity` was (it died: it won't be there).
    pub fn forget(&mut self, entity: EntRef) {
        self.positions.retain(|p| p.entity != Some(entity));
    }

    /// The positions, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &RememberPosition> {
        self.positions.iter()
    }

    /// `gotPosition`.
    pub fn any(&self) -> bool {
        !self.positions.is_empty()
    }

    /// `getNewest`.
    pub fn newest(&self) -> Option<&RememberPosition> {
        self.positions.last()
    }

    /// `positionNearest`: the least `|p - origin| + |p - from|`.
    pub fn nearest(&self, origin: Vector, from: Vector) -> Option<&RememberPosition> {
        let mut best: Option<(&RememberPosition, f32)> = None;
        for p in &self.positions {
            let d = (p.origin - origin).length() + (p.origin - from).length();
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((p, d));
            }
        }
        best.map(|(p, _)| p)
    }

    /// Forgets everything (a new life).
    pub fn clear(&mut self) {
        self.positions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_by_entity_and_keeps_eight() {
        let mut r = RememberPositions::default();
        let e = |i| Some(EntRef { index: i, serial: 0 });
        for i in 0..12 {
            r.add(Vector::new(i as f32, 0.0, 0.0), e(i), 0, Vector::ZERO);
        }
        assert_eq!(r.positions.len(), 8);
        r.add(Vector::new(100.0, 0.0, 0.0), e(11), 0, Vector::ZERO);
        assert_eq!(r.positions.len(), 8);
        assert_eq!(r.newest().unwrap().origin.x, 100.0);
        assert_eq!(r.nearest(Vector::new(5.0, 0.0, 0.0), Vector::new(5.0, 0.0, 0.0)).unwrap().origin.x, 5.0);
    }

    #[test]
    fn positions_expire_and_die_with_their_player() {
        let mut r = RememberPositions::default();
        let e = |i| Some(EntRef { index: i, serial: 0 });
        r.tick(10.0);
        r.add(Vector::ZERO, e(1), 0, Vector::ZERO);
        r.tick(20.0);
        r.add(Vector::new(500.0, 0.0, 0.0), e(2), 0, Vector::ZERO);
        // Entity-less positions are one only when near each other (upstream: all one).
        r.add(Vector::new(0.0, 900.0, 0.0), None, 0, Vector::ZERO);
        r.add(Vector::new(0.0, 0.0, 900.0), None, 0, Vector::ZERO);
        r.add(Vector::new(0.0, 10.0, 900.0), None, 0, Vector::ZERO);
        assert_eq!(r.iter().count(), 4);
        r.tick(26.0);
        assert!(r.iter().all(|p| p.entity != e(1)), "15 s old");
        r.forget(EntRef { index: 2, serial: 0 });
        assert_eq!(r.iter().count(), 2);
        assert!(r.iter().all(|p| p.time == 20.0));
    }
}
