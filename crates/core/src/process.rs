use std::ffi::OsStr;
use std::process::Command;
use std::sync::Mutex;

/// Every program pinned runs starts here. On Windows a GUI process that starts a console program,
/// such as git on every prompt, would flash a console window for each one.
#[cfg(unix)]
#[allow(clippy::disallowed_methods)]
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    for (key, value) in fixes_for_children() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command
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

/// Variables pinned set for itself alone with `set_own_env`.
static OWN_ENV: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What an AppImage's runtime sets for the app it mounts, and no program the app starts should see.
const APPIMAGE_RUNTIME: [&str; 4] = ["APPIMAGE", "APPDIR", "ARGV0", "OWD"];

/// Sets `key` in pinned's own environment only: the programs it starts don't inherit it.
/// Call it before other threads start.
pub fn set_own_env(key: &str, value: &str) {
    std::env::set_var(key, value);
    OWN_ENV.lock().unwrap().push(key.to_owned());
}

/// The changes that give a child of pinned the environment it would have outside pinned: no
/// `set_own_env` variable, and, inside an AppImage, none of the runtime's own variables and no
/// entry pointing into the mounted image, such as the bundled GTK modules or `$APPDIR/usr/share`
/// in `XDG_DATA_DIRS`, which would make the user's shell and tools load pinned's libraries.
/// `None` removes the variable.
pub fn child_env_fixes<'a>(env: impl IntoIterator<Item = (&'a str, &'a str)>, own: &[String]) -> Vec<(String, Option<String>)> {
    let env: Vec<(&str, &str)> = env.into_iter().collect();
    let mut fixes: Vec<(String, Option<String>)> = own.iter().filter(|key| env.iter().any(|(k, _)| k == key)).map(|key| (key.clone(), None)).collect();
    let appdir = env.iter().find(|(k, _)| *k == "APPDIR").map(|(_, v)| *v).filter(|dir| !dir.is_empty());
    let (true, Some(appdir)) = (env.iter().any(|(k, _)| *k == "APPIMAGE"), appdir) else { return fixes };
    for (key, value) in env {
        // The GTK hook exports GTK_THEME from an unset APPIMAGE_GTK_THEME, which would turn the
        // user's GTK programs to the default theme.
        if APPIMAGE_RUNTIME.contains(&key) || (key == "GTK_THEME" && value.is_empty()) {
            fixes.push((key.to_owned(), None));
        } else if value.contains(appdir) {
            let kept: Vec<&str> = value.split(':').filter(|entry| !entry.is_empty() && !std::path::Path::new(entry).starts_with(appdir)).collect();
            fixes.push((key.to_owned(), (!kept.is_empty()).then(|| kept.join(":"))));
        }
    }
    fixes
}

fn own_env() -> Vec<String> {
    OWN_ENV.lock().unwrap().clone()
}

/// `child_env_fixes` for pinned's own environment.
pub fn fixes_for_children() -> Vec<(String, Option<String>)> {
    let own = own_env();
    if own.is_empty() && std::env::var_os("APPIMAGE").is_none() {
        return Vec::new();
    }
    let env: Vec<(String, String)> = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    child_env_fixes(env.iter().map(|(k, v)| (k.as_str(), v.as_str())), &own)
}

/// `child_env_fixes` for a terminal or agent session, leaving what its caller set on purpose.
pub fn fix_pty_env(cmd: &mut portable_pty::CommandBuilder) {
    let own = own_env();
    if own.is_empty() && cmd.get_env("APPIMAGE").is_none() {
        return;
    }
    let explicit: Vec<String> = cmd.iter_extra_env_as_str().map(|(k, _)| k.to_owned()).collect();
    let fixes = child_env_fixes(cmd.iter_full_env_as_str(), &own);
    for (key, value) in fixes.into_iter().filter(|(key, _)| !explicit.contains(key)) {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixes(env: &[(&str, &str)], own: &[&str]) -> Vec<(String, Option<String>)> {
        let own: Vec<String> = own.iter().map(|k| k.to_string()).collect();
        child_env_fixes(env.iter().copied(), &own)
    }

    #[test]
    fn outside_an_appimage_only_pinned_own_variables_go() {
        let env = [("PATH", "/usr/bin"), ("GDK_BACKEND", "x11"), ("APPDIR", "/opt/other")];
        assert_eq!(fixes(&env, &["GDK_BACKEND", "WEBKIT_UNSET"]), [("GDK_BACKEND".to_string(), None)]);
        assert_eq!(fixes(&env, &[]), []);
    }

    #[test]
    fn inside_an_appimage_the_runtime_and_the_image_paths_go() {
        let env = [
            ("APPIMAGE", "/home/u/pinned.AppImage"),
            ("APPDIR", "/tmp/.mount_pinnedA1"),
            ("ARGV0", "./pinned.AppImage"),
            ("OWD", "/home/u"),
            ("PATH", "/tmp/.mount_pinnedA1/usr/bin:/home/u/.local/bin:/usr/bin"),
            ("XDG_DATA_DIRS", "/tmp/.mount_pinnedA1/usr/share:/usr/share:"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_pinnedA1/usr/lib"),
            ("GDK_PIXBUF_MODULE_FILE", "/tmp/.mount_pinnedA1//usr/lib/gdk-pixbuf-2.0/loaders.cache"),
            ("GTK_THEME", ""),
            ("GTK_IM_MODULE", "fcitx"),
            ("HOME", "/home/u"),
            ("NEAR", "/tmp/.mount_pinnedA1x/bin"),
        ];
        let some = |k: &str, v: &str| (k.to_string(), Some(v.to_string()));
        let none = |k: &str| (k.to_string(), None);
        assert_eq!(
            fixes(&env, &[]),
            [
                none("APPIMAGE"),
                none("APPDIR"),
                none("ARGV0"),
                none("OWD"),
                some("PATH", "/home/u/.local/bin:/usr/bin"),
                some("XDG_DATA_DIRS", "/usr/share"),
                none("LD_LIBRARY_PATH"),
                none("GDK_PIXBUF_MODULE_FILE"),
                none("GTK_THEME"),
                some("NEAR", "/tmp/.mount_pinnedA1x/bin"),
            ]
        );
    }
}
