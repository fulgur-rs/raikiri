use super::*;
use taffy::Style;

#[test]
fn balanced_column_height_rejects_empty_unavailable_or_invalid_inputs() {
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };

    assert_eq!(balanced_column_height(&[], context), None);
    assert_eq!(
        balanced_column_height(&[(0.0, 10.0)], context.in_column(2, 200.0, 0.0)),
        None
    );
    assert_eq!(balanced_column_height(&[(10.0, 0.0)], context), None);
}

#[test]
fn line_ranges_in_columns_preserves_offsets_and_satisfies_widows() {
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: Some(100.0),
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 1,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 2,
    };
    let extents = [
        (0.0, 10.0),
        (20.0, 30.0),
        (40.0, 50.0),
        (60.0, 70.0),
        (100.0, 110.0),
    ];

    assert_eq!(
        line_ranges_in_columns(&extents, context, true),
        vec![(0, 3, 1), (3, 5, 2)],
    );
}

#[test]
fn line_ranges_in_columns_balances_when_height_is_indefinite_and_handles_empty_columns() {
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: None,
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let extents = [
        (0.0, 10.0),
        (20.0, 30.0),
        (40.0, 50.0),
        (60.0, 70.0),
        (80.0, 90.0),
    ];

    assert_eq!(
        line_ranges_in_columns(&extents, context, false),
        vec![(0, 2, 0), (2, 4, 1), (4, 5, 2)],
    );
    assert!(line_ranges_in_columns(&[], context, false).is_empty());
    assert!(line_ranges_in_columns(&extents, context.in_column(3, 300.0, 0.0), false).is_empty());
    assert!(line_ranges_in_columns(&extents, context.in_column(3, 300.0, 0.0), true).is_empty());
}

#[test]
fn multicol_subtree_has_float_skips_hidden_descendants() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let hidden = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    let float = doc.append_element(Some(hidden), "div", Style::default(), None::<&str>);
    doc.nodes[hidden].style.display = Display::None;
    doc.nodes[float].style.float = taffy::Float::Left;

    assert!(!multicol_subtree_has_float(&doc, root));

    doc.nodes[hidden].style.display = Display::Block;
    assert!(multicol_subtree_has_float(&doc, root));
}

#[test]
fn nested_row_flex_float_scope_returns_false_without_a_multicol_ancestor() {
    let mut doc = Document::new();
    let node = doc.append_element(Some(0), "div", Style::default(), None::<&str>);

    assert!(!nested_row_flex_float_scope(&doc, node));
}

#[test]
fn nested_logical_min_block_size_scope_stops_at_the_subtree_root() {
    let mut doc = Document::new();
    let outer = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let subtree_root = doc.append_element(Some(outer), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(subtree_root), "div", Style::default(), None::<&str>);
    doc.nodes[outer].has_logical_min_block_size = true;

    assert!(!nested_logical_min_block_size_scope(
        &doc,
        child,
        subtree_root
    ));

    doc.nodes[subtree_root].has_logical_min_block_size = true;
    assert!(nested_logical_min_block_size_scope(
        &doc,
        child,
        subtree_root
    ));
}

#[test]
fn nested_logical_min_block_size_scope_returns_false_for_an_unrelated_node() {
    let mut doc = Document::new();
    let subtree_root = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let unrelated = doc.append_element(None, "div", Style::default(), None::<&str>);
    doc.nodes[subtree_root].has_logical_min_block_size = true;

    assert!(!nested_logical_min_block_size_scope(
        &doc,
        unrelated,
        subtree_root
    ));
}

