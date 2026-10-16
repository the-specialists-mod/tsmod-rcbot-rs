//! RCBot's configuration, users, profiles, reputation, menus and the `rcbot` command set
//! (`globals.cpp`, `dll.cpp`, `bot_commands.{h,cpp}`, `bot_menu.{h,cpp}`, `bot_rep.cpp`, the
//! `CBotGlobals`, `CClient`, `CAllowedPlayer(s)`, `CBotReputation(s)` and `bot_profile_t` parts of
//! `bot.h`).
//!
//! Upstream keeps all of this in `gBotGlobals`; here the configurable part is [`Config`], the
//! users are [`AllowedPlayers`], the per-player state the commands touch is [`Clients`], and
//! everything else a command does (adding bots, waypoint edits, squads, the bot cam, messages,
//! file I/O) is a call on [`CommandHost`]. Nothing here knows the plugin API.
//!
//! File I/O stays with the host: readers take the file's bytes, writers return the new contents,
//! and [`file_name`] names files as upstream's `UTIL_BuildFileName` does.

pub mod client;
pub mod commands;
pub mod cstr;
pub mod file;
pub mod menu;
pub mod profile;
pub mod rep;
pub mod users;

pub use client::{Client, Clients, PlayerId, ToolTip};
pub use commands::{CommandContext, CommandHost, CommandId, CvarState, Print};
pub use file::{ConfigLine, read_bot_folder, read_config};
pub use menu::{Menu, MenuAction, MenuId, MenuItem};
pub use profile::BotProfile;
pub use rep::{Reputation, Reputations};
pub use users::{AllowedPlayer, AllowedPlayers};

/// `BOT_VER` (`bot_const.h`, the plain non-debug, non-metamod build).
pub const BOT_VER: &str = "1.51b14";
/// `BOT_COMMAND_ACCESS`: the client command that runs bot commands.
pub const BOT_COMMAND_ACCESS: &str = "rcbot";
/// `BOT_DBG_MSG_TAG`: the prefix of RCBot's console messages.
pub const BOT_DBG_MSG_TAG: &str = "[RCBOT] ";
/// `MAX_PLAYERS`.
pub const MAX_PLAYERS: usize = 32;
/// `MAX_BOT_SKILL`.
pub const MAX_BOT_SKILL: i32 = 100;
/// `MIN_BOT_SKILL`.
pub const MIN_BOT_SKILL: i32 = 1;
/// `DEF_BOT_SKILL`: `(MAX_BOT_SKILL + MIN_BOT_SKILL) / 2`.
pub const DEF_BOT_SKILL: i32 = (MAX_BOT_SKILL + MIN_BOT_SKILL) / 2;
/// `TEAM_AUTOTEAM`: a profile's "any team".
pub const TEAM_AUTOTEAM: i32 = 5;
/// `DEFAULT_BOT_CHAT_PERCENT`.
pub const DEFAULT_BOT_CHAT_PERCENT: i16 = 15;
/// `BOT_WAYPOINT_TOUCH_DIST`.
pub const BOT_WAYPOINT_TOUCH_DIST: f32 = 32.0;

