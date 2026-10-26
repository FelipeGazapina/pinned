//! Pull requests a thread watches, as T3 Code's pull request watch does: the machine reads each with
//! gh now and then, and when its checks finish, someone comments, or its branch starts to conflict,
//! it tells the thread's claude once claude's turn is over.

use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// What a watch remembers of a pull request between reads.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub number: u64,
    pub url: String,
    pub open: bool,
    pub head: String,
    pub base: String,
    /// Each check by name, `None` while it runs, else whether it failed, with its link.
    pub checks: Vec<(String, Option<bool>, Option<String>)>,
    /// Comments and reviews, by who and when, with what they said.
    pub remarks: Vec<Remark>,
    pub conflicting: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Remark {
    pub key: String,
    pub author: String,
    pub said: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    ChecksFailed(Vec<(String, Option<String>)>),
    ChecksPassed(usize),
    Remarks(Vec<Remark>),
    Conflicting,
}

const FAILED: &[&str] = &["FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE", "ERROR"];

/// The fields `snapshot` reads, for `gh pr view --json`.
pub const FIELDS: &str = "number,url,state,headRefOid,baseRefName,mergeable,statusCheckRollup,comments,reviews";

pub fn snapshot(view: &Value) -> Snapshot {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let checks = view["statusCheckRollup"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let name = c["name"].as_str().or(c["context"].as_str()).unwrap_or("check").to_owned();
            let conclusion = c["conclusion"].as_str().or(c["state"].as_str()).unwrap_or_default().to_ascii_uppercase();
            let done = c["status"].as_str().map_or(!matches!(conclusion.as_str(), "PENDING" | "EXPECTED" | ""), |st| st == "COMPLETED");
            let link = c["detailsUrl"].as_str().or(c["targetUrl"].as_str()).map(str::to_owned);
            (name, done.then(|| FAILED.contains(&conclusion.as_str())), link)
        })
        .collect();
    let remark = |r: &Value, at: &str| {
        let author = s(&r["author"]["login"]);
        Remark { key: format!("{author}@{}", s(&r[at])), said: s(&r["body"]).split_whitespace().collect::<Vec<_>>().join(" "), author }
    };
    let remarks = view["comments"].as_array().into_iter().flatten().map(|c| remark(c, "createdAt"))
        .chain(view["reviews"].as_array().into_iter().flatten().map(|r| remark(r, "submittedAt")))
        .collect();
    Snapshot {
        number: view["number"].as_u64().unwrap_or_default(),
        url: s(&view["url"]),
        open: view["state"].as_str() == Some("OPEN"),
        head: s(&view["headRefOid"]),
        base: s(&view["baseRefName"]),
        checks,
        remarks,
        conflicting: view["mergeable"].as_str() == Some("CONFLICTING"),
    }
}

/// What changed from `before` to `now` that claude should hear about.
pub fn changes(before: &Snapshot, now: &Snapshot) -> Vec<Change> {
    let mut out = vec![];
    let same_head = before.head == now.head;
    let failed_before: BTreeSet<&str> = before.checks.iter().filter(|c| same_head && c.1 == Some(true)).map(|c| c.0.as_str()).collect();
    let failed: Vec<(String, Option<String>)> = now.checks.iter().filter(|c| c.1 == Some(true) && !failed_before.contains(c.0.as_str())).map(|c| (c.0.clone(), c.2.clone())).collect();
    if !failed.is_empty() {
        out.push(Change::ChecksFailed(failed));
    }
    let settled = |s: &Snapshot| !s.checks.is_empty() && s.checks.iter().all(|c| c.1.is_some());
    let passed = settled(now) && now.checks.iter().all(|c| c.1 == Some(false));
    if passed && !(same_head && settled(before) && before.checks.iter().all(|c| c.1 == Some(false))) {
        out.push(Change::ChecksPassed(now.checks.len()));
    }
    let seen: BTreeSet<&str> = before.remarks.iter().map(|r| r.key.as_str()).collect();
    let fresh: Vec<Remark> = now.remarks.iter().filter(|r| !seen.contains(r.key.as_str())).cloned().collect();
    if !fresh.is_empty() {
        out.push(Change::Remarks(fresh));
    }
    if now.conflicting && !before.conflicting {
        out.push(Change::Conflicting);
    }
    out
}

