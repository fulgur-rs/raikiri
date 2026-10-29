use super::*;
use crate::layout::test_support::{ahem_paragraph_with, page_box_800x600, with_ahem};

#[derive(Clone, Copy, Debug)]
struct Work {
    anchors: usize,
    boxes: usize,
    metadata: usize,
    calls: usize,
    anchor_indexes: usize,
    fallback_indexes: usize,
}

fn reset() {
    OOF_ANCHOR_INDEX_BUILDS.with(|count| count.set(0));
    OOF_FALLBACK_INDEX_BUILDS.with(|count| count.set(0));
    OOF_ANCHOR_VISITS.with(|count| count.set(0));
    OOF_BOX_VISITS.with(|count| count.set(0));
    OOF_METADATA_VISITS.with(|count| count.set(0));
    OOF_PLACE_CALLS.with(|count| count.set(0));
}

fn read() -> Work {
    Work {
        anchor_indexes: OOF_ANCHOR_INDEX_BUILDS.with(|count| count.get()),
        fallback_indexes: OOF_FALLBACK_INDEX_BUILDS.with(|count| count.get()),
        anchors: OOF_ANCHOR_VISITS.with(|count| count.get()),
        boxes: OOF_BOX_VISITS.with(|count| count.get()),
        metadata: OOF_METADATA_VISITS.with(|count| count.get()),
        calls: OOF_PLACE_CALLS.with(|count| count.get()),
    }
}

fn placed(count: usize, anchored: bool) -> Work {
    let mut positioned = Vec::new();
    let (mut document, cascade, root) =
        ahem_paragraph_with("position:relative;width:800px", |doc, root| {
            if !anchored {
                doc.append_text(root, "x");
            }
            for index in 0..count {
                if anchored {
                    doc.append_text(root, "x ");
                } else {
                    let block = doc.append_element(Some(root), "div", Some("display:block"));
                    doc.append_text(block, "x");
                }
                let position = if index % 2 == 0 { "absolute" } else { "fixed" };
                positioned.push(doc.append_element(
                    Some(root),
                    "div",
                    Some(format!(
                        "display:block;position:{position};width:2px;height:2px"
                    )),
                ));
            }
        });
    with_ahem(&mut document);
    reset();
    crate::layout_single_page(&mut document, &cascade, page_box_800x600()).unwrap();
    let work = read();
    assert!(work.calls > 0, "the production placement path must run");
    let lines = document.nodes[root]
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap();
    let anchors = lines
        .lines
        .iter()
        .flat_map(|line| line.fragments())
        .filter(|fragment| matches!(fragment, shodo::Fragment::OutOfFlowAnchor(_)))
        .count();
    if anchored {
        assert_eq!(anchors, count);
    } else {
        assert_eq!(anchors, 0);
    }
    for node in positioned {
        let layout = document.nodes[node].layout;
        assert_eq!((layout.size.width, layout.size.height), (2.0, 2.0));
        assert!(layout.location.x.is_finite() && layout.location.y.is_finite());
    }
    work
}

#[test]
fn production_anchor_search_visits_fragments_once_per_paragraph() {
    let cases: Vec<_> = [16, 32, 64]
        .into_iter()
        .map(|count| {
            let work = placed(count, true);
            eprintln!("anchored count={count} work={work:?}");
            (count, work)
        })
        .collect();
    for (count, work) in cases {
        assert_eq!(work.anchor_indexes, work.calls);
        assert_eq!(work.fallback_indexes, 0);
        assert!(
            work.anchors <= 8 * count * work.calls,
            "anchor search must scale with fragments: count={count}, work={work:?}"
        );
    }
}

#[test]
fn production_missing_anchors_use_one_box_and_block_index() {
    let cases: Vec<_> = [16, 32, 64]
        .into_iter()
        .map(|count| {
            let work = placed(count, false);
            eprintln!("absent count={count} work={work:?}");
            (count, work)
        })
        .collect();
    for (count, work) in cases {
        assert_eq!(work.anchor_indexes, work.calls);
        assert_eq!(work.fallback_indexes, work.calls);
        assert!(
            work.boxes + work.metadata <= 16 * count * work.calls,
            "missing-anchor fallback must scale with boxes and block metadata: count={count}, work={work:?}"
        );
    }
}

#[test]
fn one_out_of_flow_box_keeps_the_original_scan_without_indexes() {
    for anchored in [true, false] {
        let work = placed(1, anchored);
        assert_eq!(work.anchor_indexes, 0);
        assert_eq!(work.fallback_indexes, 0);
    }
}

#[test]
fn one_early_anchor_does_not_index_a_long_paragraph() {
    let (mut document, cascade, _) =
        ahem_paragraph_with("position:relative;width:800px", |doc, root| {
            doc.append_element(
                Some(root),
                "div",
                Some("display:block;position:absolute;width:2px;height:2px"),
            );
            doc.append_text(root, "x ".repeat(4096));
        });
    with_ahem(&mut document);
    reset();
    crate::layout_single_page(&mut document, &cascade, page_box_800x600()).unwrap();
    let work = read();
    assert_eq!(work.anchor_indexes, 0);
    assert_eq!(work.fallback_indexes, 0);
    assert!(
        work.anchors <= 2 * work.calls,
        "first anchor should terminate the scan: {work:?}"
    );
}

fn lines_fixture() -> IfcLines {
    let (mut document, cascade, root) =
        crate::layout::test_support::ahem_paragraph("x\nx\nx\nx", "position:relative;width:100px");
    with_ahem(&mut document);
    crate::layout_single_page(&mut document, &cascade, page_box_800x600()).unwrap();
    document.nodes[root]
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap()
        .clone()
}

