//! Pagination of a vertical grid.
//!
//! The rows of a grid run along the block axis. In a vertical document the
//! pages follow that axis, so pagination breaks between the rows of a vertical
//! grid as it does between those of a horizontal grid in a horizontal
//! document, in the order of the rows the grid placed. In a horizontal
//! document a vertical grid is an orthogonal flow and is not broken.

use super::*;
use raikiri_traits::PageBox;

fn page_100() -> PageBox {
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    page
}

/// A grid with one 40px column and two 60px rows in a document whose root has
/// `root_css`, holding a first item on row 1 and a second on row 2, each
/// styled with the CSS given. Returns the document, the grid, and the items.
fn paginated(
    root_css: &str,
    grid_css: &str,
    first_css: &str,
    second_css: &str,
) -> (Document, usize, [usize; 2], usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some(root_css));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let grid = doc.append_element(
        Some(body),
        "div",
        Some(
            format!(
                "display:grid;grid-template-columns:40px;grid-template-rows:60px 60px;{grid_css}"
            )
            .as_str(),
        ),
    );
    let first = doc.append_element(
        Some(grid),
        "div",
        Some(format!("grid-column:1;grid-row:1;{first_css}").as_str()),
    );
    let second = doc.append_element(
        Some(grid),
        "div",
        Some(format!("grid-column:1;grid-row:2;{second_css}").as_str()),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    let slices = layout_pages(&mut doc, &cascade, page_100()).expect("pagination");
    (doc, grid, [first, second], slices.len())
}

fn origin(doc: &Document, id: usize) -> (f32, f32) {
    let location = doc.nodes[id].unrounded_layout.location;
    (location.x, location.y)
}

#[test]
fn a_vertical_grid_in_a_vertical_document_breaks_between_its_rows() {
    let (doc, _, [first, second], pages) = paginated(
        "display:block;writing-mode:vertical-rl",
        "",
        "width:60px",
        "width:60px",
    );
    // The 120px grid does not fit the 100px page: row 1, the rightmost, stays
    // and row 2 continues on the next page, to the left of the first.
    assert_eq!(pages, 2);
    assert_eq!(origin(&doc, first), (60.0, 0.0));
    assert_eq!(origin(&doc, second), (-40.0, 0.0));
}

#[test]
fn the_rows_of_a_vertical_grid_are_ordered_by_resolved_placement() {
    // `order` puts row 2 first in the order-modified document order. Row 1
    // is still the first one the pages hold.
    let (doc, grid, [first, second], pages) = paginated(
        "display:block;writing-mode:vertical-rl",
        "",
        "order:1;width:60px",
        "order:0;width:60px",
    );
    assert_eq!(pages, 2);
    assert_eq!(doc.nodes[grid].layout_children(), &[second, first]);
    assert_eq!(origin(&doc, first), (60.0, 0.0));
    assert_eq!(origin(&doc, second), (-40.0, 0.0));
}

#[test]
fn a_forced_break_before_a_row_of_a_vertical_grid_follows_the_resolved_row() {
    // Row 2 is first in the source and breaks before itself: the break comes
    // after resolved row 1, not before the first item in the source.
    let (doc, grid, [first, second], pages) = paginated(
        "display:block;writing-mode:vertical-rl",
        "grid-template-rows:30px 30px",
        "order:1;width:30px",
        "order:0;width:30px;break-before:page",
    );
    assert_eq!(pages, 2);
    assert_eq!(doc.nodes[grid].layout_children(), &[second, first]);
    assert_eq!(origin(&doc, first), (30.0, 0.0));
    let (second_x, _) = origin(&doc, second);
    assert!(second_x < 0.0, "row 2 is on the next page: {second_x}");
}

#[test]
fn a_vertical_grid_in_a_horizontal_document_is_not_broken() {
    // An orthogonal flow is monolithic: the 150px grid is taller than the
    // 100px page and keeps both its rows side by side.
    let (doc, grid, [first, second], _) = paginated(
        "display:block",
        "writing-mode:vertical-rl;grid-template-columns:150px",
        "width:60px",
        "width:60px",
    );
    assert_eq!(doc.nodes[grid].unrounded_layout.size.height, 150.0);
    assert_eq!(origin(&doc, first), (60.0, 0.0));
    assert_eq!(origin(&doc, second), (0.0, 0.0));
}
