//! RCBot's feed-forward network (`nn.h`, `bot_nn.cpp`): `NNLayer`, `NN`, `NNGATrained` (weights
//! bred by a [`Ga`] instead of back-propagation), and the combat network's input and output
//! helpers (`CCombatNNInputs`, `CCombatNNOutputs`).
//!
//! Upstream builds none of these at the moment (the bot's aiming network is commented out), so
//! this is the library the bot core can use, ported as it stands.

use super::ga::{Ga, GaValue, Individual};
use super::header::{GenericHeader, LearnType};
use super::io::{Reader, WriteLe};
use super::perceptron::Perceptron;
use super::values::BotGaValues;
use crate::Random;

/// `NN::g_learnRate`: the learn rate of every network neuron.
pub const NN_LEARN_RATE: GaValue = 0.9;

/// `CNNTrainSet`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrainSet {
    /// `inputs`.
    pub inputs: Vec<GaValue>,
    /// `outputs`: what the network should answer.
    pub outputs: Vec<GaValue>,
}

/// `NNLayer`.
#[derive(Clone, Debug, PartialEq)]
pub struct NnLayer {
    neurons: Vec<Perceptron>,
}

impl NnLayer {
    /// `NNLayer(iNumNeurons, iNumInputs)`.
    pub fn new(num_neurons: i32, num_inputs: i32, rng: &mut dyn Random) -> NnLayer {
        NnLayer { neurons: (0..num_neurons).map(|_| Perceptron::new(num_inputs, NN_LEARN_RATE, rng)).collect() }
    }

    /// `NNLayer(FILE*)`.
    pub fn from_reader(r: &mut Reader) -> NnLayer {
        let mut layer = NnLayer { neurons: Vec::new() };
        layer.load(r);
        layer
    }

    /// `numNeurons`.
    pub fn num_neurons(&self) -> usize {
        self.neurons.len()
    }

    /// `getNeuron`.
    pub fn neuron(&self, index: usize) -> &Perceptron {
        &self.neurons[index]
    }

    /// `getNeuron`, writable.
    pub fn neuron_mut(&mut self, index: usize) -> &mut Perceptron {
        &mut self.neurons[index]
    }

    /// `NNLayer::save`: `unsigned` count, header (`LEARNTYPE_NN_LAYER`, count), the neurons.
    pub fn save(&self, out: &mut Vec<u8>) {
        out.put_u32(self.neurons.len() as u32);
        GenericHeader::new(LearnType::NnLayer, self.neurons.len() as i32).write(out);
        for n in &self.neurons {
            n.save(out);
        }
    }

    /// `NNLayer::load`: adds the neurons the file holds if its header agrees with its count.
    pub fn load(&mut self, r: &mut Reader) {
        let Some(count) = r.read_u32() else { return };
        if GenericHeader::read(r, &GenericHeader::new(LearnType::NnLayer, count as i32)) {
            for _ in 0..count {
                if r.eof() {
                    // Upstream keeps constructing empty neurons until the count; stop instead of
                    // allocating a damaged count.
                    break;
                }
                self.neurons.push(Perceptron::from_reader(r));
            }
        }
    }
}

/// `NN`: layers of perceptrons, each fed the previous layer's outputs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Nn {
    layers: Vec<NnLayer>,
    num_inputs: i32,
}

impl Nn {
    /// `NN(iNumHiddenLayers, iNumInputs, iNumNeuronsPerHiddenLayer, iNumOutputs)`: an input
    /// layer of `per_hidden` neurons, `hidden` more such layers, and the output layer.
    pub fn new(hidden: i32, num_inputs: i32, per_hidden: i32, num_outputs: i32, rng: &mut dyn Random) -> Nn {
        let mut layers = vec![NnLayer::new(per_hidden, num_inputs, rng)];
        for _ in 0..hidden {
            layers.push(NnLayer::new(per_hidden, per_hidden, rng));
        }
        layers.push(NnLayer::new(num_outputs, per_hidden, rng));
        Nn { layers, num_inputs }
    }

