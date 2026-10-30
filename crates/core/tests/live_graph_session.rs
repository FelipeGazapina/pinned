//! Ignored. A real Machine, the real `claude` CLI, and the hub's graph loop.
//!
//! ```text
//! cargo test -p pinned-core --test live_graph_session -- --ignored --nocapture
//! ```
//!
//! Build `pinned-server` first. Hooks call that binary. Set `PINNED_SERVER` when it is not
//! `$CARGO_TARGET_DIR/debug/pinned-server`.

use pinned_core::agent::{AgentStatus, Subject};
use pinned_core::connectors::{
    Connector, Connectors, Effort, FileStorage, Fix, PermissionMode, Placement, RunsOn, Trigger, TriggerInput, Work,
};
use pinned_core::graph::runner::{self, Session, Status};
use pinned_core::graph::{Edge, Graph, Node, NodeId, NodeKind, PortId, Watch};
use pinned_core::machine::{Launch, Machine};
use pinned_core::wire::TabView;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MACHINE: &str = "local";
const ALPHA: &str = "ALPHA";
const BETA: &str = "BETA";
const BOGUS: &str = "00000000-0000-4000-8000-000000000099";
const EVIDENCE: &str = "/Users/felipe/projetos/personal-harness/.audit/n8n-graph/swarm/w8-evidence";
const TURN: Duration = Duration::from_secs(180);

enum Proof {
    SameTab,
    Resume,
    Seeded,
}

#[test]
#[ignore]
fn live_two_talks_share_one_session() {
    run_case("same-tab", Proof::SameTab);
}

#[test]
#[ignore]
fn live_dead_tab_resumes_the_same_session() {
    run_case("resume", Proof::Resume);
}

#[test]
#[ignore]
fn live_missing_resume_opens_a_history_seeded_session() {
    run_case("seeded", Proof::Seeded);
}

