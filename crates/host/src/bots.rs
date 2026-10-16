//! RCBot's bots on the port: making one (`BotFunc_AddBot`'s profile choice, `BotFunc_ReadProfile`
//! with the HAL brain and the reputations, `setupDataStructures`' learned data, the greeting),
//! saving them (`ClientDisconnect`: `SaveHALBrainForPersonality`, `saveLearnedData`; the map end:
//! `CBotGlobals::saveLearnedData`), players joining and leaving (`CClients::ClientConnected` /
//! `ClientDisconnected`: reputation ids, `AddLoadRep`, `RemoveSaveRep`, squads), deaths, sounds,
//! touches, `say`, and the bot camera.
//!
//! Files live in the plugin's data directory (RCBot's bot folder): `bot_profiles.ini`,
//! `botprofiles/<id>.ini`, `<id>.rld`, `<id>.rkf` (the kung fu fighter), `<id>.rcr`, `<id>_hal.brn` and the keyword lists,
//! `bot_playerids.ini`, `bot_chat.ini`, `team<n>.rld`, `tsweaps.rld`.

use crate::api::{Api, Engine};
use crate::world::{EngineRandom, HostWorld};
use crate::{State, say};
use rcbot::Vector;
use rcbot::bot::{Bot, BotCam, BotWorld, NO_CLIENT};
use rcbot::chat::bot_chat::ChatKind;
use rcbot::chat::brain::{HalFileNames, HalFileOverrides, cookie_is_valid};
use rcbot::chat::reply::HalBot;
use rcbot::config::profile::{ProfileChoice, choose_profile, profile_file_name};
use rcbot::config::rep::{player_rep_id, rep_file_name};
use rcbot::config::{BOT_PLAYER_ID_FILE, BOT_PROFILES_FILE, BOT_PROFILES_FOLDER, BotProfile, PlayerId, flags as cfg};
use rcbot::learn::store::profile_key;
use std::path::PathBuf;
use ts_plugin_api::log;

/// Builds `BotGlobals` from the state's fields (a macro, so the borrows stay disjoint from
/// `s.bots`).
macro_rules! globals {
    ($s:expr, $reps:expr) => {
        rcbot::bot::BotGlobals {
            config: &$s.config,
            waypoints: &mut $s.waypoints,
            weapons: &mut $s.weapons,
            presets: &$s.presets,
            learned: &mut $s.learned,
            // `m_iTeamScores` (the `TeamScore` message), for the death fitness, by rules team.
            team_scores: $s.team_scores,
            masters: &$s.masters,
            other_bots: Vec::new(),
            squads: &mut $s.squads,
            botcam: &mut $s.botcam,
            rep_ids: $reps,
            chat: &$s.chat,
            map_name: &$s.map,
        }
    };
}
pub(crate) use globals;

impl State {
    pub(crate) fn path(&self, rel: &str) -> PathBuf {
        self.data_dir.join(rel)
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(rel)).ok()
    }

    /// Writes a data file atomically: a temporary file beside it, renamed over it, so a crash
    /// or a full disk mid-save leaves the last good file (port addition).
    pub(crate) fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.path(rel);
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut tmp = p.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let r = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &p));
        if let Err(e) = r {
            let _ = std::fs::remove_file(&tmp);
            say(log::WARNING, &format!("can't write {}: {e}", p.display()));
        }
    }

    /// Each player's reputation id by entity index (`GetClientByEdict(p)->GetPlayerRepId()`).
    pub(crate) fn rep_ids(&self) -> Vec<i32> {
        let mut ids = vec![NO_CLIENT; rcbot::config::MAX_PLAYERS + 1];
        for c in self.clients.iter() {
            if let Some(p) = c.player
                && let Some(slot) = ids.get_mut(p.0 as usize)
            {
                *slot = c.player_rep_id;
            }
        }
        ids
    }
}

