//! TS 3.0's gun data the bot fights with (no upstream counterpart: RCBot knew only
//! `bot_weapons.ini`'s ranges and priorities). Copied from the port's
//! `crates/ts30/data/weapons.toml` and `ammo.toml` (decoded from `ts_i386.so`), with the rules
//! of `CTSGun::PrimaryAttack`, `StandardFire`, `ShotgunFire` and `ScaleSpread` that decide how a
//! bot must press the trigger and how fast its spread grows (`ts30/src/gun.rs`,
//! `ts30/src/recoil.rs`, `shared/src/weapons/gun.rs`).

/// What kind of weapon (`weapons.toml`'s `kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GunKind {
    /// A handgun (the sawed-off counts as one, as in TS).
    Pistol,
    /// A submachine gun.
    Smg,
    /// A rifle (the USAS-12 and the M60 too).
    Rifle,
    /// The Barrett.
    Sniper,
    /// A pump or semi-automatic shotgun.
    Shotgun,
    /// Two guns at once.
    Akimbo,
    /// The M61.
    Grenade,
    /// A knife (thrown with `+attack2`).
    Knife,
    /// The katana (thrown with `+attack2` in TS 3.0).
    Katana,
}

/// TS 3.0's fire modes (`shared::weapons::mode`): what `PlayerInfo::fire_mode` reports.
pub mod mode {
    /// Fires while held.
    pub const AUTO: i32 = 0;
    /// Fires on a fresh press (or held with the double fire rate power-up).
    pub const SEMI: i32 = 1;
    /// A fresh press starts a three-round burst.
    pub const BURST: i32 = 2;
    /// A shotgun's pump mode: fires while held.
    pub const PUMP: i32 = 3;
    /// Akimbo, each gun on its own button, fresh presses.
    pub const AKIMBO_SEMI: i32 = 4;
    /// Akimbo, each gun on its own button, held.
    pub const AKIMBO_AUTO: i32 = 5;
}

/// The port's power-up bits (`plugin_api::powerup`).
pub mod powerup {
    /// Slow motion.
    pub const SLOWMO: i32 = 1;
    /// Infinite clip.
    pub const INFAMMO: i32 = 2;
    /// Kung fu.
    pub const KUNGFU: i32 = 4;
    /// Slow pause.
    pub const SLOWPAUSE: i32 = 8;
    /// Double fire rate.
    pub const DFIRERATE: i32 = 16;
    /// Grenade.
    pub const GRENADE: i32 = 32;
    /// Health.
    pub const HEALTH: i32 = 64;
    /// Armour.
    pub const ARMOR: i32 = 128;
}

/// One gun's numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunData {
    /// TS id.
    pub id: i32,
    /// Its kind.
    pub kind: GunKind,
    /// Whether it has an automatic mode (then semi fires every 0.1 s, `semi_auto_delay`).
    pub has_auto: bool,
    /// Seconds between shots (`rate`).
    pub rate: f32,
    /// Seconds a reload takes (`reload`; a shotgun's is per shell).
    pub reload: f32,
    /// Damage per pellet.
    pub damage: f32,
    /// Pellets per shot (1 for bullets).
    pub pellets: f32,
    /// Base spread (`accuracy`).
    pub accuracy: f32,
    /// Whether shots add the recoil penalty (`scale_spread`).
    pub scale_spread: bool,
    /// The penalty per shot (`acc_kick`).
    pub acc_kick: f32,
    /// Full clip.
    pub clip: i32,
    /// The ammo's range unit: full damage to 10 units, `1/sqrt(units / 10)` beyond.
    pub range_unit: f32,
    /// Muzzle velocity in m/s (simulated bullets in slow motion travel at it times 40 u/m).
    pub velocity: f32,
}

/// (range unit, velocity) by ammo id (`ammo.toml`).
const AMMO: [(f32, f32); 14] = [
    (80.0, 330.0),
    (80.0, 330.0),
    (90.0, 280.0),
    (90.0, 350.0),
    (150.0, 900.0),
    (90.0, 480.0),
    (170.0, 790.0),
    (100.0, 730.0),
    (200.0, 1000.0),
    (90.0, 450.0),
    (70.0, 350.0),
    (20.0, 400.0),
    (40.0, 300.0),
    (170.0, 850.0),
];

/// `(id, kind, has_auto, rate, reload, damage, pellets, accuracy, scale_spread, acc_kick, ammo, clip)`.
type Row = (i32, GunKind, bool, f32, f32, f32, f32, f32, bool, f32, usize, i32);

