//! Where the lines of a multicol container that runs onto later pages stand.
//! The positions are the ones Chromium prints for the same documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::BTreeMap;

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;
/// The distance between the lines, which have a `line-height` of 20px.
const PITCH: i32 = 20;

/// Lines `from` through `to` of a paragraph, each labeled `prefix` and its
/// number.
fn par(prefix: &str, from: u32, to: u32) -> String {
    let lines: Vec<_> = (from..=to).map(|n| format!("{prefix}{n}")).collect();
    format!("<p>{}</p>", lines.join("<br>"))
}

/// A multicol container of two columns around `inner`.
fn mc(inner: &str) -> String {
    format!("<div class=mc>{inner}</div>")
}

/// Twelve paragraphs of fourteen lines each.
fn articles() -> String {
    "ABCDEFGHIJKL"
        .chars()
        .map(|c| par(&c.to_string(), 1, 14))
        .collect()
}

/// The pages of `body` with `css`, in Ahem on 200px pages.
fn pages(body: &str, css: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}\
         body{{font:20px/20px Ahem;margin:0}}.mc{{columns:2;column-gap:0}}p{{margin:0}}\
         {css}</style><body>{body}</body>"
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

/// The page, left edge, top and text of every line the pages report.
fn lines(body: &str, css: &str) -> Vec<(u32, i32, i32, String)> {
    let result = pages(body, css);
    let mut lines = Vec::new();
    for index in 0..12 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            lines.push((
                index,
                run.origin.0.round() as i32,
                (run.origin.1 - ASCENT).round() as i32,
                run.text.trim_end().to_owned(),
            ));
        }
    }
    lines
}

/// The lines of every column of every page, as `p<page> x<left>: <runs>`.
/// A run is the first and last label of lines one after the other, and the
/// top of the first, as in `L1-L10@0`.
fn columns(body: &str, css: &str) -> Vec<String> {
    let mut cells: BTreeMap<(u32, i32), Vec<(i32, String)>> = BTreeMap::new();
    for (page, left, top, text) in lines(body, css) {
        cells.entry((page, left)).or_default().push((top, text));
    }
    cells
        .into_iter()
        .map(|((page, left), mut lines)| {
            lines.sort();
            let mut runs: Vec<Vec<(i32, String)>> = Vec::new();
            for line in lines {
                match runs.last_mut() {
                    Some(run) if run.last().is_some_and(|last| line.0 - last.0 == PITCH) => {
                        run.push(line)
                    }
                    _ => runs.push(vec![line]),
                }
            }
            let runs: Vec<_> = runs
                .iter()
                .map(|run| {
                    let (top, first) = &run[0];
                    match run.len() {
                        1 => format!("{first}@{top}"),
                        _ => format!("{first}-{}@{top}", run[run.len() - 1].1),
                    }
                })
                .collect();
            format!("p{page} x{left}: {}", runs.join(" "))
        })
        .collect()
}

#[test]
fn one_paragraph_fills_each_page_before_the_next_one() {
    // Chromium fills the columns of every page in order and balances the
    // last page only.
    assert_eq!(
        columns(&mc(&par("L", 1, 60)), ""),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
            "p2 x0: L41-L50@0",
            "p2 x100: L51-L60@0",
        ],
        "one60"
    );
    assert_eq!(
        columns(&mc(&par("L", 1, 50)), ""),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
            "p2 x0: L41-L45@0",
            "p2 x100: L46-L50@0",
        ],
        "one50"
    );
    assert_eq!(
        columns(&mc(&par("L", 1, 30)), ""),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L25@0",
            "p1 x100: L26-L30@0",
        ],
        "one30"
    );
    assert_eq!(
        columns(&mc(&par("L", 1, 22)), ""),
        ["p0 x0: L1-L10@0", "p0 x100: L11-L20@0", "p1 x0: L21-L22@0",],
        "one22"
    );
    assert_eq!(
        columns(&mc(&par("L", 1, 21)), ""),
        ["p0 x0: L1-L10@0", "p0 x100: L11-L19@0", "p1 x0: L20-L21@0",],
        "one21"
    );
}

#[test]
fn columns_that_fill_in_sequence_leave_the_last_page_unbalanced() {
    assert_eq!(
        columns(&mc(&par("L", 1, 50)), ".mc{column-fill:auto}"),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
            "p2 x0: L41-L50@0",
        ],
        "fill50"
    );
}

#[test]
fn several_paragraphs_fill_the_pages_in_order() {
    assert_eq!(
        columns(
            &mc(&(0..6)
                .map(|i| par("L", 1 + 10 * i, 10 + 10 * i))
                .collect::<String>()),
            ""
        ),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
            "p2 x0: L41-L50@0",
            "p2 x100: L51-L60@0",
        ],
        "six"
    );
    assert_eq!(
        columns(&mc(&(par("L", 1, 30) + &par("L", 31, 60))), ""),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
            "p2 x0: L41-L50@0",
            "p2 x100: L51-L60@0",
        ],
        "two"
    );
    assert_eq!(
        columns(
            &mc(&(0..16)
                .map(|i| par(&format!("P{i}_"), 1, 5))
                .collect::<String>()),
            ""
        ),
        [
            "p0 x0: P0_1-P1_5@0",
            "p0 x100: P2_1-P3_5@0",
            "p1 x0: P4_1-P5_5@0",
            "p1 x100: P6_1-P7_5@0",
            "p2 x0: P8_1-P9_5@0",
            "p2 x100: P10_1-P11_5@0",
            "p3 x0: P12_1-P13_5@0",
            "p3 x100: P14_1-P15_5@0",
        ],
        "many"
    );
    assert_eq!(
        columns(
            &mc(&(0..4)
                .map(|i| par(&format!("P{i}_"), 1, 14))
                .collect::<String>()),
            ".mc{orphans:3;widows:3}"
        ),
        [
            "p0 x0: P0_1-P0_10@0",
            "p0 x100: P0_11-P1_6@0",
            "p1 x0: P1_7-P1_14@0",
            "p1 x100: P2_1-P2_10@0",
            "p2 x0: P2_11-P3_5@0",
            "p2 x100: P3_6-P3_14@0",
        ],
        "minima"
    );
}

