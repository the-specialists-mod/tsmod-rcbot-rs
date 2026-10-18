//! Moving (`bot.cpp` `CBot::WorkMoveDirection`, `CheckStuck`, `RunPlayerMove`, `UpdateMsec`;
//! `util.cpp` `UTIL_GetAvoidAngle`).

use super::look::{fix_angles, fix_float_angle, make_vectors, vec_to_angles};
use super::{BOT_STUCK_CLIMB_SPEED, BOT_STUCK_SWIM_SPEED, Bot, BotCmd, BotGlobals, BotWorld, Climb, LookTask, TaskKind, buttons, entflags};
use crate::config::BOT_WAYPOINT_TOUCH_DIST;
use crate::nav::{CONTENTS_WATER, Glass, Monsters, flags as fl};
use crate::{Random, Vector};

/// Seconds without progress on a path before a stuck bot kills itself (upstream: 8, with or
/// without an enemy; design §4).
pub(crate) const STUCK_SUICIDE: f32 = 20.0;
/// A stuck event this soon after the last one, this near where the series began, escalates.
const UNSTICK_SERIES_TIME: f32 = 4.0;
/// See [`UNSTICK_SERIES_TIME`].
const UNSTICK_SERIES_RANGE: f32 = 128.0;
/// How far ahead a wall counts for a wall jump (TS's `CheckForWallJump`).
const WALL_JUMP_RANGE: f32 = 48.0;
/// After a dive, duck is held from this long ...
const DIVE_DUCK_FROM: f32 = 0.15;
/// ... to this long, through the landing, so it rolls out instead of landing prone.
const DIVE_DUCK_UNTIL: f32 = 1.5;
/// A bot still prone this long after its dive jumps to get up.
const DIVE_GET_UP_AFTER: f32 = 0.6;

/// What the bot tried against being stuck (design §4): upstream jumped and went back one waypoint
/// *index* on every frame between 1 and 2 s, failed whatever path that left, and killed the bot
/// after 8 s. The port reacts once per stuck event and escalates while the bot keeps getting stuck
/// at the same place: back to the previous waypoint, back up and strafe, crouch jump, wall jump or
/// dive; at 2 s the path it was on is failed and the bot finds another.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Unstick {
    /// Stuck events in this series (0 for the first).
    pub level: u32,
    /// Where the series began.
    pub at: Vector,
    /// When the series began.
    pub since: f32,
    /// When the last stuck event of the series ended (0: none yet).
    pub last: f32,
    /// The path `(previous, current)` waypoint the bot was on when this event began.
    pub edge: (i32, i32),
    /// The 1 s reaction ran for this event.
    pub reacted: bool,
    /// Backing up (forward speed reversed) until then.
    pub back_until: f32,
    /// Which way it strafes while backing up (-1 left, 1 right).
    pub back_side: f32,
}

impl Bot {
    /// `UTIL_GetAvoidAngle`.
    fn avoid_angle(&self, origin: Vector) -> f32 {
        let (forward, _) = make_vectors(self.v_angle);
        let mut view = vec_to_angles(forward * 4096.0);
        fix_angles(&mut view);
        let mut enemy = vec_to_angles(origin - self.me.origin);
        fix_angles(&mut enemy);
        let mut a = view.y - enemy.y;
        if a > 180.0 {
            a -= 360.0;
        }
        if a < -180.0 {
            a += 360.0;
        }
        -a
    }

    /// The speed to move at now: `m_fIdealMoveSpeed`, at most half the maximum at a stay-near
    /// waypoint or in a stealthy squad. Upstream set the stay-near speed in `RunPlayerMove`, after
    /// this frame's move was worked out (so a frame late), and the stealth speed was overwritten
    /// before anything read it (design §5.62). Fixed (§4).
    fn speed_limit(&self, g: &BotGlobals<'_>) -> f32 {
        let slow =
            self.current_waypoint_flags & fl::STAY_NEAR != 0 || self.squad.and_then(|id| g.squads.get(id)).is_some_and(|s| s.is_stealth());
        if slow { self.ideal_move_speed.min(self.max_speed / 2.0) } else { self.ideal_move_speed }
    }

