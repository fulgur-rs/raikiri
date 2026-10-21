use super::*;
use crate::layout::ifc::test_support::block_fixture;

#[test]
fn contents_parent_offsets_exclude_the_child_and_visited_parent() {
    let mut ids = [0; 3];
    let mut fixture = block_fixture("", |doc, root| {
        ids[0] = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        ids[1] = doc.append_element(
            Some(ids[0]),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        ids[2] = doc.append_element(Some(ids[1]), "div", Style::default(), Some("display:block"));
    });
    fixture.doc.nodes[fixture.root].unrounded_layout.location.y = 100.0;
    fixture.doc.nodes[ids[0]].unrounded_layout.location.y = 11.0;
    fixture.doc.nodes[ids[1]].unrounded_layout.location.y = 13.0;
    fixture.doc.nodes[ids[2]].unrounded_layout.location.y = 200.0;
    assert_eq!(
        pagination_child_parent_y(&fixture.doc, fixture.root, ids[2], 7.0),
        31.0
    );
}

#[test]
fn resolved_no_column_cache_hit_stops_before_parent_lookup_and_path_compression() {
    let mut ids = [0; 2];
    let fixture = block_fixture("display:flex;flex-direction:column", |doc, root| {
        ids[0] = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        ids[1] = doc.append_element(Some(ids[0]), "div", Style::default(), Some("display:block"));
    });
    let mut columns = ColumnContexts::new(fixture.doc.node_count());
    columns.cache[ids[1]] = Some(None);
    assert_eq!(
        columns.enclosing(ids[1], &fixture.doc, &fixture.cascade),
        None
    );
    assert_eq!(columns.cache[ids[0]], None);
    columns.cache[ids[1]] = None;
    assert_eq!(
        columns.enclosing(ids[1], &fixture.doc, &fixture.cascade),
        Some(fixture.root)
    );
    assert_eq!(columns.cache[ids[1]], Some(Some(fixture.root)));
    assert_eq!(columns.cache[ids[0]], Some(Some(fixture.root)));
}

#[test]
fn row_leads_use_the_nearest_table_without_registering_its_boundary() {
    let mut ids = [0; 3];
    let fixture = block_fixture("", |doc, root| {
        ids[0] = doc.append_element(Some(root), "table", Style::default(), Some("display:table"));
        ids[1] = doc.append_element(
            Some(ids[0]),
            "tbody",
            Style::default(),
            Some("display:table-row-group"),
        );
        ids[2] = doc.append_element(
            Some(ids[1]),
            "tr",
            Style::default(),
            Some("display:table-row"),
        );
        doc.append_element(
            Some(ids[2]),
            "td",
            Style::default(),
            Some("display:table-cell"),
        );
    });
    let candidate = |node_id, raw_y, is_table_row| PageCandidate {
        node_id,
        raw_y,
        height: 10.0,
        context: element_candidate_context(candidates::ElementContext {
            is_table_row,
            ..Default::default()
        }),
        page_name: None,
        named_flex_context: None,
        ifc_root: None,
    };
    let leads = table_row_leads(
        &fixture.doc,
        &fixture.cascade,
        &[
            candidate(ids[0], 20.0, false),
            candidate(ids[2], 27.0, true),
        ],
        0.0,
    );
    assert_eq!(leads.len(), 1);
    assert_eq!(leads.get(&ids[2]), Some(&7.0));
    assert!(!leads.contains_key(&ids[0]));
    assert!(!leads.contains_key(&ids[1]));
}
