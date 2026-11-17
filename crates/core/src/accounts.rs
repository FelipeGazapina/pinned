//! The agent CLI logins a machine runs sessions under. Each login lives in a CLI home folder of
//! its own, so two subscriptions of one provider coexist on one Mac.

use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Set on every session, so `pinned statusline` knows whose usage it reports.
pub const ACCOUNT_ENV: &str = "PINNED_ACCOUNT";
/// The account a session runs under when nothing names one.
pub const DEFAULT: &str = "claude";
/// A status check that takes longer than this counts as signed out.
const STATUS_WAIT: Duration = Duration::from_secs(10);
/// How long the 20 s machine report trusts a status the CLI gave, before it asks again.
pub const STATUS_FRESH: Duration = Duration::from_secs(300);
const FILE: &str = "accounts.json";

pub fn default_account() -> AccountId {
    DEFAULT.into()
}

/// One row per agent CLI pinned can host. A table, so adding a provider is one entry, not a
/// branch per call site.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Claude,
    Codex,
    Cursor,
}

pub struct Spec {
    pub name: &'static str,
    pub bin: &'static str,
    /// The id of the provider's built-in account, which runs on the CLI's own home.
    pub builtin: &'static str,
    /// Points the CLI at another home, and with it another login.
    pub home_env: &'static str,
    /// The CLI's home under `$HOME` while `home_env` is unset.
    pub default_home: &'static str,
    pub login: &'static [&'static str],
    pub status: &'static [&'static str],
    /// Reads what the `status` command printed and how it exited.
    pub parse_status: fn(&Output) -> Status,
    /// Whether pinned's hooks, statusline, MCP config, plugin and `--resume` apply.
    pub harness: bool,
    pub bypass_flag: &'static str,
}

const CLAUDE: Spec = Spec {
    name: "Claude Code",
    bin: "claude",
    builtin: "claude",
    home_env: "CLAUDE_CONFIG_DIR",
    default_home: ".claude",
    login: &["auth", "login"],
    status: &["auth", "status"],
    parse_status: claude_status,
    harness: true,
    bypass_flag: "--dangerously-skip-permissions",
};

const CODEX: Spec = Spec {
    name: "Codex",
    bin: "codex",
    builtin: "codex",
    home_env: "CODEX_HOME",
    default_home: ".codex",
    login: &["login"],
    status: &["login", "status"],
    parse_status: codex_status,
    harness: false,
    bypass_flag: "--dangerously-bypass-approvals-and-sandbox",
};

/// Cursor's CLI has one login, in its own home. An empty `home_env` is what refuses a second account.
const CURSOR: Spec = Spec {
    name: "Cursor",
    bin: "cursor-agent",
    builtin: "cursor",
    home_env: "",
    default_home: "",
    login: &["login"],
    status: &["status"],
    parse_status: cursor_status,
    harness: false,
    bypass_flag: "--force",
};

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Cursor];

    pub fn spec(self) -> &'static Spec {
        match self {
            Provider::Claude => &CLAUDE,
            Provider::Codex => &CODEX,
            Provider::Cursor => &CURSOR,
        }
    }
}

/// A lowercase slug of the account's name, such as `claude-teams`. The same on every machine of
/// the user, so a connector saved on one names the same account on another.
pub type AccountId = String;

/// How an account signs in. An account written before this field existed is a CLI login.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Auth {
    #[default]
    #[serde(rename = "login")]
    Login,
    #[serde(rename = "opencode-go")]
    OpenCodeGo,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, schemars::JsonSchema)]
pub struct Account {
    pub id: AccountId,
    pub provider: Provider,
    pub name: String,
    #[serde(default)]
    pub auth: Auth,
}

const KEY_FILE: &str = "opencode-go.key";

impl Account {
    pub fn builtin(provider: Provider) -> Account {
        let spec = provider.spec();
        Account { id: spec.builtin.into(), provider, name: spec.name.into(), auth: Auth::Login }
    }

    pub fn is_builtin(&self) -> bool {
        self.id == self.provider.spec().builtin
    }
}

/// Who the CLI says is signed in, as far as it tells.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug, schemars::JsonSchema)]
pub struct Status {
    pub signed_in: bool,
    pub email: Option<String>,
    pub org: Option<String>,
    pub plan: Option<String>,
}

/// The last status each account's CLI gave, so the machine report never runs a CLI itself. A
/// report reads what is here; a refresh, off the report's path, fills it in.
#[derive(Default)]
pub struct StatusCache {
    known: Mutex<HashMap<AccountId, Known>>,
    next_ticket: AtomicU64,
}

