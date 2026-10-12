//! The GA's individuals (`bot_ga.{h,cpp}`): `CBotGAValues` (a vector of weights), `CIntGAValues`
//! (one `int` of bits), `CBitsGAValues` (a [`Bits`] string), and `CCombatFitnessMetrics`.

use super::bits::Bits;
use super::ga::{GaValue, Individual, MAX_PERTURBATION, MUTATE_RATE, PopulationMember};
use super::header::{GenericHeader, LearnType};
use super::io::{Reader, WriteLe};
use crate::Random;

/// `CBotGAValues`: weights the bot reads by index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BotGaValues {
    values: Vec<GaValue>,
    fitness: GaValue,
}

impl BotGaValues {
    /// `CBotGAValues()`: empty, fitness 0.
    pub fn new() -> BotGaValues {
        BotGaValues::default()
    }

    /// `CBotGAValues(values)`.
    pub fn from_values(values: Vec<GaValue>) -> BotGaValues {
        BotGaValues { values, fitness: 0.0 }
    }

    /// `m_theValues.size()`.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// No values.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// `get`. Upstream asserts the index (undefined behaviour in release builds); out of range is
    /// 0 here.
    pub fn get(&self, index: usize) -> GaValue {
        self.values.get(index).copied().unwrap_or(0.0)
    }

    /// `set`: out of range is ignored (an assert upstream).
    pub fn set(&mut self, index: usize, value: GaValue) {
        if let Some(v) = self.values.get_mut(index) {
            *v = value;
        }
    }

    /// `add`.
    pub fn add(&mut self, value: GaValue) {
        self.values.push(value);
    }

    /// `getVector` / `returnVector`.
    pub fn values(&self) -> &[GaValue] {
        &self.values
    }

    /// `setVector`.
    pub fn set_values(&mut self, values: Vec<GaValue>) {
        self.values = values;
    }
}

impl Individual for BotGaValues {
    fn fitness(&self) -> GaValue {
        self.fitness
    }

    fn set_fitness(&mut self, fitness: GaValue) {
        self.fitness = fitness;
    }

    /// `CBotGAValues::load`: header (`LEARNTYPE_GAVALUES`, `req_size`), `unsigned` count, the
    /// values, the fitness. A header mismatch consumes the header and keeps the values.
    fn load(&mut self, r: &mut Reader, req_size: i32) {
        if r.eof() {
            return;
        }
        if !GenericHeader::read(r, &GenericHeader::new(LearnType::GaValues, req_size)) {
            return;
        }
        self.values.clear();
        let Some(count) = r.read_u32() else { return };
        if count > 0 && !r.read_f32s(count, &mut self.values) {
            self.values.clear();
            return;
        }
        if let Some(f) = r.read_f32() {
            self.fitness = f;
        }
    }

    /// `CBotGAValues::save`.
    fn save(&self, out: &mut Vec<u8>) {
        GenericHeader::new(LearnType::GaValues, self.values.len() as i32).write(out);
        out.put_u32(self.values.len() as u32);
        out.put_f32s(&self.values);
        out.put_f32(self.fitness);
    }

    /// `CBotGAValues::crossOver`: one point, `RANDOM_LONG(0, size - 1)`; the genes before it are
    /// swapped. Upstream assumes `other` is as long; a shorter one only swaps what it has.
    fn cross_over(&mut self, other: &mut Self, rng: &mut dyn Random) {
        if self.values.is_empty() {
            return;
        }
        let point = rng.random_long(0, self.values.len() as i32 - 1).max(0) as usize;
        for i in 0..point.min(other.values.len()) {
            std::mem::swap(&mut self.values[i], &mut other.values[i]);
        }
    }

    /// `CBotGAValues::mutate`: each gene, with probability [`MUTATE_RATE`], moves by up to
    /// ±[`MAX_PERTURBATION`] of itself.
    fn mutate(&mut self, rng: &mut dyn Random) {
        for v in &mut self.values {
            if rng.random_float(0.0, 1.0) < MUTATE_RATE {
                let cur = *v;
                *v = cur + cur * (-1.0 + rng.random_float(0.0, 2.0)) * MAX_PERTURBATION;
            }
        }
    }

    /// `CBotGAValues::mutateWithRate`: as [`Individual::mutate`] with probability `rate` and the
    /// perturbation scaled by `rate / MUTATE_RATE`.
    fn mutate_with_rate(&mut self, rate: f32, rng: &mut dyn Random) {
        for v in &mut self.values {
            if rng.random_float(0.0, 1.0) < rate {
                let cur = *v;
                let perturb_scale = rate / MUTATE_RATE;
                let perturbation = MAX_PERTURBATION * perturb_scale;
                *v = cur + cur * (-1.0 + rng.random_float(0.0, 2.0)) * perturbation;
            }
        }
    }