#[test]
fn avoid_page_min_block_child_can_continue_across_columns() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:200px;height:40px;column-count:2;column-gap:0"),
    );
    let child = doc.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some("display:block;min-block-size:20px;break-inside:avoid-page"),
    );
    for _ in 0..8 {
        doc.append_text(child, "line");
        doc.append_element(Some(child), "br", Style::default(), None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    assert!(!doc.nodes[child].has_logical_min_block_size);
    assert!(doc.nodes[child].is_ifc_root());
    let ranges = doc.nodes[child]
        .ifc_multicol_fragments()
        .expect("column line ranges");
    assert!(
        ranges.iter().any(|range| range.fragmentainer > 0),
        "avoid-page must allow the text to reach another column"
    );
}

#[test]
fn layout_offset_from_ancestor_sums_layout_parent_offsets() {
    let mut doc = Document::new();
    let ancestor = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let parent = doc.append_element(Some(ancestor), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    doc.nodes[parent].unrounded_layout.location.x = 7.0;
    doc.nodes[child].unrounded_layout.location.x = 11.0;

    assert_eq!(
        layout_offset_from_ancestor(&doc, child, ancestor),
        Some(18.0)
    );
    let detached = doc.append_element(None, "detached", Style::default(), None::<&str>);
    assert_eq!(layout_offset_from_ancestor(&doc, detached, ancestor), None);
}

#[test]
fn multicol_max_fragmentainer_height_applies_border_box_max_height() {
    let mut doc = Document::new();
    let node = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    doc.nodes[node].style.box_sizing = taffy::BoxSizing::BorderBox;
    doc.nodes[node].style.max_size.height = LengthPercentageAuto::length(100.0);

    assert_eq!(
        multicol_max_fragmentainer_height(&doc, node, 120.0, None, None),
        Some(100.0),
    );
}

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
fn balanced_auto_multicol_updates_its_container_fragment_height() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:100px;column-count:2;column-gap:0;background:green"),
    );
    for (height, extra) in [(25, ""), (50, ""), (25, ";break-before:avoid"), (50, "")] {
        doc.append_element(
            Some(container),
            "div",
            Style::default(),
            Some(&format!("display:block;height:{height}px{extra}")),
        );
    }

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    assert_eq!(doc.nodes[container].unrounded_layout.size.height, 100.0);
    let fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == container)
        .expect("multicol container fragment");
    assert_eq!(fragment.rect.height, 100.0);
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

#[test]
fn relayout_nested_flex_float_records_column_local_geometry() {
    let (mut doc, multicol, second_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);

    let style = doc.nodes[multicol].multicol.expect("multicol style");
    let context = FragmentationContext::resolve(300.0, None, style).expect("fragment context");
    assert_eq!(context.column_count, 2);
    assert!((context.column_width - 142.0).abs() < 0.01);
    assert!((context.column_gap - 16.0).abs() < 0.01);

    let float_fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .collect();
    assert!(
        !float_fragments.is_empty(),
        "the nested float needs a final fragment placement"
    );
    assert_eq!(float_fragments.len(), 1);
    assert_eq!(float_fragments[0].fragmentainer, 3);
    assert!((float_fragments[0].rect.y - 30.0).abs() < 0.01);
    let flex_item = doc.parent_of(second_float).expect("flex item parent");
    let tall_float = *doc.nodes[flex_item]
        .children
        .first()
        .expect("first float child");
    let flex_item_fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .enumerate()
        .filter(|(_, fragment)| fragment.node_id == flex_item)
        .collect();
    assert_eq!(flex_item_fragments.len(), 4);
    assert!(
        flex_item_fragments
            .iter()
            .enumerate()
            .all(|(index, (_, fragment))| {
                fragment.fragmentainer == index
                    && fragment.fragment_index == index
                    && fragment.fragment_count == 4
            })
    );
    let (last_column_item, item_fragment) = flex_item_fragments[3];
    assert_eq!(float_fragments[0].parent, Some(last_column_item));
    let item_clip = item_fragment
        .fragmentainer_clip
        .expect("flex item fragment clip");
    assert!((item_clip.x - context.column_offset_x(3)).abs() < 0.01);
    assert!((item_clip.width - context.column_width).abs() < 0.01);
    assert!((item_clip.height - 160.0).abs() < 0.01);
    let tall_fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == tall_float)
        .expect("first float fragment");
    assert_eq!(tall_fragment.fragmentainer, 0);
    assert_eq!(tall_fragment.parent, Some(flex_item_fragments[0].0));
    assert!(
        float_fragments
            .iter()
            .all(|fragment| (fragment.rect.y - 510.0).abs() > 0.01)
    );
    assert!(
        float_fragments
            .iter()
            .all(|fragment| fragment.fragmentainer_clip.is_some())
    );
    let clip = float_fragments[0]
        .fragmentainer_clip
        .expect("float fragment clip");
    assert!(clip.x.abs() < 0.01);
    assert!((clip.width - context.column_width).abs() < 0.01);
    assert!((clip.height - 160.0).abs() < 0.01);
    assert!(float_fragments.iter().enumerate().all(|(index, fragment)| {
        fragment.fragment_index == index && fragment.fragment_count == float_fragments.len()
    }));
}

