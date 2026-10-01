use super::*;
use crate::layout::ifc::projection::project_ifc;
use crate::layout::ifc::test_support::{ahem_fonts, block_fixture, span};
use shodo::LayoutContext;
use shodo::limits::Limits;

#[test]
fn the_inline_elements_leave_out_the_boxes_and_their_content() {
    let mut ids = (0, 0, 0, 0);
    let fixture = block_fixture("", |doc, root| {
        let outer = span(doc, root, "display:inline");
        doc.append_text(outer, "a");
        let inner = span(doc, outer, "display:inline");
        doc.append_text(inner, "b");
        let atomic = span(doc, root, "display:inline-block");
        let inside_atomic = span(doc, atomic, "display:inline");
        doc.append_text(inside_atomic, "c");
        ids = (outer, inner, atomic, inside_atomic);
    });
    let projected = project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("project");
    let ifc = IfcRoot::new(projected);
    assert_eq!(ifc.boxes.len(), 1);
    assert_eq!(
        inline_elements(&fixture.doc, fixture.root, &ifc),
        [ids.0, ids.1]
    );
}

#[test]
fn the_inline_elements_leave_out_detached_nodes() {
    let mut kept = 0;
    let mut fixture = block_fixture("", |doc, root| {
        kept = span(doc, root, "display:inline");
        doc.append_text(kept, "a");
    });
    let detached = span(&mut fixture.doc, fixture.root, "display:inline");
    fixture.doc.nodes[detached]
        .flags
        .remove(crate::node::NodeFlags::IS_IN_DOCUMENT);
    let projected = project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("project");
    assert!(!fixture.doc.nodes[detached].is_in_document());
    let ifc = IfcRoot::new(projected);
    assert_eq!(inline_elements(&fixture.doc, fixture.root, &ifc), [kept]);
}
