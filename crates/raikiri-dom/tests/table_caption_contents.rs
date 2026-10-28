//! A table caption inside `display: contents` elements of the table: it is the
//! caption of the table, as if it were a child of the table.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px: each is the element's
//! left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on each page of a layout with pages of `page_size`, as
/// `(page index, id, rect)`.
fn boxes_on_pages(page_size: &str, body: &str) -> Vec<(usize, String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: {page_size}; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
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
    let mut found = Vec::new();
    for (index, page) in result.pages().enumerate() {
        let dom = page.dom();
        for fragment in page.fragments() {
            if let Some(id) = dom.attr(fragment.node(), "id") {
                let rect = fragment.rect();
                found.push((
                    index,
                    id.to_owned(),
                    [rect.x, rect.y, rect.width, rect.height],
                ));
            }
        }
    }
    found
}

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page of 800px by 600px.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    boxes_on_pages("800px 600px", body)
        .into_iter()
        .filter(|(page, ..)| *page == 0)
        .map(|(_, id, rect)| (id, rect))
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            assert!(
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            );
        }
    }
}

/// A caption that sits in `display: contents` children of the table goes above
/// or below the rows like one that is a child of the table.
#[test]
fn a_caption_in_display_contents_is_the_caption_of_the_table() {
    let cases: &[Case<'_>] = &[
        (
            "one wrapper",
            "<div style='display:table'><div style='display:contents'><div id=c style='display:table-caption'>1</div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[("c", 8.0, 8.0, 10.0, 10.0), ("k", 8.0, 18.0, 10.0, 10.0)],
        ),
        (
            "two wrappers",
            "<div style='display:table'><div style='display:contents'><div style='display:contents'><div id=c style='display:table-caption'>1</div></div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[("c", 8.0, 8.0, 10.0, 10.0), ("k", 8.0, 18.0, 10.0, 10.0)],
        ),
        (
            "below the rows",
            "<div style='display:table'><div style='display:contents'><div id=c style='display:table-caption;caption-side:bottom'>1</div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[("c", 8.0, 18.0, 10.0, 10.0), ("k", 8.0, 8.0, 10.0, 10.0)],
        ),
        (
            "after the rows",
            "<div style='display:table'><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div><div style='display:contents'><div id=c style='display:table-caption'>1</div></div></div>",
            &[("c", 8.0, 8.0, 10.0, 10.0), ("k", 8.0, 18.0, 10.0, 10.0)],
        ),
        (
            "rows in a wrapper too",
            "<div style='display:table'><div style='display:contents'><div id=c style='display:table-caption'>1</div></div><div style='display:contents'><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div></div>",
            &[("c", 8.0, 8.0, 10.0, 10.0), ("k", 8.0, 18.0, 10.0, 10.0)],
        ),
    ];
    check_boxes(cases);
}

/// A caption in a `display: contents` child sizes the table and is sized by it
/// like one that is a child of the table.
#[test]
fn a_caption_in_display_contents_sizes_the_table() {
    let cases: &[Case<'_>] = &[
        (
            "a caption wider than the rows",
            "<div style='display:table'><div style='display:contents'><div id=c style='display:table-caption'>wide caption</div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[("c", 8.0, 8.0, 70.0, 20.0), ("k", 8.0, 28.0, 70.0, 10.0)],
        ),
        (
            "a float",
            "<div style='display:table;float:right'><div style='display:contents'><div id=c style='display:table-caption'>1</div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[
                ("c", 782.0, 8.0, 10.0, 10.0),
                ("k", 782.0, 18.0, 10.0, 10.0),
            ],
        ),
        (
            "an inline-table",
            "x<div style='display:inline-table'><div style='display:contents'><div id=c style='display:table-caption'>1</div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[("c", 18.0, 8.0, 10.0, 10.0), ("k", 18.0, 18.0, 10.0, 10.0)],
        ),
        (
            "a caption with padding and margin",
            "<div style='display:table'><div style='display:contents'><div id=c style='display:table-caption;padding:3px;margin:2px'>1</div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[("c", 10.0, 10.0, 16.0, 16.0), ("k", 8.0, 28.0, 20.0, 10.0)],
        ),
        (
            "a caption with an inline child",
            "<div style='display:table'><div style='display:contents'><div id=c style='display:table-caption'>1<span>1</span></div></div><div style='display:table-row'><div id=k style='display:table-cell'>k</div></div></div>",
            &[("c", 8.0, 8.0, 20.0, 10.0), ("k", 8.0, 18.0, 20.0, 10.0)],
        ),
    ];
    check_boxes(cases);
}

/// A table that breaks over pages is laid out the same with its caption in a
/// `display: contents` child as with the caption a child of the table: the
/// header group is repeated on each page, except when the caption goes below
/// the rows, where the table keeps the geometry it has without the repeat.
#[test]
fn a_caption_in_display_contents_paginates_as_a_direct_caption_does() {
    let rows: String = (0..7)
        .map(|n| {
            format!(
                "<div style='display:table-row'><div id=k{n} style='display:table-cell;height:20px'>k</div></div>"
            )
        })
        .collect();
    let header = "<div style='display:table-header-group'><div style='display:table-row'><div id=h style='display:table-cell'>h</div></div></div>";
    for side in ["top", "bottom"] {
        for header in ["", header] {
            let direct = format!(
                "<div style='display:table'><div id=c style='display:table-caption;caption-side:{side}'>cap</div>{header}{rows}</div>"
            );
            let wrapped = format!(
                "<div style='display:table'><div style='display:contents'><div id=c style='display:table-caption;caption-side:{side}'>cap</div></div>{header}{rows}</div>"
            );
            assert_eq!(
                boxes_on_pages("200px 80px", &wrapped),
                boxes_on_pages("200px 80px", &direct),
                "caption-side: {side}, header: {}",
                !header.is_empty()
            );
        }
    }
}
