//! `pinned call`: a team bot's session asks the team's people instead of guessing. The call goes
//! through the node, which sends it to the hub as `HubCall::Team { op: AskPeople }`. The hub
//! answers at once with the call's id and who was asked; the people's answer comes later, typed
//! into the session as its next message. The session never names people.

use crate::agent::Message;
use crate::teams::Situation;
use serde_json::Value;

pub const SUBCOMMAND: &str = "call";
/// Set to `1` in the environment of a team's box (its `box.env`), for its node and every session.
pub const TEAM_BOX_ENV: &str = "PINNED_TEAM_BOX";
pub const NOT_TEAM_BOX: &str = "pinned call works only in a team's box";
const USAGE: &str = "usage: pinned call --doubt \"<your question>\" [--choice <answer>]... | --before \"<what you are about to do>\" [--choice <answer>]...";

/// Whether the value of `PINNED_TEAM_BOX` says this process runs in a team's box.
pub fn is_team_box(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Whether this process's environment says it runs in a team's box.
pub fn in_team_box() -> bool {
    is_team_box(std::env::var(TEAM_BOX_ENV).ok().as_deref())
}

#[derive(Debug, PartialEq)]
struct Args {
    situation: Situation,
    question: String,
    choices: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let (mut asked, mut choices) = (None, Vec::new());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().map(|v| v.trim()).filter(|v| !v.is_empty()).map(str::to_owned).ok_or(USAGE);
        match arg.as_str() {
            "--doubt" | "--before" if asked.is_some() => return Err(USAGE.into()),
            "--doubt" => asked = Some((Situation::InDoubt, value()?)),
            "--before" => asked = Some((Situation::BeforeIrreversible, value()?)),
            "--choice" => choices.push(value()?),
            _ => return Err(USAGE.into()),
        }
    }
    let (situation, question) = asked.ok_or(USAGE)?;
    Ok(Args { situation, question, choices })
}

/// Who the hub says it asked and under which call, from the hub's immediate answer.
#[derive(Debug, PartialEq)]
pub struct Asked {
    pub id: String,
    pub who: Vec<String>,
}

