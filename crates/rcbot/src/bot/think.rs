//! The bot's frame (`globals.cpp`'s per-bot part of `CBotGlobals::StartFrame`; `bot.cpp`
//! `CBot::Think`, `SpawnInit`, `DoTasks`, `LookForNewTasks`, `WantToFindEnemy`;
//! `util.cpp` `UTIL_UpdateSounds`, `UTIL_IsFacingEntity`; `CBot::CanPickup`).
//!
//! TS only. Tasks no TS bot creates are not ported (design §2). Upstream's death-time `buy` /
//! `menuselect 7` (marked "doesn't work") is replaced by the loadout the gun choice buys.

use super::goals::Goal;
use super::look::make_vectors;
use super::{
    Action, Bot, BotGlobals, BotWorld, CmdOutcome, EF_NODRAW, Entity, Frame, ItemKind, LookTask, Task, TaskKind, TsObjective, buttons, cond,
};
use crate::config::flags as cfg;
use crate::learn::BotLearnedData;
use crate::nav::{Glass, MAX_JUMP_HEIGHT, Monsters, REACHABLE_RANGE, flags as fl};
use crate::weapons::TsWeapon;
use crate::{Random, Vector};

/// Seconds between two reads of the owned weapons ([`BotWorld::inventory`]) when nothing
/// changed.
const INVENTORY_SYNC: f32 = 1.0;
/// Seconds a weapon switch waits for the game before it may be asked again.
const SWITCH_WAIT: f32 = 1.0;

/// What the bot's team does with an objective ([`Bot::objective_role`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjectiveRole {
    /// Complete it.
    Attack,
    /// Keep the other team from it.
    Defend,
}

/// Seconds a hunt or investigation may take to reach its place.
const HUNT_TIME: f32 = 15.0;

/// Seconds of pressing use on an item before the bot gives it up as one the game won't give it.
const PICKUP_TRIES: f32 = 1.5;

/// Seconds to reach the first waypoint of a path found (then each touch gives 20-30 s more).
const PATH_TIMEOUT: f32 = 30.0;

/// A move this long in one frame is a teleport (nothing in TS runs a player 256 units in a
/// frame at any usual frame rate).
const TELEPORT_DISTANCE: f32 = 256.0;

/// `UTIL_IsGrenadeRocket`.
pub(crate) fn is_grenade_rocket(e: &Entity) -> bool {
    e.classname.contains("grenade") || e.classname.contains("rpg_rocket")
}

/// `BotFunc_TraceToss`, rewritten (design §4): where a thrown entity lands, flown in 0.1 s steps
/// under TS's grenade gravity (`0.75 sv_gravity`) until something is in the way or the 3 s fuse
/// ends. Upstream's four half-second steps grew the velocity by half of itself less gravity each
/// step (`v + (v - g) / 2`), then traced 256 times the last velocity (§5.75).
pub(crate) fn trace_toss(w: &dyn BotWorld, e: &Entity) -> Vector {
    let g = w.cvar_float("sv_gravity") * 0.75;
    let (mut origin, mut velocity) = (e.origin, e.velocity);
    for _ in 0..30 {
        velocity.z -= g * 0.1;
        let next = origin + velocity * 0.1;
        let tr = w.trace_line(origin, next, Monsters::DontIgnore, Glass::DontIgnore, Some(e.index));
        if tr.fraction < 1.0 {
            return tr.end_pos;
        }
        origin = next;
    }
    origin
}

impl Bot {
    /// One server frame for this bot: the `StartFrame` loop's kick / kill / think, then
    /// `RunPlayerMove`. `world` must report the bot's own entity, else nothing happens.
    pub fn frame(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) -> Frame {
        let mut out = Frame::default();
        let Some(me) = w.entity(self.index) else { return out };
        let (was_alive, was_at) = (self.is_alive(), self.me.origin);
        self.state = me.player.unwrap_or_default();
        // The server sets `v_angle` from our usercmd (`SET_VIEW`), unless the game turned the
        // player (spawn, teleport: `fixangle`); then its view is ours, so the bot doesn't turn
        // back against the game's turn within the command (design §4).
        self.v_angle = self.game_view.take().unwrap_or(me.v_angle);
        self.me = me;
        if was_alive && self.is_alive() && (self.me.origin - was_at).length() > TELEPORT_DISTANCE {
            self.teleported();
        }
        self.base_turn_speed = g.config.turn_speed;

        let now = w.time();
        self.now = now;

        // `BotClient_TS_WeaponInfo`: the port reports the weapon in hand, which is the bot's
        // current weapon every frame (upstream's switch set it before the game agreed; design §4).
        if self.state.alive {
            let held = TsWeapon::from_game_id(self.state.weapon).map_or(self.state.weapon, TsWeapon::id);
            if self.current_weapon != Some(held) {
                // Taken out, picked up, or the game's choice after a drop: read what is owned.
                self.next_inventory_sync = 0.0;
            }
            if self.switch_pending.is_some_and(|(id, until)| id == held || until <= now) {
                self.switch_pending = None;
            }
            self.current_weapon = Some(held);
            // With the game's inventory serial, the inventory is read again only when it changed
            // (or a sync was asked for); without one, every second.
            let serial = self.state.inventory_serial;
            let changed = serial != 0 && serial != self.inventory_serial;
            if changed || (self.next_inventory_sync <= now && (serial == 0 || self.next_inventory_sync == 0.0)) {
                self.next_inventory_sync = now + INVENTORY_SYNC;
                if let Some(owned) = w.inventory(self.index) {
                    self.weapons.sync_inventory(&owned, g.weapons, g.presets);
                    self.inventory_serial = serial;
                }
            }
            let info = (self.state.weapon, self.state.clip, self.state.ammo);
            if self.last_weapon_info != Some(info) {
                // A new weapon or clip: the reload key may be pressed again (design §4).
                if self.last_weapon_info.is_none_or(|(w, c, _)| w != info.0 || c != info.1) {
                    self.next_reload_time = 0.0;
                }
                self.last_weapon_info = Some(info);
                let r = self.weapons.weapon_info(info.0, info.1, info.2, g.weapons, g.presets);
                // Kung fu reports no clip and no ammo too; there is nothing to drop then
                // (upstream sent `drop` each time it took out kung fu).
                if r.drop && r.current != TsWeapon::KungFu.id() {
                    self.add_priority_task(Task::simple(TaskKind::DropWeapon));
                }
            }
        }
        // `StartFrame`: TS bots always have kung fu. Upstream adds weapon 36, which is TS 3.0's
        // Contender; kung fu is 38 (design §5).
        if !self.weapons.has(TsWeapon::KungFu.id()) {
            self.weapons.add(TsWeapon::KungFu.id(), g.weapons);
        }

        if self.kick {
            self.kick = false;
            out.actions.push(Action::Kick);
            return out;
        }
        if self.is_alive() && self.kill {
            self.kill = false;
            out.actions.push(Action::Kill);
            return out;
        }
        self.think(g, w, rng);
        out.actions.append(&mut self.actions);
        out.cmd = Some(self.run_player_move(w.time()));
        out
    }

    /// The game moved the bot elsewhere (a teleport: the server reports it after the usercmd,
    /// [`CmdOutcome::teleported`], or the bot moved more than 256 units in a frame). Its waypoints, path and stuck timers belong to
    /// where it was: they are dropped, so it finds its nearest waypoint and a path from there
    /// (port addition, design §4; upstream kept heading for the old waypoint, got stuck and
    /// failed the path).
    pub fn teleported(&mut self) {
        self.current_waypoint = -1;
        self.prev_waypoint = -1;
        self.current_waypoint_flags = 0;
        self.prev_waypoint_dist = 4096.0;
        self.paths.bot_paths.clear();
        self.update_waypoint_time = 0.0;
        self.stuck_time = 0.0;
        self.last_no_move = 0.0;
        self.unstick = Default::default();
        self.ladder_angles = None;
    }

    /// What the game did with the bot's last usercmd (the host's `run_cmd` result): a view the
    /// game set becomes the bot's (it must not fight the game's turn), and a teleport drops the
    /// path ([`Bot::teleported`]).
    pub fn command_ran(&mut self, r: CmdOutcome) {
        if let Some(v) = r.game_view {
            self.game_view = Some(v);
            self.v_angle = v;
            self.ideal_yaw = v.y;
            self.ideal_pitch = v.x;
        }
        if r.teleported {
            self.teleported();
        }
    }

