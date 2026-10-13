//! The bot's own weapons (`bot_weapons.h`/`.cpp` `CBotWeapon`, `CBotWeapons`,
//! `CBotWeapons::GetBestWeaponId`, `CompareBotWeapon`) and the TS `WeaponInfo` handler that
//! fills them (`bot_client.cpp` `BotClient_TS_WeaponInfo`).
//!
//! TS never sends `WeaponList`, so RCBot registered each weapon the first time the bot held it,
//! with no ammo slots (`AddWeapon(id, "tsweapon", ammo, ammo, 0, 0, 0, 0, 0)` and
//! `SetWeapon(id, NULL)`): `m_iAmmo1` stays NULL for every TS weapon, which decides what
//! `CanReload`, `OutOfAmmo` and `LowOnAmmo` answer. The port's `PlayerInfo` weapon fields stand
//! in for the message (the host calls [`BotWeapons::weapon_info`] when they change).

use crate::weapons::{MAX_WEAPONS, TsWeapon, WeaponInfo, WeaponPresets, Weapons};

/// `CBotWeapon`.
#[derive(Clone, Debug, Default)]
pub struct BotWeapon {
    /// `m_iId` (0: not set).
    pub id: i32,
    /// `m_iClip`.
    pub clip: i32,
    /// `m_iReserve`.
    pub reserve: i32,
    /// `m_iMaxClip`: the largest clip seen since the last `SetWeapon`.
    pub max_clip: i32,
    /// `m_pWeaponInfo`.
    pub info: Option<WeaponInfo>,
    /// `m_bHasWeapon`.
    pub has_weapon: bool,
}

impl BotWeapon {
    /// `IsMelee`: no info counts as melee.
    pub fn is_melee(&self) -> bool {
        self.info.as_ref().is_none_or(WeaponInfo::is_melee)
    }

    /// `ClipIsEmpty`.
    pub fn clip_is_empty(&self) -> bool {
        if self.is_melee() || self.max_clip == 0 {
            return false;
        }
        self.clip <= 0
    }

    /// `OutOfAmmo` without `m_iAmmo1` (always so in TS).
    pub fn out_of_ammo(&self) -> bool {
        self.clip <= 0 && self.reserve <= 0
    }

    /// `LowOnAmmo`: false without `m_iAmmo1`.
    pub fn low_on_ammo(&self) -> bool {
        false
    }

    /// `CanReload`: false without `m_iAmmo1`, so always false in TS (design §5).
    pub fn can_reload(&self) -> bool {
        false
    }

    /// `NeedToReload`, TS: an empty clip with a reserve, for a gun whose clip was seen.
    pub fn need_to_reload(&self) -> bool {
        if self.is_melee() || self.max_clip == 0 {
            return false;
        }
        self.clip <= 0 && self.reserve > 0
    }

    /// `CanBeUsedUnderWater`.
    pub fn can_be_used_underwater(&self) -> bool {
        self.info.as_ref().is_none_or(WeaponInfo::can_be_used_underwater)
    }

    /// `GetPriority`.
    pub fn priority(&self) -> i32 {
        self.info.as_ref().map_or(0, WeaponInfo::priority)
    }

    /// `CanShootPrimary(pEdict, flFireDist, flWallDist)`.
    pub fn can_shoot_primary(&self, underwater: bool, fire_dist: f32) -> bool {
        let Some(info) = &self.info else { return true };
        !(self.out_of_ammo()
            || self.clip_is_empty()
            || (underwater && !self.can_be_used_underwater())
            || !info.can_use_primary()
            || !info.primary_in_range(fire_dist))
    }

    /// `SetWeapon(iId, NULL)`: `Reset`, then the registered info (and `m_bHasWeapon`) if any.
    fn set_weapon(&mut self, id: i32, registry: &Weapons) {
        *self = BotWeapon { id, ..BotWeapon::default() };
        self.info = registry.get(id).cloned();
        self.has_weapon = self.info.is_some();
    }
}

/// `CBotWeapons`.
#[derive(Clone, Debug)]
pub struct BotWeapons {
    weapons: Vec<BotWeapon>,
}

impl Default for BotWeapons {
    fn default() -> Self {
        BotWeapons { weapons: vec![BotWeapon::default(); MAX_WEAPONS] }
    }
}

/// What the TS `WeaponInfo` handler asks of the bot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeaponInfoResult {
    /// The weapon now in hand (`m_pCurrentWeapon`).
    pub current: i32,
    /// `!ammo && !clip`: upstream queues `BOT_TASK_DROP_WEAPON`.
    pub drop: bool,
}

impl BotWeapons {
    /// `GetWeapon`.
    pub fn get(&self, id: i32) -> Option<&BotWeapon> {
        usize::try_from(id).ok().and_then(|i| self.weapons.get(i))
    }

    /// `CBot::HasWeapon` (TS: the `CBotWeapon`'s own flag).
    pub fn has(&self, id: i32) -> bool {
        self.get(id).is_some_and(|w| w.has_weapon)
    }

