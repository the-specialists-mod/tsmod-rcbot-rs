//! The fight in the fake world: the trigger, the aim, movement in a gunfight, weapon choice,
//! reloads, blades, grenades and power-ups (design §4).

use super::*;
use crate::weapons::ts_data::mode;

/// Changes player `index` in `w`.
fn edit(w: &Plane, index: u32, f: impl FnOnce(&mut Entity)) {
    let mut ents = w.ents.borrow_mut();
    f(ents.iter_mut().find(|e| e.index == index).unwrap());
}

/// Sets the fire mode of player `index`'s weapon.
fn fire_mode(w: &Plane, index: u32, m: i32) {
    edit(w, index, |e| e.player.as_mut().unwrap().fire_mode = m);
}

/// A bot at the origin and an enemy at `x`, the bot holding `weapon` (clip, reserve) in `mode`.
fn duel(weapon: i32, clip: i32, ammo: i32, m: i32, x: f32) -> (Rig, Plane) {
    let rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(x, 0.0, 0.0), 0)]);
    arm(&w, 1, weapon, clip, ammo);
    fire_mode(&w, 1, m);
    (rig, w)
}

/// The usercmds' times and buttons over `frames` frames, the bot and the enemy kept in place.
fn cmds(rig: &mut Rig, w: &mut Plane, seed: u64, frames: usize) -> Vec<(f32, i32)> {
    let mut rng = XorShiftRandom::new(seed);
    let mut out = Vec::new();
    let enemy_at = w.entity(2).map(|e| e.origin);
    run(rig, w, &mut rng, frames, |w, f| {
        if let Some(c) = f.cmd {
            out.push((w.time, c.buttons));
        }
        edit(w, 1, |e| e.origin = Vector::ZERO);
        if let Some(o) = enemy_at {
            edit(w, 2, |e| e.origin = o);
        }
    });
    out
}

#[test]
fn semi_automatic_guns_are_tapped_at_their_rate() {
    // A Desert Eagle (semi only, 0.1 s): a press, a release, never faster than it fires.
    let (mut rig, mut w) = duel(12, 7, 14, mode::SEMI, 300.0);
    let c = cmds(&mut rig, &mut w, 5, 300);
    let presses: Vec<f32> = c.iter().filter(|(_, b)| b & buttons::ATTACK != 0).map(|(t, _)| *t).collect();
    assert!(presses.len() >= 10, "{} presses", presses.len());
    assert!(c.windows(2).all(|p| p[0].1 & p[1].1 & buttons::ATTACK == 0), "held");
    assert!(presses.windows(2).all(|p| p[1] - p[0] >= 0.1 - 1e-4), "{presses:?}");
    // The Barrett: 0.7 s.
    let (mut rig, mut w) = duel(18, 5, 10, mode::SEMI, 300.0);
    let c = cmds(&mut rig, &mut w, 5, 300);
    let presses: Vec<f32> = c.iter().filter(|(_, b)| b & buttons::ATTACK != 0).map(|(t, _)| *t).collect();
    assert!(presses.windows(2).all(|p| p[1] - p[0] >= 0.7 - 1e-4), "{presses:?}");
    // The same Glock in automatic mode is held.
    let (mut rig, mut w) = duel(1, 17, 34, mode::AUTO, 300.0);
    let c = cmds(&mut rig, &mut w, 5, 300);
    assert!(c.windows(2).any(|p| p[0].1 & p[1].1 & buttons::ATTACK != 0), "auto is held");
}

#[test]
fn akimbo_semi_alternates_the_guns() {
    let (mut rig, mut w) = duel(8, 30, 60, mode::AKIMBO_SEMI, 300.0);
    let c = cmds(&mut rig, &mut w, 7, 300);
    let keys: Vec<i32> = c.iter().map(|(_, b)| b & (buttons::ATTACK | buttons::ATTACK2)).filter(|&k| k != 0).collect();
    assert!(keys.len() >= 6, "{keys:?}");
    assert!(keys.windows(2).all(|k| k[0] != k[1]), "{keys:?}");
}

