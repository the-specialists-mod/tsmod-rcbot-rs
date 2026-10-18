//! The bot in a fake world: an open plane where the usercmd moves the bot directly.

use super::look::make_vectors;
use super::*;
use crate::XorShiftRandom;
use crate::nav::{Glass, Monsters, Trace, flags as fl};
use crate::weapons::TsWeapon;
use std::cell::RefCell;

/// The fight: trigger, aim, movement and weapon choice (design §4).
mod fight;

struct Plane {
    time: f32,
    ents: RefCell<Vec<Entity>>,
    teamplay: bool,
    /// Players' `model` keys for a `tm_` map (teams by model), by index.
    models: Option<Vec<(u32, &'static str)>>,
    /// A wall of the world on the plane `x = wall`, when set.
    wall: std::cell::Cell<Option<f32>>,
    /// Jumps, gravity and TS's wall jump in [`Plane::apply`] (off: the bot never leaves the
    /// ground).
    physics: std::cell::Cell<bool>,
    /// What [`BotWorld::inventory`] answers (`None`: the world can't tell).
    inventory: RefCell<Option<Vec<InventoryWeapon>>>,
    /// When [`Plane::apply`] last ran: a usercmd lasts the time since (the server's `AUTO_MSEC`).
    last_cmd: std::cell::Cell<f32>,
    /// What [`BotWorld::highest_entity`] answers (0: `max_entities`).
    highest: std::cell::Cell<u32>,
    /// How often [`BotWorld::inventory`] was asked.
    inventory_reads: std::cell::Cell<u32>,
    /// What [`BotWorld::map_items`] answers.
    map_items: RefCell<Option<Vec<MapItem>>>,
}

fn player(index: u32, origin: Vector, team: i32) -> Entity {
    Entity {
        index,
        serial: 1,
        classname: "player".into(),
        netname: format!("p{index}"),
        origin,
        view_ofs: Vector::new(0.0, 0.0, 17.0),
        absmin: origin - Vector::new(16.0, 16.0, 36.0),
        absmax: origin + Vector::new(16.0, 16.0, 36.0),
        size: Vector::new(32.0, 32.0, 72.0),
        flags: entflags::CLIENT | entflags::ONGROUND,
        health: 100.0,
        max_health: 100.0,
        takedamage: 2.0,
        movetype: MOVETYPE_WALK,
        solid: 3,
        maxspeed: 290.0,
        player: Some(PlayerState { joined: true, alive: true, team, weapon: 1, clip: 17, ammo: 34, ..PlayerState::default() }),
        ..Entity::default()
    }
}

impl World for Plane {
    fn trace_line(&self, start: Vector, end: Vector, _: Monsters, _: Glass, _: Option<u32>) -> Trace {
        if let Some(x) = self.wall.get()
            && (start.x < x) != (end.x < x)
        {
            let f = (x - start.x) / (end.x - start.x);
            return Trace { fraction: f, end_pos: start + (end - start) * f, hit: Some(0) };
        }
        Trace::clear(end)
    }
    fn point_contents(&self, _: Vector) -> i32 {
        -1
    }
}

impl BotWorld for Plane {
    fn time(&self) -> f32 {
        self.time
    }
    fn frame_time(&self) -> f32 {
        0.02
    }
    fn max_entities(&self) -> u32 {
        8
    }
    fn max_clients(&self) -> u32 {
        4
    }
    fn entity(&self, index: u32) -> Option<Entity> {
        self.ents.borrow().iter().find(|e| e.index == index).cloned()
    }
    fn in_pvs(&self, _: u32, _: Vector) -> bool {
        true
    }
    fn entities_in_sphere(&self, o: Vector, r: f32) -> Vec<u32> {
        self.ents.borrow().iter().filter(|e| (e.origin - o).length() < r).map(|e| e.index).collect()
    }
    fn teamplay(&self) -> bool {
        self.teamplay || self.models.is_some()
    }
    fn model_teams(&self) -> bool {
        self.models.is_some()
    }
    fn inventory(&self, _: u32) -> Option<Vec<InventoryWeapon>> {
        self.inventory_reads.set(self.inventory_reads.get() + 1);
        self.inventory.borrow().clone()
    }
    fn map_items(&self) -> Option<Vec<MapItem>> {
        self.map_items.borrow().clone()
    }
    fn highest_entity(&self) -> u32 {
        match self.highest.get() {
            0 => self.max_entities(),
            h => h,
        }
    }
    fn find_entities(&self, key: EntityKey, value: &str) -> Vec<u32> {
        let ents = self.ents.borrow();
        let hit = |e: &&Entity| match key {
            EntityKey::Classname => e.classname == value,
            EntityKey::Target => e.target == value,
            EntityKey::Targetname => e.targetname == value,
        };
        ents.iter().filter(hit).map(|e| e.index).collect()
    }
    fn player_model(&self, index: u32) -> String {
        self.models.iter().flatten().find(|(i, _)| *i == index).map_or(String::new(), |(_, m)| (*m).into())
    }
}

impl Plane {
    /// Moves the bot as `pfnRunPlayerMove` roughly would.
    fn apply(&self, index: u32, cmd: &BotCmd) {
        let mut ents = self.ents.borrow_mut();
        let e = ents.iter_mut().find(|e| e.index == index).unwrap();
        e.v_angle = cmd.angles;
        e.button = cmd.buttons;
        let (fwd, right) = make_vectors(Vector::new(0.0, cmd.angles.y, 0.0));
        // The server's `AUTO_MSEC`: the time since the last command (a frame at first), at most
        // a byte of milliseconds.
        let last = self.last_cmd.replace(self.time);
        let dt = if last > 0.0 && last < self.time { (self.time - last).min(0.255) } else { 0.02 };
        if self.physics.get() {
            if e.flags & entflags::ONGROUND == 0 {
                // In the air: keep going, fall, land at z = 0.
                e.velocity.z -= 800.0 * dt;
                e.origin = e.origin + e.velocity * dt;
                if e.origin.z <= 0.0 {
                    e.origin.z = 0.0;
                    e.velocity.z = 0.0;
                    e.flags |= entflags::ONGROUND;
                }
                return;
            }
            // `CheckForWallJump`: a wall within 48 units ahead; `TSWallJump` throws the player
            // back off it at 210 units/s.
            let wall_ahead = self.wall.get().is_some_and(|x| (e.origin.x < x) != (e.origin.x + fwd.x * 48.0 < x));
            if cmd.buttons & buttons::ALT1 != 0 && wall_ahead {
                e.velocity = Vector::new(-fwd.x * 210.0, -fwd.y * 210.0, 300.0);
                e.flags &= !entflags::ONGROUND;
                e.origin = e.origin + e.velocity * dt;
                return;
            }
            if cmd.buttons & buttons::JUMP != 0 {
                let mut v = fwd * cmd.forward + right * cmd.side;
                v.z = 270.0;
                e.velocity = v;
                e.flags &= !entflags::ONGROUND;
                e.origin = e.origin + e.velocity * dt;
                return;
            }
        }
        let mut v = fwd * cmd.forward + right * cmd.side;
        v.z = 0.0;
        let speed = v.length();
        if speed > 290.0 {
            v = v * (290.0 / speed);
        }
        e.velocity = v;
        e.origin = e.origin + v * dt;
        e.absmin = e.origin - Vector::new(16.0, 16.0, 36.0);
        e.absmax = e.origin + Vector::new(16.0, 16.0, 36.0);
    }
}

fn setup(enemy: bool) -> (Plane, Bot, Waypoints, Weapons, WeaponPresets, GlobalLearnedData, Config) {
    let mut wp = Waypoints::new();
    for i in 0..6 {
        wp.add_origin(Vector::new(i as f32 * 150.0, 0.0, 0.0), 0, None);
    }
    for i in 0..5 {
        wp.add_path(i, (i + 1) as i16);
        wp.add_path(i + 1, i as i16);
    }
    let mut ents = vec![player(1, Vector::new(0.0, 0.0, 0.0), 0)];
    if enemy {
        ents.push(player(2, Vector::new(400.0, 0.0, 0.0), 0));
    }
    let plane = Plane {
        time: 1.0,
        ents: RefCell::new(ents),
        teamplay: false,
        models: None,
        wall: Default::default(),
        physics: Default::default(),
        inventory: Default::default(),
        last_cmd: Default::default(),
        highest: Default::default(),
        inventory_reads: Default::default(),
        map_items: Default::default(),
    };
    let config = Config::default();
    let mut rng = XorShiftRandom::new(7);
    let profile = crate::config::BotProfile::init(&config, &mut rng);
    let bot = Bot::new(1, "Test".into(), profile, &config, &mut rng);
    let presets = WeaponPresets::parse(include_str!("../../../../data/rcbot/bot_weapons.ini"), crate::weapons::MOD_TS).0;
    (plane, bot, wp, Weapons::default(), presets, GlobalLearnedData::default(), config)
}

/// The rest of `gBotGlobals` a frame needs.
struct Extra {
    squads: Squads,
    cam: BotCam,
    chat: crate::chat::bot_chat::BotChatMessages,
    reps: Vec<i32>,
}

impl Default for Extra {
    fn default() -> Self {
        Extra { squads: Squads::default(), cam: BotCam::default(), chat: Default::default(), reps: vec![NO_CLIENT; 33] }
    }
}

#[test]
fn walks_along_waypoints() {
    let (mut plane, mut bot, mut wp, mut weapons, presets, mut learned, config) = setup(false);
    let mut rng = XorShiftRandom::new(3);
    let mut extra = Extra::default();
    let mut moved = 0.0f32;
    for _ in 0..600 {
        plane.time += 0.02;
        let mut g = BotGlobals {
            config: &config,
            waypoints: &mut wp,
            weapons: &mut weapons,
            presets: &presets,
            learned: &mut learned,
            team_scores: [0; 4],
            masters: &[],
            other_bots: Vec::new(),
            squads: &mut extra.squads,
            botcam: &mut extra.cam,
            rep_ids: &extra.reps,
            chat: &extra.chat,
            map_name: "test",
            danger: None,
        };
        let f = bot.frame(&mut g, &plane, &mut rng);
        if let Some(cmd) = f.cmd {
            plane.apply(1, &cmd);
        }
        moved = moved.max(plane.entity(1).unwrap().origin.length());
    }
    assert!(moved > 200.0, "moved {moved}");
    assert!(bot.touched_waypoints >= 2, "touched {}", bot.touched_waypoints);
    let _ = fl::JUMP;
}

#[test]
fn attacks_a_visible_enemy() {
    let (mut plane, mut bot, mut wp, mut weapons, presets, mut learned, config) = setup(true);
    let mut rng = XorShiftRandom::new(5);
    let mut extra = Extra::default();
    let mut attacked = false;
    for _ in 0..300 {
        plane.time += 0.02;
        let mut g = BotGlobals {
            config: &config,
            waypoints: &mut wp,
            weapons: &mut weapons,
            presets: &presets,
            learned: &mut learned,
            team_scores: [0; 4],
            masters: &[],
            other_bots: Vec::new(),
            squads: &mut extra.squads,
            botcam: &mut extra.cam,
            rep_ids: &extra.reps,
            chat: &extra.chat,
            map_name: "test",
            danger: None,
        };
        let f = bot.frame(&mut g, &plane, &mut rng);
        if let Some(cmd) = f.cmd {
            attacked |= cmd.buttons & buttons::ATTACK != 0 && bot.enemy() == Some(2);
            plane.apply(1, &cmd);
        }
    }
    assert!(attacked, "tasks: {:?}", bot.tasks().iter().map(|t| t.kind).collect::<Vec<_>>());
    assert_eq!(bot.current_weapon(), Some(1));
}

/// A world with a door on the `x = 100` plane (entity 5, named `door1`) and a button (entity 6)
/// that targets it, for the lift and door code; otherwise [`Plane`].
struct Doors {
    plane: Plane,
}

impl World for Doors {
    fn trace_line(&self, start: Vector, end: Vector, m: Monsters, g: Glass, i: Option<u32>) -> Trace {
        if (start.x < 100.0) != (end.x < 100.0) {
            let f = (100.0 - start.x) / (end.x - start.x);
            return Trace { fraction: f, end_pos: start + (end - start) * f, hit: Some(5) };
        }
        self.plane.trace_line(start, end, m, g, i)
    }
    fn point_contents(&self, at: Vector) -> i32 {
        self.plane.point_contents(at)
    }
}

impl BotWorld for Doors {
    fn time(&self) -> f32 {
        self.plane.time
    }
    fn max_entities(&self) -> u32 {
        8
    }
    fn max_clients(&self) -> u32 {
        4
    }
    fn entity(&self, index: u32) -> Option<Entity> {
        self.plane.entity(index)
    }
    fn in_pvs(&self, _: u32, _: Vector) -> bool {
        true
    }
    fn entities_in_sphere(&self, o: Vector, r: f32) -> Vec<u32> {
        self.plane.entities_in_sphere(o, r)
    }
    fn teamplay(&self) -> bool {
        false
    }
    fn find_entities(&self, key: EntityKey, value: &str) -> Vec<u32> {
        self.plane
            .ents
            .borrow()
            .iter()
            .filter(|e| match key {
                EntityKey::Classname => e.classname == value,
                EntityKey::Target => e.target == value,
                EntityKey::Targetname => e.targetname == value,
            })
            .map(|e| e.index)
            .collect()
    }
}

fn brush(index: u32, classname: &str, origin: Vector, size: Vector) -> Entity {
    Entity {
        index,
        serial: 1,
        classname: classname.into(),
        origin,
        absmin: origin - size * 0.5,
        absmax: origin + size * 0.5,
        size,
        solid: 4,
        ..Entity::default()
    }
}

/// A bot with all of `gBotGlobals` around it, for the tests below.
struct Rig {
    bot: Bot,
    wp: Waypoints,
    weapons: Weapons,
    presets: WeaponPresets,
    learned: GlobalLearnedData,
    config: Config,
    extra: Extra,
}

impl Rig {
    fn new(config: Config) -> Rig {
        let (_, bot, wp, weapons, presets, learned, _) = setup(false);
        Rig { bot, wp, weapons, presets, learned, config, extra: Extra::default() }
    }

