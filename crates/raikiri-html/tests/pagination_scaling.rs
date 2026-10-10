//! Page counts and projection work for long runs of short paragraphs.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lay out `count` four-line paragraphs on 300×200 pages with 40px margins,
/// optionally numbering each one.
fn page_count(count: usize, numbered: bool, css: &str) -> u32 {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let body: String = (0..count)
        .map(|index| {
            let label = if numbered { format!("P{index} ") } else { String::new() };
            format!(
                "<p>{label}Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor</p>"
            )
        })
        .collect();
    let html = format!(
        "<!doctype html><style>@page{{size:300px 200px;margin:40px}}body{{font:10px/10px Ahem}}{css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(document) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    document.page_count()
}

// Each paragraph takes 50px of a 120px page area, so 60 of them fill about 25
// pages. Moving a paragraph to the next page for orphans and widows must
// measure the move from where earlier moves left it; measuring it from its
// unshifted position pushes it pages too far and the count grows with every
// move.
#[test]
fn orphans_and_widows_moves_keep_the_page_count_linear() {
    let pages = page_count(60, false, "");
    assert!((25..=28).contains(&pages), "{pages} pages");
}

// Generated boxes are projected per paragraph onto the pages its lines can
// reach. Visiting every page for every paragraph is quadratic and exhausts
// the layout work budget long before the page count is unusual.
#[test]
fn generated_boxes_project_onto_long_documents() {
    let pages = page_count(300, true, "p::before{content:\"> \"}");
    assert!(pages >= 120, "{pages} pages");
}
