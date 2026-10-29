use super::*;

struct Cache {
    at: Instant,
    list: String,
}

pub struct Delegates {
    next: AtomicU32,
    tasks: Mutex<Vec<Task>>,
    pids: Mutex<HashMap<String, u32>>,
    cursor: Mutex<Option<Cache>>,
}

impl Default for Delegates {
    fn default() -> Self {
        Self {
            next: AtomicU32::new(1),
            tasks: Mutex::new(vec![]),
            pids: Mutex::new(HashMap::new()),
            cursor: Mutex::new(None),
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
}

impl Delegates {
    pub fn begin(&self, new: NewTask) -> Task {
        let id = format!("task-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let task = Task {
            id,
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
            mode: new.mode,
            delivered: false,
        };
        self.tasks.lock().unwrap().push(task.clone());
        task
    }

    pub fn remember_pid(&self, id: &str, pid: u32) {
        self.pids.lock().unwrap().insert(id.to_owned(), pid);
    }

    pub fn fail(&self, id: &str, error: String) {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.iter_mut().find(|task| task.id == id) {
            if task.state == WorkState::Working {
                task.state = WorkState::Failed;
                task.error = Some(error);
                task.ended = Some(unix_now());
            }
        }
    }

    /// Applies one child event. Returns the parent tab when an async task just finished.
    pub fn apply(&self, id: &str, event: ChildEvent) -> Option<String> {
        let mut tasks = self.tasks.lock().unwrap();
        let index = tasks.iter().position(|task| task.id == id)?;
        let became = match event {
            ChildEvent::Line(line) => {
                let provider = tasks[index].provider;
                let events = (driver(provider).parse)(&line);
                let mut became = false;
                for event in events {
                    became |= tasks[index].note(event);
                }
                became
            }
            ChildEvent::Exited { code, stderr } => tasks[index].exited(code, stderr),
        };
        (became && tasks[index].mode == Mode::Async)
            .then(|| root_tab(&tasks, tasks[index].parent.clone()))
            .flatten()
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

    pub fn status(&self, id: &str) -> Result<Value, String> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or_else(|| format!("no task {id}"))?;
        if task.state.terminal() {
            task.delivered = true;
        }
        Ok(status_json(task))
    }

    pub fn cancel(&self, id: &str, reason: &str) -> Result<Value, String> {
        let pid = {
            let mut tasks = self.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or_else(|| format!("no task {id}"))?;
            if task.state.terminal() {
                return Ok(status_json(task));
            }
            task.state = WorkState::Cancelled;
            task.error = Some(reason.to_owned());
            task.ended = Some(unix_now());
            self.pids.lock().unwrap().remove(id)
        };
        if let Some(pid) = pid {
            kill(pid);
        }
        self.status(id)
    }

    /// The notice for `tab`, and the task ids it covers. Delivery marks them, so a failed type can retry.
    pub fn pending_notice(&self, tab: &str) -> Option<(String, Vec<String>)> {
        let tasks = self.tasks.lock().unwrap();
        let ready: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                !task.delivered
                    && task.mode == Mode::Async
                    && matches!(task.state, WorkState::ResultAvailable | WorkState::Failed)
                    && root_tab(&tasks, task.parent.clone()).as_deref() == Some(tab)
            })
            .map(|(index, _)| index)
            .collect();
        if ready.is_empty() {
            return None;
        }
        let lines = ready
            .iter()
            .map(|index| notice_line(&tasks[*index]))
            .collect::<Vec<_>>()
            .join("\n");
        let ids = ready.iter().map(|index| tasks[*index].id.clone()).collect();
        Some((lines, ids))
    }

    pub fn mark_delivered(&self, ids: &[String]) {
        let mut tasks = self.tasks.lock().unwrap();
        for task in tasks.iter_mut() {
            if ids.iter().any(|id| id == &task.id) {
                task.delivered = true;
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

    pub fn open(&self, id: &str) -> Result<OpenInfo, String> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| format!("no task {id}"))?;
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
                task.state == WorkState::Working
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
            .filter(|task| task.state == WorkState::Working)
            .map(|task| task.id.clone())
            .collect()
    }

    /// `--list-models` text, kept for ten minutes, or fifteen seconds when the CLI said nothing.
    pub fn cursor_catalog(&self, bin: &Path, path: Option<&str>) -> String {
        if let Some(cache) = self.cursor.lock().unwrap().as_ref() {
            let fresh = if cache.list.trim().is_empty() {
                MODEL_MISS
            } else {
                MODEL_CACHE
            };
            if cache.at.elapsed() < fresh {
                return cache.list.clone();
            }
        }
        let list = list_cursor_text(bin, path);
        *self.cursor.lock().unwrap() = Some(Cache {
            at: Instant::now(),
            list: list.clone(),
        });
        list
    }

    pub fn cursor_ids(&self, bin: &Path, path: Option<&str>) -> Vec<String> {
        parse_model_list(&self.cursor_catalog(bin, path))
            .into_iter()
            .map(|(id, _)| id)
            .collect()
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
    fn note(&mut self, event: StreamEvent) -> bool {
        if self.state != WorkState::Working {
            return false;
        }
        match event {
            StreamEvent::Conversation(id) => {
                self.conversation = Some(id);
                false
            }
            StreamEvent::Step(item) => {
                push_step(&mut self.steps, item);
                false
            }
            StreamEvent::Final { text, error } => {
                self.summary = Some(text);
                self.state = if error.is_some() {
                    WorkState::Failed
                } else {
                    WorkState::ResultAvailable
                };
                self.error = error;
                self.ended = Some(unix_now());
                true
            }
        }
    }

    fn exited(&mut self, code: Option<i32>, stderr: String) -> bool {
        if self.state != WorkState::Working {
            return false;
        }
        self.ended = Some(unix_now());
        if code == Some(0) {
            if let Some(text) = last_text(&self.steps) {
                self.summary = Some(text);
                self.state = WorkState::ResultAvailable;
                return true;
            }
        }
        self.state = WorkState::Failed;
        self.error = Some(if stderr.is_empty() {
            format!(
                "the agent exited ({})",
                code.map_or("no status".into(), |code| code.to_string())
            )
        } else {
            stderr
        });
        true
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
        WorkState::Failed => format!(
            "failed: {}",
            task.error.as_deref().unwrap_or("the agent failed")
        ),
        _ => "result available".into(),
    };
    format!("[pinned] Delegated task {} \"{}\" ({who}) finished: {outcome}. Call task_status with taskId \"{}\".", task.id, task.title, task.id)
}

fn card_state(state: WorkState) -> subagents::State {
    match state {
        WorkState::Working => subagents::State::Running,
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
