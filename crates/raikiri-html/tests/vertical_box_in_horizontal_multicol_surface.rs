//! A box in a vertical writing mode inside a horizontal multicol container
//! is the root of an orthogonal flow. The multicol container does not break
//! it into columns, and it lays its own content out on its vertical axes as
//! it would anywhere else (CSS Writing Modes 4 section 7.3). Where each box
//! falls is what Chromium lays out for the same documents.

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
fn a_vertical_box_in_a_horizontal_multicol_keeps_its_own_axes() {
    assert_rows(&[
        (
            r#"<div style="columns:2;height:100px"><div id=o style="display:flex;writing-mode:vertical-rl"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 70px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="display:flex;writing-mode:vertical-rl"><div id=m>xx xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 30]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="display:flex;flex-direction:column;writing-mode:vertical-lr"><div id=m>xx xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [0, 0, 10, 50]),
                ("i", [10, 0, 10, 50]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=m>xx xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="writing-mode:vertical-rl"><div id=m>x</div><div id=i>x</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 10]),
                ("m", [10, 0, 10, 10]),
                ("i", [0, 0, 10, 10]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="writing-mode:vertical-rl"><div id=m>xx xx</div><div id=i style="height:30px">xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 50]),
                ("m", [10, 0, 10, 50]),
                ("i", [0, 0, 10, 30]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="writing-mode:vertical-rl;height:50px"><div id=m>xx xx xx xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 30, 50]),
                ("m", [10, 0, 20, 50]),
                ("i", [0, 0, 10, 50]),
            ],
        ),
        (
            r#"<div style="columns:2"><div id=o style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:60px"><div id=m style="flex:1 0 30px">x</div><div id=i style="flex:1 0 50px">x</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 60]),
                ("m", [10, 0, 10, 60]),
                ("i", [0, 0, 10, 60]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="display:inline-flex;writing-mode:vertical-rl"><div id=m>xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 10, 50]),
                ("m", [0, 0, 10, 20]),
                ("i", [0, 20, 10, 30]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="display:inline-block;writing-mode:vertical-rl"><div id=m>xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 20, 30]),
                ("m", [10, 0, 10, 30]),
                ("i", [0, 0, 10, 30]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div><div id=o style="writing-mode:vertical-rl"><div id=m style="display:flex">xx</div><div id=i>xxx</div></div></div></div>"#,
            &[
                ("o", [0, 0, 20, 30]),
                ("m", [10, 0, 10, 30]),
                ("i", [0, 0, 10, 30]),
            ],
        ),
        (
            r#"<div style="columns:2;height:100px"><div id=o style="display:flex;writing-mode:vertical-rl;padding:5px;border:2px solid"><div id=m style="margin:3px">xx</div><div id=i>xxx</div></div></div>"#,
            &[
                ("o", [0, 0, 30, 70]),
                ("m", [10, 10, 10, 20]),
                ("i", [7, 33, 16, 30]),
            ],
        ),
    ]);
}
