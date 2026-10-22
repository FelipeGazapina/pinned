//! The calls block, so make them off any async runtime's workers (reqwest's blocking client
//! panics inside one). The binary installs rustls's ring provider before the first call.

use crate::wire::{AccountView, MachineId, MachineView};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

const FILE: &str = "account.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Credentials {
    pub hub: String,
    pub email: String,
    pub user_token: String,
    pub machine_id: MachineId,
    pub machine_token: String,
}

impl Credentials {
    pub fn load(dir: &Path) -> Option<Credentials> {
        serde_json::from_slice(&std::fs::read(dir.join(FILE)).ok()?).ok()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("{FILE}.tmp"));
        let _ = std::fs::remove_file(&tmp);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut f| f.write_all(&serde_json::to_vec_pretty(self).unwrap()))
            .and_then(|()| std::fs::rename(&tmp, dir.join(FILE)))
            .map_err(|e| format!("could not save {}: {e}", dir.join(FILE).display()))
    }

    pub fn remove(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(FILE));
    }
}

/// `POST /api/signup` and `/api/login` take this.
#[derive(Serialize, Deserialize)]
pub struct SignIn {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub account: AccountView,
}

/// `POST /api/password`, with the user token as bearer, takes this.
#[derive(Serialize, Deserialize)]
pub struct NewPassword {
    pub current: String,
    pub password: String,
}

/// `POST /api/machines`, with the user token as bearer, takes this.
#[derive(Serialize, Deserialize)]
pub struct NewMachine {
    pub name: String,
    pub os: String,
}

#[derive(Serialize, Deserialize)]
pub struct Registered {
    pub machine: MachineView,
    pub token: String,
}

/// `POST /api/pair` takes this.
#[derive(Serialize, Deserialize)]
pub struct Pairing {
    pub code: String,
    pub name: String,
    pub os: String,
}

#[derive(Serialize, Deserialize)]
pub struct Paired {
    pub machine: MachineView,
    pub token: String,
    /// The hub's public URL, which the machine links to from then on.
    pub hub: String,
    pub email: String,
}

#[derive(Serialize, Deserialize)]
pub struct Failure {
    pub error: String,
}

pub fn signup(hub: &str, email: &str, password: &str) -> Result<(String, AccountView), String> {
    let session: Session = post(hub, "/api/signup", None, &SignIn { email: email.into(), password: password.into() })?;
    Ok((session.token, session.account))
}

pub fn login(hub: &str, email: &str, password: &str) -> Result<(String, AccountView), String> {
    let session: Session = post(hub, "/api/login", None, &SignIn { email: email.into(), password: password.into() })?;
    Ok((session.token, session.account))
}

pub fn change_password(hub: &str, user_token: &str, current: &str, password: &str) -> Result<(), String> {
    post::<AccountView>(hub, "/api/password", Some(user_token), &NewPassword { current: current.into(), password: password.into() }).map(drop)
}

pub fn register_machine(hub: &str, user_token: &str, name: &str, os: &str) -> Result<(MachineView, String), String> {
    let registered: Registered = post(hub, "/api/machines", Some(user_token), &NewMachine { name: name.into(), os: os.into() })?;
    Ok((registered.machine, registered.token))
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
    let out = std::process::Command::new("hostname").stderr(std::process::Stdio::null()).output();
    let name = out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    let name = name.strip_suffix(".local").unwrap_or(&name);
    if name.is_empty() {
        "machine".into()
    } else {
        name.to_owned()
    }
}

fn post<T: DeserializeOwned>(hub: &str, path: &str, bearer: Option<&str>, body: &impl Serialize) -> Result<T, String> {
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
    serde_json::from_str(&text).map_err(|e| format!("{hub} answered something pinned can't read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
        assert_eq!(std::fs::metadata(dir.join("account.json")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(Credentials::load(&dir), Some(creds));
        Credentials::remove(&dir);
        assert_eq!(Credentials::load(&dir), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
