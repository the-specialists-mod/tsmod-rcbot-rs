//! `CWaypointLocations` (`waypoint.{h,cpp}`): waypoint indices hashed into 256-unit buckets over
//! the ±4096 map, so nearest-waypoint searches only look at the 3×3×3 buckets around a point.

use super::flags as fl;
use super::waypoints::Waypoints;
use super::{Glass, MAX_WAYPOINTS, Monsters, REACHABLE_RANGE, World};
use crate::Vector;

/// `MAX_WPT_LOCATIONS`: buckets per axis.
pub const MAX_WPT_LOCATIONS: usize = 32;

/// The buckets. Each is upstream's `dataStack<int>`: pushed on top, searched top first, so the
/// latest-added index of equally near waypoints wins.
#[derive(Clone)]
pub struct Locations {
    buckets: Vec<Vec<usize>>,
}

impl Default for Locations {
    fn default() -> Self {
        Locations::new()
    }
}

/// `abs(int(c + 4096) / 256)`. Deviation: clamped to the table (upstream indexes out of bounds
/// for coordinates past ±4096 when adding).
fn cell(c: f32) -> usize {
    (((c + 4096.0) as i32) / 256).unsigned_abs().min(MAX_WPT_LOCATIONS as u32 - 1) as usize
}

fn bucket_index(i: usize, j: usize, k: usize) -> usize {
    (i * MAX_WPT_LOCATIONS + j) * MAX_WPT_LOCATIONS + k
}

/// The buckets around `origin`, in upstream's `FOR_EACH_WPT_LOC_BUCKET` order (i, then j, then k).
fn around(origin: Vector) -> impl Iterator<Item = usize> {
    let (ci, cj, ck) = (cell(origin.x), cell(origin.y), cell(origin.z));
    let range = |c: usize| c.saturating_sub(1)..=(c + 1).min(MAX_WPT_LOCATIONS - 1);
    range(ci).flat_map(move |i| range(cj).flat_map(move |j| range(ck).map(move |k| bucket_index(i, j, k))))
}

impl Locations {
    /// Empty buckets.
    pub fn new() -> Locations {
        Locations { buckets: vec![Vec::new(); MAX_WPT_LOCATIONS * MAX_WPT_LOCATIONS * MAX_WPT_LOCATIONS] }
    }

    /// `AddWptLocation`.
    pub fn add(&mut self, index: usize, origin: Vector) {
        self.buckets[bucket_index(cell(origin.x), cell(origin.y), cell(origin.z))].push(index);
    }

    /// `DeleteWptLocation`: removes the topmost copy of `index` from `origin`'s bucket.
    pub fn delete(&mut self, index: usize, origin: Vector) {
        let b = &mut self.buckets[bucket_index(cell(origin.x), cell(origin.y), cell(origin.z))];
        if let Some(p) = b.iter().rposition(|&i| i == index) {
            b.remove(p);
        }
    }

    /// The indices in the buckets around `origin`, each bucket top first.
    fn around(&self, origin: Vector) -> impl Iterator<Item = usize> + '_ {
        around(origin).flat_map(move |b| self.buckets[b].iter().rev().copied())
    }
}

/// The options of `CWaypointLocations::NearestWaypoint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NearestOptions {
    /// `bGetVisible`: only waypoints with a clear trace from the point.
    pub visible: bool,
    /// `bGetUnReachable`: include `W_FL_UNREACHABLE` waypoints.
    pub unreachable: bool,
    /// `bIsBot`: skip waypoints a bot can't start from (jump, crouch-jump, fly). In TS the stunt
    /// bit (wall stick for other mods) stays allowed.
    pub is_bot: bool,
    /// `bNearestAimingOnly`: only aiming waypoints.
    pub aiming_only: bool,
}

impl Default for NearestOptions {
    /// Upstream's default arguments: visible, reachable only, not for a bot.
    fn default() -> Self {
        NearestOptions { visible: true, unreachable: false, is_bot: false, aiming_only: false }
    }
}

impl Waypoints {
    /// The location buckets.
    pub fn locations(&self) -> &Locations {
        &self.locations
    }

    /// `CWaypointLocations::NearestWaypoint`: the nearest waypoint in the buckets around `origin`
    /// closer than `nearest_dist`, other than `ignore` and the `failed` ones.
    ///
    /// Upstream traces to every candidate nearer than the best so far, in bucket order. The
    /// candidates are traced nearest first here, stopping at the first visible one: the same
    /// waypoint (equally near ones in bucket order, as upstream's strict `<` keeps the first),
    /// with fewer traces.
    ///
    /// Deviation: upstream's aiming-only search reads a global "failed" array left over from the
    /// previous search instead of `failed`; this uses `failed`.
    pub fn nearest_waypoint(
        &self,
        origin: Vector,
        nearest_dist: f32,
        ignore: Option<usize>,
        opts: NearestOptions,
        failed: &[usize],
        world: &dyn World,
    ) -> Option<usize> {
        let mut candidates: Vec<(f32, usize)> = Vec::new();
        for i in self.locations.around(origin) {
            if Some(i) == ignore || failed.contains(&i) {
                continue;
            }
            let w = self.wpts[i];
            if !opts.unreachable && w.flags & fl::UNREACHABLE != 0 {
                continue;
            }
            if w.flags & fl::DELETED != 0 {
                continue;
            }
            if opts.aiming_only {
                if w.flags & fl::AIMING == 0 {
                    continue;
                }
            } else if opts.is_bot && w.flags & (fl::JUMP | fl::CROUCHJUMP | fl::FLY) != 0 {
                continue;
            }
            let dist = (w.origin - origin).length();
            if dist < nearest_dist {
                candidates.push((dist, i));
            }
        }
        nearest_visible(candidates, |i| {
            !opts.visible || world.trace_line(origin, self.wpts[i].origin, Monsters::Ignore, Glass::DontIgnore, None).fraction >= 1.0
        })
    }

