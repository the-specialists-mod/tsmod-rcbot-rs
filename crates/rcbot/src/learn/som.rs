//! `CSom` and `CSomNeuron` (`som.{h,cpp}`): a self-organising map. Upstream builds none (it is
//! marked experimental); ported with the rest of the learning code.
//!
//! Upstream's learn rate, `CSom::m_fLearnRate`, is a static shared by every map and never reset,
//! so each map's training slows every later map's. Each map has its own here (design §4); as
//! nothing creates two maps upstream, no behaviour changes.

use super::ga::GaValue;
use crate::Random;

/// `CSom::m_fLearnRate`'s initial value.
pub const INITIAL_LEARN_RATE: GaValue = 1.0;

/// `CSomNeuron`: a weight vector at a grid position.
#[derive(Clone, Debug, PartialEq)]
pub struct SomNeuron {
    weights: Vec<GaValue>,
    x: GaValue,
    y: GaValue,
}

impl SomNeuron {
    /// `CSomNeuron(iInp, iX, iY)`: `RANDOM_FLOAT(0, 1)` weights.
    pub fn new(num_inputs: i32, x: GaValue, y: GaValue, rng: &mut dyn Random) -> SomNeuron {
        SomNeuron { weights: (0..num_inputs).map(|_| rng.random_float(0.0, 1.0)).collect(), x, y }
    }

    /// `getX`.
    pub fn x(&self) -> GaValue {
        self.x
    }

    /// `getY`.
    pub fn y(&self) -> GaValue {
        self.y
    }

    /// `weights`.
    pub fn weights(&self) -> &[GaValue] {
        &self.weights
    }

    /// `update`: moves each weight `learn_rate * influence` of the way to the input.
    pub fn update(&mut self, inputs: &[GaValue], influence: GaValue, learn_rate: GaValue) {
        for (w, i) in self.weights.iter_mut().zip(inputs) {
            let change = i - *w;
            *w += change * learn_rate * influence;
        }
    }

    /// `distance`: squared Euclidean distance. Upstream indexes the weights by the input count;
    /// extra inputs are ignored here.
    pub fn distance(&self, inputs: &[GaValue]) -> GaValue {
        let mut dist: GaValue = 0.0;
        for (w, i) in self.weights.iter().zip(inputs) {
            let comp = w - i;
            dist += comp * comp;
        }
        dist
    }

    /// `neighbourDistance`: squared grid distance.
    pub fn neighbour_distance(&self, other: &SomNeuron) -> GaValue {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }

    /// `displayWeights`: `"%0.4f,"` per weight.
    pub fn display_weights(&self) -> String {
        self.weights.iter().map(|w| format!("{w:.4},")).collect()
    }
}

/// `CSom`.
#[derive(Clone, Debug, PartialEq)]
pub struct Som {
    neurons: Vec<SomNeuron>,
    width: i32,
    height: i32,
    neighbourhood: GaValue,
    epochs: u32,
    learn_rate: GaValue,
}

impl Som {
    /// `CSom(iW, iH, iIn)`: neurons row by row; the neighbourhood starts at half the width.
    pub fn new(width: i32, height: i32, num_inputs: i32, rng: &mut dyn Random) -> Som {
        let mut neurons = Vec::new();
        for i in 0..height {
            for j in 0..width {
                neurons.push(SomNeuron::new(num_inputs, j as GaValue, i as GaValue, rng));
            }
        }
        Som { neurons, width, height, neighbourhood: width as GaValue / 2.0, epochs: 0, learn_rate: INITIAL_LEARN_RATE }
    }

    /// The neurons, row by row.
    pub fn neurons(&self) -> &[SomNeuron] {
        &self.neurons
    }

    /// The grid size.
    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// `epochs`: inputs trained on so far.
    pub fn epochs(&self) -> u32 {
        self.epochs
    }

    /// `m_fLearnRate`.
    pub fn learn_rate(&self) -> GaValue {
        self.learn_rate
    }

