//! A float that takes a forced break opens the next page, and the in-flow
//! content after it starts that page beside it. Chromium takes a change of
//! page type there as the same break: the page takes the type of that
//! content instead of opening another page. When a box around the float goes
//! on onto the page, the page takes that box's type, which the float has.
//! Pages after it that only the float reaches take the float's type.
//!
//! The widths and positions are the ones Chromium prints for the same
//! documents (`page.pdf` with the page sizes taken from `@page`): `foo` pages
//! are 120px wide, `bar` pages 140px and other pages 100px. The floats are
//! 40px wide so that the glyphs beside them stand apart.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page as its width, then each 20px Ahem glyph on it as `text@x,top`,
/// sorted, with 30px lines.
fn pages(body: &str) -> Vec<String> {
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
         @page foo{{size:120px 100px}} @page bar{{size:140px 100px}}\
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
    result
        .pages()
        .map(|page| {
            let mut glyphs = Vec::new();
            for run in page.text_runs() {
                // The 20px Ahem baseline sits 21px below the top of its 30px line.
                let top = (run.origin.1 - 21.0).round() as i32;
                let mut x = run.origin.0.round() as i32;
                for glyph in run.text.chars() {
                    if glyph != ' ' {
                        glyphs.push(format!("{glyph}@{x},{top}"));
                    }
                    x += 20;
                }
            }
            glyphs.sort();
            let width = page.geometry().page_box.width.round() as i32;
            format!("{width}: {}", glyphs.join(" "))
        })
        .collect()
}

fn expect(body: &str, expected: &[&str]) {
    assert_eq!(pages(body), expected, "{body}");
}

const F: &str = "<div style='float:left;width:40px'>F</div>";
const T_FOO: &str = "<div style='page:foo;break-after:page'>T</div>";

#[test]
fn a_named_box_after_the_float_names_the_page_the_float_opened() {
    for after in [
        "<div style='page:bar'>Z</div>",
        "<div><div style='page:bar'>Z</div></div>",
    ] {
        expect(
            &format!("{T_FOO}{F}{after}"),
            &["120: T@0,0", "140: F@0,0 Z@40,0"],
        );
    }
    expect(
        &format!("<div style='break-after:page'>T</div>{F}<div style='page:bar'>Z</div>"),
        &["100: T@0,0", "140: F@0,0 Z@40,0"],
    );
    expect(
        &format!("{T_FOO}{F}<div style='page:bar'>Z</div><div>Y</div>"),
        &["120: T@0,0", "140: F@0,0 Z@40,0", "100: Y@0,0"],
    );
    expect(
        &format!("{T_FOO}{F}<div style='page:foo'>Z</div><div style='page:bar'>Y</div>"),
        &["120: T@0,0", "120: F@0,0 Z@40,0", "140: Y@0,0"],
    );
    expect(
        &format!("{T_FOO}{F}<div style='page:foo'>Z</div>"),
        &["120: T@0,0", "120: F@0,0 Z@40,0"],
    );
}

#[test]
fn the_float_own_break_before_is_the_same_break() {
    expect(
        &format!(
            "{T_FOO}<div style='float:left;width:40px;break-before:page'>F</div>\
             <div style='page:bar'>Z</div>"
        ),
        &["120: T@0,0", "140: F@0,0 Z@40,0"],
    );
}

#[test]
fn a_float_on_a_line_keeps_a_named_box_after_it_off_its_page() {
    // The float stands on a line of the span, which takes the page the
    // float opened, and the box after the span opens a page of its own.
    // Text on that line still starts the page beside the float.
    let float_in_span = format!("<span>{F}</span>");
    expect(
        &format!("{T_FOO}{float_in_span}<div style='page:bar'>Z</div>"),
        &["120: T@0,0", "100: F@0,0", "140: Z@0,0"],
    );
    expect(
        &format!("{T_FOO}{float_in_span}Z"),
        &["120: T@0,0", "100: F@0,0 Z@40,0"],
    );
    expect(
        &format!(
            "<div style='page:foo'><span><div style='break-after:page'>T</div>{F}</span></div>\
             <div style='page:bar'>Z</div>"
        ),
        &["120: T@0,0", "120: F@0,0", "140: Z@0,0"],
    );
}

