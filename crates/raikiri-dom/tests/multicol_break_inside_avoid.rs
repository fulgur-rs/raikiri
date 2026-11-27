//! Paragraphs with `break-inside: avoid` (or `avoid-column`) in the columns
//! of a multicol container go on whole to the next column when the rest of
//! their column is too short for them, and break between their lines only at
//! the top of a column, where no break before them is left (CSS Fragmentation
//! 3, 3.2 and 4.4). `break-inside: avoid-page` asks nothing of the breaks
//! between columns, even those that end a page.
//!
//! The expected places are the ones Chromium gives the same markup, on 200px
//! by 200px pages with no margin, or on the screen. Each line holds one word,
//! 20px of Ahem, so a paragraph of `n` lines is `20 * n` px tall.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Two 100px columns, and the classes that avoid breaks.
const CLASSES: &str = "<style>.mc { columns: 2; column-gap: 0 } p { margin: 0 } \
    .a { break-inside: avoid } .c { break-inside: avoid-column } \
    .g { break-inside: avoid-page }</style>";

/// [`CLASSES`] on a 200px by 200px page with no margin.
const PAGE: &str = "<style>@page { size: 200px 200px; margin: 0 } \
    .mc { columns: 2; column-gap: 0 } p { margin: 0 } \
    .a { break-inside: avoid } .c { break-inside: avoid-column } \
    .g { break-inside: avoid-page }</style>";

/// One piece of a paragraph: its page, from 0, its left and top on that
/// page, and its height. A piece that a column end cuts reaches that end, so
/// only the height of a whole paragraph is checked.
type Piece = (usize, f32, f32, Option<f32>);

/// A piece as laid out: its page, left, top and height.
type LaidPiece = (usize, f32, f32, f32);

/// The pieces of each element with an `id`.
type Pieces = Vec<(String, Vec<LaidPiece>)>;

/// A paragraph with the id `p{index}` and the class `class`, of `lines`
/// lines.
fn p(index: usize, lines: usize, class: &str) -> String {
    let words = vec!["aaaa"; lines].join(" ");
    format!("<p id=p{index} class='{class}'>{words}</p>")
}

/// The paragraphs `p0`, `p1`, ... of the given lines and classes.
fn paragraphs(spec: &[(usize, &str)]) -> String {
    spec.iter()
        .enumerate()
        .map(|(index, &(lines, class))| p(index, lines, class))
        .collect()
}

/// The pieces of each element with an `id`, in the order the pages show
/// them.
fn pieces_of(body: &str) -> Pieces {
    let html =
        format!("<!DOCTYPE html><style>body {{ margin: 0; font: 20px/20px Ahem }}</style>{body}");
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
    let mut pieces: Pieces = Vec::new();
    for (index, page) in result.pages().enumerate() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            let rect = fragment.rect();
            let piece = (index, rect.x, rect.y, rect.height);
            match pieces.iter_mut().find(|(seen, _)| seen == id) {
                Some((_, own)) => own.push(piece),
                None => pieces.push((id.to_owned(), vec![piece])),
            }
        }
    }
    pieces
}

/// Lays out `body` and compares the pieces of each listed box with the
/// expected ones.
fn check(label: &str, body: &str, expected: &[(&str, &[Piece])]) {
    let pieces = pieces_of(body);
    for &(id, expected) in expected {
        let (_, got) = pieces
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
        let places: Vec<_> = got.iter().map(|&(page, x, y, _)| (page, x, y)).collect();
        let want: Vec<_> = expected
            .iter()
            .map(|&(page, x, y, _)| (page, x, y))
            .collect();
        assert_eq!(places, want, "{label}: pieces of #{id}");
        for (&(.., height), &(.., want)) in got.iter().zip(expected) {
            if let Some(want) = want {
                assert_eq!(height, want, "{label}: height of #{id}");
            }
        }
    }
}

