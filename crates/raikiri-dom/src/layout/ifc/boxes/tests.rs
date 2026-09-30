use super::*;
use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::test_support::{ahem_paragraph_with_float, ifc_ahem_fonts, line_start_x};

#[test]
fn a_probe_stores_nothing_and_a_performed_layout_does() {
    let (mut doc, cascade, float, root) =
        ahem_paragraph_with_float("aa", "float:left;width:30px;height:20px", " bbbb", "");
    doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        &mut doc,
        &cascade,
        crate::layout::test_support::page_box_800x600(),
        crate::layout::test_support::ahem_font_context(),
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
