//! Bot reputation (`bot_rep.cpp`; `CBotReputation(s)` in `bot.h`; `BotFunc_IncreaseRep`,
//! `BotFunc_DecreaseRep`, `CBotReputation::printRep` in `bot.cpp`). Mod-independent upstream, so
//! TS uses it: each bot profile likes or dislikes each player on a 0-10 scale, which picks the
//! tone of its chat (`BotFunc_GetRepArrayNum`) and changes with kills and team kills.
//!
//! Two files, both in the bot folder:
//!
//! * **`bot_playerids.ini`** ([`player_rep_id`]): a player's reputation id is the number of the
//!   line holding their name in quotes, counting every line except those starting with `#` (so
//!   the generated header doesn't count, but an empty line does). Unknown names are appended.
//! * **`botprofiles/<profile>.rcr`** ([`write_rep`], [`Reputations::add_load_rep`]): the bot's
//!   reputations as raw `CBotReputation` records, 8 bytes each: `int m_iPlayerRepId; int m_iRep`,
//!   little-endian (x86). A file whose size isn't a multiple of 8 is left alone and not read.

use super::cstr::{c_str, fgets_chunks};
use crate::Random;

/// `BOT_MAX_REP`.
pub const BOT_MAX_REP: i32 = 10;
/// `BOT_MIN_REP`.
pub const BOT_MIN_REP: i32 = 0;
/// `BOT_MID_REP`: a stranger's reputation.
pub const BOT_MID_REP: i32 = 5;
/// `BOT_LOW_REP`.
pub const BOT_LOW_REP: i32 = 3;
/// `BOT_HIGH_REP`.
pub const BOT_HIGH_REP: i32 = 6;
/// `sizeof(CBotReputation)`.
pub const REP_RECORD_SIZE: usize = 8;
/// `BOT_PLAYER_ID_FILE_HEADER`.
pub const BOT_PLAYER_ID_FILE_HEADER: &str =
    "# Automatically generated, do not edit.\n# however this file is safe to delete\n# but will be re-generated.\n";
/// The `BugMessage` of `IncreaseRep`/`DecreaseRep` for a player without a reputation.
pub const BAD_REP_DATA: &str = "bad rep data... (try deleting all botprofiles/*.rcr files :-( )";

/// `CBotReputation`: how much the bot likes one player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reputation {
    /// `m_iPlayerRepId`.
    pub player_rep_id: i32,
    /// `m_iRep`, 0-10.
    pub rep: i32,
}

impl Default for Reputation {
    /// `CBotReputation()`.
    fn default() -> Reputation {
        Reputation { player_rep_id: -1, rep: 5 }
    }
}

impl Reputation {
    /// The record as stored in a `.rcr` file.
    pub fn to_bytes(self) -> [u8; REP_RECORD_SIZE] {
        let mut b = [0; REP_RECORD_SIZE];
        b[..4].copy_from_slice(&self.player_rep_id.to_le_bytes());
        b[4..].copy_from_slice(&self.rep.to_le_bytes());
        b
    }

    /// A record from a `.rcr` file.
    pub fn from_bytes(b: &[u8; REP_RECORD_SIZE]) -> Reputation {
        Reputation { player_rep_id: i32::from_le_bytes([b[0], b[1], b[2], b[3]]), rep: i32::from_le_bytes([b[4], b[5], b[6], b[7]]) }
    }

    /// `printRep`: the line `showreps` prints, `None` for reputations outside 0-10.
    pub fn describe(&self, bot_name: &str, player_name: &str) -> Option<String> {
        let what = match self.rep {
            0 => "really hates",
            1 => "hates",
            2 => "kind of hates",
            3 => "doesn't like",
            4..=6 => "likes",
            7 => "really likes",
            8 => "loves",
            9 => "really loves",
            10 => "LURRVESS",
            _ => return None,
        };
        let tail = if self.rep == 10 { " !!!" } else { "" };
        Some(format!("{bot_name} {what} {player_name}{tail} ({})", self.rep))
    }
}

