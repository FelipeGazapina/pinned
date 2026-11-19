use pinned_core::account::{self, Credentials};
use pinned_core::connectors::{self, Connectors, Keys};
use pinned_core::link::{self, Link};
use pinned_core::login_store::{self, LoginStore};
use pinned_core::machine::{Event, Listener, Machine};
use pinned_core::standby::Standby;
use pinned_core::wire::HubCall;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};

/// What sign-out answers while this Mac still owes the hub a handback.
const HELD_SIGN_OUT: &str = "this Mac still has changes to hand back";

pub struct Account {
    dir: PathBuf,
    exe: String,
    /// Holds the login where this system has a credential store; `account.json` keeps the rest.
    store: Option<Box<dyn LoginStore>>,
    /// A connector callback holds a read lock while the hub answers, so signing out waits for it.
    signed_in: RwLock<Option<SignedIn>>,
    watcher: Mutex<Option<JoinHandle<()>>>,
}

struct SignedIn {
    creds: Credentials,
    link: Link,
    standby: Arc<Standby>,
}

#[derive(serde::Serialize, Default)]
pub struct View {
    signed_in: bool,
    online: bool,
    hub: String,
    email: String,
    machine_id: String,
    user_token: String,
}

impl Account {
    pub fn start(app: &AppHandle, dir: PathBuf, machine: Arc<Machine>, exe: String) -> Account {
        let store = login_store::system();
        let signed_in = Credentials::load_app(&dir, store.as_deref()).map(|creds| begin(app, &dir, &exe, creds, machine.clone()));
        let watcher = signed_in.is_none().then(|| watch_connectors(app.clone(), machine));
        Account { dir, exe, store, signed_in: RwLock::new(signed_in), watcher: Mutex::new(watcher) }
    }

    /// Stops the link and ends its login: the copy, then on disk, in the store, then at the hub.
    fn forget(&self, SignedIn { creds, link, standby }: SignedIn) {
        link.stop();
        standby.forget();
        creds.remove_app(&self.dir, self.store.as_deref());
        if let Err(e) = account::logout(&creds.hub, &creds.user_token) {
            eprintln!("pinned: sign out: {e}");
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.signed_in.read().unwrap().is_some()
    }

    /// The hub and the user token its HTTP routes take.
    fn login(&self) -> Result<(String, String), String> {
        let signed_in = self.signed_in.read().unwrap();
        let creds = &signed_in.as_ref().ok_or("sign in first")?.creds;
        Ok((creds.hub.clone(), creds.user_token.clone()))
    }

    pub fn forward(&self, message: pinned_core::agent::Message) -> Result<Result<String, String>, pinned_core::agent::Message> {
        match self.signed_in.read().unwrap().as_ref() {
            Some(signed_in) => Ok(signed_in.link.forward(message)),
            None => Err(message),
        }
    }

    fn view(&self) -> View {
        let Some(SignedIn { creds, link, .. }) = &*self.signed_in.read().unwrap() else { return View::default() };
        View {
            signed_in: true,
            online: link.online(),
            hub: creds.hub.clone(),
            email: creds.email.clone(),
            machine_id: creds.machine_id.clone(),
            user_token: creds.user_token.clone(),
        }
    }
}

fn copy_dir(dir: &Path, machine_id: &str) -> PathBuf {
    dir.join("account-copy").join(machine_id)
}

/// The copy owes a handback, and this sign-out was not told to drop it.
fn owes_handback(standby: &Standby, anyway: bool) -> bool {
    standby.holds_changes() && !anyway
}

fn begin(app: &AppHandle, dir: &Path, exe: &str, creds: Credentials, machine: Arc<Machine>) -> SignedIn {
    let standby = Arc::new(Standby::open(
        copy_dir(dir, &creds.machine_id),
        pinned_core::standby::Signer { machine: creds.machine_id.clone(), name: computer_name(), login: creds.user_token.clone() },
        machine.clone(),
        exe.to_owned(),
    ));
    let local: Arc<dyn link::Local> = standby.clone();
    let link = link::spawn_with(creds.clone(), machine, env!("CARGO_PKG_VERSION"), Some(local));
    let emit = app.clone();
    standby.follow(link.handle(), Box::new(move |check| drop(emit.emit("connectors-checked", check))));
    SignedIn { creds, link, standby }
}

fn watch_connectors(app: AppHandle, machine: Arc<Machine>) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let id = machine.subscribe(local_tabs(app.clone(), machine.clone()));
        let _stop = StopListening { machine: machine.clone(), id };
        let initial = machine.tabs();
        let connectors = app.state::<Connectors>();
        if let Err(error) = connectors.on_tabs("local", connectors::unix_millis(), &initial, local_open(machine.clone())).await {
            eprintln!("pinned: could not wake a workflow run: {error}");
        }
        let checked = |check: &connectors::Check| drop(app.emit("connectors-checked", check));
        connectors.watch(local_open(machine), checked).await
    })
}

