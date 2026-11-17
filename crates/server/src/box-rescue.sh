# Saves a worker's unpushed work before the hub deletes its box, in the mode the hub names.
# `commits`, the mode of every repository the hub has not confirmed private: it pushes only the
# commits the worker made and never adds, commits or touches the index or a work tree. The answer
# counts the files with uncommitted changes, which stay unsaved, and never names them.
# `full`, only for a repository the hub confirmed private: in the clone and in each worktree of it
# with uncommitted changes, it unstages everything, then adds its tracked changes and the new files
# git does not ignore, except any whose path has a part named like a secret (environment files, keys
# and certificates, credential and token stores, package and cloud logins, infrastructure state,
# the `.claude` folder; a name that only holds such a word inside a longer word is kept) and any
# nested repository, and commits them. The answer counts the files it left out and never names them.
# Then, in both modes, it pushes each worktree's HEAD and every local branch that has commits on no
# remote branch, each to the new branch wip/rescue/<task>, or <task>-2 and on while that name is
# taken. Before each push it reads what that push would add over the remote, commits and messages,
# and when that holds a value of one of its own secret-named environment variables or a token's
# shape it pushes nothing of that branch, undoes its own rescue commit there and answers only that
# it refused. It pushes with the credentials the clone keeps, never forces and never skips a hook.
# The hub reads only the `pinned-rescue:` lines: git's own output, which may name paths, stays here.
export GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 LC_ALL=C
task='{{task}}'
mode='{{mode}}'
[ "$mode" = full ] || mode=commits
answer() { echo "pinned-rescue: $1"; }
cd "$HOME/pinned/{{repo}}" 2>/dev/null && git rev-parse --git-dir >/dev/null 2>&1 || { answer done; exit 0; }
top=$(pwd)
trees=$(git worktree list --porcelain 2>/dev/null | sed -n 's/^worktree //p')
common=$(cd "$(git rev-parse --git-common-dir 2>/dev/null)" 2>/dev/null && pwd) || { answer 'failed scan'; answer done; exit 0; }
values=$(mktemp "$common/pinned-rescue.XXXXXX" 2>/dev/null) || { answer 'failed scan'; answer done; exit 0; }
diff=$(mktemp "$common/pinned-rescue.XXXXXX" 2>/dev/null) || { rm -f "$values"; answer 'failed scan'; answer done; exit 0; }
trap 'rm -f "$values" "$diff"' EXIT
# Each line of 12 characters or more of a value of this process's variables named like a secret, read
# inside awk, so no value is ever on a command line.
awk 'BEGIN { for (k in ENVIRON) if (toupper(k) ~ /TOKEN|KEY|SECRET|PASSWORD|PASS|AUTH|CREDENTIAL|COOKIE/) { n = split(ENVIRON[k], part, "\n"); for (i = 1; i <= n; i++) if (length(part[i]) >= 12) print part[i] } }' >"$values" 2>/dev/null || { answer 'failed scan'; answer done; exit 0; }
n=1
uncommitted=0
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
  git commit -q -m "rescue: unpushed work of task $task" >/dev/null 2>&1 || { answer 'failed commit'; return 0; }
  made="$made$(git rev-parse HEAD) $(changed)
"
}
# Whether what commit $1 adds over every remote branch, commits and messages, may hold a secret: a
# value of this process's secret-named variables, or a token's shape. A failure to read it is a yes.
leaks() {
  git log -p --cc --text --no-color --no-ext-diff --no-textconv "$1" --not --remotes >"$diff" 2>/dev/null || return 0
  grep -q -F -f "$values" "$diff"
  [ $? -eq 1 ] || return 0
  grep -q -E -e 'sk-[A-Za-z0-9_]{20}' -e 'sk-(ant|proj)-[A-Za-z0-9_-]{20}' -e 'gh[pousr]_[A-Za-z0-9_]{30}' -e 'github_pat_' \
    -e 'xox[abposr]-' -e 'box_[A-Za-z0-9_]{16}' -e 'AKIA[0-9A-Z]{16}' -e '-----BEGIN[A-Z0-9 ]*PRIVATE KEY' "$diff"
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
  [ -n "$(git rev-list -n 1 "$2" --not --remotes 2>/dev/null)" ] || return 0
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
  git push -q origin "$2:refs/heads/$ref" >/dev/null 2>&1 || { answer 'failed push'; return 0; }
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
