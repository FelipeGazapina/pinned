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
    /// The notices this machine took, oldest first: a hub that sends one again gets `true` and nothing is queued.
    notices: Mutex<Vec<String>>,
    store: Option<PathBuf>,
    cursor: Arc<Mutex<Option<Cache>>>,
    warming: Arc<AtomicBool>,
    /// The accounts a usage limit stopped, kept in `provider-limits.json` beside the tasks.
    limits: ProviderLimits,
}

/// A delegated task seen from the thread that runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadEnd {
    /// It ended: a result, a failure or a cancel.
    pub terminal: bool,
    /// Its result reached the session that delegated it.
    pub delivered: bool,
    /// Unix seconds of the later of its end and its delivery.
    pub at: u64,
}

/// How many taken notices a machine remembers: a hub resends only those decided in the last week.
const NOTICES_KEPT: usize = 4000;

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
    /// The orchestrator's `request_id` this text delivers, so the machine knows when it was typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request: Option<String>,
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
            notices: Mutex::new(vec![]),
            store: None,
            cursor: Arc::new(Mutex::new(None)),
            warming: Arc::new(AtomicBool::new(false)),
            limits: ProviderLimits::default(),
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
    /// This many workers already work in the task's project, the most it allows at once.
    Full(usize),
}

impl Delegates {
    pub fn load(file: Option<PathBuf>) -> Self {
        let limits = ProviderLimits::load(file.as_ref().map(|file| file.with_file_name("provider-limits.json")));
        let delegates = Self { store: file.clone(), limits, ..Self::default() };
        let Some(file) = file else { return delegates };
        let Ok(bytes) = std::fs::read(&file) else { return delegates };
        let Ok(saved) = serde_json::from_slice::<SavedFile>(&bytes) else { return delegates };
        *delegates.tasks.lock().unwrap() = saved.tasks.into_iter().map(SavedTask::into_task).collect();
        *delegates.reports.lock().unwrap() = saved.reports;
        *delegates.notices.lock().unwrap() = saved.notices;
        if delegates.adopt_restart() {
            delegates.persist();
        }
        delegates
    }

    /// How many tasks still at work run in `project`, the repository their folders belong to.
    /// Which accounts a usage limit stopped, and the trial that lets one back in.
    pub fn limits(&self) -> &ProviderLimits {
        &self.limits
    }

