//! How the bot works its trigger in TS 3.0 (no upstream counterpart; design §4): presses timed to
//! the gun's fire mode and rate, and the recoil penalty its shots build up, which decides when
//! another shot can hit.
//!
//! Upstream held `+attack` for most of every think, which in TS 3.0 fires a semi-automatic or
//! burst gun once and then never again until the key is let go (`CTSGun::PrimaryAttack` fires
//! those on a fresh press only).

use super::BotGlobals;
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
    /// No dodge dive before this.
    pub next_dive: f32,
    /// A dodge dive to press (`+alt1` in a usercmd without it before) from this time.
    pub dive_at: Option<f32>,
    /// `+duck` held until then, to roll out of the dive's landing instead of lying prone.
    pub roll_until: f32,
    /// Who last hurt the bot, and when.
    pub attacker: Option<(u32, f32)>,
    /// When the weapon for the range is next chosen.
    pub next_weapon_choice: f32,
    /// When the enemy was last in sight (a clear line to it), 0 for not yet.
    pub last_sight: f32,
    /// The enemy's velocity then.
    pub sight_velocity: Vector,
    /// Its head then.
    pub sight_head: Vector,
    /// Holding the angle where the enemy went out of sight until then (0: not decided, -1: no).
    pub hold_until: f32,
    /// No grenade thrown before this.
    pub next_grenade: f32,
    /// No stored power-up used before this.
    pub next_powerup: f32,
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
        // A dodge dive: `+alt1` in one usercmd (TS dives on a fresh press while moving), then
        // `+duck` through the landing, which rolls out instead of lying prone.
        let f = &mut self.fight;
        if let Some(at) = f.dive_at
            && now >= at
        {
            if f.sent & buttons::ALT1 == 0 {
                b = (b | buttons::ALT1) & !buttons::DUCK;
                f.dive_at = None;
                f.roll_until = now + 1.2;
            } else {
                b &= !buttons::ALT1;
            }
        } else if f.roll_until > now {
            b |= buttons::DUCK;
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

    /// A dodge dive (design §4; upstream pressed `+alt1` on every think its stunt perceptron
    /// fired): while moving sideways faster than 80 units/s, just hurt or with the enemy's view
    /// on the bot, light enough to dive (TS 3.0's low dive needs fewer than 62 of 81 slots used),
    /// on the ground, not in a stunt nor while one cools down (the game's timer), at most once
    /// every 3-6 s; when the perceptron fires, or by a
    /// skill-scaled chance a think.
    pub(crate) fn dodge(&mut self, enemy: &Entity, perceptron: bool, now: f32, rng: &mut dyn Random) {
        let f = &self.fight;
        if f.next_dive > now
            || f.dive_at.is_some()
            || !self.stunt_ready(now)
            || self.me.flags & super::entflags::ONGROUND == 0
            || 81 - self.state.free_slots >= 62
        {
            return;
        }
        let (_, right) = make_vectors(Vector::new(0.0, self.v_angle.y, 0.0));
        if dot(self.me.velocity, right).abs() <= 80.0 {
            return;
        }
        let hurt = self.hurt_time > 0.0 && self.hurt_time + 0.5 > now;
        let (enemy_forward, _) = make_vectors(enemy.v_angle);
        let aimed_at = dot(enemy_forward, (self.gun_position() - (enemy.origin + enemy.view_ofs)).normalize()) > 0.95;
        if !(hurt || aimed_at) || !(perceptron || rng.random_float(0.0, 1.0) < 0.05 + 0.1 * self.skill_share()) {
            return;
        }
        self.fight.dive_at = Some(now);
        self.fight.next_dive = now + rng.random_float(3.0, 6.0);
    }

    /// The enemy is in sight: remember how it moved and where its head was, for when it isn't.
    pub(crate) fn saw_enemy(&mut self, enemy: &Entity, now: f32) {
        let f = &mut self.fight;
        f.last_sight = now;
        f.sight_velocity = Vector::new(enemy.velocity.x, enemy.velocity.y, 0.0);
        f.sight_head = enemy.origin + enemy.view_ofs - Vector::new(0.0, 0.0, 2.0);
        f.hold_until = 0.0;
    }

    /// Whether the bot has lost sight of the enemy it faces (seen once, not in the last 0.15 s).
    pub(crate) fn enemy_out_of_sight(&self, now: f32) -> bool {
        self.fight.last_sight > 0.0 && self.fight.last_sight + 0.15 < now
    }

    /// Where to pre-aim for an enemy out of sight (design §4; upstream aimed at its live position
    /// through the wall): its head where it went, carried on by its last velocity for up to
    /// 0.5 s if that point is in sight, else where it was last seen.
    pub(crate) fn lost_aim_point(&self, w: &dyn super::BotWorld, now: f32) -> Vector {
        let f = &self.fight;
        let predicted = f.sight_head + f.sight_velocity * (now - f.last_sight).clamp(0.0, 0.5);
        if self.fvisible_point(w, predicted) { predicted } else { f.sight_head }
    }

    /// Hold the angle where the enemy went out of sight? Decided once per loss: for 1-3 s when
    /// healthy (over 60%) with a loaded gun; otherwise the bot pushes on as upstream does.
    pub(crate) fn hold_angle(&mut self, now: f32, rng: &mut dyn Random) -> bool {
        if self.fight.hold_until == 0.0 {
            let loaded = self.current_weapon.and_then(|id| self.weapons.get(id)).is_some_and(|wp| !wp.is_melee() && wp.clip > 0);
            self.fight.hold_until =
                if loaded && self.me.health > self.me.max_health * 0.6 { now + rng.random_float(1.0, 3.0) } else { -1.0 };
        }
        self.fight.hold_until > now
    }

    /// How much an enemy should be fought now (design §4, `FindEnemy`): its reputation's priority
    /// (as upstream's), more if it aims at the bot, hurt it in the last 3 s, is hurt, holds a gun,
    /// and the nearer it is.
    pub(crate) fn threat(&self, e: &Entity, rep: Option<i32>, now: f32) -> f32 {
        if !e.is_client() {
            // Upstream's breakable priority, 5.
            return 1.5;
        }
        let priority = rep.map_or(10, |r| super::BOT_MAX_REP - r) as f32;
        let mut score = priority * 0.3;
        let (forward, _) = make_vectors(e.v_angle);
        if dot(forward, (self.gun_position() - (e.origin + e.view_ofs)).normalize()) > 0.95 {
            score += 2.0;
        }
        if self.fight.attacker.is_some_and(|(a, t)| a == e.index && t + 3.0 > now) {
            score += 2.5;
        }
        if e.max_health > 0.0 {
            score += (1.0 - (e.health / e.max_health).clamp(0.0, 1.0)) * 1.5;
        }
        if e.player.is_some_and(|p| p.weapon != 0) {
            score += 1.0;
        }
        score + 2.0 * (1.0 - (e.origin - self.me.origin).length().min(2000.0) / 2000.0)
    }

    /// What weapon `id` is worth at `dist` (design §4): damage a second, times the range falloff
    /// and the share of shots that hit a player there with its base spread and the recoil it
    /// builds; blades within 80 units; nothing for an empty clip (unless the clip is infinite),
    /// the grenade or kung fu.
    pub(crate) fn weapon_score(&self, id: i32, dist: f32) -> f32 {
        let Some(wp) = self.weapons.get(id).filter(|w| w.has_weapon) else { return 0.0 };
        let Some(g) = ts_data::gun_data(id) else { return 0.0 };
        match g.kind {
            GunKind::Grenade => 0.0,
            GunKind::Knife | GunKind::Katana => {
                if dist < 80.0 {
                    g.damage / g.rate
                } else {
                    0.0
                }
            }
            _ => {
                if wp.clip <= 0 && self.state.active_powerup & powerup::INFAMMO == 0 {
                    return 0.0;
                }
                let interval = if g.has_auto { g.rate } else { g.press_interval(mode::SEMI) };
                let steady = if g.scale_spread {
                    let grow = g.acc_kick * 2.0 * (1.0 - interval.min(1.0)) - 0.1 * interval;
                    (grow.max(0.0) * 3.0).min(0.1)
                } else {
                    0.0
                };
                let hit = ((AIM_RADIUS / dist.max(1.0)) / (g.accuracy + steady).max(1e-4)).min(1.0);
                g.damage * g.pellets * g.range_multiplier(dist) * hit * hit / interval
            }
        }
    }

    /// [`weapon_score`](Self::weapon_score) corrected by what the weapon has done for this bot at
    /// that range (the learned table, design §4), once the bot has enough time with it there: the
    /// learned damage a second over the theory's, damped by a square root and kept within
    /// 0.5-1.4x, so experience tips close calls without overruling the gun's numbers.
    pub(crate) fn learned_score(&self, g: &BotGlobals<'_>, id: i32, dist: f32) -> f32 {
        let theory = self.weapon_score(id, dist);
        if theory <= 0.0 {
            return theory;
        }
        match self.learned_weapon_score(g, id, dist) {
            Some(learned) => theory * (learned / theory).max(0.0).sqrt().clamp(0.5, 1.4),
            None => theory,
        }
    }

    /// The best weapon the bot owns for `dist` other than `except`, with its score.
    pub(crate) fn best_scored_weapon(&self, g: &BotGlobals<'_>, dist: f32, except: Option<i32>) -> Option<(i32, f32)> {
        let mut best: Option<(i32, f32)> = None;
        for id in 1..crate::weapons::MAX_WEAPONS as i32 {
            if Some(id) == except {
                continue;
            }
            let score = self.learned_score(g, id, dist);
            if score > 0.0 && best.is_none_or(|(_, b)| score > b) {
                best = Some((id, score));
            }
        }
        best
    }

    /// Takes out the weapon for the range (design §4; upstream chose only when the clip ran
    /// dry): every 0.5 s, not while a reload runs or a switch is pending, when another weapon is
    /// worth 30% more (and a little) than the one in hand. A kung fu power-up running makes the
    /// bot fight hand to hand within 300 units.
    pub(crate) fn choose_weapon(&mut self, g: &BotGlobals<'_>, dist: f32, now: f32) {
        if self.fight.next_weapon_choice > now || self.switch_pending.is_some() {
            return;
        }
        self.fight.next_weapon_choice = now + 0.5;
        let Some(cur) = self.current_weapon else { return };
        if self.state.active_powerup & powerup::KUNGFU != 0 && dist < 300.0 {
            self.switch_weapon(TsWeapon::KungFu.id());
            return;
        }
        // A reload under way, or an empty clip (`Think`'s empty-gun choice decides that one).
        let reloading = self.next_reload_time > now && self.weapons.get(cur).is_some_and(|w| w.clip < w.max_clip);
        if reloading || self.weapons.get(cur).is_some_and(|w| w.clip_is_empty()) {
            return;
        }
        let here = self.learned_score(g, cur, dist);
        if let Some((best, score)) = self.best_scored_weapon(g, dist, Some(cur))
            && score > here * 1.3 + 1.0
        {
            self.switch_weapon(best);
        }
    }

    /// Uses the stored power-up when it helps the fight (design §4; upstream sent `usepowerup` on
    /// first seeing an enemy, gated by its chat timer): slow motion or slow pause in a mid-range
    /// fight (200-1200 units), an infinite clip or double fire rate with a gun out, kung fu
    /// within 300 units, health or armour under 60% health, the rest at once; at most every 2 s.
    pub(crate) fn use_powerup(&mut self, dist: f32, now: f32) {
        let stored = self.state.stored_powerup;
        if stored == 0 || self.state.active_powerup != 0 || self.fight.next_powerup > now {
            return;
        }
        let gun = self.gun_in_hand().is_some_and(|g| !g.is_melee_or_thrown());
        let hurt = self.me.health < self.me.max_health * 0.6;
        let useful = if stored & (powerup::SLOWMO | powerup::SLOWPAUSE) != 0 {
            (200.0..1200.0).contains(&dist)
        } else if stored & (powerup::INFAMMO | powerup::DFIRERATE) != 0 {
            gun
        } else if stored & powerup::KUNGFU != 0 {
            dist < 300.0
        } else if stored & (powerup::HEALTH | powerup::ARMOR) != 0 {
            hurt
        } else {
            true
        };
        if useful {
            self.actions.push(super::Action::Command("usepowerup".into()));
            self.fight.next_powerup = now + 2.0;
        }
    }

    /// A grenade at an enemy that went behind cover (design §4): an M61 in the inventory, the
    /// enemy out of sight for half a second to 3 s, last seen 300-750 units away, at most every
    /// 8 s, by a skill-scaled chance.
    pub(crate) fn grenade_behind_cover(&mut self, now: f32, rng: &mut dyn Random) {
        let f = &self.fight;
        let lost_for = now - f.last_sight;
        let dist = self.distance_from(self.last_see_enemy_position, false);
        if f.next_grenade > now
            || !(0.5..3.0).contains(&lost_for)
            || !(300.0..750.0).contains(&dist)
            || !self.weapons.get(TsWeapon::M61Grenade.id()).is_some_and(|w| w.has_weapon && w.clip > 0)
            || self.tasks.has_task(super::TaskKind::ThrowGrenade)
        {
            return;
        }
        self.fight.next_grenade = now + 8.0;
        if rng.random_float(0.0, 1.0) < 0.3 + 0.5 * self.skill_share() {
            let id = self.tasks.new_schedule_id();
            let t = super::Task::new(super::TaskKind::ThrowGrenade, id, None, 0, 0.0, self.last_see_enemy_position).with_timeout(now, 4.0);
            self.add_priority_task(t);
        }
    }
}

