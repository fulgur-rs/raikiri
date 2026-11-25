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
    words_and_pages(body).0
}

/// [`words`] with the number of pages.
fn words_and_pages(body: &str) -> (String, usize) {
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
    (words.join(" "), result.page_count() as usize)
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

#[test]
fn content_between_the_block_and_the_break_keeps_its_place_before_it() {
    // The margin of an inline-block is part of its line: no block margin to
    // cut. The line starts the second page and runs on to the fourth, where
    // the box after it follows (see the test below).
    let inline = words(
        "<div style='height:40px'></div><span style='display:inline-block;width:10px;\
         height:10px;margin-bottom:250px;vertical-align:top'></span>\
         <div style='break-before:page'>S</div><div>X</div>",
    );
    assert_eq!(inline, "S@3:0,60 X@3:0,70");
    // A generated block between the two starts the page after the margin's
    // first page edge, which cuts the margin, and the box follows it on the
    // page after.
    let generated = words(
        "<style>.w::after{content:'G';display:block}</style>\
         <div style='display:contents' class=w><div style='height:50px;margin-bottom:250px'>A</div></div>\
         <div style='break-before:page'>S</div><div>X</div>",
    );
    assert_eq!(generated, "A@0:0,0 G@1:0,0 S@2:0,0 X@2:0,10");
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

#[test]
fn a_margin_inside_its_page_keeps_the_lines_below_it() {
    // On a later page, the lines after a block whose margin ends on that
    // page stay below the margin; one that reaches the next page is cut.
    for (body, expected) in [
        (
            "<div style='height:150px'>Z</div>\
             <div><div style='height:10px;margin-bottom:10px'>A</div>S<br>T</div>",
            "Z@0:0,0 A@1:0,50 S@1:0,70 T@1:0,80",
        ),
        (
            "<div style='height:150px'>Z</div>\
             <div><div style='height:10px;margin-bottom:10px'>A</div>\
             <span style='display:inline-block;width:10px;height:10px'></span></div><div>X</div>",
            "Z@0:0,0 A@1:0,50 X@1:0,82",
        ),
        (
            "<div style='height:150px'>Z</div>\
             <div><div style='height:10px;margin-bottom:160px'>A</div>S<br>T</div>",
            "Z@0:0,0 A@1:0,50 S@2:0,0 T@2:0,10",
        ),
    ] {
        let mut words = words(body)
            .split(' ')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        words.sort_by_key(|word| {
            let (page, y) = word.split_once('@').unwrap().1.split_once(':').unwrap();
            (
                page.to_owned(),
                y.split(',').nth(1).unwrap().parse::<i32>().unwrap(),
            )
        });
        assert_eq!(words.join(" "), expected, "{body}");
    }
}

#[test]
fn a_forced_break_after_a_line_that_runs_past_its_page_opens_the_next_page() {
    // No page break goes through a line, so what runs past the page it starts
    // on is overflow. The break opens the page after the line's, and the box
    // keeps its place in the flow below the overflow there, as in Chromium.
    let tall = "<span style='display:inline-block;width:10px;height:10px;\
                margin-bottom:150px;vertical-align:top'></span>";
    let cases = [
        (
            format!("{tall}<div style='break-before:page'>S</div>"),
            "S@2:0,60 X@2:0,70",
        ),
        // The line holds text too.
        (
            format!("R{tall}<div style='break-before:page'>S</div>"),
            "S@2:0,60 X@2:0,70",
        ),
        // A box of the line's height in place of its margin.
        (
            "<span style='display:inline-block;width:10px;height:160px;\
             vertical-align:top'></span><div style='break-before:page'>S</div>"
                .to_owned(),
            "S@2:0,60 X@2:0,70",
        ),
        // A line below the baseline of a box as tall as the line.
        (
            "<span style='display:inline-block;width:10px;height:160px'></span>\
             <div style='break-before:page'>S</div>"
                .to_owned(),
            "S@2:0,62 X@2:0,72",
        ),
        // Blocks around the line end with it.
        (
            format!("<div>{tall}</div><div style='break-before:page'>S</div>"),
            "S@2:0,60 X@2:0,70",
        ),
        (
            format!("<div><div>{tall}</div></div><div style='break-before:page'>S</div>"),
            "S@2:0,60 X@2:0,70",
        ),
        // The break at the start of a wrapper.
        (
            format!("<div>{tall}</div><div><div style='break-before:page'>S</div></div>"),
            "S@2:0,60 X@2:0,70",
        ),
        // The box keeps its own margin below the overflow.
        (
            format!("{tall}<div style='break-before:page;margin-top:20px'>S</div>"),
            "S@2:0,80 X@2:0,90",
        ),
        // The margin of a block around the line goes with the break.
        (
            format!(
                "<div style='margin-bottom:20px'>{tall}</div><div style='break-before:page'>S</div>"
            ),
            "S@2:0,60 X@2:0,70",
        ),
        // A relative offset moves the line only where it paints.
        (
            format!(
                "<div style='position:relative;top:5px'><div>{tall}</div></div><div style='break-before:page'>S</div>"
            ),
            "S@2:0,60 X@2:0,70",
        ),
        // A relative offset on the block that holds the line.
        (
            format!(
                "<div style='position:relative;top:5px'>{tall}</div><div style='break-before:page'>S</div>"
            ),
            "S@2:0,60 X@2:0,70",
        ),
        // A block of set height runs on past its line, and the break follows
        // it.
        (
            "<div style='height:165px'>R</div><div style='break-before:page'>S</div>".to_owned(),
            "S@3:0,0 X@3:0,10",
        ),
        // An empty box opens the page after the overflow, and the margin that
        // collapses through it goes with it.
        (
            format!("{tall}<div style='break-before:page;margin-top:20px'></div><div>S</div>"),
            "S@3:0,20 X@3:0,30",
        ),
        // Padding below the line is no line: the break follows it.
        (
            format!(
                "<div style='padding-bottom:10px'>{tall}</div><div style='break-before:page'>S</div>"
            ),
            "S@3:0,0 X@3:0,10",
        ),
    ];
    for (content, expected) in cases {
        let words = words(&format!(
            "<div style='height:40px'></div>{content}<div>X</div>"
        ));
        let boxes = words
            .split(' ')
            .filter(|word| word.starts_with("S@") || word.starts_with("X@"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(boxes, expected, "{content}: {words}");
    }
}

#[test]
fn a_cut_margin_leaves_no_page_after_the_content() {
    // The box that a forced break takes up to the page after the margin's
    // first page edge takes the end of the flow up with it, as an unforced
    // break that cuts the margin does: no page follows the content.
    for (body, expected) in [
        (
            "<div style='height:50px;margin-bottom:150px'>A</div>\
             <div style='break-before:page'>S</div><div>X</div>",
            ("A@0:0,0 S@1:0,0 X@1:0,10", 2),
        ),
        (
            "<div style='height:50px;margin-bottom:250px'>A</div>\
             <div style='break-before:page'>S</div><div>X</div>",
            ("A@0:0,0 S@1:0,0 X@1:0,10", 2),
        ),
        (
            "<div style='height:50px;margin-bottom:150px;break-after:page'>A</div><div>S</div>",
            ("A@0:0,0 S@1:0,0", 2),
        ),
        (
            "<div><div style='height:50px;margin-bottom:150px'>A</div></div>\
             <div style='break-before:page;height:30px'>S</div>",
            ("A@0:0,0 S@1:0,0", 2),
        ),
        (
            "<div style='float:left;width:10px;height:10px'></div>\
             <div style='height:50px;margin-bottom:150px'>A</div>\
             <div style='break-before:page'>S</div>",
            ("A@0:10,0 S@1:0,0", 2),
        ),
    ] {
        let (words, pages) = words_and_pages(body);
        assert_eq!((words.as_str(), pages), expected, "{body}");
    }
}

#[test]
fn a_float_below_a_cut_margin_starts_the_page_with_the_box_after_it() {
    // The page edge cuts the margin above the float as it cuts the one above
    // the box after it, which sits beside the float at the top of the page.
    for (body, expected) in [
        (
            "<div style='height:50px;margin-bottom:150px'>A</div>\
             <div style='float:left;width:10px;height:10px'>F</div><div>S</div><div>X</div>",
            ("A@0:0,0 F@1:0,0 S@1:10,0 X@1:0,10", 2),
        ),
        (
            "<div style='height:50px;margin-bottom:250px'>A</div>\
             <div style='float:left;width:10px;height:10px'></div><div>S</div><div>X</div>",
            ("A@0:0,0 S@1:10,0 X@1:0,10", 2),
        ),
        // A float in the margin above a box that ends it keeps its place.
        (
            "<div style='height:50px;margin-bottom:30px'>A</div>\
             <div style='float:left;width:10px;height:10px'></div><div>S</div><div>X</div>",
            ("A@0:0,0 S@0:10,80 X@0:0,90", 1),
        ),
    ] {
        let (words, pages) = words_and_pages(body);
        assert_eq!((words.as_str(), pages), expected, "{body}");
    }
}
