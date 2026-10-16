//! `tsmod_rcbot`: the plugin library the TS Rust port loads (plugin API v5,
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
mod bots;
mod commands;
mod editor;
mod world;

use api::{Api, Engine};
use bots::globals;
use rcbot::Vector;
use rcbot::bot::{Action, BotCam, BotManager, MasterEntity, Squads};
use rcbot::chat::bot_chat::BotChatMessages;
use rcbot::config::{AllowedPlayers, Clients, Config};
use rcbot::learn::GlobalLearnedData;
use rcbot::nav::Waypoints;
use rcbot::weapons::{WeaponPresets, Weapons};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};
use ts_plugin_api::{BotBackend, BotCmd, INVALID, KillInfo, LoadInfo, OK, Plugin, PluginApi, VERSION, log};
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
    Damaged(i32, i32, i32, f32, i32, f32),
    Killed(i32, i32),
    KillScored(KillInfo),
    Stunt(i32),
    Sound(i32, Option<Vector>, f32, String),
    Say(i32, Vec<String>),
}

/// What the e2e test and `rcbot` status read about a bot.
#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    start: Option<Vector>,
    moved: f32,
    /// The weapon in hand while alive, to log changes.
    weapon: Option<i32>,
    /// The view's yaw when the life started, and the most it turned from it.
    start_yaw: Option<f32>,
    turned: f32,
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
    /// `m_sModelIndexFireball` (`sprites/zerogxplode.spr`, `util explo`).
    pub(crate) fireball: i32,
    /// `m_iTeamScores`: the `TeamScore` values (`GameState::team_scores`), refreshed every frame.
    pub(crate) team_scores: [i32; 4],
    pub(crate) editor: editor::Editor,
    pub(crate) squads: Squads,
    pub(crate) botcam: BotCam,
    pub(crate) chat: BotChatMessages,
    /// `m_Masters`: the plugin API doesn't report key values, so it stays empty (design §4).
    pub(crate) masters: Vec<MasterEntity>,
    /// The profile each bot had, by player index, for the bots the port keeps across a level
    /// change (upstream's `RESPAWN_NEED_TO_REJOIN` bots rejoin with their profile).
    previous_profiles: HashMap<u32, i32>,
    /// `CClient::m_pForceGripEntity` by player: what `+rcbot_force_grip` holds.
    pub(crate) force_grips: HashMap<u32, u32>,
    /// `m_fMapInitTime`.
    map_init_time: f32,
    /// `m_bBotCanRejoin`, `m_fBotRejoinTime`.
    can_rejoin: bool,
    rejoin_time: f32,
    /// `fpMapConfig` (`map_configs/ts/<map>.cfg`, what is left of it) and `m_fReadConfigTime`.
    map_config: std::collections::VecDeque<String>,
    read_config_time: f32,
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

