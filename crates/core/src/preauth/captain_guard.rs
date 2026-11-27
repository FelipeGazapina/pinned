//! `pinned captain-guard`: Claude Code's PreToolUse hook on every Bash call of a Captain whose
//! project is Manage only. That Captain starts without the file tools and loads this hook from
//! `hooks::CAPTAIN_SETTINGS`, so the shell is no way around them: its session reads, checks and
//! merges, and a worker changes files.
//!
//! It denies a command that writes a file, or changes the checkout or its history, wherever the
//! line runs it. It never asks the owner. A write to a standard device or into a temp folder runs,
//! the session's scratch folder among them; a folder below a temp folder that is named `pinned…` or
//! `claude…` does not, unless it is that scratch folder. It reads the line with the never list's
//! parser, and it makes no model, socket, hub or network call.
//!
//! It stops a habit, not an evasion: a program that a variable or a substitution names is not read,
//! code an interpreter runs (`python -c`, `node -e`, a script file) passes, and so does what a tool
//! writes in doing its own job (`cargo` and `target/`).

use super::never::{chdir, git_sub, is_assign, lex, resolve, walk, Cmd, Segment, Word, KEYWORDS, SHELLS, WRAPPERS};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SUBCOMMAND: &str = "captain-guard";

const REASON: &str = "pinned: this Captain is manage-only, so Bash is for reading and checking. Hand this change to a worker with delegate_task.";
/// The devices that keep nothing written to them.
const DEVICES: [&str; 4] = ["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty"];
/// The git subcommands that only read: they change neither the checkout, nor the history, nor the refs.
const GIT_READS: &[&str] = &[
    "log", "diff", "show", "status", "rev-parse", "ls-files", "ls-remote", "ls-tree", "blame", "grep", "describe", "merge-base", "cat-file", "for-each-ref", "shortlog", "rev-list", "name-rev", "count-objects", "version", "help", "whatchanged", "range-diff", "cherry", "show-ref", "show-branch", "verify-commit", "verify-tag", "check-ignore", "check-attr", "diff-tree", "merge-tree", "var",
];
/// The options of `git clone` that take the value after them, so that value is no operand.
const CLONE_VALUES: &[&str] = &["--depth", "--shallow-since", "--shallow-exclude", "--origin", "-o", "--branch", "-b", "--reference", "--separate-git-dir", "--template", "--config", "-c", "--upload-pack", "-u", "--jobs", "-j", "--filter", "--server-option", "--bundle-uri"];
/// The options of `git fetch` that take the value after them.
const FETCH_VALUES: &[&str] = &["--depth", "--deepen", "--shallow-since", "--shallow-exclude", "--jobs", "-j", "--filter", "--negotiation-tip", "--server-option", "--refmap", "--upload-pack"];
/// The commands that change the files their operands name. Items piped into one of them (`xargs`) are such
/// operands too, so the guard cannot judge where they go.
const PIPE_TARGETS: &[&str] = &["rm", "rmdir", "unlink", "shred", "chmod", "touch", "truncate", "mkdir", "tee", "sed", "gsed", "perl", "gzip", "gunzip", "bzip2", "xz", "cp", "mv", "install", "rsync", "dd", "rustfmt"];
/// The options of `git branch` and `git tag` that take the value after them.
const LIST_VALUES: [&str; 7] = ["--sort", "--format", "--contains", "--no-contains", "--merged", "--no-merged", "--points-at"];
/// The variables that run a command for git (`GIT_EXTERNAL_DIFF`, `GIT_SSH_COMMAND`) or name the config that does (`GIT_CONFIG_*`).
const GIT_HAZARD_ENV: [&str; 3] = ["GIT_EXTERNAL_DIFF=", "GIT_SSH_COMMAND=", "GIT_CONFIG_"];
/// The options before a package manager's or cargo's subcommand that take a value.
const MANAGER_VALUES: [&str; 7] = ["--prefix", "-C", "--dir", "--filter", "-w", "--workspace", "--cwd"];
const CARGO_VALUES: [&str; 5] = ["--manifest-path", "--color", "--config", "-Z", "-C"];
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
        let line = unshell(line);
        for seg in lex(&line, &self.home.to_string_lossy(), None, self.tmpdir.as_deref()) {
            let outer = folders.last().cloned().flatten();
            folders.resize(seg.depth + 1, outer);
            let cwd = folders[seg.depth].clone();
            let redirected: Vec<Word> = seg.redirs.iter().filter(|r| r.write).map(|r| r.target.clone()).collect();
            let cmds = captain_commands(&seg.words);
            let shell = cmds.iter().any(|c| SHELLS.contains(&c.prog.as_str()));
            let found = self
                .change(&cwd, "redirect", &redirected, false)
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
            // `patch --dry-run` changes nothing.
            "patch" => (!c.has("", &["dry-run"])).then(|| "patch".to_owned()),
            "cargo" => cargo(c),
            "npm" | "pnpm" | "yarn" | "bun" => package_manager(c),
            "ln" => self.ln(cwd, c),
            prog => {
                let files = written(c)?;
                // `xargs` appends its items to the command, so for these commands the items are the files written; the others are judged by the files they name.
                if c.stdin_args && PIPE_TARGETS.contains(&prog) {
                    return Some(format!("{prog} takes its files from a pipe"));
                }
                let deleting = matches!(prog, "rm" | "rmdir" | "unlink" | "shred");
                self.change(cwd, prog, &files, deleting)
            }
        }
    }

    /// `what` changing the first of `files` that is neither a device nor temporary. `deleting` says the
    /// command removes what it names.
    fn change(&self, cwd: &Option<PathBuf>, what: &str, files: &[Word], deleting: bool) -> Option<String> {
        files.iter().find(|w| !self.exempt(cwd, w, deleting)).map(|w| format!("{what} changes {}", w.text))
    }

    /// Whether `w`, named from `cwd`, is a device, or a place known to be below a temp folder before
    /// the shell runs, and not a pinned or Claude folder in it. A write names a file, so its last part
    /// is not checked. A deletion and a glob name what they remove, so their last part is checked too,
    /// and a deletion never takes a temp folder itself.
    fn exempt(&self, cwd: &Option<PathBuf>, w: &Word, deleting: bool) -> bool {
        if DEVICES.contains(&w.text.as_str()) {
            return true;
        }
        // A variable or a substitution that starts the word: a write to it runs, since the file is unknown.
        if let Some(at) = w.var_at.filter(|&at| !deleting && matches!(w.text.as_bytes().get(at), Some(b'$' | b'~'))) {
            return self.unknown_target(cwd, w, at);
        }
        // A `$` anywhere else: a value stuck to its flag (`--output=$OUT`) carries no mark of its own.
        if w.var_at.is_some() || w.text.contains('$') || w.glob_at.is_some_and(|at| w.text[at..].split('/').any(|part| part == "..")) {
            return false;
        }
        let names_last = deleting || w.glob_at.is_some();
        let Some(path) = resolve(cwd, &w.text) else { return false };
        // The deepest temp folder above the path: what lies below it is the part the write names.
        let Some(root) = self.temp.iter().filter(|root| path.starts_with(root) && (!deleting || path != **root)).max_by_key(|root| root.components().count()) else {
            return false;
        };
        let below: Vec<String> = path.strip_prefix(root).map(|rest| rest.components().map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase()).collect()).unwrap_or_default();
        if !self.session.is_empty() && below.contains(&self.session) {
            return true;
        }
        let folders = if names_last { &below[..] } else { &below[..below.len().saturating_sub(1)] };
        !folders.iter().any(|part| part.starts_with("pinned") || part.starts_with("claude"))
    }

    /// Whether a write to `w`, whose variable or substitution starts at byte `at`, runs. A variable that starts
    /// the word leaves the file unknown, so it runs. Otherwise the folder that the literal text names before the
    /// variable must lie in a temp folder or the session's scratch folder, with no pinned or Claude folder in it,
    /// and no `..` may follow.
    fn unknown_target(&self, cwd: &Option<PathBuf>, w: &Word, at: usize) -> bool {
        if at == 0 {
            return true;
        }
        if w.text.split('/').any(|part| part == "..") {
            return false;
        }
        // The file is below the folder that the literal text names up to its last `/`, before the variable.
        let Some(slash) = w.text[..at].rfind('/') else { return false };
        let Some(path) = resolve(cwd, &w.text[..=slash]) else { return false };
        let Some(root) = self.temp.iter().filter(|root| path.starts_with(root)).max_by_key(|root| root.components().count()) else {
            return false;
        };
        let below: Vec<String> = path.strip_prefix(root).map(|rest| rest.components().map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase()).collect()).unwrap_or_default();
        if !self.session.is_empty() && below.contains(&self.session) {
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
        if let Some(found) = rest.long_value("output").and_then(|out| self.change(cwd, "git", &[out], false)) {
            return Some(found);
        }
        // A clone writes its directory: the second operand, or the folder the command runs in when none is given.
        if sub == "clone" {
            let stripped = without_after(&rest, CLONE_VALUES);
            let dir = stripped.positionals().get(1).map_or_else(here, |w| (*w).clone());
            return self.change(cwd, "git clone", &[dir], false);
        }
        let positionals = rest.positionals();
        let first = positionals.first().map_or("", |w| w.text.as_str());
        // The flags without the values of `--sort V` and the like, so a sort key's letters are no flag.
        let flags = without_after(&rest, &LIST_VALUES);
        let allowed = GIT_READS.contains(&sub)
            || match sub {
                "pull" => rest.has("", &["ff-only"]) && !rest.has("r", &["rebase"]),
                // The first operand is the remote, since options such as `--depth 1` take their value after them; a refspec with a `:` after the remote updates a ref.
                "fetch" => {
                    let stripped = without_after(&rest, FETCH_VALUES);
                    !stripped.positionals().iter().skip(1).any(|w| w.text.contains(':'))
                }
                "branch" => !flags.has("dDmMfcCu", &["delete", "move", "force", "copy", "edit-description", "set-upstream-to", "unset-upstream", "create-reflog"]) && lists(&rest),
                // `-v` verifies a tag, and only lists and verifies are reads.
                "tag" => !flags.has("dafsmFeu", &["delete", "annotate", "sign", "force", "message", "file", "edit", "local-user", "create-reflog"]) && (rest.has("v", &["verify"]) || lists(&rest)),
                "stash" => matches!(first, "list" | "show"),
                "worktree" => first == "list",
                "remote" => positionals.is_empty() || matches!(first, "show" | "get-url"),
                // A read names one key, or uses `get` or `list`, or a read option; the write options never read.
                "config" => {
                    let stripped = without_after(&rest, &["--type", "--file", "-f", "--blob", "--default"]);
                    let words = stripped.positionals();
                    let reads = rest.has("l", &["get", "get-all", "get-regexp", "get-urlmatch", "list", "show-origin", "show-scope"]) || matches!(words.first().map(|w| w.text.as_str()), Some("get" | "list")) || words.len() == 1;
                    reads && !rest.has("e", &["edit", "add", "unset", "unset-all", "replace-all", "rename-section", "remove-section"])
                }
                "symbolic-ref" => positionals.len() <= 1 && !rest.has("md", &["delete", "message"]),
                // A bare `git submodule` lists them, as `status` does.
                "submodule" => first.is_empty() || matches!(first, "status" | "summary"),
                // `expire` and `delete` rewrite the log; any other operand, or none, reads it.
                "reflog" => !positionals.iter().any(|w| matches!(w.text.as_str(), "expire" | "delete")),
                // `--check`, `--stat`, `--numstat` and `--summary` change nothing, unless they also name the index.
                "apply" => rest.has("", &["check", "stat", "numstat", "summary"]) && !rest.has("", &["index", "cached", "3way"]),
                "notes" => positionals.is_empty() || matches!(first, "list" | "show"),
                _ => false,
            };
        (!allowed).then(|| format!("git {sub}"))
    }

    /// `gh` changes the checkout with `pr checkout`, `repo clone`, `repo fork --clone`, `repo sync` with
    /// no repository, `issue develop --checkout`, `repo create --clone` and `gist clone`, and `run` and
    /// `release download` write into the folder they run in unless `-D` names another.
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
            ["repo", "sync", ..] if !c.args.iter().any(|w| w.text.contains('/')) => Some("gh repo sync".to_owned()),
            ["issue", "develop", ..] if c.has("c", &["checkout"]) => Some("gh issue develop".to_owned()),
            ["repo", "create", ..] if c.has("c", &["clone"]) => Some("gh repo create".to_owned()),
            ["gist", "clone", ..] => Some("gh gist clone".to_owned()),
            ["run", "download", ..] => {
                let dir = c.value('D', "dir").unwrap_or_else(here);
                self.change(cwd, "gh run download", &[dir], false)
            }
            ["release", "download", ..] => {
                // `-O -` writes the asset to the standard output, and then nothing is written unless `-D` names a folder.
                let stdout = c.value('O', "output").is_some_and(|out| out.text == "-");
                let mut targets: Vec<Word> = [c.value('D', "dir"), c.value('O', "output").filter(|out| out.text != "-")].into_iter().flatten().collect();
                if targets.is_empty() && !stdout {
                    targets.push(here());
                }
                self.change(cwd, "gh release download", &targets, false)
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
        if let Some(found) = self.change(cwd, "find", &outputs, false) {
            return Some(found);
        }
        if c.args.iter().any(|w| w.text == "-delete") {
            if let Some(found) = self.change(cwd, "find", &starts, true) {
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
                if let Some(found) = captain_commands(&words).iter().find_map(|c| self.command(cwd, c, nest)) {
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
            if let Some(w) = sources.iter().find(|w| !self.exempt(cwd, w, false)) {
                return Some(format!("ln links {}", w.text));
            }
        }
        self.change(cwd, "ln", &c.write_args(), false)
    }
}

/// `c` without the options in `values` and the word after each, such as the value of `--sort V`.
fn without_after(c: &Cmd, values: &[&str]) -> Cmd {
    let mut skip = false;
    let args = c
        .args
        .iter()
        .filter(|w| {
            if std::mem::take(&mut skip) {
                return false;
            }
            skip = values.contains(&w.text.as_str());
            !skip
        })
        .cloned()
        .collect();
    Cmd { prog: c.prog.clone(), args, stdin_args: c.stdin_args }
}

/// Whether `git branch` or `git tag` only lists: no name is left once the options that take a value are
/// skipped, unless `-l` or `--list` makes what is left a pattern.
fn lists(rest: &Cmd) -> bool {
    if rest.has("l", &["list"]) {
        return true;
    }
    let mut value = false;
    rest.args.iter().all(|w| {
        if std::mem::take(&mut value) {
            return true;
        }
        if LIST_VALUES.contains(&w.text.as_str()) {
            value = true;
            return true;
        }
        w.text.starts_with('-')
    })
}

/// Whether a shell reads its script from a pipe: with no `-c` and no script file, the script is not in the
/// line. A heredoc is that script too, but its body is judged as commands in [`Guard::line`].
fn stdin_script(seg: &Segment, cmds: &[Cmd]) -> Option<String> {
    if !seg.piped {
        return None;
    }
    cmds.iter().find(|c| SHELLS.contains(&c.prog.as_str()) && c.scripts().is_empty() && c.positionals().iter().all(|w| w.text == "-")).map(|c| format!("{} reads a script from a pipe", c.prog))
}

/// A git command in a segment that sets a variable which runs a command for git. Only the assignments
/// before `git` set its environment: a word after it is an argument.
fn git_env(seg: &Segment, cmds: &[Cmd]) -> Option<String> {
    if !cmds.iter().any(|c| c.prog == "git") {
        return None;
    }
    let before_git = seg.words.iter().take_while(|w| !w.text.rsplit('/').next().is_some_and(|prog| prog.eq_ignore_ascii_case("git")));
    before_git.map(|w| w.text.as_str()).filter(|text| is_assign(text)).find(|text| GIT_HAZARD_ENV.iter().any(|name| text.starts_with(name))).map(|text| format!("git with {}", text.split('=').next().unwrap_or(text)))
}

/// The words of `c` that are not options (`-x`, `--long`, `+toolchain`) nor the value of one of `values`.
fn operands<'a>(c: &'a Cmd, values: &[&str]) -> Vec<&'a str> {
    let mut out = vec![];
    let mut value = false;
    for w in &c.args {
        let t = w.text.as_str();
        if std::mem::take(&mut value) {
            continue;
        }
        if values.contains(&t) {
            value = true;
        } else if !((t.starts_with('-') || t.starts_with('+')) && t.len() > 1) {
            out.push(t);
        }
    }
    out
}

