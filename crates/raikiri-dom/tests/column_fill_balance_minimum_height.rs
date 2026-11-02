//! The columns of a multicolumn container with an auto height and the default
//! `column-fill: balance` are as tall as their content needs; a minimum height
//! only makes the container taller, below them (CSS Multi-column 1 section
//! 8.1; CSS 2 section 10.7). Columns that fill in turn are as tall as the
//! container's content box, so there the minimum height does reach them.
//!
//! The expected rectangles are the ones Chromium gives the same markup: in an
//! 800px by 600px viewport with a body margin of 8px, and, for printing, on
//! 180px by 160px pages with no margin. Each is the left, top, width and
//! height of the border box of an element with an `id`; the one of a box in
//! several columns or on several pages is the union of its pieces on a page.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The left, top, width and height of a box.
type Rect = (f32, f32, f32, f32);

/// An empty block `id` of `height` px, with `extra` style.
fn block(id: &str, height: u32, extra: &str) -> String {
    format!("<div id={id} style=\"height:{height}px;{extra}\"></div>")
}

/// The union of the pieces of each element with an `id`, on each page, in the
/// order the pages show them.
fn rects_of(html: &str) -> Vec<(String, usize, Rect)> {
    let resources = RenderResources::new();
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
    let mut rects: Vec<(String, usize, Rect)> = Vec::new();
    for (index, page) in result.pages().enumerate() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            let rect = fragment.rect();
            let (left, top) = (rect.x, rect.y);
            let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
            match rects
                .iter_mut()
                .find(|(seen, page, _)| seen == id && *page == index)
            {
                Some((_, _, own)) => {
                    let (x0, y0) = (own.0.min(left), own.1.min(top));
                    let (x1, y1) = ((own.0 + own.2).max(right), (own.1 + own.3).max(bottom));
                    *own = (x0, y0, x1 - x0, y1 - y0);
                }
                None => rects.push((
                    id.to_owned(),
                    index,
                    (left, top, right - left, bottom - top),
                )),
            }
        }
    }
    rects
}

/// Whether `got` is `expected` to within a hundredth of a pixel.
fn same(got: Rect, expected: Rect) -> bool {
    [
        (got.0, expected.0),
        (got.1, expected.1),
        (got.2, expected.2),
        (got.3, expected.3),
    ]
    .iter()
    .all(|(a, b)| (a - b).abs() <= 0.01)
}

/// Compares the rectangles of the elements named in `expected`, each with the
/// page it is on.
fn check_rects(label: &str, html: &str, expected: &[(&str, usize, Rect)]) {
    let rects = rects_of(html);
    for &(id, page, rect) in expected {
        let got = rects
            .iter()
            .find(|(seen, on, _)| *seen == id && *on == page)
            .unwrap_or_else(|| panic!("{label}: no fragment for #{id} on page {page}: {rects:?}"));
        assert!(
            same(got.2, rect),
            "{label}: #{id} on page {page} is {:?}, expected {rect:?}",
            got.2
        );
    }
}

/// Lays out `body` on a screen of 800px by 600px with a body margin of 8px.
fn screen(label: &str, body: &str, expected: &[(&str, Rect)]) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let expected: Vec<_> = expected.iter().map(|&(id, rect)| (id, 0, rect)).collect();
    check_rects(label, &html, &expected);
}

/// Lays out `body` on pages of 180px by 160px with no margin, where `.mc` is
/// two 40px columns with a 20px gap.
fn paged(label: &str, body: &str, expected: &[(&str, usize, Rect)]) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 180px 160px; margin: 0 }} \
         body {{ margin: 0 }} div {{ box-sizing: border-box }} \
         .mc {{ width: 100px; column-count: 2; column-gap: 20px }}</style>{body}"
    );
    check_rects(label, &html, expected);
}

/// A container of two columns of 100px with no gap.
fn container(style: &str, content: &str) -> String {
    format!("<div id=m style=\"columns:2;column-gap:0;width:200px;{style}\">{content}</div>")
}

