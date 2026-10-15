//! The `rcbot` command set (`bot_commands.{h,cpp}`: `CBotCvar`, `CBotCvarList`, `SetupCommands`,
//! every command class and `BotFunc_AddBot`'s argument handling), how commands are reached
//! (`ClientCommand` in `dll.cpp`, `RCBot_ServerCommand`, `CBotGlobals::ReadConfig`), and the
//! menu functions `menuselect` runs (`BotMenu_Func_*`, `bot_menu.cpp`).
//!
//! The model is pure: a command reads and changes [`CommandContext`] (the config, the users, the
//! clients) and does everything else through [`CommandHost`]: queries it can't answer itself
//! (players by name, the waypoint near a player, squads) and side effects ([`Effect`], each one
//! an upstream call, and messages, [`Print`]). The bot manager, squads, waypoint editing and the
//! bot cam are the host's until they are ported.
//!
//! TS only: `config` keeps every TS setting and drops NS's (`marine_auto_build`,
//! `abnormal_game`, `not_ns3final`, `blinking`, `wait_at_res`, `gorge_amount`,
//! `wall_stick_tolerance`, `u_*`); `util` drops `buildres`/`buildcomm`; `waypoint save` drops
//! the WhichBot (NS) and GraveBot converters (design §2). `+rcbot_force_grip`'s effect is the host's.

use super::client::{PlayerId, ToolTip};
use super::cstr::{atof_f32, atoi, f2};
use super::menu::{self, Activated, MenuAction, MenuId};
use super::users::AllowedPlayers;
use super::{BOT_USERS_FILE, BOT_VER, Clients, Config, flags};
use crate::Random;
use crate::math::Vector;
use crate::nav::flags as wf;

/// `RCBOT_ACCESS_FORCE_GRIP` (`bot_const.h`): the access level `+rcbot_force_grip` needs.
pub const RCBOT_ACCESS_FORCE_GRIP: i32 = 9;

/// `BOT_COMMAND_ACCESS_*` (`bot_commands.h`): the access groups. A user's access level is a set
/// of these bits; a command needs any one of its bits (`ALLOWED`, 0: everyone).
pub mod access {
    /// `BOT_COMMAND_ACCESS_ALLOWED`.
    pub const ALLOWED: i32 = 0;
    /// `BOT_COMMAND_ACCESS_WAYPOINTING`.
    pub const WAYPOINTING: i32 = 1 << 0;
    /// `BOT_COMMAND_ACCESS_ADD_REMOVE_BOTS`.
    pub const ADD_REMOVE_BOTS: i32 = 1 << 1;
    /// `BOT_COMMAND_ACCESS_UTIL`.
    pub const UTIL: i32 = 1 << 2;
    /// `BOT_COMMAND_ACCESS_CONFIG`.
    pub const CONFIG: i32 = 1 << 3;
    /// `BOT_COMMAND_ACCESS_DEBUG`.
    pub const DEBUG: i32 = 1 << 4;
    /// `BOT_COMMAND_ACCESS_USERS`.
    pub const USERS: i32 = 1 << 5;
}

/// `eBotCvarState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CvarState {
    /// `BOT_CVAR_NOTEXIST`.
    NotExist,
    /// `BOT_CVAR_ACCESSED`.
    Accessed,
    /// `BOT_CVAR_NEEDACCESS`.
    NeedAccess,
    /// `BOT_CVAR_ERROR`.
    Error,
    /// `BOT_CVAR_CONTINUE`: the engine/game should see the command too (`menuselect`).
    Continue,
}

/// The commands `SetupCommands` registers, in registration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    /// `CAddBotCommand`.
    AddBot,
    /// `CWaypointCommand`.
    Waypoint,
    /// `CPathWaypointCommand`.
    PathWaypoint,
    /// `CDebugCommand`.
    Debug,
    /// `CWaypointMenuCommand`.
    WaypointMenu,
    /// `CBotMenuCommand`.
    BotMenu,
    /// `CMenuSelectCommand`.
    MenuSelect,
    /// `CConfigCommand`.
    Config,
    /// `CSetPassCommand`.
    SetPass,
    /// `CUtilCommand`.
    Util,
    /// `CBotSquadCommand`.
    Squad,
    /// `CSquadMenuCommand`.
    SquadMenu,
    /// `CBotCamCommand`.
    BotCam,
    /// `CAutoWaypointCommand`.
    AutoWaypoint,
    /// `CHelpCommand`.
    Help,
    /// `CUsersCommand`.
    Users,
    /// `CDebugBotCommand`.
    DebugBot,
    /// `CDebugEntCommand`.
    DebugEnt,
    /// `CRemoveBotCommand`.
    RemoveBot,
}

impl CommandId {
    /// `SetupCommands`' order. `CBotCvarList` is a stack, so lists print the other way round.
    pub const ALL: [CommandId; 19] = [
        CommandId::AddBot,
        CommandId::Waypoint,
        CommandId::PathWaypoint,
        CommandId::Debug,
        CommandId::WaypointMenu,
        CommandId::BotMenu,
        CommandId::MenuSelect,
        CommandId::Config,
        CommandId::SetPass,
        CommandId::Util,
        CommandId::Squad,
        CommandId::SquadMenu,
        CommandId::BotCam,
        CommandId::AutoWaypoint,
        CommandId::Help,
        CommandId::Users,
        CommandId::DebugBot,
        CommandId::DebugEnt,
        CommandId::RemoveBot,
    ];

    /// `(name, access level, can be used on a dedicated server)` from `setupCommand`.
    fn setup(self) -> (&'static str, i32, bool) {
        use CommandId::*;
        match self {
            AddBot => ("addbot", access::ADD_REMOVE_BOTS, true),
            Waypoint => ("waypoint", access::WAYPOINTING, false),
            PathWaypoint => ("pathwaypoint", access::WAYPOINTING, false),
            Debug => ("debug", access::DEBUG, true),
            WaypointMenu => ("waypoint_menu", access::WAYPOINTING, false),
            BotMenu => ("bot_menu", access::ADD_REMOVE_BOTS, false),
            MenuSelect => ("menuselect", access::ALLOWED, false),
            Config => ("config", access::CONFIG, true),
            SetPass => ("set_pass", access::ALLOWED, false),
            Util => ("util", access::UTIL, true),
            Squad => ("squad", access::ALLOWED, false),
            SquadMenu => ("squad_menu", access::ALLOWED, false),
            BotCam => ("botcam", access::ALLOWED, false),
            AutoWaypoint => ("autowaypoint", access::WAYPOINTING, true),
            Help => ("help", access::ALLOWED, true),
            Users => ("users", access::USERS, true),
            DebugBot => ("debug_bot", access::DEBUG, false),
            DebugEnt => ("debug_ent", access::DEBUG, false),
            // setupCommand's default: not on a dedicated server.
            RemoveBot => ("removebot", access::ADD_REMOVE_BOTS, false),
        }
    }

    /// `GetCommandName`.
    pub fn name(self) -> &'static str {
        self.setup().0
    }

    /// `m_iAccessLevel`.
    pub fn access_level(self) -> i32 {
        self.setup().1
    }

    /// `CBotCvarList::GetCvar`: exact, case-sensitive.
    pub fn lookup(name: &str) -> Option<CommandId> {
        CommandId::ALL.iter().rev().copied().find(|c| c.name() == name)
    }

    /// `CBotCvar::needAccess`: a command with an access level needs one of its bits.
    pub fn needs_access(self, level: i32) -> bool {
        let need = self.access_level();
        need > 0 && (level & need) == 0
    }

    /// `CBotCvar::error`: not for a dedicated server's console.
    pub fn error(self, dedicated: bool) -> bool {
        !self.setup().2 && dedicated
    }

    /// `showHelp`: the lines the command prints (`"There is no help for this command"` for
    /// those without). The NS-only lines of `config` and `addbot` are left out.
    pub fn help(self, mod_folder: &str) -> Vec<String> {
        use CommandId::*;
        let lines: &[&str] = match self {
            RemoveBot => &["removebot command"],
            DebugBot => &[
                "debug_bot command",
                "=================",
                "commands",
                "--------",
                "\"debug_bot bot <bot name>\" : shows bot task & wpt info on screen",
                "\"debug_bot off\" stops showing bot info on screen",
            ],
            DebugEnt => &[
                "debug_ent command",
                "=================",
                "commands",
                "--------",
                "\"debug_ent entity\" : shows facing entity info on screen",
                "\"debug_ent off\" stops showing info on screen",
            ],
            Users => &[
                "users command",
                "=============",
                "commands: ",
                "---------",
                "readusers (Reads bot_users.ini file again)",
                "adduser -- adds a user\nUsage: <player name> <access level>",
                "removeuser -- removes a user\nUsage: <(player name) / #(user id)>",
                "showusers -- shows all allowed bot users",
            ],
            Help => &[
                "help command",
                "------------",
                "Usage : rcbot help [command name]",
                "You can view useful tips on commands",
                "and how to use them",
                "-- general --",
                "commands may have parameters",
                "- if a parameter is needed that parameter is enclosed in '<' and '>'",
                "  (example: rcbot util teleport [player name])",
                "- if a parameter is optional that parameter is enclosed in '[' and ']'",
                "  (example: rcbot util killbot <bot name>)",
            ],
            AutoWaypoint => &[
                "auto_waypoint command",
                "---------------------",
                "Usage : rcbot autowaypoint [player name] [on (1) | off (0)]",
                "Automatically places waypoints on player",
                "If no player name is issued, it sets autowaypoint ON/OFF for you",
                "If no on/off value is issued, it toggles the autowaypoint state",
                "You must issue a player name if using a dedicated server",
            ],
            SetPass => &[
                "set_pass command help",
                "---------------------",
                "Usage : rcbot set_pass <password>",
                "Allows the bot program to check the entered password",
                "with your name to gain access to more rcbot commands",
                "Your password is case sensitive and your game name is",
                "also case sensitive, they must match on list of users",
            ],
            Util => &[
                "util command help",
                "-----------------",
                "killbot <bot name>",
                "kills the bot with the name of <bot name>\n",
                "set_teleport",
                "sets your current teleport possition, anyone you teleport will teleport where you are\n",
                "teleport [player name]",
                "will teleport the player with [player name] or yourself if it is not issued\n",
                "notouch [player name]",
                "will stop touch calls reaching the player with [player name] or yourself if it is not issued.",
                "no touch means you won't pick up objects and wont go through teleports (needed for waypointing teleports)\n",
                "notarget_mode [player name]",
                "will stop monsters shooting the player with [player name] or yourself if it is not issued.\n",
                "send_sound <sound path>[.wav]",
                "makes the sound in the <sound path> play on everyones client provided they have the sound\n",
                "noclip_mode [player name]",
                "will make the player fly through walls with [player name] or yourself if it is not issued\n",
                "godmode [player name]",
                "will put god mode on the player with [player name] or yourself if it is not issued\n",
                "showboredom : will print bot's boredom (CAREFUL!! May overflow your client [bug?])\n",
                "showreps : will print bots reputations (CAREFUL!! May overflow your client [bug?])\n",
            ],
            AddBot => &[
                "Add bot command help",
                "--------------------",
                "You can add a bot by typing...",
                "\"rcbot addbot\" (creates a random bot)",
            ],
            Waypoint => {
                return [
                    "waypoint command help".to_string(),
                    "---------------------".into(),
                    "Waypoints are for bots to navigate, if there are no".into(),
                    "waypoints on the map then the bots won't move around".into(),
                    format!("Waypoints are files stored in the rcbot/waypoints/{mod_folder} folder"),
                    "\"waypoint on\" (displays waypoints)".into(),
                    "\"waypoint off\" (stops displaying waypoints)".into(),
                    "\"waypoint load\" (loads waypoints for current map)".into(),
                    "\"waypoint save\" (saves waypoints to disk for future use)".into(),
                    "\"waypoint flag <name>\" (toggle a flag on nearest waypoint)".into(),
                    "\"waypoint setteam <1-4|clear>\" (assign/clear team on nearest waypoint)".into(),
                ]
                .into();
            }
            BotCam => &[
                "bot cam command help",
                "--------------------",
                "Provided that \"rcbot config allow_botcam\" is 1",
                "you can view the bot cam, simply type...",
                "\"rcbot botcam\" to view the bot cam, or",
                "\"rcbot botcam off\" to stop viewing the bot cam",
            ],
            Squad => &[
                "squad command help",
                "------------------",
                "Allows you control squad as a leader",
                "Available sub commands are:",
                "\"form <formation name>\" (change formation)",
                "\"spread <spread size>\" (change space between squad members)",
                "NOTE: I recommend using the \"rcbot squad_menu\" command instead",
            ],
            Debug => &[
                "debug command help",
                "------------------",
                "Usage: rcbot debug <option> <on/off>",
                "on = 1, off = 0",
                "Available debug options are:",
                "\"touch\"",
                "\"think\"",
                "\"hear\"",
                "\"message\"",
                "\"block\"",
                "\"move\"",
                "\"aim\"",
                "\"nav\"",
                "\"see\"",
            ],
            Config => &[
                "config command help",
                "-----------------------",
                "Boolean commands",
                "these commands take booleans as parameters only",
                "booleans are 0 or 1",
                ".......................",
                "bots_leave_and_join (incomplete)",
                "tooltips",
                "autowaypoint_humans",
                "allow_botcam",
                "balance_teams",
                "chat_dont_learn",
                "chat_reply_to_bots",
                "wait_for_orders",
                "reserve_bot_slots",
                "dont_shoot",
                "chatting",
                "real_mode",
                "commanding",
                "bots_wait_for_bots (bots wait at wpts)",
                "ts_kungfu (bots use kung fu only in TS",
                "disable_weapon_learning (bots choose random weapons)",
                "disable_bot_squads (bots dont make squads)",
                "-----------------------",
                "Integer commands",
                "these commands take integers as parameters only",
                "integers are -1, 0, 1, 2, 3, 4... etc.",
                ".......................",
                "min_bots",
                "max_bots",
                "chat_percent",
                "chat_reply_percent",
                "max_path_revs",
                "max_update_vision_revs",
                "-----------------------",
                "Float commands",
                "these commands take real numbers as parameters only",
                "float numbers are 0.0, 0.5, 0.6, 1.0, 2.1... etc.",
                ".......................",
                "bot_stuck_speed",
                "update_ladder_time",
                "update_vision_time",
            ],
            PathWaypoint => &[
                "pathwaypoint command help",
                "-------------------------",
                "\"pathwaypoint on\" shows paths between waypoints",
                "\"pathwaypoint off\" stops showing paths",
                "\"pathwaypoint create1\" marks the waypoint you are standing at to create a path from",
                "\"pathwaypoint create2\" creates a path from the marked waypoint from, to this waypoint",
                "\"pathwaypoint remove1\" marks the waypoint you are standing at to remove a path from",
                "\"pathwaypoint remove2\" removes a path from the marked waypoint from, to this waypoint",
                "\"pathwaypoint enable\" allows automatic path creation (default = enabled)",
                "\"pathwaypoint disable\" disallows automatic path creation",
            ],
            WaypointMenu | BotMenu | MenuSelect | SquadMenu => &["There is no help for this command"],
        };
        lines.iter().map(|s| s.to_string()).collect()
    }
}

