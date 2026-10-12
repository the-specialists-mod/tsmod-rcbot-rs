//! RCBot1 (rcbotold, by Paul "Cheeseh" Murphy, maintained by [APG]RoboCop) ported to Rust, for The
//! Specialists 3.0 only.
//!
//! This crate is the bot itself and has no FFI: everything it needs from the engine and the game
//! comes in through traits (such as [`Random`]) that `crates/host` implements on top of the
//! port's plugin API. Upstream file and function names are cited in doc comments
//! (``/// `WaypointFindPath` ``) so each piece can be checked against the C++.
//!
//! RCBot's licence: GPL-2.0-or-later with a linking exception for the HL engine and MODs; this port
//! is GPL-3.0-or-later and keeps that exception (`LICENSE-EXCEPTION.txt`).
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod math;

mod random;

pub use math::Vector;
pub use random::{Random, XorShiftRandom};
