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
//!   or block it presses, how often it attacks, and whether it faces the bot within reach;
//! * **chooses a stance** for a few seconds at a time, defensive (keep guard distance, wait for
//!   the attack, answer it, then counter) or offensive (close in and attack), from what each
//!   stance has earned it so far, its health against the opponent's and its power;
//! * **reacts** to an attack it sees, after its reaction time, by blocking, ducking, stepping
//!   back or sidestepping, whichever has kept it from being hit best;
//! * **attacks in combos**: short move sequences, scored by the damage they deal less the damage
//!   taken while they run; the worst are replaced by children of the best (crossover and
//!   mutation), so each bot grows its own. Besides the ground moves there are crouch-jump attacks,
//!   a run-up flying kick, and wall jumps (`+alt1` facing a wall with the opponent behind, which
//!   throws the bot backwards at it) into the back kick or, turning in the air, the flying kick;
//! * **remembers** what it learned in its profile's `<id>.rkf` across maps.
//!
//! Unarmed against a gun, the bot doesn't run at it across the map: it takes cover unless the
//! gunman is close enough to reach.

use super::look::make_vectors;
use super::{Bot, BotGlobals, BotWorld, Entity, buttons, entflags};
use crate::nav::{Glass, Monsters};
use crate::weapons::TsWeapon;
use crate::{Random, Vector};

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
const BREED_EVERY: u32 = 5;
/// The longest combo.
const MAX_COMBO_LEN: usize = 5;
/// Exploration: the chance to try a stance or reaction at random.
const EXPLORE: i32 = 12;
/// `"<profile id>.rkf"`, in the profiles folder: what a profile's fighter learned.
pub fn kungfu_file_name(profile_id: i32) -> String {
    format!("{profile_id}.rkf")
}

/// The `.rkf` header.
const FILE_HEADER: &str = "rcbot-kungfu 1";

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

/// A move sequence and what it has earned.
#[derive(Clone, Debug, PartialEq)]
pub struct Combo {
    /// The moves in order.
    pub moves: Vec<Move>,
    /// Average reward (damage dealt less damage taken, per 30 health).
    pub value: f32,
    /// Times it ran to the end.
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
    stance: Option<Stance>,
    stance_until: f32,
    /// Health (mine, the opponent's) when the stance started.
    stance_health: (f32, f32),
    /// The running combo: its index, the next step, when that step may go.
    combo: Option<(usize, usize, f32)>,
    combo_health: (f32, f32),
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
    /// A reaction waiting for the bot's reaction time, and when it goes.
    pending: Option<(f32, bool)>,
    /// The reaction running: which, until when, the health it started with, when it is judged.
    reacting: Option<(Reaction, f32, f32, f32)>,
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

/// The kung fu fighter: what it learned, and the fight now.
#[derive(Clone, Debug)]
pub struct KungFu {
    /// Value of each stance (`Stance as usize`).
    pub stance_value: [f32; 2],
    /// Value of each reaction (`REACTIONS` order): 1 when it avoided the hit.
    pub reaction_value: [f32; 4],
    /// The combo pool.
    pub combos: Vec<Combo>,
    finished: u32,
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

/// The index of the highest score, ties to the first; a random index `EXPLORE`% of the time.
fn choose(scores: &[f32], rng: &mut dyn Random) -> usize {
    if scores.is_empty() {
        return 0;
    }
    if rng.random_long(0, 99) < EXPLORE {
        return rng.random_long(0, scores.len() as i32 - 1) as usize;
    }
    let mut best = 0;
    for (i, &s) in scores.iter().enumerate() {
        if s > scores[best] {
            best = i;
        }
    }
    best
}

/// TS weapon ids (as `PlayerInfo::weapon` gives them, kung fu 0) that shoot.
pub fn is_ranged(game_id: i32) -> bool {
    !matches!(game_id, 0 | 25 | 34 | 35) && game_id != TsWeapon::KungFu.id()
}

impl KungFu {
    /// A fighter that knows only the seed combos.
    pub fn new() -> KungFu {
        KungFu { stance_value: [0.0; 2], reaction_value: [0.0; 4], combos: seed_combos(), finished: 0, fight: Fight::default() }
    }

    /// A new life: forget the fight, keep what was learned.
    pub fn reset_fight(&mut self) {
        self.fight = Fight::default();
    }

    /// `<id>.rkf`: what was learned, as text.
    pub fn save(&self) -> String {
        let mut s = format!("{FILE_HEADER}\nstance {} {}\nreaction", self.stance_value[0], self.stance_value[1]);
        for v in self.reaction_value {
            s += &format!(" {v}");
        }
        s += "\n";
        for c in &self.combos {
            let moves: Vec<String> = c.moves.iter().map(|m| m.code().to_string()).collect();
            s += &format!("combo {} {} {}\n", c.value, c.uses, moves.join(","));
        }
        s
    }

