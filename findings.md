# Diagnostic 2 findings (box drop during crates/server/tests/hub.rs), 2026-11-13 UTC

## Result: NOT REPRODUCED
- Stop order arrived before any build or test. Run A with the inherited env never ran (neither did Run B).
- Under the Captain's update I ran the suspect test once, sanitized, at 04:01:36Z: `env -u PINNED_PROJECT -u PINNED_ROLE -u PINNED_TAB -u PINNED_SOCK -u PINNED_ACCOUNT -u PINNED_TUTORIAL_HELPER HOME=<fresh mktemp -d> CARGO_HOME=/home/boxuser/.cargo RUSTUP_HOME=/home/boxuser/.rustup`. It passed in 5.7 s (runA.log). The link held.
- Then the whole suite ran the same way, one test at a time in file order (run 1's order), 04:01:45Z–04:23:38Z: 146 tests, 140 ok, 5 FAILED (#11, #37, #83, #117, #125; #83 failed in run 1 too), 1 TIMEOUT (#107, see below).
- The node (pid 288) stayed `Ssl` and kept its single ESTAB connection to the production hub (153.52.255.120:443) in all 315 five-second samples. No other process ever connected to the hub IP. No process ever showed STAT T. oom_kill 0.
- Disk: `/` lowest free 15040 MB (20 GB overlay; the build took it from 19.8 to 15.1 GB free). `/workspace` stayed at 19374 MB free. Disk never came near 1 GB, so the disk lead is untested at its limit on this box.
- Memory: memory.max 8 GiB; current ~6.5 GB, of which anon ~0.69 GB and file ~5.7 GB; memory.events `max 660` (the limit was hit 660 times; page cache was reclaimed), oom 0, oom_kill 0.
- Watched files: during the tests only the node's own tasks.json (every 60 s) and ~/.claude/.last-cleanup (04:03:24Z, Claude Code's own housekeeping: guess) changed. ~/.pinned does not exist on this box. The node uses --config-dir /workspace/home/.pinned.

## Where the hub URL and machine credentials come from (STOP order step 2)
- TestHub: `serve("127.0.0.1:0")`, `url = format!("http://{}", running.addr)` (hub.rs:83, 107). VERIFIED.
- account::signup / signup_invited / login / register_machine / pair: they take `hub` as an argument and POST to `{hub}/api/...` (core/src/account.rs:226-257). The tests pass `hub.url`. VERIFIED.
- agent_of (hub.rs:5647): Client::connect(url) -> ws://127.0.0.1:<port>/ws/client. register_machine(url) gives the machine token. `Credentials { hub: url, .. }` is saved to the test's own node_dir under /tmp. Node::start -> link::spawn(creds) -> ws_url(creds.hub) (link.rs:408, 592). VERIFIED.
- No built-in production URL is in the non-test code paths. The only production string is a unit-test label (server/src/boxes.rs:1936). VERIFIED by grep.
- Database: `PINNED_TEST_POSTGRES` or postgres://pinned@127.0.0.1:5499 (hub.rs:31). VERIFIED. (Stripping every PINNED_* would also strip this one.)
- The box's credentials live in /workspace/home/.pinned/account.json, reached only through --config-dir. Nothing in the hub test path reads $HOME/.pinned or PINNED_SOCK: PINNED_SOCK/PINNED_TAB are read only by the `hook`/`mcp` subcommands (core/src/hooks.rs, reminders.rs). Pty tabs get SOCK_ENV/TAB_ENV set to the test node's own sock (machine.rs:224-225). VERIFIED.
- Still read from the real $HOME in tests (read-only for this test): Accounts home (~/.claude, ~/.codex), `clones = $HOME/pinned`, default cwd, link.rs:275 sync::State::load(.., $HOME/.claude). Guarded under test_binary(): .claude.json, the global orchestrator folder, plugin_shares, skills places. VERIFIED (machine.rs:845-852, 3861-3876, 3888).
- Inherited-env children: hub.rs:5995 (`pinned-server login`) and hub.rs:6008 (`pinned-server node`, HOME=dir) inherit every PINNED_* from the shell. What the node binary does with an inherited PINNED_SOCK/TAB is not checked: GUESS that it ignores them.
- The Captain's prod-side check (no test traffic reached production in any drop) agrees with all of this.