#[test]
fn margins_between_paragraphs_do_not_reach_the_next_column_or_page() {
    // A margin at a break is dropped, so the paragraph after it starts at the top
    // of the column or page.
    assert_eq!(
        columns(
            &mc(&(0..6)
                .map(|i| par(&format!("P{i}_"), 1, 10))
                .collect::<String>()),
            "p{margin:10px 0}"
        ),
        [
            "p0 x0: P0_1-P0_8@10",
            "p0 x100: P0_9-P0_10@0 P1_1-P1_7@50",
            "p1 x0: P1_8-P1_10@0 P2_1-P2_6@70",
            "p1 x100: P2_7-P2_10@0 P3_1-P3_5@90",
            "p2 x0: P3_6-P3_10@0 P4_1-P4_4@110",
            "p2 x100: P4_5-P4_10@0 P5_1-P5_3@130",
            "p3 x0: P5_4-P5_7@0",
            "p3 x100: P5_8-P5_10@0",
        ],
        "between"
    );
    assert_eq!(
        columns(
            &mc(&(0..6)
                .map(|i| par(&format!("P{i}_"), 1, 10))
                .collect::<String>()),
            "p{margin:0 0 10px}"
        ),
        [
            "p0 x0: P0_1-P0_10@0",
            "p0 x100: P1_1-P1_10@0",
            "p1 x0: P2_1-P2_10@0",
            "p1 x100: P3_1-P3_10@0",
            "p2 x0: P4_1-P4_10@0",
            "p2 x100: P5_1-P5_10@0",
        ],
        "below"
    );
    assert_eq!(
        columns(&mc(&articles()), "p{margin:10px 0}"),
        [
            "p0 x0: A1-A9@10",
            "p0 x100: A10-A14@0 B1-B4@110",
            "p1 x0: B5-B14@0",
            "p1 x100: C1-C10@0",
            "p2 x0: C11-C14@0 D1-D5@90",
            "p2 x100: D6-D14@0",
            "p3 x0: E1-E10@0",
            "p3 x100: E11-E14@0 F1-F5@90",
            "p4 x0: F6-F14@0",
            "p4 x100: G1-G10@0",
            "p5 x0: G11-G14@0 H1-H5@90",
            "p5 x100: H6-H14@0",
            "p6 x0: I1-I10@0",
            "p6 x100: I11-I14@0 J1-J5@90",
            "p7 x0: J6-J14@0",
            "p7 x100: K1-K10@0",
            "p8 x0: K11-K14@0 L1-L5@90",
            "p8 x100: L6-L14@0",
        ],
        "article_top"
    );
    assert_eq!(
        columns(&mc(&articles()), "p{margin:0 0 10px}"),
        [
            "p0 x0: A1-A10@0",
            "p0 x100: A11-A14@0 B1-B5@90",
            "p1 x0: B6-B14@0",
            "p1 x100: C1-C10@0",
            "p2 x0: C11-C14@0 D1-D5@90",
            "p2 x100: D6-D14@0",
            "p3 x0: E1-E10@0",
            "p3 x100: E11-E14@0 F1-F5@90",
            "p4 x0: F6-F14@0",
            "p4 x100: G1-G10@0",
            "p5 x0: G11-G14@0 H1-H5@90",
            "p5 x100: H6-H14@0",
            "p6 x0: I1-I10@0",
            "p6 x100: I11-I14@0 J1-J5@90",
            "p7 x0: J6-J14@0",
            "p7 x100: K1-K10@0",
            "p8 x0: K11-K14@0 L1-L5@90",
            "p8 x100: L6-L14@0",
        ],
        "article"
    );
}

