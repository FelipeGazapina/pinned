# Hub memory runs

Local hub, local Postgres 15 on 127.0.0.1:5499, fake accounts only, aarch64 box.
glibc = `cargo build --release`; musl = `cargo build --release --target aarch64-unknown-linux-musl`
with `CC_aarch64_unknown_linux_musl=musl-gcc` (static, musl mallocng). Runs 3-4 carry the
counting allocator (`heap_live_mb`), census every 30 s (`PINNED_CENSUS_SECS=30`).

```
cargo build --release -p pinned-server --bin pinned-server --example hub_load
PINNED_CENSUS_SECS=30 crates/server/examples/hub_load.sh target/release/pinned-server 8791 glibc1 /tmp/out <flags>
```

| run | load (3 accounts) | glibc VmRSS MB over time | musl VmRSS MB over time |
|---|---|---|---|
| 1, 9 min | 1 machine + 1 client each, 40 tabs every 1 s, thread event every 2 s, client reads whole 1–8 MB page (+2 MB picture 1 in 4), 40 KB/s output, client reconnect ~60 s; ~22 GB/h to clients | 196 → 279 → 293 → 256 → 275 (plateau) | 69 → 78 → 77 → 64 → 61 (flat) |
| 2, 9 min | run 1 + 2 machines each, 8 MB sync push/list/pull every 20 s, 1 slow client (256 KB/s) each; fake machine frame limit 16 MB made machines reconnect every 20 s | 416–658, no trend | 112–325, sawtooth, no clear trend |
| 3, 8 min | run 2 with sync pulls working (22 MB replies) | 429 → 836 → 712 → 781; heap_live 359–540 | 197 → 489 → 287 → 363; heap_live 336–476 |
| 3h, 8 min | run 3 glibc under heaptrack | 440 → 848; peak heap 742 MB; live at exit 372 MB | |
| 4, 7.5 min | connect churn: client life ~8 s, machine life ~15 s (~520 connects), tabs every 0.5 s, pages ≤ 4 MB; 39 GB allocated | 186–240 flat; heap_live 45–63 | 46–66 flat; heap_live 38–57 |

musl VmRSS stays near heap_live (it returns freed memory); glibc sits 150–400 MB above it and
plateaus. Neither build grows under any of these loads.

heaptrack, live at the end of run 3h (372 MB): tungstenite per-socket buffers, each the size of
the largest frame that socket carried: machine read buffers (FrameCodec::read_frame BytesMut)
95 MB, machine write buffers (FrameCodec::buffer_frame Vec) 134 MB, client write buffers 63 MB;
tokio-postgres connection buffers ~41 MB (sync_push binds); in-flight serde_json Values of
relayed replies ~27 MB.

## Census diagnostics and bounded store writes (fix/hub-memory-diagnostics)

Same box, same Postgres. `hub_load` now prints the hub's CPU seconds (`cpu_s`, utime + stime from
`/proc/<pid>/stat`) and takes `--store-every <secs>`: the first client of each account saves the
store (repo insight on, then off) at that pace and prints the last value it saved.

Counting allocator overhead, release glibc builds of the same tree with and without
`#[global_allocator]`, same load, CPU seconds of the hub at the end:

| load | without counter | with counter |
|---|---|---|
| 3 accounts, 40 tabs every 1 s, 1–8 MB pages, sync every 20 s (4 MB), 240 s | 4.8 | 5.1 |
| connect churn (client life 8 s, machine life 15 s, tabs every 0.5 s, pages ≤ 4 MB), 180 s | 2.1 | 2.2 |

About 5 % of the hub's own CPU time, 0.1–0.3 CPU seconds over these runs; one run each, so
the noise is not measured.

Stalled store writes: with `--store-every 0.2` (5 saves/s per account) and every account row
locked by `select … for update` for 25–30 s, each account's census line shows `store_pending=1`
with constant `store_pending_bytes` for the whole stall. With the lock held across the hub's
SIGTERM the hub logged `3 accounts had saves waiting at stop; 3 written in 9227 ms` once the lock
went, and every row held the last value its client saved.
