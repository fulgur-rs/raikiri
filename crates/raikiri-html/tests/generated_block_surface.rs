//! Block-level `::before` and `::after` boxes take part in the layout of
//! their element: their height and edges move the content after them, size
//! the element and count towards the pages.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lay `body` out on 100x60 pages with no margins, in 20px Ahem.
fn lay_out(body: &str, css: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 60px;margin:0}}body{{margin:0;font:20px/20px Ahem}}p{{margin:0}}{css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    result
}

/// The page index and pen position of each text run, in page order.
fn runs(document: &DocumentLayout) -> Vec<(u32, String, (f32, f32))> {
    document
        .pages()
        .flat_map(|page| {
            page.text_runs()
                .into_iter()
                .map(move |run| (page.index(), run.text.to_owned(), run.origin))
        })
        .collect()
}

/// The page-local rect of the box fragments of the element `tag`, in page
/// order: `(page, y, width, height)`.
fn boxes(document: &DocumentLayout, tag: &str) -> Vec<(u32, f32, f32, f32)> {
    document
        .pages()
        .flat_map(|page| {
            let dom = page.dom();
            page.fragments()
                .filter(|fragment| {
                    fragment.kind() == raikiri_html::FragmentKind::Box
                        && dom.local_name(fragment.node()) == Some(tag)
                })
                .map(|fragment| {
                    let rect = fragment.rect();
                    (page.index(), rect.y, rect.width, rect.height)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn empty_block_before_and_after_of_a_div_take_their_height() {
    let document = lay_out(
        "<div>A</div><p>B</p>",
        "div::before{content:'';display:block;height:5px}div::after{content:'';display:block;height:10px}",
    );
    assert_eq!(boxes(&document, "div"), vec![(0, 0.0, 100.0, 35.0)]);
    assert_eq!(
        runs(&document),
        vec![
            (0, "A".to_owned(), (0.0, 21.0)),
            (0, "B".to_owned(), (0.0, 51.0))
        ]
    );
}

#[test]
fn block_after_of_a_div_counts_towards_the_pages() {
    let document = lay_out(
        "<div>A</div>",
        "div::after{content:'';display:block;height:250px}",
    );
    assert_eq!(document.page_count(), 5);
}

#[test]
fn block_before_and_after_of_the_body_count_towards_the_pages() {
    let after = lay_out("A", "body::after{content:'';display:block;height:250px}");
    assert_eq!(after.page_count(), 5);
    let before = lay_out("A", "body::before{content:'';display:block;height:250px}");
    assert_eq!(before.page_count(), 5);
    // The text follows the box: 250px down is 10px into the fifth page.
    assert_eq!(runs(&before), vec![(4, "A".to_owned(), (0.0, 26.0))]);
}

#[test]
fn block_after_of_the_body_with_a_max_height_ends_at_it() {
    let document = lay_out(
        "A",
        "body{max-height:100px}body::after{content:'';display:block;height:250px}",
    );
    assert_eq!(document.page_count(), 2);
}

#[test]
fn block_before_text_is_set_in_by_its_edges() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'X';display:block;height:30px;border-top:5px solid;padding:5px 0 0 20px;margin-left:10px}",
    );
    assert_eq!(
        runs(&document),
        vec![
            (0, "X".to_owned(), (30.0, 26.0)),
            (0, "A".to_owned(), (0.0, 56.0))
        ]
    );
}

#[test]
fn block_before_with_a_definite_width_breaks_its_lines_at_it() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'XX YY';display:block;width:50px;max-width:45px;min-width:40px}",
    );
    // Only one 20px glyph fits twice in 40px, so the two words take a line
    // each.
    assert_eq!(
        runs(&document),
        vec![
            (0, "XX ".to_owned(), (0.0, 16.0)),
            (0, "YY".to_owned(), (0.0, 36.0)),
            (0, "A".to_owned(), (0.0, 56.0))
        ]
    );
}

#[test]
fn border_box_sizing_keeps_the_edges_inside_the_height() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'';display:block;box-sizing:border-box;height:30px;padding-top:10px;border-bottom:5px solid;max-height:25px}",
    );
    assert_eq!(runs(&document), vec![(0, "A".to_owned(), (0.0, 41.0))]);
}

#[test]
fn min_height_wins_over_height() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'';display:block;height:10px;min-height:30px}",
    );
    assert_eq!(runs(&document), vec![(0, "A".to_owned(), (0.0, 46.0))]);
}

#[test]
fn percentage_height_needs_a_definite_containing_block() {
    let definite = lay_out(
        "<div style='height:50px'>A</div>",
        "div::before{content:'';display:block;height:40%}",
    );
    assert_eq!(runs(&definite), vec![(0, "A".to_owned(), (0.0, 36.0))]);
    let auto = lay_out(
        "<div>A</div>",
        "div::before{content:'';display:block;height:40%}",
    );
    assert_eq!(runs(&auto), vec![(0, "A".to_owned(), (0.0, 16.0))]);
}

#[test]
fn empty_block_margins_collapse_through_it() {
    let document = lay_out(
        "<div>A</div><p>B</p>",
        "div::after{content:'';display:block;margin:10px 0 5px}",
    );
    assert_eq!(boxes(&document, "p"), vec![(0, 30.0, 100.0, 20.0)]);
}

