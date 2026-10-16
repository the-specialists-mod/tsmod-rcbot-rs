//! How the bot works its trigger in TS 3.0 (no upstream counterpart; design §4): presses timed to
//! the gun's fire mode and rate, and the recoil penalty its shots build up, which decides when
//! another shot can hit.
//!
//! Upstream held `+attack` for most of every think, which in TS 3.0 fires a semi-automatic or
//! burst gun once and then never again until the key is let go (`CTSGun::PrimaryAttack` fires
//! those on a fresh press only).

use super::look::{dot, make_vectors};
use super::{Bot, Entity, buttons};
use crate::weapons::TsWeapon;
use crate::weapons::ts_data::GunKind;
use crate::weapons::ts_data::{self, GunData, mode, powerup};
use crate::{Random, Vector};

/// The bot's fight state that isn't upstream's (`CBot` has no such fields).
#[derive(Clone, Debug, Default)]
pub(crate) struct Fight {
    /// The attack bits of the last usercmd sent (a press is fresh when its bit wasn't in it).
    pub sent: i32,
    /// When the next fresh press may fire.
    pub next_press: f32,
    /// Akimbo semi: the left gun (`+attack2`) is next.
    pub akimbo_left: bool,
    /// The weapon and clip last seen, to count shots by clip.
    pub shot_weapon: i32,
    /// See `shot_weapon`.
    pub shot_clip: i32,
    /// When the last shot was counted.
    pub last_shot: f32,
    /// The recoil penalty as the game keeps it (`CTSGun::m_flSpreadPenalty`), estimated.
    pub penalty: f32,
    /// When `penalty` was last decayed.
    pub penalty_time: f32,
    /// Where the aim was last put (the aim point with the bot's error), to judge the view by.
    pub aim_target: Option<Vector>,
    /// The aim point without the error (led), whose angular speed the turn follows.
    pub aim_true: Option<Vector>,
    /// The aim error's direction in the view plane (radians) and its share of the error size.
    pub error_dir: f32,
    /// See `error_dir`.
    pub error_size: f32,
    /// Since when the view has been within 10 degrees of the target (the error settles).
    pub on_target_since: f32,
    /// The view's turn rate in the last aimed frame, degrees a second.
    pub turn_rate: f32,
    /// When the view was last turned at an enemy, and the aim point's (pitch, yaw) then.
    pub last_turn: f32,
    /// See `last_turn`.
    pub last_ideal: Option<(f32, f32)>,
    /// No shot before this (the reaction to a new enemy).
    pub react_at: f32,
    /// The fight's strafe: -1 left, 1 right, until when.
    pub strafe_dir: f32,
    /// See `strafe_dir`.
    pub strafe_until: f32,
    /// A speed cap (the M60's accuracy limit) and until when it holds.
    pub speed_cap: (f32, f32),
    /// The fight's spread limit ([`Bot::spread_limit`]) and until when it holds: automatic fire
    /// lets go of the trigger in any usercmd the penalty passes it, between thinks too.
    pub spread_limit: (f32, f32),
}

impl Bot {
    /// The TS data of the gun in hand (`None` for kung fu).
    pub(crate) fn gun_in_hand(&self) -> Option<GunData> {
        self.current_weapon.filter(|&id| id != TsWeapon::KungFu.id()).and_then(ts_data::gun_data)
    }

    /// Whether the double fire rate power-up runs.
    pub(crate) fn double_fire_rate(&self) -> bool {
        self.state.active_powerup & powerup::DFIRERATE != 0
    }

