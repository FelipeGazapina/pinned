//! `pinned captain-guard`: Claude Code's PreToolUse hook on every Bash call of a Captain whose
//! project is Manage only. That Captain starts without the file tools and loads this hook from
//! `hooks::CAPTAIN_SETTINGS`, so the shell is no way around them: its session reads, checks and
//! merges, and a worker changes files.
//!
//! It denies a command that writes a file, or changes the checkout or its history, wherever the
//! line runs it. It never asks the owner. A write to a standard device or below a temp folder runs,
//! the session's scratch folder among them; a file or folder below a temp folder that is named
//! `pinned…` or `claude…` does not, unless it is that scratch folder. It reads the line with the
//! never list's parser and makes no model, socket, hub or network call.
//!
//! It stops a habit, not an evasion: code an interpreter runs (`python -c`, `node -e`, a script
//! file) passes, and so does what a tool writes in doing its own job (`cargo` and `target/`).

use super::never::{chdir, commands, git_sub, lex, tgt, walk, Cmd, Segment, Word, SHELLS};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SUBCOMMAND: &str = "captain-guard";

const REASON: &str = "pinned: this Captain is manage-only, so Bash is for reading and checking. Hand this change to a worker with delegate_task.";
/// The devices that keep nothing written to them.
const DEVICES: [&str; 3] = ["/dev/null", "/dev/stdout", "/dev/stderr"];
/// The git subcommands that only read: they change neither the checkout, nor the history, nor the refs.
const GIT_READS: &[&str] = &[
    "log", "diff", "show", "status", "fetch", "rev-parse", "ls-files", "ls-remote", "ls-tree", "blame", "grep", "describe", "merge-base", "cat-file", "for-each-ref", "shortlog", "rev-list", "name-rev", "count-objects", "version", "help", "whatchanged", "range-diff", "cherry", "show-ref", "show-branch", "verify-commit", "verify-tag", "check-ignore", "check-attr",
];
/// The variables that run a command for git (`GIT_EXTERNAL_DIFF`, `GIT_SSH_COMMAND`) or name the config that does (`GIT_CONFIG_*`).
const GIT_HAZARD_ENV: [&str; 3] = ["GIT_EXTERNAL_DIFF=", "GIT_SSH_COMMAND=", "GIT_CONFIG_"];
/// The short options that take a value, per tool: a flag letter after one of these is that value's.
const CURL_VALUES: &str = "AbcCdDeEFhHKmoPQrTtuUwxXyYz";
const WGET_VALUES: &str = "oaiBtTeOPUwQlRA";
const TAR_VALUES: &str = "bCfFgILNTVX";
const SORT_VALUES: &str = "kStT";
const UNZIP_VALUES: &str = "dxPOI";

/// Runs inside the hook process. Prints a deny, or nothing so Claude Code's own permissions decide.
pub fn run() {
    let tmpdir = std::env::var_os("TMPDIR").map(PathBuf::from);
    if let Some(output) = decide(&crate::hooks::hook_payload(), &crate::machine::home(), tmpdir.as_deref(), &temp_roots()) {
        println!("{output}");
    }
}

/// The hook's stdout for one PreToolUse payload: a deny for a Bash command that changes a file or
/// the checkout, else `None`. `tmpdir` is the hook's `$TMPDIR` when it is set, and `temp` holds the
/// temp folders, resolved.
pub fn decide(payload: &str, home: &Path, tmpdir: Option<&Path>, temp: &[PathBuf]) -> Option<String> {
    let hook: Value = serde_json::from_str(payload).ok()?;
    if hook["tool_name"] != "Bash" {
        return None;
    }
    let command = hook["tool_input"]["command"].as_str()?;
    let cwd = PathBuf::from(hook["cwd"].as_str()?);
    let guard = Guard {
        home,
        // A relative `$TMPDIR` names no folder that the shell can be sure of.
        tmpdir: tmpdir.filter(|dir| dir.is_absolute()).map(|dir| dir.to_string_lossy().into_owned()),
        temp,
        session: hook["session_id"].as_str().unwrap_or_default().to_ascii_lowercase(),
    };
    let found = guard.line(command, Some(cwd), 0)?;
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": format!("{REASON} ({found})") } }).to_string())
}

/// `$TMPDIR`, `/tmp` and `/private/tmp`, resolved. Claude Code keeps each session's scratch folder
/// below one of them.
fn temp_roots() -> Vec<PathBuf> {
    // A relative or empty `$TMPDIR` would make every relative path temporary.
    [std::env::temp_dir(), "/tmp".into(), "/private/tmp".into()].into_iter().filter(|root| root.is_absolute()).map(|root| walk(&root)).collect()
}

struct Guard<'a> {
    home: &'a Path,
    /// The hook's `$TMPDIR`, when it is absolute.
    tmpdir: Option<String>,
    temp: &'a [PathBuf],
    /// The session's id, lowercase: its scratch folder is the one below a temp folder named for it.
    session: String,
}

