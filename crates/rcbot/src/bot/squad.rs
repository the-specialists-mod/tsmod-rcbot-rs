//! Squads (`bot.h` `CBotSquad`, `CBotSquads`; `bot.cpp` `CBotSquads::AddSquadMember`,
//! `ChangeLeader`, `RemoveSquad`, `CBotSquad::GetFormationVector`, `ReturnAllToFormation`,
//! `BotFunc_MakeSquad`, `CBot::AddToSquad`, `CanAddToSquad`, `RemoveMySquad`,
//! `BotSquadLeaderThink`, `SayInPosition`, the squad parts of `Think` and `UpdateVisibles`, and
//! `BOT_TASK_FOLLOW_LEADER`).
//!
//! TS bots never assemble squads of their own (`LookForNewTasks` leaves `MOD_TS` out); players
//! make them with "Make Squad" (the squad menu), `say_team "form up"`, and change them with the
//! `squad` command and the squad menus. A squad lives in [`Squads`] under an id; a bot holds the
//! id (`m_stSquad`). Upstream's `CBotSquads` reaches into other bots (`SetSquad` on a bot leader,
//! `ClearSquad` on every member of a removed squad, `ReturnAllToFormation`): a removed squad's
//! id simply stops resolving, and the rest is queued as [`SquadOrder`]s the manager hands to the
//! bots before they next think.

use super::look::{make_vectors, vec_to_angles};
use super::{Bot, BotGlobals, BotWorld, EntRef, Entity, LookTask, Task, TaskKind, cond};
use crate::Vector;
use crate::nav::{Glass, Monsters};

/// `SQUAD_DEFAULT_SPREAD`.
pub const SQUAD_DEFAULT_SPREAD: f32 = 50.0;

/// `eSquadForm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquadForm {
    /// `SQUAD_FORM_NONE`.
    None,
    /// `SQUAD_FORM_WEDGE`.
    Wedge,
    /// `SQUAD_FORM_LINE`.
    Line,
    /// `SQUAD_FORM_ECH_LEFT`.
    EchelonLeft,
    /// `SQUAD_FORM_ECH_RIGHT`.
    EchelonRight,
    /// `SQUAD_FORM_COLUMN`.
    Column,
    /// `SQUAD_FORM_VEE`.
    Vee,
}

/// `eCombatType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatType {
    /// `COMBAT_NONE`.
    None,
    /// `COMBAT_STEALTH`.
    Stealth,
    /// `COMBAT_COMBAT`.
    Combat,
    /// `COMBAT_CROUCH`.
    Crouch,
    /// `COMBAT_PRONE`.
    Prone,
}

/// A squad's id (what a bot's `m_stSquad` points at).
pub type SquadId = u32;

/// `CBotSquad`.
#[derive(Clone, Debug, PartialEq)]
pub struct Squad {
    /// Its id.
    pub id: SquadId,
    leader: Option<EntRef>,
    /// `m_theSquad`, a stack (top = end).
    members: Vec<EntRef>,
    formation: SquadForm,
    spread: f32,
    can_fire: bool,
    combat_type: CombatType,
    leader_angle: Vector,
}

/// Something a squad change asks of a bot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquadOrder {
    /// `pBot->SetSquad(squad)`.
    SetSquad(u32, SquadId),
    /// `ReturnAllToFormation`'s `FlushTasks`, a priority `FOLLOW_LEADER` and `SetSquad`.
    ReturnToFormation(u32, SquadId),
}

impl Squad {
    fn new(id: SquadId, leader: &Entity, member: EntRef) -> Squad {
        let mut s = Squad {
            id,
            leader: Some(leader.handle()),
            members: Vec::new(),
            formation: SquadForm::Wedge,
            spread: SQUAD_DEFAULT_SPREAD,
            can_fire: true,
            combat_type: CombatType::Combat,
            leader_angle: Vector::ZERO,
        };
        s.add_member(member);
        s.init(Some(leader));
        s
    }

