//! Lifts, doors and their buttons, pushables, human towers and the other special waypoints, and
//! what the bot touches (`bot_navigate.cpp` `CheckLift`; `bot.cpp` `BotFunc_FindNearestButton`,
//! `CMasterEntity::FindButton`, `CBot::Touch`, `PlayerStandingOnMe`, `StandingOnPlayer`, the
//! `PUSH_PUSHABLE`, `WAIT_FOR_LIFT`, `WAIT_FOR_ENTITY`, `HUMAN_TOWER`, `THROW_GRENADE`,
//! `WAIT_FOR_BOT_AT_WPT` and `FACE_EDICT` tasks; `util.cpp` `UTIL_FindNearestEntity`,
//! `UTIL_IsButton`, the pushable helpers; `waypoint.cpp` `PlayerNearVector`).
//!
//! Upstream's masters list (`CMasterEntities`, filled from every `master` key value in
//! `DispatchKeyValue`) comes in as [`BotGlobals::masters`]; the port's plugin API doesn't report
//! key values, so the host passes an empty list (design §4).

use super::{
    Bot, BotGlobals, BotWorld, EF_NODRAW, EntRef, Entity, EntityKey, LookTask, SOLID_TRIGGER, ScheduleDesc, Task, TaskKind, buttons,
    entflags,
};
use crate::nav::{Glass, MAX_JUMP_HEIGHT, Monsters, REACHABLE_RANGE, flags as fl};
use crate::{Random, Vector};

/// An entity that has a `master` key value (`CMasterEntity`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MasterEntity {
    /// `m_pEntity`.
    pub entity: u32,
    /// `m_szMasterName`.
    pub master: String,
}

/// `UTIL_IsButton`: a brush entity (`func_*`).
fn is_button(e: &Entity) -> bool {
    e.classname.starts_with("func_")
}

/// `UTIL_GetBestPushableDistance`.
pub(crate) fn best_pushable_distance(p: &Entity) -> f32 {
    (p.size.x + p.size.y) / 3.0
}

/// `UTIL_GetDesiredPushableVector`: where to stand to push `p` towards `origin`.
pub(crate) fn desired_pushable_vector(origin: Vector, p: &Entity) -> Vector {
    let v = p.entity_origin();
    v - (origin - v).normalize() * best_pushable_distance(p)
}

/// `UTIL_AcceptablePushableVector`: `p` is near enough `origin` (2D, from its `origin`).
pub(crate) fn acceptable_pushable_vector(p: &Entity, origin: Vector) -> bool {
    (p.origin - origin).length_2d() <= best_pushable_distance(p)
}

/// `UTIL_FindNearestEntity(classnames, origin, range, visible, ignore)`: the nearest entity of
/// those classes within `range` (which shrinks as nearer ones are found), with a clear line to it
/// when `visible`. The candidates are traced nearest first (`nearest_visible`): upstream's
/// answer, with fewer traces.
pub(crate) fn find_nearest_entity(w: &dyn BotWorld, classnames: &[&str], origin: Vector, range: f32, visible: bool) -> Option<Entity> {
    let mut found: Vec<Entity> = Vec::new();
    let mut candidates = Vec::new();
    for name in classnames {
        for i in w.find_entities(EntityKey::Classname, name) {
            let Some(e) = w.entity(i) else { continue };
            let d = (e.entity_origin() - origin).length();
            if d < range {
                candidates.push((d, found.len()));
                found.push(e);
            }
        }
    }
    let pick = crate::nav::locations::nearest_visible(candidates, |k| {
        !visible || {
            let e = &found[k];
            let tr = w.trace_line(origin, e.entity_origin(), Monsters::Ignore, Glass::Ignore, None);
            tr.fraction >= 1.0 || tr.hit == Some(e.index)
        }
    })?;
    found.into_iter().nth(pick)
}

/// `PlayerNearVector`: the nearest living, named player within `range` of `origin`.
pub(crate) fn player_near_vector(w: &dyn BotWorld, origin: Vector, mut range: f32) -> Option<Entity> {
    let mut nearest = None;
    for i in 1..=w.max_clients() {
        let Some(p) = w.entity(i) else { continue };
        if p.netname.is_empty() || !p.is_alive() {
            continue;
        }
        let d = (p.entity_origin() - origin).length();
        if d <= range {
            range = d;
            nearest = Some(p);
        }
    }
    nearest
}

