use super::*;
use taffy::Style;

#[test]
fn multicol_min_constrained_child_requires_all_constraints() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    doc.nodes[child].has_logical_min_block_size = true;
    doc.nodes[child].style.min_size.height = LengthPercentageAuto::length(40.0);

    assert!(multicol_has_min_constrained_child(&doc, parent));

    doc.nodes[child].style.min_size.height = LengthPercentageAuto::auto();
    assert!(!multicol_has_min_constrained_child(&doc, parent));
}

#[test]
fn compute_multicol_layout_spaces_break_avoid_min_height_children_across_a_definite_height() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px;height:60px"),
    );
    // `min-block-size` + `break-inside:avoid` (no direct `<br>`) keeps
    // this multicol container on the foundational block projection
    // instead of the custom nested-fragmentation path (see the doc
    // comment above `compute_multicol_layout`'s `custom_scope`).
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // Taffy's foundational block algorithm cannot express a
    // fragmentainer break here, so each break-avoid min-height child is
    // pushed into its own vertical slot: `child_height + 2 *
    // |fragmentainer_height - child_height|` apart, consuming the
    // unfragmented overflow on both sides of the mismatch instead of
    // packing contiguously.
    let child_height = doc.nodes[a].unrounded_layout.size.height;
    assert!(child_height > 0.0, "child_height={child_height}");
    let step = child_height + 2.0 * (60.0_f32 - child_height).abs();
    assert!((doc.nodes[a].unrounded_layout.location.y - 0.0).abs() < 0.01);
    assert!(
        (doc.nodes[b].unrounded_layout.location.y - step).abs() < 0.01,
        "b.y={}, expected step={step}",
        doc.nodes[b].unrounded_layout.location.y
    );
}

#[test]
fn compute_multicol_layout_clamps_auto_height_to_the_largest_min_constrained_child() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    let _b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 400.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // An ordinary auto-height block container would size to the *sum*
    // of its two 40px min-height children (80px). A multicol container
    // with min-constrained children instead clamps its own auto used
    // height to the largest child's used height: the children overflow
    // the box rather than growing it (see the comment above this
    // clamp in `compute_multicol_layout`).
    let child_height = doc.nodes[a].unrounded_layout.size.height;
    assert!(child_height > 0.0, "child_height={child_height}");
    assert!(
        (doc.nodes[container].unrounded_layout.size.height - child_height).abs() < 0.01,
        "container height={}, expected clamp to child height={child_height}",
        doc.nodes[container].unrounded_layout.size.height,
    );
}

#[test]
fn multicol_definite_dimension_resolves_absolute_length_regardless_of_basis() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(120.5), None),
        Some(120.5)
    );
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(120.5), Some(9999.0)),
        Some(120.5)
    );
}

#[test]
fn multicol_definite_dimension_resolves_percent_against_the_given_basis() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.25), Some(200.0)),
        Some(50.0)
    );
}

#[test]
fn multicol_definite_dimension_resolves_percent_against_a_zero_default_basis() {
    // `basis` defaults to 0.0 when the caller has no known containing
    // block size yet, so a percentage resolves to zero rather than
    // `None`.
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.5), None),
        Some(0.0)
    );
}

#[test]
fn multicol_definite_dimension_returns_none_for_intrinsic_sizing_keywords() {
    // Every non-definite keyword collapses to `None`: the caller falls
    // back to Taffy's ordinary intrinsic-sizing pass instead of a used
    // length.
    let doc = Document::new();
    for dimension in [
        Dimension::auto(),
        Dimension::min_content(),
        Dimension::max_content(),
        Dimension::fit_content(),
        Dimension::fit_content_px(40.0),
        Dimension::fit_content_percent(0.5),
        Dimension::stretch(),
        Dimension::content(),
    ] {
        assert_eq!(
            multicol_definite_dimension(&doc, dimension, Some(200.0)),
            None
        );
    }
}

#[test]
fn multicol_definite_dimension_clamps_a_negative_length_to_zero() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(-25.0), None),
        Some(0.0)
    );
}

#[test]
fn multicol_definite_dimension_returns_none_for_a_non_finite_result() {
    // An infinite basis makes the resolved percentage non-finite; the
    // `is_finite()` guard turns that into `None` rather than an
    // infinite used size.
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.5), Some(f32::INFINITY)),
        None
    );
}

/// Build a `<p>` with five pre-line-separated single-character lines at
/// an explicit 10px line-height, so each line's block extent is an exact
/// multiple of 10px regardless of the font actually resolved.
fn nested_text_line_ranges_fixture() -> (Document, usize) {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("font-size:10px;line-height:10px;white-space:pre-line"),
    );
    let text = doc.append_text(p, "a\nb\nc\nd\ne");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, 1000.0, 1000.0);
    (doc, text)
}

