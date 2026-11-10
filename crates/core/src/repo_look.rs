//! The global orchestrator's read-only look at a project's checkout: an overview, a file's lines
//! and a search. Nothing here writes: every git command runs with optional locks off and no
//! fsmonitor, so even the index stays as it was. Paths stay inside the checkout, and secrets
//! (`blocked`, everything gitignored, and lines that hold a token) never leave the machine.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

/// Lines and bytes one `Read` returns at most.
pub const READ_LINES: u32 = 400;
pub const READ_BYTES: usize = 24 * 1024;
/// Hits one `Search` returns by default and at most, and the characters of each.
pub const SEARCH_HITS: u32 = 40;
pub const SEARCH_MAX: u32 = 100;
const HIT_CHARS: usize = 200;
/// Paths a glob-only `Search` lists at most.
pub const GLOB_PATHS: usize = 200;
const DOC_BYTES: usize = 8 * 1024;
const README_BYTES: usize = 2 * 1024;

/// What the global orchestrator asks of a checkout.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "look", rename_all = "snake_case")]
pub enum RepoLook {
    /// Its shape: HEAD, size, languages, top folders, packages, CLAUDE.md, AGENTS.md and the
    /// start of the README, and the latest commits.
    Overview,
    /// `lines` lines of `file` from `from_line` (1 when not given), at `rev` when given.
    Read {
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_line: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<u32>,
    },
    /// Lines that hold `query`, in files that match `glob` when given; with `glob` alone, the
    /// tracked paths that match it.
    Search {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        glob: Option<String>,
        #[serde(default)]
        regex: bool,
        #[serde(default)]
        case: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
    },
}

impl RepoLook {
    /// The tool's name, as the audit trail records it.
    pub fn action(&self) -> &'static str {
        match self {
            RepoLook::Overview => "repo_overview",
            RepoLook::Read { .. } => "repo_read",
            RepoLook::Search { .. } => "repo_search",
        }
    }

    /// The arguments as the audit trail keeps them: never any file text.
    pub fn args(&self) -> Value {
        match self {
            RepoLook::Overview => json!({}),
            RepoLook::Read { file, rev, from_line, lines } => json!({ "path": file, "rev": rev, "from_line": from_line, "lines": lines }),
            RepoLook::Search { query, glob, regex, case, max } => json!({ "query": query, "glob": glob, "regex": regex, "case": case, "max": max }),
        }
    }
}

/// Whether `rel`, a path inside a checkout, names a file that may hold a secret. Its text never
/// leaves the machine, whatever the look.
pub fn blocked(rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| matches!(*p, ".git" | ".ssh" | ".aws")) {
        return true;
    }
    let Some(name) = parts.last() else { return true };
    let name = name.to_ascii_lowercase();
    let example = [".example", ".sample", ".template"].iter().any(|e| name.ends_with(e));
    if name == ".env" || name.starts_with(".env.") {
        return !example;
    }
    if matches!(name.as_str(), ".dev.vars" | ".npmrc" | ".pypirc" | ".netrc") {
        return true;
    }
    if [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"].iter().any(|e| name.ends_with(e)) {
        return true;
    }
    if ["id_rsa", "id_ed25519", "id_ecdsa", "credentials", "secrets"].iter().any(|p| name.starts_with(p)) {
        return !example;
    }
    if name.starts_with("service-account") && name.ends_with(".json") {
        return true;
    }
    name.contains(".tfstate")
}

/// Whether a line holds what looks like a token or a key.
fn secret_line(line: &str) -> bool {
    const MARKS: [&str; 5] = ["ghp_", "github_pat_", "sk-", "AKIA", "PRIVATE KEY-----"];
    if MARKS.iter().any(|m| line.contains(m)) {
        return true;
    }
    if ["xoxa-", "xoxb-", "xoxp-"].iter().any(|m| line.contains(m)) {
        return true;
    }
    // A URL with a password: scheme://user:pass@host.
    line.split("://").skip(1).any(|rest| {
        let auth = rest.split(['/', ' ', '"', '\'']).next().unwrap_or("");
        auth.contains('@') && auth.split('@').next().is_some_and(|cred| cred.contains(':'))
    })
}

