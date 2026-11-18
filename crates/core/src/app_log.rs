//! The desktop app's log: its stderr, kept in its data folder, since the stderr of an app opened from
//! the Dock or at login goes to /dev/null and the link's messages were lost there.

use std::path::{Path, PathBuf};

/// A log past this size is moved to `<log>.old` when the app starts, replacing the one there.
pub const LIMIT: u64 = 10 * 1024 * 1024;

/// Moves `log` to `<log>.old` when it holds more than `limit` bytes.
pub fn rotate(_log: &Path, _limit: u64) -> std::io::Result<()> {
    Ok(())
}

fn old(log: &Path) -> PathBuf {
    let mut name = log.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::{old, rotate, LIMIT};

    #[test]
    fn a_log_over_ten_megabytes_is_moved_to_old_replacing_the_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("pinned.log");
        std::fs::write(old(&log), "the previous run").unwrap();
        std::fs::write(&log, vec![b'x'; 11 * 1024 * 1024]).unwrap();
        rotate(&log, LIMIT).unwrap();
        assert_eq!((log.exists(), std::fs::metadata(old(&log)).unwrap().len()), (false, 11534336));
    }

    #[test]
    fn a_log_under_ten_megabytes_stays() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("pinned.log");
        std::fs::write(&log, vec![b'x'; 1024]).unwrap();
        rotate(&log, LIMIT).unwrap();
        assert_eq!((std::fs::metadata(&log).unwrap().len(), old(&log).exists()), (1024, false));
    }
}
