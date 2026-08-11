#!/usr/bin/env python3
"""scripts/lib/test_doc_pointer_lint.py — unit tests for doc_pointer_lint.py.

Covers the checker's roles: role 1 / role 2 (bd raikiri-spike-acsw /
raikiri-spike-vyse / raikiri-spike-luxp) and role 4 (bd raikiri-spike-gq7x,
informational only, plus its bd raikiri-spike-8m2l cross-file extension).
No class here invokes `cargo` or depends on this repo's actual crate tree.

ClassifyLineTests / Role1PlainBracketTests / Role2DocBarePointerTests /
Role4LinkedInTestModTests / FindExternalTestModNamesTests /
FindExternalTestModTargetsTests / EvaluateGateTests exercise
`census_file()` / `classify_line()` / `find_first_cfg_test_mod_line()` /
`find_external_test_mod_names()` / `find_external_test_mod_targets()` /
`evaluate_gate()` purely in-memory (string/dataclass in, dataclass/tuple
out) — no filesystem dependency at all.

LoadBaselineTests, MainExitCodeTests, and
ExternalTestModCrossFileCensusTests *do* touch the filesystem: all use
`tempfile.TemporaryDirectory()`, and `MainExitCodeTests` /
`ExternalTestModCrossFileCensusTests` additionally write a small throwaway
`crates/*/src/*.rs` tree to disk (to drive `main()`'s / `run_census()`'s
`--repo-root`/`discover_files()` path end-to-end, not just a single
function's pure logic) — see `MainExitCodeTests`'s own docstring for why
that's deliberate; `ExternalTestModCrossFileCensusTests` needs the same
end-to-end shape because its regression (bd raikiri-spike-8m2l) is
specifically about *crate-wide* file discovery, not anything
`census_file()` alone could exercise on a single file's text. All of it is
temp-dir-scoped and still fast.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v
"""

from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from doc_pointer_lint import (
    CensusResult,
    Occurrence,
    census_file,
    classify_line,
    evaluate_gate,
    find_external_test_mod_names,
    find_external_test_mod_targets,
    find_first_cfg_test_mod_line,
    load_baseline,
    main,
    run_census,
)


class ClassifyLineTests(unittest.TestCase):
    def test_outer_doc_comment(self) -> None:
        self.assertEqual(classify_line("/// hello"), "doc")

    def test_inner_doc_comment(self) -> None:
        self.assertEqual(classify_line("//! hello"), "doc")

    def test_doc_comment_no_space(self) -> None:
        self.assertEqual(classify_line("///hello"), "doc")

    def test_plain_comment(self) -> None:
        self.assertEqual(classify_line("// hello"), "plain")

    def test_quad_slash_is_plain_not_doc(self) -> None:
        # rustc/rustdoc convention: 4+ leading slashes is a normal comment,
        # not a doc comment (commonly used as a section separator).
        self.assertEqual(classify_line("//// separator"), "plain")

    def test_code_line(self) -> None:
        self.assertEqual(classify_line("let x = 1;"), "code")


class Role1PlainBracketTests(unittest.TestCase):
    """vyse rule: any bracket-link in a plain `//` comment, crate:: or not."""

    def test_bracket_backtick_form_flagged(self) -> None:
        text = "    // see [`crate::foo::Bar`] for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "crate::foo::Bar")

    def test_bare_bracket_form_flagged_even_without_crate_prefix(self) -> None:
        # bd raikiri-spike-vyse 2026-07-28 comment: scoping this rule to
        # crate::-prefixed brackets only would miss 18/21 known sites.
        text = "// [PropertyKey] is the enum in question\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "PropertyKey")

    def test_two_occurrences_same_line_both_counted(self) -> None:
        # bd raikiri-spike-vyse's own example: page.rs had one line with two
        # bracket occurrences, both must be counted (occurrence-level, not
        # line-level census).
        text = "// see [crate::cascade::collect_cascaded] and [crate::cascade::pick_winners]\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 2)

    def test_bracket_with_spaces_is_flagged(self) -> None:
        # §8.3 Codex final review (GATE FAIL, non-trivial): an earlier
        # version of _BARE_BRACKET_RE only matched identifier/path-shaped
        # content and silently let this through with 0 violations —
        # contradicting AGENTS.md's unconditional "一切書かない" for plain
        # `//` comments. This is the exact negative-to-positive regression
        # test for that finding.
        text = "// see [two words] for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "two words")

    def test_bracket_with_hyphen_is_flagged(self) -> None:
        # Same finding, second reported shape: a hyphen also broke the old
        # identifier-only character class.
        text = "// see [foo-bar] for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "foo-bar")

    def test_bracket_of_digits_only_is_flagged(self) -> None:
        # A footnote-shaped `[1]` is deliberately NOT exempted — a
        # content-shape exemption is exactly the class of gap the space/
        # hyphen finding came from. The remedy is the same as any other
        # non-link bracket use: wrap it in backticks.
        text = "// see [1] for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "1")

    def test_markdown_inline_link_with_url_not_flagged(self) -> None:
        # `[text](url)` is a genuine Markdown link to an explicit URL, a
        # different (and legitimate) construct from an intra-doc-link
        # look-alike — excluded via a negative lookahead on `(`, added
        # alongside the content-shape broadening since shape alone can no
        # longer rule this out either.
        text = "// see [the spec](https://example.com/spec) for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])

    def test_bare_backtick_in_plain_comment_not_a_violation(self) -> None:
        # The correct/compliant form for a plain comment is a bare code
        # span, not a bracket — must NOT be flagged.
        text = "// see `crate::foo::Bar` for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])
        self.assertEqual(result.plain_bare_crate_count, 1)

    def test_slice_type_inside_backticks_not_a_false_positive(self) -> None:
        # `` `&[Declaration]` `` must not be mistaken for a bracket-link:
        # the brackets are inside the backtick span, not markdown syntax.
        text = "// returns `&[Declaration]` sorted by source order\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])

    def test_attribute_syntax_not_a_false_positive(self) -> None:
        text = "// add #[non_exhaustive] here later\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])

    def test_attribute_with_args_not_a_false_positive(self) -> None:
        # Content-shape broadening matters more here than before: the old
        # identifier-only pattern would never have matched
        # "derive(Debug, Clone)" (parens/comma/space) even without the `#`
        # exclusion. The new any-content pattern would, so the `(?<!#)`
        # lookbehind is now load-bearing for this case, not just a
        # redundant safety net.
        text = "// add #[derive(Debug, Clone)] here later\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])

    def test_slice_type_with_punctuation_inside_backticks_not_a_false_positive(self) -> None:
        text = "// returns `&[the raw elements]` sorted by source order\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])

    def test_doc_comment_bracket_not_counted_toward_role1(self) -> None:
        text = "/// see [`crate::foo::Bar`] for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])