/// `text` with every line that holds a secret replaced by `[redacted]`, and how many were.
pub fn redact(text: &str) -> (String, usize) {
    let mut count = 0;
    let lines: Vec<&str> = text
        .split_inclusive('\n')
        .map(|line| {
            if secret_line(line) {
                count += 1;
                if line.ends_with('\n') { "[redacted]\n" } else { "[redacted]" }
            } else {
                line
            }
        })
        .collect();
    (lines.concat(), count)
}

/// `git` in `top`, read-only: no optional locks, no fsmonitor, no pager, no prompt.
fn git(top: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = crate::process::command("git")
        .args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=false"])
        .args(args)
        .current_dir(top)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "git isn't installed on this machine".to_owned())?;
    match out.status.code() {
        Some(0) => Ok(out.stdout),
        // git grep and check-ignore answer 1 for nothing found.
        Some(1) if out.stderr.is_empty() => Err(String::new()),
        _ => Err(format!("git: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}

fn git_text(top: &Path, args: &[&str]) -> Option<String> {
    git(top, args).ok().map(|out| String::from_utf8_lossy(&out).trim().to_owned())
}

/// The checkout's files that git tracks or would track, without the blocked ones.
fn files(top: &Path) -> Result<Vec<String>, String> {
    let out = git(top, &["ls-files", "-co", "--exclude-standard", "-z"])?;
    Ok(String::from_utf8_lossy(&out).split('\0').filter(|p| !p.is_empty() && !blocked(p)).map(str::to_owned).collect())
}

fn ignored(top: &Path, rel: &str) -> bool {
    git(top, &["check-ignore", "-q", "--", rel]).is_ok()
}

/// `rel` as a path inside `top`: every part a plain name, so no `..`, root or drive.
fn inside(rel: &str) -> Result<&str, String> {
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() || !Path::new(rel).components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("{rel:?} is not a path inside the project"));
    }
    Ok(rel)
}

fn check_rev(rev: &str) -> Result<(), String> {
    let ok = !rev.is_empty() && !rev.starts_with('-') && rev.len() < 200 && rev.chars().all(|c| c.is_ascii_alphanumeric() || "_./^~-".contains(c));
    ok.then_some(()).ok_or_else(|| format!("{rev:?} is not a revision"))
}

/// The branch and short commit the look read.
fn head(top: &Path) -> Value {
    json!({ "branch": git_text(top, &["rev-parse", "--abbrev-ref", "HEAD"]), "sha": git_text(top, &["rev-parse", "--short", "HEAD"]) })
}

/// Answers `look` on the checkout at `top`, a project folder of this machine.
pub fn look(top: &Path, look: &RepoLook) -> Result<Value, String> {
    let top = dunce::canonicalize(top).map_err(|e| format!("{}: {e}", top.display()))?;
    let mut out = match look {
        RepoLook::Overview => overview(&top)?,
        RepoLook::Read { file, rev, from_line, lines } => read(&top, file, rev.as_deref(), from_line.unwrap_or(1), lines.unwrap_or(READ_LINES))?,
        RepoLook::Search { query: Some(query), glob, regex, case, max } if !query.is_empty() => search(&top, query, glob.as_deref(), *regex, *case, max.unwrap_or(SEARCH_HITS))?,
        RepoLook::Search { glob: Some(glob), .. } => paths(&top, glob)?,
        RepoLook::Search { .. } => return Err("repo_search needs a query, a glob, or both".into()),
    };
    let head = head(&top);
    out["branch"] = head["branch"].clone();
    out["sha"] = head["sha"].clone();
    out["path"] = json!(top.display().to_string());
    Ok(out)
}

fn read(top: &Path, file: &str, rev: Option<&str>, from_line: u32, lines: u32) -> Result<Value, String> {
    let rel = inside(file)?;
    if blocked(rel) {
        return Err(format!("{rel} may hold secrets, so the Orchestrator can't read it"));
    }
    let bytes = match rev {
        Some(rev) => {
            check_rev(rev)?;
            git(top, &["show", &format!("{rev}:./{rel}")]).map_err(|e| if e.is_empty() { format!("no {rel} at {rev}") } else { e })?
        }
        None => {
            let real = dunce::canonicalize(top.join(rel)).map_err(|_| format!("no file {rel} in the project"))?;
            if !real.starts_with(top) {
                return Err(format!("{rel:?} leads outside the project"));
            }
            let real_rel = real.strip_prefix(top).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
            if blocked(&real_rel) || ignored(top, rel) || ignored(top, &real_rel) {
                return Err(format!("{rel} is ignored by git or may hold secrets, so the Orchestrator can't read it"));
            }
            if real.is_dir() {
                return Err(format!("{rel} is a folder: list it with repo_search and a glob such as {rel}/*"));
            }
            std::fs::read(&real).map_err(|e| format!("could not read {rel}: {e}"))?
        }
    };
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Ok(json!({ "file": rel, "rev": rev, "binary": true, "bytes": bytes.len() }));
    }
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.split_inclusive('\n').collect();
    let total = all.len() as u32;
    let from = from_line.max(1);
    let want = lines.clamp(1, READ_LINES);
    let mut shown = String::new();
    let mut to = from - 1;
    for line in all.iter().skip((from - 1) as usize).take(want as usize) {
        if !shown.is_empty() && shown.len() + line.len() > READ_BYTES {
            break;
        }
        // One line longer than the cap is cut, so a minified file still reads.
        let line = if line.len() > READ_BYTES { &line[..floor(line, READ_BYTES)] } else { line };
        shown.push_str(line);
        to += 1;
    }
    let (text, redacted) = redact(&shown);
    let next = (to < total).then_some(to + 1);
    Ok(json!({ "file": rel, "rev": rev, "from_line": from, "to_line": to, "total_lines": total, "next_line": next, "text": text, "redacted": redacted }))
}