    /// Runs `f` with the bot and its globals.
    fn with<R>(&mut self, f: impl FnOnce(&mut Bot, &mut BotGlobals<'_>) -> R) -> R {
        let mut g = BotGlobals {
            config: &self.config,
            waypoints: &mut self.wp,
            weapons: &mut self.weapons,
            presets: &self.presets,
            learned: &mut self.learned,
            team_scores: [0; 4],
            masters: &[],
            other_bots: Vec::new(),
            squads: &mut self.extra.squads,
            botcam: &mut self.extra.cam,
            rep_ids: &self.extra.reps,
            chat: &self.extra.chat,
            map_name: "ts_test",
            danger: None,
        };
        f(&mut self.bot, &mut g)
    }

    /// The event-replay harness: the host's damage and kill events, each at its game time, go
    /// through a bot manager holding the rig's bot (so the globals' learners see them too).
    fn replay(&mut self, w: &mut Plane, script: &[(f32, crate::learn::CombatEvent)]) {
        let mut mgr = BotManager::default();
        mgr.add(self.bot.clone());
        let mut rng = XorShiftRandom::new(1);
        for (t, ev) in script {
            w.time = *t;
            let mut g = BotGlobals {
                config: &self.config,
                waypoints: &mut self.wp,
                weapons: &mut self.weapons,
                presets: &self.presets,
                learned: &mut self.learned,
                team_scores: [0; 4],
                masters: &[],
                other_bots: Vec::new(),
                squads: &mut self.extra.squads,
                botcam: &mut self.extra.cam,
                rep_ids: &self.extra.reps,
                chat: &self.extra.chat,
                map_name: "ts_test",
                danger: None,
            };
            mgr.combat_event(&mut g, w, ev, &mut rng);
        }
        self.bot = mgr.remove(self.bot.index).expect("the bot");
    }
}

fn plane(ents: Vec<Entity>) -> Plane {
    Plane {
        time: 1.0,
        ents: RefCell::new(ents),
        teamplay: false,
        models: None,
        wall: Default::default(),
        physics: Default::default(),
        inventory: Default::default(),
        last_cmd: Default::default(),
        highest: Default::default(),
        inventory_reads: Default::default(),
        map_items: Default::default(),
    }
}

#[test]
fn human_tower_ducks_then_stands_for_whoever_climbs() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    let mut rng = XorShiftRandom::new(1);
    rig.with(|b, _| {
        b.me = w.entity(1).unwrap();
        b.add_task(Task::new(TaskKind::HumanTower, 0, None, 0, 0.0, Vector::new(10.0, 0.0, 0.0)));
        let (mut done, task) = (false, *b.tasks.current().unwrap());
        b.task_human_tower(&w, &mut rng, task, &mut done);
        // On the spot: the bot crouches.
        let task = *b.tasks.current().unwrap();
        b.task_human_tower(&w, &mut rng, task, &mut done);
        assert!(b.button & buttons::DUCK != 0 && !done);
    });
    // Someone stands on the bot: it stands up, then jumps.
    let mut climber = player(2, Vector::new(0.0, 0.0, 72.0), 0);
    climber.groundentity = Some(1);
    w.ents.borrow_mut().push(climber);
    rig.with(|b, _| {
        b.button = buttons::DUCK;
        let task = *b.tasks.current().unwrap();
        let mut done = false;
        b.task_human_tower(&w, &mut rng, task, &mut done);
        assert_eq!(b.button & buttons::DUCK, 0);
        assert_eq!(b.tasks.current().unwrap().int, 1);
    });
    w.ents.borrow_mut().retain(|e| e.index != 2);
    rig.with(|b, _| {
        let task = *b.tasks.current().unwrap();
        let mut done = false;
        b.task_human_tower(&w, &mut rng, task, &mut done);
        assert!(b.button & buttons::JUMP != 0);
    });
}

#[test]
fn human_tower_waypoint_touch_needs_height_and_nearness() {
    let rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    let mut b = rig.bot;
    b.me = w.entity(1).unwrap();
    assert!(b.touched_human_tower(&w, Vector::new(20.0, 0.0, 0.0), 20.0));
    assert!(!b.touched_human_tower(&w, Vector::new(50.0, 0.0, 0.0), 50.0));
    // Too low.
    assert!(!b.touched_human_tower(&w, Vector::new(0.0, 0.0, 40.0), 10.0));
    // Someone crouching there already: within 100 units is enough.
    let mut crouched = player(2, Vector::new(80.0, 0.0, 0.0), 0);
    crouched.flags |= entflags::DUCKING;
    w.ents.borrow_mut().push(crouched);
    assert!(b.touched_human_tower(&w, Vector::new(80.0, 0.0, 0.0), 80.0));
}

#[test]
fn pushable_goes_behind_faces_and_pushes() {
    let mut rig = Rig::new(Config::default());
    let crate_ = brush(4, "func_pushable", Vector::new(200.0, 0.0, 0.0), Vector::new(48.0, 48.0, 48.0));
    let w = plane(vec![player(1, Vector::ZERO, 0), crate_.clone()]);
    let mut rng = XorShiftRandom::new(2);
    let target = Vector::new(400.0, 0.0, 0.0);
    rig.with(|b, _| {
        b.me = w.entity(1).unwrap();
        b.add_task(Task::new(TaskKind::PushPushable, 0, Some(crate_.handle()), 0, 0.0, target));
        let run = |b: &mut Bot, rng: &mut XorShiftRandom| {
            let (mut done, mut failed) = (false, false);
            let t = *b.tasks.current().unwrap();
            b.task_push_pushable(&w, rng, t, &mut done, &mut failed);
            (done, failed)
        };
        assert_eq!(run(b, &mut rng), (false, false));
        assert_eq!(b.tasks.current().unwrap().int, 1);
        run(b, &mut rng);
        // Behind the crate, away from the target: (200 - 32, 0).
        let behind = special::desired_pushable_vector(target, &crate_);
        assert!((behind - Vector::new(168.0, 0.0, 0.0)).length() < 0.01, "{behind:?}");
        assert!(b.move_to_valid && b.move_to == behind);
    });
    assert!(special::acceptable_pushable_vector(&crate_, Vector::new(220.0, 0.0, 0.0)));
    assert!(!special::acceptable_pushable_vector(&crate_, target));
}

#[test]
fn waits_for_a_lift_until_one_is_near() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    let at = Vector::new(300.0, 0.0, 0.0);
    rig.with(|b, _| {
        b.me = w.entity(1).unwrap();
        b.add_task(Task::new(TaskKind::WaitForLift, 0, None, 0, 0.0, at));
        let mut done = false;
        let t = *b.tasks.current().unwrap();
        b.task_wait_for_lift(&w, t, &mut done);
        assert!(!done);
        assert_eq!(b.tasks.current().unwrap().float, 11.0);
    });
    w.ents.borrow_mut().push(brush(5, "func_plat", at, Vector::new(64.0, 64.0, 8.0)));
    rig.with(|b, _| {
        let mut done = false;
        let t = *b.tasks.current().unwrap();
        b.task_wait_for_lift(&w, t, &mut done);
        assert!(done);
    });
}

#[test]
fn check_lift_presses_the_doors_button() {
    let mut rig = Rig::new(Config::default());
    let mut door = brush(5, "func_door", Vector::new(100.0, 0.0, 0.0), Vector::new(8.0, 128.0, 128.0));
    door.targetname = "door1".into();
    let mut button = brush(6, "func_button", Vector::new(50.0, 80.0, 0.0), Vector::new(8.0, 8.0, 8.0));
    button.target = "door1".into();
    let w = Doors { plane: plane(vec![player(1, Vector::ZERO, 0), door, button]) };
    let mut rng = XorShiftRandom::new(3);
    let using = rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.goal_origin = Some(Vector::new(300.0, 0.0, 0.0));
        b.check_lift(g, &w, Vector::ZERO, Vector::new(200.0, 0.0, 0.0), &mut rng)
    });
    // A door, not a lift: walk to the button, use it, then go on to the goal.
    assert!(!using);
    let kinds: Vec<(TaskKind, Option<u32>)> = rig.bot.tasks.iter().map(|t| (t.kind, t.edict.map(|e| e.index))).collect();
    assert_eq!(kinds, vec![(TaskKind::FindPath, None), (TaskKind::Use, Some(6)), (TaskKind::FindPath, None)]);
    assert_eq!(rig.bot.tasks.iter().next().unwrap().vector, Vector::new(50.0, 80.0, 0.0));
}

#[test]
fn a_momentary_button_is_held_while_its_door_moves() {
    let mut rig = Rig::new(Config::default());
    let mut lever = brush(6, "momentary_rot_button", Vector::new(10.0, 0.0, 0.0), Vector::new(8.0, 8.0, 8.0));
    lever.target = "gate".into();
    let mut gate = brush(5, "func_door", Vector::new(100.0, 0.0, 0.0), Vector::new(8.0, 128.0, 128.0));
    gate.targetname = "gate".into();
    gate.velocity = Vector::new(0.0, 0.0, 10.0);
    let w = Doors { plane: plane(vec![player(1, Vector::ZERO, 0), lever.clone(), gate]) };
    let mut rng = XorShiftRandom::new(4);
    rig.with(|b, _| {
        b.me = w.entity(1).unwrap();
        let mut t = Task::new(TaskKind::Use, 0, Some(lever.handle()), 1, 5.0, Vector::ZERO);
        t.found_path = true;
        b.add_task(t);
        let (mut done, mut failed) = (false, false);
        let t = *b.tasks.current().unwrap();
        b.task_use(&w, &mut rng, t, &mut done, &mut failed);
        assert!(!done && b.button & buttons::USE != 0);
    });
    w.plane.ents.borrow_mut().iter_mut().find(|e| e.index == 5).unwrap().velocity = Vector::ZERO;
    rig.with(|b, _| {
        let (mut done, mut failed) = (false, false);
        let t = *b.tasks.current().unwrap();
        b.task_use(&w, &mut rng, t, &mut done, &mut failed);
        assert!(done);
    });
}

#[test]
fn grenade_throw_waypoints_throw_an_m61_at_an_enemy_just_seen() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    let mut rng = XorShiftRandom::new(5);
    rig.wp = Waypoints::new();
    rig.wp.add_origin(Vector::ZERO, 0, None);
    rig.wp.add_origin(Vector::new(100.0, 0.0, 0.0), fl::GREN_THROW, None);
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.current_waypoint = 1;
        b.prev_waypoint = 0;
        b.current_waypoint_flags = fl::GREN_THROW;
        b.last_see_enemy_time = 0.5;
        b.last_see_enemy_position = Vector::new(500.0, 0.0, 0.0);
        // HL's hand grenade id is TS's Desert Eagle (design §5.59): no throw with one.
        b.weapons.weapon_info(12, 7, 14, g.weapons, g.presets);
        b.next_waypoint_tasks(g, &w, 0, &mut rng);
        assert!(b.tasks.is_empty());
        b.weapons.weapon_info(24, 1, 0, g.weapons, g.presets);
        b.next_waypoint_tasks(g, &w, 0, &mut rng);
        let t = *b.tasks.current().unwrap();
        assert_eq!((t.kind, t.vector), (TaskKind::ThrowGrenade, Vector::new(500.0, 0.0, 0.0)));
    });
}

#[test]
fn touching_the_item_it_went_for_finishes_the_pickup() {
    let mut rig = Rig::new(Config::default());
    let mut item = brush(4, "ts_powerup", Vector::new(10.0, 0.0, 0.0), Vector::new(16.0, 16.0, 16.0));
    item.solid = SOLID_TRIGGER;
    let w = plane(vec![player(1, Vector::ZERO, 0), item.clone()]);
    let mut rng = XorShiftRandom::new(6);
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.add_task(Task::new(TaskKind::PickupItem, 0, Some(item.handle()), 1, 0.0, Vector::ZERO));
        b.pickup_entity = Some(item.handle());
        b.touch(g, &w, &item, &mut rng);
        assert!(b.tasks.is_empty());
        assert!(b.pickup_item_time > 1.0 && b.pickup_entity.is_none());
    });
}

#[test]
fn squads_form_wedges_and_hand_on_the_lead() {
    let leader = player(2, Vector::ZERO, 1);
    let (m1, m2) = (player(1, Vector::new(-100.0, 0.0, 0.0), 1), player(3, Vector::new(-100.0, 50.0, 0.0), 1));
    let w = plane(vec![leader.clone(), m1.clone(), m2.clone()]);
    let mut squads = Squads::default();
    let id = squads.add_member(&leader, &m1, false).unwrap();
    assert_eq!(squads.add_member(&leader, &m2, false), Some(id));
    assert_eq!(squads.messages.len(), 2);
    let s = squads.get(id).unwrap();
    // Newest member first, on alternate sides behind the leader (facing +x; HL's `v_right` is
    // -y there): (-1, -1) and (-1, +1) times the spread and the position.
    let v1 = s.formation_vector(&w, m2.handle());
    let v2 = s.formation_vector(&w, m1.handle());
    assert!((v1 - Vector::new(-50.0, -50.0, 0.0)).length() < 0.01, "{v1:?}");
    assert!((v2 - Vector::new(-100.0, 100.0, 0.0)).length() < 0.01, "{v2:?}");
    // Another team can't join.
    assert_eq!(squads.add_member(&leader, &player(4, Vector::ZERO, 2), false), None);
    // The newest member takes over; with one left, nobody leads and the squad goes.
    assert_eq!(squads.change_leader(id, &w), Some(m2.handle()));
    assert_eq!(squads.change_leader(id, &w), None);
    assert!(squads.get(id).is_none());
}

#[test]
fn a_disliked_player_is_the_first_enemy() {
    let mut rig = Rig::new(Config::default());
    let near = player(2, Vector::new(100.0, 0.0, 0.0), 0);
    let far = player(3, Vector::new(300.0, 0.0, 0.0), 0);
    let w = plane(vec![player(1, Vector::ZERO, 0), near, far]);
    rig.extra.reps[2] = 7;
    rig.extra.reps[3] = 8;
    rig.bot.profile.rep.add_rep(7, 5);
    rig.bot.profile.rep.add_rep(8, 1);
    let mut rng = XorShiftRandom::new(7);
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        // The priority is part of the threat score whatever the order the enemies were seen in
        // (design §4; upstream's needed the disliked one to be nearer too, §5.63).
        for order in [[2, 3], [3, 2]] {
            b.enemy = None;
            b.tasks.flush();
            b.visibles = order.into();
            assert_eq!(b.find_enemy(g, &w, &mut rng).map(|e| e.index), Some(3));
            assert_eq!(b.enemy_rep, Some(1));
        }
    });
}

#[test]
fn deaths_change_reputations() {
    let mut rig = Rig::new(Config::default());
    let killer = player(2, Vector::new(1000.0, 0.0, 0.0), 0);
    let w = plane(vec![player(1, Vector::ZERO, 0), killer.clone()]);
    rig.extra.reps[2] = 4;
    rig.bot.profile.rep.add_rep(4, 5);
    let mut rng = XorShiftRandom::new(8);
    // Killed from far away by a stranger: the roll decides (rep 5: about half the time).
    let mut lower = 0;
    for _ in 0..40 {
        rig.bot.profile.rep.clear();
        rig.bot.profile.rep.add_rep(4, 5);
        rig.with(|b, g| {
            b.me = w.entity(1).unwrap();
            b.event(g, &w, BotEvent::Died(Some(killer.handle())), &mut rng);
        });
        lower += i32::from(rig.bot.profile.rep.get(4).unwrap().rep == 4);
    }
    assert!((5..35).contains(&lower), "{lower}");
}

