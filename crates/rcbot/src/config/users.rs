//! The users allowed to run bot commands: `CAllowedPlayer`, `CAllowedPlayers` (`bot.h`,
//! `dll.cpp`) and `bot_users.ini` (`ReadBotUsersConfig`, `dll.cpp`).
//!
//! `bot_users.ini`, as upstream reads it: `fgets(buffer, 255)` chunks; a chunk starting with `#`
//! is a comment; one trailing `'\n'` is removed; empty lines are skipped. Then: everything up to
//! the first `"` is skipped, the name runs to the next `"` (at most 64 bytes), one byte is
//! skipped, and after runs of spaces come three space-separated words: the password (at most 16
//! bytes), the access level (at most 8 bytes, read with `atoi`) and the Steam ID (at most 32
//! bytes). A line is accepted if it has a name, a password and an access level, or a Steam ID
//! and an access level:
//!
//! ```text
//! "Name" password 31 STEAM_0:1:2345
//! ```
//!
//! The access level is a bit set of the `BOT_COMMAND_ACCESS_*` groups
//! ([`super::commands::access`]); a command is allowed if the user has any of its bits.

use super::client::Client;
use super::cstr::{atoi, c_str, fgets_chunks, lossy, text_mode, truncate};

/// `BOT_MAX_PASSWORD_LEN`.
pub const BOT_MAX_PASSWORD_LEN: usize = 16;
/// `STEAM_ID_LEN`.
pub const STEAM_ID_LEN: usize = 32;

/// `CAllowedPlayer`: one entry of `bot_users.ini`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowedPlayer {
    /// `m_szName`.
    pub name: String,
    /// `m_szPass` (at most 15 bytes).
    pub pass: String,
    /// `m_iAccessLevel`.
    pub access_level: i32,
    /// `m_szSteamId` (at most 31 bytes); empty when not given.
    pub steam_id: String,
}

impl AllowedPlayer {
    /// `CAllowedPlayer(szName, szPass, iAccessLevel, szSteamId)`: the password and Steam ID are
    /// cut to their buffers (`strncpy` of 15 and 31 bytes).
    pub fn new(name: &str, pass: &str, access_level: i32, steam_id: &str) -> AllowedPlayer {
        AllowedPlayer {
            name: name.to_owned(),
            pass: truncate(pass, BOT_MAX_PASSWORD_LEN),
            access_level,
            steam_id: truncate(steam_id, STEAM_ID_LEN),
        }
    }

    /// `steamID_defined`.
    pub fn steam_id_defined(&self) -> bool {
        !self.steam_id.is_empty()
    }

    /// `IsForSteamID`: both IDs non-empty and equal.
    pub fn is_for_steam_id(&self, steam_id: &str) -> bool {
        !steam_id.is_empty() && !self.steam_id.is_empty() && self.steam_id == steam_id
    }

    /// `IsForName`.
    pub fn is_for_name(&self, name: &str) -> bool {
        self.name == name
    }

    /// `IsForPass`.
    pub fn is_for_pass(&self, pass: &str) -> bool {
        self.pass == pass
    }

    /// `operator ==` as `dataStack::Remove` calls it (`node == removed`): `removed` matches the
    /// entry's name and password, or its Steam ID.
    fn matches(&self, removed: &AllowedPlayer) -> bool {
        (removed.is_for_name(&self.name) && removed.is_for_pass(&self.pass)) || removed.is_for_steam_id(&self.steam_id)
    }

    /// `IsForClient`: by Steam ID when the entry has one, else by name and password. `name` is
    /// the client's current name (`None` without a player, which never matches).
    pub fn is_for_client(&self, client: &Client, name: Option<&str>) -> bool {
        if self.steam_id_defined() {
            return self.is_for_steam_id(&client.steam_id);
        }
        let same_name = !self.name.is_empty() && name == Some(self.name.as_str());
        if client.player.is_none() {
            return false;
        }
        same_name && self.is_for_pass(client.pass())
    }

