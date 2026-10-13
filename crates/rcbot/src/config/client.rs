//! The part of `CClient`/`CClients` (`bot.h`, `client.cpp`) that commands, menus and access
//! checks use: the player's access level and password, the open menu, the waypoint display
//! switches, the debug targets, the teleport point and the auto-waypoint switches. The rest of
//! `CClient` (auto-waypoint recording, tooltips queue, debug HUD) comes with its users.

use super::menu::MenuId;
use super::users::{AllowedPlayers, BOT_MAX_PASSWORD_LEN};
use super::{MAX_PLAYERS, cstr::truncate};
use crate::Vector;

/// A player: its entity index (1 to `maxClients`), as `ENTINDEX(pEdict)` gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u32);

/// `CLIENT_CMD_PROJ_REPEL`.
pub const CLIENT_CMD_PROJ_REPEL: i32 = 1;
/// `CLIENT_CMD_GREN_DEACT`.
pub const CLIENT_CMD_GREN_DEACT: i32 = 2;

/// `eToolTip` (`bot_const.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ToolTip {
    ServerWelcome = 0,
    ClientWelcome,
    SquadHelp,
    AdminHelp,
    CpuHelp,
    NoWaypoints,
    CreateSquad,
    CommanderWelcome,
    CommanderMarineOrder,
    CommanderSquad,
    CommanderSquad2,
    CommanderHealth,
    BotcamHelp,
    BotcamOn,
}

/// The command-related state of one `CClient`.
#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    /// `m_pPlayer`: `None` for a free slot.
    pub player: Option<PlayerId>,
    /// `m_iAccessLevel`: `BOT_COMMAND_ACCESS_*` bits.
    pub access_level: i32,
    pass: String,
    /// `m_szSteamId` (`setSteamID` on connect).
    pub steam_id: String,
    /// `m_iPlayerId`: the reputation id (`GetPlayerRepId`), -1 for none.
    pub player_rep_id: i32,
    /// `m_bRecheckAuth`: look the player up in the users again on the next think.
    pub recheck_auth: bool,
    /// `m_pMenu`: the menu on screen.
    pub menu: Option<MenuId>,
    /// `m_iLastMenuItemSelected`: the key pressed, which shared menu functions switch on.
    pub last_menu_item_selected: i32,
    /// `m_iCurrentWaypoint`: the waypoint the waypoint menu edits.
    pub current_waypoint: i32,
    /// `m_bWaypointOn`.
    pub waypoint_on: bool,
    /// `m_bWaypointPathsOn`.
    pub waypoint_paths_on: bool,
    /// `m_iDebugBot`: the bot index being debugged, -1 for none.
    pub debug_bot: i32,
    /// `m_pDebugEnt`: the entity being debugged.
    pub debug_ent: Option<u32>,
    /// `m_vTeleportVector`.
    pub teleport: Option<Vector>,
    /// `m_bNoTouch`.
    pub no_touch: bool,
    /// `m_iCmd`: [`CLIENT_CMD_PROJ_REPEL`], [`CLIENT_CMD_GREN_DEACT`].
    pub cmd: i32,
    /// `m_bAutoWaypoint`.
    pub auto_waypoint: bool,
    /// `m_bDebugAutoWaypoint`.
    pub debug_auto_waypoint: bool,
    /// `m_bSetUpAutoWaypoint`.
    pub set_up_auto_waypoint: bool,
    /// `m_iToolTipsSent`: one bit per [`ToolTip`].
    pub tooltips_sent: u32,
}

impl Default for Client {
    /// `CClient::Init`: everything zero except the -1 indices.
    fn default() -> Client {
        Client {
            player: None,
            access_level: 0,
            pass: String::new(),
            steam_id: String::new(),
            player_rep_id: -1,
            recheck_auth: false,
            menu: None,
            last_menu_item_selected: -1,
            current_waypoint: -1,
            waypoint_on: false,
            waypoint_paths_on: false,
            debug_bot: -1,
            debug_ent: None,
            teleport: None,
            no_touch: false,
            cmd: 0,
            auto_waypoint: false,
            debug_auto_waypoint: false,
            set_up_auto_waypoint: false,
            tooltips_sent: 0,
        }
    }
}

