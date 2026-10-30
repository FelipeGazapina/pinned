//! The graph a fixed-pipeline connector already is.

use super::{Choice, Classify, Edge, Fallback, Gate, Graph, Node, NodeId, NodeKind, PortId, Question, Watch};
use crate::connectors::{self, Ask, Category, Connector, Decide, Question as LegacyQuestion, Trigger};
use std::collections::BTreeMap;

const COLUMN: i32 = 240;
const ROW: i32 = 120;

#[derive(Clone)]
struct Tail {
    from: NodeId,
    port: String,
}

struct Built {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    /// The next free row in each column. Spine nodes take row 0. Branches stack under them.
    rows: BTreeMap<i32, i32>,
}

impl Built {
    fn add(&mut self, id: &str, name: &str, column: i32, row: i32, kind: NodeKind) -> NodeId {
        let id = NodeId(id.into());
        self.nodes.push(Node { id: id.clone(), name: name.into(), position: [column * COLUMN, row * ROW], kind });
        let next = self.rows.entry(column).or_insert(0);
        *next = (*next).max(row + 1);
        id
    }

    fn stack(&mut self, column: i32) -> i32 {
        let next = self.rows.entry(column).or_insert(0);
        let row = (*next).max(1);
        *next = row + 1;
        row
    }

    fn edge(&mut self, from: &NodeId, port: &str, to: &NodeId) {
        self.edges.push(Edge { from: from.clone(), port: PortId(port.into()), to: to.clone() });
    }

    fn link(&mut self, tails: &[Tail], to: &NodeId) {
        for tail in tails {
            self.edge(&tail.from, &tail.port, to);
        }
    }

    fn finish(self) -> Graph {
        Graph { nodes: self.nodes, edges: self.edges }
    }
}

/// Builds the graph `connector`'s legacy fields describe, with positions laid out left to right.
/// It adds no branch the pipeline lacked: `ask` choices in `stop_on` lead to `stop`, the others
/// share the rest of the chain; a slack-message `decide` becomes a `Gate::Decide` classify whose
/// categories route to `call`, `page` or `stop`. Deterministic, so the same connector always gives
/// an equal graph: that is how a stored graph is told apart from an edited one.
///
/// Several questions with `stop_on` become a chain of classifies, one per question that can
/// actually branch, in question order. Today's ask stops at the first matching answer, so a later
/// classify is only reached when the earlier choice continues. Each stop string is a `Stop` node
/// named with that exact string and id `stop-{question index}-{stop index}`, which is how `project`
/// gives the list back, including a string that matches no choice.
pub fn migrate(connector: &Connector) -> Graph {
    let mut built = Built { nodes: Vec::new(), edges: Vec::new(), rows: BTreeMap::new() };
    let trigger = built.add("trigger", "Trigger", 0, 0, NodeKind::Trigger { watch: watch_of(&connector.trigger) });
    let mut column = 1;
    let mut tails = vec![Tail { from: trigger, port: PortId::OUT.into() }];
    if let Some(ask) = &connector.ask {
        let laid = lay_ask(&mut built, column, &tails, ask);
        column = laid.0;
        tails = laid.1;
    }
    if let Trigger::SlackMessage { decide, .. } = &connector.trigger {
        lay_decide(&mut built, column, &tails, decide);
        return built.finish();
    }
    if !tails.is_empty() && !connector.columns.picked.trim().is_empty() {
        let stage = built.add("stage", "Picked", column, 0, NodeKind::StagePicked);
        built.link(&tails, &stage);
        tails = vec![Tail { from: stage, port: PortId::OUT.into() }];
        column += 1;
    }
    if !tails.is_empty() {
        let talk = built.add("talk", "Talk", column, 0, NodeKind::Talk { prompt: String::new() });
        built.link(&tails, &talk);
    }
    built.finish()
}

fn watch_of(trigger: &Trigger) -> Watch {
    match trigger {
        Trigger::LinearLabel { label } => Watch::LinearLabel { label: label.clone() },
        Trigger::SlackMention { channel } => Watch::SlackMention { channel: channel.clone() },
        Trigger::SlackMessage { channel, .. } => Watch::SlackMessage { channel: channel.clone() },
        Trigger::Called => Watch::Called,
    }
}

