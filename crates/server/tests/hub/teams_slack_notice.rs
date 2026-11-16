//! A call that asks a group with a Slack user group pings it in the team's own Slack, through the
//! bot of the call's workflow, against a stand-in Slack that records every request it is sent.

use super::teams_calls::*;
use super::*;

/// A request the stand-in Slack got.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) token: String,
    channel: String,
    thread: String,
    pub(crate) text: String,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Starts the stand-in Slack on first use and points the hub's Slack calls at it through the
/// root `hub --dev` sets (`slack::use_dev_root`), for the rest of the test binary.
/// Each test names its own tokens, so they share the one server. A token holding `500` gets a
/// failing `chat.postMessage`.
pub(crate) fn slack_standin() {
    static SERVER: OnceLock<()> = OnceLock::new();
    SERVER.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        pinned_core::slack::use_dev_root(&format!("http://{}/api", listener.local_addr().unwrap())).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || slack_answer(stream));
            }
        });
    });
}

fn slack_answer(mut stream: TcpStream) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut head = vec![];
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let header = |name: &str| head.lines().find_map(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:")).then(|| l.split_once(':').unwrap().1.trim().to_owned())).unwrap_or_default();
    let mut body = vec![0u8; header("content-length").parse().unwrap_or(0)];
    let _ = stream.read_exact(&mut body);
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let token = header("authorization").trim_start_matches("Bearer ").to_owned();
    let method = head.split_whitespace().nth(1).unwrap_or_default().trim_start_matches("/api/").split('?').next().unwrap_or_default().to_owned();
    let text = |k: &str| body[k].as_str().unwrap_or_default().to_owned();
    SEEN.lock().unwrap().push(Seen { method: method.clone(), token: token.clone(), channel: text("channel"), thread: text("thread_ts"), text: text("text") });
    let (status, answer) = match method.as_str() {
        "auth.test" => {
            let user: String = token.chars().skip("xoxb-dev-".len()).filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase();
            (200, json!({ "ok": true, "user_id": format!("U{user}"), "team_id": "T0ACME001", "team": "Acme", "user": "pinned-bot", "bot_id": "B0BOT0001" }))
        }
        "bots.info" => (200, json!({ "ok": true, "bot": { "app_id": "A0APP0001" } })),
        "usergroups.list" => (200, json!({ "ok": true, "usergroups": [{ "id": "S0DESIGN01", "handle": "design-team" }, { "id": "S0OPS00001", "handle": "ops-team" }] })),
        // A `_rtr` token's channel holds one new message a person wrote, which starts its thread.
        "conversations.history" | "conversations.replies" if token.contains("_rtr") => {
            (200, json!({ "ok": true, "messages": [{ "user": "U0PERSON01", "ts": "4102444800.000100", "text": "ENG-9: the build is red" }] }))
        }
        "chat.postMessage" if token.contains("500") => (500, json!({ "ok": false, "error": "internal_error" })),
        _ => (200, json!({ "ok": true })),
    };
    let answer = answer.to_string();
    let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len());
}

/// What the stand-in got with a token starting `prefix`, oldest first.
pub(crate) fn seen(prefix: &str) -> Vec<Seen> {
    SEEN.lock().unwrap().iter().filter(|s| s.token.starts_with(prefix)).cloned().collect()
}

/// The messages posted with a token starting `prefix`.
pub(super) fn posts(prefix: &str) -> Vec<Seen> {
    seen(prefix).into_iter().filter(|s| s.method == "chat.postMessage").collect()
}

