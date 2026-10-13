//! `tsmod_rcbot`: the plugin library the TS Rust port loads (plugin API v3,
//! the-specialistsmod-rs#104), and the only crate in this workspace allowed `unsafe` (every other
//! crate is `#![deny(unsafe_code)]`, which `cargo xtask lint` checks).
//!
//! It registers the `rcbot` bot backend (`ts_bot_backend rcbot`) and the `rcbot` server and
//! client commands, keeps RCBot's globals (`gBotGlobals`: config, waypoints, weapons, learning,
//! the bots) in one [`State`], and every frame runs each bot ([`rcbot::bot::Bot::frame`]) and
//! hands its usercmd and actions to the port (`run_cmd`, `bot_command`, `select_weapon`, `buy`).
//!
//! Re-entrancy: the server calls back into the plugin from inside API calls (`create_bot` spawns
//! the bot, `run_cmd` runs its think). The state sits behind a mutex that callbacks only
//! `try_lock`; a callback that finds it held queues an [`Event`] that runs before the next
//! frame.
#![warn(clippy::undocumented_unsafe_blocks)]
#![allow(clippy::missing_safety_doc)] // the exports are called by the server only

mod api;
mod commands;
mod editor;
mod world;

use api::{Api, Engine};
use rcbot::Vector;
use rcbot::bot::{Action, Bot, BotGlobals, BotManager};
use rcbot::config::{AllowedPlayers, BotProfile, Clients, Config, PlayerId};
use rcbot::learn::GlobalLearnedData;
use rcbot::nav::Waypoints;
use rcbot::weapons::{WeaponPresets, Weapons};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};
use ts_plugin_api::{BotBackend, BotCmd, INVALID, LoadInfo, OK, Plugin, PluginApi, VERSION, log};
use world::{EngineRandom, HostWorld};

/// The bot backend name this plugin registers (`ts_bot_backend rcbot`).
pub const BACKEND_NAME: &str = "rcbot";

static API: AtomicPtr<PluginApi> = AtomicPtr::new(std::ptr::null_mut());
static PLUGIN_ID: AtomicI32 = AtomicI32::new(0);
static STATE: Mutex<Option<State>> = Mutex::new(None);
static PENDING: Mutex<Vec<Event>> = Mutex::new(Vec::new());

/// A callback that came while the state was busy.
#[derive(Clone, Debug)]
enum Event {
    PutInServer(i32, bool),
    Disconnect(i32),
    Damaged(i32, i32),
}

/// What the e2e test and `rcbot` status read about a bot.
#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    start: Option<Vector>,
    moved: f32,
}

/// RCBot's globals.
pub struct State {
    pub(crate) config: Config,
    pub(crate) users: AllowedPlayers,
    pub(crate) clients: Clients,
    pub(crate) waypoints: Waypoints,
    pub(crate) weapons: Weapons,
    pub(crate) presets: WeaponPresets,
    pub(crate) learned: GlobalLearnedData,
    pub(crate) bots: BotManager,
    pub(crate) data_dir: PathBuf,
    pub(crate) map: String,
    pub(crate) active: bool,
    pub(crate) sprite: i32,
    pub(crate) editor: editor::Editor,
    stats: HashMap<u32, Stats>,
    next_report: f32,
    checks: Vec<&'static str>,
    added: u32,
}

pub(crate) fn api() -> Option<Api> {
    let p = API.load(Ordering::Relaxed);
    // SAFETY: a non-NULL pointer is the server's static table, which lives for the process.
    (!p.is_null()).then(|| Api::new(unsafe { &*p }, PLUGIN_ID.load(Ordering::Relaxed)))
}

fn say(level: c_int, msg: &str) {
    if let Some(a) = api() {
        a.log(level, msg);
    }
}

impl State {
    /// Logs `check <what> ok` once (the e2e test's markers).
    fn check(&mut self, what: &'static str) {
        if !self.checks.contains(&what) {
            self.checks.push(what);
            say(log::INFO, &format!("check {what} ok"));
        }
    }
}

