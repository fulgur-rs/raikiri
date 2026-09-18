#!/usr/bin/env python3
"""scripts/lib/doc_pointer_lint.py — `crate::…` doc-pointer scan / lint.

Shared checker for three related design changes that all reduce to the same
occurrence-level scan over `crates/*/src/**/*.rs` (the earlier changes both say explicitly: don't design
three separate checks). It classifies every `//`-comment line as `doc`
(`///` / `//!`) or `plain` (any other `//` line, including `////`-style
separators) and extracts two kinds of Markdown-link-shaped occurrence:

  - "linked" — bracket-wraps-backtick or bare-bracket form, e.g.
    `` [`crate::foo::Bar`] `` or `[crate::foo::Bar]`. This is what
    `rustdoc::broken_intra_doc_links` actually resolves (only in `doc`
    lines — rustdoc never reads a `plain` `//` comment at all).
  - "bare" — a plain backtick code span, e.g. `` `crate::foo::Bar` ``, not
    wrapped in `[...]`.

Two gate roles (pass/fail, occurrence-counted) plus one informational-only
role (role 4, described right after them — see its own paragraph for why
the numbering skips 3), all restricted to `crate::…` pointers
(module-relative pointers are a known missed result — see AGENTS.md's
`crate::…` pointer section on why `crate::`-only grep already missed results
by design; this script inherits that same scope rather than widening it,
to keep its numbers comparable to the historical scan this task is
retiring):

  1. **role 1 rule (must be zero)**: bracket-link syntax in a `plain` `//`
     comment line. AGENTS.md's "既知の限界" section says rustdoc reads zero
     bytes of a plain `//` comment, so a bracket there *looks* like a
     verified intra-doc link but is not — worse than a bare backtick span,
     which at least doesn't claim to be verified. This applies to ANY
     bracket-link, not just `crate::`-prefixed ones (the earlier change
     comment 2026-07-28: scoping this to `crate::` only closes 3/21 sites;
     the danger is unrelated to the `crate::` prefix). Must be 0.

     Role 1 has **no ignore-marker exemption mechanism** — unlike role 2, a
     flagged occurrence cannot be exempted with
     `doc-pointer-lint:ignore:`. AGENTS.md's decided rule for plain `//`
     comments is unconditional ("一切書かない"), so the only remedy is to
     rewrite the line as a bare code span (or, for a non-link bracket use
     that isn't a `crate::…` pointer at all, wrap it in backticks — Step C
     of `analyze_line()` strips backtick spans before the bracket step
     ever runs, so a genuinely non-link `[...]` written as `` `[...]` ``
     is never flagged). `_BARE_BRACKET_RE` matches ANY bracket content
     (not just identifier/path-shaped text — see its own comment for why:
     an earlier, narrower version of this pattern silently missed
     `[two words]` / `[foo-bar]`-shaped brackets, a §8.3 final
     review finding), with two narrow exclusions: `#[attr]` / `&[T]`
     (Rust attribute/slice syntax, not a markdown-link look-alike) and
     `[text](url)` (a genuine Markdown inline link to an explicit URL —
     not currently present in `crates/*/src` plain comments, checked at
     authoring time, but excluded defensively). Block comments (`/* */`)
     — which this script does not scan at all — were independently
     confirmed to hold 0 `crate::` references (the earlier change's
     original scan); if that construct is ever introduced, re-check
     before trusting role 1's output blindly.

     Two further known scan coverage gaps from an independent review,
     both re-verified inert (0, or 1 harmless, live hits) and left as a
     documented caveat rather than a logic change. They pull in opposite
     directions — gap 1 is under-scanning (a hazard could go uncaught),
     gap 2 is over-flagging (legitimate Rust syntax could be misreported
     as a role-1 violation) — but neither currently changes this script's
     output on this repo:

       - **Line-position gap**: `classify_line()` classifies on
         `raw.lstrip()`, so a line with code *before* a trailing `//`
         comment (e.g. `let x = 1; // see [Foo]`) is classified `code`
         outright and the trailing comment is never scanned by either
         role. This is a different axis (line position) from the `/* */`
         exclusion above (comment syntax), even though both are the "same
         hazard" by AGENTS.md's "never write this pattern" framing.
         Repo-wide at filing/re-verification time: exactly one shape
         match, `crates/raikiri-paint/src/text.rs`'s
         `// &[i16] (anyrender::NormalizedCoord alias)`, harmless because
         `[i16]` immediately follows `&`, so it would already be excluded
         by the `(?<![#&])` pattern above even if this line were scanned.
       - **Exclusion-prefix gap**: `(?<![#&])` above only excludes a `[`
         immediately preceded by `#` or `&`; `&mut [T]` or
         `impl AsRef<[u8]>` (a token sits between the `&`/generic bracket
         and the `[`) are not excluded by the pattern as written — such a
         shape in a `plain` `//` comment would be misreported as a role-1
         violation (a false FAIL on ordinary Rust syntax, not a missed
         hazard). Repo-wide at filing/re-verification time: 0 hits.

  2. **role 2 maximum-count guard (prevents new occurrences)**: bare (non-linked) `crate::…`
     pointers in `doc` (`///` / `//!`) comment lines — i.e. occurrences of
     the exact violation AGENTS.md's "`crate::…` pointer は intra-doc link
     で書く" rule prohibits, that have not yet been fixed. Must not exceed
     a pinned baseline (`doc_pointer_lint_baseline.txt`).

  4. **role 4 (the earlier change, informational only — NOT gated, no
     baseline, no `doc-pointer-lint:ignore:` interaction)**: bracket-linked
     (not bare) `crate::…` pointers found on a `doc` (`///` / `//!`) line
     at or after the first `#[cfg(test)] mod …` block in the file — a
     hazard adjacent to, but distinct from, roles 1/2: a `#[cfg(test)] mod
     tests` block is excluded wholesale from a normal (no `--cfg test`)
     `cargo doc` build, so an intra-doc link written on a non-`#[test]`
     mod-level item's doc *inside* such a block (e.g. a private helper fn
     used only by that module's own tests) looks rustdoc-verified but
     never actually gets resolved — empirically confirmed for two such
     sites by the earlier change's block-wide re-scan (see
     `doc_pointer_lint_baseline.txt`'s 2026-08-10 block-wide scan entry). Numbering
     skips 3 because the design note that requested this role (role 4) itself
     calls it "role 4 (仮称)" against a 3-role count that was never a
     literal code construct in this file — kept as-is here so the design note's text and this code stay traceably in sync rather than silently
     renumbering.

     This is deliberately NOT a third maximum-count guard. Two structural reasons,
     not just a style preference: (a) a *linked* span cannot be exempted
     with `doc-pointer-lint:ignore:` at all — that marker's regex and
     `_classify_opt_out()` only ever run against the role-2 bare-span
     candidate set, so there is no remediation path a marker could offer
     here (the actual remedy, demoting the link to a bare span, is
     role 2's territory, not role 4's — see the earlier change's own
     note, which reaches the same conclusion); (b) role 2's own
     baseline file spends roughly 30 lines documenting that a single
     pinned integer "structurally cannot tell new-since-baseline apart
     from was-already-there" under this repo's changes from merges on another branch conditions
     conditions — adding a second gated integer would reproduce that same
     failure mode for a hazard class with no marker-based fix. So role 4
     is visibility only: every run's summary line shows the current
     count, the same way `doc_linked_crate_count` / `plain_bare_crate_count`
     below already are informational without being gated.

     Position predicate — `find_first_cfg_test_mod_line()` — is
     deliberately a **line-position comparison, not brace-depth
     tracking**: it locates the first `#[cfg(test)]` attribute line in the
     file that is (skipping only blank lines) immediately followed by a
     `mod …` declaration, and treats every line at or after that line
     number as "inside/after a test-mod block". the earlier change's
     own block-wide re-scan used exactly this same approximation by hand
     (`doc_pointer_lint_baseline.txt`: "a plain line-number comparison,
     not dependent on any brace-matching") to prove *absence* — that no
     guard-relevant bare span in this repo sits inside a
     `#[cfg(test)] mod` block, since every such span's line number was
     strictly *before* the first such block in its file. Role 4 reuses the
     same predicate to positively enumerate linked spans instead, which
     inherits a direction asymmetry that didn't matter for block-wide scan's
     absence proof but does matter here: a file with production code
     (another `mod`/`fn`, not itself gated by `#[cfg(test)]`) appearing
     *after* its first `#[cfg(test)] mod` block would have any doc-linked
     `crate::…` span in that later code counted extra by role 4, since the
     predicate has no way to know the test-mod block has already closed.
     This codebase's prevailing convention for an *inline* (brace-form)
     `#[cfg(test)] mod tests { … }` block is tests-at-end-of-file — and this
     direction-asymmetry gap not having fired for that shape is,
     concretely, role 4's repo-wide count reading 0 at authoring time
     (rather than a claim this script re-derives from the convention
     itself, which it has no way to verify). The convention does NOT
     reliably hold for the external-file (semicolon) form
     `#[cfg(test)] mod <name>;` that the earlier change (below)
     separately deals with: `crates/raikiri-style/src/lib.rs`'s
     `#[cfg(test)] mod test_dom;` declaration is grouped with the file's
     other `pub mod …` declarations, with substantial production code
     following it — not near the end of the file. Deliberately a
     file-level pointer only, no line number or embedded shell command: a
     pinned line reference goes stale (and *looks* still-verified) the
     moment `lib.rs` gains or loses a `pub mod` line above it; re-derive
     current numbers from the file directly if needed rather than trusting
     one frozen here. Presence (an actual extra result) is still not
     provable without brace-depth tracking, which this role deliberately
     does not implement (would be the single largest complexity addition
     in this script's history, spent on a non-gating, informational role).
     This is documented here in the same register as the two `1ghc`
     scan coverage gaps below: a known, inspected approximation that only
     extra counts (never missed results, and extra counts cannot fail a gate
     this role doesn't have), not a silently wrong one.

     **Cross-file extension (the earlier change)**: the position
     predicate above only ever looks *inside* the file being scanned — but
     a `#[cfg(test)] mod <name>;` declaration (semicolon form, as opposed
     to an inline `mod <name> { … }` block) gates a *different* file
     (`<name>.rs` or `<name>/mod.rs`), and that target file's own content
     has no `#[cfg(test)]` line in it at all — the attribute lives one
     file away, at the declaration site. Scanned in isolation, such a
     target file's `first_test_mod_line` was always `None`: an incorrectly clean result,
     since every line in it — doc comments included — only compiles under
     `#[cfg(test)]` in the first place. Confirmed real (not hypothetical):
     `crates/raikiri-style/src/lib.rs`'s `#[cfg(test)] mod test_dom;` →
     `crates/raikiri-style/src/test_dom.rs`; before this fix, the target
     file was entirely outside role 4's reach regardless of what it
     contained (currently inert only because `test_dom.rs` happens to hold
     no doc-linked `crate::…` span itself — it has exactly one `crate::`
     reference at all, `crate::style_dom::{…}`, and that's a plain `use`
     line, not a doc comment).

     `find_external_test_mod_names()` / `_module_dir_for()` /
     `find_external_test_mod_targets()` resolve a **standard, file-relative**
     `#[cfg(test)] mod <name>;` declaration — the ordinary `<name>.rs` /
     `<name>/mod.rs` convention, no `#[path = …]` override — to its target
     file path(s), crate-tree-wide; this is not a general resolver for
     every shape such a declaration could legally take (see
     `_module_dir_for()`'s own docstring for the `crates/*/src/bin/*.rs`
     binary-crate-root gap, and `find_external_test_mod_targets()`'s own
     docstring for the full list of what's out of scope). `run_census()`
     computes this once per invocation and passes the result into each
     `census_file()` call as `is_external_test_mod_target`.

     **Canonical statement of the override rule** — restated nowhere else
     in this module; `find_external_test_mod_targets()`'s docstring,
     `census_file()`'s inline comment, and this module's test suite all
     point back here instead of repeating it: when
     `is_external_test_mod_target` is true, `census_file()` forces
     `first_test_mod_line` to `1` (the *whole file*, not a specific line
     within it, is what's gated) **unconditionally** — not only as a
     `None`-only fallback. This matters when a file is simultaneously an
     external target *and* declares its own later in-file
     `#[cfg(test)] mod nested;` block: without the unconditional override,
     only lines at or after that later in-file block would count, even
     though the external declaration already gates the file's earlier
     lines too (`ExternalTestModCrossFileCensusTests
     .test_own_in_file_test_mod_line_overridden_when_also_an_external_target`
     pins this).

     This is deliberately a **file-discovery-step extension only**:
     `find_first_cfg_test_mod_line()` itself is untouched, and the new
     cross-file logic resolves whole target files, never partial ones — so
     the "line-position, not brace-depth" constraint above still holds
     without exception.

Opt-out classification (AGENTS.md "explicit exemption (link 化しない)"), applied only
to the role-2 maximum-count candidate set:

  1. **(removed) marker** — grep-able, same line as the flagged span.
  2. **points into `#[cfg(test)] mod tests`** — approximated by checking
     whether the pointer path itself contains a `tests::` segment (AGENTS.md
     explicit exemption 2's own reasoning: "path 中の `tests::` がtest であることを
     示す"), per the earlier change's own framing of this explicit exemption as
     "path の `tests::` パターンで概ね判定可能".
  3. **not checked by rustdoc position** (`#[test]` item doc, fn-body-local item
     doc, `#[doc(hidden)]`, `tests/`/`benches`/`examples` targets,
     `#[cfg]`-excluded item, unexpanded `macro_rules!` body) — bd
     the design note's text states plainly that this is **not
     statically decidable** from the surrounding text alone. This script
     does not attempt to decide it: such spans are counted toward the
     counted by default (count uncertain cases by default), and an author who has separately
     confirmed (by the "わざと壊して確かめる" procedure in AGENTS.md) that a
     specific span is not checked by rustdoc must say so explicitly with a
     `doc-pointer-lint:ignore: <reason>` marker on the same line — the same
     "an explicit exemption only, with no implicit exemption" posture
     `scripts/lib/patch_coverage.py`'s `cov:ignore:` uses. Unlike
     `cov:ignore:`, this marker is same-line only (no block-scoping): a doc
     comment span doesn't have a brace-delimited "block" to scope over the
     way a statement does, so there is nothing for a preceding-line marker
     to unambiguously bound.

Not in scope for this script (see the 3 issues' text for why): gate §8.1 /
scripts/gate.sh integration, lowering the baseline, and any check on
non-`crate::` pointers.
"""