#[test]
fn sounds_are_told_apart_and_an_enemy_is_listened_to() {
    assert_eq!(SoundKind::of_sample("player/pl_step1.wav"), SoundKind::Player);
    assert_eq!(SoundKind::of_sample("player/pain3.wav"), SoundKind::PlayerPain);
    assert_eq!(SoundKind::of_sample("doors/doormove1.wav"), SoundKind::Door);
    assert_eq!(SoundKind::of_sample("plats/elevator.wav"), SoundKind::Door);
    assert_eq!(SoundKind::of_sample("weapons/glock.wav"), SoundKind::Weapon);
    assert_eq!(SoundKind::of_sample("buttons/button3.wav"), SoundKind::Button);
    assert_eq!(SoundKind::of_sample("ambience/wind.wav"), SoundKind::Unknown);

    let mut mgr = BotManager::default();
    let (_, bot, ..) = setup(false);
    mgr.add(bot);
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    mgr.get_mut(1).unwrap().me = w.entity(1).unwrap();
    mgr.sound(&w, 2, None, 1.0, "player/pl_step2.wav");
    // Remembered and glanced at, no task taking over (upstream's listen task had no entity and
    // failed at once, design §5.58).
    let b = mgr.get(1).unwrap();
    assert!(b.tasks().is_empty());
    let h = *b.hearing.newest_new().unwrap();
    assert_eq!((h.origin, h.source.map(|s| s.index)), (Vector::new(300.0, 0.0, 0.0), Some(2)));
    assert_eq!((b.look_task, b.pre_aim.watch), (LookTask::PreAim, Vector::new(300.0, 0.0, 0.0)));
    // Too far to hear.
    let mut mgr2 = BotManager::default();
    mgr2.add(setup(false).1);
    let w2 = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(900.0, 0.0, 0.0), 0)]);
    mgr2.get_mut(1).unwrap().me = w2.entity(1).unwrap();
    mgr2.sound(&w2, 2, None, 1.0, "player/pl_step2.wav");
    assert!(mgr2.get(1).unwrap().hearing.newest_new().is_none());
    // Where the game played it counts, not where the entity is.
    mgr2.sound(&w2, 2, Some(Vector::new(200.0, 0.0, 0.0)), 1.0, "player/pl_step2.wav");
    assert_eq!(mgr2.get(1).unwrap().hearing.newest_new().unwrap().origin, Vector::new(200.0, 0.0, 0.0));
    // Silent sounds, kinds no bot reacts to, and teammates aren't heard.
    let mut mgr3 = BotManager::default();
    mgr3.add(setup(false).1);
    mgr3.get_mut(1).unwrap().me = w.entity(1).unwrap();
    mgr3.sound(&w, 2, None, 0.0, "player/pl_step2.wav");
    mgr3.sound(&w, 2, None, 1.0, "weapons/glock.wav");
    let mut team = plane(vec![player(1, Vector::ZERO, 1), player(2, Vector::new(300.0, 0.0, 0.0), 1)]);
    team.teamplay = true;
    mgr3.get_mut(1).unwrap().state = team.entity(1).unwrap().player.unwrap();
    mgr3.sound(&team, 2, None, 1.0, "player/pl_step2.wav");
    assert!(mgr3.get(1).unwrap().hearing.newest_new().is_none());
    assert!(SoundKind::Player.is_heard() && SoundKind::PlayerPain.is_heard() && !SoundKind::Door.is_heard());
}

#[test]
fn chat_lines_are_typed_then_said() {
    let mut config = Config::default();
    config.config_settings |= crate::config::flags::CHATTING;
    let mut rig = Rig::new(config);
    rig.extra.chat = crate::chat::bot_chat::BotChatMessages::parse(b"[greetings-neutral]\nhi %n, welcome to %m\n[idle]\nbored\n");
    rig.extra.reps[2] = 3;
    rig.bot.profile.rep.add_rep(3, 5);
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(9);
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.bot_chat(g, &w, crate::chat::bot_chat::ChatKind::Greetings, Some(2), false, &mut rng);
        assert_eq!(b.chat_string, b"hi p2, welcome to ts_test");
        // Once per player.
        b.tasks.flush();
        b.bot_chat(g, &w, crate::chat::bot_chat::ChatKind::Greetings, Some(2), false, &mut rng);
        assert!(b.tasks.is_empty());
        b.bot_chat(g, &w, crate::chat::bot_chat::ChatKind::Idle, None, false, &mut rng);
        let t = *b.tasks.current().unwrap();
        let (mut done, mut failed) = (false, false);
        b.task_type_message(&w, &mut rng, t, &mut done, &mut failed);
        // "bored" at the typing speed.
        let until = b.tasks.current().unwrap().float;
        assert!(until > 1.0 && !done);
    });
}

#[test]
fn the_bot_camera_follows_a_bot() {
    let mut mgr = BotManager::default();
    let (_, mut bot, wp, ..) = setup(false);
    let w = plane(vec![player(1, Vector::new(500.0, 0.0, 0.0), 0)]);
    bot.me = w.entity(1).unwrap();
    bot.state = bot.me.player.unwrap();
    mgr.add(bot);
    let mut cam = BotCam::default();
    cam.camera = Some(7);
    let mut rng = XorShiftRandom::new(10);
    for _ in 0..50 {
        cam.think(&mgr, &wp, &w, &mut rng);
    }
    assert_eq!(cam.current_bot(), Some(1));
    // Somewhere around the bot, closing in.
    assert!((cam.view.origin - Vector::new(500.0, 0.0, 17.0)).length() < 300.0, "{:?}", cam.view.origin);
    assert_eq!(cam.tune_in_error(1), None);
    assert!(BotCam::default().tune_in_error(1).is_some());
}

#[test]
fn a_grenade_lands_where_its_toss_ends() {
    let w = plane(vec![]);
    let g = Entity { classname: "grenade".into(), origin: Vector::ZERO, velocity: Vector::new(100.0, 0.0, 0.0), ..Entity::default() };
    assert!(think::is_grenade_rocket(&g));
    // No gravity (cvar 0): the 3 s fuse's flight in a straight line (upstream's steps grew the
    // velocity 1.5 times each and traced 256 times the last, §5.75).
    let end = think::trace_toss(&w, &g);
    assert!((end.x - 300.0).abs() < 0.1, "{end:?}");
    // A wall in the way: there.
    w.wall.set(Some(155.0));
    assert!((think::trace_toss(&w, &g).x - 155.0).abs() < 0.1);
}

#[test]
fn tm_maps_make_teams_of_models() {
    let (_, mut bot, ..) = setup(false);
    let mut w =
        plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0), player(3, Vector::new(0.0, 300.0, 0.0), 0)]);
    bot.me = w.entity(1).unwrap();
    bot.state = bot.me.player.unwrap();
    let (p2, p3) = (w.entity(2).unwrap(), w.entity(3).unwrap());
    // Deathmatch: everyone; the game's teamplay with every team 0: no one.
    assert!(bot.is_enemy(&w, &p2) && bot.is_enemy(&w, &p3));
    w.teamplay = true;
    assert!(!bot.is_enemy(&w, &p2) && !bot.is_enemy(&w, &p3));
    // A `tm_` map the game doesn't run as teamplay: upstream's `model` comparison.
    w.teamplay = false;
    w.models = Some(vec![(1, "agent"), (2, "agent"), (3, "merc")]);
    assert!(!bot.is_enemy(&w, &p2) && bot.is_enemy(&w, &p3));
}

#[test]
fn util_bot_goto_replaces_the_tasks() {
    let (_, mut bot, ..) = setup(false);
    bot.add_task(Task::new(TaskKind::Wait, 3, None, 0, 5.0, Vector::ZERO));
    bot.go_to_waypoint(4);
    let t: Vec<_> = bot.tasks().iter().map(|t| (t.kind, t.schedule_id, t.int, t.float)).collect();
    assert_eq!(t, [(TaskKind::FindPath, 0, 4, -2.0)]);
}

#[test]
fn roaming_squad_members_follow_the_leader() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(100.0, 0.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(4);
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.state = b.me.player.unwrap();
        b.squad_leader = Some(w.entity(2).unwrap().handle());
        b.look_for_new_tasks(g, &w, &mut rng);
        assert_eq!(b.tasks().iter().map(|t| t.kind).collect::<Vec<_>>(), [TaskKind::FollowLeader]);
    });
}

#[test]
fn the_loadout_is_bought_before_the_spawn() {
    let mut rig = Rig::new(Config::default());
    let mut dead = player(1, Vector::ZERO, 0);
    dead.health = 0.0;
    dead.player.as_mut().unwrap().alive = false;
    let mut w = plane(vec![dead]);
    let mut rng = XorShiftRandom::new(6);
    let kinds = |f: &Frame| {
        f.actions
            .iter()
            .map(|a| matches!(a, Action::Loadout(_)))
            .zip(f.actions.iter().map(|a| matches!(a, Action::Command(c) if c == "respawn")))
            .collect::<Vec<_>>()
    };
    // Dead: the picks go into the loadout first, then `respawn`; once per life.
    let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
    assert_eq!(kinds(&f), [(true, false), (false, true)]);
    w.time += 0.1;
    let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
    assert!(!f.actions.iter().any(|a| matches!(a, Action::Loadout(_))));
    // Alive, and dead again: a new pick for the next life.
    *w.ents.borrow_mut() = vec![player(1, Vector::ZERO, 0)];
    for _ in 0..5 {
        w.time += 0.1;
        let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
        assert!(!f.actions.iter().any(|a| matches!(a, Action::Loadout(_))));
    }
    let mut dead = player(1, Vector::ZERO, 0);
    dead.health = 0.0;
    dead.player.as_mut().unwrap().alive = false;
    *w.ents.borrow_mut() = vec![dead];
    w.time += 0.1;
    let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
    assert!(f.actions.iter().any(|a| matches!(a, Action::Loadout(_))));
}

/// A world that also lists TS objectives (`BotWorld::ts_objectives`).
struct WithObjectives(Plane, Vec<TsObjective>);

impl World for WithObjectives {
    fn trace_line(&self, s: Vector, e: Vector, m: Monsters, g: Glass, i: Option<u32>) -> Trace {
        self.0.trace_line(s, e, m, g, i)
    }
    fn point_contents(&self, at: Vector) -> i32 {
        self.0.point_contents(at)
    }
}

impl BotWorld for WithObjectives {
    fn time(&self) -> f32 {
        self.0.time()
    }
    fn max_entities(&self) -> u32 {
        self.0.max_entities()
    }
    fn max_clients(&self) -> u32 {
        self.0.max_clients()
    }
    fn entity(&self, index: u32) -> Option<Entity> {
        self.0.entity(index)
    }
    fn in_pvs(&self, v: u32, o: Vector) -> bool {
        self.0.in_pvs(v, o)
    }
    fn entities_in_sphere(&self, o: Vector, r: f32) -> Vec<u32> {
        self.0.entities_in_sphere(o, r)
    }
    fn teamplay(&self) -> bool {
        self.0.teamplay()
    }
    fn ts_objectives(&self, _: u32) -> Vec<TsObjective> {
        self.1.clone()
    }
}

/// `LookForNewTasks`' TS objectives by what they ask of the bot's team (design §4): one it can
/// complete is gone for and its `ts_hack` used for 9 s; one to defend is held from a waypoint
/// that sees it; sometimes the bot roams instead. Upstream: team 1 defended and team 2 attacked
/// whatever the objective said.
#[test]
fn teamplay_bots_go_for_the_ts_objectives() {
    let at = Vector::new(300.0, 0.0, 0.0);
    let mut hack = player(5, at + Vector::new(20.0, 0.0, 0.0), 0);
    hack.classname = "ts_hack".into();
    hack.flags = 0;
    hack.player = None;
    let mut seen = std::collections::HashSet::new();
    for (text, attack) in [("Hack the computer", true), ("Defend the computer", false), ("", false)] {
        let objective = TsObjective { id: 0, origin: at, name: text.into(), ..TsObjective::default() };
        for seed in 1..40 {
            let mut rig = Rig::new(Config::default());
            // Waypoint 2 (at 300) is the objective's; 3 and 4 see it from 150 and 300 units.
            for i in 0..6 {
                for j in 0..6 {
                    rig.wp.visibility.set(i, j, true);
                }
            }
            let mut plane = plane(vec![player(1, Vector::ZERO, 1), hack.clone()]);
            plane.teamplay = true;
            let w = WithObjectives(plane, vec![objective.clone()]);
            let mut rng = XorShiftRandom::new(seed);
            let kinds = rig.with(|b, g| {
                b.me = w.entity(1).unwrap();
                b.state = b.me.player.unwrap();
                b.look_for_new_tasks(g, &w, &mut rng);
                b.tasks().iter().map(|t| (t.kind, t.int, t.float, t.edict.is_some())).collect::<Vec<_>>()
            });
            match kinds.as_slice() {
                [(TaskKind::FindPath, 2, -1.0, false), (TaskKind::Use, 0, 9.0, true)] => assert!(attack, "{text}"),
                [(TaskKind::FindPath, spot, -1.0, false), (TaskKind::Defend, ..)] => {
                    assert!(!attack, "{text}");
                    assert!([0, 4, 5].contains(spot), "{spot}: 200-900 units from the objective, seeing it");
                }
                [(TaskKind::FindPath, ..)] | [] => {} // roaming: the end-level waypoints, or none
                other => panic!("{text}: {other:?}"),
            }
            seen.insert((attack, kinds.len()));
        }
    }
    assert!(seen.contains(&(true, 2)) && seen.contains(&(false, 2)), "{seen:?}");
}

/// Runs the rig's bot in `w` for `frames` frames of 20 ms, applying its usercmds; `each` sees
/// the world (to move the other players) and every frame the bot made.
fn run(rig: &mut Rig, w: &mut Plane, rng: &mut XorShiftRandom, frames: usize, mut each: impl FnMut(&Plane, &Frame)) {
    for _ in 0..frames {
        w.time += 0.02;
        let f = rig.with(|b, g| b.frame(g, w, rng));
        if let Some(cmd) = f.cmd {
            w.apply(1, &cmd);
        }
        each(w, &f);
    }
}

/// Sets player `index`'s weapon, clip and reserve as the port reports them.
fn arm(w: &Plane, index: u32, weapon: i32, clip: i32, ammo: i32) {
    let mut ents = w.ents.borrow_mut();
    let p = ents.iter_mut().find(|e| e.index == index).unwrap().player.as_mut().unwrap();
    (p.weapon, p.clip, p.ammo) = (weapon, clip, ammo);
}

#[test]
fn never_fires_at_an_enemy_behind_a_wall() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(9);
    let mut fired = false;
    run(&mut rig, &mut w, &mut rng, 150, |_, f| fired |= f.cmd.is_some_and(|c| c.buttons & buttons::ATTACK != 0));
    assert!(fired, "it shoots the enemy in the open");
    // A wall goes up between them; the enemy moves so it isn't where the bot last saw it.
    w.wall.set(Some(150.0));
    w.ents.borrow_mut()[1].origin.y = 120.0;
    let mut fired_behind = 0;
    for _ in 0..300 {
        w.time += 0.02;
        let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
        if let Some(cmd) = f.cmd {
            // The fake world has no walls to walk into: keep the bot on its side.
            w.apply(1, &cmd);
            w.ents.borrow_mut()[0].origin = Vector::ZERO;
            fired_behind += i32::from(cmd.buttons & buttons::ATTACK != 0);
        }
    }
    assert_eq!(fired_behind, 0);
}

#[test]
fn an_empty_clip_is_reloaded_with_one_press() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(4);
    run(&mut rig, &mut w, &mut rng, 20, |_, _| {});
    // The clip runs dry (the fake world never reloads it).
    arm(&w, 1, 1, 0, 34);
    let (mut presses, mut held) = (0, false);
    run(&mut rig, &mut w, &mut rng, 95, |_, f| {
        let down = f.cmd.map_or(held, |c| c.buttons & buttons::RELOAD != 0);
        if down && !held {
            presses += 1;
        }
        held = down;
    });
    assert_eq!(presses, 1, "one press per {RELOAD_RETRY} s");
    // A full clip, or no reserve: never.
    for (clip, ammo) in [(17, 34), (0, 0)] {
        arm(&w, 1, 1, clip, ammo);
        let mut any = false;
        run(&mut rig, &mut w, &mut rng, 150, |_, f| any |= f.cmd.is_some_and(|c| c.buttons & buttons::RELOAD != 0));
        assert!(!any, "clip {clip} ammo {ammo}");
    }
}