/// The largest char boundary of `s` at or below `at`.
fn floor(s: &str, at: usize) -> usize {
    (0..=at.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0)
}

/// A glob as the tools take it: `*` within a folder, `**` across folders, `?` one character.
fn glob_match(glob: &str, path: &str) -> bool {
    fn go(g: &[u8], p: &[u8]) -> bool {
        match g.split_first() {
            None => p.is_empty(),
            Some((b'*', rest)) if rest.first() == Some(&b'*') => {
                let rest = &rest[1..];
                let rest = rest.strip_prefix(b"/").unwrap_or(rest);
                (0..=p.len()).any(|i| (i == 0 || p[i - 1] == b'/' || rest.is_empty()) && go(rest, &p[i..]))
            }
            Some((b'*', rest)) => (0..=p.len()).take_while(|&i| i == 0 || p[i - 1] != b'/').any(|i| go(rest, &p[i..])),
            Some((b'?', rest)) => p.first().is_some_and(|&c| c != b'/') && go(rest, &p[1..]),
            Some((c, rest)) => p.first() == Some(c) && go(rest, &p[1..]),
        }
    }
    // A glob with no folder matches a file's name anywhere, as git and ripgrep read one.
    if !glob.contains('/') {
        return go(glob.as_bytes(), path.rsplit('/').next().unwrap_or(path).as_bytes());
    }
    go(glob.trim_start_matches("./").as_bytes(), path.as_bytes())
}

fn check_glob(glob: &str) -> Result<(), String> {
    let refused = glob.starts_with('/') || glob.starts_with('~') || glob.contains('\\') || glob.split('/').any(|p| p == "..") || glob.contains(':');
    if refused {
        return Err(format!("{glob:?} is not a glob inside the project"));
    }
    Ok(())
}

fn paths(top: &Path, glob: &str) -> Result<Value, String> {
    check_glob(glob)?;
    let matched: Vec<String> = files(top)?.into_iter().filter(|p| glob_match(glob, p)).collect();
    let shown: Vec<&String> = matched.iter().take(GLOB_PATHS).collect();
    Ok(json!({ "paths": shown, "shown": shown.len(), "total": matched.len() }))
}

