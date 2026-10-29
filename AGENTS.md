# Hub wire compatibility

A hub release must keep working for every app version that has a snapshot in `crates/core/wire-contract`.

`crates/core/wire-contract/current.json` is the contract of this tree. Each root stores what that version sends and what it accepts. After a wire change, write it back with `snapshot current`.

```
cargo run -p pinned-server --bin wire-contract -- snapshot current
```

Add an enum variant, or add a field with `#[serde(default)]`. Leave every field and variant that an older app sends or reads under the same name. Leave every HTTP route that an older snapshot lists.

Run this before pushing. It exits 1 when a snapshot would break. It exits 2 when `current.json` does not match the code.

```
cargo run -p pinned-server --bin wire-contract -- check
```

The release gate asks about new breaks only. `check --base <git-rev>` lists breaks this change adds over that revision under New breaks, and lists the rest under Already shipped. It exits 1 only when a new break exists. A pull request uses its base commit. A push to `main` uses the commit before the push. A hub deploy passes the commit `railway-hub` already points at.

The hub deploys itself. Every push to `main` or version tag runs `hub-deploy.yml`, which moves `railway-hub` to the newest version tag on `main` after the compatibility gate. Never move `railway-hub` by hand.

An agent must not approve the `hub-breaking-change` environment. An agent must not delete or edit a snapshot to make the check pass. Only the owner decides that a break ships.