    /// `Init`: wedge, the default spread, normal combat, open fire, the leader's angles.
    fn init(&mut self, leader: Option<&Entity>) {
        self.formation = SquadForm::Wedge;
        self.spread = SQUAD_DEFAULT_SPREAD;
        self.combat_type = CombatType::Combat;
        self.can_fire = true;
        if let Some(l) = leader {
            self.leader_angle = l.angles;
        }
    }

    /// `GetLeader`.
    pub fn leader(&self) -> Option<EntRef> {
        self.leader
    }

    /// `IsLeader`.
    pub fn is_leader(&self, e: EntRef) -> bool {
        self.leader == Some(e)
    }

    /// `IsMember`.
    pub fn is_member(&self, e: EntRef) -> bool {
        self.members.contains(&e)
    }

    /// `AddMember`.
    fn add_member(&mut self, e: EntRef) {
        if !self.is_member(e) {
            self.members.push(e);
        }
    }

    /// `removeMember`: the topmost copy.
    fn remove_member(&mut self, e: EntRef) {
        if let Some(p) = self.members.iter().rposition(|&m| m == e) {
            self.members.remove(p);
        }
    }

    /// `numMembers`.
    pub fn num_members(&self) -> usize {
        self.members.len()
    }

    /// `ToggleFireMode`, and the message the leader gets.
    pub fn toggle_fire_mode(&mut self) -> &'static str {
        self.can_fire = !self.can_fire;
        if self.can_fire { "Squad can now OPEN FIRE" } else { "Squad is now HOLDING FIRE" }
    }

    /// `SquadCanShoot`.
    pub fn can_shoot(&self) -> bool {
        self.can_fire
    }

    /// `IsStealthMode`.
    pub fn is_stealth(&self) -> bool {
        self.combat_type == CombatType::Stealth
    }

    /// `IsProneMode`.
    pub fn is_prone(&self) -> bool {
        self.combat_type == CombatType::Prone
    }

    /// `IsCrouchMode`.
    pub fn is_crouch(&self) -> bool {
        self.combat_type == CombatType::Crouch
    }

    /// `SetCombatType`, and the message the leader gets (`None` without a leader).
    pub fn set_combat_type(&mut self, t: CombatType) -> Option<String> {
        self.combat_type = t;
        self.leader?;
        let name = match t {
            CombatType::Stealth => "STEALTH",
            CombatType::Prone => "PRONE",
            CombatType::Crouch => "CROUCH",
            CombatType::None | CombatType::Combat => "NORMAL",
        };
        Some(format!("Combat mode is now {name}\n"))
    }

    /// `ChangeFormation`.
    pub fn change_formation(&mut self, f: SquadForm) {
        self.formation = f;
    }

    /// `GetSpread`.
    pub fn spread(&self) -> f32 {
        self.spread
    }

    /// `ChangeSpread`.
    pub fn change_spread(&mut self, spread: f32) {
        self.spread = spread;
    }

    /// `GetFormationPosition`: 1 for the newest member, 0 if not a member.
    fn formation_position(&self, e: EntRef) -> i32 {
        self.members.iter().rev().position(|&m| m == e).map_or(0, |p| p as i32 + 1)
    }

    /// `GetFormationVector`: where `member` stands: the formation's direction off the leader's
    /// heading, times the spread and the member's position, traced from the leader.
    ///
    /// Upstream takes the trace's end *position* and, when the trace was blocked, normalises it
    /// and shortens it by 32 units as if it were the offset, so a blocked point moves towards the
    /// map's origin instead of the leader (design §5). Kept.
    pub fn formation_vector(&self, w: &dyn BotWorld, member: EntRef) -> Vector {
        let Some(leader) = self.leader.and_then(|l| w.entity(l.index).filter(|e| e.serial == l.serial)) else { return Vector::ZERO };
        let position = self.formation_position(member);
        let leader_origin = leader.entity_origin();
        let odd = position % 2 != 0;
        let (forward, right) = make_vectors(self.leader_angle);
        let base = match self.formation {
            SquadForm::None => Vector::ZERO,
            SquadForm::Vee => {
                if odd {
                    forward - right
                } else {
                    forward + right
                }
            }
            SquadForm::Wedge => {
                if odd {
                    -(forward - right)
                } else {
                    -(forward + right)
                }
            }
            SquadForm::Line => {
                if odd {
                    right
                } else {
                    -right
                }
            }
            SquadForm::Column => -forward,
            SquadForm::EchelonLeft => -forward - right,
            SquadForm::EchelonRight => -forward + right,
        };
        let base = base * self.spread * position as f32;
        let tr = w.trace_line(leader_origin, leader_origin + base, Monsters::Ignore, Glass::DontIgnore, Some(leader.index));
        let mut v = tr.end_pos;
        if tr.fraction < 1.0 {
            let d = v.length();
            v = v.normalize() * (d - 32.0);
        }
        v
    }

    /// `UpdateAngles`: the leader's heading while it moves.
    fn update_angles(&mut self, w: &dyn BotWorld) {
        if let Some(l) = self.leader.and_then(|l| w.entity(l.index).filter(|e| e.serial == l.serial))
            && l.velocity.length_2d() != 0.0
        {
            self.leader_angle = vec_to_angles(l.velocity);
        }
    }

    /// `ChangeLeader` (the squad's own): the newest member leads; with nobody left, no leader.
    /// Returns the new leader.
    fn change_leader(&mut self, w: &dyn BotWorld) -> Option<EntRef> {
        if self.members.is_empty() {
            self.leader = None;
        } else {
            self.leader = self.members.pop();
            if self.members.is_empty() {
                self.leader = None;
            } else {
                let l = self.leader.and_then(|l| w.entity(l.index));
                self.init(l.as_ref());
            }
        }
        self.leader
    }
}

