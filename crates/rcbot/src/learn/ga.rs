//! RCBot's genetic algorithm (`ga.{h,cpp}`): `IIndividual`, `CPopulation`, the selection
//! functions and `CGA`.
//!
//! The bot breeds several things with it (`bot.cpp`): each team's enemy-cost weights, the TS
//! loadout and stunt weights, the bot's personal jump/duck/strafe weights. A finished life is
//! scored and handed to [`Ga::add_to_population`]; once the population is full, [`Ga::epoch`]
//! breeds a new generation, which bots then [`Ga::pick`] from at spawn.
//!
//! Upstream's `m_iPopType` (which class `CPopulation::load` creates) is the type parameter here.

use super::header::{GenericHeader, LearnType};
use super::io::{Reader, WriteLe};
use crate::Random;

/// `ga_value` (`gannconst.h`).
pub type GaValue = f32;

/// `IIndividual`: something the GA can score, breed and save.
pub trait Individual: Clone {
    /// `getFitness`.
    fn fitness(&self) -> GaValue;
    /// `setFitness`.
    fn set_fitness(&mut self, fitness: GaValue);
    /// `load(bfp, req_size)`.
    fn load(&mut self, r: &mut Reader, req_size: i32);
    /// `save(bfp)`.
    fn save(&self, out: &mut Vec<u8>);
    /// `crossOver(other)`: swaps genes with `other`.
    fn cross_over(&mut self, other: &mut Self, rng: &mut dyn Random);
    /// `mutate`, at [`MUTATE_RATE`].
    fn mutate(&mut self, rng: &mut dyn Random);
    /// `mutateWithRate`: falls back to [`Individual::mutate`] unless the type has its own.
    fn mutate_with_rate(&mut self, _rate: f32, rng: &mut dyn Random) {
        self.mutate(rng);
    }
    /// `clear`.
    fn clear(&mut self);
}

/// The individual types `CPopulation::load` can create (`TYPE_BOTGAVALS`, `TYPE_INTGAVALS`).
pub trait PopulationMember: Individual {
    /// `new CBotGAValues()` / `new CIntGAValues()`.
    fn blank() -> Self;
}

/// `CGA::g_iDefaultMaxPopSize`.
pub const DEFAULT_MAX_POP_SIZE: u32 = 16;
/// `CGA::g_fCrossOverRate`.
pub const CROSS_OVER_RATE: f32 = 0.7;
/// `CGA::g_fMutateRate`.
pub const MUTATE_RATE: f32 = 0.1;
/// `CGA::g_fMaxPerturbation`.
pub const MAX_PERTURBATION: f32 = 0.3;

/// `CPopulation`.
#[derive(Clone, Debug, PartialEq)]
pub struct Population<I> {
    individuals: Vec<I>,
}

impl<I> Default for Population<I> {
    fn default() -> Self {
        Population { individuals: Vec::new() }
    }
}

impl<I: Individual> Population<I> {
    /// `size`.
    pub fn len(&self) -> usize {
        self.individuals.len()
    }

    /// `size() == 0`.
    pub fn is_empty(&self) -> bool {
        self.individuals.is_empty()
    }

    /// `get`.
    pub fn get(&self, index: usize) -> Option<&I> {
        self.individuals.get(index)
    }

    /// The individuals in order (the last is the next [`Population::pick`]).
    pub fn individuals(&self) -> &[I] {
        &self.individuals
    }

    /// `add`.
    pub fn add(&mut self, individual: I) {
        self.individuals.push(individual);
    }

    /// `clear` / `freeMemory`.
    pub fn clear(&mut self) {
        self.individuals.clear();
    }