#[test]
fn unnamed_content_after_the_float_names_the_page_auto() {
    expect(
        &format!("{T_FOO}{F}Z"),
        &["120: T@0,0", "100: F@0,0 Z@40,0"],
    );
    expect(
        &format!("{T_FOO}{F}<div style='float:left;width:20px'>G</div>Z"),
        &["120: T@0,0", "100: F@0,0 G@40,0 Z@60,0"],
    );
    expect(
        &format!("{T_FOO}{F}<div style='position:absolute'>A</div><div>Z</div>"),
        &["120: T@0,0", "100: A@0,0 F@0,0 Z@40,0"],
    );
}

#[test]
fn an_empty_box_after_the_float_opens_the_named_page_on_its_own() {
    expect(
        &format!("{T_FOO}{F}<div style='height:0'></div><div style='page:bar'>Z</div>"),
        &["120: T@0,0", "100: F@0,0", "140: Z@0,0"],
    );
}

#[test]
fn a_box_around_the_float_that_goes_on_names_the_page() {
    let inner = format!("<div style='break-after:page'>T</div>{F}");
    for after in ["Z", "<div>Z</div>", "<div style='page:bar'>Z</div>"] {
        expect(
            &format!("<div style='page:foo'>{inner}</div>{after}"),
            &["120: T@0,0", "120: F@0,0 Z@40,0"],
        );
    }
    expect(
        &format!("<div style='page:foo'><div>{inner}</div></div><div style='page:bar'>Z</div>"),
        &["120: T@0,0", "120: F@0,0 Z@40,0"],
    );
    expect(
        "<div style='page:foo'><div style='break-after:page'>T</div>\
         <div style='float:left;width:40px;height:50px'>F</div></div>Z",
        &["120: T@0,0", "120: F@0,0 Z@40,0"],
    );
    // The box that forced the break has its own type; the box around the
    // float still names the page the float opened.
    expect(
        &format!("<div style='page:foo'><div style='page:bar;break-after:page'>T</div>{F}</div>Z"),
        &["140: T@0,0", "120: F@0,0 Z@40,0"],
    );
    // The pages after it take the types of their own content.
    let kept =
        format!("<div style='page:foo'><div style='page:bar;break-after:page'>T</div>{F}</div>Z");
    expect(
        &format!("{kept}<div style='break-before:page'>Y</div>"),
        &["140: T@0,0", "120: F@0,0 Z@40,0", "100: Y@0,0"],
    );
    expect(
        &format!("<div style='page:foo'>{inner}</div>Z<div style='page:bar'>Y</div>"),
        &["120: T@0,0", "120: F@0,0 Z@40,0", "140: Y@0,0"],
    );
}

#[test]
fn a_float_inside_the_named_box_after_it_takes_that_type() {
    expect(
        &format!("<div style='break-after:page'>T</div><div style='page:foo'>{F}Z</div>"),
        &["100: T@0,0", "120: F@0,0 Z@40,0"],
    );
}

#[test]
fn a_page_only_the_float_reaches_takes_the_float_type() {
    expect(
        &format!(
            "{T_FOO}<div style='float:left;width:40px;height:150px'>F</div>\
             <div style='page:bar'>Z</div>"
        ),
        &["120: T@0,0", "140: F@0,0 Z@40,0", "100: "],
    );
}

#[test]
fn a_float_without_a_forced_break_stays_on_its_page() {
    expect(
        &format!("<div style='page:foo'>T</div>{F}<div>Z</div>"),
        &["120: F@0,30 T@0,0", "100: Z@0,0"],
    );
    expect(
        &format!("<div>T</div>{F}<div style='page:bar'>Z</div>"),
        &["100: F@0,30 T@0,0", "140: Z@0,0"],
    );
}
