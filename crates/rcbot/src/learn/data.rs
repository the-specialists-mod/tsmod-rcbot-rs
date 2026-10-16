//! RCBot's learned-data files and the bookkeeping around them (`bot.cpp`, `globals.cpp`).
//!
//! * `<profile id>.rld` in the profiles folder ([`BotLearnedData`], `CBot::loadLearnedData` /
//!   `saveLearnedData`): a [`LearnedHeader`] and the bot's learning objects in a fixed order,
//!   RCZ-compressed when saved (uncompressed files still load). Objects other mods use (the NS
//!   lerk's flying GA, the combat bits) stay in the file so files keep their layout.
//! * `team<n>.rld` for teams 0-3 ([`GlobalLearnedData`], `CBotGlobals::saveLearnedData` /
//!   `loadLearnedData`): a [`LearnedHeader`] with the team number and the team's enemy-cost GA.
//! * `tsweaps.rld`: the TS loadout GA, without a header.
//!
//! Paths are the host's: these functions name files and turn bytes into data and back. NS's
//! `combat0.rld` / `combat1.rld` are dropped.

use super::ga::{Ga, Individual};
use super::header::LearnedHeader;
use super::heatmap::Heatmap;
use super::io::Reader;
use super::loadout::LoadoutBandit;
use super::opponents::{OpponentModel, PlayerKeys};
use super::perceptron::Perceptron;
use super::values::{BotGaValues, IntGaValues};
use super::weapon_stats::WeaponStats;
use crate::Random;
use crate::nav::rcz;

/// `MAX_TEAMS` (`bot_const.h`).
pub const MAX_TEAMS: usize = 4;
/// Genes in `m_GASurvival` (the load's `req_size`).
pub const SURVIVAL_GENES: usize = 16;
/// Genes in `m_pPersonalGAVals`.
pub const PERSONAL_GENES: usize = 22;
/// Genes in `m_pFlyGAVals`.
pub const FLY_GENES: usize = 6;
/// Genes in `m_pTSWeaponSelect`: three weapon choices, then five stunt weights.
pub const TS_WEAPON_SELECT_GENES: usize = 8;
/// `TSWeapon` ids span `0..35` when a gene in `0..=1` is scaled (`get(i) * 35`).
pub const TS_WEAPON_SCALE: f32 = 35.0;

/// `"<profile id>.rld"`, in the profiles folder.
pub fn bot_file_name(profile_id: i32) -> String {
    format!("{profile_id}.rld")
}

/// `"team<n>.rld"`, in the bot folder.
pub fn team_file_name(team: usize) -> String {
    format!("team{team}.rld")
}

/// The TS loadout GA's file, in the bot folder.
pub const TS_WEAPONS_FILE: &str = "tsweaps.rld";

/// One bot's learning (`CBot`'s `m_GASurvival`, `dec_*`, `m_pPersonalGAVals`, `m_personalGA`,
/// `m_pFlyGA(Vals)`, `m_pTSWeaponSelect`, `m_pCombatBits`), in the order of its file.
#[derive(Clone, Debug)]
pub struct BotLearnedData {
    /// `m_GASurvival`: enemy-cost and survival weights, bred per team (see
    /// [`GlobalLearnedData::enemy_costs`]): 0-3 friendly/enemy/inanimate/grenade cost, 4-7 run
    /// for cover, 8-11 face the hurt origin, 12-15 follow the enemy.
    pub survival: BotGaValues,
    /// `dec_attackElectrified` (7 inputs; weights from [`Self::personal`] 15-21).
    pub attack_electrified: Perceptron,
    /// `dec_flapWings` (4 inputs; NS lerk, kept for the file).
    pub flap_wings: Perceptron,
    /// `dec_runForCover` (3 inputs).
    pub run_for_cover: Perceptron,
    /// `dec_faceHurtOrigin` (3 inputs).
    pub face_hurt_origin: Perceptron,
    /// `dec_jump` (4 inputs, run with 5; weights from [`Self::personal`] 0-4).
    pub jump: Perceptron,
    /// `dec_duck` (weights from [`Self::personal`] 5-9).
    pub duck: Perceptron,
    /// `dec_strafe` (weights from [`Self::personal`] 10-14).
    pub strafe: Perceptron,
    /// `m_pPersonalGAVals`: jump, duck, strafe and electrified-enemy weights.
    pub personal: BotGaValues,
    /// `m_personalGA` (`CGA(5)`): breeds [`Self::personal`] from lives where the bot used melee.
    pub personal_ga: Ga<BotGaValues>,
    /// `m_pFlyGA` (`CGA(6)`; NS lerk, kept for the file).
    pub fly_ga: Ga<BotGaValues>,
    /// `m_pFlyGAVals` (NS lerk, kept for the file).
    pub fly: BotGaValues,
    /// `dec_followEnemy` (3 inputs; weights from [`Self::survival`] 12-15).
    pub follow_enemy: Perceptron,
    /// `m_pTSWeaponSelect`: 0-2 the loadout (scaled to weapon ids), 3-7 the stunt weights.
    pub ts_weapon_select: BotGaValues,
    /// `dec_stunt` (5 inputs; weights from [`Self::ts_weapon_select`] 3-7).
    pub stunt: Perceptron,
    /// `m_pCombatBits` (NS combat, kept for the file).
    pub combat_bits: IntGaValues,
}

