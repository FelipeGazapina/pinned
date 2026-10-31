# Owner execute and a failed repair

A plugin file counts as executable when any execute bit is set. Group or other execute is copied onto the mirror, and the owner bit is added before that machine publishes the file. The install Claude Code runs gains that bit before the mirror is copied. The other machine adds only the owner bit. Asking whether a file is executable does not change its mode.

`SyncList`, `SyncPush`, and `SyncPull` carry the bytes. A same-bytes repair sends `sync_mark_executable`. See `rules/executable-plugin-hooks.md`.

A mark that times out, or a chmod that fails, leaves the agreement unchanged when it happens before a hub call changes a file. After a push or a pull returns, that file's sha is the agreement, and the state file is written before the next call. A later error in the same sync keeps those shas. Project sync follows the same rule. A failed list leaves the project agreement in place. The next sync still compares both sides to that agreement. An edit on both sides is not resolved by mtime.

The checks are `a_group_executable_hook_becomes_owner_executable_locally_and_on_the_receiver`, `a_654_file_reads_as_executable_and_keeps_its_mode`, `a_654_install_is_owner_executable_after_sync`, `a_failed_mark_keeps_the_agreed_baseline_and_the_next_sync_pushes_the_local_edit`, `a_failed_chmod_keeps_the_agreed_baseline_and_the_next_sync_pushes_the_local_edit`, `a_push_then_a_failed_pull_keeps_the_other_machines_older_edit`, `a_failed_project_list_keeps_the_baseline_and_the_next_sync_pushes_the_local_edit`, and `a_project_push_then_a_failed_pull_keeps_the_other_machines_older_edit` in `crates/core/src/sync.rs`, and `a_node_marks_the_executable_bit_and_keeps_a_newer_edit` in `crates/server/tests/hub.rs`.
