# Agreement after each hub call

Each machine keeps the sha it last agreed with the hub for every synced file. Sync writes that sha as soon as the hub call that changed the file returns, and stores the state file before the next call. A later error in the same sync leaves those shas in place. The next sync still has an ancestor. It pulls the other machine's older edit and keeps that edit.

A failure before any hub call changes a file leaves the previous agreement as it was. That includes a mark that times out and a chmod that fails. Project sync used to remove the agreement before the exchange and then save the hole when the list failed. It now keeps the agreement, and it commits each landed push or pull the same way plugin sync does.

`executable` only reads the mode. It is true when any execute bit is set, and the call leaves the mode unchanged. The owner bit is added on the install before the mirror is copied, on a mirror whose bytes already match, and on a plugin file while it is packed for a push. An enabled plugin runs from its install path. A heal that touches only the mirror leaves that path without the owner bit.

The hub test `a_node_marks_the_executable_bit_and_keeps_a_newer_edit` marks a second hook in the same call that loses the race on the first hook. If `mark_executable` changes nothing, that second hook stays non-executable and the test fails.

The checks are the tests named in `rules/fix-sync-owner-exec-and-baseline.md`.