fn search(top: &Path, query: &str, glob: Option<&str>, regex: bool, case: bool, max: u32) -> Result<Value, String> {
    if let Some(glob) = glob {
        check_glob(glob)?;
    }
    let max = max.clamp(1, SEARCH_MAX) as usize;
    let mut args = vec!["grep", "-n", "-I", "--no-color", "--full-name", "-z", "--untracked"];
    if !case {
        args.push("-i");
    }
    args.push(if regex { "-E" } else { "-F" });
    args.extend(["-e", query]);
    let out = match git(top, &args) {
        Ok(out) => out,
        Err(e) if e.is_empty() => vec![],
        Err(e) => return Err(e),
    };
    let text = String::from_utf8_lossy(&out);
    let mut hits = vec![];
    let mut files = std::collections::BTreeSet::new();
    let mut total = 0;
    let mut redacted = 0;
    for line in text.lines() {
        let mut parts = line.splitn(3, '\0');
        let (Some(path), Some(number), Some(body)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if blocked(path) || glob.is_some_and(|g| !glob_match(g, path)) {
            continue;
        }
        total += 1;
        files.insert(path.to_owned());
        if hits.len() < max {
            let body: String = body.chars().take(HIT_CHARS).collect();
            let (body, n) = redact(&body);
            redacted += n;
            hits.push(json!({ "path": path, "line": number.parse::<u64>().unwrap_or(0), "text": body }));
        }
    }
    Ok(json!({ "hits": hits, "shown": hits.len(), "total": total, "files": files.len(), "redacted": redacted }))
}

fn overview(top: &Path) -> Result<Value, String> {
    let files = files(top)?;
    let mut bytes = 0u64;
    let mut languages: BTreeMap<String, u32> = BTreeMap::new();
    let mut tree: BTreeMap<String, u32> = BTreeMap::new();
    for path in &files {
        bytes += std::fs::symlink_metadata(top.join(path)).map(|m| m.len()).unwrap_or(0);
        let name = path.rsplit('/').next().unwrap_or(path);
        if let Some((_, ext)) = name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) {
            *languages.entry(ext.to_ascii_lowercase()).or_default() += 1;
        }
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() > 1 {
            let dir = parts[..parts.len().min(3) - 1].join("/");
            *tree.entry(dir).or_default() += 1;
        }
    }
    let top_n = |counts: BTreeMap<String, u32>, n: usize, key: &str| {
        let mut list: Vec<(String, u32)> = counts.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        list.into_iter().take(n).map(|(k, files)| json!({ key: k, "files": files })).collect::<Vec<_>>()
    };
    let packages: Vec<Value> = files
        .iter()
        .filter(|p| p.split('/').count() <= 4)
        .filter_map(|p| {
            let (dir, name) = p.rsplit_once('/').unwrap_or((".", p));
            let kind = match name {
                "Cargo.toml" => "cargo",
                "package.json" => "npm",
                "pyproject.toml" => "python",
                "go.mod" => "go",
                n if n.ends_with(".csproj") => "dotnet",
                _ => return None,
            };
            let text = std::fs::read_to_string(top.join(p)).unwrap_or_default();
            Some(json!({ "kind": kind, "path": dir, "name": package_name(kind, &text) }))
        })
        .take(30)
        .collect();
    let mut docs = serde_json::Map::new();
    for (name, cap) in [("CLAUDE.md", DOC_BYTES), ("AGENTS.md", DOC_BYTES), ("README.md", README_BYTES)] {
        if !files.iter().any(|f| f == name) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(top.join(name)) else { continue };
        let truncated = text.len() > cap;
        let (text, _) = redact(&text[..floor(&text, cap)]);
        docs.insert(name.to_owned(), json!({ "text": text, "truncated": truncated }));
    }
    let commits: Vec<Value> = git_text(top, &["log", "-5", "--no-show-signature", "--format=%h%x00%s%x00%an%x00%at"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\0');
            let (sha, subject, author, at) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(json!({ "sha": sha, "subject": redact(subject).0, "author": author, "at": at.parse::<u64>().unwrap_or(0) }))
        })
        .collect();
    let dirty = git_text(top, &["status", "--porcelain", "--untracked-files=no"]).map_or(0, |s| s.lines().count());
    let subject = git_text(top, &["log", "-1", "--format=%s"]).map(|s| redact(&s).0);
    Ok(json!({
        "head": { "subject": subject, "dirty": dirty },
        "size": { "files": files.len(), "bytes": bytes },
        "languages": top_n(languages, 12, "ext"),
        "tree": top_n(tree, 25, "dir"),
        "packages": packages,
        "docs": docs,
        "recent_commits": commits,
    }))
}