#[rustfmt::skip]
const ROWS: [Row; 36] = {
    use GunKind::*;
    [
        (1, Pistol, true, 0.054, 1.5, 24.0, 1.0, 0.01, true, 0.0065, 1, 17),
        (2, Pistol, false, 0.1, 1.2, 27.0, 1.0, 0.01, true, 0.0, 1, 15),
        (3, Smg, true, 0.06, 2.2, 20.0, 1.0, 0.02, true, 0.0065, 1, 32),
        (4, Shotgun, false, 0.225, 0.3, 12.0, 8.0, 0.035, false, 0.0, 3, 7),
        (5, Rifle, true, 0.07, 2.8, 33.0, 1.0, 0.005, true, 0.004, 4, 30),
        (6, Smg, true, 0.074, 2.3, 19.0, 1.0, 0.005, true, 0.005, 1, 30),
        (7, Smg, true, 0.066, 2.3, 23.0, 1.0, 0.02, true, 0.003, 1, 30),
        (8, Akimbo, false, 0.1, 2.4, 27.0, 1.0, 0.02, true, 0.0, 1, 30),
        (9, Pistol, false, 0.1, 1.5, 33.0, 1.0, 0.01, true, 0.0, 2, 12),
        (10, Akimbo, false, 0.1, 2.5, 33.0, 1.0, 0.02, true, 0.0, 2, 24),
        (11, Rifle, true, 0.166, 3.0, 7.0, 8.0, 0.065, false, 0.0, 3, 20),
        (12, Pistol, false, 0.1, 2.1, 49.0, 1.0, 0.005, true, 0.0125, 5, 7),
        (13, Rifle, true, 0.08, 2.5, 39.0, 1.0, 0.0025, true, 0.0001, 6, 30),
        (14, Pistol, false, 0.1, 1.5, 23.0, 1.0, 0.0025, true, 0.01, 7, 20),
        (15, Rifle, true, 0.092, 2.4, 29.0, 1.0, 0.005, true, 0.01, 4, 30),
        (16, Akimbo, true, 0.06, 3.0, 20.0, 1.0, 0.04, true, 0.0065, 1, 64),
        (17, Smg, true, 0.07, 1.0, 16.0, 1.0, 0.025, true, 0.0015, 12, 20),
        (18, Sniper, false, 0.7, 2.0, 185.0, 1.0, 0.001, true, 0.1, 8, 5),
        (19, Smg, true, 0.063, 2.5, 16.0, 1.0, 0.005, true, 0.0065, 7, 40),
        (20, Shotgun, false, 0.2, 0.3, 12.0, 8.0, 0.05, false, 0.0, 3, 8),
        (21, Akimbo, false, 0.1, 2.3, 40.0, 1.0, 0.0075, true, 0.0, 2, 16),
        (22, Pistol, false, 0.1, 2.0, 39.0, 1.0, 0.01, true, 0.0125, 9, 15),
        (23, Smg, true, 0.092, 2.5, 31.0, 1.0, 0.015, true, 0.0, 2, 25),
        (24, Grenade, true, 1.0, 1.0, 110.0, 1.0, 0.1, false, 0.0, 0, 1),
        (25, Knife, true, 0.4, 1.0, 30.0, 1.0, 0.1, false, 0.0, 0, 1),
        (26, Shotgun, false, 0.25, 0.3, 16.0, 8.0, 0.045, false, 0.0, 3, 8),
        (27, Rifle, false, 0.07, 2.8, 40.0, 1.0, 0.0015, true, 0.005, 4, 18),
        (28, Pistol, false, 0.1, 1.3, 15.0, 1.0, 0.005, true, 0.008, 10, 17),
        (30, Akimbo, false, 0.1, 3.0, 23.0, 1.0, 0.005, true, 0.01, 7, 40),
        (31, Pistol, false, 0.15, 2.0, 95.0, 1.0, 0.0025, true, 0.0, 11, 5),
        (32, Rifle, true, 0.109, 3.0, 33.0, 1.0, 0.015, true, 0.005, 4, 100),
        (33, Pistol, false, 0.1, 1.7, 21.0, 8.0, 0.07, false, 0.0, 3, 2),
        (34, Katana, true, 0.7, 1.0, 45.0, 1.0, 0.7, false, 0.0, 0, 1),
        (35, Knife, true, 0.25, 1.0, 20.0, 1.0, 0.1, false, 0.0, 0, 1),
        (36, Pistol, false, 0.25, 1.35, 100.0, 1.0, 0.001, false, 0.0, 13, 1),
        (37, Akimbo, true, 0.07, 2.0, 16.0, 1.0, 0.05, true, 0.003, 12, 40),
    ]
};

/// The table, by id (`None` for kung fu, C4 and unknown ids).
pub fn gun_data(id: i32) -> Option<GunData> {
    let r = ROWS.iter().find(|r| r.0 == id)?;
    let (range_unit, velocity) = AMMO[r.10];
    Some(GunData {
        id: r.0,
        kind: r.1,
        has_auto: r.2,
        rate: r.3,
        reload: r.4,
        damage: r.5,
        pellets: r.6,
        accuracy: r.7,
        scale_spread: r.8,
        acc_kick: r.9,
        clip: r.11,
        range_unit,
        velocity,
    })
}

