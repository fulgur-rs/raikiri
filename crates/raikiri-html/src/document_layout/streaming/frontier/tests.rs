use html5ever::driver::{ParseOpts, parse_document};
use raikiri_traits::{Dom, Element, Node, RenderLimits};

use super::*;
use crate::document_parse::assemble_document;
use crate::input::Utf8Feed;
use crate::parse::finish_document;
use crate::{ParseOptions, RaikiriTreeSink, build_rule_tree};

/// Parse `prefix` as the start of a longer document and describe its
/// frontier: `end`, `<tag>` for an element, or `"text"` for a text node.
fn frontier_of(prefix: &str) -> String {
    let mut feed = Utf8Feed::new(parse_document(
        RaikiriTreeSink::default(),
        ParseOpts::default(),
    ));
    feed.feed(prefix.as_bytes()).expect("utf-8");
    let builder = &feed.parser().tokenizer.sink;
    let sink = &builder.sink;
    let open = crate::sink::traced_handles(builder);
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = finish_document(sink.snapshot(), &options).expect("finish");
    let forward = build_rule_tree(&uncascaded).has_forward_dependent_selectors();
    let document =
        assemble_document(uncascaded, &options, &RenderLimits::default()).expect("cascade");
    match stable_frontier(&document.uncascaded.dom, &document.cascade, &open, forward) {
        Frontier::End => "end".to_owned(),
        Frontier::At(id) => describe(&document.uncascaded.dom, id),
    }
}

fn describe(dom: &raikiri_dom::Document, id: usize) -> String {
    let node = dom.node(raikiri_traits::NodeId(id as u64)).expect("node");
    match node.as_element() {
        Some(element) => format!("<{}>", element.tag_name()),
        None => format!("{:?}", node.text_content().unwrap_or("")),
    }
}

#[test]
fn closed_blocks_are_final() {
    assert_eq!(frontier_of("<div><p>one</p><p>two</p>"), "end");
}

#[test]
fn an_open_paragraph_holds_from_its_first_line_box() {
    assert_eq!(frontier_of("<p>one</p><p>two <b>bo"), "\"two \"");
    assert_eq!(frontier_of("<div><p>one</p>loose <i>text"), "\"loose \"");
}

#[test]
fn trailing_whitespace_after_a_closed_block_holds_only_itself() {
    assert_eq!(frontier_of("<div><p>one</p>\n  "), "\"\\n  \"");
}

#[test]
fn floats_belong_to_the_surrounding_paragraph() {
    assert_eq!(
        frontier_of("<div>start <span style='float:left'>f</span> more"),
        "\"start \""
    );
}

#[test]
fn open_layout_containers_hold_from_their_start() {
    assert_eq!(
        frontier_of("<p>a</p><table><tr><td>cell</td></tr>"),
        "<table>"
    );
    assert_eq!(
        frontier_of("<p>a</p><div style='display:flex'><div>item</div>"),
        "<div>"
    );
    assert_eq!(
        frontier_of("<p>a</p><section style='display:grid'><p>x</p>"),
        "<section>"
    );
    assert_eq!(
        frontier_of("<p>a</p><article style='columns:2'><p>x</p>"),
        "<article>"
    );
    assert_eq!(
        frontier_of("<p>a</p><aside style='break-inside:avoid'><p>x</p>"),
        "<aside>"
    );
}

#[test]
fn the_outermost_open_layout_container_wins() {
    assert_eq!(
        frontier_of("<main style='display:flex'><table><tr><td>x"),
        "<main>"
    );
}

#[test]
fn closed_layout_containers_are_final() {
    assert_eq!(
        frontier_of("<table><tr><td>cell</td></tr></table><p>after</p>"),
        "end"
    );
}

#[test]
fn foster_parented_text_does_not_hide_the_open_table() {
    assert_eq!(frontier_of("<p>a</p><table>stray"), "<table>");
}

