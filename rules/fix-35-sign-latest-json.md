# Sign latest.json into the node release

PR 35, branch `feat/release-linux-node`, is stacked on PR 33. After PR 33's fix round, a Linux node installs a release only when the signed `SHA256SUMS` lists `latest.json` and the musl binary for its triple, the signature is prehashed minisign text, and the signed version is strictly newer. This pass rebases the release job onto that branch and makes the job produce that set.

Nothing here was released, tagged, merged, or pointed at the production hub. `railway-hub` was not moved. The `hub-breaking-change` environment was not approved. No user machine, Railway sandbox, or installed app was touched. Docker and Colima were not started.

## What changed

`scripts/sign-node-sums.sh` hashes `latest.json`, `pinned-server-aarch64-unknown-linux-musl`, and `pinned-server-x86_64-unknown-linux-musl`, replacing any checksum file already in the directory. It signs that file with `tauri signer sign` and writes the decoded minisign text to `SHA256SUMS.minisig`. The script exits unless the signature algorithm bytes are `ED`.

The `sign-node` job runs after the desktop `build` job and the `linux-node` job. It downloads the `latest.json` the desktop jobs already uploaded, runs the script, and uploads the two binaries, `SHA256SUMS`, and `SHA256SUMS.minisig`. It does not upload `latest.json` again. `publish` only clears the draft flag, and only after `build` and `sign-node` succeed.

The node's verifier in `crates/server/src/self_update.rs` is the parent branch's. The new test calls `NodeUpdate::prepare` on files the script signed.

## Verification

Before the script hashed `latest.json`, the same test printed:

```
valid: Failed { to: "", reason: "SHA256SUMS has no latest.json" } bytes=[111, 108, 100, 45, 98, 105, 110, 97, 114, 121]
```

The assertion diff was `Failed { to: "", reason: "SHA256SUMS has no latest.json" }` against `Restarting { to: "9.9.9" }`. Those bytes are `old-binary`.

After the script hashed the three files, the test printed:

```
algorithm: ED
SHA256SUMS:
9b8be8991998fb0d143e98bdc43a9dd0b76301e1d449f8c9d8682dcf046f1c39  latest.json
762bec7611eacc0e5e4652c3bb17808ed8cbc5db3d6db9bfcc308c9bc86bce78  pinned-server-aarch64-unknown-linux-musl
1d4b8e023d50aca7abf43c0970296f3888c6c7b61a43d65b3184c627740ee1dd  pinned-server-x86_64-unknown-linux-musl

valid: Restarting { to: "9.9.9" } bytes=[120, 56, 54, 45, 98, 105, 110, 97, 114, 121]
tampered latest.json: Failed { to: "", reason: "the release checksum does not match" } bytes=[111, 108, 100, 45, 98, 105, 110, 97, 114, 121]
tampered binary: Failed { to: "9.9.9", reason: "the release checksum does not match" } bytes=[111, 108, 100, 45, 98, 105, 110, 97, 114, 121]
legacy: the release signature does not match
older: Failed { to: "0.0.1", reason: "the published release 0.0.1 is older than 0.16.0" } bytes=[111, 108, 100, 45, 98, 105, 110, 97, 114, 121]
```

The valid bytes are `x86-binary`. The other cases left `old-binary`. `cargo test -p pinned-server --lib -- --test-threads=4` passed 36 tests. `actionlint .github/workflows/release.yml` exited 0. `cargo run -p pinned-server --bin wire-contract -- check --base origin/main` exited 0 with no new breaks. The breaks it still lists are the `Connector.machine` change already shipped from 0.8.0 through 0.12.0.

A throwaway `tauri signer sign` wrote algorithm bytes `ED` (`4544`), which is the prehashed algorithm. The trusted comment was `timestamp:1793454866	file:SHA256SUMS`.

## Left open

The musl image was not built here. `update_self` was not driven through a live hub. The desktop jobs can still race while they merge `latest.json`. The sign job starts after every desktop job has finished, and nothing after the hash writes that file.
