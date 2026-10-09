//! `paint_document`'s per-element DFS walk hot-loop benchmark.
//!
//! # Why this benchmark exists
//!
//! `crates/raikiri-paint/src/walk.rs`'s `paint_document` runs an iterative
//! DFS over every rendered node once per paint. Its `Element` arm reads
//! `cascade.computed[node_id]` — a per-node `ComputedValues` (248 bytes on
//! this workspace's build, `size_of::<raikiri_style::computed::ComputedValues>()`)
//! — to resolve `vertical_align`/`display`/`font_size` for that node's
//! `vertical-align` shift. Previously only the `Text` arm touched
//! `cascade.computed` at all; every rendered `Element` now does too. That is
//! exactly the class of change `crates/raikiri-style/benches/cascade.rs`'s
//! own module doc describes catching in the cascade hot loop (a per-item
//! read/copy cost change that neither `cargo test` nor `cargo clippy` can
//! see, because the output is bit-identical and no lint fires) — this file
//! gives `walk.rs`'s DFS the same instrument `cascade.rs` already gives
//! the cascade walk, so a future change to the paint walk's
//! per-node cost has something to regress against instead of landing silent.
//!
//! Unlike `cascade.rs`, this file does not carry validated before/after
//! numbers for a specific regression — it is instrumentation laid down ahead
//! of one. See `cascade.rs`'s own module doc for the noise caveats
//! (contention from concurrent `rustc`/gate runs can move criterion's
//! numbers by 10–40%, so a bare `p < 0.05` "regressed" verdict on a shared
//! machine is not evidence on its own) and the interleaved min-of-N protocol
//! that does work — both apply here unchanged.
//!
//! # What's measured, and what isn't
//!
//! Only [`raikiri_paint::paint_single_page`] is timed. The synthetic
//! `Document` is built, cascaded, and laid out (`layout_single_page`, which
//! also lays every paragraph out with the inline engine) once per config,
//! *outside* the timed closure — those are one-time, `O(config)` costs the
//! walk itself does not pay repeatedly. `paint_single_page` calls
//! `paint_canvas_background` (a documented no-op today) and then
//! `paint_document`, so in practice this measures the DFS walk plus the
//! `anyrender::Scene` recording backend's per-command bookkeeping — the same
//! sink `crate::tests` already uses elsewhere in this crate, chosen over
//! `anyrender::NullScenePainter` because `NullScenePainter`'s `draw_glyphs`
//! ignores its `glyphs` parameter without iterating it, which would silently
//! skip the per-glyph conversion into `anyrender::Glyph` that a real backend
//! always drives to completion.
//!
//! # Workload shape
//!
//! `<html><head></head><body>` followed by `n_elems` flat `<div>` children,
//! each with one `"x"` text child — the same flat, one-text-child-per-element
//! topology `cascade.rs`'s `BenchDoc::new` uses, for the same reason: every
//! `div` has no `div` ancestor, so this shape cannot exercise selector
//! matching or ancestor-walk costs (`build_rule_tree`/`cascade` run with an
//! empty stylesheet here — there is no CSS to match), keeping the timed
//! region a clean read on `paint_document`'s traversal-plus-glyph-recording
//! cost rather than a mix of that and cascade/layout concerns those two
//! files already benchmark on their own.
//!
//! Every `div` is a block box holding one paragraph, `"x"`, which the inline
//! engine lays out; the walk draws each paragraph's lines once.
//!
//! # Does it actually walk the whole document? (checked, not assumed)
//!
//! [`workload`] asserts the recorded `GlyphRun` count equals `n_elems` after
//! a throwaway `paint_single_page` call at setup, outside any timed region —
//! mirroring `cascade.rs`'s "the assertions are the load-bearing part" note.
//! A change that walked only part of the tree (e.g. an off-by-one in the
//! `is_display_none` gate, or a stack-frame bug that dropped a subtree) would
//! silently shrink the loop and report a speedup; this catches that class by
//! construction rather than by inspection.
//!
//! # Running it
//!
//! ```text
//! cargo bench -p raikiri-paint --bench walk
//! ```
//!
//! Nothing in the merge gate runs this — same posture as `cascade.rs` (see
//! its module doc's "Running it" section).

// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
// never build this bench target, so nothing in this file has coverage
// instrumentation to attribute to.
use anyrender::Scene;
// cov:ignore: bench harness — same reason as above.
use anyrender::recording::RenderCommand;
// cov:ignore: bench harness — same reason as above.
use criterion::{Criterion, Throughput};
// cov:ignore: bench harness — same reason as above.
// cov:ignore: bench harness — same reason as above.
use raikiri_dom::{Document, layout_single_page};
// cov:ignore: bench harness — same reason as above.
use raikiri_paint::paint_single_page;
// cov:ignore: bench harness — same reason as above.
use raikiri_style::{CascadeResult, build_rule_tree, cascade};
// cov:ignore: bench harness — same reason as above.
use raikiri_traits::PageBox;
// cov:ignore: bench harness — same reason as above.
use taffy::Style;

