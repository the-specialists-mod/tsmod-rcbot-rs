//! What the bot knows of the map's items (port addition, design §4).
//!
//! Upstream's TS bot only went for a weapon or power-up it saw in front of it (the 60° view cone of
//! the vision pass), whether it could take it or not, and pressed use on it for 10 s, every time.
//! An experienced player knows where the items spawn, which of them it can carry, and when one it
//! saw taken comes back. [`ItemMemory`] keeps that per bot.

use super::{BotWorld, EF_NODRAW, EntRef, Entity, ItemKind};
use crate::Vector;

/// Seconds between two reads of the map's item list.
const SCAN_INTERVAL: f32 = 5.0;
/// What the bot assumes a taken weapon needs to come back, until it has seen one come back.
const DEFAULT_WEAPON_RESPAWN: f32 = 20.0;
/// The same for power-ups (TS's default `pwupduration` gives 60 s).
const DEFAULT_POWERUP_RESPAWN: f32 = 60.0;
/// Seconds an item the bot couldn't take is left alone.
pub(crate) const UNTAKEABLE_FOR: f32 = 30.0;
/// An item still there this long after the bot touched it is one the game won't give it.
const TAKE_TIME: f32 = 2.0;
/// The bot sees whether an item is there this far away (with a clear line).
const SEE_ITEM_RANGE: f32 = 1200.0;

/// An item the bot knows of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KnownItem {
    /// The entity.
    pub handle: EntRef,
    /// Where it lies.
    pub origin: Vector,
    /// A ground weapon or a power-up.
    pub kind: ItemKind,
    /// Its TS weapon id, or 0.
    pub weapon: i32,
    /// Its power-up bits, or 0.
    pub powerup: i32,
    /// When the bot thinks it is there again (0: it is there).
    pub available_at: f32,
    /// When the bot first saw it gone (0: not gone).
    pub gone_seen: f32,
    /// How long the bot thinks it takes to come back.
    pub respawn: f32,
    /// Left alone until then: the bot couldn't take it.
    pub ignore_until: f32,
    /// When the bot last touched it to take it (0: not since it last saw it).
    pub touched_at: f32,
}

impl KnownItem {
    /// The bot thinks it is there `at` (and it isn't left alone).
    pub fn there_at(&self, at: f32, now: f32) -> bool {
        self.available_at <= at && self.ignore_until <= now
    }
}

/// The items of the map the bot knows of.
#[derive(Clone, Debug, Default)]
pub(crate) struct ItemMemory {
    items: Vec<KnownItem>,
    next_scan: f32,
    next_look: f32,
}

impl ItemMemory {
    /// The items, in the order they were learned.
    pub fn iter(&self) -> impl Iterator<Item = &KnownItem> {
        self.items.iter()
    }

    /// The item behind a handle.
    pub fn get(&self, h: EntRef) -> Option<&KnownItem> {
        self.items.iter().find(|i| i.handle == h)
    }

    /// The bot couldn't take `h`: leave it alone for a while.
    pub fn untakeable(&mut self, h: EntRef, now: f32) {
        if let Some(i) = self.items.iter_mut().find(|i| i.handle == h) {
            i.ignore_until = now + UNTAKEABLE_FOR;
        }
    }

    /// Whether `h` is left alone at `now`.
    pub fn ignored(&self, h: EntRef, now: f32) -> bool {
        self.get(h).is_some_and(|i| i.ignore_until > now)
    }

    /// Learns the map's items (every few seconds; items removed from the map are forgotten).
    pub fn scan(&mut self, w: &dyn BotWorld, now: f32) {
        if self.next_scan > now {
            return;
        }
        self.next_scan = now + SCAN_INTERVAL;
        let live: Vec<Entity> = w.items().into_iter().filter_map(|i| w.entity(i)).collect();
        self.items.retain(|k| {
            live.iter().any(|e| e.handle() == k.handle) || w.entity(k.handle.index).is_some_and(|e| e.serial == k.handle.serial)
        });
        for e in &live {
            self.learn(e);
        }
    }

    /// Learns an item the bot sees or was told of (not one it knows already).
    pub fn learn(&mut self, e: &Entity) {
        if self.get(e.handle()).is_some() {
            return;
        }
        let Some(info) = e.item.filter(|i| matches!(i.kind, ItemKind::GroundWeapon | ItemKind::Powerup)) else { return };
        let respawn = if info.kind == ItemKind::Powerup { DEFAULT_POWERUP_RESPAWN } else { DEFAULT_WEAPON_RESPAWN };
        self.items.push(KnownItem {
            handle: e.handle(),
            origin: e.entity_origin(),
            kind: info.kind,
            weapon: info.weapon,
            powerup: info.powerup,
            // Not seen yet: assume it is there (the map starts with every item).
            available_at: 0.0,
            gone_seen: 0.0,
            respawn,
            ignore_until: 0.0,
            touched_at: 0.0,
        });
    }

