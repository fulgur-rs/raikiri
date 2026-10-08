use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

fn raster(body: &str) -> Vec<u8> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:50px 50px;margin:0}}body{{margin:0;background:white;font:10px/10px Ahem}}table{{border-spacing:0}}td{{padding:0;width:20px;height:20px;vertical-align:top}}td>div{{width:10px;height:10px;background:blue}}</style>{body}"
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
    let page = result.page(0).unwrap();
    let (doc, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    let mut budget = raikiri_dom::CounterSnapshotBudget::default();
    raikiri_paint::paint_single_page_with_origin(
        &mut scene,
        doc,
        cascade,
        page_box,
        origin,
        &mut budget,
    )
    .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        50,
        50,
    )
}

#[test]
fn hidden_explicit_rows_and_groups_do_not_paint_their_backgrounds() {
    let reference = raster("<div style='width:10px;height:10px;background:blue'></div>");
    for owner in ["tr", "tbody", "thead", "tfoot"] {
        let row_style = if owner == "tr" {
            "visibility:hidden;background:red"
        } else {
            ""
        };
        let group_style = if owner != "tr" {
            "visibility:hidden;background:red"
        } else {
            ""
        };
        let group = if owner == "tr" { "tbody" } else { owner };
        let actual = raster(&format!(
            "<table><{group} style='{group_style}'><tr style='{row_style}'><td style='visibility:visible'><div></div></td></tr></{group}></table>"
        ));
        let differences = actual
            .chunks_exact(4)
            .zip(reference.chunks_exact(4))
            .filter(|(actual, expected)| actual != expected)
            .count();
        assert_eq!(differences, 0, "hidden owner: {owner}");
    }
}

#[test]
fn visible_row_backgrounds_still_paint_around_visible_cell_content() {
    let reference = raster(
        "<div style='width:20px;height:20px;background:red'><div style='width:10px;height:10px;background:blue'></div></div>",
    );
    let actual = raster("<table><tr style='background:red'><td><div></div></td></tr></table>");
    let differences = actual
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .filter(|(actual, expected)| actual != expected)
        .count();
    assert_eq!(differences, 0);
}