fn whole(page: usize, x: f32, y: f32, lines: usize) -> [Piece; 1] {
    [(page, x, y, Some(20.0 * lines as f32))]
}

#[test]
fn a_paragraph_that_avoids_breaks_starts_the_next_column_whole() {
    let body = format!("{PAGE}<div class=mc>{}</div>", paragraphs(&[(8, "a"); 10]));
    let pages: Vec<_> = (0..10)
        .map(|index| whole(index / 2, if index % 2 == 0 { 0.0 } else { 100.0 }, 0.0, 8))
        .collect();
    let ids: Vec<_> = (0..10).map(|index| format!("p{index}")).collect();
    let expected: Vec<(&str, &[Piece])> = ids
        .iter()
        .zip(&pages)
        .map(|(id, piece)| (id.as_str(), &piece[..]))
        .collect();
    check("eight lines each", &body, &expected);
}

#[test]
fn avoid_and_avoid_column_move_paragraphs_to_the_next_column_and_page() {
    for class in ["a", "c"] {
        let body = format!(
            "{PAGE}<div class=mc>{}</div>",
            paragraphs(&[
                (6, class),
                (5, class),
                (7, class),
                (3, class),
                (8, class),
                (4, class),
                (6, class),
            ])
        );
        check(
            class,
            &body,
            &[
                ("p0", &whole(0, 0.0, 0.0, 6)),
                ("p1", &whole(0, 100.0, 0.0, 5)),
                ("p2", &whole(1, 0.0, 0.0, 7)),
                ("p3", &whole(1, 0.0, 140.0, 3)),
                ("p4", &whole(1, 100.0, 0.0, 8)),
                // The last page balances its columns at the tallest
                // paragraph that avoids breaks.
                ("p5", &whole(2, 0.0, 0.0, 4)),
                ("p6", &whole(2, 100.0, 0.0, 6)),
            ],
        );
    }
}

#[test]
fn avoid_page_leaves_the_breaks_between_columns_free() {
    let spec = [
        (6, "g"),
        (5, "g"),
        (7, "g"),
        (3, "g"),
        (8, "g"),
        (4, "g"),
        (6, "g"),
    ];
    let body = format!("{PAGE}<div class=mc>{}</div>", paragraphs(&spec));
    check(
        "avoid-page",
        &body,
        &[
            ("p0", &whole(0, 0.0, 0.0, 6)),
            ("p1", &[(0, 0.0, 120.0, None), (0, 100.0, 0.0, None)]),
            ("p2", &whole(0, 100.0, 40.0, 7)),
            // Orphans and widows of 2 leave no break inside three lines.
            ("p3", &whole(1, 0.0, 0.0, 3)),
            ("p4", &[(1, 0.0, 60.0, None), (1, 100.0, 0.0, None)]),
            ("p5", &whole(1, 100.0, 40.0, 4)),
            ("p6", &[(1, 100.0, 120.0, None), (2, 0.0, 0.0, None)]),
        ],
    );
    // On the screen too.
    let body = format!(
        "{CLASSES}<div class=mc style='width: 200px'>{}</div>",
        paragraphs(&[(3, "g"), (5, "g"), (2, "g")])
    );
    check(
        "avoid-page on the screen",
        &body,
        &[
            ("p0", &whole(0, 0.0, 0.0, 3)),
            ("p1", &[(0, 0.0, 60.0, None), (0, 100.0, 0.0, None)]),
            ("p2", &whole(0, 100.0, 60.0, 2)),
        ],
    );
}