fn run_case(name: &str, proof: Proof) {
    static GATE: Mutex<()> = Mutex::new(());
    let _gate = GATE.lock().unwrap_or_else(|error| error.into_inner());
    let exe = server_bin();
    let _trust = trust_folder(&workspace(name));
    let mut kept = Kept::new(name);
    let machine = machine(&exe, &kept.machine_dir);
    machine.listen(|_| Err("the live graph test has no hub call".into())).expect("listen");
    kept.machine = Some(machine.clone());
    let connectors = connectors(&exe, &kept.store_dir, &kept.folder);
    let saved = connectors.save_connector(talks_connector(&kept.folder)).expect("save graph");
    let stored = fs::read_to_string(kept.store_dir.join("connectors.json")).expect("connectors.json");
    assert!(stored.contains("talk-1"), "an edited two-talk graph was stored as the fixed pipeline: {stored}");
    connectors
        .begin(
            saved.id.clone(),
            Subject::LinearIssue { issue: format!("w8-{name}") },
            trigger(&format!("w8-{name}")),
            "2026-10-30T03:00:00Z",
        )
        .expect("begin");

    let open = opener(machine.clone(), kept.log.clone(), kept.screen.clone());
    drive(&connectors, &open, "open the first talk");
    let first = wait_turn(&machine, &only_tab(&machine), 0, ALPHA, &kept);
    let first_id = first.session.clone().expect("session");
    let first_tab = first.id.clone();
    kept.note(format!("first tab {first_tab} session {first_id}"));
    kept.save_thread(&machine, &first_tab, "first");

    feed(&connectors, &machine, &open, &[], &[first.clone()]);
    match proof {
        Proof::SameTab => {
            let second = wait_turn(&machine, &first_tab, 1, BETA, &kept);
            assert_eq!(second.id, first_tab, "the second talk opened another tab");
            assert_eq!(second.session.as_deref(), Some(first_id.as_str()), "the second talk opened another session");
            assert_eq!(machine.tabs().len(), 1, "a second tab stayed open");
            let page = kept.save_thread(&machine, &first_tab, "second");
            assert!(word(&page, ALPHA) && word(&page, BETA), "the second answer forgot ALPHA: {page}");
            finish(&connectors, &machine, &open, &second);
        }
        Proof::Resume => {
            let before = machine.view(&first_tab).expect("tab");
            assert!(machine.close_tab(&first_tab), "close_tab");
            feed(&connectors, &machine, &open, &[before], &machine.tabs());
            let (tab, claude) = live_session(&connectors);
            assert_eq!(claude.as_deref(), Some(first_id.as_str()), "resume stored a different session id");
            assert_ne!(tab, first_tab, "resume reused the closed tab");
            assert!(kept.log_contains(&format!("--resume\n{first_id}")), "resume did not pass the session id\n{}", kept.log_text());
            let second = wait_turn(&machine, &tab, 1, ALPHA, &kept);
            assert_eq!(second.session.as_deref(), Some(first_id.as_str()), "the resumed hook reported a new session");
            let page = kept.save_thread(&machine, &tab, "second");
            assert!(word(&page, ALPHA), "the resumed answer forgot ALPHA: {page}");
            assert!(kept.log_contains("then BETA"), "the second talk was not typed after resume\n{}", kept.log_text());
            finish(&connectors, &machine, &open, &second);
        }
        Proof::Seeded => {
            connectors.set_live_claude(BOGUS).expect("edit stored session");
            let before = machine.view(&first_tab).expect("tab");
            assert!(machine.close_tab(&first_tab), "close_tab");
            feed(&connectors, &machine, &open, &[before], &machine.tabs());
            assert!(kept.log_contains(&format!("--resume\n{BOGUS}")), "the bogus id was not resumed\n{}", kept.log_text());
            assert!(
                kept.log_contains("This session continues a run that already happened."),
                "a missing resume did not open a seeded session\n{}",
                kept.log_text()
            );
            let (tab, claude) = live_session(&connectors);
            assert_ne!(claude.as_deref(), Some(first_id.as_str()));
            assert_ne!(claude.as_deref(), Some(BOGUS));
            let second = wait_turn(&machine, &tab, 0, ALPHA, &kept);
            let reported = second.session.clone().expect("seeded session");
            assert_ne!(reported, first_id, "the seeded session kept the dead conversation id");
            assert_ne!(reported, BOGUS);
            let page = kept.save_thread(&machine, &tab, "second");
            assert!(word(&page, ALPHA), "the seeded session could not state ALPHA from the run history: {page}");
            kept.note(format!("seeded session {reported} replaced {first_id}"));
            finish(&connectors, &machine, &open, &second);
        }
    }

    let runs = connectors.graph_runs();
    assert!(matches!(runs[0].status, Status::Done), "run status {:#?}", runs[0].status);
    let disk: Vec<runner::Run> = serde_json::from_slice(&fs::read(kept.store_dir.join("runs.json")).expect("runs.json")).expect("runs json");
    assert_eq!(disk, runs, "runs.json does not match the stored run");
    kept.note(format!("run {} done", runs[0].id.0));
    kept.passed = true;
    eprintln!("LIVE {name}: PASS session observed, run {}", runs[0].id.0);
}

fn drive(connectors: &Connectors, open: &impl Fn(Work) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Placement>, String>> + Send>>, why: &str) {
    let error = block_on(connectors.drive(open));
    if let Err(error) = error {
        panic!("{why}: {error}");
    }
}

fn feed(connectors: &Connectors, machine: &Machine, open: &impl Fn(Work) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Placement>, String>> + Send>>, before: &[TabView], after: &[TabView]) {
    let error = block_on(connectors.on_tabs(MACHINE, before, after, open));
    if let Err(error) = &error {
        eprintln!("LIVE tabs now: {}", serde_json::to_string(&machine.tabs()).unwrap_or_default());
        panic!("on_tabs: {error}");
    }
}

fn finish(connectors: &Connectors, machine: &Machine, open: &impl Fn(Work) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Placement>, String>> + Send>>, done: &TabView) {
    feed(connectors, machine, open, &[], std::slice::from_ref(done));
}

