//! Composer dictation: a local, on-device speech recognizer (macOS Speech
//! framework via the `dictation-helper` Swift sidecar) that streams interim +
//! final transcripts to the UI as Tauri events.
//!
//! The UI owns the UX contract: interim text is provisional, each final result
//! confirms one phrase, typing confirms the provisional text, and the session
//! ends only when the message is sent or the mic is tapped again. This module
//! only moves audio -> text events and never stops on silence itself.

use pinned_core::process;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

const EVENT_RESULT: &str = "dictation://result";
const EVENT_STATUS: &str = "dictation://status";
const EVENT_NOTICE: &str = "dictation://notice";
/// Bounds on the expected-words list the webview hands the helper.
const VOCABULARY_WORDS: usize = 200;
const VOCABULARY_WORD_CHARS: usize = 64;

#[derive(Clone, serde::Serialize)]
struct DictationResult {
    text: String,
    #[serde(rename = "isFinal")]
    is_final: bool,
}

#[derive(Clone, serde::Serialize)]
struct DictationNotice {
    message: String,
}

#[derive(Clone, serde::Serialize)]
struct DictationStatus {
    running: bool,
    error: Option<String>,
}

struct Running {
    #[allow(dead_code)]
    child: Child,
    stdin: ChildStdin,
}

pub struct Dictation {
    app: AppHandle,
    running: Mutex<Option<Running>>,
}

impl Dictation {
    pub fn new(app: AppHandle) -> Self {
        Self { app, running: Mutex::new(None) }
    }

    fn helper_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
        let dir = app.path().resource_dir().map_err(|e| format!("could not find resources: {e}"))?;
        for name in ["dictation-helper", "binaries/dictation-helper"] {
            let p = dir.join(name);
            if p.exists() {
                return Ok(p);
            }
        }
        // Dev fallback: next to the built binary (build.rs compiles it there too).
        let exe = std::env::current_exe().map_err(|e| format!("could not find pinned: {e}"))?;
        if let Some(dir) = exe.parent() {
            let p = dir.join("dictation-helper");
            if p.exists() {
                return Ok(p);
            }
        }
        if cfg!(target_os = "macos") {
            Err("dictation helper not bundled with this build".into())
        } else {
            Err("Dictation needs macOS".into())
        }
    }

    pub fn running(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    pub fn start(&self, locale: Option<String>, vocabulary: &[String]) -> Result<(), String> {
        if self.running() {
            return Ok(());
        }
        // Reap a previous child that exited on its own (e.g. helper error).
        self.stop_silent();
        let path = Self::helper_path(&self.app)?;
        let locale = locale.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| "en-US".into());
        if !valid_locale(&locale) {
            return Err(format!("{locale:?} is not a locale name"));
        }
        let mut child = process::command(&path)
            .arg(&locale)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start dictation: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("could not pipe to dictation")?;
        if let Err(e) = stdin.write_all(config_line(vocabulary).as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("could not configure dictation: {e}"));
        }
        let stdout = child.stdout.take().ok_or("could not read dictation")?;
        let app = self.app.clone();
        std::thread::spawn(move || pump(&app, stdout));
        *self.running.lock().unwrap() = Some(Running { child, stdin });
        emit_status(&self.app, true, None);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        self.stop_silent();
        emit_status(&self.app, false, None);
        Ok(())
    }

    fn stop_silent(&self) {
        if let Some(mut r) = self.running.lock().unwrap().take() {
            let _ = r.stdin.write_all(b"{\"cmd\":\"stop\"}\n");
            let _ = r.stdin.flush();
            // Don't wait: the helper exits by itself; a stuck mic must not block the UI.
            std::thread::spawn(move || {
                let _ = r.child.wait();
            });
        }
    }
}

fn valid_locale(locale: &str) -> bool {
    !locale.is_empty()
        && locale.len() <= 20
        && locale.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The helper's first stdin line: `{"vocabulary":[...]}`, cut to the bounds above.
fn config_line(vocabulary: &[String]) -> String {
    let words: Vec<&str> = vocabulary
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty() && w.chars().count() <= VOCABULARY_WORD_CHARS && !w.chars().any(char::is_control))
        .take(VOCABULARY_WORDS)
        .collect();
    format!("{}\n", serde_json::json!({ "vocabulary": words }))
}

fn pump(app: &AppHandle, stdout: std::process::ChildStdout) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        match value.get("type").and_then(|t| t.as_str()) {
            Some("result") => {
                let text = value.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let is_final = value.get("isFinal").and_then(|f| f.as_bool()).unwrap_or(false);
                if !text.is_empty() {
                    let _ = app.emit(EVENT_RESULT, DictationResult { text, is_final });
                }
            }
            Some("notice") => {
                if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
                    let _ = app.emit(EVENT_NOTICE, DictationNotice { message: message.to_string() });
                }
            }
            Some("error") => {
                let message = value.get("message").and_then(|m| m.as_str()).unwrap_or("dictation failed").to_string();
                emit_status(app, false, Some(message));
                break;
            }
            _ => {}
        }
    }
}

fn emit_status(app: &AppHandle, running: bool, error: Option<String>) {
    let _ = app.emit(EVENT_STATUS, DictationStatus { running, error });
}

#[tauri::command]
pub fn dictation_start(dictation: State<Arc<Dictation>>, locale: Option<String>, vocabulary: Option<Vec<String>>) -> Result<(), String> {
    dictation.start(locale, &vocabulary.unwrap_or_default())
}

#[tauri::command]
pub fn dictation_stop(dictation: State<Arc<Dictation>>) -> Result<(), String> {
    dictation.stop()
}

#[tauri::command]
pub fn dictation_status(dictation: State<Arc<Dictation>>) -> bool {
    dictation.running()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_line_carries_the_vocabulary_trimmed() {
        let words = vec![" commit ".to_string(), "pull request".into()];
        assert_eq!(config_line(&words), "{\"vocabulary\":[\"commit\",\"pull request\"]}\n");
    }

    #[test]
    fn the_config_line_drops_blank_long_and_control_words() {
        let words = vec!["".to_string(), "x".repeat(65), "two\nlines".into(), "ok".into(), "y".repeat(64)];
        assert_eq!(config_line(&words), format!("{{\"vocabulary\":[\"ok\",\"{}\"]}}\n", "y".repeat(64)));
    }

    #[test]
    fn the_config_line_keeps_at_most_the_bound() {
        let words: Vec<String> = (0..500).map(|i| format!("w{i}")).collect();
        let line = config_line(&words);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["vocabulary"].as_array().unwrap().len(), 200);
        assert_eq!(value["vocabulary"][199], "w199");
    }
}
