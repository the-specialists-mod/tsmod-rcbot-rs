//! The headers in RCBot's learned-data files: `CGenericHeader` (`gannconst.h`), which precedes
//! every learning object, and `CLearnedHeader` (`bot.h`, `bot.cpp`), which starts the `.rld`
//! files.

use super::io::Reader;

/// `LEARNTYPE_*` (`gannconst.h`): the `t-` number in a [`GenericHeader`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum LearnType {
    /// `LEARNTYPE_PERCEPTRON`.
    Perceptron = 1,
    /// `LEARNTYPE_POPULATION`.
    Population = 2,
    /// `LEARNTYPE_GAVALUES`.
    GaValues = 3,
    /// `LEARNTYPE_NN`.
    Nn = 4,
    /// `LEARNTYPE_NN_LAYER`.
    NnLayer = 5,
    /// `LEARNTYPE_BITS`.
    Bits = 6,
    /// `LEARNTYPE_GA` (declared, never written).
    Ga = 7,
}

/// `sizeof(CGenericHeader)`.
pub const GENERIC_HEADER_SIZE: usize = 32;

/// `CGenericHeader`: `char[32]` holding `"t-<type>-s-<size>"`. Two headers are equal when the
/// strings are (`strcmp`), so the bytes after the terminator don't matter on reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenericHeader {
    text: [u8; GENERIC_HEADER_SIZE],
}

impl GenericHeader {
    /// `CGenericHeader(iType, iSize)`. Upstream leaves the bytes after the terminator
    /// uninitialised; they are zero here (design §4).
    pub fn new(kind: LearnType, size: i32) -> GenericHeader {
        let s = format!("t-{}-s-{}", kind as i32, size);
        let mut text = [0u8; GENERIC_HEADER_SIZE];
        // snprintf(…, 32, …) keeps at most 31 characters.
        let n = s.len().min(GENERIC_HEADER_SIZE - 1);
        text[..n].copy_from_slice(&s.as_bytes()[..n]);
        GenericHeader { text }
    }

    /// The string up to its terminator.
    pub fn as_str(&self) -> &str {
        let end = self.text.iter().position(|&b| b == 0).unwrap_or(GENERIC_HEADER_SIZE);
        std::str::from_utf8(&self.text[..end]).unwrap_or("")
    }

    /// `CGenericHeader::write`.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.text);
    }

    /// `CGenericHeader::read`: reads 32 bytes (or what is left) and compares the string with
    /// `expected`. A mismatch still consumes the header.
    pub fn read(r: &mut Reader, expected: &GenericHeader) -> bool {
        GenericHeader::read_any(r).as_str() == expected.as_str()
    }

    /// Reads a header whatever it says (`CGenericHeader read;` starts as an empty string).
    pub fn read_any(r: &mut Reader) -> GenericHeader {
        let mut text = [0u8; GENERIC_HEADER_SIZE];
        r.read_into(&mut text);
        GenericHeader { text }
    }

    /// The `(type, size)` the string names, if it has upstream's shape.
    pub fn parse(&self) -> Option<(i32, i32)> {
        let rest = self.as_str().strip_prefix("t-")?;
        let (t, s) = rest.split_once("-s-")?;
        Some((t.parse().ok()?, s.parse().ok()?))
    }
}

/// `BOT_VER` (`bot_const.h`) of the plain release build, which `.rld` headers must carry. The
/// metamod build writes `"1.51b14_mm"` and debug builds `_debug`; their files don't load here, as
/// they don't load in each other upstream.
pub const BOT_VER: &str = "1.51b14";
/// `LEARNED_FILE_VER` (`bot.h`).
pub const LEARNED_FILE_VER: i32 = 1;
/// `sizeof(CLearnedHeader)`.
pub const LEARNED_HEADER_SIZE: usize = 40;

/// `CLearnedHeader`: `char szBotVersion[32]; int iProfileId; int iDataVersion;`. Bot files use
/// the profile id, team files the team number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LearnedHeader {
    bytes: [u8; LEARNED_HEADER_SIZE],
}

impl LearnedHeader {
    /// `CLearnedHeader(iId)`: `strncpy(szBotVersion, BOT_VER, 31)` (zero-padded), the id and
    /// [`LEARNED_FILE_VER`].
    pub fn new(id: i32) -> LearnedHeader {
        let mut bytes = [0u8; LEARNED_HEADER_SIZE];
        let v = BOT_VER.as_bytes();
        let n = v.len().min(31);
        bytes[..n].copy_from_slice(&v[..n]);
        bytes[32..36].copy_from_slice(&id.to_le_bytes());
        bytes[36..40].copy_from_slice(&LEARNED_FILE_VER.to_le_bytes());
        LearnedHeader { bytes }
    }

    /// The header's bytes as `fwrite` writes them.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.bytes);
    }

    /// Upstream's check: `fread` into a header constructed for `id` (so a short read keeps the
    /// expected tail) and `operator ==` against a fresh one.
    pub fn read_matches(r: &mut Reader, id: i32) -> bool {
        let expected = LearnedHeader::new(id);
        let mut read = expected;
        r.read_into(&mut read.bytes);
        read.equal_fields(&expected)
    }

    /// `operator ==`: `strcmp` on the version (bytes after its terminator are ignored) and the
    /// two ints.
    fn equal_fields(&self, other: &LearnedHeader) -> bool {
        fn cstr(b: &[u8]) -> &[u8] {
            &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
        }
        cstr(&self.bytes[..32]) == cstr(&other.bytes[..32]) && self.bytes[32..] == other.bytes[32..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_header_text_and_compare() {
        let h = GenericHeader::new(LearnType::GaValues, 16);
        assert_eq!(h.as_str(), "t-3-s-16");
        assert_eq!(h.parse(), Some((3, 16)));
        let mut bytes = Vec::new();
        h.write(&mut bytes);
        assert_eq!(bytes.len(), 32);
        // Garbage after the terminator (as upstream's files have) still matches.
        bytes[20] = 0xcc;
        assert!(GenericHeader::read(&mut Reader::new(&bytes), &h));
        let mut r = Reader::new(&bytes);
        assert!(!GenericHeader::read(&mut r, &GenericHeader::new(LearnType::GaValues, 8)));
        assert_eq!(r.remaining(), 0, "a mismatch still consumes the header");
        assert!(!GenericHeader::read(&mut Reader::new(&[]), &h));
    }

    #[test]
    fn learned_header_layout() {
        let mut out = Vec::new();
        LearnedHeader::new(7).write(&mut out);
        assert_eq!(&out[..8], b"1.51b14\0");
        assert!(out[8..32].iter().all(|&b| b == 0));
        assert_eq!(&out[32..], &[7, 0, 0, 0, 1, 0, 0, 0]);
        assert!(LearnedHeader::read_matches(&mut Reader::new(&out), 7));
        assert!(!LearnedHeader::read_matches(&mut Reader::new(&out), 8));
        // A short read keeps the expected bytes, so an empty file "matches" as upstream's does.
        assert!(LearnedHeader::read_matches(&mut Reader::new(&[]), 3));
        let mut mm = out.clone();
        mm[7..10].copy_from_slice(b"_mm");
        assert!(!LearnedHeader::read_matches(&mut Reader::new(&mm), 7));
    }
}
