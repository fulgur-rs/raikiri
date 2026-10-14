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
