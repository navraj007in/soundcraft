#!/usr/bin/env bash
# Keep this fork in step with upstream (storytold/soundcraft). Never pushes, never rewrites history.
#
#   scripts/upstream-sync.sh status   what upstream has that we lack, and what we carry
#   scripts/upstream-sync.sh start    update the mirror, branch from main, merge upstream, list conflicts
#
# See docs/upstream-sync.md for the routine.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

UPSTREAM_REMOTE="${UPSTREAM_REMOTE:-upstream}"
MIRROR="${MIRROR_BRANCH:-upstream-main}"
BASE="${BASE_BRANCH:-main}"

die() { echo "error: $*" >&2; exit 1; }
git remote get-url "$UPSTREAM_REMOTE" >/dev/null 2>&1 || die "no '$UPSTREAM_REMOTE' remote (git remote add $UPSTREAM_REMOTE https://github.com/storytold/soundcraft.git)"

refresh_mirror() {
  git fetch -q "$UPSTREAM_REMOTE"
  # Fast-forward only: the mirror is never committed to, so this cannot lose work.
  if git show-ref -q --verify "refs/heads/$MIRROR"; then
    if [ "$(git branch --show-current)" = "$MIRROR" ]; then die "'$MIRROR' is checked out; switch to another branch"; fi
    git merge-base --is-ancestor "$MIRROR" "$UPSTREAM_REMOTE/main" || die "'$MIRROR' has commits of its own; it must only mirror $UPSTREAM_REMOTE/main"
    git branch -f "$MIRROR" "$UPSTREAM_REMOTE/main"
  else
    git branch --track "$MIRROR" "$UPSTREAM_REMOTE/main"
  fi
}

status() {
  refresh_mirror
  local behind ahead
  behind=$(git rev-list --count "$BASE..$MIRROR")
  ahead=$(git rev-list --count "$MIRROR..$BASE")
  echo "upstream ($MIRROR): $(git log -1 --format='%h %cd %s' --date=short "$MIRROR")"
  echo "$BASE is $behind commit(s) behind upstream and carries $ahead of its own."
  if [ "$behind" -gt 0 ]; then
    echo; echo "New upstream commits (newest first, merges hidden):"
    git log --no-merges --format='  %h %cd %an | %s' --date=short "$BASE..$MIRROR" | head -40
    echo; echo "Files upstream changed that we also changed (likely conflicts):"
    comm -12 <(git diff --name-only "$(git merge-base "$BASE" "$MIRROR")" "$MIRROR" | sort) \
             <(git diff --name-only "$(git merge-base "$BASE" "$MIRROR")" "$BASE" | sort) | sed 's/^/  /'
  fi
}

start() {
  git diff --quiet && git diff --cached --quiet || die "the working tree has uncommitted changes; commit or stash them first (a modified Cargo.lock from a build: git checkout -- Cargo.lock)"
  refresh_mirror
  local behind; behind=$(git rev-list --count "$BASE..$MIRROR")
  [ "$behind" -gt 0 ] || { echo "Already up to date with upstream."; return 0; }
  local branch="sync/upstream-$(date +%Y%m%d)"
  git rev-parse --verify -q "$branch" >/dev/null && die "$branch already exists"
  git checkout -q -b "$branch" "$BASE"
  echo "On $branch; merging $behind upstream commit(s)..."
  if git merge --no-edit "$MIRROR"; then
    echo "Merged cleanly. Run: cargo xtask ci"
  else
    echo; echo "Conflicts (rerere replays any resolution it has seen before):"
    git diff --name-only --diff-filter=U | sed 's/^/  /'
    echo; echo "Resolve, 'git add', 'git commit', then run: cargo xtask ci"
  fi
}

case "${1:-status}" in
  status) status ;;
  start) start ;;
  *) die "usage: $0 [status|start]" ;;
esac
