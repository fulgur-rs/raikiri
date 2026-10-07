#!/usr/bin/env python3
"""scripts/lib/test_patch_coverage.py — unit tests for patch_coverage.py.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v

Focused on `structurally_unreported_paths()` / `is_structurally_unreported()`
(the earlier change): the prior implementation classified a changed line
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

Also covers a second, unrelated regression: `code_only()` (and the
`comment_part()` / `compute_exempt_lines()` machinery built on it)
misparsing Rust lifetime apostrophes (`'i`, `'t`, `'a`, ...) as the start of
an unterminated char literal, which silently dropped every character after
the lifetime on that line and, for `compute_exempt_lines()`, corrupted the
brace-depth tracking that decides how far a `// cov:ignore:` block extends.
Reproduced for real in `crates/raikiri-style/src/counter_style.rs`'s
`QualifiedRuleParser::parse_block`, whose `Parser<'i, 't>` /
`ParseError<'i, Self::Error>` signature truncated an intended 8-line
exempt block down to 1 line. `CodeOnlyLifetimeTests` covers `code_only()`
directly; `ComputeExemptLinesLifetimeTests` mirrors the `counter_style.rs`
shape end-to-end through `compute_exempt_lines()`.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import patch_coverage

from patch_coverage import (
    MOVED_SGR,
    classify_no_lcov_record_lines,
    collect_moved_added_lines,
    code_only,
    compute_exempt_lines,
    cov_ignore_reason,
    is_structurally_unreported,
    parse_moved_added_lines,
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
        # Check: the real test target from the same fixture is still
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
        # the earlier coverage fix 2: repo_root (git's
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

    def test_src_tests_rs_module_file_is_unreported(self) -> None:
        # A `#[cfg(test)] mod tests;` file (e.g. `cascade/inherit/tests.rs`)
        # matches cargo-llvm-cov's default --ignore-filename-regex, so it
        # never gets an SF: record even though its tests ran.
        for path in (
            "crates/raikiri-style/src/cascade/inherit/tests.rs",
            "crates/raikiri-style/src/property/tests.rs",
            "crates/raikiri/src/tests.rs",
            "crates/raikiri/src/cascade_tests.rs",
            "crates/raikiri/src/cascade-tests.rs",
        ):
            with self.subTest(path=path):
                self.assertTrue(is_structurally_unreported(path, self.unreported_paths))

    def test_tests_rs_lookalike_names_are_not_unreported(self) -> None:
        for path in (
            "crates/raikiri/src/contests.rs",
            "crates/raikiri/src/tests_util.rs",
            "crates/raikiri/src/tests.rs.bak",
        ):
            with self.subTest(path=path):
                self.assertFalse(is_structurally_unreported(path, self.unreported_paths))


class ParseMovedAddedLinesTests(unittest.TestCase):
    def test_only_moved_colored_added_lines_are_reported(self) -> None:
        green = "\x1b[32m"
        reset = "\x1b[m"
        diff = "\n".join(
            [
                "\x1b[1mdiff --git a/src/b.rs b/src/b.rs\x1b[m",
                "\x1b[1m+++ b/src/b.rs\x1b[m",
                "\x1b[36m@@ -0,0 +10,3 @@\x1b[m",
                f"{MOVED_SGR}+fn moved() {{{reset}",
                f"{green}+fn fresh() {{}}{reset}",
                f"{MOVED_SGR}+}}{reset}",
                "\x1b[1m+++ /dev/null\x1b[m",
                "\x1b[36m@@ -1,2 +0,0 @@\x1b[m",
                "\x1b[31m-gone\x1b[m",
            ]
        )
        self.assertEqual(parse_moved_added_lines(diff), {"src/b.rs": {10, 12}})


class CollectMovedAddedLinesGitTests(unittest.TestCase):
    """Runs real `git` so the forced color config is exercised end to end."""

    def _git(self, repo: str, *args: str) -> str:
        env = {
            **os.environ,
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_SYSTEM": os.devnull,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.com",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.com",
        }
        return subprocess.run(
            ["git", "-C", repo, *args], check=True, capture_output=True, text=True, env=env
        ).stdout.strip()

    def test_block_moved_to_another_file_is_moved_and_new_code_is_not(self) -> None:
        body = [f"    let value_{i} = compute_something_long({i});" for i in range(8)]
        block = ["fn moved_function() {", *body, "}"]
        with tempfile.TemporaryDirectory() as repo:
            self._git(repo, "init", "-q")
            with open(os.path.join(repo, "a.rs"), "w") as f:
                f.write("\n".join(["fn keep() {}", *block, ""]))
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "base")
            base = self._git(repo, "rev-parse", "HEAD")
            with open(os.path.join(repo, "a.rs"), "w") as f:
                f.write("fn keep() {}\n")
            with open(os.path.join(repo, "b.rs"), "w") as f:
                f.write("\n".join(["fn brand_new_function_here() { unique_body(); }", *block, ""]))
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "move")
            moved = collect_moved_added_lines(repo, base, "HEAD")
        self.assertEqual(moved.get("b.rs"), set(range(2, 2 + len(block))))

    def _classify(self, before: str, after: str, missed: str,
                  reported: bool = True, destination: str = "lib.rs",
                  copy: bool = False, origin: str = "lib.rs",
                  auxiliary_before: dict[str, str] | None = None,
                  auxiliary_after: dict[str, str] | None = None,
                  target_root: str = "lib.rs", extra_targets: tuple = (),
                  covered_auxiliary: bool = False, extra_packages: tuple = ()) -> tuple[int, str]:
        with tempfile.TemporaryDirectory() as repo:
            # Ordinary file moves preserve the public module/target. Use
            # a stable crate root and relocate its `component` module.
            if (origin == "lib.rs" and destination != origin and target_root == "lib.rs"
                    and not any(name.endswith(".rs") for name in
                                (* (auxiliary_before or {}), * (auxiliary_after or {})))):
                target_root = "fixture.rs"
                auxiliary_before = {**(auxiliary_before or {}), target_root:
                                    f"#[path={json.dumps(origin)}]\nmod component;\n"}
                auxiliary_after = {**(auxiliary_after or {}), target_root:
                                   f"#[path={json.dumps(destination)}]\nmod component;\n"}
            self._git(repo, "init", "-q")
            def write_auxiliary(files: dict[str, str] | None) -> None:
                for name, content in (files or {}).items():
                    auxiliary = os.path.join(repo, name)
                    os.makedirs(os.path.dirname(auxiliary), exist_ok=True)
                    with open(auxiliary, "w") as f:
                        f.write(content)

            path = os.path.join(repo, origin)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(before)
            write_auxiliary(auxiliary_before)
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "base")
            base = self._git(repo, "rev-parse", "HEAD")
            if destination != origin:
                if not copy:
                    os.unlink(path)
                path = os.path.join(repo, destination)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(after)
            write_auxiliary(auxiliary_after)
            self._git(repo, "add", ".")
            self._git(repo, "commit", "-qm", "change")
            missed_line = after.splitlines().index(missed) + 1
            lcov = os.path.join(repo, "coverage.info")
            with open(lcov, "w") as f:
                if reported:
                    f.write(f"SF:{path}\nDA:{missed_line},0\nend_of_record\n")
                if covered_auxiliary:
                    for name, content in (auxiliary_after or {}).items():
                        if name.endswith(".rs"):
                            f.write(f"SF:{os.path.join(repo, name)}\n")
                            for line in range(1, len(content.splitlines()) + 1):
                                f.write(f"DA:{line},1\n")
                            f.write("end_of_record\n")
            output = io.StringIO()
            # Supply Cargo targets; Git parsing and LCOV classification stay real.
            targets = [_target(["lib"], os.path.join(repo, target_root))]
            targets.extend(_target([kind], os.path.join(repo, name)) for kind, name in extra_targets)
            for target in targets:
                target["name"] = os.path.basename(target["src_path"]).removesuffix(".rs")
            metadata = {"packages": [{"name": "fixture", "manifest_path": os.path.join(repo, "Cargo.toml"),
                                      "targets": targets}]}
            for manifest, root, kind in extra_packages:
                target = _target([kind], os.path.join(repo, root))
                target["name"] = os.path.dirname(manifest)
                metadata["packages"].append({"name": target["name"], "manifest_path": os.path.join(repo, manifest),
                                             "targets": [target]})
            with mock.patch.object(patch_coverage, "load_cargo_metadata", return_value=metadata), \
                 mock.patch("sys.argv", ["patch_coverage.py", "--repo-root", repo,
                                        "--base", base, "--lcov", lcov]), \
                 contextlib.redirect_stdout(output):
                status = patch_coverage.main()
            return status, output.getvalue()

    def test_match_pattern_reused_as_fallback_expression_is_uncovered(self) -> None:
        before = """fn resolve(value: Option<PropertyValue>) -> Option<PropertyValue> {
    match value {
        Some(PropertyValue::Deferred(marker))
            if marker.value.trim().eq_ignore_ascii_case("inherit") => None,
        value => value,
    }
}
"""
        after = """fn resolve(value: Option<PropertyValue>) -> Option<PropertyValue> {
    match value {
        Some(PropertyValue::Deferred(marker)) => {
            if marker.css_wide_keyword().is_some() {
                None
            } else {
                Some(PropertyValue::Deferred(marker))
            }
        }
        value => value,
    }
}
"""
        for reported in (True, False):
            with self.subTest(reported=reported):
                status, output = self._classify(
                    before, after, "                Some(PropertyValue::Deferred(marker))", reported
                )
                self.assertEqual(status, 1, output)
                self.assertIn("  lib.rs:7", output)

    def test_unchanged_statement_under_changed_condition_is_uncovered(self) -> None:
        before = """fn resolve(enabled: bool) {
    if enabled {
        record_uncovered_resolution_result();
    }
}
"""
        after = before.replace("if enabled", "if !enabled")
        # Relocation makes Git see the unchanged body as added/deleted.
        after = "fn new_function() {}\n" + after.replace("    ", "        ")
        status, output = self._classify(
            before, after, "                record_uncovered_resolution_result();"
        )
        self.assertEqual(status, 1, output)

    def test_unchanged_function_with_indentation_change_remains_exempt(self) -> None:
        before = """mod extracted {
fn resolve() {
    record_uncovered_resolution_result();
}
}
fn keep() {}
"""
        after = """fn keep() {}