from __future__ import annotations

import argparse
import glob
import re
import sys
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

# -- line classification ---------------------------------------------------

# `///x` is a doc comment (no space required); `////...` is NOT — rustc
# treats 4-or-more leading slashes as a normal (non-doc) comment, commonly
# used as a section-separator idiom. `//!` doc comments have no equivalent
# ambiguity.
_DOC_OUTER_RE = re.compile(r"^///(?!/)")
_DOC_INNER_RE = re.compile(r"^//!")


def classify_line(lstripped: str) -> str:
    """Classify an already-lstripped line as 'doc', 'plain', or 'code'."""
    if _DOC_OUTER_RE.match(lstripped) or _DOC_INNER_RE.match(lstripped):
        return "doc"
    if lstripped.startswith("//"):
        return "plain"
    return "code"


# -- role-4 position predicate (the earlier change) ----------------------

# Matches only the exact, unadorned `#[cfg(test)]` attribute line (rustfmt's
# canonical form for gating a `mod` — the earlier change's block-wide
# re-scan separately confirmed no `cfg(all(test, …))` / `cfg(any(test, …))`
# / same-line `#[cfg(test)] mod x {}` form exists anywhere in this tree at
# that scan's time). A combined-cfg or same-line form would silently not
# match here and so would not advance `find_first_cfg_test_mod_line()`'s
# search — the same "known, inspected gap, not a silent one" register as
# this module's other documented scan coverage gaps.
_CFG_TEST_ATTR_RE = re.compile(r"^#\[cfg\(test\)\]\s*$")
# The `mod` declaration a `#[cfg(test)]` attribute gates. Deliberately not
# anchored to a trailing `{` (rustfmt sometimes breaks a long `mod` line
# before its brace) or to any particular visibility/name shape.
_MOD_DECL_RE = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?mod\s+\w")
# The external-file (semicolon) subset of `_MOD_DECL_RE`'s matches — a
# `mod <name>;` declaration with no inline `{ … }` body, which points at a
# *separate* file (`<name>.rs` or `<name>/mod.rs`) rather than the
# following block in this same file. Captures the module name so
# `find_external_test_mod_names()` can resolve it to a target path (bd
# the earlier change). An inline `mod <name> { … }` block never matches
# this (no trailing `;`) — its body already lives in this same file, so it
# needs no cross-file handling at all.
_EXTERNAL_MOD_DECL_RE = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")


