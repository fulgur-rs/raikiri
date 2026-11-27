//! The columns of a multicol container with a `column-span: all` box that
//! the page walk moves, as a forced break before it does, are filled again
//! from where the walk puts the container: the content after the spanner
//! keeps its reading order across the pages it runs on (CSS Multi-column 1
//! sections 6 and 8; CSS Fragmentation 3 section 2).
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints 200px by 200px pages with no margin: two 100px columns, and lines
//! of 20px Ahem, one word each.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

const PAGE: &str = "<style>@page { size: 200px 200px; margin: 0 } body { margin: 0 } \
    .mc { columns: 2; column-gap: 0 } p { margin: 0 } \
    h2 { column-span: all; margin: 0; font-size: 20px }</style>";

/// The words `{prefix}01` to `{prefix}{count}`, each in a span with the word
/// as its id.
fn words(prefix: &str, count: usize) -> String {
    (1..=count)
        .map(|index| format!("<span id={prefix}{index:02}>{prefix}{index:02}</span>"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The page, from 0, and the left and top on it of the first piece of the
/// element with the id `id`.
fn place(pages: &[Vec<(String, f32, f32)>], id: &str) -> Option<(usize, f32, f32)> {
    pages.iter().enumerate().find_map(|(index, page)| {
        page.iter()
            .find(|(name, ..)| name == id)
            .map(|&(_, x, y)| (index, x, y))
    })
}

/// Lays out `body` and compares the places of the listed elements with the
/// expected ones.
fn check(label: &str, body: &str, expected: &[(&str, usize, f32, f32)]) {
    let html = format!("<!DOCTYPE html><style>body {{ font: 20px/20px Ahem }}</style>{PAGE}{body}");
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let pages: Vec<Vec<(String, f32, f32)>> = result
        .pages()
        .map(|page| {
            let dom = page.dom();
            page.fragments()
                .filter_map(|fragment| {
                    let id = dom.attr(fragment.node(), "id")?;
                    let rect = fragment.rect();
                    Some((id.to_owned(), rect.x, rect.y))
                })
                .collect()
        })
        .collect();
    for &(id, page, x, y) in expected {
        assert_eq!(
            place(&pages, id),
            Some((page, x, y)),
            "{label}: place of #{id}"
        );
    }
}

/// The places Chromium gives `A01` to `A12`, a spanner, then `B01` to
/// `B30`, in a container that starts at the top of the second page.
const AFTER_A_BREAK: &[(&str, usize, f32, f32)] = &[
    ("A01", 1, 0.0, 0.0),
    ("A07", 1, 100.0, 0.0),
    ("SPAN", 1, 0.0, 120.0),
    ("B01", 1, 0.0, 140.0),
    ("B03", 1, 0.0, 180.0),
    ("B04", 1, 100.0, 140.0),
    ("B06", 1, 100.0, 180.0),
    ("B07", 2, 0.0, 0.0),
    ("B16", 2, 0.0, 180.0),
    ("B17", 2, 100.0, 0.0),
    ("B26", 2, 100.0, 180.0),
    ("B27", 3, 0.0, 0.0),
    ("B28", 3, 0.0, 20.0),
    ("B29", 3, 100.0, 0.0),
    ("B30", 3, 100.0, 20.0),
];

#[test]
fn a_forced_break_before_the_container_refills_the_columns_after_its_spanner() {
    let content = format!(
        "<p>{}</p><h2 id=SPAN>SPAN</h2><p>{}</p>",
        words("A", 12),
        words("B", 30)
    );
    check(
        "break before the container",
        &format!("<p>PRE</p><div class=mc style='break-before: page'>{content}</div>"),
        AFTER_A_BREAK,
    );
    check(
        "break before a wrapper",
        &format!("<p>PRE</p><div style='break-before: page'><div class=mc>{content}</div></div>"),
        AFTER_A_BREAK,
    );
}

#[test]
fn moved_columns_of_blocks_with_no_paragraph_refill_after_their_spanner() {
    let blocks = |prefix: &str, count: usize| {
        (1..=count)
            .map(|index| format!("<div id={prefix}{index:02}>{prefix}{index:02}</div>"))
            .collect::<String>()
    };
    check(
        "blocks",
        &format!(
            "<p>PRE</p><div class=mc style='break-before: page'>{}\
             <h2 id=SPAN>SPAN</h2>{}</div>",
            blocks("A", 12),
            blocks("B", 30)
        ),
        AFTER_A_BREAK,
    );
}

#[test]
fn moved_columns_that_fill_in_turn_refill_after_their_spanner() {
    check(
        "column-fill: auto",
        &format!(
            "<p>PRE</p><div class=mc style='break-before: page; column-fill: auto'>\
             <p>{}</p><h2 id=SPAN>SPAN</h2><p>{}</p></div>",
            words("A", 12),
            words("B", 30)
        ),
        &[
            ("A01", 1, 0.0, 0.0),
            ("A07", 1, 100.0, 0.0),
            ("SPAN", 1, 0.0, 120.0),
            ("B03", 1, 0.0, 180.0),
            ("B06", 1, 100.0, 180.0),
            ("B07", 2, 0.0, 0.0),
            ("B16", 2, 0.0, 180.0),
            ("B17", 2, 100.0, 0.0),
            ("B26", 2, 100.0, 180.0),
            // The last columns fill in turn rather than balance.
            ("B27", 3, 0.0, 0.0),
            ("B30", 3, 0.0, 60.0),
        ],
    );
}

#[test]
fn columns_moved_by_a_box_before_them_and_two_spanners_refill() {
    check(
        "pushed by a box that avoids breaks",
        &format!(
            "<div style='height: 100px'>PRE</div>\
             <div id=Q style='height: 150px; break-inside: avoid'>Q</div>\
             <div class=mc><p>{}</p><h2 id=SPAN>SPAN</h2><p>{}</p></div>",
            words("A", 6),
            words("B", 30)
        ),
        &[
            ("Q", 1, 0.0, 0.0),
            ("A02", 1, 0.0, 170.0),
            ("A03", 1, 100.0, 150.0),
            ("A04", 1, 100.0, 170.0),
            ("A05", 2, 0.0, 0.0),
            ("SPAN", 2, 0.0, 40.0),
            ("B07", 2, 0.0, 180.0),
            ("B08", 2, 100.0, 60.0),
            ("B14", 2, 100.0, 180.0),
            ("B15", 3, 0.0, 0.0),
            ("B22", 3, 0.0, 140.0),
            ("B23", 3, 100.0, 0.0),
            ("B30", 3, 100.0, 140.0),
        ],
    );
    check(
        "two spanners",
        &format!(
            "<p>PRE</p><div class=mc style='break-before: page'><p>{}</p>\
             <h2 id=SPAN>SPAN</h2><p>{}</p><h2 id=SPAN2>SPAN2</h2><p>{}</p></div>",
            words("A", 6),
            words("B", 14),
            words("C", 30)
        ),
        &[
            ("A01", 1, 0.0, 0.0),
            ("A04", 1, 100.0, 0.0),
            ("SPAN", 1, 0.0, 60.0),
            ("B06", 1, 0.0, 180.0),
            ("B12", 1, 100.0, 180.0),
            ("B13", 2, 0.0, 0.0),
            ("SPAN2", 2, 0.0, 40.0),
            ("C07", 2, 0.0, 180.0),
            ("C08", 2, 100.0, 60.0),
            ("C14", 2, 100.0, 180.0),
            ("C15", 3, 0.0, 0.0),
            ("C22", 3, 0.0, 140.0),
            ("C23", 3, 100.0, 0.0),
            ("C30", 3, 100.0, 140.0),
        ],
    );
}
