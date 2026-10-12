//! `.rcw` waypoint files (`WaypointLoad`, `WaypointSave`), byte for byte as upstream writes them
//! (little-endian, as the x86 builds):
//!
//! ```text
//! WAYPOINT_HDR (52 bytes)  char filetype[8] ("RCBot\0" + 2 unspecified bytes)
//!                          int  waypoint_file_version (4)
//!                          int  waypoint_file_flags (W_FILE_FL_READ_VISIBILITY)
//!                          int  number_of_waypoints
//!                          char mapname[32] (NUL-padded)
//! WAYPOINT × n (16 bytes)  int flags; float origin[3]
//! per waypoint             short count; short path[count]
//! ```
//!
//! The loader also takes `"HPB_bot"` files (HPB bot `.wpt`, same layout) and GraveBot `.gbw`
//! files (`"[Grave]"`/`"Crabbed"`, 36-byte waypoints whose flags are converted). Choosing which
//! file to open (`.rcw`, then `.wpt`, then `maps/<map>.gbw`) is the host's job.

use super::flags as fl;
use super::waypoints::{Waypoint, Waypoints};
use super::{MAX_WAYPOINTS, WAYPOINT_VERSION};
use crate::Vector;

/// `sizeof(WAYPOINT_HDR)`.
pub const HEADER_SIZE: usize = 52;
/// `sizeof(WAYPOINT)`.
pub const WAYPOINT_SIZE: usize = 16;
/// `sizeof(GRAVEBOT_WAYPOINT)`: `int flags; Vector origin; char item[19]`, padded to 4.
pub const GRAVEBOT_WAYPOINT_SIZE: usize = 36;

/// `WAYPOINT_HDR`, as read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RcwHeader {
    /// `filetype`: "RCBot" (or "HPB_bot", "[Grave]", "Crabbed") and padding.
    pub filetype: [u8; 8],
    /// `waypoint_file_version`.
    pub version: i32,
    /// `waypoint_file_flags`: [`fl::FILE_READ_VISIBILITY`] means the visibility file is stale.
    pub flags: i32,
    /// `number_of_waypoints`.
    pub num_waypoints: i32,
    /// `mapname`.
    pub mapname: [u8; 32],
}

impl RcwHeader {
    /// Parses the first 52 bytes.
    pub fn parse(bytes: &[u8]) -> Result<RcwHeader, LoadError> {
        let b = bytes.get(..HEADER_SIZE).ok_or(LoadError::Truncated)?;
        Ok(RcwHeader {
            filetype: b[0..8].try_into().unwrap(),
            version: i32_at(b, 8),
            flags: i32_at(b, 12),
            num_waypoints: i32_at(b, 16),
            mapname: b[20..52].try_into().unwrap(),
        })
    }

    fn cstr(b: &[u8]) -> String {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    }

    /// The file type, up to its NUL (upstream sets `filetype[7] = 0` first).
    pub fn file_type(&self) -> String {
        Self::cstr(&self.filetype[..7])
    }

    /// The map name, up to its NUL (upstream sets `mapname[31] = 0` first).
    pub fn map_name(&self) -> String {
        Self::cstr(&self.mapname[..31])
    }

    /// The header says the visibility table must be worked out again.
    pub fn needs_visibility(&self) -> bool {
        self.flags & fl::FILE_READ_VISIBILITY == fl::FILE_READ_VISIBILITY
    }

    fn is_gravebot(&self) -> bool {
        matches!(self.file_type().as_str(), "[Grave]" | "Crabbed")
    }
}

/// Why waypoints didn't load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The file ends early. (Upstream `fread`s past the end and carries on with what it has.)
    Truncated,
    /// "is not a RCBot compatible waypoint file!"
    NotRcbot(String),
    /// "Incompatible waypoint file version!"
    Version(i32),
    /// "waypoints are not for this map!": the header names another map.
    WrongMap {
        /// The map in the file.
        file: String,
    },
    /// More waypoints than `MAX_WAYPOINTS` (upstream overruns its array).
    TooMany(i32),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Truncated => write!(f, "waypoint file is truncated"),
            LoadError::NotRcbot(t) => write!(f, "not a RCBot compatible waypoint file (type {t:?})"),
            LoadError::Version(v) => write!(f, "incompatible waypoint file version {v}"),
            LoadError::WrongMap { file } => write!(f, "waypoints are for map {file:?}"),
            LoadError::TooMany(n) => write!(f, "{n} waypoints, more than {MAX_WAYPOINTS}"),
        }
    }
}

