//! Where one pinned account keeps its state on this computer, and the OS user's home it shares
//! with every other account here. A machine reaches that home only through its `Space`.

use crate::account::Credentials;
use std::path::{Path, PathBuf};

#[cfg(windows)]
const HOME_ENV: &str = "USERPROFILE";
#[cfg(not(windows))]
const HOME_ENV: &str = "HOME";

/// The OS user's home. Private to this module so that every other site reads `Space::user_home`.
fn home() -> PathBuf {
    PathBuf::from(std::env::var_os(HOME_ENV).unwrap_or_default())
}

/// Whether the account owns the folders of the OS user, or only its own space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    NonOwner,
}

#[derive(Clone, Debug)]
pub struct Space {
    /// The folder the machine keeps its sessions, projects and history in.
    pub dir: PathBuf,
    /// The OS user's home. One per root, shared by every account on it. A test binary gets
    /// `<root>/home`, never the developer's own.
    pub user_home: PathBuf,
    pub role: Role,
}

pub enum Opened {
    /// The app has not yet decided whose state the root holds.
    Undecided,
    Space(Space),
}

#[derive(Debug, PartialEq)]
pub struct Closed(pub String);

/// The space of the account signed in on `root`, or none yet. Every account opens the root, as in
/// v0.18.1.
pub fn open(root: &Path, _creds: Option<&Credentials>) -> Result<Opened, Closed> {
    let user_home = if crate::machine::test_binary() { root.join("home") } else { home() };
    Ok(Opened::Space(Space { dir: root.to_owned(), user_home, role: Role::Owner }))
}

impl Space {
    /// The space of `root` for a caller with no account to ask, such as a node or a test.
    pub fn at(root: &Path) -> Space {
        match open(root, None) {
            Ok(Opened::Space(space)) => space,
            _ => unreachable!("a root opens as a space until accounts have their own"),
        }
    }
}

/// The home of a process that runs outside any machine, such as a hook.
pub fn process_home() -> PathBuf {
    home()
}