mod extracted {
    fn resolve() {
        record_uncovered_resolution_result();
    }
}
"""
        status, output = self._classify(
            before, after, "        record_uncovered_resolution_result();"
        )
        self.assertEqual(status, 0, output)
        self.assertIn("informational): 1", output)

    def test_complete_function_move_without_lcov_record_remains_exempt(self) -> None:
        source = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            source, source, "    record_uncovered_resolution_result();",
            reported=False, destination="moved.rs"
        )
        self.assertEqual(status, 0, output)
        self.assertIn("total uncovered added lines (FAIL if > 0): 0", output)

    def test_copied_function_is_not_a_move_exemption(self) -> None:
        source = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            source, source, "    record_uncovered_resolution_result();",
            destination="copied.rs", copy=True
        )
        self.assertEqual(status, 1, output)

    def test_changed_attributes_and_impl_context_are_not_exempt(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        cases = (
            ('#[cfg(feature = "old")]\n' + body, '#[cfg(feature = "new")]\n' + body),
            ('#[cfg(feature = "old")]\nmod gated {\n' + body + "}\n",
             '#[cfg(feature = "new")]\nmod gated {\n' + body + "}\n"),
            ("impl Old {\n" + body + "}\n", "impl New {\n" + body + "}\n"),
        )
        for before, after in cases:
            with self.subTest(before=before):
                status, output = self._classify(
                    before, after, "    record_uncovered_resolution_result();",
                    destination="moved.rs"
                )
                self.assertEqual(status, 1, output)

    def test_literals_comments_and_lifetimes_do_not_break_valid_move(self) -> None:
        source = '''fn resolve<'a>(value: &'a str) {
    /* outer { /* nested } */ } */
    let brace = '}';
    let text = r##"fn fake() { "quoted" }
    /* not a comment */"##;
    let continued = "one \\
        } two";
    record_uncovered_resolution_result();
}
'''
        status, output = self._classify(
            source, source, "    record_uncovered_resolution_result();",
            destination="moved.rs"
        )
        self.assertEqual(status, 0, output)

    def test_changed_literal_or_operator_does_not_exempt_relocated_function(self) -> None:
        source = '''fn resolve(a: bool, b: bool) {
    let text = r#"old { }"#;
    let both = a && b;
    record_uncovered_resolution_result();
}
'''
        for after in (source.replace("old", "new"), source.replace("&&", "& &")):
            with self.subTest(after=after):
                status, output = self._classify(
                    source, after, "    record_uncovered_resolution_result();",
                    destination="moved.rs"
                )
                self.assertEqual(status, 1, output)

    def test_inner_attributes_apply_to_every_function_in_the_scope(self) -> None:
        body = "fn keep() {}\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        for wrapper in ("{}", "mod gated {{\n{}\n}}\n"):
            before = wrapper.format('#![cfg(feature = "old")]\n' + body)
            after = wrapper.format('#![cfg(feature = "new")]\n' + body)
            with self.subTest(wrapper=wrapper):
                status, output = self._classify(
                    before, after, "    record_uncovered_resolution_result();",
                    destination="moved.rs"
                )
                self.assertEqual(status, 1, output)

    def test_one_deleted_function_cannot_exempt_two_destinations(self) -> None:
        before = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        after = """mod first {
    fn resolve() {
        record_uncovered_resolution_result();
    }
}
mod second {
        fn resolve() {
            record_uncovered_resolution_result();
        }
}
"""
        status, output = self._classify(
            before, after, "        record_uncovered_resolution_result();",
            destination="moved.rs"
        )
        self.assertEqual(status, 1, output)

    def test_multiple_identical_functions_can_move_without_becoming_copies(self) -> None:
        source = """#![allow(dead_code)]
