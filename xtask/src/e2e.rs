//! `cargo xtask e2e [root...]`: tsmod_rcbot on the port's engine roots (`~/games/...`, the port's
//! `cargo xtask roots`), modelled on the port's `cargo xtask plugins`.
//!
//! For each root: the port's own `cargo xtask install <root>` (in `TS_PORT_DIR`, default
//! `../rust-hlmod-the-specialists`), then `dist/<platform>/` over `<root>/ts/` (the library in
//! `dlls/`, the data in `tsmod/rcbot/`). A dedicated server starts on ts_dojo; its first server
//! config selects `ts_bot_backend rcbot` and changes the map (the backend is read at map start),
//! the second adds bots with the quota and `addbot`. The log must show the plugin loaded, the
//! backend selected, the waypoints loaded, bots created and run, a bot that moved more than 64
//! units and one that touched waypoints, and no panic or crash. The library is removed from
//! `dlls/` afterwards so the port's own scenarios run without it. Nothing is written into the
//! roots' `maps/` (a link to the content).
//!
//! It refuses to start while another `xtask` runs (they share the roots and Wine prefix).

use crate::{TARGETS, Target, copy_dir};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    Fwgs,
    Hlds,
}

struct Root {
    name: &'static str,
    dir: &'static str,
    engine: Engine,
    windows: bool,
    /// [`TARGETS`] index of the library it loads.
    target: usize,
}

const ROOTS: [Root; 7] = [
    Root { name: "fwgs", dir: "xash", engine: Engine::Fwgs, windows: false, target: 1 },
    Root { name: "fwgs-win", dir: "xash_win", engine: Engine::Fwgs, windows: true, target: 2 },
    Root { name: "fwgs-win64", dir: "xash_win64", engine: Engine::Fwgs, windows: true, target: 3 },
    Root { name: "rehlds-linux", dir: "hlds_linux", engine: Engine::Hlds, windows: false, target: 0 },
    Root { name: "rehlds-win", dir: "hlds_win", engine: Engine::Hlds, windows: true, target: 2 },
    Root { name: "hlds25-linux", dir: "hlds25_linux", engine: Engine::Hlds, windows: false, target: 0 },
    Root { name: "hlds25-win", dir: "hlds25_win", engine: Engine::Hlds, windows: true, target: 2 },
];

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn wineprefix() -> PathBuf {
    std::env::var_os("WINEPREFIX").map(PathBuf::from).unwrap_or_else(|| home().join("games/wineprefix"))
}

impl Root {
    fn path(&self) -> PathBuf {
        let var = format!("TS_ROOT_{}", self.name.to_ascii_uppercase().replace('-', "_"));
        std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home().join("games").join(self.dir))
    }

    fn exists(&self) -> bool {
        self.path()
            .join(match (self.engine, self.windows) {
                (Engine::Fwgs, false) => "xash",
                (Engine::Fwgs, true) => "xash3d.exe",
                (Engine::Hlds, false) => "hlds_linux",
                (Engine::Hlds, true) => "hlds.exe",
            })
            .exists()
    }

    fn log_path(&self) -> PathBuf {
        self.path().join(if self.engine == Engine::Fwgs { "engine.log" } else { "qconsole.log" })
    }

    /// The port's dedicated server command (its `Root::server_command_with`).
    fn server(&self, map: &str, port: u16, cfg: &str) -> Command {
        let path = self.path();
        let mut c = match (self.engine, self.windows) {
            (Engine::Fwgs, true) => {
                let mut c = Command::new("wine");
                c.env("WINEPREFIX", wineprefix()).env("WINEDEBUG", "-all").args(["xash3d.exe", "-dedicated", "-dev", "1", "-log"]);
                c
            }
            (Engine::Fwgs, false) => {
                let mut c = Command::new(path.join("xash"));
                c.args(["-dev", "1"]);
                c
            }
            (Engine::Hlds, false) => {
                let mut c = Command::new(path.join("hlds_linux"));
                c.env("LD_LIBRARY_PATH", &path).arg("-condebug");
                c
            }
            (Engine::Hlds, true) => {
                let mut c = Command::new("wine");
                c.env("WINEPREFIX", wineprefix()).env("WINEDEBUG", "-all").args(["hlds.exe", "-console", "-condebug"]);
                c
            }
        };
        c.current_dir(&path).args(["-game", "ts", "-port", &port.to_string(), "+maxplayers", "8", "+sv_lan", "1"]);
        c.args(["+ts_bot_quota", "0", "+servercfgfile", cfg, "+map", map]);
        c
    }
}

/// Other `xtask` processes (the port's or ours) that would share the roots.
fn other_xtasks() -> Vec<String> {
    let Ok(out) = Command::new("pgrep").args(["-x", "xtask"]).output() else { return Vec::new() };
    let me = std::process::id().to_string();
    String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|p| !p.is_empty() && *p != me).map(String::from).collect()
}

/// The checks on one run's log.
pub fn checks(text: &str, file: &str, up: bool) -> Vec<(String, bool)> {
    let mut c: Vec<(String, bool)> = vec![
        ("server started".into(), up),
        (format!("{file} loaded"), text.lines().any(|l| l.contains("[ts] plugins:") && l.contains(&format!("loaded {file}")))),
        ("backend rcbot selected".into(), text.contains("[ts] bots: backend rcbot")),
        ("waypoints loaded for ts_dojo".into(), text.lines().any(|l| l.contains("[tsmod_rcbot]") && l.contains("waypoints for ts_dojo"))),
        (
            "no panic or crash".into(),
            !(text.contains("panicked") || text.contains("PANIC") || text.contains("FATAL ERROR") || text.contains("Segmentation")),
        ),
    ];
    for what in ["data_dir", "backend", "create_bot", "run_cmd", "moved", "waypoints"] {
        c.push((format!("check {what}"), text.contains(&format!("[tsmod_rcbot] check {what} ok"))));
    }
    c
}

