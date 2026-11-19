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

/// The cost of a thread and everything it delegated. Only costs that are known are added up,
/// and a team box's worker priced at the ceiling rate (`Task::at_ceiling`); `unknown` counts
/// the ones that are not known, that worker included.
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
    /// Workers and thread sessions whose agent cost is not known (a Cursor worker is not metered,
    /// a team box's worker on a model with no price is priced at the ceiling rate).
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

/// The tasks below `roots`, each with its depth, as the workers they started and theirs in turn.
fn rollup_of(tasks: &[Task], roots: Vec<&Task>) -> WorkCost {
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
        if task.at_ceiling {
            cost.unknown += 1;
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
        let children = tasks.iter().filter(|child| matches!(&child.parent, Parent::Task(parent) if *parent == task.id));
        stack.extend(children.collect::<Vec<_>>().into_iter().rev().map(|child| (child, depth + 1)));
    }
    cost.total_usd = cost.agents_usd + cost.box_usd;
    cost
}

impl Delegates {
    /// Everything the thread in `tab` delegated, workers of workers included. Its own session is
    /// not here: `WorkCost::with_own` adds it.
    pub fn work_cost_of_tab(&self, tab: &str) -> WorkCost {
        let tasks = self.tasks.lock().unwrap();
        let roots = tasks.iter().filter(|task| matches!(&task.parent, Parent::Tab(parent) if parent == tab)).collect();
        rollup_of(&tasks, roots)
    }

    /// Task `id` and everything it delegated, when `caller` owns it.
    pub fn work_cost_of_task(&self, caller: &Caller, id: &str) -> Result<WorkCost, String> {
        let tasks = self.tasks.lock().unwrap();
        let root = tasks.iter().find(|task| task.id == id).ok_or_else(|| format!("no task {id}"))?;
        if !allows(&tasks, id, caller) {
            return Err(format!("task {id} belongs to another session"));
        }
        Ok(rollup_of(&tasks, vec![root]))
    }

    /// The cost of each thread that delegated anything, by tab, for the tabs' views.
    pub fn work_costs(&self) -> HashMap<String, WorkCost> {
        let tasks = self.tasks.lock().unwrap();
        let tabs: HashSet<&str> = tasks.iter().filter_map(|task| if let Parent::Tab(tab) = &task.parent { Some(tab.as_str()) } else { None }).collect();
        tabs.into_iter()
            .map(|tab| {
                let roots = tasks.iter().filter(|task| matches!(&task.parent, Parent::Tab(parent) if parent == tab)).collect();
                (tab.to_owned(), rollup_of(&tasks, roots))
            })
            .collect()
    }
}

impl Delegates {
    /// What the plans need of each task in `ids`, read now: how it stands, what its worker waits on, and what
    /// its tree cost. An id no task holds is left out, so a plan reads its saved outcome for it.
    pub fn plan_facts(&self, ids: &[String]) -> Vec<crate::plan::TaskFacts> {
        let tasks = self.tasks.lock().unwrap();
        ids.iter()
            .filter_map(|id| {
                let task = tasks.iter().find(|task| &task.id == id)?;
                let rolled = rollup_of(&tasks, vec![task]);
                let work = match task.state {
                    WorkState::Working => crate::plan::Work::Working,
                    WorkState::WaitingForChildren => crate::plan::Work::WaitingForChildren,
                    WorkState::ResultAvailable => crate::plan::Work::ResultAvailable,
                    WorkState::Failed => crate::plan::Work::Failed,
                    WorkState::Cancelled => crate::plan::Work::Cancelled,
                };
                let review = match task.review.as_ref().map(|review| review.verdict) {
                    None => crate::plan::Review::Unreviewed,
                    Some(Verdict::Accepted) => crate::plan::Review::Accepted,
                    Some(Verdict::Redone) => crate::plan::Review::Redone,
                };
                Some(crate::plan::TaskFacts {
                    id: task.id.clone(),
                    work,
                    waits_on_user: task.run.user_wait.is_some() || task.remote.as_ref().is_some_and(|remote| remote.waiting.is_some()),
                    review,
                    failure: task.limit.clone().or_else(|| task.error.clone()),
                    // A worker whose cost is unknown leaves the total short, so the tree has no cost to show.
                    cost: (rolled.unknown == 0).then_some(rolled.total_usd),
                    ended_at: task.ended,
                })
            })
            .collect()
    }
}