class MultiLineBacktickSpanTests(unittest.TestCase):
    """A backtick code span word-wrapped across consecutive `//`/`///`
    lines must protect a bracket inside it from role 1/2, the same way a
    single-line backtick span does. This whole class exists because an
    earlier version of the §8.3 Codex broadening fix got this wrong: it
    correctly widened _BARE_BRACKET_RE's content match (see
    Role1PlainBracketTests), but analyze_line()/census_file() were still
    line-scoped, so a bracket that's actually safely inside a still-open
    multi-line span (real, pre-existing shape in
    crates/raikiri-style/src/property.rs's CSS-grammar-citation comments,
    e.g. `` `<length [0,∞]> | thin | medium | thick` `` word-wrapped
    across two lines) got a false positive — and an early, automated pass
    of the corresponding fix script "fixed" it by adding a second, nested,
    syntactically-broken backtick layer on top of the (already-correct)
    original text, rather than recognizing it needed no fix at all."""

    def test_bracket_protected_by_span_closing_on_a_later_line(self) -> None:
        # The "carry-in" direction: `` `<length [0,∞]> `` opens on line 1
        # and the bracket appears within the still-open span on that same
        # line; the span doesn't close until "thick`" on line 2.
        text = (
            "        // grammar: `<line-width>` = `<length [0,∞]> |\n"
            "        // thin | medium | thick`。\n"
        )
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])

    def test_bracket_protected_across_a_closing_line(self) -> None:
        # The "carry-out then carry-in" direction: the span opens on line
        # 1 ("`normal |"), and BOTH brackets on line 2 are inside it,
        # closing only at the very end of line 2.
        text = (
            "    // grammar `normal |\n"
            "    // <number [0,∞]> | <length-percentage [0,∞]>` — 4 branches\n"
        )
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])

    def test_bracket_after_span_still_flagged(self) -> None:
        # Once the multi-line span actually closes, a bracket *after* the
        # close on the same line is genuinely outside it and must still be
        # flagged — the carry-over logic must not become a blanket
        # exemption for the rest of the line. The opening line matters
        # here (this is the real crates/raikiri-style/src/property.rs
        # shape): without it, the lone backtick on line 2 has no carried
        # `in_backtick` context and would itself be (correctly, given no
        # more information) treated as a *new* opener extending to EOL —
        # this test's own preceding line is what establishes that it is
        # instead the *closer* of a span opened even earlier.
        text = (
            "    // CSS Fonts 4 §2.2 `<font-weight-absolute> = [ normal | bold |\n"
            "    // <number [1,1000]> ]`。旧実装は [100, 900] に絞っていたが spec は\n"
        )
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "100, 900")

    def test_span_reset_across_a_code_line(self) -> None:
        # A code line ends any comment run outright — an unterminated
        # backtick on the last comment line before a code line must NOT
        # leak "in_backtick" state into a later, unrelated comment run.
        text = (
            "    // opens a span here `never closes\n"
            "    let x = 1;\n"
            "    // [PropertyKey] must still be flagged\n"
        )
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "PropertyKey")

    def test_span_reset_across_doc_plain_boundary(self) -> None:
        # A `doc` -> `plain` (or vice versa) transition is never observed
        # to happen mid-span in this codebase's authoring style; carried
        # state must not leak across that boundary either.
        text = (
            "    /// opens a span here `never closes\n"
            "    // [PropertyKey] must still be flagged\n"
        )
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertEqual(result.plain_bracket_violations[0].span, "PropertyKey")

    def test_nested_bracket_only_outer_flagged(self) -> None:
        # A pre-existing real shape (crates/raikiri-style/src/cascade.rs):
        # an un-backtick-quoted nested bracket structure where an earlier
        # fix pass already wrapped just the *inner* bracket in backticks.
        # _BARE_BRACKET_RE's content class explicitly excludes further
        # `[`/`]`, so it cannot match the outer bracket *and* see through
        # to the inner one in a single pass — but once the inner backtick
        # span is stripped (Step C), the outer bracket's interior is just
        # blanks-and-punctuation, which the outer match correctly still
        # catches. This documents that behavior rather than asserting a
        # single "correct" span count (the real fix, done by hand for the
        # actual sites, was to wrap the *whole* nested expression as one
        # backtick span instead of leaving a partial inner wrap).
        text = "// に [(chapter_title, `[Literal(\"hello\")]`)] が届く。\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.plain_bracket_violations), 1)
        self.assertIn("chapter_title", result.plain_bracket_violations[0].span)


