//! A workflow drawn as a graph, as n8n draws one: nodes, and edges from a node's output port to
//! another node. One run walks it depth first, the first edge of a port to its end before the next,
//! and the whole run talks to one Claude session.
//!
//! `migrate` builds the graph a fixed-pipeline connector already is. `project` writes the legacy
//! fields back for apps that predate graphs. `runner` walks a graph without doing any I/O.

use crate::vercel::Chosen;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod migrate;
pub mod project;
pub mod runner;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, JsonSchema)]
#[serde(transparent)]
pub struct NodeId(pub String);

/// A choice's stable id, or one of the fixed ports below. Reordering or renaming choices keeps edges.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, JsonSchema)]
#[serde(transparent)]
pub struct PortId(pub String);

impl PortId {
    /// The one output of every node that does not branch.
    pub const OUT: &'static str = "out";
    /// A classify node's port for an answer the model is unsure about, when `Fallback::Extra`.
    pub const FALLBACK: &'static str = "fallback";
    /// A `Gate::Decide` node's port when no choice may take the message.
    pub const NOWHERE: &'static str = "nowhere";
}

/// Invariants, checked by `Graph::validate` at save:
/// - exactly one node is a trigger, and no edge enters it;
/// - every edge's `from` and `to` exist, and its `port` is one of `ports(from)`;
/// - node ids are unique; names are labels, not keys;
/// - `call` targets form no cycle across the connectors that hold graphs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Graph {
    pub nodes: Vec<Node>,
    /// Order matters: a port's edges run in this order, each branch to its end before the next.
    pub edges: Vec<Edge>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    /// Canvas pixels of the node's center. The editor owns them.
    pub position: [i32; 2],
    #[serde(flatten)]
    pub kind: NodeKind,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Edge {
    pub from: NodeId,
    pub port: PortId,
    pub to: NodeId,
}

/// Everything a node can be. Coding and review moves, the Slack reply, filing a Linear issue and the
/// proof video are not nodes: the session does them through `pinned` callbacks, and the connector's
/// `columns`, `fix` and `proof` still describe them, so nothing else writes the same state.
///
/// Slack follow-ups in the thread are not a node either. While the run's session is open, new
/// thread messages are typed into it, as today.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum NodeKind {
    /// What starts a run. The walk begins at its outgoing edges.
    Trigger { watch: Watch },
    /// One Vercel AI Gateway call that answers every question, and branches on one of them.
    Classify(Classify),
    /// Tells the run's one Claude session something. The first `talk` of a run opens the session
    /// with the session owner's settings; every later one, on any branch, types into it.
    Talk { prompt: String },
    /// Pings a Slack user group, by handle or id, in the run's thread.
    Page { group: String },
    /// Moves the Linear issue to the connector's `picked` column, as opening a session does today.
    StagePicked,
    /// Walks another connector's graph inside this run, with this run's session.
    Call { workflow: String },
    /// Ends this branch on purpose. A port with no edge also ends its branch.
    Stop,
}

