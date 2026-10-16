//! The kung fu fighter: a port addition with no upstream counterpart (design §4). Upstream's TS
//! bot pressed a random punch or kick within 72 units of its enemy (`NORMAL_ATTACK`'s melee
//! branch); this one reads the opponent and fights with TS 3.0's kung fu as a player would.
//!
//! What it builds on is TS 3.0's `CTSGun::CCombatAttack` and `CCombatBlock` (the port's
//! `ts30::melee`): the move comes from the buttons (a fresh `+attack` is a straight punch, held
//! it is jabs that end in a heavy punch, `+attack2` kicks the same way, `+duck` turns them into
//! a heavy punch or the uppercut kick, `+back` or running backwards into the back kick, both in
//! the air into jump attacks), and `+reload` blocks for half a second at 33 of the 100 power,
//! which comes back at 15 a second. A blocked hit costs its attacker half a second and 33 power.
//!
//! The fighter:
//! * **watches** the opponent: the weapon in hand (a gun, kung fu or a blade), each new attack
//!   or block it presses, the game's close-combat timers when the port reports them (hit and
//!   block windows, when it may strike again), how often it attacks, and whether it faces the bot
//!   within reach;
//! * **chooses a stance** for a few seconds at a time, defensive (keep guard distance, wait for
//!   the attack, answer it, then counter) or offensive (close in and attack), from what each
//!   stance has earned against this kind of opponent (human or bot, aggressive or passive), its
//!   health against the opponent's and its power;
//! * **reacts** to an attack it sees, after its reaction time, by blocking, ducking, stepping
//!   back, sidestepping or not at all, whichever has kept it from being hit best against that
//!   kind of attack (a punch, a kick, a low or flying attack, the back kick);
//! * **attacks in combos**: short move sequences, scored by the damage they deal less the damage
//!   taken while they run, per second; the bot tries them by UCB1, and every 20 combos breeds a
//!   child of the best (crossover and mutation), replacing a well-tried combo that is clearly
//!   worse, so each bot grows its own. Besides the ground moves there are crouch-jump attacks,
//!   a run-up flying kick, and wall jumps (`+alt1` facing a wall with the opponent behind, which
//!   throws the bot backwards at it) into the back kick or, turning in the air, the flying kick;
//! * **remembers** what it learned in its profile's `<id>.rkf` across maps.
//!
//! Every score comes from the combat ledger (the port's damage and kill events between the bot
//! and this opponent only), and whatever is open when the fight ends (a kill +2, a death -2, a
//! broken-off fight what it earned so far) is scored then. Estimates blend this opponent's (for
//! the map), the opponent class's and the profile's, each shrunk toward the next by its count;
//! counts decay by 1% a fight and human fights weigh 2.5 times a bot's.
//!
//! Unarmed against a gun, the bot doesn't run at it across the map: it takes cover unless the
//! gunman is close enough to reach.

use super::look::make_vectors;
use super::{Bot, BotGlobals, BotWorld, Entity, buttons, entflags};
use crate::learn::bandit::{Arm, argmax, lcb, ucb};
use crate::learn::opponents::OpponentKey;
use crate::learn::store;
use crate::nav::{Glass, Monsters};
use crate::weapons::TsWeapon;
use crate::{Random, Vector};
use std::collections::HashMap;

/// Seconds without a fight think after which the fight is over.
const FIGHT_GAP: f32 = 0.5;
/// Distance within which the opponent's kung fu can reach the bot (the moves' reach, 35-55
/// units from the eyes, plus the two hulls' half widths).
const THREAT_RANGE: f32 = 96.0;
/// Where the defensive stance waits: just outside the longest kick.
const GUARD_DISTANCE: f32 = 82.0;
/// How close the bot gets to attack.
const STRIKE_DISTANCE: f32 = 52.0;
/// A gunman nearer than this is rushed even unarmed; farther, the bot takes cover.
const RUSH_GUNMAN: f32 = 220.0;
/// `CCombatBlock`'s power cost and window, and the power's regeneration per second.
const BLOCK_COST: f32 = 33.0;
const BLOCK_TIME: f32 = 0.5;
const POWER_REGEN: f32 = 15.0;
/// How long after a reaction its outcome is judged.
const REACTION_EVAL: f32 = 0.6;
/// The combo pool's size, and how many combos finish between two new ones.
const MAX_COMBOS: usize = 14;
const BREED_EVERY: u32 = 20;
/// The longest combo.
const MAX_COMBO_LEN: usize = 5;
/// A combo is culled only after this many runs, and the best few never are.
const CULL_MIN_USES: u32 = 8;
const PROTECTED_COMBOS: usize = 3;
/// UCB1's exploration weights: combos, then stances and answers.
const UCB_COMBO: f32 = 0.7;
const UCB_CHOICE: f32 = 0.5;
/// How many samples of the wider estimate a narrower one is shrunk toward.
const SHRINK: f32 = 3.0;
/// Counts decay by this a fight, so old lessons fade.
const FIGHT_DECAY: f32 = 0.99;
/// A human fight's lessons weigh this many bot fights'.
const HUMAN_WEIGHT: f32 = 2.5;
/// What a block costs beside its outcome: a third of the power bar.
const BLOCK_PENALTY: f32 = 0.3;
/// Health a reward counts per unit.
const REWARD_HEALTH: f32 = 30.0;
/// An opponent attacking at least this often (s) is aggressive.
const AGGRESSIVE_INTERVAL: f32 = 1.2;
/// `"<key>.rkf"`, in the profiles folder: what a profile's fighter learned (the key is the
/// profile id, or [`store::profile_key`]'s name for a bot without one).
pub fn kungfu_file_name(profile_id: i32) -> String {
    format!("{profile_id}.rkf")
}

/// The `.rkf` kind and version (1 had one value per stance and reaction).
const FILE_KIND: &str = "kungfu";
const FILE_VERSION: u32 = 2;

/// One kung fu move, as the buttons that make it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// A fresh `+attack`: the straight punch.
    Punch,
    /// `+attack` held on: a jab (the fourth in a row is the heavy punch).
    Jab,
    /// `+attack` with a fresh `+duck`: the heavy punch.
    HeavyPunch,
    /// A fresh `+attack2`: a kick.
    Kick,
    /// `+attack2` held on: the quick kicks.
    QuickKick,
    /// `+duck` and `+attack2` on the ground: the uppercut kick.
    Sweep,
    /// `+back` and `+attack2`, stepping back: the back kick (stronger the faster).
    BackKick,
    /// A crouch jump, then `+attack2` in the air.
    JumpKick,
    /// A crouch jump, then `+attack` in the air.
    JumpPunch,
    /// A run at the opponent, a crouch jump, then `+duck` `+attack2` in the air: TS's flying kick
    /// above 270 units/s moving the way the bot looks, else its airborne speed kick.
    FlyKick,
    /// Face a wall with the opponent behind, wall jump off it (`+alt1`, `CheckForWallJump`: a wall
    /// within 48 units ahead), then `+attack2` flying backwards: the running back kick, which
    /// reaches behind the bot, at the wall jump's speed.
    WallKick,
    /// The wall jump, then turn to the opponent in the air and `+duck` `+attack2`: the flying
    /// kick when fast enough.
    WallFlyKick,
}

/// Every move; a move's code in `.rkf` files is its place here, so new ones go at the end.
const MOVES: [Move; 12] = [
    Move::Punch,
    Move::Jab,
    Move::HeavyPunch,
    Move::Kick,
    Move::QuickKick,
    Move::Sweep,
    Move::BackKick,
    Move::JumpKick,
    Move::JumpPunch,
    Move::FlyKick,
    Move::WallKick,
    Move::WallFlyKick,
];

impl Move {
    fn code(self) -> u8 {
        MOVES.iter().position(|&m| m == self).unwrap_or(0) as u8
    }

    fn from_code(c: u8) -> Option<Move> {
        MOVES.get(usize::from(c)).copied()
    }

    /// `CCombatAttack`'s delay for the move: no other attack before it ends.
    fn recovery(self) -> f32 {
        match self {
            Move::Punch => 0.35,
            Move::Jab => 0.25,
            Move::HeavyPunch => 0.65,
            Move::Kick => 0.5,
            Move::QuickKick => 0.3,
            Move::Sweep | Move::BackKick => 1.0,
            Move::JumpKick => 0.75,
            Move::JumpPunch => 0.5,
            Move::FlyKick | Move::WallFlyKick => 1.0,
            Move::WallKick => 1.35,
        }
    }

