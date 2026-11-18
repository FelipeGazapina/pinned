//! The feature map (U3): which files make up each feature of a project, kept on the Captain's
//! machine next to `config/plans.json` and read and written with `feature_map_read` and
//! `feature_map_set`.
//!
//! Owner: U3 (this file and `machine/feature_map.rs`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

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

/// The maps saved in the config folder `dir`: none when the file is missing. A file that cannot be
/// read or parsed is renamed to `feature-map.json.corrupt-<unix millis>` first, so nothing is ever
/// saved over it, and the map starts empty. When even the rename fails, the answer is an error.
pub fn load(dir: &Path) -> Result<FeatureMaps, String> {
    let path = dir.join(FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FeatureMaps::default()),
        Err(e) => return keep_aside(&path, &e.to_string()),
    };
    // serde's message may quote the file, and a feature's text stays out of the log.
    match serde_json::from_slice(&bytes) {
        Ok(maps) => Ok(maps),
        Err(e) => keep_aside(&path, &format!("no JSON feature map at line {} column {}", e.line(), e.column())),
    }
}

fn keep_aside(path: &Path, why: &str) -> Result<FeatureMaps, String> {
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let name = format!("{FILE}.corrupt-{millis}");
    eprintln!("pinned: {FILE} is unreadable ({why}); kept aside as {name}");
    match std::fs::rename(path, path.with_file_name(&name)) {
        Ok(()) => Ok(FeatureMaps::default()),
        Err(e) => {
            eprintln!("pinned: {FILE} could not be kept aside ({e}); the feature map will not change");
            Err(format!("{FILE} is unreadable and could not be kept aside; nothing changed"))
        }
    }
}

/// Writes `maps` to `dir`: a sibling file only this user may read, flushed, then renamed over the
/// last save, so a failed or cut write keeps that one.
pub fn save(dir: &Path, maps: &FeatureMaps) -> Result<(), String> {
    use std::io::Write;
    let file = dir.join(FILE);
    let saved = (|| {
        std::fs::create_dir_all(dir)?;
        let tmp = file.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        let written = crate::private::file().create_new(true).open(&tmp).and_then(|mut f| {
            f.write_all(&serde_json::to_vec(maps).expect("a feature map serializes"))?;
            f.sync_all()
        });
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, &file)?;
        // The rename itself lasts once the folder is flushed too.
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    saved.map_err(|e| format!("the feature map did not save: {e}"))
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

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-feature-map-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn login() -> FeatureMaps {
        let map = FeatureMap { features: BTreeMap::from([("login".into(), Feature { files: vec!["src/login.rs".into()], about: "Signs a user in.".into(), at: 5 })]) };
        FeatureMaps { projects: BTreeMap::from([("/work/app".into(), map)]) }
    }

    fn corrupt_files(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("feature-map.json.corrupt-")).collect();
        found.sort();
        found
    }

    #[test]
    fn a_saved_map_loads_again_and_a_missing_file_loads_empty() {
        let dir = temp("roundtrip");
        assert_eq!(load(&dir), Ok(FeatureMaps::default()));
        save(&dir, &login()).unwrap();
        assert_eq!(load(&dir), Ok(login()));
        assert!(corrupt_files(&dir).is_empty());
        assert!(!dir.join("feature-map.json.tmp").exists(), "the temp file was renamed away");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_truncated_file_is_kept_aside_and_never_saved_over() {
        let dir = temp("truncated");
        save(&dir, &login()).unwrap();
        let whole = std::fs::read(dir.join(FILE)).unwrap();
        let cut = &whole[..whole.len() / 2];
        std::fs::write(dir.join(FILE), cut).unwrap();
        assert_eq!(load(&dir), Ok(FeatureMaps::default()), "it starts empty");
        let kept = corrupt_files(&dir);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), cut, "the old bytes survive");
        assert!(!dir.join(FILE).exists());
        save(&dir, &FeatureMaps::default()).unwrap();
        assert_eq!(std::fs::read(&kept[0]).unwrap(), cut, "a set after it leaves them alone");
        let _ = std::fs::remove_dir_all(dir);
    }
}