class Role2DocBarePointerTests(unittest.TestCase):
    """luxp ratchet: bare (non-linked) crate:: pointers in doc comments."""

    def test_bare_crate_pointer_in_doc_comment_counted(self) -> None:
        text = "    /// see `crate::foo::Bar` for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_bare_crate_all), 1)
        self.assertEqual(len(result.doc_bare_crate_ratchet), 1)

    def test_linked_crate_pointer_not_counted_as_bare(self) -> None:
        text = "    /// see [`crate::foo::Bar`] for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_bare_crate_all, [])
        self.assertEqual(result.doc_linked_crate_count, 1)

    def test_non_crate_bare_pointer_not_counted(self) -> None:
        # Short-form (no crate:: prefix) bare code spans are out of this
        # metric's scope (bd raikiri-spike-acsw's target, tracked
        # separately/manually, not by this ratchet).
        text = "    /// see `expand_shorthand_into` for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_bare_crate_all, [])

    def test_removed_marker_excludes_from_ratchet(self) -> None:
        text = "/// historical reference: `crate::old::Target` (removed)\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_bare_crate_all), 1)
        self.assertEqual(result.doc_bare_crate_ratchet, [])
        self.assertEqual(len(result.doc_bare_crate_excluded), 1)
        self.assertEqual(result.doc_bare_crate_excluded[0][1], "opt-out-1:(removed)")

    def test_tests_module_target_excludes_from_ratchet(self) -> None:
        text = "/// see `crate::rule::tests::some_test_fn` for the pin\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_bare_crate_ratchet, [])
        self.assertEqual(result.doc_bare_crate_excluded[0][1], "opt-out-2:tests::")

    def test_explicit_ignore_marker_excludes_from_ratchet(self) -> None:
        text = "/// see `crate::foo::Bar` // doc-pointer-lint:ignore: rustdoc-blind, verified\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_bare_crate_ratchet, [])
        self.assertEqual(
            result.doc_bare_crate_excluded[0][1], "explicit:doc-pointer-lint:ignore:"
        )

    def test_ignore_marker_without_reason_does_not_exclude(self) -> None:
        # Same "non-empty reason required" posture as patch_coverage.py's
        # cov:ignore:.
        text = "/// see `crate::foo::Bar` // doc-pointer-lint:ignore:\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_bare_crate_ratchet), 1)

    def test_fail_closed_default_no_marker_counts_toward_ratchet(self) -> None:
        # bd raikiri-spike-luxp: opt-out 3 (rustdoc-blind position) is not
        # statically decidable, so with no explicit marker the default is
        # to count it (fail-closed), not silently exempt it.
        text = "    /// [`#[test]` item doc] `crate::foo::Bar`\n"
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_bare_crate_ratchet), 1)

    def test_ignore_marker_requires_a_nested_double_slash(self) -> None:
        # §8.2 roborev-refine iter1 quality lens (optional item): the
        # marker must be anchored to an actual `//`-introduced fragment,
        # the same way scripts/lib/patch_coverage.py's cov:ignore: is
        # anchored via comment_part() rather than matched as a bare
        # substring anywhere on the line. Without the anchor, prose that
        # merely *mentions* the marker text (no nested `//` of its own)
        # would falsely exempt an unrelated occurrence.
        text = (
            "/// see `crate::foo::Bar`. Note: writing "
            "doc-pointer-lint:ignore: reason without a leading // does "
            "not count as the marker.\n"
        )
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_bare_crate_ratchet), 1)
        self.assertEqual(result.doc_bare_crate_excluded, [])


