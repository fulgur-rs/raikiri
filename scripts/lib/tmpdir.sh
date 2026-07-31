#!/usr/bin/env bash
# scripts/lib/tmpdir.sh — shared TMPDIR pin for gate scripts.
#
# bd raikiri-spike-weky: `rustdoc --test` (and, more generally, any `cc`/`ld`
# invocation cargo triggers while linking a test/bench binary) writes its
# link scratch space under $TMPDIR. On this machine $TMPDIR defaults to
# `/tmp`, a 16 GB tmpfs shared across every concurrent worktree session
# (feedback-worktree-per-task). Once combined usage crosses roughly 13 GB,
# `cc`/`ld` starts failing with "No space left on device" — intermittently,
# depending on what other sessions happen to be doing at that moment. The
# umbrella crate (`raikiri`) is the most exposed doctest target because its
# dependency graph (vello / parley / icu / html5ever …) makes each doctest
# binary a large link job.
#
# The fix pins TMPDIR to a location on a filesystem that is not a small
# shared tmpfs. `/home` on this machine is a 1.9 TB disk at ~2% used — ample
# headroom regardless of how many sessions are concurrently building. This
# is not a code fix (nothing about the workspace's Rust is wrong); it is an
# environment pin, which is why it lives in a script rather than in
# `rules/gate.md` prose.
#
# Any gate script that shells out to `cargo` (test / clippy / doc / bench /
# llvm-cov) should `source` this file first so all of them share the same
# TMPDIR discipline and none of them re-litigate the default.
#
# Override for a different machine / user via `RAIKIRI_GATE_TMPDIR`. The
# default below intentionally matches the exact path bd raikiri-spike-weky
# verified clean (3/3, TOTAL_PASSED=1084) rather than inventing a new one.

: "${RAIKIRI_GATE_TMPDIR:="$HOME/.cache/raikiri-gate-tmp"}"

mkdir -p "$RAIKIRI_GATE_TMPDIR"
export TMPDIR="$RAIKIRI_GATE_TMPDIR"

# Best-effort noise: warn (do not fail) if TMPDIR still resolves to a tmpfs
# mount, since that's the exact condition this file exists to avoid. This is
# diagnostic only — gate.md §8.1.4-style "widen the net" bias does not apply
# here because a false warning costs nothing and a missed one reproduces the
# whole bug.
if command -v df >/dev/null 2>&1; then
  fs_type="$(df --output=fstype "$RAIKIRI_GATE_TMPDIR" 2>/dev/null | tail -n1 | tr -d '[:space:]')"
  if [[ "$fs_type" == "tmpfs" ]]; then
    echo "warning: RAIKIRI_GATE_TMPDIR ($RAIKIRI_GATE_TMPDIR) resolves to a tmpfs mount." >&2
    echo "         This is the exact condition bd raikiri-spike-weky fixed; set" >&2
    echo "         RAIKIRI_GATE_TMPDIR to a non-tmpfs path with headroom." >&2
  fi
fi
