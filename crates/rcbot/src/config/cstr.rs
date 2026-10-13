//! The C library behaviour RCBot's config code leans on: `fgets` line chunks, `strlen`, `atoi`,
//! `atof`, `strlow` and `printf`'s `%0.2f`.
//!
//! Files are read as bytes and turned into `String`s lossily where upstream would have handed a
//! `char *` on. Upstream reads them with `fopen(…, "r")`: text mode turns `"\r\n"` into `"\n"`
//! on Windows and keeps the `'\r'` on Linux; the line-based readers here read as Windows does on
//! every platform ([`text_mode`], design §4), like the chat files.

/// `fgets(buffer, n, fp)` called until it returns `NULL`: chunks of at most `n - 1` bytes, each
/// ending after a `'\n'` if one came first. A line longer than `n - 2` characters becomes several
/// chunks, which upstream's loops treat as separate lines.
pub fn fgets_chunks(data: &[u8], n: usize) -> impl Iterator<Item = &[u8]> {
    let max = n.saturating_sub(1).max(1);
    let mut rest = data;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let limit = rest.len().min(max);
        let end = rest[..limit].iter().position(|&b| b == b'\n').map_or(limit, |p| p + 1);
        let (chunk, tail) = rest.split_at(end);
        rest = tail;
        Some(chunk)
    })
}

/// A text file as `fopen(…, "r")` reads it on Windows: `"\r\n"` becomes `"\n"`.
pub fn text_mode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for (i, &b) in data.iter().enumerate() {
        if !(b == b'\r' && data.get(i + 1) == Some(&b'\n')) {
            out.push(b);
        }
    }
    out
}

/// `strlen` semantics: the bytes before the first NUL.
pub fn c_str(bytes: &[u8]) -> &[u8] {
    bytes.iter().position(|&b| b == 0).map_or(bytes, |p| &bytes[..p])
}

