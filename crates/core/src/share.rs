use crate::private;
use crate::provider::Provider;
use crate::upstash::CLAUDE_TOKEN_PREFIX;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RELAY_LIMIT: usize = 1 << 20;

const FILE: &str = "share.json";
const TOKENS: &str = "share-tokens";

#[derive(Serialize, Deserialize, Clone, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShareBody {
    Accounts {
        grants: Vec<Grant>,
        #[serde(default)]
        projects: Vec<ProjectChoice>,
    },
}

#[derive(Serialize, Deserialize, Clone, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Grant {
    Live {
        id: String,
        provider: Provider,
        name: String,
        revision: u64,
        token: String,
    },
    Gone { id: String, revision: u64 },
}

/// Which account a repository's sessions run under. `account: null` is the default account.
#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct ProjectChoice {
    pub repo: String,
    #[serde(default)]
    pub account: Option<String>,
    pub revision: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Saved {
    id: String,
    provider: Provider,
    name: String,
    revision: u64,
    #[serde(default)]
    removed: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Disk {
    #[serde(default)]
    grants: Vec<Saved>,
    #[serde(default)]
    projects: Vec<ProjectChoice>,
}

pub struct Ledger {
    dir: Option<PathBuf>,
    grants: Vec<Saved>,
    projects: Vec<ProjectChoice>,
}

impl Ledger {
    pub fn load(dir: Option<&Path>) -> Ledger {
        let disk = dir.and_then(|d| std::fs::read(d.join(FILE)).ok()).and_then(|b| serde_json::from_slice::<Disk>(&b).ok()).unwrap_or_default();
        Ledger { dir: dir.map(Path::to_path_buf), grants: disk.grants, projects: disk.projects }
    }

    pub fn keep(&mut self, id: &str, provider: Provider, name: &str, token: &str) -> Result<(), String> {
        let Some(dir) = self.dir.clone() else { return Err("this machine keeps no accounts".into()) };
        if !account_id(id) {
            return Err("a shared account has a bad id".into());
        }
        let token = accept_token(provider, token)?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 200 {
            return Err("a shared account needs a name".into());
        }
        if let Some(saved) = self.grants.iter().find(|g| g.id == id && !g.removed) {
            if saved.provider == provider && saved.name == name && read_secret(&dir, id).as_deref() == Some(token.as_str()) {
                return Ok(());
            }
        }
        let revision = self.grants.iter().find(|g| g.id == id).map(|g| g.revision.saturating_add(1)).unwrap_or(1);
        write_secret(&dir, id, &token)?;
        if let Some(saved) = self.grants.iter_mut().find(|g| g.id == id) {
            *saved = Saved { id: id.to_owned(), provider, name: name.to_owned(), revision, removed: false };
        } else {
            self.grants.push(Saved { id: id.to_owned(), provider, name: name.to_owned(), revision, removed: false });
        }
        self.save()
    }

    /// Forgets the login and leaves a record so other machines forget it too.
    pub fn revoke(&mut self, id: &str) -> Result<(), String> {
        let Some(dir) = self.dir.clone() else { return Ok(()) };
        delete_secret(&dir, id)?;
        let Some(saved) = self.grants.iter_mut().find(|g| g.id == id) else { return Ok(()) };
        if saved.removed {
            return Ok(());
        }
        saved.removed = true;
        saved.revision = saved.revision.saturating_add(1);
        self.save()
    }

    pub fn export(&self) -> Option<ShareBody> {
        let Some(dir) = &self.dir else { return None };
        let grants = self
            .grants
            .iter()
            .filter_map(|saved| {
                if saved.removed {
                    return Some(Grant::Gone { id: saved.id.clone(), revision: saved.revision });
                }
                let token = read_secret(dir, &saved.id)?;
                if accept_token(saved.provider, &token).is_err() {
                    return None;
                }
                Some(Grant::Live {
                    id: saved.id.clone(),
                    provider: saved.provider,
                    name: saved.name.clone(),
                    revision: saved.revision,
                    token,
                })
            })
            .collect::<Vec<_>>();
        if grants.is_empty() && self.projects.is_empty() {
            return None;
        }
        Some(ShareBody::Accounts { grants, projects: self.projects.clone() })
    }

    /// Takes grants and project choices that are newer than the ones already here.
    pub fn apply(&mut self, body: &ShareBody) -> Result<(), String> {
        let ShareBody::Accounts { grants, projects } = body;
        let Some(dir) = self.dir.clone() else { return Err("this machine keeps no accounts".into()) };
        let mut next_grants = self.grants.clone();
        let mut ops = vec![];
        for grant in grants {
            if !take_grant(grant)? {
                continue;
            }
            match grant {
                Grant::Live { id, provider, name, revision, token } => {
                    let token = accept_token(*provider, token)?;
                    let name = name.trim();
                    let saved = Saved { id: id.clone(), provider: *provider, name: name.to_owned(), revision: *revision, removed: false };
                    if let Some(index) = next_grants.iter().position(|g| g.id == *id) {
                        if *revision > next_grants[index].revision {
                            next_grants[index] = saved;
                            ops.push((id.clone(), Some(token)));
                        }
                    } else {
                        next_grants.push(saved);
                        ops.push((id.clone(), Some(token)));
                    }
                }
                Grant::Gone { id, revision } => {
                    if let Some(index) = next_grants.iter().position(|g| g.id == *id) {
                        if *revision > next_grants[index].revision {
                            next_grants[index].removed = true;
                            next_grants[index].revision = *revision;
                            ops.push((id.clone(), None));
                        }
                    } else {
                        next_grants.push(Saved { id: id.clone(), provider: Provider::Claude, name: String::new(), revision: *revision, removed: true });
                        ops.push((id.clone(), None));
                    }
                }
            }
        }
        let mut next_projects = self.projects.clone();
        for choice in projects {
            if !safe_repo(&choice.repo) || choice.account.as_deref().is_some_and(|id| !account_id(id)) {
                continue;
            }
            if let Some(index) = next_projects.iter().position(|p| p.repo == choice.repo) {
                if choice.revision > next_projects[index].revision {
                    next_projects[index] = choice.clone();
                }
            } else if choice.revision > 0 {
                next_projects.push(choice.clone());
            }
        }
        for (id, token) in ops {
            match token {
                Some(token) => write_secret(&dir, &id, &token)?,
                None => delete_secret(&dir, &id)?,
            }
        }
        self.grants = next_grants;
        self.projects = next_projects;
        self.save()
    }

    pub fn token(&self, id: &str) -> Option<String> {
        let saved = self.grants.iter().find(|g| g.id == id && !g.removed)?;
        let dir = self.dir.as_ref()?;
        let token = read_secret(dir, id)?;
        accept_token(saved.provider, &token).ok()
    }

    pub fn session_env(&self, provider: Provider, id: &str) -> Option<(String, String)> {
        let key = share_env(provider)?;
        let token = self.token(id)?;
        Some((key.to_owned(), token))
    }

    pub fn live(&self) -> Vec<(String, Provider, String)> {
        let Some(dir) = &self.dir else { return vec![] };
        self.grants
            .iter()
            .filter(|g| !g.removed && read_secret(dir, &g.id).is_some())
            .map(|g| (g.id.clone(), g.provider, g.name.clone()))
            .collect()
    }

    pub fn removed(&self) -> Vec<String> {
        self.grants.iter().filter(|g| g.removed).map(|g| g.id.clone()).collect()
    }

    pub fn note_project(&mut self, repo: &str, account: Option<String>) -> Result<(), String> {
        if !safe_repo(repo) || account.as_deref().is_some_and(|id| !account_id(id)) {
            return Ok(());
        }
        if let Some(found) = self.projects.iter_mut().find(|p| p.repo == repo) {
            if found.account == account {
                return Ok(());
            }
            found.account = account;
            found.revision = found.revision.saturating_add(1);
        } else {
            self.projects.push(ProjectChoice { repo: repo.to_owned(), account, revision: 1 });
        }
        self.save()
    }

    pub fn project(&self, repo: &str) -> Option<Option<String>> {
        self.projects.iter().find(|p| p.repo == repo).map(|p| p.account.clone())
    }

    pub fn projects(&self) -> Vec<ProjectChoice> {
        self.projects.clone()
    }

    fn save(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| format!("could not save a shared login: {e}"))?;
        let disk = Disk { grants: self.grants.clone(), projects: self.projects.clone() };
        std::fs::write(dir.join(FILE), serde_json::to_vec(&disk).unwrap()).map_err(|e| format!("could not save a shared login: {e}"))
    }
}