mod first {
    fn resolve() {
        record_uncovered_resolution_result();
    }
}
mod second {
    fn resolve() {
        record_uncovered_resolution_result();
    }
}
"""
        status, output = self._classify(
            source, source, "        record_uncovered_resolution_result();",
            destination="moved.rs"
        )
        self.assertEqual(status, 0, output)

    def test_const_generic_braces_do_not_hide_a_changed_body(self) -> None:
        before = """fn resolve_uncovered_resolution_result() -> GenericArray<{ 3 }> {
    old_uncovered_resolution_result()
}
"""
        after = before.replace("old_uncovered", "new_uncovered")
        status, output = self._classify(
            before, after, after.splitlines()[0], destination="moved.rs"
        )
        self.assertEqual(status, 1, output)

    def test_complete_const_generic_function_move_remains_exempt(self) -> None:
        source = """fn resolve_uncovered_resolution_result() -> GenericArray<{ 3 }> {
    old_uncovered_resolution_result()
}
"""
        status, output = self._classify(
            source, source, "    old_uncovered_resolution_result()", destination="moved.rs"
        )
        self.assertEqual(status, 0, output)

    def test_external_module_attributes_follow_the_function(self) -> None:
        source = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        for parent in ('#[cfg(feature = "old")]\nmod old;\n',
                       '#![cfg(feature = "old")]\nmod old;\n'):
            with self.subTest(parent=parent):
                status, output = self._classify(
                    source, source, "    record_uncovered_resolution_result();",
                    origin="old.rs", destination="new.rs",
                    auxiliary_before={"lib.rs": parent},
                    auxiliary_after={"lib.rs": "mod new;\n"}
                )
                self.assertEqual(status, 1, output)

    def test_external_module_move_preserving_attributes_remains_exempt(self) -> None:
        source = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            source, source, "    record_uncovered_resolution_result();",
            origin="old.rs", destination="new.rs",
            auxiliary_before={"lib.rs": '#[cfg(feature = "gate")]\n#[path="old.rs"]\nmod component;\n'},
            auxiliary_after={"lib.rs": '#[cfg(feature = "gate")]\n#[path="new.rs"]\nmod component;\n'}
        )
        self.assertEqual(status, 0, output)

    def test_nested_and_custom_path_module_attributes_are_not_lost(self) -> None:
        source = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        cases = (
            ("gated/old.rs", "lib.rs", {"lib.rs": '#[cfg(feature="old")] mod gated;\n',
                                        "gated/mod.rs": "mod old;\n"}),
            ("gated/old.rs", "lib.rs", {"lib.rs": '#[cfg(feature="old")] mod gated { mod old; }\n'}),
            ("shared/old.rs", "src/lib.rs",
             {"src/lib.rs": '#[path="../shared/old.rs"] #[cfg(feature="old")] mod old;\n'}),
            ("old.rs", "entry.rs", {"entry.rs": '#[cfg(feature="old")] mod old;\n'}),
            ("old.rs", "lib.rs",
             {"lib.rs": '#[cfg_attr(all(), path="old.rs")] #[cfg(feature="old")] mod gated;\n'}),
            ("old.rs", "lib.rs",
             {"lib.rs": '#[cfg_attr(all(), path="old.rs")] #[path="fallback.rs"] #[cfg(feature="old")] mod gated;\n'}),
        )
        for origin, root, auxiliary in cases:
            with self.subTest(origin=origin, root=root):
                status, output = self._classify(
                    source, source, "    record_uncovered_resolution_result();",
                    origin=origin, destination="new.rs", target_root=root,
                    auxiliary_before=auxiliary, auxiliary_after={root: "mod new;\n"}
                )
                self.assertEqual(status, 1, output)

    def test_parent_scope_attributes_can_follow_function_extraction(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        for before, parent in (
            ("#![allow(dead_code)]\nmod moved {\n" + body + "}\n", "#![allow(dead_code)]\nmod moved;\n"),
            ('#![allow(dead_code)]\n#[cfg(feature="gate")] mod moved {\n' + body + "}\n",
             '#![allow(dead_code)]\n#[cfg(feature="gate")]\nmod moved;\n'),
            ('#[cfg(feature="gate")] mod moved {\n' + body + "}\n",
             '#[cfg(feature="gate")]\nmod moved;\n'),
        ):
            with self.subTest(parent=parent):
                status, output = self._classify(
                    before, body, "    record_uncovered_resolution_result();",
                    destination="moved.rs", auxiliary_after={"lib.rs": parent}
                )
                self.assertEqual(status, 0, output)

    def test_changed_cargo_target_root_does_not_exempt_moved_function(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            body, body, "    record_uncovered_resolution_result();",
            origin="old.rs", destination="new.rs", target_root="new_entry.rs",
            auxiliary_before={"Cargo.toml": '[package]\nname="fixture"\nversion="0.1.0"\n[lib]\npath="old_entry.rs"\n',
                              "old_entry.rs": '#[cfg(feature="old")] mod old;\n'},
            auxiliary_after={"Cargo.toml": '[package]\nname="fixture"\nversion="0.1.0"\n[lib]\npath="new_entry.rs"\n',
                             "new_entry.rs": "mod new;\n"}
        )
        self.assertEqual(status, 1, output)

    def test_dependency_edit_leaves_old_compilation_context_unproven(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\n[dependencies]\nnoop="0.1"\n'
        status, output = self._classify(
            body, body, "    record_uncovered_resolution_result();",
            destination="moved.rs", auxiliary_before={"Cargo.toml": manifest},
            auxiliary_after={"Cargo.toml": manifest.replace('noop="0.1"', 'noop="0.2"')}
        )
        self.assertEqual(status, 1, output)

    def test_function_reorder_with_shared_statement_remains_exempt(self) -> None:
        moved = "fn moved_uncovered_resolution_result() {\n    record_uncovered_resolution_result();\n}\n"
        kept = moved.replace("moved_uncovered", "kept_uncovered")
        status, output = self._classify(moved + kept, kept + moved, moved.splitlines()[0])
        self.assertEqual(status, 0, output)

    def test_cross_target_function_move_is_not_exempt(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        for kind, folder in (("test", "tests"), ("example", "examples"), ("bench", "benches")):
            origin = folder + "/helper.rs"
            with self.subTest(kind=kind):
                status, output = self._classify(
                    body, body, "    record_uncovered_resolution_result();",
                    origin=origin, destination="src/lib.rs", target_root="src/lib.rs",
                    extra_targets=((kind, origin),)
                )
                self.assertEqual(status, 1, output)

    def test_moving_to_another_logical_module_is_not_exempt(self) -> None:
        before = """mod old {
const VALUE: u8 = 1;
fn resolve() {
    record_uncovered_resolution_result(VALUE);
}
}
"""
        after = before.replace("mod old", "mod new").replace("= 1", "= 2").replace("    record", "        record")
        status, output = self._classify(before, after, "        record_uncovered_resolution_result(VALUE);")
        self.assertEqual(status, 1, output)

    def test_binding_context_is_preserved_during_inline_module_extraction(self) -> None:
        before = """mod resolver {
use old::VALUE;
fn resolve() {
    record_uncovered_resolution_result(VALUE);
}
}
"""
        body = "use old::VALUE;\nfn resolve() {\n    record_uncovered_resolution_result(VALUE);\n}\n"
        for binding in ("old", "new"):
            with self.subTest(binding=binding):
                status, output = self._classify(
                    before, body.replace("old", binding), "    record_uncovered_resolution_result(VALUE);",
                    destination="resolver.rs", auxiliary_after={"lib.rs": "mod resolver;\n"}
                )
                self.assertEqual(status, 0 if binding == "old" else 1, output)

    def test_nested_lib_filename_does_not_make_a_cargo_root(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            body, body, "    record_uncovered_resolution_result();",
            origin="outer/lib/child.rs", destination="new.rs",
            auxiliary_before={"lib.rs": "mod outer;\n", "outer/mod.rs": "mod lib;\n",
                              "outer/lib.rs": '#[cfg(feature="old")]\nmod child;\n'},
            auxiliary_after={"lib.rs": "mod new;\n"}
        )
        self.assertEqual(status, 1, output)

    def test_cargo_feature_activation_change_does_not_exempt_function(self) -> None:
        body = '#[cfg(feature="gate")]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n'
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\n[features]\ngate=[]\ndefault=[]\n'
        status, output = self._classify(
            body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
            "        record_uncovered_resolution_result();",
            auxiliary_before={"Cargo.toml": manifest},
            auxiliary_after={"Cargo.toml": manifest.replace("default=[]", 'default=["gate"]')}
        )
        self.assertEqual(status, 1, output)

    def test_constants_aliases_and_macros_keep_their_binding_during_extraction(self) -> None:
        cases = (
            ("const VALUE: u8 = 1;", "const VALUE: u8 = 2;", "VALUE"),
            ("type Value = OldValue;", "type Value = NewValue;", "Value::default()"),
            ("macro_rules! value { () => { 1 } }", "macro_rules! value { () => { 2 } }", "value!()"),
            ("use old::*;", "use new::*;", "VALUE"),
        )
        for original, changed, expression in cases:
            body = f"fn resolve() {{\n    record_uncovered_resolution_result({expression});\n}}\n"
            for binding in (original, changed):
                with self.subTest(original=original, binding=binding):
                    status, output = self._classify(
                        "mod resolver {\n" + original + "\n" + body + "}\n",
                        binding + "\n" + body, body.splitlines()[1],
                        destination="resolver.rs", auxiliary_after={"lib.rs": "mod resolver;\n"}
                    )
                    self.assertEqual(status, 0 if binding == original else 1, output)

    def test_macro_shadowing_order_changes_are_not_move_exemptions(self) -> None:
        definition = "macro_rules! value { () => { 1 } }\n"
        shadow = definition.replace("1", "2")
        body = "fn resolve() {\n    record_uncovered_resolution_result(value!());\n}\n"
        status, output = self._classify(
            definition + shadow + body, shadow + definition + body,
            body.splitlines()[1], destination="moved.rs"
        )
        self.assertEqual(status, 1, output)

    def test_unrelated_sibling_edit_preserves_the_public_function_path(self) -> None:
        before = """mod unrelated { const OTHER: u8 = 1; }
