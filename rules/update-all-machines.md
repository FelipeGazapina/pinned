# Update every machine

A signed-in person, on any of their machines or on the web client, presses one control and every machine of theirs that is behind the newest release installs it and comes back online. That includes the machine they pressed it on. The web client has nothing to install. It stays connected and shows each row.

## What the person sees

The machines page has "Update every machine". A row that can be asked also has "Update". Each row shows one status. Up to date names the version. Updating and restarting name the version they are moving to. A failure names the reason. A machine that cannot install a release names why, including "update by hand" when the installed app does not know the command.

The page asks the other online machines first, then the machine the page is running on. A second press while that is running does nothing. An Upstash box is left out. The hub also refuses one that is online, because the hub replaces that box itself.

The header Update button still installs only the desktop app it is on.

## Data shape

The command is `Call::UpdateSelf` with no fields. It rides the existing client call. The hub only forwards it to a machine in that same account.

`UpdateStatus` is one enum. `UpToDate { version }`, `Updating { to }`, `Restarting { to }`, `Failed { to, reason }`, `Cannot { reason }`. The machine is the only writer. The hub keeps the last phase in memory and overlays it on `MachineView.update`. A missing status is not up to date.

`MachineView.version` is the string from the last machine auth. It is a `machines.version` column, added with `alter table ... add column if not exists` so an existing database grows it. An in-progress phase is not stored in Postgres. A hub restart forgets it.

When a machine connects with no snapshot, an idle connect after updating or restarting becomes up to date if the version matches, and failed with "came back on {version}" if it does not.

## Installers

The desktop app runs the same updater check and download as the Update button, then relaunches. A node on Linux x86_64 or aarch64 downloads the musl binary, checks the minisign signature of `SHA256SUMS` and the file's checksum, keeps the previous binary beside it, swaps the executable, and re-execs with the same arguments and working directory. A bad signature or checksum deletes the temp file and leaves the old binary running. A second press while the gate is busy returns the phase already in progress.

This Mac answers `Cannot` with "no release binary for macos aarch64" and does not download.

## Limits

An app older than this change cannot run `update_self`. The hub turns that refusal into "update by hand". That machine has to be updated once by someone at the keyboard.

The GitHub release workflow does not yet publish the Linux musl binaries or `SHA256SUMS`. Until that ships, a Linux node that tries the public release fails the download and keeps the binary it is running.

Nothing in this change was released, tagged, or pointed at the production hub.
