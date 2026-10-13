//! String helpers of bot chat: `HumanizeString` and `RemoveNameTags` (`megahal.cpp`),
//! `BotFunc_FillString` (`bot.cpp`) and `strlow` (`util.cpp`).

use super::ctype::{cstr, find, is_alnum, is_upper};
use crate::Random;

/// `strlow`: ASCII lower case.
pub fn strlow(s: &mut [u8]) {
    super::ctype::strlow(s);
}

/// `HumanizeString`: typos before a bot says something. Per character: 1% to swap it with the
/// next (both then skipped), else 1% to drop it (the character moving into its place is then
/// skipped), else 2% to capitalise it at the start of a word, then 2% to lower an upper-case one.
/// The random calls are upstream's, in order.
pub fn humanize_string(string: &mut Vec<u8>, rng: &mut dyn Random) {
    const DROP_PERCENT: i32 = 1;
    const SWAP_PERCENT: i32 = 1;
    const CAPITALISE_PERCENT: i32 = 1;
    const LOWER_PERCENT: i32 = 2;

    let n = cstr(string).len();
    string.truncate(n);
    let mut i = 0;
    while i < string.len() {
        if i + 1 < string.len() && rng.random_long(0, 100) < SWAP_PERCENT {
            string.swap(i, i + 1);
            i += 2;
            continue;
        }
        // Upstream also drops a non-digit at `rand < drop_percent * 2`, but its test
        // (`c < '0' && c > '9'`) is never true.
        if rng.random_long(0, 100) < DROP_PERCENT {
            string.remove(i);
            i += 1;
            continue;
        }
        if !is_upper(string[i]) && (i == 0 || string[i - 1] == b' ') && rng.random_long(0, 100) < CAPITALISE_PERCENT * 2 {
            string[i] = string[i].to_ascii_uppercase();
        }
        if is_upper(string[i]) && rng.random_long(0, 100) < LOWER_PERCENT {
            string[i] = string[i].to_ascii_lowercase();
        }
        i += 1;
    }
}

/// `RemoveNameTags`: a player's name without clan tags, as bots say it. Drops what is inside
/// `()`, `[]`, `{}` and `-=…=-`, keeps letters and digits (l33t ones turned back into letters)
/// with one space for each run of other characters, trims trailing spaces, and lower-cases.
/// If nothing is left, the whole name (lower case). Names over 127 bytes give an empty string.
///
/// Upstream tests the second character of `-=` and `=-` at `i + i` instead of `i + 1` (§5), so
/// they only work at the start of a name; past the end of the name it reads the terminator
/// (or, further out, whatever memory follows; here, as if it were NUL).
pub fn remove_name_tags(in_string: &[u8]) -> Vec<u8> {
    let in_string = cstr(in_string);
    let length = in_string.len();
    if length > 127 {
        return Vec::new();
    }
    let at = |i: usize| in_string.get(i).copied().unwrap_or(0);
    let mut out = Vec::new();
    let mut space_allowed = false;
    let mut inside_tag = false;
    let mut i = 0;
    while i < length {
        let c = in_string[i];
        if inside_tag {
            if i + 1 < length && c == b'=' && at(i + i) == b'-' {
                inside_tag = false;
                i += 2;
                continue;
            }
            if matches!(c, b')' | b']' | b'}') {
                inside_tag = false;
            }
            i += 1;
            continue;
        }
        let current = if is_alnum(c) {
            space_allowed = true;
            c
        } else if matches!(c, b'(' | b'[' | b'{') {
            inside_tag = true;
            i += 1;
            continue;
        } else if i + 1 < length && c == b'-' && at(i + i) == b'=' {
            inside_tag = true;
            i += 2;
            continue;
        } else if space_allowed {
            space_allowed = false;
            b' '
        } else {
            i += 1;
            continue;
        };
        out.push(match current {
            b'5' => b's',
            b'0' => b'o',
            b'7' => b't',
            b'3' => b'e',
            b'1' => b'i',
            b'4' => b'a',
            other => other,
        });
        i += 1;
    }
    // Upstream trims down to (not including) the first character.
    while out.len() > 1 && out.last() == Some(&b' ') {
        out.pop();
    }
    if out.is_empty() {
        out = in_string.to_vec();
    }
    strlow(&mut out);
    out
}