class FindFirstCfgTestModLineTests(unittest.TestCase):
    """`find_first_cfg_test_mod_line()` — role 4's position predicate. A
    line-position search, deliberately not a brace-depth parser (see role
    4's paragraph in doc_pointer_lint.py's module docstring for why)."""

    def test_no_cfg_test_returns_none(self) -> None:
        lines = ["fn f() {}", "mod not_gated {", "}"]
        self.assertIsNone(find_first_cfg_test_mod_line(lines))

    def test_attribute_directly_above_mod_found(self) -> None:
        lines = ["fn f() {}", "#[cfg(test)]", "mod tests {", "}"]
        self.assertEqual(find_first_cfg_test_mod_line(lines), 2)

    def test_blank_line_between_attribute_and_mod_still_found(self) -> None:
        # Not the prevailing rustfmt style in this repo, but the search
        # tolerates it rather than requiring the two lines be adjacent.
        lines = ["#[cfg(test)]", "", "mod tests {", "}"]
        self.assertEqual(find_first_cfg_test_mod_line(lines), 1)

    def test_attribute_not_followed_by_mod_is_not_a_match(self) -> None:
        # #[cfg(test)] can gate a standalone fn too, not just a mod — only
        # the mod-gating shape is role 4's concern (see this function's
        # docstring: "the start of the first #[cfg(test)] mod … block").
        lines = ["#[cfg(test)]", "fn helper() {}"]
        self.assertIsNone(find_first_cfg_test_mod_line(lines))

    def test_stacked_attribute_between_cfg_test_and_mod_still_found(self) -> None:
        # Real shape found while verifying this function against the whole
        # tree post-implementation (crates/raikiri-html/src/lib.rs:15-17):
        # `#[cfg(test)]` followed by a second attribute (e.g.
        # `#[allow(...)]`) *before* the `mod` line — an earlier version of
        # this function only skipped blank lines, so it stopped at the
        # second attribute line, didn't see `mod`, and returned None for
        # the whole file, silently hiding every doc-linked crate:: span in
        # it from role 4 (a false-clean, not the documented over-count
        # gap). Any number of stacked attributes must be skipped, not just
        # one.
        lines = [
            "#[cfg(test)]",
            "#[allow(clippy::needless_lifetimes, clippy::collapsible_if)]",
            "mod tests {",
            "}",
        ]
        self.assertEqual(find_first_cfg_test_mod_line(lines), 1)

    def test_attribute_followed_by_non_mod_non_attribute_line_is_not_a_match(self) -> None:
        # The stacked-attribute skip must not become unconditional — once
        # a non-blank, non-attribute line is reached and it isn't `mod`,
        # the search still correctly reports no match at this attribute.
        lines = ["#[cfg(test)]", "#[allow(dead_code)]", "fn helper() {}"]
        self.assertIsNone(find_first_cfg_test_mod_line(lines))

    def test_named_test_mod_matches_same_as_plain_tests(self) -> None:
        # bd raikiri-spike-csmj's own re-scan found #[cfg(test)] mod blocks
        # under many names (flags_tests, stylesheets_tests, …), not just
        # the literal `tests` — the predicate must not be name-anchored.
        lines = ["#[cfg(test)]", "mod flags_tests {", "}"]
        self.assertEqual(find_first_cfg_test_mod_line(lines), 1)

    def test_pub_mod_still_matches(self) -> None:
        lines = ["#[cfg(test)]", "pub mod tests {", "}"]
        self.assertEqual(find_first_cfg_test_mod_line(lines), 1)

    def test_only_the_first_matching_attribute_is_reported(self) -> None:
        lines = [
            "#[cfg(test)]",
            "mod tests_a {",
            "}",
            "#[cfg(test)]",
            "mod tests_b {",
            "}",
        ]
        self.assertEqual(find_first_cfg_test_mod_line(lines), 1)

    def test_combined_cfg_form_not_matched_documented_gap(self) -> None:
        # bd raikiri-spike-csmj confirmed no cfg(all(test, …)) / cfg(any(
        # test, …)) form exists in this tree at its scan time, so
        # _CFG_TEST_ATTR_RE deliberately only matches the exact, unadorned
        # `#[cfg(test)]` line — this test documents that as a known,
        # inspected gap (module docstring's own framing), not asserts it's
        # impossible to hit.
        lines = ["#[cfg(all(test, feature = \"x\"))]", "mod tests {", "}"]
        self.assertIsNone(find_first_cfg_test_mod_line(lines))