fn take_grant(grant: &Grant) -> Result<bool, String> {
    let (id, provider, token, name) = match grant {
        Grant::Gone { id, .. } => {
            if !account_id(id) {
                return Err("a shared account has a bad id".into());
            }
            return Ok(true);
        }
        Grant::Live { id, provider, token, name, .. } => (id, *provider, token, name),
    };
    if !account_id(id) {
        return Err("a shared account has a bad id".into());
    }
    if share_env(provider).is_none() {
        return Ok(false);
    }
    accept_token(provider, token)?;
    if name.trim().is_empty() || name.chars().count() > 200 {
        return Err("a shared account needs a name".into());
    }
    Ok(true)
}

struct SharedLogin {
    env: &'static str,
    prefix: Option<&'static str>,
}

fn shared_login(provider: Provider) -> Option<SharedLogin> {
    match provider {
        // `claude setup-token` prints this. The refresh token in the CLI home is a different secret.
        Provider::Claude => Some(SharedLogin { env: "CLAUDE_CODE_OAUTH_TOKEN", prefix: Some(CLAUDE_TOKEN_PREFIX) }),
        // `cursor-agent --help` names `CURSOR_API_KEY`. `cursor-agent login` stays on this machine.
        Provider::Cursor => Some(SharedLogin { env: "CURSOR_API_KEY", prefix: None }),
        // ChatGPT `auth.json` holds a rotating refresh token, so that file stays on this machine.
        Provider::Codex => Some(SharedLogin { env: "CODEX_API_KEY", prefix: None }),
    }
}

