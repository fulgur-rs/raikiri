//! A box that avoids a break after it stays with the first row of the table
//! that follows it, inside wrappers as at the top of the body. A header group
//! that does not repeat on each page holds the table's first row; a repeated
//! one goes along with the first body row below it. Generated content of the
//! table sits in an anonymous row of its own, which is the first row when it
//! comes before the others. Where each glyph falls is what Chromium prints
//! for the same documents on pages 100px square, with Ahem glyphs of 20px on
//! lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}} td{{padding:0}} table{{border-spacing:0}} .b::before{{content:\"B\"}} .k::before{{content:\"B\";display:block}} .q::after{{content:\"Q\"}}</style><body>{body}</body>"
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

/// The page, glyph and glyph top of every word of text in `body`, but for
/// the glyphs in `skip`.
fn glyph_tops(body: &str, skip: &[char]) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for word in run.text.split_whitespace() {
                let glyph = word.chars().next().unwrap();
                if !skip.contains(&glyph) {
                    tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
                }
            }
        }
    }
    tops.sort_unstable();
    tops
}

/// A spacer of `height` with `Y`, then a heading `H` that avoids a break
/// after it and `table`, inside `open` and `close`, then `Z`.
fn document(height: u32, open: &str, table: &str, close: &str) -> String {
    format!(
        r#"<div style="height:{height}px">Y</div>{open}<div style="break-after:avoid">H</div>{table}{close}<div>Z</div>"#
    )
}

const WRAPPERS: [(&str, &str); 3] = [
    ("", ""),
    ("<div>", "</div>"),
    ("<div><div>", "</div></div>"),
];

/// Rows `R` and `S` after `head`, in a table of class `class`.
fn table(class: &str, head: &str) -> String {
    format!(r#"<table class="{class}">{head}<tr><td>R</td></tr><tr><td>S</td></tr></table>"#)
}

#[test]
fn a_header_that_does_not_repeat_is_the_first_row() {
    // A header row of 30px takes more than a quarter of the page, so it does
    // not repeat: the heading stays with it, and the body rows break away.
    let head = "<thead><tr><td>T</td></tr></thead>";
    for (open, close) in WRAPPERS {
        for height in [20, 30, 40] {
            let top = height as i32 + 5;
            assert_eq!(
                glyph_tops(&document(height, open, &table("", head), close), &[]),
                [
                    (0, 'H', top),
                    (0, 'T', top + 30),
                    (0, 'Y', 5),
                    (1, 'R', 5),
                    (1, 'S', 35),
                    (1, 'Z', 65)
                ],
                "{open} {height}"
            );
        }
        // The header row crossing the page end takes the heading along.
        assert_eq!(
            glyph_tops(&document(50, open, &table("", head), close), &[]),
            [
                (0, 'Y', 5),
                (1, 'H', 5),
                (1, 'R', 65),
                (1, 'T', 35),
                (2, 'S', 5),
                (2, 'Z', 35)
            ],
            "{open}"
        );
    }
}

#[test]
fn a_repeated_header_goes_with_the_first_body_row() {
    let head = r#"<thead><tr><td style="line-height:20px">T</td></tr></thead>"#;
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(50, open, &table("", head), close), &[]),
            [
                (0, 'Y', 5),
                (1, 'H', 5),
                (1, 'R', 55),
                (1, 'T', 30),
                (2, 'S', 25),
                (2, 'T', 0),
                (2, 'Z', 55)
            ],
            "{open}"
        );
    }
}

#[test]
fn a_footer_that_does_not_repeat_stays_below_the_rows() {
    let foot = "<tfoot><tr><td>F</td></tr></tfoot>";
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(20, open, &table("", foot), close), &[]),
            [
                (0, 'H', 25),
                (0, 'R', 55),
                (0, 'Y', 5),
                (1, 'F', 35),
                (1, 'S', 5),
                (1, 'Z', 65)
            ],
            "{open}"
        );
    }
}

