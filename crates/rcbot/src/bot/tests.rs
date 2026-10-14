//! The bot in a fake world: an open plane where the usercmd moves the bot directly.

use super::look::make_vectors;
use super::*;
use crate::XorShiftRandom;
use crate::nav::{Glass, Monsters, Trace, flags as fl};
use std::cell::RefCell;

struct Plane {
    time: f32,
    ents: RefCell<Vec<Entity>>,
    teamplay: bool,
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
    fn trace_line(&self, _: Vector, end: Vector, _: Monsters, _: Glass, _: Option<u32>) -> Trace {
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
        self.teamplay
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
    let plane = Plane { time: 1.0, ents: RefCell::new(ents), teamplay: false };
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
}

fn plane(ents: Vec<Entity>) -> Plane {
    Plane { time: 1.0, ents: RefCell::new(ents), teamplay: false }
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
        b.visibles = vec![2, 3];
        assert_eq!(b.find_enemy(g, &w, &mut rng).map(|e| e.index), Some(2));
        assert_eq!(b.enemy_rep, Some(5));
        b.enemy = None;
        b.tasks.flush();
        b.visibles = vec![3, 2];
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
    mgr.sound(&w, 2, "player/pl_step2.wav");
    let t = *mgr.get(1).unwrap().tasks().current().unwrap();
    // Upstream's listen task has no entity, so it fails as soon as it runs (design §5).
    assert_eq!((t.kind, t.edict), (TaskKind::ListenToSound, None));
    // Too far to hear.
    let mut mgr2 = BotManager::default();
    mgr2.add(setup(false).1);
    let w2 = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(900.0, 0.0, 0.0), 0)]);
    mgr2.get_mut(1).unwrap().me = w2.entity(1).unwrap();
    mgr2.sound(&w2, 2, "player/pl_step2.wav");
    assert!(mgr2.get(1).unwrap().tasks().is_empty());
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
