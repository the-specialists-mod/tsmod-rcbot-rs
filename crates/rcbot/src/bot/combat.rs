//! What the bot sees and whom it fights (`bot.cpp` `CBot::UpdateVisibles`, `FVisible`,
//! `CanAvoid`, `FindEnemy`, `IsEnemy`, `EnemyFound`, `EnemyDown`, `workEnemyCosts`,
//! `clearEnemyCosts`, `decideJumpDuckStrafe`, `WantToFollowEnemy`, `RunForCover`,
//! `UpdateConditions`; `bot_visibles.cpp` `CBotVisibles`; `BotFunc_BreakableIsEnemy`).
//!
//! The visible list keeps upstream's insertion order (`m_iVisibleList`), which decides ties in
//! `FindEnemy`; a player the bot has a reputation of gets priority `BOT_MAX_REP - rep`, which is
//! part of each enemy's threat score (design §4).

use super::remember::REMEMBER_LOST_ENEMY;
use super::{
    BOT_COST_BUCKETS, BOT_COST_RANGE, BOT_VIEW_DISTANCE, Bot, BotGlobals, BotWorld, EF_NODRAW, EntRef, Entity, SOLID_NOT, SOLID_TRIGGER,
    ScheduleDesc, TS_STATE_STUNT, Task, TaskKind, cond, entflags,
};
use crate::nav::{Glass, Monsters};
use crate::{Random, Vector};

/// `BOT_REMEMBER_1ST_SEE_ENEMY`.
const REMEMBER_1ST_SEE_ENEMY: i32 = 1 << 3;
/// `SF_BREAK_TRIGGER_ONLY`.
const SF_BREAK_TRIGGER_ONLY: i32 = 1;

/// `CBotVisibles`: what the bot sees, in the order it came into view (what `FindEnemy` and the
/// pickup scan walk), with a bit per entity so the vision pass asks "already in?" without a scan.
#[derive(Clone, Debug, Default)]
pub(crate) struct Visibles {
    order: Vec<u32>,
    bits: Vec<u64>,
}

impl Visibles {
    /// `IsInVisibleList`.
    pub(crate) fn contains(&self, index: u32) -> bool {
        self.bits.get(index as usize / 64).is_some_and(|w| w & (1 << (index % 64)) != 0)
    }

    /// Adds `index` at the end (`AddVisible`) or removes it (`RemoveVisible`).
    pub(crate) fn set(&mut self, index: u32, on: bool) {
        if self.contains(index) == on {
            return;
        }
        let word = index as usize / 64;
        if word >= self.bits.len() {
            self.bits.resize(word + 1, 0);
        }
        self.bits[word] ^= 1 << (index % 64);
        if on {
            self.order.push(index);
        } else if let Some(p) = self.order.iter().position(|&i| i == index) {
            self.order.remove(p);
        }
    }

    /// The entities seen, oldest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.order.iter().copied()
    }

    /// The list as indices (to walk it while the bot changes).
    pub(crate) fn to_vec(&self) -> Vec<u32> {
        self.order.clone()
    }
}

impl<const N: usize> From<[u32; N]> for Visibles {
    fn from(list: [u32; N]) -> Visibles {
        let mut v = Visibles::default();
        for i in list {
            v.set(i, true);
        }
        v
    }
}

impl Bot {
    /// `IsInVisibleList`.
    pub(crate) fn is_visible(&self, index: u32) -> bool {
        self.visibles.contains(index)
    }

    fn set_visible(&mut self, index: u32, on: bool) {
        self.visibles.set(index, on);
    }

