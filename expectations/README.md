# WPT expectations

These files describe how Raikiri evaluates the pinned Web Platform Tests
(WPT). Keep each file limited to one outcome policy:

| File | Meaning |
| --- | --- |
| `raikiri-baseline.txt` | Tests expected to pass. A result other than `PASS` is a regression. |
| `expected-failures.txt` | Exact tests that still run and currently produce a known, deterministic failure. |
| `known-issues.txt` | Tests outside current scope. Matching tests are skipped. A directory prefix ends in `/`. |
| `quarantine.txt` | Tests with intermittent failures, scoped by platform and renderer. |
| `deprecated.txt` | Tests excluded from evaluation, such as known crashers. |
| `tracked-wpt.txt` | Informational test tracking categories. |
| `meta-assert-baseline.txt` | Separate reviewed results for tests without a reftest. |

## Expected failures

`expected-failures.txt` uses one exact test id per row:

```text
<test_id> | <reason> | <local_beads_issue_id> | <added_date> | <review_by>
```

Both dates use `YYYY-MM-DD`. The test id is a root-relative WPT path. To track
a query variant, append its exact declared query string, preserving its order
and spelling:

```text
css/css-text/example-001.html?class=halt,vrl | Raikiri output differs from the pinned reference | <local-beads-issue-id> | 2026-10-04 | 2026-12-31
```

This row shows the field shape only; replace the placeholder with the issue
that tracks the specific test failure.

Wildcards, directory prefixes, fragments, and normalized or reordered queries
are not accepted. Query variants are separate ids from the base test URL.
The current in-process baseline report executes query variants for reftests;
query variants under `css/**/parsing/*` are reported as `ERROR` until the
testharness runner can execute them.

The expected-failure rule is applied after execution:

| Actual result | Reported result |
| --- | --- |
| Pixel mismatch or failed assertion | `XFAIL` |
| Pass | `XPASS` |
| Render, resource, or harness error | `ERROR` |
| Skip | `SKIP` |

`XFAIL` is visible as a failure and never counts as `PASS`. `XPASS` means the
record is stale: remove it and promote the id to the pass baseline when the
test is in scope. `--strict` on `run-baseline-report` exits non-zero for
`FAIL`, `XPASS`, or `ERROR`; recorded `XFAIL` remains visible and does not make
that report fail.

An exact expected-failure id can override a matching directory-prefix entry
in `known-issues.txt`, so the test runs and its result stays visible. Exact
conflicts with the pass baseline, `deprecated.txt`, `quarantine.txt`, or an
exact `known-issues.txt` row are rejected by `validate-expectations`.
Deprecated and quarantined tests remain excluded even if a matching
expected-failure row exists.

## Adding and reviewing an entry

1. Reproduce the exact test URL and confirm a stable assertion or pixel
   mismatch. Do not record a render, network, font-loading, or harness error as
   an expected failure.
2. Track the cause in a local Raikiri Beads issue. Do not open an upstream
   issue for this workflow.
3. Add one exact test id with a concise reason, local Beads id, added date, and
   review date. Do not add a directory-wide entry or move a whole failing
   suite here without case-by-case triage.
4. Run the expectation validator and the affected test variant:

   ```sh
   cargo run --locked -p raikiri-wpt --bin validate-expectations
   cargo run --locked -p raikiri-wpt --bin run-baseline-report -- \
       --only 'css/css-text/example-001.html?class=halt,vrl'
   ```

   The default baseline report also runs every id in
   `expected-failures.txt`. Add `--strict` when using that report as a gate.
5. Review each active entry by `review_by`. If it still fails for the same
   reason, update the issue and review date. If it passes, remove the row and
   add the exact id to `raikiri-baseline.txt` when it belongs in the pass set.
6. Remove entries when the issue is resolved, the test is obsolete, or the
   behavior is no longer in scope. Do not leave a closed issue attached to an
   active exception.

Use `known-issues.txt` when the test should not run because it is outside the
product scope. Use `quarantine.txt` only for intermittent behavior. Neither
file is a substitute for tracking a deterministic failure that should keep
running.
