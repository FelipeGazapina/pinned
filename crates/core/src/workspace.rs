use crate::history;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

const DIFF_LIMIT: usize = 400_000;
const PR_FIELDS: &str = "number,title,state,isDraft,url,body,author,createdAt,updatedAt,mergedAt,closedAt,headRefName,baseRefName,headRefOid,baseRefOid,additions,deletions,changedFiles,reviewDecision,mergeable,statusCheckRollup,reviews,comments,commits,labels,reviewRequests,autoMergeRequest";
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Serialize, Debug, PartialEq)]
pub struct Workspace {
    /// The folder the project's history is kept under, which a worktree shares with its repository.
    pub project: String,
    /// The folder is inside a git repository.
    pub repo: bool,
    pub branch: Option<String>,
    pub worktree: bool,
}

pub fn of(cwd: &Path) -> Workspace {
    let git = |args: &[&str]| run(cwd, None, "git", args).ok().map(|out| out.trim().to_owned()).filter(|out| !out.is_empty());
    // symbolic-ref names the branch of a repository with no commit yet too; a detached HEAD has none.
    let branch = git(&["symbolic-ref", "--short", "-q", "HEAD"]);
    let dir = git(&["rev-parse", "--git-dir"]);
    let worktree = matches!((&dir, git(&["rev-parse", "--git-common-dir"])), (Some(dir), Some(common)) if *dir != common);
    Workspace { project: history::project_of(cwd).display().to_string(), repo: dir.is_some(), branch, worktree }
}

/// What `gh` knows of the pull request `pr`, a number or URL, with its diff under `diff`. `path` is
/// the PATH to find gh on.
pub fn pull_request(cwd: &Path, pr: &str, path: Option<&str>) -> Result<Value, String> {
    check_pr(pr)?;
    let view = gh(cwd, path, &["pr", "view", "--json", PR_FIELDS, "--", pr])?;
    let mut view: Value = serde_json::from_str(&view).map_err(|e| format!("gh sent a bad answer: {e}"))?;
    let mut diff = gh(cwd, path, &["pr", "diff", "--", pr])?;
    if clip(&mut diff) {
        view["truncated"] = true.into();
    }
    view["diff"] = diff.into();
    if let Value::Object(extras) = crate::panel::pr_extras(cwd, pr, &view, path) {
        for (key, value) in extras {
            view[key] = value;
        }
    }
    Ok(view)
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct PrStatus {
    pub state: PrState,
    pub title: String,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
}

pub fn pr_status(cwd: &Path, pr: &str, path: Option<&str>) -> Result<PrStatus, String> {
    check_pr(pr)?;
    parse_pr_status(&gh(cwd, path, &["pr", "view", "--json", "state,isDraft,title,mergedAt,closedAt", "--", pr])?)
}

fn parse_pr_status(view: &str) -> Result<PrStatus, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct View {
        state: String,
        #[serde(default)]
        is_draft: bool,
        #[serde(default)]
        title: String,
        merged_at: Option<String>,
        closed_at: Option<String>,
    }
    let view: View = serde_json::from_str(view).map_err(|e| format!("gh sent a bad answer: {e}"))?;
    let state = match view.state.as_str() {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ if view.is_draft => PrState::Draft,
        _ => PrState::Open,
    };
    Ok(PrStatus { state, title: view.title, merged_at: view.merged_at, closed_at: view.closed_at })
}

/// Checks out the pull request `pr`, a number or URL, in `cwd` with gh.
pub fn checkout_pr(cwd: &Path, pr: &str, path: Option<&str>) -> Result<(), String> {
    check_pr(pr)?;
    gh(cwd, path, &["pr", "checkout", "--", pr]).map(|_| ())
}

