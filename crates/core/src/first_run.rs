//! Claude Code's first-run screens: the theme picker, the login picker, the folder-trust prompt
//! and the bypass-permissions warning. Each one holds a session in "starting" until someone
//! answers it, and a session no one watches never gets answered.
//!
//! Pinned prevents them by seeding the launching account's config before the session starts, and
//! falls back to reading the session's screen when one shows anyway.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The theme a seeded config gets when it names none. It is the picker's own default.
const THEME: &str = "dark";

/// Marks onboarding done, with a theme when none is set, and `trust` as a trusted folder, in the
/// account config at `file`. Every other key is kept. Writes nothing when all of it is already
/// there, so the file a running claude rewrites is touched only when something is missing.
pub fn seed(file: &Path, trust: Option<&Path>) -> Result<(), String> {
    let mut value = match std::fs::read(file) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(|e| format!("could not read {}: {e}", file.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(e) => return Err(format!("could not read {}: {e}", file.display())),
    };
    let object = value.as_object_mut().ok_or_else(|| format!("{} is not a JSON object", file.display()))?;
    let mut changed = false;
    if object.get("hasCompletedOnboarding") != Some(&Value::Bool(true)) {
        object.insert("hasCompletedOnboarding".into(), Value::Bool(true));
        changed = true;
    }
    if !object.get("theme").is_some_and(Value::is_string) {
        object.insert("theme".into(), Value::String(THEME.into()));
        changed = true;
    }
    if let Some(folder) = trust {
        let projects = object.entry("projects").or_insert_with(|| Value::Object(Default::default()));
        let projects = projects.as_object_mut().ok_or_else(|| format!("{} has a projects entry that is not an object", file.display()))?;
        let entry = projects.entry(key(folder)).or_insert_with(|| Value::Object(Default::default()));
        let entry = entry.as_object_mut().ok_or_else(|| format!("{} has a project entry that is not an object", file.display()))?;
        if entry.get("hasTrustDialogAccepted") != Some(&Value::Bool(true)) {
            entry.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    crate::fsx::replace(file, &bytes, None, None)
}

/// The folder as claude names it in its `projects` map: the path its own working directory
/// resolves to.
fn key(folder: &Path) -> String {
    dunce::canonicalize(folder).unwrap_or_else(|_| folder.to_owned()).display().to_string()
}

/// The folders pinned cloned or made, the only ones it marks trusted for a session. A folder the
/// user opened by hand stays the user's call.
pub struct Made {
    file: Option<PathBuf>,
    folders: Mutex<BTreeSet<PathBuf>>,
}

impl Made {
    /// The folders kept in `file`, minus those gone since.
    pub fn load(file: Option<PathBuf>) -> Made {
        let folders: BTreeSet<PathBuf> = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|folder| folder.is_dir())
            .collect();
        Made { file, folders: Mutex::new(folders) }
    }

    pub fn add(&self, folder: &Path) {
        let mut folders = self.folders.lock().unwrap();
        if !folders.insert(folder.to_owned()) {
            return;
        }
        let Some(file) = &self.file else { return };
        let bytes = serde_json::to_vec_pretty(&*folders).unwrap_or_default();
        if let Err(e) = crate::fsx::replace(file, &bytes, None, None) {
            eprintln!("pinned: the folders pinned made did not save: {e}");
        }
    }

    /// Whether `folder` is one pinned made, or inside one.
    pub fn covers(&self, folder: &Path) -> bool {
        self.folders.lock().unwrap().iter().any(|made| folder.starts_with(made))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Theme,
    Login,
    Trust,
    Bypass,
}

/// Words only each screen shows, as `flat` spells them.
const SCREENS: [(Screen, &[&str]); 5] = [
    (Screen::Theme, &["choosethetextstylethatlooksbestwithyourterminal"]),
    (Screen::Login, &["selectloginmethod"]),
    (Screen::Trust, &["yesitrustthisfolder"]),
    (Screen::Trust, &["doyoutrustthefilesinthisfolder"]),
    (Screen::Bypass, &["bypasspermissionsmode", "yesiaccept"]),
];

/// The first-run screen the session shows now: of those its output drew, the one drawn last.
pub fn showing(screen: &str) -> Option<Screen> {
    let flat = flat(screen);
    SCREENS
        .iter()
        .filter_map(|(kind, words)| {
            let at = words.iter().map(|word| flat.rfind(word)).collect::<Option<Vec<_>>>()?;
            Some((at.into_iter().max()?, *kind))
        })
        .max_by_key(|(at, _)| *at)
        .map(|(_, kind)| kind)
}

/// The keys that answer `screen`, typed one at a time, or why the start can't go on without the
/// user. Trust is answered only for a folder pinned made, and the bypass warning only for a
/// session pinned started with bypass.
pub fn answer(screen: Screen, folder: &Path, made: bool, bypass: bool) -> Result<&'static [&'static str], String> {
    const DOWN_ENTER: &[&str] = &["\u{1b}[B", "\r"];
    match screen {
        Screen::Theme => Ok(&["\r"]),
        Screen::Trust if made => Ok(DOWN_ENTER),
        Screen::Bypass if bypass => Ok(DOWN_ENTER),
        Screen::Trust => Err(format!("Claude Code is waiting on its folder-trust prompt in {}; open the session to answer it", folder.display())),
        Screen::Bypass => Err("Claude Code is waiting on its bypass-permissions warning; open the session to answer it".into()),
        Screen::Login => Err("Claude Code is not signed in for this account; sign in under Settings > Accounts, then start the session again".into()),
    }
}

/// The screen's letters and digits, lowercased, with terminal commands removed. Claude's TUI
/// places each word with a cursor command, so a sentence is not contiguous in the pty bytes.
pub fn flat(screen: &str) -> String {
    let mut flat = String::new();
    let mut chars = screen.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\u{7}' || next == '\u{1b}' {
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
            continue;
        }
        if c.is_ascii_alphanumeric() {
            flat.push(c.to_ascii_lowercase());
        }
    }
    flat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-first-run-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn seeding_keeps_every_other_key_and_trusts_only_the_folder_named() {
        let dir = temp("seed");
        let file = dir.join(".claude.json");
        let (made, other) = (dir.join("made"), dir.join("other"));
        std::fs::create_dir_all(&made).unwrap();
        std::fs::write(&file, r#"{"numStartups":7,"projects":{"/elsewhere":{"hasTrustDialogAccepted":false,"allowedTools":["Bash"]}}}"#).unwrap();
        seed(&file, Some(&made)).unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(value["numStartups"], 7);
        assert_eq!(value["hasCompletedOnboarding"], true);
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["projects"][key(&made)]["hasTrustDialogAccepted"], true);
        assert_eq!(value["projects"]["/elsewhere"], serde_json::json!({"hasTrustDialogAccepted": false, "allowedTools": ["Bash"]}));
        assert!(value["projects"].get(key(&other)).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_seeded_config_is_not_written_again() {
        let dir = temp("seed-again");
        let file = dir.join(".claude.json");
        std::fs::write(&file, r#"{"hasCompletedOnboarding":true,"theme":"light"}"#).unwrap();
        let before = std::fs::read(&file).unwrap();
        seed(&file, None).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), before, "a config that needs nothing is left byte for byte");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_screen_drawn_last_is_the_one_showing() {
        let theme = "\u{1b}[2K Choose the text style\u{1b}[1Cthat looks best with your terminal";
        let trust = " Quick safety check \u{1b}[3;1H❯ No, exit\r\n Yes, I\u{1b}[1Ctrust this folder";
        let bypass = "WARNING: Claude Code running in Bypass Permissions mode ... ❯ No, exit  Yes, I accept";
        assert_eq!(showing(theme), Some(Screen::Theme));
        assert_eq!(showing(&format!("{theme}{trust}")), Some(Screen::Trust));
        assert_eq!(showing(&format!("{trust}{bypass}")), Some(Screen::Bypass));
        assert_eq!(showing(" Select login method: ❯ 1. Claude account"), Some(Screen::Login));
        assert_eq!(showing("❯ \r\n ⏵⏵ bypass permissions on (shift+tab to cycle)"), None, "the prompt line is no first-run screen");
    }

    #[test]
    fn only_safe_screens_are_answered() {
        let folder = Path::new("/w/app");
        assert_eq!(answer(Screen::Theme, folder, false, false), Ok(&["\r"][..]));
        assert_eq!(answer(Screen::Trust, folder, true, false), Ok(&["\u{1b}[B", "\r"][..]));
        assert_eq!(answer(Screen::Trust, folder, false, true), Err("Claude Code is waiting on its folder-trust prompt in /w/app; open the session to answer it".into()));
        assert!(answer(Screen::Bypass, folder, true, false).is_err(), "a session started without bypass never accepts the warning");
        assert_eq!(answer(Screen::Bypass, folder, false, true), Ok(&["\u{1b}[B", "\r"][..]));
        assert!(answer(Screen::Login, folder, true, true).is_err());
    }
}
