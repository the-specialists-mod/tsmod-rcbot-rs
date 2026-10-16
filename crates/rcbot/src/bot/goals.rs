//! Where the bot goes when it has nothing to do (port addition, design §4).
//!
//! Upstream's TS bot picked up the nearest *visible* item, went for a random objective, or roamed
//! to one of the first 50 waypoints (`WaypointFindRandomGoal`) or to the newest remembered enemy
//! position. The port scores what it knows of the map instead: roaming prefers waypoints the bot
//! hasn't been to for a while ([`Explore`]).

use crate::Vector;
use crate::nav::{MAX_WAYPOINTS, Waypoints};

/// Seconds after which a waypoint counts as unvisited again.
const FORGET_VISIT: f32 = 120.0;
/// Roaming goals nearer than this are rarely chosen: the bot would be there at once.
const NEAR_GOAL: f32 = 400.0;

/// When the bot last touched each waypoint, for roaming to the parts of the map it hasn't seen
/// lately.
#[derive(Clone, Debug)]
pub(crate) struct Explore {
    visited: Vec<f32>,
}

impl Default for Explore {
    fn default() -> Self {
        Explore { visited: vec![f32::NEG_INFINITY; MAX_WAYPOINTS] }
    }
}

impl Explore {
    /// The bot touched waypoint `i` at `now`.
    pub fn touch(&mut self, i: i32, now: f32) {
        if let Some(v) = usize::try_from(i).ok().and_then(|i| self.visited.get_mut(i)) {
            *v = now;
        }
    }

    /// Seconds since the bot was at waypoint `i` (at most [`FORGET_VISIT`]).
    pub fn since_visit(&self, i: usize, now: f32) -> f32 {
        self.visited.get(i).map_or(FORGET_VISIT, |&t| (now - t).clamp(0.0, FORGET_VISIT))
    }

    /// How much the bot wants to roam to waypoint `i` from `from`: more the longer it hasn't been
    /// there, little when it is near.
    pub fn weight(&self, waypoints: &Waypoints, i: usize, from: Vector, now: f32) -> f32 {
        let mut w = 0.25 + self.since_visit(i, now) / 30.0;
        if (waypoints.origin(i as i32) - from).length() < NEAR_GOAL {
            w *= 0.25;
        }
        w
    }
}