    /// `CBot::FVisible(pEntity)`: a clear line to its middle or its eyes; keeps the enemy's
    /// see-body / see-head conditions.
    pub(crate) fn fvisible(&mut self, w: &dyn BotWorld, e: &Entity) -> bool {
        let is_enemy = self.enemy == Some(e.handle());
        if is_enemy {
            self.set(cond::SEE_ENEMY_BODY);
            self.clear(cond::SEE_ENEMY_HEAD);
        }
        let gun = self.gun_position();
        let tr = w.trace_line(gun, e.entity_origin(), Monsters::DontIgnore, Glass::DontIgnore, Some(self.index));
        if tr.fraction >= 1.0 || tr.hit == Some(e.index) {
            return true;
        }
        if is_enemy {
            self.clear(cond::SEE_ENEMY_BODY);
        }
        let tr = w.trace_line(gun, e.origin + e.view_ofs, Monsters::DontIgnore, Glass::DontIgnore, Some(self.index));
        if tr.fraction >= 1.0 || tr.hit == Some(e.index) {
            if is_enemy {
                self.set(cond::SEE_ENEMY_HEAD);
            }
            return true;
        }
        if is_enemy {
            self.clear(cond::SEE_ENEMY_BODY | cond::SEE_ENEMY_HEAD | cond::SEE_ENEMY);
        }
        false
    }

    /// `CBot::FVisible(vecOrigin)`.
    pub(crate) fn fvisible_point(&self, w: &dyn BotWorld, v: Vector) -> bool {
        w.trace_line(self.gun_position(), v, Monsters::Ignore, Glass::DontIgnore, Some(self.index)).fraction >= 1.0
    }

    /// `CBot::IsEnemy`, TS: a living player of another team (any player outside teamplay), or
    /// a breakable the bot or a visible enemy near it stands by. Upstream compared `model` keys
    /// for teams; the port's rules team is used (the-specialistsmod-rs#104).
    pub(crate) fn is_enemy(&self, w: &dyn BotWorld, e: &Entity) -> bool {
        if e.index == self.index || e.takedamage == 0.0 || e.effects & EF_NODRAW != 0 || !e.is_alive() {
            return false;
        }
        if e.is_client() {
            let Some(p) = e.player else { return false };
            if !p.alive {
                return false;
            }
            if w.teamplay() {
                if w.model_teams() {
                    return w.player_model(self.index) != w.player_model(e.index);
                }
                return p.team != self.state.team;
            }
            return true;
        }
        if e.classname == "func_breakable" {
            let origin = e.entity_origin();
            let mut player = None;
            // Upstream's `UTIL_FindEntityInSphere` loop looks for the first client: clients
            // are the first entities, so the clients alone, in order, give the same one without
            // walking every entity for every breakable.
            for i in 1..=w.max_clients() {
                let Some(p) = w.entity(i) else { continue };
                if !p.is_client() || !box_in_sphere(&p, origin, 200.0) {
                    continue;
                }
                if i == self.index {
                    if self.distance_from(origin, false) < 80.0 {
                        player = Some(p);
                        break;
                    }
                } else {
                    player = Some(p);
                    break;
                }
            }
            let mine = player.as_ref().is_some_and(|p| p.index == self.index);
            let near_enemy = player.as_ref().is_some_and(|p| p.index != self.index && self.is_visible(p.index) && self.is_enemy(w, p));
            return ((self.state.stunt_state != TS_STATE_STUNT && mine) || near_enemy) && self.breakable_is_enemy(e);
        }
        false
    }

    /// `BotFunc_BreakableIsEnemy`.
    fn breakable_is_enemy(&self, b: &Entity) -> bool {
        if b.flags & entflags::WORLDBRUSH != 0 || b.spawnflags & SF_BREAK_TRIGGER_ONLY != 0 {
            return false;
        }
        let me = self.me.size;
        if !b.target.is_empty() || b.size.x >= me.x || b.size.y >= me.y || b.size.z >= me.z / 2.0 {
            return b.health < 1000.0 && b.effects & EF_NODRAW == 0;
        }
        false
    }

