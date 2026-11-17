//! Every Captain session's plans, by its Claude session id, kept in `plans.json` in the data dir.

use crate::plan::CaptainPlans;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const FILE: &str = "plans.json";

#[derive(Default)]
pub struct PlanStore {
    by_session: HashMap<String, CaptainPlans>,
    /// None keeps the plans in memory only.
    file: Option<PathBuf>,
}

impl PlanStore {
    /// The plans saved in `dir`, or none when the file is missing or unreadable.
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join(FILE));
        let by_session = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        PlanStore { by_session, file }
    }

    pub fn get(&self, session: &str) -> Option<&CaptainPlans> {
        self.by_session.get(session)
    }

    /// Runs `write` on the plans of `session` and saves them when it succeeds. A failed write keeps
    /// the plans as they were, since the `plan` functions only commit what passes.
    pub fn write(&mut self, session: &str, write: impl FnOnce(&mut CaptainPlans) -> Result<String, String>) -> Result<String, String> {
        let mut plans = self.by_session.get(session).cloned().unwrap_or_default();
        let answer = write(&mut plans)?;
        self.by_session.insert(session.to_owned(), plans);
        self.save();
        Ok(answer)
    }

    /// Gives `to` the plans of `from` unless it has its own, and drops them from `from` unless
    /// `keep` asks for both. True when any session's plans changed.
    pub fn carry(&mut self, from: &str, to: &str, keep: bool) -> bool {
        let plans = if keep { self.by_session.get(from).cloned() } else { self.by_session.remove(from) };
        let Some(plans) = plans else { return false };
        let taken = !self.by_session.contains_key(to);
        if taken {
            self.by_session.insert(to.to_owned(), plans);
        }
        if taken || !keep {
            self.save();
        }
        taken || !keep
    }

    pub fn drop_session(&mut self, session: &str) {
        if self.by_session.remove(session).is_some() {
            self.save();
        }
    }

    /// Drops the plans of every session not in `sessions`.
    pub fn keep_only(&mut self, sessions: &HashSet<&str>) {
        let before = self.by_session.len();
        self.by_session.retain(|session, _| sessions.contains(session.as_str()));
        if self.by_session.len() != before {
            self.save();
        }
    }

    /// Writes a sibling file first and renames it over the last save, so a failed write keeps it.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let saved = (|| {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = file.with_extension("json.tmp");
            if let Err(e) = std::fs::write(&tmp, serde_json::to_vec(&self.by_session).expect("plans serialize")) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            std::fs::rename(&tmp, file)
        })();
        if let Err(e) = saved {
            eprintln!("pinned: plans did not save: {e}");
        }
    }
}
