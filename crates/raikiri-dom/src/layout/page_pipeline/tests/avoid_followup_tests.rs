use super::*;

fn block(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Style::default(), Some(css))
}

#[test]
fn an_empty_block_follows_a_forced_break_sibling() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let wrapper = block(doc, body, "display:block");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:60px;break-before:page",
        ));
        ids.push(block(doc, wrapper, "display:block;height:20px"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![100.0, 160.0, 180.0]);
}

#[test]
fn boxes_after_a_contents_wrapper_follow_a_forced_break_inside_it() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let wrapper = block(doc, body, "display:block");
        let contents = block(doc, wrapper, "display:contents");
        ids.push(block(
            doc,
            contents,
            "display:block;height:60px;break-before:page",
        ));
        ids.push(block(doc, contents, "display:block;height:20px"));
        ids.push(block(doc, wrapper, "display:block;height:20px"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![100.0, 160.0, 180.0, 200.0]);
}

#[test]
fn an_avoiding_box_in_a_single_column_grid_item_moves() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(doc, body, "display:grid");
        let item = block(doc, grid, "display:block;height:150px");
        ids.push(item);
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![60.0, 100.0, 210.0]);
}

#[test]
fn an_avoiding_box_in_an_auto_height_single_column_grid_item_grows_the_grid() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(doc, body, "display:grid");
        ids.push(grid);
        let item = block(doc, grid, "display:block");
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, item, "display:block;height:80px"));
        ids.push(block(doc, grid, "display:block;height:10px"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let grid_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!(
        (ys, grid_height),
        (vec![60.0, 100.0, 160.0, 240.0, 250.0], 190.0)
    );
}

#[test]
fn a_flow_root_grows_around_a_moved_float_only_as_far_as_its_flow() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let root = block(doc, body, "display:flow-root");
        ids.push(root);
        let wrapper = block(doc, root, "display:block");
        block(doc, wrapper, "float:left;width:20px;height:150px");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let root_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!((ys, root_height), (vec![60.0, 100.0, 210.0], 150.0));
}

#[test]
fn the_flow_after_a_block_with_a_hanging_float_follows_its_moved_content() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let wrapper = block(doc, body, "display:block");
        ids.push(wrapper);
        block(doc, wrapper, "float:left;width:20px;height:150px");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let wrapper_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!((ys, wrapper_height), (vec![60.0, 100.0, 160.0], 100.0));
}

#[test]
fn a_shared_ifc_block_child_reports_its_float_from_the_content_edge() {
    let mut root = 0;
    let mut paragraph = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        root = block(doc, body, "display:flow-root");
        paragraph = block(
            doc,
            root,
            "display:block;padding-top:20px;border-top:2px solid black;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "x");
        let child = block(doc, paragraph, "display:block");
        block(doc, child, "float:left;width:20px;height:50px");
    });
    assert!(doc.nodes[paragraph].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 82.0);
}

#[test]
fn a_local_ifc_root_holds_its_own_float_end() {
    let mut root = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        root = block(
            doc,
            body,
            "display:flow-root;padding-top:20px;border-top:2px solid black;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(root, "x");
        block(doc, root, "float:left;width:20px;height:50px");
    });
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 72.0);
}

#[test]
fn a_nested_bfc_hides_its_overflowing_float_from_the_outer_bfc() {
    let mut outer = 0;
    let mut inner = 0;
    let (doc, _) = paged_body(|doc, body| {
        outer = block(doc, body, "display:flow-root");
        inner = block(doc, outer, "display:flow-root;height:20px");
        block(doc, inner, "float:left;width:20px;height:50px");
    });
    assert_eq!(doc.nodes[inner].unrounded_layout.size.height, 20.0);
    assert_eq!(doc.nodes[outer].unrounded_layout.size.height, 20.0);
}

#[test]
fn a_flow_root_grows_when_the_moved_content_passes_a_hanging_float() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let root = block(doc, body, "display:flow-root");
        ids.push(root);
        let wrapper = block(doc, root, "display:block");
        block(doc, wrapper, "float:left;width:20px;height:70px");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let root_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    // The moved box ends at 160, below the float's 130 end.
    assert_eq!((ys, root_height), (vec![60.0, 100.0, 160.0], 100.0));
}

#[test]
fn a_flow_root_grows_for_a_float_after_the_moved_block() {
    let mut root = 0;
    let mut moved = 0;
    let mut later_float = 0;
    let mut following = 0;
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        root = block(doc, body, "display:flow-root");
        moved = block(doc, root, "display:block;height:60px;break-inside:avoid");
        later_float = block(doc, root, "float:left;width:20px;height:80px");
        following = block(doc, body, "display:block;height:10px");
    });
    assert_eq!(
        (
            page_flow_y(&doc, root),
            page_flow_y(&doc, moved),
            doc.nodes[root].unrounded_layout.size.height,
            page_flow_y(&doc, later_float),
            page_flow_y(&doc, following),
        ),
        (60.0, 100.0, 180.0, 160.0, 240.0)
    );
}

