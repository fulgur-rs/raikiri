use super::*;
use crate::Document;
pub(super) mod clear_break_tests;
mod group_align_tests;
mod intrinsic_block_size_tests;
mod oof_index_tests;
mod orthogonal_baseline_tests;
mod percent_edge_tests;
mod rtl_margin_tests;
mod static_position_tests;
mod vertical_atomic_tests;
mod vertical_bfc_tests;
thread_local! {
    pub(super) static OOF_ANCHOR_INDEX_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static OOF_FALLBACK_INDEX_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static OOF_ANCHOR_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static OOF_BOX_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static OOF_METADATA_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static OOF_PLACE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::test_support::{
    absolute_rect, ahem_paragraph, ahem_paragraph_with_atomic, ahem_paragraph_with_float,
    ifc_ahem_fonts, line_start_x,
};
use crate::layout::test_support::{with_ahem, with_bridged_session};

pub(super) fn layout_with_boxes(
    tree: &mut Document,
    idx: usize,
    geometry: FlowGeometry,
    block_ctx: Option<&mut BlockContext<'_>>,
    perform: bool,
) -> IfcLines {
    with_bridged_session(tree, |session| {
        super::layout_with_boxes(session, idx, geometry, block_ctx, perform)
    })
}

pub(super) fn layout_with_boxes_in(
    tree: &mut Document,
    idx: usize,
    geometry: FlowGeometry,
    block_ctx: Option<&mut BlockContext<'_>>,
    perform: bool,
    top_margin_escapes: bool,
    bottom_margin_escapes: bool,
) -> IfcLines {
    with_bridged_session(tree, |session| {
        super::layout_with_boxes_in(
            session,
            idx,
            geometry,
            block_ctx,
            perform,
            top_margin_escapes,
            bottom_margin_escapes,
        )
    })
}

pub(super) fn layout_with_boxes_in_fragmentainers(
    tree: &mut Document,
    idx: usize,
    geometry: FlowGeometry,
    context: crate::fragment::FragmentationContext,
    perform: bool,
) -> IfcLines {
    with_bridged_session(tree, |session| {
        super::layout_with_boxes_in_fragmentainers(session, idx, geometry, context, perform)
    })
}

#[test]
fn many_atomics_keep_placement_and_visibility_work_linear() {
    const ATOMIC_COUNT: usize = 64;
    let mut atomics = Vec::with_capacity(ATOMIC_COUNT);
    let (mut doc, cascade, _root) =
        crate::layout::test_support::ahem_paragraph_with("width:800px", |doc, root| {
            for _ in 0..ATOMIC_COUNT {
                atomics.push(doc.append_element(
                    Some(root),
                    "span",
                    Some("display:inline-block;width:1px;height:10px"),
                ));
            }
        });
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::test_support::IFC_ATOMIC_OUTPUT_LOOKUP_STEPS.with(|steps| steps.set(0));
    crate::layout::test_support::IFC_ATOMIC_VISIBILITY_STEPS.with(|steps| steps.set(0));

    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");

    for pair in atomics.windows(2) {
        let left = doc.nodes[pair[0]].layout;
        let right = doc.nodes[pair[1]].layout;
        assert_eq!(left.size.width, 1.0);
        assert_eq!(right.location.x - left.location.x, 1.0);
    }
    for &atomic in &atomics {
        assert!(
            !doc.nodes[atomic]
                .flags
                .contains(crate::node::NodeFlags::HIDDEN_BY_TEXT_OVERFLOW)
        );
    }

    let placement_steps =
        crate::layout::test_support::IFC_ATOMIC_OUTPUT_LOOKUP_STEPS.with(std::cell::Cell::get);
    let visibility_steps =
        crate::layout::test_support::IFC_ATOMIC_VISIBILITY_STEPS.with(std::cell::Cell::get);
    assert!(
        placement_steps <= ATOMIC_COUNT * 8 && visibility_steps <= ATOMIC_COUNT * 8,
        "atomic placement did {placement_steps} steps and visibility did {visibility_steps} steps for {ATOMIC_COUNT} atomics"
    );
}

#[test]
fn text_only_layout_skips_atomic_visibility_indexing() {
    let (mut doc, cascade, _root) =
        crate::layout::test_support::ahem_paragraph_with("width:100px", |doc, root| {
            doc.append_text(root, "ordinary paragraph text");
        });
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::test_support::IFC_ATOMIC_VISIBILITY_STEPS.with(|steps| steps.set(0));

    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");

    let visibility_steps =
        crate::layout::test_support::IFC_ATOMIC_VISIBILITY_STEPS.with(std::cell::Cell::get);
    assert_eq!(visibility_steps, 0);
}

#[test]
fn ellipsis_hides_only_atomic_fragments_it_removes() {
    let mut atomics = Vec::new();
    let (mut doc, cascade, _root) = crate::layout::test_support::ahem_paragraph_with(
        "width:25px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis",
        |doc, root| {
            for _ in 0..5 {
                atomics.push(doc.append_element(
                    Some(root),
                    "span",
                    Some("display:inline-block;width:10px;height:10px"),
                ));
            }
        },
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());

    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");

    assert!(
        atomics
            .iter()
            .any(|&id| doc.nodes[id].is_hidden_by_text_overflow())
    );
    assert!(
        atomics
            .iter()
            .any(|&id| !doc.nodes[id].is_hidden_by_text_overflow())
    );
}

#[test]
fn unprojected_root_has_no_fragmentainer_tail() {
    let mut doc = Document::new();
    let geometry = FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0);
    let ordinary = layout_with_boxes_in(&mut doc, 0, geometry, None, false, false, false);
    assert!(ordinary.lines.is_empty());
    assert_eq!(ordinary.unfragmented_tail_column, None);
    let fragmented = layout_with_boxes_in_fragmentainers(
        &mut doc,
        0,
        geometry,
        FragmentationContext {
            rtl: false,
            available_width: 100.0,
            available_height: Some(10.0),
            column_fill: raikiri_style::property::ColumnFillValue::Balance,
            column_width: 100.0,
            column_count: 1,
            column_gap: 0.0,
            column_index: 0,
            origin_x: 0.0,
            origin_y: 0.0,
            orphans: 1,
            widows: 1,
        },
        false,
    );
    assert!(fragmented.lines.is_empty());
    assert_eq!(fragmented.unfragmented_tail_column, None);
}