/// The package's own name from its manifest, when it names one.
fn package_name(kind: &str, text: &str) -> Option<String> {
    match kind {
        "npm" => serde_json::from_str::<Value>(text).ok()?["name"].as_str().map(str::to_owned),
        "go" => text.lines().find_map(|l| l.strip_prefix("module ")).map(|m| m.trim().to_owned()),
        "cargo" | "python" => text
            .lines()
            .skip_while(|l| !matches!(l.trim(), "[package]" | "[project]" | "[tool.poetry]"))
            .find_map(|l| l.trim().strip_prefix("name").map(|r| r.trim_start().trim_start_matches('=').trim().trim_matches('"').to_owned())),
        _ => None,
    }
}

/// The folders a confined session may reach, each for reading or for writing too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roots {
    pub read: Vec<PathBuf>,
    pub write: Vec<PathBuf>,
}

/// What the global orchestrator's own file tools may do with `input`, the tool call Claude Code's
/// PreToolUse hook hands over: Ok to let it run, Err with the reason to refuse it. Paths are made
/// absolute against `cwd`, then resolved through `..` and symlinks before they are matched, so
/// anything that leads out of the roots is refused. Any tool this does not know is refused.
pub fn guard(roots: &Roots, cwd: &Path, tool: &str, input: &Value) -> Result<(), String> {
    const OUTSIDE: &str = "outside the Orchestrator's folders; read projects with repo_read and repo_search";
    let (key, write) = match tool {
        "Read" => ("file_path", false),
        "Write" | "Edit" | "MultiEdit" => ("file_path", true),
        "Grep" | "Glob" | "LS" => ("path", false),
        _ => return Err(format!("{tool} is not one of the Orchestrator's tools")),
    };
    if tool == "Glob" {
        let pattern = input["pattern"].as_str().unwrap_or("");
        if Path::new(pattern).is_absolute() || pattern.starts_with('~') || pattern.split(['/', '\\']).any(|p| p == "..") {
            return Err(format!("the glob {pattern:?} is {OUTSIDE}"));
        }
    }
    let asked = match input[key].as_str().filter(|p| !p.is_empty()) {
        Some(path) => path.to_owned(),
        // Grep and Glob search the session's own folder without a path.
        None if !write && tool != "Read" => cwd.display().to_string(),
        None => return Err(format!("{tool} needs a {key}")),
    };
    let path = if Path::new(&asked).is_absolute() { PathBuf::from(&asked) } else { cwd.join(&asked) };
    let real = resolve(&path).ok_or_else(|| format!("{asked} is {OUTSIDE}"))?;
    let allowed = if write { &roots.write } else { &roots.read };
    let roots: Vec<PathBuf> = allowed.iter().filter_map(|r| resolve(r)).collect();
    if roots.iter().any(|root| real.starts_with(root)) {
        return Ok(());
    }
    Err(format!("{asked} is {OUTSIDE}"))
}