/// `CMasterEntity::FindButton`: the nearest visible button targeting the master (or, for a relay,
/// the first entity targeting the relay), else the nearest hidden one. Upstream's relay loop
/// starts each search from the relay instead of the last result, so only the first entity
/// targeting the relay is looked at; kept.
fn master_find_button(w: &dyn BotWorld, master: &str, origin: Vector) -> Option<u32> {
    let mut nearest_vis: Option<(u32, f32)> = None;
    let mut nearest: Option<(u32, f32)> = None;
    let consider = |b: &Entity, nearest_vis: &mut Option<(u32, f32)>, nearest: &mut Option<(u32, f32)>| {
        let o = b.entity_origin();
        let d = (origin - o).length();
        let tr = w.trace_line(origin, o, Monsters::Ignore, Glass::DontIgnore, None);
        let slot = if tr.fraction >= 1.0 { nearest_vis } else { nearest };
        if slot.is_none_or(|(_, nd)| d < nd) {
            *slot = Some((b.index, d));
        }
    };
    for i in w.find_entities(EntityKey::Target, master) {
        let Some(b) = w.entity(i) else { continue };
        if is_button(&b) {
            consider(&b, &mut nearest_vis, &mut nearest);
        } else if let Some(b2) = w.find_entities(EntityKey::Target, &b.targetname).first().and_then(|&j| w.entity(j))
            && is_button(&b2)
        {
            consider(&b2, &mut nearest_vis, &mut nearest);
        }
    }
    nearest_vis.or(nearest).map(|(i, _)| i)
}

/// `BotFunc_FindNearestButton(origin, door, &found_origin)`: the master's button, else the
/// nearest entity targeting the door that can be seen at its middle or either corner (triggers
/// always count); `None` unless it is a button. Also where to go to reach it.
pub(crate) fn find_nearest_button(w: &dyn BotWorld, masters: &[MasterEntity], origin: Vector, door: &Entity) -> Option<(Entity, Vector)> {
    if let Some(m) = masters.iter().find(|m| m.entity == door.index)
        && let Some(b) = master_find_button(w, &m.master, origin).and_then(|i| w.entity(i))
    {
        let o = b.entity_origin();
        return Some((b, o));
    }
    let mut best: Option<(Entity, Vector, f32)> = None;
    if !door.targetname.is_empty() {
        for i in w.find_entities(EntityKey::Target, &door.targetname) {
            let Some(t) = w.entity(i) else { continue };
            let o = t.entity_origin();
            let d = (o - origin).length();
            if best.as_ref().is_some_and(|(_, _, bd)| d >= *bd) {
                continue;
            }
            if t.solid != SOLID_TRIGGER {
                let mut found = None;
                for target in [o, o + t.size, o - t.size] {
                    let tr = w.trace_line(origin, target, Monsters::Ignore, Glass::DontIgnore, None);
                    if tr.hit == Some(t.index) || tr.fraction >= 1.0 {
                        found = Some(target);
                        break;
                    }
                }
                if let Some(f) = found {
                    best = Some((t, f, d));
                    // Upstream stops at the first visible one.
                    break;
                }
            } else {
                best = Some((t, o, d));
            }
        }
    }
    best.filter(|(b, _, _)| is_button(b)).map(|(b, o, _)| (b, o))
}

impl Bot {
    /// `PlayerStandingOnMe`: a named player whose ground entity is the bot.
    pub(crate) fn player_standing_on_me(&self, w: &dyn BotWorld) -> Option<Entity> {
        (1..=w.max_clients()).filter_map(|i| w.entity(i)).find(|p| !p.netname.is_empty() && p.groundentity == Some(self.index))
    }

    /// `StandingOnPlayer`: the client the bot stands on.
    pub(crate) fn standing_on_player(&self, w: &dyn BotWorld) -> Option<Entity> {
        self.me.groundentity.and_then(|g| w.entity(g)).filter(Entity::is_client)
    }

