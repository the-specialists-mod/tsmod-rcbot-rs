//! Where the bot last saw things (`bot.h` `CRememberPosition`, `CRememberPositions`).

use super::EntRef;
use crate::Vector;

/// `MAX_REMEMBER_POSITIONS`.
pub const MAX_REMEMBER_POSITIONS: usize = 8;
/// `BOT_REMEMBER_LOST_ENEMY`.
pub const REMEMBER_LOST_ENEMY: i32 = 1 << 1;

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
}

/// `CRememberPositions`: oldest first.
#[derive(Clone, Debug, Default)]
pub struct RememberPositions {
    positions: Vec<RememberPosition>,
}

impl RememberPositions {
    /// `addPosition`. An earlier position of the same entity is replaced (positions compare by
    /// entity only, so two entity-less positions count as the same). Upstream drops the oldest
    /// only once there are *more* than [`MAX_REMEMBER_POSITIONS`], so it keeps up to 9. Kept.
    pub fn add(&mut self, origin: Vector, entity: Option<EntRef>, flags: i32, visible_origin: Vector) {
        let p = RememberPosition { origin, entity, flags, visible_origin };
        if let Some(i) = self.positions.iter().position(|x| x.entity == entity) {
            self.positions.remove(i);
        } else if self.positions.len() > MAX_REMEMBER_POSITIONS {
            self.positions.remove(0);
        }
        self.positions.push(p);
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
    fn replaces_by_entity_and_keeps_nine() {
        let mut r = RememberPositions::default();
        let e = |i| Some(EntRef { index: i, serial: 0 });
        for i in 0..12 {
            r.add(Vector::new(i as f32, 0.0, 0.0), e(i), 0, Vector::ZERO);
        }
        assert_eq!(r.positions.len(), 9);
        r.add(Vector::new(100.0, 0.0, 0.0), e(11), 0, Vector::ZERO);
        assert_eq!(r.positions.len(), 9);
        assert_eq!(r.newest().unwrap().origin.x, 100.0);
        assert_eq!(r.nearest(Vector::new(5.0, 0.0, 0.0), Vector::new(5.0, 0.0, 0.0)).unwrap().origin.x, 5.0);
    }
}
