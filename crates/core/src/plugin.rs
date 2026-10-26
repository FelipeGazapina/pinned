use std::path::{Path, PathBuf};

/// Set in every session to the path of the tutorial helper, so a script can import it.
pub const HELPER_ENV: &str = "PINNED_TUTORIAL_HELPER";

const MANIFEST: &str = include_str!("../../../plugin/.claude-plugin/plugin.json");
const SKILL: &str = include_str!("../../../plugin/skills/tutorial-video/SKILL.md");
const HELPER: &str = include_str!("../../../plugin/skills/tutorial-video/tutorial.mjs");
const BRO: &str = include_str!("../../../plugin/skills/bro/SKILL.md");

/// Where `install` puts the helper, relative to the plugin directory.
fn helper_path(dir: &Path) -> PathBuf {
    dir.join("skills/tutorial-video/tutorial.mjs")
}

/// Writes the plugin that every session loads with `--plugin-dir`, and returns the helper's path.
pub fn install(dir: &Path) -> std::io::Result<PathBuf> {
    for (name, text) in [
        (".claude-plugin/plugin.json", MANIFEST),
        ("skills/tutorial-video/SKILL.md", SKILL),
        ("skills/tutorial-video/tutorial.mjs", HELPER),
        ("skills/bro/SKILL.md", BRO),
    ] {
        let file = dir.join(name);
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(file, text)?;
    }
    Ok(helper_path(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_lands_on_disk_with_a_helper_the_skill_points_at() {
        let dir = std::env::temp_dir().join(format!("pinned-plugin-test-{}", std::process::id()));
        let helper = install(&dir).unwrap();
        assert_eq!(helper, dir.join("skills/tutorial-video/tutorial.mjs"));
        assert!(std::fs::read_to_string(&helper).unwrap().contains("export async function startTutorial"));
        let skill = std::fs::read_to_string(dir.join("skills/tutorial-video/SKILL.md")).unwrap();
        assert!(skill.starts_with("---\nname: tutorial-video\n"));
        assert!(skill.contains(HELPER_ENV));
        assert!(std::fs::read_to_string(dir.join("skills/bro/SKILL.md")).unwrap().starts_with("---\nname: bro\n"));
        assert!(std::fs::read_to_string(dir.join(".claude-plugin/plugin.json")).unwrap().contains("\"name\": \"pinned\""));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