/// `CBotSquads`: every squad, a stack (top = end), and what squad changes ask of bots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Squads {
    squads: Vec<Squad>,
    next_id: SquadId,
    /// Orders for bots, taken by [`super::BotManager`].
    pub orders: Vec<SquadOrder>,
    /// Lines for players' chat areas (`ClientPrint(HUD_PRINTTALK)`, `BotPrintTalkMessageOne`).
    pub messages: Vec<(u32, String)>,
    /// What each team's bots tell each other (port addition, design §4), kept here because the
    /// squads are the bots' shared state that lives across frames.
    pub boards: super::board::TeamBoards,
}

impl Squads {
    /// `FreeMemory` (and the team boards).
    pub fn clear(&mut self) {
        self.squads.clear();
        self.orders.clear();
        self.messages.clear();
        self.boards.clear();
    }

    /// The squad with this id, if it still exists.
    pub fn get(&self, id: SquadId) -> Option<&Squad> {
        self.squads.iter().find(|s| s.id == id)
    }

    /// The squad with this id, for changing it.
    pub fn get_mut(&mut self, id: SquadId) -> Option<&mut Squad> {
        self.squads.iter_mut().find(|s| s.id == id)
    }

    /// `FindSquadByLeader`, searching from the top.
    pub fn find_by_leader(&self, leader: EntRef) -> Option<SquadId> {
        self.squads.iter().rev().find(|s| s.is_leader(leader)).map(|s| s.id)
    }

    /// The squads.
    pub fn iter(&self) -> impl Iterator<Item = &Squad> {
        self.squads.iter()
    }

