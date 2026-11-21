//! Where a box with a forced page break before it lands when the block before
//! it has a block-end margin that reaches past the page it ends on. The break
//! sits right after that block and cuts its margin, which the layout
//! collapsed with the box's own; only the box's own margin follows the break
//! (CSS Fragmentation 3, 5.2). The positions are the ones Chromium prints for
//! the same documents.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// Every word of `body` as `word@page:x,y`, the left and top of its glyphs on
/// its page, on 100x100 pages with no margin and 10px Ahem lines 10px apart.
fn words(body: &str) -> String {
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
         body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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
    let mut words = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("a page");
        for run in page.text_runs() {
            let mut x = run.origin.0;
            let y = (run.origin.1 - ASCENT).round();
            for word in run.text.split(' ') {
                if !word.trim().is_empty() {
                    words.push(format!("{}@{index}:{},{}", word.trim(), x.round(), y));
                }
                x += ADVANCE * (word.chars().count() + 1) as f32;
            }
        }
    }
    words.join(" ")
}

/// A 50px box with a block-end margin of `margin_bottom` and a line, then a
/// box with `style` and a line, then a line.
fn after_margin(margin_bottom: i32, style: &str) -> String {
    words(&format!(
        "<div style='height:50px;margin-bottom:{margin_bottom}px'>A</div>\
         <div style='{style}'>S</div><div>X</div>"
    ))
}

#[test]
fn a_forced_break_cuts_the_margin_of_the_block_before_it() {
    let cases = [
        (250, "break-before:page", "A@0:0,0 S@1:0,0 X@1:0,10"),
        (
            250,
            "break-before:page;margin-top:50px",
            "A@0:0,0 S@1:0,50 X@1:0,60",
        ),
        (
            250,
            "break-before:page;margin-top:20px",
            "A@0:0,0 S@1:0,20 X@1:0,30",
        ),
        (
            250,
            "break-before:page;margin-top:-10px",
            "A@0:0,0 S@0:0,90 X@1:0,0",
        ),
        (
            250,
            "break-before:page;margin-top:400px",
            "A@0:0,0 S@2:0,0 X@2:0,10",
        ),
        (80, "break-before:page", "A@0:0,0 S@1:0,0 X@1:0,10"),
    ];
    for (margin_bottom, style, expected) in cases {
        assert_eq!(
            after_margin(margin_bottom, style),
            expected,
            "{margin_bottom} {style}"
        );
    }
}

#[test]
fn a_break_after_the_block_cuts_its_margin_too() {
    assert_eq!(
        words(
            "<div style='height:50px;margin-bottom:250px;break-after:page'>A</div>\
             <div>S</div><div>X</div>"
        ),
        "A@0:0,0 S@1:0,0 X@1:0,10"
    );
    assert_eq!(
        words(
            "<div style='height:50px;margin-bottom:250px;break-after:page'>A</div>\
             <div style='margin-top:30px'>S</div><div>X</div>"
        ),
        "A@0:0,0 S@1:0,30 X@1:0,40"
    );
}

#[test]
fn the_margin_of_a_nested_or_padded_block_is_cut() {
    assert_eq!(
        words(
            "<div><div style='height:50px;margin-bottom:250px'>A</div></div>\
             <div style='break-before:page'>S</div><div>X</div>"
        ),
        "A@0:0,0 S@1:0,0 X@1:0,10"
    );
    assert_eq!(
        words(
            "<div style='height:50px;padding-bottom:10px;margin-bottom:150px'>A</div>\
             <div style='break-before:page'>S</div><div>X</div>"
        ),
        "A@0:0,0 S@1:0,0 X@1:0,10"
    );
    // A block that ends at a page edge, or runs onto later pages, puts the
    // break there.
    assert_eq!(
        words(
            "<div style='height:100px;margin-bottom:250px'>A</div>\
             <div style='break-before:page'>S</div><div>X</div>"
        ),
        "A@0:0,0 S@1:0,0 X@1:0,10"
    );
    assert_eq!(
        words(
            "<div style='height:250px;margin-bottom:250px'>A</div>\
             <div style='break-before:page'>S</div><div>X</div>"
        ),
        "A@0:0,0 S@3:0,0 X@3:0,10"
    );
}

#[test]
fn the_block_before_is_found_past_empty_boxes_and_contents_wrappers() {
    let cases = [
        // An empty box between the two, even one whose margins do not
        // collapse through it, leaves the break after the block.
        (
            "<div style='overflow:hidden'></div><div style='break-before:page'>S</div>",
            "A@0:0,0 S@2:0,0 X@2:0,10",
        ),
        (
            "<div style='clear:both'></div><div style='break-before:page'>S</div>",
            "A@0:0,0 S@2:0,0 X@2:0,10",
        ),
        // A box whose `display: contents` parent comes right after the block.
        (
            "<div style='display:contents'><div style='break-before:page'>S</div></div>",
            "A@0:0,0 S@1:0,0 X@1:0,10",
        ),
    ];
    for (after, expected) in cases {
        assert_eq!(
            words(&format!(
                "<div style='height:50px;margin-bottom:250px'>A</div>{after}<div>X</div>"
            )),
            expected,
            "{after}"
        );
    }
}

