//! Benchmarks the cascade and selector-matching hot paths.
//!
//! Rule-tree construction stays outside the timed closure. The workload
//! probes check the resulting values so the benchmark does not silently time
//! an empty or partial cascade. Each workload is built the first time its
//! benchmark runs, so a name filter skips the setup of every other one.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p raikiri-style --bench cascade
//! ```

use std::cell::OnceCell;

use criterion::{Criterion, Throughput};
use raikiri_style::{
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedValues, CssColor, Origin,
    RuleTree, SelectorQuery, StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind,
    cascade,
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

/// Depth of the `div` chain in the descendant-miss workload.
// cov:ignore: bench harness constant — same reason as `CASCADE_NEVER_ERRS`.
const DESCENDANT_MISS_DOC_DEPTH: usize = 1000;

/// Rule count of the descendant-miss workload.
// cov:ignore: bench harness constant — same reason as `CASCADE_NEVER_ERRS`.
const DESCENDANT_MISS_RULES: usize = 50;

/// Depth of the `div` chain in the `:first-child` chain workload.
// cov:ignore: bench harness constant — same reason as `CASCADE_NEVER_ERRS`.
const FIRST_CHILD_CHAIN_DEPTH: usize = 5000;

/// Number of `article` levels in the mixed-combinator workload.
// cov:ignore: same reason as the constant above.
const MIXED_CHAIN_ARTICLES: usize = 300;

// ── Minimal DOM ───────────────────────────────────────────────────────────
//
// Keep the benchmark's small DOM local so the test-only helper is not part
// of the crate's public surface.

/// One node of [`BenchDoc`]: either the document root, a `div`, or a text
/// child. `tag` is empty for non-elements.
// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
// never build this bench target, so nothing in this item has coverage
// instrumentation to attribute to.
struct BenchNode {
    kind: StyleNodeKind,
    tag: &'static str,
    text: Option<&'static str>,
    /// `id` attribute value. `None` unless a workload assigns one, so the
    /// pre-existing workloads keep measuring exactly what they measured before.
    id: Option<String>,
    /// `class` attribute tokens. Empty unless a workload assigns classes.
    classes: Vec<String>,
    /// `style` attribute source. `None` unless a workload assigns one.
    style: Option<String>,
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
    // cov:ignore: bench harness — same reason as `BenchNode` above.
    fn new(n_elems: usize) -> Self {
        let mut nodes = Vec::with_capacity(1 + n_elems * 2);
        nodes.push(BenchNode {
            kind: StyleNodeKind::Document,
            tag: "",
            text: None,
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::with_capacity(n_elems),
        });
        for _ in 0..n_elems {
            let div = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "div",
                text: None,
                id: None,
                classes: Vec::new(),
                style: None,
                children: Vec::with_capacity(1),
            });
            let text = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Text,
                tag: "",
                text: Some("x"),
                id: None,
                classes: Vec::new(),
                style: None,
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
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::with_capacity(1),
        });
        let mut parent = 0usize;
        for _ in 0..depth {
            let div = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "div",
                text: None,
                id: None,
                classes: Vec::new(),
                style: None,
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
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::with_capacity(2),
        });

        let section = nodes.len();
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "section",
            text: None,
            id: None,
            classes: Vec::new(),
            style: None,
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
                id: None,
                classes: Vec::new(),
                style: None,
                children: Vec::with_capacity(1),
            });
            nodes[parent].children.push(article);

            let div = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "div",
                text: None,
                id: None,
                classes: Vec::new(),
                style: None,
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
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::with_capacity(1),
        });
        nodes[0].children.push(neg_article_1);
        let neg_article_2 = nodes.len();
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "article",
            text: None,
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::with_capacity(1),
        });
        nodes[neg_article_1].children.push(neg_article_2);
        let negative_div_id = nodes.len();
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "div",
            text: None,
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::new(),
        });
        nodes[neg_article_2].children.push(negative_div_id);

        (Self { nodes }, positive_div_ids, negative_div_id)
    }

    /// Build a wide sibling topology: a `section` under the document root
    /// holding `n_elems` `div`s side by side, each with one text child —
    /// the shape whose per-element sibling-position scans a
    /// sibling-position cache would attack. Positions below are 1-indexed
    /// among the `section`'s element children: the `div` at
    /// `nodes[1].children[k]` is position `k + 1`.
    // cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
    // never build this bench target, so nothing in this function has
    // coverage instrumentation to attribute to.
    fn wide(n_elems: usize) -> Self {
        let mut nodes = Vec::with_capacity(2 + n_elems * 2);
        nodes.push(BenchNode {
            kind: StyleNodeKind::Document,
            tag: "",
            text: None,
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::with_capacity(1),
        });
        nodes.push(BenchNode {
            kind: StyleNodeKind::Element,
            tag: "section",
            text: None,
            id: None,
            classes: Vec::new(),
            style: None,
            children: Vec::with_capacity(n_elems),
        });
        nodes[0].children.push(1);
        for _ in 0..n_elems {
            let div = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Element,
                tag: "div",
                text: None,
                id: None,
                classes: Vec::new(),
                style: None,
                children: Vec::with_capacity(1),
            });
            let text = nodes.len();
            nodes.push(BenchNode {
                kind: StyleNodeKind::Text,
                tag: "",
                text: Some("x"),
                id: None,
                classes: Vec::new(),
                style: None,
                children: Vec::new(),
            });
            nodes[div].children.push(text);
            nodes[1].children.push(div);
        }
        Self { nodes }
    }

    /// Give the `div`s of a [`BenchDoc::wide`] document distinct sparse
    /// classes: the `div` at sibling position `k + 1` (0-indexed `k`) gets
    /// class `s{k * stride}`, unless it is an unmatched control (every 5th,
    /// `k % 5 == 4`), which stays classless so the probe can check the
    /// must-not-match direction too.
    // cov:ignore: bench harness — same reason as `BenchDoc::wide` above.
    fn assign_sparse_classes(&mut self, stride: usize) {
        let divs = self.nodes[1].children.clone();
        for (k, div) in divs.iter().enumerate() {
            if k % 5 == 4 {
                continue;
            }
            self.nodes[*div].classes.push(format!("s{}", k * stride));
        }
    }

    /// Give the `div`s of a [`BenchDoc::wide`] document distinct sparse ids:
    /// the `div` at sibling position `k + 1` gets id `nid{k * stride}`,
    /// with the same every-5th unmatched controls as
    /// [`BenchDoc::assign_sparse_classes`].
    // cov:ignore: bench harness — same reason as `BenchDoc::wide` above.
    fn assign_sparse_ids(&mut self, stride: usize) {
        let divs = self.nodes[1].children.clone();
        for (k, div) in divs.iter().enumerate() {
            if k % 5 == 4 {
                continue;
            }
            self.nodes[*div].id = Some(format!("nid{}", k * stride));
        }
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

    fn parent_id(&self, child: StyleNodeId) -> Option<StyleNodeId> {
        let idx = child.0 as usize;
        self.nodes.iter().enumerate().find_map(|(i, n)| {
            n.children
                .contains(&idx)
                .then_some(StyleNodeId::new(i as u64))
        })
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

// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
// never build this bench target, so nothing in this impl block has coverage
// instrumentation to attribute to.
impl StyleElement for BenchElementRef<'_> {
    fn tag_name(&self) -> &str {
        self.node.tag
    }

    /// Bench-local `id` lookup. Empty values read as absent, mirroring the
    /// real DOM's empty-is-absent contract; nodes without an assigned id
    /// (every pre-existing workload) read as `None`.
    fn id(&self) -> Option<&str> {
        self.node.id.as_deref().filter(|s| !s.is_empty())
    }

    /// Bench-local class lookup over the assigned tokens. ASCII-whitespace
    /// splitting is unnecessary here: workloads push one token per entry,
    /// so an exact per-token comparison is the same set of matches.
    fn has_class(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        self.node.classes.iter().any(|c| c == class)
    }

    /// Bench-local case-insensitive class lookup, same token set as
    /// [`BenchElementRef::has_class`] — the bench DOM only runs in
    /// standards mode, but an explicit override keeps both consistent if a
    /// future workload ever flips the quirks context.
    fn has_class_ascii_case_insensitive(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        self.node
            .classes
            .iter()
            .any(|c| c.eq_ignore_ascii_case(class))
    }

    /// Bench-local class enumeration over the same tokens as
    /// [`BenchElementRef::has_class`].
    fn for_each_class(&self, f: &mut dyn FnMut(&str)) {
        self.node
            .classes
            .iter()
            .filter(|c| !c.is_empty())
            .for_each(|c| f(c));
    }

    fn inline_style_source(&self) -> Option<&str> {
        self.node.style.as_deref().filter(|s| !s.is_empty())
    }

    /// Bench-local attribute lookup. Only `id` is backed: valued class and
    /// attribute selectors are not part of any bench workload, and class
    /// matching goes through [`BenchElementRef::has_class`].
    fn attr(&self, local: &str) -> Option<&str> {
        match local {
            "id" => self.id(),
            "style" => self.inline_style_source(),
            _ => None,
        }
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

/// Build the descendant-miss workload: a `doc_depth`-deep `div` chain
/// against `n_rules` rules of the form `.absentN div`. Every rule's subject
/// compound matches every element, but no element carries any `absentN`
/// class, so every attempt is a miss that a matcher without an ancestor
/// filter only discovers after walking the element's entire ancestor chain.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn descendant_miss_workload(n_rules: usize, doc_depth: usize) -> (BenchDoc, RuleTree, u64) {
    let mut tree = RuleTree::empty();
    let body = shared_winner_body();
    let css = (0..n_rules)
        .map(|i| format!(".absent{i} div {{ {body} }}\n"))
        .collect::<String>();
    tree.add_stylesheet(&css, Origin::Author);
    assert_eq!(
        tree.style_rules().len(),
        n_rules,
        "descendant-miss stylesheet did not parse into the expected rule count"
    );

    let doc = BenchDoc::chain(doc_depth);
    let initial = ComputedValues::initial();
    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    for i in 1..=doc_depth {
        assert_eq!(
            probe.computed[i].color, initial.color,
            "div at chain position {i} has no `absentN` ancestor and must not \
             match any rule"
        );
    }
    (doc, tree, (n_rules * doc_depth) as u64)
}

/// Build an inline-style workload: [`BenchDoc::wide`]'s `n_elems` sibling
/// `div`s, each with a `style` attribute and no stylesheet at all. With
/// `repeated`, every element carries the same source (the shape of
/// template-generated markup); otherwise every source differs, which is the
/// worst case for a per-source parse cache.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn inline_style_workload(n_elems: usize, repeated: bool) -> (BenchDoc, RuleTree, u64) {
    let tree = RuleTree::empty();
    let mut doc = BenchDoc::wide(n_elems);
    let divs = doc.nodes[1].children.clone();
    for (k, div) in divs.iter().enumerate() {
        let px = if repeated { 7 } else { k % 997 + 1 };
        doc.nodes[*div].style = Some(format!(
            "margin: {px}px; padding: 3px 4px; font-size: 42px; \
             color: rgb(9, 8, 7); border: 1px solid rgb(1, 2, {})",
            if repeated { 3 } else { k % 256 }
        ));
    }
    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    for (k, div) in divs.iter().enumerate() {
        let px = if repeated { 7.0 } else { (k % 997 + 1) as f32 };
        assert_eq!(
            probe.computed[*div].margin.top,
            ComputedLengthPercentageOrAuto::Px(px),
            "div {k} lost its inline margin"
        );
        assert_eq!(probe.computed[*div].font_size.px(), 42.0);
    }
    (doc, tree, n_elems as u64)
}

/// Build the `:has()` workload: a `doc_depth`-deep `div` chain against one
/// `div:has(> div) { ... }` rule. Every `div` evaluates the relative
/// selector once, and every one but the deepest matches, so the timing is
/// dominated by setting up one `:has()` search per element.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn has_child_chain_workload(doc_depth: usize) -> (BenchDoc, RuleTree, u64) {
    let mut tree = RuleTree::empty();
    let body = shared_winner_body();
    tree.add_stylesheet(&format!("div:has(> div) {{ {body} }}\n"), Origin::Author);
    assert_eq!(
        tree.style_rules().len(),
        1,
        ":has() stylesheet did not parse into exactly one rule"
    );
    let doc = BenchDoc::chain(doc_depth);
    let want = shared_winners();
    let initial = ComputedValues::initial();
    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    // `margin-top` is not inherited, so it tells a matching `div` apart from
    // the deepest one, which only inherits the matched ancestors' color.
    for i in 1..=doc_depth {
        let expected = if i < doc_depth {
            ComputedLengthPercentageOrAuto::Px(want.box_px)
        } else {
            initial.margin.top
        };
        assert_eq!(
            probe.computed[i].margin.top, expected,
            "div at chain position {i} of {doc_depth} has the wrong :has() result"
        );
    }
    (doc, tree, doc_depth as u64)
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

/// The declaration values shared by every sparse, nth-child, and adjacent
/// rule below. Identical across rules on purpose: each element matches at
/// most one of these rules, so there is no order contest to resolve and one
/// [`Winners`] probes every match. All three values differ from the initial
/// ones (same vacuous-probe guard as [`workload`]).
// cov:ignore: bench harness constant — `cargo test`/`cargo llvm-cov
// --workspace` never build this bench target, so nothing in this file has
// coverage instrumentation to attribute to.
const SHARED_WINNER_BOX_PX: f32 = 7.0;

/// Shared winner font size for the sparse / structural workloads.
// cov:ignore: bench harness constant — same reason as above.
const SHARED_WINNER_FONT_PX: f32 = 42.0;

/// Build the ten longhand declarations every sparse / structural rule sets.
// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
// never build this bench target, so nothing in this function has coverage
// instrumentation to attribute to.
fn shared_winner_body() -> String {
    let box_px = SHARED_WINNER_BOX_PX;
    let font_size = SHARED_WINNER_FONT_PX;
    format!(
        "margin-top: {box_px}px; margin-right: {box_px}px; \
         margin-bottom: {box_px}px; margin-left: {box_px}px; \
         padding-top: {box_px}px; padding-right: {box_px}px; \
         padding-bottom: {box_px}px; padding-left: {box_px}px; \
         font-size: {font_size}px; color: rgb(9, 8, 7)"
    )
}

/// The [`Winners`] every sparse / structural rule cascades to.
// cov:ignore: bench harness — same reason as `shared_winner_body` above.
fn shared_winners() -> Winners {
    Winners {
        font_size: SHARED_WINNER_FONT_PX,
        box_px: SHARED_WINNER_BOX_PX,
        color: CssColor {
            r: 9,
            g: 8,
            b: 7,
            a: 255,
        },
    }
}

/// Build a stylesheet of `n_rules` sparse class rules (`.s0`, `.s1`, …),
/// each setting [`DECLS_PER_RULE`] longhands to the shared winner values.
// cov:ignore: bench harness — same reason as `shared_winner_body` above.
fn sparse_class_stylesheet(n_rules: usize) -> (String, Winners) {
    let body = shared_winner_body();
    let mut css = String::new();
    for i in 0..n_rules {
        css.push_str(&format!(".s{i} {{ {body} }}\n"));
    }
    (css, shared_winners())
}

/// Build a stylesheet of `n_rules` sparse id rules (`#nid0`, `#nid1`, …),
/// each setting [`DECLS_PER_RULE`] longhands to the shared winner values.
// cov:ignore: bench harness — same reason as `shared_winner_body` above.
fn sparse_id_stylesheet(n_rules: usize) -> (String, Winners) {
    let body = shared_winner_body();
    let mut css = String::new();
    for i in 0..n_rules {
        css.push_str(&format!("#nid{i} {{ {body} }}\n"));
    }
    (css, shared_winners())
}

/// Build the single-rule `div:nth-child(2n+1)` stylesheet for a
/// [`BenchDoc::wide`] document: every odd-position `div` matches, every
/// even-position one must not.
// cov:ignore: bench harness — same reason as `shared_winner_body` above.
fn wide_nth_stylesheet() -> (String, Winners) {
    let body = shared_winner_body();
    (
        format!("div:nth-child(2n+1) {{ {body} }}\n"),
        shared_winners(),
    )
}

/// Build the single-rule `div:first-child` stylesheet for a
/// [`BenchDoc::chain`] document: every `div` is its parent's only child, so
/// every one matches.
// cov:ignore: bench harness — same reason as `shared_winner_body` above.
fn first_child_stylesheet() -> (String, Winners) {
    let body = shared_winner_body();
    (format!("div:first-child {{ {body} }}\n"), shared_winners())
}

/// Build the single-rule `div + div` stylesheet for a [`BenchDoc::wide`]
/// document: every `div` but the first matches.
// cov:ignore: bench harness — same reason as `shared_winner_body` above.
fn wide_adjacent_stylesheet() -> (String, Winners) {
    let body = shared_winner_body();
    (format!("div + div {{ {body} }}\n"), shared_winners())
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
///   pass, the collector's rule loop would never push a candidate, and this file
///   would report a large improvement while measuring nothing.
///
/// So the input assertions are not enough on their own: the function also
/// runs one throwaway `cascade()` and checks that *every* declaration reached
/// *every* element's computed values, and that both inherited ones reached
/// every text child. That probe is outside every timed region — it runs once
/// per config at setup.
/// `wrapped` places the `div`s under one `section` ([`BenchDoc::wide`])
/// instead of directly under the document ([`BenchDoc::new`]). Top-level
/// elements are each a root element with their own `rem` basis, so only the
/// wrapped shape lets repeated siblings share computed values the way list
/// items or table rows in a real document do.
fn workload(n_rules: usize, n_elems: usize, wrapped: bool) -> (BenchDoc, RuleTree, u64) {
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

    let doc = if wrapped {
        BenchDoc::wide(n_elems)
    } else {
        BenchDoc::new(n_elems)
    };
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
    // The wrapping `section` (node 1) matches none of the `div` rules.
    let first_checked = if wrapped { 2 } else { 1 };
    for (idx, cv) in probe.computed.iter().enumerate().skip(first_checked) {
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

/// Check one cascaded `div` (and its text child, for the two inherited
/// longhands) against either the shared winner values or the initial ones.
/// `div_id` is the arena index; the text child is its single child.
// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
// never build this bench target, so nothing in this function has coverage
// instrumentation to attribute to.
fn check_wide_div(
    probe: &raikiri_style::CascadeResult,
    doc: &BenchDoc,
    initial: &ComputedValues,
    want: &Winners,
    div_id: usize,
    matches: bool,
    what: &str,
) {
    let cv = &probe.computed[div_id];
    let want_margin = ComputedLengthPercentageOrAuto::Px(want.box_px);
    let want_padding = ComputedLengthPercentage::Px(want.box_px);
    if matches {
        assert_eq!(
            cv.font_size.px(),
            want.font_size,
            "{what} div at node {div_id} should match but does not carry the winning font-size",
        );
        assert_eq!(
            cv.color, want.color,
            "{what} div at node {div_id} should match but does not carry the winning color",
        );
        assert_eq!(
            cv.margin.top, want_margin,
            "{what} div at node {div_id}: margin-top does not carry the winning rule's value",
        );
        assert_eq!(
            cv.padding.top, want_padding,
            "{what} div at node {div_id}: padding-top does not carry the winning rule's value",
        );
    } else {
        assert_eq!(
            cv.font_size.px(),
            initial.font_size.px(),
            "{what} div at node {div_id} must NOT match — a matcher that              returned true unconditionally would wrongly style it",
        );
        assert_eq!(
            cv.color, initial.color,
            "{what} div at node {div_id} must keep the initial color",
        );
    }
    // The text child inherits `font-size` and `color` from its `div` parent
    // (and only those two of the ten longhands); `margin`/`padding` are not
    // inherited and are not checked here.
    let text_id = doc.nodes[div_id].children[0];
    let text_cv = &probe.computed[text_id];
    if matches {
        assert_eq!(
            text_cv.font_size.px(),
            want.font_size,
            "{what} text under node {div_id} should inherit the winning font-size",
        );
        assert_eq!(
            text_cv.color, want.color,
            "{what} text under node {div_id} should inherit the winning color",
        );
    } else {
        assert_eq!(
            text_cv.font_size.px(),
            initial.font_size.px(),
            "{what} text under non-matching node {div_id} must keep the initial font-size",
        );
        assert_eq!(
            text_cv.color, initial.color,
            "{what} text under non-matching node {div_id} must keep the initial color",
        );
    }
}

/// Assemble the sparse-class config: `n_rules` `.s{i}` rules against a
/// [`BenchDoc::wide`] document whose `div`s carry distinct classes
/// ([`BenchDoc::assign_sparse_classes`]). With `stride = n_rules / n_elems`,
/// each classed `div` matches exactly one rule and `n_rules - n_matched`
/// rules match nothing at all, so per-element time is dominated by rejecting
/// non-matching rules — the quantity a rule index would attack. Every 5th
/// `div` is an unmatched control proving the negative direction.
///
/// On [`workload`]'s load-bearing-assertions logic: the rule-count and
/// per-rule declaration-count checks prove the input side, and the
/// per-`div` winner-vs-initial sweep proves every match ran to completion.
// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace`
// never build this bench target, so nothing in this function has coverage
// instrumentation to attribute to.
fn sparse_class_workload(n_rules: usize, n_elems: usize) -> (BenchDoc, RuleTree, u64) {
    assert!(
        n_elems >= 1 && n_rules >= n_elems && n_rules.is_multiple_of(n_elems),
        "need stride = n_rules / n_elems >= 1 so every classed div names a distinct rule"
    );
    let mut tree = RuleTree::empty();
    let (css, want) = sparse_class_stylesheet(n_rules);
    tree.add_stylesheet(&css, Origin::Author);
    assert_eq!(
        tree.style_rules().len(),
        n_rules,
        "sparse-class stylesheet did not parse into the expected rule count"
    );
    for rule in tree.style_rules() {
        assert_eq!(
            rule.declarations().len(),
            DECLS_PER_RULE,
            "sparse-class rule did not parse into the expected declaration count"
        );
    }

    let mut doc = BenchDoc::wide(n_elems);
    doc.assign_sparse_classes(n_rules / n_elems);

    let initial = ComputedValues::initial();
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "shared winner values must differ from the initial ones, or the probe below would be vacuous"
    );

    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    let divs = doc.nodes[1].children.clone();
    assert_eq!(
        divs.len(),
        n_elems,
        "wide document does not hold n_elems divs"
    );
    for (k, div) in divs.iter().enumerate() {
        check_wide_div(
            &probe,
            &doc,
            &initial,
            &want,
            *div,
            k % 5 != 4,
            "sparse-class",
        );
    }

    (doc, tree, n_elems as u64)
}

/// Assemble the sparse-id config: `n_rules` `#nid{i}` rules against a
/// [`BenchDoc::wide`] document with distinct ids
/// ([`BenchDoc::assign_sparse_ids`]). Same sparsity contract and probe
/// shape as [`sparse_class_workload`], exercising the id-lookup path
/// instead of the class-token path.
// cov:ignore: bench harness — same reason as `sparse_class_workload` above.
fn sparse_id_workload(n_rules: usize, n_elems: usize) -> (BenchDoc, RuleTree, u64) {
    assert!(
        n_elems >= 1 && n_rules >= n_elems && n_rules.is_multiple_of(n_elems),
        "need stride = n_rules / n_elems >= 1 so every id div names a distinct rule"
    );
    let mut tree = RuleTree::empty();
    let (css, want) = sparse_id_stylesheet(n_rules);
    tree.add_stylesheet(&css, Origin::Author);
    assert_eq!(
        tree.style_rules().len(),
        n_rules,
        "sparse-id stylesheet did not parse into the expected rule count"
    );
    for rule in tree.style_rules() {
        assert_eq!(
            rule.declarations().len(),
            DECLS_PER_RULE,
            "sparse-id rule did not parse into the expected declaration count"
        );
    }

    let mut doc = BenchDoc::wide(n_elems);
    doc.assign_sparse_ids(n_rules / n_elems);

    let initial = ComputedValues::initial();
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "shared winner values must differ from the initial ones, or the probe below would be vacuous"
    );

    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    let divs = doc.nodes[1].children.clone();
    assert_eq!(
        divs.len(),
        n_elems,
        "wide document does not hold n_elems divs"
    );
    for (k, div) in divs.iter().enumerate() {
        check_wide_div(&probe, &doc, &initial, &want, *div, k % 5 != 4, "sparse-id");
    }

    (doc, tree, n_elems as u64)
}

/// Assemble the wide `nth-child` config: one `div:nth-child(2n+1)` rule
/// ([`wide_nth_stylesheet`]) against a [`BenchDoc::wide`] document. Every
/// element triggers a sibling-position scan over up to `n_elems` siblings,
/// so total matching work scales quadratically without a sibling-position
/// cache — that scaling is the signal, measured at 500/1000/2000/4000.
/// Odd positions must match, even positions must not.
// cov:ignore: bench harness — same reason as `sparse_class_workload` above.
fn wide_nth_workload(n_elems: usize) -> (BenchDoc, RuleTree, u64) {
    let mut tree = RuleTree::empty();
    let (css, want) = wide_nth_stylesheet();
    tree.add_stylesheet(&css, Origin::Author);
    assert_eq!(
        tree.style_rules().len(),
        1,
        "nth-child stylesheet did not parse into exactly one rule"
    );
    assert_eq!(
        tree.style_rules()[0].declarations().len(),
        DECLS_PER_RULE,
        "nth-child rule did not parse into the expected declaration count"
    );

    let doc = BenchDoc::wide(n_elems);
    let initial = ComputedValues::initial();
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "shared winner values must differ from the initial ones, or the probe below would be vacuous"
    );

    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    let divs = doc.nodes[1].children.clone();
    assert_eq!(
        divs.len(),
        n_elems,
        "wide document does not hold n_elems divs"
    );
    for (k, div) in divs.iter().enumerate() {
        // Sibling position `k + 1` is odd iff `k` is even.
        check_wide_div(&probe, &doc, &initial, &want, *div, k % 2 == 0, "nth-child");
    }

    (doc, tree, n_elems as u64)
}

/// Assemble the wide adjacent-sibling config: one `div + div` rule
/// ([`wide_adjacent_stylesheet`]) against a [`BenchDoc::wide`] document.
/// Every `div` but the first must match — the `NextSibling` arm at the same
/// four widths as [`wide_nth_workload`].
// cov:ignore: bench harness — same reason as `sparse_class_workload` above.
fn wide_adjacent_workload(n_elems: usize) -> (BenchDoc, RuleTree, u64) {
    let mut tree = RuleTree::empty();
    let (css, want) = wide_adjacent_stylesheet();
    tree.add_stylesheet(&css, Origin::Author);
    assert_eq!(
        tree.style_rules().len(),
        1,
        "adjacent-sibling stylesheet did not parse into exactly one rule"
    );
    assert_eq!(
        tree.style_rules()[0].declarations().len(),
        DECLS_PER_RULE,
        "adjacent-sibling rule did not parse into the expected declaration count"
    );

    let doc = BenchDoc::wide(n_elems);
    let initial = ComputedValues::initial();
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "shared winner values must differ from the initial ones, or the probe below would be vacuous"
    );

    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    let divs = doc.nodes[1].children.clone();
    assert_eq!(
        divs.len(),
        n_elems,
        "wide document does not hold n_elems divs"
    );
    for (k, div) in divs.iter().enumerate() {
        check_wide_div(&probe, &doc, &initial, &want, *div, k != 0, "adjacent");
    }

    (doc, tree, n_elems as u64)
}

