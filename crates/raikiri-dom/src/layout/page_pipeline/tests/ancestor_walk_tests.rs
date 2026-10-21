use super::*;
use crate::layout::ifc::test_support::block_fixture;

#[test]
fn defensive_ancestor_position_skips_inline_frames_and_stops_at_root() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let inline = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
    let child = doc.append_element(Some(inline), "div", Style::default(), None::<&str>);
    doc.nodes[0].unrounded_layout.location.y = 1.0;
    doc.nodes[root].unrounded_layout.location.y = 11.0;
    doc.nodes[inline].unrounded_layout.location.y = 13.0;
    doc.nodes[child].unrounded_layout.location.y = 7.0;
    doc.nodes[root].flags.insert(NodeFlags::IS_IFC_ROOT);
    doc.nodes[inline].flags.insert(NodeFlags::IN_IFC_SUBTREE);
    assert_eq!(current_abs_y_fallback(&doc, child), 19.0);
    assert_eq!(current_abs_y_fallback(&doc, 0), 1.0);
    doc.nodes[child].parent = None;
    assert_eq!(current_abs_y_fallback(&doc, child), 7.0);
}

#[test]
fn defensive_ancestor_position_bounds_a_cycle() {
    let mut doc = Document::new();
    let a = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let b = doc.append_element(Some(a), "div", Style::default(), None::<&str>);
    doc.nodes[a].parent = Some(b);
    doc.nodes[a].unrounded_layout.location.y = 2.0;
    doc.nodes[b].unrounded_layout.location.y = 3.0;
    assert_eq!(current_abs_y_fallback(&doc, a), 10.0);
}

#[test]
fn a_candidate_parent_skips_contents_and_noncontributing_inline_layout_frames() {
    let mut ids = [0; 3];
    let mut fixture = block_fixture("", |doc, root| {
        ids[0] = doc.append_element(Some(root), "span", Style::default(), Some("display:inline"));
        ids[1] = doc.append_element(
            Some(ids[0]),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        ids[2] = doc.append_element(Some(ids[1]), "div", Style::default(), Some("display:block"));
    });
    fixture.doc.nodes[fixture.root]
        .flags
        .insert(NodeFlags::IS_IFC_ROOT);
    fixture.doc.nodes[ids[0]]
        .flags
        .insert(NodeFlags::IN_IFC_SUBTREE);
    assert_eq!(
        candidates::candidate_parent(&fixture.doc, ids[2]),
        Some(fixture.root)
    );
    assert_eq!(candidates::candidate_parent(&fixture.doc, 0), None);
}

fn spanning_growth_fixture(
    owner_css: &str,
) -> (crate::layout::ifc::test_support::Fixture, usize, usize) {
    let mut owner = 0;
    let mut fixture = block_fixture("height:30px", |doc, root| {
        owner = doc.append_element(Some(root), "div", Style::default(), Some(owner_css));
        doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;height:10px"),
        );
    });
    let outer = fixture.doc.parent_of(fixture.root).unwrap();
    fixture.doc.nodes[fixture.root].unrounded_layout.size.height = 30.0;
    fixture.doc.nodes[outer].unrounded_layout.size.height = 100.0;
    (fixture, owner, outer)
}

#[test]
fn spanning_growth_error_does_not_grow_the_current_or_later_ancestor() {
    let (mut fixture, owner, outer) = spanning_growth_fixture("display:block;height:10px");
    let control = PageLayoutControl::default();
    let mut work = ProjectionWork::new(&control, 0);
    let mut edits = FragmentEdits::new(&fixture.doc.fragment_tree);
    let mut siblings = sibling_shifts::SiblingShifts::default();
    let result = grow_spanning_column_ancestors(
        &mut crate::layout::context::LayoutSession::new(&mut fixture.doc),
        owner,
        40.0,
        outer,
        &mut siblings,
        &mut edits,
        &mut work,
    );
    assert!(matches!(
        result,
        Err(LayoutError::FragmentLimitExceeded { limit: 0 })
    ));
    assert_eq!(
        fixture.doc.nodes[fixture.root].unrounded_layout.size.height,
        30.0
    );
    assert_eq!(fixture.doc.nodes[outer].unrounded_layout.size.height, 100.0);
    assert_eq!(fixture.doc.parent_of(owner), Some(fixture.root));
}

