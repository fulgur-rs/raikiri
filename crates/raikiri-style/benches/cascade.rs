//! Cascade hot-loop benchmark — guards the **per-declaration constant**.
//!
//! # Why this benchmark exists
//!
//! bd raikiri-spike-nqkj added a shorthand-expansion guard to
//! `cascade::collect_cascaded` and, in doing so, started moving a
//! `Declaration` (72 bytes) by value once per declaration. The declaration
//! *count* did not change — `collect_cascaded` is a single whole-tree walk
//! either way — only the cost of one loop iteration did (see below for the
//! per-declaration figures, which differ by source). That is
//! a regression class neither `cargo test` nor `cargo clippy` can see: the
//! output is bit-identical and no lint fires. It was caught only because a
//! reviewer hand-rolled a throwaway timing harness (raikiri-spike-nqkj gate
//! §8.2, reviewer:perf, CONFIRMED at +22% / +16%).
//!
//! This file makes that harness a repo artifact so the same class is
//! measurable on demand instead of by luck.
//!
//! # Reference numbers
//!
//! From the nqkj gate §8.2 review, default release profile, wall-clock per
//! `cascade()` call. Note nqkj recorded the harness **twice**: this table is
//! the review's run, and the landing commit (`7ae92e9`) reports a re-run at
//! min-of-30 × 2 rounds giving +25% / +14% for the same two configs. Both are
//! the origin's; neither closes the gap discussed below, since which pairing
//! you take drives the light config's answer (+12.2% or +18.0%).
//!
//! | workload                            | good     | regressed | delta |
//! |-------------------------------------|----------|-----------|-------|
//! | 50 rule × 500 elem × 10 decl (250k) | ≈4.9 ms  | ≈6.0 ms   | +22%  |
//! | 5 rule × 2000 elem × 10 decl (100k) | ≈3.1 ms  | ≈3.6 ms   | +16%  |
//!
//! What moved is the *delta*, not the absolute per-declaration cost — and the
//! absolute is **not** comparable between the configs anyway (see the note on
//! [`Throughput`] at the call site). Per declaration these rows give roughly
//! 4 ns (rule_heavy) and 5 ns (element_heavy) — one significant figure,
//! because the origin recorded spreads (4.87–4.96 → 5.96–6.13 and
//! 3.06–3.10 → 3.62 ms) that this table has already rounded. The ≈4 ns figure
//! quoted elsewhere in this file is from the **validated** table below (4.46
//! and 3.63 ns), not from these rows — see the note directly beneath.
//!
//! The validated table further down reproduces +22.1% for rule_heavy but
//! **+10.7% rather than +16%** for element_heavy, and that gap is **not
//! reconciled**. The obvious candidate does not survive arithmetic: the origin
//! harness built its DOM through `raikiri::parse`, so it carried the UA sheet
//! too — but of its ten rules exactly one matches a `div`, adding 2000
//! declarations to element_heavy's 100k (+2%) while adding selector-matching
//! work to every element, which moves the *relative* delta down, not up. So it
//! has both the wrong magnitude and the wrong sign. Resist the temptation to
//! explain the gap away: the origin was a single throwaway harness measuring a
//! quantity this file goes on to show contention can inflate by up to 40%.
//! Treat the numbers below as this file's baseline, at whichever base each
//! one names.
//!
//! **Absolute numbers here are host-relative.** Everything below was measured
//! on an AMD Ryzen 5 5600G (12 threads). Quote the host whenever you add a
//! number, and compare against a baseline you took on the same machine —
//! never against these.
//!
//! Healthy baseline at the base this landed on (`832500e`): **4.955 ms**
//! (rule_heavy) and **3.276 ms** (element_heavy) — minimum of 6 runs on an
//! idle machine. That is a single-variant minimum, not the two-variant
//! interleaved protocol below, which needs a second tree to alternate against.
//! The sensitivity A/B was measured one base earlier (`2cca455`) at
//! 5.054 / 3.387 ms; the two bases agree to 2.0% and 3.3% respectively, so the
//! +22.1% and +10.7% figures remain representative of the committed workload.
//!
//! # Measurement noise — read this before believing any number
//!
//! Several worktree sessions work this repo in parallel, and a concurrent
//! `rustc` moves these numbers by 10–40%: the same order as the regression
//! they exist to detect. On a tree with **zero** source changes, measured
//! against a baseline saved minutes earlier, criterion reported
//! `+18.777%` and `+11.450%`, both at `p = 0.00 < 0.05`, both labelled
//! "Performance has regressed".
//!
//! Nothing had regressed; four `rustc` processes had started. criterion's
//! p-value only asks whether two sample sets came from the same
//! distribution, so it cannot tell contention from a code change — **`p <
//! 0.05` is not evidence of a regression here**.
//!
//! ## The protocol that does work: interleaved min-of-N
//!
//! Contention only ever *adds* time, so the minimum over repeated runs
//! converges on the true cost while the mean and the p-value do not.
//! Interleaving matters as much as the minimum: it stops a slow stretch of
//! machine landing entirely on one side.
//!
//! ```text
//! # build each tree, save target/release/deps/cascade-<hash> aside
//! for i in $(seq 8); do
//!   for v in good bad; do
//!     ./cascade_$v --bench --warm-up-time 0.5 --measurement-time 1.2 \
//!                  --sample-size 10 --noplot
//!   done
//! done
//! # then compare the per-benchmark minimum across runs
//! ```
//!
//! # Does it actually catch the thing? (validated, not assumed)
//!
//! A benchmark whose sensitivity was never tested is decoration. This one was
//! checked by reintroducing the nqkj regression — `expand_shorthand_into`
//! back to `d: Declaration` by value, `expand_none` likewise, call sites
//! passing `decl.clone()` — and running the protocol above, n = 8
//! interleaved:
//!
//! | workload      | good (min) | regressed (min) | delta  | per declaration |
//! |---------------|-----------:|----------------:|-------:|----------------:|
//! | rule_heavy    |   5.054 ms |        6.169 ms | +22.1% |       4.46 ns   |
//! | element_heavy |   3.387 ms |        3.750 ms | +10.7% |       3.63 ns   |
//!
//! rule_heavy's +22.1% reproduces the origin's +22% directly.
//!
//! **What the cross-config agreement establishes, and what it does not.** The
//! configs differ 4× in node count, 4× in element count, and 2.5× in
//! declaration count. If the delta were proportional to nodes rather than to
//! declarations, the per-declaration figures above would differ by
//! `(4001/100k) / (1001/250k)` ≈ **10×** — and by the same 10× under an
//! element-count hypothesis. They in fact agree within **23%**, so the delta
//! demonstrably does not scale with node or element count.
//!
//! What the agreement does **not** do is separate "per declaration" from
//! "per rule-match": both configs fix `DECLS_PER_RULE`, so
//! declarations are exactly 10× rule-matches in each and the two hypotheses
//! predict identical numbers. Breaking that ratio needs a third config, e.g.
//! 500 rules × 500 elements × 1 declaration — which holds *declarations*
//! fixed at 250k against rule_heavy while multiplying *rule-matches* by 10,
//! so a per-match cost would show up as a ~10× larger delta and a
//! per-declaration cost as an unchanged one. Not implemented (bd
//! raikiri-spike-0z68, which owns it); until it is, read the ≈4 ns as
//! *consistent with* a per-declaration constant rather than as proof of one.
//! Note it is not a drop-in. `DECLS_PER_RULE` is read by the per-rule parse
//! assertion and by the throughput denominator, and is *duplicated* as a
//! hardcoded ten-declaration rule body in [`stylesheet`] — which is exactly
//! why that parse assertion exists, since it is the only thing tying the
//! literal to the constant. Beyond those, the probe's ten-property comparison
//! and the [`Winners`] vacuity guard both assume the ten-longhand body: with a
//! one-declaration rule, nine of the ten comparisons would be against values
//! the stylesheet never set, so the probe does not merely weaken — it fails.
//! Every one of those sites has to move together.
//!
//! Gate integration (bd raikiri-spike-iebo) has to solve the noise problem
//! before any threshold means anything; a naive 10% trigger on this machine
//! fires on an unmodified tree.
//!
//! # Design constraints (do not "simplify" these away)
//!
//! - **Only `cascade()` is timed.** The `RuleTree` is parsed once, outside
//!   the measured closure. Parsing is far more expensive per declaration
//!   than cascading is, so folding it in would bury a 4 ns/declaration
//!   change under noise.
//! - **The result is dropped outside the timed region**
//!   (`Bencher::iter_with_large_drop`). `CascadeResult` owns a
//!   `Vec<ComputedValues>` sized to the node count; freeing it is real work
//!   that is not part of the loop under study, and the reference numbers
//!   above excluded it.
//! - **Two configs varying the ratios.** At fixed `DECLS_PER_RULE` they
//!   differ 10× in rules-per-element and 4× in node count, which is what lets
//!   cross-config agreement rule out node- and element-count scaling. It is
//!   *not* a clean phase split: `collect_cascaded` is `O(element × matching
//!   rule × declaration)`, and `resolve_inheritance` is **not** `O(node)` —
//!   it calls `pick_winners`, which rescans every candidate declaration of
//!   every node, so it is `O(node + element × matching rule × declaration)`
//!   too. Both phases carry a per-declaration term.
//! - **Longhand declarations only.** This is bookkeeping, not code-path
//!   selection: `parse_declaration_block` already routes through
//!   `expand_shorthand_into`, so `margin: 1px` would land the same four
//!   longhands and the cascade would see an identical declaration list. What
//!   a shorthand would break is the accounting — one source declaration
//!   becomes four, desyncing `DECLS_PER_RULE` from reality.
//! - **Every rule matches every element** (bare `div` type selector), which
//!   is what makes rule count multiply into the inner loop.
//!
//! # What the setup guards do and do not prove
//!
//! `workload()` sweeps the whole arena, comparing against the **exact** values
//! the winning (last) rule sets: all ten declarations on every element, and
//! the two inherited ones (`font-size`, `color`) on every text child — the
//! eight box longhands are non-inherited, so text nodes legitimately do not
//! carry them. That proves every element matched, every text child inherited,
//! and each property resolved to the final rule's value rather than to an
//! initial or an intermediate one. A change that skipped nodes, processed only
//! some properties, or stopped at an earlier rule for some property is caught:
//! the benchmark cannot silently time an empty or shrunken loop on those axes.
//!
//! It does **not** prove that all `n_rules` rules were *processed*. The
//! cascade keeps one winner per property, so under last-declaration-wins the
//! losing rules leave no trace in `ComputedValues` — a change that pushed only
//! the final rule's declarations as candidates would produce identical output,
//! while the throughput denominator (`n_rules × n_elems × DECLS_PER_RULE`)
//! kept claiming the full count. That property is not observable through
//! `cascade()`'s public output, and pinning it is the job of the cascade's own
//! unit tests in `crates/raikiri-style/src/cascade.rs`, which assert
//! specificity, origin and source-order resolution directly. If you are
//! reading a suspiciously large improvement, check those tests still pass
//! before believing it.
//!
//! # Running it
//!
//! ```text
//! cargo bench -p raikiri-style --bench cascade
//! ```
//!
//! On a second run this prints criterion's `change:` and p-value lines.
//! **Those are not evidence for this regression class** — see the noise
//! section above, and use the interleaved min-of-N protocol instead.
//!
//! Nothing in the merge gate runs this: `cargo test` does not build the bench
//! target and `cargo clippy --all-targets` compiles it without executing it.
//! Wiring it into the discipline is bd raikiri-spike-iebo (a rules change, so
//! it goes through retro → bd decision → human approve).

