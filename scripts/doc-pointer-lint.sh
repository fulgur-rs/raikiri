#!/usr/bin/env bash
# scripts/doc-pointer-lint.sh — crate::… doc-pointer census / lint.
#
# Shared checker for three sibling bd issues (bd raikiri-spike-acsw /
# raikiri-spike-vyse / raikiri-spike-luxp), all of which reduce to the same
# occurrence-level census over crates/*/src/**/*.rs — see the module
# docstring of scripts/lib/doc_pointer_lint.py for the full design
# rationale. In one command:
#
#   1. **vyse rule (hard-zero)**: bracket-link syntax
#      (`[...]` / `` [`...`] ``) inside a plain `//` comment — rustdoc reads
#      zero bytes of a plain `//` comment, so a bracket there looks
#      verified but never is. Must be 0, unconditionally (no baseline).
#   2. **luxp ratchet (drift prevention)**: bare (non-linked) `crate::…`
#      pointers inside `///` / `//!` doc comments — the exact violation
#      AGENTS.md's "`crate::…` pointer は intra-doc link で書く" rule
#      prohibits. Must not exceed the pinned baseline in
#      scripts/lib/doc_pointer_lint_baseline.txt.
#
# Usage:
#   scripts/doc-pointer-lint.sh            # gate mode: prints census, PASS/FAIL
#   scripts/doc-pointer-lint.sh -v          # also lists every occurrence
#   scripts/doc-pointer-lint.sh --print-count   # print current ratchet count only, exit 0
#
# Opt-out (role 2 only, see AGENTS.md's opt-out list): a line the census
# would otherwise count against the ratchet can be excluded explicitly with
#   // doc-pointer-lint:ignore: <reason>
# on the same line — same-line only, no block scoping (unlike
# scripts/lib/patch_coverage.py's cov:ignore:, a doc-comment span has no
# brace-delimited "block" for a preceding-line marker to unambiguously
# bound). This is deliberately narrower than scripts/lib/patch_coverage.py's
# marker, and deliberately fail-closed: a span this script cannot itself
# determine is rustdoc-blind (AGENTS.md opt-out 3 — #[test] item doc,
# fn-body-local item doc, #[doc(hidden)], tests/benches/examples targets,
# #[cfg]-excluded items, unexpanded macro_rules! bodies) is counted toward
# the ratchet by default; only an explicit marker, written after actually
# confirming rustdoc-blindness by AGENTS.md's "わざと壊して確かめる"
# procedure, exempts it.
#
# Scope note: this script is standalone and NOT wired into
# scripts/gate.sh / gate.md §8.1 — that integration decision is explicitly
# out of scope for the task that added this script (bd raikiri-spike-luxp)
# and is left to a separate decision (e.g. a retro).
#
# Exit status: 0 if both roles pass, 1 if either fails, 2 on a tooling
# error (e.g. the baseline file is missing or unparseable).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && git rev-parse --show-toplevel)"

exec python3 "$SCRIPT_DIR/lib/doc_pointer_lint.py" --repo-root "$REPO_ROOT" "$@"
