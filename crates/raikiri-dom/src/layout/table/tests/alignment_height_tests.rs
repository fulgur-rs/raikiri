use crate::layout::test_support::{
    ahem_paragraph, page_box_800x600, with_ahem, with_resumed_session,
};

#[test]
fn fragmented_ifc_extent_is_not_a_cell_alignment_height() {
    let (mut doc, cascade, root) =
        ahem_paragraph("a\nb", "width:40px;padding:3px;border:2px solid");
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    doc.nodes[root].auto_block_size = None;
    with_resumed_session(&mut doc, |session| {
        assert_eq!(
            super::super::horizontal_cell_alignment_height(session, root, 50.0),
            Some(30.0)
        );
    });

    // A retained line height spans all fragments, not the cell's used extent.
    let lines = doc.nodes[root]
        .ifc
        .as_mut()
        .and_then(|ifc| ifc.lines.as_mut())
        .expect("retained lines");
    lines.fragmentainer_line_ranges = Some(vec![crate::node::MulticolTextFragment {
        line_start: 0,
        line_end: 1,
        fragmentainer: 0,
        x: 0.0,
        y: 0.0,
    }]);
    with_resumed_session(&mut doc, |session| {
        assert_eq!(
            super::super::horizontal_cell_alignment_height(session, root, 50.0),
            None
        );
    });

    // Block flow's explicit alignment size remains authoritative.
    doc.nodes[root].auto_block_size = Some(15.0);
    with_resumed_session(&mut doc, |session| {
        assert_eq!(
            super::super::horizontal_cell_alignment_height(session, root, 50.0),
            Some(15.0)
        );
    });
}
