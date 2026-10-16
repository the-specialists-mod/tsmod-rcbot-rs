//! Running estimates for the port's bandit learners (port addition, design §4): a weighted mean
//! with its (decaying) weight, shrinkage toward a prior, and UCB1 scores. Deterministic: no
//! random draws, so a seeded run replays exactly.

/// One option's estimate: the weighted mean of its rewards and the total weight behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Arm {
    /// The weighted mean reward.
    pub value: f32,
    /// The weight (a count, decayed by [`Arm::decay`]).
    pub n: f32,
}

impl Arm {
    /// An arm with `value` worth `n` samples.
    pub fn new(value: f32, n: f32) -> Arm {
        Arm { value, n }
    }

    /// Adds a reward with `weight` (1 an ordinary sample).
    pub fn update(&mut self, reward: f32, weight: f32) {
        if weight <= 0.0 || !reward.is_finite() {
            return;
        }
        self.n += weight;
        self.value += (reward - self.value) * weight / self.n;
    }

    /// Forgets a little: the weight shrinks by `factor`, the mean stays.
    pub fn decay(&mut self, factor: f32) {
        self.n *= factor;
    }

    /// The mean pulled toward `prior` as if it had `m` samples of it: `(n v + m p) / (n + m)`.
    pub fn shrunk(&self, prior: f32, m: f32) -> f32 {
        if self.n + m <= 0.0 {
            return prior;
        }
        (self.n * self.value + m * prior) / (self.n + m)
    }
}

/// UCB1: `mean + c sqrt(ln(total + 1) / n)`; an option never tried scores infinity (it is tried
/// first).
pub fn ucb(mean: f32, n: f32, total: f32, c: f32) -> f32 {
    if n <= 1e-3 {
        return f32::INFINITY;
    }
    mean + c * ((total.max(0.0) + 1.0).ln() / n).sqrt()
}

/// The lower bound matching [`ucb`]: `mean - c sqrt(ln(total + 1) / n)`.
pub fn lcb(mean: f32, n: f32, total: f32, c: f32) -> f32 {
    if n <= 1e-3 {
        return f32::NEG_INFINITY;
    }
    mean - c * ((total.max(0.0) + 1.0).ln() / n).sqrt()
}

/// The index of the highest score (ties to the first), `None` for no scores or only `-inf`.
pub fn argmax(scores: &[f32]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, &s) in scores.iter().enumerate() {
        if s == f32::NEG_INFINITY || s.is_nan() {
            continue;
        }
        if best.is_none_or(|b| s > scores[b]) {
            best = Some(i);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_means_decay_and_shrink() {
        let mut a = Arm::default();
        a.update(1.0, 1.0);
        a.update(0.0, 3.0);
        assert_eq!((a.value, a.n), (0.25, 4.0));
        a.decay(0.5);
        assert_eq!((a.value, a.n), (0.25, 2.0));
        assert_eq!(a.shrunk(1.0, 2.0), 0.625);
        assert_eq!(Arm::default().shrunk(0.3, 0.0), 0.3);
        a.update(f32::NAN, 1.0);
        assert_eq!(a.n, 2.0);
    }

    #[test]
    fn untried_options_come_first_and_bounds_bracket_the_mean() {
        assert_eq!(ucb(0.0, 0.0, 10.0, 1.0), f32::INFINITY);
        let (u, l) = (ucb(0.5, 4.0, 10.0, 1.0), lcb(0.5, 4.0, 10.0, 1.0));
        assert!(u > 0.5 && l < 0.5 && (u - 0.5 - (0.5 - l)).abs() < 1e-6);
        assert!(ucb(0.5, 40.0, 10.0, 1.0) < u);
        assert_eq!(argmax(&[1.0, 3.0, 3.0, f32::NEG_INFINITY]), Some(1));
        assert_eq!(argmax(&[f32::NEG_INFINITY]), None);
        assert_eq!(argmax(&[]), None);
    }
}