/// The page of `word` in `words`.
fn page_of(words: &str, word: &str) -> u32 {
    words
        .split(' ')
        .find_map(|entry| entry.strip_prefix(&format!("{word}@")))
        .and_then(|rest| rest.split(':').next())
        .and_then(|page| page.parse().ok())
        .unwrap_or_else(|| panic!("{word} in {words}"))
}

#[test]
fn content_between_the_block_and_the_break_keeps_its_place_before_it() {
    // The margin of an inline-block is part of its line, which Chromium ends
    // on the fourth page with the box after it: no block margin to cut.
    let inline = words(
        "<div style='height:40px'></div><span style='display:inline-block;width:10px;\
         height:10px;margin-bottom:250px;vertical-align:top'></span>\
         <div style='break-before:page'>S</div><div>X</div>",
    );
    assert!(page_of(&inline, "S") >= 3, "{inline}");
    // A generated block between the two comes before the box, on an earlier
    // page.
    let generated = words(
        "<style>.w::after{content:'G';display:block}</style>\
         <div style='display:contents' class=w><div style='height:50px;margin-bottom:250px'>A</div></div>\
         <div style='break-before:page'>S</div><div>X</div>",
    );
    assert!(
        page_of(&generated, "G") < page_of(&generated, "S"),
        "{generated}"
    );
}

#[test]
fn a_forced_break_at_the_start_of_a_wrapper_cuts_the_margin_before_the_wrapper() {
    // The break before the first box of a block sits before the block too,
    // and keeps the margins of the block and of the boxes it begins.
    let cases = [
        (
            "<div><div style='break-before:page'>S</div></div>",
            "S@1:0,0",
        ),
        (
            "<div><div><div style='break-before:page'>S</div></div></div>",
            "S@1:0,0",
        ),
        (
            "<div style='margin-top:20px'><div style='break-before:page'>S</div></div>",
            "S@1:0,20",
        ),
        (
            "<div><div style='break-before:page;margin-top:30px'>S</div></div>",
            "S@1:0,30",
        ),
        (
            "<div style='margin-top:10px'><div style='break-before:page;margin-top:30px'>S</div></div>",
            "S@1:0,30",
        ),
        (
            "<div style='padding-top:5px'><div style='break-before:page'>S</div></div>",
            "S@1:0,5",
        ),
        (
            "<div style='break-before:page'><div style='margin-top:30px'>S</div></div>",
            "S@1:0,30",
        ),
    ];
    for margin_bottom in [160, 250] {
        for (after, expected) in cases {
            let words = words(&format!(
                "<div style='height:50px;margin-bottom:{margin_bottom}px'>A</div>{after}"
            ));
            assert_eq!(
                words,
                format!("A@0:0,0 {expected}"),
                "{margin_bottom} {after}"
            );
        }
    }
}

#[test]
fn a_margin_over_several_pages_is_cut_at_its_first_page_edge() {
    // Without any forced break, the first page edge in the margin breaks the
    // page, even where the rest of the margin would end right at a later
    // edge, and the box after it starts the next page.
    for (height, margin_bottom) in [(50, 150), (50, 250), (100, 100), (100, 200), (30, 170)] {
        for after in [
            "<div>S</div>",
            "<div style='margin-top:20px'>S</div>",
            "<div><div>S</div></div>",
            "<div style='break-inside:avoid;height:30px'>S</div>",
        ] {
            let words = words(&format!(
                "<div style='height:{height}px;margin-bottom:{margin_bottom}px'>A</div>{after}"
            ));
            assert_eq!(words, "A@0:0,0 S@1:0,0", "{height} {margin_bottom} {after}");
        }
    }
    // The margin of a nested block, and one cut before the lines of a
    // paragraph, a single line of an inline-block among them.
    for (body, expected) in [
        (
            "<div><div style='height:50px;margin-bottom:150px'>A</div></div><div>S</div>",
            "A@0:0,0 S@1:0,0",
        ),
        (
            "<div><div style='height:50px;margin-bottom:160px'>A</div>S<br>T</div>",
            "A@0:0,0 S@1:0,0 T@1:0,10",
        ),
        (
            "<div><div style='height:50px;margin-bottom:150px'>A</div>S</div>",
            "A@0:0,0 S@1:0,0",
        ),
        (
            "<div><div style='height:50px;margin-bottom:160px'>A</div>\
             <span style='display:inline-block;width:10px;height:10px'></span></div><div>X</div>",
            "A@0:0,0 X@1:0,12",
        ),
    ] {
        assert_eq!(words(body), expected, "{body}");
    }
}
