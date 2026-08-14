#!/usr/bin/env python3
"""scripts/lib/test_patch_coverage.py — unit tests for patch_coverage.py.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v

Focused on `structurally_unreported_paths()` / `is_structurally_unreported()`
(bd raikiri-spike-0gk8): the prior implementation classified a changed line
as non-gating "unreported" via a bare path-shape regex
(`(^|/)(tests|examples)/`), which fires on *any* path containing a `tests/`
or `examples/` directory component at *any* depth — including an ordinary
production module like `crates/raikiri/src/tests/fixtures.rs` that is not a
Cargo test/example target at all. That would silently mask a real coverage
gap. The fixed version cross-references `cargo metadata`'s own target list
(kind `"test"` / `"example"`) instead of guessing from the path string, so
only an actual auto-discovered integration-test/example binary's entry file
is exempted.

These tests exercise `structurally_unreported_paths()` against a fixture
metadata dict (no real `cargo metadata` invocation — kept fast and
deterministic; the JSON shape used here was confirmed by hand against this
repo's actual `cargo metadata --no-deps --format-version=1` output during
development of this fix) and `is_structurally_unreported()` against plain
sets, so no test here depends on the `cargo` binary being on PATH.
"""

from __future__ import annotations

import contextlib
import io
import unittest

from patch_coverage import (
    classify_no_lcov_record_lines,
    is_structurally_unreported,
    structurally_unreported_paths,
)

REPO_ROOT = "/repo"


def _target(kind: list[str], src_path: str) -> dict:
    return {"kind": kind, "src_path": src_path}


def _metadata(*targets: dict) -> dict:
    """Wrap targets into a single fake package, mirroring `cargo metadata`'s
    `{"packages": [{"targets": [...]}]}` shape closely enough for
    `structurally_unreported_paths()`, which only reads those two keys."""
    return {"packages": [{"targets": list(targets)}]}