#[test]
fn final_unfragmented_column_records_only_real_line_overflow() {
    let text = "A\n".repeat(1_030);
    let (mut doc, cascade, root) =
        ahem_paragraph(&text, "white-space:pre;font-size:10px;line-height:10px");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");
    let geometry = FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0);
    let context = FragmentationContext {
        rtl: false,
        available_width: 100.0,
        available_height: Some(0.001),
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        column_width: 100.0,
        column_count: 1,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let overflow = layout_with_boxes_in_fragmentainers(&mut doc, root, geometry, context, false);
    assert!(overflow.lines.len() >= 1_030);
    assert_eq!(overflow.unfragmented_tail_column, Some(1_023));
    let fitting = layout_with_boxes_in_fragmentainers(
        &mut doc,
        root,
        geometry,
        FragmentationContext {
            available_height: Some(20.0),
            ..context
        },
        false,
    );
    assert_eq!(fitting.unfragmented_tail_column, None);
}

#[test]
fn a_probe_stores_nothing_and_a_performed_layout_does() {
    let (mut doc, cascade, float, root) =
        ahem_paragraph_with_float("aa", "float:left;width:30px;height:20px", " bbbb", "");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");
    // Forget what the pass stored, then run the two modes by hand.
    doc.nodes[float].layout = BoxLayout::new();
    let geometry = FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0);

    let probed = layout_with_boxes(&mut doc, root, geometry, None, false);
    assert_eq!(
        doc.nodes[float].layout.size.width, 0.0,
        "a probe must not store the float's layout"
    );
    // The float still shortens the first line: the probe sees it.
    assert_eq!(line_start_x(&probed.lines[0]), Some(30.0));

    layout_with_boxes(&mut doc, root, geometry, None, true);
    let layout = doc.nodes[float].layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 20.0));
}

