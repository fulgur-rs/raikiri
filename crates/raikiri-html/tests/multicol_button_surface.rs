//! A block-level button inside columns is monolithic, as Chromium lays it
//! out: a column end never breaks between its lines, so the button goes
//! whole to the next column when it does not fit, and stays whole, past the
//! column end, when it is taller than a column.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph, left edge and top of every glyph in `body`, on 300x200
/// pages with Ahem 10px/10px.
fn glyphs(body: &str) -> Vec<(u32, char, i32, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:300px 200px;margin:0}}body{{margin:8px;font:10px/10px Ahem}}</style><body>{body}</body>"
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
    let mut glyphs = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let mut x = run.origin.0;
            for glyph in run.text.chars() {
                if !glyph.is_whitespace() {
                    glyphs.push((
                        index,
                        glyph,
                        x.round() as i32,
                        (run.origin.1 - run.ascent).round() as i32,
                    ));
                }
                x += run.font_size;
            }
        }
    }
    glyphs.sort_unstable();
    glyphs.dedup();
    glyphs
}

#[test]
fn a_button_that_does_not_fit_moves_whole_to_the_next_column() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><button style='all:unset;display:block;width:20px'>cd ef gh ij</button></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 108, 8),
            (0, 'd', 118, 8),
            (0, 'e', 108, 18),
            (0, 'f', 118, 18),
            (0, 'g', 108, 28),
            (0, 'h', 118, 28),
            (0, 'i', 108, 38),
            (0, 'j', 118, 38)
        ]
    );
}

#[test]
fn a_button_that_fits_stays_in_its_column() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><button style='all:unset;display:block;width:20px'>cd ef</button></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 28),
            (0, 'd', 18, 28),
            (0, 'e', 8, 38),
            (0, 'f', 18, 38)
        ]
    );
}

#[test]
fn a_button_with_its_default_style_moves_whole() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><button style='display:block;font:inherit;padding:0;border:0;width:20px'>cd ef gh ij</button></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 108, 8),
            (0, 'd', 118, 8),
            (0, 'e', 108, 18),
            (0, 'f', 118, 18),
            (0, 'g', 108, 28),
            (0, 'h', 118, 28),
            (0, 'i', 108, 38),
            (0, 'j', 118, 38)
        ]
    );
}

#[test]
fn a_button_of_blocks_moves_whole() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><button style='all:unset;display:block'><div>cd</div><div>ef</div><div>gh</div><div>ij</div></button></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 108, 8),
            (0, 'd', 118, 8),
            (0, 'e', 108, 18),
            (0, 'f', 118, 18),
            (0, 'g', 108, 28),
            (0, 'h', 118, 28),
            (0, 'i', 108, 38),
            (0, 'j', 118, 38)
        ]
    );
}

#[test]
fn buttons_with_a_height_of_their_own_stay_whole() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><button style='all:unset;display:block;height:10px'>ab</button><button style='all:unset;display:block;width:20px;height:40px'>cd ef gh ij</button></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'd', 18, 18),
            (0, 'e', 8, 28),
            (0, 'f', 18, 28),
            (0, 'g', 8, 38),
            (0, 'h', 18, 38),
            (0, 'i', 8, 48),
            (0, 'j', 18, 48)
        ]
    );
}

#[test]
fn balanced_columns_keep_a_button_whole() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0'><button style='all:unset;display:block;width:20px'>ab cd ef gh</button></div><p style='margin:0'>Z</p>"
        ),
        [
            (0, 'Z', 8, 48),
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'd', 18, 18),
            (0, 'e', 8, 28),
            (0, 'f', 18, 28),
            (0, 'g', 8, 38),
            (0, 'h', 18, 38)
        ]
    );
}

#[test]
fn a_button_taller_than_the_column_overflows_it_whole() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:30px;column-fill:auto'><button style='all:unset;display:block;width:20px'>ab cd ef gh</button></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'd', 18, 18),
            (0, 'e', 8, 28),
            (0, 'f', 18, 28),
            (0, 'g', 8, 38),
            (0, 'h', 18, 38)
        ]
    );
}

#[test]
fn a_wrapped_button_moves_whole_after_a_paragraph() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><div><p style='margin:0'>kl</p><button style='all:unset;display:block;width:20px'>cd ef gh ij</button></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 108, 8),
            (0, 'd', 118, 8),
            (0, 'e', 108, 18),
            (0, 'f', 118, 18),
            (0, 'g', 108, 28),
            (0, 'h', 118, 28),
            (0, 'i', 108, 38),
            (0, 'j', 118, 38),
            (0, 'k', 8, 28),
            (0, 'l', 18, 28)
        ]
    );
}