    /// `ShowUser`'s line.
    pub fn show(&self) -> String {
        format!("name=\"{}\", pass=\"{}\", accesslevel={}, steamID=\"{}\"", self.name, self.pass, self.access_level, self.steam_id)
    }
}

/// `CAllowedPlayers`: a stack, newest on top (the end of the `Vec`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowedPlayers {
    players: Vec<AllowedPlayer>,
}

impl AllowedPlayers {
    /// `AddPlayer`: removes the first entry (from the top) matching the new one, then pushes it.
    pub fn add_player(&mut self, name: &str, pass: &str, access_level: i32, steam_id: &str) {
        let p = AllowedPlayer::new(name, pass, access_level, steam_id);
        self.remove(&p);
        self.players.push(p);
    }

    /// `RemovePlayer`: removes the first entry (from the top) with this name and password, or
    /// this Steam ID.
    pub fn remove_player(&mut self, name: &str, pass: &str, steam_id: &str) {
        self.remove(&AllowedPlayer::new(name, pass, 0, steam_id));
    }

    fn remove(&mut self, p: &AllowedPlayer) {
        if let Some(i) = self.players.iter().rposition(|x| x.matches(p)) {
            self.players.remove(i);
        }
    }

    /// `GetPlayer`: the newest entry for this client.
    pub fn get_player(&self, client: &Client, name: Option<&str>) -> Option<&AllowedPlayer> {
        self.players.iter().rev().find(|p| p.is_for_client(client, name))
    }

    /// `ShowUsers`: one line per entry, newest first.
    pub fn show_users(&self) -> Vec<String> {
        self.players.iter().rev().map(AllowedPlayer::show).collect()
    }

    /// `Destroy`.
    pub fn clear(&mut self) {
        self.players.clear();
    }

    /// The entries, oldest first.
    pub fn all(&self) -> &[AllowedPlayer] {
        &self.players
    }