    /// The layers, input side first.
    pub fn layers(&self) -> &[NnLayer] {
        &self.layers
    }

    /// `m_iNumInputs`.
    pub fn num_inputs(&self) -> i32 {
        self.num_inputs
    }

    /// `setWeights`: fills every neuron's weights in order until `weights` runs out.
    pub fn set_weights(&mut self, weights: &[GaValue]) {
        let mut it = weights.iter();
        for l in &mut self.layers {
            for n in &mut l.neurons {
                for k in 0..n.num_weights() {
                    if let Some(&w) = it.next() {
                        n.set_weight(k, w);
                    }
                }
            }
        }
    }

    /// `getWeights`: every neuron's weights in order.
    pub fn weights(&self) -> Vec<GaValue> {
        self.layers.iter().flat_map(|l| &l.neurons).flat_map(|n| n.weights().iter().copied()).collect()
    }

    /// `execute`: runs the layers in turn and returns the last layer's outputs.
    pub fn execute(&mut self, inputs: &[GaValue]) -> Vec<GaValue> {
        let mut current = inputs.to_vec();
        for l in &mut self.layers {
            let mut outputs = Vec::with_capacity(l.neurons.len());
            for n in &mut l.neurons {
                n.input(&current);
                n.execute();
                outputs.push(n.output());
            }
            current = outputs;
        }
        current
    }

    /// `randomize`.
    pub fn randomize(&mut self, rng: &mut dyn Random) {
        for l in &mut self.layers {
            for n in &mut l.neurons {
                n.randomize(rng);
            }
        }
    }

    /// `getOutputs`: the last layer's outputs.
    pub fn outputs(&self) -> Vec<GaValue> {
        self.layers.last().map_or_else(Vec::new, |l| l.neurons.iter().map(Perceptron::output).collect())
    }

    /// `trainOutputs`. Upstream hands the wanted outputs to the neurons in layer order, so it
    /// trains the first layer's neurons, not the output layer's (§5); kept.
    pub fn train_outputs(&mut self, wanted: &[GaValue]) {
        let mut it = wanted.iter();
        for l in &mut self.layers {
            for n in &mut l.neurons {
                if let Some(&w) = it.next() {
                    n.train(w);
                }
            }
        }
    }

    /// `NN::save`: `unsigned` layer count, header (`LEARNTYPE_NN`, count), the layers.
    pub fn save(&self, out: &mut Vec<u8>) {
        out.put_u32(self.layers.len() as u32);
        GenericHeader::new(LearnType::Nn, self.layers.len() as i32).write(out);
        for l in &self.layers {
            l.save(out);
        }
    }

    /// `NN::load`: adds the layers the file holds if its header agrees with its count.
    pub fn load(&mut self, r: &mut Reader) {
        if r.eof() {
            return;
        }
        let Some(count) = r.read_u32() else { return };
        if GenericHeader::read(r, &GenericHeader::new(LearnType::Nn, count as i32)) {
            for _ in 0..count {
                if r.eof() {
                    break;
                }
                self.layers.push(NnLayer::from_reader(r));
            }
        }
    }
}

/// `NNGATrained`: a network whose weight vectors are individuals of a GA of 12.
#[derive(Clone, Debug)]
pub struct NnGaTrained {
    /// The network.
    pub nn: Nn,
    ga: Ga<BotGaValues>,
    individual: BotGaValues,
}

impl NnGaTrained {
    /// `NNGATrained(...)`: as [`Nn::new`], with `CGA(12)` and an empty individual.
    pub fn new(hidden: i32, num_inputs: i32, per_hidden: i32, num_outputs: i32, rng: &mut dyn Random) -> NnGaTrained {
        NnGaTrained { nn: Nn::new(hidden, num_inputs, per_hidden, num_outputs, rng), ga: Ga::new(12), individual: BotGaValues::new() }
    }

    /// The GA.
    pub fn ga(&self) -> &Ga<BotGaValues> {
        &self.ga
    }

