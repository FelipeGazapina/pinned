//! Keeps the reports' states in step with GitHub, so "My reports" shows what happened to each issue: a maintainer
//! answered, it was closed (and why), it was fixed in a release, or it was reopened.
//!
//! The hub asks GitHub about the issues changed since the last poll, with one search whose query holds no word of
//! any report. Nothing GitHub says is copied into a row except from a fixed set: a state, a comment's time, a
//! reason from a short list and a version that reads as `x.y.z`.

use super::github::{Comment, Error, Github, Release, Tracked};
use super::hub::Config;
use crate::db::Db;
use crate::hub::Hub;
use pinned_core::reports::ReportState;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// The time of the last successful poll, one row. A restart keeps it.
pub const SCHEMA: &str = "
create table if not exists reports_poll (one boolean primary key default true check (one), polled_at bigint not null);
";

/// How far before the last poll the next search looks, for an index that lags and a clock that is not GitHub's.
const MARGIN_SECS: i64 = 120;
const PER_PAGE: usize = 100;
/// Most pages of search results one poll reads.
const MAX_PAGES: u32 = 10;
/// Who, when they comment, answers a report: the people who run the repository.
const ANSWERERS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];
const FIXED_IN: &str = "fixed-in:";
/// Longest version kept, in characters.
const VERSION_CHARS: usize = 32;

/// Polls every `config.poll`, for as long as the hub lives.
pub fn spawn(hub: &Arc<Hub>, config: Config) {
    let hub = Arc::downgrade(hub);
    let github = Github::with_api(&config.api, &config.repo, &config.token).with_timeout(config.timeout);
    let every = config.poll.max(Duration::from_millis(100));
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tick.tick().await;
            let Some(hub) = hub.upgrade() else { break };
            if let Err(e) = poll_once(&hub.db, &github, now_secs()).await {
                eprintln!("pinned-server: reports poll failed, no report changed, trying again in {}s: {e}", every.as_secs());
            }
        }
    });
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A tracked report's row, as far as the poller reads it.
struct Row {
    id: Uuid,
    issue: u64,
    /// As stored, to write the new state only over the one that was read.
    stored: Value,
    state: ReportState,
    sent_at: i64,
    seen_comments: i64,
    own_comments: Vec<i64>,
}

/// What a poll decided for one row.
struct Change {
    id: Uuid,
    stored: Value,
    state: ReportState,
    seen_comments: i64,
    state_changed: bool,
    seen_changed: bool,
}

/// What the log says of a failure: the kind, never a token or anything GitHub wrote.
fn reason(error: &Error) -> String {
    match error {
        Error::Unauthorized => "GitHub refused the token".into(),
        Error::Failed(text) | Error::Busy(text) | Error::Refused { text, .. } | Error::Unreachable(text) | Error::TimedOut(text) if text.to_lowercase().contains("rate limit") => {
            "GitHub rate limit reached".into()
        }
        Error::Failed(text) | Error::Busy(text) | Error::Refused { text, .. } | Error::Unreachable(text) | Error::TimedOut(text) => text.split(':').next().unwrap_or_default().chars().take(60).collect(),
    }
}

fn github_failed(error: Error) -> String {
    reason(&error)
}

fn db_failed(error: tokio_postgres::Error) -> String {
    format!("database: {error}")
}