/// `PrepareHALBrainForPersonality` and `LoadHALBrainForPersonality`: the keyword lists, the brain
/// (a new one, saved, when the file is missing or damaged, then pretrained from the `.trn`).
/// A bot without a profile (id 0) gets a brain in memory only (design §4).
fn wake_hal(s: &State, profile: &BotProfile, name: &str) -> HalBot {
    let o = HalFileOverrides {
        pretrain: profile.hal_pretrain_file.clone(),
        aux: profile.hal_aux_file.clone(),
        ban: profile.hal_ban_file.clone(),
        swap: profile.hal_swap_file.clone(),
    };
    let f = HalFileNames::new(profile.profile_id, &o);
    let mut hal = HalBot::new(s.read(&f.ban).as_deref(), s.read(&f.aux).as_deref(), s.read(&f.swap).as_deref());
    if profile.profile_id == 0 {
        hal.pretrain(s.read(&f.pretrain).as_deref());
        return hal;
    }
    let brain = s.read(&f.brain);
    let pretrain = !cookie_is_valid(brain.as_deref());
    if pretrain {
        say(log::INFO, &format!("bot profile ({}) HAL brain damaged!", profile.profile_id));
        say(log::INFO, &format!("inferring a new HAL brain to profile ({})", profile.profile_id));
        s.write(&f.brain, &hal.save_brain());
    }
    match s.read(&f.brain) {
        Some(b) => {
            if hal.load_brain(&b).is_err() {
                say(log::WARNING, &format!("LoadHALBrainForPersonality(): {name}'s HAL brain damaged!"));
            } else if pretrain {
                hal.pretrain(s.read(&f.pretrain).as_deref());
            }
        }
        None => say(log::WARNING, &format!("LoadHALBrainForPersonality(): {name}'s HAL brain refuses to wake up! (file does not exist)")),
    }
    hal
}

/// The name of a bot's own learned files in the profiles folder: `<key>.<ext>`, the key being
/// the profile id, or `default-<name>` for a bot without a profile (design §4: upstream saved
/// nothing for those, so they never kept what they learned).
fn learned_file(b: &Bot, ext: &str) -> String {
    format!("{BOT_PROFILES_FOLDER}/{}.{ext}", profile_key(b.profile.profile_id, &b.name))
}

/// `SaveHALBrainForPersonality` (bots with a profile) and `CBot::saveLearnedData` (every bot,
/// design §4).
pub(crate) fn save_bot(s: &State, b: &Bot) {
    let id = b.profile.profile_id;
    if id != 0
        && let Some(hal) = &b.hal
    {
        let f = HalFileNames::new(id, &HalFileOverrides::default());
        s.write(&f.brain, &hal.save_brain());
    }
    s.write(&learned_file(b, "rld"), &b.learned.save(id));
    s.write(&learned_file(b, "rkf"), b.kungfu.save().as_bytes());
    s.write(&learned_file(b, rcbot::learn::weapon_stats::FILE_EXT), b.learning.weapons.save().as_bytes());
}

/// `CBotGlobals::saveLearnedData`: the teams' enemy-cost GAs and the TS loadout GA.
pub(crate) fn save_globals(s: &State) {
    for team in 0..rcbot::learn::data::MAX_TEAMS {
        s.write(&rcbot::learn::data::team_file_name(team), &s.learned.save_team(team));
    }
    s.write(rcbot::learn::data::TS_WEAPONS_FILE, &s.learned.save_ts_weapons());
    s.write(rcbot::learn::weapon_stats::GLOBAL_FILE, s.learned.weapon_prior.save().as_bytes());
    let h = &s.learned.heatmap;
    if !h.is_empty() && h.map_name() == s.map {
        s.write(&heatmap_path(s), &h.to_bytes());
    }
}

/// `waypoints/ts/<map>.rhm`: the map's heatmap, beside its waypoints.
fn heatmap_path(s: &State) -> String {
    format!("waypoints/{}/{}", s.config.mod_folder, rcbot::learn::heatmap::heatmap_file_name(&s.map))
}

/// The map's heatmap, for its waypoints as loaded (a new one when the file is missing or was
/// made for other waypoints), faded once for this load.
pub(crate) fn load_heatmap(s: &mut State) {
    let n = s.waypoints.len();
    let mut h = s
        .read(&heatmap_path(s))
        .and_then(|b| rcbot::learn::Heatmap::from_bytes(&b, &s.map, n))
        .unwrap_or_else(|| rcbot::learn::Heatmap::new(&s.map, n));
    h.decay(rcbot::learn::heatmap::MAP_DECAY);
    s.learned.heatmap = h;
}

