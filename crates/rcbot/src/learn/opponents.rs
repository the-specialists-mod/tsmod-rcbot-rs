//! What the bots learn about the players they meet (a port addition, design §4): per player
//! (humans and other backends' bots by reputation id, RCBot's bots by profile), from the host's
//! damage, kill and stunt events and what bots see: how much it hurts bots per second in their
//! sight, its weapons and range, kung fu share, stunts, dodging, and its kung fu (attack mix,
//! blocks, rhythm). [`OpponentModel::threat`] scores it for target choice; the kung fu fighter
//! starts from its rhythm. Saved in `botprofiles/players.rop`.

use super::store;

/// Who a player is across maps: humans (and other backends' bots) by their reputation id
/// (`bot_playerids.ini`), RCBot's own bots by their profile id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OpponentKey {
    /// A player by reputation id.
    Player(i32),
    /// One of RCBot's bots by profile id (0: bots without a profile, all together).
    Bot(i32),
}

/// The key of each player slot (entity index 1..=32), refreshed by the bot manager.
#[derive(Clone, Debug, Default)]
pub struct PlayerKeys {
    keys: Vec<Option<OpponentKey>>,
}

impl PlayerKeys {
    /// Sets the keys: RCBot's bots by profile (`bots`: index, profile id), everyone else by
    /// reputation id (`rep_ids` by entity index, [`crate::bot::NO_CLIENT`] for none).
    pub fn refresh(&mut self, rep_ids: &[i32], bots: impl Iterator<Item = (u32, i32)>) {
        self.keys.clear();
        self.keys.extend(rep_ids.iter().map(|&r| (r != crate::bot::NO_CLIENT).then_some(OpponentKey::Player(r))));
        for (i, profile) in bots {
            if let Some(k) = self.keys.get_mut(i as usize) {
                *k = Some(OpponentKey::Bot(profile));
            }
        }
    }

    /// The key of the player at `index`.
    pub fn get(&self, index: u32) -> Option<OpponentKey> {
        self.keys.get(index as usize).copied().flatten()
    }
}

/// Kung fu attack kinds counted ([`crate::bot::kungfu::ATTACK_KINDS`]).
pub const KF_KINDS: usize = 5;
/// TS weapon ids counted (0 kung fu .. 37).
pub const WEAPON_IDS: usize = 38;
/// Moving averages' step.
const EMA: f32 = 0.05;
/// What a player's counts keep at each map load.
pub const MAP_DECAY: f32 = 0.95;
/// Seconds of exposure a player's damage rate is shrunk toward everyone's with.
const THREAT_PRIOR: f32 = 10.0;
/// The players file, in the profiles folder.
pub const PLAYERS_FILE: &str = "players.rop";

const FILE_KIND: &str = "players";
const FILE_VERSION: u32 = 1;

/// What the bots saw of one player.
#[derive(Clone, Debug, PartialEq)]
pub struct OpponentProfile {
    /// Bot-seconds some bot had the player in sight as its enemy.
    pub exposure: f32,
    /// Damage it dealt to bots, and its hits on them.
    pub damage_to_bots: f32,
    /// Hits on bots.
    pub hits_on_bots: f32,
    /// Bots it killed.
    pub kills_of_bots: f32,
    /// Its deaths to bots.
    pub deaths_to_bots: f32,
    /// Its hits on bots by TS weapon id.
    pub weapons: [f32; WEAPON_IDS],
    /// Moving average of the distance it hit bots from.
    pub range: f32,
    /// Moving average share of its hits with kung fu (or a blade).
    pub kungfu_share: f32,
    /// Stunts started, and a moving average of them per minute (from the gaps between them).
    pub stunts: f32,
    /// Stunts per minute.
    pub stunt_rate: f32,
    /// Moving average of its sideways speed across a bot's view (/ 290) while seen: dodging.
    pub dodge: f32,
    /// Kung fu fights with a bot, its attacks by kind and blocks in them, and its moving average
    /// time between attacks.
    pub kf_fights: f32,
    /// Its kung fu attacks by kind.
    pub kf_attacks: [f32; KF_KINDS],
    /// Its blocks.
    pub kf_blocks: f32,
    /// Seconds between its kung fu attacks.
    pub kf_interval: f32,
    /// When its last stunt started (this map; not saved).
    last_stunt: f32,
}

