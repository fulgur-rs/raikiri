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
    let document = assemble_document(uncascaded, &options, &RenderLimits::default(), None, None)
        .expect("cascade");
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
fn an_open_box_transformed_about_its_height_holds_from_its_start() {
    assert_eq!(
        frontier_of("<p>a</p><section style='transform: rotate(5deg)'><p>x</p>"),
        "<section>"
    );
    assert_eq!(
        frontier_of("<p>a</p><section style='transform: translateY(10%)'><p>x</p>"),
        "<section>"
    );
    // Translations by lengths, or along the width, do not use the height.
    assert_eq!(
        frontier_of("<p>a</p><section style='transform: translate(10%, 4px)'><p>x</p>"),
        "end"
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
    for width in [
        "max-content",
        "fit-content",
        "fit-content(10em)",
        "fit-content(50%)",
    ] {
        assert_eq!(
            frontier_of(&format!("<p>a</p><div style='width:{width}'><p>x</p>")),
            "<div>"
        );
    }
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
fn a_content_sized_width_on_a_box_it_does_not_size_moves_nothing() {
    // `width` sizes no box of a `display: contents` element, nor an inline
    // box that stays in flow. Both already belong to the surrounding run, so
    // the keyword leaves the frontier where it was.
    for width in ["min-content", "max-content", "fit-content"] {
        assert_eq!(
            frontier_of(&format!(
                "<p>a</p><div style='display:contents;width:{width}'><p>x</p>"
            )),
            frontier_of("<p>a</p><div style='display:contents'><p>x</p>"),
            "contents {width}"
        );
        assert_eq!(
            frontier_of(&format!("<div>prefix <span style='width:{width}'>inside")),
            frontier_of("<div>prefix <span>inside"),
            "inline {width}"
        );
    }
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

mod avoided_chain_tests;

thread_local! {
    static SIBLING_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_sibling_visit() {
    SIBLING_VISITS.with(|visits| visits.set(visits.get() + 1));
}

/// A chain of `depth` nested `div`s under the document. Returns the document
/// and the ids from the outermost.
fn chain_of_open_elements(depth: usize) -> (raikiri_dom::Document, Vec<usize>) {
    let mut document = raikiri_dom::Document::new();
    let mut parent = document.root_index();
    let mut chain = Vec::new();
    for _ in 0..depth {
        parent = document.append_element(Some(parent), "div", None::<&str>);
        chain.push(parent);
    }
    (document, chain)
}

#[test]
fn the_open_elements_of_a_deep_stack_are_found_in_linear_time() {
    let depth = 40_000;
    let (document, chain) = chain_of_open_elements(depth);
    let start = std::time::Instant::now();
    let open = open_elements(&document, &chain);
    let elapsed = start.elapsed();
    assert_eq!(open, chain);
    // Looking each handle up among the kept ones takes milliseconds; scanning
    // them for every handle takes seconds at this depth.
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "{elapsed:?} for {depth} open elements"
    );
}

#[test]
fn a_handle_is_open_when_its_parent_is_the_document_or_an_open_element() {
    let mut document = raikiri_dom::Document::new();
    let root = document.root_index();
    let html = document.append_element(Some(root), "html", None::<&str>);
    let head = document.append_element(Some(html), "head", None::<&str>);
    let body = document.append_element(Some(html), "body", None::<&str>);
    let outer = document.append_element(Some(body), "div", None::<&str>);
    let inner = document.append_element(Some(outer), "span", None::<&str>);
    // A closed element whose parent is not open, and an element that is not
    // in the tree.
    let closed_child = document.append_element(Some(head), "meta", None::<&str>);
    let detached = document.append_element(None, "p", None::<&str>);
    // The stack of open elements first, then a list of active formatting
    // elements that repeats `inner`, lists a stale element and one that is
    // not in the document.
    let traced = [html, body, outer, inner, inner, closed_child, detached];
    // The parent of each is kept before it, so only the handles that sit on a
    // chain from the document stay, once each and in the order of `traced`.
    assert_eq!(
        open_elements(&document, &traced),
        [html, body, outer, inner]
    );
    // A handle comes before its parent in the list: its parent is not kept
    // yet, so it is dropped (and not kept for the ones after it).
    assert_eq!(
        open_elements(&document, &[outer, html, outer, body]),
        [html, body]
    );
    // A closed `<head>` is a child of the open `<html>`, so it stays.
    assert_eq!(open_elements(&document, &[html, head]), [html, head]);
    assert_eq!(open_elements(&document, &[]), Vec::<usize>::new());
}

/// `open_elements` as it was when it scanned the elements kept so far for
/// every handle.
fn open_elements_by_scan(document: &raikiri_dom::Document, traced: &[usize]) -> Vec<usize> {
    let root = document.root_index();
    let mut open: Vec<usize> = Vec::new();
    for &id in traced {
        if open.contains(&id) {
            continue;
        }
        let parent = document.parent_of(id);
        if parent == Some(root) || parent.is_some_and(|parent| open.contains(&parent)) {
            open.push(id);
        }
    }
    open
}

#[test]
fn the_open_elements_are_the_ones_a_scan_of_the_kept_handles_keeps() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let (mut dropped, mut kept) = (0, 0);
    for _ in 0..2_000 {
        let mut document = raikiri_dom::Document::new();
        let mut ids = vec![document.root_index()];
        for _ in 0..next(40) {
            // Most elements go under an earlier one; some stay detached.
            let parent = (next(6) != 0).then(|| ids[next(ids.len())]);
            ids.push(document.append_element(parent, "div", None::<&str>));
        }
        // Handles in any order, with repeats.
        let traced: Vec<usize> = (0..next(60)).map(|_| ids[next(ids.len())]).collect();
        let open = open_elements(&document, &traced);
        assert_eq!(
            open,
            open_elements_by_scan(&document, &traced),
            "{traced:?}"
        );
        kept += open.len();
        dropped += traced.len() - open.len();
    }
    assert!(
        kept > 5_000 && dropped > 5_000,
        "{kept} kept, {dropped} dropped"
    );
}
