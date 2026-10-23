use std::ffi::OsStr;
use std::process::Command;

/// Every program pinned runs starts here. On Windows a GUI process that starts a console program,
/// such as git on every prompt, would flash a console window for each one.
#[cfg(unix)]
#[allow(clippy::disallowed_methods)]
pub fn command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

#[cfg(windows)]
#[allow(clippy::disallowed_methods)]
pub fn command(program: impl AsRef<OsStr>) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}
