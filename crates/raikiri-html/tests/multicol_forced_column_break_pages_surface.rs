//! A forced column break inside a multicol container whose columns go on
//! across pages: the columns end at the break, and those of the last page
//! balance what follows it. Where each line falls is what Chromium prints for
//! the same documents on pages 200px square, in two columns 100px wide, with
//! Ahem lines of 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Words of five glyphs, one to a line of a column: the `first`th to the
/// `last`th letter of the alphabet.
fn w(first: u8, last: u8) -> String {
    (first..=last)
        .map(|index| char::from(b'A' + index - 1).to_string().repeat(5))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The columns of `body`: for each page and column left edge, the top of its
/// first line and the glyph of each line, top to bottom.
fn columns(body: &str) -> Vec<(u32, i32, i32, String)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}} body{{margin:0;font:20px/20px Ahem}} .mc{{columns:2;column-gap:0}} p{{margin:0}}</style><body>{body}</body>"
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
    let mut lines = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let top = (run.origin.1 - run.ascent).round() as i32;
            if let Some(glyph) = run.text.chars().find(|glyph| !glyph.is_whitespace()) {
                lines.push((index, run.origin.0.round() as i32, top, glyph));
            }
        }
    }
    lines.sort_unstable();
    let mut columns: Vec<(u32, i32, i32, String)> = Vec::new();
    for (page, x, top, glyph) in lines {
        match columns.last_mut() {
            Some(column) if (column.0, column.1) == (page, x) => {
                assert_eq!(column.2 + 20 * column.3.len() as i32, top, "{body}");
                column.3.push(glyph);
            }
            _ => columns.push((page, x, top, glyph.to_string())),
        }
    }
    columns
}

fn assert_columns(body: &str, expected: &[(u32, i32, i32, &str)]) {
    let expected: Vec<_> = expected
        .iter()
        .map(|&(page, x, top, glyphs)| (page, x, top, glyphs.to_string()))
        .collect();
    assert_eq!(columns(body), expected, "{body}");
}