const LISTED: usize = 5;
const SNIPPET: usize = 160;

/// What claude reads when it wakes, in T3's words for its own wake.
pub fn message(now: &Snapshot, changes: &[Change]) -> String {
    let commit = if now.head.is_empty() { String::new() } else { format!(" on {}", &now.head[..now.head.len().min(7)]) };
    let mut lines = vec![format!("Update on pull request #{} ({}), which Pinned is watching for you:", now.number, now.url), String::new()];
    let listed = |lines: &mut Vec<String>, items: Vec<String>| {
        let more = items.len().saturating_sub(LISTED);
        lines.extend(items.into_iter().take(LISTED));
        if more > 0 {
            lines.push(format!("  - and {more} more"));
        }
    };
    for change in changes {
        match change {
            Change::ChecksFailed(failed) => {
                lines.push(format!("- Checks failed{commit}:"));
                listed(&mut lines, failed.iter().map(|(name, link)| format!("  - {name}{}", link.as_ref().map(|l| format!(" {l}")).unwrap_or_default())).collect());
            }
            Change::ChecksPassed(n) => lines.push(format!("- All {n} {} passed{commit}.", if *n == 1 { "check" } else { "checks" })),
            Change::Remarks(remarks) => {
                lines.push(format!("- {} new {}:", remarks.len(), if remarks.len() == 1 { "comment" } else { "comments" }));
                listed(&mut lines, remarks.iter().map(|r| {
                    let said = if r.said.chars().count() > SNIPPET { format!("{}...", r.said.chars().take(SNIPPET - 3).collect::<String>()) } else { r.said.clone() };
                    format!("  - {}: {}", if r.author.is_empty() { "someone" } else { &r.author }, if said.is_empty() { "reviewed".to_owned() } else { format!("\"{said}\"") })
                }).collect());
            }
            Change::Conflicting => lines.push(format!("- The branch now conflicts with {}.", if now.base.is_empty() { "its base" } else { &now.base })),
        }
    }
    lines.push(String::new());
    lines.push("Look into each item and act on it as your task requires. Pinned keeps watching and wakes you on the next change, so end your turn when you are done.".into());
    lines.join("\n")
}

/// Each tab's watched pull requests, by URL: the last read, and what claude hasn't heard yet.
#[derive(Default)]
pub struct Watches {
    pub tabs: Mutex<HashMap<String, Vec<Watch>>>,
    pub started: std::sync::atomic::AtomicBool,
}

#[derive(Clone, Debug)]
pub struct Watch {
    pub url: String,
    pub seen: Option<Snapshot>,
    pub pending: Vec<Change>,
}

impl Watches {
    /// Starts or stops watching `url` in `tab`, and answers the tab's watched URLs.
    pub fn set(&self, tab: &str, url: &str, watching: bool) -> Vec<String> {
        let mut tabs = self.tabs.lock().unwrap();
        let list = tabs.entry(tab.to_owned()).or_default();
        list.retain(|w| w.url != url);
        if watching {
            list.push(Watch { url: url.to_owned(), seen: None, pending: vec![] });
        }
        list.iter().map(|w| w.url.clone()).collect()
    }

    pub fn of(&self, tab: &str) -> Vec<String> {
        self.tabs.lock().unwrap().get(tab).map(|l| l.iter().map(|w| w.url.clone()).collect()).unwrap_or_default()
    }