fn lay_ask(built: &mut Built, mut column: i32, tails: &[Tail], ask: &Ask) -> (i32, Vec<Tail>) {
    let questions: Vec<Question> = ask.questions.iter().map(graph_question).collect();
    let branch: Vec<usize> = ask
        .questions
        .iter()
        .enumerate()
        .filter(|(_, question)| question.choices.iter().any(|choice| question.stop_on.iter().any(|stop| stop.eq_ignore_ascii_case(choice))))
        .map(|(index, _)| index)
        .collect();
    if branch.is_empty() {
        let id = built.add("ask", "Ask", column, 0, classify(ask, questions, None));
        built.link(tails, &id);
        for (index, question) in ask.questions.iter().enumerate() {
            place_stops(built, index, question, column + 1, None);
        }
        return (column + 1, vec![Tail { from: id, port: PortId::OUT.into() }]);
    }
    let mut incoming = tails.to_vec();
    let mut placed = Vec::new();
    for (step, question_index) in branch.iter().copied().enumerate() {
        let question = &ask.questions[question_index];
        let id_text = if step == 0 { "ask".to_owned() } else { format!("ask-{question_index}") };
        let name = if step == 0 { "Ask".to_owned() } else { format!("Ask {}", question.name) };
        let id = built.add(&id_text, &name, column, 0, classify(ask, questions.clone(), Some(question.name.clone())));
        built.link(&incoming, &id);
        place_stops(built, question_index, question, column + 1, Some(&id));
        placed.push(question_index);
        if step == 0 {
            for (index, other) in ask.questions.iter().enumerate() {
                if placed.contains(&index) || branch.contains(&index) {
                    continue;
                }
                place_stops(built, index, other, column + 1, None);
            }
        }
        incoming = continue_tails(&id, question);
        column += 1;
        if incoming.is_empty() {
            for index in branch.iter().copied().filter(|index| !placed.contains(index)) {
                place_stops(built, index, &ask.questions[index], column, None);
            }
            break;
        }
    }
    (column, incoming)
}

fn classify(ask: &Ask, questions: Vec<Question>, branch_on: Option<String>) -> NodeKind {
    NodeKind::Classify(Classify {
        model: ask.model.clone(),
        instructions: ask.instructions.clone(),
        questions,
        branch_on,
        gate: Gate::Direct,
        fallback: Fallback::None,
    })
}

fn lay_decide(built: &mut Built, column: i32, tails: &[Tail], decide: &Decide) {
    let choices: Vec<Choice> = decide.categories.iter().enumerate().map(|(index, category)| category_choice(category, index, &decide.categories)).collect();
    let fallback = match &decide.fallback {
        Some(name) => {
            let port = choices.iter().find(|choice| &choice.label == name).map(|choice| choice.id.clone()).unwrap_or_else(|| PortId(name.clone()));
            Fallback::Existing { port }
        }
        None => Fallback::None,
    };
    let id = built.add(
        "decide",
        "Decide",
        column,
        0,
        NodeKind::Classify(Classify {
            model: decide.model.clone(),
            instructions: decide.instructions.clone(),
            questions: vec![Question { name: "category".into(), ask: String::new(), choices: choices.clone() }],
            branch_on: Some("category".into()),
            gate: Gate::Decide { unsure_below: connectors::UNSURE_BELOW, page_at: connectors::MIN_CONFIDENCE_TO_PAGE },
            fallback,
        }),
    );
    built.link(tails, &id);
    for (index, category) in decide.categories.iter().enumerate() {
        let port = choices[index].id.0.clone();
        let key = node_key(&port);
        let mut from = id.clone();
        let mut from_port = port;
        let mut at_column = column + 1;
        if let Some(group) = &category.mention {
            let page = built.add(&format!("page-{key}"), group, at_column, index as i32, NodeKind::Page { group: group.clone() });
            built.edge(&from, &from_port, &page);
            from = page;
            from_port = PortId::OUT.into();
            at_column += 1;
        }
        let next = match &category.workflow {
            Some(workflow) => built.add(&format!("call-{key}"), &category.name, at_column, index as i32, NodeKind::Call { workflow: workflow.clone() }),
            None => built.add(&format!("stop-{key}"), &category.name, at_column, index as i32, NodeKind::Stop),
        };
        built.edge(&from, &from_port, &next);
    }
    let nowhere = built.add("nowhere", "nowhere", column + 1, decide.categories.len() as i32, NodeKind::Stop);
    built.edge(&id, PortId::NOWHERE, &nowhere);
}