#[test]
fn what_follows_the_container_starts_below_the_last_column() {
    // The margin below the last paragraph is not part of the container.
    assert_eq!(
        columns(
            &format!("{}<p>FOOT</p>", mc(&(par("A", 1, 14) + &par("B", 1, 14)))),
            "p{margin:0 0 10px}"
        ),
        [
            "p0 x0: A1-A10@0",
            "p0 x100: A11-A14@0 B1-B5@90",
            "p1 x0: B6-FOOT@0",
            "p1 x100: B11-B14@0",
        ],
        "after_short"
    );
    assert_eq!(
        columns(
            &format!(
                "{}<p>FOOT</p>",
                mc(&"ABCDEF"
                    .chars()
                    .map(|c| par(&c.to_string(), 1, 10))
                    .collect::<String>())
            ),
            "p{margin:0 0 10px}"
        ),
        [
            "p0 x0: A1-A10@0",
            "p0 x100: B1-B10@0",
            "p1 x0: C1-C10@0",
            "p1 x100: D1-D10@0",
            "p2 x0: E1-E10@0",
            "p2 x100: F1-F10@0",
            "p3 x0: FOOT@0",
        ],
        "after_full"
    );
    assert_eq!(
        columns(
            &format!(
                "{}<p>FOOT</p>",
                mc(&"ABCDEFG"
                    .chars()
                    .map(|c| par(&c.to_string(), 1, 9))
                    .collect::<String>())
            ),
            "p{margin:10px 0}"
        ),
        [
            "p0 x0: A1-A9@10",
            "p0 x100: B1-B9@0",
            "p1 x0: C1-C9@0",
            "p1 x100: D1-D9@0",
            "p2 x0: E1-E9@0",
            "p2 x100: F1-F9@0",
            "p3 x0: G1-G5@0 FOOT@110",
            "p3 x100: G6-G9@0",
        ],
        "after_top"
    );
}

#[test]
fn headings_between_paragraphs_fill_the_pages_in_order() {
    assert_eq!(
        columns(
            &mc(&"ABCDEF"
                .chars()
                .map(|c| format!("<h2>H{c}</h2>{}", par(&c.to_string(), 1, 9)))
                .collect::<String>()),
            "h2{margin:10px 0;font-size:20px;font-weight:normal} p{margin:0 0 10px}"
        ),
        [
            "p0 x0: HA@10 A1-A7@40",
            "p0 x100: A8-A9@0 HB@50 B1-B6@80",
            "p1 x0: B7-B9@0 HC@70 C1-C5@100",
            "p1 x100: C6-C9@0 HD@90 D1-D4@120",
            "p2 x0: D5-D9@0 HE@110 E1-E3@140",
            "p2 x100: E4-E9@0 HF@130 F1-F2@160",
            "p3 x0: F3-F6@0",
            "p3 x100: F7-F9@0",
        ],
        "headings"
    );
}

#[test]
fn the_container_geometry_is_kept_on_every_page() {
    assert_eq!(
        columns(&mc(&par("L", 1, 90)), ".mc{columns:3}"),
        [
            "p0 x0: L1-L10@0",
            "p0 x67: L11-L20@0",
            "p0 x133: L21-L30@0",
            "p1 x0: L31-L40@0",
            "p1 x67: L41-L50@0",
            "p1 x133: L51-L60@0",
            "p2 x0: L61-L70@0",
            "p2 x67: L71-L80@0",
            "p2 x133: L81-L90@0",
        ],
        "three"
    );
    assert_eq!(
        columns(&mc(&par("L", 1, 40)), ".mc{column-gap:20px}"),
        [
            "p0 x0: L1-L10@0",
            "p0 x110: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x110: L31-L40@0",
        ],
        "gap"
    );
    assert_eq!(
        columns(&mc(&par("L", 1, 40)), ".mc{padding:10px;border:5px solid}"),
        [
            "p0 x15: L1-L9@15",
            "p0 x100: L10-L18@15",
            "p1 x15: L19-L28@0",
            "p1 x100: L29-L38@0",
            "p2 x15: L39-L40@0",
        ],
        "padded"
    );
}

#[test]
fn a_container_that_starts_below_other_content_fills_the_rest_of_its_page_first() {
    assert_eq!(
        columns(
            &format!("<div style='height:50px'>B</div>{}", mc(&par("L", 1, 60))),
            ""
        ),
        [
            "p0 x0: B@0 L1-L7@50",
            "p0 x100: L8-L14@50",
            "p1 x0: L15-L24@0",
            "p1 x100: L25-L34@0",
            "p2 x0: L35-L44@0",
            "p2 x100: L45-L54@0",
            "p3 x0: L55-L57@0",
            "p3 x100: L58-L60@0",
        ],
        "below_block"
    );
    assert_eq!(
        columns(
            &format!("<div style='height:150px'>B</div>{}", mc(&par("L", 1, 40))),
            ""
        ),
        [
            "p0 x0: B@0 L1-L2@150",
            "p0 x100: L3-L4@150",
            "p1 x0: L5-L14@0",
            "p1 x100: L15-L24@0",
            "p2 x0: L25-L32@0",
            "p2 x100: L33-L40@0",
        ],
        "late_start"
    );
}

#[test]
fn a_container_that_fits_its_page_stays_balanced() {
    assert_eq!(
        columns(&mc(&par("L", 1, 12)), ""),
        ["p0 x0: L1-L6@0", "p0 x100: L7-L12@0",],
        "single"
    );
}

#[test]
fn a_paragraph_that_is_shifted_keeps_its_runs_where_it_stays_whole() {
    assert_eq!(
        columns(
            &mc(&(0..6)
                .map(|i| par(&format!("P{i}_"), 1, 10))
                .collect::<String>()),
            "p{position:relative;top:3px}"
        ),
        [
            "p0 x0: P0_1-P0_10@3",
            "p0 x100: P1_1-P1_10@3",
            "p1 x0: P2_1-P2_10@3",
            "p1 x100: P3_1-P3_10@3",
            "p2 x0: P4_1-P4_10@3",
            "p2 x100: P5_1-P5_10@3",
        ],
        "shifted"
    );
}