    /// `AddWeapon(iId)`.
    pub fn add(&mut self, id: i32, registry: &Weapons) {
        if let Some(w) = usize::try_from(id).ok().and_then(|i| self.weapons.get_mut(i)) {
            w.set_weapon(id, registry);
        }
    }

    /// `BotClient_TS_WeaponInfo`: the game says the bot holds `game_id` (0 for kung fu) with
    /// `clip` rounds and `ammo` in reserve.
    ///
    /// `SetWeapon` resets `m_iMaxClip` before `checkMaxClip(clip)`, so the "max clip" is always
    /// the clip of the last message, and an empty gun has none: `NeedToReload` and `ClipIsEmpty`
    /// are then false. Kept (design §5).
    pub fn weapon_info(&mut self, game_id: i32, clip: i32, ammo: i32, registry: &mut Weapons, presets: &WeaponPresets) -> WeaponInfoResult {
        let id = TsWeapon::from_game_id(game_id).map_or(game_id, TsWeapon::id);
        registry.add(presets, crate::weapons::MOD_TS, id, "tsweapon", ammo, ammo, 0, 0, 0, 0, 0);
        self.add(id, registry);
        if let Some(w) = usize::try_from(id).ok().and_then(|i| self.weapons.get_mut(i)) {
            w.set_weapon(id, registry);
            w.clip = clip;
            w.has_weapon = true;
            w.reserve = ammo;
            w.max_clip = w.max_clip.max(clip);
        }
        WeaponInfoResult { current: id, drop: ammo == 0 && clip == 0 }
    }

    /// `CBotWeapons::GetBestWeaponId(pBot, pEnemy)`, TS: among the weapons the bot has (melee
    /// only within 40 units of an enemy, `[APG]RoboCop`'s TS rule), the highest priority one
    /// that can fire or just needs a reload; else the best of any; else kung fu. Ties go to the
    /// first by id (`std::max_element`). `enemy_dist` is `None` without an enemy;
    /// `enemy_too_high` is "more than a jump above".
    pub fn best_weapon_id(&self, enemy_dist: Option<f32>, enemy_too_high: bool, underwater: bool) -> (i32, bool) {
        let fire_dist = enemy_dist.unwrap_or(0.0);
        let mut usable: Vec<&BotWeapon> = Vec::new();
        let mut other: Vec<&BotWeapon> = Vec::new();
        for w in self.weapons.iter().skip(1) {
            if !w.has_weapon {
                continue;
            }
            if underwater && !w.can_be_used_underwater() {
                continue;
            }
            if w.is_melee() {
                if enemy_too_high {
                    continue;
                }
                other.push(w);
                if enemy_dist.is_some_and(|d| d > 40.0) {
                    continue;
                }
                usable.push(w);
                continue;
            }
            if !w.can_shoot_primary(underwater, fire_dist) {
                if w.need_to_reload() {
                    other.push(w);
                    usable.push(w);
                }
                continue;
            }
            other.push(w);
            if w.out_of_ammo() {
                continue;
            }
            usable.push(w);
        }
        let best = |v: &[&BotWeapon]| {
            let mut b: Option<&BotWeapon> = None;
            for w in v {
                if b.is_none_or(|x| x.priority() < w.priority()) {
                    b = Some(w);
                }
            }
            b.map(|w| w.id)
        };
        if let Some(id) = best(&usable) {
            return (id, false);
        }
        // `SetWeaponsNeeded` + `BOT_CONDITION_NEED_WEAPONS`.
        (best(&other).unwrap_or(TsWeapon::KungFu.id()), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presets() -> WeaponPresets {
        WeaponPresets::parse(include_str!("../../../../data/rcbot/bot_weapons.ini"), crate::weapons::MOD_TS).0
    }

    #[test]
    fn weapon_info_and_best_weapon() {
        let p = presets();
        let mut reg = Weapons::default();
        let mut w = BotWeapons::default();
        // Kung fu (game id 0) is 38 and melee.
        let r = w.weapon_info(0, 0, 0, &mut reg, &p);
        assert_eq!(r.current, 38);
        assert!(w.get(38).unwrap().is_melee());
        // A Glock with rounds: the best at range.
        let r = w.weapon_info(1, 17, 34, &mut reg, &p);
        assert_eq!(r, WeaponInfoResult { current: 1, drop: false });
        assert_eq!(w.best_weapon_id(Some(500.0), false, false), (1, false));
        // Too close for the Glock's preset (min 64): kung fu at 30 units.
        assert_eq!(w.best_weapon_id(Some(30.0), false, false).0, 38);
        // An empty Glock has max clip 0 (upstream's reset), so it doesn't "need a reload".
        w.weapon_info(1, 0, 34, &mut reg, &p);
        assert!(!w.get(1).unwrap().need_to_reload());
    }
}