impl std::error::Error for LoadError {}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// `ConvertGraveBotFlags`.
pub fn convert_gravebot_flags(grave: u32) -> u32 {
    let mut rc = grave & 0x3FF; // bits 0-9 are the same
    for (from, to) in [
        (11, fl::IMPORTANT),
        (12, fl::RESCUE),
        (14, fl::AIMING),
        (17, fl::WEAPON),
        (18, fl::JUMP),
        (20, fl::CROUCHJUMP),
        (21, fl::DEFEND_ZONE),
        (31, fl::DELETED),
    ] {
        if grave & (1 << from) != 0 {
            rc |= to;
        }
    }
    rc
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], LoadError> {
        let s = self.b.get(self.at..self.at + n).ok_or(LoadError::Truncated)?;
        self.at += n;
        Ok(s)
    }

    fn i16(&mut self) -> Result<i16, LoadError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
}

impl Waypoints {
    /// `WaypointLoad` from a file's bytes, for the map `map_name` (compared case-sensitively with
    /// the header's, as upstream). The visibility table is left clear: the caller reads the
    /// `.rcv` (unless [`RcwHeader::needs_visibility`]) or works it out.
    ///
    /// Deviation: upstream skips the count for a waypoint outside ±4096 (it marks it deleted at the
    /// origin), which misaligns every later index and the path section; here it is marked the same
    /// way but keeps its slot.
    pub fn from_rcw(bytes: &[u8], map_name: &str) -> Result<(Waypoints, RcwHeader), LoadError> {
        let header = RcwHeader::parse(bytes)?;
        let file_type = header.file_type();
        let gravebot = header.is_gravebot();
        if !(file_type == "RCBot" || file_type == "HPB_bot" || gravebot) {
            return Err(LoadError::NotRcbot(file_type));
        }
        if header.version != WAYPOINT_VERSION {
            return Err(LoadError::Version(header.version));
        }
        if header.map_name() != map_name {
            return Err(LoadError::WrongMap { file: header.map_name() });
        }
        let n = header.num_waypoints.max(0) as usize;
        if n > MAX_WAYPOINTS {
            return Err(LoadError::TooMany(header.num_waypoints));
        }
        let mut w = Waypoints::new();
        w.filetype_tail = [header.filetype[6], header.filetype[7]];
        let mut r = Reader { b: bytes, at: HEADER_SIZE };
        let record = if gravebot { GRAVEBOT_WAYPOINT_SIZE } else { WAYPOINT_SIZE };
        for i in 0..n {
            let b = r.take(record)?;
            let flags = i32_at(b, 0) as u32;
            let origin = Vector::new(f32_at(b, 4), f32_at(b, 8), f32_at(b, 12));
            let flags = if gravebot { convert_gravebot_flags(flags) } else { flags };
            w.num += 1;
            if origin.x > 4096.0 || origin.y > 4096.0 || origin.z > 4096.0 {
                // "ERROR!!! Invalid waypoint (id: %d) origin outside map !!!"
                w.wpts[i] = Waypoint { flags: flags | fl::DELETED, origin: Vector::ZERO };
                continue;
            }
            w.wpts[i] = Waypoint { flags, origin };
            w.locations.add(i, origin);
        }
        for index in 0..w.num {
            let count = r.i16()?;
            for _ in 0..count.max(0) {
                let p = r.i16()?;
                w.add_path(index, p);
            }
        }
        Ok((w, header))
    }

