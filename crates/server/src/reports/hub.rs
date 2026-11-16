//! The hub's side of "report an issue from the app": a person (the dialog) or an agent (with the person's
//! confirmation) files an issue in pinned's public GitHub repository. The hub files it with its own token, never the
//! person's, and everything public is scrubbed first (`pinned_core::reports`).

use super::github::{Error as GithubError, Github, IssueInfo};
use crate::db::Db;
use crate::hub::{Account, Hub};
use pinned_core::reports::{self, Ask, Context, Draft, Environment, Kind, Preview, Report, ReportState, Similar};
use pinned_core::wire::{Call, ProjectView};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Why every call but `status` refuses on a hub with no token.
pub const NOT_AVAILABLE: &str = "Reporting is not available on this hub";
/// Reports one account sends in any rolling 24 hours.
pub const DAILY_LIMIT: usize = 5;
const WINDOW_SECS: i64 = 24 * 60 * 60;
const DEFAULT_API: &str = "https://api.github.com";
/// The variable the token is read from. Logs name it, never its value.
pub const TOKEN_VARIABLE: &str = "PINNED_REPORTS_GITHUB_TOKEN";
const REPO_VARIABLE: &str = "PINNED_REPORTS_REPO";
const API_VARIABLE: &str = "PINNED_REPORTS_GITHUB_API";
const POLL_VARIABLE: &str = "PINNED_REPORTS_POLL_SECS";
const POLL_DEFAULT: Duration = Duration::from_secs(10 * 60);
/// How long a search answer is reused.
const SIMILAR_FRESH: Duration = Duration::from_secs(60);
/// Most searches the hub makes in any minute, for all accounts together.
const SEARCHES_PER_MINUTE: usize = 20;
/// Most searches the hub makes in any minute for one account.
const ACCOUNT_SEARCHES_PER_MINUTE: usize = 5;
const SEARCH_BUSY: &str = "Searching the issues is busy. Try again in a minute.";
/// Most search answers kept.
const SIMILAR_KEPT: usize = 200;
/// How long a preview's words stay known, for the send that follows it.
const PREVIEW_FRESH: Duration = Duration::from_secs(60 * 60);
/// Most previews kept for one account, so one account cannot push out another's.
const PREVIEWS_KEPT: usize = 20;
/// Most words in each list of the context an app sends, and the longest word, in characters.
const CONTEXT_WORDS: usize = 500;
const CONTEXT_WORD_CHARS: usize = 200;
/// How long the hub waits for an online machine to list its projects before a report goes on without them.
const PROJECTS_WAIT: Duration = Duration::from_secs(2);
/// The footer reference of a preview: the report's own id exists only once it is sent, so the hash the person
/// reviewed (`shown`) is of the issue with this one.
const PREVIEW_REF: &str = "r-xxxxxxxx";
/// Most characters of a draft the hub keeps or sends.
const DRAFT_CHARS: usize = 64_000;
/// Most drafts one account keeps.
const DRAFTS_KEPT: i64 = 50;
/// Most of those drafts its agents saved.
const AGENT_DRAFTS_KEPT: i64 = 10;
/// The error a send left when the hub stopped before GitHub answered.
pub const RESTARTED: &str = "the hub restarted while sending; check My reports before trying again";
/// Most reports `list` answers.
const LIST_LIMIT: i64 = 200;

pub const SCHEMA: &str = "
create table if not exists reports (id uuid primary key, account uuid not null references accounts on delete cascade,
  kind text not null, title text not null, draft jsonb, state jsonb not null, issue bigint, url text,
  followed boolean not null default false, by_agent boolean not null default false,
  own_comments bigint[] not null default '{}', seen_comments int not null default 0,
  sent_at bigint, created_at bigint not null, updated_at bigint not null, request_id text);
create index if not exists reports_by_account on reports (account, updated_at desc);
create index if not exists reports_by_issue on reports (issue) where issue is not null;
create unique index if not exists reports_request on reports (account, request_id) where request_id is not null;
alter table reports add column if not exists counts boolean not null default true;
alter table reports add column if not exists hidden boolean not null default false;
";

const COLUMNS: &str = "id, kind, title, draft, state, issue, url, followed, by_agent, created_at, updated_at";
/// `COLUMNS`, then whether the row counts toward the daily limit and when it was sent.
const SEND_COLUMNS: &str = "id, kind, title, draft, state, issue, url, followed, by_agent, created_at, updated_at, counts, sent_at";

/// Where the hub files reports: the GitHub API, the repository (`owner/name`) and the hub's own token.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub api: String,
    pub repo: String,
    pub token: String,
    /// How long one call to GitHub may take.
    pub timeout: Duration,
    /// How often the poller looks at the issues the reports became. Not a wire value.
    pub poll: Duration,
}

// Not derived, so the token never reaches a log.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").field("api", &self.api).field("repo", &self.repo).finish_non_exhaustive()
    }
}

impl Config {
    /// The configuration `env` asks for, or None when reporting is not available: no token, or an override that must
    /// not be honoured as given. The token is `PINNED_REPORTS_GITHUB_TOKEN`; the repository defaults to pinned's own
    /// and the API to GitHub's. `PINNED_REPORTS_REPO` and `PINNED_REPORTS_GITHUB_API` count only when `dev`, and the
    /// API only on a loopback host, so the token never goes anywhere else; outside `dev` they are ignored with a warning.
    pub fn from_env(dev: bool, env: &dyn Fn(&str) -> Option<String>) -> Option<Config> {
        let given = |name: &str| env(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let token = given(TOKEN_VARIABLE)?;
        let (mut repo, mut api) = (default_repo(), DEFAULT_API.to_owned());
        for (name, value) in [(REPO_VARIABLE, given(REPO_VARIABLE)), (API_VARIABLE, given(API_VARIABLE))] {
            let Some(value) = value else { continue };
            if !dev {
                eprintln!("pinned-server: {name} is ignored: it counts only under --dev");
            } else if name == REPO_VARIABLE && is_repo(&value) {
                repo = value;
            } else if name == API_VARIABLE && is_loopback(&value) {
                api = value;
            } else {
                eprintln!("pinned-server: {name} is refused, so reporting is not available: {}", if name == REPO_VARIABLE { "it is not owner/name" } else { "the API must be on a loopback host" });
                return None;
            }
        }
        Some(Config { api, repo, token, timeout: pinned_core::github::CALL_TIMEOUT, poll: poll_every(dev, env) })
    }
}

/// How often the poller runs: ten minutes, or `PINNED_REPORTS_POLL_SECS` under `dev`.
fn poll_every(dev: bool, env: &dyn Fn(&str) -> Option<String>) -> Duration {
    let given = env(POLL_VARIABLE).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    match given {
        Some(secs) if dev => match secs.parse::<u64>() {
            Ok(secs) if secs > 0 => Duration::from_secs(secs),
            _ => {
                eprintln!("pinned-server: {POLL_VARIABLE} is ignored: it is not a number of seconds");
                POLL_DEFAULT
            }
        },
        Some(_) => {
            eprintln!("pinned-server: {POLL_VARIABLE} is ignored: it counts only under --dev");
            POLL_DEFAULT
        }
        None => POLL_DEFAULT,
    }
}

/// pinned's own repository, `owner/name`, as the updater already names it.
fn default_repo() -> String {
    crate::self_update::RELEASE_BASE.trim_start_matches("https://github.com/").splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
}

/// `owner/name`.
fn is_repo(text: &str) -> bool {
    let mut parts = text.split('/');
    let part = |p: Option<&str>| p.is_some_and(|p| !matches!(p, "" | "." | "..") && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)));
    part(parts.next()) && part(parts.next()) && parts.next().is_none()
}

