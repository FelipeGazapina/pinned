//! Keeps the computer from sleeping while the user asks for it: `caffeinate` on macOS,
//! SetThreadExecutionState on Windows.

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
#[cfg(not(windows))]
struct Hold(std::process::Child);

#[cfg(not(windows))]
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

#[cfg(not(windows))]
impl Drop for Hold {
    fn drop(&mut self) {
        let _ = self.0.kill();
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