/// Runs `f` on the state, or queues `event` when it is held further up the stack.
fn with_state<R>(event: Option<Event>, f: impl FnOnce(&mut State, Api, Engine) -> R) -> Option<R> {
    let (a, e) = api().and_then(|a| a.engine().map(|e| (a, e)))?;
    match STATE.try_lock() {
        Ok(mut g) => {
            let s = g.as_mut()?;
            drain(s, a, e);
            Some(f(s, a, e))
        }
        Err(_) => {
            if let (Some(ev), Ok(mut q)) = (event, PENDING.lock()) {
                q.push(ev);
            }
            None
        }
    }
}

/// Runs the queued events.
fn drain(s: &mut State, a: Api, e: Engine) {
    let events: Vec<Event> = PENDING.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for ev in events {
        match ev {
            Event::PutInServer(i, is_bot) => put_in_server(s, a, e, i, is_bot),
            Event::Disconnect(i) => disconnect(s, i),
            Event::Damaged(victim, attacker) => damaged(s, e, victim, attacker),
        }
    }
}

// ---- the entry point ---------------------------------------------------------------------------

/// `TSRS_PluginLoad`: checks the table, registers the backend and the `rcbot` command, fills the
/// descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn TSRS_PluginLoad(
    api_ptr: *const PluginApi,
    api_size: u32,
    info: *const LoadInfo,
    out: *mut Plugin,
    out_size: u32,
) -> c_int {
    if api_ptr.is_null() || info.is_null() || out.is_null() || (out_size as usize) < size_of::<Plugin>() {
        return INVALID;
    }
    // SAFETY: the server passes its static table, valid for the process.
    let table: &'static PluginApi = unsafe { &*api_ptr };
    if (api_size as usize) < size_of::<PluginApi>() || table.version != VERSION {
        return INVALID;
    }
    // SAFETY: `info` is valid for the call.
    let id = unsafe { (*info).id };
    API.store(api_ptr.cast_mut(), Ordering::Relaxed);
    PLUGIN_ID.store(id, Ordering::Relaxed);
    let a = Api::new(table, id);
    let rc = a.register_bot_backend(BACKEND_NAME, &BACKEND);
    if rc != OK {
        a.log(log::WARNING, &format!("bot backend \"{BACKEND_NAME}\" not registered ({rc})"));
    }
    a.register_command(rcbot::config::BOT_COMMAND_ACCESS, commands::server_command);
    a.register_cvar("rcbot_ver", rcbot::config::BOT_VER);
    // SAFETY: `out` is a writable `Plugin` (size checked above).
    unsafe { out.write(DESCRIPTOR) };
    OK
}

const DESCRIPTOR: Plugin = Plugin {
    name: c"RCBot".as_ptr(),
    version: c"0.1".as_ptr(),
    game_init: Some(game_init),
    precache: Some(precache),
    server_activate: Some(server_activate),
    server_deactivate: Some(server_deactivate),
    start_frame: Some(start_frame),
    client_put_in_server: Some(client_put_in_server),
    client_disconnect: Some(client_disconnect),
    client_command: Some(commands::client_command),
    player_damaged: Some(player_damaged),
    ..Plugin::EMPTY
};

const BACKEND: BotBackend = BotBackend {
    size: size_of::<BotBackend>() as u32,
    add_bot: Some(add_bot),
    activate: Some(activate),
    deactivate: Some(deactivate),
    adopt_bot: Some(adopt_bot),
};

// ---- plugin callbacks ----------------------------------------------------------------------------