/// An http(s) URL whose host is this machine.
fn is_loopback(text: &str) -> bool {
    let Ok(url) = url::Url::parse(text) else { return false };
    matches!(url.scheme(), "http" | "https")
        && match url.host() {
            Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        }
}

/// What the hub keeps in memory for reports.
pub struct Reports {
    github: Option<Github>,
    /// One send at a time per account, so the daily limit counts every send before the next one starts.
    sending: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    similar: Mutex<HashMap<String, (Instant, Vec<Similar>)>>,
    searches: Mutex<VecDeque<Instant>>,
    /// The searches made for each account in the last minute.
    account_searches: Mutex<HashMap<Uuid, VecDeque<Instant>>>,
    /// The words a preview scrubbed with, by account and the hash it showed, so the send that follows renders it alike.
    previews: Mutex<HashMap<(Uuid, String), (Instant, Context)>>,
    /// The repository reports go to, `owner/name`, lower case. It is public, so its names are never scrubbed.
    own_repo: Option<String>,
}

tokio::task_local! {
    /// The account a `reports` ask runs for, so its searches count against its own budget too.
    static SEARCHER: Uuid;
}

impl Reports {
    /// Keeps the GitHub client when there is a token, once the table is ready and the sends a stopped hub left are
    /// failed. With no token it touches nothing; a table it cannot ready leaves reporting not available.
    pub async fn start(db: &Db, config: Option<Config>) -> Reports {
        let own_repo = config.as_ref().map(|c| c.repo.to_lowercase());
        let github = match config {
            Some(c) => match ready(db).await {
                Ok(()) => Some(Github::with_api(&c.api, &c.repo, &c.token).with_timeout(c.timeout)),
                Err(e) => {
                    eprintln!("pinned-server: reporting an issue from the app is not available: {e}");
                    None
                }
            },
            None => None,
        };
        Reports { github, sending: Mutex::default(), similar: Mutex::default(), searches: Mutex::default(), account_searches: Mutex::default(), previews: Mutex::default(), own_repo }
    }

    pub fn available(&self) -> bool {
        self.github.is_some()
    }

    fn github(&self) -> Result<&Github, String> {
        self.github.as_ref().ok_or_else(|| NOT_AVAILABLE.to_owned())
    }

    async fn hold(&self, account: Uuid) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.sending.lock().unwrap();
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
            locks.entry(account).or_default().clone()
        };
        lock.lock_owned().await
    }

    fn remember_preview(&self, account: Uuid, shown: &str, ctx: Context) {
        let mut previews = self.previews.lock().unwrap();
        previews.retain(|_, (at, _)| at.elapsed() < PREVIEW_FRESH);
        let key = (account, shown.to_owned());
        let own = previews.keys().filter(|(of, _)| *of == account).count();
        if own >= PREVIEWS_KEPT && !previews.contains_key(&key) {
            let oldest = previews.iter().filter(|((of, _), _)| *of == account).min_by_key(|(_, (at, _))| *at).map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                previews.remove(&oldest);
            }
        }
        previews.insert(key, (Instant::now(), ctx));
    }

    /// `ctx` without the names of the repository reports go to: its owner, its name and the two together, in any
    /// form. They are public, and scrubbing them would spoil every report about pinned itself.
    fn without_own(&self, mut ctx: Context) -> Context {
        let Some(own) = &self.own_repo else { return ctx };
        let (owner, name) = own.split_once('/').unwrap_or(("", own));
        ctx.projects.retain(|word| {
            let word = word.trim().trim_end_matches('/').to_lowercase();
            let word = word.strip_suffix(".git").unwrap_or(&word);
            !(word == own || word == name || word == owner || word.ends_with(&format!("/{own}")) || word.ends_with(&format!(":{own}")))
        });
        ctx
    }

    fn preview_context(&self, account: Uuid, shown: &str) -> Option<Context> {
        let previews = self.previews.lock().unwrap();
        previews.get(&(account, shown.to_owned())).filter(|(at, _)| at.elapsed() < PREVIEW_FRESH).map(|(_, ctx)| ctx.clone())
    }

    /// The open issues that look like `query`: an answer under a minute old, or a search, of which the hub makes
    /// at most `SEARCHES_PER_MINUTE`.
    async fn similar(&self, query: &str) -> Result<Vec<Similar>, String> {
        if query.is_empty() {
            return Ok(vec![]);
        }
        if let Some((at, found)) = self.similar.lock().unwrap().get(query) {
            if at.elapsed() < SIMILAR_FRESH {
                return Ok(found.clone());
            }
        }
        let github = self.github()?;
        {
            let fresh = |searches: &mut VecDeque<Instant>| {
                while searches.front().is_some_and(|at| at.elapsed() >= Duration::from_secs(60)) {
                    searches.pop_front();
                }
            };
            let mut searches = self.searches.lock().unwrap();
            let mut accounts = self.account_searches.lock().unwrap();
            fresh(&mut searches);
            accounts.retain(|_, own| {
                fresh(own);
                !own.is_empty()
            });
            let own = SEARCHER.try_with(|account| *account).ok().map(|account| accounts.entry(account).or_default());
            if searches.len() >= SEARCHES_PER_MINUTE || own.as_ref().is_some_and(|own| own.len() >= ACCOUNT_SEARCHES_PER_MINUTE) {
                return Err(SEARCH_BUSY.into());
            }
            searches.push_back(Instant::now());
            own.into_iter().for_each(|own| own.push_back(Instant::now()));
        }
        let found: Vec<Similar> = github
            .search_open(query)
            .await
            .map_err(|e| {
                eprintln!("pinned-server: could not search the issues: {e}");
                format!("Could not search the issues: {}", e.public())
            })?
            .into_iter()
            .map(|hit| Similar { number: hit.number, title: hit.title, url: hit.html_url, comments: u32::try_from(hit.comments).unwrap_or(u32::MAX) })
            .collect();
        let mut cache = self.similar.lock().unwrap();
        cache.retain(|_, (at, _)| at.elapsed() < SIMILAR_FRESH);
        if cache.len() >= SIMILAR_KEPT {
            cache.clear();
        }
        cache.insert(query.to_owned(), (Instant::now(), found.clone()));
        Ok(found)
    }
}

