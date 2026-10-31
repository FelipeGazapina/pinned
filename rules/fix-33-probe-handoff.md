# Probe, then replace the same process

PR 33, branch `feat/update-all-machines`. An independent review said the new binary must not become the daemon while the old process is still the service. This pass changed that handoff. The signature check was left as it was.

Nothing here was released, tagged, merged to `main`, or pointed at the production hub. `railway-hub` was not moved. The `hub-breaking-change` environment was not approved. No user machine, Railway service, or installed app was touched.

## Why the child cannot be the daemon

`PINNED_POST_UPDATE` was set and never read, so the child ran a full node. That listens on its own hook socket, resumes sessions, and writes `sessions.json` while the parent is still the service. A failed probe that is only killed by pid can leave Claude processes behind. `Command::spawn` failing returned before the backup was restored, so a second update copied the new bytes onto `name.bak` and lost the previous binary. The Linux unit is `Type=simple` with `Restart=always` and the default `KillMode=control-group`. A parent that exits makes systemd kill that cgroup, including the child, and start `ExecStart` again.

The child is a probe. The parent replaces itself.

## What the node does

`node` returns from `prove_connected` when `PINNED_POST_UPDATE` is set, before it prepares a launch. That dials the hub once, waits for hello, writes `name.ready`, and exits. The ordinary connection no longer writes the ready file.

`install` reports `Updating` with an empty `to` and returns that status before any download. A background thread downloads, verifies, and stages. The hub call therefore returns while the files are still coming down. Later phases go out as events the hub already forwards.

`stage` writes `name.unproven` after the new bytes are in place. If that marker is still there, the next stage restores `name.bak` first. A missing backup is an error. Every failure after stage, including a spawn error and a probe timeout, restores the backup and removes the marker. The timeout checks the ready file before it kills the probe. A ready file that appears after the kill is not success. The probe is started in its own process group and that group is killed.

On ready, the parent reports `Restarting`, quits the machine the way a restart already did, and `exec`s the new binary. The process id stays the same. If `exec` returns, the parent restores the backup and resumes the saved sessions itself.

Each download stops at 128 MB, before verify.

The machines page waits for a final phase (`up_to_date`, `failed`, `cannot`, `restarting`), a rejected call, or 21 minutes. A rejection is stored on that row as `failed` with the error message. This machine is asked only after the others finish that wait. The header tests call `install_update` and check the button text and status for `failed`, `cannot`, `up_to_date`, and an empty result.

## Tests that failed first

Written on `d92c791`, committed as `cf2d2ba`, then fixed in `eaff741`.

A second stage copied the unproven bytes onto the backup. The assertion was `left` `[117, 110, 112, 114, 111, 118, 101, 110]` (`unproven`) and `right` `[114, 117, 110, 110, 105, 110, 103]` (`running`).

A binary that could not be spawned left the new bytes in place. The destination kept 84990480 bytes starting `[cf, fa, ed, fe, 0c, 00, 00, 01]`.

A rejected update call threw `Cannot read properties of undefined (reading 'state')` because the rejection was discarded. An `updating` phase started this machine immediately: actual calls `['a','self']`, expected `['a']`.

With the hub down, the real `pinned-server` probe resumed the session as well as the parent. Resume count left 2, right 1. The second resume's parent was the probe.

On success, the resumed session's parent was the child, not the original pid. Left `20909`, right `20389`.

After the fix, `cargo test -p pinned-server --test update_handoff` passed both cases: hub down restores the binary and the hook socket still accepts; a ready probe exits and the same pid resumes the session once.

## Live run

A local hub, a throwaway minisign key, and a signed release of a 0.16.1 binary. The running node was 0.16.0, pid 10361. After the update the same pid was still alive, the on-disk binary matched the 0.16.1 release, and the hub log showed that machine disconnect and connect as `pinned 0.16.1`. The fake Claude session logged `--resume sess-live` twice, both with parent pid 10361: once at boot and once after `exec`.

The rollback used pid 14200. The hub was killed after the `Updating` reply and before the probe was accepted. The binary hash before and after was `76be620a89b62ef28ff934c5a6cd76ae6e21437c4ff201cce79d9a8c42f4e3c7`. The session resumed once, parent pid 14200. The probe's last line was a connection refused with no retry. The parent kept retrying and stayed alive.

systemd itself was not started. The same-pid `exec` is the stand-in for `Type=simple`. The page was not opened in a browser. `node --test` loads the functions.

## Suites after merging `origin/main`

Merge commit `4bfc24e`. `node --test ui/*.test.js` passed 70. `cargo test -p pinned-core --lib` passed 510, 3 ignored. `cargo test -p pinned-server --lib` passed 38. `cargo test -p pinned-server --test hub` passed 42 on `postgres://felipe@127.0.0.1:55101`. `cargo test -p pinned-server --test update_handoff` passed 2. `wire-contract snapshot current` did not change `current.json`. `check --base origin/main` exited 0. New breaks: none. Already shipped: `Connector.machine` type changes from 0.8.0 through 0.12.0.

The tab snapshot tests needed an empty `viewListeners` list because `showMachines` notifies that list. That one-line harness fix is in the merge commit.
