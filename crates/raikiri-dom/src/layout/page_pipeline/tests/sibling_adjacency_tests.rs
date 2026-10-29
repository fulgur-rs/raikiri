use super::super::sibling_adjacency::SiblingAdjacency;
use super::*;
use crate::layout::context::LayoutSession;
use crate::layout::ifc::test_support::block_fixture;

#[test]
fn sibling_adjacency_work_does_not_multiply_prefix_width_by_break_count() {
    let mut bounded = true;
    for n in [128, 256, 512] {
        for k in [1, 8, n / 2] {
            let mut blocks = Vec::new();
            let mut fixture = block_fixture("", |doc, root| {
                for _ in 0..n {
                    blocks.push(doc.append_element(
                        Some(root),
                        "div",
                        Some("display:block;height:90px;margin-top:30px"),
                    ));
                }
            });
            for &id in &blocks {
                fixture.doc.nodes[id].layout.size.height = 90.0;
            }
            let session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
            let mut margins = SiblingMargins::default();
            let origins = PageOrigins::new(&[], 100.0);
            SIBLING_ADJACENCY_WORK.set([0; 3]);
            let start = std::time::Instant::now();
            for i in n - k..n {
                let page_top = i as f32 * 100.0;
                margins.record_bottom(fixture.root, blocks[i - 1], true, page_top - 10.0);
                let found = margins
                    .truncation_at_break(
                        &session,
                        &fixture.cascade,
                        &origins,
                        &MarginBreak {
                            node_id: blocks[i],
                            raw_y: page_top + 20.0,
                            effective_y: page_top + 20.0,
                            natural_page: i as u32,
                            forced_before: false,
                            margin_top: 30.0,
                            collapses_with_previous_block: true,
                        },
                    )
                    .expect("the page edge crosses the adjoining margin");
                assert_eq!(found.delta, -20.0);
                assert_eq!(found.underflow, None);
            }
            let elapsed = start.elapsed();
            let [calls, positions, boxes] = SIBLING_ADJACENCY_WORK.get();
            eprintln!(
                "sibling_margin n={n} k={k} calls={calls} positions={positions} boxes={boxes} elapsed={elapsed:?}"
            );
            assert_eq!(calls, k);
            bounded &= positions <= n + 2 * k && boxes <= 2 * k;
        }
    }
    assert!(
        bounded,
        "adjacent boxes must not search and collect their entire sibling prefix"
    );
}

#[test]
fn sibling_adjacency_is_used_by_pagination_and_preserves_truncated_page_tops() {
    for n in [16, 32, 64] {
        let mut ids = Vec::new();
        SIBLING_ADJACENCY_WORK.set([0; 3]);
        let (doc, pages) = paged_body(|doc, body| {
            ids.push(doc.append_element(Some(body), "div", Some("display:block;height:90px")));
            for _ in 1..n {
                ids.push(doc.append_element(
                    Some(body),
                    "div",
                    Some("display:block;height:90px;margin-top:30px"),
                ));
            }
        });
        assert_eq!(pages, n);
        for (i, &id) in ids.iter().enumerate() {
            assert_eq!(page_flow_y(&doc, id), i as f32 * 100.0);
            assert_eq!(doc.nodes[id].layout.size.height, 90.0);
        }
        let [calls, positions, boxes] = SIBLING_ADJACENCY_WORK.get();
        assert!(
            calls > 0,
            "the public pagination path must exercise adjacency"
        );
        eprintln!("sibling_margin paged_n={n} calls={calls} positions={positions} boxes={boxes}");
    }
}

#[test]
fn sibling_adjacency_keeps_skipped_and_empty_box_rules() {
    for (style, empty, padding, expected) in [
        ("display:block", true, false, true),
        ("display:list-item", true, false, true),
        ("display:flow-root", true, false, true),
        ("display:inline", true, false, false),
        ("display:block", false, false, false),
        ("display:block", true, true, false),
        ("display:none", false, false, true),
        ("display:block;position:absolute", false, false, true),
        ("display:block;float:left", false, false, true),
        ("display:contents", false, false, false),
        ("display:contents;position:absolute", false, false, false),
        ("display:contents;float:left", false, false, false),
    ] {
        let mut ids = [0; 3];
        let mut fixture = block_fixture("", |doc, root| {
            ids[0] = doc.append_element(Some(root), "div", Some("display:block"));
            doc.append_comment(Some(root), "no box");
            ids[1] = doc.append_element(Some(root), "div", Some(style));
            // Contents has no positioned/floated box; its text still separates
            // the siblings. Ordinary boxes do not expose their children.
            doc.append_text(ids[1], "text inside the intervening element");
            ids[2] = doc.append_element(Some(root), "div", Some("display:block"));
        });
        fixture.doc.nodes[ids[0]].layout.size.height = 20.0;
        fixture.doc.nodes[ids[1]].layout.size.height = if empty { 0.0 } else { 20.0 };
        fixture.doc.nodes[ids[1]].layout.padding.top = if padding { 1.0 } else { 0.0 };
        assert_eq!(
            SiblingAdjacency::default().adjoins(&fixture.doc, &fixture.cascade, ids[2], ids[0]),
            expected,
            "{style} empty={empty} padding={padding}"
        );
    }
}

