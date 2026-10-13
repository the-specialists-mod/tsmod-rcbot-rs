//! Bot profiles: `bot_profile_t` (`bot.h`), `BotFunc_InitProfile`, `BotFunc_ReadProfile`,
//! `BotFunc_WriteProfile` (`dll.cpp`) and the profile choice of `BotFunc_AddBot`
//! (`bot_commands.cpp`), on the files in the bot folder:
//!
//! * **`bot_profiles.ini`**: the profile ids a new bot may use ([`parse_profile_list`]).
//! * **`botprofiles/<id>.ini`**: one profile ([`BotProfile::read`]). As upstream reads it:
//!   `fgets(buffer, 127)` chunks; `#` at the start of a chunk is a comment; one trailing `'\n'` is
//!   removed; then each line is matched by prefix (case-sensitive, no spaces allowed around
//!   `=`). Numbers are read with `atoi`/`atof` (`skill=12abc` is 12, `skill=abc` is 0). Strings
//!   are read from a fixed offset to the next `"` or the end of the line: the offset skips the
//!   opening quote of `name="…"`, `favmap="…"`, `spray="…"` and the `hal_*_file="…"` keys, but
//!   not of `model=`, so `model="gordon"` gives an empty model and `model=gordon` works.
//!   `bottomcolor`/`topcolor` are clamped to 0-255.
//!
//! ```text
//! name="[RCB]Neo"
//! skill=80
//! favteam=1
//! aim_speed=0.4
//! ```
//!
//! The NS keys (`gorge_percent`, `lerk_percent`, `fade_percent`, `onos_percent`) and `favmod`,
//! `favmap`, `spray`, `numgames` are read and written but nothing uses them on TS.

use super::cstr::{atof_f32, atoi, c_str, fgets_chunks, is_c_space, lossy, text_mode};
use super::rep::Reputations;
use super::{Config, DEF_BOT_SKILL, TEAM_AUTOTEAM};
use crate::Random;

/// `BOT_DEFAULT_NAME`.
pub const BOT_DEFAULT_NAME: &str = "RCBot";

/// `bot_profile_t`. Strings upstream keeps as `NULL` are `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BotProfile {
    /// `m_iProfileId`: the `<id>.ini`/`<id>.rcr` number; 0 for none.
    pub profile_id: i32,
    /// `m_iFavMod` (`favmod=`).
    pub fav_mod: i32,
    /// `m_iFavTeam` (`favteam=`): the team the bot joins, [`TEAM_AUTOTEAM`] for any.
    pub fav_team: i32,
    /// `m_iSkill` (`skill=`).
    pub skill: i32,
    /// `m_szFavMap` (`favmap=`).
    pub fav_map: Option<String>,
    /// `m_szSpray` (`spray=`).
    pub spray: Option<String>,
    /// `m_szBotName` (`name=`).
    pub bot_name: Option<String>,
    /// `m_GorgePercent` (`gorge_percent=`, NS).
    pub gorge_percent: i32,
    /// `m_LerkPercent` (`lerk_percent=`, NS).
    pub lerk_percent: i32,
    /// `m_FadePercent` (`fade_percent=`, NS).
    pub fade_percent: i32,
    /// `m_OnosPercent` (`onos_percent=`, NS).
    pub onos_percent: i32,
    /// `m_iNumGames` (`numgames=`).
    pub num_games: i32,
    /// `m_iBottomColour` (`bottomcolor=`).
    pub bottom_colour: i32,
    /// `m_iTopColour` (`topcolor=`).
    pub top_colour: i32,
    /// `m_fAimSpeed` (`aim_speed=`).
    pub aim_speed: f32,
    /// `m_fAimSkill` (`aim_skill=`).
    pub aim_skill: f32,
    /// `m_fAimTime` (`aim_time=`).
    pub aim_time: f32,
    /// `m_iPathRevs` (`max_path_revs=`).
    pub path_revs: i32,
    /// `m_iVisionRevs` (`max_update_vision_revs=`).
    pub vision_revs: i32,
    /// `m_fVisionTime` (`update_vision_time=`).
    pub vision_time: f32,
    /// `m_szHAL_PreTrainFile` (`hal_pretrain_file=`).
    pub hal_pretrain_file: Option<String>,
    /// `m_szHAL_AuxFile` (`hal_aux_file=`).
    pub hal_aux_file: Option<String>,
    /// `m_szHAL_BanFile` (`hal_ban_file=`).
    pub hal_ban_file: Option<String>,
    /// `m_szHAL_SwapFile` (`hal_swap_file=`).
    pub hal_swap_file: Option<String>,
    /// `m_Rep`: the bot's reputations.
    pub rep: Reputations,
    /// `m_iClass` (`class=`): -1 for none.
    pub class: i32,
    /// `m_szModel` (`model=`): the player model, if the profile sets one.
    pub model: Option<String>,
}

