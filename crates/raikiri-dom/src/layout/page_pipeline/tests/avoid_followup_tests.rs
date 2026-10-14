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
    assert_eq!((ys, wrapper_height), (vec![60.0, 100.0, 160.0], 150.0));
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
