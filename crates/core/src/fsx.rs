//! The one way pinned replaces a file another process may be reading.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

/// Prefix of the file `replace` writes beside its target. `sync::blocked` skips it, so a sync
/// cycle that runs mid-write neither pushes it nor deletes it on other machines.
pub const TEMP_PREFIX: &str = ".pinned-tmp-";

/// Writes `bytes` to a file beside `target` and renames it over `target`, so a reader sees the
/// old file or the new one, never part of one. The mode is the old file's, or 0644 for a new one,
/// with the owner's execute bit set or cleared when `executable` says. `modified` is in
/// milliseconds since the epoch.
pub fn replace(target: &Path, bytes: &[u8], modified: Option<u64>, executable: Option<bool>) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("could not write {}: {e}", target.display());
    let dir = target.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(failed)?;
    let previous = std::fs::metadata(target).ok().filter(|meta| meta.is_file());
    let mut temp = tempfile::Builder::new().prefix(TEMP_PREFIX).tempfile_in(dir).map_err(failed)?;
    temp.write_all(bytes).map_err(failed)?;
    set_mode(temp.as_file(), previous.as_ref(), executable).map_err(failed)?;
    if let Some(ms) = modified {
        let _ = temp.as_file().set_modified(UNIX_EPOCH + Duration::from_millis(ms));
    }
    flush(temp.as_file()).map_err(failed)?;
    temp.persist(target).map_err(|e| failed(e.error))?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(file: &std::fs::File, previous: Option<&std::fs::Metadata>, executable: Option<bool>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = previous.map_or(0o644, |meta| meta.permissions().mode() & 0o7777);
    let mode = match executable {
        Some(true) => mode | 0o100,
        Some(false) => mode & !0o111,
        None => mode,
    };
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(file: &std::fs::File, previous: Option<&std::fs::Metadata>, _: Option<bool>) -> std::io::Result<()> {
    match previous {
        Some(meta) => file.set_permissions(meta.permissions()),
        None => Ok(()),
    }
}

/// `fsync`, not the `F_FULLFSYNC` that `File::sync_all` asks for on macOS. `first_pull_bench`
/// writes 1000 files in about 0.4 s this way and 4.7 s with a full flush per file.
#[cfg(unix)]
fn flush(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::fsync(file.as_raw_fd()) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn flush(file: &std::fs::File) -> std::io::Result<()> {
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_replaced_file_holds_the_new_bytes_and_leaves_nothing_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("skills/notes/SKILL.md");
        replace(&file, b"first", None, None).unwrap();
        replace(&file, b"second", Some(1_700_000_000_000), None).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"second");
        assert_eq!(names(&dir.path().join("skills/notes")), ["SKILL.md"]);
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), UNIX_EPOCH + Duration::from_millis(1_700_000_000_000));
    }

    #[test]
    fn a_reader_with_the_old_file_open_keeps_reading_the_old_bytes() {
        use std::io::Read;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("SKILL.md");
        std::fs::write(&file, "old body").unwrap();
        let mut reader = std::fs::File::open(&file).unwrap();
        replace(&file, b"new body", None, None).unwrap();
        let mut seen = String::new();
        reader.read_to_string(&mut seen).unwrap();
        assert_eq!(seen, "old body");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new body");
    }

    #[cfg(unix)]
    #[test]
    fn the_mode_is_kept_or_given_its_execute_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mode = |name: &str| std::fs::metadata(dir.path().join(name)).unwrap().permissions().mode() & 0o777;
        replace(&dir.path().join("new.md"), b"x", None, None).unwrap();
        assert_eq!(mode("new.md"), 0o644);
        std::fs::write(dir.path().join("kept.sh"), "x").unwrap();
        std::fs::set_permissions(dir.path().join("kept.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
        replace(&dir.path().join("kept.sh"), b"y", None, None).unwrap();
        assert_eq!(mode("kept.sh"), 0o750);
        replace(&dir.path().join("hook.sh"), b"z", None, Some(true)).unwrap();
        assert_eq!(mode("hook.sh"), 0o744);
        replace(&dir.path().join("kept.sh"), b"y", None, Some(false)).unwrap();
        assert_eq!(mode("kept.sh"), 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_file_is_replaced_without_making_it_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hook.sh");
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        replace(&file, b"new", None, None).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o444);
    }
}
