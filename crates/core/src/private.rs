//! Files and folders only this user may read. Unix needs mode bits for that; on Windows a file
//! under the user's profile inherits an ACL that already keeps other users out.

use std::fs::OpenOptions;
use std::path::Path;

/// Options that write a file only this user may read, once the caller picks create or create_new.
pub fn file() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

/// Makes `dir` and its parents, and leaves `dir` readable by this user only even if it existed.
pub fn dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(windows)]
    std::fs::create_dir_all(dir)
}