/// The size of `szTemp` in `BotFunc_ReadProfile`, which every string value goes through.
const PROFILE_STRING_LEN: usize = 64;

impl BotProfile {
    /// `BotFunc_InitProfile`: the defaults a profile file is read over. Draws the bottom, then the
    /// top colour with `RANDOM_LONG(0, 255)`; the A* and vision budgets come from the config.
    pub fn init(config: &Config, rng: &mut dyn Random) -> BotProfile {
        let bottom_colour = rng.random_long(0, 255);
        let top_colour = rng.random_long(0, 255);
        BotProfile {
            model: None,
            class: -1,
            fade_percent: 0,
            gorge_percent: 0,
            fav_mod: 0,
            fav_team: TEAM_AUTOTEAM,
            profile_id: 0,
            skill: DEF_BOT_SKILL,
            lerk_percent: 50,
            onos_percent: 50,
            bot_name: Some(BOT_DEFAULT_NAME.into()),
            fav_map: None,
            spray: None,
            num_games: 0,
            hal_aux_file: None,
            hal_ban_file: None,
            hal_pretrain_file: None,
            hal_swap_file: None,
            aim_speed: 0.25,
            aim_skill: 0.2,
            aim_time: 1.0,
            bottom_colour,
            top_colour,
            path_revs: config.max_path_revs,
            vision_revs: config.max_vis_update_revs,
            vision_time: config.update_vis_time,
            rep: Reputations::default(),
        }
    }

    /// The key/value part of `BotFunc_ReadProfile` over the file's bytes. Upstream then wakes the
    /// bot's MegaHAL brain and loads its reputation with every player on the server
    /// ([`Reputations::add_load_rep`]); those belong to the caller.
    ///
    /// Deviation: upstream copies string values into a 64-byte buffer without a bound; the port
    /// keeps the first 63 bytes.
    pub fn read(&mut self, data: &[u8]) {
        let data = &text_mode(data)[..];
        for chunk in fgets_chunks(data, 127) {
            let mut b = c_str(chunk);
            if b.first() == Some(&b'#') {
                continue;
            }
            if b.last() == Some(&b'\n') {
                b = &b[..b.len() - 1];
            }
            let text = lossy(b);
            let num = |at: usize| atoi(text.get(at..).unwrap_or(""));
            let float = |at: usize| atof_f32(text.get(at..).unwrap_or(""));
            let string = |at: usize| {
                let rest = b.get(at..).unwrap_or(&[]);
                let n = rest.iter().take_while(|&&c| c != b'"').count().min(PROFILE_STRING_LEN - 1);
                Some(lossy(&rest[..n]))
            };
            let k = |p: &[u8]| b.starts_with(p);
            if k(b"name=") {
                self.bot_name = string(6);
            } else if k(b"class=") {
                self.class = num(6);
            } else if k(b"model=") {
                self.model = string(6);
            } else if k(b"favmod=") {
                self.fav_mod = num(7);
            } else if k(b"favteam=") {
                self.fav_team = num(8);
            } else if k(b"favmap=") {
                self.fav_map = string(8);
            } else if k(b"skill=") {
                self.skill = num(6);
            } else if k(b"spray=") {
                self.spray = string(7);
            } else if k(b"gorge_percent=") {
                self.gorge_percent = num(14);
            } else if k(b"lerk_percent=") {
                self.lerk_percent = num(13);
            } else if k(b"fade_percent=") {
                self.fade_percent = num(13);
            } else if k(b"onos_percent=") {
                self.onos_percent = num(13);
            } else if k(b"aim_speed=") {
                self.aim_speed = float(10);
            } else if k(b"aim_skill=") {
                self.aim_skill = float(10);
            } else if k(b"aim_time=") {
                self.aim_time = float(9);
            } else if k(b"bottomcolor=") {
                self.bottom_colour = num(12).clamp(0, 255);
            } else if k(b"topcolor=") {
                self.top_colour = num(9).clamp(0, 255);
            } else if k(b"numgames=") {
                self.num_games = num(9);
            } else if k(b"hal_pretrain_file=") {
                self.hal_pretrain_file = string(19);
            } else if k(b"hal_aux_file=") {
                self.hal_aux_file = string(14);
            } else if k(b"hal_ban_file=") {
                self.hal_ban_file = string(14);
            } else if k(b"hal_swap_file=") {
                self.hal_swap_file = string(15);
            } else if k(b"max_path_revs=") {
                self.path_revs = num(14);
            } else if k(b"max_update_vision_revs=") {
                self.vision_revs = num(23);
            } else if k(b"update_vision_time=") {
                self.vision_time = float(19);
            }
        }
    }