/// Assemble the deep-narrow `:first-child` config: one `div:first-child`
/// rule ([`first_child_stylesheet`]) against a `depth`-deep
/// [`BenchDoc::chain`]. Every parent has a single child, so this measures
/// the per-element cost of structural matching where a sibling-position
/// cache cannot help and must not add per-parent allocations. Every `div`
/// must match; a chain has no non-first child to probe the negative case.
// cov:ignore: bench harness — same reason as `sparse_class_workload` above.
fn first_child_chain_workload(depth: usize) -> (BenchDoc, RuleTree, u64) {
    let mut tree = RuleTree::empty();
    let (css, want) = first_child_stylesheet();
    tree.add_stylesheet(&css, Origin::Author);
    assert_eq!(
        tree.style_rules().len(),
        1,
        "first-child stylesheet did not parse into exactly one rule"
    );
    assert_eq!(
        tree.style_rules()[0].declarations().len(),
        DECLS_PER_RULE,
        "first-child rule did not parse into the expected declaration count"
    );

    let doc = BenchDoc::chain(depth);
    let initial = ComputedValues::initial();
    assert!(
        want.font_size != initial.font_size.px()
            && want.box_px != 0.0
            && want.color != initial.color,
        "shared winner values must differ from the initial ones, or the probe below would be vacuous"
    );

    let probe = cascade(&doc, &tree).expect(CASCADE_NEVER_ERRS);
    assert_eq!(
        probe.computed.len(),
        doc.node_count(),
        "cascade did not produce one entry per arena node"
    );

    let want_margin = ComputedLengthPercentageOrAuto::Px(want.box_px);
    let want_padding = ComputedLengthPercentage::Px(want.box_px);
    for i in 1..=depth {
        let cv = &probe.computed[i];
        assert_eq!(
            cv.font_size.px(),
            want.font_size,
            "div at chain position {i} of {depth} is a first child but does not carry the winning font-size",
        );
        assert_eq!(
            cv.color, want.color,
            "div at chain position {i} of {depth} is a first child but does not carry the winning color",
        );
        assert_eq!(
            cv.margin.top, want_margin,
            "div at chain position {i} of {depth}: margin-top does not carry the winning rule's value",
        );
        assert_eq!(
            cv.padding.top, want_padding,
            "div at chain position {i} of {depth}: padding-top does not carry the winning rule's value",
        );
    }

    (doc, tree, depth as u64)
}

