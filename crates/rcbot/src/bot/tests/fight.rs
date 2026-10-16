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
    duel_at(weapon, clip, ammo, m, Vector::new(x, 0.0, 0.0))
}

/// [`duel`] with the enemy at `at`.
fn duel_at(weapon: i32, clip: i32, ammo: i32, m: i32, at: Vector) -> (Rig, Plane) {
    let rig = Rig::new(Config::default());
    let w = plane(vec![player(1, Vector::ZERO, 0), player(2, at, 0)]);
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

/// The angle in degrees between the bot's view and the direction to `p`.
fn view_off(b: &Bot, p: Vector) -> f32 {
    let (fw, _) = make_vectors(b.v_angle);
    look::dot(fw, (p - b.gun_position()).normalize()).clamp(-1.0, 1.0).acos().to_degrees()
}

/// The angle in degrees between two points seen from the bot's gun.
fn apart(b: &Bot, p: Vector, q: Vector) -> f32 {
    let g = b.gun_position();
    look::dot((p - g).normalize(), (q - g).normalize()).clamp(-1.0, 1.0).acos().to_degrees()
}

#[test]
fn the_aim_error_shrinks_with_skill_and_time_on_target() {
    let mut rig = Rig::new(Config::default());
    let e = player(2, Vector::new(1000.0, 0.0, 0.0), 0);
    let mut rng = XorShiftRandom::new(11);
    let mean_error = |b: &mut Bot, t: f32, rng: &mut XorShiftRandom| {
        let mut sum = 0.0;
        for _ in 0..400 {
            b.get_aim_vector_time = -1.0;
            let aim = b.aim_vector(&e, t, 0.02, rng);
            sum += apart(b, aim, b.fight.aim_true.unwrap());
        }
        sum / 400.0
    };
    let b = &mut rig.bot;
    b.me = player(1, Vector::ZERO, 0);
    b.current_weapon = Some(1);
    b.profile.aim_skill = 0.0;
    let mut by_skill = Vec::new();
    for skill in [0, 50, 100] {
        b.profile.skill = skill;
        b.fight.on_target_since = 0.0;
        let fresh = mean_error(b, 0.0, &mut rng);
        let settled = mean_error(b, 1.0, &mut rng);
        // Three times the error on a new target, settled after a fraction of a second.
        assert!((fresh / settled - 3.0).abs() < 0.3, "skill {skill}: {fresh} vs {settled}");
        by_skill.push(settled);
    }
    // Within 0.3-1.5 degrees at the extremes (the mean of a uniform disc is 2/3 of its radius).
    assert!(by_skill[0] > by_skill[1] && by_skill[1] > by_skill[2], "{by_skill:?}");
    assert!((by_skill[0] - 1.0).abs() < 0.1 && (by_skill[2] - 0.2).abs() < 0.03, "{by_skill:?}");
    // A target crossing the view adds to it.
    let mut runner = e.clone();
    runner.velocity = Vector::new(0.0, 250.0, 0.0);
    b.profile.skill = 100;
    let mut sum = 0.0;
    for _ in 0..400 {
        b.get_aim_vector_time = -1.0;
        let aim = b.aim_vector(&runner, 1.0, 0.02, &mut rng);
        sum += apart(b, aim, b.fight.aim_true.unwrap());
    }
    assert!(sum / 400.0 > by_skill[2] * 1.5, "{} vs {}", sum / 400.0, by_skill[2]);
    // The lead: a frame of the runner's velocity.
    assert!((b.fight.aim_true.unwrap().y - 250.0 * 0.02).abs() < 1e-3);
}

#[test]
fn turning_is_timed_not_per_frame() {
    // A quarter turn, at 50 and at 100 frames a second: about the same view after the same time.
    let view_after = |dt: f32, secs: f32| {
        let mut rig = Rig::new(Config::default());
        let b = &mut rig.bot;
        b.me = player(1, Vector::ZERO, 0);
        b.profile.skill = 50;
        let target = Vector::new(0.0, 500.0, 17.0);
        let mut t = 1.0;
        while t < 1.0 + secs {
            t += dt;
            b.fight.aim_true = Some(target);
            b.turn_to(target, t, dt);
        }
        view_off(b, target)
    };
    let (a50, a100) = (view_after(0.02, 0.1), view_after(0.01, 0.1));
    assert!(a50 > 5.0 && (a50 - a100).abs() < 3.0, "{a50} vs {a100}");
    assert!(view_after(0.02, 0.5) < 0.5, "{}", view_after(0.02, 0.5));
    // A target crossing at 250 units/s 500 units away is tracked within a degree.
    let mut rig = Rig::new(Config::default());
    let b = &mut rig.bot;
    b.me = player(1, Vector::ZERO, 0);
    b.profile.skill = 50;
    let mut t = 1.0;
    let mut off = 0.0;
    for i in 0..100 {
        t += 0.02;
        let p = Vector::new(500.0, -250.0 + 250.0 * 0.02 * i as f32, 17.0);
        b.fight.aim_true = Some(p);
        b.turn_to(p, t, 0.02);
        off = view_off(b, p);
    }
    assert!(off < 1.0, "{off}");
}

/// The cmds of a duel where the enemy's clip of the bot goes down with each shot (as the game's
/// would), with the bot's view and velocity: (time, buttons, view off the enemy, side speed,
/// forward speed, bot origin).
fn shots(rig: &mut Rig, w: &mut Plane, seed: u64, frames: usize, pin: bool) -> Vec<(f32, i32, f32, f32, f32, Vector)> {
    let mut rng = XorShiftRandom::new(seed);
    let mut out = Vec::new();
    // The gun fires at its rate while the trigger is down, as the game's would.
    let rate = w.entity(1).unwrap().player.and_then(|p| crate::weapons::ts_data::gun_data(p.weapon)).map_or(0.1, |g| g.rate);
    let mut next_shot = 0.0;
    let enemy_at = w.entity(2).unwrap().origin;
    for _ in 0..frames {
        w.time += 0.02;
        let f = rig.with(|b, g| b.frame(g, w, &mut rng));
        if let Some(c) = f.cmd {
            w.apply(1, &c);
            let off = view_off(&rig.bot, enemy_at + Vector::new(0.0, 0.0, 10.0));
            out.push((w.time, c.buttons, off, c.side, c.forward, w.entity(1).unwrap().origin));
            if c.buttons & buttons::ATTACK != 0 && w.time >= next_shot {
                next_shot = w.time + rate - 1e-4;
                edit(w, 1, |e| e.player.as_mut().unwrap().clip -= 1);
            }
        }
        if pin {
            edit(w, 1, |e| e.origin = Vector::ZERO);
        }
        edit(w, 2, |e| e.origin = enemy_at);
    }
    out
}

#[test]
fn no_shot_before_the_reaction_time_or_off_target() {
    // The enemy 40 degrees aside: it turns first, and fires only on target.
    let (mut rig, mut w) = duel_at(1, 999, 0, mode::AUTO, Vector::new(300.0, 250.0, 0.0));
    rig.bot.profile.skill = 50;
    let c = shots(&mut rig, &mut w, 2, 200, true);
    let first = c.iter().find(|s| s.1 & buttons::ATTACK != 0).expect("fires");
    assert!(c.iter().filter(|s| s.1 & buttons::ATTACK != 0).all(|s| s.2 < 5.0));
    // The reaction: 0.25 s at skill 50 after it was seen.
    let seen = rig.bot.first_see_enemy_time;
    assert!(first.0 >= seen + 0.25 - 1e-3, "fired at {} seen at {seen}", first.0);
}

#[test]
fn the_barrett_waits_for_its_spread_and_stands_still() {
    let (mut rig, mut w) = duel(18, 999, 0, mode::SEMI, 1000.0);
    rig.bot.profile.skill = 50;
    let c = shots(&mut rig, &mut w, 3, 400, false);
    let fired: Vec<_> = c.iter().filter(|s| s.1 & buttons::ATTACK != 0).collect();
    assert!(fired.len() >= 3, "{}", fired.len());
    // One shot fills the penalty (0.1); at 1000 units and skill 50 it must fall to 0.027 first.
    assert!(fired.windows(2).all(|p| p[1].0 - p[0].0 >= 0.7), "{:?}", fired.iter().map(|s| s.0).collect::<Vec<_>>());
    assert!(c.iter().all(|s| s.3 == 0.0 && s.4 == 0.0));
}

/// The longest run of commands holding `+attack`.
fn longest_hold(c: &[(f32, i32, f32, f32, f32, Vector)]) -> usize {
    let (mut best, mut run) = (0, 0);
    for s in c {
        run = if s.1 & buttons::ATTACK != 0 { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

#[test]
fn automatic_fire_comes_in_bursts_at_range() {
    // The AUG kicks 0.01 a shot: far away it lets go between short bursts; near, it holds.
    let held = |x: f32| {
        let (mut rig, mut w) = duel(15, 999, 0, mode::AUTO, x);
        rig.bot.profile.skill = 50;
        let c = shots(&mut rig, &mut w, 4, 300, true);
        let first = c.iter().position(|s| s.1 & buttons::ATTACK != 0).expect("fires");
        let rest = &c[first..];
        (rest.iter().filter(|s| s.1 & buttons::ATTACK != 0).count() as f32 / rest.len() as f32, longest_hold(rest))
    };
    let ((far, far_run), (near, near_run)) = (held(1500.0), held(200.0));
    assert!(far < 0.75 && near > 0.9 && far_run < near_run, "far {far} ({far_run}) near {near} ({near_run})");
}

#[test]
fn gunfights_strafe_and_keep_the_guns_range() {
    // A pistol in its range: side to side, both ways, at full speed.
    let (mut rig, mut w) = duel(1, 999, 0, mode::AUTO, 400.0);
    let c = shots(&mut rig, &mut w, 5, 150, true);
    let sides: Vec<f32> = c.iter().skip(25).map(|s| s.3).collect();
    assert!(sides.iter().any(|&s| s > 200.0) && sides.iter().any(|&s| s < -200.0), "{sides:?}");
    assert!(sides.iter().filter(|&&s| s.abs() > 200.0).count() * 10 > sides.len() * 7);
    // A shotgun closes in; a rifle backs off.
    let (mut rig, mut w) = duel(26, 999, 32, mode::PUMP, 600.0);
    let c = shots(&mut rig, &mut w, 5, 100, false);
    let to = |c: &[(f32, i32, f32, f32, f32, Vector)], x: f32| (c.last().unwrap().5 - Vector::new(x, 0.0, 0.0)).length();
    assert!(to(&c, 600.0) < 400.0, "{}", to(&c, 600.0));
    let (mut rig, mut w) = duel(5, 999, 0, mode::AUTO, 150.0);
    let c = shots(&mut rig, &mut w, 5, 100, false);
    assert!(to(&c, 150.0) > 280.0, "{}", to(&c, 150.0));
}

#[test]
fn shots_build_the_recoil_penalty_and_it_decays() {
    let mut rig = Rig::new(Config::default());
    let b = &mut rig.bot;
    b.current_weapon = Some(5);
    b.state = PlayerState { weapon: 5, clip: 30, time_scale: 1.0, ..PlayerState::default() };
    b.track_shots(1.0);
    for i in 1..=3 {
        b.state.clip -= 1;
        b.track_shots(1.0 + 0.07 * i as f32);
    }
    // The first shot after a pause: (1 - 0.5) * 0.004 * 2; then (1 - 0.07) * 0.004 * 2 each.
    // The decay (0.007 between shots) runs first and stops at 0.
    let per_shot = 0.93 * 0.004 * 2.0;
    let want = ((0.004f32 - 0.007).max(0.0) + per_shot - 0.007).max(0.0) + per_shot;
    assert!((b.fight.penalty - want).abs() < 1e-3, "{} vs {want}", b.fight.penalty);
    b.track_shots(3.0);
    assert_eq!(b.fight.penalty, 0.0);
}

/// Frames of a duel with everything the bot did: (time, cmd, actions).
fn frames(
    rig: &mut Rig,
    w: &mut Plane,
    seed: u64,
    n: usize,
    mut each: impl FnMut(&Plane, &Frame),
) -> Vec<(f32, Option<BotCmd>, Vec<Action>)> {
    let mut rng = XorShiftRandom::new(seed);
    let mut out = Vec::new();
    run(rig, w, &mut rng, n, |w, f| {
        each(w, f);
        out.push((w.time, f.cmd, f.actions.clone()));
    });
    out
}

#[test]
fn dodge_dives_are_single_presses_rolled_out_of() {
    // Strafing under an enemy's aim, light enough: a dive now and then, `+alt1` in one usercmd
    // and `+duck` held after it; never two dives within 3 s.
    let dives = |free: i32, yaw: f32| {
        let (mut rig, mut w) = duel(5, 999, 0, mode::AUTO, 400.0);
        rig.bot.profile.skill = 100;
        edit(&w, 1, |e| e.player.as_mut().unwrap().free_slots = free);
        edit(&w, 2, |e| e.v_angle = Vector::new(0.0, yaw, 0.0));
        let f = frames(&mut rig, &mut w, 9, 500, |w, _| {
            edit(w, 1, |e| e.origin = Vector::ZERO);
        });
        let cmds: Vec<(f32, i32)> = f.iter().filter_map(|(t, c, _)| c.map(|c| (*t, c.buttons))).collect();
        assert!(cmds.windows(2).all(|p| p[0].1 & p[1].1 & buttons::ALT1 == 0), "held");
        let at: Vec<usize> = (0..cmds.len()).filter(|&i| cmds[i].1 & buttons::ALT1 != 0).collect();
        for &i in &at {
            assert!(cmds[i].1 & buttons::DUCK == 0 && cmds[i + 1].1 & buttons::DUCK != 0, "rolls out");
        }
        assert!(at.windows(2).all(|p| cmds[p[1]].0 - cmds[p[0]].0 >= 3.0));
        at.len()
    };
    assert!(dives(40, 180.0) >= 1);
    // Too heavy (71 of 81 slots used), or nobody aiming: no dive, and no stunt key at all.
    assert_eq!(dives(10, 180.0), 0);
    assert_eq!(dives(40, 0.0), 0);
}

#[test]
fn the_enemy_is_kept_until_another_is_clearly_worse() {
    // Two alike enemies: the bot stays on one (upstream switched every second).
    let mut rig = Rig::new(Config::default());
    let mut w =
        plane(vec![player(1, Vector::ZERO, 0), player(2, Vector::new(400.0, 60.0, 0.0), 0), player(3, Vector::new(400.0, -60.0, 0.0), 0)]);
    let mut rng = XorShiftRandom::new(4);
    run(&mut rig, &mut w, &mut rng, 150, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
    let first = rig.bot.enemy().expect("an enemy");
    let (mut switches, mut last) = (0, first);
    for _ in 0..250 {
        run(&mut rig, &mut w, &mut rng, 1, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
        if let Some(e) = rig.bot.enemy()
            && e != last
        {
            switches += 1;
            last = e;
        }
    }
    assert_eq!(switches, 0);
    // Hurt by the other one: it turns on it, out of view or not.
    let other = if first == 2 { 3 } else { 2 };
    let from = w.entity(other).unwrap().origin;
    rig.bot.hurt(Some(other), from, w.time);
    run(&mut rig, &mut w, &mut rng, 30, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
    assert_eq!(rig.bot.enemy(), Some(other));
    // Someone behind it, out of the view cone, hurts it: it takes it on.
    edit(&w, 3, |e| e.health = 0.0);
    edit(&w, 2, |e| e.health = 0.0);
    w.ents.borrow_mut().push(player(4, Vector::new(-300.0, 0.0, 0.0), 0));
    run(&mut rig, &mut w, &mut rng, 10, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
    rig.bot.hurt(Some(4), Vector::new(-300.0, 0.0, 0.0), w.time);
    run(&mut rig, &mut w, &mut rng, 30, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
    assert_eq!(rig.bot.enemy(), Some(4));
}

#[test]
fn losing_the_trade_it_breaks_off_to_cover() {
    let (mut rig, mut w) = duel(1, 999, 0, mode::AUTO, 400.0);
    edit(&w, 1, |e| e.health = 30.0);
    frames(&mut rig, &mut w, 2, 40, |_, _| {});
    assert!(rig.bot.tasks().has_schedule(ScheduleDesc::RunForCover), "{:?}", rig.bot.tasks().iter().map(|t| t.kind).collect::<Vec<_>>());
    // Healthier than the enemy: fights on.
    let (mut rig, mut w) = duel(1, 999, 0, mode::AUTO, 400.0);
    edit(&w, 1, |e| e.health = 30.0);
    edit(&w, 2, |e| e.health = 20.0);
    frames(&mut rig, &mut w, 2, 40, |_, _| {});
    assert!(!rig.bot.tasks().has_schedule(ScheduleDesc::RunForCover));
}

#[test]
fn the_weapon_suits_the_range() {
    // A Glock in hand and an M4 owned: at 1500 units the M4 (5.56's 1500-unit full damage and
    // less spread).
    let switched = |x: f32, hand: (i32, i32, i32), owned_list: &[(i32, i32, i32)]| {
        let (mut rig, mut w) = duel(hand.0, hand.1, hand.2, mode::AUTO, x);
        *w.inventory.borrow_mut() = owned(owned_list);
        let f = frames(&mut rig, &mut w, 3, 100, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
        f.iter().flat_map(|(_, _, a)| a.iter()).find_map(|a| if let Action::SelectWeapon(id) = a { Some(*id) } else { None })
    };
    assert_eq!(switched(1500.0, (1, 17, 34), &[(1, 17, 34), (5, 30, 60)]), Some(5));
    // Close in, both do: no switch for a few percent.
    assert_eq!(switched(150.0, (5, 30, 60), &[(1, 17, 34), (5, 30, 60)]), None);
    // The M4 empty with an enemy near: the pistol rather than a 2.8 s reload; far off, reload.
    assert_eq!(switched(300.0, (5, 0, 60), &[(1, 17, 34), (5, 0, 60)]), Some(1));
    assert_eq!(switched(900.0, (5, 0, 60), &[(1, 17, 34), (5, 0, 60)]), None);
}

#[test]
fn out_of_sight_it_holds_the_angle_where_the_enemy_went() {
    let (mut rig, mut w) = duel(1, 999, 0, mode::AUTO, 400.0);
    frames(&mut rig, &mut w, 6, 40, |w, _| {
        edit(w, 1, |e| e.origin = Vector::ZERO);
        edit(w, 2, |e| e.velocity = Vector::new(0.0, 200.0, 0.0));
    });
    let head = rig.bot.fight.sight_head;
    // A wall comes between them and the enemy walks on behind it.
    w.wall.set(Some(200.0));
    edit(&w, 2, |e| e.origin = Vector::new(400.0, 300.0, 0.0));
    let f = frames(&mut rig, &mut w, 6, 40, |w, _| edit(w, 2, |e| e.origin = Vector::new(400.0, 300.0, 0.0)));
    let cmds: Vec<BotCmd> = f.iter().filter_map(|(_, c, _)| *c).collect();
    // (The first usercmds carry the last think's trigger.)
    assert!(cmds.iter().skip(3).all(|c| c.buttons & buttons::ATTACK == 0));
    assert!(cmds.iter().skip(5).all(|c| c.forward == 0.0 && c.side == 0.0), "holds still");
    let b = &rig.bot;
    assert!(view_off(b, head) < 3.0, "{} off the corner", view_off(b, head));
    assert!(view_off(b, Vector::new(400.0, 300.0, 17.0)) > 20.0, "no wallhack");
}

#[test]
fn a_grenade_goes_behind_the_cover_the_enemy_took() {
    let (mut rig, mut w) = duel(5, 30, 60, mode::AUTO, 500.0);
    rig.bot.profile.skill = 100;
    *w.inventory.borrow_mut() = owned(&[(5, 30, 60), (24, 1, 0)]);
    frames(&mut rig, &mut w, 8, 40, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
    w.wall.set(Some(300.0));
    let (mut selected, mut pressed, mut released) = (false, false, false);
    frames(&mut rig, &mut w, 8, 250, |w, f| {
        edit(w, 1, |e| e.origin = Vector::ZERO);
        if f.actions.contains(&Action::SelectWeapon(24)) {
            selected = true;
            // The game takes the grenade out.
            arm(w, 1, 24, 1, 0);
        }
        let attack = f.cmd.is_some_and(|c| c.buttons & buttons::ATTACK != 0);
        if selected && attack {
            pressed = true;
        }
        if pressed && !attack {
            released = true;
        }
    });
    assert!(selected && pressed && released, "selected {selected} pressed {pressed} released {released}");
}

#[test]
fn throws_land_where_they_are_aimed() {
    let src = Vector::new(0.0, 0.0, 17.0);
    for target in [Vector::new(300.0, 0.0, 0.0), Vector::new(600.0, 100.0, -50.0), Vector::new(700.0, 0.0, 100.0)] {
        let (p, miss) = super::super::fire::throw_pitch(src, target, 600.0).expect("a throw");
        // Fly it as TS throws it.
        let thrown = p * if p >= 0.0 { 10.0 / 9.0 } else { 8.0 / 9.0 } - 10.0;
        let speed = ((90.0 - thrown) * 7.0).min(750.0);
        let (s, c) = (-thrown).to_radians().sin_cos();
        let t = (target - src).length_2d() / (speed * c);
        let z = src.z + speed * s * t - 300.0 * t * t;
        assert!((z - target.z).abs() < 32.0 && miss < 32.0, "{target:?}: pitch {p} lands at {z}");
    }
    // Out of reach.
    assert!(super::super::fire::throw_pitch(src, Vector::new(5000.0, 0.0, 0.0), 600.0).is_none());
}

#[test]
fn it_runs_from_any_grenade_landing_near_it() {
    let flees = |at: Vector, velocity: Vector| {
        let mut rig = Rig::new(Config::default());
        let mut nade = player(5, at, 0);
        (nade.classname, nade.flags, nade.player, nade.velocity) = ("grenade".into(), 0, None, velocity);
        let mut w = plane(vec![player(1, Vector::ZERO, 0), nade]);
        frames(&mut rig, &mut w, 3, 10, |_, _| {});
        rig.bot.tasks().has_schedule(ScheduleDesc::RunForCover)
    };
    // Behind the bot (out of view), rolling to it.
    assert!(flees(Vector::new(-250.0, 0.0, 0.0), Vector::new(50.0, 0.0, 0.0)));
    // Rolling away, landing over 330 units off.
    assert!(!flees(Vector::new(-250.0, 0.0, 0.0), Vector::new(-100.0, 0.0, 0.0)));
}

#[test]
fn power_ups_are_used_when_they_help() {
    let used = |stored: i32, x: f32, weapon: i32| {
        let (mut rig, mut w) = duel(weapon, 17, 34, mode::AUTO, x);
        edit(&w, 1, |e| e.player.as_mut().unwrap().stored_powerup = stored);
        let f = frames(&mut rig, &mut w, 3, 75, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
        f.iter().flat_map(|(_, _, a)| a.iter()).filter(|a| **a == Action::Command("usepowerup".into())).count()
    };
    use crate::weapons::ts_data::powerup;
    // Slow motion in a mid-range fight, once per 2 s; not at 1500 units.
    assert_eq!(used(powerup::SLOWMO, 500.0, 1), 1);
    assert_eq!(used(powerup::SLOWMO, 1500.0, 1), 0);
    // Infinite clip with a gun out; kung fu close in only.
    assert_eq!(used(powerup::INFAMMO, 1500.0, 1), 1);
    assert_eq!(used(powerup::KUNGFU, 500.0, 1), 0);
    // A kung fu power-up running: hand to hand within 300 units.
    let (mut rig, mut w) = duel(1, 17, 34, mode::AUTO, 200.0);
    edit(&w, 1, |e| e.player.as_mut().unwrap().active_powerup = powerup::KUNGFU);
    let f = frames(&mut rig, &mut w, 3, 50, |w, _| edit(w, 1, |e| e.origin = Vector::ZERO));
    assert!(f.iter().any(|(_, _, a)| a.contains(&Action::SelectWeapon(0))));
}