    /// `getBMU`: the index of the closest neuron, the first of equals.
    pub fn bmu(&self, inputs: &[GaValue]) -> Option<usize> {
        let mut best = GaValue::MAX;
        let mut winner = None;
        for (i, n) in self.neurons.iter().enumerate() {
            let d = n.distance(inputs);
            if d < best {
                winner = Some(i);
                best = d;
            }
        }
        winner
    }

    /// `updateAround`: neurons within the neighbourhood of `bmu` move with influence
    /// `exp(-d / (2 n²))`, `d` being the squared grid distance and `n` the neighbourhood.
    pub fn update_around(&mut self, inputs: &[GaValue], bmu: usize) {
        let nsiz = self.neighbourhood * self.neighbourhood;
        let centre = self.neurons[bmu].clone();
        for n in &mut self.neurons {
            let dist = centre.neighbour_distance(n);
            if dist <= nsiz {
                n.update(inputs, (-dist / (2.0 * nsiz)).exp(), self.learn_rate);
            }
        }
    }

    /// `inputOne`: trains on one input; the neighbourhood and the learn rate shrink by a quarter.
    pub fn input_one(&mut self, inputs: &[GaValue]) -> Option<usize> {
        let winner = self.bmu(inputs);
        if let Some(w) = winner {
            self.update_around(inputs, w);
        }
        self.neighbourhood *= 0.75;
        self.learn_rate *= 0.75;
        self.epochs += 1;
        winner
    }

    /// `input(inputs)`: trains on one input chosen with `RANDOM_LONG`.
    pub fn input(&mut self, inputs: &[Vec<GaValue>], rng: &mut dyn Random) -> Option<usize> {
        if inputs.is_empty() {
            return None;
        }
        let i = rng.random_long(0, inputs.len() as i32 - 1) as usize;
        self.input_one(&inputs[i])
    }

    /// `input(inputs, epochs)`: `epochs` rounds of as many random picks as there are inputs.
    pub fn input_epochs(&mut self, inputs: &[Vec<GaValue>], epochs: i32, rng: &mut dyn Random) {
        if inputs.is_empty() || epochs <= 0 {
            return;
        }
        for _ in 0..epochs {
            for _ in 0..inputs.len() {
                self.input(inputs, rng);
            }
        }
    }

    /// `display`: every neuron's [`SomNeuron::display_weights`], concatenated.
    pub fn display(&self) -> String {
        self.neurons.iter().map(SomNeuron::display_weights).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    #[test]
    fn grid_layout() {
        let som = Som::new(3, 2, 2, &mut XorShiftRandom::new(1));
        let pos: Vec<_> = som.neurons().iter().map(|n| (n.x(), n.y())).collect();
        assert_eq!(pos, [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (0.0, 1.0), (1.0, 1.0), (2.0, 1.0)]);
        assert!(som.neurons().iter().all(|n| n.weights().iter().all(|w| (0.0..1.0).contains(w))));
    }

    #[test]
    fn training_pulls_the_winner_and_shrinks() {
        let mut som = Som::new(4, 1, 1, &mut XorShiftRandom::new(2));
        let target = [5.0];
        let w = som.bmu(&target).unwrap();
        let before = som.neurons()[w].weights()[0];
        assert_eq!(som.input_one(&target), Some(w));
        // The winner has influence 1 at learn rate 1: it lands on the input.
        assert_eq!(som.neurons()[w].weights()[0], before + (5.0 - before));
        assert_eq!(som.learn_rate(), 0.75);
        assert_eq!(som.epochs(), 1);
        som.input_epochs(&[vec![1.0], vec![2.0]], 2, &mut XorShiftRandom::new(3));
        assert_eq!(som.epochs(), 5);
        assert_eq!(som.display().matches(',').count(), 4);
    }

    #[test]
    fn neighbourhood_limits_updates() {
        let mut som = Som::new(4, 1, 1, &mut XorShiftRandom::new(2));
        // Neighbourhood 2: neurons up to 2 cells away move, the one 3 away doesn't.
        let far = som.neurons()[3].weights()[0];
        som.update_around(&[0.5], 0);
        assert_eq!(som.neurons()[3].weights()[0], far);
        assert_eq!(som.neurons()[0].weights()[0], 0.5);
        assert_eq!(som.size(), (4, 1));
    }
}
