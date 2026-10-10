use super::*;
use crate::cascade::first_line::cascade_with_first_line_within;
use crate::cascade::limits::WalkCounts;
use crate::cascade::{CascadeLimits, CascadeOptions, cascade_with_options};
use crate::error::{CascadeError, CascadeLimitKind};
use crate::page::PageContextQuery;

const SVG: &str = "http://www.w3.org/2000/svg";

fn unlimited() -> CascadeLimits {
    CascadeLimits {
        max_candidates_per_element: None,
        max_declarations_visited: None,
        max_selector_tests: None,
        max_retained_bytes: None,
        max_output_bytes: None,
    }
}

fn walk_within(
    doc: &TestDoc,
    tree: &RuleTree,
    limits: CascadeLimits,
    sibling_sharing: bool,
) -> Result<WalkOutputs, CascadeError> {
    walk(
        doc,
        tree,
        &MediaContext::default(),
        WalkOptions {
            sibling_sharing,
            limits,
            ..WalkOptions::default()
        },
    )
}

/// The kind, limit and count of a cascade that passed a limit.
fn exceeded<T>(result: Result<T, CascadeError>) -> (CascadeLimitKind, u64, u64) {
    match result {
        Err(CascadeError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => (kind, limit, actual),
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => panic!("the cascade stayed within its limits"),
    }
}

/// A walk's outputs with what it counted, for comparing walks.
fn outputs_and_counts(
    doc: &TestDoc,
    tree: &RuleTree,
    limits: CascadeLimits,
    sibling_sharing: bool,
) -> (ComparedWalkOutputs, WalkCounts, usize) {
    let (outputs, shared) = walk_outputs_with(
        doc,
        tree,
        WalkOptions {
            sibling_sharing,
            limits,
            ..WalkOptions::default()
        },
    );
    let counts = walk_within(doc, tree, limits, sibling_sharing)
        .expect("the walk succeeds")
        .counts;
    (outputs, counts, shared)
}

const LIMITS_CSS: &str = "
    p { color: rgb(1, 2, 3); margin: 4px }
    p::before { content: 'b' }
    article::first-line { letter-spacing: 1px }
    article p::first-letter { font-size: 2em }
    rect { color: blue; opacity: .25 }
    [data-x], div::after { padding: 1px }
";

/// A document every count sees: element and pseudo-element rules, a rule
/// with several selectors, inline styles seen once and repeated,
/// presentational hints, SVG paint properties, first-line and first-letter
/// styles, and repeated siblings that share.
fn limits_doc() -> TestDoc {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, LIMITS_CSS);
    let body = doc.push_element(0, "body", None);
    let article = doc.push_element(body, "article", None);
    for _ in 0..3 {
        let p = doc.push_element(article, "p", None);
        doc.push_text(p, "text");
        let span = doc.push_element(p, "span", Some("color: red"));
        doc.push_text(span, "x");
    }
    doc.push_element_with_attrs(body, "div", Some("font-weight: bold"), &[("data-x", "1")]);
    doc.push_element_with_attrs(body, "img", None, &[("width", "10"), ("height", "20")]);
    let svg = doc.push_element_with_namespace(body, "svg", SVG, &[]);
    for _ in 0..2 {
        doc.push_element_with_namespace(svg, "rect", SVG, &[]);
    }
    doc
}

#[test]
fn limits_at_what_the_document_needs_change_nothing() {
    let doc = limits_doc();
    let tree = build_rule_tree(&doc);
    let (reference, counts, shared) = outputs_and_counts(&doc, &tree, unlimited(), true);
    assert!(shared > 0, "the repeated paragraphs shared nothing");
    for (name, count) in [
        ("candidates of one element", counts.max_element_candidates),
        ("declarations", counts.declarations_visited),
        ("selector tests", counts.selector_tests),
        ("retained bytes", counts.retained_bytes),
        ("output bytes", counts.output_bytes),
    ] {
        assert!(count > 0, "the document counts no {name}");
    }
    let exact = CascadeLimits {
        max_candidates_per_element: Some(
            u32::try_from(counts.max_element_candidates).expect("a small count"),
        ),
        max_declarations_visited: Some(counts.declarations_visited),
        max_selector_tests: Some(counts.selector_tests),
        max_retained_bytes: Some(counts.retained_bytes),
        max_output_bytes: Some(counts.output_bytes),
    };
    for limits in [exact, CascadeLimits::default()] {
        let (outputs, within, _) = outputs_and_counts(&doc, &tree, limits, true);
        assert_eq!(outputs, reference, "{limits:?}");
        assert_eq!(within, counts, "{limits:?}");
    }
}

