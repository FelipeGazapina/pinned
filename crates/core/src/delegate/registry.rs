use super::*;
use std::sync::atomic::AtomicU32;

#[derive(Clone)]
struct Cache {
    at: Instant,
    list: String,
}

pub struct Delegates {
    tasks: Mutex<Vec<Task>>,
    pids: Mutex<HashMap<String, u32>>,
    cursor: Arc<Mutex<Option<Cache>>>,
    warming: Arc<AtomicBool>,
}

impl Default for Delegates {
    fn default() -> Self {
        Self {
            tasks: Mutex::new(vec![]),
            pids: Mutex::new(HashMap::new()),
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
    }

    pub fn remember_pid(&self, id: &str, pid: u32) {
        self.pids.lock().unwrap().insert(id.to_owned(), pid);
    }

    pub fn fail(&self, id: &str, error: String, path: Option<&str>) {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.iter_mut().find(|task| task.id == id) {
            if !task.state.terminal() {
                task.state = WorkState::Failed;
                task.error = Some(error);
                task.ended = Some(unix_now());
                drop_isolated(task, path);
            }
        }
    }

    /// Applies one child event. Returns the parent tab when a task just became deliverable.
    pub fn apply(&self, id: &str, event: ChildEvent, path: Option<&str>) -> Option<String> {
        let mut tasks = self.tasks.lock().unwrap();
        let index = tasks.iter().position(|task| task.id == id)?;
        let held = match event {
            ChildEvent::Line(line) => {
                let provider = tasks[index].provider;
                let events = (driver(provider).parse)(&line);
                let mut held = None;
                for event in events {
                    if let Some(next) = tasks[index].note(event) {
                        held = Some(next);
                    }
                }
                held
            }
            ChildEvent::Exited { code, stderr } => tasks[index].exited(code, stderr),
        };
        if let Some(held) = held {
            settle(&mut tasks, index, held);
        }
        if tasks[index].state.terminal() {
            drop_isolated(&tasks[index], path);
        }
        release(&mut tasks, path);
        let id = tasks[index].id.clone();
        chain_ready(&tasks, &id).then(|| root_tab(&tasks, tasks[index].parent.clone())).flatten()
    }

    pub fn started_json(&self, id: &str) -> Result<Value, String> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| format!("no task {id}"))?;
        Ok(json!({
            "taskId": task.id,
            "title": task.title,
            "providerInstanceId": task.account,
            "model": task.model,
            "workState": task.state.as_str(),
            "cwd": task.cwd.display().to_string(),
            "next": next_line(task),
        }))
    }

    pub fn status(&self, caller: &Caller, id: &str) -> Result<Value, String> {
        let mut tasks = self.tasks.lock().unwrap();
        self.read(&mut tasks, caller, id)
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
        let order = {
            let tasks = self.tasks.lock().unwrap();
            if !tasks.iter().any(|task| task.id == id) {
                return Err(format!("no task {id}"));
            }
            let mut order = vec![];
            postorder(&tasks, id, &mut order);
            order
        };
        for id in &order {
            self.stop_one(id, reason, path);
        }
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks.iter_mut().find(|task| task.id == id).ok_or_else(|| format!("no task {id}"))?;
        if task.state.terminal() {
            task.delivered = true;
        }
        Ok(status_json(task))
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

    fn stop_one(&self, id: &str, reason: &str, path: Option<&str>) {
        let pid = {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.iter_mut().find(|task| task.id == id) else { return };
            if task.state.terminal() {
                return;
            }
            task.state = WorkState::Cancelled;
            task.error = Some(reason.to_owned());
            task.ended = Some(unix_now());
            drop_isolated(task, path);
            self.pids.lock().unwrap().remove(id)
        };
        if let Some(pid) = pid {
            kill(pid);
        }
    }

    /// Claims the notice for tasks whose direct parent is `tab`, and marks those tasks delivered.
    /// A grandchild stays out: its headless parent reads it, and it is never typed into the root tab.
    /// The caller puts the ids back with `release_notice` when the bytes were not written.
    pub fn claim_notice(&self, tab: &str) -> Option<(String, Vec<String>)> {
        let mut tasks = self.tasks.lock().unwrap();
        let ready: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                !task.delivered && task.state.terminal() && matches!(&task.parent, Parent::Tab(parent) if parent == tab)
            })
            .map(|(index, _)| index)
            .collect();
        if ready.is_empty() {
            return None;
        }
        let lines = ready.iter().map(|index| notice_line(&tasks[*index])).collect::<Vec<_>>().join("\n");
        let ids: Vec<String> = ready.iter().map(|index| tasks[*index].id.clone()).collect();
        for index in &ready {
            tasks[*index].delivered = true;
        }
        Some((lines, ids))
    }

    pub fn release_notice(&self, ids: &[String]) {
        let mut tasks = self.tasks.lock().unwrap();
        for task in tasks.iter_mut() {
            if ids.iter().any(|id| id == &task.id) {
                task.delivered = false;
            }
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
                steps: task.steps.clone(),
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
        let conversation = task
            .conversation
            .clone()
            .ok_or_else(|| format!("{id} has no conversation to resume"))?;
        Ok(OpenInfo {
            cwd: task.cwd.clone(),
            account: task.account.clone(),
            provider: task.provider,
            conversation,
        })
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

    /// Cached `--list-models` text. A stale or empty cache refreshes on a background thread and this returns what is already known.
    pub fn cursor_catalog(&self, bin: &Path, path: Option<&str>) -> String {
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
        if !bin.is_file() || self.warming.swap(true, Ordering::Relaxed) {
            return;
        }
        let cursor = self.cursor.clone();
        let warming = self.warming.clone();
        let bin = bin.to_path_buf();
        let path = path.map(str::to_owned);
        std::thread::spawn(move || {
            let list = list_cursor_text(&bin, path.as_deref());
            *cursor.lock().unwrap() = Some(Cache { at: Instant::now(), list });
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
            StreamEvent::Step(item) => {
                if let Item::Text { text, .. } = &item {
                    if !text.is_empty() {
                        self.assistant = Some(text.clone());
                    }
                }
                push_step(&mut self.steps, item);
                None
            }
            StreamEvent::Final { text, error } => Some(HeldExit {
                failed: error.is_some(),
                summary: Some(text),
                error,
            }),
        }
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

fn next_line(task: &Task) -> String {
    match task.mode {
        Mode::Async => "keep working; pinned will type a notice into this session when it finishes; then call task_status".into(),
        Mode::Wait => format!("call task_status with taskId \"{}\"", task.id),
    }
}

fn status_json(task: &Task) -> Value {
    let end = task.ended.unwrap_or_else(unix_now);
    json!({
        "taskId": task.id,
        "title": task.title,
        "providerInstanceId": task.account,
        "model": task.model,
        "workState": task.state.as_str(),
        "summary": task.summary,
        "error": task.error,
        "recentSteps": task.steps,
        "elapsedSecs": end.saturating_sub(task.started),
        "conversationId": task.conversation,
        "cwd": task.cwd.display().to_string(),
    })
}

fn notice_line(task: &Task) -> String {
    let who = match &task.model {
        Some(model) => format!("{} · {model}", task.provider.spec().name),
        None => task.provider.spec().name.to_owned(),
    };
    let outcome = match task.state {
        WorkState::Failed => format!("failed: {}", task.error.as_deref().unwrap_or("the agent failed")),
        WorkState::Cancelled => format!("cancelled: {}", task.error.as_deref().unwrap_or("cancelled")),
        _ => "result available".into(),
    };
    format!("[pinned] Delegated task {} \"{}\" ({who}) finished: {outcome}. Call task_status with taskId \"{}\".", task.id, task.title, task.id)
}

fn card_state(state: WorkState) -> subagents::State {
    match state {
        WorkState::Working | WorkState::WaitingForChildren => subagents::State::Running,
        WorkState::ResultAvailable => subagents::State::Done,
        WorkState::Failed | WorkState::Cancelled => subagents::State::Stopped,
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
        started: unix_now(),
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
    }
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
    tasks[index].ended = Some(unix_now());
}

fn release(tasks: &mut [Task], path: Option<&str>) {
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
            tasks[index].ended = Some(unix_now());
            drop_isolated(&tasks[index], path);
            changed = true;
        }
        if !changed {
            break;
        }
    }
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

fn drop_isolated(task: &Task, path: Option<&str>) {
    if task.isolated && task.conversation.is_none() && matches!(task.state, WorkState::Failed | WorkState::Cancelled) {
        crate::branches::drop_worktree(&task.cwd, path);
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn stamp(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|time| time.to_rfc3339())
        .unwrap_or_default()
}
