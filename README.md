# tsmod-rcbot-rs

[RCBot1](https://github.com/APGRoboCop/rcbotold) (Cheeseh's Half-Life bot) ported to Rust for
**The Specialists 3.0**, as the `tsmod_rcbot` bot plugin of the
[TS Rust port](https://github.com/the-specialists-mod/the-specialistsmod-rs). The port loads it
through its plugin API, and `ts_bot_backend rcbot` selects it instead of the native TS bots
(the-specialistsmod-rs#104). Progress: [#1](https://github.com/the-specialists-mod/tsmod-rcbot-rs/issues/1).

## Status

| Part | State |
|---|---|
| Workspace, licences, xtask | done |
| Navigation: `.rcw` waypoints (byte-compatible), flags, paths, visibility table (`.rcv`), A* | done; checked against RCBot's own code (docs/design.md §6) |
| `bot_weapons.ini` (TS section) and the TS weapon table | done |
| Learning: GA, perceptrons, NN, SOM, `.rld` learned-data files (byte-compatible) | done; checked against RCBot's own code (docs/design.md §6) |
| Chat: MegaHAL (byte-compatible brains), `bot_chat.ini`, replies | done; checked against RCBot's own code |
| Config: `bot_config.ini`, users, profiles, reputation, menus, the `rcbot` command set and `±rcbot_force_grip` (side effects through a host trait) | done; the host runs the commands |
| Plugin entry point and host (`ts_bot_backend rcbot`, `rcbot` commands, data in `tsmod/rcbot/`, `map_configs/ts/<map>.cfg`, RCBot's `min_bots`/`max_bots`) | done, on the port's plugin API v5 |
| Bot core: think, tasks, waypoint following with A*, stuck handling, view and aim | done |
| Combat: what the bot sees (a fresh line of sight before every shot), enemy choice and aim by reputation, best weapon (of the inventory the game reports), reloads (one press per empty clip) and switching (upstream's TS reload bug fixed), empty guns dropped, cover, hearing (the port's `sound` callback); TS 3.0's triggers (semi, burst and akimbo guns tapped at their rate), aim points (chest, head, shotgun centre), partial reloads, blades that slash instead of throwing | done |
| Kung fu (a port addition): reads the opponent, defensive or offensive stance, blocks, ducks and steps out of attacks, counters, learned and evolving combos with crouch-jump, flying and wall-jump kicks, saved in `<id>.rkf`; unarmed bots take cover from distant gunmen | done |
| Waypoint editing, display (beams), autowaypointing, the waypoint menus on the HUD (`ShowMenu`) | done |
| Special waypoints: lifts and their buttons, pushables, human towers, grenade-throw waypoints | done |
| TS: loadouts by GA (bought before each spawn), kung fu, power-up pickup and `usepowerup`, stunts (the perceptron), prone and crouch, teamplay end-level goals (and `tm_` maps' model teams) | done; TS objectives and team scores wait for plugin API additions (the-specialistsmod-rs#117) |
| Bot events and reputation (kills, deaths, what bots see), squads, botcam, chat (idle, events, greetings, MegaHAL replies, typing), profiles from `bot_profiles.ini`, learned data, brains and reputations saved and loaded | done |
| End-to-end: `cargo xtask e2e` on all seven engine roots | bots join, move, use waypoints, kill, change weapons; back to `native` at the next map |

Only The Specialists 3.0 is supported. Code for RCBot's other mods, its copies of metamod, its
engine hooks and its user-message parsing are not ported (the port supplies structured state
instead). See [docs/design.md](docs/design.md) for the C++ → Rust map.

## Layout

```
crates/rcbot     the bot (no unsafe, no FFI)
  src/nav/       waypoints, .rcw/.rcv files, location buckets, visibility, A*
  src/weapons/   bot_weapons.ini presets, weapon info, the TS weapon table
  src/learn/     GA, perceptrons, NN, SOM, .rld learned-data files
  src/chat/      MegaHAL and its brain files, bot_chat.ini, replies
  src/config/    bot_config.ini, users, profiles, reputation, menus, rcbot commands
  src/bot/       the bot: think, tasks, navigation, special waypoints, movement, aim, combat,
                 its weapons, events and hearing, squads, the bot camera
crates/host      the tsmod_rcbot library: the plugin API v5 and the engine (the only crate allowed unsafe)
data/rcbot       bot_weapons.ini (TS), waypoints/ts/ (44 maps)
tools/reference  C++ harnesses that record RCBot's own results for the tests
xtask            cargo xtask build | dist | test | lint | all
```

## Building

Rust stable with the `i686-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, `i686-pc-windows-gnu`
and `x86_64-pc-windows-gnu` targets, 32-bit glibc and MinGW-w64.

```sh
cargo xtask test     # unit and fixture tests, x86_64 and i686
cargo xtask lint     # fmt, clippy -D warnings on all four targets, the unsafe rule
cargo xtask build    # the library for all four targets
cargo xtask dist     # dist/<platform>/dlls/tsmod_rcbot{.dll,_amd64.dll,_i386.so,_amd64.so} + tsmod/rcbot/ data
cargo xtask e2e      # on the port's engine roots (~/games): rcbot bots join, move, use waypoints,
                     # kill and change weapons, then the native bots take over at the next map
```

Releases are built by GitHub Actions (`.github/workflows/release.yml`): pushing a `v*` tag that
matches `Cargo.toml`'s version tests, builds and publishes the archives (and `-debug` ones with
debug info): `tsmod-rcbot-rs-<tag>-linux-{i686,x86_64}-glibc{2.36,2.43}.tar.gz`, built on
Debian 12 and Ubuntu 26.04 (pick the newest glibc your server has or exceeds), and
`tsmod-rcbot-rs-<tag>-windows-{i686,x86_64}.zip`. Extract one over the server's game folder: it
holds `ts/dlls/` and `ts/tsmod/rcbot/`.

## Credits

* **RCBot** by Paul "Cheeseh" Murphy (rcbot.net), based on botman's **HPB bot** template;
  maintained by **[APG]RoboCop** ([APGRoboCop/rcbotold](https://github.com/APGRoboCop/rcbotold)).
  Its credits also thank PM of RACC (FakeClientCommand), pierre of RACC (MegaHAL), and the
  tsmod.net admins for The Specialists 3.0 support.
* **TS waypoints**: the 44 `.rcw` files in `data/rcbot/waypoints/ts/` come from
  [2cwldys/ts_h9_team_elimination](https://github.com/2cwldys/ts_h9_team_elimination) (GPL-3),
  made by RCBot's authors, players and server admins; see that directory's README.
* **miniz** (Rich Geldreich), which RCBot uses for its compressed files; the port uses
  [miniz_oxide](https://github.com/Frommi/miniz_oxide), its Rust port.

## Licence

GPL-3.0-or-later ([LICENSE](LICENSE)) with RCBot's exception for linking with the Half-Life
engine and MODs ([GPL-EXCEPTION.txt](GPL-EXCEPTION.txt)). RCBot itself is
GPL-2.0-or-later with that exception. The library is loaded by the port at runtime and is never
linked into the port's own libraries.

The port's plugin API crate `ts-plugin-api` (a dependency from the-specialistsmod-rs) is
MIT-licensed. MIT is compatible with the GPL, so the built library is distributed under the
GPL as a whole. Keep the API crate's MIT notice with it.
