//! `bot_weapons.ini` (`CWeaponPresets::ReadPresets`, `weapon_preset_t`).
//!
//! The format, as upstream reads it line by line: lines that are empty or start with `#` are
//! skipped (before trimming, so an indented `#` is not a comment), then trailing whitespace is
//! trimmed. `[mod_id=N]` starts a mod's section; sections of other mods are skipped.
//! `[weapon_id=N]` starts a weapon with every field zero, `[/weapon]` stores it, and `key=value`
//! sets a field (values are read with `std::stoi`, so `12abc` is 12). Unknown keys are ignored;
//! a bad number is reported and the line skipped. Lines before the first `[mod_id=…]` belong to
//! mod 0.

use crate::stoi::{StoiError, stoi};

/// `weapon_preset_t`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeaponPreset {
    /// `m_iId`: the weapon id.
    pub id: i32,
    /// `m_iModId`.
    pub mod_id: i32,
    /// `underwater`.
    pub can_fire_underwater: bool,
    /// `primaryfire`.
    pub has_primary_fire: bool,
    /// `secondaryfire`.
    pub has_secondary_fire: bool,
    /// `primary_min_range`.
    pub prim_min_range: f32,
    /// `primary_max_range`.
    pub prim_max_range: f32,
    /// `secondary_min_range`.
    pub sec_min_range: f32,
    /// `secondary_max_range`.
    pub sec_max_range: f32,
    /// `is_melee`.
    pub is_melee: bool,
    /// `priority`: higher is better.
    pub priority: i32,
    /// `maxclip`.
    pub max_clip: i32,
}

/// A line upstream's parser reported (`BotMessage`) and skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetError {
    /// 1-based line number.
    pub line: usize,
    /// The trimmed line.
    pub text: String,
    /// `std::invalid_argument` or `std::out_of_range`.
    pub kind: StoiError,
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.kind {
            StoiError::InvalidArgument => "Invalid argument",
            StoiError::OutOfRange => "Out of range value",
        };
        write!(f, "{what} in weapon preset file. Line {}: '{}'", self.line, self.text)
    }
}

/// `CWeaponPresets`: the presets of one mod, in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeaponPresets {
    presets: Vec<WeaponPreset>,
}

/// `substr(start, len - start - 1)` of a `[key=N]` line, as upstream cuts it.
fn bracket_value(line: &str, start: usize) -> &str {
    line.get(start..line.len().saturating_sub(1).max(start)).unwrap_or("")
}

impl WeaponPresets {
    /// `ReadPresets` over the file's text, keeping the sections of `current_mod` (TS is
    /// [`super::MOD_TS`]). Returns the presets and the lines upstream would have reported.
    pub fn parse(text: &str, current_mod: i32) -> (WeaponPresets, Vec<PresetError>) {
        let mut presets = Vec::new();
        let mut errors = Vec::new();
        let mut current = WeaponPreset::default();
        let mut mod_id = 0;
        let mut skip_mod = false;
        // std::getline splits on '\n' only; a '\r' is trimmed with the trailing whitespace.
        for (n, raw) in text.split('\n').enumerate() {
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            let line = raw.trim_end_matches([' ', '\n', '\r', '\t']);
            if line.is_empty() {
                continue;
            }
            let mut report = |kind| errors.push(PresetError { line: n + 1, text: line.to_owned(), kind });
            if line.starts_with("[mod_id=") {
                match stoi(bracket_value(line, 8)) {
                    Ok(v) => {
                        mod_id = v;
                        skip_mod = mod_id != current_mod;
                    }
                    Err(e) => report(e),
                }
            } else if skip_mod {
                continue;
            } else if line.starts_with("[weapon_id=") {
                current = WeaponPreset { mod_id, ..WeaponPreset::default() };
                match stoi(bracket_value(line, 11)) {
                    Ok(v) => current.id = v,
                    Err(e) => report(e),
                }
            } else if line == "[/weapon]" {
                presets.push(current.clone());
            } else if let Some((key, value)) = line.split_once('=') {
                let v = match stoi(value) {
                    Ok(v) => v,
                    Err(e) => {
                        report(e);
                        continue;
                    }
                };
                let p = &mut current;
                match key {
                    "underwater" => p.can_fire_underwater = v != 0,
                    "primaryfire" => p.has_primary_fire = v != 0,
                    "secondaryfire" => p.has_secondary_fire = v != 0,
                    "primary_min_range" => p.prim_min_range = v as f32,
                    "primary_max_range" => p.prim_max_range = v as f32,
                    "secondary_min_range" => p.sec_min_range = v as f32,
                    "secondary_max_range" => p.sec_max_range = v as f32,
                    "is_melee" => p.is_melee = v != 0,
                    "priority" => p.priority = v,
                    "maxclip" => p.max_clip = v,
                    _ => {}
                }
            }
        }
        (WeaponPresets { presets }, errors)
    }

    /// `GetPreset`: upstream keeps a stack, so the **last** preset for an id wins.
    pub fn get(&self, mod_id: i32, weapon_id: i32) -> Option<&WeaponPreset> {
        self.presets.iter().rev().find(|p| p.id == weapon_id && p.mod_id == mod_id)
    }