#[test]
fn sibling_adjacency_preserves_each_white_space_collapse_mode() {
    use raikiri_style::property::WhiteSpaceCollapse::*;
    for (mode, text, expected) in [
        (Collapse, " \t\n\r\u{000c}", true),
        (Discard, " \n", true),
        (PreserveBreaks, " \t", true),
        (PreserveBreaks, " \n", false),
        (PreserveBreaks, "\r", false),
        (Preserve, " ", false),
        (PreserveSpaces, " ", false),
        (BreakSpaces, " ", false),
        (Preserve, "", true),
        (PreserveSpaces, "", true),
        (BreakSpaces, "", true),
        (Collapse, "x", false),
    ] {
        let mut ids = [0; 3];
        let mut fixture = block_fixture("", |doc, root| {
            ids[0] = doc.append_element(Some(root), "div", Some("display:block"));
            ids[1] = doc.append_text(root, text);
            ids[2] = doc.append_element(Some(root), "div", Some("display:block"));
        });
        fixture.cascade.computed[ids[1]].effective_white_space_collapse = mode;
        assert_eq!(
            SiblingAdjacency::default().adjoins(&fixture.doc, &fixture.cascade, ids[2], ids[0]),
            expected,
            "{mode:?}, text={text:?}"
        );
    }
}

#[test]
fn sibling_adjacency_enters_and_leaves_nested_contents_but_not_ordinary_boxes() {
    let mut ids = [0; 6];
    let mut fixture = block_fixture("", |doc, root| {
        ids[0] = doc.append_element(Some(root), "div", Some("display:block"));
        let outer = doc.append_element(Some(root), "div", Some("display:contents"));
        ids[1] = doc.append_element(Some(outer), "div", Some("display:block"));
        let inner = doc.append_element(Some(outer), "div", Some("display:contents"));
        doc.append_element(Some(inner), "div", Some("display:block"));
        doc.append_comment(Some(inner), "no box");
        ids[2] = doc.append_element(Some(inner), "div", Some("display:block"));
        ids[3] = doc.append_element(Some(root), "div", Some("display:block"));
        let ordinary = doc.append_element(Some(root), "div", Some("display:block"));
        ids[4] = doc.append_element(Some(ordinary), "div", Some("display:block"));
        ids[5] = doc.append_element(Some(root), "div", Some("display:block"));
    });
    for &id in &ids[..4] {
        fixture.doc.nodes[id].layout.size.height = 20.0;
    }
    let mut adjacency = SiblingAdjacency::default();
    assert!(adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[2], ids[1]));
    assert!(adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[3], ids[2]));
    assert!(!adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[2], ids[0]));
    assert!(!adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[3], ids[0]));
    assert!(!adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[5], ids[4]));
}

#[test]
fn sibling_adjacency_reuses_positions_but_reads_current_box_sizes() {
    let mut ids = [0; 3];
    let mut fixture = block_fixture("", |doc, root| {
        for id in &mut ids {
            *id = doc.append_element(Some(root), "div", Some("display:block"));
        }
    });
    let mut adjacency = SiblingAdjacency::default();
    assert!(adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[2], ids[0]));
    fixture.doc.nodes[ids[1]].layout.size.height = 10.0;
    assert!(!adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[2], ids[0]));
    fixture.doc.nodes[ids[1]].layout.size.height = 0.0;
    fixture.doc.nodes[ids[1]].layout.border.top = 1.0;
    assert!(!adjacency.adjoins(&fixture.doc, &fixture.cascade, ids[2], ids[0]));
}

#[test]
fn sibling_adjacency_skips_empty_contents_subtrees() {
    let mut ids = [0; 2];
    let fixture = block_fixture("", |doc, root| {
        ids[0] = doc.append_element(Some(root), "div", Some("display:block"));
        let contents = doc.append_element(Some(root), "div", Some("display:contents"));
        doc.append_element(Some(contents), "div", Some("display:contents"));
        doc.append_comment(Some(contents), "no box");
        doc.append_text(contents, " ");
        doc.append_element(Some(contents), "div", Some("display:block"));
        ids[1] = doc.append_element(Some(root), "div", Some("display:block"));
    });
    assert!(SiblingAdjacency::default().adjoins(&fixture.doc, &fixture.cascade, ids[1], ids[0]));
}

#[test]
fn sibling_adjacency_rejects_a_parent_link_missing_from_the_child_list() {
    let mut ids = [0; 2];
    let mut fixture = block_fixture("", |doc, root| {
        for id in &mut ids {
            *id = doc.append_element(Some(root), "div", Some("display:block"));
        }
    });
    fixture.doc.nodes[fixture.root]
        .children
        .retain(|&id| id != ids[1]);
    assert!(!SiblingAdjacency::default().adjoins(&fixture.doc, &fixture.cascade, ids[1], ids[0]));
}

#[test]
fn sibling_adjacency_does_not_cross_an_ordinary_parent_boundary() {
    let mut ids = [0; 2];
    let fixture = block_fixture("", |doc, root| {
        ids[0] = doc.append_element(Some(root), "div", Some("display:block"));
        let ordinary = doc.append_element(Some(root), "div", Some("display:block"));
        ids[1] = doc.append_element(Some(ordinary), "div", Some("display:block"));
    });
    // The earlier box is a sibling of the parent, not of its first child.
    assert!(!SiblingAdjacency::default().adjoins(&fixture.doc, &fixture.cascade, ids[1], ids[0]));
}
