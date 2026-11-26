//! The Memory settings page's report: what the app, its webview helpers, each thread's and
//! terminal's processes and pinned's other children hold, read once per ask. Figures are
//! `footprint` on macOS, as Activity Monitor shows it, and resident memory on Linux.
#![cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]

use pinned_core::machine::{Machine, ProcessRoot, RootKind};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Marks when the app started, for the report's uptime. `main` calls it first.
pub fn started() {
    STARTED.get_or_init(Instant::now);
}

/// One row of `ps -A -o pid=,ppid=,etime=,rss=,comm=`.
#[derive(Clone, Debug, PartialEq)]
struct Proc {
    pid: u32,
    ppid: u32,
    /// Seconds since the process started.
    age: Option<u64>,
    /// KiB; `ps` prints `-` when it has none.
    rss: Option<u64>,
    /// The last path component of `comm`.
    name: String,
}

#[derive(Serialize, Debug, Default, Clone, Copy, PartialEq)]
pub struct Sum {
    pub bytes: u64,
    /// Processes still running when their figure was read, unreadable ones included.
    pub processes: u32,
    /// Processes whose figure could not be read, which add nothing to `bytes`.
    pub unreadable: u32,
}

impl Sum {
    fn add(&mut self, other: Sum) {
        self.bytes += other.bytes;
        self.processes += other.processes;
        self.unreadable += other.unreadable;
    }
}

#[derive(Serialize, Debug)]
pub struct Row {
    pub label: String,
    pub kind: &'static str,
    pub agent: Option<&'static str>,
    pub state: &'static str,
    #[serde(flatten)]
    pub sum: Sum,
}

#[derive(Serialize, Debug)]
pub struct Group {
    pub name: &'static str,
    #[serde(flatten)]
    pub sum: Sum,
    /// One per thread, delegated task and terminal, in `Threads and terminals` only.
    pub rows: Vec<Row>,
}

#[derive(Serialize, Debug)]
pub struct Report {
    pub version: &'static str,
    pub os: &'static str,
    pub measure: &'static str,
    pub uptime_secs: u64,
    pub groups: Vec<Group>,
    #[serde(flatten)]
    pub total: Sum,
}

pub const APP: &str = "App";
pub const WEBVIEW: &str = "Webview helpers";
pub const THREADS: &str = "Threads and terminals";
pub const OTHER: &str = "Other";

#[cfg(target_os = "macos")]
const MEASURE: &str = "physical footprint";
#[cfg(not(target_os = "macos"))]
const MEASURE: &str = "resident memory";

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn read(machine: &Machine) -> Result<Report, String> {
    let (table, ps) = table()?;
    let plain: String = table.iter().map(|p| format!("{} {} {}\n", p.pid, p.ppid, p.name)).collect();
    Ok(measure(&table, &machine.process_roots(&plain), ps))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn read(_machine: &Machine) -> Result<Report, String> {
    Err("The memory report is not available on Windows yet.".into())
}

/// Every process, from one `ps` call, and that call's own pid.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn table() -> Result<(Vec<Proc>, u32), String> {
    let child = pinned_core::process::command("ps")
        .args(["-A", "-o", "pid=,ppid=,etime=,rss=,comm="])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run ps: {e}"))?;
    let ps = child.id();
    let out = child.wait_with_output().map_err(|e| format!("could not run ps: {e}"))?;
    if !out.status.success() {
        return Err(format!("ps failed: {}", out.status));
    }
    Ok((parse(&String::from_utf8_lossy(&out.stdout)), ps))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn measure(table: &[Proc], roots: &[ProcessRoot], ps: u32) -> Report {
    let app = std::process::id();
    let groups = group(table, &figures(table), roots, app, &webview(table, app), ps);
    let mut total = Sum::default();
    for g in &groups {
        total.add(g.sum);
    }
    Report {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        measure: MEASURE,
        uptime_secs: STARTED.get().map_or(0, |at| at.elapsed().as_secs()),
        groups,
        total,
    }
}

fn parse(text: &str) -> Vec<Proc> {
    text.lines()
        .filter_map(|line| {
            let mut rest = line.trim_start();
            let mut field = || {
                let (word, after) = rest.split_once(char::is_whitespace)?;
                rest = after.trim_start();
                Some(word)
            };
            let (pid, ppid, etime, rss) = (field()?, field()?, field()?, field()?);
            let comm = rest.trim_end();
            let name = Path::new(comm).file_name().map_or_else(|| comm.to_owned(), |n| n.to_string_lossy().into_owned());
            Some(Proc { pid: pid.parse().ok()?, ppid: ppid.parse().ok()?, age: seconds(etime), rss: rss.parse().ok(), name })
        })
        .collect()
}

/// `ps`'s `[[dd-]hh:]mm:ss`.
fn seconds(etime: &str) -> Option<u64> {
    let (days, clock) = etime.split_once('-').map_or((Some(0), etime), |(d, c)| (d.parse().ok(), c));
    let mut secs = 0;
    for part in clock.split(':') {
        secs = secs * 60 + part.parse::<u64>().ok()?;
    }
    Some(days? * 86400 + secs)
}

fn children(table: &[Proc]) -> HashMap<u32, Vec<u32>> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in table.iter().filter(|p| p.pid != p.ppid) {
        children.entry(p.ppid).or_default().push(p.pid);
    }
    children
}

