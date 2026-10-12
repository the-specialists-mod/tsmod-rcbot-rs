//! `CWaypointVisibilityTable` (`waypoint.{h,cpp}`): one bit per ordered pair of waypoints, "can
//! `to` be seen from `from`", and its `.rcv` file.
//!
//! The table is `MAX_WAYPOINTS` × `MAX_WAYPOINTS` bits, bit `from * 1024 + to`. Upstream saves
//! only the first `ceil(n² / 8)` bytes of it, which is about the first `n² / 1024` rows, not an
//! n×n table: on a map with 154 waypoints only rows 0-23 survive a save and load, and every other
//! row reads as "nothing visible". The file format is kept as upstream writes it;
//! [`VisibilityTable::rows_in_file`] tells the loader which rows must be worked out again.

use super::flags as fl;
use super::waypoints::Waypoint;
use super::{Glass, MAX_WAYPOINTS, Monsters, World, rcz};
use crate::Vector;

/// `g_iMaxVisibilityByte`.
pub const MAX_VISIBILITY_BYTES: usize = MAX_WAYPOINTS * MAX_WAYPOINTS / 8;

/// The bit table.
#[derive(Clone)]
pub struct VisibilityTable {
    bits: Box<[u8]>,
}

impl Default for VisibilityTable {
    fn default() -> Self {
        VisibilityTable::new()
    }
}

/// Why a `.rcv` file wasn't used (`ReadFromFile` returning false).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibilityError {
    /// An RCZ file that doesn't decompress to the expected size.
    BadCompressed,
    /// An old uncompressed file of the wrong size.
    WrongSize,
}

/// `Ceiling((float)(n * n) / 8)`: the bytes upstream reads and writes.
pub fn file_bytes(num_waypoints: usize) -> usize {
    (num_waypoints * num_waypoints).div_ceil(8)
}

impl VisibilityTable {
    /// All clear.
    pub fn new() -> VisibilityTable {
        VisibilityTable { bits: vec![0; MAX_VISIBILITY_BYTES].into_boxed_slice() }
    }

    fn position(from: usize, to: usize) -> Option<(usize, u8)> {
        let pos = from.checked_mul(MAX_WAYPOINTS)?.checked_add(to)?;
        (pos / 8 < MAX_VISIBILITY_BYTES).then_some((pos / 8, 1 << (pos % 8)))
    }

    /// `GetVisibilityFromTo`.
    pub fn get(&self, from: usize, to: usize) -> bool {
        Self::position(from, to).is_some_and(|(byte, bit)| self.bits[byte] & bit != 0)
    }

    /// `SetVisibilityFromTo`.
    pub fn set(&mut self, from: usize, to: usize, visible: bool) {
        if let Some((byte, bit)) = Self::position(from, to) {
            if visible {
                self.bits[byte] |= bit;
            } else {
                self.bits[byte] &= !bit;
            }
        }
    }

    /// `ClearVisibilityTable`.
    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    /// The raw table, `MAX_VISIBILITY_BYTES` long.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    /// `WorkOutVisibilityTable`: traces from each waypoint to 16 units above every other one
    /// (deleted waypoints stay invisible; each waypoint sees itself).
    pub fn work_out(&mut self, waypoints: &[Waypoint], world: &dyn World) {
        self.clear();
        self.work_out_rows(waypoints, 0, world);
    }

    /// [`VisibilityTable::work_out`] for the rows `first_row..` only, e.g. those a loaded file
    /// didn't hold ([`VisibilityTable::rows_in_file`]). A port addition.
    pub fn work_out_rows(&mut self, waypoints: &[Waypoint], first_row: usize, world: &dyn World) {
        for (i, w1) in waypoints.iter().enumerate().skip(first_row) {
            if w1.flags & fl::DELETED != 0 {
                continue;
            }
            for (j, w2) in waypoints.iter().enumerate() {
                if i == j {
                    self.set(i, j, true);
                    continue;
                }
                if w2.flags & fl::DELETED != 0 {
                    continue;
                }
                let tr = world.trace_line(w1.origin, w2.origin + Vector::new(0.0, 0.0, 16.0), Monsters::Ignore, Glass::Ignore, None);
                self.set(i, j, tr.fraction >= 1.0);
            }
        }
    }

    /// How many leading rows of an `num_waypoints` table a `.rcv` file holds in full (see the
    /// module docs). A port addition.
    pub fn rows_in_file(num_waypoints: usize) -> usize {
        let bits = file_bytes(num_waypoints).min(MAX_VISIBILITY_BYTES) * 8;
        // Row r is whole when its columns 0..n are: r * 1024 + n <= bits.
        if num_waypoints == 0 || bits < num_waypoints {
            return 0;
        }
        ((bits - num_waypoints) / MAX_WAYPOINTS + 1).min(num_waypoints)
    }

    /// `SaveToFile`: the `.rcv` bytes (RCZ), `None` when there are no waypoints.
    pub fn to_rcv(&self, num_waypoints: usize) -> Option<Vec<u8>> {
        rcz::compress(&self.bits[..file_bytes(num_waypoints).min(MAX_VISIBILITY_BYTES)])
    }

    /// `ReadFromFile`: clears the table, then reads an RCZ file, or an old raw file of exactly
    /// `ceil(n² / 8)` bytes.
    pub fn read_rcv(&mut self, bytes: &[u8], num_waypoints: usize) -> Result<(), VisibilityError> {
        self.clear();
        let size = file_bytes(num_waypoints).min(MAX_VISIBILITY_BYTES);
        if rcz::is_compressed(bytes) {
            let data = rcz::decompress(bytes, size).ok_or(VisibilityError::BadCompressed)?;
            self.bits[..size].copy_from_slice(&data);
            return Ok(());
        }
        if bytes.len() != size {
            return Err(VisibilityError::WrongSize);
        }
        self.bits[..size].copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::EmptyWorld;

    #[test]
    fn bits() {
        let mut t = VisibilityTable::new();
        t.set(3, 1000, true);
        assert!(t.get(3, 1000) && !t.get(1000, 3));
        t.set(3, 1000, false);
        assert!(!t.get(3, 1000));
        t.set(5000, 0, true); // outside: ignored
        assert!(!t.get(5000, 0));
    }

    #[test]
    fn file_holds_only_leading_rows() {
        let n = 154;
        let wpts: Vec<Waypoint> = (0..n).map(|i| Waypoint { flags: 0, origin: Vector::new(i as f32, 0.0, 0.0) }).collect();
        let mut t = VisibilityTable::new();
        t.work_out(&wpts, &EmptyWorld);
        assert!(t.get(100, 3));
        let rcv = t.to_rcv(n).unwrap();
        let mut back = VisibilityTable::new();
        back.read_rcv(&rcv, n).unwrap();
        assert_eq!(VisibilityTable::rows_in_file(n), 24);
        assert_eq!(VisibilityTable::rows_in_file(1024), 1024);
        assert_eq!(VisibilityTable::rows_in_file(1), 1);
        assert!(back.get(23, 153));
        assert!(!back.get(24, 0)); // upstream's lost rows
        back.work_out_rows(&wpts, VisibilityTable::rows_in_file(n), &EmptyWorld);
        assert_eq!(back.as_bytes(), t.as_bytes());
        // Raw (pre-RCZ) files of the right size load too.
        let raw = &t.as_bytes()[..file_bytes(n)];
        back.read_rcv(raw, n).unwrap();
        assert_eq!(back.read_rcv(&raw[1..], n), Err(VisibilityError::WrongSize));
        assert_eq!(back.read_rcv(&rcv, n + 1), Err(VisibilityError::BadCompressed));
    }
}
