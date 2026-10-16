//! The bot's side of the port's learning (port additions, design §4): the damage and kill
//! events the host reports go into the bot's [`Ledger`], and the learners read it.
//!
//! Upstream graded a whole life with one fitness (`survival * 0.001 + frags * 0.2 + team *
//! 0.2`), which in TS deathmatch scores 0, 0.2 or 0.4 and grades the loadout, the stunts, the
//! survival and the personal genes at once. The ledger instead knows what passed between the bot
//! and each opponent, when, with which weapon and at what range.

use super::kungfu::FightEnd;
use super::{Bot, BotGlobals, BotWorld, cond};
use crate::Random;
use crate::config::flags as cfg;
use crate::learn::ledger::{CombatEvent, DamageEvent, KillEvent, Ledger, LedgerEntry, range_bin};
use crate::learn::loadout::{CREDIT_WINDOW, LOADOUTS, LoadoutBandit, credited_loadout, life_reward, weapon_slots};
use crate::learn::weapon_stats::{MIN_SAMPLES, WeaponStats};

/// The longest think gap counted as exposure (a stall isn't time spent fighting).
const MAX_TICK: f32 = 0.25;
/// Most rounds a clip can drop between two thinks by firing (more is a reload or a switch).
const MAX_SHOTS_PER_TICK: i32 = 10;

/// Seconds after its last think in which a kung fu fight still counts as under way when it ends.
const FIGHT_RECENT: f32 = 1.0;

/// What a bot's learners keep beside upstream's `.rld` objects.
#[derive(Clone, Debug, Default)]
pub struct BotLearning {
    /// What passed between the bot and the other players.
    pub ledger: Ledger,
    /// Weapon effectiveness by range: damage per second of exposure, accuracy.
    pub weapons: WeaponStats,
    /// The loadout estimates.
    pub loadout: LoadoutBandit,
    /// The loadout bought for the next life, and the one the inventory showed this life (once
    /// checked).
    pub(crate) pending_loadout: Option<usize>,
    pub(crate) credited_loadout: Option<usize>,
    pub(crate) credit_checked: bool,
    /// Kills and damage dealt this life.
    pub(crate) life_kills: u32,
    pub(crate) life_dealt: f32,
    /// When this life started (the first think alive), 0 while dead.
    pub(crate) life_start: f32,
    /// The last learning think (0 none this life), the weapon and clip then, and the enemy's
    /// distance last known.
    pub(crate) last_tick: f32,
    pub(crate) last_clip: Option<(i32, i32)>,
    pub(crate) last_enemy_dist: f32,
}

impl Bot {
    /// The bot's combat ledger.
    pub fn ledger(&self) -> &Ledger {
        &self.learning.ledger
    }

    /// How dangerous `waypoint` is for the bot's team on this map (the heatmap, 0..1).
    pub fn waypoint_danger(&self, g: &BotGlobals<'_>, waypoint: usize) -> f32 {
        g.learned.heatmap.danger(waypoint, self.team)
    }

    /// The bot's weapon-by-range table.
    pub fn weapon_stats(&self) -> &WeaponStats {
        &self.learning.weapons
    }

    /// How well `weapon` (a TS id, 0 or 38 kung fu) has worked for this bot at `dist`: damage per
    /// second with the enemy in sight, shrunk toward every bot's results; `None` until the bot
    /// has [`MIN_SAMPLES`] seconds of its own there.
    pub fn weapon_score(&self, g: &BotGlobals<'_>, weapon: i32, dist: f32) -> Option<f32> {
        let t = &self.learning.weapons;
        (t.samples(weapon, dist) >= MIN_SAMPLES).then(|| t.score(&g.learned.weapon_prior, weapon, dist))
    }

    /// The TS weapon id of a blow `attacker` dealt through `inflictor`: a grenade's 24, another
    /// projectile unknown (-1), else what the attacker holds.
    fn blow_weapon(w: &dyn BotWorld, attacker: u32, inflictor: i32) -> i32 {
        if inflictor > 0 && inflictor as u32 != attacker {
            let grenade = w.entity(inflictor as u32).is_some_and(|e| e.classname.contains("grenade"));
            return if grenade { 24 } else { -1 };
        }
        w.entity(attacker).and_then(|e| e.player).map_or(-1, |p| p.weapon)
    }

    /// A damage or kill event (the host reports every one): the bot keeps those it dealt or
    /// took, and a kill that ends its kung fu fight settles it.
    pub fn combat_event(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, ev: &CombatEvent, rng: &mut dyn Random) {
        let now = w.time();
        let me = self.index;
        let (my_origin, my_waypoint) = (self.me.origin, self.current_waypoint);
        let entry = |other: u32, weapon: i32| {
            let dist = if other == 0 { 0.0 } else { w.entity(other).map_or(0.0, |e| (e.origin - my_origin).length()) };
            LedgerEntry {
                time: now,
                other,
                weapon,
                dealt: 0.0,
                taken: 0.0,
                kill: false,
                death: false,
                range_bin: range_bin(dist) as u8,
                waypoint: my_waypoint,
            }
        };
        match *ev {
            CombatEvent::Damage(DamageEvent { victim, attacker, inflictor, amount, .. }) => {
                if attacker == me && victim != me {
                    let e = LedgerEntry { dealt: amount, ..entry(victim, Self::blow_weapon(w, attacker, inflictor)) };
                    self.learning.ledger.record(e);
                    self.learning.life_dealt += amount;
                    if let Some(v) = w.entity(victim) {
                        let dist = (v.origin - my_origin).length();
                        self.learning.weapons.add_hit(e.weapon, dist, amount);
                        g.learned.weapon_prior.add_hit(e.weapon, dist, amount);
                    }
                } else if victim == me && attacker != me {
                    let e = LedgerEntry { taken: amount, ..entry(attacker, Self::blow_weapon(w, attacker, inflictor)) };
                    self.learning.ledger.record(e);
                }
            }
            CombatEvent::Kill(KillEvent { killer, victim, weapon, .. }) => {
                if killer == me && victim != me {
                    self.learning.ledger.record(LedgerEntry { kill: true, ..entry(victim, weapon) });
                    self.learning.life_kills += 1;
                } else if victim == me {
                    let other = if killer == me { 0 } else { killer };
                    self.learning.ledger.record(LedgerEntry { death: true, ..entry(other, weapon) });
                    self.learning.ledger.close_all();
                }
                // The fight ends with the opponent's death (by anyone) or the bot's.
                if self.kungfu.opponent() == Some(victim) || victim == me {
                    let end = if !self.kungfu.fighting(now, FIGHT_RECENT) {
                        FightEnd::Broken
                    } else if victim == me {
                        FightEnd::Death
                    } else if killer == me {
                        FightEnd::Kill
                    } else {
                        FightEnd::Broken
                    };
                    self.kf_settle(g, now, end, rng);
                }
            }
        }
    }

