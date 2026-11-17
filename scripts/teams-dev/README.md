# Teams dev stack

## Box only

BOX ONLY. Do not run this on a real computer (a laptop, a Mac mini, a server with a real pinned).
The fake box runner checks only the command line it is given: a path the command builds while it runs (a variable, a script file, an interpreter) slips past its "stay inside the stack dir" check.
The real isolation is the Upstash box the stack runs in, not this guard.

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
| hub (`--ui ./ui --signup open --dev`, serves `ui/` and the pinned-server download) | http://127.0.0.1:8811 |
| fake Upstash Box (`fake-box`) | http://127.0.0.1:8812 |
| fake Slack (`fake-slack`, log in `$TEAMS_DEV_DIR/fake-slack.log.jsonl`, `GET /sink` lists the calls it got) | http://127.0.0.1:8813 |
| fake GitHub (`pinned-server fake-github`, where the hub files reports) | http://127.0.0.1:8814 |
| Postgres, empty, user `pinned`, database `pinned` | 127.0.0.1:5611 |

Open http://127.0.0.1:8811 and sign in as `ana@example.test` (the team owner), `bruno@example.test`
or `carla@example.test`. Their generated passwords are in `$TEAMS_DEV_DIR/accounts.json` (mode 600),
which `urls` names. Add more seeding as a step in `STEPS` of `seed.mjs`; `dev.sh seed` runs it again.

`TEAMS_DEV_DIR` (default a mktemp dir, recorded in `.dev/teams-dev.dir`) holds every file: database,
logs, the hub key, each fake box's directory. `TEAMS_DEV_BIN=<pinned-server>` skips the cargo build;
`TEAMS_DEV_HUB_PORT`, `_BOX_PORT`, `_SLACK_PORT`, `_GITHUB_PORT` and `_PG_PORT` change the ports. The hub always
starts with `--dev` and `PINNED_DEV_TIME_SCALE=${TEAMS_DEV_TIME_SCALE:-60}` (a box's idle and
lifetime timers run 60 times faster), sends every Slack call to the fake Slack, and files every
report on the fake GitHub.

**Reports:** "report an issue" files on the fake GitHub: `dev.sh` points `PINNED_REPORTS_GITHUB_API`
at it and gives the hub the placeholder token `fake-teams-dev`, which only the fake takes. Under
`--dev` the hub refuses a reports API that is not on this machine, and has reporting off when none is
set, so a dev hub never files on the real GitHub.
Never set PINNED_REPORTS_GITHUB_TOKEN for a dev hub, box or test environment.

**Slack:** a team or account connects Slack with the token `xoxb-dev-<anything>`; the fake answers
it as the workspace "Pinned Dev". Any other token (say `xoxb-real-…`) gets `invalid_auth`, so a real
token typed by mistake goes nowhere.

The fake box keeps boxes under `$TEAMS_DEV_DIR/boxes/<id>/`: `/workspace/home` in a path, command or
file the hub sends is the box's `home/`, `/tmp` its `tmp/`. It runs the real bootstrap, so a box
downloads this checkout's pinned-server and its node signs in to the dev hub, with the web sweep's fake
`claude` and `gh` first on its PATH. `https://github.com/example/{alpha,beta}` clone from local
repositories in `$TEAMS_DEV_DIR/git`. Try it: `node scripts/teams-dev/smoke.mjs open` (connects
Upstash as ana with made-up keys and opens a box; `disconnect` ends it).

The stack never touches anything outside its own directory:

- `dev.sh` resolves the directory (symlinks followed, nothing created yet) and exits 2 when it is, or
  is inside, `~/.pinned`, `~/.config/pinned`, the desktop app's data folder, `/workspace/home`, or is
  your home folder or `/`. It also refuses to empty a folder that is neither empty nor an earlier
  stack's (a `.teams-dev-stack` file marks one). It does not rely on a running node as a guard.
- `fake-box` resolves `TEAMS_DEV_DIR` with realpath at start. `files/write` refuses (400) a target
  that is not inside the box's own directory, symlinks followed. `exec` refuses (400) an argument that
  names an absolute path outside the box (also inside an `sh -c` string), a `..` that leaves it, or a
  `/workspace/home` or `~/.pinned` that the rewrite did not turn into the box's own; only `/bin/`,
  `/usr/`, `/lib/`, `/lib64/`, `/etc/`, `/dev/null`, `/proc/self/`, node, git and the fake's own bin
  are allowed besides. This reads the command line; it is a guard against a stale path, not a sandbox.
  Deleting a box signals only the pids the fake spawned for it and what those started, each checked
  in `/proc/<pid>/environ`; no pid is read from a file.

Two more refusals keep the stack away from anything real:

- `hub-client.mjs` (seed, smoke and the end-to-end steps) takes the hub from `TEAMS_DEV_HUB`, and
  exits 1 for any address whose host is not `127.0.0.1`, `::1` or `localhost`, whose scheme is not
  `ws` or `http` (a `wss://` remote included), or that carries a user name or password. The message
  names the variable, never its value.
- `dev.sh up` runs `check-bin.sh` on the pinned-server (`TEAMS_DEV_BIN` or the cargo build) before it
  empties, creates or starts anything, and exits 2 unless it is a debug build. The binary is asked
  with an empty environment (`hub --dev` is refused before it listens); the dev check in
  `crates/server/src/dev.rs` refuses a release build first, so a release build never gets to send Box
  calls to the real Upstash.

Tests: `node --test scripts/teams-dev/fake-box.test.mjs scripts/teams-dev/fake-slack.test.mjs scripts/teams-dev/hub-client.test.mjs`,
`sh scripts/teams-dev/dev-guard.test.sh`, `sh scripts/teams-dev/check-bin.test.sh`,
`cargo test -p pinned-server --lib dev::tests`, and `TEAMS_DEV_BIN=<pinned-server> sh scripts/teams-dev/e2e-empty.sh`
(up, status, sign-in, a box opened and ended, Slack through the hub, down, reset, in a fresh dir).

Safety rules: only `box_dev-…` keys work, and `UPSTASH_BOX_BASE_URL` always points at the fake, so no
real Upstash, Slack or production hub is reached; accounts are `@example.test`; every child runs with
its own HOME, TMPDIR and CLAUDE_CONFIG_DIR inside the dev dir and without `ANTHROPIC_*`,
`CLAUDE_CODE_OAUTH_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`, `PINNED_OPENCODE_GO_KEY`,
`UPSTASH_BOX_API_KEY` or a `PINNED_REPORTS_*` variable of the developer's; passwords go only to `accounts.json`. Never point the stack at a real hub.
