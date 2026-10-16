//! The combat ledger (a port addition, design §4): what passed between a bot and each player,
//! from the port's damage and kill callbacks (`damage`, `kill_scored`), so the bot's
//! learners score their choices by what they did to *that* opponent and what it did back, not
//! by health deltas that mix in every other source of damage.
//!
//! Each bot keeps a ring of recent [`LedgerEntry`]s and its fights as [`Episode`]s, one per
//! opponent: an episode opens when the bot engages (or first trades damage with) a player and
//! closes after [`EPISODE_TIMEOUT`] without contact, when the bot loses the enemy, or at a
//! death. Rewards are `(dealt - taken) / 100`, `+1` a kill and `-1` a death, counted only
//! between the bot and that opponent.

use std::collections::VecDeque;

/// Upper ends of the range bins (units): 0-64, 64-192, 192-384, 384-768, 768-1500, over 1500.
pub const RANGE_BIN_LIMITS: [f32; 5] = [64.0, 192.0, 384.0, 768.0, 1500.0];
/// Number of range bins.
pub const NUM_RANGE_BINS: usize = RANGE_BIN_LIMITS.len() + 1;
/// Entries a bot keeps.
pub const LEDGER_LEN: usize = 128;
/// Closed episodes a bot keeps.
pub const CLOSED_EPISODES: usize = 16;
/// Seconds without contact after which an episode closes.
pub const EPISODE_TIMEOUT: f32 = 4.0;
/// Time constant (s) of the credit a decision gets for what follows it.
pub const CREDIT_TAU: f32 = 1.0;

/// The range bin of a distance.
pub fn range_bin(dist: f32) -> usize {
    RANGE_BIN_LIMITS.iter().position(|&b| dist < b).unwrap_or(NUM_RANGE_BINS - 1)
}

/// The port's `damage` callback (plugin API v6's `DamageEvent`): `attacker` hurt `victim` (entity
/// indices; 0 the world).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DamageEvent {
    /// The player hurt.
    pub victim: u32,
    /// Who hurt it (0 the world).
    pub attacker: u32,
    /// What hurt it (a grenade, a thrown knife, the attacker itself), -1 none.
    pub inflictor: i32,
    /// The damage asked for.
    pub amount: f32,
    /// `DMG_*` bits.
    pub bits: i32,
    /// The victim's health left.
    pub health: f32,
    /// The victim's armour left.
    pub armor: f32,
    /// Where the blow landed (1 head, 2 chest, 3 stomach, 4/5 arms, 6/7 legs), 0 for damage that
    /// didn't come through a trace (falls, explosions).
    pub hitgroup: i32,
    /// The blow's TS weapon id as the rules count it (0 kung fu, 24 a grenade), -1 unknown.
    pub weapon: i32,
    /// The shot's direction of travel (unit) and where it hit, zero without a trace.
    pub dir: crate::Vector,
    /// See [`dir`](Self::dir).
    pub pos: crate::Vector,
}

/// [`DamageEvent::hitgroup`] of a headshot.
pub const HITGROUP_HEAD: i32 = 1;

/// `kill_scored`: the rules scored a death.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KillEvent {
    /// The killing player (the victim for a suicide, 0 for none).
    pub killer: u32,
    /// Who died.
    pub victim: u32,
    /// The TS weapon id of the blow (0 kung fu, 24 a grenade, -1 anything else).
    pub weapon: i32,
    /// `glast_headshot`.
    pub headshot: bool,
    /// `glast_closecombat`.
    pub close_combat: bool,
    /// A kung fu frag.
    pub kung_fu: bool,
}

/// What the host tells the bots about fights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CombatEvent {
    /// Damage.
    Damage(DamageEvent),
    /// A scored death.
    Kill(KillEvent),
}

/// One thing that passed between the bot and another player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LedgerEntry {
    /// Game time.
    pub time: f32,
    /// The other player (0 the world).
    pub other: u32,
    /// The TS weapon id used (the bot's own when it dealt, the other's when it took; -1 unknown).
    pub weapon: i32,
    /// Damage the bot dealt.
    pub dealt: f32,
    /// Damage the bot took.
    pub taken: f32,
    /// The bot killed the other.
    pub kill: bool,
    /// The other killed the bot.
    pub death: bool,
    /// [`range_bin`] of the distance between them.
    pub range_bin: u8,
    /// The bot's waypoint, -1 none.
    pub waypoint: i32,
}