#[test]
fn generated_content_before_the_rows_is_the_first_row() {
    for class in ["b", "k"] {
        for (open, close) in WRAPPERS {
            for height in [50, 60, 70] {
                assert_eq!(
                    glyph_tops(&document(height, open, &table(class, ""), close), &[]),
                    [
                        (0, 'Y', 5),
                        (1, 'B', 35),
                        (1, 'H', 5),
                        (1, 'R', 65),
                        (2, 'S', 5),
                        (2, 'Z', 35)
                    ],
                    "{class} {open} {height}"
                );
            }
        }
    }
    // A table of `display` values, its first row in a group that avoids
    // breaks inside it, and one with borders.
    let boxes = r#"<div class="b" style="display:table"><div style="display:table-row"><div style="display:table-cell">R</div></div><div style="display:table-row"><div style="display:table-cell">S</div></div></div>"#;
    let group = r#"<table class="b"><tbody style="break-inside:avoid"><tr><td>R</td></tr><tr><td>S</td></tr></tbody></table>"#;
    let bordered = r#"<table class="b" style="border:5px solid"><tr><td>R</td></tr><tr><td>S</td></tr></table>"#;
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(50, open, boxes, close), &[]),
            [
                (0, 'Y', 5),
                (1, 'B', 35),
                (1, 'H', 5),
                (1, 'R', 65),
                (2, 'S', 5),
                (2, 'Z', 35)
            ],
            "{open}"
        );
        assert_eq!(
            glyph_tops(&document(60, open, group, close), &[]),
            [
                (0, 'Y', 5),
                (1, 'B', 35),
                (1, 'H', 5),
                (2, 'R', 5),
                (2, 'S', 35),
                (2, 'Z', 65)
            ],
            "{open}"
        );
        assert_eq!(
            glyph_tops(&document(50, open, bordered, close), &[]),
            [
                (0, 'Y', 5),
                (1, 'B', 40),
                (1, 'H', 5),
                (1, 'R', 70),
                (2, 'S', 5),
                (2, 'Z', 40)
            ],
            "{open}"
        );
    }
}

#[test]
fn generated_content_after_the_rows_leaves_the_first_row_first() {
    // Where the generated row itself lands is left out.
    for (open, close) in WRAPPERS {
        for height in [50, 60, 70] {
            assert_eq!(
                glyph_tops(&document(height, open, &table("q", ""), close), &['Q', 'Z']),
                [(0, 'Y', 5), (1, 'H', 5), (1, 'R', 35), (1, 'S', 65)],
                "{open} {height}"
            );
        }
    }
}

const ROW: &str = r#"<div style="display:table-row"><div style="display:table-cell">R</div></div>"#;
const LAST_ROW: &str =
    r#"<div style="display:table-row"><div style="display:table-cell">S</div></div>"#;

#[test]
fn text_in_a_repeated_header_goes_with_the_first_body_row() {
    let table = format!(
        r#"<div style="display:table"><div style="display:table-header-group;break-inside:avoid;line-height:20px">T</div>{ROW}{LAST_ROW}</div>"#
    );
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(30, open, &table, close), &[]),
            [
                (0, 'Y', 5),
                (1, 'H', 5),
                (1, 'R', 55),
                (1, 'T', 30),
                (2, 'S', 25),
                (2, 'T', 0),
                (2, 'Z', 55)
            ],
            "{open}"
        );
    }
}

#[test]
fn text_in_a_group_that_avoids_breaks_holds_the_group() {
    let table = format!(
        r#"<div style="display:table"><div style="display:table-row-group;break-inside:avoid">A{ROW}</div>{LAST_ROW}</div>"#
    );
    for (open, close) in WRAPPERS {
        for height in [20, 40] {
            assert_eq!(
                glyph_tops(&document(height, open, &table, close), &[]),
                [
                    (0, 'Y', 5),
                    (1, 'A', 35),
                    (1, 'H', 5),
                    (1, 'R', 65),
                    (2, 'S', 5),
                    (2, 'Z', 35)
                ],
                "{open} {height}"
            );
        }
    }
}

#[test]
fn a_forced_break_in_a_group_that_avoids_breaks_ends_what_the_text_holds() {
    let group = |rows: &str| {
        format!(
            r#"<div style="display:table"><div style="display:table-row-group;break-inside:avoid">A{rows}</div><div style="display:table-row"><div style="display:table-cell">T</div></div></div>"#
        )
    };
    let row = |glyph: char, style: &str| {
        format!(
            r#"<div style="display:table-row;{style}"><div style="display:table-cell">{glyph}</div></div>"#
        )
    };
    // The break before the second row leaves the text with the first one.
    let first = group(&(row('R', "") + &row('S', "break-before:page")));
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(10, open, &first, close), &[]),
            [
                (0, 'A', 45),
                (0, 'H', 15),
                (0, 'R', 75),
                (0, 'Y', 5),
                (1, 'S', 5),
                (1, 'T', 35),
                (1, 'Z', 65)
            ],
            "{open}"
        );
        assert_eq!(
            glyph_tops(&document(20, open, &first, close), &[]),
            [
                (0, 'Y', 5),
                (1, 'A', 35),
                (1, 'H', 5),
                (1, 'R', 65),
                (2, 'S', 5),
                (2, 'T', 35),
                (2, 'Z', 65)
            ],
            "{open}"
        );
    }
    // A break before the first row leaves the text alone.
    let alone = group(&(row('R', "break-before:page") + &row('S', "")));
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(40, open, &alone, close), &[]),
            [
                (0, 'A', 75),
                (0, 'H', 45),
                (0, 'Y', 5),
                (1, 'R', 5),
                (1, 'S', 35),
                (1, 'T', 65),
                (2, 'Z', 5)
            ],
            "{open}"
        );
    }
}

