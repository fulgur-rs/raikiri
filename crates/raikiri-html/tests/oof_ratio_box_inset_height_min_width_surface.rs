//! An absolutely positioned box with an aspect ratio whose `top` and
//! `bottom` give it its height takes its width from that height, so its
//! automatic minimum width is the min-content width of its content, held to
//! its `max-width`, and the ratio gives its height from the width that minimum
//! raises it to. Where each box falls is what Chromium lays out for the same
//! documents.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out on one
/// page 1280px wide.
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
        "<!doctype html><style>@page{{size:1280px 1000px;margin:0}} body{{margin:0}}</style><body>{body}</body>"
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
fn an_out_of_flow_ratio_box_between_top_and_bottom_is_no_narrower_than_its_content() {
    // Content wider than the width the box states widens it, up to its
    // `max-width`, and the box is as tall as the ratio makes that width or as
    // its content, whichever is taller. `align-self: stretch` keeps the height
    // the insets give it. A `center` or `end` alignment, `left` and `right`
    // that stretch the width, and `overflow: clip` keep the width it states.
    // A height the box states does the same as the insets.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:100px;bottom:100px;align-self:stretch'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 100, 150, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:100px;bottom:100px;align-self:stretch'><div style='width:50px;height:10px'></div></div></div>",
            &[("c", [0, 100, 100, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;max-height:20%'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 150, 60])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;max-height:60px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 150, 60])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;max-height:60px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 150, 60])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:100px;bottom:100px;max-height:60px'><div style='width:150px;height:250px'></div></div></div>",
            &[("c", [0, 100, 150, 60])],
        ),
        (
            "<div style='display:grid;align-items:center;width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 150, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 150, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;width:80px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 80, 80])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;align-self:center'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 110, 80, 80])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px'><div style='width:150px;height:250px'></div></div></div>",
            &[("c", [0, 0, 150, 250])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px'><div style='width:50px;height:250px'></div></div></div>",
            &[("c", [0, 0, 80, 250])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;align-self:stretch'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 150, 300])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;left:0;right:0'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 80, 80])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;max-width:120px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 120, 120])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;height:100px;width:80px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 150, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 300, 300])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:100px;bottom:100px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 100, 150, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;padding:5px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 160, 160])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;align-self:end'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 220, 80, 80])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;align-self:stretch'></div></div>",
            &[("c", [0, 0, 80, 300])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;height:100px;width:80px;overflow:clip'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 80, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;overflow:clip'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 80, 80])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;width:80px;align-self:stretch;max-width:120px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 0, 120, 300])],
        ),
    ]);
}
