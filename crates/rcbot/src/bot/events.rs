//! What happens to a bot and what it hears and says (`bot.cpp` `CBot::BotEvent` and its
//! `BotEvent_*` handlers, `BotFunc_IncreaseRep`, `BotFunc_DecreaseRep`, `HearSound`, `BotChat`,
//! `ReplyToMessage`, `BOT_TASK_TYPE_MESSAGE`, `havingFun`, `gettingBored`, `WantToLeaveGame`;
//! `engine.cpp` `pfnEmitSound`'s sound kinds; `bot_client.cpp`
//! `BotClient_Generic_DeathMessage`'s events, dispatched by [`super::BotManager::death`]).
//!
//! The port tells the bots about deaths through its `player_killed` callback and about sounds
//! through `sound`; upstream read the `DeathMsg` message and hooked `pfnEmitSound`.

use super::remember::REMEMBER_LOST_ENEMY;
use super::{Action, Bot, BotGlobals, BotWorld, EntRef, Entity, LookTask, Task, TaskKind, cond};
use crate::chat::bot_chat::{ChatContext, ChatKind, ChatPlayer, ReplyParams, bot_chat, bot_chat_reply, rep_array_num, typing_time};
use crate::chat::text::humanize_string;
use crate::config::flags as cfg;
use crate::config::rep::{BOT_MID_REP, decrease_rep_roll, increase_rep_roll};
use crate::{Random, Vector};

/// `BOT_HEAR_DISTANCE`.
pub const BOT_HEAR_DISTANCE: f32 = 650.0;

/// [`BotGlobals::rep_ids`] for a player without a `CClient`.
pub const NO_CLIENT: i32 = i32::MIN;

/// `eSoundType`, the kinds `pfnEmitSound` tells apart for TS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundKind {
    /// `SOUND_UNKNOWN`.
    Unknown,
    /// `SOUND_PLAYER`: `player/…`.
    Player,
    /// `SOUND_PLAYER_PAIN`: `player/pain…`.
    PlayerPain,
    /// `SOUND_DOOR`: `doors/…`, `plats/…`.
    Door,
    /// `SOUND_WEAPON`: `weapons/…`.
    Weapon,
    /// `SOUND_BUTTON`: `buttons/…`.
    Button,
}

impl SoundKind {
    /// `pfnEmitSound`'s classification by sample name (other mods' kinds dropped).
    pub fn of_sample(sample: &str) -> SoundKind {
        if sample.starts_with("doors/") || sample.starts_with("plats/") {
            SoundKind::Door
        } else if sample.starts_with("weapons/") {
            SoundKind::Weapon
        } else if let Some(rest) = sample.strip_prefix("player/") {
            if rest.starts_with("pain") { SoundKind::PlayerPain } else { SoundKind::Player }
        } else if sample.starts_with("buttons/") {
            SoundKind::Button
        } else {
            SoundKind::Unknown
        }
    }
}

impl SoundKind {
    /// Whether a bot does anything on hearing this kind ([`Bot::hear_sound`]): players' sounds.
    pub fn is_heard(self) -> bool {
        matches!(self, SoundKind::Player | SoundKind::PlayerPain)
    }
}

/// `eBotEvent`, the events a TS bot gets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotEvent {
    /// `BOT_EVENT_LEAVING`.
    Leaving,
    /// `BOT_EVENT_HEAR_TEAMMATE_DIE`: the victim.
    HearTeammateDie(EntRef),
    /// `BOT_EVENT_SEE_TEAMMATE_DIE`: victim, killer.
    SeeTeammateDie(EntRef, Option<EntRef>),
    /// `BOT_EVENT_KILL_SELF`.
    KillSelf,
    /// `BOT_EVENT_SEE_PLAYER_DIE` / `SEE_ENEMY_DIE`: the two players (`pInfo`, `pExtInfo`).
    SeePlayerDie(EntRef, Option<EntRef>),
    /// `BOT_EVENT_SEE_TEAMMATE_KILL`: does nothing.
    SeeTeammateKill,
    /// `BOT_EVENT_SEE_ENEMY_KILL`: killer, victim.
    SeeEnemyKill(EntRef, EntRef),
    /// `BOT_EVENT_KILL`: the victim.
    Kill(EntRef),
    /// `BOT_EVENT_DIED`: the killer (the world is index 0).
    Died(Option<EntRef>),
}