class StructurallyUnreportedPathsTests(unittest.TestCase):
    def test_test_target_entry_file_is_included(self) -> None:
        metadata = _metadata(
            _target(["test"], f"{REPO_ROOT}/crates/raikiri/tests/parse_html_limits.rs"),
        )
        paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertIn("crates/raikiri/tests/parse_html_limits.rs", paths)

    def test_example_target_entry_file_is_included(self) -> None:
        metadata = _metadata(
            _target(["example"], f"{REPO_ROOT}/crates/raikiri/examples/demo.rs"),
        )
        paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertIn("crates/raikiri/examples/demo.rs", paths)

    def test_bench_target_is_excluded(self) -> None:
        # Absence of an SF: record for a bench file means "never ran" (a
        # real gap), not "structurally never reported" — benches must stay
        # out of this set even though cargo-llvm-cov also omits them from
        # the lcov export. See module docstring in patch_coverage.py.
        metadata = _metadata(
            _target(["bench"], f"{REPO_ROOT}/crates/raikiri-style/benches/cascade.rs"),
        )
        paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertEqual(paths, set())

    def test_lib_and_bin_targets_are_excluded(self) -> None:
        metadata = _metadata(
            _target(["lib"], f"{REPO_ROOT}/crates/raikiri/src/lib.rs"),
            _target(["bin"], f"{REPO_ROOT}/crates/raikiri-wpt/src/main.rs"),
        )
        paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertEqual(paths, set())

    def test_tests_shaped_path_on_a_non_test_target_is_excluded(self) -> None:
        # Regression test for the bug this task fixes, made to actually
        # discriminate: a `["bench"]`-kind target with an explicit
        # `tests/`-shaped `src_path` (a `[[bench]]` entry with a
        # non-default `path`, realizable in a real Cargo.toml — e.g.
        # `crates/raikiri/tests/perf_smoke.rs`) is *not* a test/example
        # target and must be excluded. A path-shape-guessing
        # reimplementation (the old `(^|/)(tests|examples)/` regex) would
        # wrongly include this path; the kind-driven implementation
        # correctly excludes it because the target's own `kind` says
        # `"bench"`, not `"test"`/`"example"`. (The previous version of
        # this test asserted a path that never appeared as any target's
        # `src_path` in the fixture, which held even with the kind filter
        # deleted entirely — it could not fail, so it could not
        # regress-test anything. This version can.)
        metadata = _metadata(
            _target(["lib"], f"{REPO_ROOT}/crates/raikiri/src/lib.rs"),
            _target(["test"], f"{REPO_ROOT}/crates/raikiri/tests/external_consumer.rs"),
            _target(["bench"], f"{REPO_ROOT}/crates/raikiri/tests/perf_smoke.rs"),
        )
        paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertNotIn("crates/raikiri/tests/perf_smoke.rs", paths)
        # Sanity: the real test target from the same fixture is still
        # included, so the exclusion above is about kind, not a bug that
        # dropped everything.
        self.assertIn("crates/raikiri/tests/external_consumer.rs", paths)

    def test_explicit_test_path_outside_tests_dir_is_still_included(self) -> None:
        # Classification is kind-driven, not path-driven: a `[[test]]`
        # section with an explicit non-`tests/`-shaped `path` is still a
        # real Cargo test target and must be exempted.
        metadata = _metadata(
            _target(["test"], f"{REPO_ROOT}/crates/raikiri/it/custom_integration.rs"),
        )
        paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertIn("crates/raikiri/it/custom_integration.rs", paths)

    def test_src_path_outside_repo_root_is_kept_verbatim(self) -> None:
        # Defensive: a target whose src_path doesn't start with repo_root
        # (shouldn't happen for a workspace member, but --no-deps means we
        # never see an external dependency's targets to begin with) is
        # kept as-is rather than dropped, so it simply can never match a
        # repo-relative diff path instead of crashing. This is the
        # intentional silent-pass-through *value*; the diagnostic added
        # alongside it is covered separately below (stderr suppressed here
        # so this test's output stays clean).
        metadata = _metadata(
            _target(["test"], "/elsewhere/tests/outside.rs"),
        )
        with contextlib.redirect_stderr(io.StringIO()):
            paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertIn("/elsewhere/tests/outside.rs", paths)

    def test_prefix_mismatch_emits_diagnostic(self) -> None:
        # bd raikiri-spike-0gk8 debt-lens fix 2: repo_root (git's
        # toplevel, uncanonicalized) and a target's src_path (from `cargo
        # metadata`, canonicalized) disagreeing on prefix — e.g. a
        # symlinked checkout or container bind-mount — would otherwise
        # make every src_path.startswith(prefix) fail simultaneously,
        # silently reverting every test/example target to gating
        # `uncovered`. A warning naming the mismatch count must be printed
        # to stderr so this is diagnosable instead of a second silent
        # failure mode reintroduced through the cargo-metadata coupling
        # this task added.
        metadata = _metadata(
            _target(
                ["test"],
                "/different/prefix/crates/raikiri/tests/parse_html_limits.rs",
            ),
        )
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertIn(
            "/different/prefix/crates/raikiri/tests/parse_html_limits.rs", paths
        )
        warning = stderr.getvalue()
        self.assertIn("WARNING", warning)
        self.assertIn("1/1", warning)
        self.assertIn(REPO_ROOT, warning)

    def test_no_diagnostic_when_prefixes_match(self) -> None:
        # Negative case: no mismatch, no warning noise.
        metadata = _metadata(
            _target(["test"], f"{REPO_ROOT}/crates/raikiri/tests/parse_html_limits.rs"),
        )
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertEqual(stderr.getvalue(), "")

    def test_multiple_packages_are_all_scanned(self) -> None:
        metadata = {
            "packages": [
                {"targets": [_target(["test"], f"{REPO_ROOT}/crates/a/tests/one.rs")]},
                {"targets": [_target(["test"], f"{REPO_ROOT}/crates/b/tests/two.rs")]},
            ]
        }
        paths = structurally_unreported_paths(metadata, REPO_ROOT)
        self.assertEqual(
            paths,
            {"crates/a/tests/one.rs", "crates/b/tests/two.rs"},
        )

    def test_empty_metadata_yields_empty_set(self) -> None:
        self.assertEqual(structurally_unreported_paths({"packages": []}, REPO_ROOT), set())


class IsStructurallyUnreportedTests(unittest.TestCase):
    def setUp(self) -> None:
        self.unreported_paths = {
            "crates/raikiri/tests/parse_html_limits.rs",
            "crates/raikiri/tests/external_consumer.rs",
        }

    def test_known_test_target_is_unreported(self) -> None:
        self.assertTrue(
            is_structurally_unreported(
                "crates/raikiri/tests/parse_html_limits.rs", self.unreported_paths
            )
        )

    def test_src_tests_module_is_not_unreported(self) -> None:
        # The exact false-negative this task fixes: with the old bare
        # `(^|/)(tests|examples)/` regex this path would have matched and
        # been silently exempted from the coverage gate even though it is
        # ordinary production code.
        self.assertFalse(
            is_structurally_unreported(
                "crates/raikiri/src/tests/fixtures.rs", self.unreported_paths
            )
        )

    def test_bench_file_is_not_unreported(self) -> None:
        self.assertFalse(
            is_structurally_unreported(
                "crates/raikiri-style/benches/cascade.rs", self.unreported_paths
            )
        )

    def test_unrelated_src_file_is_not_unreported(self) -> None:
        self.assertFalse(
            is_structurally_unreported(
                "crates/raikiri/src/lib.rs", self.unreported_paths
            )
        )


