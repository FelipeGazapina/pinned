# Update every machine

A signed-in person, on any of their machines or on the web client, presses one control and every machine of theirs that is behind the newest release installs it and comes back online. That includes the machine they pressed it on. The web client has nothing to install. It stays connected and shows each row.

## What the person sees

The machines page has "Update every machine". A row that can be asked also has "Update". Each row shows one status. Up to date names the version. Updating and restarting name the version they are moving to. A failure names the reason. A machine that cannot install a release names why, including "update by hand" when the installed app does not know the command.

The page asks the other online machines first, then the machine the page is running on. A second press while that is running does nothing. An Upstash box is left out. The hub also refuses one that is online, because the hub replaces that box itself.

The header Update button still installs only the desktop app it is on.

## Data shape

The command is `Call::UpdateSelf` with no fields. It rides the existing client call. The hub only forwards it to a machine in that same account.

`UpdateStatus` is one enum. `UpToDate { version }`, `Updating { to }`, `Restarting { to }`, `Failed { to, reason }`, `Cannot { reason }`. The machine is the only writer. The hub keeps the last phase in memory and overlays it on `MachineView.update`. A missing status is not up to date.

`MachineView.version` is the string from the last machine auth. It is a `machines.version` column, added with `alter table ... add column if not exists` so an existing database grows it. An in-progress phase is not stored in Postgres.

The machine sends its last phase again when it connects, including a finished failure. The hub uses that snapshot. A hub restart therefore shows the phase again once the machine reconnects. A process that has never reported a phase sends none. An idle connect after updating or restarting then becomes up to date when the versions are the same release, and failed with "came back on {version}" when they are not. One leading `v` does not make them different. `v0.17.0` and `0.17.0` are the same release.

## Installers

The desktop app runs the same updater check and download as the Update button, then relaunches. Only the `update_self` call that armed the restart may relaunch, and only after its reply is flushed. Another call finishing in the meantime does not.

A node on Linux x86_64 or aarch64 downloads the release files below. It installs only when the signed version is strictly newer than its own. The same number is up to date. An older number is refused and does not restart. It plans the restart before it replaces the binary. It keeps the previous binary as `name.bak`, starts the new binary as a child, and stays running. The child gets `PINNED_POST_UPDATE=1` and `PINNED_UPDATE_READY` set to `name.ready` beside the binary. The child creates that file after the hub accepts it. If the child exits, or does not signal within 60 seconds, the parent puts `name.bak` back and reports Failed on the connection it still has. It does not replace its own process, so a binary that dies immediately can still be restored. Once the child has connected, the parent exits and the child is the daemon.

A bad signature or checksum leaves the old binary running. A second press while the gate is busy returns the phase already in progress. A finished failure, refusal, or up-to-date phase clears that gate in the same process, so a later press can try again. The header Update button shows that result and enables itself again, except while a restart or download is still in progress.

Release downloads use a 5 minute timeout so a 20 to 40 MB binary can finish on a slow link.

This Mac answers `Cannot` with "no release binary for macos aarch64" and does not download.

## Limits

An app older than this change cannot run `update_self`. The hub turns that refusal into "update by hand". That machine has to be updated once by someone at the keyboard.

The release workflow publishes the Linux musl binaries, `SHA256SUMS`, and `SHA256SUMS.minisig`. `latest.json` is assembled once from the desktop updater fragments. `SHA256SUMS` lists that file and both musl binaries, and the signature covers those exact bytes. The release stays a draft until that upload succeeds. This client does not change how a minisign signature is decoded.

## Release files

The node reads these names from `https://github.com/FelipeGazapina/pinned/releases/latest/download/`.

`latest.json` is a JSON object with a `version` string. The version is a semver number. One leading `v` is allowed.

`SHA256SUMS` is text. Each line is 64 hex digits, whitespace, then the file name. The GNU binary form, one asterisk before the name, is the same name. The signature is over the exact bytes of this file.

`SHA256SUMS.minisig` is the raw minisign signature text. `Signature::decode` already accepts that text. A base64 `.sig` is a different format and this client does not read it.

`SHA256SUMS` must list `latest.json` and the musl binary for the triple this node is running. The names are `pinned-server-x86_64-unknown-linux-musl` and `pinned-server-aarch64-unknown-linux-musl`. The node checks `latest.json` against that signed list before it trusts the version. A version that is not listed is not a release, and the node does not install.

The signature must be a current prehashed minisign signature. `allow_legacy` is false. A legacy signature, one that is not prehashed, is rejected. The public key is the updater key in `tauri.conf.json`.

Nothing in this change was released, tagged, or pointed at the production hub.