#[test]
fn a_paragraph_that_is_cut_and_shifted_fills_each_page_in_column_order() {
    // The painter moves the clip of each piece with the paragraph's own
    // offset, so the pages report the runs of every piece where the
    // paragraph's own offset puts them.
    for (shift, x, y) in [
        ("p{position:relative;top:3px}", 0, 3),
        ("p{position:relative;top:-9px}", 0, -9),
        ("p{position:relative;left:7px}", 7, 0),
        ("p{transform:translate(5px,3px)}", 5, 3),
    ] {
        let right = 100 + x;
        assert_eq!(
            columns(&mc(&par("L", 1, 45)), shift),
            [
                format!("p0 x{x}: L1-L10@{y}"),
                format!("p0 x{right}: L11-L20@{y}"),
                format!("p1 x{x}: L21-L30@{y}"),
                format!("p1 x{right}: L31-L40@{y}"),
                format!("p2 x{x}: L41-L43@{y}"),
                format!("p2 x{right}: L44-L45@{y}"),
            ],
            "{shift}"
        );
    }
}

#[test]
fn a_paragraph_shifted_past_its_pages_stays_in_the_columns_the_pages_cut() {
    // An offset of half a line or more would report the edge lines of a
    // piece on another page than the one that paints them. Such a paragraph
    // stays in the columns that the pages cut, where every line is reported
    // once.
    for (shift, y) in [
        ("p{position:relative;top:-15px}", -15),
        ("p{position:relative;top:10px}", 10),
        ("p{transform:translate(0,10px)}", 10),
    ] {
        let lines = lines(&mc(&par("L", 1, 45)), shift);
        let mut texts: Vec<_> = lines.iter().map(|line| line.3.clone()).collect();
        texts.sort_by_key(|text| text[1..].parse::<u32>().unwrap());
        let expected: Vec<_> = (1..=45).map(|n| format!("L{n}")).collect();
        assert_eq!(texts, expected, "{shift}");
        let right = lines
            .iter()
            .filter(|line| line.0 == 0 && line.1 == 100)
            .min_by_key(|line| line.2)
            .unwrap();
        assert_eq!((right.2, right.3.as_str()), (y, "L24"), "{shift}");
    }
    // A transform that does more than translate, or translates by a
    // percentage, resolves against the box size the pieces of a filled page
    // do not keep. The runs of a transformed paragraph in columns are not
    // reported either way.
    for shift in ["p{transform:translate(0,10%)}", "p{transform:rotate(1deg)}"] {
        assert!(lines(&mc(&par("L", 1, 45)), shift).is_empty(), "{shift}");
    }
}

#[test]
fn a_container_the_pages_move_is_cut_where_it_lands() {
    // Whatever moves the container before its columns are filled leaves them as
    // one tall set that the pages cut, and Chromium prints it that way when it
    // moves the whole container.
    assert_eq!(
        columns(
            &format!("<div style='height:100px'>B</div>{}", mc(&par("L", 1, 15))),
            ".mc{break-inside:avoid}"
        ),
        ["p0 x0: B@0", "p1 x0: L1-L8@0", "p1 x100: L9-L15@0",],
        "avoid"
    );
    assert_eq!(
        columns(
            &format!(
                "<div style='height:150px'>X</div><div style='break-before:page;height:20px'>H</div>{}",
                mc(&par("L", 1, 12))
            ),
            ""
        ),
        ["p0 x0: X@0", "p1 x0: H-L6@0", "p1 x100: L7-L12@20",],
        "forced"
    );
    assert_eq!(
        columns(
            &format!(
                "<div style='height:150px'>X</div><div style='break-inside:avoid;height:60px'>H</div>{}",
                mc(&par("L", 1, 12))
            ),
            ""
        ),
        ["p0 x0: X@0", "p1 x0: H@0 L1-L6@60", "p1 x100: L7-L12@60",],
        "kept"
    );
}

#[test]
fn a_container_in_an_inline_block_is_cut_as_one_tall_box() {
    assert_eq!(
        columns(
            &format!("<div class=ib>{}</div>", mc(&par("L", 1, 50))),
            ".ib{display:inline-block;width:200px;vertical-align:top}"
        ),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L26-L35@0",
            "p1 x0: L11-L20@0",
            "p1 x100: L36-L45@0",
            "p2 x0: L21-L25@0",
            "p2 x100: L46-L50@0",
        ],
        "inline_block"
    );
}

/// Paragraphs `prefix`1 through `prefix``sizes[0]`, then the next prefix
/// letter for the next size, and so on, labeled from `L`.
fn pars(sizes: &[u32]) -> String {
    sizes
        .iter()
        .zip("LMN".chars())
        .map(|(&size, prefix)| par(&prefix.to_string(), 1, size))
        .collect()
}

/// Labels lines `L1`.. of `count` paragraphs of `size` lines each, numbered
/// on from one paragraph to the next.
fn numbered(count: u32, size: u32) -> String {
    (0..count)
        .map(|index| par("L", index * size + 1, (index + 1) * size))
        .collect()
}

