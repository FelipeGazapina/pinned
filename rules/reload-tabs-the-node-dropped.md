# Reload tabs the node dropped

The composer shows "claude isn't running. Press Enter to start it." while `tab.view.running` is false. Enter calls `start_claude`. The model button stays on "Default" until a thread loads, and a thread loads only after that call succeeds.

## What went wrong

The hub answered those calls with `no tab <id>`. The node did not have the id the client sent, so Enter did not start Claude. The client asked for a fresh tab list only when a machine changed from offline to online. An update that arrived while the client already treated the machine as online left the old ids in place. A later `no tab` error was written to the hidden terminal and the tab stayed.

On the railway box, Claude started with the same arguments pinned builds, including the synced plugin directories, and stayed up. It answered a prompt. The missing tab id is what blocked the composer.

## What the client does now

`tabListAction` returns `refresh` for an online machine when the update has no tab snapshot. `callOn` reloads that machine's tabs when a call fails with `no tab <id>`, then `startClaude` writes the error only if the tab is still open.

The hub serves `ui/main.js`. This fix ships with the hub.
