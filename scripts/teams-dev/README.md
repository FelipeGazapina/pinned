# Teams dev stack

One command for a local pinned hub with empty data, three invented accounts and a fake Upstash Box
runner, so Teams flows can be tried end to end. Nothing leaves the machine.

```
scripts/teams-dev/dev.sh up       # builds pinned-server, starts everything, seeds the accounts
scripts/teams-dev/dev.sh status   # up/down per part (exit 1 if any is down)
scripts/teams-dev/dev.sh urls     # the hub URL and the users' emails
scripts/teams-dev/dev.sh reset    # down, then a fresh up: empty database, new accounts
scripts/teams-dev/dev.sh down     # stops only the processes it started
```

| What | Where |
| --- | --- |
| hub (`--ui ./ui --signup open`, serves `ui/` and the pinned-server download) | http://127.0.0.1:8811 |
| fake Upstash Box (`fake-box`) | http://127.0.0.1:8812 |
| fake Slack, only when `scripts/teams-dev/fake-slack` exists | http://127.0.0.1:8813 |
| Postgres, empty, user `pinned`, database `pinned` | 127.0.0.1:5611 |

Open http://127.0.0.1:8811 and sign in as `ana@example.test` (the team owner), `bruno@example.test`
or `carla@example.test`. Their generated passwords are in `$TEAMS_DEV_DIR/accounts.json` (mode 600),
which `urls` names. Add more seeding as a step in `STEPS` of `seed.mjs`; `dev.sh seed` runs it again.

`TEAMS_DEV_DIR` (default a mktemp dir, recorded in `.dev/teams-dev.dir`) holds every file: database,
logs, the hub key, each fake box's directory. `TEAMS_DEV_BIN=<pinned-server>` skips the cargo build;
`TEAMS_DEV_HUB_PORT`, `_BOX_PORT`, `_SLACK_PORT` and `_PG_PORT` change the ports. When the hub has a
`--dev` flag, `up` passes it with `PINNED_DEV_TIME_SCALE=${TEAMS_DEV_TIME_SCALE:-60}`.

The fake box keeps boxes under `$TEAMS_DEV_DIR/boxes/<id>/`: `/workspace/home` in a path, command or
file the hub sends is the box's `home/`, `/tmp` its `tmp/`. It runs the real bootstrap, so a box
downloads this checkout's pinned-server and its node signs in to the dev hub, with the web sweep's fake
`claude` and `gh` first on its PATH. `https://github.com/example/{alpha,beta}` clone from local
repositories in `$TEAMS_DEV_DIR/git`. Try it: `node scripts/teams-dev/smoke.mjs open` (connects
Upstash as ana with made-up keys and opens a box; `disconnect` ends it).

Safety rules: only `box_dev-…` keys work, and `UPSTASH_BOX_BASE_URL` always points at the fake, so no
real Upstash, Slack or production hub is reached; accounts are `@example.test`; every child runs with
its own HOME, TMPDIR and CLAUDE_CONFIG_DIR inside the dev dir and without `ANTHROPIC_*`,
`CLAUDE_CODE_OAUTH_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`, `PINNED_OPENCODE_GO_KEY` or
`UPSTASH_BOX_API_KEY`; passwords go only to `accounts.json`. Never point the stack at a real hub.
