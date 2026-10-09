//! Native multicolumn placements match independently positioned literal controls.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{DecodedImage, ImagePixelSource, LayoutConfig, PageDefaults};
use std::sync::Arc;

struct NoPixels;
impl ImagePixelSource for NoPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        None
    }
}

fn raster(body: &str, css: &str) -> Vec<u8> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:180px 160px;margin:0}}body{{margin:0;background:white;font:20px/20px Ahem}}p{{margin:0}}.mc{{width:100px;column-count:2;column-gap:20px;orphans:1;widows:1}}{css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(document) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    assert_eq!(document.page_count(), 1);
    let page = document.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(
        &mut scene,
        document,
        cascade,
        page_box,
        &NoPixels,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        180,
        160,
    )
}

fn compare(body: &str, css: &str, reference: &str) {
    let actual = raster(body, css);
    let expected = raster(reference, "");
    assert_eq!(
        actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count(),
        0
    );
}

#[test]
fn balanced_line_columns_match_absolute_literal_text() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

#[test]
fn a_padded_and_bordered_column_container_uses_its_content_origin() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        ".mc{padding:4px 5px;border:1px solid black;background:lime}",
        "<div style='position:absolute;left:0;top:0;box-sizing:border-box;width:112px;height:50px;border:1px solid black;background:lime'></div><div style='position:absolute;left:6px;top:5px'>A<br>B</div><div style='position:absolute;left:66px;top:5px'>C<br>D</div><div style='position:absolute;left:0;top:50px'>E</div>",
    );
}

#[test]
fn a_border_box_column_container_balances_inside_its_insets() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        ".mc{box-sizing:border-box;padding:4px 5px;border:1px solid black;background:lime}",
        "<div style='position:absolute;left:0;top:0;box-sizing:border-box;width:100px;height:50px;border:1px solid black;background:lime'></div><div style='position:absolute;left:6px;top:5px'>A<br>B</div><div style='position:absolute;left:60px;top:5px'>C<br>D</div><div style='position:absolute;left:0;top:50px'>E</div>",
    );
}

#[test]
fn a_padded_column_minimum_reserves_the_full_border_box() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        ".mc{min-height:60px;padding:4px 5px;border:1px solid black;background:lime}",
        "<div style='position:absolute;left:0;top:0;box-sizing:border-box;width:112px;height:70px;border:1px solid black;background:lime'></div><div style='position:absolute;left:6px;top:5px'>A<br>B</div><div style='position:absolute;left:66px;top:5px'>C<br>D</div><div style='position:absolute;left:0;top:70px'>E</div>",
    );
}

#[test]
fn a_fragmented_child_paragraph_matches_absolute_literal_text() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn wrapped_child_words_match_literal_lines_at_the_column_start() {
    compare(
        "<div class=mc><p>aaaa bbbb cccc dddd</p></div><p>E</p>",
        "body{font-size:10px;line-height:10px}.mc{column-gap:10px}",
        "<div style='position:absolute;left:0;top:0;font:10px/10px Ahem'>aaaa<br>bbbb</div><div style='position:absolute;left:55px;top:0;font:10px/10px Ahem'>cccc<br>dddd</div><div style='position:absolute;left:0;top:20px;font:10px/10px Ahem'>E</div>",
    );
}

#[test]
fn a_child_paragraph_in_three_columns_matches_literal_positions() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D<br>E<br>F</p></div><p>G</p>",
        ".mc{width:160px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:120px;top:0'>E<br>F</div><div style='position:absolute;left:0;top:40px'>G</div>",
    );
}

#[test]
fn an_uneven_child_paragraph_keeps_its_final_lines_and_following_flow() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D<br>E</p></div><p>F</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>E</div><div style='position:absolute;left:0;top:60px'>F</div>",
    );
}

#[test]
fn a_child_paragraph_background_covers_both_column_fragments() {
    compare(
        "<div class=mc><p style='background:lime'>A<br>B<br>C<br>D</p></div><p>E</p>",
        "",
        "<div style='position:absolute;left:0;top:0;width:40px;background:lime'>A<br>B</div><div style='position:absolute;left:60px;top:0;width:40px;background:lime'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_child_paragraph_bottom_margin_advances_following_flow_once() {
    compare(
        "<div class=mc><p style='margin-bottom:10px'>A<br>B<br>C<br>D</p></div><p>E</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:50px'>E</div>",
    );
}

#[test]
fn a_parent_minimum_preserves_space_after_the_balanced_child_paragraph() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        ".mc{min-height:60px;background:yellow}",
        "<div style='position:absolute;left:0;top:0;width:100px;height:60px;background:yellow'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:60px'>E</div>",
    );
}

#[test]
fn separate_column_boxes_match_absolute_literal_rectangles() {
    compare(
        "<div class=mc><p style='background:red'>A</p><p style='background:lime'>B</p><p style='background:blue'>C</p><p style='background:yellow'>D</p></div>",
        "body{color:transparent}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:20px;background:red'></div><div style='position:absolute;left:0;top:20px;width:40px;height:20px;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:20px;background:blue'></div><div style='position:absolute;left:60px;top:20px;width:40px;height:20px;background:yellow'></div>",
    );
}
