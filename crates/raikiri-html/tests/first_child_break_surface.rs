//! A forced break before the first child of a block. A page does not break
//! between the start of a block and its first child, so the break is the one
//! before the block: it shares the break that already opened the page, and the
//! block keeps its padding, border and margin on the new page. The positions
//! are the ones Chromium prints for the same documents (`page.pdf` with the
//! page size taken from `@page`, the text read from the rendered pages).

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// A forced break after a 30px block.
const BREAK: &str = "<div style='break-after:page;height:30px'>a</div>";
/// Two lines of two words.
const CHILD: &str = "<div style='break-before:page'>XX XX XX XX</div>";

fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

/// The number of pages, and the page, left edge and top of every line of text
/// but the lead-in `a`. A 20px glyph in a 30px line sits 5px below the line.
fn pages(body: &str) -> (usize, Vec<(u32, f32, f32)>) {
    let result = lay_out(body);
    let mut count = 0;
    let mut lines = Vec::new();
    for index in 0..6 {
        let Some(page) = result.page(index) else {
            continue;
        };
        count += 1;
        for run in page.text_runs() {
            if run.text.trim_end() != "a" {
                lines.push((index, run.origin.0, run.origin.1 - ASCENT));
            }
        }
    }
    (count, lines)
}

/// The text of `CHILD` on the second page, its first line `top` down the page.
fn on_second_page(top: f32) -> (usize, Vec<(u32, f32, f32)>) {
    (2, vec![(1, 0.0, top), (1, 0.0, top + 30.0)])
}

#[test]
fn the_edges_of_the_block_stay_with_the_child_on_the_page_the_break_opened() {
    // The break before the child does not open a page of its own after the
    // one the block before the wrapper forced: the wrapper's padding, border
    // and margin stay above the child.
    for (wrapper, top) in [
        ("", 5.0),
        ("padding-top:5px", 10.0),
        ("border-top:5px solid red", 10.0),
        ("margin-top:10px", 15.0),
        ("padding-top:5px;margin-top:10px", 20.0),
        ("overflow:hidden;padding-top:5px", 10.0),
    ] {
        let body = format!("{BREAK}<div style='{wrapper}'>{CHILD}</div>");
        assert_eq!(pages(&body), on_second_page(top), "wrapper {wrapper:?}");
    }
}

#[test]
fn every_block_the_child_begins_keeps_its_edges() {
    let body = format!(
        "{BREAK}<div style='padding-top:5px'><div style='padding-top:7px'>{CHILD}</div></div>"
    );
    assert_eq!(pages(&body), on_second_page(17.0));
    // The margin of the wrapper collapses with the larger margin of the child.
    let body = format!(
        "{BREAK}<div style='margin-top:10px'>\
         <div style='margin-top:20px;break-before:page'>XX XX XX XX</div></div>"
    );
    assert_eq!(pages(&body), on_second_page(25.0));
    // A list item begins the list.
    let body = format!(
        "{BREAK}<ul style='margin:0;padding:5px 0 0 0;list-style:none'>\
         <li style='break-before:page'>XX XX XX XX</li></ul>"
    );
    assert_eq!(pages(&body), on_second_page(10.0));
}

#[test]
fn white_space_and_out_of_flow_boxes_do_not_come_before_the_child() {
    let body = format!("{BREAK}<div style='padding-top:5px'>\n  {CHILD}\n</div>");
    assert_eq!(pages(&body), on_second_page(10.0));
    let body = format!(
        "{BREAK}<div style='padding-top:5px'>\
         <div style='position:absolute;width:0;height:0'></div>{CHILD}</div>"
    );
    assert_eq!(pages(&body), on_second_page(10.0));
}

#[test]
fn a_box_or_text_before_the_child_keeps_its_break() {
    // The wrapper starts the second page with what comes before the child, and
    // the child begins the third.
    for before in ["<div style='height:10px'></div>", "<div></div>"] {
        let body = format!("{BREAK}<div style='padding-top:5px'>{before}{CHILD}</div>");
        assert_eq!(
            pages(&body),
            (3, vec![(2, 0.0, 5.0), (2, 0.0, 35.0)]),
            "before {before:?}"
        );
    }
    let body = format!("{BREAK}<div style='padding-top:5px'>b{CHILD}</div>");
    assert_eq!(
        pages(&body),
        (3, vec![(1, 0.0, 10.0), (2, 0.0, 5.0), (2, 0.0, 35.0)])
    );
}

