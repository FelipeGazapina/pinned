use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const PROOF_ENV: &str = "PINNED_PROOF_BOOT";
pub const MARKER_ENV: &str = "PINNED_UNPROVEN";
pub const VERSION_ENV: &str = "PINNED_VERSION";
pub const WAIT_ENV: &str = "PINNED_PROOF_WAIT";

static GATE: Mutex<()> = Mutex::new(());

pub fn lock() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn marker_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(MARKER_ENV) {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let exe = std::env::current_exe().ok()?;
    sibling(&exe, "unproven")
}

pub fn commit() {
    let _gate = lock();
    if let Some(path) = marker_path() {
        let _ = std::fs::remove_file(path);
    }
}

fn sibling(path: &Path, suffix: &str) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy();
    Some(path.with_file_name(format!("{name}.{suffix}")))
}