#[test]
fn one_less_than_the_document_needs_fails() {
    let doc = limits_doc();
    let tree = build_rule_tree(&doc);
    let (_, counts, _) = outputs_and_counts(&doc, &tree, unlimited(), true);
    let cases = [
        (
            CascadeLimitKind::CandidatesPerElement,
            counts.max_element_candidates,
            CascadeLimits {
                max_candidates_per_element: Some(
                    u32::try_from(counts.max_element_candidates - 1).expect("a small count"),
                ),
                ..unlimited()
            },
        ),
        (
            CascadeLimitKind::DeclarationsVisited,
            counts.declarations_visited,
            CascadeLimits {
                max_declarations_visited: Some(counts.declarations_visited - 1),
                ..unlimited()
            },
        ),
        (
            CascadeLimitKind::SelectorTests,
            counts.selector_tests,
            CascadeLimits {
                max_selector_tests: Some(counts.selector_tests - 1),
                ..unlimited()
            },
        ),
        (
            CascadeLimitKind::RetainedBytes,
            counts.retained_bytes,
            CascadeLimits {
                max_retained_bytes: Some(counts.retained_bytes - 1),
                ..unlimited()
            },
        ),
        (
            CascadeLimitKind::OutputBytes,
            counts.output_bytes,
            CascadeLimits {
                max_output_bytes: Some(counts.output_bytes - 1),
                ..unlimited()
            },
        ),
    ];
    for (kind, needed, limits) in cases {
        for sibling_sharing in [true, false] {
            let (failed, limit, actual) =
                exceeded(walk_within(&doc, &tree, limits, sibling_sharing));
            assert_eq!(failed, kind, "{limits:?}");
            assert_eq!(limit, needed - 1);
            assert!(
                limit < actual && actual <= needed,
                "{kind:?}: {actual} of {needed}"
            );
        }
    }
}

#[test]
fn the_counts_do_not_depend_on_sharing() {
    let doc = limits_doc();
    let tree = build_rule_tree(&doc);
    let sharing = sharing_doc(StyleQuirksMode::NoQuirks);
    let sharing_tree = build_rule_tree(&sharing);
    for (doc, tree) in [(&doc, &tree), (&sharing, &sharing_tree)] {
        let (_, unshared, none) = outputs_and_counts(doc, tree, unlimited(), false);
        let (_, shared, some) = outputs_and_counts(doc, tree, unlimited(), true);
        assert_eq!(none, 0);
        assert!(some > 0);
        assert_eq!(shared, unshared);
    }
}

/// A document of one `<body>` with the elements `build` adds under it, styled
/// by `css`.
fn small_doc(css: &str, build: impl FnOnce(&mut TestDoc, usize)) -> (TestDoc, RuleTree) {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, css);
    let body = doc.push_element(0, "body", None);
    build(&mut doc, body);
    let tree = build_rule_tree(&doc);
    (doc, tree)
}

fn candidate_limit(count: u32) -> CascadeLimits {
    CascadeLimits {
        max_candidates_per_element: Some(count),
        ..unlimited()
    }
}

#[test]
fn every_kind_of_candidate_counts_against_its_element() {
    let rule = small_doc("p { color: red; color: blue }", |doc, body| {
        doc.push_element(body, "p", None);
    });
    let pseudo = small_doc("p::before { color: red; color: blue }", |doc, body| {
        doc.push_element(body, "p", None);
    });
    let inline = small_doc("", |doc, body| {
        doc.push_element(body, "p", Some("color: red; color: blue"));
    });
    let hints = small_doc("", |doc, body| {
        doc.push_element_with_attrs(body, "img", None, &[("width", "10"), ("height", "20")]);
    });
    for (doc, tree) in [&rule, &pseudo, &inline, &hints] {
        assert_eq!(
            exceeded(walk_within(doc, tree, candidate_limit(1), true)),
            (CascadeLimitKind::CandidatesPerElement, 1, 2)
        );
        assert!(walk_within(doc, tree, candidate_limit(2), true).is_ok());
    }
}

