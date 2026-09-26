# scripts/wpt/

`fetch.sh` keeps the sparse, shallow W3C web-platform-tests checkout at
`$HOME/.cache/raikiri/wpt`, pinned to the SHA in `pinned_sha.txt`. Every
worktree gets a replaceable `target/wpt` symlink to that checkout for existing
commands and Rust tests. Removing `target/` (for example with `cargo clean`)
removes only the link; `fetch.sh` and the gate recreate it from the home cache.

`subset.txt` defines the shared sparse roots: all of `acid/`, `css/`, `fonts/`,
`images/`, and the top-level `resources/` (anchored as `/resources` so it
pulls in only the WPT-root `resources/` directory, not the many per-test
`resources/` helper directories nested throughout the tree). The top-level
`resources/` directory carries the real `testharness.js`,
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

The `run-css-text-i18n` binary runs the testharness-only pages under
`css/css-text/i18n` with Raikiri layout geometry. It executes the inline WPT
JavaScript unchanged. It does not load the external `testharness.js` or
`testharnessreport.js`; it supplies the small helper API used by this test set.
It does not change either expectations file. The command can return nonzero
when a WPT assertion fails or when a file cannot run; both are reported
separately.

```sh
scripts/wpt/fetch.sh
cargo run --locked -p raikiri-wpt --bin run-css-text-i18n -- \
    --wpt-root target/wpt
```

This route is intentionally limited to the CSS Text i18n testharness APIs. The
158 reftest-only files in the directory remain on the visual reftest path; this
command does not run arbitrary WPT JavaScript or implement unsupported DOM APIs.

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
