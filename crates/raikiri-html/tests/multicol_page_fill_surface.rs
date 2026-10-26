//! Where the lines of a multicol container that runs onto later pages stand.
//! The positions are the ones Chromium prints for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
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

/// The page, left edge, top and text of every line the pages report.
fn lines(body: &str, css: &str) -> Vec<(u32, i32, i32, String)> {
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
fn a_paragraph_that_is_cut_and_shifted_keeps_its_runs() {
    // A piece of a paragraph is clipped at its column, and the painter moves
    // the paragraph's own offset inside that clip, so the runs of the pieces
    // are not reported. The paragraph stays in the columns that the pages cut,
    // where all its lines are reported.
    for shift in [
        "p{position:relative;top:3px}",
        "p{transform:translate(5px,3px)}",
    ] {
        let mut texts: Vec<_> = lines(&mc(&par("L", 1, 45)), shift)
            .into_iter()
            .map(|line| line.3)
            .collect();
        texts.sort_by_key(|text| text[1..].parse::<u32>().unwrap());
        let expected: Vec<_> = (1..=45).map(|n| format!("L{n}")).collect();
        assert_eq!(texts, expected, "{shift}");
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