/// `BOT_CONFIG_*` (`bot_const.h`): the bits of [`Config::config_settings`]. The whole set is kept
/// because the bits are what `rcbot config` sets; the NS-only ones have no `config` sub-command
/// in this port and nothing reads them.
pub mod flags {
    /// `BOT_CONFIG_WAIT_FOR_ORDERS`.
    pub const WAIT_FOR_ORDERS: i32 = 1 << 0;
    /// `BOT_CONFIG_DONT_SHOOT`.
    pub const DONT_SHOOT: i32 = 1 << 1;
    /// `BOT_CONFIG_CHATTING`.
    pub const CHATTING: i32 = 1 << 2;
    /// `BOT_CONFIG_REAL_MODE`.
    pub const REAL_MODE: i32 = 1 << 3;
    /// `BOT_CONFIG_COMMANDING`.
    pub const COMMANDING: i32 = 1 << 4;
    /// `BOT_CONFIG_RESERVE_BOT_SLOTS`.
    pub const RESERVE_BOT_SLOTS: i32 = 1 << 5;
    /// `BOT_CONFIG_CHAT_REPLY_TO_BOTS`.
    pub const CHAT_REPLY_TO_BOTS: i32 = 1 << 6;
    /// `BOT_CONFIG_MARINE_AUTO_BUILD` (NS).
    pub const MARINE_AUTO_BUILD: i32 = 1 << 7;
    /// `BOT_CONFIG_CHAT_DONT_LEARN`.
    pub const CHAT_DONT_LEARN: i32 = 1 << 8;
    /// `BOT_CONFIG_BALANCE_TEAMS`.
    pub const BALANCE_TEAMS: i32 = 1 << 9;
    /// `BOT_CONFIG_TOOLTIPS`.
    pub const TOOLTIPS: i32 = 1 << 10;
    /// `BOT_CONFIG_ENABLE_BOTCAM`.
    pub const ENABLE_BOTCAM: i32 = 1 << 11;
    /// `BOT_CONFIG_AUTOWAYPOINT_HUMANS`.
    pub const AUTOWAYPOINT_HUMANS: i32 = 1 << 12;
    /// `BOT_CONFIG_UNSTICK`.
    pub const UNSTICK: i32 = 1 << 13;
    /// `BOT_CONFIG_ABNORMAL_GAME` (NS: marines v marines).
    pub const ABNORMAL_GAME: i32 = 1 << 14;
    /// `BOT_CONFIG_BOTS_LEAVE_AND_JOIN`.
    pub const BOTS_LEAVE_AND_JOIN: i32 = 1 << 15;
    /// `BOT_CONFIG_BOTS_WAIT_FOR_BOTS`.
    pub const BOTS_WAIT_FOR_BOTS: i32 = 1 << 16;
    /// `BOT_CONFIG_NOT_NS3_FINAL` (NS).
    pub const NOT_NS3_FINAL: i32 = 1 << 17;
    /// `BOT_CONFIG_BLINKING` (NS fades).
    pub const BLINKING: i32 = 1 << 18;
    /// `BOT_CONFIG_WAIT_AT_RESOURCES` (NS).
    pub const WAIT_AT_RESOURCES: i32 = 1 << 19;
    /// `BOT_CONFIG_DISABLE_WEAPON_LEARN`.
    pub const DISABLE_WEAPON_LEARN: i32 = 1 << 20;
    /// `BOT_CONFIG_TS_KUNGFU`.
    pub const TS_KUNGFU: i32 = 1 << 21;
    /// `BOT_CONFIG_DISABLE_BOT_SQUADS`.
    pub const DISABLE_BOT_SQUADS: i32 = 1 << 22;
    /// `BOT_CONFIG_TS_DONT_STEAL_WEAPONS`.
    pub const TS_DONT_STEAL_WEAPONS: i32 = 1 << 23;
    /// `BOT_CONFIG_TS_DONT_PICKUP_WEAPONS`.
    pub const TS_DONT_PICKUP_WEAPONS: i32 = 1 << 24;
    /// Port addition (`tactical_paths`, design §4): paths avoid dangerous and exposed waypoints
    /// and teammates' paths ([`crate::nav::Tactics`]); on by default. Off: upstream's shortest
    /// paths.
    pub const TACTICAL_PATHS: i32 = 1 << 25;
}

