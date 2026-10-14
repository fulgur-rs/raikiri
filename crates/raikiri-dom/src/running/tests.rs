use super::*;

use smol_str::SmolStr;
use taffy::Style;

#[test]
fn element_string_value_resolves_bare_content_to_element_text_value() {
    // GCPM 3 §1.1.1.1: content() / content(text) = "the string value of
    // the element, determined as if white-space: normal had been set".
    let mut doc = Document::new();
    let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
    doc.append_text(h1, "  Loomings   Ch.  1  ");

    let value = element_string_value(&doc, h1);
    assert_eq!(value, "Loomings Ch. 1");
}

#[test]
fn element_string_value_content_text_preserves_no_break_space() {
    // CSS Text 3 §4.1 scopes white-space:normal collapsing to space /
    // tab / segment-break, explicitly excluding U+00A0 NO-BREAK SPACE —
    // regression check against using str::split_whitespace (Unicode
    // White_Space, which would wrongly collapse/trim it too).
    let mut doc = Document::new();
    let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
    doc.append_text(h1, "A\u{A0}\u{A0}B");

    let value = element_string_value(&doc, h1);
    assert_eq!(value, "A\u{A0}\u{A0}B");
}

#[test]
fn element_string_value_content_text_collapses_tab_lf_and_cr() {
    // CSS Text 3 §4.1: tab (U+0009) and segment breaks (line feed
    // U+000A, per §4's HTML-DOM rule) are document white space and must
    // collapse like ordinary spaces; §4 additionally states carriage
    // return (U+000D) is "treated identically to spaces ... in all
    // respects". Pins that is_css_document_white_space still collapses
    // these three after narrowing away from split_ascii_whitespace's
    // ASCII set (which also collapsed them, plus form feed).
    let mut doc = Document::new();
    let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
    doc.append_text(h1, "A\t\t\u{A}\u{A}\u{D}\u{D}B");

    let value = element_string_value(&doc, h1);
    assert_eq!(value, "A B");
}

#[test]
fn element_string_value_content_text_preserves_form_feed() {
    // CSS Text 3 §4 <https://www.w3.org/TR/css-text-3/#white-space-processing>:
    // control characters other than tab/LF/CR/segment-break — form feed
    // (U+000C) included — must render as a visible glyph, not collapse
    // or trim away. Regression check against str::split_ascii_whitespace,
    // whose Rust-defined ASCII whitespace set wrongly includes U+000C.
    let mut doc = Document::new();
    let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
    doc.append_text(h1, "A\u{C}\u{C}B");

    let value = element_string_value(&doc, h1);
    assert_eq!(value, "A\u{C}\u{C}B");
}

#[test]
fn element_string_value_content_text_concatenates_across_descendant_elements() {
    // "String value of the element" reaches through descendant
    // elements, not just direct-child text nodes.
    let mut doc = Document::new();
    let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
    let em = doc.append_element(Some(h1), "em", Style::default(), None::<&str>);
    doc.append_text(em, "Moby");
    doc.append_text(h1, " Dick");

    let value = element_string_value(&doc, h1);
    assert_eq!(value, "Moby Dick");
}

#[test]
fn element_string_value_content_text_no_stack_overflow_on_deep_dom_and_preserves_order() {
    // collect_descendant_text (element_string_value's DFS
    // accumulator) must be iterative, not recursive, so a deeply
    // nested DOM cannot exhaust the call stack — same deep-DOM
    // regression shape as
    // crate::layout::find_body_iterative_no_stack_overflow_on_deep_dom
    // and raikiri-html's parse_survives_deeply_nested_html, both of
    // which use 5000 levels. That depth was measured (locally,
    // reverting this function to its old recursive form) to *not*
    // overflow the default ~2 MiB test-thread stack for this specific
    // function — its per-frame state is small (an index, two
    // references) compared to those other walkers' recursive frames —
    // so it would not have caught the bug this test exists to check.
    // 200_000 was measured to reliably overflow the old recursive
    // form under the same default stack, giving roughly a 10x margin
    // over the empirically observed ~15_000-20_000 overflow threshold.
    //
    // Each level contributes both a text child ("t{i} ") *and* the
    // next nested element, so document order only comes out right if
    // the stack-based walk pushes children in reverse (popping them
    // back out in original order) rather than, say, descending into
    // the nested child before visiting that level's own text — a
    // single-child chain wouldn't exercise that ordering at all.
    let mut doc = Document::new();
    let mut parent = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
    let root = parent;
    let depth = 200_000;
    for i in 0..depth {
        doc.append_text(parent, format!("t{i} "));
        parent = doc.append_element(Some(parent), "span", Style::default(), None::<&str>);
    }
    // Built independently of collect_descendant_text's own
    // whitespace-collapse pipeline, so this isn't just checking the
    // implementation against itself.
    let expected = (0..depth)
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(" ");

    let value = element_string_value(&doc, root);
    assert_eq!(value, expected);
}