#[test]
fn a_break_before_the_first_box_of_the_document_opens_no_page() {
    // Nothing comes before the wrapper, so the break has no page to end.
    let nested =
        format!("<div style='padding-top:5px'><div style='padding-top:7px'>{CHILD}</div></div>");
    for (body, top) in [
        (format!("<div>{CHILD}</div>"), 5.0),
        (format!("<div style='padding-top:5px'>{CHILD}</div>"), 10.0),
        (nested, 17.0),
    ] {
        let expected = (1, vec![(0, 0.0, top), (0, 0.0, top + 30.0)]);
        assert_eq!(pages(&body), expected, "{body}");
    }
}

#[test]
fn the_edges_of_the_block_stay_beside_the_float_that_started_the_page() {
    // The float is 50px tall, so the first two 30px lines are beside it and
    // the third is below it, if the page has room.
    let float = "<div style='float:left;width:40px;height:50px'></div>";
    for (wrapper, top) in [
        ("", 5.0),
        ("padding-top:5px", 10.0),
        ("border-top:5px solid red", 10.0),
        ("margin-top:10px", 15.0),
    ] {
        let body = format!("{BREAK}{float}<div style='{wrapper}'>{CHILD}</div>");
        let expected = (
            2,
            vec![(1, 40.0, top), (1, 40.0, top + 30.0), (1, 0.0, top + 60.0)],
        );
        assert_eq!(pages(&body), expected, "wrapper {wrapper:?}");
    }
    // The third line does not fit the page below the edges, and starts the next.
    let body = format!("{BREAK}{float}<div style='padding-top:5px;margin-top:10px'>{CHILD}</div>");
    let expected = (3, vec![(1, 40.0, 20.0), (1, 40.0, 50.0), (2, 0.0, 5.0)]);
    assert_eq!(pages(&body), expected);
    let body = format!(
        "{BREAK}{float}<div style='padding-top:5px'><div style='padding-top:7px'>{CHILD}</div></div>"
    );
    let expected = (3, vec![(1, 40.0, 17.0), (1, 40.0, 47.0), (2, 0.0, 5.0)]);
    assert_eq!(pages(&body), expected);
}

fn box_extents(body: &str, id: &str) -> Vec<(usize, f32, f32)> {
    let document = lay_out(body);
    let mut found = Vec::new();
    for (index, page) in document.pages().enumerate() {
        let dom = page.dom();
        found.extend(
            page.fragments()
                .filter(|fragment| {
                    fragment.kind() == FragmentKind::Box
                        && dom.attr(fragment.node(), "id") == Some(id)
                })
                .map(|fragment| {
                    let rect = fragment.rect();
                    (index, rect.y, rect.y + rect.height)
                }),
        );
    }
    found
}

#[test]
fn a_break_before_the_first_child_takes_the_midpage_wrappers_edges_with_it() {
    for (wrapper, top, start, end) in [
        ("padding-top:5px", 10.0, 0.0, 65.0),
        ("border-top:5px solid red", 10.0, 0.0, 65.0),
        ("margin-top:10px", 15.0, 10.0, 70.0),
        ("padding-top:5px;margin-top:10px", 20.0, 10.0, 75.0),
    ] {
        let body =
            format!("<div style='height:30px'>a</div><div id=o style='{wrapper}'>{CHILD}</div>");
        assert_eq!(pages(&body), on_second_page(top), "{wrapper}");
        assert_eq!(box_extents(&body, "o"), [(1, start, end)], "{wrapper}");
    }
}

#[test]
fn a_separate_first_child_break_after_an_empty_sibling_keeps_its_wrappers_edges() {
    for (wrapper, top, start, end) in [
        ("padding-top:5px", 10.0, 0.0, 65.0),
        ("border-top:5px solid red", 10.0, 0.0, 65.0),
        ("margin-top:10px", 15.0, 10.0, 70.0),
    ] {
        let body =
            format!("{BREAK}<div style='height:0'></div><div id=o style='{wrapper}'>{CHILD}</div>");
        assert_eq!(
            pages(&body),
            (3, vec![(2, 0.0, top), (2, 0.0, top + 30.0)]),
            "{wrapper}"
        );
        assert_eq!(box_extents(&body, "o"), [(2, start, end)], "{wrapper}");
    }
}

#[test]
fn a_new_first_child_break_keeps_nested_edges_and_collapsed_margins() {
    for (content, top, start, end) in [
        (format!("<div id=o style='padding-top:5px'><div style='padding-top:7px'>{CHILD}</div></div>"), 17.0, 0.0, 72.0),
        ("<div id=o style='margin-top:10px'><div style='break-before:page;margin-top:20px'>XX XX XX XX</div></div>".to_owned(), 25.0, 20.0, 80.0),
    ] {
        let body = format!("<div style='height:30px'>a</div>{content}");
        assert_eq!(pages(&body), on_second_page(top));
        assert_eq!(box_extents(&body, "o"), [(1, start, end)]);
    }
}

