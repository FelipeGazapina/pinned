//! The team's chat: a member talks to the team's orchestrator.

use super::Scope;
use crate::hub::Hub;
use serde_json::Value;
use std::sync::Arc;

/// Any member. Sends `text`, signed by the member, to the team's orchestrator session on the
/// team's account and answers its reply.
pub async fn chat(_hub: &Arc<Hub>, _scope: &Scope, _text: &str) -> Result<Value, String> {
    Err("not built yet".into())
}