    /// `NNGATrained::train`: takes the next bred weights (or random ones when there are none),
    /// scores them by `1 / mean squared error` over the sets (fitness 0 when the error is
    /// within `FLT_EPSILON`), and gives a copy to the GA.
    pub fn train(&mut self, sets: &[TrainSet], rng: &mut dyn Random) {
        if let Some(ind) = self.ga.pick() {
            self.individual = ind;
            self.individual.set_fitness(0.0);
            let w = self.individual.values().to_vec();
            self.nn.set_weights(&w);
        } else {
            self.individual.clear();
            self.individual.set_fitness(0.0);
            self.nn.randomize(rng);
            self.individual.set_values(self.nn.weights());
        }
        let mut total: GaValue = 0.0;
        let mut num: GaValue = 0.0;
        for set in sets {
            let outputs = self.nn.execute(&set.inputs);
            for (o, want) in outputs.iter().zip(&set.outputs) {
                let e = o - want;
                total += e * e;
                num += 1.0;
            }
        }
        if num > 0.0 {
            total /= num;
            if total > GaValue::EPSILON {
                self.individual.set_fitness(1.0 / total);
            }
        }
        self.ga.add_to_population(self.individual.clone(), rng);
    }
}

/// `COMBAT_NN_NUM_INPUTS`.
pub const COMBAT_NN_NUM_INPUTS: usize = 8;
/// `COMBAT_NN_NUM_OUTPUTS`.
pub const COMBAT_NN_NUM_OUTPUTS: usize = 4;
/// `COMBAT_MAX_DISTANCE`.
pub const COMBAT_MAX_DISTANCE: f32 = 2048.0;
/// `COMBAT_MAX_HEALTH`.
pub const COMBAT_MAX_HEALTH: f32 = 100.0;
/// `COMBAT_MAX_TEAMMATES`.
pub const COMBAT_MAX_TEAMMATES: f32 = 5.0;

/// `CCombatNNInputs`: the combat network's inputs, normalised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatNnInputs {
    /// `enemyDistance`, 0 to 1.
    pub enemy_distance: GaValue,
    /// `enemyHealth`, 0 to 1.
    pub enemy_health: GaValue,
    /// `selfHealth`, 0 to 1.
    pub self_health: GaValue,
    /// `selfAmmo`, 0 to 1.
    pub self_ammo: GaValue,
    /// `enemyVisible`, 0 or 1.
    pub enemy_visible: GaValue,
    /// `coverNearby`, 0 or 1.
    pub cover_nearby: GaValue,
    /// `heightAdvantage`, -1 to 1 (positive: the bot is higher).
    pub height_advantage: GaValue,
    /// `teammatesNearby`, 0 to 1.
    pub teammates_nearby: GaValue,
}

impl Default for CombatNnInputs {
    fn default() -> Self {
        CombatNnInputs {
            enemy_distance: 0.5,
            enemy_health: 1.0,
            self_health: 1.0,
            self_ammo: 1.0,
            enemy_visible: 0.0,
            cover_nearby: 0.0,
            height_advantage: 0.0,
            teammates_nearby: 0.0,
        }
    }
}

impl CombatNnInputs {
    /// `setEnemyDistance`.
    pub fn set_enemy_distance(&mut self, distance: f32) {
        self.enemy_distance = (distance / COMBAT_MAX_DISTANCE).clamp(0.0, 1.0);
    }

    /// `setEnemyHealth`.
    pub fn set_enemy_health(&mut self, health: f32, max_health: f32) {
        self.enemy_health = if max_health > 0.0 { (health / max_health).clamp(0.0, 1.0) } else { 0.0 };
    }

    /// `setSelfHealth`.
    pub fn set_self_health(&mut self, health: f32, max_health: f32) {
        self.self_health = if max_health > 0.0 { (health / max_health).clamp(0.0, 1.0) } else { 0.0 };
    }

