//! Walks one run of a graph. Pure: it asks for each side effect as an `Effect`, and the hub
//! performs it and hands the result back with `supply`. The hub stores `Run` after every step, so a
//! restart calls `drive` again and gets the same effect back until it is supplied.
//!
//! One run has one Claude session. The first `talk` opens it; later ones type into it. When the tab
//! is gone the run resumes the same Claude conversation, and when that conversation is gone too it
//! opens a new one seeded with the run's history, so the new session knows everything so far.

use super::{Answers, Classify, Fallback, Gate, Graph, NodeId, NodeKind, PortId};
use crate::agent::Subject;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct RunId(pub String);

/// Every graph the hub holds, by connector id. `call` reads its target here.
pub type Library = BTreeMap<String, Graph>;

/// One execution. Owned by the hub, stored beside `connectors.json`, never sent to apps.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Run {
    pub id: RunId,
    /// The connector whose trigger started the run.
    pub connector: String,
    pub subject: Subject,
    pub status: Status,
    /// The connector whose `talk` opened the session; its settings, env and `followed` are the
    /// session's. Fixed once set.
    pub session_owner: Option<String>,
    pub session: Session,
    /// What has happened so far, in order. The history a new session is seeded with.
    pub log: Vec<Entry>,
    /// The data flowing through the run, as n8n's item: `trigger` holds the input, and each
    /// classify node writes its answers under its name.
    pub context: Value,
    /// Outermost frame first. The last frame is the graph `drive` is inside.
    stack: Vec<Frame>,
    /// Returned by `drive` until `supply` answers it.
    pending: Option<Effect>,
    /// The classify `pending` is asking. `supply` has no library, so the gate travels with the run.
    ready: Option<ReadyClassify>,
    /// Set once a claim has been supplied, so a finished run does not claim twice.
    claimed: bool,
}

/// One graph the walk is inside. `queue` is the nodes still to run at this depth, front first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Frame {
    connector: String,
    queue: Vec<NodeId>,
    phase: Phase,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    /// Execute `queue`'s front node.
    Enter,
    /// The front node is a talk whose text was sent. `status` is `Waiting`.
    WaitTurn,
    /// The front node finished. Splice this port's targets in front of its siblings.
    Follow(PortId),
    /// The frame above is the called graph. When it pops, follow this node's `out`.
    InCall,
}

/// Captured when a classify effect is issued. Port choice reads it on `supply`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct ReadyClassify {
    name: String,
    classify: Classify,
    /// Ports whose branch reaches a `page` before a talk, a call, or a stop.
    paging: Vec<PortId>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Running,
    /// Waiting for the session to end its turn.
    Waiting,
    Done,
    Failed {
        why: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Session {
    None,
    /// A tab is running the conversation. `claude` is Claude Code's session id once a hook reported it.
    Live {
        machine: String,
        tab: String,
        claude: Option<String>,
    },
    /// The tab is gone. The next `talk` resumes `claude`, or opens a seeded session when it is None.
    Detached {
        claude: Option<String>,
    },
}

/// One thing that happened, for the log and for seeding a new session.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// RFC 3339.
    pub at: String,
    /// The connector whose graph holds `node`; differs from `Run.connector` inside a `call`.
    pub connector: String,
    pub node: NodeId,
    pub what: Value,
}

/// A side effect the hub performs. Every effect is safe to perform twice.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    /// Mark the subject handled on `connector`, so the poll does not start a second run.
    Claim {
        connector: String,
        key: String,
    },
    Classify {
        connector: String,
        node: NodeId,
        input: String,
    },
    Page {
        connector: String,
        group: String,
    },
    StagePicked {
        connector: String,
    },
    /// Open the session with `owner`'s settings. `prompt` is the owner's usual opening prompt plus
    /// the talk's text, and, when `seeded`, the run's history.
    Open {
        owner: String,
        prompt: String,
        seeded: bool,
    },
    /// Reopen the tab on `claude --resume <claude>`, then type `text`.
    Resume {
        owner: String,
        claude: String,
        text: String,
    },
    /// Type into the live tab.
    Type {
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Supplied {
    Done,
    Classified(Answers),
    Opened {
        machine: String,
        tab: String,
    },
    /// `claude --resume` found no conversation; the runner opens a seeded session next.
    NoConversation,
    Failed(String),
}

/// News from outside the run that it may be waiting for.
#[derive(Clone, Debug, PartialEq)]
pub enum Wake {
    /// The tab's status went done or failed. `claude` is the id a hook reported, if any.
    TurnEnded {
        claude: Option<String>,
    },
    TabGone,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Drive {
    /// Perform this, then `supply` the result.
    Act(Effect),
    /// Store the run and wait for a `Wake`.
    Park,
    Done,
}

/// Starts a run at the trigger of `library[connector]`. `input` is the trigger's text: the Linear
/// issue or the Slack message, which goes into `context.trigger` and the first log entry.
pub fn start(library: &Library, connector: &str, subject: Subject, input: Value, at: &str) -> Run {
    let mut run = Run {
        id: RunId(format!("{connector}:{}:{at}", subject_key(&subject))),
        connector: connector.to_owned(),
        subject,
        status: Status::Running,
        session_owner: None,
        session: Session::None,
        log: Vec::new(),
        context: json!({ "trigger": input }),
        stack: Vec::new(),
        pending: None,
        ready: None,
        claimed: false,
    };
    let Some(graph) = library.get(connector) else {
        return failed(run, format!("workflow {connector:?} has no graph"));
    };
    let Some(trigger) = graph
        .nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::Trigger { .. }))
    else {
        return failed(run, format!("workflow {connector:?} has no trigger"));
    };
    let targets = targets_of(graph, &trigger.id, &out_port());
    run.log.push(Entry {
        at: at.to_owned(),
        connector: connector.to_owned(),
        node: trigger.id.clone(),
        what: run.context["trigger"].clone(),
    });
    run.stack.push(Frame {
        connector: connector.to_owned(),
        queue: targets,
        phase: Phase::Enter,
    });
    run
}

/// The next step. Returns the same `Act` until `supply` answers it.
pub fn drive(run: &mut Run, library: &Library) -> Drive {
    if let Some(effect) = run.pending.clone() {
        return Drive::Act(effect);
    }
    match run.status {
        Status::Done | Status::Failed { .. } => return Drive::Done,
        Status::Waiting => return Drive::Park,
        Status::Running => {}
    }
    for _ in 0..10_000 {
        pop_finished(&mut run.stack);
        if run.stack.is_empty() {
            return finish(run);
        }
        let phase = run.stack.last().expect("frame").phase.clone();
        match phase {
            Phase::WaitTurn => {
                run.status = Status::Waiting;
                return Drive::Park;
            }
            Phase::Follow(port) => {
                if let Err(why) = follow(run, library, &port) {
                    return fail(run, why);
                }
            }
            Phase::InCall => return fail(run, "a call is missing its workflow frame".into()),
            Phase::Enter => match step(run, library) {
                Next::Cont => {}
                Next::Act(effect) => return issue(run, effect),
                Next::Fail(why) => return fail(run, why),
            },
        }
    }
    fail(run, "the walk did not end".into())
}

pub fn supply(run: &mut Run, supplied: Supplied, at: &str) {
    if matches!(run.status, Status::Done | Status::Failed { .. }) {
        return;
    }
    let Some(effect) = run.pending.take() else {
        return;
    };
    let ready = run.ready.take();
    if let Supplied::Failed(why) = supplied {
        fail_status(run, why);
        return;
    }
    if let Err(why) = apply(run, effect, ready, supplied, at) {
        fail_status(run, why);
    }
}

pub fn push(run: &mut Run, wake: Wake, at: &str) {
    if matches!(run.status, Status::Done | Status::Failed { .. }) {
        return;
    }
    match wake {
        Wake::TabGone => detach(run, at),
        Wake::TurnEnded { claude } => turn_ended(run, claude),
    }
}