use criterion::{Criterion, Throughput};
use raikiri_style::{
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedValues, CssColor, Origin,
    RuleTree, StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, cascade,
};

/// Declarations emitted per generated rule. Kept at 10 to match the
/// reference numbers in the module doc.
const DECLS_PER_RULE: usize = 10;

// ── Minimal DOM ───────────────────────────────────────────────────────────
//
// `crate::test_dom::TestDoc` is `pub(crate)` and a benchmark compiles as a
// separate crate, so it is out of reach. Widening its visibility to serve a
// bench would grow raikiri-style's public surface for a test-only helper;
// duplicating a ~60-line mock here is the cheaper trade.

/// One node of [`BenchDoc`]: either the document root, a `div`, or a text
/// child. `tag` is empty for non-elements.
struct BenchNode {
    kind: StyleNodeKind,
    tag: &'static str,
    text: Option<&'static str>,
    children: Vec<usize>,
}

/// Flat arena implementing [`StyleDom`]: a document root whose children are
/// `n_elems` `div`s, each holding one text node.
///
/// The text nodes are not incidental — they are visited by the inheritance
/// walk and were present in the workload that produced the reference
/// numbers.
struct BenchDoc {
    nodes: Vec<BenchNode>,
}

impl BenchDoc {
    fn new(n_elems: usize) -> Self {
        let mut nodes = Vec::with_capacity(1 + n_elems * 2);
        nodes.push(BenchNode {
            kind: StyleNodeKind::Document,
            tag: "",
            text: None,
            children: Vec::with_capacity(n_elems),
        });
        for _ in 0..n_elems {
            let div = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "div",
                text: None,
                children: Vec::with_capacity(1),
            });
            let text = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Text,
                tag: "",
                text: Some("x"),
                children: Vec::new(),
            });
            nodes[div].children.push(text);
            nodes[0].children.push(div);
        }
        Self { nodes }
    }
}

