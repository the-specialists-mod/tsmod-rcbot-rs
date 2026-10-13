//! Waypoint editing (`waypoint.cpp` `WaypointAddOrigin`, `WaypointAdd`, `WaypointDelete`,
//! `WaypointCreatePath`, `WaypointRemovePath`, `WaypointDrawIndex`,
//! `CWaypointLocations::DrawWaypoints`; `util.cpp` `UTIL_CanStand`) and auto-waypointing
//! (`client.cpp` `CClient::Think`, including the TS stunt-jump waypoint).
//!
//! Effects that reach a player (beams, sounds, messages) come back as [`EditorEvent`]s for the
//! host to send.

use super::{Glass, Monsters, NearestOptions, Waypoints, flags as fl};
use crate::Vector;
use crate::bot::{BotWorld, Entity, MOVETYPE_FLY, MOVETYPE_WALK, buttons, entflags};
use crate::config::BOT_WAYPOINT_TOUCH_DIST;

/// A `WaypointDrawBeam` call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    /// Start.
    pub start: Vector,
    /// End.
    pub end: Vector,
    /// Width.
    pub width: i32,
    /// Noise.
    pub noise: i32,
    /// Colour.
    pub rgb: [u8; 3],
    /// Brightness.
    pub brightness: i32,
    /// Scroll speed.
    pub speed: i32,
}

/// What an edit asks the host to show the editing player.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorEvent {
    /// `WaypointDrawBeam`.
    Beam(Beam),
    /// `UTIL_PlaySound(pEntity, sample)`.
    Sound(&'static str),
    /// `BotMessage(pEntity, 0, text)`.
    Message(String),
}

fn beam(start: Vector, end: Vector, width: i32, noise: i32, rgb: [u8; 3], brightness: i32, speed: i32) -> EditorEvent {
    EditorEvent::Beam(Beam { start, end, width, noise, rgb, brightness, speed })
}

/// `WaypointLocations.NearestWaypoint(origin, range, ignore, visible, unreachable)`.
fn nearest(wps: &Waypoints, w: &dyn BotWorld, origin: Vector, range: f32, ignore: i32, visible: bool, unreachable: bool) -> i32 {
    let opts = NearestOptions { visible, unreachable, is_bot: false, aiming_only: false };
    wps.nearest_waypoint(origin, range, usize::try_from(ignore).ok(), opts, &[], w).map_or(-1, |i| i as i32)
}

/// `WaypointAddOrigin(vOrigin, iFlags, pEntity, bDraw, bSound, bAutoSetFlagsForPlayer)`: the
/// player's crouch and ladder flags, the ammo/weapon/health/armour flags of unowned items in
/// sight within 72 units, then the waypoint (with automatic paths when `auto_path`).
#[allow(clippy::too_many_arguments)] // upstream's arguments
pub fn add_origin(
    wps: &mut Waypoints,
    w: &dyn BotWorld,
    origin: Vector,
    mut flags: u32,
    player: Option<&Entity>,
    draw_sound: bool,
    auto_flags: bool,
    auto_path: bool,
    out: &mut Vec<EditorEvent>,
) -> Option<usize> {
    if let (Some(p), true) = (player, auto_flags) {
        if p.flags & entflags::DUCKING != 0 {
            flags |= fl::CROUCH;
        }
        if p.movetype == MOVETYPE_FLY {
            flags |= fl::LADDER;
        }
    }
    for i in w.entities_in_sphere(origin, 72.0) {
        let Some(e) = w.entity(i) else { continue };
        if e.owner.is_some() {
            continue;
        }
        let tr = w.trace_line(origin, e.entity_origin(), Monsters::Ignore, Glass::DontIgnore, player.map(|p| p.index));
        if tr.fraction < 1.0 && tr.hit != Some(i) {
            continue;
        }
        let c = e.classname.as_str();
        if c.starts_with("ammo_") {
            flags |= fl::AMMO;
        } else if c.starts_with("weapon_") {
            flags |= fl::WEAPON;
        } else if c.starts_with("item_health") || c.starts_with("func_healthcharger") {
            flags |= fl::HEALTH;
        } else if c.starts_with("item_armor") || c.starts_with("item_battery") || c.starts_with("func_recharge") {
            flags |= fl::ARMOR;
        }
    }
    let Some(index) = wps.add_origin(origin, flags, auto_path.then_some(w as &dyn super::World)) else {
        if player.is_some() {
            out.push(EditorEvent::Message("Can't add waypoint, max waypoints already added".into()));
        }
        return None;
    };
    if player.is_some() && draw_sound {
        out.push(EditorEvent::Sound("weapons/xbow_hit1.wav"));
        out.extend(draw_index(wps, index));
    }
    Some(index)
}