    /// Distances to the opponent the move starts from.
    fn range(self) -> (f32, f32) {
        match self {
            Move::JumpKick | Move::JumpPunch => (0.0, 110.0),
            Move::BackKick => (0.0, 70.0),
            Move::FlyKick => (90.0, 260.0),
            Move::WallKick | Move::WallFlyKick => (40.0, 200.0),
            _ => (0.0, STRIKE_DISTANCE + 24.0),
        }
    }

    /// Needs a wall behind the bot (away from the opponent) to kick off.
    fn needs_wall(self) -> bool {
        matches!(self, Move::WallKick | Move::WallFlyKick)
    }

    /// The button the move attacks with.
    fn attack_button(self) -> i32 {
        match self {
            Move::Punch | Move::Jab | Move::HeavyPunch | Move::JumpPunch => buttons::ATTACK,
            _ => buttons::ATTACK2,
        }
    }

    /// Needs the attack button released first (the game tells a fresh press from a held one).
    fn fresh(self) -> bool {
        !matches!(self, Move::Jab | Move::QuickKick)
    }

    /// Starts with a jump (or wall jump) and attacks in the air.
    fn airborne(self) -> bool {
        matches!(self, Move::JumpKick | Move::JumpPunch | Move::FlyKick | Move::WallKick | Move::WallFlyKick)
    }
}

/// What the bot does about an attack it sees coming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// `+reload`: `CCombatBlock`.
    Block,
    /// Duck under it.
    Duck,
    /// Step out of reach.
    StepBack,
    /// Step aside.
    SideStep,
}

const REACTIONS: [Reaction; 4] = [Reaction::Block, Reaction::Duck, Reaction::StepBack, Reaction::SideStep];

/// The two stances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    /// Hold guard distance, answer attacks, counter.
    Defensive,
    /// Close in and attack.
    Offensive,
}

/// The answers to an attack: the [`Reaction`]s in order, then not reacting (the baseline every
/// reaction is measured against).
pub const ANSWERS: usize = REACTIONS.len() + 1;
/// [`ANSWERS`]' index of not reacting.
pub const NO_REACTION: usize = REACTIONS.len();

/// What the opponent's attack looks like, by its buttons; answers are learned per kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackKind {
    /// `+attack` standing: punches.
    Punch,
    /// `+attack2` standing: kicks.
    Kick,
    /// With `+duck`: the heavy punch, the uppercut kick.
    Low,
    /// Off the ground: jump and flying attacks.
    Air,
    /// With `+back`: the back kick.
    Back,
}

/// Number of [`AttackKind`]s.
pub const ATTACK_KINDS: usize = 5;

impl AttackKind {
    /// The kind of an attack pressed with `buttons`.
    fn of(buttons: i32, on_ground: bool) -> AttackKind {
        if !on_ground {
            AttackKind::Air
        } else if buttons & buttons::DUCK != 0 {
            AttackKind::Low
        } else if buttons & buttons::BACK != 0 {
            AttackKind::Back
        } else if buttons & buttons::ATTACK2 != 0 {
            AttackKind::Kick
        } else {
            AttackKind::Punch
        }
    }
}

/// Opponent classes the stances are learned for: `human * 2 + aggressive`.
pub const OPP_CLASSES: usize = 4;

fn opp_class(human: bool, interval: f32) -> usize {
    usize::from(human) * 2 + usize::from(interval < AGGRESSIVE_INTERVAL)
}

/// A move sequence and what it has earned.
#[derive(Clone, Debug, PartialEq)]
pub struct Combo {
    /// The moves in order.
    pub moves: Vec<Move>,
    /// Mean reward: damage dealt less damage taken (per 30 health, between the bot and its
    /// opponent only) per second the combo ran.
    pub value: f32,
    /// Times it ran.
    pub uses: u32,
}

impl Combo {
    fn new(moves: &[Move]) -> Combo {
        Combo { moves: moves.to_vec(), value: 0.0, uses: 0 }
    }
}

/// The seed combos: TS's own chains (jabs into the heavy punch, quick kicks into the spin
/// kick) and a few openers.
fn seed_combos() -> Vec<Combo> {
    use Move::*;
    [
        &[Punch, Jab, Jab, Jab][..],
        &[Kick, QuickKick, QuickKick, QuickKick],
        &[Punch, Kick],
        &[Sweep],
        &[HeavyPunch, Kick],
        &[JumpKick, Punch],
        &[BackKick],
        &[Jab, Jab, Sweep],
        &[FlyKick, Punch],
        &[WallKick, Kick],
        &[WallFlyKick],
    ]
    .iter()
    .map(|m| Combo::new(m))
    .collect()
}

/// The fight in progress: none of it is saved.
#[derive(Clone, Debug, Default)]
struct Fight {
    /// The opponent's entity index.
    opponent: Option<u32>,
    /// When the fight last ran: a fight not run for a moment is over.
    updated: f32,
    /// Who the opponent is (for its own tables), and whether it is human.
    opp_key: Option<OpponentKey>,
    opp_human: bool,
    stance: Option<Stance>,
    stance_until: f32,
    /// When the stance started, and the opponent class it was chosen for.
    stance_start: f32,
    stance_class: usize,
    /// The running combo: its index, the next step, when that step may go.
    combo: Option<(usize, usize, f32)>,
    combo_start: f32,
    /// The airborne move's progress: 0 not started, 1 jump pressed, 2 in the air.
    phase: u8,
    /// When the airborne move started.
    phase_time: f32,
    /// Where to look instead of at the opponent (facing the wall for a wall jump).
    face: Option<Vector>,
    /// A wall behind the bot, away from the opponent, close enough to wall jump off.
    wall: Option<Vector>,
    /// What the bot pressed last think (fresh presses need a release first).
    last_buttons: i32,
    /// The opponent's buttons at the last think.
    opp_buttons: i32,
    opp_last_attack: f32,
    /// Moving average of the time between the opponent's attacks.
    opp_interval: f32,
    opp_block_until: f32,
    /// The opponent's last hit window (`CombatState::melee_window`) seen.
    opp_window: f32,
    /// A reaction waiting for the bot's reaction time: when it goes, whether the attack was
    /// seen (else predicted), and its kind.
    pending: Option<(f32, bool, AttackKind)>,
    /// The answer running (or, for not reacting, being judged).
    reacting: Option<Answering>,
    /// The opponent's attacks by kind and blocks this fight (for the opponent model).
    opp_attacks: [u32; ATTACK_KINDS],
    opp_blocks: u32,
    /// When the bot's own block ends and when it may attack again.
    block_until: f32,
    recover_until: f32,
    /// Counter-attack until then (after a reaction kept the bot from being hit).
    counter_until: f32,
    /// The bot's power as it estimates it.
    power: f32,
    power_time: f32,
    side: f32,
}

/// An answer to an attack under way.
#[derive(Clone, Copy, Debug)]
struct Answering {
    /// [`ANSWERS`]' index.
    answer: usize,
    /// When its movement ends.
    until: f32,
    /// When it started, and when its outcome is judged.
    start: f32,
    judge_at: f32,
    /// The attack was seen (a predicted one isn't scored).
    seen: bool,
    kind: AttackKind,
}

/// How a fight ended, for what is still open in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FightEnd {
    /// The bot killed the opponent.
    Kill,
    /// The bot died.
    Death,
    /// Broken off (the opponent hidden, killed by someone else, or another one taken on).
    Broken,
}

/// What the bot learned against one opponent this map (not saved).
#[derive(Clone, Debug, Default)]
struct PerOpponent {
    stances: [Arm; 2],
    reactions: [[Arm; ANSWERS]; ATTACK_KINDS],
}

/// The kung fu fighter: what it learned, and the fight now.
#[derive(Clone, Debug)]
pub struct KungFu {
    /// Each stance's value by opponent class (`[class][Stance as usize]`).
    pub stances: [[Arm; 2]; OPP_CLASSES],
    /// Each stance's value over every opponent: what the classes shrink toward.
    pub stance_all: [Arm; 2],
    /// Each answer's value ([`ANSWERS`] order: 1 when it avoided the hit) by attack kind.
    pub reactions: [[Arm; ANSWERS]; ATTACK_KINDS],
    /// Each answer's value over every kind: what the kinds shrink toward.
    pub reaction_all: [Arm; ANSWERS],
    /// The combo pool.
    pub combos: Vec<Combo>,
    finished: u32,
    opponents: HashMap<OpponentKey, PerOpponent>,
    fight: Fight,
}

