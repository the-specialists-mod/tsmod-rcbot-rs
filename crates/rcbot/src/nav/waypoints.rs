//! The waypoint array and its paths (`waypoint.cpp`: `WAYPOINTS`, `paths`, `WaypointAddPath`,
//! `WaypointFindPath` and the searches over all waypoints).

use super::flags as fl;
use super::locations::Locations;
use super::visibility::VisibilityTable;
use super::{CONTENTS_WATER, Glass, MAX_JUMP_HEIGHT, MAX_PATH_INDEX, MAX_WAYPOINTS, Monsters, REACHABLE_RANGE, World};
use crate::{Random, Vector};

/// `WAYPOINT`: what a `.rcw` file stores per waypoint (16 bytes: `int flags; Vector origin`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Waypoint {
    /// `W_FL_*` bits ([`super::flags`]).
    pub flags: u32,
    /// Where it is.
    pub origin: Vector,
}

/// One `PATH` node of upstream's per-waypoint linked list: four slots, `-1` when unused.
pub type PathChunk = [i16; MAX_PATH_INDEX];

/// Which path node a path was found in: upstream identifies a path by its `PATH*` (a node of four
/// slots), which is what `CBotFailedPaths` remembers. Marking one failed path makes the bot skip
/// every successor in that node, as upstream does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathRef {
    /// The waypoint the path leaves from.
    pub from: usize,
    /// The node in that waypoint's list.
    pub chunk: usize,
}

/// The iteration state of `WaypointFindPath(&pPath, &path_index, …)`: start with
/// [`PathCursor::new`] (upstream's `pPath = NULL`) and call [`Waypoints::find_path`] until it
/// returns `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathCursor {
    from: usize,
    chunk: usize,
    slot: usize,
    started: bool,
}

impl PathCursor {
    /// A cursor over the paths leaving `from`.
    pub fn new(from: usize) -> PathCursor {
        PathCursor { from, chunk: 0, slot: 0, started: false }
    }

    /// The path node the last returned successor came from (`*pPath`).
    pub fn path_ref(&self) -> PathRef {
        PathRef { from: self.from, chunk: self.chunk }
    }
}

/// All the waypoints of a map: upstream's `waypoints`, `num_waypoints`, `paths`,
/// `WaypointLocations` and `WaypointVisibility` globals.
#[derive(Clone)]
pub struct Waypoints {
    pub(crate) wpts: Vec<Waypoint>,
    pub(crate) num: usize,
    pub(crate) paths: Vec<Vec<PathChunk>>,
    pub(crate) locations: Locations,
    /// `WaypointVisibility`.
    pub visibility: VisibilityTable,
    /// The bytes of the loaded file's `filetype` after "RCBot\0". Upstream `strcpy`s "RCBot" into
    /// an uninitialised header, so saved files carry stack garbage there; keeping what was loaded
    /// makes load + save byte-exact. Zero for new waypoints.
    pub(crate) filetype_tail: [u8; 2],
}

impl Default for Waypoints {
    fn default() -> Self {
        Waypoints::new()
    }
}

impl std::fmt::Debug for Waypoints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waypoints").field("num", &self.num).finish_non_exhaustive()
    }
}

impl Waypoints {
    /// `WaypointInit`: no waypoints, no paths, an empty visibility table.
    pub fn new() -> Waypoints {
        Waypoints {
            wpts: vec![Waypoint::default(); MAX_WAYPOINTS],
            num: 0,
            paths: vec![Vec::new(); MAX_WAYPOINTS],
            locations: Locations::new(),
            visibility: VisibilityTable::new(),
            filetype_tail: [0; 2],
        }
    }

    /// `num_waypoints`: one past the highest slot in use (deleted slots included).
    pub fn len(&self) -> usize {
        self.num
    }

    /// No waypoints at all.
    pub fn is_empty(&self) -> bool {
        self.num == 0
    }

    /// The waypoint in slot `index` (`index < MAX_WAYPOINTS`; deleted ones included).
    pub fn get(&self, index: usize) -> Option<&Waypoint> {
        self.wpts.get(index)
    }

    /// The slots `0..len()`.
    pub fn as_slice(&self) -> &[Waypoint] {
        &self.wpts[..self.num]
    }

