//! A wrapper around one paragraph beside other boxes in columns: Chromium
//! breaks the paragraph between its lines where a column ends, as it does a
//! paragraph with no wrapper.

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
fn a_wrapped_paragraph_after_a_box_breaks_between_columns() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><div><div style='width:20px'>cd ef gh ij</div></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 28),
            (0, 'd', 18, 28),
            (0, 'e', 8, 38),
            (0, 'f', 18, 38),
            (0, 'g', 108, 8),
            (0, 'h', 118, 8),
            (0, 'i', 108, 18),
            (0, 'j', 118, 18)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:40px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><div><div>cd ef gh ij</div></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 28),
            (0, 'd', 18, 28),
            (0, 'e', 8, 38),
            (0, 'f', 18, 38),
            (0, 'g', 28, 8),
            (0, 'h', 38, 8),
            (0, 'i', 28, 18),
            (0, 'j', 38, 18)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><div><div><div style='width:20px'>cd ef gh ij</div></div></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 28),
            (0, 'd', 18, 28),
            (0, 'e', 8, 38),
            (0, 'f', 18, 38),
            (0, 'g', 108, 8),
            (0, 'h', 118, 8),
            (0, 'i', 108, 18),
            (0, 'j', 118, 18)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:10px'>ab</div><div style='padding-top:10px'><div style='width:20px'>cd ef gh ij</div></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 28),
            (0, 'd', 18, 28),
            (0, 'e', 8, 38),
            (0, 'f', 18, 38),
            (0, 'g', 108, 8),
            (0, 'h', 118, 8),
            (0, 'i', 108, 18),
            (0, 'j', 118, 18)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><div style='margin-top:10px'><div style='width:20px'>cd ef gh ij</div></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 38),
            (0, 'd', 18, 38),
            (0, 'e', 8, 48),
            (0, 'f', 18, 48),
            (0, 'g', 108, 8),
            (0, 'h', 118, 8),
            (0, 'i', 108, 18),
            (0, 'j', 118, 18)
        ]
    );
}

#[test]
fn balanced_columns_break_a_wrapped_paragraph_after_a_box() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px'><div style='height:20px'>ab</div><div><div style='width:20px'>cd ef gh ij</div></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 28),
            (0, 'd', 18, 28),
            (0, 'e', 8, 38),
            (0, 'f', 18, 38),
            (0, 'g', 108, 8),
            (0, 'h', 118, 8),
            (0, 'i', 108, 18),
            (0, 'j', 118, 18)
        ]
    );
}

#[test]
fn a_button_in_a_wrapper_moves_whole_beside_other_boxes() {
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><div><button style='all:unset;display:block;width:20px'>cd ef gh ij</button></div></div>"
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
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div style='height:20px'>ab</div><div><button style='all:unset;display:block;width:20px'>cd ef</button><div style='width:20px'>gh ij kl</div></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 28),
            (0, 'd', 18, 28),
            (0, 'e', 8, 38),
            (0, 'f', 18, 38),
            (0, 'g', 108, 8),
            (0, 'h', 118, 8),
            (0, 'i', 108, 18),
            (0, 'j', 118, 18),
            (0, 'k', 108, 28),
            (0, 'l', 118, 28)
        ]
    );
    assert_eq!(
        glyphs(
            "<div style='columns:2;width:200px;column-gap:0;height:50px;column-fill:auto'><div><div style='width:20px'>ab cd</div><button style='all:unset;display:block;width:20px'>ef gh ij kl</button></div></div>"
        ),
        [
            (0, 'a', 8, 8),
            (0, 'b', 18, 8),
            (0, 'c', 8, 18),
            (0, 'd', 18, 18),
            (0, 'e', 108, 8),
            (0, 'f', 118, 8),
            (0, 'g', 108, 18),
            (0, 'h', 118, 18),
            (0, 'i', 108, 28),
            (0, 'j', 118, 28),
            (0, 'k', 108, 38),
            (0, 'l', 118, 38)
        ]
    );
}
