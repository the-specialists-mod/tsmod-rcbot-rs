//! Build, package and check tooling: `cargo xtask <command>`.
//!
//! ```text
//! build [--release]   the tsmod_rcbot library for the port's four platforms
//! dist [--debug-info] release build -> dist/<platform>/dlls/<port file name> (stripped unless
//!                     --debug-info) + dist/<platform>/rcbot/ (bot_weapons.ini, waypoints);
//!                     checks the DLLs import no MinGW runtime
//! test                unit tests on the host and on i686 Linux
//! lint                cargo fmt --check; clippy -D warnings on every target; the unsafe rule
//!                     (only crates/host may use unsafe)
//! all                 lint, test, build
//! ```

#![deny(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// A build target and the file name the port's loader looks for (the-specialistsmod-rs#104).
struct Target {
    triple: &'static str,
    platform: &'static str,
    /// Cargo's output file name.
    built: &'static str,
    /// The name in the port's dlls/ directory.
    file: &'static str,
    strip: &'static str,
}

const TARGETS: [Target; 4] = [
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

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn run(cmd: &mut Command) -> Result<(), String> {
    eprintln!("$ {cmd:?}");
    let st = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("{cmd:?} failed: {st}")) }
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn build(release: bool) -> Result<(), String> {
    for t in &TARGETS {
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

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
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

fn dist(debug_info: bool) -> Result<(), String> {
    build(true)?;
    for t in &TARGETS {
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
        // TODO(the-specialistsmod-rs#104): the data goes wherever the API's data_dir points.
        copy_dir(&root().join("data/rcbot"), &base.join("rcbot"))?;
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
        Some("dist") => dist(flag("--debug-info")),
        Some("test") => test(),
        Some("lint") => lint(),
        Some("all") => lint().and_then(|()| test()).and_then(|()| build(false)),
        _ => Err("usage: cargo xtask build [--release] | dist [--debug-info] | test | lint | all".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