fn opener(machine: Arc<Machine>, log: Arc<Mutex<Vec<String>>>, screen: Arc<Mutex<Vec<u8>>>) -> impl Fn(Work) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Placement>, String>> + Send>> {
    move |work: Work| {
        let machine = machine.clone();
        let log = log.clone();
        let screen = screen.clone();
        let line = format_work(&work);
        Box::pin(async move {
            let result = tokio::task::spawn_blocking(move || perform(machine, screen, work))
                .await
                .unwrap_or_else(|error| Err(format!("the machine task panicked: {error}")));
            log.lock().expect("log").push(format!("{line}\n-> {result:?}\n"));
            result
        })
    }
}

fn perform(machine: Arc<Machine>, screen: Arc<Mutex<Vec<u8>>>, work: Work) -> Result<Option<Placement>, String> {
    match work {
        Work::Run(run) => {
            let tab = machine.run(&run.spec())?;
            let buf = screen.clone();
            let _ = machine.attach(
                &tab,
                "w8".to_owned(),
                Box::new(move |bytes| {
                    let mut screen = buf.lock().expect("screen");
                    screen.extend_from_slice(bytes);
                    if screen.len() > 400_000 {
                        let drop_n = screen.len() - 300_000;
                        screen.drain(..drop_n);
                    }
                }),
            );
            Ok(Some(Placement { machine: MACHINE.into(), tab }))
        }
        Work::Prompt { env, text } => match machine.deliver(&env, &text) {
            true => Ok(None),
            false => Err("no session answers that thread".into()),
        },
        Work::Delivery(_) => Err("this live test types into the session".into()),
    }
}