#[test]
fn paragraphs_with_padding_and_border_fill_each_page_before_the_next_one() {
    const DECORATED: &str = "p{background:#ccc;border:2px solid blue;padding:2px}";
    // A piece that goes on in the next column ends at the column end, and the
    // next piece begins with its first line at the top of its column.
    assert_eq!(
        columns(&mc(&numbered(5, 8)), DECORATED),
        [
            "p0 x4: L1-L8@4",
            "p0 x104: L9-L16@4",
            "p1 x4: L17-L24@4",
            "p1 x104: L25-L32@4",
            "p2 x4: L33-L36@4",
            "p2 x104: L37-L40@0",
        ],
        "five"
    );
    assert_eq!(
        columns(&mc(&par("L", 1, 30)), DECORATED),
        [
            "p0 x4: L1-L9@4",
            "p0 x104: L10-L19@0",
            "p1 x4: L20-L25@0",
            "p1 x104: L26-L30@0",
        ],
        "one"
    );
    assert_eq!(
        columns(&mc(&numbered(5, 8)), "p{background:#ccc}"),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
        ],
        "background"
    );
    assert_eq!(
        columns(&mc(&pars(&[30, 12])), "p{padding:10px 0;background:#cfc}"),
        [
            "p0 x0: L1-L9@10",
            "p0 x100: L10-L19@0",
            "p1 x0: L20-L28@0",
            "p1 x100: L29-L30@0 M1-M7@60",
            "p2 x0: M8-M10@0",
            "p2 x100: M11-M12@0",
        ],
        "padding"
    );
    assert_eq!(
        columns(
            &mc(&pars(&[12, 25])),
            "p{border-top:4px solid red;border-bottom:6px solid blue}"
        ),
        [
            "p0 x0: L1-L9@4",
            "p0 x100: L10-L12@0 M1-M6@70",
            "p1 x0: M7-M16@0",
            "p1 x100: M17-M25@0",
        ],
        "borders"
    );
    assert_eq!(
        columns(
            &mc(&numbered(6, 7)),
            "p{margin:10px 0;background:#ccc;border:2px solid blue;padding:3px}"
        ),
        [
            "p0 x5: L1-L7@15",
            "p0 x105: L8-L14@5",
            "p1 x5: L15-L21@5",
            "p1 x105: L22-L28@5",
            "p2 x5: L29-L35@5",
            "p2 x105: L36-L42@5",
        ],
        "margins"
    );
    // The last line stays with a bottom border that a column cannot hold
    // after it.
    assert_eq!(
        columns(
            &mc(&pars(&[9, 20])),
            "p{background:#ccc;border-bottom:30px solid blue}"
        ),
        [
            "p0 x0: L1-L7@0",
            "p0 x100: L8-L9@0 M1-M6@70",
            "p1 x0: M7-M14@0",
            "p1 x100: M15-M20@0",
        ],
        "bottom_border"
    );
    assert_eq!(
        columns(
            &mc(&pars(&[6, 14, 20])),
            ".mc p:nth-child(2){background:#ccc;border:4px solid blue}"
        ),
        [
            "p0 x0: L1-L6@0",
            "p0 x4: M1-M3@124",
            "p0 x104: M4-M12@0",
            "p1 x0: N1-N7@44",
            "p1 x4: M13-M14@0",
            "p1 x100: N8-N17@0",
            "p2 x0: N18-N19@0",
            "p2 x100: N20@0",
        ],
        "mixed"
    );
}

#[test]
fn cloned_edges_sit_in_every_piece_of_a_paginated_paragraph() {
    assert_eq!(
        columns(
            &mc(&par("L", 1, 30)),
            "p{background:#ccc;border:3px solid blue;padding:2px;box-decoration-break:clone}"
        ),
        [
            "p0 x5: L1-L9@5",
            "p0 x105: L10-L18@5",
            "p1 x5: L19-L24@5",
            "p1 x105: L25-L30@5",
        ],
        "clone"
    );
    assert_eq!(
        columns(
            &mc(&par("L", 1, 23)),
            "p{background:#ccc;border:3px solid blue;padding:2px;\
             box-decoration-break:clone;widows:2;orphans:2}"
        ),
        [
            "p0 x5: L1-L9@5",
            "p0 x105: L10-L18@5",
            "p1 x5: L19-L21@5",
            "p1 x105: L22-L23@5",
        ],
        "clone_widows"
    );
}

#[test]
fn decorated_paragraphs_fill_columns_in_sequence_when_asked() {
    assert_eq!(
        columns(
            &mc(&pars(&[25, 5])),
            "p{background:#ccc;border:2px solid blue;padding:2px}.mc{column-fill:auto}"
        ),
        [
            "p0 x4: L1-L9@4",
            "p0 x104: L10-L19@0",
            "p1 x4: L20-L25@0 M1-M3@128",
            "p1 x104: M4-M5@0",
        ],
        "fill_auto"
    );
}

