//! Moving (`bot.cpp` `CBot::WorkMoveDirection`, `CheckStuck`, `RunPlayerMove`, `UpdateMsec`;
//! `util.cpp` `UTIL_GetAvoidAngle`).

use super::look::{fix_angles, fix_float_angle, make_vectors, vec_to_angles};
use super::{BOT_STUCK_CLIMB_SPEED, BOT_STUCK_SWIM_SPEED, Bot, BotCmd, BotGlobals, BotWorld, Climb, LookTask, TaskKind, buttons, entflags};
use crate::config::BOT_WAYPOINT_TOUCH_DIST;
use crate::nav::{CONTENTS_WATER, Glass, Monsters, flags as fl};
use crate::{Random, Vector};

/// The shortest usercmd sent, in ms (w00tguy's 10, less half a millisecond: game time is an
/// `f32`, so a 100 Hz frame often measures 9.99 ms).
pub(crate) const MIN_MSEC: f32 = 9.5;

/// When the bot's last usercmd went (`m_fLastCallRunPlayerMove`) and the part of a
/// millisecond it didn't carry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MsecClock {
    /// Game time of the last command; 0 before the first.
    pub last: f32,
    /// What rounding left over, in ms (-0.5..0.5).
    pub carry: f32,
}

/// The `msec` of the bot's next usercmd: the time since the last one plus what the last left
/// over, rounded, from 1 to 255; `None` below [`MIN_MSEC`]. The rest carries over to the next
/// command, so the bot simulates the time that passed (design §4).
///
/// Upstream's `UpdateMsec`/`RunPlayerMove` truncated the time: each command lost up to a
/// millisecond (3-5% of the time at 20-30 ms frames), and at 100 Hz the frames measuring
/// 9.99 ms (game time is an `f32`) were skipped, so the bot ran every other frame at half speed.
/// A new clock (unset, or the time went back: a new map) starts one frame (`frame_time`) ago;
/// upstream's started at 0 and again at each spawn, so the first command was 255 ms.
pub(crate) fn next_msec(clock: &mut MsecClock, now: f32, frame_time: f32) -> Option<u8> {
    if clock.last <= 0.0 || now < clock.last {
        *clock = MsecClock { last: now - frame_time.max(0.0), carry: 0.0 };
    }
    let ms = (now - clock.last) * 1000.0 + clock.carry;
    if ms < MIN_MSEC {
        return None;
    }
    clock.last = now;
    if ms > 255.0 {
        // A pause: drop what one command can't carry rather than catching up for seconds.
        clock.carry = 0.0;
        return Some(255);
    }
    let msec = ms.round().max(1.0);
    clock.carry = ms - msec;
    Some(msec as u8)
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
        self.move_speed = self.ideal_move_speed * radians.cos();
        if self.strafe_time < now {
            self.strafe_speed = self.ideal_move_speed * radians.sin();
        }
        if climb == Climb::Climbing {
            self.button |= buttons::FORWARD;
        }
        if self.move_speed.abs() < 1.0 {
            self.move_speed = 0.0;
        }
        if self.strafe_speed.abs() < 1.0 {
            self.strafe_speed = 0.0;
        }
    }

    /// `CBot::CheckStuck`: no progress for 8 s on a path kills the bot (`m_bKill`); slow for a
    /// second jumps, for two fails the path and strafes away from a wall.
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
            } else if self.last_no_move + 8.0 < now {
                self.last_no_move = 0.0;
                self.kill = true;
            }
        } else {
            self.last_no_move = 0.0;
        }

        let ducking = self.me.flags & entflags::DUCKING != 0;
        if self.stuck_time == 0.0 {
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
                    if self.velocity_2d < self.move_speed / 4.0 {
                        self.stuck_time = now;
                    }
                } else if self.velocity_2d < self.move_speed / 2.0 {
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
                self.last_failed_waypoint = self.current_waypoint;
                if self.current_waypoint != -1 && self.prev_waypoint != -1 {
                    self.fail_path(g, self.prev_waypoint, self.current_waypoint);
                }
                if let Some(t) = self.tasks.current_mut() {
                    t.found_path = false;
                }
                if self.strafe_time < now {
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
            if climb == Climb::None {
                self.jump_and_duck(now);
            }
            self.got_stuck();
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
            if self.velocity_2d >= self.move_speed / 4.0 {
                self.stuck_time = 0.0;
            }
        } else if self.velocity_2d >= self.move_speed / 2.0 {
            self.stuck_time = 0.0;
        }
    }

    /// `CBot::RunPlayerMove`: the usercmd, or `None` while less than [`MIN_MSEC`] has passed
    /// since the last one (w00tguy's fix against slow motion at high frame rates); see
    /// [`next_msec`].
    pub(crate) fn run_player_move(&mut self, now: f32, frame_time: f32) -> Option<BotCmd> {
        let msec = next_msec(&mut self.msec_clock, now, frame_time)?;
        fix_angles(&mut self.v_angle);
        self.ideal_move_speed = if self.current_waypoint_flags & fl::STAY_NEAR != 0 { self.max_speed / 2.0 } else { self.max_speed };
        if self.current_waypoint_flags & fl::CROUCH != 0 || (self.start_duck_time <= now && self.end_duck_time > now) {
            self.duck();
        }
        if self.start_jump_time <= now && self.end_jump_time > now {
            self.jump();
        }
        if self.hold_attack_time > now {
            self.primary_attack();
        }
        let buttons = self.fire_buttons(self.button, now);
        let (forward, side) = self.capped_speeds(now);
        Some(BotCmd { angles: self.v_angle, forward, side, up: self.up_speed, buttons, impulse: 0, msec })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `frames` server frames of `dt` from `start` in `f32` game time, as the engine counts
    /// it: the commands sent and their total msec.
    fn simulate(start: f32, dt: f64, frames: usize) -> (usize, u32, f32) {
        let mut last = MsecClock::default();
        let (mut cmds, mut total) = (0, 0u32);
        let mut t = f64::from(start);
        for _ in 0..frames {
            t += dt;
            if let Some(ms) = next_msec(&mut last, t as f32, dt as f32) {
                cmds += 1;
                total += u32::from(ms);
            }
        }
        (cmds, total, (t - f64::from(start)) as f32 * 1000.0)
    }

    #[test]
    fn usercmds_carry_the_time_that_passed() {
        // 100 Hz at a late game time, where frames measure 9.99 or 10.01 ms: every frame sends.
        let (cmds, total, elapsed) = simulate(1000.0, 0.01, 1000);
        assert_eq!(cmds, 1000);
        assert!((total as f32 - elapsed).abs() <= 10.0, "{total} ms of {elapsed}");
        // 30 Hz: nothing lost to truncation (upstream: 33 of every 33.3 ms).
        let (cmds, total, elapsed) = simulate(50.0, 1.0 / 30.0, 300);
        assert_eq!(cmds, 300);
        assert!((total as f32 - elapsed).abs() <= 34.0, "{total} ms of {elapsed}");
        // 1000 Hz: one command per 10 frames, the time still all there.
        let (cmds, total, elapsed) = simulate(20.0, 0.001, 1000);
        assert!((95..=105).contains(&cmds), "{cmds}");
        assert!((total as f32 - elapsed).abs() <= 10.0, "{total} ms of {elapsed}");
    }

    #[test]
    fn a_new_clock_starts_one_frame_back() {
        // The first command covers the last frame, not the 255 ms upstream sent.
        let mut last = MsecClock::default();
        assert_eq!(next_msec(&mut last, 12.0, 0.02), Some(20));
        // A pause longer than one command: 255, then on from now.
        assert_eq!(next_msec(&mut last, 13.0, 0.02), Some(255));
        assert_eq!(next_msec(&mut last, 13.02, 0.02), Some(20));
        // A new map (time went back): one frame again.
        assert_eq!(next_msec(&mut last, 1.5, 0.05), Some(50));
        // Too soon after the last.
        assert_eq!(next_msec(&mut last, 1.505, 0.005), None);
    }
}
