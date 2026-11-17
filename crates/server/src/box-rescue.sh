# Saves a worker's unpushed work before the hub deletes its box, in the mode the hub names.
# `commits`, the mode of every repository the hub has not confirmed private: it pushes only the
# commits the worker made and never adds, commits or touches the index or a work tree. The answer
# counts the files with uncommitted changes, which stay unsaved, and never names them.
# `full`, only for a repository the hub confirmed private, and only while the clone's origin pushes
# to that one repository alone (else it runs as `commits`): in the clone and in each worktree of it
# with uncommitted changes, it unstages everything, then adds its tracked changes and the new files
# git does not ignore, except any whose path has a part named like a secret (environment files, keys
# and certificates, credential and token stores, package and cloud logins, infrastructure state,
# the `.claude` folder; a name that only holds such a word inside a longer word is kept) and any
# nested repository, and commits them. The answer counts the files it left out and never names them.
# Then, in both modes, it pushes each worktree's HEAD and every local branch that has commits on no
# remote branch, each to the new branch wip/rescue/<task>, or <task>-2 and on while that name is
# taken. Before the first push it fetches origin, pruning, and before each push it reads what that
# push would add over origin's branches, commits and messages, and when that holds a value of a
# secret-named variable of its own, of the box's `box.env` or of the node's environment, or a token's
# shape, or adds or changes a file whose name may hold a secret, it pushes nothing of that branch,
# undoes its own rescue commit there and answers only that it refused. It pushes with the
# credentials the clone keeps, never forces, never skips a hook, and never pushes a submodule or a
# tag the clone's settings would add.
# The hub reads only the `pinned-rescue:` lines: git's own output, which may name paths, stays here.
export GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 LC_ALL=C
task='{{task}}'
mode='{{mode}}'
# The github.com repository, `owner/name`, whose visibility the hub checked, and the folder of the
# box's `box.env` and the node's pid file.
github='{{github}}'
pinned='{{dir}}'
[ "$mode" = full ] || mode=commits
answer() { echo "pinned-rescue: $1"; }
cd "$HOME/pinned/{{repo}}" 2>/dev/null && git rev-parse --git-dir >/dev/null 2>&1 || { answer done; exit 0; }
top=$(pwd)
trees=$(git worktree list --porcelain 2>/dev/null | sed -n 's/^worktree //p')
common=$(cd "$(git rev-parse --git-common-dir 2>/dev/null)" 2>/dev/null && pwd) || { answer 'failed scan'; answer done; exit 0; }
values=$(mktemp "$common/pinned-rescue.XXXXXX" 2>/dev/null) || { answer 'failed scan'; answer done; exit 0; }
diff=$(mktemp "$common/pinned-rescue.XXXXXX" 2>/dev/null) || { rm -f "$values"; answer 'failed scan'; answer done; exit 0; }
trap 'rm -f "$values" "$diff"' EXIT
# Each line of 12 characters or more of a value of a variable named like a secret: of this process's
# own, read inside awk, so no value is ever on a command line.
awk 'BEGIN { for (k in ENVIRON) if (toupper(k) ~ /TOKEN|KEY|SECRET|PASSWORD|PASS|AUTH|CREDENTIAL|COOKIE/) { n = split(ENVIRON[k], part, "\n"); for (i = 1; i <= n; i++) if (length(part[i]) >= 12) print part[i] } }' >"$values" 2>/dev/null || { answer 'failed scan'; answer done; exit 0; }
# The same of `NAME=value` lines on its input, with no quotes around and with them, as box.env and
# the node's environment hold them: read as text, never run. A line with no name goes on the value
# before it.
assigned() {
  awk 'function keep(v) { if (length(v) >= 12) print v }
    { line = $0; sub(/\r$/, "", line) }
    line ~ /^(export[ \t]+)?[A-Za-z_][A-Za-z0-9_]*=/ { sub(/^export[ \t]+/, "", line); i = index(line, "="); named = toupper(substr(line, 1, i - 1)) ~ /TOKEN|KEY|SECRET|PASSWORD|PASS|AUTH|CREDENTIAL|COOKIE/; line = substr(line, i + 1) }
    named { keep(line); q = sprintf("%c", 39); v = line; gsub(q "\\\\" q q, q, v); gsub("^[" q "\"]|[" q "\"]$", "", v); keep(v) }' >>"$values" 2>/dev/null
}
# box.env, which start.sh loads into the node's environment but no exec of the hub runs with.
[ -f "$pinned/box.env" ] && { assigned <"$pinned/box.env" || { answer 'failed scan'; answer done; exit 0; }; }
# The node's environment, when its pid file names a running pinned-server this user may read.
node=$(cat "$pinned/node.pid" 2>/dev/null)
case "$node" in '' | *[!0-9]*) node='' ;; esac
[ -n "$node" ] && grep -qs pinned-server "/proc/$node/cmdline" && [ -r "/proc/$node/environ" ] && tr '\000' '\n' <"/proc/$node/environ" | assigned
# `full` only while origin pushes to the checked repository alone: one push URL, on github.com,
# over https or ssh, with no other user. git expands insteadOf and pushInsteadOf in what it reads.
if [ "$mode" = full ]; then
  url=$(git remote get-url --push --all origin 2>/dev/null | tr 'A-Z' 'a-z')
  checked=$(printf '%s' "$github" | tr 'A-Z' 'a-z')
  case "$url" in
    "https://github.com/$checked" | "https://github.com/$checked.git" | "git@github.com:$checked" | "git@github.com:$checked.git" | \
      "ssh://git@github.com/$checked" | "ssh://git@github.com/$checked.git") [ -n "$checked" ] || mode=commits ;;
    *) mode=commits ;;
  esac
