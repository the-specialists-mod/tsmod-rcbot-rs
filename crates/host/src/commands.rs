//! The `rcbot` commands (`RCBot_ServerCommand`, `ClientCommand`'s `rcbot`/`menuselect`,
//! `bot_config.ini`), `rcbot::config::CommandHost` on the engine, and loading RCBot's files from
//! the plugin's data directory (`<gamedir>/tsmod/rcbot/`, RCBot's bot folder).
//!
//! Effects of features not ported yet (squads, the bot cam, menus' HUD, chat, `util` toys) print
//! that they aren't available instead of doing nothing silently.

use crate::api::{Api, Engine};
use crate::world::{EngineRandom, HostWorld};
use crate::{State, editor, with_state};
use rcbot::Vector;
use rcbot::bot::BotWorld;
use rcbot::config::commands::{self as cmds, AddBotRequest, CommandContext, CommandHost, CvarState, Effect, Print};
use rcbot::config::{BOT_CONFIG_FILE, BOT_DBG_MSG_TAG, BOT_USERS_FILE, PlayerId, read_config as parse_config};
use rcbot::nav::editor as ed;
use rcbot::nav::{NearestOptions, Waypoints, World};
use std::ffi::{CStr, c_char, c_int};
use ts_plugin_api::log;

fn args_of(argc: c_int, argv: *const *const c_char) -> Vec<String> {
    (0..argc.max(0) as usize)
        .map(|i| {
            // SAFETY: the server passes `argc` NUL-terminated strings for the call.
            let p = unsafe { *argv.add(i) };
            if p.is_null() {
                String::new()
            } else {
                // SAFETY: as above.
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        })
        .collect()
}

/// The `rcbot` server command (`RCBot_ServerCommand`).
pub unsafe extern "C" fn server_command(argc: c_int, argv: *const *const c_char) {
    let args = args_of(argc, argv);
    let a = |i: usize| args.get(i).map_or("", String::as_str);
    with_state(None, |s, api, e| {
        let (pcmd, rest) = (a(1).to_owned(), [a(2), a(3), a(4), a(5)]);
        run(s, api, e, |ctx, host| {
            cmds::server_command(ctx, host, &pcmd, rest);
        });
    });
}

/// `ClientCommand`'s `rcbot <command>` and `menuselect`: non-zero when RCBot handled it.
pub unsafe extern "C" fn client_command(player: c_int, argc: c_int, argv: *const *const c_char) -> c_int {
    let args = args_of(argc, argv);
    let a = |i: usize| args.get(i).map_or("", String::as_str);
    if a(0) != rcbot::config::BOT_COMMAND_ACCESS && a(0) != "menuselect" {
        return 0;
    }
    with_state(None, |s, api, e| {
        let client = s.clients.index_of(PlayerId(player as u32));
        let pcmd = a(0).to_owned();
        let rest = [a(1), a(2), a(3), a(4), a(5)];
        let mut handled = false;
        run(s, api, e, |ctx, host| {
            handled = cmds::client_command(ctx, host, client, &pcmd, rest).is_some();
        });
        c_int::from(handled)
    })
    .unwrap_or(0)
}

/// Runs a command with the context (config, users, clients) taken out of the state and the
/// rest of the state as the host.
fn run(s: &mut State, api: Api, e: Engine, f: impl FnOnce(&mut CommandContext<'_>, &mut dyn CommandHost)) {
    let mut rng = EngineRandom(e);
    let mut config = std::mem::take(&mut s.config);
    let mut users = std::mem::take(&mut s.users);
    let mut clients = std::mem::take(&mut s.clients);
    {
        let mut host = Host { s, api, e };
        let mut ctx = CommandContext { config: &mut config, users: &mut users, clients: &mut clients, rng: &mut rng };
        f(&mut ctx, &mut host);
    }
    s.config = config;
    s.users = users;
    s.clients = clients;
}

/// `ReadBotUsersConfig`: `bot_users.ini`.
pub fn read_users(s: &mut State, a: Api, _e: Engine) {
    let path = s.data_dir.join(BOT_USERS_FILE);
    let file = std::fs::read(&path).ok();
    for m in s.users.read_users(file.as_deref(), &path.to_string_lossy()) {
        a.log(log::INFO, &m);
    }
}

/// `CBotGlobals::ReadConfig`: `bot_config.ini`, each line an `rcbot` command without a client.
pub fn read_config(s: &mut State, a: Api, e: Engine) {
    let path = s.data_dir.join(BOT_CONFIG_FILE);
    let Ok(data) = std::fs::read(&path) else {
        a.log(log::INFO, &format!("no {} (defaults)", path.display()));
        return;
    };
    let lines = parse_config(&data);
    run(s, a, e, |ctx, host| cmds::run_config(ctx, host, &lines));
}

/// `WaypointLoad` for this map from `<data>/waypoints/ts/<map>.rcw`, and its `.rcv`; rows the
/// `.rcv` doesn't hold are worked out (design §5.1).
pub fn load_waypoints(s: &mut State, a: Api, e: Engine) {
    let dir = s.data_dir.join("waypoints").join(&s.config.mod_folder);
    let rcw = dir.join(format!("{}.rcw", s.map));
    s.waypoints = Waypoints::new();
    let Ok(bytes) = std::fs::read(&rcw) else {
        a.log(log::WARNING, &format!("no waypoints for {} ({})", s.map, rcw.display()));
        return;
    };
    match Waypoints::from_rcw(&bytes, &s.map) {
        Ok((mut w, header)) => {
            let n = w.len();
            let world = HostWorld { api: a, eng: e, teamplay: false };
            let mut first_row = 0;
            if !header.needs_visibility()
                && let Ok(rcv) = std::fs::read(dir.join(format!("{}.rcv", s.map)))
                && w.visibility.read_rcv(&rcv, n).is_ok()
            {
                first_row = rcbot::nav::VisibilityTable::rows_in_file(n);
            }
            if first_row < n {
                let wpts = w.as_slice().to_vec();
                w.visibility.work_out_rows(&wpts, first_row, &world);
            }
            s.waypoints = w;
            a.log(log::INFO, &format!("loaded {n} waypoints for {} (visibility rows from {first_row})", s.map));
            s.check("waypoints_loaded");
        }
        Err(err) => a.log(log::WARNING, &format!("{}: {err}", rcw.display())),
    }
}

/// `CommandHost` on the engine and the state (everything but the config, users and clients,
/// which the command context holds).
struct Host<'a> {
    s: &'a mut State,
    api: Api,
    e: Engine,
}

impl Host<'_> {
    fn world(&self) -> HostWorld {
        HostWorld { api: self.api, eng: self.e, teamplay: false }
    }

    fn player_entity(&self, p: PlayerId) -> Option<rcbot::bot::Entity> {
        self.world().entity(p.0)
    }

    fn not_ported(&mut self, what: &str) {
        self.print(Print::Message(None), &format!("{what} is not available yet in this port of RCBot"));
    }

    /// The edit commands of a client slot: its player and position.
    fn editor_player(&self, client: usize) -> Option<rcbot::bot::Entity> {
        self.player_entity(PlayerId(client as u32 + 1))
    }
}