## Signals (Captain update 2)
1. pty.rs kill_tree(root) (pty.rs:172): root = `child.process_id()` of a child that is reaped only in the same Drop, after the kill (pty.rs:154-164). So it cannot be 0, 1 or a reused pid. The walk follows only ppid==member links down from root. Orphans reparent to pid 1, which is never a descendant. It never STOPs the node or the logger. VERIFIED by reading.
2. delegate/drivers.rs kill(pid) = kill(-(pid as i32), SIGKILL) (drivers.rs:751). pid 0 -> kill(0) = the caller's own process group. pid 1 -> kill(-1) = EVERY process the user may signal (node, claude, logger: everything at once). No guard. Pids come from Child::id() of a just-spawned child or from remember_pid(agent.id()), never 0 or 1 in practice. targets_under guards `pid > 1` for descendants but not for the root (drivers.rs:897). delegate/mod.rs Leftover::drop (mod.rs:1366) does kill(-pid) on a pid parsed from a file; a file holding "1" would kill(-1), but it is a pinned-core unit-test helper, not used by hub.rs. No reachable 0/1 found: VERIFIED for the call sites I read. GUESS that no other path supplies 0/1.
3. hub.rs kill sites: only hub.rs:6024 `kill -KILL -{pid}` (pid = node.id() of a process_group(0) child: right target) and hub.rs:7671 `kill -0`. There is no killpg/pkill/SIGTERM/STOP elsewhere in hub.rs. TestHub::drop drops its own databases and removes its own /tmp/pinned-server-hub-* dir. temp() removes only /tmp/pinned-server-<name>-<pid>. VERIFIED.
   NEW, VERIFIED: on this box `/bin/kill` is procps-ng 4.0.2. `kill -KILL -<pgid>` exits 0 and kills NOTHING (checked on a dummy `setsid sleep`). `kill -s KILL -- -<pgid>` works. So #107 a_node_keeps_only_its_machine_token_and_ends_a_login_an_older_pinned_kept hangs forever in node.wait() (hub.rs:6026), leaving an orphaned test binary (ppid 1) and a running `pinned-server node` on the local hub. It does not touch the box's node. In run 1 the drop happened at #105, before #107.

## Mechanism
Not found. The sanitized suite cannot reproduce the drop on this box. What the evidence fits (GUESS): the box process tree died all at once from the box side, because the node's socket and the logger stopped together and the Captain saw a box-side close. That is consistent with an external box kill (platform: disk or memory accounting) or a user-wide kill(-1). This run found no kill(-1) path in the hub tests, and the run never came near the disk limit (15 GB free at worst). The page cache sits at the 8 GiB memory.max (memory.events max=660); whether the platform reacts to that is not tested here.

## macOS (a developer's Mac, inside a pinned session)
- Hub URL and credentials: same code, so tests hit only their local TestHub. VERIFIED.
- kill_tree and the group kills target only the test's own descendants or groups. The Mac app's node is not ppid 1 and not in cargo's process group, so kill(0) would hit only cargo's group. GUESS from reading. A kill(-1) would take down every process of the user on the Mac too, ppid notwithstanding.
- macOS /bin/kill is BSD kill, which (GUESS) accepts `kill -KILL -<pgid>`, so #107 would not hang there.
- Real-$HOME reads (~/.claude, ~/.codex, ~/pinned) happen on the Mac. They are read-only in the paths I read (GUESS beyond those). The inherited PINNED_SOCK points at the Mac app's node socket, and only the spawned `pinned-server login/node` children (hub.rs:5995, 6008) inherit it.

## Guard proposal (no code written)
Harness side:
- One `test_env()` at TestHub/Node start that panics unless every hub URL in use parses to a loopback host (127.0.0.0/8, ::1, localhost). Check it in Credentials::save and link::spawn under test_binary() as well.
- Strip PINNED_* from the test process at start (keep PINNED_TEST_POSTGRES, or rename it so it survives), and env_clear plus an allow-list for every child the tests spawn (hub.rs:5995, 6008).
- Set HOME (and USERPROFILE) to a per-test temp dir for the whole binary (a ctor or a build-script env), so the unguarded home() uses (Accounts, clones, link.rs:275) never see the real home.
- Replace `kill -KILL -{pid}` with libc::kill(-(pid as i32), SIGKILL) or `kill -s KILL -- -{pid}`, and bound node.wait() with a timeout.
Node side:
- drivers::kill and every kill(-pid): refuse pid <= 1 (and pid > i32::MAX), and refuse a pgid equal to our own getpgrp(). The same for kill_tree roots and Leftover.
- Spawn each claude/agent in its own session (setsid) so a group signal from a tab never reaches the node, and the node never shares a group with its children.
