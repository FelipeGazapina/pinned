use pinned_core::account::{self, Credentials};
use pinned_core::connectors::{self, Connectors, Keys};
use pinned_core::link::{self, Link};
use pinned_core::machine::Machine;
use pinned_core::wire::HubCall;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};

pub struct Account {
    dir: PathBuf,
    /// A connector callback holds a read lock while the hub answers, so signing out waits for it.
    signed_in: RwLock<Option<SignedIn>>,
    watcher: Mutex<Option<JoinHandle<()>>>,
}

struct SignedIn {
    creds: Credentials,
    link: Link,
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
    pub fn start(app: &AppHandle, dir: PathBuf, machine: Arc<Machine>) -> Account {
        let signed_in = Credentials::load(&dir).map(|creds| SignedIn { link: connect(&creds, machine.clone()), creds });
        let watcher = signed_in.is_none().then(|| watch_connectors(app.clone(), machine));
        Account { dir, signed_in: RwLock::new(signed_in), watcher: Mutex::new(watcher) }
    }

    pub fn is_signed_in(&self) -> bool {
        self.signed_in.read().unwrap().is_some()
    }

    pub fn forward(&self, message: pinned_core::agent::Message) -> Result<Result<String, String>, pinned_core::agent::Message> {
        match self.signed_in.read().unwrap().as_ref() {
            Some(signed_in) => Ok(signed_in.link.forward(message)),
            None => Err(message),
        }
    }

    fn view(&self) -> View {
        let Some(SignedIn { creds, link }) = &*self.signed_in.read().unwrap() else { return View::default() };
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

fn connect(creds: &Credentials, machine: Arc<Machine>) -> Link {
    link::spawn(creds.clone(), machine, env!("CARGO_PKG_VERSION"))
}

/// The machine id runs record while this signed-out Mac runs its own workflows.
const LOCAL: &str = "this-mac";

fn watch_connectors(app: AppHandle, machine: Arc<Machine>) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        // This Mac is the only machine, so a run resumes here, where its transcript already is.
        let open = |work: connectors::Work| {
            let taken = match &work {
                connectors::Work::Run(run) => machine.run(&run.resume_spec(None).unwrap_or_else(|| run.spec())).is_ok(),
                connectors::Work::Delivery(d) => machine.deliver(&d.env(), &d.text()),
                connectors::Work::Prompt { text, .. } => machine.deliver(&work.env(), text),
                connectors::Work::Close { tab, .. } => machine.close_tab(tab),
            };
            async move { taken.then(|| LOCAL.to_owned()) }
        };
        let online = || vec![(LOCAL.to_owned(), machine.tabs())];
        let checked = |check: &connectors::Check| drop(app.emit("connectors-checked", check));
        app.state::<Connectors>().watch(open, online, checked).await
    })
}

fn computer_name() -> String {
    std::process::Command::new("scutil")
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
pub async fn sign_in(app: AppHandle, hub: String, email: String, password: String, create: bool) -> Result<View, String> {
    // The account calls block on their own HTTP client, which must not run on an async worker.
    tauri::async_runtime::spawn_blocking(move || {
        let hub = hub.trim().trim_end_matches('/').to_owned();
        let (user_token, view) = if create { account::signup(&hub, &email, &password)? } else { account::login(&hub, &email, &password)? };
        let (machine, machine_token) = account::register_machine(&hub, &user_token, &computer_name(), std::env::consts::OS)?;
        let creds = Credentials { hub, email: view.email, user_token, machine_id: machine.id, machine_token };
        let state = app.state::<Account>();
        creds.save(&state.dir)?;
        if let Some(watcher) = state.watcher.lock().unwrap().take() {
            watcher.abort();
        }
        let link = connect(&creds, app.state::<Arc<Machine>>().inner().clone());
        *state.signed_in.write().unwrap() = Some(SignedIn { creds, link });
        import_when_empty(app.clone());
        Ok(state.view())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn change_password(app: AppHandle, current: String, password: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Account>();
        let (hub, token) = state.signed_in.read().unwrap().as_ref().map(|s| (s.creds.hub.clone(), s.creds.user_token.clone())).ok_or("sign in first")?;
        account::change_password(&hub, &token, &current, &password)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Disconnects this Mac and forgets the credentials; the app's own connectors take over again.
/// The webview removes this machine from the account first, while its login still works.
#[tauri::command]
pub async fn sign_out(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Account>();
        let Some(SignedIn { creds, link }) = state.signed_in.write().unwrap().take() else { return Ok(()) };
        link.stop();
        account::Credentials::remove(&state.dir);
        if let Err(e) = account::logout(&creds.hub, &creds.user_token) {
            eprintln!("pinned: sign out: {e}");
        }
        let machine = app.state::<Arc<Machine>>().inner().clone();
        *state.watcher.lock().unwrap() = Some(watch_connectors(app.clone(), machine));
        Ok(())
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
        let Some(SignedIn { link, creds }) = signed_in.as_ref() else { return };
        // Their folders are on this Mac, so they keep running here while it's online.
        for connector in store["connectors"].as_array_mut().into_iter().flatten() {
            if connector["machine"].as_str().unwrap_or_default().is_empty() {
                connector["machine"] = creds.machine_id.clone().into();
            }
        }
        if let Err(e) = link.call_hub(HubCall::Import { store, linear_key: keys.linear, slack_key: keys.slack, github_key: keys.github }) {
            eprintln!("pinned: could not move this Mac's workflows to the account: {e}");
        }
    });
}
