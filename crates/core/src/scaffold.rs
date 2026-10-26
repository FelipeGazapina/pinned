//! T3 Code's New project: a named folder under the machine's projects root, a git repository
//! with a README and an icon, and a first commit; then, if asked, a private GitHub repository.

use crate::workspace::{run, Failure};
use serde::Serialize;
use std::path::{Path, PathBuf};

const ATTEMPTS: usize = 100;
const ICON_BACKGROUNDS: [&str; 15] = [
    "#dc2626", "#ea580c", "#d97706", "#16a34a", "#059669", "#0d9488", "#0891b2", "#0284c7", "#2563eb", "#4f46e5", "#7c3aed", "#9333ea", "#c026d3", "#db2777", "#e11d48",
];

#[derive(Serialize, Debug, PartialEq)]
pub struct Created {
    pub path: String,
    /// Why git made no first commit, when it couldn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_error: Option<String>,
}

/// The folder name for `name`: lowercase ASCII words joined by dashes, as T3 Code's newProjectFolderName.
pub fn folder_name(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(fold_accent) {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.chars().take(64).collect::<String>().trim_end_matches('-').to_owned();
    if slug.is_empty() {
        return "project".into();
    }
    let reserved = matches!(slug.as_str(), "con" | "prn" | "aux" | "nul") || (slug.len() == 4 && (slug.starts_with("com") || slug.starts_with("lpt")) && slug.as_bytes()[3].is_ascii_digit() && slug.as_bytes()[3] != b'0');
    if reserved { format!("{slug}-project") } else { slug }
}

/// The letters of the Latin-1 and Latin Extended-A blocks without their accents, the ones NFKD splits.
fn fold_accent(c: char) -> Vec<char> {
    const FROM: &str = "ÀÁÂÃÄÅàáâãäåÇçÈÉÊËèéêëÌÍÎÏìíîïÑñÒÓÔÕÖòóôõöÙÚÛÜùúûüÝýÿ";
    const TO: &str = "AAAAAAaaaaaaCcEEEEeeeeIIIIiiiiNnOOOOOoooooUUUUuuuuYyy";
    FROM.chars().position(|f| f == c).map_or(vec![c], |i| vec![TO.chars().nth(i).unwrap()])
}

fn icon_svg(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let mut initials: String = words.iter().take(2).filter_map(|w| w.chars().next()).collect::<String>().to_uppercase();
    if initials.is_empty() {
        initials = name.trim().chars().next().map_or("?".into(), String::from);
    }
    let hash = name.chars().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32));
    let background = ICON_BACKGROUNDS[hash as usize % ICON_BACKGROUNDS.len()];
    let size = if initials.chars().count() > 1 { 26 } else { 32 };
    let text = initials.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\">\n  <rect width=\"64\" height=\"64\" rx=\"14\" fill=\"{background}\"/>\n  <text x=\"32\" y=\"32\" dy=\"0.35em\" text-anchor=\"middle\" font-family=\"ui-sans-serif, system-ui, -apple-system, sans-serif\" font-size=\"{size}\" font-weight=\"600\" fill=\"#ffffff\">{text}</text>\n</svg>\n"
    )
}

fn commit_failure(said: &str) -> String {
    let lower = said.to_lowercase();
    if ["identity unknown", "tell me who you are", "no name was given", "no email was given"].iter().any(|s| lower.contains(s)) {
        return "Git has no name or email on this machine. Set user.name and user.email, then commit.".into();
    }
    said.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("Git could not make the first commit.").to_owned()
}

/// The first free folder for `name` under `root`, made and claimed: `slug`, then `slug-2` and on.
fn claim(root: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("could not make {}: {e}", root.display()))?;
    let slug = folder_name(name);
    for attempt in 1..=ATTEMPTS {
        let dir = root.join(if attempt == 1 { slug.clone() } else { format!("{slug}-{attempt}") });
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not make {}: {e}", dir.display())),
        }
    }
    Err(format!("Every folder name for \"{slug}\" is taken."))
}

/// Makes the project `name` under `root`. A failed first commit leaves the project, and says why.
pub fn create(root: &Path, name: &str, path: Option<&str>) -> Result<Created, String> {
    create_as(root, name, path, &[])
}