/// The records of a `.rcr` file, `None` if its size isn't a multiple of 8.
pub fn parse_rep_file(data: &[u8]) -> Option<Vec<Reputation>> {
    if !data.len().is_multiple_of(REP_RECORD_SIZE) {
        return None;
    }
    Some(data.as_chunks::<REP_RECORD_SIZE>().0.iter().map(Reputation::from_bytes).collect())
}

/// `CBotReputations::WriteToFile` on the file's current contents (`None`: there is no file):
/// the first record for the same player is overwritten in place, otherwise the record is
/// appended (or the file created with it). Returns the new contents, or `None` when upstream
/// writes nothing (the file's size isn't a multiple of 8).
pub fn write_rep(file: Option<&[u8]>, rep: Reputation) -> Option<Vec<u8>> {
    let Some(data) = file else {
        return Some(rep.to_bytes().to_vec());
    };
    if !data.len().is_multiple_of(REP_RECORD_SIZE) {
        return None;
    }
    let mut out = data.to_vec();
    match out.as_chunks::<REP_RECORD_SIZE>().0.iter().position(|c| Reputation::from_bytes(c).player_rep_id == rep.player_rep_id) {
        Some(i) => out[i * REP_RECORD_SIZE..(i + 1) * REP_RECORD_SIZE].copy_from_slice(&rep.to_bytes()),
        None => out.extend_from_slice(&rep.to_bytes()),
    }
    Some(out)
}

/// The `.rcr` file name of a profile: `"%d.rcr"`, in [`super::BOT_PROFILES_FOLDER`].
pub fn rep_file_name(profile_id: i32) -> String {
    format!("{profile_id}.rcr")
}

/// `CBotReputations`: a stack, newest on top (the end of the `Vec`); searches go from the top.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reputations {
    list: Vec<Reputation>,
}

impl Reputations {
    /// Every reputation, oldest first.
    pub fn all(&self) -> &[Reputation] {
        &self.list
    }

    /// `Destroy` / `Init`.
    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// `GetRep`.
    pub fn get(&self, player_rep_id: i32) -> Option<&Reputation> {
        self.list.iter().rev().find(|r| r.player_rep_id == player_rep_id)
    }

    fn get_mut(&mut self, player_rep_id: i32) -> Option<&mut Reputation> {
        self.list.iter_mut().rev().find(|r| r.player_rep_id == player_rep_id)
    }

    /// `AddRep`: pushes a reputation unless the player has one.
    pub fn add_rep(&mut self, player_rep_id: i32, rep: i32) {
        if self.get(player_rep_id).is_none() {
            self.list.push(Reputation { player_rep_id, rep });
        }
    }

    /// `NewRep`: [`Self::add_rep`] with [`BOT_MID_REP`].
    pub fn new_rep(&mut self, player_rep_id: i32) {
        self.add_rep(player_rep_id, BOT_MID_REP);
    }

    /// `GetCreateRep`.
    pub fn get_create_rep(&mut self, player_rep_id: i32) -> &mut Reputation {
        if let Some(i) = self.list.iter().rposition(|r| r.player_rep_id == player_rep_id) {
            return &mut self.list[i];
        }
        self.list.push(Reputation { player_rep_id, rep: BOT_MID_REP });
        let last = self.list.len() - 1;
        &mut self.list[last]
    }

    /// `AverageRepOnServer`: the integer mean, 0 with none.
    pub fn average_rep_on_server(&self) -> i32 {
        if self.list.is_empty() {
            return 0;
        }
        self.list.iter().map(|r| r.rep).sum::<i32>() / self.list.len() as i32
    }

    /// `GetClientRep`: `None` is a client upstream didn't find (`NULL`), `Some(-1)` a client
    /// without a reputation id. Returns the reputation ([`BOT_MID_REP`] when unknown) and
    /// upstream's warning, if any.
    pub fn client_rep(&self, client_rep_id: Option<i32>) -> (i32, Option<String>) {
        let Some(id) = client_rep_id else {
            return (BOT_MID_REP, Some("warning: GetClientRep(): Bad pClient received (NULL) (returning default)".into()));
        };
        if id == -1 {
            return (BOT_MID_REP, None);
        }
        match self.get(id) {
            Some(r) => (r.rep, None),
            None => (BOT_MID_REP, Some(format!("warning: GetClientRep(): No Rep for pClient (id: {id}) (returning default)"))),
        }
    }

