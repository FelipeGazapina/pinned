//! What a worker box's node does when the hub has refused it for good.
//!
//! The hub ends a box by deleting it on Upstash. When it cannot (Upstash is disconnected or its API
//! is down), the box keeps running, the hub answers its node with close code 4001 and the node would
//! retry for ever, with a Claude session in the box still using the plan. A box's node that the hub
//! has refused for `BOX_REFUSED_FOR`, with no hello accepted in between, and refuses once more at or
//! after that span, ends its sessions, removes its `account.json` so the box's start script does not
//! launch it again, and exits. A hub that is only unreachable never makes it leave.
//!
//! This does not stop the Upstash charge: the box stays on Upstash until someone deletes it.
//!
//! Only a box does this. The hub adds `PINNED_BOX=1` to the environment `start.sh` sources before it
//! launches the node, and the desktop app and a headless node never have it.

use pinned_core::account::Credentials;
use pinned_core::link::BOX_REFUSED_FOR;
use std::future::Future;
use std::path::Path;

/// The environment variable that tells a node it runs in a worker box.
pub const BOX_ENV: &str = "PINNED_BOX";

/// Whether this process's environment says it runs in a worker box.
pub fn in_box() -> bool {
    is_box(std::env::var(BOX_ENV).ok().as_deref())
}

