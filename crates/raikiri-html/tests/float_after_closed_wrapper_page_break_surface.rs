//! A forced break after the last box of a wrapper that only floats follow
//! sends the first float to the next page. When the block-end padding,
//! border and remaining height of the wrapper fit on the page that the
//! break leaves, the wrapper ends there. The next page then starts with the
//! float and the content after the wrapper side by side. Edges that do not
//! fit go to the next page with the float. The positions are the ones
//! Chromium prints for the same documents (`page.pdf` with the page size
//! taken from `@page`); the floats are 40px wide so that the glyphs beside
//! them stand apart.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each 20px Ahem glyph as `text@page:x,top`, sorted, on pages 100px
/// square with 30px lines.
fn glyphs(body: &str) -> Vec<String> {
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
    let mut glyphs = Vec::new();
    for (index, page) in result.pages().enumerate() {
        for run in page.text_runs() {
            // The 20px Ahem baseline sits 21px below the top of its 30px line.
            let top = (run.origin.1 - 21.0).round() as i32;
            let mut x = run.origin.0.round() as i32;
            for glyph in run.text.chars() {
                if glyph != ' ' {
                    glyphs.push(format!("{glyph}@{index}:{x},{top}"));
                }
                x += 20;
            }
        }
    }
    glyphs.sort();
    glyphs
}

fn expect(body: &str, glyphs_expected: &[&str]) {
    let mut expected: Vec<String> = glyphs_expected.iter().map(|g| g.to_string()).collect();
    expected.sort();
    assert_eq!(glyphs(body), expected, "{body}");
}

/// A wrapper with the style `style` whose last box forces a break, then the
/// float `float`.
fn wrapper(style: &str, float: &str) -> String {
    format!("<div style='{style}'><div style='break-after:page'>T</div>{float}</div>")
}

const F: &str = "<div style='float:left;width:40px'>F</div>";

#[test]
fn the_content_after_the_wrapper_starts_the_page_beside_the_float() {
    // The wrapper's padding, border, set height and minimum height end on
    // the page that the break leaves. A height that leaves the content after
    // the wrapper clear of the float before the break still lines it up
    // beside the float.
    for style in [
        "padding-bottom:10px",
        "border-bottom:5px solid",
        "height:50px",
        "height:90px",
        "min-height:70px",
        "padding-top:5px;padding-bottom:10px",
    ] {
        for after in ["Z", "<div>Z</div>", "<div><div>Z</div></div>"] {
            let top = if style.starts_with("padding-top") {
                5
            } else {
                0
            };
            expect(
                &format!("{}{after}", wrapper(style, F)),
                &[&format!("T@0:0,{top}"), "F@1:0,0", "Z@1:40,0"],
            );
        }
    }
}

#[test]
fn the_margins_after_the_wrapper_follow_the_break() {
    // The margin after the wrapper is cut at the break; the margin of the
    // box after the wrapper is kept below the top of the page.
    expect(
        &format!(
            "{}<div>Z</div>",
            wrapper("padding-bottom:10px;margin-bottom:20px", F)
        ),
        &["T@0:0,0", "F@1:0,0", "Z@1:40,0"],
    );
    expect(
        &format!(
            "{}<div style='margin-top:15px'>Z</div>",
            wrapper("padding-bottom:10px", F)
        ),
        &["T@0:0,0", "F@1:0,0", "Z@1:40,15"],
    );
    expect(
        &format!(
            "{}<div style='margin-top:50px'>Z</div><div>Y</div>",
            wrapper("padding-bottom:30px", F)
        ),
        &["T@0:0,0", "F@1:0,0", "Z@1:0,50", "Y@2:0,0"],
    );
}

#[test]
fn nested_wrappers_all_end_before_the_break() {
    expect(
        &format!(
            "<div style='padding-bottom:10px'>{}</div><div>Z</div>",
            wrapper("padding-bottom:7px", F)
        ),
        &["T@0:0,0", "F@1:0,0", "Z@1:40,0"],
    );
    // A list item inside a list with padding below it.
    expect(
        "<ul style='margin:0;padding:0 0 30px 0;list-style:none'><li>\
         <div style='break-after:page'>T</div>\
         <div style='float:left;width:40px'>F</div></li></ul><div>Z</div>",
        &["T@0:0,0", "F@1:0,0", "Z@1:40,0"],
    );
}