fn wait_turn(machine: &Machine, tab: &str, prior: usize, need: &str, kept: &Kept) -> TabView {
    let start = Instant::now();
    let mut last = String::new();
    let mut saw_working = false;
    loop {
        let view = machine.view(tab).unwrap_or_else(|error| panic!("{error}\n{}", kept.screen_text()));
        let line = format!("{:?} session {:?} updated {:?}", view.status, view.session, view.updated);
        if line != last {
            eprintln!("LIVE {tab}: {line}");
            last = line;
        }
        if view.status == Some(AgentStatus::Failed) {
            panic!("claude failed\n{}", kept.screen_text());
        }
        if view.status == Some(AgentStatus::Working) {
            saw_working = true;
        }
        // `prior` is the assistant lines already in the transcript. The stop hook can mark Done
        // before the new line is flushed, so a Done with no new line keeps waiting.
        let lines = assistant_lines(machine, tab);
        if saw_working && view.status == Some(AgentStatus::Done) && view.session.is_some() && lines.len() > prior {
            let answer = lines.last().cloned().unwrap_or_default();
            if word(&answer, need) {
                return view;
            }
            panic!("the new answer missed {need}: {answer}\nthread: {:?}", thread_text(machine, tab));
        }
        if start.elapsed() > TURN {
            panic!("timed out waiting for {need} on {tab}\nthread: {:?}\nscreen:\n{}", thread_text(machine, tab), kept.screen_text());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn live_session(connectors: &Connectors) -> (String, Option<String>) {
    let runs = connectors.graph_runs();
    let Session::Live { tab, claude, .. } = &runs[0].session else {
        panic!("session after the tab died: {:?}", runs[0].session);
    };
    (tab.clone(), claude.clone())
}

fn only_tab(machine: &Machine) -> String {
    let tabs = machine.tabs();
    assert_eq!(tabs.len(), 1, "expected one tab, saw {}", serde_json::to_string(&tabs).unwrap_or_default());
    tabs[0].id.clone()
}

fn assistant_lines(machine: &Machine, tab: &str) -> Vec<String> {
    machine.thread(tab, 0).ok().map(|page| page.items.into_iter().filter_map(|item| match item {
        pinned_core::thread::Item::Text { text, .. } => Some(text),
        _ => None,
    }).collect()).unwrap_or_default()
}

fn thread_text(machine: &Machine, tab: &str) -> Option<String> {
    let lines = assistant_lines(machine, tab);
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn word(page: &str, word: &str) -> bool {
    page.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| part == word)
}

fn format_work(work: &Work) -> String {
    match work {
        Work::Run(run) => format!("run\n{}", run.args.join("\n")),
        Work::Prompt { text, .. } => format!("prompt\n{text}"),
        Work::Delivery(delivery) => format!("delivery {}", delivery.connector),
    }
}

fn talks_connector(folder: &Path) -> Connector {
    Connector {
        id: "w8".into(),
        name: "Live session".into(),
        enabled: true,
        trigger: Trigger::LinearLabel { label: "claude".into() },
        folder: folder.into(),
        model: "haiku".into(),
        effort: Effort::Low,
        permission_mode: PermissionMode::BypassPermissions,
        worktree: false,
        instructions: "Do not use tools. Do not read or change files. Answer in the chat only.".into(),
        columns: Default::default(),
        since: String::new(),
        handled: vec![],
        team: String::new(),
        label: String::new(),
        fix: Fix::Off,
        created: Default::default(),
        followed: Default::default(),
        paged: Default::default(),
        proof: false,
        ask: None,
        runs_on: RunsOn::Any,
        repo: String::new(),
        account: String::new(),
        graph: Some(two_talks()),
    }
}

fn two_talks() -> Graph {
    let talk = |id: &str, prompt: &str, x: i32| Node {
        id: NodeId(id.into()),
        name: id.into(),
        position: [x, 0],
        kind: NodeKind::Talk { prompt: prompt.into() },
    };
    Graph {
        nodes: vec![
            Node {
                id: NodeId("t".into()),
                name: "Trigger".into(),
                position: [0, 0],
                kind: NodeKind::Trigger { watch: Watch::LinearLabel { label: "claude".into() } },
            },
            talk("talk-0", "Reply with exactly ALPHA and nothing else.", 1),
            talk("talk-1", "Reply with exactly the word you replied last time, then BETA.", 2),
        ],
        edges: vec![
            Edge { from: NodeId("t".into()), port: PortId("out".into()), to: NodeId("talk-0".into()) },
            Edge { from: NodeId("talk-0".into()), port: PortId("out".into()), to: NodeId("talk-1".into()) },
        ],
    }
}

fn trigger(id: &str) -> TriggerInput {
    TriggerInput::Issue {
        id: id.into(),
        identifier: "W8-1".into(),
        title: "Live session check".into(),
        description: Some("Reply to the talks. Do not change the repository.".into()),
        url: "https://example.test/W8-1".into(),
    }
}

fn connectors(exe: &Path, dir: &Path, folder: &Path) -> Connectors {
    fs::create_dir_all(dir).expect("store");
    fs::create_dir_all(folder).expect("folder");
    Connectors::new(FileStorage::new(dir.into()), exe.display().to_string())
}

fn machine(exe: &Path, dir: &Path) -> Arc<Machine> {
    fs::create_dir_all(dir).expect("machine dir");
    let mut launch = Launch::prepare(exe).expect("prepare");
    launch.vector_search = false;
    Machine::new(launch, Some(dir.into()), None, |_| {})
}

fn server_bin() -> PathBuf {
    let mut candidates = vec![];
    if let Some(path) = std::env::var_os("PINNED_SERVER") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(dir).join("debug/pinned-server"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/pinned-server"));
    candidates.into_iter().find(|path| path.is_file()).unwrap_or_else(|| {
        panic!("build pinned-server first and set PINNED_SERVER or CARGO_TARGET_DIR");
    })
}

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pinned-w8-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("workspace");
    fs::canonicalize(&dir).unwrap_or(dir)
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().build().expect("runtime").block_on(future)
}

struct Kept {
    name: String,
    folder: PathBuf,
    store_dir: PathBuf,
    machine_dir: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
    screen: Arc<Mutex<Vec<u8>>>,
    notes: Mutex<Vec<String>>,
    threads: Mutex<Vec<(String, Value)>>,
    machine: Option<Arc<Machine>>,
    passed: bool,
}

impl Kept {
    fn new(name: &str) -> Self {
        let root = PathBuf::from(EVIDENCE);
        fs::create_dir_all(&root).expect("evidence");
        let store_dir = root.join(format!("{name}-store"));
        let machine_dir = root.join(format!("{name}-machine"));
        let _ = fs::remove_dir_all(&store_dir);
        let _ = fs::remove_dir_all(&machine_dir);
        for suffix in ["work.txt", "screen.txt", "notes.txt", "runs.json", "thread-first.json", "thread-second.json"] {
            let _ = fs::remove_file(root.join(format!("{name}-{suffix}")));
        }
        Self {
            folder: workspace(name),
            store_dir: root.join(format!("{name}-store")),
            machine_dir: root.join(format!("{name}-machine")),
            name: name.into(),
            log: Arc::new(Mutex::new(vec![])),
            screen: Arc::new(Mutex::new(vec![])),
            notes: Mutex::new(vec![]),
            threads: Mutex::new(vec![]),
            machine: None,
            passed: false,
        }
    }

    fn note(&self, line: String) {
        eprintln!("LIVE {}: {line}", self.name);
        self.notes.lock().expect("notes").push(line);
    }

    fn log_text(&self) -> String {
        self.log.lock().expect("log").join("\n")
    }

    fn log_contains(&self, needle: &str) -> bool {
        self.log_text().contains(needle)
    }

    fn screen_text(&self) -> String {
        let bytes = self.screen.lock().expect("screen").clone();
        let text = String::from_utf8_lossy(&bytes);
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                continue;
            }
            out.push(c);
        }
        out
    }

    fn save_thread(&self, machine: &Machine, tab: &str, label: &str) -> String {
        let page = machine.thread(tab, 0).unwrap_or_else(|error| panic!("thread {label}: {error}\n{}", self.screen_text()));
        let value = serde_json::to_value(&page).unwrap_or(Value::Null);
        self.threads.lock().expect("threads").push((label.into(), value));
        assistant_lines(machine, tab).last().cloned().unwrap_or_default()
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        if let Some(machine) = &self.machine {
            machine.quit();
        }
        let root = PathBuf::from(EVIDENCE);
        let _ = fs::create_dir_all(&root);
        let name = &self.name;
        let _ = fs::write(root.join(format!("{name}-work.txt")), self.log_text());
        let _ = fs::write(root.join(format!("{name}-screen.txt")), self.screen_text());
        let _ = fs::write(root.join(format!("{name}-notes.txt")), self.notes.lock().expect("notes").join("\n"));
        if let Ok(bytes) = fs::read(self.store_dir.join("runs.json")) {
            let _ = fs::write(root.join(format!("{name}-runs.json")), bytes);
        }
        let threads = self.threads.lock().expect("threads");
        for (label, value) in threads.iter() {
            let _ = fs::write(root.join(format!("{name}-thread-{label}.json")), serde_json::to_string_pretty(value).unwrap_or_default());
        }
        eprintln!("LIVE {name}: evidence in {EVIDENCE} passed {}", self.passed);
    }
}

struct Trust {
    path: PathBuf,
    original: Option<Vec<u8>>,
}

fn trust_folder(folder: &Path) -> Trust {
    let path = PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".claude.json");
    let original = fs::read(&path).ok();
    if let Some(bytes) = &original {
        if let Ok(mut data) = serde_json::from_slice::<Value>(bytes) {
            let key = folder.display().to_string();
            data.as_object_mut().expect("claude.json").entry("projects").or_insert_with(|| json!({}));
            data["projects"][&key]["hasTrustDialogAccepted"] = json!(true);
            if fs::write(&path, serde_json::to_vec_pretty(&data).unwrap_or_default()).is_err() {
                return Trust { path, original: None };
            }
        }
    }
    Trust { path, original }
}

impl Drop for Trust {
    fn drop(&mut self) {
        if let Some(bytes) = self.original.take() {
            let _ = fs::write(&self.path, bytes);
        }
    }
}