    /// `getBestIndividual`: `std::max_element`, so the first of equal bests.
    pub fn best_index(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, ind) in self.individuals.iter().enumerate() {
            if best.is_none_or(|b| self.individuals[b].fitness() < ind.fitness()) {
                best = Some(i);
            }
        }
        best
    }

    /// `getBestIndividual`.
    pub fn best(&self) -> Option<&I> {
        self.best_index().map(|i| &self.individuals[i])
    }

    /// `totalFitness`: summed in order from 0.
    pub fn total_fitness(&self) -> GaValue {
        self.individuals.iter().fold(0.0, |sum, i| sum + i.fitness())
    }

    /// `bestFitness`: 0 when empty.
    pub fn best_fitness(&self) -> GaValue {
        self.best().map_or(0.0, |b| b.fitness())
    }

    /// `averageFitness`: 0 when empty.
    pub fn average_fitness(&self) -> GaValue {
        if self.individuals.is_empty() {
            return 0.0;
        }
        self.total_fitness() / self.individuals.len() as GaValue
    }

    /// `diversity`: the fitness standard deviation over the best fitness, at most 1; 0 for fewer
    /// than two individuals and 1 when the best is below 0.0001.
    pub fn diversity(&self) -> GaValue {
        if self.individuals.len() < 2 {
            return 0.0;
        }
        let avg = self.average_fitness();
        let best = self.best_fitness();
        if best < 0.0001 {
            return 1.0;
        }
        let mut variance: GaValue = 0.0;
        for i in &self.individuals {
            let diff = i.fitness() - avg;
            variance += diff * diff;
        }
        variance /= self.individuals.len() as GaValue;
        (variance.sqrt() / best).min(1.0)
    }

    /// `pick`: takes the last individual (upstream's random pick is commented out).
    pub fn pick(&mut self) -> Option<I> {
        self.individuals.pop()
    }

    /// `save`: header (`LEARNTYPE_POPULATION`, the GA's maximum size, not this population's),
    /// `unsigned` count, each individual.
    pub fn save(&self, out: &mut Vec<u8>, max_pop_size: u32) {
        GenericHeader::new(LearnType::Population, max_pop_size as i32).write(out);
        out.put_u32(self.individuals.len() as u32);
        for i in &self.individuals {
            i.save(out);
        }
    }

    /// `load`. Nothing happens at end of file or on a header mismatch (the population is kept);
    /// otherwise the population is replaced by as many individuals as the count says, until end
    /// of file. Every individual is kept even when its own load fails, as upstream keeps it.
    pub fn load(&mut self, r: &mut Reader, chromosize: i32, max_pop_size: u32)
    where
        I: PopulationMember,
    {
        if r.eof() {
            return;
        }
        if !GenericHeader::read(r, &GenericHeader::new(LearnType::Population, max_pop_size as i32)) {
            return;
        }
        let count = r.read_u32();
        self.individuals.clear();
        let Some(count) = count else { return };
        for _ in 0..count {
            if r.eof() {
                return;
            }
            let mut ind = I::blank();
            ind.load(r, chromosize);
            self.individuals.push(ind);
        }
    }
}

/// `ISelection`: how parents are chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// `CRouletteSelection`, the default.
    Roulette,
    /// `CTournamentSelection(size)` (upstream's default size is 3). Nothing upstream uses it.
    Tournament(i32),
}

impl Selection {
    /// `select`: an index into a non-empty population.
    pub fn select<I: Individual>(self, population: &Population<I>, rng: &mut dyn Random) -> usize {
        match self {
            Selection::Roulette => {
                let slice = rng.random_float(0.0, population.total_fitness());
                let mut so_far: GaValue = 0.0;
                for (i, ind) in population.individuals.iter().enumerate() {
                    so_far += ind.fitness();
                    if so_far >= slice {
                        return i;
                    }
                }
                0
            }
            Selection::Tournament(size) => {
                let mut best: Option<usize> = None;
                for _ in 0..size {
                    let idx = rng.random_long(0, population.len() as i32 - 1) as usize;
                    if best.is_none_or(|b| population.individuals[idx].fitness() > population.individuals[b].fitness()) {
                        best = Some(idx);
                    }
                }
                best.unwrap_or(0)
            }
        }
    }
}

/// `CGA`.
#[derive(Clone, Debug)]
pub struct Ga<I> {
    /// `m_iMaxPopSize`: the population that triggers an epoch, and how many a new generation has
    /// (give or take one, as babies come in pairs).
    pub max_pop_size: u32,
    population: Population<I>,
    new_population: Population<I>,
    generations: u32,
    prev_avg_fitness: f32,
    adaptive_mutation: bool,
    selection: Selection,
    best: Option<I>,
}

/// `CGA::ELITE_COUNT` (local to `epoch`).
const ELITE_COUNT: usize = 2;

impl<I: Individual> Ga<I> {
    /// `CGA(iMaxPopSize)`: roulette selection; 0 means [`DEFAULT_MAX_POP_SIZE`].
    pub fn new(max_pop_size: u32) -> Ga<I> {
        let max_pop_size = if max_pop_size == 0 { DEFAULT_MAX_POP_SIZE } else { max_pop_size };
        Ga {
            max_pop_size,
            population: Population::default(),
            new_population: Population::default(),
            generations: 0,
            prev_avg_fitness: 0.0,
            adaptive_mutation: true,
            selection: Selection::Roulette,
            best: None,
        }
    }