pub fn share_env(provider: Provider) -> Option<&'static str> {
    shared_login(provider).map(|login| login.env)
}

fn accept_token(provider: Provider, token: &str) -> Result<String, String> {
    let Some(login) = shared_login(provider) else {
        return Err(format!("{} has no login pinned can share, so it stays on this machine", label(provider)));
    };
    let token = token.trim();
    if token.is_empty() || token.len() > 4096 || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a shared login is one line".into());
    }
    if token.starts_with('{') || token.contains("refresh_token") {
        return Err("a shared login is a token, not a login file".into());
    }
    if let Some(prefix) = login.prefix {
        if !(token.starts_with(prefix) && token.len() > prefix.len()) {
            return Err(format!("a {} login pinned can share starts with {prefix}", label(provider)));
        }
    }
    Ok(token.to_owned())
}

fn label(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude Code",
        Provider::Codex => "Codex",
        Provider::Cursor => "Cursor",
    }
}

fn account_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn safe_repo(repo: &str) -> bool {
    (1..=200).contains(&repo.len())
        && repo.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'))
        && !repo.split('/').any(|part| part.is_empty() || part == "." || part == "..")
}

fn write_secret(dir: &Path, id: &str, token: &str) -> Result<(), String> {
    let folder = dir.join(TOKENS);
    private::dir(&folder).map_err(|e| format!("could not keep a shared login: {e}"))?;
    let path = folder.join(id);
    let tmp = folder.join(format!(".{id}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    {
        use std::io::Write;
        let mut file = private::file().create_new(true).open(&tmp).map_err(|e| format!("could not keep a shared login: {e}"))?;
        file.write_all(token.as_bytes()).map_err(|e| format!("could not keep a shared login: {e}"))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not keep a shared login: {e}"))?;
    Ok(())
}

fn read_secret(dir: &Path, id: &str) -> Option<String> {
    let token = std::fs::read_to_string(dir.join(TOKENS).join(id)).ok()?;
    let token = token.trim().to_owned();
    (!token.is_empty()).then_some(token)
}

fn delete_secret(dir: &Path, id: &str) -> Result<(), String> {
    let path = dir.join(TOKENS).join(id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not forget a shared login: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("pinned-share-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_claude_setup_token_is_kept_out_of_the_catalog_and_a_stale_grant_does_not_restore_it() {
        let token = "sk-ant-oat01-TESTTOKEN";
        let main_dir = dir("main");
        let mut main = Ledger::load(Some(&main_dir));
        main.keep("bloom", Provider::Claude, "Bloom", token).unwrap();
        let catalog = std::fs::read_to_string(main_dir.join(FILE)).unwrap();
        assert!(!catalog.contains(token), "{catalog}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(main_dir.join(TOKENS).join("bloom")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let first = main.export().unwrap();
        let remote_dir = dir("remote");
        let mut remote = Ledger::load(Some(&remote_dir));
        remote.apply(&first).unwrap();
        assert_eq!(remote.token("bloom").as_deref(), Some(token));
        assert_eq!(main.session_env(Provider::Claude, "bloom"), Some(("CLAUDE_CODE_OAUTH_TOKEN".into(), token.into())));
        main.revoke("bloom").unwrap();
        remote.apply(&main.export().unwrap()).unwrap();
        assert_eq!(remote.token("bloom"), None);
        assert!(!remote_dir.join(TOKENS).join("bloom").exists());
        remote.apply(&first).unwrap();
        assert_eq!(remote.token("bloom"), None);
        main.keep("cursor", Provider::Cursor, "Cursor", "crsr_test_key").unwrap();
        main.keep("codex", Provider::Codex, "Codex", "sk-codex-test").unwrap();
        assert_eq!(main.session_env(Provider::Cursor, "cursor"), Some(("CURSOR_API_KEY".into(), "crsr_test_key".into())));
        assert_eq!(main.session_env(Provider::Codex, "codex"), Some(("CODEX_API_KEY".into(), "sk-codex-test".into())));
        assert!(main.keep("claude", Provider::Claude, "Claude Code", "refresh-token").unwrap_err().contains("sk-ant-oat01-"));
        assert!(main.keep("file", Provider::Codex, "File", r#"{"refresh_token":"rotating"}"#).unwrap_err().contains("login file"));
        let _ = std::fs::remove_dir_all(main_dir);
        let _ = std::fs::remove_dir_all(remote_dir);
    }
}