    /// `CBot::SpawnInit(false)`: forget this life (tasks, path, enemy, waypoints) and pick new
    /// genes.
    pub(crate) fn spawn_init(&mut self, g: &mut BotGlobals<'_>, rng: &mut dyn Random) {
        self.respawn_time = 0.0;
        self.tasks.flush();
        self.paths.bot_paths.clear();
        let team = usize::try_from(self.team).ok().filter(|&t| t < crate::learn::data::MAX_TEAMS).unwrap_or(0);
        self.learned.on_spawn(&mut g.learned.enemy_costs[team], rng);
        self.zoom = false;
        self.used_melee = false;
        self.next_get_enemy_time = 0.0;
        self.avoid_move_to = Vector::ZERO;
        self.avoid_time = 0.0;
        self.hold_attack_time = 0.0;
        self.not_following_waypoint = false;
        self.last_no_move = 0.0;
        self.vis_update_revs = 0;
        self.up_time = 0.0;
        self.paths.next_clear_failed_goals = 0.0;
        self.current_weapon = None;
        self.last_weapon_info = None;
        // A new life starts with what the game gives it: forget the last one's guns (TS bots
        // always have kung fu) and read the inventory at the first frame alive (design §4).
        self.weapons = super::BotWeapons::default();
        self.weapons.add(TsWeapon::KungFu.id(), g.weapons);
        self.switch_pending = None;
        self.next_inventory_sync = 0.0;
        self.inventory_serial = 0;
        self.next_reload_time = 0.0;
        let now = self.now;
        self.kf_settle(g, now, super::kungfu::FightEnd::Broken, rng);
        self.kungfu.reset_fight();
        self.fight = super::fire::Fight::default();
        self.last_enemy = None;
        self.last_failed_waypoint = -1;
        self.listen_to_sound_time = 0.0;
        self.pickup_entity = None;
        self.last_update_conditions = 0.0;
        self.last_think_time = 0.0;
        self.update_waypoint_time = 0.0;
        self.kill = false;
        self.last_look_time = 0.0;
        self.move_to_valid = false;
        self.goal_waypoint = -1;
        self.current_waypoint = -1;
        self.prev_waypoint = -1;
        self.current_waypoint_flags = 0;
        self.prev_waypoint_dist = 4096.0;
        self.last_see_waypoint = 0.0;
        self.stuck_time = 0.0;
        self.unstick = Default::default();
        self.dive_time = 0.0;
        self.move_speed = 0.0;
        self.up_speed = 0.0;
        self.strafe_speed = 0.0;
        self.strafe_time = 0.0;
        self.start_jump_time = 0.0;
        self.end_jump_time = 0.0;
        self.start_duck_time = 0.0;
        self.end_duck_time = 0.0;
        self.update_visibles_time = 0.0;
        self.enemy = None;
        self.first_see_enemy_time = 0.0;
        self.get_aim_vector_time = 0.0;
        self.enemy_rep = None;
        self.conditions = 0;
        self.find_path_time = 0.0;
        self.avoid_entity = None;
        self.need_to_init = false;
        self.move_to = Vector::ZERO;
        self.look_task = LookTask::NextWaypoint;
        self.hurt_origin = Vector::ZERO;
        self.hurt_time = 0.0;
        self.ideal_move_speed = self.max_speed;
        self.touched_waypoints = 0;
    }

    /// `Think`'s TS weapon choice (`BOT_CONDITION_SELECTED_GUN`), bought unless
    /// `BOT_CONFIG_TS_KUNGFU`. Upstream's GA still picks its genes (they feed the stunt
    /// perceptron and breed as upstream), but the loadout bought is the bandit's (design §4).
    fn select_loadout(&mut self, g: &mut BotGlobals<'_>, rng: &mut dyn Random) {
        if self.has(cond::SELECTED_GUN) {
            return;
        }
        let _ga_picks = self.learned.select_ts_weapons(&mut g.learned.ts_weapon_choices, g.config.is_on(cfg::DISABLE_WEAPON_LEARN), rng);
        let loadout = self.choose_loadout(g, rng);
        if g.config.is_on(cfg::TS_KUNGFU) {
            self.learning.pending_loadout = None;
        } else {
            self.actions.push(Action::Loadout(loadout));
        }
        self.set(cond::SELECTED_GUN);
    }

