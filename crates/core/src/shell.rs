//! The user's shell, for the terminal surface and for the PATH agents run with. An app a desktop
//! launcher starts may have no SHELL, and the PATH it inherits is not the one the user's terminal sets.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// Shells the terminal surface knows how to start as a login shell.
const KNOWN: [&str; 8] = ["zsh", "bash", "sh", "fish", "dash", "ksh", "tcsh", "nu"];

/// The shells a system without SHELL or a passwd entry most likely has, best first.
#[cfg(target_os = "macos")]
const USUAL: [&str; 2] = ["/bin/zsh", "/bin/bash"];
#[cfg(not(target_os = "macos"))]
const USUAL: [&str; 2] = ["/bin/bash", "/bin/zsh"];

/// How long the login shell may take to print its PATH. A slow or stuck rc file would otherwise
/// hold up the app's start.
const PROBE_LIMIT: Duration = Duration::from_secs(5);
const MARK: &str = "__PINNED_PATH__";

pub fn known(shell: &str) -> bool {
    Path::new(shell).file_name().is_some_and(|n| KNOWN.contains(&n.to_string_lossy().as_ref()))
}

/// `env` (SHELL) when `accept` takes it, else the passwd database's shell when it is one of the
/// known shells and `exists`, else the first usual shell that exists, else /bin/sh.
pub fn choose(env: Option<String>, passwd: Option<String>, accept: impl Fn(&str) -> bool, exists: impl Fn(&str) -> bool) -> String {
    env.filter(|s| !s.is_empty() && accept(s))
        .or_else(|| passwd.filter(|s| known(s) && exists(s)))
        .or_else(|| USUAL.iter().find(|s| exists(s)).map(|s| s.to_string()))
        .unwrap_or_else(|| "/bin/sh".into())
}

fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// The shell the user's terminal starts. SHELL can name a wrapper, as a test's does; a shell this
/// doesn't know falls back to one it does.
pub fn terminal() -> String {
    choose(std::env::var("SHELL").ok(), passwd_shell(), known, exists)
}

/// The shell whose login setup gives the user's PATH: SHELL as it is, else as `terminal` picks.
fn login() -> String {
    choose(std::env::var("SHELL").ok(), passwd_shell(), |_| true, exists)
}

/// This user's shell in the passwd database (or the directory service behind it).
pub fn passwd_shell() -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let code = unsafe { libc::getpwuid_r(libc::getuid(), &mut entry, buf.as_mut_ptr(), buf.len(), &mut found) };
    if code != 0 || found.is_null() || entry.pw_shell.is_null() {
        return None;
    }
    let shell = unsafe { std::ffi::CStr::from_ptr(entry.pw_shell) }.to_string_lossy().into_owned();
    (!shell.is_empty()).then_some(shell)
}

/// Apps opened from Finder or a desktop launcher inherit a bare PATH. Ask the user's login shell
/// for the PATH their terminal sees, so claude and the tools it runs (git, node, brew) resolve.
pub fn login_path() -> Option<String> {
    login_path_from(&login(), PROBE_LIMIT)
}

/// Runs `shell` as an interactive login shell to print PATH, and gives up past `limit`.
pub fn login_path_from(shell: &str, limit: Duration) -> Option<String> {
    let mut child = crate::process::command(shell)
        .args(["-ilc", &format!("printf '\\n{MARK}%s{MARK}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let (found, heard) = std::sync::mpsc::channel();
    // Read until the PATH is whole rather than to the end: a program an rc file left running may
    // hold the pipe open after the shell exits.
    std::thread::spawn(move || {
        let (mut seen, mut buf) = (Vec::new(), [0u8; 4096]);
        while let Ok(n @ 1..) = out.read(&mut buf) {
            seen.extend_from_slice(&buf[..n]);
            if let Some(path) = between_marks(&String::from_utf8_lossy(&seen)) {
                let _ = found.send(path);
                return;
            }
        }
    });
    let path = heard.recv_timeout(limit).ok();
    let _ = child.kill();
    let _ = child.wait();
    path
}

fn between_marks(text: &str) -> Option<String> {
    let mut parts = text.split(MARK);
    parts.next()?;
    let path = parts.next()?;
    parts.next().map(|_| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(list: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |p| list.contains(&p)
    }

    #[test]
    fn with_no_shell_variable_the_passwd_shell_is_used() {
        assert_eq!(choose(None, Some("/usr/bin/fish".into()), known, present(&["/usr/bin/fish", "/bin/bash", "/bin/zsh"])), "/usr/bin/fish");
        assert_eq!(choose(Some(String::new()), Some("/usr/bin/fish".into()), |_| true, present(&["/usr/bin/fish"])), "/usr/bin/fish");
    }

    #[test]
    fn the_shell_variable_wins_when_accepted() {
        assert_eq!(choose(Some("/bin/zsh".into()), Some("/bin/bash".into()), known, present(&["/bin/bash"])), "/bin/zsh");
        assert_eq!(choose(Some("/tmp/wrapper".into()), Some("/bin/bash".into()), known, present(&["/bin/bash"])), "/bin/bash");
        assert_eq!(choose(Some("/tmp/wrapper".into()), None, |_| true, present(&[])), "/tmp/wrapper");
    }

    #[test]
    fn a_nologin_or_missing_passwd_shell_falls_to_the_usual_one() {
        let usual = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" };
        assert_eq!(choose(None, Some("/usr/sbin/nologin".into()), known, present(&["/usr/sbin/nologin", "/bin/bash", "/bin/zsh"])), usual);
        assert_eq!(choose(None, Some("/usr/bin/fish".into()), known, present(&["/bin/bash", "/bin/zsh"])), usual);
        assert_eq!(choose(None, None, known, present(&["/bin/zsh"])), "/bin/zsh");
        assert_eq!(choose(None, None, known, present(&[])), "/bin/sh");
    }

    #[test]
    fn this_user_has_a_passwd_shell() {
        let shell = passwd_shell().expect("no passwd shell for this user");
        assert!(shell.starts_with('/'), "{shell}");
    }

    fn script(name: &str, body: &str) -> (tempfile::TempDir, String) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path.display().to_string())
    }

    #[test]
    fn the_probe_answers_once_the_path_is_printed_and_gives_up_on_a_stuck_shell() {
        let (_dir, printing) = script("printing", &format!("printf 'motd\\n{MARK}/a:/b{MARK}'; exec sleep 30"));
        let started = std::time::Instant::now();
        assert_eq!(login_path_from(&printing, Duration::from_secs(5)).as_deref(), Some("/a:/b"));
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());

        let (_dir, stuck) = script("stuck", "exec sleep 30");
        let started = std::time::Instant::now();
        assert_eq!(login_path_from(&stuck, Duration::from_millis(300)), None);
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    }
}
