//! Bidi isolation: an isolate is a neutral in the text around it, so its
//! content is not reordered together with neighbouring text.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each non-blank run on the first page as its text and left x.
fn runs(body: &str) -> Vec<(String, f32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 300px;margin:0}}\
         body{{margin:0;font:10px/20px Ahem}}</style><body>{body}</body>"
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
    let page = result.page(0).unwrap();
    page.text_runs()
        .into_iter()
        .filter(|run| !run.text.trim().is_empty())
        .map(|run| (run.text.trim().to_string(), run.origin.0))
        .collect()
}

// The expected x positions are Chromium's (print, Ahem 10px, page 200px
// wide, body margin 0). Run origins are the left edges of the glyphs.

#[test]
fn an_isolate_comes_before_the_text_after_it_in_a_right_to_left_line() {
    assert_eq!(
        runs("<div dir=rtl><span style='unicode-bidi:isolate'>CD </span>AB</div>"),
        [("CD".to_owned(), 180.0), ("AB".to_owned(), 150.0)]
    );
    // Without the isolate, `CD AB` is one left-to-right run.
    assert_eq!(
        runs("<div dir=rtl><span>CD </span>AB</div>"),
        [("AB".to_owned(), 180.0), ("CD".to_owned(), 150.0)]
    );
}

#[test]
fn an_inside_marker_starts_a_right_to_left_line() {
    // `::marker` is an isolate, so `1.` stays at the right edge.
    assert_eq!(
        runs("<ul dir=rtl style='list-style:decimal inside;margin:0;padding:0'><li>AB</li></ul>"),
        [
            ("1".to_owned(), 190.0),
            (".".to_owned(), 180.0),
            ("AB".to_owned(), 150.0)
        ]
    );
}

#[test]
fn html_elements_that_isolate_by_default() {
    // HTML "Bidirectional text": `bdi, output, [dir]` isolate and `bdo`
    // isolates and overrides.
    for (body, cd) in [
        ("<div dir=rtl><bdi>CD </bdi>AB</div>", 180.0),
        ("<div dir=rtl><output>CD </output>AB</div>", 180.0),
        ("<div dir=rtl><span dir=rtl>CD </span>AB</div>", 180.0),
        ("<div dir=rtl><span dir=ltr>CD </span>AB</div>", 170.0),
        ("<div dir=rtl><bdo dir=ltr>CD </bdo>AB</div>", 170.0),
    ] {
        assert_eq!(
            runs(body),
            [("CD".to_owned(), cd), ("AB".to_owned(), 150.0)],
            "{body}"
        );
    }
}
