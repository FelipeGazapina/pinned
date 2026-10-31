# Release re-run stays a draft until the sums match

PR 35. A second run of a published tag left the release public. `tauri-action@v0` then replaced `latest.json` with a new `pub_date` and did not set the draft flag. If the musl build failed after that, nothing re-signed. Every Linux node failed with `the release checksum does not match`.

The `draft` job now runs `gh release edit --draft=true` when the release exists, and `gh release create --draft` when it does not. `build` and `linux-node` need that job, so a full run sets the draft flag before any upload. A re-run of one job does not re-run `draft`. The uploading job drafts itself. The workflow concurrency group is `release-<tag>` with `cancel-in-progress` false, so two runs of one tag do not overlap.

`tauri-action@v0` calls the switch `includeUpdaterJson`. It is false on the matrix. Each leg writes `updater-fragment.json` for its own platforms. `scripts/assemble-latest-json.sh` merges those files in one job. It requires `darwin-aarch64`, `darwin-aarch64-app`, `darwin-x86_64`, `darwin-x86_64-app`, `windows-x86_64`, and `windows-x86_64-nsis`. Those are the keys on the v0.16.0 release. Two fragments that set the same key to different bytes fail the job. The script strips one leading `v` from the tag and rejects a `.version` that is not the rest. `sign-node` runs `scripts/sign-node-sums.sh` on that file and the two musl binaries, then uploads `SHA256SUMS` before `latest.json`. `publish` needs `build`, `linux-node`, and `sign-node`, and it is the step that clears the draft flag.

A full run that fails before `publish` leaves the tag a draft. A `sign-node` re-run that fails in the signer uploads nothing, so a public release stays public. `releases/latest` stays on the previous public release until a later run publishes.

## Proof

`bash scripts/test-assemble-latest-json.sh` on fixture legs and on the downloaded v0.16.0 files printed `ok: v0.16.0 latest.json byte match` and `ok: updater fragments match v0.16.0 signature and url`. A merge without the Windows fragment printed `latest.json is missing windows-x86_64, windows-x86_64-nsis`. Tag `v0.17.0` against version `0.16.0` printed `latest.json version 0.16.0 does not match tag v0.17.0`. A second assemble of the same fragments matched the first byte for byte.

`scripts/sign-node-sums.sh` with a throwaway key replaced a stale checksum file and wrote:

```
9b8be8991998fb0d143e98bdc43a9dd0b76301e1d449f8c9d8682dcf046f1c39  latest.json
762bec7611eacc0e5e4652c3bb17808ed8cbc5db3d6db9bfcc308c9bc86bce78  pinned-server-aarch64-unknown-linux-musl
1d4b8e023d50aca7abf43c0970296f3888c6c7b61a43d65b3184c627740ee1dd  pinned-server-x86_64-unknown-linux-musl
```

The first two decoded signature bytes were `4544`, which is `ED`.

`PATH=~/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH CARGO_BUILD_JOBS=4 cargo test -p pinned-server --lib -- --test-threads=4` printed `test self_update::tests::a_signed_release_installs_a_newer_version_and_refuses_a_tampered_or_older_one ... ok` and `test result: ok. 36 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.80s`.

`actionlint .github/workflows/release.yml .github/workflows/linux.yml` exited 0.

## What this does not change

`crates/server/src/self_update.rs` gained the signer test `a_signed_release_installs_a_newer_version_and_refuses_a_tampered_or_older_one`. The job-rerun round does not edit that file. Hub deploy still does not wait for `publish`. No tag was pushed, no GitHub release was edited, and the workflow was not run. Docker was not started. The musl image was not built.