    /// `CBot::Think`.
    pub(crate) fn think(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) {
        let now = w.time();
        let v = self.me.velocity + self.me.basevelocity;
        self.velocity_3d = v.length();
        self.velocity_2d = v.length_2d();
        self.look_for_new_tasks = true;
        self.move_speed = 0.0;
        if self.up_time < now {
            self.up_speed = 0.0;
        }
        let last_buttons = self.button;
        // `NotStartedGame` is false in TS; the port's rules team (from 1) is RCBot's team from 0.
        self.team = if w.teamplay() && self.state.team > 0 { self.state.team - 1 } else { -1 };

        if !self.paths.failed_goals.is_empty() && self.paths.next_clear_failed_goals < now {
            self.paths.failed_goals.clear();
        }
        self.paths.expire_backed_off_goals(now);
        self.squad_think(g, w);
        if !self.has(cond::WANT_TO_LEAVE_GAME) && g.config.is_on(cfg::BOTS_LEAVE_AND_JOIN) && self.want_to_leave_game(w, rng) {
            self.set(cond::WANT_TO_LEAVE_GAME);
        }

        if !self.is_alive() {
            if self.need_to_init {
                self.num_deaths += 1;
                let survival = now - self.spawn_time;
                let frags_since = self.me.frags as i32 - self.frags_since_death;
                let team = usize::try_from(self.team).ok().filter(|&t| t < 4).unwrap_or(0);
                let team_score_since = g.team_scores[team] - self.prev_team_score;
                self.prev_team_score = g.team_scores[team];
                let fitness = BotLearnedData::death_fitness(survival, frags_since, team_score_since);
                self.learn_on_death(g, now, rng);
                let used_melee = self.used_melee;
                self.learned.on_death(fitness, used_melee, &mut g.learned.ts_weapon_choices, &mut g.learned.enemy_costs[team], rng);
                self.spawn_init(g, rng);
                self.spawn_time = 0.0;
            }
            // The loadout is chosen before the spawn: the port's `buy` fills the next spawn's
            // loadout, and upstream's `UTIL_makeTSweapon` gave the picks at the first think of
            // the life, whose death then grades them (design §4).
            self.select_loadout(g, rng);
            if self.respawn_time < now {
                self.actions.push(Action::Command("respawn".into()));
                self.respawn_time = now + 5.0;
            }
            self.button = if rng.random_long(0, 1) != 0 { buttons::ATTACK } else { 0 };
            return;
        }

        if self.spawn_time == 0.0 {
            self.spawn_time = now;
            self.frags_since_death = self.me.frags as i32;
            self.prev_health = self.me.health;
            self.spawn_position = self.me.origin;
        }
        self.need_to_init = true;

        self.remembered.tick(now);
        self.hearing.tick(now);
        self.work_view_angles(w, rng);
        self.work_move_direction(w, g);
        self.check_stuck(w, g, rng);
        self.dive_landing(now);

        if self.last_think_time >= now {
            return;
        }
        self.last_think_time = now + self.think_time;

        // A bot the port handed over alive (`adopt_bot`) picks here, as upstream.
        self.select_loadout(g, rng);
        self.learn_items(w, now);

        self.ideal_move_speed = self.max_speed;
        self.want_to_find_enemy = !g.config.is_on(cfg::DONT_SHOOT);
        self.update_conditions(w);

        if self.update_visibles_time < now {
            self.update_visibles_time = now + self.profile.vision_time;
            if !self.update_visibles(g, w, rng) && self.next_work_range_costs < now {
                self.next_work_range_costs = now + 1.0;
            }
        }

        if last_buttons == self.button {
            self.button = 0;
        }

        // TS: run from a grenade that is coming (design §4: any within 400 units).
        self.flee_grenades(g, w, rng);

        // TS: reload or switch an empty gun. A gun with nothing in the clip or the reserve is
        // dropped for the next weapon with rounds, else kung fu (design §4; upstream kept
        // dry-firing it).
        if let Some(cur) = self.current_weapon.and_then(|id| self.weapons.get(id)).cloned()
            && !cur.is_melee()
        {
            if cur.out_of_ammo() && cur.max_clip > 0 {
                if !self.tasks.has_task(TaskKind::DropWeapon) {
                    self.add_priority_task(Task::simple(TaskKind::DropWeapon));
                }
            } else if cur.clip_is_empty() {
                if let Some(enemy) = self.resolve(w, self.enemy) {
                    // Within 600 units a reload over a second is worse than the next loaded
                    // weapon (design §4; upstream took `GetBestWeaponId`'s pick by priority).
                    let dist = self.distance_from(enemy.entity_origin(), false);
                    let slow = crate::weapons::ts_data::gun_data(cur.id).is_none_or(|d| d.reload > 1.0);
                    match self.best_scored_weapon(g, dist, Some(cur.id)) {
                        Some((best, _)) if (dist < 600.0 && slow) || !cur.can_reload() => {
                            self.switch_weapon(best);
                        }
                        _ if cur.can_reload() => self.reload(now),
                        _ => {}
                    }
                } else if cur.can_reload() && !cur.out_of_ammo() {
                    self.reload(now);
                }
            } else if self.enemy.is_none() && cur.low_on_ammo() && cur.can_reload() && self.safe_to_reload(now) {
                // Under half a clip with no enemy seen for a moment: reload now rather than
                // run dry in the next fight (design §4).
                self.reload(now);
            }
        }

        // Can the bot see where it is going?
        if self.current_waypoint == -1 || self.current_waypoint_flags & fl::LADDER == 0 {
            let tr = w.trace_line(self.me.origin, self.move_to, Monsters::Ignore, Glass::Ignore, Some(self.index));
            let mut see = tr.fraction >= 1.0;
            if !see
                && let Some(hit) = tr.hit.filter(|&h| h != 0).and_then(|h| w.entity(h))
                && hit.classname != "worldspawn"
                && (hit.velocity != Vector::ZERO || hit.classname.starts_with("func_door"))
            {
                // A door or something moving: walk through.
                see = true;
            }
            if see {
                self.set(cond::SEE_NEXT_WAYPOINT);
                self.last_see_waypoint = 0.0;
            } else {
                if self.has(cond::SEE_NEXT_WAYPOINT) {
                    self.last_see_waypoint = now;
                }
                self.clear(cond::SEE_NEXT_WAYPOINT);
            }
        } else if self.move_to_valid {
            self.last_see_waypoint = 0.0;
        }

        if let Some(t) = self.tasks.current().and_then(|t| t.edict).and_then(|e| self.resolve(w, Some(e))) {
            let tr = w.trace_line(self.me.origin, t.entity_origin(), Monsters::Ignore, Glass::Ignore, Some(self.index));
            if tr.fraction >= 1.0 {
                self.set(cond::SEE_TASK_EDICT);
            } else {
                self.clear(cond::SEE_TASK_EDICT);
            }
        }

        // Upstream waited half a second whenever there was no next waypoint, so a bot stood still
        // at every goal it reached; only a bot without any waypoint near waits now
        // (`update_waypoint`, design §4).
        if !self.follow_formation(g, w) && self.update_waypoint_time < now {
            self.update_waypoint(g, w, rng);
        }

        if self.enemy.is_none() {
            if self.climb_type() == super::Climb::None
                && self.player_standing_on_me(w).is_none()
                && self.standing_on_player(w).is_none()
                && self.listen_to_sound_time < now
            {
                self.listen_to_sound_time = now + self.reaction_time + rng.random_float(0.3, 0.75);
                // Gunfire heard (the port's `weapon_fired`, `Bot::hear_gunfire`) is glanced at
                // (upstream: a listen task, then a path to the shooter that ended once found;
                // design §4).
                if let Some(h) = self.hearing.newest_new().copied()
                    && now - h.time < 1.0
                {
                    self.watch(h.origin, now, 0.8);
                    self.drop_roaming_for_sound(now);
                }
            } else if self.chat_message_time < now {
                if !self.tasks.has_task(TaskKind::TypeMessage) {
                    self.chat_message_time = now + rng.random_float(10.0, 20.0);
                    if rng.random_long(0, 100) < i32::from(g.config.bot_chat_percent) {
                        self.bot_chat(g, w, crate::chat::bot_chat::ChatKind::Idle, None, false, rng);
                        self.getting_bored();
                    }
                }
            } else if self.last_see_enemy_time > 0.0 && self.last_see_enemy_time + 5.0 < now {
                if self.current_weapon.and_then(|id| self.weapons.get(id)).is_some_and(|w| w.can_reload() && w.out_of_ammo()) {
                    self.add_priority_task(Task::simple(TaskKind::Reload));
                }
                self.last_see_enemy_time = 0.0;
            }
        } else {
            self.chat_message_time = now + 5.0;
            self.last_see_enemy_time = now;
        }

        self.squad_modes(g);
        self.clear(cond::CANT_SHOOT);
        self.enemy = None;
        self.do_tasks(g, w, rng);
        self.learn_think(g, w);
        self.clear(cond::TASK_EDICT_PAIN);
        if self.look_for_new_tasks {
            self.look_for_new_tasks(g, w, rng);
        }
        self.update_pre_aim(g, now, rng);
        // `WantToFindEnemy`, TS: unless `dont_shoot` is on.
        self.enemy = if self.want_to_find_enemy { self.find_enemy(g, w, rng) } else { None };
        self.share_with_team(g, w);
    }

    /// The map's items ([`super::items::ItemMemory`]): the list now and then, and whether the ones
    /// in view are there.
    fn learn_items(&mut self, w: &dyn BotWorld, now: f32) {
        self.items.scan(w, now);
        // Items in sight the map's list doesn't name (a weapon a player dropped, say).
        for i in self.visibles.iter() {
            if let Some(e) = w.entity(i).filter(|e| e.item.is_some()) {
                self.items.learn(&e);
            }
        }
        let (forward, _) = make_vectors(self.v_angle);
        let eye = self.gun_position();
        let in_view = move |o: Vector| super::look::dot((o - eye).normalize(), forward) > 0.5 || (o - eye).length() < 200.0;
        self.items.look(w, eye, self.index, in_view, now);
    }

    /// `CBot::SwitchWeapon`, TS: the port's weapon switch (kung fu is the game's 0). The
    /// current weapon changes when the game reports it (upstream set it at once, so a refused
    /// switch left the bot thinking it held a weapon it didn't); the switch isn't asked again
    /// while it is pending.
    pub(crate) fn switch_weapon(&mut self, id: i32) -> bool {
        if self.current_weapon == Some(id) || self.switch_pending.is_some_and(|(p, _)| p == id) {
            return true;
        }
        let game_id = if id == TsWeapon::KungFu.id() { 0 } else { id };
        self.actions.push(Action::SelectWeapon(game_id));
        self.switch_pending = Some((id, self.now + SWITCH_WAIT));
        self.zoom = false;
        true
    }

    /// `UTIL_UpdateSounds`, from the port's `weapon_fired` (each shot, plugin API v6) instead of
    /// polling for `IN_ATTACK`: an enemy's shot from `origin` is heard within 1500 units, 350 with a
    /// silencer (upstream: the nearest player within 750 units, teammates included, whatever the
    /// gun; design §4), and remembered for the goals and the next glance.
    pub fn hear_gunfire(&mut self, w: &dyn BotWorld, shooter: &Entity, origin: Vector, silenced: bool) {
        use super::hearing::{GUNFIRE_RANGE, SILENCED_RANGE};
        let range = if silenced { SILENCED_RANGE } else { GUNFIRE_RANGE };
        if shooter.index == self.index || !self.is_alive() || (self.me.origin - origin).length() >= range || !self.is_enemy(w, shooter) {
            return;
        }
        self.hearing.add(origin, w.time(), super::hearing::LOUD_GUNFIRE, Some(shooter.handle()));
    }

    /// A sound was just heard: a bot only roaming drops that to choose again (the sound is
    /// among its goals); anything else it was doing goes on (upstream's priority listen task made
    /// every later task forget its path).
    fn drop_roaming_for_sound(&mut self, now: f32) {
        let Some(t) = self.tasks.current().copied() else { return };
        if t.kind == TaskKind::FindPath
            && t.float == -1.0
            && self.tasks.iter().filter(|x| x.schedule_id == t.schedule_id).count() == 1
            && t.edict.is_none()
        {
            let _ = now;
            self.tasks.finish_schedule(t.schedule_id);
            if t.schedule_id == 0 {
                self.tasks.finished_current(false);
            }
        }
    }

