//! A forced break or a change of page type after a block among the lines of
//! a box with a fixed height. The break moves the lines after the block to
//! the next page, but the box keeps its height: its fragment on the page the
//! break leaves ends there, and the rest of its height follows the break, so
//! the content after the box stays where it was. Lines moved past the end of
//! a box that clips them are hidden and reach no page. Where each glyph falls
//! and the page count are what Chromium prints for the same documents on
//! pages 100px square (200px wide for the page type `a`), with Ahem glyphs of
//! 20px on lines of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The distance from the top of a 30px line to the top of its 20px Ahem
/// glyphs.
const HALF_LEADING: f32 = 5.0;

/// The page count of `body`, then each word of it made of the glyphs
/// `glyphs` as `word@page:x,y`, the left of its first glyph and the top of
/// its line on its page, sorted.
fn pages(body: &str, glyphs: &str) -> String {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} @page a{{size:200px 100px}} \
         .a{{page:a}} .b{{page:b}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let top = (run.origin.1 - run.ascent - HALF_LEADING).round();
            for word in run.text.split_whitespace() {
                if word.chars().all(|glyph| glyphs.contains(glyph)) {
                    words.push(format!("{word}@{index}:{},{top}", run.origin.0.round()));
                }
            }
        }
    }
    words.sort();
    format!("{}p {}", result.page_count(), words.join(" "))
}

/// A box `style` of the page type `class` holding a block `B` 20px tall
/// of the page type `block_class` (with `block_style`) and then the lines
/// `lines` in its own paragraph, followed by a block `X` of the page type
/// `after`.
fn after_block(class: &str, style: &str, block: (&str, &str), lines: &str, after: &str) -> String {
    let (block_class, block_style) = block;
    let lines = lines
        .chars()
        .map(String::from)
        .collect::<Vec<_>>()
        .join("<br>");
    format!(
        "<div class='{class}' style='{style}'>\
         <div class='{block_class}' style='height:20px;{block_style}'>B</div>{lines}</div>\
         <div class='{after}'>X</div>"
    )
}

#[test]
fn a_forced_break_among_the_lines_of_a_fixed_height_box_leaves_the_content_after_it() {
    for (style, lines, glyphs, expected) in [
        // The lines past the end of the box that clips them are hidden.
        (
            "height:150px;overflow:hidden",
            "abcdef",
            "BXab",
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        (
            "height:150px;overflow:hidden",
            "ab",
            "BXab",
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        // Without a clip, the lines run on past the end of the box, under the
        // content after it.
        (
            "height:150px",
            "abcdef",
            "BXabcdef",
            "3p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30 c@1:0,60 d@2:0,0 e@2:0,30 f@2:0,60",
        ),
        (
            "height:150px",
            "ab",
            "BXab",
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        // A box whose height follows its content grows with the lines.
        (
            "overflow:hidden",
            "abcdef",
            "BXabcdef",
            "4p B@0:0,0 X@3:0,0 a@1:0,0 b@1:0,30 c@1:0,60 d@2:0,0 e@2:0,30 f@2:0,60",
        ),
    ] {
        let body = after_block("", style, ("", "break-after:page"), lines, "");
        assert_eq!(pages(&body, glyphs), expected, "{body}");
    }
}

#[test]
fn a_change_of_page_type_among_the_lines_of_a_fixed_height_box_leaves_the_content_after_it() {
    for (style, lines, after, glyphs, expected) in [
        // The lines take the page type of their box, after the block that
        // has one of its own; the content after the box stays on the page
        // the box ends on.
        (
            "height:150px;overflow:hidden",
            "abcdef",
            "a",
            "BXab",
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        (
            "height:150px",
            "ab",
            "a",
            "BXab",
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        // Content of another page type after the box starts the page after
        // the one the box ends on, and the box's hidden lines reach no page.
        (
            "height:150px;overflow:hidden",
            "abcdef",
            "",
            "BXab",
            "3p B@0:0,0 X@2:0,0 a@1:0,0 b@1:0,30",
        ),
        (
            "height:190px;overflow:hidden",
            "abcdef",
            "",
            "BXabc",
            "3p B@0:0,0 X@2:0,0 a@1:0,0 b@1:0,30 c@1:0,60",
        ),
        // The lines break across the pages as they would from the top of
        // the page the change opens.
        (
            "height:150px",
            "abcdef",
            "",
            "BXabcdef",
            "3p B@0:0,0 X@2:0,0 a@1:0,0 b@1:0,30 c@1:0,60 d@2:0,0 e@2:0,30 f@2:0,60",
        ),
    ] {
        let body = after_block("a", style, ("b", ""), lines, after);
        assert_eq!(pages(&body, glyphs), expected, "{body}");
    }
}

#[test]
fn a_clip_that_holds_forced_breaks_holds_a_change_of_page_type() {
    // The box that clips ends on the page the change would leave, so the rest
    // of it prints there in one piece, as after a forced break in it.
    for style in ["height:100px", "height:80px", "max-height:80px"] {
        let body = format!(
            "<div style='{style};overflow:hidden'><div class='b' style='height:20px'>B</div>\
             <div class='a'>A</div></div><div>X</div>"
        );
        assert_eq!(pages(&body, "ABX"), "2p A@0:0,20 B@0:0,0 X@1:0,0", "{body}");
    }
}

#[test]
fn boxes_with_a_page_type_below_the_end_of_a_clip_add_no_page() {
    // The boxes start below the end of the box that clips them: they are
    // hidden, and the content after the box starts the page after it.
    for (after, inner) in [
        ("", "<div class='a'>A</div><div class='b'>B</div>"),
        ("a", "<div class='a'>A</div>"),
    ] {
        let body = format!(
            "<div style='height:100px;overflow:hidden'><div style='height:120px'></div>{inner}</div>\
             <div class='{after}'>X</div>"
        );
        assert_eq!(pages(&body, "X"), "2p X@1:0,0", "{body}");
    }
}

#[test]
fn the_nearest_fixed_height_box_around_the_paragraph_keeps_its_end() {
    // The paragraph's own box grows with its lines; the box around it that
    // keeps its height ends where it did. Out-of-flow boxes on the way keep
    // their own flow.
    let lines = "<div><div style='height:20px;break-after:page'>B</div>a<br>b</div>";
    for (body, expected) in [
        (
            format!("<div style='height:150px'>{lines}</div><div>X</div>"),
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        (
            "<div class='a' style='height:150px'><div><div class='b' style='height:20px'>B</div>\
                 a<br>b</div></div><div class='a'>X</div>"
                .to_string(),
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        (
            format!(
                "<div style='position:relative;height:150px'>\
                 <div style='position:absolute;top:0;height:150px'>{lines}</div></div><div>X</div>"
            ),
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        (
            format!(
                "<div style='position:relative;height:150px'>\
                 <div style='position:absolute;top:0'>{lines}</div></div><div>X</div>"
            ),
            "2p B@0:0,0 X@1:0,50 a@1:0,0 b@1:0,30",
        ),
        (
            "<div style='height:150px'><div style='float:left;width:40px'>\
             <div style='height:20px;break-after:page'>B</div>a<br>b</div></div>\
             <div style='clear:both'>X</div>"
                .to_string(),
            "2p B@0:0,0 X@1:0,60 a@1:0,0 b@1:0,30",
        ),
    ] {
        assert_eq!(pages(&body, "BXab"), expected, "{body}");
    }
}
