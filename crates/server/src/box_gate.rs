//! What the hub takes from a box machine of a person's account. A box is a machine of the account
//! that opened it, and the account's other machines load and run what reaches its synced and
//! shared files, so a box may only pull the account's files: the hub refuses every call of a box
//! that would change what the account's other machines or clients read.

use pinned_core::wire::HubCall;

/// The refusal a box's write gets. It starts with `sync::LIMIT_REFUSAL`, so a node that reads a
/// refused push as the account's size limit backs off its push and still pulls in the same cycle.
pub const BOX_REFUSAL: &str = "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files";

/// Whether a box machine may send `call`.
pub fn box_may_send(_call: &HubCall) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{box_may_send, BOX_REFUSAL};
    use pinned_core::sync;
    use pinned_core::wire::HubCall;

    fn call(value: serde_json::Value) -> HubCall {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_refusal_reads_as_the_accounts_size_limit_to_a_node() {
        assert_eq!(BOX_REFUSAL, "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files");
        assert!(BOX_REFUSAL.starts_with(sync::LIMIT_REFUSAL));
    }

    #[test]
    fn a_box_may_only_pull_the_accounts_files() {
        let refused = Err("the account's synced projects would pass 0 MB from a box: a box only pulls the account's files".to_owned());
        let push = call(serde_json::json!({ "cmd": "sync_push", "project": "pinned:plugins", "files": [] }));
        assert_eq!(box_may_send(&push), refused);
        let mark = call(serde_json::json!({ "cmd": "sync_mark_executable", "project": "pinned:plugins", "repairs": [{ "path": "plugins/a/hooks/h.sh", "sha": "00" }] }));
        assert_eq!(box_may_send(&mark), refused);
        let share = call(serde_json::json!({ "cmd": "share_project", "project": { "key": "github.com/acme/site", "name": "site", "repo": "git@github.com:acme/site.git" } }));
        assert_eq!(box_may_send(&share), refused);
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "sync_list", "project": "pinned:plugins" }))), Ok(()));
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "sync_pull", "project": "pinned:plugins", "paths": ["user-skills/ship/SKILL.md"] }))), Ok(()));
    }
}
