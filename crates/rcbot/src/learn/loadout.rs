//! The loadout choice (a port addition, design §4): a bandit over curated TS loadouts, replacing
//! upstream's loadout GA as the chooser (its genes still load, breed and save byte for byte in
//! `tsweaps.rld`, and still feed the stunt perceptron).
//!
//! Upstream mapped three genes `× 35` to weapon ids: a 0 was an empty slot forever, 35 the seal
//! knife, every bot shared one GA, the buy's result was never checked, and a life's fitness
//! (0, 0.2 or 0.4 in deathmatch) graded it. Here each arm is a primary and a sidearm a TS player
//! would carry, light enough to keep diving (at most [`MAX_SLOTS`] of TS's slots). An arm is
//! credited only when the inventory the game reports after the spawn holds it, with the life's
//! `(kills - 0.5 deaths + dealt / 200)` per minute. Arms are chosen by discounted UCB1 (every
//! arm's weight × [`GAMMA`] per life), each bot's estimate shrunk toward every bot's.

use super::bandit::{Arm, ucb};
use super::store;
use crate::Random;

/// Slots (`TSSpace`) a loadout may use: TS 3.0 refuses dives above 62 used.
pub const MAX_SLOTS: i32 = 60;
/// What every arm's weight keeps per life (old lives fade).
pub const GAMMA: f32 = 0.97;
/// Samples of the global estimate a bot's own is shrunk toward.
pub const PRIOR_WEIGHT: f32 = 2.0;
/// UCB1's exploration weight.
const UCB_C: f32 = 1.0;
/// Seconds after a spawn in which the inventory is checked for the loadout.
pub const CREDIT_WINDOW: f32 = 3.0;
/// `"<key>.rlo"`, in the profiles folder: a bot's estimates.
pub const FILE_EXT: &str = "rlo";
/// Every bot's estimates, in the bot folder.
pub const GLOBAL_FILE: &str = "loadouts.rlo";

const FILE_KIND: &str = "loadout";
const FILE_VERSION: u32 = 1;

/// The curated loadouts: TS weapon ids (`weapons.toml`), primary then sidearm.
pub const LOADOUTS: &[&[i32]] = &[
    &[5, 12],  // M4A1, Desert Eagle (55 slots)
    &[13, 12], // AK47, Desert Eagle (55)
    &[27, 12], // M16A4, Desert Eagle (55)
    &[15, 12], // STEYR-AUG, Desert Eagle (45)
    &[15, 31], // STEYR-AUG, Raging Bull (45)
    &[23, 12], // UMP, Desert Eagle (45)
    &[6, 12],  // MP5SD, Desert Eagle (40)
    &[19, 12], // MP7-PDW, Desert Eagle (40)
    &[7, 31],  // MP5K, Raging Bull (35)
    &[20, 12], // SPAS-12, Desert Eagle (45)
    &[26, 1],  // Mossberg 500, Glock-18 (50)
    &[4, 22],  // Benelli M3, Glock-20C (50)
    &[11, 9],  // USAS-12, SOCOM-MK23 (50)
    &[5, 31],  // M4A1, Raging Bull (55)
    &[13, 22], // AK47, Glock-20C (50)
    &[16, 12], // Akimbo Mini-Uzis, Desert Eagle (35)
    &[37, 12], // Akimbo Skorpions, Desert Eagle (30)
    &[3, 31],  // Mini-Uzi, Raging Bull (35)
    &[21, 23], // Golden Colts, UMP (50)
    &[13, 34], // AK47, Katana (50)
    &[36, 23], // Contender G2, UMP (45)
    &[30, 6],  // Akimbo Five-seveN, MP5SD (35)
    &[31, 34], // Raging Bull, Katana (25)
    &[17, 12], // Skorpion, Desert Eagle (30)
];

/// TS 3.0's slot cost (`weapons.toml` `slots`) of a weapon id.
pub fn weapon_slots(id: i32) -> i32 {
    match id {
        1 | 2 | 8 | 9 | 10 | 14 | 22 | 30 | 34 => 10,
        3 | 7 | 16 | 21 | 33 => 20,
        4 | 5 | 11 | 13 | 26 | 27 => 40,
        6 | 19 => 25,
        12 | 17 | 31 | 36 | 37 => 15,
        15 | 20 | 23 => 30,
        18 | 32 => 70,
        24 => 7,
        25 | 35 => 1,
        28 => 5,
        _ => 0,
    }
}