/// The text a new session is seeded with: the run's log and context as JSON, said plainly.
pub fn history(run: &Run) -> String {
    let body = serde_json::to_string(&json!({ "log": &run.log, "context": &run.context }))
        .unwrap_or_else(|_| "{}".into());
    format!("This session continues a run that already happened. Everything that happened so far is in this JSON.\n{body}")
}

/// The port a classify node takes for `answers`. Pure; `pages(port)` says whether that port's
/// branch pages a group before it reaches a talk, a call or a stop.
pub fn port_for(classify: &Classify, answers: &Answers, pages: &dyn Fn(&PortId) -> bool) -> PortId {
    match pick_port(classify, answers, pages) {
        Ok(port) => port,
        Err(PickStop::Branch(_)) => PortId(String::new()),
        Err(PickStop::Run(_)) => nowhere_port(),
    }
}

enum Next {
    Cont,
    Act(Effect),
    Fail(String),
}

enum PickStop {
    /// End this branch and record `why`. The run continues with the next sibling.
    Branch(String),
    /// The classify result cannot be applied.
    Run(String),
}

fn step(run: &mut Run, library: &Library) -> Next {
    run.ready = None;
    let (connector, node_id) = match front(run) {
        Ok(front) => front,
        Err(why) => return Next::Fail(why),
    };
    let (name, port_ids, kind) = {
        let Some(graph) = library.get(&connector) else {
            return Next::Fail(format!("workflow {connector:?} has no graph"));
        };
        let Some(node) = graph.node(&node_id) else {
            return Next::Fail(format!(
                "node {} is not in workflow {connector:?}",
                node_id.0
            ));
        };
        let port_ids: Vec<PortId> = node.kind.ports().into_iter().map(|port| port.id).collect();
        (node.name.clone(), port_ids, node.kind.clone())
    };
    match kind {
        NodeKind::Trigger { .. } => Next::Fail(format!(
            "the walk entered trigger {} in workflow {connector:?}",
            node_id.0
        )),
        NodeKind::Stop => {
            let at = stamped(run);
            record(run, &connector, &node_id, json!({ "stop": true }), &at);
            drop_front(run);
            Next::Cont
        }
        NodeKind::Page { group } => Next::Act(Effect::Page { connector, group }),
        NodeKind::StagePicked => Next::Act(Effect::StagePicked { connector }),
        NodeKind::Talk { prompt } => match talk_effect(run, &connector, &prompt) {
            Ok(effect) => Next::Act(effect),
            Err(why) => Next::Fail(why),
        },
        NodeKind::Call { workflow } => enter_call(run, library, &connector, &node_id, &workflow),
        NodeKind::Classify(classify) => {
            let paging = port_ids
                .into_iter()
                .filter(|port| branch_pages(library, &connector, &node_id, port))
                .collect();
            let input = serde_json::to_string(&run.context).unwrap_or_else(|_| "{}".into());
            run.ready = Some(ReadyClassify {
                name,
                classify,
                paging,
            });
            Next::Act(Effect::Classify {
                connector,
                node: node_id,
                input,
            })
        }
    }
}

fn enter_call(
    run: &mut Run,
    library: &Library,
    caller: &str,
    node: &NodeId,
    workflow: &str,
) -> Next {
    if run.stack.iter().any(|frame| frame.connector == workflow) {
        let chain = run
            .stack
            .iter()
            .map(|frame| frame.connector.as_str())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Next::Fail(format!("call to workflow {workflow:?} cycles: it is already on the stack ({chain} -> {workflow})"));
    }
    let Some(graph) = library.get(workflow) else {
        return Next::Fail(format!("call to workflow {workflow:?}, which has no graph"));
    };
    let Some(trigger) = graph
        .nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::Trigger { .. }))
    else {
        return Next::Fail(format!("workflow {workflow:?} has no trigger"));
    };
    let targets = targets_of(graph, &trigger.id, &out_port());
    let at = stamped(run);
    record(run, caller, node, json!({ "call": workflow }), &at);
    run.stack.last_mut().expect("caller frame").phase = Phase::InCall;
    run.stack.push(Frame {
        connector: workflow.to_owned(),
        queue: targets,
        phase: Phase::Enter,
    });
    Next::Cont
}

fn talk_effect(run: &mut Run, connector: &str, prompt: &str) -> Result<Effect, String> {
    if run.session_owner.is_none() {
        run.session_owner = Some(connector.to_owned());
    }
    let owner = run
        .session_owner
        .clone()
        .ok_or_else(|| "a talk has no session owner".to_owned())?;
    let effect = match &run.session {
        Session::None if !run.claimed => Effect::Claim {
            connector: owner,
            key: subject_key(&run.subject),
        },
        Session::None => Effect::Open {
            owner,
            prompt: prompt.to_owned(),
            seeded: false,
        },
        Session::Live { .. } => Effect::Type {
            text: prompt.to_owned(),
        },
        Session::Detached { claude: Some(id) } => Effect::Resume {
            owner,
            claude: id.clone(),
            text: prompt.to_owned(),
        },
        Session::Detached { claude: None } => Effect::Open {
            owner,
            prompt: seeded_prompt(run, prompt),
            seeded: true,
        },
    };
    Ok(effect)
}

fn apply(
    run: &mut Run,
    effect: Effect,
    ready: Option<ReadyClassify>,
    supplied: Supplied,
    at: &str,
) -> Result<(), String> {
    match (effect, supplied) {
        (Effect::Claim { .. }, Supplied::Done) => {
            run.claimed = true;
            if run.stack.is_empty() {
                run.status = Status::Done;
            }
            Ok(())
        }
        (Effect::Classify { .. }, Supplied::Classified(answers)) => {
            classify_supplied(run, ready, &answers, at)
        }
        (Effect::Page { connector, group }, Supplied::Done) => {
            let node = front_node(run)?;
            record(run, &connector, &node, json!({ "page": group }), at);
            follow_out(run);
            Ok(())
        }
        (Effect::StagePicked { connector }, Supplied::Done) => {
            let node = front_node(run)?;
            record(run, &connector, &node, json!({ "stage": "picked" }), at);
            follow_out(run);
            Ok(())
        }
        (
            effect @ (Effect::Open { .. } | Effect::Type { .. } | Effect::Resume { .. }),
            supplied,
        ) => talk_supplied(run, effect, supplied, at),
        (effect, _) => Err(format!(
            "the result does not answer the outstanding {} effect",
            effect_name(&effect)
        )),
    }
}

fn classify_supplied(
    run: &mut Run,
    ready: Option<ReadyClassify>,
    answers: &Answers,
    at: &str,
) -> Result<(), String> {
    let Some(ready) = ready else {
        return Err("classify came back with nothing to decide".into());
    };
    write_answers(run, &ready.name, answers);
    let paging = ready.paging.clone();
    let node = front_node(run)?;
    let connector = run.stack.last().expect("frame").connector.clone();
    match pick_port(&ready.classify, answers, &|port| {
        paging.iter().any(|id| id == port)
    }) {
        Ok(port) => {
            record(
                run,
                &connector,
                &node,
                json!({ "answers": answers_value(answers), "port": port.0 }),
                at,
            );
            if let Some(frame) = run.stack.last_mut() {
                frame.phase = Phase::Follow(port);
            }
            Ok(())
        }
        Err(PickStop::Branch(shown)) => {
            let why = format!(
                "classify {:?} answered {shown}, which is not one of its choices",
                ready.name
            );
            record(
                run,
                &connector,
                &node,
                json!({ "answers": answers_value(answers), "stopped": why }),
                at,
            );
            drop_front(run);
            Ok(())
        }
        Err(PickStop::Run(why)) => Err(format!("classify {:?}: {why}", ready.name)),
    }
}