/// `root` and every process under it that `claimed` does not hold yet, which it then holds.
fn take_tree(root: u32, children: &HashMap<u32, Vec<u32>>, claimed: &mut HashSet<u32>) -> Vec<u32> {
    let mut taken = vec![];
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if !claimed.insert(pid) {
            continue;
        }
        taken.push(pid);
        stack.extend(children.get(&pid).into_iter().flatten());
    }
    taken
}

/// Adds up `pids`. `figures` holds `None` for a figure that could not be read, and nothing for a
/// process that exited before it was read, which is left out.
fn sum(pids: &[u32], figures: &HashMap<u32, Option<u64>>) -> Sum {
    let mut sum = Sum::default();
    for pid in pids {
        match figures.get(pid) {
            Some(Some(bytes)) => {
                sum.bytes += bytes;
                sum.processes += 1;
            }
            Some(None) => {
                sum.processes += 1;
                sum.unreadable += 1;
            }
            None => {}
        }
    }
    sum
}

/// Puts every process in at most one group: each root's tree in its row, threads before
/// delegated tasks before terminals, then the app, its webview helpers, and the rest of its
/// descendants in `Other`. `ps` is the call that read the table, which no group counts.
fn group(table: &[Proc], figures: &HashMap<u32, Option<u64>>, roots: &[ProcessRoot], app: u32, webview: &HashSet<u32>, ps: u32) -> Vec<Group> {
    let children = children(table);
    let listed: HashSet<u32> = table.iter().map(|p| p.pid).collect();
    let mut claimed = HashSet::from([ps]);
    let mut ordered: Vec<&ProcessRoot> = roots.iter().filter(|r| listed.contains(&r.pid)).collect();
    ordered.sort_by_key(|r| r.kind);
    let mut rows = vec![];
    let mut threads = Sum::default();
    for root in ordered {
        let row = sum(&take_tree(root.pid, &children, &mut claimed), figures);
        threads.add(row);
        let kind = match root.kind {
            RootKind::Thread => "thread",
            RootKind::Delegated => "delegated",
            RootKind::Terminal => "terminal",
        };
        let state = if root.running { "running" } else { "idle" };
        rows.push(Row { label: root.label.clone(), kind, agent: root.agent, state, sum: row });
    }
    let helpers: Vec<u32> = table.iter().map(|p| p.pid).filter(|pid| *pid != app && webview.contains(pid) && claimed.insert(*pid)).collect();
    let other: Vec<u32> = take_tree(app, &children, &mut claimed).into_iter().filter(|pid| *pid != app).collect();
    vec![
        Group { name: APP, sum: sum(&[app], figures), rows: vec![] },
        Group { name: WEBVIEW, sum: sum(&helpers, figures), rows: vec![] },
        Group { name: THREADS, sum: threads, rows },
        Group { name: OTHER, sum: sum(&other, figures), rows: vec![] },
    ]
}

/// macOS starts WebKit's helpers outside the app's tree: they are the `com.apple.WebKit`
/// processes the app's responsible process answers for that started no earlier than the app,
/// which covers an app started from a terminal.
#[cfg(target_os = "macos")]
fn webview(table: &[Proc], app: u32) -> HashSet<u32> {
    extern "C" {
        fn responsibility_get_pid_responsible_for_pid(pid: libc::pid_t) -> libc::pid_t;
    }
    let responsible = |pid: u32| unsafe { responsibility_get_pid_responsible_for_pid(pid as libc::pid_t) };
    let Some(app_age) = table.iter().find(|p| p.pid == app).and_then(|p| p.age) else { return HashSet::new() };
    let owner = responsible(app);
    table.iter().filter(|p| p.name.contains("com.apple.WebKit") && p.age.is_some_and(|age| age <= app_age) && responsible(p.pid) == owner).map(|p| p.pid).collect()
}