#[test]
fn an_empty_gun_is_dropped_and_the_game_picks_the_next() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(2);
    run(&mut rig, &mut w, &mut rng, 10, |_, _| {});
    arm(&w, 1, 1, 0, 0);
    let mut actions = Vec::new();
    run(&mut rig, &mut w, &mut rng, 20, |_, f| actions.extend(f.actions.iter().cloned()));
    // Dropped once; no forced kung fu: TS takes out the next weapon the bot owns.
    assert_eq!(actions.iter().filter(|a| **a == Action::Command("drop".into())).count(), 1, "{actions:?}");
    assert!(!actions.iter().any(|a| matches!(a, Action::SelectWeapon(_))), "{actions:?}");
    // A gun it knows has rounds is taken out at once.
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    arm(&w, 1, 5, 30, 60);
    run(&mut rig, &mut w, &mut rng, 5, |_, _| {});
    arm(&w, 1, 1, 0, 0);
    let mut actions = Vec::new();
    run(&mut rig, &mut w, &mut rng, 20, |_, f| actions.extend(f.actions.iter().cloned()));
    assert!(actions.contains(&Action::SelectWeapon(5)), "{actions:?}");
}

#[test]
fn unarmed_it_takes_cover_from_a_gunman_aiming_at_it() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(500.0, 0.0, 0.0), 0)]);
    arm(&w, 1, 0, 100, 0);
    w.ents.borrow_mut()[1].v_angle = Vector::new(0.0, 180.0, 0.0);
    let mut rng = XorShiftRandom::new(6);
    let mut attacked = false;
    run(&mut rig, &mut w, &mut rng, 100, |_, f| attacked |= f.cmd.is_some_and(|c| c.buttons & (buttons::ATTACK | buttons::ATTACK2) != 0));
    let covered = rig.bot.tasks().has_schedule(ScheduleDesc::RunForCover) || rig.bot.tasks().has_task(TaskKind::Crouch);
    assert!(!attacked);
    assert!(covered, "{:?}", rig.bot.tasks().iter().map(|t| t.kind).collect::<Vec<_>>());
}

#[test]
fn unarmed_it_rushes_a_gunman_looking_away() {
    let mut rig = Rig::new(Config::default());
    // The gunman at 500 looks along +x, away from the bot.
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(500.0, 0.0, 0.0), 0)]);
    arm(&w, 1, 0, 100, 0);
    let mut rng = XorShiftRandom::new(6);
    run(&mut rig, &mut w, &mut rng, 100, |w, _| w.ents.borrow_mut()[1].origin = Vector::new(500.0, 0.0, 0.0));
    assert!(!rig.bot.tasks().has_schedule(ScheduleDesc::RunForCover));
    assert!(w.entity(1).unwrap().origin.x > 150.0, "at {:?}", w.entity(1).unwrap().origin);
}

#[test]
fn a_broken_off_wall_kick_leaves_the_view_alone() {
    // A wall kick under way: the bot turns to the wall.
    let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::WallKick], 120.0);
    w.wall.set(Some(-60.0));
    let mut rng = XorShiftRandom::new(5);
    run(&mut rig, &mut w, &mut rng, 20, |_, _| {});
    assert!(rig.bot.kungfu_face(w.time).is_some());
    // The opponent vanishes mid-move: the fight is over, no wall to face.
    w.ents.borrow_mut().remove(1);
    run(&mut rig, &mut w, &mut rng, 50, |_, _| {});
    assert!(rig.bot.kungfu_face(w.time).is_none());
}

#[test]
fn kung_fu_answers_attacks_and_strikes_back() {
    let mut rig = Rig::new(Config::default());
    // Kung fu both, 64 units apart, the opponent facing the bot.
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(64.0, 0.0, 0.0), 0)]);
    arm(&w, 1, 0, 100, 0);
    arm(&w, 2, 0, 100, 0);
    w.ents.borrow_mut()[1].v_angle = Vector::new(0.0, 180.0, 0.0);
    let mut rng = XorShiftRandom::new(8);
    let (mut defended, mut attacks) = (0, 0);
    let mut n = 0;
    let mut last = 0;
    run(&mut rig, &mut w, &mut rng, 1000, |w, f| {
        n += 1;
        // The opponent punches every 0.6 s, and stays where it is.
        let mut ents = w.ents.borrow_mut();
        ents[1].button = if n % 30 < 3 { buttons::ATTACK } else { 0 };
        ents[1].origin = Vector::new(64.0, 0.0, 0.0);
        drop(ents);
        let Some(c) = f.cmd else { return };
        let new = c.buttons & !last;
        last = c.buttons;
        if new & (buttons::RELOAD | buttons::DUCK) != 0 || c.forward < -1.0 {
            defended += 1;
        }
        if new & (buttons::ATTACK | buttons::ATTACK2) != 0 {
            attacks += 1;
        }
    });
    assert!(defended > 3, "defended {defended}");
    assert!(attacks > 3, "attacks {attacks}");
    // It scored what it did.
    let k = &rig.bot.kungfu;
    assert!(k.reaction_all.iter().any(|a| a.n > 0.0), "{:?}", k.reaction_all);
    assert!(k.combos.iter().any(|c| c.uses > 0));
}

#[test]
fn a_kung_fu_fight_is_settled_by_how_it_ends() {
    // A kill scores the open stance +2; a death -2.
    for (ev, want) in [(kill(1, 2, 0), 2.0), (kill(2, 1, 0), -2.0)] {
        let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::Punch], 200.0);
        let mut rng = XorShiftRandom::new(5);
        run(&mut rig, &mut w, &mut rng, 10, |_, _| {});
        assert_eq!(rig.bot.kungfu.opponent(), Some(2));
        rig.with(|b, g| b.combat_event(g, &w, &ev, &mut rng));
        let k = &rig.bot.kungfu;
        assert!(k.stance_all.iter().any(|a| a.value == want && a.n > 0.0), "{:?}", k.stance_all);
        // Settled once: the fight going on doesn't score it again.
        let before = k.stance_all;
        rig.with(|b, g| b.combat_event(g, &w, &ev, &mut rng));
        assert_eq!(rig.bot.kungfu.stance_all, before);
    }
}

/// A kung fu bot that knows only `moves`, against a passive kung fu opponent at `x`, with the
/// physics on.
fn kung_fu_duel(moves: &[kungfu::Move], x: f32) -> (Rig, Plane) {
    let mut rig = Rig::new(Config::default());
    rig.bot.kungfu.combos = vec![kungfu::Combo { moves: moves.to_vec(), value: 0.0, uses: 0 }];
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(x, 0.0, 0.0), 0)]);
    w.physics.set(true);
    arm(&w, 1, 0, 100, 0);
    arm(&w, 2, 0, 100, 0);
    (rig, w)
}

#[test]
fn the_fly_kick_runs_crouch_jumps_and_kicks_in_the_air() {
    let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::FlyKick], 220.0);
    let mut rng = XorShiftRandom::new(3);
    let (mut crouch_jumped, mut air_kick) = (false, false);
    run(&mut rig, &mut w, &mut rng, 400, |w, f| {
        let Some(c) = f.cmd else { return };
        let grounded = w.entity(1).unwrap().flags & entflags::ONGROUND != 0;
        crouch_jumped |= c.buttons & (buttons::JUMP | buttons::DUCK) == buttons::JUMP | buttons::DUCK;
        air_kick |= crouch_jumped && !grounded && c.buttons & (buttons::DUCK | buttons::ATTACK2) == buttons::DUCK | buttons::ATTACK2;
        // The opponent stays put.
        w.ents.borrow_mut()[1].origin = Vector::new(220.0, 0.0, 0.0);
    });
    assert!(crouch_jumped && air_kick, "jumped {crouch_jumped} kicked {air_kick}");
    assert!(rig.bot.kungfu.combos[0].uses > 0);
}

#[test]
fn the_wall_kick_faces_the_wall_jumps_off_it_and_kicks_back() {
    // The wall 60 units behind the bot, the opponent 120 in front.
    let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::WallKick], 120.0);
    w.wall.set(Some(-60.0));
    let mut rng = XorShiftRandom::new(5);
    let (mut wall_jumped, mut kicked_flying_back) = (false, false);
    run(&mut rig, &mut w, &mut rng, 400, |w, f| {
        let Some(c) = f.cmd else { return };
        let me = w.entity(1).unwrap();
        let (fwd, _) = make_vectors(Vector::new(0.0, c.angles.y, 0.0));
        if c.buttons & buttons::ALT1 != 0 && fwd.x < -0.7 {
            wall_jumped = true;
        }
        // In the air, moving toward the opponent while looking at the wall: the back kick.
        if wall_jumped && me.flags & entflags::ONGROUND == 0 && c.buttons & buttons::ATTACK2 != 0 && me.velocity.x > 100.0 && fwd.x < 0.0 {
            kicked_flying_back = true;
        }
        w.ents.borrow_mut()[1].origin = Vector::new(120.0, 0.0, 0.0);
    });
    assert!(wall_jumped && kicked_flying_back, "wall jumped {wall_jumped} kicked {kicked_flying_back}");
}

#[test]
fn obstacles_are_crouch_jumped() {
    // `jump_and_duck`: the jump, then the duck held through the air.
    let mut rig = Rig::new(Config::default());
    rig.bot.jump_and_duck(1.0);
    let cmd = rig.bot.run_player_move(1.05);
    assert_ne!(cmd.buttons & buttons::JUMP, 0);
    let cmd = rig.bot.run_player_move(1.2);
    assert_ne!(cmd.buttons & buttons::DUCK, 0);
}

fn owned(list: &[(i32, i32, i32)]) -> Option<Vec<InventoryWeapon>> {
    Some(list.iter().map(|&(weapon, clip, ammo)| InventoryWeapon { weapon, clip, ammo }).collect())
}

#[test]
fn owned_weapons_come_from_the_inventory() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
    *w.inventory.borrow_mut() = owned(&[(1, 17, 34), (5, 30, 60)]);
    let mut rng = XorShiftRandom::new(3);
    run(&mut rig, &mut w, &mut rng, 1, |_, _| {});
    // A gun it never held is known with its rounds.
    assert!(rig.bot.weapons.has(5) && rig.bot.weapons.get(5).unwrap().clip == 30);
    assert_eq!(rig.bot.current_weapon(), Some(1));
    assert_eq!(rig.bot.weapons.best_loaded_gun(), Some(5));
    // Taken away: forgotten at the next read, within a second.
    *w.inventory.borrow_mut() = owned(&[(1, 17, 34)]);
    run(&mut rig, &mut w, &mut rng, 60, |_, _| {});
    assert!(!rig.bot.weapons.has(5) && rig.bot.weapons.has(1));
}

#[test]
fn the_weapon_in_hand_is_what_the_game_reports() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
    *w.inventory.borrow_mut() = owned(&[(1, 17, 34), (5, 30, 60)]);
    let mut rng = XorShiftRandom::new(3);
    run(&mut rig, &mut w, &mut rng, 1, |_, _| {});
    // The switch is asked for once; the bot still holds the Glock until the game says otherwise.
    rig.bot.switch_weapon(5);
    rig.bot.switch_weapon(5);
    assert_eq!(rig.bot.actions, vec![Action::SelectWeapon(5)]);
    assert_eq!(rig.bot.current_weapon(), Some(1));
    // Refused (not owned after all): forgotten, and asked again only if chosen again.
    rig.bot.weapon_select_failed(5);
    assert!(!rig.bot.weapons.has(5) && rig.bot.switch_pending.is_none());
    rig.bot.actions.clear();
    // The game switched: that is the current weapon.
    arm(&w, 1, 5, 30, 60);
    run(&mut rig, &mut w, &mut rng, 1, |_, _| {});
    assert_eq!(rig.bot.current_weapon(), Some(5));
    assert!(rig.bot.weapons.has(5));
}

#[test]
fn a_new_life_forgets_the_last_ones_guns() {
    let mut rig = Rig::new(Config::default());
    let mut rng = XorShiftRandom::new(3);
    rig.with(|b, g| {
        b.weapons.weapon_info(0, 100, 0, g.weapons, g.presets);
        b.weapons.weapon_info(5, 30, 60, g.weapons, g.presets);
        b.next_inventory_sync = 50.0;
        b.spawn_init(g, &mut rng);
        assert!(!b.weapons.has(5));
        assert!(b.weapons.has(TsWeapon::KungFu.id()));
        assert_eq!(b.next_inventory_sync, 0.0);
    });
}

#[test]
fn a_teleport_drops_the_path() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
    let (mut wp, mut rng) = (Waypoints::new(), XorShiftRandom::new(3));
    for i in 0..6 {
        wp.add_origin(Vector::new(i as f32 * 150.0, 0.0, 0.0), 0, None);
    }
    for i in 0..5 {
        wp.add_path(i, (i + 1) as i16);
        wp.add_path(i + 1, i as i16);
    }
    rig.wp = wp;
    run(&mut rig, &mut w, &mut rng, 100, |_, _| {});
    assert!(rig.bot.current_waypoint != -1, "on its way");
    // Far from every waypoint in one frame: nothing of the old path is kept.
    w.ents.borrow_mut()[0].origin = Vector::new(3000.0, 3000.0, 0.0);
    run(&mut rig, &mut w, &mut rng, 1, |_, _| {});
    assert_eq!((rig.bot.current_waypoint, rig.bot.prev_waypoint), (-1, -1));
    assert!(rig.bot.paths.bot_paths.is_empty());
    // What the host does on `fixangle` 1.
    rig.bot.current_waypoint = 3;
    rig.bot.stuck_time = 5.0;
    rig.bot.teleported();
    assert_eq!((rig.bot.current_waypoint, rig.bot.stuck_time), (-1, 0.0));
}

#[test]
fn a_breakable_is_an_enemy_by_the_first_client_near_it() {
    let rig = Rig::new(Config::default());
    let breakable = |x: f32| {
        let mut b = brush(6, "func_breakable", Vector::new(x, 0.0, 0.0), Vector::new(16.0, 16.0, 16.0));
        (b.health, b.takedamage, b.target) = (50.0, 1.0, "t".into());
        b
    };
    let mut b = rig.bot;
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(260.0, 0.0, 0.0), 0)]);
    b.me = w.entity(1).unwrap();
    b.state = b.me.player.unwrap();
    // Next to the bot: in its way.
    assert!(b.is_enemy(&w, &breakable(50.0)));
    // Near the bot's box but not within 80 units, and no one else near: not an enemy.
    assert!(!b.is_enemy(&w, &breakable(150.0)));
    // An enemy the bot sees stands by it (the bot, client 1, is skipped as too far).
    b.visibles = [2].into();
    assert!(b.is_enemy(&w, &breakable(170.0)));
    assert!(super::combat::box_in_sphere(&w.entity(2).unwrap(), Vector::new(170.0, 0.0, 0.0), 200.0));
    assert!(!super::combat::box_in_sphere(&w.entity(2).unwrap(), Vector::new(-100.0, 0.0, 0.0), 200.0));
}