fn talk_supplied(
    run: &mut Run,
    effect: Effect,
    supplied: Supplied,
    at: &str,
) -> Result<(), String> {
    let prompt = own_prompt(run, &effect);
    match (effect, supplied) {
        (Effect::Resume { .. }, Supplied::NoConversation) => {
            run.session = Session::Detached { claude: None };
            if let Some(frame) = run.stack.last_mut() {
                frame.phase = Phase::Enter;
            }
            Ok(())
        }
        (Effect::Open { .. }, Supplied::Opened { machine, tab }) => {
            log_talk(run, &prompt, at)?;
            run.session = Session::Live {
                machine,
                tab,
                claude: None,
            };
            wait_turn(run);
            Ok(())
        }
        (Effect::Resume { claude, .. }, Supplied::Opened { machine, tab }) => {
            log_talk(run, &prompt, at)?;
            run.session = Session::Live {
                machine,
                tab,
                claude: Some(claude),
            };
            wait_turn(run);
            Ok(())
        }
        (Effect::Type { .. }, Supplied::Done) => {
            log_talk(run, &prompt, at)?;
            wait_turn(run);
            Ok(())
        }
        (effect, _) => Err(format!(
            "the result does not answer the outstanding {} effect",
            effect_name(&effect)
        )),
    }
}

fn log_talk(run: &mut Run, prompt: &str, at: &str) -> Result<(), String> {
    let node = front_node(run)?;
    let connector = run.stack.last().expect("frame").connector.clone();
    record(run, &connector, &node, json!({ "talk": prompt }), at);
    Ok(())
}

fn wait_turn(run: &mut Run) {
    run.status = Status::Waiting;
    if let Some(frame) = run.stack.last_mut() {
        frame.phase = Phase::WaitTurn;
    }
}

fn follow_out(run: &mut Run) {
    if let Some(frame) = run.stack.last_mut() {
        frame.phase = Phase::Follow(out_port());
    }
}

fn pick_port(
    classify: &Classify,
    answers: &Answers,
    pages: &dyn Fn(&PortId) -> bool,
) -> Result<PortId, PickStop> {
    if classify.branch_on.is_none() {
        return Ok(out_port());
    }
    if let Some(why) = decision_error(classify, answers) {
        return Err(PickStop::Run(why));
    }
    match &classify.gate {
        Gate::Direct => match labeled_port(classify, answers) {
            Some(port) => Ok(port),
            None if matches!(classify.fallback, Fallback::Extra) => Ok(fallback_port()),
            None => Err(PickStop::Branch(shown_answer(classify, answers))),
        },
        Gate::Decide {
            unsure_below,
            page_at,
        } => Ok(decide_port(
            classify,
            answers,
            *unsure_below,
            *page_at,
            pages,
        )),
    }
}

fn labeled_port(classify: &Classify, answers: &Answers) -> Option<PortId> {
    let name = classify.branch_on.as_ref()?;
    let question = classify
        .questions
        .iter()
        .find(|question| &question.name == name)?;
    let answer = answers.answers.iter().find(|(n, _)| n == name)?.1.as_str();
    question
        .choices
        .iter()
        .find(|choice| choice.label.eq_ignore_ascii_case(answer))
        .map(|choice| choice.id.clone())
}

fn shown_answer(classify: &Classify, answers: &Answers) -> String {
    match classify
        .branch_on
        .as_ref()
        .and_then(|name| answers.answers.iter().find(|(n, _)| n == name))
    {
        Some((_, answer)) => format!("{answer:?}"),
        None => "nothing".into(),
    }
}

fn decision_error(classify: &Classify, answers: &Answers) -> Option<String> {
    let Gate::Decide { .. } = &classify.gate else {
        return None;
    };
    let name = classify.branch_on.as_ref()?;
    let Some(question) = classify
        .questions
        .iter()
        .find(|question| &question.name == name)
    else {
        return Some(format!(
            "branches on {name:?}, which is not one of its questions"
        ));
    };
    let choices = question.choices.len();
    let Some(chosen) = &answers.chosen else {
        return Some("decides a port and came back without probabilities".into());
    };
    if chosen.index >= choices || chosen.probabilities.len() < choices {
        return Some(format!(
            "came back with index {} and {} probabilities for {choices} choices",
            chosen.index,
            chosen.probabilities.len()
        ));
    }
    None
}

/// Same branch as `Decide::pick`: an unsure quiet fallback, then a quiet guess or one at
/// `page_at`, then the likeliest quiet choice, else `nowhere`. Ties keep the earlier choice.
fn decide_port(
    classify: &Classify,
    answers: &Answers,
    unsure_below: f64,
    page_at: f64,
    pages: &dyn Fn(&PortId) -> bool,
) -> PortId {
    let Some(question) = classify.branch_on.as_ref().and_then(|name| {
        classify
            .questions
            .iter()
            .find(|question| &question.name == name)
    }) else {
        return nowhere_port();
    };
    let choices = &question.choices;
    let Some(chosen) = &answers.chosen else {
        return nowhere_port();
    };
    if chosen.index >= choices.len() || chosen.probabilities.len() < choices.len() {
        return nowhere_port();
    }
    let likelihood = chosen.probabilities[chosen.index];
    let quiet = |index: usize| !pages(&choices[index].id);
    if let Fallback::Existing { port } = &classify.fallback {
        if let Some(fallback) = choices.iter().position(|choice| &choice.id == port) {
            if quiet(fallback) && likelihood < unsure_below {
                return choices[fallback].id.clone();
            }
        }
    }
    if matches!(classify.fallback, Fallback::Extra)
        && likelihood < unsure_below
        && !pages(&fallback_port())
    {
        return fallback_port();
    }
    if quiet(chosen.index) || likelihood >= page_at {
        return choices[chosen.index].id.clone();
    }
    let best = (0..choices.len())
        .filter(|index| quiet(*index))
        .reduce(|best, index| {
            if chosen.probabilities[index] > chosen.probabilities[best] {
                index
            } else {
                best
            }
        });
    match best {
        Some(index) => choices[index].id.clone(),
        None => nowhere_port(),
    }
}

fn branch_pages(library: &Library, connector: &str, from: &NodeId, port: &PortId) -> bool {
    let Some(graph) = library.get(connector) else {
        return false;
    };
    let mut seen = BTreeSet::new();
    targets_of(graph, from, port)
        .iter()
        .any(|next| path_pages(library, connector, next, &mut seen))
}

fn path_pages(
    library: &Library,
    connector: &str,
    id: &NodeId,
    seen: &mut BTreeSet<(String, NodeId)>,
) -> bool {
    if !seen.insert((connector.to_owned(), id.clone())) {
        return false;
    }
    let next = {
        let Some(graph) = library.get(connector) else {
            return false;
        };
        let Some(node) = graph.node(id) else {
            return false;
        };
        match &node.kind {
            NodeKind::Page { .. } => return true,
            NodeKind::Talk { .. } | NodeKind::Call { .. } | NodeKind::Stop => return false,
            NodeKind::Trigger { .. } | NodeKind::StagePicked | NodeKind::Classify(_) => node
                .kind
                .ports()
                .iter()
                .flat_map(|port| targets_of(graph, &node.id, &port.id))
                .collect::<Vec<_>>(),
        }
    };
    next.iter()
        .any(|next| path_pages(library, connector, next, seen))
}

fn follow(run: &mut Run, library: &Library, port: &PortId) -> Result<(), String> {
    let (connector, node) = front(run)?;
    let graph = library
        .get(&connector)
        .ok_or_else(|| format!("workflow {connector:?} has no graph"))?;
    let targets = targets_of(graph, &node, port);
    splice(run, targets);
    Ok(())
}

fn targets_of(graph: &Graph, from: &NodeId, port: &PortId) -> Vec<NodeId> {
    graph.next(from, port).into_iter().cloned().collect()
}

fn splice(run: &mut Run, targets: Vec<NodeId>) {
    let frame = run.stack.last_mut().expect("frame");
    frame.queue.remove(0);
    for (index, target) in targets.into_iter().enumerate() {
        frame.queue.insert(index, target);
    }
    frame.phase = Phase::Enter;
}

fn drop_front(run: &mut Run) {
    let frame = run.stack.last_mut().expect("frame");
    frame.queue.remove(0);
    frame.phase = Phase::Enter;
}