#[test]
fn children_and_outer_siblings_follow_the_holder_once() {
    for nested in [false, true] {
        let content = if nested {
            format!(
                "<section><div style='height:10px'></div><div id=o style='padding-top:5px'>{CHILD}</div><div id=t style='height:10px'></div></section>"
            )
        } else {
            format!(
                "<div id=o style='padding-top:5px'>{CHILD}<div id=t style='height:10px'></div></div>"
            )
        };
        let body = format!(
            "<div style='height:30px'>a</div>{content}<div id=z style='height:10px'></div>"
        );
        assert_eq!(pages(&body), on_second_page(10.0), "nested {nested}");
        assert_eq!(
            box_extents(&body, "o"),
            [(1, 0.0, if nested { 65.0 } else { 75.0 })]
        );
        assert_eq!(
            box_extents(&body, "t"),
            [(1, 65.0, 75.0)],
            "nested {nested}"
        );
        assert_eq!(
            box_extents(&body, "z"),
            [(1, 75.0, 85.0)],
            "nested {nested}"
        );
    }
}

#[test]
fn a_relative_offset_paints_the_forced_holder_without_moving_its_following_sibling() {
    let body = format!(
        "<div style='height:30px'>a</div><div id=o style='padding-top:5px;position:relative;top:7px'>{CHILD}</div><div id=z style='height:10px'></div>"
    );
    assert_eq!(pages(&body), on_second_page(17.0));
    assert_eq!(box_extents(&body, "o"), [(1, 7.0, 72.0)]);
    assert_eq!(box_extents(&body, "z"), [(1, 65.0, 75.0)]);
}

#[test]
fn a_negative_holder_margin_keeps_the_flow_after_its_actual_end() {
    for nested in [false, true] {
        let holder = format!("<div id=o style='padding-top:5px;margin-top:-10px'>{CHILD}</div>");
        let content = if nested {
            format!(
                "<section><div style='height:10px'></div>{holder}<div id=t style='height:10px'></div></section>"
            )
        } else {
            holder
        };
        let body = format!(
            "<div style='height:30px'>a</div>{content}<div id=z style='height:10px'></div>"
        );
        assert_eq!(pages(&body), on_second_page(0.0));
        assert_eq!(box_extents(&body, "o"), [(0, 90.0, 100.0), (1, 0.0, 55.0)]);
        let next = if nested { 65.0 } else { 55.0 };
        assert_eq!(box_extents(&body, "z"), [(1, next, next + 10.0)]);
        if nested {
            assert_eq!(box_extents(&body, "t"), [(1, 55.0, 65.0)]);
        }
    }
}

#[test]
fn an_absolute_box_before_the_child_keeps_its_containing_blocks_coordinates() {
    for own in [false, true] {
        let position = if own { "position:relative" } else { "" };
        let body = format!(
            "<div style='height:30px'>a</div><div id=o style='padding-top:5px;{position}'><div id=q style='position:absolute;top:10px;left:80px;width:5px;height:5px'></div>{CHILD}</div>"
        );
        assert_eq!(pages(&body), on_second_page(10.0), "own {own}");
        assert_eq!(box_extents(&body, "o"), [(1, 0.0, 65.0)]);
        assert_eq!(
            box_extents(&body, "q"),
            [(usize::from(own), 10.0, 15.0)],
            "own {own}"
        );
    }
}

#[test]
fn generated_before_content_is_a_boundary_even_when_it_has_zero_height() {
    for before in [true, false] {
        let pseudo = if before { "before" } else { "after" };
        let body = format!(
            "<style>#o::{pseudo}{{content:'';display:block;height:0}}</style><div style='height:30px'>a</div><div id=o style='padding-top:5px'>{CHILD}</div>"
        );
        assert_eq!(
            pages(&body),
            on_second_page(if before { 5.0 } else { 10.0 })
        );
        let fragments = box_extents(&body, "o");
        if before {
            // This checks ownership without pinning the pre-existing missing
            // continuation extent of a wrapper with preceding generated content.
            assert!(
                fragments
                    .iter()
                    .any(|&(page, start, _)| page == 0 && start == 30.0)
            );
        } else {
            assert_eq!(fragments, [(1, 0.0, 65.0)]);
        }
    }
}