    /// `CBotGAValues::clear`: drops the values (the fitness stays).
    fn clear(&mut self) {
        self.values.clear();
    }
}

impl PopulationMember for BotGaValues {
    fn blank() -> Self {
        BotGaValues::new()
    }
}

/// `CIntGAValues`: an `int` bred bitwise (NS combat upgrades upstream; kept because the bot's
/// learned-data file carries one).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntGaValues {
    value: i32,
    fitness: GaValue,
}

impl IntGaValues {
    /// `CIntGAValues(val)`.
    pub fn new(value: i32) -> IntGaValues {
        IntGaValues { value, fitness: 0.0 }
    }

    /// `get`.
    pub fn get(&self) -> i32 {
        self.value
    }

    /// `set`.
    pub fn set(&mut self, value: i32) {
        self.value = value;
    }
}

impl Individual for IntGaValues {
    fn fitness(&self) -> GaValue {
        self.fitness
    }

    fn set_fitness(&mut self, fitness: GaValue) {
        self.fitness = fitness;
    }

    /// `CIntGAValues::load`: an `int` check that must equal `req_size`, then the value and the
    /// fitness. Upstream saves the check as `sizeof(int)` (4) and the bot loads with 1, so a
    /// bot's own value never loads back (§5); kept.
    fn load(&mut self, r: &mut Reader, req_size: i32) {
        if r.read_i32() == Some(req_size) {
            if let Some(v) = r.read_i32() {
                self.value = v;
            }
            if let Some(f) = r.read_f32() {
                self.set_fitness(f);
            }
        }
    }

    /// `CIntGAValues::save`: `4`, the value, the fitness.
    fn save(&self, out: &mut Vec<u8>) {
        out.put_i32(4);
        out.put_i32(self.value);
        out.put_f32(self.fitness);
    }

    /// `CIntGAValues::crossOver`: the mask is `RANDOM_LONG(0, 32)`, so only the low six bits
    /// ever come from this side; kept.
    fn cross_over(&mut self, other: &mut Self, rng: &mut dyn Random) {
        let mask = rng.random_long(0, 32);
        let temp = self.value;
        self.value = (self.value & mask) | (other.value & !mask);
        other.value = (other.value & mask) | (temp & !mask);
    }

    /// `CIntGAValues::mutate`: `mutateWithRate(g_fMutateRate)`.
    fn mutate(&mut self, rng: &mut dyn Random) {
        self.mutate_with_rate(MUTATE_RATE, rng);
    }

    /// `CIntGAValues::mutateWithRate`: with probability `rate`, flips one random bit.
    fn mutate_with_rate(&mut self, rate: f32, rng: &mut dyn Random) {
        if rng.random_float(0.0, 1.0) < rate {
            let bit = rng.random_long(0, 31);
            self.value ^= 1i32.wrapping_shl(bit as u32);
        }
    }

    /// `CIntGAValues::clear`.
    fn clear(&mut self) {
        self.value = 0;
    }
}

impl PopulationMember for IntGaValues {
    fn blank() -> Self {
        IntGaValues::new(0)
    }
}

/// `CBitsGAValues`: a bit string. Nothing upstream breeds one; ported with the rest of the GA.
#[derive(Clone, Debug, PartialEq)]
pub struct BitsGaValues {
    bits: Bits,
    fitness: GaValue,
}

impl BitsGaValues {
    /// `CBitsGAValues(iNumBits)`.
    pub fn new(num_bits: u32) -> BitsGaValues {
        BitsGaValues { bits: Bits::new(num_bits), fitness: 0.0 }
    }

    /// `CBitsGAValues(CBits*)`.
    pub fn from_bits(bits: Bits) -> BitsGaValues {
        BitsGaValues { bits, fitness: 0.0 }
    }

    /// `size`: the number of bits.
    pub fn size(&self) -> u32 {
        self.bits.num_bits()
    }

    /// The bits.
    pub fn bits(&self) -> &Bits {
        &self.bits
    }

    /// `get`.
    pub fn get(&self, index: u32) -> bool {
        self.bits.get(index)
    }

    /// `set`.
    pub fn set(&mut self, index: u32, on: bool) {
        self.bits.set(index, on);
    }

    /// `random`.
    pub fn randomize(&mut self, rng: &mut dyn Random) {
        self.bits.randomize(rng);
    }