/// `GameDLLInit`: RCBot's `CBotGlobals` set up: the data folder, `bot_weapons.ini`,
/// `bot_users.ini`, the learned GAs, then `bot_config.ini`.
extern "C" fn game_init() {
    let Some(a) = api() else { return };
    let Some(dir) = a.data_dir() else {
        a.log(log::ERROR, "no data directory");
        return;
    };
    let data_dir = PathBuf::from(dir.trim_end_matches(['/', '\\']));
    let mut config = Config { bot_folder: data_dir.to_string_lossy().into_owned(), ..Config::default() };
    let ini = std::fs::read_to_string(data_dir.join("bot_weapons.ini")).unwrap_or_default();
    let (presets, errors) = WeaponPresets::parse(&ini, rcbot::weapons::MOD_TS);
    for e in errors {
        a.log(log::WARNING, &format!("bot_weapons.ini line {}: {}", e.line, e.text));
    }
    let mut learned = GlobalLearnedData::default();
    for team in 0..rcbot::learn::data::MAX_TEAMS {
        if let Ok(b) = std::fs::read(data_dir.join(rcbot::learn::data::team_file_name(team))) {
            learned.load_team(team, &b);
        }
    }
    if let Ok(b) = std::fs::read(data_dir.join(rcbot::learn::data::TS_WEAPONS_FILE)) {
        learned.load_ts_weapons(&b);
    }
    config.mod_folder = "ts".into();
    let mut state = State {
        config,
        users: AllowedPlayers::default(),
        clients: Clients::default(),
        waypoints: Waypoints::new(),
        weapons: Weapons::default(),
        presets,
        learned,
        bots: BotManager::default(),
        data_dir,
        map: String::new(),
        active: false,
        sprite: 0,
        editor: editor::Editor::default(),
        stats: HashMap::new(),
        next_report: 0.0,
        checks: Vec::new(),
        added: 0,
    };
    a.log(log::INFO, &format!("RCBot {} for TS 3.0, data in {}", rcbot::config::BOT_VER, state.data_dir.display()));
    if let Some(e) = a.engine() {
        commands::read_users(&mut state, a, e);
        commands::read_config(&mut state, a, e);
    }
    state.check("data_dir");
    if let Ok(mut g) = STATE.lock() {
        *g = Some(state);
    }
}

/// The map's precache: the beam sprite (`m_iWaypointTexture`).
extern "C" fn precache() {
    let Some(e) = api().and_then(|a| a.engine()) else { return };
    let sprite = e.precache_model(c"sprites/lgtning.spr");
    with_state(None, |s, _, _| s.sprite = sprite);
}

/// `ServerActivate`: `WaypointLoad` for the map, and the visibility table.
unsafe extern "C" fn server_activate(map: *const c_char) {
    // SAFETY: the server's string for the call.
    let map = if map.is_null() { String::new() } else { unsafe { CStr::from_ptr(map) }.to_string_lossy().into_owned() };
    with_state(None, |s, a, e| {
        s.map = map;
        s.weapons.clear();
        s.bots.clear();
        s.stats.clear();
        commands::load_waypoints(s, a, e);
    });
}

extern "C" fn server_deactivate() {
    with_state(None, |s, _, _| {
        s.bots.clear();
        s.editor = editor::Editor::default();
    });
}

/// `StartFrame`: each bot thinks and moves; the waypoint editor draws.
extern "C" fn start_frame() {
    with_state(None, |s, a, e| {
        editor::frame(s, a, e);
        if !s.active {
            return;
        }
        let gs = a.game_state();
        let w = HostWorld { api: a, eng: e, teamplay: gs.teamplay != 0 };
        let mut rng = EngineRandom(e);
        let mut g = BotGlobals {
            config: &s.config,
            waypoints: &mut s.waypoints,
            weapons: &mut s.weapons,
            presets: &s.presets,
            learned: &mut s.learned,
            team_scores: [0; 4],
        };
        let frames = s.bots.frame(&mut g, &w, &mut rng);
        for (idx, f) in frames {
            let i = idx as i32;
            for act in f.actions {
                match act {
                    Action::Command(line) => {
                        a.bot_command(i, &line);
                    }
                    Action::SelectWeapon(id) => {
                        a.select_weapon(i, id);
                    }
                    Action::Loadout(ids) => {
                        a.clear_loadout(i);
                        for id in ids.into_iter().filter(|&id| (1..=37).contains(&id) && id != 29) {
                            a.buy(i, id, 4);
                        }
                    }
                    Action::Kill => {
                        a.bot_command(i, "kill");
                    }
                    Action::Kick => {
                        a.kick_bot(i);
                    }
                }
            }
            if let Some(c) = f.cmd {
                let cmd = BotCmd {
                    angles: c.angles.to_array(),
                    forward: c.forward,
                    side: c.side,
                    up: c.up,
                    buttons: c.buttons,
                    impulse: c.impulse,
                    msec: i32::from(c.msec).clamp(1, 255),
                };
                if a.run_cmd(i, &cmd) == OK {
                    s.check("run_cmd");
                }
            }
        }
        report(s, a, e);
    });
}

