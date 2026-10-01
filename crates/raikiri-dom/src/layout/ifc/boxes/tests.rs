use super::*;
use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::test_support::with_ahem;
use crate::layout::test_support::{
    ahem_paragraph_with_atomic, ahem_paragraph_with_float, ifc_ahem_fonts, line_start_x,
};

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
