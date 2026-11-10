//! `claude setup-token` for one agent account of this machine, so its boxes sign in as that
//! account with no token copied by hand. It runs in a terminal of its own, which no tab, viewer or
//! hub mirrors. The browser it opens is where the user approves, and the long-lived token it
//! prints goes back over the hub link alone. The account's own login, which refreshes itself, is
//! never read or copied.

use crate::accounts::AccountId;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

/// How long setup-token waits for the user's approval in the browser before pinned stops it.
pub const WAIT: Duration = Duration::from_secs(10 * 60);
const PREFIX: &str = "sk-ant-oat01-";
/// The CLI draws with Ink, which wraps at the terminal's width: this one is wide enough that the
/// token stays on one line.
const COLS: u16 = 1000;
/// The output kept, from its end: a run that draws for minutes stays bounded.
const KEPT: usize = 1 << 20;

struct Running {
    pid: u32,
    cancelled: bool,
}

/// This machine's setup-token runs, one per account, each by the process id it started.
#[derive(Default)]
pub struct SetupTokens(Mutex<HashMap<AccountId, Running>>);

impl SetupTokens {
    /// Runs `bin setup-token` with `env` for `account`, and answers the token it prints once it
    /// exits. Past `wait`, or once cancelled, it kills the process it started, and only that one.
    pub fn run(&self, account: &str, bin: &Path, env: &[(String, String)], wait: Duration) -> Result<String, String> {
        let pair = native_pty_system().openpty(PtySize { rows: 50, cols: COLS, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
        let mut cmd = CommandBuilder::new(bin);
        cmd.arg("setup-token");
        cmd.cwd(std::env::temp_dir());
        for key in crate::machine::INHERITED_SESSION_ENV.iter().chain(&["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"]) {
            cmd.env_remove(key);
        }
        for (key, value) in env {
            cmd.env(key, value);
        }
        let mut child = {
            let mut running = self.0.lock().unwrap();
            if running.contains_key(account) {
                return Err(format!("claude setup-token already runs for {account} on this machine: approve it in the browser it opened, or cancel it"));
            }
            let child = pair.slave.spawn_command(cmd).map_err(|e| format!("could not run claude setup-token: {e}"))?;
            let pid = child.process_id().ok_or("claude setup-token started with no process id")?;
            running.insert(account.to_owned(), Running { pid, cancelled: false });
            child
        };
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let (sent, output) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut out, mut buf) = (Vec::new(), [0u8; 8192]);
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                out.extend_from_slice(&buf[..n]);
                if out.len() > KEPT {
                    out.drain(..out.len() - KEPT / 2);
                }
            }
            let _ = sent.send(out);
        });
        let deadline = Instant::now() + wait;
        // The child is reaped and its record dropped under the lock a cancel takes, so no kill
        // reaches a process id the system may have given to another process since.
        let (timed_out, cancelled) = loop {
            {
                let mut running = self.0.lock().unwrap();
                if child.try_wait().map_or(true, |status| status.is_some()) {
                    break (false, running.remove(account).is_some_and(|run| run.cancelled));
                }
                if Instant::now() >= deadline {
                    if let Some(run) = running.remove(account) {
                        kill(run.pid);
                    }
                    drop(running);
                    let _ = child.wait();
                    break (true, false);
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        // What the CLI started may still hold the terminal open; closing the master ends the read.
        drop(pair.master);
        let out = output.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
        if cancelled {
            return Err("claude setup-token was cancelled".into());
        }
        if timed_out {
            return Err(format!("no one approved claude setup-token in the browser within {} min, so pinned stopped it", wait.as_secs() / 60));
        }
        token_in(&out).ok_or_else(|| format!("claude setup-token ended with no token: {}", last_line(&out)))
    }

    /// Stops `account`'s run, if one waits. Answers whether one did.
    pub fn cancel(&self, account: &str) -> bool {
        let mut running = self.0.lock().unwrap();
        let Some(run) = running.get_mut(account) else { return false };
        run.cancelled = true;
        kill(run.pid);
        true
    }
}

/// Kills the process `pid` alone. It is still this run's child, not yet reaped, so the id can't
/// name another process.
fn kill(pid: u32) {
    #[cfg(unix)]
    // SAFETY: kill sends a signal and touches no memory.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = crate::process::command("taskkill").args(["/F", "/PID", &pid.to_string()]).output();
    }
}

