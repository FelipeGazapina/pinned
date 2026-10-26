use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Option<Box<dyn Child + Send + Sync>>,
    killed: Arc<AtomicBool>,
}

pub fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { cols, rows, pixel_width: 0, pixel_height: 0 }
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
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let killed = Arc::new(AtomicBool::new(false));
        let reader_killed = killed.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                on_output(buf[..n].to_vec());
            }
            // A session we dropped ourselves must not report an exit that the UI would pin on its replacement.
            if !reader_killed.load(Ordering::SeqCst) {
                on_exit();
            }
        });

        Ok(Self { master: pair.master, writer, child: Some(child), killed })
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref()?.process_id()
    }

    pub fn resize(&self, size: PtySize) -> Result<(), String> {
        self.master.resize(size).map_err(|e| e.to_string())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        let Some(mut child) = self.child.take() else { return };
        // Before the app quitting can exit under it.
        #[cfg(unix)]
        if let Some(pid) = child.process_id() {
            kill_tree(pid);
        }
        // portable-pty's kill gives SIGHUP 200ms before SIGKILL and never reaps a process that
        // outlived the SIGHUP, as claude does: off the caller's thread, and reaped, or a node piles up zombies.
        std::thread::spawn(move || {
            let _ = child.kill();
            let _ = child.wait();
        });
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
    let Ok(ps) = std::process::Command::new("ps").args(["-A", "-o", "pid=", "-o", "ppid="]).output() else { return vec![] };
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
    let _ = std::process::Command::new("kill").arg(which).args(pids.iter().map(u32::to_string)).output();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn alive(pid: &str) -> bool {
        std::process::Command::new("kill").args(["-0", pid]).status().is_ok_and(|s| s.success())
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
        let _ = std::process::Command::new("kill").args(["-9", &pid]).status();
        assert!(!survived, "the detached child outlived its session");
    }
}