/// The players for [`bot_chat`]: names, the bot's reputations, enemies.
struct Chat<'a> {
    to: Option<Vec<u8>>,
    players: Vec<ChatPlayer>,
    map: &'a str,
}

impl ChatContext for Chat<'_> {
    fn chat_to_name(&self) -> Option<Vec<u8>> {
        self.to.clone()
    }
    fn players(&self) -> Vec<ChatPlayer> {
        self.players.clone()
    }
    fn map_name(&self) -> Vec<u8> {
        self.map.as_bytes().to_vec()
    }
}

impl Bot {
    /// The reputation id of a player's `CClient` (`GetClientByEdict(p)->GetPlayerRepId()`),
    /// `None` without one.
    pub(crate) fn client_rep_id(&self, g: &BotGlobals<'_>, index: u32) -> Option<i32> {
        g.rep_ids.get(index as usize).copied().filter(|&r| r != NO_CLIENT)
    }

    /// `m_Profile.m_Rep.GetClientRep(pClient)`.
    pub(crate) fn client_rep(&self, g: &BotGlobals<'_>, index: Option<u32>) -> i32 {
        self.profile.rep.client_rep(index.and_then(|i| self.client_rep_id(g, i))).0
    }

    /// `m_Profile.m_Rep.GetRep(id)`'s current reputation, if the bot has one.
    pub(crate) fn rep_of(&self, g: &BotGlobals<'_>, index: u32) -> Option<i32> {
        let id = self.client_rep_id(g, index)?;
        self.profile.rep.get(id).map(|r| r.rep)
    }

    fn increase_rep_of(&mut self, g: &BotGlobals<'_>, index: u32) {
        if let Some(id) = self.client_rep_id(g, index) {
            let _ = self.profile.rep.increase_rep(id);
        }
    }

    fn decrease_rep_of(&mut self, g: &BotGlobals<'_>, index: u32) {
        if let Some(id) = self.client_rep_id(g, index) {
            let _ = self.profile.rep.decrease_rep(id);
        }
    }

    /// `m_iSaidGreetingsTo &= ~(1 << (player - 1))`: a player who left may be greeted again.
    pub fn forget_greeting(&mut self, player: u32) {
        if (1..=32).contains(&player) {
            self.said_greetings_to &= !(1u32 << (player - 1));
        }
    }

    /// The bot's rules team (`GetTeam()`, design §4).
    pub fn team_number(&self) -> i32 {
        self.state.team
    }

    /// `havingFun`.
    pub(crate) fn having_fun(&mut self) {
        self.boredom = self.boredom.saturating_add(1);
    }

    /// `gettingBored`.
    pub(crate) fn getting_bored(&mut self) {
        if self.boredom > 0 {
            self.boredom -= 1;
        }
    }

    /// `printBoredoms`.
    pub fn boredom_text(&self) -> String {
        format!("{}'s boredom = {} ({:.1} percent)", self.name, self.boredom, f32::from(self.boredom) / 255.0)
    }

    /// `WantToLeaveGame`: `(255 - boredom) / 255` times the share of `mp_timelimit` the bot has
    /// been on the server, over a random 0.75-1.
    pub(crate) fn want_to_leave_game(&self, w: &dyn BotWorld, rng: &mut dyn Random) -> bool {
        let limit = w.cvar_float("mp_timelimit");
        let boredom = (255 - i32::from(self.boredom)) as f32 / 255.0;
        let long_game = 1.0 - (limit * 60.0 - (w.time() - self.join_server_time)) / (limit * 60.0);
        boredom * long_game > rng.random_float(0.75, 1.0)
    }

    /// `CBot::BotEvent`.
    pub fn event(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, ev: BotEvent, rng: &mut dyn Random) {
        match ev {
            BotEvent::Leaving => self.bot_chat(g, w, ChatKind::LeaveGame, None, true, rng),
            BotEvent::HearTeammateDie(victim) => {
                if self.enemy.is_none() {
                    self.add_priority_task(Task::new(TaskKind::FaceEdict, 0, Some(victim), 0, 1.0, crate::Vector::ZERO));
                }
            }
            BotEvent::SeeTeammateDie(victim, killer) => self.see_teammate_die(g, w, victim, killer, rng),
            BotEvent::KillSelf => {
                if rng.random_long(0, 100) < i32::from(g.config.bot_chat_percent) {
                    if self.profile.rep.average_rep_on_server() >= BOT_MID_REP {
                        self.bot_chat(g, w, ChatKind::Laugh, None, false, rng);
                        self.having_fun();
                    } else {
                        self.bot_chat(g, w, ChatKind::Killed, None, false, rng);
                        self.getting_bored();
                    }
                }
            }
            BotEvent::SeePlayerDie(a, b) => self.see_player_die(g, w, a, b, rng),
            BotEvent::SeeTeammateKill => {}
            BotEvent::SeeEnemyKill(killer, victim) => self.see_enemy_kill(g, w, killer, victim, rng),
            BotEvent::Kill(victim) => self.event_kill(g, w, victim, rng),
            BotEvent::Died(killer) => self.event_died(g, w, killer, rng),
        }
    }

