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
    /// What the message attaches, as `conversations.replies` lists it.
    pub files: Vec<SlackFile>,
}

/// A file a message attaches. `url_private` downloads it with the bot token.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SlackFile {
    pub id: String,
    pub name: String,
    pub mimetype: String,
    pub size: u64,
    pub url_private: String,
}

/// The files one message attaches.
fn files(message: &Value) -> Vec<SlackFile> {
    let text = |file: &Value, key: &str| file[key].as_str().unwrap_or_default().to_owned();
    message["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|file| file["id"].is_string())
        .map(|file| SlackFile {
            id: text(file, "id"),
            name: text(file, "name"),
            mimetype: text(file, "mimetype"),
            size: file["size"].as_u64().unwrap_or_default(),
            url_private: text(file, "url_private"),
        })
        .collect()
}

fn api_root() -> String {
    #[cfg(test)]
    if let Some(root) = API_ROOT.lock().unwrap().clone() {
        return root;
    }
    API.to_owned()
}

/// Points Slack calls at `root` for a test. `None` restores the real API.
#[cfg(test)]
pub fn set_api_root(root: Option<String>) {
    if root.is_none() {
        *FILE_WAIT.lock().unwrap() = None;
    }
    *API_ROOT.lock().unwrap() = root;
}

/// Shortens the wait for one file for a test; `set_api_root(None)` restores it.
#[cfg(test)]
pub fn set_file_wait(wait: std::time::Duration) {
    *FILE_WAIT.lock().unwrap() = Some(wait);
}

#[cfg(test)]
static FILE_WAIT: std::sync::Mutex<Option<std::time::Duration>> = std::sync::Mutex::new(None);

fn file_wait() -> std::time::Duration {
    #[cfg(test)]
    if let Some(wait) = *FILE_WAIT.lock().unwrap() {
        return wait;
    }
    FILE_REQUEST_WAIT
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

pub async fn account(token: &str) -> Result<Account, String> {
    let body = call(token, "auth.test", &[]).await?;
    if body["bot_id"].as_str().is_none() {
        return Err("that is not a bot token; use the one that starts with xoxb-".into());
    }
    let field = |k: &str| body[k].as_str().unwrap_or_default().to_owned();
    Ok(Account { team: field("team"), user_id: field("user_id"), name: field("user") })
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

fn from_people(messages: &Value, bot: &str, since: DateTime<Utc>) -> Vec<Value> {
    let since = since.timestamp_micros();
    let mut people: Vec<Value> = messages
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["user"].as_str().is_some_and(|u| u != bot) && m.get("subtype").is_none() && m.get("bot_id").is_none())
        .filter(|m| m["ts"].as_str().and_then(micros).is_some_and(|at| at > since))
        .filter(|m| m["thread_ts"].as_str().is_none_or(|thread| Some(thread) == m["ts"].as_str()))
        .cloned()
        .collect();
    people.sort_by_key(|m| m["ts"].as_str().and_then(micros));
    people
}

/// The top-level messages people wrote in `channel` after `since`, oldest first, by author name.
pub async fn messages(token: &str, channel: &str, bot: &str, since: DateTime<Utc>) -> Result<Vec<Post>, String> {
    let history = call(token, "conversations.history", &[("channel", channel), ("oldest", &ts(since)), ("limit", PAGE)]).await?;
    let people = Value::Array(from_people(&history["messages"], bot, since));
    Ok(posts(&people, &names(token, &people).await?))
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
            Post { ts: m["ts"].as_str().unwrap_or_default().to_owned(), author, text, files: files(m) }
        })
        .collect()
}

/// Every message of the thread, oldest first.
pub async fn thread(token: &str, channel: &str, thread: &str) -> Result<Vec<Post>, String> {
    let replies = call(token, "conversations.replies", &[("channel", channel), ("ts", thread), ("limit", "1000")]).await?;
    Ok(posts(&replies["messages"], &names(token, &replies["messages"]).await?))
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

/// What `pinned slack files` prints when the Slack app may not read files.
pub const NO_FILES_SCOPE: &str =
    "pinned could not read this thread's files: the Slack app lacks the files:read scope; ask the workspace admin to reinstall it with files:read.";
/// The biggest file `pinned slack files` downloads.
pub const FILE_LIMIT: u64 = 20 << 20;
/// The most files one `pinned slack files` downloads.
pub const FILE_COUNT: usize = 20;
/// The most bytes one `pinned slack files` downloads, all of which travel to the session's machine
/// in base64 in one websocket message.
pub const FILES_TOTAL: u64 = 40 << 20;
/// `download_all` starts no new download this long after it began. It stays under the machine's
/// wait for the hub's answer, so what the hub downloaded still arrives.
pub const DOWNLOAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(75);
/// How long one file may take, from the request to its last byte.
const FILE_REQUEST_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
const _: () = assert!((FILES_TOTAL as usize).div_ceil(3) * 4 + (1 << 20) <= crate::sync::MESSAGE_LIMIT);

/// A thread file the hub downloaded for its session.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Download {
    pub name: String,
    pub mimetype: String,
    pub author: String,
    /// The ts of the message that attaches it.
    pub ts: String,
    pub data: crate::wire::Bytes,
}

