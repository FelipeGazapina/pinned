# Linux node release

A Linux node updates from the public GitHub release. The release workflow builds the same musl binaries the hub image builds, signs `SHA256SUMS` with the Tauri updater key, and leaves the release a draft until `publish`.

## What gets published

A tag `v*` starts the workflow. The `draft` job opens a draft release, or sets an existing release back to draft. The `build` job drafts the release again, then uploads the Mac and Windows bundles. Each leg writes `updater-fragment.json` and uploads it as the Actions artifact `updater-<target>`. It does not upload that fragment or `latest.json` to the release. The `linux-node` job builds the musl binaries and does not upload a release asset. The `sign-node` job runs only after both of those jobs succeed. It assembles `latest.json` from the fragments, checks the version against the tag, signs that file with the two binaries, then uploads `SHA256SUMS` before `latest.json`. That upload drafts the release first. The desktop job runs its steps in bash, including on Windows.

- `latest.json`
- `pinned-server-x86_64-unknown-linux-musl`
- `pinned-server-aarch64-unknown-linux-musl`
- `SHA256SUMS`
- `SHA256SUMS.minisig`

`sign-node` signs the bytes on disk, then uploads them. `publish` is the only job that clears the draft flag. It does that only when the hash of `latest.json` is a line in `SHA256SUMS` and no release asset is newer than `latest.json`. It passes `--latest` only when this tag is the same as the current latest or newer than it. Nothing after the signature writes `latest.json`.

`SHA256SUMS` has one line per file. The line is 64 hex digits, two spaces, then the file name. The files are `latest.json`, `pinned-server-aarch64-unknown-linux-musl`, and `pinned-server-x86_64-unknown-linux-musl`. The node splits each line on whitespace. One `*` before the name is the same name. The signature is over the exact bytes of `SHA256SUMS`.

## Signature

`scripts/sign-node-sums.sh` writes `SHA256SUMS` from the three files in the directory. A checksum file already there is replaced.

`tauri signer sign` writes `SHA256SUMS.sig` as base64 of a minisign signature. The script decodes that to `SHA256SUMS.minisig`. The node calls `Signature::decode` on that text. The workflow uploads the decoded file and does not upload the `.sig` wrapper.

The first two bytes of the decoded signature are `ED`. That is the prehashed algorithm. `Ed` is the legacy algorithm, and the script exits if it sees anything else. `allow_legacy` in the node is false.

The public key is `plugins.updater.pubkey` in `tauri.conf.json`. The private key is the existing `TAURI_SIGNING_PRIVATE_KEY` secret, with an empty password, the same values the desktop bundles already use. There is no new secret.

## Build

The Linux job runs `docker build --target build`. That stage is the hub `Dockerfile` stage that runs `cargo zigbuild --release -p pinned-server` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, then copies the binaries to `/downloads`. The sign job receives those binaries as the `linux-node-binaries` artifact.

## What this does not change

`hub-deploy.yml` and `railway-hub` are unchanged. The hub image still serves the first install at `GET /download/pinned-server-<target>`. A node that is already installed updates from GitHub, not from that route.

If the signer fails, `sign-node` has not uploaded, and the draft flag is unchanged. A tag that is already the public latest stays the latest. If a desktop leg or the signed upload runs, that job drafts the release before it writes. A full run that fails before `publish` leaves a new tag as a draft, so `releases/latest` stays on the previous public release. `publish` exits and leaves the draft when the `latest.json` hash is missing from `SHA256SUMS`, or when a release asset is newer than `latest.json`. An older tag does not pass `--latest`. Two runs of the same tag do not overlap.

## Proof limit

Docker was not running here, so the musl image was not built, and `update_self` was not driven through a live hub. The library test `a_signed_release_installs_a_newer_version_and_refuses_a_tampered_or_older_one` runs this script with a throwaway key and calls `NodeUpdate::prepare` over a local HTTP server.