    /// `NearestWaypointToOrigin`: a visible, reachable waypoint within [`REACHABLE_RANGE`].
    pub fn nearest_to_origin(&self, origin: Vector, ignore: Option<usize>, failed: &[usize], world: &dyn World) -> Option<usize> {
        self.nearest_waypoint(origin, REACHABLE_RANGE, ignore, NearestOptions::default(), failed, world)
    }

    /// `CWaypointLocations::GetCoverWaypoint`: the waypoint nearest `player_origin` (within 4096,
    /// in the buckets around it) that the visibility table says can't be seen from the waypoint
    /// nearest `cover_from`.
    pub fn cover_waypoint(&self, player_origin: Vector, cover_from: Vector, ignore: &[usize], world: &dyn World) -> Option<usize> {
        self.cover_waypoint_weighted(player_origin, cover_from, ignore, world, &|_| 0.0)
    }

    /// [`cover_waypoint`](Self::cover_waypoint) with `penalty(waypoint)` units added to each
    /// candidate's distance (a port addition, design §4: the learned heatmap's
    /// [`cover_penalty`](crate::learn::heatmap::Heatmap::cover_penalty) prefers cover where the
    /// team doesn't keep dying). A zero penalty is upstream's choice exactly.
    pub fn cover_waypoint_weighted(
        &self,
        player_origin: Vector,
        cover_from: Vector,
        ignore: &[usize],
        world: &dyn World,
        penalty: &dyn Fn(usize) -> f32,
    ) -> Option<usize> {
        let opts = NearestOptions { visible: false, unreachable: true, ..NearestOptions::default() };
        let from_wpt = self.nearest_waypoint(cover_from, REACHABLE_RANGE, None, opts, &[], world)?;
        let mut nearest_dist = 4096.0f32;
        let mut nearest = None;
        for i in self.locations.around(player_origin) {
            if i == from_wpt || ignore.contains(&i) {
                continue;
            }
            let w = self.wpts[i];
            if w.flags & (fl::DELETED | fl::UNREACHABLE) != 0 || self.visibility.get(from_wpt, i) {
                continue;
            }
            let dist = (w.origin - player_origin).length() + penalty(i);
            if dist < nearest_dist {
                nearest = Some(i);
                nearest_dist = dist;
            }
        }
        nearest
    }

    /// Every waypoint in the buckets around `origin`, in bucket order (`DrawWaypoints`' loop).
    pub fn in_buckets_around(&self, origin: Vector) -> Vec<usize> {
        self.locations.around(origin).filter(|&i| i < MAX_WAYPOINTS).collect()
    }

    /// `CWaypointLocations::FillNearbyWaypoints`: every reachable waypoint in the buckets around
    /// `origin` (upstream ignores its failed list here).
    pub fn nearby_waypoints(&self, origin: Vector) -> Vec<usize> {
        self.locations.around(origin).filter(|&i| i < MAX_WAYPOINTS && self.wpts[i].flags & fl::UNREACHABLE == 0).collect()
    }
}

