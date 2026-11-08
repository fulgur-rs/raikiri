//! A table caption that crosses the end of a page: its lines break between
//! like those of a paragraph (orphans, widows and `break-inside`), and the
//! rows, the other captions and the blocks after it follow. Positions come
//! from Chromium 152 (printToPDF, page 200x200, Ahem 8px with a 10px line
//! height, tables with collapsed borders and no cell padding).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Chromium rounds the 6.4px ascent of an 8px Ahem glyph to 6px, and the 10px
/// line adds 1px of half-leading above it: the baseline sits 7px below the
/// top of the line.
const BASELINE: f32 = 7.0;

/// The text of every line on every page as `text@top`, the pages separated
/// by ` | `. The caption text `t1` fits one word to a 16px line.
fn pages(body: &str) -> String {
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
         html,body{{margin:0}}body{{font:8px/10px Ahem}}\
         table{{border-collapse:collapse;border-spacing:0}}\
         td{{padding:0;height:10px}}</style>{body}"
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
    (0..result.page_count())
        .map(|index| {
            result
                .page(index)
                .unwrap()
                .text_runs()
                .into_iter()
                .map(|run| {
                    let top = ((run.origin.1 - BASELINE) * 10.0).round() / 10.0;
                    format!("{}@{top}", run.text.trim_end())
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// A block `before` px high, then a table with one row `c` and a caption
/// `style` with `lines` lines of `t1`.
fn table_after(before: u32, style: &str, lines: usize) -> String {
    let text = "t1 ".repeat(lines);
    format!(
        "<div style='height:{before}px'>A</div><table>\
         <caption style='width:16px;{style}'>{text}</caption>\
         <tbody><tr><td>c</td></tr></tbody></table>"
    )
}

/// [`table_after`] followed by the block `Z`.
fn captioned(before: u32, style: &str, lines: usize) -> String {
    table_after(before, style, lines) + "<div>Z</div>"
}

#[test]
fn a_top_caption_breaks_between_its_lines_at_the_page_end() {
    // Two lines fit in the 25px left on the page, which is the orphans, and
    // the rest is at least the widows: the row and the block follow the
    // lines on the second page.
    for (lines, expected) in [
        (4, "A@0 t1@175 t1@185 | t1@0 t1@10 c@20 Z@30"),
        (5, "A@0 t1@175 t1@185 | t1@0 t1@10 t1@20 c@30 Z@40"),
        (6, "A@0 t1@175 t1@185 | t1@0 t1@10 t1@20 t1@30 c@40 Z@50"),
    ] {
        assert_eq!(pages(&captioned(175, "", lines)), expected, "{lines} lines");
    }
    // The same with two lines left on the first page after the break, 35px
    // above the page end.
    assert_eq!(
        pages(&captioned(165, "", 4)),
        "A@0 t1@165 t1@175 | t1@0 t1@10 c@20 Z@30"
    );
}

#[test]
fn the_orphans_and_widows_of_the_caption_decide_the_break() {
    // Three orphans and three widows of seven lines: three lines fit.
    assert_eq!(
        pages(&captioned(165, "orphans:3;widows:3", 7)),
        "A@0 t1@165 t1@175 t1@185 | t1@0 t1@10 t1@20 t1@30 c@40 Z@50"
    );
    // One orphan and one widow allow a break after the only line that fits
    // 15px above the page end, and after the two that fit 25px above it.
    assert_eq!(
        pages(&captioned(175, "orphans:1;widows:1", 4)),
        "A@0 t1@175 t1@185 | t1@0 t1@10 c@20 Z@30"
    );
    assert_eq!(
        pages(&captioned(185, "orphans:1;widows:1", 4)),
        "A@0 t1@185 | t1@0 t1@10 t1@20 c@30 Z@40"
    );
}

#[test]
fn a_caption_that_may_not_break_inside_moves_whole() {
    let moved = "A@0 | t1@0 t1@10 t1@20 t1@30 c@40 Z@50";
    assert_eq!(pages(&captioned(175, "break-inside:avoid", 4)), moved);
    let html = captioned(175, "", 4).replace("<table>", "<table style='break-inside:avoid'>");
    assert_eq!(pages(&html), moved);
}

#[test]
fn a_bottom_caption_breaks_between_its_lines_and_a_short_one_moves_whole() {
    // A 60px row and six lines under it: four lines fit below the row, and
    // the last two are the widows.
    let html = "<div style='height:100px'>A</div><table>\
        <tbody><tr><td style='height:60px'>c</td></tr></tbody>\
        <caption style='width:16px;caption-side:bottom'>t1 t1 t1 t1 t1 t1</caption>\
        </table><div>Z</div>";
    assert_eq!(
        pages(html),
        "A@0 c@125 t1@160 t1@170 t1@180 t1@190 | t1@0 t1@10 Z@20"
    );
    // Four lines under a row that ends 15px above the page end: only one
    // fits, short of the two orphans, so the caption moves whole and the row
    // stays.
    assert_eq!(
        pages(&captioned(175, "caption-side:bottom", 4)),
        "A@0 c@175 | t1@0 t1@10 t1@20 t1@30 Z@40"
    );
}

#[test]
fn several_captions_and_every_row_follow_the_broken_lines() {
    // Two captions above the row: the first fits, the second breaks.
    let html = "<div style='height:165px'>A</div><table>\
        <caption style='width:16px'>t1 t1 t1</caption>\
        <caption style='width:16px'>t2 t2 t2 t2</caption>\
        <tbody><tr><td>c</td></tr></tbody></table><div>Z</div>";
    assert_eq!(
        pages(html),
        "A@0 t1@165 t1@175 t1@185 | t2@0 t2@10 t2@20 t2@30 c@40 Z@50"
    );
    // A caption above and one below the row.
    let html = "<div style='height:165px'>A</div><table>\
        <caption style='width:16px'>t1 t1 t1 t1</caption>\
        <tbody><tr><td>c</td></tr></tbody>\
        <caption style='width:16px;caption-side:bottom'>b1 b1 b1 b1</caption>\
        </table><div>Z</div>";
    assert_eq!(
        pages(html),
        "A@0 t1@165 t1@175 | t1@0 t1@10 c@20 b1@30 b1@40 b1@50 b1@60 Z@70"
    );
    // Three rows after the caption.
    let html = captioned(175, "", 4).replace(
        "<td>c</td></tr>",
        "<td>c</td></tr><tr><td>d</td></tr><tr><td>e</td></tr>",
    );
    assert_eq!(
        pages(&html),
        "A@0 t1@175 t1@185 | t1@0 t1@10 c@20 d@30 e@40 Z@50"
    );
    // No row at all.
    let html = "<div style='height:175px'>A</div><table>\
        <caption style='width:16px'>t1 t1 t1 t1</caption></table><div>Z</div>";
    assert_eq!(pages(html), "A@0 t1@175 t1@185 | t1@0 t1@10 Z@20");
}

#[test]
fn the_space_around_the_caption_moves_with_the_lines() {
    // Bottom padding of the caption.
    assert_eq!(
        pages(&captioned(175, "padding-bottom:5px", 4)),
        "A@0 t1@175 t1@185 | t1@0 t1@10 c@25 Z@35"
    );
    // Top padding takes 5px of the room before the page end.
    assert_eq!(
        pages(&captioned(175, "padding-top:5px", 4)),
        "A@0 t1@180 t1@190 | t1@0 t1@10 c@20 Z@30"
    );
    // The margins of the table.
    let html = captioned(165, "", 4).replace("<table>", "<table style='margin:10px 0'>");
    assert_eq!(pages(&html), "A@0 t1@175 t1@185 | t1@0 t1@10 c@20 Z@40");
    // The padding of a block around the table keeps its bottom edge under
    // the table.
    let html = format!(
        "<div style='padding-bottom:12px'>{}</div><div>Z</div>",
        table_after(175, "", 4)
    );
    assert_eq!(pages(&html), "A@0 t1@175 t1@185 | t1@0 t1@10 c@20 Z@42");
    // The border of a block around the table.
    let html = format!(
        "<div style='border-bottom:4px solid'>{}</div><div>Z</div>",
        table_after(175, "", 4)
    );
    assert_eq!(pages(&html), "A@0 t1@175 t1@185 | t1@0 t1@10 c@20 Z@34");
    // A table with separate borders.
    let html = captioned(175, "", 4).replace(
        "<table>",
        "<table style='border-collapse:separate;border:3px solid'>",
    );
    assert_eq!(pages(&html), "A@0 t1@175 t1@185 | t1@0 t1@10 c@23 Z@36");
}

#[test]
fn a_caption_of_many_lines_breaks_at_every_page_end() {
    let words = |count: u32| {
        (0..count)
            .map(|line| format!("t1@{}", line * 10))
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(
        pages(&captioned(175, "", 30)),
        format!("A@0 t1@175 t1@185 | {} | {} c@80 Z@90", words(20), words(8))
    );
}
