//! The Specialists 3.0 weapon ids (`enum class TSWeapon`, `bot_weapons.h`) and their world models
//! (`pszTSWeaponModels`, `bot_weapons.cpp`). The ids are TS 3.0's (36 is the Contender, 37 the
//! akimbo Skorpions); 38 is RCBot's own id for kung fu, which the game reports as weapon 0.

/// `TSWeapon`.
#[allow(missing_docs)] // the variant names are the weapons
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TsWeapon {
    Glock18 = 1,
    Beretta = 2,
    MiniUzi = 3,
    BenelliM3 = 4,
    M4A1 = 5,
    MP5SD = 6,
    MP5K = 7,
    AkimboBerettas = 8,
    SocomMk23 = 9,
    AkimboSocoms = 10,
    Usas12 = 11,
    DesertEagle = 12,
    Ak47 = 13,
    FiveSeven = 14,
    SteyrAug = 15,
    AkimboUzis = 16,
    Skorpion = 17,
    BarrettM82 = 18,
    HKPdw = 19,
    Spas12 = 20,
    Akimbocolts = 21,
    Glock20 = 22,
    Ump45 = 23,
    M61Grenade = 24,
    CombatKnife = 25,
    Mossberg500 = 26,
    M16A4 = 27,
    RugerMk1 = 28,
    C4 = 29,
    AkimboFiveSeven = 30,
    RagingBull = 31,
    M60 = 32,
    Sawedoff = 33,
    Katana = 34,
    SealKnife = 35,
    ContenderG2 = 36,
    AkimboSkorpions = 37,
    KungFu = 38,
}

/// `pszTSWeaponModels`, by id (0 and 29 are placeholders upstream: "0", "29 no gun here").
const MODELS: [&str; super::MAX_WEAPONS] = [
    "0",
    "models/w_glock18.mdl",
    "models/v_beretta.mdl",
    "models/w_uzi.mdl",
    "models/w_m3.mdl",
    "models/w_m4.mdl",
    "models/w_mp5sd.mdl",
    "models/w_mp5k.mdl",
    "models/w_berettas.mdl",
    "models/w_mk23.mdl",
    "models/v_mk23_akimbo.mdl",
    "models/w_usas.mdl",
    "models/w_desert.mdl",
    "models/w_ak47.mdl",
    "models/w_fnh.mdl",
    "models/w_aug.mdl",
    "models/w_uzi.mdl",
    "models/w_skorpion.mdl",
    "models/w_m82.mdl",
    "models/w_pdw.mdl",
    "models/w_spas12.mdl",
    "models/w_gold.mdl",
    "models/w_glock22.mdl",
    "models/w_ump.mdl",
    "models/w_m61.mdl",
    "models/w_knife.mdl",
    "models/w_mossberg.mdl",
    "models/w_m16.mdl",
    "models/w_ruger.mdl",
    "29 no gun here",
    "models/v_fnh_akimbo.mdl",
    "models/w_bull.mdl",
    "models/w_m60.mdl",
    "models/w_sawedoff.mdl",
    "models/w_katana.mdl",
    "models/w_sealknife.mdl",
    "models/w_contender.mdl",
    "models/v_skorpion_ak.mdl",
    "models/v_melee.mdl",
];

impl TsWeapon {
    /// Every weapon, by id.
    pub const ALL: [TsWeapon; 38] = {
        use TsWeapon::*;
        [
            Glock18,
            Beretta,
            MiniUzi,
            BenelliM3,
            M4A1,
            MP5SD,
            MP5K,
            AkimboBerettas,
            SocomMk23,
            AkimboSocoms,
            Usas12,
            DesertEagle,
            Ak47,
            FiveSeven,
            SteyrAug,
            AkimboUzis,
            Skorpion,
            BarrettM82,
            HKPdw,
            Spas12,
            Akimbocolts,
            Glock20,
            Ump45,
            M61Grenade,
            CombatKnife,
            Mossberg500,
            M16A4,
            RugerMk1,
            C4,
            AkimboFiveSeven,
            RagingBull,
            M60,
            Sawedoff,
            Katana,
            SealKnife,
            ContenderG2,
            AkimboSkorpions,
            KungFu,
        ]
    };

