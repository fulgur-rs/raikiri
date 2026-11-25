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
| `table_memory` | One three-column table; row counts from `RAIKIRI_TABLE_ROWS` | Peak live heap and retained heap of parsing and of batch layout, the sizes of the parsed tree and cascade, streaming peak, and elapsed time; `RAIKIRI_TABLE_TIMING=1` reports times only, without counting (the counting allocator stays installed and idle) | Input generation and cache warm-up |
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
its ten cases and checks formatting context and laid-out content.

`flex_oof` and `grid_oof` add a container that is the containing block of many
absolutely positioned children. Of the `items` boxes, `items / 10` (at least
one) are ordinary items with text and the rest are empty and out of flow. Nine
in ten of the out-of-flow boxes are absolute and one in ten is fixed; the
absolute ones are placed by insets, by their static position (`align-self` and
`justify-self`) or, in the grid, against a grid area. The
fixture exercises the out-of-flow candidates that bubble up from each child and
the positioning pass of the container, so it covers the layout call protocol
of out-of-flow boxes that `flex_*` and `grid_*` do not reach. The timing cases
run as `layout_document/(flex|grid)_oof_{100,1000}`.

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

### Full PDF generation with the three-column table

An integration run on 2026-11-26 used the HTML generated by
`table_memory::table_document`, the Ahem font in
`crates/raikiri-svg/tests/data/Ahem.ttf`, and a release build of Fulgur
(`bd6222fe`, local Raikiri dependency). Fulgur used its public `render_streaming` entry with default
configuration, the font supplied through an asset bundle, and system fonts
disabled.
Each run started a new process and wrote the PDF to disk. Elapsed time
includes the write. RSS is Linux wait4 maximum resident set size in KiB.

| Rows | Renderer | Pages | Seconds, two runs | Peak RSS KiB, two runs |
| ---: | --- | ---: | --- | --- |
| 20,000 | Fulgur | 318 | 14.091, 14.060 | 1,133,076, 1,146,288 |
| 40,000 | Fulgur | 635 | 43.839, 47.853 | 2,243,872, 2,243,932 |

These Fulgur samples precede support for its injected `:where(html)` font
rule and heading bookmark stylesheet in the table-window eligibility check.
They measure the fallback path. Under these same conditions, Fulgur batch and streaming at 1,400
rows both produced 23 pages. Poppler `pdftoppm -r 72` produced identical
34,626,753-byte PPM streams with SHA-256
`38b3ba89a923eda367da21e4711fa7d8d0cbe5c43f5d0d50890988e349413c5e`.

With the same fixture, replaying one already delivered page at each window
boundary preserved the repeated header. The 1,400-row PDF matched the batch
raster digest above across all 23 pages with 23 window pages reported.
With separate borders, zero border spacing and body margin, and avoided
row breaks, all 24 window pages also matched batch. Before replaying the
preceding page the latter case differed by 976 pixels on each of pages
9 and 17, both in the header area.

| Rows | Renderer | Window pages | Seconds, two runs | Peak RSS KiB, two runs |
| ---: | --- | ---: | --- | --- |
| 20,000 | Fulgur | 318 of 318 | 13.055, 12.965 | 79,700, 79,124 |
| 40,000 | Fulgur | 635 of 635 | 26.846, 27.904 | 82,292, 90,724 |

These development measurements precede skipping redundant intrinsic cell
measurement during the planning and output passes. Some larger runs shared
the host with compilation. The 20,000-row run met the RSS target and exceeded
the 5-second time target.

Reusing the measured intrinsic widths of single-column cells in both later
passes retained all three 1,400-row batch raster hashes. In two further runs
the 20,000-row Fulgur PDF took 11.366 and 11.367 seconds at 82,236 and
83,624 KiB, and the 40,000-row PDF took 22.482 and 22.314 seconds at 85,992
and 87,132 KiB. Diagnostics attributed all 318 and 635 pages to window layout.

At 20,000 rows the same release build produced 318 pages with either Fulgur
entry point. Each page's width, height, and SHA-256 over its 72 dpi Poppler
RGB raster matched. The SHA-256 over the 318 concatenated binary per-page
digests was `e198db7f062a23d8b6cc24bc3f5adf2c975423c52b2ef178d8f8be1b8c30c43c`.
Streaming diagnostics counted 318 window pages and no checkpoint snapshots.

A phase-timed 20,000-row run took 11.387 seconds: 3.734 seconds measuring
columns, 3.293 seconds validating the page plan, and 4.244 seconds laying
out and delivering it to the PDF sink. Returning a width-only box during
normal layout lowered the measurement phase to 2.422 seconds, but triggered
the geometry invariant for negative content-box extents on collapsed-border
tables. That experiment was reverted. A dedicated measurement entry must
finish before the placed-box validation and page projection stages.

With a dedicated horizontal column query, two 20,000-row runs produced 318
window pages in 9.143 and 9.114 seconds at 83,888 and 79,664 KiB peak RSS.
Column measurement took 1.561 and 1.538 seconds, page planning 3.216 and
3.247 seconds, and delivery 4.261 and 4.223 seconds.
In a separate 1,400-row check, the name in zero-based row 1,250 started with
36 consecutive W characters. SHA-256 of the full 72 dpi PPM stream matched
batch rendering for each of three styles: the baseline (23 pages), zero body
margin plus avoidance of internal row breaks (23), and that same style with
separate borders and zero border spacing (24).
At 40,000 rows the dedicated query produced 635 window pages in 18.302
and 18.590 seconds at 81,632 and 87,064 KiB peak RSS. The second Fulgur run
overlapped a debug test build, so elapsed times include possible contention.