fn one(root: &Root, port: u16, repo: &Path) -> Result<bool, String> {
    let t: &Target = &TARGETS[root.target];
    let gamedir = root.path().join("ts");
    let dlls = gamedir.join("dlls");
    let dist = repo.join("dist").join(t.platform);
    std::fs::copy(dist.join("dlls").join(t.file), dlls.join(t.file)).map_err(|e| format!("{}: {e}", dlls.display()))?;
    copy_dir(&dist.join("tsmod/rcbot"), &gamedir.join("tsmod/rcbot"))?;
    let map = "ts_dojo";
    std::fs::write(
        gamedir.join("xtask_rcbot.cfg"),
        format!("// Written by tsmod-rcbot-rs cargo xtask e2e.\nexec server.cfg\n// server.cfg ends without a newline: this line is the one it swallows\nts_bot_backend rcbot\nservercfgfile xtask_rcbot2.cfg\nchangelevel {map}\n"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(gamedir.join("xtask_rcbot2.cfg"), "// Written by tsmod-rcbot-rs cargo xtask e2e.\nts_bot_quota 3\naddbot\n")
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(root.log_path());
    let out_path = repo.join("target").join(format!("e2e-{}.log", root.name));
    let out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
    let mut cmd = root.server(map, port, "xtask_rcbot.cfg");
    cmd.stdin(Stdio::null()).stdout(out.try_clone().map_err(|e| e.to_string())?).stderr(out);
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", root.name))?;
    let start = Instant::now();
    let limit = Duration::from_secs(if root.windows { 100 } else { 50 });
    let mut up = true;
    while start.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(Some(_)) = child.try_wait() {
            up = false;
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(dlls.join(t.file));
    let mut text = std::fs::read(root.log_path()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    text.push_str(&std::fs::read(&out_path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default());
    let text = text.replace('\r', "");
    let mut report = format!("== {}: tsmod_rcbot on {map}, port {port}\n", root.name);
    for l in text.lines().filter(|l| l.contains("[tsmod_rcbot]") || l.contains("[ts] bots: backend") || l.contains("tsmod_rcbot")).take(60)
    {
        report += &format!("   | {}\n", l.trim_end());
    }
    let c = checks(&text, t.file, up);
    for (what, ok) in &c {
        report += &format!("   {} {what}\n", if *ok { "PASS" } else { "FAIL" });
    }
    let ok = c.iter().all(|(_, ok)| *ok);
    report += &format!("   {}\n", if ok { "PASS" } else { "FAIL" });
    print!("{report}");
    Ok(ok)
}

/// Runs the scenario on `names` (all existing roots by default).
pub fn run(repo: &Path, names: &[String]) -> Result<(), String> {
    let others = other_xtasks();
    if !others.is_empty() {
        return Err(format!("another xtask is running (pid {}); wait for it, the roots are shared", others.join(", ")));
    }
    let roots: Vec<&Root> = ROOTS.iter().filter(|r| names.is_empty() || names.iter().any(|n| n == r.name)).collect();
    let port_dir = std::env::var_os("TS_PORT_DIR").map(PathBuf::from).unwrap_or_else(|| repo.join("../rust-hlmod-the-specialists"));
    let mut present = Vec::new();
    for r in roots {
        if !r.exists() {
            println!("== {}: skipped ({} missing)", r.name, r.path().display());
            continue;
        }
        crate::run(
            Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
                .current_dir(&port_dir)
                .args(["xtask", "install", r.name]),
        )?;
        present.push(r);
    }
    let results: Vec<(&str, Result<bool, String>)> = std::thread::scope(|s| {
        let hs: Vec<_> = present.iter().enumerate().map(|(i, r)| (r.name, s.spawn(move || one(r, 27615 + i as u16, repo)))).collect();
        hs.into_iter().map(|(n, h)| (n, h.join().unwrap_or_else(|_| Err("panicked".into())))).collect()
    });
    if present.iter().any(|r| r.windows) && other_xtasks().is_empty() {
        let _ = Command::new("wineserver").env("WINEPREFIX", wineprefix()).arg("-k").status();
    }
    let mut failed = Vec::new();
    for (name, r) in results {
        match r {
            Ok(true) => {}
            Ok(false) => failed.push(name.to_owned()),
            Err(e) => failed.push(format!("{name} ({e})")),
        }
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("e2e failed on {}", failed.join(", "))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passing_log() {
        let mut log = String::from("[ts] plugins: loaded tsmod_rcbot_amd64.so (RCBot 0.1)\n[ts] bots: backend rcbot\n");
        log += "[tsmod_rcbot] loaded 154 waypoints for ts_dojo (visibility rows from 23)\n";
        for c in ["data_dir", "backend", "create_bot", "run_cmd", "moved", "waypoints"] {
            log += &format!("[tsmod_rcbot] check {c} ok\n");
        }
        assert!(checks(&log, "tsmod_rcbot_amd64.so", true).iter().all(|(_, ok)| *ok));
        assert!(checks(&format!("{log}Segmentation fault\n"), "tsmod_rcbot_amd64.so", true).iter().any(|(_, ok)| !ok));
    }
}
