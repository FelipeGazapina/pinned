//! Project scripts: commands the header's scripts menu runs in a terminal, kept per project, as
//! T3 Code keeps its project actions, and offered for import from a checked-in `t3.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default = "play")]
    pub icon: String,
    /// Runs in a new worktree as it is made for a thread.
    #[serde(default)]
    pub run_on_worktree_create: bool,
    /// With `run_on_worktree_create`, claude starts only once the script exits.
    #[serde(default, rename = "async", skip_serializing_if = "Option::is_none")]
    pub run_async: Option<bool>,
    /// Runs in a thread's worktree when the thread settles.
    #[serde(default)]
    pub run_on_settle: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(default)]
    pub auto_open_preview: bool,
    /// A shortcut such as `mod+shift+r`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keybinding: Option<String>,
}

fn play() -> String {
    "play".into()
}

/// A project's own scripts and the ones its `t3.json` offers.
#[derive(Serialize, Debug, PartialEq, schemars::JsonSchema)]
pub struct ProjectScripts {
    pub scripts: Vec<Script>,
    pub file: Vec<Script>,
}

/// The scripts of `project` saved in `file`, and those of its `t3.json`.
pub fn of(file: Option<&Path>, project: &Path) -> ProjectScripts {
    let saved = file.map(load).unwrap_or_default();
    let checked_in = std::fs::read(project.join("t3.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let file_scripts = checked_in.and_then(|v| serde_json::from_value::<Vec<Script>>(v["scripts"].clone()).ok()).unwrap_or_default();
    ProjectScripts { scripts: saved.get(&project.display().to_string()).cloned().unwrap_or_default(), file: file_scripts }
}

/// Replaces `project`'s scripts in `file`. Refused for a script with no name or command.
pub fn save(file: &Path, project: &Path, scripts: Vec<Script>) -> Result<(), String> {
    if let Some(bad) = scripts.iter().find(|s| s.name.trim().is_empty() || s.command.trim().is_empty()) {
        return Err(format!("the script {:?} needs a name and a command", bad.name));
    }
    let mut saved = load(file);
    let key = project.display().to_string();
    if scripts.is_empty() {
        saved.remove(&key);
    } else {
        saved.insert(key, scripts);
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(file, serde_json::to_vec_pretty(&saved).unwrap()).map_err(|e| format!("could not save the scripts: {e}"))
}

fn load(file: &Path) -> BTreeMap<String, Vec<Script>> {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_save_per_project_and_a_t3_json_offers_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("scripts.json");
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("t3.json"), r#"{"scripts":[{"name":"Test","command":"npm test","icon":"test","runOnWorktreeCreate":true,"async":false}]}"#).unwrap();
        let script: Script = serde_json::from_str(r#"{"id":"dev","name":"Dev","command":"npm run dev","previewUrl":"http://localhost:5173"}"#).unwrap();
        assert_eq!((script.icon.as_str(), script.run_on_worktree_create), ("play", false));
        save(&file, &a, vec![script.clone()]).unwrap();
        let shown = of(Some(&file), &a);
        assert_eq!(shown.scripts, vec![script]);
        assert_eq!((shown.file[0].name.as_str(), shown.file[0].icon.as_str(), shown.file[0].run_async), ("Test", "test", Some(false)));
        assert_eq!(of(Some(&file), &b), ProjectScripts { scripts: vec![], file: vec![] });
        let blank = Script { name: " ".into(), ..shown.file[0].clone() };
        assert!(save(&file, &b, vec![blank]).unwrap_err().contains("needs a name"));
        save(&file, &a, vec![]).unwrap();
        assert!(of(Some(&file), &a).scripts.is_empty());
    }
}