    /// `CBot::CanAvoid`, TS.
    fn can_avoid(&self, g: &BotGlobals<'_>, e: &Entity, dist: f32, avoid_dist: f32) -> bool {
        if self.me.groundentity == Some(e.index) || e.index == self.index || e.effects & EF_NODRAW != 0 {
            return false;
        }
        // Upstream stepped around its enemy unless it held a blade; the fight's own movement
        // keeps the distance now (design §4).
        if self.enemy == Some(e.handle()) {
            return false;
        }
        if dist > avoid_dist || self.is_on_ladder() || self.current_waypoint_flags & crate::nav::flags::LADDER != 0 {
            return false;
        }
        if self.tasks.current().and_then(|t| t.edict) == Some(e.handle()) {
            return false;
        }
        if e.solid == SOLID_NOT || e.solid == SOLID_TRIGGER || e.classname.starts_with("func_door") {
            return false;
        }
        if e.flags & entflags::WORLDBRUSH != 0 {
            if self.current_waypoint != -1 && g.waypoints.distance(e.entity_origin(), self.current_waypoint) < 100.0 {
                return false;
            }
            return self.dot_from_origin(e.entity_origin()) > 0.9;
        }
        true
    }

    /// `CBot::UpdateVisibles`: up to `m_iVisionRevs` entities per call, each one in view, in the
    /// PVS and with a clear line counts as visible. True when the pass over every entity ended.
    pub(crate) fn update_visibles(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) -> bool {
        let now = w.time();
        let mut work_costs = false;
        if self.vis_update_revs == 0 {
            self.vis_update_revs = 1;
            self.avoid_entity = None;
            self.clear(cond::SEE_ENEMY | cond::SEE_ENEMY_HEAD);
            let (forward, _) = super::look::make_vectors(self.v_angle);
            let src = self.gun_position();
            let tr = w.trace_line(src, src + forward * 768.0, Monsters::DontIgnore, Glass::DontIgnore, Some(self.index));
            self.distance_from_wall = (tr.end_pos - src).length();
            work_costs = true;
            self.clear_enemy_costs(g, w, rng);
        }
        let need_to_follow = !self.tasks.has_task(TaskKind::FollowLeader) && self.squad_leader.is_some();
        let mut saw_leader = false;
        let avoid_dist = self.me.size.length_2d() * 3.0;
        // Nothing exists above the frame's highest index: the sweep ends there (port addition).
        let top = w.highest_entity().min(w.max_entities());
        let max = (self.vis_update_revs + self.profile.vision_revs.max(1) as u32).min(top);

        // Upstream checks the bot against its own PVS, which always passes: any remembered
        // position counts as visible. Kept.
        match self.remembered.nearest(self.goal_origin.unwrap_or(Vector::ZERO), self.me.origin) {
            Some(pos) => {
                self.nearest_remember_point = pos.origin;
                self.nearest_remember_point_visible = true;
            }
            None => self.nearest_remember_point_visible = false,
        }

        for i in self.vis_update_revs..=max {
            // Upstream's tests in its order, the cheap ones on a glance at the entity: most fail
            // the distance or the view cone, and need no full entity (names, player state).
            let Some((origin, effects)) = w.entity_glance(i) else {
                self.set_visible(i, false);
                continue;
            };
            if effects & EF_NODRAW != 0 {
                self.set_visible(i, false);
                continue;
            }
            let dist = self.distance_from(origin, false);
            if dist > BOT_VIEW_DISTANCE || !self.in_view_cone(origin) {
                self.set_visible(i, false);
                continue;
            }
            let Some(e) = w.entity(i).filter(|_| w.in_pvs(self.index, origin)) else {
                self.set_visible(i, false);
                continue;
            };
            if !self.fvisible(w, &e) {
                self.set_visible(i, false);
                continue;
            }
            if self.enemy == Some(e.handle()) {
                self.set(cond::SEE_ENEMY);
            }
            if (self.avoid_entity.is_none() || dist < self.nearest_avoid_dist)
                && self.current_waypoint_flags & crate::nav::flags::JUMP == 0
                && self.can_avoid(g, &e, dist, avoid_dist)
                && self.avoid_time < now
            {
                self.nearest_avoid_dist = dist;
                self.avoid_entity = Some(e.handle());
            }
            if !saw_leader && self.squad_leader == Some(e.handle()) {
                saw_leader = true;
            }
            self.set_visible(i, true);
            if work_costs {
                self.work_enemy_costs(w, &e, dist);
            }
        }
        let finished = if max >= top {
            self.vis_update_revs = 0;
            // Entities seen above the top are gone (their slots freed since).
            for i in self.visibles.to_vec().into_iter().filter(|&i| i > top) {
                self.set_visible(i, false);
            }
            true
        } else {
            self.vis_update_revs = max + 1;
            false
        };
        self.see_leader_again(w, saw_leader, need_to_follow);
        finished
    }