    /// `CBot::WorkMoveDirection`: forward and side speeds toward `m_vMoveToVector` relative to
    /// the view, around an entity to avoid.
    pub(crate) fn work_move_direction(&mut self, w: &dyn BotWorld, g: &BotGlobals<'_>) {
        let now = w.time();
        let climb = self.climb_type();
        if !self.not_following_waypoint
            && climb == Climb::None
            && self.current_waypoint == -1
            && self.distance_from(self.move_to, false) < BOT_WAYPOINT_TOUCH_DIST
        {
            self.stop_moving();
        }
        let mut null_move = !self.move_to_valid && climb != Climb::Climbing;
        if (self.move_to - self.me.origin).length() < 4.0 {
            null_move = true;
        } else if self.not_following_waypoint {
            null_move = false;
        }
        if self.avoid_entity.is_some() && self.move_to_valid {
            null_move = false;
        }
        if null_move || self.me.flags & entflags::FROZEN != 0 {
            self.move_speed = 0.0;
            // A strafe under way (the fight's, kung fu's circling) goes on without a place to
            // go (design §4; upstream stopped it).
            if self.strafe_time < now || self.me.flags & entflags::FROZEN != 0 {
                self.strafe_speed = 0.0;
            }
            if self.me.waterlevel == 3 {
                self.jump();
            }
            return;
        }
        if climb != Climb::None && self.current_waypoint != -1 {
            self.look_task = LookTask::NextWaypoint;
            self.move_to = g.waypoints.origin(self.current_waypoint);
        }
        if self.current_weapon.and_then(|id| self.weapons.get(id)).is_some_and(|wp| wp.is_melee()) {
            self.avoid_entity = None;
        }
        let mut angle = match self.resolve(w, self.avoid_entity) {
            Some(avoid) => {
                if self.avoid_time <= now && self.is_enemy(w, &avoid) && (avoid.velocity.x != 0.0 || avoid.velocity.y != 0.0) {
                    // Leave room to walk past: my size and a bit.
                    let mut v = ((self.move_to - self.me.origin).normalize() + (self.me.origin - avoid.entity_origin()).normalize())
                        * (self.me.size.length_2d() * 1.1);
                    v = self.me.origin + v;
                    v.z = self.me.origin.z;
                    self.avoid_move_to = v;
                    self.avoid_time = now + 0.2;
                }
                if self.avoid_time > now { self.yaw_angle_to(self.avoid_move_to) } else { self.avoid_angle(avoid.entity_origin()) }
            }
            None => self.yaw_angle_to(self.move_to),
        };
        if self.me.waterlevel > 0 {
            if self.up_time < now {
                if self.move_to.z < self.me.origin.z {
                    self.up_speed = -self.max_speed;
                } else if self.move_to.z > self.me.origin.z {
                    self.up_speed = self.max_speed;
                }
            }
            if self.up_speed > 0.0 && w.point_contents(self.move_to) != CONTENTS_WATER {
                self.button &= !buttons::JUMP;
                self.up_speed = (self.move_to.z - self.me.origin.z).min(self.max_speed);
            }
        }
        fix_float_angle(&mut angle);
        let radians = angle * std::f32::consts::PI / 180.0;
        let speed = self.speed_limit(g);
        self.move_speed = speed * radians.cos();
        if self.strafe_time < now {
            self.strafe_speed = speed * radians.sin();
        }
        if climb == Climb::Climbing {
            self.button |= buttons::FORWARD;
        }
        if self.unstick.back_until > now && climb == Climb::None {
            // Stuck again at the same place: step back from it, sideways (design §4).
            self.move_speed = -self.move_speed;
            self.strafe_speed = self.unstick.back_side * self.ideal_move_speed * 0.7;
        }
        if self.move_speed.abs() < 1.0 {
            self.move_speed = 0.0;
        }
        if self.strafe_speed.abs() < 1.0 {
            self.strafe_speed = 0.0;
        }
    }

