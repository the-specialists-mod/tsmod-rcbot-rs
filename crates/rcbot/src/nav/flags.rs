//! Waypoint flags (`W_FL_*`, `waypoint.h`). The bits are what `.rcw` files store, so they never
//! change. Several bits have more than one upstream name; the aliases other mods use on the same
//! bit (NS marine buildings, S&I scientists, Sven barneys, TFC points) are left out, the bit
//! itself stays.

/// `W_FL_TEAM`: a team number 0-3 in the low two bits.
pub const TEAM: u32 = (1 << 0) + (1 << 1);
/// `W_FL_TEAM_SPECIFIC`: only bots of the team in [`TEAM`] use this waypoint.
pub const TEAM_SPECIFIC: u32 = 1 << 2;
/// `W_FL_CROUCH`: crouch to reach it.
pub const CROUCH: u32 = 1 << 3;
/// `W_FL_LADDER`: on a ladder.
pub const LADDER: u32 = 1 << 4;
/// `W_FL_LIFT`: a lift button.
pub const LIFT: u32 = 1 << 5;
/// `W_FL_DOOR`: wait for a door to open.
pub const DOOR: u32 = 1 << 6;
/// `W_FL_HEALTH`: health kit or charger.
pub const HEALTH: u32 = 1 << 7;
/// `W_FL_ARMOR`: armour or HEV charger.
pub const ARMOR: u32 = 1 << 8;
/// `W_FL_AMMO`: ammo.
pub const AMMO: u32 = 1 << 9;
/// `W_FL_CHECK_LIFT`: check for a lift here.
pub const CHECK_LIFT: u32 = 1 << 10;
/// `W_FL_IMPORTANT`: an objective (flag, hostage…).
pub const IMPORTANT: u32 = 1 << 11;
/// `W_FL_RESCUE`: an objective's goal (flag return, rescue zone).
pub const RESCUE: u32 = 1 << 12;
/// `W_FL_DEFEND_ZONE`.
pub const DEFEND_ZONE: u32 = 1 << 13;
/// `W_FL_AIMING`: a camping/aiming waypoint (not walked to).
pub const AIMING: u32 = 1 << 14;
/// `W_FL_CROUCHJUMP` (bit 16, "compatible with old rcbot 1.5"; bit 15 is unused).
pub const CROUCHJUMP: u32 = 1 << 16;
/// `W_FL_WAIT_FOR_LIFT`: wait for the lift to come down before approaching.
pub const WAIT_FOR_LIFT: u32 = 1 << 17;
/// `W_FL_STUNT`: TS's name for bit 18 (`bot_menu.cpp`), a dive/stunt waypoint. The same bit is
/// `W_FL_WALL_STICK` (NS) and `W_FL_PAIN` (Op4); see [`WALL_STICK`].
pub const STUNT: u32 = 1 << 18;
/// `W_FL_WALL_STICK`: the generic name of bit 18, which navigation code tests ([`STUNT`] in TS).
pub const WALL_STICK: u32 = 1 << 18;
/// `W_FL_JUMP`: jump to reach it.
pub const JUMP: u32 = 1 << 19;
/// `W_FL_WEAPON`: a weapon lies here.
pub const WEAPON: u32 = 1 << 20;
/// `W_FL_TELEPORT`: a teleporter; taking a path from it costs nothing.
pub const TELEPORT: u32 = 1 << 21;
/// `W_FL_TANK`: a func_tank nearby.
pub const TANK: u32 = 1 << 22;
/// `W_FL_FLY`: grapple/fly waypoint.
pub const FLY: u32 = 1 << 23;
/// `W_FL_STAY_NEAR`.
pub const STAY_NEAR: u32 = 1 << 24;
/// `W_FL_ENDLEVEL`: end of level.
pub const ENDLEVEL: u32 = 1 << 25;
/// `W_FL_OPENS_LATER`: a path that opens later (A* traces it before using it).
pub const OPENS_LATER: u32 = 1 << 26;
/// `W_FL_HUMAN_TOWER`: crouch and wait for a player to jump on.
pub const HUMAN_TOWER: u32 = 1 << 27;
/// `W_FL_UNREACHABLE`: a reference point for visibility only.
pub const UNREACHABLE: u32 = 1 << 28;
/// `W_FL_PUSHABLE`.
pub const PUSHABLE: u32 = 1 << 29;
/// `W_FL_GREN_THROW`: throw a grenade from here.
pub const GREN_THROW: u32 = 1 << 30;
/// `W_FL_DELETED`: a free slot.
pub const DELETED: u32 = 1 << 31;

/// `W_FILE_FL_READ_VISIBILITY`: the header flag that says the visibility table must be rebuilt.
pub const FILE_READ_VISIBILITY: i32 = 1 << 0;

/// Short flag names, bit by bit, for logs and tests (a port addition; the editor's
/// `WaypointPrintInfo` sentences come with the editor). Bits 0-1 are the team number and bit 15 is
/// unused.
pub const NAMES: [(u32, &str); 28] = [
    (TEAM_SPECIFIC, "team specific"),
    (CROUCH, "crouch"),
    (LADDER, "ladder"),
    (LIFT, "lift"),
    (DOOR, "door"),
    (HEALTH, "health"),
    (ARMOR, "armour"),
    (AMMO, "ammo"),
    (CHECK_LIFT, "check lift"),
    (IMPORTANT, "important"),
    (RESCUE, "rescue"),
    (DEFEND_ZONE, "defend zone"),
    (AIMING, "aiming"),
    (CROUCHJUMP, "crouch jump"),
    (WAIT_FOR_LIFT, "wait for lift"),
    (STUNT, "stunt"),
    (JUMP, "jump"),
    (WEAPON, "weapon"),
    (TELEPORT, "teleport"),
    (TANK, "tank"),
    (FLY, "fly"),
    (STAY_NEAR, "stay near"),
    (ENDLEVEL, "end level"),
    (OPENS_LATER, "opens later"),
    (HUMAN_TOWER, "human tower"),
    (UNREACHABLE, "unreachable"),
    (PUSHABLE, "pushable"),
    (GREN_THROW, "grenade throw"),
];

/// `WaypointFlagsOnLadderOrFly`.
pub fn on_ladder_or_fly(flags: u32) -> bool {
    flags & (FLY | LADDER) != 0
}

/// The waypoint is usable by `team` (-1 = any): not team-specific, or for that team. This is the
/// check every upstream search repeats inline.
pub fn team_allows(flags: u32, team: i32) -> bool {
    !(team != -1 && flags & TEAM_SPECIFIC != 0 && (flags & TEAM) as i32 != team)
}