/// Returns the workload in `slot`, building it with `build` on first use and
/// checking that it reports `expected` throughput elements.
///
/// Criterion calls `bench_function` for every configuration but runs its
/// closure only for the benchmarks a name filter selects. Building each
/// workload inside its closure therefore keeps the setup of unselected
/// configurations (construction, probe cascade, assertions) out of a filtered
/// run, and out of any profile taken of it. The selected configuration's own
/// setup now runs at the start of its warm-up (inside a criterion profiler's
/// window and `--quick`'s time budget), never inside a timed iteration.
/// Throughput has to be set before `bench_function`, so it is computed from
/// the configuration up front and the workload's own count is checked against
/// it once built.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn prepared<T>(slot: &OnceCell<T>, expected: u64, build: impl FnOnce() -> (T, u64)) -> &T {
    slot.get_or_init(|| {
        let (workload, count) = build();
        assert_eq!(
            count, expected,
            "workload throughput differs from the configured count"
        );
        workload
    })
}

/// Splits a workload builder's `(doc, tree, count)` into the cached pair and
/// its count.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn cached((doc, tree, count): (BenchDoc, RuleTree, u64)) -> ((BenchDoc, RuleTree), u64) {
    ((doc, tree), count)
}

/// `n_rules` generated rules wrapped in `@media {query}` over `n_elems`
/// elements. The probe checks the rules apply exactly when `active`, and that
/// the opposite media context on the same tree inverts that.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn media_workload(
    n_rules: usize,
    n_elems: usize,
    query: &str,
    active: bool,
) -> (BenchDoc, RuleTree, u64) {
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
    (doc, tree, n_elems as u64)
}