/// The repository's pull requests, open ones first and then the newest, for the composer's `#` menu.
pub fn pull_requests(cwd: &Path, path: Option<&str>) -> Result<Value, String> {
    let list = gh(cwd, path, &["pr", "list", "--state", "all", "--limit", "50", "--json", "number,title,state,isDraft,url,headRefName"])?;
    let mut list: Vec<Value> = serde_json::from_str(&list).map_err(|e| format!("gh sent a bad answer: {e}"))?;
    list.sort_by_key(|pr| (pr["state"] != "OPEN", std::cmp::Reverse(pr["number"].as_u64())));
    Ok(Value::Array(list))
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PrAction {
    Merge,
    Squash,
    Rebase,
    /// Marks a draft ready for review.
    Ready,
    /// Turns an open pull request back into a draft.
    Draft,
    Close,
    Reopen,
    /// Opens a pull request that undoes this merged one.
    Revert,
    /// Merges once the required checks and reviews pass, by the method named.
    AutoMerge,
    AutoSquash,
    AutoRebase,
    DisableAuto,
    /// Deletes a merged pull request's branch on GitHub.
    DeleteBranch,
}

pub fn pr_action(cwd: &Path, pr: &str, action: PrAction, path: Option<&str>) -> Result<(), String> {
    check_pr(pr)?;
    if action == PrAction::DeleteBranch {
        let view: Value = serde_json::from_str(&gh(cwd, path, &["pr", "view", "--json", "headRefName,headRepository,headRepositoryOwner", "--", pr])?).map_err(|e| format!("gh sent a bad answer: {e}"))?;
        let (owner, repo, branch) = (view["headRepositoryOwner"]["login"].as_str().unwrap_or(""), view["headRepository"]["name"].as_str().unwrap_or(""), view["headRefName"].as_str().unwrap_or(""));
        if owner.is_empty() || repo.is_empty() || branch.is_empty() {
            return Err("gh didn't say which branch this pull request is on".into());
        }
        return gh(cwd, path, &["api", "-X", "DELETE", &format!("repos/{owner}/{repo}/git/refs/heads/{branch}")]).map(drop);
    }
    let args: &[&str] = match action {
        PrAction::Merge => &["pr", "merge", "--merge", "--", pr],
        PrAction::Squash => &["pr", "merge", "--squash", "--", pr],
        PrAction::Rebase => &["pr", "merge", "--rebase", "--", pr],
        PrAction::Ready => &["pr", "ready", "--", pr],
        PrAction::Draft => &["pr", "ready", "--undo", "--", pr],
        PrAction::Close => &["pr", "close", "--", pr],
        PrAction::Reopen => &["pr", "reopen", "--", pr],
        PrAction::Revert => &["pr", "revert", "--", pr],
        PrAction::AutoMerge => &["pr", "merge", "--auto", "--merge", "--", pr],
        PrAction::AutoSquash => &["pr", "merge", "--auto", "--squash", "--", pr],
        PrAction::AutoRebase => &["pr", "merge", "--auto", "--rebase", "--", pr],
        PrAction::DisableAuto => &["pr", "merge", "--disable-auto", "--", pr],
        PrAction::DeleteBranch => unreachable!("answered above"),
    };
    gh(cwd, path, args).map(drop)
}

fn check_pr(pr: &str) -> Result<(), String> {
    let is_number = !pr.is_empty() && pr.bytes().all(|b| b.is_ascii_digit());
    if is_number || (pr.starts_with("https://") && pr.contains("/pull/")) {
        Ok(())
    } else {
        Err(format!("{pr:?} is not a pull request number or URL"))
    }
}

fn gh(cwd: &Path, path: Option<&str>, args: &[&str]) -> Result<String, String> {
    run(cwd, path, "gh", args).map_err(|e| match e {
        Failure::Missing => "the GitHub CLI isn't installed on this machine. Install gh and run `gh auth login`.".to_owned(),
        Failure::Said(said) => format!("gh: {said}"),
    })
}

/// Cuts `text` to `DIFF_LIMIT` bytes, answering whether it was longer.
fn clip(text: &mut String) -> bool {
    if text.len() <= DIFF_LIMIT {
        return false;
    }
    let cut = (0..=DIFF_LIMIT).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
    text.truncate(cut);
    true
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    /// The working tree, untracked files too, against HEAD.
    Uncommitted,
    /// The working tree against where HEAD left the default branch.
    Base,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Diff {
    pub patch: String,
    pub truncated: bool,
    /// The ref `DiffScope::Base` compared with, such as `origin/main`.
    pub base: Option<String>,
    /// The working tree's snapshot the diff ends on, a git tree, to read its files from.
    pub tree: String,
}

pub fn diff(cwd: &Path, scope: DiffScope, ignore_whitespace: bool, against: Option<&str>) -> Result<Diff, String> {
    let git = |args: &[&str]| run(cwd, None, "git", args).ok().map(|out| out.trim().to_owned()).filter(|out| !out.is_empty());
    let tree = history::snapshot(cwd).ok_or("this folder is not a git repository")?;
    let (from, base) = match scope {
        DiffScope::Uncommitted => (git(&["rev-parse", "--verify", "-q", "HEAD"]).unwrap_or_else(|| EMPTY_TREE.to_owned()), None),
        DiffScope::Base => {
            let chosen = match against {
                Some(b) if b.starts_with('-') || b.is_empty() => return Err(format!("{b:?} is not a branch")),
                Some(b) => Some(git(&["rev-parse", "--verify", "-q", b]).map(|_| b.to_owned()).ok_or_else(|| format!("there is no branch {b}"))?),
                None => None,
            };
            let base = chosen
                .or_else(|| git(&["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"]))
                .or_else(|| ["origin/main", "origin/master", "main", "master"].into_iter().find(|b| git(&["rev-parse", "--verify", "-q", b]).is_some()).map(str::to_owned))
                .ok_or("there is no default branch to compare with: no origin/HEAD, main or master")?;
            let fork = git(&["merge-base", "HEAD", &base]).ok_or_else(|| format!("this branch shares no history with {base}"))?;
            (fork, Some(base))
        }
    };
    let mut args = vec!["diff", "--no-color", "--no-renames", "--no-ext-diff"];
    if ignore_whitespace {
        args.push("-w");
    }
    args.extend([from.as_str(), tree.as_str()]);
    let mut patch = run(cwd, None, "git", &args).map_err(|e| match e {
        Failure::Missing => "git isn't installed on this machine".to_owned(),
        Failure::Said(said) => format!("git: {said}"),
    })?;
    let truncated = clip(&mut patch);
    Ok(Diff { patch, truncated, base, tree })
}

/// A TCP port a process on the machine listens on and that answers HTTP with a page.
#[derive(Serialize, Debug, PartialEq)]
pub struct Listener {
    pub port: u16,
    pub pid: u32,
    pub command: String,
    /// Run by the tab's session or a process it started.
    pub own: bool,
}

/// The machine's web servers, the session `root`'s own first, then by port. `path` is the PATH to
/// find lsof and ps on.
pub fn listeners(root: Option<u32>, path: Option<&str>) -> Vec<Listener> {
    let tree = root.map(|root| descendants(root, path)).unwrap_or_default();
    let mut seen = HashSet::new();
    let candidates: Vec<(u16, u32, String)> = listening(path).into_iter().filter(|(port, pid, _)| seen.insert((*port, *pid))).collect();
    let probes: Vec<_> = candidates.iter().map(|(port, ..)| std::thread::spawn({
        let port = *port;
        move || serves_html(port)
    })).collect();
    let mut found: Vec<Listener> = candidates
        .into_iter()
        .zip(probes)
        .filter_map(|((port, pid, command), probe)| probe.join().unwrap_or(false).then(|| Listener { port, pid, own: tree.contains(&pid), command }))
        .collect();
    found.sort_by_key(|l| (!l.own, l.port));
    found.dedup_by_key(|l| l.port);
    found
}

/// `(port, pid, command)` of every listening TCP socket, from lsof, or /proc where lsof is missing.
#[cfg(not(windows))]
fn listening(path: Option<&str>) -> Vec<(u16, u32, String)> {
    // macOS keeps lsof in /usr/sbin, which a PATH may leave out.
    let lsof = if Path::new("/usr/sbin/lsof").exists() { "/usr/sbin/lsof" } else { "lsof" };
    match run(Path::new("/"), path, lsof, &["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"]) {
        Ok(out) => parse_lsof(&out),
        // lsof exits 1 when nothing listens.
        Err(Failure::Said(_)) => vec![],
        Err(Failure::Missing) => proc_listening(),
    }
}

/// `(port, pid, command)` of every listening TCP socket, from netstat.
#[cfg(windows)]
fn listening(path: Option<&str>) -> Vec<(u16, u32, String)> {
    let names: HashMap<u32, String> = processes().into_iter().map(|(pid, _, name)| (pid, name)).collect();
    let out = run(Path::new("/"), path, "netstat", &["-ano"]).unwrap_or_default();
    parse_netstat(&out).into_iter().map(|(port, pid)| (port, pid, names.get(&pid).cloned().unwrap_or_default())).collect()
}

/// `(port, pid)` of each listening TCP socket in `netstat -ano`. netstat names the state in the
/// system's language, so a listening socket is told by its unset foreign address.
#[cfg(any(windows, test))]
fn parse_netstat(out: &str) -> Vec<(u16, u32)> {
    out.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let [proto, local, foreign, _, pid] = cols[..] else { return None };
            if proto != "TCP" || !matches!(foreign, "0.0.0.0:0" | "[::]:0") {
                return None;
            }
            Some((local.rsplit(':').next()?.parse().ok()?, pid.parse().ok()?))
        })
        .collect()
}

