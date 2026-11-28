#!/bin/sh
# Claude Code hook (PreToolUse on Bash), declared in .claude/settings.json.
# Before any `git commit` run by Claude: no AI attribution, validate command green (--changed: the
# quick checks and the tests of the groups the change touches; the firmware, the slow tests and
# QEMU run before the merge, scaled to the change, and every night: D87), and a commit on main only
# with the user's yes (trivial fixes).
# Exit 2 blocks the command and shows the message to Claude, which must fix the cause.

cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

input=$(cat)

# The branch and the files checked below are those of the directory where the command runs (an
# agent's worktree, or the main checkout): two iterations can run side by side, each validated on
# its own files. `--changed` builds no firmware, so a worktree needs no ESP-IDF tools of its own.
dir=$(printf '%s' "$input" | tr '\n' ' ' | sed -nE 's/.*"cwd" *: *"([^"]*)".*/\1/p')
[ -d "$dir" ] || dir=.

# Look at the command only, not at its description: the hook receives the whole tool call as JSON.
cmd=$(printf '%s' "$input" | tr '\n' ' ' | sed -nE 's/.*"command" *: *"(([^"\\]|\\.)*)".*/\1/p')
case "$cmd" in
  *"git commit"*|*"git -C "*" commit"*) ;;
  *) exit 0 ;;
esac

case "$cmd" in
  *Co-Authored-By*|*"Generated with"*)
    echo "Blocked: no AI attribution line in commits (see CLAUDE.md)." >&2
    exit 2 ;;
esac

if ! out=$(cd "$dir" && sh scripts/validate.sh --changed 2>&1); then
  echo "Blocked: the validate command fails. Fix the cause, never the check. Last lines:" >&2
  echo "$out" | tail -n 30 >&2
  exit 2
fi

gitdir=$(git rev-parse --git-dir 2>/dev/null) || exit 0

# The git hooks must be on, even after a fresh clone: git does not keep that setting.
[ -d .githooks ] && [ "$(git config core.hooksPath)" != ".githooks" ] && git config core.hooksPath .githooks

# On main, a commit is allowed only for a trivial fix: the user is asked to confirm.
branch=$(git -C "$dir" branch --show-current 2>/dev/null)
if [ "$branch" = "main" ] && git rev-parse --verify -q HEAD >/dev/null && [ ! -f "$gitdir/SQUASH_MSG" ]; then
  cat <<'EOF'
{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": "Commit on main: only for a trivial fix (typo, one-line doc). Anything else goes on a branch (feat/, fix/, docs/, chore/), see CLAUDE.md."}}
EOF
fi
exit 0
