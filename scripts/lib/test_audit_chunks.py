#!/usr/bin/env python3
"""scripts/lib/test_audit_chunks.py — unit tests for audit_chunks.py.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v

The audited "binary" is a small script that behaves like audit-vacuous-passes
for ids named after what they do: `hog*` allocates memory and waits, `hang*`
waits, `die*` exits with status 3, and any other id gets a `KEEP` row. It
prints each row to stderr as it finishes and writes the report at the end, as
the real binary does.
"""

import io
import os
import stat
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path

import audit_chunks

FAKE_BINARY = """#!{python}
import os, sys, time

args = sys.argv[1:]
ids = open(args[args.index("--ids") + 1]).read().split()
report = args[args.index("--output") + 1]
print(f"auditing {{len(ids)}} tests with 1 job(s)", file=sys.stderr, flush=True)
rows = []
for test_id in ids:
    if test_id.startswith("hang"):
        time.sleep(600)
    if test_id.startswith("hog"):
        block = b"x" * (200 * 1024 * 1024)
        time.sleep(600)
    if test_id.startswith("die"):
        print("thread 'main' panicked at somewhere", file=sys.stderr, flush=True)
        os._exit(3)
    row = f"{{test_id}}\\tKEEP\\tchecked {{test_id}}"
    print(row, file=sys.stderr, flush=True)
    rows.append(row)
print(f"-- KEEP {{len(rows)}} ({{len(ids)}} total) --", file=sys.stderr, flush=True)
open(report, "w").write("".join(row + "\\n" for row in rows))
"""


def limits(**overrides):
    values = dict(jobs=1, chunk_size=3, max_rss_mb=120, stall_seconds=1.0, poll_seconds=0.05)
    values.update(overrides)
    return audit_chunks.Limits(**values)


class ParseRowsTests(unittest.TestCase):
    def test_only_verdict_rows_of_wanted_ids_count(self):
        lines = [
            "auditing 2 tests with 4 job(s)\n",
            "a.html\tKEEP\t3 declarations ablated\n",
            "thread 'main' panicked at x\n",
            "b.html\tNOT-PASS\tFAIL: mismatch\n",
            "other.html\tKEEP\tnot asked for\n",
            "a.html-extra\tKEEP\tprefix of a wanted id is not a wanted id\n",
            "-- KEEP 1, NOT-PASS 1 (2 total) --\n",
        ]
        self.assertEqual(
            audit_chunks.parse_rows(lines, {"a.html", "b.html"}),
            {"a.html": "KEEP\t3 declarations ablated", "b.html": "NOT-PASS\tFAIL: mismatch"},
        )

    def test_a_detail_with_tabs_is_kept_whole_and_a_lowercase_label_is_not_a_verdict(self):
        lines = ["a.html\tERROR\tone\ttwo\n", "b.html\tkeep\tnot a verdict\n"]
        self.assertEqual(
            audit_chunks.parse_rows(lines, {"a.html", "b.html"}),
            {"a.html": "ERROR\tone\ttwo"},
        )

    def test_the_exit_of_a_killed_process_names_the_signal(self):
        self.assertEqual(audit_chunks.describe_exit(-9), "killed by SIGKILL")
        self.assertEqual(audit_chunks.describe_exit(-999), "killed by signal 999")
        self.assertEqual(audit_chunks.describe_exit(3), "exit status 3")

    @unittest.skipUnless(os.path.exists("/proc/self/status"), "needs /proc")
    def test_resident_memory_of_this_process_is_known(self):
        self.assertGreater(audit_chunks.rss_mb(os.getpid()), 1)

    def test_resident_memory_of_a_process_that_is_gone_is_unknown(self):
        self.assertIsNone(audit_chunks.rss_mb(2**22 + 1))


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="audit-chunks-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.binary = self.root / "fake-audit"
        self.binary.write_text(FAKE_BINARY.format(python=sys.executable), encoding="utf-8")
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)

    def audit(self, ids, **overrides):
        settings = limits(**overrides)

        def command(ids_file, report):
            return [str(self.binary), "--ids", ids_file, "--output", report]

        with redirect_stderr(io.StringIO()):
            return audit_chunks.audit(
                ids, lambda chunk: audit_chunks.run_chunk(command, chunk, settings), settings
            )

    def test_every_id_gets_the_row_the_binary_wrote(self):
        ids = [f"t{n}.html" for n in range(7)]
        rows = self.audit(ids)
        self.assertEqual(list(rows), ids)
        self.assertEqual(rows["t4.html"], "KEEP\tchecked t4.html")

    def test_an_id_that_exhausts_memory_is_marked_and_the_rest_are_audited(self):
        ids = ["a.html", "b.html", "hog.html", "c.html", "d.html"]
        rows = self.audit(ids)
        self.assertEqual(set(rows), set(ids))
        verdict, reason = rows["hog.html"].split("\t")
        self.assertEqual(verdict, "CRASH")
        self.assertIn("memory limit exceeded", reason)
        for test_id in ["a.html", "b.html", "c.html", "d.html"]:
            self.assertEqual(rows[test_id], f"KEEP\tchecked {test_id}")

    def test_two_failing_ids_in_one_chunk_are_both_found(self):
        ids = ["a.html", "hog1.html", "hog2.html", "b.html"]
        rows = self.audit(ids, chunk_size=10)
        self.assertEqual(
            [rows[test_id].split("\t")[0] for test_id in ids], ["KEEP", "CRASH", "CRASH", "KEEP"]
        )

    def test_a_test_that_never_finishes_is_marked_by_the_stall_time(self):
        rows = self.audit(["a.html", "hang.html", "b.html"], stall_seconds=0.5)
        self.assertEqual(rows["a.html"], "KEEP\tchecked a.html")
        self.assertEqual(rows["hang.html"], "CRASH\tno result for 0.5 s")
        self.assertEqual(rows["b.html"], "KEEP\tchecked b.html")

    def test_a_process_that_exits_with_an_error_is_marked_with_its_status(self):
        rows = self.audit(["a.html", "die.html", "b.html"])
        self.assertEqual(rows["die.html"], "CRASH\texit status 3")
        self.assertEqual(rows["b.html"], "KEEP\tchecked b.html")

    def test_a_chunk_that_fails_at_its_first_id_is_split_to_find_it(self):
        ids = ["hang.html"] + [f"t{n}.html" for n in range(5)]
        rows = self.audit(ids, chunk_size=6, stall_seconds=0.5)
        self.assertEqual(rows["hang.html"].split("\t")[0], "CRASH")
        self.assertEqual(
            [rows[f"t{n}.html"].split("\t")[0] for n in range(5)], ["KEEP"] * 5
        )

    def test_a_missing_executable_fails_the_run_instead_of_hiding_ids(self):
        self.binary.unlink()
        with self.assertRaises(FileNotFoundError):
            self.audit(["a.html"])


class MainTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="audit-chunks-main-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.binary = self.root / "fake-audit"
        self.binary.write_text(FAKE_BINARY.format(python=sys.executable), encoding="utf-8")
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)
        self.ids = self.root / "ids.txt"

    def run_main(self, ids, *extra):
        self.ids.write_text("".join(f"{test_id}\n" for test_id in ids), encoding="utf-8")
        output = self.root / "out" / "audit.tsv"
        argv = [
            "--binary", str(self.binary),
            "--wpt-root", "wpt",
            "--ids", str(self.ids),
            "--output", str(output),
            "--jobs", "1",
            "--chunk-size", "2",
            "--max-rss-mb", "120",
            "--stall-seconds", "1",
            "--poll-seconds", "0.05",
            *extra,
        ]
        with redirect_stderr(io.StringIO()) as log:
            status = audit_chunks.main(argv)
        return status, output, log.getvalue()

    def test_the_report_lists_every_id_in_order_with_crashes_marked(self):
        status, output, log = self.run_main(["a.html", "hog.html", "b.html", "a.html"])
        self.assertEqual(status, 0)
        lines = output.read_text(encoding="utf-8").splitlines()
        self.assertEqual([line.split("\t")[0] for line in lines], ["a.html", "hog.html", "b.html"])
        self.assertEqual([line.split("\t")[1] for line in lines], ["KEEP", "CRASH", "KEEP"])
        self.assertIn("-- CRASH 1, KEEP 2 (3 total) --", log)

    def test_an_empty_list_writes_an_empty_report(self):
        status, output, _ = self.run_main([])
        self.assertEqual(status, 0)
        self.assertEqual(output.read_text(encoding="utf-8"), "")

    def test_a_missing_ids_file_is_bad_input(self):
        with redirect_stderr(io.StringIO()):
            status = audit_chunks.main(
                ["--binary", "x", "--wpt-root", "w", "--ids", str(self.root / "nope"), "--output", "o"]
            )
        self.assertEqual(status, 2)

    def test_a_binary_that_cannot_start_is_bad_input(self):
        self.binary.unlink()
        status, output, log = self.run_main(["a.html"])
        self.assertEqual(status, 2)
        self.assertIn("cannot run", log)
        self.assertFalse(output.exists())

    def test_a_limit_below_one_is_rejected(self):
        with self.assertRaises(SystemExit) as raised, redirect_stderr(io.StringIO()):
            audit_chunks.main(
                ["--binary", "x", "--wpt-root", "w", "--ids", "i", "--output", "o", "--chunk-size", "0"]
            )
        self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