fn pop_finished(stack: &mut Vec<Frame>) {
    while stack.last().is_some_and(|frame| frame.queue.is_empty()) {
        stack.pop();
        if let Some(parent) = stack.last_mut() {
            if parent.phase == Phase::InCall {
                parent.phase = Phase::Follow(out_port());
            }
        }
    }
}

fn finish(run: &mut Run) -> Drive {
    if !run.claimed && run.session_owner.is_none() {
        let effect = Effect::Claim {
            connector: run.connector.clone(),
            key: subject_key(&run.subject),
        };
        return issue(run, effect);
    }
    run.status = Status::Done;
    Drive::Done
}

fn issue(run: &mut Run, effect: Effect) -> Drive {
    run.pending = Some(effect.clone());
    Drive::Act(effect)
}

fn detach(run: &mut Run, at: &str) {
    let claude = match &run.session {
        Session::Live { claude, .. } => claude.clone(),
        Session::None | Session::Detached { .. } => return,
    };
    if let Ok(node) = front_node(run) {
        let connector = run.stack.last().expect("frame").connector.clone();
        record(
            run,
            &connector,
            &node,
            json!({ "tab": "gone", "claude": claude.clone() }),
            at,
        );
    }
    run.session = Session::Detached { claude };
    if run.status == Status::Waiting {
        run.status = Status::Running;
        if let Some(frame) = run.stack.last_mut() {
            if frame.phase == Phase::WaitTurn {
                frame.phase = Phase::Enter;
            }
        }
    }
}

fn turn_ended(run: &mut Run, claude: Option<String>) {
    if let Session::Live { claude: slot, .. } = &mut run.session {
        if claude.is_some() {
            *slot = claude;
        }
    }
    if run.status != Status::Waiting {
        return;
    }
    run.status = Status::Running;
    if let Some(frame) = run.stack.last_mut() {
        if frame.phase == Phase::WaitTurn {
            frame.phase = Phase::Follow(out_port());
        }
    }
}

fn front(run: &Run) -> Result<(String, NodeId), String> {
    let frame = run.stack.last().ok_or("the walk has no frame")?;
    let node = frame
        .queue
        .first()
        .cloned()
        .ok_or("the frame has no node")?;
    Ok((frame.connector.clone(), node))
}

fn front_node(run: &Run) -> Result<NodeId, String> {
    front(run).map(|(_, node)| node)
}

fn write_answers(run: &mut Run, name: &str, answers: &Answers) {
    if !run.context.is_object() {
        run.context = json!({});
    }
    run.context
        .as_object_mut()
        .expect("context object")
        .insert(name.to_owned(), answers_value(answers));
}

fn answers_value(answers: &Answers) -> Value {
    let mut object = serde_json::Map::new();
    for (name, answer) in &answers.answers {
        object.insert(name.clone(), Value::String(answer.clone()));
    }
    Value::Object(object)
}

fn record(run: &mut Run, connector: &str, node: &NodeId, what: Value, at: &str) {
    run.log.push(Entry {
        at: at.to_owned(),
        connector: connector.to_owned(),
        node: node.clone(),
        what,
    });
}

fn stamped(run: &Run) -> String {
    run.log
        .last()
        .map(|entry| entry.at.clone())
        .unwrap_or_default()
}

fn seeded_prompt(run: &Run, prompt: &str) -> String {
    format!("{}\n\n{prompt}", history(run))
}

fn own_prompt(run: &Run, effect: &Effect) -> String {
    match effect {
        Effect::Type { text } | Effect::Resume { text, .. } => text.clone(),
        Effect::Open {
            prompt,
            seeded: false,
            ..
        } => prompt.clone(),
        Effect::Open {
            prompt,
            seeded: true,
            ..
        } => {
            let prefix = format!("{}\n\n", history(run));
            prompt.strip_prefix(&prefix).unwrap_or(prompt).to_owned()
        }
        Effect::Claim { .. }
        | Effect::Classify { .. }
        | Effect::Page { .. }
        | Effect::StagePicked { .. } => String::new(),
    }
}

fn effect_name(effect: &Effect) -> &'static str {
    match effect {
        Effect::Claim { .. } => "claim",
        Effect::Classify { .. } => "classify",
        Effect::Page { .. } => "page",
        Effect::StagePicked { .. } => "stage-picked",
        Effect::Open { .. } => "open",
        Effect::Resume { .. } => "resume",
        Effect::Type { .. } => "type",
    }
}

fn subject_key(subject: &Subject) -> String {
    match subject {
        Subject::LinearIssue { issue } => issue.clone(),
        Subject::SlackThread { channel, thread } => format!("{channel}:{thread}"),
    }
}

fn out_port() -> PortId {
    PortId(PortId::OUT.to_owned())
}

fn fallback_port() -> PortId {
    PortId(PortId::FALLBACK.to_owned())
}

fn nowhere_port() -> PortId {
    PortId(PortId::NOWHERE.to_owned())
}

fn failed(mut run: Run, why: String) -> Run {
    run.status = Status::Failed { why };
    run
}

fn fail_status(run: &mut Run, why: String) {
    run.status = Status::Failed { why };
    run.pending = None;
    run.ready = None;
    run.stack.clear();
}

fn fail(run: &mut Run, why: String) -> Drive {
    fail_status(run, why);
    Drive::Done
}

#[cfg(test)]
mod tests {
    use super::super::{
        Answers, Choice, ChosenOn, Classify, Edge, Fallback, Gate, Graph, Node, NodeId, NodeKind,
        PortId, Question, Watch,
    };
    use super::{
        drive, history, port_for, push, start, supply, Drive, Effect, Entry, Library, Session,
        Status, Supplied, Wake,
    };
    use crate::agent::Subject;
    use serde_json::json;

    const AT: &str = "2026-10-29T12:00:00Z";
    const CLAUDE: &str = "ses_1";
    const THREAD: &str = "C0123ABCD:1792400050.000200";

    fn nid(id: &str) -> NodeId {
        NodeId(id.into())
    }

    fn pid(id: &str) -> PortId {
        PortId(id.into())
    }

    fn node(id: &str, name: &str, kind: NodeKind) -> Node {
        Node {
            id: nid(id),
            name: name.into(),
            position: [0, 0],
            kind,
        }
    }

    fn edge(from: &str, port: &str, to: &str) -> Edge {
        Edge {
            from: nid(from),
            port: pid(port),
            to: nid(to),
        }
    }

    fn graph(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph { nodes, edges }
    }

    fn library(items: Vec<(&str, Graph)>) -> Library {
        items
            .into_iter()
            .map(|(id, graph)| (id.to_owned(), graph))
            .collect()
    }

    fn trigger() -> NodeKind {
        NodeKind::Trigger {
            watch: Watch::Called,
        }
    }

    fn talk(prompt: &str) -> NodeKind {
        NodeKind::Talk {
            prompt: prompt.into(),
        }
    }

    fn choice(id: &str, label: &str) -> Choice {
        Choice {
            id: pid(id),
            label: label.into(),
            description: String::new(),
        }
    }

    fn linear() -> Subject {
        Subject::LinearIssue {
            issue: "uuid".into(),
        }
    }

    fn slack() -> Subject {
        Subject::SlackThread {
            channel: "C0123ABCD".into(),
            thread: "1792400050.000200".into(),
        }
    }

    fn opened() -> Supplied {
        Supplied::Opened {
            machine: "mac".into(),
            tab: "tab-1".into(),
        }
    }

    fn model_input(trigger: &str) -> String {
        serde_json::to_string(&json!({ "trigger": trigger })).unwrap()
    }

    fn answers(pairs: &[(&str, &str)], chosen: Option<(usize, &[f64])>) -> Answers {
        Answers {
            answers: pairs
                .iter()
                .map(|(name, answer)| ((*name).to_owned(), (*answer).to_owned()))
                .collect(),
            chosen: chosen.map(|(index, probabilities)| ChosenOn {
                index,
                probabilities: probabilities.to_vec(),
            }),
        }
    }

    fn act(run: &mut super::Run, library: &Library) -> Effect {
        match drive(run, library) {
            Drive::Act(effect) => effect,
            Drive::Park => panic!("parked"),
            Drive::Done => panic!("done: {:?}", run.status),
        }
    }

