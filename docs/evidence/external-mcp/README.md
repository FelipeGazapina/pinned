# External MCP evidence

These screenshots are real captures of this branch's web app, served by a real local hub. They are not mockups.

**What ran.** `scripts/web-sweep/stack.sh` started a throwaway stack: Postgres, `pinned-server hub` serving this checkout's `ui/`, an account, and a paired `pinned-server node`. The node's `claude` is `scripts/web-sweep/fake-claude`, the repository's test stand-in. It echoes prompts into a real transcript and raises a real permission prompt on `/ask`. [`capture.js`](capture.js) played the **external MCP client** itself. It registered, sent the signed-in browser (Playwright Chromium) through `/oauth/authorize`, received the code on its own loopback callback, exchanged it with PKCE, and called `/mcp`. Its setup opens the `alpha` and `beta` folders on the node as projects.

**What this is not.** It is not Grok or any hosted agent. It is not a production hub, account or deployment. It is not a live end-to-end test with a real model: the agent's sessions ran `fake-claude`. The tests that check the backend's behavior are the integration tests in `crates/server/tests/hub.rs`: `an_external_agent_connects_through_oauth_and_works_only_where_it_was_let_in` and `a_lost_answer_is_reported_as_unknown_and_never_acted_on_twice`.

`fake-claude` ignores its argv task and sends no idle notification. Real Claude does both. So `capture.js` types the first turn itself, through the account's own client socket, with the same `prompt` call the composer makes: the task, "Look at the README and summarize it.", into the session, and "Ready." into the orchestrator. Each first turn fires the `UserPromptSubmit` and `Stop` hooks that real Claude's argv task or its idle notice would. Until then, the transcript shows `send_message` refused and `message_orchestrator` starting the orchestrator with nothing typed. Later the harness types `/ask` as the user, so the session raises a permission prompt.

| File | Shows |
| --- | --- |
| [01-consent-card.png](01-consent-card.png) | The consent card a pending authorization opens on: the agent's self-chosen name, its exact redirect, scopes, and projects, none ticked. |
| [02-consent-in-connectors.png](02-consent-in-connectors.png) | The same card on Settings → Connectors. |
| [03-consent-picked.png](03-consent-picked.png) | The user picked both scopes and one project, `alpha`. |
| [04-test-client-callback.png](04-test-client-callback.png) | The test client's loopback callback received the code. The page is the harness's own and says so. |
| [05-connectors-agent-listed.png](05-connectors-agent-listed.png) | The External agents (MCP) card: the endpoint to copy, the guided steps, the warning that a non-HTTPS hub can't be reached by a hosted agent, and the connected agent with Revoke. |
| [06-agent-session-in-pinned.png](06-agent-session-in-pinned.png) | The session the agent started, as an Automation thread, with its message attributed to the agent. |
| [07-permission-stays-with-the-user.png](07-permission-stays-with-the-user.png) | The session asks for permission. Approve and Decline are the user's. At the same moment, the transcript shows `read_session` reporting `needs_permission` and `send_message` refused. |
| [08-orchestrator-message.png](08-orchestrator-message.png) | The project's own orchestrator thread with the agent's message. |
| [09-connectors-after-revoke.png](09-connectors-after-revoke.png) | After Revoke, the card lists no agent, and the next MCP call is 401. |
| [mcp-transcript.json](mcp-transcript.json) | Every discovery, registration, token and MCP exchange of the run, with tokens and codes redacted. |

To capture again (ports are examples):

```
SWEEP_DIR=/tmp/pinned-external-mcp-sweep SWEEP_PG_PORT=5491 SWEEP_HUB_PORT=8791 SWEEP_GO_PORT=8792 scripts/web-sweep/stack.sh start
HUB=http://127.0.0.1:8791 SWEEP_DIR=/tmp/pinned-external-mcp-sweep PLAYWRIGHT=<path to playwright> node docs/evidence/external-mcp/capture.js
SWEEP_DIR=/tmp/pinned-external-mcp-sweep scripts/web-sweep/stack.sh stop
```
