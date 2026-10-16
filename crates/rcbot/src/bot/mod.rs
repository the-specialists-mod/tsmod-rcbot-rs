//! The bot (`bot.h`/`bot.cpp` `CBot`, `bot_navigate.cpp`'s per-frame movement, `bot_visibles.cpp`,
//! `bot_weapons.cpp`'s `CBotWeapon(s)`, `globals.cpp`'s bot loop).
//!
//! A [`Bot`] is one `CBot`: [`Bot::frame`] is upstream's `Think()` followed by
//! `RunPlayerMove()`, reading the world through [`BotWorld`] and answering with the usercmd and
//! the game actions upstream did through the engine ([`Action`]). Shared state (`gBotGlobals`'
//! waypoints, config, weapons, learning) comes in as [`BotGlobals`].
//!
//! TS only: every `IsMod(MOD_TS)` branch is taken, the others dropped (design §2). What upstream
//! read from the edict's private data or user messages comes from the port's structured state
//! ([`PlayerState`]), which is also how teams are known (upstream compared model strings, which
//! TS 3.0's teams don't use; the-specialistsmod-rs#104).

pub mod board;
pub mod botcam;
mod combat;
pub mod events;
mod fire;
mod goals;
mod hearing;
mod items;
pub mod kungfu;
pub mod learning;
mod look;
mod manager;
mod movement;
mod navigate;
mod preaim;
mod remember;
pub(crate) mod special;
pub mod squad;
pub mod tasks;
pub(crate) mod think;
pub mod weapons;

#[cfg(test)]
mod tests;

pub use botcam::BotCam;
pub use events::{BotEvent, NO_CLIENT, SoundKind};
pub use learning::BotLearning;
pub use look::{fix_angles, make_vectors, vec_to_angles};
pub use manager::{BotManager, BotSlot};
pub use remember::RememberPositions;
pub use special::{MasterEntity, OtherBot};
pub use squad::{SquadId, SquadOrder, Squads};
pub use tasks::{EntRef, ScheduleDesc, Task, TaskKind, Tasks};
pub use weapons::{BotWeapon, BotWeapons, InventoryWeapon};

use crate::config::{BotProfile, Config};
use crate::learn::{BotLearnedData, GlobalLearnedData};
use crate::nav::{AStar, AStarBot, Waypoints, World};
use crate::weapons::{WeaponPresets, Weapons};
use crate::{Random, Vector};

/// `in_buttons.h`.
pub mod buttons {
    /// `IN_ATTACK`.
    pub const ATTACK: i32 = 1 << 0;
    /// `IN_JUMP`.
    pub const JUMP: i32 = 1 << 1;
    /// `IN_DUCK`.
    pub const DUCK: i32 = 1 << 2;
    /// `IN_FORWARD`.
    pub const FORWARD: i32 = 1 << 3;
    /// `IN_BACK`: TS's kung fu reads it for the back kick below 100 units/s.
    pub const BACK: i32 = 1 << 4;
    /// `IN_USE`.
    pub const USE: i32 = 1 << 5;
    /// `IN_ATTACK2`.
    pub const ATTACK2: i32 = 1 << 11;
    /// `IN_RELOAD`.
    pub const RELOAD: i32 = 1 << 13;
    /// `IN_ALT1`: TS's stunt / dive key.
    pub const ALT1: i32 = 1 << 14;
}

/// `pev->flags` bits the bot reads (`const.h`).
pub mod entflags {
    /// `FL_CLIENT`.
    pub const CLIENT: i32 = 1 << 3;
    /// `FL_MONSTER`.
    pub const MONSTER: i32 = 1 << 5;
    /// `FL_ONGROUND`.
    pub const ONGROUND: i32 = 1 << 9;
    /// `FL_FROZEN`.
    pub const FROZEN: i32 = 1 << 12;
    /// `FL_DUCKING`.
    pub const DUCKING: i32 = 1 << 14;
    /// `FL_WORLDBRUSH`.
    pub const WORLDBRUSH: i32 = 1 << 25;
    /// `FL_KILLME`.
    pub const KILLME: i32 = 1 << 30;
    /// `FL_DORMANT`.
    pub const DORMANT: i32 = 1 << 31;
}

/// `MOVETYPE_FLY` (ladders).
pub const MOVETYPE_FLY: i32 = 5;
/// `MOVETYPE_WALK`.
pub const MOVETYPE_WALK: i32 = 3;
/// `SOLID_NOT`.
pub const SOLID_NOT: i32 = 0;
/// `SOLID_TRIGGER`.
pub const SOLID_TRIGGER: i32 = 1;
/// `EF_NODRAW`.
pub const EF_NODRAW: i32 = 128;
/// `DEAD_NO`.
pub const DEAD_NO: i32 = 0;

/// Seconds between two presses of the reload key for the same clip (TS's reloads take 1.5-3 s;
/// a reload that started holds the gun until it ends, so one press is enough).
pub const RELOAD_RETRY: f32 = 2.0;
/// `BOT_DEFAULT_REACTION_TIME`.
pub const BOT_DEFAULT_REACTION_TIME: f32 = 0.3;
/// `BOT_DEFAULT_THINK_TIME`.
pub const BOT_DEFAULT_THINK_TIME: f32 = 0.04;
/// `BOT_VIEW_DISTANCE` (`bot_const.h`): how far the bot sees.
pub const BOT_VIEW_DISTANCE: f32 = 2048.0;
/// `BOT_MAX_REP`.
pub const BOT_MAX_REP: i32 = 10;
/// `BOT_FACE_HURT_TIME`.
pub const BOT_FACE_HURT_TIME: f32 = 0.5;
/// `BOT_STUCK_CLIMB_SPEED`.
pub const BOT_STUCK_CLIMB_SPEED: f32 = 10.0;
/// `BOT_STUCK_SWIM_SPEED`.
pub const BOT_STUCK_SWIM_SPEED: f32 = 32.0;
/// `BOT_COST_RANGE`.
pub const BOT_COST_RANGE: f32 = 64.0;
/// `BOT_COST_BUCKETS`.
pub const BOT_COST_BUCKETS: usize = 7;
/// `eTS_State`'s `TS_State_Stunt`.
pub const TS_STATE_STUNT: i32 = 2;

