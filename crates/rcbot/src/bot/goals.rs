//! Where the bot goes when it has nothing to do (port addition, design §4).
//!
//! Upstream's TS bot picked up the nearest *visible* item, went for a random objective, or roamed
//! to one of the first 50 waypoints (`WaypointFindRandomGoal`) or to the newest remembered enemy
//! position. The port scores what it knows of the map instead: roaming prefers waypoints the bot
//! hasn't been to for a while ([`Explore`]), and items are worth `value / (distance + danger)`
//! ([`Bot::choose_goal`]): weapons by `bot_weapons.ini`'s priority over what the bot carries (and
//! only when it has the inventory space), power-ups by kind (health when hurt, any when unarmed,
//! none while one is held: taking another would swap it), ammo when low.

use super::items::KnownItem;
use super::{Bot, BotGlobals, BotWorld, EntRef, ItemKind};
use crate::Vector;
use crate::config::flags as cfg;
use crate::nav::{MAX_WAYPOINTS, Waypoints};
use crate::weapons::{MOD_TS, TsWeapon};

/// Seconds after which a waypoint counts as unvisited again.
const FORGET_VISIT: f32 = 120.0;
/// Roaming goals nearer than this are rarely chosen: the bot would be there at once.
const NEAR_GOAL: f32 = 400.0;

/// When the bot last touched each waypoint, for roaming to the parts of the map it hasn't seen
/// lately.
#[derive(Clone, Debug)]
pub(crate) struct Explore {
    visited: Vec<f32>,
}

impl Default for Explore {
    fn default() -> Self {
        Explore { visited: vec![f32::NEG_INFINITY; MAX_WAYPOINTS] }
    }
}

impl Explore {
    /// The bot touched waypoint `i` at `now`.
    pub fn touch(&mut self, i: i32, now: f32) {
        if let Some(v) = usize::try_from(i).ok().and_then(|i| self.visited.get_mut(i)) {
            *v = now;
        }
    }

    /// Seconds since the bot was at waypoint `i` (at most [`FORGET_VISIT`]).
    pub fn since_visit(&self, i: usize, now: f32) -> f32 {
        self.visited.get(i).map_or(FORGET_VISIT, |&t| (now - t).clamp(0.0, FORGET_VISIT))
    }

    /// How much the bot wants to roam to waypoint `i` from `from`: more the longer it hasn't been
    /// there, little when it is near.
    pub fn weight(&self, waypoints: &Waypoints, i: usize, from: Vector, now: f32) -> f32 {
        let mut w = 0.25 + self.since_visit(i, now) / 30.0;
        if (waypoints.origin(i as i32) - from).length() < NEAR_GOAL {
            w *= 0.25;
        }
        w
    }
}

/// Inventory space each TS weapon takes (TS 3.0's weapon table, `slots`), by id.
const WEAPON_SLOTS: [i32; 38] = [
    0, 10, 10, 20, 40, 40, 25, 20, 10, 10, 10, 40, 15, 40, 10, 30, 20, 15, 70, 25, 30, 20, 10, 30, 7, 1, 40, 40, 5, 0, 10, 15, 70, 20, 10,
    1, 15, 15,
];

/// The power-ups TS gives at a touch even with one held (`instant`): health and armour.
const INSTANT_POWERUPS: i32 = POWERUP_HEALTH | POWERUP_ARMOR;
/// The port's power-up bits.
const POWERUP_HEALTH: i32 = 64;
/// See [`POWERUP_HEALTH`].
const POWERUP_ARMOR: i32 = 128;
/// See [`POWERUP_HEALTH`].
const POWERUP_KUNGFU: i32 = 4;

/// The Barrett, TS's sniper rifle.
const SNIPER: i32 = 18;
/// Guns that do best from a held spot: the Barrett and the scoped rifles (M4A1, AK47, AUG, M16A4).
const LONG_GUNS: [i32; 5] = [SNIPER, 5, 13, 15, 27];

/// A goal must score this much to beat roaming ([`Bot::choose_goal`]).
pub(crate) const ROAM_SCORE: f32 = 10.0;