/// `BOT_DEBUG_*_LEVEL` (`bot_const.h`): the bits of [`Config::debug_levels`].
pub mod debug {
    /// `BOT_DEBUG_TOUCH_LEVEL`.
    pub const TOUCH: i32 = 1 << 0;
    /// `BOT_DEBUG_THINK_LEVEL`.
    pub const THINK: i32 = 1 << 1;
    /// `BOT_DEBUG_HEAR_LEVEL`.
    pub const HEAR: i32 = 1 << 2;
    /// `BOT_DEBUG_MESSAGE_LEVEL`.
    pub const MESSAGE: i32 = 1 << 3;
    /// `BOT_DEBUG_BLOCK_LEVEL`.
    pub const BLOCK: i32 = 1 << 4;
    /// `BOT_DEBUG_MOVE_LEVEL`.
    pub const MOVE: i32 = 1 << 5;
    /// `BOT_DEBUG_AIM_LEVEL`.
    pub const AIM: i32 = 1 << 6;
    /// `BOT_DEBUG_NAV_LEVEL`.
    pub const NAV: i32 = 1 << 7;
    /// `BOT_DEBUG_SEE_LEVEL`.
    pub const SEE: i32 = 1 << 8;
}

/// The configurable part of `CBotGlobals`: what `bot_config.ini`, `rcbot_folder.ini` and the
/// `rcbot config`/`debug`/`pathwaypoint` commands set. The NS-only values (`m_fGorgeAmount`,
/// `m_fWallStickTol`, the `u_*` importances) are left out with their commands.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// `m_iConfigSettings`: [`flags`] bits.
    pub config_settings: i32,
    /// `m_fTurnSpeed`.
    pub turn_speed: f32,
    /// `m_iForceTeam`: 1-based index into `mp_teamlist` for new bots in TS teamplay; 0 or less
    /// for none.
    pub force_team: i32,
    /// `m_fBotStuckSpeed`.
    pub bot_stuck_speed: f32,
    /// `m_fUpdateLadderTime`.
    pub update_ladder_time: f32,
    /// `m_iMinBots` (-1: off).
    pub min_bots: i16,
    /// `m_iMaxBots` (-1: off).
    pub max_bots: i16,
    /// `m_iBotChatPercent`.
    pub bot_chat_percent: i16,
    /// `m_iBotChatReplyPercent`.
    pub bot_chat_reply_percent: i16,
    /// `m_iMaxPathRevs`: A* expansions per frame, copied into new profiles.
    pub max_path_revs: i32,
    /// `m_iMaxVisUpdateRevs`, copied into new profiles.
    pub max_vis_update_revs: i32,
    /// `m_fUpdateVisTime`, copied into new profiles.
    pub update_vis_time: f32,
    /// `m_bAutoPathWaypoint`: new waypoints get paths automatically.
    pub auto_path_waypoint: bool,
    /// `m_iDebugLevels`: [`debug`] bits.
    pub debug_levels: i32,
    /// `m_szModFolder`: the game directory (`ts`), where `rcbot_folder.ini` is looked for.
    pub mod_folder: String,
    /// `m_szBotFolder`: where every other file lives (default `rcbot`).
    pub bot_folder: String,
}

impl Default for Config {
    /// What upstream has when `GameInit` reads `bot_config.ini`: the constructor's values, then
    /// `Init()` → `setDefaults()` again at plugin load (`Meta_Attach` / `GiveFnptrsToDll`), which
    /// wins where they differ (config bits, chat percent, auto paths), then `GameInit`'s
    /// `m_iForceTeam = -1`. The bot folder is what `ReadBotFolder` falls back to.
    fn default() -> Config {
        Config {
            config_settings: flags::TOOLTIPS | flags::BLINKING | flags::TACTICAL_PATHS,
            turn_speed: 15.0,
            force_team: -1,
            bot_stuck_speed: 7.0,
            update_ladder_time: -1.0,
            min_bots: -1,
            max_bots: -1,
            bot_chat_percent: DEFAULT_BOT_CHAT_PERCENT,
            bot_chat_reply_percent: 33,
            max_path_revs: 200,
            max_vis_update_revs: 200,
            update_vis_time: 0.0,
            auto_path_waypoint: true,
            debug_levels: 0,
            mod_folder: "ts".into(),
            bot_folder: "rcbot".into(),
        }
    }
}

