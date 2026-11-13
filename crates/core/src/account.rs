//! The calls block, so make them off any async runtime's workers (reqwest's blocking client
//! panics inside one). The binary installs rustls's ring provider before the first call.

use crate::login_store::LoginStore;
use crate::wire::{AccountView, MachineId, MachineView};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

const FILE: &str = "account.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Credentials {
    pub hub: String,
    pub email: String,
    pub user_token: String,
    pub machine_id: MachineId,
    pub machine_token: String,
}

impl Credentials {
    /// The file in `dir` that `load` reads and `remove` deletes.
    pub fn path(dir: &Path) -> std::path::PathBuf {
        dir.join(FILE)
    }

    pub fn load(dir: &Path) -> Option<Credentials> {
        serde_json::from_slice(&std::fs::read(Credentials::path(dir)).ok()?).ok()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        use std::io::Write;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("{FILE}.tmp"));
        let _ = std::fs::remove_file(&tmp);
        crate::private::file()
            .create_new(true)
            .open(&tmp)
            .and_then(|mut f| f.write_all(&serde_json::to_vec_pretty(self).unwrap()))
            .and_then(|()| std::fs::rename(&tmp, dir.join(FILE)))
            .map_err(|e| format!("could not save {}: {e}", dir.join(FILE).display()))
    }

    pub fn remove(dir: &Path) {
        let _ = std::fs::remove_file(Credentials::path(dir));
    }

    /// The entry the app's login store keeps this login under: the hub, then the account's email,
    /// which the hub keeps unique and never changes. `account.json` holds both, so a start with no
    /// network names the entry too.
    pub fn login_account(&self) -> String {
        format!("{} {}", self.hub, self.email)
    }

    /// The app's credentials. With a store, the login comes from it, and a login `account.json`
    /// still holds moves there first: written, read back, then taken out of the file. A move that
    /// fails or stops halfway leaves the file's login in place, and the next start tries again.
    /// When the file has no login and the store has none to give, or fails, the app is signed out.
    pub fn load_app(dir: &Path, store: Option<&dyn LoginStore>) -> Option<Credentials> {
        let mut creds = Credentials::load(dir)?;
        let Some(store) = store else { return Some(creds) };
        if !creds.user_token.is_empty() {
            if let Err(e) = creds.move_login(dir, store) {
                eprintln!("pinned: the hub login stays in account.json for now: {e}");
            }
        } else {
            match store.get(&creds.login_account()) {
                Ok(Some(login)) => creds.user_token = login,
                // Signed out: a link started with an empty token would never connect. The file
                // stays, and a new sign-in overwrites it.
                Ok(None) => return None,
                Err(e) => {
                    eprintln!("pinned: could not read the hub login: {e}");
                    return None;
                }
            }
        }
        Some(creds)
    }

    /// Saves the app's credentials: the login to the store, every other field to `account.json`.
    /// When the store refuses the login, the file keeps it, as before, until a start moves it.
    pub fn save_app(&self, dir: &Path, store: Option<&dyn LoginStore>) -> Result<(), String> {
        let Some(store) = store else { return self.save(dir) };
        self.move_login(dir, store).or_else(|e| {
            eprintln!("pinned: the hub login stays in account.json for now: {e}");
            self.save(dir)
        })
    }

    /// Forgets the app's credentials. The file goes first, so a sign-out cut short leaves the app
    /// signed out rather than signed in without its login.
    pub fn remove_app(&self, dir: &Path, store: Option<&dyn LoginStore>) {
        Credentials::remove(dir);
        if let Some(Err(e)) = store.map(|store| store.delete(&self.login_account())) {
            eprintln!("pinned: could not delete the hub login from the store: {e}");
        }
    }

    fn move_login(&self, dir: &Path, store: &dyn LoginStore) -> Result<(), String> {
        let account = self.login_account();
        store.set(&account, &self.user_token)?;
        if store.get(&account)?.as_deref() != Some(self.user_token.as_str()) {
            return Err("the store did not keep it".into());
        }
        Credentials { user_token: String::new(), ..self.clone() }.save(dir)
    }

    /// Saves these credentials without the login an older pinned kept beside the machine's token,
    /// and answers it, so the caller ends it at the hub. A machine needs only its own token, and a
    /// login on its disk lets anything running as this user act as the account's owner.
    pub fn forget_login(&mut self, dir: &Path) -> Result<Option<String>, String> {
        if self.user_token.is_empty() {
            return Ok(None);
        }
        let login = std::mem::take(&mut self.user_token);
        self.save(dir)?;
        Ok(Some(login))
    }
}

