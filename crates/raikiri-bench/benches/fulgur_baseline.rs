//! fulgur v0.12 baseline parity harness — first landing.
//!
//! # Why this benchmark exists
//!
//! Raikiri's failure condition is explicit: **if raikiri renders slower than
//! fulgur v0.12, the migration this project exists to enable has negative
//! economic value**, independent of how much correctness (WPT pass rate,
//! nested multicol, etc.) improves. A concrete evidence point — fulgur
//! v0.12's own "100 pages × 100 tables" scenario dropped from 139s to 0.67s
//! (**209×**) — is the baseline raikiri must meet or beat, which calls for a
//! `raikiri-bench` crate to make the comparison runnable instead of
//! aspirational. This file is that crate's first real (non-vaporware)
//! benchmark.
//!
//! The 209×/0.67s/100-pages-×-100-tables numbers come from that decision's
//! record, which cites a Zenn article (<https://zenn.dev/mitzh/articles/93a99ce7201e95>).
//! **They are not re-derived here.** Treat them as the decision's stated
//! target, not as something this benchmark proves.
//!
//! # What could and could not be ported from fulgur
//!
//! The task that produced this file was explicitly allowed to read
//! `/home/mitz/Work/oss/fulgur` (a local clone, working tree on `main`, full
//! history and tags available) for structural reference. That clone was
//! inspected directly, not guessed at:
//!
//! - `git show v0.12.0:Cargo.lock | grep -c '^name = "criterion"'` → `0`, and
//!   `git ls-tree -r v0.12.0 --name-only | grep -i bench` → no matches — this
//!   is the exact tag the benchmark specification's baseline names, not just current `main`.
//!   `git log --all --diff-filter=D -- '**/benches/**' '*bench*'` across the
//!   whole history is also empty, so no such harness existed and was later
//!   deleted either. **fulgur v0.12 (and every version, as far as this
//!   history shows) has no committed benchmark harness to port** — the
//!   100-pages-×-100-tables scenario is not in-repo code at any point, only
//!   an external-article claim quoted by the benchmark specification. So nothing here is a
//!   port; it is a from-scratch construction aimed at the same axis fulgur's
//!   number names.
//! - `crates/fulgur/src/drawables.rs:1-60` (real content, read directly) *is*
//!   available and matches what the decision record and
//!   `crates/raikiri/src/entries.rs`/`page_drawables.rs` already cite: a
//!   `Drawables` struct of per-NodeId side-channel maps (replacing a
//!   `Pageable` trait + 17 impls) plus `TrackedMap`, a `BTreeMap` that also
//!   logs insertion order so "which NodeIds were added since an earlier
//!   point" is `O(inserted-since)` instead of an `O(N²)` snapshot-diff across
//!   a whole document. That is the *architectural* explanation for fulgur's
//!   209× (per the decision's rationale section), not benchmark
//!   *code* — raikiri's own `PageDrawables`/`TrackedMap` (`page_drawables.rs`)
//!   already inherits the shape. This benchmark cannot yet exercise that path
//!   at all: see the PageDrawables note below.
//!
//! # What had to be scoped down, and why
//!
//! A literal "100 pages × 100 tables" port is not possible against raikiri
//! today, on two independent axes:
//!
//! 1. **No page-stream benchmark yet.** `raikiri::plan` remains an explicit
//!    stub, and this benchmark does not call the now-implemented neutral
//!    `raikiri_html::layout` bridge. The only rendering path measured here
//!    is [`raikiri::html_to_png`] /
//!    [`raikiri::html_to_png_with_fonts`]
//!    (`parse_html` → `layout_single_page` → `build_page_scene` →
//!    `PageScene::rasterize`), and it is hard-pinned to a single `PageBox::A4`
//!    page (`crates/raikiri/src/html_to_png.rs` doc comment: "単一ページのみ").
//!    There is no multi-page flow to measure a *pagination* cost against.
//!
//!    **What this file actually does about it:** it does not construct a
//!    page-count axis at all. There is no `n_pages` parameter anywhere in
//!    this file, and [`bench_pages`]'s timed closure calls
//!    `html_to_png_with_fonts` exactly **once** per criterion iteration —
//!    not N times, not in a batch. What the two benchmark variants actually
//!    vary is `n_tables` (10 vs. 100), rendered onto a single synthetic page
//!    each time; that is the *table* axis, not a page axis. Criterion's own
//!    repeated sampling (many iterations per benchmark run, for statistical
//!    robustness) is ordinary benchmarking methodology present in *every*
//!    criterion benchmark regardless of subject matter — it is not, and
//!    should not be read as, a deliberate stand-in for "N pages". In short:
//!    fulgur's page axis is **not represented in this file at all**; only
//!    the table-count axis is measured, at single-page granularity. A future
//!    benchmark that wants a page-count axis should call the neutral
//!    `layout` bridge directly rather than infer pages from this
//!    single-page PNG workload.
//!
//! 2. **No `<table>` layout.** `crates/raikiri-style/src/property.rs`'s own
//!    test pins `assert_eq!(parse("table", "display"), None)` — `display:
//!    table` does not parse in raikiri today, and
//!    `crates/raikiri-html/src/ua/minimal.css` says outright: "`<table>`
//!    presentation are deliberately deferred". `raikiri::TableEntry`
//!    (`crates/raikiri/src/entries.rs`) is a `#[non_exhaustive]` placeholder
//!    with zero fields — `PageDrawables` does not populate it yet, so a
//!    perf-only benchmark cannot exercise it regardless of markup.
//!
//!    **Approximation used here:** the workload uses literal `<table>`/`<tr>`/
//!    `<td>` markup (so html5ever parses the same tag names fulgur's scenario
//!    would), but the synthetic stylesheet declares `table, tr, td { display:
//!    block; }` explicitly — the same "just make it a block box" treatment
//!    `minimal.css` already gives `div`/`p`/headings — rather than relying on
//!    an unparseable `display: table` silently falling back to the initial
//!    value. What this benchmark actually measures is parse + cascade +
//!    block-layout + paint + PNG-encode cost scaling with the *node count* a
//!    100-table page implies (100 tables × rows × cols `<td>`s, each cascaded
//!    against real declarations), **not** the CSS table layout algorithm
//!    itself, which does not exist in raikiri yet.
//!
//! Point 1 is a gap this file leaves open (no page axis at all), and point 2
//! is a genuine approximation (real markup, substitute layout treatment).
//! Both are the "closest feasible using raikiri's actual current rendering
//! entry point" for this benchmark's initial-landing scope, not a
//! claim of parity with fulgur's real scenario. Future work restoring
//! real pagination and/or table layout should replace this file's workload
//! rather than layer a second one beside it, so the "what does
//! `fulgur_baseline_pages` measure" story does not fork.
//!
//! # Explicitly out of scope for this landing
//!
//! Per this benchmark's own initial-landing framing, none of the
//! following are attempted here — they are follow-up work:
//!
//! - CI integration / nightly cron integration (fulgur `wpt-nightly.yml` pattern)
//! - `regressions.json` aggregation → GitHub issue auto-filing
//! - The full 5-axis measurement table (memory/RSS, time-to-first-page,
//!   allocation count, WPT run time) — this file covers only the "rendering
//!   throughput" row, and denominates it in **tables/sec**
//!   (`Throughput::Elements(n_tables)`, see [`bench_pages`]), not pages/sec —
//!   there is no page-count axis in this file (see point 1 above)
//! - A calibrated "X% within fulgur baseline" per-sprint gate threshold —
//!   that needs a real fulgur v0.12 number to compare against, which this
//!   session did not have (only fulgur's current `main`, and no in-repo
//!   fulgur benchmark to run for a fresh number either)
//!
//! # Running it
//!
//! ```text
//! cargo bench -p raikiri-bench --bench fulgur_baseline
//! ```
//!
//! Rendering (font shaping + software rasterization + PNG encoding) is far
//! slower per call than `raikiri-style`'s `cascade()` unit, so this file
//! trades sample count for wall-clock budget explicitly (`Criterion::
//! sample_size`, see [`bench_pages`]) rather than using criterion's default
//! 100 samples.
//!
//! # Measured once — not a calibrated threshold
//!
//! Two back-to-back runs of this **unmodified** file, same host (AMD Ryzen 5
//! 5600G, 12 threads — this repo runs several parallel worktree sessions, so
//! "idle" is not guaranteed):
//!
//! | config              | run 1 (quick flags) | run 2 (defaults) |
//! |---------------------|---------------------|-------------------|
//! | `page_tables_10`    | 15.96 ms            | 32.19 ms          |
//! | `page_tables_100`   | 65.79 ms            | 114.07 ms         |
//!
//! Run 2 reported `change: +100.17%` / `+73.39%`, `p = 0.00 < 0.05`,
//! "Performance has regressed" against run 1 — with **zero source changes**
//! between them. This is the same measurement-noise failure mode
//! `cascade.rs`'s module doc warns about (contention from concurrent
//! `rustc`/`cargo` activity), reproduced live while writing this file rather
//! than asserted from that file's numbers. **Do not treat either column as
//! "the" baseline** — they bound a range this machine produced under unknown
//! concurrent load, not a calibrated gate threshold. A real per-sprint gate
//! needs the interleaved min-of-N protocol `cascade.rs` documents (or
//! equivalent), which is explicitly follow-up work (see above), not something
//! this file's single-shot criterion run provides.
//!
//! # What's actually inside the timed region
//!
//! `bench_pages`'s timed closure calls `font_ctx.clone()` on every iteration
//! (see [`FontContext`]'s call site) rather than passing a shared reference —
//! [`raikiri::html_to_png_with_fonts`] takes `FontContext` by value. Measured
//! directly on this host (1000 clones, outside any criterion timing): **~157
//! ns/clone**, i.e. roughly 0.0002% of a `page_tables_100` iteration (~66 ms).
//! The measured unit is therefore "clone + render", not "render" in
//! isolation, but the clone's share is negligible next to the render cost —
//! unlike the alternative of calling `FontContext::new()` per iteration,
//! which would re-run system font discovery on every sample and dominate the
//! measurement instead.

