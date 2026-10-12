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
tools/reference  the C++ reference harnesses: upstream code compiled with stubs (§6)
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
| `gannconst.h` `CGenericHeader`, `LEARNTYPE_*`; `bot.h`/`bot.cpp` `CLearnedHeader`, `LEARNED_FILE_VER`; `bot_const.h` `BOT_VER` | `learn/header.rs` | |
| The `fread`/`feof` behaviour the loaders rely on | `learn/io.rs` `Reader` | Short reads copy what is there and set end-of-file, as glibc's `fread` does |
| `bits.{h,cpp}` `CBits` | `learn/bits.rs` | |
| `ga.{h,cpp}` `IIndividual`, `CPopulation`, `CRouletteSelection`, `CTournamentSelection`, `CGA` (incl. adaptive mutation), `RCBOpenFile`'s names | `learn/ga.rs` | `m_iPopType` is the type parameter; `loadTeam`/`saveTeam` (unused, "not functional") are only the file name |
| `bot_ga.{h,cpp}` `CBotGAValues`, `CIntGAValues`, `CBitsGAValues`, `CCombatFitnessMetrics` | `learn/values.rs` | |
| `perceptron.{h,cpp}` `CPerceptron`, `CSigmoidTransfer` | `learn/perceptron.rs` | The file-path `load`/`save` overloads (unused) are dropped |
| `nn.h`/`bot_nn.cpp` `NNLayer`, `NN`, `NNGATrained`, `CNNTrainSet`, `CCombatNNInputs`, `CCombatNNOutputs` | `learn/nn.rs` | `NN::train` is empty upstream and not ported |
| `som.{h,cpp}` `CSom`, `CSomNeuron` | `learn/som.rs` | Unused upstream ("experimental") |
| `bot.cpp` `setupDataStructures`, `SeedFlyGAValues`, `SeedTSWeaponSelectValues`, `loadLearnedData`, `saveLearnedData`; the learning parts of `SpawnInit` (personal and survival picks), of the death handling (fitness, breeding) and of the TS loadout choice; `globals.cpp` `CBotGlobals::loadLearnedData`/`saveLearnedData` | `learn/data.rs` `BotLearnedData`, `GlobalLearnedData` | NS's `combat0/1.rld` dropped; `bot_enemy_cost_ga.h` is empty upstream |

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
| Using the learning in decisions: `dec_stunt` and the jump/duck/strafe, cover, hurt-origin and follow-enemy perceptrons' inputs (`bot.cpp`), enemy costs from `m_GASurvival` | `bot/`, `ts/` (on `learn/`) | 5, 6 |
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

**Learned data** (`learn/`), little-endian, no padding. Every learning object starts with a
32-byte `CGenericHeader`, the string `"t-<LEARNTYPE>-s-<size>"`; loaders compare the string
(`strcmp`) with the one they expect, and a mismatch consumes the header and nothing else.

* `CBotGAValues`: header (3, values), `u32` count, `f32` values, `f32` fitness.
* `CIntGAValues`: `i32` 4, `i32` value, `f32` fitness (no header).
* `CBitsGAValues`: header (3, bits), a `CBits` (header (6, bits), `u32` bits, the bytes), `f32` fitness.
* `CPopulation`: header (2, the GA's *maximum* size), `u32` count, the individuals. A `CGA` is its
  population then its new population.
* `CPerceptron`: header (1, inputs), `i32` inputs, `f32` bias, `f32` learn rate, `u32` + `f32`s
  last inputs, `u32` + `f32`s weights (bias weight first), `f32` output, 1-byte `trained`.
* `NN`: `u32` layers, header (4, layers), layers; `NNLayer`: `u32` neurons, header (5, neurons),
  perceptrons.
* `<profile>.rld` (profiles folder): a 40-byte `CLearnedHeader` (`char[32]` `"1.51b14"`
  zero-padded, `i32` profile id, `i32` 1), then `m_GASurvival` (16), `dec_attackElectrified`,
  `dec_flapWings`, `dec_runForCover`, `dec_faceHurtOrigin`, `dec_jump`, `dec_duck`, `dec_strafe`,
  `m_pPersonalGAVals` (22), `m_personalGA` (`CGA(5)`, 22), `m_pFlyGA` (`CGA(6)`, 6),
  `m_pFlyGAVals` (6), `dec_followEnemy`, `m_pTSWeaponSelect` (8), `dec_stunt`, `m_pCombatBits`;
  saved in an RCZ container, uncompressed files still load. The header's version must match
  exactly: files from the metamod build (`"1.51b14_mm"`) don't load, as upstream.
* `team<n>.rld` (bot folder, teams 0-3): `CLearnedHeader` with the team number, the team's
  enemy-cost `CGA()` (16 genes). `tsweaps.rld`: the TS loadout `CGA()` (8 genes), no header.
  Both uncompressed.

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
* **Learned data**: the bytes after a `CGenericHeader`'s terminator are zero (upstream writes
  uninitialised stack, like §5.8; loaders ignore them). Counts in a damaged file larger than what
  is left are short reads instead of allocations of that size. Out-of-range gene reads
  (`CBotGAValues::get`, asserts upstream) give 0; the GA breeds nothing from an empty population
  (undefined upstream); crossover of individuals of different sizes swaps what both have.
* **`tsweaps.rld` loads with 8 genes**, what the loadouts have (upstream asks for 16, so nothing
  loads, §5.18). Every other load keeps upstream's sizes, including the ones that never match
  (`m_pCombatBits`, §5.20).
