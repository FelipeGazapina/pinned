set -eu
# Turns a fresh Upstash Box into a node of the hub that created it. The hub wrote account.json,
# box.env and start.sh into the directory first, so nothing secret is on this script's command line.
DIR=/workspace/home/.pinned
mkdir -p "$DIR/bin"
chmod 600 "$DIR/box.env" "$DIR/account.json"
curl -fsSL '{{hub}}/download/pinned-server-aarch64-unknown-linux-musl' -o "$DIR/pinned-server.part"
chmod 755 "$DIR/pinned-server.part"
mv -f "$DIR/pinned-server.part" "$DIR/pinned-server"
if ! command -v gh >/dev/null 2>&1 && [ ! -x "$DIR/bin/gh" ]; then
  curl -fsSL https://github.com/cli/cli/releases/download/v2.62.0/gh_2.62.0_linux_arm64.tar.gz | tar -xzf - -C /tmp
  mv -f /tmp/gh_2.62.0_linux_arm64/bin/gh "$DIR/bin/gh"
fi
# The hub boots a box only while its node is offline: a node left running holds a token the hub replaced.
if [ -f "$DIR/node.pid" ]; then
  kill "$(cat "$DIR/node.pid")" 2>/dev/null || true
  rm -f "$DIR/node.pid"
fi
sh "$DIR/start.sh"