#[test]
fn element_string_value_content_text_skips_template_descendants() {
    // collect_descendant_text's `is_in_document()` check must skip a
    // `<template>` element's detached contents fragment: the fragment is
    // unreachable from the Document root (the parser's shape), so the
    // walk never descends into it and the "hidden" text never leaks into
    // the resolved string. An ordinary light-DOM child appended directly
    // under the template element stays in-document instead, so the probe
    // `<p>` is placed in the contents fragment here.
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
    doc.append_text(root, "before ");
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    let frag = doc.allocate_template_fragment_root(tmpl);
    let inner = doc.append_element(Some(frag), "p", Style::default(), None::<&str>);
    doc.append_text(inner, "hidden");
    doc.append_text(root, " after");

    doc.mark_in_document_flags();
    assert!(doc.get_node(tmpl).unwrap().is_in_document());
    assert!(!doc.get_node(inner).unwrap().is_in_document());

    let value = element_string_value(&doc, root);
    assert_eq!(value, "before after");
}

fn index(placements: &[(u64, u32, bool)]) -> RunningIndex {
    let placements: Vec<_> = placements
        .iter()
        .map(|&(node, page, starts_page)| Placement {
            node: NodeId(node),
            page,
            starts_page,
        })
        .collect();
    let by_page = placements.is_sorted_by_key(|placement| placement.page);
    let mut index = RunningIndex::default();
    index.pools.insert(
        SmolStr::new("hdr"),
        Pool {
            placements,
            by_page,
        },
    );
    index
}

const ALL: [StringFetchMode; 4] = [
    StringFetchMode::First,
    StringFetchMode::Start,
    StringFetchMode::Last,
    StringFetchMode::FirstExcept,
];

fn selections(index: &RunningIndex, page: u32) -> Vec<Option<u64>> {
    ALL.iter()
        .map(|&fetch| index.select("hdr", fetch, page).map(|node| node.0))
        .collect()
}

#[test]
fn a_page_with_no_assignment_shows_the_entry_value_for_every_keyword() {
    let index = index(&[(1, 0, true), (2, 0, false)]);
    // Page 1 has no assignment: every keyword falls back to the last
    // element assigned on an earlier page.
    assert_eq!(selections(&index, 1), vec![Some(2); 4]);
}

#[test]
fn keywords_pick_from_the_assignments_on_the_page() {
    let index = index(&[(1, 0, true), (2, 1, false), (3, 1, false)]);
    // first, start (2 does not start page 1), last, first-except.
    assert_eq!(selections(&index, 1), vec![Some(2), Some(1), Some(3), None]);
}

#[test]
fn start_takes_an_assignment_that_starts_the_page() {
    let index = index(&[(1, 0, true), (2, 1, true), (3, 1, false)]);
    assert_eq!(
        index.select("hdr", StringFetchMode::Start, 1),
        Some(NodeId(2))
    );
}

#[test]
fn nothing_applies_before_the_first_assignment() {
    let index = index(&[(1, 2, true)]);
    assert_eq!(selections(&index, 0), vec![None; 4]);
    assert_eq!(selections(&index, 2), vec![Some(1), Some(1), Some(1), None]);
    assert_eq!(selections(&index, 3), vec![Some(1); 4]);
}

#[test]
fn unknown_names_select_nothing() {
    let index = index(&[(1, 0, true)]);
    assert_eq!(index.select("ftr", StringFetchMode::First, 0), None);
}

#[test]
fn page_search_matches_a_scan_of_the_pool() {
    let sorted = index(&[
        (1, 0, true),
        (2, 2, true),
        (3, 2, false),
        (4, 2, false),
        (5, 5, true),
    ]);
    assert!(sorted.pools["hdr"].by_page);
    let mut scanned = index(&[
        (1, 0, true),
        (2, 2, true),
        (3, 2, false),
        (4, 2, false),
        (5, 5, true),
    ]);
    scanned.pools.get_mut("hdr").expect("pool").by_page = false;
    for page in 0..7 {
        assert_eq!(selections(&sorted, page), selections(&scanned, page));
    }
    // A pool out of page order is scanned.
    let unsorted = index(&[(1, 3, true), (2, 1, true), (3, 3, false)]);
    assert!(!unsorted.pools["hdr"].by_page);
    assert_eq!(
        selections(&unsorted, 3),
        vec![Some(1), Some(1), Some(3), None]
    );
    assert_eq!(selections(&unsorted, 2), vec![Some(2); 4]);
}

