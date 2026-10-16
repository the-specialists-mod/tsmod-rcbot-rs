//! Safe wrappers over the port's plugin API v5 (`ts_plugin_api::PluginApi`) and the engine's
//! function table it hands out. Every `unsafe` call here passes valid pointers for the call's
//! duration; the tables themselves are the server's statics and live for the process.

use goldsrc_sys::{TraceResult, edict_t, enginefuncs_t, globalvars_t};
use rcbot::Vector;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use ts_plugin_api::{BotBackend, BotCmd, GameState, HamArg, HamCall, OK, ObjectiveInfo, PlayerInfo, PluginApi, PluginId, WeaponEntry, ham};

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

    /// `player_inventory`: the weapons the player owns (kung fu not included), `None` for a
    /// non-player.
    pub fn player_inventory(&self, player: i32) -> Option<Vec<WeaponEntry>> {
        let mut out = vec![WeaponEntry::default(); 64];
        // SAFETY: `out` is writable for its length.
        let n = unsafe { (self.table.player_inventory)(player, out.as_mut_ptr(), out.len() as c_int) };
        out.truncate(usize::try_from(n).ok()?);
        Some(out)
    }

    /// `game_state`.
    pub fn game_state(&self) -> GameState {
        let mut g = GameState::default();
        // SAFETY: `g` is writable.
        unsafe { (self.table.game_state)(&mut g) };
        g
    }

    /// `objectives(player)`: the HUD's objective list as `player` reads it.
    pub fn objectives(&self, player: i32) -> Vec<ObjectiveInfo> {
        let mut out = vec![ObjectiveInfo::default(); 32];
        // SAFETY: `out` is writable for its length.
        let n = unsafe { (self.table.objectives)(player, out.as_mut_ptr(), out.len() as c_int) };
        out.truncate(usize::try_from(n).unwrap_or(0));
        out
    }

    /// `create_entity(classname)`: the new, not yet spawned entity's index.
    pub fn create_entity(&self, classname: &CStr) -> Option<i32> {
        // SAFETY: a NUL-terminated name read during the call.
        let e = unsafe { (self.table.create_entity)(classname.as_ptr()) };
        (e > 0).then_some(e)
    }

    /// `entity_key_value(entity, key, value)` (`DispatchKeyValue`): 1 if handled.
    pub fn entity_key_value(&self, entity: i32, key: &str, value: &str) -> c_int {
        let (k, v) = (cstring(key), cstring(value));
        // SAFETY: NUL-terminated strings read during the call.
        unsafe { (self.table.entity_key_value)(entity, k.as_ptr(), v.as_ptr()) }
    }

    /// `spawn_entity(entity)` (`DispatchSpawn`): `OK`, or below 0 (the entity is gone).
    pub fn spawn_entity(&self, entity: i32) -> c_int {
        (self.table.spawn_entity)(entity)
    }

    /// `ham_execute` with the hooks (the game's call): `args` are `(kind, int, float)`.
    fn ham_execute(&self, ham_id: c_int, this: i32, args: &[(c_int, c_int, f32)], ret: c_int) -> (c_int, HamArg) {
        let mut call = HamCall { ham: ham_id, this, argc: args.len() as c_int, ..Default::default() };
        for (slot, &(kind, i, f)) in call.args.iter_mut().zip(args) {
            *slot = HamArg { kind, i, f, ..Default::default() };
        }
        call.ret = HamArg { kind: ret, ..Default::default() };
        // SAFETY: a fully initialised call valid for reads and writes during it.
        let rc = unsafe { (self.table.ham_execute)(&mut call, 1) };
        (rc, call.ret)
    }

    /// `DispatchUse(entity, other)`: `Use(other, other, USE_TOGGLE, 0)`.
    pub fn use_entity(&self, entity: i32, other: i32) -> c_int {
        use ham::kind::{ENTITY, FLOAT, INT, VOID};
        const HAM_USE: c_int = 43;
        self.ham_execute(HAM_USE, entity, &[(ENTITY, other, 0.0), (ENTITY, other, 0.0), (INT, 3, 0.0), (FLOAT, 0, 0.0)], VOID).0
    }

    /// `pEntity->TakeDamage(pevInflictor, pevAttacker, flDamage, bitsDamageType)`.
    pub fn take_damage(&self, victim: i32, inflictor: i32, attacker: i32, damage: f32, bits: c_int) -> c_int {
        use ham::kind::{ENTITY, FLOAT, INT};
        const HAM_TAKEDAMAGE: c_int = 9;
        let args = [(ENTITY, inflictor, 0.0), (ENTITY, attacker, 0.0), (FLOAT, 0, damage), (INT, bits, 0.0)];
        self.ham_execute(HAM_TAKEDAMAGE, victim, &args, INT).0
    }

    /// `create_bot`: the index, or a status below 1. `name` is bytes (the API's strings aren't
    /// UTF-8: a name typed with `addcustombot` reaches the netname as typed).
    pub fn create_bot(&self, name: &CStr, model: *const c_char, skill: f32) -> c_int {
        // SAFETY: the name is read during the call; `model` is the server's string for the
        // `add_bot` call we are in (or NULL-free empty).
        unsafe { (self.table.create_bot)(self.id, name.as_ptr(), model, skill) }
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

    /// `gpGlobals->frametime`.
    pub fn frame_time(&self) -> f32 {
        self.globals().frametime
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

    /// `INFOKEY_VALUE(GET_INFOKEYBUFFER(e), key)`: a client's user info key ("" if unset).
    pub fn info_key(&self, e: Edict, key: &CStr) -> String {
        let (Some(buf), Some(val)) = (self.f.pfnGetInfoKeyBuffer, self.f.pfnInfoKeyValue) else { return String::new() };
        // SAFETY: `e` is a live edict (see `edict`); the engine returns its info buffer or NULL.
        let b = unsafe { buf(e.0) };
        if b.is_null() {
            return String::new();
        }
        // SAFETY: a buffer from `pfnGetInfoKeyBuffer` and a NUL-terminated key; the engine
        // returns a NUL-terminated string (static, possibly empty).
        let v = unsafe { val(b, key.as_ptr()) };
        if v.is_null() {
            return String::new();
        }
        // SAFETY: as above.
        unsafe { CStr::from_ptr(v) }.to_string_lossy().into_owned()
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

    /// [`trace_line`](Self::trace_line) that also says whether it started in a solid
    /// (`fStartSolid`).
    pub fn trace_line_solid(
        &self,
        start: Vector,
        end: Vector,
        no_monsters: bool,
        ignore: Option<i32>,
    ) -> (f32, Vector, *mut edict_t, bool) {
        let Some(f) = self.f.pfnTraceLine else { return (1.0, end, std::ptr::null_mut(), false) };
        let skip = ignore.and_then(|i| self.edict(i)).map_or(std::ptr::null_mut(), |e| e.0);
        let mut tr = TraceResult::default();
        let (s, e) = (start.to_array(), end.to_array());
        // SAFETY: the vectors and the result are valid for the call; `skip` is NULL or an edict.
        unsafe { f(s.as_ptr(), e.as_ptr(), c_int::from(no_monsters), skip, &mut tr) };
        (tr.flFraction, Vector::from(tr.vecEndPos), tr.pHit, tr.fStartSolid != 0)
    }

    /// `TE_EXPLOSION` at `center` to the PAS (`ExplosionCreate`'s message): the fireball sprite,
    /// `scale` in 0.1s (at most 255), framerate 15, no flags.
    pub fn temp_explosion(&self, center: Vector, sprite: i32, scale: i32) {
        let f = self.f;
        let (Some(begin), Some(byte), Some(short), Some(coord), Some(endm)) =
            (f.pfnMessageBegin, f.pfnWriteByte, f.pfnWriteShort, f.pfnWriteCoord, f.pfnMessageEnd)
        else {
            return;
        };
        const MSG_PAS: c_int = 4;
        const SVC_TEMPENTITY: c_int = 23;
        const TE_EXPLOSION: c_int = 3;
        let o = center.to_array();
        // SAFETY: a well-formed temp-entity message to the PAS of `o`, on the engine's thread.
        unsafe {
            begin(MSG_PAS, SVC_TEMPENTITY, o.as_ptr(), std::ptr::null_mut());
            byte(TE_EXPLOSION);
            coord(center.x);
            coord(center.y);
            coord(center.z);
            short(sprite);
            byte(scale.min(255));
            byte(15);
            byte(0);
            endm();
        }
    }

    /// `pfnTraceHull(start, end, no_monsters, hull, ignore)`.
    pub fn trace_hull(&self, start: Vector, end: Vector, no_monsters: bool, hull: i32, ignore: Option<i32>) -> (f32, Vector, *mut edict_t) {
        let Some(f) = self.f.pfnTraceHull else { return (1.0, end, std::ptr::null_mut()) };
        let skip = ignore.and_then(|i| self.edict(i)).map_or(std::ptr::null_mut(), |e| e.0);
        let mut tr = TraceResult::default();
        let (s, e) = (start.to_array(), end.to_array());
        // SAFETY: the vectors and the result are valid for the call; `skip` is NULL or an edict.
        unsafe { f(s.as_ptr(), e.as_ptr(), c_int::from(no_monsters), hull, skip, &mut tr) };
        (tr.flFraction, Vector::from(tr.vecEndPos), tr.pHit)
    }

    /// `UTIL_FindEntityByString` from the start of the list to the end: every match, once.
    pub fn find_entities(&self, field: &str, value: &str) -> Vec<i32> {
        let Some(f) = self.f.pfnFindEntityByString else { return Vec::new() };
        let (k, v) = (cstring(field), cstring(value));
        let mut e: *mut edict_t = std::ptr::null_mut();
        collect_search(|| {
            // SAFETY: `e` is NULL or the engine's last result; both strings live for the call.
            e = unsafe { f(e, k.as_ptr(), v.as_ptr()) };
            self.search_result(e)
        })
    }

    /// An engine search's result as an index: `None` for NULL or a free edict.
    fn search_result(&self, e: *mut edict_t) -> Option<i32> {
        // SAFETY: a non-NULL result is one of the engine's edicts.
        if e.is_null() || unsafe { (*e).free } != 0 {
            return None;
        }
        Some(self.index_of(e))
    }

    /// `CVAR_GET_FLOAT`.
    pub fn cvar_float(&self, name: &str) -> f32 {
        let Some(f) = self.f.pfnCVarGetFloat else { return 0.0 };
        let n = cstring(name);
        // SAFETY: the name is read during the call.
        unsafe { f(n.as_ptr()) }
    }

    /// `CREATE_NAMED_ENTITY(MAKE_STRING(classname))`: the new entity's index.
    pub fn create_named_entity(&self, classname: &'static CStr) -> Option<i32> {
        let (Some(alloc), Some(create)) = (self.f.pfnAllocString, self.f.pfnCreateNamedEntity) else { return None };
        // SAFETY: a static string the engine copies into its pool.
        let s = unsafe { alloc(classname.as_ptr()) };
        // SAFETY: a string_t the engine just made.
        let e = unsafe { create(s) };
        (!e.is_null()).then(|| self.index_of(e))
    }

    /// `MAKE_STRING`.
    pub fn alloc_string(&self, s: &'static CStr) -> c_int {
        // SAFETY: a static string the engine copies into its pool.
        self.f.pfnAllocString.map_or(0, |f| unsafe { f(s.as_ptr()) })
    }

    /// `SET_MODEL`.
    pub fn set_model(&self, entity: i32, model: &'static CStr) {
        let (Some(f), Some(e)) = (self.f.pfnSetModel, self.edict(entity)) else { return };
        // SAFETY: a live edict and a precached model's static name.
        unsafe { f(e.0, model.as_ptr()) };
    }

    /// `SET_VIEW(player, view)`.
    pub fn set_view(&self, player: i32, view: i32) {
        let (Some(f), Some(p), Some(v)) = (self.f.pfnSetView, self.edict(player), self.edict(view)) else { return };
        // SAFETY: two live edicts.
        unsafe { f(p.0, v.0) };
    }

    /// `UTIL_ShowMenu(player, slots, displaytime, needmore, text)`: the game's `ShowMenu`
    /// message (`pfnRegUserMsg` gives the id the game registered).
    pub fn show_menu(&self, player: i32, slots: i32, display_time: i32, need_more: bool, text: &str) {
        let f = self.f;
        let (Some(reg), Some(begin), Some(short), Some(chr), Some(byte), Some(string), Some(endm), Some(e)) = (
            f.pfnRegUserMsg,
            f.pfnMessageBegin,
            f.pfnWriteShort,
            f.pfnWriteChar,
            f.pfnWriteByte,
            f.pfnWriteString,
            f.pfnMessageEnd,
            self.edict(player),
        ) else {
            return;
        };
        // SAFETY: a static name; an existing message's id comes back.
        let id = unsafe { reg(c"ShowMenu".as_ptr(), -1) };
        if id <= 0 {
            return;
        }
        let t = cstring(text);
        const MSG_ONE: c_int = 1;
        // SAFETY: a well-formed user message to a live client, on the engine's thread.
        unsafe {
            begin(MSG_ONE, id, std::ptr::null(), e.0);
            short(slots);
            chr(display_time);
            byte(c_int::from(need_more));
            string(t.as_ptr());
            endm();
        }
    }

    /// `UTIL_BotHudMessage` (the SDK's `UTIL_HudMessage`): a `TE_TEXTMESSAGE` to one client, not
    /// to bots. `fade`, `hold` and `fx` are in seconds; `effect` 0 fades in and out.
    #[allow(clippy::too_many_arguments)] // hudtextparms_t's fields
    pub fn hud_message(
        &self,
        to: i32,
        channel: i32,
        pos: (f32, f32),
        effect: i32,
        rgba1: [u8; 4],
        rgba2: [u8; 4],
        fade: (f32, f32, f32, f32),
        text: &str,
    ) {
        let f = self.f;
        let (Some(begin), Some(byte), Some(short), Some(string), Some(endm), Some(e)) =
            (f.pfnMessageBegin, f.pfnWriteByte, f.pfnWriteShort, f.pfnWriteString, f.pfnMessageEnd, self.edict(to))
        else {
            return;
        };
        if self.vars(e, |v, _| v.flags & goldsrc_sys::FL_CLIENT as i32 == 0 || v.flags & goldsrc_sys::FL_FAKECLIENT as i32 != 0) {
            return;
        }
        let signed = |v: f32, scale: f32| ((v * scale) as i32).clamp(-32768, 32767);
        let unsigned = |v: f32, scale: f32| ((v * scale) as i32).clamp(0, 65535);
        let mut t = text.to_owned();
        t.truncate(511);
        let t = cstring(&t);
        const MSG_ONE_UNRELIABLE: c_int = 8;
        const SVC_TEMPENTITY: c_int = 23;
        const TE_TEXTMESSAGE: c_int = 29;
        let (fade_in, fade_out, hold, fx) = fade;
        // SAFETY: a well-formed temp-entity message to a live client, on the engine's thread.
        unsafe {
            begin(MSG_ONE_UNRELIABLE, SVC_TEMPENTITY, std::ptr::null(), e.0);
            byte(TE_TEXTMESSAGE);
            byte(channel & 0xFF);
            short(signed(pos.0, 8192.0));
            short(signed(pos.1, 8192.0));
            byte(effect);
            for c in rgba1.into_iter().chain(rgba2) {
                byte(c_int::from(c));
            }
            short(unsigned(fade_in, 256.0));
            short(unsigned(fade_out, 256.0));
            short(unsigned(hold, 256.0));
            if effect == 2 {
                short(unsigned(fx, 256.0));
            }
            string(t.as_ptr());
            endm();
        }
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

    /// `UTIL_FindEntityInSphere`, all of them, once each.
    pub fn entities_in_sphere(&self, origin: Vector, radius: f32) -> Vec<i32> {
        let Some(f) = self.f.pfnFindEntityInSphere else { return Vec::new() };
        let o = origin.to_array();
        let mut e: *mut edict_t = std::ptr::null_mut();
        collect_search(|| {
            // SAFETY: `e` is NULL or the engine's last result; the origin is valid.
            e = unsafe { f(e, o.as_ptr(), radius) };
            self.search_result(e)
        })
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

    /// `DROP_TO_FLOOR`.
    pub fn drop_to_floor(&self, entity: i32) {
        let (Some(f), Some(e)) = (self.f.pfnDropToFloor, self.edict(entity)) else { return };
        // SAFETY: a live edict (`edict`); the engine moves it down onto the floor.
        unsafe { f(e.0) };
    }

    /// `SERVER_COMMAND`.
    pub fn server_command(&self, line: &str) {
        let Some(f) = self.f.pfnServerCommand else { return };
        let l = cstring(line);
        // SAFETY: the engine copies the line into its command buffer.
        unsafe { f(l.as_ptr()) };
    }

    /// `CVAR_GET_STRING` as the engine's bytes, unconverted (empty when unset).
    pub fn cvar_bytes(&self, name: &str) -> Vec<u8> {
        let Some(f) = self.f.pfnCVarGetString else { return Vec::new() };
        let n = cstring(name);
        // SAFETY: the name is read during the call; the engine returns its own string.
        let p = unsafe { f(n.as_ptr()) };
        if p.is_null() {
            return Vec::new();
        }
        // SAFETY: a NUL-terminated engine string, copied before the engine can change it.
        unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
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

/// The most results one search collects (`MAX_EDICTS` in FWGS).
const MAX_SEARCH: usize = 4096;

/// Runs an engine search (`pfnFindEntityByString`, `pfnFindEntityInSphere`) to its end: `next`
/// makes the call after the previous result and gives its index (`None` for NULL or a free edict).
///
/// The engines don't answer "no more" with NULL: FWGS (`sv_game.c` `SV_FindEntityByString`,
/// `pfnFindEntityInSphere`) and ReHLDS (`pr_cmds.cpp`) return the world, index 0, and a search
/// started from the world begins again at 1. Stopping only on NULL repeated every match until
/// 4097 results (every bot's touch check, every frame), and a search with no match gave 4097
/// worlds. The search ends at the world, and at an index it already gave, in case an engine
/// loops some other way.
fn collect_search(mut next: impl FnMut() -> Option<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    let mut seen = vec![0u64; MAX_SEARCH / 64 + 1];
    while out.len() < MAX_SEARCH {
        let Some(i) = next().filter(|&i| i > 0) else { break };
        let (word, bit) = (i as usize / 64, 1u64 << (i as usize % 64));
        if word >= seen.len() {
            seen.resize(word + 1, 0);
        }
        if seen[word] & bit != 0 {
            break;
        }
        seen[word] |= bit;
        out.push(i);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::collect_search;

    /// An engine search over `matches` as FWGS runs it: after the last match the world (0), and
    /// from the world the first match again.
    fn engine(matches: &[i32]) -> impl FnMut() -> Option<i32> + '_ {
        let mut prev = 0;
        move || {
            let next = matches.iter().copied().find(|&m| m > prev).unwrap_or(0);
            prev = next;
            Some(next)
        }
    }

    #[test]
    fn a_search_ends_at_the_world() {
        assert_eq!(collect_search(engine(&[3, 7, 40])), vec![3, 7, 40]);
        assert_eq!(collect_search(engine(&[])), Vec::<i32>::new());
        // NULL or a free edict ends it too.
        let mut calls = [Some(5), None, Some(6)].into_iter();
        assert_eq!(collect_search(|| calls.next().flatten()), vec![5]);
    }

    #[test]
    fn a_search_that_loops_ends_at_a_repeat() {
        let mut calls = [2, 9, 2, 9].into_iter().cycle();
        assert_eq!(collect_search(|| calls.next()), vec![2, 9]);
        let mut n = 0;
        let all = collect_search(|| {
            n += 1;
            Some(n)
        });
        assert_eq!(all.len(), 4096);
    }
}
