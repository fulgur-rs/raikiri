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
    // isolates and overrides. A bdi without `dir` takes the direction of
    // its first strong character, here left to right.
    for (body, cd) in [
        ("<div dir=rtl><bdi>CD </bdi>AB</div>", 170.0),
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

#[test]
fn block_elements_made_inline_isolate_and_an_invalid_dir_does_not() {
    // The isolate list of HTML "Bidirectional text" also names the block
    // elements, which matters once an author makes one inline.
    for body in [
        "<div dir=rtl><p style='display:inline'>CD </p>AB</div>",
        "<div dir=rtl><li style='display:inline'>CD </li>AB</div>",
    ] {
        assert_eq!(
            runs(body),
            [("CD".to_owned(), 180.0), ("AB".to_owned(), 150.0)],
            "{body}"
        );
    }
    // Only the `ltr`, `rtl` and `auto` keywords isolate.
    assert_eq!(
        runs("<div dir=rtl><span dir=foo>CD </span>AB</div>"),
        [("AB".to_owned(), 180.0), ("CD".to_owned(), 150.0)]
    );
}

#[test]
fn bdo_overrides_the_order_of_its_content() {
    assert_eq!(
        runs("<div><bdo dir=rtl><span>A</span><span>B</span></bdo></div>"),
        [("B".to_owned(), 0.0), ("A".to_owned(), 10.0)]
    );
}

#[test]
fn bdi_takes_the_direction_of_its_first_strong_character() {
    // A Hebrew letter makes the bdi right to left, so `CD EF` is one
    // left-to-right run at the left of it. Without the bdi the letter
    // comes first.
    let bdi = runs("<div><bdi>\u{5d0}<span>CD</span> <span>EF</span></bdi></div>");
    let at =
        |runs: &[(String, f32)], text: &str| runs.iter().find(|(t, _)| t == text).map(|(_, x)| *x);
    assert_eq!((at(&bdi, "CD"), at(&bdi, "EF")), (Some(0.0), Some(30.0)));
    let span = runs("<div><span>\u{5d0}<span>CD</span> <span>EF</span></span></div>");
    assert!(at(&span, "CD").unwrap() > 0.0, "{span:?}");
}

#[test]
fn pre_with_dir_auto_sets_the_direction_of_each_line() {
    // `pre[dir=auto]` is `unicode-bidi: plaintext`: the second line starts
    // with a Hebrew letter, so it is right to left and aligned right.
    let auto =
        runs("<pre dir=auto style='margin:0;font:inherit'>AB\n\u{5d0} <span>CD</span></pre>");
    let cd = auto.iter().find(|(t, _)| t == "CD").unwrap().1;
    assert!(cd > 100.0, "{auto:?}");
    let ltr = runs("<pre style='margin:0;font:inherit'>AB\n\u{5d0} <span>CD</span></pre>");
    let cd = ltr.iter().find(|(t, _)| t == "CD").unwrap().1;
    assert!(cd < 100.0, "{ltr:?}");
}