/// `CBotCvarList::PrintCommands`: `"\"<name>\""` per command, the last registered first.
pub fn command_list() -> Vec<String> {
    CommandId::ALL.iter().rev().map(|c| format!("\"{}\"", c.name())).collect()
}

/// How a message reaches players.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Print {
    /// `BotMessage(pEntity, 0, …)`: the player's console, or the server's for `None`.
    Message(Option<PlayerId>),
    /// `BotPrintTalkMessage`: everyone's chat area.
    TalkAll,
    /// `BotPrintTalkMessageOne`: one player's chat area.
    TalkOne(PlayerId),
    /// `ClientPrint(pEntity, HUD_PRINTTALK, …)`.
    HudTalk(PlayerId),
}

/// `ESquadFormation` values the commands and menus set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Formation {
    /// `SQUAD_FORM_WEDGE`.
    Wedge,
    /// `SQUAD_FORM_LINE`.
    Line,
    /// `SQUAD_FORM_COLUMN`.
    Column,
    /// `SQUAD_FORM_ECH_LEFT`.
    EchelonLeft,
    /// `SQUAD_FORM_ECH_RIGHT`.
    EchelonRight,
    /// `SQUAD_FORM_VEE`.
    Vee,
}

/// `eCombatType` values the squad menu sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatType {
    /// `COMBAT_COMBAT`.
    Combat,
    /// `COMBAT_CROUCH`.
    Crouch,
    /// `COMBAT_STEALTH`.
    Stealth,
}

/// A side effect of a command, named after the upstream call that does it.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `util killbot`: `pBot->m_bKill = true`.
    KillBot(PlayerId),
    /// `util bot_goto`: `ClearTasks` and a priority `BOT_TASK_FIND_PATH` to the waypoint.
    BotGoto {
        /// The bot.
        bot: PlayerId,
        /// The waypoint (the given one, or the nearest within 100 of the caller; may be -1).
        waypoint: i32,
    },
    /// `util givetsweapon`: `UTIL_makeTSweapon`.
    GiveTsWeapon {
        /// Who gets it.
        player: PlayerId,
        /// The `TSWeapon` id (`atoi`, not checked).
        weapon: i32,
    },
    /// `util hudtext`: a HUD message to everyone (channel 3, fade in/out 0.75 s, effect time
    /// 0.75 s, hold 2 s, centred, both colours at alpha 200).
    HudText {
        /// The text, `^` turned into newlines and `_` into spaces.
        text: String,
        /// The colour.
        colour: [u8; 3],
    },
    /// `util explo`: a beam and an explosion where the player looks (4096 units), with a screen
    /// shake (`UTIL_BotScreenShake(end, 16, 100, 2, magnitude)`).
    Explosion {
        /// Who looks.
        player: PlayerId,
        /// `magnitude` (`atoi`; 400 by default).
        magnitude: f32,
        /// `dodamage`: false when a third argument > 0 is given.
        damage: bool,
    },
    /// `util showboredom`: `pBot->printBoredoms(caller)`.
    PrintBoredoms {
        /// The bot.
        bot: PlayerId,
        /// Who reads it.
        to: Option<PlayerId>,
    },
    /// `util showreps`: `m_Rep.printReps(pBot, caller)` (see [`super::rep::Reputations::print_reps`]).
    PrintReps {
        /// The bot.
        bot: PlayerId,
        /// Who reads it.
        to: Option<PlayerId>,
    },
    /// `+rcbot_force_grip` (`grip`: the client grips the player or monster it faces,
    /// `UTIL_FacingEnt`, or nothing) and `-rcbot_force_grip` (lets go: `FL_FROZEN` off,
    /// `DROP_TO_FLOOR`). While gripped, `CClient::Think` holds the entity in front of the player.
    ForceGrip {
        /// Who grips.
        player: PlayerId,
        /// `+` or `-`.
        grip: bool,
    },
    /// `util teleport_ent`: `SET_ORIGIN` of the (non-world) entity in front of the player.
    TeleportEntityInFront {
        /// Who looks.
        player: PlayerId,
        /// Where to.
        to: Vector,
    },
    /// `util teleport`, `waypoint teleport`: `UTIL_SetOrigin` / `SET_ORIGIN`.
    SetOrigin {
        /// Who moves.
        player: PlayerId,
        /// Where to.
        to: Vector,
    },
    /// `util send_sound`: `UTIL_PlaySoundToAll`.
    PlaySoundToAll(String),
    /// `util search`: list the entities within `range` of the player (`UTIL_FindEntityInSphere`).
    Search {
        /// Around whom, and who reads it.
        player: PlayerId,
        /// The radius (`atof`, 100 by default).
        range: f32,
    },
    /// `removebot`: `BotFunc_KickBotFromTeam(team)` (-1: any team).
    KickBotFromTeam(i32),
    /// Bot menu "Kick random bot": `BotMenu_Func_KickBot` (`m_bKick` on a random bot in game).
    KickRandomBot,
    /// `debug message 0`: `m_pDebugMessage = nullptr`.
    ClearDebugMessage,
    /// `config turnspeed`: every bot's `m_fTurnSpeed`.
    SetBotsTurnSpeed(f32),
    /// `config max_path_revs`: every used bot's profile `m_iPathRevs`.
    SetBotsPathRevs(i32),
    /// `config max_update_vision_revs`: every used bot's profile `m_iVisionRevs`.
    SetBotsVisionRevs(i32),
    /// `config update_vision_time`: every used bot's profile `m_fVisionTime`.
    SetBotsVisionTime(f32),
    /// `squad form`, the formation menus: `ChangeFormation` on the player's squad.
    SquadFormation {
        /// The squad's leader.
        leader: PlayerId,
        /// The formation.
        formation: Formation,
    },
    /// `squad spread`: `ChangeSpread(spread)`.
    SquadSetSpread {
        /// The squad's leader.
        leader: PlayerId,
        /// The new spread.
        spread: f32,
    },
    /// The spread menu: `ChangeSpread(GetSpread() + delta)` (±24).
    SquadChangeSpread {
        /// The squad's leader.
        leader: PlayerId,
        /// The change.
        delta: f32,
    },
    /// Squad modes key 1: `ToggleFireMode`.
    SquadToggleFireMode(PlayerId),
    /// Squad modes keys 2-3: crouch/stealth on, or back to `COMBAT_COMBAT` if already on.
    SquadToggleCombatType {
        /// The squad's leader.
        leader: PlayerId,
        /// `COMBAT_CROUCH` or `COMBAT_STEALTH`.
        combat_type: CombatType,
    },
    /// Squad modes key 4: `ReturnAllToFormation`.
    SquadReturnToFormation(PlayerId),
    /// "Leave Squad": `m_Squads.ChangeLeader`.
    SquadLeave(PlayerId),
    /// "Remove Squad": `m_Squads.RemoveSquad`.
    SquadRemove(PlayerId),
    /// "Make Squad": `BotFunc_MakeSquad`.
    MakeSquad(usize),
    /// "Remove All Bot Squads": bots on the player's team drop their squads; the host says
    /// `"[RCBOT]  <n> bot squads removed\n"` (`HudTalk`).
    RemoveAllBotSquads(PlayerId),
    /// `waypoint load`: `WaypointLoad(nullptr)`.
    WaypointLoad,
    /// `waypoint add`: `WaypointAdd(client)`.
    WaypointAdd(usize),
    /// `waypoint delete`: `WaypointDelete(client)`.
    WaypointDelete(usize),
    /// `waypoint info`: `WaypointPrintInfo(player)`.
    WaypointPrintInfo(PlayerId),
    /// `pathwaypoint create1/2`: `WaypointCreatePath(client, n)`.
    WaypointCreatePath(usize, i32),
    /// `pathwaypoint remove1/2`: `WaypointRemovePath(client, n)`.
    WaypointRemovePath(usize, i32),
    /// "Clear Waypoints"/"Yes": `WaypointInit`.
    WaypointClearAll,
    /// The path menu: `WaypointDeletePath` to (`WaypointDeletePath(wpt)`) and/or from
    /// (`WaypointDeletePath(wpt, i)` for every i) the waypoint, from first then to.
    RemovePaths {
        /// The waypoint.
        waypoint: i32,
        /// Paths into it.
        to: bool,
        /// Paths out of it.
        from: bool,
    },
    /// `CBotMenu::Render`: `UTIL_ShowMenu(player, slots, -1, false, text)`.
    ShowMenu {
        /// Who sees it.
        player: PlayerId,
        /// Key mask.
        slots: i32,
        /// Text.
        text: String,
    },
    /// `botcam off`: `CBotCam::TuneOff`.
    BotcamOff(PlayerId),
    /// `CClient::AddNewToolTip`.
    AddToolTip(usize, ToolTip),
    /// `CBotGlobals::SayToolTip`.
    SayToolTip(PlayerId, ToolTip),
}

/// What `BotFunc_AddBot` was asked for, after its argument handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddBotRequest {
    /// `iTeam`: `atoi(arg1)` with anything outside 1-5 made 5 (auto); `None`: the slot's
    /// previous profile's team.
    pub team: Option<i32>,
    /// `iClass`: `atoi(arg2)`, negative ones made `RANDOM_LONG(0, 9)`; `None`: the previous.
    pub class: Option<i32>,
}

