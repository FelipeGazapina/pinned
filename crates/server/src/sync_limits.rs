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
/// its default kept: the hub starts all the same. Empty is the same as unset.
pub fn from_env(env: &dyn Fn(&str) -> Option<String>, warn: &mut dyn FnMut(String)) -> SyncLimits {
    let mut read = |name: &str, default_mb: i64| -> Option<i64> {
        let given = env(name).filter(|v| !v.trim().is_empty())?;
        Some(megabytes(&given).unwrap_or_else(|| {
            warn(format!("{name} is not a whole number of MB above 0; the limit stays {default_mb} MB"));
            default_mb * MB
        }))
    };
    let account = read(ACCOUNT_VARIABLE, ACCOUNT_FALLBACK_MB);
    let total = read(TOTAL_VARIABLE, TOTAL_DEFAULT_MB).unwrap_or(TOTAL_DEFAULT_MB * MB);
    let db = read(DB_VARIABLE, (total >> 20).saturating_mul(2)).unwrap_or(total.saturating_mul(2));
    SyncLimits { account, total, db }
}

/// Bytes, for a whole number of MB above 0 that fits.
fn megabytes(given: &str) -> Option<i64> {
    given.trim().parse::<i64>().ok().filter(|&mb| mb >= 1)?.checked_mul(MB)
}

impl SyncLimits {
    /// The line the hub logs at start.
    pub fn line(&self) -> String {
        let account = self.account.map_or_else(|| "none".to_owned(), |bytes| format!("{} MB", bytes >> 20));
        format!("pinned-server: sync limits account={account} total={} MB db={} MB", self.total >> 20, self.db >> 20)
    }
}

/// The census line of what synced projects take, the accounts that take the most by the first 8
/// characters of their id, and a warning while the total is past 80% or 90% of its limit.
pub fn census(limits: &SyncLimits, total: i64, db: i64, top: &[(uuid::Uuid, i64)]) -> Vec<String> {
    let top: Vec<String> = top.iter().map(|(account, bytes)| format!("{}:{}", &account.simple().to_string()[..8], bytes >> 20)).collect();
    let mut lines = vec![format!(
        "pinned-server: census-sync total_mb={} limit_mb={} db_mb={} db_limit_mb={} top={}",
        total >> 20,
        limits.total >> 20,
        db >> 20,
        limits.db >> 20,
        top.join(",")
    )];
    if let Some(past) = [90, 80].into_iter().find(|percent| total as i128 * 100 > limits.total as i128 * percent) {
        lines.push(format!("pinned-server: synced projects are past {past}% of the hub's sync limit: {} of {} MB", total >> 20, limits.total >> 20));
    }
    lines
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

    #[test]
    fn the_census_names_accounts_by_a_prefix_and_warns_past_80_and_90_percent() {
        let limits = SyncLimits { account: None, total: 1000 << 20, db: 2000 << 20 };
        let ana = uuid::Uuid::parse_str("0123abcd-0000-0000-0000-000000000000").unwrap();
        assert_eq!(census(&limits, 500 << 20, 40 << 20, &[(ana, 300 << 20)]), vec!["pinned-server: census-sync total_mb=500 limit_mb=1000 db_mb=40 db_limit_mb=2000 top=0123abcd:300"]);
        assert_eq!(census(&limits, 850 << 20, 40 << 20, &[])[1], "pinned-server: synced projects are past 80% of the hub's sync limit: 850 of 1000 MB");
        assert_eq!(census(&limits, 950 << 20, 40 << 20, &[])[1], "pinned-server: synced projects are past 90% of the hub's sync limit: 950 of 1000 MB");
    }

    #[test]
    fn the_start_line_names_each_limit() {
        assert_eq!(SyncLimits::default().line(), "pinned-server: sync limits account=none total=8192 MB db=16384 MB");
        assert_eq!(SyncLimits { account: Some(1024 << 20), ..SyncLimits::default() }.line(), "pinned-server: sync limits account=1024 MB total=8192 MB db=16384 MB");
    }
}