    /// The weapon with this id (1-38).
    pub fn from_id(id: i32) -> Option<TsWeapon> {
        usize::try_from(id).ok().and_then(|i| i.checked_sub(1)).and_then(|i| Self::ALL.get(i)).copied()
    }

    /// The id the game (and `bot_weapons.ini`) uses; 38 for kung fu.
    pub fn id(self) -> i32 {
        self as i32
    }

    /// The model upstream's weapon table lists for it (`pszTSWeaponModels`; C4 has none).
    pub fn model(self) -> Option<&'static str> {
        let m = MODELS[self as usize];
        m.starts_with("models/").then_some(m)
    }

    /// The weapon's name in `bot_weapons.ini`'s comments, without the `TS_` prefix.
    pub fn name(self) -> &'static str {
        use TsWeapon::*;
        match self {
            Glock18 => "Glock18",
            Beretta => "Beretta",
            MiniUzi => "MiniUzi",
            BenelliM3 => "BenelliM3",
            M4A1 => "M4A1",
            MP5SD => "MP5SD",
            MP5K => "MP5K",
            AkimboBerettas => "AkimboBerettas",
            SocomMk23 => "SocomMk23",
            AkimboSocoms => "AkimboSocomMk23",
            Usas12 => "Usas12",
            DesertEagle => "DesertEagle",
            Ak47 => "Ak47",
            FiveSeven => "FiveSeven",
            SteyrAug => "SteyrAug",
            AkimboUzis => "AkimboMiniUzis",
            Skorpion => "Skorpion",
            BarrettM82 => "BarrettM82",
            HKPdw => "HKPdw",
            Spas12 => "Spas12",
            Akimbocolts => "Akimbocolts",
            Glock20 => "Glock20",
            Ump45 => "Ump45",
            M61Grenade => "M61Grenade",
            CombatKnife => "CombatKnife",
            Mossberg500 => "Mossberg500",
            M16A4 => "M16A4",
            RugerMk1 => "RugerMk1",
            C4 => "C4",
            AkimboFiveSeven => "AkimboFiveSeven",
            RagingBull => "RagingBull",
            M60 => "M60",
            Sawedoff => "Sawedoff",
            Katana => "Katana",
            SealKnife => "SealKnife",
            ContenderG2 => "ContenderG2",
            AkimboSkorpions => "AkimboSkorpions",
            KungFu => "KungFu",
        }
    }

    /// RCBot's kung fu remap: the game reports kung fu as weapon 0 (`WeaponInfo`), RCBot as 38.
    pub fn from_game_id(id: i32) -> Option<TsWeapon> {
        if id == 0 { Some(TsWeapon::KungFu) } else { Self::from_id(id) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_models() {
        for (i, w) in TsWeapon::ALL.iter().enumerate() {
            assert_eq!(w.id(), i as i32 + 1);
            assert_eq!(TsWeapon::from_id(w.id()), Some(*w));
        }
        assert_eq!(TsWeapon::from_id(0), None);
        assert_eq!(TsWeapon::from_id(39), None);
        assert_eq!(TsWeapon::from_game_id(0), Some(TsWeapon::KungFu));
        assert_eq!(TsWeapon::ContenderG2.model(), Some("models/w_contender.mdl"));
        assert_eq!(TsWeapon::C4.model(), None);
        assert_eq!(TsWeapon::KungFu.model(), Some("models/v_melee.mdl"));
    }

    #[test]
    fn names_match_the_ini_comments() {
        let ini = include_str!("../../../../data/rcbot/bot_weapons.ini");
        let lines: Vec<&str> = ini.lines().collect();
        for w in TsWeapon::ALL {
            let tag = format!("[weapon_id={}]", w.id());
            let at = lines.iter().position(|l| *l == tag).unwrap();
            assert_eq!(lines[at - 1], format!("#TS_{}", w.name()), "{w:?}");
        }
    }
}