impl LedgerEntry {
    /// The entry's reward: `(dealt - taken) / 100`, `+1` a kill, `-1` a death.
    pub fn reward(&self) -> f32 {
        (self.dealt - self.taken) / 100.0 + f32::from(u8::from(self.kill)) - f32::from(u8::from(self.death))
    }
}

/// A fight with one opponent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Episode {
    /// The opponent.
    pub opponent: u32,
    /// When it opened.
    pub start: f32,
    /// The last contact (or the engagement).
    pub last: f32,
    /// Damage dealt to the opponent.
    pub dealt: f32,
    /// Damage taken from it.
    pub taken: f32,
    /// The bot killed it.
    pub kill: bool,
    /// It killed the bot.
    pub death: bool,
}

impl Episode {
    /// `(dealt - taken) / 100`, `+1` a kill, `-1` a death.
    pub fn reward(&self) -> f32 {
        (self.dealt - self.taken) / 100.0 + f32::from(u8::from(self.kill)) - f32::from(u8::from(self.death))
    }
}

/// A bot's ledger.
#[derive(Clone, Debug, Default)]
pub struct Ledger {
    entries: VecDeque<LedgerEntry>,
    open: Vec<Episode>,
    closed: VecDeque<Episode>,
}

impl Ledger {
    /// Records an entry and adds it to the episode with its player (opening one if needed);
    /// a kill or death closes the episode.
    pub fn record(&mut self, e: LedgerEntry) {
        if self.entries.len() == LEDGER_LEN {
            self.entries.pop_front();
        }
        self.entries.push_back(e);
        if e.other == 0 {
            return;
        }
        self.engage(e.time, e.other);
        if let Some(ep) = self.open.iter_mut().find(|x| x.opponent == e.other) {
            ep.last = e.time;
            ep.dealt += e.dealt;
            ep.taken += e.taken;
            ep.kill |= e.kill;
            ep.death |= e.death;
        }
        if e.kill || e.death {
            self.close(e.other);
        }
    }

    /// The bot engages `opponent` now: opens an episode, or keeps the open one alive.
    pub fn engage(&mut self, now: f32, opponent: u32) {
        match self.open.iter_mut().find(|x| x.opponent == opponent) {
            Some(ep) => ep.last = ep.last.max(now),
            None => self.open.push(Episode { opponent, start: now, last: now, dealt: 0.0, taken: 0.0, kill: false, death: false }),
        }
    }

    /// Closes episodes without contact for [`EPISODE_TIMEOUT`].
    pub fn tick(&mut self, now: f32) {
        let stale: Vec<u32> = self.open.iter().filter(|e| now - e.last > EPISODE_TIMEOUT).map(|e| e.opponent).collect();
        for o in stale {
            self.close(o);
        }
    }

    /// Closes the episode with `opponent` (the enemy was lost).
    pub fn close(&mut self, opponent: u32) {
        if let Some(i) = self.open.iter().position(|e| e.opponent == opponent) {
            let ep = self.open.remove(i);
            if self.closed.len() == CLOSED_EPISODES {
                self.closed.pop_front();
            }
            self.closed.push_back(ep);
        }
    }

    /// Closes every episode (the bot died).
    pub fn close_all(&mut self) {
        let all: Vec<u32> = self.open.iter().map(|e| e.opponent).collect();
        for o in all {
            self.close(o);
        }
    }