/// The hub's answer to `pinned slack files`: the files it downloaded, and a line for each it did not.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Downloads {
    pub files: Vec<Download>,
    pub notes: Vec<String>,
}

/// Whether a Slack error means the app may not read files.
pub fn lacks_files_scope(error: &str) -> bool {
    matches!(error, "Slack: missing_scope" | "Slack: not_allowed_token_type") || error == NO_FILES_SCOPE
}

/// Images and PDFs first, then the rest, each in thread order. A session reads those itself.
fn readable_first(files: &mut [(&Post, SlackFile)]) {
    files.sort_by_key(|(_, file)| !(file.mimetype.starts_with("image/") || file.mimetype == "application/pdf"));
}

/// Downloads the files of `posts`, `FILE_COUNT` at most and `FILES_TOTAL` bytes in all, skipping
/// any over `FILE_LIMIT`. A file that fails is noted and the rest still download, until Slack says
/// the app may not read files.
pub async fn download_all(token: &str, posts: &[Post]) -> Downloads {
    download_all_within(token, posts, DOWNLOAD_DEADLINE).await
}

async fn download_all_within(token: &str, posts: &[Post], deadline: std::time::Duration) -> Downloads {
    let mut files: Vec<(&Post, SlackFile)> = posts.iter().flat_map(|post| post.files.iter().map(move |file| (post, file.clone()))).collect();
    let mut downloads = Downloads::default();
    if files.is_empty() {
        downloads.notes.push("this thread has no attached files".into());
        return downloads;
    }
    readable_first(&mut files);
    let mut total = 0u64;
    let began = std::time::Instant::now();
    for (n, (post, file)) in files.into_iter().enumerate() {
        let skip = |why: String| format!("skipped {}: {why}", file.name);
        if began.elapsed() >= deadline {
            downloads.notes.push(skip(format!("pinned stopped downloading after {} seconds", deadline.as_secs())));
            continue;
        }
        if n >= FILE_COUNT {
            downloads.notes.push(skip(format!("pinned downloads at most {FILE_COUNT} files at a time")));
            continue;
        }
        if file.size > FILE_LIMIT {
            downloads.notes.push(skip(format!("{} is over the {} limit for one file", megabytes(file.size), megabytes(FILE_LIMIT))));
            continue;
        }
        if total + file.size > FILES_TOTAL {
            downloads.notes.push(skip(format!("the files before it already make up the {} pinned downloads at a time", megabytes(FILES_TOTAL))));
            continue;
        }
        match download(token, &file.url_private).await {
            Ok(data) if data.len() as u64 > FILE_LIMIT => {
                downloads.notes.push(skip(format!("{} is over the {} limit for one file", megabytes(data.len() as u64), megabytes(FILE_LIMIT))))
            }
            Ok(data) if total + data.len() as u64 > FILES_TOTAL => {
                downloads.notes.push(skip(format!("the files before it already make up the {} pinned downloads at a time", megabytes(FILES_TOTAL))))
            }
            Ok(data) => {
                total += data.len() as u64;
                let name = if file.name.is_empty() { file.id.clone() } else { file.name.clone() };
                let download = Download { name, mimetype: file.mimetype.clone(), author: post.author.clone(), ts: post.ts.clone(), data: crate::wire::Bytes(data) };
                downloads.files.push(download);
            }
            Err(e) if lacks_files_scope(&e) => {
                downloads.notes.push(NO_FILES_SCOPE.into());
                break;
            }
            Err(e) => downloads.notes.push(skip(e)),
        }
    }
    downloads
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
}