/// A player's TS state from the port (`PlayerInfo`), what upstream read from user messages.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerState {
    /// Joined and not spectating.
    pub joined: bool,
    /// Alive and not observing.
    pub alive: bool,
    /// Any backend's bot.
    pub is_bot: bool,
    /// The rules team from 1 in teamplay, 0 for none.
    pub team: i32,
    /// The weapon in hand (TS id, 0 kung fu).
    pub weapon: i32,
    /// Rounds in the clip.
    pub clip: i32,
    /// Reserve of its ammo type.
    pub ammo: i32,
    /// `TSState` (`eTS_State`).
    pub stunt_state: i32,
    /// The weapon in hand's fire mode.
    pub fire_mode: i32,
    /// The weapon in hand's attachment bits (`TSA_*`).
    pub attachments: i32,
    /// Free inventory slots (`TSSpace`).
    pub free_slots: i32,
    /// The power-up running now (the port's `powerup` bits), 0 for none.
    pub active_powerup: i32,
    /// Game time at which `active_powerup` ends, 0 for none.
    pub powerup_until: f32,
    /// The power-up held but not used yet, 0 for none.
    pub stored_powerup: i32,
    /// The player's time scale (1 normal, below 1 slowed).
    pub time_scale: f32,
    /// Permanent kung fu picked up this life.
    pub kungfu: bool,
    /// Carries the capture-the-briefcase case.
    pub has_flag: bool,
    /// [`pflag`] bits (the port's `PlayerSnapshot::flags`), 0 when the world can't tell.
    pub flags: i32,
    /// TS's stunt bits (`pev->iuser4`, [`stunt_bits`]).
    pub stunt_bits: i32,
    /// `pev->armorvalue`.
    pub armor: f32,
    /// Changes whenever the player's inventory does (a weapon got or lost, a clip, reserve, mode
    /// or attachment changed); 0 when the world can't tell.
    pub inventory_serial: i32,
    /// The close-combat and stunt timers, `None` when the world can't tell.
    pub combat: Option<CombatState>,
}

impl PlayerState {
    /// Lying after a dive or slide (TS's prone stunt bit).
    pub fn prone(&self) -> bool {
        self.flags & pflag::PRONE != 0
    }

    /// In a stunt: diving, flipping, rolling, on a wall, or ducked for one.
    pub fn stunting(&self) -> bool {
        self.flags & pflag::STUNTING != 0
    }
}

/// [`PlayerState::flags`]: the plugin API's `player_flag` bits.
pub mod pflag {
    /// Past the team menus and not spectating.
    pub const JOINED: i32 = 0x1;
    /// Alive and not observing.
    pub const ALIVE: i32 = 0x2;
    /// A bot of any backend.
    pub const BOT: i32 = 0x4;
    /// `FL_ONGROUND`.
    pub const ON_GROUND: i32 = 0x8;
    /// `FL_DUCKING`.
    pub const DUCKING: i32 = 0x10;
    /// On a ladder.
    pub const ON_LADDER: i32 = 0x20;
    /// Waist deep in water or more.
    pub const IN_WATER: i32 = 0x40;
    /// Carries the capture-the-briefcase case.
    pub const HAS_FLAG: i32 = 0x80;
    /// Observing.
    pub const SPECTATOR: i32 = 0x100;
    /// Lying after a dive or slide.
    pub const PRONE: i32 = 0x200;
    /// In a stunt (dive, flip, roll, wall jump or climb, or ducked for one).
    pub const STUNTING: i32 = 0x400;
}

/// TS's stunt bits ([`PlayerState::stunt_bits`], the plugin API's `stunt_bits`) the bot reads.
pub mod stunt_bits {
    /// Half speed while getting up.
    pub const RECOVER: i32 = 0x1;
    /// Diving.
    pub const DIVING: i32 = 0x10;
    /// Lying after a dive or slide.
    pub const PRONE: i32 = 0x20;
}

/// A player's close-combat and stunt timers (the plugin API's `CombatState`), as game times at
/// which they run out, 0 for one that isn't running. TS counts most of them in the player's own
/// time, so a slowed player's run out later than they say.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CombatState {
    /// Kung fu power, 0..100: the bar a move costs and blocks drain.
    pub melee_power: f32,
    /// No attack before (`m_flNextAttack`, or the gun's primary delay when later).
    pub next_attack_at: f32,
    /// No stunt (dive, wall jump) before.
    pub next_stunt_at: f32,
    /// Getting up: half speed.
    pub recover_until: f32,
    /// Can't get up from lying before.
    pub prone_lock: f32,
    /// The slow-motion dodge: bullets do x0.7 until.
    pub dodge_until: f32,
    /// Blows of kung fu are blocked until.
    pub block_kungfu: f32,
    /// Knife blows are blocked until.
    pub block_knife: f32,
    /// Katana blows are blocked until.
    pub block_katana: f32,
    /// A delayed melee hit lands until.
    pub melee_window: f32,
}

/// What a game item is ([`ItemInfo::kind`], the plugin API's `entity_info` kinds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemKind {
    /// Anything else.
    #[default]
    Other,
    /// A weapon lying in the map (`ts_groundweapon`): [`ItemInfo::weapon`] is its TS id.
    GroundWeapon,
    /// `ts_powerup`: [`ItemInfo::powerup`] is its bits.
    Powerup,
    /// A thrown grenade, knife or gun.
    Projectile,
    /// A gun a player holds or dropped.
    Gun,
    /// The capture-the-briefcase case lying in the map: [`ItemInfo::team`] owns it.
    CtbFlag,
    /// A capture point: [`ItemInfo::team`] may capture there (0 any).
    CtbBase,
    /// A team or score objective (`ts_wingiver`, `ts_scoregiver`).
    Objective,
}

/// The port's `entity_info` for a game item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemInfo {
    /// What it is.
    pub kind: ItemKind,
    /// TS weapon id (ground weapons, guns), else 0.
    pub weapon: i32,
    /// Power-up bits, else 0.
    pub powerup: i32,
    /// The responsible player (thrower, holder), 0 for none.
    pub owner: u32,
    /// Its rules team, 0 for none or any.
    pub team: i32,
    /// It can be picked up or used now (visible, not waiting to respawn).
    pub available: bool,
}

/// An item lying in the map, as [`BotWorld::map_items`] lists it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapItem {
    /// The entity.
    pub handle: EntRef,
    /// A ground weapon, a power-up or a dropped gun.
    pub kind: ItemKind,
    /// Its TS weapon id, or 0.
    pub weapon: i32,
    /// Its power-up bits, or 0.
    pub powerup: i32,
    /// Where it lies.
    pub origin: Vector,
    /// It can be taken now.
    pub available: bool,
    /// Taken and coming back: the game time it shows again (0 when there, or not before the
    /// next round).
    pub respawn_at: f32,
}

