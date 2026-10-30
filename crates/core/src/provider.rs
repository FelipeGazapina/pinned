use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Claude,
    Codex,
    Cursor,
}