fn compare_fallbacks(lines: &IfcLines, boxes: &[IfcBox]) {
    let index = StaticFallbackIndex::new(lines, boxes);
    let order: Vec<_> = boxes.iter().map(|box_| (box_.node, box_.kind)).collect();
    for node in boxes.iter().map(|box_| box_.node).chain([usize::MAX]) {
        assert_eq!(
            index.offset(node).to_bits(),
            static_block_offset(lines, &order, node, None, false).to_bits(),
            "node {node}"
        );
    }
}

#[test]
fn indexed_fallback_preserves_duplicates_missing_nodes_and_source_order() {
    let mut lines = lines_fixture();
    lines.block_ends = vec![(10, 1, 11.5), (20, 2, 22.5), (10, 0, 99.0), (30, 3, 33.5)];
    lines.block_line_starts = vec![(10, 1), (20, 2), (10, 0), (30, 3)];
    lines.generated_block_starts = vec![(2, 2), (0, 0), (4, 1), (3, 3), (usize::MAX, 0)];
    lines.shifts.push(1, 3.25);
    lines.shifts.push(3, -1.5);
    let boxes: Vec<_> = [
        (7, IfcBoxKind::OutOfFlow),
        (10, IfcBoxKind::Block),
        (8, IfcBoxKind::OutOfFlow),
        (8, IfcBoxKind::Block),
        (8, IfcBoxKind::OutOfFlow),
        (20, IfcBoxKind::Block),
        (9, IfcBoxKind::OutOfFlow),
        (10, IfcBoxKind::Atomic),
        (10, IfcBoxKind::Block),
        (11, IfcBoxKind::OutOfFlow),
        (30, IfcBoxKind::Block),
    ]
    .into_iter()
    .map(|(node, kind)| IfcBox { node, kind })
    .collect();
    compare_fallbacks(&lines, &boxes);
    lines.block_line_starts.retain(|&(node, _)| node != 10);
    compare_fallbacks(&lines, &boxes);
    lines.generated_block_starts.clear();
    compare_fallbacks(&lines, &boxes);
    lines.block_ends.clear();
    compare_fallbacks(&lines, &boxes);
    lines.lines = std::sync::Arc::new(Vec::new());
    compare_fallbacks(&lines, &boxes);
    compare_fallbacks(&lines, &[]);
}

#[test]
fn first_anchor_index_preserves_duplicate_line_and_fragment_order() {
    let mut lines = lines_fixture();
    let limits = shodo::limits::Limits::default();
    let mut builder =
        shodo::ParagraphBuilder::new(&shodo::style::ParagraphStyle::default(), &limits);
    for _ in 0..2 {
        builder.push_text(
            shodo::node::TextSource::Generated {
                node: shodo::node::NodeId(1),
            },
            "x ",
        );
        builder.push_out_of_flow(shodo::node::NodeId(7), shodo::node::OutOfFlowKind::Absolute);
    }
    builder.push_text(
        shodo::node::TextSource::Generated {
            node: shodo::node::NodeId(1),
        },
        "x",
    );
    let mut context = shodo::LayoutContext::new();
    let paragraph = builder
        .build(&mut context, &shodo::font::FontCollection::new(&limits))
        .unwrap();
    let shodo::LineResult::Line(line) = paragraph.next_line(
        &mut context,
        paragraph.start_token(),
        &shodo::style::LineOptions::default(),
        &shodo::LineConstraint::new(1000.0),
        &shodo::AtomicSizes::EMPTY,
    ) else {
        panic!("expected a line")
    };
    let positions: Vec<_> = line
        .fragments()
        .filter_map(|fragment| match fragment {
            shodo::Fragment::OutOfFlowAnchor(anchor) => Some(anchor.inline_position),
            _ => None,
        })
        .collect();
    assert_eq!(positions.len(), 2);
    assert_ne!(positions[0], positions[1]);
    lines.lines = std::sync::Arc::new(vec![line.clone(), line]);
    let anchors = first_anchors(&lines);
    for (&node, &anchor) in &anchors {
        assert_eq!(Some(anchor), find_oof_anchor(&lines, node));
    }
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors.values().next().unwrap().0, 0);
}

#[test]
fn empty_lines_with_generated_blocks_keep_the_content_end_fallback() {
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
        "div::before {display:block;content:'x'}",
        "position:relative;width:100px",
        |doc, root| {
            doc.append_text(root, "x");
        },
    );
    with_ahem(&mut fixture.doc);
    crate::layout_single_page(&mut fixture.doc, &fixture.cascade, page_box_800x600()).unwrap();
    let mut lines = fixture.doc.nodes[fixture.root]
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap()
        .clone();
    assert!(!lines.placed_blocks.is_empty());
    lines.lines = std::sync::Arc::new(Vec::new());
    lines.height = 17.0;
    let boxes = [IfcBox {
        node: 7,
        kind: IfcBoxKind::OutOfFlow,
    }];
    compare_fallbacks(&lines, &boxes);
    assert_eq!(StaticFallbackIndex::new(&lines, &boxes).offset(7), 17.0);
}

#[test]
#[ignore = "manual diagnostic; defaults to a tiny fixture"]
fn probe_oof_search_operations() {
    let count = std::env::var("RAIKIRI_OOF_PROBE_COUNT")
        .ok()
        .map(|value| value.parse::<usize>().expect("positive integer"))
        .unwrap_or(16);
    assert!(
        (1..=2048).contains(&count),
        "probe count must be in 1..=2048"
    );
    for anchored in [true, false] {
        let work = placed(count, anchored);
        println!("OOF_WORK count={count} anchored={anchored} {work:?}");
    }
}
