//! Where the bot looks (`bot.cpp` `CBot::WorkViewAngles`, `SetViewAngles`, `ChangeAngles`
//! (`BotFunc_ChangeAngles`, by Whistler), `GetAimVector`) and the angle helpers of `util.cpp`.

use super::{BOT_FACE_HURT_TIME, BOT_MAX_REP, Bot, BotWorld, Climb, LookTask};
use crate::config::MAX_BOT_SKILL;
use crate::nav::MAX_JUMP_HEIGHT;
use crate::{Random, Vector};

/// `UTIL_FixFloatAngle`: into -180..180 in at most four turns; after four it gives up and
/// returns 0.
pub fn fix_float_angle(a: &mut f32) {
    let mut loops = 0;
    if *a < -180.0 {
        while loops < 4 && *a < -180.0 {
            *a += 360.0;
            loops += 1;
        }
    } else if *a > 180.0 {
        while loops < 4 && *a > 180.0 {
            *a -= 360.0;
            loops += 1;
        }
    }
    if loops >= 4 {
        *a = 0.0;
    }
}

/// `UTIL_FixAngles`.
pub fn fix_angles(v: &mut Vector) {
    fix_float_angle(&mut v.x);
    fix_float_angle(&mut v.y);
    fix_float_angle(&mut v.z);
}

