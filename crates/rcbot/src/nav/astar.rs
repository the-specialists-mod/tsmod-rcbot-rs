//! A* over the waypoint graph (`BotNavigate_AStarAlgo`, `AStarNode`, `CBotFailedPaths`;
//! `bot_navigate.cpp`, `bot.h`), with upstream's quirks kept because bots behave by them:
//!
//! * the heuristic is distance from the **bot** (its gun position) **plus** distance to the goal,
//!   set once per node; leaving a teleport waypoint costs nothing and its successors' heuristic is
//!   the goal distance only;
//! * the open list is an unsorted array searched for the first strictly lowest `cost + heuristic`;
//! * a successor equal to the waypoint expanded just before is skipped ("going backwards"), and a
//!   path from a waypoint to itself is deleted from the graph;
//! * a node already seen is only updated when it has a parent and its old cost is not lower;
//! * the search runs at most `max_loops` expansions per call (the profile's `m_iPathRevs`) and
//!   then asks to be continued next frame.
//!
//! TS rules from the mod branches: stunt (wall-stick) waypoints are usable, fly waypoints only
//! when they are stunt waypoints too, crouch waypoints always.

use super::flags as fl;
use super::waypoints::{PathCursor, PathRef, Waypoints};
use super::{Glass, MAX_WAYPOINTS, Monsters, World};
use crate::{Random, Vector};

/// `gBotGlobals.m_iMaxPathRevs` when unset (`globals.cpp`): the default `m_iPathRevs`.
pub const DEFAULT_PATH_REVS: i32 = 150;

const FL_OPEN: u8 = 1 << 0;
const FL_CLOSED: u8 = 1 << 1;
const FL_PARENT: u8 = 1 << 2;
const FL_HEURISTIC: u8 = 1 << 3;

/// `AStarNode`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AStarNode {
    waypoint: i32,
    cost: f32,
    heuristic: f32,
    parent: i16,
    flags: u8,
}

impl Default for AStarNode {
    fn default() -> Self {
        AStarNode { waypoint: -1, cost: 0.0, heuristic: 0.0, parent: 0, flags: 0 }
    }
}

impl AStarNode {
    /// `setHeuristic`.
    fn set_heuristic(&mut self, bot_dist: f32, goal_dist: f32, teleport: bool) {
        self.heuristic = if teleport { goal_dist } else { bot_dist + goal_dist };
        self.flags |= FL_HEURISTIC;
    }

    fn has(&self, f: u8) -> bool {
        self.flags & f == f
    }

    /// `precedes`: lower `cost + heuristic` first.
    fn precedes(&self, b: &AStarNode) -> bool {
        self.cost + self.heuristic < b.cost + b.heuristic
    }

    /// `m_fCost`.
    pub fn cost(&self) -> f32 {
        self.cost
    }
}

/// `CBotFailedPaths`: path nodes ([`PathRef`]) the bot failed to walk and A* avoids. A* marks each
/// one it skips as violated; when a search fails, violated entries are forgotten so the next
/// search may try them again.
#[derive(Clone, Debug, Default)]
pub struct FailedPaths {
    // dataStack: the top is the end.
    paths: Vec<(PathRef, bool)>,
}

impl FailedPaths {
    /// `Violate`: is `p` failed? Marks it violated if so.
    pub fn violate(&mut self, p: PathRef) -> bool {
        if let Some(e) = self.paths.iter_mut().rev().find(|e| e.0 == p) {
            e.1 = true;
            return true;
        }
        false
    }

    /// `ClearViolatedPaths`: forgets the violated ones; true if there were any.
    pub fn clear_violated(&mut self) -> bool {
        let before = self.paths.len();
        self.paths.retain(|e| !e.1);
        self.paths.len() != before
    }

    /// `AddFailedPath`.
    pub fn add(&mut self, p: PathRef) {
        if !self.paths.iter().any(|e| e.0 == p) {
            self.paths.push((p, false));
        }
    }