/// Creates the table and fails every send a hub that stopped left in `sending`: it may have reached GitHub, so it
/// still counts, and the person checks before trying again.
async fn ready(db: &Db) -> Result<(), String> {
    db.run_schema(SCHEMA).await?;
    db.run_schema(super::poller::SCHEMA).await?;
    let failed = json(ReportState::Failed { error: RESTARTED.into() })?;
    let swept = db.client().await?.execute("update reports set state = $1, updated_at = $2 where state->>'state' = 'sending'", &[&failed, &now_secs()]).await.map_err(err)?;
    if swept > 0 {
        eprintln!("pinned-server: {swept} report(s) were sending when a hub stopped; they read as failed");
    }
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn err(e: tokio_postgres::Error) -> String {
    e.to_string()
}

fn json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::Problem => "problem",
        Kind::Idea => "idea",
    }
}

fn state_name(state: &ReportState) -> String {
    json(state).ok().and_then(|v| v["state"].as_str().map(str::to_owned)).unwrap_or_default()
}

/// The hub's line of a report's environment section, after the reporter's own.
fn environment() -> String {
    format!("hub {}", env!("CARGO_PKG_VERSION"))
}

/// The environment of an agent's report: the app version and system of the machine its session runs on, as the hub
/// knows them. Never the machine's name.
fn machine_environment(machine: Option<&pinned_core::wire::MachineView>) -> String {
    let Some(machine) = machine else { return String::new() };
    let version = (!machine.version.is_empty()).then(|| format!("pinned {}", machine.version));
    version.into_iter().chain((!machine.os.is_empty()).then(|| machine.os.clone())).collect::<Vec<_>>().join(" · ")
}

/// What an agent reads of a report: where it is, never its text or anything GitHub said.
fn agent_report(report: &Report) -> Value {
    json!({ "number": report.issue, "url": report.url, "state": state_name(&report.state) })
}

fn stored(row: &tokio_postgres::Row) -> Result<Report, String> {
    let kind: String = row.get(1);
    let kind: Kind = serde_json::from_value(Value::String(kind)).map_err(|e| e.to_string())?;
    let draft: Option<Value> = row.get(3);
    let state: Value = row.get(4);
    let issue: Option<i64> = row.get(5);
    Ok(Report {
        id: row.get::<_, Uuid>(0).to_string(),
        kind,
        title: row.get(2),
        issue: issue.and_then(|n| u64::try_from(n).ok()),
        url: row.get(6),
        followed: row.get(7),
        by_agent: row.get(8),
        state: serde_json::from_value(state).map_err(|e| format!("a stored report's state does not read: {e}"))?,
        draft: draft.map(serde_json::from_value).transpose().map_err(|e| format!("a stored draft does not read: {e}"))?,
        created_at: row.get(9),
        updated_at: row.get(10),
    })
}

/// `HH:MM UTC` of a Unix time.
fn clock(at: i64) -> String {
    let day = at.rem_euclid(86_400);
    format!("{:02}:{:02} UTC", day / 3600, day % 3600 / 60)
}

/// A stored report with what its next send needs: whether it counts toward the daily limit and when it was sent.
struct SendRow {
    key: Uuid,
    report: Report,
    counts: bool,
    sent_at: Option<i64>,
}

/// A row of `SEND_COLUMNS`.
fn send_row(row: &tokio_postgres::Row) -> Result<SendRow, String> {
    Ok(SendRow { key: row.get(0), report: stored(row)?, counts: row.get(11), sent_at: row.get(12) })
}

fn could_not_send(error: &str) -> String {
    format!("Could not send the report: {error}")
}

/// The refusal of a send over the daily limit, given when the counted sends in the window were made, oldest first.
fn over_limit(sent: &[i64]) -> Option<String> {
    (sent.len() >= DAILY_LIMIT).then(|| {
        let again = sent.first().map_or(0, |first| first + WINDOW_SECS);
        format!("You sent {DAILY_LIMIT} reports in the last day. You can send again at {}.", clock(again))
    })
}

const COUNTED_SENDS: &str = "select sent_at from reports where account = $1 and sent_at > $2 and counts order by sent_at";

fn too_long(draft: &Draft) -> Result<(), String> {
    let size = serde_json::to_string(draft).map_err(|e| e.to_string())?.chars().count();
    if size > DRAFT_CHARS {
        return Err(format!("The report is too long: keep it under {DRAFT_CHARS} characters."));
    }
    Ok(())
}

/// Refuses a context from the app with more than `CONTEXT_WORDS` words in a list, or a word over `CONTEXT_WORD_CHARS`.
fn too_large(ctx: &Context) -> Result<(), String> {
    let lists = [&ctx.projects, &ctx.folders, &ctx.files, &ctx.names];
    if lists.iter().any(|list| list.len() > CONTEXT_WORDS || list.iter().any(|word| word.chars().count() > CONTEXT_WORD_CHARS)) {
        return Err("The report's context is too large.".into());
    }
    Ok(())
}

/// The words of a session's folder a public report must not hold: the folder, its name, and for a worktree
/// (`<project>/.claude/worktrees/<name>` or `<project>/.worktrees/<name>`) the project's folder and name too. A
/// Windows folder splits on `\` as well.
pub fn cwd_context(cwd: &str) -> Context {
    let mut ctx = Context::default();
    let cwd = cwd.trim().trim_end_matches(['/', '\\']);
    let parts: Vec<&str> = cwd.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return ctx;
    }
    ctx.folders.push(cwd.to_owned());
    let n = parts.len();
    let project = match parts.as_slice() {
        [.., project, ".worktrees", _] | [.., project, ".claude", "worktrees", _] => Some(*project),
        _ => None,
    };
    for name in parts.last().into_iter().chain(project.as_ref()) {
        // A drive such as `C:` names nothing private.
        if !name.ends_with(':') {
            ctx.projects.push((*name).to_owned());
        }
    }
    if let Some(project) = project {
        let depth = if parts[n - 2] == ".worktrees" { 2 } else { 3 };
        let mut end = cwd.len();
        for _ in 0..depth {
            end = cwd[..end].rfind(['/', '\\']).unwrap_or(0);
        }
        if end > 0 && cwd[..end].ends_with(project) {
            ctx.folders.push(cwd[..end].to_owned());
        }
    }
    ctx
}