mod resolver {
const VALUE: u8 = 7;
fn resolve() {
    record_uncovered_resolution_result(VALUE);
}
}
pub fn call() { crate::resolver::resolve(); }
"""
        body = "const VALUE: u8 = 7;\nfn resolve() {\n    record_uncovered_resolution_result(VALUE);\n}\n"
        for value in ("7", "8"):
            with self.subTest(value=value):
                status, output = self._classify(
                    before, body.replace("= 7", "= " + value), body.splitlines()[2],
                    destination="resolver.rs", auxiliary_after={"lib.rs":
                        "mod unrelated { const OTHER: u8 = 2; }\nmod resolver;\n"
                        "pub fn call() { crate::resolver::resolve(); }\n"}, covered_auxiliary=True
                )
                self.assertEqual(status, 0 if value == "7" else 1, output)

    def test_referenced_module_attributes_are_part_of_the_binding_context(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result(crate::shared::VALUE);\n}\n"
        shared = "pub const VALUE: u8 = 7;\n"
        declaration = '#[cfg(feature="old")]\nmod shared;\n'
        for feature in ("old", "new"):
            with self.subTest(feature=feature):
                status, output = self._classify(
                    declaration + "mod resolver {\n" + body + "}\n", body, body.splitlines()[1],
                    destination="resolver.rs", auxiliary_before={"shared.rs": shared},
                    auxiliary_after={"lib.rs": declaration.replace('"old"', f'"{feature}"') + "mod resolver;\n",
                                     "shared.rs": shared}, covered_auxiliary=True
                )
                self.assertEqual(status, 0 if feature == "old" else 1, output)

    def test_nested_lib_child_can_move_preserving_module_identity(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        graph = {"lib.rs": "mod outer;\n", "outer/mod.rs": "mod lib;\n",
                 "outer/lib.rs": '#[cfg(feature="gate")]\nmod child;\n'}
        status, output = self._classify(
            body, body, body.splitlines()[1], origin="outer/lib/child.rs",
            destination="outer/lib/child/mod.rs", auxiliary_before=graph, auxiliary_after=graph
        )
        self.assertEqual(status, 0, output)

    def test_nested_function_scope_bindings_cannot_receive_move_exemptions(self) -> None:
        for declaration, expression in (("const VALUE: u8 = 1;", "VALUE"),
                                        ("macro_rules! value { () => { 1 } }", "value!()")):
            source = f"""fn outer() {{
    {declaration}
    fn resolve() {{
        record_uncovered_resolution_result({expression});
    }}
    resolve();
}}
"""
            for after in (source, source.replace("1", "2")):
                with self.subTest(declaration=declaration, unchanged=after == source):
                    status, output = self._classify(
                        source, after, source.splitlines()[3], destination="moved.rs"
                    )
                    self.assertEqual(status, 0 if after == source else 1, output)

    def test_complete_impl_method_move_preserves_its_context(self) -> None:
        source = """impl Resolver {
    fn resolve() {
        record_uncovered_resolution_result();
    }
}
"""
        status, output = self._classify(
            source, source, source.splitlines()[2], destination="moved.rs"
        )
        self.assertEqual(status, 0, output)

    def test_compilation_manifest_changes_do_not_exempt_relocated_functions(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result([1, 2].into_iter());\n}\n"
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\nedition="2018"\n'
        cases = ((manifest, manifest.replace('"2018"', '"2021"')),
                 (manifest + '[dependencies]\nother={version="0.1",features=[]}\n',
                  manifest + '[dependencies]\nother={version="0.1",features=["gate"]}\n'),
                 (manifest + '[workspace.dependencies]\nother={version="0.1",default-features=false}\n',
                  manifest + '[workspace.dependencies]\nother={version="0.1",default-features=true}\n'))
        for before, after in cases:
            with self.subTest(after=after):
                status, output = self._classify(
                    body, body, body.splitlines()[1], destination="moved.rs",
                    auxiliary_before={"Cargo.toml": before}, auxiliary_after={"Cargo.toml": after}
                )
                self.assertEqual(status, 1, output)

    def test_trait_import_changes_affect_implicit_method_resolution(self) -> None:
        body = "fn resolve(value: Value) {\n    record_uncovered_resolution_result(value.resolve());\n}\n"
        for alias in ("", " as _"):
            for imported in ("old", "new"):
                with self.subTest(alias=alias, imported=imported):
                    status, output = self._classify(
                        "mod resolver {\nuse old::Resolve" + alias + ";\n" + body + "}\n",
                        "use " + imported + "::Resolve" + alias + ";\n" + body, body.splitlines()[1],
                        destination="resolver.rs", auxiliary_after={"lib.rs": "mod resolver;\n"}
                    )
                    self.assertEqual(status, 0 if imported == "old" else 1, output)

    def test_include_inputs_cannot_silently_change_move_context(self) -> None:
        body = 'include!("bindings.rs");\nfn resolve() {\n    record_uncovered_resolution_result(VALUE);\n}\n'
        status, output = self._classify(
            body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
            "        record_uncovered_resolution_result(VALUE);",
            auxiliary_before={"bindings.rs": "const VALUE: u8 = 1;\n"},
            auxiliary_after={"bindings.rs": "const VALUE: u8 = 2;\n"}, covered_auxiliary=True
        )
        self.assertEqual(status, 1, output)

    def test_unrelated_include_does_not_disable_valid_module_extraction(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            "mod unrelated;\nmod resolver {\n" + body + "}\n", body, body.splitlines()[1],
            destination="resolver.rs", auxiliary_before={"unrelated.rs": 'include!("bindings.rs");\n'},
            auxiliary_after={"lib.rs": "mod unrelated;\nmod resolver;\n",
                             "unrelated.rs": 'include!("bindings.rs");\n'}
        )
        self.assertEqual(status, 0, output)


    def test_include_macro_indirection_inputs_are_not_move_exemptions(self) -> None:
        for expansion in ('use std::include as inject;\ninject!("bindings.rs");\n',
                          'macro_rules! inject { ($m:ident) => { $m!("bindings.rs"); } }\ninject!(include);\n'):
            body = expansion + 'fn resolve() {\n    record_uncovered_resolution_result(VALUE);\n}\n'
            with self.subTest(expansion=expansion):
                status, output = self._classify(
                    body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
                    "        record_uncovered_resolution_result(VALUE);",
                    auxiliary_before={"bindings.rs": "const VALUE: u8 = 1;\n"},
                    auxiliary_after={"bindings.rs": "const VALUE: u8 = 2;\n"}, covered_auxiliary=True
                )
                self.assertEqual(status, 1, output)

    def test_build_script_metadata_is_not_a_dependency_version_edit(self) -> None:
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\nbuild="build.rs"\n[package.metadata.dependencies]\ngate="off"\n'
        build = 'fn main() { let text = std::fs::read_to_string("Cargo.toml").unwrap(); if text.contains("gate=\\\"on\\\"") { println!("cargo:rustc-cfg=custom_gate"); } }\n'
        body = '#[cfg(custom_gate)]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n'
        status, output = self._classify(
            body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
            "        record_uncovered_resolution_result();",
            auxiliary_before={"Cargo.toml": manifest, "build.rs": build},
            auxiliary_after={"Cargo.toml": manifest.replace('"off"', '"on"'), "build.rs": build},
            extra_targets=(("custom-build", "build.rs"),), covered_auxiliary=True
        )
        self.assertEqual(status, 1, output)

    def test_helper_include_inputs_follow_references_without_rejecting_unrelated_moves(self) -> None:
        included = 'fn input() -> &\'static str { include_str!("value.txt") }\n'
        wrapper = "fn wrapper() -> &'static str { input() }\n"
        for called in ("input", "wrapper", "unrelated"):
            body = f"use crate::{{{called}}};\nfn resolve() {{\n    record_uncovered_resolution_result({called}());\n}}\n"
            helper = included + wrapper + "fn unrelated() -> &'static str { \"stable\" }\n"
            with self.subTest(called=called):
                status, output = self._classify(
                    helper + "mod resolver {\n" + body + "}\n", body, body.splitlines()[2],
                    destination="resolver.rs", auxiliary_before={"value.txt": "old"},
                    auxiliary_after={"lib.rs": helper + "mod resolver;\n", "value.txt": "new"},
                    covered_auxiliary=True
                )
                self.assertEqual(status, 0 if called == "unrelated" else 1, output)

    def test_macro_use_module_bindings_follow_extraction(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result(value!());\n}\n"
        helper = "macro_rules! value { () => { 1 } }\n"
        for value in ("1", "2"):
            with self.subTest(value=value):
                status, output = self._classify(
                    "#[macro_use] mod helpers;\nmod resolver {\n" + body + "}\n", body, body.splitlines()[1],
                    destination="resolver.rs", auxiliary_before={"helpers.rs": helper},
                    auxiliary_after={"lib.rs": "#[macro_use] mod helpers;\nmod resolver;\n",
                                     "helpers.rs": helper.replace("1", value)}, covered_auxiliary=True
                )
                self.assertEqual(status, 0 if value == "1" else 1, output)

    def test_macro_use_shadowing_order_survives_module_extraction(self) -> None:
        body = "fn resolve() {\n    record_uncovered_resolution_result(value!());\n}\n"
        first = "#[macro_use] mod old;\n"
        second = "#[macro_use] mod new;\n"
        helpers = {"old.rs": "macro_rules! value { () => { 1 } }\n",
                   "new.rs": "macro_rules! value { () => { 2 } }\n"}
        for imports in (first + second, second + first):
            with self.subTest(unchanged=imports == first + second):
                status, output = self._classify(
                    first + second + "mod resolver {\n" + body + "}\n", body, body.splitlines()[1],
                    destination="resolver.rs", auxiliary_before=helpers,
                    auxiliary_after={**helpers, "lib.rs": imports + "mod resolver;\n"},
                    covered_auxiliary=True
                )
                self.assertEqual(status, 0 if imports == first + second else 1, output)

    def test_literal_suffix_is_part_of_the_macro_token(self) -> None:
        for literal in ('"x"', "'x'", 'b"x"', "b'x'", 'r#"x"#', 'br#"x"#', 'c"x"', 'cr#"x"#'):
            original = literal + "suffix"
            source = "macro_rules! choose { ($value:tt) => { 1 }; ($($value:tt)+) => { 2 }; }\n"
            source += f"fn resolve() {{\n    let value = choose!({original});\n    record_uncovered_resolution_result(value);\n}}\n"
            for after in (source, source.replace(original, literal + " suffix")):
                with self.subTest(literal=literal, unchanged=after == source):
                    status, output = self._classify(source, after, source.splitlines()[3], destination="moved.rs")
                    self.assertEqual(status, 0 if after == source else 1, output)

    def test_doc_comments_are_macro_attribute_tokens(self) -> None:
        for comment in ("/// old", "/** old */", "//! old", "/*! old */"):
            source = 'macro_rules! choose { (#[doc=$text:literal]) => { $text }; (#![doc=$text:literal]) => { $text }; }\n'
            source += "fn resolve() {\n    let value = choose!(\n" + comment + "\n);\n    record_uncovered_resolution_result(value);\n}\n"
            for after in (source, source.replace("old", "new")):
                with self.subTest(comment=comment, unchanged=after == source):
                    status, output = self._classify(source, after, source.splitlines()[5], destination="moved.rs")
                    self.assertEqual(status, 0 if after == source else 1, output)

    def test_build_script_include_inputs_make_compilation_context_unknown(self) -> None:
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\nbuild="build.rs"\n'
        build = 'fn main() { if include_str!("gate.txt").trim()=="on" { println!("cargo:rustc-cfg=gate"); } }\n'
        body = "#[cfg(gate)]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
            "        record_uncovered_resolution_result();",
            auxiliary_before={"Cargo.toml": manifest, "build.rs": build, "gate.txt": "off"},
            auxiliary_after={"Cargo.toml": manifest, "build.rs": build, "gate.txt": "on"},
            extra_targets=(("custom-build", "build.rs"),), covered_auxiliary=True
        )
        self.assertEqual(status, 1, output)

    def test_raw_include_identifier_does_not_hide_file_inputs(self) -> None:
        body = 'fn resolve() {\n    record_uncovered_resolution_result(r#include_str!("value.txt"));\n}\n'
        status, output = self._classify(
            body, body, body.splitlines()[1], destination="moved.rs",
            auxiliary_before={"value.txt": "old"}, auxiliary_after={"value.txt": "new"}
        )
        self.assertEqual(status, 1, output)

    def test_dependency_resolution_changes_invalidate_moves(self) -> None:
        body = "#[attribute_macro::gate]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\n[dependencies]\nattribute_macro="0.1"\n'
        for name, before, after in (("Cargo.toml", manifest, manifest.replace('"0.1"', '"0.2"')),
                                    ("Cargo.lock", 'version=3\n[[package]]\nname="attribute_macro"\nversion="0.1.0"\n',
                                     'version=3\n[[package]]\nname="attribute_macro"\nversion="0.2.0"\n')):
            with self.subTest(name=name):
                status, output = self._classify(
                    body, body, body.splitlines()[2], destination="moved.rs",
                    auxiliary_before={name: before}, auxiliary_after={name: after}
                )
                self.assertEqual(status, 1, output)

    def test_build_script_runtime_file_inputs_are_not_assumed_unchanged(self) -> None:
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\nbuild="build.rs"\n'
        build = 'fn main() { if std::fs::read_to_string("gate.txt").unwrap().trim()=="on" { println!("cargo:rustc-cfg=gate"); } }\n'
        body = "#[cfg(gate)]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        for name in ("gate.txt", "input.rs"):
            with self.subTest(name=name):
                status, output = self._classify(
                    body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
                    "        record_uncovered_resolution_result();",
                    auxiliary_before={"Cargo.toml": manifest, "build.rs": build.replace("gate.txt", name), name: "off"},
                    auxiliary_after={"Cargo.toml": manifest, "build.rs": build.replace("gate.txt", name), name: "on"},
                    extra_targets=(("custom-build", "build.rs"),), covered_auxiliary=True
                )
                self.assertEqual(status, 1, output)

    def test_referenced_helper_implementation_changes_invalidate_moves(self) -> None:
        helper = "fn helper() -> bool { true }\n"
        unrelated = "fn unrelated() -> bool { true }\n"
        body = "use crate::helper;\nfn resolve() {\n    if helper() {\n        record_uncovered_resolution_result();\n    }\n}\n"
        for changed in ("helper", "unrelated"):
            with self.subTest(changed=changed):
                after_helpers = (helper.replace("true", "false") if changed == "helper" else helper)
                after_helpers += unrelated.replace("true", "false") if changed == "unrelated" else unrelated
                status, output = self._classify(
                    helper + unrelated + "mod resolver {\n" + body + "}\n", body, body.splitlines()[3],
                    destination="resolver.rs", auxiliary_after={"lib.rs": after_helpers + "mod resolver;\n"},
                    covered_auxiliary=True
                )
                self.assertEqual(status, 1 if changed == "helper" else 0, output)

    def test_build_script_can_read_a_modeled_rust_source(self) -> None:
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\nbuild="build.rs"\n'
        build = 'fn main() { if std::fs::read_to_string("lib.rs").unwrap().starts_with("fn keep") { println!("cargo:rustc-cfg=gate"); } }\n'
        body = "#[cfg(gate)]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
            "        record_uncovered_resolution_result();",
            auxiliary_before={"Cargo.toml": manifest, "build.rs": build},
            auxiliary_after={"Cargo.toml": manifest, "build.rs": build},
            extra_targets=(("custom-build", "build.rs"),), covered_auxiliary=True
        )
        self.assertEqual(status, 1, output)

    def test_macro_export_definitions_affect_sibling_callers(self) -> None:
        helper = "#[macro_export]\nmacro_rules! exported { () => { 1 } }\n"
        body = "fn resolve() {\n    record_uncovered_resolution_result(exported!());\n}\n"
        for value in ("1", "2"):
            with self.subTest(value=value):
                status, output = self._classify(
                    "mod helpers;\nmod resolver {\n" + body + "}\n", body, body.splitlines()[1],
                    destination="resolver.rs", auxiliary_before={"helpers.rs": helper},
                    auxiliary_after={"lib.rs": "mod helpers;\nmod resolver;\n",
                                     "helpers.rs": helper.replace("1", value)}, covered_auxiliary=True
                )
                self.assertEqual(status, 0 if value == "1" else 1, output)

    def test_modeled_build_dependency_source_changes_compilation_context(self) -> None:
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\nbuild="build.rs"\n[lib]\npath="lib.rs"\n[workspace]\nmembers=["cfgsupport"]\n[build-dependencies]\ncfgsupport={path="cfgsupport"}\n'
        support_manifest = '[package]\nname="cfgsupport"\nversion="0.1.0"\n[lib]\npath="lib.rs"\n'
        build = 'fn main() { if cfgsupport::enabled() { println!("cargo:rustc-cfg=gate"); } }\n'
        helper = "pub fn enabled() -> bool { false }\n"
        body = "#[cfg(gate)]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        for changed in (False, True):
            with self.subTest(changed=changed):
                files = {"Cargo.toml": manifest, "cfgsupport/Cargo.toml": support_manifest,
                         "build.rs": build, "cfgsupport/lib.rs": helper}
                status, output = self._classify(
                    body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
                    "        record_uncovered_resolution_result();", auxiliary_before=files,
                    auxiliary_after={**files, "cfgsupport/lib.rs": helper.replace("false", "true") if changed else helper},
                    extra_targets=(("custom-build", "build.rs"),), covered_auxiliary=True,
                    extra_packages=(("cfgsupport/Cargo.toml", "cfgsupport/lib.rs", "lib"),)
                )
                self.assertEqual(status, 1 if changed else 0, output)

    def test_location_macro_inputs_do_not_receive_move_exemptions(self) -> None:
        for expansion in ("file!()", "line!()", "column!()", "r#file!()"):
            body = "fn resolve() {\n    record_uncovered_resolution_result(" + expansion + ");\n}\n"
            with self.subTest(expansion=expansion):
                status, output = self._classify(body, body, body.splitlines()[1], destination="moved.rs")
                self.assertEqual(status, 1, output)

    def test_location_macro_indirection_does_not_hide_span_inputs(self) -> None:
        for helper in ('use std::file as location;\n',
                       'macro_rules! location { () => { file!() }; }\n'):
            body = helper + "fn resolve() {\n    record_uncovered_resolution_result(location!());\n}\n"
            with self.subTest(helper=helper):
                status, output = self._classify(body, body, body.splitlines()[2], destination="moved.rs")
                self.assertEqual(status, 1, output)

    def test_location_named_values_do_not_disable_plain_function_moves(self) -> None:
        body = "fn resolve(file: u8, line: u8, column: u8) {\n    record_uncovered_resolution_result(file, line, column);\n}\n"
        status, output = self._classify(body, body, body.splitlines()[1], destination="moved.rs")
        self.assertEqual(status, 0, output)

    def test_cli_can_start_without_tomllib(self) -> None:
        script = os.path.join(os.path.dirname(__file__), "patch_coverage.py")
        command = "import runpy,sys; sys.modules['tomllib']=None; sys.argv=[sys.argv[1],'--help']; runpy.run_path(sys.argv[0],run_name='__main__')"
        proc = subprocess.run([sys.executable, "-c", command, script], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("--lcov", proc.stdout)

    def test_manifest_line_endings_are_compilation_inputs(self) -> None:
        body = "#[attribute_macro::gate]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        manifest = '[package]\r\nname="fixture"\r\nversion="0.1.0"\r\n'
        for after in (manifest, manifest.replace("\r\n", "\n")):
            with self.subTest(unchanged=after == manifest):
                status, output = self._classify(
                    body, body, body.splitlines()[2], destination="moved.rs",
                    auxiliary_before={"Cargo.toml": manifest}, auxiliary_after={"Cargo.toml": after}
                )
                self.assertEqual(status, 0 if after == manifest else 1, output)

    def test_quoted_manifest_paths_do_not_hide_compilation_changes(self) -> None:
        body = "#[attribute_macro::gate]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        manifest = '[package]\nname="attribute_macro"\nversion="0.1.0"\n'
        for folder in ("日本語", "split\nname", 'with"quote'):
            with self.subTest(folder=folder):
                path = folder + "/Cargo.toml"
                status, output = self._classify(
                    body, body, body.splitlines()[2], destination="moved.rs",
                    auxiliary_before={path: manifest}, auxiliary_after={path: manifest.replace("0.1.0", "0.2.0")}
                )
                self.assertEqual(status, 1, output)

    def test_compiler_configuration_changes_do_not_exempt_functions(self) -> None:
        body = "#[cfg(gate)]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        for name, before, after in (
            (".cargo/config.toml", '[build]\nrustflags=["--check-cfg=cfg(gate)"]\n',
             '[build]\nrustflags=["--check-cfg=cfg(gate)","--cfg=gate"]\n'),
            (".cargo/config", '[build]\nrustflags=[]\n', '[build]\nrustflags=["--cfg=gate"]\n'),
            ("rust-toolchain.toml", '[toolchain]\nchannel="1.90"\n', '[toolchain]\nchannel="1.91"\n'),
        ):
            with self.subTest(name=name):
                status, output = self._classify(
                    body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
                    "        record_uncovered_resolution_result();",
                    auxiliary_before={name: before}, auxiliary_after={name: after}
                )
                self.assertEqual(status, 1, output)

    def test_literal_and_raw_identifier_tokens_are_not_whitespace_equivalent(self) -> None:
        for original, changed in (('b"x"', 'b "x"'), ("b'x'", "b 'x'"),
                                  ('c"x"', 'c "x"'), ("1.0", "1 . 0"), ("r#value", "r # value")):
            source = "macro_rules! choose { ($value:tt) => { 1 }; ($($value:tt)+) => { 2 }; }\n"
            source += f"fn resolve() {{\n    let value = choose!({original});\n    record_uncovered_resolution_result(value);\n}}\n"
            for after in (source, source.replace(original, changed)):
                with self.subTest(original=original, unchanged=after == source):
                    status, output = self._classify(
                        source, after, source.splitlines()[3], destination="moved.rs"
                    )
                    self.assertEqual(status, 0 if after == source else 1, output)

    def test_build_script_source_change_invalidates_old_compilation_context(self) -> None:
        manifest = '[package]\nname="fixture"\nversion="0.1.0"\nbuild="build.rs"\n'
        body = "#[cfg(gate)]\nfn resolve() {\n    record_uncovered_resolution_result();\n}\n"
        status, output = self._classify(
            body + "fn keep() {}\n", "fn keep() {}\n" + body.replace("    record", "        record"),
            "        record_uncovered_resolution_result();",
            auxiliary_before={"Cargo.toml": manifest, "build.rs": "fn main() {}\n"},
            auxiliary_after={"Cargo.toml": manifest, "build.rs": 'fn main() { println!("cargo:rustc-cfg=gate"); }\n'},
            extra_targets=(("custom-build", "build.rs"),), covered_auxiliary=True
        )
        self.assertEqual(status, 1, output)


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
    `// cov:ignore:` provided no exemption mechanism — it only ever exempts the
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

    def test_attribute_added_line_is_not_uncovered_without_lcov_record(self) -> None:
        # Rust attributes describe compiler and documentation metadata;
        # they do not produce runtime instructions for coverage to count.
        uncovered, exempted = classify_no_lcov_record_lines(
            added_lines=[1], file_lines=["#[doc(hidden)]"], exempt=set()
        )
        self.assertEqual(uncovered, [])
        self.assertEqual(exempted, [])

    def test_module_declaration_lines_are_not_uncovered(self) -> None:
        file_lines = [
            "mod absolutize;",
            "pub(crate) mod cascade;",
            "pub use cascade::{PageCascadeResult, cascade_page};",
            "use crate::property::Length;",
            "mod inline { fn f() {} }",
        ]
        uncovered, exempted = classify_no_lcov_record_lines(
            added_lines=[1, 2, 3, 4, 5], file_lines=file_lines, exempt=set()
        )
        self.assertEqual(uncovered, [5])
        self.assertEqual(exempted, [])

    def test_real_code_line_is_still_uncovered(self) -> None:
        # Discrimination check: mixing a comment line (3) with a genuine
        # code line (6, `pub struct TagName {`) in the same diff must
        # still flag the code line. Without this test,
        # test_comment_only_diff_is_not_uncovered alone would also pass
        # if the whole branch were changed to route every line to "not
        # uncovered" regardless of content — this pins that the fix
        # discriminates rather than blanket-passing. This is also the
        # case that keeps the earlier change's bench-file behavior
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


class CodeOnlyLifetimeTests(unittest.TestCase):
    """Regression tests for the lifetime-apostrophe misparse (see module
    docstring): a bare `'` introducing a Rust lifetime (`'i`, `'t`, `'a`,
    `'static`, ...) must not be mistaken for the start of an unterminated
    char literal, which used to silently drop every character after it on
    the line."""

    def test_lifetime_generics_do_not_swallow_rest_of_line(self) -> None:
        line = (
            "    fn parse_block<'t>(&mut self, input: &mut Parser<'i, 't>) "
            "-> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {"
        )
        # Nothing on the line — in particular the trailing `{` that
        # brace-depth tracking depends on — may be dropped.
        self.assertEqual(code_only(line)[0], line)

    def test_impl_block_lifetime_param_is_preserved(self) -> None:
        line = "impl<'i> QualifiedRuleParser<'i> for CounterStyleSheetParser {"
        self.assertEqual(code_only(line)[0], line)

    def test_reference_lifetime_is_preserved(self) -> None:
        line = "fn f<'a: 'b>(x: &'a str, y: &'b str) {}"
        self.assertEqual(code_only(line)[0], line)

    def test_comment_after_lifetime_generics_is_still_found(self) -> None:
        # comment_part() shares the same literal-tracking as code_only();
        # a lifetime earlier on the line must not blind it to a later
        # real `//` comment (here, a `cov:ignore:` marker).
        line = (
            "fn parse_block<'t>(&mut self, input: &mut Parser<'i, 't>) {} "
            "// cov:ignore: stub, never invoked"
        )
        self.assertEqual(cov_ignore_reason(line), "stub, never invoked")

    def test_single_char_literal_is_still_stripped(self) -> None:
        # Not just "doesn't crash on lifetimes" — genuine char literals
        # (shape `'x'`) must still be recognized and stripped.
        self.assertEqual(code_only("if c == 'x' { do_thing(); }")[0], "if c ==  { do_thing(); }")

    def test_escaped_newline_char_literal_is_still_stripped(self) -> None:
        self.assertEqual(code_only(r"if c == '\n' { }")[0], "if c ==  { }")

    def test_escaped_quote_char_literal_is_still_stripped(self) -> None:
        self.assertEqual(code_only(r"if c == '\'' { }")[0], "if c ==  { }")

    def test_escaped_backslash_char_literal_is_still_stripped(self) -> None:
        self.assertEqual(code_only(r"if c == '\\' { }")[0], "if c ==  { }")

    def test_hex_escape_char_literal_is_still_stripped(self) -> None:
        self.assertEqual(code_only(r"if c == '\x41' { }")[0], "if c ==  { }")

    def test_unicode_escape_char_literal_is_still_stripped(self) -> None:
        self.assertEqual(code_only(r"if c == '\u{1F600}' { }")[0], "if c ==  { }")

    def test_char_range_pattern_literals_are_still_stripped(self) -> None:
        # Two char literals back to back, no lifetime involved — brace/paren
        # counting elsewhere in the codebase depends on both being handled.
        self.assertEqual(code_only("'a'..='z' => true,")[0], "..= => true,")

    def test_double_quoted_string_is_unaffected(self) -> None:
        # Check: this fix only changes `'` handling; `"..."` string
        # stripping must be untouched.
        self.assertEqual(code_only('let s = "contains { and } braces";')[0], "let s = ;")


class ComputeExemptLinesLifetimeTests(unittest.TestCase):
    """End-to-end regression test mirroring the real
    `crates/raikiri-style/src/counter_style.rs` repro: a `cov:ignore:`
    comment placed before a trait method whose signature contains
    `Parser<'i, 't>` / `ParseError<'i, Self::Error>` must exempt the
    method's *entire* body, not just its first line."""

    def test_cov_ignore_before_lifetime_bearing_signature_exempts_full_body(self) -> None:
        lines = [
            "// cov:ignore: stub trait method exists only to satisfy the",
            "// trait, never invoked in practice",
            "fn parse_block<'t>(",
            "    &mut self,",
            "    _prelude: Self::Prelude,",
            "    _start: &ParserState,",
            "    input: &mut Parser<'i, 't>,",
            ") -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {",
            "    Err(input.new_custom_error(()))",
            "}",
            "",
            "fn unrelated_next_item() {}",
        ]
        exempt = compute_exempt_lines(lines)
        # 1-indexed lines 3..10 are the method's full 8-line body (the
        # signature through the closing `}`). Before the fix, the lone `'`
        # in `<'t>` on line 3 corrupted code_only()'s output for that line
        # to `"fn parse_block<"` — no unmatched parens left to count — so
        # brace-depth returned to 0 immediately and only line 3 was
        # exempted.
        self.assertEqual(exempt, {3, 4, 5, 6, 7, 8, 9, 10})
        # The following unrelated item must not be swept in.
        self.assertNotIn(12, exempt)