    /// `CGA(ISelection*)`: note that this constructor leaves the maximum size at 0 (no default),
    /// so every [`Ga::add_to_population`] runs an epoch that breeds nobody. Unused upstream.
    pub fn with_selection(selection: Selection) -> Ga<I> {
        Ga { max_pop_size: 0, selection, ..Ga::new(1) }
    }

    /// `setSize`.
    pub fn set_size(&mut self, size: u32) {
        self.max_pop_size = size;
    }

    /// `setAdaptiveMutation`.
    pub fn set_adaptive_mutation(&mut self, on: bool) {
        self.adaptive_mutation = on;
    }

    /// `isAdaptiveMutationEnabled`.
    pub fn is_adaptive_mutation_enabled(&self) -> bool {
        self.adaptive_mutation
    }

    /// `getNumGenerations`.
    pub fn generations(&self) -> u32 {
        self.generations
    }

    /// `m_fPrevAvgFitness`: the average fitness of the population the last epoch bred from.
    pub fn prev_avg_fitness(&self) -> f32 {
        self.prev_avg_fitness
    }

    /// `m_thePopulation`: scored individuals waiting for the next epoch.
    pub fn population(&self) -> &Population<I> {
        &self.population
    }

    /// `m_theNewPopulation`: the bred generation that [`Ga::pick`] hands out.
    pub fn new_population(&self) -> &Population<I> {
        &self.new_population
    }

    /// `m_bestIndividual`: the best individual seen at any epoch.
    pub fn best(&self) -> Option<&I> {
        self.best.as_ref()
    }

    /// `addToPopulation`: once the population is full, breed, remember the best ever, and start
    /// the next population with a copy of it.
    pub fn add_to_population(&mut self, individual: I, rng: &mut dyn Random) {
        self.population.add(individual);
        if self.population.len() as u32 >= self.max_pop_size {
            self.epoch(rng);
            if let Some(best) = self.population.best()
                && self.best.as_ref().is_none_or(|b| b.fitness() < best.fitness())
            {
                self.best = Some(best.clone());
            }
            self.population.clear();
            if let Some(best) = &self.best {
                self.population.add(best.clone());
            }
        }
    }

    /// `canPick`.
    pub fn can_pick(&self) -> bool {
        !self.new_population.is_empty()
    }

    /// `pick`: the last individual of the new generation.
    pub fn pick(&mut self) -> Option<I> {
        self.new_population.pick()
    }

    /// `getAdaptiveMutateRate`: 0.05 to 0.3 by how converged the population is, at least 0.15
    /// for the first 10 generations; [`MUTATE_RATE`] when adaptive mutation is off or the best
    /// fitness isn't positive.
    pub fn adaptive_mutate_rate(&self) -> f32 {
        if !self.adaptive_mutation {
            return MUTATE_RATE;
        }
        if self.population.best_fitness() <= 0.0 {
            return MUTATE_RATE;
        }
        const MIN_MUTATION: f32 = 0.05;
        const MAX_MUTATION: f32 = 0.30;
        let need = 1.0 - self.population.diversity();
        let mut rate = MIN_MUTATION + (MAX_MUTATION - MIN_MUTATION) * need;
        if self.generations < 10 {
            rate = rate.max(0.15);
        }
        rate
    }

    /// `epoch`: the new generation is two elite copies, then pairs of babies: parents chosen by
    /// the selection function, crossed over at [`CROSS_OVER_RATE`], both mutated at the adaptive
    /// rate.
    ///
    /// Upstream's elite loop asks for the best individual twice, so both elite slots hold the
    /// same one (§5); kept.
    pub fn epoch(&mut self, rng: &mut dyn Random) {
        self.new_population.clear();
        // Selecting from an empty population is undefined upstream; breed nothing instead.
        if !self.population.is_empty() {
            for _ in 0..ELITE_COUNT.min(self.population.len()) {
                if let Some(best) = self.population.best() {
                    self.new_population.add(best.clone());
                }
            }
            let rate = self.adaptive_mutate_rate();
            while (self.new_population.len() as u32) < self.max_pop_size {
                let mum = self.selection.select(&self.population, rng);
                let dad = self.selection.select(&self.population, rng);
                let mut baby1 = self.population.individuals[mum].clone();
                let mut baby2 = self.population.individuals[dad].clone();
                if rng.random_float(0.0, 1.0) < CROSS_OVER_RATE {
                    baby1.cross_over(&mut baby2, rng);
                }
                baby1.mutate_with_rate(rate, rng);
                baby2.mutate_with_rate(rate, rng);
                self.new_population.add(baby1);
                self.new_population.add(baby2);
            }
        }
        self.prev_avg_fitness = self.population.average_fitness();
        self.generations += 1;
    }