    /// `setSelfAmmo`.
    pub fn set_self_ammo(&mut self, current: i32, max: i32) {
        self.self_ammo = if max > 0 { (current as f32 / max as f32).clamp(0.0, 1.0) } else { 0.0 };
    }

    /// `setEnemyVisible`.
    pub fn set_enemy_visible(&mut self, visible: bool) {
        self.enemy_visible = if visible { 1.0 } else { 0.0 };
    }

    /// `setCoverNearby`.
    pub fn set_cover_nearby(&mut self, cover: bool) {
        self.cover_nearby = if cover { 1.0 } else { 0.0 };
    }

    /// `setHeightAdvantage`: the height difference over 200 units.
    pub fn set_height_advantage(&mut self, bot_z: f32, enemy_z: f32) {
        self.height_advantage = ((bot_z - enemy_z) / 200.0).clamp(-1.0, 1.0);
    }

    /// `setTeammatesNearby`.
    pub fn set_teammates_nearby(&mut self, count: i32) {
        self.teammates_nearby = (count as f32 / COMBAT_MAX_TEAMMATES).clamp(0.0, 1.0);
    }

    /// `toVector`.
    pub fn to_vec(&self) -> Vec<GaValue> {
        vec![
            self.enemy_distance,
            self.enemy_health,
            self.self_health,
            self.self_ammo,
            self.enemy_visible,
            self.cover_nearby,
            self.height_advantage,
            self.teammates_nearby,
        ]
    }
}

/// `eCombatAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatAction {
    /// `COMBAT_ACTION_ATTACK`.
    Attack = 0,
    /// `COMBAT_ACTION_RETREAT`.
    Retreat = 1,
    /// `COMBAT_ACTION_STRAFE`.
    Strafe = 2,
    /// `COMBAT_ACTION_JUMP`.
    Jump = 3,
}

/// `CCombatNNOutputs`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CombatNnOutputs {
    /// `attackWeight`.
    pub attack: GaValue,
    /// `retreatWeight`.
    pub retreat: GaValue,
    /// `strafeWeight`.
    pub strafe: GaValue,
    /// `jumpWeight`.
    pub jump: GaValue,
}

impl CombatNnOutputs {
    /// `fromVector`: ignored unless there are exactly four outputs.
    pub fn from_slice(&mut self, outputs: &[GaValue]) {
        if let [attack, retreat, strafe, jump] = *outputs {
            *self = CombatNnOutputs { attack, retreat, strafe, jump };
        }
    }

    /// `getBestAction`: the highest weight, earlier actions winning ties.
    pub fn best_action(&self) -> CombatAction {
        let mut best = (self.attack, CombatAction::Attack);
        for (w, a) in [(self.retreat, CombatAction::Retreat), (self.strafe, CombatAction::Strafe), (self.jump, CombatAction::Jump)] {
            if w > best.0 {
                best = (w, a);
            }
        }
        best.1
    }

    /// `shouldAttack` (upstream's default threshold is 0.5).
    pub fn should_attack(&self, threshold: GaValue) -> bool {
        self.attack > threshold
    }

    /// `shouldRetreat` (default 0.5).
    pub fn should_retreat(&self, threshold: GaValue) -> bool {
        self.retreat > threshold
    }

    /// `shouldStrafe` (default 0.3).
    pub fn should_strafe(&self, threshold: GaValue) -> bool {
        self.strafe > threshold
    }