    /// Counts the shots the clip shows and keeps the recoil penalty as `ScaleSpread` and
    /// `PreThink` do: the Barrett fired faster than 35 units/s starts from 0.3, the M60 faster
    /// than 175 kicks five times as hard.
    pub(crate) fn track_shots(&mut self, now: f32) {
        let scale = if self.state.time_scale > 0.0 { self.state.time_scale } else { 1.0 };
        let dfr = self.double_fire_rate();
        let speed = self.velocity_2d;
        let gun = self.gun_in_hand();
        let (weapon, clip) = (self.state.weapon, self.state.clip);
        let f = &mut self.fight;
        f.penalty = ts_data::decay_penalty(f.penalty, (now - f.penalty_time).max(0.0), scale);
        f.penalty_time = now;
        if let Some(gun) = gun
            && weapon == f.shot_weapon
            && clip < f.shot_clip
            && !gun.is_melee_or_thrown()
        {
            for _ in 0..(f.shot_clip - clip).min(3) {
                if gun.id == TsWeapon::BarrettM82.id() && speed > 35.0 {
                    f.penalty = 0.3;
                }
                let kick = if gun.id == TsWeapon::M60.id() && speed > 175.0 { 5.0 } else { 1.0 };
                if let Some(p) = gun.grow_penalty(f.penalty, now - f.last_shot, scale, dfr, kick) {
                    f.penalty = p;
                }
                f.last_shot = now;
            }
        }
        f.shot_weapon = weapon;
        f.shot_clip = clip;
    }

    /// The trigger as TS reads it: `+attack` wanted on a semi-automatic or burst gun becomes a
    /// press of one usercmd, released at least one, no faster than the gun fires; akimbo semi
    /// alternates `+attack` and `+attack2` (each gun its own button). Automatic, pump and blades
    /// fire while held and pass as they are.
    pub(crate) fn fire_buttons(&mut self, mut b: i32, now: f32) -> i32 {
        self.track_shots(now);
        let dfr = self.double_fire_rate();
        let fire_mode = self.state.fire_mode;
        let (limit, until) = self.fight.spread_limit;
        if b & buttons::ATTACK != 0
            && until > now
            && let Some(gun) = self.gun_in_hand()
            && gun.scale_spread
            && gun.accuracy + self.fight.penalty > limit
        {
            b &= !buttons::ATTACK;
        }
        if b & buttons::ATTACK != 0
            && let Some(gun) = self.gun_in_hand()
            && gun.needs_fresh_press(fire_mode, dfr)
        {
            let f = &mut self.fight;
            let akimbo = fire_mode == mode::AKIMBO_SEMI;
            let key = if akimbo && f.akimbo_left { buttons::ATTACK2 } else { buttons::ATTACK };
            b &= !buttons::ATTACK;
            if f.sent & key == 0 && now >= f.next_press {
                b |= key;
                f.next_press = now + gun.press_interval(fire_mode);
                if akimbo {
                    f.akimbo_left = !f.akimbo_left;
                }
            }
        }
        self.fight.sent = b & (buttons::ATTACK | buttons::ATTACK2 | buttons::ALT1);
        b
    }

    /// Safe to top up the clip: nothing in sight for 1.5 s, and no infinite clip running.
    pub(crate) fn safe_to_reload(&self, now: f32) -> bool {
        self.state.active_powerup & powerup::INFAMMO == 0 && (self.last_see_enemy_time == 0.0 || self.last_see_enemy_time + 1.5 < now)
    }

    /// The view is on the target: within 1.5 times the angle a 14-unit radius makes at `dist`
    /// of where the bot means to aim (its aim point with its aim error).
    pub(crate) fn in_aim(&self, enemy: &Entity, dist: f32) -> bool {
        let target = self.fight.aim_target.unwrap_or_else(|| self.aim_point(enemy));
        let (forward, _) = make_vectors(self.v_angle);
        let to = (target - self.gun_position()).normalize();
        let off = dot(forward, to).clamp(-1.0, 1.0).acos();
        off <= 1.5 * (AIM_RADIUS / dist.max(1.0)).atan()
    }

    /// The usercmd's forward and side speeds under the fight's speed cap (the M60 loses
    /// accuracy above 175 units/s).
    pub(crate) fn capped_speeds(&self, now: f32) -> (f32, f32) {
        let (f, s) = (self.move_speed, self.strafe_speed);
        let (cap, until) = self.fight.speed_cap;
        let speed = (f * f + s * s).sqrt();
        if until > now && cap > 0.0 && speed > cap { (f * cap / speed, s * cap / speed) } else { (f, s) }
    }