    /// `CheckLift(pBot, vCheckOrigin, vCheckToOrigin)`: a point trace from the waypoint towards
    /// the next; if it hits a still door, plat or train with a name, plan to press its button
    /// (and, for a lift, wait for it at the lift waypoint). True while a `USE_LIFT` schedule runs.
    pub(crate) fn check_lift(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, from: Vector, to: Vector, rng: &mut dyn Random) -> bool {
        let now = w.time();
        let dir = (to - from).normalize();
        let tr = w.trace_point_hull(from, from + dir * 4096.0, Monsters::Ignore, Some(self.index));
        let hit = if tr.fraction < 1.0 { tr.hit.and_then(|h| w.entity(h)) } else { None };
        if let Some(hit) = hit
            && !hit.targetname.is_empty()
            && (hit.classname == "func_seethroughdoor"
                || hit.classname.starts_with("func_door")
                || hit.classname.starts_with("func_plat")
                || hit.classname.starts_with("func_train"))
        {
            let is_lift = hit.movedir.z != 0.0 && hit.size.x > self.me.size.z && hit.size.y > self.me.size.z;
            if !hit.is_moving() {
                let eye = self.me.origin + self.me.view_ofs;
                let id = self.tasks.new_schedule_id();
                if let Some((button, button_origin)) = find_nearest_button(w, g.masters, eye, &hit) {
                    let mut normal_door = true;
                    self.tasks.flush();
                    self.paths.failed_paths.clear();
                    if is_lift {
                        let mut wait_wpt = g
                            .waypoints
                            .find_nearest_goal(self.gun_position(), REACHABLE_RANGE, self.team, fl::LIFT, &self.paths.failed_goals)
                            .map_or(-1, |i| i as i32);
                        if wait_wpt == -1 {
                            wait_wpt = self.prev_waypoint;
                        }
                        if wait_wpt != -1 {
                            match find_nearest_button(w, g.masters, g.waypoints.origin(wait_wpt), &hit) {
                                Some((other, _)) => {
                                    self.add_task(Task::new(TaskKind::FindPath, id, Some(other.handle()), 0, -1.0, Vector::ZERO));
                                    self.add_task(Task::new(TaskKind::Use, id, Some(other.handle()), 0, 1.0, Vector::ZERO));
                                }
                                None => {
                                    self.add_task(Task::new(TaskKind::FindPath, id, None, 0, 0.0, button_origin));
                                    self.add_task(Task::new(TaskKind::Use, id, Some(button.handle()), 0, 1.0, Vector::ZERO));
                                }
                            }
                            self.add_task(Task::new(TaskKind::FindPath, id, None, wait_wpt, -1.0, Vector::ZERO));
                            self.add_task(Task::new(TaskKind::WaitForEntity, id, Some(hit.handle()), 0, 0.0, Vector::ZERO));
                            self.tasks.set_time_to_complete_schedule(id, now, rng.random_float(10.0, 20.0));
                            normal_door = false;
                        }
                    }
                    if normal_door {
                        self.add_task(Task::new(TaskKind::FindPath, id, None, 0, 0.0, button_origin));
                        self.add_task(Task::new(TaskKind::Use, id, Some(button.handle()), 0, 1.0, Vector::ZERO));
                    }
                    let goal = self.goal_origin.unwrap_or(Vector::ZERO);
                    self.add_task(Task::new(TaskKind::FindPath, 0, None, 0, 0.0, goal));
                } else if !self.tasks.has_schedule(ScheduleDesc::UseLift) {
                    // No button found: a lift button waypoint near the lift?
                    let range = hit.size.length_2d();
                    if let Some(wpt) = g.waypoints.find_nearest_goal(self.gun_position(), range, -1, fl::LIFT, &self.paths.failed_goals)
                        && let Some(button) = find_nearest_entity(
                            w,
                            &["func_button", "button_target", "func_rot_button"],
                            g.waypoints.origin(wpt as i32),
                            range,
                            true,
                        )
                    {
                        let sid = self.tasks.new_schedule_id();
                        self.add_priority_task(Task::new(TaskKind::WaitForEntity, sid, Some(hit.handle()), 0, 0.0, Vector::ZERO));
                        self.add_priority_task(Task::new(TaskKind::FindPath, sid, None, wpt as i32, -1.0, Vector::ZERO));
                        self.add_priority_task(Task::new(TaskKind::Use, sid, Some(button.handle()), 0, 0.0, Vector::ZERO));
                        self.add_priority_task(Task::new(TaskKind::FindPath, sid, Some(button.handle()), 0, 0.0, Vector::ZERO));
                        self.tasks.give_schedule_description(sid, ScheduleDesc::UseLift);
                    }
                }
            }
        }
        self.tasks.has_schedule(ScheduleDesc::UseLift)
    }