    /// `ReadBotUsersConfig` over the file's bytes (`None`: it couldn't be opened, `path` is
    /// named in the message). Adds the users and returns upstream's console messages.
    ///
    /// Deviation: upstream writes each field's terminating NUL one past its buffer when the field
    /// fills it (a 64-byte name, 16-byte password, 8-byte level, 32-byte Steam ID); the port
    /// just keeps the bytes read.
    pub fn read_users(&mut self, file: Option<&[u8]>, path: &str) -> Vec<String> {
        let Some(data) = file else {
            return vec![format!(
                "!!! Cannot find bot_users.ini !!!! (trying to look in {path} - not found) permissions are bad or rcbot is not installed properly"
            )];
        };
        let mut msgs = Vec::new();
        let mut users = 0;
        let data = &text_mode(data)[..];
        for (n, chunk) in fgets_chunks(data, 255).enumerate() {
            let line = n + 1;
            let mut b = c_str(chunk);
            if b.first() == Some(&b'#') {
                continue;
            }
            if b.last() == Some(&b'\n') {
                b = &b[..b.len() - 1];
            }
            if b.is_empty() {
                continue;
            }
            let len = b.len();
            let mut i = 0;
            while i < len && b[i] != b'"' {
                i += 1;
            }
            i += 1;
            let field = |stop: u8, max: usize, i: &mut usize| {
                let start = (*i).min(len);
                let mut j = 0;
                while *i < len && b[*i] != stop && j < max {
                    *i += 1;
                    j += 1;
                }
                lossy(&b[start..start + j])
            };
            let name = field(b'"', 64, &mut i);
            i += 1;
            let skip = |i: &mut usize| {
                while *i < len && b[*i] == b' ' {
                    *i += 1;
                }
            };
            skip(&mut i);
            let pass = field(b' ', 16, &mut i);
            skip(&mut i);
            let level = field(b' ', 8, &mut i);
            skip(&mut i);
            let steam = field(b' ', STEAM_ID_LEN, &mut i);
            if (!name.is_empty() && !pass.is_empty() && !level.is_empty()) || (!steam.is_empty() && !level.is_empty()) {
                msgs.push(format!("Added: name=\"{name}\", pass={pass}, accesslev={level}, steamid={steam}"));
                self.add_player(&name, &pass, atoi(&level), &steam);
                users += 1;
            } else {
                msgs.push(format!("Error in bot_users.ini, error on line {line} (bad format?)"));
            }
        }
        msgs.push(format!("{users} users added to list of RCBot users"));
        msgs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PlayerId;

    fn client(pass: &str, steam: &str) -> Client {
        let mut c = Client::default();
        c.player = Some(PlayerId(1));
        c.steam_id = steam.into();
        c.set_pass(pass);
        c
    }

    #[test]
    fn read_users_file() {
        let text = "# users\n\
                    \"Bob Smith\"  secret   31\n\
                    \"Al\" pw 2 STEAM_0:1:5\n\
                    \"\" x 4 STEAM_0:0:9\n\
                    nonsense\n\
                    \"NoLevel\" pw\n\
                    \"Long\" 12345678901234567890 1\n\
                    \"Bob Smith\" secret 1\n";
        let mut u = AllowedPlayers::default();
        let msgs = u.read_users(Some(text.as_bytes()), "rcbot/bot_users.ini");
        assert_eq!(msgs[0], "Added: name=\"Bob Smith\", pass=secret, accesslev=31, steamid=");
        assert_eq!(msgs[1], "Added: name=\"Al\", pass=pw, accesslev=2, steamid=STEAM_0:1:5");
        assert_eq!(msgs[2], "Added: name=\"\", pass=x, accesslev=4, steamid=STEAM_0:0:9");
        assert_eq!(msgs[3], "Error in bot_users.ini, error on line 5 (bad format?)");
        assert_eq!(msgs[4], "Error in bot_users.ini, error on line 6 (bad format?)");
        // 16 bytes of password are read, the rest of the word becomes the access level, and the
        // next word the Steam ID.
        assert_eq!(msgs[5], "Added: name=\"Long\", pass=1234567890123456, accesslev=7890, steamid=1");
        assert_eq!(msgs.last().unwrap(), "5 users added to list of RCBot users");
        // The second "Bob Smith"/"secret" replaced the first.
        assert_eq!(u.all().iter().filter(|p| p.name == "Bob Smith").count(), 1);
        assert_eq!(u.all().last().unwrap().access_level, 1);
        // The stored password is cut to 15 bytes.
        assert_eq!(u.all().iter().find(|p| p.name == "Long").unwrap().pass, "123456789012345");
        assert_eq!(u.show_users()[0], "name=\"Bob Smith\", pass=\"secret\", accesslevel=1, steamID=\"\"");

        let missing = AllowedPlayers::default().read_users(None, "p");
        assert!(missing[0].starts_with("!!! Cannot find bot_users.ini !!!! (trying to look in p"));
    }

    #[test]
    fn matching_clients() {
        let mut u = AllowedPlayers::default();
        u.add_player("Bob", "pw", 8, "");
        u.add_player("Al", "x", 2, "STEAM_1");
        // Name and password, case-sensitive.
        assert_eq!(u.get_player(&client("pw", ""), Some("Bob")).unwrap().access_level, 8);
        assert!(u.get_player(&client("PW", ""), Some("Bob")).is_none());
        assert!(u.get_player(&client("pw", ""), Some("bob")).is_none());
        // A Steam ID entry ignores name and password.
        assert_eq!(u.get_player(&client("", "STEAM_1"), Some("Whoever")).unwrap().name, "Al");
        assert!(u.get_player(&client("x", ""), Some("Al")).is_none());
        // No player: no match by name.
        let mut c = client("pw", "");
        c.player = None;
        assert!(u.get_player(&c, Some("Bob")).is_none());
        // Removing by Steam ID or by name + password.
        u.remove_player("nobody", "", "STEAM_1");
        assert_eq!(u.all().len(), 1);
        u.remove_player("Bob", "wrong", "");
        assert_eq!(u.all().len(), 1);
        u.remove_player("Bob", "pw", "");
        assert!(u.all().is_empty());
    }
}