/// The engine and the bot manager, as the commands need them.
pub trait CommandHost {
    /// `IS_DEDICATED_SERVER`.
    fn is_dedicated_server(&self) -> bool;
    /// Prints a message.
    fn print(&mut self, to: Print, text: &str);
    /// Does a side effect.
    fn apply(&mut self, effect: Effect);
    /// `UTIL_FindPlayerByTruncName`.
    fn find_player(&self, name: &str) -> Option<PlayerId>;
    /// `UTIL_GetPlayerByPlayerId` (`#<userid>`).
    fn player_by_user_id(&self, user_id: i32) -> Option<PlayerId>;
    /// `STRING(pev->netname)`.
    fn player_name(&self, player: PlayerId) -> String;
    /// `pev->origin` / `EntityOrigin`.
    fn player_origin(&self, player: PlayerId) -> Vector;
    /// The player is a fake client run by RCBot (`FL_FAKECLIENT` and `UTIL_GetBotPointer`).
    fn is_rcbot(&self, player: PlayerId) -> bool;
    /// `UTIL_GetBotIndex` of a player (-1: not an RCBot).
    fn bot_index(&self, player: PlayerId) -> i32;
    /// `UTIL_FacingEnt(player, any)`: the entity index the player looks at.
    fn facing_entity(&self, player: PlayerId, any: bool) -> Option<u32>;
    /// `UTIL_GetBotIndex` of an entity (-1: not an RCBot).
    fn bot_index_of_entity(&self, entity: u32) -> i32;
    /// `WaypointLocations.NearestWaypoint(origin, range, -1, false, visible)`.
    fn nearest_waypoint(&self, origin: Vector, range: f32, visible: bool) -> i32;
    /// `waypoints[i].flags`.
    fn waypoint_flags(&self, waypoint: i32) -> u32;
    /// Sets `waypoints[i].flags`.
    fn set_waypoint_flags(&mut self, waypoint: i32, flags: u32);
    /// `WaypointOrigin`.
    fn waypoint_origin(&self, waypoint: i32) -> Vector;
    /// `WaypointSave(false, nullptr)`: false if the file couldn't be written.
    fn waypoint_save(&mut self) -> bool;
    /// `UTIL_GetNumClients(true)`.
    fn num_clients(&self) -> i32;
    /// `m_Squads.FindSquadByLeader(player) != nullptr`.
    fn leads_squad(&self, player: PlayerId) -> bool;
    /// `pev->flags ^= FL_NOTARGET`; the new state.
    fn toggle_notarget(&mut self, player: PlayerId) -> bool;
    /// `movetype` between `MOVETYPE_NOCLIP` and `MOVETYPE_WALK`; true if now noclip.
    fn toggle_noclip(&mut self, player: PlayerId) -> bool;
    /// `takedamage` between `DAMAGE_NO` and `DAMAGE_YES`; true if now `DAMAGE_NO`.
    fn toggle_godmode(&mut self, player: PlayerId) -> bool;
    /// `m_BotCam.TuneIn(player)`.
    fn botcam_tune_in(&mut self, player: PlayerId) -> bool;
    /// The rest of `BotFunc_AddBot` (the game must have started, a free slot, `max_bots`, the
    /// profile, creating the fake client, the TS team, the greeting); see
    /// [`super::profile::choose_profile`] and [`ts_team_choice`].
    fn add_bot(&mut self, caller: Option<usize>, request: AddBotRequest) -> CvarState;
    /// The contents of `bot_users.ini` (`None`: it can't be opened) and its path.
    fn read_users_file(&mut self) -> (Option<Vec<u8>>, String);
}

/// The state commands change: `gBotGlobals`' config, users and clients.
pub struct CommandContext<'a> {
    /// Settings.
    pub config: &'a mut Config,
    /// `m_BotUsers`.
    pub users: &'a mut AllowedPlayers,
    /// `m_Clients`.
    pub clients: &'a mut Clients,
    /// `RANDOM_LONG`.
    pub rng: &'a mut dyn Random,
}

fn has(s: &str) -> bool {
    !s.is_empty()
}

struct Run<'c, 'a> {
    ctx: &'c mut CommandContext<'a>,
    host: &'c mut dyn CommandHost,
    client: Option<usize>,
}

impl Run<'_, '_> {
    /// `pClient->GetPlayer()`.
    fn player(&self) -> Option<PlayerId> {
        self.client.and_then(|i| self.ctx.clients.get(i)).and_then(|c| c.player)
    }

    fn msg(&mut self, to: Option<PlayerId>, text: &str) {
        self.host.print(Print::Message(to), text);
    }

    fn help(&mut self, cmd: CommandId, to: Option<PlayerId>) {
        for l in cmd.help(&self.ctx.config.mod_folder) {
            self.msg(to, &l);
        }
    }

    fn client_mut(&mut self) -> Option<&mut super::Client> {
        self.client.and_then(|i| self.ctx.clients.get_mut(i))
    }

    fn render(&mut self, id: MenuId) {
        let Some(c) = self.client else { return };
        let Some(client) = self.ctx.clients.get_mut(c) else { return };
        let (slots, text) = menu::Menu::show(id, client);
        if let Some(player) = client.player {
            self.host.apply(Effect::ShowMenu { player, slots, text });
        }
    }

    fn action(&mut self, cmd: CommandId, a: [&str; 4]) -> CvarState {
        use CommandId::*;
        match cmd {
            AddBot => self.add_bot(a),
            Waypoint => self.waypoint(a),
            PathWaypoint => self.path_waypoint(a),
            Debug => self.debug(a),
            WaypointMenu => self.waypoint_menu(),
            BotMenu => {
                if self.client.is_none() {
                    self.msg(None, "This command can only be used on the client");
                    return CvarState::Error;
                }
                self.render(MenuId::BotMain);
                CvarState::Accessed
            }
            MenuSelect => self.menu_select(a),
            Config => self.config(a),
            SetPass => self.set_pass(a),
            Util => self.util(a),
            Squad => self.squad(a),
            SquadMenu => {
                if self.client.is_none() {
                    self.msg(None, "This command can only be used on the client");
                    return CvarState::Error;
                }
                self.render(MenuId::Squad);
                CvarState::Accessed
            }
            BotCam => self.botcam(a),
            AutoWaypoint => self.auto_waypoint(a),
            Help => self.help_command(a),
            Users => self.users(a),
            DebugBot => self.debug_bot(a),
            DebugEnt => self.debug_ent(a),
            RemoveBot => {
                let team = if has(a[0]) { atoi(a[0]) } else { -1 };
                self.host.apply(Effect::KickBotFromTeam(team));
                CvarState::Accessed
            }
        }
    }

    /// `BotFunc_AddBot`'s argument handling (the third argument, the skill, is unused upstream).
    fn add_bot(&mut self, a: [&str; 4]) -> CvarState {
        let team = has(a[0]).then(|| {
            let t = atoi(a[0]);
            if t <= 0 || t > 5 { 5 } else { t }
        });
        let class = has(a[1]).then(|| {
            let c = atoi(a[1]);
            if c < 0 { self.ctx.rng.random_long(0, 9) } else { c }
        });
        self.host.add_bot(self.client, AddBotRequest { team, class })
    }

    /// `CMenuSelectCommand::action`.
    fn menu_select(&mut self, a: [&str; 4]) -> CvarState {
        let Some(ci) = self.client else { return CvarState::Continue };
        if self.ctx.clients.get(ci).is_none_or(|c| c.menu.is_none()) {
            return CvarState::Continue;
        }
        if !has(a[0]) {
            return CvarState::Error;
        }
        let key = atoi(a[0]);
        if key < 10 {
            let Some(client) = self.ctx.clients.get_mut(ci) else { return CvarState::Continue };
            client.last_menu_item_selected = key;
            // A negative key indexes before the item array upstream; nothing happens here.
            match menu::activate(client, key) {
                Activated::Opened(id) => self.render(id),
                Activated::Run(action) => self.menu_action(action),
                Activated::Nothing | Activated::Closed => {}
            }
        }
        CvarState::Continue
    }

    /// The `BotMenu_Func_*` functions.
    fn menu_action(&mut self, action: MenuAction) {
        let Some(ci) = self.client else { return };
        let Some(client) = self.ctx.clients.get(ci) else { return };
        let (wpt, key, player) = (client.current_waypoint, client.last_menu_item_selected, client.player);
        let toggle = |host: &mut dyn CommandHost, bits: u32| {
            let f = host.waypoint_flags(wpt);
            host.set_waypoint_flags(wpt, if f & bits != 0 { f & !bits } else { f | bits });
        };
        let squad = |host: &mut dyn CommandHost| -> Option<PlayerId> {
            let p = player?;
            if host.leads_squad(p) {
                Some(p)
            } else {
                host.print(Print::HudTalk(p), "You are not leader of a squad\n");
                None
            }
        };
        match action {
            MenuAction::ToggleFlag(bits) => {
                if wpt != -1 {
                    toggle(self.host, bits);
                }
            }
            MenuAction::TeamWaypoint => {
                if wpt == -1 || key == -1 || key > 4 {
                    return;
                }
                let f = self.host.waypoint_flags(wpt);
                if f & wf::TEAM_SPECIFIC != 0 {
                    self.host.set_waypoint_flags(wpt, f & !wf::TEAM & !wf::TEAM_SPECIFIC);
                } else {
                    // Upstream ORs the team in without clearing the old bits (they were cleared
                    // when the team was turned off).
                    self.host.set_waypoint_flags(wpt, f | (key - 1) as u32 | wf::TEAM_SPECIFIC);
                }
            }
            MenuAction::UnreachableWaypoint => {
                if wpt != -1 {
                    toggle(self.host, wf::UNREACHABLE);
                    self.host.apply(Effect::RemovePaths { waypoint: wpt, to: true, from: true });
                }
            }
            MenuAction::RemoveAllPaths | MenuAction::RemovePathsTo | MenuAction::RemovePathsFrom => {
                if wpt != -1 {
                    let to = action != MenuAction::RemovePathsFrom;
                    let from = action != MenuAction::RemovePathsTo;
                    self.host.apply(Effect::RemovePaths { waypoint: wpt, to, from });
                }
            }
            MenuAction::DeleteWaypoints => self.host.apply(Effect::WaypointClearAll),
            MenuAction::AddBot => {
                self.host.add_bot(self.client, AddBotRequest { team: None, class: None });
            }
            MenuAction::KickBot => self.host.apply(Effect::KickRandomBot),
            MenuAction::MakeSquad => self.host.apply(Effect::MakeSquad(ci)),
            MenuAction::SquadLeave => {
                if let Some(p) = squad(self.host) {
                    self.host.apply(Effect::SquadLeave(p));
                    self.host.print(Print::HudTalk(p), "You have left your squad\n");
                }
            }
            MenuAction::SquadRemove => {
                if let Some(p) = squad(self.host) {
                    self.host.apply(Effect::SquadRemove(p));
                    self.host.print(Print::HudTalk(p), "Squad removed\n");
                }
            }
            MenuAction::SquadRemoveAllBotSquads => {
                if let Some(p) = player {
                    self.host.apply(Effect::RemoveAllBotSquads(p));
                }
            }
            MenuAction::SquadMode1 => {
                if let Some(p) = squad(self.host) {
                    match key {
                        1 => self.host.apply(Effect::SquadToggleFireMode(p)),
                        2 => self.host.apply(Effect::SquadToggleCombatType { leader: p, combat_type: CombatType::Crouch }),
                        3 => self.host.apply(Effect::SquadToggleCombatType { leader: p, combat_type: CombatType::Stealth }),
                        4 => self.host.apply(Effect::SquadReturnToFormation(p)),
                        _ => {}
                    }
                }
            }
            MenuAction::SquadForm | MenuAction::SquadForm2 => {
                if let Some(p) = squad(self.host) {
                    let first = action == MenuAction::SquadForm;
                    let pick = match (first, key) {
                        (true, 1) => Some((Formation::Wedge, "WEDGE")),
                        (true, 2) => Some((Formation::Line, "LINE")),
                        (true, 3) => Some((Formation::Column, "COLUMN")),
                        (false, 1) => Some((Formation::EchelonLeft, "ECHELON LEFT")),
                        (false, 2) => Some((Formation::EchelonRight, "ECHELON RIGHT")),
                        (false, 3) => Some((Formation::Vee, "VEE")),
                        _ => None,
                    };
                    if let Some((formation, name)) = pick {
                        self.host.apply(Effect::SquadFormation { leader: p, formation });
                        self.host.print(Print::HudTalk(p), &format!("Squad formation is now {name}\n"));
                    }
                }
            }
            MenuAction::SquadSpread => {
                if let Some(p) = squad(self.host) {
                    match key {
                        1 => self.host.apply(Effect::SquadChangeSpread { leader: p, delta: 24.0 }),
                        2 => self.host.apply(Effect::SquadChangeSpread { leader: p, delta: -24.0 }),
                        _ => {}
                    }
                }
            }
        }
    }

    /// `CSetPassCommand::action`.
    fn set_pass(&mut self, a: [&str; 4]) -> CvarState {
        if !has(a[0]) || self.client.is_none() {
            return CvarState::Error;
        }
        if let Some(p) = self.player() {
            if let Some(c) = self.client_mut() {
                c.set_pass(a[0]);
            }
            self.msg(Some(p), "Password Set");
        }
        CvarState::Accessed
    }

