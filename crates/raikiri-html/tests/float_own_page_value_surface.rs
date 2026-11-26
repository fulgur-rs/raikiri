//! A float's own `page` value is no part of the page flow: Chromium places
//! the float, and the pages it reaches, as if the value were absent. A float
//! right after a forced break still goes on to the new page, and a page that
//! only the float reaches takes the page type of the flow around it.
//!
//! The widths and positions are the ones Chromium prints for the same
//! documents (`page.pdf` with the page sizes taken from `@page`): `foo`
//! pages are 200px wide, `bar` pages 300px and other pages 100px, all 100px
//! high. Each page lists the 20px Ahem glyphs on it as `glyph@x,top`, with
//! 30px lines. The floats are 40px wide so that the glyphs beside them stand
//! apart.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page as its width, then each 20px Ahem glyph on it as
/// `glyph@x,top`, sorted.
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
         @page foo{{size:200px 100px}} @page bar{{size:300px 100px}}\
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

const T_FOO: &str = "<div style='page:foo;break-after:page'>T</div>";

#[test]
fn a_float_with_its_own_page_value_goes_on_after_a_forced_break() {
    // The float's `page:bar` changes nothing: it starts the page the break
    // opened, and the content after it lines up beside it there.
    for (before, after, expected) in [
        (T_FOO, "Z", &["200: T@0,0", "100: F@0,0 Z@40,0"][..]),
        (
            "<div style='break-after:page'>T</div>",
            "Z",
            &["100: T@0,0", "100: F@0,0 Z@40,0"][..],
        ),
    ] {
        expect(
            &format!("{before}<div style='float:left;width:40px;page:bar'>F</div>{after}"),
            expected,
        );
    }
    expect(
        "<div><div style='break-after:page'>T</div>\
         <div style='float:left;width:40px;page:bar'>F</div>Z</div>",
        &["100: T@0,0", "100: F@0,0 Z@40,0"],
    );
}

#[test]
fn a_page_only_the_float_reaches_takes_the_type_of_the_flow_around_it() {
    // The content after the float names the page the float opened; the
    // page after it holds only the rest of the float.
    for float in [
        "<div style='float:left;width:40px;height:150px;page:bar'>F</div>",
        "<div style='float:left;width:40px;height:150px'>F</div>",
    ] {
        expect(
            &format!("{T_FOO}{float}<div style='page:foo'>Z</div>"),
            &["200: T@0,0", "200: F@0,0 Z@40,0", "100: "],
        );
        expect(
            &format!("<div>{T_FOO}{float}<div style='page:foo'>Z</div></div>"),
            &["200: T@0,0", "200: F@0,0 Z@40,0", "100: "],
        );
        // With nothing after the float, the page it opened is its own too.
        expect(
            &format!("{T_FOO}{float}"),
            &["200: T@0,0", "100: F@0,0", "100: "],
        );
    }
}