    /// `BotEvent_SeeTeammateDie`: attack a visible enemy killer; a teammate's killer the bot
    /// likes the victim can lose reputation; an unseen killer is remembered.
    fn see_teammate_die(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, victim: EntRef, killer: Option<EntRef>, rng: &mut dyn Random) {
        let Some(k) = killer.and_then(|k| self.resolve(w, Some(k))) else { return };
        if self.is_visible(k.index) {
            if self.is_enemy(w, &k) {
                self.enemy = Some(k.handle());
                let id = self.tasks.new_schedule_id();
                self.add_priority_task(Task::new(TaskKind::NormalAttack, id, Some(k.handle()), 0, 0.0, crate::Vector::ZERO));
            } else if self.client_rep(g, Some(victim.index)) > BOT_MID_REP {
                let rep = self.client_rep(g, Some(k.index));
                if decrease_rep_roll(rep, 0.0, 0.0, rng) {
                    self.decrease_rep_of(g, k.index);
                }
            }
        } else {
            self.remembered.add(k.entity_origin(), Some(k.handle()), REMEMBER_LOST_ENEMY, self.me.origin);
        }
    }

    /// `BotEvent_SeePlayerDie`: someone the bot dislikes died; whoever did it gains (or, for a
    /// liked victim, loses) reputation, and the bot laughs.
    fn see_player_die(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, a: EntRef, b: Option<EntRef>, rng: &mut dyn Random) {
        let Some(b) = b else { return };
        let (Some(_), Some(_)) = (self.client_rep_id(g, a.index), self.client_rep_id(g, b.index)) else { return };
        let rep = self.client_rep(g, Some(b.index));
        let dist = match (self.resolve(w, Some(a)), self.resolve(w, Some(b))) {
            (Some(x), Some(y)) => (x.origin - y.origin).length(),
            _ => -1.0,
        };
        let skill = self.profile.skill as f32;
        let victim_rep = self.client_rep(g, Some(a.index));
        if victim_rep < BOT_MID_REP {
            if increase_rep_roll(rep, dist, skill, rng) {
                self.increase_rep_of(g, b.index);
                self.bot_chat(g, w, ChatKind::Laugh, Some(a.index), false, rng);
                self.having_fun();
            }
        } else if victim_rep > BOT_MID_REP && decrease_rep_roll(rep, dist, skill, rng) {
            self.decrease_rep_of(g, b.index);
            self.bot_chat(g, w, ChatKind::Laugh, Some(a.index), false, rng);
            self.having_fun();
        }
    }

    /// `BotEvent_SeeEnemyKill`: attack the killer; if the bot liked the victim, the killer may
    /// lose reputation.
    fn see_enemy_kill(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, killer: EntRef, victim: EntRef, rng: &mut dyn Random) {
        if let Some(k) = self.resolve(w, Some(killer))
            && self.is_enemy(w, &k)
        {
            self.enemy = Some(k.handle());
            let id = self.tasks.new_schedule_id();
            self.add_priority_task(Task::new(TaskKind::NormalAttack, id, Some(k.handle()), 0, 0.0, crate::Vector::ZERO));
        }
        let (Some(_), Some(_)) = (self.client_rep_id(g, killer.index), self.client_rep_id(g, victim.index)) else { return };
        let rep = self.client_rep(g, Some(victim.index));
        if self.client_rep(g, Some(killer.index)) > BOT_MID_REP && decrease_rep_roll(rep, 0.0, 0.0, rng) {
            self.decrease_rep_of(g, victim.index);
            self.getting_bored();
        }
    }

