//! A block inside an inline box ends with the line that closes the inline
//! box, empty as that line may be. A break forced after the block leaves that
//! line to start the next page, where it is content: a break forced after the
//! paragraph, or before the box after it, opens one more page. The page counts
//! and line positions are the ones Chromium prints for the same documents
//! (`page.pdf` with the page size taken from `@page`).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each text run as `text@page:top`, sorted, with the page count, on pages
/// 100px square with 30px lines.
fn lines(body: &str) -> (usize, Vec<String>) {
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
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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
    let mut runs = Vec::new();
    let mut pages = 0;
    for (index, page) in result.pages().enumerate() {
        pages += 1;
        for run in page.text_runs() {
            // The 20px Ahem baseline sits 21px below the top of its 30px line.
            let top = (run.origin.1 - 21.0).round() as i32;
            runs.push(format!("{}@{index}:{top}", run.text));
        }
    }
    runs.sort();
    (pages, runs)
}

fn expect(body: &str, pages: usize, runs: &[&str]) {
    let mut runs: Vec<String> = runs.iter().map(|run| run.to_string()).collect();
    runs.sort();
    assert_eq!(lines(body), (pages, runs), "{body}");
}

#[test]
fn the_line_closing_an_inline_box_after_a_forced_break_takes_a_page_of_its_own() {
    // A forced break after a block inside an inline box leaves the line that
    // closes the inline box, empty as it is, to start the next page: a break
    // forced after the paragraph, or before the box after it, opens one more
    // page.
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='break-after:page'><div style='height:20px;break-after:page'>T</div></div></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><div><span><div style='height:20px;break-after:page'>T</div></span></div></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></div><div>Z</div></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><span><div style='height:20px;break-after:page'>T</div></span></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div> </span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px'>S</div><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            3,
            &["S@0:0", "T@0:20", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div><span></span></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div><span><div style='height:20px;break-after:page'>T</div></span></div></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page;background:red'><span><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='height:40px'>A</div><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            3,
            &["A@0:0", "T@0:40", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div><span><div style='height:20px;break-after:page'>T</div></span></div><div style='break-after:page'><span><div style='height:20px;break-after:page'>U</div></span></div>Z</div>",
            4,
            &["T@0:0", "U@1:0", "Z@3:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page;min-height:50px'><span><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><ul style='margin:0;padding:0;list-style:none'><li style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></li><li>Z</li></ul></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></div><div style='break-before:avoid'>Z</div></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></div><div style='margin-top:10px'>Z</div></div>",
            3,
            &["T@0:0", "Z@2:10"][..],
        ),
        (
            "<style>.n{page:n}</style><div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></div><div class=n>Z</div></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<style>.n{page:n}</style><div style='width:20px'><div><span><div style='height:20px;break-after:page'>T</div></span></div><div class=n>Z</div></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span>S<div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            3,
            &["S@0:0", "T@0:30", "Z@2:0"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}

#[test]
fn the_flow_after_the_closing_line_stays_with_it() {
    // Without another forced break the flow after the paragraph follows the
    // closing line on the page the break opened. An atomic inline keeps the
    // break inside itself, and a set height keeps the place of the line in the
    // block.
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><div><span><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            2,
            &["T@0:0", "Z@1:0"][..],
        ),
        (
            "<div style='width:20px'><div><span><div style='height:20px;break-after:page'>T</div></span></div><div style='margin-top:10px'>Z</div></div>",
            2,
            &["T@0:0", "Z@1:0"][..],
        ),
        (
            "<div style='width:20px'><div><span><div style='height:100px;break-after:page'>T</div></span></div>Z</div>",
            2,
            &["T@0:0", "Z@1:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span style='display:inline-block'><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            2,
            &["T@0:0", "Z@1:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page;height:50px'><span><div style='height:20px;break-after:page'>T</div></span></div>Z</div>",
            2,
            &["T@0:0", "Z@1:0"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}

#[test]
fn a_break_before_content_of_no_height_at_the_top_of_a_page_still_opens_one() {
    // An empty line or an empty box at the top of the page a forced break
    // opened is content before the box after it, whose own forced break opens
    // another page.
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><div><span><div style='height:20px;break-after:page'>T</div></span></div><div style='break-before:page'>Z</div></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div><span><div style='height:100px;break-after:page'>T</div></span></div><div style='break-before:page'>Z</div></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:20px;break-after:page'>T</div></span></div><div></div><div style='break-before:page'>Z</div></div>",
            4,
            &["T@0:0", "Z@3:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'>X</div><div></div><div style='break-before:page'>Z</div></div>",
            3,
            &["X@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div>X</div><div style='break-before:page'></div><div style='break-before:page'>Z</div></div>",
            3,
            &["X@0:0", "Z@2:0"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}

#[test]
fn a_block_that_fills_the_page_leaves_the_closing_line_to_the_next() {
    // A block that ends at or past the end of the page leaves the closing line
    // to start the page after it.
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:100px;break-after:page'>T</div></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><span><div style='height:130px;break-after:page'>T</div></span></div>Z</div>",
            4,
            &["T@0:0", "Z@3:0"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}

#[test]
fn boxes_inside_an_empty_box_share_the_break_that_opened_its_page() {
    // Empty boxes inside an empty box at the top of the page a forced break
    // opened share that break, and only the box after them breaks again.
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><div>X</div><div style='break-before:page'><div style='break-before:page'></div></div><div>Z</div></div>",
            2,
            &["X@0:0", "Z@1:0"][..],
        ),
        (
            "<div style='width:20px'><div>X</div><div style='break-before:page'><div style='break-before:page'></div></div><div style='break-before:page'>Z</div></div>",
            3,
            &["X@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div>X</div><div style='break-before:page'><div style='break-before:page'><div style='break-before:page'></div></div></div><div>Z</div></div>",
            2,
            &["X@0:0", "Z@1:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'>X</div><div><div style='break-before:page'></div></div><div>Z</div></div>",
            2,
            &["X@0:0", "Z@1:0"][..],
        ),
        (
            "<div style='width:20px'><div>X</div><div style='break-before:page'><div></div></div><div style='break-before:page'>Z</div></div>",
            3,
            &["X@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div>X</div><div style='break-before:page'></div><div style='break-before:page'>Z</div></div>",
            3,
            &["X@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div>X</div><div><div style='break-before:page'></div></div><div style='break-before:page'>Z</div></div>",
            3,
            &["X@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div>X</div><div style='break-before:page'><div style='break-before:page'></div></div><div style='break-before:page'>Z</div></div>",
            3,
            &["X@0:0", "Z@2:0"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}

#[test]
fn the_closing_line_is_found_through_any_inline_content() {
    // The closing line follows a block inside a box that generates none, or a
    // block of the paragraph that inline content follows, and starts a page
    // even at the end of the document.
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><div><span><div style='height:20px;break-after:page'>T</div></span></div></div>",
            2,
            &["T@0:0"][..],
        ),
        (
            "<div style='width:20px'><div style='padding-bottom:1px'><span><div style='height:20px;break-after:page'>T</div></span></div></div>",
            2,
            &["T@0:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><div style='display:contents'><span><div style='height:20px;break-after:page'>T</div></span></div></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><div style='display:contents'><span><div style='height:20px;break-after:page'>T</div></span></div></div><div>Z</div></div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page;padding-bottom:1px'><div style='display:contents'><span><div style='height:20px;break-after:page'>T</div></span></div></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><div style='height:20px;break-after:page'>T</div><span></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page;padding-bottom:1px'><div style='height:20px;break-after:page'>T</div><span></span></div>Z</div>",
            3,
            &["T@0:0", "Z@2:0"][..],
        ),
        (
            "<div style='width:20px'><div style='break-after:page'><div style='height:20px;break-after:page'>T</div><span>U</span></div>Z</div>",
            3,
            &["T@0:0", "U@1:0", "Z@2:0"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}
