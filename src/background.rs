//! Run in the background: the app keeps its sessions, its hub link and everything the web app asks
//! of it, with no window at all. The window's webview is destroyed rather than hidden, since the
//! point is the memory it holds.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// What a switch says when the app is not signed in to a hub.
pub const NEEDS_HUB: &str = "sign in to a hub first: with no window, the web app is how you reach this machine";

/// The setting, remembered across launches by a file in the app's config folder that exists while it is on.
pub struct Background {
    flag: Option<PathBuf>,
    on: AtomicBool,
}

impl Background {
    pub fn load(config_dir: Option<PathBuf>) -> Self {
        let flag = config_dir.map(|c| c.join("run-in-background"));
        let on = flag.as_ref().is_some_and(|f| f.exists());
        Background { flag, on: AtomicBool::new(on) }
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    pub fn save(&self, on: bool) -> Result<(), String> {
        if let Some(flag) = &self.flag {
            let written = match (on, flag.parent()) {
                (true, Some(dir)) => std::fs::create_dir_all(dir).and_then(|()| std::fs::write(flag, "")),
                (true, None) => std::fs::write(flag, ""),
                (false, _) => std::fs::remove_file(flag).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
            };
            written.map_err(|e| format!("could not save Run in the background: {e}"))?;
        }
        self.on.store(on, Ordering::SeqCst);
        Ok(())
    }
}

/// What asks the app whether it should have its window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Moment {
    /// A launch, the window closed, or the setting changed: the setting decides.
    Settle,
    /// The user opened the app again while it runs, from Finder, Spotlight, the Dock or the menu bar icon.
    Reopen,
}

/// Whether the app has its window after `moment`, with Run in the background `on`. Signed out, no
/// web app can reach the machine, so the setting holds only while `signed_in`. A reopen shows the
/// window for this run whatever the setting: a user whose hub is out of reach still gets it back.
pub fn has_window(moment: Moment, on: bool, signed_in: bool) -> bool {
    moment == Moment::Reopen || !(on && signed_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-background-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_setting_holds_across_launches() {
        let dir = config();
        assert!(!Background::load(Some(dir.clone())).is_on(), "off until turned on");

        Background::load(Some(dir.clone())).save(true).unwrap();
        let relaunched = Background::load(Some(dir.clone()));
        assert!(relaunched.is_on(), "a launch reads it back on");

        relaunched.save(false).unwrap();
        assert!(!relaunched.is_on());
        assert!(!Background::load(Some(dir.clone())).is_on(), "a launch reads it back off");
        relaunched.save(false).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_config_folder_it_lasts_for_the_run() {
        let background = Background::load(None);
        background.save(true).unwrap();
        assert!(background.is_on());
    }

    #[test]
    fn a_launch_opens_no_window_only_when_on_and_signed_in() {
        let launch = |on, signed_in| has_window(Moment::Settle, on, signed_in);
        assert_eq!([launch(false, false), launch(false, true), launch(true, false), launch(true, true)], [true, true, true, false]);
    }

    #[test]
    fn a_reopen_always_shows_the_window() {
        let reopen = |on, signed_in| has_window(Moment::Reopen, on, signed_in);
        assert_eq!([reopen(false, false), reopen(false, true), reopen(true, false), reopen(true, true)], [true, true, true, true]);
    }
}
