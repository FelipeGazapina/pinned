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

    /// Whether the program has taken the terminal out of canonical mode, as claude does once it
    /// draws. Until then the tty itself echoes typed bytes and turns a \r into \n, so a prompt
    /// typed that early is shown and never submitted.
    #[cfg(unix)]
    pub fn raw(&self) -> bool {
        let Some(fd) = self.master.as_raw_fd() else { return true };
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
        self.master.resize(size).map_err(|e| e.to_string())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        let Some(mut child) = self.child.take() else { return };
        // portable-pty's kill gives SIGHUP 200ms before SIGKILL and never reaps a process that
        // outlived the SIGHUP, as claude does: off the caller's thread, and reaped, or a node piles up zombies.
        std::thread::spawn(move || {
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}