/// Tracks how far bots got, and logs a status line per bot every 5 s.
fn report(s: &mut State, a: Api, e: Engine) {
    let now = e.time();
    let mut moved = false;
    let mut used_waypoints = false;
    let mut attacked = false;
    for b in s.bots.iter() {
        let Some(origin) = e.edict(b.index as i32).map(|ed| e.vars(ed, |v, _| Vector::from(v.origin))) else { continue };
        let alive = b.is_alive();
        let st = s.stats.entry(b.index).or_default();
        if !alive {
            st.start = None;
            continue;
        }
        let start = *st.start.get_or_insert(origin);
        st.moved = st.moved.max((origin - start).length_2d());
        moved |= st.moved > 64.0;
        used_waypoints |= b.touched_waypoints >= 3;
        attacked |= b.enemy().is_some();
    }
    if moved {
        s.check("moved");
    }
    if used_waypoints {
        s.check("waypoints");
    }
    if attacked {
        s.check("enemy");
    }
    if now < s.next_report {
        return;
    }
    s.next_report = now + 5.0;
    for b in s.bots.iter() {
        let st = s.stats.get(&b.index).copied().unwrap_or_default();
        let task = b.tasks().current().map_or("none", |t| t.kind.description());
        a.log(
            log::INFO,
            &format!(
                "bot #{} \"{}\" alive={} wpt={} goal={} touched={} moved={:.0} enemy={:?} weapon={:?} task={task}",
                b.index,
                b.name,
                b.is_alive(),
                b.current_waypoint(),
                b.goal_waypoint(),
                b.touched_waypoints,
                st.moved,
                b.enemy(),
                b.current_weapon()
            ),
        );
    }
}

/// `ClientPutInServer`: a human gets a `CClient`; a bot made elsewhere is ignored.
fn put_in_server(s: &mut State, a: Api, e: Engine, i: i32, is_bot: bool) {
    if is_bot {
        return;
    }
    let name = e.edict(i).map(|ed| e.vars(ed, |v, _| v.netname)).map(|n| e.string(n)).unwrap_or_default();
    let dedicated = e.is_dedicated();
    if let Some((_, Some(msg))) = s.clients.connect(PlayerId(i as u32), &name, "", -1, &s.users, dedicated) {
        e.client_print(i, &format!("{}{msg}\n", rcbot::config::BOT_DBG_MSG_TAG));
    }
    let _ = a;
}

extern "C" fn client_put_in_server(player: c_int, is_bot: c_int) {
    with_state(Some(Event::PutInServer(player, is_bot != 0)), |s, a, e| put_in_server(s, a, e, player, is_bot != 0));
}

fn disconnect(s: &mut State, i: i32) {
    s.bots.remove(i as u32);
    s.stats.remove(&(i as u32));
    if let Some(ci) = s.clients.index_of(PlayerId(i as u32)) {
        s.clients.disconnect(ci);
    }
}

extern "C" fn client_disconnect(player: c_int, _is_bot: c_int) {
    with_state(Some(Event::Disconnect(player)), |s, _, _| disconnect(s, player));
}

