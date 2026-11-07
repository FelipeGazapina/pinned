# External agents over MCP

A hub serves an MCP server at `<hub>/mcp`. An external agent, such as a hosted chat agent, connects to it with OAuth and the user's approval. It can then start coding sessions in the projects the user picked, message them, and read them. It can also talk to a project's orchestrator. Everything goes through the same machine calls the app uses: `Call::Run`, `Call::OpenOrchestrator`, `Call::StartClaude`, `Call::Prompt`, `Call::Thread` and `Call::Tabs`. So the orchestration and delegation already in Pinned are what the agent drives.

## Connecting an agent

1. In Pinned, open Settings → Connectors → **External agents (MCP)** and copy the endpoint, `<hub>/mcp`.
2. In the agent, add a custom MCP server (remote, Streamable HTTP) with that URL. The agent discovers OAuth on its own.
3. The agent opens the hub in your browser. Sign in if asked. Pinned shows a consent card on the Connectors page with the agent's self-chosen name, the exact address it returns to, what it may do (`sessions`, `orchestrator`), and the projects it may work in. No project is ticked for you.
4. Approve or Deny. Approving sends the browser back to the agent with a one-use code.
5. Revoke the agent on the same card at any time. Its tokens stop working at once.

A hosted agent must reach the hub over public HTTPS. A hub on `http://127.0.0.1` works only for clients on that machine, and the card says so. This change does not deploy, tunnel or expose anything.

## Tools

| Tool | Scope | What it does |
| --- | --- | --- |
| `list_projects` | any | The projects the user granted, with ids, machine and online state. |
| `start_session` | `sessions` | Starts a session in a project with the task. Same path as a connector run. Keyed by `request_id`. |
| `send_message` | `sessions` | Types a follow-up into one of the agent's own sessions. Keyed by `request_id`. |
| `read_session` | `sessions` | State, the question the session asks the user, and the latest conversation from a cursor. |
| `list_sessions` | `sessions` | The agent's own sessions with their state. |
| `message_orchestrator` | `orchestrator` | Messages the project's orchestrator. It starts the orchestrator first when it is not running. Keyed by `request_id`. |
| `read_orchestrator` | `orchestrator` | The orchestrator's state and conversation. |

The states are `starting`, `working`, `done`, `failed`, `needs_permission`, `waiting_for_input`, `ended`, `offline`, `not_found` and `not_started`.

## Security model

- **Identity.** The agent never holds a login token. It holds an access token, valid for one hour, and a rotating refresh token, valid for 30 days. Both are bound to one grant: the account, the scopes and a list of `(machine, project path)` pairs. The hub stores only their sha256, as it does for logins. Machines can't approve, list or revoke agents. Those calls are not on the list of callbacks a machine may forward.
- **OAuth 2.1.** RFC 9728 protected-resource metadata is served at `/.well-known/oauth-protected-resource[/mcp]`. RFC 8414 authorization-server metadata is at `/.well-known/oauth-authorization-server`. RFC 7591 dynamic registration accepts public clients only. PKCE must be S256. Codes are one-use and expire after five minutes. A code or a rotated refresh token that comes back ends its grant. `resource` (RFC 8707) must name this hub's `/mcp`. The redirect carries `iss` (RFC 9207). Revocation (RFC 7009) is at `/oauth/revoke`. A redirect URI must be https, loopback http, or a native app scheme. `javascript:`, `data:`, `file:` and remote plain http are refused. An authorization with an unregistered redirect never redirects.
- **Consent.** The approval is a hub call on the account's signed-in socket, so a page on another site can't forge it. The consent card refuses to approve inside a frame. Requests expire after ten minutes and are answered once. `/connect-agent` is sent with `Cache-Control: no-store` and `Referrer-Policy: no-referrer`. The page drops the request id from its address at once.
- **Scope.** Every project id is checked against the grant. A project of another account or machine gets the same answer as an unknown one. Sessions are found by the grant that started them, never by tab id. At approval, the hub checks each project against the list its machine reports.
- **Permissions stay with the user.** A session starts without `--dangerously-skip-permissions` and with `--permission-mode default`. Its task goes in as Claude's argv prompt, as a connector run's does, so any startup dialog waits for the user. Typed text ends with Enter, which would pick an option of a menu on screen. So the hub types only into a session a hook has reported idle (`done` or `failed`). It refuses `starting`, `working`, `needs_permission` and `waiting_for_input`. A stopped orchestrator is started with nothing typed into it. Nothing here can answer a permission prompt.
- **No duplicates.** Each side-effecting call needs a `request_id`. Postgres keeps the first outcome per grant and key. A retry answers that outcome without acting again. The same key with other arguments is refused. When the machine refused (`Unanswered::Refused`), nothing happened and the key is released. When the answer was lost (`Unanswered::Lost`), the call may have acted: the key stays `unknown`, and a retry never sends it again. A lost `start_session` is settled from the machine's tabs, since the session's tab carries `PINNED_CONNECTOR=mcp:<session id>`. A lost message is reported as `delivered: "unknown"`. One agent may start at most ten sessions an hour.
- **Origin.** `/mcp` refuses a request whose `Origin` is not the hub's own, as MCP requires against DNS rebinding.

## Fix in passing

Before this change, `Account::went_offline` answered every pending call with `Err("… went offline")`, which `Hub::answer` read as `Refused`. A machine that dropped mid-call therefore looked as if it had done nothing, and `open_on_pool` could hand a run that may have started to a second machine. It now drops the pending answers, so callers see `Lost`, which is what the `Unanswered` docs and the `open_on_pool` comment already describe. The error text is the same.

## Limits

- **Not tested with Grok or any hosted agent.** The tests and screenshots use a test MCP client: the hub's integration tests, and `docs/evidence/external-mcp/capture.js` against `scripts/web-sweep/stack.sh`. No production hub, account or agent was connected.
- Responses are plain JSON. There is no SSE stream (`GET /mcp` answers 405) and no server-initiated notifications. An agent polls `read_session`.
- Client ID Metadata Documents and CORS for browser-based MCP clients are not implemented.
- A freshly started orchestrator, or a session whose task didn't start a turn, takes a message only once Claude reports it idle. Claude sends that notice after about a minute at the prompt.
- The OAuth protocol logic is written here on the crates the hub already uses: `sha2`, `rand_core`'s OS RNG, `subtle` for the PKCE comparison, and `url`. No new OAuth library was added. It is small and covered by unit and integration tests, but it has had no external security review.
