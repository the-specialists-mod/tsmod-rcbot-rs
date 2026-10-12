# TS waypoints

44 RCBot waypoint files (`.rcw`, format version 4) for The Specialists 3.0 maps, copied unchanged
from the TS match-mode server package
[2cwldys/ts_h9_team_elimination](https://github.com/2cwldys/ts_h9_team_elimination)
(`rcbot/waypoints/ts/`, commit `531fc999448c5a653b982271200e66c39219e4e7`), which is licensed
under the GNU GPL version 3.

The files don't record who made them. RCBot 1.5-final shipped "a few waypoints for The
Specialists v3.0" (its changelog), and RCBot's waypointing guide is by Cheeseh; the rest were made
by RCBot players and server admins over the years. Thanks to all of them, and to 2cwldys for
collecting them.

Notes from loading them (see `crates/rcbot/tests/ts_waypoints.rs`):

* `ts_hammertime_d1.rcw` names the map `ts_hammertime2_d1` inside, so RCBot (and this port)
  refuses it on `ts_hammertime_d1`.
* `ahl_museum.rcw` has a deleted waypoint (99, at the origin) that still has paths, which
  upstream's auto-pathing creates (`docs/design.md`, upstream bugs).
* `combat2.rcw` has no waypoints.
* Bytes 6-7 of the header's file type are uninitialised memory from whatever build saved the file;
  the port keeps them when it saves a loaded file, so load + save is byte-exact.