fi
n=1
uncommitted=0
# Empty until origin was fetched, then 0 when that worked.
fetched=''
# The rescue commits this run made, as `<commit> <files it left out>` lines.
made=''
# The commits handled already, and the branches whose push was refused.
sent=' '
refused=' '
# How many NUL-ended names it reads.
names() { tr -cd '\000' | wc -c | tr -d ' '; }
# The nested repositories of this tree, one per line: new folders that hold a `.git`, and submodules.
nested() {
  git ls-files -z --others --exclude-standard 2>/dev/null | tr '\000\n' '\n\000' | sed -n 's|/$||p'
  git ls-files -z --stage 2>/dev/null | tr '\000\n' '\n\000' | sed -n 's|^160000 [0-9a-f]* [0-9]*	||p'
}
# How many files of this tree hold changes no commit has: tracked ones and new ones git does not
# ignore. A nested repository is not a file.
changed() {
  tracked=$(git diff -z --name-only --no-renames --ignore-submodules=dirty HEAD 2>/dev/null | names)
  new=$(git ls-files -z --others --exclude-standard 2>/dev/null | tr '\000\n' '\n\000' | grep -vc '/$')
  echo $((tracked + new))
}
# Runs git with its arguments and the pathspec of the whole tree but the names that may hold a secret
# and the nested repositories.
safe() {
  set -- "$@" -- .
  for name in '.env*' '*.env' 'env.local' '*.pem' '*.key' '*.p12' '*.pfx' '*.jks' '*.keystore' 'id_rsa*' 'id_*' \
    '.npmrc' '.netrc' '.pgpass' '.htpasswd' '.dockercfg' '*.tfstate' '*.tfvars' '*kubeconfig*' '.mcp.json' '.claude' \
    '*secret' '*secrets' '*secret[._-]*' '*secrets[._-]*' '*credential' '*credentials' '*credential[._-]*' '*credentials[._-]*' \
    'token' 'tokens' '.token' '.tokens'; do
    set -- "$@" ":(exclude,icase,glob)**/$name" ":(exclude,icase,glob)**/$name/**"
  done
  # A token word only as a file's name, so a folder like `design-tokens` keeps its files.
  for name in '*token' '*tokens' '*token[._-]*' '*tokens[._-]*'; do
    set -- "$@" ":(exclude,icase,glob)**/$name"
  done
  while IFS= read -r dir; do
    [ -n "$dir" ] && set -- "$@" ":(exclude,literal)$dir"
  done <<EOF
$(nested)
EOF
  git "$@"
}
# In `full` mode, commits what tree $1 holds uncommitted.
commit() {
  cd "$1" 2>/dev/null || return 0
  [ -n "$(git status --porcelain --ignore-submodules=dirty 2>/dev/null)" ] || return 0
  git reset -q >/dev/null 2>&1 || { answer 'failed add'; return 0; }
  safe add -A >/dev/null 2>&1 || { answer 'failed add'; return 0; }
  if git diff --cached --raw --no-renames 2>/dev/null | grep -q '^:[0-7]* 160000 '; then
    git reset -q >/dev/null 2>&1
    answer 'failed add'
    return 0
  fi
  git diff --cached --quiet 2>/dev/null && return 0
  # A hook may stage more than this added: a commit whose tree is not this one is undone, never sent.
  staged=$(git write-tree 2>/dev/null) || { answer 'failed add'; return 0; }
  git commit -q -m "rescue: unpushed work of task $task" >/dev/null 2>&1 || { answer 'failed commit'; return 0; }
  if [ "$(git rev-parse -q --verify 'HEAD^{tree}' 2>/dev/null)" != "$staged" ]; then
    sent="$sent$(git rev-parse -q --verify HEAD 2>/dev/null) "
    git reset -q HEAD~1 >/dev/null 2>&1
    answer 'failed commit'
    return 0
  fi
  made="$made$(git rev-parse HEAD) $(changed)
"
}
# Whether what commit $1 adds over origin's branches, commits and messages, may hold a secret: a
# value of a secret-named variable, a token's shape, or a file added or changed whose name may hold
# a secret, by the names `safe` leaves out. A failure to read it is a yes.
leaks() {
  git log -p --cc --text --no-color --no-ext-diff --no-textconv "$1" --not --remotes=origin >"$diff" 2>/dev/null || return 0
  grep -q -F -f "$values" "$diff"
  [ $? -eq 1 ] || return 0
  grep -q -E -e 'sk-[A-Za-z0-9_-]{20}' -e 'sk_live_[A-Za-z0-9]{10}' -e 'gh[pousr]_[A-Za-z0-9_]{30}' -e 'github_pat_' \
    -e 'xox[abposr]-' -e 'xapp-[A-Za-z0-9-]{10}' -e 'glpat-[A-Za-z0-9_-]{20}' -e 'AIza[A-Za-z0-9_-]{30}' -e 'box_[A-Za-z0-9_]{16}' \
    -e 'A(KI|SI)A[0-9A-Z]{16}' -e 'eyJ[A-Za-z0-9_-]*\.eyJ[A-Za-z0-9_-]*\.' -e '://[^/:@[:space:]]+:[^/@[:space:]]+@' \
    -e '-----BEGIN[A-Z0-9 ]*PRIVATE KEY' "$diff"
  [ $? -eq 1 ] || return 0
  git log -z --format= --name-only --no-renames --diff-filter=d --cc "$1" --not --remotes=origin >"$diff" 2>/dev/null || return 0
  tr '\000\n' '\n\000' <"$diff" | tr 'A-Z' 'a-z' | grep -q -E \
    -e '(^|/)(\.env[^/]*|[^/]*\.env|env\.local|[^/]*\.(pem|key|p12|pfx|jks|keystore|tfstate|tfvars)|id_[^/]*)(/|$)' \
    -e '(^|/)(\.npmrc|\.netrc|\.pgpass|\.htpasswd|\.dockercfg|[^/]*kubeconfig[^/]*|\.mcp\.json|\.claude|\.?tokens?)(/|$)' \
    -e '(^|/)([^/]*secrets?|[^/]*credentials?)([._-][^/]*)?(/|$)' -e '(^|/)[^/]*tokens?([._-][^/]*)?$'
  [ $? -eq 1 ] || return 0
  return 1
}
# Undoes rescue commit $1 in the tree whose HEAD it is, keeping its files.
undo() {
  printf '%s' "$made" | grep -q "^$1 " || return 0
  while IFS= read -r tree; do
    [ -n "$tree" ] && [ "$(git -C "$tree" rev-parse HEAD 2>/dev/null)" = "$1" ] && git -C "$tree" reset -q --soft HEAD~1 >/dev/null 2>&1
  done <<EOF
$trees
EOF
}
# Pushes commit $2, the tip of branch $1 (empty for a detached HEAD), to the task's next free rescue
# branch, once, when it has commits on no remote branch and nothing in them may hold a secret.
send() {
  case "$sent" in *" $2 "*) return 0 ;; esac
  case "$refused" in *" $1 "*) return 0 ;; esac
  sent="$sent$2 "
  [ -n "$(git rev-list -n 1 "$2" --not --remotes=origin 2>/dev/null)" ] || return 0
  # Once, before the first scan: origin's branches as they are now, so a stale one hides nothing.
  [ -n "$fetched" ] || { git fetch -q --prune --no-recurse-submodules origin >/dev/null 2>&1; fetched=$?; }
  [ "$fetched" -eq 0 ] || { answer 'failed remote'; return 0; }
  [ -n "$(git rev-list -n 1 "$2" --not --remotes=origin 2>/dev/null)" ] || return 0
  leaks "$2"
  hit=$?
  : >"$diff"
  if [ "$hit" -eq 0 ]; then
    [ -n "$1" ] && refused="$refused$1 "
    undo "$2"
    answer 'refused secret'
    return 0
  fi
  while :; do
    ref="wip/rescue/$task"
    [ "$n" -gt 1 ] && ref="$ref-$n"
    git ls-remote --exit-code --heads origin "refs/heads/$ref" >/dev/null 2>&1
    case $? in
      0) n=$((n + 1)) ;;
      2) break ;;
      *) answer 'failed remote'; return 0 ;;
    esac
    [ "$n" -le 20 ] || { answer 'failed name'; return 0; }
  done
  git -c push.recurseSubmodules=no -c push.followTags=false push -q origin "$2:refs/heads/$ref" >/dev/null 2>&1 || { answer 'failed push'; return 0; }
  n=$((n + 1))
  left=$(printf '%s' "$made" | sed -n "s/^$2 //p")
  if [ "${left:-0}" -gt 0 ]; then answer "saved $ref (left out $left files that may hold secrets)"; else answer "saved $ref"; fi
}
while IFS= read -r tree; do
  [ -n "$tree" ] || continue
  if [ "$mode" = full ]; then
    commit "$tree"
  elif cd "$tree" 2>/dev/null; then
    uncommitted=$((uncommitted + $(changed)))
  fi
done <<EOF
$trees
EOF
while IFS= read -r tree; do
  [ -n "$tree" ] && cd "$tree" 2>/dev/null || continue
  tip=$(git rev-parse -q --verify HEAD 2>/dev/null) || continue
  send "$(git symbolic-ref -q HEAD 2>/dev/null)" "$tip"
done <<EOF
$trees
EOF
cd "$top" 2>/dev/null
while read -r tip branch; do
  [ -n "$tip" ] && send "$branch" "$tip"
done <<EOF
$(git for-each-ref --format='%(objectname) %(refname)' refs/heads/ 2>/dev/null)
EOF
[ "$uncommitted" -gt 0 ] && answer "uncommitted $uncommitted"
answer done
