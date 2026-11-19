//! What this machine answers from its own store when it is signed out. The desktop's hook
//! listener calls [`signed_out`] with every message the account hands back.

use crate::agent::{Message, Stage, Subject};
use crate::connectors::Connectors;

/// What `ask_people` answers when no hub can take the question.
pub const NEEDS_THE_HUB: &str = "needs the hub";

/// A callback the account handed back, answered here or left for the desktop's connectors.
#[derive(Debug, PartialEq)]
pub enum SignedOut {
    Done(Result<String, String>),
    Stage {
        connector: String,
        subject: Subject,
        stage: Stage,
    },
    SlackReply {
        connector: String,
        channel: String,
        thread: String,
        text: String,
    },
    LinearCreate {
        connector: String,
        channel: String,
        thread: String,
        title: String,
        description: String,
        assignee: String,
    },
    LinearTake {
        connector: String,
        channel: String,
        thread: String,
        issue: String,
    },
}

/// Answers a signed-out hook message. `RunSet` goes to the store's live run. `AskPeople` cannot.
pub fn signed_out(connectors: &Connectors, message: Message) -> SignedOut {
    match message {
        Message::Stage {
            connector,
            subject,
            stage,
        } => SignedOut::Stage {
            connector,
            subject,
            stage,
        },
        Message::SlackReply {
            connector,
            channel,
            thread,
            text,
        } => SignedOut::SlackReply {
            connector,
            channel,
            thread,
            text,
        },
        Message::LinearCreate {
            connector,
            channel,
            thread,
            title,
            description,
            assignee,
        } => SignedOut::LinearCreate {
            connector,
            channel,
            thread,
            title,
            description,
            assignee,
        },
        Message::LinearTake {
            connector,
            channel,
            thread,
            issue,
        } => SignedOut::LinearTake {
            connector,
            channel,
            thread,
            issue,
        },
        Message::RunSet {
            connector,
            subject,
            name,
            value,
        } => SignedOut::Done(connectors.run_set(&connector, &subject, &name, &value)),
        Message::AskPeople { .. } => SignedOut::Done(Err(NEEDS_THE_HUB.into())),
        Message::Agent(_)
        | Message::Compacting { .. }
        | Message::Team { .. }
        | Message::Remind { .. }
        | Message::ListReminders
        | Message::CancelReminder { .. }
        | Message::Project { .. }
        | Message::Guard { .. }
        | Message::Usage { .. }
        | Message::RefreshUsage { .. }
        | Message::Delegate { .. }
        | Message::TaskStatus { .. }
        | Message::TaskCancel { .. }
        | Message::Capabilities { .. }
        | Message::TaskReview { .. }
        | Message::DelegationStats { .. } => SignedOut::Done(Err(
            "only a workflow session's callbacks go to the hub".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Subject;
    use crate::connectors::FileStorage;
    use crate::graph::runner::{self, Session, Status};
    use crate::graph::{Graph, Node, NodeId, NodeKind, Watch};
    use crate::machine::refuse_real_home;
    use crate::teams::Situation;
    use std::collections::BTreeMap;

    fn live_run(dir: &std::path::Path, connector: &str, subject: Subject) {
        let graph = Graph {
            nodes: vec![Node {
                id: NodeId("t".into()),
                name: "start".into(),
                position: [0, 0],
                group_name: None,
                kind: NodeKind::Trigger {
                    watch: Watch::Manual,
                },
            }],
            edges: vec![],
        };
        let library = BTreeMap::from([(connector.to_owned(), graph)]);
        let mut run = runner::start(
            &library,
            connector,
            subject,
            serde_json::json!({}),
            "2026-11-19T00:00:00Z",
        );
        assert_eq!(run.status, Status::Running);
        run.session = Session::Live {
            machine: "local".into(),
            tab: "tab".into(),
            claude: None,
            since: 0,
            saw_working: false,
            await_working: false,
        };
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("runs.json"),
            serde_json::to_vec(&vec![run]).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn signed_out_run_set_and_ask_people_are_answered() {
        refuse_real_home();
        let dir = std::env::temp_dir().join(format!("pinned-signed-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let subject = Subject::LinearIssue {
            issue: "ISS-1".into(),
        };
        live_run(&dir, "wf", subject.clone());
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into());

        let set = signed_out(
            &connectors,
            Message::RunSet {
                connector: "wf".into(),
                subject: subject.clone(),
                name: "title".into(),
                value: "Export".into(),
            },
        );
        assert_eq!(
            set,
            SignedOut::Done(Ok("set {{vars.title}} for this workflow run".into()))
        );
        let stored = connectors
            .run_set("wf", &subject, "title", "Export")
            .unwrap();
        assert_eq!(stored, "set {{vars.title}} for this workflow run");

        let missing = signed_out(
            &connectors,
            Message::RunSet {
                connector: "wf".into(),
                subject: Subject::LinearIssue {
                    issue: "other".into(),
                },
                name: "title".into(),
                value: "x".into(),
            },
        );
        assert!(matches!(missing, SignedOut::Done(Err(_))));

        let ask = signed_out(
            &connectors,
            Message::AskPeople {
                situation: Situation::InDoubt,
                question: "Which?".into(),
                choices: vec!["red".into()],
                step: None,
            },
        );
        assert_eq!(ask, SignedOut::Done(Err(NEEDS_THE_HUB.into())));

        let _ = std::fs::remove_dir_all(dir);
    }
}
