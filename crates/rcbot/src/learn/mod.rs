//! RCBot's learning: the genetic algorithm, perceptrons, the neural network, the self-organising
//! map, bit arrays, and the learned-data files (`ga.{h,cpp}`, `bot_ga.{h,cpp}`,
//! `perceptron.{h,cpp}`, `nn.h`, `bot_nn.cpp`, `som.{h,cpp}`, `bits.{h,cpp}`, `gannconst.h`, and
//! the learned-data parts of `bot.cpp` and `globals.cpp`).
//!
//! On TS the bot breeds its loadout and stunt weights, its jump/duck/strafe weights and its
//! team's enemy-cost weights with the GA, and decides stunts and movement with perceptrons fed
//! those weights ([`data`]). Every object saves and loads in upstream's byte layout, so `.rld`
//! files carry over between RCBot and the port.
//!
//! All randomness goes through [`crate::Random`], in upstream's order, so a seeded generator
//! replays a run exactly (`tests/upstream_learn.rs` checks this against upstream's own code).
//! `bot_enemy_cost_ga.h` is empty upstream.

pub mod bandit;
pub mod bits;
pub mod data;
pub mod ga;
pub mod header;
pub mod io;
pub mod ledger;
pub mod nn;
pub mod opponents;
pub mod perceptron;
pub mod som;
pub mod store;
pub mod values;
pub mod weapon_stats;

pub use bits::Bits;
pub use data::{BotLearnedData, GlobalLearnedData};
pub use ga::{Ga, GaValue, Individual, Population, PopulationMember, Selection};
pub use io::Reader;
pub use ledger::{CombatEvent, DamageEvent, KillEvent, Ledger, LedgerEntry};
pub use nn::{Nn, NnGaTrained, NnLayer, TrainSet};
pub use perceptron::Perceptron;
pub use som::Som;
pub use values::{BitsGaValues, BotGaValues, CombatFitnessMetrics, IntGaValues};
pub use weapon_stats::WeaponStats;
