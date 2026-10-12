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
| Plugin entry point and host interface | waiting for the plugin API for bots (#104) |
| Combat, TS behaviour (loadouts, stunts, kung fu, pickups, teams), squads, menus, profiles, commands, MegaHAL, botcam, waypoint editing and autowaypointing | to do |

Only The Specialists 3.0 is supported. Code for RCBot's other mods, its copies of metamod, its
engine hooks and its user-message parsing are not ported (the port supplies structured state
instead). See [docs/design.md](docs/design.md) for the C++ → Rust map.

## Layout

```
crates/rcbot     the bot (no unsafe, no FFI)
  src/nav/       waypoints, .rcw/.rcv files, location buckets, visibility, A*
  src/weapons/   bot_weapons.ini presets, weapon info, the TS weapon table
  src/learn/     GA, perceptrons, NN, SOM, .rld learned-data files
crates/host      the tsmod_rcbot library (the only crate allowed unsafe; a stub for now)
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
cargo xtask dist     # dist/<platform>/dlls/tsmod_rcbot{.dll,_amd64.dll,_i386.so,_amd64.so} + data
```

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
engine and MODs ([LICENSE-EXCEPTION.txt](LICENSE-EXCEPTION.txt)). RCBot itself is
GPL-2.0-or-later with that exception. The library is loaded by the port at runtime and is never
linked into the port's own libraries.