struct Known {
    seen: Option<(Instant, Status)>,
    /// The refresh that is asking the CLI now. A forgotten account drops its ticket, so that
    /// refresh's answer, asked before the change, is thrown away.
    asking: Option<u64>,
}

impl StatusCache {
    /// The last status of `id`, however old.
    pub fn get(&self, id: &str) -> Option<Status> {
        self.known.lock().unwrap().get(id)?.seen.as_ref().map(|(_, status)| status.clone())
    }

    /// Keeps what a live read just found.
    pub fn put(&self, id: &str, status: Status) {
        let mut known = self.known.lock().unwrap();
        let entry = known.entry(id.to_owned()).or_insert(Known { seen: None, asking: None });
        entry.seen = Some((Instant::now(), status));
    }

    /// Drops `id`: its sign-in changed, or it is gone.
    pub fn forget(&self, id: &str) {
        self.known.lock().unwrap().remove(id);
    }

    /// A ticket to refresh `id` with, when nothing is asking its CLI and its status is missing or
    /// at least `fresh` old. At most one refresh per account runs at a time.
    pub fn claim(&self, id: &str, fresh: Duration) -> Option<u64> {
        let mut known = self.known.lock().unwrap();
        let entry = known.entry(id.to_owned()).or_insert(Known { seen: None, asking: None });
        if entry.asking.is_some() || entry.seen.as_ref().is_some_and(|(at, _)| at.elapsed() < fresh) {
            return None;
        }
        let ticket = self.next_ticket.fetch_add(1, Ordering::SeqCst);
        entry.asking = Some(ticket);
        Some(ticket)
    }

    /// Keeps the answer of the refresh `ticket`, unless `id` was forgotten while it asked.
    pub fn finish(&self, id: &str, ticket: u64, status: Status) {
        let mut known = self.known.lock().unwrap();
        if let Some(entry) = known.get_mut(id).filter(|entry| entry.asking == Some(ticket)) {
            *entry = Known { seen: Some((Instant::now(), status)), asking: None };
        }
    }
}

/// An account as a client lists it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, schemars::JsonSchema)]
pub struct View {
    #[serde(flatten)]
    pub account: Account,
    pub provider_name: String,
    pub builtin: bool,
    #[serde(flatten)]
    pub status: Status,
    /// The models the composer's picker offers for a Claude Code login, newest first. An older
    /// machine sends none, and the picker falls back to its own short list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<crate::claude_models::ClaudeModel>,
    /// A usage limit stopped this account, so `delegate_task` routes around it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub limited: bool,
    /// What stopped it: the error's first line, or the usage window that is full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_reason: Option<String>,
    /// Unix seconds when the limit lifts, when the provider said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_resume_at: Option<u64>,
}

impl View {
    pub fn new(account: Account, status: Status) -> View {
        let provider_name = if account.auth == Auth::OpenCodeGo { "OpenCode Go" } else { account.provider.spec().name };
        View { provider_name: provider_name.into(), builtin: account.is_builtin(), account, status, models: vec![], limited: false, limit_reason: None, limit_resume_at: None }
    }
}

/// The built-in account of each provider, and the ones the user added in `<dir>/accounts.json`,
/// whose homes are `<dir>/accounts/<id>`.
pub struct Accounts {
    dir: Option<PathBuf>,
    home: PathBuf,
    /// Where the built-in Claude account's home is, instead of the environment's or `~/.claude`.
    claude_home: Option<PathBuf>,
    /// The homes the user exported for the built-in accounts.
    exported: ExportedHomes,
    added: Mutex<Vec<Account>>,
}

/// The CLI homes the user exported in the shell the node started from, which the built-in
/// accounts run on in place of the CLI's default one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportedHomes(Vec<(Provider, PathBuf)>);

impl ExportedHomes {
    /// The home the user exported for `provider`'s built-in account.
    pub fn of(&self, provider: Provider) -> Option<&Path> {
        self.0.iter().find(|(p, _)| *p == provider).map(|(_, home)| home.as_path())
    }
}

