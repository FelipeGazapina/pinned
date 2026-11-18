# U8 review diff: pinned computes it from state the worker cannot edit

## Design

1. The folder is the task record's `cwd`, set when the task started. The tab's live cwd is never read.
2. The base is the commit `note_base` recorded at the start. An unborn HEAD at the start records the
   empty tree, so the first commit is reviewed. A task with no recorded base is `not_reviewed`.
3. Each diff runs git in a NEUTRAL repository that pinned makes for the review under its own config
   folder (`reviews/<id>/diff-<n>`) and removes afterwards. Every git call clears the inherited `GIT_*`
   variables and sets `GIT_DIR=<neutral>`, `GIT_WORK_TREE=<folder>`, `GIT_CONFIG_NOSYSTEM=1`,
   `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_NO_REPLACE_OBJECTS=1`, `GIT_OPTIONAL_LOCKS=0`, a neutral `HOME`
   and `XDG_CONFIG_HOME`, `core.fsmonitor=false`, `diff.external=`, `core.autocrlf=false`, and a 20 s limit.
   The worker's repo config, `info/exclude`, `info/attributes`, index, replace refs and filter
   drivers are never read, because none of them is in the neutral repository.
4. The neutral repository's `objects/info/alternates` names the worker's object store (found by reading
   `.git`, or the `gitdir:` file and `commondir`, never by running git there). Objects are content
   addressed, so the recorded base cannot be faked; a missing or corrupt object fails git, and the
   round is `not_reviewed`.
5. The snapshot: `read-tree <base>`, then `add -A` from the folder (its `.gitignore` files apply, so
   `target/` is not read), then `add -f` of every blob path in the tree of the worker's current HEAD
   (a force-added, ignored, committed file), then `add -f` of every untracked ignored `.gitignore`
   (one that ignores itself). `write-tree` names the snapshot.
6. Untracked nested repositories (a `sub/` entry from `ls-files -o`) are taken out of the index and
   listed in the round's note as "not read: nested repository <path>"; they count as cut, so the
   round is never `passed`.
7. The diff is `diff --cached --text --no-ext-diff --no-textconv --no-renames <base>`: base tree
   against the snapshot, one file at a time, smallest first, at most 96 KiB, the rest cut.
8. Freshness: when the review answers, pinned takes the snapshot again. Another tree, or other
   nested repositories, makes the round `not_reviewed`: "the worker changed its files while the
   review ran". No next round starts.
9. Switch Off, or no rules: return before any git call. Switch On with an empty diff writes a
   `nothing_to_review` round (new `ReviewState` variant), never `passed`. Auto with an empty diff
   writes no round.

## Instances and what closes them

| # | Instance | Closed by |
|---|---|---|
| 1 | tab cwd moved to `.claude/worktrees/x` | 1 (task record cwd) |
| 2 | file changed while the review ran | 8 (snapshot again, tree compared) |
| 3 | `git replace <base> <HEAD>` | 3 (neutral repo, `GIT_NO_REPLACE_OBJECTS`) |
| 4 | `info/attributes` `* -diff` | 3 (no worker `info/`), 7 (`--text`) |
| 5 | `info/exclude` `*` | 3 (no worker `info/exclude`) |
| 6 | assume-unchanged, skip-worktree, `core.worktree` | 5 (fresh index), 3 (no worker config; work tree is the folder) |
| 7 | `filter.x.clean` | 3 (no filter driver configured, so no program runs) |
| 8 | untracked nested repo | 6 |
| 9 | empty diff under On; no git under Off | 9 |
| 10 | force-added ignored committed file | 5 (HEAD tree paths with `-f`) |
| 11 | self-ignoring new `.gitignore` | 5 (ignored `.gitignore` files with `-f`) |
| 12 | unborn HEAD at start | 2 (empty tree base) |
| 13 | a .gitignore the worker added or changed hides a file | 5: `check-ignore` names the deciding rule; one from a .gitignore not byte-equal to the base's (or no rule) is "not read: ignored by a changed .gitignore: <path>" |
| 14 | a file listed as changed whose diff is empty | 7: counted as cut, "not read: <path> is listed as changed but its diff is empty" |
| 15 | a file changed inside a tracked submodule (160000 entry) | 6: every 160000 entry is a nested repository |

The worker's HEAD sha is read once with `git --git-dir=<worker gitdir> rev-parse` under the same
cleared environment; it only adds paths to the snapshot, so a false HEAD can at most leave an
ignored file out, which the report names as out of scope.

## Known limits

The Reviewer trusts the worker's .git; a worker that alters its own .git can hide changes.

- Forged base objects (B1): a worker that writes objects into its own store under the base's ids before the review can make the base say what it likes.
- A soft reset that drops a committed ignored file from HEAD (B3): HEAD only adds paths to the snapshot, so a file ignored as at the base and no longer in HEAD is left out unnamed.
- In-tree `.gitattributes` normalisation (`text`, `eol`, `ident`): the folder's own `.gitattributes` still applies when files are added, so a change it normalises away is not in the diff.
- The 20 s limit on each git call: on a large repository the snapshot can run out of time, and the round is `not_reviewed`.

A check that the base's objects hash to the recorded base (the base-hash check) is planned for 0.18.8.

## Judgment

Yes: this closes the class in one round. Every input the diff reads is the folder's files, the
recorded base sha and object bytes checked by their hash. The remaining limits (same OS user, the
worker-writable `origin`, a restart mid-review) are release-note lines, not code.
