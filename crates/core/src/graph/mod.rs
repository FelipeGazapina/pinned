//! A workflow drawn as a graph, as n8n draws one: nodes, and edges from a node's output port to
//! another node. One run walks it depth first, the first edge of a port to its end before the next,
//! and the whole run talks to one Claude session.
//!
//! `migrate` builds the graph a fixed-pipeline connector already is. `project` writes the legacy
//! fields back for apps that predate graphs. `runner` walks a graph without doing any I/O.

use crate::vercel::Chosen;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod migrate;
pub mod project;
pub mod runner;

#[derive(
    Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, JsonSchema,
)]
#[serde(transparent)]
pub struct NodeId(pub String);

/// A choice's stable id, or one of the fixed ports below. Reordering or renaming choices keeps edges.
#[derive(
    Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, JsonSchema,
)]
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
    LinearLabel {
        label: String,
    },
    SlackMention {
        channel: String,
    },
    SlackMessage {
        channel: String,
    },
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

impl NodeKind {
    /// The node's output ports, in drawing order. Derived, never stored.
    pub fn ports(&self) -> Vec<Port> {
        match self {
            NodeKind::Stop => Vec::new(),
            NodeKind::Trigger { .. }
            | NodeKind::Talk { .. }
            | NodeKind::Page { .. }
            | NodeKind::StagePicked
            | NodeKind::Call { .. } => {
                vec![fixed_port(PortId::OUT)]
            }
            NodeKind::Classify(classify) => classify_ports(classify),
        }
    }
}

fn fixed_port(id: &str) -> Port {
    Port {
        id: PortId(id.to_owned()),
        label: id.to_owned(),
    }
}

fn classify_ports(classify: &Classify) -> Vec<Port> {
    let Some(name) = &classify.branch_on else {
        return vec![fixed_port(PortId::OUT)];
    };
    let mut ports = Vec::new();
    if let Some(question) = classify
        .questions
        .iter()
        .find(|question| &question.name == name)
    {
        for choice in &question.choices {
            ports.push(Port {
                id: choice.id.clone(),
                label: choice.label.clone(),
            });
        }
    }
    if matches!(classify.fallback, Fallback::Extra) {
        ports.push(fixed_port(PortId::FALLBACK));
    }
    if matches!(classify.gate, Gate::Decide { .. }) {
        ports.push(fixed_port(PortId::NOWHERE));
    }
    ports
}

impl Graph {
    /// Checks the invariants on `Graph`. `called` says whether a connector id holds a graph whose
    /// trigger is `Watch::Called`, for `call` targets. Errors name the node, for the editor.
    pub fn validate(&self, called: &dyn Fn(&str) -> bool) -> Result<(), String> {
        let _ = called;
        todo!()
    }

    pub fn trigger(&self) -> &Node {
        self.nodes
            .iter()
            .find(|node| matches!(node.kind, NodeKind::Trigger { .. }))
            .expect("a graph has one trigger")
    }

    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    /// The targets of `port` on `from`, in edge order.
    pub fn next(&self, from: &NodeId, port: &PortId) -> Vec<&NodeId> {
        self.edges
            .iter()
            .filter(|e| &e.from == from && &e.port == port)
            .map(|e| &e.to)
            .collect()
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
        ChosenOn {
            index: chosen.index,
            probabilities: chosen.probabilities.clone(),
        }
    }
}