#[test]
fn a_style_attribute_stops_being_parsed_past_the_element_limit() {
    // The block expands to far more declarations than the limit; parsing
    // stops at the first one past it, as `margin` adds its four longhands.
    let style = format!("color: red; margin: 1px;{}", " color: blue;".repeat(1000));
    let (doc, tree) = small_doc("", |doc, body| {
        doc.push_element(body, "p", Some(&style));
    });
    assert_eq!(
        exceeded(walk_within(&doc, &tree, candidate_limit(2), true)),
        (CascadeLimitKind::CandidatesPerElement, 2, 5)
    );
}

#[test]
fn stored_style_attributes_past_their_budget_change_nothing() {
    // Without room for any stored block, every repeated style attribute is
    // parsed for each element instead, with the same results and counts.
    let doc = sharing_doc(StyleQuirksMode::NoQuirks);
    let tree = build_rule_tree(&doc);
    let (reference, counts, _) = outputs_and_counts(&doc, &tree, unlimited(), true);
    let options = WalkOptions {
        stored_block_budget: 0,
        limits: unlimited(),
        ..WalkOptions::default()
    };
    let (outputs, _) = walk_outputs_with(&doc, &tree, options);
    assert_eq!(outputs, reference);
    let walked = walk(&doc, &tree, &MediaContext::default(), options).expect("the walk succeeds");
    assert_eq!(walked.counts, counts);
}

#[test]
fn a_stored_style_attribute_counts_every_declaration() {
    // The second element with the same style attribute refers to the stored
    // block instead of parsing it, and still counts each declaration.
    let (doc, tree) = small_doc("", |doc, body| {
        for _ in 0..2 {
            doc.push_element(body, "p", Some("color: red; color: blue"));
        }
    });
    let limits = CascadeLimits {
        max_declarations_visited: Some(3),
        ..unlimited()
    };
    assert_eq!(
        exceeded(walk_within(&doc, &tree, limits, false)),
        (CascadeLimitKind::DeclarationsVisited, 3, 4)
    );
}

#[test]
fn every_selector_tested_counts_whether_it_matches_or_not() {
    let selector_limit = |count| CascadeLimits {
        max_selector_tests: Some(count),
        ..unlimited()
    };
    // One rule with two selectors, one of them for a pseudo-element: both
    // are tested for the element and again for its pseudo-elements.
    let (doc, tree) = small_doc("p, div::before { color: red }", |doc, body| {
        doc.push_element(body, "p", None);
    });
    assert_eq!(
        exceeded(walk_within(&doc, &tree, selector_limit(3), true)),
        (CascadeLimitKind::SelectorTests, 3, 4)
    );
    // A rule no element matches costs a test per element it is tried on.
    let (doc, tree) = small_doc("[data-none] { color: red }", |doc, body| {
        for _ in 0..3 {
            doc.push_element(body, "div", None);
        }
    });
    assert_eq!(
        exceeded(walk_within(&doc, &tree, selector_limit(2), true)),
        (CascadeLimitKind::SelectorTests, 2, 3)
    );
}

fn output_limit(bytes: u64) -> CascadeLimits {
    CascadeLimits {
        max_output_bytes: Some(bytes),
        ..unlimited()
    }
}

#[test]
fn the_per_node_values_are_counted_before_they_are_allocated() {
    let (doc, tree) = small_doc("", |doc, body| {
        doc.push_element(body, "p", None);
    });
    let per_node = (doc.node_count() * NODE_OUTPUT_BYTES) as u64;
    assert_eq!(
        exceeded(walk_within(&doc, &tree, output_limit(per_node - 1), true)),
        (CascadeLimitKind::OutputBytes, per_node - 1, per_node)
    );
    let counts = walk_within(&doc, &tree, output_limit(per_node), true)
        .expect("the per-node values fit")
        .counts;
    assert_eq!(counts.output_bytes, per_node);
}