/// `body > p, hdr (running), p, hdr (running)`, with the cascade and the
/// node ids of the body, the paragraphs and the running elements.
fn running_document() -> (Document, CascadeResult, [usize; 5]) {
    let mut doc = Document::new();
    let style = doc.append_element(Some(0), "style", Style::default(), None::<&str>);
    doc.append_text(style, ".hdr { position: running(hdr) }");
    let body = doc.append_element(Some(0), "body", Style::default(), None::<&str>);
    let first = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let head = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    doc.set_element_attributes(head, vec![(SmolStr::new("class"), SmolStr::new("hdr"))]);
    doc.append_text(head, "Head");
    let second = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let tail = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    doc.set_element_attributes(tail, vec![(SmolStr::new("class"), SmolStr::new("hdr"))]);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
    (doc, cascade, [body, first, head, second, tail])
}

#[test]
fn running_elements_take_the_page_of_the_content_around_them() {
    let (doc, cascade, [_, first, head, second, tail]) = running_document();
    let rendered = HashMap::from([(first, (0, 0)), (second, (1, 2))]);
    let index = RunningIndex::from_rendered_pages(&doc, &cascade, &rendered);
    // `head` takes the first page of the paragraph after it and starts that
    // page; `tail`, with nothing after it, takes the last page of the
    // paragraph before it, which precedes it on that page.
    assert_eq!(
        index.pools["hdr"].placements,
        [
            Placement {
                node: NodeId(head as u64),
                page: 1,
                starts_page: true,
            },
            Placement {
                node: NodeId(tail as u64),
                page: 2,
                starts_page: false,
            },
        ]
    );
    let mut nodes: Vec<_> = index.nodes().map(|node| node.0 as usize).collect();
    nodes.sort_unstable();
    assert_eq!(nodes, [head, tail]);
}

#[test]
fn a_document_without_running_elements_has_an_empty_index() {
    let mut doc = Document::new();
    doc.append_element(Some(0), "body", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
    let index = RunningIndex::build(&doc, &cascade, [(&doc, 0)]);
    assert_eq!(index.nodes().count(), 0);
}

#[test]
fn a_running_element_lays_out_alone_at_the_given_width() {
    let (doc, cascade, [body, first, head, ..]) = running_document();
    assert!(
        layout_running_element(&doc, &cascade, NodeId(first as u64), 100.0)
            .expect("layout")
            .is_none(),
        "only running elements are laid out"
    );
    let laid_out = layout_running_element(&doc, &cascade, NodeId(head as u64), 100.0)
        .expect("layout")
        .expect("running element");
    assert_eq!(laid_out.geometry.page_box.width, 100.0);
    assert_eq!(laid_out.geometry.page_box.height, laid_out.height);
    // The copy holds the element's ancestors, then the element and its text.
    assert_eq!(laid_out.source_nodes, [0, body, head, head + 1]);
    assert_eq!(laid_out.document.node_count(), laid_out.source_nodes.len());
}

#[test]
fn a_node_outside_the_document_has_an_empty_string_value() {
    let doc = Document::new();
    assert_eq!(element_string_value(&doc, 9999), "");
}

#[test]
fn negative_and_non_finite_widths_lay_out_at_zero() {
    for width in [-10.0, f32::NAN, f32::INFINITY] {
        assert_eq!(used_running_element_width(width), 0.0);
    }
    assert_eq!(used_running_element_width(12.5), 12.5);
    let (doc, cascade, [_, _, head, ..]) = running_document();
    let laid_out = layout_running_element(&doc, &cascade, NodeId(head as u64), f32::NAN)
        .expect("layout")
        .expect("running element");
    assert_eq!(laid_out.geometry.page_box.width, 0.0);
}

#[test]
fn a_running_element_too_deep_to_lay_out_is_an_error() {
    let (mut doc, _, [_, _, head, ..]) = running_document();
    let mut parent = head;
    for _ in 0..=crate::layout::MAX_LAYOUT_DEPTH {
        parent = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
    let result = layout_running_element(&doc, &cascade, NodeId(head as u64), 100.0);
    assert!(matches!(
        result,
        Err(LayoutError::TreeDepthLimitExceeded { .. })
    ));
}