#[test]
fn the_nearest_entity_is_upstreams() {
    // Upstream's loop: trace every candidate nearer than the best so far.
    fn upstream(w: &dyn BotWorld, names: &[&str], origin: Vector, mut range: f32) -> Option<u32> {
        let mut nearest = None;
        for name in names {
            for i in w.find_entities(EntityKey::Classname, name) {
                let e = w.entity(i).unwrap();
                let d = (e.entity_origin() - origin).length();
                if d < range {
                    let tr = w.trace_line(origin, e.entity_origin(), Monsters::Ignore, Glass::Ignore, None);
                    if tr.fraction >= 1.0 || tr.hit == Some(e.index) {
                        range = d;
                        nearest = Some(i);
                    }
                }
            }
        }
        nearest
    }
    let mut rng = XorShiftRandom::new(21);
    for _ in 0..30 {
        // Items on both sides of the door at x = 100 (entity 5, which a trace hits).
        let mut ents = vec![player(1, Vector::ZERO, 0), brush(5, "item_b", Vector::new(100.0, 0.0, 0.0), Vector::new(8.0, 64.0, 64.0))];
        for i in 10..30 {
            let c = |r: &mut XorShiftRandom| r.random_long(-8, 8) as f32 * 40.0;
            let name = if i % 2 == 0 { "item_a" } else { "item_b" };
            ents.push(brush(i, name, Vector::new(c(&mut rng), c(&mut rng), 0.0), Vector::new(8.0, 8.0, 8.0)));
        }
        let w = Doors { plane: plane(ents) };
        for _ in 0..10 {
            let o = Vector::new(rng.random_float(-300.0, 300.0), rng.random_float(-300.0, 300.0), 0.0);
            let names = ["item_a", "item_b"];
            let ours = special::find_nearest_entity(&w, &names, o, 400.0, true).map(|e| e.index);
            assert_eq!(ours, upstream(&w, &names, o, 400.0));
        }
    }
}

#[test]
fn the_visible_list_keeps_its_order_and_membership() {
    let mut v = combat::Visibles::default();
    for i in [7, 300, 2, 7] {
        v.set(i, true);
    }
    assert_eq!(v.iter().collect::<Vec<_>>(), vec![7, 300, 2]);
    v.set(300, false);
    v.set(5, false);
    assert!(v.contains(7) && v.contains(2) && !v.contains(300) && !v.contains(5) && !v.contains(9000));
    v.set(300, true);
    assert_eq!(v.to_vec(), vec![7, 2, 300]);
}

/// A damage event from the host (`damage`), the attacker its own inflictor.
fn damage(victim: u32, attacker: u32, amount: f32) -> crate::learn::CombatEvent {
    crate::learn::CombatEvent::Damage(crate::learn::DamageEvent {
        victim,
        attacker,
        inflictor: attacker as i32,
        amount,
        weapon: -1,
        ..Default::default()
    })
}

/// A kill event from the host (`kill_scored`).
fn kill(killer: u32, victim: u32, weapon: i32) -> crate::learn::CombatEvent {
    crate::learn::CombatEvent::Kill(crate::learn::KillEvent {
        killer,
        victim,
        weapon,
        headshot: false,
        close_combat: false,
        kung_fu: weapon == 0,
    })
}

#[test]
fn the_ledger_keeps_what_passed_with_each_opponent() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0), player(3, Vector::new(0.0, 800.0, 0.0), 0)]);
    arm(&w, 3, 5, 30, 60);
    let mut rng = XorShiftRandom::new(1);
    rig.with(|b, g| {
        for ev in [damage(2, 1, 30.0), damage(1, 3, 50.0), damage(3, 2, 10.0), kill(1, 2, 1)] {
            b.combat_event(g, &w, &ev, &mut rng);
        }
    });
    let l = rig.bot.ledger();
    // Not the bot's: the damage between 2 and 3.
    assert_eq!(l.entries().count(), 3);
    assert!((l.reward_since(0.0, 2) - 1.3).abs() < 1e-6);
    assert!((l.reward_since(0.0, 3) + 0.5).abs() < 1e-6);
    let e: Vec<_> = l.entries().map(|e| (e.other, e.weapon, e.range_bin)).collect();
    assert_eq!(e, [(2, 1, 2), (3, 5, 4), (2, 1, 2)]);
    // The kill closed the fight with 2; the one with 3 is open.
    assert!(l.episode(2).is_none() && l.episode(3).is_some());
    rig.with(|b, g| b.combat_event(g, &w, &kill(3, 1, 5), &mut rng));
    assert!(rig.bot.ledger().open_episodes().is_empty());
    assert_eq!(rig.bot.ledger().closed_episodes().last().map(|e| e.reward()), Some(-1.5));
}

#[test]
fn the_learned_strafe_goes_both_ways() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(60.0, 0.0, 0.0), 0)]);
    w.time = 10.0;
    rig.bot.me = w.entity(1).unwrap();
    rig.bot.enemy = Some(w.entity(2).unwrap().handle());
    // Personal genes 10-14 weigh the strafe perceptron's inputs (speed, distance, height, size,
    // on ground): strongly negative, the sigmoid is near 0, which upstream turned into a slow
    // strafe right; now it is a full strafe left.
    let mut genes = vec![0.0; crate::learn::data::PERSONAL_GENES];
    genes[13] = -20.0;
    genes[14] = -20.0;
    rig.bot.learned.personal = crate::learn::BotGaValues::from_values(genes.clone());
    rig.bot.decide_jump_duck_strafe(&w, 60.0, Vector::new(60.0, 0.0, 0.0));
    assert_eq!(rig.bot.strafe_speed, -rig.bot.max_speed * (2.0 * rig.bot.learned.strafe.output() - 1.0).abs());
    assert!(rig.bot.strafe_speed < -0.9 * rig.bot.max_speed);
    genes[13] = 20.0;
    genes[14] = 20.0;
    rig.bot.learned.personal = crate::learn::BotGaValues::from_values(genes);
    rig.bot.decide_jump_duck_strafe(&w, 60.0, Vector::new(60.0, 0.0, 0.0));
    assert!(rig.bot.strafe_speed > 0.9 * rig.bot.max_speed);
}

#[test]
fn kung_fu_lives_dont_breed_the_personal_genes() {
    let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::Punch], 60.0);
    let mut rng = XorShiftRandom::new(2);
    run(&mut rig, &mut w, &mut rng, 50, |_, _| {});
    assert_eq!(rig.bot.kungfu.opponent(), Some(2));
    assert!(!rig.bot.used_melee);
}

#[test]
fn the_cheapest_cost_bucket_is_away_from_the_danger() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(200.0, 50.0, 0.0), 0)]);
    rig.bot.me = w.entity(1).unwrap();
    let mut rng = XorShiftRandom::new(1);
    rig.bot.learned.seed_survival(&mut rng);
    rig.bot.lowest_enemy_cost = 99999.0;
    let enemy = w.entity(2).unwrap();
    rig.bot.work_enemy_costs(&w, &enemy, 206.0);
    // Upstream: always origin + (-192, -192, origin.z). Now the bucket farthest from the enemy.
    let v = rig.bot.lowest_enemy_cost_vec;
    assert_eq!((v.x, v.y, v.z), (-192.0, -192.0, 0.0));
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(-200.0, 150.0, 0.0), 0)]);
    let enemy = w.entity(2).unwrap();
    rig.bot.range_costs = [[0.0; BOT_COST_BUCKETS]; BOT_COST_BUCKETS];
    rig.bot.work_enemy_costs(&w, &enemy, 250.0);
    let v = rig.bot.lowest_enemy_cost_vec;
    assert_eq!((v.x, v.y), (192.0, -192.0));
}

#[test]
fn time_in_sight_hits_and_rounds_go_to_the_weapon_table() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(9);
    let pin = |w: &Plane, _: &Frame| w.ents.borrow_mut()[0].origin = Vector::ZERO;
    run(&mut rig, &mut w, &mut rng, 100, pin);
    let seen = rig.bot.weapon_stats().samples(1, 300.0);
    assert!(seen > 0.5, "{seen}");
    assert!(rig.with(|b, g| b.learned_weapon_score(g, 1, 300.0)).is_none());
    // Two rounds fired, one hit for 24.
    arm(&w, 1, 1, 15, 34);
    run(&mut rig, &mut w, &mut rng, 3, pin);
    rig.with(|b, g| b.combat_event(g, &w, &damage(2, 1, 24.0), &mut rng));
    let t = rig.bot.weapon_stats();
    assert_eq!(t.accuracy(1, 300.0), Some(0.5));
    assert!(t.dps(1, 300.0).is_some_and(|(v, _)| v > 0.0));
    assert!(rig.learned.weapon_prior.samples(1, 300.0) >= seen);
    // A score once five seconds are in.
    run(&mut rig, &mut w, &mut rng, 250, pin);
    let score = rig.with(|b, g| b.learned_weapon_score(g, 1, 300.0));
    assert!(score.is_some_and(|s| s > 0.0), "{score:?}");
}

#[test]
fn replayed_fights_fill_the_ledger_heatmap_and_weapon_table() {
    let mut rig = Rig::new(Config::default());
    rig.learned.heatmap = crate::learn::Heatmap::new("ts_test", rig.wp.len());
    // The bot at waypoint 0, a player at waypoint 2 (300 units), another at 5 (750).
    let mut w =
        plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0), player(3, Vector::new(750.0, 0.0, 0.0), 0)]);
    arm(&w, 3, 18, 5, 10);
    let script =
        [(1.0, damage(1, 2, 30.0)), (1.2, damage(2, 1, 50.0)), (1.5, kill(1, 2, 1)), (3.0, damage(1, 3, 100.0)), (3.0, kill(3, 1, 18))];
    rig.replay(&mut w, &script);
    let l = rig.bot.ledger();
    assert_eq!(l.entries().count(), 5);
    assert!((l.reward_since(0.0, 2) - 1.2).abs() < 1e-6);
    assert_eq!(l.reward_since(0.0, 3), -2.0);
    assert!(l.open_episodes().is_empty());
    let h = &rig.learned.heatmap;
    assert_eq!((h.deaths_at(2, 0), h.kills_from(0, 0), h.deaths_at(0, 0), h.kills_from(5, 0)), (1.0, 1.0, 1.0, 1.0));
    assert!((h.damage_taken_at(0, 0) - 1.3).abs() < 1e-6 && (h.damage_taken_at(2, 0) - 0.5).abs() < 1e-6);
    assert!(h.danger(0, -1) > h.danger(1, -1));
    assert_eq!(rig.bot.weapon_stats().cell(1, 300.0).map(|c| c.hits), Some(1.0));
    assert_eq!(rig.learned.weapon_prior.cell(1, 300.0).map(|c| c.damage), Some(50.0));
}

#[test]
fn the_loadout_bought_is_credited_only_when_the_inventory_shows_it() {
    use crate::learn::loadout::LOADOUTS;
    let mut rig = Rig::new(Config::default());
    let dead = || {
        let mut d = player(1, Vector::ZERO, 0);
        d.health = 0.0;
        d.player.as_mut().unwrap().alive = false;
        d
    };
    let mut w = plane(vec![dead(), player(2, Vector::new(300.0, 0.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(6);
    let bought = |f: &Frame| f.actions.iter().find_map(|a| if let Action::Loadout(l) = a { Some(l.clone()) } else { None });
    // Dead: a curated loadout is bought.
    let ids = rig.with(|b, g| bought(&b.frame(g, &w, &mut rng))).expect("a loadout");
    let arm = LOADOUTS.iter().position(|l| *l == ids.as_slice()).expect("curated");
    // Alive with it: credited; a kill and 100 damage in 30 s, then a death.
    w.ents.borrow_mut()[0] = player(1, Vector::ZERO, 0);
    *w.inventory.borrow_mut() = owned(&[(ids[0], 10, 10), (ids[1], 7, 7)]);
    run(&mut rig, &mut w, &mut rng, 10, |_, _| {});
    assert_eq!(rig.bot.learning.credited_loadout, Some(arm));
    rig.with(|b, g| {
        b.combat_event(g, &w, &damage(2, 1, 100.0), &mut rng);
        b.combat_event(g, &w, &kill(1, 2, ids[0]), &mut rng);
    });
    w.time += 30.0;
    w.ents.borrow_mut()[0] = dead();
    let next = rig.with(|b, g| bought(&b.frame(g, &w, &mut rng)));
    let a = rig.bot.learning.loadout.arms[arm];
    // (1 kill - 0.5 death + 100/200) per minute over about half a minute.
    assert!((a.value - 2.0).abs() < 0.05 && a.n == 1.0, "{a:?}");
    assert_eq!(rig.learned.loadout_prior.arms[arm].n, 1.0);
    assert!(next.is_some(), "a new pick for the next life");
    // The buy didn't come through: no credit, nothing learned.
    w.ents.borrow_mut()[0] = player(1, Vector::ZERO, 0);
    *w.inventory.borrow_mut() = owned(&[(28, 10, 10)]);
    run(&mut rig, &mut w, &mut rng, 200, |_, _| {});
    assert_eq!(rig.bot.learning.credited_loadout, None);
    w.ents.borrow_mut()[0] = dead();
    let before = rig.bot.learning.loadout.arms.iter().map(|a| a.n).sum::<f32>();
    rig.with(|b, g| b.frame(g, &w, &mut rng));
    assert_eq!(rig.bot.learning.loadout.arms.iter().map(|a| a.n).sum::<f32>(), before);
}

#[test]
fn replayed_fights_teach_the_opponent_model() {
    use crate::learn::opponents::OpponentKey;
    let mut rig = Rig::new(Config::default());
    rig.extra.reps[2] = 20;
    rig.extra.reps[3] = 30;
    let mut w =
        plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(300.0, 0.0, 0.0), 0), player(3, Vector::new(750.0, 0.0, 0.0), 0)]);
    arm(&w, 3, 18, 5, 10);
    arm(&w, 2, 0, 100, 0);
    let script = [
        (1.0, damage(1, 2, 10.0)),
        (1.5, kill(1, 2, 1)),
        (3.0, damage(1, 3, 100.0)),
        (3.0, kill(3, 1, 18)),
        // Not about a bot: the model ignores it.
        (4.0, damage(2, 3, 50.0)),
    ];
    rig.replay(&mut w, &script);
    let m = &rig.learned.opponents;
    let sniper = m.profile(OpponentKey::Player(30)).unwrap();
    assert_eq!((sniper.hits_on_bots, sniper.kills_of_bots, sniper.range), (1.0, 1.0, 750.0));
    assert_eq!(sniper.favourite_weapon(), Some((18, 1.0)));
    let brawler = m.profile(OpponentKey::Player(20)).unwrap();
    assert_eq!((brawler.deaths_to_bots, brawler.kungfu_share), (1.0, 1.0));
    assert!(rig.with(|b, g| b.learned_threat(g, 3)) > rig.with(|b, g| b.learned_threat(g, 2)));
    assert_eq!(rig.with(|b, g| b.learned_threat(g, 4)), 1.0);
    // Time in the bot's sight counts as exposure, its sideways speed as dodging.
    w.ents.borrow_mut()[2].velocity = Vector::new(0.0, 250.0, 0.0);
    let mut rng = XorShiftRandom::new(3);
    w.ents.borrow_mut().remove(1);
    run(&mut rig, &mut w, &mut rng, 100, |w, _| w.ents.borrow_mut()[0].origin = Vector::ZERO);
    let sniper = rig.learned.opponents.profile(OpponentKey::Player(30)).unwrap();
    assert!(sniper.exposure > 0.5 && sniper.dodge > 0.5, "{sniper:?}");
}

#[test]
fn learning_replays_exactly_from_the_same_seed() {
    // The same fight and events with the same seed learn the same, to the saved text.
    let learn = || {
        let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::Punch, kungfu::Move::Kick], 60.0);
        rig.learned.heatmap = crate::learn::Heatmap::new("ts_test", rig.wp.len());
        let mut rng = XorShiftRandom::new(12);
        let mut n = 0;
        run(&mut rig, &mut w, &mut rng, 400, |w, _| {
            n += 1;
            w.ents.borrow_mut()[1].button = if n % 25 < 2 { buttons::ATTACK2 } else { 0 };
        });
        let t = w.time;
        rig.replay(&mut w, &[(t, damage(2, 1, 20.0)), (t + 0.5, damage(1, 2, 35.0)), (t + 1.0, kill(1, 2, 0))]);
        (
            rig.bot.kungfu.save(),
            rig.bot.weapon_stats().save(),
            rig.bot.learning.loadout.save(),
            rig.learned.opponents.save(),
            rig.learned.heatmap.to_bytes(),
        )
    };
    let a = learn();
    assert_eq!(a, learn());
    assert!(a.0.contains("stance all"), "{}", a.0);
}

