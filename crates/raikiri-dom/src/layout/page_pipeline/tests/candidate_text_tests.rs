use super::*;
use crate::layout::test_support::TEXT_LINE_VISITS;

#[test]
fn nodes_without_shaped_paragraphs_do_not_publish_owners() {
    let (mut doc, _, root) = crate::layout::test_support::ahem_paragraph("A", "");
    let source = doc.nodes[root].children[0];
    let mut index = TextLineIndex::default();
    for node in [doc.nodes.len(), root, source] {
        index
            .prepare_for(&doc, node, || {
                panic!("unshaped nodes have no lines to scan")
            })
            .unwrap();
        assert!(index.get(node).is_none());
    }

    // A text node can have no paragraph root before shaping is complete.
    doc.nodes[source].flags.insert(NodeFlags::IN_IFC_SUBTREE);
    index
        .prepare_for(&doc, source, || {
            panic!("a missing root has no lines to scan")
        })
        .unwrap();
    assert!(index.get(source).is_none());
    assert!(doc.ifc_text_lines_by_node(root).is_empty());
}

#[test]
fn candidate_text_line_scans_grow_with_lines_instead_of_text_owners() {
    for count in [32, 64, 128] {
        let (mut doc, cascade, _) = ahem_paragraph_with("width:40px", |doc, root| {
            for index in 0..count {
                if index > 0 {
                    doc.append_element(Some(root), "br", Style::default(), Some("display:inline"));
                }
                let span = doc.append_element(
                    Some(root),
                    "span",
                    Style::default(),
                    Some("display:inline"),
                );
                doc.append_text(span, "A");
            }
        });
        let mut page = page_box_800x600();
        page.height = 40.0;
        TEXT_LINE_VISITS.with(|visits| visits.set(0));
        let pages = layout_pages(with_ahem(&mut doc), &cascade, page).unwrap();
        let visits = TEXT_LINE_VISITS.with(|visits| visits.get());
        eprintln!("{count} lines: {visits} ownership line visits");
        assert_eq!(pages.len(), count / 4);
        assert!(
            visits <= count * 8,
            "{count} lines required {visits} ownership line visits"
        );
    }
}

#[test]
fn indexed_owners_keep_shared_lines_and_read_current_pagination_shifts() {
    let mut sources = Vec::new();
    let (mut doc, cascade, root) = ahem_paragraph_with("width:40px", |doc, root| {
        sources.push(doc.append_text(root, "   "));
        sources.push(doc.append_text(root, "aa"));
        let span = doc.append_element(Some(root), "span", Style::default(), Some("display:inline"));
        sources.push(doc.append_text(span, "bb cc"));
    });
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    let mut index = TextLineIndex::default();
    for &source in &sources {
        index.prepare_for(&doc, source, || Ok(())).unwrap();
        assert_eq!(index.get(source), doc.ifc_text_lines(source).as_ref());
    }
    assert!(index.get(sources[0]).is_none());
    let owned = index.get(sources[2]).unwrap();
    assert_eq!(
        positioned_owned_text_line_bounds(&doc, owned),
        Some(vec![(0.0, 10.0), (10.0, 20.0)])
    );

    doc.nodes[root]
        .ifc
        .as_mut()
        .unwrap()
        .lines
        .as_mut()
        .unwrap()
        .shifts
        .push((1, 7.0));
    assert_eq!(
        positioned_owned_text_line_bounds(&doc, owned),
        Some(vec![(0.0, 10.0), (17.0, 27.0)])
    );
    assert_eq!(
        positioned_owned_text_line_bounds(&doc, owned),
        positioned_text_line_bounds(&doc, sources[2])
    );

    doc.nodes[root].ifc.as_mut().unwrap().multicol_fragments = Some(vec![
        crate::node::MulticolTextFragment {
            line_start: 0,
            line_end: 1,
            fragmentainer: 0,
            x: 0.0,
            y: 50.0,
        },
        crate::node::MulticolTextFragment {
            line_start: 1,
            line_end: 2,
            fragmentainer: 1,
            x: 60.0,
            y: 20.0,
        },
    ]);
    assert_eq!(
        positioned_owned_text_line_bounds(&doc, owned),
        Some(vec![(50.0, 60.0), (27.0, 37.0)])
    );
}