    /// `CWaypointMenuCommand::action`.
    fn waypoint_menu(&mut self) -> CvarState {
        let Some(p) = self.player() else {
            self.msg(None, "This command can only be used on the client");
            return CvarState::Error;
        };
        let wpt = self.host.nearest_waypoint(self.host.player_origin(p), 50.0, true);
        if let Some(c) = self.client_mut() {
            c.current_waypoint = wpt;
        }
        if wpt != -1 {
            self.render(MenuId::WaypointMain);
        } else {
            self.msg(Some(p), "Error: You are not standing close enough to a waypoint!");
        }
        CvarState::Accessed
    }

    /// Finds a player by name and says so when it can't.
    fn util(&mut self, a: [&str; 4]) -> CvarState {
        let me = self.player();
        if !has(a[0]) {
            self.msg(me, "no command entered");
            return CvarState::Error;
        }
        let name_of = |host: &dyn CommandHost, p: PlayerId| host.player_name(p);
        match a[0] {
            "showboredom" | "showreps" => {
                if has(a[1])
                    && let Some(bot) = self.host.find_player(a[1]).filter(|&p| self.host.is_rcbot(p))
                {
                    self.host.apply(if a[0] == "showreps" {
                        Effect::PrintReps { bot, to: me }
                    } else {
                        Effect::PrintBoredoms { bot, to: me }
                    });
                }
            }
            "bot_goto" => {
                if let (Some(me), true) = (me, has(a[1]))
                    && let Some(bot) = self.host.find_player(a[1]).filter(|&p| self.host.bot_index(p) != -1)
                {
                    let waypoint =
                        if has(a[2]) { atoi(a[2]) } else { self.host.nearest_waypoint(self.host.player_origin(me), 100.0, false) };
                    self.host.apply(Effect::BotGoto { bot, waypoint });
                }
            }
            "version" => self.msg(me, &format!("Bot Version : {BOT_VER}")),
            "givetsweapon" => {
                if let (Some(player), true) = (me, has(a[1])) {
                    self.host.apply(Effect::GiveTsWeapon { player, weapon: atoi(a[1]) });
                }
            }
            "numclients" => {
                let n = self.host.num_clients();
                self.msg(me, &format!("Loop count = {n}"));
            }
            "hudtext" => {
                if has(a[1]) {
                    let mut text: String = a[1].chars().take(255).collect();
                    text = text.replace('^', "\n").replace('_', " ");
                    let colour = match a[2] {
                        "blue" => [0, 0, 255],
                        "green" => [0, 255, 0],
                        "red" => [255, 0, 0],
                        "yellow" => [255, 255, 0],
                        "purple" => [255, 0, 255],
                        "cyan" => [0, 255, 255],
                        _ => [255, 255, 255],
                    };
                    self.host.apply(Effect::HudText { text, colour });
                }
            }
            "explo" => {
                let Some(player) = me else {
                    self.msg(None, "can't use on dedicated server");
                    return CvarState::Error;
                };
                let magnitude = if has(a[1]) { atoi(a[1]) as f32 } else { 400.0 };
                let damage = !(has(a[2]) && atoi(a[2]) > 0);
                self.host.apply(Effect::Explosion { player, magnitude, damage });
            }
            "killbot" => {
                if has(a[1]) {
                    match self.host.find_player(a[1]) {
                        Some(bot) => {
                            if self.host.is_rcbot(bot) {
                                self.host.apply(Effect::KillBot(bot));
                                let text = match me {
                                    Some(m) => format!("{} is killing bot \"{}\"...\n", name_of(self.host, m), name_of(self.host, bot)),
                                    None => format!("Server is killing bot \"{}\"...\n", name_of(self.host, bot)),
                                };
                                self.host.print(Print::TalkAll, &text);
                            }
                        }
                        None => self.msg(me, "bot name not found"),
                    }
                } else {
                    self.msg(me, "usage rcbot util killbot <botname>");
                }
            }
            "set_teleport" => {
                if let Some(m) = me {
                    let o = self.host.player_origin(m);
                    if let Some(c) = self.client_mut() {
                        c.teleport = Some(o);
                    }
                }
            }
            "projectile_rep" | "grenade_deact" => {
                let cmd =
                    if a[0] == "projectile_rep" { super::client::CLIENT_CMD_PROJ_REPEL } else { super::client::CLIENT_CMD_GREN_DEACT };
                if let Some(c) = self.client_mut() {
                    c.toggle_cmd(cmd);
                    let on = i32::from(c.has_cmd(cmd));
                    self.msg(me, &format!("{} : {on}", a[0]));
                }
            }
            "teleport_ent" => {
                if let Some(c) = self.client.and_then(|i| self.ctx.clients.get(i)) {
                    match (c.teleport, me) {
                        (Some(to), Some(player)) => self.host.apply(Effect::TeleportEntityInFront { player, to }),
                        (Some(_), None) => {}
                        (None, _) => self.msg(None, "no teleport vector set yet"),
                    }
                }
            }
            "teleport" if self.client.is_some() => {
                let tele = self.client.and_then(|i| self.ctx.clients.get(i)).and_then(|c| c.teleport);
                let Some(to) = tele else {
                    self.msg(me, "Error: No teleport destination set, use \"set_teleport\"");
                    return CvarState::Accessed;
                };
                if has(a[1]) {
                    if let Some(p) = self.host.find_player(a[1]) {
                        self.host.apply(Effect::SetOrigin { player: p, to });
                        if let Some(m) = me {
                            let text = format!("{} teleported {} to another location.\n", name_of(self.host, m), name_of(self.host, p));
                            self.host.print(Print::TalkAll, &text);
                        }
                    }
                } else if let Some(m) = me {
                    self.host.apply(Effect::SetOrigin { player: m, to });
                    let text = format!("{} teleported self to another location\n", name_of(self.host, m));
                    self.host.print(Print::TalkAll, &text);
                }
            }
            "notouch" if self.client.is_some() && me.is_some() => {
                let me = me.unwrap_or(PlayerId(0));
                let on = self.client_mut().map(|c| {
                    c.no_touch = !c.no_touch;
                    c.no_touch
                });
                let text = format!("{} used notouch {} on self\n", name_of(self.host, me), i32::from(on.unwrap_or(false)));
                self.host.print(Print::TalkAll, &text);
            }
            "notarget_mode" | "noclip_mode" | "godmode" => {
                type Toggle = fn(&mut dyn CommandHost, PlayerId) -> bool;
                let (verb, toggle): (&str, Toggle) = match a[0] {
                    "notarget_mode" => ("notarget_mode", |h, p| h.toggle_notarget(p)),
                    "noclip_mode" => ("no_clip", |h, p| h.toggle_noclip(p)),
                    _ => ("godmode", |h, p| h.toggle_godmode(p)),
                };
                if has(a[1]) {
                    if let Some(p) = self.host.find_player(a[1]) {
                        let on = toggle(self.host, p);
                        // Upstream names the caller with STRING(pEntity->v.netname), a null
                        // dereference from the server console (§5); the port says "Server".
                        let by = me.map_or_else(|| "Server".to_string(), |m| name_of(self.host, m));
                        let text = format!("{by} used {verb} {} on {}\n", i32::from(on), name_of(self.host, p));
                        self.host.print(Print::TalkAll, &text);
                    }
                } else if let Some(m) = me {
                    let on = toggle(self.host, m);
                    let text = format!("{} used {verb} {} on self\n", name_of(self.host, m), i32::from(on));
                    self.host.print(Print::TalkAll, &text);
                }
            }
            "send_sound" => {
                if has(a[1]) {
                    self.host.apply(Effect::PlaySoundToAll(a[1].to_string()));
                }
            }
            "search" if me.is_some() => {
                let range = if has(a[1]) { super::cstr::atof(a[1]) as f32 } else { 100.0 };
                if let Some(player) = me {
                    self.host.apply(Effect::Search { player, range });
                }
            }
            _ => {}
        }
        CvarState::Accessed
    }

    /// `CUsersCommand::action`.
    fn users(&mut self, a: [&str; 4]) -> CvarState {
        let me = self.player();
        match a[0] {
            "readusers" => {
                self.ctx.users.clear();
                self.msg(me, "re-reading bot users...");
                let (file, path) = self.host.read_users_file();
                let path = if path.is_empty() { self.ctx.config.file_name(BOT_USERS_FILE, "") } else { path };
                for m in self.ctx.users.read_users(file.as_deref(), &path) {
                    self.msg(None, &m);
                }
                for c in self.ctx.clients.iter_mut() {
                    if c.is_used() {
                        c.access_level = 0;
                        c.recheck_auth = true;
                    }
                }
            }
            "adduser" => {
                if has(a[1]) && has(a[2]) {
                    let target = self.host.find_player(a[1]);
                    if target == me {
                        let text =
                            if self.host.is_dedicated_server() { "Player not found!!" } else { "Can't give yourself different access!!" };
                        self.msg(me, text);
                        return CvarState::Error;
                    }
                    let Some(p) = target else {
                        self.msg(me, "Player not found!!");
                        return CvarState::Error;
                    };
                    let Some(ci) = self.ctx.clients.index_of(p) else {
                        self.msg(me, "Player not found!!");
                        return CvarState::Error;
                    };
                    let level = atoi(a[2]);
                    let name = self.host.player_name(p);
                    let steam = self.ctx.clients.get(ci).map(|c| c.steam_id.clone()).unwrap_or_default();
                    self.ctx.users.add_player(&name, "***", level, &steam);
                    if let Some(c) = self.ctx.clients.get_mut(ci) {
                        c.recheck_auth = true;
                    }
                    self.msg(me, &format!("Gave {name} access level of {level}"));
                } else {
                    self.msg(me, "Usage: rcbot readusers add <player name> <access level>");
                }
            }
            "removeuser" => {
                if has(a[1]) {
                    let target = match a[1].strip_prefix('#') {
                        Some(id) => self.host.player_by_user_id(atoi(id)),
                        None => self.host.find_player(a[1]),
                    };
                    let Some(p) = target else {
                        self.msg(me, "Player not found!!");
                        return CvarState::Error;
                    };
                    if Some(p) == me {
                        self.msg(me, "Cannot remove yourself!!!");
                        return CvarState::Error;
                    }
                    let Some(c) = self.ctx.clients.index_of(p).and_then(|i| self.ctx.clients.get(i)) else {
                        self.msg(me, "Player not found!!");
                        return CvarState::Error;
                    };
                    let (pass, steam) = (c.pass().to_string(), c.steam_id.clone());
                    let name = self.host.player_name(p);
                    self.ctx.users.remove_player(&name, &pass, &steam);
                    self.msg(me, &format!("Removed {name}'s access"));
                } else {
                    self.msg(me, "Usage: rcbot readusers add <player name>");
                }
            }
            "showusers" => {
                for l in self.ctx.users.show_users() {
                    self.msg(me, &l);
                }
            }
            _ => {
                self.msg(me, "Error: No command issued or command does not exist!");
                self.help(CommandId::Users, me);
                return CvarState::Error;
            }
        }
        CvarState::Accessed
    }

    /// `CDebugEntCommand::action`. Upstream dereferences a missing client for `off` (§5).
    fn debug_ent(&mut self, a: [&str; 4]) -> CvarState {
        if a[0] == "off" {
            if let Some(c) = self.client_mut() {
                c.debug_ent = None;
            }
        } else if let Some(p) = self.player() {
            match self.host.facing_entity(p, true) {
                Some(e) => {
                    if let Some(c) = self.client_mut() {
                        c.debug_ent = Some(e);
                    }
                }
                None => self.msg(Some(p), "No entity found"),
            }
        }
        CvarState::Accessed
    }

    /// `CDebugBotCommand::action`. Upstream dereferences a missing client (§5); nothing happens
    /// here.
    fn debug_bot(&mut self, a: [&str; 4]) -> CvarState {
        let Some(ci) = self.client else { return CvarState::Accessed };
        let me = self.player();
        match a[0] {
            "bot" => {
                if !has(a[1]) {
                    self.help(CommandId::DebugBot, me);
                    return CvarState::Error;
                }
                let idx = self.host.find_player(a[1]).map_or(-1, |p| self.host.bot_index(p));
                if let Some(c) = self.ctx.clients.get_mut(ci) {
                    c.debug_bot = idx;
                }
                if idx == -1 {
                    self.msg(me, "Bot not found (not found/ not an RCBot/ or is a human player)");
                    return CvarState::Error;
                }
            }
            "facing" => {
                if let Some(p) = me {
                    match self.host.facing_entity(p, false) {
                        Some(e) => {
                            let idx = self.host.bot_index_of_entity(e);
                            if let Some(c) = self.ctx.clients.get_mut(ci) {
                                c.debug_bot = idx;
                            }
                            if idx == -1 {
                                self.msg(me, "Facing entity is not a bot");
                                return CvarState::Error;
                            }
                        }
                        None => self.msg(me, "No entity found"),
                    }
                }
            }
            "off" => {
                if let Some(c) = self.ctx.clients.get_mut(ci) {
                    c.debug_bot = -1;
                }
            }
            _ => {}
        }
        CvarState::Accessed
    }

