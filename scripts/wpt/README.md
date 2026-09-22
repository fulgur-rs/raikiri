# scripts/wpt/

`fetch.sh` executes a sparse, shallow clone of the W3C web-platform-tests
repository into `target/wpt/`, pinned to the SHA in `pinned_sha.txt`.

`subset.txt` defines the shared sparse roots: all of `css/`, `fonts/`, and
`images/`. `fetch.sh` validates this exact set, symlinks one checkout into task
worktrees, and makes its local sparse-pattern file read-only. A stale branch's
older `fetch.sh` therefore fails instead of narrowing the shared checkout and
hiding tests from sibling worktrees. Do not narrow or add per-task paths; use
the survey's category/theme filters. To change shared roots, use a reviewed
project-level change that updates both `subset.txt` and the guard in `fetch.sh`.
Only then should a maintainer unlock `target/wpt/.git/info/sparse-checkout`
with `chmod u+w` before rerunning the fetch.

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
