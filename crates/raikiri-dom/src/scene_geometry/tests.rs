use super::*;
#[test]
fn warm_clone_shares_geometry_and_clear_is_local() {
    let dom = Document::new();
    let original = dom.scene_geometry();
    assert!(original.records().is_empty());
    let mut clone = dom.clone();
    assert!(std::ptr::eq(original, clone.scene_geometry()));
    clone.page_projection.clear();
    assert!(clone.page_projection.scene_geometry.get().is_none());
    assert!(std::ptr::eq(original, dom.scene_geometry()));
    assert!(!std::ptr::eq(original, clone.scene_geometry()));
    let cold = Document::new();
    let copy = cold.clone();
    assert!(!std::ptr::eq(cold.scene_geometry(), copy.scene_geometry()));
}

use std::cell::Cell;
thread_local! {static HARVEST: Cell<(usize,usize)> = const {Cell::new((0,0))};}
pub(super) fn count_harvest(nodes: usize, lines: usize) {
    HARVEST.with(|v| {
        let (n, l) = v.get();
        v.set((n + nodes, l + lines));
    });
}
#[test]
fn shared_geometry_harvests_each_node_and_text_line_once() {
    use crate::layout::ifc::test_support::block_fixture_with_ahem;
    for count in [100, 200, 400, 800] {
        let fixture = block_fixture_with_ahem("width:600px", |dom, root| {
            for _ in 0..count {
                let section =
                    dom.append_element(Some(root), "section", Some("display:block;height:440px"));
                let p = dom.append_element(Some(section), "p", Some("display:block;margin:0"));
                dom.append_text(p, "X");
            }
        });
        let mut dom = fixture.doc;
        let mut page = raikiri_traits::PageBox::new();
        page.width = 640.0;
        page.height = 480.0;
        crate::layout_single_page(&mut dom, &fixture.cascade, page).unwrap();
        HARVEST.with(|v| v.set((0, 0)));
        index::take_query_visits();
        let mut candidates = 0;
        for i in 0..count {
            candidates += dom
                .scene_geometry()
                .candidates(false, -(i as f32 * 440.0), 440.0)
                .len();
        }
        let (nodes, lines) = HARVEST.with(Cell::get);
        let visits = index::take_query_visits();
        let geometry = dom.scene_geometry();
        let bytes = geometry.records.capacity() * std::mem::size_of::<SceneGeometryNode>()
            + geometry.pieces.capacity() * std::mem::size_of::<(usize, SceneRect)>()
            + geometry
                .axes
                .iter()
                .map(|axis| axis.retained_bytes())
                .sum::<usize>();
        eprintln!(
            "geometry-index pages={count} visited_branches={visits} retained_vec_capacity_bytes={bytes}"
        );
        assert!(visits < 128 * count);
        eprintln!(
            "geometry-work pages={count} harvest_nodes={nodes} harvest_lines={lines} candidate_pieces={candidates}"
        );
        assert_eq!(nodes, dom.scene_geometry().records().len());
        assert!(nodes <= 4 * count + 4);
        assert_eq!(lines, count);
        assert!(candidates <= 16 * count);
        let saved = HARVEST.with(Cell::get);
        dom.scene_geometry();
        assert_eq!(HARVEST.with(Cell::get), saved);
    }
}
