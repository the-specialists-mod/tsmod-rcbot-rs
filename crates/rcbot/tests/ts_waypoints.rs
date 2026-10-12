//! The real TS waypoints (data/rcbot/waypoints/ts, from 2cwldys/ts_h9_team_elimination): every
//! file loads and saves back byte for byte, its graph is sane, and A* finds sensible paths.

use std::collections::BinaryHeap;
use std::path::PathBuf;

use rcbot::nav::astar::DEFAULT_PATH_REVS;
use rcbot::nav::{AStar, AStarBot, AStarQuery, AStarResult, EmptyWorld, LoadError, MAX_WAYPOINTS, RcwHeader, Waypoints, flags as fl};
use rcbot::{Random, XorShiftRandom};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/rcbot/waypoints/ts")
}

fn files() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rcw"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap()))
        .collect();
    out.sort();
    out
}

fn load(bytes: &[u8]) -> (Waypoints, RcwHeader) {
    let header = RcwHeader::parse(bytes).unwrap();
    Waypoints::from_rcw(bytes, &header.map_name()).unwrap()
}

#[test]
fn all_44_files_round_trip_byte_exact() {
    let files = files();
    assert_eq!(files.len(), 44);
    for (name, bytes) in &files {
        let (w, header) = load(bytes);
        assert_eq!(w.len() as i32, header.num_waypoints, "{name}");
        let saved = w.to_rcw(&header.map_name(), !header.needs_visibility());
        assert!(saved == *bytes, "{name}: save differs from the file");
    }
}

