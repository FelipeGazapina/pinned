use super::*;
use std::sync::atomic::AtomicU32;

#[derive(Clone)]
struct Cache {
    at: Instant,
    list: String,
}

pub struct Delegates {
    tasks: Mutex<Vec<Task>>,
    agents: Arc<Mutex<HashMap<String, Agent>>>,
    reports: Mutex<Vec<Report>>,
    store: Option<PathBuf>,
    cursor: Arc<Mutex<Option<Cache>>>,
    warming: Arc<AtomicBool>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Report {
    tab: String,
    #[serde(default)]
    session: Option<String>,
    text: String,
    #[serde(default)]
    claimed: bool,
    #[serde(default)]
    task: Option<String>,
}

struct Agent {
    pid: u32,
    seen: Vec<Process>,
}

impl Agent {
    fn stop(self) {
        kill_tree(self.pid);
        kill_seen(&self.seen);
    }
}

enum Stop {
    Exited(Agent),
    Answered(u32),
    Watch(u32, Vec<Process>),
}

impl Default for Delegates {
    fn default() -> Self {
        Self {
            tasks: Mutex::new(vec![]),
            agents: Arc::new(Mutex::new(HashMap::new())),
            reports: Mutex::new(vec![]),
            store: None,
            cursor: Arc::new(Mutex::new(None)),
            warming: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct NewTask {
    pub parent: Parent,
    pub tool: Option<String>,
    pub title: String,
    pub account: AccountId,
    pub provider: Provider,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub mode: Mode,
    pub access: Access,
    pub isolated: bool,
    pub caller_key: String,
    pub client_request_id: Option<String>,
    pub role: Option<String>,
}

pub enum Claim {
    Fresh(Box<Task>),
    Existing(String),
}

impl Delegates {
    pub fn load(file: Option<PathBuf>) -> Self {
        let delegates = Self { store: file.clone(), ..Self::default() };
        let Some(file) = file else { return delegates };
        let Ok(bytes) = std::fs::read(&file) else { return delegates };
        let Ok(saved) = serde_json::from_slice::<SavedFile>(&bytes) else { return delegates };
        *delegates.tasks.lock().unwrap() = saved.tasks.into_iter().map(SavedTask::into_task).collect();
        *delegates.reports.lock().unwrap() = saved.reports;
        if delegates.adopt_restart() {
            delegates.persist();
        }
        delegates
    }

    /// The same caller and `clientRequestId` returns the task already started.
    pub fn claim(&self, new: NewTask) -> Claim {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(request_id) = new.client_request_id.as_deref() {
            if let Some(existing) = tasks.iter().find(|task| {
                task.caller_key == new.caller_key && task.client_request_id.as_deref() == Some(request_id)
            }) {
                return Claim::Existing(existing.id.clone());
            }
        }
        let task = fresh(new);
        tasks.push(task.clone());
        drop(tasks);
        self.persist();
        Claim::Fresh(Box::new(task))
    }

    pub fn begin(&self, new: NewTask) -> Task {
        match self.claim(new) {
            Claim::Fresh(task) => *task,
            Claim::Existing(id) => self.tasks.lock().unwrap().iter().find(|task| task.id == id).unwrap().clone(),
        }
    }

    pub fn place(&self, id: &str, cwd: PathBuf, isolated: bool) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.cwd = cwd;
            task.isolated = isolated;
        }
        self.persist();
    }

    pub fn set_model(&self, id: &str, model: Option<String>) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.model = model;
        }
        self.persist();
    }