/// A profile: `profile_id`'s `<id>.ini` (a bot coming back after a map change), else
/// `BotFunc_AddBot`'s choice from `bot_profiles.ini` among those no bot uses, else (no file, or
/// none free) `BotFunc_InitProfile`'s defaults with id 0 (upstream adds no bot then; the port has
/// to, design §4).
pub(crate) fn profile_for(s: &State, e: Engine, again: Option<i32>) -> BotProfile {
    let mut rng = EngineRandom(e);
    let mut profile = BotProfile::init(&s.config, &mut rng);
    let read_profile = |file: &str| s.read(&format!("{BOT_PROFILES_FOLDER}/{file}"));
    if let Some(id) = again.filter(|&i| i != 0)
        && let Some(data) = read_profile(&profile_file_name(id))
    {
        profile.read(&data);
        profile.profile_id = id;
        return profile;
    }
    let list = s.read(BOT_PROFILES_FILE);
    if list.is_none() {
        return profile;
    }
    let in_use: Vec<i32> = s.bots.iter().map(|b| b.profile.profile_id).collect();
    let (choice, msgs) = choose_profile(list.as_deref(), &in_use, &mut rng, read_profile);
    for m in msgs {
        say(log::INFO, &m);
    }
    if let ProfileChoice::Chosen { id, data } = choice {
        profile.read(&data);
        profile.profile_id = id;
    }
    profile
}

/// The rest of `BotFunc_ReadProfile` and `setupDataStructures` for a new `CBot`: the HAL brain,
/// the learned data, the reputations of every player here.
pub(crate) fn new_bot(s: &mut State, e: Engine, index: u32, name: String, profile: BotProfile) -> Bot {
    let mut rng = EngineRandom(e);
    let hal = wake_hal(s, &profile, &name);
    let mut bot = Bot::new(index, name, profile, &s.config, &mut rng);
    bot.hal = Some(hal);
    bot.join_server_time = e.time();
    let id = bot.profile.profile_id;
    if let Some(b) = s.read(&learned_file(&bot, "rld")) {
        bot.learned.load(&b, id);
    }
    if let Some(k) = s.read(&learned_file(&bot, "rkf")) {
        bot.kungfu.load(&String::from_utf8_lossy(&k));
    }
    if let Some(t) = s.read(&learned_file(&bot, rcbot::learn::weapon_stats::FILE_EXT)) {
        bot.learning.weapons.load(&String::from_utf8_lossy(&t));
    }
    if id != 0 {
        let rcr = format!("{BOT_PROFILES_FOLDER}/{}", rep_file_name(id));
        for c in s.clients.iter().filter(|c| c.is_used()) {
            if let Some(new) = bot.profile.rep.add_load_rep(c.player_rep_id, s.read(&rcr).as_deref()) {
                s.write(&rcr, &new);
            }
        }
    } else {
        for c in s.clients.iter().filter(|c| c.is_used()) {
            bot.profile.rep.add_load_rep(c.player_rep_id, None);
        }
    }
    bot
}

/// `BotFunc_AddBot`'s greeting: a liked player by name, or (with a chance growing with the
/// server's average reputation) everyone, which upstream's `ENTINDEX(NULL)` check stops.
pub(crate) fn greet(s: &mut State, a: Api, e: Engine, index: u32) {
    if !s.config.is_on(cfg::CHATTING) {
        return;
    }
    let reps = s.rep_ids();
    let w = HostWorld::new(a, e);
    let mut rng = EngineRandom(e);
    let clients = &s.clients;
    let Some(bot) = s.bots.get(index) else { return };
    let liked = bot
        .profile
        .rep
        .random_client(1, &mut rng, |id| clients.iter().find(|c| c.is_used() && c.player_rep_id == id).and_then(|c| c.player));
    let avg = bot.profile.rep.average_rep_on_server();
    let mut g = globals!(s, &reps);
    let Some(bot) = s.bots.get_mut(index) else { return };
    match liked {
        Some(p) => bot.bot_chat(&g, &w, ChatKind::Greetings, Some(p.0), false, &mut rng),
        None => {
            if rcbot::Random::random_long(&mut rng, 0, 10) < avg {
                bot.bot_chat(&g, &w, ChatKind::Greetings, None, false, &mut rng);
            }
        }
    }
    g.other_bots.clear();
}