#[test]
fn map_name_must_match_like_upstream() {
    for (name, bytes) in files() {
        let r = Waypoints::from_rcw(&bytes, &name);
        if name == "ts_hammertime_d1" {
            // The file says ts_hammertime2_d1: upstream refuses it on ts_hammertime_d1 too.
            assert_eq!(r.unwrap_err(), LoadError::WrongMap { file: "ts_hammertime2_d1".into() });
        } else {
            r.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}

#[test]
fn graphs_are_sane() {
    for (name, bytes) in files() {
        let (w, _) = load(&bytes);
        let mut touching_deleted = 0;
        for (from, to) in w.all_paths() {
            assert!((0..w.len() as i16).contains(&to), "{name}: path {from} -> {to}");
            if (w.flags(from as i32) | w.flags(i32::from(to))) & fl::DELETED != 0 {
                touching_deleted += 1;
            }
        }
        // Upstream's auto-path links new waypoints to deleted slots (which sit at the origin), and
        // A* doesn't skip them. One shipped file shows it: waypoint 99 of ahl_museum.
        let expected = if name == "ahl_museum" { 16 } else { 0 };
        assert_eq!(touching_deleted, expected, "{name}: paths from or to deleted waypoints");
        for (i, wp) in w.as_slice().iter().enumerate() {
            if wp.flags & fl::DELETED == 0 {
                for c in wp.origin.to_array() {
                    assert!(c.abs() <= 4096.0, "{name}: waypoint {i} at {:?}", wp.origin);
                }
            }
        }
    }
}

/// The edges A* may take from `from`, by upstream's rules for TS with team -1 (no self paths, no
/// unreachable waypoints, fly waypoints only with the stunt bit; deleted waypoints are *not*
/// skipped, as upstream doesn't), with their costs (free from a teleport).
fn edges(w: &Waypoints, from: usize) -> Vec<(usize, f32)> {
    let teleport = w.flags(from as i32) & fl::TELEPORT != 0;
    w.successors(from, -1)
        .into_iter()
        .filter(|&s| s != from)
        .filter(|&s| {
            let f = w.flags(s as i32);
            f & fl::UNREACHABLE == 0 && (f & fl::FLY == 0 || f & fl::STUNT != 0)
        })
        .map(|s| (s, if teleport { 0.0 } else { (w.origin(s as i32) - w.origin(from as i32)).length() }))
        .collect()
}

/// Dijkstra over [`edges`]: the optimal cost, if `to` can be reached.
fn dijkstra(w: &Waypoints, from: usize, to: usize) -> Option<f32> {
    #[derive(PartialEq)]
    struct E(f32, usize);
    impl Eq for E {}
    impl PartialOrd for E {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for E {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            o.0.total_cmp(&self.0)
        }
    }
    let mut best = vec![f32::INFINITY; MAX_WAYPOINTS];
    let mut heap = BinaryHeap::from([E(0.0, from)]);
    best[from] = 0.0;
    while let Some(E(c, n)) = heap.pop() {
        if n == to {
            return Some(c);
        }
        if c > best[n] {
            continue;
        }
        for (s, e) in edges(w, n) {
            if c + e < best[s] {
                best[s] = c + e;
                heap.push(E(c + e, s));
            }
        }
    }
    None
}

#[test]
fn astar_paths_on_real_maps() {
    let mut rng = XorShiftRandom::new(0x5eed);
    let (mut total, mut found, mut worst) = (0, 0, 1.0f32);
    let mut ratio_sum = 0.0f64;
    for (name, bytes) in files() {
        let (mut w, _) = load(&bytes);
        let live: Vec<usize> = (0..w.len()).filter(|&i| w.flags(i as i32) & (fl::DELETED | fl::AIMING | fl::UNREACHABLE) == 0).collect();
        if live.len() < 2 {
            continue;
        }
        for _ in 0..40 {
            let from = live[rng.random_long(0, live.len() as i32 - 1) as usize];
            let to = live[rng.random_long(0, live.len() as i32 - 1) as usize];
            if from == to {
                continue;
            }
            total += 1;
            let q = AStarQuery {
                from: Some(from),
                to: Some(to),
                gun_position: w.origin(from as i32),
                team: -1,
                max_loops: DEFAULT_PATH_REVS,
                time: 0.0,
            };
            let mut bot = AStarBot::default();
            let mut astar = AStar::new();
            let mut r = astar.search(&mut bot, &q, false, &mut w, &EmptyWorld, &mut rng);
            let mut frames = 1;
            while r == AStarResult::Continue {
                r = astar.search(&mut bot, &q, true, &mut w, &EmptyWorld, &mut rng);
                frames += 1;
                assert!(frames < 100, "{name}: {from} -> {to} never ends");
            }
            let optimum = dijkstra(&w, from, to);
            match r {
                AStarResult::Next(next) => {
                    found += 1;
                    let opt = optimum.unwrap_or_else(|| panic!("{name}: A* found {from} -> {to}, Dijkstra didn't"));
                    // The stack holds the path, next waypoint on top, goal at the bottom.
                    let mut path: Vec<usize> = bot.bot_paths.iter().rev().copied().collect();
                    assert_eq!(path[0], next);
                    assert_eq!(*path.last().unwrap(), to);
                    path.insert(0, from);
                    let mut cost = 0.0;
                    for pair in path.windows(2) {
                        let e = edges(&w, pair[0]).into_iter().find(|e| e.0 == pair[1]);
                        let (_, c) = e.unwrap_or_else(|| panic!("{name}: {} -> {} is not an edge", pair[0], pair[1]));
                        cost += c;
                    }
                    assert!(cost >= opt - 0.01, "{name}: cheaper than optimal?");
                    let ratio = if opt > 0.0 { cost / opt } else { 1.0 };
                    ratio_sum += f64::from(ratio);
                    worst = worst.max(ratio);
                    // Upstream's heuristic (bot distance + goal distance) is not admissible, so
                    // paths are not always shortest; they are never absurd.
                    assert!(ratio < 2.0, "{name}: {from} -> {to} costs {cost}, optimum {opt}");
                }
                AStarResult::NoPath => assert!(optimum.is_none(), "{name}: A* missed {from} -> {to}"),
                AStarResult::Continue => unreachable!(),
            }
        }
    }
    eprintln!("A*: {found}/{total} pairs connected, mean cost/optimum {:.4}, worst {worst:.4}", ratio_sum / f64::from(found.max(1)));
    assert!(found * 10 > total * 8, "most random pairs should connect ({found}/{total})");
}
