#!/usr/bin/env bash
# Shallow-clone WPT upstream and sparse-checkout the stable shared roots
# (acid/, css/, fonts/, images/, resources/). Idempotent: re-running updates
# to the pinned SHA.
# Keep subset.txt broad and branch-independent: this checkout is shared across
# worktrees, and narrowing sparse paths in one branch hides files from others.
# The shared sparse-checkout file is made read-only so stale branch scripts
# fail instead of silently replacing these roots.
#
# target/ is per-worktree build output and can be removed by `cargo clean`.
# Keep the physical checkout at $HOME/.cache/raikiri/wpt, outside repository
# cleanup. Each worktree's target/wpt is a replaceable symlink for existing
# Rust tests and scripts. The home cache is the only checkout with the fixed
# shared sparse roots.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve REPO_ROOT from the caller's shell cwd, with a cross-tree
# mismatch guard — see scripts/lib/repo_root.sh for the rationale. This
# matters here specifically because REPO_ROOT below feeds LOCAL_WPT_DIR: a
# tree mismatch would previously have symlinked the *wrong* worktree's
# target/wpt without any indication something was off.
# shellcheck source=../lib/repo_root.sh
source "$SCRIPT_DIR/../lib/repo_root.sh"
# shellcheck source=lib/shared_sparse.sh
source "$SCRIPT_DIR/lib/shared_sparse.sh"
# shellcheck source=lib/cache_path.sh
source "$SCRIPT_DIR/lib/cache_path.sh"

SHA_FILE="$SCRIPT_DIR/pinned_sha.txt"
SUBSET_FILE="$SCRIPT_DIR/subset.txt"
REMOTE_URL="${WPT_REMOTE_URL:-https://github.com/web-platform-tests/wpt.git}"

WPT_DIR="$(wpt_cache_dir)"
LOCAL_WPT_DIR="$REPO_ROOT/target/wpt"

# Detect the old physical checkout under the main worktree. Refuse to create a
# second 400MB clone until the existing checkout has been moved to the cache.
GIT_COMMON_DIR="$(cd "$REPO_ROOT" && git rev-parse --git-common-dir)"
case "$GIT_COMMON_DIR" in
  /*) : ;;
  *) GIT_COMMON_DIR="$REPO_ROOT/$GIT_COMMON_DIR" ;;
esac
GIT_COMMON_DIR="$(cd "$(dirname "$GIT_COMMON_DIR")" && pwd)/$(basename "$GIT_COMMON_DIR")"
MAIN_WORKTREE_ROOT="$(dirname "$GIT_COMMON_DIR")"
LEGACY_SHARED_WPT_DIR="$MAIN_WORKTREE_ROOT/target/wpt"
if [ "$LEGACY_SHARED_WPT_DIR" != "$WPT_DIR" ] &&
  [ -e "$LEGACY_SHARED_WPT_DIR/.git" ] &&
  [ ! -L "$LEGACY_SHARED_WPT_DIR" ]; then
  echo "error: legacy shared WPT checkout still exists at $LEGACY_SHARED_WPT_DIR" >&2
  echo "       Move it to $WPT_DIR and replace the old path with a symlink; see" >&2
  echo "       the migration instructions in scripts/wpt/README.md." >&2
  exit 2
fi
if [ -e "$LOCAL_WPT_DIR" ] && [ ! -L "$LOCAL_WPT_DIR" ]; then
  echo "error: $LOCAL_WPT_DIR exists and is not a symlink; refusing to replace it." >&2
  echo "       Preserve or migrate any old checkout before fetching WPT." >&2
  exit 2
fi

SHA="$(awk '!/^#/ && NF { print; exit }' "$SHA_FILE" | tr -d '[:space:]')"
if [ -z "$SHA" ]; then
  echo "error: no SHA in $SHA_FILE" >&2
  exit 1
fi

# This checkout is shared across branches. Enforce the stable roots before
# touching the shared repository; never accept a task-specific sparse subset.
if ! SUBSET_CONTENT="$(validate_shared_wpt_subset "$SUBSET_FILE")"; then
  exit 2
fi
mapfile -t EXPECTED_SUBSET_PATTERNS <<< "$SUBSET_CONTENT"

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

# Atomically install the shared roots as a read-only inode. A stale branch's
# old fetch.sh fails to open the path; an already-open writer is detached by
# the rename before sparse-checkout reapply reads the canonical file.
SPARSE_CHECKOUT_FILE="$WPT_DIR/.git/info/sparse-checkout"
install_locked_shared_sparse_file "$SPARSE_CHECKOUT_FILE" "${EXPECTED_SUBSET_PATTERNS[@]}"

# Fetch only the pinned SHA, filter=blob:none to keep it lean. Skip when
# already at $SHA: with WPT_DIR now shared across every worktree of this
# repo (the earlier change), an unconditional fetch+checkout here would be
# redundant network I/O on every worktree setup, and a source of
# `index.lock` contention when concurrent worktree sessions race to fetch
# the same already-current checkout.
if [ "$(git -C "$WPT_DIR" rev-parse HEAD 2>/dev/null || true)" != "$SHA" ]; then
  git -C "$WPT_DIR" fetch --depth=1 --filter=blob:none origin "$SHA"
  git -C "$WPT_DIR" checkout -q --detach FETCH_HEAD
fi

# Re-apply the fixed shared roots even when HEAD already equals the pin.
# Per-task path changes are rejected above; category/theme filters belong in
# survey_reftests.py, not in the shared checkout config.
git -C "$WPT_DIR" sparse-checkout reapply

ensure_wpt_cache_link "$REPO_ROOT" "$WPT_DIR"
echo "Linked $LOCAL_WPT_DIR -> $WPT_DIR (shared home cache checkout)"

echo "WPT ready at $WPT_DIR (SHA: $SHA)"