#[test]
fn out_of_flow_spanning_owners_stop_before_charging_or_growing_ancestors() {
    for css in [
        "display:block;position:absolute",
        "display:block;float:left",
    ] {
        let (mut fixture, owner, outer) = spanning_growth_fixture(css);
        let control = PageLayoutControl::default();
        let mut work = ProjectionWork::new(&control, 0);
        let mut edits = FragmentEdits::new(&fixture.doc.fragment_tree);
        let mut siblings = sibling_shifts::SiblingShifts::default();
        grow_spanning_column_ancestors(
            &mut crate::layout::context::LayoutSession::new(&mut fixture.doc),
            owner,
            40.0,
            outer,
            &mut siblings,
            &mut edits,
            &mut work,
        )
        .unwrap();
        assert_eq!(
            fixture.doc.nodes[fixture.root].unrounded_layout.size.height,
            30.0
        );
        assert_eq!(fixture.doc.nodes[outer].unrounded_layout.size.height, 100.0);
    }
}

#[test]
fn spanning_growth_stops_at_a_fixed_height_parent() {
    let (mut fixture, owner, outer) = spanning_growth_fixture("display:block;height:10px");
    let control = PageLayoutControl::default();
    let mut work = ProjectionWork::new(&control, 10);
    let mut edits = FragmentEdits::new(&fixture.doc.fragment_tree);
    let mut siblings = sibling_shifts::SiblingShifts::default();
    grow_spanning_column_ancestors(
        &mut crate::layout::context::LayoutSession::new(&mut fixture.doc),
        owner,
        40.0,
        outer,
        &mut siblings,
        &mut edits,
        &mut work,
    )
    .unwrap();
    assert_eq!(
        fixture.doc.nodes[fixture.root].unrounded_layout.size.height,
        30.0
    );
    assert_eq!(fixture.doc.nodes[outer].unrounded_layout.size.height, 100.0);
}

#[test]
fn sibling_adjacency_stops_at_a_node_without_a_parent() {
    let fixture = block_fixture("", |_, _| {});
    assert!(!adjoins_previous_sibling(
        &fixture.doc,
        &fixture.cascade,
        0,
        fixture.root
    ));
}

#[test]
fn growth_through_two_contents_ancestors_keeps_the_moved_child() {
    let mut ids = [0; 7];
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:60px"),
        );
        ids[0] = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
        ids[1] = doc.append_element(
            Some(ids[0]),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        ids[2] = doc.append_element(
            Some(ids[1]),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        ids[3] = doc.append_element(
            Some(ids[2]),
            "div",
            Style::default(),
            Some("display:block;height:60px;break-before:page"),
        );
        ids[4] = doc.append_element(
            Some(ids[2]),
            "div",
            Style::default(),
            Some("display:block;height:20px"),
        );
        ids[5] = doc.append_element(
            Some(ids[0]),
            "div",
            Style::default(),
            Some("display:block;height:10px"),
        );
        ids[6] = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:10px"),
        );
    });
    assert_eq!(doc.parent_of(ids[3]), Some(ids[2]));
    assert_eq!(
        [
            page_flow_y(&doc, ids[3]),
            page_flow_y(&doc, ids[4]),
            page_flow_y(&doc, ids[5]),
            page_flow_y(&doc, ids[6])
        ],
        [100.0, 160.0, 180.0, 190.0]
    );
    assert_eq!(doc.nodes[ids[0]].unrounded_layout.size.height, 130.0);
}

#[test]
fn a_fixed_height_ancestor_keeps_its_height_when_the_following_flow_moves() {
    let mut ids = [0; 3];
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:60px"),
        );
        ids[0] = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:100px"),
        );
        let contents = doc.append_element(
            Some(ids[0]),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        ids[1] = doc.append_element(
            Some(contents),
            "div",
            Style::default(),
            Some("display:block;height:60px;break-before:page"),
        );
        ids[2] = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:10px"),
        );
    });
    assert_eq!(doc.nodes[ids[0]].unrounded_layout.size.height, 100.0);
    assert_eq!(
        [page_flow_y(&doc, ids[1]), page_flow_y(&doc, ids[2])],
        [100.0, 200.0]
    );
}

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