/// The trigger kinds, without the routing a fixed-pipeline `SlackMessage` carried in `Decide`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Watch {
    LinearLabel { label: String },
    SlackMention { channel: String },
    SlackMessage { channel: String },
    /// Runs only when another graph `call`s it.
    Called,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Classify {
    /// A gateway model id, as `typesafe-ai/jev`.
    pub model: String,
    pub instructions: String,
    pub questions: Vec<Question>,
    /// The name of the question whose choices are the ports. None leaves one `out` port.
    #[serde(default)]
    pub branch_on: Option<String>,
    #[serde(default)]
    pub gate: Gate,
    #[serde(default)]
    pub fallback: Fallback,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Question {
    /// The key the answer goes under in the run's context.
    pub name: String,
    pub ask: String,
    /// The answers the model must pick from; empty takes free text and cannot be branched on.
    #[serde(default)]
    pub choices: Vec<Choice>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Choice {
    pub id: PortId,
    pub label: String,
    /// What the model reads to tell this choice from the others.
    #[serde(default)]
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Gate {
    /// The port is the model's choice.
    #[default]
    Direct,
    /// Today's `Decide::pick`: below `unsure_below` the fallback takes it, and below `page_at` a
    /// choice whose branch pages a group gives way to the likeliest quiet one, or to `nowhere`.
    #[serde(rename_all = "camelCase")]
    Decide { unsure_below: f64, page_at: f64 },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Fallback {
    /// An unsure answer keeps the model's guess.
    #[default]
    None,
    /// An unsure answer takes this existing choice's port, as a `Decide` fallback category does.
    Existing { port: PortId },
    /// An unsure answer takes an extra `fallback` port.
    Extra,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub id: PortId,
    pub label: String,
}

fn out_port() -> Port {
    Port { id: PortId(PortId::OUT.into()), label: "out".into() }
}

fn classify_ports(classify: &Classify) -> Vec<Port> {
    let Some(name) = &classify.branch_on else {
        return vec![out_port()];
    };
    let mut ports = Vec::new();
    if let Some(question) = classify.questions.iter().find(|question| &question.name == name) {
        for choice in &question.choices {
            ports.push(Port { id: choice.id.clone(), label: choice.label.clone() });
        }
    }
    if matches!(classify.fallback, Fallback::Extra) {
        ports.push(Port { id: PortId(PortId::FALLBACK.into()), label: "fallback".into() });
    }
    if matches!(classify.gate, Gate::Decide { .. }) {
        ports.push(Port { id: PortId(PortId::NOWHERE.into()), label: "nowhere".into() });
    }
    ports
}

impl NodeKind {
    /// The node's output ports, in drawing order. Derived, never stored.
    pub fn ports(&self) -> Vec<Port> {
        match self {
            NodeKind::Trigger { .. } | NodeKind::Talk { .. } | NodeKind::Page { .. } | NodeKind::StagePicked | NodeKind::Call { .. } => {
                vec![out_port()]
            }
            NodeKind::Stop => Vec::new(),
            NodeKind::Classify(classify) => classify_ports(classify),
        }
    }
}

impl Graph {
    /// Checks the invariants on `Graph`. `called` says whether a connector id holds a graph whose
    /// trigger is `Watch::Called`, for `call` targets. Errors name the node, for the editor.
    ///
    /// A cycle that runs through another connector's graph is rejected at save, which can see every
    /// connector. This callback only answers whether one id is a legal call target.
    pub fn validate(&self, called: &dyn Fn(&str) -> bool) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        let mut trigger = None;
        for node in &self.nodes {
            if !ids.insert(node.id.0.clone()) {
                return Err(format!("\"{}\" reuses the id \"{}\"", node.name, node.id.0));
            }
            if matches!(node.kind, NodeKind::Trigger { .. }) && trigger.replace(node).is_some() {
                return Err(format!("\"{}\" is a second trigger", node.name));
            }
        }
        let Some(trigger) = trigger else {
            return Err("a workflow needs a trigger".into());
        };
        for edge in &self.edges {
            if edge.to == trigger.id {
                return Err(format!("an edge enters the trigger \"{}\"", trigger.name));
            }
            let Some(from) = self.node(&edge.from) else {
                return Err(format!("an edge leaves \"{}\", which is not a node", edge.from.0));
            };
            if self.node(&edge.to).is_none() {
                return Err(format!("\"{}\" connects to \"{}\", which is not a node", from.name, edge.to.0));
            }
            if !from.kind.ports().iter().any(|port| port.id == edge.port) {
                return Err(format!("\"{}\" has no port \"{}\"", from.name, edge.port.0));
            }
        }
        for node in &self.nodes {
            if let NodeKind::Call { workflow } = &node.kind {
                if !called(workflow) {
                    return Err(format!("\"{name}\" calls \"{workflow}\", which is not a called workflow", name = node.name));
                }
            }
        }
        Ok(())
    }

    pub fn trigger(&self) -> &Node {
        self.nodes.iter().find(|node| matches!(node.kind, NodeKind::Trigger { .. })).expect("a graph has one trigger")
    }

    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    /// The targets of `port` on `from`, in edge order.
    pub fn next(&self, from: &NodeId, port: &PortId) -> Vec<&NodeId> {
        self.edges.iter().filter(|e| &e.from == from && &e.port == port).map(|e| &e.to).collect()
    }
}

/// What a classify node got back, before the gate picks a port.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Answers {
    /// (question name, answer), in question order.
    pub answers: Vec<(String, String)>,
    /// The decision model's pick on `branch_on`, with probabilities, when the model gave them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<ChosenOn>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChosenOn {
    pub index: usize,
    pub probabilities: Vec<f64>,
}

impl From<&Chosen> for ChosenOn {
    fn from(chosen: &Chosen) -> Self {
        ChosenOn { index: chosen.index, probabilities: chosen.probabilities.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, name: &str, kind: NodeKind) -> Node {
        Node { id: NodeId(id.into()), name: name.into(), position: [0, 0], kind }
    }

    fn edge(from: &str, port: &str, to: &str) -> Edge {
        Edge { from: NodeId(from.into()), port: PortId(port.into()), to: NodeId(to.into()) }
    }

    fn classify(branch_on: Option<&str>, choices: &[(&str, &str)], gate: Gate, fallback: Fallback) -> NodeKind {
        NodeKind::Classify(Classify {
            model: "m".into(),
            instructions: String::new(),
            questions: vec![Question {
                name: "kind".into(),
                ask: String::new(),
                choices: choices.iter().map(|(id, label)| Choice { id: PortId((*id).into()), label: (*label).into(), description: String::new() }).collect(),
            }],
            branch_on: branch_on.map(str::to_owned),
            gate,
            fallback,
        })
    }

    #[test]
    fn ports_follow_the_node() {
        assert_eq!(NodeKind::Talk { prompt: String::new() }.ports(), vec![Port { id: PortId("out".into()), label: "out".into() }]);
        assert_eq!(NodeKind::StagePicked.ports(), vec![Port { id: PortId("out".into()), label: "out".into() }]);
        assert_eq!(NodeKind::Stop.ports(), vec![]);
        assert_eq!(
            classify(None, &[("bug", "Bug")], Gate::Decide { unsure_below: 0.6, page_at: 0.85 }, Fallback::Extra).ports(),
            vec![Port { id: PortId("out".into()), label: "out".into() }],
            "a classify with no branch question has one out port"
        );
        assert_eq!(
            classify(Some("kind"), &[("bug", "Bug"), ("ask", "Ask")], Gate::Direct, Fallback::None).ports(),
            vec![Port { id: PortId("bug".into()), label: "Bug".into() }, Port { id: PortId("ask".into()), label: "Ask".into() }]
        );
        assert_eq!(
            classify(Some("kind"), &[("bug", "Bug")], Gate::Decide { unsure_below: 0.6, page_at: 0.85 }, Fallback::Extra).ports(),
            vec![
                Port { id: PortId("bug".into()), label: "Bug".into() },
                Port { id: PortId("fallback".into()), label: "fallback".into() },
                Port { id: PortId("nowhere".into()), label: "nowhere".into() },
            ]
        );
    }

    #[test]
    fn validate_names_the_node() {
        let talk = node("talk", "Talk", NodeKind::Talk { prompt: String::new() });
        let trigger = node("trigger", "Start", NodeKind::Trigger { watch: Watch::Called });
        let ok = Graph { nodes: vec![trigger.clone(), talk.clone()], edges: vec![edge("trigger", "out", "talk")] };
        assert_eq!(ok.validate(&|_| false), Ok(()));

        assert_eq!(Graph { nodes: vec![], edges: vec![] }.validate(&|_| true), Err("a workflow needs a trigger".into()));
        let second = node("again", "Extra", NodeKind::Trigger { watch: Watch::Called });
        assert_eq!(
            Graph { nodes: vec![trigger.clone(), second], edges: vec![] }.validate(&|_| true),
            Err("\"Extra\" is a second trigger".into())
        );
        assert_eq!(
            Graph { nodes: vec![trigger.clone(), talk.clone()], edges: vec![edge("talk", "out", "trigger")] }.validate(&|_| true),
            Err("an edge enters the trigger \"Start\"".into())
        );
        assert_eq!(
            Graph { nodes: vec![trigger.clone(), talk.clone()], edges: vec![edge("missing", "out", "talk")] }.validate(&|_| true),
            Err("an edge leaves \"missing\", which is not a node".into())
        );
        assert_eq!(
            Graph { nodes: vec![trigger.clone()], edges: vec![edge("trigger", "out", "missing")] }.validate(&|_| true),
            Err("\"Start\" connects to \"missing\", which is not a node".into())
        );
        assert_eq!(
            Graph { nodes: vec![trigger.clone(), talk.clone()], edges: vec![edge("talk", "nope", "talk")] }.validate(&|_| true),
            Err("\"Talk\" has no port \"nope\"".into())
        );
        let reused = node("trigger", "Talk", NodeKind::Talk { prompt: String::new() });
        assert_eq!(
            Graph { nodes: vec![trigger.clone(), reused], edges: vec![] }.validate(&|_| true),
            Err("\"Talk\" reuses the id \"trigger\"".into())
        );
        let call = node("route", "Route", NodeKind::Call { workflow: "gone".into() });
        assert_eq!(
            Graph { nodes: vec![trigger, call], edges: vec![edge("trigger", "out", "route")] }.validate(&|id| id == "kept"),
            Err("\"Route\" calls \"gone\", which is not a called workflow".into())
        );
    }
}
