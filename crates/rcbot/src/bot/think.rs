//! The bot's frame (`globals.cpp`'s per-bot part of `CBotGlobals::StartFrame`; `bot.cpp`
//! `CBot::Think`, `SpawnInit`, `DoTasks`, `LookForNewTasks`, `WantToFindEnemy`;
//! `util.cpp` `UTIL_UpdateSounds`, `UTIL_IsFacingEntity`; `CBot::CanPickup`).
//!
//! TS only. Tasks no TS bot creates are not ported (design §2). Upstream's death-time `buy` /
//! `menuselect 7` (marked "doesn't work") is replaced by the loadout the gun choice buys.

use super::look::make_vectors;
use super::{Action, Bot, BotGlobals, BotWorld, EF_NODRAW, Entity, Frame, LookTask, TS_STATE_STUNT, Task, TaskKind, buttons, cond};
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
        // The engine sets `v_angle` from our usercmd; it differs when the game turned the
        // player (spawn, teleport: `fixangle`).
        self.v_angle = me.v_angle;
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
            if self.next_inventory_sync <= now {
                self.next_inventory_sync = now + INVENTORY_SYNC;
                if let Some(owned) = w.inventory(self.index) {
                    self.weapons.sync_inventory(&owned, g.weapons, g.presets);
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
        out.cmd = self.run_player_move(w.time(), w.frame_time());
        out
    }

    /// The game moved the bot elsewhere (a teleport: the host sees `fixangle` 1, or the bot
    /// moved more than 256 units in a frame). Its waypoints, path and stuck timers belong to
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
        self.ladder_angles = None;
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
        self.next_reload_time = 0.0;
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

    /// `Think`'s TS weapon choice (`BOT_CONDITION_SELECTED_GUN`): the GA's pick (or seeded
    /// values), bought unless `BOT_CONFIG_TS_KUNGFU`.
    fn select_loadout(&mut self, g: &mut BotGlobals<'_>, rng: &mut dyn Random) {
        if self.has(cond::SELECTED_GUN) {
            return;
        }
        let picks = self.learned.select_ts_weapons(&mut g.learned.ts_weapon_choices, g.config.is_on(cfg::DISABLE_WEAPON_LEARN), rng);
        if !g.config.is_on(cfg::TS_KUNGFU) {
            self.actions.push(Action::Loadout(picks.to_vec()));
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
        // A stealthy squad walks at half speed; `Think` resets the speed before anything reads
        // it, so this never shows (design §5). Kept.
        if self.squad.and_then(|id| g.squads.get(id)).is_some_and(|s| s.is_stealth()) {
            self.ideal_move_speed = self.max_speed / 2.0;
        }

        self.work_view_angles(w, rng);
        self.work_move_direction(w, g);
        self.check_stuck(w, g, rng);

        if self.last_think_time >= now {
            return;
        }
        self.last_think_time = now + self.think_time;

        // A bot the port handed over alive (`adopt_bot`) picks here, as upstream.
        self.select_loadout(g, rng);

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
                    match self.best_scored_weapon(dist, Some(cur.id)) {
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

        if !self.follow_formation(g, w) && self.update_waypoint_time < now && !self.update_waypoint(g, w, rng) {
            self.update_waypoint_time = now + 0.5;
        }

        if self.enemy.is_none() {
            if self.climb_type() == super::Climb::None
                && self.player_standing_on_me(w).is_none()
                && self.standing_on_player(w).is_none()
                && self.listen_to_sound_time < now
            {
                self.listen_to_sound_time = now + self.reaction_time + rng.random_float(0.3, 0.75);
                if let Some(p) = self.update_sounds(w)
                    && !self.tasks.has_task(TaskKind::ListenToSound)
                {
                    let f = rng.random_float(2.0, 3.0);
                    self.add_priority_task(Task::new(TaskKind::ListenToSound, 0, Some(p.handle()), 0, f, Vector::ZERO));
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
        self.clear(cond::TASK_EDICT_PAIN);
        if self.look_for_new_tasks {
            self.look_for_new_tasks(g, w, rng);
        }
        // `WantToFindEnemy`, TS: unless `dont_shoot` is on.
        self.enemy = if self.want_to_find_enemy { self.find_enemy(g, w, rng) } else { None };
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

    /// `UTIL_UpdateSounds`: the nearest other living player within 750 units firing (`IN_ATTACK`;
    /// TS doesn't listen for footsteps), more than 128 units away for a teammate, 64 for others.
    fn update_sounds(&self, w: &dyn BotWorld) -> Option<Entity> {
        let mut nearest = 750.0;
        let mut found = None;
        for i in 1..=w.max_clients() {
            if i == self.index {
                continue;
            }
            let Some(p) = w.entity(i) else { continue };
            if !p.is_alive() || p.netname.is_empty() || p.button & buttons::ATTACK == 0 {
                continue;
            }
            let same_team = p.player.map(|s| s.team) == Some(self.state.team);
            let d = (self.me.origin - p.origin).length();
            if d < nearest && ((same_team && d > 128.0) || (!same_team && d > 64.0)) {
                nearest = d;
                found = Some(p);
            }
        }
        found
    }

    /// `CBot::CanPickup`, TS: power-ups and ground weapons in reach.
    fn can_pickup(&self, now: f32, e: &Entity) -> bool {
        if self.pickup_item_time > now || e.effects & EF_NODRAW != 0 || e.origin.z > self.me.origin.z + MAX_JUMP_HEIGHT {
            return false;
        }
        e.classname.starts_with("ts_powerup") || e.classname == "ts_groundweapon"
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

    /// `LookForNewTasks`' TS objectives: a random one of the objectives the bot was told of
    /// ([`BotWorld::ts_objectives`], upstream's `m_TSObjectives.Random()`), unless the bot roams
    /// instead (1 in 3 on team 1, 1 in 11 on team 2): the path to its nearest waypoint, then team 1
    /// `DEFEND`s it and team 2 `USE`s a `ts_hack` / `ts_bomb` within 100 units of that waypoint for
    /// 9 s. False when there is nothing to do (no objective, roaming, no waypoint): the end-level
    /// waypoints come next.
    fn objective_tasks(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random, id: i32) -> bool {
        let objectives = w.ts_objectives(self.index);
        if objectives.is_empty() {
            return false;
        }
        let obj = &objectives[rng.random_long(0, objectives.len() as i32 - 1) as usize];
        let wpt = self.nearest_waypoint(g, w, obj.origin, REACHABLE_RANGE, self.last_failed_waypoint, false, false, true);
        let team = self.team_number();
        let roam = (team == 1 && rng.random_long(0, 2) == 0) || (team == 2 && rng.random_long(0, 10) == 0);
        if roam || wpt == -1 {
            return false;
        }
        self.add_task(Task::new(TaskKind::FindPath, id, None, wpt, -1.0, Vector::ZERO));
        let at = g.waypoints.origin(wpt);
        let target = w.entities_in_sphere(at, 100.0).into_iter().find_map(|i| {
            let e = w.entity(i)?;
            (e.classname == "ts_hack" || e.classname == "ts_bomb").then_some(e)
        });
        if team == 1 {
            self.add_task(Task::new(TaskKind::Defend, id, None, 0, 0.0, Vector::ZERO));
        } else if let Some(t) = target {
            self.add_task(Task::new(TaskKind::Use, id, Some(t.handle()), 0, 9.0, Vector::ZERO));
        }
        true
    }

    /// `CBot::LookForNewTasks`, TS: pick up the nearest visible weapon or power-up; in teamplay
    /// go to a random end-level waypoint; else roam (follow the squad leader; or a remembered
    /// enemy position half the time, otherwise a random goal).
    pub(crate) fn look_for_new_tasks(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) {
        let now = w.time();
        self.stop_moving();
        self.look_task = if self.resolve(w, self.last_enemy).is_some_and(|e| e.is_alive()) {
            LookTask::SearchForLastEnemy
        } else {
            LookTask::LookAround
        };
        let mut nearest_pickup: Option<(Entity, f32)> = None;
        for i in self.visibles.iter() {
            if i == self.index {
                continue;
            }
            let Some(e) = w.entity(i) else { continue };
            let d = self.distance_from(e.entity_origin(), false);
            if nearest_pickup.as_ref().is_none_or(|(_, nd)| d < *nd) && self.can_pickup(now, &e) {
                nearest_pickup = Some((e, d));
            }
        }
        let id = self.tasks.new_schedule_id();
        let mut roam = true;
        if !g.config.is_on(cfg::TS_DONT_PICKUP_WEAPONS)
            && let Some((p, _)) = nearest_pickup
        {
            let h = Some(p.handle());
            self.add_task(Task::new(TaskKind::FindPath, id, h, 0, 0.0, Vector::ZERO));
            self.add_task(Task::new(TaskKind::PickupItem, id, h, 0, 0.0, Vector::ZERO));
            self.add_task(Task::new(TaskKind::Use, id, h, 0, 0.0, Vector::ZERO));
            roam = false;
        } else if w.teamplay() && self.objective_tasks(g, w, rng, id) {
            roam = false;
        } else if w.teamplay() {
            let wpt = g
                .waypoints
                .find_random_goal_near(self.me.origin, 4096.0, -1, fl::ENDLEVEL, &self.paths.failed_goals, rng)
                .map_or(-1, |i| i as i32);
            if self.current_waypoint != wpt && wpt != -1 {
                self.add_task(Task::new(TaskKind::FindPath, 0, None, wpt, -1.0, Vector::ZERO));
                roam = false;
            }
        }
        if !roam {
            return;
        }
        // In a squad (one a player made: TS bots never assemble their own), follow the leader.
        if self.squad_leader.is_some() {
            self.add_task(Task::new(TaskKind::FollowLeader, id, None, 0, 0.0, Vector::ZERO));
            return;
        }
        self.looking_for_enemy = false;
        let check_enemy = rng.random_long(0, 1) != 0 && self.remembered.any();
        if !check_enemy {
            if self.tasks.is_empty() {
                // TS bots never assemble squads.
                self.add_priority_task(Task::new(TaskKind::FindPath, id, None, -1, -1.0, Vector::ZERO));
            }
        } else if let Some(pos) = self.remembered.newest().copied() {
            let spotted = pos.origin;
            let mut v = pos.origin;
            let mut wpt = self.nearest_waypoint(g, w, v, REACHABLE_RANGE, -1, true, false, false);
            if wpt == -1 {
                v = pos.visible_origin;
                wpt = self.nearest_waypoint(g, w, v, REACHABLE_RANGE, -1, true, false, false);
            }
            if wpt != -1 && !self.paths.failed_goals.contains(&(wpt as usize)) {
                if spotted != v {
                    self.add_priority_task(Task::new(TaskKind::WaitAndFaceVector, id, None, 0, 1.0, spotted));
                }
                self.add_priority_task(Task::new(TaskKind::FindPath, id, None, wpt, -1.0, Vector::ZERO));
                self.looking_for_enemy = true;
            }
        }
        if self.tasks.is_empty() {
            self.add_priority_task(Task::new(TaskKind::FindPath, id, None, -1, -1.0, Vector::ZERO));
        }
        if self.looking_for_enemy {
            self.add_task(Task::new(TaskKind::SearchForEnemy, id, None, 0, 0.0, Vector::ZERO));
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
                    self.stop_moving();
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
                    Some(p) if self.can_pickup(now, &p) => {
                        self.set_move_vector(p.entity_origin());
                        if self.distance_from(p.entity_origin(), false) < 64.0 {
                            self.use_button();
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
                if task.float == 0.0 {
                    let f = now + rng.random_float(20.0, 80.0);
                    self.tasks.current_mut().expect("current task").float = f;
                }
                if self.tasks.current().is_some_and(|t| t.float < now) {
                    done = true;
                }
                if self.state.stunt_state != TS_STATE_STUNT {
                    self.alt_button();
                } else {
                    self.stop_moving();
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
                match self.astar.search(&mut self.paths, &q, true, g.waypoints, w, rng) {
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
                    let goal = g.waypoints.find_random_goal(self.team, 0, &self.paths.failed_goals, rng).map_or(-1, |i| i as i32);
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
                match self.astar.search(&mut self.paths, &q, false, g.waypoints, w, rng) {
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
                self.look_task = LookTask::NextWaypoint;
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
                self.choose_weapon(dist, now);
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
