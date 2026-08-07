#!/usr/bin/env python3
"""scripts/lib/test_doc_pointer_lint.py — unit tests for doc_pointer_lint.py.

Covers the three checker roles (bd raikiri-spike-acsw / raikiri-spike-vyse /
raikiri-spike-luxp). No class here invokes `cargo` or depends on this
repo's actual crate tree.

ClassifyLineTests / Role1PlainBracketTests / Role2DocBarePointerTests /
EvaluateGateTests exercise `census_file()` / `classify_line()` /
`evaluate_gate()` purely in-memory (string/dataclass in, dataclass/tuple
out) — no filesystem dependency at all.

LoadBaselineTests and MainExitCodeTests *do* touch the filesystem: both
use `tempfile.TemporaryDirectory()`, and `MainExitCodeTests` additionally
writes a small throwaway `crates/*/src/*.rs` tree to disk (to drive
`main()`'s `--repo-root`/`discover_files()` path end-to-end, not just
`evaluate_gate()`'s pure logic) — see `MainExitCodeTests`'s own docstring
for why that's deliberate. All of it is temp-dir-scoped and still fast
(38 cases in ~0.006s as of this writing).

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
    load_baseline,
    main,
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
            self.assertIn("PASS: both roles satisfied.", out)

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
            self.assertNotIn("PASS: both roles satisfied.", out)

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