/// Where the bot may go next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Goal {
    /// Pick up an item.
    Item(EntRef),
    /// Go after an enemy last seen at this place and time.
    Hunt(Vector, f32),
    /// Go and see what made a sound there (or what a teammate saw).
    Investigate(Vector),
    /// Outnumbered: join the teammate there.
    Regroup(Vector),
    /// Hold a waypoint with long sight lines for a while (a long gun, low health, a fight near).
    Hold(i32),
    /// Nothing better: roam (or upstream's objectives and end-level waypoints).
    Roam,
}

/// A goal's score: what it is worth over the way there.
pub(crate) fn score(value: f32, distance: f32, danger: f32) -> f32 {
    value * 1000.0 / (distance + 200.0 + 300.0 * danger.max(0.0))
}

impl Bot {
    /// What weapon `id` lying in the map is worth to the bot: a better gun than it has (more
    /// unarmed), ammo for one it has when low; nothing it has no room for.
    pub(crate) fn weapon_value(&self, g: &BotGlobals<'_>, id: i32) -> f32 {
        if g.config.is_on(cfg::TS_DONT_PICKUP_WEAPONS) || id <= 0 || id == TsWeapon::KungFu.id() {
            return 0.0;
        }
        if let Some(wp) = self.weapons.get(id).filter(|w| w.has_weapon) {
            return if !wp.is_melee() && wp.reserve < wp.max_clip.max(1) { 25.0 } else { 0.0 };
        }
        let slots = usize::try_from(id).ok().and_then(|i| WEAPON_SLOTS.get(i)).copied().unwrap_or(0);
        if self.state.free_slots < slots {
            return 0.0;
        }
        let priority = |id: i32| g.presets.get(MOD_TS, id).map_or(0, |p| p.priority);
        match self.weapons.best_loaded_gun() {
            None => 60.0 + 5.0 * priority(id) as f32,
            Some(best) => {
                let gain = priority(id) - self.weapons.get(best).map_or(0, |w| w.priority());
                if gain > 0 { 15.0 * gain as f32 } else { 0.0 }
            }
        }
    }

    /// What a power-up of `bits` is worth: health by what the bot lacks, armour some, the others
    /// a lot unarmed (kung fu most); nothing but the instant kinds while one is held or running
    /// (`+use` on another swaps the held one).
    pub(crate) fn powerup_value(&self, bits: i32) -> f32 {
        if bits & POWERUP_HEALTH != 0 {
            return (self.me.max_health.max(100.0) - self.me.health).max(0.0);
        }
        if bits & POWERUP_ARMOR != 0 {
            return 20.0;
        }
        if bits & INSTANT_POWERUPS == 0 && (self.state.stored_powerup != 0 || self.state.active_powerup != 0) {
            return 0.0;
        }
        let unarmed = self.weapons.best_loaded_gun().is_none();
        match (unarmed, bits & POWERUP_KUNGFU != 0) {
            (true, true) => 80.0,
            (true, false) => 50.0,
            (false, _) => 35.0,
        }
    }

    /// What a known item is worth now.
    pub(crate) fn item_value(&self, g: &BotGlobals<'_>, k: &KnownItem) -> f32 {
        match k.kind {
            ItemKind::GroundWeapon => self.weapon_value(g, k.weapon),
            ItemKind::Powerup => self.powerup_value(k.powerup),
            _ => 0.0,
        }
    }

    /// The danger at the waypoint nearest `at` (the heat map, [`BotGlobals::danger`]).
    pub(crate) fn danger_near(&self, g: &BotGlobals<'_>, w: &dyn BotWorld, at: Vector) -> f32 {
        let wpt = self.nearest_waypoint(g, w, at, 400.0, -1, false, false, false);
        usize::try_from(wpt).map_or(0.0, |i| g.danger_at(i, self.team))
    }