/// `BotFunc_FillString`: replaces every `fill_point` in `string` with `fill_with`, searching again
/// from the start after each one. A string that changed is cut to `max_len - 1` bytes (each time,
/// and to 1023 first, upstream's scratch buffer); one without a `fill_point` is left as it is.
/// Always true (upstream's false is for a NULL `fill_with`).
///
/// If `fill_with` contains `fill_point` (or `fill_point` is empty), upstream never stops; the
/// port then searches on after what it put in (§4).
pub fn fill_string(string: &mut Vec<u8>, fill_point: &[u8], fill_with: &[u8], max_len: usize) -> bool {
    let n = cstr(string).len();
    string.truncate(n);
    if fill_point.is_empty() {
        return true;
    }
    let loops = find(fill_with, fill_point).is_some();
    let mut from = 0;
    while let Some(p) = string.get(from..).and_then(|s| find(s, fill_point)) {
        let start = from + p;
        let end = start + fill_point.len();
        let mut temp = [&string[..start], fill_with, &string[end..]].concat();
        temp.truncate(1023);
        temp.truncate(max_len.saturating_sub(1));
        *string = temp;
        from = if loops { start + fill_with.len() } else { 0 };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    /// A [`Random`] answering from a list.
    struct Script(Vec<i32>, Vec<(i32, i32)>);
    impl Random for Script {
        fn random_long(&mut self, lo: i32, hi: i32) -> i32 {
            self.1.push((lo, hi));
            self.0.remove(0)
        }
        fn random_float(&mut self, lo: f32, _: f32) -> f32 {
            lo
        }
    }

    #[test]
    fn humanize_calls() {
        // "ab": i=0 swap? no (50); drop? no (50); start of word: capitalise? yes (0) -> 'A';
        // upper: lower? no (50). i=1: no swap call (last char); drop? no; not a word start.
        let mut s = b"ab".to_vec();
        let mut r = Script(vec![50, 50, 0, 50, 50], Vec::new());
        humanize_string(&mut s, &mut r);
        assert_eq!(s, b"Ab");
        assert_eq!(r.1, [(0, 100); 5]);
        // Swap at 0 skips both; then "c" is dropped, and nothing after it is looked at.
        let mut s = b"abcd".to_vec();
        let mut r = Script(vec![0, 50, 0], Vec::new());
        humanize_string(&mut s, &mut r);
        assert_eq!(s, b"bad");
        assert!(r.0.is_empty());
    }

    #[test]
    fn humanize_seeded() {
        let mut a = b"hello there, how are you doing today? I am Fine".to_vec();
        humanize_string(&mut a, &mut XorShiftRandom::new(99));
        let mut b = b"hello there, how are you doing today? I am Fine".to_vec();
        humanize_string(&mut b, &mut XorShiftRandom::new(99));
        assert_eq!(a, b);
    }

    #[test]
    fn name_tags() {
        let t = |s: &str| String::from_utf8(remove_name_tags(s.as_bytes())).unwrap();
        assert_eq!(t("[TS]Neo"), "neo");
        assert_eq!(t("Neo(clan)"), "neo");
        assert_eq!(t("{x}L33t_H4x0r!!"), "leet hax0r".replace('0', "o"));
        assert_eq!(t("Mr. Smith"), "mr smith");
        assert_eq!(t("a!!!"), "a");
        assert_eq!(t("[***]"), "[***]");
        assert_eq!(t("Unclosed[tag"), "unclosed");
        // "-=" at 0 opens a tag (index 0 + 0 = 0 is '-', 0 + 1 ... upstream reads [i+i]):
        // at i=0, [0] is '-' and [0] must be '=' -> no tag; the l33t "7" still turns into "t".
        assert_eq!(t("-=7op=-"), "top");
        // "x-y=": at i=1, [2] is 'y', not '=' -> not a tag.
        assert_eq!(t("x-y="), "x y");
        // "ab-c=": at i=2, [4] is '=' -> a tag opens at "-c" and runs to the end.
        assert_eq!(t("ab-c="), "ab");
        assert_eq!(t(&"x".repeat(128)), "");
        assert_eq!(t(&"X".repeat(127)), "x".repeat(127));
        assert_eq!(t(""), "");
    }

    #[test]
    fn fill() {
        let mut s = b"hi %n, bye %n".to_vec();
        assert!(fill_string(&mut s, b"%n", b"bob", 128));
        assert_eq!(s, b"hi bob, bye bob");
        // Only a string that changed is cut, to max_len - 1.
        let mut s = b"0123456789".to_vec();
        fill_string(&mut s, b"%n", b"x", 5);
        assert_eq!(s, b"0123456789");
        fill_string(&mut s, b"9", b"nine", 8);
        assert_eq!(s, b"0123456");
        // BotChatReply's own use: the bot's name to "%n", cut to strlen - 1.
        let mut s = b"hey bot?".to_vec();
        let max = s.len();
        fill_string(&mut s, b"bot", b"%n", max);
        assert_eq!(s, b"hey %n?");
        let mut s = b"hey bo".to_vec();
        fill_string(&mut s, b"bo", b"%n", 6);
        assert_eq!(s, b"hey %");
        // Upstream would loop forever here.
        let mut s = b"an n".to_vec();
        fill_string(&mut s, b"n", b"%n", 64);
        assert_eq!(s, b"a%n %n");
        let mut s = b"abc".to_vec();
        fill_string(&mut s, b"", b"%n", 64);
        assert_eq!(s, b"abc");
    }
}