    /// Notes that a usage limit moved task `id` off the account and model its request named.
    pub fn set_moved(&self, id: &str, moved: Moved) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.moved = Some(moved);
        }
        self.persist();
    }

    /// What a task that just ended says about its account: a limit error marks it, a success that
    /// began after the mark clears it.
    fn observe(&self, id: &str, resume_at: Option<u64>) {
        let seen = self.tasks.lock().unwrap().iter().find(|task| task.id == id).map(|task| (task.account.clone(), task.state, task.error.clone(), task.started));
        match seen {
            Some((account, WorkState::Failed, Some(error), _)) => {
                if !self.limits.mark(&account, &error, resume_at) {
                    self.limits.release_trial(&account);
                }
            }
            Some((account, WorkState::Cancelled, ..)) => self.limits.release_trial(&account),
            Some((account, WorkState::ResultAvailable, _, started)) => {
                self.limits.clear(&account, started);
            }
            _ => {}
        }
    }

    pub fn working_in(&self, project: &Path) -> usize {
        self.tasks.lock().unwrap().iter().filter(|task| !task.state.terminal() && crate::history::project_of(&task.cwd) == project).count()
    }

    /// How many tasks the session `caller_key` (`Caller::key`) started are still at work.
    pub fn working_for(&self, caller_key: &str) -> usize {
        self.tasks.lock().unwrap().iter().filter(|task| !task.state.terminal() && task.caller_key == caller_key).count()
    }

    /// How each task whose pinned thread is `tab` stands, for the auto-archive rule. `at` is the
    /// later of the task's end and its delivery, 0 while neither is known.
    pub fn thread_ends(&self, tab: &str) -> Vec<ThreadEnd> {
        let tasks = self.tasks.lock().unwrap();
        let on_tab = tasks.iter().filter(|task| task.thread.as_ref().is_some_and(|thread| thread.tab == tab));
        on_tab.map(|task| ThreadEnd { terminal: task.state.terminal(), delivered: task.delivered, at: (task.ended.unwrap_or(0) / 1000).max(task.delivered_at.unwrap_or(0)) }).collect()
    }

    /// The folder of each task still at work.
    #[cfg(test)]
    pub fn working_cwds(&self) -> Vec<PathBuf> {
        self.tasks.lock().unwrap().iter().filter(|task| !task.state.terminal()).map(|task| task.cwd.clone()).collect()
    }

    /// Ties the task to the pinned thread that shows it.
    pub fn link_thread(&self, id: &str, thread: Thread) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.thread_tab = Some(thread.tab.clone());
            task.thread = Some(thread);
        }
    }

    pub fn set_conversation(&self, id: &str, conversation: String) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.conversation = Some(conversation);
        }
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.tasks.lock().unwrap().iter().find(|task| task.id == id).cloned()
    }

    /// Ends a task whose thread's session is the child: its turn ended with `summary`, or failed
    /// with `error`. Returns the parent tab when the notice is ready. The session stays open as
    /// the thread, so nothing is killed.
    pub fn settle_thread(&self, id: &str, summary: Option<String>, error: Option<String>, resume_at: Option<u64>, steps: Vec<Item>, path: Option<&str>) -> Option<String> {
        let (ready, dirs) = {
            let mut tasks = self.tasks.lock().unwrap();
            let index = tasks.iter().position(|task| task.id == id && task.state == WorkState::Working)?;
            self.forget_pid(id);
            if !steps.is_empty() {
                tasks[index].steps = steps;
            }
            let failed = error.is_some();
            let dirs = close(&mut tasks, index, HeldExit { summary, error, failed });
            let parent = tasks[index].parent.clone();
            (chain_ready(&tasks, id).then(|| root_tab(&tasks, parent)).flatten(), dirs)
        };
        for dir in dirs {
            crate::branches::drop_worktree(&dir, path);
        }
        self.observe(id, resume_at);
        self.persist();
        ready
    }

    /// The task the caller already started with this `clientRequestId`.
    pub fn requested(&self, caller_key: &str, request_id: Option<&str>) -> Option<String> {
        let request_id = request_id?;
        self.tasks.lock().unwrap().iter().find(|task| task.caller_key == caller_key && task.client_request_id.as_deref() == Some(request_id)).map(|task| task.id.clone())
    }

    /// The same caller and `clientRequestId` returns the task already started. Otherwise a new
    /// task starts unless `at_once` tasks already work in its project, counted under the same lock
    /// so two callers can't both take the last place.
    pub fn claim(&self, new: NewTask, at_once: Option<u32>) -> Claim {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(request_id) = new.client_request_id.as_deref() {
            if let Some(existing) = tasks.iter().find(|task| {
                task.caller_key == new.caller_key && task.client_request_id.as_deref() == Some(request_id)
            }) {
                return Claim::Existing(existing.id.clone());
            }
        }
        if let Some(most) = at_once {
            let project = crate::history::project_of(&new.cwd);
            let working = tasks.iter().filter(|task| !task.state.terminal() && crate::history::project_of(&task.cwd) == project).count();
            if working >= most as usize {
                return Claim::Full(working);
            }
        }
        let task = fresh(new);
        mark_redone(&mut tasks, &task);
        tasks.push(task.clone());
        drop(tasks);
        self.persist();
        Claim::Fresh(Box::new(task))
    }

    pub fn begin(&self, new: NewTask) -> Task {
        match self.claim(new, None) {
            Claim::Fresh(task) => *task,
            Claim::Existing(id) => self.tasks.lock().unwrap().iter().find(|task| task.id == id).unwrap().clone(),
            Claim::Full(_) => unreachable!("begin sets no limit"),
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

    /// The delegation limits the task runs under, which its own children inherit.
    pub fn hold_to(&self, id: &str, limits: crate::taste::Limits) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.limits = limits;
        }
        self.persist();
    }

    /// Who started the task and the effort it asked for, which its record keeps.
    pub fn describe(&self, id: &str, delegator: Delegator, effort: Option<String>) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            task.delegator = delegator;
            task.effort = effort;
        }
        self.persist();
    }

    /// Records what each working task used, and stops the ones past their budget.
    pub fn meter(&self, used: &[(String, crate::usage::TaskUsage)], path: Option<&str>) {
        let over: Vec<(String, String)> = {
            let mut tasks = self.tasks.lock().unwrap();
            used.iter()
                .filter_map(|(id, usage)| {
                    let task = tasks.iter_mut().find(|task| &task.id == id && task.state == WorkState::Working)?;
                    task.usage = *usage;
                    task.limits.over_budget(usage.cost_usd).map(|reason| (id.clone(), reason))
                })
                .collect()
        };
        for (id, reason) in &over {
            self.stop_for_limit(id, "budget", reason, path);
        }
        if !over.is_empty() {
            self.persist();
        }
    }

    /// Stops a task and its descendants because it passed `limit` (`time` or `budget`). Its record
    /// says it was stopped by that limit; the descendants read as cancelled with the same reason.
    pub fn stop_for_limit(&self, id: &str, limit: &str, reason: &str, path: Option<&str>) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.iter_mut().find(|task| task.id == id && !task.state.terminal()) else { return };
            task.limit = Some(limit.to_owned());
        }
        self.stop_tree(id, reason, path);
        self.persist();
    }

    /// The delegator's verdict on a finished task's result.
    pub fn review(&self, caller: &Caller, id: &str, verdict: Verdict, note: Option<String>) -> Result<Value, String> {
        let record = {
            let mut tasks = self.tasks.lock().unwrap();
            if !tasks.iter().any(|task| task.id == id) {
                return Err(format!("no task {id}"));
            }
            if !allows(&tasks, id, caller) {
                return Err(format!("task {id} belongs to another session"));
            }
            let task = tasks.iter_mut().find(|task| task.id == id).unwrap();
            if !task.state.terminal() {
                return Err(format!("{id} is still working; review it once it ends"));
            }
            task.review = Some(Review { verdict, auto: false, note: note.filter(|note| !note.trim().is_empty()), at: unix_millis() });
            task.recorded = false;
            record(task)
        };
        self.persist();
        Ok(json!({ "taskId": id, "review": record.and_then(|r| r.review) }))
    }

    /// Marks a finished task accepted from a signal, unless it already has a verdict. Says whether
    /// it changed the task.
    pub fn accept_auto(&self, id: &str, note: String) -> bool {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.iter_mut().find(|task| task.id == id && task.state.terminal() && task.review.is_none()) else { return false };
        task.review = Some(Review { verdict: Verdict::Accepted, auto: true, note: Some(note), at: unix_millis() });
        task.recorded = false;
        drop(tasks);
        self.persist();
        true
    }

    /// The record of every finished task.
    pub fn records(&self) -> Vec<TaskRecord> {
        self.tasks.lock().unwrap().iter().filter_map(record).collect()
    }

    /// The records the hub does not hold as they are now, at most `max`.
    pub fn unrecorded(&self, max: usize) -> Vec<TaskRecord> {
        self.tasks.lock().unwrap().iter().filter(|task| !task.recorded).filter_map(record).take(max).collect()
    }

    /// Notes that the hub took `sent`. A task that changed since keeps its turn for the next send.
    pub fn mark_recorded(&self, sent: &[TaskRecord]) {
        let mut changed = false;
        for task in self.tasks.lock().unwrap().iter_mut() {
            if sent.iter().any(|r| r.id == task.id && record(task).as_ref() == Some(r)) {
                task.recorded = true;
                changed = true;
            }
        }
        if changed {
            self.persist();
        }
    }

    /// Stops each task past its time limit and ends quiet ones. A task on another machine gets
    /// `REMOTE_GRACE` past its limit for that machine to report its end; the ones stopped here are
    /// returned as (tab, task, machine), for the machine to be told best-effort.
    pub fn enforce_limits(&self, now: Instant, path: Option<&str>) -> Vec<(String, String, RemoteTarget)> {
        let (late, over, quiet, elsewhere) = {
            let mut tasks = self.tasks.lock().unwrap();
            let mut late = vec![];
            let mut over = vec![];
            let mut quiet = vec![];
            let mut elsewhere = vec![];
            for task in tasks.iter_mut().filter(|task| !task.state.terminal()) {
                let ran = millis(now.saturating_duration_since(task.run.begun));
                if let (Some(remote), true) = (&task.remote, ran >= task.run.limit_ms) {
                    if ran >= task.run.limit_ms.saturating_add(millis(REMOTE_GRACE)) {
                        over.push((task.id.clone(), format!("pinned stopped it after {}, past its limit of {}: {} never reported its end", span(ran), span(task.run.limit_ms), remote.name)));
                        if let Parent::Tab(tab) = &task.parent {
                            elsewhere.push((tab.clone(), task.id.clone(), remote.clone()));
                        }
                    }
                    continue;
                }
                if ran >= task.run.limit_ms {
                    over.push((task.id.clone(), format!("pinned stopped it after {}, its limit", span(ran))));
                } else if task.quiet_after_reply(now) {
                    quiet.push(task.id.clone());
                } else if ran >= task.run.budget_ms && !task.run.told_late {
                    task.run.told_late = true;
                    if let (Parent::Tab(tab), true) = (&task.parent, announces(task)) {
                        late.push(Report { tab: tab.clone(), session: task.parent_session.clone(), text: late_line(task, ran), claimed: false, task: Some(task.id.clone()), request: None });
                    }
                }
            }
            (late, over, quiet, elsewhere)
        };
        for report in late {
            self.push_report(report);
        }
        for (id, reason) in &over {
            self.stop_for_limit(id, "time", reason, path);
        }
        for id in &quiet {
            self.end_quiet(id, path);
        }
        if !over.is_empty() || !quiet.is_empty() {
            self.persist();
        }
        elsewhere
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

    /// Each working task on another machine that no one asked after for `every`, with its tab and
    /// how long it ran, marked asked now.
    pub fn remotes_due(&self, now: Instant, every: Duration) -> Vec<(String, String, RemoteTarget, Duration)> {
        let mut tasks = self.tasks.lock().unwrap();
        let mut due = vec![];
        for task in tasks.iter_mut().filter(|task| !task.state.terminal()) {
            let (Parent::Tab(tab), Some(remote)) = (&task.parent, task.remote.as_mut()) else { continue };
            if remote.machine.is_empty() || remote.looked.is_some_and(|at| now.saturating_duration_since(at) < every) {
                continue;
            }
            remote.looked = Some(now);
            due.push((tab.clone(), task.id.clone(), remote.clone(), now.saturating_duration_since(task.run.begun)));
        }
        due
    }

    /// Notes the machine that runs the task. A later answer that names the thread there fills it in.
    pub fn set_remote(&self, id: &str, remote: RemoteTarget) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id) {
            if let (Parent::Tab(tab), None) = (&task.parent, &task.remote_tab) {
                task.remote_tab = Some(tab.clone());
            }
            task.remote = Some(remote);
        }
        self.persist();
    }

    /// The remote task's status as the machine running it last answered, when it is time to ask
    /// again: never more often than `every`, so a waiting `delegate_task` does not flood the hub.
    pub fn remote_due(&self, id: &str, now: Instant, every: Duration) -> Option<RemoteTarget> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks.iter_mut().find(|task| task.id == id && !task.state.terminal())?;
        let remote = task.remote.as_mut()?;
        if remote.looked.is_some_and(|at| now.saturating_duration_since(at) < every) {
            return None;
        }
        remote.looked = Some(now);
        Some(remote.clone())
    }

    /// Ends a task that runs on another machine as that machine reports it. Only a working task
    /// that `tab` owns, by the id it has now or the one it was delegated from, and `from` runs (or
    /// that no machine is known to run yet) takes it. Returns the tab that owns it now when it took it.
    pub fn settle_remote(&self, id: &str, tab: &str, from: &str, end: &RemoteEnd) -> Option<String> {
        let parent = {
            let mut tasks = self.tasks.lock().unwrap();
            let task = tasks.iter_mut().find(|task| task.id == id)?;
            let Parent::Tab(parent) = &task.parent else { return None };
            let owned = parent == tab || task.remote_tab.as_deref() == Some(tab);
            // A task the hub is still starting has no machine yet: its first word is from the machine that runs it.
            if !owned || task.state.terminal() || task.remote.as_ref().is_none_or(|remote| !remote.machine.is_empty() && remote.machine != from) {
                return None;
            }
            let parent = parent.clone();
            task.state = match end.state.as_str() {
                "failed" => WorkState::Failed,
                "cancelled" => WorkState::Cancelled,
                _ => WorkState::ResultAvailable,
            };
            task.summary = end.summary.clone();
            task.error = end.error.clone();
            task.ended = Some(unix_millis());
            if let Some(usage) = end.usage {
                task.usage = usage;
            }
            parent
        };
        self.persist();
        Some(parent)
    }

    /// Shows what the worker of a remote task has used so far, as the machine that runs it
    /// reported. Nothing is recorded from it: that machine records the run.
    pub fn note_remote_usage(&self, id: &str, usage: TaskUsage) {
        if let Some(task) = self.tasks.lock().unwrap().iter_mut().find(|task| task.id == id && !task.state.terminal() && task.remote.is_some()) {
            task.usage = usage;
        }
    }

    /// The end of each finished task that a tab on another machine delegated here and whose end
    /// that machine has not taken yet: this task, that machine, its tab, its own id for the task
    /// and how it ended. `remote_end_sent` marks one taken; until then it stays on disk, across
    /// restarts, to be sent again.
    pub fn remote_ends(&self) -> Vec<(String, String, String, String, RemoteEnd)> {
        let tasks = self.tasks.lock().unwrap();
        let mut ends = vec![];
        for task in tasks.iter().filter(|task| task.state.terminal() && !task.delivered) {
            let Parent::Tab(key) = &task.parent else { continue };
            let Some((machine, tab)) = Origin::of_key(key) else { continue };
            let Some(origin_task) = task.client_request_id.clone() else { continue };
            let usage = TaskUsage { wall_ms: task.ended.unwrap_or(task.started).saturating_sub(task.started), ..task.usage };
            let end = RemoteEnd { state: task.state.as_str().to_owned(), summary: task.summary.clone(), error: task.error.clone(), usage: Some(usage) };
            ends.push((task.id.clone(), machine.to_owned(), tab.to_owned(), origin_task, end));
        }
        ends
    }

    /// The hub passed task `id`'s end back to the machine that delegated it.
    pub fn remote_end_sent(&self, id: &str) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.iter_mut().find(|task| task.id == id && !task.delivered) else { return };
            task.deliver();
        }
        self.persist();
    }

    /// The task started for `origin_task` of the tab on another machine that `parent_key` names.
    pub fn for_origin(&self, parent_key: &str, origin_task: &str) -> Option<String> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks.iter().find(|task| matches!(&task.parent, Parent::Tab(key) if key == parent_key) && task.client_request_id.as_deref() == Some(origin_task))?;
        Some(task.id.clone())
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
        let (ready, dirs, stop, pending, ended) = {
            let mut tasks = self.tasks.lock().unwrap();
            let index = tasks.iter().position(|task| task.id == id)?;
            let was_working = !tasks[index].state.terminal();
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
            (ready, dirs, stop, pending, was_working && tasks[index].state.terminal())
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
        if ended {
            self.observe(id, None);
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
                task.deliver();
                let mut body = status_json(task);
                body["next"] = json!(result_next(task));
                body
            } else {
                let mut body = json!({
                    "taskId": task.id,
                    "title": task.title,
                    "providerInstanceId": task.account,
                    "model": task.model,
                    "workState": task.state.as_str(),
                    "cwd": task.cwd.display().to_string(),
                    "tabId": task.thread.as_ref().map(|thread| thread.tab.clone()),
                    "next": next_line(task),
                });
                with_moved(&mut body, task);
                body
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
                task.deliver();
            }
            status_json(task)
        };
        self.persist();
        Ok(body)
    }

    /// Ends the working tasks that cannot outlive this process, so the next launch finds them
    /// ended with a reason. A thread's worker whose session is saved stays working: the next
    /// launch resumes it. So does a task that runs on another machine.
    pub fn stop_for_restart(&self, path: Option<&str>) {
        let order: Vec<String> = {
            let tasks = self.tasks.lock().unwrap();
            let mut order = vec![];
            for task in tasks.iter().filter(|task| !task.state.terminal() && matches!(task.parent, Parent::Tab(_))) {
                postorder(&tasks, &task.id, &mut order);
            }
            order
        };
        for id in order.into_iter().chain(self.running()) {
            if !self.outlives_restart(&id) {
                self.stop_one(&id, RESTART_ENDED, path);
            }
        }
        self.persist();
    }

    fn outlives_restart(&self, id: &str) -> bool {
        let tasks = self.tasks.lock().unwrap();
        tasks.iter().find(|task| task.id == id).is_some_and(|task| {
            task.state == WorkState::Working
                && (task.thread.as_ref().is_some_and(|thread| thread.runs) && task.worker_session.is_some()
                    || task.remote.as_ref().is_some_and(|remote| !remote.machine.is_empty()))
        })
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
            task.deliver();
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
            task.summary = task.newest_text.clone().or_else(|| task.summary.take());
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
                !task.delivered && task.state.terminal() && announces(task) && matches!(&task.parent, Parent::Tab(parent) if parent == tab)
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
            tasks[*index].deliver();
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
        task.deliver();
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
        self.push_report(Report { tab: tab.to_owned(), session, text, claimed: false, task: None, request: None });
    }

    /// Queues a message from the global orchestrator, typed with the tab's other news once it is idle.
    pub fn queue_request(&self, tab: &str, session: Option<String>, text: String, request: String) {
        self.push_report(Report { tab: tab.to_owned(), session, text, claimed: false, task: None, request: Some(request) });
    }

    /// Whether the notice `key` was already taken, queued or typed.
    pub fn took_notice(&self, key: &str) -> bool {
        self.notices.lock().unwrap().iter().any(|taken| taken == key)
    }

    /// Queues a notice for `tab` and remembers `key`, across a restart, so a notice sent again is
    /// queued no more, whether it still waits or was typed long ago. False when `key` was taken.
    pub fn queue_notice(&self, key: &str, tab: &str, session: Option<String>, text: String) -> bool {
        {
            let mut notices = self.notices.lock().unwrap();
            if notices.iter().any(|taken| taken == key) {
                return false;
            }
            notices.push(key.to_owned());
            if notices.len() > NOTICES_KEPT {
                let extra = notices.len() - NOTICES_KEPT;
                notices.drain(..extra);
            }
        }
        self.queue_report(tab, session, text);
        true
    }

    fn push_report(&self, report: Report) {
        let session = report.session.filter(|session| !session.is_empty());
        self.reports.lock().unwrap().push(Report { session, ..report });
        self.persist();
    }

    /// How many messages wait to be typed into each tab: worker news and orchestrator requests.
    pub fn queued(&self) -> HashMap<String, u32> {
        let mut by_tab = HashMap::new();
        for report in self.reports.lock().unwrap().iter() {
            *by_tab.entry(report.tab.clone()).or_default() += 1;
        }
        by_tab
    }

    /// Drops the tab's reports that were typed, and returns the orchestrator requests among them.
    pub fn drop_claimed_reports(&self, tab: &str) -> Vec<String> {
        let mut typed = vec![];
        self.reports.lock().unwrap().retain(|report| {
            let drop = report.tab == tab && report.claimed;
            if drop {
                typed.extend(report.request.clone());
            }
            !drop
        });
        self.persist();
        typed
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
        let finished = tasks.iter().any(|task| !task.delivered && task.state.terminal() && announces(task) && matches!(task.parent, Parent::Tab(_)));
        let reports = self.reports.lock().unwrap().iter().any(|report| !report.claimed && !stale(report, &tasks));
        finished || reports
    }

    /// Ends each working task that a restart left nothing to carry on. A thread's worker keeps
    /// working for `restore_tabs` to bring back, and a task on another machine keeps working there.
    fn adopt_restart(&self) -> bool {
        let mut changed = false;
        for task in self.tasks.lock().unwrap().iter_mut() {
            if task.state.terminal() {
                continue;
            }
            let carried_on = task.state == WorkState::Working && task.remote.as_ref().is_some_and(|remote| !remote.machine.is_empty());
            if (task.resuming && task.worker_session.is_some()) || carried_on {
                continue;
            }
            end_after_restart(task);
            changed = true;
        }
        changed
    }

    /// Ties each task that was loaded as a thread's worker to the restored tab that has its
    /// session, whatever id that tab got this time. Returns (task, tab) for each.
    pub fn resume_workers(&self, sessions: &[(String, String)]) -> Vec<(String, String)> {
        let mut resumed = vec![];
        for task in self.tasks.lock().unwrap().iter_mut().filter(|task| task.resuming && !task.state.terminal()) {
            let Some(tab) = task.worker_session.as_ref().and_then(|session| sessions.iter().find(|(id, _)| id == session)).map(|(_, tab)| tab.clone()) else { continue };
            task.resuming = false;
            task.thread_tab = Some(tab.clone());
            task.thread = Some(Thread { tab: tab.clone(), runs: true });
            resumed.push((task.id.clone(), tab));
        }
        if !resumed.is_empty() {
            self.persist();
        }
        resumed
    }

    /// Ends the tasks that were to resume but whose worker session no restored tab has.
    pub fn end_unresumed(&self) {
        let mut changed = false;
        for task in self.tasks.lock().unwrap().iter_mut().filter(|task| task.resuming) {
            if !task.state.terminal() {
                end_after_restart(task);
                changed = true;
            }
            task.resuming = false;
        }
        if changed {
            self.persist();
        }
    }

    /// Notes the Claude session of the thread that runs the task, for a restart to resume.
    pub fn set_worker_session(&self, id: &str, session: &str) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.iter_mut().find(|task| task.id == id && !task.state.terminal()) else { return };
            if task.worker_session.as_deref() == Some(session) {
                return;
            }
            task.worker_session = Some(session.to_owned());
        }
        self.persist();
    }

    /// The tab id to name the task's parent by to the machine that runs it: the one it was
    /// delegated from, which a restart here does not change over there.
    pub fn remote_tab(&self, id: &str, current: &str) -> String {
        let tasks = self.tasks.lock().unwrap();
        tasks.iter().find(|task| task.id == id).and_then(|task| task.remote_tab.clone()).unwrap_or_else(|| current.to_owned())
    }

    fn persist(&self) {
        let Some(path) = self.store.clone() else { return };
        let saved = SavedFile {
            tasks: self.tasks.lock().unwrap().iter().map(SavedTask::from_task).collect(),
            reports: self.reports.lock().unwrap().clone(),
            notices: self.notices.lock().unwrap().clone(),
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
            limits: task.limits,
        })
    }

    pub fn cards(&self, tab: &str) -> Vec<Subagent> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            // A task on another machine shows as its thread there, nested under this tab.
            .filter(|task| task.remote.is_none())
            .filter(|task| match &task.thread {
                Some(thread) => !thread.runs && thread.tab == tab,
                None => root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab),
            })
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
                undelivered: task.state.terminal() && !task.delivered && matches!(&task.parent, Parent::Tab(_)) && task.thread.is_none(),
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
                    && task.thread.is_none()
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
    pub limits: crate::taste::Limits,
}