#[test]
fn the_floats_and_lines_after_the_wrapper_line_up_beside_the_float() {
    let style = "padding-bottom:30px";
    // A second float lines up beside the first.
    expect(
        &format!(
            "{}<div>Z</div>",
            wrapper(
                style,
                &format!("{F}<div style='float:left;width:20px'>G</div>")
            )
        ),
        &["T@0:0,0", "F@1:0,0", "G@1:40,0", "Z@1:60,0"],
    );
    // A right float, a float with a margin, a tall float.
    expect(
        &format!(
            "{}<div>Z</div>",
            wrapper(style, "<div style='float:right;width:40px'>F</div>")
        ),
        &["T@0:0,0", "F@1:60,0", "Z@1:0,0"],
    );
    expect(
        &format!(
            "{}<div>Z</div>",
            wrapper(
                style,
                "<div style='float:left;width:40px;margin-top:5px'>F</div>"
            )
        ),
        &["T@0:0,0", "F@1:0,5", "Z@1:40,0"],
    );
    expect(
        &format!(
            "{}<div>Z</div>",
            wrapper(
                style,
                "<div style='float:left;width:40px;height:150px'>F</div>"
            )
        ),
        &["T@0:0,0", "F@1:0,0", "Z@1:40,0"],
    );
    // A second line of text after the wrapper.
    expect(
        &format!("{}<div>Z Y</div>", wrapper("padding-bottom:40px", F)),
        &["T@0:0,0", "F@1:0,0", "Z@1:40,0", "Y@1:80,0"],
    );
    // A box that clears the float, and one that holds its own floats.
    expect(
        &format!("{}<div style='clear:left'>Z</div>", wrapper(style, F)),
        &["T@0:0,0", "F@1:0,0", "Z@1:0,30"],
    );
    expect(
        &format!(
            "{}<div style='display:flow-root'>Z</div>",
            wrapper(style, F)
        ),
        &["T@0:0,0", "F@1:0,0", "Z@1:40,0"],
    );
}

#[test]
fn a_break_before_the_content_after_the_wrapper_is_the_same_break() {
    expect(
        &format!(
            "{}<div style='break-before:page'>Z</div>",
            wrapper("padding-bottom:30px", F)
        ),
        &["T@0:0,0", "F@1:0,0", "Z@1:40,0"],
    );
}

#[test]
fn edges_that_do_not_fit_go_to_the_next_page_with_the_float() {
    // The padding runs past the page the break leaves: it ends 80px down
    // the next page, and Z, which no longer fits there, starts the page
    // after.
    expect(
        &format!("{}<div>Z</div>", wrapper("padding-bottom:80px", F)),
        &["T@0:0,0", "F@1:0,0", "Z@2:0,0"],
    );
    // Lower down the page, the same padding fits or does not.
    let spacer = "<div style='height:60px'></div>";
    expect(
        &format!("{spacer}{}<div>Z</div>", wrapper("padding-bottom:5px", F)),
        &["T@0:0,60", "F@1:0,0", "Z@1:40,0"],
    );
    expect(
        &format!("{spacer}{}<div>Z</div>", wrapper("padding-bottom:15px", F)),
        &["T@0:0,60", "F@1:0,0", "Z@1:40,15"],
    );
}

#[test]
fn each_wrapper_ends_before_its_own_break() {
    expect(
        &format!(
            "{}{}<div>Z</div>",
            wrapper("padding-bottom:30px", F),
            wrapper(
                "padding-bottom:30px",
                "<div style='float:left;width:40px'>G</div>"
            )
            .replace(">T<", ">S<")
        ),
        &["T@0:0,0", "F@1:0,0", "S@1:40,0", "G@2:0,0", "Z@2:40,0"],
    );
}
