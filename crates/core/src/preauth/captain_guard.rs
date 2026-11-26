//! `pinned captain-guard`: Claude Code's PreToolUse hook on every Bash call of a Captain whose
//! project is Manage only. That Captain starts without the file tools and loads this hook from
//! `hooks::CAPTAIN_SETTINGS`, so the shell is no way around them: its session reads, checks and
//! merges, and a worker changes files.
//!
//! It denies a command that writes a file, or changes the checkout or its history, wherever the
//! line runs it. It never asks the owner. A write to a standard device or below a temp folder, the
//! session's scratch folder among them, runs. It reads the line with the never list's parser and
//! makes no model, socket, hub or network call.
//!
//! It stops a habit, not an evasion: code an interpreter runs (`python -c`, `node -e`, a script
//! file) passes, and so does what a tool writes in doing its own job (`cargo` and `target/`).

use super::never::{chdir, commands, git_sub, lex, tgt, walk, Cmd, Word, SHELLS};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SUBCOMMAND: &str = "captain-guard";

const REASON: &str = "pinned: this Captain is manage-only, so Bash is for reading and checking. Hand this change to a worker with delegate_task.";
/// The devices that keep nothing written to them.
const DEVICES: [&str; 3] = ["/dev/null", "/dev/stdout", "/dev/stderr"];
/// The git subcommands that change the checkout or its history in every form.
const GIT_CHANGES: [&str; 15] = ["commit", "push", "apply", "am", "merge", "rebase", "cherry-pick", "revert", "reset", "restore", "checkout", "switch", "clean", "rm", "mv"];
/// The flags that make `git tag` list or check tags.
const TAG_LISTING: [&str; 10] = ["list", "verify", "contains", "no-contains", "points-at", "merged", "no-merged", "sort", "format", "column"];

/// Runs inside the hook process. Prints a deny, or nothing so Claude Code's own permissions decide.
pub fn run() {
    if let Some(output) = decide(&crate::hooks::hook_payload(), &crate::machine::home(), &temp_roots()) {
        println!("{output}");
    }
}

/// The hook's stdout for one PreToolUse payload: a deny for a Bash command that changes a file or
/// the checkout, else `None`. `temp` holds the temp folders, resolved.
pub fn decide(payload: &str, home: &Path, temp: &[PathBuf]) -> Option<String> {
    let hook: Value = serde_json::from_str(payload).ok()?;
    if hook["tool_name"] != "Bash" {
        return None;
    }
    let command = hook["tool_input"]["command"].as_str()?;
    let cwd = PathBuf::from(hook["cwd"].as_str()?);
    let found = Guard { home, temp }.line(command, Some(cwd), 0)?;
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
    temp: &'a [PathBuf],
}

