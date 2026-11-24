//! When the app offers to compact a session. A long conversation whose prompt cache has gone cold
//! costs its whole history again on the next turn, so from a context size the composer offers to
//! type `/compact` before the user's message. That size is a setting of each machine, which the
//! settings page keeps the same across the account and agents read and set through `act`.

use crate::surface::Parsed;
use crate::wire::Call;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One file in the machine's config folder.
pub const SETTINGS_FILE: &str = "compaction.json";

/// The offer threshold of a machine that holds no setting, in tokens.
pub const DEFAULT_OFFER_TOKENS: u32 = 200_000;

/// The smallest threshold a machine takes, so no setting makes every send compact.
pub const MIN_OFFER_TOKENS: u32 = 10_000;

/// When this machine's sessions are offered a compaction.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(rename = "CompactionSettings")]
pub struct Settings {
    /// The context size, in tokens, from which the app offers to compact a session before the
    /// user's next message. Null is never.
    #[serde(default = "default_offer_tokens")]
    pub offer_tokens: Option<u32>,
}

fn default_offer_tokens() -> Option<u32> {
    Some(DEFAULT_OFFER_TOKENS)
}

impl Default for Settings {
    fn default() -> Self {
        Settings { offer_tokens: default_offer_tokens() }
    }
}

impl Settings {
    /// Refuses a threshold under [`MIN_OFFER_TOKENS`].
    pub fn check(&self) -> Result<(), String> {
        match self.offer_tokens {
            Some(tokens) if tokens < MIN_OFFER_TOKENS => Err("offer_tokens must be at least 10000 tokens, or null for never".into()),
            _ => Ok(()),
        }
    }

    /// The saved settings, or the default when the file is missing, unreadable or refused by `check`.
    pub fn load(dir: Option<&Path>) -> Self {
        dir.and_then(|d| std::fs::read(d.join(SETTINGS_FILE)).ok()).and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok()).filter(|s| s.check().is_ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: Option<&Path>) -> Result<(), String> {
        let Some(dir) = dir else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(SETTINGS_FILE), serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

/// The argument check of an agent's `set_compaction_settings`: the bound the machine holds too.
pub fn agent_check(call: &Parsed) -> Result<(), String> {
    let Parsed::Node(Call::SetCompactionSettings { settings }) = call else { return Ok(()) };
    settings.check()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_offers_from_200k_tokens() {
        assert_eq!(Settings::default(), Settings { offer_tokens: Some(200_000) });
    }

    #[test]
    fn a_missing_unreadable_or_too_low_file_loads_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(Some(dir.path())), Settings::default(), "no file");
        assert_eq!(Settings::load(None), Settings::default(), "no config folder");
        for text in ["not json", r#"{"offer_tokens":5000}"#] {
            std::fs::write(dir.path().join(SETTINGS_FILE), text).unwrap();
            assert_eq!(Settings::load(Some(dir.path())), Settings::default(), "{text}");
        }
    }

    #[test]
    fn a_null_loads_as_never_and_a_missing_field_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), r#"{"offer_tokens":null}"#).unwrap();
        assert_eq!(Settings::load(Some(dir.path())), Settings { offer_tokens: None });
        std::fs::write(dir.path().join(SETTINGS_FILE), "{}").unwrap();
        assert_eq!(Settings::load(Some(dir.path())), Settings { offer_tokens: Some(200_000) });
    }

    #[test]
    fn saved_settings_load_back() {
        let dir = tempfile::tempdir().unwrap();
        let saved = Settings { offer_tokens: Some(50_000) };
        saved.save(Some(dir.path())).unwrap();
        assert_eq!(Settings::load(Some(dir.path())), saved);
        let never = Settings { offer_tokens: None };
        never.save(Some(dir.path())).unwrap();
        assert_eq!(Settings::load(Some(dir.path())), never);
        assert_eq!(saved.save(None), Ok(()), "no config folder saves nothing");
    }

    #[test]
    fn check_refuses_under_10000_tokens_and_takes_never() {
        assert_eq!(Settings { offer_tokens: Some(9_999) }.check(), Err("offer_tokens must be at least 10000 tokens, or null for never".into()));
        assert_eq!(Settings { offer_tokens: Some(10_000) }.check(), Ok(()));
        assert_eq!(Settings { offer_tokens: None }.check(), Ok(()));
    }
}
