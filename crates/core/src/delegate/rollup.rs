//! What one piece of work cost: a thread, the workers it delegated, theirs in turn, and their boxes.
use super::*;
use std::collections::HashSet;

/// One worker's share of a rollup.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct WorkerCost {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The machine that ran it: its name, or "this machine".
    pub machine: String,
    pub state: String,
    /// How many tasks lie between this one and the thread: 0 for a worker the thread started itself.
    pub depth: u32,
    /// What its agent cost. None while the cost is unknown, which is not zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// What its Upstash Box cost, for box work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_usd: Option<f64>,
    pub wall_ms: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// The cost of a thread and everything it delegated. Only costs that are known are added up;
/// `unknown` counts the ones that are not, so the total is a floor when it is above zero.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct WorkCost {
    /// `own_usd` + `agents_usd` + `box_usd`.
    pub total_usd: f64,
    /// What the thread's own session cost, when its transcript says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_usd: Option<f64>,
    /// The workers' agents.
    pub agents_usd: f64,
    /// The workers' boxes.
    pub box_usd: f64,
    /// Workers and thread sessions whose agent cost is not known (a Cursor worker is not metered).
    pub unknown: u32,
    pub workers: u32,
    pub wall_ms: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tasks: Vec<WorkerCost>,
}

impl WorkCost {
    /// Adds the cost of the thread's own session, when its transcript has one yet. A thread that
    /// has not run a turn has spent nothing, so it is not counted as unknown.
    pub fn with_own(mut self, own: Option<f64>) -> WorkCost {
        if let Some(usd) = own {
            self.total_usd += usd;
            self.own_usd = Some(usd);
        }
        self
    }
}

/// What a thread's card shows of its rollup: the cost of its own session and its workers, added up.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct WorkTotal {
    /// The known costs, the thread's own session, agents and boxes together.
    pub usd: f64,
    /// Parts whose cost is not known, which `usd` leaves out.
    pub unknown: u32,
    /// Workers under the thread, theirs in turn included.
    pub workers: u32,
}

impl WorkCost {
    pub fn total(&self) -> WorkTotal {
        WorkTotal { usd: self.total_usd, unknown: self.unknown, workers: self.workers }
    }
}

/// Each task's children, in registry order, built once so a rollup does not scan the registry per task.
type Children<'a> = HashMap<&'a str, Vec<&'a Task>>;

fn children_index(tasks: &[Task]) -> Children<'_> {
    let mut index = Children::new();
    for task in tasks {
        if let Parent::Task(parent) = &task.parent {
            index.entry(parent.as_str()).or_default().push(task);
        }
    }
    index
}

fn roots_of<'a>(tasks: &'a [Task], tab: &str) -> Vec<&'a Task> {
    tasks.iter().filter(|task| matches!(&task.parent, Parent::Tab(parent) if parent == tab)).collect()
}

/// The tasks below `roots`, each with its depth, as the workers they started and theirs in turn.
fn rollup_of(children_of: &Children, roots: Vec<&Task>) -> WorkCost {
    let mut cost = WorkCost::default();
    let mut seen = HashSet::new();
    let mut stack: Vec<(&Task, u32)> = roots.into_iter().rev().map(|task| (task, 0)).collect();
    while let Some((task, depth)) = stack.pop() {
        if !seen.insert(task.id.as_str()) {
            continue;
        }
        let usage = task.usage;
        let end = task.ended.unwrap_or_else(|| SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64));
        let wall_ms = end.saturating_sub(task.started).max(usage.wall_ms);
        match usage.cost_usd {
            Some(usd) => cost.agents_usd += usd,
            None => cost.unknown += 1,
        }
        cost.box_usd += usage.box_usd.unwrap_or(0.0);
        cost.wall_ms += wall_ms;
        cost.tokens_in += usage.tokens_in;
        cost.tokens_out += usage.tokens_out;
        cost.workers += 1;
        cost.tasks.push(WorkerCost {
            id: task.id.clone(),
            title: task.title.clone(),
            model: task.model.clone(),
            machine: task.remote.as_ref().map_or_else(|| "this machine".to_owned(), |remote| remote.name.clone()),
            state: task.state.as_str().to_owned(),
            depth,
            cost_usd: usage.cost_usd,
            box_usd: usage.box_usd,
            wall_ms,
            tokens_in: usage.tokens_in,
            tokens_out: usage.tokens_out,
        });
        if let Some(children) = children_of.get(task.id.as_str()) {
            stack.extend(children.iter().rev().map(|child| (*child, depth + 1)));
        }
    }
    cost.total_usd = cost.agents_usd + cost.box_usd;
    cost
}