    /// `CBot::isInAnimate`.
    fn is_inanimate(e: &Entity) -> bool {
        e.flags & (entflags::WORLDBRUSH | entflags::DORMANT | entflags::KILLME) != 0
    }

    /// `CBot::workEnemyCosts`: adds the entity's danger (a survival gene by kind, over distance
    /// squared, scaled by size) to every cost bucket around the bot and remembers the cheapest.
    ///
    /// Upstream gave every bucket the same cost (the distance from the bot), so the cheapest was
    /// always the first, `origin + (-192, -192, origin.z)`, whatever the danger. The port measures
    /// from each bucket to the entity, and takes the gene's size only (a cost repels; the genes'
    /// signs were bred from a fitness that couldn't tell): the cheapest bucket is away from the
    /// danger (design §4).
    pub(crate) fn work_enemy_costs(&mut self, w: &dyn BotWorld, e: &Entity, dist: f32) {
        if self.next_work_range_costs > w.time() || dist > 768.0 {
            return;
        }
        let state = if Self::is_inanimate(e) {
            2
        } else if self.is_enemy(w, e) {
            1
        } else if e.classname.contains("grenade") {
            3
        } else if e.is_alive() {
            0
        } else {
            2
        };
        let mid = (BOT_COST_BUCKETS / 2) as i32;
        let weight = self.learned.survival.get(state).abs() * (e.size.length() / self.me.size.length());
        for x in 0..BOT_COST_BUCKETS {
            for y in 0..BOT_COST_BUCKETS {
                let at =
                    self.me.origin + Vector::new((x as i32 - mid) as f32 * BOT_COST_RANGE, (y as i32 - mid) as f32 * BOT_COST_RANGE, 0.0);
                let d = (at - e.origin).length_2d().max(BOT_COST_RANGE / 4.0);
                let c = self.range_costs[x][y] + weight / (d * d);
                self.range_costs[x][y] = c;
            }
        }
        // The cheapest after this entity, nearest the bot first on ties (the centre outward).
        self.lowest_enemy_cost = f32::MAX;
        let mut best_ring = i32::MAX;
        for x in 0..BOT_COST_BUCKETS {
            for y in 0..BOT_COST_BUCKETS {
                let c = self.range_costs[x][y];
                let ring = (x as i32 - mid).abs().max((y as i32 - mid).abs());
                if c < self.lowest_enemy_cost || (c == self.lowest_enemy_cost && ring < best_ring) {
                    self.lowest_enemy_cost = c;
                    best_ring = ring;
                    let pos = Vector::new((x as i32 - mid) as f32 * BOT_COST_RANGE, (y as i32 - mid) as f32 * BOT_COST_RANGE, 0.0);
                    self.lowest_enemy_cost_vec = self.me.origin + pos;
                }
            }
        }
    }

    /// `CBot::clearEnemyCosts`, which also looks for an enemy when the bot wants one.
    fn clear_enemy_costs(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) {
        self.range_costs = [[0.0; BOT_COST_BUCKETS]; BOT_COST_BUCKETS];
        self.lowest_enemy_cost = 99999.0;
        self.lowest_enemy_cost_vec = self.me.origin;
        let old = self.enemy;
        // `WantToFindEnemy` is answered by the caller's config; `clearEnemyCosts` runs inside
        // `UpdateVisibles`, where upstream asks it again.
        if self.want_to_find_enemy {
            self.enemy = self.find_enemy(g, w, rng);
            if old != self.enemy {
                if let Some(o) = old {
                    self.enemy_down(o);
                }
                if let Some(e) = self.resolve(w, self.enemy) {
                    self.enemy_found(g, &e, w.time(), rng);
                }
            }
        } else {
            self.enemy = None;
        }
    }

