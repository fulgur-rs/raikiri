# scripts/wpt/

`fetch.sh` keeps the sparse, shallow W3C web-platform-tests checkout at
`$HOME/.cache/raikiri/wpt`, pinned to the SHA in `pinned_sha.txt`. Every
worktree gets a replaceable `target/wpt` symlink to that checkout for existing
commands and Rust tests. Removing `target/` (for example with `cargo clean`)
removes only the link; `fetch.sh` and the gate recreate it from the home cache.

`subset.txt` defines the shared sparse roots: all of `acid/`, `css/`, `fonts/`,
`images/`, and the top-level `common/` and `resources/` trees. The
`/resources` pattern is anchored so it selects only the WPT-root directory,
not the per-test `resources/` helper directories nested throughout the tree.
It also includes upstream
`tools/`, the root `wpt` CLI, and `/docs/commands.json`, which that CLI reads
while loading its command registry. The top-level
`common/` carries shared helpers used by reftests, including
`rendering-utils.js`. The top-level `resources/` directory carries the real
`testharness.js`,
`testharnessreport.js`, `check-layout-th.js`, and `testdriver*.js`, so
JS-driven WPT pages can load their scripts from files instead of needing a
substitute harness. `fetch.sh` validates this exact set and atomically
replaces the cache's sparse-pattern file with a read-only inode. An
already-open stale writer is detached; later runs of an old `fetch.sh` fail
instead of narrowing the shared checkout and hiding tests from sibling
worktrees. Do not narrow or add per-task paths; use the survey's
category/theme filters. To change shared roots, use a reviewed project-level
change that updates both `subset.txt` and the guard in `fetch.sh`. Only then
should a maintainer unlock
`$HOME/.cache/raikiri/wpt/.git/info/sparse-checkout` with `chmod u+w` before
rerunning the fetch.

## One-time migration from `target/wpt`

If an older checkout still has a physical `target/wpt` directory, run this once
from the main worktree when the home-cache destination does not already exist:

```sh
mkdir -p "$HOME/.cache/raikiri"
mv target/wpt "$HOME/.cache/raikiri/wpt"
ln -s "$HOME/.cache/raikiri/wpt" target/wpt
```

This preserves the pinned Git checkout and any local files. `fetch.sh` refuses
to create a duplicate cache while the old physical checkout is still present.

## Usage

    scripts/wpt/fetch.sh

Idempotent: re-running updates to the current pinned SHA. Override the remote
URL with `WPT_REMOTE_URL=...` (mirrors / CI cache warmup).

## Running upstream reftests directly

The Raikiri product plugin lets upstream `wpt run` own test discovery,
wptserve, reference traversal, comparison, timeout, and result reporting.
Mise pins uv and exposes the task entry point. The wrapper fetches the pinned
checkout, builds the screenshot process, asks uv to prepare
`target/wpt-venv`, installs WPT's runner requirements and the local product
plugin, and starts the upstream runner with WPT's own environment setup
disabled:

```sh
mise run wpt:reftest -- css/css-color/background-color-rgb-001.html
mise run wpt:print-reftest -- css/css-page/page-box-001-print.html
```

The print runner renders every paginated result to contiguous
`page-0001.png`, `page-0002.png`, ... files, then returns the ordered page set
to upstream wptrunner for page-count and per-page comparison. The temporary
PNGs are removed after each comparison.

The virtualenv is disposable build output under `target/`. uv selects the
pinned Python 3.14.7 interpreter and downloads it when the host does not
already provide that version. Updating the Python version requires changing
`PYTHON_VERSION` in `run-raikiri.sh`.

This prototype supports static HTTP screen and print reftests. It does not yet
support HTTPS, `reftest-wait`, or testharness tests.

## Updating the pin

The pin is initially borrowed from fulgur (`fulgur/scripts/wpt/pinned_sha.txt`)
for cross-project consistency. Bump raikiri's pin only when:

1. fulgur bumps and raikiri should follow (default), OR
2. raikiri project-specific regression requires a fresh WPT font asset (rare)

Steps:

1. Inspect upstream WPT `main` (or fulgur's next pin) and pick a passing commit
2. Replace the SHA line in `pinned_sha.txt`
3. Re-run `scripts/wpt/fetch.sh`
4. Re-run `cargo test -p raikiri --test hello_world_vrt -- --ignored`
   (VRT test is `#[ignore]` by default — see hello_world_vrt.rs). Golden PNG
   may need regeneration if font asset content shifted:
   `RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt -- --ignored`
5. Commit `pinned_sha.txt` + updated golden (if any) in one PR

## Relation to `raikiri-wpt`

`raikiri-wpt` (WPT test runner) does not currently discover the whole
`target/wpt/` tree. The broad CSS checkout is for survey and test selection; it
does not mean every file is supported by the runner. Keep the shared roots
stable while runner coverage grows; use a reviewed project-level change if the
shared checkout scope must expand.

## Running CSS Text i18n testharness pages

The `run-css-text-i18n` binary runs the testharness pages under
`css/css-text/i18n` with Raikiri layout geometry. Each page runs end to end:
its `<script>` elements, inline or `src`, execute in document order with the
checkout's real, unmodified `resources/testharness.js`, and the runner serves
its own `resources/testharnessreport.js` to collect the results.
It does not change either expectations file. The command can return nonzero
when a WPT assertion fails or when a file cannot run; both are reported
separately.

```sh
scripts/wpt/fetch.sh
cargo run --locked -p raikiri-wpt --bin run-css-text-i18n -- \
    --wpt-root target/wpt
```

The 158 reftest-only files in the directory remain on the visual reftest path.
Pages that need DOM APIs the runtime does not implement fail or report an
execution error rather than being skipped.

General scripts can use `raikiri_js::runtime::DomRuntime`, which binds native
DOM interfaces (`Document`, `Element`, `HTMLElement`, ...) directly to a
`raikiri_dom::Document` through the embedder-supplied
`raikiri_js::runtime::DocumentHost` trait. The current runner's host
(`WptDocumentHost`) rebuilds style and layout lazily after DOM mutations, over
one live Raikiri document.

## Surveying WPT reftests

`survey_reftests.py` builds an inventory of actual `<link rel="match|mismatch">`
reftest files in the fetched WPT tree. It groups files by WPT category and the
first directory below that category. Root-level files are grouped by a
numbered filename prefix, but are marked for manual theme review. The report
includes reference-pair counts, baseline membership, missing/external
references, and file extensions that the current `raikiri-wpt` tree discovery
does not scan. It records the checked-out WPT revision, sparse-checkout
patterns, and the standard 800x600 CSS-pixel viewport.

The survey only sees files materialized in the current checkout. It records
the WPT revision and sparse patterns, so an absent category in a sparse
checkout is absent, not empty. For a Git WPT checkout the survey scans
tracked, materialized test files and ignores untracked local fixtures. CSS
categories are covered by the stable `css/` root; categories outside the fixed
roots need an explicit shared checkout-scope change, not a task-specific edit
to `subset.txt`. Use `target/wpt` as the stable working-tree path.

```sh
scripts/wpt/fetch.sh
python3 scripts/wpt/survey_reftests.py --format text
python3 scripts/wpt/survey_reftests.py \
    --category css/css-text --theme hyphens --list-tests
python3 scripts/wpt/survey_reftests.py \
    --category css/css-text \
    --format json \
    --output target/css-text-reftest-survey.json
```

This is an inventory, not a test run: a baseline entry is not proof of a fresh
PASS, and structurally resolved links do not prove the test is renderable. A
file with multiple reference links must pass every pair before it is promoted.
Root-level test files are grouped by a numbered filename prefix and marked
for manual theme review; directory names are also only an initial grouping
hint. The script is read-only and never
updates `expectations/raikiri-baseline.txt`. Use the `wpt-ref-coverage` project
skill to select one reviewed category/theme and run its implementation loop.
The separate meta-assert review flow below is not part of this survey.

## Updating the CSS dashboard

`docs/wpt-dashboard.html` is generated from the pinned WPT tree and
`expectations/raikiri-baseline.txt`; do not edit its counters by hand. From a
working tree, run:

    scripts/wpt/update-dashboard.sh

The wrapper fetches the pinned WPT checkout, runs the `raikiri-wpt` smoke
reftests, records their passed count, and invokes `generate-dashboard`. The
page reports that smoke count alongside the T2 baseline coverage. The
generator reads the WPT Git tree rather than only the sparse working files, so
the CSS denominator remains complete. To regenerate without rerunning the
smoke tests, invoke the binary directly (the optional smoke count can be
supplied when it is available):

    cargo run --locked -p raikiri-wpt --bin generate-dashboard -- \
        --wpt-root target/wpt \
        --baseline expectations/raikiri-baseline.txt \
        --basic-reftest-count 12 \
        --output docs/wpt-dashboard.html


## Reviewing meta-assert-only tests

`prepare-meta-assert-review` prepares a fixed-cost review queue for HTML-like
WPT tests that have `<meta name="assert">` but no reftest link. It does not
call an AI API and does not modify the baseline. `parsing/` tests are excluded
by default because they need a JavaScript engine; use `--include-parsing` only
when that limitation is intentional.

```bash
cargo run --locked -p raikiri-wpt --bin prepare-meta-assert-review -- \
    --wpt-root target/wpt \
    --baseline expectations/meta-assert-baseline.txt \
    --exclude-baseline expectations/raikiri-baseline.txt \
    --output target/meta-assert-review \
    --limit 20
```

Use `--path-prefix css/css-backgrounds/` (or another directory) to focus the
queue on one WPT category. The output contains `manifest.jsonl`, copied source files under `html/`, and
screenshots under `screenshots/`. It also writes `reviews.template.jsonl`;
copy this file to `reviews.jsonl` and fill in each `decision` and `reason`.
The reviewed entries belong to the separate
`expectations/meta-assert-baseline.txt`; they are not added to the normal
`expectations/raikiri-baseline.txt`. The suggested agent instructions are in
`scripts/wpt/meta-assert-review-prompt.md`. An AI coding agent reviews each
pending row and writes `reviews.jsonl`, for example:

```json
{"test_id":"css/CSS2/fonts/font-001.xht","decision":"pass","reason":"The rendered font size and family match the assertion."}
```

Allowed decisions are `pass`, `fail`, `needs-human`, and `not-renderable`.
Apply accepted decisions only after review:

```bash
python3 scripts/wpt/apply-meta-assert-review.py \
    --manifest target/meta-assert-review/manifest.jsonl \
    --reviews target/meta-assert-review/reviews.jsonl \
    --baseline expectations/meta-assert-baseline.txt
python3 scripts/wpt/apply-meta-assert-review.py \
    --manifest target/meta-assert-review/manifest.jsonl \
    --reviews target/meta-assert-review/reviews.jsonl \
    --baseline expectations/meta-assert-baseline.txt \
    --apply
```

The first command is report-only. Only the second command changes the
baseline. The manifest records the WPT SHA, viewport, renderer, and font
source so the agent's decision is reproducible.

## Baseline report

`run-baseline-report` runs every id in `expectations/raikiri-baseline.txt` and
`expectations/expected-failures.txt` through the in-process harness and writes
one `id<TAB>STATUS<TAB>detail` line per id. Query variants in the expected-failures
file are run with that exact query. It is report-only unless `--strict` is supplied.

- A reftest is PASS when one `rel=match` reference matches exactly at 800x600 (several
  `rel=match` references are alternatives, as in WPT) and every `rel=mismatch` reference
  differs. Each reference is tried with local resources enabled and without them, because
  the baseline mixes tests pinned under either mode.
- A `css/**/parsing/*` page is PASS when every assertion passes.
- Expected-failure entries remain executable: assertion or pixel mismatches report
  `XFAIL`, an unexpected pass reports `XPASS`, and execution errors stay `ERROR`.
  `XFAIL` never counts as `PASS`; `--strict` exits non-zero for `FAIL`, `XPASS`, or
  `ERROR` while allowing recorded `XFAIL` rows.
- Some baseline ids do not pass on a given commit (drift, or a different harness produced
  the pin), so compare two reports with `diff`; do not read the absolute count as a gate.

```bash
cargo run --locked -p raikiri-wpt --bin run-baseline-report -- \
  --jobs 4 --output ~/.cache/raikiri/baseline-reports/before.tsv
# ... change something, then:
cargo run --locked -p raikiri-wpt --bin run-baseline-report -- \
  --jobs 4 --output ~/.cache/raikiri/baseline-reports/after.tsv
cargo run --locked -p raikiri-wpt --bin run-baseline-report -- \
  diff ~/.cache/raikiri/baseline-reports/before.tsv ~/.cache/raikiri/baseline-reports/after.tsv
```

Use `--only ID` (repeatable) or `--limit N` for a quick run. `--jobs` was checked to give
the same report as `--jobs 1` on the first 80 ids; image-heavy tests share a process-wide
decode budget, so re-check before trusting a large `--jobs` value. `diff` only lists
PASS-to-not-PASS transitions (regressions), the reverse (fixes), ids missing from the
second report, and PASS results that became weaker: a test that passed cleanly before but
now passes only after falling back to a run without local resources (the row detail reads
`passes only without local resources`). A weakened PASS is still a PASS, so it is reported
separately rather than as a regression.

See [expectations/README.md](../../expectations/README.md) for the difference
between PASS baseline, expected failures, known issues, quarantine, and deprecated
tests, plus the review and XPASS cleanup procedure.

Text is laid out by the shodo inline engine; a paragraph it cannot lay out fails its test
with an error. The run refuses to start when the WPT font collection cannot be built (it
does not fall back to the installed fonts, which would make the numbers
machine-dependent). `--wpt-fonts` names that default and changes nothing; the former
`--ifc` and `--no-ifc` flags are rejected.

Outside `run-baseline-report` (unit tests, `run_pair`), the engine falls back to the
installed fonts when no WPT font directory exists, so a checkout
without the fetched WPT fonts still runs the tests.

## Media-query dimensions

Screen rendering and live testharness documents evaluate width and height
against the requested viewport. Print rendering evaluates them against the
requested fallback page box, including margins, before authored `@page` sizes.
The context is fixed for the document's element, page, and font-face cascades.
Changing authored page size, margins, or named pages does not change the
media-query environment. The print adapter resolves viewport units in collected
inline and external `@media` / `@import` query operands against that fixed page
box; declaration lengths retain their authored page-area basis. The page cascade
selects the declaration basis after fetching stylesheets, so inactive stylesheet
and import media conditions do not contribute page dimensions. Inline style and
animation declarations use the same basis through reparsing. Only CSS dimension
tokens are expanded; identifiers, strings, URLs, and HTML text remain intact.
The first print page uses the root direction to select its left or right page
rules. Live documents resolve fetched and dynamically replaced CSS again on
each style flush. Physical viewport units and their small, large, and dynamic
variants share the fixed requested viewport; logical `vi` and `vb` families
follow the root writing mode in declarations and the initial horizontal writing
mode in media queries.
See [Media Queries 4 §4](https://www.w3.org/TR/mediaqueries-4/#width)
and [CSS Paged Media 3 §7.1](https://www.w3.org/TR/css-page-3/#page-size),
with logical axes defined by
[CSS Values 4 §6.1.2.2](https://www.w3.org/TR/css-values-4/#viewport-relative-lengths).

The standalone style API defaults to a nominal 480 × 288 print page box;
consumers with another paper size set `LayoutConfig::media_context` explicitly
using `MediaContext::with_viewport`. `PageDefaults` remains the independent
fallback for layout geometry.

The media-query pass set is measured at exactly 800 × 600 with the pinned WPT
fonts. To rerun all 22 selected upstream references:

```sh
cargo test --locked -p raikiri-wpt --test css_mediaqueries_reftests -- --ignored
```

The pass set covers range syntax, media-type error recovery, invalid or unknown
features, and initial font-relative units. It does not imply support for every
media feature or CSS function: orientation/aspect-ratio, device/display features,
calc/sign functions, custom media, scripting, and one dynamic script error
still have separate failures in the wider 57-case survey.

Following CSS Paged Media 3 §7.1, the page cascade ignores `size` descriptors
qualified by paper-dimension media queries, including width, height, aspect
ratio, and orientation. Other qualified page declarations still apply when
the condition matches. This qualification includes inherited stylesheet and
import media conditions; a media list keeps the union of paper dependencies
from its valid arms even when another arm matches independently. Invalid arms
do not contribute dependencies. Ignored sizes do not set the declaration
viewport basis. Page-context viewport units, including size and margin-box
declarations, are resolved once against the existing nominal 480 × 288 basis used by
the page probe, before ordinary declaration units expand. This adapter policy
does not claim complete support for every paged-media viewport-unit behavior.

## Contextual background colors

`color` and `background-color` retain `currentcolor` operands until the cascade
knows their color basis, including operands in `color-mix()` and backgrounds
expanded from a shorthand or a custom property. A `color` expression uses the
inherited foreground; a background expression uses the receiving element's own
foreground. Explicit `background-color: inherit` retains the expression for the
receiver, including root-to-page and page-to-margin-box inheritance. See
[CSS Color 4 §15.5](https://www.w3.org/TR/css-color-4/#resolving-other-colors),
[CSS Color 5 §10.1](https://www.w3.org/TR/css-color-5/#resolving-color-values), and
[CSS Paged Media 3 §6](https://www.w3.org/TR/css-page-3/#page-properties).

The exact 800 × 600 bundled-font pass set includes the legacy currentcolor
background reference and four color-mix currentcolor references. The wider
`currentcolor-001` and `currentcolor-002` references still have an existing
163-pixel text residual: replacing the authored contextual colors with literal
colors produces the same residual on the original source. Those references are
kept out of the PASS baseline. General `background: inherit` shorthand expansion
is a separate existing parser gap (0vv.127); the supported longhand inheritance
above does not establish all eight shorthand longhands. Color conversion still
uses the existing bounded 8-bit sRGB model.