use criterion::{Criterion, Throughput};
use raikiri::{FontContext, ParseOptions, html_to_png_with_fonts, parse_html};
use std::fmt::Write as _;

// Rows/cols per generated `<table>`. Kept small (relative to a real-world
// table) so the "100 tables" axis — the thing this benchmark actually
// targets — is not swamped by an arbitrarily large per-table cost; see the
// module doc's scope-down note on table layout not existing yet.
const ROWS: usize = 4;
const COLS: usize = 4;

/// Build one synthetic HTML page containing `n_tables` `<table>` elements of
/// `ROWS` × `COLS` `<td>` cells each.
///
/// The inline `<style>` block is registered by raikiri-html's head-`<style>`
/// support (`crates/raikiri-html/src/sink.rs`), the same mechanism
/// `minimal.css`-style UA rules use — no `ParseOptions::extra_stylesheets`
/// integration logic is needed, so this stays reachable through the public
/// `html_to_png`/`html_to_png_with_fonts` entry points as-is.
///
/// `table, tr, td { display: block; }` is **not** load-bearing for the layout
/// path taken today: `crates/raikiri-dom/src/layout.rs` maps
/// `DisplayValue::Inline`/`InlineBlock` to `Display::Block` right alongside
/// `Display::Block` itself (only `none` diverges), so these elements would
/// land on the same block-layout code path even at the `display` initial
/// value. The rule is declared anyway for two reasons that do matter: it
/// documents intent explicitly rather than relying on that layout-side
/// collapse as an implicit assumption a future layout change could silently
/// invalidate, and it adds a genuine (if small) cascade match + resolve cost
/// per element that a workload claiming to approximate "real declarations"
/// should not skip. See the module doc's point 2 for why `display: table`
/// itself is not an option (`property.rs` pins it as unparseable).
fn build_page_html(n_tables: usize) -> String {
    let mut body = String::with_capacity(n_tables * ROWS * COLS * 24);
    for t in 0..n_tables {
        body.push_str("<table>");
        for r in 0..ROWS {
            body.push_str("<tr>");
            for c in 0..COLS {
                let _ = write!(body, "<td>{t}-{r}-{c}</td>");
            }
            body.push_str("</tr>");
        }
        body.push_str("</table>");
    }
    format!(
        "<!DOCTYPE html><html><head><style>\
         table, tr, td {{ display: block; }}\
         td {{ padding-top: 1px; padding-right: 2px; padding-bottom: 1px; \
         padding-left: 2px; color: #222222; font-size: 10px; }}\
         </style></head><body>{body}</body></html>"
    )
}