impl Default for KungFu {
    fn default() -> Self {
        KungFu::new()
    }
}

/// `value += rate * (reward - value)`.
fn learn(value: &mut f32, reward: f32, rate: f32) {
    *value += rate * (reward - *value);
}

/// `estimate`'s narrower level shrunk toward the wider one: `(n v + m w) / (n + m)`.
fn blend(narrow: Arm, wider: f32) -> f32 {
    narrow.shrunk(wider, SHRINK)
}

/// TS weapon ids (as `PlayerInfo::weapon` gives them, kung fu 0) that shoot.
pub fn is_ranged(game_id: i32) -> bool {
    !matches!(game_id, 0 | 25 | 34 | 35) && game_id != TsWeapon::KungFu.id()
}

impl KungFu {
    /// A fighter that knows only the seed combos.
    pub fn new() -> KungFu {
        KungFu {
            stances: [[Arm::default(); 2]; OPP_CLASSES],
            stance_all: [Arm::default(); 2],
            reactions: [[Arm::default(); ANSWERS]; ATTACK_KINDS],
            reaction_all: [Arm::default(); ANSWERS],
            combos: seed_combos(),
            finished: 0,
            opponents: HashMap::new(),
            fight: Fight::default(),
        }
    }

    /// A new life: forget the fight, keep what was learned.
    pub fn reset_fight(&mut self) {
        self.fight = Fight::default();
    }

    /// The opponent the fight is with, if one is under way.
    pub fn opponent(&self) -> Option<u32> {
        self.fight.opponent
    }

    /// The fight ran within `gap` seconds of `now`.
    pub(crate) fn fighting(&self, now: f32, gap: f32) -> bool {
        self.fight.opponent.is_some() && now - self.fight.updated <= gap
    }

    /// `<id>.rkf`: what was learned, as text (version 2: a value and a count per stance and
    /// opponent class, per answer and attack kind, then the combos).
    pub fn save(&self) -> String {
        let pairs = |arms: &[Arm]| arms.iter().map(|a| format!("{} {}", a.value, a.n)).collect::<Vec<_>>().join(" ");
        let mut s = store::header(FILE_KIND, FILE_VERSION);
        s += &format!("stance all {}\n", pairs(&self.stance_all));
        for (c, arms) in self.stances.iter().enumerate() {
            s += &format!("stance {c} {}\n", pairs(arms));
        }
        s += &format!("reaction all {}\n", pairs(&self.reaction_all));
        for (k, arms) in self.reactions.iter().enumerate() {
            s += &format!("reaction {k} {}\n", pairs(arms));
        }
        for c in &self.combos {
            let moves: Vec<String> = c.moves.iter().map(|m| m.code().to_string()).collect();
            s += &format!("combo {} {} {}\n", c.value, c.uses, moves.join(","));
        }
        s
    }

    /// Reads [`save`](Self::save)'s text, or a version 1 file (one value per stance and per
    /// reaction, taken as the overall values worth 4 samples); anything else (or a damaged line)
    /// leaves that part as it was. A file without combos keeps the seeds.
    pub fn load(&mut self, text: &str) {
        let Some((version, lines)) = store::read_header(text, FILE_KIND) else { return };
        if !(1..=FILE_VERSION).contains(&version) {
            return;
        }
        let arms = |v: &[f32], out: &mut [Arm]| {
            if v.len() == out.len() * 2 {
                for (a, p) in out.iter_mut().zip(v.chunks(2)) {
                    *a = Arm::new(p[0], p[1].max(0.0));
                }
            }
        };
        let mut combos = Vec::new();
        for line in lines {
            let mut f = line.split_whitespace();
            match (f.next(), version) {
                (Some("stance"), 1) => {
                    if let Some(v) = store::floats(f).filter(|v| v.len() == 2) {
                        self.stance_all = [Arm::new(v[0], 4.0), Arm::new(v[1], 4.0)];
                    }
                }
                (Some("reaction"), 1) => {
                    if let Some(v) = store::floats(f).filter(|v| v.len() == REACTIONS.len()) {
                        for (a, &x) in self.reaction_all.iter_mut().zip(&v) {
                            *a = Arm::new(x, 4.0);
                        }
                    }
                }
                (Some("stance"), _) => {
                    let which = f.next();
                    let Some(v) = store::floats(f) else { continue };
                    match which {
                        Some("all") => arms(&v, &mut self.stance_all),
                        Some(c) => {
                            if let Some(t) = c.parse::<usize>().ok().and_then(|c| self.stances.get_mut(c)) {
                                arms(&v, t);
                            }
                        }
                        None => {}
                    }
                }
                (Some("reaction"), _) => {
                    let which = f.next();
                    let Some(v) = store::floats(f) else { continue };
                    match which {
                        Some("all") => arms(&v, &mut self.reaction_all),
                        Some(k) => {
                            if let Some(t) = k.parse::<usize>().ok().and_then(|k| self.reactions.get_mut(k)) {
                                arms(&v, t);
                            }
                        }
                        None => {}
                    }
                }
                (Some("combo"), _) => {
                    let num = |s: Option<&str>| s.and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite());
                    let (Some(value), Some(uses), Some(moves)) = (num(f.next()), f.next().and_then(|u| u.parse().ok()), f.next()) else {
                        continue;
                    };
                    let moves: Option<Vec<Move>> = moves.split(',').map(|c| c.parse().ok().and_then(Move::from_code)).collect();
                    if let Some(moves) = moves.filter(|m| !m.is_empty() && m.len() <= MAX_COMBO_LEN)
                        && combos.len() < MAX_COMBOS
                    {
                        combos.push(Combo { moves, value, uses });
                    }
                }
                _ => {}
            }
        }
        if !combos.is_empty() {
            self.combos = combos;
        }
    }

    /// Runs of every combo together (UCB1's total).
    fn combo_runs(&self) -> f32 {
        self.combos.iter().map(|c| c.uses as f32).sum()
    }

    /// A combo finished with `reward` (per second): its mean moves, and every
    /// [`BREED_EVERY`] combos a child of the best is bred.
    fn combo_done(&mut self, index: usize, reward: f32, rng: &mut dyn Random) {
        let Some(c) = self.combos.get_mut(index) else { return };
        if !reward.is_finite() {
            return;
        }
        c.uses += 1;
        // The mean, but never slower than a 10% step, so a combo that stopped working shows it.
        let rate = (1.0 / c.uses as f32).max(0.1);
        c.value += rate * (reward - c.value);
        self.finished += 1;
        if self.finished.is_multiple_of(BREED_EVERY) {
            self.breed(rng);
        }
    }

    /// A new combo from the two best: a crossover, or a mutation of the best (a move changed,
    /// added or taken out). It starts at its parent's value, untried (UCB1 tries it first). In
    /// a full pool it replaces the combo with the lowest upper bound, if that combo ran at
    /// least [`CULL_MIN_USES`] times, isn't among the best [`PROTECTED_COMBOS`], and its upper
    /// bound is under the best's lower bound; otherwise the child is dropped.
    fn breed(&mut self, rng: &mut dyn Random) {
        if self.combos.is_empty() {
            self.combos = seed_combos();
            return;
        }
        let mut order: Vec<usize> = (0..self.combos.len()).collect();
        order.sort_by(|&a, &b| self.combos[b].value.total_cmp(&self.combos[a].value));
        let best = &self.combos[order[0]];
        let mut child = best.moves.clone();
        if order.len() > 1 && rng.random_long(0, 1) == 0 {
            let other = &self.combos[order[1]].moves;
            let cut_a = rng.random_long(1, child.len() as i32) as usize;
            let cut_b = rng.random_long(0, other.len() as i32 - 1) as usize;
            child.truncate(cut_a);
            child.extend_from_slice(&other[cut_b..]);
        } else {
            let m = MOVES[rng.random_long(0, MOVES.len() as i32 - 1) as usize];
            let at = rng.random_long(0, child.len() as i32 - 1) as usize;
            match rng.random_long(0, 2) {
                0 => child[at] = m,
                1 => child.insert(at, m),
                _ if child.len() > 1 => {
                    child.remove(at);
                }
                _ => child.push(m),
            }
        }
        child.truncate(MAX_COMBO_LEN);
        if self.combos.iter().any(|c| c.moves == child) {
            return;
        }
        let child = Combo { moves: child, value: best.value, uses: 0 };
        if self.combos.len() < MAX_COMBOS {
            self.combos.push(child);
            return;
        }
        let total = self.combo_runs();
        let best_lcb = lcb(best.value, best.uses as f32, total, UCB_COMBO);
        let upper = |c: &Combo| ucb(c.value, c.uses as f32, total, UCB_COMBO);
        let worst = order
            .iter()
            .skip(PROTECTED_COMBOS)
            .copied()
            .filter(|&i| self.combos[i].uses >= CULL_MIN_USES && upper(&self.combos[i]) < best_lcb)
            .min_by(|&a, &b| upper(&self.combos[a]).total_cmp(&upper(&self.combos[b])));
        if let Some(w) = worst {
            self.combos[w] = child;
        }
    }

    /// The weight of a lesson against the current opponent.
    fn lesson_weight(&self) -> f32 {
        if self.fight.opp_human { HUMAN_WEIGHT } else { 1.0 }
    }

    /// Stance `s`'s blended value and its count, for opponent class `class`.
    fn stance_estimate(&self, class: usize, s: usize) -> (f32, f32) {
        let all = self.stance_all[s];
        let cls = self.stances[class][s];
        let mut v = blend(cls, all.value);
        let mut n = cls.n;
        if let Some(o) = self.fight.opp_key.and_then(|k| self.opponents.get(&k)) {
            v = blend(o.stances[s], v);
            n += o.stances[s].n;
        }
        (v, n)
    }

    /// Answer `a`'s blended value and its count, against attack `kind`.
    fn answer_estimate(&self, kind: AttackKind, a: usize) -> (f32, f32) {
        let all = self.reaction_all[a];
        let k = self.reactions[kind as usize][a];
        let mut v = blend(k, all.value);
        let mut n = k.n;
        if let Some(o) = self.fight.opp_key.and_then(|k| self.opponents.get(&k)) {
            let o = o.reactions[kind as usize][a];
            v = blend(o, v);
            n += o.n;
        }
        (v, n)
    }

    /// Scores stance `s` for opponent class `class` at every level.
    fn learn_stance(&mut self, class: usize, s: usize, reward: f32) {
        let w = self.lesson_weight();
        self.stance_all[s].update(reward, w);
        self.stances[class][s].update(reward, w);
        if let Some(k) = self.fight.opp_key {
            self.opponents.entry(k).or_default().stances[s].update(reward, w);
        }
    }

    /// Scores answer `a` to attack `kind` at every level.
    fn learn_answer(&mut self, kind: AttackKind, a: usize, reward: f32) {
        let w = self.lesson_weight();
        self.reaction_all[a].update(reward, w);
        self.reactions[kind as usize][a].update(reward, w);
        if let Some(k) = self.fight.opp_key {
            self.opponents.entry(k).or_default().reactions[kind as usize][a].update(reward, w);
        }
    }

    /// A new fight: every count fades a little.
    fn decay(&mut self) {
        let arms = self.stance_all.iter_mut().chain(self.stances.iter_mut().flatten());
        let arms = arms.chain(self.reaction_all.iter_mut()).chain(self.reactions.iter_mut().flatten());
        for a in arms {
            a.decay(FIGHT_DECAY);
        }
        for o in self.opponents.values_mut() {
            for a in o.stances.iter_mut().chain(o.reactions.iter_mut().flatten()) {
                a.decay(FIGHT_DECAY);
            }
        }
    }
}

