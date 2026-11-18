//! The machine side of the feature map (U3), kept in the config folder next to `plans.json`.

use super::Machine;
use crate::team_tools::{FeatureMapRead, FeatureMapSet};

impl Machine {
    pub(super) fn feature_map_read(&self, _tab: Option<&str>, _task: Option<&str>, _call: FeatureMapRead) -> Result<String, String> {
        super::team::not_yet("feature_map_read")
    }

    pub(super) fn feature_map_set(&self, _tab: Option<&str>, _task: Option<&str>, _call: FeatureMapSet) -> Result<String, String> {
        super::team::not_yet("feature_map_set")
    }
}
