# Hub memory runs (wip/hub-memory-r2, on 0.18.3)

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
