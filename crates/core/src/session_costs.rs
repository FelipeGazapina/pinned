//! Every Claude session's cost, turns and active time as its tab last read them, by session id,
//! kept in `session-costs.json` in the data dir for the Usage page's threads.

use crate::accounts::AccountId;
use crate::stats::{Tally, ThreadStats};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "session-costs.json";

/// A record whose last turn is older than this, the Usage page's widest window, is dropped on save.
const KEPT_SECS: u64 = 90 * 86_400;

/// One session's figures, as its tab showed them.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Record {
    /// Claude Code's session id.
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The project folder, which a worktree's session counts under.
    pub project: String,
    pub account: AccountId,
    pub turns: u32,
    /// Unknown when the entrypoint logs no turn durations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_ms: Option<u64>,
    /// Unknown until a claude process of the session has exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Unix seconds of the session's last message at its first save.
    pub first_at: u64,
    /// Unix seconds of the session's last message.
    pub last_at: u64,
}

/// The record of `session` read from `tally`, under the tab's `title`, else the first prompt's, or
/// `None` when the tally has no figures. A session with no message yet counts as last seen `now`.
pub fn record(session: &str, tally: &Tally, title: Option<String>, cwd: &Path, account: &AccountId, now: u64) -> Option<Record> {
    let stats = tally.stats();
    if stats == ThreadStats::default() {
        return None;
    }
    let last_at = tally.last_at.unwrap_or(now);
    Some(Record {
        session: session.to_owned(),
        title: title.or_else(|| tally.prompt_title.clone()),
        project: crate::history::project_of(cwd).display().to_string(),
        account: account.clone(),
        turns: tally.turns,
        active_ms: stats.active_ms,
        cost_usd: stats.cost_usd,
        first_at: last_at,
        last_at,
    })
}

#[derive(Default)]
pub struct Store {
    by_session: HashMap<String, Record>,
    /// None keeps the records in memory only.
    file: Option<PathBuf>,
}