impl Client {
    /// `IsUsed`.
    pub fn is_used(&self) -> bool {
        self.player.is_some()
    }

    /// `GetPass`.
    pub fn pass(&self) -> &str {
        &self.pass
    }

    /// `SetPass`: cut to 15 bytes; asks for an access recheck.
    pub fn set_pass(&mut self, pass: &str) {
        self.pass = truncate(pass, BOT_MAX_PASSWORD_LEN);
        self.recheck_auth = true;
    }

    /// `hasCmd`.
    pub fn has_cmd(&self, cmd: i32) -> bool {
        (self.cmd & cmd) == cmd
    }

    /// `toggleCmd`.
    pub fn toggle_cmd(&mut self, cmd: i32) {
        if self.has_cmd(cmd) {
            self.cmd &= !cmd;
        } else {
            self.cmd |= cmd;
        }
    }

    /// `AutoWaypoint(state)`: -1 toggles, 2 is on with debug beams, otherwise `state > 0`.
    pub fn set_auto_waypoint(&mut self, state: i32) {
        self.debug_auto_waypoint = false;
        self.set_up_auto_waypoint = false;
        if state == 2 {
            self.debug_auto_waypoint = true;
        }
        if state == -1 {
            self.auto_waypoint = !self.auto_waypoint;
        } else {
            self.auto_waypoint = state > 0;
        }
    }

    /// `HasToolTipSent`.
    pub fn has_tooltip_sent(&self, tip: ToolTip) -> bool {
        self.tooltips_sent & (1 << tip as u32) != 0
    }
}

/// `CClients`: one [`Client`] per player slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Clients {
    clients: Vec<Client>,
    /// `gBotGlobals.m_pListenServerEdict`: the listen server's host, whose console commands run
    /// as server commands.
    pub listen_server: Option<PlayerId>,
}

impl Default for Clients {
    fn default() -> Clients {
        Clients { clients: vec![Client::default(); MAX_PLAYERS], listen_server: None }
    }
}

impl Clients {
    /// `GetClientByIndex` (0-based).
    pub fn get(&self, index: usize) -> Option<&Client> {
        self.clients.get(index)
    }

