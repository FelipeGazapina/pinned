# Linux node release

A Linux node updates from the public GitHub release. The release workflow builds the same musl binaries the hub image builds, signs `SHA256SUMS` with the Tauri updater key, and leaves the release as a draft until those files are uploaded.

## What gets published

A tag `v*` still starts the workflow. The release stays a draft until the Mac builds, the Windows build, and the Linux node job all succeed. The Linux job then uploads four files.

- `pinned-server-x86_64-unknown-linux-musl`
- `pinned-server-aarch64-unknown-linux-musl`
- `SHA256SUMS`
- `SHA256SUMS.minisig`

`latest.json` still comes from the Tauri updater action. The node reads its `version` field. A desktop update and a node update become public together.

`SHA256SUMS` has one line per binary. The line is 64 hex digits, two spaces, then the file name. The node splits each line on whitespace and looks up `pinned-server-<target>`.

## Signature

`tauri signer sign` writes `SHA256SUMS.sig` as base64 of a minisign signature. The node calls `Signature::decode` on the raw minisign text, the same text its tests already sign with the `minisign` crate. `scripts/sign-node-sums.sh` runs that signer and writes the decoded text to `SHA256SUMS.minisig`. The workflow uploads the decoded file and does not upload the `.sig` wrapper.

The public key is `plugins.updater.pubkey` in `tauri.conf.json`. The private key is the existing `TAURI_SIGNING_PRIVATE_KEY` secret, with an empty password, the same values the desktop bundles already use. There is no new secret.

The node rejects the base64 wrapper. A test generates a throwaway key with `npx tauri signer generate`, signs through the script, and calls `verify`. The decoded text is accepted. A changed checksum is rejected. The wrapper fails with `could not read the release signature`.

## Build

The Linux job runs `docker build --target build`. That stage is the hub `Dockerfile` stage that runs `cargo zigbuild --release -p pinned-server` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, then copies the binaries to `/downloads`.

## What this does not change

`hub-deploy.yml` and `railway-hub` are unchanged. The hub image still serves the first install at `GET /download/pinned-server-<target>`. A node that is already installed updates from GitHub, not from that route.

If the Linux job or a desktop build fails, the release stays a draft. `releases/latest` keeps the previous public release. A re-run uploads again and publishes when every job succeeds.

## Proof limit

Docker was not running here, so an old node inside a Linux container was not started, and `update_self` was not driven through a live hub. The library tests cover the signature, the checksum, the binary swap, and the restart plan.
