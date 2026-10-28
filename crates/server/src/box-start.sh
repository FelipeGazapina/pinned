# Starts this box's pinned node. The box runs it on every container start, so it also redoes
# what a restart may have reset outside /workspace/home.
DIR=/workspace/home/.pinned
[ -f "$DIR/account.json" ] && [ -x "$DIR/pinned-server" ] || exit 0
# Never `pkill -f`: inside `box exec` it matches its own `sh -c` line.
[ -f "$DIR/node.pid" ] && kill -0 "$(cat "$DIR/node.pid")" 2>/dev/null && exit 0
grep -qs "$DIR/bin" "$HOME/.profile" || echo "export PATH=\"$DIR/bin:\$PATH\"" >>"$HOME/.profile"
export PATH="$DIR/bin:$PATH"
# Interactive claude stops at the theme picker and the folder-trust dialog unless these are set.
node -e '
const fs = require("fs"), os = require("os"), file = os.homedir() + "/.claude.json";
let c = {};
try { c = JSON.parse(fs.readFileSync(file, "utf8")); } catch {}
c.hasCompletedOnboarding = true;
c.theme = c.theme || "dark";
c.projects = c.projects || {};
for (const p of ["/workspace/home", "/home/boxuser", os.homedir()]) c.projects[p] = { ...c.projects[p], hasTrustDialogAccepted: true };
fs.writeFileSync(file, JSON.stringify(c, null, 2));
'
set -a
. "$DIR/box.env"
set +a
git config --global user.name '{{login}}'
git config --global user.email '{{login}}@users.noreply.github.com'
gh auth setup-git >/dev/null 2>&1 || true
setsid nohup "$DIR/pinned-server" node --config-dir "$DIR" >/tmp/pinned-node.log 2>&1 </dev/null &
echo $! >"$DIR/node.pid"
