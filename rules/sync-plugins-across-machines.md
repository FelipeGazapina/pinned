# Sync plugins across machines

A Claude Code plugin or user skill installed on one machine shows up in pinned sessions on the account's other machines. The other machines do not install it.

## What is shared

Each enabled plugin is the join of `settings.json` `enabledPlugins[id] == true` and the last `installPath` in `plugins/installed_plugins.json`. The copy is that folder's `.claude-plugin/plugin.json`, `skills/`, `commands/`, and `agents/`. The hub path is `plugins/<id>/<relative>`, with no cache version in the path. `plugins/<id>/.pinned-plugin.json` stores `version` and `gitCommitSha`.

`~/.claude/skills` is shared as `user-skills/<relative>`, except the top-level `synced` folder. On each machine those files land in that Claude home's `skills` folder. Claude Code loads that folder itself. It has no flag for a second skills directory.

Disabled plugins, older install records, `plugins/synced`, marketplace checkouts, `node_modules`, `.git`, `target`, `dist`, `build`, and `vendor` are not copied. Names match without regard to case. An install path that is a symlink is not copied. A symlink inside a tree is not followed.

## What is not shared

These names never go to the hub, including when they sit inside a plugin tree: `settings.json`, `settings.local.json`, `.claude.json`, `plugins/installed_plugins.json`, any name that starts with `.env`, `credentials.json`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519`, `*.pem`, `*.key`, and any path through `secrets` or `.secrets`. A path with `..` is still rejected. The `skills` directory is not read or written when that directory is a symlink.

## How a session loads them

The files live under the reserved project key `pinned:plugins` in `project_files`. The same `SyncList`, `SyncPush`, and `SyncPull` calls are used. No wire type was added. Each file entry has an `executable` field, so a hook that is executable on the publishing machine stays executable on the others. See `rules/executable-plugin-hooks.md`.

Each machine keeps one folder per plugin id under its config `synced-plugins` directory. `Launch::command` adds one `--plugin-dir` for each folder whose plugin is not already enabled locally. `commands::list` reads those same folders. A running session picks the change up on restart.

Sync runs once per Claude home in the link loop, on the same timer as project sync. Newer mtime wins. A user skill that already exists on this machine, and that this machine has not synced before, stays when the hub has a different copy. It is not replaced and it is not deleted, and deleting it on the other machine does not copy this one back. Two enabled installs with the same mtime keep the synced copy already on disk, so they stop sending that file back and forth. Disabling a plugin deletes it on the other machines through the existing empty-sha path. A file removed from an install that is still enabled is left in place, so two machines that both publish it do not delete each other's copy. An old hub that rejects the push is logged. The session still starts.

A test binary whose path contains `deps` does not sync the developer's real Claude home.
