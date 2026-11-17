# Saves a worker's unpushed work before the hub deletes its box. In the task's clone and in each
# worktree of it that holds uncommitted changes or commits on no remote branch, it commits all of
# it and pushes HEAD to the new branch wip/rescue/<task>, or <task>-2 and on while that name is
# taken. It pushes with the credentials the clone keeps, never forces and never skips a hook. The
# hub reads only the `pinned-rescue:` lines: git's own output, which may name paths, stays here.
export GIT_TERMINAL_PROMPT=0
task='{{task}}'
answer() { echo "pinned-rescue: $1"; }
cd "$HOME/pinned/{{repo}}" 2>/dev/null && git rev-parse --git-dir >/dev/null 2>&1 || { answer done; exit 0; }
trees=$(git worktree list --porcelain 2>/dev/null | sed -n 's/^worktree //p')
n=1
save() {
  cd "$1" 2>/dev/null || return 0
  dirty=$(git status --porcelain --ignore-submodules=dirty 2>/dev/null)
  ahead=$(git rev-list -n 1 HEAD --not --remotes 2>/dev/null)
  [ -n "$dirty$ahead" ] || return 0
  if [ -n "$dirty" ]; then
    git add -A >/dev/null 2>&1 || { answer 'failed add'; return 0; }
    git commit -q -m "rescue: unpushed work of task $task" >/dev/null 2>&1 || { answer 'failed commit'; return 0; }
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
  git push -q origin "HEAD:refs/heads/$ref" >/dev/null 2>&1 || { answer 'failed push'; return 0; }
  n=$((n + 1))
  answer "saved $ref"
}
while IFS= read -r tree; do
  [ -n "$tree" ] && save "$tree"
done <<EOF
$trees
EOF
answer done