/// The game's mode as the port reports it (`GameState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameMode {
    /// `CTSGameRules::m_mode`: 0 deathmatch, 1 team deathmatch, 3 objective.
    pub mode: i32,
    /// The map has a capture-the-briefcase case.
    pub ctb: bool,
    /// 0 no rounds, 1 a round running, 2 a round over.
    pub round: i32,
}

/// What [`BotWorld::entity`] says about an entity: the `entvars_t` fields the bot reads, and for
/// players the port's [`PlayerState`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entity {
    /// Its index.
    pub index: u32,
    /// `serialnumber`.
    pub serial: i32,
    /// `classname`.
    pub classname: String,
    /// `netname`.
    pub netname: String,
    /// `origin`.
    pub origin: Vector,
    /// `view_ofs`.
    pub view_ofs: Vector,
    /// `absmin`.
    pub absmin: Vector,
    /// `absmax`.
    pub absmax: Vector,
    /// `size`.
    pub size: Vector,
    /// `velocity`.
    pub velocity: Vector,
    /// `basevelocity`.
    pub basevelocity: Vector,
    /// `angles`.
    pub angles: Vector,
    /// `v_angle`.
    pub v_angle: Vector,
    /// `flags`.
    pub flags: i32,
    /// `health`.
    pub health: f32,
    /// `max_health`.
    pub max_health: f32,
    /// `spawnflags`.
    pub spawnflags: i32,
    /// `target`.
    pub target: String,
    /// `targetname`.
    pub targetname: String,
    /// `movedir`.
    pub movedir: Vector,
    /// `avelocity`.
    pub avelocity: Vector,
    /// `frame`.
    pub frame: f32,
    /// `deadflag`.
    pub deadflag: i32,
    /// `takedamage`.
    pub takedamage: f32,
    /// `effects`.
    pub effects: i32,
    /// `solid`.
    pub solid: i32,
    /// `movetype`.
    pub movetype: i32,
    /// `waterlevel`.
    pub waterlevel: i32,
    /// `maxspeed`.
    pub maxspeed: f32,
    /// `button`.
    pub button: i32,
    /// `frags`.
    pub frags: f32,
    /// `owner`, as an index.
    pub owner: Option<u32>,
    /// `groundentity`, as an index.
    pub groundentity: Option<u32>,
    /// A connected player's TS state.
    pub player: Option<PlayerState>,
    /// What a game item is (weapons, power-ups, CTB entities), from the port.
    pub item: Option<ItemInfo>,
}

impl Entity {
    /// A handle on this entity.
    pub fn handle(&self) -> EntRef {
        EntRef { index: self.index, serial: self.serial }
    }

    /// `EntityOrigin`: monsters at half eye height, everything else the middle of its box.
    pub fn entity_origin(&self) -> Vector {
        if self.flags & entflags::MONSTER != 0 {
            return self.origin + self.view_ofs * 0.5;
        }
        self.absmin + self.size * 0.5
    }

    /// `BotFunc_EntityIsMoving`: any linear or angular velocity.
    pub fn is_moving(&self) -> bool {
        self.velocity != Vector::ZERO || self.avelocity != Vector::ZERO
    }

    /// `EntityIsAlive`.
    pub fn is_alive(&self) -> bool {
        self.deadflag == DEAD_NO && self.health > 0.0
    }

    /// A client (`FL_CLIENT`).
    pub fn is_client(&self) -> bool {
        self.flags & entflags::CLIENT != 0
    }
}

/// What the bot asks the engine and the game. `crates/host` implements it on the engine's
/// functions and the plugin API; tests fake it.
pub trait BotWorld: World {
    /// `gpGlobals->time`.
    fn time(&self) -> f32;
    /// `gpGlobals->frametime`: the last frame's length (0 by default).
    fn frame_time(&self) -> f32 {
        0.0
    }
    /// `gpGlobals->maxEntities`.
    fn max_entities(&self) -> u32;
    /// `gpGlobals->maxClients`.
    fn max_clients(&self) -> u32;
    /// The highest entity index in use this frame (the plugin API's `FrameInfo::highest_edict`):
    /// nothing above it exists, so entity sweeps stop there. [`max_entities`](Self::max_entities)
    /// by default.
    fn highest_entity(&self) -> u32 {
        self.max_entities()
    }
    /// The entity at `index`, `None` for a free or missing edict (`FNullEnt`).
    fn entity(&self, index: u32) -> Option<Entity>;
    /// What the vision pass looks at first: the entity's [`Entity::entity_origin`] and `effects`,
    /// `None` like [`entity`](Self::entity). Worlds that can read these without building the
    /// whole entity should.
    fn entity_glance(&self, index: u32) -> Option<(Vector, i32)> {
        self.entity(index).map(|e| (e.entity_origin(), e.effects))
    }
    /// `ENGINE_CHECK_VISIBILITY(viewer, ENGINE_SET_PVS(origin))`: whether `viewer` is in the PVS
    /// of `origin`.
    fn in_pvs(&self, viewer: u32, origin: Vector) -> bool;
    /// `UTIL_FindEntityInSphere`, every match.
    fn entities_in_sphere(&self, origin: Vector, radius: f32) -> Vec<u32>;
    /// `gBotGlobals.m_bTeamPlay || isMapType(NON_TS_TEAMPLAY)`: the game's teamplay, or a `tm_`
    /// map.
    fn teamplay(&self) -> bool;
    /// Teams are players' `model` keys, as upstream decides them: a `tm_` map the game doesn't
    /// run as teamplay (its teams would all be 0). Otherwise teams are [`PlayerState::team`].
    fn model_teams(&self) -> bool {
        false
    }
    /// A player's `model` info key (`INFOKEY_VALUE(GET_INFOKEYBUFFER(e), "model")`).
    fn player_model(&self, _index: u32) -> String {
        String::new()
    }
    /// `UTIL_FindEntityByString(start, key, value)` over the whole entity list, in the engine's
    /// order (`key` is `classname`, `target` or `targetname`). None by default.
    fn find_entities(&self, key: EntityKey, value: &str) -> Vec<u32> {
        let _ = (key, value);
        Vec::new()
    }
    /// `pfnTraceHull(start, end, monsters, point_hull, ignore)`; a line by default.
    fn trace_point_hull(&self, start: Vector, end: Vector, monsters: crate::nav::Monsters, ignore: Option<u32>) -> crate::nav::Trace {
        self.trace_line(start, end, monsters, crate::nav::Glass::DontIgnore, ignore)
    }
    /// `CVAR_GET_FLOAT`; 0 by default.
    fn cvar_float(&self, name: &str) -> f32 {
        let _ = name;
        0.0
    }
    /// The weapons player `index` owns (kung fu not included), with their clips and reserves;
    /// `None` when the world can't tell (then the bot knows only the weapons it held).
    fn inventory(&self, index: u32) -> Option<Vec<InventoryWeapon>> {
        let _ = index;
        None
    }
    /// The items lying in the map a bot may want (`ts_groundweapon`, `ts_powerup`): an experienced
    /// player knows where they spawn.
    fn items(&self) -> Vec<u32> {
        let mut out = self.find_entities(EntityKey::Classname, "ts_groundweapon");
        out.extend(self.find_entities(EntityKey::Classname, "ts_powerup"));
        out
    }
    /// The items lying in the map as the game lists them (the plugin API's `items`: ground
    /// weapons, power-ups and dropped guns, taken ones too), `None` when the world can't tell
    /// (then [`items`](Self::items) and what the bot sees).
    fn map_items(&self) -> Option<Vec<MapItem>> {
        None
    }
    /// The game's mode, CTB and rounds; deathmatch by default.
    fn game_mode(&self) -> GameMode {
        GameMode::default()
    }
    /// The TS objectives player `index` has been told of (`m_TSObjectives`, from the `TSObjective`
    /// messages): the HUD's objective list that can still be completed. None by default.
    fn ts_objectives(&self, index: u32) -> Vec<TsObjective> {
        let _ = index;
        Vec::new()
    }
}