impl BotLearnedData {
    /// `CBot::setupDataStructures`: the objects in upstream's construction order (which decides
    /// the random draws), with the fly and TS genes seeded. It doesn't load; see
    /// [`BotLearnedData::load`].
    pub fn new(rng: &mut dyn Random) -> BotLearnedData {
        let flap_wings = Perceptron::new(4, 0.0, rng);
        let run_for_cover = Perceptron::new(3, 0.0, rng);
        let face_hurt_origin = Perceptron::new(3, 0.0, rng);
        let follow_enemy = Perceptron::new(3, 0.0, rng);
        let attack_electrified = Perceptron::new(7, 0.0, rng);
        let jump = Perceptron::new(4, 0.0, rng);
        let duck = Perceptron::new(4, 0.0, rng);
        let stunt = Perceptron::new(5, 0.0, rng);
        let strafe = Perceptron::new(4, 0.0, rng);
        let mut data = BotLearnedData {
            survival: BotGaValues::new(),
            attack_electrified,
            flap_wings,
            run_for_cover,
            face_hurt_origin,
            jump,
            duck,
            strafe,
            personal: BotGaValues::new(),
            personal_ga: Ga::new(5),
            fly_ga: Ga::new(6),
            fly: BotGaValues::new(),
            follow_enemy,
            ts_weapon_select: BotGaValues::new(),
            stunt,
            combat_bits: IntGaValues::new(0),
        };
        data.seed_fly_values(rng);
        data.seed_ts_weapon_select(rng);
        data
    }

    /// `CBot::SeedFlyGAValues`: two `RANDOM_FLOAT(0, 0.2)`, four `0.3 - RANDOM_FLOAT(0, 6)`.
    pub fn seed_fly_values(&mut self, rng: &mut dyn Random) {
        self.fly.clear();
        for _ in 0..2 {
            self.fly.add(rng.random_float(0.0, 0.2));
        }
        for _ in 0..4 {
            self.fly.add(0.3 - rng.random_float(0.0, 6.0));
        }
    }

    /// `CBot::SeedTSWeaponSelectValues`: three `RANDOM_FLOAT(0, 1)` weapon genes, five
    /// `RANDOM_FLOAT(-1, 1)` stunt weights.
    pub fn seed_ts_weapon_select(&mut self, rng: &mut dyn Random) {
        self.ts_weapon_select.clear();
        for _ in 0..3 {
            self.ts_weapon_select.add(rng.random_float(0.0, 1.0));
        }
        for _ in 0..5 {
            self.ts_weapon_select.add(rng.random_float(-1.0, 1.0));
        }
    }

    /// The personal genes `CBot::SpawnInit` draws when its GA has none: 22 `RANDOM_FLOAT(-1, 1)`.
    pub fn seed_personal(&mut self, rng: &mut dyn Random) {
        self.personal.clear();
        for _ in 0..PERSONAL_GENES {
            self.personal.add(rng.random_float(-1.0, 1.0));
        }
    }