/// `ask` with the words of the asking session's folder, so a search an agent asks for leaves them out too.
pub fn with_cwd(ask: Ask, cwd: &str) -> Ask {
    match ask {
        Ask::Similar { title, context } => Ask::Similar { title, context: merged(context, &cwd_context(cwd)) },
        ask => ask,
    }
}

/// `ctx` with the words of `more` too, each once.
fn merged(mut ctx: Context, more: &Context) -> Context {
    fn add(into: &mut Vec<String>, from: &[String]) {
        for word in from {
            if !into.contains(word) {
                into.push(word.clone());
            }
        }
    }
    add(&mut ctx.projects, &more.projects);
    add(&mut ctx.folders, &more.folders);
    add(&mut ctx.files, &more.files);
    add(&mut ctx.names, &more.names);
    ctx
}

fn scrubbed(draft: &Draft, ctx: &Context) -> Result<Draft, String> {
    Ok(Draft {
        title: reports::scrub(&draft.title, ctx)?,
        happened: reports::scrub(&draft.happened, ctx)?,
        expected: reports::scrub(&draft.expected, ctx)?,
        repeat: reports::scrub(&draft.repeat, ctx)?,
        ..draft.clone()
    })
}

/// The scrubbed copy of an agent's `report_issue` arguments, which a confirmation then holds: the user reads, and
/// approving sends, exactly this public text. `cwd` is the folder of the session that asks, on `machine`; it, and the
/// folder's name, never reach GitHub. The environment, when the agent asks for one, is `machine`'s app version and
/// system, whatever the agent wrote there. Refuses when reporting is not available on this hub.
pub async fn prepare(hub: &Hub, account: &Account, machine: &str, cwd: &str, args: &Value) -> Result<Value, String> {
    hub.reports.github()?;
    let ctx = hub.reports.without_own(merged(hub.report_context(account).await?, &cwd_context(cwd)));
    let mut fields = args.as_object().cloned().ok_or("report_issue: args must be an object of the action's fields")?;
    let draft: Draft = serde_json::from_value(fields.get("draft").cloned().unwrap_or(Value::Null)).map_err(|e| format!("report_issue: {e}"))?;
    let mut draft = scrubbed(&draft, &ctx)?;
    draft.attach.errors.clear();
    draft.attach.environment = match draft.attach.environment.chosen() {
        true => {
            let machines = hub.machines(account).await?;
            Environment::Shown(reports::scrub(&machine_environment(machines.iter().find(|m| m.id == machine)), &ctx)?)
        }
        false => Environment::Chosen(false),
    };
    fields.insert("draft".into(), json(&draft)?);
    Ok(Value::Object(fields))
}

impl Hub {
    /// The words a public report must not hold, as the hub knows them: the account's e-mail address and its local
    /// part, its machines' names, the folder of every open tab on every machine, and the projects each online machine
    /// lists now (asked here, briefly) or listed last: their names, folders, repositories, and each repository's owner
    /// and name. The hub's own repository is left out.
    pub(crate) async fn report_context(&self, account: &Account) -> Result<Context, String> {
        let mut ctx = Context::default();
        ctx.names.push(account.email.clone());
        if let Some((local, _)) = account.email.split_once('@') {
            ctx.names.push(local.to_owned());
        }
        let machines = self.machines(account).await?;
        ctx.names.extend(machines.iter().map(|m| m.name.clone()));
        for (machine, _) in account.online() {
            for tab in account.tabs_of(&machine).unwrap_or_default() {
                ctx = merged(ctx, &cwd_context(&tab.cwd));
            }
        }
        // A machine that is offline, or slow to answer, adds what it listed last, or nothing; it never fails the call.
        let asked = futures_util::future::join_all(machines.iter().filter(|m| m.online).map(|m| async move {
            let listed = self.call_machine(account, &m.id, Call::ListProjects, PROJECTS_WAIT).await.ok();
            (m.id.clone(), listed.and_then(|v| serde_json::from_value::<Vec<ProjectView>>(v).ok()))
        }))
        .await;
        for (machine, listed) in asked {
            if let Some(projects) = listed {
                account.listed(&machine, projects);
            }
        }
        for project in account.listed_projects() {
            ctx = merged(ctx, &cwd_context(&project.path));
            ctx.projects.push(project.name);
            if let Some(repo) = project.repo {
                let parts: Vec<&str> = repo.trim_end_matches('/').split('/').collect();
                if let [.., owner, name] = parts.as_slice() {
                    ctx.projects.extend([owner.to_string(), name.trim_end_matches(".git").to_owned(), format!("{owner}/{}", name.trim_end_matches(".git"))]);
                }
                ctx.projects.push(repo);
            }
        }
        ctx.projects.retain(|word| !word.trim().is_empty());
        Ok(self.reports.without_own(merged(Context::default(), &ctx)))
    }

    /// What the app (or, with `agent`, a session) asks about reporting.
    pub(crate) async fn reports_ask(&self, account: &Account, ask: Ask, agent: bool) -> Result<Value, String> {
        if let Ask::Status = ask {
            if !self.reports.available() {
                return Ok(json!({ "available": false }));
            }
            return Ok(json!({ "available": true, "left_today": self.left_today(account).await? }));
        }
        self.reports.github()?;
        // A search made for this ask counts against the account's own budget too.
        SEARCHER.scope(account.id, async move { match ask {
            Ask::Status => unreachable!("answered above"),
            Ask::Preview { draft, context } => json(self.preview(account, draft, context).await?),
            Ask::Similar { title, context } => {
                too_large(&context)?;
                let ctx = self.reports.without_own(merged(self.report_context(account).await?, &context));
                let query = reports::search_query(&reports::scrub(&title, &ctx)?);
                let found = self.reports.similar(&query).await?;
                if agent {
                    // A title that does not scrub clean reads as none.
                    let public_title = |title: &str| reports::public_title(title, &ctx).unwrap_or_default();
                    return Ok(Value::Array(found.iter().map(|s| json!({ "number": s.number, "url": s.url, "state": "open", "public_title": public_title(&s.title) })).collect()));
                }
                json(found)
            }
            Ask::List => {
                let rows = self.db.client().await?.query(&format!("select {COLUMNS} from reports where account = $1 and not hidden order by updated_at desc limit $2"), &[&account.id, &LIST_LIMIT]).await.map_err(err)?;
                let list: Vec<Report> = rows.iter().map(|r| stored(r)).collect::<Result<_, _>>()?;
                if agent {
                    return Ok(Value::Array(list.iter().map(agent_report).collect()));
                }
                json(list)
            }
            Ask::SaveDraft { id, draft } => json(self.save_draft(account, id.as_deref(), draft, agent).await?),
            Ask::DeleteDraft { id } => {
                let key = Uuid::parse_str(&id).map_err(|_| no_draft(&id))?;
                let client = self.db.client().await?;
                // A failed send that still counts toward the daily limit leaves the list but stays, so deleting it
                // frees no send.
                let hidden = client
                    .execute(
                        "update reports set hidden = true, draft = null, updated_at = $4 where id = $1 and account = $2 and state->>'state' = 'failed' and counts and sent_at > $3 and not hidden",
                        &[&key, &account.id, &(now_secs() - WINDOW_SECS), &now_secs()],
                    )
                    .await
                    .map_err(err)?;
                let deleted = match hidden {
                    0 => client.execute("delete from reports where id = $1 and account = $2 and state->>'state' in ('draft', 'failed') and not hidden", &[&key, &account.id]).await.map_err(err)?,
                    hidden => hidden,
                };
                if deleted == 0 {
                    return Err(no_draft(&id));
                }
                Ok(Value::Null)
            }
        } }).await
    }