    /// The special waypoints' part of `BotNavigate_UpdateWaypoint` right after a waypoint was
    /// touched: a pushable to put in place.
    pub(crate) fn touched_pushable_waypoint(&mut self, w: &dyn BotWorld, wpt_origin: Vector) {
        if self.current_waypoint_flags & fl::PUSHABLE == 0 || self.tasks.has_task(TaskKind::PushPushable) {
            return;
        }
        if let Some(p) = find_nearest_entity(w, &["func_pushable"], wpt_origin, 512.0, false)
            && !acceptable_pushable_vector(&p, wpt_origin)
        {
            let id = self.tasks.new_schedule_id();
            self.add_priority_task(Task::new(TaskKind::PushPushable, id, Some(p.handle()), 0, 0.0, wpt_origin));
            let push = desired_pushable_vector(wpt_origin, &p);
            self.add_priority_task(Task::new(TaskKind::FindPath, id, None, 0, 0.0, push));
        }
    }

    /// The special waypoints' part of `BotNavigate_UpdateWaypoint` once the next waypoint is
    /// chosen: grenade throws, waits before jumps, lifts.
    pub(crate) fn next_waypoint_tasks(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, prev_flags: u32, rng: &mut dyn Random) {
        let wpt = g.waypoints.origin(self.current_waypoint);
        let f = self.current_waypoint_flags;
        // Upstream asked `HasWeapon(VALVE_WEAPON_HANDGRENADE)`, id 12, TS's Desert Eagle, and threw
        // at the waypoint (design §5.59). A TS bot with an M61 throws one from here at an enemy
        // seen in the last 5 s 300-750 units away.
        let now = w.time();
        let enemy_at = self.last_see_enemy_position;
        if f & fl::GREN_THROW != 0
            && self.weapons.get(crate::weapons::TsWeapon::M61Grenade.id()).is_some_and(|wp| wp.has_weapon && wp.clip > 0)
            && self.last_see_enemy_time > 0.0
            && self.last_see_enemy_time + 5.0 > now
            && (300.0..750.0).contains(&self.distance_from(enemy_at, false))
        {
            self.add_priority_task(Task::new(TaskKind::ThrowGrenade, 0, None, 0, 0.0, enemy_at).with_timeout(now, 4.0));
        } else if self.me.flags & entflags::ONGROUND != 0
            && self.me.waterlevel < 3
            && ((f & fl::JUMP != 0 && prev_flags & fl::CROUCH != 0) || (f & fl::JUMP != 0 && f & fl::STAY_NEAR != 0))
        {
            self.add_priority_task(Task::new(TaskKind::WaitAndFaceVector, 0, None, 0, 0.75, wpt));
        }
        let next = self.next_waypoint();
        if f & fl::WAIT_FOR_LIFT != 0 {
            self.add_priority_task(Task::new(TaskKind::WaitForLift, 0, None, 0, 0.0, wpt));
        } else if f & (fl::JUMP | fl::CROUCHJUMP) != 0 && next != -1 && g.waypoints.flags(next) & fl::WAIT_FOR_LIFT != 0 {
            self.add_priority_task(Task::new(TaskKind::WaitForLift, 0, None, 0, 0.0, g.waypoints.origin(next)));
        } else if !self.tasks.has_schedule(ScheduleDesc::UseLift) {
            let check_next = next != -1 && f & fl::CHECK_LIFT != 0;
            if !(check_next && self.check_lift(g, w, wpt, g.waypoints.origin(next), rng)) {
                self.check_lift(g, w, g.waypoints.origin(self.prev_waypoint), wpt, rng);
            }
        }
    }