/// `WaypointDelete`: the waypoint within 50 units (any, reachable or not); `None` when there is
/// none (a deny sound).
pub fn delete(wps: &mut Waypoints, w: &dyn BotWorld, origin: Vector, out: &mut Vec<EditorEvent>) -> i32 {
    if wps.is_empty() {
        return -1;
    }
    let index = nearest(wps, w, origin, 50.0, -1, false, true);
    if index == -1 {
        out.push(EditorEvent::Sound("weapons/wpn_denyselect.wav"));
        return -1;
    }
    wps.delete(index as usize);
    out.push(EditorEvent::Sound("weapons/mine_activate.wav"));
    index
}

/// A player's path-editing state (`CClient::m_iPathWaypointCreateIndex`,
/// `m_iPathWaypointRemoveIndex`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathEdit {
    /// The source picked by `pathwaypoint create1`.
    pub create_from: i32,
    /// The source picked by `pathwaypoint remove1`.
    pub remove_from: i32,
}

impl Default for PathEdit {
    fn default() -> Self {
        PathEdit { create_from: -1, remove_from: -1 }
    }
}

/// `WaypointCreatePath(pClient, cmd)`: 1 picks the source, 2 the destination and adds the path.
pub fn create_path(wps: &mut Waypoints, w: &dyn BotWorld, edit: &mut PathEdit, origin: Vector, cmd: i32, out: &mut Vec<EditorEvent>) {
    if cmd == 1 {
        edit.create_from = nearest(wps, w, origin, 50.0, -1, true, false);
        out.push(EditorEvent::Sound(if edit.create_from == -1 { "common/wpn_moveselect.wav" } else { "common/wpn_hudoff.wav" }));
        return;
    }
    if cmd != 2 {
        return;
    }
    let (w1, w2) = (edit.create_from, nearest(wps, w, origin, 50.0, -1, true, false));
    let error = if w1 == w2 {
        Some("Error: Waypoint is the same waypoint")
    } else if w1 == -1 || w2 == -1 {
        Some("Error: Waypoint invalid")
    } else if wps.find_path_from_to(w1 as usize, w2 as usize, -1).is_some() {
        Some("Error: Waypoint already has a path to this waypoint")
    } else {
        None
    };
    if let Some(e) = error {
        out.push(EditorEvent::Message(e.into()));
        out.push(EditorEvent::Sound("common/wpn_denyselect.wav"));
        return;
    }
    wps.add_path(w1 as usize, w2 as i16);
    out.push(EditorEvent::Sound("common/wpn_hudon.wav"));
}

/// `WaypointRemovePath(pClient, cmd)`.
pub fn remove_path(wps: &mut Waypoints, w: &dyn BotWorld, edit: &mut PathEdit, origin: Vector, cmd: i32, out: &mut Vec<EditorEvent>) {
    if cmd == 1 {
        edit.remove_from = nearest(wps, w, origin, 50.0, -1, true, false);
        out.push(EditorEvent::Sound(if edit.remove_from == -1 { "common/wpn_moveselect.wav" } else { "common/wpn_hudoff.wav" }));
        return;
    }
    if cmd != 2 {
        return;
    }
    let w2 = nearest(wps, w, origin, 50.0, -1, true, false);
    if edit.remove_from == -1 || w2 == -1 {
        out.push(EditorEvent::Sound("common/wpn_denyselect.wav"));
        return;
    }
    wps.delete_path(edit.remove_from as usize, w2 as i16);
    out.push(EditorEvent::Sound("common/wpn_hudon.wav"));
}