class Role4LinkedInTestModTests(unittest.TestCase):
    """role 4 (bd raikiri-spike-gq7x): linked crate:: spans at/after the
    first #[cfg(test)] mod block — informational only, never gated."""

    def _lines(self, *lines: str) -> str:
        return "\n".join(lines) + "\n"

    def test_linked_span_before_test_mod_not_counted(self) -> None:
        text = self._lines(
            "/// see [`crate::foo::Bar`] for details",
            "fn production() {}",
            "#[cfg(test)]",
            "mod tests {",
            "}",
        )
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_linked_crate_in_test_mod, [])
        # Still counted in the plain informational total either way.
        self.assertEqual(result.doc_linked_crate_count, 1)

    def test_linked_span_inside_test_mod_counted(self) -> None:
        text = self._lines(
            "#[cfg(test)]",
            "mod tests {",
            "    /// see [`crate::foo::Bar`] for details",
            "    fn helper() {}",
            "}",
        )
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_linked_crate_in_test_mod), 1)
        self.assertEqual(result.doc_linked_crate_in_test_mod[0].span, "crate::foo::Bar")

    def test_no_test_mod_in_file_none_counted(self) -> None:
        text = self._lines("/// see [`crate::foo::Bar`] for details", "fn f() {}")
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_linked_crate_in_test_mod, [])

    def test_non_crate_linked_span_not_counted(self) -> None:
        # Same crate::-only scope as role 2/the doc_linked_crate_count
        # informational counter above it — not a role-4-specific decision.
        text = self._lines(
            "#[cfg(test)]",
            "mod tests {",
            "    /// see [`PropertyKey`] for details",
            "    fn helper() {}",
            "}",
        )
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_linked_crate_in_test_mod, [])

    def test_bare_span_inside_test_mod_not_counted_by_role_4(self) -> None:
        # A *bare* crate:: pointer inside a test mod is role 2's territory
        # (opt-out-2, tests:: path) or role 2's ratchet — never role 4,
        # which only ever looks at linked spans.
        text = self._lines(
            "#[cfg(test)]",
            "mod tests {",
            "    /// see `crate::foo::Bar` for details",
            "    fn helper() {}",
            "}",
        )
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_linked_crate_in_test_mod, [])
        self.assertEqual(len(result.doc_bare_crate_all), 1)

    def test_ignore_marker_does_not_exempt_a_linked_span_from_role_4(self) -> None:
        # Structural point from the module docstring: doc-pointer-lint:
        # ignore: only ever exempts role 2's *bare*-span candidate set —
        # it has no effect on a linked span at all, by design (role 4 has
        # no marker-based escape hatch).
        text = self._lines(
            "#[cfg(test)]",
            "mod tests {",
            "    /// see [`crate::foo::Bar`] // doc-pointer-lint:ignore: reason",
            "    fn helper() {}",
            "}",
        )
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_linked_crate_in_test_mod), 1)

    def test_direction_asymmetry_gap_over_counts_after_block_closes(self) -> None:
        # Documented known gap (module docstring): the position predicate
        # cannot see the test-mod block's closing brace, so a doc-linked
        # span in *later* production code is over-counted. This test
        # exists to pin that documented behavior, not to claim it's
        # correct — over-counting is acceptable for an informational,
        # non-gating role.
        text = self._lines(
            "#[cfg(test)]",
            "mod tests {",
            "}",
            "/// see [`crate::foo::Bar`] for details -- actually production code",
            "fn production() {}",
        )
        result = census_file("f.rs", text)
        self.assertEqual(len(result.doc_linked_crate_in_test_mod), 1)


class FindExternalTestModNamesTests(unittest.TestCase):
    """find_external_test_mod_names() — bd raikiri-spike-8m2l's cross-file
    extension. Unlike find_first_cfg_test_mod_line(), this collects every
    match (a file may declare more than one external test module) and only
    the semicolon (external-file) form, never an inline `{ … }` block."""

    def test_semicolon_form_collected(self) -> None:
        lines = ["#[cfg(test)]", "pub(crate) mod test_dom;"]
        self.assertEqual(find_external_test_mod_names(lines), {"test_dom"})

    def test_inline_brace_form_not_collected(self) -> None:
        # The inline form's body is already in this same file — already
        # covered by find_first_cfg_test_mod_line(), out of scope here.
        lines = ["#[cfg(test)]", "mod tests {", "}"]
        self.assertEqual(find_external_test_mod_names(lines), set())

    def test_no_cfg_test_yields_empty_set(self) -> None:
        lines = ["mod not_gated;"]
        self.assertEqual(find_external_test_mod_names(lines), set())

    def test_multiple_external_declarations_all_collected(self) -> None:
        lines = [
            "#[cfg(test)]",
            "mod test_dom;",
            "#[cfg(test)]",
            "mod test_fixtures;",
        ]
        self.assertEqual(
            find_external_test_mod_names(lines), {"test_dom", "test_fixtures"}
        )

    def test_plain_mod_without_cfg_test_not_collected(self) -> None:
        # A regular (non-test-gated) `mod foo;` must never be treated as an
        # external test-mod target — only one immediately gated by
        # #[cfg(test)] counts.
        lines = ["mod helpers;", "#[cfg(test)]", "mod test_dom;"]
        self.assertEqual(find_external_test_mod_names(lines), {"test_dom"})


