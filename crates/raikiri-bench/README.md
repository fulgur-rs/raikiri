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
| `layout_allocations` | Flex, grid and many inline spans; 100/1,000 items, five fresh samples | Allocation count and cumulative requested bytes during fresh single-page layout | Parsing, cascade, bundled Ahem fonts, cache warm-up and destruction |
| `pagination_document` | `pagination_document`, `page_output_document`; 10/100 forced pages, or 100/1,000 inline paragraphs, table rows or multicol paragraphs | Paged layout including text construction and fragmentation; separately, extracting fragments, text runs and paint order across all pages | Parsing and cascade; page-output extraction also excludes layout |
| `paint_document` | `scene_document`, `paint_document`, `raster_png_document`; 5/20 paragraphs fitting within 640×800 px | PageScene extraction; separately, real paint command recording; separately, production painting + CPU rasterization + PNG encoding | Parsing, cascade and layout |
| `streaming_document` | `streaming_document`: 10/100/500 forced pages; `first_page_document`: 1,500/3,000/6,000 forced pages | Full streaming parse/cascade/layout, or elapsed time from stream creation to first page callback entry | HTML generation, fonts and a warm-up stream |
| `fulgur_baseline` | Historical single-page table-shaped workload | HTML → first-page PNG | Input generation and warm-up |
| `streaming_memory` | Text document lengths from `RAIKIRI_STREAM_SECTIONS` | Batch vs streaming peak live heap and elapsed time | Input generation and cache warm-up |
| `pseudo_output_sharing` | Repeated pseudo-element styles | Pseudo output sharing costs | See target module documentation |

The five stage/page targets add 48 Criterion cases. Fixture generation and
validation are lazy: a name filter does not prepare unrelated cases. Each
case probes its output before measuring, so a missing stylesheet, lost text,
empty paint recording or missing pages cannot silently appear as a speedup.

Benchmark-only items carry explicit `cov:ignore` annotations because the
workspace coverage run does not build benchmark targets. Their validation
comes from the release smoke runs and untimed probes.

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
consumer drawing. Its larger inputs cross the production 64 KiB checkpoint;
every probe and timed sample requires the first callback during a write,
before the final input chunk. A callback that occurs only in `finish()`
fails validation. The last section uses an explicit class rather than
`:last-child`, which would hold all streaming pages until EOF. The smaller
throughput inputs still measure complete streams finalized after EOF.

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

## Style-view migration baseline (RAIKIRI-708)

Measure the complete `layout_single_page` call, including the computed-style
bridge, IFC construction and layout. Parsing and cascade are prepared before
each call; output destruction and validation are outside measurement. Every
sample gets a fresh DOM. `inline_1000` contains 35,007 DOM nodes and supplements
the flex/grid documents with a large bridge workload. These fixtures already
exist in `common/workloads.rs`; the harness changes no production layout code.

Run timing and allocation measurements separately. `layout_allocations` uses a
Pure Rust `GlobalAlloc` forwarding to `System`, with process-wide atomic
counters including joined Rayon workers. It counts successful `alloc`,
`alloc_zeroed`, and `realloc` calls; a successful `realloc` adds the full new
requested size. Bytes are allocation traffic, not retained memory, peak live
memory or RSS. Failed allocations and deallocations do not add to the counters.
Unrelated work in the benchmark process would be counted. Instrumentation adds
overhead, so its elapsed time is not a timing measurement.

```sh
RAYON_NUM_THREADS=1 TMPDIR="$HOME/tmp" cargo bench --locked -p raikiri-bench \
  --bench layout_document -- 'layout_document/(flex|grid|inline)_' \
  --warm-up-time 1 --measurement-time 3 --save-baseline style-view-base --noplot
RAYON_NUM_THREADS=1 TMPDIR="$HOME/tmp" cargo bench --locked -p raikiri-bench \
  --bench layout_allocations
```

The second command emits CSV to stdout and Cargo diagnostics to stderr. Fixing
the Rayon pool to one worker improves reproducibility; these numbers do not
characterize parallel scaling. Production defaults still apply to IFC selection.
Smoke-run the timing cases with `-- --test`; the allocation binary always runs
its six cases and checks formatting context and laid-out content.

Measurements on 2026-10-21 used Rust 1.91.0, Taffy 0.15.0, Linux x86_64,
AMD Ryzen 5 5600G (6 cores / 12 logical CPUs), the repository release profile,
Clang/wild linker and bundled Ahem. CPU frequency was not pinned. No builds
from this task ran during timing; other host activity was not controlled.
Criterion uses 20 samples and 95% confidence intervals. Allocation counts and
requested bytes agreed across all five samples of every case below.

`3dad82950` is the historical direction-page baseline. `c0917ca89` is the
implementation base before RAIKIRI-710; intervening commits mean the two rows
must **not** be used to attribute a performance change to 710.

| Commit | Case | Mean ms [95% interval] | Allocations | Requested bytes |
| --- | --- | --- | ---: | ---: |
| `3dad82950` | flex_100 | 6.416 [5.844, 6.929] | 19,809 | 3,711,975 |
| `3dad82950` | flex_1000 | 40.179 [38.804, 41.853] | 201,993 | 36,665,879 |
| `3dad82950` | grid_100 | 4.433 [4.080, 4.866] | 23,696 | 4,274,819 |
| `3dad82950` | grid_1000 | 59.868 [54.260, 66.174] | 240,623 | 42,387,523 |
| `3dad82950` | inline_100 | 20.318 [19.996, 20.651] | 50,593 | 28,990,435 |
| `3dad82950` | inline_1000 | 228.882 [226.013, 232.926] | 501,508 | 288,693,991 |
| `c0917ca89` | flex_100 | 3.060 [3.001, 3.119] | 19,810 | 3,712,190 |
| `c0917ca89` | flex_1000 | 37.371 [36.943, 37.842] | 201,994 | 36,667,894 |
| `c0917ca89` | grid_100 | 3.941 [3.906, 3.979] | 23,697 | 4,275,034 |
| `c0917ca89` | grid_1000 | 46.741 [46.254, 47.377] | 240,624 | 42,389,538 |
| `c0917ca89` | inline_100 | 19.684 [19.495, 19.886] | 50,594 | 28,993,950 |
| `c0917ca89` | inline_1000 | 219.469 [218.331, 220.610] | 501,509 | 288,729,006 |

For another stage, pin baseline and candidate commits, reuse this exact harness
and fixtures in both, prebuild both binaries, and alternate at least three
baseline/candidate pairs on the same otherwise idle host. Keep CPU affinity,
frequency policy, allocator, toolchain, Rayon count and font inputs identical.
Use separate worktrees under `.worktrees/`. To reproduce the historical entry,
apply only this change's `crates/raikiri-bench/Cargo.toml` and
`benches/layout_allocations.rs` additions to `3dad82950`; its timing fixtures
are already identical. Do not apply production layout changes to the baseline.

Treat 5% as an initial timing noise band, **not** a license to accept a detected
regression. A difference above it triggers a repeated comparison; a smaller
repeatable difference with separated confidence intervals also needs review.
If the three baseline means vary by more than 5%, or an interval is wider than
that band, rerun under stable conditions before claiming non-regression. The
historical first run has wide intervals and is a reference measurement, not
a calibrated performance gate. For deterministic allocation traffic, accept no
unexplained increase; all five samples must agree before comparing. Preserve
commands, commit IDs, intervals and the decision here or in the stage's issue,
then remove this task's raw reports and scratch worktrees.
