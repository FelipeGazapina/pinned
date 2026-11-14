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

## Round 3: production shape

Production (hub 0.18.4, x86_64 musl, census every 5 min): rss 331 → 851 MB in 21 min while
heap_live stays 89–96 MB; ~1 GB allocated per 5 min; machine replies ~47/min, ~255 KB average,
620 KB max; machine calls in up to 3.7 MB; ~6 sync pulls/min of ~250 KB.

This round: hub built as the Dockerfile builds it, `cargo zigbuild --release --target
aarch64-unknown-linux-musl` (zig 0.17.1, cargo-zigbuild 0.24.1, static). The box is aarch64, so
x86_64 musl was not run. `hub_load` gained `--list-kb` (production-shaped sync: projects seeded
so each `sync_list` reply is about that many KB; every machine lists every project each round,
pulls a `--pull-kb` file on two rounds in three and pushes up to `--push-max-mb` on one in three).

```
PINNED_CENSUS_SECS=60 crates/server/examples/hub_load.sh <hub> 8791 before1 <out> \
  --accounts 1 --machines 3 --clients 2 --tabs 10 --tabs-every 5 --thread-every 30 \
  --page-min-mb 0.02 --page-max-mb 0.2 --output-rate 2000 --client-life 600 --machine-life 100000 \
  --sync-every 20 --list-kb 620,400,150,70,35 --pull-kb 260 --push-max-mb 2.8 --secs 1200 --every 60
```

Traffic per 1-min census: sent_machine_reply_n 53–54, sent_machine_reply_bytes 10–15.5 MB,
sent_machine_reply_max 722 KB (production: ~47/min, ~12 MB/min, max 620 KB).

**Reproduced** within 8 minutes, musl system allocator (VmRSS at each 1-min census):

| min | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|
| rss_mb | 60 | 91 | 117 | 143 | 164 | 188 | 209 | 232 |
| heap_live_mb | 12 | 19 | 22 | 26 | 26 | 28 | 24 | 26 |

~25 MB/min = ~1.5 GB/h, the production slope, while live heap stays at 12–28 MB. smaps_rollup:
Anonymous ≈ Rss (226 MB of 238 MB at minute 8), so it is heap, not file pages.

C code: the only C libraries linked are ring (no malloc) and bundled SQLite, which only the
node's `memory.rs` opens, never the hub. The same binary with only the Rust allocator under the
counter swapped (C code still on musl malloc) stays flat: mimalloc 76 → 89 → 83 MB, jemalloc
46 → 45 → 49 MB over the same first minutes. So the growth is musl's malloc keeping memory the
Rust heap freed, not C allocations the counter cannot see.

Why round 1–4 did not reproduce: those loads moved mostly 1–22 MB buffers, which musl serves with
mmap and unmaps on free. This load frees ~30 000 small allocations per `sync_list` reply (the
`serde_json::Value` tree of ~3 300 entries) about once a second, which is where mallocng keeps
memory. The full 20-minute tables follow below.