#[test]
fn a_new_pagination_pass_indexes_rebroken_lines() {
    let (mut doc, cascade, root) =
        crate::layout::test_support::ahem_paragraph("aaaa bbbb cccc", "");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    let source = doc.nodes[root].children[0];
    let mut original = TextLineIndex::default();
    original.prepare_for(&doc, source, || Ok(())).unwrap();
    assert_eq!(original.get(source).unwrap().lines.len(), 1);

    relayout_text_for_width(&mut doc, &cascade, 50.0);
    let mut rebroken = TextLineIndex::default();
    rebroken.prepare_for(&doc, source, || Ok(())).unwrap();
    assert_eq!(rebroken.get(source), doc.ifc_text_lines(source).as_ref());
    assert_eq!(rebroken.get(source).unwrap().lines.len(), 3);
    assert_eq!(rebroken.get(source).unwrap().width, 50.0);
}

#[test]
fn interrupted_owner_preparation_can_retry_without_partial_owners() {
    let (mut doc, cascade, root) =
        crate::layout::test_support::ahem_paragraph("A\nB\nC", "width:40px");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    let source = doc.nodes[root].children[0];
    let mut index = TextLineIndex::default();
    let mut checks = 0;
    let result = index.prepare_for(&doc, source, || {
        checks += 1;
        if checks == 3 {
            Err(LayoutError::Aborted)
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(LayoutError::Aborted)));
    assert!(index.get(source).is_none());
    index.prepare_for(&doc, source, || Ok(())).unwrap();
    assert_eq!(index.get(source), doc.ifc_text_lines(source).as_ref());
}

#[test]
fn indexed_owners_keep_anonymous_flex_grid_and_table_roots() {
    for css in [
        "display:flex;flex-direction:column;width:40px",
        "display:grid;grid-template-columns:40px",
    ] {
        let (mut doc, cascade, container) =
            crate::layout::test_support::ahem_paragraph("aaaa bbbb", css);
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        let source = doc.nodes[container].children[0];
        let mut index = TextLineIndex::default();
        index.prepare_for(&doc, source, || Ok(())).unwrap();
        let owned = index.get(source).unwrap();
        assert_eq!(owned.root, source);
        assert_eq!(owned.lines.len(), 2);
        assert_eq!(Some(owned), doc.ifc_text_lines(source).as_ref());
    }

    let (mut doc, cascade, table) =
        crate::layout::test_support::ahem_table_with_only_text("width:40px", "aaaa bbbb");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    let source = doc.nodes[table].children[0];
    let mut index = TextLineIndex::default();
    index.prepare_for(&doc, source, || Ok(())).unwrap();
    let owned = index.get(source).unwrap();
    assert!(owned.root >= doc.nodes.len());
    assert_eq!(owned.lines.len(), 2);
    assert_eq!(Some(owned), doc.ifc_text_lines(source).as_ref());
}

#[test]
fn an_atomic_only_line_has_no_indexed_text_owner() {
    let mut sources = Vec::new();
    let (mut doc, cascade, root) = ahem_paragraph_with("width:40px", |doc, root| {
        sources.push(doc.append_text(root, "A"));
        doc.append_element(Some(root), "br", Style::default(), Some("display:inline"));
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline-block;width:10px;height:10px"),
        );
        doc.append_element(Some(root), "br", Style::default(), Some("display:inline"));
        sources.push(doc.append_text(root, "B"));
    });
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    assert_eq!(
        doc.nodes[root]
            .ifc
            .as_ref()
            .unwrap()
            .lines
            .as_ref()
            .unwrap()
            .lines
            .len(),
        3
    );
    let mut index = TextLineIndex::default();
    for &source in &sources {
        index.prepare_for(&doc, source, || Ok(())).unwrap();
    }
    assert_eq!(
        index
            .get(sources[0])
            .unwrap()
            .lines
            .iter()
            .map(|line| line.line)
            .collect::<Vec<_>>(),
        [0]
    );
    assert_eq!(
        index
            .get(sources[1])
            .unwrap()
            .lines
            .iter()
            .map(|line| line.line)
            .collect::<Vec<_>>(),
        [2]
    );
}