* **A perceptron read on its own** (`CPerceptron(FILE*)`, used by `NN::load`) accepts the
  perceptron header for the input count stored after it, so `NN::load` reads what `NN::save`
  wrote (§5.21).
* **`CSom::m_fLearnRate`** is per map instead of a static shared by all maps (no map exists
  upstream, so nothing changes).
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
17. **The GA's two elite slots hold the same individual** (`CGA::epoch`, `ga.cpp:342-349`): the
    loop asks `getBestIndividual()` twice instead of taking the top two. Kept.
18. **`tsweaps.rld` never loads** (`CBotGlobals::loadLearnedData`, `globals.cpp:1672`): it loads
    with 16 genes per individual, but loadouts have 8 (`SeedTSWeaponSelectValues`,
    `bot.cpp:1378`). Each individual's header mismatches, the rest of the file is misread, and the
    GA fills with empty individuals; a bot that picks one reads genes 0-2 past the end of an empty
    vector (`bot.cpp:3208`, `:3224`). Fixed (§4).
19. **One mismatched object misaligns the rest of a `.rld`** (`CBotGAValues::load`,
    `CPerceptron::load`, `CPopulation::load`): a header mismatch consumes only the header, so
    every later object reads from the wrong place. A bot saved before its first spawn has no
    survival genes (header `t-3-s-0`, loaded with 16), so nothing after them loads. Kept.
20. **`m_pCombatBits` never loads** (`bot.cpp:927`, `:984`): `CIntGAValues::save` writes the check
    `sizeof(int)` (4, `bot_ga.h:259`) and the bot loads with 1. NS only, last in the file; kept.
21. **`NN::load` can't read `NN::save`'s output** (`CPerceptron(FILE*)`, `perceptron.h:85`): the
    empty perceptron checks the header against 0 inputs. Unused upstream; fixed (§4).
22. **`NN::trainOutputs` trains the first layer** (`bot_nn.cpp:122`): the wanted outputs go to the
    neurons in layer order, not to the output layer. Unused; kept.
23. **`CBitsGAValues::crossOver` swaps both sides of the point** (`bot_ga.cpp:330`): the bytes
    after the crossover byte are swapped too, so the children are the parents swapped except for
    the top bits of one byte. Unused; kept.
24. **Uninitialised `CGenericHeader` bytes** (`gannconst.h:63`): only the string is written into
    the 32-byte array; the rest is stack garbage in every learned-data file. Zero in the port.
25. **`CGA(ISelection*)`** (`ga.cpp:237`) never calls `setGA` on its populations (so saving
    dereferences null) and leaves the maximum size at 0. Unused.
26. **`CIntGAValues::crossOver`'s mask is `RANDOM_LONG(0, 32)`** (`bot_ga.h:268`), so only the low
    six bits ever cross. NS only; kept.
27. **`CSom::m_fLearnRate` is a static** (`som.cpp:48`, `:106`) that every map's training shrinks
    and nothing resets. Unused.
28. **`dec_jump`, `dec_duck`, `dec_strafe` are built for 4 inputs** (`bot.cpp:1459-1476`) but get 5
    weights and 5 inputs (`decideJumpDuckStrafe`, `bot.cpp:16892-16909`): running works (sizes
    agree), training would refuse, and their saved headers say 4. Kept (the file format depends
    on it).

Quirk kept, not a bug as far as can be told: `CPerceptron::setWeights(CBotGAValues*, …)`
(`perceptron.cpp:74`) sets the bias weight to the bias (1), so GA-bred perceptrons have a fixed
bias weight.

Data: `ts_hammertime_d1.rcw` names its map `ts_hammertime2_d1`, so it never loads on
`ts_hammertime_d1`.

The research for #104 lists RCBot's TS bugs elsewhere (weapon switching through `weaponselect`,
model-based teams, the 2.x `WStatus` kung fu id, free weapons instead of buying). The port fixes
those by going through the port's own code when that code is ported.

## 6. Tests

`cargo xtask test` runs everything on x86_64 and i686 Linux.

* **Unit tests** in each module: path slots and the cursor, the searches, buckets and cover, the
  visibility table and its rows, RCZ, `.rcw` save/load/reject, the GraveBot import, A*'s rules,
  `stoi`, the ini parser's quirks, weapon info, the TS table; the learning code (headers and
  `fread`/`feof` behaviour, each object's file layout and round trip, mismatched and truncated
  loads, crossover/mutation/selection by scripted draws, the GA's epoch and adaptive rate, the
  delta rule, the NN, the SOM, `.rld` round trips including upstream's misalignment).
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
* **`tests/upstream_learn.rs`**, the learning code against RCBot's: `build.sh` also compiles
  `ga.cpp`, `bot_ga.cpp`, `perceptron.cpp`, `bot_nn.cpp`, `som.cpp` and `bits.cpp` unchanged
  against stub headers (`tools/reference/learn_stubs/`), with `RANDOM_LONG`/`RANDOM_FLOAT`
  replaced by the port's `XorShiftRandom`, and `learn_harness.cpp` records 7 seeded scenarios: GA
  breeding with picks (`CBotGAValues`, `CIntGAValues`), bit-string crossover and mutation,
  perceptron training, a GA-trained network, SOM training, and `tsweaps.rld` saved and reloaded
  upstream's way. The port reproduces every file byte for byte (same draws in the same order,
  same float arithmetic).
