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

#[test]
fn walks_along_waypoints() {
    let (mut plane, mut bot, mut wp, mut weapons, presets, mut learned, config) = setup(false);
    let mut rng = XorShiftRandom::new(3);
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