/// The slots a loadout uses.
pub fn loadout_slots(loadout: &[i32]) -> i32 {
    loadout.iter().map(|&w| weapon_slots(w)).sum()
}

/// A life's reward: `(kills - 0.5 deaths + dealt / 200)` per minute (at least half a minute, so
/// a lucky short life doesn't dominate), clamped to -6..12.
pub fn life_reward(kills: u32, deaths: u32, dealt: f32, seconds: f32) -> f32 {
    let minutes = (seconds / 60.0).max(0.5);
    ((kills as f32 - 0.5 * deaths as f32 + dealt / 200.0) / minutes).clamp(-6.0, 12.0)
}

/// The arm the inventory after a spawn shows: the one bought if all its weapons are there, else
/// a loadout that is exactly what is owned (grenades, knives and kung fu aside), else none.
pub fn credited_loadout(bought: Option<usize>, owned: &[i32]) -> Option<usize> {
    let has_all = |l: &[i32]| l.iter().all(|w| owned.contains(w));
    if let Some(b) = bought.filter(|&b| LOADOUTS.get(b).is_some_and(|l| has_all(l))) {
        return Some(b);
    }
    let mut guns: Vec<i32> = owned.iter().copied().filter(|&w| !matches!(w, 0 | 24 | 25 | 35 | 38)).collect();
    guns.sort_unstable();
    guns.dedup();
    LOADOUTS.iter().position(|l| {
        let mut l = l.to_vec();
        l.sort_unstable();
        l == guns
    })
}

/// The estimates, one arm per [`LOADOUTS`] entry.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadoutBandit {
    /// Each loadout's discounted mean reward and weight.
    pub arms: Vec<Arm>,
}

impl Default for LoadoutBandit {
    fn default() -> Self {
        LoadoutBandit { arms: vec![Arm::default(); LOADOUTS.len()] }
    }
}

impl LoadoutBandit {
    /// The loadout for the next life, among those within `capacity` slots: by UCB1 on the
    /// bot's estimate shrunk toward `prior`'s (untried arms first, ties broken by `rng` so bots
    /// don't all try the same order), or at random when `random` (weapon learning off).
    pub fn choose(&self, prior: &LoadoutBandit, capacity: i32, random: bool, rng: &mut dyn Random) -> usize {
        let fits: Vec<usize> = (0..LOADOUTS.len()).filter(|&i| loadout_slots(LOADOUTS[i]) <= capacity.min(MAX_SLOTS)).collect();
        if fits.is_empty() {
            return 0;
        }
        if random {
            return fits[rng.random_long(0, fits.len() as i32 - 1) as usize];
        }
        let total: f32 = fits.iter().map(|&i| self.arms[i].n).sum();
        let scores: Vec<f32> = fits
            .iter()
            .map(|&i| {
                let p = prior.arms[i];
                let (prior_mean, prior_n) = if p.n > 0.0 { (p.value, PRIOR_WEIGHT.min(p.n)) } else { (0.0, 0.0) };
                let mine = self.arms[i];
                ucb(mine.shrunk(prior_mean, prior_n), mine.n + prior_n, total, UCB_C)
            })
            .collect();
        let best = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let tied: Vec<usize> = fits.iter().zip(&scores).filter(|(_, s)| **s == best).map(|(&i, _)| i).collect();
        tied[rng.random_long(0, tied.len() as i32 - 1) as usize]
    }

    /// A life with loadout `arm` earned `reward`: every arm fades by [`GAMMA`], then `arm` learns.
    pub fn reward(&mut self, arm: usize, reward: f32) {
        if arm >= self.arms.len() {
            return;
        }
        for a in &mut self.arms {
            a.decay(GAMMA);
        }
        self.arms[arm].update(reward, 1.0);
    }

    /// The estimates as text: `rcbot-loadout 1`, then `l <value> <weight> <weapon ids>` per
    /// tried loadout (keyed by its weapons, so the list can change).
    pub fn save(&self) -> String {
        let mut s = store::header(FILE_KIND, FILE_VERSION);
        for (arm, l) in self.arms.iter().zip(LOADOUTS) {
            if arm.n > 0.0 {
                let ids: Vec<String> = l.iter().map(i32::to_string).collect();
                s += &format!("l {} {} {}\n", arm.value, arm.n, ids.join(","));
            }
        }
        s
    }