/// Shared `.expect()` message for this file's `cascade()` and
/// `layout_single_page()` setup calls.
///
/// The two guarantees behind it are not the same strength. `cascade()`
/// mirrors `cascade.rs`'s `CASCADE_NEVER_ERRS`: it is unconditionally
/// infallible in the current implementation (`CascadeError` is never
/// constructed anywhere). `layout_single_page()` is not — it returns
/// `Err(LayoutError::Internal)` when the document has no `<body>` — so this
/// message's use at that call site relies on [`build_doc`] always including
/// one, not on any general guarantee `layout_single_page` itself makes.
// cov:ignore: bench harness constant — `cargo test`/`cargo llvm-cov
// --workspace` never build this bench target, so nothing in this file has
// coverage instrumentation to attribute to (see `cascade.rs`'s identical
// note).
const INFALLIBLE_SETUP: &str = "cascade/layout are Ok for this benchmark's fixed synthetic input";

/// Build `<html><head></head><body>` with `n_elems` flat block `<div>`
/// children, each holding one `"x"` text child. The cascade here has no UA
/// sheet, so the block displays are authored.
// cov:ignore: bench harness — same reason as `INFALLIBLE_SETUP` above.
fn build_doc(n_elems: usize) -> Document {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    for _ in 0..n_elems {
        let div = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
        doc.append_text(div, "x");
    }
    doc
}

/// Assemble one `n_elems` config (cascade + layout, both outside any timed
/// region) and prove `paint_single_page` walks the whole thing — see the
/// module doc's "Does it actually walk the whole document?" section.
// cov:ignore: bench harness — same reason as `INFALLIBLE_SETUP` above. The
// assertions below are the actual correctness check (run once at setup,
// outside any timed region, and would panic on failure) — coverage
// instrumentation is what's structurally unavailable here, not testing.
fn workload(n_elems: usize) -> (Document, CascadeResult) {
    let mut doc = build_doc(n_elems);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect(INFALLIBLE_SETUP);
    layout_single_page(&mut doc, &cr, PageBox::A4).expect(INFALLIBLE_SETUP);

    let mut probe = Scene::new();
    paint_single_page(&mut probe, &doc, &cr, PageBox::A4).expect("paint succeeds");
    let glyph_runs: Vec<_> = probe
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::GlyphRun(g) => Some(g),
            _ => None,
        })
        .collect();
    assert_eq!(
        glyph_runs.len(),
        n_elems,
        "expected one GlyphRun per div (each has exactly one \"x\" text child), \
         got {} for {n_elems} elements — the walk did not visit the whole document",
        glyph_runs.len(),
    );

    (doc, cr)
}

/// `n_elems` configs spanning two orders of magnitude (100 to 10,000): wide
/// enough that a genuine per-element cost change shows up as a consistent
/// relative delta across every config, rather than as an artifact of one
/// absolute measurement.
// cov:ignore: bench harness — same reason as `INFALLIBLE_SETUP` above.
const ELEMENT_COUNTS: [usize; 3] = [100, 1_000, 10_000];

// cov:ignore: bench harness — same reason as `INFALLIBLE_SETUP` above.
fn bench_paint_walk(c: &mut Criterion) {
    let mut group = c.benchmark_group("paint_walk");

    for n_elems in ELEMENT_COUNTS {
        let (doc, cr) = workload(n_elems);

        // Denominated in elements (divs), matching the module doc's
        // "Workload shape" note that the fixed html/head/body ancestors are
        // negligible next to `n_elems`.
        group.throughput(Throughput::Elements(n_elems as u64));
        group.bench_function(format!("elements_{n_elems}"), |b| {
            // `iter_with_large_drop`, not plain `b.iter` — mirrors
            // `cascade.rs`: each iteration allocates a fresh `Scene` (a
            // `Vec<RenderCommand>` sized to the walk's output), and freeing
            // that Vec is real work this benchmark does not want folded into
            // the walk's own per-node cost.
            b.iter_with_large_drop(|| {
                let mut scene = Scene::new();
                paint_single_page(&mut scene, &doc, &cr, PageBox::A4).expect("paint succeeds");
                scene
            });
        });
    }

    group.finish();
}

// Inlined `criterion_main!`/`criterion_group!` body — mirrors
// `crates/raikiri-style/benches/cascade.rs`'s rationale: the macro-generated
// `pub fn` trips the workspace's `missing_docs = "warn"` (a hard error under
// this workspace's `-D warnings` lint policy), and the `#[allow]` cannot
// attach to a macro invocation.
// cov:ignore: bench harness — same reason as `INFALLIBLE_SETUP` above.
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_paint_walk(&mut criterion);
    criterion.final_summary();
}