#[cfg(not(target_os = "macos"))]
fn webview(table: &[Proc], app: u32) -> HashSet<u32> {
    linux_webview(table, app)
}

/// WebKitGTK's `WebKitWebProcess` and `WebKitNetworkProcess` are the app's descendants; each
/// counts with everything under it.
#[cfg(any(test, not(target_os = "macos")))]
fn linux_webview(table: &[Proc], app: u32) -> HashSet<u32> {
    let children = children(table);
    let mut under_app = HashSet::new();
    take_tree(app, &children, &mut under_app);
    let mut helpers = HashSet::new();
    for p in table.iter().filter(|p| p.pid != app && under_app.contains(&p.pid) && p.name.starts_with("WebKit")) {
        take_tree(p.pid, &children, &mut helpers);
    }
    helpers
}

/// Each process's `ri_phys_footprint`. A process that exited since the table was read has none.
#[cfg(target_os = "macos")]
fn figures(table: &[Proc]) -> HashMap<u32, Option<u64>> {
    let mut figures = HashMap::new();
    for p in table {
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        let read = unsafe { libc::proc_pid_rusage(p.pid as libc::c_int, libc::RUSAGE_INFO_V2, &mut info as *mut libc::rusage_info_v2 as *mut libc::rusage_info_t) };
        if read == 0 {
            figures.insert(p.pid, Some(info.ri_phys_footprint));
        } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            figures.insert(p.pid, None);
        }
    }
    figures
}