    /// `CDebugCommand::action`: only from the server (the dedicated console or the listen
    /// server's own player).
    fn debug(&mut self, a: [&str; 4]) -> CvarState {
        let error = if self.host.is_dedicated_server() {
            self.client.is_some()
        } else {
            // Upstream calls pClient->GetPlayer() on a missing client here (bot_config.ini on a
            // listen server, §5); a missing client counts as the server.
            self.client.is_some() && self.player() != self.ctx.clients.listen_server
        };
        if error {
            self.msg(None, "You can only use this command on the SERVER");
            return CvarState::Error;
        }
        if !has(a[0]) || !has(a[1]) {
            self.msg(None, "Usage: rcbot debug <option> <1 : on, 0 : off>");
            return CvarState::Error;
        }
        use super::debug::*;
        let level = match a[0] {
            "touch" => TOUCH,
            "think" => THINK,
            "hear" => HEAR,
            "message" => MESSAGE,
            "block" => BLOCK,
            "move" => MOVE,
            "aim" => AIM,
            "nav" => NAV,
            "see" => SEE,
            _ => 0,
        };
        if level == 0 {
            self.msg(None, "Debug level not found");
        } else if atoi(a[1]) > 0 {
            self.msg(None, &format!("debugging \"{}\" is now ON", a[0]));
            self.ctx.config.debug_levels |= level;
        } else {
            self.msg(None, &format!("debugging \"{}\" is now OFF", a[0]));
            self.ctx.config.debug_levels &= !level;
            if level == MESSAGE {
                self.host.apply(Effect::ClearDebugMessage);
            }
        }
        CvarState::Accessed
    }

    /// `CConfigCommand::action`.
    fn config(&mut self, a: [&str; 4]) -> CvarState {
        let me = self.player();
        if !has(a[0]) {
            self.msg(me, "Error: no sub-command issued");
            self.help(CommandId::Config, me);
            return CvarState::Error;
        }
        let setting = has(a[1]);
        let state = atoi(a[1]);
        let mut value: f32 = 0.0;
        let mut success = true;
        let c = &mut *self.ctx.config;
        let flag = match a[0] {
            "ts_dont_steal_weapons" => flags::TS_DONT_STEAL_WEAPONS,
            "ts_dont_pickup_weapons" => flags::TS_DONT_PICKUP_WEAPONS,
            "disable_bot_squads" => flags::DISABLE_BOT_SQUADS,
            "ts_kungfu" => flags::TS_KUNGFU,
            "disable_weapon_learning" => flags::DISABLE_WEAPON_LEARN,
            "tooltips" => flags::TOOLTIPS,
            "autowaypoint_humans" => flags::AUTOWAYPOINT_HUMANS,
            "allow_botcam" => flags::ENABLE_BOTCAM,
            "balance_teams" => flags::BALANCE_TEAMS,
            "chat_dont_learn" => flags::CHAT_DONT_LEARN,
            "chat_reply_to_bots" => flags::CHAT_REPLY_TO_BOTS,
            "wait_for_orders" => flags::WAIT_FOR_ORDERS,
            "reserve_bot_slots" => flags::RESERVE_BOT_SLOTS,
            "dont_shoot" => flags::DONT_SHOOT,
            "chatting" => flags::CHATTING,
            "real_mode" => flags::REAL_MODE,
            "commanding" => flags::COMMANDING,
            "unstick" => flags::UNSTICK,
            "bots_leave_and_join" => flags::BOTS_LEAVE_AND_JOIN,
            "bots_wait_for_bots" => flags::BOTS_WAIT_FOR_BOTS,
            _ => 0,
        };
        if flag == 0 {
            match a[0] {
                "turnspeed" => {
                    value = c.turn_speed;
                    if setting {
                        let speed = atof_f32(a[1]);
                        if speed > 0.0 {
                            c.turn_speed = speed;
                            value = speed;
                            self.host.apply(Effect::SetBotsTurnSpeed(speed));
                        } else {
                            self.msg(me, "Error, must be > 0");
                        }
                    }
                }
                "force_team" => {
                    if setting {
                        c.force_team = atoi(a[1]);
                    } else {
                        value = c.force_team as f32;
                    }
                }
                "bot_stuck_speed" => {
                    if setting {
                        c.bot_stuck_speed = atof_f32(a[1]);
                    } else {
                        value = c.bot_stuck_speed;
                    }
                }
                "update_ladder_time" => {
                    if setting {
                        c.update_ladder_time = atof_f32(a[1]);
                    } else {
                        value = c.update_ladder_time;
                    }
                }
                "min_bots" | "max_bots" | "chat_percent" | "chat_reply_percent" => {
                    let field = match a[0] {
                        "min_bots" => &mut c.min_bots,
                        "max_bots" => &mut c.max_bots,
                        "chat_percent" => &mut c.bot_chat_percent,
                        _ => &mut c.bot_chat_reply_percent,
                    };
                    if setting {
                        *field = atoi(a[1]) as i16;
                    } else {
                        value = f32::from(*field);
                    }
                }
                "max_path_revs" | "max_update_vision_revs" => {
                    let path = a[0] == "max_path_revs";
                    if setting {
                        let revs = atoi(a[1]);
                        success = revs > 0;
                        if success {
                            if path {
                                c.max_path_revs = revs;
                                self.host.apply(Effect::SetBotsPathRevs(revs));
                            } else {
                                c.max_vis_update_revs = revs;
                                self.host.apply(Effect::SetBotsVisionRevs(revs));
                            }
                        }
                    } else {
                        value = if path { c.max_path_revs } else { c.max_vis_update_revs } as f32;
                    }
                }
                "update_vision_time" => {
                    if setting {
                        let t = atof_f32(a[1]);
                        success = t >= 0.0;
                        if success {
                            c.update_vis_time = t;
                            self.host.apply(Effect::SetBotsVisionTime(t));
                        }
                    } else {
                        value = c.update_vis_time;
                    }
                }
                _ => {
                    self.msg(me, "Error: Command does not exist/command not issued!");
                    self.msg(me, "-------------------------------------------------");
                    self.help(CommandId::Config, me);
                    return CvarState::Accessed;
                }
            }
        }
        let c = &mut *self.ctx.config;
        let text = if flag != 0 {
            if setting {
                c.change_setting(flag, state);
                format!("{} is now {state}", a[0])
            } else {
                format!("\"{}\" is currently \"{}\"", a[0], i32::from(c.is_on(flag)))
            }
        } else if !setting {
            format!("\"{}\" is currently \"{}\"", a[0], f2(value))
        } else if success {
            format!("{} set to {}", a[0], a[1])
        } else {
            "Error with command, wrong argument".to_string()
        };
        self.msg(me, &text);
        CvarState::Accessed
    }

    /// `CBotSquadCommand::action`.
    fn squad(&mut self, a: [&str; 4]) -> CvarState {
        if self.client.is_none() || !has(a[0]) || !has(a[1]) {
            return CvarState::Error;
        }
        let Some(p) = self.player() else { return CvarState::Error };
        if !self.host.leads_squad(p) {
            self.host.print(Print::HudTalk(p), "You are not leader of a squad\n");
            return CvarState::Error;
        }
        if a[0] == "form" {
            // Upstream's "vee" sets the wedge (§5); kept.
            let pick = match a[1] {
                "line" => Some((Formation::Line, "LINE")),
                "vee" => Some((Formation::Wedge, "VEE")),
                "wedge" => Some((Formation::Wedge, "WEDGE")),
                "column" => Some((Formation::Column, "COLUMN")),
                "echleft" => Some((Formation::EchelonLeft, "ECHELON LEFT")),
                "echright" => Some((Formation::EchelonRight, "ECHELON RIGHT")),
                _ => None,
            };
            if let Some((formation, name)) = pick {
                self.host.apply(Effect::SquadFormation { leader: p, formation });
                self.host.print(Print::HudTalk(p), &format!("Squad formation is now {name}\n"));
            }
        } else if a[0] == "spread" {
            let spread = atof_f32(a[1]);
            if spread != 0.0 {
                self.host.apply(Effect::SquadSetSpread { leader: p, spread });
                self.host.print(Print::HudTalk(p), "Squad spread changed\n");
            }
        }
        CvarState::Accessed
    }

    /// `CPathWaypointCommand::action`.
    fn path_waypoint(&mut self, a: [&str; 4]) -> CvarState {
        let Some(ci) = self.client else {
            self.msg(None, "You can only use this command on the client");
            return CvarState::Error;
        };
        let me = self.player();
        let paths_on = |run: &mut Self| {
            if let Some(c) = run.ctx.clients.get_mut(ci) {
                c.waypoint_paths_on = true;
            }
        };
        match a[0] {
            "on" => {
                if let Some(c) = self.ctx.clients.get_mut(ci) {
                    c.waypoint_paths_on = true;
                }
                self.msg(me, "Waypoint paths are now being shown");
            }
            "off" => {
                if let Some(c) = self.ctx.clients.get_mut(ci) {
                    c.waypoint_paths_on = false;
                }
                self.msg(me, "Waypoints paths are not being shown");
            }
            s if s.starts_with("create") || s.starts_with("remove") => {
                paths_on(self);
                let create = s.starts_with("create");
                match s.as_bytes().get(6) {
                    Some(b'1') => {
                        self.host.apply(if create { Effect::WaypointCreatePath(ci, 1) } else { Effect::WaypointRemovePath(ci, 1) })
                    }
                    Some(b'2') => {
                        self.host.apply(if create { Effect::WaypointCreatePath(ci, 2) } else { Effect::WaypointRemovePath(ci, 2) })
                    }
                    _ => {}
                }
            }
            "enable" => {
                self.ctx.config.auto_path_waypoint = true;
                self.msg(me, "automatic path generation is now enabled");
            }
            "disable" => {
                self.ctx.config.auto_path_waypoint = false;
                self.msg(me, "automatic path generation is now disabled");
            }
            _ => {
                self.msg(me, "Error: You must issue a command!");
                self.msg(me, "--------------------------------");
                self.help(CommandId::PathWaypoint, me);
            }
        }
        CvarState::Accessed
    }

    /// `CAutoWaypointCommand::action`.
    fn auto_waypoint(&mut self, a: [&str; 4]) -> CvarState {
        let me = self.player();
        if me.is_none() && !has(a[0]) {
            self.msg(None, "You must issue a player name");
            return CvarState::Error;
        }
        let target = if has(a[0]) { self.host.find_player(a[0]) } else { me };
        let Some(p) = target else {
            self.msg(me, "Player not found");
            return CvarState::Error;
        };
        let Some(ci) = self.ctx.clients.index_of(p) else { return CvarState::Accessed };
        let mut state = 0;
        let on = {
            let Some(c) = self.ctx.clients.get_mut(ci) else { return CvarState::Accessed };
            if has(a[1]) {
                state = atoi(a[1]);
                c.set_auto_waypoint(state);
            } else {
                c.set_auto_waypoint(-1);
            }
            i32::from(c.auto_waypoint)
        };
        let name = self.host.player_name(p);
        match me {
            Some(m) if state == 2 => {
                let text = "autowaypoint set to 2 (debug) on you, you will see beams which are check points";
                self.host.print(Print::TalkOne(m), text);
                if m != p {
                    self.host.print(Print::TalkOne(p), text);
                }
            }
            Some(m) => self.host.print(Print::TalkOne(m), &format!("autowaypoint set to {on} on {name}")),
            None => self.msg(None, &format!("autowaypoint set to {on} on {name}")),
        }
        CvarState::Accessed
    }

    /// `CBotCamCommand::action`.
    fn botcam(&mut self, a: [&str; 4]) -> CvarState {
        let Some(p) = self.player() else {
            self.msg(None, "Can't use botcam on dedicated server");
            return CvarState::Error;
        };
        if a[0] == "off" {
            self.host.apply(Effect::BotcamOff(p));
            self.msg(Some(p), "Finished using botcam");
        } else if self.host.botcam_tune_in(p) {
            self.msg(Some(p), "Using botcam");
            if let Some(ci) = self.client {
                self.host.apply(Effect::AddToolTip(ci, ToolTip::BotcamOn));
            }
        } else {
            self.msg(Some(p), "Camera never worked or is not enabled (see command \"rcbot config allow_botcam\")");
        }
        CvarState::Accessed
    }

