# Owner execute and a failed repair

A plugin file is executable when its owner bit is set. Group or other execute is copied onto the mirror, and the owner bit is added before that machine publishes the file. The other machine adds only the owner bit.

`SyncList`, `SyncPush`, and `SyncPull` carry the bytes. A same-bytes repair sends `sync_mark_executable`. See `rules/executable-plugin-hooks.md`.

If that mark times out, or the local chmod fails, the saved agreement stays the one from the last successful sync. The next sync still compares both sides to that agreement. An edit on both sides is not resolved by mtime.

The checks are `a_group_executable_hook_becomes_owner_executable_locally_and_on_the_receiver`, `a_failed_mark_keeps_the_agreed_baseline_and_the_next_sync_pushes_the_local_edit`, `a_failed_chmod_keeps_the_agreed_baseline_and_the_next_sync_pushes_the_local_edit` in `crates/core/src/sync.rs`, and `a_node_marks_the_executable_bit_and_keeps_a_newer_edit` in `crates/server/tests/hub.rs`.