def _iter_cfg_test_mod_lines(lines: list[str]) -> Iterator[tuple[int, str]]:
    """Yield `(cfg_line_index, mod_line_stripped)` for every `#[cfg(test)]`
    attribute line in `lines` that is (skipping blank lines and any further
    stacked `#[...]` attribute lines — e.g. a `#[allow(...)]` commonly
    stacked between `#[cfg(test)]` and its `mod` line, a real shape found
    at `crates/raikiri-html/src/lib.rs:15-17` while verifying this
    function) eventually followed by a `mod …` declaration — i.e. the
    start of every `#[cfg(test)] mod …` block in the file, not just the
    first. `cfg_line_index` is 0-indexed (callers that need a 1-indexed
    line number, matching this module's convention elsewhere, add 1).

    Shared by `find_first_cfg_test_mod_line()` (role 4's in-file,
    first-match-only predicate) and `find_external_test_mod_names()` (bd
    the earlier change's cross-file extension, which needs every match in
    the file, not just the first) so the two functions can never disagree
    about what counts as "the mod line a `#[cfg(test)]` attribute gates."

    Deliberately a line-position search, not a brace-depth parser — see
    role 4's paragraph in this module's docstring for why: this is the same
    approximation the earlier change's own block-wide re-scan used by
    hand ("a plain line-number comparison, not dependent on any
    brace-matching") to prove no guard-relevant bare span sits inside a
    test-mod block.
    """
    for i, raw in enumerate(lines):
        if not _CFG_TEST_ATTR_RE.match(raw.lstrip()):
            continue
        for nxt in lines[i + 1 :]:
            nxt_stripped = nxt.lstrip()
            if not nxt_stripped:
                continue  # blank line — keep looking for the mod decl
            if nxt_stripped.startswith("#["):
                continue  # another stacked attribute — keep looking
            if _MOD_DECL_RE.match(nxt_stripped):
                yield i, nxt_stripped
            break  # first non-blank, non-attribute line isn't `mod`


