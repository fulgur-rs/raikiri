use super::*;
use crate::layout::ifc::test_support::block_fixture_with_ahem;

#[test]
fn selecting_a_column_does_not_clone_projection_records() {
    let fixture = block_fixture_with_ahem("width:10px;line-height:10px", |doc, root| {
        doc.append_text(root, "A B C D");
    });
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    crate::layout_single_page(
        &mut doc,
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let control = crate::PageLayoutControl::default();
    let mut work = ProjectionWork::new(&control, 100_000);
    let prepared = ParagraphProjection::prepare(&doc, &cascade, root, 0, &mut work).unwrap();
    assert!(prepared.combined.record_count > 0);
    crate::page_projection::work_tests::reset_projection_clones();
    for column in [2, 3, 2] {
        let selected = column_data(&prepared, Some(column), true, &mut work).unwrap();
        assert!(Arc::ptr_eq(&selected.data, &prepared.combined));
        assert_eq!(selected.column_override, Some(column as u32));
    }
    assert_eq!(crate::page_projection::work_tests::projection_clones(), 0);
}

#[test]
fn a_text_node_that_is_its_own_ifc_root_keeps_prepared_lines() {
    for display in ["flex", "grid"] {
        let fixture =
            block_fixture_with_ahem(&format!("display:{display};width:100px"), |doc, root| {
                doc.append_text(root, "A B");
            });
        let (mut doc, cascade, container) = (fixture.doc, fixture.cascade, fixture.root);
        crate::layout_single_page(
            &mut doc,
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let text = doc.get_node(container).unwrap().children[0];
        let node = doc.get_node(text).unwrap();
        assert!(node.is_ifc_root());
        let control = crate::PageLayoutControl::default();
        let mut work = ProjectionWork::new(&control, 100_000);
        let mut paragraphs = Paragraphs::default();
        paragraphs
            .take_column(&doc, &cascade, text, None, false, &mut work)
            .unwrap();
        let groups = paragraphs.take_text_lines(&doc, text, node).unwrap();
        let groups = groups.groups().unwrap();
        assert!(!groups.is_empty());
        assert!(
            groups
                .iter()
                .all(|(_, _, _, lines)| lines.root == text && !lines.lines.is_empty())
        );
        let walked = walk_sources(&doc, &cascade, container, &mut work).unwrap();
        let sources: Vec<_> = walked
            .nodes
            .iter()
            .filter(|source| source.node_id.0 == text as u64)
            .collect();
        assert!(!sources.is_empty());
        assert!(sources.iter().all(|source| {
            source
                .line_metrics
                .as_ref()
                .is_some_and(|lines| !lines.is_empty())
        }));
    }
}
