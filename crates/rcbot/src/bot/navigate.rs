//! Following waypoints (`bot_navigate.cpp` `BotNavigate_UpdateWaypoint`,
//! `BotNavigate_FindNextWaypoint`, `BotNavigate_ScanFOV`, `BotCheckWallOnLeft/Right`;
//! `bot.cpp` `CBot::touchedWpt`, `gotStuck`).
//!
//! Lifts, pushables, human towers and grenade-throw waypoints are in `special.rs`.

use super::look::{fix_float_angle, make_vectors, vec_to_angles};
use super::{Bot, BotGlobals, BotWorld, Climb, LookTask, Task, TaskKind};
use crate::config::BOT_WAYPOINT_TOUCH_DIST;
use crate::nav::{Glass, Monsters, NearestOptions, REACHABLE_RANGE, flags as fl};
use crate::{Random, Vector};

impl Bot {
    /// `WaypointLocations.NearestWaypoint(origin, range, ignore, visible, unreachable, is_bot)`
    /// with the bot's failed goals when `failed`.
    #[allow(clippy::too_many_arguments)] // upstream's arguments
    pub(crate) fn nearest_waypoint(
        &self,
        g: &BotGlobals<'_>,
        w: &dyn BotWorld,
        origin: Vector,
        range: f32,
        ignore: i32,
        visible: bool,
        unreachable: bool,
        is_bot: bool,
    ) -> i32 {
        let opts = NearestOptions { visible, unreachable, is_bot, aiming_only: false };
        let ignore = usize::try_from(ignore).ok();
        g.waypoints.nearest_waypoint(origin, range, ignore, opts, &[], w).map_or(-1, |i| i as i32)
    }

    /// A roaming goal (`WaypointFindRandomGoal(pEdict, team, &m_FailedGoals)` for TS): any usable
    /// waypoint, more likely the longer the bot hasn't been there, not the failed or backed-off
    /// goals (design §4; upstream drew from the first 50 waypoints only). -1 for none.
    pub(crate) fn roam_goal(&self, g: &BotGlobals<'_>, now: f32, rng: &mut dyn Random) -> i32 {
        let ignore = self.paths.ignored_goals(now);
        let from = self.me.origin;
        let weight = |i: usize| self.explore.weight(g.waypoints, i, from, now) * self.claim_factor(g, i);
        g.waypoints.find_weighted_goal(self.team, 0, &ignore, weight, rng).map_or(-1, |i| i as i32)
    }

    /// How much the bot still wants a goal its teammates have claimed (1: nobody has).
    pub(crate) fn claim_factor(&self, g: &BotGlobals<'_>, waypoint: usize) -> f32 {
        let _ = (g, waypoint);
        1.0
    }

    /// `BotNavigate_AStarAlgo` for the bot: upstream's search, except that a goal no path reaches
    /// is backed off (design §4), and with `tactical_paths` on the costs avoid danger (the heat
    /// map, [`BotGlobals::danger`]), waypoints enemies were lately seen from and teammates' paths.
    pub(crate) fn search_path(
        &mut self,
        g: &mut BotGlobals<'_>,
        w: &dyn BotWorld,
        rng: &mut dyn Random,
        q: &crate::nav::AStarQuery,
        continue_search: bool,
    ) -> crate::nav::AStarResult {
        use crate::nav::{NoDanger, SearchOptions, Tactics};
        let tactical = g.config.is_on(crate::config::flags::TACTICAL_PATHS);
        let (threats, crowded) = if tactical { (self.threat_waypoints(g, w), self.crowded_waypoints(g)) } else { (Vec::new(), Vec::new()) };
        let danger = g.danger.unwrap_or(&NoDanger);
        let tactics = tactical.then_some(Tactics {
            danger,
            danger_weight: 2.0,
            threats: &threats,
            exposure_weight: 1.5,
            crowded: &crowded,
            crowd_weight: 0.5,
        });
        let opts = SearchOptions { back_off_failed_goals: true, tactics };
        self.astar.search_with(&mut self.paths, q, continue_search, g.waypoints, w, rng, &opts)
    }

    /// The waypoints nearest where enemies were lately seen, weighted by how lately.
    pub(crate) fn threat_waypoints(&self, g: &BotGlobals<'_>, w: &dyn BotWorld) -> Vec<(usize, f32)> {
        let mut out: Vec<(usize, f32)> = Vec::new();
        for p in self.remembered.iter() {
            let wpt = self.nearest_waypoint(g, w, p.origin, 400.0, -1, false, false, false);
            if let Ok(i) = usize::try_from(wpt)
                && !out.iter().any(|&(x, _)| x == i)
            {
                out.push((i, 1.0));
            }
        }
        out
    }

