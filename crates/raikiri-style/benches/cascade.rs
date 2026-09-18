//! Cascade hot-loop benchmark — guards the **per-declaration constant**.
//!
//! # Why this benchmark exists
//!
//! A shorthand-expansion guard was added to
//! `cascade::collect_cascaded` and, in doing so, started moving a
//! `Declaration` (72 bytes) by value once per declaration. The declaration
//! *count* did not change — `collect_cascaded` is a single whole-tree walk
//! either way — only the cost of one loop iteration did (see below for the
//! per-declaration figures, which differ by source). That is
//! a regression class neither `cargo test` nor `cargo clippy` can see: the
//! output is bit-identical and no lint fires. It was caught only because a
//! reviewer hand-rolled a throwaway timing harness (confirmed at +22% / +16%).
//!
//! This file makes that harness a repo artifact so the same class is
//! measurable on demand instead of by luck.
//!
//! # Reference numbers
//!
//! From that review, default release profile, wall-clock per
//! `cascade()` call. Note that review recorded the harness **twice**: this table is
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
//! checked by reintroducing the original regression — `expand_shorthand_into`
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
//! per-declaration cost as an unchanged one. Not implemented yet;
//! until it is, read the ≈4 ns as
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
//! Gate integration has to solve the noise problem
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
//! # Combinator-chain workload
//!
//! `match_complex_selector_list` (`crates/raikiri-style/src/cascade.rs`)
//! matches a selector's rightmost compound against the element directly, and
//! only calls into `match_combinator_chain` once `iter.next_sequence()`
//! reports a combinator remaining further left. Both configs above generate
//! bare-tag, zero-combinator rules (`div { … }`), so `next_sequence()`
//! returns `None` immediately for every one of them and
//! `match_combinator_chain` is never invoked at all — neither config can see
//! a change to that function's cost.
//!
//! `match_combinator_chain` walks its choice points on an explicit
//! `Vec<Frame>` stack (one heap allocation per top-level call) rather than
//! native recursion, so a chain of ancestor/sibling combinators has a real
//! per-call allocation cost that this file's other two configs cannot
//! exercise. The `combinator_chain_5000x4` config gives that allocation a
//! workload: a single rule with a 5-compound, 4-child-combinator selector
//! (`div > div > div > div > div`) matched against a straight 5000-deep
//! parent-child chain of `div`s ([`BenchDoc::chain`]). The rightmost compound
//! is a bare `div`, so every element in the chain triggers exactly one
//! top-level `match_combinator_chain` call — that element count is this
//! config's throughput unit (see the `group.throughput` call site), not
//! declarations, since most calls do not go on to produce any.
//!
//! 4 combinators sits within the 2–5 combinator range typical of real-world
//! selectors. A deeper chain would still allocate on every call (the `Vec`
//! grows via push-triggered doubling with no cap), but would stop
//! representing a typical selector shape.
//!
//! [`combinator_chain_workload`]'s probe checks both directions: elements at
//! chain position `selector_depth` or deeper carry the winning rule's
//! values, and — unlike [`workload`]'s single-direction check — the
//! `selector_depth - 1` elements nearest the root keep their *initial*
//! values, since they run out of ancestors before the selector's combinators
//! do. A matcher that ignored ancestor structure and returned true
//! unconditionally would pass every other assertion in this file while
//! failing only this one.
//!
//! `combinator_chain_5000x4`'s selector is pure child combinators
//! (`Combinator::Child`), which never backtrack — `PendingCandidates::Child`
//! has exactly one candidate, so `match_combinator_chain` either matches it
//! or gives up immediately. It cannot exercise the retry
//! `Combinator::Descendant` needs when a closer candidate matches the
//! `Descendant` step but then fails a `Combinator::Child` step further left
//! (see `match_combinator_chain`'s own doc, "load-bearing" retry note, and
//! the regression test `descendant_retry_past_a_failed_child_combinator_candidate_is_required`
//! in `crates/raikiri-style/src/cascade.rs`) — each such failure pops one
//! choice-point frame and resumes the outer `Descendant` search, real stack
//! churn a pure child chain never triggers.
//!
//! `mixed_combinator_chain_300` gives that path a workload:
//! [`BenchDoc::mixed_chain`] builds a `section` followed by a straight
//! 300-deep parent-child chain of `article`s, each with its own `div` child,
//! matched against `section > article div`. The `div` under `article` level
//! `k` (1-indexed, closest to `section` = 1) only matches once the
//! `Descendant` search reaches level 1 — every closer `article` candidate
//! matches the `Descendant` step but fails the following `Child` step (its
//! own immediate parent is another `article`, not `section`), so level `k`
//! costs `k` push-then-pop retry cycles, not 1. [`mixed_combinator_workload`]
//! also matches against an `article` → `article` → `div` subtree with no
//! `section` ancestor anywhere at all, which must not match — the same
//! both-directions guard `combinator_chain_workload` uses.
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
//! Wiring it into the discipline is future work (a process change, requiring
//! review and approval before adoption).

use criterion::{Criterion, Throughput};
use raikiri_style::{
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedValues, CssColor, Origin,
    RuleTree, StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, cascade,
};

/// Declarations emitted per generated rule. Kept at 10 to match the
/// reference numbers in the module doc.
const DECLS_PER_RULE: usize = 10;

/// Shared `.expect()` message for every `cascade()` call site in this file —
/// see [`cascade`]'s own doc: it always returns `Ok` in the current
/// implementation.
// cov:ignore: bench harness constant — `cargo test`/`cargo llvm-cov
// --workspace` never build this bench target, so nothing in this file has
// coverage instrumentation to attribute to.
const CASCADE_NEVER_ERRS: &str = "cascade never returns Err in the current implementation";

/// Compound count of the `combinator_chain` config's selector — see the
/// module doc's "Combinator-chain workload" section.
// cov:ignore: bench harness constant — `cargo test`/`cargo llvm-cov
// --workspace` never build this bench target, so nothing in this file has
// coverage instrumentation to attribute to.
const COMBINATOR_CHAIN_SELECTOR_DEPTH: usize = 5;

/// Depth of the `combinator_chain` config's `div` chain — see the module
/// doc's "Combinator-chain workload" section.
// cov:ignore: same reason as the constant above.
const COMBINATOR_CHAIN_DOC_DEPTH: usize = 5000;

/// `article` level count of the `mixed_combinator_chain` config — see
/// [`BenchDoc::mixed_chain`]'s doc. Total retry cycles across the whole
/// config scale roughly with the square of this count (level `k`'s `div`
/// costs `k` cycles), so this is kept an order of magnitude below
/// [`COMBINATOR_CHAIN_DOC_DEPTH`].
// cov:ignore: same reason as the constant above.
const MIXED_CHAIN_ARTICLES: usize = 300;

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

    // `rule_heavy` stresses the rules-per-element factor, `element_heavy` the
    // node count. Healthy absolute cost is ≈20 and ≈33 ns/declaration
    // respectively at the `832500e` baseline (they are not comparable — see
    // the throughput note below).
    // The ≈4 ns/declaration in the module doc is the regression *delta*, not
    // the healthy cost.
    //
    // cov:ignore: bench harness — never instrumented under `cargo llvm-cov
    // --workspace` (bench targets are not built by `cargo test`/`cargo
    // llvm-cov --workspace`), so nothing in this loop has coverage
    // instrumentation to attribute to.
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
