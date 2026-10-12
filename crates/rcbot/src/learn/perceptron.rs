//! `CPerceptron` (`perceptron.{h,cpp}`): one sigmoid neuron with a bias weight. The bot's
//! decisions use them (TS stunts, jump/duck/strafe, running for cover, ...), mostly with weights
//! set from a GA individual rather than trained.

use super::ga::GaValue;
use super::header::{GenericHeader, LearnType};
use super::io::{Reader, WriteLe};
use super::values::BotGaValues;
use crate::Random;

/// `CPerceptron::m_fDefaultLearnRate`.
pub const DEFAULT_LEARN_RATE: GaValue = 0.5;
/// `CPerceptron::m_fDefaultBias`.
pub const DEFAULT_BIAS: GaValue = 1.0;

/// `CSigmoidTransfer::transfer`: `1 / (1 + exp(-x))`, the only `ITransfer` upstream has.
pub fn sigmoid(net_input: GaValue) -> GaValue {
    1.0 / (1.0 + (-net_input).exp())
}

/// `CPerceptron`. `weights[0]` is the bias weight, then one weight per input.
#[derive(Clone, Debug, PartialEq)]
pub struct Perceptron {
    num_inputs: i32,
    bias: GaValue,
    learn_rate: GaValue,
    output: GaValue,
    trained: bool,
    weights: Vec<GaValue>,
    inputs: Vec<GaValue>,
}

/// A fresh weight: `RANDOM_FLOAT(0, 0.6f) - 0.3f`.
fn random_weight(rng: &mut dyn Random) -> GaValue {
    rng.random_float(0.0, 0.6) - 0.3
}

impl Perceptron {
    /// `CPerceptron(iInputs, nullptr, fLearnRate)`: `num_inputs + 1` random weights; a learn rate
    /// of 0 means [`DEFAULT_LEARN_RATE`].
    pub fn new(num_inputs: i32, learn_rate: GaValue, rng: &mut dyn Random) -> Perceptron {
        let mut weights = Vec::with_capacity(num_inputs.max(0) as usize + 1);
        weights.push(random_weight(rng));
        for _ in 0..num_inputs {
            weights.push(random_weight(rng));
        }
        Perceptron {
            num_inputs,
            bias: DEFAULT_BIAS,
            learn_rate: if learn_rate == 0.0 { DEFAULT_LEARN_RATE } else { learn_rate },
            output: 0.0,
            trained: false,
            weights,
            inputs: Vec::new(),
        }
    }

    /// `CPerceptron(FILE*)`: a perceptron with nothing in it, then [`Perceptron::load`].
    ///
    /// Upstream checks the header against the 0 inputs the empty perceptron has, so a saved
    /// perceptron with inputs never loads this way and the rest of the file (an `NN`'s) is read
    /// from the wrong place (§5). Here the header only has to be a perceptron header, and its
    /// input count must match the one stored after it (design §4).
    pub fn from_reader(r: &mut Reader) -> Perceptron {
        let mut p =
            Perceptron { num_inputs: 0, bias: 0.0, learn_rate: 0.0, output: 0.0, trained: false, weights: Vec::new(), inputs: Vec::new() };
        if r.eof() {
            return p;
        }
        let header = GenericHeader::read_any(r);
        if let Some((t, n)) = header.parse()
            && t == LearnType::Perceptron as i32
            && r.clone().read_i32() == Some(n)
        {
            p.load_body(r);
        }
        p
    }

    /// `m_iInputs`.
    pub fn num_inputs(&self) -> i32 {
        self.num_inputs
    }

    /// `numWeights`.
    pub fn num_weights(&self) -> usize {
        self.weights.len()
    }

    /// `getWeight` (0 out of range).
    pub fn weight(&self, index: usize) -> GaValue {
        self.weights.get(index).copied().unwrap_or(0.0)
    }

    /// `setWeight` (ignored out of range).
    pub fn set_weight(&mut self, index: usize, value: GaValue) {
        if let Some(w) = self.weights.get_mut(index) {
            *w = value;
        }
    }

    /// The weights, bias weight first.
    pub fn weights(&self) -> &[GaValue] {
        &self.weights
    }

    /// The inputs of the last [`Perceptron::input`].
    pub fn inputs(&self) -> &[GaValue] {
        &self.inputs
    }

    /// `m_Bias`.
    pub fn bias(&self) -> GaValue {
        self.bias
    }