impl Bot {
    /// The power now: the game's bar (`CombatState::melee_power`, plugin API v6; with kung fu
    /// out also the clip, `KFuPower`); without either an estimate (TS's bar regenerates at 15 a
    /// second up to 100, a block costs 33).
    fn kf_power(&mut self, now: f32) -> f32 {
        let reported = match self.state.combat {
            Some(c) => Some(c.melee_power),
            None => (self.state.alive && self.state.weapon == 0).then_some(self.state.clip as f32),
        };
        let f = &mut self.kungfu.fight;
        if let Some(bar) = reported {
            f.power = bar.clamp(0.0, 100.0);
        } else if f.power_time == 0.0 {
            f.power = 100.0;
        } else {
            f.power = (f.power + (now - f.power_time).max(0.0) * POWER_REGEN).min(100.0);
        }
        f.power_time = now;
        f.power
    }

    /// The bot may strike now: its last move's recovery is over, and the game's
    /// (`CombatState::next_attack_at`) when it reports it.
    fn kf_ready(&self, now: f32) -> bool {
        self.kungfu.fight.recover_until <= now && self.state.combat.is_none_or(|c| c.next_attack_at <= now)
    }

    /// The reaction time for kung fu: the profile's, shorter for skilled bots (0.08-0.3 s).
    fn kf_reaction_time(&self) -> f32 {
        let skill = self.profile.skill.clamp(0, 100) as f32 / 100.0;
        (self.reaction_time * (1.0 - 0.7 * skill)).max(0.08)
    }

    /// `NORMAL_ATTACK` with kung fu in hand and the enemy in sight.
    pub(crate) fn kungfu_fight(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random, enemy: &Entity, dist: f32) {
        let now = w.time();
        let origin = enemy.entity_origin();
        // No `used_melee`: that breeds the personal genes (jump/duck/strafe in knife fights),
        // which kung fu doesn't use (design §4).
        // A new opponent, or the fight broke off (the enemy hidden or gone, the bot on another
        // task): what is open is scored, and the fight starts afresh, so nothing of a half-done
        // move (a wall to face) lingers.
        if self.kungfu.fight.opponent != Some(enemy.index) || now - self.kungfu.fight.updated > FIGHT_GAP {
            self.kf_settle(g, now, FightEnd::Broken, rng);
            let opp_buttons = enemy.button;
            let opp_key = g.learned.player_keys.get(enemy.index);
            let opp_human = enemy.player.is_some_and(|p| !p.is_bot);
            // Its rhythm from earlier fights (the opponent model), else a second.
            let known = opp_key.and_then(|k| g.learned.opponents.profile(k)).filter(|p| p.kf_fights > 0.0);
            let opp_interval = known.map_or(1.0, |p| p.kf_interval);
            self.kungfu.fight = Fight { opponent: Some(enemy.index), opp_key, opp_human, opp_buttons, opp_interval, ..Fight::default() };
            self.kungfu.decay();
        }
        self.kungfu.fight.updated = now;

        // Against a gun out of reach: a gun of its own with rounds, else cover; but a gunman who
        // looks away or has an empty clip is rushed.
        let opp_weapon = enemy.player.map_or(0, |p| p.weapon);
        let (fwd, _) = make_vectors(Vector::new(0.0, enemy.v_angle.y, 0.0));
        let to_me = (self.me.origin - enemy.origin).normalize();
        let gunman_aims = fwd.x * to_me.x + fwd.y * to_me.y > 0.7 && enemy.player.is_some_and(|p| p.clip > 0);
        if is_ranged(opp_weapon) && dist > RUSH_GUNMAN && gunman_aims {
            self.kungfu.fight.combo = None;
            let best = self.best_weapon(w, Some(enemy)).0;
            if best > 0 && self.weapons.get(best).is_some_and(|wp| !wp.is_melee()) {
                self.switch_weapon(best);
            } else {
                self.run_for_cover(g, w, origin, false, rng);
            }
            return;
        }

        self.kf_watch(enemy, dist, now, rng);
        self.kf_choose_stance(enemy, now, rng);
        self.kf_judge_reaction(now, false);

        let mut press = 0;
        let reacting = self.kf_react(enemy, origin, dist, now, rng, &mut press);
        if !reacting {
            self.kf_offence(w, enemy, origin, dist, now, rng, &mut press);
        }
        self.button |= press;
        self.kungfu.fight.last_buttons = press;
    }

    /// Damage dealt to and taken from the fight's opponent since `since`, from the ledger.
    fn kf_trade(&self, since: f32) -> (f32, f32) {
        let Some(o) = self.kungfu.fight.opponent else { return (0.0, 0.0) };
        let l = &self.learning.ledger;
        (l.dealt_since(since, Some(o)), l.taken_since(since, Some(o)))
    }