/// `(pid, parent pid, program)` of every process, the program without its .exe.
#[cfg(windows)]
fn processes() -> Vec<(u32, u32, String)> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return vec![];
    }
    // SAFETY: a valid snapshot handle this function owns, closed when `snapshot` drops.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: PROCESSENTRY32W is plain data, valid zeroed.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = vec![];
    let mut more = unsafe { Process32FirstW(handle, &mut entry) } != 0;
    while more {
        let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
        let program = exe.strip_suffix(".exe").unwrap_or(&exe).to_owned();
        found.push((entry.th32ProcessID, entry.th32ParentProcessID, program));
        more = unsafe { Process32NextW(handle, &mut entry) } != 0;
    }
    drop(snapshot);
    found
}

#[cfg(any(not(windows), test))]
fn parse_lsof(out: &str) -> Vec<(u16, u32, String)> {
    let (mut pid, mut command) = (0, String::new());
    let mut found = vec![];
    for line in out.lines() {
        let (field, value) = line.split_at(line.len().min(1));
        match field {
            "p" => pid = value.parse().unwrap_or(0),
            "c" => command = value.to_owned(),
            "n" => {
                if let Some(port) = value.rsplit(':').next().and_then(|p| p.parse().ok()) {
                    found.push((port, pid, command.clone()));
                }
            }
            _ => {}
        }
    }
    found
}

