//! The bots in the game (`CBotGlobals::m_Bots` and the per-bot part of `StartFrame`,
//! `globals.cpp`; `UTIL_GetBotPointer`, `UTIL_GetBotIndex`; `bot_client.cpp`
//! `BotClient_Generic_DeathMessage`'s events; `engine.cpp` `pfnEmitSound`'s loop over the bots;
//! `dll.cpp` `DispatchTouch` and `ClientCommand`'s `say`/`say_team`; `bot.cpp`
//! `BotFunc_MakeSquad`).

use super::events::BOT_HEAR_DISTANCE;
use super::{Bot, BotEvent, BotGlobals, BotWorld, Entity, Frame, OtherBot, SoundKind, SquadOrder, cond};
use crate::config::MAX_PLAYERS;
use crate::learn::ledger::CombatEvent;
use crate::{Random, Vector};

/// A bot slot: the bot and the frame it produced.
pub type BotSlot = Option<Bot>;

/// `m_Bots`, by player index (1-based; slot 0 unused).
#[derive(Clone, Debug)]
pub struct BotManager {
    bots: Vec<BotSlot>,
}

impl Default for BotManager {
    fn default() -> Self {
        BotManager { bots: vec![None; MAX_PLAYERS + 1] }
    }
}

impl BotManager {
    /// Puts a bot in its player's slot.
    pub fn add(&mut self, bot: Bot) {
        let i = bot.index as usize;
        if let Some(s) = self.bots.get_mut(i) {
            *s = Some(bot);
        }
    }

    /// Frees the slot (the client left).
    pub fn remove(&mut self, index: u32) -> Option<Bot> {
        self.bots.get_mut(index as usize).and_then(Option::take)
    }

    /// `UTIL_GetBotPointer`.
    pub fn get(&self, index: u32) -> Option<&Bot> {
        self.bots.get(index as usize).and_then(Option::as_ref)
    }

    /// `UTIL_GetBotPointer`, for changing it.
    pub fn get_mut(&mut self, index: u32) -> Option<&mut Bot> {
        self.bots.get_mut(index as usize).and_then(Option::as_mut)
    }

    /// The bots, by index.
    pub fn iter(&self) -> impl Iterator<Item = &Bot> {
        self.bots.iter().flatten()
    }

