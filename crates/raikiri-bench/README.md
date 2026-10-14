# Rendering benchmarks

Run a target with `cargo bench -p raikiri-bench --bench <target>`. Criterion
targets accept a name filter after `--`, for example:

```sh
cargo bench -p raikiri-bench --bench layout_document -- table_auto_1000
cargo bench -p raikiri-bench --bench streaming_document -- first_page_document
```

## Stage coverage

| Target | Groups / workload | Timed work | Untimed preparation |
| --- | --- | --- | --- |
| `prepare_document` | `parse_document`, `rule_tree_document`; reports of 10×50 and 40×100 item rows | HTML parsing into an uncascaded document; separately, CSS parsing and rule-tree construction, including UA rules | Report generation and cascade validation |
| `cascade_document` | Reports of 10×50 and 40×100 item rows | Cascade | Parsing and rule-tree construction |
| `layout_document` | Text, many inline spans, fixed/auto/spanned collapsed-border tables, flex, grid, balanced multicol; 100/1,000 paragraphs, items or rows | Fresh single-page layout, including text shaping and line breaking | Parsing, cascade and font collection; a fresh DOM per iteration |
| `pagination_document` | `pagination_document`, `page_output_document`; 10/100 forced pages, or 100/1,000 inline paragraphs, table rows or multicol paragraphs | Paged layout including text construction and fragmentation; separately, extracting fragments, text runs and paint order across all pages | Parsing and cascade; page-output extraction also excludes layout |
| `paint_document` | `scene_document`, `paint_document`, `raster_png_document`; 5/20 paragraphs fitting within 640×800 px | PageScene extraction; separately, real paint command recording; separately, production painting + CPU rasterization + PNG encoding | Parsing, cascade and layout |
| `streaming_document` | `streaming_document`, `first_page_document`; 10/100/500 forced pages | Full streaming parse/cascade/layout, or elapsed time from stream creation to first page callback entry | HTML generation, fonts and a warm-up stream |
| `fulgur_baseline` | Historical single-page table-shaped workload | HTML → first-page PNG | Input generation and warm-up |
| `streaming_memory` | Text document lengths from `RAIKIRI_STREAM_SECTIONS` | Batch vs streaming peak live heap and elapsed time | Input generation and cache warm-up |
| `pseudo_output_sharing` | Repeated pseudo-element styles | Pseudo output sharing costs | See target module documentation |

The five stage/page targets add 48 Criterion cases. Fixture generation and
validation are lazy: a name filter does not prepare unrelated cases. Each
case probes its output before measuring, so a missing stylesheet, lost text,
empty paint recording or missing pages cannot silently appear as a speedup.

New layout, page-output, paint and streaming targets use only the bundled
Ahem font and ASCII content. They do not measure system font discovery,
CJK shaping, remote resources or PDF generation. Stage names describe the
whole public operation: `pagination_document` includes layout, and
`raster_png_document` includes painting and encoding. Neither is an isolated
internal subpass.

Output destruction is timed except in `layout_document`, where
`iter_batched_ref` excludes setup and destruction. Its DOM is fresh every
iteration to avoid measuring cached relayout. Throughput uses the input unit
(report item rows, paragraphs, items, table rows, or forced pages), not an
assumed output-page count. First-page latency has no throughput denominator.

First-page samples use `iter_custom`: they sum the time to callback entry,
then finish and validate each stream outside that duration. The input is
fed synchronously in 4 KiB writes. Later pages still cost wall time during
sampling, so this benchmark can take longer than its configured measurement
duration. It measures CPU latency with warm fonts, without network delays or
consumer drawing. It does not assert that the first page arrives before EOF.

## Validation and comparisons

Smoke-run every new case without collecting statistics:

```sh
cargo bench -p raikiri-bench \
  --bench prepare_document --bench layout_document \
  --bench pagination_document --bench paint_document \
  --bench streaming_document -- --test
```

For comparisons, use the same toolchain, release configuration, hardware and
font inputs. Avoid concurrent builds, and alternate baseline and candidate
runs. Criterion history from one run on a busy host is not a calibrated
regression threshold. These targets provide instrumentation, not a claim of
parity with a measured Fulgur version.