#[cfg(not(windows))]
fn proc_listening() -> Vec<(u16, u32, String)> {
    let mut ports = HashMap::new();
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        for line in text.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            // State 0A is LISTEN; column 9 is the socket's inode.
            if cols.len() > 9 && cols[3] == "0A" {
                if let Some(port) = cols[1].rsplit(':').next().and_then(|p| u16::from_str_radix(p, 16).ok()) {
                    ports.insert(format!("socket:[{}]", cols[9]), port);
                }
            }
        }
    }
    let mut found = vec![];
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        for fd in std::fs::read_dir(entry.path().join("fd")).into_iter().flatten().flatten() {
            if let Some(port) = std::fs::read_link(fd.path()).ok().and_then(|l| ports.get(&*l.to_string_lossy()).copied()) {
                let command = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default().trim().to_owned();
                found.push((port, pid, command));
            }
        }
    }
    found
}

/// `root` and every process under it.
fn descendants(root: u32, path: Option<&str>) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, ppid) in parents(path) {
        children.entry(ppid).or_default().push(pid);
    }
    let mut tree = HashSet::from([root]);
    let mut next = vec![root];
    while let Some(pid) = next.pop() {
        for &child in children.get(&pid).into_iter().flatten() {
            if tree.insert(child) {
                next.push(child);
            }
        }
    }
    tree
}