    /// The stuck event's reaction after a second ([`Unstick`]): the first time at a place back to
    /// the previous waypoint with a crouch jump, then backing up and strafing, a crouch jump, and
    /// from then on a wall jump facing a wall or else a dive.
    fn react_to_stuck(&mut self, w: &dyn BotWorld, now: f32, climb: Climb, rng: &mut dyn Random) {
        let u = &mut self.unstick;
        u.edge = (self.prev_waypoint, self.current_waypoint);
        if u.last > 0.0 && now - u.last < UNSTICK_SERIES_TIME && (self.me.origin - u.at).length() < UNSTICK_SERIES_RANGE {
            u.level += 1;
        } else {
            u.level = 0;
            u.at = self.me.origin;
            u.since = now;
        }
        if u.level >= 4 && now - u.since > STUCK_SUICIDE {
            // Nothing got it away from here: the last resort, never in a fight.
            u.since = now;
            self.give_up_stuck(now);
        }
        if climb == Climb::Climbing {
            self.got_stuck();
            return;
        }
        match self.unstick.level {
            0 => {
                self.jump_and_duck(now);
                self.got_stuck();
            }
            1 => {
                self.unstick.back_until = now + rng.random_float(0.6, 1.0);
                self.unstick.back_side = if rng.random_long(0, 1) == 0 { -1.0 } else { 1.0 };
            }
            2 => self.jump_and_duck(now),
            _ => {
                let (forward, _) = make_vectors(Vector::new(0.0, self.v_angle.y, 0.0));
                let from = self.me.origin;
                let wall = w.trace_line(from, from + forward * WALL_JUMP_RANGE, Monsters::Ignore, Glass::DontIgnore, Some(self.index));
                if !self.stunt_ready(now) {
                    // A stunt cooling down would eat the press: a crouch jump meanwhile.
                    self.jump_and_duck(now);
                } else if wall.fraction < 1.0 && self.me.flags & entflags::ONGROUND != 0 {
                    // TS's wall jump: `+alt1` facing a wall throws the player up and back off it.
                    self.alt_button();
                } else {
                    self.dive(now);
                }
            }
        }
    }

    /// The last resort against being stuck: suicide (`m_bKill`), never in a fight.
    fn give_up_stuck(&mut self, now: f32) {
        if self.enemy.is_none() && self.last_see_enemy_time + 5.0 < now {
            self.kill = true;
            self.stuck_suicides += 1;
        }
    }

    /// TS takes a stunt now: none running and its cooldown over (the game's `next_stunt_at`;
    /// without the timers, not in a stunt). A press before then is lost (design §4).
    pub(crate) fn stunt_ready(&self, now: f32) -> bool {
        self.state.stunt_state != super::TS_STATE_STUNT
            && !self.state.stunting()
            && self.state.combat.is_none_or(|c| c.next_stunt_at <= now)
    }

    /// A dive (`+alt1` while moving, TS's stunt key), with duck held through the landing so the
    /// bot rolls out of it instead of lying prone ([`Bot::dive_landing`]). Nothing while a stunt
    /// is cooling down; false then.
    pub(crate) fn dive(&mut self, now: f32) -> bool {
        if !self.stunt_ready(now) {
            return false;
        }
        self.alt_button();
        self.dive_time = now;
        self.start_duck_time = now + DIVE_DUCK_FROM;
        self.end_duck_time = now + DIVE_DUCK_UNTIL;
        true
    }