    /// `IncreaseRep`: one up, not past [`BOT_MAX_REP`]; -1 is ignored. `Err` carries upstream's
    /// `BugMessage` when the player has no reputation.
    pub fn increase_rep(&mut self, player_rep_id: i32) -> Result<(), &'static str> {
        self.step(player_rep_id, 1)
    }

    /// `DecreaseRep`: one down, not past [`BOT_MIN_REP`].
    pub fn decrease_rep(&mut self, player_rep_id: i32) -> Result<(), &'static str> {
        self.step(player_rep_id, -1)
    }

    fn step(&mut self, player_rep_id: i32, by: i32) -> Result<(), &'static str> {
        if player_rep_id == -1 {
            return Ok(());
        }
        let r = self.get_mut(player_rep_id).ok_or(BAD_REP_DATA)?;
        let new = r.rep + by;
        if (BOT_MIN_REP..=BOT_MAX_REP).contains(&new) {
            r.rep = new;
        }
        Ok(())
    }

    /// `GetRandomClient(iRep)`: a random player the bot dislikes (-1: rep ≤ 3), is neutral about
    /// (0: 4-6) or likes (1: ≥ 7), picked with `RANDOM_LONG` among the matches in stack order
    /// (top first), then found by reputation id with `client_by_rep_id`
    /// (`GetClientByRepId`). `None` when nobody matches (no random number is drawn).
    pub fn random_client<C>(&self, rep_class: i32, rng: &mut dyn Random, client_by_rep_id: impl Fn(i32) -> Option<C>) -> Option<C> {
        let ids: Vec<i32> = self
            .list
            .iter()
            .rev()
            .filter(|r| match rep_class {
                -1 => r.rep <= 3,
                0 => r.rep > 3 && r.rep < 7,
                1 => r.rep >= 7,
                _ => false,
            })
            .map(|r| r.player_rep_id)
            .collect();
        if ids.is_empty() {
            return None;
        }
        let pick = ids[rng.random_long(0, ids.len() as i32 - 1).clamp(0, ids.len() as i32 - 1) as usize];
        client_by_rep_id(pick)
    }

    /// `printReps`: [`Reputation::describe`] for each, top first. `name_of` gives the player's
    /// name for a reputation id.
    ///
    /// Deviation: upstream dereferences a `NULL` client when no player has the id; the port
    /// skips that reputation.
    pub fn print_reps(&self, bot_name: &str, name_of: impl Fn(i32) -> Option<String>) -> Vec<String> {
        self.list.iter().rev().filter_map(|r| r.describe(bot_name, &name_of(r.player_rep_id)?)).collect()
    }

    /// `AddLoadRep`: loads the player's reputation from the profile's `.rcr` file contents
    /// (`None`: no file). If the file has none, the player starts at [`BOT_MID_REP`] and that is
    /// saved: the returned bytes are the file's new contents. A file of the wrong size is
    /// ignored and the player gets no reputation at all.
    pub fn add_load_rep(&mut self, player_rep_id: i32, file: Option<&[u8]>) -> Option<Vec<u8>> {
        let records = match file {
            None => Vec::new(),
            Some(data) => parse_rep_file(data)?,
        };
        if let Some(r) = records.iter().find(|r| r.player_rep_id == player_rep_id) {
            self.list.push(*r);
            return None;
        }
        let rep = Reputation { player_rep_id, rep: BOT_MID_REP };
        self.list.push(rep);
        write_rep(file, rep)
    }

    /// `RemoveSaveRep`: saves the player's reputation (the new file contents are returned) and
    /// drops it.
    pub fn remove_save_rep(&mut self, player_rep_id: i32, file: Option<&[u8]>) -> Option<Vec<u8>> {
        let i = self.list.iter().rposition(|r| r.player_rep_id == player_rep_id)?;
        let rep = self.list.remove(i);
        write_rep(file, rep)
    }

    /// `SaveAllRep`: [`write_rep`] for each reputation, top first. Returns the file's final
    /// contents (`None`: nothing written).
    pub fn save_all_rep(&self, file: Option<&[u8]>) -> Option<Vec<u8>> {
        let mut current: Option<Vec<u8>> = file.map(<[u8]>::to_vec);
        let mut wrote = false;
        for r in self.list.iter().rev() {
            if let Some(new) = write_rep(current.as_deref(), *r) {
                current = Some(new);
                wrote = true;
            }
        }
        if wrote { current } else { None }
    }
}