    /// `CHelpCommand::action`.
    fn help_command(&mut self, a: [&str; 4]) -> CvarState {
        let me = self.player();
        if !has(a[0]) {
            self.msg(me, "No command specified for help");
            self.msg(me, "Usage: rcbot help <command name>");
            return CvarState::Error;
        }
        match CommandId::lookup(a[0]) {
            Some(cmd) => self.help(cmd, me),
            None => {
                self.msg(me, "command specified does not exist!\nAvailable commands are");
                for l in command_list() {
                    self.msg(me, &l);
                }
            }
        }
        CvarState::Accessed
    }

    /// `CWaypointCommand::action`.
    fn waypoint(&mut self, a: [&str; 4]) -> CvarState {
        let Some(ci) = self.client else {
            self.msg(None, "This command can only be used on the client");
            return CvarState::Error;
        };
        let me = self.player();
        let set_on = |run: &mut Self| {
            if let Some(c) = run.ctx.clients.get_mut(ci) {
                c.waypoint_on = true;
            }
        };
        match a[0] {
            "on" | "off" => {
                if let Some(c) = self.ctx.clients.get_mut(ci) {
                    c.waypoint_on = a[0] == "on";
                }
                self.msg(me, if a[0] == "on" { "Waypoints are now being shown" } else { "Waypoints are not being shown" });
            }
            "load" => {
                self.host.apply(Effect::WaypointLoad);
                self.msg(me, "Waypoints have been loaded");
            }
            "save" => {
                // The "wb" (WhichBot, NS) and "gb" (GraveBot) converters aren't ported (§2).
                if self.host.waypoint_save() {
                    self.msg(me, "Waypoints have been saved");
                } else {
                    self.msg(me, "ERROR: CANNOT save waypoints! (File cannot be opened for writing : missing directory/bad permissions)");
                }
            }
            "add" => {
                set_on(self);
                self.host.apply(Effect::WaypointAdd(ci));
            }
            "delete" => {
                set_on(self);
                self.host.apply(Effect::WaypointDelete(ci));
            }
            "info" => {
                if let Some(p) = me {
                    self.host.apply(Effect::WaypointPrintInfo(p));
                }
            }
            "teleport" => {
                if has(a[1])
                    && let Some(p) = me
                {
                    let to = self.host.waypoint_origin(atoi(a[1]));
                    self.host.apply(Effect::SetOrigin { player: p, to });
                }
            }
            "flag" | "setteam" => return self.waypoint_flag(a, me),
            _ => {
                self.msg(me, "Error: No command issued or command does not exist!");
                self.help(CommandId::Waypoint, me);
                return CvarState::Error;
            }
        }
        CvarState::Accessed
    }

    /// `waypoint flag` and `waypoint setteam`.
    fn waypoint_flag(&mut self, a: [&str; 4], me: Option<PlayerId>) -> CvarState {
        let origin = me.map(|p| self.host.player_origin(p)).unwrap_or_default();
        let wpt = self.host.nearest_waypoint(origin, 50.0, true);
        if wpt == -1 {
            self.msg(me, "Error: You are not standing close enough to a waypoint!");
            return CvarState::Error;
        }
        let f = self.host.waypoint_flags(wpt);
        if a[0] == "setteam" {
            if !has(a[1]) {
                self.msg(me, "Usage: rcbot waypoint setteam <1-4|clear>");
                self.msg(me, "  1-4 = assign to team (SI: 1=MCL, 2=AFD)");
                self.msg(me, "  clear = remove team restriction");
                return CvarState::Error;
            }
            if a[1] == "clear" {
                self.host.set_waypoint_flags(wpt, f & !wf::TEAM & !wf::TEAM_SPECIFIC);
                self.msg(me, &format!("Cleared team restriction on waypoint {wpt}"));
            } else {
                let team = atoi(a[1]);
                if !(1..=4).contains(&team) {
                    self.msg(me, "Error: Team must be 1-4 or 'clear'");
                    return CvarState::Error;
                }
                self.host.set_waypoint_flags(wpt, (f & !wf::TEAM) | (team - 1) as u32 | wf::TEAM_SPECIFIC);
                self.msg(me, &format!("Set waypoint {wpt} to team {team}"));
            }
            return CvarState::Accessed;
        }
        if !has(a[1]) {
            for l in [
                "Usage: rcbot waypoint flag <flagname>",
                "Flags: jump, crouch, crouchjump, lift, wallstick, fly,",
                "       teleport, tank, waitlift, endlevel, stayclose,",
                "       openslater, humantower, unreachable, ladder,",
                "       important, scientist, barney (or gman),",
                "       pushable, checklift, defend",
            ] {
                self.msg(me, l);
            }
            return CvarState::Error;
        }
        let bits = match a[1] {
            "jump" => wf::JUMP,
            "crouch" => wf::CROUCH,
            "crouchjump" => wf::CROUCHJUMP,
            "health" => wf::HEALTH,
            "armor" | "armour" => wf::ARMOR,
            "lift" => wf::LIFT,
            // W_FL_WALL_STICK: TS's stunt bit.
            "wallstick" => wf::STUNT,
            "fly" => wf::FLY,
            "teleport" => wf::TELEPORT,
            "tank" => wf::TANK,
            "waitlift" => wf::WAIT_FOR_LIFT,
            "endlevel" => wf::ENDLEVEL,
            "stayclose" => wf::STAY_NEAR,
            "openslater" => wf::OPENS_LATER,
            "humantower" => wf::HUMAN_TOWER,
            "unreachable" => wf::UNREACHABLE,
            "ladder" => wf::LADDER,
            // W_FL_SCIENTIST_POINT and W_FL_BARNEY_POINT share bits 11 and 12.
            "important" | "scientist" => wf::IMPORTANT,
            "barney" | "gman" => wf::RESCUE,
            "pushable" => wf::PUSHABLE,
            "checklift" => wf::CHECK_LIFT,
            "defend" => wf::DEFEND_ZONE,
            other => {
                self.msg(me, &format!("Error: Unknown flag '{other}'"));
                return CvarState::Error;
            }
        };
        if f & bits != 0 {
            self.host.set_waypoint_flags(wpt, f & !bits);
            self.msg(me, &format!("Removed flag '{}' from waypoint {wpt}", a[1]));
        } else {
            self.host.set_waypoint_flags(wpt, f | bits);
            self.msg(me, &format!("Added flag '{}' to waypoint {wpt}", a[1]));
        }
        CvarState::Accessed
    }
}

/// Runs a command's `action` for a client (`None`: the server console or `bot_config.ini`),
/// without access checks.
pub fn run(ctx: &mut CommandContext<'_>, host: &mut dyn CommandHost, client: Option<usize>, cmd: CommandId, args: [&str; 4]) -> CvarState {
    Run { ctx, host, client }.action(cmd, args)
}

/// `ClientCommand`'s handling of `menuselect` and `rcbot <command>` (`dll.cpp`) for client
/// `client` saying `pcmd args…`. `None`: not a bot command, the game handles it (after the
/// `addbot` tip). The listen server's own player goes through [`server_command`] instead, as
/// upstream sends its `rcbot` to the server command.
pub fn client_command(
    ctx: &mut CommandContext<'_>,
    host: &mut dyn CommandHost,
    client: Option<usize>,
    pcmd: &str,
    args: [&str; 5],
) -> Option<CvarState> {
    let player = client.and_then(|i| ctx.clients.get(i)).and_then(|c| c.player);
    let mut state = None;
    if pcmd == "menuselect" {
        if client.is_none() {
            host.print(Print::Message(player), "Your client was not found in list of clients");
            return Some(CvarState::Accessed);
        }
        state = Some(run(ctx, host, client, CommandId::MenuSelect, [args[0], args[1], args[2], args[3]]));
    } else if pcmd == "+rcbot_force_grip" || pcmd == "-rcbot_force_grip" {
        // "little force grip thing, a bit messy!!!": the access *level* is compared with
        // `RCBOT_ACCESS_FORCE_GRIP` as a number, though it is a set of bits. Kept.
        let level = client.and_then(|i| ctx.clients.get(i)).map_or(0, |c| c.access_level);
        state = Some(if level < RCBOT_ACCESS_FORCE_GRIP {
            CvarState::NeedAccess
        } else {
            if let Some(p) = player {
                host.apply(Effect::ForceGrip { player: p, grip: pcmd.starts_with('+') });
            }
            CvarState::Accessed
        });
    } else if pcmd == super::BOT_COMMAND_ACCESS && (host.is_dedicated_server() || player != ctx.clients.listen_server) {
        let level = client.and_then(|i| ctx.clients.get(i)).map_or(0, |c| c.access_level);
        state = Some(match CommandId::lookup(args[0]) {
            Some(cmd) => {
                if client.is_none() {
                    host.print(Print::Message(player), "Your client was not found in list of clients");
                    return Some(CvarState::Accessed);
                }
                if cmd.needs_access(level) {
                    CvarState::NeedAccess
                } else {
                    run(ctx, host, client, cmd, [args[1], args[2], args[3], args[4]])
                }
            }
            None => CvarState::NotExist,
        });
    }
    match state {
        Some(CvarState::NeedAccess) => {
            host.print(Print::Message(player), "You do not have access to this command...");
            if let (Some(i), Some(p)) = (client, player)
                && ctx.clients.get(i).is_some_and(|c| !c.has_tooltip_sent(ToolTip::AdminHelp))
            {
                host.apply(Effect::SayToolTip(p, ToolTip::AdminHelp));
            }
        }
        Some(CvarState::Error) => host.print(Print::Message(player), "Error accessing command..."),
        Some(CvarState::NotExist) => host.print(Print::Message(player), "No command entered or bot command does not exist!"),
        Some(CvarState::Accessed) => {}
        Some(CvarState::Continue) | None => {
            if !pcmd.is_empty() && pcmd.contains("addbot") {
                host.print(Print::Message(player), "Tip: If you want to add an RCBOT use the command \"rcbot addbot\"\n");
            }
            return None;
        }
    }
    state
}

/// `RCBot_ServerCommand`: `rcbot <pcmd> args…` from the server console, with every access. On a
/// listen server the command runs for the listen server's client.
pub fn server_command(ctx: &mut CommandContext<'_>, host: &mut dyn CommandHost, pcmd: &str, args: [&str; 4]) -> CvarState {
    let listen = ctx.clients.listen_server;
    let mut state = CvarState::NotExist;
    if let Some(cmd) = CommandId::lookup(pcmd) {
        if host.is_dedicated_server() {
            if cmd.error(true) {
                host.print(Print::Message(None), "Error: Can't use this command on the dedicated server\n");
                return CvarState::Error;
            }
            host.print(Print::Message(None), "Accessing debug commands : detected dedicated server");
            state = run(ctx, host, None, cmd, args);
        } else if let Some(ci) = listen.and_then(|p| ctx.clients.index_of(p)) {
            state = run(ctx, host, Some(ci), cmd, args);
        } else {
            host.print(Print::Message(None), "Access to debug commands failed : Detected listen server with no listenserver client");
        }
    }
    match state {
        CvarState::Accessed => {}
        CvarState::Error => host.print(Print::Message(listen), &format!("The bot command {pcmd} returned an error!\n")),
        _ => {
            if pcmd.is_empty() {
                host.print(Print::Message(listen), "No command issued!\n");
            } else {
                host.print(Print::Message(listen), &format!("The bot command {pcmd} does not exist!\n"));
            }
            host.print(Print::Message(listen), "Available SUB-commands are:\n");
            for l in command_list() {
                host.print(Print::Message(listen), &l);
            }
        }
    }
    state
}

/// `CBotGlobals::ReadConfig`'s dispatch: each line of `bot_config.ini` (see
/// [`super::file::read_config`]) runs as a command with no client and no access checks.
pub fn run_config(ctx: &mut CommandContext<'_>, host: &mut dyn CommandHost, lines: &[super::file::ConfigLine]) {
    for l in lines {
        match CommandId::lookup(&l.cmd) {
            Some(cmd) => {
                let a = [l.args[0].as_str(), l.args[1].as_str(), l.args[2].as_str(), l.args[3].as_str()];
                run(ctx, host, None, cmd, a);
            }
            None => host.print(Print::Message(None), &format!("Error with config, unknown command : {}", l.cmd)),
        }
    }
}

/// `newteam`'s size in `BotFunc_AddBot` (`char newteam[64]`): 63 bytes and the NUL.
const NEWTEAM: usize = 64;