#[test]
fn a_scroll_container_grows_around_a_moved_float() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let root = block(doc, body, "display:block;overflow:hidden");
        ids.push(root);
        block(
            doc,
            root,
            "float:left;width:40px;height:60px;break-inside:avoid",
        );
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let root_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!((ys, root_height), (vec![60.0, 160.0], 100.0));
}

#[test]
fn a_moved_float_grows_an_indefinite_percentage_context_but_not_a_fixed_one() {
    // CSS 2.1 §10.5: a percentage against an auto-height containing block
    // behaves as auto, so pagination must grow it around the moved float.
    for (height, expected_height) in [("50%", 100.0), ("60px", 60.0)] {
        let mut root = 0;
        let mut float = 0;
        let mut following = 0;
        let (doc, _) = paged_body(|doc, body| {
            block(doc, body, "display:block;height:60px");
            let wrapper = block(doc, body, "display:block");
            let contents = block(doc, wrapper, "display:contents;height:200px");
            root = block(doc, contents, &format!("display:flow-root;height:{height}"));
            float = block(
                doc,
                root,
                "float:left;width:40px;height:60px;break-inside:avoid",
            );
            following = block(doc, body, "display:block;height:10px");
        });
        assert_eq!(page_flow_y(&doc, root), 60.0);
        assert_eq!(page_flow_y(&doc, float), 100.0);
        assert_eq!(
            doc.nodes[root].unrounded_layout.size.height,
            expected_height
        );
        assert_eq!(page_flow_y(&doc, following), 60.0 + expected_height);
    }
}

#[test]
fn a_hanging_float_two_blocks_down_still_takes_up_the_move() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let root = block(doc, body, "display:flow-root");
        ids.push(root);
        let outer = block(doc, root, "display:block");
        let wrapper = block(doc, outer, "display:block");
        block(doc, wrapper, "float:left;width:20px;height:150px");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, outer, "display:block;height:10px"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let root_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    // The block after the wrapper follows the moved box to 160; the
    // flow-root still ends with the float at 210.
    assert_eq!((ys, root_height), (vec![60.0, 100.0, 160.0, 210.0], 150.0));
}

#[test]
fn a_formatting_context_grows_with_a_float_that_moved_inside_a_min_height_block() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let root = block(doc, body, "display:flow-root");
        ids.push(root);
        let wrapper = block(doc, root, "display:block;min-height:100px");
        let moved = block(doc, wrapper, "display:block;break-inside:avoid");
        ids.push(moved);
        block(doc, moved, "float:left;width:20px;height:150px");
        block(doc, moved, "display:block;height:60px");
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let root_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    // The float moves with its block to 100..250, and the flow-root holds it.
    assert_eq!((ys, root_height), (vec![60.0, 100.0, 250.0], 190.0));
}

#[test]
fn a_moved_ifc_block_child_keeps_its_origin_in_ancestor_float_extent() {
    let mut root = 0;
    let mut paragraph = 0;
    let mut moved = 0;
    let mut following = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:60px");
        root = block(doc, body, "display:flow-root");
        paragraph = block(
            doc,
            root,
            "display:block;min-height:100px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "x");
        moved = block(doc, paragraph, "display:block;break-inside:avoid");
        block(doc, moved, "float:left;width:20px;height:150px");
        block(doc, moved, "display:block;height:60px");
        following = block(doc, body, "display:block;height:10px");
    });
    assert!(doc.nodes[paragraph].is_ifc_root());
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    assert_eq!(
        super::super::same_bfc_float_end_in_subtree(
            &doc,
            &cascade,
            doc.parent_of(root).unwrap(),
            paragraph,
            doc.nodes[paragraph].unrounded_layout.location.y,
        ),
        190.0
    );
    assert_eq!(
        (
            page_flow_y(&doc, moved),
            doc.nodes[root].unrounded_layout.size.height,
            page_flow_y(&doc, following),
        ),
        (100.0, 190.0, 250.0)
    );
}

