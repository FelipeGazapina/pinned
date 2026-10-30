//! The legacy fields an app that predates graphs reads, derived from a graph.

use super::Graph;
use crate::connectors::{Ask, Trigger};

/// What an older app sees of a graph workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct Projected {
    pub trigger: Trigger,
    pub ask: Option<Ask>,
}

/// Lossy once the graph is more than a trigger, at most one classify and the nodes `migrate`
/// makes; it then keeps the trigger and the first classify before the first talk.
pub fn project(graph: &Graph) -> Projected {
    let _ = graph;
    todo!()
}
