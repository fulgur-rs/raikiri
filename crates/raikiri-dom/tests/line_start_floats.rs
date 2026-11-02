//! Floats that start a line, one after another.
//!
//! A run of left floats at the start of a line is placed one float at a time:
//! each one narrows the space the line has, so the line is measured again beside
//! the next. Past the first few the lines are no longer measured in the space the
//! last float left but in the one before it, so that the number of times a line
//! is scanned does not grow with the number of floats; the floats and the lines
//! have to land where they would if every line had been measured in its own
//! space. Each expected box is the one
//! Chromium lays out for the same markup, in an 800px by 600px viewport without a
//! body margin and with Ahem at 10px, where a word of three letters is 30px wide.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Eight words of three letters, which fill a 200px line three at a time.
const WORDS: &str = "aaa bbb ccc ddd eee fff ggg hhh";

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// The expected border box `(x, y, width, height)` of each element, by `id`.
type Expected<'a> = &'a [(&'a str, [f32; 4])];

/// Check that every `(id, expected box)` of every case is laid out as given.
fn check(cases: Vec<(&str, String, Expected<'_>)>) {
    for (what, body, expected) in cases {
        let actual = boxes(&body);
        for &(id, want) in expected {
            let got = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{what}: no fragment for #{id} in {body}"))
                .1;
            assert!(
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() < 0.01),
                "{what}: #{id} is {got:?}, expected {want:?} for {body}"
            );
        }
    }
}

/// `count` empty floats with ids `{prefix}0`, `{prefix}1`, and so on, each
/// styled by `style(index)`.
fn floats(prefix: &str, count: usize, style: impl Fn(usize) -> String) -> String {
    (0..count)
        .map(|index| format!("<span id={prefix}{index} style='{}'></span>", style(index)))
        .collect()
}

/// `count` identical empty floats.
fn same_floats(prefix: &str, count: usize, style: &str) -> String {
    floats(prefix, count, |_| style.to_owned())
}

#[test]
fn left_floats_in_a_row_sit_side_by_side_and_the_text_goes_beside_them() {
    check(vec![
        (
            "seven 13px floats, the text beside them",
            format!(
                "<div id=r style='width:200px'>{}{WORDS}</div>",
                same_floats("a", 7, "float:left;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
            ],
        ),
        (
            "seven 31px floats, the seventh no longer fits the row",
            format!(
                "<div id=r style='width:200px'>{}{WORDS}</div>",
                same_floats("a", 7, "float:left;width:31px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 32.0]),
                ("a0", [0.0, 0.0, 31.0, 12.0]),
                ("a1", [31.0, 0.0, 31.0, 12.0]),
                ("a2", [62.0, 0.0, 31.0, 12.0]),
                ("a3", [93.0, 0.0, 31.0, 12.0]),
                ("a4", [124.0, 0.0, 31.0, 12.0]),
                ("a5", [155.0, 0.0, 31.0, 12.0]),
                ("a6", [0.0, 12.0, 31.0, 12.0]),
            ],
        ),
        (
            "floats too wide to share a row stack",
            format!(
                "<div id=r style='width:60px'>{}{WORDS}</div>",
                same_floats("a", 12, "float:left;width:31px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 60.0, 224.0]),
                ("a0", [0.0, 0.0, 31.0, 12.0]),
                ("a1", [0.0, 12.0, 31.0, 12.0]),
                ("a2", [0.0, 24.0, 31.0, 12.0]),
                ("a3", [0.0, 36.0, 31.0, 12.0]),
                ("a4", [0.0, 48.0, 31.0, 12.0]),
                ("a5", [0.0, 60.0, 31.0, 12.0]),
                ("a6", [0.0, 72.0, 31.0, 12.0]),
                ("a7", [0.0, 84.0, 31.0, 12.0]),
                ("a8", [0.0, 96.0, 31.0, 12.0]),
                ("a9", [0.0, 108.0, 31.0, 12.0]),
                ("a10", [0.0, 120.0, 31.0, 12.0]),
                ("a11", [0.0, 132.0, 31.0, 12.0]),
            ],
        ),
        (
            "margins count in the space a float takes",
            format!(
                "<div id=r style='width:200px'>{}{WORDS}</div>",
                same_floats(
                    "a",
                    9,
                    "float:left;width:5px;height:12px;margin:0 3px 2px 1px"
                )
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [1.0, 0.0, 5.0, 12.0]),
                ("a1", [10.0, 0.0, 5.0, 12.0]),
                ("a2", [19.0, 0.0, 5.0, 12.0]),
                ("a3", [28.0, 0.0, 5.0, 12.0]),
                ("a4", [37.0, 0.0, 5.0, 12.0]),
                ("a5", [46.0, 0.0, 5.0, 12.0]),
                ("a6", [55.0, 0.0, 5.0, 12.0]),
                ("a7", [64.0, 0.0, 5.0, 12.0]),
                ("a8", [73.0, 0.0, 5.0, 12.0]),
            ],
        ),
        (
            "a right-to-left box still places left floats from the left",
            format!(
                "<div id=r style='width:200px;direction:rtl'>{}{WORDS}</div>",
                same_floats("a", 8, "float:left;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
                ("a7", [91.0, 0.0, 13.0, 12.0]),
            ],
        ),
    ]);
}

#[test]
fn left_floats_mixed_with_right_floats_and_clears_land_where_they_would_one_by_one() {
    check(vec![
        (
            "left and right floats of different heights",
            format!(
                "<div id=r style='width:200px'>{}{WORDS}</div>",
                floats("a", 8, |i| format!(
                    "float:{};width:13px;height:{}px",
                    if i % 3 == 2 { "right" } else { "left" },
                    8 + (i % 4) * 7
                ))
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 40.0]),
                ("a0", [0.0, 0.0, 13.0, 8.0]),
                ("a1", [13.0, 0.0, 13.0, 15.0]),
                ("a2", [187.0, 0.0, 13.0, 22.0]),
                ("a3", [26.0, 0.0, 13.0, 29.0]),
                ("a4", [39.0, 0.0, 13.0, 8.0]),
                ("a5", [174.0, 0.0, 13.0, 15.0]),
                ("a6", [52.0, 0.0, 13.0, 22.0]),
                ("a7", [65.0, 0.0, 13.0, 29.0]),
            ],
        ),
        (
            "floats that clear one side or the other",
            format!(
                "<div id=r style='width:200px'>{}{WORDS}</div>",
                floats("a", 7, |i| format!(
                    "float:{};clear:{};width:13px;height:9px",
                    if i % 2 == 0 { "left" } else { "right" },
                    ["none", "left", "right", "both", "none", "none", "left"][i]
                ))
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 20.0]),
                ("a0", [0.0, 0.0, 13.0, 9.0]),
                ("a1", [187.0, 9.0, 13.0, 9.0]),
                ("a2", [0.0, 18.0, 13.0, 9.0]),
                ("a3", [187.0, 27.0, 13.0, 9.0]),
                ("a4", [0.0, 27.0, 13.0, 9.0]),
                ("a5", [174.0, 27.0, 13.0, 9.0]),
                ("a6", [0.0, 36.0, 13.0, 9.0]),
            ],
        ),
        (
            "a second run of floats after a line break",
            format!(
                "<div id=r style='width:200px'>{}<br>{}{WORDS}</div>",
                same_floats("a", 10, "float:left;width:13px;height:12px"),
                same_floats("b", 10, "float:left;width:13px;height:20px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 60.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
                ("a7", [91.0, 0.0, 13.0, 12.0]),
                ("a8", [104.0, 0.0, 13.0, 12.0]),
                ("a9", [117.0, 0.0, 13.0, 12.0]),
                ("b0", [130.0, 10.0, 13.0, 20.0]),
                ("b1", [143.0, 10.0, 13.0, 20.0]),
                ("b2", [156.0, 10.0, 13.0, 20.0]),
                ("b3", [169.0, 10.0, 13.0, 20.0]),
                ("b4", [182.0, 10.0, 13.0, 20.0]),
                ("b5", [0.0, 30.0, 13.0, 20.0]),
                ("b6", [13.0, 30.0, 13.0, 20.0]),
                ("b7", [26.0, 30.0, 13.0, 20.0]),
                ("b8", [39.0, 30.0, 13.0, 20.0]),
                ("b9", [52.0, 30.0, 13.0, 20.0]),
            ],
        ),
    ]);
}

#[test]
fn left_floats_land_the_same_on_lines_that_are_aligned_indented_or_run_right_to_left() {
    // The position of a float in its line does not depend on how the line is
    // aligned, so the floats met at the start of a line are the same in the
    // space of the line and in the wider one it had before them.
    check(vec![
        (
            "a centered line with an indent",
            format!(
                "<div id=r style='width:200px;text-indent:17px;text-align:center'>{}{WORDS}</div>",
                same_floats("a", 8, "float:left;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
                ("a7", [91.0, 0.0, 13.0, 12.0]),
            ],
        ),
        (
            "a line aligned to the right",
            format!(
                "<div id=r style='width:200px;text-align:right'>{}{WORDS}</div>",
                same_floats("a", 8, "float:left;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
                ("a7", [91.0, 0.0, 13.0, 12.0]),
            ],
        ),
        (
            "wide floats on a centered line",
            format!(
                "<div id=r style='width:200px;text-align:center'>{}{WORDS}</div>",
                same_floats("a", 7, "float:left;width:31px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 32.0]),
                ("a0", [0.0, 0.0, 31.0, 12.0]),
                ("a1", [31.0, 0.0, 31.0, 12.0]),
                ("a2", [62.0, 0.0, 31.0, 12.0]),
                ("a3", [93.0, 0.0, 31.0, 12.0]),
                ("a4", [124.0, 0.0, 31.0, 12.0]),
                ("a5", [155.0, 0.0, 31.0, 12.0]),
                ("a6", [0.0, 12.0, 31.0, 12.0]),
            ],
        ),
        (
            "justified lines with an indent",
            format!(
                "<div id=r style='width:200px;text-indent:11px;text-align:justify'>{}{WORDS} {WORDS}</div>",
                same_floats("a", 8, "float:left;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 50.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
                ("a7", [91.0, 0.0, 13.0, 12.0]),
            ],
        ),
        (
            "a centered right-to-left line",
            format!(
                "<div id=r style='width:200px;direction:rtl;text-align:center'>{}{WORDS}</div>",
                same_floats("a", 8, "float:left;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
                ("a7", [91.0, 0.0, 13.0, 12.0]),
            ],
        ),
        (
            "right floats before text that runs right to left",
            format!(
                "<div id=r style='width:200px;direction:rtl'>{}{WORDS}</div>",
                same_floats("a", 8, "float:right;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [187.0, 0.0, 13.0, 12.0]),
                ("a1", [174.0, 0.0, 13.0, 12.0]),
                ("a2", [161.0, 0.0, 13.0, 12.0]),
                ("a3", [148.0, 0.0, 13.0, 12.0]),
                ("a4", [135.0, 0.0, 13.0, 12.0]),
                ("a5", [122.0, 0.0, 13.0, 12.0]),
                ("a6", [109.0, 0.0, 13.0, 12.0]),
                ("a7", [96.0, 0.0, 13.0, 12.0]),
            ],
        ),
        (
            "floats after a line break on an indented, centered line",
            format!(
                "<div id=r style='width:200px;text-indent:17px;text-align:center'>aaa<br>{}{WORDS}</div>",
                same_floats("a", 8, "float:left;width:13px;height:12px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 40.0]),
                ("a0", [0.0, 10.0, 13.0, 12.0]),
                ("a1", [13.0, 10.0, 13.0, 12.0]),
                ("a2", [26.0, 10.0, 13.0, 12.0]),
                ("a3", [39.0, 10.0, 13.0, 12.0]),
                ("a4", [52.0, 10.0, 13.0, 12.0]),
                ("a5", [65.0, 10.0, 13.0, 12.0]),
                ("a6", [78.0, 10.0, 13.0, 12.0]),
                ("a7", [91.0, 10.0, 13.0, 12.0]),
            ],
        ),
    ]);
}

#[test]
fn floats_that_come_after_content_are_not_taken_for_floats_at_the_start_of_the_line() {
    // The run of floats at the start of the line is followed by text, and the
    // floats after the text are met beside it, not at the start of the line.
    check(vec![
        (
            "two left floats after text, past a run of six",
            format!(
                "<div id=r style='width:200px'>{}abc{}{WORDS}</div>",
                same_floats("a", 6, "float:left;width:13px;height:12px"),
                same_floats("b", 2, "float:left;width:13px;height:7px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("b0", [78.0, 0.0, 13.0, 7.0]),
                ("b1", [91.0, 0.0, 13.0, 7.0]),
            ],
        ),
        (
            "two right floats after text, past a run of six",
            format!(
                "<div id=r style='width:200px'>{}abc{}{WORDS}</div>",
                same_floats("a", 6, "float:left;width:13px;height:12px"),
                same_floats("b", 2, "float:right;width:13px;height:7px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("b0", [187.0, 0.0, 13.0, 7.0]),
                ("b1", [174.0, 0.0, 13.0, 7.0]),
            ],
        ),
        (
            "a float in the middle of a word, past a run of seven",
            format!(
                "<div id=r style='width:200px'>{}ab{}cd {WORDS}</div>",
                same_floats("a", 7, "float:left;width:13px;height:12px"),
                same_floats("b", 1, "float:left;width:13px;height:7px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 30.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("a6", [78.0, 0.0, 13.0, 12.0]),
                ("b0", [91.0, 0.0, 13.0, 7.0]),
            ],
        ),
        (
            "wide floats after text, past a run of six",
            format!(
                "<div id=r style='width:200px'>{}abc{}{WORDS}</div>",
                same_floats("a", 6, "float:left;width:13px;height:12px"),
                same_floats("b", 3, "float:left;width:60px;height:7px")
            ),
            &[
                ("r", [0.0, 0.0, 200.0, 37.0]),
                ("a0", [0.0, 0.0, 13.0, 12.0]),
                ("a1", [13.0, 0.0, 13.0, 12.0]),
                ("a2", [26.0, 0.0, 13.0, 12.0]),
                ("a3", [39.0, 0.0, 13.0, 12.0]),
                ("a4", [52.0, 0.0, 13.0, 12.0]),
                ("a5", [65.0, 0.0, 13.0, 12.0]),
                ("b0", [78.0, 0.0, 60.0, 7.0]),
                ("b1", [78.0, 10.0, 60.0, 7.0]),
                ("b2", [138.0, 10.0, 60.0, 7.0]),
            ],
        ),
    ]);
}