    fn walk(run: &mut super::Run, library: &Library, answers: Answers) -> Vec<Effect> {
        let mut seen = Vec::new();
        for _ in 0..64 {
            match drive(run, library) {
                Drive::Act(effect) => {
                    let supplied = match &effect {
                        Effect::Classify { .. } => Supplied::Classified(answers.clone()),
                        Effect::Open { .. } | Effect::Resume { .. } => opened(),
                        Effect::Claim { .. }
                        | Effect::Page { .. }
                        | Effect::StagePicked { .. }
                        | Effect::Type { .. } => Supplied::Done,
                    };
                    seen.push(effect);
                    supply(run, supplied, AT);
                }
                Drive::Park => push(
                    run,
                    Wake::TurnEnded {
                        claude: Some(CLAUDE.into()),
                    },
                    AT,
                ),
                Drive::Done => {
                    if let Status::Failed { why } = &run.status {
                        panic!("run failed: {why}; so far {seen:?}");
                    }
                    return seen;
                }
            }
        }
        panic!("walk did not end: {:?}", run.status);
    }

    fn run_effects(
        library: &Library,
        connector: &str,
        subject: Subject,
        input: &str,
        answers: Answers,
        expected: Vec<Effect>,
    ) -> super::Run {
        let mut run = start(library, connector, subject, json!(input), AT);
        let effects = walk(&mut run, library, answers);
        assert_eq!(effects, expected);
        assert_eq!(run.status, Status::Done);
        run
    }

    fn chain(nodes: Vec<(&str, &str, NodeKind)>) -> Graph {
        let mut edges = Vec::new();
        for pair in nodes.windows(2) {
            edges.push(edge(pair[0].0, "out", pair[1].0));
        }
        graph(
            nodes
                .into_iter()
                .map(|(id, name, kind)| node(id, name, kind))
                .collect(),
            edges,
        )
    }

    #[test]
    fn linear_chain_pages_stages_then_talks_and_claims_the_issue() {
        let lib = library(vec![(
            "c1",
            chain(vec![
                (
                    "t",
                    "Trigger",
                    NodeKind::Trigger {
                        watch: Watch::LinearLabel {
                            label: "claude".into(),
                        },
                    },
                ),
                (
                    "p",
                    "Page",
                    NodeKind::Page {
                        group: "oncall".into(),
                    },
                ),
                ("s", "Stage", NodeKind::StagePicked),
                ("k", "Talk", talk("Fix the button")),
                ("z", "Stop", NodeKind::Stop),
            ]),
        )]);
        let run = run_effects(
            &lib,
            "c1",
            linear(),
            "ENG-7: Broken button",
            answers(&[], None),
            vec![
                Effect::Page {
                    connector: "c1".into(),
                    group: "oncall".into(),
                },
                Effect::StagePicked {
                    connector: "c1".into(),
                },
                Effect::Claim {
                    connector: "c1".into(),
                    key: "uuid".into(),
                },
                Effect::Open {
                    owner: "c1".into(),
                    prompt: "Fix the button".into(),
                    seeded: false,
                },
            ],
        );
        assert_eq!(run.session_owner.as_deref(), Some("c1"));
        assert_eq!(run.context["trigger"], json!("ENG-7: Broken button"));
        assert_eq!(run.log[0].what, json!("ENG-7: Broken button"));
        assert_eq!(run.log[1].what, json!({ "page": "oncall" }));
        assert_eq!(run.log[2].what, json!({ "stage": "picked" }));
        assert_eq!(run.log[3].what, json!({ "talk": "Fix the button" }));
        assert_eq!(run.log[4].what, json!({ "stop": true }));
        assert!(run.log.iter().all(|entry| entry.connector == "c1"));
    }

    #[test]
    fn depth_first_one_session_runs_each_branch_to_its_end() {
        let lib = library(vec![(
            "c1",
            graph(
                vec![
                    node("t", "Trigger", trigger()),
                    node("a", "A", talk("one")),
                    node("a2", "A2", talk("two")),
                    node("z", "Stop", NodeKind::Stop),
                    node("b", "B", talk("three")),
                ],
                vec![
                    edge("t", "out", "a"),
                    edge("t", "out", "b"),
                    edge("a", "out", "a2"),
                    edge("a2", "out", "z"),
                ],
            ),
        )]);
        let run = run_effects(
            &lib,
            "c1",
            linear(),
            "ENG-7: Broken button",
            answers(&[], None),
            vec![
                Effect::Claim {
                    connector: "c1".into(),
                    key: "uuid".into(),
                },
                Effect::Open {
                    owner: "c1".into(),
                    prompt: "one".into(),
                    seeded: false,
                },
                Effect::Type { text: "two".into() },
                Effect::Type {
                    text: "three".into(),
                },
            ],
        );
        assert_eq!(
            run.session,
            Session::Live {
                machine: "mac".into(),
                tab: "tab-1".into(),
                claude: Some(CLAUDE.into())
            }
        );
    }

    #[test]
    fn classify_four_choices_each_branch_talks_twice_in_one_session() {
        let choices = [
            ("bug", "bug"),
            ("feat", "feat"),
            ("chore", "chore"),
            ("docs", "docs"),
        ];
        let mut nodes = vec![
            node("t", "Trigger", trigger()),
            node("sort", "Sort", direct("kind", &choices, Fallback::None)),
        ];
        let mut edges = vec![edge("t", "out", "sort")];
        for (id, _) in choices {
            nodes.push(node(&format!("{id}1"), id, talk(&format!("{id} 1"))));
            nodes.push(node(&format!("{id}2"), id, talk(&format!("{id} 2"))));
            edges.push(edge("sort", id, &format!("{id}1")));
            edges.push(edge(&format!("{id}1"), "out", &format!("{id}2")));
        }
        let lib = library(vec![("c1", graph(nodes, edges))]);
        let input = model_input("ENG-7: Broken button");
        for (answer, first, second) in [
            ("Bug", "bug 1", "bug 2"),
            ("FEAT", "feat 1", "feat 2"),
            ("Chore", "chore 1", "chore 2"),
            ("docs", "docs 1", "docs 2"),
        ] {
            let run = run_effects(
                &lib,
                "c1",
                linear(),
                "ENG-7: Broken button",
                answers(&[("kind", answer)], None),
                vec![
                    Effect::Classify {
                        connector: "c1".into(),
                        node: nid("sort"),
                        input: input.clone(),
                    },
                    Effect::Claim {
                        connector: "c1".into(),
                        key: "uuid".into(),
                    },
                    Effect::Open {
                        owner: "c1".into(),
                        prompt: first.into(),
                        seeded: false,
                    },
                    Effect::Type {
                        text: second.into(),
                    },
                ],
            );
            assert_eq!(run.session_owner.as_deref(), Some("c1"));
            assert_eq!(run.context["Sort"]["kind"], json!(answer));
            assert_eq!(
                run.session,
                Session::Live {
                    machine: "mac".into(),
                    tab: "tab-1".into(),
                    claude: Some(CLAUDE.into())
                }
            );
        }
    }

    #[test]
    fn direct_unknown_answer_stops_the_branch_and_logs_why() {
        let lib = library(vec![("c1", choice_talks(Fallback::None))]);
        let run = run_effects(
            &lib,
            "c1",
            linear(),
            "ENG-7: Broken button",
            answers(&[("kind", "nope")], None),
            vec![
                Effect::Classify {
                    connector: "c1".into(),
                    node: nid("sort"),
                    input: model_input("ENG-7: Broken button"),
                },
                Effect::Claim {
                    connector: "c1".into(),
                    key: "uuid".into(),
                },
            ],
        );
        assert_eq!(run.session_owner, None);
        assert_eq!(run.context["Sort"]["kind"], json!("nope"));
        assert_eq!(
            run.log
                .iter()
                .find(|entry| entry.what.get("stopped").is_some())
                .unwrap()
                .what["stopped"],
            json!("classify \"Sort\" answered \"nope\", which is not one of its choices")
        );
    }