/// `TSObjective`: an objective the `TSObjective` message listed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TsObjective {
    /// `m_iId`: the message's index.
    pub id: i32,
    /// The objective's entity (`ts_hack`, `ts_bomb`, ...), if the port says (port addition).
    pub entity: Option<u32>,
    /// `m_vOrigin`: the objective's centre.
    pub origin: Vector,
    /// `m_szName`: its text for the bot's team.
    pub name: String,
}

/// The string field [`BotWorld::find_entities`] matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKey {
    /// `classname` (`UTIL_FindEntityByClassname`).
    Classname,
    /// `target` (`UTIL_FindEntityByTarget`, `FIND_ENTITY_BY_TARGET`).
    Target,
    /// `targetname` (`UTIL_FindEntityByTargetname`).
    Targetname,
}

/// What the bot does that isn't its usercmd: upstream's `FakeClientCommand`, the TS weapon
/// switch, `UTIL_makeTSweapon`, `ClientKill`, the kick.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// `FakeClientCommand(m_pEdict, line)`: the port's `bot_command`.
    Command(String),
    /// TS's weapon switch (`SwitchWeapon`: upstream's `CmdStart` with `weaponselect`, which TS
    /// 3.0 ignores; the port's `select_weapon`).
    SelectWeapon(i32),
    /// `UTIL_makeTSweapon` for each weapon of the loadout: the port's `clear_loadout` and `buy`,
    /// so the weapons come at the next spawn (upstream spawned them for free at once; design §4).
    Loadout(Vec<i32>),
    /// `ClientKill`.
    Kill,
    /// `kick "<name>"`.
    Kick,
}

/// The usercmd `pfnRunPlayerMove` gets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BotCmd {
    /// View angles (the port's `run_cmd` takes `v_angle`; design §4).
    pub angles: Vector,
    /// `m_fMoveSpeed`.
    pub forward: f32,
    /// `m_fStrafeSpeed`.
    pub side: f32,
    /// `m_fUpSpeed`.
    pub up: f32,
    /// `pev->button`.
    pub buttons: i32,
    /// `pev->impulse`.
    pub impulse: i32,
}

/// What the game did with the bot's last usercmd (the plugin API's `BotCmdResult`), which the
/// host hands back with [`Bot::command_ran`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CmdOutcome {
    /// The game set the bot's view (`fixangle`: a spawn, a teleport, `ts_tp`): these are its view
    /// angles now, and the command's were ignored.
    pub game_view: Option<Vector>,
    /// The game moved the bot further than it could have walked (a teleport, a respawn).
    pub teleported: bool,
}

/// What one frame of a bot produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// The usercmd, or `None` when upstream skips `pfnRunPlayerMove` this frame.
    pub cmd: Option<BotCmd>,
    /// Actions, in order.
    pub actions: Vec<Action>,
}

/// `gBotGlobals`' parts a bot reads and changes.
pub struct BotGlobals<'a> {
    /// Settings.
    pub config: &'a Config,
    /// The waypoints, their buckets and visibility.
    pub waypoints: &'a mut Waypoints,
    /// `m_Weapons`.
    pub weapons: &'a mut Weapons,
    /// `m_WeaponPresets`.
    pub presets: &'a WeaponPresets,
    /// The team and loadout GAs.
    pub learned: &'a mut GlobalLearnedData,
    /// `m_iTeamScores`.
    pub team_scores: [i32; 4],
    /// `m_Masters`: entities with a `master` key value.
    pub masters: &'a [MasterEntity],
    /// The other bots, as they are while this one thinks (the manager fills it).
    pub other_bots: Vec<OtherBot>,
    /// `m_Squads`.
    pub squads: &'a mut Squads,
    /// `m_BotCam`.
    pub botcam: &'a mut BotCam,
    /// Each player's `CClient` reputation id by entity index ([`NO_CLIENT`] for none).
    pub rep_ids: &'a [i32],
    /// `m_BotChat`.
    pub chat: &'a crate::chat::bot_chat::BotChatMessages,
    /// `gpGlobals->mapname`.
    pub map_name: &'a str,
    /// A danger source to use instead of the learned heat map (`learned.heatmap`) for tactical
    /// paths, goals and looking around ([`BotGlobals::danger_at`]); tests set it, the host leaves
    /// it `None` (port addition, design §4).
    pub danger: Option<&'a dyn crate::nav::DangerSource>,
}

impl BotGlobals<'_> {
    /// How dangerous `waypoint` is for `team`: the test's danger source if any, else the learned
    /// heat map (design §4).
    pub(crate) fn danger_at(&self, waypoint: usize, team: i32) -> f32 {
        match self.danger {
            Some(d) => d.danger(waypoint, team),
            None => self.learned.heatmap.danger(waypoint, team),
        }
    }
}