impl Delegates {
    /// Everything the thread in `tab` delegated, workers of workers included. Its own session is
    /// not here: `WorkCost::with_own` adds it.
    pub fn work_cost_of_tab(&self, tab: &str) -> WorkCost {
        let tasks = self.tasks.lock().unwrap();
        rollup_of(&children_index(&tasks), roots_of(&tasks, tab))
    }

    /// Like `work_cost_of_tab`, but None for a thread that delegated nothing: what `work_costs` omits.
    pub fn work_cost_if_any(&self, tab: &str) -> Option<WorkCost> {
        let tasks = self.tasks.lock().unwrap();
        let roots = roots_of(&tasks, tab);
        (!roots.is_empty()).then(|| rollup_of(&children_index(&tasks), roots))
    }

    /// Task `id` and everything it delegated, when `caller` owns it.
    pub fn work_cost_of_task(&self, caller: &Caller, id: &str) -> Result<WorkCost, String> {
        let tasks = self.tasks.lock().unwrap();
        let root = tasks.iter().find(|task| task.id == id).ok_or_else(|| format!("no task {id}"))?;
        if !allows(&tasks, id, caller) {
            return Err(format!("task {id} belongs to another session"));
        }
        Ok(rollup_of(&children_index(&tasks), vec![root]))
    }