impl Guard<'_> {
    /// What the first change on `line`, run from `cwd`, is.
    fn line(&self, line: &str, cwd: Option<PathBuf>, nest: u8) -> Option<String> {
        if nest > 8 {
            return None;
        }
        // The folder at each depth of parentheses: a `cd` inside doesn't leave them.
        let mut folders = vec![cwd];
        for seg in lex(line, &self.home.to_string_lossy(), None, self.tmpdir.as_deref()) {
            let outer = folders.last().cloned().flatten();
            folders.resize(seg.depth + 1, outer);
            let cwd = folders[seg.depth].clone();
            let redirected: Vec<Word> = seg.redirs.iter().filter(|r| r.write).map(|r| r.target.clone()).collect();
            let cmds = commands(&seg.words, true);
            let shell = cmds.iter().any(|c| SHELLS.contains(&c.prog.as_str()));
            let found = self
                .change(&cwd, "redirect", &redirected)
                .or_else(|| cmds.first().filter(|c| c.var).map(|_| "program from a variable".to_owned()))
                .or_else(|| cmds.iter().find_map(|c| self.command(&cwd, c, nest)))
                .or_else(|| seg.heredoc.iter().filter(|_| shell).find_map(|body| self.line(body, cwd.clone(), nest + 1)))
                .or_else(|| stdin_script(&seg, &cmds))
                .or_else(|| git_env(&seg, &cmds));
            if found.is_some() {
                return found;
            }
            if let Some(c) = cmds.first() {
                if let Some(to) = chdir(&cwd, c, self.home) {
                    // `cd /tmp/$name` could lead anywhere.
                    let unknown = c.positionals().iter().any(|w| w.var_at.is_some());
                    folders[seg.depth] = to.filter(|_| !unknown);
                }
            }
        }
        None
    }

    /// What `c`, run from `cwd`, changes.
    fn command(&self, cwd: &Option<PathBuf>, c: &Cmd, nest: u8) -> Option<String> {
        if let Some(found) = c.scripts().iter().find_map(|script| self.line(script, cwd.clone(), nest + 1)) {
            return Some(found);
        }
        // `env -S 'a b c'` splits its one string into a command line.
        if c.prog == "env" {
            if let Some(script) = flag(c, 'S', true, "", "split-string").flatten() {
                return self.line(&script.text, cwd.clone(), nest + 1);
            }
        }
        match c.prog.as_str() {
            "git" => self.git(cwd, c),
            "gh" => self.gh(cwd, c),
            "find" => self.find(cwd, c, nest),
            "patch" => Some("patch".to_owned()),
            "cargo" => cargo(c),
            "npm" | "pnpm" | "yarn" => package_manager(c),
            "ln" => self.ln(cwd, c),
            prog => {
                let files = written(c)?;
                if c.stdin_args {
                    return Some(format!("{prog} takes its files from a pipe"));
                }
                self.change(cwd, prog, &files)
            }
        }
    }

    /// `what` changing the first of `files` that is neither a device nor temporary.
    fn change(&self, cwd: &Option<PathBuf>, what: &str, files: &[Word]) -> Option<String> {
        files.iter().find(|w| !self.exempt(cwd, w)).map(|w| format!("{what} changes {}", w.text))
    }

    /// Whether `w`, named from `cwd`, is a device, or a place known to be below a temp folder before
    /// the shell runs, and not a pinned or Claude file in it.
    fn exempt(&self, cwd: &Option<PathBuf>, w: &Word) -> bool {
        if DEVICES.contains(&w.text.as_str()) {
            return true;
        }
        // A `$` anywhere: a value stuck to its flag (`--output=$OUT`) carries no mark of its own.
        if w.var_at.is_some() || w.text.contains('$') || w.glob_at.is_some_and(|at| w.text[at..].split('/').any(|part| part == "..")) {
            return false;
        }
        // What a glob matches is below the folder before it, which may be a temp folder itself.
        let glob = w.glob_at.is_some();
        let Some(path) = tgt(cwd, w).path else { return false };
        // The deepest temp folder above the path: what lies below it is the part the write names.
        let Some(root) = self.temp.iter().filter(|root| path.starts_with(root) && (glob || path != **root)).max_by_key(|root| root.components().count()) else {
            return false;
        };
        let below: Vec<String> = path.strip_prefix(root).map(|rest| rest.components().map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase()).collect()).unwrap_or_default();
        if !self.session.is_empty() && below.iter().any(|part| *part == self.session) {
            return true;
        }
        !below.iter().any(|part| part.starts_with("pinned") || part.starts_with("claude"))
    }

    /// A git command that changes the checkout, its history or its refs, or that runs a command. Only
    /// the reads in [`GIT_READS`] run, and the few subcommands below that change nothing.
    fn git(&self, cwd: &Option<PathBuf>, c: &Cmd) -> Option<String> {
        let (_, sub, rest) = git_sub(c)?;
        // What comes before the subcommand applies to every one: `-c` and `--config-env` set config that
        // can name a command, and `--exec-path` a folder of commands.
        let before = &c.args[..c.args.len() - rest.args.len() - 1];
        if let Some(flag) = before.iter().map(|w| w.text.as_str()).find(|t| *t == "-c" || t.starts_with("--config-env") || t.starts_with("--exec-path=")) {
            return Some(format!("git {}", flag.split('=').next().unwrap_or(flag)));
        }
        // `git diff --output=FILE` writes where it is told.
        if let Some(found) = rest.long_value("output").and_then(|out| self.change(cwd, "git", &[out])) {
            return Some(found);
        }
        let positionals = rest.positionals();
        let first = positionals.first().map_or("", |w| w.text.as_str());
        let allowed = GIT_READS.contains(&sub)
            || match sub {
                "pull" => rest.has("", &["ff-only"]) && !rest.has("r", &["rebase"]),
                "branch" => !rest.has("dDmMfcCu", &["delete", "move", "force", "copy", "edit-description", "set-upstream-to", "unset-upstream", "create-reflog"]) && (positionals.is_empty() || listing(&rest, "lrav")),
                "tag" => !rest.has("dafsmFeu", &["delete", "annotate", "sign", "force", "message", "file", "edit", "local-user", "create-reflog"]) && (positionals.is_empty() || listing(&rest, "ln")),
                "stash" => matches!(first, "list" | "show"),
                "worktree" => first == "list",
                "remote" => positionals.is_empty() || matches!(first, "show" | "get-url"),
                "config" => rest.has("l", &["get", "get-all", "get-regexp", "list"]) && !rest.has("e", &["edit", "add", "unset", "unset-all", "replace-all", "rename-section", "remove-section"]),
                "reflog" => positionals.is_empty() || first == "show",
                "notes" => positionals.is_empty() || matches!(first, "list" | "show"),
                _ => false,
            };
        (!allowed).then(|| format!("git {sub}"))
    }

    /// `gh` changes the checkout with `pr checkout`, `repo clone`, `repo fork --clone` and `run
    /// download`, which writes into the folder it runs in unless `-D` names another.
    fn gh(&self, cwd: &Option<PathBuf>, c: &Cmd) -> Option<String> {
        let mut words = vec![];
        let mut args = c.args.iter();
        while let Some(w) = args.next() {
            match w.text.as_str() {
                "-R" | "--repo" => {
                    args.next();
                }
                text if text.starts_with('-') => {}
                text => words.push(text),
            }
        }
        match words.as_slice() {
            ["pr", "checkout", ..] => Some("gh pr checkout".to_owned()),
            ["repo", "clone", ..] => Some("gh repo clone".to_owned()),
            ["repo", "fork", ..] if c.has("", &["clone"]) => Some("gh repo fork --clone".to_owned()),
            ["run", "download", ..] => {
                let dir = c.value('D', "dir").unwrap_or_else(here);
                self.change(cwd, "gh run download", &[dir])
            }
            _ => None,
        }
    }

    /// `find` changes what it matches below its start folders, with `-delete` or a command it runs, and
    /// the file its `-fprint`, `-fprint0`, `-fls` and `-fprintf` actions write (the first word after each).
    fn find(&self, cwd: &Option<PathBuf>, c: &Cmd, nest: u8) -> Option<String> {
        let paths = c.args.iter().skip_while(|w| matches!(w.text.as_str(), "-H" | "-L" | "-P"));
        let mut starts: Vec<Word> = paths.take_while(|w| !(w.text.starts_with('-') || w.text == "(" || w.text == "!")).cloned().collect();
        if starts.is_empty() {
            starts.push(Word { text: ".".into(), ..Word::default() });
        }
        let outputs: Vec<Word> = c
            .args
            .iter()
            .enumerate()
            .filter_map(|(k, w)| match w.text.as_str() {
                "-fprint" | "-fprint0" | "-fls" | "-fprintf" => c.args.get(k + 1).cloned(),
                _ => None,
            })
            .collect();
        if let Some(found) = self.change(cwd, "find", &outputs) {
            return Some(found);
        }
        if c.args.iter().any(|w| w.text == "-delete") {
            if let Some(found) = self.change(cwd, "find", &starts) {
                return Some(found);
            }
        }
        for (k, w) in c.args.iter().enumerate() {
            if !matches!(w.text.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                continue;
            }
            let inner = c.args[k + 1..].iter().take_while(|w| !matches!(w.text.as_str(), ";" | "+"));
            for start in &starts {
                // A `{}` stands for what `find` matched below this start.
                let words: Vec<Word> = inner.clone().map(|w| if w.text.contains("{}") { start.clone() } else { w.clone() }).collect();
                if let Some(found) = commands(&words, true).iter().find_map(|c| self.command(cwd, c, nest)) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// `ln` writes its destination, and without `-s` a hard link to a file outside the temp folders,
    /// which the link then changes.
    fn ln(&self, cwd: &Option<PathBuf>, c: &Cmd) -> Option<String> {
        let target = c.value('t', "target-directory");
        let mut sources: Vec<Word> = c.positionals().into_iter().cloned().collect();
        match &target {
            Some(dir) => sources.retain(|w| w.text != dir.text),
            None => {
                sources.pop();
            }
        }
        if !c.has("s", &["symbolic"]) {
            if let Some(w) = sources.iter().find(|w| !self.exempt(cwd, w)) {
                return Some(format!("ln links {}", w.text));
            }
        }
        self.change(cwd, "ln", &c.write_args())
    }
}

/// The git options that list rather than change: the `-l`, `--list`, `--contains` and the like.
fn listing(rest: &Cmd, shorts: &str) -> bool {
    rest.has(shorts, &["list", "show-current", "contains", "no-contains", "merged", "no-merged", "points-at", "sort", "format", "column"])
}

/// Whether a shell reads its script from a pipe or a heredoc: with no `-c` and no script file, the
/// script is not in the line.
fn stdin_script(seg: &Segment, cmds: &[Cmd]) -> Option<String> {
    let from = if seg.piped {
        "a pipe"
    } else if !seg.heredoc.is_empty() {
        "a heredoc"
    } else {
        return None;
    };
    cmds.iter().find(|c| SHELLS.contains(&c.prog.as_str()) && c.scripts().is_empty() && c.positionals().is_empty()).map(|c| format!("{} reads a script from {from}", c.prog))
}

/// A git command in a segment that sets a variable which runs a command for git.
fn git_env(seg: &Segment, cmds: &[Cmd]) -> Option<String> {
    if !cmds.iter().any(|c| c.prog == "git") {
        return None;
    }
    seg.words.iter().map(|w| w.text.as_str()).find(|text| GIT_HAZARD_ENV.iter().any(|name| text.starts_with(name))).map(|text| format!("git with {}", text.split('=').next().unwrap_or(text)))
}

/// Whether a cargo command changes `Cargo.toml` or `Cargo.lock`, or rewrites the sources: a format or
/// a fix that writes, not one that checks.
fn cargo(c: &Cmd) -> Option<String> {
    let words: Vec<&str> = c.positionals().into_iter().map(|w| w.text.as_str()).filter(|text| !text.starts_with('+')).collect();
    let sub = *words.first()?;
    let flag_given = |flag: &str| c.args.iter().any(|w| w.text == flag);
    let changes = match sub {
        "fmt" => !flag_given("--check"),
        "clippy" => flag_given("--fix"),
        "fix" | "update" | "add" | "remove" | "generate-lockfile" => true,
        _ => false,
    };
    changes.then(|| format!("cargo {sub}"))
}

/// Whether an npm, pnpm or yarn command changes `package.json`, a lockfile or the packages installed.
fn package_manager(c: &Cmd) -> Option<String> {
    let words: Vec<&str> = c.positionals().into_iter().map(|w| w.text.as_str()).collect();
    let sub = *words.first()?;
    let changes = match c.prog.as_str() {
        "npm" => match sub {
            "install" | "i" | "add" | "update" | "up" | "upgrade" | "uninstall" | "un" | "unlink" | "remove" | "rm" | "r" => true,
            "audit" => words.get(1) == Some(&"fix") || c.has("", &["fix"]),
            "pkg" => words.get(1) == Some(&"set"),
            _ => false,
        },
        "pnpm" => matches!(sub, "install" | "i" | "add" | "update" | "up" | "upgrade" | "remove" | "rm" | "un" | "uninstall"),
        "yarn" => matches!(sub, "add" | "install" | "upgrade" | "remove"),
        _ => false,
    };
    changes.then(|| format!("{} {sub}", c.prog))
}

/// The files `c` writes, when its program changes the files it names, and `None` for any other call.
fn written(c: &Cmd) -> Option<Vec<Word>> {
    let named = || c.positionals().into_iter().cloned().collect::<Vec<_>>();
    match c.prog.as_str() {
        "rm" | "rmdir" | "unlink" | "shred" => Some(named()),
        "cp" | "mv" | "install" | "tee" | "dd" => Some(c.write_args()),
        "touch" => Some(without_values(c, "tdrA", &["date", "reference"]).write_args()),
        "truncate" => Some(without_values(c, "sr", &["size", "reference"]).write_args()),
        "mkdir" => Some(without_values(c, "m", &["mode"]).write_args()),
        "sed" => c.has("i", &["in-place"]).then(|| without_values(c, "ef", &["expression", "file"]).write_args()),
        "perl" => perl_in_place(c),
        "chmod" => {
            // The first word is the mode, unless a flag gives it (`-x`) or `--reference` does.
            let flagged = c.args.iter().any(|w| w.text.starts_with("--reference") || w.text.strip_prefix('-').is_some_and(|mode| !mode.is_empty() && mode.chars().all(|m| "rwxXst,+=ugo".contains(m))));
            Some(named().into_iter().skip(usize::from(!flagged)).collect())
        }
        "rsync" => {
            let mut files = named();
            let destination = files.pop()?;
            (!files.is_empty()).then(|| vec![destination])
        }
        "sort" => Some(flag(c, 'o', true, SORT_VALUES, "output").flatten().into_iter().collect()),
        // Extraction writes the members into `-C`, or into the folder it runs in.
        "tar" => tar_extracts(c).then(|| vec![flag(c, 'C', true, TAR_VALUES, "directory").flatten().unwrap_or_else(here)]),
        "unzip" => (!c.has("lpcZtv", &[])).then(|| vec![flag(c, 'd', true, UNZIP_VALUES, "").flatten().unwrap_or_else(here)]),
        // Compressors rewrite each file they name, unless the output goes to stdout or they only test or list.
        "gzip" | "gunzip" | "bzip2" | "xz" => {
            if c.has("ctl", &["stdout", "to-stdout", "test", "list"]) {
                return None;
            }
            Some(named().into_iter().filter(|w| w.text != "-").collect())
        }
        "zip" => Some(named().into_iter().take(1).collect()),
        "curl" => {
            let mut files: Vec<Word> = [flag(c, 'o', true, CURL_VALUES, "output"), flag(c, 'D', true, CURL_VALUES, "dump-header"), flag(c, 'c', true, CURL_VALUES, "cookie-jar")].into_iter().flatten().flatten().collect();
            // `-O` and `-J` save under a name the server gives, in the folder the command runs in.
            let by_name = flag(c, 'O', false, CURL_VALUES, "remote-name").is_some() || flag(c, 'O', false, CURL_VALUES, "remote-name-all").is_some() || flag(c, 'J', false, CURL_VALUES, "remote-header-name").is_some();
            if by_name {
                files.push(here());
            }
            downloaded(files)
        }
        "wget" => match flag(c, 'O', true, WGET_VALUES, "output-document") {
            Some(out) => downloaded(out.into_iter().collect()),
            None => Some(vec![flag(c, 'P', true, WGET_VALUES, "directory-prefix").flatten().unwrap_or_else(here)]),
        },
        _ => None,
    }
}

/// What a downloader told to write `files` writes: `-` is its standard output.
fn downloaded(mut files: Vec<Word>) -> Option<Vec<Word>> {
    files.retain(|w| w.text != "-");
    (!files.is_empty()).then_some(files)
}

/// Whether tar extracts: `-x`, `--extract` or `--get`, or the first word without a dash read the old
/// way (`tar xzf file`).
fn tar_extracts(c: &Cmd) -> bool {
    let old_style = c.args.first().is_some_and(|w| !w.text.starts_with('-') && w.text.contains('x'));
    old_style || flag(c, 'x', false, TAR_VALUES, "extract").is_some() || c.has("", &["get"])
}

/// The folder a command runs in, as the word that names it.
fn here() -> Word {
    Word { text: ".".into(), ..Word::default() }
}

/// A flag, read the way a shell reads it: `-o file`, `-ofile`, `-qofile`, `--output=file` or `--output
/// file`. `Some(None)` is the flag with no value, or one that takes none; `None` is the flag absent.
/// A letter after a flag in `takers` is that flag's value, so `-XOPTS` has no `O`.
fn flag(c: &Cmd, short: char, takes: bool, takers: &str, long: &str) -> Option<Option<Word>> {
    let mut args = c.args.iter();
    while let Some(w) = args.next() {
        let text = w.text.as_str();
        if text == "--" {
            break;
        }
        if let Some(name) = text.strip_prefix("--") {
            let (key, value) = name.split_once('=').map_or((name, None), |(key, value)| (key, Some(value)));
            if !long.is_empty() && key == long {
                return Some(match value {
                    Some(value) => Some(Word { text: value.into(), ..Word::default() }),
                    None if takes => args.next().cloned(),
                    None => None,
                });
            }
        } else if text.len() > 1 && text.starts_with('-') {
            for (at, ch) in text[1..].char_indices() {
                if ch == short {
                    let rest = &text[1 + at + ch.len_utf8()..];
                    return Some(match (takes, rest.is_empty()) {
                        (false, _) => None,
                        (true, true) => args.next().cloned(),
                        (true, false) => Some(Word { text: rest.into(), ..Word::default() }),
                    });
                }
                if takers.contains(ch) {
                    break;
                }
            }
        }
    }
    None
}

/// `c` without the words that are a flag's value, such as the time after `touch -t`, and without
/// empty words, such as the suffix of BSD's `sed -i ''`: none of them is a file.
fn without_values(c: &Cmd, shorts: &str, longs: &[&str]) -> Cmd {
    let takes_value = |text: &str| match text.strip_prefix("--") {
        Some(long) => longs.contains(&long),
        None => text.len() > 1 && text.starts_with('-') && text.ends_with(|flag| shorts.contains(flag)),
    };
    let mut value_next = false;
    let args = c.args.iter().filter(|w| !std::mem::replace(&mut value_next, takes_value(&w.text)) && !w.text.is_empty()).cloned().collect();
    Cmd { prog: c.prog.clone(), args, stdin_args: c.stdin_args, var: c.var }
}

/// The files `perl -i` rewrites: the words after its switches.
fn perl_in_place(c: &Cmd) -> Option<Vec<Word>> {
    let mut in_place = false;
    let mut at = 0;
    while let Some(switches) = c.args.get(at).and_then(|w| w.text.strip_prefix('-')).filter(|s| !s.is_empty() && *s != "-") {
        at += 1;
        for (k, switch) in switches.char_indices() {
            match switch {
                'i' => in_place = true,
                // `-e` takes the next word when no code is stuck to it.
                'e' | 'E' => at += usize::from(k + 1 == switches.len()),
                // The rest of the cluster is this switch's value.
                'M' | 'm' | 'I' | 'F' | 'x' | 'C' | 'd' | 'D' | 'V' => {}
                _ => continue,
            }
            break;
        }
    }
    in_place.then(|| c.args.iter().skip(at).filter(|w| w.text != "--").cloned().collect())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    struct Fx {
        _tmp: tempfile::TempDir,
        work: PathBuf,
        home: PathBuf,
        /// The one temp folder the fixture's guard knows, which is also its `$TMPDIR`.
        temp: PathBuf,
        /// A session's scratch folder, below the temp folder as Claude Code keeps it.
        scratch: PathBuf,
    }

    /// A checkout, a home, and a temp folder holding a scratch folder and a link back into the checkout.
    fn fx() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(tmp.path()).unwrap();
        let (work, home, temp) = (base.join("work"), base.join("home"), base.join("tmp"));
        let scratch = temp.join("claude-501").join(crate::machine::claude_slug(&work)).join("s1");
        for dir in [work.join("sub"), home.clone(), scratch.clone()] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::os::unix::fs::symlink(&work, temp.join("link")).unwrap();
        Fx { _tmp: tmp, work, home, temp, scratch }
    }

    /// `s` with each `{name}` a path of the fixture.
    fn sub(f: &Fx, s: &str) -> String {
        let show = |p: &Path| p.display().to_string();
        s.replace("{work}", &show(&f.work)).replace("{home}", &show(&f.home)).replace("{temp}", &show(&f.temp)).replace("{scratch}", &show(&f.scratch))
    }

    fn bash(f: &Fx, command: &str, mode: &str) -> String {
        json!({ "session_id": "s1", "tool_name": "Bash", "tool_input": { "command": command }, "cwd": f.work, "permission_mode": mode }).to_string()
    }

    /// The guard's output for one Bash command run in the checkout, with `$TMPDIR` the fixture's temp folder.
    fn guard(f: &Fx, command: &str) -> Option<String> {
        decide(&bash(f, command, "default"), &f.home, Some(&f.temp), std::slice::from_ref(&f.temp))
    }

    /// The decision and reason of a hook's output.
    fn held(out: Option<String>) -> Option<(String, String)> {
        let out: Value = serde_json::from_str(&out?).unwrap();
        let specific = &out["hookSpecificOutput"];
        assert_eq!(specific["hookEventName"], "PreToolUse");
        Some((specific["permissionDecision"].as_str().unwrap().to_owned(), specific["permissionDecisionReason"].as_str().unwrap().to_owned()))
    }

    fn denied(found: &str) -> Option<(String, String)> {
        Some(("deny".to_owned(), format!("pinned: this Captain is manage-only, so Bash is for reading and checking. Hand this change to a worker with delegate_task. ({found})")))
    }

    /// Each command, run in the checkout, is denied for what it is paired with.
    fn assert_denied(rows: &[(&str, &str)]) {
        let f = fx();
        for (command, found) in rows {
            let command = sub(&f, command);
            assert_eq!(held(guard(&f, &command)), denied(&sub(&f, found)), "{command}");
        }
    }

    fn assert_runs(commands: &[&str]) {
        let f = fx();
        for command in commands {
            let command = sub(&f, command);
            assert_eq!(guard(&f, &command), None, "{command}");
        }
    }

    #[test]
    fn a_command_that_writes_a_file_is_denied() {
        assert_denied(&[
            ("sed -i s/a/b/ x.rs", "sed changes x.rs"),
            ("cd sub && sed -i s/a/b/ x.rs", "sed changes x.rs"),
            ("sed --in-place -e s/a/b/ x.rs", "sed changes x.rs"),
            ("sed -i '' s/a/b/ x.rs", "sed changes x.rs"),
            ("perl -pi -e 's/a/b/' x.rs", "perl changes x.rs"),
            ("perl -i.bak -pe 's/a/b/' x.rs", "perl changes x.rs"),
            ("cargo test | tee out.log", "tee changes out.log"),
            ("cargo test | tee -a {temp}/t.log notes.txt", "tee changes notes.txt"),
            ("cp {temp}/a src/a.rs", "cp changes src/a.rs"),
            ("cp -t src {temp}/a", "cp changes src"),
            ("mv a.rs {temp}/a.rs", "mv changes a.rs"),
            ("install -m 755 {temp}/bin bin/tool", "install changes bin/tool"),
            ("rsync -a {temp}/x/ src/", "rsync changes src/"),
            ("touch new.rs", "touch changes new.rs"),
            ("truncate -s 0 log.txt", "truncate changes log.txt"),
            ("dd if=/dev/zero of=blob bs=1 count=1", "dd changes blob"),
            ("patch -p1 < {temp}/fix.diff", "patch"),
            ("rm x.rs", "rm changes x.rs"),
            ("rm -rf target", "rm changes target"),
            ("rmdir sub", "rmdir changes sub"),
            ("unlink x.rs", "unlink changes x.rs"),
            ("shred x.rs", "shred changes x.rs"),
            ("chmod +x run.sh", "chmod changes run.sh"),
            ("chmod -x run.sh", "chmod changes run.sh"),
            ("chmod -R 755 scripts", "chmod changes scripts"),
            ("mkdir -p src/new", "mkdir changes src/new"),
            ("curl -s https://acme.dev/x -o x.json", "curl changes x.json"),
            ("curl -sSLO https://acme.dev/x.tgz", "curl changes ."),
            ("wget -O x.json https://acme.dev/x", "wget changes x.json"),
            ("rm ~/notes", "rm changes {home}/notes"),
        ]);
    }

    #[test]
    fn output_redirected_into_a_file_is_denied() {
        assert_denied(&[
            ("echo hi > README.md", "redirect changes README.md"),
            ("echo hi >> README.md", "redirect changes README.md"),
            ("echo hi >| README.md", "redirect changes README.md"),
            ("cargo test &> test.log", "redirect changes test.log"),
            ("cargo test 2> err.log", "redirect changes err.log"),
            ("> {work}/a.rs", "redirect changes {work}/a.rs"),
            ("cat > a.rs <<'EOF'\nx\nEOF", "redirect changes a.rs"),
            ("cat <<EOF > a.rs\nx\nEOF", "redirect changes a.rs"),
            ("tr a b <<< hi > a.rs", "redirect changes a.rs"),
            ("cd {temp} && cd {work}/sub && echo x > f", "redirect changes f"),
        ]);
    }

    #[test]
    fn a_command_that_changes_the_checkout_or_its_history_is_denied() {
        assert_denied(&[
            ("git add -A && git commit -m x", "git add"),
            ("git push origin HEAD", "git push"),
            ("git apply {temp}/fix.diff", "git apply"),
            ("git am {temp}/x.patch", "git am"),
            ("git merge main", "git merge"),
            ("git rebase origin/main", "git rebase"),
            ("git cherry-pick abc", "git cherry-pick"),
            ("git revert abc", "git revert"),
            ("git reset --hard", "git reset"),
            ("git restore x.rs", "git restore"),
            ("git checkout main", "git checkout"),
            ("git checkout -- x.rs", "git checkout"),
            ("git switch -c feat", "git switch"),
            ("git stash", "git stash"),
            ("git stash pop", "git stash"),
            ("git stash push -m x", "git stash"),
            ("git clean -fd", "git clean"),
            ("git rm x.rs", "git rm"),
            ("git mv a b", "git mv"),
            ("git tag v1", "git tag"),
            ("git tag -a v1 -m done", "git tag"),
            ("git tag -d v1", "git tag"),
            ("git branch -d feat", "git branch"),
            ("git branch -D feat", "git branch"),
            ("git branch -m old new", "git branch"),
            ("git branch -f main HEAD", "git branch"),
            ("git branch --delete feat", "git branch"),
            ("git worktree add ../w", "git worktree"),
            ("git worktree remove ../w", "git worktree"),
            ("git pull", "git pull"),
            ("git pull --rebase", "git pull"),
            ("git -C sub commit -m x", "git commit"),
            ("cd {temp} && git -c user.name=a commit -m x", "git -c"),
            ("git diff --output=d.patch", "git changes d.patch"),
        ]);
    }

    #[test]
    fn a_change_is_found_wherever_the_line_runs_it() {
        assert_denied(&[
            ("x=$(git commit -m y)", "git commit"),
            ("echo \"at `rm x.rs`\"", "rm changes x.rs"),
            ("(cd sub; rm x.rs)", "rm changes x.rs"),
            ("cargo test; false || rm x.rs", "rm changes x.rs"),
            ("if true; then rm x.rs; fi", "rm changes x.rs"),
            ("env FOO=1 rm x.rs", "rm changes x.rs"),
            ("nice -n 5 git commit -m x", "git commit"),
            ("timeout 60 sed -i s/a/b/ x.rs", "sed changes x.rs"),
            ("ls | xargs rm", "rm takes its files from a pipe"),
            ("ls | xargs sed -i s/a/b/", "sed takes its files from a pipe"),
            ("find . -name '*.rs' -exec sed -i s/a/b/ {} +", "sed changes ."),
            ("find src -type f -exec rm {} \\;", "rm changes src"),
            ("find . -name '*.orig' -delete", "find changes ."),
            ("find {temp}/x sub -delete", "find changes sub"),
            ("bash -c 'git commit -m x'", "git commit"),
            ("zsh -lc \"echo hi > a.rs\"", "redirect changes a.rs"),
            ("eval 'rm x.rs'", "rm changes x.rs"),
            ("bash <<'EOF'\nrm x.rs\nEOF", "rm changes x.rs"),
            ("cat <<EOF\n$(rm x.rs)\nEOF", "rm changes x.rs"),
        ]);
    }

    #[test]
    fn a_target_that_cannot_be_known_to_be_temporary_is_denied() {
        assert_denied(&[
            ("cargo test > $LOG", "redirect changes $LOG"),
            ("cargo test > \"$OTHER/log\"", "redirect changes $OTHER/log"),
            ("rm {temp}/$name", "rm changes {temp}/$name"),
            ("curl --output=$OUT https://acme.dev", "curl changes $OUT"),
            ("cd {temp} && dd if=a of=$OUT", "dd changes $OUT"),
            ("cd $DIR && rm x.rs", "rm changes x.rs"),
            ("cd {temp}/$name && rm x.rs", "rm changes x.rs"),
            ("rm -rf {temp}", "rm changes {temp}"),
            ("rm {temp}/*/../../work/x.rs", "rm changes {temp}/*/../../work/x.rs"),
            ("rm {temp}/../work/x.rs", "rm changes {temp}/../work/x.rs"),
            ("echo x > {temp}/link/a.rs", "redirect changes {temp}/link/a.rs"),
        ]);
    }

    #[test]
    fn a_target_under_tmpdir_runs() {
        assert_runs(&[
            "cargo test > \"$TMPDIR/log.txt\"",
            "cargo test > $TMPDIR/log.txt",
            "cargo test > ${TMPDIR}/log.txt",
            "cargo test 2> \"$TMPDIR/x\"",
            "cd $TMPDIR && rm x.rs",
        ]);
        assert_denied(&[
            ("cargo test > \"$OTHER/log\"", "redirect changes $OTHER/log"),
            ("cargo test > '$TMPDIR/log'", "redirect changes $TMPDIR/log"),
            ("cargo test > \"$TMPDIR/../work/log\"", "redirect changes {temp}/../work/log"),
        ]);
    }

    #[test]
    fn reading_checking_and_merging_run() {
        assert_runs(&[
            "cat a.rs",
            "head -5 a.rs",
            "ls -la",
            "grep -rn 'rm -rf' .",
            "rg 'git commit' src",
            "jq .name package.json",
            "sed -n 1,5p a.rs",
            "perl -ne 'print if /x/' a.rs",
            "perl -MList::Util -e 1 a.rs",
            "echo '$(rm x.rs)'",
            "cat <<'EOF'\nrm x.rs\nEOF",
            "git log --oneline -5",
            "git diff origin/main...HEAD",
            "git show HEAD",
            "git status -sb",
            "git fetch origin",
            "git pull --ff-only",
            "git stash list",
            "git stash show -p",
            "git tag",
            "git tag -l 'v*'",
            "git tag --sort=-creatordate --format '%(refname)'",
            "git branch -a",
            "git branch -vv --contains abc",
            "git worktree list",
            "git merge-base origin/main HEAD",
            "gh pr view 1 --json headRefOid",
            "gh pr merge 1 --merge --match-head-commit abc",
            "gh pr ready 1",
            "gh api -X PATCH repos/acme/site/pulls/1 -f body=done",
            "cargo test --locked -p pinned-core",
            "cargo build 2>&1 | tail -3",
            "cargo run -q -p pinned-layout",
            "cargo check && cargo clippy",
            "node --test ui/gui.test.js",
            "npm test",
            "curl -s https://acme.dev",
            "curl -s -XPOST https://acme.dev -d a=1",
            "curl -o - https://acme.dev",
            "wget -qO- https://acme.dev",
            "ls | xargs grep -n main",
            "find . -name '*.rs' -exec grep -l main {} +",
        ]);
    }

    #[test]
    fn a_write_that_stays_in_a_device_the_scratch_folder_or_a_temp_folder_runs() {
        assert_runs(&[
            "echo x > /dev/null",
            "cargo test > /dev/stdout 2> /dev/stderr",
            "curl -so /dev/null -w '%{http_code}' https://acme.dev",
            "cargo test 2> {temp}/x/log",
            "gh pr checks 1 > {scratch}/c.txt",
            "git diff > {temp}/d.patch && wc -l {temp}/d.patch",
            "mkdir -p {temp}/x && cd {temp}/x && echo hi > f && rm f",
            "cp src/a.rs {temp}/a.rs",
            "cargo test | tee {temp}/t.log",
            "sed -i -e s/a/b/ {temp}/f",
            "sed -i '' s/a/b/ {temp}/f",
            "perl -pi -e 's/a/b/' {temp}/f",
            "truncate -s 0 {temp}/log",
            "touch -t 202601010000 {temp}/f",
            "mkdir -m 700 {temp}/d",
            "chmod 600 {scratch}/c.txt",
            "rm -rf {temp}/x/*",
            "find {temp}/x -name '*.log' -exec rm {} +",
            "find {temp}/x -name '*.log' -delete",
        ]);
    }

    #[test]
    fn the_temp_folders_are_tmpdir_tmp_and_private_tmp() {
        let f = fx();
        let payload = |command: String| json!({ "tool_name": "Bash", "tool_input": { "command": command }, "cwd": "/nowhere/checkout" }).to_string();
        let roots = temp_roots();
        let tmpdir = std::env::temp_dir();
        let log = tmpdir.join("x/log").display().to_string();
        for target in ["/tmp/x/log", "/private/tmp/x/log", log.as_str()] {
            assert_eq!(decide(&payload(format!("cargo test 2> {target}")), &f.home, Some(&tmpdir), &roots), None, "{target}");
        }
        for command in ["rm -rf /tmp", "echo x > /nowhere/checkout/a.rs", "echo x > a.rs"] {
            assert!(decide(&payload(command.to_owned()), &f.home, Some(&tmpdir), &roots).is_some(), "{command}");
        }
    }

    #[test]
    fn every_mode_is_denied_and_only_bash_calls_are_read() {
        let f = fx();
        let temp = std::slice::from_ref(&f.temp);
        for mode in ["default", "acceptEdits", "plan", "bypassPermissions", "auto", "dontAsk"] {
            assert_eq!(held(decide(&bash(&f, "git commit -m x", mode), &f.home, Some(&f.temp), temp)), denied("git commit"), "{mode}");
        }
        for payload in [
            json!({ "tool_name": "Write", "tool_input": { "file_path": f.work.join("a.rs"), "content": "x" }, "cwd": f.work }).to_string(),
            json!({ "tool_name": "Bash", "tool_input": { "command": "rm x.rs" } }).to_string(),
            "not json".to_owned(),
        ] {
            assert_eq!(decide(&payload, &f.home, Some(&f.temp), temp), None, "{payload}");
        }
    }

    #[test]
    fn gh_that_changes_the_checkout_is_denied() {
        assert_denied(&[
            ("gh pr checkout 12", "gh pr checkout"),
            ("gh -R acme/site pr checkout 12", "gh pr checkout"),
            ("gh repo clone acme/site", "gh repo clone"),
            ("gh repo fork acme/site --clone", "gh repo fork --clone"),
            ("gh run download 123", "gh run download changes ."),
            ("gh run download 123 -D {work}/sub", "gh run download changes {work}/sub"),
        ]);
        assert_runs(&[
            "gh pr merge 1 --merge --match-head-commit abc",
            "gh pr ready 1",
            "gh pr checks 1 --watch",
            "gh api repos/acme/site/pulls/1",
            "gh run download 123 -D {temp}/run",
            "gh run view 123 --log",
            "gh repo view acme/site",
        ]);
    }

    #[test]
    fn git_outside_the_read_only_allowlist_is_denied() {
        assert_denied(&[
            ("git -c core.pager=evil log", "git -c"),
            ("git --exec-path=/tmp/evil status", "git --exec-path"),
            ("git --config-env=core.pager=EVIL log", "git --config-env"),
            ("GIT_EXTERNAL_DIFF=evil git diff HEAD~1", "git with GIT_EXTERNAL_DIFF"),
            ("GIT_SSH_COMMAND='sh -c evil' git fetch origin", "git with GIT_SSH_COMMAND"),
            ("GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.pager git log", "git with GIT_CONFIG_COUNT"),
            ("env GIT_EXTERNAL_DIFF=evil git log -p", "git with GIT_EXTERNAL_DIFF"),
            ("git gc", "git gc"),
            ("git add -A", "git add"),
            ("git bisect start", "git bisect"),
            ("git submodule update", "git submodule"),
            ("git archive HEAD", "git archive"),
            ("git format-patch -1", "git format-patch"),
            ("git update-ref refs/heads/x HEAD", "git update-ref"),
            ("git symbolic-ref HEAD refs/heads/x", "git symbolic-ref"),
            ("git branch newtopic", "git branch"),
            ("git branch -a -d feat", "git branch"),
            ("git branch -u origin/main", "git branch"),
            ("git remote add up {temp}/x", "git remote"),
            ("git remote -v add up x", "git remote"),
            ("git config user.name x", "git config"),
            ("git config --unset a.b", "git config"),
            ("git config --get a.b --add c", "git config"),
            ("git stash drop", "git stash"),
            ("git stash apply", "git stash"),
            ("git worktree prune", "git worktree"),
            ("git reflog expire --all", "git reflog"),
            ("git notes add -m x", "git notes"),
            ("git pull origin main", "git pull"),
            ("git pull -r --ff-only", "git pull"),
            ("git pull --ff-only --rebase", "git pull"),
            ("git tag -a v1", "git tag"),
            ("git tag -l -d v1", "git tag"),
        ]);
        assert_runs(&[
            "git log --oneline -5",
            "git log -c -p HEAD",
            "git show -c HEAD",
            "git -C sub status",
            "git diff origin/main...HEAD",
            "git fetch origin",
            "git pull --ff-only",
            "git branch --show-current",
            "git branch",
            "git branch -a",
            "git branch -vv --contains abc",
            "git branch --list 'feat*'",
            "git tag",
            "git tag -l 'v*'",
            "git tag --sort=-creatordate --format '%(refname)'",
            "git remote",
            "git remote -v",
            "git remote show origin",
            "git remote get-url origin",
            "git config --get user.name",
            "git config --get-regexp 'alias.*'",
            "git config -l",
            "git reflog",
            "git reflog show main",
            "git stash list",
            "git stash show -p",
            "git worktree list",
            "git notes list",
            "git notes show HEAD",
            "git rev-parse HEAD",
            "git ls-remote origin",
            "git cat-file -p HEAD",
            "git check-ignore x",
            "git version",
            "git blame src/a.rs",
            "git describe --tags",
            "git for-each-ref refs/heads",
            "git rev-list --count HEAD",
            "git merge-base origin/main HEAD",
            "git whatchanged -1",
            "git range-diff a...b",
            "git ls-files -o",
            "git grep -n main",
        ]);
    }

    #[test]
    fn cargo_and_package_commands_that_change_files_are_denied() {
        assert_denied(&[
            ("cargo fmt", "cargo fmt"),
            ("cargo fmt --all", "cargo fmt"),
            ("cargo +stable fmt", "cargo fmt"),
            ("cargo clippy --fix --allow-dirty", "cargo clippy"),
            ("cargo fix --allow-dirty", "cargo fix"),
            ("cargo update", "cargo update"),
            ("cargo update -p serde", "cargo update"),
            ("cargo add serde", "cargo add"),
            ("cargo remove serde", "cargo remove"),
            ("cargo generate-lockfile", "cargo generate-lockfile"),
            ("npm install", "npm install"),
            ("npm i lodash", "npm i"),
            ("npm install --save-dev x", "npm install"),
            ("npm update", "npm update"),
            ("npm uninstall x", "npm uninstall"),
            ("npm audit fix", "npm audit"),
            ("npm pkg set version=1.0.0", "npm pkg"),
            ("pnpm install", "pnpm install"),
            ("pnpm add x", "pnpm add"),
            ("yarn add x", "yarn add"),
            ("yarn install", "yarn install"),
            ("yarn upgrade", "yarn upgrade"),
            ("yarn remove x", "yarn remove"),
        ]);
        assert_runs(&[
            "cargo fmt --all -- --check",
            "cargo fmt -- --check",
            "cargo clippy -- -D warnings",
            "cargo clippy --locked -p pinned-core",
            "cargo test --locked -p pinned-core",
            "cargo build 2>&1 | tail -3",
            "cargo check && cargo clippy",
            "cargo run -q -p pinned-layout",
            "cargo tree",
            "npm ci",
            "npm test",
            "npm run lint",
            "npm audit",
            "npm pkg get version",
            "npm ls",
            "yarn test",
            "yarn run build",
        ]);
    }

    #[test]
    fn a_write_into_a_pinned_or_claude_file_in_temp_is_denied() {
        assert_denied(&[
            ("echo x > {temp}/x/pinned-1-abc/claude-settings-captain.json", "redirect changes {temp}/x/pinned-1-abc/claude-settings-captain.json"),
            ("echo x > {temp}/Pinned-123/log", "redirect changes {temp}/Pinned-123/log"),
            ("cargo test > {temp}/claude-run.log", "redirect changes {temp}/claude-run.log"),
            ("cp src/a.rs {temp}/CLAUDE/a.rs", "cp changes {temp}/CLAUDE/a.rs"),
            ("rm -rf {temp}/pinned-1", "rm changes {temp}/pinned-1"),
        ]);
        assert_runs(&[
            "echo x > {temp}/x/log",
            "cargo test > {temp}/x/cargo-pinned.log",
            "echo x > {scratch}/pinned-notes.txt",
            "gh pr checks 1 > {scratch}/c.txt",
        ]);
    }

    #[test]
    fn a_hard_link_to_a_file_outside_temp_is_denied() {
        assert_denied(&[
            ("ln {work}/sub/a.rs {temp}/a.rs", "ln links {work}/sub/a.rs"),
            ("ln sub/a.rs {temp}/a.rs", "ln links sub/a.rs"),
            ("ln -t {temp} sub/a.rs", "ln links sub/a.rs"),
            ("ln sub/a.rs b.rs", "ln links sub/a.rs"),
        ]);
        assert_runs(&[
            "ln -s sub/a.rs {temp}/alias",
            "ln {temp}/a {temp}/b",
            "ln {scratch}/a {temp}/b",
        ]);
    }

    #[test]
    fn a_shell_that_reads_its_script_from_a_pipe_or_heredoc_is_denied() {
        assert_denied(&[
            ("echo 'git commit -m x' | sh", "sh reads a script from a pipe"),
            ("cat <<EOF | sh\ngit commit -m x\nEOF", "sh reads a script from a pipe"),
            ("sh <<'EOF'\necho hi\nEOF", "sh reads a script from a heredoc"),
            ("ls | xargs bash", "bash reads a script from a pipe"),
        ]);
        assert_runs(&[
            "echo 'echo hi' | sh -c 'cat'",
            "sh scripts/build.sh",
            "bash -c 'echo hi'",
            "echo hi | cat",
            "cat <<EOF > {temp}/x\nhi\nEOF",
        ]);
    }

    #[test]
    fn writers_the_guard_missed_are_denied() {
        assert_denied(&[
            ("sort -o sorted.txt input.txt", "sort changes sorted.txt"),
            ("sort --output=sorted.txt input.txt", "sort changes sorted.txt"),
            ("tar -xzf release.tgz", "tar changes ."),
            ("tar xzf release.tgz", "tar changes ."),
            ("tar --extract -f release.tgz -C src", "tar changes src"),
            ("tar -xf release.tar -C {work}/sub", "tar changes {work}/sub"),
            ("unzip bundle.zip", "unzip changes ."),
            ("unzip bundle.zip -d src", "unzip changes src"),
            ("gzip notes.txt", "gzip changes notes.txt"),
            ("gunzip notes.txt.gz", "gunzip changes notes.txt.gz"),
            ("bzip2 -9 data", "bzip2 changes data"),
            ("xz -d file.xz", "xz changes file.xz"),
            ("zip out.zip a.txt", "zip changes out.zip"),
            ("wget https://acme.dev/x.tgz", "wget changes ."),
            ("wget -P src https://acme.dev/x.tgz", "wget changes src"),
            ("curl -sD headers.txt https://acme.dev", "curl changes headers.txt"),
            ("curl --dump-header h.txt https://acme.dev", "curl changes h.txt"),
            ("curl -c jar.txt https://acme.dev", "curl changes jar.txt"),
            ("curl --cookie-jar jar.txt https://acme.dev", "curl changes jar.txt"),
            ("curl -J https://acme.dev/x", "curl changes ."),
            ("curl -OJ https://acme.dev/x", "curl changes ."),
            ("curl --remote-name https://acme.dev/x", "curl changes ."),
            ("find . -name '*.rs' -fprint list.txt", "find changes list.txt"),
            ("find . -fprint0 names.bin", "find changes names.bin"),
            ("find . -fls ls.txt", "find changes ls.txt"),
            ("find . -fprintf out.txt '%p\\n'", "find changes out.txt"),
        ]);
        assert_runs(&[
            "sort input.txt",
            "sort -o {temp}/sorted.txt input.txt",
            "tar -tzf release.tgz",
            "tar -xzf {temp}/release.tgz -C {temp}/x",
            "unzip -l bundle.zip",
            "unzip -p bundle.zip README.md",
            "unzip bundle.zip -d {temp}/x",
            "gzip -c notes.txt > {temp}/notes.gz",
            "gzip -t notes.gz",
            "gunzip -c notes.gz",
            "xz -dc file.xz",
            "wget -O - https://acme.dev",
            "wget -qO- https://acme.dev",
            "wget -P {temp}/x https://acme.dev/x",
            "curl -sD - https://acme.dev",
            "curl -c {temp}/jar https://acme.dev",
            "curl -s -XPOST https://acme.dev -d a=1",
            "find . -name '*.rs' -fprint {temp}/files",
            "find . -fprint /dev/null",
        ]);
    }

    #[test]
    fn a_program_name_is_compared_without_case_and_a_program_from_a_variable_is_denied() {
        assert_denied(&[
            ("GIT commit -m x", "git commit"),
            ("Git push origin HEAD", "git push"),
            ("RM x.rs", "rm changes x.rs"),
            ("$CMD x.rs", "program from a variable"),
            ("$(which rm) x.rs", "program from a variable"),
            ("`which rm` x.rs", "program from a variable"),
            ("$BIN/rm x.rs", "program from a variable"),
            ("time -p rm x.rs", "rm changes x.rs"),
            ("time -p git commit -m x", "git commit"),
            ("env -S 'rm x.rs'", "rm changes x.rs"),
            ("env -S'git commit -m x'", "git commit"),
            ("TIMEOUT 5 rm x.rs", "rm changes x.rs"),
        ]);
        assert_runs(&[
            "echo $HOME",
            "cat $LOG_FILE",
            "ls $DIR",
            "env FOO=1 cat x",
            "ls | xargs grep -n \"$PAT\"",
            "GIT log --oneline -3",
        ]);
    }
}