/// Which of the CLI homes in `env`, the node's own environment, the built-in accounts keep: one
/// the user exported in their own shell. None while `env` carries any other session variable: a
/// node started from inside a session that dropped only part of them would otherwise hand that
/// session's account home to the built-in account. Never one under the `accounts` folder of a
/// pinned data folder, `own_dir` or one that keeps `accounts.json`, which is an added account's.
pub fn exported_homes(env: &[(String, String)], own_dir: Option<&Path>) -> ExportedHomes {
    let homes: Vec<&str> = Provider::ALL.into_iter().map(|p| p.spec().home_env).filter(|name| !name.is_empty()).collect();
    let session = crate::machine::INHERITED_SESSION_ENV.iter().filter(|name| !homes.contains(name));
    if session.into_iter().any(|name| env.iter().any(|(key, _)| key == name)) {
        return ExportedHomes::default();
    }
    let data_dir = |dir: &Path| own_dir == Some(dir) || dir.join(FILE).is_file();
    let added = |home: &Path| home.ancestors().any(|a| a.file_name().is_some_and(|name| name == "accounts") && a.parent().is_some_and(data_dir));
    let exported = Provider::ALL.into_iter().filter(|p| !p.spec().home_env.is_empty()).filter_map(|p| {
        let home = env.iter().find(|(key, _)| key == p.spec().home_env).map(|(_, value)| PathBuf::from(value)).filter(|home| !home.as_os_str().is_empty())?;
        (!added(&home)).then_some((p, home))
    });
    ExportedHomes(exported.collect())
}

impl Accounts {
    /// The accounts kept in `dir`, or only the built-in ones without it. `home` is the user's,
    /// where the built-in accounts' CLIs keep their logins.
    pub fn load(dir: Option<&Path>, home: PathBuf) -> Accounts {
        let added = dir.and_then(|d| std::fs::read(d.join(FILE)).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Accounts { dir: dir.map(Path::to_path_buf), home, claude_home: None, exported: ExportedHomes::default(), added: Mutex::new(added) }
    }

    /// The built-in accounts run on the homes the user exported.
    pub fn with_exported_homes(mut self, homes: ExportedHomes) -> Accounts {
        self.exported = homes;
        self
    }

    /// The home the user exported for `provider`'s built-in account, when the node keeps it.
    pub fn exported_home(&self, provider: Provider) -> Option<&Path> {
        self.exported.of(provider)
    }

    /// The built-in Claude account's home moved to `home`, when there is one.
    pub fn with_claude_home(mut self, home: Option<PathBuf>) -> Accounts {
        self.claude_home = home;
        self
    }

    /// The built-in accounts, then the added ones in the order they were added.
    pub fn list(&self) -> Vec<Account> {
        Provider::ALL.into_iter().map(Account::builtin).chain(self.added.lock().unwrap().iter().cloned()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Account> {
        self.list().into_iter().find(|a| a.id == id)
    }

    /// Adds an account with its own empty home, where signing in keeps its login. OpenCode Go is
    /// a Claude Code account: the same CLI, pointed at Go's gateway by a key kept beside the home.
    pub fn add(&self, provider: Provider, name: &str, auth: Auth) -> Result<Account, String> {
        let Some(dir) = &self.dir else { return Err("this machine keeps no accounts".into()) };
        if auth == Auth::OpenCodeGo && provider != Provider::Claude {
            return Err("OpenCode Go runs through Claude Code".into());
        }
        if provider.spec().home_env.is_empty() {
            return Err(format!("{} signs in only through its built-in account", provider.spec().name));
        }
        let name = name.trim();
        let id = slug(name);
        if id.is_empty() {
            return Err("an account needs a name with a letter or digit".into());
        }
        // Held from the check to the save, so two adds of one name can't both pass.
        let mut added = self.added.lock().unwrap();
        if Provider::ALL.iter().any(|p| p.spec().builtin == id) || added.iter().any(|a| a.id == id) {
            return Err(format!("an account {id} already exists"));
        }
        let account = Account { id, provider, name: name.into(), auth };
        let mut folder = std::fs::DirBuilder::new();
        folder.recursive(true);
        // The home holds the CLI's login, so only its owner reads it.
        #[cfg(unix)]
        folder.mode(0o700);
        folder
            .create(self.home(&account))
            .map_err(|e| format!("could not make {}'s folder: {e}", account.id))?;
        added.push(account.clone());
        save(dir, &added)?;
        Ok(account)
    }

    /// Forgets the account. Its home stays on disk, so adding it again finds its login.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let Some(account) = self.get(id) else { return Err(format!("no account {id} on this machine")) };
        if account.is_builtin() {
            return Err(format!("{id} is built in and can't be removed"));
        }
        let mut added = self.added.lock().unwrap();
        added.retain(|a| a.id != id);
        save(self.dir.as_ref().expect("an added account has a folder"), &added)?;
        if account.auth == Auth::OpenCodeGo {
            let path = self.home(&account).join(KEY_FILE);
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(format!("could not delete {}'s key: {e}", account.id));
                }
            }
        }
        Ok(())
    }

