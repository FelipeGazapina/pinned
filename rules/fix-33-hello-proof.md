# The new binary is proven by its own Hello

PR 33, branch `feat/update-all-machines`. Two earlier fixes kept a second process that signed in as the live machine. The hub's `serve_machine` runs `went_offline` then `came_online` on every Auth and keeps one socket. That sign-in stole the parent's socket. A Hello slower than the parent's reconnect rolled back a good release. A faster one showed the web client offline and an empty tab list. The handoff tests used a fake hub that allowed two sockets, so they could not catch it.

The probe is gone. `prove_connected`, `watch_new_binary`, `PINNED_POST_UPDATE`, and `crates/server/tests/update_handoff.rs` are deleted. The hub's one-socket rule is unchanged.

Nothing here was released, tagged, merged to `main`, or pointed at the production hub. `railway-hub` was not moved. The `hub-breaking-change` environment was not approved. No user machine, Railway service, or installed app was touched.

## What a node does

`stage` copies the running bytes to `name.bak`, writes `name.unproven` as `0`, then renames the new bytes onto the live path. The marker is on disk before that rename.

`hand_over` reports `Restarting`, quits (sessions are saved first, and each terminal is waited on this thread), and `exec`s the new bytes in the same pid with `PINNED_PROOF_BOOT=1` and `PINNED_UNPROVEN` set to the marker. If `exec` returns, it restores `name.bak` and resumes the saved sessions.

`link` calls `boot::commit` after Hello. That removes the marker. A deadline that still sees the marker restores and execs the old bytes. The default wait is 60 seconds. Tests set `PINNED_PROOF_WAIT`.

`node` calls `enter` before it loads the account. No marker means run. A marker without `PINNED_PROOF_BOOT` restores and execs the backup. A marker with that variable records the boot. Boot `1` runs and arms the deadline. Any later boot restores. A published version that is older than or equal to the compiled version is `UpToDate` and does not download.

`PINNED_VERSION` exists so a test asset can report `9.9.9`. Rollback clears it. The command-line version stays the compiled one.

## The other review items

`Session::drop` kills the child and waits on the calling thread. A wait thread is gone after `exec`, and the child stays a zombie.

The desktop `install` returns `Updating` and downloads afterwards. `Restarting` then calls `restart_now`. There is no automated test for that Tauri path.

`reconcile` turns an idle connect over `Updating` or `Restarting`, including an empty `to`, into `Failed` when the version is not that release.

`ui/update-all.js` treats only `up_to_date`, `failed`, and `cannot` as done. The 21-minute cap writes `failed` with "the update timed out".

The lib tests that need a binary no longer look for `target/debug/pinned-server`. The cannot-start test uses a file that is not executable. `Command::exec` changes directory before a failed exec, so that test keeps the process directory.

`quitting_through_the_hub_sends_an_empty_tab_list_before_disconnect` passed in the hub suite. It was not changed. The flake the reviewer saw once was not reproduced.

## Tests

The failing tests were committed before this fix. Reverting each guarded line failed the test named with it, and restoring the line passed it.

A good update against the real hub, which allows one socket, reports `9.9.9` and one offline gap. A parent death after stage, a proof boot that never says Hello, and a proof boot that is killed all end on the old bytes with no marker. `published <= running` is `UpToDate`. An empty `Updating` on an idle connect is `Failed`. A `restarting` row that never finishes is `failed`. Dropping a session does not leave a zombie.

`cargo test -p pinned-server --lib` passed after `rm -rf target`, so the lib suite does not need a previously built binary.

`enter` runs before the account is loaded. `stage` writes the marker, then renames the new bytes onto the live path. If `name.bak` is already gone when that marker is still there, restore exits and a `Restart=always` unit starts the same file again. That state is not one `stage` writes.
