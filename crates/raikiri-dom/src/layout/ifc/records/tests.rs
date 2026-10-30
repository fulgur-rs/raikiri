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

fn projected_root(fixture: &crate::layout::ifc::test_support::Fixture) -> IfcRoot {
    IfcRoot::new(
        project_ifc(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &mut LayoutContext::new(),
            &ahem_fonts(),
            &Limits::default(),
        )
        .expect("project"),
    )
}

fn reset_membership_work() {
    crate::layout::test_support::IFC_INLINE_ELEMENT_BOX_VISITS.with(|count| count.set(0));
    crate::layout::test_support::IFC_INLINE_ELEMENT_LOOKUPS.with(|count| count.set(0));
}
fn membership_work() -> (usize, usize) {
    (
        crate::layout::test_support::IFC_INLINE_ELEMENT_BOX_VISITS.with(std::cell::Cell::get),
        crate::layout::test_support::IFC_INLINE_ELEMENT_LOOKUPS.with(std::cell::Cell::get),
    )
}

#[test]
fn mixed_elements_build_atomic_membership_once_and_prune_atomic_descendants() {
    for count in [16, 64, 256, 1024] {
        let mut expected = Vec::new();
        let fixture = block_fixture("", |doc, root| {
            for _ in 0..count {
                let normal = span(doc, root, "display:inline");
                doc.append_text(normal, "a");
                expected.push(normal);
                let atomic = span(doc, root, "display:inline-block;width:1px;height:1px");
                let child = span(doc, atomic, "display:inline");
                doc.append_text(child, "b");
            }
        });
        let ifc = projected_root(&fixture);
        assert_eq!(ifc.boxes.len(), count);
        reset_membership_work();
        assert_eq!(inline_elements(&fixture.doc, fixture.root, &ifc), expected);
        let (box_visits, lookups) = membership_work();
        assert_eq!(
            lookups,
            2 * count,
            "one lookup per reached in-document element"
        );
        assert_eq!(
            box_visits, count,
            "scan boxes once for {count} atomics, got {box_visits} visits"
        );
    }
}

#[test]
fn a_paragraph_without_boxes_performs_no_membership_build_visits() {
    let mut expected = Vec::new();
    let fixture = block_fixture("", |doc, root| {
        for _ in 0..16 {
            let normal = span(doc, root, "display:inline");
            doc.append_text(normal, "a");
            expected.push(normal);
        }
    });
    let ifc = projected_root(&fixture);
    assert!(ifc.boxes.is_empty());
    reset_membership_work();
    assert_eq!(inline_elements(&fixture.doc, fixture.root, &ifc), expected);
    assert_eq!(membership_work(), (0, 16));
}

#[test]
fn membership_covers_all_box_kinds_duplicates_and_a_root_entry() {
    use crate::layout::ifc::boxes::{IfcBox, IfcBoxKind};
    let (mut kept, mut pruned) = (0, 0);
    let fixture = block_fixture("", |doc, root| {
        kept = span(doc, root, "display:inline");
        doc.append_text(kept, "a");
        pruned = span(doc, root, "display:inline");
        let child = span(doc, pruned, "display:inline");
        doc.append_text(child, "b");
    });
    let mut ifc = projected_root(&fixture);
    assert!(ifc.boxes.is_empty());
    for kind in [
        IfcBoxKind::Atomic,
        IfcBoxKind::Float,
        IfcBoxKind::Block,
        IfcBoxKind::OutOfFlow,
    ] {
        ifc.boxes.push(IfcBox { node: pruned, kind });
    }
    ifc.boxes.push(IfcBox {
        node: fixture.root,
        kind: IfcBoxKind::Atomic,
    });
    reset_membership_work();
    assert_eq!(inline_elements(&fixture.doc, fixture.root, &ifc), [kept]);
    assert_eq!(membership_work(), (5, 2));
}
