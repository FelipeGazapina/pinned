use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tokio::sync::Notify;

pub const PROOF_ENV: &str = "PINNED_PROOF_BOOT";
pub const MARKER_ENV: &str = "PINNED_UNPROVEN";
pub const VERSION_ENV: &str = "PINNED_VERSION";
pub const WAIT_ENV: &str = "PINNED_PROOF_WAIT";

#[derive(Debug, PartialEq, Eq)]
pub enum AfterHello {
    Serve,
    Leaving,
}

#[derive(Clone, PartialEq, Eq)]
enum Hub {
    Silent,
    Refused,
    Accepted(PathBuf),
}

#[derive(PartialEq, Eq)]
enum Step {
    Starting(Hub),
    Running(Hub),
    Proven,
    Leaving,
}

struct Boot {
    step: Mutex<Step>,
    changed: Notify,
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

    fn holds(&self) -> Option<&Path> {
        let path = self.marker.as_deref()?;
        proves(&std::fs::read(path).ok()?, &self.checksum).then_some(path)
    }
}

impl Boot {
    const fn new() -> Boot {
        Boot { step: Mutex::new(Step::Starting(Hub::Silent)), changed: Notify::const_new() }
    }

    fn step(&self) -> MutexGuard<'_, Step> {
        self.step.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ready(&self) -> bool {
        let mut step = self.step();
        let next = match &*step {
            Step::Leaving => return false,
            Step::Starting(Hub::Accepted(marker)) => commit(marker),
            Step::Starting(hub) => Step::Running(hub.clone()),
            Step::Running(_) | Step::Proven => return true,
        };
        *step = next;
        self.changed.notify_waiters();
        true
    }

    fn leave_if(&self, leaves: impl FnOnce(&Step) -> bool) -> bool {
        let mut step = self.step();
        if !leaves(&step) {
            return false;
        }
        *step = Step::Leaving;
        true
    }

    fn leave_if_starting(&self) -> bool {
        self.leave_if(|step| matches!(step, Step::Starting(_)))
    }

    async fn leave_when_refused(&self) {
        loop {
            let changed = self.changed.notified();
            if self.leave_if(|step| *step == Step::Running(Hub::Refused)) {
                return;
            }
            changed.await;
        }
    }

    async fn commit_when_ready(&self, proof: &Proof) -> AfterHello {
        loop {
            let changed = self.changed.notified();
            {
                let mut step = self.step();
                match (&*step, proof.holds()) {
                    (Step::Leaving, _) => return AfterHello::Leaving,
                    (Step::Proven, _) | (_, None) => return AfterHello::Serve,
                    (Step::Starting(_), Some(marker)) => *step = Step::Starting(Hub::Accepted(marker.to_owned())),
                    (Step::Running(_), Some(marker)) => {
                        *step = commit(marker);
                        return AfterHello::Serve;
                    }
                }
            }
            changed.await;
        }
    }

    fn refuse(&self, proof: &Proof) {
        if proof.holds().is_none() {
            return;
        }
        if let Step::Starting(hub) | Step::Running(hub) = &mut *self.step() {
            *hub = Hub::Refused;
            self.changed.notify_waiters();
        }
    }
}

