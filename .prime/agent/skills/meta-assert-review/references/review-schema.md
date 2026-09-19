# Review record schema

`reviews.jsonl` contains one JSON object for each manifest row being reviewed.
Required fields:

- `test_id`: exact `test_id` from `manifest.jsonl`.
- `decision`: one of `pass`, `fail`, `needs-human`, or `not-renderable`.
- `reason`: concise evidence-based explanation.

The apply tool rejects missing or duplicate IDs, IDs absent from the manifest,
invalid decisions, and PASS rows whose manifest status is not `pending`.
It ignores non-PASS decisions for baseline application. The manifest remains
the source of the rendered artifact paths and metadata, including WPT SHA,
viewport, renderer, and font source.