    /// `freeLocalMemory` (and `freeGlobalMemory`, which also frees the selection function).
    pub fn free_local_memory(&mut self) {
        self.population.clear();
        self.new_population.clear();
        self.generations = 0;
        self.best = None;
    }

    /// `save`: the population, then the new population.
    pub fn save(&self, out: &mut Vec<u8>) {
        self.population.save(out, self.max_pop_size);
        self.new_population.save(out, self.max_pop_size);
    }

    /// `load`: the population, then the new population; each individual must have
    /// `chromosize` genes.
    pub fn load(&mut self, r: &mut Reader, chromosize: i32)
    where
        I: PopulationMember,
    {
        self.population.load(r, chromosize, self.max_pop_size);
        self.new_population.load(r, chromosize, self.max_pop_size);
    }
}

/// `eGASaveType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveType {
    /// `SAVETYPE_BOT`: by profile id.
    Bot,
    /// `SAVETYPE_TEAM`: by team.
    Team,
}

/// The file name `RCBOpenFile` opens in the profiles folder: `<id>p<name>.rce` or
/// `<id>t<name>.rce`. Upstream marks it "not functional": only the unused `CGA::loadTeam` /
/// `saveTeam` and `CPerceptron::load/save(filename, id)` call it.
pub fn rce_file_name(save_type: SaveType, id: i32, name: &str) -> String {
    match save_type {
        SaveType::Bot => format!("{id}p{name}.rce"),
        SaveType::Team => format!("{id}t{name}.rce"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::values::{BotGaValues, IntGaValues};
    use super::*;
    use crate::XorShiftRandom;

    fn vals(v: &[f32], fitness: f32) -> BotGaValues {
        let mut b = BotGaValues::from_values(v.to_vec());
        b.set_fitness(fitness);
        b
    }

    /// Answers a script of values, so tests can steer selection and mutation.
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
    fn population_statistics() {
        let mut p = Population::default();
        assert_eq!((p.best_fitness(), p.average_fitness(), p.diversity()), (0.0, 0.0, 0.0));
        p.add(vals(&[1.0], 2.0));
        p.add(vals(&[2.0], 4.0));
        p.add(vals(&[3.0], 4.0));
        assert_eq!(p.best_index(), Some(1), "max_element keeps the first best");
        assert_eq!(p.total_fitness(), 10.0);
        assert_eq!(p.average_fitness(), 10.0 / 3.0);
        let avg: f32 = 10.0 / 3.0;
        let var = ((2.0 - avg) * (2.0 - avg) + 2.0 * (4.0 - avg) * (4.0 - avg)) / 3.0;
        assert_eq!(p.diversity(), var.sqrt() / 4.0);
        assert_eq!(p.pick().unwrap().get(0), 3.0);

        let mut flat = Population::default();
        flat.add(vals(&[], 0.0));
        flat.add(vals(&[], 0.00001));
        assert_eq!(flat.diversity(), 1.0);
    }

    #[test]
    fn roulette_and_tournament() {
        let mut p = Population::default();
        for f in [1.0, 2.0, 3.0] {
            p.add(vals(&[], f));
        }
        assert_eq!(Selection::Roulette.select(&p, &mut Script(vec![0.0])), 0);
        assert_eq!(Selection::Roulette.select(&p, &mut Script(vec![1.5])), 1);
        assert_eq!(Selection::Roulette.select(&p, &mut Script(vec![3.0])), 1, ">= stops at the boundary");
        assert_eq!(Selection::Roulette.select(&p, &mut Script(vec![99.0])), 0, "falls back to the first");
        assert_eq!(Selection::Tournament(3).select(&p, &mut Script(vec![0.0, 2.0, 1.0])), 2);
    }

    #[test]
    fn adaptive_rate() {
        let mut ga: Ga<BotGaValues> = Ga::new(0);
        assert_eq!(ga.max_pop_size, 16);
        assert_eq!(ga.adaptive_mutate_rate(), MUTATE_RATE, "no positive best fitness");
        ga.population.add(vals(&[], 1.0));
        // One individual: diversity 0, rate 0.3.
        assert_eq!(ga.adaptive_mutate_rate(), 0.05 + (0.30f32 - 0.05) * 1.0);
        ga.population.add(vals(&[], 0.00005));
        ga.population.add(vals(&[], 1.0));
        ga.generations = 10;
        let d = ga.population.diversity();
        assert_eq!(ga.adaptive_mutate_rate(), 0.05 + (0.30f32 - 0.05) * (1.0 - d));
        ga.generations = 0;
        assert!(ga.adaptive_mutate_rate() >= 0.15);
        ga.set_adaptive_mutation(false);
        assert_eq!(ga.adaptive_mutate_rate(), MUTATE_RATE);
    }

    #[test]
    fn add_to_population_breeds_when_full() {
        let mut rng = XorShiftRandom::new(11);
        let mut ga: Ga<BotGaValues> = Ga::new(4);
        for i in 0..3 {
            ga.add_to_population(vals(&[i as f32 + 1.0, 1.0, 2.0], i as f32), &mut rng);
        }
        assert!(!ga.can_pick());
        ga.add_to_population(vals(&[9.0, 1.0, 2.0], 0.5), &mut rng);
        assert_eq!(ga.generations(), 1);
        assert_eq!(ga.new_population().len(), 4);
        // Both elite slots are copies of the best (fitness 2: [3, 1, 2]).
        assert_eq!(ga.new_population().individuals()[0], vals(&[3.0, 1.0, 2.0], 2.0));
        assert_eq!(ga.new_population().individuals()[1], vals(&[3.0, 1.0, 2.0], 2.0));
        assert_eq!(ga.best().unwrap().fitness(), 2.0);
        assert_eq!(ga.population().individuals(), &[vals(&[3.0, 1.0, 2.0], 2.0)]);
        assert_eq!(ga.prev_avg_fitness(), 3.5 / 4.0);
        assert!(ga.can_pick());
        assert_eq!(ga.pick().unwrap().len(), 3);
        assert_eq!(ga.new_population().len(), 3);
        ga.free_local_memory();
        assert!(ga.best().is_none() && ga.generations() == 0 && !ga.can_pick());
    }

    #[test]
    fn odd_sizes_overshoot_by_one() {
        let mut rng = XorShiftRandom::new(3);
        let mut ga: Ga<IntGaValues> = Ga::new(5);
        for i in 0..5 {
            let mut v = IntGaValues::new(i);
            v.set_fitness(1.0);
            ga.add_to_population(v, &mut rng);
        }
        assert_eq!(ga.new_population().len(), 6);
    }

    #[test]
    fn save_load_round_trip() {
        let mut rng = XorShiftRandom::new(5);
        let mut ga: Ga<BotGaValues> = Ga::new(3);
        for i in 0..4 {
            ga.add_to_population(vals(&[i as f32, 0.5], i as f32 * 0.25), &mut rng);
        }
        let mut out = Vec::new();
        ga.save(&mut out);
        assert_eq!(&out[..7], b"t-2-s-3");
        let mut back: Ga<BotGaValues> = Ga::new(3);
        let mut r = Reader::new(&out);
        back.load(&mut r, 2);
        assert_eq!(r.remaining(), 0);
        assert_eq!(back.population(), ga.population());
        assert_eq!(back.new_population(), ga.new_population());
        let mut again = Vec::new();
        back.save(&mut again);
        assert_eq!(again, out);

        // A different maximum size doesn't load and leaves the populations alone.
        let mut other: Ga<BotGaValues> = Ga::new(4);
        other.population.add(vals(&[1.0], 1.0));
        other.load(&mut Reader::new(&out), 2);
        assert_eq!(other.population().len(), 1);
    }

    #[test]
    fn mismatched_chromosomes_keep_empty_individuals() {
        let mut ga: Ga<BotGaValues> = Ga::new(2);
        ga.new_population.add(vals(&[1.0, 2.0], 1.0));
        ga.new_population.add(vals(&[3.0, 4.0], 1.0));
        let mut out = Vec::new();
        ga.save(&mut out);
        let mut back: Ga<BotGaValues> = Ga::new(2);
        back.load(&mut Reader::new(&out), 3);
        // Each individual's header mismatch consumes 32 bytes; the rest is read as garbage
        // headers until the file runs out, as upstream does.
        assert!(back.new_population().individuals().iter().all(|i| i.is_empty()));
    }

    #[test]
    fn file_names() {
        assert_eq!(rce_file_name(SaveType::Bot, 3, "fwp"), "3pfwp.rce");
        assert_eq!(rce_file_name(SaveType::Team, 1, "x"), "1tx.rce");
    }
}