    /// The cost of each thread that delegated anything, by tab, for the tabs' views.
    pub fn work_costs(&self) -> HashMap<String, WorkCost> {
        let tasks = self.tasks.lock().unwrap();
        let children_of = children_index(&tasks);
        let mut roots: HashMap<&str, Vec<&Task>> = HashMap::new();
        for task in tasks.iter() {
            if let Parent::Tab(tab) = &task.parent {
                roots.entry(tab.as_str()).or_default().push(task);
            }
        }
        roots.into_iter().map(|(tab, roots)| (tab.to_owned(), rollup_of(&children_of, roots))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::TaskUsage;

    fn start(d: &Delegates, parent: Parent, title: &str) -> String {
        d.begin(NewTask {
            parent,
            tool: None,
            title: title.into(),
            account: "claude".into(),
            provider: Provider::Claude,
            model: None,
            cwd: PathBuf::from("/repo"),
            mode: Mode::Async,
            access: Access::Auto,
            isolated: false,
            caller_key: "tab:t".into(),
            client_request_id: None,
            role: None,
        })
        .id
    }

    /// 1 thread with 20 workers, each with 9 under it and each of those with 9 more: 1820 tasks.
    /// Plus 200 on a second thread, so `work_costs` has two keys.
    fn registry() -> (Delegates, Vec<String>) {
        let d = Delegates::default();
        let mut ids = vec![];
        for a in 0..20 {
            let top = start(&d, Parent::Tab("t1".into()), &format!("top {a}"));
            ids.push(top.clone());
            for b in 0..9 {
                let mid = start(&d, Parent::Task(top.clone()), &format!("mid {a}.{b}"));
                ids.push(mid.clone());
                for c in 0..9 {
                    ids.push(start(&d, Parent::Task(mid.clone()), &format!("leaf {a}.{b}.{c}")));
                }
            }
        }
        for n in 0..200 {
            ids.push(start(&d, Parent::Tab("t2".into()), &format!("other {n}")));
        }
        let used: Vec<_> = ids.iter().enumerate().map(|(n, id)| (id.clone(), TaskUsage { tokens_in: n as u64, tokens_out: 1, cost_usd: (n % 7 != 0).then_some(0.01 * (n % 13) as f64), wall_ms: 1000, box_usd: None, box_ms: None })).collect();
        d.meter(&used, None);
        (d, ids)
    }

    #[test]
    fn a_registry_of_two_thousand_tasks_rolls_up_to_the_same_workers_in_the_same_order() {
        let (d, ids) = registry();
        assert_eq!(ids.len(), 2020);
        let costs = d.work_costs();
        let mut keys: Vec<_> = costs.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["t1", "t2"]);

        let t1 = &costs["t1"];
        assert_eq!(t1.workers, 1820);
        let titles: Vec<_> = t1.tasks.iter().take(13).map(|w| (w.title.as_str(), w.depth)).collect();
        assert_eq!(titles, [("top 0", 0), ("mid 0.0", 1), ("leaf 0.0.0", 2), ("leaf 0.0.1", 2), ("leaf 0.0.2", 2), ("leaf 0.0.3", 2), ("leaf 0.0.4", 2), ("leaf 0.0.5", 2), ("leaf 0.0.6", 2), ("leaf 0.0.7", 2), ("leaf 0.0.8", 2), ("mid 0.1", 1), ("leaf 0.1.0", 2)], "depth first, children in registry order");
        assert_eq!((t1.tasks.last().unwrap().title.as_str(), t1.tasks.last().unwrap().depth), ("leaf 19.8.8", 2));
        assert_eq!(t1.tokens_in, (0..1820u64).sum::<u64>());
        assert_eq!(t1.tokens_out, 1820);
        let known: f64 = (0..1820usize).filter(|n| n % 7 != 0).map(|n| 0.01 * (n % 13) as f64).sum();
        assert!((t1.agents_usd - known).abs() < 1e-6, "{} vs {known}", t1.agents_usd);
        assert_eq!(t1.unknown as usize, (0..1820usize).filter(|n| n % 7 == 0).count());

        assert_eq!(costs["t2"].workers, 200);
        assert_eq!(costs["t2"].tasks.iter().map(|w| w.depth).max(), Some(0));

        // One tab's own rollup is the one the map holds, and a tab with no workers has none.
        assert_eq!(d.work_cost_if_any("t1").as_ref(), Some(t1));
        assert_eq!(d.work_cost_if_any("t2").as_ref(), Some(&costs["t2"]));
        assert_eq!(d.work_cost_if_any("t3"), None);
        assert_eq!(d.work_cost_of_tab("t1").tasks.len(), 1820);
        assert_eq!(d.work_cost_of_tab("t3").workers, 0);
        let caller = Caller { tab: Some("t1".into()), task: None };
        let mid = d.work_cost_of_task(&caller, &ids[1]).unwrap();
        assert_eq!((mid.workers, mid.tasks[0].title.as_str()), (10, "mid 0.0"));
    }

    #[test]
    #[ignore = "timing probe: cargo test -p pinned-core --release work_costs_time -- --ignored --nocapture"]
    fn work_costs_time() {
        let (d, _) = registry();
        let start = std::time::Instant::now();
        let runs = 20;
        for _ in 0..runs {
            std::hint::black_box(d.work_costs());
        }
        let all = start.elapsed() / runs;
        let start = std::time::Instant::now();
        for _ in 0..runs {
            std::hint::black_box(d.work_cost_of_tab("t2"));
        }
        eprintln!("work_costs over 2000 tasks: {all:?} per call; one tab (t2, 200 tasks): {:?} per call", start.elapsed() / runs);
    }
}
