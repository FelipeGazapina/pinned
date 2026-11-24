//! The machine side of the team board (U11). Progress, spend and the tasks that wait on the user
//! are read from the task records. The notes, hand-offs and blocks written to a board live in
//! `Machine::boards`, in memory only, by the Captain's tab.

use super::Machine;
use crate::board::{self, BoardBlock, BoardEvent, BoardNote, BoardProgress, BoardSpend, BoardView};
use crate::delegate::{Parent, Task};
use crate::team_tools::{BoardNoteArgs, BoardRead};
use std::collections::HashSet;
use std::time::Instant;

/// How many parents `captain_of_task` walks before it gives up.
const MOST_STEPS: usize = 32;

const NO_CAPTAIN: &str = "only a project's Captain and its workers have a board";

impl Machine {
    /// `Call::Board`: the board of the Captain of `tab`, or only `task`'s part. Empty for a tab
    /// with no Captain.
    pub(super) fn board(&self, tab: &str, task: Option<&str>) -> Result<BoardView, String> {
        match self.board_caller(Some(tab), None) {
            Some((captain, _)) => self.board_view(&captain, task),
            None => Ok(BoardView::default()),
        }
    }

    /// U11's hook: pinned writes `event` of task `task` to its Captain's board. An event of a task
    /// with no Captain is dropped.
    pub(super) fn board_write(&self, task: &str, event: BoardEvent) {
        // Progress reads a settled task from its record.
        if matches!(event, BoardEvent::Settled { .. }) {
            return;
        }
        if let Some(captain) = self.captain_of_task(task) {
            self.boards.lock().unwrap().add(&captain, event);
        }
    }

    pub(super) fn board_note(&self, tab: Option<&str>, task: Option<&str>, call: BoardNoteArgs) -> Result<String, String> {
        let (captain, by) = self.board_caller(tab, task).ok_or(NO_CAPTAIN)?;
        let (file, text) = board::check_note(&call.file, &call.text)?;
        let note = BoardNote { by, file, text, at: super::unix_millis() };
        self.boards.lock().unwrap().add(&captain, BoardEvent::Note(note.clone()));
        serde_json::to_string(&note).map_err(|e| e.to_string())
    }

    pub(super) fn board_read(&self, tab: Option<&str>, task: Option<&str>, call: BoardRead) -> Result<String, String> {
        let (captain, _) = self.board_caller(tab, task).ok_or(NO_CAPTAIN)?;
        serde_json::to_string(&self.board_view(&captain, call.task.as_deref())?).map_err(|e| e.to_string())
    }

    /// The Captain whose board the caller uses, and who it writes as: its task, or `captain`. A
    /// task sent beside a tab is the caller, so a worker never writes as its Captain.
    fn board_caller(&self, tab: Option<&str>, task: Option<&str>) -> Option<(String, String)> {
        let task = match task.filter(|id| !id.is_empty()) {
            Some(task) => task.to_owned(),
            None => {
                let tab = tab.filter(|id| !id.is_empty())?;
                match self.tab(tab, |t| t.task.clone()).ok()? {
                    Some(task) => task,
                    None => return self.captain_project(tab).is_ok().then(|| (tab.to_owned(), "captain".to_owned())),
                }
            }
        };
        Some((self.captain_of_task(&task)?, task))
    }

    /// The Captain at the top of task `id`'s parents. A worker thread's tab stands for the task it
    /// runs, so what that thread delegates is its Captain's too.
    fn captain_of_task(&self, id: &str) -> Option<String> {
        let mut parent = self.delegates.get(id)?.parent;
        for _ in 0..MOST_STEPS {
            let next = match parent {
                Parent::Task(task) => task,
                Parent::Tab(tab) => match self.tab(&tab, |t| t.task.clone()).ok()? {
                    Some(task) => task,
                    None => return self.captain_project(&tab).is_ok().then_some(tab),
                },
            };
            parent = self.delegates.get(&next)?.parent;
        }
        None
    }

