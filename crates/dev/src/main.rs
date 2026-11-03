//! Runs the desktop app in a throwaway home and rebuilds and restarts it when the app changes.
//!
//! `cargo run -p pinned-dev`
//!
//! The app keeps its config under `.dev/home`, so this build never shares accounts, projects,
//! sessions or history with the installed app. It talks to the hub only after you sign in here.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn main() -> io::Result<()> {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/dev is under the repo")
        .to_owned();
    let root = repo.join(".dev");
    let home = root.join("home");
    let log = root.join("dev.log");
    fs::create_dir_all(&home)?;
    let cargo = cargo();
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home().join(".cargo"));
    let rustup_home = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home().join(".rustup"));
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target"));
    let bin = target.join("debug").join(if cfg!(windows) {
        "pinned.exe"
    } else {
        "pinned"
    });

    let watched = watch_paths(&repo);
    println!(
        "[dev] watching {}, restarting on change",
        watched
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("[dev] isolated HOME {}", home.display());
    println!("[dev] log {}", log.display());

    let mut app = None;
    let mut last = snapshot(&watched);
    let mut pending = None;
    if build(&cargo, &repo, &home, &cargo_home, &rustup_home, &log)?.success() {
        start(&bin, &home, &log, &mut app);
    } else {
        println!("[dev] build failed, waiting for a change");
    }
    while !STOP.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(400));
        if let Some(child) = app.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                println!("[dev] app exited ({status}), waiting for a change");
                app = None;
            }
        }
        let now = snapshot(&watched);
        if now != last {
            last = now;
            pending = Some(Instant::now());
        } else if pending.is_some_and(|at| at.elapsed() >= Duration::from_millis(350)) {
            pending = None;
            println!("[dev] change detected, rebuilding");
            stop_app(&mut app);
            if STOP.load(Ordering::SeqCst) {
                break;
            }
            if build(&cargo, &repo, &home, &cargo_home, &rustup_home, &log)?.success() {
                start(&bin, &home, &log, &mut app);
            } else {
                println!("[dev] build failed, waiting for a change");
            }
        }
    }
    stop_app(&mut app);
    Ok(())
}

fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn cargo() -> PathBuf {
    if let Some(cargo) = std::env::var_os("CARGO") {
        return cargo.into();
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let cargo = dir.join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
            if cargo.is_file() {
                return cargo;
            }
        }
    }
    let toolchains = user_home().join(".rustup").join("toolchains");
    if let Ok(entries) = fs::read_dir(toolchains) {
        for entry in entries.flatten() {
            let cargo =
                entry
                    .path()
                    .join("bin")
                    .join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
            if cargo.is_file() {
                return cargo;
            }
        }
    }
    "cargo".into()
}

fn watch_paths(repo: &Path) -> Vec<PathBuf> {
    [
        "src",
        "crates",
        "ui",
        "build.rs",
        "Cargo.toml",
        "Cargo.lock",
        "tauri.conf.json",
        "capabilities",
    ]
    .into_iter()
    .map(|name| repo.join(name))
    .filter(|path| path.exists())
    .collect()
}

/// Every watched file's path, length and modification time, sorted. A new or removed file changes it.
fn snapshot(paths: &[PathBuf]) -> Vec<(PathBuf, u128, u64)> {
    let mut files = vec![];
    for path in paths {
        walk(path, &mut files, &mut HashSet::new());
    }
    files.sort();
    files
}

fn walk(path: &Path, out: &mut Vec<(PathBuf, u128, u64)>, seen: &mut HashSet<PathBuf>) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_dir() {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if matches!(name, "target" | "node_modules" | ".git" | ".dev" | "gen") {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            walk(&entry.path(), out, seen);
        }
    } else if meta.is_file() && seen.insert(path.to_owned()) {
        let modified = meta
            .modified()
            .unwrap_or(UNIX_EPOCH)
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        out.push((path.to_owned(), modified, meta.len()));
    }
}

fn build(
    cargo: &Path,
    repo: &Path,
    home: &Path,
    cargo_home: &Path,
    rustup_home: &Path,
    log: &Path,
) -> io::Result<std::process::ExitStatus> {
    use std::io::Write;
    let mut file = OpenOptions::new().create(true).append(true).open(log)?;
    writeln!(file, "\n[dev] {} build", stamp())?;
    let out = file.try_clone()?;
    let err = file.try_clone()?;
    Command::new(cargo)
        .args(["build", "-p", "pinned"])
        .current_dir(repo)
        .env("HOME", home)
        .env("CARGO_HOME", cargo_home)
        .env("RUSTUP_HOME", rustup_home)
        .stdout(out)
        .stderr(err)
        .status()
}

fn start(bin: &Path, home: &Path, log: &Path, app: &mut Option<Child>) {
    let mut cmd = Command::new(bin);
    cmd.env("HOME", home);
    match OpenOptions::new().create(true).append(true).open(log) {
        Ok(file) => {
            if let Ok(err) = file.try_clone() {
                cmd.stdout(file).stderr(err);
            }
        }
        Err(_) => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    match cmd.spawn() {
        Ok(child) => {
            println!("[dev] app started, pid {}", child.id());
            *app = Some(child);
        }
        Err(error) => println!("[dev] could not start {}: {error}", bin.display()),
    }
}

fn stop_app(app: &mut Option<Child>) {
    let Some(mut child) = app.take() else {
        return;
    };
    #[cfg(unix)]
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn stamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}s", now.as_secs(), now.subsec_millis())
}