    /// `CBot::CanPickup`, TS: power-ups and ground weapons in reach, that are there, that the bot
    /// can take and wants ([`Bot::item_value`]: a weapon it has room for and that is better than
    /// what it has, a power-up it doesn't have to swap for), and that it didn't just fail to take.
    /// Upstream took any it saw, held or not, so a bot pressed use on one it couldn't take for
    /// 10 s, waited, and did it again (design §4).
    fn can_pickup(&self, g: &BotGlobals<'_>, now: f32, e: &Entity) -> bool {
        if self.pickup_item_time > now || e.effects & EF_NODRAW != 0 || e.origin.z > self.me.origin.z + MAX_JUMP_HEIGHT {
            return false;
        }
        if !(e.classname.starts_with("ts_powerup") || e.classname == "ts_groundweapon") || self.items.ignored(e.handle(), now) {
            return false;
        }
        match e.item {
            Some(i) if !i.available => false,
            Some(i) if i.kind == ItemKind::GroundWeapon => self.weapon_value(g, i.weapon) > 0.0,
            Some(i) if i.kind == ItemKind::Powerup => self.powerup_value(i.powerup) > 0.0,
            _ => true,
        }
    }

    /// `UTIL_IsFacingEntity`: the view line at the entity's distance ends inside its box.
    pub(crate) fn facing(&self, e: &Entity) -> bool {
        let (forward, _) = make_vectors(self.v_angle);
        let src = self.gun_position();
        let dst = src + forward * (e.entity_origin() - src).length();
        dst.x <= e.absmax.x
            && dst.y <= e.absmax.y
            && dst.z <= e.absmax.z
            && dst.x >= e.absmin.x
            && dst.y >= e.absmin.y
            && dst.z >= e.absmin.z
    }

    /// `LookForNewTasks`' TS objectives, by what each asks of the bot's team (design §4; upstream:
    /// a random one, team 1 `DEFEND`s it and team 2 `USE`s it, whoever it belongs to): capture
    /// the briefcase first on a CTB map ([`Bot::ctb_tasks`]); then an objective the bot can
    /// complete is gone for and used (a `ts_hack` / `ts_bomb` within 100 units of its waypoint, for
    /// 9 s, as upstream), one of the other team's is defended from a waypoint that sees it, for
    /// 10-20 s at a time. Sometimes the bot roams instead (1 in 8 attacking, 1 in 4 defending).
    /// False when there is nothing to do: the end-level waypoints come next.
    fn objective_tasks(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random, id: i32) -> bool {
        let now = w.time();
        if w.game_mode().ctb && self.ctb_tasks(w, id) {
            return true;
        }
        let objectives = w.ts_objectives(self.index);
        if objectives.is_empty() {
            return false;
        }
        // An attempt that left the objective as it was: this team can't complete it.
        if let Some((oid, until)) = self.objective_attempt
            && until < now
        {
            self.objective_attempt = None;
            if objectives.iter().any(|o| o.id == oid) {
                self.objective_failed.retain(|&(f, _)| f != oid);
                self.objective_failed.push((oid, now + 60.0));
            }
        }
        self.objective_failed.retain(|&(_, until)| until > now);
        let attack: Vec<&TsObjective> = objectives.iter().filter(|o| self.objective_role(w, o) == ObjectiveRole::Attack).collect();
        let (obj, role) = if !attack.is_empty() {
            (attack[rng.random_long(0, attack.len() as i32 - 1) as usize], ObjectiveRole::Attack)
        } else {
            (&objectives[rng.random_long(0, objectives.len() as i32 - 1) as usize], ObjectiveRole::Defend)
        };
        let roam = match role {
            ObjectiveRole::Attack => rng.random_long(0, 7) == 0,
            ObjectiveRole::Defend => rng.random_long(0, 3) == 0,
        };
        let wpt = self.nearest_waypoint(g, w, obj.origin, REACHABLE_RANGE, self.last_failed_waypoint, false, false, true);
        if roam || wpt == -1 {
            return false;
        }
        match role {
            ObjectiveRole::Attack => {
                let at = g.waypoints.origin(wpt);
                let target = w.entities_in_sphere(at, 100.0).into_iter().find_map(|i| {
                    let e = w.entity(i)?;
                    (e.classname == "ts_hack" || e.classname == "ts_bomb").then_some(e)
                });
                self.add_task(Task::new(TaskKind::FindPath, id, None, wpt, -1.0, Vector::ZERO));
                match target {
                    Some(t) => {
                        self.add_task(Task::new(TaskKind::Use, id, Some(t.handle()), 0, 9.0, Vector::ZERO));
                        self.objective_attempt = Some((obj.id, now + 30.0));
                    }
                    None => self.add_task(Task::new(TaskKind::Defend, id, None, 0, 0.0, obj.origin)),
                }
            }
            ObjectiveRole::Defend => {
                let spot = self.defend_spot(g, wpt, rng);
                self.add_task(Task::new(TaskKind::FindPath, id, None, spot, -1.0, Vector::ZERO));
                self.add_task(Task::new(TaskKind::Defend, id, None, 0, 0.0, obj.origin));
            }
        }
        true
    }

    /// Whether the bot's team completes an objective or keeps the other team from it: the team
    /// the port gives the objective's entity, if any; else an objective whose use failed lately is
    /// defended, and so is one whose text for the bot's team is empty or says to defend it.
    fn objective_role(&self, w: &dyn BotWorld, o: &TsObjective) -> ObjectiveRole {
        if let Some(team) = o.entity.and_then(|e| w.entity(e)).and_then(|e| e.item).map(|i| i.team).filter(|&t| t != 0) {
            return if team == self.state.team { ObjectiveRole::Attack } else { ObjectiveRole::Defend };
        }
        if self.objective_failed.iter().any(|&(f, _)| f == o.id) {
            return ObjectiveRole::Defend;
        }
        let text = o.name.to_ascii_lowercase();
        if text.is_empty() || ["defend", "protect", "guard", "prevent", "stop", "hold"].iter().any(|k| text.contains(k)) {
            ObjectiveRole::Defend
        } else {
            ObjectiveRole::Attack
        }
    }

    /// A waypoint to defend `target` (a waypoint) from: 200-900 units from it, seeing it, not the
    /// one the bot is at (defenders rotate); `target` itself if none.
    fn defend_spot(&self, g: &BotGlobals<'_>, target: i32, rng: &mut dyn Random) -> i32 {
        let Ok(t) = usize::try_from(target) else { return target };
        let wp = &*g.waypoints;
        let to = wp.origin(target);
        let spots: Vec<usize> = (0..wp.len())
            .filter(|&i| i != t && i as i32 != self.current_waypoint)
            .filter(|&i| {
                wp.flags(i as i32) & (fl::DELETED | fl::AIMING | fl::UNREACHABLE) == 0 && fl::team_allows(wp.flags(i as i32), self.team)
            })
            .filter(|&i| (200.0..900.0).contains(&(wp.origin(i as i32) - to).length()) && wp.visibility.get(i, t))
            .filter(|&i| self.claim_factor(g, i) >= 1.0)
            .collect();
        if spots.is_empty() {
            return target;
        }
        spots[rng.random_long(0, spots.len() as i32 - 1) as usize] as i32
    }

