//! Safe wrappers over the port's plugin API v3 (`ts_plugin_api::PluginApi`) and the engine's
//! function table it hands out. Every `unsafe` call here passes valid pointers for the call's
//! duration; the tables themselves are the server's statics and live for the process.

use goldsrc_sys::{TraceResult, edict_t, enginefuncs_t, globalvars_t};
use rcbot::Vector;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use ts_plugin_api::{BotBackend, BotCmd, GameState, OK, PlayerInfo, PluginApi, PluginId};

/// The plugin API table and our plugin id.
#[derive(Clone, Copy)]
pub struct Api {
    table: &'static PluginApi,
    /// `LoadInfo::id`.
    pub id: PluginId,
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_default()
}

/// A string the API fills (`buf`, `len`): cut to fit, so ask with a big buffer.
fn read_into(f: impl FnOnce(*mut c_char, c_int) -> c_int) -> Option<String> {
    let mut buf = vec![0 as c_char; 1024];
    let n = f(buf.as_mut_ptr(), buf.len() as c_int);
    if n < 0 {
        return None;
    }
    // SAFETY: the API wrote a NUL-terminated string into `buf` (it always terminates).
    Some(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
}

impl Api {
    /// Wraps the table the server passed to `TSRS_PluginLoad`.
    pub fn new(table: &'static PluginApi, id: PluginId) -> Api {
        Api { table, id }
    }

    /// The engine's table and globals.
    pub fn engine(&self) -> Option<Engine> {
        let f = self.table.engine_funcs.cast::<enginefuncs_t>();
        let g = self.table.globals.cast::<globalvars_t>();
        if f.is_null() || g.is_null() {
            return None;
        }
        // SAFETY: the API hands out the engine's own table and globals, which live for the
        // process; the table is only read.
        Some(Engine { f: unsafe { &*f }, g })
    }

    /// `log`.
    pub fn log(&self, level: c_int, msg: &str) {
        let m = cstring(msg);
        // SAFETY: `m` is NUL-terminated for the call.
        unsafe { (self.table.log)(self.id, level, m.as_ptr()) };
    }

    /// `data_dir`: `<gamedir>/tsmod/rcbot/` with a trailing separator.
    pub fn data_dir(&self) -> Option<String> {
        // SAFETY: `read_into` passes a writable buffer of the given length.
        read_into(|b, l| unsafe { (self.table.data_dir)(self.id, b, l) })
    }

    /// `register_command`.
    pub fn register_command(&self, name: &str, f: ts_plugin_api::CommandFn) -> c_int {
        let n = cstring(name);
        // SAFETY: the name is read during the call; `f` lives for the process.
        unsafe { (self.table.register_command)(self.id, n.as_ptr(), f) }
    }

    /// `register_cvar`: the engine's `cvar_t`, or NULL.
    pub fn register_cvar(&self, name: &str, value: &str) -> *mut c_void {
        let (n, v) = (cstring(name), cstring(value));
        // SAFETY: both strings are read during the call.
        unsafe { (self.table.register_cvar)(self.id, n.as_ptr(), v.as_ptr(), 0) }
    }

    /// `register_bot_backend` (the server copies the descriptor).
    pub fn register_bot_backend(&self, name: &str, backend: &BotBackend) -> c_int {
        let n = cstring(name);
        // SAFETY: the name and descriptor are read during the call.
        unsafe { (self.table.register_bot_backend)(self.id, n.as_ptr(), backend) }
    }

    /// `player_info`.
    pub fn player_info(&self, player: i32) -> Option<PlayerInfo> {
        let mut i = PlayerInfo::default();
        // SAFETY: `i` is writable.
        (unsafe { (self.table.player_info)(player, &mut i) } == OK).then_some(i)
    }

    /// `game_state`.
    pub fn game_state(&self) -> GameState {
        let mut g = GameState::default();
        // SAFETY: `g` is writable.
        unsafe { (self.table.game_state)(&mut g) };
        g
    }

    /// `create_bot`: the index, or a status below 1.
    pub fn create_bot(&self, name: &str, model: *const c_char, skill: f32) -> c_int {
        let n = cstring(name);
        // SAFETY: the name is read during the call; `model` is the server's string for the
        // `add_bot` call we are in (or NULL-free empty).
        unsafe { (self.table.create_bot)(self.id, n.as_ptr(), model, skill) }
    }

    /// `kick_bot`.
    pub fn kick_bot(&self, player: i32) -> c_int {
        (self.table.kick_bot)(player)
    }

    /// `run_cmd`.
    pub fn run_cmd(&self, player: i32, cmd: &BotCmd) -> c_int {
        // SAFETY: `cmd` is read during the call.
        unsafe { (self.table.run_cmd)(player, cmd) }
    }

    /// `bot_command`.
    pub fn bot_command(&self, player: i32, line: &str) -> c_int {
        let l = cstring(line);
        // SAFETY: the line is read during the call.
        unsafe { (self.table.bot_command)(player, l.as_ptr()) }
    }

    /// `select_weapon`.
    pub fn select_weapon(&self, player: i32, weapon: i32) -> c_int {
        (self.table.select_weapon)(player, weapon)
    }

    /// `buy`.
    pub fn buy(&self, player: i32, weapon: i32, ammo: i32) -> c_int {
        (self.table.buy)(player, weapon, 0, 0, ammo)
    }

    /// `clear_loadout`.
    pub fn clear_loadout(&self, player: i32) -> c_int {
        (self.table.clear_loadout)(player)
    }
}

/// The engine's function table and globals.
#[derive(Clone, Copy)]
pub struct Engine {
    f: &'static enginefuncs_t,
    g: *mut globalvars_t,
}

/// An edict the engine handed out, valid while the engine keeps it (this frame).
#[derive(Clone, Copy)]
pub struct Edict(pub *mut edict_t);

impl Engine {
    fn globals(&self) -> &globalvars_t {
        // SAFETY: the engine's globals live for the process; read on the engine's thread.
        unsafe { &*self.g }
    }

    /// `gpGlobals->time`.
    pub fn time(&self) -> f32 {
        self.globals().time
    }

    /// `gpGlobals->maxClients`.
    pub fn max_clients(&self) -> i32 {
        self.globals().maxClients
    }

    /// `gpGlobals->maxEntities`.
    pub fn max_entities(&self) -> i32 {
        self.globals().maxEntities
    }

    /// `STRING(s)`.
    pub fn string(&self, s: c_int) -> String {
        let Some(f) = self.f.pfnSzFromIndex else { return String::new() };
        if s == 0 {
            return String::new();
        }
        // SAFETY: `pfnSzFromIndex` maps any string_t the engine handed out to a NUL-terminated
        // string in its string pool.
        let p = unsafe { f(s) };
        if p.is_null() {
            return String::new();
        }
        // SAFETY: as above.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    /// `INDEXENT(i)`, `None` for a missing or free edict.
    pub fn edict(&self, index: i32) -> Option<Edict> {
        let f = self.f.pfnPEntityOfEntIndex?;
        // SAFETY: any index is accepted; the engine returns NULL or one of its edicts.
        let e = unsafe { f(index) };
        // SAFETY: a non-NULL edict is the engine's, readable this frame.
        if e.is_null() || unsafe { (*e).free } != 0 {
            return None;
        }
        Some(Edict(e))
    }

    /// `ENTINDEX(e)`.
    pub fn index_of(&self, e: *const edict_t) -> i32 {
        match self.f.pfnIndexOfEdict {
            // SAFETY: `e` is an edict the engine handed out.
            Some(f) if !e.is_null() => unsafe { f(e) },
            _ => -1,
        }
    }

    /// Reads an edict's `entvars_t`.
    pub fn vars<R>(&self, e: Edict, read: impl FnOnce(&goldsrc_sys::entvars_t, i32) -> R) -> R {
        // SAFETY: `e` came from `edict` this frame; the engine owns it and we only read it,
        // without calling back into the engine while borrowing.
        let ed = unsafe { &*e.0 };
        read(&ed.v, ed.serialnumber)
    }

    /// Changes an edict's `entvars_t` (the `util` commands' toggles).
    pub fn vars_mut<R>(&self, e: Edict, write: impl FnOnce(&mut goldsrc_sys::entvars_t) -> R) -> R {
        // SAFETY: as `vars`; the engine thread owns the edict and nothing else holds a borrow.
        let ed = unsafe { &mut *e.0 };
        write(&mut ed.v)
    }

    /// `UTIL_TraceLine`: `ignore_monsters` 1, `ignore_glass` 0x100.
    pub fn trace_line(
        &self,
        start: Vector,
        end: Vector,
        no_monsters: bool,
        ignore_glass: bool,
        ignore: Option<i32>,
    ) -> (f32, Vector, *mut edict_t) {
        let Some(f) = self.f.pfnTraceLine else { return (1.0, end, std::ptr::null_mut()) };
        let skip = ignore.and_then(|i| self.edict(i)).map_or(std::ptr::null_mut(), |e| e.0);
        let mut tr = TraceResult::default();
        let (s, e) = (start.to_array(), end.to_array());
        let flags = c_int::from(no_monsters) | if ignore_glass { 0x100 } else { 0 };
        // SAFETY: the vectors and the result are valid for the call; `skip` is NULL or an edict.
        unsafe { f(s.as_ptr(), e.as_ptr(), flags, skip, &mut tr) };
        (tr.flFraction, Vector::from(tr.vecEndPos), tr.pHit)
    }

    /// `POINT_CONTENTS`.
    pub fn point_contents(&self, at: Vector) -> i32 {
        let Some(f) = self.f.pfnPointContents else { return -1 };
        let a = at.to_array();
        // SAFETY: `a` is valid for the call.
        unsafe { f(a.as_ptr()) }
    }

    /// `ENGINE_CHECK_VISIBILITY(viewer, ENGINE_SET_PVS(origin))`.
    pub fn in_pvs(&self, viewer: i32, origin: Vector) -> bool {
        let (Some(set), Some(check), Some(e)) = (self.f.pfnSetFatPVS, self.f.pfnCheckVisibility, self.edict(viewer)) else {
            return true;
        };
        let o = origin.to_array();
        // SAFETY: the origin is valid for the call; the engine returns its own PVS buffer,
        // which `pfnCheckVisibility` reads right away.
        unsafe {
            let pvs = set(o.as_ptr());
            check(e.0, pvs) != 0
        }
    }

    /// `UTIL_FindEntityInSphere`, all of them.
    pub fn entities_in_sphere(&self, origin: Vector, radius: f32) -> Vec<i32> {
        let Some(f) = self.f.pfnFindEntityInSphere else { return Vec::new() };
        let o = origin.to_array();
        let mut out = Vec::new();
        let mut e: *mut edict_t = std::ptr::null_mut();
        loop {
            // SAFETY: `e` is NULL or the engine's last result; the origin is valid.
            e = unsafe { f(e, o.as_ptr(), radius) };
            // SAFETY: a non-NULL result is one of the engine's edicts.
            if e.is_null() || unsafe { (*e).free } != 0 {
                break;
            }
            out.push(self.index_of(e));
            if out.len() > 4096 {
                break;
            }
        }
        out
    }

    /// `RANDOM_LONG`.
    pub fn random_long(&self, lo: i32, hi: i32) -> i32 {
        // SAFETY: plain values.
        self.f.pfnRandomLong.map_or(lo, |f| unsafe { f(lo, hi) })
    }

    /// `RANDOM_FLOAT`.
    pub fn random_float(&self, lo: f32, hi: f32) -> f32 {
        // SAFETY: plain values.
        self.f.pfnRandomFloat.map_or(lo, |f| unsafe { f(lo, hi) })
    }

    /// `SERVER_PRINT`.
    pub fn server_print(&self, text: &str) {
        let Some(f) = self.f.pfnServerPrint else { return };
        let t = cstring(text);
        // SAFETY: the text is read during the call.
        unsafe { f(t.as_ptr()) };
    }

    /// `CLIENT_PRINTF(e, print_console, text)`.
    pub fn client_print(&self, player: i32, text: &str) {
        let (Some(f), Some(e)) = (self.f.pfnClientPrintf, self.edict(player)) else { return };
        let t = cstring(text);
        // SAFETY: `e` is a live edict, the text is read during the call.
        unsafe { f(e.0, goldsrc_sys::PRINT_TYPE::print_console, t.as_ptr()) };
    }

    /// `UTIL_PlaySound`: `CLIENT_COMMAND(player, "play <sample>\n")`, not to bots.
    pub fn client_command_sound(&self, player: i32, sample: &str) {
        let (Some(f), Some(e)) = (self.f.pfnClientCommand, self.edict(player)) else { return };
        if self.vars(e, |v, _| v.flags) & goldsrc_sys::FL_FAKECLIENT as i32 != 0 {
            return;
        }
        let line = cstring(&format!("play {sample}\n"));
        // SAFETY: a live client edict; the format is "%s" and the one argument a NUL-terminated
        // string, both valid for the call.
        unsafe { f(e.0, c"%s".as_ptr().cast_mut(), line.as_ptr()) };
    }

    /// `SET_ORIGIN`.
    pub fn set_origin(&self, player: i32, to: Vector) {
        let (Some(f), Some(e)) = (self.f.pfnSetOrigin, self.edict(player)) else { return };
        let o = to.to_array();
        // SAFETY: as above.
        unsafe { f(e.0, o.as_ptr()) };
    }

    /// `CVAR_GET_STRING`.
    pub fn cvar_string(&self, name: &str) -> String {
        let Some(f) = self.f.pfnCVarGetString else { return String::new() };
        let n = cstring(name);
        // SAFETY: the name is read during the call; the engine returns its own string.
        let p = unsafe { f(n.as_ptr()) };
        if p.is_null() {
            return String::new();
        }
        // SAFETY: a NUL-terminated engine string.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    /// `IS_DEDICATED_SERVER`.
    pub fn is_dedicated(&self) -> bool {
        // SAFETY: no arguments.
        self.f.pfnIsDedicatedServer.is_some_and(|f| unsafe { f() } != 0)
    }

    /// `GETPLAYERUSERID`.
    pub fn user_id(&self, player: i32) -> i32 {
        let (Some(f), Some(e)) = (self.f.pfnGetPlayerUserId, self.edict(player)) else { return -1 };
        // SAFETY: a live edict.
        unsafe { f(e.0) }
    }

    /// `PRECACHE_MODEL`, the model index.
    pub fn precache_model(&self, name: &'static CStr) -> i32 {
        // SAFETY: a static string; the engine keeps the pointer.
        self.f.pfnPrecacheModel.map_or(0, |f| unsafe { f(name.as_ptr()) })
    }

    /// `WaypointDrawBeam`: `TE_BEAMPOINTS` to one player, unreliable (upstream's fix against
    /// overflows).
    #[allow(clippy::too_many_arguments)] // upstream's arguments
    pub fn beam(
        &self,
        to: i32,
        start: Vector,
        end: Vector,
        sprite: i32,
        width: i32,
        noise: i32,
        rgb: [u8; 3],
        brightness: i32,
        speed: i32,
    ) {
        let f = self.f;
        let (Some(begin), Some(byte), Some(short), Some(coord), Some(endm), Some(e)) =
            (f.pfnMessageBegin, f.pfnWriteByte, f.pfnWriteShort, f.pfnWriteCoord, f.pfnMessageEnd, self.edict(to))
        else {
            return;
        };
        const MSG_ONE_UNRELIABLE: c_int = 8;
        const SVC_TEMPENTITY: c_int = 23;
        const TE_BEAMPOINTS: c_int = 0;
        // SAFETY: a well-formed temp-entity message to a live client, on the engine's thread.
        unsafe {
            begin(MSG_ONE_UNRELIABLE, SVC_TEMPENTITY, std::ptr::null(), e.0);
            byte(TE_BEAMPOINTS);
            for v in [start, end] {
                coord(v.x);
                coord(v.y);
                coord(v.z);
            }
            short(sprite);
            byte(1); // framestart
            byte(10); // framerate
            byte(10); // life in 0.1 s
            byte(width);
            byte(noise);
            byte(c_int::from(rgb[0]));
            byte(c_int::from(rgb[1]));
            byte(c_int::from(rgb[2]));
            byte(brightness);
            byte(speed);
            endm();
        }
    }
}