/// `eBotCondition` bits (`bot_const.h`) the TS bot uses.
pub mod cond {
    /// `BOT_CONDITION_SEE_ENEMY`.
    pub const SEE_ENEMY: u32 = 1 << 0;
    /// `BOT_CONDITION_SEE_ENEMY_HEAD`.
    pub const SEE_ENEMY_HEAD: u32 = 1 << 1;
    /// `BOT_CONDITION_SEE_ENEMY_BODY`.
    pub const SEE_ENEMY_BODY: u32 = 1 << 2;
    /// `BOT_CONDITION_ENEMY_OCCLUDED`.
    pub const ENEMY_OCCLUDED: u32 = 1 << 3;
    /// `BOT_CONDITION_ENEMY_DEAD`.
    pub const ENEMY_DEAD: u32 = 1 << 4;
    /// `BOT_CONDITION_HAS_WEAPON`.
    pub const HAS_WEAPON: u32 = 1 << 5;
    /// `BOT_CONDITION_SEE_NEXT_WAYPOINT`.
    pub const SEE_NEXT_WAYPOINT: u32 = 1 << 6;
    /// `BOT_CONDITION_SEE_TASK_EDICT`.
    pub const SEE_TASK_EDICT: u32 = 1 << 7;
    /// `BOT_CONDITION_TASK_EDICT_NA`.
    pub const TASK_EDICT_NA: u32 = 1 << 8;
    /// `BOT_CONDITION_TASK_EDICT_PAIN`.
    pub const TASK_EDICT_PAIN: u32 = 1 << 9;
    /// `BOT_CONDITION_CANT_SHOOT`.
    pub const CANT_SHOOT: u32 = 1 << 10;
    /// `BOT_CONDITION_SELECTED_GUN`.
    pub const SELECTED_GUN: u32 = 1 << 11;
    /// `BOT_CONDITION_NEED_WEAPONS`.
    pub const NEED_WEAPONS: u32 = 1 << 12;
    /// `BOT_CONDITION_SEE_SQUAD_LEADER`.
    pub const SEE_SQUAD_LEADER: u32 = 1 << 13;
    /// `BOT_CONDITION_SQUAD_LEADER_DEAD`.
    pub const SQUAD_LEADER_DEAD: u32 = 1 << 14;
    /// `BOT_CONDITION_WANT_TO_LEAVE_GAME`.
    pub const WANT_TO_LEAVE_GAME: u32 = 1 << 15;
}

/// `eBotLookTask`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookTask {
    /// `BOT_LOOK_TASK_NONE`.
    None,
    /// `BOT_LOOK_TASK_FACE_NEAREST_REMEMBER_POS`.
    FaceNearestRememberPos,
    /// `BOT_LOOK_TASK_FACE_HURT_ORIGIN`.
    FaceHurtOrigin,
    /// `BOT_LOOK_TASK_NEXT_WAYPOINT`.
    NextWaypoint,
    /// `BOT_LOOK_TASK_LOOK_AROUND`.
    LookAround,
    /// `BOT_LOOK_TASK_SEARCH_FOR_LAST_ENEMY`.
    SearchForLastEnemy,
    /// `BOT_LOOK_TASK_SEARCH_FOR_ENEMY`.
    SearchForEnemy,
    /// `BOT_LOOK_TASK_FACE_ENEMY`.
    FaceEnemy,
    /// `BOT_LOOK_TASK_FACE_TASK_VECTOR`.
    FaceTaskVector,
    /// `BOT_LOOK_TASK_FACE_TASK_EDICT`.
    FaceTaskEdict,
    /// Port addition: look where an enemy may appear next (`preaim.rs`, design §4).
    PreAim,
}

/// `eClimbType`, TS's cases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Climb {
    /// `BOT_CLIMB_NONE`.
    None,
    /// `BOT_CLIMB_CLIMBING`: on a ladder.
    Climbing,
    /// `BOT_CLIMB_NOT_TOUCHING`: heading for a ladder waypoint.
    NotTouching,
}

/// `CBot`: one bot's state.
#[derive(Clone, Debug)]
pub struct Bot {
    /// The bot's player index (`m_pEdict`).
    pub index: u32,
    /// `m_szBotName`.
    pub name: String,
    /// `m_Profile`.
    pub profile: BotProfile,
    /// The learning objects of `setupDataStructures` (`m_GASurvival`, the perceptrons, ...).
    pub learned: BotLearnedData,
    /// The port's learners: the combat ledger and what is scored from it (design §4).
    pub learning: BotLearning,
    /// `m_bKick`: kick the bot next frame.
    pub kick: bool,
    /// `m_bKill`: suicide next frame.
    pub kill: bool,

    pub(crate) me: Entity,
    pub(crate) state: PlayerState,
    pub(crate) button: i32,
    pub(crate) v_angle: Vector,
    pub(crate) ideal_yaw: f32,
    pub(crate) ideal_pitch: f32,

    pub(crate) conditions: u32,
    pub(crate) tasks: Tasks,
    pub(crate) look_task: LookTask,
    pub(crate) look_for_new_tasks: bool,
    /// `WantToFindEnemy()` for this frame (TS: not `BOT_CONFIG_DONT_SHOOT`).
    pub(crate) want_to_find_enemy: bool,
    pub(crate) actions: Vec<Action>,

    pub(crate) think_time: f32,
    pub(crate) last_think_time: f32,
    pub(crate) reaction_time: f32,
    /// The view the game gave the bot in its last command ([`CmdOutcome::game_view`]), taken
    /// as the bot's own at the next frame.
    pub(crate) game_view: Option<Vector>,
    pub(crate) last_update_conditions: f32,

    pub(crate) move_speed: f32,
    pub(crate) strafe_speed: f32,
    pub(crate) up_speed: f32,
    pub(crate) ideal_move_speed: f32,
    pub(crate) max_speed: f32,
    pub(crate) up_time: f32,
    pub(crate) strafe_time: f32,
    pub(crate) move_to: Vector,
    pub(crate) move_to_valid: bool,
    pub(crate) not_following_waypoint: bool,
    pub(crate) avoid_entity: Option<EntRef>,
    pub(crate) avoid_time: f32,
    pub(crate) avoid_move_to: Vector,
    pub(crate) start_duck_time: f32,
    pub(crate) end_duck_time: f32,
    pub(crate) start_jump_time: f32,
    pub(crate) end_jump_time: f32,
    pub(crate) hold_attack_time: f32,
    pub(crate) velocity_3d: f32,
    pub(crate) velocity_2d: f32,
    pub(crate) last_no_move: f32,
    pub(crate) stuck_time: f32,
    /// What the bot tried against getting stuck at the place it is stuck now (design §4).
    pub(crate) unstick: movement::Unstick,
    /// Times the bot killed itself after being stuck for [`movement::STUCK_SUICIDE`] seconds with
    /// nothing else working (for `debug` and the host's log).
    pub stuck_suicides: u32,
    /// When the bot last dived (TS's `+alt1` stunt) on its own: it holds duck through the
    /// landing and gets up if it lands prone (design §4). 0 for none.
    pub(crate) dive_time: f32,