#[test]
fn rtl_lines_use_the_physical_inset_from_a_right_float() {
    let (mut doc, cascade, _, root) = ahem_paragraph_with_float(
        "",
        "float:right;width:30px;height:20px",
        "aaaa bbbb",
        "direction:rtl",
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");

    let line = &doc.nodes[root].ifc_lines().expect("ifc lines")[0];
    assert_eq!(line_start_x(line), Some(70.0));
}

#[test]
fn horizontal_rtl_float_positions_remain_physical() {
    for (float_side, expected_x) in [("left", 0.0), ("right", 90.0)] {
        let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
            "",
            &format!("float:{float_side};width:10px;height:20px"),
            "a",
            "direction:rtl",
        );
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        crate::layout::layout_single_page(
            with_ahem(&mut doc),
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .expect("layout");

        assert_eq!(
            absolute_rect(&doc, float).0 - absolute_rect(&doc, root).0,
            expected_x,
            "float:{float_side}"
        );
    }
}

#[test]
fn a_probe_does_not_place_an_atomic_and_a_performed_layout_does() {
    let (mut doc, cascade, atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:30px;height:10px", " bb", "");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");
    doc.nodes[atomic].layout = BoxLayout::new();
    let geometry = FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0);

    layout_with_boxes(&mut doc, root, geometry, None, false);
    assert_eq!(
        doc.nodes[atomic].layout.size.width, 0.0,
        "a probe must not store the atomic's layout"
    );

    layout_with_boxes(&mut doc, root, geometry, None, true);
    let layout = doc.nodes[atomic].layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 10.0));
}

#[test]
fn a_probe_does_not_record_an_inline_element_and_a_performed_layout_does() {
    let mut span_id = 0;
    let (mut doc, cascade, root) =
        crate::layout::test_support::ahem_paragraph_with("width:100px", |doc, root| {
            doc.append_text(root, "aa");
            span_id = doc.append_element(Some(root), "span", Some("display:inline;padding:0 3px"));
            doc.append_text(span_id, "bb");
        });
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    doc.nodes[span_id].layout = BoxLayout::new();
    let geometry = FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0);

    layout_with_boxes(&mut doc, root, geometry, None, false);
    assert_eq!(
        doc.nodes[span_id].layout.size.width, 0.0,
        "a probe must not record the element's box"
    );

    layout_with_boxes(&mut doc, root, geometry, None, true);
    let layout = doc.nodes[span_id].layout;
    assert_eq!((layout.location.x, layout.location.y), (20.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (26.0, 10.0));
}

/// `div(100px wide) > [float, float, float, "aa"]`: the third float does not
/// fit beside the first two, and nothing of the line comes before it.
fn floats_then_text_positions() -> (Vec<(f32, f32)>, (f32, f32)) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let root = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let floats: Vec<usize> = (0..3)
        .map(|_| {
            doc.append_element(
                Some(root),
                "div",
                Some("display:block;float:left;width:50px;height:20px"),
            )
        })
        .collect();
    doc.append_text(root, "aa");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");
    let root_rect = absolute_rect(&doc, root);
    let positions = floats
        .iter()
        .map(|&float| {
            let rect = absolute_rect(&doc, float);
            (rect.0 - root_rect.0, rect.1 - root_rect.1)
        })
        .collect();
    let line = &doc.nodes[root].ifc_lines().expect("ifc lines")[0];
    (
        positions,
        (line_start_x(line).unwrap(), line.block_offset()),
    )
}

#[test]
fn a_float_that_starts_its_line_goes_below_earlier_floats_before_the_line() {
    // CSS 2.1 9.5.1: the third float is placed below the first two as soon as
    // it is met; the text that follows it in the source wraps beside it.
    let (floats, line) = floats_then_text_positions();
    assert_eq!(floats, [(0.0, 0.0), (50.0, 0.0), (0.0, 20.0)]);
    assert_eq!(line, (50.0, 20.0));
}
