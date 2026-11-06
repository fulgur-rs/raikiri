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
fn column_main_limits_match_chromium_and_preserve_existing_cross_residuals() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("data/column-main-limits.json")).unwrap();
    let mut differences = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let actual = matrix_rectangles(case);
        assert_eq!(
            actual.as_object().unwrap().len(),
            case["rectangles"].as_object().unwrap().len()
        );
        let vertical = case["name"].as_str().unwrap().starts_with("vertical-");
        let close = actual.as_object().unwrap().iter().all(|(id, rect)| {
            rect.as_array()
                .unwrap()
                .iter()
                .enumerate()
                .all(|(axis, value)| {
                    let cross = if vertical {
                        axis == 1 || axis == 3
                    } else {
                        axis == 0 || axis == 2
                    };
                    let expected = if cross && case.get("existing_cross_baseline").is_some() {
                        &case["existing_cross_baseline"][id][if axis < 2 { 0 } else { 1 }]
                    } else {
                        &case["rectangles"][id][axis]
                    };
                    (value.as_f64().unwrap() - expected.as_f64().unwrap()).abs() <= 0.02
                })
        });
        if !close {
            differences.push(format!(
                "{}: {} != {}",
                case["name"], actual, case["rectangles"]
            ));
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
    for vertical in [false, true] {
        let (container, item) = if vertical {
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;row-gap:calc(10% + 5px)",
                "display:block;flex:0.5 1 160px;max-width:80px;position:relative;left:10%;right:5px;writing-mode:horizontal-tb",
            )
        } else {
            (
                "display:inline-flex;flex-direction:column;row-gap:calc(10% + 5px)",
                "display:block;flex:0.5 1 120px;max-height:80px;position:relative;top:10%;bottom:5px;writing-mode:vertical-rl",
            )
        };
        let (mut doc, container, item) =
            super::column_cross_size_tests::column_document(container, item, "xxxx xxxx xx");
        with_ahem(&mut doc);
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let epoch = tree.layout_cache_context.epoch();
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
                    let (mut fresh_doc, fresh_container, _) =
                        super::column_cross_size_tests::column_document(
                            if vertical {
                                "display:flex;flex-direction:column;writing-mode:vertical-rl;row-gap:calc(10% + 5px)"
                            } else {
                                "display:inline-flex;flex-direction:column;row-gap:calc(10% + 5px)"
                            },
                            if vertical {
                                "display:block;flex:0.5 1 160px;max-width:80px;position:relative;left:10%;right:5px;writing-mode:horizontal-tb"
                            } else {
                                "display:block;flex:0.5 1 120px;max-height:80px;position:relative;top:10%;bottom:5px;writing-mode:vertical-rl"
                            },
                            "xxxx xxxx xx",
                        );
                    with_ahem(&mut fresh_doc);
                    let fresh_cascade = cascaded(&fresh_doc);
                    let mut fresh = LayoutSession::bridged(&mut fresh_doc, &fresh_cascade);
                    let expected = fresh
                        .compute_child_layout(fresh_container.into(), constrained)
                        .size;
                    // ComputeSize does not promise the unrequested axis; a cache
                    // hit may retain it while a fresh measurement leaves it zero.
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