class FindExternalTestModTargetsTests(unittest.TestCase):
    """find_external_test_mod_targets() — resolves cross-file `#[cfg(test)]
    mod <name>;` declarations to target file path(s), restricted to files
    actually present in `texts` (repo-relative path -> content; no separate
    file list — `run_census()`, the sole real caller, always has
    `files == list(texts.keys())`, so this is the only input the function
    takes)."""

    def test_sibling_rs_file_resolved(self) -> None:
        # The real crates/raikiri-style/src/lib.rs -> test_dom.rs shape:
        # declaring file is a crate root (lib.rs), so the target resolves
        # to a same-directory sibling.
        texts = {
            "crates/x/src/lib.rs": "#[cfg(test)]\npub(crate) mod test_dom;\n",
            "crates/x/src/test_dom.rs": "pub(crate) struct TestDoc;\n",
        }
        self.assertEqual(
            find_external_test_mod_targets(texts), {"crates/x/src/test_dom.rs"}
        )

    def test_mod_rs_form_resolved(self) -> None:
        texts = {
            "crates/x/src/lib.rs": "#[cfg(test)]\nmod test_dom;\n",
            "crates/x/src/test_dom/mod.rs": "pub(crate) struct TestDoc;\n",
        }
        self.assertEqual(
            find_external_test_mod_targets(texts),
            {"crates/x/src/test_dom/mod.rs"},
        )

    def test_non_root_declaring_file_resolves_under_its_own_subdirectory(self) -> None:
        # rustc's real rule: a non-root file (`bar.rs`, not lib.rs/main.rs/
        # mod.rs) introduces its own same-named subdirectory namespace —
        # `mod foo;` in `bar.rs` is `bar/foo.rs`, not a sibling of `bar.rs`.
        texts = {
            "crates/x/src/lib.rs": "pub mod bar;\n",
            "crates/x/src/bar.rs": "#[cfg(test)]\nmod test_helpers;\n",
            "crates/x/src/bar/test_helpers.rs": "pub(crate) struct Fixture;\n",
        }
        self.assertEqual(
            find_external_test_mod_targets(texts),
            {"crates/x/src/bar/test_helpers.rs"},
        )

    def test_target_not_in_scanned_tree_silently_skipped(self) -> None:
        # find_external_test_mod_targets() must stay total rather than
        # raising when a declared target isn't part of the discovered file
        # list (e.g. behind a cfg-gated path this scan didn't reach).
        texts = {"crates/x/src/lib.rs": "#[cfg(test)]\nmod missing;\n"}
        self.assertEqual(find_external_test_mod_targets(texts), set())

    def test_non_test_gated_mod_not_resolved(self) -> None:
        texts = {
            "crates/x/src/lib.rs": "mod helpers;\n",
            "crates/x/src/helpers.rs": "pub fn f() {}\n",
        }
        self.assertEqual(find_external_test_mod_targets(texts), set())


