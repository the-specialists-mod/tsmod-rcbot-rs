//! MegaHAL against RCBot's own: `tools/reference/hal_harness.cpp` runs upstream's whole
//! `megahal.cpp` (32-bit, as shipped) on the fixture in `tests/reference/hal/` with the same random
//! generator as [`XorShiftRandom`], and records the brains it writes, every chat reply (before and
//! after `HumanizeString`) and `RemoveNameTags` on a list of names. This test replays the run.

use std::path::PathBuf;

use rcbot::XorShiftRandom;
use rcbot::chat::bot_chat::bot_chat_reply;
use rcbot::chat::text::{humanize_string, remove_name_tags};
use rcbot::chat::{HalBot, ReplyParams, brain};

/// `SEED` in the harness: one sequence for the whole run.
const SEED: u64 = 20260613;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference/hal")
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn hex(s: &[u8]) -> String {
    if s.is_empty() { "-".to_string() } else { s.iter().map(|b| format!("{b:02x}")).collect() }
}

fn lines(bytes: &[u8]) -> Vec<&[u8]> {
    // The harness reads lines up to each '\n' and drops an unterminated last one.
    let mut v: Vec<&[u8]> = bytes.split(|&c| c == b'\n').collect();
    v.pop();
    v
}

#[test]
fn megahal_matches_upstream() {
    let rng = &mut XorShiftRandom::new(SEED);
    let (ban, aux, swp, trn) = (read("hal.ban"), read("hal.aux"), read("hal.swp"), read("hal.trn"));

    // Profile 1: no brain, so an empty one is written, loaded and pretrained.
    assert!(!brain::cookie_is_valid(None));
    let mut hal = HalBot::new(Some(&ban), Some(&aux), Some(&swp));
    let empty = hal.save_brain();
    assert_eq!(empty, read("hal_empty.brn"), "empty brain");
    hal.load_brain(&empty).unwrap();
    hal.pretrain(Some(&trn));
    assert_eq!(hal.save_brain(), read("hal_trained.brn"), "pretrained brain");

    let mut replies = String::new();
    let chat = read("chat.txt");
    for l in lines(&chat) {
        let f: Vec<&[u8]> = l.splitn(4, |&c| c == b'\t').collect();
        let params = ReplyParams {
            bot_name: b"[RC]Neo",
            sender_name: f[0],
            sender_is_self: false,
            sender_is_bot: f[1] == b"1",
            sender_rep: std::str::from_utf8(f[2]).unwrap().parse().unwrap(),
            num_clients: 3,
            chatting: true,
            real_mode: true,
            dont_learn: false,
            reply_percent: 66,
        };
        let mut msg = f[3].to_vec();
        let said = bot_chat_reply(&mut hal, &mut msg, &params, rng).unwrap_or_default();
        let mut human = said.clone();
        if !human.is_empty() {
            humanize_string(&mut human, rng);
        }
        replies += &format!("R {} {} {}\n", hex(&msg), hex(&said), hex(&human));
    }
    let theirs = String::from_utf8(read("hal_replies.txt")).unwrap();
    for (n, (a, b)) in replies.lines().zip(theirs.lines()).enumerate() {
        assert_eq!(a, b, "chat line {}", n + 1);
    }
    assert_eq!(replies.lines().count(), theirs.lines().count());
    assert!(theirs.lines().filter(|l| !l.ends_with(" - -")).count() >= 3, "the fixture makes the bot talk");
    let after_chat = hal.save_brain();
    assert_eq!(after_chat, read("hal_after_chat.brn"), "brain after chatting");

    // Profile 2 wakes up with that brain (no lists, no training) and saves it again.
    assert!(brain::cookie_is_valid(Some(&after_chat)));
    let mut p2 = HalBot::new(None, None, None);
    p2.load_brain(&after_chat).unwrap();
    assert_eq!(p2.save_brain(), read("hal_reloaded.brn"), "reloaded brain");

    let mut names = String::new();
    let list = read("names.txt");
    for n in lines(&list) {
        names += &format!("N {} {}\n", hex(n), hex(&remove_name_tags(n)));
    }
    assert_eq!(names, String::from_utf8(read("hal_names.txt")).unwrap());
}
