//! `tsmod_rcbot`: the plugin library the TS Rust port loads, and the only crate in this workspace
//! allowed `unsafe` (every other crate is `#![deny(unsafe_code)]`, which `cargo xtask lint`
//! checks).
//!
//! TODO(the-specialists-mod/the-specialistsmod-rs#104): the port's plugin API (`ts-plugin-api`) is
//! being redesigned for bot plugins, so nothing here assumes its table layout yet. When it lands,
//! this crate gets the plugin entry point the port's loader calls, wraps the API's tables into safe
//! types, implements `rcbot::nav::World` and `rcbot::Random` on the engine, and registers the
//! `rcbot` bot backend (tsmod-rcbot-rs#1, step 4). Every `unsafe` block will carry a `// SAFETY:`
//! comment.
#![warn(clippy::undocumented_unsafe_blocks)]

/// The bot backend name this plugin will register (`ts_bot_backend rcbot`).
pub const BACKEND_NAME: &str = "rcbot";

// The AI, linked in for the entry point to come.
use rcbot as _;
