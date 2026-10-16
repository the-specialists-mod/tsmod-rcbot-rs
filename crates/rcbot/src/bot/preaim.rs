//! Looking where an enemy may appear (port addition, design §4).
//!
//! Upstream's moving bot looked at the waypoint it walked to (`BOT_LOOK_TASK_NEXT_WAYPOINT`) or
//! at a random direction (`LOOK_AROUND`). An experienced player checks the places that are about
//! to come into view: [`LookTask::PreAim`] looks at the waypoints the next waypoint of the path
//! sees and the current one doesn't (the visibility table), most dangerous and longest unchecked
//! first; standing (defending, searching), at those the current waypoint sees. A sound heard takes
//! the view for a moment ([`Bot::watch`]).

use super::{Bot, BotGlobals, LookTask};
use crate::nav::flags as fl;
use crate::{Random, Vector};

/// The nearest and farthest waypoints worth pre-aiming at.
const PRE_AIM_RANGE: (f32, f32) = (150.0, 1500.0);
/// Seconds a checked waypoint counts as checked.
const CHECKED_FOR: f32 = 4.0;
/// Pre-aim at chest height above a waypoint (waypoints are at a standing player's origin).
const CHEST: Vector = Vector { x: 0.0, y: 0.0, z: 8.0 };

/// Where the bot pre-aims, and what it checked lately.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreAim {
    /// What it looks at.
    pub point: Vector,
    /// Until then (then it picks the next).
    pub until: f32,
    /// A point to look at first, until `watch_until` (a sound heard).
    pub watch: Vector,
    /// See `watch`.
    pub watch_until: f32,
    /// Waypoints looked at, and when.
    checked: Vec<(usize, f32)>,
}

impl Bot {
    /// Looks at `at` for `secs` (a sound), then goes back to pre-aiming.
    pub(crate) fn watch(&mut self, at: Vector, now: f32, secs: f32) {
        self.pre_aim.watch = at;
        self.pre_aim.watch_until = now + secs;
        if !matches!(self.look_task, LookTask::FaceEnemy | LookTask::FaceTaskEdict | LookTask::FaceTaskVector) {
            self.look_task = LookTask::PreAim;
        }
    }

    /// The waypoint to pre-aim at from the bot's path: visible from `next` (where it will be) but
    /// not from `cur` (where it is), between [`PRE_AIM_RANGE`], not ahead of the bot along its
    /// path; with `cur == next` (standing), the ones `cur` sees. Most dangerous and longest
    /// unchecked first.
    pub(crate) fn pre_aim_target(&self, g: &BotGlobals<'_>, cur: usize, next: usize, now: f32, rng: &mut dyn Random) -> Option<usize> {
        let wp = &*g.waypoints;
        let (cur_o, next_o) = (wp.origin(cur as i32), wp.origin(next as i32));
        let heading = (next_o - cur_o).normalize();
        let threats: Vec<Vector> = self.remembered.iter().map(|p| p.origin).collect();
        let mut best: Option<(usize, f32)> = None;
        for (i, w) in wp.as_slice().iter().enumerate() {
            if i == cur || i == next || w.flags & (fl::DELETED | fl::AIMING) != 0 || !wp.visibility.get(next, i) {
                continue;
            }
            if cur != next && wp.visibility.get(cur, i) {
                continue;
            }
            let d = (w.origin - next_o).length();
            if d < PRE_AIM_RANGE.0 || d > PRE_AIM_RANGE.1 {
                continue;
            }
            // Where the path goes the bot looks anyway.
            if cur != next && super::look::dot((w.origin - next_o).normalize(), heading) > 0.9 {
                continue;
            }
            let mut score = 1.0 + 3.0 * g.danger.map_or(0.0, |dg| dg.danger(i));
            if threats.iter().any(|t| (*t - w.origin).length() < 400.0) {
                score += 2.0;
            }
            if self.pre_aim.checked.iter().any(|&(c, t)| c == i && now - t < CHECKED_FOR) {
                score *= 0.2;
            }
            score += rng.random_float(0.0, 0.5);
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((i, score));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Keeps [`PreAim::point`] up to date while the look task is [`LookTask::PreAim`]: a sound
    /// being watched, else the next place to check for [0.6, 1.2) s, else the move target.
    pub(crate) fn update_pre_aim(&mut self, g: &BotGlobals<'_>, now: f32, rng: &mut dyn Random) {
        if self.look_task != LookTask::PreAim {
            return;
        }
        if self.pre_aim.watch_until > now {
            self.pre_aim.point = self.pre_aim.watch;
            return;
        }
        if self.pre_aim.until > now {
            return;
        }
        let Ok(cur) = usize::try_from(self.current_waypoint) else {
            self.pre_aim.point = self.move_to;
            self.pre_aim.until = now + 0.3;
            return;
        };
        let next = usize::try_from(self.next_waypoint()).unwrap_or(cur);
        match self.pre_aim_target(g, cur, next, now, rng) {
            Some(i) => {
                self.pre_aim.point = g.waypoints.origin(i as i32) + CHEST;
                self.pre_aim.until = now + rng.random_float(0.6, 1.2);
                self.pre_aim.checked.retain(|&(_, t)| now - t < CHECKED_FOR);
                self.pre_aim.checked.push((i, now));
            }
            None => {
                self.pre_aim.point = if self.move_to_valid { self.move_to } else { self.pre_aim.point };
                self.pre_aim.until = now + 0.3;
            }
        }
    }

    /// `WorkViewAngles` for [`LookTask::PreAim`].
    pub(crate) fn pre_aim_look(&mut self) {
        let p = if self.pre_aim.point == Vector::ZERO { self.move_to } else { self.pre_aim.point };
        self.set_view_angles(p);
    }
}
