# A tab id belongs to one process

A tab id names one tab of one process. The next process draws new ids, so a client that still holds the old ones cannot type into or close a different session. The conversation is `TabView.session`, which is what a restart restores. `sessions.json` does not store the tab id.

## What an id is

`tab-<boot>-<n>`. `boot` is chosen when the process starts and is not stored. `n` is the order the tab opened in that process, and it is the part after the last `-`. `tab-1` from an older node still orders as 1. Session order and the project's orchestrator read that number.

## What clients hear

`quit` sends an empty tab list before it drops the tabs, while the hub socket is still up. A machines frame includes `tabs` for the machines that are online. An older hub leaves the field out, and an older client ignores it. A client applies `tabs` when it is present, including when it already thought the machine was online. It asks for the list only when a machine comes online and the frame has no `tabs`.

A `start_claude` that answers `no tab` reloads that machine's tabs. Enter starts the restored tab when the same session is open under a new id. Otherwise the composer says the session ended.