/// The nearest of `candidates` (distance, item; in search order) that `visible` accepts,
/// asking nearest first: what a search that keeps every visible candidate nearer than the best
/// so far finds (ties: the first in search order), without asking about the others.
pub(crate) fn nearest_visible<T: Copy>(mut candidates: Vec<(f32, T)>, mut visible: impl FnMut(T) -> bool) -> Option<T> {
    // Stable: equal distances stay in search order.
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.into_iter().map(|(_, t)| t).find(|&t| visible(t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::{EmptyWorld, Trace};
    use crate::{Random, XorShiftRandom};

    /// Walls on planes `x = c` for each `c`.
    struct Walls(Vec<f32>);

    impl World for Walls {
        fn trace_line(&self, start: Vector, end: Vector, _: Monsters, _: Glass, _: Option<u32>) -> Trace {
            if self.0.iter().any(|&x| (start.x < x) != (end.x < x)) {
                return Trace { fraction: 0.5, end_pos: start, hit: Some(0) };
            }
            Trace::clear(end)
        }
        fn point_contents(&self, _: Vector) -> i32 {
            -1
        }
    }

    /// Upstream's loop: trace every candidate nearer than the best so far.
    fn upstream_nearest(w: &Waypoints, origin: Vector, mut nearest_dist: f32, world: &dyn World) -> Option<usize> {
        let mut nearest = None;
        for i in w.locations.around(origin) {
            let wp = w.wpts[i];
            if wp.flags & (fl::UNREACHABLE | fl::DELETED) != 0 {
                continue;
            }
            let dist = (wp.origin - origin).length();
            if dist < nearest_dist && world.trace_line(origin, wp.origin, Monsters::Ignore, Glass::DontIgnore, None).fraction >= 1.0 {
                nearest = Some(i);
                nearest_dist = dist;
            }
        }
        nearest
    }

    #[test]
    fn nearest_first_finds_what_upstream_finds() {
        let mut rng = XorShiftRandom::new(11);
        for round in 0..40 {
            let mut w = Waypoints::new();
            for _ in 0..60 {
                // A coarse grid, so equal distances happen.
                let c = |r: &mut XorShiftRandom| r.random_long(-12, 12) as f32 * 32.0;
                w.add_origin(Vector::new(c(&mut rng), c(&mut rng), c(&mut rng)), 0, None);
            }
            let walls = Walls((0..round % 4).map(|_| rng.random_float(-300.0, 300.0)).collect());
            for _ in 0..20 {
                let o = Vector::new(rng.random_float(-400.0, 400.0), rng.random_float(-400.0, 400.0), rng.random_float(-400.0, 400.0));
                let opts = NearestOptions::default();
                assert_eq!(
                    w.nearest_waypoint(o, REACHABLE_RANGE, None, opts, &[], &walls),
                    upstream_nearest(&w, o, REACHABLE_RANGE, &walls)
                );
            }
        }
    }

    #[test]
    fn cells() {
        assert_eq!(cell(0.0), 16);
        assert_eq!(cell(-4096.0), 0);
        assert_eq!(cell(4095.0), 31);
        assert_eq!(cell(4096.0), 31); // clamped (upstream: out of bounds)
        assert_eq!(cell(-4300.0), 0); // int(-204)/256 = 0
        assert_eq!(cell(-9000.0), 19); // abs(-4904/256) = 19, as upstream
    }

    #[test]
    fn nearest_prefers_latest_on_ties_and_respects_options() {
        let mut w = Waypoints::new();
        let a = w.add_origin(Vector::new(10.0, 0.0, 0.0), 0, None).unwrap();
        let b = w.add_origin(Vector::new(-10.0, 0.0, 0.0), 0, None).unwrap();
        let world = EmptyWorld;
        let o = NearestOptions::default();
        assert_eq!(w.nearest_waypoint(Vector::ZERO, 100.0, None, o, &[], &world), Some(b));
        assert_eq!(w.nearest_waypoint(Vector::ZERO, 100.0, Some(b), o, &[], &world), Some(a));
        assert_eq!(w.nearest_waypoint(Vector::ZERO, 100.0, None, o, &[a, b], &world), None);
        w.set_flags(b, fl::JUMP);
        let bot = NearestOptions { is_bot: true, ..o };
        assert_eq!(w.nearest_waypoint(Vector::ZERO, 100.0, None, bot, &[], &world), Some(a));
        w.set_flags(a, fl::STUNT);
        assert_eq!(w.nearest_waypoint(Vector::ZERO, 100.0, None, bot, &[], &world), Some(a));
        w.delete(a);
        assert_eq!(w.nearest_waypoint(Vector::ZERO, 100.0, None, bot, &[], &world), None);
        assert_eq!(w.nearby_waypoints(Vector::ZERO), vec![b]);
    }

    #[test]
    fn cover() {
        let mut w = Waypoints::new();
        let enemy = w.add_origin(Vector::new(0.0, 0.0, 0.0), 0, None).unwrap();
        let seen = w.add_origin(Vector::new(100.0, 0.0, 0.0), 0, None).unwrap();
        let hidden = w.add_origin(Vector::new(150.0, 0.0, 0.0), 0, None).unwrap();
        w.visibility.set(enemy, seen, true);
        let world = EmptyWorld;
        assert_eq!(w.cover_waypoint(Vector::new(110.0, 0.0, 0.0), Vector::ZERO, &[], &world), Some(hidden));
        assert_eq!(w.cover_waypoint(Vector::new(110.0, 0.0, 0.0), Vector::ZERO, &[hidden], &world), None);
        // A penalty makes a farther hidden waypoint win; none is upstream's choice.
        let far = w.add_origin(Vector::new(300.0, 0.0, 0.0), 0, None).unwrap();
        let from = Vector::new(110.0, 0.0, 0.0);
        assert_eq!(w.cover_waypoint_weighted(from, Vector::ZERO, &[], &world, &|_| 0.0), Some(hidden));
        assert_eq!(w.cover_waypoint_weighted(from, Vector::ZERO, &[], &world, &|i| if i == hidden { 256.0 } else { 0.0 }), Some(far));
    }
}