    /// Tells the bot's team where it is going (a waypoint, or an item): teammates leave it to
    /// the bot.
    pub(crate) fn claim(&mut self, g: &mut BotGlobals<'_>, waypoint: i32, item: Option<super::EntRef>) {
        let _ = (g, waypoint, item);
    }

    /// A moving search around `center` (`SEARCH_FOR_ENEMY` with a place): walks to random
    /// waypoints within 500 units of it that the bot can see, one after another.
    pub(crate) fn search_move(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, center: Vector, rng: &mut dyn Random) {
        let arrived = self.search_point.is_none_or(|p| self.distance_from(p, true) < BOT_WAYPOINT_TOUCH_DIST * 1.5);
        if arrived {
            self.search_point = None;
            let near: Vec<usize> = (0..g.waypoints.len())
                .filter(|&i| g.waypoints.flags(i as i32) & (fl::DELETED | fl::AIMING) == 0)
                .filter(|&i| {
                    (g.waypoints.origin(i as i32) - center).length() < 500.0
                        && self.distance_from(g.waypoints.origin(i as i32), true) > 64.0
                })
                .collect();
            for _ in 0..4 {
                if near.is_empty() {
                    break;
                }
                let o = g.waypoints.origin(near[rng.random_long(0, near.len() as i32 - 1) as usize] as i32);
                if w.trace_line(self.gun_position(), o, Monsters::Ignore, Glass::Ignore, Some(self.index)).fraction >= 1.0 {
                    self.search_point = Some(o);
                    break;
                }
            }
        }
        match self.search_point {
            Some(p) => self.set_move_vector(p),
            None => self.stop_moving(),
        }
    }

    /// The waypoints on teammates' paths (sorted).
    pub(crate) fn crowded_waypoints(&self, g: &BotGlobals<'_>) -> Vec<usize> {
        let _ = g;
        Vec::new()
    }

    /// Records the path between two waypoints as failed (`m_stFailedPaths.AddFailedPath`).
    pub(crate) fn fail_path(&mut self, g: &BotGlobals<'_>, from: i32, to: i32) {
        let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else { return };
        if let Some(p) = g.waypoints.find_path_from_to(from, to, self.team) {
            self.paths.failed_paths.add(p);
        }
    }

    /// `BotNavigate_FindNextWaypoint`: the next waypoint on the path; -1 to get a new goal (one
    /// is picked, the path is worked out by `FIND_PATH`), 0 when the bot has no waypoint.
    pub(crate) fn find_next_waypoint(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) -> i32 {
        if self.goal_waypoint != -1
            && let Some(index) = self.paths.bot_paths.pop()
        {
            let index = index as i32;
            if index != self.current_waypoint {
                // Missed a waypoint: find the current one further on the path.
                while let Some(&top) = self.paths.bot_paths.last() {
                    if top as i32 == self.current_waypoint {
                        return top as i32;
                    }
                    self.paths.bot_paths.pop();
                }
                if self.prev_waypoint != -1 {
                    self.fail_path(g, self.prev_waypoint, self.current_waypoint);
                }
            } else if let Some(&top) = self.paths.bot_paths.last() {
                return top as i32;
            }
        }
        self.paths.bot_paths.clear();
        if self.goal_waypoint == -1 {
            self.goal_waypoint = self.roam_goal(g, w.time(), rng);
        }
        if self.current_waypoint == -1 {
            self.current_waypoint =
                self.nearest_waypoint(g, w, self.me.origin, REACHABLE_RANGE, self.last_failed_waypoint, true, false, true);
        }
        if self.current_waypoint == -1 {
            return 0;
        }
        -1
    }

