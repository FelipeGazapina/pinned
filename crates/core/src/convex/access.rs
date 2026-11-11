//! Which Convex reads a session may make. The hub decides here, from the tab list its machine
//! reported, and nowhere else.

use super::{Ask, Connection, Offer, UNKNOWN_TAB};
use crate::agent::{CAPTAIN_CONNECTOR, DELEGATE_CONNECTOR, EXTERNAL_CONNECTOR_PREFIX, INTERACTIVE_CONNECTOR};
use crate::wire::TabView;
use std::collections::BTreeSet;

/// How many tabs the walk up `spawned_by` reads, the calling tab included.
const WALK_MAX: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Everything,
    /// Listing these tables and reading their documents, nothing else.
    Tables(BTreeSet<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub connection: Connection,
    pub scope: Scope,
}

impl Grant {
    pub fn offer(&self) -> Offer {
        let tables = match &self.scope {
            Scope::Everything => None,
            Scope::Tables(tables) => Some(tables.iter().cloned().collect()),
        };
        Offer { deployment: self.connection.id.clone(), kind: self.connection.kind, tables }
    }
}

/// A workflow's Convex step, from its stored graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub connection: String,
    pub tables: Option<Vec<String>>,
}

/// Who a session works for, from the calling tab up its `spawned_by` chain.
enum Run {
    ByHand,
    Workflow(String),
}

/// `find(machine, tab)` is a tab as its machine reported it. `step(id)` is None when the account
/// has no workflow `id`, Some(None) when that workflow has no Convex step.
pub fn decide(
    machine: &str,
    tab: &str,
    repo: Option<&str>,
    find: &dyn Fn(&str, &str) -> Option<TabView>,
    connections: &[Connection],
    step: &dyn Fn(&str) -> Option<Option<Step>>,
    ask: &Ask,
) -> Result<Grant, String> {
    let calling = find(machine, tab).ok_or(UNKNOWN_TAB)?;
    if calling.orchestrator_global || calling.taste_chat {
        return Err("The global orchestrator and the taste chat do no project work, so they read no Convex data.".into());
    }
    let run = walk(machine, calling, find, step)?;
    let repo = repo.ok_or("This folder has no git origin, so it cannot be connected to Convex yet.")?;
    let connection = connections
        .iter()
        .find(|c| c.repos.iter().any(|r| r == repo))
        .ok_or_else(|| format!("Convex is not connected for `{repo}`. The user connects it under Settings, Connectors, Convex."))?;
    let scope = match run {
        Run::ByHand => Scope::Everything,
        Run::Workflow(id) => match step(&id).flatten() {
            None => return Err("This workflow has no Read Convex data step.".into()),
            Some(Step { connection: wanted, .. }) if wanted != connection.id => {
                return Err(format!("This workflow's Read Convex data step reads `{wanted}`, but this project is connected to `{}`.", connection.id));
            }
            Some(Step { tables: None, .. }) => Scope::Everything,
            Some(Step { tables: Some(tables), .. }) => Scope::Tables(tables.into_iter().collect()),
        },
    };
    admit(&scope, ask)?;
    Ok(Grant { connection: connection.clone(), scope })
}

