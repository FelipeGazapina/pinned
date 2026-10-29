//! `snapshot <version>` writes the hub's wire contract. `check` diffs it against every snapshot.

use pinned_core::wire_contract::{self, Break, Contract, Delta};
use pinned_server::http::ROUTES;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("snapshot") => {
            let Some(version) = args.next() else {
                return usage();
            };
            match write_snapshot(&version) {
                Ok(path) => {
                    println!("{}", path.display());
                    ExitCode::SUCCESS
                }
                Err(err) => fail(err),
            }
        }
        Some("check") => {
            let base = match args.next().as_deref() {
                None => None,
                Some("--base") => match args.next() {
                    Some(rev) => Some(rev),
                    None => return usage(),
                },
                Some(_) => return usage(),
            };
            code(check(base.as_deref()))
        }
        _ => usage(),
    }
}

fn write_snapshot(version: &str) -> Result<PathBuf, String> {
    let generated = wire_contract::current(wire_contract::CURRENT_VERSION, ROUTES);
    if version == wire_contract::CURRENT_VERSION {
        return wire_contract::store(&generated);
    }
    let committed = wire_contract::load(&wire_contract::current_path()).ok();
    wire_contract::require_fresh(&generated, committed.as_ref())?;
    let mut released = generated;
    released.version = version.to_string();
    wire_contract::store(&released)
}

fn check(base: Option<&str>) -> Result<Outcome, String> {
    let generated = wire_contract::current(wire_contract::CURRENT_VERSION, ROUTES);
    let committed = wire_contract::load(&wire_contract::current_path()).ok();
    wire_contract::require_fresh(&generated, committed.as_ref())?;
    let snapshots = load_snapshots()?;
    let head = wire_contract::collect(&snapshots, &generated);
    match base {
        None => Ok(Outcome::Absolute(head)),
        Some(rev) => {
            let earlier = base_contract(rev)?;
            let base_breaks = match &earlier {
                Some(contract) => wire_contract::collect(&snapshots, contract),
                None => Vec::new(),
            };
            let status = name_status(rev)?;
            Ok(Outcome::Compared(wire_contract::classify(
                &head,
                &base_breaks,
                &status,
            )))
        }
    }
}

enum Outcome {
    Absolute(Vec<Break>),
    Compared(Delta),
}

fn code(result: Result<Outcome, String>) -> ExitCode {
    match result {
        Ok(outcome) => {
            let text = render(&outcome);
            print!("{text}");
            if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") {
                let _ = std::fs::write(
                    &path,
                    std::fs::read_to_string(&path).unwrap_or_default() + &text,
                );
            }
            if gates(&outcome) {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => fail(err),
    }
}

fn fail(err: String) -> ExitCode {
    eprintln!("wire-contract: {err}");
    ExitCode::from(2)
}

fn render(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Absolute(breaks) => wire_contract::report(breaks),
        Outcome::Compared(delta) => wire_contract::report_delta(delta),
    }
}

fn gates(outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Absolute(breaks) => !breaks.is_empty(),
        Outcome::Compared(delta) => !delta.new_breaks.is_empty(),
    }
}

fn load_snapshots() -> Result<Vec<Contract>, String> {
    let dir = wire_contract::snapshot_dir();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|err| format!("{}: {err}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| path.file_name().is_some_and(|name| name != "current.json"))
        .collect();
    files.sort();
    let mut snapshots = Vec::new();
    for path in files {
        snapshots.push(wire_contract::load(&path)?);
    }
    Ok(snapshots)
}

fn base_contract(rev: &str) -> Result<Option<Contract>, String> {
    let root = wire_contract::repo_root();
    let verify = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .current_dir(&root)
        .output()
        .map_err(|err| err.to_string())?;
    if !verify.status.success() {
        return Err(format!("unknown git rev `{rev}`"));
    }
    let spec = format!("{rev}:crates/core/wire-contract/current.json");
    let show = Command::new("git")
        .args(["show", &spec])
        .current_dir(&root)
        .output()
        .map_err(|err| err.to_string())?;
    if !show.status.success() {
        return Ok(None);
    }
    serde_json::from_slice(&show.stdout)
        .map(Some)
        .map_err(|err| format!("{spec}: {err}"))
}

fn name_status(base: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args([
            "diff",
            "--name-status",
            base,
            "--",
            "crates/core/wire-contract",
        ])
        .current_dir(wire_contract::repo_root())
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "git diff --name-status {base}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn usage() -> ExitCode {
    eprintln!("usage: wire-contract <snapshot <version>|check [--base <git-rev>]>");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_current_json_exits_2() {
        let generated = Contract {
            version: wire_contract::CURRENT_VERSION.into(),
            routes: vec!["POST /api/login".into()],
            inbound: Default::default(),
            outbound: Default::default(),
        };
        let mut committed = generated.clone();
        committed.routes.push("GET /ws/client".into());
        let err = wire_contract::require_fresh(&generated, Some(&committed)).unwrap_err();
        assert_eq!(err, wire_contract::STALE);
        assert_eq!(code(Err(err)), ExitCode::from(2));
        assert_eq!(
            code(Ok(Outcome::Compared(Delta {
                new_breaks: vec![Break {
                    version: "0.8.0".into(),
                    direction: "inbound".into(),
                    path: "HubCall.connect_slack".into(),
                    kind: "removed_variant".into(),
                    impact: "apps 0.8.0 send `connect_slack`; this hub rejects it".into(),
                }],
                shipped: vec![],
            }))),
            ExitCode::from(1)
        );
        assert_eq!(code(Ok(Outcome::Absolute(vec![]))), ExitCode::SUCCESS);
        assert_eq!(
            code(Ok(Outcome::Compared(Delta {
                new_breaks: vec![],
                shipped: vec![],
            }))),
            ExitCode::SUCCESS
        );
    }
}