/// Whether a cargo command changes `Cargo.toml` or `Cargo.lock`, or rewrites the sources: a format or
/// a fix that writes, not one that checks.
fn cargo(c: &Cmd) -> Option<String> {
    let sub = *operands(c, &CARGO_VALUES).first()?;
    let flag_given = |flag: &str| c.args.iter().any(|w| w.text == flag);
    let changes = match sub {
        // `--check` and `--help` only check or print.
        "fmt" => !(flag_given("--check") || flag_given("--help") || flag_given("-h")),
        "clippy" => flag_given("--fix"),
        // `rm` is an alias of `remove`; `init` and `new` create a package.
        "fix" | "add" | "remove" | "rm" | "generate-lockfile" | "init" | "new" => true,
        // A dry run changes no lockfile.
        "update" => !flag_given("--dry-run"),
        _ => false,
    };
    changes.then(|| format!("cargo {sub}"))
}

/// Whether an npm, pnpm, yarn or bun command changes `package.json`, a lockfile or the packages installed.
fn package_manager(c: &Cmd) -> Option<String> {
    let words = operands(c, &MANAGER_VALUES);
    let sub = words.first().copied().unwrap_or_default();
    let changes = match c.prog.as_str() {
        "npm" => match sub {
            "install" | "i" | "add" | "update" | "up" | "upgrade" | "uninstall" | "un" | "unlink" | "remove" | "rm" | "r" | "dedupe" | "ddp" | "prune" | "init" | "create" | "link" | "ln" | "install-test" | "it" | "install-ci-test" => true,
            // A bare `npm version` prints the version; one with an argument sets it.
            "version" => words.len() > 1,
            "audit" => words.get(1) == Some(&"fix") || c.has("", &["fix"]),
            "pkg" => words.get(1) == Some(&"set"),
            _ => false,
        },
        "pnpm" => matches!(sub, "install" | "i" | "add" | "update" | "up" | "upgrade" | "remove" | "rm" | "un" | "uninstall"),
        // A bare `yarn` installs; `--version` and `--help` only print.
        "yarn" => match sub {
            "" => !c.has("vh", &["version", "help"]),
            "add" | "install" | "up" | "upgrade" | "upgrade-interactive" | "remove" => true,
            _ => false,
        },
        "bun" => matches!(sub, "install" | "i" | "add" | "remove" | "rm" | "update" | "link"),
        _ => false,
    };
    changes.then(|| format!("{} {}", c.prog, if sub.is_empty() { "install" } else { sub }))
}

