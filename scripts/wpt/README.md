# scripts/wpt/

`fetch.sh` executes a sparse, shallow clone of the W3C web-platform-tests
repository into `target/wpt/`, pinned to the SHA in `pinned_sha.txt`.

The set of fetched paths is controlled by `subset.txt` (one pattern per line,
Git sparse-checkout syntax). The current subset includes `fonts` (Ahem, Lato,
CSSTest 等) for VRT cross-machine determinism and the focused
`css/css-page/page-box-001-print.html` canvas-background pin.

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

`raikiri-wpt` (WPT test runner) does not currently discover the
whole `target/wpt/` tree. The css-page subset is used as a focused source pin;
its equivalent reftest is covered by the `raikiri-wpt` unit suite. When the
runner starts consuming the full WPT tree, extend `subset.txt` and update this
README.

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
