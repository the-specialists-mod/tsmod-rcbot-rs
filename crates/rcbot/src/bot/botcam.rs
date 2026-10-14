//! The bot camera (`bot.h` `CBotCam`; `dll.cpp` `CBotCam::Think`, `TuneIn`, `TuneOff`, `Spawn`,
//! `Clear`, `IsWorking`): an invisible entity that follows a bot (from behind, from its eyes,
//! over its goal waypoint, or past its enemy), switching bots every 7.5-10 s and views every
//! 4-6 s. Players watch it with `rcbot botcam` (`SET_VIEW`).
//!
//! [`BotCam::think`] works out where the camera goes; the host creates the entity (`Spawn`),
//! moves it and sets players' views (design §4: no `DispatchSpawn`/`DispatchThink`, which a
//! plugin can't call; `info_target` does nothing in either).

use super::look::{change_angles, fix_angles, make_vectors, vec_to_angles};
use super::{Bot, BotManager, BotWorld};
use crate::nav::{Glass, Monsters, Waypoints};
use crate::{Random, Vector};

/// `eCamLookState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamState {
    /// `BOTCAM_NONE`.
    None,
    /// `BOTCAM_BOT`: behind the bot (over its shoulder while it fights).
    Bot,
    /// `BOTCAM_ENEMY`: past the bot's enemy, looking back.
    Enemy,
    /// `BOTCAM_WAYPOINT`: over the bot's goal waypoint.
    Waypoint,
    /// `BOTCAM_FP`: from the bot's eyes.
    FirstPerson,
}

/// Where the camera entity is and looks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Camera {
    /// `origin`.
    pub origin: Vector,
    /// `angles`.
    pub angles: Vector,
    /// `v_angle`.
    pub v_angle: Vector,
}

/// `CBotCam`.
#[derive(Clone, Debug, PartialEq)]
pub struct BotCam {
    current_bot: Option<u32>,
    state: CamState,
    /// `m_pCameraEdict`: the entity, once the host made it.
    pub camera: Option<u32>,
    next_change_bot_time: f32,
    next_change_state: f32,
    /// `m_bTriedToSpawn`.
    pub tried_to_spawn: bool,
    position_set: i32,
    /// The camera's state, which [`Self::think`] changes and the host writes to the entity.
    pub view: Camera,
    /// `BotHasEnemy` for a current bot other than the one thinking (the manager sets it).
    pub other_has_enemy: bool,
}

impl Default for BotCam {
    fn default() -> Self {
        BotCam {
            current_bot: None,
            state: CamState::None,
            camera: None,
            next_change_bot_time: 0.0,
            next_change_state: 0.0,
            tried_to_spawn: false,
            position_set: 0,
            view: Camera::default(),
            other_has_enemy: false,
        }
    }
}

impl BotCam {
    /// `Clear` (each map).
    pub fn clear(&mut self) {
        *self = BotCam::default();
    }

    /// `IsWorking`: there is a camera and `allow_botcam` is on.
    pub fn is_working(&self, enabled: bool) -> bool {
        self.camera.is_some() && enabled
    }

    /// The bot it shows.
    pub fn current_bot(&self) -> Option<u32> {
        self.current_bot
    }

    /// `SetCurrentBot`.
    pub fn set_current_bot(&mut self, index: u32, now: f32, rng: &mut dyn Random) {
        self.current_bot = Some(index);
        self.next_change_bot_time = now + rng.random_float(5.0, 7.5);
        self.next_change_state = now;
    }