/// Sanity-check the workload, once, outside any timed region — same "prove
/// the workload is what it claims to be" posture as `cascade.rs`'s
/// `workload()`. Two independent checks, because either alone can pass on a
/// broken pipeline:
///
/// - **DOM side**: a well-formed PNG with an *empty* page would still pass a
///   PNG-magic-only check. `html_to_png`/`_with_fonts` do not expose the
///   intermediate DOM, so this parses the same bytes a second time via
///   [`raikiri::parse_html`] (throwaway, untimed) and asserts the cascaded
///   node count clears a floor of 2 nodes per cell (the `<td>` element itself
///   plus its text child) — `n_tables * ROWS * COLS * 2`. This is a floor,
///   not an exact count: html5ever's table insertion mode auto-inserts a
///   `<tbody>` around bare `<tr>` children, and `<table>`/`<tr>` themselves
///   also count, so the true total is somewhat higher. A regression that
///   dropped every cell (foster-parenting, a sink bug, etc.) would fail this;
///   the exact `<tbody>`/`<tr>`/`<table>` overhead does not need pinning for
///   that purpose.
/// - **Raster side**: the actual timed function's own output must be a
///   well-formed, non-trivial PNG (magic bytes present).
fn probe(html: &str, font_ctx: &FontContext, n_tables: usize) {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(html.as_bytes(), &opts).expect("parse_html for probe");
    let node_floor = n_tables * ROWS * COLS * 2;
    let node_count = doc.cascade().computed.len();
    assert!(
        node_count >= node_floor,
        "expected at least {node_floor} cascaded nodes ({n_tables} tables x \
         {ROWS}x{COLS} cells, floor of 2 nodes/cell) but got {node_count} — \
         table markup did not reach the DOM at the expected scale (dropped \
         cells? foster-parenting?)"
    );

    let png =
        html_to_png_with_fonts(html.as_bytes(), font_ctx.clone()).expect("html_to_png_with_fonts");
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
    assert!(
        png.len() > 8 && png[..8] == PNG_MAGIC,
        "probe render did not produce a well-formed PNG (got {} bytes)",
        png.len()
    );
}

