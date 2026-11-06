use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, cascaded, page_box_800x600, with_ahem};

type Rect = (f32, f32, f32, f32);

pub(super) fn column_document(
    container_style: &str,
    item_style: &str,
    text: &str,
) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:800px;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let container = doc.append_element(
        Some(body),
        "div",
        Some(format!("box-sizing:border-box;{container_style}")),
    );
    let item = doc.append_element(
        Some(container),
        "div",
        Some(format!("box-sizing:border-box;{item_style}")),
    );
    doc.append_text(item, text);
    doc.mark_in_document_flags();
    (doc, container, item)
}

pub(super) fn column_rects(container_style: &str, item_style: &str, text: &str) -> (Rect, Rect) {
    let (mut doc, container, item) = column_document(container_style, item_style, text);
    let cascade = cascaded(&doc);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (absolute_rect(&doc, container), absolute_rect(&doc, item))
}

#[test]
fn hypothetical_cross_measurements_preserve_main_limits_and_ordinary_cached_sizes() {
    use crate::layout::context::LayoutSession;
    use crate::layout::protocol::{
        AvailableSpace, LayoutInput, RequestedAxis, RunMode, SizingMode,
    };
    use raikiri_layout_geometry::Size;

    for vertical in [false, true] {
        let (container, item_style, text) = if vertical {
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl",
                "display:block;flex:0 0 80px;writing-mode:horizontal-tb;max-width:80px",
                "xxxx xxxx xxxx",
            )
        } else {
            (
                "display:inline-flex;flex-direction:column",
                "display:block;flex:0 0 60px;writing-mode:vertical-rl;max-height:80px",
                "xxxx xxxx xx",
            )
        };
        let (mut doc, _, item) = column_document(container, item_style, text);
        with_ahem(&mut doc);
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let original_max = tree.physical_style(item).max_size();
        let epoch = tree.layout_cache_context.epoch();
        let ordinary = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::ContentSize,
            axis: if vertical {
                RequestedAxis::Vertical
            } else {
                RequestedAxis::Horizontal
            },
            parent_size: if vertical {
                Size {
                    width: Some(120.0),
                    height: None,
                }
            } else {
                Size {
                    width: None,
                    height: Some(120.0),
                }
            },
            ..LayoutInput::HIDDEN
        };
        let original = tree.compute_child_layout(item.into(), ordinary).size;
        let original_cross = if vertical {
            original.height
        } else {
            original.width
        };
        assert!(original_cross > 10.0);
        for main in [60.0, 120.0, 60.0] {
            let mut inputs = ordinary;
            if vertical {
                inputs.known_dimensions.width = Some(main);
                inputs.available_space.width = AvailableSpace::Definite(main);
            } else {
                inputs.known_dimensions.height = Some(main);
                inputs.available_space.height = AvailableSpace::Definite(main);
            }
            let output = tree.compute_child_layout(item.into(), inputs);
            let expected = if vertical {
                Size {
                    width: main,
                    height: 30.0,
                }
            } else {
                Size {
                    width: 20.0,
                    height: main,
                }
            };
            assert_eq!(output.size, expected);
            assert_eq!(tree.physical_style(item).max_size(), original_max);
            assert_eq!(
                tree.compute_child_layout(item.into(), ordinary).size,
                original
            );
            assert_eq!(tree.override_checkpoint(), 0);
            assert_eq!(tree.layout_cache_context.epoch(), epoch);
        }
    }
}

#[test]
fn inline_column_cross_size_uses_the_orthogonal_items_intrinsic_contribution() {
    for mode in ["vertical-rl", "vertical-lr"] {
        for basis in [60, 120] {
            let rects = column_rects(
                "display:inline-flex;flex-direction:column",
                &format!("display:block;writing-mode:{mode};flex:0 0 {basis}px"),
                "xxxx xxxx xx",
            );
            let expected = (0.0, 0.0, 10.0, basis as f32);
            assert_eq!(rects, (expected, expected), "{mode}, basis {basis}");
        }
    }
}