    /// The text of `BotFunc_WriteProfile`: `name`, `favmod`, `favteam`, `favmap`, `skill`,
    /// `spray`, the four NS percentages and `numgames`, in that order. Upstream then saves the
    /// reputations and the MegaHAL brain; those belong to the caller.
    ///
    /// Upstream's quirk is kept: a string that is `NULL` is written as its bare tag without a
    /// newline, so the next key ends up on its line (`favmap=skill=50`) and is lost on reading.
    /// Only the old disconnect code, now commented out, ever called it.
    pub fn write(&self) -> String {
        let mut out = String::new();
        let int = |out: &mut String, tag: &str, v: i32| out.push_str(&format!("{tag}{v}\n"));
        let string = |out: &mut String, tag: &str, v: &Option<String>| match v {
            Some(v) => out.push_str(&format!("{tag}\"{v}\"\n")),
            None => out.push_str(tag),
        };
        string(&mut out, "name=", &self.bot_name);
        int(&mut out, "favmod=", self.fav_mod);
        int(&mut out, "favteam=", self.fav_team);
        string(&mut out, "favmap=", &self.fav_map);
        int(&mut out, "skill=", self.skill);
        string(&mut out, "spray=", &self.spray);
        int(&mut out, "gorge_percent=", self.gorge_percent);
        int(&mut out, "lerk_percent=", self.lerk_percent);
        int(&mut out, "fade_percent=", self.fade_percent);
        int(&mut out, "onos_percent=", self.onos_percent);
        int(&mut out, "numgames=", self.num_games);
        out
    }
}

/// A profile's file name: `"%d.ini"`, in [`super::BOT_PROFILES_FOLDER`].
pub fn profile_file_name(profile_id: i32) -> String {
    format!("{profile_id}.ini")
}

/// One entry of `bot_profiles.ini` as `BotFunc_AddBot` reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileListEntry {
    /// `atoi` of the word: the profile id (0 entries are skipped by the caller).
    pub id: i32,
    /// The word with `.ini` appended: the file upstream checks for. It is not always
    /// `"<id>.ini"` (`12abc` checks `12abc.ini` but loads `12.ini`).
    pub file: String,
}

/// `bot_profiles.ini` as `BotFunc_AddBot` reads it: `fscanf(fp, "%9s\n", …)` until end of file,
/// so whitespace-separated words of at most 9 characters (longer words are split into several),
/// each read with `atoi`.
///
/// Deviation: on a file with no word at all upstream runs `atoi` on an uninitialised buffer once;
/// the port reads nothing. Upstream also appends `.ini` to the 10-byte word buffer, overrunning
/// it for words of 6 or more characters.
pub fn parse_profile_list(data: &[u8]) -> Vec<ProfileListEntry> {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < data.len() && is_c_space(data[i]) {
            i += 1;
        }
        if i >= data.len() {
            break;
        }
        let start = i;
        while i < data.len() && !is_c_space(data[i]) && i - start < 9 {
            i += 1;
        }
        let word = lossy(&data[start..i]);
        out.push(ProfileListEntry { id: atoi(&word), file: format!("{word}.ini") });
    }
    out
}