    /// Capture the briefcase (`ts_flag` cases, `ts_briefcase` capture points; design §4; upstream
    /// had none for TS): a bot carrying the case takes it to a point its team may capture at, a
    /// teammate's carrier is escorted, an enemy carrier is cut off at the point it must reach, and
    /// otherwise the bot goes for a case its team may take. False on a map without them.
    fn ctb_tasks(&mut self, w: &dyn BotWorld, id: i32) -> bool {
        let now = w.time();
        let team = self.state.team;
        let of_kind = |class: &str, kind: ItemKind| -> Vec<Entity> {
            w.find_entities(super::EntityKey::Classname, class)
                .into_iter()
                .filter_map(|i| w.entity(i))
                .filter(|e| e.item.is_some_and(|it| it.kind == kind))
                .collect()
        };
        let cases = of_kind("ts_flag", ItemKind::CtbFlag);
        let bases = of_kind("ts_briefcase", ItemKind::CtbBase);
        let nearest = |list: &[Entity], from: Vector, ok: &dyn Fn(&Entity) -> bool| -> Option<Entity> {
            list.iter()
                .filter(|e| ok(e))
                .min_by(|a, b| (a.entity_origin() - from).length().total_cmp(&(b.entity_origin() - from).length()))
                .cloned()
        };
        let base_for = |t: i32| move |b: &Entity| b.item.is_some_and(|i| i.team == t || i.team == 0);
        if self.state.has_flag
            && let Some(base) = nearest(&bases, self.me.origin, &base_for(team))
        {
            self.add_task(Task::new(TaskKind::FindPath, id, Some(base.handle()), 0, 0.0, Vector::ZERO).with_timeout(now, 60.0));
            self.add_task(Task::new(TaskKind::MoveToVector, id, Some(base.handle()), 0, 0.0, Vector::ZERO));
            return true;
        }
        let carrier = (1..=w.max_clients())
            .filter(|&i| i != self.index)
            .filter_map(|i| w.entity(i))
            .find(|p| p.is_alive() && p.player.is_some_and(|s| s.has_flag));
        if let Some(c) = carrier {
            let their_team = c.player.map_or(0, |s| s.team);
            if their_team == team && team != 0 {
                // Escort: keep near the carrier.
                if self.distance_from(c.origin, false) > 300.0 {
                    self.add_task(Task::new(TaskKind::FindPath, id, Some(c.handle()), 0, 0.0, Vector::ZERO).with_timeout(now, 8.0));
                } else {
                    self.add_task(Task::new(TaskKind::Defend, id, None, 0, 4.0, c.origin));
                }
                return true;
            }
            if let Some(base) = nearest(&bases, c.origin, &base_for(their_team)) {
                self.add_task(Task::new(TaskKind::FindPath, id, Some(base.handle()), 0, 0.0, Vector::ZERO).with_timeout(now, 30.0));
                self.add_task(Task::new(TaskKind::Defend, id, None, 0, 0.0, base.entity_origin()));
                return true;
            }
        }
        let takeable = |c: &Entity| c.effects & EF_NODRAW == 0 && c.item.is_some_and(|i| i.available && (i.team == 0 || i.team != team));
        if let Some(case) = nearest(&cases, self.me.origin, &takeable) {
            self.add_task(Task::new(TaskKind::FindPath, id, Some(case.handle()), 0, 0.0, Vector::ZERO).with_timeout(now, 60.0));
            self.add_task(Task::new(TaskKind::MoveToVector, id, Some(case.handle()), 0, 0.0, Vector::ZERO));
            return true;
        }
        false
    }

    /// `CBot::LookForNewTasks`, TS. Upstream picked up the nearest weapon or power-up in sight,
    /// went for a random objective, or roamed (following the squad leader, or half the time to the
    /// newest remembered enemy position whatever its age, else to a random goal). The port scores
    /// its goals ([`Bot::choose_goal`], design §4): an item worth the way, an enemy seen lately
    /// (to where it may have gone, then a moving search), a sound heard; then the objectives, the
    /// end-level waypoints in teamplay, the squad leader, and roaming.
    pub(crate) fn look_for_new_tasks(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) {
        let now = w.time();
        // Upstream stopped here, so a bot stood still for a moment at every goal it reached; the
        // tasks below give it its next move (design §4).
        self.look_task =
            if self.resolve(w, self.last_enemy).is_some_and(|e| e.is_alive()) { LookTask::SearchForLastEnemy } else { LookTask::PreAim };
        let id = self.tasks.new_schedule_id();
        self.looking_for_enemy = false;
        // A new goal's path may be searched at once (upstream waited out the last search's half
        // second, standing at the goal it had reached).
        self.find_path_time = 0.0;
        let (goal, _) = self.choose_goal(g, w, now);
        match goal {
            Goal::Item(item) => {
                if !g.config.is_on(cfg::TS_DONT_PICKUP_WEAPONS)
                    && let Some(p) = self.resolve(w, Some(item))
                    && self.can_pickup(g, now, &p)
                {
                    let h = Some(p.handle());
                    self.add_task(Task::new(TaskKind::FindPath, id, h, 0, 0.0, Vector::ZERO));
                    self.add_task(Task::new(TaskKind::PickupItem, id, h, 0, 0.0, Vector::ZERO));
                    self.add_task(Task::new(TaskKind::Use, id, h, 0, 0.0, Vector::ZERO));
                    self.claim(g, -1, Some(item));
                    return;
                }
            }
            Goal::Hunt(origin, seen) => {
                self.remembered.hunted(origin);
                let wpt = self.predict_enemy(g, w, origin, now - seen, rng);
                if wpt != -1 && !self.paths.ignored_goals(now).contains(&(wpt as usize)) {
                    self.add_task(Task::new(TaskKind::FindPath, id, None, wpt, -1.0, Vector::ZERO).with_timeout(now, HUNT_TIME));
                    let search = rng.random_float(3.0, 6.0);
                    self.add_task(Task::new(TaskKind::SearchForEnemy, id, None, 0, search, g.waypoints.origin(wpt)));
                    self.looking_for_enemy = true;
                    self.claim(g, wpt, None);
                    return;
                }
            }
            Goal::Investigate(origin) => {
                self.hearing.investigate(origin);
                self.add_task(Task::new(TaskKind::FindPath, id, None, 0, 0.0, origin).with_timeout(now, HUNT_TIME));
                let search = rng.random_float(3.0, 5.0);
                self.add_task(Task::new(TaskKind::SearchForEnemy, id, None, 0, search, origin));
                self.looking_for_enemy = true;
                let wpt = self.nearest_waypoint(g, w, origin, 400.0, -1, false, false, false);
                self.claim(g, wpt, None);
                return;
            }
            Goal::Hold(spot) => {
                self.hold_again = now + rng.random_float(15.0, 30.0);
                self.add_task(Task::new(TaskKind::FindPath, id, None, spot, -1.0, Vector::ZERO).with_timeout(now, HUNT_TIME));
                self.add_task(Task::new(TaskKind::Defend, id, None, 0, 0.0, g.waypoints.origin(spot)));
                self.claim(g, spot, None);
                return;
            }
            Goal::Regroup(mate) => {
                self.add_task(Task::new(TaskKind::FindPath, id, None, 0, 0.0, mate).with_timeout(now, HUNT_TIME));
                return;
            }
            Goal::Roam => {}
        }
        if w.teamplay() && self.objective_tasks(g, w, rng, id) {
            return;
        }
        if w.teamplay() {
            let wpt = g
                .waypoints
                .find_random_goal_near(self.me.origin, 4096.0, -1, fl::ENDLEVEL, &self.paths.failed_goals, rng)
                .map_or(-1, |i| i as i32);
            if self.current_waypoint != wpt && wpt != -1 {
                self.add_task(Task::new(TaskKind::FindPath, 0, None, wpt, -1.0, Vector::ZERO));
                return;
            }
        }
        // In a squad (one a player made: TS bots never assemble their own), follow the leader.
        if self.squad_leader.is_some() {
            self.add_task(Task::new(TaskKind::FollowLeader, id, None, 0, 0.0, Vector::ZERO));
            return;
        }
        if self.tasks.is_empty() {
            self.add_priority_task(Task::new(TaskKind::FindPath, id, None, -1, -1.0, Vector::ZERO));
        }
    }