/// `BotFunc_GetRepArrayNum`: which chat array a reputation uses: 0 enemy (< 3), 1 neutral (3-6),
/// 2 friendly. -1 is a bug upstream (reported with `BugMessage`, the `String`) and gives 1.
pub fn rep_array_num(rep: i32) -> (usize, Option<String>) {
    if rep == -1 {
        return (1, Some(format!("GetRepArrayNum: Bad rep received ({rep})")));
    }
    let n = if rep < 3 {
        0
    } else if rep <= 6 {
        1
    } else {
        2
    };
    (n, None)
}

/// `BotFunc_DecreaseRep(iRep, fInfo, fSkill)`: whether the bot thinks less of a player. With a
/// distance (`info` ≥ 0), only if `min(info / 800, 1)` is at least `skill / 100`; then (and
/// always without a distance) with chance `RANDOM_LONG(0, 100) >= rep * 10`. Upstream's defaults
/// for the last two arguments are 0, so callers passing only the rep get the skill test
/// `0 >= 0`.
pub fn decrease_rep_roll(rep: i32, info: f32, skill: f32, rng: &mut dyn Random) -> bool {
    if rep == -1 {
        return false;
    }
    if info >= 0.0 {
        let skill = skill / super::MAX_BOT_SKILL as f32;
        let info = (info / 800.0).min(1.0);
        if info >= skill {
            return rng.random_long(0, 100) >= rep * 10;
        }
        false
    } else {
        rng.random_long(0, 100) >= rep * 10
    }
}

/// `BotFunc_IncreaseRep`: the opposite of one [`decrease_rep_roll`] (the same random draw).
pub fn increase_rep_roll(rep: i32, info: f32, skill: f32, rng: &mut dyn Random) -> bool {
    if rep == -1 {
        return false;
    }
    !decrease_rep_roll(rep, info, skill, rng)
}

/// What [`player_rep_id`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerRepId {
    /// The id, -1 for an empty name.
    pub id: i32,
    /// The new contents of `bot_playerids.ini` when upstream writes it (a new file, or the name
    /// appended).
    pub write: Option<Vec<u8>>,
}