#[test]
fn block_margins_stay_inside_a_bordered_element() {
    let document = lay_out(
        "<div style='border:1px solid'>A</div>",
        "div::before{content:'';display:block;height:10px;margin:7px}",
    );
    assert_eq!(boxes(&document, "div"), vec![(0, 0.0, 100.0, 46.0)]);
    assert_eq!(runs(&document), vec![(0, "A".to_owned(), (1.0, 41.0))]);
}

#[test]
fn margin_of_a_leading_block_before_with_text_collapses_with_lines_after() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'X';display:block;margin-bottom:10px}",
    );
    assert_eq!(
        runs(&document),
        vec![
            (0, "X".to_owned(), (0.0, 16.0)),
            (0, "A".to_owned(), (0.0, 46.0))
        ]
    );
}

#[test]
fn cleared_block_after_encloses_the_floats_of_its_element() {
    let floated = "<div><span style='float:left;width:20px;height:40px'></span>A</div><p>B</p>";
    let document = lay_out(floated, "div::after{content:'';display:block;clear:both}");
    assert_eq!(boxes(&document, "div"), vec![(0, 0.0, 100.0, 40.0)]);
    assert_eq!(boxes(&document, "p"), vec![(0, 40.0, 100.0, 20.0)]);
    // Without the clearance the paragraph after it starts beside the float.
    let document = lay_out(floated, "div::after{content:'';display:block}");
    assert_eq!(boxes(&document, "p"), vec![(0, 20.0, 100.0, 20.0)]);
}

#[test]
fn block_before_and_after_break_the_lines_of_an_inline_element() {
    let document = lay_out(
        "<p>A<span>B</span>C</p>",
        "p{font-size:10px;line-height:10px}span::before{content:'';display:block;height:10px}span::after{content:'X';display:block}",
    );
    assert_eq!(
        runs(&document),
        vec![
            (0, "A".to_owned(), (0.0, 8.0)),
            (0, "B".to_owned(), (0.0, 28.0)),
            (0, "X".to_owned(), (0.0, 38.0)),
            (0, "C".to_owned(), (0.0, 48.0))
        ]
    );
}

#[test]
fn block_before_of_a_table_cell_sizes_the_row() {
    let document = lay_out(
        "<table style='border-spacing:0'><tr><td style='padding:0'>A</td></tr></table>",
        "td::before{content:'';display:block;height:25px}",
    );
    assert_eq!(boxes(&document, "td")[0].3, 45.0);
}

#[test]
fn block_before_split_across_columns_keeps_its_bottom_edge() {
    let document = lay_out(
        "<div>A</div>",
        "div{columns:2;column-gap:0;height:40px}div::before{content:'X Y Z';display:block;width:20px;padding-bottom:5px}",
    );
    assert!(runs(&document).iter().any(|(_, text, _)| text == "A"));
}

#[test]
fn auto_margins_centre_a_block_before_with_a_definite_width() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'X';display:block;width:20px;margin:0 auto}",
    );
    assert_eq!(runs(&document)[0], (0, "X".to_owned(), (40.0, 16.0)));
}

#[test]
fn a_right_to_left_block_before_with_a_definite_width_sits_at_the_right() {
    let document = lay_out(
        "<div dir=rtl>A</div>",
        "div::before{content:'X';display:block;width:40px}",
    );
    assert_eq!(runs(&document)[0], (0, "X".to_owned(), (80.0, 16.0)));
}

#[test]
fn a_definite_width_block_before_widens_a_shrink_to_fit_box() {
    let document = lay_out(
        "<span style='float:left'>A</span>",
        "span::before{content:'';display:block;width:60px;padding-left:10px}",
    );
    assert_eq!(boxes(&document, "span")[0].2, 70.0);
}

#[test]
fn multicol_with_a_leading_block_before_keeps_its_height() {
    let document = lay_out(
        "<div>A</div><p>B</p>",
        "div{columns:2;column-gap:0}div::before{content:'';display:block;height:20px}",
    );
    assert_eq!(boxes(&document, "div")[0].3, 40.0);
}

#[test]
fn a_minimum_width_block_before_widens_a_shrink_to_fit_box() {
    let document = lay_out(
        "<span style='float:left'>A</span>",
        "span::before{content:'';display:block;min-width:60px;padding-left:10px}",
    );
    assert_eq!(boxes(&document, "span")[0].2, 70.0);
}

#[test]
fn a_maximum_width_breaks_the_lines_of_an_auto_width_block_before() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'XX YY';display:block;max-width:40px}",
    );
    assert_eq!(
        runs(&document)[..2],
        [
            (0, "XX ".to_owned(), (0.0, 16.0)),
            (0, "YY".to_owned(), (0.0, 36.0))
        ]
    );
}

#[test]
fn a_block_before_wider_than_its_element_keeps_its_width() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'XXX XX';display:block;width:120px}",
    );
    assert_eq!(runs(&document)[0], (0, "XXX XX".to_owned(), (0.0, 16.0)));
}

#[test]
fn a_right_to_left_block_before_wider_than_its_element_runs_out_at_the_left() {
    let document = lay_out(
        "<div dir=rtl>A</div>",
        "div::before{content:'XXX XX';display:block;width:120px}",
    );
    assert_eq!(runs(&document)[0], (0, "XXX XX".to_owned(), (-20.0, 16.0)));
}

#[test]
fn a_vertical_shrink_to_fit_box_takes_no_inline_extent_from_its_block_before() {
    let document = lay_out(
        "<span style='float:left;writing-mode:vertical-rl'>A</span>",
        "span::before{content:'';display:block;width:80px;height:20px}",
    );
    assert_eq!(boxes(&document, "span")[0].3, 20.0);
}