/// The radius the bot treats a player as for "on target" and spread (a player's hull is 16 wide).
pub(crate) const AIM_RADIUS: f32 = 14.0;

/// TS 3.0's M61 throw (`GrenadeIdle`, `shared/src/weapons/gun.rs`): the view pitch `p` (down
/// positive) becomes `p * (10/9 or 8/9) - 10`, thrown at `min(750, (90 - p') * 7)`. The view
/// pitch that lands a grenade from `src` nearest `target` under gravity `g` (the lowest arc
/// within a 3 s fuse), and its miss in units; `None` when nothing comes within 64 units.
pub(crate) fn throw_pitch(src: Vector, target: Vector, g: f32) -> Option<(f32, f32)> {
    let d = (target - src).length_2d();
    let h = target.z - src.z;
    let mut best: Option<(f32, f32)> = None;
    // From looking down (flat throws) up: the first that lands is the lowest arc.
    let mut p = 80.0f32;
    while p >= -80.0 {
        let thrown = p * if p >= 0.0 { 10.0 / 9.0 } else { 8.0 / 9.0 } - 10.0;
        let speed = ((90.0 - thrown) * 7.0).min(750.0);
        let (s, c) = (-thrown).to_radians().sin_cos();
        if c > 0.01 {
            let t = d / (speed * c);
            if t <= 3.0 {
                let miss = (speed * s * t - 0.5 * g * t * t - h).abs();
                if best.is_none_or(|(_, m)| miss < m - 1.0) {
                    best = Some((p, miss));
                }
            }
        }
        p -= 0.5;
    }
    best.filter(|&(_, m)| m < 64.0)
}