/// Reads the hub's answer to `AskPeople`: an object with the call's `id` and the `asked` people,
/// each a handle or `Who` as the hub shows it (`@design`, `the owner`, or `{"kind":"group",...}`).
pub fn parse_asked(reply: &str) -> Result<Asked, String> {
    let value: Value = serde_json::from_str(reply).map_err(|_| format!("the hub answered {reply:?}, which names no call"))?;
    let id = ["id", "call", "call_id"].iter().find_map(|k| value[*k].as_str()).filter(|id| !id.is_empty());
    let id = id.ok_or_else(|| format!("the hub answered {reply:?}, which names no call"))?.to_owned();
    let who = value["asked"]
        .as_array()
        .map(|asked| {
            asked
                .iter()
                .filter_map(|who| match who {
                    Value::String(name) => Some(name.clone()),
                    other => serde_json::from_value::<crate::teams::Who>(other.clone()).ok().map(|who| who.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Asked { id, who })
}

/// `@design`, `@design and the owner`, `@a, @b and the owner`; `the team` when the hub named no one.
fn names(who: &[String]) -> String {
    match who {
        [] => "the team".into(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// What the session reads after a call went out.
pub fn asked_text(asked: &Asked) -> String {
    format!("Asked {} (call {}). The answer will come as your next message. End your turn now and wait; do not do the step.", names(&asked.who), asked.id)
}

/// What the subcommand prints and the exit code it ends with.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub code: i32,
    pub out: String,
    pub err: String,
}

/// Runs `pinned call` with `args`. `team_box` is the environment's word on where it runs; `ask`
/// sends the message to the node and returns the hub's answer. Outside a team's box nothing is
/// parsed or sent.
pub fn run(args: &[String], team_box: bool, ask: impl FnOnce(Message) -> Result<String, String>) -> Outcome {
    let fail = |code, err: String| Outcome { code, out: String::new(), err };
    if !team_box {
        return fail(2, NOT_TEAM_BOX.into());
    }
    let args = match parse_args(args) {
        Ok(args) => args,
        Err(usage) => return fail(2, usage),
    };
    let reply = match ask(Message::AskPeople { situation: args.situation, question: args.question, choices: args.choices }) {
        Ok(reply) => reply,
        Err(e) => return fail(1, e),
    };
    match parse_asked(&reply) {
        Ok(asked) => Outcome { code: 0, out: asked_text(&asked), err: String::new() },
        Err(e) => fail(1, e),
    }
}

/// Runs when a session calls `pinned call ...`; returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let outcome = run(args, in_team_box(), crate::hooks::ask);
    if !outcome.out.is_empty() {
        println!("{}", outcome.out);
    }
    if !outcome.err.is_empty() {
        eprintln!("{}", outcome.err);
    }
    outcome.code
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn sent(args_: &[&str], team_box: bool, reply: Result<&str, &str>) -> (Outcome, Option<Message>) {
        let seen = RefCell::new(None);
        let outcome = run(&args(args_), team_box, |m| {
            *seen.borrow_mut() = Some(m);
            reply.map(str::to_owned).map_err(str::to_owned)
        });
        (outcome, seen.into_inner())
    }

    #[test]
    fn doubt_and_before_pick_the_situation_and_choices_repeat() {
        assert_eq!(parse_args(&args(&["--doubt", "Which color?"])), Ok(Args { situation: Situation::InDoubt, question: "Which color?".into(), choices: vec![] }));
        assert_eq!(
            parse_args(&args(&["--before", "deploy to staging", "--choice", "Go ahead", "--choice", "Don't"])),
            Ok(Args { situation: Situation::BeforeIrreversible, question: "deploy to staging".into(), choices: vec!["Go ahead".into(), "Don't".into()] })
        );
        assert_eq!(parse_args(&args(&["--choice", "A", "--doubt", "Which?", "--choice", "B"])).unwrap().choices, ["A", "B"], "flags in any order");
    }

    #[test]
    fn a_missing_or_empty_question_or_a_stray_argument_is_a_usage_error() {
        for bad in [&[][..], &["--doubt"], &["--doubt", "  "], &["--before"], &["--choice", "A"], &["--doubt", "q", "--choice"], &["--doubt", "q", "--before", "p"], &["--doubt", "q", "extra"], &["question"]] {
            assert_eq!(parse_args(&args(bad)), Err(USAGE.to_owned()), "{bad:?}");
        }
        let (outcome, message) = sent(&["--doubt"], true, Ok("{}"));
        assert_eq!((outcome.code, outcome.err.as_str(), outcome.out.as_str()), (2, USAGE, ""));
        assert!(message.is_none(), "a usage error contacts nothing");
    }

    #[test]
    fn outside_a_team_box_it_refuses_with_exit_2_and_contacts_nothing() {
        let (outcome, message) = sent(&["--doubt", "Which color?"], false, Ok(r#"{"id":"k1"}"#));
        assert_eq!(outcome, Outcome { code: 2, out: String::new(), err: "pinned call works only in a team's box".into() });
        assert!(message.is_none());
        let (outcome, _) = sent(&[], false, Ok("{}"));
        assert_eq!(outcome.code, 2, "even before its arguments are read");
        assert!(!is_team_box(None) && !is_team_box(Some("0")) && !is_team_box(Some("")) && is_team_box(Some("1")));
    }

    #[test]
    fn a_call_sends_the_ask_and_prints_who_was_asked() {
        let reply = r#"{"id":"k1","asked":[{"kind":"group","handle":"design"}]}"#;
        let (outcome, message) = sent(&["--doubt", "Which color?", "--choice", "red", "--choice", "blue"], true, Ok(reply));
        assert_eq!(
            outcome,
            Outcome { code: 0, out: "Asked @design (call k1). The answer will come as your next message. End your turn now and wait; do not do the step.".into(), err: String::new() }
        );
        let Some(Message::AskPeople { situation, question, choices }) = message else { panic!("no ask") };
        assert_eq!((situation, question.as_str(), choices), (Situation::InDoubt, "Which color?", vec!["red".to_owned(), "blue".to_owned()]));
        let (_, message) = sent(&["--before", "tag a release"], true, Ok(reply));
        assert!(matches!(message, Some(Message::AskPeople { situation: Situation::BeforeIrreversible, .. })));
    }

    #[test]
    fn the_hubs_answer_reads_handles_owner_and_plain_strings() {
        let asked = |reply: &str| parse_asked(reply).map(|a| asked_text(&a));
        assert!(asked(r#"{"id":"k2","asked":["@design","the owner"]}"#).unwrap().starts_with("Asked @design and the owner (call k2)."));
        assert!(asked(r#"{"id":"k3","asked":[{"kind":"owner"}]}"#).unwrap().starts_with("Asked the owner (call k3)."));
        assert!(asked(r#"{"id":"k4","asked":["@a","@b",{"kind":"owner"}]}"#).unwrap().starts_with("Asked @a, @b and the owner (call k4)."));
        assert!(asked(r#"{"id":"k5"}"#).unwrap().starts_with("Asked the team (call k5)."));
        assert!(asked(r#"{"asked":["@design"]}"#).is_err() && asked("not json").is_err());
    }

    #[test]
    fn a_hub_refusal_prints_the_hubs_error_and_exits_1() {
        let (outcome, _) = sent(&["--doubt", "Which?"], true, Err("a machine may not send the team op chat"));
        assert_eq!(outcome, Outcome { code: 1, out: String::new(), err: "a machine may not send the team op chat".into() });
        let (outcome, _) = sent(&["--doubt", "Which?"], true, Ok("fine"));
        assert_eq!(outcome.code, 1, "an answer with no call id is no ask");
    }
}
