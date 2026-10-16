//! Where players die, kill from and get hurt on each map (a port addition, design §4): per
//! waypoint and team, from the host's damage and kill events, kept across maps in
//! `waypoints/ts/<map>.rhm` beside the map's `.rcw`.
//!
//! Each event counts at the waypoint nearest the player it concerns: a death at the victim's, a
//! kill at the killer's, damage taken at the victim's. Everything fades by [`MAP_DECAY`] each
//! time the map loads, so a map's flow can change. [`Heatmap::danger`] (also the A* hook,
//! [`DangerSource`]) and [`Heatmap::ambush_value`] squash the counts to 0..1.

use crate::nav::{DangerSource, MAX_WAYPOINTS};

/// Teams counted: the rules team, 0 (none: deathmatch) to 4.
pub const HEAT_TEAMS: usize = 5;
/// What the counts keep each time the map loads.
pub const MAP_DECAY: f32 = 0.9;
/// Damage counts per this much (a death is 1).
pub const DAMAGE_UNIT: f32 = 100.0;
/// How much damage taken weighs against deaths in [`Heatmap::danger`].
const DAMAGE_WEIGHT: f32 = 0.3;
/// The count at which danger and ambush value are 0.5.
const HALF_COUNT: f32 = 2.0;
/// Units a fully dangerous cover waypoint counts as farther away ([`Heatmap::cover_penalty`]).
pub const COVER_PENALTY: f32 = 256.0;

const MAGIC: &[u8; 4] = b"RCHM";
const VERSION: u32 = 1;

/// `"<map>.rhm"`, in `waypoints/ts/`.
pub fn heatmap_file_name(map: &str) -> String {
    format!("{map}.rhm")
}

/// The slot of RCBot's team (`GetTeam()`, -1 none): rules team `team + 1`.
fn slot_of_bot_team(team: i32) -> usize {
    (team + 1).clamp(0, HEAT_TEAMS as i32 - 1) as usize
}

/// The slot of a rules team (`PlayerState::team`).
fn slot_of_rules_team(team: i32) -> usize {
    team.clamp(0, HEAT_TEAMS as i32 - 1) as usize
}

/// One map's counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heatmap {
    map: String,
    count: usize,
    deaths: [Vec<f32>; HEAT_TEAMS],
    kills: [Vec<f32>; HEAT_TEAMS],
    damage: [Vec<f32>; HEAT_TEAMS],
}

impl Heatmap {
    /// Empty counts for `map`'s `count` waypoints (at most 1024).
    pub fn new(map: &str, count: usize) -> Heatmap {
        let count = count.min(MAX_WAYPOINTS);
        let zeros = || std::array::from_fn(|_| vec![0.0; count]);
        Heatmap { map: map.to_owned(), count, deaths: zeros(), kills: zeros(), damage: zeros() }
    }

    /// The map.
    pub fn map_name(&self) -> &str {
        &self.map
    }

    /// The waypoints counted.
    pub fn len(&self) -> usize {
        self.count
    }

    /// No waypoints.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// A death at `waypoint` of a player of rules team `team`.
    pub fn add_death(&mut self, waypoint: usize, team: i32) {
        if let Some(v) = self.deaths[slot_of_rules_team(team)].get_mut(waypoint) {
            *v += 1.0;
        }
    }

    /// A kill made from `waypoint` by a player of rules team `team`.
    pub fn add_kill(&mut self, waypoint: usize, team: i32) {
        if let Some(v) = self.kills[slot_of_rules_team(team)].get_mut(waypoint) {
            *v += 1.0;
        }
    }

    /// `amount` damage taken at `waypoint` by a player of rules team `team`.
    pub fn add_damage(&mut self, waypoint: usize, team: i32, amount: f32) {
        if let Some(v) = self.damage[slot_of_rules_team(team)].get_mut(waypoint) {
            *v += amount.max(0.0) / DAMAGE_UNIT;
        }
    }

    /// Deaths at `waypoint` of rules team `team`.
    pub fn deaths_at(&self, waypoint: usize, team: i32) -> f32 {
        self.deaths[slot_of_rules_team(team)].get(waypoint).copied().unwrap_or(0.0)
    }

    /// Kills from `waypoint` by rules team `team`.
    pub fn kills_from(&self, waypoint: usize, team: i32) -> f32 {
        self.kills[slot_of_rules_team(team)].get(waypoint).copied().unwrap_or(0.0)
    }

    /// Damage (per 100) taken at `waypoint` by rules team `team`.
    pub fn damage_taken_at(&self, waypoint: usize, team: i32) -> f32 {
        self.damage[slot_of_rules_team(team)].get(waypoint).copied().unwrap_or(0.0)
    }

    /// How dangerous `waypoint` is for RCBot team `team` (`GetTeam()`, -1 none): its team's
    /// deaths there plus 0.3 per 100 damage taken, squashed to 0..1 (0.5 at 2).
    pub fn danger(&self, waypoint: usize, team: i32) -> f32 {
        let s = slot_of_bot_team(team);
        let x = self.deaths[s].get(waypoint).copied().unwrap_or(0.0) + DAMAGE_WEIGHT * self.damage[s].get(waypoint).copied().unwrap_or(0.0);
        x / (x + HALF_COUNT)
    }

