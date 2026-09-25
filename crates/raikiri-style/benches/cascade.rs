//! Benchmarks the cascade and selector-matching hot paths.
//!
//! Rule-tree construction stays outside the timed closure. The workload
//! probes check the resulting values so the benchmark does not silently time
//! an empty or partial cascade.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p raikiri-style --bench cascade
//! ```

use criterion::{Criterion, Throughput};
use raikiri_style::{
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedValues, CssColor, Origin,
    RuleTree, StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, cascade,
};

/// Declarations emitted per generated rule in the benchmark workload.
const DECLS_PER_RULE: usize = 10;

/// Shared message for the benchmark's infallible cascade calls.
// cov:ignore: bench harness constant — `cargo test`/`cargo llvm-cov
// --workspace` never build this bench target, so nothing in this file has
// coverage instrumentation to attribute to.
const CASCADE_NEVER_ERRS: &str = "cascade never returns Err in the current implementation";

/// Compound count of the selector used by the combinator-chain workload.
// cov:ignore: bench harness constant — `cargo test`/`cargo llvm-cov
// --workspace` never build this bench target, so nothing in this file has
// coverage instrumentation to attribute to.
const COMBINATOR_CHAIN_SELECTOR_DEPTH: usize = 5;

/// Depth of the `div` chain used by the combinator-chain workload.
// cov:ignore: same reason as the constant above.
const COMBINATOR_CHAIN_DOC_DEPTH: usize = 5000;

/// Number of `article` levels in the mixed-combinator workload.
// cov:ignore: same reason as the constant above.
const MIXED_CHAIN_ARTICLES: usize = 300;

// ── Minimal DOM ───────────────────────────────────────────────────────────
//
// Keep the benchmark's small DOM local so the test-only helper is not part
// of the crate's public surface.

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
/// Each element has one text child so the inheritance path is exercised.
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

    /// Build a straight `depth`-deep parent-child chain of `div`s under the
    /// document root — `div` → `div` → … → `div`, one child per level, no
    /// siblings and no text nodes.
    ///
    /// Unlike [`BenchDoc::new`]'s flat topology (every `div` a direct child
    /// of the document root, so none has a `div` ancestor at all), every
    /// `div` here has every shallower `div` in the chain as an ancestor —
    /// this is what lets [`combinator_chain_workload`] exercise
    /// `match_combinator_chain`'s ancestor walk.
    // cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
    // never build this bench target, so nothing in this function has
    // coverage instrumentation to attribute to.
    fn chain(depth: usize) -> Self {
        let mut nodes = Vec::with_capacity(1 + depth);
        nodes.push(BenchNode {
            kind: StyleNodeKind::Document,
            tag: "",
            text: None,
            children: Vec::with_capacity(1),
        });
        let mut parent = 0usize;
        for _ in 0..depth {
            let div = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "div",
                text: None,
                children: Vec::new(),
            });
            nodes[parent].children.push(div);
            parent = div;
        }
        Self { nodes }
    }

    /// Build a mixed child+descendant selector-matching topology: a
    /// `section` with a straight `n_articles`-deep parent-child chain of
    /// `article`s below it, each carrying its own `div` child — plus an
    /// unrelated `article` → `article` → `div` chain with no `section`
    /// ancestor anywhere, as a negative control.
    ///
    /// Matched against `section > article div`
    /// ([`mixed_combinator_workload`]'s selector), this is what a pure
    /// child-combinator chain ([`BenchDoc::chain`]) cannot exercise: the
    /// `Combinator::Descendant` candidate search past a failed
    /// `Combinator::Child` candidate. The `div` under `article` level `k`
    /// (1-indexed, closest to `section` = 1) only matches once the
    /// `Descendant` search reaches level-1's `article` — every closer
    /// `article` candidate matches the `Descendant` step but then fails the
    /// `Child` step (its own immediate parent is another `article`, not
    /// `section`), forcing a push-then-pop retry — so level `k`'s `div`
    /// costs `k` such choice-point cycles, not 1.
    ///
    /// Returns `(doc, positive_div_ids, negative_div_id)`: `positive_div_ids`
    /// is one `div` id per `article` level (all of which must match),
    /// `negative_div_id` is the one `div` with no `section` ancestor at all
    /// (which must not).
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    fn mixed_chain(n_articles: usize) -> (Self, Vec<usize>, usize) {
        let mut nodes = Vec::with_capacity(1 + n_articles * 2 + 3);
        nodes.push(BenchNode {
            kind: StyleNodeKind::Document,
            tag: "",
            text: None,
            children: Vec::with_capacity(2),
        });

        let section = nodes.len();
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "section",
            text: None,
            children: Vec::with_capacity(1),
        });
        nodes[0].children.push(section);

        let mut positive_div_ids = Vec::with_capacity(n_articles);
        let mut parent = section;
        for _ in 0..n_articles {
            let article = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "article",
                text: None,
                children: Vec::with_capacity(1),
            });
            nodes[parent].children.push(article);

            let div = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "div",
                text: None,
                children: Vec::new(),
            });
            nodes[article].children.push(div);
            positive_div_ids.push(div);

            parent = article;
        }

        // Negative control: `article` → `article` → `div`, no `section`
        // ancestor anywhere.
        let neg_article_1 = nodes.len();
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "article",
            text: None,
            children: Vec::with_capacity(1),
        });
        nodes[0].children.push(neg_article_1);
        let neg_article_2 = nodes.len();
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "article",
            text: None,
            children: Vec::with_capacity(1),
        });
        nodes[neg_article_1].children.push(neg_article_2);
        let negative_div_id = nodes.len();
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "div",
            text: None,
            children: Vec::new(),
        });
        nodes[neg_article_2].children.push(negative_div_id);

        (Self { nodes }, positive_div_ids, negative_div_id)
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
        // longhands would still check it, and both current configs are far below
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

