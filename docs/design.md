# Design

tsmod-rcbot-rs ports RCBot1 ([APGRoboCop/rcbotold](https://github.com/APGRoboCop/rcbotold),
commit `b9c86c2`, "V 1.51-beta13") to Rust for **The Specialists 3.0 only**. The TS Rust port
([the-specialistsmod-rs](https://github.com/the-specialists-mod/the-specialistsmod-rs)) loads it
as the `tsmod_rcbot` plugin through its plugin API (`ts-plugin-api`), and `ts_bot_backend rcbot`
selects it (the-specialistsmod-rs#104). Tracking issue: tsmod-rcbot-rs#1.

Upstream paths below are relative to `rcbotold/dlls/`, line numbers at `b9c86c2`.

## 1. Architecture

```
crates/rcbot     the bot, #![deny(unsafe_code)], no FFI. Engine queries go through traits:
                 nav::World (traces, point contents) and Random (RANDOM_LONG / RANDOM_FLOAT).
crates/host      the tsmod_rcbot cdylib, the only crate allowed unsafe. A stub until the plugin
                 API for bots lands (#104): then it holds the entry point, wraps the API in safe
                 types, implements World/Random on the engine, and turns the port's structured
                 state and events into calls on the bot.
xtask            build | dist | test | lint | all
data/rcbot       what the plugin ships: bot_weapons.ini (TS section), waypoints/ts/*.rcw
tools/reference  the C++ reference harness: upstream code compiled with stubs (§6)
```

Upstream keeps its state in globals (`waypoints`, `num_waypoints`, `paths`, `WaypointLocations`,
`WaypointVisibility`, `gBotGlobals`, ...). The port turns each group into a value
(`nav::Waypoints`, later the bot manager) that the host owns and passes in. Upstream's `MOD_TS`
branches are kept and every other mod's are removed; where a branch chose between mods
(`gBotGlobals.IsMod(MOD_TS)`), the TS answer is written in with a comment.

The host side makes no assumptions about the plugin API's table layout yet: the API is being
redesigned for bots without backward compatibility (#104).

## 2. C++ → Rust map

### Ported

| Upstream | Rust | Notes |
|---|---|---|
| `waypoint.h` `W_FL_*`, `W_FILE_FL_*`; `WaypointFlagsOnLadderOrFly` | `nav/flags.rs` | TS calls bit 18 `STUNT` (`bot_menu.cpp`); other mods' aliases of shared bits are dropped, the bits stay |
| `waypoint.h` `WAYPOINT`, `bot.h` `PATH`; `waypoint.cpp` `WaypointInit`, `WaypointAddPath`, `WaypointDeletePath` (both), `WaypointFindPath`, `WaypointFindNearest` (both), `WaypointFindNearestGoal`, `WaypointFindRandomGoal` (all 3), `WaypointFindNearestAiming`, `WaypointFindReachable`, `WaypointReachable`, `WaypointAddOrigin` (core), `WaypointAddAiming`, `WaypointDelete` (core), `WaypointDistance`, `WaypointOrigin`, `WaypointFlags` | `nav/waypoints.rs` | The `PATH` linked list is a `Vec<[i16; 4]>` per waypoint: same slot order, hole filling, and node identity (`PathRef`, what `CBotFailedPaths` remembers) |
| `bot_navigate.cpp` `BotNavigate_FindPathFromTo` | `nav/waypoints.rs` `find_path_from_to` | |
| `waypoint.h`/`.cpp` `CWaypointLocations` (`AddWptLocation`, `DeleteWptLocation`, `NearestWaypoint`, `FindNearestInBucket`, `GetCoverWaypoint`, `FindNearestCoverWaypointInBucket`, `FillNearbyWaypoints`); `NearestWaypointToOrigin` | `nav/locations.rs` | Bucket order and "latest added wins ties" kept |
| `waypoint.cpp` `WaypointLoad`, `WaypointSave` (without converter), `WAYPOINT_HDR`, `GRAVEBOT_WAYPOINT`, `ConvertGraveBotFlags` | `nav/file.rs` | Byte-compatible; also loads `HPB_bot` and GraveBot (`[Grave]`/`Crabbed`) files. Choosing the file (`.rcw`, `.wpt`, `maps/<map>.gbw`) is the host's job |
| `waypoint.h`/`.cpp` `CWaypointVisibilityTable` | `nav/visibility.rs` | `.rcv` byte-compatible (checked against miniz) |
| `rc_compress.{h,cpp}` (RCZ container; miniz `mz_compress2` level 9) | `nav/rcz.rs` | `miniz_oxide`; byte-identical output |
| `bot_const.h`/`bot_navigate.cpp` `AStarNode`, `BotNavigate_AStarAlgo`; `bot.h` `CBotFailedPath(s)` | `nav/astar.rs` | Checked against upstream on 720 searches (§6) |
| `bot_weapons.cpp` `CWeaponPresets::ReadPresets`; `bot_weapons.h` `weapon_preset_t`, `GetPreset` | `weapons/presets.rs` | `std::stoi` in `stoi.rs` |
| `bot_weapons.h`/`.cpp` `CWeapon`, `CWeaponPreset`, `CWeapons` | `weapons/mod.rs` `WeaponInfo`, `Weapons` | |
| `bot_weapons.h` `TSWeapon`; `bot_weapons.cpp` `pszTSWeaponModels`; the kung fu remap (`bot_client.cpp`) | `weapons/ts.rs` | |
| `vector.h` `Vector` (`Length` as `pow(…, 0.5)`), `AngleVectors` forward | `math.rs` | |
| `bot_weapons.ini` `[mod_id=8]` | `data/rcbot/bot_weapons.ini` | Upstream's current TS section (newer than the 2cwldys pack's copy) |

### Planned: every RCBot feature a TS server uses

| Upstream | Rust (planned) | #1 step |
|---|---|---|
| Movement in `bot_navigate.cpp`: `BotNavigate_UpdateWaypoint`, `BotNavigate_FindNextWaypoint`, `BotTurnAtWall`, `BotCantMoveForward`, `BotCanDuckUnder`, `BotCheckWallOn{Left,Right}`, `BotNavigate_ScanFOV`, `CheckLift` | `nav/movement.rs` | 4 |
| Waypoint editing: `WaypointAdd` (player flags, item scan), the delete/create-path/remove-path commands, `WaypointPrintInfo`, `WaypointDrawBeam`/`WaypointDrawIndex`/`DrawWaypoints`, the waypoint menu (`bot_menu.cpp`) | `nav/editor.rs` | 4 |
| Autowaypointing (`client.cpp` recording, including the TS stunt jump waypoint; `AutoWaypoint`) | `nav/autowaypoint.rs` | 4 |
| `CBot` (`bot.h`/`bot.cpp`): the think loop, conditions, tasks and schedules (`schedule.h`, `bot_tick.h`) | `bot/` | 4-6 |
| Squads (`CBotSquad`, `bot.h`), reputation (`bot_rep.cpp`), botcam (`botcam.cpp`) | `bot/` | 7 |
| What the bot sees (`bot_visibles.{h,cpp}`), `bot_combat.cpp`, enemy choice, aiming and skill (`bot.cpp`), hearing | `combat/` | 5 |
| `CBotWeapon(s)`, `GetBestWeaponId`, `NeedToReload`, `CanShootPrimary` (`bot_weapons`) | `weapons/bot_weapons.rs` | 5 |
| TS: the loadout GA, the stunt perceptron, kung fu and power-ups, pickups, teamplay and CTB (`bot.cpp` `MOD_TS` branches; `UTIL_makeTSweapon` becomes buying through the host) | `ts/` | 6 |
| Learning: `ga.{h,cpp}`, `bot_ga.{h,cpp}`, `perceptron.{h,cpp}`, `nn.h`/`bot_nn.cpp`, `som.{h,cpp}`, `bits.{h,cpp}`, `gannconst.h`, learned-data files (RCZ) | `learn/` | 6, 7 |
| `rcbot` commands (`bot_commands.cpp`), `CBotGlobals` and its ini files (`globals.cpp`: `bot_config.ini`, `bot_users.ini`, ...), bot profiles (`dll.cpp`), menus (`bot_menu.cpp`), `CClient` (`client.cpp`) | `config/` | 4, 7 |
| MegaHAL (`megahal.cpp`), bot chat (`bot_chat.ini`, `bot_rep_chat.ini`) | `chat/` | 7 |
| `util.cpp` helpers | next to their callers | |

### Dropped

* Code for every mod other than TS (NS, TFC, Sven Co-op, DMC, BG, Bumper Cars, S&I, Wizard Wars,
  FLF, AHL, Op4/CTF, HL Rally, HL DM-only branches), including `bot_start.cpp` (TFC/HPB start-up),
  `bot_ns.h`, the WhichBot (NS) waypoint converter and other mods' weapon tables.
* RCBot's copies of metamod and its plumbing: `metamod.cpp`, `mplugin.cpp`, `mreg.cpp`,
  `mhook.cpp`, `mutil.cpp`, `mlist.cpp`, `mqueue.cpp`, `support_meta.cpp`, `h_export_meta.cpp`,
  `log_meta.cpp`, `conf_meta.cpp`, `commands_meta.cpp`, `engine_api.cpp`, `dllapi.cpp`,
  `sdk_util.cpp`, `linkplug.cpp`, `osdep.cpp`, `reg_support.cpp`, `studioapi.cpp`, `api_info.cpp`,
  `game_support.cpp`, `thread_logparse.cpp`, `vdate.cpp`, `rcbot_meta_api.cpp`.
* The standalone game-DLL wrapper: `h_export.cpp`, `linkfunc.cpp`, `linkent.cpp`, `linkgame.cpp`.
* The engine and game-DLL hooks (`engine.cpp`, the hook parts of `dll.cpp`), replaced by the
  plugin API's callbacks, and `FakeClientCommand`'s `Cmd_Argv` hijack, replaced by a direct
  command call.
* User-message parsing (`bot_client.cpp`, `netmessages.cpp`), replaced by the port supplying
  structured state.
* The HL SDK headers; miniz (→ `miniz_oxide`); `dir.cpp` (→ `std::fs`).

Not ported because upstream's doesn't work: the GraveBot *export* (`CGraveBotConvert` through
`WaypointSave`, §5.9).

## 3. File formats

**`.rcw`** (`nav/file.rs`): a 52-byte header (`"RCBot\0"` + 2 bytes, version 4, flags, count,
32-byte map name), 16 bytes per waypoint (`int flags; float origin[3]`), then for each waypoint a
`short` count and that many `short` path indices; little-endian. `W_FILE_FL_READ_VISIBILITY` in
the flags means the `.rcv` must be rebuilt. The map name must match the map (case-sensitive).

**`.rcv`** (`nav/visibility.rs`, `nav/rcz.rs`): an RCZ container (16-byte header: `"RCZ\0"`,
version 1, uncompressed and compressed sizes; then a zlib stream) holding the first
`ceil(n² / 8)` bytes of the 1024×1024-bit table, bit `from * 1024 + to`. Older raw files (those
bytes alone) still load. See §5.1.

**`bot_weapons.ini`** (`weapons/presets.rs`): documented on the module.

## 4. Deviations

Where the port behaves differently from upstream on purpose. Everything else, quirks included, is
kept: A*'s heuristic, open-list tie-breaking, the "going backwards" skip, the team checks, the
50-candidate random goals, the bucket search order.

* **Waypoints outside ±4096 on load**: upstream marks one deleted at the origin but doesn't count
  it, which misaligns every later index and the path section; the port marks it the same way and
  keeps its slot. Truncated files and more than 1024 waypoints are errors instead of reading
  garbage or overrunning the array.
* **Location buckets**: indices are clamped to the 32×32×32 table (upstream writes out of bounds
  for coordinates ≥ 4096 or ≤ -12288). `delete` removes the index from the bucket it is in
  (upstream zeroes the origin first and looks in the wrong bucket).
* **Aiming-only nearest search** uses the caller's failed list (upstream reads whatever the
  previous search left in a global array).
* **Saving**: bytes 6-7 of the file type keep what the loaded file had (zero for new waypoints);
  upstream writes uninitialised stack bytes there.
* **`WeaponInfo::prim_max_range`/`prim_min_range`** return what their names say (upstream's are
  swapped, §5.11). Nothing calls them yet; the combat port decides where the old behaviour must
  stay.
* **Additions**: `VisibilityTable::rows_in_file` and `work_out_rows`, so the host can rebuild only
  the rows an upstream `.rcv` doesn't hold (§5.1); the file format is unchanged.

## 5. Upstream bugs found

Found while porting; not filed upstream.

1. **The visibility file holds only the first rows** (`CWaypointVisibilityTable::SaveToFile` /
   `ReadFromFile`, `waypoint.cpp:2558`, `:2587`): the table's row stride is `MAX_WAYPOINTS`
   (1024), but the file holds `ceil(n² / 8)` bytes, so only about `n² / 1024` rows survive (24 of
   154 on ts_dojo). After a reload every other waypoint "sees" nothing, which breaks cover
   (`GetCoverWaypoint`) and everything else reading the table, until it is rebuilt.
2. **Waypoints outside ±4096 corrupt the load** (`WaypointLoad`, `waypoint.cpp:605-640`): the bad
   waypoint isn't counted, so `num_waypoints` falls behind the file, the last waypoints are lost
   and the path section is read with the wrong count. Only `> 4096` is checked, not `< -4096`;
   `x == 4096` passes and then indexes bucket 32.
3. **Location buckets out of bounds** (`AddWptLocation`/`DeleteWptLocation`,
   `waypoint.cpp:183-203`): no clamping (see 2; the editor can reach it too).
4. **`WaypointDelete` leaves stale bucket entries** (`waypoint.cpp:1893-1896`): the origin is
   zeroed before `DeleteWptLocation(index, origin)`.
5. **Auto-path links to deleted waypoints** (`WaypointAddOrigin`, `waypoint.cpp:1727-1750`):
   deleted slots, which sit at the origin, aren't skipped, so waypoints added near the map
   origin get paths to and from deleted ones; neither `WaypointFindPath` nor A* skips deleted
   successors. The shipped `ahl_museum.rcw` has 16 such paths (waypoint 99).
6. **The aiming-only nearest search reads stale data** (`FindNearestInBucket`,
   `waypoint.cpp:269`): the global failed-waypoint array is only reset for other searches.
7. **`WaypointAddAiming` doesn't add to the location buckets** (`waypoint.cpp:1777`), so the
   bucket search can't find new aiming waypoints until the next load.
8. **Uninitialised header bytes** (`WaypointSave`, `waypoint.cpp:800-812`): `strcpy` into an
   uninitialised `WAYPOINT_HDR` leaves `filetype[6..8]` (and, with a converter,
   `waypoint_file_flags`) as stack garbage. The shipped files show `\xcc\xcc` (MSVC's debug fill)
   and other values.
9. **The GraveBot export can't be read back** (`CGraveBotConvert`, `waypoint.h:448`, through
   `WaypointSave`): it writes 16-byte `WAYPOINT` records under a `"[Grave]"` header, and the
   loader reads `"[Grave]"` files as 36-byte `GRAVEBOT_WAYPOINT` records.
10. **`W_FL_DYNAMIC_TELEPORTER` is 0** (`waypoint.h:136`): `((1<<22)&(1<<21))`, `&` for `|`.
    Unused.
11. **Weapon preset min/max range swapped** (`CWeaponPreset::PrimMaxRange` returns
    `m_fPrimMinRange` and `PrimMinRange` the max, `bot_weapons.h:700-708`), which
    `CBotWeapon::PrimaryInRange`'s too-near/too-far answer and `CBotWeapon::PrimMinRange` use.
12. **`maxclip` in `bot_weapons.ini` is never used**: it is read into `CWeaponPreset::m_iMaxClip`,
    but reloading uses `CBotWeapon::m_iMaxClip`, learned from the largest clip seen in
    `WeaponInfo` (`bot_client.cpp:482`); until then the bot thinks the gun has no clip and never
    reloads it.
13. **A* return values** (`BotNavigate_AStarAlgo`, `bot_navigate.cpp:575`, `:634`): it returns -2
    ("continue next frame") when the open list runs dry exactly on the last allowed loop, and -1
    ("no path") when the start is the goal.
14. **`WaypointFindPath` stops at a negative index** other than -1 (`waypoint.cpp:1195-1206`)
    instead of skipping it, hiding the rest of the waypoint's paths.
15. **`BotNavigate_FindPathFromTo`'s `iFrom = iIndex`** (`bot_navigate.cpp:1711`) is dead code:
    `WaypointFindPath` only reads the index on its first call.
16. **More than 1024 waypoints in a file** overrun `waypoints[]` on load.

Data: `ts_hammertime_d1.rcw` names its map `ts_hammertime2_d1`, so it never loads on
`ts_hammertime_d1`.

The research for #104 lists RCBot's TS bugs elsewhere (weapon switching through `weaponselect`,
model-based teams, the 2.x `WStatus` kung fu id, free weapons instead of buying). The port fixes
those by going through the port's own code when that code is ported.

## 6. Tests

`cargo xtask test` runs everything on x86_64 and i686 Linux.

* **Unit tests** in each module: path slots and the cursor, the searches, buckets and cover, the
  visibility table and its rows, RCZ, `.rcw` save/load/reject, the GraveBot import, A*'s rules,
  `stoi`, the ini parser's quirks, weapon info, the TS table.
* **`tests/ts_waypoints.rs`**, on all 44 shipped TS files: every file loads and saves back **byte
  for byte**; map names are checked as upstream does; the graphs are sane; and A* on 40 random
  pairs per map agrees with a Dijkstra reference built on the same rules (same reachability;
  found paths are real edges from start to goal; cost over optimum: mean 1.0012, worst 1.096 with
  the bot at the start).
* **`tests/upstream_reference.rs`**, against RCBot itself. `tools/reference/build.sh <rcbotold>`
  lifts `BotNavigate_AStarAlgo`, `AStarNode`, `WaypointFindPath`/`AddPath`/`DeletePath`,
  `BotNavigate_FindPathFromTo` and `CBotFailedPaths` verbatim out of the upstream sources,
  compiles them with stubs for `CBot` and the engine (an empty world, the TS mod) together with
  `rc_compress.cpp` and miniz, and records results into `tests/reference/`. The test replays 720
  searches on 6 maps (bot elsewhere on the map, teams -1/0/1, 150 or 7 expansions per frame) and
  matches upstream's return value, frame count and whole path on every one; the port's `.rcv`
  files for 154 and 791 waypoints are byte-identical to upstream's.
