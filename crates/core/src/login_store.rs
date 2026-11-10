//! The OS credential store that keeps the desktop app's hub login out of `account.json`: the
//! macOS Keychain and the Windows Credential Manager. Other systems have no store a headless node
//! can count on, so a login there stays in the file, and a node keeps none (#121).
//!
//! macOS keeps the login as a generic password in the login keychain, created by the app with the
//! default access list. The app that made the item reads it without a prompt, and any other
//! program reads it only after the user types their macOS password into a system prompt. The app
//! is ad-hoc signed, so that list names this exact build by its cdhash: after each update the new
//! build asks once too, and "Always Allow" adds it.
//!
//! Once the app is signed with a Developer ID (docs/release/developer-id.md):
//! - An item the signed app creates names it by its designated requirement, the bundle id plus
//!   the team, so updates read it with no prompt. An item an ad-hoc build made still names that
//!   build, so the first signed version deletes it and sets it again.
//! - With a Developer ID provisioning profile that grants `keychain-access-groups` =
//!   `<TEAMID>.dev.personal-harness.pinned`, the login moves to the data protection keychain
//!   (`apple_native_keyring_store::protected::Cred` with that access group). No other program can
//!   read an item there, prompt or not. An ad-hoc build cannot carry that entitlement: macOS
//!   refuses to launch an app whose restricted entitlements no profile vouches for.
//!
//! The Windows Credential Manager has no per-program access list. Its entry keeps the login out of
//! a plain file that backups, sync and searches read, but any program of the same user can read it.

/// Where the app keeps its hub login, one entry per hub account.
pub trait LoginStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, login: &str) -> Result<(), String>;
    /// Succeeds when there was nothing to delete.
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The Keychain item's service, and the second half of the Credential Manager entry's name.
pub const SERVICE: &str = "dev.personal-harness.pinned.hub-login";

/// This system's credential store, or None where it has none and `account.json` keeps the login.
pub fn system() -> Option<Box<dyn LoginStore>> {
    #[cfg(any(target_os = "macos", windows))]
    return Some(Box::new(Os { service: SERVICE }));
    #[cfg(not(any(target_os = "macos", windows)))]
    None
}

#[cfg(any(target_os = "macos", windows))]
pub struct Os {
    service: &'static str,
}

#[cfg(any(target_os = "macos", windows))]
impl Os {
    fn entry(&self, account: &str) -> Result<keyring_core::Entry, String> {
        #[cfg(target_os = "macos")]
        let entry = {
            use apple_native_keyring_store::keychain::{Cred, MacKeychainDomain};
            Cred::build(MacKeychainDomain::User, self.service, account)
        };
        // The crate's default, Enterprise, roams with a domain profile; the login stays on this PC.
        #[cfg(windows)]
        let entry = {
            use keyring_core::api::CredentialStoreApi;
            let local = std::collections::HashMap::from([("persistence", "Local")]);
            windows_native_keyring_store::Store::new().and_then(|store| store.build(self.service, account, Some(&local)))
        };
        entry.map_err(|e| e.to_string())
    }
}

#[cfg(any(target_os = "macos", windows))]
impl LoginStore for Os {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        match self.entry(account)?.get_password() {
            Ok(login) => Ok(Some(login)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, account: &str, login: &str) -> Result<(), String> {
        self.entry(account)?.set_password(login).map_err(|e| e.to_string())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// What every store does, which the fake and the real Keychain both pass.
#[cfg(test)]
pub(crate) fn assert_contract(store: &dyn LoginStore, account: &str) {
    let other = format!("{account}-other");
    assert_eq!(store.delete(account), Ok(()), "deleting nothing succeeds");
    assert_eq!(store.get(account), Ok(None));
    store.set(account, "u-first").unwrap();
    assert_eq!(store.get(account), Ok(Some("u-first".into())));
    store.set(account, "u-second").unwrap();
    assert_eq!(store.get(account), Ok(Some("u-second".into())), "a set replaces the login");
    assert_eq!(store.get(&other), Ok(None), "each account has its own entry");
    store.delete(account).unwrap();
    assert_eq!(store.get(account), Ok(None));
    assert_eq!(store.delete(account), Ok(()));
}

#[cfg(test)]
pub(crate) mod fake {
    use super::LoginStore;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default, Clone, Copy)]
    pub enum Writes {
        #[default]
        Kept,
        /// The way a locked keychain, or a user who clicks Deny, answers.
        Refused,
        /// Accepted and then not there, which only reading back catches.
        Lost,
    }

    #[derive(Default)]
    pub struct Memory {
        logins: Mutex<BTreeMap<String, String>>,
        writes: Writes,
    }

    impl Memory {
        pub fn with(writes: Writes) -> Memory {
            Memory { writes, ..Memory::default() }
        }

        pub fn logins(&self) -> Vec<(String, String)> {
            self.logins.lock().unwrap().clone().into_iter().collect()
        }
    }

    impl LoginStore for Memory {
        fn get(&self, account: &str) -> Result<Option<String>, String> {
            Ok(self.logins.lock().unwrap().get(account).cloned())
        }

        fn set(&self, account: &str, login: &str) -> Result<(), String> {
            match self.writes {
                Writes::Kept => drop(self.logins.lock().unwrap().insert(account.into(), login.into())),
                Writes::Refused => return Err("User interaction is not allowed.".into()),
                Writes::Lost => {}
            }
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), String> {
            self.logins.lock().unwrap().remove(account);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_store_keeps_one_login_per_account() {
        assert_contract(&fake::Memory::default(), "https://pinned.example.com ana@acme.dev");
    }

    // The Keychain tests write and delete items under a test service, never the app's. They need
    // the login keychain unlocked, which an ssh session's is not, so run them from the GUI session.
    #[cfg(target_os = "macos")]
    const TEST_SERVICE: &str = "dev.personal-harness.pinned.hub-login-test";

    #[cfg(target_os = "macos")]
    fn test_login(name: &str) -> crate::account::Credentials {
        crate::account::Credentials {
            hub: "https://pinned.example.com".into(),
            email: format!("{name}-{}@acme.dev", std::process::id()),
            user_token: format!("u-{name}-{}", std::process::id()),
            machine_id: "8c0f1f4e-5b8a-4c4e-9a59-0d6b4f0f2a11".into(),
            machine_token: "m-t0k".into(),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn the_keychain_keeps_one_login_per_account() {
        let store = Os { service: TEST_SERVICE };
        let account = test_login("contract").login_account();
        assert_contract(&store, &account);
        println!("keychain: contract held for {TEST_SERVICE} / {account}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn an_older_login_moves_from_account_json_into_the_keychain() {
        use crate::account::Credentials;
        let store = Os { service: TEST_SERVICE };
        let dir = tempfile::tempdir().unwrap();
        let ana = test_login("migrate");
        ana.save(dir.path()).unwrap();

        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(ana.clone()));
        assert_eq!(Credentials::load(dir.path()).unwrap().user_token, "", "the file lost the login");
        assert_eq!(store.get(&ana.login_account()), Ok(Some(ana.user_token.clone())));
        println!("keychain: moved the login out of account.json into {TEST_SERVICE} / {}", ana.login_account());
        assert_eq!(Credentials::load_app(dir.path(), Some(&store)), Some(ana.clone()), "a second start reads it back");

        ana.remove_app(dir.path(), Some(&store));
        assert_eq!(store.get(&ana.login_account()), Ok(None), "signing out deletes the item");
        println!("keychain: signing out deleted the item");
    }
}
