//! This Mac's place in an account: signing in and out, the link to the hub while signed in, and
//! the app's own connector watcher, which runs only while signed out.

use pinned_core::account::{self, Credentials};
use pinned_core::connectors::{self, Connectors, Settings};
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

/// What the webview needs to open its own client socket to the hub.
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
    /// Signed in when `dir` holds credentials: the link starts and the local watcher doesn't.
    pub fn start(app: &AppHandle, dir: PathBuf, machine: Arc<Machine>) -> Account {
        let signed_in = Credentials::load(&dir).map(|creds| SignedIn { link: connect(&creds, machine.clone()), creds });
        let watcher = signed_in.is_none().then(|| watch_connectors(app.clone(), machine));
        Account { dir, signed_in: RwLock::new(signed_in), watcher: Mutex::new(watcher) }
    }

    pub fn is_signed_in(&self) -> bool {
        self.signed_in.read().unwrap().is_some()
    }

    /// Sends a connector session's callback to the hub, which holds the keys, while signed in.
    /// Signed out, it comes back for the app's own connectors to answer.
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

/// Runs the app's own connectors, handing their runs to this Mac.
fn watch_connectors(app: AppHandle, machine: Arc<Machine>) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let open = |work| {
            let taken = match work {
                connectors::Work::Run(run) => machine.run(&run.spec()).is_ok(),
                connectors::Work::Delivery(d) => machine.deliver(&d.env(), &d.text()),
            };
            async move { taken }
        };
        let checked = |check: &connectors::Check| drop(app.emit("connectors-checked", check));
        app.state::<Connectors>().watch(open, checked).await
    })
}

/// The name the Mac shows in Finder's sidebar, which names it in the account.
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

/// Signs in, or creates the account with `create`, registers this Mac as one of its machines and
/// connects it. The account's hub then runs the connectors instead of this app.
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

/// Moves this app's connectors and keys into the account once the link is up, if the account
/// has none yet. An account that already has some keeps them, and this app's stay on disk.
fn import_when_empty(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<Account>();
        for _ in 0..60 {
            if state.signed_in.read().unwrap().as_ref().is_some_and(|s| s.link.online()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let (store, linear_key, slack_key) = app.state::<Connectors>().export();
        let has_local = linear_key.is_some() || slack_key.is_some() || store["connectors"].as_array().is_some_and(|c| !c.is_empty());
        if !has_local {
            return;
        }
        let signed_in = state.signed_in.read().unwrap();
        let Some(SignedIn { link, .. }) = signed_in.as_ref() else { return };
        let result = link.call_hub(HubCall::GetSettings).and_then(|current| {
            let current: Settings = serde_json::from_value(current).map_err(|e| e.to_string())?;
            if current.linear.is_some() || current.slack.is_some() || !current.connectors.is_empty() {
                return Ok(());
            }
            link.call_hub(HubCall::Import { store, linear_key, slack_key }).map(drop)
        });
        if let Err(e) = result {
            eprintln!("pinned: could not move this Mac's connectors to the account: {e}");
        }
    });
}
