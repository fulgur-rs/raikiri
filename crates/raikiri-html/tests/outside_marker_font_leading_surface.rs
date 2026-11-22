//! Where the first line of a list item stands when its outside marker's text
//! reaches higher above the baseline than the line does. The marker sits on
//! the first line's baseline with its own font and `line-height`, so the line
//! moves down by the difference, and the item grows with it, as Chromium
//! lays it out. The positions are the ones Chromium gives the same documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// Every word of `body` but the markers as `word@x,y`, the left and top of
/// its glyphs, in source order, on a 100x100 page with 10px Ahem lines 10px
/// apart. Markers are left out: only the lines they move are compared.
fn words(style: &str, body: &str) -> String {
    let result = lay_out(style, body);
    let page = result.page(0).expect("one page");
    let mut words = Vec::new();
    for run in page.text_runs() {
        let mut x = run.origin.0;
        let y = (run.origin.1 - ASCENT).round();
        for word in run.text.split(' ') {
            let word = word.trim();
            if !word.is_empty() && word.chars().all(char::is_alphabetic) {
                words.push(format!("{word}@{},{}", x.round(), y));
            }
            x += ADVANCE * (word.chars().count() + 1) as f32;
        }
    }
    words.join(" ")
}

fn lay_out(style: &str, body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}ul,ol{{margin:0;padding:0 0 0 40px}}\
         .m{{columns:2;column-gap:0;width:100px}}{style}</style><body>{body}</body>"
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

const BIG: &str = "li::marker{font-size:30px}";

#[test]
fn a_larger_marker_moves_the_first_line_down() {
    // A 30px Ahem marker on a 10px line reaches 24 - 10 = 14px above its
    // baseline, 6px more than the line's 8px.
    assert_eq!(
        words(BIG, "<ul><li>ab</li></ul><div>Z</div>"),
        "ab@40,6 Z@0,16"
    );
    assert_eq!(
        words(BIG, "<ol><li>ab<br>cd</li><li>ef</li></ol><div>Z</div>"),
        "ab@40,6 cd@40,16 ef@40,32 Z@0,42"
    );
    // The first line of a block inside the item moves too.
    assert_eq!(
        words(
            BIG,
            "<ul><li><div>ab</div><div>cd</div></li></ul><div>Z</div>"
        ),
        "ab@40,6 cd@40,16 Z@0,26"
    );
    // An item that holds nothing gets a line for its marker.
    assert_eq!(words(BIG, "<ul><li></li></ul><div>Z</div>"), "Z@0,16");
    // Text that is the marker's content counts as its glyphs do.
    assert_eq!(
        words(
            "li::marker{font-size:30px;content:'-'}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,6 Z@0,16"
    );
}

#[test]
fn an_empty_marker_moves_nothing() {
    // An empty string gives the marker an empty line, which takes no room.
    assert_eq!(
        words(
            "li::marker{font-size:30px;content:''}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,0 Z@0,10"
    );
    assert_eq!(
        words(
            "li{list-style-type:''}li::marker{font-size:30px}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,0 Z@0,10"
    );
    // So does a quote that inserts nothing.
    for content in ["no-open-quote", "open-quote;quotes:none"] {
        assert_eq!(
            words(
                &format!("li::marker{{font-size:30px;content:{content}}}"),
                "<ul><li>ab</li></ul><div>Z</div>"
            ),
            "ab@40,0 Z@0,10"
        );
    }
    assert_eq!(
        words(
            "li::marker{font-size:30px;content:open-quote}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,6 Z@0,16"
    );
    // A counter after an empty string still shows text.
    assert_eq!(
        words(
            "li::marker{font-size:30px;content:'' counter(list-item)}",
            "<ol><li>ab</li></ol><div>Z</div>"
        ),
        "ab@40,6 Z@0,16"
    );
}

#[test]
fn an_image_marker_has_the_line_of_its_font() {
    // A small image alone still sits on a line whose strut has the marker's
    // font.
    assert_eq!(
        words(
            "li::marker{font-size:30px;content:url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/%3E\")}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,6 Z@0,16"
    );
}

#[test]
fn the_first_box_with_a_baseline_moves() {
    // Boxes with no line come before the first line, which moves down.
    assert_eq!(
        words(
            BIG,
            "<ul><li><div></div><div style=height:5px></div><p style=margin:0>ab</p>\
             <p style=margin:0>cd</p></li></ul><div>Z</div>"
        ),
        "ab@40,11 cd@40,21 Z@0,31"
    );
}

#[test]
fn the_marker_line_height_sets_how_far_it_reaches() {
    assert_eq!(
        words(
            "li::marker{font-size:30px;line-height:60px}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,31 Z@0,41"
    );
    assert_eq!(
        words(
            "li::marker{font-size:30px;line-height:2}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,31 Z@0,41"
    );
    assert_eq!(
        words(
            "li::marker{font-size:30px;line-height:normal}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,16 Z@0,26"
    );
    // The marker inherits the item's line height.
    assert_eq!(
        words(
            BIG,
            "<ul><li style='line-height:40px'>ab</li></ul><div>Z</div>"
        ),
        "ab@40,21 Z@0,46"
    );
}

#[test]
fn a_marker_that_fits_the_line_moves_nothing() {
    assert_eq!(
        words(
            "li::marker{font-size:5px}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,0 Z@0,10"
    );
    assert_eq!(
        words(
            "li::marker{font-size:30px;content:none}",
            "<ul><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,0 Z@0,10"
    );
    assert_eq!(
        words(
            BIG,
            "<ul style='list-style:none'><li>ab</li></ul><div>Z</div>"
        ),
        "ab@40,0 Z@0,10"
    );
}

#[test]
fn a_larger_marker_moves_the_line_of_an_item_with_a_height() {
    assert_eq!(
        words(
            "li::marker{font-size:30px}ul{padding-left:30px}",
            "<ul><li style='height:50px'>ab</li>\
             <li style='width:20px;list-style:none'>cd ef gh ij</li></ul><div>Z</div>"
        ),
        "ab@30,6 cd@30,50 ef@30,60 gh@30,70 ij@30,80 Z@0,90"
    );
    // In columns too.
    assert_eq!(
        words(
            "li::marker{font-size:30px}ul{padding-left:30px}",
            "<div class=m><ul><li>ab</li><li>cd</li><li>ef</li></ul><div>Z</div></div>"
        ),
        "ab@30,6 cd@30,22 ef@80,6 Z@50,16"
    );
}
