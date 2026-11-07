use std::io::Write;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

pub const TEMP_PREFIX: &str = ".pinned-tmp-";

/// Held by a sync pull and by a skill save from the check that a file is still what they last
/// saw to the write, so neither overwrites what the other wrote after that check.
pub fn hold() -> std::sync::MutexGuard<'static, ()> {
    static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    WRITES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes `bytes` to a file beside `target` and renames it over `target`, so a reader sees the
/// old file or the new one, never part of one. A symlink at `target` keeps pointing where it did,
/// and the file it points to is replaced. A hard link to `target` keeps the old bytes. The mode is
/// the old file's, or what the umask leaves of 0666 for a new one, with the owner's execute bit set
/// or cleared when `executable` says. `modified` is in milliseconds since the epoch.
pub fn replace(target: &Path, bytes: &[u8], modified: Option<u64>, executable: Option<bool>) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("could not write {}: {e}", target.display());
    let target = &through_links(target);
    let dir = target.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(failed)?;
    let previous = std::fs::metadata(target).ok().filter(|meta| meta.is_file());
    let mut temp = temp_builder().tempfile_in(dir).map_err(failed)?;
    temp.write_all(bytes).map_err(failed)?;
    set_mode(temp.as_file(), previous.as_ref(), executable).map_err(failed)?;
    if let Some(ms) = modified {
        let _ = temp.as_file().set_modified(UNIX_EPOCH + Duration::from_millis(ms));
    }
    flush(temp.as_file()).map_err(failed)?;
    persist(temp, target).map_err(failed)?;
    #[cfg(unix)]
    std::fs::File::open(dir).and_then(|dir| flush(&dir)).map_err(failed)?;
    Ok(())
}

#[cfg(unix)]
fn persist(temp: tempfile::NamedTempFile, target: &Path) -> std::io::Result<()> {
    temp.persist(target).map(drop).map_err(|e| e.error)
}

/// `std::fs::rename` renames with POSIX semantics where Windows has them, so it replaces a file
/// that a reading session holds open. tempfile's `persist` uses `MoveFileExW`, which refuses. A
/// handle opened without delete sharing still blocks it, so a few retries wait that out.
#[cfg(not(unix))]
fn persist(temp: tempfile::NamedTempFile, target: &Path) -> std::io::Result<()> {
    let temp = temp.into_temp_path();
    let mut tries = 0;
    loop {
        match std::fs::rename(&temp, target) {
            Ok(()) => return temp.keep().map(drop).map_err(|e| e.error),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && tries < 10 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

fn through_links(path: &Path) -> std::path::PathBuf {
    let mut path = path.to_path_buf();
    for _ in 0..40 {
        let Ok(next) = std::fs::read_link(&path) else { break };
        path = path.parent().unwrap_or(Path::new("")).join(next);
    }
    path
}

#[cfg(unix)]
fn temp_builder() -> tempfile::Builder<'static, 'static> {
    use std::os::unix::fs::PermissionsExt;
    let mut builder = tempfile::Builder::new();
    builder.prefix(TEMP_PREFIX).permissions(std::fs::Permissions::from_mode(0o666));
    builder
}

#[cfg(not(unix))]
fn temp_builder() -> tempfile::Builder<'static, 'static> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(TEMP_PREFIX);
    builder
}

#[cfg(unix)]
fn set_mode(file: &std::fs::File, previous: Option<&std::fs::Metadata>, executable: Option<bool>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = match previous {
        Some(meta) => meta.permissions().mode() & 0o7777,
        None => file.metadata()?.permissions().mode() & 0o7777,
    };
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

    /// Windows refuses to rename over a file another handle has open.
    #[cfg(unix)]
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

    #[cfg(windows)]
    #[test]
    fn a_file_a_reader_holds_open_is_still_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("SKILL.md");
        std::fs::write(&file, "old body").unwrap();
        let _reader = std::fs::File::open(&file).unwrap();
        replace(&file, b"new body", None, None).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new body");
        assert_eq!(names(dir.path()), ["SKILL.md"]);
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
    fn a_symlinked_file_is_written_through_and_stays_a_link() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("real")).unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();
        std::fs::write(dir.path().join("real/SKILL.md"), "old").unwrap();
        std::os::unix::fs::symlink("../real/SKILL.md", dir.path().join("skills/SKILL.md")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real/gone.md"), dir.path().join("skills/gone.md")).unwrap();
        replace(&dir.path().join("skills/SKILL.md"), b"new", None, None).unwrap();
        replace(&dir.path().join("skills/gone.md"), b"made", None, None).unwrap();
        assert!(std::fs::symlink_metadata(dir.path().join("skills/SKILL.md")).unwrap().file_type().is_symlink());
        assert!(std::fs::symlink_metadata(dir.path().join("skills/gone.md")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(dir.path().join("real/SKILL.md")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dir.path().join("real/gone.md")).unwrap(), "made");
        assert_eq!(names(&dir.path().join("skills")), ["SKILL.md", "gone.md"]);
        assert_eq!(names(&dir.path().join("real")), ["SKILL.md", "gone.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_new_file_takes_its_mode_from_the_umask() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        // The umask is process-wide, so the writes run in a child test process that sets its own.
        if let Some(dir) = std::env::var_os("PINNED_FSX_UMASK_DIR") {
            replace(&Path::new(&dir).join("new.md"), b"x", None, None).unwrap();
            replace(&Path::new(&dir).join("hook.sh"), b"x", None, Some(true)).unwrap();
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", "fsx::tests::a_new_file_takes_its_mode_from_the_umask", "--test-threads=1"]).env("PINNED_FSX_UMASK_DIR", dir.path());
        unsafe {
            child.pre_exec(|| {
                libc::umask(0o027);
                Ok(())
            });
        }
        assert!(child.output().unwrap().status.success());
        let mode = |name: &str| std::fs::metadata(dir.path().join(name)).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode("new.md"), mode("hook.sh")), (0o640, 0o740));
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