#[test]
fn nested_flex_float_continues_text_past_declared_columns() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:200px;height:40px;column-count:2;column-gap:0"),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:row"),
    );
    let item = doc.append_element(Some(flex), "div", Style::default(), None::<&str>);
    doc.append_element(
        Some(item),
        "div",
        Style::default(),
        Some("float:left;width:1px;height:1px"),
    );
    for _ in 0..12 {
        doc.append_text(item, "line");
        doc.append_element(Some(item), "br", Style::default(), None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    let lines = doc.nodes[item].ifc_lines().expect("item lines");
    assert!(
        lines.len() >= 12,
        "all source lines must survive fragmentation"
    );
    let ranges = doc.nodes[item]
        .ifc_multicol_fragments()
        .expect("fragmentainer ranges");
    assert!(
        ranges.iter().any(|range| range.fragmentainer >= 2),
        "text must continue beyond the two declared columns"
    );
}

#[test]
fn tiny_fragmentainers_bound_nested_float_fragments() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:200px;columns:2;max-height:0.001px"),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:row"),
    );
    let item = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("font-size:0;line-height:0;margin-top:30px"),
    );
    let float = doc.append_element(
        Some(item),
        "div",
        Style::default(),
        Some("float:left;width:1px;height:2px"),
    );
    doc.append_element(Some(item), "br", Style::default(), None::<&str>);

    layout_nested_flex_float_fixture(&mut doc);
    let count = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == float)
        .count();
    assert!(count > 1, "float must cross more than one column");
    assert!(count <= 1_024, "float expansion must remain bounded");
    let last_float = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == float)
        .max_by_key(|fragment| fragment.fragmentainer)
        .expect("last float fragment");
    let last_clip = last_float.fragmentainer_clip.expect("last float clip");
    assert!(
        last_float.fragmentainer as f32 * 0.001 + last_clip.height >= 32.0,
        "the final float fragment must paint the remaining height"
    );
    let mut parent = last_float.parent;
    while let Some(parent_id) = parent {
        let ancestor = &doc.fragment_tree.fragments[parent_id];
        if let Some(clip) = ancestor.fragmentainer_clip {
            assert!(
                clip.height >= last_clip.height,
                "ancestor clips must include the float overflow"
            );
        }
        parent = ancestor.parent;
    }
}

#[test]
fn tiny_fragmentainers_keep_terminal_text_lines_visible() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:200px;columns:2;max-height:0.001px"),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:row"),
    );
    let item = doc.append_element(Some(flex), "div", Style::default(), None::<&str>);
    doc.append_element(
        Some(item),
        "div",
        Style::default(),
        Some("float:left;width:1px;height:2px"),
    );
    for _ in 0..12 {
        doc.append_text(item, "line");
        doc.append_element(Some(item), "br", Style::default(), None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    let lines = doc.nodes[item].ifc_lines().expect("item lines");
    assert!(lines.len() >= 12, "all source lines must survive");
    let ranges = doc.nodes[item]
        .ifc_multicol_fragments()
        .expect("fragmentainer ranges");
    let last_range = ranges.last().expect("last line range");
    assert_eq!(last_range.line_end, lines.len());
    let last_line = &lines[last_range.line_end - 1];
    let line_bottom = last_line.block_offset() + last_line.block_size();
    let clip_height = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| {
            fragment.node_id == item && fragment.fragmentainer == last_range.fragmentainer
        })
        .filter_map(|fragment| fragment.fragmentainer_clip.map(|clip| clip.height))
        .fold(0.0_f32, f32::max);
    assert!(
        clip_height >= line_bottom,
        "the final text fragment must include every continuation line"
    );
}

#[test]
fn direct_nested_text_root_records_each_column_range() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:200px;columns:2;column-gap:0;max-height:40px"),
    );
    let paragraph = doc.append_element(
        Some(multicol),
        "p",
        Style::default(),
        Some("display:block;margin:0;font-size:10px;line-height:10px"),
    );
    for _ in 0..8 {
        doc.append_text(paragraph, "line");
        doc.append_element(Some(paragraph), "br", Style::default(), None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    let lines = doc.nodes[paragraph].ifc_lines().expect("paragraph lines");
    let fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == paragraph && fragment.line_start.is_some())
        .collect();
    assert!(fragments.len() >= 2, "text must span columns");
    assert_eq!(
        fragments.last().expect("last fragment").line_end,
        Some(lines.len())
    );
}