class ExternalTestModCrossFileCensusTests(unittest.TestCase):
    """census_file()'s is_external_test_mod_target parameter and
    run_census()'s crate-wide wiring of it (bd raikiri-spike-8m2l) — the
    actual false-clean regression this task fixes: a target file named by
    a *different* file's `#[cfg(test)] mod <name>;` declaration has no
    #[cfg(test)] line of its own, so find_first_cfg_test_mod_line() alone
    always returned None for it (every line "before" any test-mod block,
    role 4's blind spot), even though the file's entire content only
    compiles under #[cfg(test)] in the first place. Real-world shape:
    crates/raikiri-style/src/lib.rs:74-75's
    `#[cfg(test)] pub(crate) mod test_dom;` ->
    crates/raikiri-style/src/test_dom.rs, verified by direct reading before
    this fix (test_dom.rs has zero `#[cfg(test)]` lines of its own)."""

    def _write(self, repo_root: str, crate: str, rel: str, content: str) -> None:
        path = Path(repo_root, "crates", crate, "src", rel)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def test_census_file_forces_line_1_when_flagged_external_target(self) -> None:
        # Direct, filesystem-free check of census_file()'s new keyword arg:
        # a file with NO #[cfg(test)] line of its own still gets its
        # doc-linked crate:: span counted toward role 4 once the caller
        # (run_census(), exercised end to end below) tells it this file is
        # an external test-mod target.
        text = "//! module docstring\n/// see [`crate::foo::Bar`] for details\nfn helper() {}\n"
        result = census_file("f.rs", text, is_external_test_mod_target=True)
        self.assertEqual(len(result.doc_linked_crate_in_test_mod), 1)
        self.assertEqual(result.doc_linked_crate_in_test_mod[0].line, 2)

    def test_census_file_default_false_matches_pre_8m2l_behavior(self) -> None:
        # Same text, is_external_test_mod_target omitted (defaults False):
        # must reproduce the exact pre-fix false-clean this task addresses.
        text = "/// see [`crate::foo::Bar`] for details\nfn helper() {}\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.doc_linked_crate_in_test_mod, [])

    def test_run_census_end_to_end_catches_the_lib_rs_test_dom_shape(self) -> None:
        # The actual bd raikiri-spike-8m2l scenario, reproduced as a
        # throwaway two-file crate tree and driven through run_census()
        # (not just census_file() in isolation) so the crate-wide
        # file-discovery step itself is exercised, not only the per-file
        # override it feeds.
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write(
                tmp_dir,
                "fakestyle",
                "lib.rs",
                "pub mod cascade;\n\n#[cfg(test)]\npub(crate) mod test_dom;\n\nfn f() {}\n",
            )
            self._write(
                tmp_dir,
                "fakestyle",
                "test_dom.rs",
                "//! crate-internal test helper.\n"
                "/// see [`crate::foo::Bar`] for details\n"
                "pub(crate) struct TestDoc;\n",
            )
            files = [
                "crates/fakestyle/src/lib.rs",
                "crates/fakestyle/src/test_dom.rs",
            ]
            result = run_census(tmp_dir, files=files)
            self.assertEqual(len(result.doc_linked_crate_in_test_mod), 1)
            occ = result.doc_linked_crate_in_test_mod[0]
            self.assertEqual(occ.path, "crates/fakestyle/src/test_dom.rs")
            self.assertEqual(occ.line, 2)

    def test_run_census_ignores_mod_declaration_without_cfg_test(self) -> None:
        # Negative control: a plain `mod test_dom;` (no #[cfg(test)] gate)
        # must NOT mark test_dom.rs as an external test-mod target — that
        # would be a real production module, not a test-only one, and role
        # 4 must not over-flag it.
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write(
                tmp_dir, "fakestyle", "lib.rs", "pub(crate) mod test_dom;\n"
            )
            self._write(
                tmp_dir,
                "fakestyle",
                "test_dom.rs",
                "/// see [`crate::foo::Bar`] for details\npub(crate) struct TestDoc;\n",
            )
            files = [
                "crates/fakestyle/src/lib.rs",
                "crates/fakestyle/src/test_dom.rs",
            ]
            result = run_census(tmp_dir, files=files)
            self.assertEqual(result.doc_linked_crate_in_test_mod, [])

    def test_run_census_resolves_mod_rs_target_form(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write(
                tmp_dir, "fakestyle", "lib.rs", "#[cfg(test)]\nmod test_dom;\n"
            )
            path = Path(tmp_dir, "crates", "fakestyle", "src", "test_dom", "mod.rs")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                "/// see [`crate::foo::Bar`] for details\npub(crate) struct TestDoc;\n",
                encoding="utf-8",
            )
            files = [
                "crates/fakestyle/src/lib.rs",
                "crates/fakestyle/src/test_dom/mod.rs",
            ]
            result = run_census(tmp_dir, files=files)
            self.assertEqual(len(result.doc_linked_crate_in_test_mod), 1)
            self.assertEqual(
                result.doc_linked_crate_in_test_mod[0].path,
                "crates/fakestyle/src/test_dom/mod.rs",
            )

    def test_own_in_file_test_mod_line_overridden_when_also_an_external_target(
        self,
    ) -> None:
        # Pins doc_pointer_lint.py's module docstring, "Canonical statement
        # of the override rule" paragraph: census_file()'s override is
        # unconditional, not None-only, so a file that's simultaneously an
        # external target AND declares its own later in-file
        # #[cfg(test)] mod block still counts a doc-linked span appearing
        # *before* that in-file block.
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write(
                tmp_dir, "fakestyle", "lib.rs", "#[cfg(test)]\nmod test_dom;\n"
            )
            self._write(
                tmp_dir,
                "fakestyle",
                "test_dom.rs",
                "/// see [`crate::foo::Bar`] for details\n"
                "pub(crate) struct TestDoc;\n"
                "#[cfg(test)]\n"
                "mod nested_tests {\n"
                "}\n",
            )
            files = [
                "crates/fakestyle/src/lib.rs",
                "crates/fakestyle/src/test_dom.rs",
            ]
            result = run_census(tmp_dir, files=files)
            # The span is on line 1, strictly before the in-file
            # #[cfg(test)] mod nested_tests block (line 3) that
            # find_first_cfg_test_mod_line() alone would have found — it
            # must still be counted because the whole file is externally
            # gated.
            self.assertEqual(len(result.doc_linked_crate_in_test_mod), 1)
            self.assertEqual(result.doc_linked_crate_in_test_mod[0].line, 1)


