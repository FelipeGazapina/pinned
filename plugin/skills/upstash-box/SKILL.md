---
name: upstash-box
description: Run code, a dev server, a browser test, or a whole Claude Code session inside an Upstash Box cloud sandbox with the `box` CLI, on the user's Claude subscription instead of API pricing. Use when the user mentions Upstash Box, the `box` CLI, a cloud sandbox, or wants work done off their own computer.
---

# Upstash Box

An Upstash Box is a Linux container in the cloud with Node, Python, git, Claude Code and Codex already installed. `/workspace/home` keeps its files across pauses and snapshots. A box pauses itself when idle, and the next command resumes it.

## Before you start

- The `box` CLI comes from `npm i -g @upstash/box-cli`. Check with `box --version`.
- The CLI reads the Box API key from `UPSTASH_BOX_API_KEY`. A Box key starts with `box_` and comes from the Box tab of console.upstash.com. A 36-character UUID is the account's Management API key, and Box rejects it with `Invalid box API key`. If the variable is not set, ask the user for the key. Never write it to a file in the repository.
- Run `box` from a temporary directory or pass `--no-use`. Otherwise `box create` writes a `.box` file into the current directory.

## Never use the managed agent

`box run`, and `box create` with `--agent-harness`, `--agent-model` or `--agent-api-key`, bill the model at API prices. To run Claude in a box, use Claude Code itself:

```sh
box --box <id> exec -C <repo> -- claude -p "<task>" --dangerously-skip-permissions --output-format stream-json
```

Claude in the box needs a login. Either the user signs in once over SSH (`ssh <id>@us-east-1.box.upstash.com`, password: the Box key, then `claude auth login` with the Claude account option), or the box gets `CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token`. Check with `claude auth status`. It must show `authMethod: "claude.ai"` after a login, or `"oauth_token"` with the token, and never an API key. Claude Code removes `CLAUDE_CODE_OAUTH_TOKEN` from its own Bash tool's environment, so a session that can't see the variable is still signed in. Skipping permissions is acceptable only inside a box.

## Common commands

```sh
ID=$(box create --no-repl --no-use --runtime node --browser --name <name>)
box --box $ID git clone https://github.com/<owner>/<repo>
box --box $ID exec -C <repo> -- npm test
box --box $ID exec -- '( npm run dev > /tmp/dev.log 2>&1 & )'
box --box $ID public-url 3000
box --box $ID files write <path> - < local-file
box --box $ID snapshot --name <name>
box delete $ID --yes
```

- A single argument after `--` runs as a shell expression, which is how you pipe, redirect or background something.
- `box exec` passes the remote command's exit code through. A failure of the CLI itself exits with 125.
- `box files write` drops the executable bit, so run `chmod +x` after it.
- `pkill -f "<text>"` inside `box exec` also matches its own `sh -c` line. Use a bracket pattern, such as `pkill -f "[n]pm run dev"`.
- Deleting a box cannot be undone. Delete only boxes you created, or ones the user names.

## Browser tests

`--browser` installs Chromium at `/usr/bin/chromium` and can only be chosen when the box is created. Launch that Chromium yourself, headless:

```sh
playwright-mcp --browser chromium --executable-path /usr/bin/chromium --headless --no-sandbox --isolated
```

The same flags work for a Playwright script (`executablePath`, `headless: true`, `args: ["--no-sandbox"]`). Do not attach to the box's own browser over CDP on `127.0.0.1:9222`: after Chromium restarts, the handshake hangs.

## Interactive `claude` in a box

`claude -p` starts straight away. An interactive `claude` stops at the theme picker and then the folder-trust dialog. With `--dangerously-skip-permissions` or the bypass permission mode, it also stops at a bypass warning. Before you start one:

- In `~/.claude.json`, set `hasCompletedOnboarding` to `true`, set `theme` to `"dark"`, and set `hasTrustDialogAccepted` to `true` under `projects["/workspace/home"]` and `projects["/home/boxuser"]`.
- Trusting a parent folder covers plain folders inside it, but not a git repository. Add a `projects["<repo folder>"]` entry with `hasTrustDialogAccepted: true` for each clone.
- For the bypass warning, set `skipDangerousModePermissionPrompt` to `true` in `~/.claude/settings.json`.
