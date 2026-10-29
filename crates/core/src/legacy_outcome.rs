use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// What one trigger input makes today's hub do. Later graph runs have to equal this.
#[derive(Debug)]
enum Observation {
    Open { connector: String, subject: Subject, prompt_contains: Vec<String>, args_contain: Vec<String> },
    Stop { connector: String, why: String },
    Page { connector: String, group: String, thread: String },
    Route { from: String, to: String },
    Claim { connector: String, key: String },
    StagePicked { issue: String },
    Deliver { connector: String, thread: String, messages: Vec<String> },
}

/// The driver stores the prompt and the claude args the run opened with.
/// A row lists fragments of those strings, and either side may be the fragments.
fn same_or_covers(left: &[String], right: &[String]) -> bool {
    left == right || covers(left, right) || covers(right, left)
}

fn covers(haystack: &[String], needles: &[String]) -> bool {
    needles.iter().all(|needle| haystack.iter().any(|hay| hay.contains(needle.as_str())))
}

impl PartialEq for Observation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Open { connector: left_connector, subject: left_subject, prompt_contains: left_prompt, args_contain: left_args },
                Self::Open { connector: right_connector, subject: right_subject, prompt_contains: right_prompt, args_contain: right_args },
            ) => left_connector == right_connector && left_subject == right_subject && same_or_covers(left_prompt, right_prompt) && same_or_covers(left_args, right_args),
            (Self::Stop { connector: left, why: left_why }, Self::Stop { connector: right, why: right_why }) => left == right && left_why == right_why,
            (Self::Page { connector: left, group: left_group, thread: left_thread }, Self::Page { connector: right, group: right_group, thread: right_thread }) => {
                left == right && left_group == right_group && left_thread == right_thread
            }
            (Self::Route { from: left_from, to: left_to }, Self::Route { from: right_from, to: right_to }) => left_from == right_from && left_to == right_to,
            (Self::Claim { connector: left, key: left_key }, Self::Claim { connector: right, key: right_key }) => left == right && left_key == right_key,
            (Self::StagePicked { issue: left }, Self::StagePicked { issue: right }) => left == right,
            (
                Self::Deliver { connector: left, thread: left_thread, messages: left_messages },
                Self::Deliver { connector: right, thread: right_thread, messages: right_messages },
            ) => left == right && left_thread == right_thread && left_messages == right_messages,
            _ => false,
        }
    }
}