    /// Whether `p` is failed (without marking it).
    pub fn contains(&self, p: PathRef) -> bool {
        self.paths.iter().any(|e| e.0 == p)
    }

    /// `ClearFailedPaths`.
    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// No entries.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// What A* reads and writes on the bot (`CBot` members).
#[derive(Clone, Debug, Default)]
pub struct AStarBot {
    /// `m_stFailedPaths`.
    pub failed_paths: FailedPaths,
    /// `m_FailedGoals`, a stack (top = end): goals A* couldn't reach, ignored as random goals.
    pub failed_goals: Vec<usize>,
    /// `m_fNextClearFailedGoals`.
    pub next_clear_failed_goals: f32,
    /// `m_stBotPaths`, a stack (top = end): the found path, next waypoint on top, goal at the
    /// bottom, without the start waypoint.
    pub bot_paths: Vec<usize>,
    /// Goals no path was found to, and until when they are left alone ([`SearchOptions::back_off_failed_goals`];
    /// port addition, design §4). Upstream's `failed_goals` are all forgotten 5-10 s after a
    /// failure, and a second failure of the same goal took it off the list again.
    pub backed_off_goals: Vec<(usize, f32)>,
}

impl AStarBot {
    /// `dataStack::Remove`: removes the topmost copy.
    fn remove_failed_goal(&mut self, wpt: usize) {
        if let Some(p) = self.failed_goals.iter().rposition(|&g| g == wpt) {
            self.failed_goals.remove(p);
        }
    }

    /// The goals to leave alone at `now`: upstream's failed goals and those still backed off.
    pub fn ignored_goals(&self, now: f32) -> Vec<usize> {
        let mut out = self.failed_goals.clone();
        out.extend(self.backed_off_goals.iter().filter(|&&(_, until)| until > now).map(|&(g, _)| g));
        out
    }

    /// Forgets backed-off goals whose time is up.
    pub fn expire_backed_off_goals(&mut self, now: f32) {
        self.backed_off_goals.retain(|&(_, until)| until > now);
    }
}

/// How dangerous each waypoint is: what the learned heat map (deaths, damage taken) provides
/// for [`SearchOptions::danger`]. 0 is safe; a value of `d` makes an edge into the waypoint cost
/// `1 + weight * d` times its length.
pub trait DangerSource {
    /// The danger at `waypoint` (0 or more; 0 by default).
    fn danger(&self, waypoint: usize) -> f32 {
        let _ = waypoint;
        0.0
    }
}

/// No danger anywhere.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDanger;

impl DangerSource for NoDanger {}

/// What a search may do beyond upstream's `BotNavigate_AStarAlgo`. [`SearchOptions::UPSTREAM`] is
/// upstream exactly (the reference tests compare it bit for bit).
#[derive(Clone, Copy)]
pub struct SearchOptions<'a> {
    /// A goal no path reaches stays failed, and is also backed off for 30-60 s
    /// ([`AStarBot::backed_off_goals`]); upstream toggles it off the list on a second failure.
    pub back_off_failed_goals: bool,
    /// Tactical costs; `None`: the edge cost is its length, with upstream's heuristic.
    pub tactics: Option<Tactics<'a>>,
}

impl SearchOptions<'_> {
    /// Upstream's search.
    pub const UPSTREAM: SearchOptions<'static> = SearchOptions { back_off_failed_goals: false, tactics: None };
}

/// The tactical costs of a search (design §4): an edge into waypoint `w` costs its length times
/// `1 + danger_weight * danger(w) + exposure_weight * exposure(w) + crowd_weight * crowded(w)`,
/// where `exposure` is the summed weight of the threats (waypoints enemies were seen at) that can
/// see `w` (the visibility table) and `crowded` is 1 on a teammate's path. Every factor is at
/// least 1, so the heuristic is the straight distance to the goal alone (admissible, except
/// across teleports, which cost nothing).
#[derive(Clone, Copy)]
pub struct Tactics<'a> {
    /// The danger per waypoint.
    pub danger: &'a dyn DangerSource,
    /// How much danger counts.
    pub danger_weight: f32,
    /// Waypoints enemies were seen at, with a weight each (newer, more).
    pub threats: &'a [(usize, f32)],
    /// How much being seen from a threat counts.
    pub exposure_weight: f32,
    /// Waypoints on teammates' paths (sorted, for `binary_search`).
    pub crowded: &'a [usize],
    /// How much a teammate's path counts.
    pub crowd_weight: f32,
}