/// Borrowed handle into [`BenchDoc`].
struct BenchNodeRef<'a> {
    doc: &'a BenchDoc,
    id: usize,
}

/// Borrowed handle to an element node.
struct BenchElementRef<'a> {
    node: &'a BenchNode,
}

/// Child-id iterator over a node's `children` vector.
struct BenchChildIter<'a>(std::slice::Iter<'a, usize>);

impl Iterator for BenchChildIter<'_> {
    type Item = StyleNodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(|i| StyleNodeId::new(i as u64))
    }
}

impl StyleDom for BenchDoc {
    type NodeRef<'a> = BenchNodeRef<'a>;
    type ChildIter<'a> = BenchChildIter<'a>;

    fn root_id(&self) -> StyleNodeId {
        StyleNodeId::new(0)
    }

    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        let idx = id.0 as usize;
        (idx < self.nodes.len()).then_some(BenchNodeRef { doc: self, id: idx })
    }

    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        let slice = self
            .nodes
            .get(id.0 as usize)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        BenchChildIter(slice.iter())
    }

    fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

impl StyleNode for BenchNodeRef<'_> {
    type Element<'b>
        = BenchElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> StyleNodeKind {
        self.doc.nodes[self.id].kind
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        matches!(self.kind(), StyleNodeKind::Element).then(|| BenchElementRef {
            node: &self.doc.nodes[self.id],
        })
    }

    fn text_content(&self) -> Option<&str> {
        self.doc.nodes[self.id].text
    }
}