/// One poll: reads what changed on GitHub, then writes every row's new state, or none when GitHub failed.
pub async fn poll_once(db: &Db, github: &Github, started: i64) -> Result<(), String> {
    let client = db.client().await?;
    let rows = tracked_rows(&client).await?;
    let last: Option<i64> = client.query_opt("select polled_at from reports_poll", &[]).await.map_err(db_failed)?.map(|r| r.get(0));
    let Some(oldest) = rows.iter().map(|r| r.sent_at).min() else {
        // Nothing waits: the next search starts from here, not from a time long past.
        return save_poll(&client, started).await;
    };
    let since = last.unwrap_or(oldest) - MARGIN_SECS;
    let mut changed: Vec<Tracked> = vec![];
    for page in 1..=MAX_PAGES {
        let found = github.search_updated(&iso(since), PER_PAGE, page).await.map_err(github_failed)?;
        let full = found.len() >= PER_PAGE;
        changed.extend(found);
        if !full {
            break;
        }
    }
    let mut by_issue: BTreeMap<u64, Vec<&Row>> = BTreeMap::new();
    for row in &rows {
        by_issue.entry(row.issue).or_default().push(row);
    }
    let mut releases: Option<Vec<Release>> = None;
    let mut changes: Vec<Change> = vec![];
    for (number, group) in &by_issue {
        // A closed issue that waits for a release is looked at again though it did not change.
        let waiting = group.iter().any(|r| r.state == ReportState::Closed { reason: "completed".into() });
        let issue = match changed.iter().find(|t| t.number == *number) {
            Some(issue) => issue.clone(),
            None if waiting => match github.tracked(*number).await.map_err(github_failed)? {
                Some(issue) => issue,
                None => continue,
            },
            None => continue,
        };
        changes.extend(judge(github, &issue, group, &mut releases).await?);
    }
    for change in &changes {
        write(&client, change, started).await?;
    }
    save_poll(&client, started).await
}

async fn tracked_rows(client: &tokio_postgres::Client) -> Result<Vec<Row>, String> {
    // Open, answered, closed as completed and so maybe not yet in a release, or fixed, for a version the owner
    // labels by hand later. Any account's.
    let found = client
        .query(
            "select id, issue, state, sent_at, seen_comments, own_comments from reports
             where issue is not null and sent_at is not null
               and (state->>'state' in ('open', 'answered', 'fixed') or (state->>'state' = 'closed' and state->>'reason' = 'completed'))",
            &[],
        )
        .await
        .map_err(db_failed)?;
    Ok(found
        .iter()
        .filter_map(|r| {
            let stored: Value = r.get(2);
            Some(Row {
                id: r.get(0),
                issue: u64::try_from(r.get::<_, i64>(1)).ok()?,
                state: serde_json::from_value(stored.clone()).ok()?,
                stored,
                sent_at: r.get(3),
                seen_comments: i64::from(r.get::<_, i32>(4)),
                own_comments: r.get(5),
            })
        })
        .collect())
}

async fn save_poll(client: &tokio_postgres::Client, at: i64) -> Result<(), String> {
    client.execute("insert into reports_poll (one, polled_at) values (true, $1) on conflict (one) do update set polled_at = excluded.polled_at", &[&at]).await.map_err(db_failed)?;
    Ok(())
}

/// Writes a change. `updated_at` moves only when the state does, and only over the state that was read.
async fn write(client: &tokio_postgres::Client, change: &Change, now: i64) -> Result<(), String> {
    let seen = i32::try_from(change.seen_comments).unwrap_or(i32::MAX);
    if change.state_changed {
        let state = serde_json::to_value(&change.state).map_err(|e| e.to_string())?;
        client.execute("update reports set state = $2, seen_comments = $3, updated_at = $4 where id = $1 and state = $5", &[&change.id, &state, &seen, &now, &change.stored]).await.map_err(db_failed)?;
    } else if change.seen_changed {
        client.execute("update reports set seen_comments = $2 where id = $1 and state = $3", &[&change.id, &seen, &change.stored]).await.map_err(db_failed)?;
    }
    Ok(())
}