    /// Looks at the items the bot can see from `eye` (in front of it, `in_view`, with a clear
    /// line): taken ones are expected back after their respawn time, and a time between seeing
    /// one gone and seeing it back teaches the respawn time.
    pub fn look(&mut self, w: &dyn BotWorld, eye: Vector, me: u32, in_view: impl Fn(Vector) -> bool, now: f32) {
        if self.next_look > now {
            return;
        }
        self.next_look = now + 0.5;
        for k in &mut self.items {
            if (k.origin - eye).length() > SEE_ITEM_RANGE || !in_view(k.origin) {
                continue;
            }
            let tr = w.trace_line(eye, k.origin, crate::nav::Monsters::Ignore, crate::nav::Glass::Ignore, Some(me));
            if tr.fraction < 1.0 && tr.hit != Some(k.handle.index) {
                continue;
            }
            let there = w
                .entity(k.handle.index)
                .filter(|e| e.serial == k.handle.serial)
                .is_some_and(|e| e.effects & EF_NODRAW == 0 && e.item.is_none_or(|i| i.available));
            Self::seen(k, there, now);
        }
    }

    /// What the bot saw of `k` at `now`.
    fn seen(k: &mut KnownItem, there: bool, now: f32) {
        if there && k.touched_at > 0.0 && now - k.touched_at > TAKE_TIME {
            // It touched it and pressed use, and the item is still there: the game won't give it.
            k.touched_at = 0.0;
            k.ignore_until = now + UNTAKEABLE_FOR;
        } else if !there {
            k.touched_at = 0.0;
        }
        if there {
            if k.gone_seen > 0.0 {
                let took = now - k.gone_seen;
                if took > 1.0 && took < 180.0 {
                    k.respawn = (k.respawn + took) * 0.5;
                }
            }
            k.available_at = 0.0;
            k.gone_seen = 0.0;
        } else if k.gone_seen == 0.0 {
            k.gone_seen = now;
            k.available_at = now + k.respawn;
        } else if k.available_at <= now {
            // Still not back: give it a while longer.
            k.available_at = now + k.respawn * 0.5;
        }
    }

    /// The bot touched `h` to take it.
    pub fn touched(&mut self, h: EntRef, now: f32) {
        if let Some(k) = self.items.iter_mut().find(|i| i.handle == h) {
            k.touched_at = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> KnownItem {
        KnownItem {
            handle: EntRef { index: 9, serial: 1 },
            origin: Vector::ZERO,
            kind: ItemKind::Powerup,
            weapon: 0,
            powerup: 64,
            available_at: 0.0,
            gone_seen: 0.0,
            respawn: 60.0,
            ignore_until: 0.0,
            touched_at: 0.0,
        }
    }

    #[test]
    fn respawn_times_are_learned() {
        let mut k = item();
        ItemMemory::seen(&mut k, false, 10.0);
        assert_eq!((k.gone_seen, k.available_at), (10.0, 70.0));
        assert!(!k.there_at(30.0, 30.0) && k.there_at(70.0, 30.0));
        // Back after 20 s: the guess halves the difference.
        ItemMemory::seen(&mut k, true, 30.0);
        assert_eq!((k.available_at, k.respawn), (0.0, 40.0));
        // Not back when expected: a while longer.
        ItemMemory::seen(&mut k, false, 100.0);
        ItemMemory::seen(&mut k, false, 141.0);
        assert_eq!(k.available_at, 161.0);
    }

    #[test]
    fn an_item_still_there_after_a_touch_is_left_alone() {
        let mut k = item();
        k.touched_at = 10.0;
        ItemMemory::seen(&mut k, true, 11.0);
        assert_eq!(k.ignore_until, 0.0);
        ItemMemory::seen(&mut k, true, 12.5);
        assert_eq!(k.ignore_until, 12.5 + UNTAKEABLE_FOR);
        // Taken: not a failure.
        let mut k = item();
        k.touched_at = 10.0;
        ItemMemory::seen(&mut k, false, 13.0);
        assert_eq!((k.ignore_until, k.touched_at), (0.0, 0.0));
    }

    #[test]
    fn untakeable_items_are_left_alone() {
        let mut m = ItemMemory::default();
        m.items.push(item());
        let h = m.items[0].handle;
        m.untakeable(h, 5.0);
        assert!(m.ignored(h, 34.0) && !m.ignored(h, 36.0));
        assert!(!m.get(h).unwrap().there_at(10.0, 10.0));
    }
}