#[test]
fn a_paragraph_taller_than_a_column_moves_once_then_breaks() {
    let body = format!(
        "{PAGE}<div class=mc>{}</div>",
        paragraphs(&[(4, "a"), (13, "a"), (6, "a")])
    );
    check(
        "thirteen lines",
        &body,
        &[
            ("p0", &whole(0, 0.0, 0.0, 4)),
            ("p1", &[(0, 100.0, 0.0, None), (1, 0.0, 0.0, None)]),
            ("p2", &whole(1, 100.0, 0.0, 6)),
        ],
    );
    // Orphans and widows still shape the break of such a paragraph.
    let body = format!(
        "{PAGE}<style>p {{ orphans: 3; widows: 3 }}</style><div class=mc>{}</div>",
        paragraphs(&[(8, ""), (11, "a"), (7, ""), (9, "a")])
    );
    check(
        "orphans and widows",
        &body,
        &[
            ("p0", &whole(0, 0.0, 0.0, 8)),
            ("p1", &[(0, 100.0, 0.0, None), (1, 0.0, 0.0, None)]),
            ("p2", &whole(1, 0.0, 60.0, 7)),
            ("p3", &whole(1, 100.0, 0.0, 9)),
        ],
    );
}

#[test]
fn paragraphs_that_avoid_breaks_mix_with_ones_that_break() {
    let body = format!(
        "{PAGE}<div class=mc>{}</div>",
        paragraphs(&[(7, ""), (6, ""), (5, "a"), (9, "")])
    );
    check(
        "mixed",
        &body,
        &[
            ("p0", &whole(0, 0.0, 0.0, 7)),
            ("p1", &[(0, 0.0, 140.0, None), (0, 100.0, 0.0, None)]),
            ("p2", &whole(0, 100.0, 60.0, 5)),
            (
                "p3",
                &[
                    (0, 100.0, 160.0, None),
                    (1, 0.0, 0.0, None),
                    (1, 100.0, 0.0, None),
                ],
            ),
        ],
    );
    let body = format!(
        "{PAGE}<div class=mc style='columns: 3'>{}</div>",
        paragraphs(&[(3, ""), (9, "a"), (2, ""), (9, "a"), (5, "")])
    )
    .replace("200px 200px", "300px 200px");
    check(
        "three columns",
        &body,
        &[
            ("p0", &whole(0, 0.0, 0.0, 3)),
            ("p1", &whole(0, 100.0, 0.0, 9)),
            ("p2", &whole(0, 200.0, 0.0, 2)),
            ("p3", &whole(1, 0.0, 0.0, 9)),
            ("p4", &whole(1, 100.0, 0.0, 5)),
        ],
    );
}

#[test]
fn margins_wrappers_and_column_fill_keep_paragraphs_whole() {
    let spec = [(6, ""), (5, "a"), (7, "a"), (4, "")];
    let expected: [(&str, &[Piece]); 4] = [
        ("p0", &whole(0, 0.0, 0.0, 6)),
        ("p1", &whole(0, 100.0, 0.0, 5)),
        ("p2", &whole(1, 0.0, 0.0, 7)),
        ("p3", &whole(1, 100.0, 0.0, 4)),
    ];
    let wrapped = format!(
        "{PAGE}<div class=mc><div>{}{}</div><div>{}{}</div></div>",
        p(0, 6, ""),
        p(1, 5, "a"),
        p(2, 7, "a"),
        p(3, 4, "")
    );
    check("wrappers", &wrapped, &expected);
    let margins = format!(
        "{PAGE}<style>p {{ margin: 10px 0 }}</style><div class=mc>{}</div>",
        paragraphs(&spec)
    );
    check(
        "margins",
        &margins,
        &[
            ("p0", &whole(0, 0.0, 10.0, 6)),
            ("p1", &whole(0, 100.0, 0.0, 5)),
            ("p2", &whole(1, 0.0, 0.0, 7)),
            ("p3", &whole(1, 100.0, 0.0, 4)),
        ],
    );
    let bottom_margins = format!(
        "{PAGE}<style>p {{ margin-bottom: 20px }}</style><div class=mc>{}</div>",
        paragraphs(&[
            (6, "a"),
            (5, "a"),
            (7, "a"),
            (3, "a"),
            (8, "a"),
            (4, "a"),
            (6, "a"),
        ])
    );
    check(
        "bottom margins",
        &bottom_margins,
        &[
            ("p0", &whole(0, 0.0, 0.0, 6)),
            ("p1", &whole(0, 100.0, 0.0, 5)),
            ("p2", &whole(1, 0.0, 0.0, 7)),
            ("p3", &whole(1, 100.0, 0.0, 3)),
            ("p4", &whole(2, 0.0, 0.0, 8)),
            ("p5", &whole(2, 100.0, 0.0, 4)),
            ("p6", &whole(3, 0.0, 0.0, 6)),
        ],
    );
    let filled = format!(
        "{PAGE}<div class=mc style='column-fill: auto'>{}</div>",
        paragraphs(&spec)
    );
    check(
        "column-fill: auto",
        &filled,
        &[
            ("p0", &whole(0, 0.0, 0.0, 6)),
            ("p1", &whole(0, 100.0, 0.0, 5)),
            ("p2", &whole(1, 0.0, 0.0, 7)),
            ("p3", &[(1, 0.0, 140.0, None), (1, 100.0, 0.0, None)]),
        ],
    );
}

