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
    /// The plans saved in `dir`, or none when the file is missing. A file that can't be read or
    /// parsed is renamed to `plans.json.bad` before anything is saved over it; when even that
    /// fails, the plans stay in memory only.
    pub fn load(dir: Option<&Path>) -> Self {
        let mut file = dir.map(|d| d.join(FILE));
        let mut by_session = HashMap::new();
        if let Some(path) = &file {
            let read = match std::fs::read(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
                Err(e) => Err(e.to_string()),
                // serde's message may quote the file, and a plan's text stays out of the log.
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("no JSON plans at line {} column {}", e.line(), e.column())),
            };
            match read {
                Ok(saved) => by_session = saved,
                Err(e) => {
                    let bad = path.with_extension("json.bad");
                    eprintln!("pinned: {FILE} is unreadable ({e}); kept aside as {FILE}.bad");
                    if let Err(e) = std::fs::rename(path, &bad) {
                        eprintln!("pinned: {FILE} could not be kept aside ({e}); plans will not save");
                        file = None;
                    }
                }
            }
        }
        PlanStore { by_session, file }
    }

    pub fn get(&self, session: &str) -> Option<&CaptainPlans> {
        self.by_session.get(session)
    }

    /// Runs `write` on the plans of `session` and saves them when it succeeds and changed them;
    /// the answer says whether it did. A failed write keeps the plans as they were, since the
    /// `plan` functions only commit what passes.
    pub fn write(&mut self, session: &str, write: impl FnOnce(&mut CaptainPlans) -> Result<String, String>) -> Result<(String, bool), String> {
        let before = self.by_session.get(session);
        let mut plans = before.cloned().unwrap_or_default();
        let answer = write(&mut plans)?;
        let changed = before != Some(&plans);
        if changed {
            self.by_session.insert(session.to_owned(), plans);
            self.save();
        }
        Ok((answer, changed))
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

    /// Writes a sibling file only this user may read, flushes it to disk, and renames it over the
    /// last save, so a failed or cut write keeps that one.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let saved = (|| {
            use std::io::Write;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = file.with_extension("json.tmp");
            let _ = std::fs::remove_file(&tmp);
            let written = crate::private::file().create_new(true).open(&tmp).and_then(|mut f| {
                f.write_all(&serde_json::to_vec(&self.by_session).expect("plans serialize"))?;
                f.sync_all()
            });
            if let Err(e) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            std::fs::rename(&tmp, file)?;
            // The rename itself lasts once the folder is flushed too.
            #[cfg(unix)]
            if let Some(dir) = file.parent() {
                std::fs::File::open(dir)?.sync_all()?;
            }
            Ok(())
        })();
        if let Err(e) = saved {
            eprintln!("pinned: plans did not save: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-plan-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ship(plans: &mut CaptainPlans) -> Result<String, String> {
        let call = crate::plan::SetCall { title: Some("Ship the form".into()), lists: vec![crate::plan::ListInput { id: None, name: "Build".into(), steps: vec![crate::plan::StepInput { id: None, name: "Write".into(), what: None }] }], ..Default::default() };
        crate::plan::set(plans, call, 1, &[], 1)
    }

    #[test]
    #[cfg(unix)]
    fn plans_are_saved_for_the_user_only() {
        let dir = temp("mode");
        let mut store = PlanStore::load(Some(&dir));
        store.write("cap-1", ship).unwrap();
        store.write("cap-2", ship).unwrap();
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.join(FILE)).unwrap().permissions()) & 0o777, 0o600);
        assert!(PlanStore::load(Some(&dir)).get("cap-2").is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_plans_file_that_does_not_parse_is_kept_aside_and_the_next_write_starts_a_new_one() {
        let dir = temp("corrupt");
        let corrupt = br#"{"cap-1":[{"id":"p1","title":"Ship the f"#;
        std::fs::write(dir.join(FILE), corrupt).unwrap();
        std::fs::write(dir.join(format!("{FILE}.bad")), b"an older one").unwrap();
        let mut store = PlanStore::load(Some(&dir));
        assert!(store.get("cap-1").is_none());
        assert_eq!(std::fs::read(dir.join(format!("{FILE}.bad"))).unwrap(), corrupt, "the file is kept aside as it was");
        assert!(!dir.join(FILE).exists());
        store.write("cap-2", ship).unwrap();
        assert!(PlanStore::load(Some(&dir)).get("cap-2").is_some());
        assert_eq!(std::fs::read(dir.join(format!("{FILE}.bad"))).unwrap(), corrupt, "the write left it alone");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
