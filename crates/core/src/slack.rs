use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

const API: &str = "https://slack.com/api/";
/// How far back a channel's history is read to find threads that got a new reply. A thread
/// older than this that gets a new mention or reply is missed.
const THREAD_LOOKBACK: chrono::Duration = chrono::Duration::days(7);
const PAGE: &str = "200";
/// The error a call fails with once Slack rate limits pinned.
pub const RATE_LIMITED: &str = "Slack: ratelimited";

#[derive(Serialize, Deserialize, Clone, Debug)]
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
    pub at: DateTime<Utc>,
}

/// New messages in a thread whose session is already answering it.
#[derive(Debug, PartialEq)]
pub struct Replies {
    pub thread: String,
    /// The newest message's timestamp, the bot's own included, which the thread's cursor moves to.
    pub newest: String,
    /// The messages people wrote, oldest first.
    pub posts: Vec<Post>,
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

async fn call(token: &str, method: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    let url = reqwest::Url::parse_with_params(&format!("{API}{method}"), params).map_err(|e| e.to_string())?;
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
        .post(format!("{API}{method}"))
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

/// Messages after `since` that mention `bot`, other than the bot's own.
fn mentions_in(messages: &Value, bot: &str, since: DateTime<Utc>) -> Vec<Mention> {
    let tag = format!("<@{bot}>");
    messages
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["user"].as_str() != Some(bot) && m["text"].as_str().is_some_and(|t| t.contains(&tag)))
        .filter_map(|m| {
            let ts = m["ts"].as_str()?;
            let at = DateTime::from_timestamp_micros(micros(ts)?)?;
            let thread = m["thread_ts"].as_str().unwrap_or(ts).to_owned();
            (at > since).then_some(Mention { thread, at })
        })
        .collect()
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

/// Threads of a history page in `cursors`, by thread ts, whose last reply is past their cursor.
fn followed_with_replies<'a>(history: &Value, cursors: &'a BTreeMap<String, String>) -> Vec<(String, &'a str)> {
    history
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let (thread, cursor) = cursors.get_key_value(m["ts"].as_str()?)?;
            (micros(m["latest_reply"].as_str()?)? > micros(cursor)?).then(|| (thread.clone(), cursor.as_str()))
        })
        .collect()
}

/// The newest timestamp after `cursor` in a thread, and the messages after it that people wrote:
/// neither the bot's own nor another bot's.
fn after_cursor(messages: &Value, bot: &str, cursor: &str) -> Option<(String, Vec<Value>)> {
    let cursor = micros(cursor)?;
    let new: Vec<&Value> = messages
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["ts"].as_str().and_then(micros).is_some_and(|at| at > cursor))
        .collect();
    let newest = new.iter().filter_map(|m| m["ts"].as_str()).max_by_key(|ts| micros(ts))?.to_owned();
    let people = new
        .into_iter()
        .filter(|m| m["user"].as_str() != Some(bot) && m["subtype"].as_str() != Some("bot_message"))
        .cloned()
        .collect();
    Some((newest, people))
}

/// Mentions of `bot` in `channel` after `since`, oldest first, and new messages in the threads
/// `cursors` follows, keyed by thread ts. Takes one history call, plus one replies call per
/// thread that got a reply since, because Slack rate limits both.
pub async fn activity(
    token: &str,
    channel: &str,
    bot: &str,
    since: DateTime<Utc>,
    cursors: &BTreeMap<String, String>,
) -> Result<Activity, String> {
    let oldest = ts(since - THREAD_LOOKBACK);
    let history = call(token, "conversations.history", &[("channel", channel), ("oldest", &oldest), ("limit", PAGE)]).await?;
    let messages = &history["messages"];
    let mut mentions = mentions_in(messages, bot, since);
    // A followed thread already has its session, so a mention in it is just another reply.
    for thread in threads_replied_after(messages, since).into_iter().filter(|t| !cursors.contains_key(t)) {
        let replies = call(token, "conversations.replies", &[("channel", channel), ("ts", &thread), ("limit", PAGE)]).await?;
        mentions.extend(mentions_in(&replies["messages"], bot, since));
    }
    mentions.sort_by_key(|m| m.at);
    mentions.dedup();
    let mut replies = vec![];
    for (thread, cursor) in followed_with_replies(messages, cursors) {
        let body = call(token, "conversations.replies", &[("channel", channel), ("ts", &thread), ("oldest", cursor), ("limit", PAGE)]).await?;
        let Some((newest, people)) = after_cursor(&body["messages"], bot, cursor) else { continue };
        let people = Value::Array(people);
        let names = names(token, &people).await?;
        replies.push(Replies { thread, newest, posts: posts(&people, &names) });
    }
    Ok(Activity { mentions, replies })
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
            [Mention { thread: "1792400050.000200".into(), at: DateTime::from_timestamp(1792400050, 200_000).unwrap() }]
        );
        assert_eq!(threads_replied_after(&history, since), ["1792400050.000200", "1792390000.000100"]);
        assert_eq!(
            mentions_in(&replies, "UBOT", since),
            [Mention { thread: "1792390000.000100".into(), at: DateTime::from_timestamp(1792400090, 0).unwrap() }]
        );
    }

    #[test]
    fn followed_threads_get_a_replies_call_only_past_their_cursor() {
        let (history, _) = fixture();
        let cursors = BTreeMap::from([
            ("1792400050.000200".to_owned(), "1792400060.000100".to_owned()),
            ("1792390000.000100".to_owned(), "1792400080.000000".to_owned()),
            ("1792370000.000000".to_owned(), "1792370000.000000".to_owned()),
        ]);
        assert_eq!(followed_with_replies(&history, &cursors), [("1792390000.000100".to_owned(), "1792400080.000000")]);
        assert_eq!(followed_with_replies(&history, &BTreeMap::new()), []);
    }

    #[test]
    fn follow_ups_are_what_people_wrote_after_the_cursor() {
        let replies = json!([
            { "ts": "1792390000.000100", "user": "U2", "text": "deploy notes" },
            { "ts": "1792400080.000000", "user": "UBOT", "text": "the cause is X; file it?" },
            { "ts": "1792400090.000000", "user": "U3", "text": "yes, create the task" },
            { "ts": "1792400095.000000", "subtype": "bot_message", "bot_id": "B9", "text": "deploy finished" },
            { "ts": "1792400099.000000", "user": "UBOT", "text": "on it" },
        ]);
        let (newest, people) = after_cursor(&replies, "UBOT", "1792400080.000000").unwrap();
        assert_eq!(newest, "1792400099.000000");
        assert_eq!(people, [json!({ "ts": "1792400090.000000", "user": "U3", "text": "yes, create the task" })]);
        let (newest, people) = after_cursor(&replies, "UBOT", "1792400090.000000").unwrap();
        assert_eq!((newest.as_str(), people.len()), ("1792400099.000000", 0));
        assert_eq!(after_cursor(&replies, "UBOT", "1792400099.000000"), None);
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
}