    /// `CBot::FindEnemy`, TS, rewritten (design §4): every 0.4-0.6 s the visible enemies (and
    /// whoever hurt the bot in the last 3 s, seen or not, if the line to it is clear) are scored
    /// ([`Bot::threat`]); the enemy is kept unless another scores 25% (and a little) more, or it
    /// is out of sight and another is in it. A new enemy starts a priority `NORMAL_ATTACK`
    /// schedule. Upstream dropped the enemy every 0.75-1.25 s for the nearest, slowest other one
    /// in sight, so two enemies took turns, and only a priority seen first counted (§5.63).
    pub(crate) fn find_enemy(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) -> Option<EntRef> {
        let now = w.time();
        let current = self.resolve(w, self.enemy).filter(|e| self.is_enemy(w, e));
        if current.is_some() && self.next_get_enemy_time >= now {
            return self.enemy;
        }
        if current.is_none() {
            self.enemy = None;
        }
        self.next_get_enemy_time = now + rng.random_float(0.4, 0.6);
        let mut candidates = self.visibles.to_vec();
        if let Some((a, t)) = self.fight.attacker
            && t + 3.0 > now
            && !candidates.contains(&a)
        {
            candidates.push(a);
        }
        let mut best: Option<(Entity, f32, Option<i32>)> = None;
        let mut current_score = 0.0;
        for i in candidates {
            let Some(e) = w.entity(i) else { continue };
            // The visible list can be a vision pass old: check the line now, so a new enemy
            // is never one behind a wall (design §4).
            if !self.is_enemy(w, &e) || !self.fvisible(w, &e) {
                continue;
            }
            let rep = if e.is_client() { self.rep_of(g, e.index) } else { None };
            // What every bot learned of this player (0.25-4x), damped: a known ace is fought
            // first, a known novice later (design §4).
            let learned = if e.is_client() { self.learned_threat(g, e.index).sqrt() } else { 1.0 };
            let score = self.threat(&e, rep, now) * learned;
            if current.as_ref().is_some_and(|c| c.index == e.index) {
                current_score = score;
            }
            if best.as_ref().is_none_or(|(_, b, _)| score > *b) {
                best = Some((e, score, rep));
            }
        }
        if let Some(c) = &current
            && best.as_ref().is_none_or(|(b, s, _)| b.index == c.index || *s <= current_score * 1.25 + 0.5)
        {
            return self.enemy;
        }
        let Some((e, _, rep)) = best else { return self.enemy };
        self.enemy = Some(e.handle());
        self.enemy_rep = rep;
        self.tasks.remove_similar_task(TaskKind::NormalAttack);
        let id = self.tasks.new_schedule_id();
        self.add_priority_task(Task::new(TaskKind::NormalAttack, id, Some(e.handle()), 0, 0.0, Vector::ZERO));
        self.enemy_found(g, &e, now, rng);
        self.enemy
    }

    /// `CBot::EnemyFound`. Upstream set `m_fStrafeTime` to 2-6 *seconds since the map started*
    /// and a strafe speed of 1 (design §5.51); the fight's own strafe replaces both. A new enemy
    /// starts the reaction time (0.35 s, down to 0.15 s with skill: no shot before it) and the
    /// aim's settling (design §4).
    pub(crate) fn enemy_found(&mut self, g: &mut BotGlobals<'_>, e: &Entity, now: f32, rng: &mut dyn Random) {
        self.set(cond::SEE_ENEMY);
        self.enemy = Some(e.handle());
        self.first_see_enemy_time = now;
        // The same enemy found again while still fighting it (its attack task replaced) is no
        // new sight.
        if self.last_enemy != Some(e.handle()) || self.fight.last_sight + 1.0 < now {
            self.fight.react_at = now + 0.35 - 0.2 * self.skill_share();
            self.fight.on_target_since = now;
        }
        self.squad_enemy_found(g.squads);
        self.remembered.add(e.entity_origin(), Some(e.handle()), REMEMBER_1ST_SEE_ENEMY, self.me.origin);
        self.investigate_sound_time = now + 2.0;
        // `BotHasEnemy`: this bot has one now.
        let cam_has_enemy = g.botcam.current_bot() == Some(self.index) || g.botcam.other_has_enemy;
        if !cam_has_enemy {
            g.botcam.set_current_bot(self.index, now, rng);
        }
        // Upstream's TS part (`usepowerup` and a stunt key press, gated by the chat timer) is
        // replaced by the fight's power-up use and dodge dives (design §4).
    }

