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

/// A process that connected, as it was when it connected: its pid, and when it started, so a later
/// look at that pid can tell whether it is still the same process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub pid: u32,
    pub started: u64,
}

/// The process at the other end of `stream`, read when it is accepted.
pub fn peer(stream: &UnixStream) -> Option<Peer> {
    let pid = peer_pid(stream)?;
    Some(Peer { pid, started: process(pid)?.started })
}

/// One process as the system reports it: its parent and when it started, read together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Process {
    pub parent: u32,
    pub started: u64,
}

/// How many parents up from a caller `nearest` looks for a session's agent.
pub const MAX_DEPTH: usize = 32;

/// The nearest process from `peer` up its parents, `peer` included, that `owner` names an owner
/// for, and that owner. None when `peer` is no longer the process that connected (its pid now
/// belongs to a later process), or when no owner shows up within `MAX_DEPTH` parents, before the
/// first process, or before a parent the system does not report. `read` reports a process.
pub fn nearest<T>(peer: Peer, read: impl Fn(u32) -> Option<Process>, owner: impl Fn(u32) -> Option<T>) -> Option<T> {
    let mut pid = peer.pid;
    let mut process = read(pid).filter(|p| p.started == peer.started)?;
    for _ in 0..MAX_DEPTH {
        if let Some(found) = owner(pid) {
            return Some(found);
        }
        if process.parent <= 1 {
            return None;
        }
        pid = process.parent;
        process = read(pid)?;
    }
    None
}

/// Process `pid` as the system reports it now.
#[cfg(target_os = "linux")]
pub fn process(pid: u32) -> Option<Process> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command's name, in parentheses, may hold spaces and parentheses: the fields after its
    // last `)` start with the state (field 3), the parent (4) and, at 22, the start time.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some(Process { parent: fields.get(1)?.parse().ok()?, started: fields.get(19)?.parse().ok()? })
}

#[cfg(target_os = "macos")]
pub fn process(pid: u32) -> Option<Process> {
    // SAFETY: an all-zero proc_bsdinfo is a valid value of that plain C struct.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a buffer of `size` bytes, which PROC_PIDTBSDINFO fills at most.
    let read = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (read == size).then(|| Process { parent: info.pbi_ppid, started: info.pbi_start_tvsec.wrapping_mul(1_000_000).wrapping_add(info.pbi_start_tvusec) })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process(_pid: u32) -> Option<Process> {
    None
}

/// The parent of process `pid`.
pub fn parent_of(pid: u32) -> Option<u32> {
    process(pid).map(|p| p.parent).filter(|&ppid| ppid > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A process table: pid to (parent, start time).
    fn table(rows: &[(u32, u32, u64)]) -> impl Fn(u32) -> Option<Process> {
        let rows: HashMap<u32, Process> = rows.iter().map(|&(pid, parent, started)| (pid, Process { parent, started })).collect();
        move |pid| rows.get(&pid).copied()
    }

    fn agents(pids: &[(u32, &'static str)]) -> impl Fn(u32) -> Option<&'static str> {
        let pids: HashMap<u32, &'static str> = pids.iter().copied().collect();
        move |pid| pids.get(&pid).copied()
    }

    #[test]
    fn the_nearest_agent_up_the_parents_owns_a_caller() {
        // 1 <- 10 (tab a's agent, a launcher) <- 11 (the real agent) <- 12 (a shell) <- 13 (the caller)
        let read = table(&[(1, 0, 1), (10, 1, 5), (11, 10, 6), (12, 11, 7), (13, 12, 8), (20, 1, 9), (21, 20, 10), (30, 1, 11)]);
        let owner = agents(&[(10, "a"), (20, "b")]);
        assert_eq!(nearest(Peer { pid: 13, started: 8 }, &read, &owner), Some("a"), "three parents up, past a launcher's child");
        assert_eq!(nearest(Peer { pid: 11, started: 6 }, &read, &owner), Some("a"), "the direct child");
        assert_eq!(nearest(Peer { pid: 10, started: 5 }, &read, &owner), Some("a"), "the agent itself");
        assert_eq!(nearest(Peer { pid: 21, started: 10 }, &read, &owner), Some("b"), "another tab's child");
        assert_eq!(nearest(Peer { pid: 30, started: 11 }, &read, &owner), None, "a process the first process adopted");
        assert_eq!(nearest(Peer { pid: 99, started: 1 }, &read, &owner), None, "a process that is gone");
    }

    #[test]
    fn a_pid_that_now_belongs_to_a_later_process_is_no_caller() {
        let read = table(&[(1, 0, 1), (10, 1, 5), (13, 10, 8)]);
        let owner = agents(&[(10, "a")]);
        assert_eq!(nearest(Peer { pid: 13, started: 8 }, &read, &owner), Some("a"));
        assert_eq!(nearest(Peer { pid: 13, started: 7 }, &read, &owner), None, "the pid started again since it connected");
        assert_eq!(nearest(Peer { pid: 10, started: 4 }, &read, &owner), None, "even when the pid is an agent's");
    }

    #[test]
    fn the_walk_stops_at_its_depth_and_at_a_parent_it_cannot_read() {
        let chain: Vec<(u32, u32, u64)> = (2..=(MAX_DEPTH as u32 + 2)).map(|pid| (pid, pid - 1, 1)).collect();
        let read = table(&chain);
        assert_eq!(nearest(Peer { pid: MAX_DEPTH as u32, started: 1 }, &read, &agents(&[(2, "a")])), Some("a"), "within its depth");
        assert_eq!(nearest(Peer { pid: MAX_DEPTH as u32 + 2, started: 1 }, &read, &agents(&[(2, "a")])), None, "past its depth");
        let gap = table(&[(13, 12, 8), (11, 10, 6), (10, 1, 5)]);
        assert_eq!(nearest(Peer { pid: 13, started: 8 }, &gap, &agents(&[(10, "a")])), None, "a parent it cannot read");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn this_process_reads_as_itself() {
        let me = process(std::process::id()).unwrap();
        // SAFETY: getppid has no preconditions.
        assert_eq!(parent_of(std::process::id()), Some(unsafe { libc::getppid() } as u32));
        assert_eq!(me, process(std::process::id()).unwrap(), "the same start time on a second read");
        assert_eq!(nearest(Peer { pid: std::process::id(), started: me.started }, process, |pid| (pid == std::process::id()).then_some(())), Some(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_the_parent_is_the_systems_own() {
        // SAFETY: getppid has no preconditions.
        assert_eq!(parent_of(std::process::id()), Some(unsafe { libc::getppid() } as u32));
    }
}
