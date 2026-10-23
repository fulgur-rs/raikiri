#!/usr/bin/env bash
# scripts/lib/instrumented-run.sh — run a command on top of the build that
# `cargo llvm-cov` already produced, instead of compiling the workspace again.
#
# Usage:
#   scripts/lib/instrumented-run.sh <command> [args...]
#   scripts/lib/instrumented-run.sh --bin <name> [args...]
#
# `cargo llvm-cov` builds into its own directory, <target>/llvm-cov-target, and
# instruments only the workspace crates (through a RUSTC_WRAPPER); the
# third-party crates in that directory are ordinary dev-profile builds. A
# second cargo invocation that targets a different directory compiles all of
# those third-party crates again, which is most of the gate's wall-clock time.
# This wrapper points the command at the same directory with the same
# environment, so cargo finds every unit fresh:
#
#   * <command> form: runs <command> with `cargo llvm-cov show-env`'s
#     environment and CARGO_TARGET_DIR=<target>/llvm-cov-target. gate.sh uses it
#     for `cargo test --doc`, which cargo-llvm-cov itself only supports with the
#     nightly-only --doctests flag. rustdoc does not go through the rustc
#     wrapper, so RUSTDOCFLAGS carries the same instrumentation flags the
#     libraries were built with; that lets the doctest binaries link against
#     them.
#   * --bin form: runs a binary that the instrumented build already produced.
#
# LLVM_PROFILE_FILE=/dev/null discards the profile data these runs would write,
# so the coverage report keeps counting only the test suites that
# `cargo llvm-cov` ran (the same scope as CI's coverage job).
#
# Run it only after `cargo llvm-cov --workspace ... --no-report` succeeded; it
# does not build the instrumented artifacts itself. Exit status 2 means the
# instrumented build is not available; anything else is the command's own.

set -euo pipefail

if [[ $# -eq 0 || ( "$1" == "--bin" && $# -lt 2 ) ]]; then
  echo "instrumented-run.sh: usage: instrumented-run.sh <command> [args...] | --bin <name> [args...]" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=repo_root.sh
source "$SCRIPT_DIR/repo_root.sh"
cd "$REPO_ROOT"

# show-env prints `export NAME='value'` lines; CARGO_LLVM_COV_TARGET_DIR is the
# workspace target directory as cargo-llvm-cov resolves it (it follows
# CARGO_TARGET_DIR and [build] target-dir).
if ! SHOW_ENV="$(cargo llvm-cov show-env --sh)"; then
  echo "instrumented-run.sh: cargo llvm-cov show-env failed (is cargo-llvm-cov installed?)." >&2
  exit 2
fi
eval "$SHOW_ENV"
if [[ -z "${CARGO_LLVM_COV_TARGET_DIR:-}" ]]; then
  echo "instrumented-run.sh: cargo llvm-cov show-env did not report a target directory." >&2
  exit 2
fi
INSTRUMENTED_TARGET_DIR="$CARGO_LLVM_COV_TARGET_DIR/llvm-cov-target"
if [[ ! -d "$INSTRUMENTED_TARGET_DIR/debug" ]]; then
  echo "instrumented-run.sh: $INSTRUMENTED_TARGET_DIR/debug does not exist;" >&2
  echo "  run cargo llvm-cov --workspace first (scripts/gate.sh does this)." >&2
  exit 2
fi

export LLVM_PROFILE_FILE=/dev/null

if [[ "$1" == "--bin" ]]; then
  BIN="$INSTRUMENTED_TARGET_DIR/debug/$2"
  shift 2
  if [[ ! -x "$BIN" ]]; then
    echo "instrumented-run.sh: $BIN was not built by the instrumented run." >&2
    exit 2
  fi
  exec "$BIN" "$@"
fi

export CARGO_TARGET_DIR="$INSTRUMENTED_TARGET_DIR"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:+$RUSTDOCFLAGS }-C instrument-coverage --cfg=coverage"
exec "$@"