#[test]
fn a_formatting_context_keeps_its_max_height_when_a_float_moves_inside_a_min_height_block() {
    for (max_height, expected_height, expected_following_y) in
        [("120px", 120.0, 180.0), ("170px", 170.0, 230.0)]
    {
        let mut root = 0;
        let mut following = 0;
        let (doc, _) = paged_body(|doc, body| {
            block(doc, body, "display:block;height:60px");
            root = block(
                doc,
                body,
                &format!("display:flow-root;max-height:{max_height}"),
            );
            let wrapper = block(doc, root, "display:block;min-height:100px");
            let moved = block(doc, wrapper, "display:block;break-inside:avoid");
            block(doc, moved, "float:left;width:20px;height:150px");
            block(doc, moved, "display:block;height:60px");
            following = block(doc, body, "display:block;height:10px");
        });
        assert_eq!(
            (
                page_flow_y(&doc, root),
                doc.nodes[root].unrounded_layout.size.height,
                page_flow_y(&doc, following),
            ),
            (60.0, expected_height, expected_following_y),
            "max-height: {max_height}"
        );
    }
}

#[test]
fn the_flow_after_a_fixed_height_block_stays_past_a_forced_break_in_it() {
    let mut ids = Vec::new();
    let (doc, pages) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:30px");
        let wrapper = block(doc, body, "display:block;height:50px");
        block(doc, wrapper, "display:block;height:20px");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:20px;break-before:page",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!((ys, pages), (vec![100.0, 130.0], 2));
}

#[test]
fn a_fixed_height_wrapper_reports_that_it_cannot_absorb_a_moved_child() {
    let mut body = 0;
    let mut moved = 0;
    let (mut doc, _) = paged_body(|doc, body_id| {
        body = body_id;
        let wrapper = block(doc, body, "display:block;height:20px");
        moved = block(doc, wrapper, "display:block;height:40px");
    });
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut shifts = super::super::SubtreeShifts::default();
    assert!(super::super::grow_ancestors_for_shift(
        &mut doc,
        &cascade,
        moved,
        20.0,
        body,
        &mut shifts,
        0,
    ));
}

#[test]
fn a_forced_break_in_a_two_column_grid_does_not_grow_the_grid() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:30px");
        let grid = block(doc, body, "display:grid;grid-template-columns:50px 50px");
        ids.push(grid);
        let item = block(doc, grid, "display:block");
        block(doc, item, "display:block;height:20px");
        block(doc, item, "display:block;height:20px;break-before:page");
        block(doc, grid, "display:block;height:40px");
    });
    let grid_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!(grid_height, 40.0);
}

#[test]
fn a_grid_item_holding_a_float_does_not_grow_past_it() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(doc, body, "display:grid");
        let item = block(doc, grid, "display:block");
        ids.push(item);
        block(doc, item, "float:left;width:20px;height:150px");
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let item_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!((ys, item_height), (vec![60.0, 100.0, 210.0], 150.0));
}

#[test]
fn a_one_column_grid_grows_from_its_items_in_row_order() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(doc, body, "display:grid");
        ids.push(grid);
        let item = block(doc, grid, "display:block;grid-row:2");
        block(doc, item, "display:block;height:10px");
        block(doc, item, "display:block;height:60px;break-inside:avoid");
        block(doc, item, "display:block;height:80px");
        block(doc, grid, "display:block;grid-row:1;height:10px");
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let grid_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    // The avoiding box in the second row moves from 80 to 100.
    assert_eq!((ys, grid_height), (vec![60.0, 240.0], 180.0));
}

#[test]
fn grid_items_sharing_a_row_do_not_stack() {
    let mut grids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        let stacked = block(doc, body, "display:grid");
        block(doc, stacked, "display:block;height:20px");
        block(doc, stacked, "display:block;height:20px");
        let shared = block(doc, body, "display:grid");
        block(
            doc,
            shared,
            "display:block;height:20px;grid-row:1;grid-column:1",
        );
        block(
            doc,
            shared,
            "display:block;height:20px;grid-row:1;grid-column:1",
        );
        let spanning = block(doc, body, "display:grid");
        block(
            doc,
            spanning,
            "display:block;height:40px;grid-row:1 / 3;grid-column:1",
        );
        block(
            doc,
            spanning,
            "display:block;height:20px;grid-row:2;grid-column:1",
        );
        grids.extend([stacked, shared, spanning]);
    });
    let stacks: Vec<bool> = grids
        .iter()
        .map(|&id| grid_stacks_items(&doc, id))
        .collect();
    assert_eq!(stacks, vec![true, false, false]);
}

#[test]
fn a_grid_item_in_a_fixed_row_keeps_its_track_size() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(doc, body, "display:grid;grid-template-rows:150px");
        ids.push(grid);
        let item = block(doc, grid, "display:block");
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let grid_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!((ys, grid_height), (vec![60.0, 80.0, 210.0], 150.0));
}

