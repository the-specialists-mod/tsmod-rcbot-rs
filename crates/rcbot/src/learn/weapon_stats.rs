//! How well each weapon works at each range (a port addition, design §4): per bot, and a global
//! prior every bot adds to.
//!
//! A cell per TS weapon id (0 kung fu .. 37) and range bin ([`super::ledger::range_bin`]) holds
//! the seconds the bot saw its enemy with that weapon in hand at that range (exposure), the
//! damage it dealt then, the rounds it fired and the hits. Old samples fade with exposure (half
//! weight after [`HALF_LIFE`] seconds of it), so the sums are exponential moving averages. The
//! score is damage per second of exposure, shrunk toward the prior: `(n v + m p) / (n + m)`
//! with `n` the exposure and `m` [`PRIOR_WEIGHT`].

use super::ledger::{NUM_RANGE_BINS, range_bin};
use super::store;

/// TS weapon ids 0 (kung fu) to 37.
pub const NUM_WEAPON_IDS: usize = 38;
/// Seconds of exposure after which a sample weighs half.
pub const HALF_LIFE: f32 = 600.0;
/// Seconds of the prior a bot's own estimate is shrunk toward.
pub const PRIOR_WEIGHT: f32 = 5.0;
/// Damage per second assumed with no data at all.
pub const DEFAULT_DPS: f32 = 10.0;
/// Seconds of exposure before a bot's own score is offered ([`crate::bot::Bot::weapon_score`]).
pub const MIN_SAMPLES: f32 = 5.0;
/// `"<key>.rwr"`, in the profiles folder: a bot's table.
pub const FILE_EXT: &str = "rwr";
/// The global prior's file, in the bot folder.
pub const GLOBAL_FILE: &str = "weaponstats.rwr";

const FILE_KIND: &str = "weapons";
const FILE_VERSION: u32 = 1;

/// One weapon at one range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    /// Seconds the enemy was in sight with this weapon in hand.
    pub exposure: f32,
    /// Damage dealt meanwhile.
    pub damage: f32,
    /// Rounds fired.
    pub shots: f32,
    /// Hits.
    pub hits: f32,
}

impl Cell {
    fn fade(&mut self, dt: f32) {
        let k = (-dt * std::f32::consts::LN_2 / HALF_LIFE).exp();
        self.exposure *= k;
        self.damage *= k;
        self.shots *= k;
        self.hits *= k;
    }
}

/// The table.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponStats {
    cells: Vec<[Cell; NUM_RANGE_BINS]>,
}

impl Default for WeaponStats {
    fn default() -> Self {
        WeaponStats { cells: vec![[Cell::default(); NUM_RANGE_BINS]; NUM_WEAPON_IDS] }
    }
}

/// A TS weapon id as the table keys it: the port's kung fu id (38) is 0.
fn slot(weapon: i32) -> Option<usize> {
    let w = if weapon == crate::weapons::TsWeapon::KungFu.id() { 0 } else { weapon };
    usize::try_from(w).ok().filter(|&w| w < NUM_WEAPON_IDS)
}

impl WeaponStats {
    /// The cell of `weapon` at `dist`.
    pub fn cell(&self, weapon: i32, dist: f32) -> Option<&Cell> {
        slot(weapon).map(|w| &self.cells[w][range_bin(dist)])
    }

    fn cell_mut(&mut self, weapon: i32, dist: f32) -> Option<&mut Cell> {
        slot(weapon).map(|w| &mut self.cells[w][range_bin(dist)])
    }

    /// `dt` seconds with the enemy in sight at `dist`, `weapon` in hand.
    pub fn add_exposure(&mut self, weapon: i32, dist: f32, dt: f32) {
        if let Some(c) = self.cell_mut(weapon, dist).filter(|_| dt > 0.0) {
            c.fade(dt);
            c.exposure += dt;
        }
    }

    /// A hit for `damage` with `weapon` at `dist`.
    pub fn add_hit(&mut self, weapon: i32, dist: f32, damage: f32) {
        if let Some(c) = self.cell_mut(weapon, dist) {
            c.damage += damage.max(0.0);
            c.hits += 1.0;
        }
    }

    /// `rounds` fired with `weapon` at `dist`.
    pub fn add_shots(&mut self, weapon: i32, dist: f32, rounds: f32) {
        if let Some(c) = self.cell_mut(weapon, dist) {
            c.shots += rounds.max(0.0);
        }
    }

    /// Damage per second of exposure and the exposure behind it, `None` without exposure.
    pub fn dps(&self, weapon: i32, dist: f32) -> Option<(f32, f32)> {
        self.cell(weapon, dist).filter(|c| c.exposure > 0.0).map(|c| (c.damage / c.exposure, c.exposure))
    }

    /// Hits per round fired, `None` before any round.
    pub fn accuracy(&self, weapon: i32, dist: f32) -> Option<f32> {
        self.cell(weapon, dist).filter(|c| c.shots >= 1.0).map(|c| (c.hits / c.shots).min(1.0))
    }

    /// Seconds of exposure behind `weapon` at `dist`.
    pub fn samples(&self, weapon: i32, dist: f32) -> f32 {
        self.cell(weapon, dist).map_or(0.0, |c| c.exposure)
    }

