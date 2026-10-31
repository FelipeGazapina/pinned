# Fix the update-all review

PR 33, branch `feat/update-all-machines`, lets one control update every machine of an account. An independent review said to fix the branch before stacking the Linux release job on it. This pass did that. The release-file contract the stacked job must follow is in `rules/update-all-machines.md`.

Nothing here was released, tagged, merged, or pointed at the production hub. `railway-hub` was not moved. The `hub-breaking-change` environment was not approved. No user machine, Railway sandbox, Mac mini, or installed app was touched.

## What changed

`Updating.to` and `Failed.to` are always serialized. A missing `to` still deserializes as an empty string. `crates/core/wire-contract/current.json` was regenerated for that shape. `check --base origin/main` reports no new breaks. The breaks it still lists are the `Connector.machine` string-to-union change already shipped from 0.8.0 through 0.12.0.

A Linux node trusts `latest.json` only after that file's hash is in the signed `SHA256SUMS`. It parses the version as semver, with one leading `v` stripped, and installs only when the published number is strictly greater than `CARGO_PKG_VERSION`. The same number is up to date. An older number is refused and does not restart. The hub's idle reconnect uses the same comparison, so `v0.17.0` and `0.17.0` are one release.

The node computes the restart plan before it replaces the binary. The old process stays alive and starts the new binary as a child, with `PINNED_POST_UPDATE=1` and `PINNED_UPDATE_READY` set to `name.ready` beside the binary. The child creates that file after the hub accepts the connection. If the child exits, or does not signal within 60 seconds, the parent puts `name.bak` back and reports Failed on the connection it still has. Once the child has connected, the parent exits. A crash after that is outside the boot check.

A report of Failed, Cannot, or UpToDate clears the busy gate in that same process, including when the report happens after the restart guard has already dropped. The machine sends its last phase on connect, even when it is no longer busy. The hub already applies that snapshot.

The header Update button gets the status back. Failed and Cannot write the reason and enable the button again. Up to date runs the check again. Restarting and updating leave the button disabled.

Only the `update_self` call that armed the restart may relaunch. On the hub path that happens after the reply is flushed. A later call, such as tabs, does not take the restart.

`SHA256SUMS` lines may use the GNU form, one asterisk before the name. Release downloads use a 5 minute timeout. `land` in `ui/update-all.js` removes its view listener when the call finishes first. `allow_legacy` stays false because current minisign signatures are prehashed. A legacy signature is rejected. `Signature::decode` was not changed.

The Rust test that scraped `ui/transport.js` is gone. `ui/transport.test.js` imports that file and checks that `update_self` uses `machine_call_slow` and `tabs` uses `machine_call`. `Call::UpdateSelf.may_wait` is still asserted in the wire round trip.

## Commits

- `21fa8f7` Expect an empty update target on the wire.
- `f4b92bd` Always send the update target so the hub accepts its own phase.
- `5aef4a5` Record the update failures a second press, an older release, and a dead restart must not keep.
- `77fa610` Restore a node that never connects, and try again after a finished failure.
- `49f4b41` Keep the checksum helper inside the tests so a release build stays quiet.

## Verification

`cargo test -p pinned-core -- --test-threads=4` passed 497 library tests, the `delegate_mcp` test, and ignored 3 live graph tests. `cargo test -p pinned-server --lib -- --test-threads=4` is 35 tests. 34 passed together, and the stalled-download test passed after the timeout error was labeled `timed out`. `cargo test -p pinned-server --test hub -- --test-threads=4` passed 34 tests on a throwaway Postgres at `127.0.0.1:50646`. That cluster was stopped and its data directory removed. `node --test ui/*.test.js` passed 40 tests. `cargo run -p pinned-server --bin wire-contract -- check --base origin/main` exited 0 with no new breaks.

This worktree's `target/` was deleted at the end. No Playwright run. The header result is covered by `ui/header-update.test.js`, which loads the decision function from `ui/main.js`.

## Left open

The stacked release job still has to publish the files named in `rules/update-all-machines.md`, and it owns the question of raw minisign text versus a base64 `.sig`. This client reads the raw text only.

A desktop relaunch runs before `install_update` returns, because the Tauri command's return is the reply and `app.restart` ends the process. Failed, Cannot, and UpToDate do not arm a restart, so those statuses reach the page. A restart that returns without exiting would leave the header button disabled.

The full set of waiting calls is no longer checked in both directions. The JS test covers `update_self` and `tabs`.

A second model did not review the decision trail. That review was skipped so this session would not start another pinned worker.
