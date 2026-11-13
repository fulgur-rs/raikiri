//! A flex or grid container in a vertical writing mode has its inline size
//! along its height. A `min-content`, `max-content` or `fit-content` height
//! sizes it to that intrinsic size of its content, or to the room it is
//! offered within those (CSS Sizing 3 section 3.2), whether it is block-level,
//! inline-level, floating, positioned, or a block in a vertical parent. The
//! items it lays out keep overflowing it rather than growing it. The same
//! holds for a row whose items cannot shrink to the room its parent gives it:
//! the content that overflows the row's inline size does not grow the row
//! when the document is broken into pages. Where each box falls is what
//! Chromium lays out for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out on one
/// page 800px wide and 600px tall in 10px Ahem, with border-box `div`s.
fn boxes(body: &str) -> Vec<(String, [i32; 4])> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}} body{{margin:0;font:10px/10px Ahem}} div{{box-sizing:border-box}}</style><body>{body}</body>"
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
    let page = result.page(0).unwrap();
    page.fragments()
        .filter_map(|fragment| {
            let id = page.dom().attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((
                id.to_owned(),
                [rect.x, rect.y, rect.width, rect.height].map(|value| value.round() as i32),
            ))
        })
        .collect()
}

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let expected = expected
            .iter()
            .map(|&(id, rect)| (id.to_owned(), rect))
            .collect::<Vec<_>>();
        assert_eq!(boxes(body), expected, "{body}");
    }
}