    /// `AddSquadMember(leader, member)`: joins the squad the leader leads or is in, else a new
    /// squad led by them (a bot leader is told). Players of different teams can't. Tells the
    /// leader who joined.
    pub fn add_member(&mut self, leader: &Entity, member: &Entity, leader_is_bot: bool) -> Option<SquadId> {
        if team_of(leader) != team_of(member) {
            return None;
        }
        self.messages.push((leader.index, format!("{} {} has joined your squad", crate::config::BOT_DBG_MSG_TAG, member.netname)));
        let lh = leader.handle();
        if let Some(s) = self.squads.iter_mut().rev().find(|s| s.is_leader(lh) || s.is_member(lh)) {
            s.add_member(member.handle());
            return Some(s.id);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.squads.push(Squad::new(id, leader, member.handle()));
        if leader_is_bot {
            self.orders.push(SquadOrder::SetSquad(leader.index, id));
        }
        Some(id)
    }

    /// `removeSquadMember`: and the squad goes when one or nobody is left.
    pub fn remove_member(&mut self, id: SquadId, member: EntRef) {
        if let Some(s) = self.get_mut(id) {
            s.remove_member(member);
            if s.num_members() <= 1 {
                self.remove(id);
            }
        }
    }

    /// `RemoveSquad`.
    pub fn remove(&mut self, id: SquadId) {
        self.squads.retain(|s| s.id != id);
    }

    /// `CBotSquads::ChangeLeader`: the next member leads; with no leader the squad goes.
    pub fn change_leader(&mut self, id: SquadId, w: &dyn BotWorld) -> Option<EntRef> {
        let s = self.get_mut(id)?;
        let new = s.change_leader(w);
        if new.is_none() {
            self.remove(id);
        }
        new
    }

    /// `ReturnAllToFormation`: every bot member flushes its tasks and follows the leader again.
    pub fn return_all_to_formation(&mut self, id: SquadId, is_bot: impl Fn(u32) -> bool) {
        let Some(s) = self.get(id) else { return };
        let members: Vec<u32> = s.members.iter().rev().map(|m| m.index).filter(|&i| is_bot(i)).collect();
        self.orders.extend(members.into_iter().map(|i| SquadOrder::ReturnToFormation(i, id)));
    }

    /// `m_Squads.UpdateAngles()`, every frame.
    pub fn update_angles(&mut self, w: &dyn BotWorld) {
        for s in &mut self.squads {
            s.update_angles(w);
        }
    }
}

/// `UTIL_GetTeam` for TS: the port's rules team (design §4).
fn team_of(e: &Entity) -> i32 {
    e.player.map_or(0, |p| p.team)
}

impl Bot {
    /// `ClearSquad`.
    pub(crate) fn clear_squad(&mut self) {
        self.squad = None;
        self.squad_leader = None;
    }

    /// `SetSquad`.
    pub(crate) fn set_squad(&mut self, id: SquadId) {
        self.clear_squad();
        self.squad = Some(id);
    }

    /// `m_stSquad`, if it still exists.
    pub fn squad(&self) -> Option<SquadId> {
        self.squad
    }

    /// `IsSquadLeader`: in a squad and following nobody.
    pub(crate) fn is_squad_leader(&self) -> bool {
        self.squad_leader.is_none() && self.squad.is_some()
    }

    /// `CanAddToSquad`: no squad yet, and the leader is a teammate.
    pub fn can_add_to_squad(&self, w: &dyn BotWorld, leader: &Entity) -> bool {
        self.squad.is_none() && !self.is_enemy(w, leader) && self.state.team == team_of(leader)
    }

    /// `AddToSquad`: join and follow.
    pub fn add_to_squad(&mut self, squads: &mut Squads, leader: &Entity, leader_is_bot: bool) {
        self.squad = squads.add_member(leader, &self.me, leader_is_bot);
        if self.squad.is_some() {
            let id = self.tasks.new_schedule_id();
            self.add_priority_task(Task::new(TaskKind::FollowLeader, id, Some(leader.handle()), 0, 0.0, Vector::ZERO));
        }
    }

    /// `RemoveMySquad`: a leader removes its squad.
    pub fn remove_my_squad(&mut self, squads: &mut Squads) -> bool {
        let Some(id) = self.squad else { return false };
        if squads.get(id).is_some_and(|s| s.is_leader(self.me.handle())) {
            squads.remove(id);
            self.squad = None;
            return true;
        }
        false
    }

    /// What a [`SquadOrder`] asks of this bot.
    pub fn squad_order(&mut self, order: SquadOrder) {
        match order {
            SquadOrder::SetSquad(_, id) => self.set_squad(id),
            SquadOrder::ReturnToFormation(_, id) => {
                self.tasks.flush();
                let sid = self.tasks.new_schedule_id();
                self.add_priority_task(Task::new(TaskKind::FollowLeader, sid, None, 0, 0.0, Vector::ZERO));
                self.set_squad(id);
            }
        }
    }