/// The inputs of both DOM-query benchmarks: a document whose `section` holds
/// `n_elems` `div`s, the ids of those `div`s, and the `div:nth-child(odd)`
/// query run over them.
// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
type NthChildQuery = (BenchDoc, Vec<StyleNodeId>, SelectorQuery);

// cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
fn nth_child_query_workload(n_elems: usize) -> (NthChildQuery, u64) {
    let doc = BenchDoc::wide(n_elems);
    let section = StyleNodeId::new(1);
    let divs = doc.nodes[1]
        .children
        .iter()
        .map(|&id| StyleNodeId::new(id as u64))
        .collect::<Vec<_>>();
    let query = SelectorQuery::parse("div:nth-child(odd)").expect("valid selector");
    let ancestors = [section];
    let per_call = divs
        .iter()
        .filter(|&&id| query.matches(&doc, id, &ancestors))
        .count();
    let matcher = query.matcher(&doc, None);
    let shared = divs
        .iter()
        .filter(|&&id| matcher.matches(id, &ancestors))
        .count();
    assert_eq!(per_call, n_elems.div_ceil(2), "every odd div matches");
    assert_eq!(
        shared, per_call,
        "shared matcher disagrees with per-call matching"
    );
    let count = divs.len() as u64;
    ((doc, divs, query), count)
}

