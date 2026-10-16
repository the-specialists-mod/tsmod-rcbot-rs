//! The bot's view of the server: `rcbot::nav::World` and `rcbot::bot::BotWorld` on the engine's
//! functions and the plugin API, and `rcbot::Random` on the engine's generator (bots share its
//! sequence, as upstream's `RANDOM_LONG` does).

use crate::api::{Api, Engine};
use rcbot::bot::{BotWorld, Entity, EntityKey, GameMode, ItemInfo, ItemKind, PlayerState};
use rcbot::nav::{Glass, Monsters, Trace, World};
use rcbot::{Random, Vector};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// `NON_TS_TEAMPLAY`: the map's name starts with `tm_` (`CBotGlobals::MapInit`), set at
/// `ServerActivate`.
pub static NON_TS_TEAMPLAY: AtomicBool = AtomicBool::new(false);

/// The game's teamplay (`GameState::teamplay`) as of this frame ([`refresh_game_state`]).
static GAME_TEAMPLAY: AtomicBool = AtomicBool::new(false);
/// The game's mode, CTB and round as of this frame (`mode | ctb << 8 | round << 16`).
static GAME_MODE: AtomicI32 = AtomicI32::new(0);

/// Reads the game's state once for the frame (the server scans the entities for it): the
/// worlds made until the next refresh, callbacks included, use its teamplay and mode.
pub fn refresh_game_state(api: Api) -> ts_plugin_api::GameState {
    let g = api.game_state();
    GAME_TEAMPLAY.store(g.teamplay != 0, Ordering::Relaxed);
    GAME_MODE.store((g.mode & 0xff) | (i32::from(g.ctb != 0) << 8) | ((g.round & 0xff) << 16), Ordering::Relaxed);
    g
}

/// What `entity_info` says about an item, for the classes a bot looks at (`ts_*`, `weapon_*`).
fn item_info(api: Api, index: u32, classname: &str) -> Option<ItemInfo> {
    use ts_plugin_api::entity;
    if !classname.starts_with("ts_") && !classname.starts_with("weapon") {
        return None;
    }
    let i = api.entity_info(index as i32)?;
    let kind = match i.kind {
        entity::GROUND_WEAPON => ItemKind::GroundWeapon,
        entity::POWERUP => ItemKind::Powerup,
        entity::PROJECTILE => ItemKind::Projectile,
        entity::GUN => ItemKind::Gun,
        entity::CTB_FLAG => ItemKind::CtbFlag,
        entity::CTB_BASE => ItemKind::CtbBase,
        entity::OBJECTIVE => ItemKind::Objective,
        _ => ItemKind::Other,
    };
    Some(ItemInfo {
        kind,
        weapon: i.weapon,
        powerup: i.powerup,
        owner: u32::try_from(i.owner).unwrap_or(0),
        team: i.team,
        available: i.available != 0,
    })
}

/// A cached world's slot for one index: `None` not read yet, `Some(None)` no entity there.
type Slot = Option<Option<Rc<Entity>>>;

/// The world for one frame.
pub struct HostWorld {
    pub api: Api,
    pub eng: Engine,
    /// The game's teamplay (`GameState::teamplay`).
    pub game_teamplay: bool,
    /// The cvars read through this world: a world lives for a frame, and every bot asks for
    /// the same few (`sv_gravity`, `mp_timelimit`).
    cvars: RefCell<Vec<(String, f32)>>,
    /// The entities read through a [`cached`](Self::cached) world, by index.
    entities: Option<RefCell<Vec<Slot>>>,
}

impl HostWorld {
    /// The world now, with the game's teamplay as of this frame.
    pub fn new(api: Api, eng: Engine) -> HostWorld {
        HostWorld { api, eng, game_teamplay: GAME_TEAMPLAY.load(Ordering::Relaxed), cvars: RefCell::new(Vec::new()), entities: None }
    }

    /// A world that reads each entity (its strings, a player's `player_info`) once and hands out
    /// copies: for the bots' thinks, while nothing changes the entities. Anything that writes
    /// entvars needs a world of its own made after.
    pub fn cached(api: Api, eng: Engine) -> HostWorld {
        HostWorld { entities: Some(RefCell::new(Vec::new())), ..HostWorld::new(api, eng) }
    }