pub(crate) fn say(level: c_int, msg: &str) {
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
            Event::PutInServer(i, is_bot) => bots::put_in_server(s, e, i, is_bot),
            Event::Disconnect(i) => bots::disconnect(s, a, e, i),
            Event::Damaged(victim, attacker, inflictor, damage, bits, health) => {
                damaged(s, a, e, victim, attacker, inflictor, damage, bits, health)
            }
            Event::Killed(victim, killer) => bots::killed(s, a, e, victim, killer),
            Event::KillScored(k) => bots::kill_scored(s, a, e, &k),
            Event::Stunt(player) => bots::stunt(s, a, e, player),
            Event::Sound(entity, origin, volume, sample) => bots::sound(s, a, e, entity, origin, volume, &sample),
            Event::Say(player, args) => bots::say_command(s, a, e, player, &args),
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
    shutdown: Some(shutdown),
    client_put_in_server: Some(client_put_in_server),
    client_disconnect: Some(client_disconnect),
    client_command: Some(commands::client_command),
    player_damaged: Some(player_damaged),
    player_killed: Some(player_killed),
    sound: Some(sound),
    kill_scored: Some(kill_scored),
    player_stunt: Some(player_stunt),
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
    if let Ok(b) = std::fs::read(data_dir.join(rcbot::learn::weapon_stats::GLOBAL_FILE)) {
        learned.weapon_prior.load(&String::from_utf8_lossy(&b));
    }
    if let Ok(b) = std::fs::read(data_dir.join(rcbot::learn::loadout::GLOBAL_FILE)) {
        learned.loadout_prior.load(&String::from_utf8_lossy(&b));
    }
    let players = data_dir.join(rcbot::config::BOT_PROFILES_FOLDER).join(rcbot::learn::opponents::PLAYERS_FILE);
    if let Ok(b) = std::fs::read(players) {
        learned.opponents.load(&String::from_utf8_lossy(&b));
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
        fireball: 0,
        team_scores: [0; 4],
        editor: editor::Editor::default(),
        squads: Squads::default(),
        botcam: BotCam::default(),
        chat: BotChatMessages::default(),
        masters: Vec::new(),
        previous_profiles: HashMap::new(),
        force_grips: HashMap::new(),
        map_init_time: 0.0,
        can_rejoin: false,
        rejoin_time: 5.0,
        map_config: Default::default(),
        read_config_time: 0.0,
        stats: HashMap::new(),
        next_report: 0.0,
        checks: Vec::new(),
        added: 0,
    };
    // `SetupBotChat`.
    if let Ok(b) = std::fs::read(state.data_dir.join("bot_chat.ini")) {
        state.chat = BotChatMessages::parse(&b);
    }
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

/// The map's precache: the beam sprite (`m_iWaypointTexture`) and the bot camera's model.
extern "C" fn precache() {
    let Some(e) = api().and_then(|a| a.engine()) else { return };
    let sprite = e.precache_model(c"sprites/lgtning.spr");
    e.precache_model(c"models/mechgibs.mdl");
    // `CBotGlobals::MapInit`'s `m_sModelIndexFireball`.
    let fireball = e.precache_model(c"sprites/zerogxplode.spr");
    with_state(None, |s, _, _| (s.sprite, s.fireball) = (sprite, fireball));
}

/// `ServerActivate`: `WaypointLoad` for the map, and the visibility table.
unsafe extern "C" fn server_activate(map: *const c_char) {
    // SAFETY: the server's string for the call.
    let map = if map.is_null() { String::new() } else { unsafe { CStr::from_ptr(map) }.to_string_lossy().into_owned() };
    // `CBotGlobals::MapInit`: TS's `tm_` maps are team maps (`NON_TS_TEAMPLAY`, `m_bTeamPlay`).
    world::NON_TS_TEAMPLAY.store(map.starts_with("tm_"), Ordering::Relaxed);
    with_state(None, |s, a, e| {
        s.map = map;
        world::refresh_game_state(a);
        s.weapons.clear();
        s.bots.clear();
        s.stats.clear();
        s.squads.clear();
        s.botcam.clear();
        commands::load_waypoints(s, a, e);
        bots::load_heatmap(s);
        s.learned.opponents.new_map();
        // `CBotGlobals::MapInit`.
        s.map_init_time = e.time();
        s.rejoin_time = e.time();
        s.can_rejoin = false;
        let cfg = s.data_dir.join("map_configs").join(&s.config.mod_folder).join(format!("{}.cfg", s.map));
        s.map_config = match std::fs::read(&cfg) {
            Ok(b) => {
                a.log(log::INFO, "map specific config file found...");
                let text = String::from_utf8_lossy(&b).replace("\r\n", "\n");
                text.split_inclusive('\n').map(str::to_owned).collect()
            }
            Err(_) => {
                a.log(log::INFO, &format!("No map specific config file found ({})", cfg.display()));
                Default::default()
            }
        };
    });
}

/// `ReadMapConfig`, a line a frame once the map has run 10 s: `rcbot addbot` holds the next
/// line (and new bots) back 5 s. Each line is a server command (`SERVER_COMMAND`).
fn map_config_frame(s: &mut State, e: Engine) {
    let now = e.time();
    if s.map_config.is_empty() || s.map_init_time + 10.0 >= now || s.read_config_time >= now {
        return;
    }
    let Some(mut line) = s.map_config.pop_front() else { return };
    if line.starts_with('#') {
        return;
    }
    if !line.ends_with('\n') {
        line.push('\n');
    }
    if line.starts_with("rcbot addbot") {
        s.rejoin_time = now + 5.0;
        s.read_config_time = now + 5.0;
    }
    e.server_command(&line);
}

/// `StartFrame`'s rejoin part: with `max_bots` set, RCBot adds bots while there are fewer
/// clients; with `min_bots`, while it has fewer bots; with neither, under
/// `bots_leave_and_join`, at random while the server is less than about an eighth full. One
/// bot per 5 s, from 10 s into the map. (Bots of the last map come back through the port,
/// `adopt_bot`.)
fn rejoin_frame(s: &mut State, a: Api, e: Engine) {
    let now = e.time();
    if !s.can_rejoin && s.map_init_time + 10.0 < now && s.rejoin_time < now {
        s.can_rejoin = true;
    }
    let max_clients = e.max_clients().max(1);
    let num_clients = (1..=max_clients)
        .filter(|&i| e.edict(i).is_some_and(|ed| e.vars(ed, |v, _| v.flags & goldsrc_sys::FL_CLIENT as i32 != 0 && v.netname != 0)))
        .count() as i32;
    let mut rng = EngineRandom(e);
    let bot_join = s.config.is_on(rcbot::config::flags::BOTS_LEAVE_AND_JOIN)
        && num_clients as f32 / max_clients as f32 * rcbot::Random::random_float(&mut rng, 4.9, 6.3) < 0.75;
    if num_clients >= max_clients || !s.can_rejoin {
        return;
    }
    let num_bots = s.bots.len() as i32;
    let (min, max) = (i32::from(s.config.min_bots), i32::from(s.config.max_bots));
    let mut add = false;
    if max == -1 && min == -1 {
        add = bot_join;
    }
    if max != -1 && num_clients < max {
        add = true;
    }
    if min != -1 && num_bots < min {
        add = true;
    }
    if add {
        commands::add_bot_now(s, a, e);
        s.can_rejoin = false;
        s.rejoin_time = now + 5.0;
    }
}

/// Saves the bots (brains, learned data) and remembers their profiles for when the port hands
/// them back (`adopt_bot`); then the teams' and loadout GAs.
fn save_all(s: &mut State) {
    for b in s.bots.drain() {
        bots::save_bot(s, &b);
        s.previous_profiles.insert(b.index, b.profile.profile_id);
    }
    bots::save_globals(s);
}

/// `ServerDeactivate`: `CBotGlobals::saveLearnedData`, the bots saved for the next map,
/// `FreeLocalMemory` (squads, the camera).
extern "C" fn server_deactivate() {
    with_state(None, |s, _, _| {
        save_all(s);
        s.squads.clear();
        s.botcam.clear();
        s.editor = editor::Editor::default();
    });
}

/// `GameShutdown`: saves as at a map's end (upstream relies on `ServerDeactivate` coming first).
extern "C" fn shutdown() {
    with_state(None, |s, _, _| save_all(s));
}

/// `StartFrame`: each bot thinks and moves; the waypoint editor draws.
extern "C" fn start_frame() {
    with_state(None, |s, a, e| {
        // `m_iTeamScores`, which upstream keeps from the `TeamScore` messages.
        s.team_scores = world::refresh_game_state(a).team_scores;
        editor::frame(s, a, e);
        bots::force_grip_frame(s, a, e);
        map_config_frame(s, e);
        if !s.active {
            return;
        }
        rejoin_frame(s, a, e);
        let mut rng = EngineRandom(e);
        bots::touches(s, a, e);
        bots::take_fixed_angles(s, e);
        // Every bot thinks before any acts (the actions and usercmds run below), so they can
        // share what they read of the entities this frame.
        let w = HostWorld::cached(a, e);
        let reps = s.rep_ids();
        let mut g = globals!(s, &reps);
        let frames = s.bots.frame(&mut g, &w, &mut rng);
        bots::flush_squad_messages(s, e);
        bots::botcam_frame(s, a, e);
        for (idx, f) in frames {
            let i = idx as i32;
            for act in f.actions {
                match act {
                    Action::Command(line) => {
                        a.bot_command(i, &line);
                    }
                    Action::SelectWeapon(id) => {
                        let rc = a.select_weapon(i, id);
                        if rc == OK || rc == ts_plugin_api::DEFERRED {
                            a.log(log::INFO, &format!("weapon \"{}\" switched to {id}", s.bots.get(idx).map_or("", |b| b.name.as_str())));
                            s.check("weapon_switch");
                        } else if rc == INVALID
                            && let Some(b) = s.bots.get_mut(idx)
                        {
                            // Not owned: the bot forgets it and reads its inventory again.
                            b.weapon_select_failed(id);
                        }
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
    let mut switched = false;
    let mut turned = false;
    for b in s.bots.iter() {
        let Some((origin, yaw)) = e.edict(b.index as i32).map(|ed| e.vars(ed, |v, _| (Vector::from(v.origin), v.v_angle[1]))) else {
            continue;
        };
        let alive = b.is_alive();
        let st = s.stats.entry(b.index).or_default();
        if !alive {
            st.start = None;
            st.weapon = None;
            st.start_yaw = None;
            continue;
        }
        // The view as the engine has it (`pev->v_angle`), not as the bot wants it.
        let start_yaw = *st.start_yaw.get_or_insert(yaw);
        let d = (yaw - start_yaw).rem_euclid(360.0);
        st.turned = st.turned.max(d.min(360.0 - d));
        turned |= st.turned > 30.0;
        // A weapon change while alive: RCBot's `select_weapon` (an empty gun in a fight), or TS
        // switching after a pickup or a drop.
        if let Some(cur) = b.current_weapon() {
            if st.weapon.is_some_and(|w| w != cur) {
                a.log(log::INFO, &format!("weapon \"{}\" changed {} -> {cur}", b.name, st.weapon.unwrap_or(0)));
                switched = true;
            }
            st.weapon = Some(cur);
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
    if switched {
        s.check("weapon_switch");
    }
    if turned {
        s.check("turned");
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

extern "C" fn client_put_in_server(player: c_int, is_bot: c_int) {
    with_state(Some(Event::PutInServer(player, is_bot != 0)), |s, _, e| bots::put_in_server(s, e, player, is_bot != 0));
}

extern "C" fn client_disconnect(player: c_int, _is_bot: c_int) {
    with_state(Some(Event::Disconnect(player)), |s, a, e| bots::disconnect(s, a, e, player));
}

extern "C" fn player_killed(victim: c_int, attacker: c_int, _inflictor: c_int) {
    with_state(Some(Event::Killed(victim, attacker)), |s, a, e| bots::killed(s, a, e, victim, attacker));
}

/// `pfnEmitSound` for the bots' hearing. Sounds no bot reacts to stop here, before the state is
/// locked or anything copied (most of a game's sounds).
unsafe extern "C" fn sound(entity: c_int, origin: *const f32, sample: *const c_char, volume: f32, _attenuation: f32) {
    if sample.is_null() {
        return;
    }
    // SAFETY: the server's string for the call.
    let sample = unsafe { CStr::from_ptr(sample) }.to_string_lossy();
    if !rcbot::bot::SoundKind::of_sample(&sample).is_heard() {
        return;
    }
    let sample = sample.into_owned();
    // SAFETY: a non-NULL origin is the server's 3 floats for the call.
    let origin = (!origin.is_null()).then(|| Vector::from(unsafe { *origin.cast::<[f32; 3]>() }));
    with_state(Some(Event::Sound(entity, origin, volume, sample.clone())), |s, a, e| bots::sound(s, a, e, entity, origin, volume, &sample));
}

/// `ClientCommand`'s `say`/`say_team` (bots' own, from inside `bot_command`, are queued).
pub(crate) fn player_say(player: i32, args: Vec<String>) {
    with_state(Some(Event::Say(player, args.clone())), |s, a, e| bots::say_command(s, a, e, player, &args));
}

/// `BotEvent(BOT_EVENT_HURT)`: the bot looks toward who hurt it; then every field of the
/// damage goes to the bots' combat ledgers (design §4).
#[allow(clippy::too_many_arguments)] // the callback's fields
fn damaged(s: &mut State, a: Api, e: Engine, victim: i32, attacker: i32, inflictor: i32, damage: f32, bits: i32, health: f32) {
    let from = e.edict(attacker).map(|ed| e.vars(ed, |v, _| Vector::from(v.origin)));
    if let (Some(b), Some(from)) = (s.bots.get_mut(victim as u32), from) {
        b.hurt(from, e.time());
    }
    let ev = rcbot::learn::CombatEvent::Damage(rcbot::learn::DamageEvent {
        victim: victim as u32,
        attacker: attacker as u32,
        inflictor,
        amount: damage,
        bits,
        health,
    });
    bots::combat(s, a, e, &ev);
}

extern "C" fn player_damaged(victim: c_int, attacker: c_int, inflictor: c_int, damage: f32, bits: c_int, health: f32) {
    if attacker <= 0 || attacker == victim || victim <= 0 {
        return;
    }
    with_state(Some(Event::Damaged(victim, attacker, inflictor, damage, bits, health)), |s, a, e| {
        damaged(s, a, e, victim, attacker, inflictor, damage, bits, health)
    });
}

/// `kill_scored`: the death as the rules scored it (killer, weapon, headshot), for the bots'
/// ledgers. `player_killed` still drives upstream's death events.
unsafe extern "C" fn kill_scored(kill: *const KillInfo) {
    if kill.is_null() {
        return;
    }
    // SAFETY: a non-NULL `kill` is valid for the call; it is copied out at once.
    let k = unsafe { *kill };
    with_state(Some(Event::KillScored(k)), |s, a, e| bots::kill_scored(s, a, e, &k));
}

/// `player_stunt`: a player started a stunt (for the bots' opponent model).
extern "C" fn player_stunt(player: c_int, _stunt: c_int) {
    if player <= 0 {
        return;
    }
    with_state(Some(Event::Stunt(player)), |s, a, e| bots::stunt(s, a, e, player));
}

// ---- the bot backend -------------------------------------------------------------------------------

/// The rest of `BotFunc_AddBot` once the port asks for a bot: a profile from
/// `bot_profiles.ini`, the fake client, the `CBot` with its brain, learned data and
/// reputations, the greeting.
pub(crate) fn create(s: &mut State, a: Api, e: Engine, name: &CStr, model: *const c_char, skill: f32) -> c_int {
    let mut profile = bots::profile_for(s, e, None);
    if skill > 0.0 {
        profile.skill = if skill <= 1.0 { (skill * 100.0) as i32 } else { skill as i32 };
    }
    s.added += 1;
    // A given name goes to `create_bot` as the server's bytes (a CP1252 byte stays one, as
    // RCBot's `strcpy`); the bot's own copy (chat, HAL, logs) is its lossy text.
    let cname = if name.is_empty() {
        let n = profile.bot_name.clone().unwrap_or_else(|| rcbot::config::profile::BOT_DEFAULT_NAME.into());
        CString::new(n.replace('\0', " ")).unwrap_or_default()
    } else {
        name.to_owned()
    };
    let name = cname.to_string_lossy().into_owned();
    let idx = a.create_bot(&cname, model, profile.skill as f32 / 100.0);
    if idx <= 0 {
        a.log(log::WARNING, &format!("create_bot \"{name}\": {idx}"));
        return idx;
    }
    let profile_id = profile.profile_id;
    let bot = bots::new_bot(s, e, idx as u32, name.clone(), profile);
    s.bots.add(bot);
    a.log(log::INFO, &format!("added \"{name}\" (#{idx}, profile {profile_id})"));
    s.check("create_bot");
    bots::greet(s, a, e, idx as u32);
    idx
}

/// `addbot`, `addcustombot`, the quota.
unsafe extern "C" fn add_bot(name: *const c_char, model: *const c_char, skill: f32) -> c_int {
    // SAFETY: the server passes NUL-terminated strings for the call.
    let name = if name.is_null() { c"" } else { unsafe { CStr::from_ptr(name) } };
    with_state(None, |s, a, e| create(s, a, e, name, model, skill)).unwrap_or(0)
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
        save_all(s);
        s.previous_profiles.clear();
        a.log(log::INFO, "bot backend inactive");
    });
}

/// A quota bot the engine kept across the level change: a new `CBot` for it, with the profile
/// it had.
extern "C" fn adopt_bot(idx: c_int) {
    with_state(None, |s, a, e| {
        let name = e.edict(idx).map(|ed| e.vars(ed, |v, _| v.netname)).map(|n| e.string(n)).unwrap_or_default();
        let again = s.previous_profiles.remove(&(idx as u32));
        let profile = bots::profile_for(s, e, again);
        let bot = bots::new_bot(s, e, idx as u32, name.clone(), profile);
        s.bots.add(bot);
        a.log(log::INFO, &format!("adopted \"{name}\" (#{idx})"));
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