/// `(pid, parent pid)` of every process, from `ps`.
#[cfg(not(windows))]
fn parents(path: Option<&str>) -> Vec<(u32, u32)> {
    let out = run(Path::new("/"), path, "ps", &["-A", "-o", "pid=,ppid="]).unwrap_or_default();
    out.lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace().filter_map(|c| c.parse().ok());
            Some((cols.next()?, cols.next()?))
        })
        .collect()
}

#[cfg(windows)]
fn parents(_path: Option<&str>) -> Vec<(u32, u32)> {
    processes().into_iter().map(|(pid, ppid, _)| (pid, ppid)).collect()
}

fn serves_html(port: u16) -> bool {
    let ask = |addr: SocketAddr| -> Option<bool> {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
        stream.set_read_timeout(Some(Duration::from_millis(1500))).ok()?;
        stream.write_all(format!("GET / HTTP/1.0\r\nHost: localhost:{port}\r\nAccept: text/html\r\n\r\n").as_bytes()).ok()?;
        let mut head = vec![0; 4096];
        let mut len = 0;
        while len < head.len() {
            match stream.read(&mut head[len..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => len += n,
            }
            if head[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&head[..len]).to_ascii_lowercase();
        Some(head.starts_with("http/") && head.lines().any(|l| l.starts_with("content-type:") && l.contains("text/html")))
    };
    ["127.0.0.1", "[::1]"].iter().any(|host| ask(format!("{host}:{port}").parse().unwrap()).unwrap_or(false))
}

/// An editor `open_in` can open a folder in: an app bundle on a Mac, a command on PATH elsewhere.
#[derive(Serialize, Debug, PartialEq)]
pub struct Editor {
    pub id: &'static str,
    pub name: &'static str,
}

/// id, name, the Mac app, the commands elsewhere.
const EDITORS: &[(&str, &str, &str, &[&str])] = &[
    ("cursor", "Cursor", "Cursor", &["cursor"]),
    ("trae", "Trae", "Trae", &["trae"]),
    ("kiro", "Kiro", "Kiro", &["kiro"]),
    ("vscode", "VS Code", "Visual Studio Code", &["code"]),
    ("vscode-insiders", "VS Code Insiders", "Visual Studio Code - Insiders", &["code-insiders"]),
    ("vscodium", "VSCodium", "VSCodium", &["codium"]),
    ("zed", "Zed", "Zed", &["zed", "zeditor"]),
    ("antigravity", "Antigravity", "Antigravity", &["antigravity-ide"]),
    ("idea", "IntelliJ IDEA", "IntelliJ IDEA", &["idea"]),
    ("webstorm", "WebStorm", "WebStorm", &["webstorm"]),
    ("pycharm", "PyCharm", "PyCharm", &["pycharm"]),
    ("goland", "GoLand", "GoLand", &["goland"]),
    ("rustrover", "RustRover", "RustRover", &["rustrover"]),
    ("clion", "CLion", "CLion", &["clion"]),
    ("finder", "Finder", "Finder", &["xdg-open", "explorer"]),
];

/// Where `id` lives on this machine: its app bundle, or its command.
fn editor_at(id: &str, home: &Path, path: Option<&str>) -> Option<PathBuf> {
    let (_, _, app, commands) = EDITORS.iter().find(|e| e.0 == id)?;
    if cfg!(target_os = "macos") {
        if id == "finder" {
            return Some(PathBuf::from("/System/Library/CoreServices/Finder.app"));
        }
        let bundle = format!("{app}.app");
        return [home.join("Applications"), PathBuf::from("/Applications")].into_iter().map(|dir| dir.join(&bundle)).find(|b| b.is_dir());
    }
    // Windows editors put a .cmd launcher on PATH, and explorer is an .exe.
    let suffixes: &[&str] = if cfg!(windows) { &[".exe", ".cmd"] } else { &[""] };
    let dirs: Vec<PathBuf> = std::env::split_paths(path.unwrap_or_default()).collect();
    commands.iter().flat_map(|c| dirs.iter().flat_map(move |d| suffixes.iter().map(move |s| d.join(format!("{c}{s}"))))).find(|f| f.is_file())
}

pub fn editors(home: &Path, path: Option<&str>) -> Vec<Editor> {
    let name = |id, name| if id == "finder" && !cfg!(target_os = "macos") { if cfg!(windows) { "File Explorer" } else { "File manager" } } else { name };
    EDITORS.iter().filter(|e| editor_at(e.0, home, path).is_some()).map(|&(id, n, ..)| Editor { id, name: name(id, n) }).collect()
}

/// Opens `cwd`, or `file` inside its repository, in the editor `id`.
pub fn open_in(cwd: &Path, id: &str, file: Option<&str>, home: &Path, path: Option<&str>) -> Result<(), String> {
    let editor = editor_at(id, home, path).ok_or_else(|| format!("{id} isn't installed on this machine"))?;
    let target = match file {
        None => cwd.to_owned(),
        Some(file) => {
            let rel = Path::new(file);
            if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
                return Err(format!("{file:?} is not a path inside the project"));
            }
            let top = run(cwd, None, "git", &["rev-parse", "--show-toplevel"]).map(|t| PathBuf::from(t.trim())).unwrap_or_else(|_| cwd.to_owned());
            top.join(rel)
        }
    };
    let mut command = if cfg!(target_os = "macos") {
        let mut open = crate::process::command("open");
        open.arg("-a").arg(&editor);
        open
    } else {
        crate::process::command(&editor)
    };
    command.arg(&target).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|e| format!("could not start {id}: {e}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

pub(crate) enum Failure {
    Missing,
    Said(String),
}

pub(crate) fn run(cwd: &Path, path: Option<&str>, program: &str, args: &[&str]) -> Result<String, Failure> {
    let mut command = crate::process::command(program);
    command.args(args).current_dir(cwd).stdin(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let out = command.output().map_err(|_| Failure::Missing)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Failure::Said(String::from_utf8_lossy(&out.stderr).trim().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        assert!(crate::process::command("git").args(args).current_dir(dir).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success(), "git {args:?}");
    }

    #[test]
    fn a_worktree_reports_its_branch_and_its_repository_as_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let tree = repo.join(".claude/worktrees/fix");
        git(&repo, &["worktree", "add", "-q", "-b", "fix-it", tree.to_str().unwrap()]);

        let fresh = repo.join("fresh");
        std::fs::create_dir(&fresh).unwrap();
        git(&fresh, &["init", "-q", "-b", "trunk"]);
        assert_eq!((of(&fresh).repo, of(&fresh).branch.as_deref()), (true, Some("trunk")), "a repository with no commit yet");

        let main = of(&repo);
        assert_eq!((main.repo, main.branch.as_deref(), main.worktree), (true, Some("main"), false));
        let wt = of(&tree);
        assert_eq!((wt.branch.as_deref(), wt.worktree, wt.project), (Some("fix-it"), true, repo.display().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn pull_requests_list_open_ones_first_then_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let gh = dir.path().join("gh");
        let list = r#"[{"number":3,"state":"MERGED"},{"number":5,"state":"CLOSED"},{"number":2,"state":"OPEN"},{"number":4,"state":"OPEN"}]"#;
        std::fs::write(&gh, format!("#!/bin/sh\necho \"$@\" > args\necho '{list}'\n")).unwrap();
        std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", dir.path().display());
        let prs = pull_requests(dir.path(), Some(&path)).unwrap();
        let order: Vec<u64> = prs.as_array().unwrap().iter().map(|p| p["number"].as_u64().unwrap()).collect();
        assert_eq!(order, [4, 2, 5, 3]);
        assert_eq!(std::fs::read_to_string(dir.path().join("args")).unwrap().trim(), "pr list --state all --limit 50 --json number,title,state,isDraft,url,headRefName");
    }

    #[test]
    fn a_pull_request_must_be_a_number_or_url() {
        let dir = tempfile::tempdir().unwrap();
        for pr in ["--web", "-R other/repo", "", "12a", "http://github.com/o/r/pull/1"] {
            assert!(pull_request(dir.path(), pr, None).unwrap_err().contains("is not a pull request"), "{pr:?}");
        }
        // Windows also looks for a program on pinned's own PATH, so an empty PATH can't hide gh there.
        if cfg!(unix) {
            let missing = pull_request(dir.path(), "7", Some(dir.path().to_str().unwrap())).unwrap_err();
            assert!(missing.contains("isn't installed"), "{missing}");
        }
        assert!(pr_status(dir.path(), "--web", None).unwrap_err().contains("is not a pull request"));
    }

    #[test]
    fn a_pull_request_status_reads_gh_states() {
        let status = |json: &str| parse_pr_status(json).unwrap();
        assert_eq!(status(r#"{"state":"OPEN","isDraft":false,"title":"Add notes","mergedAt":null,"closedAt":null}"#).state, PrState::Open);
        assert_eq!(status(r#"{"state":"OPEN","isDraft":true,"title":"t","mergedAt":null,"closedAt":null}"#).state, PrState::Draft);
        assert_eq!(status(r#"{"state":"CLOSED","isDraft":true,"title":"t","mergedAt":null,"closedAt":"2026-01-02T00:00:00Z"}"#).state, PrState::Closed);
        assert_eq!(
            status(r#"{"state":"MERGED","isDraft":false,"title":"Add notes","mergedAt":"2026-01-01T10:40:00Z","closedAt":"2026-01-01T10:40:00Z"}"#),
            PrStatus { state: PrState::Merged, title: "Add notes".into(), merged_at: Some("2026-01-01T10:40:00Z".into()), closed_at: Some("2026-01-01T10:40:00Z".into()) }
        );
        assert!(parse_pr_status("not json").unwrap_err().contains("bad answer"));
    }

    fn commit(dir: &Path, message: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "-m", message]);
    }

    #[test]
    fn uncommitted_changes_include_new_files_and_can_leave_out_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("new.txt"), "fresh\n").unwrap();
        let before_any_commit = diff(repo, DiffScope::Uncommitted, false, None).unwrap();
        assert!(before_any_commit.patch.contains("+++ b/new.txt"), "{}", before_any_commit.patch);

        std::fs::write(repo.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        commit(repo, "first");
        std::fs::write(repo.join("a.rs"), "fn a() {}\n    fn b() {}\nfn c() {}\n").unwrap();
        std::fs::write(repo.join("untracked.md"), "hi\n").unwrap();
        let all = diff(repo, DiffScope::Uncommitted, false, None).unwrap();
        assert!(all.patch.contains("+    fn b() {}") && all.patch.contains("+++ b/untracked.md"), "{}", all.patch);
        let trimmed = diff(repo, DiffScope::Uncommitted, true, None).unwrap();
        assert!(!trimmed.patch.contains("+    fn b() {}") && trimmed.patch.contains("+fn c() {}"), "{}", trimmed.patch);
        assert_eq!((all.base, all.truncated), (None, false));
    }

    #[test]
    fn changes_against_the_base_start_where_the_branch_left_it() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("main.txt"), "one\n").unwrap();
        commit(repo, "first");
        git(repo, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(repo.join("feature.txt"), "two\n").unwrap();
        commit(repo, "feature");
        std::fs::write(repo.join("wip.txt"), "three\n").unwrap();
        let changes = diff(repo, DiffScope::Base, false, None).unwrap();
        assert_eq!(changes.base.as_deref(), Some("main"));
        let against_feature = diff(repo, DiffScope::Base, false, Some("feature")).unwrap();
        assert_eq!(against_feature.base.as_deref(), Some("feature"));
        assert!(against_feature.patch.contains("b/wip.txt") && !against_feature.patch.contains("b/feature.txt"), "{}", against_feature.patch);
        assert!(diff(repo, DiffScope::Base, false, Some("--output=x")).unwrap_err().contains("is not a branch"));
        assert!(diff(repo, DiffScope::Base, false, Some("nope")).unwrap_err().contains("no branch nope"));
        assert!(changes.patch.contains("b/feature.txt") && changes.patch.contains("b/wip.txt") && !changes.patch.contains("main.txt"), "{}", changes.patch);

        let lone = tempfile::tempdir().unwrap();
        git(lone.path(), &["init", "-q", "-b", "trunk"]);
        assert!(diff(lone.path(), DiffScope::Base, false, None).unwrap_err().contains("no default branch"));
        assert!(diff(&lone.path().join(".git"), DiffScope::Uncommitted, false, None).is_err());
    }

    #[test]
    fn lsof_output_names_each_port_with_its_process() {
        let out = "p501\ncnode\nf23\nn*:5173\nf24\nn[::1]:5173\np77\ncrapportd\nf5\nn127.0.0.1:49152\n";
        assert_eq!(parse_lsof(out), vec![(5173, 501, "node".into()), (5173, 501, "node".into()), (49152, 77, "rapportd".into())]);
    }

    #[test]
    fn netstat_output_names_each_listening_port_with_its_process() {
        let out = "\r\nConexões ativas\r\n\r\n  Proto  Endereço local  Endereço externo  Estado\r\n  TCP    0.0.0.0:5173   0.0.0.0:0  ESCUTANDO  501\r\n  TCP    127.0.0.1:5173  127.0.0.1:61000  ESTABELECIDA  501\r\n  TCP    [::1]:3000  [::]:0  ESCUTANDO  77\r\n  UDP    0.0.0.0:5353  *:*  88\r\n";
        assert_eq!(parse_netstat(out), vec![(5173, 501), (3000, 77)]);
    }

    #[test]
    fn a_web_server_the_session_started_is_listed_as_its_own() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let quiet = silent.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in server.incoming().flatten() {
                let mut stream = stream;
                let _ = stream.read(&mut [0; 1024]);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 4\r\n\r\nhi!\n");
            }
        });
        let found = listeners(Some(std::process::id()), std::env::var("PATH").ok().as_deref());
        let ours = found.iter().find(|l| l.port == port).unwrap_or_else(|| panic!("{port} missing from {found:?}"));
        assert!(ours.own && ours.pid == std::process::id(), "{ours:?}");
        assert!(found.iter().all(|l| l.port != quiet), "a port that never answers HTML is listed: {found:?}");
        drop(silent);
    }

    #[test]
    fn editors_open_only_known_ids_and_paths_inside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().to_str().unwrap();
        assert!(open_in(dir.path(), "notepad", None, dir.path(), Some(empty)).unwrap_err().contains("isn't installed"));
        for path in ["../secret", "/etc/passwd", "a/../../b"] {
            let err = open_in(dir.path(), "finder", Some(path), dir.path(), Some(empty));
            if cfg!(target_os = "macos") {
                assert!(err.unwrap_err().contains("not a path inside"), "{path}");
            }
        }
        assert!(pr_action(dir.path(), "--admin", PrAction::Merge, None).unwrap_err().contains("is not a pull request"));
    }

    #[test]
    fn a_plain_folder_has_no_branch() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(of(dir.path()), Workspace { project: dir.path().display().to_string(), repo: false, branch: None, worktree: false });
    }
}