    /// The entity at `index` as the engine and the game have it now.
    fn read_entity(&self, index: u32) -> Option<Entity> {
        let e = self.eng.edict(index as i32)?;
        let mut ent = self.eng.vars(e, |v, serial| Entity {
            index,
            serial,
            classname: String::new(),
            netname: String::new(),
            origin: v.origin.into(),
            view_ofs: v.view_ofs.into(),
            absmin: v.absmin.into(),
            absmax: v.absmax.into(),
            size: v.size.into(),
            velocity: v.velocity.into(),
            basevelocity: v.basevelocity.into(),
            angles: v.angles.into(),
            v_angle: v.v_angle.into(),
            flags: v.flags,
            health: v.health,
            max_health: v.max_health,
            spawnflags: v.spawnflags,
            target: String::new(),
            targetname: String::new(),
            movedir: v.movedir.into(),
            avelocity: v.avelocity.into(),
            frame: v.frame,
            deadflag: v.deadflag,
            takedamage: v.takedamage,
            effects: v.effects,
            solid: v.solid,
            movetype: v.movetype,
            waterlevel: v.waterlevel,
            maxspeed: v.maxspeed,
            button: v.button,
            frags: v.frags,
            owner: None,
            groundentity: None,
            player: None,
            item: None,
        });
        let (classname, netname, target, targetname, ground, owner) =
            self.eng.vars(e, |v, _| (v.classname, v.netname, v.target, v.targetname, v.groundentity, v.owner));
        ent.classname = self.eng.string(classname);
        ent.netname = self.eng.string(netname);
        ent.target = self.eng.string(target);
        ent.targetname = self.eng.string(targetname);
        ent.groundentity = (!ground.is_null()).then(|| self.eng.index_of(ground)).and_then(|i| u32::try_from(i).ok());
        ent.owner = (!owner.is_null()).then(|| self.eng.index_of(owner)).and_then(|i| u32::try_from(i).ok());
        if index >= 1 && index <= BotWorld::max_clients(self) {
            // A player's handle is its user id, not the edict's serial number: the engine keeps
            // a client slot's edict (and its serial) for the next player in it, who must not
            // inherit the last one's enemies, tasks or squad (`EntRef`).
            ent.serial = self.eng.user_id_of(e);
            ent.player = self.api.player_info(index as i32).map(|p| PlayerState {
                joined: p.joined != 0,
                alive: p.alive != 0,
                is_bot: p.is_bot != 0,
                team: p.team,
                weapon: p.weapon,
                clip: p.clip,
                ammo: p.ammo,
                stunt_state: p.stunt_state,
                fire_mode: p.fire_mode,
                attachments: p.attachments,
                free_slots: p.free_slots,
                active_powerup: p.active_powerup,
                powerup_until: p.powerup_until,
                stored_powerup: p.stored_powerup,
                time_scale: p.time_scale,
                kungfu: p.kungfu != 0,
                has_flag: p.has_flag != 0,
            });
        } else {
            ent.item = item_info(self.api, index, &ent.classname);
        }
        Some(ent)
    }

    /// Teams by `model` key: a `tm_` map the game doesn't run as teamplay.
    fn model_teams(&self) -> bool {
        !self.game_teamplay && NON_TS_TEAMPLAY.load(Ordering::Relaxed)
    }
}

impl World for HostWorld {
    fn trace_line(&self, start: Vector, end: Vector, monsters: Monsters, glass: Glass, ignore: Option<u32>) -> Trace {
        let (fraction, end_pos, hit) =
            self.eng.trace_line(start, end, monsters == Monsters::Ignore, glass == Glass::Ignore, ignore.map(|i| i as i32));
        let hit = (!hit.is_null()).then(|| self.eng.index_of(hit)).and_then(|i| u32::try_from(i).ok());
        Trace { fraction, end_pos, hit }
    }

    fn point_contents(&self, at: Vector) -> i32 {
        self.eng.point_contents(at)
    }
}

impl BotWorld for HostWorld {
    fn time(&self) -> f32 {
        self.eng.time()
    }

    fn frame_time(&self) -> f32 {
        self.eng.frame_time()
    }

    fn max_entities(&self) -> u32 {
        self.eng.max_entities().max(1) as u32 - 1
    }

    fn max_clients(&self) -> u32 {
        self.eng.max_clients().max(0) as u32
    }