    /// The best thing to do now, by `value / (distance + danger)` against [`ROAM_SCORE`]: the
    /// items the bot knows of (not just those in sight; upstream took the nearest one it saw,
    /// wanted or not), there by the time it would arrive.
    pub(crate) fn choose_goal(&self, g: &BotGlobals<'_>, w: &dyn BotWorld, now: f32) -> (Goal, f32) {
        let mut best = (Goal::Roam, ROAM_SCORE);
        let from = self.me.origin;
        for k in self.items.iter() {
            let dist = (k.origin - from).length() * 1.3;
            let arrive = now + dist / self.max_speed.max(1.0);
            if !k.there_at(arrive, now) || k.origin.z > from.z + 1000.0 {
                continue;
            }
            let value = self.item_value(g, k) * self.claim_item_factor(g, k.handle);
            if value <= 0.0 {
                continue;
            }
            let s = score(value, dist, self.danger_near(g, w, k.origin));
            if s > best.1 {
                best = (Goal::Item(k.handle), s);
            }
        }
        // Enemies seen lately and sounds heard: worth more fresh, to a bot ready to fight.
        let ready = self.fight_readiness();
        for p in self.remembered.iter().filter(|p| p.entity.is_some() && !p.hunted) {
            let fresh = 1.0 - (now - p.time) / super::remember::REMEMBER_FOR;
            let s = score(45.0 * fresh * ready, (p.origin - from).length(), 0.0);
            if s > best.1 {
                best = (Goal::Hunt(p.origin, p.time), s);
            }
        }
        for h in self.hearing.iter().filter(|h| !h.investigated) {
            let fresh = 1.0 - (now - h.time) / super::hearing::HEARD_FOR;
            let s = score(30.0 * h.loud * fresh * ready, (h.origin - from).length(), 0.0);
            if s > best.1 {
                best = (Goal::Investigate(h.origin), s);
            }
        }
        // What teammates saw and heard, worth less (they may be on it) and only when nobody claimed it.
        if let Some(board) = g.squads.boards.get(self.state.team) {
            for t in board.sightings.iter().filter(|t| t.by != self.index) {
                let fresh = 1.0 - (now - t.time) / super::board::SIGHTING_FOR;
                let s = score(25.0 * fresh * ready * self.claim_near(g, w, t.origin), (t.origin - from).length(), 0.0);
                if s > best.1 {
                    best = (Goal::Investigate(t.origin), s);
                }
            }
            for t in board.sounds.iter().filter(|t| t.by != self.index) {
                let fresh = 1.0 - (now - t.time) / 8.0;
                let s = score(15.0 * t.loud * fresh * ready * self.claim_near(g, w, t.origin), (t.origin - from).length(), 0.0);
                if s > best.1 {
                    best = (Goal::Investigate(t.origin), s);
                }
            }
        }
        if self.hold_again <= now
            && let Some((spot, value)) = self.hold_spot(g, now)
        {
            let s = score(value, (g.waypoints.origin(spot) - from).length(), self.danger_near(g, w, g.waypoints.origin(spot)));
            if s > best.1 {
                best = (Goal::Hold(spot), s);
            }
        }
        if let Some(mate) = self.regroup_point(g, now) {
            let s = score(60.0, (mate - from).length(), 0.0);
            if s > best.1 {
                best = (Goal::Regroup(mate), s);
            }
        }
        best
    }

    /// Whether to hold a place rather than roam, and where: a waypoint within 600 units that sees
    /// the most of the map (the visibility table), not taken by a teammate. Worth it with a long
    /// gun (a sniper rifle, a scoped rifle), hurt, or soon after a fight; little after a long quiet.
    pub(crate) fn hold_spot(&self, g: &BotGlobals<'_>, now: f32) -> Option<(i32, f32)> {
        let weapon = self.current_weapon.unwrap_or(0);
        let mut value = 0.0;
        if LONG_GUNS.contains(&weapon) {
            value += if weapon == SNIPER { 30.0 } else { 12.0 };
        }
        if self.me.health < 50.0 {
            value += 10.0;
        }
        if self.last_see_enemy_time > 0.0 && now - self.last_see_enemy_time < 10.0 {
            value += 6.0;
        }
        if value <= 0.0 {
            return None;
        }
        let wp = &*g.waypoints;
        let from = self.me.origin;
        let mut best: Option<(usize, usize)> = None;
        for i in 0..wp.len() {
            let f = wp.flags(i as i32);
            if f & (crate::nav::flags::DELETED | crate::nav::flags::AIMING | crate::nav::flags::UNREACHABLE) != 0
                || (wp.origin(i as i32) - from).length() > 600.0
                || self.claim_factor(g, i) < 1.0
            {
                continue;
            }
            let sees = (0..wp.len()).filter(|&j| j != i && wp.visibility.get(i, j)).count();
            if best.is_none_or(|(_, b)| sees > b) {
                best = Some((i, sees));
            }
        }
        // A spot that sees little is no better than walking.
        let (spot, sees) = best?;
        (sees >= 8).then_some((spot as i32, value))
    }