/// What each row of one issue becomes, given the issue as GitHub has it now.
async fn judge(github: &Github, issue: &Tracked, rows: &[&Row], releases: &mut Option<Vec<Release>>) -> Result<Vec<Change>, String> {
    // The hub comments with the owner's token, so every comment it posted, from any account's row, reads as the
    // owner's: none of them is an answer.
    let ours: HashSet<i64> = rows.iter().flat_map(|r| r.own_comments.iter().copied()).collect();
    let mut comments: Option<Vec<Comment>> = None;
    let mut fix: Option<Option<String>> = None;
    let mut changes = vec![];
    for row in rows {
        let (mut state, mut seen) = (row.state.clone(), row.seen_comments);
        if let ReportState::Fixed { version: was } = &row.state {
            // A fixed report stays where it is, but for a version the owner labels by hand on the issue, still
            // closed as completed, later.
            let completed = !issue.open && issue.state_reason.as_deref() == Some("completed");
            if let Some(version) = labelled(issue).filter(|v| completed && v != was) {
                changes.push(Change { id: row.id, stored: row.stored.clone(), state: ReportState::Fixed { version }, seen_comments: seen, state_changed: true, seen_changed: false });
            }
            continue;
        }
        if issue.open {
            if !matches!(state, ReportState::Answered { .. }) {
                let reopened = matches!(state, ReportState::Closed { .. });
                state = ReportState::Open;
                if reopened || i64::try_from(issue.comments).unwrap_or(i64::MAX) > row.seen_comments {
                    if comments.is_none() {
                        let since = rows.iter().map(|r| r.sent_at).min().unwrap_or(row.sent_at);
                        comments = Some(github.comments_since(issue.number, &iso(since)).await.map_err(github_failed)?);
                    }
                    if let Some(at) = answer(comments.as_deref().unwrap_or_default(), row.sent_at, &ours) {
                        state = ReportState::Answered { at };
                    }
                    seen = i64::try_from(issue.comments).unwrap_or(i64::MAX);
                }
            }
        } else {
            state = match issue.state_reason.as_deref() {
                Some("completed") => {
                    if fix.is_none() {
                        fix = Some(fixed_in(github, issue, releases).await?);
                    }
                    match fix.clone().flatten() {
                        Some(version) => ReportState::Fixed { version },
                        None => ReportState::Closed { reason: "completed".into() },
                    }
                }
                Some("not_planned") => ReportState::Closed { reason: "not_planned".into() },
                Some("duplicate") => ReportState::Closed { reason: "duplicate".into() },
                _ => ReportState::Closed { reason: "closed".into() },
            };
        }
        let (state_changed, seen_changed) = (state != row.state, seen != row.seen_comments);
        if state_changed || seen_changed {
            changes.push(Change { id: row.id, stored: row.stored.clone(), state, seen_comments: seen, state_changed, seen_changed });
        }
    }
    Ok(changes)
}

/// The time of the first comment a maintainer wrote after the report was sent, that the hub did not write itself.
fn answer(comments: &[Comment], sent_at: i64, ours: &HashSet<i64>) -> Option<i64> {
    comments
        .iter()
        .filter(|c| ANSWERERS.contains(&c.author_association.as_str()) && i64::try_from(c.id).is_ok_and(|id| !ours.contains(&id)))
        .filter_map(|c| epoch(&c.created_at))
        .filter(|at| *at >= sent_at)
        .min()
}

/// The release that carries the fix of an issue closed as completed: a `fixed-in:x.y.z` label set by hand, else the
/// first release published after the fix landed (the closing commit, or the merged pull request that mentioned it).
async fn fixed_in(github: &Github, issue: &Tracked, releases: &mut Option<Vec<Release>>) -> Result<Option<String>, String> {
    if let Some(version) = labelled(issue) {
        return Ok(Some(version));
    }
    let timeline = github.timeline(issue.number).await.map_err(github_failed)?;
    // The last of them: the fix is in once all the work that mentions the issue is.
    let landed = timeline
        .iter()
        .filter_map(|e| match (&e.pull, &e.commit_id) {
            (Some(pull), _) => pull.merged_at.as_deref().and_then(epoch),
            (None, Some(_)) if e.event == "closed" => e.created_at.as_deref().and_then(epoch),
            _ => None,
        })
        .max();
    let Some(landed) = landed else { return Ok(None) };
    if releases.is_none() {
        *releases = Some(github.releases().await.map_err(github_failed)?);
    }
    Ok(releases
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| Some((epoch(r.published_at.as_deref()?)?, version(&r.tag_name)?)))
        .filter(|(at, _)| *at > landed)
        .min_by_key(|(at, _)| *at)
        .map(|(_, version)| version))
}