    /// The human-tower touch test of `BotNavigate_UpdateWaypoint`: high enough, then within 100
    /// units if someone crouches there already, else within 24.
    pub(crate) fn touched_human_tower(&self, w: &dyn BotWorld, wpt_origin: Vector, distance: f32) -> bool {
        let crouching = w
            .entities_in_sphere(wpt_origin, 24.0)
            .into_iter()
            .filter_map(|i| w.entity(i))
            .any(|p| p.is_client() && !p.netname.is_empty() && (p.flags & entflags::DUCKING != 0 || p.button & buttons::DUCK != 0));
        if self.me.origin.z < wpt_origin.z - 16.0 {
            false
        } else if crouching {
            distance <= 100.0
        } else {
            distance <= 24.0
        }
    }

    /// `CBot::Touch(pentTouched)`, TS: picking up the item it went for, doors and lifts in the
    /// way. Returns upstream's "don't let the game see the touch" (never true in TS).
    pub fn touch(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, touched: &Entity, rng: &mut dyn Random) -> bool {
        let now = w.time();
        if self.pickup_entity == Some(touched.handle()) {
            self.items.touched(touched.handle(), now);
            if let Some(t) = self.tasks.current()
                && t.kind == TaskKind::PickupItem
                && t.edict == self.pickup_entity
            {
                self.tasks.finished_current(false);
            }
            self.pickup_item_time = now + rng.random_float(5.0, 10.0);
            self.pickup_entity = None;
            return false;
        }
        if touched.solid == SOLID_TRIGGER {
            return false;
        }
        let is_door = touched.classname.starts_with("func_door") || touched.classname.starts_with("func_plat");
        if is_door && !touched.is_moving() {
            self.last_see_waypoint = 0.0;
        }
        let using_lift = self.tasks.has_schedule(ScheduleDesc::UseLift);
        if is_door
            && self.current_waypoint != -1
            && touched.movedir.z != 0.0
            && (g.waypoints.origin(self.current_waypoint).z > self.gun_position().z + MAX_JUMP_HEIGHT
                || !self.fvisible_point(w, self.move_to))
        {
            if let Some((button, _)) = find_nearest_button(w, g.masters, self.me.origin, touched) {
                if !using_lift {
                    let id = self.tasks.new_schedule_id();
                    self.add_priority_task(Task::new(TaskKind::Use, id, Some(button.handle()), 0, 0.0, Vector::ZERO));
                    self.add_priority_task(Task::new(TaskKind::FindPath, id, Some(button.handle()), 0, 0.0, Vector::ZERO));
                    self.tasks.give_schedule_description(id, ScheduleDesc::UseLift);
                    self.tasks.set_time_to_complete_schedule(id, now, rng.random_float(10.0, 15.0));
                }
            } else {
                let range = touched.size.length_2d();
                if let Some(wpt) = g.waypoints.find_nearest_goal(self.gun_position(), range, -1, fl::LIFT, &self.paths.failed_goals)
                    && let Some(button) = find_nearest_entity(
                        w,
                        &["func_button", "button_target", "func_rot_button"],
                        g.waypoints.origin(wpt as i32),
                        range,
                        true,
                    )
                {
                    let id = self.tasks.new_schedule_id();
                    let current = self.tasks.current().copied();
                    self.tasks.flush();
                    self.add_task(Task::new(TaskKind::FindPath, id, Some(button.handle()), 0, 0.0, Vector::ZERO));
                    self.add_task(Task::new(TaskKind::Use, id, Some(button.handle()), 0, 0.0, Vector::ZERO));
                    if let Some(c) = current {
                        self.add_task(c);
                    }
                    self.tasks.give_schedule_description(id, ScheduleDesc::UseLift);
                }
            }
        }
        false
    }