#[test]
fn projected_multicol_inline_child_refreshes_text_ranges() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:200px;columns:2;column-gap:0;height:20px"),
    );
    let first = doc.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some("display:block;height:30px;font-size:10px;line-height:10px"),
    );
    doc.append_text(first, "one");
    doc.append_element(Some(first), "br", Style::default(), None::<&str>);
    let second = doc.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some("display:block;font-size:10px;line-height:10px"),
    );
    doc.append_text(second, "two");
    doc.append_element(Some(second), "br", Style::default(), None::<&str>);

    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[multicol].style.display, Display::Flex);
    assert!(doc.nodes[first].is_ifc_root(), "first child must own lines");
    assert!(
        doc.nodes[first].ifc_multicol_fragments().is_some(),
        "projected text ranges must be assigned"
    );
}

#[test]
fn reused_fragment_clip_expands_to_later_child_overflow() {
    let mut fragment = crate::fragment::LayoutFragment {
        node_id: 0,
        parent: None,
        fragmentainer: 0,
        rect: crate::fragment::FragmentRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 1.0,
        },
        fragmentainer_clip: Some(crate::fragment::FragmentRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 1.0,
        }),
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    };
    extend_fragment_clip(&mut fragment, 2.0);
    assert_eq!(fragment.fragmentainer_clip.expect("clip").height, 2.0);
    extend_fragment_clip(&mut fragment, 0.5);
    assert_eq!(fragment.fragmentainer_clip.expect("clip").height, 2.0);
}

#[test]
fn relayout_nested_flex_float_records_only_committed_positions() {
    let (mut doc, _, second_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);

    let float_fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .collect();
    assert_eq!(
        float_fragments.len(),
        1,
        "the second float is committed once"
    );
    assert!(
        (float_fragments[0].rect.y - 510.0).abs() > 0.01,
        "the rejected intermediate float position must not be retained"
    );
    assert_eq!(float_fragments[0].fragment_index, 0);
    assert_eq!(float_fragments[0].fragment_count, 1);
}

#[test]
fn nested_float_fragment_budget_returns_a_layout_error() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let (mut doc, _, _) = nested_flex_float_fixture("row", "horizontal-tb", 1);
    doc.fragment_tree.limit = 8;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;

    assert!(matches!(
        layout_single_page(&mut doc, &cascade, page),
        Err(LayoutError::FragmentLimitExceeded { limit: 8 })
    ));
    assert_eq!(doc.fragment_tree.fragments.len(), 8);
}

#[test]
fn zero_fragment_budget_rejects_the_first_multicol_fragment() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let (mut doc, _, _) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    doc.fragment_tree.limit = 0;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;

    assert!(matches!(
        layout_single_page(&mut doc, &cascade, page),
        Err(LayoutError::FragmentLimitExceeded { limit: 0 })
    ));
    assert!(doc.fragment_tree.fragments.is_empty());
}

#[test]
fn nested_multicol_child_stops_when_the_shared_fragment_budget_is_exhausted() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let mut saw_limit_error = false;
    for limit in [1, 2, 3, 4, 5, 6, 8, 12, 16, 24, 32, 64] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let outer = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;width:300px;columns:2;max-height:80px"),
        );
        let inner = doc.append_element(
            Some(outer),
            "div",
            Style::default(),
            Some("display:block;width:200px;columns:2;max-height:80px"),
        );
        let flex = doc.append_element(
            Some(inner),
            "div",
            Style::default(),
            Some("display:flex;flex-direction:row"),
        );
        let item = doc.append_element(Some(flex), "div", Style::default(), None::<&str>);
        doc.append_element(
            Some(item),
            "div",
            Style::default(),
            Some("float:left;width:100px;height:500px"),
        );
        doc.fragment_tree.limit = limit;
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = PageBox::new();
        page.width = 800.0;
        page.height = 600.0;

        let result = layout_single_page(&mut doc, &cascade, page);
        if matches!(result, Err(LayoutError::FragmentLimitExceeded { .. })) {
            saw_limit_error = true;
            assert_eq!(doc.fragment_tree.fragments.len(), limit);
        } else {
            assert!(result.is_ok(), "{result:?}");
            assert!(doc.fragment_tree.fragments.len() <= limit);
        }
    }
    assert!(saw_limit_error);
}

