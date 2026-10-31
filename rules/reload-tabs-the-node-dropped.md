# A tab id belongs to one process

A tab id names one tab of one process. The next process draws new ids, so a client that still holds the old ones cannot type into or close a different session. The conversation is `TabView.session`, which is what a restart restores. `sessions.json` does not store the tab id.

## What an id is

`tab-<boot>-<n>`. `boot` is chosen when the process starts and is not stored. `n` is the order the tab opened in that process, and it is the part after the last `-`. `tab-1` from an older node still orders as 1. Session order and the project's orchestrator read that number.

## What clients hear

`quit` sends an empty tab list before it drops the tabs, while the hub socket is still up. A machines frame's `tabs` map includes a machine only after that socket's first Tabs event. Missing means the node has not reported yet. `[]` means it reported no tabs. The frame is queued under the same lock as the Tabs event. An older hub leaves the field out, and an older client ignores it. A client applies an entry only when that machine's key is present. It asks for the list when a machine comes online and the key is absent. It does not treat a missing key as an empty list.

Subscribing before `link::spawn` returns does not buffer events. `Shared::send` drops them until the socket exists. The tabs frame sent after hello is the list the hub stores.

A `no tab` error reloads that machine once per machine while a reload is in flight, and only when the call named a tab. Enter on a stale id activates the restored tab with the same session. It does not start that tab. A second `start_claude` on a tab that is starting, or running with nothing new queued, leaves the conversation alone. Otherwise the composer says the session ended.
