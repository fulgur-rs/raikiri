//! Table parts beside the inline content of a block container, wrapped in an
//! anonymous block-level table between the lines around it, paginated and
//! painted against the same parts inside a table that is written out.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const WIDTH: u32 = 60;
const HEIGHT: u32 = 50;
const BLUE: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

/// The raster of every page of `body`, in Ahem on 60px by 50px pages. Text
/// casts a red shadow 4px upwards, so text that paints over a table shows it.
fn pages(body: &str, css: &str) -> Vec<Vec<u8>> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:{WIDTH}px {HEIGHT}px;margin:0}}\
         body{{margin:0;background:white;font:10px/10px Ahem;text-shadow:0 -4px 0 red}}\
         .r{{display:table-row;background:rgb(230,230,100)}}\
         .c{{display:table-cell;width:20px;height:10px;background:blue}}{css}</style>{body}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    (0..result.page_count())
        .map(|index| {
            let page = result.page(index).unwrap();
            let (doc, cascade, page_box, origin) = page.paint_inputs();
            let mut scene = Scene::new();
            raikiri_paint::paint_single_page_with_origin(
                &mut scene, doc, cascade, page_box, origin,
            )
            .unwrap();
            anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |out| out.append_scene(scene, Affine::IDENTITY),
                WIDTH,
                HEIGHT,
            )
        })
        .collect()
}

fn rows(count: usize) -> String {
    "<div class=r><div class=c></div><div class=c></div></div>".repeat(count)
}

fn row_group(count: usize) -> String {
    format!(
        "<div style=\"display:table-row-group;background:rgb(100,230,230)\">{}</div>",
        rows(count)
    )
}

fn table(inner: &str) -> String {
    format!("<div style=\"display:table\">{inner}</div>")
}

fn pixel(page: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * WIDTH + x) * 4) as usize;
    page[at..at + 4].try_into().unwrap()
}

/// Asserts that the parent `css` holding `loose` (parts beside inline
/// content) pages and paints like the one holding `explicit` (the same parts
/// in a table that is written out).
fn assert_same(name: &str, parent_css: &str, extra_css: &str, loose: &str, explicit: &str) {
    let loose = pages(
        &format!("<div style=\"{parent_css}\">{loose}</div>"),
        extra_css,
    );
    let explicit = pages(
        &format!("<div style=\"{parent_css}\">{explicit}</div>"),
        extra_css,
    );
    assert_eq!(loose.len(), explicit.len(), "{name}: page count");
    for (index, (loose, explicit)) in loose.iter().zip(&explicit).enumerate() {
        assert!(loose == explicit, "{name}: page {index} differs");
    }
}

#[test]
fn rows_beside_lines_paginate_and_paint_like_a_table() {
    // Rows that fit, rows that cross the page edge, and rows that fill
    // several pages, after one line or several and before none or some.
    for lead in ["X", "X<br>X<br>X"] {
        for tail in ["", "Y", "Y<br>Y<br>Y"] {
            for count in [1, 3, 8] {
                assert_same(
                    &format!("lead={lead} tail={tail} rows={count}"),
                    "",
                    "",
                    &format!("{lead}{}{tail}", rows(count)),
                    &format!("{lead}{}{tail}", table(&rows(count))),
                );
            }
        }
    }
}

#[test]
fn the_text_after_a_row_paints_over_its_cells() {
    // The shadow of the "Y" reaches 4px into the cell above it, and the text
    // of a paragraph paints over the block backgrounds around it.
    let page = &pages(&format!("<div>X{}Y</div>", rows(1)), "")[0];
    assert_eq!(pixel(page, 0, 15), BLUE);
    assert_eq!(pixel(page, 0, 16), RED);
}

#[test]
fn the_text_of_a_block_above_table_parts_outside_a_table_paints_over_their_cells() {
    // The shadow of the "X" reaches 4px down into the cell below it. The parts
    // stand in an anonymous table, whose parts paint as block backgrounds
    // below the text of the blocks around them.
    let css = "body{text-shadow:0 4px 0 red}";
    let cells = "<div class=c></div><div class=c></div>";
    for (name, parts) in [
        ("rows", rows(1)),
        ("row group", row_group(1)),
        ("cells", cells.to_string()),
    ] {
        let loose = &pages(&format!("<div>X</div>{parts}"), css)[0];
        let explicit = &pages(&format!("<div>X</div>{}", table(&parts)), css)[0];
        assert!(loose == explicit, "{name}");
        assert_eq!(pixel(loose, 0, 12), RED, "{name}");
        assert_eq!(pixel(loose, 0, 16), BLUE, "{name}");
    }
}

#[test]
fn cells_directly_in_a_table_paint_like_cells_in_a_row() {
    // The cells stand in an anonymous row, whose cells paint as block
    // backgrounds below the text of the block above the table.
    let css = "body{text-shadow:0 4px 0 red}";
    let cells = "<div class=c></div><div class=c></div>";
    let bare = &pages(&format!("<div>X</div>{}", table(cells)), css)[0];
    let in_row = &pages(
        &format!(
            "<div>X</div>{}",
            table(&format!("<div class=r>{cells}</div>"))
        ),
        css,
    )[0];
    assert_eq!(pixel(bare, 0, 12), RED);
    assert_eq!(pixel(in_row, 0, 12), RED);
}

