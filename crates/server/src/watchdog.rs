//! Ends the hub when its runtime stops answering, so the platform restarts it.
//!
//! On 2026-11-24 every tokio worker blocked on one account's store: the process kept running with
//! no thread driving its sockets, and nothing restarted it, since a restart policy acts only on
//! an exit. This thread lives outside the runtime. It watches a beat that a runtime task moves,
//! and asks the hub's own `GET /api/version` over loopback. When either has not moved or answered
//! for `STUCK_AFTER`, it logs one line and ends the process.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const CHECK_EVERY: Duration = Duration::from_secs(10);
pub const STUCK_AFTER: Duration = Duration::from_secs(60);

/// When the watchdog looks, and how long either signal may stay still.
#[derive(Clone, Copy)]
pub struct Timing {
    pub every: Duration,
    pub stuck_after: Duration,
}

impl Timing {
    pub const HUB: Timing = Timing { every: CHECK_EVERY, stuck_after: STUCK_AFTER };

    /// The beat moves twice per check, so one late wake-up is not a still beat.
    fn beat_every(self) -> Duration {
        self.every / 2
    }

    /// A request that waits longer than a check would hold the next one up.
    fn probe_wait(self) -> Duration {
        self.every.min(Duration::from_secs(5))
    }
}

/// The last time the beat moved and the hub answered.
pub struct Stillness {
    beat: u64,
    moved_at: Instant,
    answered_at: Instant,
}

impl Stillness {
    pub fn new(now: Instant) -> Stillness {
        Stillness { beat: 0, moved_at: now, answered_at: now }
    }

    /// Takes one check's beat and answer, and says why the hub is stuck when it is.
    pub fn check(&mut self, now: Instant, beat: u64, answered: bool, stuck_after: Duration) -> Option<String> {
        if beat != self.beat {
            self.beat = beat;
            self.moved_at = now;
        }
        if answered {
            self.answered_at = now;
        }
        let still = now.duration_since(self.moved_at);
        let silent = now.duration_since(self.answered_at);
        if still > stuck_after {
            return Some(format!("the runtime's beat has not moved for {}s", still.as_secs()));
        }
        if silent > stuck_after {
            return Some(format!("GET /api/version has not answered for {}s", silent.as_secs()));
        }
        None
    }
}

/// A started watchdog. `disarm` before a deliberate stop, whose closed sockets would look stuck.
pub struct Watchdog {
    armed: Arc<AtomicBool>,
}

impl Watchdog {
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

/// Moves the beat on `runtime` and watches it and `addr` from a thread of its own. `stuck` gets
/// the reason once, when the hub is stuck; the hub's exits the process.
pub fn start(runtime: &tokio::runtime::Handle, addr: SocketAddr, timing: Timing, stuck: impl FnOnce(String) + Send + 'static) -> Watchdog {
    let beat = Arc::new(AtomicU64::new(0));
    let armed = Arc::new(AtomicBool::new(true));
    {
        let beat = beat.clone();
        runtime.spawn(async move {
            loop {
                beat.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(timing.beat_every()).await;
            }
        });
    }
    let target = loopback(addr);
    let watching = armed.clone();
    std::thread::Builder::new()
        .name("pinned-watchdog".into())
        .spawn(move || {
            let mut stillness = Stillness::new(Instant::now());
            loop {
                std::thread::sleep(timing.every);
                if !watching.load(Ordering::SeqCst) {
                    return;
                }
                let answered = answers(target, timing.probe_wait());
                if let Some(why) = stillness.check(Instant::now(), beat.load(Ordering::Relaxed), answered, timing.stuck_after) {
                    if watching.load(Ordering::SeqCst) {
                        stuck(why);
                    }
                    return;
                }
            }
        })
        .expect("the watchdog thread");
    Watchdog { armed }
}

/// The hub's own address over loopback, when it listens on every address.
fn loopback(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port()),
        IpAddr::V6(ip) if ip.is_unspecified() => SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port()),
        _ => addr,
    }
}

/// Whether `GET /api/version` answers 200 within `wait`.
fn answers(addr: SocketAddr, wait: Duration) -> bool {
    let ask = || -> std::io::Result<bool> {
        let mut stream = TcpStream::connect_timeout(&addr, wait)?;
        stream.set_read_timeout(Some(wait))?;
        stream.set_write_timeout(Some(wait))?;
        stream.write_all(b"GET /api/version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut head = [0u8; 12];
        stream.read_exact(&mut head)?;
        Ok(head.starts_with(b"HTTP/1.1 200") || head.starts_with(b"HTTP/1.0 200"))
    };
    ask().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::Mutex;

    #[test]
    fn a_still_beat_or_a_silent_hub_is_stuck_only_past_the_limit() {
        let limit = Duration::from_secs(60);
        let start = Instant::now();
        let mut stillness = Stillness::new(start);
        assert_eq!(stillness.check(start + Duration::from_secs(10), 1, true, limit), None);
        assert_eq!(stillness.check(start + Duration::from_secs(70), 1, true, limit), None, "the beat moved at 10 s");
        assert_eq!(stillness.check(start + Duration::from_secs(71), 1, true, limit), Some("the runtime's beat has not moved for 61s".into()));

        let mut stillness = Stillness::new(start);
        assert_eq!(stillness.check(start + Duration::from_secs(10), 1, false, limit), None);
        assert_eq!(stillness.check(start + Duration::from_secs(60), 2, false, limit), None, "silent for exactly the limit");
        assert_eq!(stillness.check(start + Duration::from_secs(61), 3, false, limit), Some("GET /api/version has not answered for 61s".into()));
    }

    /// A one-worker runtime serving `GET /api/version` as the hub does, and its address.
    fn serving() -> (tokio::runtime::Runtime, SocketAddr) {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let listener = runtime.block_on(tokio::net::TcpListener::bind("0.0.0.0:0")).unwrap();
        let addr = listener.local_addr().unwrap();
        runtime.spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut request = [0u8; 256];
                    let _ = socket.read(&mut request).await;
                    let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 6\r\nconnection: close\r\n\r\n0.18.x").await;
                });
            }
        });
        (runtime, addr)
    }

    const FAST: Timing = Timing { every: Duration::from_millis(200), stuck_after: Duration::from_secs(2) };

    #[test]
    fn the_watchdog_stays_quiet_while_the_runtime_answers_and_fires_once_it_blocks() {
        let (runtime, addr) = serving();
        let (said, stuck) = mpsc::channel();
        let _watchdog = start(runtime.handle(), addr, FAST, move |why| said.send(why).unwrap());
        assert!(stuck.recv_timeout(Duration::from_secs(3)).is_err(), "the watchdog fired on a runtime that answers");

        // The only worker waits on a lock this test holds, as the hub's workers waited on a store.
        let store = Arc::new(Mutex::new(()));
        let held = store.lock().unwrap();
        let waiting = store.clone();
        runtime.spawn(async move {
            drop(waiting.lock().unwrap());
        });
        let why = stuck.recv_timeout(Duration::from_secs(10)).expect("the watchdog did not fire on a blocked runtime");
        assert!(why.contains("has not moved") || why.contains("has not answered"), "{why}");
        drop(held);
        runtime.shutdown_background();
    }

    #[test]
    fn a_disarmed_watchdog_never_fires() {
        let (runtime, addr) = serving();
        let (said, stuck) = mpsc::channel();
        let watchdog = start(runtime.handle(), addr, FAST, move |why| said.send(why).unwrap());
        watchdog.disarm();
        runtime.shutdown_background();
        assert!(stuck.recv_timeout(Duration::from_secs(1)).is_err(), "a disarmed watchdog fired");
    }
}