impl StyleElement for BenchElementRef<'_> {
    fn tag_name(&self) -> &str {
        self.node.tag
    }
}

// ── Workload ──────────────────────────────────────────────────────────────

/// The computed values the **winning** (last) rule sets. Every generated rule
/// declares the same ten longhands with values that vary by rule index, and
/// equal-specificity / equal-origin / non-`!important` declarations resolve by
/// its Order of Appearance criterion — CSS Cascading L4 §6.1 "Cascade Sorting
/// Order" (<https://www.w3.org/TR/css-cascade-4/#cascade-sort>) — so the last
/// rule wins all ten.
struct Winners {
    font_size: f32,
    /// Shared by the four `margin-*` and four `padding-*` longhands.
    box_px: f32,
    color: CssColor,
}

/// Build a stylesheet of `n_rules` rules, each `div { … }` with
/// [`DECLS_PER_RULE`] longhand declarations.
///
/// Returns the CSS together with the [`Winners`] it should cascade to.
/// Returning them rather than recomputing at the call site keeps one source of
/// truth: the probe in [`workload`] compares against these exact values, and a
/// hand-copied formula would drift and then fail with a misleading message.
///
/// Values vary per rule so the last rule is distinguishable from the rest —
/// that is what lets the probe verify the *winning* rule's declarations landed,
/// rather than merely that some rule's did.
fn stylesheet(n_rules: usize) -> (String, Winners) {
    let mut css = String::new();
    let mut winners = Winners {
        font_size: 0.0,
        box_px: 0.0,
        color: CssColor::BLACK,
    };
    for i in 0..n_rules {
        let box_px = i as f32;
        let font_size = (i + 10) as f32;
        // Vary the red channel so `color` also identifies the winning rule.
        // The `% 256` wrap means a config with more than 256 rules could give
        // the winner the same colour as rule `n - 257`; `font-size` and the box
        // longhands would still pin it, and both current configs are far below
        // that, but a future large-`n_rules` config should not rely on colour
        // alone.
        let red = (i % 256) as u8;
        css.push_str(&format!(
            "div {{ margin-top: {box_px}px; margin-right: {box_px}px; \
             margin-bottom: {box_px}px; margin-left: {box_px}px; \
             padding-top: {box_px}px; padding-right: {box_px}px; \
             padding-bottom: {box_px}px; padding-left: {box_px}px; \
             font-size: {font_size}px; color: rgb({red},2,3) }}\n"
        ));
        winners = Winners {
            font_size,
            box_px,
            color: CssColor {
                r: red,
                g: 2,
                b: 3,
                a: 255,
            },
        };
    }
    (css, winners)
}