/// `UTIL_AngleDiff`.
pub fn angle_diff(dest: f32, src: f32) -> f32 {
    let mut d = dest - src;
    if dest > src {
        if d >= 180.0 {
            d -= 360.0;
        }
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

/// The engine's `VecToAngles` (`VectorAngles`): pitch up positive, both in 0..360.
pub fn vec_to_angles(v: Vector) -> Vector {
    let (yaw, pitch) = if v.y == 0.0 && v.x == 0.0 {
        (0.0, if v.z > 0.0 { 90.0 } else { 270.0 })
    } else {
        let mut yaw = v.y.atan2(v.x).to_degrees();
        if yaw < 0.0 {
            yaw += 360.0;
        }
        let forward = (v.x * v.x + v.y * v.y).sqrt();
        let mut pitch = v.z.atan2(forward).to_degrees();
        if pitch < 0.0 {
            pitch += 360.0;
        }
        (yaw, pitch)
    };
    Vector::new(pitch, yaw, 0.0)
}

/// The engine's `MakeVectors` (`AngleVectors`): forward and right.
pub fn make_vectors(angles: Vector) -> (Vector, Vector) {
    let (sy, cy) = angles.y.to_radians().sin_cos();
    let (sp, cp) = angles.x.to_radians().sin_cos();
    let (sr, cr) = angles.z.to_radians().sin_cos();
    let forward = Vector::new(cp * cy, cp * sy, -sp);
    let right = Vector::new(-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp);
    (forward, right)
}

/// `DotProduct`.
pub fn dot(a: Vector, b: Vector) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// `BotFunc_ChangeAngles`: turns `current` toward `ideal` by at most `speed` degrees the short
/// way round. Returns the new angle.
pub fn change_angles(speed: &mut f32, ideal: f32, current: &mut f32) {
    let diff = (*current - ideal).abs();
    if diff <= 0.1 {
        *speed = diff;
        return;
    }
    *speed = diff.min(*speed);
    if *current >= 0.0 && ideal >= 0.0 {
        if *current > ideal {
            *current -= *speed;
        } else {
            *current += *speed;
        }
    } else if *current >= 0.0 && ideal < 0.0 {
        if *current - 180.0 > ideal {
            *current += *speed;
        } else {
            *current -= *speed;
        }
    } else if *current < 0.0 && ideal >= 0.0 {
        if *current + 180.0 > ideal {
            *current += *speed;
        } else {
            *current -= *speed;
        }
    } else if *current > ideal {
        *current -= *speed;
    } else {
        *current += *speed;
    }
    fix_float_angle(current);
}

impl Bot {
    /// `UTIL_YawAngleBetweenOrigin(pev, vOrigin)`.
    pub(crate) fn yaw_angle_to(&self, origin: Vector) -> f32 {
        let a = vec_to_angles(origin - self.me.origin);
        let mut f = self.v_angle.y - a.y;
        fix_float_angle(&mut f);
        f
    }

    /// `DotProductFromOrigin`.
    pub(crate) fn dot_from_origin(&self, origin: Vector) -> f32 {
        let (forward, _) = make_vectors(self.v_angle);
        dot((origin - self.gun_position()).normalize(), forward)
    }

    /// `FInViewCone`: within 60 degrees of the view.
    pub(crate) fn in_view_cone(&self, origin: Vector) -> bool {
        self.dot_from_origin(origin) > 0.5
    }

    /// `CBot::WorkViewAngles`.
    pub(crate) fn work_view_angles(&mut self, w: &dyn BotWorld, rng: &mut dyn Random) {
        let now = w.time();
        match self.look_task {
            LookTask::None => {}
            LookTask::FaceNearestRememberPos => {
                if self.nearest_remember_point_visible && self.enemy.is_none() && self.looking_for_enemy {
                    self.set_view_angles(self.nearest_remember_point);
                } else {
                    self.look_task = LookTask::NextWaypoint;
                }
            }
            LookTask::FaceHurtOrigin => {
                if self.enemy.is_some() || self.hurt_time + BOT_FACE_HURT_TIME < now {
                    self.look_task = LookTask::NextWaypoint;
                } else {
                    self.set_view_angles(self.hurt_origin);
                }
            }
            LookTask::NextWaypoint => self.set_view_angles(self.move_to),
            LookTask::LookAround => {
                if self.last_look_time < now {
                    self.last_look_time = now + rng.random_float(2.0, 5.0);
                    let mut yaw = self.ideal_yaw + (rng.random_float(0.0, 180.0) - 90.0);
                    fix_float_angle(&mut yaw);
                    let (forward, _) = make_vectors(Vector::new(0.0, yaw, 0.0));
                    self.current_look_dir = self.gun_position() + forward * 64.0;
                }
                self.set_view_angles(self.current_look_dir);
            }
            LookTask::SearchForLastEnemy => {
                if self.search_enemy_time < now {
                    self.look_task = LookTask::NextWaypoint;
                } else if self.last_enemy.is_none() {
                    self.look_task = LookTask::SearchForEnemy;
                } else {
                    self.set_view_angles(self.last_see_enemy_position);
                }
            }
            LookTask::SearchForEnemy => {
                if self.search_enemy_time < now {
                    self.look_task = LookTask::NextWaypoint;
                    return;
                }
                if self.last_look_time < now {
                    let base = if self.resolve(w, self.last_enemy).is_some_and(|e| e.is_alive()) {
                        self.last_see_enemy_position
                    } else if self.move_to_valid {
                        self.move_to
                    } else {
                        self.look_task = LookTask::LookAround;
                        return;
                    };
                    let r = Vector::new(
                        rng.random_float(0.0, 128.0) - 64.0,
                        rng.random_float(0.0, 128.0) - 64.0,
                        rng.random_float(0.0, 128.0) - 64.0,
                    )
                    .normalize()
                        * 64.0;
                    self.current_look_dir = base + r;
                    self.last_look_time = now + rng.random_float(2.0, 4.0);
                }
                self.set_view_angles(self.current_look_dir);
            }
            LookTask::FaceEnemy => {
                let saved = self.turn_speed;
                self.turn_speed = 15.0 * (1.0 - self.profile.aim_speed);
                if self.turn_speed <= 0.0 {
                    self.turn_speed = 1.0;
                }
                match self.resolve(w, self.enemy) {
                    // A kung fu wall jump faces the wall, not the enemy (design §4).
                    Some(_) if let Some(v) = self.kungfu_face(now) => self.set_view_angles(v),
                    Some(e) => {
                        let aim = self.aim_vector(&e, now, rng);
                        self.set_view_angles(aim);
                    }
                    None => {
                        self.look_task = LookTask::SearchForLastEnemy;
                        self.search_enemy_time = now + rng.random_float(5.0, 8.0);
                    }
                }
                self.turn_speed = saved;
            }
            LookTask::FaceTaskVector => match self.tasks.current().map(|t| t.vector) {
                Some(v) => self.set_view_angles(v),
                None => self.look_task = LookTask::NextWaypoint,
            },
            LookTask::FaceTaskEdict => {
                self.look_task = LookTask::FaceTaskVector;
                if let Some(e) = self.resolve(w, self.tasks.current().and_then(|t| t.edict)) {
                    self.set_view_angles((e.absmin + e.absmax) * 0.5);
                    self.look_task = LookTask::FaceTaskEdict;
                }
            }
        }
    }

    /// `CBot::SetViewAngles`: turns toward `origin`, `m_fTurnSpeed` at a time. Pitch is used
    /// only for the looking tasks, in water, on ladders and for move targets over a jump away.
    pub(crate) fn set_view_angles(&mut self, origin: Vector) {
        let climbing = self.climb_type() != Climb::None;
        let mut use_pitch = false;
        let mut angles;
        if let (true, Some(l)) = (climbing, self.ladder_angles) {
            angles = l;
            use_pitch = true;
        } else {
            angles = vec_to_angles(origin - self.gun_position());
            fix_angles(&mut angles);
            if climbing && self.look_task == LookTask::NextWaypoint {
                // Upstream's ladder direction here reads the waypoint stack; the move vector
                // is what `GetLadderDir()` without arguments uses.
                let dir = if self.move_to.z > self.me.origin.z - 8.0 {
                    1
                } else if self.move_to.z < self.me.origin.z {
                    -1
                } else {
                    0
                };
                if dir == 1 {
                    angles.x = 40.0;
                } else if dir == -1 {
                    angles.x = -40.0;
                }
                self.ladder_angles = Some(angles);
                use_pitch = true;
                self.turn_speed = MAX_JUMP_HEIGHT;
            } else if self.ladder_angles.is_some() {
                self.ladder_angles = None;
            }
        }
        angles.z = 0.0;
        self.v_angle.z = 0.0;
        let z_comp = (self.move_to.z - self.me.origin.z).abs();
        if !use_pitch {
            use_pitch = matches!(
                self.look_task,
                LookTask::SearchForEnemy
                    | LookTask::FaceEnemy
                    | LookTask::FaceTaskVector
                    | LookTask::FaceTaskEdict
                    | LookTask::SearchForLastEnemy
                    | LookTask::LookAround
            );
        }
        if !use_pitch && (self.me.waterlevel > 0 || z_comp > MAX_JUMP_HEIGHT) {
            use_pitch = true;
        }
        self.ideal_pitch = if use_pitch { -angles.x } else { 0.0 };
        self.ideal_yaw = angles.y;
        let mut speed = angle_diff(self.ideal_yaw, self.v_angle.y).abs() / self.turn_speed;
        change_angles(&mut speed, self.ideal_yaw, &mut self.v_angle.y);
        let mut speed = angle_diff(self.ideal_pitch, self.v_angle.x).abs() / self.turn_speed;
        change_angles(&mut speed, self.ideal_pitch, &mut self.v_angle.x);
        self.turn_speed = self.base_turn_speed;
    }

    /// `CBot::GetAimVector`: the enemy's middle plus an offset that shrinks with skill, aim skill
    /// and (for players) reputation, redrawn every `m_fAimTime` or so.
    pub(crate) fn aim_vector(&mut self, enemy: &super::Entity, now: f32, rng: &mut dyn Random) -> Vector {
        let origin = enemy.entity_origin();
        if self.get_aim_vector_time < now {
            let speed = match self.enemy_rep {
                None => {
                    self.aim_factor = (MAX_BOT_SKILL as f32 - self.profile.skill as f32) / MAX_BOT_SKILL as f32;
                    0.5
                }
                Some(rep) => {
                    self.aim_factor = (MAX_BOT_SKILL as f32 - self.profile.skill as f32 + (BOT_MAX_REP - rep) as f32)
                        / (MAX_BOT_SKILL + BOT_MAX_REP) as f32;
                    let s = rep as f32 / BOT_MAX_REP as f32;
                    if s == 0.0 { 0.1 } else { s }
                }
            };
            let factors = enemy.size * 0.5 + enemy.velocity;
            self.aim_factor *= 1.0 - self.profile.aim_skill;
            let (fx, fy, fz) = (self.aim_factor * factors.x, self.aim_factor * factors.y, self.aim_factor * factors.z);
            self.offset_vector = Vector::new(rng.random_float(-fx, fx), rng.random_float(-fy, fy), rng.random_float(-fz, fz));
            if self.zoom {
                self.offset_vector = self.offset_vector * 0.5;
            }
            self.get_aim_vector_time = now + rng.random_float(self.profile.aim_time - 0.2, self.profile.aim_time + 0.2) * speed;
        }
        origin + self.offset_vector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles() {
        let a = vec_to_angles(Vector::new(0.0, 10.0, 0.0));
        assert!((a.y - 90.0).abs() < 1e-4 && a.x == 0.0);
        let a = vec_to_angles(Vector::new(10.0, 0.0, 10.0));
        assert!((a.x - 45.0).abs() < 1e-4);
        let mut f = 190.0;
        fix_float_angle(&mut f);
        assert_eq!(f, -170.0);
        let mut f = 2000.0;
        fix_float_angle(&mut f);
        assert_eq!(f, 0.0);
        assert_eq!(angle_diff(170.0, -170.0), -20.0);
        let (fw, right) = make_vectors(Vector::new(0.0, 90.0, 0.0));
        assert!((fw.y - 1.0).abs() < 1e-6 && (right.x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn turning_takes_the_short_way() {
        let mut cur = 170.0;
        let mut speed = 5.0;
        change_angles(&mut speed, -170.0, &mut cur);
        assert_eq!(cur, 175.0);
        let mut cur = 10.0;
        let mut speed = 50.0;
        change_angles(&mut speed, 20.0, &mut cur);
        assert_eq!((cur, speed), (20.0, 10.0));
    }
}