#[test]
fn a_forced_column_break_ends_a_column_and_the_last_page_balances() {
    // A break before a paragraph ends the first column, and the columns of the
    // last page balance what follows it.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><p style=break-before:column>{}</p><p>{}</p></div>",
            w(1, 5),
            w(6, 15),
            w(16, 20)
        ),
        &[
            (0, 0, 0, "ABCDE"),
            (0, 100, 0, "FGHIJKLMNO"),
            (1, 0, 0, "PQR"),
            (1, 100, 0, "ST"),
        ],
    );
    // A break after a paragraph does the same.
    assert_columns(
        &format!(
            "<div class=mc><p style=break-after:column>{}</p><p>{}</p><p>{}</p></div>",
            w(1, 5),
            w(6, 15),
            w(16, 20)
        ),
        &[
            (0, 0, 0, "ABCDE"),
            (0, 100, 0, "FGHIJKLMNO"),
            (1, 0, 0, "PQR"),
            (1, 100, 0, "ST"),
        ],
    );
    // A break on a later page starts the next column there.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><p style=break-before:column>{}</p><p style=break-before:column>{}</p></div>",
            w(1, 3),
            w(4, 16),
            w(17, 20)
        ),
        &[
            (0, 0, 0, "ABC"),
            (0, 100, 0, "DEFGHIJKLM"),
            (1, 0, 0, "NOP"),
            (1, 100, 0, "QRST"),
        ],
    );
    // A break at the end of the last column of a page moves what follows to the
    // next page, which balances it.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><p style=break-before:column>{}</p></div>",
            w(1, 12),
            w(13, 16)
        ),
        &[
            (0, 0, 0, "ABCDEFGHIJ"),
            (0, 100, 0, "KL"),
            (1, 0, 0, "MN"),
            (1, 100, 0, "OP"),
        ],
    );
    // The margin after a forced break stays at the top of the next page.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><p style=break-before:column;margin-top:20px>{}</p></div>",
            w(1, 12),
            w(13, 16)
        ),
        &[
            (0, 0, 0, "ABCDEFGHIJ"),
            (0, 100, 0, "KL"),
            (1, 0, 20, "MN"),
            (1, 100, 0, "OP"),
        ],
    );
    // A box with no lines forces the break as a paragraph does.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><div style=break-before:column></div><p>{}</p></div>",
            w(1, 3),
            w(4, 16)
        ),
        &[
            (0, 0, 0, "ABC"),
            (0, 100, 0, "DEFGHIJKLM"),
            (1, 0, 0, "NO"),
            (1, 100, 0, "P"),
        ],
    );
    // So does a box holding paragraphs, by its own break.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><div style=break-before:column><p>{}</p><p>{}</p></div></div>",
            w(1, 3),
            w(4, 10),
            w(11, 16)
        ),
        &[
            (0, 0, 0, "ABC"),
            (0, 100, 0, "DEFGHIJKLM"),
            (1, 0, 0, "NO"),
            (1, 100, 0, "P"),
        ],
    );
    // A break between the paragraphs of a box holding them.
    assert_columns(
        &format!(
            "<div class=mc><div><p>{}</p><p style=break-before:column>{}</p></div></div>",
            w(1, 8),
            w(9, 20)
        ),
        &[
            (0, 0, 0, "ABCDEFGH"),
            (0, 100, 0, "IJKLMNOPQR"),
            (1, 0, 0, "ST"),
        ],
    );
    // A break after the last paragraph of a box comes after the box.
    assert_columns(
        &format!(
            "<div class=mc><div><p>{}</p><p style=break-after:column>{}</p></div><p>{}</p></div>",
            w(1, 3),
            w(4, 5),
            w(6, 18)
        ),
        &[
            (0, 0, 0, "ABCDE"),
            (0, 100, 0, "FGHIJKLMNO"),
            (1, 0, 0, "PQ"),
            (1, 100, 0, "R"),
        ],
    );
    // A break before the first paragraph has nothing to end.
    assert_columns(
        &format!(
            "<div class=mc><p style=break-before:column>{}</p><p>{}</p></div>",
            w(1, 3),
            w(4, 16)
        ),
        &[(0, 0, 0, "ABCDEFGH"), (0, 100, 0, "IJKLMNOP")],
    );
    // White space between the boxes keeps a break after the first.
    assert_columns(
        &format!(
            "<div class=mc> <p style=break-after:column>{}</p> <p>{}</p> </div>",
            w(1, 3),
            w(4, 16)
        ),
        &[
            (0, 0, 0, "ABC"),
            (0, 100, 0, "DEFGHIJKLM"),
            (1, 0, 0, "NO"),
            (1, 100, 0, "P"),
        ],
    );
    // A break before the first box of a wrapper, after white space, comes
    // before the wrapper, whose margin stays at the top of the next column.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><div style=margin-top:30px> <p style=break-before:column>{}</p></div></div>",
            w(1, 3),
            w(4, 16)
        ),
        &[
            (0, 0, 0, "ABC"),
            (0, 100, 30, "DEFGHIJK"),
            (1, 0, 0, "LMN"),
            (1, 100, 0, "OP"),
        ],
    );
    // The margin of a wrapper that a forced break moves to the next page stays.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><div style=break-before:column;margin-top:20px><p>{}</p></div></div>",
            w(1, 12),
            w(13, 16)
        ),
        &[
            (0, 0, 0, "ABCDEFGHIJ"),
            (0, 100, 0, "KL"),
            (1, 0, 20, "MN"),
            (1, 100, 0, "OP"),
        ],
    );
    // An empty box between two breaks leaves a column of its own.
    assert_columns(
        &format!(
            "<div class=mc><p>{}</p><div style=break-before:column></div><p style=break-before:column>{}</p></div>",
            w(1, 3),
            w(4, 16)
        ),
        &[
            (0, 0, 0, "ABC"),
            (1, 0, 0, "DEFGHIJ"),
            (1, 100, 0, "KLMNOP"),
        ],
    );
}