/// `CClients::ClientConnected`: the player's `CClient` with its reputation id (bots too, as
/// upstream), and every bot loads its reputation of them.
pub(crate) fn put_in_server(s: &mut State, e: Engine, i: i32, is_bot: bool) {
    let name = e.edict(i).map(|ed| e.vars(ed, |v, _| v.netname)).map(|n| e.string(n)).unwrap_or_default();
    let ids = s.read(BOT_PLAYER_ID_FILE);
    let rep = player_rep_id(&name, ids.as_deref());
    if let Some(new) = &rep.write {
        s.write(BOT_PLAYER_ID_FILE, new);
    }
    let dedicated = e.is_dedicated();
    if let Some((_, Some(msg))) = s.clients.connect(PlayerId(i as u32), &name, "", rep.id, &s.users, dedicated)
        && !is_bot
    {
        e.client_print(i, &format!("{}{msg}\n", rcbot::config::BOT_DBG_MSG_TAG));
    }
    // `CBotCam::TuneOff`.
    e.set_view(i, i);
    let bots: Vec<u32> = s.bots.iter().map(|b| b.index).filter(|&b| b != i as u32).collect();
    for b in bots {
        let Some(bot) = s.bots.get_mut(b) else { continue };
        let id = bot.profile.profile_id;
        if id == 0 {
            bot.profile.rep.add_load_rep(rep.id, None);
            continue;
        }
        let rcr = format!("{BOT_PROFILES_FOLDER}/{}", rep_file_name(id));
        let file = std::fs::read(s.data_dir.join(&rcr)).ok();
        let Some(bot) = s.bots.get_mut(b) else { continue };
        if let Some(new) = bot.profile.rep.add_load_rep(rep.id, file.as_deref()) {
            s.write(&rcr, &new);
        }
    }
}

/// `ClientDisconnect` and `CClients::ClientDisconnected`: a leading player hands the squad on,
/// every bot saves and drops its reputation of them and may greet them again; a bot saves its
/// brain and learned data.
pub(crate) fn disconnect(s: &mut State, a: Api, e: Engine, i: i32) {
    let w = HostWorld::new(a, e);
    // Its client's grip ends (`CClient::Init`); a grip on it would follow the next player in
    // the slot.
    s.force_grips.remove(&(i as u32));
    s.force_grips.retain(|_, g| *g != i as u32);
    if let Some(p) = w.entity(i as u32)
        && let Some(id) = s.squads.find_by_leader(p.handle())
    {
        s.squads.change_leader(id, &w);
    }
    let ci = s.clients.index_of(PlayerId(i as u32));
    let rep_id = ci.and_then(|c| s.clients.get(c)).map_or(-1, |c| c.player_rep_id);
    if rep_id >= 0 {
        let bots: Vec<u32> = s.bots.iter().map(|b| b.index).collect();
        for b in bots {
            let Some(id) = s.bots.get(b).map(|x| x.profile.profile_id) else { continue };
            let rcr = format!("{BOT_PROFILES_FOLDER}/{}", rep_file_name(id));
            let file = if id != 0 { std::fs::read(s.data_dir.join(&rcr)).ok() } else { None };
            let Some(bot) = s.bots.get_mut(b) else { continue };
            let new = bot.profile.rep.remove_save_rep(rep_id, file.as_deref());
            if (1..=32).contains(&i) {
                bot.forget_greeting(i as u32);
            }
            if id != 0
                && let Some(new) = new
            {
                s.write(&rcr, &new);
            }
        }
    }
    if let Some(b) = s.bots.remove(i as u32) {
        save_bot(s, &b);
    }
    s.stats.remove(&(i as u32));
    if let Some(ci) = ci {
        s.clients.disconnect(ci);
    }
}

