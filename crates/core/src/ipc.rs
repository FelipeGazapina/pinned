//! The socket the harness listens on and its sessions' subcommands connect to. Windows 10 1803
//! and later speak AF_UNIX too, through a crate with std's API.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};

/// Whether this system tells which process is at the other end of a connection.
pub const KNOWS_PEERS: bool = cfg!(any(target_os = "linux", target_os = "macos"));

/// The process that connected `stream`, as the kernel recorded it when it connected: nothing the
/// process sends can change it.
#[cfg(target_os = "linux")]
pub fn peer_pid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` describe a buffer of the size SO_PEERCRED writes.
    let read = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
    (read == 0 && cred.pid > 0).then_some(cred.pid as u32)
}

#[cfg(target_os = "macos")]
pub fn peer_pid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `pid` and `len` describe a buffer of the size LOCAL_PEERPID writes.
    let read = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&mut pid as *mut libc::pid_t).cast(), &mut len) };
    (read == 0 && pid > 0).then_some(pid as u32)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn peer_pid(_stream: &UnixStream) -> Option<u32> {
    None
}

/// The parent of process `pid`.
#[cfg(target_os = "linux")]
pub fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command's name, in parentheses, may hold spaces and parentheses: the state and then the
    // parent follow its last `)`.
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok().filter(|&ppid| ppid > 0)
}

#[cfg(not(target_os = "linux"))]
pub fn parent_of(pid: u32) -> Option<u32> {
    let out = crate::process::command("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|&ppid| ppid > 0)
}