/// `BotEvent(BOT_EVENT_HURT)`: the bot looks toward who hurt it.
fn damaged(s: &mut State, e: Engine, victim: i32, attacker: i32) {
    let from = e.edict(attacker).map(|ed| e.vars(ed, |v, _| Vector::from(v.origin)));
    if let (Some(b), Some(from)) = (s.bots.get_mut(victim as u32), from) {
        b.hurt(from, e.time());
    }
}

extern "C" fn player_damaged(victim: c_int, attacker: c_int, _inflictor: c_int, _damage: f32, _bits: c_int, _health: f32) {
    if attacker <= 0 || attacker == victim {
        return;
    }
    with_state(Some(Event::Damaged(victim, attacker)), |s, _, e| damaged(s, e, victim, attacker));
}

// ---- the bot backend -------------------------------------------------------------------------------

/// The rest of `BotFunc_AddBot` once the port asks for a bot: a profile (RCBot's defaults; the
/// `bot_profiles.ini` choice comes with step 7), the fake client, the `CBot`.
pub(crate) fn create(s: &mut State, a: Api, e: Engine, name: &str, model: *const c_char, skill: f32) -> c_int {
    let mut rng = EngineRandom(e);
    let mut profile = BotProfile::init(&s.config, &mut rng);
    if skill > 0.0 {
        profile.skill = if skill <= 1.0 { (skill * 100.0) as i32 } else { skill as i32 };
    }
    s.added += 1;
    let name = if name.is_empty() {
        profile.bot_name.clone().unwrap_or_else(|| rcbot::config::profile::BOT_DEFAULT_NAME.into())
    } else {
        name.into()
    };
    let idx = a.create_bot(&name, model, profile.skill as f32 / 100.0);
    if idx <= 0 {
        a.log(log::WARNING, &format!("create_bot \"{name}\": {idx}"));
        return idx;
    }
    let bot = Bot::new(idx as u32, name.clone(), profile, &s.config, &mut rng);
    s.bots.add(bot);
    a.log(log::INFO, &format!("added \"{name}\" (#{idx})"));
    s.check("create_bot");
    idx
}

/// `addbot`, `addcustombot`, the quota.
unsafe extern "C" fn add_bot(name: *const c_char, model: *const c_char, skill: f32) -> c_int {
    // SAFETY: the server passes NUL-terminated strings for the call.
    let name = if name.is_null() { String::new() } else { unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned() };
    with_state(None, |s, a, e| create(s, a, e, &name, model, skill)).unwrap_or(0)
}

extern "C" fn activate() {
    with_state(None, |s, a, _| {
        s.active = true;
        a.log(log::INFO, "bot backend active");
        s.check("backend");
    });
}

extern "C" fn deactivate() {
    with_state(None, |s, a, _| {
        s.active = false;
        s.bots.clear();
        a.log(log::INFO, "bot backend inactive");
    });
}

/// A quota bot the engine kept across the level change: a new `CBot` for it.
extern "C" fn adopt_bot(idx: c_int) {
    with_state(None, |s, _, e| {
        let name = e.edict(idx).map(|ed| e.vars(ed, |v, _| v.netname)).map(|n| e.string(n)).unwrap_or_default();
        let mut rng = EngineRandom(e);
        let profile = BotProfile::init(&s.config, &mut rng);
        let bot = Bot::new(idx as u32, name, profile, &s.config, &mut rng);
        s.bots.add(bot);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_descriptor_matches_this_api() {
        assert_eq!(DESCRIPTOR.size as usize, size_of::<Plugin>());
        assert_eq!(DESCRIPTOR.api_version, VERSION);
        assert_eq!(BACKEND.size as usize, size_of::<BotBackend>());
    }

    #[test]
    fn a_null_call_is_refused() {
        let mut out = Plugin::EMPTY;
        // SAFETY: NULL arguments are refused before anything is read.
        let rc = unsafe { TSRS_PluginLoad(std::ptr::null(), 0, std::ptr::null(), &mut out, size_of::<Plugin>() as u32) };
        assert_eq!(rc, INVALID);
    }
}
