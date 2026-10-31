use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tokio::sync::Notify;

pub const PROOF_ENV: &str = "PINNED_PROOF_BOOT";
pub const MARKER_ENV: &str = "PINNED_UNPROVEN";
pub const VERSION_ENV: &str = "PINNED_VERSION";
pub const WAIT_ENV: &str = "PINNED_PROOF_WAIT";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Starting,
    Running,
    Leaving,
}

static STEP: Mutex<Step> = Mutex::new(Step::Starting);
static REFUSED: Notify = Notify::const_new();

fn step() -> MutexGuard<'static, Step> {
    STEP.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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

pub fn proving() -> bool {
    let Some(path) = marker_path() else { return false };
    let Ok(marker) = std::fs::read(path) else { return false };
    proves(&marker, &std::env::var(PROOF_ENV).unwrap_or_default())
}

fn proves(marker: &[u8], proof: &str) -> bool {
    !proof.is_empty() && String::from_utf8_lossy(marker).trim() == proof
}

pub fn ready() {
    let mut step = step();
    if *step == Step::Leaving {
        drop(step);
        loop {
            std::thread::park();
        }
    }
    *step = Step::Running;
}

pub fn leave_if_starting() -> bool {
    leave_from(Step::Starting)
}

pub fn leave_if_running() -> bool {
    leave_from(Step::Running)
}

fn leave_from(from: Step) -> bool {
    let mut step = step();
    if *step != from || !proving() {
        return false;
    }
    *step = Step::Leaving;
    true
}

pub fn commit_if_proving() {
    let step = step();
    if *step == Step::Leaving || !proving() {
        return;
    }
    let Some(path) = marker_path() else { return };
    if let Err(error) = std::fs::remove_file(&path) {
        eprintln!("pinned: could not remove {}, the next sign in tries again: {error}", path.display());
    }
}

pub fn refuse() {
    if proving() {
        REFUSED.notify_one();
    }
}

pub async fn refused() {
    REFUSED.notified().await
}

fn sibling(path: &Path, suffix: &str) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy();
    Some(path.with_file_name(format!("{name}.{suffix}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_checksum_the_marker_records_proves_it() {
        let marker = b"9f86d081884c7d65\n";
        assert!(proves(marker, "9f86d081884c7d65"));
        assert!(!proves(marker, ""), "a process with no proof matched the marker");
        assert!(!proves(marker, "2c26b46b68ffc68f"), "a proof of other bytes matched the marker");
        assert!(!proves(b"", ""), "an empty marker matched no proof");
    }
}
