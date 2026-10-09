#!/usr/bin/env bash
# Run tests and clippy for only the workspace crates a change can affect.
#
# Usage: scripts/affected.sh [--base <ref>] [--direct] [--list]
#
#   --base <ref>   Base ref for the merge-base diff (default: main). Committed,
#                  uncommitted, and untracked changes are all included.
#   --direct       Select only crates containing changed paths; skip the
#                  crates that depend on them.
#   --list         Print the selected crates and exit without building.
#
# This is the fast inner loop for iterating on a change. It is not a gate:
# run scripts/gate.sh once before merging. See scripts/lib/affected.py for
# how changed paths map to crates.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"
cd "$REPO_ROOT"

BASE_REF="main"
DIRECT=()
LIST_ONLY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      BASE_REF="$2"
      shift 2
      ;;
    --direct)
      DIRECT=(--direct)
      shift
      ;;
    --list)
      LIST_ONLY=1
      shift
      ;;
    -h|--help)
      awk 'NR==1 { next } /^#/ { print; next } { exit }' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "affected.sh: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

mapfile -t CRATES < <(python3 "$SCRIPT_DIR/lib/affected.py" \
  --repo-root "$REPO_ROOT" --base "$BASE_REF" "${DIRECT[@]}")

echo "selected crates: ${CRATES[*]:-(none)}"
if [[ "$LIST_ONLY" -eq 1 ]]; then
  exit 0
fi
if [[ ${#CRATES[@]} -eq 0 ]]; then
  echo "No crate is affected; nothing to build."
  exit 0
fi

ARGS=()
FEATURES=()
for crate in "${CRATES[@]}"; do
  ARGS+=(-p "$crate")
  # Cargo only accepts `pkg/feature` when `pkg` is among the selected
  # packages, so enable the default-off HTTP provider only alongside it.
  if [[ "$crate" == "raikiri-net" ]]; then
    FEATURES=(--features raikiri-net/http-ureq)
  fi
done
ARGS+=("${FEATURES[@]}")

FAIL=0
echo "-- cargo fmt --all --check --"
cargo fmt --all --check || FAIL=1
echo "-- cargo clippy ${ARGS[*]} --all-targets --locked -- -D warnings --"
cargo clippy "${ARGS[@]}" --all-targets --locked -- -D warnings || FAIL=1
echo "-- cargo test ${ARGS[*]} --locked --"
cargo test "${ARGS[@]}" --locked || FAIL=1

if [[ "$FAIL" -eq 0 ]]; then
  echo "affected.sh: PASS (not a gate result; run scripts/gate.sh before merging)"
else
  echo "affected.sh: FAIL"
fi
exit "$FAIL"