def find_first_cfg_test_mod_line(lines: list[str]) -> int | None:
    """Return the 1-indexed line number of the first `#[cfg(test)]`
    attribute in `lines` that is eventually followed by a `mod …`
    declaration — i.e. the start of the first `#[cfg(test)] mod …` block in
    the file — or `None` if there is no such block.

    Only the *first* such attribute in the file is reported (a second,
    later `#[cfg(test)] mod` block does not move this line number further
    out) — every line at or after this line number is role 4's
    "inside/after a test-mod block" region, per this function's caller.
    See `_iter_cfg_test_mod_lines()` for the shared search this and
    `find_external_test_mod_names()` both build on.
    """
    for i, _mod_line in _iter_cfg_test_mod_lines(lines):
        return i + 1
    return None


def find_external_test_mod_names(lines: list[str]) -> set[str]:
    """Return every module name declared as `#[cfg(test)] mod <name>;`
    (external-file form) in `lines` — sibling modules whose *body* lives in
    a separate file, one this file's own `#[cfg(test)]` attribute never
    appears in (the earlier change). An inline
    `#[cfg(test)] mod <name> { … }` block is deliberately not returned
    here: its body is already in this same file, so
    `find_first_cfg_test_mod_line()`'s existing in-file search already
    covers it; only the semicolon form points somewhere that search
    structurally cannot see.

    Unlike `find_first_cfg_test_mod_line()`, this collects every match in
    the file, not just the first — a file can declare more than one
    external `#[cfg(test)]` module, and each needs to be resolved to its
    own target file by the caller (`find_external_test_mod_targets()`).
    """
    names: set[str] = set()
    for _cfg_line, mod_line in _iter_cfg_test_mod_lines(lines):
        m = _EXTERNAL_MOD_DECL_RE.match(mod_line)
        if m:
            names.add(m.group(1))
    return names


def _module_dir_for(path: Path) -> Path:
    r"""The directory a `mod <name>;` declared in `path` resolves against: a
    crate/directory root (`lib.rs`, `main.rs`, or the legacy `mod.rs`
    directory-module form) resolves a submodule against its own parent
    directory; any other file introduces its own same-named subdirectory
    namespace (2018+ edition — `bar.rs`'s submodules live under `bar/`),
    e.g. `crates/x/src/bar.rs`'s `mod foo;` is `crates/x/src/bar/foo.rs`,
    not `crates/x/src/foo.rs`. This covers the one real external
    `#[cfg(test)] mod …;` site in this tree at authoring time
    (`crates/raikiri-style/src/lib.rs`, the root case) plus the ordinary
    non-root shape.

    NOT covered, and not claimed to be: `crates/*/src/bin/*.rs`. Each such
    file is its own separate binary-crate root under Cargo's default
    target auto-discovery, so by the same root/non-root logic above it
    arguably belongs in the root branch too — but this function only
    special-cases the `lib`/`main`/`mod` stems, so a `src/bin/*.rs` file
    falls into the non-root branch instead, untested against real rustc
    behavior for that shape. Currently inert regardless of which branch
    would be "correct" here: no file under `crates/*/src/bin/*.rs` in this
    tree declares any `mod` at all (`grep -rn '^\s*\(pub[^ ]* \)\?mod '
    crates/*/src/bin/*.rs` — confirmed empty), so this gap has never
    produced a wrong resolution in practice. Documented as a known,
    unhandled case rather than one this function claims to resolve.
    """
    if path.stem in ("lib", "main", "mod"):
        return path.parent
    return path.parent / path.stem


def find_external_test_mod_targets(texts: dict[str, str]) -> set[str]:
    """Cross-file counterpart to `find_first_cfg_test_mod_line()` (bd
    the earlier change) — see this module's docstring ("Cross-file
    extension" paragraph) for the full rationale and the concrete
    `lib.rs` → `test_dom.rs` example this exists to catch, and its
    "Canonical statement of the override rule" paragraph for how a `True`
    result feeds into `census_file()`.

    Resolves each **standard, file-relative** `#[cfg(test)] mod <name>;`
    declaration found across `texts` (repo-relative path -> file content,
    this repo's whole scanned `.rs` tree) to the target file path(s) it
    points at under the ordinary `<name>.rs` / `<name>/mod.rs` convention
    (via `_module_dir_for()`), restricted to paths that are themselves keys
    of `texts` — a declaration whose target isn't part of the scanned tree
    (should not happen for a real crate, but keeps this function total
    rather than assuming the filesystem always matches the declaration) is
    silently skipped rather than raising. Takes only `texts`, not a
    separate file list: `run_census()`, the sole caller, always has
    `files == list(texts.keys())` by construction, so a second parameter
    would only be able to disagree with `texts` by caller error.

    NOT a general resolver for every legal shape such a declaration could
    take: a `#[path = …]` attribute on the same declaration would silently
    redirect its real target elsewhere, but `find_external_test_mod_names()`
    (the name-collection step this builds on) skips over any stacked
    attribute — `#[path = …]` included — the same way it skips
    `#[allow(...)]`, so a `#[path]`-overridden declaration is resolved (or
    silently mis-resolved) as if it used the default convention. A
    `crates/*/src/bin/*.rs` binary-crate-root declaring file has the same
    gap one level down, in `_module_dir_for()` (see its own docstring).
    And since this module never does brace-depth tracking (this module's
    own docstring, role 4's paragraph), a declaration only reachable
    through a non-standard nested-module namespace — one whose real
    filesystem location diverges from what its declaring file's own path
    implies — is likewise outside what this can resolve. None of these
    shapes appear anywhere in this repo's current `#[cfg(test)] mod` sites
    (only the one plain, standard-convention site this cleanup task addresses),
    so this is a documented scope limit, not a known-wrong resolution.

    Returns the set of target file paths (same repo-relative string form
    as `texts`'s keys) whose *entire* content is gated by an external
    `#[cfg(test)]` declaration in a different file.

    Deliberately a second, file-discovery-level pass, not a change to
    `find_first_cfg_test_mod_line()` itself: role 4's existing
    "line-position search, not brace-depth tracking" constraint is
    preserved — the whole *file* stands in for "the block" here, so there
    is still no brace-matching anywhere in this module.
    """
    known = set(texts)
    targets: set[str] = set()
    for path_str, text in texts.items():
        names = find_external_test_mod_names(text.splitlines())
        if not names:
            continue
        module_dir = _module_dir_for(Path(path_str))
        for name in names:
            for candidate in (module_dir / f"{name}.rs", module_dir / name / "mod.rs"):
                candidate_str = str(candidate)
                if candidate_str in known:
                    targets.add(candidate_str)
    return targets