    /// The survival genes `CBot::SpawnInit` draws when the team's GA has none: 16
    /// `RANDOM_FLOAT(-1, 1)`.
    pub fn seed_survival(&mut self, rng: &mut dyn Random) {
        self.survival.clear();
        for _ in 0..SURVIVAL_GENES {
            self.survival.add(rng.random_float(-1.0, 1.0));
        }
    }

    /// `CBot::SpawnInit`'s learning part: the next bred personal genes (or fresh ones), the
    /// electrified-enemy perceptron's weights from them, then the team's next bred survival
    /// genes (or fresh ones). Picked individuals start with fitness 0.
    pub fn on_spawn(&mut self, team_costs: &mut Ga<BotGaValues>, rng: &mut dyn Random) {
        if let Some(mut ind) = self.personal_ga.pick() {
            ind.set_fitness(0.0);
            self.personal = ind;
        } else {
            self.seed_personal(rng);
        }
        self.attack_electrified.set_weights_from_values(&self.personal, 15, 7);
        if let Some(mut ind) = team_costs.pick() {
            ind.set_fitness(0.0);
            self.survival = ind;
        } else {
            self.seed_survival(rng);
        }
    }

    /// The fitness `CBot` gives a life when the bot dies: `survival time * 0.001 + frags since
    /// the last death * 0.2 + team score since then * 0.2`.
    pub fn death_fitness(survival_time: f32, frags_since_death: i32, team_score_since_death: i32) -> f32 {
        survival_time * 0.001 + frags_since_death as f32 * 0.2 + team_score_since_death as f32 * 0.2
    }

    /// `CBot`'s death bookkeeping (TS): the loadout, the survival genes (to the team's GA) and,
    /// if the bot used melee this life, the personal genes (to its own GA) are scored with
    /// `fitness` and bred.
    pub fn on_death(
        &mut self,
        fitness: f32,
        used_melee: bool,
        ts_weapon_choices: &mut Ga<BotGaValues>,
        team_costs: &mut Ga<BotGaValues>,
        rng: &mut dyn Random,
    ) {
        self.survival.set_fitness(fitness);
        self.ts_weapon_select.set_fitness(fitness);
        ts_weapon_choices.add_to_population(self.ts_weapon_select.clone(), rng);
        team_costs.add_to_population(self.survival.clone(), rng);
        if used_melee {
            self.personal.set_fitness(fitness);
            self.personal_ga.add_to_population(self.personal.clone(), rng);
        }
    }

    /// `CBot`'s TS loadout choice (when it hasn't `BOT_CONDITION_SELECTED_GUN`): the next bred
    /// loadout, its weapon genes clamped to `0..=1` (or redrawn when weapon learning is off), or
    /// fresh genes when the GA has none. Returns the three weapon ids upstream gives
    /// `UTIL_makeTSweapon` (`(int)(gene * 35)`); with `BOT_CONFIG_TS_KUNGFU` the caller buys none.
    pub fn select_ts_weapons(
        &mut self,
        ts_weapon_choices: &mut Ga<BotGaValues>,
        disable_weapon_learn: bool,
        rng: &mut dyn Random,
    ) -> [i32; 3] {
        if let Some(mut ind) = ts_weapon_choices.pick() {
            ind.set_fitness(0.0);
            self.ts_weapon_select = ind;
            if disable_weapon_learn {
                for i in 0..3 {
                    self.ts_weapon_select.set(i, rng.random_float(0.0, 1.0));
                }
            } else {
                for i in 0..3 {
                    let v = self.ts_weapon_select.get(i);
                    if v < 0.0 {
                        self.ts_weapon_select.set(i, 0.0);
                    } else if v > 1.0 {
                        self.ts_weapon_select.set(i, 1.0);
                    }
                }
            }
        } else {
            self.seed_ts_weapon_select(rng);
        }
        std::array::from_fn(|i| (self.ts_weapon_select.get(i) * TS_WEAPON_SCALE) as i32)
    }