    #[test]
    fn direct_unknown_answer_takes_the_extra_fallback() {
        let lib = library(vec![("c1", fallback_graph())]);
        let run = run_effects(
            &lib,
            "c1",
            linear(),
            "ENG-7: Broken button",
            answers(&[("kind", "nope")], None),
            vec![
                Effect::Classify {
                    connector: "c1".into(),
                    node: nid("sort"),
                    input: model_input("ENG-7: Broken button"),
                },
                Effect::Claim {
                    connector: "c1".into(),
                    key: "uuid".into(),
                },
                Effect::Open {
                    owner: "c1".into(),
                    prompt: "use fallback".into(),
                    seeded: false,
                },
            ],
        );
        assert_eq!(run.session_owner.as_deref(), Some("c1"));
    }

    #[test]
    fn classify_without_branch_on_follows_out() {
        let kind = NodeKind::Classify(Classify {
            model: "typesafe-ai/jev".into(),
            instructions: "Read it.".into(),
            questions: vec![Question {
                name: "note".into(),
                ask: "What?".into(),
                choices: vec![],
            }],
            branch_on: None,
            gate: Gate::Direct,
            fallback: Fallback::None,
        });
        let lib = library(vec![(
            "c1",
            graph(
                vec![
                    node("t", "Trigger", trigger()),
                    node("sort", "Sort", kind),
                    node("k", "Talk", talk("onward")),
                ],
                vec![edge("t", "out", "sort"), edge("sort", "out", "k")],
            ),
        )]);
        run_effects(
            &lib,
            "c1",
            linear(),
            "ENG-7: Broken button",
            answers(&[("note", "free text")], None),
            vec![
                Effect::Classify {
                    connector: "c1".into(),
                    node: nid("sort"),
                    input: model_input("ENG-7: Broken button"),
                },
                Effect::Claim {
                    connector: "c1".into(),
                    key: "uuid".into(),
                },
                Effect::Open {
                    owner: "c1".into(),
                    prompt: "onward".into(),
                    seeded: false,
                },
            ],
        );
    }

    #[test]
    fn decide_matches_todays_claims() {
        let input = model_input("export 500s");
        let sort = Effect::Classify {
            connector: "c5".into(),
            node: nid("sort"),
            input: input.clone(),
        };
        let open = |connector: &str, prompt: &str| Effect::Open {
            owner: connector.into(),
            prompt: prompt.into(),
            seeded: false,
        };
        let claim = |connector: &str| Effect::Claim {
            connector: connector.into(),
            key: THREAD.into(),
        };

        let doubt = vec![sort.clone(), claim("c6"), open("c6", "answer the doubt")];
        let error = vec![sort.clone(), claim("c2"), open("c2", "triage the error")];
        let page = vec![
            sort.clone(),
            Effect::Page {
                connector: "c5".into(),
                group: "prod-engg".into(),
            },
            claim("c5"),
        ];
        let stop = vec![sort.clone(), claim("c5")];

        let check =
            |fallback: Fallback, index: usize, probabilities: &[f64], expected: Vec<Effect>| {
                let names = ["error", "doubt", "escalation", "ignore"];
                let lib = router_library(fallback);
                run_effects(
                    &lib,
                    "c5",
                    slack(),
                    "export 500s",
                    answers(&[("category", names[index])], Some((index, probabilities))),
                    expected,
                );
            };

        check(
            Fallback::Existing { port: pid("doubt") },
            0,
            &[0.55, 0.30, 0.10, 0.05],
            doubt,
        );
        check(
            Fallback::None,
            2,
            &[0.03, 0.10, 0.84, 0.03],
            vec![sort.clone(), claim("c6"), open("c6", "answer the doubt")],
        );
        check(Fallback::None, 2, &[0.05, 0.05, 0.85, 0.05], page);
        check(Fallback::None, 3, &[0.05, 0.05, 0.05, 0.85], stop.clone());
        check(Fallback::None, 0, &[0.91, 0.03, 0.03, 0.03], error.clone());
        check(
            Fallback::Existing { port: pid("doubt") },
            0,
            &[0.6, 0.25, 0.10, 0.05],
            error,
        );
        check(
            Fallback::Existing {
                port: pid("escalation"),
            },
            0,
            &[0.55, 0.30, 0.10, 0.05],
            vec![sort.clone(), claim("c2"), open("c2", "triage the error")],
        );

        let nowhere = library(vec![(
            "c5",
            graph(
                vec![
                    node("t", "Trigger", trigger()),
                    node(
                        "sort",
                        "Sort",
                        decide(&[("escalation", "escalation")], Fallback::None),
                    ),
                    node(
                        "page",
                        "Page",
                        NodeKind::Page {
                            group: "prod-engg".into(),
                        },
                    ),
                    node("stop", "Stop", NodeKind::Stop),
                ],
                vec![
                    edge("t", "out", "sort"),
                    edge("sort", "escalation", "page"),
                    edge("sort", "nowhere", "stop"),
                ],
            ),
        )]);
        let run = run_effects(
            &nowhere,
            "c5",
            slack(),
            "export 500s",
            answers(&[("category", "escalation")], Some((0, &[0.6]))),
            vec![
                Effect::Classify {
                    connector: "c5".into(),
                    node: nid("sort"),
                    input,
                },
                claim("c5"),
            ],
        );
        assert_eq!(run.session_owner, None);
        assert_eq!(
            run.log
                .iter()
                .find(|entry| entry.what.get("port").is_some())
                .unwrap()
                .what["port"],
            json!("nowhere")
        );
    }

    #[test]
    fn call_into_another_graph_keeps_the_session() {
        let lib = library(vec![
            (
                "c1",
                graph(
                    vec![
                        node("t", "Trigger", trigger()),
                        node("a", "Open", talk("open")),
                        node(
                            "c",
                            "Call",
                            NodeKind::Call {
                                workflow: "c6".into(),
                            },
                        ),
                        node("b", "After", talk("after")),
                    ],
                    vec![
                        edge("t", "out", "a"),
                        edge("a", "out", "c"),
                        edge("c", "out", "b"),
                    ],
                ),
            ),
            (
                "c6",
                chain(vec![
                    ("t", "Trigger", trigger()),
                    ("k", "Inside", talk("inside")),
                ]),
            ),
        ]);
        let run = run_effects(
            &lib,
            "c1",
            linear(),
            "ENG-7: Broken button",
            answers(&[], None),
            vec![
                Effect::Claim {
                    connector: "c1".into(),
                    key: "uuid".into(),
                },
                Effect::Open {
                    owner: "c1".into(),
                    prompt: "open".into(),
                    seeded: false,
                },
                Effect::Type {
                    text: "inside".into(),
                },
                Effect::Type {
                    text: "after".into(),
                },
            ],
        );
        assert_eq!(run.session_owner.as_deref(), Some("c1"));
        assert_eq!(
            run.log[2],
            Entry {
                at: AT.into(),
                connector: "c1".into(),
                node: nid("c"),
                what: json!({ "call": "c6" })
            }
        );
        assert_eq!(run.log[3].connector, "c6");
        assert_eq!(run.log[3].node, nid("k"));
        assert_eq!(run.log[3].what, json!({ "talk": "inside" }));
    }

    #[test]
    fn a_call_already_on_the_stack_fails_the_run() {
        let lib = library(vec![
            (
                "c1",
                chain(vec![
                    ("t", "Trigger", trigger()),
                    (
                        "c",
                        "Call",
                        NodeKind::Call {
                            workflow: "c6".into(),
                        },
                    ),
                ]),
            ),
            (
                "c6",
                chain(vec![
                    ("t", "Trigger", trigger()),
                    (
                        "c",
                        "Call",
                        NodeKind::Call {
                            workflow: "c1".into(),
                        },
                    ),
                ]),
            ),
        ]);
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        assert_eq!(drive(&mut run, &lib), Drive::Done);
        assert_eq!(
            run.status,
            Status::Failed {
                why: "call to workflow \"c1\" cycles: it is already on the stack (c1 -> c6 -> c1)"
                    .into()
            }
        );
    }

