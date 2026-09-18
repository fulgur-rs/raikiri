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
import unittest

from patch_coverage import (
    classify_no_lcov_record_lines,
    code_only,
    compute_exempt_lines,
    cov_ignore_reason,
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