/// Asserts that `doc` fails when its output limit is one byte short of what
/// it needs, with every node visited `sibling_sharing` decides.
fn assert_last_output_fails(doc: &TestDoc, tree: &RuleTree, sibling_sharing: bool) {
    let needed = walk_within(doc, tree, unlimited(), sibling_sharing)
        .expect("the walk succeeds")
        .counts
        .output_bytes;
    let (kind, _, actual) = exceeded(walk_within(
        doc,
        tree,
        output_limit(needed - 1),
        sibling_sharing,
    ));
    assert_eq!((kind, actual), (CascadeLimitKind::OutputBytes, needed));
}

#[test]
fn pseudo_element_values_and_svg_properties_count_as_output() {
    // The last output of each document: a pseudo-element value, the copy of
    // one for a sharing sibling, and an SVG element's paint properties.
    let resolved = small_doc("p::before { content: 'x' }", |doc, body| {
        doc.push_element(body, "p", None);
    });
    let copied = small_doc("p::before { content: 'x' }", |doc, body| {
        for _ in 0..2 {
            doc.push_element(body, "p", None);
        }
    });
    let svg = small_doc("rect { opacity: .25 }", |doc, body| {
        let svg = doc.push_element_with_namespace(body, "svg", SVG, &[]);
        doc.push_element_with_namespace(svg, "rect", SVG, &[]);
    });
    assert_last_output_fails(&resolved.0, &resolved.1, true);
    assert_last_output_fails(&copied.0, &copied.1, true);
    assert!(
        walk_within(&copied.0, &copied.1, unlimited(), true)
            .expect("the walk succeeds")
            .shared_nodes
            > 0
    );
    assert_last_output_fails(&svg.0, &svg.1, true);
}

fn retained_limit(bytes: u64) -> CascadeLimits {
    CascadeLimits {
        max_retained_bytes: Some(bytes),
        ..unlimited()
    }
}

#[test]
fn kept_candidates_count_as_retained() {
    // First-letter candidates of an element, and the copy of them a sharing
    // sibling makes.
    let (doc, tree) = small_doc("p::first-letter { color: red }", |doc, body| {
        for _ in 0..2 {
            let p = doc.push_element(body, "p", None);
            doc.push_text(p, "text");
        }
    });
    let walked = walk_within(&doc, &tree, unlimited(), true).expect("the walk succeeds");
    assert!(walked.shared_nodes > 0);
    let needed = walked.counts.retained_bytes;
    let one = needed / 2;
    assert_eq!(
        exceeded(walk_within(&doc, &tree, retained_limit(one - 1), true)),
        (CascadeLimitKind::RetainedBytes, one - 1, one)
    );
    assert_eq!(
        exceeded(walk_within(&doc, &tree, retained_limit(needed - 1), true)),
        (CascadeLimitKind::RetainedBytes, needed - 1, needed)
    );
    // Candidates under an element with a first-line style, kept because some
    // rule targets `::first-letter`.
    let (doc, tree) = small_doc(
        "p::first-line { color: red } span::first-letter { color: blue }",
        |doc, body| {
            let p = doc.push_element(body, "p", None);
            doc.push_text(p, "text");
        },
    );
    assert_eq!(
        exceeded(walk_within(&doc, &tree, retained_limit(0), true)).0,
        CascadeLimitKind::RetainedBytes
    );
}

#[test]
fn the_first_line_subtree_counts_as_retained() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "p::first-line { color: red }");
    let p = doc.push_element(0, "p", Some("display: block"));
    doc.push_text(p, "text ");
    let span = doc.push_element(p, "span", Some("color: blue"));
    doc.push_text(span, "more");
    let tree = build_rule_tree(&doc);
    let root = StyleNodeId(p as u64);
    let media = MediaContext::default();
    let first_line = cascade_with_first_line_within(&doc, &tree, &media, root, unlimited())
        .expect("the first-line cascade succeeds");
    assert!(first_line.first_line.is_some());
    assert_eq!(
        exceeded(cascade_with_first_line_within(
            &doc,
            &tree,
            &media,
            root,
            retained_limit(0)
        ))
        .0,
        CascadeLimitKind::RetainedBytes
    );
    // The first-line styles count as output after what the walk holds.
    let options = WalkOptions {
        retain_subtree: Some(root),
        limits: unlimited(),
        ..WalkOptions::default()
    };
    let walked = walk(&doc, &tree, &media, options).expect("the walk succeeds");
    let walk_bytes = walked.counts.output_bytes;
    let first_line_bytes =
        (doc.node_count() * std::mem::size_of::<Option<ComputedValues>>()) as u64;
    assert_eq!(
        exceeded(cascade_with_first_line_within(
            &doc,
            &tree,
            &media,
            root,
            output_limit(walk_bytes)
        )),
        (
            CascadeLimitKind::OutputBytes,
            walk_bytes,
            walk_bytes + first_line_bytes
        )
    );
    assert!(
        cascade_with_first_line_within(
            &doc,
            &tree,
            &media,
            root,
            output_limit(walk_bytes + first_line_bytes)
        )
        .is_ok()
    );
}