    /// `convert`: adds bit `i` as `1 << i` to `out` (which it doesn't reset). Bits from 32 on
    /// are undefined behaviour upstream and ignored here.
    pub fn convert(&self, out: &mut u32) {
        for i in 0..self.size().min(32) {
            if self.bits.get(i) {
                *out = out.wrapping_add(1 << i);
            }
        }
    }
}

impl Individual for BitsGaValues {
    fn fitness(&self) -> GaValue {
        self.fitness
    }

    fn set_fitness(&mut self, fitness: GaValue) {
        self.fitness = fitness;
    }

    /// `CBitsGAValues::load`: header (`LEARNTYPE_GAVALUES`, `req_size`), the [`Bits`], fitness.
    fn load(&mut self, r: &mut Reader, req_size: i32) {
        if !GenericHeader::read(r, &GenericHeader::new(LearnType::GaValues, req_size)) {
            return;
        }
        self.bits.load(r);
        if let Some(f) = r.read_f32() {
            self.fitness = f;
        }
    }

    /// `CBitsGAValues::save`.
    fn save(&self, out: &mut Vec<u8>) {
        GenericHeader::new(LearnType::GaValues, self.bits.num_bits() as i32).write(out);
        self.bits.save(out);
        out.put_f32(self.fitness);
    }

    /// `CBitsGAValues::crossOver`: `RANDOM_LONG(0, bits - 1)` picks a point; whole bytes before
    /// its byte are swapped, the bits below it within its byte are swapped, and so are all whole
    /// bytes after it. Upstream swaps both sides of the point, so the children are the parents
    /// swapped except for the top bits of one byte (§5); kept.
    fn cross_over(&mut self, other: &mut Self, rng: &mut dyn Random) {
        let num_bits = self.bits.num_bits();
        if num_bits == 0 {
            return;
        }
        let point = rng.random_long(0, num_bits as i32 - 1).max(0) as usize;
        let (cross_byte, cross_bit) = (point / 8, point % 8);
        let p1 = self.bits.bytes_mut();
        let p2 = other.bits.bytes_mut();
        let n = p1.len().min(p2.len());
        for i in 0..cross_byte.min(n) {
            std::mem::swap(&mut p1[i], &mut p2[i]);
        }
        if cross_bit > 0 && cross_byte < n {
            let mask1 = (1u8 << cross_bit) - 1;
            let mask2 = !mask1;
            let (b1, b2) = (p1[cross_byte], p2[cross_byte]);
            p1[cross_byte] = (b1 & mask2) | (b2 & mask1);
            p2[cross_byte] = (b2 & mask2) | (b1 & mask1);
        }
        for i in cross_byte + 1..n {
            std::mem::swap(&mut p1[i], &mut p2[i]);
        }
    }

    /// `CBitsGAValues::mutate`: flips each bit with probability [`MUTATE_RATE`]. It has no
    /// `mutateWithRate`, so the GA's adaptive rate doesn't reach it.
    fn mutate(&mut self, rng: &mut dyn Random) {
        for i in 0..self.bits.num_bits() {
            if rng.random_float(0.0, 1.0) < MUTATE_RATE {
                let on = self.bits.get(i);
                self.bits.set(i, !on);
            }
        }
    }

    /// `CBitsGAValues::clear`.
    fn clear(&mut self) {
        self.bits.clear();
    }
}

/// `CCombatFitnessMetrics` (`bot_ga.h`): a weighted combat score. Upstream declares it but
/// nothing uses it yet; the bot core can.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CombatFitnessMetrics {
    /// `damageDealt`.
    pub damage_dealt: f32,
    /// `damageTaken`.
    pub damage_taken: f32,
    /// `accuracy`: hits over shots, 0 to 1.
    pub accuracy: f32,
    /// `survivalTime`, seconds.
    pub survival_time: f32,
    /// `kills`.
    pub kills: i32,
    /// `deaths`.
    pub deaths: i32,
    /// `assists`.
    pub assists: i32,
    /// `objectivesCompleted`.
    pub objectives_completed: i32,
}

impl CombatFitnessMetrics {
    /// `WEIGHT_DAMAGE_RATIO`.
    pub const WEIGHT_DAMAGE_RATIO: f32 = 0.25;
    /// `WEIGHT_KD_RATIO`.
    pub const WEIGHT_KD_RATIO: f32 = 0.25;
    /// `WEIGHT_ACCURACY`.
    pub const WEIGHT_ACCURACY: f32 = 0.15;
    /// `WEIGHT_SURVIVAL`.
    pub const WEIGHT_SURVIVAL: f32 = 0.15;
    /// `WEIGHT_OBJECTIVES`.
    pub const WEIGHT_OBJECTIVES: f32 = 0.20;