    /// `CBot::loadLearnedData` from a file's bytes. RCZ files that don't decompress load
    /// nothing; a header for another version or profile loads nothing. Each object then reads
    /// in turn, exactly as upstream (a mismatched object consumes its header and leaves the rest
    /// of the file misaligned; §5).
    pub fn load(&mut self, bytes: &[u8], profile_id: i32) {
        let plain;
        let bytes = if rcz::is_compressed(bytes) {
            match rcz::decompress_any(bytes) {
                Some(d) => {
                    plain = d;
                    &plain[..]
                }
                None => return,
            }
        } else {
            bytes
        };
        let mut r = Reader::new(bytes);
        if !LearnedHeader::read_matches(&mut r, profile_id) || r.eof() {
            return;
        }
        let r = &mut r;
        self.survival.load(r, SURVIVAL_GENES as i32);
        self.attack_electrified.load(r);
        self.flap_wings.load(r);
        self.run_for_cover.load(r);
        self.face_hurt_origin.load(r);
        self.jump.load(r);
        self.duck.load(r);
        self.strafe.load(r);
        self.personal.load(r, PERSONAL_GENES as i32);
        self.personal_ga.load(r, PERSONAL_GENES as i32);
        self.fly_ga.load(r, FLY_GENES as i32);
        self.fly.load(r, FLY_GENES as i32);
        self.follow_enemy.load(r);
        self.ts_weapon_select.load(r, TS_WEAPON_SELECT_GENES as i32);
        self.stunt.load(r);
        // Upstream asks for 1 where the file holds 4, so this never loads (§5); kept.
        self.combat_bits.load(r, 1);
    }

    /// The uncompressed contents `CBot::saveLearnedData` builds.
    pub fn to_bytes(&self, profile_id: i32) -> Vec<u8> {
        let mut out = Vec::new();
        LearnedHeader::new(profile_id).write(&mut out);
        self.survival.save(&mut out);
        self.attack_electrified.save(&mut out);
        self.flap_wings.save(&mut out);
        self.run_for_cover.save(&mut out);
        self.face_hurt_origin.save(&mut out);
        self.jump.save(&mut out);
        self.duck.save(&mut out);
        self.strafe.save(&mut out);
        self.personal.save(&mut out);
        self.personal_ga.save(&mut out);
        self.fly_ga.save(&mut out);
        self.fly.save(&mut out);
        self.follow_enemy.save(&mut out);
        self.ts_weapon_select.save(&mut out);
        self.stunt.save(&mut out);
        self.combat_bits.save(&mut out);
        out
    }

    /// `CBot::saveLearnedData`: [`BotLearnedData::to_bytes`] in an RCZ container.
    pub fn save(&self, profile_id: i32) -> Vec<u8> {
        let raw = self.to_bytes(profile_id);
        rcz::compress(&raw).unwrap_or(raw)
    }
}

/// `CBotGlobals`' learning: each team's enemy-cost GA and the TS loadout GA.
#[derive(Clone, Debug)]
pub struct GlobalLearnedData {
    /// `m_enemyCostGAsForTeam`: breeds [`BotLearnedData::survival`] per team (`CGA()`, 16).
    pub enemy_costs: [Ga<BotGaValues>; MAX_TEAMS],
    /// `m_TSWeaponChoices`: breeds [`BotLearnedData::ts_weapon_select`] (`CGA()`, 16).
    pub ts_weapon_choices: Ga<BotGaValues>,
    /// Who each player slot is, for the port's learners (design §4).
    pub player_keys: PlayerKeys,
    /// Every bot's weapon-by-range results together: the prior each bot's own shrinks toward.
    pub weapon_prior: WeaponStats,
    /// Where players die, kill from and get hurt on this map.
    pub heatmap: Heatmap,
    /// Every bot's loadout estimates together: the prior each bot's own shrinks toward.
    pub loadout_prior: LoadoutBandit,
    /// What the bots know of the players they met.
    pub opponents: OpponentModel,
}

impl Default for GlobalLearnedData {
    fn default() -> Self {
        GlobalLearnedData {
            enemy_costs: std::array::from_fn(|_| Ga::new(0)),
            ts_weapon_choices: Ga::new(0),
            player_keys: PlayerKeys::default(),
            weapon_prior: WeaponStats::default(),
            heatmap: Heatmap::default(),
            loadout_prior: LoadoutBandit::default(),
            opponents: OpponentModel::default(),
        }
    }
}