    #[test]
    fn a_call_with_no_graph_fails_the_run() {
        let lib = library(vec![(
            "c1",
            chain(vec![
                ("t", "Trigger", trigger()),
                (
                    "c",
                    "Call",
                    NodeKind::Call {
                        workflow: "missing".into(),
                    },
                ),
            ]),
        )]);
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        assert_eq!(drive(&mut run, &lib), Drive::Done);
        assert_eq!(
            run.status,
            Status::Failed {
                why: "call to workflow \"missing\", which has no graph".into()
            }
        );
    }

    #[test]
    fn tab_gone_resumes_and_a_missing_conversation_opens_seeded() {
        let lib = two_talks();
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        assert_eq!(
            act(&mut run, &lib),
            Effect::Claim {
                connector: "c1".into(),
                key: "uuid".into()
            }
        );
        supply(&mut run, Supplied::Done, AT);
        assert_eq!(
            act(&mut run, &lib),
            Effect::Open {
                owner: "c1".into(),
                prompt: "first".into(),
                seeded: false
            }
        );
        supply(&mut run, opened(), AT);
        assert_eq!(drive(&mut run, &lib), Drive::Park);
        push(
            &mut run,
            Wake::TurnEnded {
                claude: Some("ses_1".into()),
            },
            AT,
        );
        push(&mut run, Wake::TabGone, AT);
        assert_eq!(
            act(&mut run, &lib),
            Effect::Resume {
                owner: "c1".into(),
                claude: "ses_1".into(),
                text: "second".into()
            }
        );
        supply(&mut run, Supplied::NoConversation, AT);
        let log_json = serde_json::to_string(&run.log).unwrap();
        let context_json = serde_json::to_string(&run.context).unwrap();
        match act(&mut run, &lib) {
            Effect::Open {
                owner,
                prompt,
                seeded,
            } => {
                assert_eq!((owner, seeded), ("c1".into(), true));
                assert!(prompt.contains("This session continues a run that already happened."));
                assert!(prompt.contains(&log_json), "{prompt}");
                assert!(prompt.contains(&context_json), "{prompt}");
                assert!(prompt.contains("{\"talk\":\"first\"}"), "{prompt}");
                assert!(prompt.ends_with("second"), "{prompt}");
            }
            other => panic!("seeded open, got {other:?}"),
        }
    }

    #[test]
    fn detached_without_a_claude_id_opens_seeded() {
        let lib = two_talks();
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        supply_claim_and_open(&mut run, &lib, "first");
        assert_eq!(drive(&mut run, &lib), Drive::Park);
        push(&mut run, Wake::TabGone, AT);
        match act(&mut run, &lib) {
            Effect::Open {
                owner,
                prompt,
                seeded,
            } => {
                assert_eq!((owner, seeded), ("c1".into(), true));
                assert!(prompt.contains("This session continues a run that already happened."));
                assert!(prompt.contains("{\"talk\":\"first\"}"), "{prompt}");
                assert!(prompt.ends_with("first"), "{prompt}");
            }
            other => panic!("seeded open, got {other:?}"),
        }
    }

    #[test]
    fn drive_replay_returns_the_same_effect_and_a_supplied_classify_is_not_asked_again() {
        let lib = library(vec![("c1", choice_talks(Fallback::None))]);
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        let first = drive(&mut run, &lib);
        assert_eq!(drive(&mut run, &lib), first);
        let Drive::Act(Effect::Classify { node, .. }) = &first else {
            panic!("{first:?}")
        };
        assert_eq!(node, &nid("sort"));
        let classified = answers(&[("kind", "bug")], None);
        supply(&mut run, Supplied::Classified(classified.clone()), AT);
        let entries = run.log.len();
        let context = run.context.clone();
        supply(&mut run, Supplied::Classified(classified), AT);
        assert_eq!(run.log.len(), entries);
        assert_eq!(run.context, context);
        assert_eq!(run.context["Sort"]["kind"], json!("bug"));
        let third = drive(&mut run, &lib);
        assert_eq!(drive(&mut run, &lib), third);
        assert_eq!(
            third,
            Drive::Act(Effect::Claim {
                connector: "c1".into(),
                key: "uuid".into()
            })
        );
    }

