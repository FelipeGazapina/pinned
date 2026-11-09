//! How many workers this machine runs at once. One file in the config dir, so every project on
//! this machine shares it. How the agents pick models and how many workers a project wants live in
//! the account's taste (`taste.rs`) instead.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The default worker cap: high enough to never trip on ordinary fan-out, low enough to stop a
/// runaway orchestrator before it melts the machine.
pub const DEFAULT_MAX_WORKERS: u32 = 100;
const MAX_WORKERS_LIMIT: u32 = 10_000;
pub(crate) const FILE: &str = "orchestration.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Orchestration {
    #[serde(default = "default_max_workers")]
    pub max_workers: u32,
}

fn default_max_workers() -> u32 {
    DEFAULT_MAX_WORKERS
}

impl Default for Orchestration {
    fn default() -> Self {
        Orchestration { max_workers: DEFAULT_MAX_WORKERS }
    }
}

impl Orchestration {
    pub fn load(dir: Option<&Path>) -> Self {
        let mut current = Orchestration::default();
        if let Some(file) = dir.map(|d| d.join(FILE)) {
            if let Ok(bytes) = std::fs::read(&file) {
                if let Ok(saved) = serde_json::from_slice::<Orchestration>(&bytes) {
                    current = saved;
                }
            }
        }
        current.clamp();
        current
    }

    pub fn save(&self, dir: Option<&Path>) -> Result<(), String> {
        let Some(dir) = dir else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(FILE), serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    pub(crate) fn clamp(&mut self) {
        if self.max_workers < 1 {
            self.max_workers = 1;
        }
        if self.max_workers > MAX_WORKERS_LIMIT {
            self.max_workers = MAX_WORKERS_LIMIT;
        }
    }

    /// The Capitão's sentence about this machine's cap.
    pub fn hint(&self) -> String {
        format!("At most {} workers run at once on this machine; if that blocks you, tell the user to raise it under Settings rather than working around it.", self.max_workers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_a_hundred_workers() {
        let dir = std::env::temp_dir().join(format!("pinned-orchestration-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Orchestration::load(Some(&dir)), Orchestration { max_workers: 100 });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trip_and_clamp() {
        let dir = std::env::temp_dir().join(format!("pinned-orchestration-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Orchestration { max_workers: 0 }.save(Some(&dir)).unwrap();
        assert_eq!(Orchestration::load(Some(&dir)).max_workers, 1);
        Orchestration { max_workers: 1_000_000 }.save(Some(&dir)).unwrap();
        assert_eq!(Orchestration::load(Some(&dir)).max_workers, MAX_WORKERS_LIMIT);
        std::fs::write(dir.join(FILE), r#"{"style":"quality","max_workers":3}"#).unwrap();
        assert_eq!(Orchestration::load(Some(&dir)), Orchestration { max_workers: 3 }, "a file from before tastes still reads");
        assert_eq!(
            Orchestration::load(Some(&dir)).hint(),
            "At most 3 workers run at once on this machine; if that blocks you, tell the user to raise it under Settings rather than working around it."
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
