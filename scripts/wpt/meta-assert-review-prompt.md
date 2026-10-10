# Meta-assert review prompt

Copy `target/meta-assert-review/reviews.template.jsonl` to
`target/meta-assert-review/reviews.jsonl`, then review the rows in
`target/meta-assert-review/manifest.jsonl` with
`status == "pending"`. For each row, inspect the copied HTML and the matching
PNG screenshot.

Write one JSON object per row to `target/meta-assert-review/reviews.jsonl`:

```json
{"test_id":"css/path/test.html","decision":"pass","reason":"..."}
```

Allowed decisions:

- `pass`: the rendered result visibly supports the assert, and the result is
  stable without JavaScript execution.
- `fail`: the rendered result visibly contradicts the assert.
- `needs-human`: the image or assert is ambiguous.
- `not-renderable`: the test needs JavaScript, network state, interaction, or
  another capability not represented by the static render.

Do not edit `expectations/meta-assert-baseline.txt` during review. After review,
run `apply-meta-assert-review.py` first without `--apply`, inspect its summary,
and use `--apply` only for accepted `pass` rows.

## Bulk judging with a model

`scripts/wpt/haiku-judge-meta-assert.py` writes the same `reviews.jsonl` rows
with `claude-haiku-5-5`, so large sets can be offloaded while individual rows
stay open to agent review. It needs `ANTHROPIC_API_KEY` and
`python3 -m pip install anthropic`. Verdicts are cached by model, prompt, and
PNG hash under `judge-cache/`; `batch-submit` / `batch-collect` use the Message
Batches API at half price. `dump-prompts` writes the exact prompt per row
without calling the API.

`prepare-meta-assert-review --tests LIST` renders exactly the listed test ids,
including reftests and `.xht` files, which is how a judge is calibrated against
tests whose reftest outcome is already known.