impl Task {
    fn deliver(&mut self) {
        self.delivered = true;
        self.delivered_at = Some(unix_millis() / 1000);
    }

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
            StreamEvent::Usage(report) => {
                self.usage.tokens_in += report.tokens_in;
                self.usage.tokens_out += report.tokens_out;
                if report.cost_usd.is_some() {
                    self.usage.cost_usd = report.cost_usd;
                }
                None
            }
            StreamEvent::Step(item) => {
                if let Item::Text { text, .. } = &item {
                    if !text.is_empty() {
                        self.newest_text = Some(text.clone());
                    }
                }
                push_step(&mut self.steps, item);
                None
            }
            StreamEvent::TurnEnd { text, error } => {
                self.run.last_error = error.clone();
                let reply = Some(text).filter(|text| !text.is_empty()).or_else(|| self.newest_text.clone());
                self.run.open_turns = self.run.open_turns.saturating_sub(1);
                if reply.is_some() {
                    self.summary = reply.clone();
                    if self.run.open_turns == 0 {
                        self.newest_text = reply.clone();
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
            if let Some(text) = self.newest_text.clone() {
                return Some(HeldExit { summary: Some(text), error: None, failed: false });
            }
        }
        let error = if stderr.is_empty() {
            format!("the agent exited ({})", code.map_or("no status".into(), |code| code.to_string()))
        } else {
            stderr
        };
        Some(HeldExit { summary: self.newest_text.clone(), error: Some(error), failed: true })
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
        "usage": crate::usage::TaskUsage { wall_ms: end.saturating_sub(task.started), ..task.usage },
        "background": task.run.background.iter().map(|work| work.description.as_str()).collect::<Vec<_>>(),
        "endedBy": task.ended_by,
        "conversationId": task.conversation,
        "cwd": task.cwd.display().to_string(),
        "tabId": task.thread.as_ref().map(|thread| thread.tab.clone()),
    });
    if let Some(limit) = &task.limit {
        body["stoppedByLimit"] = json!(limit);
    }
    with_moved(&mut body, task);
    if let Some(review) = &task.review {
        body["review"] = json!(review);
    }
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

/// Names where a task was asked to run, when a usage limit moved it. `providerInstanceId` and
/// `model` already say where it runs.
fn with_moved(body: &mut Value, task: &Task) {
    if let Some(moved) = &task.moved {
        body["movedFrom"] = json!({ "providerInstanceId": moved.account, "model": moved.model, "reason": moved.reason });
    }
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
    if !announces(task) {
        return None;
    }
    let text = format!(
        "[pinned] Delegated task {} \"{}\" ({}) replied and still runs {}. pinned ends the task when that work finishes and the agent replies again, or, when only shells are left, after {} with no activity, and then stops what is still running. Call task_status with taskId \"{}\" to read the reply and the work, or task_cancel to stop it now.",
        task.id,
        task.title,
        who(task),
        many(task.run.background.len(), "background task"),
        span(QUIET_AFTER_REPLY_MS),
        task.id
    );
    Some(Report { tab: tab.clone(), session: task.parent_session.clone(), text, claimed: false, task: Some(task.id.clone()), request: None })
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
    let thread = match (&task.remote, &task.thread) {
        (Some(remote), _) => format!(" It ran on {}{}.", remote.name, remote.tab.as_deref().map(|tab| format!(", in thread {tab}")).unwrap_or_default()),
        (None, Some(thread)) => format!(" Its thread is {}.", thread.tab),
        (None, None) => String::new(),
    };
    format!("[pinned] Delegated task {} \"{}\" ({who}) finished: {outcome}.{note}{thread} Call task_status with taskId \"{}\".", task.id, task.title, task.id)
}

fn card_state(state: WorkState) -> subagents::State {
    match state {
        WorkState::Working | WorkState::WaitingForChildren => subagents::State::Running,
        WorkState::ResultAvailable => subagents::State::Done,
        WorkState::Failed => subagents::State::Failed,
        WorkState::Cancelled => subagents::State::Cancelled,
    }
}

/// Whether the task's end and progress are told to its parent. A task whose parent is the very
/// tab that shows it, as the user's Run now of a workflow, has no other session waiting on it.
fn announces(task: &Task) -> bool {
    !matches!((&task.parent, &task.thread_tab), (Parent::Tab(parent), Some(thread)) if parent == thread)
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
        newest_text: None,
        mode: new.mode,
        delivered: false,
        delivered_at: None,
        isolated: new.isolated,
        caller_key: new.caller_key,
        client_request_id: new.client_request_id,
        role: new.role,
        held: None,
        parent_session: None,
        ended_by: None,
        run: Run::default(),
        thread: None,
        worker_session: None,
        resuming: false,
        remote: None,
        remote_tab: None,
        limits: crate::taste::Limits::default(),
        usage: crate::usage::TaskUsage::default(),
        delegator: Delegator::default(),
        effort: None,
        limit: None,
        moved: None,
        review: None,
        thread_tab: None,
        recorded: false,
    }
}

