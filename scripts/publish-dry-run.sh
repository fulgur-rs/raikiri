#!/usr/bin/env bash
# scripts/publish-dry-run.sh — check that the crates meant for crates.io can be
# packaged and built from their packages, without publishing anything.
#
# Needs no credentials and writes nothing to a registry. It runs
# `cargo publish --dry-run` for all of the crates at once: cargo packages them
# in dependency order and resolves a dependency on another crate in the list
# from its package, so the check works before any of them is on crates.io. What
# it catches:
#
#   - missing manifest metadata (description, license, ...) and a missing or
#     unreadable `readme` file;
#   - a package that does not build on its own, for example because it uses a
#     file outside its crate directory (raikiri-net is built with its
#     `http-ureq` feature on);
#   - a dependency cycle that publishing would hit: a dev-dependency with a
#     version counts as a dependency of the published package;
#   - a workspace member that may be published but is not in the list below
#     (or the other way round).
#
# Usage:
#   scripts/publish-dry-run.sh                 # package and build the crates
#   scripts/publish-dry-run.sh --allow-dirty   # same, with uncommitted changes
#
# Exit status: 0 on success, non-zero when the list is out of date or cargo
# fails. See docs/releasing.md for the publication plan.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

cd "$REPO_ROOT"

# The crates published to crates.io, from the bottom of the dependency graph up.
# Cargo orders them itself; the order here is for the reader.
CRATES=(
  raikiri-style
  raikiri-layout-geometry
  raikiri-traits
  raikiri-layout-style
  raikiri-svg
  raikiri-net
  raikiri-dom
  raikiri-html
)

cargo_options=()
for argument in "$@"; do
  case "$argument" in
    --allow-dirty) cargo_options+=(--allow-dirty) ;;
    *)
      echo "publish-dry-run: unknown argument: $argument" >&2
      exit 2
      ;;
  esac
done

cargo metadata --no-deps --locked --format-version 1 |
  python3 "$SCRIPT_DIR/lib/publish_dry_run.py" check "${CRATES[@]}"

package_options=()
for crate in "${CRATES[@]}"; do
  package_options+=(-p "$crate")
done

# raikiri-net's `http-ureq` feature is off by default and part of what the
# package ships, so the verification build turns it on. The `${arr[@]+...}`
# form keeps an empty array from failing under `set -u` in bash before 4.4.
cargo publish --dry-run --locked "${package_options[@]}" \
  --features raikiri-net/http-ureq ${cargo_options[@]+"${cargo_options[@]}"}