/// `WaypointDrawIndex`: a vertical beam coloured by the waypoint's first flag (and an arrow up a
/// ladder).
pub fn draw_index(wps: &Waypoints, index: usize) -> Vec<EditorEvent> {
    let Some(wpt) = wps.get(index).copied() else { return Vec::new() };
    let mut out = Vec::new();
    let f = wpt.flags;
    let mut colour = [0u8, 0, 255];
    let mut start = wpt.origin - Vector::new(0.0, 0.0, 34.0);
    let mut end = start + Vector::new(0.0, 0.0, 68.0);
    if f & fl::CROUCH != 0 {
        start = wpt.origin - Vector::new(0.0, 0.0, 17.0);
        end = start + Vector::new(0.0, 0.0, 34.0);
    }
    if f & fl::AIMING != 0 {
        start = wpt.origin - Vector::new(0.0, 0.0, 10.0);
        end = start + Vector::new(0.0, 0.0, 14.0);
    } else if f & fl::JUMP != 0 {
        colour = [0, 255, 255];
    } else if f & fl::LADDER != 0 {
        let up = wps.paths_from(index).filter(|&p| p >= 0).map(|p| p as usize).find(|&n| {
            let o = wps.origin(n as i32);
            wps.flags(n as i32) & fl::LADDER != 0 && o.z > wpt.origin.z
        });
        if let Some(n) = up {
            let dir = (wps.origin(n as i32) - wpt.origin).normalize();
            out.push(beam(wpt.origin, wpt.origin + dir * 128.0, 32, 2, [0, 0, 255], 200, 1));
        }
        colour = [255, 255, 0];
    } else {
        // Upstream's chain of `else if`, in its order (TS's stunt is the wall-stick bit).
        const COLOURS: [(u32, [u8; 3]); 17] = [
            (fl::DOOR, [0, 0, 255]),
            (fl::STUNT, [200, 150, 0]),
            (fl::LIFT, [200, 0, 0]),
            (fl::GREN_THROW, [10, 127, 10]),
            (fl::CROUCH, [200, 100, 100]),
            (fl::CROUCHJUMP, [150, 100, 50]),
            (fl::TELEPORT, [150, 150, 150]),
            (fl::TANK, [200, 100, 0]),
            (fl::FLY, [0, 50, 150]),
            (fl::WAIT_FOR_LIFT, [100, 150, 100]),
            (fl::ENDLEVEL, [0, 20, 150]),
            (fl::HUMAN_TOWER, [200, 100, 200]),
            (fl::OPENS_LATER, [100, 100, 100]),
            (fl::UNREACHABLE, [255, 255, 255]),
            (fl::IMPORTANT, [0, 255, 0]),
            (fl::PUSHABLE, [255, 127, 0]),
            (fl::HEALTH, [200, 200, 180]),
        ];
        if let Some((_, c)) = COLOURS.iter().find(|(b, _)| f & b != 0) {
            colour = *c;
        } else if f & fl::ARMOR != 0 {
            colour = [220, 180, 180];
        } else if f & fl::CHECK_LIFT != 0 {
            colour = [213, 234, 129];
        }
    }
    out.push(beam(start, end, 16, 2, colour, 200, 10));
    out
}

