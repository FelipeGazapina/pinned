use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Taken and dropped once the pseudo-terminal must close.
type Master = Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>;

pub struct Session {
    master: Master,
    writer: Box<dyn Write + Send>,
    child: Option<Box<dyn Child + Send + Sync>>,
    killed: Arc<AtomicBool>,
}

pub fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { cols, rows, pixel_width: 0, pixel_height: 0 }
}

#[cfg(windows)]
const CURSOR_REQUEST: &[u8] = b"\x1b[6n";
#[cfg(windows)]
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// `output` without ConPTY's first cursor position request, which the session already answered.
#[cfg(windows)]
fn without_cursor_request(output: Vec<u8>, removed: &mut bool) -> Vec<u8> {
    if *removed {
        return output;
    }
    match output.windows(CURSOR_REQUEST.len()).position(|w| w == CURSOR_REQUEST) {
        Some(at) => {
            *removed = true;
            [&output[..at], &output[at + CURSOR_REQUEST.len()..]].concat()
        }
        None => output,
    }
}

/// ConPTY keeps its output open after the process exits, until the pseudo-console closes, so the
/// reader would never see the end of a session that ended by itself. Closing it once the process
/// is gone lets the reader drain the last output and finish.
#[cfg(windows)]
fn close_on_exit(child: &(dyn Child + Send + Sync), master: Master) {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
    let Some(pid) = child.process_id() else { return };
    // The session still holds the child's handle, so `pid` can't name another process yet.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return;
    }
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    std::thread::spawn(move || {
        unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) };
        let master = master.lock().unwrap().take();
        drop(master);
    });
}