    /// The fight ended (or broke off): the open combo, stance and answer are scored by how
    /// (a kill +2, a death -2, else what they earned so far), and what was seen of the opponent
    /// goes to the opponent model. Nothing is scored twice.
    pub(crate) fn kf_settle(&mut self, g: &mut BotGlobals<'_>, now: f32, end: FightEnd, rng: &mut dyn Random) {
        if self.kungfu.fight.opponent.is_none() {
            return;
        }
        if let Some((index, _, _)) = self.kungfu.fight.combo.take() {
            let start = self.kungfu.fight.combo_start;
            let reward = match end {
                FightEnd::Kill => Some(2.0),
                FightEnd::Death => Some(-2.0),
                FightEnd::Broken => (now - start >= 0.25).then(|| self.kf_combo_reward(start, now)),
            };
            if let Some(r) = reward {
                self.kungfu.combo_done(index, r, rng);
            }
        }
        if let Some(st) = self.kungfu.fight.stance.take() {
            let start = self.kungfu.fight.stance_start;
            let reward = match end {
                FightEnd::Kill => Some(2.0),
                FightEnd::Death => Some(-2.0),
                FightEnd::Broken => (now - start >= 0.5).then(|| self.kf_stance_reward(start)),
            };
            if let Some(r) = reward {
                self.kungfu.learn_stance(self.kungfu.fight.stance_class, st as usize, r);
            }
        }
        self.kf_judge_reaction(now, end == FightEnd::Death);
        let f = &self.kungfu.fight;
        if let Some(key) = f.opp_key {
            let interval = (f.opp_last_attack > 0.0).then_some(f.opp_interval);
            g.learned.opponents.observe_kungfu(key, &f.opp_attacks, f.opp_blocks, interval);
        }
        let f = &mut self.kungfu.fight;
        f.opp_attacks = [0; ATTACK_KINDS];
        f.opp_blocks = 0;
        f.pending = None;
    }

    /// A combo's reward: the trade with the opponent while it ran, per 30 health, per second
    /// (at least half a second, so a lucky first hit doesn't explode it).
    fn kf_combo_reward(&self, start: f32, now: f32) -> f32 {
        let (dealt, taken) = self.kf_trade(start);
        ((dealt - taken) / REWARD_HEALTH / (now - start).max(0.5)).clamp(-4.0, 4.0)
    }

    /// A stance's reward: the trade with the opponent while it held, per 30 health.
    fn kf_stance_reward(&self, start: f32) -> f32 {
        let (dealt, taken) = self.kf_trade(start);
        ((dealt - taken) / REWARD_HEALTH).clamp(-2.0, 2.0)
    }

    /// Reads the opponent's new presses: an attack starts its threat (its kind noted), a block
    /// (`+reload` with kung fu or a blade) its block window.
    fn kf_watch(&mut self, enemy: &Entity, dist: f32, now: f32, rng: &mut dyn Random) {
        let f = &mut self.kungfu.fight;
        let new = enemy.button & !f.opp_buttons;
        f.opp_buttons = enemy.button;
        let melee = enemy.player.is_some_and(|p| !is_ranged(p.weapon));
        let timers = enemy.player.and_then(|p| p.combat);
        if melee && new & buttons::RELOAD != 0 {
            f.opp_block_until = now + BLOCK_TIME;
            f.opp_blocks += 1;
        }
        // A new hit window (`CombatState::melee_window`) is an attack under way even without a
        // fresh press (held jabs); the game's timer, not a guess from the buttons.
        let window = timers.map_or(0.0, |c| c.melee_window);
        let new_window = melee && window > now && window != f.opp_window;
        f.opp_window = window;
        let attacking = new & (buttons::ATTACK | buttons::ATTACK2) != 0 || new_window;
        let kind = AttackKind::of(enemy.button, enemy.flags & entflags::ONGROUND != 0);
        if attacking {
            if f.opp_last_attack > 0.0 {
                let gap = (now - f.opp_last_attack).min(3.0);
                learn(&mut f.opp_interval, gap, 0.3);
            }
            f.opp_last_attack = now;
            if melee {
                f.opp_attacks[kind as usize] += 1;
            }
        }
        // Facing the bot within reach and attacking: answer after the reaction time.
        let (fwd, _) = make_vectors(Vector::new(0.0, enemy.v_angle.y, 0.0));
        let to_me = (self.me.origin - enemy.origin).normalize();
        let facing = fwd.x * to_me.x + fwd.y * to_me.y > 0.7;
        if melee && facing && dist < THREAT_RANGE && f.reacting.is_none() && f.pending.is_none() {
            let rt = self.kf_reaction_time();
            let f = &mut self.kungfu.fight;
            if attacking {
                f.pending = Some((now + rt, true, kind));
            } else if f.stance == Some(Stance::Defensive) && f.opp_last_attack > 0.0 && now - f.opp_last_attack > f.opp_interval * 0.8 {
                // Its usual pause is over: the next attack is due. Read it, half the time (a
                // guess, so it isn't scored).
                if rng.random_long(0, 1) == 0 {
                    f.pending = Some((now + rt, false, AttackKind::Punch));
                }
            }
        }
    }

    /// Picks a stance for the next 2-4 s by UCB1 over what each earned against this class of
    /// opponent (and this opponent), nudged by the health of both and the power; scores the
    /// stance that ends by the trade with the opponent while it held.
    fn kf_choose_stance(&mut self, enemy: &Entity, now: f32, rng: &mut dyn Random) {
        let me = self.me.health;
        let power = self.kf_power(now);
        if self.kungfu.fight.stance.is_some() && self.kungfu.fight.stance_until > now {
            return;
        }
        if let Some(s) = self.kungfu.fight.stance {
            let reward = self.kf_stance_reward(self.kungfu.fight.stance_start);
            self.kungfu.learn_stance(self.kungfu.fight.stance_class, s as usize, reward);
        }
        let kf = &self.kungfu;
        let class = opp_class(kf.fight.opp_human, kf.fight.opp_interval);
        let (def, n_def) = kf.stance_estimate(class, Stance::Defensive as usize);
        let (off, n_off) = kf.stance_estimate(class, Stance::Offensive as usize);
        let (mut def_bonus, mut off_bonus) = (0.0, 0.0);
        if me < enemy.health {
            def_bonus += 0.3;
        } else {
            off_bonus += 0.2;
        }
        if enemy.health < 35.0 {
            off_bonus += 0.3;
        }
        if kf.fight.opp_interval < 0.6 {
            def_bonus += 0.2;
        }
        if power < BLOCK_COST + 1.0 {
            off_bonus += 0.25;
        }
        let total = n_def + n_off;
        let scores = [ucb(def, n_def, total, UCB_CHOICE) + def_bonus, ucb(off, n_off, total, UCB_CHOICE) + off_bonus];
        let stance = if argmax(&scores) == Some(1) { Stance::Offensive } else { Stance::Defensive };
        let f = &mut self.kungfu.fight;
        f.stance = Some(stance);
        f.stance_until = now + rng.random_float(2.0, 4.0);
        f.stance_start = now;
        f.stance_class = class;
    }

    /// Scores the answer whose time is up (or, at the bot's death, any open one): did the
    /// opponent hurt the bot meanwhile (from the ledger)? A block also pays its power. Answers
    /// to predicted attacks aren't scored.
    fn kf_judge_reaction(&mut self, now: f32, died: bool) {
        let Some(a) = self.kungfu.fight.reacting else { return };
        if a.judge_at > now && !died {
            return;
        }
        self.kungfu.fight.reacting = None;
        let (_, lost) = self.kf_trade(a.start);
        let avoided = lost <= 0.0 && !died;
        if avoided && a.answer != NO_REACTION {
            // It worked: hit back while the opponent recovers.
            self.kungfu.fight.counter_until = now + 0.6;
        }
        if !a.seen {
            return;
        }
        let mut reward = if died {
            -2.0
        } else if avoided {
            1.0
        } else {
            -(lost / REWARD_HEALTH).min(2.0)
        };
        if a.answer == Reaction::Block as usize {
            reward -= BLOCK_PENALTY;
        }
        self.kungfu.learn_answer(a.kind, a.answer, reward);
    }

