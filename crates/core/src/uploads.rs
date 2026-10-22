//! Files a client attaches to a session, such as a photo taken on a phone, saved on the session's
//! machine so claude can read them by path.

use crate::sync;
use std::fs::{DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The biggest file a client may attach.
pub const LIMIT: usize = 25 << 20;
/// A session has long since read an upload this old, or never will.
const KEEP: Duration = Duration::from_secs(24 * 3600);
const STEM_MAX: usize = 100;

// An upload travels whole, in base64, in one websocket message.
const _: () = assert!(LIMIT.div_ceil(3) * 4 + (1 << 20) <= sync::MESSAGE_LIMIT);

/// Saves `data` in `dir` under a safe form of `name`, never over an earlier upload, and returns its path.
pub fn save(dir: &Path, name: &str, data: &[u8]) -> Result<PathBuf, String> {
    if data.len() > LIMIT {
        return Err(too_big(name));
    }
    private_dir(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    sweep(dir);
    let (stem, ext) = file_name(name);
    let mut n = 1;
    loop {
        let path = dir.join(if n == 1 { format!("{stem}{ext}") } else { format!("{stem}-{n}{ext}") });
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(mut file) => {
                return file.write_all(data).map(|()| path.clone()).map_err(|e| {
                    let _ = std::fs::remove_file(&path);
                    format!("could not save {name}: {e}")
                })
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(format!("could not save {name}: {e}")),
        }
    }
}

pub fn too_big(name: &str) -> String {
    format!("{name} is over {} MB, the largest file pinned attaches", LIMIT >> 20)
}

/// Deletes the uploads older than `KEEP`.
pub fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let modified = entry.metadata().and_then(|m| m.modified());
        if modified.is_ok_and(|t| now.duration_since(t).unwrap_or_default() > KEEP) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// The stem and extension an upload named `name` is saved under: its last path component, with
/// anything but ASCII letters, digits, `.`, `-` and `_` made `_`, and no leading dot.
fn file_name(name: &str) -> (String, String) {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let safe: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    let safe = safe.trim_start_matches('.');
    let (stem, ext) = match safe.rfind('.') {
        Some(dot) if safe.len() - dot <= 10 => safe.split_at(dot),
        _ => (safe, ""),
    };
    let stem = &stem[..stem.len().min(STEM_MAX)];
    (if stem.is_empty() { "file".into() } else { stem.into() }, ext.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-uploads-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_name_keeps_only_its_last_component_in_safe_characters() {
        assert_eq!(file_name("IMG_2041.HEIC"), ("IMG_2041".into(), ".HEIC".into()));
        assert_eq!(file_name("../../etc/My Photo (1).png"), ("My_Photo__1_".into(), ".png".into()));
        assert_eq!(file_name("C:\\Users\\ana\\captura de tela.jpg"), ("captura_de_tela".into(), ".jpg".into()));
        assert_eq!(file_name("..."), ("file".into(), "".into()));
        assert_eq!(file_name(".bashrc"), ("bashrc".into(), "".into()));
        assert_eq!(file_name("relatório.pdf"), ("relat_rio".into(), ".pdf".into()));
        assert_eq!(file_name("notes.not-an-extension"), ("notes.not-an-extension".into(), "".into()));
        assert_eq!(file_name(&format!("{}.png", "a".repeat(300))), ("a".repeat(100), ".png".into()));
    }

    #[test]
    fn uploads_land_privately_under_their_own_names_and_never_replace_each_other() {
        let dir = temp("save");
        let first = save(&dir, "../photo.png", b"first").unwrap();
        let second = save(&dir, "photo.png", b"second").unwrap();
        assert_eq!((first.clone(), second.clone()), (dir.join("photo.png"), dir.join("photo-2.png")));
        assert_eq!((std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap()), (b"first".to_vec(), b"second".to_vec()));
        assert_eq!((mode(&dir), mode(&first)), (0o700, 0o600));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_past_the_limit_is_refused_and_leaves_nothing() {
        let dir = temp("limit");
        let big = vec![7u8; LIMIT + 1];
        assert_eq!(save(&dir, "clip.mov", &big), Err("clip.mov is over 25 MB, the largest file pinned attaches".into()));
        assert!(!dir.exists());
        assert_eq!(save(&dir, "clip.mov", &big[..LIMIT]), Ok(dir.join("clip.mov")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_sweep_deletes_only_uploads_older_than_a_day() {
        let dir = temp("sweep");
        let old = save(&dir, "old.png", b"x").unwrap();
        let new = save(&dir, "new.png", b"y").unwrap();
        let file = OpenOptions::new().write(true).open(&old).unwrap();
        file.set_modified(SystemTime::now() - KEEP - Duration::from_secs(60)).unwrap();
        sweep(&dir);
        assert_eq!((old.exists(), new.exists()), (false, true));
        let _ = std::fs::remove_dir_all(dir);
    }
}