    /// After a dive: a bot that landed prone anyway jumps to get up (TS dives land prone unless
    /// duck, jump or the stunt key is held; design §4): as soon as the game lets it (its
    /// `prone_lock` over), or, without the timers, 0.6 s after the dive.
    pub(crate) fn dive_landing(&mut self, now: f32) {
        if self.dive_time <= 0.0 {
            return;
        }
        let since = now - self.dive_time;
        let prone = match self.state.combat {
            Some(c) => self.state.prone() && c.prone_lock <= now,
            None => since > DIVE_GET_UP_AFTER && self.state.stunt_state == super::TS_STATE_STUNT,
        };
        if since > 3.0 {
            self.dive_time = 0.0;
        } else if prone && self.me.flags & entflags::ONGROUND != 0 {
            self.start_jump_time = now;
            self.end_jump_time = now + 0.1;
            self.dive_time = 0.0;
        }
    }

    /// `CBot::CheckStuck`: no progress on a path for [`STUCK_SUICIDE`] seconds out of a fight kills
    /// the bot (`m_bKill`; upstream: 8 s); slow for a second it reacts ([`Bot::react_to_stuck`]), for
    /// two fails the path it was on and strafes away from a wall.
    pub(crate) fn check_stuck(&mut self, w: &dyn BotWorld, g: &BotGlobals<'_>, rng: &mut dyn Random) {
        let now = w.time();
        let climb = self.climb_type();
        let stuck_speed = g.config.bot_stuck_speed;
        let mut check = false;
        if (stuck_speed - -1.0).abs() > 0.00001 {
            let task = self.tasks.current().map(|t| t.kind);
            check = self.ideal_move_speed > 0.0
                && self.move_to_valid
                && self.me.waterlevel < 3
                && task.is_none_or(|k| k == TaskKind::FindPath || k == TaskKind::MoveToVector);
        }
        if check {
            if self.last_no_move == 0.0 {
                if self.velocity_3d <= stuck_speed {
                    self.last_no_move = now;
                }
            } else if self.velocity_3d > stuck_speed {
                self.last_no_move = 0.0;
            } else if self.last_no_move + STUCK_SUICIDE < now {
                // Upstream: after 8 s, whatever happened.
                self.last_no_move = 0.0;
                self.give_up_stuck(now);
            }
        } else {
            self.last_no_move = 0.0;
        }

        let ducking = self.me.flags & entflags::DUCKING != 0;
        // The speed asked for, sideways included: a bot looking aside (pre-aiming) walks with
        // the strafe (upstream compared the forward speed alone).
        let wanted = (self.move_speed * self.move_speed + self.strafe_speed * self.strafe_speed).sqrt();
        if self.stuck_time == 0.0 {
            self.unstick.reacted = false;
            if self.me.waterlevel != 3 {
                if climb != Climb::None {
                    let dir = self.ladder_dir(false, g.waypoints);
                    self.button &= !buttons::JUMP;
                    if (dir == 1 && self.me.velocity.z < BOT_STUCK_CLIMB_SPEED)
                        || (dir == -1 && self.me.velocity.z > -BOT_STUCK_CLIMB_SPEED)
                    {
                        self.stuck_time = now;
                    }
                } else if ducking {
                    if self.velocity_2d < wanted / 4.0 {
                        self.stuck_time = now;
                    }
                } else if self.velocity_2d < wanted / 2.0 {
                    self.stuck_time = now;
                }
            } else if self.velocity_3d < BOT_STUCK_SWIM_SPEED {
                self.stuck_time = now;
            }
        } else if self.stuck_time + 2.0 < now {
            if self.me.waterlevel == 3 {
                self.up_time = now + rng.random_float(2.0, 3.0);
                let gun = self.gun_position();
                let tr = w.trace_line(gun, gun + Vector::new(0.0, 0.0, 64.0), Monsters::Ignore, Glass::DontIgnore, Some(self.index));
                self.up_speed = if tr.fraction < 1.0 { -self.max_speed } else { self.max_speed };
            } else {
                if climb != Climb::None {
                    if self.stuck_time + 4.0 < now {
                        self.ladder_angles = None;
                    } else {
                        return;
                    }
                } else {
                    self.jump_and_duck(now);
                }
                // The path the bot was on when it got stuck fails (upstream failed whatever its
                // index decrements had left), and the bot finds another way from the nearest
                // waypoint but that one (design §4).
                let (prev, cur) = if self.unstick.reacted { self.unstick.edge } else { (self.prev_waypoint, self.current_waypoint) };
                self.last_failed_waypoint = cur;
                if cur != -1 && prev != -1 {
                    self.fail_path(g, prev, cur);
                }
                if let Some(t) = self.tasks.current_mut() {
                    t.found_path = false;
                }
                self.unstick.last = now;
                if self.strafe_time < now && self.unstick.back_until <= now {
                    if self.wall_on_side(w, -1.0) {
                        self.strafe_time = now + rng.random_float(1.5, 2.5);
                        self.strafe_speed = self.max_speed;
                    } else if self.wall_on_side(w, 1.0) {
                        self.strafe_time = now + rng.random_float(1.5, 2.5);
                        self.strafe_speed = -self.max_speed;
                    }
                }
            }
            self.stuck_time = 0.0;
        } else if self.stuck_time + 1.0 < now {
            // Once per stuck event (upstream: every frame, each time one waypoint index back).
            if !self.unstick.reacted {
                self.unstick.reacted = true;
                self.react_to_stuck(w, now, climb, rng);
            }
        } else if self.me.waterlevel == 3 {
            if self.velocity_3d >= BOT_STUCK_SWIM_SPEED {
                self.stuck_time = 0.0;
            }
        } else if climb != Climb::None {
            let dir = self.ladder_dir(false, g.waypoints);
            if (dir == 1 && self.me.velocity.z >= BOT_STUCK_CLIMB_SPEED) || (dir == -1 && self.me.velocity.z <= -BOT_STUCK_CLIMB_SPEED) {
                self.stuck_time = 0.0;
            }
        } else if ducking {
            if self.velocity_2d >= wanted / 4.0 {
                self.stuck_time = 0.0;
            }
        } else if self.velocity_2d >= wanted / 2.0 {
            self.stuck_time = 0.0;
        }
    }