/// `GetPlayerRepId(szPlayerName)` over `bot_playerids.ini`'s contents (`None`: no file).
///
/// As upstream reads it: `fgets(buffer, 127)` chunks; chunks starting with `#` don't count; one
/// trailing `'\n'` is removed; the name is what follows the first `"` up to the next `"` (at most
/// 64 bytes; a line without a quote has an empty name), compared case-sensitively. A name
/// containing `"` never matches, so such a player gets a new id on every connect.
///
/// Deviation: upstream leaves a 64-byte name unterminated and compares past it; the port compares
/// the 64 bytes.
pub fn player_rep_id(name: &str, file: Option<&[u8]>) -> PlayerRepId {
    if name.is_empty() {
        return PlayerRepId { id: -1, write: None };
    }
    let entry = format!("\"{name}\"\n");
    let Some(data) = file else {
        let mut out = BOT_PLAYER_ID_FILE_HEADER.as_bytes().to_vec();
        out.extend_from_slice(entry.as_bytes());
        return PlayerRepId { id: 0, write: Some(out) };
    };
    let mut id = 0;
    for chunk in fgets_chunks(data, 127) {
        let mut b = c_str(chunk);
        if b.first() == Some(&b'#') {
            continue;
        }
        if b.is_empty() {
            continue;
        }
        if b.last() == Some(&b'\n') {
            b = &b[..b.len() - 1];
        }
        let start = b.iter().position(|&c| c == b'"').map_or(b.len(), |p| p + 1);
        let rest = &b[start.min(b.len())..];
        let len = rest.iter().take_while(|&&c| c != b'"').count().min(64);
        if &rest[..len] == name.as_bytes() {
            return PlayerRepId { id, write: None };
        }
        id += 1;
    }
    let mut out = data.to_vec();
    out.extend_from_slice(entry.as_bytes());
    PlayerRepId { id, write: Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    struct Fixed(Vec<i32>);
    impl Random for Fixed {
        fn random_long(&mut self, lo: i32, _hi: i32) -> i32 {
            if self.0.is_empty() { lo } else { self.0.remove(0) }
        }
        fn random_float(&mut self, lo: f32, _hi: f32) -> f32 {
            lo
        }
    }

    #[test]
    fn rcr_files_byte_for_byte() {
        let a = Reputation { player_rep_id: 3, rep: 7 };
        assert_eq!(a.to_bytes(), [3, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(Reputation::from_bytes(&a.to_bytes()), a);
        // Create, append, overwrite in place.
        let f = write_rep(None, a).unwrap();
        let f = write_rep(Some(&f), Reputation { player_rep_id: -1, rep: 5 }).unwrap();
        let f = write_rep(Some(&f), Reputation { player_rep_id: 3, rep: 2 }).unwrap();
        assert_eq!(f, [3, 0, 0, 0, 2, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 5, 0, 0, 0]);
        assert_eq!(parse_rep_file(&f).unwrap(), vec![Reputation { player_rep_id: 3, rep: 2 }, Reputation::default()]);
        // A corrupt file is neither read nor written.
        assert_eq!(write_rep(Some(&[1, 2, 3]), a), None);
        assert_eq!(parse_rep_file(&[1, 2, 3]), None);
        // Only the first record for a player is replaced.
        let dup = [a.to_bytes(), a.to_bytes()].concat();
        let f = write_rep(Some(&dup), Reputation { player_rep_id: 3, rep: 9 }).unwrap();
        assert_eq!(parse_rep_file(&f).unwrap()[1].rep, 7);
        assert_eq!(rep_file_name(12), "12.rcr");
    }

    #[test]
    fn load_save_round_trip() {
        let mut r = Reputations::default();
        // No file: the player starts at 5 and the file is created.
        let f = r.add_load_rep(4, None).unwrap();
        assert_eq!(f, Reputation { player_rep_id: 4, rep: 5 }.to_bytes());
        r.increase_rep(4).unwrap();
        r.add_load_rep(9, Some(&f)).map(|_| ()).unwrap();
        let f = r.save_all_rep(Some(&f)).unwrap();
        let mut back = Reputations::default();
        assert_eq!(back.add_load_rep(4, Some(&f)), None);
        assert_eq!(back.get(4).unwrap().rep, 6);
        // Removing saves and forgets.
        r.decrease_rep(4).unwrap();
        let f2 = r.remove_save_rep(4, Some(&f)).unwrap();
        assert!(r.get(4).is_none());
        assert_eq!(parse_rep_file(&f2).unwrap()[0].rep, 5);
        assert_eq!(r.remove_save_rep(4, Some(&f2)), None);
        // A corrupt file: no reputation at all.
        let mut c = Reputations::default();
        assert_eq!(c.add_load_rep(1, Some(&[0; 5])), None);
        assert!(c.get(1).is_none());
        assert_eq!(Reputations::default().save_all_rep(None), None);
    }

    #[test]
    fn reputation_values() {
        let mut r = Reputations::default();
        assert_eq!(r.average_rep_on_server(), 0);
        r.add_rep(1, 10);
        r.add_rep(1, 0);
        r.add_rep(2, 3);
        assert_eq!(r.all().len(), 2);
        assert_eq!(r.average_rep_on_server(), 6);
        r.increase_rep(1).unwrap();
        assert_eq!(r.get(1).unwrap().rep, 10);
        r.decrease_rep(2).unwrap();
        assert_eq!(r.get(2).unwrap().rep, 2);
        assert_eq!(r.increase_rep(7), Err(BAD_REP_DATA));
        assert_eq!(r.increase_rep(-1), Ok(()));
        r.get_create_rep(8).rep = 1;
        assert_eq!(r.get_create_rep(8).rep, 1);
        assert_eq!(r.client_rep(Some(8)).0, 1);
        assert_eq!(r.client_rep(Some(-1)), (5, None));
        assert!(r.client_rep(None).1.unwrap().contains("Bad pClient"));
        assert!(r.client_rep(Some(77)).1.unwrap().contains("(id: 77)"));
        // Liked players: only 1 (rep 10).
        assert_eq!(r.random_client(1, &mut Fixed(vec![]), Some), Some(1));
        // Disliked: 8 (top) then 2, RANDOM_LONG picks index 1.
        assert_eq!(r.random_client(-1, &mut Fixed(vec![1]), Some), Some(2));
        assert_eq!(r.random_client(0, &mut Fixed(vec![]), Some), None);
        let lines = r.print_reps("Bot", |id| (id != 2).then(|| format!("P{id}")));
        assert_eq!(lines, vec!["Bot hates P8 (1)", "Bot LURRVESS P1 !!! (10)"]);
        assert_eq!(Reputation { player_rep_id: 0, rep: 11 }.describe("a", "b"), None);
    }

    #[test]
    fn rep_rolls_and_arrays() {
        assert_eq!(rep_array_num(2).0, 0);
        assert_eq!(rep_array_num(3).0, 1);
        assert_eq!(rep_array_num(6).0, 1);
        assert_eq!(rep_array_num(7).0, 2);
        assert_eq!(rep_array_num(-1), (1, Some("GetRepArrayNum: Bad rep received (-1)".into())));
        // rep 5: decrease when RANDOM_LONG(0,100) >= 50.
        assert!(decrease_rep_roll(5, 0.0, 0.0, &mut Fixed(vec![50])));
        assert!(!decrease_rep_roll(5, 0.0, 0.0, &mut Fixed(vec![49])));
        // Too skilled for the distance: no draw, no decrease.
        assert!(!decrease_rep_roll(0, 100.0, 50.0, &mut Fixed(vec![100])));
        assert!(decrease_rep_roll(0, 800.0, 100.0, &mut Fixed(vec![0])));
        assert!(decrease_rep_roll(0, -1.0, 100.0, &mut Fixed(vec![0])));
        assert!(increase_rep_roll(5, 0.0, 0.0, &mut Fixed(vec![10])));
        assert!(!increase_rep_roll(-1, 0.0, 0.0, &mut XorShiftRandom::new(1)));
    }

    #[test]
    fn player_ids_file() {
        assert_eq!(player_rep_id("", None), PlayerRepId { id: -1, write: None });
        let first = player_rep_id("Bob", None);
        assert_eq!(first.id, 0);
        let f = first.write.unwrap();
        assert_eq!(f, format!("{BOT_PLAYER_ID_FILE_HEADER}\"Bob\"\n").into_bytes());
        assert_eq!(player_rep_id("Bob", Some(&f)), PlayerRepId { id: 0, write: None });
        let al = player_rep_id("Al", Some(&f));
        assert_eq!(al.id, 1);
        let f = al.write.unwrap();
        assert!(f.ends_with(b"\"Bob\"\n\"Al\"\n"));
        // An empty line counts as an id; a line without quotes too; case matters.
        let f = b"# c\n\nnoquotes\n\"Al\"\n";
        assert_eq!(player_rep_id("Al", Some(f)).id, 2);
        assert_eq!(player_rep_id("al", Some(f)).id, 3);
        // A quote in a name never matches: appended again every time.
        let q = player_rep_id("a\"b", Some(b"\"a\"b\"\n"));
        assert_eq!(q.id, 1);
        assert!(q.write.is_some());
    }
}