/// `CWaypointLocations::DrawWaypoints` and `CClient::Think`'s path display: the waypoints in the
/// buckets around `origin` within 256 units of height whose PVS holds the viewer, then (with
/// `paths_on`) the paths into (yellow) and out of (white) the waypoint the viewer stands at.
/// Returns that waypoint (`m_iCurrentWaypoint`) when one was found.
pub fn draw_waypoints(
    wps: &Waypoints,
    w: &dyn BotWorld,
    viewer: u32,
    origin: Vector,
    paths_on: bool,
    out: &mut Vec<EditorEvent>,
) -> Option<i32> {
    for i in wps.in_buckets_around(origin) {
        let o = wps.origin(i as i32);
        if (o.z - origin.z).abs() <= 256.0 && w.in_pvs(viewer, o) {
            out.extend(draw_index(wps, i));
        }
    }
    if !paths_on {
        return None;
    }
    let cur = nearest(wps, w, origin, BOT_WAYPOINT_TOUCH_DIST, -1, true, false);
    if cur == -1 {
        return None;
    }
    let co = wps.origin(cur);
    for (from, to) in wps.all_paths() {
        if to as i32 == cur && (co - wps.origin(from as i32)).length() < 800.0 {
            out.push(beam(
                wps.origin(from as i32) - Vector::new(0.0, 0.0, 4.0),
                co - Vector::new(0.0, 0.0, 4.0),
                10,
                2,
                [200, 200, 0],
                200,
                10,
            ));
        }
    }
    for p in wps.paths_from(cur as usize).filter(|&p| p >= 0) {
        let dest = wps.origin(i32::from(p));
        out.push(beam(co + Vector::new(0.0, 0.0, 16.0), dest + Vector::new(0.0, 0.0, 16.0), 10, 2, [250, 250, 250], 200, 10));
    }
    Some(cur)
}

/// `UTIL_CanStand`: more than 72 units between floor and ceiling; the floor.
pub fn can_stand(w: &dyn BotWorld, origin: Vector) -> (bool, Vector) {
    let down = w.trace_line(origin, origin - Vector::new(0.0, 0.0, 144.0), Monsters::Ignore, Glass::Ignore, None).end_pos;
    let up = w.trace_line(origin, origin + Vector::new(0.0, 0.0, 144.0), Monsters::Ignore, Glass::Ignore, None).end_pos;
    ((origin.z - down.z) + (up.z - origin.z) > 72.0, down)
}

/// `MAX_STORED_AUTOWAYPOINT`.
pub const MAX_STORED_AUTOWAYPOINT: usize = 5;
/// `m_fAutoWaypointCheckTime`.
pub const AUTOWAYPOINT_CHECK_TIME: f32 = 0.5;

/// `JUMP_TYPE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JumpType {
    Jump,
    Stunt,
}

/// A player's auto-waypointing state (`CClient`'s `m_vLastAutoWaypointCheckPos`, ...).
#[derive(Clone, Debug)]
pub struct AutoWaypointer {
    set_up: bool,
    last_check_time: f32,
    checks: [Option<(Vector, u32)>; MAX_STORED_AUTOWAYPOINT],
    last_place: Vector,
    can_place_jump: f32,
    last_buttons: i32,
    last_jump_waypoint: i32,
    last_ladder_waypoint: i32,
    last_movetype: i32,
    can_place_ladder: f32,
    join_ladder_waypoint: i32,
    jump_type: JumpType,
}

impl Default for AutoWaypointer {
    fn default() -> Self {
        AutoWaypointer {
            set_up: false,
            last_check_time: 0.0,
            checks: [None; MAX_STORED_AUTOWAYPOINT],
            last_place: Vector::ZERO,
            can_place_jump: 0.0,
            last_buttons: 0,
            last_jump_waypoint: -1,
            last_ladder_waypoint: -1,
            last_movetype: 0,
            can_place_ladder: 0.0,
            join_ladder_waypoint: -1,
            jump_type: JumpType::Jump,
        }
    }
}

impl AutoWaypointer {
    /// Starts over (auto-waypointing switched on again, or the player died).
    pub fn reset(&mut self) {
        self.set_up = false;
    }

