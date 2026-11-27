//! The boxes of a table row that a page break splits go on over every page
//! the row runs over, as Chromium paints their backgrounds: the cells and
//! the row stretch to the row's new end, and in a table that repeats its
//! header and footer groups, the fragments of the row group, the row and its
//! cells start below each header copy, with the spacing below it, and end at
//! each footer copy.
//!
//! The positions are the ones Chromium prints for the same documents
//! (`page.pdf`, 100px square pages, 20px Ahem glyphs on 20px lines, measured
//! from the cell backgrounds at 288 dpi).

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}body{{margin:0}}\
         td{{padding:0}}</style>{body}"
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
}

/// The top and bottom of each fragment of the box with `id`, by page; an
/// empty list for a page the box does not reach.
fn extents(document: &DocumentLayout, id: &str) -> Vec<Vec<(f32, f32)>> {
    (0..document.page_count())
        .map(|index| {
            let page = document.page(index as u32).unwrap();
            let dom = page.dom();
            page.fragments()
                .filter(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (rect.y, rect.y + rect.height)
                })
                .collect()
        })
        .collect()
}

/// A table of 20px rows: an optional header `H`, the row `A`, the row `R`
/// whose cell `c` holds `content`, the row `C` and an optional footer `F`.
fn table(spacing: u32, header: bool, footer: bool, content: &str) -> String {
    let row = |id: &str, text: &str| format!("<tr id='{id}'><td>{text}</td></tr>");
    format!(
        "<table style='border-spacing:{spacing}px;font:20px/20px Ahem;width:20px'>{}\
         <tbody id='b'>{}<tr id='R'><td id='c'>{content}</td></tr>{}</tbody>{}</table>",
        if header {
            format!("<thead>{}</thead>", row("H", "H"))
        } else {
            String::new()
        },
        row("A", "A"),
        row("C", "C"),
        if footer {
            format!("<tfoot>{}</tfoot>", row("F", "F"))
        } else {
            String::new()
        },
    )
}

const LINES: &str = "X X X X X X X";
const FORCED: &str = "<div>X</div><div style='break-before:page'>X</div>";

#[test]
fn a_forced_break_in_a_cell_carries_the_cell_to_the_next_page() {
    let document = lay_out(&table(0, false, false, FORCED));
    let split = [vec![(20.0, 100.0)], vec![(0.0, 20.0)]];
    assert_eq!(extents(&document, "c"), split);
    assert_eq!(extents(&document, "R"), split);
    assert_eq!(extents(&document, "C"), [vec![], vec![(20.0, 40.0)]]);
}

#[test]
fn split_rows_go_on_between_the_header_and_footer_copies() {
    let document = lay_out(&table(0, true, true, LINES));
    let split = [vec![(40.0, 80.0)], vec![(20.0, 80.0)], vec![(20.0, 60.0)]];
    assert_eq!(extents(&document, "c"), split);
    assert_eq!(extents(&document, "R"), split);
    assert_eq!(
        extents(&document, "b"),
        [vec![(20.0, 80.0)], vec![(20.0, 80.0)], vec![(20.0, 80.0)]]
    );
    assert_eq!(
        extents(&lay_out(&table(0, true, true, FORCED)), "c"),
        [vec![(40.0, 80.0)], vec![(20.0, 40.0)]]
    );
    // Without a footer, the row runs to the end of the page.
    assert_eq!(
        extents(&lay_out(&table(0, true, false, LINES)), "c"),
        [vec![(40.0, 100.0)], vec![(20.0, 100.0)], vec![]]
    );
}

#[test]
fn split_rows_keep_the_spacing_below_the_header_copies() {
    // With 4px of spacing, the footer copies end 4px above the page end, and
    // a split row starts 4px below each header copy and ends at the footer
    // copy.
    let document = lay_out(&table(4, true, true, LINES));
    assert_eq!(
        extents(&document, "c"),
        [
            vec![],
            vec![(24.0, 76.0)],
            vec![(24.0, 76.0)],
            vec![(24.0, 76.0)],
            vec![(24.0, 44.0)],
        ]
    );
    assert_eq!(
        extents(&document, "F"),
        [
            vec![(52.0, 72.0)],
            vec![(76.0, 96.0)],
            vec![(76.0, 96.0)],
            vec![(76.0, 96.0)],
            vec![(72.0, 92.0)],
        ]
    );
}
