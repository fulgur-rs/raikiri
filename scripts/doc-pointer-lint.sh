#!/usr/bin/env bash
# scripts/doc-pointer-lint.sh — crate::… doc-pointer scan / lint.
#
# Shared checker for three related design changes, all of which reduce to the same
# occurrence-level scan over crates/*/src/**/*.rs — see the module
# docstring of scripts/lib/doc_pointer_lint.py for the full design
# rationale. In one command:
#
#   1. **role 1 rule (must be zero)**: bracket-link syntax
#      (`[...]` / `` [`...`] ``) inside a plain `//` comment — rustdoc reads
#      zero bytes of a plain `//` comment, so a bracket there looks
#      verified but never is. Must be 0, unconditionally (no baseline).
#   2. **role 2 maximum-count guard (prevents new occurrences)**: bare (non-linked) `crate::…`
#      pointers inside `///` / `//!` doc comments — the exact violation
#      AGENTS.md's "`crate::…` pointer は intra-doc link で書く" rule
#      prohibits. Must not exceed the pinned baseline in
#      scripts/lib/doc_pointer_lint_baseline.txt.
#   4. **role 4 (an earlier change, informational only, NOT gated)**:
#      already-*linked* `crate::…` pointers on a doc line at/after the
#      first `#[cfg(test)] mod …` block in the file — equally invisible to
#      a normal `cargo doc` build as a role-2 violation, but structurally
#      not eligible for an exemption marker with `doc-pointer-lint:ignore:` (that marker only ever
#      exempts a *bare* span from role 2) and deliberately not given a
#      baseline of its own — see scripts/lib/doc_pointer_lint.py's module
#      docstring for the full rationale. Printed in every run's summary;
#      never affects the exit code.
#
# Usage:
#   scripts/doc-pointer-lint.sh            # gate mode: prints scan results, PASS/FAIL
#   scripts/doc-pointer-lint.sh -v          # also lists every occurrence
#   scripts/doc-pointer-lint.sh --print-count   # print current maximum-count value only, exit 0
#
# Explicit exemption (role 2 only, see AGENTS.md's exemption list): a line the scan
# would otherwise count against the maximum-count guard can be excluded explicitly with
#   // doc-pointer-lint:ignore: <reason>
# on the same line — same-line only, no block scoping (unlike
# scripts/lib/patch_coverage.py's cov:ignore:, a doc-comment span has no
# brace-delimited "block" for a preceding-line marker to unambiguously
# bound). This is deliberately narrower than scripts/lib/patch_coverage.py's
# marker, and deliberately count uncertain cases by default: a span this script cannot itself
# determine is not checked by rustdoc (AGENTS.md explicit exemption 3 — #[test] item doc,
# fn-body-local item doc, #[doc(hidden)], tests/benches/examples targets,
# #[cfg]-excluded items, unexpanded macro_rules! bodies) is counted toward
# counted by default; only an explicit marker, written after actually
# confirming that rustdoc does not check it by AGENTS.md's "わざと壊して確かめる"
# procedure, exempts it.
#
# Scope note: this script is standalone and NOT connected to
# scripts/gate.sh / gate.md §8.1 — that integration decision is explicitly
# out of scope for the task that added this script (the earlier change)
# and is left to a separate decision (e.g. a review).
#
# Exit status: 0 if both roles pass, 1 if either fails, 2 on a tooling
# error (e.g. the baseline file is missing or unparseable).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve REPO_ROOT from the caller's shell cwd, with a cross-tree
# mismatch guard — see scripts/lib/repo_root.sh for the rationale.
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

exec python3 "$SCRIPT_DIR/lib/doc_pointer_lint.py" --repo-root "$REPO_ROOT" "$@"