#[test]
fn the_aim_point_depends_on_the_gun_and_what_is_in_sight() {
    let mut rig = Rig::new(Config::default());
    let e = player(2, Vector::new(300.0, 0.0, 0.0), 0);
    let b = &mut rig.bot;
    b.me = player(1, Vector::ZERO, 0);
    b.profile.skill = 50;
    b.current_weapon = Some(1);
    let chest = e.origin + Vector::new(0.0, 0.0, 17.0 * 0.6);
    let head = e.origin + Vector::new(0.0, 0.0, 15.0);
    assert_eq!(b.aim_point(&e), chest);
    // A skilled bot with a Desert Eagle goes for the head; with a shotgun, the middle.
    b.profile.skill = 80;
    b.current_weapon = Some(12);
    assert_eq!(b.aim_point(&e), head);
    b.current_weapon = Some(4);
    assert_eq!(b.aim_point(&e), e.entity_origin());
    // Only the head shows: the head, whatever the gun.
    b.profile.skill = 10;
    b.current_weapon = Some(1);
    b.set(cond::SEE_ENEMY_HEAD);
    b.clear(cond::SEE_ENEMY_BODY);
    assert_eq!(b.aim_point(&e), head);
}

/// Reload presses (new `+reload`s) over `frames` frames.
fn reload_presses(rig: &mut Rig, w: &mut Plane, frames: usize) -> usize {
    let c = cmds(rig, w, 3, frames);
    let mut last = 0;
    c.iter()
        .filter(|(_, b)| {
            let new = b & !last & buttons::RELOAD != 0;
            last = *b;
            new
        })
        .count()
}

#[test]
fn a_half_empty_clip_is_topped_up_when_no_enemy_is_near() {
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
    cmds(&mut rig, &mut w, 3, 5);
    arm(&w, 1, 1, 5, 34);
    assert_eq!(reload_presses(&mut rig, &mut w, 60), 1);
    // More than half a clip, or no reserve: no.
    for (clip, ammo) in [(10, 34), (5, 0)] {
        let mut rig = Rig::new(Config::default());
        let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
        cmds(&mut rig, &mut w, 3, 5);
        arm(&w, 1, 1, clip, ammo);
        assert_eq!(reload_presses(&mut rig, &mut w, 60), 0, "clip {clip} ammo {ammo}");
    }
    // With an infinite clip running: no.
    let mut rig = Rig::new(Config::default());
    let mut w = plane(vec![player(1, Vector::ZERO, 0)]);
    cmds(&mut rig, &mut w, 3, 5);
    arm(&w, 1, 1, 5, 34);
    edit(&w, 1, |e| e.player.as_mut().unwrap().active_powerup = crate::weapons::ts_data::powerup::INFAMMO);
    assert_eq!(reload_presses(&mut rig, &mut w, 60), 0);
    // An enemy in sight: fight on.
    let (mut rig, mut w) = duel(1, 17, 34, mode::AUTO, 300.0);
    cmds(&mut rig, &mut w, 3, 5);
    arm(&w, 1, 1, 5, 34);
    assert_eq!(reload_presses(&mut rig, &mut w, 60), 0);
}

#[test]
fn blades_slash_and_only_a_spare_knife_is_thrown() {
    // Within reach: `+attack`, never `+attack2` (which throws the knife away in TS 3.0).
    for weapon in [25, 34] {
        let (mut rig, mut w) = duel(weapon, 1, 0, mode::AUTO, 50.0);
        let c = cmds(&mut rig, &mut w, 4, 150);
        assert!(c.iter().any(|(_, b)| b & buttons::ATTACK != 0), "weapon {weapon}");
        assert!(!c.iter().any(|(_, b)| b & buttons::ATTACK2 != 0), "weapon {weapon}");
    }
    // Out of reach: a spare knife is thrown, the last one kept.
    let (mut rig, mut w) = duel(25, 3, 0, mode::AUTO, 300.0);
    assert!(cmds(&mut rig, &mut w, 4, 150).iter().any(|(_, b)| b & buttons::ATTACK2 != 0));
    let (mut rig, mut w) = duel(25, 1, 0, mode::AUTO, 300.0);
    assert!(!cmds(&mut rig, &mut w, 4, 150).iter().any(|(_, b)| b & buttons::ATTACK2 != 0));
}