    pub(crate) turn_speed: f32,
    /// `gBotGlobals.m_fTurnSpeed`, what `SetViewAngles` resets the turn speed to.
    pub(crate) base_turn_speed: f32,
    /// `m_vLadderAngles` when `m_bLadderAnglesSet`.
    pub(crate) ladder_angles: Option<Vector>,
    pub(crate) last_look_time: f32,
    pub(crate) current_look_dir: Vector,
    pub(crate) search_enemy_time: f32,
    pub(crate) hurt_origin: Vector,
    pub(crate) hurt_time: f32,

    pub(crate) current_waypoint: i32,
    pub(crate) current_waypoint_flags: u32,
    pub(crate) prev_waypoint: i32,
    pub(crate) goal_waypoint: i32,
    pub(crate) last_failed_waypoint: i32,
    pub(crate) prev_waypoint_dist: f32,
    pub(crate) last_see_waypoint: f32,
    pub(crate) update_waypoint_time: f32,
    pub(crate) find_path_time: f32,
    pub(crate) goal_origin: Option<Vector>,
    pub(crate) astar: AStar,
    pub(crate) paths: AStarBot,
    /// Waypoints touched since spawning (for `debug` and the e2e check).
    pub touched_waypoints: u32,
    /// When the bot was last at each waypoint, for roaming (design §4).
    pub(crate) explore: goals::Explore,

    pub(crate) enemy: Option<EntRef>,
    pub(crate) last_enemy: Option<EntRef>,
    pub(crate) enemy_rep: Option<i32>,
    pub(crate) next_get_enemy_time: f32,
    pub(crate) last_see_enemy_position: Vector,
    pub(crate) last_see_enemy_time: f32,
    pub(crate) first_see_enemy_time: f32,
    pub(crate) get_aim_vector_time: f32,
    pub(crate) zoom: bool,
    pub(crate) distance_from_wall: f32,
    pub(crate) visibles: combat::Visibles,
    pub(crate) vis_update_revs: u32,
    pub(crate) update_visibles_time: f32,
    pub(crate) nearest_avoid_dist: f32,
    pub(crate) range_costs: [[f32; BOT_COST_BUCKETS]; BOT_COST_BUCKETS],
    pub(crate) lowest_enemy_cost: f32,
    pub(crate) lowest_enemy_cost_vec: Vector,
    pub(crate) next_work_range_costs: f32,
    pub(crate) remembered: RememberPositions,
    pub(crate) nearest_remember_point: Vector,
    pub(crate) nearest_remember_point_visible: bool,
    pub(crate) looking_for_enemy: bool,
    pub(crate) listen_to_sound_time: f32,
    pub(crate) investigate_sound_time: f32,
    /// The sounds heard lately (design §4).
    pub(crate) hearing: hearing::Hearing,
    /// Where the bot pre-aims (design §4).
    pub(crate) pre_aim: preaim::PreAim,
    /// Where a moving search goes next (`SEARCH_FOR_ENEMY` with a centre).
    pub(crate) search_point: Option<Vector>,
    /// When the bot may hold a spot again (it roams in between; design §4).
    pub(crate) hold_again: f32,
    /// The objective the bot last tried to complete, and when it should be done by.
    pub(crate) objective_attempt: Option<(i32, f32)>,
    /// Objectives the bot's team couldn't complete, defended instead until the time.
    pub(crate) objective_failed: Vec<(i32, f32)>,
    pub(crate) used_melee: bool,
    pub(crate) prev_health: f32,
    pub(crate) chat_message_time: f32,

    pub(crate) weapons: BotWeapons,
    pub(crate) current_weapon: Option<i32>,
    pub(crate) last_weapon_info: Option<(i32, i32, i32)>,
    /// A weapon switch asked for that the game hasn't shown yet, and until when it is waited
    /// for (the switch isn't asked again meanwhile).
    pub(crate) switch_pending: Option<(i32, f32)>,
    /// When the owned weapons are next read from the game ([`BotWorld::inventory`]); 0 asks for
    /// a read at the next frame.
    pub(crate) next_inventory_sync: f32,
    /// [`PlayerState::inventory_serial`] at the last read.
    pub(crate) inventory_serial: i32,
    /// The time of this frame.
    pub(crate) now: f32,
    /// When the reload key may be pressed again (design §4: never spammed).
    pub(crate) next_reload_time: f32,
    /// The kung fu fighter: its learned stances, reactions and combos, and the fight now.
    pub kungfu: kungfu::KungFu,
    /// The trigger, recoil, aim and fight state the port adds (design §4).
    pub(crate) fight: fire::Fight,
    pub(crate) pickup_entity: Option<EntRef>,
    pub(crate) pickup_item_time: f32,
    /// The map's items as the bot knows them (design §4).
    pub(crate) items: items::ItemMemory,

    pub(crate) spawn_time: f32,
    pub(crate) spawn_position: Vector,
    pub(crate) need_to_init: bool,
    pub(crate) num_deaths: i32,
    pub(crate) frags_since_death: i32,
    pub(crate) prev_team_score: i32,
    pub(crate) respawn_time: f32,
    pub(crate) team: i32,

    pub(crate) squad: Option<SquadId>,
    pub(crate) squad_leader: Option<EntRef>,
    pub(crate) said_move_out: bool,
    pub(crate) said_in_position: bool,
    pub(crate) next_use_say_message: f32,
    /// `m_iBoredom` (0-255).
    pub boredom: u8,
    /// `m_fJoinServerTime`.
    pub join_server_time: f32,
    /// `m_fLeaveTime`.
    pub leave_time: f32,
    pub(crate) said_greetings_to: u32,
    pub(crate) chat_string: Vec<u8>,
    /// `m_Profile.m_HAL`: the MegaHAL brain, once loaded.
    pub hal: Option<crate::chat::reply::HalBot>,
}

