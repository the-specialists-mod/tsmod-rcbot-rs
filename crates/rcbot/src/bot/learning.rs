//! The bot's side of the port's learning (port additions, design §4): the damage and kill
//! events the host reports go into the bot's [`Ledger`], and the learners read it.
//!
//! Upstream graded a whole life with one fitness (`survival * 0.001 + frags * 0.2 + team *
//! 0.2`), which in TS deathmatch scores 0, 0.2 or 0.4 and grades the loadout, the stunts, the
//! survival and the personal genes at once. The ledger instead knows what passed between the bot
//! and each opponent, when, with which weapon and at what range.

use super::kungfu::FightEnd;
use super::{Bot, BotGlobals, BotWorld};
use crate::Random;
use crate::learn::ledger::{CombatEvent, DamageEvent, KillEvent, Ledger, LedgerEntry, range_bin};

/// Seconds after its last think in which a kung fu fight still counts as under way when it ends.
const FIGHT_RECENT: f32 = 1.0;

/// What a bot's learners keep beside upstream's `.rld` objects.
#[derive(Clone, Debug, Default)]
pub struct BotLearning {
    /// What passed between the bot and the other players.
    pub ledger: Ledger,
    /// When this life started (the first think alive), 0 while dead.
    pub(crate) life_start: f32,
}

impl Bot {
    /// The bot's combat ledger.
    pub fn ledger(&self) -> &Ledger {
        &self.learning.ledger
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
                } else if victim == me && attacker != me {
                    let e = LedgerEntry { taken: amount, ..entry(attacker, Self::blow_weapon(w, attacker, inflictor)) };
                    self.learning.ledger.record(e);
                }
            }
            CombatEvent::Kill(KillEvent { killer, victim, weapon, .. }) => {
                if killer == me && victim != me {
                    self.learning.ledger.record(LedgerEntry { kill: true, ..entry(victim, weapon) });
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
    /// stale ones close.
    pub(crate) fn learn_think(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld) {
        let now = w.time();
        if self.learning.life_start == 0.0 {
            self.learning.life_start = now;
        }
        if let Some(e) = self.enemy {
            self.learning.ledger.engage(now, e.index);
        }
        self.learning.ledger.tick(now);
        let _ = g;
    }

    /// The bot died (its first think dead, which may come before the kill is scored): the
    /// life's learners are settled; a kung fu fight under way was lost.
    pub(crate) fn learn_on_death(&mut self, g: &mut BotGlobals<'_>, now: f32, rng: &mut dyn Random) {
        if self.kungfu.fighting(now, FIGHT_RECENT) {
            self.kf_settle(g, now, FightEnd::Death, rng);
        }
        self.learning.ledger.close_all();
        self.learning.life_start = 0.0;
    }
}