    /// Takes a fresh read of a watched pull request. A closed or merged one stops being watched.
    pub fn update(&self, tab: &str, url: &str, now: Snapshot) {
        let mut tabs = self.tabs.lock().unwrap();
        let Some(list) = tabs.get_mut(tab) else { return };
        let Some(watch) = list.iter_mut().find(|w| w.url == url) else { return };
        if let Some(before) = &watch.seen {
            for change in changes(before, &now) {
                // A newer comment list or check run replaces the one claude hasn't heard yet.
                watch.pending.retain(|c| std::mem::discriminant(c) != std::mem::discriminant(&change));
                watch.pending.push(change);
            }
        }
        let open = now.open;
        watch.seen = Some(now);
        if !open && watch.pending.is_empty() {
            list.retain(|w| w.url != url);
        }
    }

    /// The wake for a tab whose claude is free, taking it off the watch.
    pub fn take_wake(&self, tab: &str) -> Option<String> {
        let mut tabs = self.tabs.lock().unwrap();
        let list = tabs.get_mut(tab)?;
        let watch = list.iter_mut().find(|w| !w.pending.is_empty())?;
        let text = message(watch.seen.as_ref()?, &watch.pending);
        watch.pending.clear();
        if !watch.seen.as_ref().is_some_and(|s| s.open) {
            let url = watch.url.clone();
            list.retain(|w| w.url != url);
        }
        Some(text)
    }

    pub fn forget(&self, keep: &[String]) {
        self.tabs.lock().unwrap().retain(|tab, list| keep.contains(tab) && !list.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(checks: Value, comments: Value, mergeable: &str) -> Value {
        json!({ "number": 41, "url": "https://github.com/o/r/pull/41", "state": "OPEN", "headRefOid": "abc1234def", "baseRefName": "main", "mergeable": mergeable,
            "statusCheckRollup": checks, "comments": comments, "reviews": [] })
    }

    #[test]
    fn a_watch_wakes_claude_when_checks_finish_someone_comments_or_the_branch_conflicts() {
        let watches = Watches::default();
        assert_eq!(watches.set("tab-1", "https://github.com/o/r/pull/41", true), vec!["https://github.com/o/r/pull/41".to_owned()]);
        let url = "https://github.com/o/r/pull/41";
        let running = json!([{ "name": "build", "status": "IN_PROGRESS", "conclusion": "" }, { "name": "lint", "status": "IN_PROGRESS", "conclusion": "" }]);
        watches.update("tab-1", url, snapshot(&view(running, json!([]), "MERGEABLE")));
        assert_eq!(watches.take_wake("tab-1"), None, "the first read only sets what was seen");

        let failed = json!([{ "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" }, { "name": "lint", "status": "COMPLETED", "conclusion": "FAILURE", "detailsUrl": "https://ci/2" }]);
        let comments = json!([{ "author": { "login": "monalisa" }, "body": "Why  a\nsecond line?", "createdAt": "2026-01-01T10:00:00Z" }]);
        watches.update("tab-1", url, snapshot(&view(failed.clone(), comments.clone(), "CONFLICTING")));
        assert_eq!(
            watches.take_wake("tab-1").unwrap(),
            "Update on pull request #41 (https://github.com/o/r/pull/41), which Pinned is watching for you:\n\n\
             - Checks failed on abc1234:\n  - lint https://ci/2\n\
             - 1 new comment:\n  - monalisa: \"Why a second line?\"\n\
             - The branch now conflicts with main.\n\n\
             Look into each item and act on it as your task requires. Pinned keeps watching and wakes you on the next change, so end your turn when you are done."
        );
        watches.update("tab-1", url, snapshot(&view(failed, comments, "CONFLICTING")));
        assert_eq!(watches.take_wake("tab-1"), None, "nothing new, no wake");

        let passed = json!([{ "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" }, { "name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS" }]);
        let mut merged = view(passed, json!([]), "MERGEABLE");
        merged["headRefOid"] = "fff0000".into();
        watches.update("tab-1", url, snapshot(&merged));
        assert!(watches.take_wake("tab-1").unwrap().contains("- All 2 checks passed on fff0000."));
        merged["state"] = "MERGED".into();
        watches.update("tab-1", url, snapshot(&merged));
        assert_eq!(watches.of("tab-1"), Vec::<String>::new(), "a merged pull request stops being watched");
    }
}