/// `create`, committing with git's options `identity` first.
fn create_as(root: &Path, name: &str, path: Option<&str>, identity: &[&str]) -> Result<Created, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name the project.".into());
    }
    let dir = claim(root, name)?;
    let git = |args: &[&str]| run(&dir, path, "git", args).map_err(|e| match e {
        Failure::Missing => "Install git to create a project.".to_owned(),
        Failure::Said(said) => said,
    });
    let scaffold = || -> Result<(), String> {
        let branch = git(&["config", "--get", "init.defaultBranch"]).ok().map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()).unwrap_or_else(|| "main".into());
        git(&["init", "-q", &format!("--initial-branch={branch}")])?;
        std::fs::write(dir.join("README.md"), format!("<img src=\"assets/icon.svg\" width=\"64\" height=\"64\" alt=\"\">\n\n# {name}\n\nCreated in Pinned.\n")).map_err(|e| e.to_string())?;
        std::fs::create_dir(dir.join("assets")).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("assets/icon.svg"), icon_svg(name)).map_err(|e| e.to_string())?;
        // Named and forced so a global ignore rule (say `*.svg`) can't drop one.
        git(&["add", "--force", "--", "README.md", "assets/icon.svg"]).map(drop)
    };
    if let Err(e) = scaffold() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    let commit_error = git(&[identity, &["commit", "-q", "--message", "Initial commit"]].concat()).err().map(|said| commit_failure(&said));
    Ok(Created { path: dir.display().to_string(), commit_error })
}

/// Publishes the project at `dir` as a private GitHub repository named after its folder, and
/// returns the repository's URL.
pub fn publish(dir: &Path, path: Option<&str>) -> Result<String, String> {
    let name = dir.file_name().and_then(|n| n.to_str()).ok_or("the project has no folder name")?;
    let out = run(dir, path, "gh", &["repo", "create", name, "--private", "--source", ".", "--remote", "origin", "--push"]).map_err(|e| match e {
        Failure::Missing => "Install the GitHub command-line tool (`gh`) to publish the project.".to_owned(),
        Failure::Said(said) => format!("gh: {said}"),
    })?;
    Ok(out.lines().map(str::trim).filter(|l| l.starts_with("https://")).last().unwrap_or_default().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_follow_t3s_slugs() {
        assert_eq!(folder_name("My App"), "my-app");
        assert_eq!(folder_name("  Café   Crème! "), "cafe-creme");
        assert_eq!(folder_name("--Hello__World--"), "hello-world");
        assert_eq!(folder_name("😀"), "project");
        assert_eq!(folder_name("CON"), "con-project");
        assert_eq!(folder_name("com3"), "com3-project");
        assert_eq!(folder_name("com0"), "com0");
        assert_eq!(folder_name(&"a".repeat(80)).len(), 64);
    }

    #[test]
    fn a_new_project_is_a_repository_with_a_first_commit_in_a_free_folder() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("my-app")).unwrap();
        let created = create_as(root.path(), "My App", None, &["-c", "user.name=A", "-c", "user.email=a@b.c"]).unwrap();
        let dir = root.path().join("my-app-2");
        assert_eq!(created, Created { path: dir.display().to_string(), commit_error: None });
        assert!(std::fs::read_to_string(dir.join("README.md")).unwrap().contains("# My App"));
        assert!(std::fs::read_to_string(dir.join("assets/icon.svg")).unwrap().contains(">MA</text>"));
        let log = std::process::Command::new("git").args(["log", "--format=%s"]).current_dir(&dir).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "Initial commit");
    }

    #[test]
    fn a_missing_identity_is_named_instead_of_quoted() {
        assert_eq!(commit_failure("Author identity unknown\n\n*** Please tell me who you are."), "Git has no name or email on this machine. Set user.name and user.email, then commit.");
        assert_eq!(commit_failure("error: one\nfatal: two\n"), "fatal: two");
        assert_eq!(create(Path::new("/nowhere"), "  ", None), Err("Name the project.".into()));
    }
}