    /// Runs a due answer and keeps a running one going. True while reacting.
    fn kf_react(&mut self, enemy: &Entity, origin: Vector, dist: f32, now: f32, rng: &mut dyn Random, press: &mut i32) -> bool {
        if let Some((at, seen, kind)) = self.kungfu.fight.pending
            && at <= now
        {
            self.kungfu.fight.pending = None;
            // An offensive bot in the middle of a combo doesn't answer: that is the baseline.
            let offensive = self.kungfu.fight.stance == Some(Stance::Offensive);
            let busy = self.kungfu.fight.combo.is_some();
            if dist < THREAT_RANGE {
                let power = self.kf_power(now);
                let can_block = power >= BLOCK_COST && self.kungfu.fight.recover_until <= now;
                let answer = if offensive && busy { NO_REACTION } else { self.kf_pick_answer(kind, seen, can_block) };
                let r = REACTIONS.get(answer).copied();
                let until = now
                    + match r {
                        Some(Reaction::Block) => BLOCK_TIME,
                        Some(_) => 0.4,
                        None => 0.0,
                    };
                self.kungfu.fight.reacting = Some(Answering { answer, until, start: now, judge_at: now + REACTION_EVAL, seen, kind });
                if r.is_some() {
                    self.kungfu.fight.combo = None;
                    self.kungfu.fight.face = None;
                }
                if r == Some(Reaction::Block) {
                    self.kungfu.fight.power -= BLOCK_COST;
                    self.kungfu.fight.block_until = until;
                    *press |= buttons::RELOAD;
                }
                self.kungfu.fight.side = if rng.random_long(0, 1) == 0 { 1.0 } else { -1.0 };
            }
        }
        let Some(a) = self.kungfu.fight.reacting else { return false };
        let Some(&r) = REACTIONS.get(a.answer) else { return false };
        if a.until <= now {
            return false;
        }
        let away = (self.me.origin - origin).normalize();
        match r {
            Reaction::Block => self.stop_moving(),
            Reaction::Duck => {
                self.start_duck_time = now;
                self.end_duck_time = a.until;
                self.stop_moving();
            }
            Reaction::StepBack => self.set_move_vector(self.me.origin + away * 96.0),
            Reaction::SideStep => {
                let side = Vector::new(-away.y, away.x, 0.0) * self.kungfu.fight.side;
                self.set_move_vector(self.me.origin + side * 96.0);
            }
        }
        let _ = enemy;
        true
    }

    /// The answer to an attack of `kind`: by UCB1 over the answers' estimates for a seen attack
    /// (not reacting included, so a reaction is used only when it beats taking the attack), the
    /// best estimate for a predicted one. No block without the power for it.
    fn kf_pick_answer(&self, kind: AttackKind, seen: bool, can_block: bool) -> usize {
        let est: Vec<(f32, f32)> = (0..ANSWERS).map(|a| self.kungfu.answer_estimate(kind, a)).collect();
        let total: f32 = est.iter().map(|e| e.1).sum();
        let scores: Vec<f32> = est
            .iter()
            .enumerate()
            .map(|(a, &(v, n))| {
                if a == Reaction::Block as usize && !can_block {
                    f32::NEG_INFINITY
                } else if seen {
                    ucb(v, n, total, UCB_CHOICE)
                } else {
                    v
                }
            })
            .collect();
        argmax(&scores).unwrap_or(NO_REACTION)
    }

    /// Stance movement, counter-attacks and combos.
    #[allow(clippy::too_many_arguments)]
    fn kf_offence(&mut self, w: &dyn BotWorld, enemy: &Entity, origin: Vector, dist: f32, now: f32, rng: &mut dyn Random, press: &mut i32) {
        let f = &self.kungfu.fight;
        let stance = f.stance.unwrap_or(Stance::Offensive);
        let timers = enemy.player.and_then(|p| p.combat);
        // The opponent can't strike before its `next_attack_at` and has no hit coming: it is
        // recovering from a move, the moment to hit back (the game's timers when reported).
        let opp_recovering = timers.is_some_and(|c| c.next_attack_at > now + 0.1 && c.melee_window <= now);
        let countering = f.counter_until > now || opp_recovering;
        // The opponent's block is up: attacking into it costs half a second and 33 power. The
        // game's block window when reported (`block_kungfu`), else a guess from its `+reload`.
        let opp_blocking = timers.map_or(f.opp_block_until > now, |c| c.block_kungfu > now);
        // A passive opponent (no attack for a while) is attacked even from the defensive stance.
        let passive = f.opp_last_attack == 0.0 || now - f.opp_last_attack > 2.0;
        let attack = (stance == Stance::Offensive || countering || passive) && !opp_blocking;

        if self.kungfu.fight.combo.is_none() {
            self.kungfu.fight.face = None;
            let away = (self.me.origin - origin).normalize();
            if attack {
                self.set_move_vector(origin);
            } else if dist < GUARD_DISTANCE - 16.0 {
                self.set_move_vector(self.me.origin + away * 64.0);
            } else if dist > GUARD_DISTANCE + 16.0 {
                self.set_move_vector(origin);
            } else {
                // Circle at guard distance.
                if self.strafe_time < now {
                    self.kungfu.fight.side = if rng.random_long(0, 1) == 0 { 1.0 } else { -1.0 };
                    self.strafe_time = now + rng.random_float(0.4, 0.9);
                    self.strafe_speed = self.max_speed * 0.6 * self.kungfu.fight.side;
                }
                self.stop_moving();
            }
            if attack && dist < Move::FlyKick.range().1 && self.kf_ready(now) {
                self.kungfu.fight.wall = self.kf_find_wall(w, origin);
                if let Some(index) = self.kf_pick_combo(dist) {
                    self.kungfu.fight.combo = Some((index, 0, now));
                    self.kungfu.fight.combo_start = now;
                    self.kungfu.fight.phase = 0;
                    self.kungfu.fight.phase_time = now;
                }
            }
        }
        self.kf_run_combo(w, enemy, origin, dist, now, rng, press);
    }

    /// A wall behind the bot, on the far side from the opponent and near enough to reach, solid
    /// at the feet and at the chest, to wall jump off.
    fn kf_find_wall(&self, w: &dyn BotWorld, opponent: Vector) -> Option<Vector> {
        let mut away = self.me.origin - opponent;
        away.z = 0.0;
        let away = away.normalize();
        let o = self.me.origin;
        let tr = w.trace_line(o, o + away * 72.0, Monsters::Ignore, Glass::DontIgnore, Some(self.index));
        if tr.fraction >= 1.0 {
            return None;
        }
        let up = Vector::new(0.0, 0.0, 24.0);
        let high = w.trace_line(o + up, o + up + away * 80.0, Monsters::Ignore, Glass::DontIgnore, Some(self.index));
        (high.fraction < 1.0).then_some(tr.end_pos)
    }

    /// The combo to run, by UCB1 among those whose first move fits the distance (and the wall,
    /// for a wall jump); `None` when none fits.
    fn kf_pick_combo(&self, dist: f32) -> Option<usize> {
        // A wall jump is a stunt: none while one cools down.
        let wall = self.kungfu.fight.wall.is_some() && self.stunt_ready(self.now);
        let total = self.kungfu.combo_runs();
        let scores: Vec<f32> = self
            .kungfu
            .combos
            .iter()
            .map(|c| {
                let first = c.moves[0];
                let (lo, hi) = first.range();
                let fits = (lo..=hi).contains(&dist) && (wall || !first.needs_wall());
                if fits { ucb(c.value, c.uses as f32, total, UCB_COMBO) } else { f32::NEG_INFINITY }
            })
            .collect();
        argmax(&scores)
    }

    /// One step of the running combo, when its time comes.
    #[allow(clippy::too_many_arguments)]
    fn kf_run_combo(
        &mut self,
        w: &dyn BotWorld,
        _enemy: &Entity,
        origin: Vector,
        dist: f32,
        now: f32,
        rng: &mut dyn Random,
        press: &mut i32,
    ) {
        let Some((index, step, at)) = self.kungfu.fight.combo else { return };
        let Some(combo) = self.kungfu.combos.get(index).cloned() else {
            self.kungfu.fight.combo = None;
            return;
        };
        if step >= combo.moves.len() {
            // Done: judge it once the last hit had its chance.
            self.kungfu.fight.face = None;
            if at <= now {
                let reward = self.kf_combo_reward(self.kungfu.fight.combo_start, now);
                self.kungfu.combo_done(index, reward, rng);
                self.kungfu.fight.combo = None;
            }
            return;
        }
        let m = combo.moves[step];
        if at > now {
            // A held move keeps the button down while it waits; a fresh one lets go.
            if !m.fresh() {
                *press |= m.attack_button() & self.kungfu.fight.last_buttons;
            }
            if !m.airborne() {
                self.kf_close_in(m, origin, dist);
            }
            return;
        }
        let button = m.attack_button();
        if m.airborne() {
            if self.kf_airborne(w, m, origin, dist, now, press) {
                self.kf_next_step(index, step, m, now);
            }
            return;
        }
        self.kf_close_in(m, origin, dist);
        if dist > STRIKE_DISTANCE + 24.0 && m != Move::BackKick {
            // Out of reach: wait for the distance to close.
            return;
        }
        if !self.kf_ready(now) {
            // The game won't take the attack yet (`next_attack_at`): a press now is lost.
            return;
        }
        if m.fresh() && self.kungfu.fight.last_buttons & button != 0 {
            // Let go for a think so the game sees a fresh press.
            return;
        }
        match m {
            Move::HeavyPunch | Move::Sweep => *press |= buttons::DUCK | button,
            Move::BackKick => *press |= buttons::BACK | button,
            _ => *press |= button,
        }
        self.kf_next_step(index, step, m, now);
    }