    /// `BOT_TASK_PUSH_PUSHABLE`: go behind the pushable, face it, and walk it to the waypoint
    /// (6-8 s per state).
    pub(crate) fn task_push_pushable(&mut self, w: &dyn BotWorld, rng: &mut dyn Random, task: Task, done: &mut bool, failed: &mut bool) {
        let now = w.time();
        let Some(p) = self.resolve(w, task.edict) else {
            *failed = true;
            return;
        };
        let origin = task.vector;
        let size = best_pushable_distance(&p);
        if acceptable_pushable_vector(&p, origin) {
            *done = true;
            return;
        }
        match task.int {
            0 => {
                let t = self.tasks.current_mut().expect("current task");
                t.float = now + rng.random_float(6.0, 8.0);
                t.int = 1;
            }
            1 => {
                let desired = desired_pushable_vector(origin, &p);
                if self.distance_from(desired, false) > size / 2.0 {
                    self.set_move_vector(desired);
                } else {
                    self.tasks.current_mut().expect("current task").int = 2;
                }
            }
            2 => {
                if !self.facing(&p) {
                    self.look_task = LookTask::FaceTaskEdict;
                } else {
                    self.tasks.current_mut().expect("current task").int = 3;
                }
                self.tasks.current_mut().expect("current task").float = now + rng.random_float(6.0, 8.0);
            }
            _ => {
                if rng.random_long(0, 100) != 0 {
                    self.use_button();
                }
                self.set_move_vector(origin);
            }
        }
        if self.tasks.current().is_some_and(|t| t.float < now) {
            *failed = true;
        }
    }

    /// `BOT_TASK_WAIT_FOR_LIFT`: stand and face where the lift comes until a door, plat or train
    /// is within 72 units, or 10 s.
    pub(crate) fn task_wait_for_lift(&mut self, w: &dyn BotWorld, task: Task, done: &mut bool) {
        let now = w.time();
        self.look_task = LookTask::FaceTaskVector;
        self.stop_moving();
        for i in w.entities_in_sphere(task.vector, 72.0) {
            let Some(l) = w.entity(i) else { continue };
            if l.effects & EF_NODRAW != 0 || l.index == self.index {
                continue;
            }
            if l.classname.starts_with("func_door") || l.classname.starts_with("func_plat") || l.classname.starts_with("func_train") {
                *done = true;
                break;
            }
        }
        if !*done {
            if task.int == 0 {
                let t = self.tasks.current_mut().expect("current task");
                t.float = now + 10.0;
                t.int = 1;
            } else if task.float < now {
                *done = true;
            }
        }
    }

    /// `BOT_TASK_WAIT_FOR_ENTITY`: wait while the entity moves.
    pub(crate) fn task_wait_for_entity(&mut self, w: &dyn BotWorld, task: Task, done: &mut bool, failed: &mut bool) {
        match self.resolve(w, task.edict) {
            Some(e) if e.is_moving() => {
                self.stop_moving();
                self.look_task = LookTask::FaceTaskEdict;
            }
            Some(_) => *done = true,
            None => *failed = true,
        }
    }

    /// `BOT_TASK_HUMAN_TOWER`: walk onto the waypoint (jumping onto whoever is there), then
    /// crouch until someone stands on the bot, stand up for them and jump; 8-10 s at most.
    pub(crate) fn task_human_tower(&mut self, w: &dyn BotWorld, rng: &mut dyn Random, task: Task, done: &mut bool) {
        let now = w.time();
        let tower = task.vector;
        let dist_2d = self.distance_from(tower, true);
        let nearby = player_near_vector(w, tower, 36.0);
        if task.float == 0.0 {
            self.tasks.current_mut().expect("current task").float = now + rng.random_float(8.0, 10.0);
        } else if task.float < now {
            *done = true;
            return;
        }
        if dist_2d > 36.0 {
            self.set_move_vector(tower);
            if dist_2d < 48.0 && nearby.is_some_and(|p| p.index != self.index) && self.end_jump_time < now {
                self.jump_and_duck(now);
            }
            return;
        }
        self.stop_moving();
        if self.standing_on_player(w).is_some() {
            if self.move_to.z - self.me.origin.z < MAX_JUMP_HEIGHT {
                *done = true;
            } else {
                self.jump_and_duck(now);
            }
        } else if self.player_standing_on_me(w).is_some() {
            self.button &= !buttons::DUCK;
            self.tasks.current_mut().expect("current task").int = 1;
        } else if task.int == 1 {
            self.button &= !buttons::DUCK;
            if self.end_jump_time + 0.5 < now {
                self.jump();
            } else {
                self.tasks.current_mut().expect("current task").int = 0;
            }
        } else {
            self.duck();
        }
    }