class CodeOnlyTests(unittest.TestCase):
    """Unit tests for `code_only()`'s cross-line string-literal state
    (the earlier change).

    `code_only()` strips string/char literal contents so callers can do
    brace-depth bookkeeping without being confused by punctuation inside a
    string. It used to only ever be called with a fresh "not in a string"
    state per physical line, which is correct for a line that opens and
    closes its own string literals, but wrong for a string literal left
    open across a line boundary (a backslash-continued `assert!`/`panic!`
    message is the common real-world shape): the continuation line's prose
    would be parsed as if it were code. These tests exercise the
    `in_str`/`quote` parameters directly, which now let a caller carry
    that state from one call to the next.
    """

    def test_single_line_call_behaves_like_the_old_no_state_version(self) -> None:
        code, in_str, quote = code_only('let x = "a(b)c" + 1; // trailing')
        self.assertEqual(code, "let x =  + 1; ")
        self.assertFalse(in_str)
        self.assertEqual(quote, "")

    def test_string_left_open_at_end_of_line_reports_in_str_true(self) -> None:
        # A backslash line-continuation inside a string literal: the `\`
        # is the last character on the line, so the string is still open
        # when the line ends.
        code, in_str, quote = code_only('    "first line of message \\')
        self.assertTrue(in_str)
        self.assertEqual(quote, '"')
        # Nothing after the opening quote is real code.
        self.assertEqual(code.strip(), "")

    def test_incoming_in_str_state_suppresses_punctuation_until_the_closing_quote(
        self,
    ) -> None:
        # Continuing the previous test's line: this physical line's prose
        # contains a `)` before the string actually closes. With the
        # incoming state honored, that `)` must not appear in the
        # stripped-code output.
        code, in_str, quote = code_only(
            'continuation has a closing paren) right here",', in_str=True, quote='"'
        )
        self.assertFalse(in_str)
        self.assertEqual(quote, "")
        self.assertEqual(code, ",")

    def test_reparsing_the_continuation_line_from_scratch_reproduces_the_old_bug(
        self,
    ) -> None:
        # Documents exactly what went wrong before this fix: calling
        # code_only() on a continuation line with the *default* (fresh,
        # "not in a string") state — i.e. what every call site did before
        # the earlier change — misreads the still-open string's `"` as
        # an opening quote instead of a close, so the `)` that precedes it
        # in the prose is treated as real code syntax.
        code, in_str, quote = code_only(
            'continuation has a closing paren) right here",'
        )
        self.assertIn(")", code)

    def test_apparently_open_apostrophe_state_is_never_propagated(self) -> None:
        # A bare `'` this function can't tell apart from an opening char
        # literal (a lifetime like `&'static`, or a generic bound `T: 'a`)
        # never legitimately continues onto the next physical line —
        # unlike `"`, `'` must not be reported as still-open at end of
        # line, or a single stray apostrophe would suppress brace-depth
        # counting for every following line instead of just its own.
        code, in_str, quote = code_only("&'static str")
        self.assertFalse(in_str)
        self.assertEqual(quote, "")