    /// `CBot::RunPlayerMove`: the usercmd. Its `msec` is the server's (the host sends the
    /// plugin API's `AUTO_MSEC`): the frame time with the fraction carried per bot, so a command
    /// goes every frame and the bot's commands add up to the server's time (design §4).
    pub(crate) fn run_player_move(&mut self, now: f32) -> BotCmd {
        fix_angles(&mut self.v_angle);
        // The stay-near speed is applied where the move is worked out (`speed_limit`).
        if self.current_waypoint_flags & fl::CROUCH != 0 || (self.start_duck_time <= now && self.end_duck_time > now) {
            self.duck();
        }
        if self.start_jump_time <= now && self.end_jump_time > now {
            self.jump();
        }
        if self.hold_attack_time > now {
            self.primary_attack();
        }
        let mut buttons = self.fire_buttons(self.button, now);
        let (forward, side) = self.capped_speeds(now);
        buttons |= prone_strafe_keys(self.state.prone(), side);
        BotCmd { angles: self.v_angle, forward, side, up: self.up_speed, buttons, impulse: 0 }
    }
}

/// The strafe key a player would hold for `side` while prone (design §4): TS rolls a prone player
/// sideways on `IN_MOVELEFT` / `IN_MOVERIGHT` (`HandleSlide`), lying facing the aim afterwards,
/// and the forward move is 0 there. Speed alone only slides the lying body at 0.3 of it, still
/// facing the dive. Standing, the keys stay off (ladders read them, kung fu doesn't).
pub(crate) fn prone_strafe_keys(prone: bool, side: f32) -> i32 {
    if !prone || side == 0.0 {
        0
    } else if side < 0.0 {
        buttons::MOVELEFT
    } else {
        buttons::MOVERIGHT
    }
}