/// A shipped TS waypoint file (`data/rcbot/waypoints/ts`).
fn real_map(name: &str) -> Waypoints {
    let path = format!("{}/../../data/rcbot/waypoints/ts/{name}.rcw", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(path).unwrap();
    Waypoints::from_rcw(&bytes, name).unwrap().0
}

/// The waypoints a bot may go to: not deleted, aiming or unreachable, and with a path out.
fn live_waypoints(wp: &Waypoints) -> Vec<usize> {
    (0..wp.len())
        .filter(|&i| wp.flags(i as i32) & (fl::DELETED | fl::AIMING | fl::UNREACHABLE) == 0 && !wp.successors(i, -1).is_empty())
        .collect()
}

/// The rig's bot on a real waypoint graph in the plane (no walls, no floors: each frame the bot is
/// put at the height of the waypoint it heads for). `frozen` frames don't move it (something in
/// the way); `each` sees every frame.
fn walk_real(
    rig: &mut Rig,
    w: &mut Plane,
    rng: &mut XorShiftRandom,
    frames: usize,
    frozen: impl Fn(usize) -> bool,
    mut each: impl FnMut(&Bot, usize),
) {
    for i in 0..frames {
        w.time += 0.02;
        let f = rig.with(|b, g| b.frame(g, w, rng));
        if let Some(cmd) = f.cmd {
            if frozen(i) {
                block(w, &cmd);
            } else {
                w.apply(1, &cmd);
            }
        }
        let z = usize::try_from(rig.bot.current_waypoint).ok().map(|c| rig.wp.origin(c as i32).z);
        if let Some(z) = z {
            let mut ents = w.ents.borrow_mut();
            let me = &mut ents[0];
            me.origin.z = z;
            me.absmin = me.origin - Vector::new(16.0, 16.0, 36.0);
            me.absmax = me.origin + Vector::new(16.0, 16.0, 36.0);
        }
        each(&rig.bot, i);
    }
}

/// Something in the bot's way: its usercmd turns it and presses its buttons, but it doesn't move.
fn block(w: &Plane, cmd: &BotCmd) {
    let mut ents = w.ents.borrow_mut();
    let me = &mut ents[0];
    me.v_angle = cmd.angles;
    me.button = cmd.buttons;
    me.velocity = Vector::ZERO;
}

/// A bot on `map` at its first live waypoint.
fn real_rig(map: &str) -> (Rig, Plane) {
    let mut rig = Rig::new(Config::default());
    rig.wp = real_map(map);
    let start = rig.wp.origin(live_waypoints(&rig.wp)[0] as i32);
    (rig, plane(vec![player(1, start, 0)]))
}

#[test]
fn roaming_goals_cover_the_whole_map() {
    // Upstream drew roaming goals from the first 50 waypoints only (17-37% of these maps).
    for map in ["ts_dojo", "ts_lobby", "ts_awaken", "ts_theatre"] {
        let (mut rig, _) = real_rig(map);
        let live = live_waypoints(&rig.wp);
        let mut rng = XorShiftRandom::new(11);
        let mut seen = std::collections::HashSet::new();
        let mut upstream = std::collections::HashSet::new();
        let mut now = 10.0;
        for _ in 0..live.len() * 12 {
            now += 1.0;
            let goal = rig.with(|b, g| b.roam_goal(g, now, &mut rng));
            assert!(goal >= 0, "{map}");
            seen.insert(goal as usize);
            rig.bot.explore.touch(goal, now);
            upstream.insert(rig.wp.find_random_goal(-1, 0, &[], &mut rng).unwrap());
        }
        let covered = live.iter().filter(|i| seen.contains(i)).count();
        assert!(covered * 100 >= live.len() * 95, "{map}: {covered} of {}", live.len());
        assert!(upstream.len() <= 50, "{map}");
    }
}

#[test]
fn a_bot_roams_over_a_real_map() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let live = live_waypoints(&rig.wp);
    let mut rng = XorShiftRandom::new(5);
    let (mut touched, mut goals) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    walk_real(
        &mut rig,
        &mut w,
        &mut rng,
        9000,
        |_| false,
        |b, _| {
            touched.insert(b.current_waypoint);
            goals.insert(b.goal_waypoint);
        },
    );
    // Three minutes: half the map, and goals all over it (upstream's could only be the first 50
    // waypoints).
    let touched = live.iter().filter(|&&i| touched.contains(&(i as i32))).count();
    assert!(touched * 2 >= live.len(), "{touched} of {} waypoints", live.len());
    assert!(goals.iter().filter(|&&g| g >= 50).count() >= 15, "{goals:?}");
    assert_eq!(rig.bot.stuck_suicides, 0);
}

#[test]
fn a_stuck_bot_goes_back_once_then_fails_the_path_within_3_s() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let mut rng = XorShiftRandom::new(8);
    // Walk until the bot is on its way between two waypoints.
    let mut frames = 0;
    let between = |b: &Bot, wp: &Waypoints| {
        b.prev_waypoint >= 0
            && b.current_waypoint >= 0
            && b.prev_waypoint != b.current_waypoint
            && wp.distance(b.me.origin, b.prev_waypoint) > 64.0
            && wp.distance(b.me.origin, b.current_waypoint) > 64.0
    };
    while !between(&rig.bot, &rig.wp) && frames < 3000 {
        walk_real(&mut rig, &mut w, &mut rng, 1, |_| false, |_, _| {});
        frames += 1;
    }
    let (prev, cur) = (rig.bot.prev_waypoint, rig.bot.current_waypoint);
    assert!(prev >= 0 && cur >= 0, "never got going");
    // Something blocks the way for 3 s.
    let mut currents = Vec::new();
    let mut failed_at = None;
    walk_real(
        &mut rig,
        &mut w,
        &mut rng,
        150,
        |_| true,
        |b, i| {
            currents.push(b.current_waypoint);
            // The 2 s step: the path it was stuck on fails (A* forgets it again at once when no
            // other way is left) and the next search starts from another waypoint.
            if failed_at.is_none() && b.unstick.last > 0.0 {
                assert_eq!(b.unstick.edge, (prev, cur));
                failed_at = Some(i);
            }
        },
    );
    // Back to the previous waypoint once (upstream: one index down each frame for a second).
    let before_fail = &currents[..failed_at.unwrap_or(currents.len())];
    assert!(before_fail.iter().all(|&c| c == prev || c == cur), "{prev} -> {cur}: {before_fail:?}");
    let failed_at = failed_at.expect("the path it was stuck on fails");
    assert!(failed_at < 150, "{failed_at}");
    assert!(!rig.bot.kill);
}

#[test]
fn stuck_again_at_the_same_place_escalates_and_never_kills_in_a_fight() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let mut rng = XorShiftRandom::new(8);
    walk_real(&mut rig, &mut w, &mut rng, 100, |_| false, |_, _| {});
    let mut levels = Vec::new();
    let mut backed_up = false;
    let mut dived_or_wall_jumped = false;
    for _ in 0..800 {
        w.time += 0.02;
        let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
        if let Some(cmd) = f.cmd {
            backed_up |= rig.bot.unstick.back_until > w.time;
            dived_or_wall_jumped |= cmd.buttons & buttons::ALT1 != 0;
            block(&w, &cmd);
        }
        if levels.last() != Some(&rig.bot.unstick.level) {
            levels.push(rig.bot.unstick.level);
        }
    }
    assert!(levels.contains(&3), "{levels:?}");
    assert!(backed_up && dived_or_wall_jumped, "{backed_up} {dived_or_wall_jumped} {levels:?}");
    // 16 s without moving: not yet the last resort (upstream killed the bot after 8 s).
    assert_eq!(rig.bot.stuck_suicides, 0);
    let mut killed_at = None;
    for _ in 0..2000 {
        w.time += 0.02;
        let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
        if f.actions.contains(&Action::Kill) {
            killed_at = Some(w.time);
            break;
        }
        if let Some(cmd) = f.cmd {
            block(&w, &cmd);
        }
    }
    assert_eq!(rig.bot.stuck_suicides, 1, "killed at {killed_at:?}");
    // Blocked since about 3 s.
    assert!(killed_at.unwrap() > 3.0 + movement::STUCK_SUICIDE);
}

#[test]
fn a_dive_holds_duck_through_the_landing_and_gets_up_if_prone() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    rig.bot.me = w.entity(1).unwrap();
    rig.bot.dive(1.0);
    assert_ne!(rig.bot.button & buttons::ALT1, 0);
    // Duck held from just after the dive through the landing.
    for t in [1.2, 1.8, 2.4] {
        let cmd = rig.bot.run_player_move(t);
        assert_ne!(cmd.buttons & buttons::DUCK, 0, "{t}");
    }
    // Landed prone anyway: it jumps up.
    rig.bot.state.stunt_state = TS_STATE_STUNT;
    rig.bot.dive_landing(2.0);
    let cmd = rig.bot.run_player_move(2.05);
    assert_ne!(cmd.buttons & buttons::JUMP, 0);
    assert_eq!(rig.bot.dive_time, 0.0);
}

/// An item lying at `at`: a `ts_powerup` with `bits` or a `ts_groundweapon` of `weapon`.
fn item_at(index: u32, at: Vector, powerup: i32, weapon: i32) -> Entity {
    let class = if weapon != 0 { "ts_groundweapon" } else { "ts_powerup" };
    let mut e = brush(index, class, at, Vector::new(16.0, 16.0, 16.0));
    e.solid = SOLID_TRIGGER;
    let kind = if weapon != 0 { ItemKind::GroundWeapon } else { ItemKind::Powerup };
    e.item = Some(ItemInfo { kind, weapon, powerup, available: true, ..ItemInfo::default() });
    e
}

#[test]
fn items_are_worth_what_the_bot_can_use() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.state = b.me.player.unwrap();
        // Unarmed with room: any gun is worth a lot, a better one more.
        b.state.free_slots = 81;
        let (glock, m4) = (b.weapon_value(g, 1), b.weapon_value(g, 5));
        assert!(glock >= 60.0 && m4 > glock, "{glock} {m4}");
        // No room for a Barrett (70 slots).
        b.state.free_slots = 40;
        assert_eq!(b.weapon_value(g, 18), 0.0);
        // Armed with a Glock: an M4 is better, another pistol isn't.
        b.weapons.weapon_info(1, 17, 34, g.weapons, g.presets);
        assert!(b.weapon_value(g, 5) > 0.0);
        assert_eq!(b.weapon_value(g, 9), 0.0);
        // Its own gun low on ammo: worth picking up for the ammo.
        b.weapons.weapon_info(1, 17, 0, g.weapons, g.presets);
        assert!(b.weapon_value(g, 1) > 0.0);
        // Power-ups: none while one is held (taking one with `+use` swaps it), health when hurt.
        assert!(b.powerup_value(1) > 0.0);
        b.state.stored_powerup = 4;
        assert_eq!(b.powerup_value(1), 0.0);
        assert_eq!(b.powerup_value(64), 0.0);
        b.me.health = 40.0;
        assert_eq!(b.powerup_value(64), 60.0);
    });
}

#[test]
fn an_item_behind_the_bot_is_remembered_and_gone_for() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let live = live_waypoints(&rig.wp);
    let me = w.entity(1).unwrap().origin;
    // The farthest waypoint from the start: certainly not in view.
    let far = *live.iter().max_by(|&&a, &&b| rig.wp.distance(me, a as i32).total_cmp(&rig.wp.distance(me, b as i32))).unwrap();
    let at = rig.wp.origin(far as i32);
    w.ents.borrow_mut().push(item_at(20, at, 2, 0));
    let mut rng = XorShiftRandom::new(4);
    let mut went = false;
    walk_real(
        &mut rig,
        &mut w,
        &mut rng,
        50,
        |_| false,
        |b, _| {
            went |= b.tasks().iter().any(|t| t.kind == TaskKind::PickupItem);
        },
    );
    assert!(went, "{:?}", rig.bot.tasks().iter().map(|t| t.kind).collect::<Vec<_>>());
}

#[test]
fn an_item_the_game_wont_give_is_tried_once_then_left_alone() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let me = w.entity(1).unwrap().origin;
    // A power-up near the start that the fake game never hands over.
    let live = live_waypoints(&rig.wp);
    let near = *live
        .iter()
        .filter(|&&i| rig.wp.distance(me, i as i32) > 150.0)
        .min_by(|&&a, &&b| rig.wp.distance(me, a as i32).total_cmp(&rig.wp.distance(me, b as i32)))
        .unwrap();
    let item = item_at(20, rig.wp.origin(near as i32), 2, 0);
    w.ents.borrow_mut().push(item.clone());
    let mut rng = XorShiftRandom::new(4);
    let (mut tries, mut was_picking) = (0, false);
    walk_real(
        &mut rig,
        &mut w,
        &mut rng,
        3000,
        |_| false,
        |b, _| {
            let picking = b.tasks().current().is_some_and(|t| t.kind == TaskKind::PickupItem);
            if picking && !was_picking {
                tries += 1;
            }
            was_picking = picking;
        },
    );
    // Upstream pressed use for 10 s, waited 5-10 s and went back, for the whole minute.
    assert!((1..=2).contains(&tries), "{tries} tries");
    assert!(rig.bot.items.ignored(item.handle(), w.time) || rig.bot.items.get(item.handle()).is_some_and(|k| k.ignore_until > 0.0));
}

#[test]
fn a_sound_heard_is_gone_to_and_searched_around() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let live = live_waypoints(&rig.wp);
    let me = w.entity(1).unwrap().origin;
    // A sound 400-600 units away (within hearing), from an enemy that then goes out of the world.
    let at = live.iter().map(|&i| rig.wp.origin(i as i32)).find(|o| (400.0..600.0).contains(&(*o - me).length())).unwrap();
    let mut mgr = BotManager::default();
    w.ents.borrow_mut().push(player(2, at, 0));
    rig.bot.me = w.entity(1).unwrap();
    mgr.add(std::mem::replace(&mut rig.bot, setup(false).1));
    mgr.sound(&w, 2, Some(at), 1.0, "player/pl_step1.wav");
    rig.bot = mgr.remove(1).unwrap();
    w.ents.borrow_mut().retain(|e| e.index != 2);
    let mut rng = XorShiftRandom::new(2);
    let (mut nearest, mut searched_moving) = (f32::MAX, false);
    walk_real(
        &mut rig,
        &mut w,
        &mut rng,
        1000,
        |_| false,
        |b, _| {
            nearest = nearest.min((b.me.origin - at).length_2d());
            searched_moving |=
                b.tasks().current().is_some_and(|t| t.kind == TaskKind::SearchForEnemy && t.vector != Vector::ZERO) && b.move_to_valid;
        },
    );
    assert!(nearest < 150.0, "came within {nearest} of the sound");
    assert!(searched_moving);
}