    /// Every task whose Captain is `captain`: what its tab delegated, what those delegated in
    /// turn, and what each worker thread among them delegated from its own tab.
    fn board_tasks(&self, captain: &str) -> Vec<Task> {
        let runs: Vec<(String, String)> = self.tabs.lock().unwrap().iter().filter_map(|(id, t)| Some((id.clone(), t.task.clone()?))).collect();
        let mut seen = HashSet::new();
        let mut tasks = vec![];
        let mut tabs = vec![captain.to_owned()];
        while let Some(tab) = tabs.pop() {
            for listed in self.delegates.work_cost_of_tab(&tab).tasks {
                if !seen.insert(listed.id.clone()) {
                    continue;
                }
                tabs.extend(runs.iter().filter(|(_, task)| *task == listed.id).map(|(tab, _)| tab.clone()));
                tasks.extend(self.delegates.get(&listed.id));
            }
        }
        tasks
    }

    /// `captain`'s board, or only task `part`'s part of it. A task that is not on the board is
    /// refused, so one team never reads another's.
    fn board_view(&self, captain: &str, part: Option<&str>) -> Result<BoardView, String> {
        let mut tasks = self.board_tasks(captain);
        let mut kept = self.boards.lock().unwrap().kept(captain);
        if let Some(id) = part {
            tasks.retain(|task| task.id == id);
            if tasks.is_empty() {
                return Err(format!("no task {id} on this board"));
            }
            kept.notes.retain(|note| note.by == id);
            kept.handoffs.retain(|handoff| handoff.from == id || handoff.to == id);
            kept.blocks.retain(|block| block.task == id);
        }
        let mut progress: Vec<BoardProgress> = tasks
            .iter()
            .map(|task| BoardProgress { task: task.id.clone(), title: task.title.clone(), state: task.state.as_str().to_owned(), at: task.ended.unwrap_or(task.started) })
            .collect();
        progress.sort_by(|a, b| (a.at, &a.task).cmp(&(b.at, &b.task)));
        // A block written for a task that has since ended is stale.
        kept.blocks.retain(|block| self.delegates.get(&block.task).is_none_or(|task| task.ended.is_none()));
        let (now, now_ms) = (Instant::now(), super::unix_millis());
        let mut blocks: Vec<BoardBlock> = tasks.iter().filter_map(|task| waits_on_user(task, now, now_ms)).chain(kept.blocks).collect();
        blocks.sort_by_key(|block| block.at);
        let usd: f64 = tasks.iter().map(|task| task.usage.cost_usd.unwrap_or(0.0) + task.usage.box_usd.unwrap_or(0.0)).sum();
        Ok(BoardView { progress, notes: kept.notes, handoffs: kept.handoffs, blocks, spend: BoardSpend { cents: (usd * 100.0).round() as u64 } })
    }
}