#[test]
fn nested_text_line_ranges_returns_empty_when_past_the_last_column() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 100.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 2,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    assert!(nested_text_line_ranges(layout, context).is_empty());
}

#[test]
fn nested_text_line_ranges_splits_by_available_height_across_three_columns() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    assert_eq!(
        layout.len(),
        5,
        "sanity: five pre-line segments produce five lines"
    );
    // Each line's bottom edge advances by exactly one 10px line-height
    // step, which is what the height-fit check below relies on. (The
    // top edge is not asserted here: it can sit slightly outside the
    // nominal line box when a line's ascent/descent exceeds the
    // explicit `line-height`, which does not affect the block-max-based
    // fit check.)
    let first_max = layout.lines().next().unwrap().metrics().block_max_coord;
    for (i, line) in layout.lines().enumerate() {
        let metrics = line.metrics();
        assert!((metrics.block_max_coord - (first_max + i as f32 * 10.0)).abs() < 0.01);
    }
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: Some(25.0),
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 2, 0), (2, 4, 1), (4, 5, 2)]);
}

#[test]
fn nested_text_line_ranges_splits_evenly_when_height_is_unconstrained() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 0), (3, 5, 1)]);
}

#[test]
fn nested_text_line_ranges_labels_fragments_with_the_starting_column_offset() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: None,
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 1,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 1), (3, 5, 2)]);
}

#[test]
fn nested_text_line_ranges_moves_lines_across_the_boundary_to_satisfy_widows() {
    // CSS Fragmentation Module Level 3 §3.3: a widows:3 minimum on the
    // final fragment borrows lines from the preceding fragment, bounded
    // by the preceding fragment's own orphans minimum.
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 3,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 2, 0), (2, 5, 1)]);
}

#[test]
fn nested_text_line_ranges_keeps_the_split_when_orphans_forbids_the_widows_move() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 3,
        widows: 3,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 0), (3, 5, 1)]);
}

#[test]
fn nested_text_line_ranges_treats_a_zero_height_budget_like_unconstrained() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: Some(0.0),
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 0), (3, 5, 1)]);
}

#[test]
fn refresh_nested_text_fragments_records_line_ranges_and_column_offsets_on_a_text_node() {
    let (mut doc, text) = nested_text_line_ranges_fixture();
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: None,
        column_width: 100.0,
        column_count: 3,
        column_gap: 10.0,
        column_index: 1,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    refresh_nested_text_fragments(&mut doc, text, context);

    let fragments = doc.nodes[text]
        .multicol_fragments()
        .expect("a shaped 5-line text node should get line-range fragments")
        .to_vec();
    assert_eq!(fragments.len(), 2);
    assert_eq!((fragments[0].line_start, fragments[0].line_end), (0, 3));
    assert_eq!((fragments[1].line_start, fragments[1].line_end), (3, 5));
    // The first fragment starts in the context's own column (index 1),
    // so its offset from that column's origin is zero; the second
    // fragment sits in the next column (index 2), one
    // column-width-plus-gap further.
    assert!((fragments[0].x - 0.0).abs() < 0.01);
    assert!((fragments[1].x - 110.0).abs() < 0.01);
    // Each line uses an explicit 10px line-height (see the fixture's
    // own doc comment), so the second fragment's line-3 origin sits
    // exactly three lines (30px) below the first fragment's line-0
    // origin.
    assert!(
        (fragments[1].y - fragments[0].y - 30.0).abs() < 0.01,
        "fragments={fragments:?}"
    );
}

#[test]
fn refresh_nested_text_fragments_recurses_into_element_children() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("font-size:10px;line-height:10px;white-space:pre-line"),
    );
    let text = doc.append_text(p, "a\nb\nc\nd\ne");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, 1000.0, 1000.0);

    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    // Called on the *element*, not the text node directly: the
    // recursive descent (the non-text arm) must reach the shaped text
    // descendant and populate its fragments exactly as a direct call
    // on the text node would.
    refresh_nested_text_fragments(&mut doc, p, context);

    let fragments = doc.nodes[text]
        .multicol_fragments()
        .expect("recursing through the <p> element should still reach its text child");
    assert_eq!(fragments.len(), 2);
    assert_eq!((fragments[0].line_start, fragments[0].line_end), (0, 3));
    assert_eq!((fragments[1].line_start, fragments[1].line_end), (3, 5));
}

#[test]
fn refresh_nested_text_fragments_leaves_an_unshaped_text_node_untouched() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let text = doc.append_text(p, "no preshape ran on this node");

    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    // No `preshape_text` call precedes this, so `text_layout` is still
    // `None`: the function must return without touching
    // `multicol_fragments`.
    refresh_nested_text_fragments(&mut doc, text, context);
    assert!(doc.nodes[text].multicol_fragments().is_none());
}