impl GlobalLearnedData {
    /// `CBotGlobals::loadLearnedData` for `team<n>.rld`: the header must carry the team number.
    pub fn load_team(&mut self, team: usize, bytes: &[u8]) {
        let Some(ga) = self.enemy_costs.get_mut(team) else { return };
        let mut r = Reader::new(bytes);
        if LearnedHeader::read_matches(&mut r, team as i32) {
            ga.load(&mut r, SURVIVAL_GENES as i32);
        }
    }

    /// `CBotGlobals::saveLearnedData` for `team<n>.rld` (uncompressed).
    pub fn save_team(&self, team: usize) -> Vec<u8> {
        let mut out = Vec::new();
        LearnedHeader::new(team as i32).write(&mut out);
        if let Some(ga) = self.enemy_costs.get(team) {
            ga.save(&mut out);
        }
        out
    }

    /// `CBotGlobals::loadLearnedData` for `tsweaps.rld`.
    ///
    /// Upstream loads it with 16 genes per individual, but the loadouts have 8, so no saved
    /// loadout ever loads and the GA fills with empty individuals that bots then read past
    /// (§5). The port loads with 8 (design §4).
    pub fn load_ts_weapons(&mut self, bytes: &[u8]) {
        self.ts_weapon_choices.load(&mut Reader::new(bytes), TS_WEAPON_SELECT_GENES as i32);
    }

    /// `CBotGlobals::saveLearnedData` for `tsweaps.rld` (uncompressed, no header).
    pub fn save_ts_weapons(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.ts_weapon_choices.save(&mut out);
        out
    }

