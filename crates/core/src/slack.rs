use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

const API: &str = "https://slack.com/api/";
/// How far back a channel's history is read to find threads that got a new reply. A thread
/// older than this, or past the newest `HISTORY_PAGES` pages, that gets a new mention or reply
/// is missed. Slack's history lists only top-level messages, each with its thread's latest reply,
/// so polling has no cheaper way to see a reply in an old thread.
const THREAD_LOOKBACK: chrono::Duration = chrono::Duration::days(90);
const PAGE: &str = "200";
/// The most history pages of `PAGE` messages one check reads, newest first.
const HISTORY_PAGES: usize = 5;
/// The error a call fails with once Slack rate limits pinned.
pub const RATE_LIMITED: &str = "Slack: ratelimited";
/// The reaction that tells people a session is working on their message.
pub const WORKING: &str = "robot_face";

#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub team: String,
    /// The bot's own user id, which `<@…>` mentions carry.
    pub user_id: String,
    pub name: String,
}

/// One of the account's Slack bots, each the user's own Slack app with its own token. Its token is
/// a key named `connectors::slack_bot_key(id)`, never part of this.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Bot {
    /// The bot's own user id, which also names its token's key.
    pub id: String,
    /// Empty for a bot moved over from the single-bot store until its token is checked again.
    #[serde(default)]
    pub team_id: String,
    pub team: String,
    /// The bot's own user id, which `<@…>` mentions carry.
    pub user_id: String,
    pub name: String,
    /// Empty until Slack is asked for it.
    #[serde(default)]
    pub app_id: String,
    /// The token's last characters.
    #[serde(default)]
    pub key_hint: String,
}

impl Bot {
    /// The single-bot shape, which `Settings.slack` and older apps read.
    pub fn account(&self) -> Account {
        Account { team: self.team.clone(), user_id: self.user_id.clone(), name: self.name.clone() }
    }
}

/// What `connect_slack` answers: the bot's account, as before, and whether the token added a bot
/// or replaced the token of one already connected.
#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
pub struct Connected {
    #[serde(flatten)]
    pub account: Account,
    pub bot: Bot,
    /// True when the token added a bot, false when it updated the token of a bot already kept.
    pub added: bool,
}

/// A new message that mentions the bot.
#[derive(Debug, PartialEq)]
pub struct Mention {
    /// Timestamp of the thread's first message, which Slack uses as the thread's id.
    pub thread: String,
    /// Timestamp of the message that mentions the bot.
    pub ts: String,
    pub at: DateTime<Utc>,
}

/// New messages in a thread whose session is already answering it.
#[derive(Debug, PartialEq)]
pub struct Replies {
    pub thread: String,
    /// The newest message's timestamp, the bot's own included, which the thread's cursors move to.
    pub newest: String,
    /// The messages people wrote since the session's last one, oldest first. Empty unless one of
    /// them mentions the bot.
    pub posts: Vec<Post>,
    /// The timestamps of the messages in `posts` that mention the bot, oldest first.
    pub mentioned: Vec<String>,
}

/// How far a followed thread was read, as Slack timestamps.
pub struct Cursor {
    /// The newest message its session has.
    pub given: String,
    /// The newest message pinned read, past `given` when people wrote without mentioning the bot.
    pub read: String,
}

/// What one check of a channel finds.
pub struct Activity {
    pub mentions: Vec<Mention>,
    pub replies: Vec<Replies>,
}

#[derive(Debug, PartialEq)]
pub struct Post {
    pub ts: String,
    pub author: String,
    pub text: String,
}

/// Set once by `use_dev_root`. Unset in every process that does not run `--dev`.
static DEV_ROOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn api_root() -> String {
    #[cfg(test)]
    if let Some(root) = API_ROOT.lock().unwrap().clone() {
        return root;
    }
    if let Some(root) = DEV_ROOT.get() {
        return root.clone();
    }
    API.to_owned()
}

/// Where calls go instead of `api` when a debug build's `env` names an http(s) root on a
/// loopback host, as the hub tests' stand-ins; otherwise `api`, so a key never goes to a stranger.
pub(crate) fn loopback_root(env: &str, api: &str) -> String {
    root_for(std::env::var(env).ok().as_deref(), cfg!(debug_assertions), api)
}

/// `loopback_root` with the variable's value and whether this is a debug build given.
fn root_for(set: Option<&str>, debug: bool, api: &str) -> String {
    match set.map(|r| r.trim().trim_end_matches('/')).filter(|r| !r.is_empty()) {
        Some(set) if debug && loopback_http(set).is_some() => set.to_owned(),
        _ => api.to_owned(),
    }
}

fn loopback_http(root: &str) -> Option<()> {
    let url = reqwest::Url::parse(root).ok()?;
    let host = url.host_str()?.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    (matches!(url.scheme(), "http" | "https") && loopback).then_some(())
}

/// A Slack root for a local fake: HTTP or HTTPS on this machine only, normalised to end in one `/`.
pub fn dev_root(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    let host = url.host_str()?.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !matches!(url.scheme(), "http" | "https") || !loopback || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    Some(format!("{}/", url.as_str().trim_end_matches('/')))
}

/// Sends every Slack call to `value` for the rest of the process. Refuses any host but this machine.
/// Only `pinned-server hub --dev` calls this, with PINNED_SLACK_API_BASE, and the hub tests for their stand-in.
pub fn use_dev_root(value: &str) -> Result<(), String> {
    let root = dev_root(value).ok_or_else(|| "Slack dev root must be an http or https URL on this machine".to_owned())?;
    DEV_ROOT.set(root).map_err(|_| "Slack dev root is already set".to_owned())
}

/// Points Slack calls at `root` for a test. `None` restores the real API.
#[cfg(test)]
pub fn set_api_root(root: Option<String>) {
    *API_ROOT.lock().unwrap() = root;
}

/// One test at a time may point Slack at a stand-in.
#[cfg(test)]
pub fn lock_api() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
static API_ROOT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

async fn call(token: &str, method: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    let url = reqwest::Url::parse_with_params(&format!("{}{method}", api_root()), params).map_err(|e| e.to_string())?;
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .timeout(crate::connectors::CALL_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Slack unreachable: {e}"))?;
    answer(response).await
}

/// Slack only reads a JSON body on its write methods.
async fn write(token: &str, method: &str, body: Value) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .post(format!("{}{method}", api_root()))
        .bearer_auth(token)
        .timeout(crate::connectors::CALL_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Slack unreachable: {e}"))?;
    answer(response).await
}

async fn answer(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(RATE_LIMITED.into());
    }
    let body: Value = response.json().await.map_err(|e| format!("Slack sent a bad response: {e}"))?;
    if body["ok"].as_bool() != Some(true) {
        return Err(format!("Slack: {}", body["error"].as_str().unwrap_or(status.as_str())));
    }
    Ok(body)
}

