//! The `rcbot` commands (`RCBot_ServerCommand`, `ClientCommand`'s `rcbot`/`menuselect`,
//! `bot_config.ini`), `rcbot::config::CommandHost` on the engine, and loading RCBot's files from
//! the plugin's data directory (`<gamedir>/tsmod/rcbot/`, RCBot's bot folder).
//!
//! Effects the plugin API can't do (`util explo`, which needs the game's `ExplosionCreate`, and
//! `util givetsweapon`, which needs a weapon given at once) print that they aren't available
//! instead of doing nothing silently.

use crate::api::{Api, Engine};
use crate::world::{EngineRandom, HostWorld};
use crate::{State, editor, with_state};
use rcbot::Vector;
use rcbot::bot::BotWorld;
use rcbot::bot::squad::{CombatType as SquadCombat, SquadForm};
use rcbot::config::commands::{self as cmds, AddBotRequest, CombatType, CommandContext, CommandHost, CvarState, Effect, Formation, Print};
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

/// `ClientCommand`'s `rcbot <command>`, `menuselect` and `±rcbot_force_grip`: non-zero when
/// RCBot handled it.
pub unsafe extern "C" fn client_command(player: c_int, argc: c_int, argv: *const *const c_char) -> c_int {
    let args = args_of(argc, argv);
    let a = |i: usize| args.get(i).map_or("", String::as_str);
    if a(0) == "say" || a(0) == "say_team" {
        crate::player_say(player, args.clone());
        return 0;
    }
    if !matches!(a(0), rcbot::config::BOT_COMMAND_ACCESS | "menuselect" | "+rcbot_force_grip" | "-rcbot_force_grip") {
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
            let world = HostWorld::new(a, e);
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

/// `BotFunc_AddBot(NULL, ...)`: a bot with the defaults, as the rejoin code adds them.
pub fn add_bot_now(s: &mut State, a: Api, e: Engine) {
    let mut host = Host { s, api: a, e };
    host.add_bot(None, AddBotRequest { team: None, class: None });
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
        HostWorld::new(self.api, self.e)
    }

    fn player_entity(&self, p: PlayerId) -> Option<rcbot::bot::Entity> {
        self.world().entity(p.0)
    }

    /// `util explo`: the view trace (4096 units), its beam to the caller
    /// (`WaypointDrawBeam(…, 20, 10, 200, 1, 1, 160, 10)`), and [`explosion_create`] 4 units
    /// short of where it ends. Upstream's `UTIL_BotScreenShake` is left out (design §4).
    fn explosion(&mut self, player: PlayerId, magnitude: f32, damage: bool) {
        let Some(p) = self.player_entity(player) else { return };
        let (forward, _) = rcbot::bot::make_vectors(p.v_angle);
        let src = p.origin + p.view_ofs;
        let (_, end, _) = self.e.trace_line(src, src + forward * 4096.0, false, false, Some(player.0 as i32));
        self.e.beam(player.0 as i32, src, end, self.s.sprite, 20, 10, [200, 1, 1], 160, 10);
        explosion_create(self.api, self.e, end - forward * 4.0, player.0 as i32, magnitude, damage, self.s.fireball);
    }

    /// The squad `leader` leads (`FindSquadByLeader`).
    fn squad_of(&self, leader: PlayerId) -> Option<rcbot::bot::SquadId> {
        self.player_entity(leader).and_then(|e| self.s.squads.find_by_leader(e.handle()))
    }

    /// Changes the squad `leader` leads; what it says goes to the leader's chat area.
    fn with_squad(&mut self, leader: PlayerId, f: impl FnOnce(&mut rcbot::bot::squad::Squad, &mut Vec<String>)) {
        let mut said = Vec::new();
        if let Some(id) = self.squad_of(leader)
            && let Some(s) = self.s.squads.get_mut(id)
        {
            f(s, &mut said);
        }
        for m in said {
            self.print(Print::TalkOne(leader), &m);
        }
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
            Effect::ForceGrip { player, grip } => {
                if grip {
                    let e = self.facing_entity(player, false);
                    match e {
                        Some(e) => self.s.force_grips.insert(player.0, e),
                        None => self.s.force_grips.remove(&player.0),
                    };
                } else if let Some(e) = self.s.force_grips.remove(&player.0) {
                    crate::bots::release_force_grip(self.e, e);
                }
            }
            Effect::BotGoto { bot, waypoint } => {
                if let Some(b) = self.s.bots.get_mut(bot.0) {
                    b.go_to_waypoint(waypoint);
                }
            }
            Effect::GiveTsWeapon { player, weapon } => give_ts_weapon(self.api, self.e, player, weapon),
            Effect::Explosion { player, magnitude, damage } => self.explosion(player, magnitude, damage),
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
            Effect::ShowMenu { player, slots, text } => self.e.show_menu(player.0 as i32, slots, -1, false, &text),
            Effect::AddToolTip(..) | Effect::SayToolTip(..) => {}
            Effect::BotcamOff(p) => self.e.set_view(p.0 as i32, p.0 as i32),
            Effect::HudText { text, colour } => {
                let [r, g, b] = colour;
                for i in 1..=self.e.max_clients() {
                    self.e.hud_message(i, 3, (-1.0, -1.0), 0, [r, g, b, 200], [r, g, b, 200], (0.75, 0.75, 2.0, 0.75), &text);
                }
            }
            Effect::PlaySoundToAll(sample) => {
                for i in 1..=self.e.max_clients() {
                    if w.entity(i as u32).is_some_and(|p| !p.netname.is_empty()) {
                        self.e.client_command_sound(i, &sample);
                    }
                }
            }
            Effect::Search { player, range } => {
                if let Some(p) = self.player_entity(player) {
                    for i in w.entities_in_sphere(p.origin, range) {
                        let Some(e) = w.entity(i) else { continue };
                        let line =
                            format!("Found: id: {i}, classname:{} distance:{:.2}", e.classname, (e.entity_origin() - p.origin).length());
                        self.print(Print::Message(Some(player)), &line);
                    }
                }
            }
            Effect::TeleportEntityInFront { player, to } => {
                // `UTIL_getEntityInFront`: 8192 units, anything but the world.
                if let Some(i) = view_trace(&w, player.0, 8192.0).filter(|&i| i != 0)
                    && w.entity(i).is_some_and(|e| e.flags & rcbot::bot::entflags::WORLDBRUSH == 0)
                {
                    self.e.set_origin(i as i32, to);
                }
            }
            Effect::PrintBoredoms { bot, to } => {
                if let Some(t) = self.s.bots.get(bot.0).map(|b| b.boredom_text()) {
                    self.print(Print::Message(to), &t);
                }
            }
            Effect::PrintReps { bot, to } => {
                let names: Vec<(i32, String)> =
                    self.s.clients.iter().filter_map(|c| c.player.map(|p| (c.player_rep_id, self.player_name(p)))).collect();
                let lines = self
                    .s
                    .bots
                    .get(bot.0)
                    .map(|b| b.profile.rep.print_reps(&b.name, |id| names.iter().find(|(r, _)| *r == id).map(|(_, n)| n.clone())));
                for l in lines.unwrap_or_default() {
                    self.print(Print::Message(to), &l);
                }
            }
            Effect::SquadFormation { leader, formation } => {
                let f = match formation {
                    Formation::Wedge => SquadForm::Wedge,
                    Formation::Line => SquadForm::Line,
                    Formation::Column => SquadForm::Column,
                    Formation::EchelonLeft => SquadForm::EchelonLeft,
                    Formation::EchelonRight => SquadForm::EchelonRight,
                    Formation::Vee => SquadForm::Vee,
                };
                self.with_squad(leader, |s, _| s.change_formation(f));
            }
            Effect::SquadSetSpread { leader, spread } => self.with_squad(leader, |s, _| s.change_spread(spread)),
            Effect::SquadChangeSpread { leader, delta } => self.with_squad(leader, |s, _| s.change_spread(s.spread() + delta)),
            Effect::SquadToggleFireMode(leader) => {
                self.with_squad(leader, |s, out| out.push(s.toggle_fire_mode().to_owned()));
            }
            Effect::SquadToggleCombatType { leader, combat_type } => self.with_squad(leader, |s, out| {
                let t = match combat_type {
                    CombatType::Crouch if !s.is_crouch() => SquadCombat::Crouch,
                    CombatType::Stealth if !s.is_stealth() => SquadCombat::Stealth,
                    _ => SquadCombat::Combat,
                };
                out.extend(s.set_combat_type(t));
            }),
            Effect::SquadReturnToFormation(leader) => {
                if let Some(id) = self.squad_of(leader) {
                    let bots: Vec<u32> = self.s.bots.iter().map(|b| b.index).collect();
                    self.s.squads.return_all_to_formation(id, |i| bots.contains(&i));
                }
            }
            Effect::SquadLeave(leader) => {
                if let Some(id) = self.squad_of(leader) {
                    self.s.squads.change_leader(id, &w);
                }
            }
            Effect::SquadRemove(leader) => {
                if let Some(id) = self.squad_of(leader) {
                    self.s.squads.remove(id);
                }
            }
            Effect::MakeSquad(c) => {
                if let Some(p) = self.editor_player(c) {
                    let reps = self.s.rep_ids();
                    let s = &mut *self.s;
                    let mut g = crate::bots::globals!(s, &reps);
                    s.bots.make_squad(&mut g, &w, &p);
                }
            }
            Effect::RemoveAllBotSquads(p) => {
                let team = self.player_entity(p).and_then(|e| e.player).map_or(0, |x| x.team);
                let n = crate::bots::remove_all_bot_squads(self.s, team);
                self.print(Print::HudTalk(p), &format!("{BOT_DBG_MSG_TAG} {n} bot squads removed"));
            }
        }
        if let Some(p) = to {
            editor::send(self.s, self.e, p, out);
        }
        crate::bots::flush_squad_messages(self.s, self.e);
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

    fn facing_entity(&self, player: PlayerId, any: bool) -> Option<u32> {
        let w = self.world();
        let hit = view_trace(&w, player.0, 4096.0)?;
        let e = w.entity(hit)?;
        (any || e.flags & (rcbot::bot::entflags::CLIENT | rcbot::bot::entflags::MONSTER) != 0).then_some(hit)
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

    fn leads_squad(&self, player: PlayerId) -> bool {
        self.squad_of(player).is_some()
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

    fn botcam_tune_in(&mut self, player: PlayerId) -> bool {
        match crate::bots::botcam_tune_in(self.s, self.e, player.0) {
            None => true,
            Some(why) => {
                self.print(Print::Message(Some(player)), why);
                false
            }
        }
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
        let idx = crate::create(self.s, self.api, self.e, c"", m.as_ptr(), 0.0);
        if idx > 0 { CvarState::Accessed } else { CvarState::Error }
    }

    fn read_users_file(&mut self) -> (Option<Vec<u8>>, String) {
        let p = self.s.data_dir.join(BOT_USERS_FILE);
        (std::fs::read(&p).ok(), p.to_string_lossy().into_owned())
    }
}

/// The entity a player looks at within `dist` (`UTIL_TraceLine` from the eyes along `v_angle`,
/// monsters and glass counted, the player ignored): 0 is the world.
fn view_trace(w: &HostWorld, player: u32, dist: f32) -> Option<u32> {
    let p = w.entity(player)?;
    let (forward, _) = rcbot::bot::make_vectors(p.v_angle);
    let src = p.origin + p.view_ofs;
    w.trace_line(src, src + forward * dist, rcbot::nav::Monsters::DontIgnore, rcbot::nav::Glass::DontIgnore, Some(player)).hit
}

/// `UTIL_makeTSweapon(owner, id)`: a `ts_groundweapon` of TS weapon `weapon` with 200 extra
/// rounds that doesn't respawn (`wextraclip 200`, `wduration -1`, spawnflags `1 << 30`, as keys
/// and then set again), spawned, moved to the owner and used by them at once (`DispatchUse`), then
/// marked `FL_KILLME`. Nothing for weapon 0.
fn give_ts_weapon(a: Api, e: Engine, player: PlayerId, weapon: i32) {
    if weapon == 0 {
        return;
    }
    let Some(w) = a.create_entity(c"ts_groundweapon") else { return };
    for (key, value) in
        [("tsweaponid", weapon.to_string()), ("wextraclip", "200".into()), ("wduration", "-1".into()), ("spawnflags", "1073741824".into())]
    {
        a.entity_key_value(w, key, &value);
    }
    if let Some(ed) = e.edict(w) {
        e.vars_mut(ed, |v| v.spawnflags = 1 << 30);
    }
    if a.spawn_entity(w) != ts_plugin_api::OK {
        return;
    }
    let Some(origin) = e.edict(player.0 as i32).map(|p| e.vars(p, |v, _| Vector::from(v.origin))) else { return };
    e.set_origin(w, origin);
    a.use_entity(w, player.0 as i32);
    if let Some(ed) = e.edict(w) {
        e.vars_mut(ed, |v| v.flags |= goldsrc_sys::FL_KILLME as i32);
    }
}

/// `ExplosionCreate(center, angles, owner, magnitude, doDamage)` (upstream's live branch): a
/// `TE_EXPLOSION` of the fireball at `magnitude` / 10 scale (at most 255), and with `damage`
/// [`radius_damage`] of `magnitude` within `magnitude / 2`, `DMG_BLAST`, by the owner.
fn explosion_create(a: Api, e: Engine, center: Vector, owner: i32, magnitude: f32, damage: bool, fireball: i32) {
    e.temp_explosion(center, fireball, magnitude as i32);
    if damage {
        radius_damage(a, e, center, owner, magnitude, magnitude / 2.0, 1 << 6);
    }
}

/// RCBot's own `RadiusDamage(vecSrc, inflictor, attacker, damage, radius, CLASS_NONE, bits)`:
/// every entity within `radius` that takes damage, on the source's side of the water line, that
/// the blast sees (the trace to its `BodyTarget` reaches it), takes `damage` less `damage /
/// radius` per unit of distance (`TakeDamage` through the API, with the plugins' hooks) and is
/// pushed away at that speed.
fn radius_damage(a: Api, e: Engine, mut src: Vector, owner: i32, damage: f32, radius: f32, bits: i32) {
    const CONTENTS_WATER: i32 = -3;
    let falloff = if radius != 0.0 { damage / radius } else { 1.0 };
    let in_water = e.point_contents(src) == CONTENTS_WATER;
    src.z += 1.0;
    for i in e.entities_in_sphere(src, radius) {
        let Some(ed) = e.edict(i) else { continue };
        let (takes, waterlevel, origin, centre, view_ofs, client) = e.vars(ed, |v, _| {
            let centre = (Vector::from(v.absmin) + Vector::from(v.absmax)) * 0.5;
            (
                v.takedamage != 0.0,
                v.waterlevel,
                Vector::from(v.origin),
                centre,
                Vector::from(v.view_ofs),
                v.flags & goldsrc_sys::FL_CLIENT as i32 != 0,
            )
        });
        if !takes || (in_water && waterlevel == 0) || (!in_water && waterlevel == 3) {
            continue;
        }
        // `BodyTarget(vecSrc)`: `Center()`, a player's raised by its view offset times a random
        // 0.5..1.1 (`CBasePlayer::BodyTarget`).
        let spot = if client { centre + view_ofs * e.random_float(0.5, 1.1) } else { centre };
        let (fraction, end, hit, start_solid) = e.trace_line_solid(src, spot, false, Some(owner));
        if !(fraction == 1.0 || (!hit.is_null() && e.index_of(hit) == i)) {
            continue;
        }
        // Stuck inside it: the distance is 0.
        let end = if start_solid { src } else { end };
        let adjusted = (damage - (src - end).length() * falloff).max(0.0);
        a.take_damage(i, owner, owner, adjusted, bits);
        if let Some(ed) = e.edict(i) {
            e.vars_mut(ed, |v| v.velocity = ((origin - src).normalize() * adjusted).to_array());
        }
    }
}