#[test]
fn boxes_after_a_paragraph_follow_a_forced_break_inside_it() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let outer = block(doc, body, "display:block");
        let paragraph = block(doc, outer, "display:block;font-size:10px;line-height:10px");
        doc.append_text(paragraph, "x");
        ids.push(block(
            doc,
            paragraph,
            "display:block;height:20px;break-before:page",
        ));
        ids.push(block(doc, outer, "display:block;height:20px"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![100.0, 120.0, 140.0]);
}

#[test]
fn a_paragraph_follows_a_forced_break_inside_a_wrapper_on_its_lines() {
    // The forced break is inside a block wrapper among the paragraph's
    // lines, so the wrapper itself stays; the paragraph's content after the
    // wrapper and the boxes after the paragraph still follow the break.
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let paragraph = block(doc, body, "display:block;font-size:10px;line-height:10px");
        doc.append_text(paragraph, "x");
        let wrapper = block(doc, paragraph, "display:block");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:20px;break-before:page",
        ));
        ids.push(block(doc, paragraph, "display:block;height:10px"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![100.0, 120.0, 130.0]);
}

#[test]
fn a_grid_item_in_an_explicit_auto_row_grows_the_grid() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(
            doc,
            body,
            "display:grid;grid-template-rows:auto repeat(2, auto)",
        );
        ids.push(grid);
        let item = block(doc, grid, "display:block");
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, item, "display:block;height:80px"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let grid_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!((ys, grid_height), (vec![60.0, 100.0, 160.0, 240.0], 180.0));
}

#[test]
fn an_avoiding_box_in_a_grid_with_a_sized_row_stays() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(doc, body, "display:grid;grid-template-rows:auto 20px");
        let item = block(doc, grid, "display:block");
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        block(doc, item, "display:block;height:80px");
        ids.push(block(doc, grid, "display:block"));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![80.0, 220.0, 240.0]);
}

#[test]
fn boxes_after_a_forced_break_in_a_two_column_grid_item_follow_it() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:30px");
        let grid = block(doc, body, "display:grid;grid-template-columns:50px 50px");
        let item = block(doc, grid, "display:block");
        let inner = block(doc, item, "display:block");
        block(doc, inner, "display:block;height:20px");
        ids.push(block(
            doc,
            inner,
            "display:block;height:20px;break-before:page",
        ));
        ids.push(block(doc, inner, "display:block;height:20px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![100.0, 120.0]);
}

#[test]
fn an_avoiding_box_in_a_grid_with_intrinsic_rows_moves() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(
            doc,
            body,
            "display:grid;grid-template-rows:min-content;grid-auto-rows:max-content",
        );
        let item = block(doc, grid, "display:block");
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        block(doc, item, "display:block;height:80px");
        ids.push(block(doc, grid, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![100.0, 240.0]);
}

#[test]
fn an_avoiding_box_in_a_grid_of_definite_height_stays() {
    assert_avoiding_box_in_a_bounded_grid_stays(
        "display:grid;height:150px;grid-template-rows:auto",
    );
}

#[test]
fn an_avoiding_box_in_a_grid_with_a_min_height_stays() {
    assert_avoiding_box_in_a_bounded_grid_stays("display:grid;min-height:150px");
}

fn assert_avoiding_box_in_a_bounded_grid_stays(grid_css: &str) {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let grid = block(doc, body, grid_css);
        ids.push(grid);
        let item = block(doc, grid, "display:block");
        block(doc, item, "display:block;height:20px");
        ids.push(block(
            doc,
            item,
            "display:block;height:60px;break-inside:avoid",
        ));
        ids.push(block(doc, body, "display:block;height:10px"));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    let grid_height = doc.nodes[ids[0]].unrounded_layout.size.height;
    assert_eq!((ys, grid_height), (vec![60.0, 80.0, 210.0], 150.0));
}

#[test]
fn boxes_after_an_underflowing_forced_break_follow_the_break() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:80px");
        let wrapper = block(doc, body, "display:block");
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:20px;margin-top:-10px;break-before:page",
        ));
        ids.push(block(doc, wrapper, "display:block;height:10px"));
        ids.push(block(
            doc,
            wrapper,
            "display:block;height:10px;break-inside:avoid",
        ));
    });
    let ys: Vec<f32> = ids.iter().map(|&id| page_flow_y(&doc, id)).collect();
    assert_eq!(ys, vec![90.0, 120.0, 130.0]);
}

#[test]
fn an_auto_block_end_margin_does_not_push_an_avoiding_float_off_its_page() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let root = block(doc, body, "display:block;overflow:hidden");
        ids.push(block(
            doc,
            root,
            "float:left;width:40px;height:40px;margin-bottom:auto;break-inside:avoid",
        ));
    });
    // The float ends exactly at the page edge; its auto margin adds nothing.
    assert_eq!(page_flow_y(&doc, ids[0]), 60.0);
}
