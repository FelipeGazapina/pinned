# A job re-run must not publish a manifest ahead of its sums

PR 35. GitHub re-runs one job without the jobs it needs. On `84f8ec6`, `sign-node` uploaded `latest.json` and then signed. Re-running that job on a published tag skipped `draft`. If the signer failed, the public tag had a new manifest and the old `SHA256SUMS`. Linux nodes then fail with `the release checksum does not match`.

`scripts/release-asset.sh draft` runs before a desktop upload and before the node upload. `publish` in that script is the only step that clears the draft flag.

- `draft` sets the release to draft and exits unless `isDraft` is true.
- `upload` checks that `SHA256SUMS` precedes `latest.json` and that the sums list that file, then drafts, then uploads.
- `publish` downloads `latest.json`, `SHA256SUMS`, and `SHA256SUMS.minisig`. It clears the draft only when the manifest hash is a line in the sums, and it passes `--latest`.

The desktop `build` job calls `draft` immediately before `tauri-action`. `includeUpdaterJson` stays false, so that action does not write `latest.json`. `linux-node` uploads no release asset. `sign-node` signs on disk, then calls `upload` with the sums before the manifest. A signer failure uploads nothing, so a public release keeps its previous files.

Two shapes were weighed. Drafting at the start of `sign-node` would unpublish a good release when the signer fails before any upload. Drafting inside `upload`, after the signature exists, leaves that release public and consistent. `publish` still refuses a draft whose manifest hash is missing, so a crash between the sums and the manifest cannot be made public by re-running `publish` alone.

## Proof

`python3 scripts/test-release-rerun.py` replays the workflow's release commands against a fake `gh`. On `84f8ec6` the signer-failure re-run printed `upload latest.json while the release is public` and `public latest.json 2fcd131acd18f085d3d7b251a1eaf09c025c1ba4a6af984f254ee3a5786736a3 missing from SHA256SUMS`. The same check on `f0cebb3` printed `ok: 8 scenarios`.

`bash scripts/test-assemble-latest-json.sh scripts/fixtures/v0.16.0` printed `ok: v0.16.0 latest.json byte match` and `ok: updater fragments match v0.16.0 signature and url`. `actionlint` on `release.yml` and `linux.yml` exited 0.

## What this does not change

`crates/server/src/self_update.rs` is not edited in this round. The merged lib test writes its own bytes and does not read `target/debug/pinned-server`, so the Linux job does not build the binary before `cargo test --lib`. Hub deploy still does not wait for `publish`. `tauri-apps/tauri-action` stays on the `v0` tag. No tag was pushed, no GitHub release was created or edited, and the workflow was not run.
