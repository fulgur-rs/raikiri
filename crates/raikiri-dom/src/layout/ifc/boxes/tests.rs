use super::*;
use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::test_support::with_ahem;
use crate::layout::test_support::{
    ahem_paragraph, ahem_paragraph_with_atomic, ahem_paragraph_with_float, ifc_ahem_fonts,
    line_start_x,
};

#[test]
fn unprojected_root_has_no_fragmentainer_tail() {
    let mut doc = Document::new();
    let geometry = FlowGeometry {
        width: 100.0,
        edges: (0.0, 0.0),
        top_edge: 0.0,
    };
    let ordinary = layout_with_boxes_in(&mut doc, 0, geometry, None, false, false);
    assert!(ordinary.lines.is_empty());
    assert_eq!(ordinary.unfragmented_tail_column, None);
    let fragmented = layout_with_boxes_in_fragmentainers(
        &mut doc,
        0,
        geometry,
        FragmentationContext {
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
    let geometry = FlowGeometry {
        width: 100.0,
        edges: (0.0, 0.0),
        top_edge: 0.0,
    };
    let context = FragmentationContext {
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
    doc.nodes[float].unrounded_layout = taffy::Layout::new();
    let geometry = FlowGeometry {
        width: 100.0,
        edges: (0.0, 0.0),
        top_edge: 0.0,
    };

    let probed = layout_with_boxes(&mut doc, root, geometry, None, false);
    assert_eq!(
        doc.nodes[float].unrounded_layout.size.width, 0.0,
        "a probe must not store the float's layout"
    );
    // The float still shortens the first line: the probe sees it.
    assert_eq!(line_start_x(&probed.lines[0]), Some(30.0));

    layout_with_boxes(&mut doc, root, geometry, None, true);
    let layout = doc.nodes[float].unrounded_layout;
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
    doc.nodes[atomic].unrounded_layout = taffy::Layout::new();
    let geometry = FlowGeometry {
        width: 100.0,
        edges: (0.0, 0.0),
        top_edge: 0.0,
    };

    layout_with_boxes(&mut doc, root, geometry, None, false);
    assert_eq!(
        doc.nodes[atomic].unrounded_layout.size.width, 0.0,
        "a probe must not store the atomic's layout"
    );

    layout_with_boxes(&mut doc, root, geometry, None, true);
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 10.0));
}

#[test]
fn a_probe_does_not_record_an_inline_element_and_a_performed_layout_does() {
    let mut span_id = 0;
    let (mut doc, cascade, root) =
        crate::layout::test_support::ahem_paragraph_with("width:100px", |doc, root| {
            doc.append_text(root, "aa");
            span_id = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline;padding:0 3px"),
            );
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
    doc.nodes[span_id].unrounded_layout = taffy::Layout::new();
    let geometry = FlowGeometry {
        width: 100.0,
        edges: (0.0, 0.0),
        top_edge: 0.0,
    };

    layout_with_boxes(&mut doc, root, geometry, None, false);
    assert_eq!(
        doc.nodes[span_id].unrounded_layout.size.width, 0.0,
        "a probe must not record the element's box"
    );

    layout_with_boxes(&mut doc, root, geometry, None, true);
    let layout = doc.nodes[span_id].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (20.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (26.0, 10.0));
}