/// Each process's resident memory, from the table's `rss` column.
#[cfg(not(target_os = "macos"))]
fn figures(table: &[Proc]) -> HashMap<u32, Option<u64>> {
    table.iter().map(|p| (p.pid, p.rss.map(|kib| kib * 1024))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_PID: u32 = 100;
    const PS: u32 = 199;

    /// The app (100) runs a thread's claude (200) with an MCP server (201) under it and that
    /// server's child (202), a terminal (300) whose shell started `npm` (301), a WebKit process
    /// (110), a stray helper (120), and the report's own `ps` (199). 400 is another app's.
    const TABLE: &str = "    1     0      10-02:03:04   1000 /sbin/launchd
  100     1        01:00:00 200000 /Applications/pinned.app/Contents/MacOS/pinned
  110   100        59:59  90000 /usr/lib/webkit2gtk-4.1/WebKitWebProcess
  111   110        59:59   1000 /usr/lib/webkit2gtk-4.1/WebKitNetworkProcess
  120   100           10   5000 /usr/bin/caffeinate
  199   100           00      1 ps
  200   100        30:00 300000 /Users/me/.local/bin/claude
  201   200        29:00  50000 /usr/local/bin/node
  202   201        29:00  10000 /opt/homebrew/bin/Google Chrome Helper
  300   100        05:00   4000 -zsh
  301   300        04:00  70000 npm run dev
  400     1        00:01   1234 /Applications/Other.app/Contents/MacOS/Other
";

    fn table() -> Vec<Proc> {
        parse(TABLE)
    }

    fn figures(table: &[Proc]) -> HashMap<u32, Option<u64>> {
        table.iter().map(|p| (p.pid, p.rss.map(|kib| kib * 1024))).collect()
    }

    fn root(kind: RootKind, label: &str, pid: u32, running: bool) -> ProcessRoot {
        ProcessRoot { kind, label: label.into(), agent: (kind == RootKind::Thread).then_some("claude"), pid, running }
    }

    fn roots() -> Vec<ProcessRoot> {
        vec![root(RootKind::Terminal, "Terminal 1", 300, true), root(RootKind::Thread, "Fix the build", 200, false)]
    }

    fn named<'a>(groups: &'a [Group], name: &str) -> &'a Group {
        groups.iter().find(|g| g.name == name).unwrap()
    }

    fn row<'a>(groups: &'a [Group], label: &str) -> &'a Row {
        named(groups, THREADS).rows.iter().find(|r| r.label == label).unwrap()
    }

    fn grouped(table: &[Proc], figures: &HashMap<u32, Option<u64>>, roots: &[ProcessRoot]) -> Vec<Group> {
        group(table, figures, roots, APP_PID, &linux_webview(table, APP_PID), PS)
    }

    #[test]
    fn a_threads_tree_holds_its_grandchildren() {
        let table = table();
        let groups = grouped(&table, &figures(&table), &roots());
        let thread = row(&groups, "Fix the build");
        assert_eq!(thread.sum, Sum { bytes: (300000 + 50000 + 10000) * 1024, processes: 3, unreadable: 0 });
        assert_eq!((thread.kind, thread.agent, thread.state), ("thread", Some("claude"), "idle"));
        let terminal = row(&groups, "Terminal 1");
        assert_eq!(terminal.sum, Sum { bytes: (4000 + 70000) * 1024, processes: 2, unreadable: 0 });
        assert_eq!((terminal.kind, terminal.agent, terminal.state), ("terminal", None, "running"));
    }

    #[test]
    fn a_process_under_a_thread_and_a_terminal_counts_once_for_the_thread() {
        let table = table();
        // The terminal's root is the MCP server, already in the thread's tree.
        let roots = vec![root(RootKind::Terminal, "Terminal 2", 201, false), root(RootKind::Thread, "Fix the build", 200, true)];
        let groups = grouped(&table, &figures(&table), &roots);
        assert_eq!(row(&groups, "Fix the build").sum.processes, 3);
        assert_eq!(row(&groups, "Terminal 2").sum, Sum::default());
        let total: u32 = groups.iter().map(|g| g.sum.processes).sum();
        assert_eq!(total, 9, "every process of the app's but ps counts once");
    }

    #[test]
    fn the_reports_own_ps_is_left_out() {
        let table = table();
        let groups = grouped(&table, &figures(&table), &roots());
        assert_eq!(named(&groups, OTHER).sum, Sum { bytes: 5000 * 1024, processes: 1, unreadable: 0 });
        let total: u32 = groups.iter().map(|g| g.sum.processes).sum();
        assert_eq!(total, 9);
    }

    #[test]
    fn an_unreadable_figure_is_counted_apart_from_the_bytes() {
        let table = table();
        let mut figures = figures(&table);
        figures.insert(201, None);
        figures.remove(&202);
        let groups = grouped(&table, &figures, &roots());
        assert_eq!(row(&groups, "Fix the build").sum, Sum { bytes: 300000 * 1024, processes: 2, unreadable: 1 }, "202 exited and is dropped");
        assert_eq!(named(&groups, THREADS).sum.unreadable, 1);
    }

    #[test]
    fn linux_webkit_processes_are_webview_helpers() {
        let table = table();
        let groups = grouped(&table, &figures(&table), &roots());
        assert_eq!(named(&groups, WEBVIEW).sum, Sum { bytes: (90000 + 1000) * 1024, processes: 2, unreadable: 0 });
        assert_eq!(named(&groups, APP).sum, Sum { bytes: 200000 * 1024, processes: 1, unreadable: 0 });
    }

    #[test]
    fn an_unclaimed_descendant_is_other_and_another_apps_process_is_nowhere() {
        let table = table();
        let groups = grouped(&table, &figures(&table), &[]);
        let other = named(&groups, OTHER).sum;
        assert_eq!(other.processes, 6, "caffeinate, claude's tree and the terminal's");
        assert_eq!(other.bytes, (5000 + 300000 + 50000 + 10000 + 4000 + 70000) * 1024);
        assert!(named(&groups, THREADS).rows.is_empty());
    }

    #[test]
    fn names_are_basenames_and_ages_are_seconds() {
        let table = table();
        let names: Vec<&str> = table.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["launchd", "pinned", "WebKitWebProcess", "WebKitNetworkProcess", "caffeinate", "ps", "claude", "node", "Google Chrome Helper", "-zsh", "npm run dev", "Other"]);
        assert!(names.iter().all(|n| !n.contains('/')));
        assert_eq!(table[0].age, Some(10 * 86400 + 2 * 3600 + 3 * 60 + 4));
        assert_eq!(table[1].age, Some(3600));
        assert_eq!(parse("  5  1  01:02  -  x\n")[0].rss, None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_live_table_finds_this_process() {
        let (table, ps) = super::table().unwrap();
        assert!(table.iter().any(|p| p.pid == std::process::id()));
        let report = measure(&table, &[], ps);
        let app = named(&report.groups, APP).sum;
        assert_eq!((app.processes, app.unreadable), (1, 0));
        assert!(app.bytes > 0);
        assert!(report.total.bytes >= app.bytes);
    }
}