    /// `shouldJump` (default 0.4).
    pub fn should_jump(&self, threshold: GaValue) -> bool {
        self.jump > threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    #[test]
    fn shape_and_weights() {
        let mut rng = XorShiftRandom::new(1);
        let mut nn = Nn::new(1, 3, 2, 2, &mut rng);
        assert_eq!(nn.layers().iter().map(NnLayer::num_neurons).collect::<Vec<_>>(), [2, 2, 2]);
        assert_eq!(nn.layers()[0].neuron(0).num_weights(), 4);
        let w = nn.weights();
        assert_eq!(w.len(), 2 * 4 + 2 * 3 + 2 * 3);
        let mut doubled: Vec<f32> = w.iter().map(|x| x * 2.0).collect();
        doubled.truncate(5);
        nn.set_weights(&doubled);
        assert_eq!(&nn.weights()[..5], &doubled[..]);
        assert_eq!(&nn.weights()[5..], &w[5..]);
    }

    #[test]
    fn execute_feeds_forward() {
        let mut rng = XorShiftRandom::new(1);
        let mut nn = Nn::new(0, 1, 1, 1, &mut rng);
        nn.set_weights(&[0.0, 1.0, 0.5, -1.0]);
        let out = nn.execute(&[2.0]);
        let hidden = super::super::perceptron::sigmoid(2.0);
        assert_eq!(out, [super::super::perceptron::sigmoid(0.5 - hidden)]);
        assert_eq!(nn.outputs(), out);
        // trainOutputs trains the first layer.
        let before = nn.layers()[1].neuron(0).weights().to_vec();
        nn.train_outputs(&[1.0]);
        assert!(nn.layers()[0].neuron(0).trained());
        assert_eq!(nn.layers()[1].neuron(0).weights(), &before[..]);
    }

    #[test]
    fn save_load_round_trip() {
        let mut rng = XorShiftRandom::new(4);
        let mut nn = Nn::new(1, 2, 2, 1, &mut rng);
        nn.execute(&[0.3, 0.7]);
        let mut out = Vec::new();
        nn.save(&mut out);
        let mut back = Nn::default();
        let mut r = Reader::new(&out);
        back.load(&mut r);
        assert_eq!(r.remaining(), 0);
        assert_eq!(back.weights(), nn.weights());
        assert_eq!(back.layers(), nn.layers());
        let mut again = Vec::new();
        back.save(&mut again);
        assert_eq!(again, out);
    }

    #[test]
    fn ga_training_scores_by_error() {
        let mut rng = XorShiftRandom::new(8);
        let mut nn = NnGaTrained::new(0, 1, 2, 1, &mut rng);
        let sets = [TrainSet { inputs: vec![1.0], outputs: vec![1.0] }, TrainSet { inputs: vec![0.0], outputs: vec![0.0] }];
        for _ in 0..12 {
            nn.train(&sets, &mut rng);
        }
        assert_eq!(nn.ga().generations(), 1);
        assert!(nn.ga().can_pick());
        let best = nn.ga().best().unwrap();
        assert!(best.fitness() > 1.0, "fitness is 1 / mean squared error");
        assert_eq!(best.len(), nn.nn.weights().len());
        nn.train(&sets, &mut rng);
        assert_eq!(nn.ga().new_population().len(), 11);
    }

    #[test]
    fn combat_inputs_and_outputs() {
        let mut i = CombatNnInputs::default();
        i.set_enemy_distance(4096.0);
        i.set_enemy_health(50.0, COMBAT_MAX_HEALTH);
        i.set_self_health(10.0, 0.0);
        i.set_self_ammo(5, 10);
        i.set_enemy_visible(true);
        i.set_cover_nearby(false);
        i.set_height_advantage(0.0, 300.0);
        i.set_teammates_nearby(2);
        assert_eq!(i.to_vec(), [1.0, 0.5, 0.0, 0.5, 1.0, 0.0, -1.0, 0.4]);
        assert_eq!(CombatNnInputs::default().to_vec().len(), COMBAT_NN_NUM_INPUTS);

        let mut o = CombatNnOutputs::default();
        o.from_slice(&[0.2, 0.6, 0.6, 0.1]);
        assert_eq!(o.best_action(), CombatAction::Retreat);
        o.from_slice(&[1.0]);
        assert_eq!(o.retreat, 0.6);
        assert!(o.should_retreat(0.5) && !o.should_attack(0.5) && o.should_strafe(0.3) && !o.should_jump(0.4));
        assert_eq!(CombatNnOutputs::default().best_action(), CombatAction::Attack);
    }
}
