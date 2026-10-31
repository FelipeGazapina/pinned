# Linux node release

A Linux node updates from the public GitHub release. The release workflow builds the same musl binaries the hub image builds, signs `SHA256SUMS` with the Tauri updater key, and leaves the release a draft until `publish`. The desktop app and the node both read `releases/latest`, so the release that GitHub marks latest is the one every machine installs.

## What a tag runs

A tag `v*` starts `.github/workflows/release.yml`. The workflow sets `GH_REPO` to `github.repository` for every job. `gh` reads the repository from `GH_REPO` or from the git remote of the current directory, and it ignores `GITHUB_REPOSITORY`. The `draft` job has no checkout, so without `GH_REPO` its `gh release view` fails with `not a git repository` and every later job is skipped.

1. `draft` opens a draft release, or sets an existing release back to draft.
2. `build` drafts the release again, then `tauri-action` uploads the Mac and Windows bundles. `includeUpdaterJson` is false, so the action does not write `latest.json`. Each leg writes `updater-fragment.json` and uploads it as the Actions artifact `updater-<target>`. The job runs its steps in bash, including on Windows.
3. `linux-node` builds the musl binaries and uploads them as the artifact `linux-node-binaries`. It uploads no release asset.
4. `sign-node` runs after `build` and `linux-node` succeed. It assembles `latest.json` from the fragments, checks the version against the tag, signs that file with the two binaries, then uploads the five files below in one `gh release upload --clobber`. That upload drafts the release first.
5. `publish` checks the signed files and clears the draft flag. It is the only step that makes a release public.

Both `upload-artifact` steps set `overwrite: true`. The action fails when an artifact with the same name exists, and a re-run can find the artifact from an earlier attempt.

The five node files are:

- `latest.json`
- `pinned-server-x86_64-unknown-linux-musl`
- `pinned-server-aarch64-unknown-linux-musl`
- `SHA256SUMS`
- `SHA256SUMS.minisig`

`sign-node` signs the bytes on disk, then uploads them. Nothing after the signature writes `latest.json`. `scripts/release-asset.sh upload` refuses `latest.json` when `SHA256SUMS` is not in the same upload or does not list that file's hash. It also refuses when `gh release edit --draft=true` leaves the release public.

## What publish checks

`publish` downloads every asset. It exits and names the file unless all of these hold:

- `SHA256SUMS.minisig` verifies `SHA256SUMS` with `plugins.updater.pubkey` from `tauri.conf.json`.
- Each file that `SHA256SUMS` lists is on the release with that sha256.
- `SHA256SUMS` lists `latest.json` and both musl binaries.
- Each desktop bundle that `latest.json` names verifies against its `signature`.

`scripts/minisign-verify.mjs` checks the signatures with `node:crypto`. It accepts only the prehashed `ED` algorithm, the same as the node. It refuses base64 that does not encode back to the same text, because the `minisign-verify` crate in the node rejects stray characters that `Buffer.from` skips.

The trust root is `tauri.conf.json` and nothing else. No environment variable replaces the key.

## Which release is latest

A tag with a suffix, such as `v0.17.0-rc.1`, publishes with `--prerelease --latest=false`. GitHub never marks a prerelease latest, so no desktop and no node installs a release candidate on its own. To try a release candidate, download it from its release page.

Hub deploy skips a suffixed tag too. It deploys the newest tag on `main` of the form `vMAJOR.MINOR.PATCH`. Git's version sort ranks `v0.17.0-rc.1` above `v0.17.0`, so without that filter a release candidate on `main` would go live on the hub, and its final would then read as already live.

A tag without a suffix takes latest when it is not older than the current latest, in semver order. `v0.17.0` is newer than `v0.17.0-rc.2`, and `v0.17.0-rc.10` is newer than `v0.17.0-rc.9`. An older tag gets `--latest=false`, because GitHub marks a newly published release latest when the flag is absent.

`publish` reads the current latest with `gh release view`. It treats only the `release not found` error as "no latest release yet". Any other error, such as an HTTP 503 or a token problem, stops `publish` and leaves the draft.

## Cutting a release

`scripts/release.sh <version>` refuses to start unless all of these hold:

- The version is `MAJOR.MINOR.PATCH`, with an optional `-suffix`.
- The tree is clean, and the current branch is `main`.
- `HEAD` equals `origin/main` after a fetch.

It pushes `main` and the tag in one `git push --atomic`, so origin gets both or neither. If another session moves `origin/main` while the release builds, the push is rejected. The script then deletes the local tag and resets `main` to the commit it started from.

A version without a suffix writes `crates/core/wire-contract/<version>.json`. A suffixed version writes no snapshot, because `AGENTS.md` makes every later hub honor every snapshot and the hub never runs a release candidate. A release candidate that someone installs by hand gets no compatibility promise from later hubs.

## Checksums and signature

`SHA256SUMS` has one line per file. The line is 64 hex digits, two spaces, then the file name. The files are `latest.json`, `pinned-server-aarch64-unknown-linux-musl`, and `pinned-server-x86_64-unknown-linux-musl`. The node splits each line on whitespace. One `*` before the name is the same name. The signature is over the exact bytes of `SHA256SUMS`.