impl Bot {
    /// `CBot::Init` for a new bot in slot `index`.
    pub fn new(index: u32, name: String, profile: BotProfile, config: &Config, rng: &mut dyn Random) -> Bot {
        Bot {
            index,
            name,
            profile,
            learned: BotLearnedData::new(rng),
            learning: BotLearning::default(),
            kick: false,
            kill: false,
            me: Entity::default(),
            state: PlayerState::default(),
            button: 0,
            v_angle: Vector::ZERO,
            ideal_yaw: 0.0,
            ideal_pitch: 0.0,
            conditions: 0,
            tasks: Tasks::default(),
            look_task: LookTask::None,
            look_for_new_tasks: true,
            want_to_find_enemy: true,
            actions: Vec::new(),
            think_time: BOT_DEFAULT_THINK_TIME,
            last_think_time: 0.0,
            reaction_time: BOT_DEFAULT_REACTION_TIME,
            game_view: None,
            last_update_conditions: 0.0,
            move_speed: 0.0,
            strafe_speed: 0.0,
            up_speed: 0.0,
            ideal_move_speed: 0.0,
            max_speed: 290.0,
            up_time: 0.0,
            strafe_time: 0.0,
            move_to: Vector::ZERO,
            move_to_valid: false,
            not_following_waypoint: false,
            avoid_entity: None,
            avoid_time: 0.0,
            avoid_move_to: Vector::ZERO,
            start_duck_time: 0.0,
            end_duck_time: 0.0,
            start_jump_time: 0.0,
            end_jump_time: 0.0,
            hold_attack_time: 0.0,
            velocity_3d: 0.0,
            velocity_2d: 0.0,
            last_no_move: 0.0,
            stuck_time: 0.0,
            unstick: movement::Unstick::default(),
            stuck_suicides: 0,
            dive_time: 0.0,
            turn_speed: config.turn_speed,
            base_turn_speed: config.turn_speed,
            ladder_angles: None,
            last_look_time: 0.0,
            current_look_dir: Vector::ZERO,
            search_enemy_time: 0.0,
            hurt_origin: Vector::ZERO,
            hurt_time: 0.0,
            current_waypoint: -1,
            current_waypoint_flags: 0,
            prev_waypoint: -1,
            goal_waypoint: -1,
            last_failed_waypoint: -1,
            prev_waypoint_dist: 0.0,
            last_see_waypoint: 0.0,
            update_waypoint_time: 0.0,
            find_path_time: 0.0,
            goal_origin: None,
            astar: AStar::new(),
            paths: AStarBot::default(),
            touched_waypoints: 0,
            explore: goals::Explore::default(),
            enemy: None,
            last_enemy: None,
            enemy_rep: None,
            next_get_enemy_time: 0.0,
            last_see_enemy_position: Vector::ZERO,
            last_see_enemy_time: 0.0,
            first_see_enemy_time: 0.0,
            get_aim_vector_time: 0.0,
            zoom: false,
            distance_from_wall: 0.0,
            visibles: combat::Visibles::default(),
            vis_update_revs: 0,
            update_visibles_time: 0.0,
            nearest_avoid_dist: 0.0,
            range_costs: [[0.0; BOT_COST_BUCKETS]; BOT_COST_BUCKETS],
            lowest_enemy_cost: 99999.0,
            lowest_enemy_cost_vec: Vector::ZERO,
            next_work_range_costs: 0.0,
            remembered: RememberPositions::default(),
            nearest_remember_point: Vector::ZERO,
            nearest_remember_point_visible: false,
            looking_for_enemy: false,
            listen_to_sound_time: 0.0,
            investigate_sound_time: 0.0,
            hearing: hearing::Hearing::default(),
            pre_aim: preaim::PreAim::default(),
            search_point: None,
            hold_again: 0.0,
            objective_attempt: None,
            objective_failed: Vec::new(),
            used_melee: false,
            prev_health: 0.0,
            chat_message_time: 0.0,
            weapons: BotWeapons::default(),
            current_weapon: None,
            last_weapon_info: None,
            switch_pending: None,
            next_inventory_sync: 0.0,
            inventory_serial: 0,
            now: 0.0,
            next_reload_time: 0.0,
            kungfu: kungfu::KungFu::new(),
            fight: fire::Fight::default(),
            pickup_entity: None,
            pickup_item_time: 0.0,
            items: items::ItemMemory::default(),
            spawn_time: 0.0,
            spawn_position: Vector::ZERO,
            need_to_init: false,
            num_deaths: 0,
            frags_since_death: 0,
            prev_team_score: 0,
            respawn_time: 0.0,
            team: -1,
            squad: None,
            squad_leader: None,
            said_move_out: false,
            said_in_position: false,
            next_use_say_message: 0.0,
            boredom: 127,
            join_server_time: 0.0,
            leave_time: 0.0,
            said_greetings_to: 0,
            chat_string: Vec::new(),
            hal: None,
        }
    }

    pub(crate) fn has(&self, c: u32) -> bool {
        self.conditions & c != 0
    }

    pub(crate) fn set(&mut self, c: u32) {
        self.conditions |= c;
    }

    pub(crate) fn clear(&mut self, c: u32) {
        self.conditions &= !c;
    }

    /// `GetGunPosition`: `origin + view_ofs`.
    pub(crate) fn gun_position(&self) -> Vector {
        self.me.origin + self.me.view_ofs
    }

    /// `DistanceFrom(vOrigin, twoD)`, measured from the gun position.
    pub(crate) fn distance_from(&self, v: Vector, two_d: bool) -> f32 {
        let d = v - self.gun_position();
        if two_d { d.length_2d() } else { d.length() }
    }

    /// `IsOnLadder`.
    pub(crate) fn is_on_ladder(&self) -> bool {
        self.me.movetype == MOVETYPE_FLY
    }

    /// `IsAlive`.
    pub fn is_alive(&self) -> bool {
        self.me.is_alive() && self.state.alive
    }

    pub(crate) fn jump(&mut self) {
        self.button |= buttons::JUMP;
    }

    pub(crate) fn duck(&mut self) {
        self.button |= buttons::DUCK;
    }

    pub(crate) fn use_button(&mut self) {
        self.button |= buttons::USE;
    }

    pub(crate) fn alt_button(&mut self) {
        self.button |= buttons::ALT1;
    }

    /// `Reload`, guarded (design §4): only a gun whose clip has room and whose reserve has
    /// rounds, and once per [`RELOAD_RETRY`] (or sooner when the clip changes), so the key is
    /// never spammed. With kung fu or a blade TS's reload key is the block (`CCombatBlock`),
    /// which [`kungfu`] presses on its own.
    pub(crate) fn reload(&mut self, now: f32) {
        let Some(wp) = self.current_weapon.and_then(|id| self.weapons.get(id)) else { return };
        if wp.is_melee() || wp.reserve <= 0 || (wp.max_clip > 0 && wp.clip >= wp.max_clip) || self.next_reload_time > now {
            return;
        }
        self.next_reload_time = now + RELOAD_RETRY;
        self.button |= buttons::RELOAD;
    }

    /// `PrimaryAttack`: nothing while reloading.
    pub(crate) fn primary_attack(&mut self) {
        if self.button & buttons::RELOAD == 0 {
            self.button |= buttons::ATTACK;
        }
    }

