"""Tests for the WPT reftest survey script."""
from __future__ import annotations

import io
import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from survey_reftests import checkout_revision, main, make_report, read_baseline, scan_reftests  # noqa: E402


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

    def survey(self, category: str | None = None, theme: str | None = None):
        baseline = read_baseline(self.baseline_path)
        entries, errors = scan_reftests(self.root, baseline, category, theme)
        return make_report(self.root, self.baseline_path, entries, errors, category, theme)

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

    def test_theme_filter_returns_only_the_selected_root_file_group(self) -> None:
        report = self.survey("css/css-text", "(root files)/root-case")
        tests = report["categories"]["css/css-text"]["themes"][0]["tests"]
        self.assertEqual(len(tests), 2)
        self.assertTrue(all("root-case" in test["test_id"] for test in tests))

    def test_sparse_scope_is_reported(self) -> None:
        sparse_file = self.root / ".git" / "info" / "sparse-checkout"
        sparse_file.parent.mkdir(parents=True)
        sparse_file.write_text("css/css-text\n", encoding="utf-8")
        report = self.survey("css/css-text")
        self.assertEqual(report["checkout_scope"], "materialized sparse checkout")
        self.assertEqual(report["sparse_checkout_patterns"], ["css/css-text"])

    def test_cli_refuses_baseline_as_output_path_or_symlink(self) -> None:
        original = self.baseline_path.read_bytes()
        symlink = self.baseline_path.parent / "baseline-output.json"
        symlink.symlink_to(self.baseline_path)
        for output in (self.baseline_path, symlink):
            errors = io.StringIO()
            with redirect_stderr(errors):
                status = main([
                    "--wpt-root", str(self.root),
                    "--baseline", str(self.baseline_path),
                    "--format", "json",
                    "--output", str(output),
                ])
            self.assertEqual(status, 2)
            self.assertIn("must not overwrite the baseline", errors.getvalue())
            self.assertEqual(self.baseline_path.read_bytes(), original)

    def test_git_wpt_scan_ignores_untracked_fixtures(self) -> None:
        self.git(["init", "-q"])
        self.git(["config", "user.name", "Survey Test"])
        self.git(["config", "user.email", "survey@example.invalid"])
        self.git(["add", "-A"])
        self.git(["commit", "-qm", "fixture WPT tree"])
        self.write(
            "css/css-text/untracked-case.html",
            '<link rel="match" href="reference/ref.html">',
        )

        entries, errors = scan_reftests(self.root, set())
        self.assertFalse(errors)
        self.assertNotIn("css/css-text/untracked-case.html", {entry["test_id"] for entry in entries})
        self.assertNotEqual(checkout_revision(self.root), "unknown")

    def test_readonly_sparse_file_rejects_stale_branch_write(self) -> None:
        if hasattr(os, "geteuid") and os.geteuid() == 0:
            self.skipTest("root can bypass file mode permissions")
        sparse_file = Path(self.temp.name) / "sparse-checkout"
        sparse_file.write_text("css\nfonts\nimages\n", encoding="utf-8")
        sparse_file.chmod(0o444)

        stale_writer = subprocess.run(
            ["bash", "-c", 'printf "css/css-text\n" > "$1"', "bash", str(sparse_file)],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(stale_writer.returncode, 0)
        self.assertEqual(sparse_file.read_text(encoding="utf-8"), "css\nfonts\nimages\n")

    def git(self, args: list[str]) -> None:
        subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True)

    def test_revision_is_unknown_for_non_git_wpt_root_inside_parent_repo(self) -> None:
        repository_root = Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory(dir=repository_root) as temporary:
            nested_wpt = Path(temporary) / "copied-wpt"
            nested_wpt.mkdir()
            self.assertEqual(checkout_revision(nested_wpt), "unknown")

    def test_xml_declared_legacy_and_utf16_markup_are_read(self) -> None:
        (self.root / "legacy.xht").write_bytes(
            b"<?xml version=\"1.0\" encoding='iso-8859-1'?>"
            b"<html><body>\xe9<link rel=match href=\"reference.html\"/></body></html>"
        )
        (self.root / "utf16.html").write_bytes(
            "<html><link rel=match href=\"reference.html\"/></html>".encode("utf-16")
        )
        self.write("reference.html", "<p>reference</p>")

        entries, errors = scan_reftests(self.root, set())
        by_id = {entry["test_id"]: entry for entry in entries}
        self.assertFalse(errors)
        self.assertEqual(by_id["legacy.xht"]["pair_count"], 1)
        self.assertEqual(by_id["utf16.html"]["pair_count"], 1)
        self.assertFalse(by_id["legacy.xht"]["runner_supported_extension"])

    def test_missing_baseline_is_an_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "baseline file not found"):
            read_baseline(self.baseline_path.parent / "absent.txt")


if __name__ == "__main__":
    unittest.main()