fn category_choice(category: &Category, index: usize, categories: &[Category]) -> Choice {
    let id = if categories.iter().filter(|other| other.name == category.name).count() == 1 {
        category.name.clone()
    } else {
        format!("{}#{index}", category.name)
    };
    Choice { id: PortId(id), label: category.name.clone(), description: category.description.clone() }
}

fn node_key(port: &str) -> String {
    port.replace('#', "-")
}

fn graph_question(question: &LegacyQuestion) -> Question {
    Question {
        name: question.name.clone(),
        ask: question.ask.clone(),
        choices: choice_ids(question).into_iter().map(|(label, id)| Choice { id: PortId(id), label, description: String::new() }).collect(),
    }
}

fn choice_ids(question: &LegacyQuestion) -> Vec<(String, String)> {
    question
        .choices
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let id = if question.choices.iter().filter(|choice| *choice == label).count() == 1 { label.clone() } else { format!("{label}#{index}") };
            (label.clone(), id)
        })
        .collect()
}

fn continue_tails(from: &NodeId, question: &LegacyQuestion) -> Vec<Tail> {
    choice_ids(question)
        .into_iter()
        .filter(|(label, _)| !question.stop_on.iter().any(|stop| stop.eq_ignore_ascii_case(label)))
        .map(|(_, port)| Tail { from: from.clone(), port })
        .collect()
}

