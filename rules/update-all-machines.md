# Update every machine

A signed-in person, on any of their machines or on the web client, presses one control and every machine of theirs that is behind the newest release installs it and comes back online. That includes the machine they pressed it on. The web client has nothing to install. It stays connected and shows each row.

## What the person sees

The machines page has "Update every machine". A row that can be asked also has "Update". Each row shows one status. Up to date names the version. Updating and restarting name the version they are moving to. A failure names the reason. A machine that cannot install a release names why, including "update by hand" when the installed app does not know the command.

The page asks the other online machines first. It updates the machine the page is running on after each of those reports `up_to_date`, `failed`, or `cannot`, the call fails, or 21 minutes pass. `restarting` is not that end. When the wait runs out, the row becomes `failed` with "the update timed out". A failed call is written on that machine's row. A second press while that is running does nothing. An Upstash box is left out. The hub also refuses one that is online, because the hub replaces that box itself.

The header Update button still installs only the desktop app it is on.

## Data shape

The command is `Call::UpdateSelf` with no fields. It rides the existing client call. The hub only forwards it to a machine in that same account.

`UpdateStatus` is one enum. `UpToDate { version }`, `Updating { to }`, `Restarting { to }`, `Failed { to, reason }`, `Cannot { reason }`. The machine is the only writer. The hub keeps the last phase in memory and overlays it on `MachineView.update`. A missing status is not up to date.

`MachineView.version` is the string from the last machine auth. It is a `machines.version` column, added with `alter table ... add column if not exists` so an existing database grows it. An in-progress phase is not stored in Postgres.

The machine sends its last phase again when it connects, including a finished failure. The hub uses that snapshot. A hub restart therefore shows the phase again once the machine reconnects. A process that has never reported a phase sends none. An idle connect after updating or restarting then becomes up to date when the versions are the same release, and failed with "came back on {version}" when they are not. One leading `v` does not make them different. `v0.17.0` and `0.17.0` are the same release.

## Installers

The desktop app runs the same updater check and download as the Update button, then relaunches. Only the `update_self` call that armed the restart may relaunch, and only after its reply is flushed. Another call finishing in the meantime does not.

A node on Linux x86_64 or aarch64 downloads the release files below. It installs only when the signed version is newer than the version compiled into the binary it is running. The same number, and an older published number, are up to date and do not restart. A node already on the hub's newer binary can still read the previous public `latest.json` on release day.

The node answers the hub call with `Updating` before it downloads. The downloads run on a background thread. Each downloaded file stops at 128 MB. A bad signature or checksum leaves the old binary running.

It copies the running binary to `name.bak`, writes `name.unproven` with a boot count of `0`, then renames the new bytes onto the live path. The marker is durable before that rename. A start that finds the marker puts the backup back, so a crash between the marker and the rename cannot copy unproven bytes over `name.bak`. A later attempt that finds the marker restores first, for the same reason.

There is no second process. The parent quits, which saves sessions and reaps each terminal on this thread, then `exec`s the new bytes in the same pid. That exec sets `PINNED_PROOF_BOOT=1`. If `exec` returns, the parent restores `name.bak` and resumes the saved sessions itself.

The new process is the machine. Its first hub Hello on that one socket removes `name.unproven`. That removal is the proof. The hub still replaces the previous socket on Auth, so a watching client sees one offline gap for the exec, then the new version.

`node` checks the marker before it loads the account. A start without `PINNED_PROOF_BOOT` restores `name.bak` and execs those bytes. That is a parent that died after stage, and a systemd `Restart=always` after a crash. A start with `PINNED_PROOF_BOOT` records one boot and runs. A second such boot restores. If Hello does not arrive within 60 seconds, the same process restores and execs the old bytes. The deadline and the Hello removal share one lock.

The desktop app returns `Updating` from `install` before the download, so the hub call is not held for the download. When that download ends in `Restarting`, it relaunches. An idle reconnect while the stored phase is `Updating` or `Restarting` with an empty `to` becomes `Failed`.

A second press while the gate is busy returns the phase already in progress. A finished failure, refusal, or up-to-date phase clears that gate in the same process, so a later press can try again. The header Update button shows that result and enables itself again, except while a restart or download is still in progress.

Release downloads use a 5 minute timeout so a 20 to 40 MB binary can finish on a slow link.

This Mac answers `Cannot` with "no release binary for macos aarch64" and does not download. A test build can set `PINNED_RELEASE_BASE`, `PINNED_RELEASE_PUBKEY`, and `PINNED_RELEASE_TRIPLE` to install from a local release. An empty triple keeps the operating-system check.

## Limits

An app older than this change cannot run `update_self`. The hub turns that refusal into "update by hand". That machine has to be updated once by someone at the keyboard.

The GitHub release workflow does not yet publish the Linux musl binaries or `SHA256SUMS`. Until that ships, a Linux node that tries the public release fails the download and keeps the binary it is running. The stacked release change has to publish the files in the contract below. This client does not change how a minisign signature is decoded.

## Release files

The node reads these names from `https://github.com/FelipeGazapina/pinned/releases/latest/download/`.

`latest.json` is a JSON object with a `version` string. The version is a semver number. One leading `v` is allowed.

`SHA256SUMS` is text. Each line is 64 hex digits, whitespace, then the file name. The GNU binary form, one asterisk before the name, is the same name. The signature is over the exact bytes of this file.

`SHA256SUMS.minisig` is the raw minisign signature text. `Signature::decode` already accepts that text. A base64 `.sig` is a different format and this client does not read it.

`SHA256SUMS` must list `latest.json` and the musl binary for the triple this node is running. The names are `pinned-server-x86_64-unknown-linux-musl` and `pinned-server-aarch64-unknown-linux-musl`. The node checks `latest.json` against that signed list before it trusts the version. A version that is not listed is not a release, and the node does not install.

The signature must be a current prehashed minisign signature. `allow_legacy` is false. A legacy signature, one that is not prehashed, is rejected. The public key is the updater key in `tauri.conf.json`.

Nothing in this change was released, tagged, or pointed at the production hub.
