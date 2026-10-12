//! Compares the port with RCBot itself. tools/reference/build.sh compiles upstream's
//! `BotNavigate_AStarAlgo`, path functions and `rc_compress.cpp` + miniz into a harness and
//! records what they produce into tests/reference/; these tests replay the same inputs.

use std::path::PathBuf;

use rcbot::nav::{AStar, AStarBot, AStarQuery, AStarResult, EmptyWorld, RcwHeader, VisibilityTable, Waypoints};
use rcbot::{Vector, XorShiftRandom};

fn reference(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference").join(name)
}

fn waypoints(map: &str) -> Waypoints {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../data/rcbot/waypoints/ts/{map}.rcw"));
    let bytes = std::fs::read(p).unwrap();
    Waypoints::from_rcw(&bytes, &RcwHeader::parse(&bytes).unwrap().map_name()).unwrap().0
}

fn f32_hex(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

#[test]
fn astar_matches_upstream() {
    let mut checked = 0;
    for map in ["ts_dojo", "ts_lobby", "mecklenburg_b5", "tm_avana", "crossfire", "ahl_museum"] {
        let mut w = waypoints(map);
        let text = std::fs::read_to_string(reference(&format!("astar_{map}.txt"))).unwrap();
        for (n, line) in text.lines().enumerate() {
            let (head, path) = line.split_once('|').unwrap();
            let f: Vec<&str> = head.split_whitespace().collect();
            let q = AStarQuery {
                from: Some(f[0].parse().unwrap()),
                to: Some(f[1].parse().unwrap()),
                team: f[2].parse().unwrap(),
                gun_position: Vector::new(f32_hex(f[3]), f32_hex(f[4]), f32_hex(f[5])),
                max_loops: f[6].parse().unwrap(),
                time: 0.0,
            };
            let want: i32 = f[7].parse().unwrap();
            let want_frames: i32 = f[8].parse().unwrap();
            let want_path: Vec<usize> = path.split_whitespace().map(|p| p.parse().unwrap()).collect();

            let mut bot = AStarBot::default();
            let mut astar = AStar::new();
            let mut rng = XorShiftRandom::new(1);
            let mut r = astar.search(&mut bot, &q, false, &mut w, &EmptyWorld, &mut rng);
            let mut frames = 1;
            while r == AStarResult::Continue && frames < 1000 {
                r = astar.search(&mut bot, &q, true, &mut w, &EmptyWorld, &mut rng);
                frames += 1;
            }
            let got_path: Vec<usize> = bot.bot_paths.iter().rev().copied().collect();
            assert_eq!((r.to_upstream(), frames, &got_path), (want, want_frames, &want_path), "{map} line {}: {line}", n + 1);
            checked += 1;
        }
    }
    assert_eq!(checked, 720);
}

/// The harness's synthetic visibility pattern.
fn synthetic_visible(i: u32, j: u32) -> bool {
    let mut h = i.wrapping_mul(73856093) ^ j.wrapping_mul(19349663);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    h.is_multiple_of(3)
}

#[test]
fn rcv_files_match_upstream_byte_for_byte() {
    for n in [154usize, 791] {
        let mut t = VisibilityTable::new();
        for i in 0..n {
            for j in 0..n {
                t.set(i, j, synthetic_visible(i as u32, j as u32));
            }
        }
        let want = std::fs::read(reference(&format!("vis{n}.rcz"))).unwrap();
        assert!(t.to_rcv(n).unwrap() == want, "vis{n}.rcz differs");
        let mut back = VisibilityTable::new();
        back.read_rcv(&want, n).unwrap();
        for i in 0..VisibilityTable::rows_in_file(n) {
            for j in 0..n {
                assert_eq!(back.get(i, j), synthetic_visible(i as u32, j as u32));
            }
        }
    }
}