class LoadBaselineTests(unittest.TestCase):
    """§8.2 roborev-refine iter1 quality lens Fix 1: load_baseline() had no
    direct test — the real baseline file has 32 comment lines before its
    integer, so a regression there would silently change what the gate
    compares against."""

    def _write(self, tmp_dir: str, content: str) -> str:
        path = Path(tmp_dir) / "baseline.txt"
        path.write_text(content, encoding="utf-8")
        return str(path)

    def test_leading_comment_block_then_integer(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            content = "\n".join(f"# comment line {i}" for i in range(32)) + "\n\n36\n"
            path = self._write(tmp_dir, content)
            self.assertEqual(load_baseline(path), 36)

    def test_blank_lines_between_comments_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            path = self._write(tmp_dir, "# a\n\n# b\n\n\n7\n")
            self.assertEqual(load_baseline(path), 7)

    def test_bare_integer_no_comments(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            path = self._write(tmp_dir, "0\n")
            self.assertEqual(load_baseline(path), 0)

    def test_missing_file_raises_os_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            missing = str(Path(tmp_dir) / "does-not-exist.txt")
            with self.assertRaises(OSError):
                load_baseline(missing)

    def test_comment_only_file_raises_value_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            path = self._write(tmp_dir, "# only comments\n# more comments\n")
            with self.assertRaises(ValueError):
                load_baseline(path)

    def test_empty_file_raises_value_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            path = self._write(tmp_dir, "")
            with self.assertRaises(ValueError):
                load_baseline(path)


def _occ(n: int = 1) -> list[Occurrence]:
    return [Occurrence("f.rs", i + 1, "crate::foo::Bar", "/// x") for i in range(n)]


class EvaluateGateTests(unittest.TestCase):
    """§8.2 roborev-refine iter1 quality lens Fix 1: the gate's PASS/FAIL
    decision, extracted from main() into evaluate_gate() specifically so
    it's testable without argparse/print/sys.exit."""

    def test_both_pass(self) -> None:
        result = CensusResult(doc_bare_crate_ratchet=_occ(3))
        self.assertEqual(evaluate_gate(result, baseline=3), (True, True))

    def test_role1_fails_role2_passes(self) -> None:
        result = CensusResult(
            plain_bracket_violations=_occ(1), doc_bare_crate_ratchet=_occ(2)
        )
        self.assertEqual(evaluate_gate(result, baseline=5), (False, True))

    def test_role2_fails_role1_passes(self) -> None:
        result = CensusResult(doc_bare_crate_ratchet=_occ(5))
        self.assertEqual(evaluate_gate(result, baseline=4), (True, False))

    def test_both_fail(self) -> None:
        result = CensusResult(
            plain_bracket_violations=_occ(1), doc_bare_crate_ratchet=_occ(5)
        )
        self.assertEqual(evaluate_gate(result, baseline=4), (False, False))

    def test_ratchet_equal_to_baseline_passes(self) -> None:
        # Boundary: role 2 uses <=, not <.
        result = CensusResult(doc_bare_crate_ratchet=_occ(4))
        self.assertEqual(evaluate_gate(result, baseline=4), (True, True))


class MainExitCodeTests(unittest.TestCase):
    """§8.2 roborev-refine iter1 quality lens Fix 1: drive main()'s actual
    PASS/FAIL/exit-code contract (0/1/2) end-to-end against a throwaway
    repo tree, not just the CensusResult-level evaluate_gate() logic —
    catches a wiring bug (e.g. main() ignoring evaluate_gate()'s verdict)
    that a pure evaluate_gate() test can't."""

    def _write_crate_file(self, repo_root: str, crate: str, rel: str, content: str) -> None:
        path = Path(repo_root, "crates", crate, "src", rel)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def _run_main(self, argv: list[str]) -> tuple[int, str]:
        buf = io.StringIO()
        with mock.patch.object(sys, "argv", ["doc_pointer_lint.py", *argv]):
            with contextlib.redirect_stdout(buf):
                code = main()
        return code, buf.getvalue()

    def test_exit_0_when_both_roles_pass(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            # One bare doc-comment crate:: pointer, no plain-comment brackets.
            self._write_crate_file(
                tmp_dir, "fakecrate", "lib.rs", "/// see `crate::foo::Bar`\nfn f() {}\n"
            )
            baseline_path = Path(tmp_dir, "baseline.txt")
            baseline_path.write_text("1\n", encoding="utf-8")
            code, out = self._run_main(
                [
                    "--repo-root",
                    tmp_dir,
                    "--baseline-file",
                    str(baseline_path),
                ]
            )
            self.assertEqual(code, 0)
            self.assertIn("PASS: both gate roles satisfied.", out)

    def test_exit_1_when_ratchet_exceeds_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write_crate_file(
                tmp_dir, "fakecrate", "lib.rs", "/// see `crate::foo::Bar`\nfn f() {}\n"
            )
            baseline_path = Path(tmp_dir, "baseline.txt")
            baseline_path.write_text("0\n", encoding="utf-8")
            code, out = self._run_main(
                [
                    "--repo-root",
                    tmp_dir,
                    "--baseline-file",
                    str(baseline_path),
                ]
            )
            self.assertEqual(code, 1)
            self.assertIn("FAIL: 1 > baseline 0", out)
            self.assertNotIn("PASS: both gate roles satisfied.", out)

    def test_exit_1_when_plain_comment_bracket_present(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write_crate_file(
                tmp_dir, "fakecrate", "lib.rs", "// see [`crate::foo::Bar`]\nfn f() {}\n"
            )
            baseline_path = Path(tmp_dir, "baseline.txt")
            baseline_path.write_text("99\n", encoding="utf-8")
            code, out = self._run_main(
                [
                    "--repo-root",
                    tmp_dir,
                    "--baseline-file",
                    str(baseline_path),
                ]
            )
            self.assertEqual(code, 1)
            self.assertIn("FAIL: 1 occurrence(s)", out)

    def test_exit_2_when_baseline_file_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write_crate_file(tmp_dir, "fakecrate", "lib.rs", "fn f() {}\n")
            code, out = self._run_main(
                [
                    "--repo-root",
                    tmp_dir,
                    "--baseline-file",
                    str(Path(tmp_dir, "does-not-exist.txt")),
                ]
            )
            self.assertEqual(code, 2)

    def test_print_count_bypasses_baseline_and_exits_0(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            self._write_crate_file(
                tmp_dir, "fakecrate", "lib.rs", "/// see `crate::foo::Bar`\nfn f() {}\n"
            )
            code, out = self._run_main(
                [
                    "--repo-root",
                    tmp_dir,
                    "--baseline-file",
                    str(Path(tmp_dir, "does-not-exist.txt")),
                    "--print-count",
                ]
            )
            self.assertEqual(code, 0)
            self.assertEqual(out.strip(), "1")


if __name__ == "__main__":
    unittest.main()
