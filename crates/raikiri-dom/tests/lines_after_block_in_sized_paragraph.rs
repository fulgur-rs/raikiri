//! The lines that follow a block child of a paragraph break across pages as
//! a run of their own: the line that would cross the end of a page starts
//! the next one, and `orphans` and `widows` move more lines with it. This
//! holds whatever sets the paragraph's height. A `min-height`, the quirks
//! mode fill of the body, a `max-height` or a `height` does not keep the
//! lines in place; the box grows as far as its height allows, and lines past
//! a set height overflow it. Only size containment, which ignores the
//! content, leaves them where the layout put them.
//!
//! The expected places are the ones Chromium prints for the same markup:
//! pages 200px by 100px without margins, Ahem at 10px on 20px lines. Each
//! is the page (from 1) and the top of the glyph on it.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

const STYLE: &str =
    "<style>@page{size:200px 100px;margin:0}body{margin:0;font:10px/20px Ahem}</style>";

/// The page and glyph top of the first run of each letter in `letters`, laid
/// out from `html`.
fn glyph_places(html: &str, letters: &str) -> Vec<(char, usize, f32)> {
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
    letters
        .chars()
        .map(|letter| {
            result
                .pages()
                .enumerate()
                .find_map(|(index, page)| {
                    page.text_runs()
                        .iter()
                        .find(|run| run.text.trim() == letter.to_string())
                        .map(|run| (letter, index + 1, run.origin.1 - run.ascent))
                })
                .unwrap_or_else(|| panic!("no run for {letter} in {html}"))
        })
        .collect()
}

/// A label, the document, and the page and glyph top of each letter.
type Case = (&'static str, String, Vec<(char, usize, f32)>);

fn standards(body: &str) -> String {
    format!("<!DOCTYPE html>{STYLE}{body}")
}

fn cases() -> Vec<Case> {
    let lines = "<div style=\"height:50px\">W</div>Q<br>R<br>S<br>T";
    let wrapped = |style: &str, after: &str| {
        standards(&format!(
            "<div style=\"{style}\"><div style=\"height:50px\">W</div>Q<br>R<br>S<br>T{after}</div><div>X</div>"
        ))
    };
    let qrst = vec![
        ('Q', 1, 55.0),
        ('R', 1, 75.0),
        ('S', 2, 5.0),
        ('T', 2, 25.0),
    ];
    let with = |extra: &[(char, usize, f32)]| {
        let mut places = qrst.clone();
        places.extend_from_slice(extra);
        places
    };
    vec![
        (
            "the filled body in quirks mode",
            format!("{STYLE}{lines}"),
            qrst.clone(),
        ),
        (
            "the filled body in quirks mode, with a block after the lines",
            format!("{STYLE}{lines}<div>X</div>"),
            with(&[('X', 2, 45.0)]),
        ),
        (
            "a body with a min-height below its content",
            standards(&format!("<style>body{{min-height:50px}}</style>{lines}")),
            qrst.clone(),
        ),
        (
            "a body with a min-height above its content",
            standards(&format!(
                "<style>body{{min-height:150px}}</style>{lines}<div>X</div>"
            )),
            with(&[('X', 2, 45.0)]),
        ),
        (
            "a block with a min-height",
            wrapped("min-height:100px", ""),
            with(&[('X', 2, 45.0)]),
        ),
        (
            "a block with a min-height past the moved lines",
            wrapped("min-height:200px", ""),
            with(&[('X', 3, 5.0)]),
        ),
        (
            "a block with a max-height",
            wrapped("max-height:120px", "<div>U</div>"),
            with(&[('U', 2, 45.0), ('X', 2, 25.0)]),
        ),
        (
            "a block with a height the moved lines overflow",
            wrapped("height:120px", "<div>U</div>"),
            with(&[('U', 2, 45.0), ('X', 2, 25.0)]),
        ),
        (
            "a block with a height that clips the moved lines",
            wrapped("height:120px;overflow:hidden", "<div>U</div>"),
            with(&[('X', 2, 25.0)]),
        ),
        (
            "two lines kept together by orphans and widows",
            standards(
                "<div style=\"min-height:10px\"><div style=\"height:80px\"></div>Q<br>R</div><div>X</div>",
            ),
            vec![('Q', 2, 5.0), ('R', 2, 25.0), ('X', 2, 45.0)],
        ),
        (
            "a block with size containment keeps the lines in place",
            wrapped("contain:size;height:120px", "<div>U</div>"),
            vec![
                ('Q', 1, 55.0),
                ('R', 1, 75.0),
                ('S', 2, -5.0),
                ('T', 2, 15.0),
                ('X', 2, 25.0),
            ],
        ),
    ]
}

#[test]
fn lines_after_a_block_child_break_like_chromium_whatever_sets_the_height() {
    let wrong: Vec<String> = cases()
        .into_iter()
        .filter_map(|(label, html, expected)| {
            let letters: String = expected.iter().map(|&(letter, ..)| letter).collect();
            let got = glyph_places(&html, &letters);
            got.iter()
                .zip(&expected)
                .any(|(got, expected)| got.1 != expected.1 || (got.2 - expected.2).abs() > 0.01)
                .then(|| format!("{label}: expected {expected:?}, got {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