#[derive(Clone, Copy)]
enum Fixture {
    Issue { connector: &'static str },
    Mention { connector: &'static str },
    Message { connector: &'static str },
    Replies { connector: &'static str, author: &'static str, text: &'static str, ts: &'static str },
}

impl Fixture {
    fn source(self) -> &'static str {
        match self {
            Self::Issue { connector } | Self::Mention { connector } | Self::Message { connector } | Self::Replies { connector, .. } => connector,
        }
    }
}

#[derive(Clone, Copy)]
enum ScriptedModel {
    None,
    Ask(&'static str),
    Decide { index: usize, probabilities: &'static [f64] },
}

const THREAD_TS: &str = "1792400050.000200";
const CHANNEL: &str = "C0123ABCD";
/// A mention run reacts after the tab takes it. With no Slack token that reaction stops here, after the open.
const SLACK_DISCONNECTED: &str = "Slack is not connected: connect it in pinned's Connectors settings";

struct Classified {
    found: Found,
    stop: Option<String>,
    group: Option<String>,
}

fn legacy_outcome(connectors: &[Connector], fixture: Fixture, scripted: ScriptedModel) -> Vec<Observation> {
    static N: AtomicU64 = AtomicU64::new(0);
    let mut owned = connectors.to_vec();
    if let Fixture::Replies { connector, .. } = fixture {
        let workflow = owned.iter_mut().find(|c| c.id == connector).expect("connector");
        let key = THREAD.to_owned();
        if !workflow.handled.contains(&key) {
            workflow.handled.push(key.clone());
        }
        workflow.followed.entry(key).or_insert(Followed { cursor: "1792400060.000000".into(), seen: None, replied: false });
    }
    let source_id = fixture.source();
    let source = owned.iter().find(|c| c.id == source_id).expect("connector").clone();
    let classified = classify(&source, fixture, scripted);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let (hub, dir) = stored(&format!("legacy-{n}"), owned);
    let _tmp = Tmp(dir.clone());
    let _reply = match scripted {
        ScriptedModel::Ask(json) => {
            std::fs::write(dir.join("vercel-key"), "vck_testa1b2").expect("vercel key");
            Some(TestReply::set(json.to_owned()))
        }
        ScriptedModel::None | ScriptedModel::Decide { .. } => None,
    };

    let (settled, claims) = {
        let mut store = hub.store.lock().unwrap();
        let before: Vec<(String, Vec<String>)> = store.connectors.iter().map(|c| (c.id.clone(), c.handled.clone())).collect();
        let settled = settle(&mut store, vec![(source_id, classified.found)]);
        let claims = new_claims(&before, &store.connectors);
        (settled, claims)
    };
    assert!(settled.errors.is_empty(), "{}", settled.errors.join("; "));
    assert!(settled.pages.len() <= 1, "{}", settled.pages.len());

    let mut observations = claims;
    if let Some(to) = route_to(source_id, &settled.work) {
        observations.push(Observation::Route { from: source_id.into(), to });
    }
    if let Some(page) = settled.pages.first() {
        let group = classified.group.expect("page without a group");
        observations.push(Observation::Page { connector: source_id.into(), group, thread: page.thread.clone() });
    }

    let had_work = !settled.work.is_empty();
    let seen = Mutex::new(Vec::new());
    let took = AtomicBool::new(false);
    let open = |work: Work| {
        seen.lock().unwrap().push(observe(work));
        took.store(true, Ordering::Relaxed);
        async { Ok(()) }
    };
    for work in settled.work {
        if let Err(error) = block_on(hub.hand_over(work, &open)) {
            if !(took.load(Ordering::Relaxed) && error == SLACK_DISCONNECTED) {
                panic!("{error}");
            }
        }
    }
    if took.load(Ordering::Relaxed) {
        observations.extend(seen.into_inner().unwrap());
    } else if let ScriptedModel::Ask(json) = scripted {
        observations.push(Observation::Stop { connector: source_id.into(), why: ask_why(&source, json) });
    } else if had_work {
        panic!("hand_over opened nothing");
    } else if let Some(why) = classified.stop {
        observations.push(Observation::Stop { connector: source_id.into(), why });
    }
    observations
}

fn observe(work: Work) -> Observation {
    match work {
        Work::Run(run) => {
            let prompt = run.args.last().cloned().unwrap_or_default();
            let args_contain = if run.args.is_empty() { Vec::new() } else { run.args[..run.args.len() - 1].to_vec() };
            Observation::Open { connector: run.connector, subject: run.subject, prompt_contains: vec![prompt], args_contain }
        }
        Work::Delivery(delivery) => Observation::Deliver { connector: delivery.connector, thread: delivery.thread, messages: delivery.messages },
    }
}

fn route_to(source: &str, work: &[Work]) -> Option<String> {
    for item in work {
        if let Work::Run(run) = item {
            if run.connector != source {
                return Some(run.connector.clone());
            }
        }
    }
    None
}

fn new_claims(before: &[(String, Vec<String>)], after: &[Connector]) -> Vec<Observation> {
    let mut claims = Vec::new();
    for connector in after {
        let previous = before.iter().find(|(id, _)| id == &connector.id).map(|(_, handled)| handled.as_slice()).unwrap_or(&[]);
        for key in &connector.handled {
            if !previous.contains(key) {
                claims.push(Observation::Claim { connector: connector.id.clone(), key: key.clone() });
            }
        }
    }
    claims
}

fn classify(connector: &Connector, fixture: Fixture, scripted: ScriptedModel) -> Classified {
    match fixture {
        Fixture::Issue { .. } => Classified { found: Found::Issue(issue()), stop: None, group: None },
        Fixture::Mention { .. } => Classified { found: mentioned(), stop: None, group: None },
        Fixture::Replies { author, text, ts, .. } => Classified {
            found: Found::Replies {
                channel: CHANNEL.into(),
                replies: slack::Replies {
                    thread: THREAD_TS.into(),
                    newest: ts.into(),
                    posts: vec![slack::Post { ts: ts.into(), author: author.into(), text: text.into() }],
                    mentioned: vec![],
                },
            },
            stop: None,
            group: None,
        },
        Fixture::Message { .. } => classify_message(connector, scripted),
    }
}

/// Same branch as `Connectors::scan` after `Decide::pick`. `slack::thread` and `slack::group_id` are the network calls.
/// The thread is the fixture's posts, and the subteam id is the one the settle tests already use.
fn classify_message(connector: &Connector, scripted: ScriptedModel) -> Classified {
    let ScriptedModel::Decide { index, probabilities } = scripted else { panic!("a message fixture needs a scripted decision") };
    let Trigger::SlackMessage { decide, .. } = &connector.trigger else { panic!("message fixture on a connector that does not sort messages") };
    let pick = decide.pick(&vercel::Chosen { index, probabilities: probabilities.to_vec() });
    let channel = CHANNEL.to_owned();
    let thread = THREAD_TS.to_owned();
    let Some(index) = pick.category() else {
        return Classified { found: Found::Stopped { channel, thread, page: None }, stop: Some("nowhere".into()), group: None };
    };
    let category = &decide.categories[index];
    let page = category.group().map(|handle| Paging { ping: page_text("S0123ABC", category), handle: handle.to_owned() });
    let group = page.as_ref().map(|paging| paging.handle.clone());
    match &category.workflow {
        None => Classified { found: Found::Stopped { channel, thread, page }, stop: Some(category.name.clone()), group },
        Some(workflow) => Classified {
            found: Found::Routed {
                posts: posts().into(),
                channel,
                channel_name: "support".into(),
                thread,
                category: category.name.clone(),
                workflow: workflow.clone(),
                note: pick.note(&decide.categories, &connector.name),
                page,
            },
            stop: None,
            group,
        },
    }
}

fn ask_why(connector: &Connector, json: &str) -> String {
    let ask = connector.ask.as_ref().expect("ask");
    let value: serde_json::Value = serde_json::from_str(json).expect("ask json");
    let answers: Vec<(String, String)> = ask
        .questions
        .iter()
        .map(|question| {
            let raw = value[&question.name].as_str().expect("answer");
            let answer = question.choices.iter().find(|choice| choice.eq_ignore_ascii_case(raw)).cloned().unwrap_or_else(|| raw.to_owned());
            (question.name.clone(), answer)
        })
        .collect();
    let (name, answer) = stops(ask, &answers).expect("stop_on");
    format!("{name}={answer}")
}

struct Tmp(std::path::PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn linear_columns() -> Vec<Connector> {
    vec![Connector { columns: Columns { picked: String::new(), coding: "In Dev".into(), review: "Ready to Review".into() }, ..linear_connector() }]
}

fn slack_fix(fix: Fix) -> Vec<Connector> {
    vec![Connector { fix, ..fixing_connector() }]
}

fn slack_ask() -> Vec<Connector> {
    vec![Connector { ask: Some(classifying()), ..slack_connector() }]
}

fn message_workflows() -> Vec<Connector> {
    vec![router(), slack_connector(), called()]
}

fn fallback_router() -> Vec<Connector> {
    let mut workflows = message_workflows();
    let Trigger::SlackMessage { channel, decide } = workflows[0].trigger.clone() else { unreachable!() };
    workflows[0].trigger = Trigger::SlackMessage { channel, decide: Decide { fallback: Some("doubt".into()), ..decide } };
    workflows
}

fn nowhere_router() -> Vec<Connector> {
    let mut workflow = router();
    let Trigger::SlackMessage { channel, mut decide } = workflow.trigger else { unreachable!() };
    decide.categories = vec![category("escalation", "Production is down", None, Some("prod-engg"))];
    decide.fallback = None;
    workflow.trigger = Trigger::SlackMessage { channel, decide };
    vec![workflow]
}

fn thread() -> Subject {
    thread_subject()
}

#[test]
fn legacy_linear_label_with_columns_opens() {
    let connectors = linear_columns();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Issue { connector: "c1" }, ScriptedModel::None),
        vec![
            Observation::Claim { connector: "c1".into(), key: "uuid".into() },
            Observation::Open {
                connector: "c1".into(),
                subject: Subject::LinearIssue { issue: "uuid".into() },
                prompt_contains: vec!["'/bin/pinned' stage coding".into(), "'/bin/pinned' stage review".into(), "ENG-7: Broken button".into()],
                args_contain: vec!["--model".into(), "sonnet".into(), "--effort".into(), "xhigh".into(), "--worktree=eng-7".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_mention_opens() {
    let connectors = vec![slack_connector()];
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Mention { connector: "c2" }, ScriptedModel::None),
        vec![
            Observation::Claim { connector: "c2".into(), key: THREAD.into() },
            Observation::Open {
                connector: "c2".into(),
                subject: thread(),
                prompt_contains: vec!["only triages".into(), "export 500s".into()],
                args_contain: vec!["--effort".into(), "high".into(), "--permission-mode".into(), "plan".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_mention_fix_ask_opens() {
    let connectors = slack_fix(Fix::Ask);
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Mention { connector: "c2" }, ScriptedModel::None),
        vec![
            Observation::Claim { connector: "c2".into(), key: THREAD.into() },
            Observation::Open {
                connector: "c2".into(),
                subject: thread(),
                prompt_contains: vec!["ask in the thread whether to file a Linear issue".into(), "'/bin/pinned' stage coding".into(), "'/bin/pinned' stage review".into()],
                args_contain: vec!["--permission-mode".into(), "auto".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_mention_fix_auto_opens() {
    let connectors = slack_fix(Fix::Auto);
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Mention { connector: "c2" }, ScriptedModel::None),
        vec![
            Observation::Claim { connector: "c2".into(), key: THREAD.into() },
            Observation::Open {
                connector: "c2".into(),
                subject: thread(),
                prompt_contains: vec!["obvious, because you reproduced it".into(), "'/bin/pinned' stage coding".into(), "'/bin/pinned' stage review".into()],
                args_contain: vec!["--permission-mode".into(), "auto".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_mention_ask_stops() {
    let connectors = slack_ask();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Mention { connector: "c2" }, ScriptedModel::Ask(r#"{"kind":"bug","agent":"No"}"#)),
        vec![
            Observation::Claim { connector: "c2".into(), key: THREAD.into() },
            Observation::Stop { connector: "c2".into(), why: "agent=no".into() },
        ]
    );
}

#[test]
fn legacy_slack_mention_ask_opens_with_answers() {
    let connectors = slack_ask();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Mention { connector: "c2" }, ScriptedModel::Ask(r#"{"kind":"bug","agent":"yes"}"#)),
        vec![
            Observation::Claim { connector: "c2".into(), key: THREAD.into() },
            Observation::Open {
                connector: "c2".into(),
                subject: thread(),
                prompt_contains: vec!["read the thread and answered".into(), "kind: bug".into(), "agent: yes".into()],
                args_contain: vec!["--permission-mode".into(), "plan".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_message_routes_to_mention_workflow() {
    let connectors = message_workflows();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Message { connector: "c5" }, ScriptedModel::Decide { index: 0, probabilities: &[0.91, 0.03, 0.03, 0.03] }),
        vec![
            Observation::Claim { connector: "c2".into(), key: THREAD.into() },
            Observation::Route { from: "c5".into(), to: "c2".into() },
            Observation::Open {
                connector: "c2".into(),
                subject: thread(),
                prompt_contains: vec!["put the first message in the category \"error\"".into()],
                args_contain: vec!["--permission-mode".into(), "plan".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_message_routes_to_called_workflow() {
    let connectors = message_workflows();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Message { connector: "c5" }, ScriptedModel::Decide { index: 1, probabilities: &[0.02, 0.96, 0.01, 0.01] }),
        vec![
            Observation::Claim { connector: "c6".into(), key: THREAD.into() },
            Observation::Route { from: "c5".into(), to: "c6".into() },
            Observation::Open {
                connector: "c6".into(),
                subject: thread(),
                prompt_contains: vec!["put the first message in the category \"doubt\"".into()],
                args_contain: vec!["--effort".into(), "high".into(), "--permission-mode".into(), "plan".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_message_pages_group_and_stops() {
    let connectors = message_workflows();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Message { connector: "c5" }, ScriptedModel::Decide { index: 2, probabilities: &[0.05, 0.05, 0.85, 0.05] }),
        vec![
            Observation::Claim { connector: "c5".into(), key: THREAD.into() },
            Observation::Page { connector: "c5".into(), group: "prod-engg".into(), thread: THREAD_TS.into() },
            Observation::Stop { connector: "c5".into(), why: "escalation".into() },
        ]
    );
}

#[test]
fn legacy_slack_message_stops() {
    let connectors = message_workflows();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Message { connector: "c5" }, ScriptedModel::Decide { index: 3, probabilities: &[0.05, 0.05, 0.05, 0.85] }),
        vec![
            Observation::Claim { connector: "c5".into(), key: THREAD.into() },
            Observation::Stop { connector: "c5".into(), why: "ignore".into() },
        ]
    );
}

#[test]
fn legacy_slack_message_fallback_below_0_6() {
    let connectors = fallback_router();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Message { connector: "c5" }, ScriptedModel::Decide { index: 0, probabilities: &[0.55, 0.30, 0.10, 0.05] }),
        vec![
            Observation::Claim { connector: "c6".into(), key: THREAD.into() },
            Observation::Route { from: "c5".into(), to: "c6".into() },
            Observation::Open {
                connector: "c6".into(),
                subject: thread(),
                prompt_contains: vec!["it guessed \"error\" at 55%".into(), "handed this thread over as \"doubt\"".into()],
                args_contain: vec!["--permission-mode".into(), "plan".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_message_paging_floor_downgrades() {
    let connectors = message_workflows();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Message { connector: "c5" }, ScriptedModel::Decide { index: 2, probabilities: &[0.03, 0.10, 0.84, 0.03] }),
        vec![
            Observation::Claim { connector: "c6".into(), key: THREAD.into() },
            Observation::Route { from: "c5".into(), to: "c6".into() },
            Observation::Open {
                connector: "c6".into(),
                subject: thread(),
                prompt_contains: vec!["put the first message in the category \"doubt\"".into()],
                args_contain: vec!["--permission-mode".into(), "plan".into()],
            },
        ]
    );
}

#[test]
fn legacy_slack_message_nowhere() {
    let connectors = nowhere_router();
    assert_eq!(
        legacy_outcome(&connectors, Fixture::Message { connector: "c5" }, ScriptedModel::Decide { index: 0, probabilities: &[0.6] }),
        vec![
            Observation::Claim { connector: "c5".into(), key: THREAD.into() },
            Observation::Stop { connector: "c5".into(), why: "nowhere".into() },
        ]
    );
}

#[test]
fn legacy_slack_delivery_to_open_thread() {
    let connectors = vec![slack_connector()];
    assert_eq!(
        legacy_outcome(
            &connectors,
            Fixture::Replies { connector: "c2", author: "Ana Lima", text: "yes, file it", ts: "1792400090.000000" },
            ScriptedModel::None,
        ),
        vec![Observation::Deliver {
            connector: "c2".into(),
            thread: THREAD_TS.into(),
            messages: vec!["[Slack reply from Ana Lima in the thread]\nyes, file it".into()],
        }]
    );
}

#[test]
#[ignore = "Connectors::stage calls linear::move_issue when columns.picked is set. No test double, and a missing Linear key returns before that call."]
fn legacy_linear_label_stage_picked_moves_the_issue() {
    let _moved = Observation::StagePicked { issue: "uuid".into() };
    panic!("skipped: linear::move_issue has no test double");
}

#[test]
#[ignore = "A failed delivery calls Connectors::fallback, which reads the thread with slack::thread. No test double. A missing Slack token returns before the new run."]
fn legacy_slack_dead_tab_opens_a_new_conversation() {
    panic!("skipped: slack::thread has no test double");
}