/// The bot a token belongs to, with no key hint.
pub async fn bot(token: &str) -> Result<Bot, String> {
    identify(token).await.map(|(bot, _)| bot)
}

/// The bot a token belongs to, with no key hint, and its bot id (`B…`), which `app_id` takes.
pub async fn identify(token: &str) -> Result<(Bot, String), String> {
    let body = call(token, "auth.test", &[]).await?;
    let Some(bot_id) = body["bot_id"].as_str() else {
        return Err("that is not a bot token; use the one that starts with xoxb-".into());
    };
    let field = |k: &str| body[k].as_str().unwrap_or_default().to_owned();
    Ok((Bot {
        id: field("user_id"),
        team_id: field("team_id"),
        team: field("team"),
        user_id: field("user_id"),
        name: field("user"),
        app_id: String::new(),
        key_hint: String::new(),
    }, bot_id.to_owned()))
}

/// The Slack app id of the bot `bot_id`. Needs the `users:read` scope.
pub async fn app_id(token: &str, bot_id: &str) -> Result<String, String> {
    let body = call(token, "bots.info", &[("bot", bot_id)]).await?;
    Ok(body["bot"]["app_id"].as_str().unwrap_or_default().to_owned())
}

/// Every channel the token can see, public and private, as (id, name, the bot is a member).
async fn channels(token: &str) -> Result<Vec<(String, String, bool)>, String> {
    let mut found = vec![];
    let mut cursor = String::new();
    loop {
        let mut params = vec![("types", "public_channel,private_channel"), ("exclude_archived", "true"), ("limit", "1000")];
        if !cursor.is_empty() {
            params.push(("cursor", &cursor));
        }
        let body = call(token, "conversations.list", &params).await?;
        for c in body["channels"].as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (c["id"].as_str(), c["name"].as_str()) {
                found.push((id.to_owned(), name.to_owned(), c["is_member"].as_bool() == Some(true)));
            }
        }
        cursor = body["response_metadata"]["next_cursor"].as_str().unwrap_or_default().to_owned();
        if cursor.is_empty() {
            return Ok(found);
        }
    }
}

/// Channel names the bot was invited to, for suggesting a connector's channel.
pub async fn member_channels(token: &str) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = channels(token).await?.into_iter().filter(|c| c.2).map(|c| c.1).collect();
    names.sort_unstable();
    Ok(names)
}

/// Channel ids are uppercase, names never are, so an id passes through without a lookup.
fn is_channel_id(channel: &str) -> bool {
    channel.len() >= 9 && channel.starts_with(['C', 'G']) && channel.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// The id of `channel`, given as a name (with or without `#`) or as an id.
pub async fn channel_id(token: &str, channel: &str) -> Result<String, String> {
    let name = channel.trim().trim_start_matches('#');
    if is_channel_id(name) {
        return Ok(name.to_owned());
    }
    channels(token)
        .await?
        .into_iter()
        .find(|c| c.1 == name)
        .map(|c| c.0)
        .ok_or_else(|| format!("Slack has no channel #{name} the bot can see"))
}

pub async fn channel_name(token: &str, id: &str) -> Result<String, String> {
    let body = call(token, "conversations.info", &[("channel", id)]).await?;
    body["channel"]["name"].as_str().map(str::to_owned).ok_or_else(|| "Slack sent no channel name".into())
}

fn is_group_id(group: &str) -> bool {
    group.len() > 1 && group.starts_with('S') && group.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn group_with_handle(groups: &Value, handle: &str) -> Option<String> {
    groups.as_array().into_iter().flatten().find(|g| g["handle"].as_str() == Some(handle)).and_then(|g| g["id"].as_str()).map(str::to_owned)
}

/// The id of the user group `group`, given as its handle (with or without `@`) or as an id.
pub async fn group_id(token: &str, group: &str) -> Result<String, String> {
    let handle = group.trim().trim_start_matches('@');
    if is_group_id(handle) {
        return Ok(handle.to_owned());
    }
    let groups = match call(token, "usergroups.list", &[]).await {
        Ok(body) => body,
        Err(e) if e == "Slack: missing_scope" => {
            return Err(format!("Slack won't let the bot look up @{handle}: add the usergroups:read scope to the Slack app and reinstall it"))
        }
        Err(e) => return Err(e),
    };
    group_with_handle(&groups["usergroups"], handle).ok_or_else(|| format!("Slack has no user group @{handle}: check the handle, or use the group's id"))
}

/// How a message mentions the user group `id`, which notifies every member.
pub fn group_mention(id: &str) -> String {
    format!("<!subteam^{id}>")
}

/// Slack's message timestamp (`"1712345678.123456"`) in microseconds, to compare exactly.
fn micros(ts: &str) -> Option<i64> {
    let (secs, frac) = ts.split_once('.').unwrap_or((ts, ""));
    let frac = format!("{frac:0<6}");
    Some(secs.parse::<i64>().ok()? * 1_000_000 + frac.get(..6)?.parse::<i64>().ok()?)
}

fn ts(at: DateTime<Utc>) -> String {
    let micros = at.timestamp_micros();
    format!("{}.{:06}", micros.div_euclid(1_000_000), micros.rem_euclid(1_000_000))
}

fn mentions(message: &Value, bot: &str) -> bool {
    message["text"].as_str().is_some_and(|t| t.contains(&format!("<@{bot}>")))
}

/// Messages after `since` that mention `bot`, other than the bot's own.
fn mentions_in(messages: &Value, bot: &str, since: DateTime<Utc>) -> Vec<Mention> {
    messages
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["user"].as_str() != Some(bot) && mentions(m, bot))
        .filter_map(|m| {
            let ts = m["ts"].as_str()?;
            let at = DateTime::from_timestamp_micros(micros(ts)?)?;
            let thread = m["thread_ts"].as_str().unwrap_or(ts).to_owned();
            (at > since).then(|| Mention { thread, ts: ts.to_owned(), at })
        })
        .collect()
}

/// Whether `message` is a top-level message, or the first of its thread, after `since` micros.
fn top_level_after(message: &Value, since: i64) -> bool {
    message["ts"].as_str().and_then(micros).is_some_and(|at| at > since)
        && message["thread_ts"].as_str().is_none_or(|thread| Some(thread) == message["ts"].as_str())
}

fn from_people(messages: &Value, bot: &str, since: DateTime<Utc>) -> Vec<Value> {
    let since = since.timestamp_micros();
    let mut people: Vec<Value> = messages
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["user"].as_str().is_some_and(|u| u != bot) && m.get("subtype").is_none() && m.get("bot_id").is_none())
        .filter(|m| top_level_after(m, since))
        .cloned()
        .collect();
    people.sort_by_key(|m| m["ts"].as_str().and_then(micros));
    people
}