# -- occurrence extraction --------------------------------------------------

# Step A: bracket wrapping a single backtick span — the canonical intra-doc
# link-with-code-span form, e.g. `` [`crate::foo::Bar`] ``.
_BRACKET_BACKTICK_RE = re.compile(r"\[`([^`\]\n]+)`\]")
# Step C (run against what step A left behind): any remaining backtick span.
_BACKTICK_RE = re.compile(r"`([^`\n]+)`")
# Step E (run against what steps A+C left behind, so nothing already
# consumed as a code span — e.g. `` `&[Declaration]` `` — can re-match
# here): a bare bracket-link. AGENTS.md's rule for plain `//` comments is
# blanket — "bracket link 構文 (`[...]` / `` [`...`] ``) を一切書かない" —
# not conditioned on the bracket content looking like a Rust path/ident.
# §8.3 final review (the earlier review iteration): an earlier version
# of this pattern only matched identifier/path-shaped content
# (`[A-Za-z_][A-Za-z0-9_:<>]*`), which silently let `[two words]` and
# `[foo-bar]` — anything with a space, hyphen, or other punctuation —
# through role 1 undetected, contradicting the "一切" (unconditionally)
# in the rule it's meant to enforce. Matches ANY non-bracket, non-newline
# content between `[` and `]` now, so shape is no longer a loophole.
#
# Two exclusions are preserved from before (content-shape broadening does
# not touch these — see the walkthrough in this module's test suite):
#   - `(?<![#&])`: a `[` immediately preceded by `#` (Rust attribute,
#     `#[cfg(test)]`) or `&` (slice type written without backticks,
#     `&[T]`) is not a markdown/intra-doc-link-shaped bracket at all, and
#     mattered even more once content-shape stopped doing that filtering
#     for us — `#[derive(Debug, Clone)]`'s content ("derive(Debug,
#     Clone)") would otherwise now match too.
#   - `(?!\()`: a `[text](url)` immediately followed by `(` is a genuine
#     Markdown inline link to an explicit URL, a different (and
#     legitimate) construct from an intra-doc-link look-alike — not
#     currently present in `crates/*/src` plain comments (checked at
#     authoring time, see this module's earlier docstring note), but
#     excluded defensively now that content-shape alone can no longer
#     rule it out either.
# A bracket containing only digits (a footnote-style `[1]`) is
# deliberately NOT exempted — content-shape exemptions are exactly what
# let the space/hyphen gap through in the first place; the remedy for a
# legitimate non-link bracket use in a plain comment is the same as for
# any other: wrap it in backticks (`` `[1]` ``), which Step C already
# strips before this step ever runs.
_BARE_BRACKET_RE = re.compile(r"(?<![#&])\[([^\[\]\n]+)\](?!\()")

# Anchored to an actual `//`-introduced marker (mirrors
# scripts/lib/patch_coverage.py's comment_part()-anchored COV_IGNORE_RE),
# not a bare substring search — every line this runs against is already a
# `doc`/`plain` comment line in its own right, so the marker is written as
# a second, nested `//` fragment on that line (e.g. `` /// `crate::foo::Bar`
# // doc-pointer-lint:ignore: reason ``), the same way `cov:ignore:` reads
# as a trailing comment on a code line. Without this anchor, the marker
# text appearing anywhere on the line (e.g. inside prose *describing* the
# marker itself) would falsely exempt an unrelated occurrence.
_IGNORE_MARKER_RE = re.compile(r"//\s*doc-pointer-lint:ignore:\s*(\S.*)$")


@dataclass
class LineFindings:
    linked_spans: list[str] = field(default_factory=list)  # from [`...`] or bare [Ident]
    bare_backtick_spans: list[str] = field(default_factory=list)  # remaining `...`