fn walk(machine: &str, calling: TabView, find: &dyn Fn(&str, &str) -> Option<TabView>, step: &dyn Fn(&str) -> Option<Option<Step>>) -> Result<Run, String> {
    let (mut on, mut tab) = (machine.to_owned(), calling);
    for _ in 0..WALK_MAX {
        match tab.connector.as_deref() {
            Some(id) if step(id).is_some() => return Ok(Run::Workflow(id.to_owned())),
            Some(id) if id.starts_with(EXTERNAL_CONNECTOR_PREFIX) => return Err("An external agent drives this session, so it reads no Convex data.".into()),
            Some(INTERACTIVE_CONNECTOR | DELEGATE_CONNECTOR | CAPTAIN_CONNECTOR) if tab.spawned_by.is_none() => {
                return Err("This session runs in an Upstash Box that nothing in pinned started, so it cannot be tied to who opened it. It reads no Convex data.".into());
            }
            Some(INTERACTIVE_CONNECTOR | DELEGATE_CONNECTOR | CAPTAIN_CONNECTOR) | None => {}
            Some(id) => return Err(format!("This session runs under `{id}`, which is no workflow pinned knows, so it reads no Convex data.")),
        }
        let Some(parent) = tab.spawned_by.clone() else { return Ok(Run::ByHand) };
        on = tab.spawned_on.clone().unwrap_or(on);
        tab = find(&on, &parent).ok_or("The session that started this one is gone, so pinned cannot tell who it works for. It reads no Convex data.")?;
    }
    Err(format!("This session sits more than {WALK_MAX} sessions below the one that started the work, so pinned cannot tell who it works for."))
}