    fn entity(&self, index: u32) -> Option<Entity> {
        let Some(cache) = &self.entities else { return self.read_entity(index) };
        let i = index as usize;
        if let Some(Some(known)) = cache.borrow().get(i) {
            return known.as_deref().cloned();
        }
        let ent = self.read_entity(index);
        let mut c = cache.borrow_mut();
        if c.len() <= i {
            c.resize(i + 1, None);
        }
        c[i] = Some(ent.clone().map(Rc::new));
        ent
    }

    fn entity_glance(&self, index: u32) -> Option<(Vector, i32)> {
        let e = self.eng.edict(index as i32)?;
        Some(self.eng.vars(e, |v, _| {
            let origin = if v.flags & rcbot::bot::entflags::MONSTER != 0 {
                Vector::from(v.origin) + Vector::from(v.view_ofs) * 0.5
            } else {
                Vector::from(v.absmin) + Vector::from(v.size) * 0.5
            };
            (origin, v.effects)
        }))
    }

    fn in_pvs(&self, viewer: u32, origin: Vector) -> bool {
        self.eng.in_pvs(viewer as i32, origin)
    }

    fn entities_in_sphere(&self, origin: Vector, radius: f32) -> Vec<u32> {
        self.eng.entities_in_sphere(origin, radius).into_iter().filter_map(|i| u32::try_from(i).ok()).collect()
    }

    fn teamplay(&self) -> bool {
        self.game_teamplay || NON_TS_TEAMPLAY.load(Ordering::Relaxed)
    }

    fn model_teams(&self) -> bool {
        HostWorld::model_teams(self)
    }

    fn game_mode(&self) -> GameMode {
        let m = GAME_MODE.load(Ordering::Relaxed);
        GameMode { mode: m & 0xff, ctb: m & 0x100 != 0, round: (m >> 16) & 0xff }
    }

    /// The HUD's objective list as the bot reads it (its team's texts), the ones that can still
    /// be completed (design §4).
    fn ts_objectives(&self, index: u32) -> Vec<rcbot::bot::TsObjective> {
        self.api
            .objectives(index as i32)
            .into_iter()
            .filter(|o| o.active != 0)
            .map(|o| rcbot::bot::TsObjective {
                id: o.id,
                entity: u32::try_from(o.entity).ok().filter(|&e| e > 0),
                origin: Vector::from(o.origin),
                name: o.name.iter().take_while(|&&c| c != 0).map(|&c| c as u8 as char).collect(),
            })
            .collect()
    }

    fn inventory(&self, index: u32) -> Option<Vec<rcbot::bot::InventoryWeapon>> {
        let owned = self.api.player_inventory(index as i32)?;
        Some(owned.into_iter().map(|o| rcbot::bot::InventoryWeapon { weapon: o.weapon, clip: o.clip, ammo: o.ammo }).collect())
    }

    fn player_model(&self, index: u32) -> String {
        self.eng.edict(index as i32).map(|e| self.eng.info_key(e, c"model")).unwrap_or_default()
    }

    fn find_entities(&self, key: EntityKey, value: &str) -> Vec<u32> {
        let field = match key {
            EntityKey::Classname => "classname",
            EntityKey::Target => "target",
            EntityKey::Targetname => "targetname",
        };
        self.eng.find_entities(field, value).into_iter().filter_map(|i| u32::try_from(i).ok()).collect()
    }

    fn trace_point_hull(&self, start: Vector, end: Vector, monsters: Monsters, ignore: Option<u32>) -> Trace {
        // `point_hull` is hull 0.
        let (fraction, end_pos, hit) = self.eng.trace_hull(start, end, monsters == Monsters::Ignore, 0, ignore.map(|i| i as i32));
        let hit = (!hit.is_null()).then(|| self.eng.index_of(hit)).and_then(|i| u32::try_from(i).ok());
        Trace { fraction, end_pos, hit }
    }

    fn cvar_float(&self, name: &str) -> f32 {
        if let Some(&(_, v)) = self.cvars.borrow().iter().find(|(n, _)| n == name) {
            return v;
        }
        let v = self.eng.cvar_float(name);
        self.cvars.borrow_mut().push((name.to_owned(), v));
        v
    }
}

/// `RANDOM_LONG` / `RANDOM_FLOAT`.
pub struct EngineRandom(pub Engine);

impl Random for EngineRandom {
    fn random_long(&mut self, lo: i32, hi: i32) -> i32 {
        self.0.random_long(lo, hi)
    }

    fn random_float(&mut self, lo: f32, hi: f32) -> f32 {
        self.0.random_float(lo, hi)
    }
}