/// The block of a task that has not ended while its worker waits on the user, here or on another
/// machine, as `Delegates::plan_facts` reads a wait.
fn waits_on_user(task: &Task, now: Instant, now_ms: u64) -> Option<BoardBlock> {
    if task.ended.is_some() {
        return None;
    }
    if let Some((remote, ask)) = task.remote.as_ref().and_then(|remote| Some((remote, remote.waiting.as_ref()?))) {
        return Some(BoardBlock { task: task.id.clone(), why: format!("waits on the user on {}", remote.name), at: ask.since.saturating_mul(1000) });
    }
    let waited = now.saturating_duration_since(task.run.user_wait?).as_millis() as u64;
    Some(BoardBlock { task: task.id.clone(), why: "waits on the user".into(), at: now_ms.saturating_sub(waited) })
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;
    use crate::accounts::{Account, Provider};
    use crate::agent::{role_env, Role};
    use crate::board::BoardHandoff;
    use crate::delegate::{Access, Mode, NewTask, RemoteAsk, RemoteTarget};
    use crate::usage::TaskUsage;
    use crate::wire::Call;
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use std::sync::Arc;

    /// A machine with a Captain on each of `projects`, folders made under a fresh scratch folder.
    fn captains(name: &str, projects: &[&str]) -> (Arc<Machine>, PathBuf, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("pinned-board-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = Machine::new(launch(), None, None, |_| {});
        let captains = projects
            .iter()
            .map(|project| {
                std::fs::create_dir_all(dir.join(project)).unwrap();
                let path = m.open_project(dir.join(project)).display().to_string();
                m.open_orchestrator(&path).unwrap()
            })
            .collect();
        (m, dir, captains)
    }

    /// A task `parent` delegated, as `delegate_task` records one.
    fn task_of(m: &Machine, parent: Parent, title: &str) -> String {
        let caller_key = match &parent {
            Parent::Tab(tab) => format!("tab:{tab}"),
            Parent::Task(task) => format!("task:{task}"),
        };
        let new = NewTask {
            parent,
            tool: None,
            title: title.into(),
            account: "work".into(),
            provider: Provider::Claude,
            model: None,
            cwd: std::env::temp_dir(),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key,
            client_request_id: None,
            role: None,
        };
        m.delegates.begin(new).id
    }

    /// A worker thread's tab that runs `task`, or a plain thread's tab for none.
    fn thread(m: &Machine, task: Option<&str>) -> String {
        let env = if task.is_some() { role_env(Role::Worker, &std::env::temp_dir()) } else { vec![] };
        let tab = m.add_tab(std::env::temp_dir(), Account::builtin(Provider::Claude), vec![], env, false, None);
        m.tab(&tab, |t| t.task = task.map(str::to_owned)).unwrap();
        tab
    }

    fn settle(m: &Machine, task: &str) {
        m.delegates.settle_thread(task, Some("done".into()), None, None, vec![], None);
    }

    fn note(m: &Machine, tab: Option<&str>, task: Option<&str>, file: &str, text: &str) -> Result<BoardNote, String> {
        let tool = crate::team_tools::request("board_note", &json!({ "file": file, "text": text })).unwrap();
        m.team_tool(tab, task, tool).map(|body| serde_json::from_str(&body).unwrap())
    }

    fn read(m: &Machine, tab: Option<&str>, task: Option<&str>, args: Value) -> Result<BoardView, String> {
        let tool = crate::team_tools::request("board_read", &args).unwrap();
        m.team_tool(tab, task, tool).map(|body| serde_json::from_str(&body).unwrap())
    }

    fn board(m: &Machine, tab: &str, task: Option<&str>) -> Result<BoardView, String> {
        m.call(Call::Board { tab: tab.into(), task: task.map(str::to_owned) }).map(|value| serde_json::from_value(value).unwrap())
    }

    fn ids(view: &BoardView) -> Vec<&str> {
        view.progress.iter().map(|entry| entry.task.as_str()).collect()
    }

    #[test]
    fn a_note_is_written_as_its_task_or_as_the_captain_and_the_board_lists_them_oldest_first() {
        let (m, dir, captains) = captains("notes", &["app"]);
        let captain = captains[0].as_str();
        let task = task_of(&m, Parent::Tab(captain.into()), "Fix login");
        let worker = thread(&m, Some(&task));
        let by = |tab: Option<&str>, task: Option<&str>| note(&m, tab, task, "src/a.rs", "moved").unwrap().by;
        assert_eq!(by(Some(captain), None), "captain");
        assert_eq!(by(Some(&worker), None), task, "a worker thread writes as its task");
        assert_eq!(by(None, Some(&task)), task, "a headless child writes as its task");
        assert_eq!(by(Some(captain), Some(&task)), task, "a task sent beside the Captain's tab still writes as the task");

        let notes = board(&m, captain, None).unwrap().notes;
        assert_eq!(notes.iter().map(|note| note.by.as_str()).collect::<Vec<_>>(), ["captain", task.as_str(), task.as_str(), task.as_str()]);
        assert!(notes.iter().all(|note| note.file == "src/a.rs" && note.text == "moved"));
        assert!(notes.windows(2).all(|pair| pair[0].at <= pair[1].at), "oldest first");
        let body = m.team_tool(Some(&worker), None, crate::team_tools::request("board_read", &json!({})).unwrap()).unwrap();
        assert_eq!(serde_json::from_str::<BoardView>(&body).unwrap().notes, notes, "board_read answers a BoardView");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_note_leaves_the_board_as_it_was() {
        let (m, dir, captains) = captains("refused", &["app"]);
        let captain = Some(captains[0].as_str());
        for (file, text) in [("", "moved"), ("   ", "moved"), ("src/a.rs", ""), ("src/a.rs", &"x".repeat(201)), (&"a".repeat(301), "moved")] {
            assert!(note(&m, captain, None, file, text).is_err(), "{file:?} {text:?}");
        }
        let long = note(&m, captain, None, "src/a.rs", &"x".repeat(201)).unwrap_err();
        assert!(long.contains("200"), "{long}");
        assert_eq!(board(&m, &captains[0], None).unwrap().notes, vec![]);

        let accents = "é".repeat(200);
        assert_eq!(note(&m, captain, None, "src/a.rs", &accents).unwrap().text, accents, "200 characters fit, however many bytes");
        assert_eq!(board(&m, &captains[0], None).unwrap().notes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_caller_with_no_captain_is_refused_and_writes_nothing() {
        let (m, dir, captains) = captains("no-captain", &["app"]);
        let plain = thread(&m, None);
        let loose = task_of(&m, Parent::Tab(plain.clone()), "Loose");
        let remote = task_of(&m, Parent::Tab(crate::delegate::Origin::key("m-mac", "tab-7")), "For another machine");
        let global = m.add_tab(std::env::temp_dir(), Account::builtin(Provider::Claude), vec![], crate::global_orchestrator::env(), false, None);
        let callers = [(Some(plain.as_str()), None), (Some("tab-1"), None), (Some(global.as_str()), None), (None, Some(loose.as_str())), (None, Some(remote.as_str())), (None, Some("task-none"))];
        for (tab, task) in callers {
            let wrote = note(&m, tab, task, "src/a.rs", "moved").unwrap_err();
            assert!(wrote.contains("Captain") && !wrote.contains("not available"), "{tab:?} {task:?}: {wrote}");
            let looked = read(&m, tab, task, json!({})).unwrap_err();
            assert!(looked.contains("Captain"), "{tab:?} {task:?}: {looked}");
        }
        assert_eq!(board(&m, &captains[0], None).unwrap(), BoardView::default());
        assert_eq!(m.boards.lock().unwrap().kept(&plain), BoardView::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_shows_each_task_under_the_captain_as_its_record_stands() {
        let (m, dir, captains) = captains("progress", &["one", "two"]);
        let (a, b) = (captains[0].as_str(), captains[1].as_str());
        let direct = task_of(&m, Parent::Tab(a.into()), "Fix login");
        let sub = task_of(&m, Parent::Task(direct.clone()), "Write the test");
        let other = task_of(&m, Parent::Tab(b.into()), "Fix the footer");
        let entry = |id: &str, title: &str, state: &str, at: u64| BoardProgress { task: id.into(), title: title.into(), state: state.into(), at };
        let started = |id: &str| m.delegates.get(id).unwrap().started;

        let progress = board(&m, a, None).unwrap().progress;
        assert_eq!(progress.len(), 2, "{progress:?}");
        assert!(progress.contains(&entry(&direct, "Fix login", "working", started(&direct))));
        assert!(progress.contains(&entry(&sub, "Write the test", "working", started(&sub))));

        settle(&m, &sub);
        settle(&m, &direct);
        let ended = m.delegates.get(&direct).unwrap().ended.unwrap();
        let progress = board(&m, a, None).unwrap().progress;
        assert!(progress.contains(&entry(&direct, "Fix login", "result_available", ended)), "{progress:?}");
        assert!(progress.windows(2).all(|pair| (pair[0].at, &pair[0].task) <= (pair[1].at, &pair[1].task)), "oldest first");
        assert!(!progress.iter().any(|entry| entry.task == other), "another project's task is on its own board");
        assert_eq!(ids(&board(&m, b, None).unwrap()), [other.as_str()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_worker_thread_delegates_is_on_its_captains_board() {
        let (m, dir, captains) = captains("worker-thread", &["app"]);
        let task = task_of(&m, Parent::Tab(captains[0].clone()), "Fix login");
        let worker = thread(&m, Some(&task));
        let its_own = task_of(&m, Parent::Tab(worker.clone()), "Check the form");
        let deeper = task_of(&m, Parent::Task(its_own.clone()), "Run the form tests");
        let view = board(&m, &captains[0], None).unwrap();
        assert!(ids(&view).contains(&its_own.as_str()) && ids(&view).contains(&deeper.as_str()), "{view:?}");
        assert_eq!(note(&m, None, Some(&deeper), "src/form.rs", "checked").unwrap().by, deeper);
        assert_eq!(board(&m, &captains[0], None).unwrap().notes.len(), 1, "its note is on that board");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spend_adds_up_the_known_costs_of_the_boards_tasks() {
        let (m, dir, captains) = captains("spend", &["app"]);
        let under = || Parent::Tab(captains[0].clone());
        let (boxed, priced, unpriced) = (task_of(&m, under(), "In a box"), task_of(&m, under(), "Priced"), task_of(&m, under(), "Not priced"));
        m.delegates.meter(
            &[(boxed, TaskUsage { cost_usd: Some(0.40), box_usd: Some(0.10), ..TaskUsage::default() }), (priced, TaskUsage { cost_usd: Some(0.25), ..TaskUsage::default() })],
            &[],
            None,
        );
        assert_eq!(m.delegates.get(&unpriced).unwrap().usage.cost_usd, None);
        assert_eq!(board(&m, &captains[0], None).unwrap().spend.cents, 75);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_task_that_waits_on_the_user_is_blocked_until_answered_or_ended() {
        let (m, dir, captains) = captains("waits", &["app"]);
        let task = task_of(&m, Parent::Tab(captains[0].clone()), "Deploy");
        let blocks = || board(&m, &captains[0], None).unwrap().blocks;
        let asked = super::super::unix_millis();
        m.delegates.note_user_waits(std::slice::from_ref(&task), Instant::now());
        let shown = blocks();
        assert_eq!(shown.iter().map(|block| (block.task.as_str(), block.why.as_str())).collect::<Vec<_>>(), [(task.as_str(), "waits on the user")]);
        assert!(shown[0].at.abs_diff(asked) < 1000, "{} {asked}", shown[0].at);

        m.delegates.note_user_waits(&[], Instant::now());
        assert_eq!(blocks(), vec![], "answered");
        m.delegates.note_user_waits(std::slice::from_ref(&task), Instant::now());
        assert_eq!(blocks().len(), 1);
        settle(&m, &task);
        assert_eq!(blocks(), vec![], "it ended while it still waited");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wait_on_another_machine_names_it_from_when_it_began() {
        let (m, dir, captains) = captains("remote-wait", &["app"]);
        let id = task_of(&m, Parent::Tab(captains[0].clone()), "Deploy");
        let mut task = m.delegates.get(&id).unwrap();
        let ask = RemoteAsk { kind: crate::global_orchestrator::AskKind::Approval, tool: Some("Bash".into()), ask: "deploy?".into(), since: 1_700_000_000 };
        task.remote = Some(RemoteTarget { machine: "m-mini".into(), name: "mini".into(), tab: None, looked: None, waiting: Some(ask) });
        assert_eq!(waits_on_user(&task, Instant::now(), 0), Some(BoardBlock { task: id, why: "waits on the user on mini".into(), at: 1_700_000_000_000 }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn board_write_keeps_handoffs_and_blocks_on_the_tasks_captains_board() {
        let (m, dir, captains) = captains("write", &["app"]);
        let captain = captains[0].as_str();
        let (t, u) = (task_of(&m, Parent::Tab(captain.into()), "Build"), task_of(&m, Parent::Tab(captain.into()), "Test"));
        let block = BoardBlock { task: t.clone(), why: "waits on the schema".into(), at: 5 };
        let handoff = BoardHandoff { from: t.clone(), to: u.clone(), at: 6 };
        m.board_write(&t, BoardEvent::Block(block.clone()));
        m.board_write(&t, BoardEvent::Handoff(handoff.clone()));
        let view = board(&m, captain, None).unwrap();
        assert_eq!((view.blocks, view.handoffs.clone()), (vec![block.clone()], vec![handoff.clone()]));

        let kept = m.boards.lock().unwrap().kept(captain);
        m.board_write(&t, BoardEvent::Settled { task: t.clone() });
        let plain = thread(&m, None);
        let loose = task_of(&m, Parent::Tab(plain.clone()), "Loose");
        m.board_write(&loose, BoardEvent::Block(BoardBlock { task: loose.clone(), ..block.clone() }));
        m.board_write("task-none", BoardEvent::Handoff(handoff.clone()));
        assert_eq!(m.boards.lock().unwrap().kept(captain), kept, "Settled and a task with no Captain change nothing");
        assert_eq!(m.boards.lock().unwrap().kept(&plain), BoardView::default());

        settle(&m, &t);
        let view = board(&m, captain, None).unwrap();
        assert_eq!(view.blocks, vec![], "a block of a task that ended is gone");
        assert_eq!(view.handoffs, vec![handoff]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_tasks_part_holds_only_its_own() {
        let (m, dir, captains) = captains("part", &["app"]);
        let captain = captains[0].as_str();
        let (t1, t2) = (task_of(&m, Parent::Tab(captain.into()), "Build"), task_of(&m, Parent::Tab(captain.into()), "Test"));
        let worker = thread(&m, Some(&t2));
        note(&m, None, Some(&t1), "src/a.rs", "built").unwrap();
        note(&m, None, Some(&t2), "src/a_test.rs", "tested").unwrap();
        let handoff = BoardHandoff { from: t1.clone(), to: t2.clone(), at: 7 };
        m.board_write(&t1, BoardEvent::Handoff(handoff.clone()));
        m.delegates.meter(&[(t1.clone(), TaskUsage { cost_usd: Some(0.30), ..TaskUsage::default() }), (t2.clone(), TaskUsage { cost_usd: Some(0.50), ..TaskUsage::default() })], &[], None);

        let from_app = board(&m, captain, Some(&t1)).unwrap();
        assert_eq!(read(&m, Some(&worker), None, json!({ "taskId": t1 })).unwrap(), from_app, "board_read answers the same part");
        assert_eq!(ids(&from_app), [t1.as_str()]);
        assert_eq!(from_app.notes.iter().map(|note| note.text.as_str()).collect::<Vec<_>>(), ["built"]);
        assert_eq!(from_app.handoffs, vec![handoff]);
        assert_eq!(from_app.spend.cents, 30);
        assert_eq!(board(&m, captain, None).unwrap().spend.cents, 80);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_team_reads_nothing_of_another_teams_board() {
        let (m, dir, captains) = captains("isolation", &["one", "two"]);
        let (a, b) = (captains[0].as_str(), captains[1].as_str());
        let secret = task_of(&m, Parent::Tab(a.into()), "Rotate the billing keys");
        note(&m, None, Some(&secret), "src/billing.rs", "the old key is still live").unwrap();
        let theirs = task_of(&m, Parent::Tab(b.into()), "Fix the footer");
        let worker = thread(&m, Some(&theirs));

        let seen = read(&m, Some(&worker), None, json!({})).unwrap();
        assert_eq!(ids(&seen), [theirs.as_str()]);
        assert!(seen.notes.is_empty(), "{seen:?}");
        let refused = read(&m, Some(&worker), None, json!({ "taskId": secret })).unwrap_err();
        assert!(refused.contains(&secret) && !refused.contains("billing"), "{refused}");
        assert!(board(&m, a, Some(&theirs)).is_err(), "nor does the app ask one board for another's task");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tab_with_no_captain_reads_an_empty_board() {
        let (m, dir, _) = captains("empty", &[]);
        let plain = thread(&m, None);
        let empty = json!({ "progress": [], "notes": [], "handoffs": [], "blocks": [], "spend": { "cents": 0 } });
        for tab in [plain.as_str(), "tab-1"] {
            assert_eq!(m.call(Call::Board { tab: tab.into(), task: None }), Ok(empty.clone()), "{tab}");
            assert_eq!(m.call(Call::Board { tab: tab.into(), task: Some("task-1".into()) }), Ok(empty.clone()), "{tab}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
