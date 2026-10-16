//! The port's own learned-data files (a port addition, design §3 and §4): small versioned text
//! files, one kind per file, beside upstream's `.rld` files (which keep their byte layout).
//!
//! Each starts with a header line `rcbot-<kind> <version>`; a loader reads the versions it knows
//! and leaves its data as it was for anything else, so a damaged or newer file never breaks a
//! bot. Numbers are written as Rust prints them (the shortest text that reads back the same
//! `f32`), so a save and a load round-trip exactly. The host writes them atomically (a temporary
//! file renamed over the old one), so a crash mid-save leaves the last good file.

/// The header line of a file of `kind` at `version`, with its newline.
pub fn header(kind: &str, version: u32) -> String {
    format!("rcbot-{kind} {version}\n")
}

/// The version in `text`'s header if it is a file of `kind`, and the lines after it.
pub fn read_header<'a>(text: &'a str, kind: &str) -> Option<(u32, std::str::Lines<'a>)> {
    let mut lines = text.lines();
    let first = lines.next()?.trim();
    let rest = first.strip_prefix("rcbot-")?.strip_prefix(kind)?;
    let version = rest.strip_prefix(' ')?.trim().parse().ok()?;
    Some((version, lines))
}

/// The finite numbers of a whitespace-separated list, `None` if any isn't one.
pub fn floats<'a>(fields: impl Iterator<Item = &'a str>) -> Option<Vec<f32>> {
    fields.map(|v| v.parse::<f32>().ok().filter(|v| v.is_finite())).collect()
}

/// `values` joined by spaces, each as Rust prints it.
pub fn join(values: &[f32]) -> String {
    values.iter().map(f32::to_string).collect::<Vec<_>>().join(" ")
}

/// The name a bot's own files use: its profile id, or for a bot without a profile (id 0)
/// `default-<name>` with the name cut to letters, digits, `-` and `_` (at most 32), or plain
/// `default` when nothing is left. Upstream saved nothing for such bots (design §4).
pub fn profile_key(profile_id: i32, name: &str) -> String {
    if profile_id != 0 {
        return profile_id.to_string();
    }
    let clean: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(32).collect();
    if clean.is_empty() { "default".into() } else { format!("default-{clean}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_and_numbers_round_trip() {
        let text = format!("{}a 1 2\n", header("thing", 3));
        let (v, mut lines) = read_header(&text, "thing").unwrap();
        assert_eq!(v, 3);
        assert_eq!(lines.next(), Some("a 1 2"));
        assert!(read_header(&text, "other").is_none());
        assert!(read_header("rcbot-thingy 1", "thing").is_none());
        assert!(read_header("rcbot-thing x", "thing").is_none());
        let v = [0.1f32, -3.25e-7, 1e30, 0.0];
        assert_eq!(floats(join(&v).split_whitespace()).unwrap(), v);
        assert!(floats("1 nan".split_whitespace()).is_none());
        assert!(floats("1 x".split_whitespace()).is_none());
    }

    #[test]
    fn bots_without_a_profile_get_a_name_key() {
        assert_eq!(profile_key(12, "x"), "12");
        assert_eq!(profile_key(0, "[RC] Bot 7!"), "default-RCBot7");
        assert_eq!(profile_key(0, "../.."), "default");
        assert_eq!(profile_key(0, &"a".repeat(80)).len(), "default-".len() + 32);
    }
}