    /// `SecondaryAttack`.
    pub(crate) fn secondary_attack(&mut self) {
        self.button |= buttons::ATTACK2;
    }

    /// `StopMoving`.
    pub(crate) fn stop_moving(&mut self) {
        self.move_to_valid = false;
    }

    /// `SetMoveVector`.
    pub(crate) fn set_move_vector(&mut self, v: Vector) {
        self.move_to = v;
        self.move_to_valid = true;
    }

    /// `DuckAndJump`.
    pub(crate) fn duck_and_jump(&mut self, now: f32) {
        self.start_jump_time = now + 0.1;
        self.end_jump_time = now + 0.2;
        self.start_duck_time = now;
        self.end_duck_time = now + 0.2;
    }

    /// `JumpAndDuck`.
    pub(crate) fn jump_and_duck(&mut self, now: f32) {
        if self.end_jump_time < now {
            self.start_jump_time = now;
            self.end_jump_time = now + 0.1;
        }
        if self.end_duck_time < now {
            self.start_duck_time = now + 0.1;
            self.end_duck_time = now + 0.7;
        }
    }

    /// `util bot_goto`: `ClearTasks` (`m_OrderTask` is TFC's and NS's) and a priority
    /// `BOT_TASK_FIND_PATH` to `waypoint` (schedule 0, float -2; -1 is any goal).
    pub fn go_to_waypoint(&mut self, waypoint: i32) {
        self.tasks.flush();
        self.add_priority_task(Task::new(TaskKind::FindPath, 0, None, waypoint, -2.0, Vector::ZERO));
    }

    /// `AddTask`.
    pub(crate) fn add_task(&mut self, t: Task) {
        self.tasks.add_to_end(t);
    }

    /// `AddPriorityTask`.
    pub(crate) fn add_priority_task(&mut self, t: Task) {
        self.tasks.add_priority(t);
    }

    /// The live entity behind a handle (`MyEHandle::Get`): `None` once its slot holds another.
    pub(crate) fn resolve(&self, w: &dyn BotWorld, e: Option<EntRef>) -> Option<Entity> {
        let e = e?;
        w.entity(e.index).filter(|x| x.serial == e.serial)
    }

    /// `GetNextWaypoint`: the waypoint after the current one on the path, or the current one.
    pub(crate) fn next_waypoint(&self) -> i32 {
        let mut next = self.current_waypoint;
        for &w in self.paths.bot_paths.iter().rev() {
            if next != self.current_waypoint {
                break;
            }
            next = w as i32;
        }
        next
    }

    /// `GetLadderDir(bCheckWaypoint)`: 1 up, -1 down, 0 stay.
    pub(crate) fn ladder_dir(&self, check_waypoint: bool, waypoints: &Waypoints) -> i32 {
        let dir = |target: f32, current: f32| {
            if target > current - 8.0 {
                1
            } else if target < current {
                -1
            } else {
                0
            }
        };
        if check_waypoint {
            let next = self.next_waypoint();
            if next != -1 {
                return dir(waypoints.origin(next).z, self.me.origin.z);
            }
        }
        dir(self.move_to.z, self.me.origin.z)
    }

    /// `GetClimbType`, TS's cases.
    pub(crate) fn climb_type(&self) -> Climb {
        if self.is_on_ladder() {
            return Climb::Climbing;
        }
        if self.current_waypoint != -1 && self.current_waypoint_flags & crate::nav::flags::LADDER != 0 {
            return Climb::NotTouching;
        }
        Climb::None
    }

    /// The tasks, current first.
    pub fn tasks(&self) -> &Tasks {
        &self.tasks
    }

    /// `m_iCurrentWaypointIndex`.
    pub fn current_waypoint(&self) -> i32 {
        self.current_waypoint
    }

    /// `m_iWaypointGoalIndex`.
    pub fn goal_waypoint(&self) -> i32 {
        self.goal_waypoint
    }

    /// The enemy the bot is after.
    pub fn enemy(&self) -> Option<u32> {
        self.enemy.map(|e| e.index)
    }

    /// `m_pCurrentWeapon`'s id: the weapon in hand as the game last reported it.
    pub fn current_weapon(&self) -> Option<i32> {
        self.current_weapon
    }

    /// The game refused to take out weapon `game_id` (TS's id, 0 kung fu): the bot doesn't own
    /// it. It is forgotten and the owned weapons are read again next frame.
    pub fn weapon_select_failed(&mut self, game_id: i32) {
        let id = crate::weapons::TsWeapon::from_game_id(game_id).map_or(game_id, crate::weapons::TsWeapon::id);
        if id != crate::weapons::TsWeapon::KungFu.id() {
            self.weapons.remove(id);
        }
        self.switch_pending = None;
        self.next_inventory_sync = 0.0;
    }

    /// `BotEvent(BOT_EVENT_HURT)` for the port's damage report ([`crate::learn::DamageEvent`]): the
    /// bot turns to where the shot came from, along the shot's direction (`dir`, what a player hit
    /// by an unseen shooter can tell; it doesn't learn where the shooter stands), else toward the
    /// attacker (damage without a trace: a grenade, a blow).
    pub fn hurt_by(&mut self, w: &dyn BotWorld, d: &crate::learn::DamageEvent) {
        let from = if d.dir != Vector::ZERO {
            Some(self.gun_position() - d.dir.normalize() * 512.0)
        } else {
            w.entity(d.attacker).map(|a| a.origin)
        };
        if let Some(from) = from {
            self.hurt(Some(d.attacker), from, w.time());
        }
    }

    /// `BotEvent(BOT_EVENT_HURT, attacker, origin)`: the port reports damage (`damage`)
    /// instead of upstream's "health went down" guess in `Think`'s TS block (design §4). Hurt by
    /// someone other than its enemy, the bot weighs its enemy again at once, the attacker scoring
    /// higher, in sight or not (design §4; upstream ignored damage while it had an enemy).
    pub fn hurt(&mut self, attacker: Option<u32>, from: Vector, now: f32) {
        self.hurt_origin = from;
        self.hurt_time = now;
        if let Some(a) = attacker {
            self.fight.attacker = Some((a, now));
            if self.enemy.is_some_and(|e| e.index != a) {
                self.next_get_enemy_time = 0.0;
            }
        }
        if self.enemy.is_none() && matches!(self.look_task, LookTask::NextWaypoint | LookTask::LookAround | LookTask::None) {
            self.look_task = LookTask::FaceHurtOrigin;
        }
    }
}
