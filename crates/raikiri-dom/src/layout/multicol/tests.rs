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
fn authored_containing_width_uses_the_resolved_ch_style_width() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:4ch;font-size:25px;font-family:monospace"),
    );
    let text = doc.append_text(container, "test");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("styles");

    doc.set_font_collection(crate::fonts::system_font_collection());
    prepare_ch_box_values_before_taffy(&mut doc, &cascade);

    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            parent_of[child] = Some(parent);
        }
    }
    let taffy_width = style_dimension_length(doc.nodes[container].style.size.width)
        .expect("resolved ch width should be a finite length");
    assert_eq!(
        authored_containing_width_with_resolved_ch(&doc, &cascade, &parent_of, text, 800.0),
        Some(taffy_width),
    );
}

#[test]
fn authored_containing_width_preserves_percentage_fallback_behavior() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:200px"),
    );
    let percentage = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;width:50%"),
    );
    let text = doc.append_text(percentage, "percentage");
    let auto_text = doc.append_text(body, "auto");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            parent_of[child] = Some(parent);
        }
    }
    assert_eq!(
        authored_containing_width_with_resolved_ch(&doc, &cascade, &parent_of, text, 800.0),
        Some(100.0),
    );
    assert_eq!(
        authored_containing_width_with_resolved_ch(&doc, &cascade, &parent_of, auto_text, 800.0,),
        None,
    );
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
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

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
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

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
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // `a`'s width narrowing from the container's full 200px to one
    // 90px column ((200 - 20) / 2) is only possible through the custom
    // nested-fragmentation path -- the ordinary Taffy block algorithm
    // would stretch it to the full content width instead. That path
    // only runs by measuring every child in `children`, the empty
    // text node included, so this indirectly exercises its branch.
    assert!((doc.nodes[a].unrounded_layout.size.width - 90.0).abs() < 0.01);
    assert!((doc.nodes[container].unrounded_layout.size.height - 30.0).abs() < 0.01);
}

#[test]
fn vertical_fallback_multicol_preserves_logical_minimum_block_extent() {
    use raikiri_style::{build_rule_tree, cascade};

    for writing_mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for (container_size, expected_height, second_y) in [
            ("", 40.0, 40.0),
            ("block-size:30px", 30.0, 60.0),
            ("block-size:60px", 60.0, 80.0),
        ] {
            let mut doc = Document::new();
            let html = doc.append_element(
                Some(0),
                "html",
                Style::default(),
                Some(format!("writing-mode:{writing_mode}")),
            );
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            let container = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some(format!(
                    "display:block;column-count:3;inline-size:500px;{container_size}"
                )),
            );
            let children: Vec<_> = (0..2)
                .map(|_| {
                    doc.append_element(
                        Some(container),
                        "div",
                        Style::default(),
                        Some("display:block;min-block-size:40px;break-inside:avoid"),
                    )
                })
                .collect();
            let rules = build_rule_tree(&doc);
            let cascade = cascade(&doc, &rules).expect("cascade Ok");
            let mut page = raikiri_traits::PageBox::new();
            page.width = 800.0;
            page.height = 600.0;
            layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

            assert_eq!(
                doc.nodes[container].unrounded_layout.size.height,
                expected_height
            );
            for &child in &children {
                assert_eq!(doc.nodes[child].unrounded_layout.size.height, 40.0);
            }
            assert_eq!(doc.nodes[children[0]].unrounded_layout.location.y, 0.0);
            assert_eq!(doc.nodes[children[1]].unrounded_layout.location.y, second_y);
        }
    }
}

#[test]
fn vertical_fallback_preserves_explicit_physical_min_height() {
    use raikiri_style::{build_rule_tree, cascade};

    for minimum in [
        "min-block-size:40px;min-height:80px",
        "min-block-size:auto;min-height:80px",
        "min-block-size:calc(20px + 20px);min-height:calc(60px + 20px)",
        "min-block-size:40px;min-height:50%",
        "min-height:80px",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(
            Some(0),
            "html",
            Style::default(),
            Some("writing-mode:vertical-rl"),
        );
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;width:200px;height:160px"),
        );
        let child = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some(format!("display:block;{minimum}")),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = raikiri_traits::PageBox::new();
        page.width = 800.0;
        page.height = 600.0;
        layout_single_page(&mut doc, &cascade, page).expect("layout Ok");
        assert_eq!(
            doc.nodes[child].unrounded_layout.size.height, 80.0,
            "{minimum}"
        );
    }
}

fn multicol_gap_fixture_metrics(style_attr: &str) -> MulticolMetrics {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(Some(body), "div", Style::default(), Some(style_attr));
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            if child < parent_of.len() {
                parent_of[child] = Some(parent);
            }
        }
    }
    multicol_metrics_for_node(&cascade, &parent_of, container, 800.0).expect("multicol metrics")
}

#[test]
fn multicol_metrics_resolves_normal_gap_to_one_em_with_default_font_size() {
    // CSS Multi-column Layout Module Level 1 section 5 Column Gaps and Rules
    // https://www.w3.org/TR/css-multicol-1/#column-gaps-and-rules
    // gives column-gap normal a used value of 1em on a multicol container.
    // The default font size is 16px, so the gap is 16.0 and two columns in
    // a 200px container are (200 - 16) / 2 = 92px wide.
    let metrics = multicol_gap_fixture_metrics("display:block;column-count:2;width:200px");
    assert!(
        (metrics.column_gap - 16.0).abs() < 0.01,
        "gap={:?}",
        metrics
    );
    assert_eq!(metrics.column_count, 2);
    assert!(
        (metrics.column_width - 92.0).abs() < 0.01,
        "metrics={:?}",
        metrics
    );
}

#[test]
fn multicol_metrics_resolves_normal_gap_against_the_container_font_size() {
    // Same spec section as above: 1em means the container own computed
    // font size, so font-size:32px gives a 32px gap and
    // (200 - 32) / 2 = 84px columns.
    let metrics =
        multicol_gap_fixture_metrics("display:block;column-count:2;width:200px;font-size:32px");
    assert!(
        (metrics.column_gap - 32.0).abs() < 0.01,
        "metrics={:?}",
        metrics
    );
    assert_eq!(metrics.column_count, 2);
    assert!(
        (metrics.column_width - 84.0).abs() < 0.01,
        "metrics={:?}",
        metrics
    );
}

#[test]
fn multicol_metrics_keeps_explicit_length_and_percentage_gaps() {
    // Length and percentage arms are unchanged by the normal fix: 20px stays
    // 20px, and 10 percent of a 200px container resolves to 20px. Both give
    // (200 - 20) / 2 = 90px columns.
    let length =
        multicol_gap_fixture_metrics("display:block;column-count:2;width:200px;column-gap:20px");
    assert!(
        (length.column_gap - 20.0).abs() < 0.01,
        "metrics={:?}",
        length
    );
    assert!(
        (length.column_width - 90.0).abs() < 0.01,
        "metrics={:?}",
        length
    );
    let percent =
        multicol_gap_fixture_metrics("display:block;column-count:2;width:200px;column-gap:10%");
    assert!(
        (percent.column_gap - 20.0).abs() < 0.01,
        "metrics={:?}",
        percent
    );
    assert!(
        (percent.column_width - 90.0).abs() < 0.01,
        "metrics={:?}",
        percent
    );
}
