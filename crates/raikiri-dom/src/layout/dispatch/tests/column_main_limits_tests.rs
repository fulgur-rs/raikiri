use super::column_cross_size_tests::column_rects;

fn matrix_rectangles(case: &serde_json::Value) -> serde_json::Value {
    use super::column_cross_size_tests::column_document;
    use crate::Document;
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{absolute_rect, cascaded, page_box_800x600, with_ahem};

    fn append_children<'a>(
        doc: &mut Document,
        parent: usize,
        spec: &'a serde_json::Value,
        nodes: &mut Vec<(&'a str, usize)>,
    ) {
        if let Some(children) = spec["children"].as_array() {
            for child in children {
                let node = doc.append_element(
                    Some(parent),
                    "div",
                    Some(format!(
                        "display:block;box-sizing:border-box;{}",
                        child["style"].as_str().unwrap()
                    )),
                );
                doc.append_text(node, child["text"].as_str().unwrap_or(""));
                nodes.push((child["id"].as_str().unwrap(), node));
                append_children(doc, node, child, nodes);
            }
        }
    }
    let items = case["items"].as_array().unwrap();
    let first = &items[0];
    let (mut doc, container, first_node) = column_document(
        case["container"].as_str().unwrap(),
        &format!("display:block;{}", first["style"].as_str().unwrap()),
        first["text"].as_str().unwrap(),
    );
    let mut nodes = vec![
        ("container", container),
        (first["id"].as_str().unwrap(), first_node),
    ];
    append_children(&mut doc, first_node, first, &mut nodes);
    for spec in &items[1..] {
        let node = doc.append_element(
            Some(container),
            "div",
            Some(format!(
                "display:block;box-sizing:border-box;{}",
                spec["style"].as_str().unwrap()
            )),
        );
        doc.append_text(node, spec["text"].as_str().unwrap());
        append_children(&mut doc, node, spec, &mut nodes);
        nodes.push((spec["id"].as_str().unwrap(), node));
    }
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let mut rectangles = serde_json::Map::new();
    for (name, node) in nodes {
        let (x, y, w, h) = absolute_rect(&doc, node);
        rectangles.insert(name.to_owned(), serde_json::json!([x, y, w, h]));
    }
    rectangles.into()
}

