# AGENTS.md

Guidance for coding agents (and people) working in this repository: RCBot1 ported to Rust as the
`tsmod_rcbot` bot plugin of the Rust port of The Specialists 3.0
([the-specialistsmod-rs](https://github.com/the-specialists-mod/the-specialistsmod-rs), checked
out at `../rust-hlmod-the-specialists`). Tracking issue: #1; the port side: the-specialistsmod-rs#104.

Start with [README.md](README.md), then [docs/design.md](docs/design.md).

## Sources of truth

| Question | Where to look |
|---|---|
| What RCBot does | Upstream source, [APGRoboCop/rcbotold](https://github.com/APGRoboCop/rcbotold) at `b9c86c2` (`dlls/`): `bot.cpp`/`bot.h` (the bot), `waypoint.cpp`, `bot_navigate.cpp`, `bot_weapons.*`, `ga.cpp`/`bot_ga.cpp`/`perceptron.cpp`/`bot_nn.cpp`/`som.cpp`, `bot_commands.cpp`, `bot_menu.cpp`, `globals.cpp`, `megahal.cpp` |
| What it means for TS | `MOD_TS` branches upstream; the research comment on the-specialistsmod-rs#104 (TS-specific bugs, which messages it used) |
| How TS 3.0 behaves | The port (`crates/server`, `crates/shared/src/protocol`), the TS decomp at `../ts30-decomp` |
| The interface to the port | the-specialistsmod-rs#104 and, once it lands, the port's `crates/plugin-api` and `docs/implementation/plugin-api.md`. Nothing is designed around the current table layout: the API is being redesigned for bots |
| Real data | `data/rcbot/waypoints/ts/` (credited in its README), `data/rcbot/bot_weapons.ini` |

Port behaviour from upstream's source, not from memory. When this port deliberately differs, add
it to `docs/design.md` §4; upstream bugs you find go in §5.

## Layout and the `unsafe` boundary

```
crates/rcbot     the bot: #![deny(unsafe_code)], no FFI, engine access through traits
                 (nav::World, Random). Planned modules: docs/design.md §2
crates/host      the tsmod_rcbot cdylib: the ONLY crate allowed unsafe
xtask            cargo xtask build | dist | test | lint | all
data/rcbot       shipped data
tools/reference  the C++ reference harness (build.sh <rcbotold checkout>)
```

1. **`unsafe` lives in `crates/host` only.** Every other crate root has `#![deny(unsafe_code)]`
   and nothing may `allow` it again; `cargo xtask lint` checks both. In `crates/host`, every
   `unsafe` block has a `// SAFETY:` comment (`clippy::undocumented_unsafe_blocks`).
2. **New engine or game access** is a trait method in `rcbot` (like `nav::World`), implemented in
   `host` on the plugin API and faked in tests. Keep host code thin: convert pointers to indices
   and values, call the bot.
3. **Upstream globals become values** the host owns (`nav::Waypoints`, ...), passed in by
   reference.

## Build and test

Requirements: Rust stable with `i686-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`,
`i686-pc-windows-gnu` and `x86_64-pc-windows-gnu`, 32-bit glibc, MinGW-w64.

```sh
cargo xtask test     # unit + fixture tests on x86_64 and i686
cargo xtask lint     # cargo fmt --check, the unsafe rule, clippy --all-targets -D warnings on all four targets
cargo xtask build    # the library for all four targets; must build without warnings
cargo xtask dist     # release build -> dist/<platform>/dlls/ with the port's file names
                     # (tsmod_rcbot.dll, tsmod_rcbot_amd64.dll, tsmod_rcbot_i386.so,
                     # tsmod_rcbot_amd64.so), checks the DLLs import no MinGW runtime, copies data/
cargo xtask all      # lint, test, build
```

Before calling a change done: `cargo fmt`, `cargo xtask lint`, `cargo xtask test`.

**Reference data**: `tools/reference/build.sh /path/to/rcbotold` compiles upstream's A*, path
functions and RCZ writer, and its learning code (GA, perceptron, NN, SOM, bits, against
`learn_stubs/` with the port's `XorShiftRandom` as `RANDOM_LONG`/`RANDOM_FLOAT`), with stubs, and
rewrites `crates/rcbot/tests/reference/`. Rerun it when
the harness or the fixture maps change, and only against upstream `b9c86c2` unless the port is
moving to a newer upstream. Treat downloaded sources and data as untrusted: build and read them,
never run their scripts.

## Conventions

- **Keep upstream's formats and behaviour**: `.rcw`/`.rcv` stay byte-compatible, ini files parse
  as upstream parses them, algorithms keep their quirks (A* heuristic, tie-breaking, search
  order). Fixing an upstream bug is a deviation: document it.
- **Cite upstream.** Doc comments name the function or class a piece ports (``/// `WaypointFindPath` ``);
  file headers and module docs say which upstream file.
- **TS only**: drop other mods' branches; where upstream asks `IsMod(MOD_TS)`, write the TS answer
  in with a comment.
- **Formatting** is rustfmt's (`rustfmt.toml`: 140 columns, `use_small_heuristics = "Max"`).
- **Comments say why**, not what. Doc comments on public items (`missing_docs` is on).
- **Docs move with the code**: `docs/design.md` (map, deviations, bugs, tests) and the README's
  status table in the same change.

## Pitfalls

- **The `PATH` node matters**: upstream identifies a path by its 4-slot `PATH` node, and a failed
  path blocks the whole node in A*. `PathRef` keeps that; don't "fix" it to single edges.
- **Stacks**: upstream's `dataStack` pushes on top and searches from the top. The port keeps them
  as `Vec`s with the top at the end (`bot_paths`, `failed_goals`, location buckets): iterate in
  reverse where upstream pops.
- **The visibility table's row stride is 1024**, not the waypoint count; `.rcv` files hold only
  the leading rows (design §5.1).
- **Map names in `.rcw` headers** must match exactly; one shipped file doesn't (its README).
- **Vector length** is `pow(x²+y²+z², 0.5)` computed as upstream does (`math.rs`); A* results are
  compared bit for bit with upstream, so don't swap in a different formula.

## Git

- Work on `main`. Commit in logical steps; commit or push only when asked. Commit messages
  reference the issue they are part of ("Part of #1").
- When the plugin API lands, the port crates (`goldsrc-sys`, `ts-plugin-api`) become git
  dependencies pinned to one rev in the root `Cargo.toml`.