#[test]
fn a_forced_break_at_a_block_of_the_text_ends_what_it_holds() {
    let group = |content: &str| {
        format!(
            r#"<div style="display:table"><div style="display:table-row-group;break-inside:avoid">{content}</div><div style="display:table-row"><div style="display:table-cell">T</div></div></div>"#
        )
    };
    let row = |glyph: char, style: &str| {
        format!(
            r#"<div style="display:table-row;{style}"><div style="display:table-cell">{glyph}</div></div>"#
        )
    };
    // After the last row, before text that follows the rows.
    let trailing = group(&format!("A{}B", row('R', "break-after:page")));
    // After a block of the text before the rows.
    let after = group(&format!(
        r#"<div style="break-after:page">A</div>{}{}"#,
        row('R', ""),
        row('S', "")
    ));
    // Before a block of the text before the rows.
    let before = group(&format!(
        r#"<div>A</div><div style="break-before:page">C</div>{}"#,
        row('R', "")
    ));
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(10, open, &trailing, close), &[]),
            [
                (0, 'A', 45),
                (0, 'H', 15),
                (0, 'R', 75),
                (0, 'Y', 5),
                (1, 'B', 5),
                (1, 'T', 35),
                (1, 'Z', 65)
            ],
            "{open}"
        );
        assert_eq!(
            glyph_tops(&document(20, open, &after, close), &[]),
            [
                (0, 'A', 55),
                (0, 'H', 25),
                (0, 'Y', 5),
                (1, 'R', 5),
                (1, 'S', 35),
                (1, 'T', 65),
                (2, 'Z', 5)
            ],
            "{open}"
        );
        assert_eq!(
            glyph_tops(&document(20, open, &before, close), &[]),
            [
                (0, 'A', 55),
                (0, 'H', 25),
                (0, 'Y', 5),
                (1, 'C', 5),
                (1, 'R', 35),
                (1, 'T', 65),
                (2, 'Z', 5)
            ],
            "{open}"
        );
    }
}

#[test]
fn a_forced_break_in_text_before_the_rows_ends_the_first_row() {
    let row = r#"<div style="display:table-row"><div style="display:table-cell">R</div></div>"#;
    let tables = [
        format!(
            r#"<div style="display:table"><div style="display:table-row-group"><div>A</div><div style="break-before:page">C</div>{row}</div></div>"#
        ),
        format!(
            r#"<div style="display:table"><div>A</div><div style="break-before:page">C</div>{row}</div>"#
        ),
        format!(
            r#"<div style="display:table"><div style="break-after:page">A</div><div>C</div>{row}</div>"#
        ),
    ];
    for table in &tables {
        for (open, close) in WRAPPERS {
            assert_eq!(
                glyph_tops(&document(20, open, table, close), &[]),
                [
                    (0, 'A', 55),
                    (0, 'H', 25),
                    (0, 'Y', 5),
                    (1, 'C', 5),
                    (1, 'R', 35),
                    (1, 'Z', 65)
                ],
                "{open} {table}"
            );
        }
    }
}

#[test]
fn generated_content_stays_tied_across_a_forced_break_before_the_rows() {
    let table = r#"<table class="b"><tr style="break-before:page"><td>R</td></tr><tr><td>S</td></tr></table>"#;
    for (open, close) in WRAPPERS {
        assert_eq!(
            glyph_tops(&document(50, open, table, close), &[]),
            [
                (0, 'Y', 5),
                (1, 'B', 35),
                (1, 'H', 5),
                (2, 'R', 5),
                (2, 'S', 35),
                (2, 'Z', 65)
            ],
            "{open}"
        );
    }
}
