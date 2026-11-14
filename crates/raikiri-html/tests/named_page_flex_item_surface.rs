//! A row flex container whose items hold a named box below them prints its
//! line on the page that box names, as Chromium does. Every row comes from
//! Chromium's print output for the same document, with 200px pages, `@page
//! a` 240px wide, `@page b` 260px wide and `@page c` 280px wide, which tells
//! the page names apart.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page's name and the glyphs printed on it.
fn pages(body: &str) -> Vec<(Option<String>, String)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:10px}}@page a{{size:240px 200px}}@page b{{size:260px 200px}}@page c{{size:280px 200px}}html,body{{margin:0}}body{{font:10px/10px Ahem}}</style><body>{body}</body>"
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
    (0..result.page_count())
        .map(|index| {
            let page = result.page(index).unwrap();
            let text: String = page
                .text_runs()
                .iter()
                .flat_map(|run| run.text.chars())
                .filter(|glyph| !glyph.is_whitespace())
                .collect();
            (page.name().map(str::to_owned), text)
        })
        .collect()
}

type Row = (
    &'static str,
    &'static [(Option<&'static str>, &'static str)],
);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(name, text)| (name.map(str::to_owned), text.to_owned()))
            .collect();
        assert_eq!(pages(body), expected, "{body}");
    }
}

#[test]
fn a_row_flex_line_takes_the_page_a_box_nested_in_an_item_names() {
    // One page for the line, named by the first item that names one, then a
    // new page for what follows, with no blank page before the line.
    assert_rows(&[
        (
            "<div style='display:flex'><div><div style='page:a'>F</div></div></div><div>Z</div>",
            &[(Some("a"), "F"), (None, "Z")],
        ),
        (
            "<div style='display:flex'><div>E</div><div><div style='page:a'>F</div></div></div><div>Z</div>",
            &[(Some("a"), "EF"), (None, "Z")],
        ),
        (
            "<div style='display:flex'><div><div style='page:a'>E</div></div><div><div style='page:b'>F</div></div></div><div>Z</div>",
            &[(Some("a"), "EF"), (None, "Z")],
        ),
        (
            "<div>Y</div><div style='display:flex'><div><div style='page:a'>F</div></div></div><div>Z</div>",
            &[(None, "Y"), (Some("a"), "F"), (None, "Z")],
        ),
        (
            "<div style='display:flex;flex-direction:column'><div style='display:flex'><div><div style='page:a'>F</div></div></div></div><div>Z</div>",
            &[(Some("a"), "F"), (None, "Z")],
        ),
    ]);
}

#[test]
fn a_page_the_line_already_has_wins_over_a_nested_name() {
    // A page the container names itself or inherits names the line, and a
    // row nested in an item of another row stays on that row's page.
    assert_rows(&[
        (
            "<div style='page:c'><div style='display:flex'><div>E</div><div><div style='page:a'>F</div></div></div></div><div>Z</div>",
            &[(Some("c"), "EF"), (None, "Z")],
        ),
        (
            "<div style='display:flex;flex-direction:column'><div style='display:flex;page:c'><div>E</div><div><div style='page:a'>F</div></div></div></div><div>Z</div>",
            &[(Some("c"), "EF"), (None, "Z")],
        ),
        (
            "<div style='display:flex'><div><div style='page:a'>E</div></div><div><div style='display:flex'><div><div style='page:b'>F</div></div></div></div></div><div>Z</div>",
            &[(Some("a"), "EF"), (None, "Z")],
        ),
    ]);
}

#[test]
fn an_inline_flex_container_names_no_page() {
    // An inline-level box takes the page of its containing block, so the
    // names inside it neither name a page nor start one after it.
    assert_rows(&[(
        "<div style='display:inline-flex'><div><div style='page:a'>F</div></div></div><div>Z</div>",
        &[(None, "FZ")],
    )]);
}
