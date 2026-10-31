use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tokio::sync::Notify;

pub const PROOF_ENV: &str = "PINNED_PROOF_BOOT";
pub const MARKER_ENV: &str = "PINNED_UNPROVEN";
pub const VERSION_ENV: &str = "PINNED_VERSION";
pub const WAIT_ENV: &str = "PINNED_PROOF_WAIT";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Starting,
    Running,
    Leaving,
}

struct Boot {
    step: Mutex<Step>,
    refused: Notify,
}

static BOOT: Boot = Boot::new();

struct Proof {
    marker: Option<PathBuf>,
    checksum: String,
}

impl Proof {
    fn of_this_process() -> Proof {
        Proof { marker: marker_path(), checksum: std::env::var(PROOF_ENV).unwrap_or_default() }
    }

    fn holds(&self) -> bool {
        let Some(path) = &self.marker else { return false };
        let Ok(marker) = std::fs::read(path) else { return false };
        proves(&marker, &self.checksum)
    }
}

impl Boot {
    const fn new() -> Boot {
        Boot { step: Mutex::new(Step::Starting), refused: Notify::const_new() }
    }

    fn step(&self) -> MutexGuard<'_, Step> {
        self.step.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ready(&self) -> bool {
        let mut step = self.step();
        if *step == Step::Leaving {
            return false;
        }
        *step = Step::Running;
        true
    }

    fn leave_from(&self, from: Step, proof: &Proof) -> bool {
        let mut step = self.step();
        if *step != from || !proof.holds() {
            return false;
        }
        *step = Step::Leaving;
        true
    }

    async fn commit_when_ready(&self, proof: &Proof) {
        let step = self.step();
        if *step == Step::Leaving || !proof.holds() {
            return;
        }
        let Some(path) = &proof.marker else { return };
        if let Err(error) = std::fs::remove_file(path) {
            eprintln!("pinned: could not remove {}, the next sign in tries again: {error}", path.display());
        }
    }

    fn refuse(&self, proof: &Proof) {
        if proof.holds() {
            self.refused.notify_one();
        }
    }
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
    Proof::of_this_process().holds()
}

fn proves(marker: &[u8], proof: &str) -> bool {
    !proof.is_empty() && String::from_utf8_lossy(marker).trim() == proof
}

pub fn ready() {
    if !BOOT.ready() {
        loop {
            std::thread::park();
        }
    }
}

pub fn leave_if_starting() -> bool {
    BOOT.leave_from(Step::Starting, &Proof::of_this_process())
}

pub fn leave_if_running() -> bool {
    BOOT.leave_from(Step::Running, &Proof::of_this_process())
}

pub async fn commit_when_ready() {
    BOOT.commit_when_ready(&Proof::of_this_process()).await
}

pub fn refuse() {
    BOOT.refuse(&Proof::of_this_process())
}

pub async fn refused() {
    BOOT.refused.notified().await
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