#[test]
fn columns_below_other_content_go_on_at_the_next_page_before_a_cut() {
    // The first paragraph avoids breaks and does not fit below the box
    // before the columns: the columns start at the next page.
    let late = format!(
        "{PAGE}<div style='height: 120px'></div><div class=mc>{}</div>",
        paragraphs(&[(6, "a"), (4, ""), (9, "a")])
    );
    check(
        "late",
        &late,
        &[
            ("p0", &whole(1, 0.0, 0.0, 6)),
            ("p1", &whole(1, 0.0, 120.0, 4)),
            ("p2", &whole(1, 100.0, 0.0, 9)),
        ],
    );
    // One after another paragraph goes to the next column of the page.
    let later = format!(
        "{PAGE}<div style='height: 60px'></div><div class=mc>{}</div>",
        paragraphs(&[(2, ""), (6, "a"), (9, "a"), (3, "")])
    );
    check(
        "later",
        &later,
        &[
            ("p0", &whole(0, 0.0, 60.0, 2)),
            ("p1", &whole(0, 100.0, 60.0, 6)),
            ("p2", &whole(1, 0.0, 0.0, 9)),
            ("p3", &whole(1, 100.0, 0.0, 3)),
        ],
    );
}

#[test]
fn columns_with_nothing_before_them_on_the_page_break_inside() {
    // The container's padding or margin above the columns leaves no break
    // before them, so the paragraph breaks between its lines.
    for (style, wrapper) in [
        ("padding-top: 20px", false),
        ("margin-top: 20px", false),
        ("margin-top: 20px", true),
    ] {
        let columns = format!(
            "<div class=mc style='{style}'>{}</div>",
            paragraphs(&[(10, "a"), (4, "")])
        );
        // A `display: contents` box around the columns adds nothing before
        // them.
        let body = if wrapper {
            format!("{PAGE}<div style='display: contents'>{columns}</div>")
        } else {
            format!("{PAGE}{columns}")
        };
        check(
            style,
            &body,
            &[
                ("p0", &[(0, 0.0, 20.0, None), (0, 100.0, 20.0, None)]),
                ("p1", &whole(0, 100.0, 60.0, 4)),
            ],
        );
    }
    // Text before the columns leaves a break before them; an absolutely
    // positioned box leaves none.
    let text = format!(
        "{PAGE}<div>tttt<div class=mc>{}</div></div>",
        paragraphs(&[(10, "a"), (4, "")])
    );
    check(
        "after text",
        &text,
        &[
            ("p0", &whole(1, 0.0, 0.0, 10)),
            ("p1", &whole(1, 100.0, 0.0, 4)),
        ],
    );
    let positioned = format!(
        "{PAGE}<div style='position: absolute; height: 20px; width: 5px'></div>\
         <div class=mc>{}</div>",
        paragraphs(&[(10, "a"), (4, "")])
    );
    check(
        "after a positioned box",
        &positioned,
        &[
            ("p0", &whole(0, 0.0, 0.0, 10)),
            ("p1", &whole(0, 100.0, 0.0, 4)),
        ],
    );
    let after = format!(
        "{PAGE}<div style='height: 20px'></div><div class=mc>{}</div>",
        paragraphs(&[(10, "a"), (4, "")])
    );
    check(
        "after a box",
        &after,
        &[
            ("p0", &whole(1, 0.0, 0.0, 10)),
            ("p1", &whole(1, 100.0, 0.0, 4)),
        ],
    );
}

