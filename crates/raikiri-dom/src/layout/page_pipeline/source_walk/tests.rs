use super::*;
use crate::layout::ifc::test_support::block_fixture_with_ahem;

#[test]
fn selecting_a_column_shares_the_prepared_projection() {
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
    for column in [2, 3, 2] {
        let selected = column_data(&prepared, Some(column), true, &mut work).unwrap();
        assert!(Arc::ptr_eq(&selected.data, &prepared.combined));
        assert_eq!(selected.column_override, Some(column as u32));
    }
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

#[test]
fn text_in_another_column_is_suppressed_and_consumption_resets_per_visit() {
    let mut second = 0;
    let fixture = block_fixture_with_ahem(
        "width:30px;height:10px;column-count:2;column-gap:10px;column-fill:auto;line-height:10px",
        |doc, root| {
            doc.append_text(root, "A ");
            second = doc.append_text(root, "B");
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    crate::layout_single_page(
        &mut doc,
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let control = crate::PageLayoutControl::default();
    let mut work = ProjectionWork::new(&control, 100_000);
    let mut paragraphs = Paragraphs::default();
    paragraphs
        .take_column(&doc, &cascade, root, Some(0), false, &mut work)
        .unwrap();
    let node = doc.get_node(second).unwrap();
    assert!(
        paragraphs
            .take_text_lines(&doc, second, node)
            .unwrap()
            .groups()
            .is_none()
    );
    paragraphs
        .take_column(&doc, &cascade, root, Some(1), false, &mut work)
        .unwrap();
    assert!(
        paragraphs
            .take_text_lines(&doc, second, node)
            .unwrap()
            .groups()
            .is_some_and(|groups| !groups.is_empty())
    );
    assert!(
        paragraphs
            .take_text_lines(&doc, second, node)
            .unwrap()
            .groups()
            .is_none()
    );
    paragraphs
        .take_column(&doc, &cascade, root, Some(1), false, &mut work)
        .unwrap();
    assert!(
        paragraphs
            .take_text_lines(&doc, second, node)
            .unwrap()
            .groups()
            .is_some_and(|groups| !groups.is_empty())
    );
}

#[test]
fn a_cached_owner_without_an_active_column_remains_suppressed() {
    let mut text = 0;
    let fixture = block_fixture_with_ahem("width:100px", |doc, root| {
        text = doc.append_text(root, "X");
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
    let mut paragraphs = Paragraphs::default();
    paragraphs
        .take_column(&doc, &cascade, root, None, false, &mut work)
        .unwrap();
    let active = paragraphs.selected.remove(&root).unwrap();
    assert!(active.view.data.owners.contains_key(&text));
    let hidden = paragraphs
        .take_text_lines(&doc, text, doc.get_node(text).unwrap())
        .unwrap();
    assert!(hidden.groups().is_none());
    assert!(Arc::ptr_eq(&hidden.data, &paragraphs.cache[&root].combined));
}