/// `player_killed`: `BotClient_Generic_DeathMessage`'s events, and the e2e test's kill lines.
pub(crate) fn killed(s: &mut State, a: Api, e: Engine, victim: i32, killer: i32) {
    let name = |i: i32| e.edict(i).map(|ed| e.vars(ed, |v, _| v.netname)).map(|n| e.string(n)).unwrap_or_default();
    a.log(log::DEBUG, &format!("death #{victim} \"{}\" by #{killer} \"{}\"", name(victim), name(killer)));
    if killer > 0 && killer != victim && s.bots.get(killer as u32).is_some() {
        a.log(log::INFO, &format!("kill \"{}\" killed \"{}\"", name(killer), name(victim)));
        s.check("kill");
    }
    let reps = s.rep_ids();
    let w = HostWorld::new(a, e);
    let mut rng = EngineRandom(e);
    let mut g = globals!(s, &reps);
    s.bots.death(&mut g, &w, killer, victim, &mut rng);
}

/// A damage or kill event for the bots' learners (design §4).
pub(crate) fn combat(s: &mut State, a: Api, e: Engine, ev: &rcbot::learn::CombatEvent) {
    let reps = s.rep_ids();
    let w = HostWorld::new(a, e);
    let mut rng = EngineRandom(e);
    let mut g = globals!(s, &reps);
    s.bots.combat_event(&mut g, &w, ev, &mut rng);
}

/// `kill_scored`: the rules' view of a death (its killer, weapon and tags).
pub(crate) fn kill_scored(s: &mut State, a: Api, e: Engine, k: &ts_plugin_api::KillInfo) {
    let (Ok(victim), Ok(killer)) = (u32::try_from(k.victim), u32::try_from(k.killer.max(0))) else { return };
    if victim == 0 {
        return;
    }
    let tags = k.tags;
    let ev = rcbot::learn::CombatEvent::Kill(rcbot::learn::KillEvent {
        killer,
        victim,
        weapon: k.weapon,
        headshot: tags & ts_plugin_api::kill::HEADSHOT != 0,
        close_combat: tags & ts_plugin_api::kill::CLOSE_COMBAT != 0,
        kung_fu: tags & ts_plugin_api::kill::KUNG_FU != 0,
    });
    combat(s, a, e, &ev);
}

/// `sound`: `pfnEmitSound`'s part for the bots.
pub(crate) fn sound(s: &mut State, a: Api, e: Engine, entity: i32, origin: Option<Vector>, volume: f32, sample: &str) {
    let Ok(entity) = u32::try_from(entity) else { return };
    let w = HostWorld::new(a, e);
    s.bots.sound(&w, entity, origin, volume, sample);
}

/// The view angles the game sets on a bot (`pev->fixangle`: a spawn, a teleport, a stunt), as
/// the engine hands them to a real client: `SV_WriteClientdataToMessage` sends `svc_setangle`
/// (`pev->angles`) or `svc_addangle` (`avelocity` yaw) and clears `fixangle`. The engine skips
/// fake clients there, so a bot's `fixangle` was never cleared, and `SV_RunCmd` copies the
/// usercmd's angles into `pev->v_angle` only while it is 0: after its first spawn the bot's view
/// stayed where the game put it, and it never turned or aimed. Upstream wrote `pev->v_angle`
/// itself (`ChangeAngles`), as the port's own bots do (design §4). A set angle (1) also means
/// the game put the bot somewhere (a spawn, a teleport): its path is dropped
/// ([`Bot::teleported`]).
pub(crate) fn take_fixed_angles(s: &mut State, e: Engine) {
    for b in s.bots.iter_mut() {
        let Some(ed) = e.edict(b.index as i32) else { continue };
        let fixed = e.vars_mut(ed, |v| {
            match v.fixangle {
                1 => v.v_angle = v.angles,
                2 => {
                    v.v_angle[1] += v.avelocity[1];
                    v.avelocity[1] = 0.0;
                }
                _ => return 0,
            }
            std::mem::replace(&mut v.fixangle, 0)
        });
        if fixed == 1 {
            b.teleported();
        }
    }
}