/// Downloads a file's `url_private` with the bot token. Slack answers 403, or a 200 with its
/// HTML sign-in page, when the app may not read files.
pub async fn download(token: &str, url: &str) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(&file_url(url)).map_err(|e| format!("Slack sent a bad file address: {e}"))?;
    if !slack_host(&url) {
        return Err(format!("{} is not a Slack address", url.host_str().unwrap_or_default()));
    }
    let client = reqwest::Client::builder().timeout(file_wait()).build().map_err(|e| format!("pinned could not start the download: {e}"))?;
    let mut response = client.get(url).bearer_auth(token).send().await.map_err(request_error)?;
    let status = response.status();
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(NO_FILES_SCOPE.into());
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(RATE_LIMITED.into());
    }
    if !status.is_success() {
        return Err(format!("Slack answered {status}"));
    }
    let html = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/html"));
    if html {
        return Err(NO_FILES_SCOPE.into());
    }
    if response.content_length().is_some_and(|length| length > FILE_LIMIT) {
        return Err(format!("it is over the {} limit for one file", megabytes(FILE_LIMIT)));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        data.extend_from_slice(&chunk);
        if data.len() as u64 > FILE_LIMIT {
            return Err(format!("it is over the {} limit for one file", megabytes(FILE_LIMIT)));
        }
    }
    Ok(data)
}

/// What a file request that failed on the way says.
fn request_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        format!("Slack did not send it within {} seconds", file_wait().as_secs())
    } else if e.is_body() || e.is_decode() {
        format!("the download broke off: {e}")
    } else {
        format!("Slack unreachable: {e}")
    }
}

/// Only Slack gets the bot token.
fn slack_host(url: &reqwest::Url) -> bool {
    #[cfg(test)]
    if API_ROOT.lock().unwrap().as_deref().is_some_and(|root| url.as_str().starts_with(root)) {
        return true;
    }
    url.scheme() == "https" && url.host_str().is_some_and(|host| host == "slack.com" || host.ends_with(".slack.com"))
}

/// A test's stand-in for Slack serves files.slack.com's paths too.
fn file_url(url: &str) -> String {
    #[cfg(test)]
    if let (Some(root), Some(path)) = (API_ROOT.lock().unwrap().clone(), url.strip_prefix("https://files.slack.com/")) {
        return format!("{root}{path}");
    }
    url.to_owned()
}

/// Saves the files of the hub's `answer` to `pinned slack files` in `dir`, and returns what the
/// command prints: one line per file saved, then one per file left behind.
pub fn save_downloads(dir: Option<&std::path::Path>, answer: Result<String, String>) -> String {
    let downloads = match answer.and_then(|text| serde_json::from_str::<Downloads>(&text).map_err(|e| format!("the hub sent a bad answer: {e}"))) {
        Ok(downloads) => downloads,
        Err(e) if lacks_files_scope(&e) => return NO_FILES_SCOPE.into(),
        Err(e) => return format!("pinned could not download this thread's files: {e}"),
    };
    let mut lines = vec![];
    for file in &downloads.files {
        let saved = dir.ok_or_else(|| "this machine keeps no files".to_owned()).and_then(|dir| crate::uploads::save(dir, &file.name, &file.data.0));
        lines.push(match saved {
            Ok(path) => format!("{}  ({}, {}, from {} at {})", path.display(), file.name, file.mimetype, file.author, posted_at(&file.ts)),
            Err(e) => format!("skipped {}: {e}", file.name),
        });
    }
    lines.extend(downloads.notes);
    lines.join("\n")
}

/// A message's ts as the time it was posted, in UTC.
fn posted_at(ts: &str) -> String {
    let seconds = ts.split('.').next().and_then(|s| s.parse::<i64>().ok());
    match seconds.and_then(|s| DateTime::from_timestamp(s, 0)) {
        Some(at) => at.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => ts.to_owned(),
    }
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

    #[test]
    fn download_all_starts_no_download_once_its_deadline_has_passed() {
        let file = |name: &str| SlackFile { id: name.into(), name: name.into(), mimetype: "text/plain".into(), size: 1, url_private: format!("https://files.slack.com/files-pri/T1-{name}/{name}") };
        let post = Post { ts: "1.2".into(), author: "ana".into(), text: "logs".into(), files: vec![file("a.txt"), file("b.txt")] };
        let downloads = block_on(download_all_within("xoxb-test", &[post], std::time::Duration::ZERO));
        assert_eq!(
            downloads,
            Downloads {
                files: vec![],
                notes: vec!["skipped a.txt: pinned stopped downloading after 0 seconds".into(), "skipped b.txt: pinned stopped downloading after 0 seconds".into()],
            }
        );
        assert_eq!(DOWNLOAD_DEADLINE.as_secs(), 75);
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
                Post { ts: "1792400050.000200".into(), author: "ana".into(), text: "@pinned it broke for <@U9>".into(), files: vec![] },
                Post { ts: "1792400060.000000".into(), author: "deploybot".into(), text: "deploy failed".into(), files: vec![] },
            ]
        );
    }
}
