//! Handoff chains (U12): `delegate_task` with `then` starts the first worker and hands its work to
//! the next profile when it ends. In a chain pinned sends a review's findings back on its own, up
//! to the rounds the user set (`reviewer::Reviewer::rounds`), with `Machine::send_back` and
//! `SendBackBy::Chain`, and the Captain gets one notice at the end or at the limit. Each member
//! keeps its own `budget_cents`; the root's card shows the chain's total, with no chain-wide cap.
//!
//! Owner: U12 (this file and `machine/chain.rs`).

use crate::delegate::Request;
use serde::{Deserialize, Serialize};

/// Where a task stands in its chain, kept on its record.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct ChainLink {
    /// The chain's first task.
    pub root: String,
    /// 0 for the root.
    pub index: u8,
    /// The profiles still to run after this task, in order.
    #[serde(default)]
    pub then: Vec<String>,
}

/// `delegate_task`'s `then`, which `parse_request` calls. U12 turns it on.
pub fn admit(request: &Request) -> Result<(), String> {
    match request.then.is_empty() {
        true => Ok(()),
        false => Err("then is not available in this build".into()),
    }
}

/// `delegate_task`'s schema for `then`, offered once U12 turns it on.
pub fn delegate_property() -> Option<(&'static str, serde_json::Value)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_chain_link_round_trips() {
        let link = ChainLink { root: "t1".into(), index: 1, then: vec!["tester".into()] };
        let value = serde_json::to_value(&link).unwrap();
        assert_eq!(value, json!({ "root": "t1", "index": 1, "then": ["tester"] }));
        assert_eq!(serde_json::from_value::<ChainLink>(value).unwrap(), link);
        assert_eq!(serde_json::from_value::<ChainLink>(json!({ "root": "t1", "index": 0 })).unwrap().then, Vec::<String>::new());
    }
}