class ClassifyNoLcovRecordLinesTests(unittest.TestCase):
    """Regression tests for the whole-file-zero-SF-record false positive: a
    "pure declaration" file (only `use` statements, struct/enum
    definitions, doc comments — no executable statements anywhere, e.g.
    crates/raikiri-html/src/types.rs) gets zero `SF:`/`DA:` records from
    cargo-llvm-cov, so `main()`'s `da_map is None` branch used to check
    every added line — including a `///` doc-comment prose edit — directly
    against `exempt`, with no equivalent of the normal (`da_map is not
    None`) branch's `if ln not in da_map: continue` pre-filter that
    already lets comment/blank lines through there. A diff that only
    touched doc-comment prose was therefore reported 100% uncovered, and
    `// cov:ignore:` provided no escape hatch — it only ever exempts the
    *code* block following a marker, never the marker's own comment lines
    or unrelated comment lines elsewhere in the file.

    `classify_no_lcov_record_lines()` is the function that now does what
    the `if ln not in da_map: continue` check does implicitly in the
    normal branch, using `COMMENT_LINE_RE` (the same regex
    `compute_exempt_lines()` uses to recognize a comment line) plus a
    blank check, so it is tested directly here rather than through
    `main()` — same approach the rest of this file takes for
    `structurally_unreported_paths()`/`is_structurally_unreported()`
    above, avoiding a real `git`/`cargo` invocation.
    """

    def setUp(self) -> None:
        # A synthetic stand-in for a "pure declaration" file: a `use`
        # statement, a blank line, doc comments (one `///`, one bare
        # `//`), and a struct with a doc'd field. No line here is
        # executable Rust — consistent with a file cargo-llvm-cov would
        # give zero SF:/DA: records to.
        self.file_lines = [
            "use std::fmt;",                                     # 1
            "",                                                    # 2
            "/// Represents a parsed HTML tag name.",              # 3
            "//",                                                   # 4
            "/// Second line of prose, edited independently.",      # 5
            "pub struct TagName {",                                  # 6
            "    /// The raw tag string.",                            # 7
            "    pub raw: String,",                                    # 8
            "}",                                                         # 9
        ]

    def test_comment_only_diff_is_not_uncovered(self) -> None:
        # The exact reported bug: a diff touching only `///`/`//` prose
        # lines in a zero-SF-record file must not be flagged uncovered.
        uncovered, exempted = classify_no_lcov_record_lines(
            added_lines=[3, 4, 5], file_lines=self.file_lines, exempt=set()
        )
        self.assertEqual(uncovered, [])
        self.assertEqual(exempted, [])

    def test_blank_added_line_is_not_uncovered(self) -> None:
        # Same reasoning applies to a blank added line: it never has a
        # DA: record in the normal branch either, so it must not be
        # treated as uncovered here just because there's no DA: table to
        # filter it out with directly.
        uncovered, exempted = classify_no_lcov_record_lines(
            added_lines=[2], file_lines=self.file_lines, exempt=set()
        )
        self.assertEqual(uncovered, [])
        self.assertEqual(exempted, [])

    def test_real_code_line_is_still_uncovered(self) -> None:
        # Discrimination check: mixing a comment line (3) with a genuine
        # code line (6, `pub struct TagName {`) in the same diff must
        # still flag the code line. Without this test,
        # test_comment_only_diff_is_not_uncovered alone would also pass
        # if the whole branch were changed to route every line to "not
        # uncovered" regardless of content — this pins that the fix
        # discriminates rather than blanket-passing. This is also the
        # case that keeps bd raikiri-spike-iebo's bench-file behavior
        # intact: a genuine added code line in a file with no lcov
        # records (bench or otherwise) is still a real gap.
        uncovered, exempted = classify_no_lcov_record_lines(
            added_lines=[3, 6], file_lines=self.file_lines, exempt=set()
        )
        self.assertEqual(uncovered, [6])
        self.assertEqual(exempted, [])

    def test_cov_ignore_exempted_code_line_still_exempted(self) -> None:
        # cov:ignore-driven exemption (computed by compute_exempt_lines()
        # elsewhere and passed in here as `exempt`) must keep working
        # alongside the new comment/blank filtering, not be bypassed or
        # shadowed by it.
        uncovered, exempted = classify_no_lcov_record_lines(
            added_lines=[3, 6], file_lines=self.file_lines, exempt={6}
        )
        self.assertEqual(uncovered, [])
        self.assertEqual(exempted, [6])


if __name__ == "__main__":
    unittest.main()
