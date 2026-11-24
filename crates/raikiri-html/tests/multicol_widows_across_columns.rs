//! A paragraph split over three or more columns keeps its widows in the last
//! column: Chromium ends the column before it early, so that one holds fewer
//! lines, and keeps the orphans before each break. The widows and orphans are
//! those of the paragraph, which may differ from the container's. Orphans
//! win when both cannot hold, and widows taller than a column are dropped.
//!
//! The lines are the ones Chromium lays out for the same documents (10px
//! Ahem), shown per column, top to bottom, with `|` between the columns.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The lines of the 100px wide columns of `body`, laid out on one page.
fn columns(body: &str) -> String {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:400px 400px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}</style>{body}"
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
    let mut lines = std::collections::BTreeMap::<(i32, i32), Vec<(i32, char)>>::new();
    for page in result.pages() {
        for run in page.text_runs() {
            let mut x = run.origin.0.round() as i32;
            let y = run.origin.1.round() as i32;
            for glyph in run.text.chars() {
                if glyph != ' ' {
                    lines.entry((x / 100, y)).or_default().push((x, glyph));
                }
                x += 10;
            }
        }
    }
    let mut columns = std::collections::BTreeMap::<i32, Vec<String>>::new();
    for ((column, _), mut glyphs) in lines {
        glyphs.sort();
        columns
            .entry(column)
            .or_default()
            .push(glyphs.into_iter().map(|(_, glyph)| glyph).collect());
    }
    columns
        .into_values()
        .map(|lines| lines.join(" "))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn the_last_column_keeps_the_widows_of_a_paragraph_over_three_columns_or_more() {
    for (body, expected) in [
        // balanced
        (
            r#"<div style="columns:3;column-gap:0;width:300px"><div style="width:20px;orphans:1;widows:3">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij | kl mn op",
        ),
        // column-fill:auto
        (
            r#"<div style="columns:3;column-gap:0;width:300px;column-fill:auto;height:30px"><div style="width:20px;orphans:1;widows:3">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij | kl mn op",
        ),
        // sized box
        (
            r#"<div style="columns:3;column-gap:0;width:300px"><div style="width:20px;orphans:1;widows:3;height:80px">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij | kl mn op",
        ),
        // four columns
        (
            r#"<div style="columns:4;column-gap:0;width:400px"><div style="width:20px;orphans:1;widows:3">ab cd ef gh ij kl mn op qr st</div></div>"#,
            "ab cd ef | gh ij kl | mn | op qr st",
        ),
        // orphans 2
        (
            r#"<div style="columns:3;column-gap:0;width:300px"><div style="width:20px;orphans:2;widows:3">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij | kl mn op",
        ),
        // widows 4
        (
            r#"<div style="columns:3;column-gap:0;width:300px"><div style="width:20px;orphans:1;widows:4">ab cd ef gh ij kl mn op qr st</div></div>"#,
            "ab cd ef gh | ij kl | mn op qr st",
        ),
        // widows inherited from the container
        (
            r#"<div style="columns:3;column-gap:0;width:300px;widows:3;orphans:1"><div style="width:20px">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij | kl mn op",
        ),
        // four auto-filled columns
        (
            r#"<div style="columns:4;column-gap:0;width:400px;column-fill:auto;height:30px"><div style="width:20px;orphans:1;widows:3">ab cd ef gh ij kl mn op qr st</div></div>"#,
            "ab cd ef | gh ij kl | mn | op qr st",
        ),
        // auto-filled, orphans 2
        (
            r#"<div style="columns:3;column-gap:0;width:300px;column-fill:auto;height:30px"><div style="width:20px;orphans:2;widows:3">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij | kl mn op",
        ),
        // widows larger than the last columns hold
        (
            r#"<div style="columns:3;column-gap:0;width:300px;widows:5;orphans:1"><div style="width:20px">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij kl mn op",
        ),
    ] {
        assert_eq!(columns(body), expected, "{body}");
    }
}

#[test]
fn orphans_win_over_widows_and_widows_taller_than_a_column_are_dropped() {
    for (body, expected) in [
        // orphans win over widows
        (
            r#"<div style="columns:3;column-gap:0;width:300px"><div style="width:20px;orphans:3;widows:2">ab cd ef gh ij kl mn</div></div>"#,
            "ab cd ef | gh ij kl | mn",
        ),
        // auto-filled, orphans win over widows
        (
            r#"<div style="columns:3;column-gap:0;width:300px;column-fill:auto;height:30px"><div style="width:20px;orphans:3;widows:2">ab cd ef gh ij kl mn</div></div>"#,
            "ab cd ef | gh ij kl | mn",
        ),
        // auto-filled, widows that begin at the start of the piece
        (
            r#"<div style="columns:3;column-gap:0;width:300px;column-fill:auto;height:30px"><div style="width:20px;orphans:2;widows:4">ab cd ef gh ij kl mn</div></div>"#,
            "ab cd ef | gh ij kl | mn",
        ),
        // auto-filled, the break moves up as far as the orphans let it
        (
            r#"<div style="columns:3;column-gap:0;width:300px;column-fill:auto;height:30px"><div style="width:20px;orphans:2;widows:3">ab cd ef gh ij kl mn</div></div>"#,
            "ab cd ef | gh ij | kl mn",
        ),
        // auto-filled, widows taller than a column are dropped
        (
            r#"<div style="columns:3;column-gap:0;width:300px;column-fill:auto;height:30px"><div style="width:20px;orphans:2;widows:4">ab cd ef gh ij kl mn op</div></div>"#,
            "ab cd ef | gh ij kl | mn op",
        ),
        // auto-filled, overflow column
        (
            r#"<div style="columns:3;column-gap:0;width:300px;column-fill:auto;height:30px"><div style="width:20px;orphans:2;widows:4">ab cd ef gh ij kl mn op qr st</div></div>"#,
            "ab cd ef | gh ij kl | mn op qr | st",
        ),
    ] {
        assert_eq!(columns(body), expected, "{body}");
    }
}

#[test]
fn a_paragraph_beside_a_float_keeps_its_own_orphans_in_its_first_column() {
    for (body, expected) in [
        // Too few lines fit for the paragraph's orphans: it starts the next
        // column, though the container's orphans would let it begin here.
        (
            r#"<div style="columns:2;column-gap:0;width:200px;height:50px;orphans:1;widows:1"><div style="height:25px"></div><div style="float:left;width:80px;height:10px"></div><div style="orphans:3">ab cd ef gh ij kl mn op</div></div>"#,
            "abcdef ghijkl mnop",
        ),
        // The paragraph's single orphan fits, though the container's would not.
        (
            r#"<div style="columns:2;column-gap:0;width:200px;height:50px;orphans:3;widows:1"><div style="height:25px"></div><div style="float:left;width:80px;height:10px"></div><div style="orphans:1">ab cd ef gh ij kl mn op</div></div>"#,
            "ab | cdefgh ijklmn op",
        ),
    ] {
        assert_eq!(columns(body), expected, "{body}");
    }
}