impl Tactics<'_> {
    /// The cost factor of an edge into `w` (1 or more).
    fn factor(&self, waypoints: &Waypoints, w: usize) -> f32 {
        let mut f = 1.0 + self.danger_weight * self.danger.danger(w).max(0.0);
        if self.exposure_weight > 0.0 {
            let seen: f32 = self.threats.iter().filter(|&&(t, _)| t == w || waypoints.visibility.get(t, w)).map(|&(_, wt)| wt).sum();
            f += self.exposure_weight * seen;
        }
        if self.crowd_weight > 0.0 && self.crowded.binary_search(&w).is_ok() {
            f += self.crowd_weight;
        }
        f
    }
}

/// One search request.
#[derive(Clone, Copy, Debug)]
pub struct AStarQuery {
    /// The start waypoint (`iFrom`), `None` for -1.
    pub from: Option<usize>,
    /// The goal waypoint (`iTo`), `None` for -1.
    pub to: Option<usize>,
    /// `pBot->GetGunPosition()`, which `DistanceFrom` measures from.
    pub gun_position: Vector,
    /// `pBot->GetTeam()` (-1 = any).
    pub team: i32,
    /// `m_Profile.m_iPathRevs`: expansions per call.
    pub max_loops: i32,
    /// `gpGlobals->time`.
    pub time: f32,
}

/// The result of `BotNavigate_AStarAlgo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AStarResult {
    /// The first waypoint after the start; [`AStarBot::bot_paths`] holds the whole path.
    Next(usize),
    /// -1: no path (or invalid waypoints, or start == goal, which upstream reports this way too).
    NoPath,
    /// -2: out of expansions; call again with `continue_search` to carry on.
    Continue,
}

impl AStarResult {
    /// Upstream's return value.
    pub fn to_upstream(self) -> i32 {
        match self {
            AStarResult::Next(i) => i as i32,
            AStarResult::NoPath => -1,
            AStarResult::Continue => -2,
        }
    }
}

/// The search state a bot keeps between frames (`CBot::aPathsFound`, `CBot::sOpenList`).
#[derive(Clone)]
pub struct AStar {
    nodes: Vec<AStarNode>,
    open: Vec<usize>,
}

impl Default for AStar {
    fn default() -> Self {
        AStar::new()
    }
}

impl std::fmt::Debug for AStar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AStar").field("open", &self.open.len()).finish_non_exhaustive()
    }
}

impl AStar {
    /// A fresh search state.
    pub fn new() -> AStar {
        AStar { nodes: vec![AStarNode::default(); MAX_WAYPOINTS], open: Vec::new() }
    }

    /// The node info of a waypoint (`aPathsFound[i]`).
    pub fn node(&self, index: usize) -> Option<&AStarNode> {
        self.nodes.get(index)
    }

    /// `BotNavigate_AStarAlgo(pBot, iFrom, iTo, bContinue)`. It may delete self-paths from
    /// `waypoints`, as upstream does. `world` traces `W_FL_OPENS_LATER` paths; `rng` picks when
    /// failed goals are next cleared.
    pub fn search(
        &mut self,
        bot: &mut AStarBot,
        q: &AStarQuery,
        continue_search: bool,
        waypoints: &mut Waypoints,
        world: &dyn World,
        rng: &mut dyn Random,
    ) -> AStarResult {
        self.search_with(bot, q, continue_search, waypoints, world, rng, &SearchOptions::UPSTREAM)
    }