/// A hub, and a team whose Slack bot holds `token`, whose workflow "Footer" watches `channel`
/// (none keeps its Linear trigger), with Dina in `@design` (Slack user group "design-team") and
/// Omar in `@ops` ("ops-team") when `slack_groups`, and its bot asking `in_doubt`.
fn slack_team(name: &str, token: &str, channel: Option<&str>, slack_groups: bool, in_doubt: Value) -> (TestHub, TeamBox, Person, Person) {
    slack_standin();
    let hub = TestHub::start_with_team_policy(name, fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let team = tb.team.to_string();
    let mut on_team = |call: Value| tb.owner.client.hub(json!({ "cmd": "team", "team": team, "op": { "op": "hub", "call": call } }));
    if !token.is_empty() {
        on_team(json!({ "cmd": "connect_slack", "token": token })).unwrap();
    }
    if let Some(channel) = channel {
        let repo = temp(&format!("team-repo-{}", hub.database));
        tb.owner.client.hub(json!({ "cmd": "team", "team": team, "op": { "op": "save_bot", "bot": { "workflow": tb.workflow }, "connector": {
            "id": tb.workflow, "name": "Footer", "enabled": false, "trigger": { "kind": "slack-mention", "channel": channel },
            "folder": repo, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
        } } }))
        .unwrap();
        // `tb.bot` writes the bot row this test needs.
        sql(&hub, &format!("delete from team_bots where team = '{}'", tb.team));
    }
    let dina = tb.member(&hub, "dina@example.test", &["design"]);
    let omar = tb.member(&hub, "omar@example.test", &["ops"]);
    if slack_groups {
        sql(&hub, &format!(
            "update team_groups set slack_group = 'design-team' where team = '{0}' and handle = 'design';
             update team_groups set slack_group = 'ops-team' where team = '{0}' and handle = 'ops'",
            tb.team
        ));
    }
    tb.bot(&hub, in_doubt, None);
    tb.run(&hub);
    (hub, tb, dina, omar)
}

/// Waits until the stand-in got `count` posts with a token starting `prefix`.
fn wait_for_posts(prefix: &str, count: usize) -> Vec<Seen> {
    wait_for(&format!("{count} Slack post(s) from {prefix}"), || posts(prefix).len() >= count);
    posts(prefix)
}

/// Long enough for a notice that is sent in the background to have been sent.
fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

#[test]
fn a_call_to_a_group_with_a_slack_group_posts_one_notice_in_the_workflows_channel_with_the_teams_bot() {
    let (hub, tb, _dina, _omar) = slack_team("teams_slack_notice_one", "xoxb-dev-notice-one", Some("C0FOOTER01"), true, design());
    let asked = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap();
    assert_eq!(asked["asked"], json!(["@design"]));
    let sent = wait_for_posts("xoxb-dev-notice-one", 1);
    settle();
    let sent_now = posts("xoxb-dev-notice-one");
    assert_eq!(sent_now.len(), 1, "exactly one message: {sent_now:?}");
    assert_eq!(
        (sent[0].channel.as_str(), sent[0].token.as_str(), sent[0].thread.as_str(), sent[0].text.as_str()),
        ("C0FOOTER01", "xoxb-dev-notice-one", "", "Footer needs <!subteam^S0DESIGN01>: Red or blue footer?. Answer in pinned.")
    );
}

#[test]
fn a_question_goes_to_slack_without_secrets_on_one_line_and_cut_at_300_characters() {
    let (hub, tb, mut dina, _omar) = slack_team("teams_slack_notice_text", "xoxb-dev-notice-text", Some("C0FOOTER01"), true, design());
    let secret = format!("sk-ant-oat01-{}", "A1b2C3d4E5".repeat(4));
    tb.ask(&hub, &format!("Deploy with {secret}\nto prod?"), &[]).unwrap();
    let first = wait_for_posts("xoxb-dev-notice-text", 1);
    assert_eq!(first[0].text, "Footer needs <!subteam^S0DESIGN01>: Deploy with [hidden] to prod?. Answer in pinned.");
    assert!(!first[0].text.contains("sk-ant"), "the key never reaches Slack");

    // A second question on the same run: the first is answered, so the run works again.
    let call = rows(&hub, "select id::text from team_calls")[0][0].clone().unwrap();
    op(tb.team, &mut dina.client, json!({ "op": "answer_call", "id": call, "answer": choice("", "ok") })).unwrap();
    tb.ask(&hub, &"x".repeat(1000), &[]).unwrap();
    let both = wait_for_posts("xoxb-dev-notice-text", 2);
    assert_eq!(both[1].text, format!("Footer needs <!subteam^S0DESIGN01>: {}. Answer in pinned.", "x".repeat(300)));
}

#[test]
fn a_mention_in_the_question_is_escaped_and_pings_nobody() {
    let (hub, tb, _dina, _omar) = slack_team("teams_slack_notice_escaped", "xoxb-dev-notice-escaped", Some("C0FOOTER01"), true, design());
    tb.ask(&hub, "Ship it? <!channel> <!here> <@U123> <!subteam^S1> & a>b", &[]).unwrap();
    let sent = wait_for_posts("xoxb-dev-notice-escaped", 1);
    assert_eq!(
        sent[0].text,
        "Footer needs <!subteam^S0DESIGN01>: Ship it? &lt;!channel&gt; &lt;!here&gt; &lt;@U123&gt; &lt;!subteam^S1&gt; &amp; a&gt;b. Answer in pinned."
    );
}

#[test]
fn a_call_to_a_person_or_the_owner_posts_nothing() {
    let (hub, mut tb, dina, _omar) = slack_team("teams_slack_notice_person", "xoxb-dev-notice-person", Some("C0FOOTER01"), true, json!({ "kind": "owner" }));
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    settle();
    assert_eq!(posts("xoxb-dev-notice-person").len(), 0, "the owner is asked in the app only");
    // The same run, now asked by person: answered first, then a person's call.
    let call = rows(&hub, "select id::text from team_calls")[0][0].clone().unwrap();
    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": choice("", "ok") })).unwrap();
    sql(&hub, &format!("update team_bots set calls = '{}' where team = '{}'", json!({ "in_doubt": { "kind": "person", "account": dina.id.to_string() } }), tb.team));
    tb.ask(&hub, "And the header?", &[]).unwrap();
    settle();
    assert_eq!(posts("xoxb-dev-notice-person").len(), 0, "a person is asked in the app only");
}

#[test]
fn a_group_without_a_slack_group_or_a_workflow_without_a_slack_channel_posts_nothing() {
    let (hub, tb, _dina, _omar) = slack_team("teams_slack_notice_none", "xoxb-dev-notice-none", Some("C0FOOTER01"), false, design());
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    settle();
    assert_eq!(posts("xoxb-dev-notice-none").len(), 0, "@design has no Slack user group");

    let (hub, tb, _dina, _omar) = slack_team("teams_slack_notice_nochannel", "xoxb-dev-notice-nochannel", None, true, design());
    assert_eq!(tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["asked"], json!(["@design"]));
    settle();
    assert_eq!(posts("xoxb-dev-notice-nochannel").len(), 0, "a workflow that watches no Slack channel has none to post in");
}

#[test]
fn a_members_own_slack_bot_is_never_used() {
    let (hub, mut tb, _dina, _omar) = slack_team("teams_slack_notice_member", "xoxb-dev-notice-team", Some("C0FOOTER01"), true, design());
    // Olga's own account has its own Slack bot and a workflow in the same channel.
    tb.owner.client.hub(json!({ "cmd": "connect_slack", "token": "xoxb-dev-notice-mine" })).unwrap();
    tb.owner.client.hub(json!({ "cmd": "save_connector", "connector": {
        "name": "Mine", "enabled": false, "trigger": { "kind": "slack-mention", "channel": "C0FOOTER01" },
        "folder": temp(&format!("own-repo-{}", hub.database)), "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "",
    } })).unwrap();
    let before = seen("xoxb-dev-notice-mine").len();
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    let sent = wait_for_posts("xoxb-dev-notice-team", 1);
    settle();
    assert_eq!(sent[0].text, "Footer needs <!subteam^S0DESIGN01>: Red or blue footer?. Answer in pinned.");
    assert_eq!(seen("xoxb-dev-notice-mine").len(), before, "Slack never heard from Olga's own bot for the call");
    assert_eq!(posts("xoxb-dev-notice-mine").len(), 0);
}

#[test]
fn a_team_without_its_own_slack_bot_posts_nothing_even_when_a_member_has_one() {
    let (hub, mut tb, _dina, _omar) = slack_team("teams_slack_notice_nobot", "", Some("C0FOOTER01"), true, design());
    tb.owner.client.hub(json!({ "cmd": "connect_slack", "token": "xoxb-dev-notice-lonely" })).unwrap();
    tb.owner.client.hub(json!({ "cmd": "save_connector", "connector": {
        "name": "Mine", "enabled": false, "trigger": { "kind": "slack-mention", "channel": "C0FOOTER01" },
        "folder": temp(&format!("own-repo-{}", hub.database)), "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "",
    } })).unwrap();
    let before = seen("xoxb-dev-notice-lonely").len();
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    settle();
    assert_eq!(seen("xoxb-dev-notice-lonely").len(), before);
    assert_eq!(posts("xoxb-dev-notice-lonely").len(), 0);
}

#[test]
fn a_slack_failure_does_not_fail_the_call() {
    let (hub, tb, mut dina, _omar) = slack_team("teams_slack_notice_500", "xoxb-dev-notice-500", Some("C0FOOTER01"), true, design());
    let asked = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap();
    let call = asked["call"].as_str().unwrap().to_owned();
    assert_eq!(asked, json!({ "call": call, "asked": ["@design"] }));
    wait_for_posts("xoxb-dev-notice-500", 1);
    let listed = calls_of(tb.team, &mut dina.client);
    assert_eq!((listed.len(), &listed[0]["id"], &listed[0]["answer"]), (1, &json!(call), &Value::Null));
    let answered = op(tb.team, &mut dina.client, json!({ "op": "answer_call", "id": call, "answer": choice("red", "") })).unwrap();
    assert_eq!(answered["answer"]["choice"], "red");
    assert_eq!(tb.run_state(&hub, rows(&hub, "select id::text from team_runs")[0][0].as_ref().unwrap().parse().unwrap()).0, "working");
}

#[test]
fn passing_a_call_to_another_group_posts_once_more_for_the_new_call() {
    let (hub, tb, mut dina, _omar) = slack_team("teams_slack_notice_pass", "xoxb-dev-notice-pass", Some("C0FOOTER01"), true, design());
    let first = tb.ask(&hub, "Deploy now?", &[]).unwrap()["call"].clone();
    wait_for_posts("xoxb-dev-notice-pass", 1);
    let pass = json!({ "op": "answer_call", "id": first, "answer": { "kind": { "kind": "pass_to", "handle": "@ops" } } });
    op(tb.team, &mut dina.client, pass).unwrap();
    let sent = wait_for_posts("xoxb-dev-notice-pass", 2);
    settle();
    let sent_now = posts("xoxb-dev-notice-pass");
    assert_eq!(sent_now.len(), 2, "one per call: {sent_now:?}");
    assert_eq!(sent[0].text, "Footer needs <!subteam^S0DESIGN01>: Deploy now?. Answer in pinned.");
    assert_eq!((sent[1].channel.as_str(), sent[1].text.as_str()), ("C0FOOTER01", "Footer needs <!subteam^S0OPS00001>: Deploy now?. Answer in pinned."));
}
