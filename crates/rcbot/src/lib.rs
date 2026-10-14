//! RCBot1 (rcbotold, by Paul "Cheeseh" Murphy, maintained by [APG]RoboCop) ported to Rust, for The
//! Specialists 3.0 only.
//!
//! This crate is the bot itself and has no FFI: everything it needs from the engine and the game
//! comes in through traits ([`nav::World`], [`Random`]) that `crates/host` implements on top of the
//! port's plugin API. Upstream file and function names are cited in doc comments
//! (``/// `WaypointFindPath` ``) so each piece can be checked against the C++.
//!
//! Ported so far (see `docs/design.md` for the full C++ → Rust map and the planned modules):
//! * [`nav`]: `.rcw` waypoints (byte-compatible), flags, paths, the location buckets, the
//!   visibility table and its `.rcv` file, A* (`waypoint.cpp`, `bot_navigate.cpp`).
//! * [`weapons`]: `bot_weapons.ini` presets and the TS weapon table (`bot_weapons.{h,cpp}`).
//! * [`chat`]: MegaHAL with byte-compatible brain files, `bot_chat.ini` and how bots pick, fill
//!   and reply with chat lines (`megahal.{h,cpp}`, `CBot::BotChat`, `CBotGlobals::SetupBotChat`).
//! * [`config`]: `bot_config.ini`, users, profiles, reputation, menus and the `rcbot` command set
//!   (`globals.cpp`, `dll.cpp`, `bot_commands.cpp`, `bot_menu.cpp`, `bot_rep.cpp`).
//! * [`learn`]: the GA, perceptrons, the NN, the SOM and the `.rld` learned-data files (`ga.cpp`,
//!   `bot_ga.cpp`, `perceptron.cpp`, `bot_nn.cpp`, `som.cpp`, `bits.cpp`).
//!
//! RCBot's licence: GPL-2.0-or-later with a linking exception for the HL engine and MODs; this port
//! is GPL-3.0-or-later and keeps that exception (`GPL-EXCEPTION.txt`).
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod bot;
pub mod chat;
pub mod config;
pub mod learn;
pub mod math;
pub mod nav;
pub mod weapons;

mod random;
mod stoi;

pub use math::Vector;
pub use random::{Random, XorShiftRandom};