#[test]
fn a_minimum_height_does_not_lengthen_the_balanced_columns() {
    let two = block("a", 40, "") + &block("b", 40, "");
    screen(
        "min-height above the content",
        &container("min-height:100px", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
    screen(
        "min-height between the balanced and the stacked content",
        &container("min-height:50px", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 50.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
    screen(
        "min-height at the balanced height",
        &container("min-height:40px", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
    screen(
        "the box after the container follows its minimum",
        &(container("min-height:100px", &two) + "<div id=z style=\"height:10px\"></div>"),
        &[
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 108.0, 784.0, 10.0)),
        ],
    );
}

#[test]
fn content_taller_than_the_minimum_height_sets_the_container_height() {
    let four = block("a", 60, "") + &block("b", 60, "") + &block("c", 60, "") + &block("d", 60, "");
    screen(
        "four blocks",
        &container("min-height:50px", &four),
        &[
            ("m", (8.0, 8.0, 200.0, 120.0)),
            ("a", (8.0, 8.0, 100.0, 60.0)),
            ("b", (8.0, 68.0, 100.0, 60.0)),
            ("c", (108.0, 8.0, 100.0, 60.0)),
            ("d", (108.0, 68.0, 100.0, 60.0)),
        ],
    );
    let tall = block("a", 100, "") + &block("b", 100, "");
    screen(
        "content as tall as the minimum",
        &container("min-height:100px", &tall),
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 100.0)),
            ("b", (108.0, 8.0, 100.0, 100.0)),
        ],
    );
}

#[test]
fn a_box_taller_than_the_balanced_columns_is_cut_under_the_minimum_height() {
    screen(
        "one box",
        &container("min-height:300px", &block("a", 200, "")),
        &[
            ("m", (8.0, 8.0, 200.0, 300.0)),
            ("a", (8.0, 8.0, 200.0, 100.0)),
        ],
    );
}

#[test]
fn the_minimum_height_counts_the_padding_and_border_of_the_container() {
    let two = block("a", 40, "") + &block("b", 40, "");
    screen(
        "border-box",
        &container("min-height:100px;padding:10px", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (18.0, 18.0, 90.0, 40.0)),
            ("b", (108.0, 18.0, 90.0, 40.0)),
        ],
    );
    screen(
        "content-box",
        &container(
            "box-sizing:content-box;min-height:100px;padding:10px;border:5px solid",
            &two,
        ),
        &[
            ("m", (8.0, 8.0, 230.0, 130.0)),
            ("a", (23.0, 23.0, 100.0, 40.0)),
            ("b", (123.0, 23.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn a_minimum_height_beats_a_maximum_height_below_it() {
    let two = block("a", 40, "") + &block("b", 40, "");
    screen(
        "max-height below min-height",
        &container("min-height:100px;max-height:30px", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
    let four = block("a", 40, "") + &block("b", 40, "") + &block("c", 40, "") + &block("d", 40, "");
    screen(
        "max-height above min-height",
        &container("min-height:50px;max-height:60px", &four),
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 8.0, 200.0, 60.0)),
            ("c", (108.0, 28.0, 100.0, 40.0)),
            ("d", (208.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn a_height_below_the_minimum_height_balances_in_the_taller_container() {
    let two = block("a", 40, "") + &block("b", 40, "");
    screen(
        "height below min-height",
        &container("height:100px;min-height:150px", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 150.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
    screen(
        "height above min-height",
        &container("height:100px;min-height:60px", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn columns_that_fill_in_turn_are_as_tall_as_the_minimum_height() {
    let two = block("a", 40, "") + &block("b", 40, "");
    screen(
        "content below the minimum",
        &container("min-height:100px;column-fill:auto", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 48.0, 100.0, 40.0)),
        ],
    );
    let three = block("a", 60, "") + &block("b", 60, "") + &block("c", 60, "");
    screen(
        "content above the minimum",
        &container("min-height:100px;column-fill:auto", &three),
        &[
            ("m", (8.0, 8.0, 200.0, 180.0)),
            ("c", (8.0, 128.0, 100.0, 60.0)),
        ],
    );
    screen(
        "a height below the minimum",
        &container("height:50px;min-height:100px;column-fill:auto", &two),
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("b", (8.0, 48.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn the_balanced_columns_of_a_printed_container_stay_on_the_page_it_starts_on() {
    let two = block("a", 40, "") + &block("b", 40, "");
    let rest = |style: &str| {
        format!(
            "<div id=m class=mc style=\"{style}\">{two}</div><div id=z style=\"height:10px;width:30px\"></div>"
        )
    };
    let spacer = |height| block("s", height, "");
    paged(
        "the container fits",
        &rest("min-height:100px"),
        &[
            ("a", 0, (0.0, 0.0, 40.0, 40.0)),
            ("b", 0, (60.0, 0.0, 40.0, 40.0)),
            ("z", 0, (0.0, 100.0, 30.0, 10.0)),
        ],
    );
    // The balanced columns end on the page, and the container goes on to the
    // next page for the rest of its minimum height.
    paged(
        "the minimum height crosses the page",
        &(spacer(100) + &rest("min-height:100px")),
        &[
            ("s", 0, (0.0, 0.0, 180.0, 100.0)),
            ("a", 0, (0.0, 100.0, 40.0, 40.0)),
            ("b", 0, (60.0, 100.0, 40.0, 40.0)),
            ("z", 1, (0.0, 40.0, 30.0, 10.0)),
        ],
    );
    paged(
        "the minimum height crosses the page by a little",
        &(spacer(120) + &rest("min-height:60px")),
        &[
            ("a", 0, (0.0, 120.0, 40.0, 40.0)),
            ("b", 0, (60.0, 120.0, 40.0, 40.0)),
            ("z", 1, (0.0, 20.0, 30.0, 10.0)),
        ],
    );
    paged(
        "the minimum height is taller than a page",
        &rest("min-height:200px"),
        &[
            ("a", 0, (0.0, 0.0, 40.0, 40.0)),
            ("b", 0, (60.0, 0.0, 40.0, 40.0)),
            ("z", 1, (0.0, 40.0, 30.0, 10.0)),
        ],
    );
    paged(
        "a maximum height above the minimum",
        &(spacer(100) + &rest("min-height:100px;max-height:120px")),
        &[
            ("a", 0, (0.0, 100.0, 40.0, 40.0)),
            ("b", 0, (60.0, 100.0, 40.0, 40.0)),
            ("z", 1, (0.0, 40.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn the_balanced_columns_of_a_printed_container_end_where_the_content_does() {
    let rest = |style: &str, content: &str| {
        format!(
            "<div id=m class=mc style=\"{style}\">{content}</div><div id=z style=\"height:10px;width:30px\"></div>"
        )
    };
    let four = block("a", 60, "") + &block("b", 60, "") + &block("c", 60, "") + &block("d", 60, "");
    paged(
        "content taller than the minimum",
        &rest("min-height:100px", &four),
        &[
            ("a", 0, (0.0, 0.0, 40.0, 60.0)),
            ("b", 0, (0.0, 60.0, 40.0, 60.0)),
            ("c", 0, (60.0, 0.0, 40.0, 60.0)),
            ("d", 0, (60.0, 60.0, 40.0, 60.0)),
            ("z", 0, (0.0, 120.0, 30.0, 10.0)),
        ],
    );
    let two = block("a", 40, "") + &block("b", 40, "");
    paged(
        "columns that fill in turn",
        &rest("min-height:100px;column-fill:auto", &two),
        &[
            ("a", 0, (0.0, 0.0, 40.0, 40.0)),
            ("b", 0, (0.0, 40.0, 40.0, 40.0)),
            ("z", 0, (0.0, 100.0, 30.0, 10.0)),
        ],
    );
}