/// What the profile-choosing part of `BotFunc_AddBot` decided.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileChoice {
    /// Read this profile id from `<id>.ini` (its contents are attached).
    Chosen {
        /// The profile id.
        id: i32,
        /// The `<id>.ini` contents.
        data: Vec<u8>,
    },
    /// "Error: No Bot Profiles have been found that are free...".
    NoneFree,
}

/// The profile choice of `BotFunc_AddBot` for a fresh bot slot. Reads `bot_profiles.ini`
/// (`list`, `None`: no file); skips ids that are 0 or used by a bot in the game (`in_use`);
/// keeps those whose file exists (`read_profile(file name)`), reporting the others; picks one
/// with `RANDOM_LONG(0, n - 1)` and reads its `"%d.ini"`. Returns the choice and upstream's
/// messages.
pub fn choose_profile(
    list: Option<&[u8]>,
    in_use: &[i32],
    rng: &mut dyn Random,
    mut read_profile: impl FnMut(&str) -> Option<Vec<u8>>,
) -> (ProfileChoice, Vec<String>) {
    let mut msgs = Vec::new();
    let Some(list) = list else {
        msgs.push("Error: No Bot Profiles have been found that are free...".into());
        return (ProfileChoice::NoneFree, msgs);
    };
    let mut can_use = Vec::new();
    for e in parse_profile_list(list) {
        if e.id == 0 || in_use.contains(&e.id) {
            continue;
        }
        if read_profile(&e.file).is_some() {
            can_use.push(e.id);
        } else {
            msgs.push(format!("Error: Bot Profile : {}.ini Expected, Profile Not Found", e.id));
        }
    }
    if !can_use.is_empty() {
        let id = can_use[rng.random_long(0, can_use.len() as i32 - 1).clamp(0, can_use.len() as i32 - 1) as usize];
        if let Some(data) = read_profile(&profile_file_name(id)) {
            return (ProfileChoice::Chosen { id, data }, msgs);
        }
    }
    msgs.push("Error: No Bot Profiles have been found that are free...".into());
    (ProfileChoice::NoneFree, msgs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    struct Seq(Vec<i32>);
    impl Random for Seq {
        fn random_long(&mut self, lo: i32, _: i32) -> i32 {
            if self.0.is_empty() { lo } else { self.0.remove(0) }
        }
        fn random_float(&mut self, lo: f32, _: f32) -> f32 {
            lo
        }
    }

    #[test]
    fn init_defaults() {
        let c = Config { max_path_revs: 33, ..Config::default() };
        let p = BotProfile::init(&c, &mut Seq(vec![10, 20]));
        assert_eq!((p.bottom_colour, p.top_colour), (10, 20));
        assert_eq!(p.bot_name.as_deref(), Some("RCBot"));
        assert_eq!((p.fav_team, p.skill, p.class, p.path_revs, p.vision_revs), (5, 50, -1, 33, 200));
        assert_eq!((p.lerk_percent, p.onos_percent, p.gorge_percent, p.fade_percent), (50, 50, 0, 0));
        assert_eq!((p.aim_speed, p.aim_skill, p.aim_time), (0.25, 0.2, 1.0));
    }

    #[test]
    fn read_like_upstream() {
        let mut p = BotProfile::init(&Config::default(), &mut XorShiftRandom::new(1));
        let text = "# a comment\n\
                    name=\"[RCB]Neo\"\n\
                    skill=80x\n\
                    favteam=2\n\
                    class=3\n\
                    model=\"gordon\"\n\
                    favmap=\"ts_dojo\"\n\
                    spray=\"logo\"\n\
                    aim_speed=0.5\n\
                    aim_skill=abc\n\
                    topcolor=300\n\
                    bottomcolor=-4\n\
                    \x20skill=1\n\
                    gorge_percent=12\n\
                    onos_percent=7\n\
                    numgames=9\n\
                    hal_aux_file=\"aux.txt\"\n\
                    max_path_revs=50\n\
                    update_vision_time=0.25\r\n\
                    unknown=1\n";
        p.read(text.as_bytes());
        assert_eq!(p.bot_name.as_deref(), Some("[RCB]Neo"));
        assert_eq!(p.skill, 80);
        assert_eq!(p.fav_team, 2);
        assert_eq!(p.class, 3);
        // model= doesn't skip the quote.
        assert_eq!(p.model.as_deref(), Some(""));
        assert_eq!(p.fav_map.as_deref(), Some("ts_dojo"));
        assert_eq!(p.spray.as_deref(), Some("logo"));
        assert_eq!((p.aim_speed, p.aim_skill), (0.5, 0.0));
        assert_eq!((p.top_colour, p.bottom_colour), (255, 0));
        assert_eq!((p.gorge_percent, p.onos_percent, p.num_games), (12, 7, 9));
        assert_eq!(p.hal_aux_file.as_deref(), Some("aux.txt"));
        assert_eq!((p.path_revs, p.vision_time), (50, 0.25));

        let mut q = BotProfile::default();
        // No quote: the first character is lost; a long name is cut to 63 bytes.
        q.read(b"name=Neo\nmodel=gordon\nfavmap=");
        assert_eq!(q.bot_name.as_deref(), Some("eo"));
        assert_eq!(q.model.as_deref(), Some("gordon"));
        assert_eq!(q.fav_map.as_deref(), Some(""));
        q.read(format!("name=\"{}\"", "n".repeat(100)).as_bytes());
        assert_eq!(q.bot_name.unwrap().len(), 63);
    }

    #[test]
    fn write_and_read_back() {
        let mut p = BotProfile::init(&Config::default(), &mut XorShiftRandom::new(2));
        p.bot_name = Some("Trinity".into());
        p.fav_map = Some("ts_bikini".into());
        p.spray = Some("s".into());
        p.skill = 77;
        p.fav_team = 1;
        p.gorge_percent = 1;
        p.lerk_percent = 2;
        p.fade_percent = 3;
        p.onos_percent = 4;
        p.num_games = 12;
        p.fav_mod = 8;
        let text = p.write();
        assert_eq!(
            text,
            "name=\"Trinity\"\nfavmod=8\nfavteam=1\nfavmap=\"ts_bikini\"\nskill=77\nspray=\"s\"\n\
             gorge_percent=1\nlerk_percent=2\nfade_percent=3\nonos_percent=4\nnumgames=12\n"
        );
        let mut back = BotProfile::init(&Config::default(), &mut XorShiftRandom::new(2));
        back.read(text.as_bytes());
        assert_eq!(back, p);
        assert_eq!(back.write(), text);

        // The NULL-string quirk: favmap= swallows skill.
        let mut n = p.clone();
        n.fav_map = None;
        let text = n.write();
        assert!(text.contains("favteam=1\nfavmap=skill=77\n"));
        let mut back = BotProfile::init(&Config::default(), &mut XorShiftRandom::new(2));
        back.read(text.as_bytes());
        assert_eq!(back.fav_map.as_deref(), Some("kill=77"));
        assert_eq!(back.skill, 50);
    }

    #[test]
    fn profile_list_and_choice() {
        let list = parse_profile_list(b"  1\n2 12abc\n0\n1234567890 -3");
        let ids: Vec<i32> = list.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 12, 0, 123456789, 0, -3]);
        assert_eq!(list[2].file, "12abc.ini");
        assert_eq!(list[5].file, "0.ini");
        assert!(parse_profile_list(b" \n ").is_empty());

        let files = |name: &str| match name {
            "1.ini" | "2.ini" | "12abc.ini" => Some(name.as_bytes().to_vec()),
            _ => None,
        };
        // 2 is in use; 12abc.ini exists but 12.ini doesn't; 123456789 and -3 are missing.
        let (c, msgs) = choose_profile(Some(b"1 2 12abc 123456789 -3"), &[2], &mut Seq(vec![0]), files);
        assert_eq!(c, ProfileChoice::Chosen { id: 1, data: b"1.ini".to_vec() });
        assert_eq!(
            msgs,
            vec![
                "Error: Bot Profile : 123456789.ini Expected, Profile Not Found",
                "Error: Bot Profile : -3.ini Expected, Profile Not Found"
            ]
        );
        let (c, msgs) = choose_profile(Some(b"1 2 12abc"), &[2], &mut Seq(vec![1]), files);
        assert_eq!(c, ProfileChoice::NoneFree);
        assert_eq!(msgs, vec!["Error: No Bot Profiles have been found that are free..."]);
        assert_eq!(choose_profile(None, &[], &mut Seq(vec![]), files).0, ProfileChoice::NoneFree);
        assert_eq!(profile_file_name(4), "4.ini");
    }
}
