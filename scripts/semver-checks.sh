#!/usr/bin/env bash
# scripts/semver-checks.sh — which breaking changes does a branch make to the
# published API?
#
# Runs cargo-semver-checks over the crates whose API other code builds on
# (`scripts/public-api-lint.sh --list-semver-crates`), comparing the working
# tree with the merge base of HEAD and a base revision, and prints what it
# finds. The crates are at 0.0.x, where every release may break the API, so the
# default is to report: the exit status is 0 even when it finds breaking
# changes. `--strict` makes them an error, for after the first publication.
# `#[doc(hidden)]` items are not part of the API for this tool, as for the
# version number; docs/public-api.md has the policy.
#
# Usage:
#   scripts/semver-checks.sh                 # base: origin/main
#   scripts/semver-checks.sh --base REV      # base: REV (a branch, tag or commit)
#   scripts/semver-checks.sh --strict        # exit 1 when a breaking change is found
#
# Needs cargo-semver-checks 0.49.x, which reads the rustdoc
# JSON of the pinned toolchain (format 57, see KNOWN_FORMAT_VERSIONS in
# scripts/lib/public_api_lint.py; change them together when the toolchain
# moves):
#
#   cargo install --locked cargo-semver-checks --version 0.49.0
#
# Exit status: 0 no breaking change, or breaking changes without --strict;
# 1 breaking changes with --strict; 2 on a tooling error (the tool is missing
# or failed, the base revision does not exist). cargo-semver-checks itself exits
# 0 when it finds nothing, 100 when it finds a breaking change and 101 when it
# fails.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

cd "$REPO_ROOT"
# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

base="origin/main"
strict=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      [[ $# -ge 2 ]] || { echo "semver-checks: --base needs a revision" >&2; exit 2; }
      base="$2"
      shift 2
      ;;
    --strict)
      strict=1
      shift
      ;;
    -h | --help)
      sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "semver-checks: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if ! cargo semver-checks --version >/dev/null 2>&1; then
  echo "semver-checks: cargo-semver-checks is not installed;" >&2
  echo "  cargo install --locked cargo-semver-checks --version 0.49.0" >&2
  exit 2
fi

merge_base="$(git merge-base "$base" HEAD 2>/dev/null || true)"
if [[ -z "$merge_base" ]]; then
  echo "semver-checks: no merge base between '$base' and HEAD (fetch the base branch?)" >&2
  exit 2
fi

# Read the list first: the exit status of a process substitution is ignored,
# and an empty list would turn into a confusing cargo error.
crates="$(python3 "$SCRIPT_DIR/lib/public_api_lint.py" --repo-root "$REPO_ROOT" --list-semver-crates)"

# A crate that the base revision does not have has nothing to compare with
# (a new crate, a rename): leave it out and say so.
args=()
while read -r crate; do
  if git cat-file -e "$merge_base:crates/$crate/Cargo.toml" 2>/dev/null; then
    args+=(--package "$crate")
  else
    echo "semver-checks: skipping $crate, which the base revision does not have"
  fi
done <<<"$crates"
if [[ ${#args[@]} -eq 0 ]]; then
  echo "semver-checks: no published crate exists in both revisions, nothing to compare"
  exit 0
fi

echo "semver-checks: comparing HEAD with ${merge_base:0:12} (merge base with $base)"
status=0
# The version is the same on both sides, which the tool reads as "a major bump
# is coming" and then skips every lint. `--release-type minor` makes it check
# for what would need a major bump, which is what this script reports.
cargo semver-checks check-release "${args[@]}" --baseline-rev "$merge_base" --release-type minor || status=$?

case "$status" in
  0)
    echo "semver-checks: no breaking change in the published API"
    ;;
  100)
    echo "semver-checks: this branch breaks the published API (listed above)."
    echo "  Expected while the crates are at 0.0.x; docs/public-api.md has the order in which breaking changes are planned."
    if [[ "$strict" -eq 1 ]]; then
      exit 1
    fi
    ;;
  *)
    echo "semver-checks: cargo-semver-checks failed (exit status $status)" >&2
    exit 2
    ;;
esac