/// `path` with `..` and every symlink resolved. A path that doesn't exist yet, such as a file
/// about to be written, resolves its nearest existing folder and keeps the plain names after it.
fn resolve(path: &Path) -> Option<PathBuf> {
    if let Ok(real) = dunce::canonicalize(path) {
        return Some(real);
    }
    let mut missing = vec![];
    let mut at = path;
    loop {
        let name = at.file_name()?;
        let parent = at.parent()?;
        if !matches!(Path::new(name).components().next(), Some(Component::Normal(_))) {
            return None;
        }
        missing.push(name.to_owned());
        if let Ok(real) = dunce::canonicalize(parent) {
            return Some(missing.into_iter().rev().fold(real, |acc, n| acc.join(n)));
        }
        at = parent;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git").args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    /// A committed repo with secrets, an ignored file, a big file and a symlink out, beside a
    /// folder outside it that holds a private key.
    fn repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().join("app");
        let outside = tmp.path().join("home-ssh");
        std::fs::create_dir_all(top.join("src")).unwrap();
        std::fs::create_dir_all(top.join("config")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("id_rsa"), "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        std::fs::write(outside.join("notes"), "outside text\n").unwrap();
        let w = |rel: &str, text: &str| std::fs::write(top.join(rel), text).unwrap();
        w("Cargo.toml", "[package]\nname = \"demo-app\"\n");
        w("AGENTS.md", "Run the wire check before pushing.\n");
        w("src/main.rs", "fn main() {\n    let token = \"ghp_abcdefabcdef\";\n    println!(\"needle\");\n}\n");
        w(".env", "API=1\n");
        w(".env.local", "API=2\n");
        w("config/.env.production", "API=3\n");
        w(".env.example", "API=\n");
        w("id_ed25519", "key\n");
        w("server.pem", "pem\n");
        w(".gitignore", "notes.txt\n");
        w("notes.txt", "needle in an ignored file\n");
        let big: String = (1..=20_000).map(|i| format!("needle line {i} of a big file\n")).collect();
        w("big.txt", &big);
        symlink(&outside, top.join("ssh")).unwrap();
        symlink(outside.join("notes"), top.join("outside-notes")).unwrap();
        symlink(top.join("AGENTS.md"), top.join("agents-link.md")).unwrap();
        sh(&top, &["init", "-q", "-b", "main"]);
        sh(&top, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "-A"]);
        sh(&top, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "first"]);
        (tmp, top, outside)
    }

    fn read(top: &Path, file: &str) -> Result<Value, String> {
        look(top, &RepoLook::Read { file: file.into(), rev: None, from_line: None, lines: None })
    }

    fn search(top: &Path, query: Option<&str>, glob: Option<&str>, max: Option<u32>) -> Result<Value, String> {
        look(top, &RepoLook::Search { query: query.map(str::to_owned), glob: glob.map(str::to_owned), regex: false, case: false, max })
    }

    #[test]
    fn a_path_that_leads_out_of_the_checkout_is_refused() {
        let (_tmp, top, _) = repo();
        for file in ["../x", "a/../../x", "/etc/passwd", "~/.ssh/id_rsa", "ssh/id_rsa", "outside-notes", ".git/config"] {
            assert!(read(&top, file).is_err(), "{file} read: {:?}", read(&top, file));
        }
        for glob in ["../**", "/**", "~/**", "a/../../*"] {
            assert!(search(&top, None, Some(glob), None).is_err(), "glob {glob}");
        }
        let rev = look(&top, &RepoLook::Read { file: "AGENTS.md".into(), rev: Some("HEAD:../x".into()), from_line: None, lines: None });
        assert_eq!(rev, Err("\"HEAD:../x\" is not a revision".into()));
        assert_eq!(read(&top, "agents-link.md").unwrap()["text"], "Run the wire check before pushing.\n", "a symlink to a file inside reads");
        let at_head = look(&top, &RepoLook::Read { file: "src/main.rs".into(), rev: Some("HEAD".into()), from_line: Some(3), lines: Some(1) }).unwrap();
        assert_eq!(at_head["text"], "    println!(\"needle\");\n");
    }

    #[test]
    fn secrets_are_never_read_listed_or_found() {
        let (_tmp, top, _) = repo();
        for file in [".env", ".env.local", "config/.env.production", "id_ed25519", "server.pem", "notes.txt"] {
            assert!(read(&top, file).unwrap_err().contains("secrets"), "{file}");
        }
        assert_eq!(read(&top, ".env.example").unwrap()["text"], "API=\n");
        let main = read(&top, "src/main.rs").unwrap();
        assert_eq!(main["text"], "fn main() {\n[redacted]\n    println!(\"needle\");\n}\n");
        assert_eq!(main["redacted"], 1);
        let listed = search(&top, None, Some("**"), None).unwrap();
        let paths: Vec<&str> = listed["paths"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
        for secret in [".env", ".env.local", "config/.env.production", "id_ed25519", "server.pem", "notes.txt"] {
            assert!(!paths.contains(&secret), "{secret} listed: {paths:?}");
        }
        assert!(paths.contains(&".env.example") && paths.contains(&"src/main.rs"), "{paths:?}");
        let found = search(&top, Some("needle"), Some("*.txt"), Some(100)).unwrap();
        assert!(found["hits"].as_array().unwrap().iter().all(|h| h["path"] == "big.txt"), "the ignored notes.txt is not searched: {found}");
        let api = search(&top, Some("API="), None, None).unwrap();
        let hit_paths: Vec<&str> = api["hits"].as_array().unwrap().iter().map(|h| h["path"].as_str().unwrap()).collect();
        assert_eq!(hit_paths, [".env.example"], "only the example's line is found");
        let token = search(&top, Some("token"), None, None).unwrap();
        assert_eq!((token["hits"][0]["text"].as_str(), token["redacted"].as_u64()), (Some("[redacted]"), Some(1)));
    }

    #[test]
    fn reads_and_searches_are_capped() {
        let (_tmp, top, _) = repo();
        let first = read(&top, "big.txt").unwrap();
        assert_eq!((first["from_line"].as_u64(), first["to_line"].as_u64(), first["next_line"].as_u64(), first["total_lines"].as_u64()), (Some(1), Some(400), Some(401), Some(20_000)));
        assert!(first["text"].as_str().unwrap().len() <= READ_BYTES);
        let asked_more = look(&top, &RepoLook::Read { file: "big.txt".into(), rev: None, from_line: Some(19_900), lines: Some(5000) }).unwrap();
        assert_eq!((asked_more["to_line"].as_u64(), asked_more["next_line"].as_u64()), (Some(20_000), None));
        let long: String = (0..2000).map(|i| format!("{i:040}\n")).collect();
        std::fs::write(top.join("wide.txt"), &long).unwrap();
        let wide = read(&top, "wide.txt").unwrap();
        assert!(wide["text"].as_str().unwrap().len() <= READ_BYTES && wide["to_line"].as_u64() < Some(400), "24 KB first: {}", wide["to_line"]);
        let hits = search(&top, Some("needle"), None, None).unwrap();
        assert_eq!((hits["shown"].as_u64(), hits["total"].as_u64()), (Some(40), Some(20_001)));
        assert_eq!(search(&top, Some("needle"), None, Some(500)).unwrap()["shown"], 100, "max is clamped");
        assert!(hits["hits"][0]["text"].as_str().unwrap().chars().count() <= 200);
    }

    #[test]
    fn no_look_changes_the_checkout_or_its_git_folder() {
        let (_tmp, top, _) = repo();
        std::fs::write(top.join("src/main.rs"), "changed\n").unwrap();
        let snapshot = |dir: &Path| {
            let mut all = vec![];
            let mut stack = vec![dir.to_owned()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap().flatten() {
                    let p = e.path();
                    let meta = std::fs::symlink_metadata(&p).unwrap();
                    if meta.is_dir() {
                        stack.push(p.clone());
                    } else if meta.is_file() {
                        all.push((p.clone(), std::fs::read(&p).unwrap(), meta.modified().unwrap()));
                    }
                }
            }
            all.sort();
            all
        };
        let before = snapshot(&top);
        look(&top, &RepoLook::Overview).unwrap();
        read(&top, "src/main.rs").unwrap();
        search(&top, Some("needle"), None, None).unwrap();
        search(&top, None, Some("src/*"), None).unwrap();
        assert!(before == snapshot(&top), "the work tree and .git, index included, are byte for byte the same");
    }

    #[test]
    fn the_overview_names_the_head_the_package_and_the_rules() {
        let (_tmp, top, _) = repo();
        let card = look(&top, &RepoLook::Overview).unwrap();
        assert_eq!(card["branch"], "main");
        assert_eq!(card["head"]["subject"], "first");
        assert_eq!(card["packages"], json!([{ "kind": "cargo", "path": ".", "name": "demo-app" }]));
        assert_eq!(card["docs"]["AGENTS.md"]["text"], "Run the wire check before pushing.\n");
        assert!(!card.to_string().contains("API="), "no secret in the card: {card}");
    }

    #[test]
    fn globs_match_like_the_tools_do() {
        assert!(glob_match("*.rs", "crates/core/src/wire.rs"));
        assert!(glob_match("crates/**/*.rs", "crates/core/src/wire.rs"));
        assert!(glob_match("**", "a/b/c"));
        assert!(!glob_match("crates/*.rs", "crates/core/wire.rs"));
        assert!(glob_match("src/?ain.rs", "src/main.rs"));
    }

    fn roots(tmp: &Path) -> (Roots, PathBuf) {
        let [own, scratch, memory, uploads, captain, project] = ["own", "scratch", "memory", "uploads", "captain-scratch", "project"].map(|d| {
            let p = tmp.join(d);
            std::fs::create_dir_all(&p).unwrap();
            p
        });
        std::fs::write(project.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(uploads.join("shot.png"), "png").unwrap();
        std::fs::write(captain.join("plan.md"), "plan").unwrap();
        symlink(&project, scratch.join("into-project")).unwrap();
        let roots = Roots { read: vec![uploads, scratch.clone(), memory.clone(), own.clone(), captain], write: vec![scratch, memory, own.clone()] };
        (roots, own)
    }

    #[test]
    fn the_orchestrators_file_tools_reach_only_its_own_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let t = dunce::canonicalize(tmp.path()).unwrap();
        let (roots, own) = roots(&t);
        let at = |tool: &str, input: Value| guard(&roots, &own, tool, &input);
        let p = |rel: &str| t.join(rel).display().to_string();
        assert_eq!(at("Read", json!({ "file_path": p("uploads/shot.png") })), Ok(()));
        assert_eq!(at("Read", json!({ "file_path": p("scratch/page.html") })), Ok(()));
        assert_eq!(at("Write", json!({ "file_path": p("scratch/new/page.html") })), Ok(()), "a new file in a new folder of the scratchpad");
        assert_eq!(at("Edit", json!({ "file_path": p("memory/MEMORY.md") })), Ok(()));
        assert_eq!(at("Write", json!({ "file_path": "notes.md" })), Ok(()), "relative to its own folder");
        assert_eq!(at("Grep", json!({ "pattern": "x" })), Ok(()), "no path searches its own folder");
        assert_eq!(at("Glob", json!({ "pattern": "**/*.md" })), Ok(()));
        assert_eq!(at("Read", json!({ "file_path": p("captain-scratch/plan.md") })), Ok(()), "a Captain's scratchpad reads");
        let denied = |tool: &str, input: Value| assert!(at(tool, input.clone()).unwrap_err().contains("outside the Orchestrator's folders"), "{tool} {input}");
        denied("Write", json!({ "file_path": p("captain-scratch/plan.md") }));
        denied("Write", json!({ "file_path": p("uploads/shot.png") }));
        denied("Read", json!({ "file_path": p("project/main.rs") }));
        denied("Edit", json!({ "file_path": p("project/main.rs") }));
        denied("Read", json!({ "file_path": p("scratch/into-project/main.rs") }));
        denied("Write", json!({ "file_path": p("scratch/into-project/new.rs") }));
        denied("Read", json!({ "file_path": p("scratch/../project/main.rs") }));
        denied("Write", json!({ "file_path": p("scratch/missing/../../project/x.rs") }));
        denied("Read", json!({ "file_path": "../project/main.rs" }));
        denied("Grep", json!({ "pattern": "x", "path": t.parent().unwrap().display().to_string() }));
        denied("Glob", json!({ "pattern": "**", "path": p("project") }));
        denied("Glob", json!({ "pattern": "../project/**" }));
        denied("Glob", json!({ "pattern": format!("{}/**", p("project")) }));
        for tool in ["Bash", "NotebookEdit", "WebFetch"] {
            assert!(at(tool, json!({ "command": "ls" })).unwrap_err().contains("not one of the Orchestrator's tools"), "{tool}");
        }
    }
}