#[test]
fn a_button_taller_than_the_column_after_a_paragraph_overflows_the_next_column() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:30px;column-fill:auto'><p style='margin:0'>kl</p><button style='all:unset;display:block;width:20px'>ab cd ef gh</button></div>"
        ),
        [
            (0, 'a', 108, 8),
            (0, 'b', 118, 8),
            (0, 'c', 108, 18),
            (0, 'd', 118, 18),
            (0, 'e', 108, 28),
            (0, 'f', 118, 28),
            (0, 'g', 108, 38),
            (0, 'h', 118, 38),
            (0, 'k', 8, 8),
            (0, 'l', 18, 8)
        ]
    );
}

#[test]
fn a_button_that_fits_a_column_but_not_below_its_margin_moves_to_the_next() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><button style='all:unset;display:block;width:20px;margin-top:20px'>cd ef gh ij</button></div>"
        ),
        [
            (0, 'c', 108, 8),
            (0, 'd', 118, 8),
            (0, 'e', 108, 18),
            (0, 'f', 118, 18),
            (0, 'g', 108, 28),
            (0, 'h', 118, 28),
            (0, 'i', 108, 38),
            (0, 'j', 118, 38)
        ]
    );
}

/// A button that fits a column, but not below its top margin, goes to the
/// next column even from an empty first one, and the break there cuts the
/// margin. Balanced columns place it the same way.
#[test]
fn a_button_below_its_top_margin_moves_even_from_an_empty_column() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><button style='all:unset;display:block;width:20px;margin-top:20px'>ab cd ef gh</button><div>xy</div></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 108, 8),
            (0, 'b', 118, 8),
            (0, 'c', 108, 18),
            (0, 'd', 118, 18),
            (0, 'e', 108, 28),
            (0, 'f', 118, 28),
            (0, 'g', 108, 38),
            (0, 'h', 118, 38),
            (0, 'x', 108, 48),
            (0, 'y', 118, 48)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div>xy</div><button style='all:unset;display:block;width:20px;margin-top:20px'>ab cd ef gh</button></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 108, 8),
            (0, 'b', 118, 8),
            (0, 'c', 108, 18),
            (0, 'd', 118, 18),
            (0, 'e', 108, 28),
            (0, 'f', 118, 28),
            (0, 'g', 108, 38),
            (0, 'h', 118, 38),
            (0, 'x', 8, 8),
            (0, 'y', 18, 8)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px'><button style='all:unset;display:block;width:20px;margin-top:20px'>ab cd ef gh</button><div>xy</div></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 108, 8),
            (0, 'b', 118, 8),
            (0, 'c', 108, 18),
            (0, 'd', 118, 18),
            (0, 'e', 108, 28),
            (0, 'f', 118, 28),
            (0, 'g', 108, 38),
            (0, 'h', 118, 38),
            (0, 'x', 108, 48),
            (0, 'y', 118, 48)
        ]
    );
}

/// Balanced columns are at least as tall as a button in them, which never
/// overflows a shorter trial column.
#[test]
fn balanced_columns_are_at_least_as_tall_as_a_button() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px'><button style='all:unset;display:block;width:20px'>ab cd ef gh</button><div>xy</div></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'd', 18, 18),
            (0, 'e', 8, 28),
            (0, 'f', 18, 28),
            (0, 'g', 8, 38),
            (0, 'h', 18, 38),
            (0, 'x', 108, 8),
            (0, 'y', 118, 8)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0'><button style='all:unset;display:block;width:20px'>ab cd ef gh</button><div>xy</div></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 48),
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'd', 18, 18),
            (0, 'e', 8, 28),
            (0, 'f', 18, 28),
            (0, 'g', 8, 38),
            (0, 'h', 18, 38),
            (0, 'x', 108, 8),
            (0, 'y', 118, 8)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0'><button style='all:unset;display:block;width:20px'>ab cd ef gh</button><div style='width:20px'>pq rs tu</div></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 48),
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'd', 18, 18),
            (0, 'e', 8, 28),
            (0, 'f', 18, 28),
            (0, 'g', 8, 38),
            (0, 'h', 18, 38),
            (0, 'p', 108, 8),
            (0, 'q', 118, 8),
            (0, 'r', 108, 18),
            (0, 's', 118, 18),
            (0, 't', 108, 28),
            (0, 'u', 118, 28)
        ]
    );
}