class ComputeExemptLinesTests(unittest.TestCase):
    """Integration tests for `compute_exempt_lines()`'s block-scoping
    (the earlier change).

    Focused on the interaction between a `// cov:ignore:`-scoped block and
    a Rust string literal inside it: the block-depth scan must not let a
    string literal's contents be miscounted as real brace/paren/bracket
    syntax, whether the string is confined to one physical line or spans
    several via backslash continuation.
    """

    def test_string_with_punctuation_on_a_single_line_does_not_affect_depth(
        self,
    ) -> None:
        # Baseline / non-regression: a single-line string containing
        # `)`/`]`/`}` must not perturb the block's depth count. (This case
        # never depended on cross-line state — code_only() always stripped
        # a same-line string's contents — but is worth pinning alongside
        # the multi-line case below.)
        lines = [
            "// cov:ignore: reason",
            'foo("has ) and ] and } inside a single-line string");',
            "let after = 1;",
        ]
        exempt = compute_exempt_lines(lines)
        self.assertEqual(exempt, {2})

    def test_backslash_continued_string_with_prose_punctuation_is_not_truncated_early(
        self,
    ) -> None:
        # The bug this task fixes: a cov:ignore block containing a
        # backslash-continued multi-line string literal (the common
        # `assert!`/`panic!` message style in this codebase) whose
        # continuation line's prose has a `)` before the string's actual
        # closing quote. Before the fix, code_only() reset its
        # string-literal state per physical line, so the continuation
        # line was parsed as if no string were open: the `)` in "closing
        # paren)" was miscounted as a real closing paren, driving the
        # cumulative depth to <= 0 one line early and truncating the
        # exempted block before the statement's real closing `);` line.
        lines = [
            "// cov:ignore: multi-line panic message, punctuation in continuation prose",
            "assert!(",
            "    condition,",
            '    "first line of message \\',
            '     continuation has a closing paren) right here",',
            ");",
            "let after_block = 1;",
        ]
        exempt = compute_exempt_lines(lines)
        # The full statement (assert!( ... );) is lines 2-6 inclusive.
        self.assertEqual(exempt, {2, 3, 4, 5, 6})
        # In particular, the closing `);` line must still be exempted —
        # this is the exact line the pre-fix bug dropped.
        self.assertIn(6, exempt)
        # And the block must not overreach into unrelated code after it.
        self.assertNotIn(7, exempt)

    def test_single_line_statement_is_exempted_alone(self) -> None:
        lines = [
            "// cov:ignore: reason",
            "let x = 1;",
            "let y = 2;",
        ]
        exempt = compute_exempt_lines(lines)
        self.assertEqual(exempt, {2})

    def test_odd_apostrophe_on_a_middle_line_does_not_leak_state_past_the_block(
        self,
    ) -> None:
        # Guards against a regression the cross-line `"`-threading fix
        # could otherwise introduce: `code_only()` already has a
        # pre-existing, independent quirk where a lifetime (`&'static`,
        # `T: 'a`) is momentarily mistaken for an opening char-literal
        # quote, since a bare `'` can't be told apart from one by this
        # best-effort parser. Before cross-line threading existed, that
        # mistake only ever mis-parsed its own line (state reset every
        # call), so it self-corrected by the next line. If a still-"open"
        # `'` state were threaded forward the same way a `"` is, that
        # single-line quirk would instead suppress brace-depth counting
        # for every subsequent line through end of block (or end of file)
        # — turning a cosmetic mis-parse into a real under-exemption bug
        # of its own. `code_only()` must only ever propagate `"` state
        # across the line boundary, never `'` state, to avoid this.
        lines = [
            "// cov:ignore: reason",
            "foo(",
            "    bar::<&'static str>(),",
            ");",
            "let after = 1;",
        ]
        exempt = compute_exempt_lines(lines)
        self.assertEqual(exempt, {2, 3, 4})
        self.assertNotIn(5, exempt)


if __name__ == "__main__":
    unittest.main()