    /// Reads [`save`](Self::save)'s text; anything else (or a damaged line) leaves that part as
    /// it was. A file without combos keeps the seeds.
    pub fn load(&mut self, text: &str) {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(FILE_HEADER) {
            return;
        }
        let num = |s: Option<&str>| s.and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite());
        let mut combos = Vec::new();
        for line in lines {
            let mut f = line.split_whitespace();
            match f.next() {
                Some("stance") => {
                    if let (Some(a), Some(b)) = (num(f.next()), num(f.next())) {
                        self.stance_value = [a, b];
                    }
                }
                Some("reaction") => {
                    let v: Vec<f32> = f.filter_map(|v| num(Some(v))).collect();
                    if v.len() == 4 {
                        self.reaction_value.copy_from_slice(&v);
                    }
                }
                Some("combo") => {
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

    /// A combo finished: score it, and every few combos replace the worst well-tried one with
    /// a child of the best.
    fn combo_done(&mut self, index: usize, reward: f32, rng: &mut dyn Random) {
        let Some(c) = self.combos.get_mut(index) else { return };
        c.uses += 1;
        learn(&mut c.value, reward, if c.uses < 4 { 0.5 } else { 0.25 });
        self.finished += 1;
        if self.finished.is_multiple_of(BREED_EVERY) {
            self.breed(rng);
        }
    }

    /// A new combo from the two best: a crossover, or a mutation of the best (a move changed,
    /// added or taken out).
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
        // The child starts at its parent's value, a little lower, so it gets tried.
        let child = Combo { moves: child, value: best.value - 0.05, uses: 0 };
        if self.combos.len() < MAX_COMBOS {
            self.combos.push(child);
        } else if let Some(&worst) = order.iter().rev().find(|&&i| self.combos[i].uses >= 3) {
            self.combos[worst] = child;
        }
    }
}

impl Bot {
    /// The power now: with kung fu out the game reports its bar as the clip (`KFuPower`, the
    /// port's `PlayerInfo::clip`); otherwise an estimate (TS's bar regenerates at 15 a second up
    /// to 100, a block costs 33).
    fn kf_power(&mut self, now: f32) -> f32 {
        let reported = (self.state.alive && self.state.weapon == 0).then_some(self.state.clip);
        let f = &mut self.kungfu.fight;
        if let Some(bar) = reported {
            f.power = (bar as f32).clamp(0.0, 100.0);
        } else if f.power_time == 0.0 {
            f.power = 100.0;
        } else {
            f.power = (f.power + (now - f.power_time).max(0.0) * POWER_REGEN).min(100.0);
        }
        f.power_time = now;
        f.power
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
        self.used_melee = true;
        // A new opponent, or the fight broke off (the enemy hidden or gone, the bot on another
        // task): start afresh, so nothing of a half-done move (a wall to face) lingers.
        if self.kungfu.fight.opponent != Some(enemy.index) || now - self.kungfu.fight.updated > FIGHT_GAP {
            let opp_buttons = enemy.button;
            self.kungfu.fight = Fight { opponent: Some(enemy.index), opp_buttons, opp_interval: 1.0, ..Fight::default() };
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
        self.kf_judge_reaction(now);

        let mut press = 0;
        let reacting = self.kf_react(enemy, origin, dist, now, rng, &mut press);
        if !reacting {
            self.kf_offence(w, enemy, origin, dist, now, rng, &mut press);
        }
        self.button |= press;
        self.kungfu.fight.last_buttons = press;
    }

    /// Reads the opponent's new presses: an attack starts its threat, a block (`+reload` with
    /// kung fu or a blade) its block window.
    fn kf_watch(&mut self, enemy: &Entity, dist: f32, now: f32, rng: &mut dyn Random) {
        let f = &mut self.kungfu.fight;
        let new = enemy.button & !f.opp_buttons;
        f.opp_buttons = enemy.button;
        let melee = enemy.player.is_some_and(|p| !is_ranged(p.weapon));
        if melee && new & buttons::RELOAD != 0 {
            f.opp_block_until = now + BLOCK_TIME;
        }
        let attacking = new & (buttons::ATTACK | buttons::ATTACK2) != 0;
        if attacking {
            if f.opp_last_attack > 0.0 {
                let gap = (now - f.opp_last_attack).min(3.0);
                learn(&mut f.opp_interval, gap, 0.3);
            }
            f.opp_last_attack = now;
        }
        // Facing the bot within reach and attacking: answer after the reaction time.
        let (fwd, _) = make_vectors(Vector::new(0.0, enemy.v_angle.y, 0.0));
        let to_me = (self.me.origin - enemy.origin).normalize();
        let facing = fwd.x * to_me.x + fwd.y * to_me.y > 0.7;
        if melee && facing && dist < THREAT_RANGE && f.reacting.is_none() && f.pending.is_none() {
            let rt = self.kf_reaction_time();
            let f = &mut self.kungfu.fight;
            if attacking {
                f.pending = Some((now + rt, true));
            } else if f.stance == Some(Stance::Defensive) && f.opp_last_attack > 0.0 && now - f.opp_last_attack > f.opp_interval * 0.8 {
                // Its usual pause is over: the next attack is due. Read it, half the time.
                if rng.random_long(0, 1) == 0 {
                    f.pending = Some((now + rt, false));
                }
            }
        }
    }

    /// Picks a stance for the next 2-4 s from what each earned, the health of both and the
    /// power; scores the stance that ends.
    fn kf_choose_stance(&mut self, enemy: &Entity, now: f32, rng: &mut dyn Random) {
        let me = self.me.health;
        let power = self.kf_power(now);
        let kf = &mut self.kungfu;
        if kf.fight.stance.is_some() && kf.fight.stance_until > now {
            return;
        }
        if let Some(s) = kf.fight.stance {
            let (my_start, opp_start) = kf.fight.stance_health;
            let reward = (((opp_start - enemy.health) - (my_start - me)) / 30.0).clamp(-2.0, 2.0);
            learn(&mut kf.stance_value[s as usize], reward, 0.2);
        }
        let mut def = kf.stance_value[Stance::Defensive as usize];
        let mut off = kf.stance_value[Stance::Offensive as usize];
        if me < enemy.health {
            def += 0.3;
        } else {
            off += 0.2;
        }
        if enemy.health < 35.0 {
            off += 0.3;
        }
        if kf.fight.opp_interval < 0.6 {
            def += 0.2;
        }
        if power < BLOCK_COST + 1.0 {
            off += 0.25;
        }
        let stance = if choose(&[def, off], rng) == 0 { Stance::Defensive } else { Stance::Offensive };
        kf.fight.stance = Some(stance);
        kf.fight.stance_until = now + rng.random_float(2.0, 4.0);
        kf.fight.stance_health = (me, enemy.health);
    }

    /// Scores the reaction whose time is up: did the bot keep its health?
    fn kf_judge_reaction(&mut self, now: f32) {
        let me = self.me.health;
        let f = &mut self.kungfu.fight;
        let Some((r, _, health, judge_at)) = f.reacting else { return };
        if judge_at > now {
            return;
        }
        f.reacting = None;
        let lost = health - me;
        let reward = if lost <= 0.0 { 1.0 } else { -(lost / 30.0).min(2.0) };
        if lost <= 0.0 {
            // It worked: hit back while the opponent recovers.
            f.counter_until = now + 0.6;
        }
        let i = REACTIONS.iter().position(|&x| x == r).unwrap_or(0);
        learn(&mut self.kungfu.reaction_value[i], reward, 0.2);
    }

    /// Runs a due reaction and keeps a running one going. True while reacting.
    fn kf_react(&mut self, enemy: &Entity, origin: Vector, dist: f32, now: f32, rng: &mut dyn Random, press: &mut i32) -> bool {
        if let Some((at, _seen)) = self.kungfu.fight.pending
            && at <= now
        {
            self.kungfu.fight.pending = None;
            // Offensive bots answer only some attacks, and none in the middle of a combo.
            let offensive = self.kungfu.fight.stance == Some(Stance::Offensive);
            let busy = self.kungfu.fight.combo.is_some();
            if dist < THREAT_RANGE && !(offensive && (busy || rng.random_long(0, 99) < 50)) {
                let power = self.kf_power(now);
                let can_block = power >= BLOCK_COST && self.kungfu.fight.recover_until <= now;
                let mut scores = self.kungfu.reaction_value;
                if !can_block {
                    scores[0] = f32::MIN;
                }
                let mut r = REACTIONS[choose(&scores, rng)];
                if r == Reaction::Block && !can_block {
                    r = Reaction::StepBack;
                }
                let until = now + if r == Reaction::Block { BLOCK_TIME } else { 0.4 };
                self.kungfu.fight.reacting = Some((r, until, self.me.health, now + REACTION_EVAL));
                self.kungfu.fight.combo = None;
                self.kungfu.fight.face = None;
                if r == Reaction::Block {
                    self.kungfu.fight.power -= BLOCK_COST;
                    self.kungfu.fight.block_until = until;
                    *press |= buttons::RELOAD;
                }
                self.kungfu.fight.side = if rng.random_long(0, 1) == 0 { 1.0 } else { -1.0 };
            }
        }
        let Some((r, until, _, _)) = self.kungfu.fight.reacting else { return false };
        if until <= now {
            return false;
        }
        let away = (self.me.origin - origin).normalize();
        match r {
            Reaction::Block => self.stop_moving(),
            Reaction::Duck => {
                self.start_duck_time = now;
                self.end_duck_time = until;
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

    /// Stance movement, counter-attacks and combos.
    #[allow(clippy::too_many_arguments)]
    fn kf_offence(&mut self, w: &dyn BotWorld, enemy: &Entity, origin: Vector, dist: f32, now: f32, rng: &mut dyn Random, press: &mut i32) {
        let f = &self.kungfu.fight;
        let stance = f.stance.unwrap_or(Stance::Offensive);
        let countering = f.counter_until > now;
        // The opponent's block is up: attacking into it costs half a second and 33 power.
        let opp_blocking = f.opp_block_until > now;
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
            if attack && dist < Move::FlyKick.range().1 && self.kungfu.fight.recover_until <= now {
                self.kungfu.fight.wall = self.kf_find_wall(w, origin);
                let index = self.kf_pick_combo(dist, rng);
                let first = self.kungfu.combos[index].moves[0];
                let (lo, hi) = first.range();
                if (lo..=hi).contains(&dist) && (!first.needs_wall() || self.kungfu.fight.wall.is_some()) {
                    self.kungfu.fight.combo = Some((index, 0, now));
                    self.kungfu.fight.combo_health = (self.me.health, enemy.health);
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

    /// The combo to run: the best scored among those whose first move fits the distance (and
    /// the wall, for a wall jump).
    fn kf_pick_combo(&self, dist: f32, rng: &mut dyn Random) -> usize {
        let wall = self.kungfu.fight.wall.is_some();
        let scores: Vec<f32> = self
            .kungfu
            .combos
            .iter()
            .map(|c| {
                let first = c.moves[0];
                let (lo, hi) = first.range();
                let fits = (lo..=hi).contains(&dist) && (wall || !first.needs_wall());
                if fits { c.value } else { c.value - 3.0 }
            })
            .collect();
        choose(&scores, rng)
    }

    /// One step of the running combo, when its time comes.
    #[allow(clippy::too_many_arguments)]
    fn kf_run_combo(
        &mut self,
        w: &dyn BotWorld,
        enemy: &Entity,
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
                let (my_start, opp_start) = self.kungfu.fight.combo_health;
                let reward = (((opp_start - enemy.health) - (my_start - self.me.health)) / 30.0).clamp(-2.0, 2.0);
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
                        if tr.fraction < 1.0 && self.kungfu.fight.last_buttons & buttons::ALT1 == 0 {
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
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    #[test]
    fn the_power_is_the_games_bar_when_kung_fu_is_out() {
        let config = crate::config::Config::default();
        let mut rng = XorShiftRandom::new(1);
        let profile = crate::config::BotProfile::init(&config, &mut rng);
        let mut b = Bot::new(1, "kf".into(), profile, &config, &mut rng);
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
        k.stance_value = [0.5, -0.25];
        k.reaction_value = [1.0, 0.0, -0.5, 0.25];
        k.combos[2].value = 1.5;
        k.combos[2].uses = 7;
        let text = k.save();
        let mut l = KungFu::new();
        l.combos.clear();
        l.load(&text);
        assert_eq!(l.stance_value, k.stance_value);
        assert_eq!(l.reaction_value, k.reaction_value);
        assert_eq!(l.combos, k.combos);
        // Anything else leaves it as it was.
        let mut m = KungFu::new();
        m.load("garbage\ncombo 1 1 0");
        assert_eq!(m.combos, seed_combos());
        m.load(&format!("{FILE_HEADER}\ncombo 1 2 0,99\ncombo x 1 0\ncombo 0.5 1 3,4"));
        assert_eq!(m.combos, vec![Combo { moves: vec![Move::Kick, Move::QuickKick], value: 0.5, uses: 1 }]);
    }

    #[test]
    fn combos_breed_from_the_best_and_replace_the_worst() {
        let mut k = KungFu::new();
        let mut rng = XorShiftRandom::new(11);
        for _ in 0..200 {
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
    fn guns_shoot_kung_fu_and_blades_dont() {
        assert!(!is_ranged(0) && !is_ranged(25) && !is_ranged(34) && !is_ranged(35));
        assert!(is_ranged(1) && is_ranged(5) && is_ranged(24));
    }
}