    /// `BOT_TASK_WAIT_FOR_BOT_AT_WPT`: wait 4-6 s, or until the bot is nearer the waypoint than
    /// the other bot.
    pub(crate) fn task_wait_for_bot_at_wpt(
        &mut self,
        g: &BotGlobals<'_>,
        w: &dyn BotWorld,
        rng: &mut dyn Random,
        task: Task,
        done: &mut bool,
        failed: &mut bool,
    ) {
        let now = w.time();
        let (wpt, Some(other)) = (task.int, task.edict) else {
            *failed = true;
            return;
        };
        if wpt == -1 {
            *failed = true;
            return;
        }
        if task.float == 0.0 {
            self.tasks.current_mut().expect("current task").float = now + rng.random_float(4.0, 6.0);
        }
        let Some(o) = g.other_bots.iter().find(|b| b.handle == other) else {
            *failed = true;
            return;
        };
        let v = g.waypoints.origin(wpt);
        self.stop_moving();
        let until = self.tasks.current().map_or(0.0, |t| t.float);
        *done = until < now || self.distance_from(v, false) <= (v - o.gun_position).length();
    }

    /// `BOT_TASK_FACE_EDICT`: face the entity for the task's time.
    pub(crate) fn task_face_edict(&mut self, w: &dyn BotWorld, task: Task, done: &mut bool, failed: &mut bool) {
        let now = w.time();
        if self.resolve(w, task.edict).is_none() {
            *failed = true;
            return;
        }
        self.look_task = LookTask::FaceTaskEdict;
        if task.int == 0 {
            let t = self.tasks.current_mut().expect("current task");
            t.float += now;
            t.int = 1;
        } else if task.float < now {
            *done = true;
        }
    }

    /// `BOT_TASK_THROW_GRENADE`, TS (design §4; upstream only threw for Opposing Force and failed
    /// the task in TS): takes out the M61, turns to the view pitch whose throw lands at the
    /// task's vector ([`super::fire::throw_pitch`], gravity `0.75 sv_gravity`), pulls the pin
    /// (`+attack`) for 0.2 s and lets go (TS throws on release), then takes out a loaded gun.
    pub(crate) fn task_throw_grenade(&mut self, w: &dyn BotWorld, task: Task, done: &mut bool, failed: &mut bool) {
        let now = w.time();
        let m61 = crate::weapons::TsWeapon::M61Grenade.id();
        self.stop_moving();
        match task.int {
            0 => {
                if !self.weapons.get(m61).is_some_and(|wp| wp.has_weapon && wp.clip > 0) {
                    *failed = true;
                    return;
                }
                if self.current_weapon != Some(m61) {
                    self.switch_weapon(m61);
                    return;
                }
                let src = self.gun_position();
                let Some((pitch, _)) = super::fire::throw_pitch(src, task.vector, w.cvar_float("sv_gravity") * 0.75) else {
                    *failed = true;
                    return;
                };
                let yaw = super::look::vec_to_angles(task.vector - src).y;
                let (forward, _) = super::look::make_vectors(Vector::new(pitch, yaw, 0.0));
                let aim = src + forward * 128.0;
                self.look_task = LookTask::None;
                self.fight.aim_true = Some(aim);
                self.turn_to(aim, now, 0.04);
                let off = |a: f32, b: f32| super::look::angle_diff(a, b).abs();
                if off(pitch, self.v_angle.x) < 1.5 && off(yaw, self.v_angle.y) < 1.5 {
                    self.primary_attack();
                    let t = self.tasks.current_mut().expect("current task");
                    t.int = 1;
                    t.float = now + 0.2;
                }
            }
            1 => {
                self.look_task = LookTask::None;
                if task.float > now {
                    self.primary_attack();
                } else {
                    let t = self.tasks.current_mut().expect("current task");
                    t.int = 2;
                    t.float = now + 0.6;
                }
            }
            _ => {
                if task.float < now {
                    *done = true;
                    if let Some(next) = self.weapons.best_loaded_gun().filter(|&id| id != m61) {
                        self.switch_weapon(next);
                    }
                }
            }
        }
    }
}

/// A handle and position of another bot (`gBotGlobals.m_Bots`, read while this one thinks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OtherBot {
    /// The bot's entity.
    pub handle: EntRef,
    /// `m_iCurrentWaypointIndex`.
    pub current_waypoint: i32,
    /// `GetGunPosition()`.
    pub gun_position: Vector,
}
