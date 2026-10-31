# Windows release draft and publish gates

PR 35. The desktop `build` job includes `windows-latest` and had no `shell: bash` on the draft step. GitHub runs that step in pwsh, so `$GITHUB_REF_NAME` is empty, `scripts/release-asset.sh` exits 2, and `sign-node` never runs.

The job now sets `defaults.run.shell` to bash, and the draft step sets `shell: bash`. The updater fragment step already set `shell: bash`. `scripts/test-release-rerun.py` reads each step's effective shell and fails when a bash-syntax script would run under pwsh.

`publish` marks this tag latest only when it is not older than the current latest. `rules/fix-35-publish-content-check.md` replaces its upload-time check with a check of the signed files. The replay's fake `gh` deletes assets on `--clobber` before it writes them and records a violation when the release is public. `tauri-action` is pinned to `84b9d35b5fc46c1e45415bdb6144030364f7ebc5`, the commit that still reads `includeUpdaterJson`.

The signer test expects `UpToDate` for release `0.0.1` and still refuses a tampered release. Production `prepare` was not edited.

## Proof

On `09fbcaf`, with workflow blob `448dcb2c2daf2bc5265d5da9b47ca8215b45dac7`, `python3 scripts/test-release-rerun.py` printed `FAIL shell: build step 6 runs bash syntax under pwsh: bash scripts/release-asset.sh draft "$GITHUB_REF_NAME"` and `ok: 8 scenarios`, then exited 1.

After the shell change, the same command printed `ok: shells` and `ok: 8 scenarios`. On the later head it printed `ok: shells`, `ok: public clobber`, and `ok: 11 scenarios`, including `publish an older tag does not take latest`, `publish a newer tag takes latest`, and `build re-run then publish without sign-node`.

`bash scripts/test-assemble-latest-json.sh scripts/fixtures/v0.16.0` printed `ok: v0.16.0 latest.json byte match` and `ok: updater fragments match v0.16.0 signature and url`.

`actionlint .github/workflows/release.yml .github/workflows/linux.yml` exited 0.

Before the assertion change, `a_signed_release_installs_a_newer_version_and_refuses_a_tampered_or_older_one` panicked at `crates/server/src/self_update.rs:611` with `left: UpToDate { version: "0.16.0" }` and `right: Failed { to: "0.0.1", reason: "the published release 0.0.1 is older than 0.16.0" }`. After the change, the same test passed. A clean `cargo test -p pinned-server --lib -- --test-threads=4` printed `test result: ok. 38 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.09s`.

No tag was pushed. No GitHub release was created or edited. The release workflow was not run. Docker was not started. The production hub was not touched.
