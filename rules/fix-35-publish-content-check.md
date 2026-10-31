# Publish checks the signed files, not upload times

PR 35. `publish` refused a draft when any asset had an `updatedAt` later than the one on `latest.json`. `sign-node` uploads `latest.json`, `SHA256SUMS`, `SHA256SUMS.minisig`, and both musl binaries in one `gh release upload`. `gh` runs five upload workers, and `updatedAt` is the time a file finishes, so the binaries finish after the smaller `latest.json`. A normal release then stopped at `publish`. Upload times also cannot show that the signed sums cover the files on the release.

`publish` now downloads every asset and checks what the updaters check. `rules/release-linux-node.md` lists the four checks. A refusal names the file. `scripts/minisign-verify.mjs` verifies a prehashed minisign signature with `node:crypto`, and the `publish` job sets up Node 22 for it.

GitHub defaults `make_latest` to true for a newly published release, and `gh release edit` sends `make_latest` only when `--latest` is passed. `publish` now passes `--latest=false` when the current latest is newer. A tag whose version equals the current latest takes latest, so a final tag published after its release candidate does.

`upload` no longer checks that `SHA256SUMS` comes before `latest.json` in argv, because `gh` gives no landing order.

GitHub release assets carry a `digest` field, and `gh release view --json assets` returns it. On v0.16.0 it matches the sha256 of the committed fixtures, for example `sha256:96d47097554b141ac93b3763687cef7701be6a15682883cd95cd0b91ed99fcb0` for `latest.json`. `publish` does not read it. The desktop bundles must be downloaded for their signature check anyway, so one download path covers every file on any `gh` version.

## Replay

The fake `gh` in `scripts/test-release-rerun.py` follows the `gh` 2.102.0 source:

- It lands the files of one upload smallest first, each with its own `updatedAt`.
- With `--clobber` it deletes every old asset before a new one lands. `REPLAY_UPLOAD_LANDS` cancels an upload after the named files land.
- `edit` marks a release latest when it publishes a draft without `--latest`.
- `download` fails only when no asset matches a pattern.

The replay signs with a throwaway key through a fake `npx`. It runs the real `scripts/assemble-latest-json.sh` and `scripts/sign-node-sums.sh`. It also checks `scripts/minisign-verify.mjs` against the `minisign-verify` 0.3.0 test vectors.

## Proof

On `9bf137f`, which still has the `publish` from `5bb40ae`, `python3 scripts/test-release-rerun.py` failed 7 of 16 scenarios. `full run, then the same run again` printed `sign-node did not run after the last build: pinned-server-aarch64-unknown-linux-musl, pinned-server-x86_64-unknown-linux-musl`. `publish an older tag does not take latest` printed `latest=True, want False`. Four drafts went public: one after a cancelled upload, one with a stale signature, one with a changed binary, and one with sums that omit a binary.

On `5105542` the same command printed `ok: 21 scenarios`. Undoing each new check in `publish`, one at a time, made the scenario for that check fail.

With `plugins.updater.pubkey`, `scripts/minisign-verify.mjs` accepted the v0.16.0 signatures of `pinned_aarch64.app.tar.gz` and `pinned_0.16.0_x64-setup.exe`. It refused the wrong bundle, a truncated bundle, and an edited trusted comment.

No tag was pushed. No GitHub release was created or edited. The release workflow was not run.