#[test]
fn the_public_entry_point_reports_a_passed_limit() {
    let (doc, tree) = small_doc("p { color: red; color: blue }", |doc, body| {
        doc.push_element(body, "p", None);
    });
    let mut options = CascadeOptions::default();
    options.limits.max_candidates_per_element = Some(1);
    let error = cascade_with_options(
        &doc,
        &tree,
        &MediaContext::default(),
        &PageContextQuery::default(),
        &options,
    )
    .expect_err("one candidate is too few");
    assert_eq!(
        error.to_string(),
        "CSS cascade limit exceeded: CandidatesPerElement (limit=1, actual=2)"
    );
    options.limits.max_candidates_per_element = Some(2);
    let result = cascade_with_options(
        &doc,
        &tree,
        &MediaContext::default(),
        &PageContextQuery::default(),
        &options,
    )
    .expect("two candidates fit");
    assert_eq!(result.computed.len(), doc.node_count());
}

fn output_bytes(doc: &TestDoc, tree: &RuleTree, sibling_sharing: bool) -> u64 {
    walk_within(doc, tree, unlimited(), sibling_sharing)
        .expect("the walk succeeds")
        .counts
        .output_bytes
}

fn paragraphs(css: &str, inline: impl Fn(usize) -> Option<String>) -> (TestDoc, RuleTree) {
    small_doc(css, |doc, body| {
        for i in 0..4 {
            doc.push_element(body, "p", inline(i).as_deref());
        }
    })
}

#[test]
fn values_a_node_holds_of_its_own_count_as_output() {
    let base = paragraphs("", |_| None);
    let base = output_bytes(&base.0, &base.1, true);
    // Each element resolves its own `var()` substitution: a custom property
    // on each keeps the elements from sharing.
    let big = "x".repeat(1024);
    let substituted = paragraphs(
        &format!(":root {{ --b: {big} }} p {{ --a: var(--b) var(--b) }}"),
        |i| Some(format!("--u: {i}")),
    );
    // A long declared list, which sharing siblings copy.
    let shadows = vec!["1px 1px red"; 100].join(",");
    let listed = paragraphs(&format!("p {{ box-shadow: {shadows} }}"), |_| None);
    let shadow_list = 100 * std::mem::size_of::<crate::resolve::ComputedBoxShadowItem>() as u64;
    for ((doc, tree), at_least) in [(&substituted, 4 * 2 * 1024), (&listed, 4 * shadow_list)] {
        let shared = output_bytes(doc, tree, true);
        assert_eq!(
            shared,
            output_bytes(doc, tree, false),
            "sharing changed the count"
        );
        assert!(shared >= base + at_least, "{shared} < {base} + {at_least}");
        assert_last_output_fails(doc, tree, true);
        assert_last_output_fails(doc, tree, false);
    }
}

#[test]
fn inherited_values_count_once() {
    // The paragraphs share the body's text-shadow list, which counts on the
    // body alone.
    let base = paragraphs("", |_| None);
    let shadows = vec!["1px 1px red"; 100].join(",");
    let inherited = paragraphs(&format!("body {{ text-shadow: {shadows} }}"), |_| None);
    assert_eq!(
        output_bytes(&inherited.0, &inherited.1, true) - output_bytes(&base.0, &base.1, true),
        100 * std::mem::size_of::<crate::resolve::ComputedTextShadow>() as u64
    );
}