#[test]
fn the_last_page_of_decorated_paragraphs_balances_to_the_lines_a_break_keeps() {
    const DECORATED: &str = "p{background:#ccc;border:2px solid blue;padding:2px}";
    // Four lines left under `orphans: 3` keep three before the break, and
    // the widows give way.
    assert_eq!(
        columns(
            &mc(&pars(&[12, 30])),
            &format!("{DECORATED}p{{widows:3;orphans:3}}")
        ),
        [
            "p0 x4: L1-L9@4",
            "p0 x104: L10-L12@0 M1-M6@68",
            "p1 x4: M7-M16@0",
            "p1 x104: M17-M26@0",
            "p2 x4: M27-M29@0",
            "p2 x104: M30@0",
        ],
        "widows"
    );
    // Three lines left are as tall as the orphans, and the bottom edge that
    // the column cannot hold after them moves the last line on.
    assert_eq!(
        columns(
            &mc(&pars(&[12, 28])),
            &format!("{DECORATED}p{{widows:3;orphans:3}}")
        ),
        [
            "p0 x4: L1-L9@4",
            "p0 x104: L10-L12@0 M1-M6@68",
            "p1 x4: M7-M16@0",
            "p1 x104: M17-M25@0",
            "p2 x4: M26-M27@0",
            "p2 x104: M28@0",
        ],
        "three_left"
    );
    assert_eq!(
        columns(
            &mc(&pars(&[12, 29])),
            &format!("{DECORATED}p{{widows:1;orphans:5}}")
        ),
        [
            "p0 x4: L1-L9@4",
            "p0 x104: L10-L12@0 M1-M6@68",
            "p1 x4: M7-M16@0",
            "p1 x104: M17-M26@0",
            "p2 x4: M27-M28@0",
            "p2 x104: M29@0",
        ],
        "orphans"
    );
    assert_eq!(
        columns(
            &mc(&pars(&[5, 29])),
            &format!("{DECORATED}p{{widows:2;orphans:4}}")
        ),
        [
            "p0 x4: L1-L5@4 M1-M4@112",
            "p0 x104: M5-M14@0",
            "p1 x4: M15-M22@0",
            "p1 x104: M23-M29@0",
        ],
        "first_page"
    );
    assert_eq!(
        columns(
            &mc(&par("L", 1, 24)),
            "p{background:#ccc;border-bottom:30px solid blue;widows:2;orphans:2}"
        ),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L22@0",
            "p1 x100: L23-L24@0",
        ],
        "bottom_border"
    );
    // Plain paragraphs balance the same way.
    assert_eq!(
        columns(&mc(&pars(&[12, 28])), "p{widows:3;orphans:3}"),
        [
            "p0 x0: L1-L9@0",
            "p0 x100: L10-M7@0",
            "p1 x0: M8-M17@0",
            "p1 x100: M18-M25@0",
            "p2 x0: M26-M28@0",
        ],
        "plain"
    );
}

