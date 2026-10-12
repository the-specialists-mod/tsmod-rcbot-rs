//! `std::stoi`, which RCBot's ini parsers use: it skips leading whitespace, reads an optional sign
//! and decimal digits, ignores whatever follows, and throws when there are no digits
//! (`std::invalid_argument`) or the value doesn't fit an `int` (`std::out_of_range`).

/// Why [`stoi`] failed, named after the C++ exceptions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoiError {
    /// `std::invalid_argument`: no digits.
    InvalidArgument,
    /// `std::out_of_range`: the value doesn't fit an `int`.
    OutOfRange,
}

/// `std::stoi(s)` (base 10).
pub fn stoi(s: &str) -> Result<i32, StoiError> {
    // strtol's isspace: space, \t, \n, \v, \f, \r.
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Err(StoiError::InvalidArgument);
    }
    let mut v: i64 = 0;
    for b in rest[..digits].bytes() {
        v = v * 10 + i64::from(b - b'0');
        if v > i64::from(i32::MAX) + 1 {
            return Err(StoiError::OutOfRange);
        }
    }
    let v = if neg { -v } else { v };
    i32::try_from(v).map_err(|_| StoiError::OutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_std_stoi() {
        assert_eq!(stoi("42"), Ok(42));
        assert_eq!(stoi("  -7"), Ok(-7));
        assert_eq!(stoi("+3x"), Ok(3));
        assert_eq!(stoi("12 34"), Ok(12));
        assert_eq!(stoi("2147483647"), Ok(i32::MAX));
        assert_eq!(stoi("-2147483648"), Ok(i32::MIN));
        assert_eq!(stoi("2147483648"), Err(StoiError::OutOfRange));
        assert_eq!(stoi("99999999999999999999"), Err(StoiError::OutOfRange));
        assert_eq!(stoi(""), Err(StoiError::InvalidArgument));
        assert_eq!(stoi("x1"), Err(StoiError::InvalidArgument));
        assert_eq!(stoi("-"), Err(StoiError::InvalidArgument));
    }
}
