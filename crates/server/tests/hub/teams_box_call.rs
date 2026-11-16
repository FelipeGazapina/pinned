//! A team box's `pinned call` reaches the hub as a machine frame: the node forwards the session's
//! ask as `HubCall::Team { op: AskPeople }` and the hub runs it through `teams::machine_call`.

use super::*;
use pinned_core::teams::Situation;

#[test]
fn a_machines_ask_people_frame_reaches_the_teams_machine_call() {
    let hub = TestHub::start("teamboxcall");
    let url = hub.url.clone();
    let (token, account) = account::signup(&url, "ana@acme.dev", "correct horse").unwrap();
    let (machine, machine_token) = account::register_machine(&url, &token, "box", "linux").unwrap();
    let dir = temp("teambox-call-node");
    Credentials { hub: url.clone(), email: account.email.clone(), user_token: token.clone(), machine_id: machine.id, machine_token }.save(&dir).unwrap();
    let node = Node::start(&dir, Credentials::load(&dir).unwrap());
    wait_for("the box node online", || node.online());
    let link = node.link.as_ref().unwrap();

    let ask = |situation| agent::Message::AskPeople { situation, question: "Which color?".into(), choices: vec!["red".into(), "blue".into()], step: None };
    let refused = link.forward(ask(Situation::InDoubt)).expect_err("no team's box runs this node, so nobody can be asked");
    // Whatever the calls unit answers (not built yet, or a refusal of a machine that is no team's),
    // the frame got to `ask_people`: the hub neither failed to read it nor refused the op itself.
    for not_this in ["unknown", "may not send", "not a hub call", "only a workflow session"] {
        assert!(!refused.contains(not_this), "the ask did not reach teams::machine_call: {refused}");
    }
    assert!(!pinned_core::link::is_unanswered(&refused), "the hub answered: {refused}");

    // The CLI end to end, minus the socket: its message goes through the same forward.
    let args: Vec<String> = ["--before", "tag a release", "--choice", "Go ahead", "--choice", "Don't"].map(String::from).into();
    let outcome = pinned_core::team_call::run(&args, true, |message| link.forward(message));
    assert_eq!((outcome.code, outcome.out.as_str()), (1, ""), "{outcome:?}");
    assert_eq!(outcome.err, refused);
    let outside = pinned_core::team_call::run(&args, false, |message| link.forward(message));
    assert_eq!((outside.code, outside.err.as_str()), (2, "pinned call works only in a team's box"));
}