/// The most app posts one workflow takes at one check. A channel that is mostly app output can
/// post hundreds a minute, and each post taken costs a classify call and may open a session.
/// Posts past it are skipped, never queued; people's messages are never counted against it.
pub const APPS_PER_CHECK: usize = 10;
/// The longest trigger text an app post gets, in characters. Slack lets one message hold 40,000,
/// far more than a model needs to sort an alert; 4,000 keeps a title, the body and a stack
/// summary and bounds what one post can cost.
pub const APP_TEXT_MAX: usize = 4000;

/// The account's own Slack bots, which a channel read never takes a post from, so a workflow
/// cannot answer itself: their user ids (`U…`) and their bot ids (`B…`).
#[derive(Debug, Default, Clone)]
pub struct Own {
    pub users: std::collections::HashSet<String>,
    pub bots: std::collections::HashSet<String>,
}

impl Own {
    /// Whether one of the account's bots wrote `message`, by its user id or its bot id.
    fn wrote(&self, message: &Value) -> bool {
        message["user"].as_str().is_some_and(|u| self.users.contains(u)) || message["bot_id"].as_str().is_some_and(|b| self.bots.contains(b))
    }
}

/// A workflow that also reads posts from apps and integrations: its id, for the log, and the
/// account's own bots, which it never reads back.
pub struct Apps<'a> {
    pub workflow: &'a str,
    pub own: &'a Own,
}

/// What one history page gives a workflow: the messages it takes, oldest first, and the
/// timestamps of the app posts it left out past `APPS_PER_CHECK`.
#[derive(Debug, Default)]
pub struct Selected {
    pub taken: Vec<Value>,
    pub skipped: Vec<String>,
}

/// A post from an app or integration: a `bot_message`, or a message with a `bot_id` and no
/// other subtype. Every other subtype (edits, joins, broadcasts) is not one.
fn is_app_post(message: &Value) -> bool {
    match message["subtype"].as_str() {
        None => message.get("subtype").is_none() && message["bot_id"].is_string(),
        Some(subtype) => subtype == "bot_message",
    }
}

/// The messages of a history page after `since` that `from_people` takes and, with `apps`, the
/// top-level posts of apps and integrations, up to `APPS_PER_CHECK` of them, oldest first. Never
/// a post by the account's own bots (`own`, and `bot`, the one reading), whether `apps` is on or not.
fn select(messages: &Value, bot: &str, own: &Own, since: DateTime<Utc>, apps: bool) -> Selected {
    let wrote = |m: &Value| own.wrote(m) || m["user"].as_str() == Some(bot);
    let mut taken = from_people(messages, bot, since);
    taken.retain(|m| !wrote(m));
    let mut skipped = vec![];
    if apps {
        let after = since.timestamp_micros();
        let mut posts: Vec<Value> =
            messages.as_array().into_iter().flatten().filter(|m| is_app_post(m) && !wrote(m) && top_level_after(m, after)).cloned().collect();
        posts.sort_by_key(|m| m["ts"].as_str().and_then(micros));
        skipped = posts.split_off(posts.len().min(APPS_PER_CHECK)).iter().filter_map(|m| m["ts"].as_str().map(str::to_owned)).collect();
        taken.extend(posts);
        taken.sort_by_key(|m| m["ts"].as_str().and_then(micros));
    }
    Selected { taken, skipped }
}

/// What the log says of an app post a check left out: the workflow's id and the post's ts, never
/// its text or its author.
fn skipped_line(workflow: &str, ts: &str) -> String {
    format!("pinned: workflow {workflow} skipped app post {ts} over the cap of {APPS_PER_CHECK} per check")
}

/// The text of every part of a rich text block, one line per section or list item.
fn rich_lines(elements: &Value, lines: &mut Vec<String>) {
    fn inline(elements: &Value) -> String {
        elements
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| match e["type"].as_str() {
                Some("text") => e["text"].as_str().unwrap_or_default().to_owned(),
                Some("link") => e["text"].as_str().or(e["url"].as_str()).unwrap_or_default().to_owned(),
                Some("user") => format!("<@{}>", e["user_id"].as_str().unwrap_or_default()),
                Some("channel") => format!("<#{}>", e["channel_id"].as_str().unwrap_or_default()),
                Some("emoji") => format!(":{}:", e["name"].as_str().unwrap_or_default()),
                Some("broadcast") => format!("@{}", e["range"].as_str().unwrap_or("here")),
                _ => String::new(),
            })
            .collect()
    }
    for element in elements.as_array().into_iter().flatten() {
        match element["type"].as_str() {
            Some("rich_text_list") => {
                for item in element["elements"].as_array().into_iter().flatten() {
                    lines.push(inline(&item["elements"]));
                }
            }
            _ => lines.push(inline(&element["elements"])),
        }
    }
}

/// The text of the block kinds that carry some.
fn block_lines(block: &Value, lines: &mut Vec<String>) {
    let text = |v: &Value| v["text"].as_str().map(str::to_owned);
    match block["type"].as_str() {
        Some("header" | "section") => {
            lines.extend(text(&block["text"]));
            lines.extend(block["fields"].as_array().into_iter().flatten().filter_map(text));
        }
        Some("markdown") => lines.extend(text(block)),
        Some("context") => lines.extend(block["elements"].as_array().into_iter().flatten().filter_map(text)),
        Some("rich_text") => rich_lines(&block["elements"], lines),
        _ => {}
    }
}

