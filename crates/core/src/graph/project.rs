//! The legacy fields an app that predates graphs reads, derived from a graph.

use super::{Fallback, Gate, Graph, Node, NodeId, NodeKind, PortId, Watch};
use crate::connectors::{Ask, Category, Decide, Question, Trigger};

/// What an older app sees of a graph workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct Projected {
    pub trigger: Trigger,
    /// Present when the graph branches with `Gate::Decide`. The same value sits on
    /// `Trigger::SlackMessage`.
    pub decide: Option<Decide>,
    pub ask: Option<Ask>,
}

/// Lossy once the graph is more than what `migrate` builds. It keeps the trigger, the first
/// direct classify before the first talk, and that classify's switch cases as `stop_on`.
pub fn project(graph: &Graph) -> Projected {
    let trigger = graph.trigger();
    let NodeKind::Trigger { watch } = &trigger.kind else { unreachable!("trigger() returns the trigger") };
    let asks = direct_asks(graph, trigger);
    let decide = graph.nodes.iter().find_map(|node| match &node.kind {
        NodeKind::Classify(classify) if matches!(classify.gate, Gate::Decide { .. }) => Some(decide_of(graph, node)),
        _ => None,
    });
    let trigger = match watch {
        Watch::LinearLabel { label } => Trigger::LinearLabel { label: label.clone() },
        Watch::SlackMention { channel } => Trigger::SlackMention { channel: channel.clone() },
        Watch::Called => Trigger::Called,
        Watch::SlackMessage { channel } => Trigger::SlackMessage { channel: channel.clone(), decide: decide.clone().unwrap_or_else(empty_decide) },
    };
    Projected { ask: asks.first().map(|node| ask_of(graph, node)), decide, trigger }
}

fn empty_decide() -> Decide {
    Decide { model: String::new(), instructions: String::new(), categories: Vec::new(), fallback: None }
}

/// Direct classifies on the spine, in walk order, stopping at the first talk or decide.
fn direct_asks<'a>(graph: &'a Graph, trigger: &'a Node) -> Vec<&'a Node> {
    let mut asks = Vec::new();
    let mut current = trigger;
    while let Some(next) = follow(graph, current) {
        match &next.kind {
            NodeKind::Classify(classify) if matches!(classify.gate, Gate::Decide { .. }) => break,
            NodeKind::Classify(_) => {
                asks.push(next);
                current = next;
            }
            NodeKind::Talk { .. } | NodeKind::Stop => break,
            _ => current = next,
        }
    }
    asks
}

fn follow<'a>(graph: &'a Graph, node: &'a Node) -> Option<&'a Node> {
    if let NodeKind::Classify(classify) = &node.kind {
        if matches!(classify.gate, Gate::Decide { .. }) {
            return None;
        }
        if classify.branch_on.is_some() {
            for port in node.kind.ports() {
                for target_id in graph.next(&node.id, &port.id) {
                    let target = graph.node(target_id)?;
                    if !matches!(target.kind, NodeKind::Stop) {
                        return Some(target);
                    }
                }
            }
            return None;
        }
    }
    let out = PortId(PortId::OUT.into());
    graph.node(graph.next(&node.id, &out).into_iter().next()?)
}

fn ask_of(graph: &Graph, node: &Node) -> Ask {
    let NodeKind::Classify(classify) = &node.kind else { unreachable!("direct_asks yields classifies") };
    let questions = classify
        .questions
        .iter()
        .map(|question| Question {
            name: question.name.clone(),
            ask: question.ask.clone(),
            choices: question.choices.iter().map(|choice| choice.label.clone()).collect(),
            stop_on: stop_on(graph, &node.id, &question.name),
        })
        .collect();
    Ask { model: classify.model.clone(), instructions: classify.instructions.clone(), questions }
}

fn stop_on(graph: &Graph, ask: &NodeId, question: &str) -> Vec<String> {
    graph
        .nodes
        .iter()
        .find_map(|node| {
            let NodeKind::Switch(switch) = &node.kind else { return None };
            (switch.classify == *ask && switch.question == question).then(|| switch.cases.iter().map(|case| case.equals.clone()).collect())
        })
        .unwrap_or_default()
}

fn decide_of(graph: &Graph, node: &Node) -> Decide {
    let NodeKind::Classify(classify) = &node.kind else { unreachable!("caller matches Classify") };
    let question = classify
        .branch_on
        .as_ref()
        .and_then(|name| classify.questions.iter().find(|question| &question.name == name))
        .or_else(|| classify.questions.first());
    let categories = question.map(|question| question.choices.iter().map(|choice| category_of(graph, node, choice)).collect()).unwrap_or_default();
    let fallback = match &classify.fallback {
        Fallback::Existing { port } => Some(
            question.and_then(|question| question.choices.iter().find(|choice| &choice.id == port)).map(|choice| choice.label.clone()).unwrap_or_else(|| port.0.clone()),
        ),
        Fallback::None | Fallback::Extra => None,
    };
    Decide { model: classify.model.clone(), instructions: classify.instructions.clone(), categories, fallback }
}

fn category_of(graph: &Graph, classify: &Node, choice: &super::Choice) -> Category {
    let mut mention = None;
    let mut workflow = None;
    let mut next = graph.next(&classify.id, &choice.id);
    if let Some(node) = next.first().and_then(|id| graph.node(id)) {
        if let NodeKind::Page { group } = &node.kind {
            mention = Some(group.clone());
            next = graph.next(&node.id, &PortId(PortId::OUT.into()));
        }
    }
    if let Some(node) = next.first().and_then(|id| graph.node(id)) {
        if let NodeKind::Call { workflow: target } = &node.kind {
            workflow = Some(target.clone());
        }
    }
    Category { name: choice.label.clone(), description: choice.description.clone(), workflow, mention }
}
