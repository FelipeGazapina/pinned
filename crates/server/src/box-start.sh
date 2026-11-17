# Starts this box's pinned node. The box runs it on every container start, so it also redoes
# what a restart may have reset outside /workspace/home.
DIR=/workspace/home/.pinned
[ -f "$DIR/account.json" ] && [ -x "$DIR/pinned-server" ] || exit 0
# Never `pkill -f`: inside `box exec` it matches its own `sh -c` line. The pid file outlives a
# container restart, where its number may belong to another process, so the command line must match.
[ -f "$DIR/node.pid" ] && grep -qs pinned-server "/proc/$(cat "$DIR/node.pid")/cmdline" && exit 0
grep -qs "$DIR/bin" "$HOME/.profile" || echo "export PATH=\"$DIR/bin:\$PATH\"" >>"$HOME/.profile"
export PATH="$DIR/bin:$PATH"
# Interactive claude stops at the theme picker, the folder-trust dialog and, for a workflow that
# bypasses permissions, the bypass warning, unless these are set. Trusting a parent covers plain
# folders but not a git repository, so the run's clone gets its own entry.
node -e '
const fs = require("fs"), os = require("os"), file = os.homedir() + "/.claude.json";
const read = (f) => { try { return JSON.parse(fs.readFileSync(f, "utf8")); } catch { return {}; } };
const settings = os.homedir() + "/.claude/settings.json";
fs.mkdirSync(os.homedir() + "/.claude", { recursive: true });
fs.writeFileSync(settings, JSON.stringify({ ...read(settings), skipDangerousModePermissionPrompt: true }, null, 2));
const c = read(file);
c.hasCompletedOnboarding = true;
c.theme = c.theme || "dark";
c.projects = c.projects || {};
const clones = {{clones}}.map((name) => os.homedir() + "/pinned/" + name);
for (const p of ["/workspace/home", "/home/boxuser", os.homedir(), ...clones]) c.projects[p] = { ...c.projects[p], hasTrustDialogAccepted: true };
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