/// Assemble one config and prove the workload is the one it claims to be.
///
/// The assertions are the load-bearing part. A benchmark that measures
/// nothing is worse than no benchmark, because it reports green — and both
/// failure directions are silent:
///
/// - **Input side.** If a parser change dropped a declaration or a whole
///   rule, the loop would simply run fewer times and the benchmark would
///   report a speedup.
/// - **Matching side.** This is the subtler one. `match_by_tag`'s selector
///   walk ends in `_ => { matches = false; break; }`
///   (`crates/raikiri-style/src/cascade.rs`), so if a `selectors` upgrade
///   ever changes how a bare type selector decomposes into components, *every
///   rule stops matching every element*. The parse-side assertions below
///   would still pass, `collect_cascaded`'s inner loop would never execute,
///   and this file would report a large improvement while measuring nothing.
///
/// So the input assertions are not enough on their own: the function also
/// runs one throwaway `cascade()` and checks that *every* declaration reached
/// *every* element's computed values, and that both inherited ones reached
/// every text child. That probe is outside every timed region — it runs once
/// per config at setup.
fn workload(n_rules: usize, n_elems: usize) -> (BenchDoc, RuleTree, u64) {
    let mut tree = RuleTree::empty();
    let (css, want) = stylesheet(n_rules);
    tree.add_stylesheet(&css, Origin::Author);

    assert_eq!(
        tree.style_rules.len(),
        n_rules,
        "stylesheet did not parse into the expected rule count"
    );
    for rule in &tree.style_rules {
        assert_eq!(
            rule.declarations.len(),
            DECLS_PER_RULE,
            "rule did not parse into the expected declaration count"
        );
    }

    let doc = BenchDoc::new(n_elems);
    let initial = ComputedValues::initial();

    // Each expected winner must differ from the corresponding initial value,
    // or the check below could not tell a working cascade from a dead one.
    // `n_rules == 1` would trip this: the sole rule sets the box longhands to
    // 0px, which *is* the initial.
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "config {n_rules}×{n_elems} cascades to a value indistinguishable from \
         the initial one, so the probe below would be vacuous — pick a \
         different rule count"
    );

    let probe = cascade(&doc, &tree).expect("cascade is infallible in M1");
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    // Sweep the **whole** arena, checking **all ten** declarations against the
    // exact values the winning rule sets.
    //
    // Node 0 is excluded: `BenchDoc::new` puts the document there, and a
    // document node has no declarations and correctly keeps initial values.
    //
    // Three things are deliberately not weakened here:
    //
    // - **Whole arena, not sampled endpoints.** `probe.computed.len() ==
    //   doc.node_count()` above is near-tautological, since `cascade()`
    //   pre-allocates that Vec to `node_count()`. A change dropping every other
    //   element, or every text child but the first, would satisfy any fixed set
    //   of sampled indices while cutting real work far below the throughput
    //   denominator — reporting a large speedup for doing less.
    // - **All ten declarations, not just `font-size`.** Otherwise a change that
    //   processed only the one property checked would pass.
    // - **Exact winning values, not merely "differs from initial".** The
    //   inequality form would accept a change that stopped at some earlier
    //   rule, since earlier rules also write non-initial values. Comparing
    //   against the last rule's values is what pins that the cascade ran to
    //   completion for each property.
    //
    // The sweep is O(node), once per config at setup, against a measurement
    // that runs for a minute.
    let want_margin = ComputedLengthPercentageOrAuto::Px(want.box_px);
    let want_padding = ComputedLengthPercentage::Px(want.box_px);
    for (idx, cv) in probe.computed.iter().enumerate().skip(1) {
        // Ask the arena for the kind rather than deriving it from index
        // parity: node id is the arena index throughout, so if
        // `BenchDoc::new`'s interleaving ever changes, this still reports the
        // right kind instead of silently mislabelling.
        let is_element = matches!(doc.nodes[idx].kind, StyleNodeKind::Element);
        let what = if is_element { "element" } else { "text node" };
        // `font-size` and `color` are inherited, so they must have reached the
        // text children too.
        assert_eq!(
            cv.font_size.px(),
            want.font_size,
            "{what} at node {idx} of {} does not carry the winning rule's \
             font-size — a partial walk leaves the throughput denominator \
             overstating the work actually done",
            doc.node_count(),
        );
        assert_eq!(
            cv.color, want.color,
            "{what} at node {idx} does not carry the winning rule's color"
        );
        if !is_element {
            // Text node: `margin` / `padding` are not inherited.
            continue;
        }
        for (name, got) in [
            ("margin-top", cv.margin.top),
            ("margin-right", cv.margin.right),
            ("margin-bottom", cv.margin.bottom),
            ("margin-left", cv.margin.left),
        ] {
            assert_eq!(
                got, want_margin,
                "element at node {idx}: {name} does not carry the winning \
                 rule's value"
            );
        }
        for (name, got) in [
            ("padding-top", cv.padding.top),
            ("padding-right", cv.padding.right),
            ("padding-bottom", cv.padding.bottom),
            ("padding-left", cv.padding.left),
        ] {
            assert_eq!(
                got, want_padding,
                "element at node {idx}: {name} does not carry the winning \
                 rule's value"
            );
        }
    }

    let declarations = (n_rules * n_elems * DECLS_PER_RULE) as u64;
    (doc, tree, declarations)
}