/// The text of an app post as plain text: `text`, then each attachment's `pretext`, `title`,
/// `text` and `fields` (its `fallback` only when it has nothing else), then the text of its
/// blocks. Lines are joined by newlines, a line already given is not given twice, and the whole
/// is clipped to `APP_TEXT_MAX`. Slack's markup is left as it is, as in a person's message.
fn app_text(message: &Value) -> String {
    let mut lines: Vec<String> = vec![message["text"].as_str().unwrap_or_default().to_owned()];
    for attachment in message["attachments"].as_array().into_iter().flatten() {
        let from = lines.len();
        let field = |key: &str| attachment[key].as_str().map(str::to_owned);
        lines.extend(["pretext", "title", "text"].into_iter().filter_map(field));
        for f in attachment["fields"].as_array().into_iter().flatten() {
            let (title, value) = (f["title"].as_str().unwrap_or_default().trim(), f["value"].as_str().unwrap_or_default().trim());
            lines.push(match (title.is_empty(), value.is_empty()) {
                (false, false) => format!("{title}: {value}"),
                _ => format!("{title}{value}"),
            });
        }
        if lines[from..].iter().all(|l| l.trim().is_empty()) {
            lines.extend(field("fallback"));
        }
    }
    for block in message["blocks"].as_array().into_iter().flatten() {
        block_lines(block, &mut lines);
    }
    let mut seen = std::collections::HashSet::new();
    let text = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty() && seen.insert(*l)).collect::<Vec<_>>().join("\n");
    match text.chars().count() > APP_TEXT_MAX {
        true => text.chars().take(APP_TEXT_MAX - 1).chain(['…']).collect(),
        false => text,
    }
}

/// The app or bot an app post is by: its name, never a person looked up.
fn app_author(message: &Value) -> String {
    message["bot_profile"]["name"].as_str().or(message["username"].as_str()).filter(|n| !n.is_empty()).unwrap_or("an app").to_owned()
}

fn app_post(message: &Value) -> Post {
    Post { ts: message["ts"].as_str().unwrap_or_default().to_owned(), author: app_author(message), text: app_text(message) }
}

/// The top-level messages people wrote in `channel` after `since`, oldest first, by author name.
/// With `apps`, also the posts of apps and integrations (`select`), by the app's name, with no
/// person looked up for them; each one past `APPS_PER_CHECK` is logged by its workflow and ts.
pub async fn messages(token: &str, channel: &str, bot: &str, since: DateTime<Utc>, apps: Option<&Apps<'_>>) -> Result<Vec<Post>, String> {
    let history = call(token, "conversations.history", &[("channel", channel), ("oldest", &ts(since)), ("limit", PAGE)]).await?;
    let none = Own::default();
    let Selected { taken, skipped } = select(&history["messages"], bot, apps.map_or(&none, |a| a.own), since, apps.is_some());
    for ts in &skipped {
        eprintln!("{}", skipped_line(apps.map_or("", |a| a.workflow), ts));
    }
    let (from_apps, people): (Vec<Value>, Vec<Value>) = taken.into_iter().partition(is_app_post);
    let people = Value::Array(people);
    let mut posts = posts(&people, &names(token, &people).await?);
    posts.extend(from_apps.iter().map(app_post));
    posts.sort_by_key(|p| micros(&p.ts));
    Ok(posts)
}

/// Threads in a history page whose last reply came after `since`.
fn threads_replied_after(history: &Value, since: DateTime<Utc>) -> Vec<String> {
    let since = since.timestamp_micros();
    history
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["latest_reply"].as_str().and_then(micros).is_some_and(|at| at > since))
        .filter_map(|m| m["ts"].as_str().map(str::to_owned))
        .collect()
}

/// Threads of a history page in `cursors`, by thread ts, whose last reply pinned hasn't read,
/// with the newest message their session has.
fn followed_with_replies<'a>(history: &Value, cursors: &'a BTreeMap<String, Cursor>) -> Vec<(String, &'a str)> {
    history
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let (thread, cursor) = cursors.get_key_value(m["ts"].as_str()?)?;
            (micros(m["latest_reply"].as_str()?)? > micros(&cursor.read)?).then(|| (thread.clone(), cursor.given.as_str()))
        })
        .collect()
}

/// The newest timestamp after `cursor` in a thread, and the messages after it that people wrote,
/// neither the bot's own nor another bot's. People talk among themselves in a thread, so their
/// messages come back only once one of them mentions the bot.
fn after_cursor(messages: &Value, bot: &str, cursor: &str) -> Option<(String, Vec<Value>)> {
    let cursor = micros(cursor)?;
    let new: Vec<&Value> = messages
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["ts"].as_str().and_then(micros).is_some_and(|at| at > cursor))
        .collect();
    let newest = new.iter().filter_map(|m| m["ts"].as_str()).max_by_key(|ts| micros(ts))?.to_owned();
    let mut people: Vec<Value> = new
        .into_iter()
        .filter(|m| m["user"].as_str() != Some(bot) && m["subtype"].as_str() != Some("bot_message"))
        .cloned()
        .collect();
    if !people.iter().any(|m| mentions(m, bot)) {
        people.clear();
    }
    Some((newest, people))
}

/// The timestamps of the messages that mention `bot`.
fn mentioning(messages: &[Value], bot: &str) -> Vec<String> {
    messages.iter().filter(|m| mentions(m, bot)).filter_map(|m| m["ts"].as_str().map(str::to_owned)).collect()
}

/// Mentions of `bot` in `channel` after `since`, oldest first, and new messages in the threads
/// `cursors` follows, keyed by thread ts. A mention in a thread no cursor follows counts however
/// old the thread is, within `THREAD_LOOKBACK`. Takes a history call per page of top-level
/// messages in that window, at most `HISTORY_PAGES`, plus one replies call per thread that got a
/// reply since, because Slack rate limits both.
pub async fn activity(
    token: &str,
    channel: &str,
    bot: &str,
    since: DateTime<Utc>,
    cursors: &BTreeMap<String, Cursor>,
) -> Result<Activity, String> {
    let oldest = ts(since - THREAD_LOOKBACK);
    let messages = &pages(token, "conversations.history", &[("channel", channel), ("oldest", &oldest), ("limit", PAGE)], HISTORY_PAGES).await?;
    let mut mentions = mentions_in(messages, bot, since);
    // A followed thread already has its session, so a mention in it is just another reply.
    for thread in threads_replied_after(messages, since).into_iter().filter(|t| !cursors.contains_key(t)) {
        let replies = call(token, "conversations.replies", &[("channel", channel), ("ts", &thread), ("limit", PAGE)]).await?;
        mentions.extend(mentions_in(&replies["messages"], bot, since));
    }
    mentions.sort_by_key(|m| m.at);
    mentions.dedup();
    let mut replies = vec![];
    for (thread, given) in followed_with_replies(messages, cursors) {
        let Some((newest, people)) = after_cursor(&replies_after(token, channel, &thread, given).await?, bot, given) else { continue };
        let mentioned = mentioning(&people, bot);
        let people = Value::Array(people);
        let names = names(token, &people).await?;
        replies.push(Replies { thread, newest, posts: posts(&people, &names), mentioned });
    }
    Ok(Activity { mentions, replies })
}

