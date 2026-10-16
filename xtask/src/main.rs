//! Build, package and check tooling: `cargo xtask <command>`.
//!
//! ```text
//! build [--release]   the tsmod_rcbot library for the port's four platforms
//! dist [--debug-info] [--platform <prefix>]
//!                     release build -> dist/<platform>/dlls/<port file name> (stripped unless
//!                     --debug-info) + dist/<platform>/tsmod/rcbot/ (the plugin's data directory:
//!                     bot_weapons.ini, waypoints); checks the DLLs import no MinGW runtime.
//!                     Copy dist/<platform>/ over a port install's ts/ directory. --platform
//!                     keeps the platforms starting with the prefix (`linux`, `windows-i686`): the
//!                     release workflow builds Linux in containers of each glibc it ships.
//! e2e [root...]       end to end on the port's engine roots (e2e.rs): installs the port, then
//!                     tsmod_rcbot, and checks rcbot bots join, move and use waypoints
//! test                unit tests on the host and on i686 Linux
//! lint                cargo fmt --check; clippy -D warnings on every target; the unsafe rule
//!                     (only crates/host may use unsafe)
//! all                 lint, test, build
//! ```

#![deny(unsafe_code)]

mod e2e;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// A build target and the file name the port's loader looks for (the-specialistsmod-rs#104).
pub(crate) struct Target {
    pub(crate) triple: &'static str,
    pub(crate) platform: &'static str,
    /// Cargo's output file name.
    built: &'static str,
    /// The name in the port's dlls/ directory.
    pub(crate) file: &'static str,
    strip: &'static str,
}

pub(crate) const TARGETS: [Target; 4] = [
    Target {
        triple: "i686-unknown-linux-gnu",
        platform: "linux-i686",
        built: "libtsmod_rcbot.so",
        file: "tsmod_rcbot_i386.so",
        strip: "strip",
    },
    Target {
        triple: "x86_64-unknown-linux-gnu",
        platform: "linux-x86_64",
        built: "libtsmod_rcbot.so",
        file: "tsmod_rcbot_amd64.so",
        strip: "strip",
    },
    Target {
        triple: "i686-pc-windows-gnu",
        platform: "windows-i686",
        built: "tsmod_rcbot.dll",
        file: "tsmod_rcbot.dll",
        strip: "i686-w64-mingw32-strip",
    },
    Target {
        triple: "x86_64-pc-windows-gnu",
        platform: "windows-x86_64",
        built: "tsmod_rcbot.dll",
        file: "tsmod_rcbot_amd64.dll",
        strip: "x86_64-w64-mingw32-strip",
    },
];

/// The only crate allowed `unsafe`.
const UNSAFE_CRATE: &str = "crates/host";

pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

pub(crate) fn run(cmd: &mut Command) -> Result<(), String> {
    eprintln!("$ {cmd:?}");
    let st = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("{cmd:?} failed: {st}")) }
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

/// The targets whose platform starts with `prefix` (all without one).
fn targets(prefix: Option<&str>) -> Result<Vec<&'static Target>, String> {
    let v: Vec<_> = TARGETS.iter().filter(|t| prefix.is_none_or(|p| t.platform.starts_with(p))).collect();
    if v.is_empty() {
        return Err(format!("no platform starts with {:?}", prefix.unwrap_or_default()));
    }
    Ok(v)
}

fn build(release: bool) -> Result<(), String> {
    build_targets(release, &targets(None)?)
}

fn build_targets(release: bool, targets: &[&Target]) -> Result<(), String> {
    for t in targets {
        let mut c = cargo();
        c.args(["build", "-p", "tsmod-rcbot", "--target", t.triple]);
        if release {
            c.arg("--release");
        }
        run(&mut c)?;
    }
    Ok(())
}

/// The port loads the DLL with only system DLLs around: no MinGW runtime imports.
fn check_windows_dll(dll: &Path, objdump: &str) -> Result<(), String> {
    let out = Command::new(objdump).arg("-p").arg(dll).output().map_err(|e| format!("{objdump}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    for bad in ["libgcc", "libwinpthread", "libstdc"] {
        if text.lines().any(|l| l.contains("dll name:") && l.contains(bad)) {
            return Err(format!("{} imports a MinGW runtime DLL ({bad})", dll.display()));
        }
    }
    Ok(())
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let dest = to.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        }
    }
    Ok(())
}

