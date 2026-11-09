//! How this machine's orchestrator works for the user: which cost style it routes by,
//! and how many workers it may run at once. One file in the config dir, so every project on
//! this machine shares it; per-project habits stay in that project's memory instead.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The default worker cap: high enough to never trip on ordinary fan-out, low enough to stop a
/// runaway orchestrator before it melts the machine.
pub const DEFAULT_MAX_WORKERS: u32 = 100;
const MAX_WORKERS_LIMIT: u32 = 10_000;
pub(crate) const FILE: &str = "orchestration.json";

/// What the user optimizes for when the orchestrator picks providers and models.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// The cheapest model that can do the job, one worker, short budgets.
    Economical,
    /// Mid models first, a second opinion only when it matters.
    #[default]
    Balanced,
    /// The strongest models in parallel, exploring every angle at once.
    Quality,
}

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Style::Economical => "economical",
            Style::Balanced => "balanced",
            Style::Quality => "quality",
        }
    }

    pub fn parse(name: &str) -> Option<Style> {
        match name.trim().to_lowercase().as_str() {
            "economical" => Some(Style::Economical),
            "balanced" => Some(Style::Balanced),
            "quality" => Some(Style::Quality),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Orchestration {
    #[serde(default)]
    pub style: Style,
    #[serde(default = "default_max_workers")]
    pub max_workers: u32,
}

fn default_max_workers() -> u32 {
    DEFAULT_MAX_WORKERS
}

impl Default for Orchestration {
    fn default() -> Self {
        Orchestration { style: Style::Balanced, max_workers: DEFAULT_MAX_WORKERS }
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

    /// The routing rules the orchestrator's prompt carries for the current style.
    pub fn hint(&self) -> String {
        let workers = format!(
            "At most {} workers run at once on this machine; if that blocks you, tell the user to raise it under Settings rather than working around it. How many parallel workers a project wants lives in that project's memory: look it up with memory_search before fanning out, and save what you learn with memory_save.",
            self.max_workers
        );
        match self.style {
            Style::Economical => format!(
                "Cost style is ECONOMICAL: pick the cheapest signed-in model that can do the job (cheap models first for sweeps, drafts and translations), one worker, short budgets. A second opinion or a stronger model only when the cheap one fails or the task needs architecture-level judgment. {workers}"
            ),
            Style::Balanced => format!(
                "Cost style is BALANCED: pick mid models first and the strongest only for architecture-level judgment or hairy bugs. A second parallel worker only when two angles genuinely disagree. Skip accounts a usage limit stopped. {workers}"
            ),
            Style::Quality => format!(
                "Cost style is QUALITY: pick the strongest signed-in models and run 2-3 workers in parallel on different providers or models with the same brief, then compare their reports and continue with the best, telling the user what each found. Skip accounts a usage limit stopped. {workers}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_balanced_with_a_hundred_workers() {
        let dir = std::env::temp_dir().join(format!("pinned-orchestration-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let loaded = Orchestration::load(Some(&dir));
        assert_eq!(loaded, Orchestration { style: Style::Balanced, max_workers: 100 });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trip_and_clamp() {
        let dir = std::env::temp_dir().join(format!("pinned-orchestration-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Orchestration { style: Style::Quality, max_workers: 0 }.save(Some(&dir)).unwrap();
        assert_eq!(Orchestration::load(Some(&dir)).max_workers, 1);
        Orchestration { style: Style::Economical, max_workers: 1_000_000 }.save(Some(&dir)).unwrap();
        assert_eq!(Orchestration::load(Some(&dir)).max_workers, MAX_WORKERS_LIMIT);
        Orchestration { style: Style::Economical, max_workers: 3 }.save(Some(&dir)).unwrap();
        assert_eq!(Orchestration::load(Some(&dir)), Orchestration { style: Style::Economical, max_workers: 3 });
        assert!(Orchestration::load(Some(&dir)).hint().contains("ECONOMICAL"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn style_parses() {
        assert_eq!(Style::parse("Quality"), Some(Style::Quality));
        assert_eq!(Style::parse(" economical "), Some(Style::Economical));
        assert_eq!(Style::parse("turbo"), None);
    }
}