/// `POST /api/signup` and `/api/login` take this.
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct SignIn {
    pub email: String,
    pub password: String,
    /// The invite code a hub in `invite` mode asks a new account for. Sign in ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
}

/// Who may create an account on a hub.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Signup {
    /// Only the hub's first account.
    First,
    /// Anyone who reaches the hub.
    Open,
    /// The first account, then whoever holds an invite code an account made.
    Invite,
}

/// `GET /api/signup` answers this, before anyone signs in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct SignupInfo {
    pub signup: Signup,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct Session {
    pub token: String,
    pub account: AccountView,
}

/// `POST /api/password`, with the user token as bearer, takes this.
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct NewPassword {
    pub current: String,
    pub password: String,
}

/// `POST /api/dictation/feedback`, with the user token as bearer, takes this: one dictation
/// session the user rated and agreed to share. Text only, never audio.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct DictationFeedback {
    pub rating: DictationRating,
    /// The final phrases exactly as the recognizer returned them, one per line.
    pub raw: String,
    /// The same phrases after the composer's cleanup, one per line.
    pub cleaned: String,
    /// The composer's text when the session ended.
    pub final_text: String,
    pub locale: String,
    pub engine: DictationEngine,
    pub app_version: String,
    pub platform: String,
    pub duration_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DictationRating {
    Good,
    Bad,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DictationEngine {
    Native,
    Web,
}

/// `POST /api/machines`, with the user token as bearer, takes this.
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct NewMachine {
    pub name: String,
    pub os: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct Registered {
    pub machine: MachineView,
    pub token: String,
}

/// `POST /api/pair` takes this.
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pairing {
    pub code: String,
    pub name: String,
    pub os: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct Paired {
    pub machine: MachineView,
    pub token: String,
    /// The hub's public URL, which the machine links to from then on.
    pub hub: String,
    pub email: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct Failure {
    pub error: String,
}

pub fn signup(hub: &str, email: &str, password: &str) -> Result<(String, AccountView), String> {
    signup_invited(hub, email, password, None)
}

/// Signs up with the invite code a hub in `invite` mode asks for, when there is one.
pub fn signup_invited(hub: &str, email: &str, password: &str, invite: Option<&str>) -> Result<(String, AccountView), String> {
    let invite = invite.map(str::trim).filter(|code| !code.is_empty()).map(str::to_owned);
    let session: Session = post(hub, "/api/signup", None, &SignIn { email: email.into(), password: password.into(), invite })?;
    Ok((session.token, session.account))
}

pub fn login(hub: &str, email: &str, password: &str) -> Result<(String, AccountView), String> {
    let session: Session = post(hub, "/api/login", None, &SignIn { email: email.into(), password: password.into(), invite: None })?;
    Ok((session.token, session.account))
}

pub fn change_password(hub: &str, user_token: &str, current: &str, password: &str) -> Result<(), String> {
    post::<AccountView>(hub, "/api/password", Some(user_token), &NewPassword { current: current.into(), password: password.into() }).map(drop)
}

pub fn register_machine(hub: &str, user_token: &str, name: &str, os: &str) -> Result<(MachineView, String), String> {
    let registered: Registered = post(hub, "/api/machines", Some(user_token), &NewMachine { name: name.into(), os: os.into() })?;
    Ok((registered.machine, registered.token))
}

pub fn dictation_feedback(hub: &str, user_token: &str, feedback: &DictationFeedback) -> Result<(), String> {
    send(hub, "/api/dictation/feedback", Some(user_token), feedback).map(drop)
}

/// Adds this machine to the account that made `code`. The credentials name the hub's public URL,
/// and carry no user token.
pub fn pair(hub: &str, code: &str, name: &str, os: &str) -> Result<Credentials, String> {
    let pairing = Pairing { code: code.trim().to_uppercase(), name: name.into(), os: os.into() };
    let paired: Paired = post(hub, "/api/pair", None, &pairing)?;
    Ok(Credentials {
        hub: paired.hub,
        email: paired.email,
        user_token: String::new(),
        machine_id: paired.machine.id,
        machine_token: paired.token,
    })
}

/// Ends the login, so its token no longer opens the hub. The machine's own token stays valid.
pub fn logout(hub: &str, user_token: &str) -> Result<(), String> {
    let hub = hub.trim().trim_end_matches('/');
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let response = client.post(format!("{hub}/api/logout")).bearer_auth(user_token).send().map_err(|e| format!("could not reach {hub}: {e}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("{hub} answered {}", response.status()))
    }
}

pub fn hostname() -> String {
    let name = system_hostname();
    let name = name.trim();
    let name = name.strip_suffix(".local").unwrap_or(name);
    if name.is_empty() {
        "machine".into()
    } else {
        name.to_owned()
    }
}

/// The kernel's host name, as `hostname` prints it on Linux, without starting a process.
#[cfg(target_os = "linux")]
fn system_hostname() -> String {
    let mut buf = [0u8; 256];
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[cfg(not(target_os = "linux"))]
fn system_hostname() -> String {
    let out = crate::process::command("hostname").stderr(std::process::Stdio::null()).output();
    out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// The system the hub lists the machine under: `std::env::consts::OS`, or "wsl" for Linux running
/// under Windows' WSL, whose folders T3 Code's palette opens on their own.
pub fn os() -> String {
    os_named(std::env::consts::OS, std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().as_deref())
}

fn os_named(os: &str, kernel_release: Option<&str>) -> String {
    let wsl = os == "linux" && kernel_release.is_some_and(|r| r.to_ascii_lowercase().contains("microsoft"));
    if wsl { "wsl".into() } else { os.into() }
}

fn post<T: DeserializeOwned>(hub: &str, path: &str, bearer: Option<&str>, body: &impl Serialize) -> Result<T, String> {
    let text = send(hub, path, bearer, body)?;
    serde_json::from_str(&text).map_err(|e| format!("{} answered something pinned can't read: {e}", hub.trim().trim_end_matches('/')))
}

/// The body of a successful answer, or the hub's own words for a failed one.
fn send(hub: &str, path: &str, bearer: Option<&str>, body: &impl Serialize) -> Result<String, String> {
    let hub = hub.trim().trim_end_matches('/');
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let mut request = client.post(format!("{hub}{path}")).json(body);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request.send().map_err(|e| format!("could not reach {hub}: {e}"))?;
    let status = response.status();
    let text = response.text().map_err(|e| format!("{hub} answered {status}: {e}"))?;
    if !status.is_success() {
        return Err(serde_json::from_str::<Failure>(&text).map_or_else(|_| format!("{hub} answered {status}"), |f| f.error));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::login_store::fake::{Memory, Reads, Writes};

    #[cfg(target_os = "linux")]
    #[test]
    fn the_host_name_is_the_one_hostname_prints() {
        let out = crate::process::command("hostname").output().unwrap();
        assert_eq!(system_hostname(), String::from_utf8_lossy(&out.stdout).trim());
    }

    #[test]
    fn credentials_are_saved_for_the_user_only_and_removed() {
        let dir = std::env::temp_dir().join(format!("pinned-account-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Credentials::load(&dir), None);
        let creds = Credentials {
            hub: "https://pinned.example.com".into(),
            email: "ana@acme.dev".into(),
            user_token: String::new(),
            machine_id: "8c0f1f4e-5b8a-4c4e-9a59-0d6b4f0f2a11".into(),
            machine_token: "m-t0k".into(),
        };
        creds.save(&dir).unwrap();
        creds.save(&dir).unwrap();
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.join("account.json")).unwrap().permissions()) & 0o777, 0o600);
        assert_eq!(Credentials::load(&dir), Some(creds));
        Credentials::remove(&dir);
        assert_eq!(Credentials::load(&dir), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_machine_forgets_the_login_an_older_pinned_kept_beside_its_token() {
        let dir = std::env::temp_dir().join(format!("pinned-forget-login-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut creds = Credentials {
            hub: "https://pinned.example.com".into(),
            email: "ana@acme.dev".into(),
            user_token: "u-t0k".into(),
            machine_id: "8c0f1f4e-5b8a-4c4e-9a59-0d6b4f0f2a11".into(),
            machine_token: "m-t0k".into(),
        };
        creds.save(&dir).unwrap();
        assert_eq!(creds.forget_login(&dir), Ok(Some("u-t0k".into())));
        let saved = std::fs::read_to_string(dir.join("account.json")).unwrap();
        assert!(!saved.contains("u-t0k") && saved.contains("m-t0k"), "{saved}");
        assert_eq!(Credentials::load(&dir).unwrap().user_token, "");
        assert_eq!(creds.forget_login(&dir), Ok(None), "nothing left to forget");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn signed_in(email: &str, login: &str) -> Credentials {
        Credentials {
            hub: "https://pinned.example.com".into(),
            email: email.into(),
            user_token: login.into(),
            machine_id: "8c0f1f4e-5b8a-4c4e-9a59-0d6b4f0f2a11".into(),
            machine_token: "m-t0k".into(),
        }
    }

    fn on_disk(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("account.json")).unwrap()
    }

    #[test]
    fn an_older_login_moves_from_account_json_into_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        signed_in("ana@acme.dev", "u-t0k").save(dir.path()).unwrap();

        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(signed_in("ana@acme.dev", "u-t0k")));
        assert_eq!(store.logins(), [("https://pinned.example.com ana@acme.dev".to_owned(), "u-t0k".to_owned())]);
        assert_eq!(Credentials::load(dir.path()), Some(signed_in("ana@acme.dev", "")), "the file keeps every field but the login");
        assert!(!on_disk(dir.path()).contains("u-t0k"));
    }

    #[test]
    fn moving_the_login_again_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        signed_in("ana@acme.dev", "u-t0k").save(dir.path()).unwrap();
        Credentials::load_app(dir.path(), Some(&store)).unwrap();
        let (file, logins) = (on_disk(dir.path()), store.logins());

        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(signed_in("ana@acme.dev", "u-t0k")));
        assert_eq!((on_disk(dir.path()), store.logins()), (file.clone(), logins.clone()));

        signed_in("ana@acme.dev", "u-t0k").save(dir.path()).unwrap();
        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(signed_in("ana@acme.dev", "u-t0k")), "a move cut short after the store's write");
        assert_eq!((on_disk(dir.path()), store.logins()), (file, logins), "ends where a whole move ends");
    }

    #[test]
    fn a_store_that_does_not_keep_the_login_leaves_it_in_account_json() {
        for writes in [Writes::Refused, Writes::Lost] {
            let dir = tempfile::tempdir().unwrap();
            let store = Memory::with(writes);
            signed_in("ana@acme.dev", "u-t0k").save(dir.path()).unwrap();
            let file = on_disk(dir.path());

            assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(signed_in("ana@acme.dev", "u-t0k")), "the app still signs in");
            assert_eq!(on_disk(dir.path()), file, "the file is untouched");
            assert_eq!(store.logins(), []);

            let working = Memory::default();
            assert_eq!(Credentials::load_app(dir.path(), Some(&working)), Some(signed_in("ana@acme.dev", "u-t0k")));
            assert_eq!(working.logins().len(), 1, "a later start moves it");
            assert_eq!(Credentials::load(dir.path()).unwrap().user_token, "");
        }
    }

    #[test]
    fn a_store_that_cannot_give_the_login_means_signed_out() {
        let dir = tempfile::tempdir().unwrap();
        signed_in("ana@acme.dev", "").save(dir.path()).unwrap();
        let file = on_disk(dir.path());

        assert_eq!(Credentials::load_app(dir.path(), Some(&Memory::with_reads(Reads::Refused))), None, "a failing read");
        assert_eq!(Credentials::load_app(dir.path(), Some(&Memory::default())), None, "an item that is gone");
        assert_eq!(on_disk(dir.path()), file, "the file stays for a new sign-in to overwrite");

        let store = Memory::default();
        store.set("https://pinned.example.com ana@acme.dev", "u-t0k").unwrap();
        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(signed_in("ana@acme.dev", "u-t0k")), "a login it has still loads");
    }

    #[test]
    fn signing_in_puts_the_login_in_the_store_and_the_rest_in_account_json() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        signed_in("ana@acme.dev", "u-t0k").save_app(dir.path(), Some(&store)).unwrap();
        assert_eq!(Credentials::load(dir.path()), Some(signed_in("ana@acme.dev", "")));
        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(signed_in("ana@acme.dev", "u-t0k")));

        let refused = tempfile::tempdir().unwrap();
        signed_in("ana@acme.dev", "u-t0k").save_app(refused.path(), Some(&Memory::with(Writes::Refused))).unwrap();
        assert_eq!(Credentials::load(refused.path()), Some(signed_in("ana@acme.dev", "u-t0k")), "a refused login is not lost");
    }

    #[test]
    fn signing_out_deletes_account_json_and_the_store_entry() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        let ana = signed_in("ana@acme.dev", "u-t0k");
        ana.save_app(dir.path(), Some(&store)).unwrap();

        ana.remove_app(dir.path(), Some(&store));
        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), None);
        assert_eq!(store.logins(), []);
        ana.remove_app(dir.path(), Some(&store));
    }

    #[test]
    fn switching_accounts_leaves_only_the_new_login_in_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        let ana = signed_in("ana@acme.dev", "u-ana");
        ana.save_app(dir.path(), Some(&store)).unwrap();

        ana.remove_app(dir.path(), Some(&store));
        signed_in("bob@acme.dev", "u-bob").save_app(dir.path(), Some(&store)).unwrap();
        assert_eq!(store.logins(), [("https://pinned.example.com bob@acme.dev".to_owned(), "u-bob".to_owned())]);
        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(signed_in("bob@acme.dev", "u-bob")));
    }

    #[test]
    fn without_a_store_the_login_stays_in_account_json() {
        let dir = tempfile::tempdir().unwrap();
        signed_in("ana@acme.dev", "u-t0k").save_app(dir.path(), None).unwrap();
        assert_eq!(Credentials::load(dir.path()), Some(signed_in("ana@acme.dev", "u-t0k")));
        assert_eq!(Credentials::load_app(dir.path(), None), Some(signed_in("ana@acme.dev", "u-t0k")));
        signed_in("ana@acme.dev", "u-t0k").remove_app(dir.path(), None);
        assert_eq!(Credentials::load_app(dir.path(), None), None);
    }

    #[test]
    fn linux_under_wsl_is_named_wsl() {
        assert_eq!(os_named("linux", Some("5.15.153.1-microsoft-standard-WSL2\n")), "wsl");
        assert_eq!(os_named("linux", Some("6.8.0-45-generic\n")), "linux");
        assert_eq!(os_named("linux", None), "linux");
        assert_eq!(os_named("macos", Some("microsoft")), "macos");
    }
}
