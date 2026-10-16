//! What a team's bots tell each other (port addition, design §4): enemies seen, sounds heard,
//! where each is going and by which path, and where each is. Upstream's bots shared nothing
//! but squads, which TS bots never form on their own.
//!
//! The boards live in [`super::Squads`] (the host's, kept across frames and cleared at a map
//! change), one per rules team; in deathmatch nobody shares.

use super::EntRef;
use crate::Vector;
use crate::config::MAX_PLAYERS;

/// Seconds a sighting counts.
pub(crate) const SIGHTING_FOR: f32 = 10.0;
/// Seconds a sound counts.
const SOUND_FOR: f32 = 8.0;
/// Seconds a bot's post of its position and path counts.
const POST_FOR: f32 = 2.0;
/// Rules teams with a board (TS has four).
const TEAMS: usize = 4;

/// An enemy a teammate saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sighting {
    /// Where.
    pub origin: Vector,
    /// When.
    pub time: f32,
    /// Who.
    pub enemy: EntRef,
    /// The bot that saw it.
    pub by: u32,
}

/// A sound a teammate heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TeamSound {
    /// Where.
    pub origin: Vector,
    /// When.
    pub time: f32,
    /// How loud.
    pub loud: f32,
    /// The bot that heard it.
    pub by: u32,
}

/// Where a teammate is going.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Claim {
    /// The bot.
    pub by: u32,
    /// Its goal waypoint (-1 none).
    pub waypoint: i32,
    /// The item it goes for.
    pub item: Option<EntRef>,
    /// Until when.
    pub until: f32,
}

/// One team's board.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TeamBoard {
    /// Enemies seen lately.
    pub sightings: Vec<Sighting>,
    /// Sounds heard lately.
    pub sounds: Vec<TeamSound>,
    /// Goals taken.
    pub claims: Vec<Claim>,
    /// Each bot's position, path (waypoints left) and when it posted them.
    pub members: Vec<(u32, Vector, Vec<usize>, f32)>,
}

impl TeamBoard {
    /// A bot saw `enemy` at `origin`.
    pub fn sight(&mut self, by: u32, enemy: EntRef, origin: Vector, now: f32) {
        self.sightings.retain(|s| s.enemy != enemy);
        self.sightings.push(Sighting { origin, time: now, enemy, by });
    }

    /// A bot heard something at `origin`.
    pub fn hear(&mut self, by: u32, origin: Vector, loud: f32, now: f32) {
        if let Some(s) = self.sounds.iter_mut().find(|s| (s.origin - origin).length() < 200.0) {
            *s = TeamSound { origin, time: now, loud: s.loud.max(loud), by };
            return;
        }
        self.sounds.push(TeamSound { origin, time: now, loud, by });
    }

    /// A bot goes for a waypoint or an item until `until`.
    pub fn claim(&mut self, c: Claim) {
        self.claims.retain(|x| x.by != c.by);
        self.claims.push(c);
    }

    /// A bot's position and path now.
    pub fn post(&mut self, by: u32, at: Vector, path: &[usize], now: f32) {
        self.members.retain(|m| m.0 != by);
        self.members.push((by, at, path.to_vec(), now));
    }

    /// A bot left the team (died, left, changed team): what it posted goes.
    pub fn leave(&mut self, by: u32) {
        self.members.retain(|m| m.0 != by);
        self.claims.retain(|c| c.by != by);
    }

    /// Forgets what is old.
    pub fn expire(&mut self, now: f32) {
        self.sightings.retain(|s| s.time <= now && now - s.time <= SIGHTING_FOR);
        self.sounds.retain(|s| s.time <= now && now - s.time <= SOUND_FOR);
        self.claims.retain(|c| c.until > now);
        self.members.retain(|m| m.3 <= now && now - m.3 <= POST_FOR);
    }

    /// An enemy that died isn't anywhere.
    pub fn forget(&mut self, enemy: EntRef) {
        self.sightings.retain(|s| s.enemy != enemy);
    }

    /// The teammates' (not `me`'s) positions.
    pub fn mates(&self, me: u32) -> impl Iterator<Item = Vector> + '_ {
        self.members.iter().filter(move |m| m.0 != me).map(|m| m.1)
    }

    /// The waypoints on teammates' paths, sorted and without repeats.
    pub fn paths_of_others(&self, me: u32) -> Vec<usize> {
        let mut out: Vec<usize> = self.members.iter().filter(|m| m.0 != me).flat_map(|m| m.2.iter().copied()).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Every team's board.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TeamBoards {
    boards: [TeamBoard; TEAMS],
}

impl TeamBoards {
    /// Rules team `team`'s board (1-4); none outside teamplay (team 0).
    pub fn get(&self, team: i32) -> Option<&TeamBoard> {
        usize::try_from(team - 1).ok().and_then(|i| self.boards.get(i))
    }

    /// See [`TeamBoards::get`].
    pub fn get_mut(&mut self, team: i32) -> Option<&mut TeamBoard> {
        usize::try_from(team - 1).ok().and_then(|i| self.boards.get_mut(i))
    }

    /// A player died: no board sights it any more, and a bot's posts go.
    pub fn died(&mut self, player: EntRef) {
        for b in &mut self.boards {
            b.forget(player);
            if (player.index as usize) <= MAX_PLAYERS {
                b.leave(player.index);
            }
        }
    }

    /// Forgets everything (a new map).
    pub fn clear(&mut self) {
        *self = TeamBoards::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boards_share_and_forget() {
        let mut b = TeamBoards::default();
        assert!(b.get(0).is_none() && b.get(5).is_none());
        let e = EntRef { index: 7, serial: 1 };
        let t = b.get_mut(2).unwrap();
        t.sight(1, e, Vector::ZERO, 10.0);
        t.sight(3, e, Vector::new(100.0, 0.0, 0.0), 11.0);
        assert_eq!(t.sightings.len(), 1);
        t.post(1, Vector::ZERO, &[5, 3], 11.0);
        t.post(3, Vector::ZERO, &[3, 9], 11.0);
        assert_eq!(t.paths_of_others(1), vec![3, 9]);
        assert_eq!(t.mates(1).count(), 1);
        t.claim(Claim { by: 1, waypoint: 4, item: None, until: 30.0 });
        t.expire(14.0);
        assert!(t.members.is_empty() && t.claims.len() == 1 && t.sightings.len() == 1);
        b.died(e);
        assert!(b.get(2).unwrap().sightings.is_empty());
        b.died(EntRef { index: 1, serial: 1 });
        assert!(b.get(2).unwrap().claims.is_empty());
    }
}