#[test]
fn row_groups_cells_and_several_tables_beside_lines_paint_like_tables() {
    let group = row_group(3);
    let cells = "<div class=c></div>".repeat(3);
    assert_same(
        "row group",
        "",
        "",
        &format!("X<br>X{group}Y"),
        &format!("X<br>X{}Y", table(&group)),
    );
    assert_same(
        "bare cells",
        "",
        "",
        &format!("X{cells}Y"),
        &format!("X{}Y", table(&cells)),
    );
    assert_same(
        "text between rows",
        "",
        "",
        &format!("X{}Y{}Z", rows(2), rows(2)),
        &format!("X{}Y{}Z", table(&rows(2)), table(&rows(2))),
    );
}

#[test]
fn rows_beside_lines_follow_the_context_around_them() {
    let float = "<div style=\"float:left;width:10px;height:20px;background:red\"></div>";
    let positioned = "<div style=\"position:absolute;bottom:0;right:0;width:10px;height:10px;background:red\"></div>";
    let raised = "<div class=r style=\"position:relative;top:2px\"><div class=c></div></div>";
    let page_break = "<div class=r style=\"break-before:page\"><div class=c></div></div>";
    for (name, parent, loose, explicit) in [
        (
            "float",
            "",
            format!("{float}X{}Y", rows(2)),
            format!("{float}X{}Y", table(&rows(2))),
        ),
        (
            "absolute",
            "position:relative;height:45px",
            format!("X{positioned}{}Y", rows(2)),
            format!("X{positioned}{}Y", table(&rows(2))),
        ),
        (
            "absolutely positioned parent",
            "position:absolute;left:5px;top:12px;width:40px",
            format!("X{}Y", rows(8)),
            format!("X{}Y", table(&rows(8))),
        ),
        (
            "relative row",
            "",
            format!("X{raised}Y"),
            format!("X{}Y", table(raised)),
        ),
        (
            "break before a row",
            "",
            format!("X{page_break}{}Y", rows(2)),
            format!("X{}Y", table(&format!("{page_break}{}", rows(2)))),
        ),
        (
            "inline elements",
            "",
            format!("X<b>B</b>{}<i>I</i>Y", rows(2)),
            format!("X<b>B</b>{}<i>I</i>Y", table(&rows(2))),
        ),
        (
            "clipped block",
            "height:45px;overflow:hidden",
            format!("X{}Y", rows(6)),
            format!("X{}Y", table(&rows(6))),
        ),
        (
            "inline-block",
            "",
            format!(
                "X<span style=\"display:inline-block\">A{}B</span>Y",
                rows(2)
            ),
            format!(
                "X<span style=\"display:inline-block\">A{}B</span>Y",
                table(&rows(2))
            ),
        ),
        (
            "nested",
            "",
            format!("X<div>A{}B</div>Y{}Z", rows(2), rows(2)),
            format!("X<div>A{}B</div>Y{}Z", table(&rows(2)), table(&rows(2))),
        ),
        (
            "table cell",
            "display:table-cell;width:40px",
            format!("X{}Y", rows(2)),
            format!("X{}Y", table(&rows(2))),
        ),
    ] {
        assert_same(name, parent, "", &loose, &explicit);
    }
}

#[test]
fn the_parent_decides_inherited_table_properties_and_alignment() {
    for (name, parent) in [
        (
            "background and padding",
            "background:rgb(200,255,200);padding:2px;border:1px solid red",
        ),
        ("collapsed borders", "border-collapse:collapse"),
        ("border spacing", "border-spacing:3px 2px"),
        ("right aligned", "text-align:right;padding:0 3px"),
        ("right to left", "direction:rtl"),
        ("line height", "line-height:14px"),
    ] {
        assert_same(
            name,
            parent,
            "",
            &format!("X<br>X{}Y", rows(3)),
            &format!("X<br>X{}Y", table(&rows(3))),
        );
    }
}

#[test]
fn generated_content_and_first_line_styles_of_the_parent_surround_the_table() {
    let loose = format!("X{}Y", rows(2));
    let explicit = format!("X{}Y", table(&rows(2)));
    let only_rows = (rows(2), table(&rows(2)));
    for (name, css, loose, explicit) in [
        (
            "inline ::before and ::after",
            "body>div::before{content:'B';color:green}body>div::after{content:'A';color:green}",
            &loose,
            &explicit,
        ),
        (
            "only rows between generated text",
            "body>div::before{content:'B';color:green}body>div::after{content:'A';color:green}",
            &only_rows.0,
            &only_rows.1,
        ),
        (
            "block ::before",
            "body>div::before{content:'B';display:block;background:green}",
            &loose,
            &explicit,
        ),
        (
            "block ::after",
            "body>div::after{content:'A';display:block;background:green}",
            &loose,
            &explicit,
        ),
        (
            "::first-line",
            "body>div::first-line{color:red;background:rgb(255,200,0)}",
            &loose,
            &explicit,
        ),
        (
            "::first-letter",
            "body>div::first-letter{color:red}",
            &loose,
            &explicit,
        ),
        (
            "::before of the row",
            ".r::before{content:'';display:block;height:2px;background:green}",
            &loose,
            &explicit,
        ),
    ] {
        assert_same(name, "", css, loose, explicit);
    }
}

#[test]
fn cells_in_a_multicol_container_stay_blocks() {
    // The container keeps its parts as blocks, where nothing wraps them in a
    // table, and they still paint.
    let page = &pages(
        "<div style=\"column-count:2\"><div class=c></div></div>",
        "",
    )[0];
    assert_eq!(pixel(page, 0, 0), BLUE);
    assert_eq!(pixel(page, 19, 9), BLUE);
}