    /// `BotNavigate_UpdateWaypoint`: checks whether the bot reached its waypoint and moves on to
    /// the next. False when it has no waypoint to go to.
    pub(crate) fn update_waypoint(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) -> bool {
        let now = w.time();
        let origin = self.me.origin;
        let mut cur = self.current_waypoint;
        let mut touched = false;

        if self.goal_waypoint == -1 {
            self.look_task = if self.resolve(w, self.last_enemy).is_some_and(|e| e.is_alive()) {
                LookTask::SearchForLastEnemy
            } else {
                LookTask::LookAround
            };
            if self.current_waypoint == -1 {
                if self.not_following_waypoint {
                    self.current_waypoint = self.nearest_waypoint(g, w, origin, 200.0, -1, false, false, false);
                }
                if self.current_waypoint == -1 {
                    let get_new = !self.move_to_valid || self.distance_from(self.move_to, true) < BOT_WAYPOINT_TOUCH_DIST;
                    if get_new {
                        let v = self.scan_fov(w, rng);
                        self.set_move_vector(v);
                        self.not_following_waypoint = true;
                    }
                }
            }
        }

        if cur != -1 && !self.has(super::cond::SEE_NEXT_WAYPOINT) {
            if self.last_see_waypoint != 0.0 && self.last_see_waypoint + 2.0 <= now {
                // Not seen for two seconds: give up on it and its path.
                self.fail_path(g, self.prev_waypoint, self.current_waypoint);
                self.current_waypoint = self.nearest_waypoint(g, w, origin, REACHABLE_RANGE, self.last_failed_waypoint, true, false, true);
                cur = self.current_waypoint;
                if let Some(t) = self.tasks.current_mut() {
                    t.found_path = false;
                }
                if self.current_waypoint != -1 {
                    self.prev_waypoint_dist = g.waypoints.distance(origin, self.current_waypoint);
                }
                self.last_see_waypoint = 0.0;
            } else if self.last_see_waypoint == 0.0 {
                self.last_see_waypoint = now;
            }
        }

        if cur == -1 {
            self.current_waypoint = self.nearest_waypoint(g, w, origin, REACHABLE_RANGE, self.last_failed_waypoint, true, false, true);
            if self.current_waypoint == -1 {
                return false;
            }
            self.prev_waypoint_dist = g.waypoints.distance(origin, self.current_waypoint);
            cur = self.current_waypoint;
        }

        self.not_following_waypoint = false;
        let wpt_origin = g.waypoints.origin(cur);
        let wpt_flags = g.waypoints.flags(cur);
        let climb = self.climb_type();
        let distance = if climb != Climb::None {
            (wpt_origin - origin).length()
        } else if (wpt_origin.z - origin.z).abs() < 72.0 {
            (wpt_origin - origin).length_2d()
        } else {
            (wpt_origin - origin).length()
        };

        if fl::on_ladder_or_fly(wpt_flags) || climb == Climb::Climbing {
            let next = self.next_waypoint();
            if next == cur {
                touched = true;
            } else if self.current_waypoint_flags & fl::FLY != 0 {
                // TS bots can't fly (`CanFly`).
                touched = true;
            } else if next != -1 && fl::on_ladder_or_fly(g.waypoints.flags(next)) {
                let dir = self.ladder_dir(false, g.waypoints);
                let next_dir = self.ladder_dir(true, g.waypoints);
                let touching = (wpt_origin - origin).length_2d() <= self.me.size.length_2d() / 2.0;
                touched = touching
                    && match (dir, next_dir) {
                        (0, _) | (-1, 0) | (-1, 1) | (1, 0) | (1, -1) => true,
                        (-1, -1) => wpt_origin.z >= origin.z,
                        (1, 1) => wpt_origin.z <= origin.z,
                        _ => false,
                    };
            } else {
                if self.is_on_ladder()
                    && self.current_waypoint_flags & fl::LADDER == 0
                    && next != -1
                    && self.prev_waypoint != -1
                    && g.waypoints.origin(self.current_waypoint).z <= g.waypoints.origin(self.prev_waypoint).z
                    && (origin - g.waypoints.origin(next)).length_2d() > 100.0
                    && rng.random_long(0, 1) != 0
                {
                    self.jump();
                }
                touched = (wpt_origin - origin).length_2d() <= self.me.size.length_2d() / 2.0
                    && (wpt_origin.z - origin.z).abs() <= self.me.size.z / 2.0;
            }
        } else if wpt_flags & fl::HUMAN_TOWER != 0 {
            touched = self.touched_human_tower(w, wpt_origin, distance);
        } else if wpt_flags & (fl::STAY_NEAR | fl::CROUCH) != 0 {
            if distance <= 24.0 {
                touched = true;
            } else if distance <= 64.0 {
                // `PlayerNearVector`.
                touched = w.entities_in_sphere(wpt_origin, 24.0).into_iter().any(|i| w.entity(i).is_some_and(|e| e.is_client()));
            }
        } else if wpt_flags & (fl::STUNT | fl::JUMP | fl::CROUCHJUMP) != 0 {
            touched = distance < 32.0;
        } else if self.distance_from(wpt_origin, true) < BOT_WAYPOINT_TOUCH_DIST && (wpt_origin.z - origin.z).abs() <= self.me.size.z {
            touched = true;
        } else if self.me.waterlevel > 1 {
            touched = distance < BOT_WAYPOINT_TOUCH_DIST * 2.0;
        } else if self.enemy.is_none() {
            // Walking away from it again: it was passed.
            touched = distance - 1.0 > self.prev_waypoint_dist;
        }

        if touched {
            self.touched_waypoints += 1;
            self.explore.touch(self.current_waypoint, now);
            if let Some(t) = self.tasks.current_mut()
                && t.kind == TaskKind::FindPath
            {
                t.set_time_to_complete(now, rng.random_float(20.0, 30.0));
            }
            self.prev_waypoint_dist = 9999.0;
            if self.current_waypoint == self.goal_waypoint {
                // A goal reached isn't a roaming goal again for a while. Upstream took it off the
                // list instead when it was on it already (design §4).
                let c = self.current_waypoint as usize;
                if !self.paths.failed_goals.contains(&c) {
                    self.paths.failed_goals.push(c);
                    self.paths.next_clear_failed_goals = now + rng.random_float(5.0, 10.0);
                }
            }
            self.ladder_angles = None;
            self.prev_waypoint = self.current_waypoint;
            self.current_waypoint_flags = g.waypoints.flags(self.current_waypoint);
            if self.current_waypoint_flags & fl::JUMP != 0 {
                self.jump_and_duck(now);
            } else if self.current_waypoint_flags & fl::CROUCHJUMP != 0 {
                self.duck_and_jump(now);
            }
            if self.current_waypoint_flags & fl::STUNT != 0 {
                let kung_fu = self.current_weapon.is_none_or(|id| id == crate::weapons::TsWeapon::KungFu.id());
                if !kung_fu || self.enemy.is_none() {
                    // Upstream pressed the key alone, so the bot landed prone and lay there
                    // (design §4).
                    self.dive(now);
                }
            }
            self.touched_pushable_waypoint(w, wpt_origin);
            if self.goal_waypoint == self.current_waypoint {
                if let Some(t) = self.tasks.current().copied()
                    && t.kind == TaskKind::FindPath
                {
                    self.tasks.finished_current(false);
                    if t.float >= 0.0
                        && t.vector.z - origin.z <= crate::nav::MAX_JUMP_HEIGHT
                        && self.distance_from(t.vector, false) > BOT_WAYPOINT_TOUCH_DIST
                    {
                        let id = self.tasks.new_schedule_id();
                        self.add_priority_task(Task::new(TaskKind::MoveToVector, id, t.edict, 0, 0.0, t.vector));
                    }
                }
                self.stop_moving();
                self.goal_waypoint = -1;
                return false;
            }
            let prev_flags = self.current_waypoint_flags;
            self.prev_waypoint = self.current_waypoint;
            self.current_waypoint = self.find_next_waypoint(g, w, rng);
            if g.config.is_on(crate::config::flags::BOTS_WAIT_FOR_BOTS)
                && self.current_waypoint != -1
                && let Some(other) = g.other_bots.iter().find(|b| b.current_waypoint == self.current_waypoint)
            {
                self.add_priority_task(Task::new(
                    TaskKind::WaitForBotAtWpt,
                    0,
                    Some(other.handle),
                    self.current_waypoint,
                    0.0,
                    Vector::ZERO,
                ));
            }
            if self.current_waypoint == -1 {
                self.current_waypoint_flags = 0;
                return false;
            }
            let wpt = g.waypoints.origin(self.current_waypoint);
            self.current_waypoint_flags = g.waypoints.flags(self.current_waypoint);
            self.next_waypoint_tasks(g, w, prev_flags, rng);
            let on_ladder = self.is_on_ladder();
            let wpt_on_ladder = self.current_waypoint_flags & fl::LADDER != 0;
            if !on_ladder && wpt_on_ladder {
                if wpt.z - 16.0 > origin.z {
                    self.jump();
                }
            } else if on_ladder && !wpt_on_ladder {
                self.start_duck_time = now;
                self.end_duck_time = now + 1.0;
            } else if self.current_waypoint_flags & fl::HUMAN_TOWER != 0 {
                self.add_priority_task(Task::new(TaskKind::HumanTower, 0, None, 0, 0.0, wpt));
            }
        } else {
            self.prev_waypoint_dist = distance;
        }

        if self.current_waypoint == -1 {
            self.current_waypoint_flags = 0;
            return false;
        }
        self.current_waypoint_flags = g.waypoints.flags(self.current_waypoint);
        self.set_move_vector(g.waypoints.origin(self.current_waypoint));
        true
    }