/// The TS team part of `BotFunc_AddBot` (in teamplay): `mp_teamlist` split at `;` (as
/// upstream splits it: the last byte always ends the last team, even a `;`), then team
/// `force_team` (1-based) if there is one and it isn't empty, else a random team
/// (`RANDOM_LONG`). The team name becomes the bot's `model` key. Bytes throughout, as upstream's
/// `char` copies (a CP1252 byte stays that byte). `None` for an empty list (upstream picks from an
/// empty array).
///
/// Upstream builds each team in `newteam`, NUL-terminating after every byte, and `strdup`s it at
/// each `;` without clearing it, so an empty team (`;;`, or a `;` right after the last byte's
/// team) repeats the team before it, and a leading `;` gives an empty team. Kept. Guard: a team of
/// 64 bytes or more overruns `newteam` upstream; it is cut at 63 bytes here.
pub fn ts_team_choice(teamlist: &[u8], force_team: i32, rng: &mut dyn Random) -> Option<Vec<u8>> {
    let teamlist = &teamlist[..teamlist.iter().position(|&c| c == 0).unwrap_or(teamlist.len())];
    let mut teams: Vec<Vec<u8>> = Vec::new();
    // `newteam` as the C string it holds, and the write index `j`.
    let mut newteam: Vec<u8> = Vec::new();
    let mut j = 0usize;
    let put = |newteam: &mut Vec<u8>, j: &mut usize, c: u8| {
        if *j < NEWTEAM - 1 {
            // `newteam[j++] = c; newteam[j] = 0;`
            newteam.truncate(*j);
            newteam.push(c);
        }
        *j += 1;
    };
    for (i, &c) in teamlist.iter().enumerate() {
        let last = i == teamlist.len() - 1;
        if last || c == b';' {
            if last {
                put(&mut newteam, &mut j, c);
            }
            teams.push(newteam.clone());
            j = 0;
        } else {
            put(&mut newteam, &mut j, c);
        }
    }
    if force_team > 0 && force_team as usize <= teams.len() {
        let t = &teams[force_team as usize - 1];
        if !t.is_empty() {
            return Some(t.clone());
        }
    }
    if teams.is_empty() {
        return None;
    }
    let i = rng.random_long(0, teams.len() as i32 - 1).clamp(0, teams.len() as i32 - 1);
    Some(teams[i as usize].clone())
}

#[cfg(test)]
mod tests {
    use super::super::{AllowedPlayers, Client, Clients};
    use super::*;
    use crate::XorShiftRandom;

    #[derive(Default)]
    struct Fake {
        dedicated: bool,
        printed: Vec<(Print, String)>,
        effects: Vec<Effect>,
        players: Vec<(PlayerId, &'static str, bool)>,
        flags: std::collections::HashMap<i32, u32>,
        nearest: i32,
        squad_leader: Option<PlayerId>,
        added: Vec<AddBotRequest>,
        users_file: Option<Vec<u8>>,
    }

    impl CommandHost for Fake {
        fn is_dedicated_server(&self) -> bool {
            self.dedicated
        }
        fn print(&mut self, to: Print, text: &str) {
            self.printed.push((to, text.to_string()));
        }
        fn apply(&mut self, effect: Effect) {
            self.effects.push(effect);
        }
        fn find_player(&self, name: &str) -> Option<PlayerId> {
            self.players.iter().find(|p| p.1.starts_with(name)).map(|p| p.0)
        }
        fn player_by_user_id(&self, user_id: i32) -> Option<PlayerId> {
            self.players.iter().find(|p| p.0.0 as i32 == user_id).map(|p| p.0)
        }
        fn player_name(&self, player: PlayerId) -> String {
            self.players.iter().find(|p| p.0 == player).map_or("?", |p| p.1).to_string()
        }
        fn player_origin(&self, player: PlayerId) -> Vector {
            Vector::new(player.0 as f32, 0.0, 0.0)
        }
        fn is_rcbot(&self, player: PlayerId) -> bool {
            self.players.iter().any(|p| p.0 == player && p.2)
        }
        fn bot_index(&self, player: PlayerId) -> i32 {
            if self.is_rcbot(player) { player.0 as i32 } else { -1 }
        }
        fn facing_entity(&self, _: PlayerId, _: bool) -> Option<u32> {
            Some(7)
        }
        fn bot_index_of_entity(&self, _: u32) -> i32 {
            -1
        }
        fn nearest_waypoint(&self, _: Vector, _: f32, _: bool) -> i32 {
            self.nearest
        }
        fn waypoint_flags(&self, w: i32) -> u32 {
            self.flags.get(&w).copied().unwrap_or(0)
        }
        fn set_waypoint_flags(&mut self, w: i32, f: u32) {
            self.flags.insert(w, f);
        }
        fn waypoint_origin(&self, w: i32) -> Vector {
            Vector::new(w as f32, 1.0, 2.0)
        }
        fn waypoint_save(&mut self) -> bool {
            true
        }
        fn num_clients(&self) -> i32 {
            3
        }
        fn leads_squad(&self, p: PlayerId) -> bool {
            self.squad_leader == Some(p)
        }
        fn toggle_notarget(&mut self, _: PlayerId) -> bool {
            true
        }
        fn toggle_noclip(&mut self, _: PlayerId) -> bool {
            false
        }
        fn toggle_godmode(&mut self, _: PlayerId) -> bool {
            true
        }
        fn botcam_tune_in(&mut self, _: PlayerId) -> bool {
            true
        }
        fn add_bot(&mut self, _: Option<usize>, request: AddBotRequest) -> CvarState {
            self.added.push(request);
            CvarState::Accessed
        }
        fn read_users_file(&mut self) -> (Option<Vec<u8>>, String) {
            (self.users_file.clone(), "rcbot/bot_users.ini".into())
        }
    }

    struct World {
        config: Config,
        users: AllowedPlayers,
        clients: Clients,
        rng: XorShiftRandom,
        host: Fake,
    }

    impl World {
        fn new() -> World {
            let mut clients = Clients::default();
            clients.get_mut(0).unwrap().player = Some(PlayerId(1));
            clients.get_mut(1).unwrap().player = Some(PlayerId(2));
            let host = Fake {
                players: vec![(PlayerId(1), "Neo", false), (PlayerId(2), "Trinity", false), (PlayerId(3), "[RCB]Smith", true)],
                ..Fake::default()
            };
            World { config: Config::default(), users: AllowedPlayers::default(), clients, rng: XorShiftRandom::new(1), host }
        }

        fn with<R>(&mut self, f: impl FnOnce(&mut CommandContext<'_>, &mut Fake) -> R) -> R {
            let mut ctx =
                CommandContext { config: &mut self.config, users: &mut self.users, clients: &mut self.clients, rng: &mut self.rng };
            f(&mut ctx, &mut self.host)
        }

        fn client(&mut self, args: [&str; 5]) -> Option<CvarState> {
            self.with(|ctx, host| client_command(ctx, host, Some(0), "rcbot", args))
        }

        fn texts(&self) -> Vec<String> {
            self.host.printed.iter().map(|p| p.1.clone()).collect()
        }
    }

    #[test]
    fn table() {
        assert_eq!(CommandId::lookup("addbot"), Some(CommandId::AddBot));
        assert_eq!(CommandId::lookup("AddBot"), None);
        assert_eq!(command_list().first().map(String::as_str), Some("\"removebot\""));
        assert_eq!(command_list().last().map(String::as_str), Some("\"addbot\""));
        assert!(CommandId::Config.needs_access(access::USERS));
        assert!(!CommandId::Config.needs_access(access::CONFIG | access::USERS));
        assert!(!CommandId::Help.needs_access(0));
        assert!(CommandId::Waypoint.error(true) && !CommandId::Waypoint.error(false) && !CommandId::Util.error(true));
        assert!(CommandId::Waypoint.help("ts").iter().any(|l| l.contains("rcbot/waypoints/ts")));
        assert_eq!(CommandId::BotMenu.help("ts"), ["There is no help for this command"]);
    }