    /// `CBotGlobals::FreeGlobalMemory`'s reset of the GAs.
    pub fn free_local_memory(&mut self) {
        for ga in &mut self.enemy_costs {
            ga.free_local_memory();
        }
        self.ts_weapon_choices.free_local_memory();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    fn trained(rng: &mut XorShiftRandom) -> (BotLearnedData, GlobalLearnedData) {
        let mut g = GlobalLearnedData::default();
        let mut bot = BotLearnedData::new(rng);
        for life in 0..20 {
            bot.on_spawn(&mut g.enemy_costs[1], rng);
            bot.select_ts_weapons(&mut g.ts_weapon_choices, false, rng);
            bot.jump.set_weights_from_values(&bot.personal, 0, 5);
            bot.jump.input(&[0.5, 1.0, 0.0, 1.0, 1.0]);
            bot.jump.execute();
            bot.run_for_cover.input(&[0.1, 0.2, 0.3]);
            bot.run_for_cover.execute();
            bot.run_for_cover.train(1.0);
            let fit = BotLearnedData::death_fitness(life as f32 * 10.0, life % 3, 0);
            bot.on_death(fit, life % 2 == 0, &mut g.ts_weapon_choices, &mut g.enemy_costs[1], rng);
        }
        (bot, g)
    }

    #[test]
    fn construction_seeds() {
        let bot = BotLearnedData::new(&mut XorShiftRandom::new(1));
        assert_eq!(bot.fly.len(), FLY_GENES);
        assert!(bot.fly.values()[..2].iter().all(|v| (0.0..0.2).contains(v)));
        assert!(bot.fly.values()[2..].iter().all(|v| (-5.7..=0.3).contains(v)));
        assert_eq!(bot.ts_weapon_select.len(), TS_WEAPON_SELECT_GENES);
        assert!(bot.survival.is_empty() && bot.personal.is_empty());
        assert_eq!(bot.attack_electrified.num_weights(), 8);
    }

    #[test]
    fn spawn_and_death_breed() {
        let mut rng = XorShiftRandom::new(2);
        let (bot, g) = trained(&mut rng);
        assert_eq!(bot.survival.len(), SURVIVAL_GENES);
        assert_eq!(bot.personal.len(), PERSONAL_GENES);
        assert_eq!(bot.attack_electrified.weights()[0], 1.0, "the bias weight is the bias");
        assert_eq!(bot.attack_electrified.weights()[1..], bot.personal.values()[15..22]);
        assert!(g.ts_weapon_choices.generations() >= 1);
        assert!(g.enemy_costs[1].generations() >= 1);
        assert!(bot.personal_ga.generations() >= 1);
        assert_eq!(g.enemy_costs[0].generations(), 0);
    }

    #[test]
    fn ts_weapon_choice() {
        let mut rng = XorShiftRandom::new(3);
        let mut bot = BotLearnedData::new(&mut rng);
        let mut ga = Ga::new(2);
        let mut a = BotGaValues::from_values(vec![-0.5, 0.5, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        a.set_fitness(1.0);
        ga.add_to_population(a.clone(), &mut rng);
        ga.add_to_population(a, &mut rng);
        let ids = bot.select_ts_weapons(&mut ga, false, &mut rng);
        // The elite copies come back unmutated: clamped to 0, 0.5, 1.
        assert_eq!(ids, [0, 17, 35]);
        assert_eq!(bot.ts_weapon_select.fitness(), 0.0);
        let ids = bot.select_ts_weapons(&mut ga, true, &mut rng);
        assert!(ids.iter().all(|i| (0..35).contains(i)));
        // Nothing left to pick: fresh genes.
        bot.select_ts_weapons(&mut ga, false, &mut rng);
        assert!(bot.ts_weapon_select.values()[3..].iter().all(|v| (-1.0..1.0).contains(v)));
    }

    #[test]
    fn fitness_formula() {
        assert_eq!(BotLearnedData::death_fitness(100.0, 2, 1), 100.0 * 0.001 + 2.0 * 0.2 + 1.0 * 0.2);
    }

    #[test]
    fn bot_file_round_trip() {
        let mut rng = XorShiftRandom::new(4);
        let (bot, _) = trained(&mut rng);
        let raw = bot.to_bytes(12);
        let saved = bot.save(12);
        assert!(rcz::is_compressed(&saved));
        for bytes in [&saved, &raw] {
            let mut back = BotLearnedData::new(&mut XorShiftRandom::new(99));
            back.load(bytes, 12);
            // Everything but the combat bits (never loads) comes back.
            assert_eq!(back.to_bytes(12)[..raw.len() - 12], raw[..raw.len() - 12]);
            assert_eq!(back.combat_bits, IntGaValues::new(0));
        }
        // Another profile's file loads nothing.
        let fresh = BotLearnedData::new(&mut XorShiftRandom::new(99));
        let mut other = fresh.clone();
        other.load(&saved, 13);
        assert_eq!(other.to_bytes(13), fresh.to_bytes(13));
    }

    #[test]
    fn empty_survival_desyncs_the_rest_as_upstream() {
        // A bot saved before its first spawn has no survival genes: its header says 0, the
        // loader asks for 16, and every later object reads from the wrong place.
        let saved = BotLearnedData::new(&mut XorShiftRandom::new(5));
        let raw = saved.to_bytes(1);
        let fresh = BotLearnedData::new(&mut XorShiftRandom::new(6));
        let mut back = fresh.clone();
        back.load(&raw, 1);
        assert_eq!(back.attack_electrified, fresh.attack_electrified);
        assert_eq!(back.stunt, fresh.stunt);
    }

    #[test]
    fn global_files() {
        let mut rng = XorShiftRandom::new(7);
        let (_, g) = trained(&mut rng);
        let team = g.save_team(1);
        assert_eq!(&team[..7], b"1.51b14");
        let mut back = GlobalLearnedData::default();
        back.load_team(1, &team);
        assert_eq!(back.save_team(1), team);
        let mut wrong = GlobalLearnedData::default();
        wrong.load_team(2, &team);
        assert!(wrong.enemy_costs[2].population().is_empty());

        let ts = g.save_ts_weapons();
        back.load_ts_weapons(&ts);
        assert_eq!(back.save_ts_weapons(), ts);
        assert!(back.ts_weapon_choices.new_population().individuals().iter().all(|i| i.len() == TS_WEAPON_SELECT_GENES));
        back.free_local_memory();
        assert!(!back.ts_weapon_choices.can_pick());

        assert_eq!(bot_file_name(3), "3.rld");
        assert_eq!(team_file_name(0), "team0.rld");
    }
}