    /// `m_LearnRate`.
    pub fn learn_rate(&self) -> GaValue {
        self.learn_rate
    }

    /// `setWeights(weights)`: replaces all weights (bias weight first).
    pub fn set_weights(&mut self, weights: &[GaValue]) {
        self.weights = weights.to_vec();
    }

    /// `setWeights(weights, iFrom, iNum)`: replaces all weights with `weights[from..from + num]`
    /// (no bias weight is added, unlike [`Perceptron::set_weights_from_values`]); an invalid
    /// range changes nothing.
    pub fn set_weights_range(&mut self, weights: &[GaValue], from: i32, num: i32) {
        if from < 0 || num < 0 || from as usize + num as usize > weights.len() {
            return;
        }
        self.weights = weights[from as usize..(from + num) as usize].to_vec();
    }

    /// `setWeights(const CBotGAValues*, iFrom, iNum)`: the bias weight becomes the bias itself
    /// (`m_Bias`, 1), then `num` values from `from`. Values past the end are 0 here (an assert
    /// upstream).
    pub fn set_weights_from_values(&mut self, values: &BotGaValues, from: usize, num: usize) {
        self.weights.clear();
        self.weights.push(self.bias);
        self.weights.extend((from..from + num).map(|i| values.get(i)));
    }

    /// `randomize`: a new random value for every weight.
    pub fn randomize(&mut self, rng: &mut dyn Random) {
        for w in &mut self.weights {
            *w = random_weight(rng);
        }
    }

    /// `input`.
    pub fn input(&mut self, inputs: &[GaValue]) {
        self.inputs = inputs.to_vec();
    }

    /// `execute`: the sigmoid of the bias weight plus the inputs' weighted sum. With a weight
    /// count other than inputs + 1 it returns 0 and leaves the previous output in place, as
    /// upstream does.
    pub fn execute(&mut self) -> GaValue {
        if self.weights.len() != self.inputs.len() + 1 {
            return 0.0;
        }
        // std::inner_product from 0.0f, then added to the bias weight.
        let mut sum: GaValue = 0.0;
        for (i, w) in self.inputs.iter().zip(&self.weights[1..]) {
            sum += i * w;
        }
        self.output = sigmoid(self.weights[0] + sum);
        self.output
    }

    /// `fired`: output at least 0.5.
    pub fn fired(&self) -> bool {
        self.output >= 0.5
    }

    /// `getOutput`.
    pub fn output(&self) -> GaValue {
        self.output
    }

    /// `trained`.
    pub fn trained(&self) -> bool {
        self.trained
    }

    /// `setTrained`.
    pub fn set_trained(&mut self) {
        self.trained = true;
    }

    /// `train`: the delta rule on the last output (no sigmoid derivative). Needs exactly
    /// `num_inputs` inputs and one weight more.
    pub fn train(&mut self, expected: GaValue) {
        if self.inputs.len() != self.num_inputs.max(0) as usize || self.num_inputs < 0 {
            return;
        }
        if self.weights.len() != self.inputs.len() + 1 {
            return;
        }
        self.trained = true;
        let error = expected - self.output;
        let delta = self.learn_rate * error;
        self.weights[0] += delta * self.bias;
        for (w, i) in self.weights[1..].iter_mut().zip(&self.inputs) {
            *w += delta * i;
        }
    }

    /// `save`: header (`LEARNTYPE_PERCEPTRON`, inputs), `int` inputs, bias, learn rate,
    /// `unsigned` input count and the last inputs, `unsigned` weight count and the weights, the
    /// output, and `trained` as one byte.
    pub fn save(&self, out: &mut Vec<u8>) {
        GenericHeader::new(LearnType::Perceptron, self.num_inputs).write(out);
        out.put_i32(self.num_inputs);
        out.put_f32(self.bias);
        out.put_f32(self.learn_rate);
        out.put_u32(self.inputs.len() as u32);
        out.put_f32s(&self.inputs);
        out.put_u32(self.weights.len() as u32);
        out.put_f32s(&self.weights);
        out.put_f32(self.output);
        out.push(u8::from(self.trained));
    }

    /// `load`: the header must name this perceptron's input count; on a mismatch only the
    /// header is consumed and nothing changes. A short read stops with what was read so far.
    pub fn load(&mut self, r: &mut Reader) {
        if r.eof() {
            return;
        }
        if !GenericHeader::read(r, &GenericHeader::new(LearnType::Perceptron, self.num_inputs)) {
            return;
        }
        self.load_body(r);
    }