/// The files `c` writes, when its program changes the files it names, and `None` for any other call.
fn written(c: &Cmd) -> Option<Vec<Word>> {
    let named = || c.positionals().into_iter().cloned().collect::<Vec<_>>();
    match c.prog.as_str() {
        "rm" | "rmdir" | "unlink" | "shred" => Some(named()),
        "cp" | "mv" | "install" | "tee" => Some(c.write_args()),
        // `of=` names the file; its value keeps the variable marks that `write_args` would drop.
        "dd" => Some(c.args.iter().filter_map(|w| w.text.strip_prefix("of=").map(|value| stuck(w, w.text.len() - value.len()))).collect()),
        "touch" => Some(without_values(c, "tdrA", &["date", "reference"]).write_args()),
        "truncate" => Some(without_values(c, "sr", &["size", "reference"]).write_args()),
        "mkdir" => Some(without_values(c, "m", &["mode"]).write_args()),
        // `-I` is the BSD sed's in-place flag, and `gsed` is GNU sed on macOS.
        "sed" | "gsed" => {
            if !c.has("iI", &["in-place"]) {
                return None;
            }
            // The first file word is the script, unless `-e` or `-f` gives it.
            let script_given = c.has("ef", &["expression", "file"]);
            Some(without_values(c, "efI", &["expression", "file"]).positionals().into_iter().skip(usize::from(!script_given)).cloned().collect())
        }
        "perl" => perl_in_place(c),
        "chmod" => {
            // The first word is the mode, unless a flag gives it (`-x`) or `--reference` does.
            let flagged = c.args.iter().any(|w| w.text.starts_with("--reference") || w.text.strip_prefix('-').is_some_and(|mode| !mode.is_empty() && mode.chars().all(|m| "rwxXst,+=ugo".contains(m))));
            Some(named().into_iter().skip(usize::from(!flagged)).collect())
        }
        "rsync" => {
            // A dry run (`-n`, `--dry-run`) writes nothing.
            if c.has("n", &["dry-run"]) {
                return None;
            }
            let mut files = named();
            let destination = files.pop()?;
            (!files.is_empty()).then(|| vec![destination])
        }
        // `rustfmt` rewrites each file it names, unless it only checks or prints what it would do.
        "rustfmt" => (!c.has("", &["check", "help", "version", "print-config"])).then(|| without_values(c, "", &["edition", "emit", "config", "config-path", "color", "style-edition"]).positionals().into_iter().cloned().collect()),
        // Without `-o` it writes nothing, so it names no file: `sort -u` runs even from a pipe.
        "sort" => flag(c, 'o', true, SORT_VALUES, "output").map(|out| out.into_iter().collect()),
        "tar" => {
            if tar_extracts(c) {
                // Extraction writes the members into `-C`, or into the folder it runs in.
                Some(vec![flag(c, 'C', true, TAR_VALUES, "directory").flatten().unwrap_or_else(here)])
            } else if tar_creates(c) {
                // The archive that `-f` names; `-` is the standard output.
                downloaded(flag(c, 'f', true, TAR_VALUES, "file").flatten().into_iter().collect())
            } else {
                None
            }
        }
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
        "wget" if c.has("", &["spider"]) => None,
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
/// way (`tar xzf file`). Extracting to the standard output (`-O`, `--to-stdout`) writes no file.
fn tar_extracts(c: &Cmd) -> bool {
    let first = c.args.first().filter(|w| !w.text.starts_with('-')).map(|w| w.text.as_str());
    let extracts = first.is_some_and(|t| t.contains('x')) || flag(c, 'x', false, TAR_VALUES, "extract").is_some() || c.has("", &["get"]);
    let to_stdout = first.is_some_and(|t| t.contains('O')) || flag(c, 'O', false, TAR_VALUES, "to-stdout").is_some();
    extracts && !to_stdout
}

/// Whether tar creates an archive: `-c` or `--create`.
fn tar_creates(c: &Cmd) -> bool {
    flag(c, 'c', false, TAR_VALUES, "create").is_some()
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
                    Some(value) => Some(stuck(w, text.len() - value.len())),
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
                        (true, false) => Some(stuck(w, text.len() - rest.len())),
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

/// The value of `w` from byte `from` on, when it is stuck to its flag (`--output=VALUE`, `-oVALUE`). It keeps the
/// variable and glob marks that lie in it, so a `$` in the value still makes the target unknown.
fn stuck(w: &Word, from: usize) -> Word {
    Word { text: w.text[from..].to_owned(), var_at: w.var_at.and_then(|at| at.checked_sub(from)), glob_at: w.glob_at.and_then(|at| at.checked_sub(from)), fetch: false }
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
    Cmd { prog: c.prog.clone(), args, stdin_args: c.stdin_args }
}

/// The command that `words` starts: its program's file name in lower case, and the words after it.
/// Only the program is lower-cased, since macOS folds the case of a file name.
fn cmd_at(words: &[Word], stdin_args: bool) -> Cmd {
    let prog = words[0].text.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    Cmd { prog, args: words[1..].to_vec(), stdin_args }
}

/// The commands a line runs, for the captain: its keywords and assignments are skipped, and each wrapper
/// runs the program after its options (see [`program_index`]). A program lookup or a test runs nothing.
fn captain_commands(words: &[Word]) -> Vec<Cmd> {
    let mut at = 0;
    while at < words.len() && (is_assign(&words[at].text) || KEYWORDS.contains(&words[at].text.as_str())) {
        // `time -p` takes its flag too.
        at += if words[at].text == "time" && words.get(at + 1).is_some_and(|w| w.text == "-p") { 2 } else { 1 };
    }
    if at >= words.len() {
        return vec![];
    }
    captain_words(&words[at..], false)
}

/// The commands that `words` starts, the first being its program, with `stdin` for a program that takes
/// its files from a pipe. A wrapper with options runs one program, and the others in [`WRAPPERS`] read
/// every later word that is not an option or an assignment as a program.
fn captain_words(words: &[Word], stdin: bool) -> Vec<Cmd> {
    let own = cmd_at(words, stdin);
    let args = &words[1..];
    let name = own.prog.clone();
    // `command -v` and `command -V` look a program up and run none.
    let looks_up = args.iter().take_while(|w| w.text.starts_with('-')).any(|w| !w.text.starts_with("--") && w.text.chars().any(|c| matches!(c, 'v' | 'V')));
    if matches!(name.as_str(), "which" | "type" | "whereis" | "hash" | "[" | "test" | "((") || (name == "command" && looks_up) {
        return vec![];
    }
    if let Some(k) = program_index(&name, args) {
        let mut out = vec![own];
        // `xargs` appends its items to the command, unless `-I`, `-J` or `-i` puts each one where `{}` stands: then
        // the command names its files itself.
        let replaces = args[..k].iter().any(|w| w.text.starts_with("-I") || w.text.starts_with("-J") || w.text.starts_with("-i"));
        out.extend(captain_words(&args[k..], stdin || (name == "xargs" && !replaces)));
        return out;
    }
    let mut out = vec![own];
    if WRAPPERS.contains(&name.as_str()) && !LISTED.contains(&name.as_str()) {
        for j in 1..words.len() {
            let t = words[j].text.as_str();
            if !(t.starts_with('-') || is_assign(t)) {
                out.push(cmd_at(&words[j..], stdin || name == "xargs"));
            }
        }
    }
    out
}

/// The wrappers whose options [`program_index`] reads, and `time`.
const LISTED: &[&str] = &["env", "timeout", "nice", "nohup", "exec", "builtin", "command", "sudo", "caffeinate", "xargs", "time"];

/// Where the program that a listed wrapper runs starts in its `args`: after its options, and for
/// `timeout` after its duration. `None` when it runs none, or when `name` is no listed wrapper.
fn program_index(name: &str, args: &[Word]) -> Option<usize> {
    let values: &[&str] = match name {
        "env" => &["-u", "-C", "-P", "-S"],
        "timeout" => &["-s", "-k", "--signal", "--kill-after"],
        "nice" => &["-n"],
        "exec" => &["-a"],
        "sudo" => &["-u", "-g", "-C", "-D"],
        "caffeinate" => &["-t", "-w"],
        "xargs" => &["-n", "-L", "-P", "-s", "-I", "-J", "-E", "-d"],
        _ if LISTED.contains(&name) => &[],
        _ => return None,
    };
    let mut k = 0;
    while let Some(w) = args.get(k) {
        let t = w.text.as_str();
        if t == "--" {
            k += 1;
            break;
        }
        if values.contains(&t) {
            k += 2;
        } else if (t.starts_with('-') && t.len() > 1) || (name == "env" && is_assign(t)) {
            k += 1;
        } else {
            break;
        }
    }
    if name == "timeout" {
        k += 1;
    }
    (k < args.len()).then_some(k)
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

/// The line with the shell syntax that runs no command blanked out: the words of a `for` list, the patterns of
/// a `case` (each `)` becomes the `;` that ends the header) and the operands of `[[ … ]]`. The commands in
/// `$(…)` and backquotes stay, and are blanked the same way where they stand.
fn unshell(line: &str) -> String {
    let cs: Vec<char> = line.chars().collect();
    unshell_chars(&cs)
}

fn unshell_chars(cs: &[char]) -> String {
    let mut out = String::new();
    // Per open `case`: whether its next word is a pattern, rather than a word of a command.
    let mut cases: Vec<bool> = vec![];
    // Whether the next word starts a command.
    let mut cmd = true;
    let mut i = 0;
    while i < cs.len() {
        if cases.last() == Some(&true) {
            let c = cs[i];
            if c.is_whitespace() {
                out.push(c);
                i += 1;
            } else if word_is(cs, i, "esac") {
                out.push_str("esac");
                i += 4;
                cases.pop();
                cmd = false;
            } else if let Some(close) = pattern_end(cs, i) {
                out.push_str(&" ".repeat(close - i));
                out.push(';');
                i = close + 1;
                if let Some(top) = cases.last_mut() {
                    *top = false;
                }
                cmd = true;
            } else {
                // A pattern that can't be read: the rest of the line is judged as it stands.
                cases.pop();
            }
            continue;
        }
        match cs[i] {
            ' ' | '\t' | '\r' => {
                out.push(cs[i]);
                i += 1;
            }
            '\n' => {
                out.push('\n');
                i += 1;
                cmd = true;
            }
            '#' => {
                let end = cs[i..].iter().position(|&c| c == '\n').map_or(cs.len(), |k| i + k);
                out.extend(&cs[i..end]);
                i = end;
            }
            ';' => {
                if cs.get(i + 1) == Some(&';') {
                    out.push_str(";;");
                    i += 2;
                    if let Some(top) = cases.last_mut() {
                        *top = true;
                    }
                } else {
                    out.push(';');
                    i += 1;
                }
                cmd = true;
            }
            '&' | '|' | '(' | ')' => {
                // The `&` of `>&` or `<&` names a descriptor, and no command starts with it.
                let descriptor = cs[i] == '&' && i > 0 && matches!(cs[i - 1], '<' | '>');
                out.push(cs[i]);
                i += 1;
                cmd = !descriptor;
            }
            '<' | '>' => {
                out.push(cs[i]);
                i += 1;
                cmd = false;
            }
            _ => {
                let end = word_end(cs, i);
                let word: String = cs[i..end].iter().collect();
                if cmd && word == "case" {
                    if let Some(after_in) = case_header(cs, end) {
                        emit(cs, i, after_in, false, &mut out);
                        cases.push(true);
                        i = after_in;
                        cmd = false;
                        continue;
                    }
                }
                if cmd && word == "esac" && !cases.is_empty() {
                    cases.pop();
                    out.push_str("esac");
                    i = end;
                    cmd = false;
                    continue;
                }
                if cmd && word == "for" {
                    if let Some((name, list_end)) = for_list(cs, end) {
                        out.push_str("for");
                        emit(cs, end, name, false, &mut out);
                        emit(cs, name, list_end, true, &mut out);
                        i = list_end;
                        cmd = false;
                        continue;
                    }
                }
                if cmd && word == "[[" {
                    if let Some(close) = dbracket_close(cs, end) {
                        out.push_str("[[");
                        emit(cs, end, close, true, &mut out);
                        out.push_str("]]");
                        i = close + 2;
                        cmd = false;
                        continue;
                    }
                }
                emit(cs, i, end, false, &mut out);
                cmd = cmd && matches!(word.as_str(), "then" | "do" | "else" | "elif" | "if" | "while" | "until" | "time" | "!" | "{" | "coproc");
                i = end;
            }
        }
    }
    out
}

/// Writes `cs[a..z]` to `out`. With `mask`, each character becomes a space, but a substitution keeps its
/// commands, which [`unshell_chars`] reads in turn.
fn emit(cs: &[char], a: usize, z: usize, mask: bool, out: &mut String) {
    let put = |out: &mut String, c: char| out.push(if mask { ' ' } else { c });
    let mut in_dq = false;
    let mut i = a;
    while i < z {
        match cs[i] {
            '\\' if i + 1 < z => {
                put(out, '\\');
                put(out, cs[i + 1]);
                i += 2;
            }
            '\'' if !in_dq => {
                let end = cs[i + 1..z].iter().position(|&c| c == '\'').map_or(z, |k| i + k + 2);
                cs[i..end].iter().for_each(|&c| put(out, c));
                i = end;
            }
            '"' => {
                in_dq = !in_dq;
                put(out, '"');
                i += 1;
            }
            '$' if cs.get(i + 1) == Some(&'(') => match close_paren(cs, i + 2) {
                Some(close) if close < z => {
                    out.push_str("$(");
                    out.push_str(&unshell_chars(&cs[i + 2..close]));
                    out.push(')');
                    i = close + 1;
                }
                _ => {
                    put(out, '$');
                    i += 1;
                }
            },
            '`' => match close_backtick(cs, i + 1) {
                Some(close) if close < z => {
                    out.push('`');
                    out.push_str(&unshell_chars(&cs[i + 1..close]));
                    out.push('`');
                    i = close + 1;
                }
                _ => {
                    put(out, '`');
                    i += 1;
                }
            },
            c => {
                put(out, c);
                i += 1;
            }
        }
    }
}

/// Whether `c` ends a word of a shell line.
fn is_break(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>')
}

/// The index just past the quoted text, or the substitution, that starts at `i`; `None` when it never ends.
fn span_end(cs: &[char], i: usize) -> Option<usize> {
    Some(match cs[i] {
        '\\' => (i + 2).min(cs.len()),
        '\'' => cs[i + 1..].iter().position(|&c| c == '\'').map(|k| i + k + 2)?,
        '"' => skip_dq(cs, i + 1)?,
        '`' => close_backtick(cs, i + 1)? + 1,
        '$' if cs.get(i + 1) == Some(&'(') => close_paren(cs, i + 2)? + 1,
        '$' if cs.get(i + 1) == Some(&'{') => close_brace(cs, i + 2)? + 1,
        _ => i + 1,
    })
}

/// The index just past the double quotes whose text starts at `i`.
fn skip_dq(cs: &[char], mut i: usize) -> Option<usize> {
    while i < cs.len() {
        match cs[i] {
            '"' => return Some(i + 1),
            '\\' => i += 2,
            '$' if cs.get(i + 1) == Some(&'(') => i = close_paren(cs, i + 2)? + 1,
            '`' => i = close_backtick(cs, i + 1)? + 1,
            _ => i += 1,
        }
    }
    None
}

/// The index of the `)` that closes a `$(` whose commands start at `i`.
fn close_paren(cs: &[char], mut i: usize) -> Option<usize> {
    let mut depth = 1;
    while i < cs.len() {
        match cs[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {
                i = span_end(cs, i)?;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// The index of the backquote that closes one whose commands start at `i`.
fn close_backtick(cs: &[char], mut i: usize) -> Option<usize> {
    while i < cs.len() {
        match cs[i] {
            '`' => return Some(i),
            '\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

/// The index of the `}` that closes a `${` whose text starts at `i`.
fn close_brace(cs: &[char], mut i: usize) -> Option<usize> {
    let mut depth = 1;
    while i < cs.len() {
        match cs[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The index where the word that starts at `i` ends: at its first character that ends a word, outside quotes
/// and substitutions.
fn word_end(cs: &[char], mut i: usize) -> usize {
    while i < cs.len() && !is_break(cs[i]) {
        i = span_end(cs, i).unwrap_or(cs.len());
    }
    i
}

/// Whether the word that starts at `i` is `word`.
fn word_is(cs: &[char], i: usize, word: &str) -> bool {
    let end = word_end(cs, i);
    end > i && cs[i..end].iter().copied().eq(word.chars())
}

fn skip_blanks(cs: &[char], mut i: usize) -> usize {
    while i < cs.len() && matches!(cs[i], ' ' | '\t' | '\r') {
        i += 1;
    }
    i
}

/// The end of a `case` header that starts after `after`, once its subject is followed by `in`.
fn case_header(cs: &[char], after: usize) -> Option<usize> {
    let subject = skip_blanks(cs, after);
    if subject >= cs.len() || is_break(cs[subject]) {
        return None;
    }
    let keyword = skip_blanks(cs, word_end(cs, subject));
    word_is(cs, keyword, "in").then(|| word_end(cs, keyword))
}

/// The name and the list of a `for NAME in LIST` whose name starts after `after`: where the name starts, and
/// where the list ends, at a `;`, a newline, an operator or `do`.
fn for_list(cs: &[char], after: usize) -> Option<(usize, usize)> {
    let name = skip_blanks(cs, after);
    let name_end = word_end(cs, name);
    if name >= cs.len() || !(cs[name].is_ascii_alphabetic() || cs[name] == '_') || !cs[name..name_end].iter().all(|c| c.is_ascii_alphanumeric() || *c == '_') {
        return None;
    }
    let keyword = skip_blanks(cs, name_end);
    if !word_is(cs, keyword, "in") {
        return None;
    }
    let mut j = word_end(cs, keyword);
    loop {
        j = skip_blanks(cs, j);
        if j >= cs.len() || is_break(cs[j]) || word_is(cs, j, "do") {
            break;
        }
        j = word_end(cs, j);
    }
    Some((name, j))
}

/// The index of the `]]` that closes a `[[` whose operands start at `start`, when it closes on the same line.
fn dbracket_close(cs: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < cs.len() {
        match cs[i] {
            '\n' | ';' => return None,
            ']' if cs.get(i + 1) == Some(&']') && i > start && matches!(cs[i - 1], ' ' | '\t' | '\r') && cs.get(i + 2).is_none_or(|&c| is_break(c)) => return Some(i),
            '\\' | '\'' | '"' | '$' | '`' => i = span_end(cs, i)?,
            _ => i += 1,
        }
    }
    None
}

/// The index of the `)` that ends a `case` pattern which starts at `i`, when the pattern holds no substitution.
fn pattern_end(cs: &[char], mut i: usize) -> Option<usize> {
    while i < cs.len() {
        match cs[i] {
            ')' => return Some(i),
            '\n' | ';' | '(' | '&' | '<' | '>' | '$' | '`' => return None,
            '\'' | '"' | '\\' => {
                let end = span_end(cs, i)?;
                if cs[i..end].iter().any(|&c| c == '$' || c == '`') {
                    return None;
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    None
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
    fn a_deletion_whose_target_cannot_be_known_is_denied() {
        assert_denied(&[
            ("rm {temp}/$name", "rm changes {temp}/$name"),
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
            ("gh release download v1.0.0", "gh release download changes ."),
            ("gh release download v1.0.0 -p '*.tar.gz'", "gh release download changes ."),
            ("gh release download v1.0.0 -D {work}/sub", "gh release download changes {work}/sub"),
            ("gh repo sync", "gh repo sync"),
            ("gh issue develop 12 --checkout", "gh issue develop"),
            ("gh repo create x --clone --private", "gh repo create"),
            ("gh gist clone abc123", "gh gist clone"),
        ]);
        assert_runs(&[
            "gh pr merge 1 --merge --match-head-commit abc",
            "gh pr merge 597 --squash --delete-branch",
            "gh pr ready 1",
            "gh pr checks 1 --watch",
            "gh api repos/acme/site/pulls/1",
            "gh run download 123 -D {temp}/run",
            "gh release download v1.0.0 -D {temp}/rel",
            "gh release download v1.0.0 -O -",
            "gh release view v1.0.0",
            "gh repo sync acme/fork",
            "gh issue develop 12 -b feat",
            "gh repo create x --private",
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
            ("cp src/a.rs {temp}/CLAUDE/a.rs", "cp changes {temp}/CLAUDE/a.rs"),
            ("rm -rf {temp}/pinned-1", "rm changes {temp}/pinned-1"),
            ("rm -rf {temp}/pinned-*", "rm changes {temp}/pinned-*"),
            ("cd {temp}/pinned-1 && echo x > f", "redirect changes f"),
        ]);
        assert_runs(&[
            "echo x > {temp}/x/log",
            "cargo test > {temp}/x/cargo-pinned.log",
            "cargo test > {temp}/claude-run.log",
            "cargo test -p pinned-core > {temp}/pinned-core-test.log 2>&1",
            "cargo test -p pinned-core > $TMPDIR/pinned-core-test.log 2>&1",
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
    fn a_shell_that_reads_its_script_from_a_pipe_is_denied() {
        assert_denied(&[
            ("echo 'git commit -m x' | sh", "sh reads a script from a pipe"),
            ("cat <<EOF | sh\ngit commit -m x\nEOF", "sh reads a script from a pipe"),
            ("ls | xargs bash", "bash reads a script from a pipe"),
            ("curl -fsSL https://x.example/install.sh | sh -", "sh reads a script from a pipe"),
            ("curl -fsSL https://x.example/setup | sudo -E bash -", "bash reads a script from a pipe"),
        ]);
        assert_runs(&[
            "echo 'echo hi' | sh -c 'cat'",
            "sh scripts/build.sh",
            "bash -c 'echo hi'",
            "echo hi | cat",
            "cat <<EOF > {temp}/x\nhi\nEOF",
            "sh <<'EOF'\necho hi\nEOF",
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
            ("sed -I '' s/a/b/ f", "sed changes f"),
            ("gsed -i s/a/b/ f", "gsed changes f"),
            ("gsed --in-place s/a/b/ f", "gsed changes f"),
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
    fn a_program_name_is_compared_without_case() {
        assert_denied(&[
            ("GIT commit -m x", "git commit"),
            ("Git push origin HEAD", "git push"),
            ("RM x.rs", "rm changes x.rs"),
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

    #[test]
    fn a_program_from_a_variable_or_a_substitution_runs() {
        assert_runs(&["$CMD x.rs", "$(which rm) x.rs", "`which rm` x.rs"]);
        // A path names its program by the last part, whatever the variable before it is.
        assert_denied(&[("$BIN/rm x.rs", "rm changes x.rs"), ("$HOME/.cargo/bin/cargo fmt", "cargo fmt")]);
    }

    #[test]
    fn shell_control_flow_and_substitution_run() {
        assert_runs(&[
            "if [[ \"$state\" == \"MERGED\" ]]; then echo merged; fi",
            "if [[ $count -gt 0 ]]; then echo some; fi",
            "[[ $? -eq 0 ]] && echo ok",
            "while read -r f; do wc -l \"$f\"; done < <(git ls-files '*.rs')",
            "case \"$state\" in MERGED) echo done;; *) echo open;; esac",
            "case $f in *.rs) wc -l \"$f\";; esac",
            "files=($(git diff --name-only origin/main)); echo ${#files[@]}",
            "prs=(\"$a\" \"$b\"); echo ${prs[0]}",
            "out+=\"$(git log -1 --format=%s)\"; echo \"$out\"",
            "for f in a b; do list+=\"$f \"; done; echo \"$list\"",
            "for f in $(git ls-files); do case \"$f\" in *.rs) echo \"$f\";; esac; done",
            "while [[ \"$(gh pr view 1 --json state -q .state)\" != MERGED ]]; do sleep 30; done",
        ]);
    }

    #[test]
    fn a_program_lookup_or_a_test_runs_nothing() {
        assert_runs(&[
            "command -v git gh cargo node",
            "command -v patch",
            "command -v rm cp mv",
            "which rm",
            "type rm",
            "whereis rm",
            "hash rm",
            "[ -f x.rs ] && cat x.rs",
            "test -s x.rs",
            "[[ -d src ]]",
        ]);
    }

    #[test]
    fn a_wrapper_runs_the_one_program_after_its_options() {
        assert_denied(&[
            ("env -u GIT_DIR git commit -m x", "git commit"),
            ("env A=1 -i rm x.rs", "rm changes x.rs"),
            ("timeout 30 git commit -m x", "git commit"),
            ("timeout -s KILL 30 rm x.rs", "rm changes x.rs"),
            ("nice -n 5 rm x.rs", "rm changes x.rs"),
            ("nohup git push origin HEAD", "git push"),
            ("exec -a tool rm x.rs", "rm changes x.rs"),
            ("command -p rm x.rs", "rm changes x.rs"),
            ("sudo -u root rm x.rs", "rm changes x.rs"),
            ("sudo -E git commit -m x", "git commit"),
            ("caffeinate -t 60 git push origin HEAD", "git push"),
            ("ls | xargs -0 -n 1 rm", "rm takes its files from a pipe"),
        ]);
        assert_runs(&[
            "env -u GH_TOKEN gh api -X PATCH repos/o/r/pulls/1 -f title=x",
            "env -u GITHUB_TOKEN gh api --method PATCH repos/o/r/pulls/1 -f title=x",
            "timeout 60 gh api -X PATCH repos/o/r/pulls/1 -f title=x",
            "caffeinate -i gh api -X PATCH repos/o/r/pulls/1 -f title=x",
            "timeout 60 curl -sS -X PATCH https://api.example.com/x -d a=1",
            "timeout 30 rg git crates",
            "timeout 30 grep -rn rm scripts/",
            "timeout 600 cargo test patch -- --nocapture",
            "timeout 600 cargo test touch -- --nocapture",
            "env -u GIT_DIR which git rg",
            "ls crates | xargs grep -rn patch",
            "git ls-files | xargs grep -n install",
            "git ls-files | xargs rg -n sort",
            "git ls-files | xargs grep -ln bash",
        ]);
    }

    #[test]
    fn a_write_into_a_temp_folder_itself_runs() {
        assert_runs(&[
            "cp Cargo.toml {temp}/",
            "cp Cargo.toml \"$TMPDIR\"",
            "cp Cargo.toml $TMPDIR/",
            "wget -P {temp} https://acme.dev/x",
            "cd {temp} && curl -sSLO https://acme.dev/a.tgz",
            "tar -xzf $TMPDIR/x.tgz -C $TMPDIR",
            "cd $TMPDIR && tar -xzf x.tgz",
            "unzip -q $TMPDIR/x.zip -d \"$TMPDIR\"",
            "gh run download 123 -D $TMPDIR",
            "mkdir -p $TMPDIR/x && cd $TMPDIR/x && gh run download 123",
        ]);
        assert_denied(&[("rm -rf $TMPDIR", "rm changes {temp}"), ("rm -rf {temp}", "rm changes {temp}")]);
    }

    #[test]
    fn a_temp_variable_with_a_default_is_the_hook_tmpdir() {
        assert_runs(&["cargo test > ${TMPDIR:-/tmp}/x.log 2>&1", "cargo test > \"${TMPDIR:-/tmp}/x.log\"", "cargo test > ${TMPDIR:-/work}/x.log", "cd ${TMPDIR:-/tmp} && rm x.rs", "cargo test > ${OTHER:-/tmp}/x.log"]);
    }

    #[test]
    fn a_read_that_looks_like_a_write_runs() {
        assert_runs(&[
            "tar -xzOf pkg.tgz package/package.json",
            "tar -xOf pkg.tar package.json | jq .",
            "tar --to-stdout -xf pkg.tar",
            "tar -tzf release.tgz",
            "wget --spider -q https://acme.dev",
            "sort -u input.txt",
            "git ls-files | xargs sort -u",
            "gzip -c notes.txt | wc -c",
            "gzip --stdout notes.txt",
        ]);
        assert_denied(&[
            ("tar -czf out.tgz crates", "tar changes out.tgz"),
            ("tar --create --file=out.tgz crates", "tar changes out.tgz"),
        ]);
    }

    #[test]
    fn the_git_reads_that_look_like_writes_run_and_the_refs_they_name_are_denied() {
        assert_denied(&[
            ("git symbolic-ref -d HEAD", "git symbolic-ref"),
            ("git symbolic-ref --delete refs/heads/x", "git symbolic-ref"),
            ("git config remote.origin.url x", "git config"),
            ("git config --replace-all a.b x", "git config"),
            ("git submodule add {temp}/x sub", "git submodule"),
            ("git submodule update --init", "git submodule"),
            ("git branch -a newtopic", "git branch"),
            ("git branch -v newtopic", "git branch"),
            ("git branch -vv newtopic", "git branch"),
            ("git -C wt branch --sort=refname newtopic", "git branch"),
            ("git branch --format=x newtopic", "git branch"),
            ("git branch --column newtopic", "git branch"),
            ("git tag --sort=refname v9", "git tag"),
            ("git tag --format=x v9", "git tag"),
            ("git tag --column v9", "git tag"),
            ("git tag -n v9", "git tag"),
            ("git fetch origin main:main", "git fetch"),
            ("git fetch origin pull/12/head:pr-12", "git fetch"),
            ("git fetch origin +refs/heads/*:refs/remotes/origin/*", "git fetch"),
        ]);
        assert_runs(&[
            "git diff-tree --no-commit-id --name-only -r HEAD",
            "git merge-tree --write-tree origin/main HEAD",
            "git symbolic-ref --short HEAD",
            "git symbolic-ref refs/remotes/origin/HEAD",
            "git symbolic-ref -q HEAD",
            "git config remote.origin.url",
            "git config user.name",
            "git submodule status",
            "git submodule summary",
            "git grep -n GIT_CONFIG_ crates",
            "git log -S GIT_SSH_COMMAND= --oneline",
            "git branch --sort -committerdate",
            "git branch --contains abc",
            "git tag --sort -creatordate | head",
            "git tag -n",
            "git fetch --all",
            "git fetch origin",
        ]);
    }

    #[test]
    fn package_managers_skip_their_global_options_and_a_bare_yarn_installs() {
        assert_denied(&[
            ("npm --prefix ui install", "npm install"),
            ("npm -w ui install lodash", "npm install"),
            ("npm install-test", "npm install-test"),
            ("npm dedupe", "npm dedupe"),
            ("npm prune", "npm prune"),
            ("npm version patch", "npm version"),
            ("npm init -y", "npm init"),
            ("npm link", "npm link"),
            ("pnpm -C ui install", "pnpm install"),
            ("pnpm --dir ui add x", "pnpm add"),
            ("pnpm --filter web add x", "pnpm add"),
            ("yarn", "yarn install"),
            ("yarn --frozen-lockfile", "yarn install"),
            ("yarn --immutable", "yarn install"),
            ("yarn --cwd ui install", "yarn install"),
            ("yarn up lodash", "yarn up"),
            ("yarn upgrade-interactive", "yarn upgrade-interactive"),
            ("bun install", "bun install"),
            ("bun add x", "bun add"),
        ]);
        assert_runs(&["npm ci", "npm test", "npm run x", "npx prettier --check .", "npm --prefix ui run build", "npm -w ui test", "yarn --version", "yarn -v", "yarn test", "yarn run build", "bun test", "bun run dev"]);
    }

    #[test]
    fn cargo_skips_its_global_options() {
        assert_denied(&[
            ("cargo --color never fmt", "cargo fmt"),
            ("cargo --manifest-path crates/core/Cargo.toml fmt", "cargo fmt"),
            ("cargo --locked --color never clippy --fix", "cargo clippy"),
        ]);
        assert_runs(&["cargo --color never test", "cargo --locked fmt -- --check", "cargo --manifest-path crates/core/Cargo.toml fmt --check"]);
    }

    #[test]
    fn shell_syntax_that_is_not_a_program_is_not_judged_as_one() {
        assert_runs(&[
            "for zip in a.zip b.zip; do unzip -l \"$zip\"; done",
            "for patch in {scratch}/*.patch; do head -3 \"$patch\"; done",
            "for install in a b; do echo $install; done",
            "for git in a; do echo $git; done",
            "case \"$bump\" in major) echo M;; patch) echo p;; esac",
            "case $x in rm) echo x;; esac",
            "[[ \"$f\" =~ \\.(patch|diff)$ ]] && echo diff",
        ]);
        assert_denied(&[
            ("for f in a; do git push --force; done", "git push"),
            ("for f in a; do rm x.rs; done", "rm changes x.rs"),
            ("for f in $(git push); do echo $f; done", "git push"),
            ("for patch in a; do patch -p1 < x; done", "patch"),
            ("case $x in a) git push;; esac", "git push"),
            ("[[ -n \"$(rm x.rs)\" ]]", "rm changes x.rs"),
        ]);
    }

    #[test]
    fn a_write_target_with_a_variable_or_a_substitution_is_judged_by_its_literal_prefix() {
        assert_runs(&[
            "S={scratch}; cargo test > $S/test.log 2>&1",
            "SP={scratch}; mkdir -p $SP/probes",
            "mkdir -p \"$SCRATCH\"",
            "for pr in 596 597; do gh pr diff $pr > {scratch}/pr-$pr.diff; done",
            "cargo test -p $f > $TMPDIR/$f.log",
            "echo x > {scratch}/test-$(date +%H%M).log",
            "echo x > {temp}/test-$$.log",
            "out=$(mktemp); gh pr diff 597 > \"$out\"",
            "cargo test 2>&1 | tee $out",
            "curl -s -o \"$TMPDIR/r.json\" https://api.github.com",
            "cargo test | tee /dev/tty",
            "cargo test > \"$OTHER/log\"",
            "curl --output=$OUT https://acme.dev",
            "cd {temp} && dd if=a of=$OUT",
        ]);
        assert_denied(&[
            ("echo x > src/$x.rs", "redirect changes src/$x.rs"),
            ("echo x > out-$$.log", "redirect changes out-$$.log"),
            ("rm $DIR/foo", "rm changes $DIR/foo"),
        ]);
    }

    #[test]
    fn xargs_judges_the_command_it_runs() {
        assert_runs(&[
            "cat urls.txt | xargs -n1 curl -s -o /dev/null -w '%{http_code}'",
            "xargs -I{} curl -o {scratch}/{}.json https://x/{}",
            "ls | xargs -I{} cp {} {scratch}/snap/",
        ]);
        assert_denied(&[
            ("find {scratch} -name x | xargs rm -f", "rm takes its files from a pipe"),
            ("ls | xargs touch", "touch takes its files from a pipe"),
            ("ls | xargs cp {scratch}/snap", "cp takes its files from a pipe"),
            ("ls | xargs -I{} rm {}", "rm changes {}"),
        ]);
    }

    #[test]
    fn gh_release_download_judges_its_output_and_its_directory() {
        assert_runs(&[
            "gh release download v1 -p x -O {scratch}/latest.json",
            "gh release download v1 --output \"$TMPDIR/latest.json\"",
            "gh release download v1 -D {temp}/x",
        ]);
        assert_denied(&[("gh release download v1 -O src/f", "gh release download changes src/f")]);
    }

    #[test]
    fn git_clone_into_a_temp_folder_or_the_scratch_folder_runs() {
        assert_runs(&["git clone https://github.com/o/r {scratch}/r", "git clone --depth 1 https://github.com/o/r \"$TMPDIR/r\""]);
        assert_denied(&[
            ("git clone https://github.com/o/r", "git clone changes ."),
            ("git clone --depth 1 https://github.com/o/r sub/r", "git clone changes sub/r"),
        ]);
    }

    #[test]
    fn git_reads_run_and_the_writes_next_to_them_stay_denied() {
        assert_runs(&[
            "git config --type bool core.bare",
            "git config --file .gitmodules submodule.x.path",
            "git config get remote.origin.url",
            "git config list",
            "git config --get-urlmatch http.sslverify https://acme.dev",
            "git config --show-origin --get user.name",
            "git config --show-scope user.name",
            "git reflog -n 10",
            "git reflog HEAD",
            "git reflog show main",
            "git apply --check x.patch",
            "git apply --stat x.patch",
            "git apply --numstat x.patch",
            "git apply --summary x.patch",
            "git tag -v v0.18.0",
            "git var GIT_AUTHOR_IDENT",
            "git submodule",
            "git submodule status",
            "git fetch --depth 1 https://github.com/o/r main",
        ]);
        assert_denied(&[
            ("git config --type bool core.bare x", "git config"),
            ("git config --file .gitmodules submodule.x.path y", "git config"),
            ("git reflog delete HEAD@{1}", "git reflog"),
            ("git apply x.patch", "git apply"),
            ("git apply --check --index x.patch", "git apply"),
            ("git apply --check --cached x.patch", "git apply"),
            ("git apply --check --3way x.patch", "git apply"),
            ("git tag -v -d v1", "git tag"),
            ("git submodule foreach ls", "git submodule"),
            ("git fetch --depth 1 https://github.com/o/r main:main", "git fetch"),
        ]);
    }

    #[test]
    fn dry_runs_and_help_run() {
        assert_runs(&["cargo update --dry-run", "cargo fmt --help", "patch --dry-run -p1 < x.patch", "rsync -an a/ b/", "npm version"]);
        assert_denied(&[("rsync -a a/ b/", "rsync changes b/"), ("npm version patch", "npm version")]);
    }

    #[test]
    fn a_heredoc_fed_to_a_shell_is_judged_as_commands() {
        assert_runs(&["bash -s <<'EOF'\ngit status\ngh pr view 597\nEOF", "zsh <<'EOF'\nls\nEOF"]);
        assert_denied(&[
            ("bash <<'EOF'\ngit push --force\nEOF", "git push"),
            ("bash -s <<'EOF'\nrm x.rs\nEOF", "rm changes x.rs"),
            ("zsh -s <<EOF\ngit push origin HEAD\nEOF", "git push"),
        ]);
    }

    #[test]
    fn cargo_and_rustfmt_writes_are_denied_and_their_checks_run() {
        assert_denied(&[
            ("cargo rm serde", "cargo rm"),
            ("cargo init", "cargo init"),
            ("cargo new crates/foo", "cargo new"),
            ("rustfmt crates/core/src/hooks.rs", "rustfmt changes crates/core/src/hooks.rs"),
        ]);
        assert_runs(&["rustfmt --check x.rs", "rustfmt --help"]);
    }
}