fn commit(marker: &Path) -> Step {
    match std::fs::remove_file(marker) {
        Ok(()) => Step::Proven,
        Err(error) => {
            eprintln!("pinned: could not remove {}, the next sign in tries again: {error}", marker.display());
            Step::Running(Hub::Accepted(marker.to_owned()))
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
    Proof::of_this_process().holds().is_some()
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
    BOOT.leave_if_starting()
}

pub async fn commit_when_ready() -> AfterHello {
    BOOT.commit_when_ready(&Proof::of_this_process()).await
}

pub fn refuse() {
    BOOT.refuse(&Proof::of_this_process())
}

pub async fn leave_when_refused() {
    BOOT.leave_when_refused().await
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
        assert_eq!(hello.as_mut().now_or_never(), Some(AfterHello::Serve), "the hello was still waiting once the boot was ready");
        assert!(!marker_kept(&staged), "the hello did not commit once the boot was ready");
    }

    #[test]
    fn the_deadline_restores_a_boot_whose_hello_came_before_ready() {
        let (boot, staged) = (Boot::new(), staged("hang-after-hello", CHECKSUM));
        let mut hello = pin!(boot.commit_when_ready(&staged.proof));
        assert_eq!(hello.as_mut().now_or_never(), None, "a hello before ready did not wait for ready");
        assert!(boot.leave_if_starting(), "the deadline kept a boot that never became ready");
        assert!(!boot.ready(), "the main thread went on after the deadline chose to leave");
        assert_eq!(hello.as_mut().now_or_never(), None, "the link went on to serve the hub while the boot was leaving");
        assert!(marker_kept(&staged), "the marker went away before the rollback");
    }

    #[test]
    fn a_refusal_queued_before_a_hello_does_not_restore_the_proven_boot() {
        let (boot, staged) = (Boot::new(), staged("late-refusal", CHECKSUM));
        assert!(boot.ready());
        boot.refuse(&staged.proof);
        assert_eq!(pin!(boot.commit_when_ready(&staged.proof)).now_or_never(), Some(AfterHello::Serve), "a hello after ready waited");
        assert!(!marker_kept(&staged), "a hello after ready did not commit");
        assert_eq!(pin!(boot.leave_when_refused()).now_or_never(), None, "a refusal queued before the hub accepted the boot rolled it back");
    }

    #[test]
    fn a_refusal_before_ready_does_not_restore_a_boot_the_hub_then_accepted() {
        let (boot, staged) = (Boot::new(), staged("refused-then-accepted", CHECKSUM));
        boot.refuse(&staged.proof);
        let mut hello = pin!(boot.commit_when_ready(&staged.proof));
        assert_eq!(hello.as_mut().now_or_never(), None, "a hello before ready did not wait for ready");
        assert!(boot.ready());
        let left = pin!(boot.leave_when_refused()).now_or_never();
        let proven = !marker_kept(&staged);
        let served = hello.as_mut().now_or_never();
        assert_eq!((left, proven, served), (None, true, Some(AfterHello::Serve)), "(rolled back, proven as it became ready, link serves)");
    }

    #[test]
    #[cfg(unix)]
    fn a_hello_the_hub_accepted_clears_its_refusal_even_when_the_marker_stays() {
        use std::os::unix::fs::PermissionsExt;
        let (boot, staged) = (Boot::new(), staged("refused-unremovable", CHECKSUM));
        assert!(boot.ready());
        boot.refuse(&staged.proof);
        std::fs::set_permissions(&staged.dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let served = pin!(boot.commit_when_ready(&staged.proof)).now_or_never();
        std::fs::set_permissions(&staged.dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let left = pin!(boot.leave_when_refused()).now_or_never();
        assert_eq!((served, marker_kept(&staged), left), (Some(AfterHello::Serve), true, None), "(link serves, marker stays, rolled back)");
    }

    #[test]
    fn a_refusal_while_the_boot_runs_unproven_restores_it() {
        let (boot, staged) = (Boot::new(), staged("refused-running", CHECKSUM));
        assert!(boot.ready());
        boot.refuse(&staged.proof);
        assert_eq!(pin!(boot.leave_when_refused()).now_or_never(), Some(()), "the hub refused the unproven boot and it stayed");
        assert_eq!(pin!(boot.commit_when_ready(&staged.proof)).now_or_never(), Some(AfterHello::Leaving), "a hello during the rollback went on to serve the hub");
        assert!(marker_kept(&staged), "the marker went away before the rollback");
    }

    #[test]
    fn a_refusal_wakes_the_rollback_already_waiting_on_the_running_boot() {
        let (boot, staged) = (Boot::new(), staged("refused-later", CHECKSUM));
        assert!(boot.ready());
        let mut left = pin!(boot.leave_when_refused());
        assert_eq!(left.as_mut().now_or_never(), None, "the boot left before the hub refused it");
        boot.refuse(&staged.proof);
        assert_eq!(left.as_mut().now_or_never(), Some(()), "the hub refused the unproven boot and the waiting rollback slept on");
    }

    #[test]
    fn a_ready_boot_is_past_the_deadline() {
        let (boot, staged) = (Boot::new(), staged("ready-then-deadline", CHECKSUM));
        assert!(boot.ready());
        assert!(!boot.leave_if_starting(), "the deadline rolled back a boot that was ready");
        assert!(marker_kept(&staged));
    }

    #[test]
    fn a_boot_the_deadline_left_neither_becomes_ready_nor_serves_the_hub() {
        let (boot, staged) = (Boot::new(), staged("deadline-then-ready", CHECKSUM));
        assert!(boot.leave_if_starting());
        assert!(!boot.ready(), "the main thread went on to resume sessions during a rollback");
        assert_eq!(pin!(boot.commit_when_ready(&staged.proof)).now_or_never(), Some(AfterHello::Leaving), "a hello during the rollback went on to serve the hub");
        assert!(marker_kept(&staged), "a hello during the rollback removed the marker");
    }

    #[test]
    fn a_process_with_nothing_to_prove_signs_in_without_ready_and_never_queues_a_refusal() {
        let (boot, staged) = (Boot::new(), staged("no-proof", ""));
        assert_eq!(pin!(boot.commit_when_ready(&staged.proof)).now_or_never(), Some(AfterHello::Serve), "a process with nothing to prove waited for ready");
        assert!(marker_kept(&staged), "a process with nothing to prove removed the marker");
        boot.refuse(&staged.proof);
        assert!(boot.ready());
        assert_eq!(pin!(boot.leave_when_refused()).now_or_never(), None, "a process with nothing to prove queued a refusal");
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