pub(crate) fn is_box(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Resolves once `refused` does, and only in a box. Anywhere else it never resolves, and never
/// polls `refused`, so the machine is not left over for good by anything the hub does.
pub async fn refused_in_box(in_box: bool, refused: impl Future<Output = ()>) {
    if in_box {
        refused.await;
    } else {
        std::future::pending::<()>().await;
    }
}

/// The one line the node logs when it leaves. It names no key, token or hub address.
pub fn reason() -> String {
    format!(
        "pinned-server: the hub has refused this box's node for {} minutes; ending its sessions and exiting for good. The box itself stays on Upstash until it is deleted",
        BOX_REFUSED_FOR.as_secs() / 60
    )
}

/// Leaves for good: logs why, runs `quit` to end every session, then removes `account.json` from
/// `dir`, so a start script that finds no account does not launch the node again. The caller
/// returns from `main` after it.
///
/// The file goes only if it still holds `machine_token`, this node's own. A re-boot of the box
/// writes a new token there while this node still runs, and that file belongs to the node the
/// re-boot starts: it stays, and one more line says so. A file that cannot be read stays too, with
/// one more line; a node with no file has nothing to keep.
pub fn leave(dir: &Path, machine_token: &str, quit: impl FnOnce(), mut log: impl FnMut(&str)) {
    log(&reason());
    quit();
    match Credentials::load(dir) {
        Some(creds) if creds.machine_token == machine_token => Credentials::remove(dir),
        Some(_) => log("pinned-server: account.json holds another machine token than this node's, so it stays for the node that wrote it"),
        None if Credentials::path(dir).exists() => log("pinned-server: account.json could not be read, so it stays"),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinned_core::wire::MachineId;
    use std::cell::RefCell;
    use std::time::Duration;

    fn paired(dir: &Path, token: &str) {
        let creds = Credentials { hub: "https://hub.example".into(), email: "box@example.invalid".into(), user_token: String::new(), machine_id: MachineId::from("m1"), machine_token: token.into() };
        creds.save(dir).unwrap();
    }

    #[test]
    fn only_the_value_one_means_a_box() {
        assert!(is_box(Some("1")));
        for other in [None, Some(""), Some("0"), Some("true"), Some("yes"), Some(" 1")] {
            assert!(!is_box(other), "{other:?}");
        }
    }

    /// The hub refuses for 60 virtual minutes: the wait `Link::refused_for` stands for would end
    /// after the span. Without `PINNED_BOX` the node must stay up, paired, with its sessions untouched.
    #[tokio::test(start_paused = true)]
    async fn a_node_outside_a_box_refused_for_an_hour_stays_running_and_paired() {
        let dir = tempfile::tempdir().unwrap();
        paired(dir.path(), "fake-machine-token");
        let quit = RefCell::new(0);
        let hour = Duration::from_secs(3600);
        let refused = async {
            tokio::time::sleep(BOX_REFUSED_FOR).await;
        };
        let left = tokio::time::timeout(hour, refused_in_box(is_box(None), refused)).await.is_ok();
        // As in `node()`: when the wait ends the node leaves, and then the checks below fail.
        if left {
            leave(dir.path(), "fake-machine-token", || *quit.borrow_mut() += 1, |_| {});
        }
        assert_eq!(*quit.borrow(), 0, "a node outside a box ended its sessions");
        assert!(Credentials::load(dir.path()).is_some(), "a node outside a box lost its account.json");
        assert!(dir.path().join("account.json").exists());
    }

    #[tokio::test(start_paused = true)]
    async fn a_node_in_a_box_leaves_once_the_hub_has_refused_it_for_the_span() {
        let started = tokio::time::Instant::now();
        let refused = async {
            tokio::time::sleep(BOX_REFUSED_FOR).await;
        };
        refused_in_box(is_box(Some("1")), refused).await;
        assert_eq!(started.elapsed(), BOX_REFUSED_FOR);
    }

    #[test]
    fn a_box_ends_its_sessions_before_it_removes_its_account_and_leaves() {
        let dir = tempfile::tempdir().unwrap();
        paired(dir.path(), "fake-machine-token");
        let steps = RefCell::new(Vec::new());
        let account = dir.path().join("account.json");
        leave(
            dir.path(),
            "fake-machine-token",
            || steps.borrow_mut().push(format!("quit, account.json present: {}", account.exists())),
            |line| steps.borrow_mut().push(format!("log: {}", line.starts_with("pinned-server: the hub has refused"))),
        );
        steps.borrow_mut().push(format!("returned, account.json present: {}", account.exists()));
        assert_eq!(*steps.borrow(), ["log: true", "quit, account.json present: true", "returned, account.json present: false"]);
    }

    #[test]
    fn a_second_start_finds_no_account_and_stays_down() {
        let dir = tempfile::tempdir().unwrap();
        paired(dir.path(), "fake-machine-token");
        assert!(Credentials::load(dir.path()).is_some());
        leave(dir.path(), "fake-machine-token", || {}, |_| {});
        assert_eq!(Credentials::load(dir.path()), None);
    }

    #[test]
    fn the_exit_log_line_holds_no_token_and_says_the_box_stays_on_upstash() {
        let dir = tempfile::tempdir().unwrap();
        let token = "fake-token-7f3a91c2e5";
        paired(dir.path(), token);
        let lines = RefCell::new(Vec::new());
        leave(dir.path(), token, || {}, |line| lines.borrow_mut().push(line.to_owned()));
        let lines = lines.into_inner();
        assert_eq!(lines.len(), 1, "{lines:?}");
        for line in &lines {
            assert!(!line.contains(token), "{line}");
            assert!(!line.contains("hub.example") && !line.contains("://") && !line.contains('@'), "{line}");
            assert!(line.contains("15 minutes"), "{line}");
            assert!(line.contains("stays on Upstash until it is deleted"), "{line}");
        }
    }

    #[test]
    fn an_account_file_that_cannot_be_read_stays_and_the_log_says_so_without_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("account.json");
        std::fs::write(&file, "{ not json, fake-token-0a1b2c").unwrap();
        let (quit, lines) = (RefCell::new(0), RefCell::new(Vec::new()));
        leave(dir.path(), "fake-old-token", || *quit.borrow_mut() += 1, |line| lines.borrow_mut().push(line.to_owned()));
        assert_eq!(*quit.borrow(), 1, "the node still ends its sessions");
        assert!(file.exists(), "an unreadable file is kept");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json, fake-token-0a1b2c", "and left as it was");
        let lines = lines.into_inner();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1], "pinned-server: account.json could not be read, so it stays");
        for line in &lines {
            assert!(!line.contains("fake-") && !line.contains(dir.path().to_str().unwrap()), "{line}");
        }
    }

    #[test]
    fn a_node_with_no_account_file_logs_only_why_it_left() {
        let dir = tempfile::tempdir().unwrap();
        let lines = RefCell::new(Vec::new());
        leave(dir.path(), "fake-old-token", || {}, |line| lines.borrow_mut().push(line.to_owned()));
        assert_eq!(lines.borrow().len(), 1, "{:?}", lines.borrow());
    }

    #[test]
    fn an_account_file_a_re_boot_rewrote_with_a_newer_token_stays_and_the_log_names_no_token() {
        let dir = tempfile::tempdir().unwrap();
        paired(dir.path(), "fake-new-token");
        let (quit, lines) = (RefCell::new(0), RefCell::new(Vec::new()));
        leave(dir.path(), "fake-old-token", || *quit.borrow_mut() += 1, |line| lines.borrow_mut().push(line.to_owned()));
        assert_eq!(*quit.borrow(), 1, "the node still ends its sessions");
        assert_eq!(Credentials::load(dir.path()).unwrap().machine_token, "fake-new-token");
        let lines = lines.into_inner();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[1].contains("account.json"), "{}", lines[1]);
        for line in &lines {
            assert!(!line.contains("fake-"), "{line}");
        }
    }
}
