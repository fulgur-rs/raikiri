use super::*;
use crate::layout::page_pipeline::page_start::ChildPositions;

/// [`ChildPositions::begins_page_below_leading_edges`] for one query.
fn begins_page_below_leading_edges(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_start: f32,
) -> bool {
    ChildPositions::default()
        .begins_page_below_leading_edges(document, cascade, body_id, node_id, page_start)
}

#[test]
fn cached_page_start_queries_read_current_preceding_geometry() {
    let (mut previous, mut child) = (0, 0);
    let (mut doc, cascade, body) = laid_out(|doc, body| {
        previous = block(doc, body, "display:block;height:30px;break-after:page");
        let wrapper = block(doc, body, "display:block;padding-top:5px");
        child = block(doc, wrapper, "display:block;height:20px;break-before:page");
    });
    let mut positions = ChildPositions::default();
    assert!(positions.begins_page_below_leading_edges(&doc, &cascade, body, child, 100.0));
    let indexed = positions.indexed_children;
    doc.nodes[previous].layout.size.height = 130.0;
    assert!(!positions.begins_page_below_leading_edges(&doc, &cascade, body, child, 100.0));
    doc.nodes[previous].layout.size.height = 30.0;
    assert!(positions.begins_page_below_leading_edges(&doc, &cascade, body, child, 100.0));
    assert_eq!(positions.indexed_children, indexed);
}

#[test]
fn cached_page_start_index_work_does_not_multiply_wrapper_count_by_query_count() {
    for count in [32, 128, 512] {
        let mut children = Vec::new();
        let (doc, cascade, body) = laid_out(|doc, body| {
            for _ in 0..count {
                let wrapper = block(doc, body, "display:block");
                children.push(block(doc, wrapper, "display:block;height:1px"));
            }
        });
        let mut positions = ChildPositions::default();
        for &child in &children {
            positions.begins_page_below_leading_edges(&doc, &cascade, body, child, 0.0);
        }
        let indexed = positions.indexed_children;
        assert!(indexed > 0 && indexed <= count * 2);
        for page_start in [0.0, 100.0, 200.0] {
            for &child in &children {
                positions.begins_page_below_leading_edges(&doc, &cascade, body, child, page_start);
            }
        }
        assert_eq!(positions.indexed_children, indexed);
    }
}

/// Lays out the document the closure builds on 100px pages, and returns it with
/// its cascade and its body.
fn laid_out(build: impl FnOnce(&mut Document, usize)) -> (Document, CascadeResult, usize) {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    build(&mut doc, body);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_pages(&mut doc, &cascade, page).unwrap();
    (doc, cascade, body)
}

fn block(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Some(css))
}

/// The box that breaks before itself inside `wrapper_css`, after `before` has
/// built what precedes the wrapper in the body.
fn child_after(
    before: impl FnOnce(&mut Document, usize),
    wrapper_css: &str,
) -> (Document, CascadeResult, usize, usize) {
    let mut child = 0;
    let (doc, cascade, body) = laid_out(|doc, body| {
        before(doc, body);
        let wrapper = block(doc, body, wrapper_css);
        child = block(doc, wrapper, "display:block;height:20px;break-before:page");
    });
    (doc, cascade, body, child)
}

fn break_after_block(doc: &mut Document, body: usize) {
    block(doc, body, "display:block;height:30px;break-after:page");
}

#[test]
fn a_box_the_wrapper_begins_starts_the_page_the_block_before_the_wrapper_ended() {
    let (doc, cascade, body, child) =
        child_after(break_after_block, "display:block;padding-top:5px");
    assert_eq!(page_flow_y(&doc, child), 105.0);
    assert!(begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
    // The block before the wrapper lies on the page that starts at 0.
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 0.0
    ));
}

#[test]
fn a_box_with_a_sibling_before_it_is_left_to_the_margin_box_test() {
    let mut child = 0;
    let (doc, cascade, body) = laid_out(|doc, body| {
        break_after_block(doc, body);
        child = block(doc, body, "display:block;height:20px;break-before:page");
    });
    assert_eq!(page_flow_y(&doc, child), 100.0);
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
}