    /// `BotEvent_Kill`: chat about the kill; a disliked victim may gain reputation.
    fn event_kill(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, victim: EntRef, rng: &mut dyn Random) {
        let now = w.time();
        if rng.random_long(0, 100) < i32::from(g.config.bot_chat_percent) {
            self.bot_chat(g, w, ChatKind::Kills, Some(victim.index), false, rng);
            self.having_fun();
        } else if self.next_use_say_message < now {
            self.next_use_say_message = now + rng.random_float(10.0, 20.0);
        }
        if self.client_rep_id(g, victim.index).is_some() && self.client_rep(g, Some(victim.index)) < BOT_MID_REP {
            let rep = self.client_rep(g, Some(victim.index));
            let dist = self.resolve(w, Some(victim)).map_or(-1.0, |v| (v.origin - self.me.origin).length());
            if increase_rep_roll(rep, dist, self.profile.skill as f32, rng) {
                self.increase_rep_of(g, victim.index);
            }
        }
    }

    /// `BotEvent_Died`: chat about it; the killer loses reputation (always for a teammate).
    fn event_died(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, killer: Option<EntRef>, rng: &mut dyn Random) {
        self.getting_bored();
        let Some(k) = killer else { return };
        if rng.random_long(0, 100) < i32::from(g.config.bot_chat_percent) {
            self.bot_chat(g, w, ChatKind::Killed, Some(k.index), false, rng);
        }
        if self.client_rep_id(g, k.index).is_none() {
            return;
        }
        let killer_ent = self.resolve(w, Some(k));
        // Upstream compares `pev->team`, which TS 3.0 doesn't set: the rules team (design §4).
        let killer_team = killer_ent.as_ref().and_then(|e| e.player).map_or(0, |p| p.team);
        if killer_team != 0 && killer_team == self.state.team {
            self.decrease_rep_of(g, k.index);
        } else if k.index != self.index {
            let rep = self.client_rep(g, Some(k.index));
            let dist = killer_ent.map_or(-1.0, |e| (e.origin - self.me.origin).length());
            if decrease_rep_roll(rep, dist, self.profile.skill as f32, rng) {
                self.decrease_rep_of(g, k.index);
            }
        }
    }

    /// `CBot::HearSound`, TS's kinds: an enemy player's sound at `origin` (unless building a
    /// human tower) is remembered ([`super::hearing::Hearing`]) and the bot glances at it; pain of
    /// its task's entity is noted.
    ///
    /// Upstream added a `LISTEN_TO_SOUND` task without an entity, which failed at once (§5.58)
    /// after making every later task forget its path; teammates' sounds are ignored as before.
    pub fn hear_sound(&mut self, w: &dyn BotWorld, kind: SoundKind, source: &Entity, origin: Vector) {
        match kind {
            SoundKind::PlayerPain => {
                if self.tasks.current().and_then(|t| t.edict) == Some(source.handle()) {
                    self.set(cond::TASK_EDICT_PAIN);
                }
            }
            SoundKind::Player => {
                if self.is_enemy(w, source) && self.enemy.is_none() && self.tasks.current().is_none_or(|t| t.kind != TaskKind::HumanTower) {
                    let now = w.time();
                    self.hearing.add(origin, now, super::hearing::LOUD_STEPS, Some(source.handle()));
                    self.watch(origin, now, 0.8);
                }
            }
            SoundKind::Unknown | SoundKind::Door | SoundKind::Weapon | SoundKind::Button => {}
        }
    }

    /// The players [`bot_chat`] may name.
    fn chat_players(&self, g: &BotGlobals<'_>, w: &dyn BotWorld) -> Vec<ChatPlayer> {
        (1..=w.max_clients())
            .filter(|&i| i != self.index)
            .filter_map(|i| w.entity(i))
            .map(|p| ChatPlayer {
                name: p.netname.as_bytes().to_vec(),
                rep: self.client_rep(g, Some(p.index)),
                is_enemy: self.is_enemy(w, &p),
            })
            .collect()
    }

