#!/usr/bin/env python3
"""scripts/lib/test_doc_pointer_lint.py — unit tests for doc_pointer_lint.py.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v

Covers the three checker roles (bd raikiri-spike-acsw / raikiri-spike-vyse /
raikiri-spike-luxp) at the `census_file()` level — no filesystem or `cargo`
dependency, so these run fast and don't need a real crate tree.
"""

from __future__ import annotations

import unittest

from doc_pointer_lint import census_file, classify_line


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

    def test_doc_comment_bracket_not_counted_toward_role1(self) -> None:
        text = "/// see [`crate::foo::Bar`] for details\n"
        result = census_file("f.rs", text)
        self.assertEqual(result.plain_bracket_violations, [])


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


if __name__ == "__main__":
    unittest.main()