    #[test]
    fn access_and_messages() {
        let mut w = World::new();
        assert_eq!(w.client(["config", "chatting", "1", "", ""]), Some(CvarState::NeedAccess));
        assert_eq!(w.texts(), ["You do not have access to this command..."]);
        assert_eq!(w.host.effects, [Effect::SayToolTip(PlayerId(1), ToolTip::AdminHelp)]);
        assert!(!w.config.is_on(flags::CHATTING));

        w.clients.get_mut(0).unwrap().access_level = access::CONFIG;
        assert_eq!(w.client(["config", "chatting", "1", "", ""]), Some(CvarState::Accessed));
        assert!(w.config.is_on(flags::CHATTING));
        assert_eq!(w.texts().last().unwrap(), "chatting is now 1");
        w.client(["config", "chatting", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "\"chatting\" is currently \"1\"");
        w.client(["config", "max_bots", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "\"max_bots\" is currently \"-1.00\"");
        w.client(["config", "max_path_revs", "0", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "Error with command, wrong argument");
        w.client(["config", "max_path_revs", "50", "", ""]);
        assert_eq!((w.config.max_path_revs, w.host.effects.last()), (50, Some(&Effect::SetBotsPathRevs(50))));
        w.client(["config", "turnspeed", "-1", "", ""]);
        assert_eq!(w.texts()[w.texts().len() - 2], "Error, must be > 0");
        w.client(["config", "blinking", "1", "", ""]);
        assert!(w.texts().contains(&"Error: Command does not exist/command not issued!".to_string()));

        assert_eq!(w.client(["nothing", "", "", "", ""]), Some(CvarState::NotExist));
        assert_eq!(w.texts().last().unwrap(), "No command entered or bot command does not exist!");
        assert_eq!(w.with(|ctx, host| client_command(ctx, host, Some(0), "say", ["hi", "", "", "", ""])), None);
        assert_eq!(w.with(|ctx, host| client_command(ctx, host, Some(0), "addbots", ["", "", "", "", ""])), None);
        assert!(w.texts().last().unwrap().starts_with("Tip: "));
    }

    #[test]
    fn force_grip_needs_level_nine() {
        let mut w = World::new();
        let grip = |w: &mut World, c: &str| w.with(|ctx, host| client_command(ctx, host, Some(0), c, ["", "", "", "", ""]));
        assert_eq!(grip(&mut w, "+rcbot_force_grip"), Some(CvarState::NeedAccess));
        assert!(w.host.effects.iter().all(|e| !matches!(e, Effect::ForceGrip { .. })));
        // A number, not a bit: 9 or more (any level with a group above `CONFIG`) will do.
        w.clients.get_mut(0).unwrap().access_level = RCBOT_ACCESS_FORCE_GRIP;
        assert_eq!(grip(&mut w, "+rcbot_force_grip"), Some(CvarState::Accessed));
        assert_eq!(grip(&mut w, "-rcbot_force_grip"), Some(CvarState::Accessed));
        let grips: Vec<_> = w.host.effects.iter().filter(|e| matches!(e, Effect::ForceGrip { .. })).cloned().collect();
        assert_eq!(grips, [Effect::ForceGrip { player: PlayerId(1), grip: true }, Effect::ForceGrip { player: PlayerId(1), grip: false }]);
    }

    #[test]
    fn listen_server_player_goes_to_the_server_command() {
        let mut w = World::new();
        w.clients.listen_server = Some(PlayerId(1));
        assert_eq!(w.client(["help", "", "", "", ""]), None);
        assert_eq!(w.with(|ctx, host| server_command(ctx, host, "debug", ["nav", "1", "", ""])), CvarState::Accessed);
        assert!(w.config.is_debug_on(super::super::debug::NAV));
        assert_eq!(w.with(|ctx, host| server_command(ctx, host, "", ["", "", "", ""])), CvarState::NotExist);
        assert!(w.texts().contains(&"No command issued!\n".to_string()));
        w.host.dedicated = true;
        assert_eq!(w.with(|ctx, host| server_command(ctx, host, "waypoint", ["on", "", "", ""])), CvarState::Error);
    }

    #[test]
    fn config_file_runs_without_a_client() {
        let mut w = World::new();
        let lines = super::super::file::read_config(b"config chatting 1\nconfig max_bots 6\nbogus x\ndebug aim 1\nwaypoint on\n");
        w.with(|ctx, host| run_config(ctx, host, &lines));
        assert!(w.config.is_on(flags::CHATTING));
        assert_eq!(w.config.max_bots, 6);
        assert!(w.config.is_debug_on(super::super::debug::AIM));
        assert!(w.texts().contains(&"Error with config, unknown command : bogus".to_string()));
        assert!(w.texts().contains(&"This command can only be used on the client".to_string()));
    }

    #[test]
    fn add_bot_arguments() {
        let mut w = World::new();
        w.clients.get_mut(0).unwrap().access_level = access::ADD_REMOVE_BOTS;
        w.client(["addbot", "", "", "", ""]);
        w.client(["addbot", "9", "3", "", ""]);
        w.client(["addbot", "2", "-1", "", ""]);
        assert_eq!(w.host.added[0], AddBotRequest { team: None, class: None });
        assert_eq!(w.host.added[1], AddBotRequest { team: Some(5), class: Some(3) });
        assert_eq!(w.host.added[2].team, Some(2));
        assert!((0..=9).contains(&w.host.added[2].class.unwrap()));
        w.client(["removebot", "", "", "", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::KickBotFromTeam(-1)));
    }

    #[test]
    fn waypoint_menu_and_menuselect() {
        let mut w = World::new();
        w.clients.get_mut(0).unwrap().access_level = access::WAYPOINTING;
        w.host.nearest = -1;
        w.client(["waypoint_menu", "", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "Error: You are not standing close enough to a waypoint!");
        w.host.nearest = 12;
        w.client(["waypoint_menu", "", "", "", ""]);
        assert!(matches!(&w.host.effects[0], Effect::ShowMenu { slots: 0b11111, text, .. } if text.starts_with("Waypoint Menu")));
        let select = |w: &mut World, k: &str| w.with(|ctx, host| client_command(ctx, host, Some(0), "menuselect", [k, "", "", "", ""]));
        assert_eq!(select(&mut w, "2"), None, "menuselect goes on to the game");
        assert_eq!(select(&mut w, "4"), None);
        assert_eq!(select(&mut w, "2"), None);
        // Give Flags -> More -> "Stunt".
        assert_eq!(w.host.flags.get(&12), Some(&wf::STUNT));
        assert_eq!(w.clients.get(0).unwrap().menu, None);
        // Team menu: key 3 makes it team 2's (bits 0-1 = 2).
        w.client(["waypoint_menu", "", "", "", ""]);
        select(&mut w, "1");
        select(&mut w, "3");
        assert_eq!(w.host.flags[&12], wf::STUNT | 2 | wf::TEAM_SPECIFIC);
        w.client(["waypoint_menu", "", "", "", ""]);
        select(&mut w, "1");
        select(&mut w, "1");
        assert_eq!(w.host.flags[&12], wf::STUNT);
        // Unreachable also removes the waypoint's paths.
        w.clients.get_mut(0).unwrap().menu = Some(MenuId::WaypointGiveFlags5);
        select(&mut w, "2");
        assert_eq!(w.host.effects.last(), Some(&Effect::RemovePaths { waypoint: 12, to: true, from: true }));
        // No menu open: menuselect is the game's.
        assert_eq!(select(&mut w, "1"), None);
        assert_eq!(w.with(|ctx, host| run(ctx, host, Some(0), CommandId::MenuSelect, ["", "", "", ""])), CvarState::Continue);
    }

    #[test]
    fn waypoint_flags_by_name() {
        let mut w = World::new();
        w.clients.get_mut(0).unwrap().access_level = access::WAYPOINTING;
        w.host.nearest = 3;
        w.client(["waypoint", "flag", "wallstick", "", ""]);
        assert_eq!(w.host.flags[&3], wf::STUNT);
        assert_eq!(w.texts().last().unwrap(), "Added flag 'wallstick' to waypoint 3");
        w.client(["waypoint", "flag", "wallstick", "", ""]);
        assert_eq!(w.host.flags[&3], 0);
        assert_eq!(w.client(["waypoint", "flag", "nope", "", ""]), Some(CvarState::Error));
        w.client(["waypoint", "setteam", "3", "", ""]);
        assert_eq!(w.host.flags[&3], 2 | wf::TEAM_SPECIFIC);
        w.client(["waypoint", "setteam", "clear", "", ""]);
        assert_eq!(w.host.flags[&3], 0);
        w.client(["waypoint", "teleport", "5", "", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::SetOrigin { player: PlayerId(1), to: Vector::new(5.0, 1.0, 2.0) }));
        w.client(["waypoint", "on", "", "", ""]);
        assert!(w.clients.get(0).unwrap().waypoint_on);
        w.client(["pathwaypoint", "create1", "", "", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::WaypointCreatePath(0, 1)));
        w.client(["pathwaypoint", "disable", "", "", ""]);
        assert!(!w.config.auto_path_waypoint);
    }

    #[test]
    fn util_and_users() {
        let mut w = World::new();
        w.clients.get_mut(0).unwrap().access_level = access::UTIL | access::USERS;
        w.client(["util", "killbot", "[RCB]", "", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::KillBot(PlayerId(3))));
        assert_eq!(w.host.printed.last().unwrap(), &(Print::TalkAll, "Neo is killing bot \"[RCB]Smith\"...\n".to_string()));
        w.client(["util", "killbot", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "usage rcbot util killbot <botname>");
        w.client(["util", "version", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "Bot Version : 1.51b14");
        w.client(["util", "hudtext", "hi^there_you", "red", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::HudText { text: "hi\nthere you".into(), colour: [255, 0, 0] }));
        w.client(["util", "teleport", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "Error: No teleport destination set, use \"set_teleport\"");
        w.client(["util", "set_teleport", "", "", ""]);
        w.client(["util", "teleport", "Trin", "", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::SetOrigin { player: PlayerId(2), to: Vector::new(1.0, 0.0, 0.0) }));
        w.client(["util", "godmode", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "Neo used godmode 1 on self\n");
        w.client(["util", "projectile_rep", "", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "projectile_rep : 1");

        w.client(["users", "adduser", "Trinity", "4", ""]);
        assert_eq!(w.texts().last().unwrap(), "Gave Trinity access level of 4");
        assert_eq!(w.users.all().len(), 1);
        assert!(w.clients.get(1).unwrap().recheck_auth);
        assert_eq!(w.client(["users", "adduser", "Neo", "4", ""]), Some(CvarState::Error));
        assert_eq!(w.texts()[w.texts().len() - 2], "Can't give yourself different access!!");
        assert_eq!(w.texts().last().unwrap(), "Error accessing command...");
        w.client(["users", "removeuser", "#2", "", ""]);
        assert_eq!(w.texts().last().unwrap(), "Removed Trinity's access");
        w.host.users_file = Some(b"\"Morpheus\" pw 8\n".to_vec());
        w.client(["users", "readusers", "", "", ""]);
        assert_eq!(w.users.all().len(), 1);
        assert_eq!(w.clients.get(0).unwrap().access_level, 0);
        assert_eq!(w.client(["users", "", "", "", ""]), Some(CvarState::NeedAccess));
    }

    #[test]
    fn squads_botcam_autowaypoint_debug() {
        let mut w = World::new();
        assert_eq!(w.client(["squad", "form", "vee", "", ""]), Some(CvarState::Error));
        assert_eq!(w.host.printed[0], (Print::HudTalk(PlayerId(1)), "You are not leader of a squad\n".to_string()));
        w.host.squad_leader = Some(PlayerId(1));
        w.client(["squad", "form", "vee", "", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::SquadFormation { leader: PlayerId(1), formation: Formation::Wedge }));
        w.client(["squad", "spread", "0", "", ""]);
        assert!(!matches!(w.host.effects.last(), Some(Effect::SquadSetSpread { .. })));
        w.client(["squad_menu", "", "", "", ""]);
        let select = |w: &mut World, k: &str| w.with(|ctx, host| client_command(ctx, host, Some(0), "menuselect", [k, "", "", "", ""]));
        select(&mut w, "2");
        select(&mut w, "2");
        assert_eq!(w.host.effects.last(), Some(&Effect::SquadChangeSpread { leader: PlayerId(1), delta: -24.0 }));

        w.client(["botcam", "", "", "", ""]);
        assert_eq!(w.host.effects.last(), Some(&Effect::AddToolTip(0, ToolTip::BotcamOn)));
        w.clients.get_mut(0).unwrap().access_level = access::WAYPOINTING | access::DEBUG;
        w.client(["autowaypoint", "Trin", "2", "", ""]);
        assert!(w.clients.get(1).unwrap().auto_waypoint && w.clients.get(1).unwrap().debug_auto_waypoint);
        assert_eq!(w.host.printed.iter().filter(|p| matches!(p.0, Print::TalkOne(_))).count(), 2);
        w.client(["autowaypoint", "", "", "", ""]);
        assert!(w.clients.get(0).unwrap().auto_waypoint);
        // Not the server: debug is refused.
        assert_eq!(w.client(["debug", "nav", "1", "", ""]), Some(CvarState::Error));
        w.client(["debug_bot", "bot", "[RCB]", "", ""]);
        assert_eq!(w.clients.get(0).unwrap().debug_bot, 3);
        w.client(["debug_ent", "", "", "", ""]);
        assert_eq!(w.clients.get(0).unwrap().debug_ent, Some(7));
        let mut c = Client::default();
        c.set_pass("x");
        assert!(c.recheck_auth);
    }

    #[test]
    fn ts_teams() {
        let mut r = XorShiftRandom::new(1);
        assert_eq!(ts_team_choice(b"Lone Wolf;Shadow", 2, &mut r).as_deref(), Some(&b"Shadow"[..]));
        assert_eq!(ts_team_choice(b"Lone Wolf;Shadow;", 2, &mut r).as_deref(), Some(&b"Shadow;"[..]));
        assert_eq!(ts_team_choice(b"a;b", 3, &mut r).map(|t| t == b"a" || t == b"b"), Some(true));
        assert_eq!(ts_team_choice(b"", 0, &mut r), None);
        assert_eq!(ts_team_choice(b"a\0b;c", 1, &mut r).as_deref(), Some(&b"a"[..]), "the C string ends at a NUL");
    }

    /// The team names are the cvar's bytes: a CP1252 0xE9 stays 0xE9 (#3).
    #[test]
    fn ts_teams_are_bytes() {
        let mut r = XorShiftRandom::new(1);
        assert_eq!(ts_team_choice(b"\xe9quipe;Caf\xe9", 1, &mut r).as_deref(), Some(&b"\xe9quipe"[..]));
        assert_eq!(ts_team_choice(b"\xe9quipe;Caf\xe9", 2, &mut r).as_deref(), Some(&b"Caf\xe9"[..]));
    }

    /// `newteam` isn't cleared between teams: an empty team repeats the one before it, a leading
    /// `;` is an empty team, and a forced empty team falls back to the random pick.
    #[test]
    fn ts_teams_stale_newteam() {
        let mut r = XorShiftRandom::new(1);
        let all = |list: &[u8], r: &mut XorShiftRandom| -> Vec<Option<Vec<u8>>> { (1..=4).map(|f| ts_team_choice(list, f, r)).collect() };
        let t = |s: &[u8]| Some(s.to_vec());
        let got = all(b"abc;;de;x", &mut r);
        assert_eq!(got[..3], [t(b"abc"), t(b"abc"), t(b"de")]);
        assert_eq!(ts_team_choice(b"abc;;de;x", 4, &mut r), t(b"x"));
        // A shorter team after a longer one: NUL-terminated after each byte, nothing left over.
        assert_eq!(ts_team_choice(b"abcdef;xy;", 2, &mut r), t(b"xy;"));
        // "a;;": the second `;` is the last byte, so it is a team of its own.
        assert_eq!(ts_team_choice(b"a;;", 2, &mut r), t(b";"));
        assert_eq!(ts_team_choice(b";", 1, &mut r), t(b";"));
        // A leading `;`: an empty first team; forcing it picks at random (only "b" is non-empty
        // besides it, and the draw may give the empty one).
        let first = ts_team_choice(b";b", 1, &mut r).unwrap();
        assert!(first.is_empty() || first == b"b");
    }

    /// Teams are cut at 63 bytes (guard: upstream overruns `char newteam[64]`), on byte
    /// boundaries, so no length or UTF-8 sequence can panic (#3).
    #[test]
    fn ts_teams_cut_at_63_bytes() {
        let mut r = XorShiftRandom::new(1);
        for len in [62usize, 63, 64, 65, 200] {
            let team = vec![b'x'; len];
            let list = [&team[..], b";b"].concat();
            assert_eq!(ts_team_choice(&list, 1, &mut r), Some(team[..len.min(63)].to_vec()), "team of {len} bytes");
            // The last team ends at the last byte instead of a `;`.
            let list = [&b"b;"[..], &team[..]].concat();
            assert_eq!(ts_team_choice(&list, 2, &mut r), Some(team[..len.min(63)].to_vec()), "last team of {len} bytes");
        }
        // Multi-byte UTF-8 across byte 63 (62 ASCII bytes, then "é" = C3 A9): cut mid-sequence.
        let list = [&[b'x'; 62][..], "\u{e9}\u{e9}".as_bytes()].concat();
        let got = ts_team_choice(&list, 1, &mut r).unwrap();
        assert_eq!(got.len(), 63);
        assert_eq!(got[62], 0xc3);
        // A stale copy of a cut team is the cut team.
        let list = [&[b'y'; 70][..], b";;b"].concat();
        assert_eq!(ts_team_choice(&list, 2, &mut r), Some(vec![b'y'; 63]));
    }
}