    /// Every think while alive, after the tasks ran: the episode with the enemy stays open,
    /// stale ones close; time with the enemy in sight and rounds fired go to the weapon table.
    pub(crate) fn learn_think(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld) {
        let now = w.time();
        let l = &mut self.learning;
        if l.life_start == 0.0 {
            l.life_start = now;
        }
        let dt = if l.last_tick > 0.0 { (now - l.last_tick).clamp(0.0, MAX_TICK) } else { 0.0 };
        l.last_tick = now;
        if let Some(e) = self.enemy {
            self.learning.ledger.engage(now, e.index);
        }
        self.learning.ledger.tick(now);

        let enemy_dist = self.resolve(w, self.enemy).map(|e| (e.origin - self.me.origin).length());
        let weapon = self.state.weapon;
        let l = &mut self.learning;
        if let Some(d) = enemy_dist {
            l.last_enemy_dist = d;
            if self.conditions & cond::SEE_ENEMY != 0 {
                l.weapons.add_exposure(weapon, d, dt);
                g.learned.weapon_prior.add_exposure(weapon, d, dt);
            }
        }
        // Rounds fired: the clip of the same gun going down (kung fu's clip is its power bar).
        let clip = (weapon, self.state.clip);
        if let Some((w0, c0)) = l.last_clip
            && w0 == clip.0
            && w0 != 0
            && (1..=MAX_SHOTS_PER_TICK).contains(&(c0 - clip.1))
            && enemy_dist.is_some()
        {
            let fired = (c0 - clip.1) as f32;
            l.weapons.add_shots(weapon, l.last_enemy_dist, fired);
            g.learned.weapon_prior.add_shots(weapon, l.last_enemy_dist, fired);
        }
        l.last_clip = Some(clip);
        self.credit_loadout(w, now);
    }

    /// The loadout for the next life (`Think`'s TS weapon choice, design §4): the bandit's pick
    /// among the loadouts that fit the slots the bot has (what is free and what it carries,
    /// which the new loadout replaces), at random with weapon learning off.
    pub(crate) fn choose_loadout(&mut self, g: &BotGlobals<'_>, rng: &mut dyn Random) -> Vec<i32> {
        let carried: i32 = (1..=37).filter(|&id| self.weapons.has(id)).map(weapon_slots).sum();
        let capacity = if self.state.free_slots > 0 { self.state.free_slots + carried } else { 100 };
        let random = g.config.is_on(cfg::DISABLE_WEAPON_LEARN);
        let i = self.learning.loadout.choose(&g.learned.loadout_prior, capacity, random, rng);
        self.learning.pending_loadout = Some(i);
        LOADOUTS[i].to_vec()
    }

    /// Early in a life: which loadout the game's inventory shows (only that is credited).
    fn credit_loadout(&mut self, w: &dyn BotWorld, now: f32) {
        let l = &mut self.learning;
        if l.credit_checked || l.pending_loadout.is_none() {
            return;
        }
        if now - l.life_start > CREDIT_WINDOW {
            l.credit_checked = true;
            return;
        }
        let Some(owned) = w.inventory(self.index) else { return };
        let ids: Vec<i32> = owned.iter().map(|o| o.weapon).collect();
        if let Some(arm) = credited_loadout(l.pending_loadout, &ids) {
            l.credited_loadout = Some(arm);
            l.credit_checked = true;
        }
    }

    /// The bot died (its first think dead, which may come before the kill is scored): the
    /// life's learners are settled; a kung fu fight under way was lost.
    pub(crate) fn learn_on_death(&mut self, g: &mut BotGlobals<'_>, now: f32, rng: &mut dyn Random) {
        if self.kungfu.fighting(now, FIGHT_RECENT) {
            self.kf_settle(g, now, FightEnd::Death, rng);
        }
        let l = &mut self.learning;
        if let Some(arm) = l.credited_loadout.take()
            && l.life_start > 0.0
            && !g.config.is_on(cfg::DISABLE_WEAPON_LEARN)
        {
            let r = life_reward(l.life_kills, 1, l.life_dealt, now - l.life_start);
            l.loadout.reward(arm, r);
            g.learned.loadout_prior.reward(arm, r);
        }
        l.ledger.close_all();
        l.life_start = 0.0;
        l.last_tick = 0.0;
        l.last_clip = None;
        l.pending_loadout = None;
        l.credit_checked = false;
        l.life_kills = 0;
        l.life_dealt = 0.0;
    }
}
