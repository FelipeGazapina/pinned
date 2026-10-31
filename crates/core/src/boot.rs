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
    use futures_util::FutureExt;
    use std::pin::pin;

    const CHECKSUM: &str = "9f86d081884c7d65";

    struct Staged {
        dir: PathBuf,
        proof: Proof,
    }

    impl Drop for Staged {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn staged(name: &str, checksum: &str) -> Staged {
        let dir = std::env::temp_dir().join(format!("pinned-boot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("pinned-server.unproven");
        std::fs::write(&marker, format!("{CHECKSUM}\n")).unwrap();
        Staged { dir, proof: Proof { marker: Some(marker), checksum: checksum.into() } }
    }

    fn marker_kept(staged: &Staged) -> bool {
        staged.proof.marker.as_ref().unwrap().exists()
    }

    #[test]
    fn a_hello_before_ready_commits_once_the_boot_is_ready() {
        let (boot, staged) = (Boot::new(), staged("early-hello", CHECKSUM));
        let mut hello = pin!(boot.commit_when_ready(&staged.proof));
        assert_eq!(hello.as_mut().now_or_never(), None, "a hello before ready did not wait for ready");
        assert!(marker_kept(&staged), "a hello before ready committed");
        assert!(boot.ready());
        assert_eq!(hello.as_mut().now_or_never(), Some(()), "the hello was still waiting once the boot was ready");
        assert!(!marker_kept(&staged), "the hello did not commit once the boot was ready");
    }

    #[test]
    fn the_deadline_restores_a_boot_whose_hello_came_before_ready() {
        let (boot, staged) = (Boot::new(), staged("hang-after-hello", CHECKSUM));
        let mut hello = pin!(boot.commit_when_ready(&staged.proof));
        assert_eq!(hello.as_mut().now_or_never(), None, "a hello before ready did not wait for ready");
        assert!(boot.leave_from(Step::Starting, &staged.proof), "the deadline kept a boot that never became ready");
        assert!(!boot.ready(), "the main thread went on after the deadline chose to leave");
        assert_eq!(hello.as_mut().now_or_never(), None, "the link went on to serve the hub while the boot was leaving");
        assert!(marker_kept(&staged), "the marker went away before the rollback");
    }

    #[test]
    fn a_refusal_queued_before_a_hello_does_not_restore_the_proven_boot() {
        let (boot, staged) = (Boot::new(), staged("late-refusal", CHECKSUM));
        assert!(boot.ready());
        boot.refuse(&staged.proof);
        assert_eq!(pin!(boot.commit_when_ready(&staged.proof)).now_or_never(), Some(()), "a hello after ready waited");
        assert!(!marker_kept(&staged), "a hello after ready did not commit");
        assert_eq!(pin!(boot.refused.notified()).now_or_never(), Some(()), "the refusal was not queued");
        assert!(!boot.leave_from(Step::Running, &staged.proof), "a refusal queued before the hub accepted the boot rolled it back");
    }

    #[test]
    fn a_ready_boot_is_past_the_deadline() {
        let (boot, staged) = (Boot::new(), staged("ready-then-deadline", CHECKSUM));
        assert!(boot.ready());
        assert!(!boot.leave_from(Step::Starting, &staged.proof), "the deadline rolled back a boot that was ready");
        assert!(marker_kept(&staged));
    }

    #[test]
    fn a_boot_the_deadline_left_never_becomes_ready() {
        let (boot, staged) = (Boot::new(), staged("deadline-then-ready", CHECKSUM));
        assert!(boot.leave_from(Step::Starting, &staged.proof));
        assert!(!boot.ready(), "the main thread went on to resume sessions during a rollback");
    }

    #[test]
    fn a_process_with_nothing_to_prove_signs_in_without_ready_and_never_queues_a_refusal() {
        let (boot, staged) = (Boot::new(), staged("no-proof", ""));
        assert_eq!(pin!(boot.commit_when_ready(&staged.proof)).now_or_never(), Some(()), "a process with nothing to prove waited for ready");
        assert!(marker_kept(&staged), "a process with nothing to prove removed the marker");
        boot.refuse(&staged.proof);
        assert_eq!(pin!(boot.refused.notified()).now_or_never(), None, "a process with nothing to prove queued a refusal");
    }

    #[test]
    fn only_the_checksum_the_marker_records_proves_it() {
        let marker = b"9f86d081884c7d65\n";
        assert!(proves(marker, "9f86d081884c7d65"));
        assert!(!proves(marker, ""), "a process with no proof matched the marker");
        assert!(!proves(marker, "2c26b46b68ffc68f"), "a proof of other bytes matched the marker");
        assert!(!proves(b"", ""), "an empty marker matched no proof");
    }
}
