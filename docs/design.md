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
                 nav::World (traces, point contents), bot::BotWorld (entities, PVS, time),
                 Random (RANDOM_LONG / RANDOM_FLOAT) and config::CommandHost (commands' effects).
crates/host      the tsmod_rcbot cdylib, the only crate allowed unsafe: TSRS_PluginLoad, safe
                 wrappers over the plugin API v5 and the engine table (api.rs), the traits on
                 them (world.rs, commands.rs), RCBot's globals in one State, the per-frame bot
                 loop and the editors' display (lib.rs, editor.rs)
xtask            build | dist | test | lint | all | e2e
data/rcbot       what the plugin ships: bot_weapons.ini (TS section), waypoints/ts/*.rcw
tools/reference  the C++ reference harnesses: upstream code compiled with stubs (§6)
```

Upstream keeps its state in globals (`waypoints`, `num_waypoints`, `paths`, `WaypointLocations`,
`WaypointVisibility`, `gBotGlobals`, ...). The port turns each group into a value
(`nav::Waypoints`, later the bot manager) that the host owns and passes in. Upstream's `MOD_TS`
branches are kept and every other mod's are removed; where a branch chose between mods
(`gBotGlobals.IsMod(MOD_TS)`), the TS answer is written in with a comment.

The host uses the port's plugin API v5 (the-specialistsmod-rs#104, #117; `goldsrc-sys` and
`ts-plugin-api` pinned to the port's `db84e1d`, branch `fix/plugin-api`, from a local checkout
until it is merged). It registers the bot backend `rcbot`
(`ts_bot_backend rcbot`), the `rcbot` server command (and handles `rcbot`/`menuselect` in
`client_command`), and the `rcbot_ver` cvar. The plugin's data directory
(`<gamedir>/tsmod/rcbot/`) is RCBot's bot folder: `bot_weapons.ini`, `bot_config.ini`,
`bot_users.ini`, `bot_profiles.ini` and `botprofiles/` (profiles, `.rld` learned data, `.rcr`
reputations, HAL brains and keyword lists), `bot_playerids.ini`, `bot_chat.ini`,
`map_configs/ts/<map>.cfg`, `waypoints/ts/<map>.rcw`/`.rcv`, the teams' and loadout GAs. Each
`start_frame` it runs every bot's [`Bot::frame`] (upstream's `Think` + `RunPlayerMove`) and
passes the usercmd to `run_cmd` and the bot's actions to `bot_command`, `select_weapon`,
`clear_loadout`/`buy` and `kick_bot` (all bots think before any of that runs, so their thinks
share one world that reads each entity, and each player's `player_info`, once a frame; the game
state is read once a frame too); it hears deaths (`player_killed`), damage
(`player_damaged`), sounds (`sound`) and players' `say` (`client_command`), and draws the HUD
menus (`ShowMenu`) and the bot camera with the engine's functions.

The state sits behind a mutex that callbacks only `try_lock`: the server calls back into the
plugin from inside API calls (`create_bot` spawns the bot, `run_cmd` runs its think), and a
callback that finds the state busy is queued and runs before the next frame.

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
| `megahal.{h,cpp}` `HAL_STRING`, `HAL_DICTIONARY` (`HAL_AddWord`, `HAL_SearchDictionary`, `HAL_FindWord`, `HAL_CompareWords`, `HAL_InitializeDictionary`, `HAL_EmptyDictionary`, `HAL_WordExists`, `HAL_DictionariesDiffer`) | `chat/dictionary.rs` | Words are bytes; ordering compares signed `char`s as upstream |
| `megahal.cpp` `HAL_MODEL`, `HAL_TREE` (`HAL_NewModel`, `HAL_UpdateModel`, `HAL_UpdateContext`, `HAL_AddSymbol`, `HAL_FindSymbol(Add)`, `HAL_AddNode`, `HAL_SearchNode`, `HAL_InitializeContext`, `HAL_Learn`), `HAL_MakeWords`, `HAL_BoundaryExists` | `chat/model.rs` | Nodes in an arena (`NodeId`) instead of pointers |
| `megahal.cpp` `HAL_SaveTree`/`LoadTree`, `HAL_SaveDictionary`/`LoadDictionary`, the brain parts of `PrepareHALBrainForPersonality`, `LoadHALBrainForPersonality`, `SaveHALBrainForPersonality`; the file names | `chat/brain.rs` | Byte-compatible with the 32-bit builds; no recursion on load/save |
| `megahal.cpp` `HAL_InitializeList`, `HAL_InitializeSwap`, `HAL_SWAP`, the pretraining loop | `chat/lists.rs` | |
| `megahal.h` `HAL_bot_t`; `megahal.cpp` `BotHALGenerateReply`, `BotHALMakeKeywords`, `BotHALAddKeyword`, `BotHALAddAuxiliaryKeyword`, `BotHALBuildReplyDictionary`, `BotHALBabble`, `BotHALSeedReply` | `chat/reply.rs` `HalBot` | |
| `megahal.cpp` `HumanizeString`, `RemoveNameTags`; `bot.cpp` `BotFunc_FillString`; `util.cpp` `strlow` | `chat/text.rs` | |
| `bot.h` `SBotChatMessages`; `globals.cpp` `CBotGlobals::SetupBotChat` (`bot_chat.ini`); `bot.cpp` `CBot::BotChat`, `BotFunc_GetRandomPlayerName`, `BotFunc_GetRepArrayNum`; `megahal.cpp` `BotChatReply` and `CBot::ReplyToMessage`'s use of it | `chat/bot_chat.rs` | Players, names and the map come through `ChatContext`; config flags as plain values |
| `bot_const.h` `BOT_CONFIG_*`, `BOT_DEBUG_*`; the configurable part of `CBotGlobals` (`ChangeConfigSetting`, `IsConfigSettingOn`, defaults of the constructor/`setDefaults`); `util.cpp` `UTIL_BuildFileName` | `config/mod.rs` `Config` | NS-only values left out |
| `globals.cpp` `ReadBotFolder` (`rcbot_folder.ini`), `ReadConfig`'s tokenizer (`bot_config.ini`) | `config/file.rs` | Running the lines: `commands::run_config` |
| `bot.h` `CAllowedPlayer(s)`; `dll.cpp` `ReadBotUsersConfig` (`bot_users.ini`) | `config/users.rs` | |
| `bot.h`/`client.cpp` the access, menu, waypoint-display, debug and teleport state of `CClient`/`CClients` (`ClientConnected`'s authentication, `m_bRecheckAuth`) | `config/client.rs` | The rest of `CClient` comes with its users |
| `bot.h` `bot_profile_t`; `dll.cpp` `BotFunc_InitProfile`, `BotFunc_ReadProfile`, `BotFunc_WriteProfile`; `bot_profiles.ini` and `BotFunc_AddBot`'s profile choice | `config/profile.rs` | |
| `bot_rep.cpp` (`CBotReputations` load/save, `GetPlayerRepId`, `GetClientRep`), `bot.h` `CBotReputation(s)`; `bot.cpp` `BotFunc_GetRepArrayNum`, `BotFunc_IncreaseRep`/`DecreaseRep`, `printRep(s)` | `config/rep.rs` | Mod-independent upstream, so TS keeps it |
| `bot_menu.{h,cpp}` `SetupMenus` (TS menus), `CBotMenu`, `CBotMenuItem` (`Render`, `Activate`) | `config/menu.rs` | NS's add/kick-to-team menus dropped |
| `bot_commands.{h,cpp}` `CBotCvar`, `CBotCvarList`, `SetupCommands`, every command class and help text, `BotFunc_AddBot`'s arguments and TS team (`mp_teamlist`) choice, `RCBot_ServerCommand`; `dll.cpp` `ClientCommand`'s `rcbot`/`menuselect` dispatch; `globals.cpp` `ReadConfig`'s dispatch; `bot_menu.cpp` `BotMenu_Func_*` | `config/commands.rs` | Side effects through `CommandHost` (`Effect`, `Print`). NS `config` settings, `util buildres/buildcomm`, the WhichBot/GraveBot `waypoint save` converters dropped; `±rcbot_force_grip`'s access check here, its grip in the host |
| `bot.h` `CBotTask`, `CBotTasks` (schedules, front insertion, path hand-over) | `bot/tasks.rs` | Only the tasks a TS bot creates |
| `bot.h` `CRememberPosition(s)` | `bot/remember.rs` | |
| `bot.cpp` `CBot::Think`, `SpawnInit`, `DoTasks` (`FIND_PATH`, `NORMAL_ATTACK` (TS), `SEARCH_FOR_ENEMY`, `RELOAD`, `WAIT_AND_FACE_VECTOR`, `FACE_VECTOR`, `MOVE_TO_VECTOR`, `PICKUP_ITEM`, `USE`, `WAIT`, `DEFEND`, `LISTEN_TO_SOUND`, `CHANGE_WEAPON`, `DROP_WEAPON`, `CROUCH`), `LookForNewTasks` (TS), `SwitchWeapon`, `CanPickup`; `util.cpp` `UTIL_UpdateSounds`, `UTIL_IsFacingEntity`; `globals.cpp` `StartFrame`'s bot loop (kick, kill, think, move) | `bot/think.rs`, `bot/manager.rs` | `Bot::frame` |
| `bot_navigate.cpp` `BotNavigate_UpdateWaypoint`, `BotNavigate_FindNextWaypoint`, `BotNavigate_ScanFOV`, `BotCheckWallOn{Left,Right}`; `bot.cpp` `touchedWpt`, `gotStuck`, `GetLadderDir`, `GetNextWaypoint`, `GetClimbType` | `bot/navigate.rs` | |
| `bot.cpp` `WorkMoveDirection`, `CheckStuck`, `RunPlayerMove`, `UpdateMsec`; `util.cpp` `UTIL_GetAvoidAngle` | `bot/movement.rs` | |
| `bot.cpp` `WorkViewAngles`, `SetViewAngles`, `ChangeAngles` (Whistler's), `GetAimVector`; `util.cpp` angle helpers | `bot/look.rs` | |
| `bot.cpp` `UpdateVisibles`, `FVisible`, `CanAvoid`, `FindEnemy`, `IsEnemy` (TS), `EnemyFound`, `workEnemyCosts`, `clearEnemyCosts`, `decideJumpDuckStrafe`, `WantToFollowEnemy`, `RunForCover`, `UpdateConditions`; `bot_visibles.cpp` `CBotVisibles`; `BotFunc_BreakableIsEnemy` | `bot/combat.rs` | |
| `bot_weapons.{h,cpp}` `CBotWeapon`, `CBotWeapons`, `GetBestWeaponId`, `NeedToReload`, `CanShootPrimary`; `bot_client.cpp` `BotClient_TS_WeaponInfo` | `bot/weapons.rs` | The port's `PlayerInfo` weapon fields stand in for the message |
| `waypoint.cpp` `WaypointAddOrigin` (player flags, item scan), `WaypointAdd`, `WaypointDelete`, `WaypointCreatePath`, `WaypointRemovePath`, `WaypointDrawIndex`, `CWaypointLocations::DrawWaypoints`; `client.cpp` `CClient::Think`'s waypoint display and auto-waypointing (jumps, TS stunt jumps, ladders, distance, corners); `util.cpp` `UTIL_CanStand` | `nav/editor.rs` | Beams, sounds and messages come back as `EditorEvent`s |
| `bot_navigate.cpp` `CheckLift`; `bot.cpp` `BotFunc_FindNearestButton`, `CMasterEntity::FindButton`, `CBot::Touch`, `PlayerStandingOnMe`, `StandingOnPlayer`, `BotFunc_EntityIsMoving`, the tasks `PUSH_PUSHABLE`, `WAIT_FOR_LIFT`, `WAIT_FOR_ENTITY`, `HUMAN_TOWER`, `THROW_GRENADE`, `WAIT_FOR_BOT_AT_WPT`, `FACE_EDICT`, `BOT_TASK_USE`'s momentary buttons; `BotNavigate_UpdateWaypoint`'s human-tower touch, pushable, grenade, lift and wait-for-bots parts; `util.cpp` `UTIL_FindNearestEntity`, `UTIL_IsButton`, the pushable helpers; `waypoint.cpp` `PlayerNearVector` | `bot/special.rs` | |
| `bot.h` `CBotSquad`, `CBotSquads`; `bot.cpp` `AddSquadMember`, `ChangeLeader`, `RemoveSquad`, `GetFormationVector`, `ReturnAllToFormation`, `BotFunc_MakeSquad`, `AddToSquad`, `CanAddToSquad`, `RemoveMySquad`, `BotSquadLeaderThink`, `SayInPosition`, `BOT_TASK_FOLLOW_LEADER`, the squad parts of `Think`, `UpdateConditions`, `UpdateVisibles`, `EnemyFound` | `bot/squad.rs` | Squads by id; what upstream did to other bots is queued (`SquadOrder`) |
| `bot.cpp` `CBot::BotEvent` and `BotEvent_*`, `HearSound`, `BotChat`'s first half, `ReplyToMessage`, `BOT_TASK_TYPE_MESSAGE`, `havingFun`, `gettingBored`, `printBoredoms`, `WantToLeaveGame`; `engine.cpp` `pfnEmitSound`'s sound kinds | `bot/events.rs` | |
| `bot_client.cpp` `BotClient_Generic_DeathMessage`'s events; `engine.cpp` `pfnEmitSound`'s loop; `dll.cpp` `DispatchTouch`, `ClientCommand`'s `say`; `globals.cpp` `StartFrame`'s leave-the-game part | `bot/manager.rs` | |
| `bot.h` `CBotCam`; `dll.cpp` `CBotCam::Think`, `TuneIn`, `TuneOff`, `Spawn`, `Clear` | `bot/botcam.rs`, `host` | |
| (none: a port addition) the kung fu fighter | `bot/kungfu.rs` | §4 |
| `bot.cpp` `FindEnemy`'s reputation priority, `EnemyFound`'s TS part (`usepowerup`), Think's grenade cover (`UTIL_IsGrenadeRocket`, `dll.cpp` `BotFunc_TraceToss`), idle chat | `bot/combat.rs`, `bot/think.rs` | |
| `dll.cpp`/`engine.cpp` hooks, `RCBot_ServerCommand`, `ClientCommand`'s dispatch, `BotFunc_AddBot`'s rest (profile choice, greeting), `BotFunc_ReadProfile`'s brain and reputations, `ClientDisconnect`'s saving, `ServerDeactivate`'s `saveLearnedData`, `CClients::ClientConnected`/`ClientDisconnected` (reputation ids, `AddLoadRep`, `RemoveSaveRep`), `megahal.cpp` `PrepareHALBrainForPersonality`/`LoadHALBrainForPersonality`/`SaveHALBrainForPersonality`'s files, `globals.cpp` `MapInit`'s map config and `NON_TS_TEAMPLAY` (`tm_` maps), `ReadMapConfig`, `StartFrame`'s rejoin (`min_bots`/`max_bots`, leave and join), `SetupBotChat`'s file, `WaypointLoad`'s file choice, `WaypointSave`'s files, `UTIL_ShowMenu`, `UTIL_PlaySound`, `WaypointDrawBeam`, `ClientCommand`'s `±rcbot_force_grip` with `CClient::Think`'s grip, `UTIL_FacingEnt`, `UTIL_getEntityInFront` | `host` | Plugin callbacks and API calls |
| `bot.cpp` `LookForNewTasks`' TS objectives (`m_TSObjectives`: a random one, team 1 `DEFEND`s it, team 2 `USE`s its `ts_hack`/`ts_bomb` for 9 s); `bot_client.cpp` `BotClient_TS_Objective`, `BotClient_Generic_TeamScore` (`m_iTeamScores`) | `bot/think.rs` (`objective_tasks`), `host` | The plugin API's `objectives` and `GameState::team_scores` stand in for the messages (§4) |
| `util.cpp` `UTIL_makeTSweapon`, `ExplosionCreate`'s live branch (`TE_EXPLOSION`), `RadiusDamage`; `bot_commands.cpp` `util explo` | `host/src/commands.rs` | `create_entity`/`entity_key_value`/`spawn_entity` and `ham_execute(Use, TakeDamage)` |

### Waiting for the port

| Upstream | Why | Where |
|---|---|---|
| (none: plugin API v4, the-specialistsmod-rs#117, added the TS objectives, the team scores and entities made from keys) | | |

Upstream RCBot has no capture-the-briefcase code for TS (its `m_bHasFlag` logic is TFC's and
Opposing Force's), so CTB maps are played as teamplay maps: end-level waypoints.

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

**HAL brain** (`chat/brain.rs`), `botprofiles/<id>_hal.brn`, as the 32-bit builds write it
(`unsigned long` is 4 bytes): `"RCBOTHAL\0"`, `u8` order (5), the forward then the backward tree
depth first (each node `u16` symbol, `u32` usage, `u16` count, `u16` children, then the children),
the dictionary (`u32` size, then per word `u8` length and the bytes). A file without the cookie is
replaced by an empty brain that is then pretrained from `<id>_hal.trn`. The keyword lists
(`.ban`, `.aux`: one word per line; `.swp`: `from<TAB>to`) and the training text are text files
read with `fgets`. **`bot_chat.ini`**: `[section]` lines (`thanks`, `greetings`, `kills`,
`killed`, `laugh`, `leave` each `-friendly`/`-neutral`/`-unfriendly`, plus `[idle]`, `[help]`,
matched exactly) followed by chat lines with `%n`, `%r`, `%g`, `%b`, `%e`, `%m`; `#` comments in
the first column.

**Config files** (`config/`), parsed as upstream parses them (each module documents its quirks):
`rcbot_folder.ini` (one word: the bot folder), `bot_config.ini` (`rcbot` commands, one per line:
command and up to four space-separated arguments), `bot_users.ini` (`"name" password level
[steamid]`), `bot_profiles.ini` (profile ids), `botprofiles/<id>.ini` (`key=value`, strings
quoted), `bot_playerids.ini` (`"name"` per line; the line number is the reputation id) and
`botprofiles/<id>.rcr` (8-byte records `i32 player rep id, i32 rep`, little-endian).

## 4. Deviations

Where the port behaves differently from upstream on purpose. Everything else, quirks included, is
kept: A*'s heuristic, open-list tie-breaking, the "going backwards" skip, the team checks, the
50-candidate random goals (`find_random_goal`, which the bots no longer use for roaming), the
bucket search order.

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
* **Chat text files** (`bot_chat.ini`, the HAL lists and training text) read `"\r\n"` as `"\n"` on
  every platform, as the Windows build does (the Linux build keeps the `'\r'` in section names and
  words, so Windows-saved files don't work there).
* **MegaHAL crashes and hangs avoided**: a `.swp` line without a second field is skipped (upstream
  passes `NULL` to `strlen`, §5.33); `BotFunc_FillString` with a replacement that contains what it
  replaces searches on after it (upstream loops forever); `BotHALBabble` stops after one round when
  every count is 0 (a damaged brain; upstream spins); `m_Laugh[-1]` (outside the struct upstream,
  §5.35) has no lines; symbols past a damaged dictionary read as empty words; a truncated brain
  is an error and leaves the model as it was; trees are loaded and saved without recursion.
* **Config files** read `"\r\n"` as `"\n"` too (`bot_config.ini`, `bot_users.ini`, profiles), and
  fields that overrun their fixed buffers upstream (config tokens, `bot_users.ini` fields,
  profile strings, 64-byte player names) are cut to them.
* **Commands that crash upstream without a client** (§5.39-41) do nothing or name the caller
  "Server" instead; `debug` from `bot_config.ini` on a listen server counts as the server.
* **`mp_teamlist` teams** are cut at 63 bytes (upstream overruns its 64-byte `newteam`, §5.45);
  the cvar is read and split as bytes, as upstream's `char` copies (#3).
* **Missing data**: an empty `mp_teamlist` gives no TS team (upstream picks from an empty array);
  `showreps` skips reputations of players who left (upstream dereferences `NULL`).
* **Additions**: `VisibilityTable::rows_in_file` and `work_out_rows`, so the host can rebuild only
  the rows an upstream `.rcv` doesn't hold (§5.1); the file format is unchanged.

* **The bot through the port** (the-specialistsmod-rs#104): `pfnRunPlayerMove` is the API's
  `run_cmd`, `FakeClientCommand` is `bot_command`, `ClientKill` is the `kill` command, the kick is
  `kick_bot`, and the weapon switch is `select_weapon` (upstream's `CmdStart` with `weaponselect`,
  which TS 3.0 ignores, so RCBot never switched). Teams are the port's rules team
  (`PlayerInfo::team`; upstream compared `model` keys, which TS 3.0's teams don't use), except on
  a `tm_` map the game doesn't run as teamplay (upstream's `NON_TS_TEAMPLAY`, which forces
  `m_bTeamPlay`): there every team is 0, so the bots compare `model` keys as upstream does. What the
  TS user messages told the bot comes from `PlayerInfo`: `WeaponInfo` when the weapon, clip or
  reserve changes (the weapon in hand is the current weapon every frame), `TSState` as the stunt
  state; the fire mode, attachments, free slots, power-ups, time scale and permanent kung fu are
  in `PlayerState` too. Damage comes from `player_damaged` (the bot
  faces the attacker); upstream guessed from a health drop and looked 128 units behind itself.
* **View angles**: the usercmd carries `v_angle`; upstream passes `pev->angles` (§5.47). The
  engine copies a usercmd's angles into `pev->v_angle` only while `pev->fixangle` is 0
  (`SV_RunCmd`), and clears `fixangle` only when it sends a client `svc_setangle`/`svc_addangle`,
  which it never does for fake clients; TS sets `fixangle` at every spawn, so the bots' views
  stayed frozen where they spawned (they never turned or aimed). Upstream wrote `pev->v_angle`
  itself (`ChangeAngles`). The host now does for each bot, before it thinks, what the engine does
  for a client (`take_fixed_angles`): the game's `angles` (or the `avelocity` yaw for an add)
  become the view and `fixangle` is cleared. `cargo xtask e2e` checks a bot turns (`turned`).
* **A TS gun's max clip is the largest clip seen** (`checkMaxClip`'s intent; upstream resets it
  on every `WeaponInfo`, §5.49), so an empty gun needs a reload and a fighting bot reloads or
  switches as upstream's TS code says. `CanReload`/`LowOnAmmo` stay false (`m_iAmmo1`).
* **The force grip** holds the entity in front of the player's view (upstream: along a stale
  `gpGlobals->v_forward`, §5.71), and ends when either player leaves.
* **Kung fu is 38** everywhere (`TSWeapon::KungFu`); upstream also uses 36, TS 3.0's Contender
  (§5.48).
* **The loadout**: `UTIL_makeTSweapon` spawned the three GA-picked weapons for free at once; the
  port empties the loadout (`clear_loadout`) and buys them (`buy`, ids 1-37 but 29) for the next
  spawn, under the server's rules. So the pick is made while the bot is dead, before its
  `respawn` (upstream: at the first think of the life): the weapons a life uses are the ones its
  death grades, as upstream, and a new bot's first life has them too. A bot the port hands over
  alive picks at its first think, for its next life. Upstream's death-time `buy` /
  `menuselect 7` ("doesn't work") is dropped. With `BOT_CONFIG_TS_KUNGFU` nothing is bought, as
  upstream.
* **Visibility rows** an `.rcv` doesn't hold (or all of them, without one) are worked out when
  the map's waypoints load (§5.1).
* **What the port can't tell a plugin**, worked around: touches (`DispatchTouch`) are the bot's
  box meeting another entity's (expanded by one unit) each frame, which is what the engine does
  for triggers; the masters list (`master` key values from `DispatchKeyValue`) is empty, so
  `BotFunc_FindNearestButton` goes straight to the entities targeting the door; a client's
  `no_touch` can't stop the game's touch; a renamed player keeps the old reputation id
  (upstream copied it in `SetClientKeyValue`). The port's entity hooks (`ham_set_hook`) would
  give touches and key values, but there is one hook per server, which the AMX Mod X host owns.
* **The bot camera** is an `info_target` made with `CREATE_NAMED_ENTITY`, without
  `DispatchSpawn`/`DispatchThink` (a plugin has no game-DLL table; `info_target` does nothing in
  either), moved by writing its `entvars` as upstream does.
* **Teams in events**: `BotEvent_Died` and the death message's teammate tests read the port's
  rules team (upstream read `pev->team`, which TS 3.0 leaves 0).
* **Every client gets a `CClient`**, bots included, as upstream's `ClientPutInServer` does (the
  port's step 4 skipped bots), so bots have reputations of each other.
* **Bots without a profile**: with no `bot_profiles.ini`, or none free, upstream adds no bot; the
  port has to (its quota asks), so the bot gets `BotFunc_InitProfile`'s defaults with id 0, a
  MegaHAL brain in memory (pretrained from `botprofiles0_hal.trn` if there is one) and no files
  of its own (brain, `.rld`, `.rcr`), which would be shared by every such bot.
* **Bots kept across a level change**: upstream's `RESPAWN_NEED_TO_REJOIN` bots rejoin through
  `BotFunc_AddBot`; the port keeps its bots and hands them back (`adopt_bot`), so they get the
  profile they had again (saved at `ServerDeactivate`, as upstream saves at `ClientDisconnect`).
  `GameShutdown` saves too, in case the engine doesn't deactivate the server first.
* **The e2e log lines**: `kill "<killer>" killed "<victim>"` for a kill by an RCBot bot, and
  `weapon "<bot>" changed <a> -> <b>` for a living bot's weapon changing, are the port's
  additions (`cargo xtask e2e` checks them; the port logs no kills).
* **`util` toys**: `givetsweapon` makes upstream's `ts_groundweapon` from keys through the plugin
  API (`create_entity`, `entity_key_value`, `spawn_entity`) and has the player use it
  (`ham_execute(Ham_Use)`). `explo` draws the beam, sends `TE_EXPLOSION` and runs RCBot's
  `RadiusDamage` with `TakeDamage` through `ham_execute` (`BodyTarget` as the port's classes have
  it: the centre, a player's raised by `view_ofs` × 0.5..1.1); upstream's `UTIL_BotScreenShake` is
  left out, as a plugin can't look up the port's `ScreenShake` message id without registering it.
  `search` prints each entity's index, classname and distance (upstream's NS and TFC fields are
  dropped); `hudtext`, `send_sound` and `teleport_ent` are upstream's.
* **The HUD menus** are one `ShowMenu` message each, as upstream's `UTIL_ShowMenu`.
* **Grenades**: `UTIL_IsGrenadeRocket` is "classname contains `grenade` or `rpg_rocket`", as
  upstream; the enemy costs use it too.
* **Bot names**: the port's quota names its bots; `rcbot addbot` and nameless `addbot` use
  RCBot's default name (`RCBot`). `addcustombot`'s skill (0-1) becomes the profile's skill
  (0-100).

* **TS objectives**: upstream collects the `TSObjective` messages a bot gets into
  `m_TSObjectives` and clears them at `SpawnInit` unless one came since the last weapon pick
  (`BOT_CONDITION_DONT_CLEAR_OBJECTIVES`). The port asks the server for the HUD's list as the bot
  reads it (`objectives`, plugin API v5) each time, so the list is always the current one, and
  keeps only the objectives that can still be completed (`ObjectiveInfo::active`: not frozen,
  a giver that hasn't fired), which the messages can't tell.
* **No shots through walls**: `SEE_ENEMY` can outlive the line of sight (set by
  `NORMAL_ATTACK`'s last-seen-position check, by `EnemyFound`, or kept until the vision pass
  reaches the enemy again), and upstream then aimed and fired at the enemy's *current* origin
  behind the wall. `NORMAL_ATTACK` re-checks `FVisible` on the enemy every time before acting on
  the condition, and `FindEnemy` checks it on each candidate instead of trusting the visible
  list.
* **Reloads**: `Reload` presses the key only for a gun whose clip has room and whose reserve
  has rounds, at most once per 2 s or until the weapon or clip changes (TS keeps a started
  reload going, so one press is enough; with kung fu or a blade the key is `CCombatBlock`, which
  upstream's reload task held down for up to 1.5 s). `BOT_TASK_RELOAD` ends at once with nothing
  to reload.
* **Empty guns**: a gun with no rounds in the clip or reserve is dropped (`drop`), forgotten,
  and the next weapon with rounds (else kung fu) taken out; upstream kept dry-firing it. Kung fu's
  own empty `WeaponInfo` no longer sends `drop`.
* **Kung fu** (`bot/kungfu.rs`, no upstream counterpart): `NORMAL_ATTACK`'s random punches and
  kicks within 72 units are replaced by a fighter built on TS 3.0's `CCombatAttack` and
  `CCombatBlock`. It reads the opponent's weapon and new presses (`pev->button`: attacks, blocks),
  picks a defensive or offensive stance every 2-4 s from what each earned (damage dealt less
  taken), its health, the opponent's and its power (the game's bar, which the port reports as the clip
  while kung fu is out; else estimated: 100, -33 a block, +15/s); answers
  an attack it sees, after a skill-scaled reaction time, by blocking (`+reload`), ducking,
  stepping back or aside, scored by whether it was hit, then counters; and attacks in combos
  (move sequences pressed as TS reads them: fresh or held `+attack`/`+attack2`, with `+duck`,
  `+back`, crouch jumps (`+jump` `+duck`), a run-up flying kick, and wall jumps (`+alt1` facing a
  wall with the opponent behind) into the back kick or, turning in the air, the flying kick).
  Combos are scored the same way; every 5 the worst well-tried one is replaced by a crossover or
  mutation of the best. What it learned is saved per profile in `botprofiles/<id>.rkf` (text:
  `rcbot-kungfu 1`, `stance`, `reaction`, `combo <value> <uses> <move codes>`), beside the
  byte-compatible `.rld`. While a wall jump turns its back to the opponent, the bot keeps it seen
  if the line is clear. Unarmed against a gun more than 220 units away it takes out a gun with
  rounds or takes cover instead of running at it.
* **Crouch jumps**: the low-obstacle hop in `ScanFOV` is `JumpAndDuck` (upstream: `Jump`), as
  the stuck and jump-waypoint jumps already are.
* **Handles on players** (`MyEHandle`, `EntRef`) use the player's user id
  (`GETPLAYERUSERID`) instead of the edict's serial number, which a client slot keeps for the next
  player in it: a player joining in a slot someone left doesn't inherit bots' enemy, task,
  squad and remembered-position handles on the one before.
* **Teleports**: when the game sets a bot's view (`fixangle` 1: a spawn or a teleport) or the bot
  moves more than 256 units in a frame, it drops its current and previous waypoints, its path
  and its stuck timers (`Bot::teleported`), and starts again from its nearest waypoint;
  upstream went on towards the old waypoint, got stuck and failed the path.
* **Sounds** (`sound` callback): a bot hears a sound where the game played it (the callback's
  origin; upstream's `pfnEmitSound` hook took the entity's centre, which for the world is the
  middle of the map), not when it is silent (volume 0), and only the kinds it reacts to
  (`player/…`) are looked at; the host drops the rest before taking the state lock.
* **Weapons the bot owns** come from the game (`player_inventory`, `BotWorld::inventory`) at the
  first frame of a life, when the weapon in hand changes, after a drop, after a refused switch
  and every second: guns it never held are known with their rounds (upstream knew only weapons
  it had held, TS sending no `WeaponList`), and weapons it lost are forgotten. A new life starts
  with no guns but kung fu (upstream kept the last life's). The current weapon is the one the
  game reports in hand; `SwitchWeapon` asks for a switch (once, until the game shows it or a
  second passes) instead of setting it, and a switch the game refuses (`select_weapon`'s
  `INVALID`: not owned) makes the bot forget that weapon.
* **Usercmd `msec`** (`movement.rs` `next_msec`): rounded instead of truncated, with the
  remainder carried to the next command, and sent from 9.5 ms (w00tguy's 10, less what `f32`
  game time loses: a 100 Hz frame often measures 9.99 ms). The clock starts one frame back for a
  new bot or map and keeps running through deaths. Upstream lost up to 1 ms a command (bots
  simulated 3-5% less time than passed), at 100 Hz skipped every other frame (half speed), and
  sent 255 ms as the first command and again after each spawn (`SpawnInit` reset the clock).
* **Team scores**: `m_iTeamScores` comes from `GameState::team_scores` every frame, by rules team
  (the bot's `team`); see §5 for upstream's indexing.

Navigation, movement and tactics (#1, phase 3: bots that move like experienced TS players):

* **Stuck recovery** (`movement.rs` `Unstick`): upstream's `CheckStuck` ran `gotStuck` on every
  frame between 1 and 2 s of being stuck, each time one waypoint *index* back (§5.52), then failed
  whatever path that left, and killed the bot after 8 s without progress, in a fight or not. The
  port reacts once per stuck event: back to the previous waypoint (the one it was heading for
  goes back on the path after it); at 2 s the path it was on when it got stuck fails and the bot
  finds another way from the nearest waypoint but that one. Stuck again at the same place within
  4 s escalates: backing up and strafing, a crouch jump, then a wall jump when facing a wall or
  else a dive. Suicide is the last resort: 20 s at one place (or without progress), never with an
  enemy in sight in the last 5 s, counted (`Bot::stuck_suicides`, `stuck_kills` in the host's
  log). Being stuck compares the speed with the whole speed asked for, sideways included (a bot
  pre-aiming walks partly with the strafe; upstream compared the forward speed).
* **Roaming** (`roam_goal`, `Waypoints::find_weighted_goal`): upstream's random goal is one of
  the first 50 usable waypoints (`int indexes[50]`), 17-37% of the shipped maps; the port draws
  from all of them, weighted by how long the bot hasn't been there (`goals.rs` `Explore`, up to
  2 min) and less for goals within 400 units. A goal reached stays off the roaming list until the
  next clear (upstream took a goal it reached twice off the list instead).
* **Dives** (`Bot::dive`): TS 3.0 dives (`+alt1`) land prone unless duck, jump or the stunt key is
  held; upstream pressed the key alone at stunt waypoints, and `DEFEND` pressed it on every think
  until prone, which standing still does nothing outside slow motion. A bot's own dive holds duck
  from 0.15 to 1.5 s, so it rolls out of the landing, and a bot that lands prone anyway jumps to
  get up; `DEFEND` no longer presses it.
* **Task timeouts**: `SetTimeToCompleteSchedule` times out `secs` from now (upstream's `2 * now +
  secs` never came, §5.50); a `FIND_PATH` that found its path has 30 s to reach the first waypoint
  (upstream: no limit until a waypoint was touched, then 20-30 s per touch, as now). A goal A*
  can't reach stays failed instead of being taken off the list by a second failure, and is backed
  off for 30-60 s (`AStarBot::backed_off_goals`, past upstream's clear of every failed goal 5-10 s
  after the last failure); `SearchOptions::UPSTREAM` keeps upstream's search for the reference
  tests.
* **Goals** (`goals.rs` `choose_goal`, `items.rs` `ItemMemory`): upstream's TS bot went for the
  nearest weapon or power-up in its 60° view, wanted or not (`CanPickup` checked only the class),
  so it never went for one behind it and pressed use for 10 s on ones it couldn't take (no
  inventory space, a power-up already held: `+use` on another swaps it), waited 5-10 s and went
  back. The port knows the map's items (`BotWorld::items`: `ts_groundweapon`, `ts_powerup`, plus
  what it sees; the host fills `Entity::item` from the plugin API's `entity_info`), sees whether
  the ones in view are there, learns how long a taken one takes to come back (from 20 s for
  weapons, 60 s for power-ups), and scores each as `value * 1000 / (distance * 1.3 + 200 + 300
  danger)` against roaming (10): a gun it has room for (`TSSpace` against TS's weapon slots) by
  `bot_weapons.ini` priority over its best loaded gun (60 + 5 priority when unarmed), its own gun
  when low on reserve, health by what it lacks, armour, other power-ups only with none held or
  running (more unarmed, kung fu most). `CanPickup` asks the same. An item still there 2 s after
  the bot touched it, or after 1.5 s of pressing use on it, is left alone for 30 s.
* **Hearing** (`hearing.rs`): an enemy's footsteps (the `sound` callback) and enemies firing
  (`IN_ATTACK`, now within 1500 units, 350 with a silencer, `TSA_SILENCER`; upstream: the nearest
  player within 750, teammates included) are remembered for 8 s with where, when, how loud and
  who; the bot glances at a new one for 0.8 s and, if it was only roaming, chooses its goal
  again. Investigating a sound is a goal (`Investigate`: 30 x loudness, fresher and more for a
  healthy, armed bot): a path there (15 s at most) and a moving search around it. Upstream's
  listen task had no entity and failed at once (§5.58) after `AddPriorityTask` made every later
  task forget its path, and gunfire led to a "find the path and finish" task that never walked it.
* **Enemy memory** (`remember.rs`): remembered positions carry the time and are forgotten after
  15 s or when their player dies (every bot hears the death); at most 8 are kept (upstream 9), and
  positions without an entity are one only when within 64 units (§5.53). A lost enemy is a goal
  (`Hunt`: 45, fresher and more for a ready bot) instead of half of all roaming going to the
  newest position whatever its age: the bot paths (15 s at most) to a waypoint the enemy could have
  walked to since (200 units/s along the graph, at most 1500, one the bot can't see from where it
  is), pre-aiming, then searches around it on the move for 3-6 s (upstream's search after roaming
  had duration 0 and ended at the next think; its searches stood still). Each position is hunted
  once.
* **Pre-aim** (`preaim.rs`, `LookTask::PreAim`): following a path, the bot looks at the waypoints
  the next waypoint sees and the current one doesn't (the places about to come into view),
  150-1500 units away and not straight ahead, most dangerous first (the heat map, places enemies
  were seen near), each for 0.6-1.2 s, not again within 4 s; standing (defending), at what its
  waypoint sees. Upstream looked at the waypoint it walked to.
* **Objectives** (`objective_tasks`): upstream picked a random objective; team 1 defended it and
  team 2 used it, whatever it was. The port gives each a role for the bot's team: the team the
  port reports for the objective's entity if any (API v5 gives none for `ts_hack`/`ts_bomb`), else
  defend one whose use by this team failed in the last 60 s (still active after the 9 s use) or
  whose text for the team is empty or says to defend, protect, guard, prevent, stop or hold, else
  complete it. Attackers go for one (1 in 8 roam instead), defenders hold a waypoint 200-900 units
  from it that sees it (the visibility table) for 10-20 s, pre-aiming, then rotate to another (1
  in 4 roam). `DEFEND` stands still for 10-20 s (upstream 20-80 s, diving until prone).
* **Capture the briefcase** (`ctb_tasks`, `GameState::ctb`; upstream had none for TS): a bot with
  the case (`PlayerInfo::has_flag`) takes it to the nearest `ts_briefcase` its team may capture
  at; a teammate carrying it is escorted (kept within 300 units); an enemy carrier is cut off at
  the point it must reach; otherwise the bot goes for a case lying in the map that its team may
  take (`ts_flag` of another team or any).
* **Tactical paths** (`rcbot config tactical_paths 1`, off by default: upstream's shortest
  paths; `nav::Tactics`): an edge into waypoint `w` costs its length times `1 + 2 danger(w) +
  1.5 exposure(w) + 0.5 crowded(w)`: `danger` from the learned heat map (`BotGlobals::danger`, a
  `nav::DangerSource`; none yet), `exposure` the waypoints near where enemies were lately seen that
  see `w` (the visibility table), `crowded` teammates' paths. The heuristic is then the distance to
  the goal alone (upstream adds the distance from the bot, which overestimates), admissible as
  every factor is at least 1 (teleports, which cost nothing, excepted).

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

29. **The first word of a keyword list can't be found** (`HAL_FindWord`, `megahal.cpp:671`):
    symbol 0 means "not found", which is right for a model's dictionary (`<ERROR>`) but not for
    the banned/auxiliary lists. Kept.
30. **HAL file names lack a separator** (`PrepareHALBrainForPersonality`,
    `megahal.cpp:2085-2105`; `LoadHALBrainForPersonality` `:2225`): the default `.ban`, `.aux`,
    `.swp` and `.trn` files are looked for as `botprofiles<id>_hal.ban` in the bot folder, next to
    `botprofiles/`, not in it. Kept (the names are upstream's).
31. **`RemoveNameTags` reads `in_string[i + i]`** for the second character of `-=` / `=-`
    (`megahal.cpp:270`, `:311`), so those tags only work at the start of a name (and read past
    the name further on). Kept.
32. **`HumanizeString`'s second drop test is never true** (`megahal.cpp:209`):
    `c < '0' && c > '9'`. Kept.
33. **A `.swp` line without a target crashes** (`HAL_InitializeSwap`, `megahal.cpp:1849`):
    `strtok` returns `NULL` and it goes to `strlen`. Fixed (§4).
34. **An empty line in the training text relearns the previous line** (pretraining,
    `megahal.cpp:2236`): `HAL_MakeWords` returns early and `HAL_Learn` learns the old words, read
    from the shared buffer the empty line overwrote. Kept.
35. **`CBot::BotChat` indexes chat lists with -1** (`bot.cpp:1959-2020`) when the bot talks to
    nobody: leaving says a `[kills-friendly]` line, cursing a `[greetings-friendly]` one, and
    laughing reads before `m_BotChat`. Kept, except the laugh (§4).
36. **`BotChatReply` cuts the message by one byte** when the bot's name is in it
    (`megahal.cpp:139`, `BotFunc_FillString(…, strlen(szMsg))`), and every bot gets the same
    buffer, so each sees what the bots before it did. Kept.
37. **Question replies overrun the reply buffer** (`BotHALGenerateReply`, `megahal.cpp:463`):
    they are cut to 128 characters plus the terminator in a 128-byte buffer. The port cuts to
    128 and the caller to 127.
38. **A loaded brain keeps the context array of the model's creation order**
    (`megahal.cpp:835`): a brain of another order would overrun it. Kept for order 5.

39. **`debug_ent off` and `debug_bot` without a client** (`bot_commands.cpp:742`, `:778`): the
    server console (or `bot_config.ini`) dereferences a `NULL` `pClient`. Fixed (§4).
40. **`debug` from `bot_config.ini` on a listen server** (`bot_commands.cpp:829`):
    `pClient->GetPlayer()` on a `NULL` client. Fixed (§4).
41. **`util notarget_mode/noclip_mode/godmode <player>` from a dedicated server's console**
    (`bot_commands.cpp:499`, `:532`, `:558`) print the caller's name through a `NULL` edict.
    Fixed (§4).
42. **`squad form vee` sets the wedge** (`bot_commands.cpp:1251`). Kept.
43. **`bot_config.ini` tokens and `bot_users.ini` fields overrun their buffers**
    (`globals.cpp:1520-1545`, `ReadBotUsersConfig`): unbounded copies into 64-byte buffers, and
    a NUL one past each full user field. Fixed (§4).
44. **A `.rcr` file of the wrong size leaks its `FILE*`** (`bot_rep.cpp:204`, `:276`: `return`
    before `fclose`), and that profile never gets reputations again until it is deleted. Kept,
    without the leak.
45. **A trailing `;` in `mp_teamlist` stays in the last team's name** and an empty list picks from
    an empty array (`BotFunc_AddBot`, `bot_commands.cpp:2040`, `:2061`). The first is kept, the
    second fixed (§4). `newteam` isn't cleared at a `;`, so an empty team (`a;;b`) is a copy of
    the one before it (kept), and a team of 64 bytes or more overruns `char newteam[64]` (cut at
    63 bytes, §4).
46. **`BotFunc_WriteProfile` writes a `NULL` string as its bare tag without a newline**
    (`dll.cpp:1636`), so the next key is lost on reading. Kept (nothing calls it).

47. **Bots aim with a third of the inverted pitch** (`CBot::RunPlayerMove`, `bot.cpp:8961`):
    `pfnRunPlayerMove` gets `pev->angles` (`angles.x = -v_angle.x / 3`), and the engine copies the
    usercmd's angles into `v_angle`. Fixed (§4).
48. **TS kung fu is 38, but the code also uses 36** (TS 3.0's Contender): `StartFrame` adds
    weapon 36 to every TS bot (`globals.cpp:735`), `WStatus` makes 36 current when a weapon is lost
    (`bot_client.cpp:341`), `NORMAL_ATTACK` treats 36 as melee (`bot.cpp:15122`) and the stunt
    waypoint's kung fu check reads 36 (`bot_navigate.cpp:1490`). Fixed (§4).
49. **TS bots never reload by RCBot's logic** (`BotClient_TS_WeaponInfo`, `bot_client.cpp:468`):
    `AddWeapon`/`SetWeapon` reset `m_iMaxClip` before `checkMaxClip(clip)`, so the "max clip" is
    the last clip and 0 for an empty gun (`NeedToReload`, `ClipIsEmpty` false), and `m_iAmmo1` is
    NULL for every TS weapon (`CanReload`, `LowOnAmmo` false). TS's `WeaponIdle` reloads an empty
    gun only once the trigger has been up for half a second (a dry fire sets
    `m_flTimeWeaponIdle` 0.5 s on), and `NORMAL_ATTACK` presses it on 90% of thinks (every 0.04
    s) while the enemy is in sight: a bot that empties its gun dry-fires until the enemy is out
    of sight. In `cargo xtask e2e` three quarters of the bots' attack commands were on an empty
    clip, and kills were rare enough that a two-minute run sometimes had none. **Fixed** for the
    max clip (§4): the largest clip seen is kept, so `NeedToReload` makes `NORMAL_ATTACK` take
    cover and press reload, as upstream means to; `m_iAmmo1` stays NULL (`CanReload` false).
50. **`SetTimeToCompleteSchedule` adds the time twice** (`bot.h:1134`): schedules time out at
    `2 * time + secs`. **Fixed** (§4).
51. **`EnemyFound` sets `m_fStrafeTime` to 2-6 s since the map started** (`bot.cpp:715`), not
    from now. Kept.
52. **`gotStuck` decrements the waypoint index** (`bot.cpp:8000`) instead of going back to the
    previous waypoint, and `CheckStuck` calls it on every frame of the second second of being
    stuck (about 100 decrements at 100 Hz), then fails the path that left. **Fixed** (§4).
53. **Remembered positions**: up to 9 are kept (`Size() > MAX_REMEMBER_POSITIONS`), positions
    without an entity all compare equal (`bot.h:1888`), and `EnemyDown` runs after
    `NORMAL_ATTACK` cleared the enemy, so nothing is removed: a dead enemy's position is hunted
    for ever. **Fixed** (§4: 8 kept, entity-less ones by place, forgotten on death and after 15 s).
54. **The remembered-point check is always true** (`UpdateVisibles`, `bot.cpp:6091`): it checks
    the bot against its own PVS. Kept.
55. **The enemy-cost buckets all get the same cost** (`workEnemyCosts`, `bot.cpp:16873`): the
    cheapest is always the first, so `m_vLowestEnemyCostVec`, where bots back off to within 256
    units of an enemy, is always 192 units to the -x/-y side. Kept.
56. **`WantToFollowEnemy` ignores its perceptron** (`bot.cpp:11040`, "Not required?"): the bot
    follows while above 30% health. Kept.
57. **Auto-waypoints moved to the floor stay in their old location bucket** (`client.cpp:541`,
    `:587`: `waypoints[i].origin = v_floor + 36`). Kept.

58. **The listen task a sound starts has no entity** (`HearSound`, `bot.cpp:10285`): `SOUND_PLAYER`
    adds `BOT_TASK_LISTEN_TO_SOUND` with a `NULL` edict, which the task fails at once, so hearing
    an enemy's footsteps only costs the bot its current task's place. **Fixed** (§4: sounds are
    remembered and investigated).
59. **Grenade-throw waypoints ask for HL's hand grenade** (`BotNavigate_UpdateWaypoint`,
    `bot_navigate.cpp:1580`): `HasWeapon(VALVE_WEAPON_HANDGRENADE)` is id 12, TS's Desert Eagle,
    and `BOT_TASK_THROW_GRENADE` fails for TS anyway (only Opposing Force throws), so a bot with a
    Desert Eagle gets a task that fails. Kept.
60. **A blocked formation place moves towards the map's origin** (`CBotSquad::GetFormationVector`,
    `bot.cpp:10763`): the trace's end *position* is normalised and shortened by 32 as if it were
    the offset from the leader. Kept.
61. **Squads can't hold fire** (`Think`, `bot.cpp:3800`): `BOT_CONDITION_CANT_SHOOT` from
    `SquadCanShoot` is removed on the next line, before any task reads it (and TS's
    `WantToFindEnemy` ignores it). Kept.
62. **Stealth squads don't slow down** (`Think`, `bot.cpp:3150`): the half speed is overwritten
    with the full speed before anything reads it. Kept.
63. **An enemy's priority only counts if it is also nearer** (`FindEnemy`, `bot.cpp:9184`):
    `iPriority >= iBestPriority && fFitness < fBestFitness`, so a disliked player seen after a
    nearer stranger isn't chosen; a higher priority seen first does keep lower ones out. Kept.
64. **A message said in several arguments keeps only the last** (`ClientCommand`, `dll.cpp:884`):
    the old message (`szTemp`) is never set, so each argument replaces the message. Kept.
65. **TS bots run for cover from a grenade every think** (`Think`, `bot.cpp:3523`):
    `m_fNextCheckCover` is never set in the TS branch, so `RunForCover(…, true)` replaces the
    cover schedule each think while the grenade is the entity to avoid. Kept.
66. **A follower's task goes on following the dead leader** (`BOT_TASK_FOLLOW_LEADER`,
    `bot.cpp:13072`): after `ChangeLeader` the task's edict is set to `m_pSquadLeader`, still the
    old leader until the next think. Kept.
67. **The "nearest" button is the first one seen** (`BotFunc_FindNearestButton`,
    `bot.cpp:10436`): the loop stops at the first entity targeting the door that a trace reaches,
    whatever its distance. Kept.
68. **A master's relay is followed to its first target only** (`CMasterEntity::FindButton`,
    `bot.cpp:11135`): the inner search starts from the relay each time, so it finds the same
    entity and stops. Kept.
69. **Without `mp_timelimit`, every bot wants to leave** (`WantToLeaveGame`, `bot.cpp:1243`): the
    share of the time limit is `1 - (-t) / 0`, infinite, so with `bots_leave_and_join` on and no
    time limit bots leave as soon as they can (unless bored to 255, which gives `NaN`). Kept.
70. **TS bots never switch weapons by RCBot's logic**, a consequence of 49: the only TS switch
    (`Think`, an empty clip while fighting) needs `ClipIsEmpty`, which an empty TS gun never is.
    TS itself changes weapons (a dropped empty gun, a pickup). Fixed with 49.
71. **The force grip holds the entity along the last computed direction** (`CClient::Think`,
    `client.cpp:234`): the end point is built from `gpGlobals->v_forward` before
    `UTIL_MakeVectors(pev->v_angle)`, so from whatever the previous `MakeVectors` (any bot's or
    client's) left there. **Fixed** (the player's view). `+rcbot_force_grip` also compares the
    access bit set with 9 as a number (`dll.cpp:946`); kept.
72. **Bots shoot through walls** (`BOT_TASK_NORMAL_ATTACK`, `bot.cpp:14719`, `:15492`): the
    task acts on `BOT_CONDITION_SEE_ENEMY`, which `HasSeenEnemy`'s `FVisible(vLastEnemyPosition)`
    sets when the *last seen position* is in sight, `EnemyFound` sets unconditionally, and
    `UpdateVisibles` keeps until its pass reaches the enemy again; the next think then aims and
    fires at the enemy's current origin, behind the wall. **Fixed** (§4).
73. **Held reload key** (`BOT_TASK_RELOAD`, `bot.cpp`): the task presses `IN_RELOAD` every think
    for 0.5-1.5 s whatever the weapon; with TS kung fu or a blade that key is the block, so a
    bot that switched weapons blocked over and over, draining its power. **Fixed** (§4).

Quirk kept, not a bug as far as can be told: `CPerceptron::setWeights(CBotGAValues*, …)`
(`perceptron.cpp:74`) sets the bias weight to the bias (1), so GA-bred perceptrons have a fixed
bias weight.

Data: `ts_hammertime_d1.rcw` names its map `ts_hammertime2_d1`, so it never loads on
`ts_hammertime_d1`.

The research for #104 lists RCBot's TS bugs elsewhere (weapon switching through `weaponselect`,
model-based teams, the 2.x `WStatus` kung fu id, free weapons instead of buying). The port fixes
those by going through the port's own code when that code is ported.

* **`TeamScore` and TS's team names** (`BotClient_Generic_TeamScore`): the team index comes from
  TFC's colour names (`Blue`, `Red`, `Green`, `Yellow`), so every TS team's score lands in slot 0
  (the last message wins) and the death fitness compares the wrong team's score. The port reads the
  scores by rules team.

## 6. Tests

`cargo xtask test` runs everything on x86_64 and i686 Linux.

* **Combat and kung fu** (`bot/tests.rs`, `bot/kungfu.rs`): no shot at an enemy once a wall
  stands between them (239 shots without the §5.72 fix); one reload press per empty clip and
  none with a full clip or no reserve; an empty gun dropped once for kung fu; cover from a far
  gunman; blocks, ducks and steps against a punching opponent, with strikes back and learning;
  the run-up crouch-jump flying kick and the wall-jump back kick in a world with jumps and TS's
  wall jump; `.rkf` round trip and damaged files; combos bred from the best.

* **Navigation** (`bot/tests.rs`, on shipped `.rcw` graphs in the plane world, the bot put at
  the height of the waypoint it heads for): roaming goals cover at least 95% of four maps' usable
  waypoints (upstream's at most 50); a bot roaming ts_dojo for three minutes touches half the map
  with goals all over it; a bot blocked between two waypoints goes back to the previous one once
  (no index jumps) and fails that path within 3 s; blocked for good, it escalates (backing up, a
  dive or wall jump) and kills itself only after 20 s; a dive holds duck through the landing and
  jumps up from prone. A* unit tests: failed goals backed off, tactical costs (danger, exposure,
  teammates' paths), zero tactics giving shortest paths. Items: what weapons and power-ups are
  worth (room, priority, ammo, a held power-up, health); an item at the far end of ts_dojo is gone
  for at once; a power-up the game never hands over is tried at most twice in a minute (upstream:
  10 s of use, then again every 5-10 s); respawn times learned; untakeable items left alone.
  Hearing and hunting: footsteps heard are remembered (not teammates', not silent ones or too far),
  and on ts_dojo the bot walks to the sound and searches around it on the move; an enemy lost 4 s
  ago is hunted where it could have gone, once, and forgotten after 15 s or when it dies;
  pre-aim picks the waypoint coming into view, the dangerous one first, then another; objectives
  by their text (attack: path and 9 s use; defend: a waypoint 200-900 units off that sees it); CTB
  (take the other team's case, carry it home, escort a teammate, cut off an enemy carrier).
* **Unit tests** in each module: path slots and the cursor, the searches (nearest waypoint and
  nearest entity, traced nearest first, against upstream's loop on random worlds with walls), buckets and cover, the
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
* **`tests/upstream_hal.rs`**, MegaHAL against RCBot's: `build.sh` pastes the whole of
  `megahal.cpp` (with `strlow`, `UTIL_BuildFileName`, `BotFunc_FillString`) into
  `hal_harness.cpp`, built 32-bit, with `XorShiftRandom` as `RANDOM_LONG`. On the fixture in
  `tests/reference/hal/` (keyword lists, a training text, chat from humans and bots with
  reputations, clan-tagged names) it records the empty, pretrained, after-chat and reloaded brains,
  every reply before and after `HumanizeString`, and `RemoveNameTags`; the port matches all of it
  byte for byte. Unit tests cover the dictionary order, learning, keywords, lists, `bot_chat.ini`
  parsing and substitutions, and the bugs kept.
* **Config unit tests**: the tokenizers' quirks (`fgets` chunks, `#`, spaces vs tabs, `atoi`/`atof`),
  user, profile and player-id files, `.rcr` records byte for byte, every menu's text and key
  mask and menu navigation, and the commands on a fake `CommandHost`: access checks and their
  messages, the listen-server/dedicated paths, `bot_config.ini` without a client, `config`
  get/set for flags, ints and floats, `addbot` arguments, waypoint flags by name and by menu
  (TS's "Stunt"), team waypoints, `util`, `users`, squads, `botcam`, `autowaypoint`, `debug`,
  and the TS team choice from `mp_teamlist`.
* **Bot tests** (`bot/tests.rs`): a bot on an open plane, its usercmds moving it, walks a line of
  waypoints (touching them in order) and attacks a visible enemy with the gun it holds; human
  towers (the touch test, crouching, standing for a climber, jumping), pushables (where to stand),
  waiting for a lift, `CheckLift` pressing a door's button (a world with a door and its button),
  a momentary button held while its door moves, grenade-throw waypoints failing in TS, touching
  a picked-up item, squads (wedge places, teams, handing the lead on), enemy choice by reputation
  (and the priority quirk), reputation after a death, breakables as enemies by the first client near them, the visible list's order and membership, sound kinds, hearing range, the sound's origin and silent or ignored sounds, chat lines
  (greetings once, `%n`/`%m`, typing), the bot camera, the grenade toss, `tm_` maps' model
  teams, `util bot_goto`, squad members following the leader when they roam, and the loadout
  bought before each spawn; unit tests cover the
  usercmd `msec` (every 100 Hz frame sends, nothing lost at 30 Hz or 1000 Hz, the first
  command one frame long), the task queue (schedules, front insertion, path hand-over, timeouts), angles and turning,
  remembered positions, the TS `WeaponInfo` handler and the best-weapon choice (kung fu up close,
  an emptied gun needing a reload), a teleport dropping the path, the owned weapons read from the inventory (a gun never held
  known, a lost one forgotten, a new life without the last one's guns), the weapon in hand as
  the game reports it (a switch asked once, a refused one forgetting the weapon), and the
  waypoint editor (flags, paths, delete, beam colours).
* **Host unit tests** (`crates/host`): the plugin descriptor, and engine entity searches
  (`collect_search`) ending at the world FWGS and ReHLDS return for "no more" (and at a repeat)
  instead of starting over: upstream's loops stop at `FNullEnt`, the port's first ones only at
  NULL, so each search gave up to 4097 repeats.
* **`cargo xtask e2e [root...]`** (all seven `~/games` roots in parallel, about 5 min): installs
  the port (`cargo xtask install <root>` in `../rust-hlmod-the-specialists`) and
  `dist/<platform>/` over `<root>/ts/`, starts a dedicated server on ts_dojo that selects
  `ts_bot_backend rcbot`, changes the map and adds six bots (quota 5 and `addbot`), sets
  `mp_timelimit 2` and selects `native` from the next map's `mapchangecfgfile` (the port reloads
  the map when `ts_bot_backend` changes, so selecting it on the RCBot map would end RCBot's run at
  once). Checks: the plugin loaded, the backend
  selected, ts_dojo's 154 waypoints loaded, bots created and run, one moved more than 64 units,
  one touched three waypoints, a kill by one of RCBot's bots (the plugin's `kill` lines), a
  living bot's weapon changing, then after the time limit the next map with the native backend
  (`[ts] bots: backend native`, the plugin's backend inactive, native bots added), and no panic
  or crash. Every 5 s the log has a line per bot (waypoint, goal, waypoints touched, distance,
  enemy, weapon, task). It refuses to run while another `xtask` runs, and removes the library
  from `dlls/` afterwards.