#[test]
fn balanced_screen_columns_grow_to_hold_paragraphs_that_avoid_breaks() {
    let screen = |spec: &[(usize, &str)], style: &str| {
        format!(
            "{CLASSES}<div id=o class=mc style='width: 200px; {style}'>{}</div>",
            paragraphs(spec)
        )
    };
    for class in ["a", "c"] {
        check(
            class,
            &screen(&[(3, class), (5, class), (2, class)], ""),
            &[
                ("o", &[(0, 0.0, 0.0, Some(140.0))]),
                ("p0", &whole(0, 0.0, 0.0, 3)),
                ("p1", &whole(0, 100.0, 0.0, 5)),
                ("p2", &whole(0, 100.0, 100.0, 2)),
            ],
        );
    }
    check(
        "mixed",
        &screen(&[(4, ""), (5, "a"), (3, "")], ""),
        &[
            ("o", &[(0, 0.0, 0.0, Some(160.0))]),
            ("p0", &whole(0, 0.0, 0.0, 4)),
            ("p1", &whole(0, 100.0, 0.0, 5)),
            ("p2", &whole(0, 100.0, 100.0, 3)),
        ],
    );
    check(
        "many short",
        &screen(
            &[
                (2, "a"),
                (2, "a"),
                (2, "a"),
                (2, "a"),
                (2, "a"),
                (2, "a"),
                (7, "a"),
            ],
            "",
        ),
        &[
            ("o", &[(0, 0.0, 0.0, Some(200.0))]),
            ("p4", &whole(0, 0.0, 160.0, 2)),
            ("p5", &whole(0, 100.0, 0.0, 2)),
            ("p6", &whole(0, 100.0, 40.0, 7)),
        ],
    );
    // The margins above the first paragraph count, through an empty box too.
    check(
        "top margin",
        &screen(&[(6, "a"), (2, "")], "").replace("p { margin: 0 }", "p { margin: 10px 0 0 }"),
        &[
            ("o", &[(0, 0.0, 0.0, Some(130.0))]),
            ("p0", &whole(0, 0.0, 10.0, 6)),
            ("p1", &whole(0, 100.0, 0.0, 2)),
        ],
    );
    check(
        "empty box",
        &format!(
            "{CLASSES}<div id=o class=mc style='width: 200px'>\
             <div style='margin-bottom: 30px'></div>{}</div>",
            paragraphs(&[(6, "a"), (2, "")])
        ),
        &[
            ("o", &[(0, 0.0, 0.0, Some(150.0))]),
            ("p0", &whole(0, 0.0, 30.0, 6)),
            ("p1", &whole(0, 100.0, 0.0, 2)),
        ],
    );
    // Columns of a definite height go on in overflow columns.
    check(
        "definite height",
        &screen(
            &[(3, "a"), (4, "a"), (2, "a")],
            "height: 100px; column-fill: auto",
        ),
        &[
            ("p0", &whole(0, 0.0, 0.0, 3)),
            ("p1", &whole(0, 100.0, 0.0, 4)),
            ("p2", &whole(0, 200.0, 0.0, 2)),
        ],
    );
    check(
        "taller than the columns",
        &screen(&[(2, "a"), (7, "a")], "height: 100px"),
        &[
            ("p0", &whole(0, 0.0, 0.0, 2)),
            ("p1", &[(0, 100.0, 0.0, None), (0, 200.0, 0.0, None)]),
        ],
    );
}