    /// `CBot::DoTasks`: runs the current task, then finishes or fails it.
    fn do_tasks(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) {
        let now = w.time();
        if let Some(t) = self.tasks.current() {
            if t.timed_out(now) {
                let id = t.schedule_id;
                self.tasks.finish_schedule(id);
            } else {
                self.tasks.remove_timed_out_schedules(now);
            }
        }
        let Some(task) = self.tasks.current().copied() else { return };
        let mut done = false;
        let mut path_found = false;
        let mut failed = false;
        let mut to_add: Option<Task> = None;
        self.look_for_new_tasks = false;

        match task.kind {
            TaskKind::None => {}
            TaskKind::FindPath => self.task_find_path(g, w, rng, task, &mut done, &mut failed, &mut path_found),
            TaskKind::NormalAttack => self.task_normal_attack(g, w, rng, task, &mut done, &mut failed, &mut to_add),
            TaskKind::SearchForEnemy => {
                if self.enemy.is_some() || self.has(cond::SEE_ENEMY) {
                    failed = true;
                } else {
                    self.look_task = LookTask::SearchForEnemy;
                    if task.vector == Vector::ZERO {
                        self.stop_moving();
                    } else {
                        // Searching around a place, on the move (design §4; upstream stood still).
                        self.search_move(g, w, task.vector, rng);
                    }
                    if task.int == 0 {
                        let t = self.tasks.current_mut().expect("current task");
                        t.float += now;
                        t.int = 1;
                        self.search_enemy_time = t.float;
                    } else if task.float <= now {
                        done = true;
                    }
                }
            }
            TaskKind::Reload => {
                self.set(cond::CANT_SHOOT);
                match self.current_weapon {
                    None => failed = true,
                    Some(id) if !self.weapons.has(id) => {
                        failed = true;
                        self.current_weapon = None;
                    }
                    // Nothing to reload (a full clip, no reserve, kung fu or a blade, whose
                    // reload key blocks): done at once instead of holding the key (design §4).
                    Some(id)
                        if !self.weapons.get(id).is_some_and(|wp| wp.need_to_reload() || (wp.reserve > 0 && wp.clip < wp.max_clip)) =>
                    {
                        done = true;
                    }
                    Some(_) => {
                        if self.last_enemy.is_some() {
                            self.look_task = LookTask::SearchForLastEnemy;
                            self.search_enemy_time = now + 0.5;
                        }
                        if rng.random_long(0, 100) > 0 {
                            self.reload(now);
                        }
                        if task.int == 0 {
                            let f = now + rng.random_float(0.5, 1.5);
                            let t = self.tasks.current_mut().expect("current task");
                            t.float = f;
                            t.int = 1;
                        } else if task.float < now {
                            done = true;
                        }
                    }
                }
            }
            TaskKind::WaitAndFaceVector | TaskKind::FaceVector => {
                if task.kind == TaskKind::WaitAndFaceVector && (self.enemy.is_some() || self.has(cond::SEE_ENEMY)) {
                    failed = true;
                } else {
                    self.look_task = LookTask::FaceTaskVector;
                    if task.kind == TaskKind::WaitAndFaceVector {
                        self.stop_moving();
                    }
                    if task.int == 1 {
                        if task.float < now {
                            done = true;
                        }
                    } else {
                        let t = self.tasks.current_mut().expect("current task");
                        t.float += now;
                        t.int = 1;
                    }
                }
            }
            TaskKind::MoveToVector => {
                self.look_task = LookTask::FaceTaskVector;
                let target = self.resolve(w, task.edict);
                let origin = match &target {
                    Some(e) => {
                        let o = e.entity_origin();
                        let d = (o - self.me.origin).length_2d();
                        if d <= e.size.length_2d() || d < self.distance_from(task.vector, false) {
                            done = true;
                        }
                        o
                    }
                    None => {
                        if self.distance_from(task.vector, false) < task.float {
                            done = true;
                        }
                        task.vector
                    }
                };
                if !done {
                    if (origin - self.me.origin).length_2d() <= 100.0 {
                        self.stop_moving();
                        done = true;
                    } else if self.has(cond::SEE_NEXT_WAYPOINT) {
                        if origin.z > self.me.origin.z + MAX_JUMP_HEIGHT {
                            failed = true;
                        }
                        self.set_move_vector(origin);
                    } else {
                        failed = true;
                    }
                }
            }
            TaskKind::PickupItem => {
                if task.int == 0 {
                    self.pickup_entity = task.edict;
                    let t = self.tasks.current_mut().expect("current task");
                    t.int = 1;
                    t.set_time_to_complete(now, 10.0);
                }
                match self.resolve(w, self.pickup_entity) {
                    Some(p) if self.can_pickup(g, now, &p) => {
                        self.set_move_vector(p.entity_origin());
                        if self.distance_from(p.entity_origin(), false) < 64.0 {
                            self.use_button();
                            // Pressing use on it for a while and it's still there: the game
                            // won't give it (full, or a power-up held). Leave it alone instead of
                            // trying for the rest of the 10 s, and again later (design §4).
                            let t = self.tasks.current_mut().expect("current task");
                            if t.float == 0.0 {
                                t.float = now;
                            } else if now - t.float > PICKUP_TRIES {
                                self.items.untakeable(p.handle(), now);
                                failed = true;
                            }
                        }
                    }
                    _ => failed = true,
                }
            }
            TaskKind::Use => self.task_use(w, rng, task, &mut done, &mut failed),
            TaskKind::Wait => {
                if task.int == 0 {
                    let t = self.tasks.current_mut().expect("current task");
                    t.float += now;
                    t.int = 1;
                }
                if self.tasks.current().is_some_and(|t| t.float < now) {
                    done = true;
                } else {
                    self.stop_moving();
                }
            }
            TaskKind::Defend => {
                // Held for 10-20 s (a set time when given), then the bot moves to another spot
                // (upstream: 20-80 s, diving until prone, which standing still does nothing in TS
                // 3.0 outside slow motion), watching the approaches (design §4).
                if task.int == 0 {
                    let secs = if task.float > 0.0 { task.float } else { rng.random_float(10.0, 20.0) };
                    let t = self.tasks.current_mut().expect("current task");
                    t.float = now + secs;
                    t.int = 1;
                }
                if self.tasks.current().is_some_and(|t| t.float < now) {
                    done = true;
                }
                self.stop_moving();
                if self.enemy.is_none() {
                    self.look_task = LookTask::PreAim;
                }
            }
            TaskKind::ListenToSound => self.task_listen(w, rng, task, &mut done, &mut failed, &mut to_add),
            TaskKind::ChangeWeapon => {
                self.set(cond::CANT_SHOOT);
                if !self.switch_weapon(task.int) {
                    failed = true;
                }
                done = true;
            }
            TaskKind::DropWeapon => {
                // The port's `drop` drops the gun in hand; forget it so nothing picks it again.
                // TS then takes out the next weapon the player owns (`GetBetterWeapon`), which
                // the bot may never have held and so can't know: only a gun it knows has rounds
                // is chosen here; otherwise the game's choice stands (its `WeaponInfo` follows).
                if let Some(id) = self.current_weapon.filter(|&id| id != TsWeapon::KungFu.id()) {
                    self.actions.push(Action::Command("drop".into()));
                    self.weapons.remove(id);
                    self.current_weapon = None;
                    self.next_inventory_sync = 0.0;
                    if let Some(next) = self.weapons.best_loaded_gun() {
                        self.switch_weapon(next);
                    }
                }
                done = true;
            }
            TaskKind::PushPushable => self.task_push_pushable(w, rng, task, &mut done, &mut failed),
            TaskKind::WaitForLift => self.task_wait_for_lift(w, task, &mut done),
            TaskKind::WaitForEntity => self.task_wait_for_entity(w, task, &mut done, &mut failed),
            TaskKind::HumanTower => self.task_human_tower(w, rng, task, &mut done),
            TaskKind::ThrowGrenade => self.task_throw_grenade(w, task, &mut done, &mut failed),
            TaskKind::WaitForBotAtWpt => self.task_wait_for_bot_at_wpt(g, w, rng, task, &mut done, &mut failed),
            TaskKind::FaceEdict => self.task_face_edict(w, task, &mut done, &mut failed),
            TaskKind::FollowLeader => self.task_follow_leader(g, w, task, &mut failed),
            TaskKind::TypeMessage => self.task_type_message(w, rng, task, &mut done, &mut failed),
            TaskKind::Crouch => {
                self.duck();
                if task.int == 0 {
                    let t = self.tasks.current_mut().expect("current task");
                    t.float += now;
                    t.int = 1;
                } else if task.float < now {
                    done = true;
                }
            }
        }

        if let Some(cur) = self.tasks.current().copied() {
            if failed {
                if cur.kind == TaskKind::FindPath {
                    self.last_failed_waypoint = -1;
                }
                done = true;
            }
            if cur.kind == TaskKind::FindPath && !self.tasks.is_empty() && !cur.found_path {
                if path_found {
                    self.move_to_valid = true;
                }
                if let Some(t) = self.tasks.current_mut() {
                    t.found_path = path_found;
                }
            }
            if done {
                self.tasks.finished_current(failed);
                if cur.kind == TaskKind::NormalAttack {
                    self.enemy = None;
                }
            }
        }
        if let Some(t) = to_add {
            self.add_priority_task(t);
        }
    }

