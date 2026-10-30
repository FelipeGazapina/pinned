//! The graph a fixed-pipeline connector already is.

use super::Graph;
use crate::connectors::Connector;

/// Builds the graph `connector`'s legacy fields describe, with positions laid out left to right.
/// It adds no branch the pipeline lacked: `ask` choices in `stop_on` lead to `stop`, the others
/// share the rest of the chain; a slack-message `decide` becomes a `Gate::Decide` classify whose
/// categories route to `call`, `page` or `stop`. Deterministic, so the same connector always gives
/// an equal graph: that is how a stored graph is told apart from an edited one.
pub fn migrate(connector: &Connector) -> Graph {
    let _ = connector;
    todo!()
}