/// The thread's messages after `oldest`, every page of them, so a mention past the first page
/// isn't missed.
async fn replies_after(token: &str, channel: &str, thread: &str, oldest: &str) -> Result<Value, String> {
    pages(token, "conversations.replies", &[("channel", channel), ("ts", thread), ("oldest", oldest), ("limit", PAGE)], usize::MAX).await
}

/// The messages of up to `most` pages of `method`.
async fn pages(token: &str, method: &str, params: &[(&str, &str)], most: usize) -> Result<Value, String> {
    let mut messages = vec![];
    let mut page = None;
    for _ in 0..most {
        let mut params = params.to_vec();
        params.extend(page.as_deref().map(|p| ("cursor", p)));
        let body = call(token, method, &params).await?;
        messages.extend(body["messages"].as_array().into_iter().flatten().cloned());
        match body["response_metadata"]["next_cursor"].as_str() {
            Some(next) if !next.is_empty() => page = Some(next.to_owned()),
            _ => break,
        }
    }
    Ok(Value::Array(messages))
}

/// Users a thread names, as authors or `<@…>` mentions.
fn user_ids(messages: &Value) -> Vec<String> {
    let mut ids = vec![];
    for m in messages.as_array().into_iter().flatten() {
        ids.extend(m["user"].as_str().map(str::to_owned));
        let text = m["text"].as_str().unwrap_or_default();
        ids.extend(text.split("<@").skip(1).filter_map(|rest| rest.split_once('>')).map(|(id, _)| id.to_owned()));
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The thread's messages with user ids replaced by the names in `names`.
fn posts(messages: &Value, names: &HashMap<String, String>) -> Vec<Post> {
    let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_owned());
    messages
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let author = match m["user"].as_str() {
                Some(id) => name(id),
                None => m["bot_profile"]["name"].as_str().or(m["username"].as_str()).unwrap_or("someone").to_owned(),
            };
            let mut text = m["text"].as_str().unwrap_or_default().to_owned();
            for (id, n) in names {
                text = text.replace(&format!("<@{id}>"), &format!("@{n}"));
            }
            Post { ts: m["ts"].as_str().unwrap_or_default().to_owned(), author, text }
        })
        .collect()
}

/// Every message of the thread, oldest first.
pub async fn thread(token: &str, channel: &str, thread: &str) -> Result<Vec<Post>, String> {
    thread_with(token, channel, thread, None).await
}

/// `thread`, and with `apps` the posts of apps and integrations in it read as `app_post` reads
/// them, by the app's name and with no person looked up, as the message that started the thread
/// was. Messages of the account's own bots and of people read as they always did.
pub async fn thread_with(token: &str, channel: &str, thread: &str, apps: Option<&Apps<'_>>) -> Result<Vec<Post>, String> {
    let replies = call(token, "conversations.replies", &[("channel", channel), ("ts", thread), ("limit", "1000")]).await?;
    let messages = &replies["messages"];
    let Some(apps) = apps else { return Ok(posts(messages, &names(token, messages).await?)) };
    let by_app = |m: &Value| is_app_post(m) && !apps.own.wrote(m);
    let rest = Value::Array(messages.as_array().into_iter().flatten().filter(|m| !by_app(m)).cloned().collect());
    let mut people = posts(&rest, &names(token, &rest).await?).into_iter();
    Ok(messages.as_array().into_iter().flatten().filter_map(|m| if by_app(m) { Some(app_post(m)) } else { people.next() }).collect())
}

/// The display name of each user `messages` names.
async fn names(token: &str, messages: &Value) -> Result<HashMap<String, String>, String> {
    let mut names = HashMap::new();
    for id in user_ids(messages) {
        let user = call(token, "users.info", &[("user", &id)]).await?;
        let profile = &user["user"]["profile"];
        let name = [&profile["display_name"], &profile["real_name"], &user["user"]["name"]]
            .into_iter()
            .filter_map(|v| v.as_str())
            .find(|n| !n.is_empty())
            .unwrap_or(&id)
            .to_owned();
        names.insert(id, name);
    }
    Ok(names)
}

/// A link to the thread, for the Linear issue a session files from it.
pub async fn permalink(token: &str, channel: &str, thread: &str) -> Result<String, String> {
    let body = call(token, "chat.getPermalink", &[("channel", channel), ("message_ts", thread)]).await?;
    body["permalink"].as_str().map(str::to_owned).ok_or_else(|| "Slack sent no permalink".into())
}

pub async fn post(token: &str, channel: &str, thread: &str, text: &str) -> Result<(), String> {
    write(token, "chat.postMessage", json!({ "channel": channel, "thread_ts": thread, "text": text })).await.map(drop)
}

/// A new message in `channel`, not in a thread.
pub async fn announce(token: &str, channel: &str, text: &str) -> Result<(), String> {
    write(token, "chat.postMessage", json!({ "channel": channel, "text": text })).await.map(drop)
}