    /// [`Bot::claim_factor`] for the waypoint nearest a place.
    fn claim_near(&self, g: &BotGlobals<'_>, w: &dyn BotWorld, at: Vector) -> f32 {
        let wpt = self.nearest_waypoint(g, w, at, 400.0, -1, false, false, false);
        usize::try_from(wpt).map_or(1.0, |i| self.claim_factor(g, i))
    }

    /// How ready the bot is to look for a fight: its health, less without a loaded gun.
    pub(crate) fn fight_readiness(&self) -> f32 {
        let health = (self.me.health / self.me.max_health.max(100.0)).clamp(0.1, 1.0);
        if self.weapons.best_loaded_gun().is_some() { health } else { health * 0.5 }
    }

    /// Where an enemy last seen at `origin` `secs` ago may be now: a waypoint about as far along
    /// the graph from there as it could have walked (200 units/s, at most 1500), out of the bot's
    /// sight (it went where the bot doesn't see it); the nearest waypoint to `origin` when it was
    /// seen just now. -1 for none.
    pub(crate) fn predict_enemy(
        &self,
        g: &BotGlobals<'_>,
        w: &dyn BotWorld,
        origin: Vector,
        secs: f32,
        rng: &mut dyn crate::Random,
    ) -> i32 {
        let start = self.nearest_waypoint(g, w, origin, crate::nav::REACHABLE_RANGE, -1, false, false, false);
        let Ok(start) = usize::try_from(start) else { return -1 };
        let reach = (secs * 200.0).min(1500.0);
        if reach < 300.0 {
            return start as i32;
        }
        let wp = &*g.waypoints;
        // Dijkstra by length from `start`, up to `reach`.
        let mut dist = vec![f32::INFINITY; wp.len()];
        let mut open = vec![start];
        dist[start] = 0.0;
        while let Some(i) = open.pop() {
            for s in wp.successors(i, -1) {
                let d = dist[i] + (wp.origin(s as i32) - wp.origin(i as i32)).length();
                if d < dist[s] && d <= reach {
                    dist[s] = d;
                    open.push(s);
                }
            }
        }
        let mine = usize::try_from(self.current_waypoint).ok();
        let candidates: Vec<usize> = (0..wp.len())
            .filter(|&i| dist[i] >= reach * 0.5 && dist[i].is_finite())
            .filter(|&i| wp.flags(i as i32) & (crate::nav::flags::DELETED | crate::nav::flags::AIMING) == 0)
            .filter(|&i| mine.is_none_or(|m| !wp.visibility.get(m, i)))
            .collect();
        if candidates.is_empty() {
            return start as i32;
        }
        candidates[rng.random_long(0, candidates.len() as i32 - 1) as usize] as i32
    }

    /// How much the bot still wants an item a teammate is going for (1: nobody is).
    pub(crate) fn claim_item_factor(&self, g: &BotGlobals<'_>, item: EntRef) -> f32 {
        let taken =
            g.squads.boards.get(self.state.team).is_some_and(|b| b.claims.iter().any(|c| c.by != self.index && c.item == Some(item)));
        if taken { 0.1 } else { 1.0 }
    }

    /// Outnumbered: more enemies seen near the bot in the last 5 s (by it or its team) than it has
    /// teammates near it, by two or more. Where the nearest teammate is, if so.
    pub(crate) fn regroup_point(&self, g: &BotGlobals<'_>, now: f32) -> Option<Vector> {
        let board = g.squads.boards.get(self.state.team)?;
        let me = self.me.origin;
        let mut enemies: Vec<EntRef> =
            board.sightings.iter().filter(|s| now - s.time < 5.0 && (s.origin - me).length() < 1500.0).map(|s| s.enemy).collect();
        enemies.extend(self.remembered.iter().filter(|p| now - p.time < 5.0 && (p.origin - me).length() < 1500.0).filter_map(|p| p.entity));
        enemies.sort_by_key(|e| (e.index, e.serial));
        enemies.dedup();
        let near = board.mates(self.index).filter(|m| (*m - me).length() < 1000.0).count();
        if enemies.len() < near + 2 {
            return None;
        }
        board.mates(self.index).filter(|m| (*m - me).length() >= 300.0).min_by(|a, b| (*a - me).length().total_cmp(&(*b - me).length()))
    }
}