impl Store {
    /// The records saved in `dir`, or none when the file is missing or can't be read.
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join(FILE));
        let saved: Vec<Record> = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Store { by_session: saved.into_iter().map(|r| (r.session.clone(), r)).collect(), file }
    }

    /// Puts `record` in place of its session's last one, keeping when that one was first saved,
    /// drops the records last seen more than 90 days before `now`, and writes the file. A record
    /// whose figures and title equal the last one's writes nothing, even when its last message is
    /// newer, so a working tab read every few seconds saves only as its figures change.
    pub fn save(&mut self, mut record: Record, now: u64) {
        if let Some(last) = self.by_session.get(&record.session) {
            record.first_at = last.first_at;
            if *last == (Record { last_at: last.last_at, ..record.clone() }) {
                return;
            }
        }
        self.by_session.insert(record.session.clone(), record);
        self.by_session.retain(|_, r| now.saturating_sub(r.last_at) <= KEPT_SECS);
        self.write();
    }

    /// Every record whose last message is at or after unix time `since`.
    pub fn since(&self, since: u64) -> Vec<Record> {
        self.by_session.values().filter(|r| r.last_at >= since).cloned().collect()
    }

    /// Writes a sibling file only this user may read, flushes it to disk, and renames it over the
    /// last save, so a failed or cut write keeps that one. A failure keeps the records in memory.
    fn write(&self) {
        let Some(file) = &self.file else { return };
        let saved = (|| {
            use std::io::Write;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = file.with_extension("json.tmp");
            let _ = std::fs::remove_file(&tmp);
            let records: Vec<&Record> = self.by_session.values().collect();
            let written = crate::private::file().create_new(true).open(&tmp).and_then(|mut f| {
                f.write_all(&serde_json::to_vec(&records).expect("session costs serialize"))?;
                f.sync_all()
            });
            if let Err(e) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            std::fs::rename(&tmp, file)?;
            // The rename itself lasts once the folder is flushed too.
            #[cfg(unix)]
            if let Some(dir) = file.parent() {
                std::fs::File::open(dir)?.sync_all()?;
            }
            Ok(())
        })();
        if let Err(e) = saved {
            eprintln!("pinned: session costs did not save: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_793_491_210;
    const DAY: u64 = 86_400;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-session-costs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn spent(session: &str, turns: u32, cost_usd: Option<f64>, last_at: u64) -> Record {
        Record { session: session.into(), title: Some("Fix the build".into()), project: "/repo".into(), account: "claude".into(), turns, active_ms: Some(60_000 * turns as u64), cost_usd, first_at: last_at, last_at }
    }

    #[test]
    fn a_saved_session_cost_loads_again_after_a_restart() {
        let dir = temp("restart");
        let mut store = Store::load(Some(&dir));
        store.save(spent("s1", 2, Some(1.25), NOW), NOW);
        drop(store);
        assert_eq!(Store::load(Some(&dir)).since(0), vec![spent("s1", 2, Some(1.25), NOW)]);
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.join(FILE)).unwrap().permissions()) & 0o777, 0o600, "titles are the user's own text");
        assert!(!dir.join("session-costs.json.tmp").exists(), "no sibling file is left behind");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_second_save_of_a_session_cost_replaces_its_totals_and_keeps_its_first_save_time() {
        let mut store = Store::load(None);
        store.save(spent("s1", 1, None, NOW - 600), NOW - 600);
        store.save(spent("s1", 3, Some(2.5), NOW), NOW);
        assert_eq!(store.since(0), vec![Record { first_at: NOW - 600, ..spent("s1", 3, Some(2.5), NOW) }]);
    }

    #[test]
    fn a_session_cost_whose_figures_did_not_move_is_not_written_again_when_only_its_last_message_did() {
        let dir = temp("unmoved");
        let mut store = Store::load(Some(&dir));
        store.save(spent("s1", 2, Some(1.25), NOW - 60), NOW - 60);
        std::fs::remove_file(dir.join(FILE)).unwrap();
        store.save(spent("s1", 2, Some(1.25), NOW), NOW);
        assert!(!dir.join(FILE).exists(), "a newer last message alone writes nothing");
        assert_eq!(store.since(0), vec![spent("s1", 2, Some(1.25), NOW - 60)]);
        store.save(spent("s1", 3, Some(1.25), NOW), NOW);
        assert_eq!(Store::load(Some(&dir)).since(0), vec![Record { first_at: NOW - 60, ..spent("s1", 3, Some(1.25), NOW) }], "a moved figure writes, with the newer last message");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_session_cost_last_seen_91_days_ago_is_dropped_on_the_next_save() {
        let dir = temp("prune");
        let mut store = Store::load(Some(&dir));
        store.save(spent("old", 1, Some(1.0), NOW - 91 * DAY), NOW - 91 * DAY);
        store.save(spent("edge", 1, Some(1.0), NOW - 90 * DAY), NOW - 90 * DAY);
        store.save(spent("new", 1, Some(1.0), NOW), NOW);
        let mut kept: Vec<String> = Store::load(Some(&dir)).since(0).into_iter().map(|r| r.session).collect();
        kept.sort();
        assert_eq!(kept, ["edge", "new"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn session_costs_since_keeps_the_records_last_seen_at_or_after_it() {
        let mut store = Store::load(None);
        store.save(spent("before", 1, Some(1.0), NOW - 2 * DAY), NOW);
        store.save(spent("at", 1, Some(1.0), NOW - DAY), NOW);
        store.save(spent("after", 1, Some(1.0), NOW), NOW);
        let mut since: Vec<String> = store.since(NOW - DAY).into_iter().map(|r| r.session).collect();
        since.sort();
        assert_eq!(since, ["after", "at"]);
    }

    #[test]
    fn an_unknown_session_cost_or_active_time_is_left_out_not_zero() {
        let unknown = Record { title: None, active_ms: None, ..spent("s1", 0, None, NOW) };
        assert_eq!(
            serde_json::to_value(&unknown).unwrap(),
            serde_json::json!({ "session": "s1", "project": "/repo", "account": "claude", "turns": 0, "first_at": NOW, "last_at": NOW })
        );
        let mut tally = Tally::default();
        assert_eq!(record("s1", &tally, None, Path::new("/repo"), &"claude".into(), NOW), None, "a tally with no figures makes no record");
        tally.scan(b"{\"type\":\"cost-state\",\"startTime\":1,\"totalCostUSD\":0.5}\n");
        let saved = record("s1", &tally, None, Path::new("/repo/.claude/worktrees/eng-7"), &"claude".into(), NOW).unwrap();
        assert_eq!((saved.cost_usd, saved.active_ms, saved.project.as_str(), saved.last_at), (Some(0.5), None, "/repo", NOW));
    }
}
