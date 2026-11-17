# 0.18.7 2b run progress (update_live on int/0.18.7, HEAD c2e2af38)
13:39:23 | step1 checkout int/0.18.7 detached c2e2af38 | exit=0 | wall=0s | err=none | disk: 30G free, 3% used | mem: avail=210438MB
13:39:47 | step2 env.sh written, opening check ok (HOME=work/home, no PINNED/CLAUDE/GH_TOKEN names), cargo 1.100.0 | exit=0 | wall=1s | err=none | disk: 29G free, 5% used | mem: avail=209773MB
13:40:05 | step3 postgres 15.20 initdb+pinned db, port 5499 started | exit=0 | wall=25s | err=none | disk: 29G free, 5% used | mem: avail=210939MB
13:42:25 | step3 build update_live --no-run done | exit=0 | wall=0s | err=none | disk: 25G free, 19% used | mem: avail=209498MB
13:43:38 | step3 build pinned-server bin | exit=0 | wall=68s | err=none | disk: 22G free, 27% used | mem: avail=209734MB
13:47:57 | test1 a_binary_that_exits_before_it_is_ready_is_replaced_by_the_old_one HANG killed 240s; node pgid 7123 killed; node config dir removed by cleanup glob before log read | exit=137 | wall=240s | err=has been running for over 60 seconds | disk: 22G free, 28% used | mem: avail=208887MB
13:52:06 | test2 a_binary_that_hangs_before_it_is_ready_is_replaced_by_the_old_one HANG killed 240s; node pgid 8127 killed; stderr read before cleanup | exit=137 | wall=240s | err=has been running for over 60 seconds | disk: 22G free, 28% used | mem: avail=209462MB
