#!/usr/bin/env bash
# scripts/lib/repo_root.sh — shared repo-root resolution for gate scripts.
#
# Source this (after computing the caller's own SCRIPT_DIR, which every
# caller already needs to locate its sibling files) to set REPO_ROOT and
# guard against a cross-worktree invocation mismatch:
#
#   SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
#   # shellcheck source=lib/repo_root.sh
#   source "$SCRIPT_DIR/lib/repo_root.sh"
#
# Resolves REPO_ROOT from the caller's actual shell cwd, not from the
# on-disk location of the script that sourced this file. BASH_SOURCE[0] (as
# seen inside the sourcing script) is the literal path string used to
# invoke it, which is not necessarily inside the tree the caller actually
# wants checked — e.g. running another checkout's absolute
# .../scripts/foo.sh while the shell's cwd is a different git worktree.
# Deriving REPO_ROOT from that on-disk location would silently operate
# against the wrong tree instead of the one the caller is sitting in.
# `git rev-parse --show-toplevel` with no `-C` already resolves against the
# shell's current directory, which is exactly what we want: whatever tree
# the shell was actually in when it invoked the script is the tree that
# gets checked.
#
# Also guards against the same mismatch reappearing one level down: some
# callers delegate to sibling scripts that also source this file, so if the
# file used to invoke the sourcing script sits in a different git tree than
# the one just resolved above, this fails loudly here rather than letting
# the two trees split invisibly deeper in.
#
# The comparison walks up from the sourcing script's own directory (not
# from some fixed number of ".." hops) via `git rev-parse --show-toplevel`
# again, so this works unmodified regardless of how deep under scripts/ the
# sourcing script lives.
#
# Sets: REPO_ROOT. Does not `cd` there — callers that want that do it
# themselves.
#
# Every failure path below exits non-zero. Since `source` runs in the
# caller's own shell, that only aborts the caller if the caller has `set
# -e` (errexit) in effect — all current callers do (via `set -euo
# pipefail`). A caller that sources this file without errexit would
# silently continue past a failed guard with an empty or stale REPO_ROOT
# instead of stopping; keep `set -e` enabled in any script that sources
# this file.

if [[ -z "${BASH_SOURCE[1]:-}" ]]; then
  echo "repo_root.sh: must be sourced from another script, not run directly." >&2
  exit 2
fi

# Unlike the old per-script resolution (which `cd`'d to the script's own
# on-disk directory before calling git, so it worked from any cwd), this
# resolves against the caller's actual cwd and so can fail outright when
# that cwd is outside any git working tree. Without this guard, that would
# surface as a bare `fatal: not a git repository ... (exit 128)` from git
# itself instead of a diagnostic naming the script and what's wrong.
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "$(basename "${BASH_SOURCE[1]}"): must be run from inside a git working tree." >&2
  exit 2
fi

_repo_root_caller="${BASH_SOURCE[1]}"
_repo_root_script_tree="$(cd "$(dirname "$_repo_root_caller")" && git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -n "$_repo_root_script_tree" && "$_repo_root_script_tree" != "$REPO_ROOT" ]]; then
  _repo_root_caller_name="$(basename "$_repo_root_caller")"
  echo "$_repo_root_caller_name: refusing to run." >&2
  echo "  The script file you invoked lives in a different git tree than" >&2
  echo "  your shell's current directory:" >&2
  echo "    invoked script's tree     : $_repo_root_script_tree" >&2
  echo "    current directory's tree  : $REPO_ROOT" >&2
  echo "  Invoke the $_repo_root_caller_name that belongs to the tree you" >&2
  echo "  want checked (e.g. cd there and run it) instead of another" >&2
  echo "  checkout's absolute path." >&2
  exit 2
fi
unset _repo_root_caller _repo_root_script_tree _repo_root_caller_name
