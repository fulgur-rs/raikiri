use super::*;
use crate::Document;
use crate::layout::context::LayoutSession;
use crate::layout::test_support::cascaded;

type Rect = (f32, f32, f32, f32);

fn boxes(case: &str) -> Vec<Rect> {
    let fields: Vec<_> = case.split('/').collect();
    let (direction, wrap, text_direction) = (fields[0], fields[1], fields[2]);
    let count: usize = fields[3].parse().unwrap();
    let cross: f32 = fields[4].parse().unwrap();
    let alignment = fields[5];
    let row = direction.starts_with("row");
    let (width, height) = if row { (100.0, cross) } else { (cross, 50.0) };
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "div", Some(&format!(
        "display:flex;box-sizing:border-box;flex-direction:{direction};flex-wrap:{wrap};direction:{text_direction};width:{width}px;height:{height}px;align-content:{alignment}"
    )));
    let children: Vec<_> = [100, 20]
        .into_iter()
        .take(count)
        .map(|cross| {
            doc.append_element(
                Some(root),
                "div",
                Some(&format!(
                    "box-sizing:border-box;width:100px;height:{}px",
                    if row { cross } else { 40 }
                )),
            )
        })
        .collect();
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.compute_child_layout(
        root.into(),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            known_dimensions: Size {
                width: Some(width),
                height: Some(height),
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
            ..LayoutInput::HIDDEN
        },
    );
    children
        .into_iter()
        .map(|child| {
            let b = tree.nodes[child].layout;
            (b.location.x, b.location.y, b.size.width, b.size.height)
        })
        .collect()
}

fn parse_rect(text: &str) -> Rect {
    let n: Vec<f32> = text.split(',').map(|n| n.parse().unwrap()).collect();
    (n[0], n[1], n[2], n[3])
}

#[test]
fn wrapped_line_positions_match_the_chromium_alignment_matrix() {
    // Read the unrounded border boxes: a page fragment intentionally clips
    // a negative y coordinate at its page edge and is not the whole box.
    let mut failures = Vec::new();
    let mut count = 0;
    for line in include_str!("data/flex_wrap_reverse_chromium.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        let mut expected = vec![parse_rect(fields[1])];
        if fields[2] != "-" {
            expected.push(parse_rect(fields[2]));
        }
        let actual = boxes(fields[0]);
        // Chromium quantizes layout to 1/64px; the engine retains f32 positions.
        let close = actual.len() == expected.len()
            && actual.iter().zip(&expected).all(|(a, b)| {
                [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)]
                    .into_iter()
                    .all(|(a, b)| (a - b).abs() <= 0.02)
            });
        if !close {
            failures.push(format!("{}: {actual:?} != {expected:?}", fields[0]));
        }
        count += 1;
    }
    assert_eq!(count, 1344);
    assert!(
        failures.is_empty(),
        "{} cases differ; first differences:\n{}",
        failures.len(),
        failures.into_iter().take(24).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn wrapped_overflow_keeps_baselines_absolute_children_and_writing_modes() {
    use raikiri_html::{
        LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
        parse_html_with_resources,
    };
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("data/flex_wrap_reverse_edges.json")).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 76);
    let mut failures = Vec::new();
    for case in cases.as_array().unwrap() {
        let markup = format!(
            "<!doctype html><style>@page{{size:800px 600px;margin:0}}body{{margin:0}}div{{box-sizing:border-box}}</style>{}",
            case["markup"].as_str().unwrap()
        );
        let resources = RenderResources::new();
        let parsed = parse_html_with_resources(markup.as_bytes(), &resources).unwrap();
        let LayoutStatus::Completed(doc) = layout(
            &parsed,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .unwrap() else {
            panic!("incomplete layout")
        };
        for expected in case["boxes"].as_array().unwrap() {
            let id = expected[0].as_str().unwrap();
            if id == "h" {
                continue;
            }
            let mut found = Vec::new();
            for page in doc.pages() {
                for f in page.fragments() {
                    if page.dom().attr(f.node(), "id") == Some(id) {
                        let r = f.rect();
                        found.push([r.x, r.y, r.width, r.height]);
                    }
                }
            }
            let expected: Vec<f32> = expected[1]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap() as f32)
                .collect();
            if found.len() != 1
                || !found[0]
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| (a - b).abs() <= 0.02)
            {
                failures.push(format!(
                    "{} / {id}: {found:?} != {expected:?}",
                    case["name"]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_large_relative_inset_does_not_hide_a_representable_one_pixel_shift() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "div",
        Some("display:flex;flex-wrap:wrap-reverse;width:100px;height:100px"),
    );
    let child = doc.append_element(
        Some(root),
        "div",
        Some("width:100px;height:101px;position:relative;top:10000000px"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.compute_child_layout(
        root.into(),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            known_dimensions: Size {
                width: Some(100.0),
                height: Some(100.0),
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(100.0),
                height: AvailableSpace::Definite(100.0),
            },
            ..LayoutInput::HIDDEN
        },
    );
    assert_eq!(tree.nodes[child].layout.location.y, 9999999.0);
}

#[test]
fn unequal_relative_coordinate_scales_keep_the_same_line_shift() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "div",
        Some("display:flex;flex-wrap:wrap-reverse;width:100px;height:50px"),
    );
    let first = doc.append_element(
        Some(root),
        "div",
        Some("width:100px;height:100.1px;position:relative;top:1000000px"),
    );
    let second = doc.append_element(Some(root), "div", Some("width:100px;height:20px"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.compute_child_layout(
        root.into(),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            known_dimensions: Size {
                width: Some(100.0),
                height: Some(50.0),
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(100.0),
                height: AvailableSpace::Definite(50.0),
            },
            ..LayoutInput::HIDDEN
        },
    );
    assert_eq!(f64::from(tree.nodes[first].layout.location.y), 999_949.875);
    assert!((tree.nodes[second].layout.location.y + 70.1).abs() < 0.0001);
}