impl CommandHost for Host<'_> {
    fn is_dedicated_server(&self) -> bool {
        self.e.is_dedicated()
    }

    fn print(&mut self, to: Print, text: &str) {
        let line = if text.ends_with('\n') { text.to_owned() } else { format!("{text}\n") };
        match to {
            Print::Message(None) => self.e.server_print(&format!("{BOT_DBG_MSG_TAG}{line}")),
            Print::Message(Some(p)) | Print::TalkOne(p) | Print::HudTalk(p) => {
                self.e.client_print(p.0 as i32, &format!("{BOT_DBG_MSG_TAG}{line}"))
            }
            Print::TalkAll => {
                for i in 1..=self.e.max_clients() {
                    self.e.client_print(i, &line);
                }
            }
        }
    }

    fn apply(&mut self, effect: Effect) {
        let w = self.world();
        let mut out = Vec::new();
        let mut to = None;
        match effect {
            Effect::WaypointLoad => load_waypoints(self.s, self.api, self.e),
            Effect::WaypointAdd(c) => {
                if let Some(p) = self.editor_player(c) {
                    ed::add_origin(
                        &mut self.s.waypoints,
                        &w,
                        p.origin,
                        0,
                        Some(&p),
                        true,
                        true,
                        self.s.config.auto_path_waypoint,
                        &mut out,
                    );
                    to = Some(p.index);
                }
            }
            Effect::WaypointDelete(c) => {
                if let Some(p) = self.editor_player(c) {
                    ed::delete(&mut self.s.waypoints, &w, p.origin, &mut out);
                    to = Some(p.index);
                }
            }
            Effect::WaypointCreatePath(c, n) | Effect::WaypointRemovePath(c, n) => {
                if let Some(p) = self.editor_player(c) {
                    let edit = self.s.editor.paths.entry(p.index).or_default();
                    if matches!(effect, Effect::WaypointCreatePath(..)) {
                        ed::create_path(&mut self.s.waypoints, &w, edit, p.origin, n, &mut out);
                    } else {
                        ed::remove_path(&mut self.s.waypoints, &w, edit, p.origin, n, &mut out);
                    }
                    to = Some(p.index);
                }
            }
            Effect::WaypointClearAll => self.s.waypoints = Waypoints::new(),
            Effect::RemovePaths { waypoint, to: into, from } => {
                if let Ok(i) = usize::try_from(waypoint) {
                    if from {
                        let targets: Vec<i16> = self.s.waypoints.paths_from(i).collect();
                        for t in targets {
                            self.s.waypoints.delete_path(i, t);
                        }
                    }
                    if into {
                        self.s.waypoints.delete_paths_to(waypoint as i16);
                    }
                }
            }
            Effect::WaypointPrintInfo(p) => {
                let origin = self.player_origin(p);
                let i = self.nearest_waypoint(origin, 50.0, false);
                let text = if i == -1 {
                    "No waypoint nearby".to_owned()
                } else {
                    let f = self.s.waypoints.flags(i);
                    let names: Vec<&str> = rcbot::nav::flags::NAMES.iter().filter(|(b, _)| f & b != 0).map(|(_, n)| *n).collect();
                    format!(
                        "Waypoint {i} flags {f:#x} ({}) paths {:?}",
                        names.join(", "),
                        self.s.waypoints.paths_from(i as usize).collect::<Vec<_>>()
                    )
                };
                self.print(Print::Message(Some(p)), &text);
            }
            Effect::KillBot(p) => {
                if let Some(b) = self.s.bots.get_mut(p.0) {
                    b.kill = true;
                }
            }
            Effect::KickBotFromTeam(_) | Effect::KickRandomBot => {
                if let Some(b) = self.s.bots.iter_mut().last() {
                    b.kick = true;
                }
            }
            Effect::SetOrigin { player, to } => self.e.set_origin(player.0 as i32, to),
            Effect::SetBotsPathRevs(n) => self.s.bots.iter_mut().for_each(|b| b.profile.path_revs = n),
            Effect::SetBotsVisionRevs(n) => self.s.bots.iter_mut().for_each(|b| b.profile.vision_revs = n),
            Effect::SetBotsVisionTime(t) => self.s.bots.iter_mut().for_each(|b| b.profile.vision_time = t),
            // `SetViewAngles` resets to the configured turn speed every frame.
            Effect::SetBotsTurnSpeed(_) | Effect::ClearDebugMessage => {}
            Effect::ShowMenu { player, text, .. } => self.print(Print::Message(Some(player)), &text),
            Effect::AddToolTip(..) | Effect::SayToolTip(..) => {}
            other => self.not_ported(format!("{other:?}").split(['(', ' ', '{']).next().unwrap_or("it")),
        }
        if let Some(p) = to {
            editor::send(self.s, self.e, p, out);
        }
    }

    fn find_player(&self, name: &str) -> Option<PlayerId> {
        (1..=self.e.max_clients() as u32)
            .find(|&i| {
                self.player_entity(PlayerId(i))
                    .is_some_and(|e| e.is_client() && !e.netname.is_empty() && e.netname.to_lowercase().starts_with(&name.to_lowercase()))
            })
            .map(PlayerId)
    }

    fn player_by_user_id(&self, user_id: i32) -> Option<PlayerId> {
        (1..=self.e.max_clients()).find(|&i| self.e.user_id(i) == user_id).map(|i| PlayerId(i as u32))
    }

    fn player_name(&self, player: PlayerId) -> String {
        self.player_entity(player).map(|e| e.netname).unwrap_or_default()
    }

    fn player_origin(&self, player: PlayerId) -> Vector {
        self.player_entity(player).map_or(Vector::ZERO, |e| e.origin)
    }

    fn is_rcbot(&self, player: PlayerId) -> bool {
        self.s.bots.get(player.0).is_some()
    }

    fn bot_index(&self, player: PlayerId) -> i32 {
        if self.is_rcbot(player) { player.0 as i32 - 1 } else { -1 }
    }

    fn facing_entity(&self, player: PlayerId, _any: bool) -> Option<u32> {
        let p = self.player_entity(player)?;
        let (forward, _) = rcbot::bot::make_vectors(p.v_angle);
        let src = p.origin + p.view_ofs;
        let tr = self.world().trace_line(
            src,
            src + forward * 8192.0,
            rcbot::nav::Monsters::DontIgnore,
            rcbot::nav::Glass::DontIgnore,
            Some(player.0),
        );
        tr.hit.filter(|&h| h != 0)
    }

    fn bot_index_of_entity(&self, entity: u32) -> i32 {
        self.bot_index(PlayerId(entity))
    }

    fn nearest_waypoint(&self, origin: Vector, range: f32, visible: bool) -> i32 {
        let opts = NearestOptions { visible, ..NearestOptions::default() };
        self.s.waypoints.nearest_waypoint(origin, range, None, opts, &[], &self.world()).map_or(-1, |i| i as i32)
    }

    fn waypoint_flags(&self, waypoint: i32) -> u32 {
        self.s.waypoints.flags(waypoint)
    }

    fn set_waypoint_flags(&mut self, waypoint: i32, flags: u32) {
        if let Ok(i) = usize::try_from(waypoint) {
            self.s.waypoints.set_flags(i, flags);
        }
    }

    fn waypoint_origin(&self, waypoint: i32) -> Vector {
        self.s.waypoints.origin(waypoint)
    }

    /// `WaypointSave(false, NULL)`: the `.rcw` and the `.rcv` into the data directory.
    fn waypoint_save(&mut self) -> bool {
        let dir = self.s.data_dir.join("waypoints").join(&self.s.config.mod_folder);
        let n = self.s.waypoints.len();
        let rcv = self.s.waypoints.visibility.to_rcv(n);
        let rcw = self.s.waypoints.to_rcw(&self.s.map, rcv.is_some());
        let ok = std::fs::create_dir_all(&dir).is_ok() && std::fs::write(dir.join(format!("{}.rcw", self.s.map)), rcw).is_ok();
        if ok && let Some(v) = rcv {
            let _ = std::fs::write(dir.join(format!("{}.rcv", self.s.map)), v);
        }
        ok
    }

    fn num_clients(&self) -> i32 {
        (1..=self.e.max_clients() as u32).filter(|&i| self.player_entity(PlayerId(i)).is_some_and(|e| e.is_client())).count() as i32
    }

    fn leads_squad(&self, _player: PlayerId) -> bool {
        false
    }

    fn toggle_notarget(&mut self, player: PlayerId) -> bool {
        const FL_NOTARGET: i32 = 1 << 7;
        self.e.edict(player.0 as i32).is_some_and(|ed| {
            self.e.vars_mut(ed, |v| {
                v.flags ^= FL_NOTARGET;
                v.flags & FL_NOTARGET != 0
            })
        })
    }

    fn toggle_noclip(&mut self, player: PlayerId) -> bool {
        self.e.edict(player.0 as i32).is_some_and(|ed| {
            self.e.vars_mut(ed, |v| {
                let noclip = goldsrc_sys::MOVETYPE_NOCLIP as i32;
                v.movetype = if v.movetype == noclip { goldsrc_sys::MOVETYPE_WALK as i32 } else { noclip };
                v.movetype == noclip
            })
        })
    }

    fn toggle_godmode(&mut self, player: PlayerId) -> bool {
        self.e.edict(player.0 as i32).is_some_and(|ed| {
            self.e.vars_mut(ed, |v| {
                v.takedamage = if v.takedamage == 0.0 { 2.0 } else { 0.0 };
                v.takedamage == 0.0
            })
        })
    }

    fn botcam_tune_in(&mut self, _player: PlayerId) -> bool {
        self.not_ported("botcam");
        false
    }

    /// The rest of `BotFunc_AddBot`: a bot through the port (`create_bot`), RCBot's team choice
    /// as its `model` in teamplay.
    fn add_bot(&mut self, _caller: Option<usize>, request: AddBotRequest) -> CvarState {
        if !self.s.active {
            self.print(Print::Message(None), "the rcbot bot backend isn't active (ts_bot_backend rcbot, then change map)");
            return CvarState::Error;
        }
        let mut rng = EngineRandom(self.e);
        let teamplay = self.api.game_state().teamplay != 0;
        let model = if teamplay {
            let list = self.e.cvar_string("mp_teamlist");
            let force = request.team.filter(|&t| (1..5).contains(&t)).unwrap_or(self.s.config.force_team);
            cmds::ts_team_choice(&list, force, &mut rng).unwrap_or_default()
        } else {
            String::new()
        };
        let m = std::ffi::CString::new(model).unwrap_or_default();
        let idx = crate::create(self.s, self.api, self.e, "", m.as_ptr(), 0.0);
        if idx > 0 { CvarState::Accessed } else { CvarState::Error }
    }

    fn read_users_file(&mut self) -> (Option<Vec<u8>>, String) {
        let p = self.s.data_dir.join(BOT_USERS_FILE);
        (std::fs::read(&p).ok(), p.to_string_lossy().into_owned())
    }
}