    /// The Go key, trimmed. It is not part of the account record.
    pub fn key(&self, account: &Account) -> Option<String> {
        if account.auth != Auth::OpenCodeGo {
            return None;
        }
        let text = std::fs::read_to_string(self.home(account).join(KEY_FILE)).ok()?;
        let key = text.trim();
        if key.is_empty() { None } else { Some(key.to_owned()) }
    }

    /// Writes the Go key where only this user can read it. Refuses a login account and an empty key.
    pub fn set_key(&self, account: &Account, key: &str) -> Result<(), String> {
        if account.auth != Auth::OpenCodeGo {
            return Err("only an OpenCode Go account takes an API key".into());
        }
        let key = key.trim();
        if key.is_empty() {
            return Err("OpenCode Go needs an API key".into());
        }
        let path = self.home(account).join(KEY_FILE);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("could not save the OpenCode Go key: {e}"))?;
        }
        let mut options = crate::private::file();
        options.create(true).truncate(true);
        let mut file = options.open(&path).map_err(|e| format!("could not save the OpenCode Go key: {e}"))?;
        std::io::Write::write_all(&mut file, key.as_bytes()).map_err(|e| format!("could not save the OpenCode Go key: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("could not save the OpenCode Go key: {e}"))?;
        }
        Ok(())
    }

    /// The CLI home the account's sessions run on. A built-in account's is the one the user
    /// exported, else the CLI's default one: its sessions never inherit pinned's own `home_env`
    /// otherwise, which may name another account's home.
    pub fn home(&self, account: &Account) -> PathBuf {
        let spec = account.provider.spec();
        let exported = self.exported_home(account.provider).filter(|_| account.is_builtin());
        match (&self.dir, &self.claude_home, exported) {
            (Some(dir), _, _) if !account.is_builtin() => dir.join("accounts").join(&account.id),
            (_, Some(claude_home), _) if account.provider == Provider::Claude => claude_home.clone(),
            (_, _, Some(exported)) => exported.to_path_buf(),
            _ if spec.home_env.is_empty() => self.home.clone(),
            _ => self.home.join(spec.default_home),
        }
    }

    /// What every process of the account runs with. A built-in account sets only the home the
    /// user exported, so its CLI keeps the login it had before pinned knew about accounts.
    pub fn env(&self, account: &Account) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = self.home_env(account).into_iter().collect();
        env.push((ACCOUNT_ENV.to_owned(), account.id.clone()));
        if account.auth == Auth::OpenCodeGo {
            if let Some(key) = self.key(account) {
                env.extend(crate::opencode_go::env(&key));
            }
        }
        env
    }

    /// The variable that points the account's CLI at its home: an added account's own one, a
    /// built-in account's only when the user exported it.
    pub fn home_env(&self, account: &Account) -> Option<(String, String)> {
        let home_env = account.provider.spec().home_env.to_owned();
        if account.is_builtin() {
            return self.exported_home(account.provider).map(|_| (home_env, self.home(account).display().to_string()));
        }
        Some((home_env, self.home(account).display().to_string()))
    }

    /// What any process of the account's CLI starts with: its env and nothing a parent left.
    pub(crate) fn child_env(&self, account: &Account) -> crate::machine::ChildEnv<'static> {
        crate::machine::ChildEnv::new(&[], self.env(account))
    }

    /// Asks the account's CLI, at `bin`, who is signed in. A Go account is signed in while its key
    /// file holds a key. Its empty home would otherwise look signed out.
    pub fn status(&self, account: &Account, bin: &Path, path_env: Option<&str>) -> Status {
        if account.auth == Auth::OpenCodeGo {
            return Status { signed_in: self.key(account).is_some(), plan: Some("OpenCode Go".into()), ..Status::default() };
        }
        let spec = account.provider.spec();
        let mut cmd = crate::process::command(bin);
        self.child_env(account).apply(&mut cmd);
        cmd.args(spec.status).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(path) = path_env {
            cmd.env("PATH", path);
        }
        run(cmd).map(|out| (spec.parse_status)(&out)).unwrap_or_default()
    }
}