    /// `BotNavigate_ScanFOV`: with no waypoints, the furthest open point ahead (or along the
    /// velocity, counted double); jumps when something low is close in front.
    pub(crate) fn scan_fov(&mut self, w: &dyn BotWorld, rng: &mut dyn Random) -> Vector {
        let start = self.gun_position();
        let mut max_distance = BOT_WAYPOINT_TOUCH_DIST * 2.0;
        let mut positions: Vec<Vector> = Vec::new();
        let ignore = Some(self.index);
        let consider = |end: Vector, d: f32, positions: &mut Vec<Vector>, max: &mut f32| {
            if d > *max {
                positions.clear();
                *max = d;
                positions.push(end);
            } else if !positions.is_empty() && (d - *max).abs() < f32::EPSILON {
                positions.push(end);
            }
        };
        let mut step = -60;
        while step <= 60 {
            let mut a = self.me.angles;
            a.y += step as f32 - 30.0;
            fix_float_angle(&mut a.y);
            let (forward, _) = make_vectors(a);
            let end = w.trace_line(start, start + forward * REACHABLE_RANGE, Monsters::Ignore, Glass::Ignore, ignore).end_pos;
            consider(end, self.distance_from(end, false), &mut positions, &mut max_distance);
            step += 10;
        }
        let mut high = 0.0;
        let mut step = -60;
        while step <= 60 {
            let mut a = vec_to_angles(self.me.velocity);
            a.y += step as f32 - 30.0;
            fix_float_angle(&mut a.y);
            let (forward, _) = make_vectors(a);
            let end = w.trace_line(start, start + forward * REACHABLE_RANGE, Monsters::Ignore, Glass::Ignore, ignore).end_pos;
            let d = self.distance_from(end, false) * 2.0;
            if step == 30 {
                high = d;
            }
            consider(end, d, &mut positions, &mut max_distance);
            step += 15;
        }
        let mut low = self.me.origin;
        low.z -= self.me.size.z / 3.0;
        let end = w.trace_line(low, low + self.me.velocity * 2.0, Monsters::Ignore, Glass::Ignore, ignore).end_pos;
        let low_distance = self.distance_from(end, false);
        if low_distance < 64.0 && low_distance < high {
            // A crouch jump clears what a plain jump hits with its feet (design §4).
            self.jump_and_duck(w.time());
        }
        if positions.is_empty() {
            self.look_task = LookTask::NextWaypoint;
            let (forward, _) = make_vectors(Vector::new(0.0, self.me.angles.y, 0.0));
            return w.trace_line(start, start - forward * REACHABLE_RANGE, Monsters::Ignore, Glass::Ignore, ignore).end_pos;
        }
        let i = rng.random_long(0, positions.len() as i32 - 1) as usize;
        positions[i.min(positions.len() - 1)]
    }