#[test]
fn avoided_breaks_join_the_frontier_to_earlier_siblings() {
    assert_eq!(
        frontier_of("<h2 style='break-after:avoid'>Title</h2><p>body"),
        "<h2>"
    );
    assert_eq!(
        frontier_of("<p>one</p><p>two</p><p style='break-before:avoid'>three"),
        "<p>",
    );
}

#[test]
fn void_elements_do_not_stay_open() {
    assert_eq!(
        frontier_of("<div><img style='display:block; break-inside:avoid'><p>x</p>"),
        "end"
    );
}

#[test]
fn open_elements_with_auto_direction_hold_from_their_start() {
    assert_eq!(frontier_of("<p>a</p><div dir=AUTO><p>x</p>"), "<div>");
    assert_eq!(frontier_of("<p>a</p><p>b <bdi>x"), "\"b \"");
    assert_eq!(
        frontier_of("<p>a</p><div><bdi style='display:block'>x"),
        "<bdi>"
    );
    assert_eq!(frontier_of("<p>a</p><div dir=rtl><p>x</p>"), "end");
    assert_eq!(frontier_of("<div dir=auto><p>x</p></div><p>y</p>"), "end");
}

#[test]
fn forward_dependent_selectors_hold_everything() {
    // The virtual document root has no tag and no text.
    assert_eq!(
        frontier_of("<style>li:last-child{color:red}</style><p>a</p>"),
        "\"\""
    );
}

#[test]
fn elements_closed_without_a_pop_notification_are_final() {
    // `</p>` and `</div>` close their open descendants without telling the
    // sink; stale active formatting entries must not keep them open.
    assert_eq!(frontier_of("<p><b>x</p><p>y</p>"), "end");
    assert_eq!(
        frontier_of("<div style='display:flex'><span><i>x</span></div><p>y</p>"),
        "end"
    );
}

#[test]
fn open_boxes_sized_by_their_content_hold_from_their_start() {
    assert_eq!(
        frontier_of("<p>a</p><div style='width:min-content'><p>x</p>"),
        "<div>"
    );
    assert_eq!(
        frontier_of("<p>a</p><div style='float:left'><p>x</p>"),
        "<div>"
    );
    assert_eq!(
        frontier_of("<p>a</p><div style='position:absolute'><p>x</p>"),
        "<div>"
    );
    // A float with a definite width still sits in the trailing inline run
    // of its parent.
    assert_eq!(
        frontier_of("<p>a</p><div style='float:left; width:10em'><p>x</p>"),
        "<div>"
    );
    assert_eq!(
        frontier_of("<p>a</p><div style='width:10em'><p>x</p>"),
        "end"
    );
}

#[test]
fn foster_parented_open_elements_hold_before_their_table() {
    // The div is moved before the table, so its text precedes the table.
    assert_eq!(frontier_of("<p>before</p><table><div>one"), "\"one\"");
}

#[test]
fn open_fixed_boxes_hold_everything() {
    assert_eq!(
        frontier_of("<p>a</p><div style='position:fixed'><p>head</p>"),
        frontier_of("<style>p:last-child { color: red }</style><p>a</p>")
    );
}

#[test]
fn open_svg_holds_from_its_root() {
    assert_eq!(
        frontier_of("<p>a</p><svg style='display:block'><rect/>"),
        "<svg>"
    );
}

#[test]
fn boxless_wrappers_belong_to_the_surrounding_run() {
    assert_eq!(
        frontier_of("<div>prefix <span style='display:contents'>inside"),
        "\"prefix \""
    );
}

#[test]
fn open_inline_layout_boxes_hold_from_the_surrounding_run() {
    assert_eq!(
        frontier_of("<div>prefix <span style='display:inline-flex'><b>item"),
        "\"prefix \""
    );
    assert_eq!(
        frontier_of("<div>prefix <aside style='float:left'><p>item</p>"),
        "\"prefix \""
    );
}