    /// `GetClientByIndex`, mutably.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Client> {
        self.clients.get_mut(index)
    }

    /// `GetClientByEdict`: the used slot holding this player.
    pub fn index_of(&self, player: PlayerId) -> Option<usize> {
        self.clients.iter().position(|c| c.player == Some(player))
    }

    /// `GetClientByRepId`: the used slot with this reputation id.
    pub fn index_by_rep_id(&self, rep_id: i32) -> Option<usize> {
        self.clients.iter().position(|c| c.is_used() && c.player_rep_id == rep_id)
    }

    /// Every slot.
    pub fn iter(&self) -> impl Iterator<Item = &Client> {
        self.clients.iter()
    }

    /// Every slot, mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Client> {
        self.clients.iter_mut()
    }

    /// The access part of `CClients::ClientConnected`: the slot `ENTINDEX - 1` takes the player
    /// (with its Steam ID and reputation id), then gets the access of its `bot_users.ini` entry
    /// (and the message "You have been authenticated", returned), or on a listen server the
    /// first player without an entry becomes the host with access level 10. Returns the slot
    /// index and the message for the player, `None` for an index outside the slots.
    pub fn connect(
        &mut self,
        player: PlayerId,
        name: &str,
        steam_id: &str,
        player_rep_id: i32,
        users: &AllowedPlayers,
        dedicated: bool,
    ) -> Option<(usize, Option<&'static str>)> {
        let i = (player.0 as usize).checked_sub(1).filter(|&i| i < MAX_PLAYERS)?;
        let c = &mut self.clients[i];
        c.player = Some(player);
        c.steam_id = truncate(steam_id, super::users::STEAM_ID_LEN);
        c.player_rep_id = player_rep_id;
        if let Some(u) = users.get_player(c, Some(name)) {
            c.access_level = u.access_level;
            return Some((i, Some("You have been authenticated")));
        }
        if !dedicated && self.listen_server.is_none() {
            self.listen_server = Some(player);
            c.access_level = 10;
        }
        Some((i, None))
    }

    /// The `m_bRecheckAuth` part of `CClient::Think`: if a recheck is pending, look the player up
    /// again (`name` is its current name) and clear the flag. Returns "You have been
    /// authenticated" when it found an entry.
    pub fn recheck_auth(&mut self, index: usize, users: &AllowedPlayers, name: Option<&str>) -> Option<&'static str> {
        let c = self.clients.get_mut(index)?;
        if !c.recheck_auth {
            return None;
        }
        c.recheck_auth = false;
        let u = users.get_player(c, name)?;
        c.access_level = u.access_level;
        Some("You have been authenticated")
    }

    /// The access part of `CClients::ClientDisconnected`: the slot is reset (`CClient::Init`) and
    /// stops being the listen server's host.
    pub fn disconnect(&mut self, index: usize) {
        if let Some(c) = self.clients.get_mut(index) {
            if c.player.is_some() && c.player == self.listen_server {
                self.listen_server = None;
            }
            *c = Client::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_state() {
        let mut c = Client::default();
        assert_eq!((c.current_waypoint, c.player_rep_id, c.last_menu_item_selected, c.debug_bot), (-1, -1, -1, -1));
        c.set_pass("0123456789abcdefXYZ");
        assert_eq!(c.pass(), "0123456789abcde");
        assert!(c.recheck_auth);
        c.toggle_cmd(CLIENT_CMD_GREN_DEACT);
        assert!(c.has_cmd(CLIENT_CMD_GREN_DEACT) && !c.has_cmd(CLIENT_CMD_PROJ_REPEL));
        c.toggle_cmd(CLIENT_CMD_GREN_DEACT);
        assert_eq!(c.cmd, 0);
        c.set_auto_waypoint(-1);
        assert!(c.auto_waypoint);
        c.set_auto_waypoint(2);
        assert!(c.auto_waypoint && c.debug_auto_waypoint);
        c.set_auto_waypoint(0);
        assert!(!c.auto_waypoint && !c.debug_auto_waypoint);
        c.tooltips_sent = 1 << ToolTip::AdminHelp as u32;
        assert!(c.has_tooltip_sent(ToolTip::AdminHelp) && !c.has_tooltip_sent(ToolTip::BotcamOn));
    }

    #[test]
    fn connect_and_auth() {
        let mut users = AllowedPlayers::default();
        users.add_player("Admin", "pw", 31, "");
        let mut cl = Clients::default();
        // Listen server: the first unknown player becomes the host.
        assert_eq!(cl.connect(PlayerId(1), "Host", "", 0, &users, false), Some((0, None)));
        assert_eq!(cl.listen_server, Some(PlayerId(1)));
        assert_eq!(cl.get(0).unwrap().access_level, 10);
        assert_eq!(cl.connect(PlayerId(2), "Other", "", 1, &users, false), Some((1, None)));
        assert_eq!(cl.get(1).unwrap().access_level, 0);
        assert_eq!(cl.index_by_rep_id(1), Some(1));
        assert_eq!(cl.connect(PlayerId(40), "x", "", 1, &users, false), None);
        // Password then recheck.
        cl.get_mut(1).unwrap().set_pass("pw");
        assert_eq!(cl.recheck_auth(1, &users, Some("Admin")), Some("You have been authenticated"));
        assert_eq!(cl.get(1).unwrap().access_level, 31);
        assert_eq!(cl.recheck_auth(1, &users, Some("Admin")), None);
        cl.disconnect(0);
        assert_eq!(cl.listen_server, None);
        assert!(!cl.get(0).unwrap().is_used());
        assert_eq!(cl.index_of(PlayerId(2)), Some(1));
    }
}