    /// The squad part at the top of `Think`: who leads the bot, the leader's "Let's go!", and
    /// whether the leader is dead (or turned enemy).
    pub(crate) fn squad_think(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld) {
        let now = w.time();
        self.squad_leader = None;
        // A squad that was removed or lost its leader is gone (`ClearSquad`).
        if self.squad.is_some_and(|id| g.squads.get(id).is_none()) {
            self.clear_squad();
        }
        let Some(s) = self.squad.and_then(|id| g.squads.get(id)) else { return };
        if !s.is_leader(self.me.handle()) {
            self.squad_leader = s.leader();
        } else if !self.said_move_out && self.next_use_say_message < now {
            self.actions.push(super::Action::Command("say_team \"Let's go!\"".into()));
            self.said_move_out = true;
            self.next_use_say_message = now + 5.0;
        }
        if self.squad_leader.is_some() {
            match self.resolve(w, self.squad_leader) {
                Some(l) if l.is_alive() => {
                    if self.is_enemy(w, &l) {
                        self.set(cond::SQUAD_LEADER_DEAD);
                    } else {
                        self.clear(cond::SQUAD_LEADER_DEAD);
                    }
                }
                _ => self.set(cond::SQUAD_LEADER_DEAD),
            }
        }
    }

    /// `UpdateConditions`' squad part: whether the bot sees its leader.
    pub(crate) fn update_squad_conditions(&mut self, w: &dyn BotWorld) {
        if let Some(l) = self.resolve(w, self.squad_leader) {
            let tr = w.trace_line(self.gun_position(), l.entity_origin(), Monsters::Ignore, Glass::Ignore, Some(self.index));
            if tr.fraction >= 1.0 {
                self.set(cond::SEE_SQUAD_LEADER);
            } else {
                self.clear(cond::SEE_SQUAD_LEADER);
            }
        }
    }

    /// `Think`'s formation step: a member who sees a living leader (and isn't finding a path or
    /// fighting with a knife) walks to its place, else the bot follows waypoints (a leader also
    /// runs `BotSquadLeaderThink`). True when the formation moved the bot instead of waypoints.
    pub(crate) fn follow_formation(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld) -> bool {
        let mut follow = true;
        if let Some(t) = self.tasks.current() {
            follow = t.kind != TaskKind::FindPath;
            if t.kind == TaskKind::NormalAttack && self.current_weapon.and_then(|id| self.weapons.get(id)).is_some_and(|wp| wp.is_melee()) {
                follow = false;
            }
        }
        let squad = self.squad.and_then(|id| g.squads.get(id));
        if follow
            && let Some(s) = squad
            && self.squad_leader.is_some()
            && self.has(cond::SEE_SQUAD_LEADER)
            && !self.has(cond::SQUAD_LEADER_DEAD)
        {
            let to = s.formation_vector(w, self.me.handle());
            if self.distance_from(to, true) > s.spread() {
                self.set_move_vector(to);
            } else {
                if self.resolve(w, self.squad_leader).is_some_and(|l| l.flags & super::entflags::ONGROUND == 0) {
                    self.touched_waypoints += 1;
                }
                self.stop_moving();
            }
            return true;
        }
        if self.is_squad_leader() {
            self.squad_leader_think(g, w);
        }
        false
    }

    /// `BotSquadLeaderThink`: a leader listening to an enemy's sound makes the squad a vee.
    fn squad_leader_think(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld) {
        if let Some(t) = self.tasks.current()
            && t.kind == TaskKind::ListenToSound
            && let Some(e) = self.resolve(w, t.edict)
            && self.is_enemy(w, &e)
            && let Some(s) = self.squad.and_then(|id| g.squads.get_mut(id))
        {
            s.change_formation(SquadForm::Vee);
        }
    }