/// The record a finished task leaves. A working one has none yet, and one that ran on another
/// machine leaves its record there.
fn record(task: &Task) -> Option<TaskRecord> {
    if !task.state.terminal() || task.remote.is_some() {
        return None;
    }
    let for_elsewhere = matches!(&task.parent, Parent::Tab(key) if Origin::of_key(key).is_some());
    let ended = task.ended.unwrap_or(task.started);
    let outcome = match (task.state, &task.limit) {
        (WorkState::Failed, _) => TaskOutcome::Failed,
        (WorkState::Cancelled, Some(_)) => TaskOutcome::Limit,
        (WorkState::Cancelled, None) => TaskOutcome::Cancelled,
        _ => TaskOutcome::Done,
    };
    Some(TaskRecord {
        id: task.id.clone(),
        machine: String::new(),
        machine_name: String::new(),
        title: task.title.clone(),
        delegator: task.delegator.clone(),
        provider: task.provider,
        account: task.account.clone(),
        model: task.model.clone(),
        tier: task.model.as_deref().map(|model| cost_of(model).name().to_owned()),
        effort: task.effort.clone(),
        target: if for_elsewhere { RunTarget::Machine } else { RunTarget::Local },
        started: task.started,
        ended,
        usage: TaskUsage { wall_ms: ended.saturating_sub(task.started), ..task.usage },
        outcome,
        reason: if outcome == TaskOutcome::Done { None } else { task.error.clone() },
        limit: task.limit.clone().filter(|_| outcome == TaskOutcome::Limit),
        asked_account: task.moved.as_ref().map(|moved| moved.account.clone()),
        asked_model: task.moved.as_ref().and_then(|moved| moved.model.clone()),
        moved_because: task.moved.as_ref().map(|moved| moved.reason.clone()),
        review: task.review.clone(),
        thread: task.thread_tab.clone(),
    })
}