struct StopListening {
    machine: Arc<Machine>,
    id: u64,
}

impl Drop for StopListening {
    fn drop(&mut self) {
        self.machine.unsubscribe(self.id);
    }
}

fn local_tabs(app: AppHandle, machine: Arc<Machine>) -> Listener {
    Box::new(move |event| {
        let Event::Tabs { tabs } = event else { return };
        let observed = connectors::unix_millis();
        let tabs = tabs.clone();
        let app = app.clone();
        let machine = machine.clone();
        // The listener can run on the thread that holds a run, so the wake waits on its own task.
        tauri::async_runtime::spawn(async move {
            if let Err(error) = app.state::<Connectors>().on_tabs("local", observed, &tabs, local_open(machine)).await {
                eprintln!("pinned: could not wake a workflow run: {error}");
            }
        });
    })
}

fn local_open(
    machine: Arc<Machine>,
) -> impl Fn(connectors::Work) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<connectors::Placement>, String>> + Send>> + Send + Sync {
    move |work| {
        let machine = machine.clone();
        Box::pin(async move { take_local(machine, work).await })
    }
}

async fn take_local(machine: Arc<Machine>, work: connectors::Work) -> Result<Option<connectors::Placement>, String> {
    match work {
        connectors::Work::Run(run) if matches!(run.runs_on, connectors::RunsOn::FreshBox(_)) => {
            Err("this workflow runs in an Upstash Box, which needs pinned signed in".into())
        }
        connectors::Work::Run(run) => {
            let spec = run.spec();
            let tab = connectors::off_worker(move || machine.run(&spec)).await?;
            Ok(Some(connectors::Placement { machine: "local".into(), tab }))
        }
        connectors::Work::Delivery(delivery) => {
            if machine.deliver(&delivery.env(), &delivery.text()) { Ok(None) } else { Err("no session answers that thread".into()) }
        }
        connectors::Work::Prompt { env, text } => {
            if machine.deliver(&env, &text) { Ok(None) } else { Err("no session answers that thread".into()) }
        }
    }
}

/// The name the hub lists this computer under: the one the user gave it in system settings.
#[cfg(target_os = "linux")]
fn computer_name() -> String {
    std::fs::read_to_string("/etc/machine-info").ok().and_then(|info| pretty_hostname(&info)).unwrap_or_else(account::hostname)
}

/// `PRETTY_HOSTNAME` from systemd's /etc/machine-info, which GNOME and KDE settings name the device.
#[cfg(any(target_os = "linux", test))]
fn pretty_hostname(info: &str) -> Option<String> {
    let value = info.lines().find_map(|line| line.trim().strip_prefix("PRETTY_HOSTNAME="))?;
    let value = value.trim();
    let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
    (!value.trim().is_empty()).then(|| value.replace("\\\"", "\"").trim().to_owned())
}