    /// [`AStar::search`] with the port's [`SearchOptions`]; with [`SearchOptions::UPSTREAM`] it is
    /// upstream's search. A search continued next frame must pass the same options.
    #[allow(clippy::too_many_arguments)] // upstream's arguments and the options
    pub fn search_with(
        &mut self,
        bot: &mut AStarBot,
        q: &AStarQuery,
        continue_search: bool,
        waypoints: &mut Waypoints,
        world: &dyn World,
        rng: &mut dyn Random,
        opts: &SearchOptions<'_>,
    ) -> AStarResult {
        let (Some(from), Some(to)) = (q.from, q.to) else { return AStarResult::NoPath };
        if from >= MAX_WAYPOINTS || to >= MAX_WAYPOINTS {
            return AStarResult::NoPath;
        }
        let tactics = opts.tactics;
        // Upstream's heuristic adds the distance from the bot (not admissible); tactical costs
        // are never below the length, so the goal distance alone is.
        let dist_from_bot = |v: Vector| if tactics.is_some() { 0.0 } else { (v - q.gun_position).length() };

        if !continue_search {
            self.nodes.fill(AStarNode::default());
            self.open.clear();
            let from_origin = waypoints.origin(from as i32);
            let node = &mut self.nodes[from];
            node.flags |= FL_OPEN;
            node.waypoint = from as i32;
            node.set_heuristic(dist_from_bot(from_origin), (from_origin - waypoints.origin(to as i32)).length(), false);
            self.open.push(from);
        }

        // TS: bBotCanUseFlyWpt is false (NS jetpacks, Op4 grapple, DMC only), and
        // bBotCanUseWallStickWpt is true.
        let can_fly = false;
        let can_wall_stick = true;
        let goal_origin = waypoints.origin(to as i32);

        let mut loops = 0;
        let mut found = false;
        let mut prev: Option<usize> = None;
        while !found && !self.open.is_empty() && loops < q.max_loops {
            loops += 1;
            let mut best = 0;
            for i in 1..self.open.len() {
                if self.nodes[self.open[i]].precedes(&self.nodes[self.open[best]]) {
                    best = i;
                }
            }
            let current = self.open.remove(best);
            self.nodes[current].flags &= !FL_OPEN;
            found = current == to;
            if found {
                break;
            }
            bot.remove_failed_goal(current);

            let cur_wpt = waypoints.get(current).copied().unwrap_or_default();
            let mut delete_loop_path = false;
            let mut cursor = PathCursor::new(current);
            while let Some(succ) = waypoints.find_path(&mut cursor, q.team) {
                if succ == current {
                    delete_loop_path = true;
                    continue;
                }
                if prev == Some(succ) {
                    continue; // going backwards
                }
                if bot.failed_paths.violate(cursor.path_ref()) {
                    continue;
                }
                let succ_flags = waypoints.flags(succ as i32);
                if !fl::team_allows(succ_flags, q.team) || succ_flags & fl::UNREACHABLE != 0 {
                    continue;
                }
                if succ_flags & fl::FLY != 0 && !can_fly && !(succ_flags & fl::WALL_STICK != 0 && can_wall_stick) {
                    continue;
                }
                let succ_origin = waypoints.origin(succ as i32);
                if succ_flags & fl::OPENS_LATER != 0
                    && world.trace_line(cur_wpt.origin, succ_origin, Monsters::Ignore, Glass::DontIgnore, None).fraction < 1.0
                {
                    continue;
                }

                let mut cur_cost = self.nodes[current].cost;
                let teleport = cur_wpt.flags & fl::TELEPORT == fl::TELEPORT;
                if !teleport {
                    match &tactics {
                        None => cur_cost += (succ_origin - cur_wpt.origin).length(),
                        Some(t) => cur_cost += (succ_origin - cur_wpt.origin).length() * t.factor(waypoints, succ),
                    }
                }

                let node = &mut self.nodes[succ];
                // Seen before: keep it unless it has a parent and its old cost isn't lower.
                if (node.has(FL_OPEN) || node.has(FL_CLOSED)) && (!node.has(FL_PARENT) || node.cost < cur_cost) {
                    continue;
                }
                node.flags &= !FL_CLOSED;
                node.parent = current as i16;
                node.flags |= FL_PARENT;
                node.cost = cur_cost;
                node.waypoint = succ as i32;
                if !node.has(FL_HEURISTIC) {
                    node.set_heuristic(dist_from_bot(succ_origin), (goal_origin - succ_origin).length(), teleport);
                }
                if !node.has(FL_OPEN) {
                    self.open.push(succ);
                    node.flags |= FL_OPEN;
                }
            }
            self.nodes[current].flags |= FL_CLOSED;
            prev = Some(current);
            if delete_loop_path {
                waypoints.delete_path(current, current as i16);
            }
        }

        // Note: this also reports "continue" when the open list ran dry on the last allowed loop.
        if !found && loops >= q.max_loops {
            return AStarResult::Continue;
        }
        self.open.clear();

        if !found {
            if !bot.failed_paths.clear_violated() {
                if !bot.failed_goals.contains(&to) {
                    bot.failed_goals.push(to);
                    bot.next_clear_failed_goals = q.time + rng.random_float(5.0, 10.0);
                } else if !opts.back_off_failed_goals {
                    // Upstream: a second failure takes the goal off the list again.
                    bot.remove_failed_goal(to);
                }
                if opts.back_off_failed_goals {
                    let until = q.time + rng.random_float(30.0, 60.0);
                    bot.backed_off_goals.retain(|&(g, _)| g != to);
                    bot.backed_off_goals.push((to, until));
                }
            }
            return AStarResult::NoPath;
        }

        bot.bot_paths.clear();
        let mut current = to as i32;
        let mut next = None;
        let mut loops = 0;
        while current != -1 && current != from as i32 && loops < MAX_WAYPOINTS {
            loops += 1;
            bot.bot_paths.push(current as usize);
            next = Some(current as usize);
            current = i32::from(self.nodes[current as usize].parent);
        }
        if loops == MAX_WAYPOINTS {
            bot.bot_paths.clear();
            return AStarResult::NoPath;
        }
        next.map_or(AStarResult::NoPath, AStarResult::Next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;
    use crate::nav::EmptyWorld;

    /// A 3×3 grid, 100 units apart, connected both ways along rows and columns.
    fn grid() -> Waypoints {
        let mut w = Waypoints::new();
        for y in 0..3 {
            for x in 0..3 {
                w.add_origin(Vector::new(x as f32 * 100.0, y as f32 * 100.0, 0.0), 0, None);
            }
        }
        for i in 0..9usize {
            let (x, y) = (i % 3, i / 3);
            if x < 2 {
                w.add_path(i, (i + 1) as i16);
                w.add_path(i + 1, i as i16);
            }
            if y < 2 {
                w.add_path(i, (i + 3) as i16);
                w.add_path(i + 3, i as i16);
            }
        }
        w
    }

    fn query(from: usize, to: usize) -> AStarQuery {
        AStarQuery { from: Some(from), to: Some(to), gun_position: Vector::ZERO, team: -1, max_loops: DEFAULT_PATH_REVS, time: 0.0 }
    }

    fn run(w: &mut Waypoints, bot: &mut AStarBot, q: AStarQuery) -> AStarResult {
        AStar::new().search(bot, &q, false, w, &EmptyWorld, &mut XorShiftRandom::new(1))
    }

    #[test]
    fn finds_a_path() {
        let mut w = grid();
        let mut bot = AStarBot::default();
        let r = run(&mut w, &mut bot, query(0, 8));
        let AStarResult::Next(n) = r else { panic!("{r:?}") };
        assert!(n == 1 || n == 3);
        // Stack: next on top, goal at the bottom, four steps on a grid.
        assert_eq!(bot.bot_paths.len(), 4);
        assert_eq!(bot.bot_paths[0], 8);
        assert_eq!(*bot.bot_paths.last().unwrap(), n);
    }

    #[test]
    fn start_is_goal_reports_no_path() {
        let mut w = grid();
        assert_eq!(run(&mut w, &mut AStarBot::default(), query(4, 4)), AStarResult::NoPath);
    }

    #[test]
    fn unreachable_goal_is_remembered() {
        let mut w = grid();
        w.add_origin(Vector::new(1000.0, 0.0, 0.0), 0, None); // 9, no paths
        let mut bot = AStarBot::default();
        assert_eq!(run(&mut w, &mut bot, query(0, 9)), AStarResult::NoPath);
        assert_eq!(bot.failed_goals, vec![9]);
        assert!(bot.next_clear_failed_goals >= 5.0 && bot.next_clear_failed_goals < 10.0);
        // A second failure takes it off again (upstream toggles).
        assert_eq!(run(&mut w, &mut bot, query(0, 9)), AStarResult::NoPath);
        assert!(bot.failed_goals.is_empty());
    }

    #[test]
    fn backed_off_goals_stay_failed() {
        let mut w = grid();
        w.add_origin(Vector::new(1000.0, 0.0, 0.0), 0, None); // 9, no paths
        let mut bot = AStarBot::default();
        let opts = SearchOptions { back_off_failed_goals: true, tactics: None };
        let mut search = |bot: &mut AStarBot, time: f32| {
            let q = AStarQuery { time, ..query(0, 9) };
            AStar::new().search_with(bot, &q, false, &mut w, &EmptyWorld, &mut XorShiftRandom::new(1), &opts)
        };
        assert_eq!(search(&mut bot, 100.0), AStarResult::NoPath);
        assert_eq!(search(&mut bot, 101.0), AStarResult::NoPath);
        // Still failed after the second failure, and backed off for 30-60 s from the last.
        assert_eq!(bot.failed_goals, vec![9]);
        assert_eq!(bot.backed_off_goals.len(), 1);
        let until = bot.backed_off_goals[0].1;
        assert!((131.0..161.0).contains(&until), "{until}");
        bot.failed_goals.clear(); // what the bot's periodic clear does
        assert_eq!(bot.ignored_goals(120.0), vec![9]);
        assert!(bot.ignored_goals(until + 1.0).is_empty());
    }

    /// A danger source for tests: listed waypoints have danger 1.
    struct Hot(Vec<usize>);
    impl DangerSource for Hot {
        fn danger(&self, w: usize) -> f32 {
            if self.0.contains(&w) { 1.0 } else { 0.0 }
        }
    }

    fn path_of(bot: &AStarBot) -> Vec<usize> {
        bot.bot_paths.iter().rev().copied().collect()
    }

    #[test]
    fn tactical_costs_avoid_danger_exposure_and_teammates() {
        // 0 -> 8 on the grid: through the middle row or around it.
        let tactical = |danger: &dyn DangerSource, threats: &[(usize, f32)], crowded: &[usize], w: &mut Waypoints| {
            let t = Tactics { danger, danger_weight: 4.0, threats, exposure_weight: 4.0, crowded, crowd_weight: 4.0 };
            let opts = SearchOptions { back_off_failed_goals: true, tactics: Some(t) };
            let mut bot = AStarBot::default();
            let r = AStar::new().search_with(&mut bot, &query(0, 8), false, w, &EmptyWorld, &mut XorShiftRandom::new(1), &opts);
            assert!(matches!(r, AStarResult::Next(_)), "{r:?}");
            path_of(&bot)
        };
        let mut w = grid();
        // Waypoint i is at (i % 3, i / 3); the two ways round are 0-3-6-7-8 and 0-1-2-5-8.
        let p = tactical(&Hot(vec![1, 2, 4]), &[], &[], &mut w);
        assert_eq!(p, vec![3, 6, 7, 8]);
        let p = tactical(&Hot(vec![3, 4, 6]), &[], &[], &mut w);
        assert_eq!(p, vec![1, 2, 5, 8]);
        // Exposure: an enemy seen at 6 sees 3 and 7 (and itself).
        w.visibility.set(6, 3, true);
        w.visibility.set(6, 7, true);
        let p = tactical(&NoDanger, &[(6, 1.0)], &[], &mut w);
        assert_eq!(p, vec![1, 2, 5, 8]);
        // A teammate's path through 1 and 2: the other way.
        let p = tactical(&NoDanger, &[], &[1, 2], &mut w);
        assert_eq!(p, vec![3, 6, 7, 8]);
    }

    #[test]
    fn zero_tactics_find_shortest_paths() {
        // With no danger, threats or teammates the costs are the lengths, and the goal distance
        // alone is admissible: the path is a shortest one.
        let mut w = grid();
        let t = Tactics { danger: &NoDanger, danger_weight: 1.0, threats: &[], exposure_weight: 1.0, crowded: &[], crowd_weight: 1.0 };
        let opts = SearchOptions { back_off_failed_goals: false, tactics: Some(t) };
        let mut bot = AStarBot::default();
        let mut astar = AStar::new();
        let r = astar.search_with(&mut bot, &query(0, 8), false, &mut w, &EmptyWorld, &mut XorShiftRandom::new(1), &opts);
        assert!(matches!(r, AStarResult::Next(_)));
        assert_eq!(bot.bot_paths.len(), 4);
        assert_eq!(astar.node(8).unwrap().cost(), 400.0);
    }

    #[test]
    fn continues_across_calls() {
        let mut w = grid();
        let mut bot = AStarBot::default();
        let mut astar = AStar::new();
        let mut rng = XorShiftRandom::new(1);
        let q = AStarQuery { max_loops: 2, ..query(0, 8) };
        let mut r = astar.search(&mut bot, &q, false, &mut w, &EmptyWorld, &mut rng);
        let mut calls = 1;
        while r == AStarResult::Continue {
            r = astar.search(&mut bot, &q, true, &mut w, &EmptyWorld, &mut rng);
            calls += 1;
        }
        assert!(calls > 1);
        assert!(matches!(r, AStarResult::Next(_)));
        assert_eq!(bot.bot_paths[0], 8);
    }

    #[test]
    fn failed_paths_block_their_node_and_are_cleared_on_failure() {
        let mut w = Waypoints::new();
        for x in 0..3 {
            w.add_origin(Vector::new(x as f32 * 100.0, 0.0, 0.0), 0, None);
        }
        w.add_path(0, 1);
        w.add_path(1, 2);
        let mut bot = AStarBot::default();
        bot.failed_paths.add(w.find_path_from_to(0, 1, -1).unwrap());
        assert_eq!(run(&mut w, &mut bot, query(0, 2)), AStarResult::NoPath);
        // The violated path was forgotten instead of the goal being marked failed.
        assert!(bot.failed_paths.is_empty());
        assert!(bot.failed_goals.is_empty());
        assert_eq!(run(&mut w, &mut bot, query(0, 2)), AStarResult::Next(1));
    }

    #[test]
    fn self_paths_are_deleted_and_flags_respected() {
        let mut w = grid();
        w.add_path(4, 4);
        w.set_flags(1, fl::UNREACHABLE);
        w.set_flags(3, fl::FLY);
        let mut bot = AStarBot::default();
        // 0's only neighbours (1, 3) are unusable.
        assert_eq!(run(&mut w, &mut bot, query(0, 8)), AStarResult::NoPath);
        w.set_flags(3, fl::FLY | fl::STUNT);
        assert_eq!(run(&mut w, &mut bot, query(0, 5)), AStarResult::Next(3));
        assert!(!w.successors(4, -1).contains(&4));
    }

    #[test]
    fn teams() {
        let mut w = grid();
        w.set_flags(1, fl::TEAM_SPECIFIC | 1);
        w.set_flags(3, fl::TEAM_SPECIFIC | 1);
        let mut bot = AStarBot::default();
        assert_eq!(run(&mut w, &mut bot, AStarQuery { team: 0, ..query(0, 8) }), AStarResult::NoPath);
        assert!(matches!(run(&mut w, &mut bot, AStarQuery { team: 1, ..query(0, 8) }), AStarResult::Next(1 | 3)));
    }
}