    /// How many reports the account may still send today.
    async fn left_today(&self, account: &Account) -> Result<u32, String> {
        let sent = self.sends_in_window(account).await?.len();
        Ok(DAILY_LIMIT.saturating_sub(sent) as u32)
    }

    /// When the account's sends in the last 24 hours were made, oldest first. A failed one counts when it may have
    /// reached GitHub.
    async fn sends_in_window(&self, account: &Account) -> Result<Vec<i64>, String> {
        let since = now_secs() - WINDOW_SECS;
        let rows = self.db.client().await?.query(COUNTED_SENDS, &[&account.id, &since]).await.map_err(err)?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    async fn preview(&self, account: &Account, draft: Draft, app: Context) -> Result<Preview, String> {
        too_long(&draft)?;
        too_large(&app)?;
        let ctx = self.reports.without_own(merged(self.report_context(account).await?, &app));
        let (title, body, labels) = reports::render(&draft, &ctx, Some(&environment()), PREVIEW_REF)?;
        let shown = reports::shown(&title, &body);
        self.reports.remember_preview(account.id, &shown, ctx.clone());
        let query = reports::search_query(&title);
        // A search that fails or is held back leaves the preview without suggestions, not without a preview.
        let mut similar = self.reports.similar(&query).await.unwrap_or_default();
        similar.truncate(5);
        Ok(Preview { title, body, labels: labels.into_iter().collect(), similar, left_today: self.left_today(account).await?, shown })
    }

    /// Saves `draft`, new or over draft `id`. A new one an agent saves is marked as its, and agents keep at most
    /// `AGENT_DRAFTS_KEPT` of the account's drafts.
    async fn save_draft(&self, account: &Account, id: Option<&str>, draft: Draft, by_agent: bool) -> Result<Report, String> {
        too_long(&draft)?;
        let client = self.db.client().await?;
        let (now, saved) = (now_secs(), json(&draft)?);
        let state = json(ReportState::Draft)?;
        let row = match id {
            Some(id) => {
                let key = Uuid::parse_str(id).map_err(|_| no_draft(id))?;
                let row = client
                    .query_opt(
                        &format!("update reports set kind = $3, title = $4, draft = $5, updated_at = $6 where id = $1 and account = $2 and state->>'state' = 'draft' returning {COLUMNS}"),
                        &[&key, &account.id, &kind_name(&draft.kind), &draft.title, &saved, &now],
                    )
                    .await
                    .map_err(err)?;
                row.ok_or_else(|| no_draft(id))?
            }
            None => {
                let kept = client.query_one("select count(*), count(*) filter (where by_agent) from reports where account = $1 and state->>'state' = 'draft'", &[&account.id]).await.map_err(err)?;
                if by_agent && kept.get::<_, i64>(1) >= AGENT_DRAFTS_KEPT {
                    return Err(format!("There are {AGENT_DRAFTS_KEPT} drafts from agents. Delete one first."));
                }
                if kept.get::<_, i64>(0) >= DRAFTS_KEPT {
                    return Err(format!("You have {DRAFTS_KEPT} drafts. Delete one to save another."));
                }
                client
                    .query_one(
                        &format!(
                            "insert into reports (id, account, kind, title, draft, state, by_agent, created_at, updated_at) values ($1, $2, $3, $4, $5, $6, $7, $8, $8) returning {COLUMNS}"
                        ),
                        &[&Uuid::new_v4(), &account.id, &kind_name(&draft.kind), &draft.title, &saved, &state, &by_agent, &now],
                    )
                    .await
                    .map_err(err)?
            }
        };
        Ok(stored(&row)?)
    }

    /// Saves the draft an agent's `report_issue` confirmation held, once the person denied it, as a draft of the
    /// account that its agents saved.
    pub(crate) async fn keep_denied_report(&self, account: &Account, args: &Value) -> Result<(), String> {
        self.reports.github()?;
        let draft: Draft = serde_json::from_value(args["draft"].clone()).map_err(|e| format!("the held report does not read: {e}"))?;
        self.save_draft(account, None, draft, true).await.map(drop)
    }

    /// The account's report sent with `request_id`, if one was.
    async fn request_row(&self, account: &Account, request_id: &str) -> Result<Option<SendRow>, String> {
        let row = self.db.client().await?.query_opt(&format!("select {SEND_COLUMNS} from reports where account = $1 and request_id = $2"), &[&account.id, &request_id]).await.map_err(err)?;
        row.as_ref().map(send_row).transpose()
    }

    /// Runs `sql` on report `key` of `account` once more when the database fails the first time.
    async fn record(&self, sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> Result<Option<tokio_postgres::Row>, String> {
        match self.db.client().await?.query_opt(sql, params).await {
            Ok(row) => Ok(row),
            Err(first) => {
                eprintln!("pinned-server: could not record a report's outcome, trying once more: {first}");
                self.db.client().await?.query_opt(sql, params).await.map_err(err)
            }
        }
    }

    /// Files `draft` as an issue, or with `to_issue` as a comment on that open issue, once the person (or the agent
    /// they approved) sent it. All of it runs under the account's send lock, so the daily limit sees every send.
    ///
    /// A send that failed is tried again in its own row, under the same footer reference, when it is sent again with
    /// its `request_id` or as `draft_id`; that row counts toward the limit once, whatever happens. Before it posts
    /// again, it looks on GitHub for what the failed try may have filed.
    pub(crate) async fn report_issue(&self, account: &Account, draft: Draft, shown: Option<String>, draft_id: Option<String>, request_id: Option<String>, by_agent: bool) -> Result<Value, String> {
        let github = self.reports.github()?;
        let answer = |report: &Report| if by_agent { Ok(json!({ "number": report.issue, "url": report.url })) } else { json(report) };
        if draft.title.trim().is_empty() {
            return Err("Give the report a title.".into());
        }
        if !by_agent && shown.is_none() {
            return Err("Review the report before sending it.".into());
        }
        too_long(&draft)?;
        let request_id = request_id.map(|id| id.trim().to_owned()).filter(|id| !id.is_empty());
        if request_id.as_ref().is_some_and(|id| id.chars().count() > 200) {
            return Err("The request id is too long.".into());
        }
        // A send of this request already under way: once it ends, its outcome is the answer.
        let in_flight = match &request_id {
            Some(id) => self.request_row(account, id).await?.is_some_and(|row| row.report.state == ReportState::Sending),
            None => false,
        };
        let _sending = self.reports.hold(account.id).await;
        // The failed row this send tries again, if it is one.
        let mut again = None;
        if let Some(id) = &request_id {
            if let Some(row) = self.request_row(account, id).await? {
                match &row.report.state {
                    ReportState::Failed { error } if in_flight => return Err(could_not_send(error)),
                    ReportState::Failed { .. } => again = Some(row),
                    _ => return answer(&row.report),
                }
            }
        }
        let client = self.db.client().await?;
        let saved_as = match &draft_id {
            Some(id) => {
                let key = Uuid::parse_str(id).map_err(|_| no_draft(id))?;
                let row = client
                    .query_opt(&format!("select {SEND_COLUMNS} from reports where id = $1 and account = $2 and state->>'state' in ('draft', 'failed') and not hidden"), &[&key, &account.id])
                    .await
                    .map_err(err)?;
                let row = send_row(&row.ok_or_else(|| no_draft(id))?)?;
                if again.as_ref().is_some_and(|again: &SendRow| again.key != key) {
                    return Err("This request id was sent with another report.".into());
                }
                if matches!(row.report.state, ReportState::Failed { .. }) {
                    again = Some(row);
                }
                Some(key)
            }
            None => again.as_ref().map(|again| again.key),
        };
        // A row that already counts in the window is not counted, nor held to the limit, again.
        let already = again.as_ref().is_some_and(|row| row.counts && row.sent_at.is_some_and(|at| at > now_secs() - WINDOW_SECS));
        if !already {
            if let Some(refusal) = over_limit(&self.sends_in_window(account).await?) {
                return Err(refusal);
            }
        }
        let mut ctx = self.report_context(account).await?;
        if let Some(shown) = &shown {
            // What the preview the person reviewed scrubbed with, so the hash it showed can still match.
            if let Some(known) = self.reports.preview_context(account.id, shown) {
                ctx = merged(ctx, &known);
            }
            let (title, body, _) = reports::render(&draft, &ctx, Some(&environment()), PREVIEW_REF)?;
            if &reports::shown(&title, &body) != shown {
                return Err("The report changed since you reviewed it. Review it again.".into());
            }
        }
        let id = saved_as.unwrap_or_else(Uuid::new_v4);
        let report_ref = reports::ref_of(&id.to_string());
        let (title, body, labels) = reports::render(&draft, &ctx, Some(&environment()), &report_ref)?;
        // An issue that cannot take a comment is refused before the report counts.
        let target = match draft.to_issue {
            Some(number) => {
                let info = github.issue(number).await.map_err(|e| {
                    eprintln!("pinned-server: could not read issue #{number} for account {}'s report: {e}", account.id);
                    match e {
                        GithubError::Refused { status: 404, .. } => format!("There is no issue #{number}."),
                        e => format!("Could not read issue #{number}: {}", e.public()),
                    }
                })?;
                if info.pull_request.is_some() {
                    return Err(format!("#{number} is a pull request, not an issue."));
                }
                if info.state != "open" {
                    return Err(format!("Issue #{number} is closed. Send this as a new report instead."));
                }
                if info.locked {
                    return Err(format!("Issue #{number} is locked. Send this as a new report instead."));
                }
                Some(info)
            }
            None => None,
        };
        if let Some(earlier) = self.mark_sending(account, id, &draft, &title, saved_as.is_some(), already, again.as_ref(), request_id.as_deref(), by_agent).await? {
            return answer(&earlier);
        }
        // What a failed try may have filed already: the issue, or the comment, that carries this report's reference.
        let found = match &again {
            Some(row) => match self.filed_before(github, row, target.as_ref(), &report_ref).await {
                Ok(found) => found,
                // A look that fails filed nothing.
                Err(e) => return self.failed(account, id, &e, already).await,
            },
            None => None,
        };
        let posted: Result<(u64, String, Option<u64>), GithubError> = match (found, &target) {
            (Some(found), _) => Ok(found),
            (None, Some(info)) => {
                let text = format!("### Another case from the pinned app\n\n**{title}**\n\n{body}");
                github.add_comment(info.number, &text).await.map(|comment| (info.number, info.html_url.clone(), Some(comment)))
            }
            (None, None) => github.create_issue(&title, &body, &labels).await.map(|issue| (issue.number, issue.html_url, None)),
        };
        let (number, url, comment) = match posted {
            Ok(posted) => posted,
            Err(e) => {
                let counts = already || e.may_have_reached();
                return self.failed(account, id, &e, counts).await;
            }
        };
        let now = now_secs();
        let open = json(ReportState::Open)?;
        let (stored_number, own) = (i64::try_from(number).map_err(|e| e.to_string())?, comment.map(|c| c as i64).into_iter().collect::<Vec<i64>>());
        let followed = draft.to_issue.is_some() || again.as_ref().is_some_and(|row| row.report.draft.as_ref().is_some_and(|d| d.to_issue.is_some()));
        let recorded = self
            .record(
                &format!("update reports set draft = null, state = $3, issue = $4, url = $5, followed = $6, own_comments = $7, counts = true, updated_at = $8 where id = $1 and account = $2 returning {COLUMNS}"),
                &[&id, &account.id, &open, &stored_number, &url, &followed, &own, &now],
            )
            .await;
        match recorded {
            Ok(Some(row)) => answer(&stored(&row)?),
            outcome => {
                // It is on GitHub: the answer says where, and the hub's next start fails the row that still reads sending.
                let why = outcome.err().unwrap_or_else(|| "the row is gone".into());
                eprintln!("pinned-server: account {}'s report {id} went to GitHub as #{number} but could not be recorded: {why}", account.id);
                let report = Report { id: id.to_string(), kind: draft.kind.clone(), title, issue: Some(number), url: Some(url), followed, by_agent, state: ReportState::Open, draft: None, created_at: now, updated_at: now };
                answer(&report)
            }
        }
    }

    /// Marks report `id` as sending, counted toward the daily limit, in one transaction that holds the account's
    /// advisory lock, so two hubs on one database never both pass the limit. Some(report) when another send of
    /// `request_id` got there first: that report is the answer.
    #[allow(clippy::too_many_arguments)]
    async fn mark_sending(&self, account: &Account, id: Uuid, draft: &Draft, title: &str, saved: bool, already: bool, again: Option<&SendRow>, request_id: Option<&str>, by_agent: bool) -> Result<Option<Report>, String> {
        let (now, kept, sending) = (now_secs(), json(draft)?, json(ReportState::Sending)?);
        let mut held = self.db.transaction_client().await?;
        let tx = held.transaction().await.map_err(err)?;
        tx.execute("select pg_advisory_xact_lock(hashtextextended('pinned-reports:' || $1::uuid::text, 0))", &[&account.id]).await.map_err(err)?;
        if let Some(request_id) = request_id {
            let row = tx.query_opt(&format!("select {SEND_COLUMNS} from reports where account = $1 and request_id = $2"), &[&account.id, &request_id]).await.map_err(err)?;
            if let Some(row) = row.as_ref().map(send_row).transpose()? {
                let ours = again.is_some_and(|again| again.key == row.key) && matches!(row.report.state, ReportState::Failed { .. });
                if !ours {
                    return Ok(Some(row.report));
                }
            }
        }
        if !already {
            let sent: Vec<i64> = tx.query(COUNTED_SENDS, &[&account.id, &(now - WINDOW_SECS)]).await.map_err(err)?.iter().map(|r| r.get(0)).collect();
            if let Some(refusal) = over_limit(&sent) {
                return Err(refusal);
            }
        }
        if saved {
            let updated = tx
                .execute(
                    "update reports set kind = $3, title = $4, draft = $5, state = $6, by_agent = by_agent or $7, counts = true, sent_at = case when $10 then sent_at else $8 end, \
                     updated_at = $8, request_id = coalesce($9, request_id), hidden = false where id = $1 and account = $2 and state->>'state' in ('draft', 'failed')",
                    &[&id, &account.id, &kind_name(&draft.kind), &title, &kept, &sending, &by_agent, &now, &request_id, &already],
                )
                .await
                .map_err(err)?;
            if updated == 0 {
                return Err(no_draft(&id.to_string()));
            }
        } else {
            tx.execute(
                "insert into reports (id, account, kind, title, draft, state, by_agent, sent_at, created_at, updated_at, request_id) values ($1, $2, $3, $4, $5, $6, $7, $8, $8, $8, $9)",
                &[&id, &account.id, &kind_name(&draft.kind), &title, &kept, &sending, &by_agent, &now, &request_id],
            )
            .await
            .map_err(err)?;
        }
        tx.commit().await.map_err(err)?;
        Ok(None)
    }

    /// Where the failed try of `row` may have landed: the issue whose body carries `report_ref`, or, when it was a
    /// case for an issue, the comment there that does.
    async fn filed_before(&self, github: &Github, row: &SendRow, target: Option<&IssueInfo>, report_ref: &str) -> Result<Option<(u64, String, Option<u64>)>, GithubError> {
        let tried = row.report.draft.as_ref().and_then(|draft| draft.to_issue).or(target.map(|info| info.number));
        match tried {
            Some(number) => {
                let Some(comment) = github.comment_with_ref(number, report_ref).await? else { return Ok(None) };
                let url = match target.filter(|info| info.number == number) {
                    Some(info) => info.html_url.clone(),
                    None => github.issue(number).await?.html_url,
                };
                Ok(Some((number, url, Some(comment))))
            }
            None => Ok(github.issue_with_ref(report_ref).await?.map(|issue| (issue.number, issue.html_url, None))),
        }
    }

    /// Fails report `id` with what a person may read of `error`; it keeps counting toward the daily limit when
    /// `counts`.
    async fn failed(&self, account: &Account, id: Uuid, error: &GithubError, counts: bool) -> Result<Value, String> {
        eprintln!("pinned-server: could not file account {}'s report {id}: {error}", account.id);
        let failed = json(ReportState::Failed { error: error.public().into() })?;
        if let Err(e) = self.record("update reports set state = $3, counts = $4, updated_at = $5 where id = $1 and account = $2 returning id", &[&id, &account.id, &failed, &counts, &now_secs()]).await {
            eprintln!("pinned-server: account {}'s report {id} failed and could not be recorded; the hub's next start fails it: {e}", account.id);
        }
        Err(could_not_send(error.public()))
    }
}

fn no_draft(id: &str) -> String {
    format!("this account has no draft {id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn without_a_token_reporting_is_not_available() {
        assert_eq!(Config::from_env(true, &env(&[])), None);
        assert_eq!(Config::from_env(false, &env(&[(TOKEN_VARIABLE, "  ")])), None);
    }

    #[test]
    fn outside_dev_the_repo_and_api_overrides_are_ignored() {
        let vars = [(TOKEN_VARIABLE, "tok"), (REPO_VARIABLE, "other/repo"), (API_VARIABLE, "http://127.0.0.1:9")];
        let config = Config::from_env(false, &env(&vars)).unwrap();
        assert_eq!((config.api.as_str(), config.repo.as_str(), config.token.as_str()), ("https://api.github.com", "FelipeGazapina/pinned", "tok"));
        assert_eq!(default_repo(), "FelipeGazapina/pinned", "the default is the updater's repository");
        assert!(!format!("{config:?}").contains("tok\""), "the token stays out of Debug");
    }

    #[test]
    fn under_dev_the_overrides_count_and_the_api_must_be_loopback() {
        let vars = [(TOKEN_VARIABLE, "fake-1"), (REPO_VARIABLE, "acme/app"), (API_VARIABLE, "http://127.0.0.1:4010")];
        assert_eq!(Config::from_env(true, &env(&vars)), Some(Config { api: "http://127.0.0.1:4010".into(), repo: "acme/app".into(), token: "fake-1".into(), timeout: pinned_core::github::CALL_TIMEOUT, poll: POLL_DEFAULT }));
        let only_repo = [(TOKEN_VARIABLE, "t"), (REPO_VARIABLE, "acme/app")];
        assert_eq!(Config::from_env(true, &env(&only_repo)).map(|c| (c.api, c.repo)), Some(("https://api.github.com".into(), "acme/app".into())));
        for api in ["https://evil.example.test", "http://127.0.0.1.example.test:9", "http://169.254.169.254", "ftp://127.0.0.1", "not a url"] {
            assert_eq!(Config::from_env(true, &env(&[(TOKEN_VARIABLE, "t"), (API_VARIABLE, api)])), None, "{api}");
        }
        for api in ["http://localhost:9", "http://[::1]:9", "https://127.0.0.1"] {
            assert!(Config::from_env(true, &env(&[(TOKEN_VARIABLE, "t"), (API_VARIABLE, api)])).is_some(), "{api}");
        }
        assert_eq!(Config::from_env(true, &env(&[(TOKEN_VARIABLE, "t"), (REPO_VARIABLE, "../x")])), None);
    }

    fn reports(own: Option<&str>) -> Reports {
        Reports { github: None, sending: Mutex::default(), similar: Mutex::default(), searches: Mutex::default(), account_searches: Mutex::default(), previews: Mutex::default(), own_repo: own.map(str::to_lowercase) }
    }

    fn scrubbed_text(text: &str, ctx: &Context) -> String {
        reports::scrub(text, ctx).unwrap()
    }

    #[test]
    fn a_worktree_folder_names_its_project_too() {
        let ctx = cwd_context("/home/x/projetos/loja-web/.claude/worktrees/fix-1");
        assert_eq!(ctx.projects, ["fix-1", "loja-web"]);
        assert_eq!(ctx.folders, ["/home/x/projetos/loja-web/.claude/worktrees/fix-1", "/home/x/projetos/loja-web"]);
        assert_eq!(scrubbed_text("loja-web crashes on save", &ctx), "[project] crashes on save");
        let ctx = cwd_context("/srv/code/loja-web/.worktrees/fix-2/");
        assert_eq!(ctx.projects, ["fix-2", "loja-web"]);
        assert_eq!(scrubbed_text("loja-web crashes on save in fix-2", &ctx), "[project] crashes on save in [project]");
    }

    #[test]
    fn a_windows_folder_splits_on_backslashes() {
        let ctx = cwd_context("C:\\Users\\x\\proj\\loja-web");
        assert_eq!(ctx.projects, ["loja-web"]);
        assert_eq!(scrubbed_text("loja-web crashes on save", &ctx), "[project] crashes on save");
        assert_eq!(cwd_context("C:\\").projects, Vec::<String>::new(), "a drive names nothing");
        assert_eq!(cwd_context("  "), Context::default());
    }

    #[test]
    fn a_search_an_agent_asks_for_gains_its_sessions_folder_words_and_no_other_ask_changes() {
        let asked = Ask::Similar { title: "loja-web export fails".into(), context: Context { names: vec!["mac-mini".into()], projects: vec!["caixa-pdv".into()], ..Context::default() } };
        let Ask::Similar { title, context } = with_cwd(asked, "/home/x/projetos/loja-web/.worktrees/fix-1") else { panic!("still a search") };
        assert_eq!(title, "loja-web export fails");
        assert_eq!(context.names, ["mac-mini"]);
        assert_eq!(context.projects, ["caixa-pdv", "fix-1", "loja-web"]);
        assert_eq!(context.folders, ["/home/x/projetos/loja-web/.worktrees/fix-1", "/home/x/projetos/loja-web"]);
        assert_eq!(scrubbed_text(&title, &context), "[project] export fails");
        assert!(matches!(with_cwd(Ask::List, "/home/x/loja-web"), Ask::List));
    }

    #[test]
    fn an_apps_context_over_500_words_or_with_a_word_over_200_characters_is_refused() {
        let too_large = |ctx: &Context| super::too_large(ctx).unwrap_err();
        let words = |n: usize, len: usize| (0..n).map(|i| format!("{i:0len$}")).collect::<Vec<String>>();
        assert!(super::too_large(&Context { names: words(500, 200), ..Context::default() }).is_ok());
        assert_eq!(too_large(&Context { projects: words(501, 3), ..Context::default() }), "The report's context is too large.");
        assert_eq!(too_large(&Context { files: words(1, 201), ..Context::default() }), "The report's context is too large.");
    }

    #[test]
    fn previews_are_kept_per_account_so_one_account_cannot_push_out_anothers() {
        let kept = reports(None);
        let (ana, bob) = (Uuid::new_v4(), Uuid::new_v4());
        kept.remember_preview(bob, "bob-1", Context { names: vec!["bob-word".into()], ..Context::default() });
        for n in 0..(PREVIEWS_KEPT + 30) {
            kept.remember_preview(ana, &format!("ana-{n}"), Context::default());
        }
        assert_eq!(kept.preview_context(bob, "bob-1").unwrap().names, ["bob-word"], "ana's previews never push out bob's");
        assert!(kept.preview_context(ana, "ana-0").is_none(), "ana's oldest went first");
        assert!(kept.preview_context(ana, &format!("ana-{}", PREVIEWS_KEPT + 29)).is_some());
        assert_eq!(kept.previews.lock().unwrap().keys().filter(|(of, _)| *of == ana).count(), PREVIEWS_KEPT);
    }

    #[test]
    fn the_hubs_own_repository_is_never_scrubbed_in_any_form() {
        let own = reports(Some("FelipeGazapina/pinned"));
        let words = ["pinned", "FelipeGazapina", "FelipeGazapina/pinned", "github.com/FelipeGazapina/pinned", "https://github.com/FelipeGazapina/pinned.git", "git@github.com:FelipeGazapina/pinned.git", "loja-web", "pinned-fork"];
        let ctx = own.without_own(Context { projects: words.iter().map(|w| w.to_string()).collect(), ..Context::default() });
        assert_eq!(ctx.projects, ["loja-web", "pinned-fork"]);
    }

    #[test]
    fn the_poll_interval_is_ten_minutes_and_only_dev_shortens_it() {
        let secs = [(TOKEN_VARIABLE, "t"), ("PINNED_REPORTS_POLL_SECS", "3")];
        assert_eq!(Config::from_env(true, &env(&secs)).map(|c| c.poll), Some(Duration::from_secs(3)));
        assert_eq!(Config::from_env(false, &env(&secs)).map(|c| c.poll), Some(Duration::from_secs(600)));
        assert_eq!(Config::from_env(true, &env(&[(TOKEN_VARIABLE, "t")])).map(|c| c.poll), Some(Duration::from_secs(600)));
        for bad in ["0", "soon", "-1"] {
            assert_eq!(Config::from_env(true, &env(&[(TOKEN_VARIABLE, "t"), ("PINNED_REPORTS_POLL_SECS", bad)])).map(|c| c.poll), Some(Duration::from_secs(600)), "{bad}");
        }
    }

    #[test]
    fn the_wait_is_told_as_a_utc_clock() {
        assert_eq!(clock(1_767_225_600 + 9 * 3600 + 5 * 60 + 59), "09:05 UTC");
        assert_eq!(clock(0), "00:00 UTC");
    }
}