    /// `BotCheckWallOnLeft` (`side` -1) / `BotCheckWallOnRight` (1): a wall within 40 units.
    pub(crate) fn wall_on_side(&self, w: &dyn BotWorld, side: f32) -> bool {
        let (_, right) = make_vectors(self.v_angle);
        let src = self.me.origin;
        w.trace_line(src, src + right * (40.0 * side), Monsters::DontIgnore, Glass::DontIgnore, Some(self.index)).fraction < 1.0
    }

    /// `CBot::gotStuck`: back to the previous waypoint, with the one it was heading for on the
    /// path again after it. Upstream decrements the waypoint *index* (design §5.52), and did so on
    /// every frame of the second second of being stuck: fixed (design §4).
    pub(crate) fn got_stuck(&mut self) {
        let (prev, cur) = (self.prev_waypoint, self.current_waypoint);
        if prev != -1 && prev != cur {
            // The path stack's top is the current waypoint: touching `prev` pops it and goes on to
            // `cur` (`find_next_waypoint`).
            if cur != -1 && self.paths.bot_paths.last() != Some(&(cur as usize)) {
                self.paths.bot_paths.push(cur as usize);
            }
            self.paths.bot_paths.push(prev as usize);
            self.current_waypoint = prev;
            self.prev_waypoint = cur;
            // The "walked past it" touch compares with the old waypoint's distance otherwise.
            self.prev_waypoint_dist = 9999.0;
        }
        self.not_following_waypoint = true;
    }
}