    /// `CBot::BotChat(type, pChatEdict, bSayNow)`: picks a line for the player talked to (by the
    /// bot's reputation of them, the server's average for a non-client, the bad -1 slot for
    /// nobody) and types it in a message task. Greetings go once to each player.
    pub fn bot_chat(&mut self, g: &BotGlobals<'_>, w: &dyn BotWorld, kind: ChatKind, to: Option<u32>, say_now: bool, rng: &mut dyn Random) {
        if !g.config.is_on(cfg::CHATTING) || self.tasks.has_task(TaskKind::TypeMessage) {
            return;
        }
        let to_ent = to.and_then(|i| w.entity(i));
        let to_client = to_ent.as_ref().filter(|e| e.is_client()).and_then(|e| self.client_rep_id(g, e.index).map(|_| e.index));
        let mut array_num = -1;
        if to.is_some() {
            array_num = match to_client {
                None => rep_array_num(self.profile.rep.average_rep_on_server()),
                Some(i) => rep_array_num(self.client_rep(g, Some(i))),
            };
        }
        if kind == ChatKind::Greetings {
            // `ENTINDEX(pChatEdict) - 1`: nobody is index -1.
            let Some(i) = to.filter(|&i| (1..=crate::config::MAX_PLAYERS as u32).contains(&i)) else { return };
            let bit = 1u32 << (i - 1);
            if self.said_greetings_to & bit != 0 {
                return;
            }
            self.said_greetings_to |= bit;
            array_num = rep_array_num(self.client_rep(g, to_client));
        }
        let name = to_ent.as_ref().filter(|e| e.is_client()).map(|e| e.netname.as_bytes().to_vec());
        let ctx = Chat { to: name, players: self.chat_players(g, w), map: g.map_name };
        let Some(line) = bot_chat(g.chat, kind, array_num, &ctx, rng) else { return };
        self.chat_string = line.text;
        let t = Task::new(TaskKind::TypeMessage, 0, None, i32::from(line.team_only), 0.0, crate::Vector::ZERO);
        if say_now {
            self.add_priority_task(t);
        } else {
            self.add_task(t);
        }
    }

    /// `CBot::ReplyToMessage` (with `BotChatReply`): a MegaHAL reply, typed in a priority message
    /// task. `message` is the shared buffer every bot sees in turn (design §5).
    #[allow(clippy::too_many_arguments)] // upstream's arguments and the shared buffer
    pub fn reply_to_message(
        &mut self,
        g: &BotGlobals<'_>,
        w: &dyn BotWorld,
        message: &mut Vec<u8>,
        sender: &Entity,
        sender_is_bot: bool,
        team_only: bool,
        rng: &mut dyn Random,
    ) {
        if self.tasks.has_task(TaskKind::TypeMessage) {
            return;
        }
        let Some(hal) = self.hal.as_mut() else { return };
        let num_clients = (1..=w.max_clients()).filter(|&i| w.entity(i).is_some_and(|e| e.is_client() && !e.netname.is_empty())).count();
        let sender_rep = self.profile.rep.client_rep(g.rep_ids.get(sender.index as usize).copied().filter(|&r| r != NO_CLIENT)).0;
        let p = ReplyParams {
            bot_name: self.name.as_bytes(),
            sender_name: sender.netname.as_bytes(),
            sender_is_self: sender.index == self.index,
            sender_is_bot,
            sender_rep,
            num_clients: num_clients as i32,
            chatting: g.config.is_on(cfg::CHATTING),
            real_mode: g.config.is_on(cfg::REAL_MODE),
            dont_learn: g.config.is_on(cfg::CHAT_DONT_LEARN),
            reply_percent: i32::from(g.config.bot_chat_reply_percent),
        };
        if let Some(reply) = bot_chat_reply(hal, message, &p, rng) {
            self.chat_string = reply;
            self.add_priority_task(Task::new(
                TaskKind::TypeMessage,
                0,
                Some(sender.handle()),
                i32::from(team_only),
                0.0,
                crate::Vector::ZERO,
            ));
        }
    }

    /// `BOT_TASK_TYPE_MESSAGE`: stand still "typing" (its length over the typing speed), then
    /// say it, humanised.
    pub(crate) fn task_type_message(&mut self, w: &dyn BotWorld, rng: &mut dyn Random, task: Task, done: &mut bool, failed: &mut bool) {
        let now = w.time();
        self.look_task = LookTask::None;
        self.stop_moving();
        if task.float != 0.0 {
            if task.float <= now {
                let mut text = std::mem::take(&mut self.chat_string);
                humanize_string(&mut text, rng);
                let text = String::from_utf8_lossy(&text);
                let cmd = if task.int == 1 { "say_team" } else { "say" };
                self.actions.push(Action::Command(format!("{cmd} \"{text}\"")));
                *done = true;
            }
        } else if !self.chat_string.is_empty() {
            let t = now + typing_time(&self.chat_string);
            self.tasks.current_mut().expect("current task").float = t;
        } else {
            *failed = true;
        }
    }
}
