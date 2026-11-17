//! How much the hub keeps of synced projects: per account, if set, and for the whole hub, always.
//! Read once from the environment at start (`from_env`).

pub const ACCOUNT_VARIABLE: &str = "PINNED_SYNC_ACCOUNT_LIMIT_MB";
pub const TOTAL_VARIABLE: &str = "PINNED_SYNC_TOTAL_LIMIT_MB";
pub const DB_VARIABLE: &str = "PINNED_SYNC_DB_LIMIT_MB";

const MB: i64 = 1 << 20;
/// What an account limit that is set but not a number falls back to: the limit before 0.18.7.
const ACCOUNT_FALLBACK_MB: i64 = 1024;
const TOTAL_DEFAULT_MB: i64 = 8192;

/// Bytes, as `octet_length` counts the stored files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncLimits {
    /// What one account's synced projects may take; none unless `PINNED_SYNC_ACCOUNT_LIMIT_MB` is set.
    pub account: Option<i64>,
    /// What every account's synced projects may take together. It cannot be turned off.
    pub total: i64,
    /// Past this size of the whole database, the hub takes no push that grows.
    pub db: i64,
}

impl Default for SyncLimits {
    fn default() -> SyncLimits {
        SyncLimits { account: None, total: TOTAL_DEFAULT_MB * MB, db: 2 * TOTAL_DEFAULT_MB * MB }
    }
}

/// Reads the limits. A value that is not a whole number of MB above 0 is warned about once and
/// its default kept: the hub starts all the same.
pub fn from_env(_env: &dyn Fn(&str) -> Option<String>, _warn: &mut dyn FnMut(String)) -> SyncLimits {
    SyncLimits { account: Some(ACCOUNT_FALLBACK_MB * MB), ..SyncLimits::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn read(vars: &[(&str, &str)]) -> (SyncLimits, Vec<String>) {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut warnings = vec![];
        let limits = from_env(&|name| vars.get(name).cloned(), &mut |w| warnings.push(w));
        (limits, warnings)
    }

    #[test]
    fn with_nothing_set_no_account_limit_and_a_total_of_8192_mb() {
        assert_eq!(read(&[]), (SyncLimits { account: None, total: 8192 << 20, db: 16384 << 20 }, vec![]));
    }

    #[test]
    fn an_account_limit_is_a_whole_number_of_mb() {
        assert_eq!(read(&[(ACCOUNT_VARIABLE, "2048")]).0.account, Some(2048 << 20));
        assert_eq!(read(&[(ACCOUNT_VARIABLE, " 512 ")]), (SyncLimits { account: Some(512 << 20), ..SyncLimits::default() }, vec![]));
        assert_eq!(read(&[(ACCOUNT_VARIABLE, "")]), (SyncLimits::default(), vec![]));
        assert_eq!(read(&[(ACCOUNT_VARIABLE, "  ")]), (SyncLimits::default(), vec![]));
    }

    #[test]
    fn an_account_limit_that_is_not_a_number_of_mb_stays_1024_mb_with_a_warning() {
        for given in ["0", "-1", "1GB", "1.5", "99999999999999"] {
            assert_eq!(
                read(&[(ACCOUNT_VARIABLE, given)]),
                (
                    SyncLimits { account: Some(1024 << 20), ..SyncLimits::default() },
                    vec!["PINNED_SYNC_ACCOUNT_LIMIT_MB is not a whole number of MB above 0; the limit stays 1024 MB".to_owned()]
                ),
                "{given}"
            );
        }
    }

    #[test]
    fn a_total_that_is_not_a_number_stays_8192_mb_with_a_warning() {
        assert_eq!(
            read(&[(TOTAL_VARIABLE, "x")]),
            (SyncLimits::default(), vec!["PINNED_SYNC_TOTAL_LIMIT_MB is not a whole number of MB above 0; the limit stays 8192 MB".to_owned()])
        );
    }

    #[test]
    fn the_database_limit_is_twice_the_total_unless_set() {
        assert_eq!(read(&[(TOTAL_VARIABLE, "1000")]), (SyncLimits { account: None, total: 1000 << 20, db: 2000 << 20 }, vec![]));
        assert_eq!(read(&[(TOTAL_VARIABLE, "1000"), (DB_VARIABLE, "1500")]).0.db, 1500 << 20);
        assert_eq!(
            read(&[(TOTAL_VARIABLE, "1000"), (DB_VARIABLE, "lots")]),
            (
                SyncLimits { account: None, total: 1000 << 20, db: 2000 << 20 },
                vec!["PINNED_SYNC_DB_LIMIT_MB is not a whole number of MB above 0; the limit stays 2000 MB".to_owned()]
            )
        );
    }
}