#[test]
fn column_main_limits_match_loaded_ahem_or_preserve_documented_baselines() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("data/column-main-limits.json")).unwrap();
    assert_eq!(
        fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case.get("preserved_baseline").is_some())
            .count(),
        7
    );
    let mut differences = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let actual = matrix_rectangles(case);
        assert_eq!(
            actual.as_object().unwrap().len(),
            case["rectangles"].as_object().unwrap().len()
        );
        let expected = case
            .get("preserved_baseline")
            .map_or(&case["rectangles"], |baseline| &baseline["rectangles"]);
        let close = actual.as_object().unwrap().iter().all(|(id, rect)| {
            rect.as_array()
                .unwrap()
                .iter()
                .enumerate()
                .all(|(axis, value)| {
                    (value.as_f64().unwrap() - expected[id][axis].as_f64().unwrap()).abs() <= 0.02
                })
        });
        if !close {
            differences.push(format!("{}: {} != {}", case["name"], actual, expected));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn column_container_main_size_respects_the_items_active_main_maximum() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("data/column-cross-size.json")).expect("fixture");
    let rect = |value: &serde_json::Value| {
        let coordinate = |i: usize| value[i].as_f64().expect("coordinate") as f32;
        (coordinate(0), coordinate(1), coordinate(2), coordinate(3))
    };
    let mut differences = Vec::new();
    for case in fixture["main_limit_controls"].as_array().expect("controls") {
        let actual = column_rects(
            case["container"].as_str().unwrap(),
            &format!("display:block;{}", case["item"].as_str().unwrap()),
            case["text"].as_str().unwrap(),
        );
        let expected = (rect(&case["container_rect"]), rect(&case["item_rect"]));
        if actual != expected {
            differences.push(format!("{}: {actual:?} != {expected:?}", case["name"]));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn nested_bounded_columns_keep_measurement_work_bounded() {
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{
        COLUMN_MAIN_PROBES, absolute_rect, cascaded, page_box_800x600, with_ahem,
    };
    let mut counts = Vec::new();
    for depth in [1, 2, 4, 8] {
        let (mut doc, root, mut child) = super::column_cross_size_tests::column_document(
            "display:inline-flex;flex-direction:column;width:10px",
            "display:flex;flex-direction:column;flex:0 0 120px;max-height:80px",
            "",
        );
        for _ in 1..depth {
            child = doc.append_element(
                Some(child),
                "div",
                Some("display:flex;flex-direction:column;flex:0 0 120px;max-height:80px"),
            );
        }
        doc.append_text(child, "x");
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        COLUMN_MAIN_PROBES.with(|count| count.set(0));
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        counts.push(COLUMN_MAIN_PROBES.with(|count| count.get()));
        assert_eq!(absolute_rect(&doc, root).3, 80.0);
        assert_eq!(absolute_rect(&doc, child).3, 80.0);
    }
    assert!(counts[3] <= counts[2] * 4 + 8, "{counts:?}");
}

#[test]
fn bounded_column_measurements_restore_styles_and_cached_results() {
    use crate::layout::context::LayoutSession;
    use crate::layout::protocol::{
        AvailableSpace, LayoutInput, RequestedAxis, RunMode, SizingMode,
    };
    use crate::layout::test_support::{cascaded, with_ahem};
    fn fixture(vertical: bool) -> (crate::Document, usize, usize, usize, usize) {
        let (container, item) = if vertical {
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;row-gap:calc(10% + 5px)",
                "display:block;flex:0.5 1 160px;max-width:80px;position:relative;left:10%;right:5px;writing-mode:horizontal-tb;padding:5%",
            )
        } else {
            (
                "display:inline-flex;flex-direction:column;row-gap:calc(10% + 5px)",
                "display:block;flex:0.5 1 120px;max-height:80px;position:relative;top:10%;bottom:5px;writing-mode:vertical-rl;padding:5%",
            )
        };
        let (mut doc, root, item) =
            super::column_cross_size_tests::column_document(container, item, "xxxx xxxx xx");
        let child = doc.append_element(
            Some(item),
            "span",
            Some("display:inline-block;width:25%;height:20%;padding:5%"),
        );
        doc.append_text(child, "xx");
        let image = doc.append_element(Some(root), "img", Some("display:block;writing-mode:horizontal-tb;flex:0 0 40px;width:15px;height:20px;padding:5%"));
        doc.mark_in_document_flags();
        with_ahem(&mut doc);
        (doc, root, item, child, image)
    }
    for vertical in [false, true] {
        let (mut doc, container, item, child, image) = fixture(vertical);
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let basis = tree.physical_style(item).flex_basis();
        let gap = tree.physical_style(container).gap();
        let inset = tree.physical_style(item).inset();
        let mut inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            ..LayoutInput::HIDDEN
        };
        inputs.available_space.width = AvailableSpace::MaxContent;
        inputs.available_space.height = AvailableSpace::MaxContent;
        let original = tree.compute_child_layout(container.into(), inputs).size;
        let epoch = tree.layout_cache_context.epoch();
        for definite in [true, false, true, false] {
            let mut constrained = inputs;
            if vertical {
                constrained.known_dimensions.width = Some(200.0);
                constrained.known_dimensions_are_definite.width = definite;
            } else {
                constrained.known_dimensions.height = Some(200.0);
                constrained.known_dimensions_are_definite.height = definite;
            }
            for run_mode in [RunMode::ComputeSize, RunMode::PerformLayout] {
                for axis in [
                    RequestedAxis::Both,
                    RequestedAxis::Horizontal,
                    RequestedAxis::Vertical,
                    RequestedAxis::Both,
                ] {
                    constrained.run_mode = run_mode;
                    constrained.axis = axis;
                    let actual = tree
                        .compute_child_layout(container.into(), constrained)
                        .size;
                    let (mut fresh_doc, fresh_root, fresh_item, fresh_child, fresh_image) =
                        fixture(vertical);
                    let fresh_cascade = cascaded(&fresh_doc);
                    let mut fresh = LayoutSession::bridged(&mut fresh_doc, &fresh_cascade);
                    let expected = fresh
                        .compute_child_layout(fresh_root.into(), constrained)
                        .size;
                    if run_mode == RunMode::PerformLayout {
                        assert_eq!(tree.nodes[item].layout, fresh.nodes[fresh_item].layout);
                        assert_eq!(tree.nodes[child].layout, fresh.nodes[fresh_child].layout);
                        assert_eq!(tree.nodes[image].layout, fresh.nodes[fresh_image].layout);
                    }
                    // A single-axis ComputeSize does not promise the other dimension.
                    if run_mode == RunMode::PerformLayout || axis != RequestedAxis::Vertical {
                        assert_eq!(
                            actual.width, expected.width,
                            "vertical={vertical} definite={definite} mode={run_mode:?} axis={axis:?}"
                        );
                    }
                    if run_mode == RunMode::PerformLayout || axis != RequestedAxis::Horizontal {
                        assert_eq!(
                            actual.height, expected.height,
                            "vertical={vertical} definite={definite} mode={run_mode:?} axis={axis:?}"
                        );
                    }
                }
            }
            assert_eq!(
                tree.compute_child_layout(container.into(), inputs).size,
                original
            );
            assert_eq!(tree.physical_style(item).flex_basis(), basis);
            assert_eq!(tree.physical_style(item).flex_grow(), 0.5);
            assert_eq!(tree.physical_style(item).flex_shrink(), 1.0);
            assert_eq!(tree.physical_style(container).gap(), gap);
            assert_eq!(tree.physical_style(item).inset(), inset);
            assert_eq!(tree.override_checkpoint(), 0);
            assert_eq!(tree.layout_cache_context.epoch(), epoch);
        }
    }
}

