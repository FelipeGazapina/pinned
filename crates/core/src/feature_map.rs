//! The feature map (U3): which files make up each feature of a project, kept on the Captain's
//! machine next to `config/plans.json` and read and written with `feature_map_read` and
//! `feature_map_set`.
//!
//! Owner: U3 (this file and `machine/feature_map.rs`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Its file in the machine's config folder, beside `plans.json`.
pub const FILE: &str = "feature-map.json";

/// Every project's features, by the project's folder.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct FeatureMaps {
    pub projects: BTreeMap<String, FeatureMap>,
}

/// One project's features, by name.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct FeatureMap {
    pub features: BTreeMap<String, Feature>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct Feature {
    /// Paths relative to the project.
    pub files: Vec<String>,
    /// What the feature does, in one line.
    pub about: String,
    /// Unix milliseconds of the last change.
    pub at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_feature_map_round_trips_and_reads_empty_from_nothing() {
        assert_eq!(serde_json::from_value::<FeatureMaps>(json!({})).unwrap(), FeatureMaps::default());
        let map = FeatureMap { features: BTreeMap::from([("login".into(), Feature { files: vec!["src/login.rs".into()], about: "Signs a user in.".into(), at: 5 })]) };
        let value = serde_json::to_value(&map).unwrap();
        assert_eq!(serde_json::from_value::<FeatureMap>(value).unwrap(), map);
    }
}