    /// `CBot::EnemyDown`.
    pub(crate) fn enemy_down(&mut self, _e: EntRef) {
        // `m_vRememberedPositions.removePosition(pEnemy)`; upstream calls this with the enemy
        // already cleared in `NORMAL_ATTACK`, so it removes nothing there.
    }

    /// `CBot::decideJumpDuckStrafe`: the personal genes drive three perceptrons that jump, duck
    /// and strafe in melee.
    pub(crate) fn decide_jump_duck_strafe(&mut self, w: &dyn BotWorld, enemy_dist: f32, enemy_origin: Vector) {
        let now = w.time();
        let l = &mut self.learned;
        l.jump.set_weights_from_values(&l.personal, 0, 5);
        l.duck.set_weights_from_values(&l.personal, 5, 5);
        l.strafe.set_weights_from_values(&l.personal, 10, 5);
        let enemy_size = self.resolve(w, self.enemy).map(|e| e.size.length());
        let inputs = [
            self.me.velocity.length() / self.me.size.length(),
            enemy_dist * 0.01,
            (enemy_origin.z - self.me.origin.z) * 0.1,
            enemy_size.map_or(1.0, |s| self.me.size.length() / s),
            f32::from(self.me.flags & entflags::ONGROUND != 0 && self.me.movetype == super::MOVETYPE_WALK),
        ];
        let l = &mut self.learned;
        for p in [&mut l.jump, &mut l.duck, &mut l.strafe] {
            p.input(&inputs);
            p.execute();
        }
        // The sigmoid's 0..1 mapped to -1..1: upstream's `output * speed` only ever strafed
        // right (design §4); the dead band (0.25-0.75 upstream) is the same.
        let (jump, duck, strafe) = (l.jump.fired(), l.duck.fired(), 2.0 * l.strafe.output() - 1.0);
        if jump {
            self.jump();
        }
        if duck {
            self.jump_and_duck(now);
        }
        if self.strafe_time + 0.75 < now && strafe.abs() >= 0.5 {
            self.strafe_speed = strafe * self.max_speed;
        }
        self.used_melee = true;
    }

    /// `CBot::WantToFollowEnemy`: upstream runs the follow perceptron but ignores it; the bot
    /// follows while above 30% health.
    pub(crate) fn want_to_follow_enemy(&self) -> bool {
        self.me.health > self.me.max_health * 0.3
    }

    /// `CBot::RunForCover`: crouch, then go to a waypoint hidden from `origin` (or near the
    /// cheapest cost bucket), reloading on the way.
    pub(crate) fn run_for_cover(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, origin: Vector, now_flag: bool, rng: &mut dyn Random) {
        let covering = self.tasks.has_schedule(ScheduleDesc::RunForCover);
        if !(now_flag || !covering) {
            return;
        }
        if covering {
            self.tasks.finish_schedule_desc(ScheduleDesc::RunForCover);
        }
        let now = w.time();
        let id = self.tasks.new_schedule_id();
        // Cover where the team doesn't keep dying (the learned heatmap, design §4).
        let heatmap = &g.learned.heatmap;
        let team = self.team;
        let mut cover = g
            .waypoints
            .cover_waypoint_weighted(self.me.origin, origin, &self.paths.failed_goals, w, &|i| heatmap.cover_penalty(i, team))
            .map_or(-1, |i| i as i32);
        self.add_priority_task(Task::new(TaskKind::Crouch, id, None, 0, rng.random_float(1.0, 2.0), Vector::ZERO));
        if cover == -1 {
            cover = self.nearest_waypoint(g, w, self.lowest_enemy_cost_vec, 320.0, -1, true, false, false);
            if cover == -1 {
                return;
            }
        }
        if self.goal_waypoint != cover {
            self.add_priority_task(Task::new(TaskKind::FindPath, id, None, cover, -2.0, Vector::ZERO));
        }
        if self.current_weapon.and_then(|i| self.weapons.get(i)).is_some_and(|wp| wp.need_to_reload()) {
            self.add_priority_task(Task::new(TaskKind::Reload, id, None, 1, 0.0, Vector::ZERO));
        }
        self.tasks.give_schedule_description(id, ScheduleDesc::RunForCover);
        self.tasks.set_time_to_complete_schedule(id, now, rng.random_float(5.0, 10.0));
    }