/// A task that starts again under the same title, from the same caller, means the earlier one's
/// result was not used: the newest earlier one that ended and has no verdict yet is redone.
fn mark_redone(tasks: &mut [Task], fresh: &Task) {
    let title = fresh.title.trim().to_lowercase();
    let earlier = tasks
        .iter_mut()
        .filter(|task| task.caller_key == fresh.caller_key && task.state.terminal() && task.review.is_none() && task.title.trim().to_lowercase() == title)
        .max_by_key(|task| task.started);
    if let Some(task) = earlier {
        task.review = Some(Review { verdict: Verdict::Redone, auto: true, note: Some(format!("started again as {}", fresh.id)), at: unix_millis() });
        task.recorded = false;
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedFile {
    tasks: Vec<SavedTask>,
    #[serde(default)]
    reports: Vec<Report>,
    #[serde(default)]
    notices: Vec<String>,
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
    delivered_at: Option<u64>,
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
    #[serde(default)]
    limits: crate::taste::Limits,
    #[serde(default)]
    usage: crate::usage::TaskUsage,
    #[serde(default)]
    delegator: Delegator,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    moved: Option<Moved>,
    #[serde(default)]
    review: Option<Review>,
    #[serde(default)]
    thread_tab: Option<String>,
    /// A task saved before records existed has nothing worth sending, so it counts as sent.
    #[serde(default = "already_recorded")]
    recorded: bool,
    /// The task's own session runs it in a thread, which a restart resumes.
    #[serde(default)]
    thread_runs: bool,
    #[serde(default)]
    worker_session: Option<String>,
    #[serde(default)]
    remote: Option<SavedRemote>,
    #[serde(default)]
    remote_tab: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SavedRemote {
    machine: String,
    name: String,
    #[serde(default)]
    tab: Option<String>,
}

/// When a task that started at `started` (unix milliseconds) began, on this process's clock, so
/// its time limit keeps counting from the original start after a restart.
fn began(started: u64) -> Instant {
    let now = Instant::now();
    now.checked_sub(Duration::from_millis(unix_millis().saturating_sub(started))).unwrap_or(now)
}

/// Why a restart ended a task.
pub const RESTART_ENDED: &str = "pinned restarted and the worker could not resume";

fn end_after_restart(task: &mut Task) {
    task.resuming = false;
    task.state = WorkState::Cancelled;
    task.summary = task.newest_text.clone().or_else(|| task.summary.take());
    task.error = Some(match &task.remote {
        Some(remote) if remote.machine.is_empty() => format!("pinned restarted before the task reached {}", remote.name),
        _ => RESTART_ENDED.into(),
    });
    task.ended = Some(unix_millis());
    task.delivered = false;
    task.held = None;
}

fn already_recorded() -> bool {
    true
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
            delivered_at: task.delivered_at,
            isolated: task.isolated,
            caller_key: task.caller_key.clone(),
            client_request_id: task.client_request_id.clone(),
            role: task.role.clone(),
            assistant: task.newest_text.clone().filter(|_| !task.state.terminal()),
            ended_by: task.ended_by.clone(),
            budget_ms: task.run.budget_ms,
            limit_ms: task.run.limit_ms,
            limits: task.limits,
            usage: task.usage,
            delegator: task.delegator.clone(),
            effort: task.effort.clone(),
            limit: task.limit.clone(),
            moved: task.moved.clone(),
            review: task.review.clone(),
            thread_tab: task.thread_tab.clone(),
            recorded: task.recorded,
            thread_runs: task.thread.as_ref().is_some_and(|thread| thread.runs) || task.resuming,
            worker_session: task.worker_session.clone(),
            remote: task.remote.as_ref().map(|remote| SavedRemote { machine: remote.machine.clone(), name: remote.name.clone(), tab: remote.tab.clone() }),
            remote_tab: task.remote_tab.clone(),
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
            newest_text: self.assistant,
            mode: self.mode,
            delivered: self.delivered,
            delivered_at: self.delivered_at,
            isolated: self.isolated,
            caller_key: self.caller_key,
            client_request_id: self.client_request_id,
            role: self.role,
            held: None,
            parent_session: self.parent_session,
            ended_by: self.ended_by,
            run: Run { begun: began(self.started), budget_ms: self.budget_ms, limit_ms: self.limit_ms, ..Run::default() },
            thread: None,
            worker_session: self.worker_session,
            resuming: self.thread_runs && self.state == "working",
            remote: self.remote.map(|remote| RemoteTarget { machine: remote.machine, name: remote.name, tab: remote.tab, looked: None }),
            remote_tab: self.remote_tab,
            limits: self.limits,
            usage: self.usage,
            delegator: self.delegator,
            effort: self.effort,
            limit: self.limit,
            moved: self.moved,
            review: self.review,
            thread_tab: self.thread_tab,
            recorded: self.recorded,
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
    if caller.key() == task.caller_key || task.thread.as_ref().is_some_and(|thread| caller.tab.as_deref() == Some(thread.tab.as_str())) {
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
