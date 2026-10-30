//! The agent CLIs pinned can host.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One row per agent CLI pinned can host. A table, so adding a provider is one entry, not a
/// branch per call site.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Claude,
    Codex,
    Cursor,
}
