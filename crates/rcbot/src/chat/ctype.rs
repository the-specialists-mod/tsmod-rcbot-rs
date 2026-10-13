//! `<ctype.h>` in the C locale, on `char`s as upstream passes them: signed (x86), so bytes from
//! 0x80 up are negative and belong to no class.

pub(crate) fn is_alpha(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

pub(crate) fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

pub(crate) fn is_alnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

pub(crate) fn is_upper(c: u8) -> bool {
    c.is_ascii_uppercase()
}

/// `toupper(c)` on a signed `char`, as an `int`: what `HAL_CompareWords` subtracts.
pub(crate) fn to_upper_signed(c: u8) -> i32 {
    i32::from(c.to_ascii_uppercase() as i8)
}

/// `strlow`.
pub(crate) fn strlow(s: &mut [u8]) {
    s.make_ascii_lowercase();
}

/// The C string in `s`: everything before the first NUL.
pub(crate) fn cstr(s: &[u8]) -> &[u8] {
    s.iter().position(|&c| c == 0).map_or(s, |n| &s[..n])
}

/// `strstr(haystack, needle)`: the first position, an empty needle matching at 0.
pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The lines `fgets(buffer, size, fp)` returns: at most `size - 1` bytes each, a line longer
/// than that coming back in pieces, each piece keeping its `'\n'` if it reached it.
pub(crate) fn fgets_lines(text: &[u8], size: usize) -> impl Iterator<Item = &[u8]> {
    let max = size - 1;
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let nl = rest.iter().position(|&c| c == b'\n').map_or(rest.len(), |n| n + 1);
        let n = nl.min(max);
        let (line, tail) = rest.split_at(n);
        rest = tail;
        Some(line)
    })
}

/// A text file as upstream's `fopen(…, "r")` reads it on Windows, where text mode turns `"\r\n"`
/// into `"\n"`. The Linux build keeps the `'\r'`, which breaks every file saved on Windows
/// (section names don't match, words end in `'\r'`); the port reads them as Windows does.
pub(crate) fn text_mode(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'\r' && text.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_upper() {
        assert_eq!(to_upper_signed(b'a'), 65);
        assert_eq!(to_upper_signed(0xe9), -23);
        assert!(!is_alpha(0xe9) && !is_alnum(0xe9));
    }

    #[test]
    fn fgets_pieces() {
        let lines: Vec<&[u8]> = fgets_lines(b"abcdef\nxy\n\nz", 4).collect();
        assert_eq!(lines, [&b"abc"[..], b"def", b"\n", b"xy\n", b"\n", b"z"]);
        assert_eq!(text_mode(b"a\r\nb\rc\r\n"), b"a\nb\rc\n");
        assert_eq!(find(b"abc", b""), Some(0));
        assert_eq!(find(b"abc", b"bc"), Some(1));
        assert_eq!(cstr(b"ab\0cd"), b"ab");
    }
}