/// Runs `cmd` to its end, or kills it after `STATUS_WAIT`. Its output must fit a pipe's buffer.
pub(crate) fn run(mut cmd: Command) -> Option<Output> {
    let mut child = cmd.spawn().ok()?;
    let start = Instant::now();
    while child.try_wait().ok()?.is_none() {
        if start.elapsed() > STATUS_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().ok()
}

fn save(dir: &Path, added: &[Account]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(FILE), serde_json::to_vec_pretty(added).unwrap()).map_err(|e| format!("could not save the accounts: {e}"))
}

/// `Claude Teams` becomes `claude-teams`.
fn slug(name: &str) -> String {
    let dashed: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    dashed.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-")
}

/// `claude auth status` prints JSON whether or not anyone is signed in.
fn claude_status(out: &Output) -> Status {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reported {
        #[serde(default)]
        logged_in: bool,
        email: Option<String>,
        org_name: Option<String>,
        subscription_type: Option<String>,
    }
    match serde_json::from_slice::<Reported>(&out.stdout) {
        Ok(r) => Status { signed_in: r.logged_in, email: r.email, org: r.org_name, plan: r.subscription_type },
        Err(_) => Status::default(),
    }
}

/// `codex login status` says only how it is signed in, and exits 1 when it isn't.
fn codex_status(out: &Output) -> Status {
    Status { signed_in: out.status.success(), ..Status::default() }
}