    /// `reset`.
    pub fn reset(&mut self) {
        *self = CombatFitnessMetrics::default();
    }

    /// `recordHit`.
    pub fn record_hit(&mut self, damage: f32) {
        self.damage_dealt += damage;
    }

    /// `recordDamageTaken`.
    pub fn record_damage_taken(&mut self, damage: f32) {
        self.damage_taken += damage;
    }

    /// `recordAccuracy`: ignored without shots.
    pub fn record_accuracy(&mut self, hits: i32, shots_fired: i32) {
        if shots_fired > 0 {
            self.accuracy = hits as f32 / shots_fired as f32;
        }
    }

    /// `recordKill`.
    pub fn record_kill(&mut self) {
        self.kills += 1;
    }

    /// `recordDeath`.
    pub fn record_death(&mut self) {
        self.deaths += 1;
    }

    /// `recordAssist`.
    pub fn record_assist(&mut self) {
        self.assists += 1;
    }

    /// `recordObjective`.
    pub fn record_objective(&mut self) {
        self.objectives_completed += 1;
    }

    /// `updateSurvivalTime`.
    pub fn update_survival_time(&mut self, delta: f32) {
        self.survival_time += delta;
    }

    /// `getComponentScores`: damage ratio, K/D, accuracy, survival, objectives, each 0 to 1.
    pub fn component_scores(&self) -> [GaValue; 5] {
        let damage = if self.damage_taken > 0.0 {
            (self.damage_dealt / self.damage_taken).min(10.0) / 10.0
        } else if self.damage_dealt > 0.0 {
            1.0
        } else {
            0.0
        };
        let effective_kills = self.kills as f32 + self.assists as f32 * 0.5;
        let kd = if self.deaths > 0 {
            (effective_kills / self.deaths as f32).min(5.0) / 5.0
        } else if effective_kills > 0.0 {
            1.0
        } else {
            0.0
        };
        [
            damage,
            kd,
            self.accuracy.clamp(0.0, 1.0),
            (self.survival_time / 300.0).min(1.0),
            (self.objectives_completed as f32 / 5.0).min(1.0),
        ]
    }