    /// The bots, for changing them.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Bot> {
        self.bots.iter_mut().flatten()
    }

    /// `m_iNumBots`.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// No bots.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forgets every bot (map change; the port kicks or re-adopts them).
    pub fn clear(&mut self) {
        self.bots.fill(None);
    }

    /// Takes every bot out (to save them).
    pub fn drain(&mut self) -> Vec<Bot> {
        self.bots.iter_mut().filter_map(Option::take).collect()
    }

    /// What another bot sees of `b`.
    fn other(b: &Bot) -> OtherBot {
        OtherBot { handle: b.me.handle(), current_waypoint: b.current_waypoint, gun_position: b.gun_position() }
    }

    /// Hands the squad orders to their bots.
    pub fn apply_squad_orders(&mut self, g: &mut BotGlobals<'_>) {
        for order in std::mem::take(&mut g.squads.orders) {
            let (SquadOrder::SetSquad(i, _) | SquadOrder::ReturnToFormation(i, _)) = order;
            if let Some(b) = self.get_mut(i) {
                b.squad_order(order);
            }
        }
    }

    /// `StartFrame`'s loop: each bot in index order may leave (`BOTS_LEAVE_AND_JOIN`: it says
    /// goodbye, and is kicked 5-8 s later), then kicks, suicides, or thinks and moves. Then the
    /// squads follow their leaders' headings.
    pub fn frame(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) -> Vec<(u32, Frame)> {
        let now = w.time();
        let mut out = Vec::new();
        let indices: Vec<u32> = self.iter().map(|b| b.index).collect();
        // Every bot as the others see it, built once and kept up to date as each one runs (the
        // bots before it this frame are seen as they are now, as upstream's loop sees them).
        let mut all: Vec<(u32, OtherBot)> = self.iter().map(|b| (b.index, Self::other(b))).collect();
        self.refresh_player_keys(g);
        for i in indices {
            self.apply_squad_orders(g);
            g.other_bots.clear();
            g.other_bots.extend(all.iter().filter(|(j, _)| *j != i).map(|(_, o)| *o));
            let cam = g.botcam.current_bot();
            g.botcam.other_has_enemy = cam.filter(|&c| c != i).and_then(|c| self.get(c)).is_some_and(|b| b.enemy.is_some());
            let Some(bot) = self.get_mut(i) else { continue };
            if bot.has(cond::WANT_TO_LEAVE_GAME) {
                if bot.leave_time == 0.0 {
                    bot.leave_time = now + rng.random_float(5.0, 8.0);
                    bot.event(g, w, BotEvent::Leaving, rng);
                } else if bot.leave_time < now {
                    bot.kick = true;
                    bot.leave_time = 0.0;
                    bot.clear(cond::WANT_TO_LEAVE_GAME);
                }
            }
            out.push((i, bot.frame(g, w, rng)));
            if let Some(slot) = all.iter_mut().find(|(j, _)| *j == i) {
                slot.1 = Self::other(bot);
            }
        }
        self.apply_squad_orders(g);
        g.squads.update_angles(w);
        out
    }

    /// `BotClient_Generic_DeathMessage`: `killer` (-1 none, 0 the world) killed `victim`. The
    /// victim bot of a suicide curses or laughs; otherwise the killer and victim bots hear
    /// about it, and every other bot that saw the victim or the killer reacts (a teammate's
    /// death, an enemy's kill, a player's death), or hears a teammate die within 650 units.
    ///
    /// Upstream tells teams apart with `pev->team`, which TS 3.0 doesn't set: the port's rules
    /// team is used (design §4).
    pub fn death(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, killer: i32, victim: i32, rng: &mut dyn Random) {
        // A dead player isn't where it was last seen any more (design §4).
        if let Some(v) = u32::try_from(victim).ok().and_then(|v| w.entity(v)) {
            for b in self.iter_mut() {
                b.remembered.forget(v.handle());
            }
            g.squads.boards.died(v.handle());
        }
        if killer == victim || killer == -1 {
            if let Some(b) = u32::try_from(victim).ok().and_then(|v| self.get_mut(v)) {
                b.event(g, w, BotEvent::KillSelf, rng);
            }
            return;
        }
        if victim == -1 {
            return;
        }
        let (Some(k), Some(v)) = (w.entity(killer as u32), w.entity(victim as u32)) else { return };
        if let Some(b) = self.get_mut(k.index) {
            b.event(g, w, BotEvent::Kill(v.handle()), rng);
        }
        if let Some(b) = self.get_mut(v.index) {
            b.event(g, w, BotEvent::Died(Some(k.handle())), rng);
        }
        let has_team = |e: &Entity| e.player.is_some_and(|p| p.team != 0);
        let mut not_handled = Vec::new();
        let indices: Vec<u32> = self.iter().map(|b| b.index).filter(|&i| i != k.index && i != v.index).collect();
        for i in indices {
            let Some(b) = self.get_mut(i) else { continue };
            if b.is_visible(v.index) {
                let ev = if has_team(&v) {
                    if !b.is_enemy(w, &v) {
                        BotEvent::SeeTeammateDie(v.handle(), Some(k.handle()))
                    } else {
                        BotEvent::SeePlayerDie(v.handle(), Some(k.handle()))
                    }
                } else {
                    BotEvent::SeePlayerDie(v.handle(), Some(k.handle()))
                };
                b.event(g, w, ev, rng);
                continue;
            }
            if b.is_visible(k.index) {
                if has_team(&k) {
                    if !b.is_enemy(w, &k) {
                        b.event(g, w, BotEvent::SeeTeammateKill, rng);
                        continue;
                    } else if !b.is_enemy(w, &v) {
                        b.event(g, w, BotEvent::SeeEnemyKill(k.handle(), v.handle()), rng);
                        continue;
                    }
                } else {
                    b.event(g, w, BotEvent::SeePlayerDie(k.handle(), Some(v.handle())), rng);
                    continue;
                }
            }
            not_handled.push(i);
        }
        for i in not_handled {
            let Some(b) = self.get_mut(i) else { continue };
            if !b.is_enemy(w, &v) && b.distance_from(v.entity_origin(), false) < BOT_HEAR_DISTANCE && b.fvisible_point(w, v.entity_origin())
            {
                b.event(g, w, BotEvent::HearTeammateDie(v.handle()), rng);
            }
        }
    }

    /// The host's damage and kill events (`damage`, `kill_scored`; port addition, design §4): a
    /// bot that was hurt reacts (`BotEvent(BOT_EVENT_HURT)`), and the bots that dealt or took it
    /// record it.
    pub fn combat_event(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, ev: &CombatEvent, rng: &mut dyn Random) {
        if let CombatEvent::Damage(d) = *ev
            && d.attacker != 0
            && d.attacker != d.victim
            && let Some(b) = self.get_mut(d.victim)
        {
            b.hurt_by(w, &d);
        }
        self.refresh_player_keys(g);
        self.update_heatmap(g, w, ev);
        self.update_opponents(g, w, ev);
        // Every bot hears it: those who dealt or took it keep it, one whose opponent died ends
        // its fight.
        for bot in self.iter_mut() {
            bot.combat_event(g, w, ev, rng);
        }
    }

    /// The opponent model: hits on bots and kills of bots by each player, and players' deaths
    /// to bots (any backend's bots).
    fn update_opponents(&self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, ev: &CombatEvent) {
        let is_bot = |i: u32| w.entity(i).and_then(|e| e.player).is_some_and(|p| p.is_bot) || self.get(i).is_some();
        let keys = &g.learned.player_keys;
        match *ev {
            CombatEvent::Damage(d) => {
                let (Some(key), Some(a), Some(v)) = (keys.get(d.attacker), w.entity(d.attacker), w.entity(d.victim)) else { return };
                if d.attacker != d.victim && is_bot(d.victim) {
                    // The blow's weapon as the rules count it, else what the attacker holds.
                    let weapon = if d.weapon >= 0 { d.weapon } else { a.player.map_or(-1, |p| p.weapon) };
                    let headshot = d.hitgroup == crate::learn::ledger::HITGROUP_HEAD;
                    g.learned.opponents.observe_hit_on_bot(key, weapon, (a.origin - v.origin).length(), d.amount, headshot);
                }
            }
            CombatEvent::Kill(k) => {
                if k.killer == 0 || k.killer == k.victim {
                    return;
                }
                if let Some(key) = keys.get(k.killer).filter(|_| is_bot(k.victim)) {
                    g.learned.opponents.observe_kill_of_bot(key);
                }
                if let Some(key) = keys.get(k.victim).filter(|_| is_bot(k.killer)) {
                    g.learned.opponents.observe_death_to_bot(key);
                }
            }
        }
    }

    /// The host's `player_stunt`: a player started a stunt (for the opponent model).
    pub fn player_stunt(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, player: u32) {
        self.refresh_player_keys(g);
        if let Some(key) = g.learned.player_keys.get(player) {
            g.learned.opponents.observe_stunt(key, w.time());
        }
    }

    /// The waypoint a player is at (a bot's current one, else the nearest to its origin) and
    /// its rules team.
    fn player_waypoint(&self, g: &BotGlobals<'_>, w: &dyn BotWorld, index: u32) -> Option<(usize, i32)> {
        let e = w.entity(index)?;
        let team = e.player.map_or(0, |p| p.team);
        let own = self.get(index).and_then(|b| usize::try_from(b.current_waypoint).ok());
        let opts = crate::nav::NearestOptions { visible: false, unreachable: true, ..crate::nav::NearestOptions::default() };
        let wpt = own.or_else(|| g.waypoints.nearest_waypoint(e.origin, crate::nav::REACHABLE_RANGE, None, opts, &[], w))?;
        Some((wpt, team))
    }

    /// The map's heatmap: a death at the victim's waypoint, a kill at the killer's, damage taken
    /// at the victim's.
    fn update_heatmap(&self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, ev: &CombatEvent) {
        if g.learned.heatmap.is_empty() {
            return;
        }
        match *ev {
            CombatEvent::Damage(d) => {
                if let Some((wpt, team)) = self.player_waypoint(g, w, d.victim) {
                    g.learned.heatmap.add_damage(wpt, team, d.amount);
                }
            }
            CombatEvent::Kill(k) => {
                if let Some((wpt, team)) = self.player_waypoint(g, w, k.victim) {
                    g.learned.heatmap.add_death(wpt, team);
                }
                if k.killer != 0
                    && k.killer != k.victim
                    && let Some((wpt, team)) = self.player_waypoint(g, w, k.killer)
                {
                    g.learned.heatmap.add_kill(wpt, team);
                }
            }
        }
    }

    /// Who each player is for the learners: RCBot's bots by profile, the rest by reputation id.
    fn refresh_player_keys(&self, g: &mut BotGlobals<'_>) {
        g.learned.player_keys.refresh(g.rep_ids, self.iter().map(|b| (b.index, b.profile.profile_id)));
    }

    /// `pfnEmitSound`'s loop: every bot without an enemy within [`BOT_HEAR_DISTANCE`] of the
    /// sound (not the entity itself or its owner) hears it. The sound is where the game played
    /// it (`origin`; else the entity's centre, upstream's `EntityOrigin`, which for the world is
    /// the middle of the map); a silent one (`volume` 0) isn't heard. Kinds no bot reacts to stop
    /// here.
    pub fn sound(&mut self, w: &dyn BotWorld, entity: u32, origin: Option<Vector>, volume: f32, sample: &str) {
        let kind = SoundKind::of_sample(sample);
        if !kind.is_heard() || volume <= 0.0 {
            return;
        }
        let Some(e) = w.entity(entity) else { return };
        let origin = origin.unwrap_or_else(|| e.entity_origin());
        for b in self.iter_mut() {
            if Some(b.index) == e.owner || b.index == e.index || b.enemy.is_some() {
                continue;
            }
            if b.distance_from(origin, false) < BOT_HEAR_DISTANCE {
                b.hear_sound(w, kind, &e, origin);
            }
        }
    }

    /// `DispatchTouch(touched, bot)`.
    pub fn touch(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, bot: u32, touched: &Entity, rng: &mut dyn Random) {
        if let Some(b) = self.get_mut(bot) {
            b.touch(g, w, touched, rng);
        }
    }

    /// `BotFunc_MakeSquad`: every bot within 512 units of the player that can joins their squad.
    pub fn make_squad(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, leader: &Entity) {
        let leader_is_bot = self.get(leader.index).is_some();
        for b in self.iter_mut() {
            if b.distance_from(leader.entity_origin(), false) > 512.0 {
                continue;
            }
            if b.can_add_to_squad(w, leader) {
                b.add_to_squad(g.squads, leader, leader_is_bot);
            }
        }
        self.apply_squad_orders(g);
    }

    /// `ClientCommand`'s `say`/`say_team`: every bot (the sender's team for `say_team`) may
    /// reply, all reading the same buffer.
    pub fn say(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, sender: &Entity, message: &str, team_only: bool, rng: &mut dyn Random) {
        let sender_is_bot = self.get(sender.index).is_some();
        let sender_team = sender.player.map_or(0, |p| p.team);
        let mut buffer = message.as_bytes().to_vec();
        for b in self.iter_mut() {
            if !team_only || sender_team == b.state.team {
                b.reply_to_message(g, w, &mut buffer, sender, sender_is_bot, team_only, rng);
            }
        }
    }
}