/// Bytes as a `String`, lossily (upstream passes the bytes on as they are).
pub fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// C's `isspace` in the "C" locale.
pub fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `atoi(s)`: `(int)strtol(s, NULL, 10)`. Leading whitespace, an optional sign, then digits; no
/// digits gives 0. Out-of-range values are undefined in C; this saturates at the `int` range, as
/// 32-bit `long` platforms (Windows, i686 Linux: where HL servers run) do.
pub fn atoi(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = b.iter().take_while(|&&c| is_c_space(c)).count();
    let neg = match b.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    for &c in b[i..].iter().take_while(|c| c.is_ascii_digit()) {
        v = (v * 10 + i64::from(c - b'0')).min(i64::from(i32::MAX) + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `atof(s)`: `strtod(s, NULL)`. Leading whitespace, then the longest prefix that is a decimal
/// float (`[+-]digits[.digits][e[+-]digits]`), a hex float (`0x…[p…]`), `inf`/`infinity` or
/// `nan`, case-insensitively; anything else gives 0.
pub fn atof(s: &str) -> f64 {
    let b = s.as_bytes();
    let start = b.iter().take_while(|&&c| is_c_space(c)).count();
    let b = &b[start..];
    let mut i = 0;
    let neg = match b.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let sign = if neg { -1.0 } else { 1.0 };
    let rest = &b[i..];
    let lower: Vec<u8> = rest.iter().take(8).map(u8::to_ascii_lowercase).collect();
    if lower.starts_with(b"inf") {
        return sign * f64::INFINITY;
    }
    if lower.starts_with(b"nan") {
        return f64::NAN.copysign(sign);
    }
    if lower.starts_with(b"0x") {
        if let Some(v) = hex_float(&rest[2..]) {
            return sign * v;
        }
        // "0x" without hex digits: strtod reads the "0".
        return sign * 0.0;
    }
    let digits = |from: usize| rest[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int_digits = digits(0);
    let mut end = int_digits;
    let mut frac_digits = 0;
    if rest.get(end) == Some(&b'.') {
        frac_digits = digits(end + 1);
        if int_digits + frac_digits > 0 {
            end += 1 + frac_digits;
        }
    }
    if int_digits + frac_digits == 0 {
        return 0.0;
    }
    if matches!(rest.get(end), Some(b'e' | b'E')) {
        let mut e = end + 1;
        if matches!(rest.get(e), Some(b'+' | b'-')) {
            e += 1;
        }
        let exp_digits = digits(e);
        if exp_digits > 0 {
            end = e + exp_digits;
        }
    }
    let text = std::str::from_utf8(&rest[..end]).unwrap_or("0");
    sign * text.parse::<f64>().unwrap_or(0.0)
}

/// The hex digits (and optional binary exponent) after `0x`; `None` without any hex digit.
fn hex_float(b: &[u8]) -> Option<f64> {
    let mut v = 0.0f64;
    let mut i = 0;
    let mut any = false;
    while let Some(d) = b.get(i).and_then(|c| (*c as char).to_digit(16)) {
        v = v * 16.0 + f64::from(d);
        i += 1;
        any = true;
    }
    let mut scale = 0i32;
    if b.get(i) == Some(&b'.') {
        i += 1;
        while let Some(d) = b.get(i).and_then(|c| (*c as char).to_digit(16)) {
            v = v * 16.0 + f64::from(d);
            scale -= 4;
            i += 1;
            any = true;
        }
    }
    if !any {
        return None;
    }
    if matches!(b.get(i), Some(b'p' | b'P')) {
        let mut e = i + 1;
        let neg = match b.get(e) {
            Some(b'-') => {
                e += 1;
                true
            }
            Some(b'+') => {
                e += 1;
                false
            }
            _ => false,
        };
        let n = b[e..].iter().take_while(|c| c.is_ascii_digit()).count();
        if n > 0 {
            let exp: i32 = b[e..e + n].iter().fold(0i32, |a, &c| a.saturating_mul(10).saturating_add(i32::from(c - b'0')));
            scale = scale.saturating_add(if neg { -exp } else { exp });
        }
    }
    Some(v * 2f64.powi(scale))
}

/// `atof` narrowed to `float`, as upstream stores it (`static_cast<float>(std::atof(…))`).
pub fn atof_f32(s: &str) -> f32 {
    atof(s) as f32
}

/// `printf("%0.2f", v)`, including C's spelling of infinities and NaN.
pub fn f2(v: f32) -> String {
    if v.is_nan() {
        if v.is_sign_negative() { "-nan".into() } else { "nan".into() }
    } else if v.is_infinite() {
        if v < 0.0 { "-inf".into() } else { "inf".into() }
    } else {
        format!("{:.2}", f64::from(v))
    }
}

/// `strlow`: ASCII lower case (`tolower` in the "C" locale).
pub fn strlow(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// `strncpy(dst, src, n - 1); dst[n - 1] = 0` on a `char[n]`: at most `n - 1` bytes, cut on a
/// character boundary.
pub fn truncate(s: &str, n: usize) -> String {
    let mut end = s.len().min(n.saturating_sub(1));
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fgets_splits_like_c() {
        let v: Vec<&[u8]> = fgets_chunks(b"ab\ncdefg\n\nh", 4).collect();
        assert_eq!(v, vec![&b"ab\n"[..], b"cde", b"fg\n", b"\n", b"h"]);
        assert_eq!(fgets_chunks(b"", 10).count(), 0);
        assert_eq!(c_str(b"ab\0cd"), b"ab");
    }

    #[test]
    fn atoi_like_c() {
        assert_eq!(atoi("42"), 42);
        assert_eq!(atoi("  -7x"), -7);
        assert_eq!(atoi("+3"), 3);
        assert_eq!(atoi("x1"), 0);
        assert_eq!(atoi(""), 0);
        assert_eq!(atoi("1\r"), 1);
        assert_eq!(atoi("99999999999"), i32::MAX);
        assert_eq!(atoi("-99999999999"), i32::MIN);
        assert_eq!(atoi("\t\n12"), 12);
        assert_eq!(atoi("- 1"), 0);
    }

    #[test]
    fn atof_like_strtod() {
        assert_eq!(atof("1.5"), 1.5);
        assert_eq!(atof("  -2.25abc"), -2.25);
        assert_eq!(atof(".5"), 0.5);
        assert_eq!(atof("5."), 5.0);
        assert_eq!(atof("1e3"), 1000.0);
        assert_eq!(atof("1e"), 1.0);
        assert_eq!(atof("1e+"), 1.0);
        assert_eq!(atof("2E-1x"), 0.2);
        assert_eq!(atof("abc"), 0.0);
        assert_eq!(atof("."), 0.0);
        assert_eq!(atof("-"), 0.0);
        assert_eq!(atof("INF"), f64::INFINITY);
        assert_eq!(atof("-infinity"), f64::NEG_INFINITY);
        assert!(atof("nan").is_nan());
        assert_eq!(atof("0x10"), 16.0);
        assert_eq!(atof("0x1p4"), 16.0);
        assert_eq!(atof("0x.8"), 0.5);
        assert_eq!(atof("0xg"), 0.0);
        assert_eq!(atof_f32("0.1"), 0.1f32);
    }

    #[test]
    fn printf_and_strings() {
        assert_eq!(f2(-1.0), "-1.00");
        assert_eq!(f2(0.125), "0.12");
        assert_eq!(f2(15.0), "15.00");
        assert_eq!(f2(f32::INFINITY), "inf");
        assert_eq!(f2(f32::NAN), "nan");
        assert_eq!(strlow("AbC"), "abc");
        assert_eq!(truncate("abcdef", 4), "abc");
        assert_eq!(truncate("ab", 4), "ab");
        assert_eq!(truncate("aé", 3), "a");
    }
}