#[test]
fn a_wrapper_inside_a_contents_element_is_still_begun() {
    let mut child = 0;
    let (doc, cascade, body) = laid_out(|doc, body| {
        break_after_block(doc, body);
        let wrapper = block(doc, body, "display:block;padding-top:5px");
        let contents = block(doc, wrapper, "display:contents");
        child = block(doc, contents, "display:block;height:20px;break-before:page");
    });
    assert!(begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
}

#[test]
fn the_first_box_of_the_flow_is_not_begun_through_a_wrapper() {
    let mut child = 0;
    let (doc, cascade, body) = laid_out(|doc, body| {
        child = block(doc, body, "display:block;height:20px;break-before:page");
    });
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 0.0
    ));
    // The same box in a wrapper has nothing before the wrapper either.
    let (doc, cascade, body, child) = child_after(|_, _| {}, "display:block;padding-top:5px");
    assert!(begins_page_below_leading_edges(
        &doc, &cascade, body, child, 0.0
    ));
}

#[test]
fn an_empty_box_at_the_page_start_comes_before_the_box() {
    let (doc, cascade, body, child) = child_after(
        |doc, body| {
            break_after_block(doc, body);
            block(doc, body, "display:block");
        },
        "display:block;padding-top:5px",
    );
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
}

#[test]
fn text_inline_boxes_and_contents_elements_before_the_wrapper_come_before_the_box() {
    let (doc, cascade, body, child) = child_after(
        |doc, body| {
            break_after_block(doc, body);
            doc.append_text(body, "x");
        },
        "display:block;padding-top:5px",
    );
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
    let (doc, cascade, body, child) = child_after(
        |doc, body| {
            break_after_block(doc, body);
            doc.append_element(Some(body), "span", Some("display:inline"));
        },
        "display:block;padding-top:5px",
    );
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
    let (doc, cascade, body, child) = child_after(
        |doc, body| {
            break_after_block(doc, body);
            block(doc, body, "display:contents");
        },
        "display:block;padding-top:5px",
    );
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
}

#[test]
fn a_float_comes_before_the_box_unless_it_starts_the_page_or_lies_on_an_earlier_one() {
    // The float is 10px down the page that starts at 0.
    let float_css = "float:left;width:20px;height:20px";
    let (doc, cascade, body, child) = child_after(
        |doc, body| {
            block(doc, body, "display:block;height:10px");
            block(doc, body, float_css);
        },
        "display:block;padding-top:5px",
    );
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 0.0
    ));
    // On the page that starts at 100, that float lies on an earlier page.
    assert!(begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
    // A float at the top of the page starts it.
    let (doc, cascade, body, child) = child_after(
        |doc, body| {
            block(doc, body, float_css);
        },
        "display:block;padding-top:5px",
    );
    assert!(begins_page_below_leading_edges(
        &doc, &cascade, body, child, 0.0
    ));
}

#[test]
fn a_wrapper_that_is_not_a_plain_block_does_not_hold_its_first_child() {
    for wrapper in [
        "display:flex;padding-top:5px",
        "display:block;padding-top:5px;columns:2",
        "display:block;padding-top:5px;float:left;width:50px",
    ] {
        let (doc, cascade, body, child) = child_after(break_after_block, wrapper);
        assert!(
            !begins_page_below_leading_edges(&doc, &cascade, body, child, 100.0),
            "{wrapper}"
        );
    }
}

#[test]
fn a_box_placed_out_of_the_flow_does_not_begin_the_page() {
    let mut child = 0;
    let (doc, cascade, body) = laid_out(|doc, body| {
        break_after_block(doc, body);
        let wrapper = block(doc, body, "display:block;padding-top:5px");
        child = block(
            doc,
            wrapper,
            "position:absolute;top:0;left:0;width:20px;height:20px;break-before:page",
        );
    });
    assert!(!begins_page_below_leading_edges(
        &doc, &cascade, body, child, 100.0
    ));
}