    /// Every preset, in file order.
    pub fn all(&self) -> &[WeaponPreset] {
        &self.presets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weapons::{MOD_TS, TsWeapon, WeaponInfo, Weapons};

    const SHIPPED: &str = include_str!("../../../../data/rcbot/bot_weapons.ini");

    #[test]
    fn shipped_ts_section() {
        let (p, errors) = WeaponPresets::parse(SHIPPED, MOD_TS);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(p.all().len(), 38);
        // Every TS id 1-38 has exactly one preset.
        for w in TsWeapon::ALL {
            assert_eq!(p.all().iter().filter(|x| x.id == w as i32).count(), 1, "{w:?}");
        }
        let glock = p.get(MOD_TS, TsWeapon::Glock18 as i32).unwrap();
        assert_eq!(
            *glock,
            WeaponPreset {
                id: 1,
                mod_id: 8,
                has_primary_fire: true,
                prim_min_range: 64.0,
                prim_max_range: 4096.0,
                priority: 3,
                max_clip: 17,
                ..WeaponPreset::default()
            }
        );
        let katana = p.get(MOD_TS, TsWeapon::Katana as i32).unwrap();
        assert!(katana.is_melee && katana.has_secondary_fire && katana.sec_max_range == 48.0);
        let knife = p.get(MOD_TS, TsWeapon::CombatKnife as i32).unwrap();
        assert!(knife.can_fire_underwater && knife.priority == 1);
        assert_eq!(p.get(MOD_TS, TsWeapon::KungFu as i32).unwrap().priority, 2);
        assert_eq!(p.get(MOD_TS, TsWeapon::BarrettM82 as i32).unwrap().prim_min_range, 1024.0);
        assert!(p.get(2, 1).is_none());
        // Nothing for another mod.
        assert!(WeaponPresets::parse(SHIPPED, 2).0.all().is_empty());
    }

    #[test]
    fn upstream_parsing_rules() {
        let text = "\
[weapon_id=1]\npriority=9\n[/weapon]\n\
[mod_id=2]\n[weapon_id=1]\npriority=1\n[/weapon]\n\
[mod_id=8]\r\n\
\x20 # indented: not a comment, but no equals sign either\n\
[weapon_id=5]   \n\
priority=4x\n\
maxclip=abc\n\
primary_max_range=99999999999\n\
unknown=3\n\
[/weapon]\n\
[weapon_id=5]\npriority=6\n[/weapon]\n\
[/weapon]\n\
[mod_id=x]\n\
[weapon_id=7]\npriority=1\n[/weapon]\n";
        let (p, errors) = WeaponPresets::parse(text, 8);
        // mod 0's weapon (before any [mod_id]) is kept with mod 0; mod 2's is skipped.
        assert_eq!(p.get(0, 1).unwrap().priority, 9);
        assert!(p.get(2, 1).is_none());
        // The later preset for id 5 wins; the first was still stored.
        assert_eq!(p.get(8, 5).unwrap().priority, 6);
        assert_eq!(p.all()[1].priority, 4);
        assert_eq!(p.all()[1].max_clip, 0);
        // A stray [/weapon] stores the current preset again.
        assert_eq!(p.all().len(), 4 + 1);
        // [mod_id=x] fails, so we stay in mod 8.
        assert_eq!(p.get(8, 7).unwrap().priority, 1);
        let kinds: Vec<_> = errors.iter().map(|e| (e.line, e.kind)).collect();
        assert_eq!(kinds, vec![(12, StoiError::InvalidArgument), (13, StoiError::OutOfRange), (20, StoiError::InvalidArgument)]);
    }

    #[test]
    fn weapon_info_defaults_and_presets() {
        let (p, _) = WeaponPresets::parse(SHIPPED, MOD_TS);
        let mut w = Weapons::default();
        w.add(&p, MOD_TS, 25, "weapon_combatknife", -1, -1, 0, 0, 0, -1, -1);
        w.add(&p, MOD_TS, 40, "out_of_range", -1, -1, 0, 0, 0, -1, -1);
        w.add(&p, MOD_TS, 25, "ignored_second_time", -1, -1, 0, 0, 0, -1, -1);
        let knife = w.get(25).unwrap();
        assert_eq!(knife.classname, "weapon_combatknife");
        assert!(knife.is_melee() && knife.can_be_used_underwater() && knife.priority() == 1);
        assert!(knife.primary_in_range(60.0) && !knife.primary_in_range(61.0));
        let bare = WeaponInfo {
            id: 3,
            classname: String::new(),
            prim_ammo_max: 0,
            sec_ammo_max: 0,
            hud_slot: 0,
            hud_position: 0,
            flags: 0,
            ammo_index1: 2,
            ammo_index2: -1,
            preset: None,
        };
        assert!(!bare.is_melee() && bare.primary_in_range(1e9) && bare.priority() == 0 && bare.prim_max_range() == 2048.0);
        w.clear();
        assert!(w.get(25).is_none());
    }
}