#[test]
fn atomic_inline_descendants_measure_the_same_with_fresh_and_reused_caches() {
    use crate::layout::context::LayoutSession;
    use crate::layout::protocol::{
        AvailableSpace, LayoutInput, RequestedAxis, RunMode, SizingMode,
    };

    fn fixture(vertical: bool) -> (Document, usize) {
        let (container, item, text) = if vertical {
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl",
                "display:block;flex:0 0 80px;writing-mode:horizontal-tb;max-width:80px",
                "xxxx xxxx xxxx",
            )
        } else {
            (
                "display:inline-flex;flex-direction:column",
                "display:block;flex:0 0 60px;writing-mode:vertical-rl;max-height:80px",
                "xxxx xxxx xx",
            )
        };
        let (mut doc, _, item) = column_document(container, item, text);
        let inline = doc.append_element(
            Some(item),
            "span",
            Some("display:inline-block;width:25%;height:20%;padding:5%"),
        );
        doc.append_text(inline, "xx");
        doc.mark_in_document_flags();
        with_ahem(&mut doc);
        (doc, item)
    }

    for vertical in [false, true] {
        let (mut doc, item) = fixture(vertical);
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let original_max = tree.physical_style(item).max_size();
        let ordinary = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::ContentSize,
            axis: if vertical {
                RequestedAxis::Vertical
            } else {
                RequestedAxis::Horizontal
            },
            ..LayoutInput::HIDDEN
        };
        let original = tree.compute_child_layout(item.into(), ordinary).size;
        for main in [60.0, 120.0, 60.0] {
            let mut inputs = ordinary;
            if vertical {
                inputs.known_dimensions.width = Some(main);
                inputs.parent_size.width = Some(main);
                inputs.available_space.width = AvailableSpace::Definite(main);
            } else {
                inputs.known_dimensions.height = Some(main);
                inputs.parent_size.height = Some(main);
                inputs.available_space.height = AvailableSpace::Definite(main);
            }
            let (mut fresh_doc, fresh_item) = fixture(vertical);
            let fresh_cascade = cascaded(&fresh_doc);
            let mut fresh_tree = LayoutSession::bridged(&mut fresh_doc, &fresh_cascade);
            let expected = fresh_tree
                .compute_child_layout(fresh_item.into(), inputs)
                .size;
            assert_eq!(
                tree.compute_child_layout(item.into(), inputs).size,
                expected
            );
            assert_eq!(
                tree.compute_child_layout(item.into(), ordinary).size,
                original
            );
            assert_eq!(tree.physical_style(item).max_size(), original_max);
            assert_eq!(tree.override_checkpoint(), 0);
        }
    }
}

#[test]
fn vertical_column_cross_size_uses_the_orthogonal_items_intrinsic_contribution() {
    for mode in ["vertical-rl", "vertical-lr"] {
        for basis in [80, 160] {
            let rects = column_rects(
                &format!("display:flex;flex-direction:column;writing-mode:{mode}"),
                &format!("display:block;writing-mode:horizontal-tb;flex:0 0 {basis}px"),
                "xxxx xxxx xxxx",
            );
            let expected = (0.0, 0.0, basis as f32, 10.0);
            assert_eq!(rects, (expected, expected), "{mode}, basis {basis}");
        }
    }
}

#[test]
fn orthogonal_column_items_match_the_chromium_cross_size_matrix() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("data/column-cross-size.json")).expect("fixture");
    let mut differences = Vec::new();
    let rect = |value: &serde_json::Value| {
        let values = value.as_array().expect("rect");
        let n = |i: usize| values[i].as_f64().expect("coordinate") as f32;
        (n(0), n(1), n(2), n(3))
    };
    for case in data["cases"].as_array().expect("cases") {
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
fn item_main_maximum_remains_effective_after_hypothetical_cross_measurement() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("data/column-cross-size.json")).expect("fixture");
    for case in data["main_limit_controls"].as_array().expect("controls") {
        let actual = column_rects(
            case["container"].as_str().unwrap(),
            &format!("display:block;{}", case["item"].as_str().unwrap()),
            case["text"].as_str().unwrap(),
        );
        let width = case["item_rect"][2].as_f64().unwrap() as f32;
        let height = case["item_rect"][3].as_f64().unwrap() as f32;
        assert_eq!(
            (actual.1.2, actual.1.3),
            (width, height),
            "{}",
            case["name"]
        );
        let vertical = case["container"].as_str().unwrap().contains("writing-mode");
        // Full container sizes and positions are also checked by column_main_limits_tests.
        let cross = if vertical { actual.0.3 } else { actual.0.2 };
        assert_eq!(
            cross,
            if vertical { height } else { width },
            "{}",
            case["name"]
        );
    }
}