    /// Keeps in reach while a ground move waits (the back kick steps away instead).
    fn kf_close_in(&mut self, m: Move, origin: Vector, dist: f32) {
        if m == Move::BackKick {
            self.set_move_vector(self.me.origin + (self.me.origin - origin).normalize() * 96.0);
        } else if dist > STRIKE_DISTANCE {
            self.set_move_vector(origin);
        } else {
            self.stop_moving();
        }
    }

    /// The move went (or was given up): the next step after its recovery.
    fn kf_next_step(&mut self, index: usize, step: usize, m: Move, now: f32) {
        let f = &mut self.kungfu.fight;
        f.phase = 0;
        f.phase_time = now;
        f.face = None;
        f.recover_until = now + m.recovery();
        f.combo = Some((index, step + 1, now + m.recovery()));
    }

    /// The jump moves, step by step over the thinks. True once the move is done or given up.
    ///
    /// * Jump kick and punch: a crouch jump (`+jump` `+duck`), then the attack in the air with
    ///   `+duck` held, which a fast enough kick makes the flying kick.
    /// * Fly kick: run at the opponent and crouch jump into it from up to 150 units, then
    ///   `+duck` `+attack2` when in reach.
    /// * Wall kick: face the wall behind (the opponent at the bot's back), walk to it, `+alt1`
    ///   (the wall jump flies the bot backwards, away from the wall), then `+attack2`: the back
    ///   kick, which hits behind. The wall fly kick turns to the opponent in the air instead and
    ///   kicks with `+duck`.
    fn kf_airborne(&mut self, w: &dyn BotWorld, m: Move, origin: Vector, dist: f32, now: f32, press: &mut i32) -> bool {
        let on_ground = self.me.flags & entflags::ONGROUND != 0;
        let button = m.attack_button();
        let since = now - self.kungfu.fight.phase_time;
        match self.kungfu.fight.phase {
            0 => {
                // Getting there.
                if since > 1.5 {
                    return true;
                }
                if m.needs_wall() {
                    let Some(wall) = self.kungfu.fight.wall else { return true };
                    self.kungfu.fight.face = Some(wall + Vector::new(0.0, 0.0, self.me.view_ofs.z));
                    self.set_move_vector(wall);
                    let near = (wall - self.me.origin).length_2d() < 40.0;
                    if near && on_ground && self.dot_from_origin(wall + Vector::new(0.0, 0.0, self.me.view_ofs.z)) > 0.8 {
                        // `CheckForWallJump`'s 48 units ahead, from the origin, along the view.
                        let (fwd, _) = make_vectors(self.v_angle);
                        let tr = w.trace_line(
                            self.me.origin,
                            self.me.origin + fwd * 48.0,
                            Monsters::Ignore,
                            Glass::DontIgnore,
                            Some(self.index),
                        );
                        if tr.fraction < 1.0 && self.kungfu.fight.last_buttons & buttons::ALT1 == 0 && self.stunt_ready(now) {
                            *press |= buttons::ALT1;
                            self.kf_phase(1, now);
                        }
                    }
                    return false;
                }
                self.set_move_vector(origin);
                let jump_from = if m == Move::FlyKick { 150.0 } else { Move::JumpKick.range().1 };
                if on_ground && dist <= jump_from && self.kungfu.fight.last_buttons & buttons::JUMP == 0 {
                    // A fly kick wants a run-up: full speed toward the opponent first.
                    if m == Move::FlyKick && self.velocity_2d < 200.0 && dist > 90.0 {
                        return false;
                    }
                    *press |= buttons::JUMP | buttons::DUCK;
                    self.kf_phase(1, now);
                }
                false
            }
            1 => {
                // Waiting to leave the ground; a jump that never does (a ceiling, the stunt
                // cooling down) is given up.
                if m.needs_wall() {
                    self.kungfu.fight.face = self.kungfu.fight.wall.map(|v| v + Vector::new(0.0, 0.0, self.me.view_ofs.z));
                } else {
                    *press |= buttons::DUCK;
                    self.set_move_vector(origin);
                }
                if !on_ground {
                    self.kf_phase(2, now);
                } else if since > 0.6 {
                    return true;
                }
                false
            }
            _ => {
                if on_ground && since > 0.1 {
                    // Landed without the kick.
                    return true;
                }
                let kick_at = match m {
                    Move::WallKick => {
                        // Keep looking at the wall: flying backwards is what makes the back kick.
                        self.kungfu.fight.face = self.kungfu.fight.wall.map(|v| v + Vector::new(0.0, 0.0, self.me.view_ofs.z));
                        70.0
                    }
                    Move::WallFlyKick => {
                        // Turn round to the opponent, then kick when facing it.
                        self.kungfu.fight.face = None;
                        *press |= buttons::DUCK;
                        if self.dot_from_origin(origin) < 0.7 {
                            return false;
                        }
                        90.0
                    }
                    _ => {
                        *press |= buttons::DUCK;
                        self.set_move_vector(origin);
                        if m == Move::FlyKick { 80.0 } else { 70.0 }
                    }
                };
                // In reach, or coming down: kick now.
                if dist <= kick_at || self.me.velocity.z < -150.0 {
                    *press |= button;
                    return true;
                }
                false
            }
        }
    }

    fn kf_phase(&mut self, phase: u8, now: f32) {
        self.kungfu.fight.phase = phase;
        self.kungfu.fight.phase_time = now;
    }

    /// Where the look code faces instead of the enemy, while a wall jump needs the wall: only
    /// with kung fu in hand, in a fight that ran just now.
    pub(crate) fn kungfu_face(&self, now: f32) -> Option<Vector> {
        let f = &self.kungfu.fight;
        let fighting = f.combo.is_some() && now - f.updated <= FIGHT_GAP && self.current_weapon == Some(TsWeapon::KungFu.id());
        f.face.filter(|_| fighting)
    }
}

#[cfg(test)]
impl Bot {
    /// [`Bot::kf_power`], for the bot's tests.
    pub(crate) fn kungfu_power_for_tests(&mut self, now: f32) -> f32 {
        self.kf_power(now)
    }
}