    /// `BOT_TASK_FIND_PATH`.
    #[allow(clippy::too_many_arguments)] // DoTasks' locals
    fn task_find_path(
        &mut self,
        g: &mut BotGlobals<'_>,
        w: &dyn BotWorld,
        rng: &mut dyn Random,
        task: Task,
        done: &mut bool,
        failed: &mut bool,
        path_found: &mut bool,
    ) {
        let now = w.time();
        let target = self.resolve(w, task.edict);
        self.ideal_move_speed = self.max_speed;
        if !task.found_path {
            if self.paths.bot_paths.is_empty() && !self.not_following_waypoint {
                self.stop_moving();
            }
            let query = |b: &Bot, from: i32, to: i32| crate::nav::AStarQuery {
                from: usize::try_from(from).ok(),
                to: usize::try_from(to).ok(),
                gun_position: b.gun_position(),
                team: b.team,
                max_loops: b.profile.path_revs.max(1),
                time: now,
            };
            if task.float == -3.0 {
                if self.current_waypoint == -1 {
                    self.current_waypoint =
                        self.nearest_waypoint(g, w, self.me.origin, REACHABLE_RANGE, self.last_failed_waypoint, true, false, true);
                }
                let q = query(self, self.current_waypoint, self.goal_waypoint);
                match self.search_path(g, w, rng, &q, true) {
                    crate::nav::AStarResult::NoPath => {
                        *failed = true;
                        return;
                    }
                    crate::nav::AStarResult::Continue => return,
                    crate::nav::AStarResult::Next(_) => {}
                }
                self.push_current_on_path();
                self.move_to_valid = true;
                *path_found = true;
                self.path_deadline(now);
                let t = self.tasks.current_mut().expect("current task");
                t.found_path = true;
                self.paths.failed_goals.clear();
                if t.float == -2.0 {
                    *done = true;
                }
                t.float = 1.0;
            } else if task.float == -2.0 || self.find_path_time < now {
                self.find_path_time = now + 0.5;
                if task.float == -1.0 && task.int == -1 {
                    let goal = self.roam_goal(g, now, rng);
                    self.goal_waypoint = goal;
                    let t = self.tasks.current_mut().expect("current task");
                    t.int = goal;
                    if goal == -1 {
                        *failed = true;
                        return;
                    }
                    t.vector = g.waypoints.origin(goal);
                } else if let Some(e) = &target {
                    let o = e.entity_origin();
                    self.tasks.current_mut().expect("current task").vector = o;
                    self.goal_waypoint = self.nearest_to_origin(g, w, o);
                } else if task.float == 0.0 {
                    self.tasks.current_mut().expect("current task").float = 1.0;
                    self.goal_waypoint = self.nearest_to_origin(g, w, task.vector);
                } else {
                    self.goal_waypoint = task.int;
                }
                if self.goal_waypoint == -1 {
                    *failed = true;
                    return;
                }
                self.current_waypoint =
                    self.nearest_waypoint(g, w, self.me.origin, REACHABLE_RANGE, self.last_failed_waypoint, true, false, false);
                let goal = self.goal_waypoint;
                {
                    let t = self.tasks.current_mut().expect("current task");
                    t.int = goal;
                    if t.vector == Vector::ZERO {
                        t.vector = g.waypoints.origin(goal);
                    }
                }
                if self.current_waypoint == -1 {
                    *failed = true;
                    return;
                }
                self.goal_origin = self.tasks.current().map(|t| t.vector);
                if self.goal_waypoint == self.current_waypoint {
                    self.paths.bot_paths.push(self.current_waypoint as usize);
                    self.move_to_valid = true;
                    *path_found = true;
                    return;
                }
                let q = query(self, self.current_waypoint, self.goal_waypoint);
                match self.search_path(g, w, rng, &q, false) {
                    crate::nav::AStarResult::NoPath => {
                        *failed = true;
                        return;
                    }
                    crate::nav::AStarResult::Continue => {
                        self.tasks.current_mut().expect("current task").float = -3.0;
                        return;
                    }
                    crate::nav::AStarResult::Next(_) => {}
                }
                self.push_current_on_path();
                self.move_to_valid = true;
                *path_found = true;
                self.path_deadline(now);
                if task.float == -2.0 {
                    *done = true;
                }
            }
        } else {
            if self.nearest_remember_point_visible && self.enemy.is_none() && self.looking_for_enemy {
                self.look_task = LookTask::FaceNearestRememberPos;
            } else if self.resolve(w, self.last_enemy).is_some_and(|e| e.is_alive()) && self.last_see_enemy_time + 6.0 > now {
                self.look_task = LookTask::SearchForLastEnemy;
                self.search_enemy_time = now + 0.5;
            } else {
                // Upstream looked where it walked; the port checks where enemies may appear.
                self.look_task = LookTask::PreAim;
            }
            if self.current_waypoint == -1 || self.goal_waypoint != task.int || task.int == -1 {
                self.tasks.current_mut().expect("current task").found_path = false;
                return;
            }
            if let Some(e) = &target
                && self.goal_waypoint == self.current_waypoint
                && self.distance_from(e.entity_origin(), false) < g.waypoints.distance(self.gun_position(), self.goal_waypoint)
            {
                *done = true;
            }
            self.move_to_valid = true;
        }
    }

    /// A path just found must be walked within [`PATH_TIMEOUT`] unless the task has a time
    /// already: upstream's had none until the first waypoint was touched (then 20-30 s from each
    /// touch), so a bot that never reached it kept the task for ever (design §4).
    fn path_deadline(&mut self, now: f32) {
        if let Some(t) = self.tasks.current_mut()
            && t.kind == TaskKind::FindPath
            && t.time_to_complete == -1.0
        {
            t.set_time_to_complete(now, PATH_TIMEOUT);
        }
    }

    /// `if (m_stBotPaths.IsEmpty() || top != current) push(current)`.
    fn push_current_on_path(&mut self) {
        let cur = self.current_waypoint as usize;
        if self.paths.bot_paths.last() != Some(&cur) {
            self.paths.bot_paths.push(cur);
        }
    }

    /// `NearestWaypointToOrigin(v, m_iLastFailedWaypoint, &m_FailedGoals)`.
    fn nearest_to_origin(&self, g: &BotGlobals<'_>, w: &dyn BotWorld, v: Vector) -> i32 {
        g.waypoints
            .nearest_to_origin(v, usize::try_from(self.last_failed_waypoint).ok(), &self.paths.failed_goals, w)
            .map_or(-1, |i| i as i32)
    }