    /// `WaypointFlags`: 0 for an invalid index.
    pub fn flags(&self, index: i32) -> u32 {
        usize::try_from(index).ok().and_then(|i| self.wpts.get(i)).map_or(0, |w| w.flags)
    }

    /// Replaces a waypoint's flags (the editor's flag toggles).
    pub fn set_flags(&mut self, index: usize, flags: u32) {
        if let Some(w) = self.wpts.get_mut(index) {
            w.flags = flags;
        }
    }

    /// `WaypointOrigin`: `(0,0,0)` for an invalid index.
    pub fn origin(&self, index: i32) -> Vector {
        usize::try_from(index).ok().and_then(|i| self.wpts.get(i)).map_or(Vector::ZERO, |w| w.origin)
    }

    /// `WaypointDistance`.
    pub fn distance(&self, from: Vector, index: i32) -> f32 {
        if index < 0 {
            return 0.0;
        }
        (self.origin(index) - from).length()
    }

    /// The raw path slots of a waypoint, node by node (`-1` = unused), as `paths[index]` holds them.
    pub fn path_chunks(&self, index: usize) -> &[PathChunk] {
        self.paths.get(index).map_or(&[], Vec::as_slice)
    }

    /// The used path slots of a waypoint in list order: what `WaypointSave` writes. Values are
    /// raw (a file may hold out-of-range indices; [`Waypoints::find_path`] skips them).
    pub fn paths_from(&self, index: usize) -> impl Iterator<Item = i16> + '_ {
        self.path_chunks(index).iter().flatten().copied().filter(|&p| p != -1)
    }

    /// `WaypointAddPath`: the first free slot in the list, or a new node at its end. Doesn't check
    /// for duplicates (the editor's `WaypointCreatePath` does).
    pub fn add_path(&mut self, add_index: usize, path_index: i16) {
        let Some(list) = self.paths.get_mut(add_index) else { return };
        if let Some(slot) = list.iter_mut().flatten().find(|s| **s == -1) {
            *slot = path_index;
            return;
        }
        list.push([path_index, -1, -1, -1]);
    }

    /// `WaypointDeletePath(del_index)`: removes every path to `del_index` (from waypoints
    /// `0..len()`).
    pub fn delete_paths_to(&mut self, del_index: i16) {
        for list in &mut self.paths[..self.num] {
            for slot in list.iter_mut().flatten() {
                if *slot == del_index {
                    *slot = -1;
                }
            }
        }
    }

    /// `WaypointDeletePath(path_index, del_index)`: removes the paths from `from` to `del_index`.
    pub fn delete_path(&mut self, from: usize, del_index: i16) {
        if let Some(list) = self.paths.get_mut(from) {
            for slot in list.iter_mut().flatten() {
                if *slot == del_index {
                    *slot = -1;
                }
            }
        }
    }

    /// `WaypointFindPath`: the next successor of the cursor's waypoint usable by `team` (-1 = any),
    /// skipping team-specific waypoints of other teams and unused or out-of-range slots. Like
    /// upstream it gives up (returns `None`) at a negative index other than -1.
    pub fn find_path(&self, cursor: &mut PathCursor, team: i32) -> Option<usize> {
        if !cursor.started {
            if cursor.from >= MAX_WAYPOINTS {
                return None;
            }
            cursor.chunk = 0;
            cursor.slot = 0;
            cursor.started = true;
        }
        let list = &self.paths[cursor.from];
        while cursor.chunk < list.len() {
            while cursor.slot < MAX_PATH_INDEX {
                let v = list[cursor.chunk][cursor.slot];
                if v != -1 && i32::from(v) < self.num as i32 {
                    if v < 0 {
                        return None;
                    }
                    let index = v as usize;
                    cursor.slot += 1;
                    if !fl::team_allows(self.wpts[index].flags, team) {
                        continue;
                    }
                    return Some(index);
                }
                cursor.slot += 1;
            }
            cursor.slot = 0;
            if cursor.chunk + 1 < list.len() {
                cursor.chunk += 1;
            } else {
                break;
            }
        }
        None
    }

    /// The successors of `from` for `team`, in `WaypointFindPath` order.
    pub fn successors(&self, from: usize, team: i32) -> Vec<usize> {
        let mut cursor = PathCursor::new(from);
        std::iter::from_fn(|| self.find_path(&mut cursor, team)).collect()
    }

    /// `BotNavigate_FindPathFromTo`: the path node of `from` that leads to `to`, if there is a path.
    pub fn find_path_from_to(&self, from: usize, to: usize, team: i32) -> Option<PathRef> {
        let mut cursor = PathCursor::new(from);
        while let Some(index) = self.find_path(&mut cursor, team) {
            if index == to {
                return Some(cursor.path_ref());
            }
        }
        None
    }

    fn usable_goal(w: &Waypoint, team: i32) -> bool {
        w.flags & (fl::DELETED | fl::AIMING) == 0 && fl::team_allows(w.flags, team)
    }

    fn ignore_set(ignore: &[usize]) -> Vec<bool> {
        let mut set = vec![false; MAX_WAYPOINTS];
        for &i in ignore {
            if let Some(s) = set.get_mut(i) {
                *s = true;
            }
        }
        set
    }

    /// `WaypointFindNearest(v_src, pEntity, range, team)`: the nearest usable waypoint closer than
    /// `range` with a clear trace from `trace_from`. `trace_from` is `v_src` for this overload and
    /// the eyes (`origin + view_ofs`) for the `pEntity`-only one, which measures from the origin.
    pub fn find_nearest(
        &self,
        v_src: Vector,
        trace_from: Vector,
        ignore_ent: Option<u32>,
        range: f32,
        team: i32,
        world: &dyn World,
    ) -> Option<usize> {
        let mut min_index = None;
        let mut min_distance = 9999.0f32;
        for (i, w) in self.as_slice().iter().enumerate() {
            if !Self::usable_goal(w, team) {
                continue;
            }
            let distance = (w.origin - v_src).length();
            if distance < min_distance && distance < range {
                let tr = world.trace_line(trace_from, w.origin, Monsters::Ignore, Glass::DontIgnore, ignore_ent);
                if tr.fraction >= 1.0 {
                    min_index = Some(i);
                    min_distance = distance;
                }
            }
        }
        min_index
    }

    /// `WaypointFindNearestGoal`: the nearest usable waypoint with all of `flags`, closer than
    /// `range`, not in `ignore` (no visibility check).
    pub fn find_nearest_goal(&self, v_src: Vector, range: f32, team: i32, flags: u32, ignore: &[usize]) -> Option<usize> {
        let failed = Self::ignore_set(ignore);
        let mut min_index = None;
        let mut min_distance = 99999.0f32;
        for (i, w) in self.as_slice().iter().enumerate() {
            if failed[i] || !Self::usable_goal(w, team) || w.flags & flags != flags {
                continue;
            }
            let distance = (w.origin - v_src).length();
            if distance < range && distance < min_distance {
                min_index = Some(i);
                min_distance = distance;
            }
        }
        min_index
    }

    /// Up to 50 candidates for the random-goal searches, in index order (upstream's
    /// `int indexes[50]`).
    fn random_goal_candidates(&self, team: i32, flags: u32, near: Option<(Vector, f32)>, ignore: &[usize]) -> Vec<usize> {
        let failed = Self::ignore_set(ignore);
        let mut out = Vec::with_capacity(50);
        for (i, w) in self.as_slice().iter().enumerate() {
            if failed[i] || !Self::usable_goal(w, team) || w.flags & flags != flags {
                continue;
            }
            if let Some((src, range)) = near
                && (w.origin - src).length() >= range
            {
                continue;
            }
            if out.len() < 50 {
                out.push(i);
            }
        }
        out
    }

    /// `WaypointFindRandomGoal(pEntity, team, flags, ignore)` (and, with `flags == 0`, the overload
    /// without flags): one of the first 50 usable waypoints with all of `flags`.
    pub fn find_random_goal(&self, team: i32, flags: u32, ignore: &[usize], rng: &mut dyn Random) -> Option<usize> {
        if self.num < 1 {
            return None;
        }
        let c = self.random_goal_candidates(team, flags, None, ignore);
        if c.is_empty() {
            return None;
        }
        let i = rng.random_long(1, c.len() as i32) - 1;
        c.get(i as usize).copied()
    }

    /// `WaypointFindRandomGoal(v_src, pEntity, range, team, flags, ignore)`: as
    /// [`Waypoints::find_random_goal`], only waypoints closer than `range` to `v_src`.
    pub fn find_random_goal_near(
        &self,
        v_src: Vector,
        range: f32,
        team: i32,
        flags: u32,
        ignore: &[usize],
        rng: &mut dyn Random,
    ) -> Option<usize> {
        if self.num < 1 {
            return None;
        }
        let c = self.random_goal_candidates(team, flags, Some((v_src, range)), ignore);
        if c.is_empty() {
            return None;
        }
        let i = rng.random_long(0, c.len() as i32 - 1);
        c.get(i as usize).copied()
    }

    /// `WaypointFindNearestAiming`: the nearest aiming waypoint within 40 units.
    pub fn find_nearest_aiming(&self, v_origin: Vector) -> Option<usize> {
        let mut min_index = None;
        let mut min_distance = 9999.0f32;
        for (i, w) in self.as_slice().iter().enumerate() {
            if w.flags & fl::DELETED != 0 || w.flags & fl::AIMING == 0 {
                continue;
            }
            let distance = (v_origin - w.origin).length();
            if distance < min_distance && distance < 40.0 {
                min_index = Some(i);
                min_distance = distance;
            }
        }
        min_index
    }

    /// `WaypointFindReachable`: the nearest usable waypoint visible from the eyes
    /// (`origin + view_ofs`) and [`Waypoints::reachable`] from `origin`, if within `range`.
    pub fn find_reachable(
        &self,
        origin: Vector,
        view_ofs: Vector,
        ignore_ent: Option<u32>,
        range: f32,
        team: i32,
        world: &dyn World,
    ) -> Option<usize> {
        let mut min_index = None;
        let mut min_distance = 9999.0f32;
        for (i, w) in self.as_slice().iter().enumerate() {
            if !Self::usable_goal(w, team) {
                continue;
            }
            let distance = (w.origin - origin).length();
            if distance < min_distance {
                let tr = world.trace_line(origin + view_ofs, w.origin, Monsters::Ignore, Glass::DontIgnore, ignore_ent);
                if tr.fraction >= 1.0 && Self::reachable(origin, w.origin, true, world) {
                    min_index = Some(i);
                    min_distance = distance;
                }
            }
        }
        if min_distance > range { None } else { min_index }
    }

    /// `WaypointReachable`: can a bot walk from `src` to `dest`? Closer than
    /// [`REACHABLE_RANGE`] (when `dist_check`), a clear trace, both underwater or else: not a
    /// mid-air destination more than a jump above, and no drop deeper than a jump on the way
    /// (sampled every 10 units).
    pub fn reachable(src: Vector, dest: Vector, dist_check: bool, world: &dyn World) -> bool {
        let mut distance = (dest - src).length();
        if dist_check && distance >= REACHABLE_RANGE {
            return false;
        }
        if world.trace_line(src, dest, Monsters::Ignore, Glass::DontIgnore, None).fraction < 1.0 {
            return false;
        }
        if world.point_contents(src) == CONTENTS_WATER && world.point_contents(dest) == CONTENTS_WATER {
            return true;
        }
        if dest.z > src.z + MAX_JUMP_HEIGHT {
            let down = Vector::new(dest.x, dest.y, dest.z - 50.0);
            if world.trace_line(dest, down, Monsters::DontIgnore, Glass::DontIgnore, None).fraction >= 1.0 {
                return false; // in mid-air
            }
        }
        let direction = (dest - src).normalize();
        let mut check = src;
        let mut down = Vector::new(src.x, src.y, src.z - 1000.0);
        let mut last_height = world.trace_line(check, down, Monsters::Ignore, Glass::DontIgnore, None).fraction * 1000.0;
        distance = (dest - check).length();
        while distance > 10.0 {
            check = check + direction * 10.0;
            down = Vector::new(check.x, check.y, check.z - 1000.0);
            let curr_height = world.trace_line(check, down, Monsters::Ignore, Glass::DontIgnore, None).fraction * 1000.0;
            if last_height - curr_height > MAX_JUMP_HEIGHT {
                return false;
            }
            last_height = curr_height;
            distance = (dest - check).length();
        }
        true
    }

    /// The core of `WaypointAddOrigin`: puts a waypoint in the first deleted slot (or at the end)
    /// and returns its index, `None` when full. `flags` are the caller's; the host adds what
    /// upstream works out from the player and nearby items (crouch, ladder, ammo, weapon, health,
    /// armour). With `auto_path` (`m_bAutoPathWaypoint`), it updates the visibility rows of the
    /// new waypoint and adds paths both ways to every waypoint [`Waypoints::reachable`] from it.
    pub fn add_origin(&mut self, origin: Vector, flags: u32, auto_path: Option<&dyn World>) -> Option<usize> {
        let index = self.wpts[..self.num].iter().position(|w| w.flags & fl::DELETED != 0).unwrap_or(self.num);
        if index >= MAX_WAYPOINTS {
            return None;
        }
        self.wpts[index] = Waypoint { flags, origin };
        if index == self.num {
            self.num += 1;
        }
        if let Some(world) = auto_path {
            for i in 0..self.num {
                if i == index {
                    continue;
                }
                // Upstream doesn't skip deleted slots here (they sit at the origin).
                let other = self.wpts[i];
                if other.flags & fl::AIMING != 0 {
                    continue;
                }
                let visible = world.trace_line(origin, other.origin, Monsters::Ignore, Glass::Ignore, None).fraction >= 1.0;
                self.visibility.set(index, i, visible);
                self.visibility.set(i, index, visible);
                if Self::reachable(origin, other.origin, true, world) {
                    self.add_path(index, i as i16);
                }
                if Self::reachable(other.origin, origin, true, world) {
                    self.add_path(i, index as i16);
                }
            }
        }
        self.locations.add(index, origin);
        Some(index)
    }

    /// `WaypointAddAiming`: an aiming waypoint 25 units in front of `origin` along the yaw of
    /// `v_angle`. Like upstream it isn't added to the location buckets (until the next load).
    pub fn add_aiming(&mut self, origin: Vector, v_angle: Vector) -> Option<usize> {
        if self.num >= MAX_WAYPOINTS {
            return None;
        }
        let index = self.wpts[..self.num].iter().position(|w| w.flags & fl::DELETED != 0).unwrap_or(self.num);
        let forward = Vector::forward_from_angles(Vector::new(0.0, v_angle.y, 0.0));
        self.wpts[index] = Waypoint { flags: fl::AIMING, origin: origin + forward * 25.0 };
        if index == self.num {
            self.num += 1;
        }
        Some(index)
    }

    /// The core of `WaypointDelete`: removes every path to and from `index` and marks the slot
    /// deleted. Deviation: upstream zeroes the origin before removing the index from the location
    /// buckets, so it looks in the wrong bucket and leaves a stale entry; this removes it from the
    /// bucket it is in.
    pub fn delete(&mut self, index: usize) {
        if index >= self.num {
            return;
        }
        self.delete_paths_to(index as i16);
        self.paths[index].clear();
        let old = self.wpts[index].origin;
        self.wpts[index] = Waypoint { flags: fl::DELETED, origin: Vector::ZERO };
        self.locations.delete(index, old);
    }

    /// The paths of every waypoint, as `(from, to)` pairs of used slots in list order.
    pub fn all_paths(&self) -> impl Iterator<Item = (usize, i16)> + '_ {
        (0..self.num).flat_map(move |i| self.paths_from(i).map(move |p| (i, p)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;
    use crate::nav::EmptyWorld;

    fn line(n: usize) -> Waypoints {
        let mut w = Waypoints::new();
        for i in 0..n {
            w.add_origin(Vector::new(i as f32 * 100.0, 0.0, 0.0), 0, None).unwrap();
        }
        for i in 0..n - 1 {
            w.add_path(i, (i + 1) as i16);
            w.add_path(i + 1, i as i16);
        }
        w
    }

    #[test]
    fn path_slots_fill_holes_first() {
        let mut w = line(3);
        for p in [2, 2, 2, 2, 2] {
            w.add_path(0, p);
        }
        // 1, then five 2s: two nodes.
        assert_eq!(w.path_chunks(0), &[[1, 2, 2, 2], [2, 2, -1, -1]]);
        w.delete_path(0, 2);
        assert_eq!(w.path_chunks(0), &[[1, -1, -1, -1], [-1, -1, -1, -1]]);
        w.add_path(0, 7);
        assert_eq!(w.path_chunks(0)[0], [1, 7, -1, -1]);
    }

    #[test]
    fn find_path_skips_other_teams_and_bad_slots() {
        let mut w = line(4);
        w.set_flags(2, fl::TEAM_SPECIFIC | 1);
        w.add_path(1, 3);
        w.add_path(1, 99); // out of range: skipped
        assert_eq!(w.successors(1, -1), vec![0, 2, 3]);
        assert_eq!(w.successors(1, 0), vec![0, 3]);
        assert_eq!(w.successors(1, 1), vec![0, 2, 3]);
        w.add_path(1, -5); // a negative index ends the search, as upstream
        w.add_path(1, 0);
        assert_eq!(w.successors(1, -1), vec![0, 2, 3]);
        assert_eq!(w.find_path_from_to(1, 3, -1), Some(PathRef { from: 1, chunk: 0 }));
        assert_eq!(w.find_path_from_to(0, 3, -1), None);
    }

    #[test]
    fn add_delete_reuses_slots() {
        let mut w = line(3);
        w.delete(1);
        assert_eq!(w.flags(1), fl::DELETED);
        assert_eq!(w.successors(0, -1), Vec::<usize>::new());
        assert_eq!(w.add_origin(Vector::new(5.0, 5.0, 0.0), fl::JUMP, None), Some(1));
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn auto_path_in_empty_world() {
        let mut w = Waypoints::new();
        let world = EmptyWorld;
        w.add_origin(Vector::new(0.0, 0.0, 0.0), 0, Some(&world));
        w.add_origin(Vector::new(300.0, 0.0, 0.0), 0, Some(&world));
        w.add_origin(Vector::new(800.0, 0.0, 0.0), 0, Some(&world)); // too far from 0, 500 from 1
        assert_eq!(w.successors(0, -1), vec![1]);
        assert_eq!(w.successors(1, -1), vec![0]);
        assert!(w.successors(2, -1).is_empty());
        assert!(w.visibility.get(0, 2) && w.visibility.get(2, 0));
    }

    #[test]
    fn reachable_rules() {
        let world = EmptyWorld;
        assert!(Waypoints::reachable(Vector::ZERO, Vector::new(399.0, 0.0, 0.0), true, &world));
        assert!(!Waypoints::reachable(Vector::ZERO, Vector::new(400.0, 0.0, 0.0), true, &world));
        assert!(Waypoints::reachable(Vector::ZERO, Vector::new(400.0, 0.0, 0.0), false, &world));
        // More than a jump up, and nothing below the destination: mid-air.
        assert!(!Waypoints::reachable(Vector::ZERO, Vector::new(0.0, 0.0, 46.0), true, &world));
    }

    #[test]
    fn goals() {
        let mut w = line(5);
        w.set_flags(3, fl::HEALTH);
        w.set_flags(4, fl::HEALTH);
        assert_eq!(w.find_nearest_goal(Vector::ZERO, 9999.0, -1, fl::HEALTH, &[]), Some(3));
        assert_eq!(w.find_nearest_goal(Vector::ZERO, 9999.0, -1, fl::HEALTH, &[3]), Some(4));
        let mut rng = XorShiftRandom::new(1);
        for _ in 0..20 {
            let g = w.find_random_goal(-1, fl::HEALTH, &[], &mut rng).unwrap();
            assert!(g == 3 || g == 4);
            let g = w.find_random_goal_near(Vector::ZERO, 350.0, -1, 0, &[], &mut rng).unwrap();
            assert!(g < 4);
        }
        assert_eq!(w.find_nearest(Vector::new(120.0, 0.0, 0.0), Vector::new(120.0, 0.0, 0.0), None, 400.0, -1, &EmptyWorld), Some(1));
        let a = w.add_aiming(Vector::ZERO, Vector::new(10.0, 90.0, 0.0)).unwrap();
        assert_eq!(w.find_nearest_aiming(Vector::new(0.0, 25.0, 0.0)), Some(a));
        assert_eq!(w.find_reachable(Vector::new(10.0, 0.0, 0.0), Vector::ZERO, None, 50.0, -1, &EmptyWorld), Some(0));
    }
}
