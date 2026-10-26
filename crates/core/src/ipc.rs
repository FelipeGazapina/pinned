//! The socket the harness listens on and its sessions' subcommands connect to. Windows 10 1803
//! and later speak AF_UNIX too, through a crate with std's API.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};