    /// `Think`'s squad modes: prone (TS's dive key) and crouch. The "can't shoot" condition it
    /// sets is cleared right after, before any task reads it, so holding fire never works
    /// (design §5). Kept.
    pub(crate) fn squad_modes(&mut self, g: &BotGlobals<'_>) {
        let Some(s) = self.squad.and_then(|id| g.squads.get(id)) else { return };
        let (prone, crouch, can_shoot) = (s.is_prone(), s.is_crouch(), s.can_shoot());
        if prone && self.stunt_ready(self.now) {
            self.alt_button();
        }
        if crouch {
            self.duck();
        }
        if !can_shoot {
            self.set(cond::CANT_SHOOT);
        } else {
            self.clear(cond::CANT_SHOOT);
        }
    }

    /// `EnemyFound`'s squad part: a leader's squad goes back to a normal wedge.
    pub(crate) fn squad_enemy_found(&mut self, squads: &mut Squads) {
        if self.is_squad_leader()
            && let Some(s) = self.squad.and_then(|id| squads.get_mut(id))
        {
            s.change_formation(SquadForm::Wedge);
            if let Some(m) = s.set_combat_type(CombatType::Combat)
                && let Some(l) = s.leader()
            {
                squads.messages.push((l.index, m));
            }
        }
    }

    /// `UpdateVisibles`' last step: seeing a leader it isn't following, the bot follows again.
    pub(crate) fn see_leader_again(&mut self, w: &dyn BotWorld, saw_leader: bool, need_to_follow: bool) {
        if saw_leader
            && need_to_follow
            && let Some(l) = self.resolve(w, self.squad_leader)
            && !self.is_enemy(w, &l)
        {
            let id = self.tasks.new_schedule_id();
            self.add_priority_task(Task::new(TaskKind::FollowLeader, id, Some(l.handle()), 0, 0.0, Vector::ZERO));
            self.said_in_position = false;
        }
    }

    /// `SayInPosition`.
    fn say_in_position(&mut self, now: f32) {
        if !self.said_in_position && self.next_use_say_message < now {
            self.actions.push(super::Action::Command("say_team \"In Position!\"".into()));
            self.said_in_position = true;
            self.next_use_say_message = now + 5.0;
        }
    }

    /// `BOT_TASK_FOLLOW_LEADER`: go to the formation place (by path when the leader is out of
    /// sight), crouch with the leader; a dead leader hands the squad on.
    pub(crate) fn task_follow_leader(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, task: Task, failed: &mut bool) {
        let now = w.time();
        let (Some(leader), Some(id)) = (self.resolve(w, self.squad_leader), self.squad) else {
            *failed = true;
            return;
        };
        self.look_task = LookTask::LookAround;
        if self.has(cond::SQUAD_LEADER_DEAD) {
            let new = g.squads.change_leader(id, w);
            if g.squads.get(id).is_none() {
                self.clear_squad();
            }
            // `SetEdict(m_pSquadLeader)`: the old leader, until the next think finds the new one.
            let _ = new;
            let leader = self.squad_leader;
            if let Some(t) = self.tasks.current_mut() {
                t.edict = leader;
            }
            return;
        }
        let Some(s) = g.squads.get(id) else {
            *failed = true;
            return;
        };
        let to = s.formation_vector(w, self.me.handle());
        let find_path = self.distance_from(to, true) > s.spread();
        if !find_path {
            self.say_in_position(now);
            self.stop_moving();
        }
        if !find_path || self.has(cond::SEE_SQUAD_LEADER) {
            if find_path {
                self.set_move_vector(to);
            }
            if leader.flags & super::entflags::DUCKING != 0 {
                self.duck();
            }
            if let Some(t) = self.tasks.current_mut() {
                t.found_path = false;
            }
            self.current_waypoint = -1;
            self.goal_waypoint = -1;
            self.prev_waypoint = -1;
        } else if !task.found_path {
            self.add_priority_task(Task::new(TaskKind::FindPath, task.schedule_id, Some(leader.handle()), -1, 0.0, Vector::ZERO));
            if let Some(t) = self.tasks.iter_mut().find(|t| t.kind == TaskKind::FollowLeader) {
                t.found_path = true;
            }
        } else {
            *failed = true;
        }
    }
}