    /// The most spread a shot at `dist` may have: `k` times the angle a player makes there (`k`
    /// 1-3 with skill), or as tight as the gun gets; twice that with an infinite clip (rounds
    /// cost nothing: spray).
    pub(crate) fn spread_limit(&self, dist: f32) -> f32 {
        let Some(g) = self.gun_in_hand() else { return f32::MAX };
        let skill = self.skill_share();
        let limit = ((1.0 + 2.0 * skill) * AIM_RADIUS / dist.max(1.0)).max(g.accuracy + 0.003);
        if self.state.active_powerup & powerup::INFAMMO != 0 { limit * 2.0 } else { limit }
    }

    /// The gun's spread allows a hit: base spread plus the recoil penalty within
    /// [`spread_limit`](Self::spread_limit). The Barrett moving faster than 35 units/s fires at
    /// 0.3 (`StandardFire`): never then.
    pub(crate) fn spread_ok(&self, dist: f32) -> bool {
        let Some(g) = self.gun_in_hand() else { return true };
        if g.id == TsWeapon::BarrettM82.id() && self.velocity_2d > 35.0 {
            return false;
        }
        !g.scale_spread || g.accuracy + self.fight.penalty <= self.spread_limit(dist)
    }

    /// The skill as a share of the most (0-1).
    pub(crate) fn skill_share(&self) -> f32 {
        (self.profile.skill as f32 / crate::config::MAX_BOT_SKILL as f32).clamp(0.0, 1.0)
    }

    /// A shot now can hit: the reaction time since the enemy appeared has passed, the view is on
    /// the aim, and the spread allows it (design §4; upstream fired on 90% of thinks wherever it
    /// looked).
    pub(crate) fn ready_to_fire(&self, enemy: &Entity, dist: f32, now: f32) -> bool {
        now >= self.fight.react_at && self.in_aim(enemy, dist) && self.spread_ok(dist)
    }

    /// Moving in a gunfight (design §4; upstream walked to `m_vLowestEnemyCostVec` within 256
    /// units, always 192 units to -x/-y, §5.55): to the gun's range (close in with a shotgun,
    /// back off with a rifle or the Barrett), strafing left and right in 0.25-0.6 s spells at
    /// full speed (moving costs TS 3.0 guns no accuracy), crouching in one spell in seven. The
    /// Barrett stands still to shoot; the M60 keeps under 175 units/s.
    pub(crate) fn combat_move(&mut self, enemy_origin: Vector, dist: f32, now: f32, rng: &mut dyn Random) {
        let gun = self.gun_in_hand();
        let (near, far) = match gun.map(|g| (g.kind, g.pellets > 1.0)) {
            Some((_, true)) | Some((GunKind::Shotgun, _)) => (0.0, 250.0),
            Some((GunKind::Sniper, _)) => (500.0, f32::MAX),
            Some((GunKind::Rifle, _)) => (300.0, 1400.0),
            _ => (150.0, 800.0),
        };
        let mut away = self.me.origin - enemy_origin;
        away.z = 0.0;
        let away = away.normalize();
        if dist > far {
            self.set_move_vector(enemy_origin);
        } else if dist < near {
            self.set_move_vector(self.me.origin + away * 128.0);
        } else {
            self.stop_moving();
        }
        if gun.is_some_and(|g| g.id == TsWeapon::BarrettM82.id()) {
            self.stop_moving();
            self.strafe_speed = 0.0;
            self.strafe_time = 0.0;
            return;
        }
        if gun.is_some_and(|g| g.id == TsWeapon::M60.id()) {
            self.fight.speed_cap = (170.0, now + 0.2);
        }
        if self.fight.strafe_until < now {
            // Mostly the other way: a change of direction is what throws an aim off.
            let flip = rng.random_long(0, 100) < 70;
            self.fight.strafe_dir = if self.fight.strafe_dir == 0.0 || !flip {
                if rng.random_long(0, 1) == 0 { -1.0 } else { 1.0 }
            } else {
                -self.fight.strafe_dir
            };
            self.fight.strafe_until = now + rng.random_float(0.25, 0.6);
            if rng.random_long(0, 6) == 0 {
                self.start_duck_time = now;
                self.end_duck_time = self.fight.strafe_until;
            }
        }
        self.strafe_time = self.fight.strafe_until;
        self.strafe_speed = self.fight.strafe_dir * self.max_speed;
    }
}

/// The radius the bot treats a player as for "on target" and spread (a player's hull is 16 wide).
pub(crate) const AIM_RADIUS: f32 = 14.0;
