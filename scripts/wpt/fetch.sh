#!/usr/bin/env bash
# Shallow-clone WPT upstream and sparse-checkout only the paths needed
# by raikiri-wpt / raikiri VRT tests. Idempotent: re-running updates to
# the pinned SHA.
#
# Worktree-aware (raikiri-spike-dz8t): target/ is per-worktree working
# state (gitignored; cargo/git don't share it across `git worktree`
# checkouts), so each fresh worktree-<taskid> would otherwise need its own
# shallow clone of WPT. To avoid that, the actual checkout always lives
# under the *main* worktree's target/wpt, found via
# `git rev-parse --git-common-dir` (the shared .git dir, stable regardless
# of which worktree invokes this script). When run from a linked worktree,
# this script also symlinks that worktree's own target/wpt to the shared
# checkout, so env!("CARGO_MANIFEST_DIR")/../../target/wpt
# (crates/raikiri/tests/hello_world_vrt.rs) resolves to it transparently —
# no test-side changes needed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve REPO_ROOT from the caller's shell cwd, with a cross-tree
# mismatch guard — see scripts/lib/repo_root.sh for the rationale. This
# matters here specifically because REPO_ROOT below feeds LOCAL_WPT_DIR: a
# tree mismatch would previously have symlinked the *wrong* worktree's
# target/wpt without any indication something was off.
# shellcheck source=../lib/repo_root.sh
source "$SCRIPT_DIR/../lib/repo_root.sh"

SHA_FILE="$SCRIPT_DIR/pinned_sha.txt"
SUBSET_FILE="$SCRIPT_DIR/subset.txt"
REMOTE_URL="${WPT_REMOTE_URL:-https://github.com/web-platform-tests/wpt.git}"

# git-common-dir is the main worktree's real .git directory even when this
# script runs from a linked worktree; its parent is the main worktree root.
GIT_COMMON_DIR="$(cd "$REPO_ROOT" && git rev-parse --git-common-dir)"
case "$GIT_COMMON_DIR" in
  /*) : ;;
  *) GIT_COMMON_DIR="$REPO_ROOT/$GIT_COMMON_DIR" ;;
esac
GIT_COMMON_DIR="$(cd "$(dirname "$GIT_COMMON_DIR")" && pwd)/$(basename "$GIT_COMMON_DIR")"
MAIN_WORKTREE_ROOT="$(dirname "$GIT_COMMON_DIR")"

WPT_DIR="$MAIN_WORKTREE_ROOT/target/wpt"
LOCAL_WPT_DIR="$REPO_ROOT/target/wpt"

SHA="$(awk '!/^#/ && NF { print; exit }' "$SHA_FILE" | tr -d '[:space:]')"
if [ -z "$SHA" ]; then
  echo "error: no SHA in $SHA_FILE" >&2
  exit 1
fi

if [ ! -d "$WPT_DIR/.git" ]; then
  mkdir -p "$WPT_DIR"
  git -C "$WPT_DIR" init -q
  git -C "$WPT_DIR" config core.sparseCheckout true
  git -C "$WPT_DIR" config extensions.partialClone origin
fi

# Keep the remote URL in sync on every run so WPT_REMOTE_URL overrides
# (mirrors, CI caches) take effect even when target/wpt already exists.
git -C "$WPT_DIR" remote set-url origin "$REMOTE_URL" 2>/dev/null \
  || git -C "$WPT_DIR" remote add origin "$REMOTE_URL"

# Write sparse-checkout patterns (strip comments and blanks)
mkdir -p "$WPT_DIR/.git/info"
grep -v '^#' "$SUBSET_FILE" | sed '/^[[:space:]]*$/d' > "$WPT_DIR/.git/info/sparse-checkout"

# Fetch only the pinned SHA, filter=blob:none to keep it lean. Skip when
# already at $SHA: with WPT_DIR now shared across every worktree of this
# repo (raikiri-spike-dz8t), an unconditional fetch+checkout here would be
# redundant network I/O on every worktree setup, and a source of
# `index.lock` contention when concurrent worktree sessions race to fetch
# the same already-current checkout.
if [ "$(git -C "$WPT_DIR" rev-parse HEAD 2>/dev/null || true)" != "$SHA" ]; then
  git -C "$WPT_DIR" fetch --depth=1 --filter=blob:none origin "$SHA"
  git -C "$WPT_DIR" checkout -q --detach FETCH_HEAD
fi

if [ "$REPO_ROOT" != "$MAIN_WORKTREE_ROOT" ]; then
  mkdir -p "$REPO_ROOT/target"
  if [ -e "$LOCAL_WPT_DIR" ] && [ ! -L "$LOCAL_WPT_DIR" ]; then
    echo "error: $LOCAL_WPT_DIR exists and is not a symlink; refusing to" \
      "overwrite (remove it manually if it's a stale per-worktree clone" \
      "from before raikiri-spike-dz8t)" >&2
    exit 1
  fi
  ln -sfn "$WPT_DIR" "$LOCAL_WPT_DIR"
  echo "Linked $LOCAL_WPT_DIR -> $WPT_DIR (shared main-worktree checkout)"
fi

echo "WPT ready at $WPT_DIR (SHA: $SHA)"
