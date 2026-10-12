//! The learning code against RCBot's own: `tools/reference/learn_harness.cpp` runs upstream's
//! `ga.cpp`, `bot_ga.cpp`, `perceptron.cpp`, `bot_nn.cpp`, `som.cpp` and `bits.cpp` on fixed
//! scenarios with the same random generator as [`XorShiftRandom`] and records what they save in
//! `tests/reference/learn_*`. Each test replays a scenario here and compares the bytes: same
//! random draws in the same order, same float arithmetic, same file layout.

use std::path::PathBuf;

use rcbot::learn::{
    Bits, BitsGaValues, BotGaValues, Ga, GlobalLearnedData, Individual, IntGaValues, NnGaTrained, Perceptron, Reader, Som, TrainSet,
};
use rcbot::{Random, XorShiftRandom};

fn reference(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[track_caller]
fn assert_bytes(ours: &[u8], name: &str) {
    let theirs = reference(name);
    if ours != theirs {
        let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
        panic!("{name}: differs from upstream at byte {at} (ours {} bytes, upstream {})", ours.len(), theirs.len());
    }
}

#[test]
fn ga_breeding_matches_upstream() {
    let rng = &mut XorShiftRandom::new(101);
    let mut ga: Ga<BotGaValues> = Ga::new(6);
    let mut out = Vec::new();
    for life in 0..40 {
        let mut v = BotGaValues::new();
        for _ in 0..5 {
            v.add(rng.random_float(-1.0, 1.0));
        }
        v.set_fitness(rng.random_float(0.0, 2.0));
        ga.add_to_population(v, rng);
        if life % 3 == 2
            && let Some(p) = ga.pick()
        {
            p.save(&mut out);
        }
    }
    ga.save(&mut out);
    put_u32(&mut out, ga.generations());
    assert!(ga.generations() >= 5);
    assert_bytes(&out, "learn_ga.bin");
}

#[test]
fn int_ga_matches_upstream() {
    let rng = &mut XorShiftRandom::new(202);
    let mut ga: Ga<IntGaValues> = Ga::new(5);
    ga.set_adaptive_mutation(false);
    for _ in 0..23 {
        let mut v = IntGaValues::new(rng.random_long(0, 0x7fff_ffff));
        v.set_fitness(rng.random_float(0.0, 1.0));
        ga.add_to_population(v, rng);
    }
    let mut out = Vec::new();
    ga.save(&mut out);
    assert_bytes(&out, "learn_intga.bin");

    let mut back: Ga<IntGaValues> = Ga::new(5);
    back.load(&mut Reader::new(&out), 4);
    let mut again = Vec::new();
    back.save(&mut again);
    assert_eq!(again, out);
}

#[test]
fn bits_match_upstream() {
    let rng = &mut XorShiftRandom::new(303);
    let mut ba = Bits::new(21);
    let mut bb = Bits::new(21);
    ba.randomize(rng);
    bb.randomize(rng);
    let mut a = BitsGaValues::from_bits(ba);
    let mut b = BitsGaValues::from_bits(bb);
    a.cross_over(&mut b, rng);
    for _ in 0..3 {
        a.mutate(rng);
        b.mutate(rng);
    }
    a.set_fitness(0.5);
    let mut out = Vec::new();
    a.save(&mut out);
    b.save(&mut out);
    let mut n = 0;
    a.convert(&mut n);
    put_u32(&mut out, n);
    assert_bytes(&out, "learn_bits.bin");
}

#[test]
fn perceptron_matches_upstream() {
    let rng = &mut XorShiftRandom::new(404);
    let mut p = Perceptron::new(4, 0.0, rng);
    let mut out = Vec::new();
    for _ in 0..30 {
        let mut inputs = Vec::new();
        let mut sum = 0.0f32;
        for _ in 0..4 {
            inputs.push(rng.random_float(-1.0, 1.0));
            sum += inputs[inputs.len() - 1];
        }
        p.input(&inputs);
        put_f32(&mut out, p.execute());
        p.train(if sum > 0.0 { 1.0 } else { 0.0 });
    }
    p.save(&mut out);
    assert_bytes(&out, "learn_perceptron.bin");
}

#[test]
fn ga_trained_network_matches_upstream() {
    let rng = &mut XorShiftRandom::new(505);
    let mut nn = NnGaTrained::new(1, 3, 3, 2, rng);
    let sets: Vec<TrainSet> = (0..4)
        .map(|s| TrainSet {
            inputs: vec![s as f32 * 0.25, 1.0 - s as f32 * 0.25, if s % 2 == 1 { 1.0 } else { 0.0 }],
            outputs: vec![if s % 2 == 1 { 1.0 } else { 0.0 }, if s < 2 { 1.0 } else { 0.0 }],
        })
        .collect();
    for _ in 0..30 {
        nn.train(&sets, rng);
    }
    let mut out = Vec::new();
    let outputs = nn.nn.execute(&[0.3, 0.6, 1.0]);
    put_u32(&mut out, outputs.len() as u32);
    for o in outputs {
        put_f32(&mut out, o);
    }
    nn.nn.save(&mut out);
    assert_bytes(&out, "learn_nn.bin");
}

#[test]
fn som_matches_upstream() {
    let rng = &mut XorShiftRandom::new(606);
    let mut som = Som::new(4, 3, 2, rng);
    let inputs = vec![vec![0.0, 0.0], vec![1.0, 1.0], vec![0.0, 1.0], vec![1.0, 0.0], vec![0.5, 0.5]];
    som.input_epochs(&inputs, 3, rng);
    let mut out = Vec::new();
    put_u32(&mut out, som.epochs());
    for input in &inputs {
        let n = &som.neurons()[som.bmu(input).unwrap()];
        put_f32(&mut out, n.x());
        put_f32(&mut out, n.y());
        for &w in n.weights() {
            put_f32(&mut out, w);
        }
    }
    assert_bytes(&out, "learn_som.bin");
}

#[test]
fn ts_loadout_file_matches_upstream() {
    let rng = &mut XorShiftRandom::new(707);
    let mut g = GlobalLearnedData::default();
    for _ in 0..20 {
        let mut v = BotGaValues::new();
        for _ in 0..8 {
            v.add(rng.random_float(0.0, 1.0));
        }
        v.set_fitness(rng.random_float(0.0, 1.0));
        g.ts_weapon_choices.add_to_population(v, rng);
    }
    let saved = g.save_ts_weapons();
    assert_bytes(&saved, "learn_tsweaps.rld");

    // Upstream loads tsweaps.rld with 16 genes: everything after the first individual's header
    // is misread. The port reproduces that through the generic loader...
    let mut upstream_way: Ga<BotGaValues> = Ga::new(0);
    upstream_way.load(&mut Reader::new(&saved), 16);
    let mut out = Vec::new();
    upstream_way.save(&mut out);
    assert_bytes(&out, "learn_tsweaps_reloaded.rld");

    // ...and loads the file with 8 (design §4), which keeps every loadout.
    let mut ported = GlobalLearnedData::default();
    ported.load_ts_weapons(&saved);
    assert_eq!(ported.save_ts_weapons(), saved);
}
