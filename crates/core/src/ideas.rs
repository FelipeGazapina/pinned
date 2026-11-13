//! Ideas: what might be done, as opposed to memory, which is what is true. Account-wide, with an optional
//! project. Nothing reads an idea into a prompt on its own. No callers yet (0.18.2, PR 1).

use serde::{Deserialize, Serialize};

use crate::schedule::Actor;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum IdeaStatus {
    #[default]
    Open,
    Done,
    Dropped,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Idea {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// A repo key such as `github.com/acme/site`; none for an idea that has no project yet.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub status: IdeaStatus,
    /// The task scheduled from this idea.
    #[serde(default)]
    pub task: Option<String>,
    pub created_by: Actor,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let idea: Idea = serde_json::from_str(
            r#"{"id":"i1","title":"an iPad app","created_by":{"kind":"user"},"created_at":1,"updated_at":2}"#,
        )
        .unwrap();
        assert_eq!(idea.status, IdeaStatus::Open);
        assert_eq!(idea.project, None);
        assert_eq!(idea.task, None);
        assert_eq!(idea.text, "");
        let back: Idea = serde_json::from_str(&serde_json::to_string(&idea).unwrap()).unwrap();
        assert_eq!(back, idea);
    }

    #[test]
    fn status_wire_names() {
        assert_eq!(serde_json::to_string(&IdeaStatus::Dropped).unwrap(), r#""dropped""#);
    }
}
