//! Weapons (`bot_weapons.{h,cpp}`): the `bot_weapons.ini` presets, what the bot knows about each
//! weapon the game registers, and The Specialists 3.0's weapon table.

pub mod presets;
pub mod ts;

pub use presets::{PresetError, WeaponPreset, WeaponPresets};
pub use ts::TsWeapon;

/// `MAX_WEAPONS` (`cdll_dll.h`): 38 TS weapon ids plus kung fu remapped to 38.
pub const MAX_WEAPONS: usize = 39;

/// `MOD_TS` (`bot_const.h`): The Specialists' `[mod_id=…]` in `bot_weapons.ini`.
pub const MOD_TS: i32 = 8;

/// `BOT_VIEW_DISTANCE` (`bot_const.h`): a weapon's max range when it has no preset.
pub const BOT_VIEW_DISTANCE: f32 = 2048.0;

/// `CWeapon` and `CWeaponPreset`: a weapon the game registered (its `WeaponList` data, which the
/// port will supply), with the ini preset for its id if there is one. Without a preset upstream's
/// base class answers: usable underwater, both fire modes, any range, priority 0, melee when it
/// has no primary ammo.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponInfo {
    /// `m_iId`.
    pub id: i32,
    /// `m_szClassname`.
    pub classname: String,
    /// `m_iPrimAmmoMax`.
    pub prim_ammo_max: i32,
    /// `m_iSecAmmoMax`.
    pub sec_ammo_max: i32,
    /// `m_iHudSlot`.
    pub hud_slot: i32,
    /// `m_iHudPosition`.
    pub hud_position: i32,
    /// `m_iFlags`.
    pub flags: i32,
    /// `m_iAmmoIndex1` (-1 = none).
    pub ammo_index1: i32,
    /// `m_iAmmoIndex2` (-1 = none).
    pub ammo_index2: i32,
    /// The ini preset (`CWeaponPreset`), if any.
    pub preset: Option<WeaponPreset>,
}

impl WeaponInfo {
    /// `CanBeUsedUnderWater`.
    pub fn can_be_used_underwater(&self) -> bool {
        self.preset.as_ref().is_none_or(|p| p.can_fire_underwater)
    }

    /// `CanUsePrimary`.
    pub fn can_use_primary(&self) -> bool {
        self.preset.as_ref().is_none_or(|p| p.has_primary_fire)
    }

    /// `CanUseSecondary`.
    pub fn can_use_secondary(&self) -> bool {
        self.preset.as_ref().is_none_or(|p| p.has_secondary_fire)
    }

    /// `GetPriority`.
    pub fn priority(&self) -> i32 {
        self.preset.as_ref().map_or(0, |p| p.priority)
    }

    /// `PrimaryInRange`: `min <= range <= max`.
    pub fn primary_in_range(&self, range: f32) -> bool {
        self.preset.as_ref().is_none_or(|p| range >= p.prim_min_range && range <= p.prim_max_range)
    }

    /// `SecondaryInRange`.
    pub fn secondary_in_range(&self, range: f32) -> bool {
        self.preset.as_ref().is_none_or(|p| range >= p.sec_min_range && range <= p.sec_max_range)
    }

    /// The preset's primary max range ([`BOT_VIEW_DISTANCE`] without one). Upstream's
    /// `CWeaponPreset::PrimMaxRange` returns the *min* range and `PrimMinRange` the max (swapped);
    /// these return what their names say, and the combat port decides how to keep the old
    /// behaviour where it matters.
    pub fn prim_max_range(&self) -> f32 {
        self.preset.as_ref().map_or(BOT_VIEW_DISTANCE, |p| p.prim_max_range)
    }

    /// The preset's primary min range (0 without one); see [`WeaponInfo::prim_max_range`].
    pub fn prim_min_range(&self) -> f32 {
        self.preset.as_ref().map_or(0.0, |p| p.prim_min_range)
    }

    /// `IsMelee`: the preset's `is_melee`, or no primary ammo type without one.
    pub fn is_melee(&self) -> bool {
        self.preset.as_ref().map_or(self.ammo_index1 == -1, |p| p.is_melee)
    }
}

/// `CWeapons`: the weapons the game registered, by id.
#[derive(Clone, Debug)]
pub struct Weapons {
    weapons: Vec<Option<WeaponInfo>>,
}

impl Default for Weapons {
    fn default() -> Self {
        Weapons { weapons: vec![None; MAX_WEAPONS] }
    }
}

impl Weapons {
    /// `CWeapons::AddWeapon`: registers a weapon once (later registrations of the id are ignored),
    /// attaching the preset of `current_mod` for its id.
    #[allow(clippy::too_many_arguments)] // upstream's signature, field for field
    pub fn add(
        &mut self,
        presets: &WeaponPresets,
        current_mod: i32,
        id: i32,
        classname: &str,
        prim_ammo_max: i32,
        sec_ammo_max: i32,
        hud_slot: i32,
        hud_position: i32,
        flags: i32,
        ammo_index1: i32,
        ammo_index2: i32,
    ) {
        let Some(slot) = usize::try_from(id).ok().and_then(|i| self.weapons.get_mut(i)) else { return };
        if slot.is_some() {
            return;
        }
        *slot = Some(WeaponInfo {
            id,
            classname: classname.to_owned(),
            prim_ammo_max,
            sec_ammo_max,
            hud_slot,
            hud_position,
            flags,
            ammo_index1,
            ammo_index2,
            preset: presets.get(current_mod, id).cloned(),
        });
    }

    /// `CWeapons::GetWeapon`.
    pub fn get(&self, id: i32) -> Option<&WeaponInfo> {
        usize::try_from(id).ok().and_then(|i| self.weapons.get(i)).and_then(Option::as_ref)
    }

    /// `CWeapons::Init`: forgets every weapon (at map change).
    pub fn clear(&mut self) {
        self.weapons.fill(None);
    }
}