#[cfg(not(target_os = "linux"))]
fn computer_name() -> String {
    pinned_core::process::command("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(account::hostname)
}

#[tauri::command]
pub fn account(account: State<Account>) -> View {
    account.view()
}

#[tauri::command]
pub async fn sign_in(app: AppHandle, hub: String, email: String, password: String, create: bool, invite: Option<String>) -> Result<View, String> {
    // The account calls block on their own HTTP client, which must not run on an async worker.
    tauri::async_runtime::spawn_blocking(move || {
        if app.state::<Account>().signed_in.read().unwrap().as_ref().is_some_and(|signed| owes_handback(&signed.standby, false)) {
            return Err(HELD_SIGN_OUT.into());
        }
        let hub = hub.trim().trim_end_matches('/').to_owned();
        let (user_token, view) = if create { account::signup_invited(&hub, &email, &password, invite.as_deref())? } else { account::login(&hub, &email, &password)? };
        let (machine, machine_token) = account::register_machine(&hub, &user_token, &computer_name(), std::env::consts::OS)?;
        let creds = Credentials { hub, email: view.email, user_token, machine_id: machine.id, machine_token };
        let state = app.state::<Account>();
        let previous = state.signed_in.write().unwrap().take();
        if let Some(previous) = previous {
            state.forget(previous);
        }
        creds.save_app(&state.dir, state.store.as_deref())?;
        if let Some(watcher) = state.watcher.lock().unwrap().take() {
            watcher.abort();
        }
        let machine = app.state::<Arc<Machine>>().inner().clone();
        let signed_in = begin(&app, &state.dir, &state.exe, creds, machine);
        *state.signed_in.write().unwrap() = Some(signed_in);
        import_when_empty(app.clone());
        Ok(state.view())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn change_password(app: AppHandle, current: String, password: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (hub, token) = app.state::<Account>().login()?;
        account::change_password(&hub, &token, &current, &password)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Shares one rated dictation with the account's hub, after the user said Save.
#[tauri::command]
pub async fn dictation_feedback(app: AppHandle, feedback: account::DictationFeedback) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (hub, token) = app.state::<Account>().login()?;
        account::dictation_feedback(&hub, &token, &feedback)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Disconnects this Mac and forgets the credentials; the app's own connectors take over again.
/// Refuses while the copy holds changes the hub has not taken, unless `anyway`, which deletes that copy.
/// The webview removes this machine from the account first, while its login still works. Run in
/// the background goes off with the login, since no web app can reach this Mac without one.
#[tauri::command]
pub async fn sign_out(app: AppHandle, anyway: Option<bool>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Account>();
        let Some(signed_in) = state.signed_in.write().unwrap().take() else { return Ok(()) };
        if owes_handback(&signed_in.standby, anyway.unwrap_or(false)) {
            *state.signed_in.write().unwrap() = Some(signed_in);
            return Err(HELD_SIGN_OUT.into());
        }
        state.forget(signed_in);
        let machine = app.state::<Arc<Machine>>().inner().clone();
        *state.watcher.lock().unwrap() = Some(watch_connectors(app.clone(), machine));
        app.state::<crate::background::Background>().save(false)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn import_when_empty(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<Account>();
        for _ in 0..60 {
            if state.signed_in.read().unwrap().as_ref().is_some_and(|s| s.link.online()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let (mut store, keys) = app.state::<Connectors>().export();
        let has_local = keys != Keys::default() || store["connectors"].as_array().is_some_and(|c| !c.is_empty());
        if !has_local {
            return;
        }
        let signed_in = state.signed_in.read().unwrap();
        let Some(SignedIn { link, creds, .. }) = signed_in.as_ref() else { return };
        // Their folders are on this Mac, so they keep running here while it's online.
        for connector in store["connectors"].as_array_mut().into_iter().flatten() {
            if connector["machine"].as_str().unwrap_or_default().is_empty() {
                connector["machine"] = creds.machine_id.clone().into();
            }
        }
        if let Err(e) = link.call_hub(HubCall::Import { store, linear_key: keys.linear, slack_key: keys.slack, slack_keys: keys.slack_bots, github_key: keys.github, vercel_key: keys.vercel }) {
            eprintln!("pinned: could not move this Mac's workflows to the account: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinned_core::connectors::{Connector, FileStorage};
    use pinned_core::machine::{refuse_real_home, Launch};
    use pinned_core::standby::Signer;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn the_pretty_hostname_is_read_from_machine_info() {
        assert_eq!(pretty_hostname("CHASSIS=laptop\nPRETTY_HOSTNAME=\"The \\\"Box\\\"\"\n").as_deref(), Some("The \"Box\""));
        assert_eq!(pretty_hostname("PRETTY_HOSTNAME=studio\n").as_deref(), Some("studio"));
        assert_eq!(pretty_hostname("PRETTY_HOSTNAME=\"\"\nCHASSIS=desktop\n"), None);
        assert_eq!(pretty_hostname("CHASSIS=desktop\n"), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-account-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn machine_in(dir: &Path) -> Arc<Machine> {
        let launch = Launch {
            exe: dir.join("pinned"),
            bins: BTreeMap::new(),
            path_env: None,
            settings: dir.join("claude-settings.json"),
            mcp_config: None,
            reminders: false,
            plugin_dir: dir.join("plugin"),
            tutorial_helper: dir.join("helper"),
            sock: dir.join("sock"),
            vector_search: false,
            first_run_wait: Duration::from_millis(1),
            starting_limit: Duration::from_secs(1),
            claude_home: Some(dir.join("claude")),
        };
        Machine::new(launch, Some(dir.join("machine")), None, |_| {})
    }

    fn workflow(instructions: &str) -> Connector {
        serde_json::from_value(json!({
            "name": "Triage",
            "enabled": true,
            "trigger": {"kind": "manual"},
            "folder": "/work",
            "model": "",
            "effort": "medium",
            "permissionMode": "acceptEdits",
            "worktree": false,
            "instructions": instructions,
            "graph": {
                "nodes": [
                    {"id": "t", "name": "Trigger", "position": [0, 0], "type": "trigger", "watch": {"kind": "manual"}},
                    {"id": "s", "name": "Do", "position": [1, 0], "type": "session"}
                ],
                "edges": [{"from": "t", "port": "out", "to": "s"}]
            }
        }))
        .unwrap()
    }

    #[test]
    fn sign_out_keeps_a_copy_that_holds_changes_until_anyway() {
        refuse_real_home();
        let root = scratch("sign-out");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let machine = machine_in(&home);
        let copy = root.join("copy");
        let exe = home.join("pinned").display().to_string();
        let connectors = Connectors::new(FileStorage::new(copy.clone()), exe.clone());
        let saved = connectors.save_connector(workflow("one")).unwrap();
        let (items, _, _) = connectors.copy_parts();
        std::fs::write(copy.join("standby.json"), serde_json::to_vec_pretty(&json!({ "copy": { "tag": "base", "at": 1 }, "base": items })).unwrap()).unwrap();
        let mut again = saved;
        again.instructions = "two".into();
        connectors.save_connector(again).unwrap();
        drop(connectors);

        let standby = Standby::open(copy.clone(), Signer { machine: machine.listed_id(), name: "MacBook".into(), login: "user-token".into() }, machine, exe);
        assert!(owes_handback(&standby, false));
        assert!(copy.join("standby.json").is_file());
        assert!(!owes_handback(&standby, true));
        standby.forget();
        assert!(!copy.exists(), "signing out with anyway deletes the copy folder");

        let empty = root.join("empty");
        let machine = machine_in(&home);
        let standby = Standby::open(empty.clone(), Signer { machine: machine.listed_id(), name: "MacBook".into(), login: "user-token".into() }, machine, home.join("pinned").display().to_string());
        assert!(!owes_handback(&standby, false));
        standby.forget();
        assert!(!empty.exists());
        assert_eq!(copy_dir(Path::new("cfg"), "m1"), PathBuf::from("cfg/account-copy/m1"));
        let _ = std::fs::remove_dir_all(root);
    }
}