    fn load_body(&mut self, r: &mut Reader) {
        let Some(n) = r.read_i32() else { return };
        self.num_inputs = n;
        let Some(b) = r.read_f32() else { return };
        self.bias = b;
        let Some(l) = r.read_f32() else { return };
        self.learn_rate = l;
        let Some(count) = r.read_u32() else { return };
        if !r.read_f32s(count, &mut self.inputs) {
            return;
        }
        let Some(count) = r.read_u32() else { return };
        if !r.read_f32s(count, &mut self.weights) {
            return;
        }
        let Some(o) = r.read_f32() else { return };
        self.output = o;
        if let Some(t) = r.read_bool() {
            self.trained = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    #[test]
    fn construction_draws_inputs_plus_one_weights() {
        let mut rng = XorShiftRandom::new(9);
        let p = Perceptron::new(4, 0.0, &mut rng);
        assert_eq!(p.num_weights(), 5);
        assert!(p.weights().iter().all(|w| (-0.3..0.3).contains(w)));
        assert_eq!(p.learn_rate(), DEFAULT_LEARN_RATE);
        assert_eq!(Perceptron::new(1, 0.9, &mut rng).learn_rate(), 0.9);
        let mut again = XorShiftRandom::new(9);
        assert_eq!(Perceptron::new(4, 0.0, &mut again), p);
    }

    #[test]
    fn execute_and_fire() {
        let mut p = Perceptron::new(2, 0.0, &mut XorShiftRandom::new(1));
        p.set_weights(&[0.5, 1.0, -2.0]);
        p.input(&[1.0, 0.25]);
        let out = p.execute();
        assert_eq!(out, sigmoid(0.5 + (1.0 + 0.25 * -2.0)));
        assert!(p.fired());
        // A size mismatch returns 0 and keeps the old output.
        p.input(&[1.0]);
        assert_eq!(p.execute(), 0.0);
        assert_eq!(p.output(), out);
    }

    #[test]
    fn weights_from_ga_values_put_the_bias_first() {
        let mut p = Perceptron::new(5, 0.0, &mut XorShiftRandom::new(1));
        let v = BotGaValues::from_values((0..8).map(|i| i as f32).collect());
        p.set_weights_from_values(&v, 3, 5);
        assert_eq!(p.weights(), &[1.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        p.set_weights_range(&[9.0, 8.0, 7.0], 1, 2);
        assert_eq!(p.weights(), &[8.0, 7.0]);
        p.set_weights_range(&[9.0], 1, 2);
        assert_eq!(p.weights(), &[8.0, 7.0]);
    }

    #[test]
    fn delta_rule() {
        let mut p = Perceptron::new(2, 0.5, &mut XorShiftRandom::new(1));
        p.set_weights(&[0.0, 0.0, 0.0]);
        p.input(&[1.0, -1.0]);
        p.execute();
        assert_eq!(p.output(), 0.5);
        p.train(1.0);
        assert!(p.trained());
        assert_eq!(p.weights(), &[0.25, 0.25, -0.25]);
        // Wrong input count: no training.
        p.input(&[1.0]);
        p.train(0.0);
        assert_eq!(p.weights(), &[0.25, 0.25, -0.25]);
    }

    #[test]
    fn save_load() {
        let mut p = Perceptron::new(3, 0.0, &mut XorShiftRandom::new(2));
        p.input(&[0.1, 0.2, 0.3]);
        p.execute();
        p.train(1.0);
        let mut out = Vec::new();
        p.save(&mut out);
        assert_eq!(&out[..7], b"t-1-s-3");
        assert_eq!(out.len(), 32 + 4 + 4 + 4 + 4 + 12 + 4 + 16 + 4 + 1);
        let mut q = Perceptron::new(3, 0.0, &mut XorShiftRandom::new(5));
        q.load(&mut Reader::new(&out));
        assert_eq!(q, p);
        let mut again = Vec::new();
        q.save(&mut again);
        assert_eq!(again, out);

        let mut wrong = Perceptron::new(4, 0.0, &mut XorShiftRandom::new(5));
        let before = wrong.clone();
        let mut r = Reader::new(&out);
        wrong.load(&mut r);
        assert_eq!(wrong, before);
        assert_eq!(r.remaining(), out.len() - 32);

        let mut r = Reader::new(&out);
        assert_eq!(Perceptron::from_reader(&mut r), p);
        assert_eq!(r.remaining(), 0);
    }
}