#[test]
fn out_of_flow_and_hidden_items_do_not_set_the_column_main_contribution() {
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{absolute_rect, cascaded, page_box_800x600, with_ahem};
    for vertical in [false, true] {
        let (container_style, item_style) = if vertical {
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;position:relative",
                "display:block;writing-mode:horizontal-tb;flex:0 0 160px;max-width:80px",
            )
        } else {
            (
                "display:inline-flex;flex-direction:column;position:relative",
                "display:block;writing-mode:vertical-rl;flex:0 0 120px;max-height:80px",
            )
        };
        let (mut doc, container, item) = super::column_cross_size_tests::column_document(
            container_style,
            item_style,
            "xxxx xxxx xx",
        );
        let absolute = doc.append_element(Some(container), "div", Some("display:block;position:absolute;flex-basis:auto;width:500px;height:500px;top:0;left:0"));
        let hidden = doc.append_element(
            Some(container),
            "div",
            Some("display:none;flex-basis:auto;width:900px;height:900px"),
        );
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        let root_rect = absolute_rect(&doc, container);
        let item_rect = absolute_rect(&doc, item);
        assert_eq!(if vertical { root_rect.2 } else { root_rect.3 }, 80.0);
        assert_eq!(if vertical { item_rect.2 } else { item_rect.3 }, 80.0);
        let abs = absolute_rect(&doc, absolute);
        assert_eq!((abs.2, abs.3), (500.0, 500.0));
        let hidden = absolute_rect(&doc, hidden);
        assert_eq!((hidden.2, hidden.3), (0.0, 0.0));
    }
}

