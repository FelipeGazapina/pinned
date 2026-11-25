# Rules to follow

1. never create docs files or push docs files, after the usage remove it, we don't want to have documents to context drift the agent
2. don't create rules, the code should talk by him self, any agent rule should be at AGENTS.md file.
3. never left don't used or trash on the codebase, we aren't supposed to have a bunch of not utilized code saved, this distils the agent

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

# Hub integration tests

`crates/server/tests/hub.rs` needs Postgres at `postgres://pinned@127.0.0.1:5499`, where the role `pinned` owns a database `pinned`. Set `PINNED_TEST_POSTGRES` to use another server. The URL must not end in a database name: each test creates its own. Run the tests with a throwaway `HOME`, the login variables unset (`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CODEX_HOME`, `GH_TOKEN`, `GITHUB_TOKEN`, `PINNED_SOCK`, `PINNED_TAB`, `PINNED_TASK`, `XDG_CONFIG_HOME`) and a short `TMPDIR` such as `/tmp/x`, since Unix socket paths are short.

This command runs the test that an agent's changes to the global orchestrator settings wait for the owner's approval, and that another account reaches none of them:

```
cargo test --locked -p pinned-server --test hub -- --exact an_agent_manages_its_accounts_workflows_projects_sessions_settings_and_usage_and_another_account_reaches_none_of_it
```

The `agent-permissions` job in `linux.yml` runs that one test on pull requests, against a Postgres service on port 5499. The rest of `hub.rs` runs in no CI job.