fn admit(scope: &Scope, ask: &Ask) -> Result<(), String> {
    let Scope::Tables(tables) = scope else { return Ok(()) };
    let limit = format!("This run's Read Convex data step limits it to the tables {}", tables.iter().cloned().collect::<Vec<_>>().join(", "));
    match ask {
        Ask::Offer | Ask::Tables { table: None } => Ok(()),
        Ask::Tables { table: Some(table) } | Ask::Data { table, .. } if tables.contains(table) => Ok(()),
        Ask::Tables { table: Some(table) } | Ask::Data { table, .. } => Err(format!("{limit}, so {} cannot read {table}.", ask.tool())),
        Ask::Query { .. } | Ask::Run { .. } | Ask::Logs { .. } => Err(format!("{limit}, so {} is off.", ask.tool())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Role;
    use crate::convex::Kind;
    use serde_json::json;

    const SITE: &str = "github.com/acme/site";
    const SHOP: &str = "github.com/acme/shop";

    fn connection(id: &str, repo: &str) -> Connection {
        Connection { id: id.into(), kind: Kind::Dev, url: format!("https://{id}.convex.cloud"), team: "acme".into(), project: "site".into(), key_hint: "…a1b2".into(), repos: vec![repo.into()] }
    }

    fn tab(id: &str) -> TabView {
        serde_json::from_value(json!({ "id": id, "cwd": "/Users/f/site", "bypass": false, "running": true, "status": null, "title": null, "message": null, "resume_at": null, "connector": null, "opened": 1 })).unwrap()
    }

    fn with(id: &str, edit: impl FnOnce(&mut TabView)) -> TabView {
        let mut t = tab(id);
        edit(&mut t);
        t
    }

    fn child(id: &str, parent: &str) -> TabView {
        with(id, |t| t.spawned_by = Some(parent.into()))
    }

    fn workflow(id: &str, connector: &str) -> TabView {
        with(id, |t| t.connector = Some(connector.into()))
    }

    fn step(id: &str) -> Option<Option<Step>> {
        let step = |connection: &str, tables: Option<&[&str]>| Some(Some(Step { connection: connection.into(), tables: tables.map(|t| t.iter().map(|s| s.to_string()).collect()) }));
        match id {
            "wf-all" => step("quick-lemur-789", None),
            "wf-invoices" => step("quick-lemur-789", Some(&["invoices", "customers"])),
            "wf-other" => step("happy-otter-123", None),
            "wf-none" => Some(None),
            _ => None,
        }
    }

    #[test]
    fn the_calling_tab_and_its_spawn_chain_decide_what_a_session_reads() {
        let m1 = vec![
            tab("thread"),
            with("worktree", |t| t.cwd = "/Users/f/site/.claude/worktrees/pinned-1".into()),
            with("captain", |t| t.role = Some(Role::Orchestrator)),
            with("worker", |t| {
                t.role = Some(Role::Worker);
                t.spawned_by = Some("captain".into());
            }),
            workflow("wf-all-tab", "wf-all"),
            workflow("wf-invoices-tab", "wf-invoices"),
            workflow("wf-none-tab", "wf-none"),
            workflow("wf-other-tab", "wf-other"),
            with("run-now-tab", |t| {
                t.connector = Some("wf-none".into());
                t.spawned_by = Some("thread".into());
            }),
            with("run-now-limited", |t| {
                t.connector = Some("wf-invoices".into());
                t.spawned_by = Some("thread".into());
            }),
            child("limited-child", "wf-invoices-tab"),
            child("grandchild", "limited-child"),
            child("orphan", "tab-gone"),
            workflow("box", INTERACTIVE_CONNECTOR),
            workflow("captain-box", CAPTAIN_CONNECTOR),
            workflow("external", "mcp:4f1c"),
            workflow("unknown", "agent-session-7"),
            with("global", |t| t.orchestrator_global = true),
            with("taste", |t| t.taste_chat = true),
        ];
        let m2 = [with("delegate-box", |t| {
            t.connector = Some(DELEGATE_CONNECTOR.into());
            t.spawned_by = Some("thread".into());
            t.spawned_on = Some("m1".into());
        })];
        let find = |machine: &str, id: &str| match machine {
            "m1" => m1.iter().find(|t| t.id == id).cloned(),
            "m2" => m2.iter().find(|t| t.id == id).cloned(),
            _ => None,
        };
        let connections = [connection("quick-lemur-789", SITE), connection("happy-otter-123", SHOP)];
        let everything = Ok(Scope::Everything);
        let limited: Result<Scope, String> = Ok(Scope::Tables(["customers".to_string(), "invoices".to_string()].into()));
        let no_step = Err("This workflow has no Read Convex data step.".to_string());
        let tables = Ask::Tables { table: None };
        type Row<'a> = (&'a str, &'a str, Option<&'a str>, Ask, Result<Scope, String>);
        let rows: Vec<Row> = vec![
            ("m1", "thread", Some(SITE), tables.clone(), everything.clone()),
            ("m1", "thread", Some("github.com/acme/blog"), tables.clone(), Err("Convex is not connected for `github.com/acme/blog`. The user connects it under Settings, Connectors, Convex.".into())),
            ("m1", "thread", None, Ask::Offer, Err("This folder has no git origin, so it cannot be connected to Convex yet.".into())),
            ("m1", "worktree", Some(SITE), Ask::Query { code: "return 1;".into() }, everything.clone()),
            ("m1", "captain", Some(SITE), Ask::Logs { function: None, errors: false, limit: None }, everything.clone()),
            ("m1", "worker", Some(SITE), Ask::Run { function: "invoices:byNumber".into(), args: json!({}) }, everything.clone()),
            ("m2", "delegate-box", Some(SITE), tables.clone(), everything.clone()),
            ("m1", "wf-all-tab", Some(SITE), Ask::Query { code: "return 1;".into() }, everything.clone()),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Offer, limited.clone()),
            ("m1", "wf-invoices-tab", Some(SITE), tables.clone(), limited.clone()),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Tables { table: Some("invoices".into()) }, limited.clone()),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Tables { table: Some("payouts".into()) }, Err("This run's Read Convex data step limits it to the tables customers, invoices, so convex_tables cannot read payouts.".into())),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Data { table: "customers".into(), limit: None, cursor: None, oldest_first: false }, limited.clone()),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Data { table: "payouts".into(), limit: None, cursor: None, oldest_first: false }, Err("This run's Read Convex data step limits it to the tables customers, invoices, so convex_data cannot read payouts.".into())),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Query { code: "return 1;".into() }, Err("This run's Read Convex data step limits it to the tables customers, invoices, so convex_query is off.".into())),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Run { function: "invoices:byNumber".into(), args: json!({}) }, Err("This run's Read Convex data step limits it to the tables customers, invoices, so convex_run is off.".into())),
            ("m1", "wf-invoices-tab", Some(SITE), Ask::Logs { function: None, errors: true, limit: None }, Err("This run's Read Convex data step limits it to the tables customers, invoices, so convex_logs is off.".into())),
            ("m1", "wf-none-tab", Some(SITE), tables.clone(), no_step.clone()),
            ("m1", "wf-other-tab", Some(SITE), tables.clone(), Err("This workflow's Read Convex data step reads `happy-otter-123`, but this project is connected to `quick-lemur-789`.".into())),
            ("m1", "run-now-tab", Some(SITE), tables.clone(), no_step.clone()),
            ("m1", "run-now-limited", Some(SITE), Ask::Query { code: "return 1;".into() }, Err("This run's Read Convex data step limits it to the tables customers, invoices, so convex_query is off.".into())),
            ("m1", "limited-child", Some(SITE), tables.clone(), limited.clone()),
            ("m1", "grandchild", Some(SITE), Ask::Logs { function: None, errors: false, limit: None }, Err("This run's Read Convex data step limits it to the tables customers, invoices, so convex_logs is off.".into())),
            ("m1", "orphan", Some(SITE), tables.clone(), Err("The session that started this one is gone, so pinned cannot tell who it works for. It reads no Convex data.".into())),
            ("m1", "box", Some(SITE), tables.clone(), Err("This session runs in an Upstash Box that nothing in pinned started, so it cannot be tied to who opened it. It reads no Convex data.".into())),
            ("m1", "captain-box", Some(SITE), tables.clone(), Err("This session runs in an Upstash Box that nothing in pinned started, so it cannot be tied to who opened it. It reads no Convex data.".into())),
            ("m1", "external", Some(SITE), tables.clone(), Err("An external agent drives this session, so it reads no Convex data.".into())),
            ("m1", "unknown", Some(SITE), tables.clone(), Err("This session runs under `agent-session-7`, which is no workflow pinned knows, so it reads no Convex data.".into())),
            ("m1", "global", Some(SITE), tables.clone(), Err("The global orchestrator and the taste chat do no project work, so they read no Convex data.".into())),
            ("m1", "taste", Some(SITE), tables.clone(), Err("The global orchestrator and the taste chat do no project work, so they read no Convex data.".into())),
            ("m1", "tab-9", Some(SITE), Ask::Offer, Err(UNKNOWN_TAB.into())),
            ("m2", "thread", Some(SITE), Ask::Offer, Err(UNKNOWN_TAB.into())),
        ];
        for (machine, id, repo, ask, expected) in rows {
            let decided = decide(machine, id, repo, &find, &connections, &step, &ask);
            assert_eq!(decided.as_ref().map(|g| &g.scope), expected.as_ref(), "{machine} {id} {ask:?}");
            if let Ok(grant) = decided {
                assert_eq!(grant.connection.id, "quick-lemur-789", "{id}");
            }
        }
    }

    #[test]
    fn a_spawn_chain_longer_than_the_walk_is_refused() {
        let chain: Vec<TabView> = (0..9).map(|n| if n == 0 { tab("t0") } else { child(&format!("t{n}"), &format!("t{}", n - 1)) }).collect();
        let find = |_: &str, id: &str| chain.iter().find(|t| t.id == id).cloned();
        let connections = [connection("quick-lemur-789", SITE)];
        let decided = |id: &str| decide("m1", id, Some(SITE), &find, &connections, &step, &Ask::Offer).map(|g| g.scope);
        assert_eq!(decided("t7"), Ok(Scope::Everything));
        assert_eq!(decided("t8"), Err("This session sits more than 8 sessions below the one that started the work, so pinned cannot tell who it works for.".into()));
    }

    #[test]
    fn a_grant_offers_its_deployment_and_its_table_limit() {
        let grant = Grant { connection: connection("quick-lemur-789", SITE), scope: Scope::Tables(["invoices".to_string()].into()) };
        assert_eq!(grant.offer(), Offer { deployment: "quick-lemur-789".into(), kind: Kind::Dev, tables: Some(vec!["invoices".into()]) });
        assert_eq!(Grant { scope: Scope::Everything, ..grant }.offer().tables, None);
    }
}