/// The version of the issue's `fixed-in:x.y.z` label, set by hand.
fn labelled(issue: &Tracked) -> Option<String> {
    issue.labels.iter().find_map(|l| l.strip_prefix(FIXED_IN).and_then(version))
}

/// `x.y.z` of `v?x.y.z`, or None for anything else.
fn version(text: &str) -> Option<String> {
    let bare = text.strip_prefix('v').unwrap_or(text);
    let parts: Vec<&str> = bare.split('.').collect();
    let numbers = parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    (numbers && bare.chars().count() <= VERSION_CHARS).then(|| bare.to_owned())
}

/// `2026-03-02T10:00:00Z` for a count of seconds since the Unix epoch.
pub fn iso(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Seconds since the Unix epoch of `2026-03-02T10:00:00Z`, the form GitHub writes times in.
fn epoch(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let num = |from: usize, to: usize| iso.get(from..to)?.parse::<i64>().ok();
    let (year, month, day, hour, minute, second) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Days from a civil date (Howard Hinnant's algorithm).
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_x_y_z_with_or_without_the_v_and_nothing_else() {
        assert_eq!(version("v0.18.7").as_deref(), Some("0.18.7"));
        assert_eq!(version("1.2.3").as_deref(), Some("1.2.3"));
        for bad in ["latest", "nightly", "v1.2", "1.2.3.4", "1.2.x", "v1.2.3-rc1", "", "vv1.2.3", "1..3", " 1.2.3", "1.2.3\n", "+1.2.3"] {
            assert_eq!(version(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn github_times_read_back_as_the_seconds_they_were_made_from() {
        for at in [0, 1_767_225_600, 1_772_445_600, 1_709_251_199, 4_102_444_800] {
            assert_eq!(epoch(&iso(at)), Some(at), "{at}");
        }
        assert_eq!(epoch("2026-03-02T10:00:00Z"), Some(1_772_445_600));
        for bad in ["", "2026-03-02", "2026-03-02T10:00:00+00:00", "2026-13-02T10:00:00Z", "soon"] {
            assert_eq!(epoch(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_failure_is_logged_by_its_kind_and_never_by_what_github_wrote() {
        assert_eq!(reason(&Error::Unauthorized), "GitHub refused the token");
        assert_eq!(reason(&Error::Failed("GitHub rate limit reached, try again later: secret words".into())), "GitHub rate limit reached");
        assert_eq!(reason(&Error::Failed("GitHub answered 500 Internal Server Error: secret words".into())), "GitHub answered 500 Internal Server Error");
        assert_eq!(reason(&Error::Failed("GitHub unreachable: connection refused".into())), "GitHub unreachable");
    }

    fn comment(id: u64, at: &str, association: &str) -> Comment {
        Comment { id, created_at: at.into(), user_login: String::new(), author_association: association.into() }
    }

    #[test]
    fn only_a_maintainer_comment_the_hub_did_not_write_answers() {
        let sent = epoch("2026-03-02T10:00:00Z").unwrap();
        let ours: HashSet<i64> = [2].into();
        let all = [
            comment(1, "2026-03-02T10:00:01Z", "NONE"),
            comment(2, "2026-03-02T10:00:02Z", "OWNER"),
            comment(3, "2026-03-02T09:59:59Z", "OWNER"),
            comment(4, "2026-03-02T10:00:05Z", "COLLABORATOR"),
            comment(5, "2026-03-02T10:00:04Z", "MEMBER"),
        ];
        assert_eq!(answer(&all, sent, &ours), epoch("2026-03-02T10:00:04Z"));
        assert_eq!(answer(&all[..3], sent, &ours), None);
    }
}