fn place_stops(built: &mut Built, question_index: usize, question: &LegacyQuestion, column: i32, from: Option<&NodeId>) {
    let ports = choice_ids(question);
    for (index, stop) in question.stop_on.iter().enumerate() {
        let row = built.stack(column);
        let id = built.add(&format!("stop-{question_index}-{index}"), stop, column, row, NodeKind::Stop);
        let Some(from) = from else { continue };
        if let Some((_, port)) = ports.iter().find(|(label, _)| label.eq_ignore_ascii_case(stop)) {
            built.edge(from, port, &id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::{Ask, Category, Columns, Connector, Decide, Effort, Fix, PermissionMode, Question as LegacyQuestion, RunsOn, Trigger};
    use crate::graph::project::project;
    use std::collections::BTreeMap;

    fn workflow(trigger: Trigger, ask: Option<Ask>, picked: &str) -> Connector {
        Connector {
            id: "c1".into(),
            name: "Workflow".into(),
            enabled: true,
            trigger,
            folder: "/repo".into(),
            model: String::new(),
            effort: Effort::High,
            permission_mode: PermissionMode::Plan,
            worktree: false,
            instructions: String::new(),
            columns: Columns { picked: picked.into(), coding: String::new(), review: String::new() },
            since: String::new(),
            handled: vec![],
            team: String::new(),
            label: String::new(),
            fix: Fix::Off,
            created: BTreeMap::new(),
            followed: BTreeMap::new(),
            paged: BTreeMap::new(),
            proof: false,
            ask,
            runs_on: RunsOn::Any,
            repo: String::new(),
            account: String::new(),
            graph: None,
        }
    }

    fn at(column: i32, row: i32) -> [i32; 2] {
        [column * COLUMN, row * ROW]
    }

    fn node(id: &str, name: &str, at: [i32; 2], kind: NodeKind) -> Node {
        Node { id: NodeId(id.into()), name: name.into(), position: at, kind }
    }

    fn edge(from: &str, port: &str, to: &str) -> Edge {
        Edge { from: NodeId(from.into()), port: PortId(port.into()), to: NodeId(to.into()) }
    }

    fn choice(id: &str, label: &str, description: &str) -> Choice {
        Choice { id: PortId(id.into()), label: label.into(), description: description.into() }
    }

    fn legacy_question(name: &str, ask: &str, choices: &[&str], stop_on: &[&str]) -> LegacyQuestion {
        LegacyQuestion {
            name: name.into(),
            ask: ask.into(),
            choices: choices.iter().map(|choice| (*choice).to_owned()).collect(),
            stop_on: stop_on.iter().map(|stop| (*stop).to_owned()).collect(),
        }
    }

    fn round_trip(connector: &Connector) {
        let projected = project(&migrate(connector));
        assert_eq!(projected.trigger, connector.trigger);
        assert_eq!(projected.ask, connector.ask);
        let decide = match &connector.trigger {
            Trigger::SlackMessage { decide, .. } => Some(decide.clone()),
            _ => None,
        };
        assert_eq!(projected.decide, decide);
    }

    #[test]
    fn a_linear_label_with_a_picked_column_is_trigger_stage_talk() {
        let connector = workflow(Trigger::LinearLabel { label: "claude".into() }, None, "Analyzing");
        let graph = migrate(&connector);
        assert_eq!(
            graph,
            Graph {
                nodes: vec![
                    node("trigger", "Trigger", at(0, 0), NodeKind::Trigger { watch: Watch::LinearLabel { label: "claude".into() } }),
                    node("stage", "Picked", at(1, 0), NodeKind::StagePicked),
                    node("talk", "Talk", at(2, 0), NodeKind::Talk { prompt: String::new() }),
                ],
                edges: vec![edge("trigger", "out", "stage"), edge("stage", "out", "talk")],
            }
        );
        round_trip(&connector);
        let bare = workflow(Trigger::LinearLabel { label: "claude".into() }, None, "  ");
        assert!(migrate(&bare).nodes.iter().all(|node| !matches!(node.kind, NodeKind::StagePicked)));
        round_trip(&bare);
    }

    #[test]
    fn a_mention_and_a_called_workflow_are_a_trigger_then_a_talk() {
        let mention = workflow(Trigger::SlackMention { channel: "#support".into() }, None, "");
        assert_eq!(
            migrate(&mention),
            Graph {
                nodes: vec![
                    node("trigger", "Trigger", at(0, 0), NodeKind::Trigger { watch: Watch::SlackMention { channel: "#support".into() } }),
                    node("talk", "Talk", at(1, 0), NodeKind::Talk { prompt: String::new() }),
                ],
                edges: vec![edge("trigger", "out", "talk")],
            }
        );
        round_trip(&mention);
        let called = workflow(Trigger::Called, None, "");
        assert!(matches!(migrate(&called).trigger().kind, NodeKind::Trigger { watch: Watch::Called }));
        assert_eq!(migrate(&called).nodes.iter().find_map(|node| match &node.kind {
            NodeKind::Talk { prompt } => Some(prompt.as_str()),
            _ => None,
        }), Some(""));
        round_trip(&called);
    }

    #[test]
    fn an_ask_chains_one_classify_per_stopping_question_and_keeps_the_stop_spelling() {
        let ask = Ask {
            model: "google/gemini-3.5-flash".into(),
            instructions: "Classify.".into(),
            questions: vec![
                legacy_question("kind", "What does it ask for?", &["bug", "question"], &["question"]),
                legacy_question("agent", "Can a coding agent handle it?", &["yes", "no"], &["No", "maybe"]),
            ],
        };
        let connector = workflow(Trigger::SlackMention { channel: "#support".into() }, Some(ask), "");
        let questions = vec![
            Question { name: "kind".into(), ask: "What does it ask for?".into(), choices: vec![choice("bug", "bug", ""), choice("question", "question", "")] },
            Question { name: "agent".into(), ask: "Can a coding agent handle it?".into(), choices: vec![choice("yes", "yes", ""), choice("no", "no", "")] },
        ];
        let ask_node = |id: &str, name: &str, column: i32, branch: &str| {
            node(
                id,
                name,
                at(column, 0),
                NodeKind::Classify(Classify {
                    model: "google/gemini-3.5-flash".into(),
                    instructions: "Classify.".into(),
                    questions: questions.clone(),
                    branch_on: Some(branch.into()),
                    gate: Gate::Direct,
                    fallback: Fallback::None,
                }),
            )
        };
        assert_eq!(
            migrate(&connector),
            Graph {
                nodes: vec![
                    node("trigger", "Trigger", at(0, 0), NodeKind::Trigger { watch: Watch::SlackMention { channel: "#support".into() } }),
                    ask_node("ask", "Ask", 1, "kind"),
                    node("stop-0-0", "question", at(2, 1), NodeKind::Stop),
                    ask_node("ask-1", "Ask agent", 2, "agent"),
                    node("stop-1-0", "No", at(3, 1), NodeKind::Stop),
                    node("stop-1-1", "maybe", at(3, 2), NodeKind::Stop),
                    node("talk", "Talk", at(3, 0), NodeKind::Talk { prompt: String::new() }),
                ],
                edges: vec![
                    edge("trigger", "out", "ask"),
                    edge("ask", "question", "stop-0-0"),
                    edge("ask", "bug", "ask-1"),
                    edge("ask-1", "no", "stop-1-0"),
                    edge("ask-1", "yes", "talk"),
                ],
            }
        );
        round_trip(&connector);
    }

    #[test]
    fn a_message_workflow_is_a_decide_whose_categories_page_call_or_stop() {
        let decide = Decide {
            model: "typesafe-ai/jev".into(),
            instructions: "Categorize this Slack message.".into(),
            categories: vec![
                Category { name: "error".into(), description: "a bug".into(), workflow: Some("c2".into()), mention: None },
                Category { name: "doubt".into(), description: "a question".into(), workflow: Some("c6".into()), mention: None },
                Category { name: "escalation".into(), description: "page someone".into(), workflow: None, mention: Some("prod-engg".into()) },
                Category { name: "ignore".into(), description: "drop it".into(), workflow: None, mention: None },
            ],
            fallback: None,
        };
        let connector = workflow(Trigger::SlackMessage { channel: "#support".into(), decide: decide.clone() }, None, "Analyzing");
        let choices = vec![
            choice("error", "error", "a bug"),
            choice("doubt", "doubt", "a question"),
            choice("escalation", "escalation", "page someone"),
            choice("ignore", "ignore", "drop it"),
        ];
        assert_eq!(
            migrate(&connector),
            Graph {
                nodes: vec![
                    node("trigger", "Trigger", at(0, 0), NodeKind::Trigger { watch: Watch::SlackMessage { channel: "#support".into() } }),
                    node(
                        "decide",
                        "Decide",
                        at(1, 0),
                        NodeKind::Classify(Classify {
                            model: "typesafe-ai/jev".into(),
                            instructions: "Categorize this Slack message.".into(),
                            questions: vec![Question { name: "category".into(), ask: String::new(), choices }],
                            branch_on: Some("category".into()),
                            gate: Gate::Decide { unsure_below: connectors::UNSURE_BELOW, page_at: connectors::MIN_CONFIDENCE_TO_PAGE },
                            fallback: Fallback::None,
                        }),
                    ),
                    node("call-error", "error", at(2, 0), NodeKind::Call { workflow: "c2".into() }),
                    node("call-doubt", "doubt", at(2, 1), NodeKind::Call { workflow: "c6".into() }),
                    node("page-escalation", "prod-engg", at(2, 2), NodeKind::Page { group: "prod-engg".into() }),
                    node("stop-escalation", "escalation", at(3, 2), NodeKind::Stop),
                    node("stop-ignore", "ignore", at(2, 3), NodeKind::Stop),
                    node("nowhere", "nowhere", at(2, 4), NodeKind::Stop),
                ],
                edges: vec![
                    edge("trigger", "out", "decide"),
                    edge("decide", "error", "call-error"),
                    edge("decide", "doubt", "call-doubt"),
                    edge("decide", "escalation", "page-escalation"),
                    edge("page-escalation", "out", "stop-escalation"),
                    edge("decide", "ignore", "stop-ignore"),
                    edge("decide", "nowhere", "nowhere"),
                ],
            }
        );
        round_trip(&connector);
        let mut with_fallback = decide;
        with_fallback.fallback = Some("doubt".into());
        with_fallback.categories[0].mention = Some("oncall".into());
        let connector = workflow(Trigger::SlackMessage { channel: "#support".into(), decide: with_fallback }, None, "");
        round_trip(&connector);
        let graph = migrate(&connector);
        let NodeKind::Classify(classify) = &graph.node(&NodeId("decide".into())).unwrap().kind else { panic!("decide") };
        assert_eq!(classify.fallback, Fallback::Existing { port: PortId("doubt".into()) });
        assert_eq!(classify.gate, Gate::Decide { unsure_below: 0.6, page_at: 0.85 });
        let reloaded: Graph = serde_json::from_value(serde_json::to_value(&graph).unwrap()).unwrap();
        assert_eq!(reloaded, graph);
    }
}