def analyze_line(raw: str, *, in_backtick: bool = False) -> tuple[LineFindings, bool]:
    """Extract link-shaped and bare-code-span occurrences from one line.

    `in_backtick`: True if a backtick code span opened on a *previous*
    line of the same contiguous comment run and has not yet closed —
    i.e. this line's leading text, up to its first backtick, is itself
    still inside that span and must not be scanned for brackets at all.
    Returns `(findings, still_in_backtick)`, where `still_in_backtick` is
    True iff a span opened on (or carried into) this line remains
    unclosed at end-of-line, for the caller to pass to the next line.

    This carry-over matters because this codebase's dense prose comments
    routinely word-wrap a single backtick-quoted CSS grammar excerpt
    across several `//`/`///` lines, e.g. (crates/raikiri-style/src/
    property.rs, condensed):

    ```
    // grammar: `auto | <length-percentage [0,∞]> | min-content | max-content
    // | fit-content(<length-percentage>)` のうち ...
    ```

    Without carry-over, the first line above looks — in isolation — like
    it has one *unpaired* backtick, so nothing on it gets recognized as
    "inside a code span"; `[0,∞]` would then wrongly reach the bare-bracket
    step and get flagged as a role-1 violation, even though it is (once
    the two lines are read as the single quoted phrase they are) already
    safely inside backticks. the earlier review §8.3 final review
    surfaced this the moment role 1's bracket matching was broadened to
    catch non-identifier-shaped content (this exact `[0,∞]` shape was one
    of the false negatives that broadening fixed) — a version of this
    function that both scanned in isolation *and* had already stopped
    requiring identifier-shaped content would have then falsely flagged,
    and an earlier pass of the accompanying fix did, in fact, mechanically
    "fix" it by adding a second, nested, syntactically-broken layer of
    backticks. This function exists specifically so that doesn't happen.

    Order otherwise matters the same way it always has: each step
    consumes (blanks out) what it matched before the next step runs, so a
    backtick span already claimed by the bracket-wrapping-backtick form
    isn't re-counted as a bare backtick span, and neither of those can
    leave stray brackets for the bare-bracket step to misfire on (e.g.
    `` `&[Declaration]` `` is fully consumed by step C before step E ever
    sees it).
    """
    findings = LineFindings()
    work = raw

    if in_backtick:
        closer = work.find("`")
        if closer == -1:
            # The whole line is still inside the carried-over span —
            # nothing on it is eligible for any of the steps below.
            return findings, True
        # Blank out through (and including) the closing backtick so the
        # steps below never see it as if it were a fresh opener.
        work = " " * (closer + 1) + work[closer + 1 :]

    def _consume_linked(m: re.Match[str]) -> str:
        findings.linked_spans.append(m.group(1))
        return " " * len(m.group(0))

    work = _BRACKET_BACKTICK_RE.sub(_consume_linked, work)

    def _consume_backtick(m: re.Match[str]) -> str:
        findings.bare_backtick_spans.append(m.group(1))
        return " " * len(m.group(0))

    work = _BACKTICK_RE.sub(_consume_backtick, work)

    # `_BACKTICK_RE.sub` above pairs backticks left-to-right and blanks
    # each complete pair, so at most one backtick can be left in `work`
    # afterwards (a run of 3, 5, 7... backticks pairs down to exactly 1
    # leftover; an even run pairs down to 0). One leftover means a new
    # span opened on this line and didn't close before end-of-line —
    # everything from that backtick to end-of-line is *inside* that span
    # (it's the text between the opener and wherever the closer ends up
    # appearing on a later line), so it must be blanked out here too, the
    # same way the carried-in prefix was blanked above, before the
    # bare-bracket step runs. Without this, a bracket following a
    # same-line span-opener (e.g. `` `<length [0,∞]> | `` — the span
    # opens at `` `<length `` and doesn't close until "...thick`" on the
    # *next* line) would still reach the bare-bracket step unprotected;
    # only content *preceding* an unclosed opener on this line, and
    # content on a *later* line once the carried-in prefix above is
    # blanked, was covered without this step.
    still_in_backtick = False
    opener = work.rfind("`")
    if opener != -1 and work.count("`") == 1:
        still_in_backtick = True
        work = work[:opener] + " " * (len(work) - opener)

    for m in _BARE_BRACKET_RE.finditer(work):
        findings.linked_spans.append(m.group(1))

    return findings, still_in_backtick


# -- per-occurrence records ---------------------------------------------------


@dataclass
class Occurrence:
    path: str
    line: int
    span: str
    raw_line: str


@dataclass
class CensusResult:
    # Role 1 (role 1): ANY bracket-link in a `plain` line, crate:: or not.
    plain_bracket_violations: list[Occurrence] = field(default_factory=list)
    # Role 2 (role 2) candidate set, pre-explicit exemption: bare `crate::` backtick spans
    # in `doc` lines.
    doc_bare_crate_all: list[Occurrence] = field(default_factory=list)
    # Subset of the above that survives explicit exemption classification — this is
    # the number compared against the pinned baseline.
    doc_bare_crate_ratchet: list[Occurrence] = field(default_factory=list)
    doc_bare_crate_excluded: list[tuple[Occurrence, str]] = field(default_factory=list)
    # Role 4 (role 4): linked `crate::` spans in `doc` lines at/after the
    # first `#[cfg(test)] mod …` block in their file — informational only,
    # not gated (see this module's docstring for why), so unlike role 2's
    # bare-span set this has no ratchet/baseline/explicit exemption counterpart.
    doc_linked_crate_in_test_mod: list[Occurrence] = field(default_factory=list)
    # Informational only, not gated:
    doc_linked_crate_count: int = 0
    plain_bare_crate_count: int = 0


def _classify_opt_out(occ: Occurrence) -> str | None:
    """Return the explicit exemption reason name if `occ` is exempted from the role-2
    maximum-count check, else None (counted by default)."""
    if "(removed)" in occ.raw_line:
        return "opt-out-1:(removed)"
    if "tests::" in occ.span:
        return "opt-out-2:tests::"
    if _IGNORE_MARKER_RE.search(occ.raw_line):
        return "explicit:doc-pointer-lint:ignore:"
    return None


