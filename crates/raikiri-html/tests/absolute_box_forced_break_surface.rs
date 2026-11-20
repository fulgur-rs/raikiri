//! Break properties apply to boxes in the flow. An absolutely or fixed
//! positioned box breaks no page before or after itself, and a break forced
//! inside one never reaches the flow outside it: the boxes after it stay on
//! the page. The page counts and line positions are the ones Chromium prints
//! for the same documents (`page.pdf` with the page size taken from `@page`).

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

/// A 20px wide container whose first 40px are taken, then `inner` and a `Z`
/// line.
fn after_spacer(inner: &str) -> String {
    format!("<div style='width:20px'><div style='height:40px'></div>{inner}Z</div>")
}

#[test]
fn a_break_forced_inside_an_absolute_box_does_not_move_the_flow_after_it() {
    let tail = "<div style='height:20px;break-after:page'>T</div>";
    // Inside an inline box, inside a relative wrapper with padding, or
    // straight inside the container.
    for (inner, z) in [
        (
            format!(
                "<div style='padding-bottom:10px;position:relative'><span>\
                 <div style='position:absolute;width:20px'>{tail}</div></span></div>"
            ),
            "Z@0:50",
        ),
        (
            format!(
                "<div style='padding-bottom:10px;position:relative'>\
                 <div style='position:absolute;width:20px'>{tail}</div></div>"
            ),
            "Z@0:50",
        ),
        (
            format!("<div style='position:absolute;width:20px'>{tail}</div>"),
            "Z@0:40",
        ),
        (
            format!(
                "<div style='position:relative'><div style='position:absolute;width:20px'><div>{tail}</div></div></div>"
            ),
            "Z@0:40",
        ),
        (
            format!("<div><div style='position:absolute;width:20px'>{tail}</div></div>"),
            "Z@0:40",
        ),
        (
            format!(
                "<div style='position:relative'><span style='position:absolute;width:20px'>{tail}</span></div>"
            ),
            "Z@0:40",
        ),
    ] {
        expect(&after_spacer(&inner), 1, &["T@0:40", z]);
    }
}

#[test]
fn a_box_after_the_absolute_box_stays_on_the_page() {
    let tail = "<div style='height:20px;break-after:page'>T</div>";
    expect(
        &after_spacer(&format!(
            "<div style='position:relative'><div style='position:absolute;width:20px'>{tail}</div>\
             <div>Y</div></div>"
        )),
        1,
        &["T@0:40", "Y@0:40", "Z@0:70"],
    );
    // An absolute box with an inset.
    expect(
        &after_spacer(&format!(
            "<div style='position:relative'><div style='position:absolute;top:0;width:20px'>{tail}</div>\
             <div>Y</div></div>"
        )),
        1,
        &["T@0:40", "Y@0:40", "Z@0:70"],
    );
    // A fixed box.
    expect(
        &after_spacer(&format!(
            "<div style='position:fixed;top:0;left:50px;width:20px'>{tail}</div>"
        )),
        1,
        &["T@0:0", "Z@0:40"],
    );
    // Breaks after a box inside the absolute box and after the box around
    // it.
    expect(
        &after_spacer(
            "<div style='position:relative'><div style='position:absolute;width:20px'>\
             <div style='break-after:page'><div style='height:20px;break-after:page'>T</div></div>\
             </div></div>",
        ),
        1,
        &["T@0:40", "Z@0:40"],
    );
    // An absolute box inside another, with a break of its own.
    expect(
        &after_spacer(&format!(
            "<div style='position:relative'><div style='position:absolute;width:20px'>\
             <div style='position:absolute;width:20px'>{tail}</div>\
             <div style='height:20px;break-after:page'>V</div></div></div>"
        )),
        1,
        &["T@0:40", "V@0:40", "Z@0:40"],
    );
}

#[test]
fn an_absolute_box_breaks_no_page_before_or_after_itself() {
    expect(
        &after_spacer(
            "<div style='position:absolute;width:20px;break-after:page'>T</div><div></div>",
        ),
        1,
        &["T@0:40", "Z@0:40"],
    );
    expect(
        &after_spacer(
            "<div>Y</div><div style='position:absolute;width:20px;break-before:page'>T</div><div></div>",
        ),
        1,
        &["T@0:70", "Y@0:40", "Z@0:70"],
    );
    // A box in the flow after it still breaks the page.
    expect(
        &after_spacer(
            "<div style='position:relative'><div style='position:absolute;width:20px;\
             break-before:page;break-after:page'>T</div>\
             <div style='break-after:page'>Y</div></div>",
        ),
        2,
        &["T@0:40", "Y@0:40", "Z@1:0"],
    );
}

#[test]
fn the_break_after_a_box_around_the_absolute_box_still_falls_after_it() {
    expect(
        "<div style='width:20px'><div style='break-after:page;position:relative'><div>X</div>\
         <div style='position:absolute;width:20px'>\
         <div style='height:20px;break-after:page'>T</div></div></div><div>Z</div></div>",
        2,
        &["T@0:30", "X@0:0", "Z@1:0"],
    );
    // Relative and sticky boxes are in the flow: the break inside them does
    // move the line after them.
    for position in ["position:relative;top:5px", "position:sticky;top:0"] {
        let top = if position.contains("relative") {
            45
        } else {
            40
        };
        expect(
            &after_spacer(&format!(
                "<div style='{position}'><div style='height:20px;break-after:page'>T</div></div>"
            )),
            2,
            &[&format!("T@0:{top}"), "Z@1:0"],
        );
    }
}

#[test]
fn a_break_still_pending_when_the_flow_enters_an_absolute_box_falls_after_it() {
    // The break after a flex item waits for the end of its line, past an
    // absolute box with a break of its own in the next item.
    expect(
        "<div style='width:60px'><div style='display:flex'>\
         <div style='break-after:page;width:20px'>S</div><div style='width:20px'>\
         <div style='position:absolute;width:20px'><div style='break-after:page'>T</div></div>\
         </div></div><div>Z</div></div>",
        2,
        &["S@0:0", "T@0:0", "Z@1:0"],
    );
}

#[test]
fn a_positioned_element_that_generates_no_box_leaves_its_breaks_in_the_flow() {
    expect(
        "<div style='width:20px'><div style='display:contents;position:absolute'>\
         <div style='break-after:page'>T</div></div><div>Z</div></div>",
        2,
        &["T@0:0", "Z@1:0"],
    );
}