fn dist(debug_info: bool, platform: Option<&str>) -> Result<(), String> {
    let targets = targets(platform)?;
    build_targets(true, &targets)?;
    for t in targets {
        let base = root().join("dist").join(t.platform);
        let dlls = base.join("dlls");
        std::fs::create_dir_all(&dlls).map_err(|e| e.to_string())?;
        let dest = dlls.join(t.file);
        let built = root().join("target").join(t.triple).join("release").join(t.built);
        std::fs::copy(&built, &dest).map_err(|e| format!("{}: {e}", built.display()))?;
        if !debug_info {
            run(Command::new(t.strip).arg(&dest))?;
        }
        if t.triple.contains("windows") {
            let objdump = t.strip.replace("strip", "objdump");
            check_windows_dll(&dest, &objdump)?;
        }
        eprintln!("  {}", dest.display());
        // The port's `data_dir` for the plugin: <gamedir>/tsmod/rcbot/.
        let _ = std::fs::remove_dir_all(base.join("rcbot"));
        copy_dir(&root().join("data/rcbot"), &base.join("tsmod").join("rcbot"))?;
    }
    Ok(())
}

/// Every crate except [`UNSAFE_CRATE`] denies `unsafe_code` at its root and never allows it again.
fn check_unsafe_rule() -> Result<(), String> {
    let mut roots = vec![root().join("xtask/src/main.rs")];
    for entry in std::fs::read_dir(root().join("crates")).map_err(|e| e.to_string())? {
        let dir = entry.map_err(|e| e.to_string())?.path();
        if dir.ends_with(UNSAFE_CRATE.trim_start_matches("crates/")) {
            continue;
        }
        let lib = dir.join("src/lib.rs");
        roots.push(if lib.exists() { lib } else { dir.join("src/main.rs") });
    }
    let mut errors = Vec::new();
    for crate_root in &roots {
        let text = std::fs::read_to_string(crate_root).map_err(|e| format!("{}: {e}", crate_root.display()))?;
        if !text.lines().any(|l| matches!(l.trim(), "#![deny(unsafe_code)]" | "#![forbid(unsafe_code)]")) {
            errors.push(format!("{}: missing #![deny(unsafe_code)]", crate_root.display()));
        }
        let src = crate_root.parent().unwrap();
        let mut stack = vec![src.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let p = entry.map_err(|e| e.to_string())?.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
                    // Split so this file doesn't match itself.
                    let lint = concat!("unsafe", "_code");
                    for (n, line) in text.lines().enumerate() {
                        let l = line.replace(' ', "");
                        if l.contains(&format!("allow({lint}")) || l.contains(&format!("expect({lint}")) {
                            errors.push(format!("{}:{}: unsafe is only allowed in {UNSAFE_CRATE}", p.display(), n + 1));
                        }
                    }
                }
            }
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

fn lint() -> Result<(), String> {
    run(cargo().args(["fmt", "--all", "--check"]))?;
    check_unsafe_rule()?;
    for t in &TARGETS {
        run(cargo().args(["clippy", "--workspace", "--all-targets", "--target", t.triple, "--", "-D", "warnings"]))?;
    }
    Ok(())
}

fn test() -> Result<(), String> {
    run(cargo().args(["test", "--workspace"]))?;
    run(cargo().args(["test", "--workspace", "--target", "i686-unknown-linux-gnu"]))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let result = match args.first().map(String::as_str) {
        Some("build") => build(flag("--release")),
        Some("dist") => {
            let platform = args.iter().position(|a| a == "--platform").map(|i| args.get(i + 1).map(String::as_str));
            match platform {
                Some(None) => Err("--platform needs a prefix".into()),
                p => dist(flag("--debug-info"), p.flatten()),
            }
        }
        Some("test") => test(),
        Some("lint") => lint(),
        Some("all") => lint().and_then(|()| test()).and_then(|()| build(false)),
        Some("e2e") => dist(false, None).and_then(|()| e2e::run(&root(), &args[1..])),
        _ => {
            Err("usage: cargo xtask build [--release] | dist [--debug-info] [--platform <prefix>] | test | lint | all | e2e [root...]"
                .into())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
