//! Where the lines inside blocks that clone their decorations stand when the
//! blocks run onto later pages: each page a block is cut on repeats its
//! padding and border, which the lines leave room for (CSS Fragmentation 3,
//! section 5.4). The positions are the ones Chromium prints for the same
//! documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::BTreeMap;

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// Lines `from` through `to`, each labeled `prefix` and its number.
fn lines(prefix: &str, from: u32, to: u32) -> String {
    let lines: Vec<_> = (from..=to).map(|n| format!("{prefix}{n}")).collect();
    lines.join("<br>")
}

/// The lines of every page, as `p<page> x<left>: <runs>`. A run is the first
/// and last label of lines one after the other, and the top of the first, as
/// in `L1-L9@10`.
fn pages(body: &str, css: &str) -> Vec<String> {
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
         body{{font:20px/20px Ahem;margin:0}}p{{margin:0}}{css}</style><body>{body}</body>"
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
    let mut cells: BTreeMap<(u32, i32), Vec<(i32, String)>> = BTreeMap::new();
    for page in result.pages() {
        for run in page.text_runs() {
            cells
                .entry((page.index(), run.origin.0.round() as i32))
                .or_default()
                .push((
                    (run.origin.1 - ASCENT).round() as i32,
                    run.text.trim_end().to_owned(),
                ));
        }
    }
    cells
        .into_iter()
        .map(|((page, left), mut lines)| {
            lines.sort();
            let mut runs: Vec<Vec<(i32, String)>> = Vec::new();
            for line in lines {
                match runs.last_mut() {
                    Some(run) if run.last().is_some_and(|last| line.0 - last.0 == 20) => {
                        run.push(line)
                    }
                    _ => runs.push(vec![line]),
                }
            }
            let runs: Vec<_> = runs
                .iter()
                .map(|run| match run.len() {
                    1 => format!("{}@{}", run[0].1, run[0].0),
                    _ => format!("{}-{}@{}", run[0].1, run[run.len() - 1].1, run[0].0),
                })
                .collect();
            format!("p{page} x{left}: {}", runs.join(" "))
        })
        .collect()
}

const CLONED: &str = ".b{box-decoration-break:clone;border:6px solid blue;padding:4px}";

#[test]
fn lines_in_a_cloned_block_leave_room_for_its_edges_on_every_page() {
    assert_eq!(
        pages(
            &format!("<div class=b><p>{}</p></div>", lines("L", 1, 25)),
            CLONED
        ),
        [
            "p0 x10: L1-L9@10",
            "p1 x10: L10-L18@10",
            "p2 x10: L19-L25@10",
        ]
    );
}

#[test]
fn lines_in_nested_cloned_blocks_leave_room_for_the_edges_of_both() {
    assert_eq!(
        pages(
            &format!(
                "<div class=b><div class=c><p>{}</p></div></div>",
                lines("L", 1, 25)
            ),
            &format!("{CLONED}.c{{box-decoration-break:clone;border:3px solid red;padding:2px}}")
        ),
        [
            "p0 x15: L1-L8@15",
            "p1 x15: L9-L16@15",
            "p2 x15: L17-L23@15",
            "p3 x15: L24-L25@15",
        ]
    );
}

#[test]
fn what_follows_a_cloned_block_starts_below_its_last_fragment() {
    assert_eq!(
        pages(
            &format!(
                "<div class=b><p>{}</p></div><p>{}</p>",
                lines("L", 1, 12),
                lines("M", 1, 10)
            ),
            CLONED
        ),
        [
            "p0 x10: L1-L9@10",
            "p1 x0: M1-M6@80",
            "p1 x10: L10-L12@10",
            "p2 x0: M7-M10@0",
        ]
    );
}

#[test]
fn a_cloned_paragraph_repeats_its_own_edges_around_its_lines() {
    assert_eq!(
        pages(
            &format!("<p>{}</p>", lines("L", 1, 25)),
            "p{box-decoration-break:clone;border:6px solid blue;padding:4px;widows:1;orphans:1}"
        ),
        [
            "p0 x10: L1-L9@10",
            "p1 x10: L10-L18@10",
            "p2 x10: L19-L25@10",
        ]
    );
}