    /// `TuneIn`'s checks: the message for the player when it can't.
    pub fn tune_in_error(&self, num_bots: usize) -> Option<&'static str> {
        if num_bots == 0 {
            Some("No Bots are running for bot cam")
        } else if self.camera.is_none() {
            Some("Oops, looks like camera never worked...")
        } else {
            None
        }
    }

    /// `CBotCam::Think`.
    pub fn think(&mut self, bots: &BotManager, waypoints: &Waypoints, w: &dyn BotWorld, rng: &mut dyn Random) {
        let now = w.time();
        if bots.is_empty() {
            return;
        }
        let current = self.current_bot.and_then(|i| bots.get(i));
        let not_alive = current.is_some_and(|b| !b.is_alive());
        if current.is_none() || self.next_change_bot_time < now || not_alive {
            let old = self.current_bot;
            self.next_change_bot_time = now + rng.random_float(7.5, 10.0);
            self.current_bot = None;
            if not_alive {
                let mut nearest = 0.0;
                for b in bots.iter() {
                    if Some(b.index) == old || !b.is_alive() {
                        continue;
                    }
                    let d = b.distance_from(self.view.origin, false);
                    if self.current_bot.is_none() || d < nearest {
                        self.current_bot = Some(b.index);
                        nearest = d;
                        self.state = if b.enemy.is_some() { CamState::Enemy } else { CamState::Bot };
                    }
                }
            } else {
                let possible: Vec<u32> = bots.iter().map(|b| b.index).filter(|&i| Some(i) != old).collect();
                self.current_bot =
                    if possible.is_empty() { old } else { Some(possible[rng.random_long(0, possible.len() as i32 - 1) as usize]) };
            }
        }
        let Some(bot) = self.current_bot.and_then(|i| bots.get(i)) else { return };
        if self.state == CamState::None || self.next_change_state < now {
            let mut states = vec![CamState::Bot, CamState::FirstPerson];
            if bot.goal_waypoint != -1 && bot.distance_from(waypoints.origin(bot.goal_waypoint), false) < 512.0 {
                states.push(CamState::Waypoint);
            }
            if bot.enemy.is_some() {
                states.push(CamState::Enemy);
            }
            self.state = states[rng.random_long(0, states.len() as i32 - 1) as usize];
            self.position_set = -1;
            self.next_change_state = now + rng.random_float(4.0, 6.0);
        }
        if self.state == CamState::None {
            return;
        }
        self.move_camera(bot, waypoints, w, rng);
    }

    fn move_camera(&mut self, bot: &Bot, waypoints: &Waypoints, w: &dyn BotWorld, rng: &mut dyn Random) {
        let mut look_at = bot.me.origin + bot.me.view_ofs;
        let old_origin = self.view.origin;
        let enemy = bot.resolve(w, bot.enemy);
        let cam = self.camera;
        match self.state {
            CamState::Bot => {
                if let Some(e) = &enemy {
                    let (forward, right) = make_vectors(bot.me.v_angle);
                    self.view.origin = look_at - forward * 128.0;
                    if self.position_set == -1 {
                        self.position_set = rng.random_long(0, 1);
                    }
                    self.view.origin =
                        if self.position_set == 1 { self.view.origin + right * 64.0 } else { self.view.origin + -right * 64.0 };
                    look_at = e.entity_origin();
                } else {
                    let (forward, _) = make_vectors(bot.me.angles);
                    self.view.origin = look_at - forward * 256.0;
                }
                let tr = w.trace_line(look_at, self.view.origin, Monsters::Ignore, Glass::Ignore, cam);
                if tr.fraction < 1.0 {
                    self.view.origin = tr.end_pos;
                }
            }
            CamState::Waypoint => {
                if bot.goal_waypoint != -1 {
                    self.view.origin = waypoints.origin(bot.goal_waypoint);
                    self.view.origin.z += 16.0;
                } else {
                    self.next_change_state = 0.0;
                }
            }
            CamState::Enemy => {
                if let Some(e) = &enemy {
                    let comp = e.origin + e.view_ofs - look_at;
                    let len = comp.length();
                    let comp = comp.normalize() * (len + 128.0);
                    self.view.origin = look_at + comp;
                    self.view.origin.z += 24.0;
                    if self.position_set == -1 {
                        self.position_set = rng.random_long(0, 1);
                    }
                    let n = comp.normalize();
                    let side = Vector::new(n.y, -n.x, 0.0) * e.size.length_2d();
                    self.view.origin = if self.position_set == 1 { self.view.origin + side } else { self.view.origin + -side };
                    let tr = w.trace_line(e.origin, self.view.origin, Monsters::Ignore, Glass::Ignore, cam);
                    if tr.fraction < 1.0 {
                        self.view.origin = tr.end_pos - comp.normalize() * 32.0;
                    }
                } else {
                    self.next_change_state = 0.0;
                }
            }
            CamState::FirstPerson => {
                self.view.origin = bot.me.origin + bot.me.view_ofs;
                let (forward, _) = make_vectors(bot.me.v_angle);
                look_at = self.view.origin + forward * 512.0;
                self.view.angles = bot.me.angles;
                self.view.v_angle = bot.me.v_angle;
            }
            CamState::None => {}
        }
        let mut ideal = vec_to_angles(look_at - self.view.origin);
        fix_angles(&mut ideal);
        // `BotFunc_ChangeAngles(speed, ideal, &v_angle, &angles)`: `angles` follows unless the
        // view is already within 0.1 degrees.
        let mut speed = (180.0 + ideal.x - (180.0 + self.view.angles.x)).abs() / 10.0;
        if (self.view.v_angle.x - ideal.x).abs() > 0.1 {
            change_angles(&mut speed, ideal.x, &mut self.view.v_angle.x);
            self.view.angles.x = self.view.v_angle.x;
        }
        let mut speed = (180.0 + ideal.y - (180.0 + self.view.angles.y)).abs() / 20.0;
        if (self.view.v_angle.y - ideal.y).abs() > 0.1 {
            change_angles(&mut speed, ideal.y, &mut self.view.v_angle.y);
            self.view.angles.y = self.view.v_angle.y;
        }
        self.view.origin = self.view.origin - (self.view.origin - old_origin) * 0.5;
    }
}