    /// Runs from a grenade that will land within its 300-unit blast (and a bit) of the bot: any
    /// grenade within 400 units, in view or not, its landing flown out ([`super::think::trace_toss`]).
    /// Upstream fled only the grenade that was its entity to avoid (in view, within about 135
    /// units) and replaced its cover schedule every think (§5.65); an existing one is kept.
    pub(crate) fn flee_grenades(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) {
        for i in w.entities_in_sphere(self.me.origin, 400.0) {
            let Some(e) = w.entity(i).filter(super::think::is_grenade_rocket) else { continue };
            let end = super::think::trace_toss(w, &e);
            if (end - self.me.origin).length() < 330.0 {
                self.run_for_cover(g, w, end, false, rng);
                return;
            }
        }
    }

    /// `RememberPosition(v, enemy, BOT_REMEMBER_LOST_ENEMY)`.
    pub(crate) fn remember_lost_enemy(&mut self, v: Vector, e: Option<EntRef>) {
        self.remembered.add(v, e, REMEMBER_LOST_ENEMY, self.me.origin);
    }

    /// `CBot::UpdateConditions`.
    pub(crate) fn update_conditions(&mut self, w: &dyn BotWorld) {
        let now = w.time();
        self.update_squad_conditions(w);
        if self.last_update_conditions > now {
            return;
        }
        self.clear(cond::HAS_WEAPON | cond::ENEMY_OCCLUDED | cond::TASK_EDICT_NA);
        if self.best_weapon(w, None).0 > 0 {
            self.set(cond::HAS_WEAPON);
        }
        if let Some(e) = self.enemy {
            if !self.has(cond::SEE_ENEMY) {
                self.set(cond::ENEMY_OCCLUDED);
            } else if self.resolve(w, Some(e)).is_none_or(|x| !x.is_alive()) {
                self.set(cond::ENEMY_DEAD);
            }
        }
        if let Some(t) = self.tasks.current().and_then(|t| t.edict)
            && self.resolve(w, Some(t)).is_none()
        {
            self.set(cond::TASK_EDICT_NA);
            self.tasks.forget_edict(t);
        }
        self.last_update_conditions = now + self.reaction_time;
    }

    /// `m_Weapons.GetBestWeaponId(this, pEnemy)`.
    pub(crate) fn best_weapon(&self, w: &dyn BotWorld, enemy: Option<&Entity>) -> (i32, bool) {
        let (dist, too_high) = match enemy {
            Some(e) => {
                let o = e.entity_origin();
                (Some(self.distance_from(o, false)), o.z > self.me.origin.z + crate::nav::MAX_JUMP_HEIGHT)
            }
            None => (None, false),
        };
        let _ = w;
        self.weapons.best_weapon_id(dist, too_high, self.me.waterlevel == 3)
    }
}

/// `pfnFindEntityInSphere`'s test: the entity's box (`absmin`..`absmax`) comes nearer `origin`
/// than `radius`.
pub(crate) fn box_in_sphere(e: &Entity, origin: Vector, radius: f32) -> bool {
    let mut dist_sq = 0.0;
    for (o, (lo, hi)) in [(origin.x, (e.absmin.x, e.absmax.x)), (origin.y, (e.absmin.y, e.absmax.y)), (origin.z, (e.absmin.z, e.absmax.z))]
    {
        let d = if o < lo {
            o - lo
        } else if o > hi {
            o - hi
        } else {
            0.0
        };
        dist_sq += d * d;
    }
    dist_sq < radius * radius
}