    /// `calculateFitness`: the weighted sum of [`CombatFitnessMetrics::component_scores`].
    pub fn calculate_fitness(&self) -> GaValue {
        let [d, kd, acc, surv, obj] = self.component_scores();
        d * Self::WEIGHT_DAMAGE_RATIO
            + kd * Self::WEIGHT_KD_RATIO
            + acc * Self::WEIGHT_ACCURACY
            + surv * Self::WEIGHT_SURVIVAL
            + obj * Self::WEIGHT_OBJECTIVES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    struct Script(Vec<f32>);
    impl Random for Script {
        fn random_long(&mut self, lo: i32, _hi: i32) -> i32 {
            if self.0.is_empty() { lo } else { self.0.remove(0) as i32 }
        }
        fn random_float(&mut self, lo: f32, _hi: f32) -> f32 {
            if self.0.is_empty() { lo } else { self.0.remove(0) }
        }
    }

    #[test]
    fn values_file_layout_and_round_trip() {
        let mut v = BotGaValues::from_values(vec![0.5, -1.0]);
        v.set_fitness(2.0);
        let mut out = Vec::new();
        v.save(&mut out);
        let mut expect = b"t-3-s-2".to_vec();
        expect.resize(32, 0);
        expect.extend_from_slice(&2u32.to_le_bytes());
        for f in [0.5f32, -1.0, 2.0] {
            expect.extend_from_slice(&f.to_le_bytes());
        }
        assert_eq!(out, expect);
        let mut back = BotGaValues::new();
        back.load(&mut Reader::new(&out), 2);
        assert_eq!(back, v);

        // Wrong size: only the header is consumed, values kept.
        let mut other = BotGaValues::from_values(vec![7.0]);
        let mut r = Reader::new(&out);
        other.load(&mut r, 3);
        assert_eq!(other.values(), &[7.0]);
        assert_eq!(r.remaining(), 16);

        // Truncated values: cleared.
        let mut t = BotGaValues::from_values(vec![7.0]);
        t.load(&mut Reader::new(&out[..38]), 2);
        assert!(t.is_empty());
    }

    #[test]
    fn values_crossover_and_mutation() {
        let mut a = BotGaValues::from_values(vec![1.0, 2.0, 3.0]);
        let mut b = BotGaValues::from_values(vec![4.0, 5.0, 6.0]);
        a.cross_over(&mut b, &mut Script(vec![2.0]));
        assert_eq!(a.values(), &[4.0, 5.0, 3.0]);
        assert_eq!(b.values(), &[1.0, 2.0, 6.0]);

        // Gene 0 mutates by (-1 + 1.5) * 0.3 = +15%, gene 1 doesn't.
        let mut m = BotGaValues::from_values(vec![2.0, 2.0]);
        m.mutate(&mut Script(vec![0.05, 1.5, 0.5]));
        assert_eq!(m.values(), &[2.0 + 2.0 * 0.5 * 0.3, 2.0]);

        // At rate 0.2 the perturbation doubles.
        let mut w = BotGaValues::from_values(vec![2.0]);
        w.mutate_with_rate(0.2, &mut Script(vec![0.1, 1.5]));
        assert_eq!(w.values(), &[2.0 + 2.0 * 0.5 * (0.3 * (0.2 / 0.1))]);

        assert_eq!(w.get(5), 0.0);
        w.set(5, 1.0);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn int_values() {
        let mut a = IntGaValues::new(0b1111_0000);
        let mut b = IntGaValues::new(0b0000_1111);
        a.cross_over(&mut b, &mut Script(vec![0b11_0000 as f32]));
        assert_eq!(a.get(), 0b11_1111);
        assert_eq!(b.get(), 0b1100_0000);
        a.mutate(&mut Script(vec![0.0, 3.0]));
        assert_eq!(a.get(), 0b11_0111);
        a.mutate_with_rate(0.5, &mut Script(vec![0.6]));
        assert_eq!(a.get(), 0b11_0111);

        a.set_fitness(1.5);
        let mut out = Vec::new();
        a.save(&mut out);
        assert_eq!(&out[..8], &[4, 0, 0, 0, 0b11_0111, 0, 0, 0]);
        let mut back = IntGaValues::new(0);
        back.load(&mut Reader::new(&out), 4);
        assert_eq!(back, a);
        // The bot loads with req_size 1: nothing after the check is read.
        let mut bot = IntGaValues::new(0);
        let mut r = Reader::new(&out);
        bot.load(&mut r, 1);
        assert_eq!((bot.get(), r.remaining()), (0, 8));
    }

    #[test]
    fn bits_values() {
        let mut a = BitsGaValues::new(20);
        let mut b = BitsGaValues::new(20);
        for i in 0..20 {
            a.set(i, true);
        }
        // Point 10: byte 0 swapped, byte 1's low 2 bits swapped, byte 2 swapped.
        a.cross_over(&mut b, &mut Script(vec![10.0]));
        assert_eq!(a.bits().bytes(), &[0, 0b1111_1100, 0]);
        assert_eq!(b.bits().bytes(), &[0xff, 0b0000_0011, 0x0f]);

        let mut n = 0;
        b.convert(&mut n);
        assert_eq!(n, 0x0f03ff);

        b.set_fitness(0.25);
        let mut out = Vec::new();
        b.save(&mut out);
        assert_eq!(out.len(), 32 + 32 + 4 + 3 + 4);
        let mut back = BitsGaValues::new(20);
        back.load(&mut Reader::new(&out), 20);
        assert_eq!(back, b);

        let mut r = XorShiftRandom::new(1);
        back.mutate(&mut r);
        back.randomize(&mut r);
        back.clear();
        assert!(back.bits().bytes().iter().all(|&x| x == 0));
    }

    #[test]
    fn combat_fitness() {
        let mut m = CombatFitnessMetrics::default();
        assert_eq!(m.calculate_fitness(), 0.0);
        m.record_hit(50.0);
        m.record_damage_taken(10.0);
        m.record_kill();
        m.record_assist();
        m.record_death();
        m.record_accuracy(1, 4);
        m.record_accuracy(1, 0);
        m.update_survival_time(150.0);
        m.record_objective();
        assert_eq!(m.component_scores(), [0.5, 1.5 / 5.0, 0.25, 0.5, 0.2]);
        assert_eq!(m.calculate_fitness(), 0.5 * 0.25 + (1.5 / 5.0) * 0.25 + 0.25 * 0.15 + 0.5 * 0.15 + 0.2 * 0.2);
        m.record_hit(1000.0);
        assert_eq!(m.component_scores()[0], 1.0);
        m.reset();
        assert_eq!(m, CombatFitnessMetrics::default());
    }
}
