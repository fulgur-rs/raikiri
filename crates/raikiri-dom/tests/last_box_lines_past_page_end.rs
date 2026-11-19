//! A block whose bottom padding or border does not fit the page breaks before
//! its last box, so that the box goes on with them to the next page. Chromium
//! does not when the lines of that box, or of a box inside it, run past the
//! bottom of the box and past the end of the page: the box breaks inside that
//! overflow and stays on its page, and the padding and border go on alone.
//! The same holds for the last box of a run that avoided breaks tie together.
//!
//! The expected places are the ones Chromium prints for the same markup:
//! 100px pages without margins, Ahem at 20px on 30px lines, and a block
//! 20px wide whose first child wraps four words onto two pages. Each is the
//! page (from 1) and the top of the glyph on it.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page and glyph top of the first run of `U` and of `Z` in `body`.
fn places(body: &str) -> [(usize, f32); 2] {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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
    ["U", "Z"].map(|letter| {
        result
            .pages()
            .enumerate()
            .find_map(|(index, page)| {
                page.text_runs()
                    .iter()
                    .find(|run| run.text.trim() == letter)
                    .map(|run| (index + 1, run.origin.1 - run.ascent))
            })
            .unwrap_or_else(|| panic!("no run for {letter} in {body}"))
    })
}

/// A block with the style `style` that holds wrapped words, `before`, then
/// `last`, followed by `Z`.
fn block(style: &str, before: &str, last: &str) -> String {
    format!("<div style=\"width:20px;{style}\"><div>X X X X</div>{before}{last}</div><div>Z</div>")
}

/// A label, the markup, and the places of `U` and `Z` it expects.
type Case = (&'static str, String, [(usize, f32); 2]);

const T: &str = "<div style=\"height:20px\">T</div>";

#[test]
fn a_last_box_whose_lines_run_past_the_page_end_stays_on_its_page() {
    let cases: Vec<Case> = vec![
        (
            "a last box whose line runs past it and the page end",
            block(
                "padding-bottom:15px",
                T,
                "<div style=\"height:10px\">U</div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "the same under a bottom border",
            block(
                "border-bottom:15px solid",
                T,
                "<div style=\"height:10px\">U</div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "a last box with padding of its own",
            block(
                "padding-bottom:15px",
                T,
                "<div style=\"height:10px;padding-bottom:5px\">U</div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "a last box inside display: contents",
            block(
                "padding-bottom:15px",
                T,
                "<div style=\"display:contents\"><div style=\"height:10px\">U</div></div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "a last box whose line runs past a box inside it",
            block(
                "padding-bottom:15px",
                T,
                "<div><div style=\"height:10px\">U</div></div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "the last box of a run that an avoided break ties",
            block(
                "padding-bottom:15px",
                "<div style=\"height:20px;break-after:avoid\">T</div>",
                "<div style=\"height:10px\">U</div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "a block whose content reaches its min-height",
            block(
                "min-height:150px;padding-bottom:15px",
                T,
                "<div style=\"height:10px\">U</div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "a block whose min-height leaves room below the content",
            block(
                "min-height:160px;padding-bottom:15px",
                T,
                "<div style=\"height:10px\">U</div>",
            ),
            [(2, 85.0), (3, 20.0)],
        ),
        (
            "a last box whose line ends inside it moves with the padding",
            block(
                "padding-bottom:15px",
                T,
                "<div style=\"height:10px;line-height:10px;font-size:10px\">U</div>",
            ),
            [(3, 0.0), (3, 30.0)],
        ),
        (
            "a last box whose line ends on its page moves with the padding",
            block("padding-bottom:15px", T, "<div>U</div>"),
            [(3, 5.0), (3, 50.0)],
        ),
        (
            "a last box that clips its line moves with the padding",
            block(
                "padding-bottom:15px",
                T,
                "<div style=\"height:10px;overflow:hidden\">U</div>",
            ),
            [(3, 5.0), (3, 30.0)],
        ),
        (
            "a last box that clips the box inside it moves with the padding",
            block(
                "padding-bottom:15px",
                T,
                "<div style=\"overflow:hidden\"><div style=\"height:10px\">U</div></div>",
            ),
            [(3, 5.0), (3, 30.0)],
        ),
    ];
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(label, body, expected)| {
            let got = places(body);
            got.iter()
                .zip(expected)
                .any(|(got, expected)| got.0 != expected.0 || (got.1 - expected.1).abs() > 0.01)
                .then(|| format!("{label}: expected {expected:?}, got {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
