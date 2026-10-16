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
use crate::weapons::ts_data::{self, GunData, mode, powerup};

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
    pub aim_target: Option<super::Vector>,
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
}

/// The radius the bot treats a player as for "on target" and spread (a player's hull is 16 wide).
pub(crate) const AIM_RADIUS: f32 = 14.0;