    pub fn set_limits(&self, id: &str, budget_ms: u64, limit_ms: u64) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.run.budget_ms = budget_ms;
            task.run.limit_ms = limit_ms;
        }
        self.persist();
    }

    pub fn enforce_limits(&self, now: Instant, path: Option<&str>) {
        let (late, over, quiet) = {
            let mut tasks = self.tasks.lock().unwrap();
            let mut late = vec![];
            let mut over = vec![];
            let mut quiet = vec![];
            for task in tasks.iter_mut().filter(|task| !task.state.terminal()) {
                let ran = millis(now.saturating_duration_since(task.run.begun));
                if ran >= task.run.limit_ms {
                    over.push((task.id.clone(), format!("pinned stopped it after {}, its limit", span(ran))));
                } else if task.quiet_after_reply(now) {
                    quiet.push(task.id.clone());
                } else if ran >= task.run.budget_ms && !task.run.told_late {
                    task.run.told_late = true;
                    if let Parent::Tab(tab) = &task.parent {
                        late.push(Report { tab: tab.clone(), session: task.parent_session.clone(), text: late_line(task, ran), claimed: false, task: Some(task.id.clone()) });
                    }
                }
            }
            (late, over, quiet)
        };
        for report in late {
            self.push_report(report);
        }
        for (id, reason) in &over {
            self.stop_tree(id, reason, path);
        }
        for id in &quiet {
            self.end_quiet(id, path);
        }
        if !over.is_empty() || !quiet.is_empty() {
            self.persist();
        }
    }

    fn end_quiet(&self, id: &str, path: Option<&str>) {
        let (agent, dirs) = {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(index) = tasks.iter().position(|task| task.id == id && task.state == WorkState::Working) else { return };
            let task = &mut tasks[index];
            let mut note = format!("pinned ended it after {} with no activity", span(QUIET_AFTER_REPLY_MS));
            if !task.run.background.is_empty() {
                note.push_str(&format!(" and stopped the {} it left running", many(task.run.background.len(), "shell")));
            }
            task.ended_by = Some(note);
            let error = task.run.last_error.clone();
            let held = HeldExit { summary: task.summary.clone(), failed: error.is_some(), error };
            let dirs = close(&mut tasks, index, held);
            (self.agents.lock().unwrap().remove(id), dirs)
        };
        if let Some(agent) = agent {
            agent.stop();
        }
        for dir in dirs {
            crate::branches::drop_worktree(&dir, path);
        }
    }

    #[cfg(test)]
    pub(crate) fn backdate(&self, id: &str, by: Duration) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.started -= by.as_millis() as u64;
        }
    }

    pub fn note_parent(&self, id: &str, session: Option<String>) {
        let Some(session) = session.filter(|session| !session.is_empty()) else { return };
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.parent_session = Some(session);
        }
        self.persist();
    }

    /// Records `pid` for a task that is still working. A task that already finished is killed now,
    /// so a cancel during startup cannot leave the child running.
    pub fn remember_pid(&self, id: &str, pid: u32) {
        let kill_now = {
            let tasks = self.tasks.lock().unwrap();
            let terminal = tasks.iter().find(|task| task.id == id).is_none_or(|task| task.state.terminal());
            if terminal {
                true
            } else {
                self.agents.lock().unwrap().insert(id.to_owned(), Agent { pid, seen: vec![] });
                false
            }
        };
        if kill_now {
            kill_tree(pid);
        }
    }

    /// The pid recorded for `id`, when the child is still one quit should stop.
    pub fn remembered(&self, id: &str) -> Option<u32> {
        self.agents.lock().unwrap().get(id).map(|agent| agent.pid)
    }

    fn forget_pid(&self, id: &str) {
        self.agents.lock().unwrap().remove(id);
    }

    pub fn is_terminal(&self, id: &str) -> bool {
        self.tasks.lock().unwrap().iter().find(|task| task.id == id).is_none_or(|task| task.state.terminal())
    }

    /// Kills every child whose pid was recorded, including one remembered after `running` was read.
    pub fn kill_remembered(&self) {
        let agents: Vec<Agent> = self.agents.lock().unwrap().drain().map(|(_, agent)| agent).collect();
        for agent in agents {
            agent.stop();
        }
    }

    pub fn fail(&self, id: &str, error: String, path: Option<&str>) {
        let dir = {
            let mut tasks = self.tasks.lock().unwrap();
            self.forget_pid(id);
            let Some(task) = tasks.iter_mut().find(|task| task.id == id) else { return };
            if task.state.terminal() {
                return;
            }
            task.state = WorkState::Failed;
            task.error = Some(error);
            task.ended = Some(unix_millis());
            isolated_cwd(task)
        };
        if let Some(dir) = dir {
            crate::branches::drop_worktree(&dir, path);
        }
        self.persist();
    }

    /// Applies one child event. Returns the parent tab when a task just became deliverable.
    pub fn apply(&self, id: &str, event: ChildEvent, path: Option<&str>) -> Option<String> {
        let (ready, dirs, stop, pending) = {
            let mut tasks = self.tasks.lock().unwrap();
            let index = tasks.iter().position(|task| task.id == id)?;
            let mut watch = false;
            let (held, reaped) = match event {
                ChildEvent::Line(line) => {
                    let task = &mut tasks[index];
                    task.run.last_line = Instant::now();
                    let mut held = None;
                    for event in (driver(task.provider).parse)(&line) {
                        watch |= matches!(event, StreamEvent::Background(_) | StreamEvent::TurnEnd { .. });
                        if let Some(next) = task.note(event) {
                            held = Some(next);
                        }
                    }
                    (held, false)
                }
                ChildEvent::Exited { code, stderr } => (tasks[index].exited(code, stderr), true),
            };
            let stop = {
                let mut agents = self.agents.lock().unwrap();
                match (reaped, held.is_some()) {
                    (true, _) => agents.remove(id).map(Stop::Exited),
                    (false, true) => agents.get(id).map(|agent| Stop::Answered(agent.pid)),
                    (false, false) if watch => agents.get(id).map(|agent| Stop::Watch(agent.pid, agent.seen.clone())),
                    (false, false) => None,
                }
            };
            let dirs = match held {
                Some(held) => close(&mut tasks, index, held),
                None => close_released(&mut tasks, index),
            };
            let pending = pending_report(&mut tasks[index]);
            let id = tasks[index].id.clone();
            let ready = chain_ready(&tasks, &id).then(|| root_tab(&tasks, tasks[index].parent.clone())).flatten();
            (ready, dirs, stop, pending)
        };
        match stop {
            Some(Stop::Exited(agent)) => {
                // A reaped pid can already name another process. On Unix its group cannot, while a member lives.
                #[cfg(unix)]
                kill(agent.pid);
                kill_seen(&agent.seen);
            }
            Some(Stop::Answered(pid)) => self.stop_with_grace(id, pid),
            Some(Stop::Watch(pid, before)) => {
                let seen = seen_under(pid, &before);
                if let Some(agent) = self.agents.lock().unwrap().get_mut(id) {
                    agent.seen = seen;
                }
            }
            None => {}
        }
        for dir in dirs {
            crate::branches::drop_worktree(&dir, path);
        }
        if let Some(report) = pending {
            self.push_report(report);
        }
        self.persist();
        ready
    }

    fn stop_with_grace(&self, id: &str, pid: u32) {
        #[cfg(unix)]
        {
            kill_descendants(pid);
            let agents = self.agents.clone();
            let id = id.to_owned();
            std::thread::spawn(move || {
                std::thread::sleep(EXIT_GRACE);
                let left = agents.lock().unwrap().remove(&id);
                if let Some(agent) = left {
                    agent.stop();
                }
            });
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            if let Some(agent) = self.agents.lock().unwrap().remove(id) {
                agent.stop();
            }
        }
    }

    pub fn started_json(&self, id: &str) -> Result<Value, String> {
        let body = {
            let mut tasks = self.tasks.lock().unwrap();
            let task = tasks.iter_mut().find(|task| task.id == id).ok_or_else(|| format!("no task {id}"))?;
            if task.state.terminal() {
                task.delivered = true;
                let mut body = status_json(task);
                body["next"] = json!(result_next(task));
                body
            } else {
                json!({
                    "taskId": task.id,
                    "title": task.title,
                    "providerInstanceId": task.account,
                    "model": task.model,
                    "workState": task.state.as_str(),
                    "cwd": task.cwd.display().to_string(),
                    "next": next_line(task),
                })
            }
        };
        self.persist();
        Ok(body)
    }

    pub fn status(&self, caller: &Caller, id: &str) -> Result<Value, String> {
        let value = {
            let mut tasks = self.tasks.lock().unwrap();
            self.read(&mut tasks, caller, id)?
        };
        self.persist();
        Ok(value)
    }

    pub fn cancel(&self, caller: &Caller, id: &str, reason: &str, path: Option<&str>) -> Result<Value, String> {
        {
            let tasks = self.tasks.lock().unwrap();
            if !tasks.iter().any(|task| task.id == id) {
                return Err(format!("no task {id}"));
            }
            if !allows(&tasks, id, caller) {
                return Err(format!("task {id} belongs to another session"));
            }
        }
        self.cancel_tree(id, reason, path)
    }

    /// Cancels every working descendant, then `id`. Used when the tab closes or pinned quits.
    pub fn cancel_tree(&self, id: &str, reason: &str, path: Option<&str>) -> Result<Value, String> {
        if !self.tasks.lock().unwrap().iter().any(|task| task.id == id) {
            return Err(format!("no task {id}"));
        }
        self.stop_tree(id, reason, path);
        let body = {
            let mut tasks = self.tasks.lock().unwrap();
            let task = tasks.iter_mut().find(|task| task.id == id).ok_or_else(|| format!("no task {id}"))?;
            if task.state.terminal() {
                task.delivered = true;
            }
            status_json(task)
        };
        self.persist();
        Ok(body)
    }

    pub fn stop_for_restart(&self, path: Option<&str>) {
        let roots: Vec<String> = {
            let tasks = self.tasks.lock().unwrap();
            tasks.iter().filter(|task| !task.state.terminal() && matches!(task.parent, Parent::Tab(_))).map(|task| task.id.clone()).collect()
        };
        for id in roots {
            self.stop_tree(&id, "stopped by restart", path);
        }
        for id in self.running() {
            self.stop_one(&id, "stopped by restart", path);
        }
        self.persist();
    }

    fn read(&self, tasks: &mut [Task], caller: &Caller, id: &str) -> Result<Value, String> {
        if !tasks.iter().any(|task| task.id == id) {
            return Err(format!("no task {id}"));
        }
        if !allows(tasks, id, caller) {
            return Err(format!("task {id} belongs to another session"));
        }
        let task = tasks.iter_mut().find(|task| task.id == id).unwrap();
        if task.state.terminal() {
            task.delivered = true;
        }
        Ok(status_json(task))
    }

    fn stop_tree(&self, id: &str, reason: &str, path: Option<&str>) {
        let order = {
            let tasks = self.tasks.lock().unwrap();
            let mut order = vec![];
            postorder(&tasks, id, &mut order);
            order
        };
        for id in &order {
            self.stop_one(id, reason, path);
        }
    }

    fn stop_one(&self, id: &str, reason: &str, path: Option<&str>) {
        let (agent, dir) = {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.iter_mut().find(|task| task.id == id) else { return };
            if task.state.terminal() {
                return;
            }
            task.state = WorkState::Cancelled;
            task.summary = task.assistant.clone().or_else(|| task.summary.take());
            task.error = Some(reason.to_owned());
            task.ended = Some(unix_millis());
            let dir = isolated_cwd(task);
            let agent = self.agents.lock().unwrap().remove(id);
            (agent, dir)
        };
        if let Some(agent) = agent {
            agent.stop();
        }
        if let Some(dir) = dir {
            crate::branches::drop_worktree(&dir, path);
        }
    }

    /// Claims the notice for tasks whose direct parent is `tab`, and marks those tasks delivered.
    /// A grandchild stays out: its headless parent reads it, and it is never typed into the root tab.
    /// The caller puts the ids back with `release_notice` when the bytes were not written.
    pub fn claim_notice(&self, tab: &str) -> Option<(String, Vec<String>)> {
        let mut tasks = self.tasks.lock().unwrap();
        self.reports.lock().unwrap().retain(|report| report.claimed || !stale(report, &tasks));
        let ready: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                !task.delivered && task.state.terminal() && matches!(&task.parent, Parent::Tab(parent) if parent == tab)
            })
            .map(|(index, _)| index)
            .collect();
        let mut reports = self.reports.lock().unwrap();
        let pending: Vec<usize> = reports.iter().enumerate().filter(|(_, report)| report.tab == tab && !report.claimed).map(|(index, _)| index).collect();
        if ready.is_empty() && pending.is_empty() {
            return None;
        }
        let mut lines: Vec<String> = ready.iter().map(|index| notice_line(&tasks[*index])).collect();
        let ids: Vec<String> = ready.iter().map(|index| tasks[*index].id.clone()).collect();
        for index in &ready {
            tasks[*index].delivered = true;
        }
        for index in pending {
            reports[index].claimed = true;
            lines.push(reports[index].text.clone());
        }
        drop(reports);
        drop(tasks);
        self.persist();
        Some((lines.join("\n"), ids))
    }

    /// Claims one finished task's notice for the session that is its direct parent and marks it delivered.
    /// A grandchild stays with its headless parent. The root tab can read it, and does not send it.
    pub fn claim_task_notice(&self, caller: &Caller, id: &str) -> Result<String, String> {
        let mut tasks = self.tasks.lock().unwrap();
        if !tasks.iter().any(|task| task.id == id) {
            return Err(format!("no task {id}"));
        }
        let parent_is_caller = tasks.iter().any(|task| {
            task.id == id && matches!(&task.parent, Parent::Tab(parent) if caller.tab.as_deref() == Some(parent.as_str()))
        });
        if !parent_is_caller {
            return Err(format!("task {id} belongs to another session"));
        }
        let task = tasks.iter_mut().find(|task| task.id == id).unwrap();
        if !task.state.terminal() {
            return Err(format!("{id} is still working"));
        }
        if task.delivered {
            return Err(format!("{id} was already sent"));
        }
        task.delivered = true;
        let line = notice_line(task);
        drop(tasks);
        self.persist();
        Ok(line)
    }

    pub fn release_notice(&self, ids: &[String]) {
        let mut tasks = self.tasks.lock().unwrap();
        for task in tasks.iter_mut() {
            if ids.iter().any(|id| id == &task.id) {
                task.delivered = false;
            }
        }
        drop(tasks);
        self.persist();
    }

    pub fn queue_report(&self, tab: &str, session: Option<String>, text: String) {
        self.push_report(Report { tab: tab.to_owned(), session, text, claimed: false, task: None });
    }

    fn push_report(&self, report: Report) {
        let session = report.session.filter(|session| !session.is_empty());
        self.reports.lock().unwrap().push(Report { session, ..report });
        self.persist();
    }

    pub fn drop_claimed_reports(&self, tab: &str) {
        self.reports.lock().unwrap().retain(|report| !(report.tab == tab && report.claimed));
        self.persist();
    }

    pub fn release_reports(&self, tab: &str) {
        for report in self.reports.lock().unwrap().iter_mut() {
            if report.tab == tab {
                report.claimed = false;
            }
        }
        self.persist();
    }

    pub fn stamp_sessions(&self, sessions: &HashMap<String, String>) {
        for task in self.tasks.lock().unwrap().iter_mut() {
            if let Parent::Tab(tab) = &task.parent {
                if let Some(session) = sessions.get(tab) {
                    task.parent_session = Some(session.clone());
                }
            }
        }
        for report in self.reports.lock().unwrap().iter_mut() {
            if let Some(session) = sessions.get(&report.tab) {
                report.session = Some(session.clone());
            }
        }
        self.persist();
    }

    pub fn rebind(&self, sessions: &[(String, String)]) {
        let tab_for = |session: &str| sessions.iter().find(|(id, _)| id == session).map(|(_, tab)| tab.clone());
        for task in self.tasks.lock().unwrap().iter_mut() {
            let (old, session) = match (&task.parent, task.parent_session.clone()) {
                (Parent::Tab(old), Some(session)) => (old.clone(), session),
                _ => continue,
            };
            let Some(tab) = tab_for(&session) else { continue };
            if task.caller_key == format!("tab:{old}") {
                task.caller_key = format!("tab:{tab}");
            }
            task.parent = Parent::Tab(tab);
        }
        for report in self.reports.lock().unwrap().iter_mut() {
            let Some(session) = report.session.clone() else { continue };
            if let Some(tab) = tab_for(&session) {
                report.tab = tab;
            }
        }
        self.persist();
    }

    pub fn has_pending(&self) -> bool {
        let tasks = self.tasks.lock().unwrap();
        let finished = tasks.iter().any(|task| !task.delivered && task.state.terminal() && matches!(task.parent, Parent::Tab(_)));
        let reports = self.reports.lock().unwrap().iter().any(|report| !report.claimed && !stale(report, &tasks));
        finished || reports
    }

    fn adopt_restart(&self) -> bool {
        let mut changed = false;
        for task in self.tasks.lock().unwrap().iter_mut() {
            if task.state.terminal() {
                continue;
            }
            task.state = WorkState::Cancelled;
            task.summary = task.assistant.clone().or_else(|| task.summary.take());
            task.error = Some("stopped by restart".into());
            task.ended = Some(unix_millis());
            task.delivered = false;
            task.held = None;
            changed = true;
        }
        changed
    }

    fn persist(&self) {
        let Some(path) = self.store.clone() else { return };
        let saved = SavedFile {
            tasks: self.tasks.lock().unwrap().iter().map(SavedTask::from_task).collect(),
            reports: self.reports.lock().unwrap().clone(),
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(bytes) = serde_json::to_vec_pretty(&saved) else { return };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn lineage(&self, id: &str) -> Option<Lineage> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks.iter().find(|task| task.id == id)?;
        Some(Lineage {
            account: task.account.clone(),
            provider: task.provider,
            model: task.model.clone(),
            cwd: task.cwd.clone(),
            access: task.access,
        })
    }

    pub fn cards(&self, tab: &str) -> Vec<Subagent> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            .filter(|task| root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab))
            .map(|task| Subagent {
                id: task.id.clone(),
                tool: task.tool.clone().unwrap_or_default(),
                parent: match &task.parent {
                    Parent::Task(id) => Some(id.clone()),
                    Parent::Tab(_) => None,
                },
                name: task.title.clone(),
                kind: task.provider.spec().name.into(),
                model: task.model.clone(),
                state: card_state(task.state),
                updated: stamp(task.ended.unwrap_or(task.started)),
                started: Some(stamp(task.started)),
                error: task.error.clone(),
                conversation: task.conversation.clone(),
                steps: task.steps.clone(),
                undelivered: task.state.terminal() && !task.delivered && matches!(&task.parent, Parent::Tab(_)),
            })
            .collect()
    }

    pub fn steps_for(&self, tab: &str, tool: &str) -> Option<Vec<Item>> {
        if tool.is_empty() {
            return None;
        }
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            .find(|task| {
                task.tool.as_deref() == Some(tool)
                    && root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab)
            })
            .map(|task| task.steps.clone())
    }

    pub fn open(&self, caller: &Caller, id: &str) -> Result<OpenInfo, String> {
        let tasks = self.tasks.lock().unwrap();
        if !tasks.iter().any(|task| task.id == id) {
            return Err(format!("no task {id}"));
        }
        if !allows(&tasks, id, caller) {
            return Err(format!("task {id} belongs to another session"));
        }
        let task = tasks.iter().find(|task| task.id == id).unwrap();
        if !task.state.terminal() {
            return Err(format!("{id} is still working"));
        }
        let conversation = task
            .conversation
            .clone()
            .ok_or_else(|| format!("{id} has no conversation to resume"))?;
        Ok(OpenInfo {
            cwd: task.cwd.clone(),
            account: task.account.clone(),
            provider: task.provider,
            conversation,
            isolated: task.isolated,
            access: task.access,
        })
    }

    /// Drops isolated worktrees that still have a conversation and whose root tab is `tab`,
    /// except folders an opened tab already owns.
    pub fn drop_kept_worktrees(&self, tab: &str, owned: &[PathBuf], path: Option<&str>) {
        let cwds: Vec<PathBuf> = {
            let tasks = self.tasks.lock().unwrap();
            tasks
                .iter()
                .filter(|task| {
                    task.isolated
                        && task.conversation.is_some()
                        && root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab)
                        && !owned.iter().any(|dir| dir == &task.cwd)
                })
                .map(|task| task.cwd.clone())
                .collect()
        };
        for cwd in cwds {
            crate::branches::drop_worktree(&cwd, path);
        }
    }

    pub fn running_under(&self, tab: &str) -> Vec<String> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            .filter(|task| {
                !task.state.terminal()
                    && root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab)
            })
            .map(|task| task.id.clone())
            .collect()
    }

    pub fn running(&self) -> Vec<String> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .filter(|task| !task.state.terminal())
            .map(|task| task.id.clone())
            .collect()
    }

    /// Cached `--list-models` text. The first call waits up to 2s. Later calls return
    /// the cache, including a cached miss, and refresh a stale one in the background.
    pub fn cursor_catalog(&self, bin: &Path, path: Option<&str>) -> String {
        if self.cursor.lock().unwrap().is_none() {
            self.warm_cursor(bin, path);
            let start = Instant::now();
            while self.cursor.lock().unwrap().is_none() && start.elapsed() < super::CATALOG_WAIT {
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut cache = self.cursor.lock().unwrap();
            if cache.is_none() {
                *cache = Some(Cache { at: Instant::now(), list: String::new() });
            }
        }
        let cached = self.cursor.lock().unwrap().clone();
        let stale = match &cached {
            Some(cache) => {
                let fresh = if cache.list.trim().is_empty() { MODEL_MISS } else { MODEL_CACHE };
                cache.at.elapsed() >= fresh
            }
            None => true,
        };
        if stale {
            self.warm_cursor(bin, path);
        }
        cached.map(|cache| cache.list).unwrap_or_default()
    }

    pub fn cursor_ids(&self, bin: &Path, path: Option<&str>) -> Vec<String> {
        parse_model_list(&self.cursor_catalog(bin, path))
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    fn warm_cursor(&self, bin: &Path, path: Option<&str>) {
        if !bin.is_file() {
            *self.cursor.lock().unwrap() = Some(Cache { at: Instant::now(), list: String::new() });
            return;
        }
        if self.warming.swap(true, Ordering::Relaxed) {
            return;
        }
        let cursor = self.cursor.clone();
        let warming = self.warming.clone();
        let bin = bin.to_path_buf();
        let path = path.map(str::to_owned);
        std::thread::spawn(move || {
            let fresh = list_cursor_text(&bin, path.as_deref());
            let mut cache = cursor.lock().unwrap();
            let previous = cache.as_ref().map(|cache| cache.list.as_str());
            *cache = Some(Cache { at: Instant::now(), list: merge_cursor_list(previous, &fresh) });
            warming.store(false, Ordering::Relaxed);
        });
    }
}

pub struct Lineage {
    pub account: AccountId,
    pub provider: Provider,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub access: Access,
}

impl Task {
    fn note(&mut self, event: StreamEvent) -> Option<HeldExit> {
        if self.state != WorkState::Working {
            return None;
        }
        match event {
            StreamEvent::Conversation(id) => {
                self.conversation = Some(id);
                None
            }
            StreamEvent::TurnStart => {
                self.run.open_turns += 1;
                None
            }
            StreamEvent::Background(pending) => {
                self.run.background = pending;
                None
            }
            StreamEvent::Step(item) => {
                if let Item::Text { text, .. } = &item {
                    if !text.is_empty() {
                        self.assistant = Some(text.clone());
                    }
                }
                push_step(&mut self.steps, item);
                None
            }
            StreamEvent::TurnEnd { text, error } => {
                self.run.last_error = error.clone();
                let reply = Some(text).filter(|text| !text.is_empty()).or_else(|| self.assistant.clone());
                self.run.open_turns = self.run.open_turns.saturating_sub(1);
                if reply.is_some() {
                    self.summary = reply.clone();
                    if self.run.open_turns == 0 {
                        self.assistant = reply.clone();
                    }
                }
                if self.run.open_turns > 0 || !self.run.background.is_empty() {
                    return None;
                }
                Some(HeldExit { failed: error.is_some(), summary: reply, error })
            }
        }
    }

    fn quiet_after_reply(&self, now: Instant) -> bool {
        self.state == WorkState::Working
            && self.summary.is_some()
            && self.run.open_turns == 0
            && self.run.background.iter().all(|work| work.kind == SHELL_TASK)
            && millis(now.saturating_duration_since(self.run.last_line)) >= QUIET_AFTER_REPLY_MS
    }

    fn exited(&mut self, code: Option<i32>, stderr: String) -> Option<HeldExit> {
        if self.state != WorkState::Working {
            return None;
        }
        if code == Some(0) {
            if let Some(text) = self.assistant.clone() {
                return Some(HeldExit { summary: Some(text), error: None, failed: false });
            }
        }
        let error = if stderr.is_empty() {
            format!("the agent exited ({})", code.map_or("no status".into(), |code| code.to_string()))
        } else {
            stderr
        };
        Some(HeldExit { summary: self.assistant.clone(), error: Some(error), failed: true })
    }
}

/// A session hears back through a typed notice. A headless task has no terminal to type into, so it asks again.
fn working_next(task: &Task) -> String {
    match task.parent {
        Parent::Tab(_) => format!(
            "end your turn and do not poll; pinned delivers the notice when the turn ends or when this session is idle, or the user sends it from the task's card; then call task_status with this taskId \"{}\"",
            task.id
        ),
        Parent::Task(_) => format!("call task_status with taskId \"{}\" when you need it", task.id),
    }
}

fn next_line(task: &Task) -> String {
    match task.mode {
        Mode::Async => working_next(task),
        Mode::Wait => format!("call task_status with taskId \"{}\"", task.id),
    }
}

fn result_next(task: &Task) -> String {
    match task.state {
        WorkState::Failed | WorkState::Cancelled => "the error is in error".into(),
        WorkState::ResultAvailable => "the result is in summary".into(),
        WorkState::Working | WorkState::WaitingForChildren => next_line(task),
    }
}

fn status_json(task: &Task) -> Value {
    let end = task.ended.unwrap_or_else(unix_millis);
    let mut body = json!({
        "taskId": task.id,
        "title": task.title,
        "providerInstanceId": task.account,
        "model": task.model,
        "workState": task.state.as_str(),
        "summary": task.summary,
        "error": task.error,
        "recentSteps": task.steps,
        "elapsedSecs": end.saturating_sub(task.started) / 1000,
        "budgetSecs": task.run.budget_ms / 1000,
        "limitSecs": task.run.limit_ms / 1000,
        "background": task.run.background.iter().map(|work| work.description.as_str()).collect::<Vec<_>>(),
        "endedBy": task.ended_by,
        "conversationId": task.conversation,
        "cwd": task.cwd.display().to_string(),
    });
    body["next"] = if task.state.terminal() {
        json!(result_next(task))
    } else if task.summary.is_some() && !task.run.background.is_empty() {
        json!(format!(
            "it replied, and summary holds the reply so far; it still runs the work listed in background; pinned ends it when that work finishes and it replies again, or, when only shells are left, after {} with no activity; to stop it now, call task_cancel with taskId \"{}\"; otherwise {}",
            span(QUIET_AFTER_REPLY_MS),
            task.id,
            working_next(task)
        ))
    } else if task.run.told_late {
        json!(format!(
            "it is past its budget of {}, and pinned cancels it at its limit of {}; if recentSteps show it is done or stuck, call task_cancel with taskId \"{}\"; otherwise {}",
            span(task.run.budget_ms),
            span(task.run.limit_ms),
            task.id,
            working_next(task)
        ))
    } else {
        json!(working_next(task))
    };
    body
}

fn who(task: &Task) -> String {
    match &task.model {
        Some(model) => format!("{} · {model}", task.provider.spec().name),
        None => task.provider.spec().name.to_owned(),
    }
}

fn late_line(task: &Task, ran: u64) -> String {
    format!(
        "[pinned] Delegated task {} \"{}\" ({}) is still working after {}, past its budget of {}. pinned cancels it at its limit of {}. Call task_status with taskId \"{}\" to read its recent steps, or task_cancel to stop it.",
        task.id,
        task.title,
        who(task),
        span(ran),
        span(task.run.budget_ms),
        span(task.run.limit_ms),
        task.id
    )
}

fn pending_report(task: &mut Task) -> Option<Report> {
    if task.state != WorkState::Working || task.summary.is_none() || task.run.background.is_empty() || task.run.told_pending {
        return None;
    }
    task.run.told_pending = true;
    let Parent::Tab(tab) = &task.parent else { return None };
    let text = format!(
        "[pinned] Delegated task {} \"{}\" ({}) replied and still runs {}. pinned ends the task when that work finishes and the agent replies again, or, when only shells are left, after {} with no activity, and then stops what is still running. Call task_status with taskId \"{}\" to read the reply and the work, or task_cancel to stop it now.",
        task.id,
        task.title,
        who(task),
        many(task.run.background.len(), "background task"),
        span(QUIET_AFTER_REPLY_MS),
        task.id
    );
    Some(Report { tab: tab.clone(), session: task.parent_session.clone(), text, claimed: false, task: Some(task.id.clone()) })
}

const SHELL_TASK: &str = "local_bash";

fn stale(report: &Report, tasks: &[Task]) -> bool {
    report.task.as_ref().is_some_and(|id| tasks.iter().any(|task| &task.id == id && task.state.terminal()))
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn many(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        count => format!("{count} {noun}s"),
    }
}

fn span(ms: u64) -> String {
    let minutes = ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => format!("{}s", ms / 1000),
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

fn notice_line(task: &Task) -> String {
    let who = who(task);
    let outcome = match task.state {
        WorkState::Failed => format!("failed: {}", task.error.as_deref().unwrap_or("the agent failed")),
        WorkState::Cancelled => format!("cancelled: {}", task.error.as_deref().unwrap_or("cancelled")),
        _ => "result available".into(),
    };
    let note = task.ended_by.as_deref().map(|how| format!(" {how}.")).unwrap_or_default();
    format!("[pinned] Delegated task {} \"{}\" ({who}) finished: {outcome}.{note} Call task_status with taskId \"{}\".", task.id, task.title, task.id)
}

fn card_state(state: WorkState) -> subagents::State {
    match state {
        WorkState::Working | WorkState::WaitingForChildren => subagents::State::Running,
        WorkState::ResultAvailable => subagents::State::Done,
        WorkState::Failed => subagents::State::Failed,
        WorkState::Cancelled => subagents::State::Cancelled,
    }
}

fn root_tab(tasks: &[Task], mut parent: Parent) -> Option<String> {
    for _ in 0..32 {
        match parent {
            Parent::Tab(tab) => return Some(tab),
            Parent::Task(id) => parent = tasks.iter().find(|task| task.id == id)?.parent.clone(),
        }
    }
    None
}

fn list_cursor_text(bin: &Path, path: Option<&str>) -> String {
    if !bin.is_file() {
        return String::new();
    }
    let mut cmd = crate::process::command(bin);
    cmd.arg("--list-models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    output_within(cmd, Duration::from_secs(10))
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn output_within(mut cmd: std::process::Command, limit: Duration) -> Option<std::process::Output> {
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    };
    // try_wait reaps the child, so the stdout thread is what still holds the bytes.
    let stdout = reader.join().ok()?;
    Some(std::process::Output {
        status,
        stdout,
        stderr: vec![],
    })
}

fn fresh(new: NewTask) -> Task {
    use sha2::{Digest, Sha256};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(n.to_le_bytes());
    let dig = hasher.finalize();
    Task {
        id: format!("task-{:02x}{:02x}{:02x}{:02x}", dig[0], dig[1], dig[2], dig[3]),
        parent: new.parent,
        tool: new.tool,
        title: new.title,
        account: new.account,
        provider: new.provider,
        model: new.model,
        cwd: new.cwd,
        access: new.access,
        started: unix_millis(),
        ended: None,
        state: WorkState::Working,
        summary: None,
        error: None,
        conversation: None,
        steps: vec![],
        assistant: None,
        mode: new.mode,
        delivered: false,
        isolated: new.isolated,
        caller_key: new.caller_key,
        client_request_id: new.client_request_id,
        role: new.role,
        held: None,
        parent_session: None,
        ended_by: None,
        run: Run::default(),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedFile {
    tasks: Vec<SavedTask>,
    #[serde(default)]
    reports: Vec<Report>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedTask {
    id: String,
    parent_kind: String,
    parent_id: String,
    #[serde(default)]
    parent_session: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    title: String,
    account: AccountId,
    provider: Provider,
    #[serde(default)]
    model: Option<String>,
    cwd: PathBuf,
    access: String,
    started: u64,
    #[serde(default)]
    ended: Option<u64>,
    state: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    conversation: Option<String>,
    mode: Mode,
    #[serde(default)]
    delivered: bool,
    #[serde(default)]
    isolated: bool,
    caller_key: String,
    #[serde(default)]
    client_request_id: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    assistant: Option<String>,
    #[serde(default)]
    ended_by: Option<String>,
    #[serde(default = "default_budget")]
    budget_ms: u64,
    #[serde(default = "default_limit")]
    limit_ms: u64,
}

impl SavedTask {
    fn from_task(task: &Task) -> Self {
        let (parent_kind, parent_id) = match &task.parent {
            Parent::Tab(id) => ("tab", id.clone()),
            Parent::Task(id) => ("task", id.clone()),
        };
        Self {
            id: task.id.clone(),
            parent_kind: parent_kind.into(),
            parent_id,
            parent_session: task.parent_session.clone(),
            tool: task.tool.clone(),
            title: task.title.clone(),
            account: task.account.clone(),
            provider: task.provider,
            model: task.model.clone(),
            cwd: task.cwd.clone(),
            access: task.access.as_str().into(),
            started: task.started,
            ended: task.ended,
            state: task.state.as_str().into(),
            summary: task.summary.clone(),
            error: task.error.clone(),
            conversation: task.conversation.clone(),
            mode: task.mode,
            delivered: task.delivered,
            isolated: task.isolated,
            caller_key: task.caller_key.clone(),
            client_request_id: task.client_request_id.clone(),
            role: task.role.clone(),
            assistant: task.assistant.clone().filter(|_| !task.state.terminal()),
            ended_by: task.ended_by.clone(),
            budget_ms: task.run.budget_ms,
            limit_ms: task.run.limit_ms,
        }
    }

    fn into_task(self) -> Task {
        let parent = match self.parent_kind.as_str() {
            "task" => Parent::Task(self.parent_id),
            _ => Parent::Tab(self.parent_id),
        };
        Task {
            id: self.id,
            parent,
            tool: self.tool,
            title: self.title,
            account: self.account,
            provider: self.provider,
            model: self.model,
            cwd: self.cwd,
            access: access_from(&self.access),
            started: self.started,
            ended: self.ended,
            state: state_from(&self.state),
            summary: self.summary,
            error: self.error,
            conversation: self.conversation,
            steps: vec![],
            assistant: self.assistant,
            mode: self.mode,
            delivered: self.delivered,
            isolated: self.isolated,
            caller_key: self.caller_key,
            client_request_id: self.client_request_id,
            role: self.role,
            held: None,
            parent_session: self.parent_session,
            ended_by: self.ended_by,
            run: Run { budget_ms: self.budget_ms, limit_ms: self.limit_ms, ..Run::default() },
        }
    }
}

fn access_from(access: &str) -> Access {
    match access {
        "full" => Access::Full,
        "read-only" => Access::ReadOnly,
        _ => Access::Auto,
    }
}

fn state_from(state: &str) -> WorkState {
    match state {
        "working" => WorkState::Working,
        "waiting_for_children" => WorkState::WaitingForChildren,
        "failed" => WorkState::Failed,
        "cancelled" => WorkState::Cancelled,
        _ => WorkState::ResultAvailable,
    }
}

fn close(tasks: &mut [Task], index: usize, held: HeldExit) -> Vec<PathBuf> {
    settle(tasks, index, held);
    close_released(tasks, index)
}

fn close_released(tasks: &mut [Task], index: usize) -> Vec<PathBuf> {
    let mut dirs = release(tasks);
    if let Some(dir) = isolated_cwd(&tasks[index]) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

fn settle(tasks: &mut [Task], index: usize, held: HeldExit) {
    let id = tasks[index].id.clone();
    if has_working_descendant(tasks, &id) {
        tasks[index].held = Some(held);
        tasks[index].state = WorkState::WaitingForChildren;
        return;
    }
    tasks[index].summary = held.summary;
    tasks[index].error = held.error;
    tasks[index].state = if held.failed { WorkState::Failed } else { WorkState::ResultAvailable };
    tasks[index].ended = Some(unix_millis());
}

fn release(tasks: &mut [Task]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    loop {
        let waiting: Vec<String> = tasks.iter().filter(|task| task.state == WorkState::WaitingForChildren).map(|task| task.id.clone()).collect();
        let mut changed = false;
        for id in waiting {
            if has_working_descendant(tasks, &id) {
                continue;
            }
            let Some(index) = tasks.iter().position(|task| task.id == id) else { continue };
            let Some(held) = tasks[index].held.take() else { continue };
            tasks[index].summary = held.summary;
            tasks[index].error = held.error;
            tasks[index].state = if held.failed { WorkState::Failed } else { WorkState::ResultAvailable };
            tasks[index].ended = Some(unix_millis());
            if let Some(dir) = isolated_cwd(&tasks[index]) {
                dirs.push(dir);
            }
            changed = true;
        }
        if !changed {
            break;
        }
    }
    dirs
}

fn has_working_descendant(tasks: &[Task], id: &str) -> bool {
    tasks.iter().any(|task| task.id != id && !task.state.terminal() && descends(tasks, &task.id, id))
}

fn descends(tasks: &[Task], id: &str, ancestor: &str) -> bool {
    let mut current = id.to_owned();
    for _ in 0..32 {
        let Some(task) = tasks.iter().find(|task| task.id == current) else { return false };
        match &task.parent {
            Parent::Task(parent) if parent == ancestor => return true,
            Parent::Task(parent) => current = parent.clone(),
            Parent::Tab(_) => return false,
        }
    }
    false
}

fn chain_ready(tasks: &[Task], id: &str) -> bool {
    let mut current = Some(id.to_owned());
    while let Some(id) = current {
        let Some(task) = tasks.iter().find(|task| task.id == id) else { return false };
        if task.state.terminal() && !task.delivered {
            return true;
        }
        current = match &task.parent {
            Parent::Task(parent) => Some(parent.clone()),
            Parent::Tab(_) => None,
        };
    }
    false
}

fn allows(tasks: &[Task], id: &str, caller: &Caller) -> bool {
    let Some(task) = tasks.iter().find(|task| task.id == id) else { return false };
    if caller.key() == task.caller_key {
        return true;
    }
    let mut parent = task.parent.clone();
    for _ in 0..32 {
        match parent {
            Parent::Tab(tab) => return caller.tab.as_deref() == Some(tab.as_str()),
            Parent::Task(pid) => {
                if caller.task.as_deref() == Some(pid.as_str()) {
                    return true;
                }
                let Some(next) = tasks.iter().find(|task| task.id == pid) else { return false };
                parent = next.parent.clone();
            }
        }
    }
    false
}

fn postorder(tasks: &[Task], id: &str, out: &mut Vec<String>) {
    let children: Vec<String> = tasks
        .iter()
        .filter(|task| matches!(&task.parent, Parent::Task(parent) if parent == id) && !task.state.terminal())
        .map(|task| task.id.clone())
        .collect();
    for child in children {
        postorder(tasks, &child, out);
    }
    if tasks.iter().any(|task| task.id == id && !task.state.terminal()) {
        out.push(id.to_owned());
    }
}

/// An empty refresh must not wipe a catalog that already has models. The caller still
/// updates the cache time, so a miss does not refresh again on every read.
pub(crate) fn merge_cursor_list(previous: Option<&str>, fresh: &str) -> String {
    match previous {
        Some(previous) if fresh.trim().is_empty() && !previous.trim().is_empty() => previous.to_owned(),
        _ => fresh.to_owned(),
    }
}

fn isolated_cwd(task: &Task) -> Option<PathBuf> {
    (task.isolated && task.conversation.is_none() && task.state.terminal()).then(|| task.cwd.clone())
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn stamp(ms: u64) -> String {
    chrono::DateTime::from_timestamp((ms / 1000) as i64, ((ms % 1000) * 1_000_000) as u32)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}