/// `DispatchTouch` for the bots: the port has no touch callback for plugins, so a bot touches
/// what its box meets (what the engine does for triggers; a solid it is pressed against touches
/// within one unit). Design §4.
pub(crate) fn touches(s: &mut State, a: Api, e: Engine) {
    let reps = s.rep_ids();
    let w = HostWorld::new(a, e);
    let mut rng = EngineRandom(e);
    let indices: Vec<u32> = s.bots.iter().filter(|b| b.is_alive()).map(|b| b.index).collect();
    let g = globals!(s, &reps);
    for i in indices {
        let Some(me) = w.entity(i) else { continue };
        let (lo, hi) = (me.absmin - Vector::new(1.0, 1.0, 1.0), me.absmax + Vector::new(1.0, 1.0, 1.0));
        for t in w.entities_in_sphere(me.origin, 128.0) {
            if t == 0 || t == i {
                continue;
            }
            let Some(other) = w.entity(t) else { continue };
            let overlap = other.absmin.x <= hi.x
                && other.absmax.x >= lo.x
                && other.absmin.y <= hi.y
                && other.absmax.y >= lo.y
                && other.absmin.z <= hi.z
                && other.absmax.z >= lo.z;
            if overlap && other.solid != rcbot::bot::SOLID_NOT {
                s.bots.touch(&g, &w, i, &other, &mut rng);
            }
        }
    }
}

/// `ClientCommand`'s `say` / `say_team`: `say_team "form up"` makes a squad; anything else may
/// get MegaHAL replies (not from bots unless `chat_reply_to_bots`). Upstream builds a message
/// said in several arguments from the last one only (its `szTemp` is never set; design §5).
pub(crate) fn say_command(s: &mut State, a: Api, e: Engine, player: i32, args: &[String]) {
    let arg = |i: usize| args.get(i).map_or("", String::as_str);
    let team_only = arg(0) == "say_team";
    let w = HostWorld::new(a, e);
    let Some(sender) = w.entity(player as u32) else { return };
    if team_only && arg(1).starts_with("form") && (arg(1)[4..].starts_with(" up") || arg(2).starts_with("up")) {
        let reps = s.rep_ids();
        let mut g = globals!(s, &reps);
        s.bots.make_squad(&mut g, &w, &sender);
        flush_squad_messages(s, e);
        return;
    }
    let sender_is_bot = s.bots.get(player as u32).is_some();
    if sender_is_bot && !s.config.is_on(cfg::CHAT_REPLY_TO_BOTS) {
        return;
    }
    let message =
        if !arg(2).is_empty() { args.iter().skip(1).rev().find(|x| !x.is_empty()).cloned().unwrap_or_default() } else { arg(1).to_owned() };
    if message.is_empty() {
        return;
    }
    let reps = s.rep_ids();
    let mut rng = EngineRandom(e);
    let g = globals!(s, &reps);
    s.bots.say(&g, &w, &sender, &message, team_only, &mut rng);
}

/// Prints what squads told their leaders (`ClientPrint(HUD_PRINTTALK)`).
pub(crate) fn flush_squad_messages(s: &mut State, e: Engine) {
    for (p, m) in std::mem::take(&mut s.squads.messages) {
        let line = if m.ends_with('\n') { m } else { format!("{m}\n") };
        e.client_print(p as i32, &line);
    }
}

/// `CBotCam::Spawn`: an invisible `info_target` with the mechgibs model, once a map, if
/// `allow_botcam` is on.
fn spawn_camera(cam: &mut BotCam, e: Engine) {
    cam.tried_to_spawn = true;
    let Some(i) = e.create_named_entity(c"info_target") else { return };
    e.set_model(i, c"models/mechgibs.mdl");
    let class = e.alloc_string(c"entity_botcam");
    let now = e.time();
    if let Some(ed) = e.edict(i) {
        e.vars_mut(ed, |v| {
            v.takedamage = 0.0;
            v.solid = goldsrc_sys::SOLID_NOT as i32;
            v.movetype = goldsrc_sys::MOVETYPE_FLY as i32;
            v.classname = class;
            v.nextthink = now;
            v.renderamt = 0.0;
        });
    }
    cam.camera = Some(i as u32);
}