/// "100 tables" — the table-count axis of fulgur's named scenario, preserved
/// literally (see module doc for what could not be preserved alongside it).
///
/// `sample_size` is lowered from criterion's default (100) because each
/// sample here is a full parse→cascade→layout→paint→encode render, not a
/// microsecond-scale pure function — see the module doc's "Running it"
/// section. 20 was chosen as a first-landing budget (a handful of seconds
/// total on the author's machine); raising it once real per-sprint gate
/// timing budgets are known is expected follow-up, not a defect to fix now.
fn bench_pages(c: &mut Criterion) {
    let mut group = c.benchmark_group("fulgur_baseline_pages");
    group.sample_size(20);

    let font_ctx = FontContext::new();

    for n_tables in [10usize, 100usize] {
        let html = build_page_html(n_tables);
        probe(&html, &font_ctx, n_tables);

        // Throughput is denominated in tables-per-second: table count is the
        // axis under study, and unlike `cascade.rs`'s declaration-count
        // denominator, table count here does not vary within one config, so
        // reading this as "how long the batch would take with n_tables"
        // stays a fair per-config comparison even between the two configs.
        group.throughput(Throughput::Elements(n_tables as u64));
        group.bench_function(format!("page_tables_{n_tables}"), |b| {
            // Plain `b.iter`, not `cascade.rs`'s `iter_with_large_drop`: that
            // constraint exists there because `CascadeResult` owns a
            // per-node `Vec<ComputedValues>` whose free time is comparable to
            // the ~ms-scale operation under study. Here the timed region
            // drops one `Vec<u8>` PNG buffer against a tens-of-ms render —
            // the drop is not a comparable fraction of the measurement, so
            // the constraint does not transfer.
            b.iter(|| {
                // `font_ctx.clone()` runs inside the timed closure — see the
                // module doc's "Measured once" section for why this is a
                // deliberate, measured trade rather than an oversight:
                // cloning is ~157ns, negligible next to a tens-of-ms render,
                // while re-running `FontContext::new()` per iteration would
                // re-enumerate system fonts and dominate the measurement.
                let png = html_to_png_with_fonts(html.as_bytes(), font_ctx.clone())
                    .expect("html_to_png_with_fonts must succeed");
                assert!(!png.is_empty());
            });
        });
    }

    group.finish();
}

// Inlined `criterion_main!`/`criterion_group!` body — mirrors
// `crates/raikiri-style/benches/cascade.rs`'s rationale: the macro-generated
// `pub fn` trips the workspace's `missing_docs = "warn"` (a hard error under
// gate `-D warnings`), and `#[allow]` cannot attach to a macro invocation.
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_pages(&mut criterion);
    criterion.final_summary();
}