impl Default for OpponentProfile {
    fn default() -> Self {
        OpponentProfile {
            exposure: 0.0,
            damage_to_bots: 0.0,
            hits_on_bots: 0.0,
            kills_of_bots: 0.0,
            deaths_to_bots: 0.0,
            weapons: [0.0; WEAPON_IDS],
            range: 0.0,
            kungfu_share: 0.0,
            stunts: 0.0,
            stunt_rate: 0.0,
            dodge: 0.0,
            kf_fights: 0.0,
            kf_attacks: [0.0; KF_KINDS],
            kf_blocks: 0.0,
            kf_interval: 1.0,
            last_stunt: 0.0,
        }
    }
}

/// `avg += rate * (x - avg)`, the first sample taken as it is.
fn ema(avg: &mut f32, x: f32, rate: f32, first: bool) {
    if first {
        *avg = x;
    } else {
        *avg += rate * (x - *avg);
    }
}

impl OpponentProfile {
    /// Hits on bots per second in their sight.
    pub fn accuracy(&self) -> Option<f32> {
        (self.exposure >= 1.0).then(|| self.hits_on_bots / self.exposure)
    }

    /// The weapon it hit bots with most, with its share.
    pub fn favourite_weapon(&self) -> Option<(i32, f32)> {
        let total: f32 = self.weapons.iter().sum();
        let (i, &n) = self.weapons.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
        (n > 0.0).then(|| (i as i32, n / total))
    }

    /// Its kung fu block rate: blocks per attack.
    pub fn kf_block_rate(&self) -> Option<f32> {
        let attacks: f32 = self.kf_attacks.iter().sum();
        (attacks + self.kf_blocks >= 1.0).then(|| self.kf_blocks / (attacks + self.kf_blocks))
    }

    fn fields(&self) -> Vec<f32> {
        let mut v = vec![
            self.exposure,
            self.damage_to_bots,
            self.hits_on_bots,
            self.kills_of_bots,
            self.deaths_to_bots,
            self.range,
            self.kungfu_share,
            self.stunts,
            self.stunt_rate,
            self.dodge,
            self.kf_fights,
            self.kf_blocks,
            self.kf_interval,
        ];
        v.extend_from_slice(&self.kf_attacks);
        v.extend_from_slice(&self.weapons);
        v
    }

    fn from_fields(v: &[f32]) -> Option<OpponentProfile> {
        if v.len() != 13 + KF_KINDS + WEAPON_IDS || v.iter().any(|&x| x < 0.0) {
            return None;
        }
        let mut p = OpponentProfile {
            exposure: v[0],
            damage_to_bots: v[1],
            hits_on_bots: v[2],
            kills_of_bots: v[3],
            deaths_to_bots: v[4],
            range: v[5],
            kungfu_share: v[6],
            stunts: v[7],
            stunt_rate: v[8],
            dodge: v[9],
            kf_fights: v[10],
            kf_blocks: v[11],
            kf_interval: v[12],
            ..OpponentProfile::default()
        };
        p.kf_attacks.copy_from_slice(&v[13..13 + KF_KINDS]);
        p.weapons.copy_from_slice(&v[13 + KF_KINDS..]);
        Some(p)
    }
}

/// What the bots know of every player they met.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpponentModel {
    players: std::collections::BTreeMap<OpponentKey, OpponentProfile>,
}

impl OpponentModel {
    /// What is known of `key`.
    pub fn profile(&self, key: OpponentKey) -> Option<&OpponentProfile> {
        self.players.get(&key)
    }

    fn entry(&mut self, key: OpponentKey) -> &mut OpponentProfile {
        self.players.entry(key).or_default()
    }

    /// `key` hit a bot for `damage` with `weapon` (TS id) from `dist`.
    pub fn observe_hit_on_bot(&mut self, key: OpponentKey, weapon: i32, dist: f32, damage: f32) {
        let p = self.entry(key);
        let first = p.hits_on_bots == 0.0;
        p.damage_to_bots += damage.max(0.0);
        p.hits_on_bots += 1.0;
        let melee = matches!(weapon, 0 | 25 | 34 | 35 | 38);
        let slot = if weapon == 38 { 0 } else { weapon };
        if let Some(w) = usize::try_from(slot).ok().and_then(|w| p.weapons.get_mut(w)) {
            *w += 1.0;
        }
        ema(&mut p.range, dist, EMA, first);
        ema(&mut p.kungfu_share, f32::from(u8::from(melee)), EMA, first);
    }