#[test]
fn an_enemy_lost_is_hunted_where_it_may_have_gone() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let live = live_waypoints(&rig.wp);
    let me = w.entity(1).unwrap().origin;
    let at = live.iter().map(|&i| rig.wp.origin(i as i32)).find(|o| (500.0..800.0).contains(&(*o - me).length())).unwrap();
    let mut rng = XorShiftRandom::new(3);
    walk_real(&mut rig, &mut w, &mut rng, 1, |_| false, |_, _| {});
    rig.bot.tasks.flush();
    // Seen 4 s ago, then gone.
    rig.bot.remembered.tick(w.time - 4.0);
    rig.bot.remembered.add(at, Some(EntRef { index: 3, serial: 1 }), 0, me);
    rig.bot.remembered.tick(w.time);
    let mut hunted = None;
    walk_real(
        &mut rig,
        &mut w,
        &mut rng,
        400,
        |_| false,
        |b, _| {
            if hunted.is_none() && b.looking_for_enemy {
                hunted = b.tasks().iter().find(|t| t.kind == TaskKind::FindPath).map(|t| t.int);
            }
        },
    );
    let goal = hunted.expect("it went after the enemy");
    // Where the enemy could have walked in 4 s (up to 800 units on), not where it was.
    let d = (rig.wp.origin(goal) - at).length();
    assert!(d < 900.0, "{goal} is {d} from where it was seen");
    // The position isn't hunted twice, and it is forgotten after 15 s.
    assert!(rig.bot.remembered.iter().all(|p| p.hunted));
    rig.bot.remembered.tick(w.time + 20.0);
    assert!(!rig.bot.remembered.any());
}

#[test]
fn a_dead_enemy_is_forgotten() {
    let mut mgr = BotManager::default();
    let (_, mut bot, mut wp, mut weapons, presets, mut learned, config) = setup(true);
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(400.0, 0.0, 0.0), 0)]);
    bot.me = w.entity(1).unwrap();
    bot.remembered.add(Vector::new(400.0, 0.0, 0.0), Some(w.entity(2).unwrap().handle()), 0, Vector::ZERO);
    mgr.add(bot);
    let mut extra = Extra::default();
    let mut g = BotGlobals {
        config: &config,
        waypoints: &mut wp,
        weapons: &mut weapons,
        presets: &presets,
        learned: &mut learned,
        team_scores: [0; 4],
        masters: &[],
        other_bots: Vec::new(),
        squads: &mut extra.squads,
        botcam: &mut extra.cam,
        rep_ids: &extra.reps,
        chat: &extra.chat,
        map_name: "test",
        danger: None,
    };
    mgr.death(&mut g, &w, 0, 2, &mut XorShiftRandom::new(1));
    assert!(!mgr.get(1).unwrap().remembered.any());
}

#[test]
fn pre_aim_checks_what_comes_into_view() {
    // A path 0 -> 1 along x; 2 is seen from 1 only, 3 from both, 4 from neither.
    let mut rig = Rig::new(Config::default());
    let mut wp = Waypoints::new();
    for o in [(0.0, 0.0), (300.0, 0.0), (400.0, 400.0), (300.0, -400.0), (600.0, 600.0)] {
        wp.add_origin(Vector::new(o.0, o.1, 0.0), 0, None);
    }
    for (a, b) in [(0, 1), (1, 2), (1, 3), (0, 3), (1, 4)] {
        wp.visibility.set(a, b, true);
        wp.visibility.set(b, a, true);
    }
    wp.visibility.set(4, 1, false);
    wp.visibility.set(1, 4, false);
    rig.wp = wp;
    let mut rng = XorShiftRandom::new(1);
    let (t, standing) = rig.with(|b, g| (b.pre_aim_target(g, 0, 1, 5.0, &mut rng), b.pre_aim_target(g, 1, 1, 5.0, &mut rng)));
    assert_eq!(t, Some(2));
    // Standing at 1: what 1 sees.
    assert!(matches!(standing, Some(2 | 3)), "{standing:?}");
    // A danger map makes the dangerous one first.
    struct Hot;
    impl crate::nav::DangerSource for Hot {
        fn danger(&self, w: usize, _team: i32) -> f32 {
            if w == 3 { 1.0 } else { 0.0 }
        }
    }
    let g = BotGlobals {
        config: &rig.config,
        waypoints: &mut rig.wp,
        weapons: &mut rig.weapons,
        presets: &rig.presets,
        learned: &mut rig.learned,
        team_scores: [0; 4],
        masters: &[],
        other_bots: Vec::new(),
        squads: &mut rig.extra.squads,
        botcam: &mut rig.extra.cam,
        rep_ids: &rig.extra.reps,
        chat: &rig.extra.chat,
        map_name: "ts_test",
        danger: Some(&Hot),
    };
    assert_eq!(rig.bot.pre_aim_target(&g, 1, 1, 5.0, &mut rng), Some(3));
    // Looked at, it waits its turn.
    rig.bot.current_waypoint = 1;
    rig.bot.look_task = LookTask::PreAim;
    rig.bot.update_pre_aim(&g, 5.0, &mut rng);
    assert_eq!(rig.bot.pre_aim.point, Vector::new(300.0, -400.0, 8.0));
    rig.bot.update_pre_aim(&g, 7.0, &mut rng);
    assert_ne!(rig.bot.pre_aim.point, Vector::new(300.0, -400.0, 8.0));
}

/// A CTB world: the plane with game mode CTB.
struct Ctb(Plane);

impl World for Ctb {
    fn trace_line(&self, s: Vector, e: Vector, m: Monsters, g: Glass, i: Option<u32>) -> Trace {
        self.0.trace_line(s, e, m, g, i)
    }
    fn point_contents(&self, at: Vector) -> i32 {
        self.0.point_contents(at)
    }
}

impl BotWorld for Ctb {
    fn time(&self) -> f32 {
        self.0.time()
    }
    fn max_entities(&self) -> u32 {
        self.0.max_entities()
    }
    fn max_clients(&self) -> u32 {
        self.0.max_clients()
    }
    fn entity(&self, index: u32) -> Option<Entity> {
        self.0.entity(index)
    }
    fn in_pvs(&self, v: u32, o: Vector) -> bool {
        self.0.in_pvs(v, o)
    }
    fn entities_in_sphere(&self, o: Vector, r: f32) -> Vec<u32> {
        self.0.entities_in_sphere(o, r)
    }
    fn teamplay(&self) -> bool {
        true
    }
    fn find_entities(&self, key: EntityKey, value: &str) -> Vec<u32> {
        self.0.find_entities(key, value)
    }
    fn game_mode(&self) -> GameMode {
        GameMode { mode: 1, ctb: true, round: 0 }
    }
}

#[test]
fn ctb_bots_take_the_case_home_and_escort_or_cut_off_carriers() {
    let ctb_ent = |index: u32, class: &str, x: f32, kind: ItemKind, team: i32| {
        let mut e = brush(index, class, Vector::new(x, 0.0, 0.0), Vector::new(16.0, 16.0, 16.0));
        e.item = Some(ItemInfo { kind, team, available: true, ..ItemInfo::default() });
        e
    };
    let world = |me: Entity, others: Vec<Entity>| {
        let mut ents = vec![
            me,
            ctb_ent(5, "ts_flag", 600.0, ItemKind::CtbFlag, 2),
            ctb_ent(6, "ts_briefcase", 0.0, ItemKind::CtbBase, 1),
            ctb_ent(7, "ts_briefcase", 750.0, ItemKind::CtbBase, 2),
        ];
        ents.extend(others);
        Ctb(plane(ents))
    };
    let tasks = |w: &Ctb, rig: &mut Rig| {
        let mut rng = XorShiftRandom::new(1);
        rig.with(|b, g| {
            b.me = w.entity(1).unwrap();
            b.state = b.me.player.unwrap();
            b.tasks.flush();
            b.look_for_new_tasks(g, w, &mut rng);
            b.tasks().iter().map(|t| (t.kind, t.edict.map(|e| e.index))).collect::<Vec<_>>()
        })
    };
    let mut rig = Rig::new(Config::default());
    let me = player(1, Vector::new(300.0, 0.0, 0.0), 1);
    // The other team's case lies there: go and take it.
    let w = world(me.clone(), vec![]);
    assert_eq!(tasks(&w, &mut rig)[0], (TaskKind::FindPath, Some(5)));
    // Carrying it: home to team 1's point.
    let mut carrying = me.clone();
    carrying.player.as_mut().unwrap().has_flag = true;
    assert_eq!(tasks(&world(carrying, vec![]), &mut rig)[0], (TaskKind::FindPath, Some(6)));
    // A teammate carries it far away: escort; an enemy does: wait at its point.
    let mut mate = player(2, Vector::new(-400.0, 0.0, 0.0), 1);
    mate.player.as_mut().unwrap().has_flag = true;
    assert_eq!(tasks(&world(me.clone(), vec![mate]), &mut rig)[0], (TaskKind::FindPath, Some(2)));
    let mut enemy = player(3, Vector::new(700.0, 0.0, 0.0), 2);
    enemy.player.as_mut().unwrap().has_flag = true;
    assert_eq!(tasks(&world(me, vec![enemy]), &mut rig)[0], (TaskKind::FindPath, Some(7)));
}

#[test]
fn stealth_squads_and_stay_near_waypoints_slow_down_at_once() {
    let mut rig = Rig::new(Config::default());
    let leader = player(2, Vector::new(-100.0, 0.0, 0.0), 0);
    let w = plane(vec![player(1, Vector::ZERO, 0), leader.clone()]);
    let speed = |rig: &mut Rig| {
        rig.with(|b, g| {
            b.me = w.entity(1).unwrap();
            b.ideal_move_speed = b.max_speed;
            b.set_move_vector(Vector::new(500.0, 0.0, 0.0));
            b.work_move_direction(&w, g);
            b.move_speed.hypot(b.strafe_speed)
        })
    };
    assert!((speed(&mut rig) - 290.0).abs() < 1.0);
    // A stay-near waypoint: half speed in the same frame (upstream: the next one).
    rig.bot.current_waypoint_flags = fl::STAY_NEAR;
    assert!(speed(&mut rig) <= 145.5);
    rig.bot.current_waypoint_flags = 0;
    // A stealthy squad: half speed (upstream overwrote it before it was read, design §5.62).
    let me = w.entity(1).unwrap();
    let id = rig.extra.squads.add_member(&leader, &me, false).unwrap();
    rig.extra.squads.get_mut(id).unwrap().set_combat_type(squad::CombatType::Stealth);
    rig.bot.squad = Some(id);
    assert!(speed(&mut rig) <= 145.5);
}

#[test]
fn teammates_share_sightings_claims_and_paths() {
    let mut rig = Rig::new(Config::default());
    for i in 0..6 {
        for j in 0..6 {
            rig.wp.visibility.set(i, j, true);
        }
    }
    let mut w = plane(vec![player(1, Vector::ZERO, 1), player(3, Vector::new(900.0, 0.0, 0.0), 1)]);
    w.teamplay = true;
    let enemies = [EntRef { index: 5, serial: 1 }, EntRef { index: 6, serial: 1 }];
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.state = b.me.player.unwrap();
        b.now = 10.0;
        let board = g.squads.boards.get_mut(1).unwrap();
        // Teammate 3 goes to waypoint 4 by 2, 3, 4 and goes for an item.
        board.post(3, Vector::new(1200.0, 0.0, 0.0), &[4, 3, 2], 10.0);
        board.claim(board::Claim { by: 3, waypoint: 4, item: Some(EntRef { index: 9, serial: 1 }), until: 40.0 });
        assert_eq!(b.crowded_waypoints(g), vec![2, 3, 4]);
        assert!(b.claim_factor(g, 4) < 0.25 && b.claim_factor(g, 0) == 1.0);
        assert_eq!(b.claim_item_factor(g, EntRef { index: 9, serial: 1 }), 0.1);
        // Two enemies seen near the bot, no teammate near: regroup with the teammate.
        assert_eq!(b.regroup_point(g, 10.0), None);
        for (i, e) in enemies.iter().enumerate() {
            g.squads.boards.get_mut(1).unwrap().sight(3, *e, Vector::new(200.0, 100.0 * i as f32, 0.0), 9.0);
        }
        assert_eq!(b.regroup_point(g, 10.0), Some(Vector::new(1200.0, 0.0, 0.0)));
        // A teammate's sighting is worth going to see; its sightings are threats for tactical paths.
        let (goal, _) = b.choose_goal(g, &w, 10.0);
        assert!(matches!(goal, goals::Goal::Regroup(_)), "{goal:?}");
        assert!(!b.threat_waypoints(g, &w).is_empty());
        // Deathmatch: no board.
        b.state.team = 0;
        assert_eq!(b.claim_factor(g, 4), 1.0);
    });
}

#[test]
fn a_bot_never_stands_at_a_reached_goal() {
    let (mut rig, mut w) = real_rig("ts_dojo");
    let mut rng = XorShiftRandom::new(5);
    let (mut still, mut last) = (0, Vector::ZERO);
    walk_real(
        &mut rig,
        &mut w,
        &mut rng,
        6000,
        |_| false,
        |b, _| {
            if (b.me.origin - last).length_2d() < 0.5 {
                still += 1;
            }
            last = b.me.origin;
        },
    );
    // Upstream stopped at every goal (`StopMoving` and half a second without a waypoint): about
    // one frame in ten on this map.
    assert!(still * 100 < 6000 * 4, "{still} of 6000 frames standing");
}

#[test]
fn a_sniper_holds_a_spot_that_sees_far() {
    let mut rig = Rig::new(Config::default());
    // Waypoint 3 sees all the others; the rest see only their neighbours.
    for i in 0..12 {
        rig.wp.add_origin(Vector::new(900.0 + i as f32 * 100.0, 300.0, 0.0), 0, None);
    }
    for j in 0..rig.wp.len() {
        rig.wp.visibility.set(3, j, true);
    }
    let w = plane(vec![player(1, Vector::new(300.0, 0.0, 0.0), 0)]);
    rig.with(|b, g| {
        b.me = w.entity(1).unwrap();
        b.state = b.me.player.unwrap();
        b.current_weapon = Some(1);
        assert!(b.hold_spot(g, 5.0).is_none(), "a pistol roams");
        b.current_weapon = Some(18);
        let (spot, _) = b.hold_spot(g, 5.0).unwrap();
        assert_eq!(spot, 3);
        let mut rng = XorShiftRandom::new(2);
        b.look_for_new_tasks(g, &w, &mut rng);
        let kinds: Vec<_> = b.tasks().iter().map(|t| (t.kind, t.int)).collect();
        assert_eq!(kinds, [(TaskKind::FindPath, 3), (TaskKind::Defend, 0)]);
        // Then it roams a while before holding again.
        assert!(b.hold_again >= 16.0);
    });
}

#[test]
fn the_games_view_and_teleports_come_from_the_command_result() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
    let mut rng = XorShiftRandom::new(4);
    run(&mut rig, &mut w, &mut rng, 30, |_, _| {});
    // The game turned the bot (a spawn's `fixangle`): its view is the game's from the next frame
    // on, though the entity still shows the old one (the server writes `v_angle` at the next
    // command), so it doesn't turn back against the game's turn.
    rig.bot.command_ran(CmdOutcome { game_view: Some(Vector::new(0.0, 90.0, 0.0)), teleported: false });
    w.ents.borrow_mut()[0].v_angle = Vector::ZERO;
    w.time += 0.02;
    let f = rig.with(|b, g| b.frame(g, &w, &mut rng));
    let yaw = f.cmd.unwrap().angles.y;
    assert!(yaw > 45.0, "{yaw}");
    // A teleport drops the path.
    rig.bot.current_waypoint = 2;
    rig.bot.command_ran(CmdOutcome { game_view: None, teleported: true });
    assert_eq!(rig.bot.current_waypoint(), -1);
}