impl Config {
    /// `IsConfigSettingOn`.
    pub fn is_on(&self, setting: i32) -> bool {
        (self.config_settings & setting) > 0
    }

    /// `ChangeConfigSetting`: 0 turns it off, anything else on.
    pub fn change_setting(&mut self, setting: i32, state: i32) {
        if state == 0 {
            self.config_settings &= !setting;
        } else {
            self.config_settings |= setting;
        }
    }

    /// `IsDebugLevelOn`.
    pub fn is_debug_on(&self, level: i32) -> bool {
        (self.debug_levels & level) > 0
    }

    /// `UTIL_BuildFileName(filename, arg1, arg2)` in this bot folder.
    pub fn file_name(&self, arg1: &str, arg2: &str) -> String {
        file_name(&self.bot_folder, arg1, arg2)
    }
}

/// The path separator `UTIL_BuildFileName` uses: `\` on Windows, `/` elsewhere.
pub const PATH_SEP: char = if cfg!(windows) { '\\' } else { '/' };

/// `UTIL_BuildFileName`: `<bot folder><sep><arg1>[<sep><arg2>]`; `arg2` only counts when both are
/// non-empty, and an empty `arg1` leaves just the folder and a separator.
pub fn file_name(bot_folder: &str, arg1: &str, arg2: &str) -> String {
    let mut s = format!("{bot_folder}{PATH_SEP}");
    if !arg1.is_empty() && !arg2.is_empty() {
        s.push_str(arg1);
        s.push(PATH_SEP);
        s.push_str(arg2);
    } else if !arg1.is_empty() {
        s.push_str(arg1);
    }
    s
}

/// `bot_config.ini`.
pub const BOT_CONFIG_FILE: &str = "bot_config.ini";
/// `BOT_USERS_FILE`.
pub const BOT_USERS_FILE: &str = "bot_users.ini";
/// `BOT_PROFILES_FILE`: the list of profile ids.
pub const BOT_PROFILES_FILE: &str = "bot_profiles.ini";
/// `BOT_PROFILES_FOLDER`: where `<id>.ini` and `<id>.rcr` live.
pub const BOT_PROFILES_FOLDER: &str = "botprofiles";
/// `BOT_PLAYER_ID_FILE`: player names, one per reputation id.
pub const BOT_PLAYER_ID_FILE: &str = "bot_playerids.ini";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_settings() {
        let mut c = Config::default();
        assert!(c.is_on(flags::TOOLTIPS) && c.is_on(flags::BLINKING) && !c.is_on(flags::CHATTING));
        c.change_setting(flags::CHATTING, 7);
        assert!(c.is_on(flags::CHATTING));
        c.change_setting(flags::CHATTING | flags::TOOLTIPS, 0);
        assert!(!c.is_on(flags::CHATTING) && !c.is_on(flags::TOOLTIPS));
        // Multi-bit queries are "any bit".
        assert!(c.is_on(flags::BLINKING | flags::CHATTING));
        assert_eq!(c.force_team, -1);
        assert_eq!(DEF_BOT_SKILL, 50);
        c.debug_levels = debug::NAV;
        assert!(c.is_debug_on(debug::NAV) && !c.is_debug_on(debug::AIM));
    }

    #[test]
    fn build_file_name() {
        let s = PATH_SEP;
        assert_eq!(file_name("rcbot", "botprofiles", "3.ini"), format!("rcbot{s}botprofiles{s}3.ini"));
        assert_eq!(file_name("rcbot", "bot_users.ini", ""), format!("rcbot{s}bot_users.ini"));
        assert_eq!(file_name("rcbot", "", "x"), format!("rcbot{s}"));
        assert_eq!(Config::default().file_name(BOT_CONFIG_FILE, ""), format!("rcbot{s}bot_config.ini"));
    }
}