def census_file(
    path: str, text: str, *, is_external_test_mod_target: bool = False
) -> CensusResult:
    """Census one file's `text`. `is_external_test_mod_target`
    (the earlier change): True iff some *other* file in the crate
    declares `#[cfg(test)] mod <name>;` where `<name>` resolves to this
    file — see `find_external_test_mod_targets()`. Defaults to False so
    existing single-file callers (this module's own test suite) are
    unaffected; `run_census()` is the only caller that computes and passes
    a real value.
    """
    result = CensusResult()
    # Carried across consecutive lines of the *same* comment kind (see
    # analyze_line()'s docstring for why: a backtick span word-wrapped
    # across several `//`/`///` lines must not have its interior treated
    # as bracket-scannable just because the scan is line-based). Reset
    # on any `code` line and on a `doc`<->`plain` kind change — a comment
    # run authored as one continuous phrase is never observed to switch
    # comment style mid-span in this codebase, and a code line ends any
    # comment run outright.
    in_backtick = False
    prev_kind: str | None = None
    lines = text.splitlines()
    # Role 4 (role 4): computed once per file, up front — see
    # find_first_cfg_test_mod_line()'s own docstring and this module's
    # docstring for why this is a line-position search, not a brace-depth
    # parser.
    first_test_mod_line = find_first_cfg_test_mod_line(lines)
    if is_external_test_mod_target:
        # the earlier change: unconditional override (not None-only) —
        # see this module's docstring, "Canonical statement of the
        # override rule" paragraph, for why.
        first_test_mod_line = 1
    for lineno, raw in enumerate(lines, start=1):
        lstripped = raw.lstrip()
        kind = classify_line(lstripped)
        if kind == "code":
            in_backtick = False
            prev_kind = None
            continue
        if kind != prev_kind:
            in_backtick = False
        findings, in_backtick = analyze_line(raw, in_backtick=in_backtick)
        prev_kind = kind
        if kind == "plain":
            for span in findings.linked_spans:
                result.plain_bracket_violations.append(Occurrence(path, lineno, span, raw))
            for span in findings.bare_backtick_spans:
                if span.startswith("crate::"):
                    result.plain_bare_crate_count += 1
        else:  # kind == "doc"
            for span in findings.linked_spans:
                if span.startswith("crate::"):
                    result.doc_linked_crate_count += 1
                    if first_test_mod_line is not None and lineno >= first_test_mod_line:
                        result.doc_linked_crate_in_test_mod.append(
                            Occurrence(path, lineno, span, raw)
                        )
            for span in findings.bare_backtick_spans:
                if span.startswith("crate::"):
                    occ = Occurrence(path, lineno, span, raw)
                    result.doc_bare_crate_all.append(occ)
                    reason = _classify_opt_out(occ)
                    if reason is None:
                        result.doc_bare_crate_ratchet.append(occ)
                    else:
                        result.doc_bare_crate_excluded.append((occ, reason))
    return result


def merge(results: list[CensusResult]) -> CensusResult:
    merged = CensusResult()
    for r in results:
        merged.plain_bracket_violations.extend(r.plain_bracket_violations)
        merged.doc_bare_crate_all.extend(r.doc_bare_crate_all)
        merged.doc_bare_crate_ratchet.extend(r.doc_bare_crate_ratchet)
        merged.doc_bare_crate_excluded.extend(r.doc_bare_crate_excluded)
        merged.doc_linked_crate_in_test_mod.extend(r.doc_linked_crate_in_test_mod)
        merged.doc_linked_crate_count += r.doc_linked_crate_count
        merged.plain_bare_crate_count += r.plain_bare_crate_count
    return merged


def discover_files(repo_root: str) -> list[str]:
    """`crates/*/src/**/*.rs`, repo-relative, sorted for stable output.

    Recursive under each crate's `src/` (not just the top level) so nested
    module dirs (`crates/raikiri-html/src/ua/`, `crates/raikiri-traits/
    src/page/`, `crates/raikiri-wpt/src/bin/`, `.../snapshots/`) are
    covered — a flat `crates/*/src/*.rs` glob (the literal pattern quoted
    in the role 2 design note) would silently skip
    those.
    """
    root = Path(repo_root)
    pattern = str(root / "crates" / "*" / "src" / "**" / "*.rs")
    paths = sorted(glob.glob(pattern, recursive=True))
    return [str(Path(p).relative_to(root)) for p in paths]


def run_census(repo_root: str, files: list[str] | None = None) -> CensusResult:
    """Census `files` (repo-relative paths), or `discover_files(repo_root)`
    if `files` is omitted. Accepting a pre-computed file list lets a caller
    that also wants the file *count* (main(), for the summary line) reuse a
    single `discover_files()` call instead of running the glob twice.

    Reads every file's text into memory up front, rather than streaming
    one file at a time (the pre-cross-file extension shape): role 4's cross-file
    extension (`find_external_test_mod_targets()`) needs a first pass over
    every file's content before any single file's `census_file()` call can
    know whether *it* is an external `#[cfg(test)] mod` target, and a full
    read is required either way since `census_file()` itself needs each
    file's text regardless — this repo's `crates/*/src/**/*.rs` tree
    (~65 files at authoring time) is small enough that holding all of it
    in memory at once is not a concern.
    """
    if files is None:
        files = discover_files(repo_root)
    texts = {
        rel_path: Path(repo_root, rel_path).read_text(encoding="utf-8")
        for rel_path in files
    }
    external_test_mod_targets = find_external_test_mod_targets(texts)
    results = [
        census_file(
            rel_path,
            texts[rel_path],
            is_external_test_mod_target=rel_path in external_test_mod_targets,
        )
        for rel_path in files
    ]
    return merge(results)


def load_baseline(baseline_file: str) -> int:
    """Read the pinned maximum-count baseline: the first non-blank, non-`#`-comment
    line, parsed as an int. Raises `FileNotFoundError` (an `OSError`
    subclass) if `baseline_file` doesn't exist, and `ValueError` if it
    exists but contains no such line (or that line isn't a valid int) —
    both are caught by main() and reported as a tooling error (exit 2),
    not a gate failure."""
    for line in Path(baseline_file).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        return int(line)
    raise ValueError(f"{baseline_file}: no non-comment integer line found")