/// The page, left edge, top and bottom of what the clips around it leave
/// visible of each box of the element with id `id`.
fn visible_boxes(body: &str, css: &str, id: &str) -> Vec<(u32, i32, i32, i32)> {
    let result = pages(body, css);
    let mut boxes = Vec::new();
    for page in result.pages() {
        let dom = page.dom();
        let mut clips: Vec<(f32, f32)> = Vec::new();
        for event in page.paint_order() {
            match event {
                PaintEvent::PushClip(clip, _) => clips.push(if clip.clip_y {
                    (clip.rect.y, clip.rect.y + clip.rect.height)
                } else {
                    (f32::MIN, f32::MAX)
                }),
                PaintEvent::PopClip => {
                    clips.pop();
                }
                PaintEvent::Box(fragment) if dom.attr(fragment.node(), "id") == Some(id) => {
                    let rect = fragment.rect();
                    let (top, bottom) = clips
                        .iter()
                        .fold((rect.y, rect.y + rect.height), |(top, bottom), clip| {
                            (top.max(clip.0), bottom.min(clip.1))
                        });
                    if bottom > top {
                        boxes.push((
                            page.index(),
                            rect.x.round() as i32,
                            top.round() as i32,
                            bottom.round() as i32,
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    boxes
}

#[test]
fn a_bottom_border_that_goes_on_alone_shows_in_the_next_column_only() {
    // The line and the border below it do not fit one column together, and
    // no break comes before the line, so the columns break between them: the
    // border goes on alone at the top of the next column (Chromium).
    let body = mc(&format!("<p id=x>X1</p>{}", par("M", 1, 30)));
    let css = "p{background:#ccc}#x{border-bottom:190px solid blue}";
    assert_eq!(
        columns(&body, css),
        [
            "p0 x0: X1@0",
            "p1 x0: M1-M10@0",
            "p1 x100: M11-M20@0",
            "p2 x0: M21-M25@0",
            "p2 x100: M26-M30@0",
        ],
        "lines"
    );
    assert_eq!(
        visible_boxes(&body, css, "x"),
        [(0, 0, 0, 20), (0, 100, 0, 190)],
        "boxes"
    );
}

#[test]
fn the_last_page_columns_are_as_tall_as_the_orphans_a_break_keeps() {
    // Three lines left under `orphans: 3` make the columns three lines tall;
    // the bottom border cannot follow the third line there, so that line
    // goes on with it, and the piece before runs to the column end
    // (Chromium).
    let body = mc(&format!("{}<p id=m>{}</p>", par("L", 1, 12), {
        let lines: Vec<_> = (1..=28).map(|n| format!("M{n}")).collect();
        lines.join("<br>")
    }));
    let css = "p{background:#ccc;border:2px solid blue;padding:2px;widows:3;orphans:3}";
    let last: Vec<_> = visible_boxes(&body, css, "m")
        .into_iter()
        .filter(|piece| piece.0 == 2)
        .collect();
    assert_eq!(last, [(2, 0, 0, 60), (2, 100, 0, 24)]);
}

#[test]
fn wrappers_with_edges_or_paint_fill_each_page_in_column_order() {
    // Padding and border above or below the boxes of a wrapper, a wrapper
    // that paints, and paragraphs with padding or border inside a wrapper
    // fill the pages as plain paragraphs do (Chromium).
    let l = par("L", 1, 15);
    let m = par("M", 1, 12);
    let wrapped = mc(&format!("<div class=w id=w>{l}</div>{m}"));
    for (body, css, expected) in [
        (
            &wrapped,
            ".w{padding-bottom:6px}",
            &[
                "p0 x0: L1-L10@0",
                "p0 x100: L11-L15@0 M1-M4@106",
                "p1 x0: M5-M8@0",
                "p1 x100: M9-M12@0",
            ][..],
        ),
        (
            &wrapped,
            ".w{padding-top:6px}",
            &[
                "p0 x0: L1-L9@6",
                "p0 x100: L10-M4@0",
                "p1 x0: M5-M8@0",
                "p1 x100: M9-M12@0",
            ][..],
        ),
        (
            &wrapped,
            ".w{background:#fcc}",
            &[
                "p0 x0: L1-L10@0",
                "p0 x100: L11-M5@0",
                "p1 x0: M6-M9@0",
                "p1 x100: M10-M12@0",
            ][..],
        ),
        (
            &wrapped,
            ".w{border:3px solid green;background:#fcc}",
            &[
                "p0 x3: L1-L9@3",
                "p0 x100: M1-M3@123",
                "p0 x103: L10-L15@0",
                "p1 x0: M4-M8@0",
                "p1 x100: M9-M12@0",
            ][..],
        ),
        (
            &mc(&format!("<div class=w>{l}{}</div>{m}", par("N", 1, 3))),
            "p{background:#ccc;border:2px solid blue}",
            &[
                "p0 x2: L1-L9@2",
                "p0 x102: L10-L15@0 N1-N3@124",
                "p1 x2: M1-M6@2",
                "p1 x102: M7-M12@0",
            ][..],
        ),
    ] {
        assert_eq!(columns(body, css), expected, "{css}");
    }
    assert_eq!(
        visible_boxes(&wrapped, ".w{border:3px solid green;background:#fcc}", "w"),
        [(0, 0, 0, 200), (0, 100, 0, 123)],
        "border pieces"
    );
}

#[test]
fn a_painted_wrapper_has_a_piece_in_each_column_of_every_page() {
    // The boxes inside the wrapper are placed in the piece of their column,
    // so each page paints the wrapper's background there before its lines,
    // and the border below the boxes shows only in the last piece (Chromium).
    let body = mc(&format!(
        "<div class=w id=w>{}</div>{}",
        par("L", 1, 45),
        par("M", 1, 12)
    ));
    let css = ".w{background:#fcc;border-bottom:4px solid green}";
    assert_eq!(
        columns(&body, css),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
            "p2 x0: L41-L45@0 M1-M3@104",
            "p2 x100: M4-M12@0",
        ]
    );
    assert_eq!(
        visible_boxes(&body, css, "w"),
        [
            (0, 0, 0, 200),
            (0, 100, 0, 200),
            (1, 0, 0, 200),
            (1, 100, 0, 200),
            (2, 0, 0, 104),
        ]
    );
}

#[test]
fn wrappers_inside_a_painted_wrapper_have_a_piece_in_each_column_too() {
    // A wrapper inside one with pieces has a piece in each column it
    // reaches, placed in the outer piece there, and the boxes it holds are
    // placed in its piece of their column (Chromium).
    let m = par("M", 1, 12);
    let nested = mc(&format!(
        "<div class=w><div class=v id=v>{}</div>{}</div>{m}",
        par("L", 1, 45),
        par("K", 1, 3)
    ));
    let css =
        ".w{background:#fcc}.v{background:#cfc;border-bottom:4px solid blue;padding-left:5px}";
    assert_eq!(
        columns(&nested, css),
        [
            "p0 x5: L1-L10@0",
            "p0 x105: L11-L20@0",
            "p1 x5: L21-L30@0",
            "p1 x105: L31-L40@0",
            "p2 x0: K1-K3@104",
            "p2 x5: L41-L45@0",
            "p2 x100: M1-M10@0",
            "p3 x0: M11-M12@0",
        ]
    );
    assert_eq!(
        visible_boxes(&nested, css, "v"),
        [
            (0, 0, 0, 200),
            (0, 100, 0, 200),
            (1, 0, 0, 200),
            (1, 100, 0, 200),
            (2, 0, 0, 104),
        ]
    );
    // An unpainted wrapper inside, and a painted wrapper that one column
    // holds.
    let l = par("L", 1, 15);
    for (body, expected) in [
        (
            mc(&format!("<div class=w><div>{l}</div></div>{m}")),
            [
                "p0 x0: L1-L10@0",
                "p0 x100: L11-M5@0",
                "p1 x0: M6-M9@0",
                "p1 x100: M10-M12@0",
            ],
        ),
        (
            mc(&format!("{l}<div class=w>{}</div>{m}", par("N", 1, 2))),
            [
                "p0 x0: L1-L10@0",
                "p0 x100: L11-M3@0",
                "p1 x0: M4-M8@0",
                "p1 x100: M9-M12@0",
            ],
        ),
    ] {
        assert_eq!(columns(&body, ".w{background:#fcc}"), expected, "{body}");
    }
}

#[test]
fn sized_boxes_fill_each_page_in_column_order() {
    // A sized box with no lines, between the paragraphs or inside a
    // wrapper, joins the flow of the page fill: the columns of each page fill
    // in turn, and the last page balances (Chromium).
    let spacer = "<div style='height:30px'></div>";
    let filled = [
        "p0 x0: L1-L10@0",
        "p0 x100: L11-L15@0 M1-M3@130",
        "p1 x0: M4-M8@0",
        "p1 x100: M9-M12@0",
    ];
    let between = mc(&format!("{}{spacer}{}", par("L", 1, 15), par("M", 1, 12)));
    assert_eq!(columns(&between, ""), filled, "between paragraphs");
    let wrapped = mc(&format!(
        "<div>{}{spacer}</div>{}",
        par("L", 1, 15),
        par("M", 1, 12)
    ));
    assert_eq!(columns(&wrapped, ""), filled, "inside a wrapper");
    let padded = mc(&format!(
        "<div style='padding-bottom:6px'>{}{spacer}</div>{}",
        par("L", 1, 15),
        par("M", 1, 12)
    ));
    assert_eq!(
        columns(&padded, ""),
        [
            "p0 x0: L1-L10@0",
            "p0 x100: L11-L15@0 M1-M3@136",
            "p1 x0: M4-M8@0",
            "p1 x100: M9-M12@0",
        ],
        "inside a padded wrapper"
    );
}

#[test]
fn a_column_end_slices_a_sized_box_on_a_filled_page() {
    // A column end cuts the box, whose block-axis borders are sliced: the
    // top border shows in the first piece and the bottom one in the last
    // (Chromium).
    let tall = |inner: &str| {
        mc(&format!(
            "{}{}",
            inner.replace(
                "BOX",
                "<div id=b style='height:150px;background:#00f'></div>"
            ),
            par("M", 1, 12)
        ))
    };
    let lines = [
        "p0 x0: L1-L8@0",
        "p0 x100: M1-M4@110",
        "p1 x0: M5-M8@0",
        "p1 x100: M9-M12@0",
    ];
    let between = tall(&format!("{}BOX", par("L", 1, 8)));
    assert_eq!(columns(&between, ""), lines, "between paragraphs");
    assert_eq!(
        visible_boxes(&between, "", "b"),
        [(0, 0, 160, 200), (0, 100, 0, 110)]
    );
    let wrapped = tall(&format!(
        "<div id=w style='background:#ff0'>{}BOX</div>",
        par("L", 1, 8)
    ));
    assert_eq!(columns(&wrapped, ""), lines, "inside a painted wrapper");
    assert_eq!(
        visible_boxes(&wrapped, "", "b"),
        [(0, 0, 160, 200), (0, 100, 0, 110)]
    );
    assert_eq!(
        visible_boxes(&wrapped, "", "w"),
        [(0, 0, 0, 200), (0, 100, 0, 110)]
    );
    let bordered = mc(&format!(
        "{}<div id=b style='height:150px;border-top:5px solid red;border-bottom:5px solid lime'>\
         </div>{}",
        par("L", 1, 8),
        par("M", 1, 12)
    ));
    assert_eq!(
        columns(&bordered, ""),
        [
            "p0 x0: L1-L8@0",
            "p0 x100: M1-M4@120",
            "p1 x0: M5-M8@0",
            "p1 x100: M9-M12@0",
        ],
        "bordered"
    );
    assert_eq!(
        visible_boxes(&bordered, "", "b"),
        [(0, 0, 160, 200), (0, 100, 0, 120)]
    );
}

#[test]
fn a_sized_box_a_page_end_cuts_stays_in_the_columns_the_pages_cut() {
    // The page fill resumes no box in the middle, so one that a page end
    // cuts leaves the container to the columns that the pages cut, where
    // every line is reported once.
    let body = mc(&format!(
        "<div>{}<div style='height:60px'></div></div>{}",
        par("L", 1, 18),
        par("M", 1, 12)
    ));
    let mut texts: Vec<_> = lines(&body, "").into_iter().map(|line| line.3).collect();
    texts.sort();
    let mut expected: Vec<_> = (1..=18)
        .map(|n| format!("L{n}"))
        .chain((1..=12).map(|n| format!("M{n}")))
        .collect();
    expected.sort();
    assert_eq!(texts, expected);
}

#[test]
fn a_wrapper_whose_padding_a_column_end_cuts_ends_with_what_it_holds() {
    // Padding taller than a column is not sliced into pieces that each
    // hold all of it: the wrapper's background ends with its line, so it
    // paints over none of what follows, as in Chromium.
    let body = mc(&format!(
        "<div class=w id=w>{}</div>{}",
        par("L", 1, 1),
        par("M", 1, 30)
    ));
    let css = ".mc{column-fill:auto}.w{background:#fcc;padding-top:250px}";
    let line = lines(&body, css)
        .into_iter()
        .find(|(.., text)| text == "L1")
        .expect("the wrapper's line is laid out");
    let last = *visible_boxes(&body, css, "w")
        .last()
        .expect("the wrapper paints");
    assert_eq!((last.0, last.1, last.3), (line.0, line.1, line.2 + 20));
}