    /// The entries, oldest first.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &LedgerEntry> {
        self.entries.iter()
    }

    /// The open episode with `opponent`.
    pub fn episode(&self, opponent: u32) -> Option<&Episode> {
        self.open.iter().find(|e| e.opponent == opponent)
    }

    /// The open episodes.
    pub fn open_episodes(&self) -> &[Episode] {
        &self.open
    }

    /// The closed episodes, oldest first.
    pub fn closed_episodes(&self) -> impl DoubleEndedIterator<Item = &Episode> {
        self.closed.iter()
    }

    /// Entries at or after `since` with `opponent` (any player for `None`), newest first.
    fn since(&self, since: f32, opponent: Option<u32>) -> impl Iterator<Item = &LedgerEntry> {
        self.entries.iter().rev().take_while(move |e| e.time >= since).filter(move |e| opponent.is_none_or(|o| e.other == o))
    }

    /// The reward since `since` between the bot and `opponent` only.
    pub fn reward_since(&self, since: f32, opponent: u32) -> f32 {
        self.since(since, Some(opponent)).map(LedgerEntry::reward).sum()
    }

    /// Damage dealt to `opponent` (any player for `None`) since `since`.
    pub fn dealt_since(&self, since: f32, opponent: Option<u32>) -> f32 {
        self.since(since, opponent).map(|e| e.dealt).sum()
    }

    /// Damage taken from `opponent` (anyone, the world included, for `None`) since `since`.
    pub fn taken_since(&self, since: f32, opponent: Option<u32>) -> f32 {
        self.since(since, opponent).map(|e| e.taken).sum()
    }

    /// Kills of anyone since `since`.
    pub fn kills_since(&self, since: f32) -> u32 {
        self.since(since, None).filter(|e| e.kill).count() as u32
    }

    /// The credit a decision taken at `decision` gets: the rewards with `opponent` (anyone for
    /// `None`) within `horizon` seconds after it, each discounted by `exp(-dt / 1 s)`.
    pub fn credit(&self, decision: f32, opponent: Option<u32>, horizon: f32) -> f32 {
        self.since(decision, opponent)
            .filter(|e| e.time <= decision + horizon)
            .map(|e| e.reward() * (-(e.time - decision) / CREDIT_TAU).exp())
            .sum()
    }

    /// Forgets everything.
    pub fn clear(&mut self) {
        *self = Ledger::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(time: f32, other: u32, dealt: f32, taken: f32) -> LedgerEntry {
        LedgerEntry { time, other, weapon: 1, dealt, taken, kill: false, death: false, range_bin: 2, waypoint: -1 }
    }

    #[test]
    fn bins() {
        assert_eq!(range_bin(0.0), 0);
        assert_eq!(range_bin(64.0), 1);
        assert_eq!(range_bin(700.0), 3);
        assert_eq!(range_bin(1499.0), 4);
        assert_eq!(range_bin(9000.0), 5);
    }

    #[test]
    fn rewards_count_only_that_opponent() {
        let mut l = Ledger::default();
        l.record(entry(1.0, 2, 30.0, 0.0));
        l.record(entry(1.5, 3, 0.0, 50.0));
        l.record(entry(2.0, 2, 0.0, 10.0));
        assert!((l.reward_since(0.0, 2) - 0.2).abs() < 1e-6);
        assert!((l.reward_since(0.0, 3) + 0.5).abs() < 1e-6);
        assert!((l.reward_since(1.8, 2) + 0.1).abs() < 1e-6);
        assert_eq!(l.taken_since(0.0, None), 60.0);
        assert_eq!(l.dealt_since(0.0, Some(3)), 0.0);
        // Credit decays with the time after the decision.
        let c = l.credit(1.0, Some(2), 2.0);
        assert!((c - (0.3 - 0.1 * (-1.0f32).exp())).abs() < 1e-6, "{c}");
        assert_eq!(l.credit(1.0, Some(2), 0.5), 0.3);
    }

    #[test]
    fn episodes_open_on_contact_and_close_on_timeout_or_death() {
        let mut l = Ledger::default();
        l.engage(1.0, 2);
        l.record(entry(2.0, 2, 40.0, 20.0));
        assert_eq!(l.episode(2).map(|e| (e.start, e.dealt, e.taken)), Some((1.0, 40.0, 20.0)));
        l.tick(5.0);
        assert!(l.episode(2).is_some());
        l.tick(6.5);
        assert!(l.episode(2).is_none());
        assert!((l.closed_episodes().last().unwrap().reward() - 0.2).abs() < 1e-6);
        l.record(LedgerEntry { death: true, ..entry(7.0, 4, 0.0, 100.0) });
        assert!(l.episode(4).is_none());
        assert_eq!(l.closed_episodes().last().unwrap().reward(), -2.0);
        // The ring keeps the newest.
        for i in 0..(LEDGER_LEN + 10) {
            l.record(entry(10.0 + i as f32, 5, 1.0, 0.0));
        }
        assert_eq!(l.entries().count(), LEDGER_LEN);
        assert_eq!(l.entries().next().unwrap().time, 20.0);
    }
}
