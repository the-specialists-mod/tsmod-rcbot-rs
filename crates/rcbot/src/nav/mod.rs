//! Navigation: waypoints, their paths and visibility, and A* (`waypoint.{h,cpp}`,
//! `bot_navigate.cpp`, `rc_compress.cpp`).
//!
//! Upstream keeps all of this in globals (`waypoints`, `num_waypoints`, `paths`,
//! `WaypointLocations`, `WaypointVisibility`); here they are one [`Waypoints`] value. Engine
//! queries (traces, point contents) go through [`World`].

pub mod astar;
pub mod editor;
pub mod file;
pub mod flags;
pub mod locations;
pub mod rcz;
pub mod visibility;
pub mod waypoints;

pub use astar::{AStar, AStarBot, AStarQuery, AStarResult, DangerSource, FailedPaths};
pub use file::{LoadError, RcwHeader};
pub use locations::{Locations, NearestOptions};
pub use visibility::VisibilityTable;
pub use waypoints::{PathChunk, PathCursor, PathRef, Waypoint, Waypoints};

use crate::Vector;

/// `MAX_WAYPOINTS` (`bot_const.h`): the size of the waypoint array, and the row stride of the
/// visibility table.
pub const MAX_WAYPOINTS: usize = 1024;
/// `MAX_PATH_INDEX`: path slots per `PATH` node of a waypoint's linked list.
pub const MAX_PATH_INDEX: usize = 4;
/// `REACHABLE_RANGE`: the default search radius and the auto-path distance limit.
pub const REACHABLE_RANGE: f32 = 400.0;
/// `MAX_JUMP_HEIGHT` (`bot_const.h`).
pub const MAX_JUMP_HEIGHT: f32 = 45.0;
/// `WAYPOINT_VERSION`: the only `.rcw` version upstream loads.
pub const WAYPOINT_VERSION: i32 = 4;
/// `CONTENTS_WATER` (`const.h`), as [`World::point_contents`] returns it.
pub const CONTENTS_WATER: i32 = -3;

/// `IGNORE_MONSTERS` of a trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Monsters {
    /// `ignore_monsters`.
    Ignore,
    /// `dont_ignore_monsters`.
    DontIgnore,
}

/// `IGNORE_GLASS` of a trace. Upstream's 5-argument `UTIL_TraceLine` doesn't ignore glass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glass {
    /// `ignore_glass`.
    Ignore,
    /// `dont_ignore_glass`.
    DontIgnore,
}

/// The part of `TraceResult` the bot reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trace {
    /// `flFraction`: 1.0 when nothing was hit.
    pub fraction: f32,
    /// `vecEndPos`.
    pub end_pos: Vector,
    /// `pHit`: the entity index hit (0 is the world), `None` when nothing was.
    pub hit: Option<u32>,
}

impl Trace {
    /// A trace that hit nothing on its way to `end`.
    pub fn clear(end: Vector) -> Trace {
        Trace { fraction: 1.0, end_pos: end, hit: None }
    }
}

/// What navigation asks the engine. `crates/host` implements it with `pfnTraceLine` and
/// `pfnPointContents`; tests use simple worlds.
pub trait World {
    /// `UTIL_TraceLine(start, end, monsters, glass, ignore, &tr)`. `ignore` is the entity index of
    /// `pentIgnore` (`None` for `NULL`).
    fn trace_line(&self, start: Vector, end: Vector, monsters: Monsters, glass: Glass, ignore: Option<u32>) -> Trace;
    /// `POINT_CONTENTS(at)`.
    fn point_contents(&self, at: Vector) -> i32;
}

/// A world with nothing in it: every trace is clear, everything is air. Useful for tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyWorld;

impl World for EmptyWorld {
    fn trace_line(&self, _: Vector, end: Vector, _: Monsters, _: Glass, _: Option<u32>) -> Trace {
        Trace::clear(end)
    }
    fn point_contents(&self, _: Vector) -> i32 {
        -1 // CONTENTS_EMPTY
    }
}