impl Guard<'_> {
    /// What the first change on `line`, run from `cwd`, is.
    fn line(&self, line: &str, cwd: Option<PathBuf>, nest: u8) -> Option<String> {
        if nest > 8 {
            return None;
        }
        // The folder at each depth of parentheses: a `cd` inside doesn't leave them.
        let mut folders = vec![cwd];
        for seg in lex(line, &self.home.to_string_lossy(), None) {
            let outer = folders.last().cloned().flatten();
            folders.resize(seg.depth + 1, outer);
            let cwd = folders[seg.depth].clone();
            let redirected: Vec<Word> = seg.redirs.iter().filter(|r| r.write).map(|r| r.target.clone()).collect();
            let cmds = commands(&seg.words);
            let shell = cmds.iter().any(|c| SHELLS.contains(&c.prog.as_str()));
            let found = self
                .change(&cwd, "redirect", &redirected)
                .or_else(|| cmds.iter().find_map(|c| self.command(&cwd, c, nest)))
                .or_else(|| seg.heredoc.iter().filter(|_| shell).find_map(|body| self.line(body, cwd.clone(), nest + 1)));
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
        match c.prog.as_str() {
            "git" => self.git(cwd, c),
            "find" => self.find(cwd, c, nest),
            "patch" => Some("patch".to_owned()),
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
    /// the shell runs.
    fn exempt(&self, cwd: &Option<PathBuf>, w: &Word) -> bool {
        if DEVICES.contains(&w.text.as_str()) {
            return true;
        }
        // A `$` anywhere: a value stuck to its flag (`--output=$OUT`) carries no mark of its own.
        if w.var_at.is_some() || w.text.contains('$') || w.glob_at.is_some_and(|at| w.text[at..].split('/').any(|part| part == "..")) {
            return false;
        }
        // What a glob matches is below the folder before it, which may be a temp folder itself.
        tgt(cwd, w).path.is_some_and(|path| self.temp.iter().any(|root| path.starts_with(root) && (w.glob_at.is_some() || path != *root)))
    }

    fn git(&self, cwd: &Option<PathBuf>, c: &Cmd) -> Option<String> {
        let (_, sub, rest) = git_sub(c)?;
        // `git diff --output=FILE` writes where it is told.
        if let Some(found) = rest.long_value("output").and_then(|out| self.change(cwd, "git", &[out])) {
            return Some(found);
        }
        let first = rest.positionals().first().map_or("", |w| w.text.as_str());
        let changes = match sub {
            "pull" => !rest.has("", &["ff-only"]),
            "stash" => !matches!(first, "list" | "show"),
            "worktree" => matches!(first, "add" | "remove"),
            "branch" => rest.has("dDmMfC", &["delete", "move", "force"]),
            "tag" => rest.has("d", &["delete"]) || (!first.is_empty() && !rest.has("lnv", &TAG_LISTING)),
            _ => GIT_CHANGES.contains(&sub),
        };
        changes.then(|| format!("git {sub}"))
    }

    /// `find` changes what it matches below its start folders, with `-delete` or a command it runs.
    fn find(&self, cwd: &Option<PathBuf>, c: &Cmd, nest: u8) -> Option<String> {
        let paths = c.args.iter().skip_while(|w| matches!(w.text.as_str(), "-H" | "-L" | "-P"));
        let mut starts: Vec<Word> = paths.take_while(|w| !(w.text.starts_with('-') || w.text == "(" || w.text == "!")).cloned().collect();
        if starts.is_empty() {
            starts.push(Word { text: ".".into(), ..Word::default() });
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
                if let Some(found) = commands(&words).iter().find_map(|c| self.command(cwd, c, nest)) {
                    return Some(found);
                }
            }
        }
        None
    }
}

/// The files `c` writes, when its program changes the files it names, and `None` for any other call.
fn written(c: &Cmd) -> Option<Vec<Word>> {
    let named = || c.positionals().into_iter().cloned().collect::<Vec<_>>();
    match c.prog.as_str() {
        "rm" | "rmdir" | "unlink" | "shred" => Some(named()),
        "cp" | "mv" | "ln" | "install" | "tee" | "dd" => Some(c.write_args()),
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
        "curl" => {
            let mut files = c.write_args();
            // `-O` saves under the remote name, in the folder the command runs in.
            if saves_by_name(c) {
                files.push(Word { text: ".".into(), ..Word::default() });
            }
            downloaded(files)
        }
        "wget" => downloaded(c.write_args()),
        _ => None,
    }
}

/// What a downloader told to write `files` writes: `-` is its standard output.
fn downloaded(mut files: Vec<Word>) -> Option<Vec<Word>> {
    files.retain(|w| w.text != "-");
    (!files.is_empty()).then_some(files)
}

/// Whether curl has `-O` or `--remote-name`. A flag that takes a value ends its cluster, so the
/// `O` of `-XPOST` is none.
fn saves_by_name(c: &Cmd) -> bool {
    c.args.iter().any(|w| match w.text.strip_prefix('-') {
        Some("-remote-name" | "-remote-name-all") => true,
        Some(cluster) if !cluster.starts_with('-') => cluster.chars().take_while(|flag| !"AbcCdDeEFhHKmoPQrTtuUwxXyYz".contains(*flag)).any(|flag| flag == 'O'),
        _ => false,
    })
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
        /// The one temp folder the fixture's guard knows.
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
            let out = decide(&bash(&f, &command, "default"), &f.home, std::slice::from_ref(&f.temp));
            assert_eq!(held(out), denied(&sub(&f, found)), "{command}");
        }
    }

    fn assert_runs(commands: &[&str]) {
        let f = fx();
        for command in commands {
            let command = sub(&f, command);
            assert_eq!(decide(&bash(&f, &command, "default"), &f.home, std::slice::from_ref(&f.temp)), None, "{command}");
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
            ("ln -s {temp}/a link", "ln changes link"),
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
            ("git add -A && git commit -m x", "git commit"),
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
            ("cd {temp} && git -c user.name=a commit -m x", "git commit"),
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
            ("cargo test > \"$TMPDIR/log\"", "redirect changes $TMPDIR/log"),
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
        let tmpdir = std::env::temp_dir().join("x/log").display().to_string();
        for target in ["/tmp/x/log", "/private/tmp/x/log", tmpdir.as_str()] {
            assert_eq!(decide(&payload(format!("cargo test 2> {target}")), &f.home, &roots), None, "{target}");
        }
        for command in ["rm -rf /tmp", "echo x > /nowhere/checkout/a.rs", "echo x > a.rs"] {
            assert!(decide(&payload(command.to_owned()), &f.home, &roots).is_some(), "{command}");
        }
    }

    #[test]
    fn every_mode_is_denied_and_only_bash_calls_are_read() {
        let f = fx();
        let temp = std::slice::from_ref(&f.temp);
        for mode in ["default", "acceptEdits", "plan", "bypassPermissions", "auto", "dontAsk"] {
            assert_eq!(held(decide(&bash(&f, "git commit -m x", mode), &f.home, temp)), denied("git commit"), "{mode}");
        }
        for payload in [
            json!({ "tool_name": "Write", "tool_input": { "file_path": f.work.join("a.rs"), "content": "x" }, "cwd": f.work }).to_string(),
            json!({ "tool_name": "Bash", "tool_input": { "command": "rm x.rs" } }).to_string(),
            "not json".to_owned(),
        ] {
            assert_eq!(decide(&payload, &f.home, temp), None, "{payload}");
        }
    }
}
