//! The subcommands Claude Code and connector sessions run on the binary that started them.
//! Every pinned binary answers them the same way, so a session can't tell which one hosts it.

use crate::{evidence, hooks, reminders};

/// Runs the subcommand `args` names (the arguments after the program) and returns the exit
/// code, or None when `args` is not one, so the caller goes on to start normally.
pub fn dispatch(args: &[String]) -> Option<i32> {
    let (command, rest) = args.split_first()?;
    let first = rest.first().map(String::as_str);
    let answer = match command.as_str() {
        hooks::SUBCOMMAND => {
            hooks::forward(first);
            return Some(0);
        }
        hooks::STATUSLINE_SUBCOMMAND => {
            hooks::statusline();
            return Some(0);
        }
        reminders::SUBCOMMAND => {
            reminders::serve(first != Some(reminders::NO_REMINDERS));
            return Some(0);
        }
        hooks::STAGE_SUBCOMMAND => hooks::stage(first),
        hooks::SLACK_SUBCOMMAND => hooks::slack(rest),
        hooks::LINEAR_SUBCOMMAND => hooks::linear(rest),
        evidence::SUBCOMMAND => evidence::run(rest),
        _ => return None,
    };
    Some(match answer {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => {
            eprintln!("pinned: {e}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_subcommands_are_dispatched() {
        assert_eq!(dispatch(&[]), None);
        assert_eq!(dispatch(&args(&["/Users/ana/acme"])), None);
        assert_eq!(dispatch(&args(&["stage", "shipping"])), Some(1));
        assert_eq!(dispatch(&args(&["linear", "file"])), Some(1));
    }
}