    /// The score of `weapon` at `dist`: this table's damage per second shrunk toward `prior`'s
    /// (itself shrunk toward [`DEFAULT_DPS`]).
    pub fn score(&self, prior: &WeaponStats, weapon: i32, dist: f32) -> f32 {
        let p = prior.dps(weapon, dist).map_or(DEFAULT_DPS, |(v, n)| (n * v + PRIOR_WEIGHT * DEFAULT_DPS) / (n + PRIOR_WEIGHT));
        match self.dps(weapon, dist) {
            Some((v, n)) => (n * v + PRIOR_WEIGHT * p) / (n + PRIOR_WEIGHT),
            None => p,
        }
    }

    /// The table as text: `rcbot-weapons 1`, then `w <id> <bin> <exposure> <damage> <shots>
    /// <hits>` for every cell with data.
    pub fn save(&self) -> String {
        let mut s = store::header(FILE_KIND, FILE_VERSION);
        for (w, bins) in self.cells.iter().enumerate() {
            for (b, c) in bins.iter().enumerate() {
                if *c != Cell::default() {
                    s += &format!("w {w} {b} {}\n", store::join(&[c.exposure, c.damage, c.shots, c.hits]));
                }
            }
        }
        s
    }

    /// Reads [`save`](Self::save)'s text; anything else, or a damaged line, leaves that part as
    /// it was.
    pub fn load(&mut self, text: &str) {
        let Some((FILE_VERSION, lines)) = store::read_header(text, FILE_KIND) else { return };
        for line in lines {
            let mut f = line.split_whitespace();
            if f.next() != Some("w") {
                continue;
            }
            let (Some(w), Some(b)) = (f.next().and_then(|v| v.parse::<usize>().ok()), f.next().and_then(|v| v.parse::<usize>().ok()))
            else {
                continue;
            };
            let Some(v) = store::floats(f).filter(|v| v.len() == 4 && v.iter().all(|&x| x >= 0.0)) else { continue };
            if let Some(c) = self.cells.get_mut(w).and_then(|bins| bins.get_mut(b)) {
                *c = Cell { exposure: v[0], damage: v[1], shots: v[2], hits: v[3] };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_per_second_of_exposure_by_range() {
        let mut t = WeaponStats::default();
        t.add_exposure(5, 300.0, 2.0);
        t.add_hit(5, 300.0, 30.0);
        t.add_hit(5, 250.0, 30.0);
        t.add_shots(5, 300.0, 6.0);
        let (v, n) = t.dps(5, 200.0).unwrap();
        assert!((v - 30.0).abs() < 0.01 && (n - 2.0).abs() < 0.01, "{v} {n}");
        assert_eq!(t.accuracy(5, 300.0), Some(2.0 / 6.0));
        // Other ranges and weapons know nothing; kung fu is 0 or 38.
        assert!(t.dps(5, 1000.0).is_none() && t.dps(1, 300.0).is_none());
        t.add_exposure(38, 10.0, 1.0);
        assert!(t.dps(0, 10.0).is_some());
        assert!(t.cell(99, 0.0).is_none());
    }

    #[test]
    fn scores_shrink_to_the_prior() {
        let mut prior = WeaponStats::default();
        let t = WeaponStats::default();
        assert_eq!(t.score(&prior, 5, 300.0), DEFAULT_DPS);
        prior.add_exposure(5, 300.0, 1000.0);
        prior.add_hit(5, 300.0, 20000.0);
        let p = t.score(&prior, 5, 300.0);
        assert!(p > DEFAULT_DPS, "{p}");
        let mut mine = WeaponStats::default();
        mine.add_exposure(5, 300.0, PRIOR_WEIGHT);
        let s = mine.score(&prior, 5, 300.0);
        // Five seconds without a hit against five of the prior: half way to 0.
        assert!((s - p / 2.0).abs() < 0.05, "{s} {p}");
    }

    #[test]
    fn old_samples_fade_with_exposure() {
        let mut t = WeaponStats::default();
        t.add_exposure(1, 100.0, 1.0);
        t.add_hit(1, 100.0, 100.0);
        t.add_exposure(1, 100.0, HALF_LIFE);
        let c = t.cell(1, 100.0).unwrap();
        assert!((c.damage - 50.0).abs() < 0.01, "{c:?}");
    }

    #[test]
    fn saves_and_loads() {
        let mut t = WeaponStats::default();
        t.add_exposure(13, 900.0, 3.5);
        t.add_hit(13, 900.0, 39.0);
        t.add_shots(13, 900.0, 4.0);
        let text = t.save();
        let mut back = WeaponStats::default();
        back.load(&text);
        assert_eq!(back, t);
        let mut bad = WeaponStats::default();
        bad.load("rcbot-weapons 1\nw 99 0 1 1 1 1\nw 1 9 1 1 1 1\nw 1 0 1 -1 1 1\nw 1 0 1 x 1 1\nw 2 1 1 2 3 4");
        assert_eq!(bad.cell(2, 100.0), Some(&Cell { exposure: 1.0, damage: 2.0, shots: 3.0, hits: 4.0 }));
        assert!(bad.cell(1, 0.0).is_some_and(|c| *c == Cell::default()));
        let mut other = WeaponStats::default();
        other.load("rcbot-weapons 2\nw 2 1 1 2 3 4");
        assert_eq!(other, WeaponStats::default());
    }
}