impl GunData {
    /// A blade or the grenade: no bullets.
    pub fn is_melee_or_thrown(&self) -> bool {
        matches!(self.kind, GunKind::Grenade | GunKind::Knife | GunKind::Katana)
    }

    /// Whether `+attack` fires only on a fresh press in `fire_mode` (`CTSGun::PrimaryAttack`:
    /// semi and akimbo semi unless the double fire rate power-up runs; burst starts on a press).
    pub fn needs_fresh_press(&self, fire_mode: i32, double_fire_rate: bool) -> bool {
        if self.is_melee_or_thrown() {
            return false;
        }
        match fire_mode {
            mode::SEMI | mode::AKIMBO_SEMI => !double_fire_rate,
            mode::BURST => true,
            _ => false,
        }
    }

    /// The fewest seconds between two presses that each fire in `fire_mode`: semi is 0.1 s for a
    /// gun with an automatic mode (`semi_auto_delay`), else its rate; a semi shotgun 0.15 s
    /// (`ShotgunFire`); a burst is three rounds 0.03 s apart and 0.2 s after.
    pub fn press_interval(&self, fire_mode: i32) -> f32 {
        match (fire_mode, self.kind) {
            (mode::BURST, _) => 0.26,
            (mode::SEMI, GunKind::Shotgun) => 0.15,
            (mode::SEMI, _) if self.has_auto => 0.1,
            _ => self.rate,
        }
    }

    /// The damage multiplier at `dist` units (`range_multiplier(dist / range_unit)`,
    /// `shared/src/weapons/mod.rs`).
    pub fn range_multiplier(&self, dist: f32) -> f32 {
        let idx = (dist / self.range_unit).clamp(1.0, 9999.0);
        (1.0 / (idx * 0.1).sqrt()).min(1.0)
    }

    /// The recoil penalty after a shot fired `elapsed` seconds after the last at `time_scale`
    /// (`ScaleSpread`): `pen + (1 - x) * acc_kick * k`, `x` the scaled interval (0.5 from 1 s
    /// up), `k` 2 (1 with the double fire rate), at most 0.1. `None` below 1 ms (untouched).
    pub fn grow_penalty(&self, pen: f32, elapsed: f32, time_scale: f32, double_fire_rate: bool, kick_scale: f32) -> Option<f32> {
        if !self.scale_spread {
            return Some(pen);
        }
        if elapsed < 0.001 {
            return None;
        }
        let p = time_scale * elapsed;
        let x = if p >= 1.0 { 0.5 } else { p };
        let k = if double_fire_rate { 1.0 } else { 2.0 };
        Some((pen + (1.0 - x) * self.acc_kick * kick_scale * k).min(0.1))
    }
}

/// `PreThink`'s penalty decay: 0.1 a second of the player's time.
pub fn decay_penalty(pen: f32, dt: f32, time_scale: f32) -> f32 {
    (pen - dt * time_scale.max(0.0) * 0.1).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_games() {
        let g = gun_data(18).unwrap();
        assert_eq!((g.kind, g.clip, g.range_unit), (GunKind::Sniper, 5, 200.0));
        // One Barrett shot fills the penalty's cap; it takes a second to decay.
        let pen = g.grow_penalty(0.0, 5.0, 1.0, false, 1.0).unwrap();
        assert_eq!(pen, 0.1);
        assert_eq!(decay_penalty(pen, 1.0, 1.0), 0.0);
        // Semi fire rates: 0.1 s for a gun with an automatic mode, else its own.
        assert_eq!(gun_data(1).unwrap().press_interval(mode::SEMI), 0.1);
        assert_eq!(g.press_interval(mode::SEMI), 0.7);
        assert_eq!(gun_data(36).unwrap().press_interval(mode::SEMI), 0.25);
        assert_eq!(gun_data(31).unwrap().press_interval(mode::SEMI), 0.15);
        assert!(gun_data(12).unwrap().needs_fresh_press(mode::SEMI, false));
        assert!(!gun_data(12).unwrap().needs_fresh_press(mode::SEMI, true));
        assert!(!gun_data(5).unwrap().needs_fresh_press(mode::AUTO, false));
        assert!(!gun_data(26).unwrap().needs_fresh_press(mode::PUMP, false));
        assert!(!gun_data(25).unwrap().needs_fresh_press(mode::SEMI, false));
        // Full damage to 10 range units (9 mm: 800), then 1/sqrt.
        let glock = gun_data(1).unwrap();
        assert_eq!(glock.range_multiplier(800.0), 1.0);
        assert!((glock.range_multiplier(3200.0) - 0.5).abs() < 1e-6);
        assert!(gun_data(29).is_none() && gun_data(38).is_none());
    }
}