    #[test]
    fn a_run_serialized_mid_wait_continues_after_deserialize() {
        let lib = two_talks();
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        let pending = drive(&mut run, &lib);
        let saved = serde_json::to_string(&run).unwrap();
        let mut restored: super::Run = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored, run);
        assert_eq!(drive(&mut restored, &lib), pending);
        supply(&mut restored, Supplied::Done, AT);
        assert_eq!(
            act(&mut restored, &lib),
            Effect::Open {
                owner: "c1".into(),
                prompt: "first".into(),
                seeded: false
            }
        );
        supply(&mut restored, opened(), AT);
        assert_eq!(drive(&mut restored, &lib), Drive::Park);
        let waiting = serde_json::to_string(&restored).unwrap();
        let mut again: super::Run = serde_json::from_str(&waiting).unwrap();
        assert_eq!(again, restored);
        assert_eq!(drive(&mut again, &lib), Drive::Park);
        push(
            &mut again,
            Wake::TurnEnded {
                claude: Some("ses_9".into()),
            },
            AT,
        );
        assert_eq!(
            act(&mut again, &lib),
            Effect::Type {
                text: "second".into()
            }
        );
        assert_eq!(
            again.session,
            Session::Live {
                machine: "mac".into(),
                tab: "tab-1".into(),
                claude: Some("ses_9".into())
            }
        );
    }

    #[test]
    fn a_resumed_tab_stays_the_same_session() {
        let lib = library(vec![(
            "c1",
            chain(vec![
                ("t", "Trigger", trigger()),
                ("a", "First", talk("first")),
                ("b", "Second", talk("second")),
                ("c", "Third", talk("third")),
            ]),
        )]);
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        supply_claim_and_open(&mut run, &lib, "first");
        assert_eq!(drive(&mut run, &lib), Drive::Park);
        push(
            &mut run,
            Wake::TurnEnded {
                claude: Some("ses_1".into()),
            },
            AT,
        );
        push(&mut run, Wake::TabGone, AT);
        assert_eq!(
            act(&mut run, &lib),
            Effect::Resume {
                owner: "c1".into(),
                claude: "ses_1".into(),
                text: "second".into()
            }
        );
        supply(&mut run, opened(), AT);
        assert_eq!(drive(&mut run, &lib), Drive::Park);
        assert_eq!(
            run.session,
            Session::Live {
                machine: "mac".into(),
                tab: "tab-1".into(),
                claude: Some("ses_1".into())
            }
        );
        push(
            &mut run,
            Wake::TurnEnded {
                claude: Some("ses_1".into()),
            },
            AT,
        );
        assert_eq!(
            act(&mut run, &lib),
            Effect::Type {
                text: "third".into()
            }
        );
        assert_eq!(run.session_owner.as_deref(), Some("c1"));
    }

    #[test]
    fn a_failed_supply_fails_the_run() {
        let lib = library(vec![("c1", choice_talks(Fallback::None))]);
        let mut run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        let _ = act(&mut run, &lib);
        supply(&mut run, Supplied::Failed("gateway down".into()), AT);
        assert_eq!(
            run.status,
            Status::Failed {
                why: "gateway down".into()
            }
        );
        assert_eq!(drive(&mut run, &lib), Drive::Done);
        supply(&mut run, Supplied::Failed("again".into()), AT);
        assert_eq!(
            run.status,
            Status::Failed {
                why: "gateway down".into()
            }
        );
    }

    #[test]
    fn history_is_a_preface_plus_the_log_and_context_json() {
        let lib = library(vec![(
            "c1",
            graph(vec![node("t", "Trigger", trigger())], vec![]),
        )]);
        let run = start(&lib, "c1", linear(), json!("ENG-7: Broken button"), AT);
        let body =
            serde_json::to_string(&json!({ "log": run.log, "context": run.context })).unwrap();
        assert_eq!(
            history(&run),
            format!("This session continues a run that already happened. Everything that happened so far is in this JSON.\n{body}")
        );
        assert!(body.contains("ENG-7: Broken button"));
    }

    #[test]
    fn port_for_direct_and_decide() {
        let pages = |port: &PortId| port.0 == "escalation" || port.0 == "fallback";
        let direct = classify_of(
            "kind",
            &[("bug", "bug"), ("feat", "feat")],
            Gate::Direct,
            Fallback::None,
            Some("kind"),
        );
        assert_eq!(
            port_for(&direct, &answers(&[("kind", "Bug")], None), &pages).0,
            "bug"
        );
        let extra = classify_of(
            "kind",
            &[("bug", "bug")],
            Gate::Direct,
            Fallback::Extra,
            Some("kind"),
        );
        assert_eq!(
            port_for(&extra, &answers(&[("kind", "nope")], None), &pages).0,
            "fallback"
        );
        let plain = classify_of("note", &[], Gate::Direct, Fallback::None, None);
        assert_eq!(
            port_for(&plain, &answers(&[("note", "free")], None), &pages).0,
            "out"
        );

        let decide = classify_of(
            "category",
            &[
                ("error", "error"),
                ("doubt", "doubt"),
                ("escalation", "escalation"),
                ("ignore", "ignore"),
            ],
            Gate::Decide {
                unsure_below: 0.6,
                page_at: 0.85,
            },
            Fallback::Existing { port: pid("doubt") },
            Some("category"),
        );
        assert_eq!(
            port_for(
                &decide,
                &answers(
                    &[("category", "error")],
                    Some((0, &[0.55, 0.30, 0.10, 0.05]))
                ),
                &pages
            )
            .0,
            "doubt"
        );
        let no_fallback = classify_of(
            "category",
            &[
                ("error", "error"),
                ("doubt", "doubt"),
                ("escalation", "escalation"),
                ("ignore", "ignore"),
            ],
            Gate::Decide {
                unsure_below: 0.6,
                page_at: 0.85,
            },
            Fallback::None,
            Some("category"),
        );
        assert_eq!(
            port_for(
                &no_fallback,
                &answers(
                    &[("category", "escalation")],
                    Some((2, &[0.03, 0.10, 0.84, 0.03]))
                ),
                &pages
            )
            .0,
            "doubt"
        );
        assert_eq!(
            port_for(
                &no_fallback,
                &answers(
                    &[("category", "escalation")],
                    Some((2, &[0.05, 0.05, 0.85, 0.05]))
                ),
                &pages
            )
            .0,
            "escalation"
        );
        assert_eq!(
            port_for(
                &no_fallback,
                &answers(
                    &[("category", "escalation")],
                    Some((2, &[0.20, 0.20, 0.50, 0.10]))
                ),
                &pages
            )
            .0,
            "error"
        );
        let only = classify_of(
            "category",
            &[("escalation", "escalation")],
            Gate::Decide {
                unsure_below: 0.6,
                page_at: 0.85,
            },
            Fallback::None,
            Some("category"),
        );
        assert_eq!(
            port_for(
                &only,
                &answers(&[("category", "escalation")], Some((0, &[0.6]))),
                &pages
            )
            .0,
            "nowhere"
        );
    }

    fn two_talks() -> Library {
        library(vec![(
            "c1",
            chain(vec![
                ("t", "Trigger", trigger()),
                ("a", "First", talk("first")),
                ("b", "Second", talk("second")),
            ]),
        )])
    }

    fn supply_claim_and_open(run: &mut super::Run, library: &Library, prompt: &str) {
        assert_eq!(
            act(run, library),
            Effect::Claim {
                connector: "c1".into(),
                key: "uuid".into()
            }
        );
        supply(run, Supplied::Done, AT);
        assert_eq!(
            act(run, library),
            Effect::Open {
                owner: "c1".into(),
                prompt: prompt.into(),
                seeded: false
            }
        );
        supply(run, opened(), AT);
    }

    fn choice_talks(fallback: Fallback) -> Graph {
        graph(
            vec![
                node("t", "Trigger", trigger()),
                node(
                    "sort",
                    "Sort",
                    direct("kind", &[("bug", "bug"), ("feat", "feat")], fallback),
                ),
                node("bug", "Bug", talk("saw bug")),
                node("feat", "Feat", talk("saw feat")),
            ],
            vec![
                edge("t", "out", "sort"),
                edge("sort", "bug", "bug"),
                edge("sort", "feat", "feat"),
            ],
        )
    }

    fn fallback_graph() -> Graph {
        graph(
            vec![
                node("t", "Trigger", trigger()),
                node(
                    "sort",
                    "Sort",
                    direct("kind", &[("bug", "bug"), ("feat", "feat")], Fallback::Extra),
                ),
                node("bug", "Bug", talk("saw bug")),
                node("feat", "Feat", talk("saw feat")),
                node("fb", "Fallback", talk("use fallback")),
            ],
            vec![
                edge("t", "out", "sort"),
                edge("sort", "bug", "bug"),
                edge("sort", "feat", "feat"),
                edge("sort", "fallback", "fb"),
            ],
        )
    }

    fn direct(branch: &str, choices: &[(&str, &str)], fallback: Fallback) -> NodeKind {
        NodeKind::Classify(classify_of(
            branch,
            choices,
            Gate::Direct,
            fallback,
            Some(branch),
        ))
    }

    fn decide(choices: &[(&str, &str)], fallback: Fallback) -> NodeKind {
        NodeKind::Classify(classify_of(
            "category",
            choices,
            Gate::Decide {
                unsure_below: 0.6,
                page_at: 0.85,
            },
            fallback,
            Some("category"),
        ))
    }

    fn classify_of(
        branch: &str,
        choices: &[(&str, &str)],
        gate: Gate,
        fallback: Fallback,
        branch_on: Option<&str>,
    ) -> Classify {
        Classify {
            model: "typesafe-ai/jev".into(),
            instructions: "Sort it.".into(),
            questions: vec![Question {
                name: branch.into(),
                ask: "Which?".into(),
                choices: choices
                    .iter()
                    .map(|(id, label)| choice(id, label))
                    .collect(),
            }],
            branch_on: branch_on.map(str::to_owned),
            gate,
            fallback,
        }
    }

    fn router_library(fallback: Fallback) -> Library {
        library(vec![
            (
                "c5",
                graph(
                    vec![
                        node("t", "Trigger", trigger()),
                        node(
                            "sort",
                            "Sort",
                            decide(
                                &[
                                    ("error", "error"),
                                    ("doubt", "doubt"),
                                    ("escalation", "escalation"),
                                    ("ignore", "ignore"),
                                ],
                                fallback,
                            ),
                        ),
                        node(
                            "call2",
                            "Call mention",
                            NodeKind::Call {
                                workflow: "c2".into(),
                            },
                        ),
                        node(
                            "call6",
                            "Call answer",
                            NodeKind::Call {
                                workflow: "c6".into(),
                            },
                        ),
                        node(
                            "page",
                            "Page",
                            NodeKind::Page {
                                group: "prod-engg".into(),
                            },
                        ),
                        node("stop-page", "Stop page", NodeKind::Stop),
                        node("stop-ignore", "Stop ignore", NodeKind::Stop),
                        node("stop-nowhere", "Stop nowhere", NodeKind::Stop),
                    ],
                    vec![
                        edge("t", "out", "sort"),
                        edge("sort", "error", "call2"),
                        edge("sort", "doubt", "call6"),
                        edge("sort", "escalation", "page"),
                        edge("page", "out", "stop-page"),
                        edge("sort", "ignore", "stop-ignore"),
                        edge("sort", "nowhere", "stop-nowhere"),
                    ],
                ),
            ),
            (
                "c2",
                chain(vec![
                    ("t", "Trigger", trigger()),
                    ("k", "Talk", talk("triage the error")),
                ]),
            ),
            (
                "c6",
                chain(vec![
                    ("t", "Trigger", trigger()),
                    ("k", "Talk", talk("answer the doubt")),
                ]),
            ),
        ])
    }
}
