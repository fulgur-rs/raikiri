//! Repeated table headers match independently positioned page contents.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

fn raster(body: &str) -> Vec<Vec<u8>> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}body{{margin:0;background:white;font:10px/10px Ahem;color:white}}table{{border-spacing:0}}th,td{{padding:0;width:20px;vertical-align:top}}th{{height:10px;font-weight:400}}td{{height:20px}}thead{{background:red}}tbody{{background:blue}}</style>{body}"
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
    result
        .pages()
        .map(|page| {
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
                100,
                100,
            )
        })
        .collect()
}

fn assert_same(actual: &[u8], expected: &[u8], page: usize) {
    let differences = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(actual, expected)| actual != expected)
        .count();
    assert_eq!(differences, 0, "page {page}");
}

#[test]
fn every_table_page_paints_the_full_header_and_its_body_bands() {
    for opacity in ["1", "0.5"] {
        let rows = "<tr><td>X</td></tr>".repeat(9);
        let actual = raster(&format!(
            "<table style='opacity:{opacity}'><thead><tr><th>H</th></tr></thead><tbody>{rows}</tbody></table>"
        ));
        assert_eq!(actual.len(), 3);
        for (page, body_rows) in [4, 4, 1].into_iter().enumerate() {
            let body =
                "<div style='height:20px;width:20px;background:blue'>X</div>".repeat(body_rows);
            let expected = raster(&format!(
                "<div style='opacity:{opacity};width:20px'><div style='height:10px;background:red;text-align:center'>H</div>{body}</div>"
            ));
            assert_same(&actual[page], &expected[0], page);
        }
    }
}

#[test]
fn clipping_inside_a_header_moves_with_the_repeated_subtree() {
    let rows = "<tr><td></td></tr>".repeat(5);
    let actual = raster(&format!(
        "<table><thead><tr><th style='overflow:hidden'><div style='height:10px;background:lime;transform:translateY(5px)'></div></th></tr></thead><tbody>{rows}</tbody></table>"
    ));
    assert_eq!(actual.len(), 2);
    for (page, body_rows) in [4, 1].into_iter().enumerate() {
        let expected = raster(&format!(
            "<div style='width:20px;height:5px;background:red'></div><div style='width:20px;height:5px;background:lime'></div><div style='width:20px;height:{}px;background:blue'></div>",
            body_rows * 20
        ));
        assert_same(&actual[page], &expected[0], page);
    }
}

#[test]
fn an_initial_header_near_the_page_bottom_moves_with_its_first_row() {
    let rows = "<tr><td>X</td></tr>".repeat(5);
    let empty = raster("<div style='height:85px'></div>");
    for spacer in [85, 95] {
        let actual = raster(&format!(
            "<div style='height:{spacer}px'></div><table style='opacity:0.5'><thead><tr><th>H</th></tr></thead><tbody>{rows}</tbody></table>"
        ));
        assert_eq!(actual.len(), 3);
        assert_same(&actual[0], &empty[0], 0);
        for (page, count) in [(1, 4), (2, 1)] {
            let body = "<div style='height:20px;width:20px;background:blue'>X</div>".repeat(count);
            let expected = raster(&format!(
                "<div style='opacity:0.5;width:20px'><div style='height:10px;background:red;text-align:center'>H</div>{body}</div>"
            ));
            assert_same(&actual[page], &expected[0], page);
        }
    }
}

#[test]
fn fixed_descendants_still_paint_after_the_table_has_ended() {
    let fixed = "<span style='position:fixed;top:2px;left:30px;width:10px;height:10px;color:black'>F</span>";
    let rows = "<tr><td>X</td></tr>".repeat(5);
    let actual = raster(&format!(
        "<table><thead><tr><th>H{fixed}</th></tr></thead><tbody>{rows}</tbody></table><div style='height:120px'></div>"
    ));
    assert_eq!(actual.len(), 3);
    let expected = raster(fixed);
    assert_same(&actual[2], &expected[0], 2);
}
