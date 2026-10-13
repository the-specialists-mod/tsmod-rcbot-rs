//! The editing players' per-frame part of `CClient::Think`: the waypoint display (once a second)
//! and auto-waypointing, sending what `rcbot::nav::editor` asks for.

use crate::State;
use crate::api::{Api, Engine};
use crate::world::HostWorld;
use rcbot::bot::BotWorld;
use rcbot::nav::editor::{self, AutoWaypointer, EditorEvent, PathEdit};
use std::collections::HashMap;

/// Per player: path editing, the auto-waypointer, `m_fWaypointDisplayTime`.
#[derive(Default)]
pub struct Editor {
    pub paths: HashMap<u32, PathEdit>,
    auto: HashMap<u32, AutoWaypointer>,
    display_time: HashMap<u32, f32>,
}

/// Sends editor events to `player`.
pub fn send(s: &State, e: Engine, player: u32, events: Vec<EditorEvent>) {
    for ev in events {
        match ev {
            EditorEvent::Beam(b) => e.beam(player as i32, b.start, b.end, s.sprite, b.width, b.noise, b.rgb, b.brightness, b.speed),
            EditorEvent::Sound(sample) => e.client_command_sound(player as i32, sample),
            EditorEvent::Message(m) => e.client_print(player as i32, &format!("{}{m}\n", rcbot::config::BOT_DBG_MSG_TAG)),
        }
    }
}

pub fn frame(s: &mut State, a: Api, e: Engine) {
    let now = e.time();
    let w = HostWorld { api: a, eng: e, teamplay: false };
    let editing: Vec<(u32, bool, bool, bool, bool)> = s
        .clients
        .iter()
        .filter_map(|c| c.player.map(|p| (p.0, c.waypoint_on, c.waypoint_paths_on, c.auto_waypoint, c.debug_auto_waypoint)))
        .filter(|&(_, on, _, auto, _)| on || auto)
        .collect();
    for (p, on, paths_on, auto, debug) in editing {
        let Some(player) = w.entity(p) else { continue };
        let mut out = Vec::new();
        if on && s.editor.display_time.get(&p).is_none_or(|&t| t < now) {
            s.editor.display_time.insert(p, now + 1.0);
            if let Some(cur) = editor::draw_waypoints(&s.waypoints, &w, p, player.origin, paths_on, &mut out)
                && let Some(ci) = s.clients.index_of(rcbot::config::PlayerId(p))
                && let Some(c) = s.clients.get_mut(ci)
            {
                c.current_waypoint = cur;
            }
        }
        if auto {
            let aw = s.editor.auto.entry(p).or_default();
            aw.think(&mut s.waypoints, &w, &player, on, debug, s.config.auto_path_waypoint, &mut out);
        } else {
            s.editor.auto.remove(&p);
        }
        send(s, e, p, out);
    }
}
