# Shared by scripts/web-sweep/stack.sh and scripts/teams-dev/dev.sh. Source it: it only defines
# functions, and it expects the caller to run with `set -eu`.

# wait_for "<command>" "<what>": runs the command every 0.2s, up to 20s, and exits the caller on a timeout.
wait_for() {
  i=0
  until eval "$1" >/dev/null 2>&1; do
    i=$((i + 1))
    [ $i -gt 100 ] && { echo "timed out waiting for $2" >&2; exit 1; }
    sleep 0.2
  done
}

# kill_pid_files <dir>: signals the process each <dir>/*.pid names, then removes the file.
kill_pid_files() {
  for pid in "$1"/*.pid; do
    [ -f "$pid" ] && kill "$(cat "$pid")" 2>/dev/null || true
    rm -f "$pid"
  done
}

# pg_start <dir> <port> <db>: a fresh cluster in <dir> (trust auth, user pinned, TCP on 127.0.0.1
# only) with an empty database <db>. initdb refuses to run as root.
pg_start() {
  export LC_ALL=C
  initdb -U pinned --auth=trust -D "$1" >/dev/null
  pg_ctl -D "$1" -l "$1.log" -o "-p $2 -c listen_addresses=127.0.0.1 -c unix_socket_directories=" start >/dev/null
  wait_for "psql -h 127.0.0.1 -p $2 -U pinned -d postgres -c 'select 1'" postgres
  psql -h 127.0.0.1 -p "$2" -U pinned -d postgres -qc "create database $3"
}

# pg_stop <dir>: stops the cluster in <dir> when there is one.
pg_stop() {
  [ -d "$1" ] && pg_ctl -D "$1" -m fast stop >/dev/null 2>&1 || true
}