#[test]
fn nested_float_fragment_budget_accumulates_across_items() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let (mut doc, multicol, _) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    let body = doc.parent_of(multicol).expect("body");
    let mut siblings = Vec::new();
    for _ in 0..8 {
        let sibling = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;width:300px;columns:100px auto;max-height:160px"),
        );
        siblings.push(sibling);
        let flex = doc.append_element(
            Some(sibling),
            "div",
            Style::default(),
            Some("display:flex;flex-direction:row"),
        );
        let item = doc.append_element(Some(flex), "div", Style::default(), None::<&str>);
        doc.append_element(
            Some(item),
            "div",
            Style::default(),
            Some("float:left;width:100px;height:500px"),
        );
    }
    doc.fragment_tree.limit = 16;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;

    let result = layout_single_page(&mut doc, &cascade, page);
    assert!(
        matches!(
            result,
            Err(LayoutError::FragmentLimitExceeded { limit: 16 })
        ),
        "result: {result:?}, fragments: {}",
        doc.fragment_tree.fragments.len()
    );
    assert_eq!(doc.fragment_tree.fragments.len(), 16);
    assert!(
        doc.fragment_tree
            .fragments
            .iter()
            .any(|fragment| siblings.contains(&fragment.node_id))
    );
    let source_count = doc
        .fragment_tree
        .fragments
        .iter()
        .map(|fragment| fragment.node_id)
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(
        source_count > 2,
        "fragments must span multiple source nodes"
    );
}

#[test]
fn relayout_nested_flex_float_replaces_old_fragment_records() {
    let (mut doc, multicol, second_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);
    let first: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .copied()
        .collect();
    assert!(!first.is_empty(), "the first layout records the float");

    doc.set_element_inline_style(
        multicol,
        Some(
            "display:block;width:300px;columns:100px auto;max-height:120px;border:3px solid pink"
                .into(),
        ),
    );
    layout_nested_flex_float_fixture(&mut doc);
    let second: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .copied()
        .collect();

    assert!(!second.is_empty(), "the second layout records the float");
    assert_ne!(first, second, "the second pass replaces old placements");
}

#[test]
fn relayout_nested_flex_float_scope_is_row_only() {
    let (mut row_doc, _, row_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut row_doc);
    assert!(
        row_doc
            .fragment_tree
            .fragments
            .iter()
            .any(|fragment| fragment.node_id == row_float)
    );

    for (direction, writing_mode) in [("column", "horizontal-tb"), ("row", "vertical-rl")] {
        let (mut doc, multicol, second_float) =
            nested_flex_float_fixture(direction, writing_mode, 160);
        layout_nested_flex_float_fixture(&mut doc);
        if writing_mode != "horizontal-tb" {
            assert_eq!(
                doc.nodes[multicol].authored_writing_mode,
                Some(raikiri_style::property::WritingMode::VerticalRl)
            );
        }
        assert!(
            !doc.fragment_tree
                .fragments
                .iter()
                .any(|fragment| fragment.node_id == second_float),
            "float fragments should be scoped out for flex-direction:{direction}, writing-mode:{writing_mode}"
        );
    }
}

fn nested_flex_float_fixture(
    flex_direction: &str,
    writing_mode: &str,
    max_height: u16,
) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let html_style =
        (writing_mode != "horizontal-tb").then(|| format!("writing-mode:{writing_mode}"));
    let html = doc.append_element(Some(0), "html", Style::default(), html_style.as_deref());
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let multicol_writing_mode = if writing_mode == "horizontal-tb" {
        String::new()
    } else {
        format!(";writing-mode:{writing_mode}")
    };
    let multicol = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(format!(
            "display:block;width:300px;columns:100px auto;max-height:{max_height}px;border:3px solid pink{multicol_writing_mode}"
        )),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Style::default(),
        Some(format!("display:flex;flex-direction:{flex_direction}")),
    );
    let flex_item = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("border:4px solid teal;outline:4px solid blue"),
    );
    doc.append_element(
        Some(flex_item),
        "div",
        Style::default(),
        Some("float:left;border:3px solid black;height:500px;width:100px;background:yellow"),
    );
    doc.append_element(Some(flex_item), "br", Style::default(), None::<&str>);
    let second_float = doc.append_element(
        Some(flex_item),
        "div",
        Style::default(),
        Some("float:left;background:cyan;width:100px"),
    );
    doc.append_element(
        Some(second_float),
        "div",
        Style::default(),
        Some("height:30px;width:30px;background:purple;display:inline-block"),
    );
    (doc, multicol, second_float)
}

fn layout_nested_flex_float_fixture(doc: &mut Document) {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let rules = build_rule_tree(doc);
    let cascade = cascade(doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(doc, &cascade, page).expect("layout Ok");
}

#[test]
fn max_height_multicol_fragment_does_not_expand_to_float_overflow() {
    let (mut doc, multicol, _) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);

    let fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == multicol)
        .expect("multicol fragment");
    assert!(
        fragment.rect.height <= 166.0,
        "max-height:160px plus 3px borders must bound the fragment, got {}",
        fragment.rect.height
    );
}