fn bench_cascade(c: &mut Criterion) {
    let mut group = c.benchmark_group("cascade");

    // Exercise both rule-heavy and element-heavy workloads.
    // cov:ignore: bench harness — never instrumented under `cargo llvm-cov
    // --workspace` (bench targets are not built by `cargo test`/`cargo
    // llvm-cov --workspace`), so nothing in this loop has coverage
    // instrumentation to attribute to.
    for (name, n_rules, n_elems, wrapped) in [
        ("rule_heavy_50x500", 50usize, 500usize, false),
        ("element_heavy_5x2000", 5usize, 2000usize, false),
        ("element_heavy_wrapped_5x2000", 5usize, 2000usize, true),
    ] {
        // Report throughput in declarations so the workloads share a unit.
        let declarations = (n_rules * n_elems * DECLS_PER_RULE) as u64;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(declarations));
        group.bench_function(name, |b| {
            let (doc, tree) = prepared(&slot, declarations, || {
                cached(workload(n_rules, n_elems, wrapped))
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Keep inactive rules in the tree to measure media filtering separately
    // from selector matching. Also exercise a fully active media workload.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    for (name, n_rules, n_elems, query, active) in [
        ("media_inactive_2000x1000", 2000, 1000, "screen", false),
        ("media_active_50x500", 50, 500, "print", true),
    ] {
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(n_elems as u64));
        group.bench_function(name, |b| {
            let (doc, tree) = prepared(&slot, n_elems as u64, || {
                cached(media_workload(n_rules, n_elems, query, active))
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Combinator-chain config — see the module doc's "Combinator-chain
    // workload" section for why the two configs above cannot exercise
    // `match_combinator_chain` at all.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    {
        // Throughput denominated in elements attempted, not declarations:
        // every `div` in the chain triggers exactly one top-level call into
        // `match_combinator_chain` (see the module doc), which is the
        // quantity whose per-call allocation cost is under study here —
        // unlike the two configs above, most attempts do not go on to
        // produce any declarations at all.
        let match_attempts = COMBINATOR_CHAIN_DOC_DEPTH as u64;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(match_attempts));
        group.bench_function("combinator_chain_5000x4", |b| {
            let (doc, tree) = prepared(&slot, match_attempts, || {
                cached(combinator_chain_workload(
                    COMBINATOR_CHAIN_SELECTOR_DEPTH,
                    COMBINATOR_CHAIN_DOC_DEPTH,
                ))
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Inline-style configs — the same source on every element versus a
    // distinct source per element.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    for (name, repeated) in [
        ("inline_style_repeated_2000", true),
        ("inline_style_unique_2000", false),
    ] {
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(2000));
        group.bench_function(name, |b| {
            let (doc, tree) = prepared(&slot, 2000, || {
                cached(inline_style_workload(2000, repeated))
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // `:has()` config — one relative-selector search per element.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    {
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(2000));
        group.bench_function("has_child_chain_2000", |b| {
            let (doc, tree) = prepared(&slot, 2000, || cached(has_child_chain_workload(2000)));
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // DOM-query configs — a `querySelectorAll`-style pass testing every
    // child of a wide `section` against `div:nth-child(odd)`, either with a
    // fresh per-call match (each call rebuilds the sibling positions) or
    // through one `SelectorQuery::matcher` shared by the whole pass.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    {
        let ancestors = [StyleNodeId::new(1)];
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(4000));
        group.bench_function("query_nth_child_per_call_4000", |b| {
            let (doc, divs, query) = prepared(&slot, 4000, || nth_child_query_workload(4000));
            b.iter(|| {
                divs.iter()
                    .filter(|&&id| query.matches(doc, id, &ancestors))
                    .count()
            });
        });
        group.bench_function("query_nth_child_shared_matcher_4000", |b| {
            let (doc, divs, query) = prepared(&slot, 4000, || nth_child_query_workload(4000));
            b.iter(|| {
                let matcher = query.matcher(doc, None);
                divs.iter()
                    .filter(|&&id| matcher.matches(id, &ancestors))
                    .count()
            });
        });
    }

    // Descendant-miss config — every (element, rule) pair is a descendant
    // combinator miss whose required ancestor class never occurs, the case an
    // ancestor Bloom filter rejects without walking the ancestor chain.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    {
        let match_attempts = (DESCENDANT_MISS_RULES * DESCENDANT_MISS_DOC_DEPTH) as u64;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(match_attempts));
        group.bench_function("descendant_miss_1000x50", |b| {
            let (doc, tree) = prepared(&slot, match_attempts, || {
                cached(descendant_miss_workload(
                    DESCENDANT_MISS_RULES,
                    DESCENDANT_MISS_DOC_DEPTH,
                ))
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
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
        // Throughput denominated in elements attempted, same convention as
        // `combinator_chain_5000x4` above: one `div` per article plus the
        // negative control.
        let match_attempts = MIXED_CHAIN_ARTICLES as u64 + 1;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(match_attempts));
        group.bench_function("mixed_combinator_chain_300", |b| {
            let (doc, tree) = prepared(&slot, match_attempts, || {
                cached(mixed_combinator_workload(MIXED_CHAIN_ARTICLES))
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Sparse class/id configs — 2000 rules whose selectors each match at
    // most one of 500 elements. Per-element time is dominated by rejecting
    // non-matching rules, the quantity a rule index would attack; the dense
    // configs above cannot show it because every rule matches every element.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    for (name, n_rules, n_elems, is_id) in [
        ("sparse_class_2000x500", 2000usize, 500usize, false),
        ("sparse_id_2000x500", 2000usize, 500usize, true),
    ] {
        // Throughput in elements: rule count is fixed per bench name, so
        // per-element time is directly comparable across names.
        let elems = n_elems as u64;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(elems));
        group.bench_function(name, |b| {
            let (doc, tree) = prepared(&slot, elems, || {
                cached(if is_id {
                    sparse_id_workload(n_rules, n_elems)
                } else {
                    sparse_class_workload(n_rules, n_elems)
                })
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Wide-sibling configs at 500/1000/2000/4000 elements. `nth-child`
    // exercises the per-element sibling-position scan (quadratic total
    // without a sibling-position cache — that scaling across the four
    // widths is the signal); `div + div` exercises the `NextSibling` arm
    // over the same widths.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    for n_elems in [500usize, 1000, 2000, 4000] {
        let elems = n_elems as u64;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(elems));
        group.bench_function(format!("nth_child_wide_{n_elems}"), |b| {
            let (doc, tree) = prepared(&slot, elems, || cached(wide_nth_workload(n_elems)));
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    for n_elems in [500usize, 1000, 2000, 4000] {
        let elems = n_elems as u64;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(elems));
        group.bench_function(format!("adjacent_wide_{n_elems}"), |b| {
            let (doc, tree) = prepared(&slot, elems, || cached(wide_adjacent_workload(n_elems)));
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    // Deep-narrow structural config — the counterpart of the wide configs
    // above: many parents with one child each.
    //
    // cov:ignore: bench harness — same reason as `BenchDoc::chain` above.
    {
        let elems = FIRST_CHILD_CHAIN_DEPTH as u64;
        let slot = OnceCell::new();
        group.throughput(Throughput::Elements(elems));
        group.bench_function("first_child_chain_5000", |b| {
            let (doc, tree) = prepared(&slot, elems, || {
                cached(first_child_chain_workload(FIRST_CHILD_CHAIN_DEPTH))
            });
            b.iter_with_large_drop(|| cascade(doc, tree).expect(CASCADE_NEVER_ERRS));
        });
    }

    group.finish();
}

// criterion's `criterion_main!`/`criterion_group!` pair is inlined here: the
// macro generates a `pub fn`, which trips the workspace's
// `missing_docs = "warn"` (an error under the gate's `-D warnings`), and the
// `allow` cannot be attached to a macro invocation. This is the macro body,
// minus its second redundant `configure_from_args()`.
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_cascade(&mut criterion);
    criterion.final_summary();
}
