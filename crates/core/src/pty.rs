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

    pub fn resize(&self, size: PtySize) -> Result<(), String> {
        match &*self.master.lock().unwrap() {
            Some(master) => master.resize(size).map_err(|e| e.to_string()),
            None => Ok(()),
        }
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