/// Build a single-rule stylesheet whose selector is a `depth`-compound
/// child-combinator chain (`div > div > … > div`, `depth - 1` `>`
/// combinators), with [`DECLS_PER_RULE`] longhand declarations.
///
/// Returns the CSS together with the [`Winners`] it cascades to, on the same
/// one-source-of-truth contract as [`stylesheet`] — the probe in
/// [`combinator_chain_workload`] compares against these exact values.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn chain_stylesheet(depth: usize) -> (String, Winners) {
    let box_px = 7.0;
    let font_size = 42.0;
    let color = CssColor {
        r: 9,
        g: 8,
        b: 7,
        a: 255,
    };
    let selector = vec!["div"; depth].join(" > ");
    let css = format!(
        "{selector} {{ margin-top: {box_px}px; margin-right: {box_px}px; \
         margin-bottom: {box_px}px; margin-left: {box_px}px; \
         padding-top: {box_px}px; padding-right: {box_px}px; \
         padding-bottom: {box_px}px; padding-left: {box_px}px; \
         font-size: {font_size}px; color: rgb({}, {}, {}) }}\n",
        color.r, color.g, color.b,
    );
    (
        css,
        Winners {
            font_size,
            box_px,
            color,
        },
    )
}

/// Build a single-rule stylesheet for the `section > article div` mixed
/// child+descendant selector ([`BenchDoc::mixed_chain`]'s topology), with
/// [`DECLS_PER_RULE`] longhand declarations.
///
/// Same one-source-of-truth contract as [`stylesheet`]/[`chain_stylesheet`].
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn mixed_stylesheet() -> (String, Winners) {
    let box_px = 3.0;
    let font_size = 21.0;
    let color = CssColor {
        r: 1,
        g: 2,
        b: 3,
        a: 255,
    };
    let css = format!(
        "section > article div {{ margin-top: {box_px}px; margin-right: {box_px}px; \
         margin-bottom: {box_px}px; margin-left: {box_px}px; \
         padding-top: {box_px}px; padding-right: {box_px}px; \
         padding-bottom: {box_px}px; padding-left: {box_px}px; \
         font-size: {font_size}px; color: rgb({}, {}, {}) }}\n",
        color.r, color.g, color.b,
    );
    (
        css,
        Winners {
            font_size,
            box_px,
            color,
        },
    )
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
/// - **Matching side.** This is the subtler one. `compound_matches`'s
///   per-component walk ends in a `_ => false` safety-net arm
///   (`crates/raikiri-style/src/cascade.rs`) that fails the whole compound on
///   any unrecognized component, so if a `selectors` upgrade ever changes how
///   a bare type selector decomposes into components, *every rule stops
///   matching every element*. The parse-side assertions below would still
///   pass, `collect_cascaded`'s inner loop would never execute, and this file
///   would report a large improvement while measuring nothing.
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

    // Accessors, not fields: a bench is a separate compilation unit, so the
    // `pub(crate)` fields do not resolve here.
    assert_eq!(
        tree.style_rules().len(),
        n_rules,
        "stylesheet did not parse into the expected rule count"
    );
    for rule in tree.style_rules() {
        assert_eq!(
            rule.declarations().len(),
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

    // cov:ignore: bench harness — never instrumented under `cargo llvm-cov
    // --workspace` (bench targets are not built by `cargo test`/`cargo
    // llvm-cov --workspace`). The assertions in this function are the actual
    // correctness check, run once at setup outside any timed region.
    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
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

/// Assemble the combinator-chain config and prove it exercises what it
/// claims to — see the module doc's "Combinator-chain workload" section for
/// the design.
///
/// `selector_depth` compounds (`selector_depth - 1` child combinators) are
/// matched against a straight `doc_depth`-deep parent-child chain of `div`s
/// ([`BenchDoc::chain`]). `doc.nodes` index `i` (`1..=doc_depth`) is the
/// `div` at chain position `i` (1-indexed), with `i - 1` `div` ancestors
/// above it — exactly the ancestor count the selector's `selector_depth - 1`
/// child combinators need. So `i >= selector_depth` matches the whole
/// selector, and `i < selector_depth` runs out of ancestors first and does
/// not.
///
/// Unlike [`workload`], not every element matches: the probe below checks
/// both directions; on [`workload`]'s "the assertions are the load-bearing
/// part" logic, proving the negative (shallow elements keep their initial
/// values) is what catches a matcher that ignored ancestor structure and
/// returned true unconditionally.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above. The
// assertions in this function are the actual correctness check (run once at
// setup, outside any timed region, and would panic on failure) — coverage
// instrumentation is what's structurally unavailable here, not testing.
fn combinator_chain_workload(selector_depth: usize, doc_depth: usize) -> (BenchDoc, RuleTree, u64) {
    assert!(
        selector_depth >= 1 && selector_depth <= doc_depth,
        "selector_depth must be in [1, doc_depth] for the probe below to see \
         both a matching and a non-matching element"
    );

    let mut tree = RuleTree::empty();
    let (css, want) = chain_stylesheet(selector_depth);
    tree.add_stylesheet(&css, Origin::Author);

    assert_eq!(
        tree.style_rules().len(),
        1,
        "combinator-chain stylesheet did not parse into exactly one rule"
    );
    assert_eq!(
        tree.style_rules()[0].declarations().len(),
        DECLS_PER_RULE,
        "combinator-chain rule did not parse into the expected declaration count"
    );

    let doc = BenchDoc::chain(doc_depth);
    let initial = ComputedValues::initial();
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "combinator-chain winner values must differ from the initial ones, \
         or the probe below would be vacuous"
    );

    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    let want_margin = ComputedLengthPercentageOrAuto::Px(want.box_px);
    let want_padding = ComputedLengthPercentage::Px(want.box_px);

    for i in 1..=doc_depth {
        let cv = &probe.computed[i];
        if i >= selector_depth {
            assert_eq!(
                cv.font_size.px(),
                want.font_size,
                "div at chain position {i} of {doc_depth} has {} div ancestors \
                 ({selector_depth} needed) and should match the selector, but \
                 does not carry the winning font-size",
                i - 1,
            );
            assert_eq!(
                cv.color, want.color,
                "div at chain position {i} of {doc_depth} should match the \
                 selector but does not carry the winning color"
            );
            assert_eq!(
                cv.margin.top, want_margin,
                "div at chain position {i} of {doc_depth}: margin-top does \
                 not carry the winning rule's value"
            );
            assert_eq!(
                cv.padding.top, want_padding,
                "div at chain position {i} of {doc_depth}: padding-top does \
                 not carry the winning rule's value"
            );
        } else {
            assert_eq!(
                cv.font_size.px(),
                initial.font_size.px(),
                "div at chain position {i} of {doc_depth} has only {} div \
                 ancestors ({selector_depth} needed) and must NOT match — a \
                 matcher that ignores ancestor structure would wrongly style \
                 it",
                i - 1,
            );
            assert_eq!(
                cv.color, initial.color,
                "div at chain position {i} of {doc_depth} has insufficient \
                 div ancestors and must keep the initial color"
            );
        }
    }

    (doc, tree, doc_depth as u64)
}

/// Assemble the mixed child+descendant config and prove it exercises what it
/// claims to — see [`BenchDoc::mixed_chain`]'s doc for the topology and why
/// it forces `match_combinator_chain`'s `Combinator::Descendant` retry-past-
/// a-failed-`Combinator::Child`-candidate path, which neither [`workload`]
/// nor [`combinator_chain_workload`] reaches (the former never invokes
/// `match_combinator_chain` at all; the latter's pure child-combinator chain
/// never backtracks — `PendingCandidates::Child` has exactly one
/// candidate).
///
/// On [`workload`]'s "the assertions are the load-bearing part" logic: every
/// `article` level's `div` must carry the winning rule's values (the
/// `Descendant` retry must eventually succeed), and the negative-control
/// `div` (no `section` ancestor) must not (the retry must not
/// over-match once a `Child` candidate fails).
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above. The
// assertions in this function are the actual correctness check (run once at
// setup, outside any timed region, and would panic on failure) — coverage
// instrumentation is what's structurally unavailable here, not testing.
fn mixed_combinator_workload(n_articles: usize) -> (BenchDoc, RuleTree, u64) {
    let mut tree = RuleTree::empty();
    let (css, want) = mixed_stylesheet();
    tree.add_stylesheet(&css, Origin::Author);

    assert_eq!(
        tree.style_rules().len(),
        1,
        "mixed-combinator stylesheet did not parse into exactly one rule"
    );
    assert_eq!(
        tree.style_rules()[0].declarations().len(),
        DECLS_PER_RULE,
        "mixed-combinator rule did not parse into the expected declaration count"
    );

    let (doc, positive_div_ids, negative_div_id) = BenchDoc::mixed_chain(n_articles);
    let initial = ComputedValues::initial();
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "mixed-combinator winner values must differ from the initial ones, \
         or the probe below would be vacuous"
    );

    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    let want_margin = ComputedLengthPercentageOrAuto::Px(want.box_px);
    let want_padding = ComputedLengthPercentage::Px(want.box_px);

    for (level, &div_id) in positive_div_ids.iter().enumerate() {
        let cv = &probe.computed[div_id];
        assert_eq!(
            cv.font_size.px(),
            want.font_size,
            "div under article level {} of {n_articles} should match \
             `section > article div` but does not carry the winning \
             font-size — the Descendant retry past a failed Child candidate \
             did not run to completion",
            level + 1,
        );
        assert_eq!(
            cv.color,
            want.color,
            "div under article level {} of {n_articles} should match but \
             does not carry the winning color",
            level + 1,
        );
        assert_eq!(
            cv.margin.top,
            want_margin,
            "div under article level {} of {n_articles}: margin-top does \
             not carry the winning rule's value",
            level + 1,
        );
        assert_eq!(
            cv.padding.top,
            want_padding,
            "div under article level {} of {n_articles}: padding-top does \
             not carry the winning rule's value",
            level + 1,
        );
    }

    let negative_cv = &probe.computed[negative_div_id];
    assert_eq!(
        negative_cv.font_size.px(),
        initial.font_size.px(),
        "the negative-control div (no `section` ancestor) must not match \
         `section > article div` — a matcher that over-matched once a \
         Child candidate failed would wrongly style it"
    );
    assert_eq!(
        negative_cv.color, initial.color,
        "the negative-control div must keep the initial color"
    );

    // Throughput unit: one top-level `match_combinator_chain` call per `div`
    // in the tree (both the `n_articles` positive ones and the one negative
    // control) — see the module doc.
    let match_attempts = (positive_div_ids.len() + 1) as u64;
    (doc, tree, match_attempts)
}

fn bench_cascade(c: &mut Criterion) {
    let mut group = c.benchmark_group("cascade");

    // Exercise both rule-heavy and element-heavy workloads.
    // cov:ignore: bench harness — never instrumented under `cargo llvm-cov
    // --workspace` (bench targets are not built by `cargo test`/`cargo
    // llvm-cov --workspace`), so nothing in this loop has coverage
    // instrumentation to attribute to.
    for (name, n_rules, n_elems) in [
        ("rule_heavy_50x500", 50usize, 500usize),
        ("element_heavy_5x2000", 5usize, 2000usize),
    ] {
        let (doc, tree, declarations) = workload(n_rules, n_elems);

        // Report throughput in declarations so the workloads share a unit.
        group.throughput(Throughput::Elements(declarations));
        group.bench_function(name, |b| {
            b.iter_with_large_drop(|| cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Keep inactive rules in the tree to measure media filtering separately
    // from selector matching. Also exercise a fully active media workload.
    for (name, n_rules, n_elems, query, active) in [
        ("media_inactive_2000x1000", 2000, 1000, "screen", false),
        ("media_active_50x500", 50, 500, "print", true),
    ] {
        let doc = BenchDoc::new(n_elems);
        let (css, want) = stylesheet(n_rules);
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&format!("@media {query} {{ {css} }}"), Origin::Author);
        let initial = ComputedValues::initial();
        let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
        assert_eq!(probe.computed.len(), doc.node_count());
        for (idx, node) in doc.nodes.iter().enumerate() {
            if node.kind != StyleNodeKind::Element {
                continue;
            }
            let cv = &probe.computed[idx];
            assert_eq!(cv.color, if active { want.color } else { initial.color });
            assert_eq!(
                cv.font_size.px(),
                if active {
                    want.font_size
                } else {
                    initial.font_size.px()
                }
            );
        }
        // Invert the context on the same tree to prove that the inactive
        // case contains valid rules, and that filtering is per invocation.
        let screen = raikiri_style::cascade_with_media_context(
            &BenchDoc::new(1),
            &tree,
            &raikiri_style::MediaContext::screen(),
        )
        .expect(CASCADE_NEVER_ERRS);
        assert_eq!(
            screen.computed[1].color,
            if active { initial.color } else { want.color }
        );
        assert_eq!(
            screen.computed[1].font_size.px(),
            if active {
                initial.font_size.px()
            } else {
                want.font_size
            }
        );

        group.throughput(Throughput::Elements(n_elems as u64));
        group.bench_function(name, |b| {
            b.iter_with_large_drop(|| cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Combinator-chain config — see the module doc's "Combinator-chain
    // workload" section for why the two configs above cannot exercise
    // `match_combinator_chain` at all.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    {
        let (doc, tree, match_attempts) =
            combinator_chain_workload(COMBINATOR_CHAIN_SELECTOR_DEPTH, COMBINATOR_CHAIN_DOC_DEPTH);

        // Throughput denominated in elements attempted, not declarations:
        // every `div` in the chain triggers exactly one top-level call into
        // `match_combinator_chain` (see the module doc), which is the
        // quantity whose per-call allocation cost is under study here —
        // unlike the two configs above, most attempts do not go on to
        // produce any declarations at all.
        group.throughput(Throughput::Elements(match_attempts));
        group.bench_function("combinator_chain_5000x4", |b| {
            b.iter_with_large_drop(|| cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Mixed child+descendant config — see `mixed_combinator_workload`'s doc
    // for why `combinator_chain_5000x4` above, despite exercising
    // `match_combinator_chain`, still cannot reach its
    // `Combinator::Descendant` retry-past-a-failed-`Combinator::Child`-
    // candidate path (a pure child-combinator chain never backtracks).
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    {
        let (doc, tree, match_attempts) = mixed_combinator_workload(MIXED_CHAIN_ARTICLES);

        // Throughput denominated in elements attempted, same convention as
        // `combinator_chain_5000x4` above.
        group.throughput(Throughput::Elements(match_attempts));
        group.bench_function("mixed_combinator_chain_300", |b| {
            b.iter_with_large_drop(|| cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS));
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