fn bench_cascade(c: &mut Criterion) {
    let mut group = c.benchmark_group("cascade");

    // `rule_heavy` stresses the rules-per-element factor, `element_heavy` the
    // node count. Healthy absolute cost is ≈20 and ≈33 ns/declaration
    // respectively at the `832500e` baseline (they are not comparable — see
    // the throughput note below).
    // The ≈4 ns/declaration in the module doc is the regression *delta*, not
    // the healthy cost.
    for (name, n_rules, n_elems) in [
        ("rule_heavy_50x500", 50usize, 500usize),
        ("element_heavy_5x2000", 5usize, 2000usize),
    ] {
        let (doc, tree, declarations) = workload(n_rules, n_elems);

        // Denominate in declarations rather than calls: the guarded class
        // moves a per-declaration constant, so a delta expressed this way is
        // the quantity under study and is comparable across configs.
        //
        // Two caveats. criterion prints this as a *rate* (`Melem/s`), so a
        // per-declaration time has to be inverted out of it. And the absolute
        // figure is **not** comparable between the two configs — element_heavy
        // carries 4× the nodes over 0.4× the declarations, which is why it
        // reads ≈33 ns against rule_heavy's ≈20 ns on identical code (both
        // at the `832500e` baseline). Only the *delta* is comparable.
        group.throughput(Throughput::Elements(declarations));
        group.bench_function(name, |b| {
            b.iter_with_large_drop(|| cascade(&doc, &tree).expect("cascade is infallible in M1"));
        });
    }

    group.finish();
}

// criterion's `criterion_main!`/`criterion_group!` pair is inlined here: the
// macro generates a `pub fn`, which trips the workspace's
// `missing_docs = "warn"` (an error under gate §8.1's `-D warnings`), and the
// `allow` cannot be attached to a macro invocation. This is the macro body,
// minus its second redundant `configure_from_args()`.
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_cascade(&mut criterion);
    criterion.final_summary();
}
