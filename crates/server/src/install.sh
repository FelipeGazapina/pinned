#!/bin/sh
# Usage: curl -fsSL {{hub}}/install.sh | sh -s -- <pairing code>
set -eu

HUB='{{hub}}'
CODE="${1:-}"
if [ -z "$CODE" ]; then
    echo "usage: curl -fsSL $HUB/install.sh | sh -s -- <pairing code>" >&2
    exit 1
fi

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64 | Linux-amd64) TARGET=x86_64-unknown-linux-musl ;;
    Linux-aarch64 | Linux-arm64) TARGET=aarch64-unknown-linux-musl ;;
    Darwin-arm64) TARGET=aarch64-apple-darwin ;;
    Darwin-x86_64) TARGET=x86_64-apple-darwin ;;
    *)
        echo "pinned: there is no pinned-server for $(uname -s) $(uname -m)" >&2
        exit 1
        ;;
esac

if [ "$(id -u)" = 0 ]; then
    BIN=/usr/local/bin
else
    BIN="$HOME/.local/bin"
fi
mkdir -p "$BIN"
PART="$BIN/.pinned-server.$$"
trap 'rm -f "$PART"' EXIT
if ! curl -fsSL "$HUB/download/pinned-server-$TARGET" -o "$PART"; then
    echo "pinned: $HUB has no pinned-server build for $TARGET" >&2
    exit 1
fi
chmod 755 "$PART"
mv "$PART" "$BIN/pinned-server"

CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/pinned"
NAME="$("$BIN/pinned-server" pair "$HUB" "$CODE" --config-dir "$CONFIG")"
echo "pinned: paired this machine as $NAME"

if [ "$(uname -s)" = Darwin ]; then
    echo "pinned: start the node with: $BIN/pinned-server node"
    exit 0
fi
if ! command -v systemctl >/dev/null 2>&1; then
    echo "pinned: no systemd here; start the node with: $BIN/pinned-server node"
    exit 0
fi

# The node runs claude as this user, with the PATH this shell has, so it finds what a terminal finds.
unit() {
    cat <<UNIT
[Unit]
Description=pinned node
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$BIN/pinned-server node --config-dir $CONFIG
Environment=HOME=$HOME
Environment=PATH=$PATH
Environment=SHELL=${SHELL:-/bin/sh}
Restart=always
RestartSec=5

[Install]
WantedBy=$1
UNIT
}

if [ "$(id -u)" = 0 ]; then
    unit multi-user.target >/etc/systemd/system/pinned-node.service
    systemctl daemon-reload
    systemctl enable pinned-node.service
    systemctl restart pinned-node.service
    LOGS="journalctl -u pinned-node -f"
else
    mkdir -p "$HOME/.config/systemd/user"
    unit default.target >"$HOME/.config/systemd/user/pinned-node.service"
    systemctl --user daemon-reload
    systemctl --user enable pinned-node.service
    systemctl --user restart pinned-node.service
    # Keeps the user's services running after they log out, and starts them at boot.
    loginctl enable-linger "$(id -un)" 2>/dev/null || echo "pinned: run 'sudo loginctl enable-linger $(id -un)' so the node survives logout and reboot"
    LOGS="journalctl --user -u pinned-node -f"
fi
echo "pinned: the node is running; follow it with: $LOGS"