    /// `WaypointSave` without a converter: the `.rcw` bytes for `map_name`. `visibility_made` says
    /// the `.rcv` was written (otherwise the header asks the next load to rebuild it). The
    /// visibility table itself is [`super::VisibilityTable::to_rcv`].
    pub fn to_rcw(&self, map_name: &str, visibility_made: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_SIZE + self.num * WAYPOINT_SIZE + self.num * 8);
        let mut filetype = *b"RCBot\0\0\0";
        filetype[6..].copy_from_slice(&self.filetype_tail);
        out.extend_from_slice(&filetype);
        out.extend_from_slice(&WAYPOINT_VERSION.to_le_bytes());
        let flags = if visibility_made { 0 } else { fl::FILE_READ_VISIBILITY };
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(self.num as i32).to_le_bytes());
        // memset(0) + strncpy(…, 31).
        let mut mapname = [0u8; 32];
        let name = map_name.as_bytes();
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len()).min(31);
        mapname[..len].copy_from_slice(&name[..len]);
        out.extend_from_slice(&mapname);
        for w in self.as_slice() {
            out.extend_from_slice(&w.flags.to_le_bytes());
            for c in w.origin.to_array() {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        for index in 0..self.num {
            // Upstream counts in a short; a waypoint can't hold 32768 paths to 1024 waypoints
            // unless they repeat, and then it wraps as upstream's does.
            let paths: Vec<i16> = self.paths_from(index).collect();
            out.extend_from_slice(&(paths.len() as i16).to_le_bytes());
            for p in paths {
                out.extend_from_slice(&p.to_le_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Waypoints {
        let mut w = Waypoints::new();
        w.add_origin(Vector::new(1.0, 2.0, 3.0), fl::CROUCH, None);
        w.add_origin(Vector::new(-100.5, 50.25, -36.0), fl::STUNT | fl::JUMP, None);
        w.add_path(0, 1);
        w.add_path(1, 0);
        w.add_path(1, 0);
        w
    }

    #[test]
    fn save_load_save() {
        let w = sample();
        let bytes = w.to_rcw("ts_test", false);
        assert_eq!(bytes.len(), 52 + 2 * 16 + 2 + 2 + 2 + 4);
        assert_eq!(&bytes[..8], b"RCBot\0\0\0");
        let (back, h) = Waypoints::from_rcw(&bytes, "ts_test").unwrap();
        assert!(h.needs_visibility());
        assert_eq!(back.as_slice(), w.as_slice());
        assert_eq!(back.successors(1, -1), vec![0, 0]);
        assert_eq!(back.to_rcw("ts_test", false), bytes);
    }

    #[test]
    fn rejects() {
        let bytes = sample().to_rcw("ts_test", true);
        assert_eq!(Waypoints::from_rcw(&bytes, "ts_other").unwrap_err(), LoadError::WrongMap { file: "ts_test".into() });
        assert_eq!(Waypoints::from_rcw(&bytes[..60], "ts_test").unwrap_err(), LoadError::Truncated);
        let mut v5 = bytes.clone();
        v5[8] = 5;
        assert_eq!(Waypoints::from_rcw(&v5, "ts_test").unwrap_err(), LoadError::Version(5));
        let mut other = bytes.clone();
        other[..8].copy_from_slice(b"Whatev\0\0");
        assert!(matches!(Waypoints::from_rcw(&other, "ts_test"), Err(LoadError::NotRcbot(_))));
        let mut hpb = bytes;
        hpb[..8].copy_from_slice(b"HPB_bot\0");
        assert_eq!(Waypoints::from_rcw(&hpb, "ts_test").unwrap().0.len(), 2);
    }

    #[test]
    fn long_map_names_are_cut_at_31() {
        let name = "a".repeat(40);
        let bytes = sample().to_rcw(&name, true);
        assert_eq!(Waypoints::from_rcw(&bytes, &name[..31]).unwrap().1.map_name(), name[..31]);
    }

    #[test]
    fn outside_waypoints_keep_their_slot() {
        let mut w = sample();
        w.add_origin(Vector::new(5000.0, 0.0, 0.0), 0, None);
        w.add_path(2, 0);
        let (back, _) = Waypoints::from_rcw(&w.to_rcw("m", true), "m").unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back.flags(2), fl::DELETED);
        assert_eq!(back.successors(2, -1), vec![0]);
        assert_eq!(back.successors(1, -1), vec![0, 0]);
    }

    #[test]
    fn gravebot_import() {
        let mut b = Vec::new();
        b.extend_from_slice(b"[Grave]\0");
        b.extend_from_slice(&4i32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        let mut name = [0u8; 32];
        name[..2].copy_from_slice(b"gb");
        b.extend_from_slice(&name);
        let grave_flags: u32 = (1 << 3) | (1 << 18) | (1 << 20) | (1 << 21) | (1 << 10);
        b.extend_from_slice(&grave_flags.to_le_bytes());
        for c in [10.0f32, 20.0, 30.0] {
            b.extend_from_slice(&c.to_le_bytes());
        }
        b.extend_from_slice(&[0u8; 20]); // item[19] + padding
        b.extend_from_slice(&0i16.to_le_bytes());
        let (w, _) = Waypoints::from_rcw(&b, "gb").unwrap();
        assert_eq!(w.flags(0), fl::CROUCH | fl::JUMP | fl::CROUCHJUMP | fl::DEFEND_ZONE);
        assert_eq!(w.origin(0), Vector::new(10.0, 20.0, 30.0));
    }
}
