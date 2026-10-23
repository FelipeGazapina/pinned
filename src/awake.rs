//! Keeps the Mac from sleeping while the user asks for it, by running `caffeinate`.

use std::path::PathBuf;
use pinned_core::process;
use std::process::Child;
use std::sync::Mutex;

pub struct Awake {
    child: Mutex<Option<Child>>,
    /// Remembers the choice across launches: the file exists while the toggle is on.
    flag: Option<PathBuf>,
}

impl Awake {
    pub fn load(config_dir: Option<PathBuf>) -> Self {
        let awake = Awake { child: Mutex::new(None), flag: config_dir.map(|c| c.join("keep-awake")) };
        if awake.flag.as_ref().is_some_and(|f| f.exists()) {
            let _ = awake.set(true);
        }
        awake
    }

    pub fn is_on(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }

    /// `-d` keeps the display on, `-i` the system; `-w` ends it when pinned exits, however that happens.
    pub fn set(&self, on: bool) -> Result<(), String> {
        let mut child = self.child.lock().unwrap();
        if on == child.is_some() {
            return Ok(());
        }
        match child.take() {
            Some(mut running) => {
                let _ = running.kill();
                let _ = running.wait();
            }
            None => {
                let started = process::command("caffeinate")
                    .args(["-d", "-i", "-w", &std::process::id().to_string()])
                    .spawn()
                    .map_err(|e| format!("could not run caffeinate: {e}"))?;
                *child = Some(started);
            }
        }
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
        let out = process::command("pgrep").args(["-f", &format!("caffeinate -d -i -w {}", std::process::id())]).output().unwrap();
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