/// `StartFrame`'s `m_BotCam.Think()` / `Spawn()`.
pub(crate) fn botcam_frame(s: &mut State, a: Api, e: Engine) {
    let enabled = s.config.is_on(cfg::ENABLE_BOTCAM);
    if !s.botcam.is_working(enabled) {
        if !s.botcam.tried_to_spawn && enabled {
            spawn_camera(&mut s.botcam, e);
        }
        return;
    }
    let w = HostWorld::new(a, e);
    let mut rng = EngineRandom(e);
    s.botcam.think(&s.bots, &s.waypoints, &w, &mut rng);
    let (Some(cam), view) = (s.botcam.camera, s.botcam.view) else { return };
    if let Some(ed) = e.edict(cam as i32) {
        e.vars_mut(ed, |v| {
            v.origin = view.origin.to_array();
            v.angles = view.angles.to_array();
            v.v_angle = view.v_angle.to_array();
        });
    }
}

/// `TuneIn`: `None` if the player now watches the camera, else why not.
pub(crate) fn botcam_tune_in(s: &State, e: Engine, player: u32) -> Option<&'static str> {
    if let Some(err) = s.botcam.tune_in_error(s.bots.len()) {
        return Some(err);
    }
    if let Some(cam) = s.botcam.camera {
        e.set_view(player as i32, cam as i32);
    }
    None
}

/// `BotMenu_Func_Squad_RemoveAllBotSquads`: bots of the player's team drop their squads.
pub(crate) fn remove_all_bot_squads(s: &mut State, team: i32) -> i32 {
    let mut n = 0;
    for b in s.bots.iter_mut() {
        if b.team_number() == team && b.remove_my_squad(&mut s.squads) {
            n += 1;
        }
    }
    n
}

/// `CClient::Think`'s force grip: each gripped entity is held 210 units in front of its
/// player's eyes (or short of what is in the way), turned to face the player, and frozen.
/// Upstream builds the end point from `gpGlobals->v_forward` before `UTIL_MakeVectors(v_angle)`,
/// so from whatever direction was computed last (design §5); the port uses the player's view.
pub(crate) fn force_grip_frame(s: &mut State, a: Api, e: Engine) {
    if s.force_grips.is_empty() {
        return;
    }
    let w = HostWorld::new(a, e);
    let grips: Vec<(u32, u32)> = s.force_grips.iter().map(|(&p, &g)| (p, g)).collect();
    for (player, gripped) in grips {
        let (Some(p), Some(g)) = (w.entity(player), w.entity(gripped)) else {
            s.force_grips.remove(&player);
            continue;
        };
        let (forward, _) = rcbot::bot::make_vectors(p.v_angle);
        let src = p.origin + p.view_ofs;
        let mut end = src + forward * 210.0;
        let tr = rcbot::nav::World::trace_line(&w, src, end, rcbot::nav::Monsters::Ignore, rcbot::nav::Glass::DontIgnore, Some(player));
        if tr.fraction < 1.0 {
            let comp = tr.end_pos - src;
            let dist = comp.length();
            end = src + comp * (1.0 / dist) * (dist - g.size.length_2d());
        }
        e.set_origin(gripped as i32, end);
        let mut angles = rcbot::bot::vec_to_angles(src - end);
        rcbot::bot::fix_angles(&mut angles);
        angles.z = 0.0;
        angles.x = g.v_angle.x;
        if let Some(ed) = e.edict(gripped as i32) {
            e.vars_mut(ed, |v| {
                v.v_angle = angles.to_array();
                v.angles = angles.to_array();
                v.angles[0] = angles.x / 3.0;
                v.flags |= rcbot::bot::entflags::FROZEN;
            });
        }
    }
}

/// `-rcbot_force_grip`: `FL_FROZEN` off and `DROP_TO_FLOOR`.
pub(crate) fn release_force_grip(e: Engine, gripped: u32) {
    if let Some(ed) = e.edict(gripped as i32) {
        e.vars_mut(ed, |v| v.flags &= !rcbot::bot::entflags::FROZEN);
        e.drop_to_floor(gripped as i32);
    }
}