#[test]
fn the_inventory_is_read_again_when_its_serial_changes() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
    let mut rng = XorShiftRandom::new(4);
    *w.inventory.borrow_mut() = Some(vec![InventoryWeapon { weapon: 1, clip: 17, ammo: 34 }]);
    w.ents.borrow_mut()[0].player.as_mut().unwrap().inventory_serial = 1;
    // Only the weapon sync reads it here (not the loadout credit).
    rig.bot.learning.credit_checked = true;
    run(&mut rig, &mut w, &mut rng, 1, |_, _| {});
    let first = w.inventory_reads.get();
    assert_eq!(first, 1, "read at the first frame");
    // Three seconds with nothing changed: no reads (v5's world read it every second).
    run(&mut rig, &mut w, &mut rng, 150, |_, _| {});
    assert_eq!(w.inventory_reads.get(), first);
    // The game's inventory changed: read at the next frame.
    *w.inventory.borrow_mut() =
        Some(vec![InventoryWeapon { weapon: 1, clip: 17, ammo: 34 }, InventoryWeapon { weapon: 5, clip: 30, ammo: 60 }]);
    w.ents.borrow_mut()[0].player.as_mut().unwrap().inventory_serial = 2;
    run(&mut rig, &mut w, &mut rng, 1, |_, _| {});
    assert_eq!(w.inventory_reads.get(), first + 1);
    assert!(rig.bot.weapons.has(5));
    // A world without serials: every second, as before.
    w.ents.borrow_mut()[0].player.as_mut().unwrap().inventory_serial = 0;
    let before = w.inventory_reads.get();
    run(&mut rig, &mut w, &mut rng, 150, |_, _| {});
    assert!((2..=4).contains(&(w.inventory_reads.get() - before)), "{}", w.inventory_reads.get() - before);
}

#[test]
fn the_vision_pass_ends_at_the_highest_entity() {
    let mut rig = Rig::new(Config::default());
    let mut item = brush(7, "ts_powerup", Vector::new(200.0, 0.0, 0.0), Vector::new(16.0, 16.0, 16.0));
    item.solid = 1;
    let w = plane(vec![player(1, Vector::ZERO, 0), item]);
    let mut rng = XorShiftRandom::new(4);
    rig.bot.profile.vision_revs = 2;
    rig.bot.me = w.entity(1).unwrap();
    let pass = |rig: &mut Rig, w: &Plane, rng: &mut XorShiftRandom| {
        let mut n = 0;
        while !rig.with(|b, g| b.update_visibles(g, w, rng)) {
            n += 1;
        }
        n
    };
    let full = pass(&mut rig, &w, &mut rng);
    assert!(rig.bot.is_visible(7));
    // Nothing above index 3 is in use: a pass takes fewer calls, and what it saw above is gone.
    w.highest.set(3);
    let short = pass(&mut rig, &w, &mut rng);
    assert!(short < full, "{short} {full}");
    assert!(!rig.bot.is_visible(7));
}

/// Sets player `index`'s close-combat and stunt timers as the port reports them.
fn timers(w: &Plane, index: u32, c: CombatState) {
    w.ents.borrow_mut().iter_mut().find(|e| e.index == index).unwrap().player.as_mut().unwrap().combat = Some(c);
}

#[test]
fn kung_fu_waits_out_the_opponents_block_and_its_own_recovery() {
    let presses = |rig: &mut Rig, w: &mut Plane, rng: &mut XorShiftRandom, frames: usize| {
        let mut n = 0;
        run(rig, w, rng, frames, |w, f| {
            n += usize::from(f.cmd.is_some_and(|c| c.buttons & (buttons::ATTACK | buttons::ATTACK2) != 0));
            w.ents.borrow_mut()[1].origin = Vector::new(60.0, 0.0, 0.0);
        });
        n
    };
    // The opponent's block is up (the game's window, not a `+reload` it was seen to press): the
    // bot doesn't punch into it.
    let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::Punch], 60.0);
    let mut rng = XorShiftRandom::new(2);
    timers(&w, 1, CombatState { melee_power: 100.0, ..CombatState::default() });
    timers(&w, 2, CombatState { melee_power: 100.0, block_kungfu: 1000.0, ..CombatState::default() });
    assert_eq!(presses(&mut rig, &mut w, &mut rng, 100), 0);
    // The block ends: it attacks.
    timers(&w, 2, CombatState { melee_power: 100.0, ..CombatState::default() });
    assert!(presses(&mut rig, &mut w, &mut rng, 100) > 0);
    // Its own next attack is far off (the game's `next_attack_at`): no presses the game would drop.
    let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::Punch], 60.0);
    timers(&w, 1, CombatState { melee_power: 100.0, next_attack_at: 1000.0, ..CombatState::default() });
    assert_eq!(presses(&mut rig, &mut w, &mut rng, 100), 0);
    // The power is the game's bar, whatever is in hand.
    rig.bot.state = PlayerState {
        alive: true,
        weapon: 5,
        clip: 30,
        combat: Some(CombatState { melee_power: 40.0, ..Default::default() }),
        ..Default::default()
    };
    assert_eq!(rig.bot.kungfu_power_for_tests(3.0), 40.0);
}

#[test]
fn an_opponents_hit_window_is_answered_without_a_fresh_press() {
    // The opponent holds its attack (no new press for the button watcher), but the game shows a new
    // hit window each time: the bot reacts to it.
    let (mut rig, mut w) = kung_fu_duel(&[kungfu::Move::Punch], 60.0);
    let mut rng = XorShiftRandom::new(6);
    timers(&w, 1, CombatState { melee_power: 100.0, ..CombatState::default() });
    w.ents.borrow_mut()[1].button = buttons::ATTACK;
    w.ents.borrow_mut()[1].v_angle = Vector::new(0.0, 180.0, 0.0);
    let mut reacted = false;
    for i in 0..100 {
        let t = w.time;
        timers(&w, 2, CombatState { melee_power: 100.0, melee_window: t + 0.2 + (i / 20) as f32, ..CombatState::default() });
        run(&mut rig, &mut w, &mut rng, 1, |w, _| w.ents.borrow_mut()[1].origin = Vector::new(60.0, 0.0, 0.0));
        reacted |= rig.bot.kungfu.reacting_for_tests();
    }
    assert!(reacted);
}

#[test]
fn dives_wait_for_the_stunt_cooldown_and_get_up_when_the_game_allows() {
    let mut rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    rig.bot.me = w.entity(1).unwrap();
    rig.bot.state = w.entity(1).unwrap().player.unwrap();
    // Cooling down: no press (it would be lost), no duck held for a dive that didn't happen.
    rig.bot.state.combat = Some(CombatState { next_stunt_at: 5.0, ..CombatState::default() });
    assert!(!rig.bot.dive(1.0));
    assert_eq!(rig.bot.button & buttons::ALT1, 0);
    assert!(rig.bot.dive(5.0));
    assert_ne!(rig.bot.button & buttons::ALT1, 0);
    // Landed prone: up as soon as the game's prone lock ends, not after a guessed delay.
    rig.bot.state.flags |= pflag::PRONE;
    rig.bot.state.combat = Some(CombatState { prone_lock: 5.4, ..CombatState::default() });
    rig.bot.dive_landing(5.3);
    assert_ne!(rig.bot.dive_time, 0.0, "still locked");
    rig.bot.dive_landing(5.45);
    assert_eq!(rig.bot.dive_time, 0.0);
    assert_ne!(rig.bot.run_player_move(5.46).buttons & buttons::JUMP, 0);
}

#[test]
fn a_prone_bot_strafes_with_the_strafe_keys_so_it_rolls() {
    let mut rig = Rig::new(Config::default());
    let strafe = buttons::MOVELEFT | buttons::MOVERIGHT;
    // Standing: the side speed alone, as before.
    rig.bot.strafe_speed = -200.0;
    assert_eq!(rig.bot.run_player_move(1.0).buttons & strafe, 0);
    // Prone: the key of the side it strafes to (TS's side roll), none without a side speed.
    rig.bot.state.flags |= pflag::PRONE;
    let cmd = rig.bot.run_player_move(1.1);
    assert_eq!((cmd.buttons & strafe, cmd.side), (buttons::MOVELEFT, -200.0));
    rig.bot.strafe_speed = 150.0;
    assert_eq!(rig.bot.run_player_move(1.2).buttons & strafe, buttons::MOVERIGHT);
    rig.bot.strafe_speed = 0.0;
    assert_eq!(rig.bot.run_player_move(1.3).buttons & strafe, 0);
}

#[test]
fn a_shot_from_out_of_sight_turns_the_bot_along_it_and_headshots_are_counted() {
    let mut rig = Rig::new(Config::default());
    // The shooter stands behind the bot, out of its view.
    let mut w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(-600.0, 300.0, 0.0), 0)]);
    rig.bot.me = w.entity(1).unwrap();
    rig.bot.look_task = LookTask::NextWaypoint;
    rig.extra.reps[2] = 5;
    // Travelling from the shooter to the bot, hitting its head.
    let dir = (Vector::ZERO - Vector::new(-600.0, 300.0, 0.0)).normalize();
    let shot = crate::learn::CombatEvent::Damage(crate::learn::DamageEvent {
        victim: 1,
        attacker: 2,
        inflictor: 2,
        amount: 40.0,
        hitgroup: crate::learn::ledger::HITGROUP_HEAD,
        weapon: 27,
        dir,
        pos: Vector::new(0.0, 0.0, 17.0),
        ..Default::default()
    });
    rig.replay(&mut w, &[(2.0, shot)]);
    // It faces back along the shot: toward the shooter's side, not where it stands exactly.
    assert_eq!(rig.bot.look_task, LookTask::FaceHurtOrigin);
    let to = (rig.bot.hurt_origin - rig.bot.gun_position()).normalize();
    assert!(super::look::dot(to, -dir) > 0.99, "{to:?}");
    assert_eq!(rig.bot.ledger().taken_since(0.0, Some(2)), 40.0);
    // The opponent model has the rules' weapon and the headshot.
    let key = crate::learn::opponents::OpponentKey::Player(5);
    let p = rig.learned.opponents.profile(key).unwrap();
    assert_eq!((p.headshots_on_bots, p.favourite_weapon()), (1.0, Some((27, 1.0))));
}

#[test]
fn shots_are_heard_from_the_servers_report() {
    let heard = |team: i32, at: f32, silenced: bool| {
        let mut mgr = BotManager::default();
        mgr.add(setup(false).1);
        let mut w = plane(vec![player(1, Vector::ZERO, team), player(2, Vector::new(at, 0.0, 0.0), team)]);
        w.teamplay = team != 0;
        let b = mgr.get_mut(1).unwrap();
        (b.me, b.state) = (w.entity(1).unwrap(), w.entity(1).unwrap().player.unwrap());
        // No `IN_ATTACK` in the shooter's buttons: the shot itself is what is heard.
        mgr.weapon_fired(&w, 2, Vector::new(at, 0.0, 17.0), silenced);
        mgr.get(1).unwrap().hearing.newest_new().map(|h| (h.origin.x, h.source.map(|s| s.index)))
    };
    assert_eq!(heard(0, 1000.0, false), Some((1000.0, Some(2))));
    // A silencer: only near.
    assert_eq!(heard(0, 1000.0, true), None);
    assert!(heard(0, 300.0, true).is_some());
    // Too far, or a teammate.
    assert_eq!(heard(0, 1600.0, false), None);
    assert_eq!(heard(1, 500.0, false), None);
}

#[test]
fn the_map_items_are_known_from_the_start_but_taken_ones_only_when_seen() {
    let mut rig = Rig::new(Config::default());
    // A power-up behind the bot (it looks along +x), out of sight, and a weapon in front of it.
    let w = plane(vec![player(1, Vector::ZERO, 0)]);
    let listed = |index: u32, x: f32, kind: ItemKind| MapItem {
        handle: EntRef { index, serial: 3 },
        kind,
        weapon: if kind == ItemKind::GroundWeapon { 5 } else { 0 },
        powerup: if kind == ItemKind::Powerup { 4 } else { 0 },
        origin: Vector::new(x, 0.0, 0.0),
        available: true,
        respawn_at: 0.0,
    };
    let mut list = vec![listed(6, -500.0, ItemKind::Powerup), listed(7, 400.0, ItemKind::GroundWeapon), listed(8, 300.0, ItemKind::Gun)];
    *w.map_items.borrow_mut() = Some(list.clone());
    rig.bot.me = w.entity(1).unwrap();
    rig.bot.state = rig.bot.me.player.unwrap();
    rig.bot.items.scan(&w, 1.0);
    // Places and kinds of the map's items (a dropped gun isn't one).
    let known: Vec<u32> = rig.bot.items.iter().map(|k| k.handle.index).collect();
    assert_eq!(known, vec![6, 7]);
    // Both are taken; the game says when they come back. The one taken out of sight is still
    // expected; the one taken in front of the bot is gone until the game's respawn time.
    for m in list.iter_mut().take(2) {
        (m.available, m.respawn_at) = (false, 31.0);
    }
    *w.map_items.borrow_mut() = Some(list);
    rig.bot.item_taken(&w, 3, 6);
    rig.bot.item_taken(&w, 3, 7);
    let at = |rig: &Rig, i: u32| rig.bot.items.iter().find(|k| k.handle.index == i).unwrap().available_at;
    assert_eq!((at(&rig, 6), at(&rig, 7)), (0.0, 31.0));
    // Its own pickup it knows of anyway, and the pickup it went for is done.
    rig.bot.pickup_entity = Some(EntRef { index: 6, serial: 3 });
    rig.bot.item_taken(&w, 1, 6);
    assert_eq!(at(&rig, 6), 31.0);
    assert!(rig.bot.pickup_entity.is_none());
}

#[test]
fn a_door_with_a_master_is_opened_by_the_masters_button() {
    // The door is locked by a multisource (`master` key): its own trigger (6) doesn't open it, the
    // button that fires the multisource (7) does.
    let mut door = brush(5, "func_door", Vector::new(100.0, 0.0, 0.0), Vector::new(8.0, 128.0, 128.0));
    door.targetname = "door1".into();
    let mut trigger = brush(6, "func_button", Vector::new(50.0, 80.0, 0.0), Vector::new(8.0, 8.0, 8.0));
    trigger.target = "door1".into();
    let mut button = brush(7, "func_button", Vector::new(40.0, -90.0, 0.0), Vector::new(8.0, 8.0, 8.0));
    button.target = "ms1".into();
    let w = Doors { plane: plane(vec![player(1, Vector::ZERO, 0), door.clone(), trigger, button]) };
    let masters = [MasterEntity { entity: 5, master: "ms1".into() }];
    let found = special::find_nearest_button(&w, &masters, Vector::ZERO, &door).map(|(b, _)| b.index);
    assert_eq!(found, Some(7));
    // Without the masters list (plugin API v5), the door's own trigger.
    assert_eq!(special::find_nearest_button(&w, &[], Vector::ZERO, &door).map(|(b, _)| b.index), Some(6));
}