#[test]
fn an_invalid_resolved_flex_basis_delegates_without_a_measurement_override() {
    use crate::layout::context::LayoutSession;
    use crate::layout::protocol::{LayoutInput, LayoutOutput, RequestedAxis, RunMode};
    use crate::layout::test_support::{cascaded, with_ahem};
    use raikiri_layout_geometry::Size;
    use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
    use raikiri_style::ComputedLengthPercentageOrAuto as L;
    let (mut doc, container, item) = super::column_cross_size_tests::column_document(
        "display:flex;flex-direction:column;writing-mode:vertical-rl",
        "display:block;flex:0 0 120px;max-width:80px",
        "x",
    );
    with_ahem(&mut doc);
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(item).unwrap();
    tree.push_local_overrides(vec![(
        key,
        OwnedPropertyOverride::length(LengthProperty::FlexBasis, L::Px(-1.0), true).unwrap(),
    )]);
    let checkpoint = tree.override_checkpoint();
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Both,
        ..LayoutInput::HIDDEN
    };
    let calls = std::cell::Cell::new(0);
    let marker = Size {
        width: 17.0,
        height: 19.0,
    };
    let output = super::super::column_main_limits::layout(
        &mut tree,
        container.into(),
        inputs,
        false,
        |tree, node, received| {
            calls.set(calls.get() + 1);
            assert_eq!(node, container.into());
            assert_eq!(received, inputs);
            assert_eq!(tree.override_checkpoint(), checkpoint);
            LayoutOutput::from_outer_size(marker)
        },
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(output.size, marker);
    tree.restore_overrides_to(0);
}

#[test]
fn nested_orthogonal_columns_do_not_remeasure_descendants_exponentially() {
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{
        COLUMN_CROSS_MEASUREMENTS, absolute_rect, cascaded, page_box_800x600, with_ahem,
    };
    let mut counts = Vec::new();
    for depth in [1, 2, 4, 8] {
        let (mut doc, root, mut paragraph) = super::column_cross_size_tests::column_document(
            "display:inline-flex;flex-direction:column",
            "display:block;writing-mode:vertical-rl;flex:0 0 60px",
            "x",
        );
        for _ in 1..depth {
            let flex = doc.append_element(
                Some(paragraph),
                "span",
                Some("display:inline-flex;writing-mode:horizontal-tb;flex-direction:column"),
            );
            paragraph = doc.append_element(
                Some(flex),
                "div",
                Some("display:block;writing-mode:vertical-rl;flex:0 0 60px"),
            );
            doc.append_text(paragraph, "x");
        }
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        COLUMN_CROSS_MEASUREMENTS.with(|count| count.set(0));
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        counts.push(COLUMN_CROSS_MEASUREMENTS.with(|count| count.get()));
        let (_, _, width, height) = absolute_rect(&doc, root);
        assert!(width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0);
    }
    eprintln!("nested orthogonal measurement counts: {counts:?}");
    assert!(counts[3] <= counts[2] * 4 + 64, "{counts:?}");
}

#[test]
fn an_unresolved_minimum_keeps_every_siblings_original_flex_basis() {
    use crate::layout::context::LayoutSession;
    use crate::layout::protocol::{LayoutInput, LayoutOutput, RequestedAxis, RunMode};
    use crate::layout::test_support::{cascaded, with_ahem};
    use raikiri_layout_style::values::Dimension;
    let (mut doc, container, first) = super::column_cross_size_tests::column_document(
        "display:flex;flex-direction:column;writing-mode:vertical-rl",
        "display:block;flex:0 0 120px;max-width:80px",
        "x",
    );
    doc.append_element(
        Some(container),
        "div",
        Some("display:block;flex:0 0 40px;min-width:50%"),
    );
    doc.mark_in_document_flags();
    with_ahem(&mut doc);
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Both,
        ..LayoutInput::HIDDEN
    };
    let calls = std::cell::Cell::new(0);
    super::super::column_main_limits::layout(
        &mut tree,
        container.into(),
        inputs,
        false,
        |tree, _, received| {
            calls.set(calls.get() + 1);
            assert_eq!(received, inputs);
            assert_eq!(
                tree.physical_style(first).flex_basis(),
                Dimension::length(120.0)
            );
            assert_eq!(tree.override_checkpoint(), 0);
            LayoutOutput::HIDDEN
        },
    );
    assert_eq!(calls.get(), 1);
}
