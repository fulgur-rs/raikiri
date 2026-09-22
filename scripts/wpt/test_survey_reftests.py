"""Tests for the WPT reftest survey script."""
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from survey_reftests import make_report, read_baseline, scan_reftests  # noqa: E402


class SurveyReftestsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name) / "wpt"
        self.root.mkdir()
        self.baseline_path = Path(self.temp.name) / "baseline.txt"
        self.baseline_path.write_text(
            "# baseline\ncss/css-text/white-space/ws-case.html\n",
            encoding="utf-8",
        )
        self.write(
            "css/css-text/white-space/ws-case.html",
            '<link rel="match" href="../reference/ref.html">'
            '<link rel="mismatch" href="https://example.invalid/ref.html">',
        )
        self.write("css/css-text/reference/ref.html", "<p>reference</p>")
        self.write(
            "css/css-text/root-case.html",
            '<link rel="match" href="#same-document">'
            '<link rel="match" href="not-present.html">',
        )
        self.write(
            "css/css-text/root-case-002.html",
            '<link rel="match" href="reference/ref.html">',
        )
        self.write(
            "html/rendering/table/table-case.xht",
            '<link rel="match" href="table-ref.xht">',
        )
        self.write("html/rendering/table/table-ref.xht", "<p>reference</p>")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write(self, relative: str, contents: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding="utf-8")

    def survey(self, category: str | None = None):
        baseline = read_baseline(self.baseline_path)
        entries, errors = scan_reftests(self.root, baseline, category)
        return make_report(self.root, self.baseline_path, entries, errors, category)

    def test_counts_multiple_match_and_mismatch_pairs_and_baseline_membership(self) -> None:
        report = self.survey("css/css-text")
        summary = report["summary"]
        self.assertEqual(summary["test_files_with_reftest_links"], 3)
        self.assertEqual(summary["reference_pairs"], 5)
        self.assertEqual(summary["baselined_test_files"], 1)
        all_tests = [
            test
            for theme in report["categories"]["css/css-text"]["themes"]
            for test in theme["tests"]
        ]
        ws_test = next(test for test in all_tests if test["test_id"].endswith("ws-case.html"))
        self.assertTrue(ws_test["baseline"])
        self.assertEqual(ws_test["pair_count"], 2)
        self.assertEqual([ref["status"] for ref in ws_test["references"]], ["local", "external"])

    def test_missing_refs_and_root_level_theme_are_flagged(self) -> None:
        report = self.survey("css/css-text")
        root_test = next(
            test
            for theme in report["categories"]["css/css-text"]["themes"]
            for test in theme["tests"]
            if test["test_id"].endswith("root-case.html")
        )
        self.assertEqual(
            [reference["status"] for reference in root_test["references"]],
            ["local", "missing"],
        )
        self.assertIn("missing", root_test["blockers"])
        self.assertTrue(root_test["theme_needs_review"])
        self.assertEqual(root_test["theme"], "(root files)/root-case")
        self.assertEqual(root_test["theme_source"], "filename-prefix")
        self.assertFalse(root_test["structurally_runnable"])
        root_group = next(
            theme
            for theme in report["categories"]["css/css-text"]["themes"]
            if theme["theme"] == "(root files)/root-case"
        )
        self.assertEqual(root_group["test_files"], 2)

    def test_xht_is_surveyed_but_marked_unsupported_by_current_discovery(self) -> None:
        report = self.survey("html/rendering")
        test = report["categories"]["html/rendering"]["themes"][0]["tests"][0]
        self.assertEqual(test["test_id"], "html/rendering/table/table-case.xht")
        self.assertFalse(test["runner_supported_extension"])
        self.assertIn("runner-unsupported-extension", test["blockers"])
        self.assertFalse(test["structurally_runnable"])

    def test_category_filter_is_a_test_path_prefix(self) -> None:
        report = self.survey("css/css-text/white-space")
        self.assertEqual(set(report["categories"]), {"css/css-text"})
        ids = {
            test["test_id"]
            for theme in report["categories"]["css/css-text"]["themes"]
            for test in theme["tests"]
        }
        self.assertEqual(ids, {"css/css-text/white-space/ws-case.html"})

    def test_sparse_scope_is_reported(self) -> None:
        sparse_file = self.root / ".git" / "info" / "sparse-checkout"
        sparse_file.parent.mkdir(parents=True)
        sparse_file.write_text("css/css-text\n", encoding="utf-8")
        report = self.survey("css/css-text")
        self.assertEqual(report["checkout_scope"], "materialized sparse checkout")
        self.assertEqual(report["sparse_checkout_patterns"], ["css/css-text"])

    def test_missing_baseline_is_an_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "baseline file not found"):
            read_baseline(self.baseline_path.parent / "absent.txt")


if __name__ == "__main__":
    unittest.main()