    /// `BOT_TASK_NORMAL_ATTACK`, TS.
    #[allow(clippy::too_many_arguments)] // DoTasks' locals
    fn task_normal_attack(
        &mut self,
        g: &mut BotGlobals<'_>,
        w: &dyn BotWorld,
        rng: &mut dyn Random,
        task: Task,
        done: &mut bool,
        failed: &mut bool,
        to_add: &mut Option<Task>,
    ) {
        let now = w.time();
        self.enemy = task.edict;
        let Some(enemy) = self.resolve(w, task.edict) else {
            self.last_enemy = None;
            *done = true;
            return;
        };
        if !self.is_enemy(w, &enemy) {
            self.last_enemy = None;
            self.enemy = None;
            *done = true;
            return;
        }
        if task.float != 0.0 {
            self.stop_moving();
            if task.float < now {
                *done = true;
                return;
            }
        }
        let _ = to_add;
        // `SEE_ENEMY` can be stale: set by the last-seen-position check below, by `EnemyFound`,
        // or kept until the vision pass reaches the enemy again. Upstream then aimed and fired
        // at the enemy's current origin through walls; the port asks for a clear line to the
        // enemy itself every time (design §4). `FVisible` clears the condition when blocked.
        if self.has(cond::SEE_ENEMY) {
            self.fvisible(w, &enemy);
        } else if self.kungfu_face(now).is_some() && self.fvisible(w, &enemy) {
            // A kung fu wall jump turns the bot's back to the opponent for a moment: it still
            // knows where the opponent is (outside the view cone the vision pass needs).
            self.set(cond::SEE_ENEMY);
        }
        if self.has(cond::SEE_ENEMY) {
            let enemy_origin = enemy.entity_origin();
            self.look_task = LookTask::FaceEnemy;
            let dist = (enemy_origin - self.gun_position()).length();
            self.last_enemy = self.enemy;
            self.last_see_enemy_position = enemy_origin;
            self.saw_enemy(&enemy, now);
            self.use_powerup(dist, now);
            let cur = self.current_weapon.and_then(|id| self.weapons.get(id)).cloned();
            if cur.as_ref().is_some_and(|c| c.is_melee()) && self.me.movetype == super::MOVETYPE_FLY && rng.random_long(0, 1) != 0 {
                self.jump();
            }
            let weap_id = cur.as_ref().map_or(0, |c| c.id);
            // TS's melee weapons: combat knife (25), katana (34), seal knife (35) and kung fu.
            // Upstream lists 36, the Contender in TS 3.0, for kung fu (design §5).
            let melee = cur.is_none() || matches!(weap_id, 25 | 34 | 35) || weap_id == TsWeapon::KungFu.id();
            if weap_id == TsWeapon::KungFu.id() {
                // The port's kung fu fighter (design §4) in place of upstream's random punches
                // and kicks.
                self.kungfu_fight(g, w, rng, &enemy, dist);
            } else if melee {
                // Upstream first moves to `m_vLowestEnemyCostVec` when `WaypointReachable` says
                // the enemy can't be walked to, then overwrites that with the enemy's origin on
                // the next line: only the second counts, so the check (40+ traces) is left out.
                self.set_move_vector(enemy_origin);
                if dist < 256.0 {
                    self.decide_jump_duck_strafe(w, dist, enemy_origin);
                }
                // TS 3.0's `+attack2` throws a knife and the katana: slash with `+attack`, and
                // throw only a spare knife at a target out of reach (design §4; upstream pressed
                // `+attack2` nine times in ten within 72 units).
                if dist < 72.0 {
                    self.primary_attack();
                    if !g.config.is_on(cfg::TS_DONT_STEAL_WEAPONS) {
                        self.use_button();
                    }
                } else if matches!(weap_id, 25 | 35)
                    && (150.0..500.0).contains(&dist)
                    && cur.as_ref().is_some_and(|c| c.clip > 1)
                    && self.in_aim(&enemy, dist)
                {
                    self.secondary_attack();
                }
            } else {
                let l = &mut self.learned;
                l.stunt.set_weights_from_values(&l.ts_weapon_select, 3, 5);
                let inputs = [
                    self.state.stunt_state as f32 / 2.0,
                    self.me.velocity.length() / self.max_speed,
                    enemy.velocity.length() / enemy.maxspeed,
                    self.yaw_angle_to(enemy_origin) / 180.0,
                    weap_id as f32 / crate::weapons::MAX_WEAPONS as f32,
                ];
                let l = &mut self.learned;
                l.stunt.input(&inputs);
                l.stunt.execute();
                // The perceptron no longer presses `+alt1` every think it fires (each press a
                // dive in TS 3.0): it is one way a dodge dive starts (design §4).
                let stunt = l.stunt.fired();
                self.dodge(&enemy, stunt, now, rng);
                // Losing the trade (under 38% health, the enemy healthier): break off to cover.
                if self.me.health < self.me.max_health * 0.38 && enemy.health > self.me.health {
                    self.run_for_cover(g, w, enemy_origin, false, rng);
                }
                self.choose_weapon(g, dist, now);
                self.fight.spread_limit = (self.spread_limit(dist), now + 0.1);
                if cur.as_ref().is_some_and(|c| c.need_to_reload()) {
                    self.run_for_cover(g, w, enemy_origin, false, rng);
                    self.reload(now);
                } else if self.hold_attack_time > now || self.ready_to_fire(&enemy, dist, now) {
                    self.primary_attack();
                }
                self.combat_move(enemy_origin, dist, now, rng);
            }
        } else if self.last_enemy == self.enemy {
            // `HasSeenEnemy`.
            let last = self.last_see_enemy_position;
            self.last_enemy = self.enemy;
            self.search_enemy_time = now + rng.random_float(5.0, 10.0);
            self.remember_lost_enemy(last, self.enemy);
            self.grenade_behind_cover(now, rng);
            if self.hold_angle(now, rng) {
                // Hold the angle where it went, aiming where it will show (design §4).
                self.stop_moving();
                self.strafe_time = 0.0;
                self.look_task = LookTask::FaceEnemy;
            } else if self.fvisible_point(w, last) && (self.fight.last_sight == 0.0 || self.fight.last_sight + 1.0 > now) {
                // Upstream kept facing while the last seen position was in sight, for as long as
                // it was; the bot goes after the enemy a second after losing it (design §4).
                self.set(cond::SEE_ENEMY);
                self.look_task = LookTask::FaceEnemy;
            } else if !self.want_to_follow_enemy() {
                *failed = true;
            } else if self.distance_from(last, false) > 100.0 {
                self.tasks.finished_current(false);
                let id = self.tasks.new_schedule_id();
                if !self.tasks.has_task(TaskKind::SearchForEnemy) {
                    let f = rng.random_float(3.0, 6.0);
                    self.add_priority_task(Task::new(TaskKind::SearchForEnemy, id, None, 0, f, Vector::ZERO));
                    let secs = rng.random_float(10.0, 20.0);
                    self.add_priority_task(Task::new(TaskKind::FindPath, id, None, 0, 0.0, last).with_timeout(now, secs));
                }
            } else {
                self.look_task = LookTask::SearchForEnemy;
                *failed = true;
            }
        } else {
            self.look_task = LookTask::SearchForLastEnemy;
            self.enemy = None;
            self.last_enemy = None;
            *done = true;
        }
    }

    /// `BOT_TASK_USE`.
    pub(crate) fn task_use(&mut self, w: &dyn BotWorld, _rng: &mut dyn Random, task: Task, done: &mut bool, failed: &mut bool) {
        let now = w.time();
        if task.int != -2 && task.int != -1 && !task.found_path {
            *failed = true;
            return;
        }
        let Some(e) = self.resolve(w, task.edict) else {
            *failed = true;
            return;
        };
        self.look_task = LookTask::FaceTaskEdict;
        self.stop_moving();
        if task.int == 1 {
            let mut until = task.vector.x;
            if until == 0.0 {
                until = now + task.float;
                self.tasks.current_mut().expect("current task").vector = Vector::new(until, 0.0, 0.0);
            }
            self.use_button();
            if until > now {
                // A momentary button is held until what it targets stops moving.
                if e.classname.starts_with("momentary") {
                    *done = true;
                    if !e.target.is_empty() {
                        let moving = w
                            .find_entities(super::EntityKey::Targetname, &e.target)
                            .into_iter()
                            .filter_map(|i| w.entity(i))
                            .any(|d| d.is_moving());
                        if moving {
                            *done = false;
                        }
                    }
                }
            } else {
                *done = until <= now;
            }
            return;
        }
        let o = e.entity_origin();
        if (o - self.me.origin).length_2d() < 24.0 && (o.z - self.me.origin.z).abs() > MAX_JUMP_HEIGHT {
            *failed = true;
            return;
        }
        if task.int == -1 || self.distance_from(o, false) < 72.0 {
            if self.facing(&e) {
                if task.int == -1 {
                    if _rng.random_long(0, 1) != 0 {
                        self.use_button();
                        *done = true;
                    }
                    return;
                }
                self.tasks.current_mut().expect("current task").int = 1;
                self.use_button();
            }
        } else {
            self.set_move_vector(o);
        }
    }

    /// `BOT_TASK_LISTEN_TO_SOUND`.
    fn task_listen(
        &mut self,
        w: &dyn BotWorld,
        rng: &mut dyn Random,
        task: Task,
        done: &mut bool,
        failed: &mut bool,
        to_add: &mut Option<Task>,
    ) {
        let now = w.time();
        if (self.enemy.is_some() && self.has(cond::SEE_ENEMY)) || self.is_on_ladder() {
            *failed = true;
            return;
        }
        let Some(sound) = self.resolve(w, task.edict) else {
            *failed = true;
            return;
        };
        let mut state = task.int;
        let mut until = task.float;
        let mut v = task.vector;
        if state == 0 {
            until = now + task.float;
            state = 1;
            v = sound.origin;
        } else if state == 1 {
            v = sound.origin;
            if sound.button & buttons::ATTACK != 0 {
                if self.is_visible(sound.index) {
                    if !self.is_enemy(w, &sound) {
                        state = 2;
                    }
                } else if self.investigate_sound_time < now {
                    self.investigate_sound_time = now + rng.random_float(8.0, 16.0);
                    *to_add = Some(Task::new(TaskKind::FindPath, task.schedule_id, Some(sound.handle()), -1, -2.0, Vector::ZERO));
                }
            }
        } else if state == 2 {
            let (forward, _) = make_vectors(sound.v_angle);
            v = sound.origin + sound.view_ofs + forward * 8192.0;
            state = 3;
        }
        let t = self.tasks.current_mut().expect("current task");
        t.float = until;
        t.int = state;
        t.vector = v;
        self.look_task = LookTask::FaceTaskVector;
        if until <= now {
            *done = true;
            self.listen_to_sound_time =
                if !self.tasks.has_task(TaskKind::Use) { now + self.reaction_time * 4.0 } else { now + rng.random_float(4.5, 10.0) };
        }
    }
}
