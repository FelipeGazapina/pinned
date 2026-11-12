//! Keeps the computer from sleeping while the user asks for it: `caffeinate` on macOS,
//! `systemd-inhibit` on Linux, SetThreadExecutionState on Windows.

use std::path::PathBuf;
use std::sync::Mutex;

pub struct Awake {
    hold: Mutex<Option<Hold>>,
    /// Remembers the choice across launches: the file exists while the toggle is on.
    flag: Option<PathBuf>,
}

impl Awake {
    pub fn load(config_dir: Option<PathBuf>) -> Self {
        let awake = Awake { hold: Mutex::new(None), flag: config_dir.map(|c| c.join("keep-awake")) };
        if awake.flag.as_ref().is_some_and(|f| f.exists()) {
            let _ = awake.set(true);
        }
        awake
    }

    pub fn is_on(&self) -> bool {
        self.hold.lock().unwrap().is_some()
    }

    pub fn set(&self, on: bool) -> Result<(), String> {
        let mut hold = self.hold.lock().unwrap();
        if on == hold.is_some() {
            return Ok(());
        }
        *hold = if on { Some(Hold::start()?) } else { None };
        if let Some(flag) = &self.flag {
            if on {
                if let Some(dir) = flag.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(flag, "");
            } else {
                let _ = std::fs::remove_file(flag);
            }
        }
        Ok(())
    }
}

/// Keeps the computer awake until dropped.
#[cfg(not(any(windows, target_os = "linux")))]
struct Hold(std::process::Child);

#[cfg(not(any(windows, target_os = "linux")))]
impl Hold {
    /// `-d` keeps the display on, `-i` the system; `-w` ends it when pinned exits, however that happens.
    fn start() -> Result<Hold, String> {
        pinned_core::process::command("caffeinate")
            .args(["-d", "-i", "-w", &std::process::id().to_string()])
            .spawn()
            .map(Hold)
            .map_err(|e| format!("could not run caffeinate: {e}"))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
impl Drop for Hold {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Keeps the computer awake until dropped: systemd-inhibit holds logind's sleep and idle locks
/// while its child runs. It runs in a process group of its own, so dropping the hold ends the
/// child too, and `tail --pid` ends that child when pinned exits, however that happens.
#[cfg(target_os = "linux")]
struct Hold(std::process::Child);

#[cfg(target_os = "linux")]
impl Hold {
    fn start() -> Result<Hold, String> {
        use std::io::Read;
        use std::os::unix::process::CommandExt;
        let mut child = pinned_core::process::command("systemd-inhibit")
            .args(["--what=sleep:idle", "--mode=block", "--who=pinned", "--why=Keep awake is on", "tail", &format!("--pid={}", std::process::id()), "-f", "/dev/null"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "Keep awake needs systemd-inhibit on Linux".to_string(),
                _ => format!("could not run systemd-inhibit: {e}"),
            })?;
        // With no logind, or a policy that refuses the lock, it ends at once.
        std::thread::sleep(std::time::Duration::from_millis(300));
        if let Ok(Some(status)) = child.try_wait() {
            let mut why = String::new();
            if let Some(mut err) = child.stderr.take() {
                let _ = err.read_to_string(&mut why);
            }
            return Err(format!("systemd-inhibit would not keep the computer awake ({status}): {}", why.trim()));
        }
        Ok(Hold(child))
    }
}

#[cfg(target_os = "linux")]
impl Drop for Hold {
    fn drop(&mut self) {
        if let Ok(group) = i32::try_from(self.0.id()) {
            unsafe { libc::kill(-group, libc::SIGTERM) };
        }
        let _ = self.0.wait();
    }
}

/// Keeps the computer awake until dropped. Windows ties the request to the thread that made it,
/// and ends it with that thread, so a thread of its own holds it until the sender goes.
#[cfg(windows)]
struct Hold {
    _release: std::sync::mpsc::Sender<()>,
}

#[cfg(windows)]
impl Hold {
    fn start() -> Result<Hold, String> {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};
        let (release, released) = std::sync::mpsc::channel::<()>();
        let (started, start) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let held = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) } != 0;
            let _ = started.send(held);
            if held {
                let _ = released.recv();
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            }
        });
        match start.recv() {
            Ok(true) => Ok(Hold { _release: release }),
            _ => Err("Windows would not keep the computer awake".into()),
        }
    }
}

#[tauri::command]
pub fn get_keep_awake(awake: tauri::State<Awake>) -> bool {
    awake.is_on()
}

#[tauri::command]
pub fn set_keep_awake(awake: tauri::State<Awake>, on: bool) -> Result<(), String> {
    awake.set(on)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn caffeinating() -> bool {
        let out = pinned_core::process::command("pgrep").args(["-f", &format!("caffeinate -d -i -w {}", std::process::id())]).output().unwrap();
        out.status.success()
    }

    #[test]
    fn the_toggle_runs_and_stops_caffeinate_and_is_remembered() {
        let dir = std::env::temp_dir().join(format!("pinned-awake-test-{}", std::process::id()));
        let awake = Awake::load(Some(dir.clone()));
        assert!(!awake.is_on() && !caffeinating());

        awake.set(true).unwrap();
        assert!(awake.is_on() && caffeinating() && dir.join("keep-awake").exists());

        // Next launch picks the choice back up; this one gives the assertion up first.
        awake.set(false).unwrap();
        assert!(!awake.is_on() && !caffeinating() && !dir.join("keep-awake").exists());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep-awake"), "").unwrap();
        let relaunched = Awake::load(Some(dir.clone()));
        assert!(relaunched.is_on() && caffeinating());
        relaunched.set(false).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    /// Runners may have no logind session to take the lock from, so a refusal must name the tool.
    #[test]
    fn the_hold_takes_an_inhibitor_and_leaves_nothing_running_or_says_why_not() {
        match Hold::start() {
            Err(why) => {
                eprintln!("no inhibitor here: {why}");
                assert!(why.contains("systemd-inhibit"), "{why}");
            }
            Ok(hold) => {
                let group = hold.0.id().to_string();
                let running = || pinned_core::process::command("pgrep").args(["-g", &group]).output().unwrap().status.success();
                assert!(running());
                drop(hold);
                std::thread::sleep(std::time::Duration::from_millis(200));
                assert!(!running(), "the inhibitor's group outlived the hold");
            }
        }
    }
}