    /// `key` killed a bot.
    pub fn observe_kill_of_bot(&mut self, key: OpponentKey) {
        self.entry(key).kills_of_bots += 1.0;
    }

    /// `key` died to a bot.
    pub fn observe_death_to_bot(&mut self, key: OpponentKey) {
        self.entry(key).deaths_to_bots += 1.0;
    }

    /// A bot had `key` in sight as its enemy for `dt` seconds, moving `sideways` (its speed
    /// across the bot's view, / 290).
    pub fn observe_exposure(&mut self, key: OpponentKey, dt: f32, sideways: f32) {
        if dt <= 0.0 {
            return;
        }
        let p = self.entry(key);
        let first = p.exposure == 0.0;
        p.exposure += dt;
        ema(&mut p.dodge, sideways.clamp(0.0, 2.0), (EMA * dt * 10.0).min(1.0), first);
    }

    /// `key` started a stunt at `now`.
    pub fn observe_stunt(&mut self, key: OpponentKey, now: f32) {
        let p = self.entry(key);
        p.stunts += 1.0;
        if p.last_stunt > 0.0 && now > p.last_stunt {
            let per_minute = (60.0 / (now - p.last_stunt)).min(60.0);
            let first = p.stunt_rate == 0.0;
            ema(&mut p.stunt_rate, per_minute, EMA * 2.0, first);
        }
        p.last_stunt = now;
    }

    /// A kung fu fight with `key` ended: its attacks by kind, its blocks, and the time between
    /// its attacks (if it attacked twice).
    pub fn observe_kungfu(&mut self, key: OpponentKey, attacks: &[u32], blocks: u32, interval: Option<f32>) {
        let p = self.entry(key);
        let first = p.kf_fights == 0.0;
        p.kf_fights += 1.0;
        for (a, &n) in p.kf_attacks.iter_mut().zip(attacks) {
            *a += n as f32;
        }
        p.kf_blocks += blocks as f32;
        if let Some(i) = interval {
            ema(&mut p.kf_interval, i, 0.2, first);
        }
    }

    /// Everyone's damage to bots per second of exposure.
    fn mean_rate(&self) -> f32 {
        let (d, e) = self.players.values().fold((0.0, 0.0), |(d, e), p| (d + p.damage_to_bots, e + p.exposure));
        if e >= THREAT_PRIOR { d / e } else { 10.0 }
    }

    /// How dangerous `key` is to bots, 1 for an average (or unknown) player, 0.25-4: its damage
    /// to bots per second in their sight shrunk toward everyone's (10 s of it), over everyone's,
    /// times the square root of its kills of bots over its deaths to them (each +1, the ratio
    /// held to 0.5-2).
    pub fn threat(&self, key: OpponentKey) -> f32 {
        let Some(p) = self.players.get(&key) else { return 1.0 };
        let mean = self.mean_rate().max(1e-3);
        let rate = (p.damage_to_bots + THREAT_PRIOR * mean) / (p.exposure + THREAT_PRIOR);
        let kd = ((p.kills_of_bots + 1.0) / (p.deaths_to_bots + 1.0)).clamp(0.5, 2.0);
        (rate / mean * kd.sqrt()).clamp(0.25, 4.0)
    }

    /// A new map: counts fade, stunt timing restarts.
    pub fn new_map(&mut self) {
        for p in self.players.values_mut() {
            for v in
                [&mut p.exposure, &mut p.damage_to_bots, &mut p.hits_on_bots, &mut p.kills_of_bots, &mut p.deaths_to_bots, &mut p.stunts]
            {
                *v *= MAP_DECAY;
            }
            for v in p.weapons.iter_mut().chain(p.kf_attacks.iter_mut()) {
                *v *= MAP_DECAY;
            }
            p.kf_blocks *= MAP_DECAY;
            p.kf_fights *= MAP_DECAY;
            p.last_stunt = 0.0;
        }
    }