#[test]
fn refresh_nested_text_fragments_clears_stale_fragments_for_a_zero_line_layout() {
    use parley::{FontContext, LayoutContext};

    // A hand-built, never-`break_all_lines`-run layout reports zero
    // lines, giving a direct unit fixture for the function's own
    // `line_count == 0` guard without depending on any particular
    // content producing it through the real `preshape_text` pipeline
    // (an empty string there still shapes to one empty line).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let text = doc.append_text(p, "");

    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    let mut builder = layout_cx.ranged_builder(&mut fonts, "", 1.0, false);
    builder.push_default(parley::StyleProperty::FontSize(16.0));
    let empty_layout: parley::Layout<()> = builder.build("");
    assert_eq!(
        empty_layout.len(),
        0,
        "sanity: an unbroken layout has zero lines"
    );

    let NodeData::Text(text_data) = &mut doc.nodes[text].data else {
        panic!("expected a text node");
    };
    text_data.text_layout = Some(empty_layout);
    // Stale fragments left over from a previous (non-empty) layout pass.
    text_data.multicol_fragments = Some(vec![MulticolTextFragment {
        line_start: 0,
        line_end: 1,
        x: 5.0,
        y: 5.0,
    }]);

    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    refresh_nested_text_fragments(&mut doc, text, context);
    assert!(
        doc.nodes[text].multicol_fragments().is_none(),
        "a zero-line layout must clear any stale fragments rather than keep them"
    );
}

#[test]
fn relayout_nested_multicol_children_uses_the_definite_height_budget_per_child() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    // The two tests above give the multicol container an auto block
    // size, which routes every child through the auto-measured
    // balancing pass. A *definite* container height instead skips that
    // pass (`auto_measurements` stays `None`) and lays each child out
    // against its own remaining per-column height budget directly. A
    // direct `<br>` child is what forces `compute_multicol_layout`'s
    // custom nested path even though the height is definite (see the
    // doc comment above its `custom_scope` computation).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px;height:40px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let _br = doc.append_element(Some(a), "br", Style::default(), None::<&str>);
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    // A direct text child (not wrapped in its own block box) exercises
    // the text-specific reset (`style.size.height = auto` + cache
    // clear) and the per-child multicol text-fragment recording, both
    // only reachable on this non-auto-measured child path.
    let text = doc.append_text(container, "hello column text");

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // `a` (30px) fits the 40px column budget; `b` (30px) does not
    // (30 + 30 > 40), so it starts a fresh column at y=0.
    assert!((doc.nodes[a].unrounded_layout.location.y - 0.0).abs() < 0.01);
    assert!((doc.nodes[b].unrounded_layout.location.y - 0.0).abs() < 0.01);
    // width 200 / 2 columns with a 20px gap: (200 - 20) / 2 = 90;
    // column 1 starts at 90 + 20 = 110.
    assert!((doc.nodes[a].unrounded_layout.location.x - 0.0).abs() < 0.01);
    assert!((doc.nodes[b].unrounded_layout.location.x - 110.0).abs() < 0.01);

    assert!(
        doc.nodes[text].multicol_fragments().is_some(),
        "direct multicol text should get explicit line-range fragments"
    );
}

#[test]
fn relayout_nested_multicol_children_column_places_a_block_sibling_of_an_empty_direct_text_child() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    // An *auto*-height multicol container (unlike the definite-height
    // test above) routes every child through the auto-measurement pass
    // first (`auto_measurements` is `Some` here), which has its own
    // copy of the "reset a direct text child's style before measuring"
    // branch. A direct text sibling with real content would make
    // `prepare_multicol_layout`'s pre-pass pin the container's own
    // height to a definite value (its line-count projection), which
    // would route this container through the definite-height path
    // instead and defeat the point of this test; an empty text node
    // never gets a shaped layout, so that pre-pass leaves the
    // container's auto height alone while the node still participates
    // in `relayout_nested_multicol_children`'s child list.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let _text = doc.append_text(container, "");

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // `a`'s width narrowing from the container's full 200px to one
    // 90px column ((200 - 20) / 2) is only possible through the custom
    // nested-fragmentation path -- the ordinary Taffy block algorithm
    // would stretch it to the full content width instead. That path
    // only runs by measuring every child in `children`, the empty
    // text node included, so this indirectly exercises its branch.
    assert!((doc.nodes[a].unrounded_layout.size.width - 90.0).abs() < 0.01);
    assert!((doc.nodes[container].unrounded_layout.size.height - 30.0).abs() < 0.01);
}
