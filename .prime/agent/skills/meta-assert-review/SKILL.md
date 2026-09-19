---
name: meta-assert-review
description: Reviews WPT meta-assert-only tests with rendered HTML and screenshots, records pass/fail/needs-human/not-renderable decisions, and safely applies approved PASS entries to the separate meta-assert VRT baseline. Use when preparing or reviewing Raikiri WPT meta-assert tests without an AI API.
---

# Meta-assert WPT review

Use this workflow for WPT tests that contain `<meta name="assert">` but have no
`<link rel="match">` or `<link rel="mismatch">`. Do not call a paid vision API.
The review is performed from the generated HTML and PNG artifacts.

## 1. Generate review artifacts

Run from the repository root or the `meta-assert-review` worktree:

```bash
cargo run --locked -p raikiri-wpt --bin prepare-meta-assert-review -- \
  --wpt-root target/wpt \
  --output target/meta-assert-review \
  --limit 20
```

The command uses `expectations/meta-assert-baseline.txt` by default and also
excludes the existing `expectations/raikiri-baseline.txt`. It does not modify
either baseline. It writes `manifest.jsonl`, `reviews.template.jsonl`, copied
HTML, and screenshots. `parsing/` tests are excluded unless
`--include-parsing` is explicitly requested.

## 2. Review every pending row

Copy the template, then inspect each pending row's HTML and PNG:

```bash
cp target/meta-assert-review/reviews.template.jsonl \
   target/meta-assert-review/reviews.jsonl
```

Write one JSON object per row to `reviews.jsonl`:

```json
{"test_id":"css/path/test.html","decision":"pass","reason":"..."}
```

Use only these decisions:

- `pass`: the rendered result visibly supports the assertion and is stable in
  the static render.
- `fail`: the rendered result visibly contradicts the assertion.
- `needs-human`: the image or assertion is ambiguous.
- `not-renderable`: it needs JavaScript, interaction, network state, external
  resources, or another unsupported capability.

Do not mark a test `pass` when a required resource such as Ahem is missing or
the screenshot visibly uses a fallback font. Do not edit a baseline while
reviewing. See `references/review-schema.md` for the record contract.

## 3. Validate the decisions

Run the report-only command first:

```bash
python3 scripts/wpt/apply-meta-assert-review.py \
  --manifest target/meta-assert-review/manifest.jsonl \
  --reviews target/meta-assert-review/reviews.jsonl
```

Check the accepted PASS count and the review errors. This command leaves the
baseline unchanged.

## 4. Apply only approved PASS rows

After a human reviews the summary, explicitly apply the accepted rows:

```bash
python3 scripts/wpt/apply-meta-assert-review.py \
  --manifest target/meta-assert-review/manifest.jsonl \
  --reviews target/meta-assert-review/reviews.jsonl \
  --apply
```

Only `pass` rows are added, and they are added to
`expectations/meta-assert-baseline.txt`, never to the normal reftest baseline.

## Validation

When changing the implementation, run:

```bash
cargo fmt --all -- --check
cargo test --locked -p raikiri-wpt
```