    /// The file: `rcbot-players 1`, then per player `p <rep id>` or `b <profile id>` and its
    /// fields (exposure, damage, hits, kills, deaths, range, kung fu share, stunts, stunt rate,
    /// dodge, kung fu fights, blocks, interval, the 5 attack kinds, the 38 weapons).
    pub fn save(&self) -> String {
        let mut s = store::header(FILE_KIND, FILE_VERSION);
        for (k, p) in &self.players {
            let (kind, id) = match *k {
                OpponentKey::Player(r) => ("p", r),
                OpponentKey::Bot(b) => ("b", b),
            };
            s += &format!("{kind} {id} {}\n", store::join(&p.fields()));
        }
        s
    }

    /// Reads [`save`](Self::save)'s text; other versions and damaged lines are skipped.
    pub fn load(&mut self, text: &str) {
        let Some((FILE_VERSION, lines)) = store::read_header(text, FILE_KIND) else { return };
        for line in lines {
            let mut f = line.split_whitespace();
            let key = match (f.next(), f.next().and_then(|v| v.parse::<i32>().ok())) {
                (Some("p"), Some(id)) => OpponentKey::Player(id),
                (Some("b"), Some(id)) => OpponentKey::Bot(id),
                _ => continue,
            };
            if let Some(p) = store::floats(f).and_then(|v| OpponentProfile::from_fields(&v)) {
                self.players.insert(key, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_by_reputation_and_profile() {
        let mut k = PlayerKeys::default();
        let mut reps = vec![crate::bot::NO_CLIENT; 5];
        reps[1] = 7;
        reps[2] = 8;
        k.refresh(&reps, [(2, 12)].into_iter());
        assert_eq!(k.get(1), Some(OpponentKey::Player(7)));
        assert_eq!(k.get(2), Some(OpponentKey::Bot(12)));
        assert_eq!(k.get(3), None);
        assert_eq!(k.get(99), None);
    }

    #[test]
    fn a_dangerous_player_is_a_threat() {
        let mut m = OpponentModel::default();
        let (ace, novice) = (OpponentKey::Player(1), OpponentKey::Player(2));
        assert_eq!(m.threat(ace), 1.0);
        for _ in 0..20 {
            m.observe_exposure(ace, 1.0, 0.8);
            m.observe_exposure(novice, 1.0, 0.1);
            m.observe_hit_on_bot(ace, 27, 800.0, 40.0);
        }
        m.observe_hit_on_bot(novice, 0, 40.0, 10.0);
        m.observe_kill_of_bot(ace);
        m.observe_death_to_bot(novice);
        assert!(m.threat(ace) > 1.5 && m.threat(novice) < 0.5, "{} {}", m.threat(ace), m.threat(novice));
        let a = m.profile(ace).unwrap();
        assert_eq!(a.favourite_weapon(), Some((27, 1.0)));
        assert_eq!(a.range, 800.0);
        assert_eq!(a.kungfu_share, 0.0);
        assert_eq!(a.accuracy(), Some(1.0));
        assert!(a.dodge > 0.7);
        assert_eq!(m.profile(novice).unwrap().kungfu_share, 1.0);
    }

    #[test]
    fn stunts_kung_fu_and_the_file() {
        let mut m = OpponentModel::default();
        let k = OpponentKey::Bot(3);
        m.observe_stunt(k, 10.0);
        m.observe_stunt(k, 16.0);
        assert_eq!(m.profile(k).unwrap().stunt_rate, 10.0);
        m.observe_kungfu(k, &[3, 1, 0, 0, 0], 2, Some(0.7));
        let p = m.profile(k).unwrap();
        assert_eq!((p.kf_interval, p.kf_block_rate()), (0.7, Some(2.0 / 6.0)));
        let text = m.save();
        let mut back = OpponentModel::default();
        back.load(&text);
        // The stunt clock isn't saved.
        let mut want = m.clone();
        want.players.get_mut(&k).unwrap().last_stunt = 0.0;
        assert_eq!(back, want);
        back.load("rcbot-players 1\np x 1\nq 1 1\np 4 1 2 3\n");
        assert_eq!(back.players.len(), 1);
        m.new_map();
        assert!((m.profile(k).unwrap().stunts - 2.0 * MAP_DECAY).abs() < 1e-6);
    }
}