#[test]
fn a_first_child_break_crosses_a_contents_wrapper() {
    let body = format!(
        "<div style='height:30px'>a</div><div id=o style='padding-top:5px'><span style='display:contents'>{CHILD}</span></div>"
    );
    assert_eq!(pages(&body), on_second_page(10.0));
    assert_eq!(box_extents(&body, "o"), [(1, 0.0, 65.0)]);
}

#[test]
fn a_relative_holder_keeps_its_childs_larger_collapsed_margin() {
    let body = "<div style='height:30px'>a</div><div id=o style='position:relative;top:7px;margin-top:10px'><div style='break-before:page;margin-top:20px'>XX XX XX XX</div></div>";
    assert_eq!(pages(body), on_second_page(32.0));
    assert_eq!(box_extents(body, "o"), [(1, 27.0, 87.0)]);
}

#[test]
fn a_leading_forced_break_keeps_large_padding_from_an_earlier_page() {
    for (padding, count, expected, fragments) in [
        (
            70,
            3,
            vec![(1, 0.0, 75.0), (2, 0.0, 5.0)],
            vec![(1, 0.0, 100.0), (2, 0.0, 30.0)],
        ),
        (
            85,
            3,
            vec![(2, 0.0, 5.0), (2, 0.0, 35.0)],
            vec![(1, 0.0, 100.0), (2, 0.0, 60.0)],
        ),
        (
            170,
            4,
            vec![(2, 0.0, 75.0), (3, 0.0, 5.0)],
            vec![(1, 0.0, 100.0), (2, 0.0, 100.0), (3, 0.0, 30.0)],
        ),
    ] {
        let body = format!(
            "<div style='height:30px'>a</div><div id=o style='padding-top:{padding}px'>{CHILD}</div>"
        );
        assert_eq!(pages(&body), (count, expected), "padding {padding}");
        assert_eq!(box_extents(&body, "o"), fragments, "padding {padding}");
    }
}

#[test]
fn forced_leading_lines_split_when_the_second_line_straddles_a_page() {
    for avoid in ["", "break-inside:avoid"] {
        let body = format!(
            "<div style='height:30px'>a</div><div id=o style='padding-top:65px'><div style='break-before:page;{avoid}'>XX XX XX XX</div></div>"
        );
        assert_eq!(
            pages(&body),
            (3, vec![(1, 0.0, 70.0), (2, 0.0, 5.0)]),
            "{avoid}"
        );
        assert_eq!(box_extents(&body, "o"), [(1, 0.0, 100.0), (2, 0.0, 30.0)]);
    }
}

#[test]
fn large_leading_edges_keep_the_first_document_and_shared_break_boundaries() {
    for (prefix, page) in [("", 0), (BREAK, 1), ("<div style='height:30px'>a</div>", 1)] {
        let content = if prefix == "<div style='height:30px'>a</div>" {
            format!(
                "<div id=o style='padding-top:35px'><div style='padding-top:35px'>{CHILD}</div></div>"
            )
        } else {
            format!("<div id=o style='padding-top:70px'>{CHILD}</div>")
        };
        let body = format!("{prefix}{content}");
        assert_eq!(
            pages(&body),
            (
                page as usize + 2,
                vec![(page, 0.0, 75.0), (page + 1, 0.0, 5.0)]
            )
        );
        assert_eq!(
            box_extents(&body, "o"),
            [(page as usize, 0.0, 100.0), (page as usize + 1, 0.0, 30.0)]
        );
    }
}

#[test]
fn a_paragraph_with_an_atomic_keeps_the_whole_box_fallback() {
    let body = "<div style='height:30px'>a</div><div style='padding-top:65px'><div style='break-before:page'>XX XX <span id=q style='display:inline-block;width:20px;height:20px'></span> XX</div></div>";
    let mut atomic = box_extents(body, "q");
    atomic.dedup();
    assert_eq!(atomic.len(), 1);
    assert_eq!(atomic[0].2 - atomic[0].1, 20.0);
}

#[test]
fn a_relative_child_keeps_the_margin_collapsed_through_its_holder() {
    for (content, top, start, end) in [
        (
            "<div style='position:relative;top:7px;margin-top:20px;break-before:page'>XX XX XX XX</div>",
            32.0,
            20.0,
            80.0,
        ),
        (
            "<div style='margin-top:20px'><div style='position:relative;top:7px;margin-top:30px;break-before:page'>XX XX XX XX</div></div>",
            42.0,
            30.0,
            90.0,
        ),
    ] {
        let body = format!(
            "<div style='height:30px'>a</div><div id=o style='margin-top:10px'>{content}</div>"
        );
        assert_eq!(pages(&body), on_second_page(top));
        assert_eq!(box_extents(&body, "o"), [(1, start, end)]);
    }
}
