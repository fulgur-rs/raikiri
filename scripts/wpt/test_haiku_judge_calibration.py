"""Tests for the judge calibration helpers in haiku-judge-meta-assert.py."""
from __future__ import annotations

import importlib.util
import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SCRIPT = Path(__file__).parent / "haiku-judge-meta-assert.py"
spec = importlib.util.spec_from_file_location("haiku_judge", SCRIPT)
judge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(judge)

ASSERT = '<meta name="assert" content="Borders are drawn.">'
BODY = "<body><p>Test passes if there is a green square.</p><div></div></body>"


def page(*head: str) -> str:
    return "<html><head>" + "".join(head) + ASSERT + "</head>" + BODY + "</html>"


class CandidateTests(unittest.TestCase):
    def test_reference_free_test_is_only_a_judge_candidate(self) -> None:
        source = page()
        self.assertTrue(judge.is_judge_candidate("css/a.xht", source))
        self.assertFalse(judge.is_judge_candidate("css/a.xht", source, reftest=True))

    def test_match_reftest_is_only_a_calibration_candidate(self) -> None:
        source = page('<link rel="match" href="a-ref.xht">')
        self.assertFalse(judge.is_judge_candidate("css/a.xht", source))
        self.assertTrue(judge.is_judge_candidate("css/a.xht", source, reftest=True))

    def test_mismatch_reftest_is_never_a_calibration_candidate(self) -> None:
        # A mismatch PASS only says the page differs from one reference, so
        # it is no ground truth for the pass condition.
        for links in (
            ('<link rel="mismatch" href="a-notref.xht">',),
            ('<link rel="match" href="a-ref.xht">', '<link rel=mismatch href="b.xht">'),
        ):
            self.assertFalse(judge.is_judge_candidate("css/a.xht", page(*links), reftest=True))

    def test_absence_only_condition_is_not_a_candidate(self) -> None:
        # Most broken renderings show no red either.
        for condition in ("Test passes if there is no red.", "Test passes if there is no red ."):
            source = page().replace("Test passes if there is a green square.", condition)
            self.assertFalse(judge.is_judge_candidate("css/a.xht", source))
        source = page().replace(
            "Test passes if there is a green square.",
            "Test passes if there is a filled green square and no red.",
        )
        self.assertTrue(judge.is_judge_candidate("css/a.xht", source))

    def test_calibration_candidate_still_excludes_script(self) -> None:
        source = page('<link rel="match" href="a-ref.xht">', "<script></script>")
        self.assertFalse(judge.is_judge_candidate("css/a.xht", source, reftest=True))


class CalibrationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.dir = Path(self.temp.name)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write_report(self, rows: list[str]) -> Path:
        path = self.dir / "report.tsv"
        path.write_text("".join(f"{row}\n" for row in rows))
        return path

    def test_report_maps_expectation_statuses_and_skips_errors_and_variants(self) -> None:
        report = self.write_report(
            [
                "a.xht\tPASS\t",
                "b.xht\tXPASS\tlisted as failing",
                "c.xht\tFAIL\t12 pixels",
                "d.xht\tXFAIL\t",
                "e.xht\tERROR\tparse",
                "f.html?x=1\tPASS\t",
                "g.html#h\tFAIL\t",
            ]
        )
        self.assertEqual(
            judge.read_report(report),
            {"a.xht": "PASS", "b.xht": "PASS", "c.xht": "FAIL", "d.xht": "FAIL"},
        )

    def test_sample_is_bounded_per_outcome_and_stable(self) -> None:
        rows = [f"p{i}.xht\tPASS\t" for i in range(5)] + [f"f{i}.xht\tFAIL\t" for i in range(3)]
        report = self.write_report(rows)

        def sample() -> list[str]:
            output = self.dir / "tests.txt"
            args = type(
                "Args", (), {"report": report, "per_outcome": 2, "output": output, "pass_from": None, "candidates": None}
            )
            with redirect_stdout(io.StringIO()):
                judge.command_calibration_sample(args)
            return output.read_text().split()

        first = sample()
        self.assertEqual(len([t for t in first if t.startswith("p")]), 2)
        self.assertEqual(len([t for t in first if t.startswith("f")]), 2)
        self.assertEqual(first, sample())

    def test_sample_takes_pass_rows_only_from_the_accepted_baseline(self) -> None:
        report = self.write_report(["kept.xht\tPASS\t", "sweep.xht\tPASS\t", "f.xht\tFAIL\t"])
        baseline = self.dir / "baseline.txt"
        baseline.write_text("# header\nkept.xht\n")
        output = self.dir / "tests.txt"
        args = type(
            "Args", (), {"report": report, "per_outcome": 5, "output": output, "pass_from": baseline, "candidates": None}
        )
        with redirect_stdout(io.StringIO()):
            judge.command_calibration_sample(args)
        self.assertEqual(output.read_text().split(), ["f.xht", "kept.xht"])

    def test_sample_ignores_report_rows_outside_the_candidates(self) -> None:
        report = self.write_report(["listed.xht\tFAIL\t", "expected.html\tXFAIL\t"])
        candidates = self.dir / "candidates.txt"
        candidates.write_text("listed.xht\n")
        output = self.dir / "tests.txt"
        args = type(
            "Args",
            (),
            {"report": report, "per_outcome": 5, "output": output, "pass_from": None, "candidates": candidates},
        )
        with redirect_stdout(io.StringIO()):
            judge.command_calibration_sample(args)
        self.assertEqual(output.read_text().split(), ["listed.xht"])

    def test_score_counts_false_passes_and_precision(self) -> None:
        outcomes = {"a": "PASS", "b": "PASS", "c": "FAIL", "d": "FAIL", "e": "FAIL"}
        decisions = {
            "a": "pass",
            "b": "needs-human",
            "c": "pass",
            "d": "fail",
            "e": "not-renderable",
            # Not in the report, so it has no ground truth.
            "z": "pass",
        }
        result = judge.score(outcomes, decisions)
        self.assertEqual(result["matrix"]["FAIL"]["pass"], 1)
        self.assertEqual(result["false_passes"], ["c"])
        self.assertAlmostEqual(result["false_pass_rate"], 1 / 3)
        self.assertAlmostEqual(result["pass_precision"], 1 / 2)
        self.assertAlmostEqual(result["pass_recall"], 1 / 2)
        self.assertIn("| FAIL | 1 | 1 | 0 | 1 |", judge.format_score(result, "m"))

    def test_score_without_rows_reports_no_rate(self) -> None:
        result = judge.score({}, {})
        self.assertIsNone(result["false_pass_rate"])
        self.assertIn("n/a", judge.format_score(result, "m"))

    def test_score_command_writes_calibration_json(self) -> None:
        report = self.write_report(["a.xht\tFAIL\t", "b.html\tPASS\t"])
        (self.dir / "reviews.jsonl").write_text(
            json.dumps({"test_id": "a.xht", "decision": "pass"}) + "\n"
            + json.dumps({"test_id": "b.html", "decision": "pass"}) + "\n"
        )
        args = type(
            "Args", (), {"report": report, "review_dir": self.dir, "reviews": "reviews.jsonl", "model": "m"}
        )
        with redirect_stdout(io.StringIO()):
            judge.command_score(args)
        saved = json.loads((self.dir / "calibration.json").read_text())
        self.assertEqual(saved["false_pass_rate"], 1.0)
        self.assertEqual(saved["pass_precision"], 0.5)
        self.assertEqual(saved["model"], "m")
        self.assertEqual(saved["by_suffix"]["xht"]["false_pass_rate"], 1.0)
        self.assertIsNone(saved["by_suffix"]["html"]["false_pass_rate"])


if __name__ == "__main__":
    unittest.main()