impl Session {
    pub fn spawn(
        cmd: CommandBuilder,
        size: PtySize,
        mut on_output: impl FnMut(Vec<u8>) + Send + 'static,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        // ConPTY asks the terminal where its cursor is and shows nothing until it hears back. A tab
        // may have no terminal attached yet, so the session answers, and viewers never see the question.
        #[cfg(windows)]
        writer.write_all(CURSOR_REPLY).and_then(|()| writer.flush()).map_err(|e| e.to_string())?;

        let killed = Arc::new(AtomicBool::new(false));
        let reader_killed = killed.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            #[cfg(windows)]
            let mut asked = false;
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                #[cfg(windows)]
                let output = without_cursor_request(buf[..n].to_vec(), &mut asked);
                #[cfg(not(windows))]
                let output = buf[..n].to_vec();
                on_output(output);
            }
            // A session we dropped ourselves must not report an exit that the UI would pin on its replacement.
            if !reader_killed.load(Ordering::SeqCst) {
                on_exit();
            }
        });

        let master = Arc::new(Mutex::new(Some(pair.master)));
        #[cfg(windows)]
        close_on_exit(&*child, master.clone());
        Ok(Self { master, writer, child: Some(child), killed })
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref()?.process_id()
    }

    /// Whether the program has taken the terminal out of canonical mode, as claude does once it
    /// draws. Until then the tty itself echoes typed bytes and turns a \r into \n, so a prompt
    /// typed that early is shown and never submitted.
    #[cfg(unix)]
    pub fn raw(&self) -> bool {
        let Some(fd) = self.master.lock().unwrap().as_ref().and_then(|master| master.as_raw_fd()) else { return true };
        let mut term = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: fd is the open pty master, and tcgetattr fills the termios it is given.
        if unsafe { libc::tcgetattr(fd, term.as_mut_ptr()) } != 0 {
            return true;
        }
        unsafe { term.assume_init() }.c_lflag & libc::ICANON == 0
    }

    #[cfg(not(unix))]
    pub fn raw(&self) -> bool {
        true
    }

    pub fn resize(&self, size: PtySize) -> Result<(), String> {
        match &*self.master.lock().unwrap() {
            Some(master) => master.resize(size).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static BEFORE_REAP: std::cell::Cell<Option<Box<dyn FnOnce()>>> = const { std::cell::Cell::new(None) };
}

impl Drop for Session {
    fn drop(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        let Some(mut child) = self.child.take() else { return };
        // Before the app quitting can exit under it.
        #[cfg(unix)]
        if let Some(pid) = child.process_id() {
            kill_tree(pid);
            // portable-pty's kill would send SIGHUP and poll for 50 ms first.
            // SAFETY: the child is not reaped yet, so `pid` still names it.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
        #[cfg(not(unix))]
        let _ = child.kill();
        #[cfg(test)]
        if let Some(pause) = BEFORE_REAP.take() {
            pause();
        }
        // Reaped on this thread: a thread spawned for the wait is gone when this process execs,
        // and the child stays a zombie.
        let _ = child.wait();
    }
}

/// claude runs its tools, background shells and subagents' commands in process groups of their
/// own, which the terminal's hangup never reaches and which outlive claude. Each process found is
/// stopped before its children are looked up, so none can fork away or be orphaned, then all are killed.
#[cfg(unix)]
fn kill_tree(root: u32) {
    let mut tree = vec![root];
    let mut found = vec![root];
    while !found.is_empty() {
        signal("-STOP", &found);
        found = children(&tree);
        tree.extend(&found);
    }
    signal("-KILL", &tree);
}

#[cfg(unix)]
fn children(of: &[u32]) -> Vec<u32> {
    let Ok(ps) = crate::process::command("ps").args(["-A", "-o", "pid=", "-o", "ppid="]).output() else { return vec![] };
    String::from_utf8_lossy(&ps.stdout)
        .lines()
        .filter_map(|line| {
            let mut ids = line.split_whitespace().map(|id| id.parse::<u32>().ok());
            Some((ids.next()??, ids.next()??))
        })
        .filter(|(pid, ppid)| of.contains(ppid) && !of.contains(pid))
        .map(|(pid, _)| pid)
        .collect()
}

#[cfg(unix)]
fn signal(which: &str, pids: &[u32]) {
    let _ = crate::process::command("kill").arg(which).args(pids.iter().map(u32::to_string)).output();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn alive(pid: &str) -> bool {
        crate::process::command("kill").args(["-0", pid]).status().is_ok_and(|s| s.success())
    }

    #[test]
    fn ending_a_session_ends_what_it_started_in_process_groups_of_its_own() {
        // claude runs its tools and background shells in process groups the terminal's hangup never reaches.
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "perl -e 'setpgrp(0, 0); print \"$$\\n\"; sleep 300' & wait"]);
        let (tx, rx) = mpsc::channel();
        let session = Session::spawn(cmd, size(80, 24), move |bytes| _ = tx.send(bytes), || {}).unwrap();
        let mut out = String::new();
        while !out.contains('\n') {
            out.push_str(&String::from_utf8_lossy(&rx.recv_timeout(Duration::from_secs(5)).unwrap()));
        }
        let pid = out.trim().to_owned();
        assert!(alive(&pid), "the detached child is running");

        drop(session);
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(&pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let survived = alive(&pid);
        let _ = crate::process::command("kill").args(["-9", &pid]).status();
        assert!(!survived, "the detached child outlived its session");
    }

    #[test]
    fn dropping_a_session_reaps_before_drop_returns() {
        for _ in 0..8 {
            let mut cmd = CommandBuilder::new("/bin/sleep");
            cmd.arg("30");
            let session = Session::spawn(cmd, size(80, 24), |_| {}, || {}).unwrap();
            let pid = session.pid().unwrap().to_string();
            drop(session);
            let stat = crate::process::command("ps").args(["-o", "stat=", "-p", &pid]).output().unwrap();
            let stat = String::from_utf8_lossy(&stat.stdout);
            assert!(!stat.contains('Z'), "left a zombie: {stat}");
            assert!(!alive(&pid), "sleep {pid} is still running");
        }
    }
}