/// Whether `handle` is a Slack user-group handle: 1 to 80 of `a-z`, `0-9`, `.`, `_` and `-`.
pub fn is_group_handle(handle: &str) -> bool {
    (1..=80).contains(&handle.len()) && handle.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Reacts to the message `ts` with the emoji `name`. A reaction the bot already left counts as
/// done, so a retry after a crash converges.
pub async fn react(token: &str, channel: &str, ts: &str, name: &str) -> Result<(), String> {
    match write(token, "reactions.add", json!({ "channel": channel, "timestamp": ts, "name": name })).await {
        Ok(_) => Ok(()),
        Err(e) if e == "Slack: already_reacted" => Ok(()),
        // Apps installed before pinned reacted lack the scope.
        Err(e) if e == "Slack: missing_scope" => Err("Slack won't let the bot react: add the reactions:write scope to the Slack app and reinstall it".into()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().into()
    }

    /// A channel as `conversations.history` and `conversations.replies` return it: an old thread
    /// with a fresh reply, a new thread that mentions the bot, the bot's own reply, and chatter.
    fn fixture() -> (Value, Value) {
        let history = json!([
            { "ts": "1792400100.000300", "user": "U2", "text": "lunch?" },
            { "ts": "1792400050.000200", "user": "U1", "text": "<@UBOT> the export button 500s", "thread_ts": "1792400050.000200", "latest_reply": "1792400060.000100", "reply_count": 1 },
            { "ts": "1792390000.000100", "user": "U2", "text": "deploy notes", "thread_ts": "1792390000.000100", "latest_reply": "1792400090.000000", "reply_count": 2 },
            { "ts": "1792380000.000000", "user": "U1", "text": "<@UBOT> old ask, before the connector", "thread_ts": "1792380000.000000", "latest_reply": "1792380100.000000", "reply_count": 1 },
        ]);
        let replies = json!([
            { "ts": "1792390000.000100", "user": "U2", "text": "deploy notes", "thread_ts": "1792390000.000100" },
            { "ts": "1792390500.000000", "user": "U1", "text": "<@UBOT> before the watermark", "thread_ts": "1792390000.000100" },
            { "ts": "1792400080.000000", "user": "UBOT", "text": "I am <@UBOT>", "thread_ts": "1792390000.000100" },
            { "ts": "1792400090.000000", "user": "U3", "text": "<@UBOT> why did the deploy fail?", "thread_ts": "1792390000.000100" },
        ]);
        (history, replies)
    }

    #[test]
    fn mentions_are_new_messages_to_the_bot_keyed_by_thread() {
        let (history, replies) = fixture();
        let since = DateTime::from_timestamp(1792395000, 0).unwrap();
        assert_eq!(
            mentions_in(&history, "UBOT", since),
            [Mention {
                thread: "1792400050.000200".into(),
                ts: "1792400050.000200".into(),
                at: DateTime::from_timestamp(1792400050, 200_000).unwrap()
            }]
        );
        assert_eq!(threads_replied_after(&history, since), ["1792400050.000200", "1792390000.000100"]);
        assert_eq!(
            mentions_in(&replies, "UBOT", since),
            [Mention {
                thread: "1792390000.000100".into(),
                ts: "1792400090.000000".into(),
                at: DateTime::from_timestamp(1792400090, 0).unwrap()
            }]
        );
    }

    #[test]
    fn followed_threads_get_a_replies_call_only_past_what_was_read() {
        let (history, _) = fixture();
        let cursor = |given: &str, read: &str| Cursor { given: given.into(), read: read.into() };
        let cursors = BTreeMap::from([
            ("1792400050.000200".to_owned(), cursor("1792400050.000200", "1792400060.000100")),
            ("1792390000.000100".to_owned(), cursor("1792400070.000000", "1792400080.000000")),
            ("1792370000.000000".to_owned(), cursor("1792370000.000000", "1792370000.000000")),
        ]);
        assert_eq!(followed_with_replies(&history, &cursors), [("1792390000.000100".to_owned(), "1792400070.000000")]);
        assert_eq!(followed_with_replies(&history, &BTreeMap::new()), []);
    }

    #[test]
    fn follow_ups_are_what_people_wrote_after_the_cursor_once_one_mentions_the_bot() {
        let replies = json!([
            { "ts": "1792390000.000100", "user": "U2", "text": "deploy notes" },
            { "ts": "1792400080.000000", "user": "UBOT", "text": "the cause is X; file it?" },
            { "ts": "1792400085.000000", "user": "U2", "text": "I think we should" },
            { "ts": "1792400090.000000", "user": "U3", "text": "<@UBOT> yes, create the task" },
            { "ts": "1792400095.000000", "subtype": "bot_message", "bot_id": "B9", "text": "deploy finished" },
            { "ts": "1792400099.000000", "user": "UBOT", "text": "on it" },
            { "ts": "1792400105.000000", "user": "U2", "text": "thanks!" },
        ]);
        let (newest, people) = after_cursor(&replies, "UBOT", "1792400080.000000").unwrap();
        assert_eq!(newest, "1792400105.000000");
        assert_eq!(
            people,
            [
                json!({ "ts": "1792400085.000000", "user": "U2", "text": "I think we should" }),
                json!({ "ts": "1792400090.000000", "user": "U3", "text": "<@UBOT> yes, create the task" }),
                json!({ "ts": "1792400105.000000", "user": "U2", "text": "thanks!" }),
            ]
        );
        assert_eq!(mentioning(&people, "UBOT"), ["1792400090.000000"], "only the message to the bot gets the reaction");
        let two = [json!({ "ts": "1.1", "user": "U2", "text": "<@UBOT> first" }), json!({ "ts": "1.2", "user": "U3", "text": "<@U2> and <@UBOT> again" })];
        assert_eq!(mentioning(&two, "UBOT"), ["1.1", "1.2"]);
        let (newest, people) = after_cursor(&replies, "UBOT", "1792400090.000000").unwrap();
        assert_eq!((newest.as_str(), people.len()), ("1792400105.000000", 0));
        let (newest, people) = after_cursor(&replies, "UBOT", "1792400099.000000").unwrap();
        assert_eq!((newest.as_str(), people.len()), ("1792400105.000000", 0));
        assert_eq!(after_cursor(&replies, "UBOT", "1792400105.000000"), None);
    }

    #[test]
    fn new_messages_are_what_people_started_in_the_channel() {
        let history = json!([
            { "ts": "1792400110.000000", "user": "U3", "text": "also in the thread", "thread_ts": "1792400050.000200", "subtype": "thread_broadcast" },
            { "ts": "1792400105.000000", "user": "U2", "text": "replied and sent to the channel", "thread_ts": "1792400050.000200" },
            { "ts": "1792400100.000300", "user": "U2", "text": "lunch?" },
            { "ts": "1792400090.000000", "user": "UBOT", "text": "I am the bot" },
            { "ts": "1792400080.000000", "bot_id": "B9", "user": "U9", "text": "deploy finished" },
            { "ts": "1792400070.000000", "subtype": "channel_join", "user": "U4", "text": "<@U4> has joined the channel" },
            { "ts": "1792400050.000200", "user": "U1", "text": "the export button 500s", "thread_ts": "1792400050.000200", "latest_reply": "1792400105.000000" },
            { "ts": "1792390000.000100", "user": "U1", "text": "before the watermark" },
        ]);
        let since = DateTime::from_timestamp(1792395000, 0).unwrap();
        assert_eq!(
            from_people(&history, "UBOT", since),
            [
                json!({ "ts": "1792400050.000200", "user": "U1", "text": "the export button 500s", "thread_ts": "1792400050.000200", "latest_reply": "1792400105.000000" }),
                json!({ "ts": "1792400100.000300", "user": "U2", "text": "lunch?" }),
            ]
        );
    }

    fn own_bots() -> Own {
        Own { users: ["UBOT".to_owned(), "UBOT2".to_owned()].into(), bots: ["B1".to_owned(), "B2".to_owned()].into() }
    }

    /// One history page of #alerts: a person, an integration, the account's own bots, and every
    /// other kind of row Slack lists.
    fn mixed_page() -> Value {
        json!([
            { "ts": "1792400190.000000", "subtype": "message_changed", "bot_id": "B9", "channel": "C1", "message": { "text": "edited" } },
            { "ts": "1792400180.000000", "subtype": "thread_broadcast", "bot_id": "B9", "thread_ts": "1792400000.000000", "text": "also in the channel" },
            { "ts": "1792400170.000000", "subtype": "channel_join", "bot_id": "B9", "user": "U9", "text": "<@U9> has joined the channel" },
            { "ts": "1792400160.000000", "subtype": "bot_message", "bot_id": "B9", "thread_ts": "1792400000.000000", "username": "CI Bot", "text": "a reply in a thread" },
            { "ts": "1792400150.000000", "subtype": "bot_message", "bot_id": "B2", "username": "pinned-ops", "text": "posted by the second pinned bot's bot id" },
            { "ts": "1792400140.000000", "subtype": "bot_message", "bot_id": "B77", "user": "UBOT2", "text": "posted by the second pinned bot's user id" },
            { "ts": "1792400130.000000", "user": "UBOT", "bot_id": "B1", "text": "posted by the default pinned bot" },
            {
                "ts": "1792400120.000000", "subtype": "bot_message", "bot_id": "B9", "username": "CI Bot", "text": "",
                "attachments": [{ "title": "Deploy failed", "text": "api went red on 3 hosts", "fallback": "Deploy failed" }]
            },
            { "ts": "1792400110.000000", "user": "U1", "text": "can someone look at the footer?" },
            { "ts": "1792390000.000000", "subtype": "bot_message", "bot_id": "B9", "username": "CI Bot", "text": "before the watermark" },
        ])
    }

    fn ts_of(messages: &[Value]) -> Vec<&str> {
        messages.iter().filter_map(|m| m["ts"].as_str()).collect()
    }

    #[test]
    fn a_channel_read_without_apps_takes_only_what_people_started() {
        let since = DateTime::from_timestamp(1792395000, 0).unwrap();
        let selected = select(&mixed_page(), "UBOT", &own_bots(), since, false);
        assert_eq!(ts_of(&selected.taken), ["1792400110.000000"]);
        assert_eq!(selected.taken, from_people(&mixed_page(), "UBOT", since), "with apps off the page reads as it always did");
        assert!(selected.skipped.is_empty());
    }

    #[test]
    fn a_channel_read_with_apps_adds_integration_posts_and_nothing_else() {
        let since = DateTime::from_timestamp(1792395000, 0).unwrap();
        let selected = select(&mixed_page(), "UBOT", &own_bots(), since, true);
        assert_eq!(
            ts_of(&selected.taken),
            ["1792400110.000000", "1792400120.000000"],
            "the person's message and the integration's post; not the pinned bots, the thread reply, the join, the edit or the broadcast"
        );
        assert!(selected.skipped.is_empty());
    }

    #[test]
    fn an_app_post_with_no_text_reads_from_its_attachments() {
        let alert = json!({
            "subtype": "bot_message", "bot_id": "B9", "username": "CI Bot", "text": "",
            "attachments": [{
                "pretext": "Pipeline #42", "title": "Deploy failed", "text": "api went red on 3 hosts",
                "fields": [{ "title": "Region", "value": "eu-west", "short": true }], "fallback": "Deploy failed"
            }]
        });
        assert_eq!(app_text(&alert), "Pipeline #42\nDeploy failed\napi went red on 3 hosts\nRegion: eu-west");
        let only_fallback = json!({ "subtype": "bot_message", "attachments": [{ "fallback": "Build 7 passed" }] });
        assert_eq!(app_text(&only_fallback), "Build 7 passed");
    }

    #[test]
    fn an_app_post_with_no_text_reads_from_its_blocks() {
        let notice = json!({
            "subtype": "bot_message", "bot_id": "B9", "text": "",
            "blocks": [
                { "type": "header", "text": { "type": "plain_text", "text": "PR merged" } },
                { "type": "section", "text": { "type": "mrkdwn", "text": "Ana merged *fix footer*" }, "fields": [{ "type": "mrkdwn", "text": "Branch: main" }] },
                { "type": "context", "elements": [{ "type": "mrkdwn", "text": "repo web" }, { "type": "image", "alt_text": "avatar" }] },
                { "type": "divider" },
                { "type": "rich_text", "elements": [{ "type": "rich_text_section", "elements": [
                    { "type": "text", "text": "see " }, { "type": "link", "url": "https://example.test/pr/1", "text": "the PR" }
                ] }] }
            ]
        });
        assert_eq!(app_text(&notice), "PR merged\nAna merged *fix footer*\nBranch: main\nrepo web\nsee the PR");
    }

    #[test]
    fn text_that_blocks_and_attachments_repeat_is_given_once() {
        let post = json!({
            "subtype": "bot_message", "text": "Deploy failed",
            "attachments": [{ "title": "Deploy failed", "text": "api went red" }],
            "blocks": [{ "type": "section", "text": { "type": "mrkdwn", "text": "Deploy failed" } }]
        });
        assert_eq!(app_text(&post), "Deploy failed\napi went red");
    }

    #[test]
    fn an_app_post_longer_than_the_clip_is_clipped() {
        let post = json!({ "subtype": "bot_message", "text": "x".repeat(APP_TEXT_MAX + 500) });
        let text = app_text(&post);
        assert_eq!(text.chars().count(), APP_TEXT_MAX);
        assert!(text.ends_with('…'));
        assert_eq!(app_text(&json!({ "subtype": "bot_message", "text": "short" })), "short");
    }

    #[test]
    fn an_app_post_is_by_the_app_not_by_a_person() {
        assert_eq!(app_author(&json!({ "username": "CI Bot", "bot_profile": { "name": "Other" } })), "Other");
        assert_eq!(app_author(&json!({ "username": "CI Bot" })), "CI Bot");
        assert_eq!(app_author(&json!({ "bot_id": "B9" })), "an app");
    }

    #[test]
    fn one_check_takes_at_most_the_cap_of_app_posts_and_every_message_from_people() {
        let mut page: Vec<Value> = (0..15)
            .map(|i| json!({ "ts": format!("17924001{:02}.000000", i), "subtype": "bot_message", "bot_id": "B9", "username": "CI Bot", "text": format!("alert {i}") }))
            .collect();
        page.extend((0..3).map(|i| json!({ "ts": format!("17924002{:02}.000000", i), "user": format!("U{i}"), "text": "person" })));
        let since = DateTime::from_timestamp(1792395000, 0).unwrap();
        let selected = select(&Value::Array(page), "UBOT", &own_bots(), since, true);
        let apps = selected.taken.iter().filter(|m| m["bot_id"].is_string()).count();
        let people = selected.taken.iter().filter(|m| m["user"].is_string()).count();
        assert_eq!((apps, people), (APPS_PER_CHECK, 3));
        assert_eq!(
            selected.skipped,
            ["1792400110.000000", "1792400111.000000", "1792400112.000000", "1792400113.000000", "1792400114.000000"],
            "the newest app posts past the cap are skipped, by ts"
        );
        assert_eq!(skipped_line("wf1", "1792400114.000000"), "pinned: workflow wf1 skipped app post 1792400114.000000 over the cap of 10 per check");
    }

    #[test]
    fn a_user_group_is_named_by_id_or_by_handle() {
        assert!(is_group_id("S0123ABC"));
        assert!(!is_group_id("prod-engg"));
        assert!(!is_group_id("SUPPORT-team"));
        assert!(!is_group_id("S"));
        let groups = json!([
            { "id": "S0AAA111", "handle": "design", "name": "Design" },
            { "id": "S0123ABC", "handle": "prod-engg", "name": "Production engineers" },
        ]);
        assert_eq!(group_with_handle(&groups, "prod-engg"), Some("S0123ABC".into()));
        assert_eq!(group_with_handle(&groups, "Production engineers"), None);
        assert_eq!(group_mention("S0123ABC"), "<!subteam^S0123ABC>");
        assert_eq!(block_on(group_id("xoxb-unused", " @S0123ABC ")), Ok("S0123ABC".into()), "an id needs no lookup");
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
    }

    #[test]
    fn slack_timestamps_convert_both_ways() {
        assert_eq!(ts(at("2026-10-19T10:00:00.25Z")), "1792404000.250000");
        assert_eq!(micros("1792404000.250000"), Some(1_792_404_000_250_000));
        assert_eq!(micros("1792404000"), Some(1_792_404_000_000_000));
        assert_eq!(micros("nope"), None);
    }

    #[test]
    fn channel_ids_pass_through_and_names_do_not() {
        assert!(is_channel_id("C0123ABCD45"));
        assert!(is_channel_id("G0123ABCD"));
        assert!(!is_channel_id("general"));
        assert!(!is_channel_id("CORE"));
    }

    #[test]
    fn thread_posts_name_their_authors_and_mentions() {
        let (_, replies) = fixture();
        assert_eq!(user_ids(&replies), ["U1", "U2", "U3", "UBOT"]);
        let names = HashMap::from([("U1".to_owned(), "ana".to_owned()), ("UBOT".to_owned(), "pinned".to_owned())]);
        let thread = json!([
            { "ts": "1792400050.000200", "user": "U1", "text": "<@UBOT> it broke for <@U9>" },
            { "ts": "1792400060.000000", "bot_profile": { "name": "deploybot" }, "text": "deploy failed" },
        ]);
        assert_eq!(
            posts(&thread, &names),
            [
                Post { ts: "1792400050.000200".into(), author: "ana".into(), text: "@pinned it broke for <@U9>".into() },
                Post { ts: "1792400060.000000".into(), author: "deploybot".into(), text: "deploy failed".into() },
            ]
        );
    }

    #[test]
    fn dev_root_keeps_loopback_urls_and_ends_them_in_one_slash() {
        assert_eq!(dev_root("http://127.0.0.1:8813/api").as_deref(), Some("http://127.0.0.1:8813/api/"));
        assert_eq!(dev_root("http://localhost:9/api/").as_deref(), Some("http://localhost:9/api/"));
        assert_eq!(dev_root("http://[::1]:9/api/").as_deref(), Some("http://[::1]:9/api/"));
    }

    #[test]
    fn dev_root_refuses_every_other_host_and_shape() {
        assert_eq!(dev_root("https://slack.com/api/"), None);
        assert_eq!(dev_root("http://10.0.0.5/api/"), None);
        assert_eq!(dev_root("http://127.0.0.1.example.com/"), None);
        assert_eq!(dev_root("http://127.0.0.1:8813/api?x=1"), None);
        assert_eq!(dev_root("garbage"), None);
    }

    #[test]
    fn use_dev_root_refuses_a_public_host_and_stays_unset() {
        assert!(use_dev_root("https://slack.com/api/").is_err());
        assert!(DEV_ROOT.get().is_none());
    }

    #[test]
    fn slack_base_env_is_ignored_without_dev() {
        let _guard = lock_api();
        set_api_root(None);
        std::env::set_var("PINNED_SLACK_API_BASE", "http://127.0.0.1:9/api/");
        let root = api_root();
        std::env::remove_var("PINNED_SLACK_API_BASE");
        assert_eq!(root, "https://slack.com/api/");
    }

    /// Linear's and Vercel's stand-ins (`loopback_root`): a debug build's loopback address only.
    #[test]
    fn a_stand_in_root_reaches_only_a_loopback_host_in_a_debug_build() {
        const REAL: &str = "https://api.example.test";
        for set in ["http://127.0.0.1:5000/graphql", "http://localhost:5000/v1/", "https://[::1]:5000"] {
            assert_eq!(root_for(Some(set), true, REAL), set.trim_end_matches('/'), "{set}");
            assert_eq!(root_for(Some(set), false, REAL), REAL, "{set} in a release build");
        }
        for set in ["https://evil.example.test/v1", "http://127.0.0.1.example.test/", "http://169.254.169.254/", "ftp://127.0.0.1/", "not a url"] {
            assert_eq!(root_for(Some(set), true, REAL), REAL, "{set}");
        }
        assert_eq!(root_for(None, true, REAL), REAL);
    }

    /// The only way to point Slack elsewhere is `hub --dev`'s root: the variable older hub tests
    /// set reaches nothing, a loopback address in a debug build included.
    #[test]
    fn the_retired_root_variable_is_ignored_in_a_debug_build() {
        const RETIRED: &str = concat!("PINNED_SLACK", "_API_ROOT");
        let _guard = lock_api();
        set_api_root(None);
        std::env::set_var(RETIRED, "http://127.0.0.1:9/api/");
        let root = api_root();
        std::env::remove_var(RETIRED);
        assert_eq!(root, "https://slack.com/api/");
    }
}