    /// Reads [`save`](Self::save)'s text; loadouts no longer listed, other versions and damaged
    /// lines are skipped.
    pub fn load(&mut self, text: &str) {
        let Some((FILE_VERSION, lines)) = store::read_header(text, FILE_KIND) else { return };
        for line in lines {
            let mut f = line.split_whitespace();
            if f.next() != Some("l") {
                continue;
            }
            let (Some(v), Some(ids)) = (store::floats(f.by_ref().take(2)), f.next()) else { continue };
            let ids: Option<Vec<i32>> = ids.split(',').map(|x| x.parse().ok()).collect();
            let (Some(ids), &[value, n]) = (ids, v.as_slice()) else { continue };
            if n < 0.0 {
                continue;
            }
            if let Some(i) = LOADOUTS.iter().position(|l| *l == ids.as_slice()) {
                self.arms[i] = Arm::new(value, n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    #[test]
    fn the_loadouts_are_legal_and_light() {
        assert!((20..=40).contains(&LOADOUTS.len()));
        for l in LOADOUTS {
            assert_eq!(l.len(), 2);
            assert!(l.iter().all(|&w| (1..=37).contains(&w) && w != 29 && weapon_slots(w) > 0), "{l:?}");
            assert!(loadout_slots(l) <= MAX_SLOTS, "{l:?}");
        }
        for (i, a) in LOADOUTS.iter().enumerate() {
            assert!(LOADOUTS[i + 1..].iter().all(|b| b != a));
        }
    }

    #[test]
    fn rewards_per_minute() {
        assert_eq!(life_reward(2, 1, 100.0, 120.0), 1.0);
        // A short life counts as half a minute.
        assert_eq!(life_reward(1, 1, 0.0, 5.0), 1.0);
        assert_eq!(life_reward(0, 1, 0.0, 1.0), -1.0);
        assert_eq!(life_reward(50, 0, 0.0, 30.0), 12.0);
    }

    #[test]
    fn only_what_the_inventory_shows_is_credited() {
        assert_eq!(credited_loadout(Some(0), &[5, 12, 24]), Some(0));
        // The buy failed: only the pistol came; no loadout is exactly that.
        assert_eq!(credited_loadout(Some(0), &[12]), None);
        // Something else came: the loadout it is.
        assert_eq!(credited_loadout(Some(0), &[12, 13, 25]), Some(1));
        assert_eq!(credited_loadout(None, &[]), None);
    }

    #[test]
    fn untried_first_then_the_best_and_old_lives_fade() {
        let mut b = LoadoutBandit::default();
        let prior = LoadoutBandit::default();
        let mut rng = XorShiftRandom::new(4);
        let mut tried = Vec::new();
        for _ in 0..LOADOUTS.len() {
            let i = b.choose(&prior, 100, false, &mut rng);
            assert!(!tried.contains(&i));
            tried.push(i);
            b.reward(i, if i == 7 { 3.0 } else { -1.0 });
        }
        // Mostly the best now; discounting keeps re-trying the others now and then.
        let mut best = 0;
        for _ in 0..60 {
            let i = b.choose(&prior, 100, false, &mut rng);
            best += u32::from(i == 7);
            b.reward(i, if i == 7 { 3.0 } else { -1.0 });
        }
        assert!(best > 30, "{best}");
        assert!(b.arms[0].n < 1.0);
        // Capacity: only what fits.
        let i = b.choose(&prior, 30, false, &mut rng);
        assert!(loadout_slots(LOADOUTS[i]) <= 30);
        let r = b.choose(&prior, 100, true, &mut rng);
        assert!(r < LOADOUTS.len());
    }

    #[test]
    fn the_prior_guides_a_new_bot() {
        let mut prior = LoadoutBandit::default();
        for (i, a) in prior.arms.iter_mut().enumerate() {
            *a = Arm::new(if i == 3 { 2.0 } else { 0.0 }, 10.0);
        }
        let b = LoadoutBandit::default();
        assert_eq!(b.choose(&prior, 100, false, &mut XorShiftRandom::new(1)), 3);
    }

    #[test]
    fn saves_and_loads() {
        let mut b = LoadoutBandit::default();
        b.reward(2, 1.5);
        b.reward(5, -0.25);
        let text = b.save();
        let mut back = LoadoutBandit::default();
        back.load(&text);
        assert_eq!(back, b);
        let mut bad = LoadoutBandit::default();
        bad.load("rcbot-loadout 1\nl 1 2 99,98\nl x 1 5,12\nl 1 -1 5,12\nl 0.5 3 13,12\n");
        assert_eq!(bad.arms[1], Arm::new(0.5, 3.0));
        assert_eq!(bad.arms[0], Arm::default());
    }
}