def evaluate_gate(result: CensusResult, baseline: int) -> tuple[bool, bool]:
    """The gate's PASS/FAIL decision, isolated from argument parsing and
    printing so it's directly unit-testable. Returns `(role1_ok, role2_ok)`:

      - role1_ok: True iff there are 0 plain-comment bracket-link
        violations (role 1's must be zero rule — no baseline involved).
      - role2_ok: True iff the guard-relevant doc-comment bare `crate::…`
        pointer count is at or below `baseline` (role 2's maximum-count check).

    `main()`'s overall exit code is 0 iff both are True, else 1 — see its
    body for the exact mapping (this function does not decide exit codes
    itself, to keep it free of any print/argparse/sys dependency)."""
    role1_ok = not result.plain_bracket_violations
    role2_ok = len(result.doc_bare_crate_ratchet) <= baseline
    return role1_ok, role2_ok


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo-root", required=True)
    ap.add_argument(
        "--baseline-file",
        default=None,
        help="path to the pinned maximum-count baseline (default: "
        "<this script's dir>/doc_pointer_lint_baseline.txt)",
    )
    ap.add_argument(
        "--print-count",
        action="store_true",
        help="print only the current role-2 guard-relevant count and exit "
        "0 (for regenerating the baseline file; does not gate)",
    )
    ap.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="list every occurrence (not just violations)",
    )
    args = ap.parse_args()

    baseline_file = args.baseline_file or str(
        Path(__file__).resolve().parent / "doc_pointer_lint_baseline.txt"
    )

    files = discover_files(args.repo_root)
    result = run_census(args.repo_root, files=files)

    if args.print_count:
        print(len(result.doc_bare_crate_ratchet))
        return 0

    try:
        baseline = load_baseline(baseline_file)
    except (OSError, ValueError) as exc:
        print(f"doc_pointer_lint.py: {exc}", file=sys.stderr)
        return 2

    ratchet_count = len(result.doc_bare_crate_ratchet)

    summary_rows = [
        ("files scanned", len(files)),
        ("doc-comment linked crate:: pointers (info)", result.doc_linked_crate_count),
        (
            "  inside/after #[cfg(test)] mod (role 4, info)",
            len(result.doc_linked_crate_in_test_mod),
        ),
        ("doc-comment bare crate:: pointers (total)", len(result.doc_bare_crate_all)),
        ("  excluded by explicit exemption/marker", len(result.doc_bare_crate_excluded)),
        ("  guard-relevant (role 2)", ratchet_count),
        ("  pinned baseline", baseline),
        ("plain-comment bare crate:: pointers (info)", result.plain_bare_crate_count),
        ("plain-comment bracket-link violations (role 1)", len(result.plain_bracket_violations)),
    ]
    label_width = max(len(label) for label, _ in summary_rows)

    print("== scripts/doc-pointer-lint.sh scan ==")
    for label, value in summary_rows:
        print(f"{label.ljust(label_width)} : {value}")
    print()

    if args.verbose and result.doc_bare_crate_excluded:
        print("doc-comment bare crate:: pointers excluded by the maximum-count guard:")
        for occ, reason in result.doc_bare_crate_excluded:
            print(f"  {occ.path}:{occ.line}  [{reason}]  `{occ.span}`")
        print()

    role1_ok, role2_ok = evaluate_gate(result, baseline)

    print("-- role 1: plain `//` comment bracket-link, must be 0 --")
    if not role1_ok:
        print(f"FAIL: {len(result.plain_bracket_violations)} occurrence(s):")
        for occ in result.plain_bracket_violations:
            print(f"  {occ.path}:{occ.line}  `{occ.span}`")
    else:
        print("PASS: 0 occurrences.")
    print()

    print("-- role 2: doc-comment bare crate:: pointers, maximum-count check vs baseline --")
    if not role2_ok:
        print(f"FAIL: {ratchet_count} > baseline {baseline} (+{ratchet_count - baseline}).")
        # NOT a new-vs-baseline diff: the baseline is a plain integer, not a
        # line set, so this script has no record of *which* occurrences it
        # was measured against — it structurally cannot tell "new since the
        # baseline was pinned" apart from "was already there but happens to
        # push the total over." This lists every current guard-relevant
        # occurrence; per scripts/lib/doc_pointer_lint_baseline.txt's own
        # guidance (a foreign merge can move this count without this
        # branch's own commits touching a single one of these lines),
        # isolate what actually changed by diffing this same command's
        # output against a run at the baseline-pinning commit.
        print(
            "All guard-relevant occurrences (not a new-vs-baseline diff — "
            "compare against a run at the baseline-pinning commit to "
            "isolate what changed):"
        )
        if args.verbose:
            for occ in result.doc_bare_crate_ratchet:
                print(f"  {occ.path}:{occ.line}  `{occ.span}`")
        else:
            print("  (re-run with -v to list every guard-relevant occurrence)")
        print()
        print(
            "Fix: convert to an intra-doc link (AGENTS.md's `crate::…` "
            "pointer 節) — but ONLY after checking this span isn't in a "
            "not checked by rustdoc position first (explicit exemption 3: #[test]-item doc, "
            "fn-body-local item doc, #[doc(hidden)], #[cfg]-excluded item, "
            "unexpanded macro_rules! body — see this file's module "
            "docstring). A bracket link written there looks like a "
            "verified intra-doc link but rustdoc never resolves it (review note — "
            "this exact guard's own baseline was "
            "~40% such spans). If this span is a confirmed explicit exemption (removed "
            "marker / tests:: target / not checked by rustdoc position verified by "
            "the わざと壊して確かめる procedure), mark it with "
            "`doc-pointer-lint:ignore: <reason>` on the same line instead "
            "of linking it."
        )
    else:
        print(f"PASS: {ratchet_count} <= baseline {baseline}.")
        if args.verbose:
            for occ in result.doc_bare_crate_ratchet:
                print(f"  {occ.path}:{occ.line}  `{occ.span}`")
    print()

    print(
        "-- role 4: linked crate:: pointers inside/after a "
        "#[cfg(test)] mod block, informational only, NOT gated --"
    )
    if result.doc_linked_crate_in_test_mod:
        print(
            f"{len(result.doc_linked_crate_in_test_mod)} occurrence(s) — "
            "not a FAIL, this role has no baseline (see this file's module "
            "docstring for why: an already-linked span cannot be exempted "
            "with doc-pointer-lint:ignore:, so the only real remedy is "
            "demoting it to a bare code span, which then falls under "
            "role 2's maximum-count and marker rules instead)."
        )
        if args.verbose:
            for occ in result.doc_linked_crate_in_test_mod:
                print(f"  {occ.path}:{occ.line}  `{occ.span}`")
        else:
            print("  (re-run with -v to list every occurrence)")
    else:
        print("0 occurrences.")
    print()

    if role1_ok and role2_ok:
        # "both" here means the two *gate* roles (1, 2) — role 4 is
        # informational only and never affects this verdict; see its
        # section above, always printed, regardless of this outcome.
        print("PASS: both gate roles satisfied.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
