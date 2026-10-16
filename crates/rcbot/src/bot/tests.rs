//! The bot in a fake world: an open plane where the usercmd moves the bot directly.

use super::look::make_vectors;
use super::*;
use crate::XorShiftRandom;
use crate::nav::{Glass, Monsters, Trace, flags as fl};
use crate::weapons::TsWeapon;
use std::cell::RefCell;

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
        self.inventory.borrow().clone()
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
        let dt = f32::from(cmd.msec) / 1000.0;
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
fn grenade_throw_waypoints_fail_in_ts() {
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
        // HL's hand grenade id is TS's Desert Eagle (design §5).
        b.weapons.weapon_info(12, 7, 14, g.weapons, g.presets);
        b.next_waypoint_tasks(g, &w, 0, &mut rng);
        assert_eq!(b.tasks.current().map(|t| t.kind), Some(TaskKind::ThrowGrenade));
        let mut failed = false;
        b.task_throw_grenade(&mut failed);
        assert!(failed);
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
fn a_disliked_player_is_the_first_enemy_and_aimed_at_by_reputation() {
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
        // A higher priority only wins over an enemy already picked if it is also nearer (or
        // slower): seen first, the near stranger stays the enemy (design §5).
        b.visibles = [2, 3].into();
        assert_eq!(b.find_enemy(g, &w, &mut rng).map(|e| e.index), Some(2));
        assert_eq!(b.enemy_rep, Some(5));
        b.enemy = None;
        b.tasks.flush();
        b.visibles = [3, 2].into();
        let e = b.find_enemy(g, &w, &mut rng);
        assert_eq!(e.map(|e| e.index), Some(3));
        assert_eq!(b.enemy_rep, Some(1));
        // EnemyFound asked for the power-up.
        assert!(b.actions.contains(&Action::Command("usepowerup".into())));
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
    let t = *mgr.get(1).unwrap().tasks().current().unwrap();
    // Upstream's listen task has no entity, so it fails as soon as it runs (design §5).
    assert_eq!((t.kind, t.edict), (TaskKind::ListenToSound, None));
    // Too far to hear.
    let mut mgr2 = BotManager::default();
    mgr2.add(setup(false).1);
    let w2 = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(900.0, 0.0, 0.0), 0)]);
    mgr2.get_mut(1).unwrap().me = w2.entity(1).unwrap();
    mgr2.sound(&w2, 2, None, 1.0, "player/pl_step2.wav");
    assert!(mgr2.get(1).unwrap().tasks().is_empty());
    // Where the game played it counts, not where the entity is.
    mgr2.sound(&w2, 2, Some(Vector::new(200.0, 0.0, 0.0)), 1.0, "player/pl_step2.wav");
    let t = *mgr2.get(1).unwrap().tasks().current().unwrap();
    assert_eq!((t.kind, t.vector), (TaskKind::ListenToSound, Vector::new(200.0, 0.0, 0.0)));
    // Silent sounds, and kinds no bot reacts to, aren't heard.
    let mut mgr3 = BotManager::default();
    mgr3.add(setup(false).1);
    mgr3.get_mut(1).unwrap().me = w.entity(1).unwrap();
    mgr3.sound(&w, 2, None, 0.0, "player/pl_step2.wav");
    mgr3.sound(&w, 2, None, 1.0, "weapons/glock.wav");
    assert!(mgr3.get(1).unwrap().tasks().is_empty());
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
    let end = think::trace_toss(&w, &g);
    // No gravity (cvar 0): each step goes 1.5 times the last (150, 225, 337.5, 506.25), then
    // 256 times the last velocity.
    assert!((end.x - (150.0 + 225.0 + 337.5 + 506.25 + 506.25 * 256.0)).abs() < 1.0, "{end:?}");
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

/// `LookForNewTasks`' TS objectives: team 1 defends one, team 2 uses the `ts_hack` by its
/// waypoint for 9 s, each after the path to that waypoint; sometimes they roam instead.
#[test]
fn teamplay_bots_go_for_the_ts_objectives() {
    let at = Vector::new(300.0, 0.0, 0.0);
    let mut hack = player(5, at + Vector::new(20.0, 0.0, 0.0), 0);
    hack.classname = "ts_hack".into();
    hack.flags = 0;
    let objective = TsObjective { id: 0, origin: at, name: "Hack the computer".into() };
    let mut seen = std::collections::HashSet::new();
    for team in [1, 2] {
        for seed in 1..40 {
            let mut rig = Rig::new(Config::default());
            let mut plane = plane(vec![player(1, Vector::ZERO, team), hack.clone()]);
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
                [(TaskKind::FindPath, 2, -1.0, false), (TaskKind::Defend, ..)] => assert_eq!(team, 1),
                [(TaskKind::FindPath, 2, -1.0, false), (TaskKind::Use, 0, 9.0, true)] => assert_eq!(team, 2),
                [(TaskKind::FindPath, ..)] | [] => {} // roaming: the end-level waypoints, or none
                other => panic!("team {team}: {other:?}"),
            }
            seen.insert((team, kinds.len()));
        }
    }
    assert!(seen.contains(&(1, 2)) && seen.contains(&(2, 2)), "{seen:?}");
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
    let cmd = rig.bot.run_player_move(1.05, 0.05).unwrap();
    assert_ne!(cmd.buttons & buttons::JUMP, 0);
    let cmd = rig.bot.run_player_move(1.2, 0.02).unwrap();
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

/// A damage event from the host (`player_damaged`), the attacker its own inflictor.
fn damage(victim: u32, attacker: u32, amount: f32) -> crate::learn::CombatEvent {
    crate::learn::CombatEvent::Damage(crate::learn::DamageEvent {
        victim,
        attacker,
        inflictor: attacker as i32,
        amount,
        bits: 0,
        health: 0.0,
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
    assert!(rig.with(|b, g| b.weapon_score(g, 1, 300.0)).is_none());
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
    let score = rig.with(|b, g| b.weapon_score(g, 1, 300.0));
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