`scripts/sign-node-sums.sh` writes `SHA256SUMS` from the three files in the directory and replaces a checksum file already there. `tauri signer sign` writes `SHA256SUMS.sig` as base64 of a minisign signature. The script decodes that to `SHA256SUMS.minisig` and exits unless the first two bytes are `ED`. The node calls `Signature::decode` on that text with `allow_legacy` false. The workflow does not upload the `.sig` wrapper.

The private key is the existing `TAURI_SIGNING_PRIVATE_KEY` secret, with an empty password, the same values the desktop bundles use. There is no new secret.

## Re-runs and partial failures

GitHub re-runs one job without the jobs it needs, so each uploading job drafts the release itself before it writes. The concurrency group `release-<tag>` has `cancel-in-progress` false, so two runs of one tag do not overlap.

- If the signer fails, `sign-node` has uploaded nothing. A public release keeps its files and stays latest.
- A full run that fails before `publish` leaves a new tag as a draft, so `releases/latest` stays on the previous release.
- `gh release upload --clobber` deletes each asset right before it uploads that file, in five parallel workers. A cancelled upload can leave any mix of new, old, and missing files. `publish` checks content, not order, so it refuses every such mix.
- A `build` re-run after `sign-node` replaces a bundle that `latest.json` signed. `publish` refuses until `sign-node` runs again.

`publish` also has the job-level group `release-publish`, so the `publish` jobs of two tags run one at a time. The second reads the latest release after the first sets it, and an older tag cannot end up latest. A group keeps one pending job. If three tags reach `publish` together, GitHub cancels the pending one when the third arrives, and that tag stays a draft until someone re-runs its `publish`. The `queue: max` option keeps every pending job, but actionlint 1.7.12 rejects the key.

## Replay

`python3 scripts/test-release-rerun.py` runs in the Linux CI job. It reads `release.yml` with `scripts/workflow.py`, a YAML reader that fails on syntax it does not know. It runs every `run` step of every job against the real scripts. A job fails at any `uses` step that is not one of `actions/checkout`, `actions/setup-node`, `dtolnay/rust-toolchain`, `swatinem/rust-cache`, `actions/upload-artifact`, `actions/download-artifact`, or `tauri-apps/tauri-action`. A step that writes the release some other way therefore fails the replay. It models these parts of the runner:

- A full run tries every job order that `needs` allows. A job whose need failed is skipped, and the others still run. A scenario that names jobs fails if it runs a job before one it needs.
- `build` runs all three matrix legs. Each job and leg gets its own directory, and files move between jobs only through `upload-artifact` and `download-artifact`.
- `tauri-action` writes a signed bundle for the `--target` in `args`, uploads it under the name GitHub shows, and sets `artifactPaths` and `appVersion`.
- Fakes replace `gh`, `npx`, `npm`, `docker`, `sudo`, `curl`, and `wget`. `docker cp` copies two fixed binaries, `sudo` runs nothing, and `curl` and `wget` fail.
- Each step runs under the shell GitHub picks, `bash -e` by default and `bash -eo pipefail` for `shell: bash`. Each step gets the workflow, job, and step `env`. A job without `actions/checkout` has no `.git` and no `scripts/`.
- The fake `gh` refuses without `GH_TOKEN` or `GITHUB_TOKEN`, and refuses without `GH_REPO` or a `.git`, the way `gh` 2.102.0 does.
- `--clobber` deletes and uploads one file at a time. `REPLAY_UPLOAD_CANCEL` maps a file name to `new` or `missing` and cancels the upload. Files it does not name keep their old bytes.
- `upload-artifact` fails when the artifact exists from an earlier attempt and `overwrite` is not true.
- `REPLAY_LATEST_ERROR` fails the read of the latest release. `REPLAY_DRAFT_IGNORED` makes `gh release edit --draft=true` leave the release public.
- The fake `gh` records a violation when an edit asks for `--latest=true` on a prerelease. GitHub ignores that request, so only the fake can see it.
- Two tags publish at once. The replay reads the `concurrency` groups of the workflow and of `publish`. If the groups let the two runs overlap, both read the latest release before either edits it.
- A job with a checkout gets a throwaway `tauri.conf.json` that holds the replay key.

It also checks `version_is_newer` against a table of tags, and `scripts/minisign-verify.mjs` against the `minisign-verify` 0.3.0 test vectors.

`python3 scripts/test-release-tags.py` also runs in the Linux CI job. It runs `scripts/release.sh` against a scratch bare remote with a fake `cargo`, and the `pick` step of `hub-deploy.yml` against scratch tags.

## Build

The Linux job runs `docker build --target build`. That stage is the hub `Dockerfile` stage that runs `cargo zigbuild --release -p pinned-server` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, then copies the binaries to `/downloads`.

## What this does not change

Hub deploy does not wait for `publish`. The hub image still serves the first install at `GET /download/pinned-server-<target>`. A node that is already installed updates from GitHub, not from that route.

## Proof limit

The musl image was not built here, and `update_self` was not driven through a live hub. The library test `a_signed_release_installs_a_newer_version_and_refuses_a_tampered_or_older_one` runs `scripts/sign-node-sums.sh` with a throwaway key and calls `NodeUpdate::prepare` over a local HTTP server. No release workflow has run on this branch.