#[cfg(test)]
impl KungFu {
    /// An answer to an attack is running or waiting for the reaction time.
    pub(crate) fn reacting_for_tests(&self) -> bool {
        self.fight.reacting.is_some_and(|a| a.answer != NO_REACTION) || self.fight.pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;
    use crate::learn::ledger::LedgerEntry;

    fn bot() -> Bot {
        let config = crate::config::Config::default();
        let mut rng = XorShiftRandom::new(1);
        let profile = crate::config::BotProfile::init(&config, &mut rng);
        Bot::new(1, "kf".into(), profile, &config, &mut rng)
    }

    #[test]
    fn the_power_is_the_games_bar_when_kung_fu_is_out() {
        let mut b = bot();
        b.state = super::super::PlayerState { alive: true, weapon: 0, clip: 20, ..Default::default() };
        assert_eq!(b.kf_power(1.0), 20.0);
        // Not reported (a gun in hand): the estimate, full at first, back up 15 a second.
        b.state.weapon = 1;
        b.kungfu.fight.power = 10.0;
        assert_eq!(b.kf_power(2.0), 25.0);
    }

    #[test]
    fn saves_and_loads_what_it_learned() {
        let mut k = KungFu::new();
        k.stance_all = [Arm::new(0.5, 3.0), Arm::new(-0.25, 1.5)];
        k.stances[3][1] = Arm::new(1.25, 7.0);
        k.reaction_all[NO_REACTION] = Arm::new(0.75, 2.0);
        k.reactions[AttackKind::Air as usize][0] = Arm::new(-1.0, 4.5);
        k.combos[2].value = 1.5;
        k.combos[2].uses = 7;
        let text = k.save();
        assert!(text.starts_with("rcbot-kungfu 2\n"));
        let mut l = KungFu::new();
        l.combos.clear();
        l.load(&text);
        assert_eq!((l.stance_all, l.stances), (k.stance_all, k.stances));
        assert_eq!((l.reaction_all, l.reactions), (k.reaction_all, k.reactions));
        assert_eq!(l.combos, k.combos);
        // Anything else leaves it as it was.
        let mut m = KungFu::new();
        m.load("garbage\ncombo 1 1 0");
        assert_eq!(m.combos, seed_combos());
        m.load("rcbot-kungfu 9\ncombo 1 1 0");
        assert_eq!(m.combos, seed_combos());
        m.load("rcbot-kungfu 2\ncombo 1 2 0,99\ncombo x 1 0\ncombo 0.5 1 3,4\nstance 7 1 1 1 1\nreaction all 1 2");
        assert_eq!(m.combos, vec![Combo { moves: vec![Move::Kick, Move::QuickKick], value: 0.5, uses: 1 }]);
        assert_eq!(m.reaction_all, [Arm::default(); ANSWERS]);
        // A version 1 file: its single values are the overall ones, worth 4 samples.
        let mut v1 = KungFu::new();
        v1.load("rcbot-kungfu 1\nstance 0.5 -0.25\nreaction 1 0 -0.5 0.25\ncombo 1.5 7 3,4\n");
        assert_eq!(v1.stance_all, [Arm::new(0.5, 4.0), Arm::new(-0.25, 4.0)]);
        assert_eq!(v1.reaction_all[..NO_REACTION], [Arm::new(1.0, 4.0), Arm::new(0.0, 4.0), Arm::new(-0.5, 4.0), Arm::new(0.25, 4.0)]);
        assert_eq!(v1.reaction_all[NO_REACTION], Arm::default());
        assert_eq!(v1.combos.len(), 1);
    }

    #[test]
    fn combos_breed_from_the_best_and_replace_the_worst() {
        let mut k = KungFu::new();
        let mut rng = XorShiftRandom::new(11);
        for _ in 0..400 {
            // The sweep always works, the rest never.
            let i = rng.random_long(0, k.combos.len() as i32 - 1) as usize;
            let reward = if k.combos[i].moves.contains(&Move::Sweep) { 1.5 } else { -1.0 };
            k.combo_done(i, reward, &mut rng);
        }
        assert_eq!(k.combos.len(), MAX_COMBOS);
        assert!(k.combos.iter().all(|c| !c.moves.is_empty() && c.moves.len() <= MAX_COMBO_LEN));
        let with_sweep = k.combos.iter().filter(|c| c.moves.contains(&Move::Sweep)).count();
        assert!(with_sweep > 2, "{:?}", k.combos);
        // No two the same.
        for (i, a) in k.combos.iter().enumerate() {
            assert!(k.combos[i + 1..].iter().all(|b| b.moves != a.moves));
        }
    }

    #[test]
    fn a_full_pool_keeps_combos_not_yet_proven_worse() {
        let mut k = KungFu::new();
        let mut rng = XorShiftRandom::new(3);
        while k.combos.len() < MAX_COMBOS {
            k.breed(&mut rng);
        }
        // Children start at their parent's value, untried, so UCB1 tries them first.
        assert!(k.combos[seed_combos().len()..].iter().all(|c| c.uses == 0));
        for c in &mut k.combos {
            c.uses = 5;
            c.value = -1.0;
        }
        k.combos[0].value = 2.0;
        let before = k.combos.clone();
        for _ in 0..10 {
            k.breed(&mut rng);
        }
        // Too few runs to cull any.
        assert_eq!(k.combos, before);
        for c in &mut k.combos {
            c.uses = 50;
        }
        k.breed(&mut rng);
        k.breed(&mut rng);
        assert_ne!(k.combos, {
            let mut b = before.clone();
            b.iter_mut().for_each(|c| c.uses = 50);
            b
        });
        // The best is never culled.
        assert_eq!(k.combos[0].value, 2.0);
    }

    #[test]
    fn untried_combos_that_fit_come_first() {
        let mut b = bot();
        b.kungfu.combos = vec![
            Combo { moves: vec![Move::Punch], value: 1.0, uses: 10 },
            Combo { moves: vec![Move::FlyKick], value: 0.0, uses: 0 },
            Combo { moves: vec![Move::Kick], value: 0.5, uses: 0 },
        ];
        // The fly kick doesn't fit at 40 units; the untried kick beats the proven punch.
        assert_eq!(b.kf_pick_combo(40.0), Some(2));
        b.kungfu.combos[2].uses = 10;
        assert_eq!(b.kf_pick_combo(40.0), Some(0));
        assert_eq!(b.kf_pick_combo(1000.0), None);
    }

    #[test]
    fn answers_are_scored_by_the_ledger_and_predicted_ones_not_at_all() {
        let mut b = bot();
        b.kungfu.fight.opponent = Some(2);
        let answering = |answer, seen| Answering { answer, until: 1.4, start: 1.0, judge_at: 1.6, seen, kind: AttackKind::Kick };
        // Predicted: not scored.
        b.kungfu.fight.reacting = Some(answering(Reaction::Duck as usize, false));
        b.kf_judge_reaction(2.0, false);
        assert_eq!(b.kungfu.reaction_all, [Arm::default(); ANSWERS]);
        // Seen, nothing taken from the opponent (a third player's hit doesn't count): avoided,
        // a block pays for its power.
        let hit = |from, amount| LedgerEntry {
            time: 1.2,
            other: from,
            weapon: 0,
            dealt: 0.0,
            taken: amount,
            kill: false,
            death: false,
            range_bin: 0,
            waypoint: -1,
        };
        b.learning.ledger.record(hit(3, 40.0));
        b.kungfu.fight.reacting = Some(answering(Reaction::Block as usize, true));
        b.kf_judge_reaction(2.0, false);
        let block = b.kungfu.reactions[AttackKind::Kick as usize][Reaction::Block as usize];
        assert_eq!(block.value, 1.0 - BLOCK_PENALTY);
        // Not reacting and getting hit by the opponent for 15: -0.5.
        b.learning.ledger.record(hit(2, 15.0));
        b.kungfu.fight.reacting = Some(answering(NO_REACTION, true));
        b.kf_judge_reaction(2.0, false);
        assert_eq!(b.kungfu.reaction_all[NO_REACTION].value, -0.5);
        // Too early: nothing yet; at a death, -2 at once.
        b.kungfu.fight.reacting = Some(answering(Reaction::SideStep as usize, true));
        b.kf_judge_reaction(1.1, false);
        assert!(b.kungfu.fight.reacting.is_some());
        b.kf_judge_reaction(1.1, true);
        assert_eq!(b.kungfu.reaction_all[Reaction::SideStep as usize].value, -2.0);
    }

    #[test]
    fn estimates_blend_the_opponent_into_its_class_and_the_profile() {
        let mut k = KungFu::new();
        k.stance_all[0] = Arm::new(1.0, 10.0);
        // Nothing for the class: the profile's value.
        assert_eq!(k.stance_estimate(2, 0).0, 1.0);
        k.stances[2][0] = Arm::new(-1.0, 3.0);
        assert_eq!(k.stance_estimate(2, 0), (0.0, 3.0));
        k.fight.opp_key = Some(OpponentKey::Player(4));
        k.opponents.entry(OpponentKey::Player(4)).or_default().stances[0] = Arm::new(2.0, 3.0);
        assert_eq!(k.stance_estimate(2, 0), (1.0, 6.0));
        // Human lessons weigh more; every fight fades the counts.
        k.fight.opp_human = true;
        k.learn_stance(2, 0, 1.0);
        assert_eq!(k.stances[2][0].n, 3.0 + HUMAN_WEIGHT);
        k.decay();
        assert_eq!(k.stance_all[0].n, (10.0 + HUMAN_WEIGHT) * FIGHT_DECAY);
    }

    #[test]
    fn attack_kinds_come_from_the_buttons() {
        assert_eq!(AttackKind::of(buttons::ATTACK, true), AttackKind::Punch);
        assert_eq!(AttackKind::of(buttons::ATTACK2, true), AttackKind::Kick);
        assert_eq!(AttackKind::of(buttons::ATTACK2 | buttons::DUCK, true), AttackKind::Low);
        assert_eq!(AttackKind::of(buttons::ATTACK2 | buttons::BACK, true), AttackKind::Back);
        assert_eq!(AttackKind::of(buttons::ATTACK, false), AttackKind::Air);
        assert_eq!(opp_class(true, 0.5), 3);
        assert_eq!(opp_class(false, 2.0), 0);
    }

    #[test]
    fn guns_shoot_kung_fu_and_blades_dont() {
        assert!(!is_ranged(0) && !is_ranged(25) && !is_ranged(34) && !is_ranged(35));
        assert!(is_ranged(1) && is_ranged(5) && is_ranged(24));
    }
}