    /// How good `waypoint` has been to kill from, any team: 0..1 (0.5 at 2 kills).
    pub fn ambush_value(&self, waypoint: usize) -> f32 {
        let k: f32 = self.kills.iter().filter_map(|t| t.get(waypoint)).sum();
        k / (k + HALF_COUNT)
    }

    /// Units to add to a cover waypoint's distance for RCBot team `team`: a waypoint where the
    /// team keeps dying counts as up to [`COVER_PENALTY`] farther (a tie-breaker toward safer
    /// cover).
    pub fn cover_penalty(&self, waypoint: usize, team: i32) -> f32 {
        self.danger(waypoint, team) * COVER_PENALTY
    }

    /// Every count times `factor`.
    pub fn decay(&mut self, factor: f32) {
        for v in self.deaths.iter_mut().chain(self.kills.iter_mut()).chain(self.damage.iter_mut()).flatten() {
            *v *= factor;
        }
    }

    /// The `.rhm` file: `"RCHM"`, `u32` version 1, `u16` map name length and the name, `u32`
    /// waypoints, `u32` teams (5), then per team the deaths, kills and damage as `f32`s, one
    /// per waypoint; little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.map.len() + self.count * HEAT_TEAMS * 12);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        let name = &self.map.as_bytes()[..self.map.len().min(u16::MAX as usize)];
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&(self.count as u32).to_le_bytes());
        out.extend_from_slice(&(HEAT_TEAMS as u32).to_le_bytes());
        for t in 0..HEAT_TEAMS {
            for v in self.deaths[t].iter().chain(&self.kills[t]).chain(&self.damage[t]) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    /// Reads [`to_bytes`](Self::to_bytes)'s file for `map` with `count` waypoints: `None` for
    /// another map, another waypoint count (the waypoints changed), another version, a damaged
    /// or truncated file, or counts that aren't finite and non-negative.
    pub fn from_bytes(bytes: &[u8], map: &str, count: usize) -> Option<Heatmap> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = bytes.get(at..at + n)?;
            at += n;
            Some(s)
        };
        if take(4)? != MAGIC {
            return None;
        }
        let u32_le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        if u32_le(take(4)?) != VERSION {
            return None;
        }
        let len = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        if take(len)? != map.as_bytes() {
            return None;
        }
        let n = u32_le(take(4)?) as usize;
        if n != count || n > MAX_WAYPOINTS || u32_le(take(4)?) as usize != HEAT_TEAMS {
            return None;
        }
        let mut h = Heatmap::new(map, n);
        for t in 0..HEAT_TEAMS {
            for v in h.deaths[t].iter_mut().chain(h.kills[t].iter_mut()).chain(h.damage[t].iter_mut()) {
                let x = f32::from_le_bytes(take(4)?.try_into().ok()?);
                if !x.is_finite() || x < 0.0 {
                    return None;
                }
                *v = x;
            }
        }
        (at == bytes.len()).then_some(h)
    }
}

impl DangerSource for Heatmap {
    fn danger(&self, waypoint: usize, team: i32) -> f32 {
        Heatmap::danger(self, waypoint, team)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_by_waypoint_and_team() {
        let mut h = Heatmap::new("ts_x", 4);
        h.add_death(1, 2);
        h.add_death(1, 2);
        h.add_kill(3, 1);
        h.add_damage(1, 2, 50.0);
        h.add_death(9, 2);
        assert_eq!((h.deaths_at(1, 2), h.kills_from(3, 1), h.damage_taken_at(1, 2)), (2.0, 1.0, 0.5));
        // RCBot team 1 is rules team 2.
        let d = h.danger(1, 1);
        assert!((d - 2.15 / 4.15).abs() < 1e-6, "{d}");
        assert_eq!(h.danger(1, 0), 0.0);
        assert_eq!(h.danger(1, -1), 0.0);
        assert!((h.ambush_value(3) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(h.cover_penalty(0, 1), 0.0);
        assert!(h.cover_penalty(1, 1) > 100.0);
        h.decay(MAP_DECAY);
        assert!((h.deaths_at(1, 2) - 1.8).abs() < 1e-6);
    }

    #[test]
    fn file_round_trip_and_rejects() {
        let mut h = Heatmap::new("ts_x", 3);
        h.add_death(0, 0);
        h.add_kill(2, 4);
        h.add_damage(1, 3, 77.0);
        let b = h.to_bytes();
        assert_eq!(&b[..4], b"RCHM");
        assert_eq!(b.len(), 4 + 4 + 2 + 4 + 4 + 4 + 3 * HEAT_TEAMS * 12);
        assert_eq!(Heatmap::from_bytes(&b, "ts_x", 3), Some(h.clone()));
        assert!(Heatmap::from_bytes(&b, "ts_y", 3).is_none());
        assert!(Heatmap::from_bytes(&b, "ts_x", 4).is_none());
        assert!(Heatmap::from_bytes(&b[..b.len() - 1], "ts_x", 3).is_none());
        let mut extra = b.clone();
        extra.push(0);
        assert!(Heatmap::from_bytes(&extra, "ts_x", 3).is_none());
        let mut nan = b.clone();
        let n = nan.len();
        nan[n - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(Heatmap::from_bytes(&nan, "ts_x", 3).is_none());
        assert_eq!(Heatmap::new("big", 5000).len(), MAX_WAYPOINTS);
    }
}