For the dedicated-query build, the 20,000-row three-column Ahem input also
matched batch output across the entire 72 dpi raster stream. The SHA-256 over
the concatenated PPM bytes from `pdftoppm -r 72` was
`6c3d892b17ed849f733620e14da7fed7acfea3c437873ad56d87d17a65f3594f`
for both modes. Streaming diagnostics reported 318 window pages. To collect
phase timings in the paired Fulgur worktree, build the probe with the
`fulgur-raikiri/streaming-diagnostics` feature.

With reversible page painting enabled in the Fulgur PDF sink, the same
20,000-row input took 5.968 and 5.835 seconds at 82,924 and 81,288 KiB.
All 318 pages were painted while validating the windows, with no second
layout for delivery. Column measurement took 1.553 and 1.560 seconds and
planning including painting took 4.302 and 4.166 seconds.
The full 72 dpi PPM digest matched the batch reference above.
At 40,000 rows, reversible painting produced 635 window pages in 11.849
and 11.754 seconds at 86,480 and 86,240 KiB peak RSS.

Increasing the window from 512 to 768 rows produced 318 pages in 5.896
and 5.892 seconds at 100,740 and 98,144 KiB for the 20,000-row input.
This overlapped the 512-row timings and raised peak RSS. The production
window remains 512 rows.

Splitting each 512-row measurement window between two simultaneous horizontal
queries reduced column measurement to 0.958 seconds in both 20,000-row runs.
End-to-end times were 5.234 and 5.300 seconds, with 88,592 and 85,572 KiB
peak RSS and all 318 pages painted during planning.
With two simultaneous column queries, the full 20,000-row raster digest
also matched the batch reference. At 40,000 rows, Fulgur painted all 635
window pages in 10.594 and 10.661 seconds at 92,784 and 93,296 KiB peak RSS.

Using up to four queries within the same 512-row window (no more than one
query per 128 rows) produced 318 pages in 5.077 and 5.027 seconds at 90,388
and 90,224 KiB peak RSS. Column measurement took 0.712 and 0.694 seconds.

The four-query path matched the full 20,000-row batch raster digest. At
40,000 rows it produced all 635 window pages in 10.192 and 10.158 seconds,
with peak RSS of 94,664 and 97,600 KiB.

Building each measurement document on its query worker as well took 5.020
and 5.048 seconds at 91,600 and 91,248 KiB for 20,000 rows, and 10.224 and
10.243 seconds at 95,028 and 94,772 KiB for 40,000 rows. Both counts used
window painting throughout (318 and 635 pages). These times overlap the
preceding measurements and do not establish an end-to-end speedup.

Of the 4.209 and 4.240 seconds in planning on two 20,000-row runs, window
assembly and layout consumed 2.992 and 3.008 seconds, and the sink's
tentative paint callback consumed 0.866 and 0.879 seconds. End-to-end times
were 5.011 and 5.053 seconds. An experiment storing the cumulative DOM node
count alongside each row offset and reserving the exact window arena size
took 5.212 and 5.059 seconds at 89,172 and 90,540 KiB. This did not shorten
the run, so the extra row index and reservation were removed.

Sizing the next read to a multiple of the preceding complete page's row
count plus one confirming row, within the same lookahead bound, produced
318 window pages in 5.028, 4.960 and 4.982 seconds at 92,788, 92,776 and
94,048 KiB. The 72 dpi digest across all pages
matched the batch reference. Page acceptance still checks actual layout.
At 40,000 rows the same read-ahead policy produced 635 pages in 10.219
and 10.327 seconds with 96,772 and 97,052 KiB peak RSS.

After preserving document source IDs across windows, two 20,000-row runs
took 5.094 and 5.270 seconds at 90,148 and 90,692 KiB. At 40,000 rows
they took 10.542 and 10.640 seconds at 95,440 and 98,624 KiB. All 318
and 635 pages used window layout.
Resolving just the source and line identifiers for paint order, without
cloning glyph buffers, produced the same full 318-page reference raster.
Two 20,000-row runs took 5.057 and 5.041 seconds at 90,408 and 88,212 KiB.

At 40,000 rows this build produced 635 window pages in 10.278 and 10.250
seconds at 98,976 and 97,836 KiB peak RSS.

A window-width comparison used Raikiri f86907411 and Fulgur a7413d9e.
In a scratch build the production lookahead (512) was supplied at runtime.
The input contained a 54,000-space prefix comment and 600 two-cell rows
(numbered 0 through 599, second cell text "row"). Pages measured 240 by
150 CSS px, with zero page and body margins, Ahem at 12px with 16px line
height, collapsed 1px solid cell borders, 21.25px cell height, 1.25px
padding and break-inside:avoid on rows. The public batch and streaming
entry points each produced 100 pages. Diagnostics confirmed 100 window
pages at each lookahead of 14, 18 and 31. Poppler pdftoppm at 72 dpi
produced identical PPM bytes per page in all four runs. SHA-256 over
the concatenated binary SHA-256 digests of those PPM files in page order
was 29b8796d2ef49b620a0289dfba4a7e3669dc00f13fea8aaa090ebe57f6eee9e5.