/// The margin of a block inside a button stays inside the button, which
/// fits a column or not by its border box: one taller than the column stays
/// whole in it, and one that moves keeps that margin above its lines.
#[test]
fn a_button_keeps_the_margins_inside_it() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><button style='all:unset;display:block;width:20px;overflow:hidden'><div style='margin-top:20px'>ab cd ef gh</div></button></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 8, 28),
            (0, 'b', 18, 28),
            (0, 'c', 8, 38),
            (0, 'd', 18, 38),
            (0, 'e', 8, 48),
            (0, 'f', 18, 48),
            (0, 'g', 8, 58),
            (0, 'h', 18, 58)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><button style='all:unset;display:block;width:20px'><div style='margin-top:20px'>ab cd ef gh</div></button></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 8, 28),
            (0, 'b', 18, 28),
            (0, 'c', 8, 38),
            (0, 'd', 18, 38),
            (0, 'e', 8, 48),
            (0, 'f', 18, 48),
            (0, 'g', 8, 58),
            (0, 'h', 18, 58)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div>xy</div><button style='all:unset;display:block;width:20px;overflow:hidden'><div style='margin-top:20px'>ab cd ef</div></button></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 108, 28),
            (0, 'b', 118, 28),
            (0, 'c', 108, 38),
            (0, 'd', 118, 38),
            (0, 'e', 108, 48),
            (0, 'f', 118, 48),
            (0, 'x', 8, 8),
            (0, 'y', 18, 8)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><button style='all:unset;display:block;width:20px;overflow:hidden'><div style='margin-top:20px'>ab cd</div></button></div><div>Z</div>"
        ),
        [
            (0, 'Z', 8, 58),
            (0, 'a', 8, 28),
            (0, 'b', 18, 28),
            (0, 'c', 8, 38),
            (0, 'd', 18, 38)
        ]
    );
}

#[test]
fn a_button_in_a_wrapper_moves_past_its_top_margin_with_the_wrapper_left_behind() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div><button style='all:unset;display:block;width:20px;margin-top:20px'>cd ef gh ij</button></div></div>"
        ),
        [
            (0, 'c', 108, 8),
            (0, 'd', 118, 8),
            (0, 'e', 108, 18),
            (0, 'f', 118, 18),
            (0, 'g', 108, 28),
            (0, 'h', 118, 28),
            (0, 'i', 108, 38),
            (0, 'j', 118, 38)
        ]
    );
}

#[test]
fn auto_height_columns_balance_the_text_around_a_whole_button() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0'><button style='all:unset;display:block;width:20px'>ab</button><div style='width:20px'>c1 c2 c3 c4 c5 c6 c7 c8 c9 c0</div></div><div>Z</div>"
        ),
        [
            (0, '0', 118, 48),
            (0, '1', 18, 18),
            (0, '2', 18, 28),
            (0, '3', 18, 38),
            (0, '4', 18, 48),
            (0, '5', 18, 58),
            (0, '6', 118, 8),
            (0, '7', 118, 18),
            (0, '8', 118, 28),
            (0, '9', 118, 38),
            (0, 'Z', 8, 68),
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'c', 8, 28),
            (0, 'c', 8, 38),
            (0, 'c', 8, 48),
            (0, 'c', 8, 58),
            (0, 'c', 108, 8),
            (0, 'c', 108, 18),
            (0, 'c', 108, 28),
            (0, 'c', 108, 38),
            (0, 'c', 108, 48)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0'><div style='width:20px'>d1 d2 d3</div><button style='all:unset;display:block;width:20px'>ab cd</button><div style='width:20px'>c1 c2 c3 c4 c5 c6</div></div><div>Z</div>"
        ),
        [
            (0, '1', 18, 8),
            (0, '1', 118, 8),
            (0, '2', 18, 18),
            (0, '2', 118, 18),
            (0, '3', 18, 28),
            (0, '3', 118, 28),
            (0, '4', 118, 38),
            (0, '5', 118, 48),
            (0, '6', 118, 58),
            (0, 'Z', 8, 68),
            (0, 'a', 8, 38),
            (0, 'b', 18, 38),
            (0, 'c', 8, 48),
            (0, 'c', 108, 8),
            (0, 'c', 108, 18),
            (0, 'c', 108, 28),
            (0, 'c', 108, 38),
            (0, 'c', 108, 48),
            (0, 'c', 108, 58),
            (0, 'd', 8, 8),
            (0, 'd', 8, 18),
            (0, 'd', 8, 28),
            (0, 'd', 18, 48)
        ]
    );
}