    /// The auto-waypoint part of `CClient::Think` for `player`. `show` is `m_bWaypointOn` (new
    /// waypoints are drawn with a sound), `debug` `m_bDebugAutoWaypoint`.
    #[allow(clippy::too_many_arguments)] // CClient's state
    pub fn think(
        &mut self,
        wps: &mut Waypoints,
        w: &dyn BotWorld,
        player: &Entity,
        show: bool,
        debug: bool,
        auto_path: bool,
        out: &mut Vec<EditorEvent>,
    ) {
        let now = w.time();
        let alive = player.is_alive();
        if !self.set_up || !alive {
            let start = usize::from(alive);
            self.last_check_time = now + AUTOWAYPOINT_CHECK_TIME;
            if alive {
                self.checks[0] = Some((player.origin, 0));
            }
            for c in self.checks.iter_mut().skip(start) {
                *c = None;
            }
            self.last_place = player.origin;
            self.set_up = true;
            self.can_place_jump = 0.0;
            self.last_buttons = 0;
            self.last_jump_waypoint = -1;
            self.last_ladder_waypoint = -1;
            self.last_movetype = 0;
            self.can_place_ladder = 0.0;
            self.join_ladder_waypoint = -1;
            return;
        }
        let pos = player.origin;
        let on_ground = player.flags & entflags::ONGROUND != 0;
        let add = |wps: &mut Waypoints, at: Vector, flags: u32, auto_flags: bool, out: &mut Vec<EditorEvent>| {
            add_origin(wps, w, at, flags, Some(player), show, auto_flags, auto_path, out).map_or(-1, |i| i as i32)
        };

        // Jumps and TS stunts: a waypoint where the player left the ground, and a path to where
        // it landed.
        if player.waterlevel < 3 && self.can_place_jump < now {
            let stunt = self.last_buttons & buttons::ALT1 != 0;
            if (self.can_place_jump + 1.0).abs() > 0.0001 && (stunt || self.last_buttons & buttons::JUMP != 0) && !on_ground {
                let near = nearest(wps, w, pos, 80.0, -1, true, false);
                self.last_jump_waypoint = -1;
                if near == -1 {
                    if stunt {
                        self.last_jump_waypoint = add(wps, pos, fl::STUNT, true, out);
                        if self.last_jump_waypoint != -1 {
                            let i = self.last_jump_waypoint as usize;
                            let (ok, floor) = can_stand(w, wps.origin(i as i32));
                            if ok {
                                wps.set_flags(i, wps.flags(i as i32) & !fl::CROUCH);
                                wps.set_origin(i, floor + Vector::new(0.0, 0.0, 36.0));
                            }
                        }
                    } else {
                        self.last_jump_waypoint = add(wps, pos, fl::JUMP, true, out);
                    }
                } else {
                    self.last_jump_waypoint = near;
                }
                self.last_place = pos;
                self.can_place_jump = -1.0;
                self.jump_type = if stunt { JumpType::Stunt } else { JumpType::Jump };
            } else if (self.can_place_jump + 1.0).abs() <= 0.0001 && on_ground {
                if self.last_jump_waypoint != -1 {
                    let jw = self.last_jump_waypoint as usize;
                    let jump_flag = if self.jump_type == JumpType::Stunt { fl::STUNT } else { fl::JUMP };
                    let near = nearest(wps, w, pos, 80.0, -1, true, false);
                    if near == -1 {
                        let new = add(wps, pos, 0, true, out);
                        if new != -1 {
                            if wps.find_path_from_to(jw, new as usize, -1).is_none() {
                                wps.add_path(jw, new as i16);
                            }
                            wps.set_flags(jw, wps.flags(jw as i32) | jump_flag);
                            let (ok, floor) = can_stand(w, wps.origin(new));
                            if ok {
                                wps.set_flags(new as usize, wps.flags(new) & !fl::CROUCH);
                                wps.set_origin(new as usize, floor + Vector::new(0.0, 0.0, 36.0));
                            }
                        }
                    } else if near != self.last_jump_waypoint && wps.find_path_from_to(jw, near as usize, -1).is_none() {
                        wps.add_path(jw, near as i16);
                        wps.set_flags(jw, wps.flags(jw as i32) | jump_flag);
                    }
                }
                self.last_jump_waypoint = -1;
                self.can_place_jump = now + 0.5;
            }
        }

        // Ladders: a waypoint where the player got on, one where it got off, one where it
        // walked on from there.
        let mut check_distance = player.movetype != MOVETYPE_FLY && self.can_place_ladder == 0.0;
        if player.movetype == MOVETYPE_FLY && self.last_movetype != MOVETYPE_FLY {
            let near = nearest(wps, w, pos, 80.0, -1, true, false);
            self.last_ladder_waypoint = -1;
            if near == -1 {
                self.last_ladder_waypoint = add(wps, pos, fl::LADDER, true, out);
            } else {
                self.last_ladder_waypoint = near;
                wps.set_flags(near as usize, wps.flags(near) | fl::LADDER);
            }
            self.last_place = pos;
            check_distance = false;
            self.can_place_ladder = 0.0;
            self.checks = [None; MAX_STORED_AUTOWAYPOINT];
        } else if player.movetype != MOVETYPE_FLY && self.last_movetype == MOVETYPE_FLY {
            self.can_place_ladder = now + 0.2;
        }
        if self.can_place_ladder != 0.0 && self.can_place_ladder < now {
            if self.last_ladder_waypoint != -1 {
                let lw = self.last_ladder_waypoint as usize;
                let near = nearest(wps, w, pos, 80.0, -1, true, false);
                if near == -1 {
                    let new = add(wps, pos, fl::LADDER, true, out);
                    if new != -1 {
                        self.join_ladder_waypoint = new;
                        if wps.find_path_from_to(lw, new as usize, -1).is_none() {
                            wps.add_path(lw, new as i16);
                        }
                    }
                } else if near != self.last_ladder_waypoint {
                    self.join_ladder_waypoint = near;
                    if wps.find_path_from_to(lw, near as usize, -1).is_none() {
                        wps.add_path(lw, near as i16);
                    }
                }
            }
            self.last_ladder_waypoint = -1;
            check_distance = false;
            self.can_place_ladder = 0.0;
        }
        if self.join_ladder_waypoint != -1 && on_ground && player.movetype == MOVETYPE_WALK {
            let jw = self.join_ladder_waypoint as usize;
            let near = nearest(wps, w, pos, 40.0, self.join_ladder_waypoint, true, false);
            if near == -1 {
                let new = add(wps, pos, 0, true, out);
                if new != -1 && wps.find_path_from_to(jw, new as usize, -1).is_none() {
                    wps.add_path(jw, new as i16);
                }
            } else if near != self.join_ladder_waypoint && wps.find_path_from_to(jw, near as usize, -1).is_none() {
                wps.add_path(jw, near as i16);
            }
            self.join_ladder_waypoint = -1;
        }
        self.last_buttons = player.button;
        self.last_movetype = player.movetype;

        // Walking: every 200 units, and at corners (the last place the player could see that
        // it can't any more).
        if self.last_check_time >= now {
            return;
        }
        let numset = self.checks.iter().filter(|c| c.is_some()).count();
        let last = if numset == MAX_STORED_AUTOWAYPOINT {
            self.checks.rotate_left(1);
            MAX_STORED_AUTOWAYPOINT - 1
        } else {
            numset
        };
        let mut flags = 0;
        if player.flags & entflags::DUCKING != 0 {
            flags |= fl::CROUCH;
        }
        if player.movetype == MOVETYPE_FLY {
            flags |= fl::LADDER;
        }
        self.checks[last] = Some((pos, flags));
        if self.last_jump_waypoint == -1 && check_distance && (pos - self.last_place).length() > 200.0 {
            if nearest(wps, w, pos, 150.0, -1, true, false) == -1 {
                add(wps, pos, 0, true, out);
            }
            self.last_place = pos;
        }
        let mut place: Option<(Vector, u32)> = None;
        let mut n = 0;
        for i in 0..MAX_STORED_AUTOWAYPOINT {
            let Some((check, _)) = self.checks[i] else { continue };
            let tr = w.trace_line(pos, check, Monsters::Ignore, Glass::Ignore, Some(player.index));
            if debug {
                out.push(beam(check + Vector::new(0.0, 0.0, 16.0), check - Vector::new(0.0, 0.0, 16.0), 20, 1, [255, 1, 1], 150, 10));
                out.push(beam(pos, check, 20, 1, [1, 255, 1], 150, 10));
            }
            if tr.fraction >= 1.0 {
                continue;
            }
            if tr.hit.and_then(|h| w.entity(h)).is_some_and(|e| e.velocity.length() > 20.0) {
                continue;
            }
            n = i + 1;
            while n < MAX_STORED_AUTOWAYPOINT {
                if let Some((c2, f2)) = self.checks[n] {
                    let tr = w.trace_line(pos, c2, Monsters::Ignore, Glass::Ignore, Some(player.index));
                    if debug {
                        out.push(beam(pos, c2, 20, 1, [1, 1, 255], 150, 10));
                    }
                    if tr.fraction >= 1.0 && nearest(wps, w, c2, 100.0, -1, true, false) == -1 {
                        place = Some((c2, f2));
                        break;
                    }
                }
                n += 1;
            }
        }
        if let Some((at, f)) = place {
            let new = add_origin(wps, w, at, f, Some(player), show, false, auto_path, out);
            self.last_place = at;
            if let Some(new) = new
                && self.last_jump_waypoint != -1
                && can_stand(w, wps.origin(new as i32)).0
            {
                wps.set_flags(new, wps.flags(new as i32) & !fl::CROUCH);
            }
            // Upstream shifts the checks down from `n` and then clears all but the first.
            let mut pos_i = n;
            let mut j = 0;
            while pos_i < MAX_STORED_AUTOWAYPOINT {
                self.checks[j] = self.checks[pos_i];
                pos_i += 1;
                j += 1;
            }
            for c in self.checks.iter_mut().skip(1) {
                *c = None;
            }
        }
        self.last_check_time = now + AUTOWAYPOINT_CHECK_TIME;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::Entity;
    use crate::nav::{Trace, World};

    struct Open;

    impl World for Open {
        fn trace_line(&self, _: Vector, end: Vector, _: Monsters, _: Glass, _: Option<u32>) -> Trace {
            Trace::clear(end)
        }
        fn point_contents(&self, _: Vector) -> i32 {
            -1
        }
    }

    impl BotWorld for Open {
        fn time(&self) -> f32 {
            1.0
        }
        fn max_entities(&self) -> u32 {
            1
        }
        fn max_clients(&self) -> u32 {
            1
        }
        fn entity(&self, _: u32) -> Option<Entity> {
            None
        }
        fn in_pvs(&self, _: u32, _: Vector) -> bool {
            true
        }
        fn entities_in_sphere(&self, _: Vector, _: f32) -> Vec<u32> {
            Vec::new()
        }
        fn teamplay(&self) -> bool {
            false
        }
    }

    #[test]
    fn add_path_delete() {
        let (w, mut wps, mut out) = (Open, Waypoints::new(), Vec::new());
        let player = Entity { index: 1, flags: entflags::DUCKING, ..Entity::default() };
        let a = add_origin(&mut wps, &w, Vector::ZERO, 0, Some(&player), true, true, false, &mut out).unwrap();
        assert!(wps.flags(a as i32) & fl::CROUCH != 0);
        assert!(out.contains(&EditorEvent::Sound("weapons/xbow_hit1.wav")));
        let b = add_origin(&mut wps, &w, Vector::new(200.0, 0.0, 0.0), 0, None, false, false, false, &mut out).unwrap();
        let mut edit = PathEdit::default();
        create_path(&mut wps, &w, &mut edit, Vector::ZERO, 1, &mut out);
        create_path(&mut wps, &w, &mut edit, Vector::new(200.0, 0.0, 0.0), 2, &mut out);
        assert!(wps.find_path_from_to(a, b, -1).is_some());
        out.clear();
        create_path(&mut wps, &w, &mut edit, Vector::new(200.0, 0.0, 0.0), 2, &mut out);
        assert!(out.contains(&EditorEvent::Message("Error: Waypoint already has a path to this waypoint".into())));
        remove_path(&mut wps, &w, &mut edit, Vector::ZERO, 1, &mut out);
        remove_path(&mut wps, &w, &mut edit, Vector::new(200.0, 0.0, 0.0), 2, &mut out);
        assert!(wps.find_path_from_to(a, b, -1).is_none());
        assert_eq!(delete(&mut wps, &w, Vector::new(10.0, 0.0, 0.0), &mut out), a as i32);
        assert!(wps.flags(a as i32) & fl::DELETED != 0);
        // A crouch waypoint is drawn half height in its colour.
        let beams = draw_index(&wps, b);
        assert!(matches!(beams[0], EditorEvent::Beam(Beam { rgb: [0, 0, 255], .. })));
    }
}