#[test]
fn a_vertical_flex_container_takes_its_content_keyword_height() {
    assert_rows(&[
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl"><div id=m style="flex:1 0 30px;height:min-content">xxxxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap"><div id=m style="flex:1 0 30px;height:min-content">xxxxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content"><div id=m style="flex:0 0 30px;overflow:hidden">xxxxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 30]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content"><div id=m style="flex:1 0 30px;max-height:20px">xxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 40]),
                ("m", [0, 0, 10, 20]),
                ("i", [0, 20, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content"><div id=m style="flex:1 1 30px">xxxx</div><div id=i style="flex:1 1 70px">xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 60]),
                ("m", [10, 0, 10, 60]),
                ("i", [0, 0, 10, 60]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content"><div id=m style="flex:1 1 30px;min-height:50px">x</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 70]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:min-content"><div id=m style="flex:0 0 30px;overflow:hidden">xxxxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 30]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:min-content"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div>"#,
            &[
                ("o", [0, 0, 20, 10]),
                ("m", [10, 0, 10, 30]),
                ("i", [0, 0, 10, 70]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:min-content"><div id=m style="flex:1 0 30px;max-height:20px">xxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 20, 20]),
                ("m", [10, 0, 10, 20]),
                ("i", [0, 0, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:min-content"><div id=m style="flex:1 1 30px">xxxx</div><div id=i style="flex:1 1 70px">xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 40]),
                ("m", [10, 0, 10, 40]),
                ("i", [0, 0, 10, 40]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:min-content"><div id=m style="flex:1 1 30px;min-height:50px">x</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:min-content"><div id=m>xx xx</div><div id=i>xxx</div></div>"#,
            &[
                ("o", [0, 0, 30, 30]),
                ("m", [10, 0, 20, 30]),
                ("i", [0, 0, 10, 30]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;height:min-content"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;height:min-content"><div id=m>xx xx</div><div id=i>xxx</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [0, 0, 20, 20]),
                ("i", [0, 20, 20, 30]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap"><div id=m style="flex:1 0 30px;height:min-content">xxxxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap"><div id=m style="flex:1 0 30px;height:min-content">xxxxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap"><div id=m style="flex:1 0 30px;height:min-content">xxxxxx</div><div id=i style="flex:1 0 20px">x</div></div>"#,
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="writing-mode:vertical-rl;height:min-content"><div id=m>xx xx</div><div id=i>xxx</div></div>"#,
            &[
                ("o", [0, 0, 30, 30]),
                ("m", [10, 0, 20, 30]),
                ("i", [0, 0, 10, 30]),
            ],
        ),
        (
            r#"<div style="display:inline-block"><div id=o style="display:inline-flex;writing-mode:vertical-rl;flex-wrap:wrap;height:min-content"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 10]),
                ("m", [10, 0, 10, 30]),
                ("i", [0, 0, 10, 70]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl"><div id=o style="display:flex;flex-wrap:wrap;height:min-content"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 10]),
                ("m", [10, 0, 10, 30]),
                ("i", [0, 0, 10, 70]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_row_overflowing_its_parents_room_keeps_its_height() {
    assert_rows(&[
        (
            r#"<div style="height:50px"><div id=o style="display:flex;writing-mode:vertical-rl"><div id=m style="flex:0 0 30px">x</div><div id=i style="flex:0 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            r#"<div style="height:50px"><div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 60px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 60]),
            ],
        ),
        (
            r#"<div style="height:50px"><div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 70]),
            ],
        ),
        (
            r#"<div style="height:50px"><div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap"><div id=m style="flex:1 0 30px">xxxxxxxx</div><div id=i style="flex:1 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 80]),
                ("m", [10, 0, 10, 80]),
                ("i", [0, 0, 10, 80]),
            ],
        ),
        (
            r#"<div style="height:50px"><div id=o style="display:inline-flex;writing-mode:vertical-rl;flex-wrap:wrap"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">xxxxxxxx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 80]),
                ("m", [10, 0, 10, 80]),
                ("i", [0, 0, 10, 80]),
            ],
        ),
        (
            r#"<div style="height:50px"><div id=o style="float:left;display:flex;writing-mode:vertical-lr;flex-wrap:wrap"><div id=m style="flex:0 0 30px;overflow:hidden">xxxxxxxx</div><div id=i style="flex:1 0 20px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 50]),
            ],
        ),
        (
            r#"<div style="height:50px"><div id=o style="writing-mode:vertical-rl"><div id=m style="height:70px">x</div></div></div>"#,
            &[("o", [0, 0, 10, 70]), ("m", [0, 0, 10, 70])],
        ),
    ]);
}

#[test]
fn vertical_containers_take_their_keyword_heights_wherever_they_are_laid_out() {
    assert_rows(&[
        (
            r#"<div id=o style="display:inline-block;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="float:left;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-block;writing-mode:vertical-rl;height:fit-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-flex;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 30]),
                ("m", [0, 0, 20, 10]),
                ("i", [0, 10, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-flex;flex-wrap:wrap;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 10]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-flex;flex-direction:column;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-grid;writing-mode:vertical-rl;height:fit-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-grid;writing-mode:vertical-rl;height:max-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div id=o style="display:grid;writing-mode:vertical-rl;height:max-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div id=o style="display:grid;writing-mode:vertical-lr;height:min-content;min-height:40px"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 40]),
                ("m", [0, 0, 10, 40]),
                ("i", [10, 0, 20, 40]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-grid;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="float:left;display:grid;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div style="position:relative"><div id=o style="position:absolute;display:grid;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:grid;grid-template-columns:auto auto;writing-mode:vertical-rl;height:min-content"><div id=m>xx xx</div><div id=i>xxx</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [0, 0, 20, 20]),
                ("i", [0, 20, 20, 30]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:grid;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:grid;height:max-content"><div id=m>x</div><div id=i>xx xx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div id=o style="display:grid;writing-mode:vertical-rl;height:min-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 30, 20]),
                ("m", [20, 0, 10, 20]),
                ("i", [0, 0, 20, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:grid;writing-mode:vertical-rl;height:fit-content"><div id=m>x</div><div id=i>xx xx</div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:flex;flex-wrap:wrap;height:fit-content"><div id=m>xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 50]),
                ("m", [0, 0, 10, 20]),
                ("i", [0, 20, 10, 30]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:flex;flex-wrap:wrap;height:min-content"><div id=m>xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 30]),
                ("m", [10, 0, 10, 20]),
                ("i", [0, 0, 10, 30]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:flex;flex-direction:column;height:fit-content"><div id=m>x</div><div id=i>xx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 20]),
                ("m", [10, 0, 10, 20]),
                ("i", [0, 0, 10, 20]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:flex;height:fit-content"><div id=m>x</div><div id=i>xx</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 30]),
                ("m", [0, 0, 10, 10]),
                ("i", [0, 10, 10, 20]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:25px"><div id=o style="display:flex;flex-wrap:wrap;height:fit-content"><div id=m>xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 30]),
                ("m", [10, 0, 10, 20]),
                ("i", [0, 0, 10, 30]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:flex;flex-wrap:wrap;height:fit-content"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            r#"<div id=o style="display:flex;writing-mode:vertical-rl;height:fit-content"><div id=m style="flex:1 0 30px">x</div><div id=i>xx</div></div>"#,
            &[
                ("o", [0, 0, 10, 50]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 20]),
            ],
        ),
        (
            r#"<div id=o style="display:inline-flex;writing-mode:vertical-rl;height:fit-content"><div id=m style="flex:1 0 30px">x</div><div id=i>xx</div></div>"#,
            &[
                ("o", [0, 0, 10, 50]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 20]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="height:fit-content"><div id=m>x</div><div id=i>xx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 20]),
                ("m", [10, 0, 10, 20]),
                ("i", [0, 0, 10, 20]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:grid;height:fit-content"><div id=m>x</div><div id=i>xx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 20]),
                ("m", [10, 0, 10, 20]),
                ("i", [0, 0, 10, 20]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:flex;height:min-content"><div id=m>x</div><div id=i>xx</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 30]),
                ("m", [0, 0, 10, 10]),
                ("i", [0, 10, 10, 20]),
            ],
        ),
        (
            r#"<div style="writing-mode:vertical-rl;height:300px"><div id=o style="display:flex;flex-wrap:wrap;height:max-content"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
    ]);
}