/// `bytes` as text without its terminal control sequences.
fn plain(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then one final byte from @ to ~.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ESC \.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The last long-lived token the output shows.
fn token_in(bytes: &[u8]) -> Option<String> {
    let text = plain(bytes);
    let at = text.rfind(PREFIX)?;
    let token: String = text[at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (token.len() > PREFIX.len() + 20).then_some(token)
}

/// The last line the CLI printed, for a run that ended without a token, with anything shaped like
/// a secret hidden.
fn last_line(bytes: &[u8]) -> String {
    let text = plain(bytes);
    let line = text.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it printed nothing");
    crate::surface::mask_secrets(&line.chars().take(200).collect::<String>())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    const TOKEN: &str = "sk-ant-oat01-Fake_Token-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJ-KLMNOPQRSTUVWXYZ_0123456789-AA";

    fn fake_claude(name: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("pinned-setup-token-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(&bin, format!("#!/bin/sh\necho $$ > \"{}/pid\"\n{body}\n", dir.display())).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, bin)
    }

    fn alive(pid: u32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    fn pid_in(dir: &Path) -> u32 {
        for _ in 0..500 {
            if let Some(pid) = std::fs::read_to_string(dir.join("pid")).ok().and_then(|p| p.trim().parse().ok()) {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the fake claude never started");
    }

    #[test]
    fn the_token_setup_token_prints_is_taken_from_its_own_terminal_with_the_accounts_env() {
        let body = format!(
            "[ \"$1\" = setup-token ] || exit 9\necho \"CONFIG=$CLAUDE_CONFIG_DIR OAUTH=${{CLAUDE_CODE_OAUTH_TOKEN:-none}}\" > \"$(dirname \"$0\")/seen\"\nprintf 'Opening https://claude.com/cai/oauth/authorize?code=true&scope=user%%3Ainference\\n'\nprintf '\\033[32m\\342\\234\\223 Long-lived authentication token created successfully!\\033[39m\\n\\nYour OAuth token (valid for 1 year):\\n\\n\\033[1m{TOKEN}\\033[22m\\n\\nStore this token securely.\\n'"
        );
        let (dir, bin) = fake_claude("ok", &body);
        let env = [("CLAUDE_CONFIG_DIR".to_owned(), "/accounts/bloom".to_owned()), ("PINNED_ACCOUNT".to_owned(), "bloom".to_owned())];
        let runs = SetupTokens::default();
        assert_eq!(runs.run("bloom", &bin, &env, Duration::from_secs(10)), Ok(TOKEN.to_owned()));
        assert_eq!(std::fs::read_to_string(dir.join("seen")).unwrap().trim(), "CONFIG=/accounts/bloom OAUTH=none", "it runs as bloom, with no inherited token");
        assert!(runs.0.lock().unwrap().is_empty(), "the run is forgotten once it ends");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_run_that_ends_with_no_token_says_how_it_ended_and_hides_any_secret() {
        let (dir, bin) = fake_claude("none", "printf 'OAuth error: sk-ant-api03-leakedkeyleakedkeyleaked denied\\n'; exit 1");
        assert_eq!(SetupTokens::default().run("bloom", &bin, &[], Duration::from_secs(10)), Err("claude setup-token ended with no token: OAuth error: [hidden] denied".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn past_its_wait_it_kills_the_process_it_started_and_only_that_one() {
        let (dir, bin) = fake_claude("timeout", "printf 'Opening the browser\\n'\nexec sleep 30");
        let bystander = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let runs = SetupTokens::default();
        let started = Instant::now();
        let answer = runs.run("bloom", &bin, &[], Duration::from_secs(3));
        assert_eq!(answer, Err("no one approved claude setup-token in the browser within 0 min, so pinned stopped it".into()));
        assert!(started.elapsed() < Duration::from_secs(8), "it stopped at its wait: {:?}", started.elapsed());
        assert!(!alive(pid_in(&dir)), "the recorded process is gone");
        assert!(alive(bystander.id()), "another process of the same program lives on");
        let mut bystander = bystander;
        bystander.kill().unwrap();
        bystander.wait().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cancel_kills_the_recorded_process_and_a_second_run_for_the_account_waits_its_turn() {
        let (dir, bin) = fake_claude("cancel", "printf 'Opening the browser\\n'\nexec sleep 30");
        let runs = std::sync::Arc::new(SetupTokens::default());
        let running = {
            let (runs, bin) = (runs.clone(), bin.clone());
            std::thread::spawn(move || runs.run("bloom", &bin, &[], Duration::from_secs(30)))
        };
        let pid = pid_in(&dir);
        assert_eq!(
            runs.run("bloom", &bin, &[], Duration::from_secs(1)),
            Err("claude setup-token already runs for bloom on this machine: approve it in the browser it opened, or cancel it".into())
        );
        assert!(!runs.cancel("work"), "nothing runs for work");
        assert!(runs.cancel("bloom"));
        assert_eq!(running.join().unwrap(), Err("claude setup-token was cancelled".into()));
        assert!(!alive(pid), "the recorded process is gone");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_token_survives_the_terminal_codes_ink_draws_around_it() {
        let drawn = format!("\x1b[2K\x1b[1A\x1b[2K\x1b[G\x1b]8;;https://x\x07link\x1b]8;;\x07\r\n\x1b[1m{TOKEN}\x1b[22m\r\n");
        assert_eq!(token_in(drawn.as_bytes()).as_deref(), Some(TOKEN));
        assert_eq!(token_in(b"sk-ant-oat01-short"), None);
    }
}