/// `cursor-agent status` prints `✓ Logged in as <email>` when the built-in account is signed in.
fn cursor_status(out: &Output) -> Status {
    let text = String::from_utf8_lossy(&out.stdout);
    let signed_in = out.status.success() && text.to_ascii_lowercase().contains("logged in");
    let email = text.split(" as ").nth(1).and_then(|rest| rest.split_whitespace().next()).filter(|s| s.contains('@')).map(str::to_owned);
    Status { signed_in, email, ..Status::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-accounts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        path
    }

    fn ids(accounts: &Accounts) -> Vec<String> {
        accounts.list().into_iter().map(|a| a.id).collect()
    }

    #[test]
    fn an_added_account_is_named_by_a_slug_and_kept_across_loads() {
        let dir = temp("add");
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into());
        assert_eq!(accounts.add(Provider::Claude, "  Claude Teams! ", Auth::Login), Ok(Account { id: "claude-teams".into(), provider: Provider::Claude, name: "Claude Teams!".into(), auth: Auth::Login }));
        assert_eq!(accounts.add(Provider::Claude, "claude teams", Auth::Login), Err("an account claude-teams already exists".into()));
        assert_eq!(accounts.add(Provider::Codex, "Claude", Auth::Login), Err("an account claude already exists".into()), "a built-in id is taken");
        assert_eq!(accounts.add(Provider::Codex, " ?! ", Auth::Login), Err("an account needs a name with a letter or digit".into()));
        assert_eq!(ids(&Accounts::load(Some(&dir), "/Users/ana".into())), ["claude", "codex", "cursor", "claude-teams"]);
        assert_eq!(accounts.add(Provider::Cursor, "Other", Auth::Login), Err("Cursor signs in only through its built-in account".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_machine_without_a_folder_has_only_the_built_in_accounts() {
        let accounts = Accounts::load(None, "/Users/ana".into());
        assert_eq!(ids(&accounts), ["claude", "codex", "cursor"]);
        assert_eq!(accounts.add(Provider::Claude, "work", Auth::Login), Err("this machine keeps no accounts".into()));
    }

    #[test]
    fn removing_an_account_keeps_its_login_for_a_later_add() {
        let dir = temp("remove");
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into());
        let work = accounts.add(Provider::Claude, "work", Auth::Login).unwrap();
        std::fs::write(dir.join("accounts/work/.credentials.json"), "{}").unwrap();
        assert_eq!(accounts.remove("claude"), Err("claude is built in and can't be removed".into()));
        assert_eq!(accounts.remove("home"), Err("no account home on this machine".into()));
        assert_eq!(accounts.remove("work"), Ok(()));
        assert_eq!(ids(&Accounts::load(Some(&dir), "/Users/ana".into())), ["claude", "codex", "cursor"]);
        assert_eq!(accounts.add(Provider::Claude, "Work", Auth::Login).map(|a| accounts.home(&a)), Ok(accounts.home(&work)));
        assert!(dir.join("accounts/work/.credentials.json").is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn only_an_added_account_points_its_cli_at_another_home() {
        let dir = temp("env");
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into());
        let work = accounts.add(Provider::Claude, "work", Auth::Login).unwrap();
        let side = accounts.add(Provider::Codex, "side", Auth::Login).unwrap();
        let env = |id: &str| accounts.env(&accounts.get(id).unwrap());
        let pair = |k: &str, v: String| (k.to_owned(), v);
        assert_eq!(env("claude"), [pair("PINNED_ACCOUNT", "claude".into())]);
        assert_eq!(env("codex"), [pair("PINNED_ACCOUNT", "codex".into())]);
        assert_eq!(env("work"), [pair("CLAUDE_CONFIG_DIR", format!("{}/accounts/work", dir.display())), pair("PINNED_ACCOUNT", "work".into())]);
        assert_eq!(env("side"), [pair("CODEX_HOME", format!("{}/accounts/side", dir.display())), pair("PINNED_ACCOUNT", "side".into())]);
        assert_eq!(accounts.home(&accounts.get("claude").unwrap()), PathBuf::from("/Users/ana/.claude"));
        assert_eq!(accounts.home(&accounts.get("codex").unwrap()), PathBuf::from("/Users/ana/.codex"));
        let mode = |a: &Account| std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(accounts.home(a)).unwrap().permissions()) & 0o777;
        assert_eq!((mode(&work), mode(&side)), (0o700, 0o700));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The environment of a shell the user started the node from, with the homes they exported.
    fn exported_in_own_shell(claude: &str, codex: &str) -> Vec<(String, String)> {
        [("HOME", "/Users/ana"), ("PATH", "/usr/bin:/bin"), ("CLAUDE_CONFIG_DIR", claude), ("CODEX_HOME", codex)].into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
    }

    #[test]
    fn a_home_the_user_exported_stays_the_built_in_accounts_home() {
        use crate::machine::tests::assert_starts_with_only;
        let dir = temp("exported");
        let exported = exported_homes(&exported_in_own_shell("/Users/ana/claude-own", "/Users/ana/codex-own"), Some(&dir));
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into()).with_exported_homes(exported);
        let work = accounts.add(Provider::Claude, "work", Auth::Login).unwrap();
        let side = accounts.add(Provider::Codex, "side", Auth::Login).unwrap();
        let (claude, codex) = (Account::builtin(Provider::Claude), Account::builtin(Provider::Codex));
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        assert_eq!(accounts.home(&claude), PathBuf::from("/Users/ana/claude-own"));
        assert_eq!(accounts.home(&codex), PathBuf::from("/Users/ana/codex-own"));
        assert_eq!(accounts.exported_home(Provider::Claude), Some(Path::new("/Users/ana/claude-own")));
        assert_eq!(accounts.home_env(&claude), Some(pair("CLAUDE_CONFIG_DIR", "/Users/ana/claude-own")));
        assert_eq!(accounts.env(&claude), [pair("CLAUDE_CONFIG_DIR", "/Users/ana/claude-own"), pair("PINNED_ACCOUNT", "claude")]);
        assert_eq!(accounts.env(&codex), [pair("CODEX_HOME", "/Users/ana/codex-own"), pair("PINNED_ACCOUNT", "codex")]);
        assert_starts_with_only(&accounts.child_env(&claude), &[("CLAUDE_CONFIG_DIR", "/Users/ana/claude-own"), ("PINNED_ACCOUNT", "claude")]);
        assert_starts_with_only(&accounts.child_env(&codex), &[("CODEX_HOME", "/Users/ana/codex-own"), ("PINNED_ACCOUNT", "codex")]);
        // An added account runs on its own home, never the exported one.
        let (work_home, side_home) = (format!("{}/accounts/work", dir.display()), format!("{}/accounts/side", dir.display()));
        assert_eq!(accounts.home(&work), PathBuf::from(&work_home));
        assert_eq!(accounts.env(&work), [pair("CLAUDE_CONFIG_DIR", &work_home), pair("PINNED_ACCOUNT", "work")]);
        assert_eq!(accounts.env(&side), [pair("CODEX_HOME", &side_home), pair("PINNED_ACCOUNT", "side")]);
        assert_starts_with_only(&accounts.child_env(&work), &[("CLAUDE_CONFIG_DIR", work_home.as_str()), ("PINNED_ACCOUNT", "work")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_exported_home_beside_any_other_session_variable_is_dropped() {
        let mut checked = 0;
        for name in crate::machine::INHERITED_SESSION_ENV.into_iter().filter(|name| !["CLAUDE_CONFIG_DIR", "CODEX_HOME"].contains(name)) {
            let mut env = exported_in_own_shell("/Users/ana/claude-own", "/Users/ana/codex-own");
            env.push((name.to_owned(), "1".to_owned()));
            let accounts = Accounts::load(None, "/Users/ana".into()).with_exported_homes(exported_homes(&env, None));
            assert_eq!(accounts.home(&Account::builtin(Provider::Claude)), PathBuf::from("/Users/ana/.claude"), "kept beside {name}");
            assert_eq!(accounts.home(&Account::builtin(Provider::Codex)), PathBuf::from("/Users/ana/.codex"), "kept beside {name}");
            assert_eq!(accounts.env(&Account::builtin(Provider::Claude)), [("PINNED_ACCOUNT".to_owned(), "claude".to_owned())], "kept beside {name}");
            checked += 1;
        }
        assert_eq!(checked, 25);
        // The same home with nothing else is kept.
        let accounts = Accounts::load(None, "/Users/ana".into()).with_exported_homes(exported_homes(&exported_in_own_shell("/Users/ana/claude-own", "/Users/ana/codex-own"), None));
        assert_eq!(accounts.home(&Account::builtin(Provider::Claude)), PathBuf::from("/Users/ana/claude-own"));
    }

    #[test]
    fn an_exported_home_under_a_pinned_data_folders_accounts_is_dropped() {
        let own = temp("exported-own");
        let other = temp("exported-other");
        std::fs::write(other.join(FILE), "[]").unwrap();
        let plain = temp("exported-plain");
        let home_of = |claude: String| {
            let accounts = Accounts::load(Some(&own), "/Users/ana".into()).with_exported_homes(exported_homes(&exported_in_own_shell(&claude, "/Users/ana/codex-own"), Some(&own)));
            accounts.home(&Account::builtin(Provider::Claude))
        };
        assert_eq!(home_of(format!("{}/accounts/x", own.display())), PathBuf::from("/Users/ana/.claude"), "the node's own data folder");
        assert_eq!(home_of(format!("{}/accounts/x", other.display())), PathBuf::from("/Users/ana/.claude"), "another node's data folder");
        assert_eq!(home_of(format!("{}/accounts/x", plain.display())), PathBuf::from(format!("{}/accounts/x", plain.display())), "a folder pinned keeps nothing in");
        for dir in [own, other, plain] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_default_account_session_starts_with_no_other_accounts_home() {
        use crate::machine::tests::assert_starts_with_only;
        let dir = temp("child-env");
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into());
        let work = accounts.add(Provider::Claude, "work", Auth::Login).unwrap();
        let side = accounts.add(Provider::Codex, "side", Auth::Login).unwrap();
        assert_starts_with_only(&accounts.child_env(&Account::builtin(Provider::Claude)), &[("PINNED_ACCOUNT", "claude")]);
        assert_starts_with_only(&accounts.child_env(&Account::builtin(Provider::Codex)), &[("PINNED_ACCOUNT", "codex")]);
        let work_home = format!("{}/accounts/work", dir.display());
        assert_starts_with_only(&accounts.child_env(&work), &[("CLAUDE_CONFIG_DIR", work_home.as_str()), ("PINNED_ACCOUNT", "work")]);
        let side_home = format!("{}/accounts/side", dir.display());
        assert_starts_with_only(&accounts.child_env(&side), &[("CODEX_HOME", side_home.as_str()), ("PINNED_ACCOUNT", "side")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The fake CLIs print what the real ones printed on this Mac, signed in on the default home
    /// and signed out on any other.
    #[test]
    #[cfg(unix)]
    fn status_is_what_each_cli_reports_for_the_accounts_home() {
        let dir = temp("status");
        let claude = script(
            &dir,
            "claude",
            r#"[ "$1 $2" = "auth status" ] || exit 2
if [ -n "$CLAUDE_CONFIG_DIR" ]; then
  printf '{\n  "loggedIn": false,\n  "authMethod": "none",\n  "configDirectory": "%s"\n}\n' "$CLAUDE_CONFIG_DIR"; exit 1
fi
printf '{\n  "loggedIn": true,\n  "authMethod": "claude.ai",\n  "email": "ana@acme.dev",\n  "orgId": "598f9262",\n  "orgName": "Acme",\n  "subscriptionType": "max"\n}\n'"#,
        );
        let codex = script(&dir, "codex", r#"[ "$1 $2" = "login status" ] || exit 2
[ -n "$CODEX_HOME" ] && { echo "Not logged in"; exit 1; }
echo "Logged in using ChatGPT""#);
        let cursor = script(&dir, "cursor-agent", r#"[ "$1" = "status" ] || exit 2
echo "✓ Logged in as ana@acme.dev""#);
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into());
        accounts.add(Provider::Claude, "work", Auth::Login).unwrap();
        accounts.add(Provider::Codex, "side", Auth::Login).unwrap();
        let status = |id: &str, bin: &Path| accounts.status(&accounts.get(id).unwrap(), bin, None);
        assert_eq!(
            status("claude", &claude),
            Status { signed_in: true, email: Some("ana@acme.dev".into()), org: Some("Acme".into()), plan: Some("max".into()) }
        );
        assert_eq!(status("work", &claude), Status::default());
        assert_eq!(status("codex", &codex), Status { signed_in: true, ..Status::default() });
        assert_eq!(status("side", &codex), Status::default());
        assert_eq!(status("cursor", &cursor), Status { signed_in: true, email: Some("ana@acme.dev".into()), ..Status::default() });
        assert_eq!(status("claude", &dir.join("missing")), Status::default(), "a CLI that isn't installed signs no one in");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_account_lists_with_its_provider_and_status() {
        let view = View::new(Account { id: "work".into(), provider: Provider::Claude, name: "Work".into(), auth: Auth::Login }, Status { signed_in: true, email: Some("ana@acme.dev".into()), org: Some("Acme".into()), plan: Some("team".into()) });
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            serde_json::json!({
                "id": "work", "provider": "claude", "name": "Work", "auth": "login", "provider_name": "Claude Code", "builtin": false,
                "signed_in": true, "email": "ana@acme.dev", "org": "Acme", "plan": "team",
            })
        );
    }

    #[test]
    fn an_old_accounts_file_loads_as_a_cli_login() {
        let dir = temp("old-auth");
        std::fs::write(dir.join(FILE), r#"[{"id":"work","provider":"claude","name":"Work"}]"#).unwrap();
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into());
        let work = accounts.get("work").unwrap();
        assert_eq!(work.auth, Auth::Login);
        assert_eq!(accounts.key(&work), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_go_account_keeps_its_key_out_of_the_record_and_signs_in_only_with_it() {
        let dir = temp("go-key");
        let accounts = Accounts::load(Some(&dir), "/Users/ana".into());
        assert_eq!(accounts.add(Provider::Codex, "Go", Auth::OpenCodeGo).unwrap_err(), "OpenCode Go runs through Claude Code");
        let go = accounts.add(Provider::Claude, "Go", Auth::OpenCodeGo).unwrap();
        assert_eq!(go.auth, Auth::OpenCodeGo);
        assert_eq!(accounts.set_key(&go, "  ").unwrap_err(), "OpenCode Go needs an API key");
        let login = accounts.add(Provider::Claude, "Work", Auth::Login).unwrap();
        assert_eq!(accounts.set_key(&login, "oc_sk_test").unwrap_err(), "only an OpenCode Go account takes an API key");
        assert!(!accounts.status(&go, Path::new("missing"), None).signed_in);
        accounts.set_key(&go, "  oc_sk_test \n").unwrap();
        assert_eq!(accounts.key(&go).as_deref(), Some("oc_sk_test"));
        assert!(accounts.status(&go, Path::new("missing"), None).signed_in);
        let saved = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(!saved.contains("oc_sk_test"), "the key was written into accounts.json");
        let view = View::new(go.clone(), accounts.status(&go, Path::new("missing"), None));
        assert_eq!(view.provider_name, "OpenCode Go");
        assert!(!serde_json::to_string(&view).unwrap().contains("oc_sk_test"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(accounts.home(&go).join("opencode-go.key")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let env = accounts.env(&go);
        assert!(env.iter().any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &accounts.home(&go).display().to_string()));
        assert!(env.iter().any(|(k, v)| k == "PINNED_ACCOUNT" && v == "go"));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_AUTH_TOKEN" && v == "oc_sk_test"));
        assert!(env.iter().any(|(k, v)| k == "PINNED_OPENCODE_GO_KEY" && v == "oc_sk_test"));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_BASE_URL" && v == "https://opencode.ai/inference/go/anthropic"));
        let login_env = accounts.env(&login);
        assert!(login_env.iter().all(|(k, _)| k != "ANTHROPIC_AUTH_TOKEN" && k != "PINNED_OPENCODE_GO_KEY"));
        accounts.remove("go").unwrap();
        assert!(!accounts.home(&go).join("opencode-go.key").exists());
        assert!(accounts.home(&go).is_dir(), "removing a Go account still leaves its home");
        let _ = std::fs::remove_dir_all(dir);
    }
}
