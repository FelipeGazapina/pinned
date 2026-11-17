//! Scaffold: the store of every Captain session's plans. Not built yet.

use crate::plan::CaptainPlans;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Default)]
pub struct PlanStore {
    by_session: HashMap<String, CaptainPlans>,
}

impl PlanStore {
    pub fn load(_dir: Option<&Path>) -> Self {
        PlanStore::default()
    }

    pub fn get(&self, session: &str) -> Option<&CaptainPlans> {
        self.by_session.get(session)
    }

    pub fn write(&mut self, _session: &str, _write: impl FnOnce(&mut CaptainPlans) -> Result<String, String>) -> Result<String, String> {
        Err("not built yet".into())
    }

    pub fn drop_session(&mut self, _session: &str) {}

    pub fn keep_only(&mut self, _sessions: &HashSet<&str>) {}
}
