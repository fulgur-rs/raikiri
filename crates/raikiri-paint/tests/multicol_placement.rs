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
    raster_at(body, css, None)
}

/// `raster` for a device with `device_scale` device pixels per CSS pixel,
/// whose clips lie on its pixels, or for a vector output with `None`.
fn raster_at(body: &str, css: &str, device_scale: Option<f32>) -> Vec<u8> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .fonts(fonts)
        .border_width_device_scale(device_scale);
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
        &mut scene, document, cascade, page_box, &NoPixels,
    )
    .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        180,
        160,
    )
}

fn compare(body: &str, css: &str, reference: &str) {
    compare_at(body, css, reference, None);
}

fn compare_at(body: &str, css: &str, reference: &str, device_scale: Option<f32>) {
    let actual = raster_at(body, css, device_scale);
    let expected = raster_at(reference, "", device_scale);
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
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:45px'>E</div>",
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

#[test]
fn a_fixed_height_column_container_splits_a_nested_plain_block() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_fixed_height_plain_chain_balances_its_lines_in_the_columns() {
    // The columns are as tall as the lines need, and the container keeps its
    // height, so the paragraph after it starts below the whole height.
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:100px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:100px'>E</div>",
    );
    compare(
        "<div class=mc><div><div><p>A<br>B<br>C</p></div></div></div><p>E</p>",
        ".mc{height:100px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C</div><div style='position:absolute;left:0;top:100px'>E</div>",
    );
}

#[test]
fn a_fixed_height_plain_chain_fills_its_columns_in_turn_with_column_fill_auto() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div>",
        ".mc{height:100px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C<br>D</div>",
    );
}

#[test]
fn a_balanced_fixed_height_chain_paints_its_wrapper_background_in_each_column() {
    compare(
        "<div class=mc><div style='background:lime'><p>A<br>B<br>C<br>D</p></div></div>",
        ".mc{height:100px}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:40px;background:lime'>A<br>B</div><div style='position:absolute;left:60px;top:0;width:40px;height:40px;background:lime'>C<br>D</div>",
    );
}

#[test]
fn a_balanced_fixed_height_chain_paints_its_paragraph_background_in_each_column() {
    compare(
        "<div class=mc><div><p style='background:lime'>A<br>B<br>C<br>D</p></div></div>",
        ".mc{height:100px}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:40px;background:lime'>A<br>B</div><div style='position:absolute;left:60px;top:0;width:40px;height:40px;background:lime'>C<br>D</div>",
    );
}

#[test]
fn boxes_with_text_and_a_height_are_placed_whole_in_balanced_columns() {
    compare(
        "<div class=mc><div style='height:40px;background:red'>A</div><div style='height:40px;background:lime'>B</div><div style='height:40px;background:blue'>C</div><div style='height:40px;background:yellow'>D</div></div><p>E</p>",
        ".mc{height:100px}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:40px;background:red'>A</div><div style='position:absolute;left:0;top:40px;width:40px;height:40px;background:lime'>B</div><div style='position:absolute;left:60px;top:0;width:40px;height:40px;background:blue'>C</div><div style='position:absolute;left:60px;top:40px;width:40px;height:40px;background:yellow'>D</div><div style='position:absolute;left:0;top:100px'>E</div>",
    );
    // The text wraps at the width of a column.
    compare(
        "<div class=mc><div style='height:40px;background:red'>A B</div><div style='height:40px;background:lime'>C D</div></div>",
        ".mc{height:100px}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:40px;background:red'>A<br>B</div><div style='position:absolute;left:60px;top:0;width:40px;height:40px;background:lime'>C<br>D</div>",
    );
}

#[test]
fn a_fixed_height_plain_chain_uses_three_columns_below_a_prefix() {
    compare(
        "<p>X</p><div class=mc><div><div><p>A<br>B<br>C<br>D<br>E<br>F</p></div></div></div><p>G</p>",
        ".mc{width:160px;height:40px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>X</div><div style='position:absolute;left:0;top:20px'>A<br>B</div><div style='position:absolute;left:60px;top:20px'>C<br>D</div><div style='position:absolute;left:120px;top:20px'>E<br>F</div><div style='position:absolute;left:0;top:60px'>G</div>",
    );
}

#[test]
fn a_fixed_height_direct_inline_root_preserves_its_columns() {
    compare(
        "<div class=mc><span>A<br>B<br>C<br>D</span></div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_fixed_height_plain_wrapper_respects_the_parent_content_origin() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px;padding:4px 5px;border:1px solid black;background:lime}",
        "<div style='position:absolute;left:0;top:0;box-sizing:border-box;width:112px;height:50px;border:1px solid black;background:lime'></div><div style='position:absolute;left:6px;top:5px'>A<br>B</div><div style='position:absolute;left:66px;top:5px'>C<br>D</div><div style='position:absolute;left:0;top:50px'>E</div>",
    );
}

#[test]
fn a_fixed_height_border_box_wrapper_uses_the_inner_column_width() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{box-sizing:border-box;height:50px;padding:4px 5px;border:1px solid black;background:lime}",
        "<div style='position:absolute;left:0;top:0;box-sizing:border-box;width:100px;height:50px;border:1px solid black;background:lime'></div><div style='position:absolute;left:6px;top:5px'>A<br>B</div><div style='position:absolute;left:60px;top:5px'>C<br>D</div><div style='position:absolute;left:0;top:50px'>E</div>",
    );
}

#[test]
fn a_deep_plain_wrapper_keeps_column_clips_below_a_prefix() {
    compare(
        "<p>X</p><div class=mc><div><div><p>A<br>B<br>C<br>D<br>E<br>F</p></div></div></div><p>G</p>",
        ".mc{width:160px;height:40px;column-count:3;padding:4px 5px;border:1px solid black;background:lime}.mc p{overflow:hidden}",
        "<div style='position:absolute;left:0;top:0'>X</div><div style='position:absolute;left:0;top:20px;box-sizing:border-box;width:172px;height:50px;border:1px solid black;background:lime'></div><div style='position:absolute;left:6px;top:25px'>A<br>B</div><div style='position:absolute;left:66px;top:25px'>C<br>D</div><div style='position:absolute;left:126px;top:25px'>E<br>F</div><div style='position:absolute;left:0;top:70px'>G</div>",
    );
}

#[test]
fn a_fixed_height_wrapper_retains_its_authored_width() {
    compare(
        "<div class=mc><div style='width:80px'><p>A A</p></div></div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:0;top:0;width:80px'>A A</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_fixed_height_wrapper_splits_at_its_authored_width() {
    for sizing in [
        "width:80px",
        "min-width:80px",
        "width:200%",
        "width:calc(200%)",
    ] {
        compare(
            &format!(
                "<div class=mc><div style='{sizing}'><p>A A<br>B B<br>C C<br>D D</p></div></div><p>E</p>"
            ),
            ".mc{height:40px}",
            "<div style='position:absolute;left:0;top:0;width:80px'>A A<br>B B</div><div style='position:absolute;left:60px;top:0;width:80px'>C C<br>D D</div><div style='position:absolute;left:0;top:40px'>E</div>",
        );
    }
}

#[test]
fn a_fixed_height_wrapper_consumes_its_initial_margin_once() {
    compare(
        "<div class=mc><div style='overflow:hidden'><p style='margin-top:20px'>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:60px}",
        "<div style='position:absolute;left:0;top:20px'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:60px'>E</div>",
    );
}

#[test]
fn a_fixed_height_wrapper_margin_generates_a_visible_overflow_column() {
    compare(
        "<div class=mc><div style='overflow:hidden'><p style='margin-top:20px'>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:0;top:20px'>A</div><div style='position:absolute;left:60px;top:0'>B<br>C</div><div style='position:absolute;left:120px;top:0'>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_constrained_wrapper_centers_auto_margins_in_each_column() {
    compare(
        "<div class=mc><div style='width:20px;margin-left:auto;margin-right:auto'><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:10px;top:0'>A<br>B</div><div style='position:absolute;left:70px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_constrained_wrapper_resolves_its_percentage_margin_at_column_width() {
    compare(
        "<div class=mc><div style='margin-top:50%'><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:60px}",
        "<div style='position:absolute;left:0;top:20px'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:60px'>E</div>",
    );
}

#[test]
fn a_constrained_wrapper_keeps_possible_paragraph_break_minima() {
    compare(
        "<div class=mc><div style='width:80px'><p style='orphans:3;widows:3;background:lime'>A<br>B<br>C<br>D<br>E<br>F</p></div></div><p>G</p>",
        ".mc{height:100px}",
        "<div style='position:absolute;left:0;top:0;width:80px;height:100px;background:lime'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0;width:80px;height:60px;background:lime'>D<br>E<br>F</div><div style='position:absolute;left:0;top:100px'>G</div>",
    );
}

#[test]
fn a_constrained_wrapper_relaxes_impossible_line_count_minima() {
    compare(
        "<div class=mc><div style='margin-top:20px'><p style='orphans:4;widows:4'>A<br>B<br>C<br>D<br>E<br>F</p></div></div><p>G</p>",
        ".mc{height:80px}",
        "<div style='position:absolute;left:0;top:20px'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>E<br>F</div><div style='position:absolute;left:0;top:80px'>G</div>",
    );
}

#[test]
fn a_constrained_wrapper_keeps_a_line_below_a_tall_initial_margin_visible() {
    compare(
        "<div class=mc><div style='overflow:hidden'><p style='margin-top:100px'>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:0;top:100px'>A</div><div style='position:absolute;left:60px;top:0'>B<br>C</div><div style='position:absolute;left:120px;top:0'>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_constrained_wrapper_preserves_widows_in_each_middle_column() {
    compare(
        "<div class=mc><div style='width:80px'><p style='orphans:2;widows:4'>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H<br>I<br>J<br>K<br>L</p></div></div><p>M</p>",
        ".mc{height:100px;width:160px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C<br>D</div><div style='position:absolute;left:60px;top:0'>E<br>F<br>G<br>H</div><div style='position:absolute;left:120px;top:0'>I<br>J<br>K<br>L</div><div style='position:absolute;left:0;top:100px'>M</div>",
    );
}

#[test]
fn a_constrained_wrapper_refills_middle_orphans_when_lending_to_final_widows() {
    compare(
        "<div class=mc><div style='width:80px'><p style='orphans:4;widows:4'>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H<br>I<br>J<br>K<br>L</p></div></div><p>M</p>",
        ".mc{height:100px;width:160px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C<br>D</div><div style='position:absolute;left:60px;top:0'>E<br>F<br>G<br>H</div><div style='position:absolute;left:120px;top:0'>I<br>J<br>K<br>L</div><div style='position:absolute;left:0;top:100px'>M</div>",
    );
}

#[test]
fn a_constrained_wrapper_redistributes_variable_height_lines_to_keep_widows() {
    compare(
        "<div class=mc><div style='width:80px'><p style='orphans:1;widows:3'><span style='line-height:20px'>A</span><br><span style='line-height:5px'>B</span><br><span style='line-height:10px'>C</span><br><span style='line-height:15px'>D</span><br><span style='line-height:20px'>E</span><br><span style='line-height:10px'>F</span><br><span style='line-height:5px'>G</span><br><span style='line-height:5px'>H</span></p></div></div><p>I</p>",
        "body{font:5px/5px Ahem}.mc{height:40px;width:160px;column-count:3}",
        "<style>body{font:5px/5px Ahem}</style><div style='position:absolute;left:0;top:0'><span style='line-height:20px'>A</span></div><div style='position:absolute;left:60px;top:0'><span style='line-height:5px'>B</span><br><span style='line-height:10px'>C</span><br><span style='line-height:15px'>D</span></div><div style='position:absolute;left:120px;top:0'><span style='line-height:20px'>E</span><br><span style='line-height:10px'>F</span><br><span style='line-height:5px'>G</span><br><span style='line-height:5px'>H</span></div><div style='position:absolute;left:0;top:40px'>I</div>",
    );
}

#[test]
fn a_constrained_wrapper_keeps_lines_when_all_break_minima_cannot_fit() {
    compare(
        "<div class=mc><div style='width:80px'><p style='orphans:4;widows:5'>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H<br>I<br>J<br>K<br>L</p></div></div><p>M</p>",
        ".mc{height:100px;width:160px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C<br>D</div><div style='position:absolute;left:60px;top:0'>E<br>F<br>G</div><div style='position:absolute;left:120px;top:0'>H<br>I<br>J<br>K<br>L</div><div style='position:absolute;left:0;top:100px'>M</div>",
    );
}

#[test]
fn a_constrained_wrapper_cannot_lend_a_line_beyond_the_final_column_height() {
    compare(
        "<div class=mc><div style='width:80px'><p style='orphans:1;widows:4'>A<br>B<br>C<br>D<br>E<br>F</p></div></div><p>G</p>",
        ".mc{height:60px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>E<br>F</div><div style='position:absolute;left:0;top:60px'>G</div>",
    );
}

#[test]
fn a_constrained_wrapper_preserves_visible_unbreakable_inline_overflow() {
    compare(
        "<div class=mc><div style='width:20px'><p style='white-space:nowrap'>ABCDEF</p></div></div><p>G</p>",
        ".mc{height:60px}",
        "<div style='position:absolute;left:0;top:0;white-space:nowrap'>ABCDEF</div><div style='position:absolute;left:0;top:60px'>G</div>",
    );
}

#[test]
fn a_spanner_leaves_overflow_wider_than_a_column_visible() {
    // The block is wider than the 80px column; its text reaches 100px with or
    // without a spanner between the two groups.
    compare(
        "<div class=mc><div style='width:100px;white-space:nowrap'>ABCDE</div><div style='column-span:all'>x</div><div style='width:100px;white-space:nowrap'>ABCDE</div></div>",
        ".mc{width:80px;column-count:auto;column-width:80px}",
        "<div style='position:absolute;left:0;top:0;white-space:nowrap'>ABCDE</div><div style='position:absolute;left:0;top:20px'>x</div><div style='position:absolute;left:0;top:40px;white-space:nowrap'>ABCDE</div>",
    );
}

#[test]
fn a_spanner_leaves_a_line_that_cannot_wrap_visible_past_its_column() {
    compare(
        "<div class=mc><p style='white-space:nowrap'>ABCDEFGH</p><div style='column-span:all'>x</div><p>B</p></div>",
        ".mc{width:80px;column-count:auto;column-width:80px}",
        "<div style='position:absolute;left:0;top:0;white-space:nowrap'>ABCDEFGH</div><div style='position:absolute;left:0;top:20px'>x</div><div style='position:absolute;left:0;top:40px'>B</div>",
    );
}

#[test]
fn a_spanner_leaves_an_inline_box_visible_past_the_glyphs_it_wraps() {
    // The padding of the span reaches past its glyph run, and past the column.
    let span = "<span style='background:yellow;padding:0 20px'>ABCD</span>";
    compare(
        &format!(
            "<div class=mc><p style='white-space:nowrap'>{span}</p><div style='column-span:all'>x</div><p>B</p></div>"
        ),
        ".mc{width:80px;column-count:auto;column-width:80px}",
        &format!(
            "<div style='position:absolute;left:0;top:0;white-space:nowrap'>{span}</div><div style='position:absolute;left:0;top:20px'>x</div><div style='position:absolute;left:0;top:40px'>B</div>"
        ),
    );
}

#[test]
fn a_spanner_leaves_a_ruby_base_wider_than_its_column_visible() {
    let ruby = "<ruby>ABCDEF<rt>x</rt></ruby>";
    compare(
        &format!(
            "<div class=mc><p>B</p><div style='column-span:all'>y</div><p style='white-space:nowrap'>{ruby}</p></div>"
        ),
        ".mc{width:80px;column-count:auto;column-width:80px}",
        &format!(
            "<div style='position:absolute;left:0;top:0'>B</div><div style='position:absolute;left:0;top:20px'>y</div><div style='position:absolute;left:0;top:40px;white-space:nowrap'>{ruby}</div>"
        ),
    );
}

#[test]
fn a_spanner_leaves_a_padded_line_visible_past_its_column() {
    // The lines start inside the border and padding of the paragraph.
    let p =
        "<p style='white-space:nowrap;padding-left:30px;border-left:5px solid blue'>ABCDEFGH</p>";
    compare(
        &format!("<div class=mc>{p}<div style='column-span:all'>x</div><p>B</p></div>"),
        ".mc{width:80px;column-count:auto;column-width:80px}",
        &format!(
            "<div style='position:absolute;left:0;top:0;width:80px'>{p}</div><div style='position:absolute;left:0;top:20px'>x</div><div style='position:absolute;left:0;top:40px'>B</div>"
        ),
    );
}

#[test]
fn a_spanner_leaves_a_right_to_left_line_visible_past_the_left_edge_of_its_column() {
    let p = "<p dir=rtl style='white-space:nowrap'>ABCDEFGH</p>";
    compare(
        &format!("<div class=mc>{p}<div style='column-span:all'>x</div><p>B</p></div>"),
        ".mc{margin-left:100px;width:80px;column-count:auto;column-width:80px}",
        &format!(
            "<div style='position:absolute;left:100px;top:0;width:80px'>{p}</div><div style='position:absolute;left:100px;top:20px'>x</div><div style='position:absolute;left:100px;top:40px'>B</div>"
        ),
    );
}

#[test]
fn a_spanner_leaves_the_shadows_of_a_line_visible_past_its_column() {
    // The shadows fall 90px to the right, past the 80px column.
    for p in [
        "<p style='text-shadow:90px 0 red'>AB</p>",
        "<p>A<span style='box-shadow:90px 0 blue'>B</span></p>",
    ] {
        compare(
            &format!("<div class=mc>{p}<div style='column-span:all'>x</div><p>B</p></div>"),
            ".mc{width:80px;column-count:auto;column-width:80px}",
            &format!(
                "<div style='position:absolute;left:0;top:0;width:80px'>{p}</div><div style='position:absolute;left:0;top:20px'>x</div><div style='position:absolute;left:0;top:40px'>B</div>"
            ),
        );
    }
}

#[test]
fn a_spanner_leaves_the_outline_of_a_paragraph_visible_past_its_column_group() {
    // The outline reaches 10px past the first line above and past the last
    // line below, where its column group ends.
    compare(
        "<div class=mc><p>A</p><div style='column-span:all'>x</div><p style='outline:10px solid red'>B</p></div>",
        ".mc{width:80px;column-count:auto;column-width:80px}",
        "<div style='position:absolute;left:0;top:0;width:80px'>A</div><div style='position:absolute;left:0;top:20px'>x</div><div style='position:absolute;left:0;top:40px;width:80px;outline:10px solid red'>B</div>",
    );
}

/// The lines `first` and `second`, `row_height` apart, in the column at `left`
/// of a container whose first line is `top`.
fn column_lines(left: i32, top: i32, first: &str, second: &str) -> String {
    format!("<div style='position:absolute;left:{left}px;top:{top}px'>{first}<br>{second}</div>")
}

#[test]
fn bare_lines_around_a_spanner_fill_their_columns_group_by_group() {
    // The container holds the lines itself, with no paragraph around them: the
    // lines before and after the spanner fill two columns each, and the whole
    // fits the page, where one column of all the lines did not.
    let spanner = "<div style='column-span:all;height:20px;background:lime'></div>";
    compare(
        &format!("<div class=mc>A<br>B<br>C<br>D{spanner}E<br>F<br>G<br>H</div>"),
        "",
        &format!(
            "{}{}<div style='position:absolute;left:0;top:40px;width:100px;height:20px;background:lime'></div>{}{}",
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
            column_lines(0, 60, "E", "F"),
            column_lines(60, 60, "G", "H"),
        ),
    );
}

#[test]
fn bare_lines_around_a_spanner_match_the_lines_of_paragraphs() {
    let spanner = "<div style='column-span:all;height:20px;background:lime'></div>";
    compare(
        &format!("<div class=mc>A<br>B<br>C<br>D{spanner}E<br>F<br>G</div>"),
        "",
        &format!("<div class=mc><p>A<br>B<br>C<br>D</p>{spanner}<p>E<br>F<br>G</p></div>"),
    );
}

#[test]
fn bare_lines_around_a_spanner_start_inside_the_border_and_padding_of_the_container() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D<div style='column-span:all;height:20px;background:yellow'></div>E<br>F<br>G<br>H</div>",
        ".mc{padding:10px;border:5px solid black;background:lime}",
        &format!(
            "<div style='position:absolute;left:0;top:0;box-sizing:border-box;width:130px;height:130px;border:5px solid black;background:lime'></div>{}{}<div style='position:absolute;left:15px;top:55px;width:100px;height:20px;background:yellow'></div>{}{}",
            column_lines(15, 15, "A", "B"),
            column_lines(75, 15, "C", "D"),
            column_lines(15, 75, "E", "F"),
            column_lines(75, 75, "G", "H"),
        ),
    );
}

#[test]
fn bare_lines_before_a_spanner_balance_under_column_fill_auto() {
    // The lines after the last spanner fill the first column, as they would
    // without a spanner.
    compare(
        "<div class=mc>A<br>B<br>C<br>D<div style='column-span:all;height:20px'></div>E<br>F<br>G<br>H</div>",
        ".mc{column-fill:auto}",
        &format!(
            "{}{}<div style='position:absolute;left:0;top:60px'>E<br>F<br>G<br>H</div>",
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
        ),
    );
}

#[test]
fn bare_lines_between_spanners_follow_their_margins() {
    // The margins between the two spanners collapse into the larger one.
    let spanners = "<div style='column-span:all;height:20px;margin-bottom:10px;background:lime'></div><div style='column-span:all;height:20px;margin-top:20px;background:yellow'></div>";
    compare(
        &format!("<div class=mc>A<br>B<br>C<br>D{spanners}E<br>F<br>G<br>H</div>"),
        "",
        &format!(
            "{}{}<div style='position:absolute;left:0;top:40px;width:100px;height:20px;background:lime'></div><div style='position:absolute;left:0;top:80px;width:100px;height:20px;background:yellow'></div>{}{}",
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
            column_lines(0, 100, "E", "F"),
            column_lines(60, 100, "G", "H"),
        ),
    );
}

#[test]
fn a_spanner_between_bare_lines_is_laid_out_across_the_columns() {
    // Its own lines wrap at the width of the container, not of a column.
    compare(
        "<div class=mc>A<br>B<br>C<br>D<div style='column-span:all;background:lime'>XX XX XX XX XX XX</div>E<br>F<br>G<br>H</div>",
        "",
        &format!(
            "{}{}<div style='position:absolute;left:0;top:40px;width:100px;background:lime'>XX XX XX XX XX XX</div>{}{}",
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
            column_lines(0, 100, "E", "F"),
            column_lines(60, 100, "G", "H"),
        ),
    );
}

#[test]
fn bare_lines_around_a_spanner_fill_right_to_left_columns_from_the_right() {
    let rtl = |left: i32, top: i32, first: &str, second: &str| {
        format!(
            "<div style='position:absolute;left:{left}px;top:{top}px;width:40px;direction:rtl'>{first}<br>{second}</div>"
        )
    };
    compare(
        "<div class=mc>A<br>B<br>C<br>D<div style='column-span:all;height:20px;background:lime'></div>E<br>F<br>G<br>H</div>",
        ".mc{direction:rtl}",
        &format!(
            "{}{}<div style='position:absolute;left:0;top:40px;width:100px;height:20px;background:lime'></div>{}{}",
            rtl(60, 0, "A", "B"),
            rtl(0, 0, "C", "D"),
            rtl(60, 60, "E", "F"),
            rtl(0, 60, "G", "H"),
        ),
    );
}

#[test]
fn bare_lines_around_a_spanner_keep_the_boxes_in_their_lines_in_their_columns() {
    // The box of the second group moves with its line to the column and
    // the group it is in.
    let boxed = "<span style='display:inline-block;background:red'>E</span>";
    compare(
        &format!(
            "<div class=mc>A<br>B<br>C<br>D<div style='column-span:all;height:20px'></div>{boxed}<br>F<br>G<br>H</div>"
        ),
        "",
        &format!(
            "{}{}{}{}",
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
            column_lines(0, 60, boxed, "F"),
            column_lines(60, 60, "G", "H"),
        ),
    );
}

#[test]
fn the_rules_between_the_columns_of_bare_lines_stop_at_a_spanner() {
    let rule = |top: i32| {
        format!(
            "<div style='position:absolute;left:49px;top:{top}px;width:2px;height:40px;background:black'></div>"
        )
    };
    compare(
        "<div class=mc>A<br>B<br>C<br>D<div style='column-span:all;height:20px'></div>E<br>F<br>G<br>H</div>",
        ".mc{column-rule:2px solid black}",
        &format!(
            "{}{}{}{}{}{}",
            rule(0),
            rule(60),
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
            column_lines(0, 60, "E", "F"),
            column_lines(60, 60, "G", "H"),
        ),
    );
}

#[test]
fn the_rules_of_bare_lines_start_inside_the_border_and_padding_of_the_container() {
    let rule = |top: i32| {
        format!(
            "<div style='position:absolute;left:64px;top:{top}px;width:2px;height:40px;background:black'></div>"
        )
    };
    compare(
        "<div class=mc>A<br>B<br>C<br>D<div style='column-span:all;height:20px'></div>E<br>F<br>G<br>H</div>",
        ".mc{padding:10px;border:5px solid transparent;column-rule:2px solid black}",
        &format!(
            "{}{}{}{}{}{}",
            rule(15),
            rule(75),
            column_lines(15, 15, "A", "B"),
            column_lines(75, 15, "C", "D"),
            column_lines(15, 75, "E", "F"),
            column_lines(75, 75, "G", "H"),
        ),
    );
}

#[test]
fn bare_lines_around_a_spanner_stay_in_their_groups_when_a_flex_container_measures_them() {
    // The flex container sizes the item before it places it.
    compare(
        "<div style='display:flex'><div class=mc>A<br>B<br>C<br>D<div style='column-span:all;height:20px;background:lime'></div>E<br>F<br>G<br>H</div></div><p>I</p>",
        "",
        &format!(
            "{}{}<div style='position:absolute;left:0;top:40px;width:100px;height:20px;background:lime'></div>{}{}<div style='position:absolute;left:0;top:100px'>I</div>",
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
            column_lines(0, 60, "E", "F"),
            column_lines(60, 60, "G", "H"),
        ),
    );
}

#[test]
fn bare_lines_around_a_spanner_in_a_wrapper_fill_their_columns_group_by_group() {
    // The wrapper makes no box of its own, so its lines are the container's.
    compare(
        "<div class=mc><div>A<br>B<br>C<br>D<div style='column-span:all;height:20px;background:lime'></div>E<br>F<br>G<br>H</div></div>",
        "",
        &format!(
            "{}{}<div style='position:absolute;left:0;top:40px;width:100px;height:20px;background:lime'></div>{}{}",
            column_lines(0, 0, "A", "B"),
            column_lines(60, 0, "C", "D"),
            column_lines(0, 60, "E", "F"),
            column_lines(60, 60, "G", "H"),
        ),
    );
}

#[test]
fn a_box_with_a_block_border_slices_it_across_the_columns() {
    // The border is where the box starts or ends, not at the break.
    compare(
        "<div class=mc><div style='height:150px;border-top:1px solid;background:lime'></div></div>",
        ".mc{height:100px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:99px;border-top:1px solid;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:51px;background:lime'></div>",
    );
    compare(
        "<div class=mc><div style='height:100px;padding:10px 0;border-bottom:2px solid;background:lime'></div></div>",
        ".mc{height:100px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:100px;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:20px;border-bottom:2px solid;background:lime'></div>",
    );
}

#[test]
fn a_box_with_block_borders_cut_into_three_pieces_has_borders_on_the_outer_edges_only() {
    compare(
        "<div class=mc><div style='height:249px;border:solid;border-width:1px 0 2px;background:lime'></div></div>",
        ".mc{width:160px;column-count:3;height:100px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:99px;border-top:1px solid;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:100px;background:lime'></div><div style='position:absolute;left:120px;top:0;width:40px;height:50px;border-bottom:2px solid;background:lime'></div>",
    );
}

#[test]
fn a_wrapped_box_with_a_block_border_slices_it_across_the_columns() {
    compare(
        "<div class=mc><div><div style='height:150px;border-top:1px solid;background:lime'></div></div></div>",
        ".mc{height:100px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:99px;border-top:1px solid;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:51px;background:lime'></div>",
    );
}

#[test]
fn a_constrained_wrapper_background_includes_its_last_child_margin() {
    compare(
        "<div class=mc><div style='width:80px;overflow:hidden;background:lime'><p style='margin-bottom:20px'>A</p></div></div><p>B</p>",
        ".mc{height:60px}",
        "<div style='position:absolute;left:0;top:0;width:80px;height:40px;background:lime'>A</div><div style='position:absolute;left:0;top:60px'>B</div>",
    );
}

#[test]
fn a_constrained_wrapper_keeps_tall_line_ink_beyond_a_short_column() {
    compare(
        "<div class=mc><div style='width:80px;overflow:hidden'><p>A<br>B<br>C</p></div></div><p>D</p>",
        ".mc{height:10px;width:160px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>A</div><div style='position:absolute;left:60px;top:0'>B</div><div style='position:absolute;left:120px;top:0'>C</div><div style='position:absolute;left:0;top:10px'>D</div>",
    );
}

#[test]
fn a_constrained_deep_wrapper_keeps_parent_insets_and_each_column_clip() {
    compare(
        "<p>X</p><div class=mc><div style='width:80px;overflow:hidden'><div><p style='margin-top:20px'>A<br>B<br>C<br>D</p></div></div></div><p>E</p>",
        ".mc{height:40px;width:160px;column-count:3;padding:4px 5px;border:1px solid black;background:lime}",
        "<div style='position:absolute;left:0;top:0'>X</div><div style='position:absolute;left:0;top:20px;box-sizing:border-box;width:172px;height:50px;border:1px solid black;background:lime'></div><div style='position:absolute;left:6px;top:45px'>A</div><div style='position:absolute;left:66px;top:25px'>B<br>C</div><div style='position:absolute;left:126px;top:25px'>D</div><div style='position:absolute;left:0;top:70px'>E</div>",
    );
}

#[test]
fn a_fixed_height_direct_text_root_preserves_its_columns() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn an_empty_fixed_height_column_container_reserves_its_box() {
    compare(
        "<div class=mc></div><p>E</p>",
        ".mc{height:40px;background:lime}",
        "<div style='position:absolute;left:0;top:0;width:100px;height:40px;background:lime'></div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_single_column_rtl_container_continues_in_an_overflow_column_to_the_left() {
    // The lines past the first column go to a column 120px to the left of the
    // page, so only the first two stay on it.
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px;column-count:1;direction:rtl}.mc p{direction:ltr}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn a_relative_block_in_an_rtl_container_is_painted_shifted_once() {
    // The painter adds the offset of a block that columns fragment, so the
    // column placement leaves it out: the block is painted where it is in
    // ordinary flow.
    compare(
        "<div class=mc><p style='position:relative;left:5px;background:lime'>A<br>B</p></div>",
        ".mc{direction:rtl;height:40px;column-fill:auto}",
        "<div style='position:absolute;left:60px;top:0;width:40px;direction:rtl'><p style='position:relative;left:5px;background:lime'>A<br>B</p></div>",
    );
}

#[test]
fn multiple_paragraphs_balance_through_a_paragraph_break() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>E<br>F</div><div style='position:absolute;left:0;top:60px'>G</div>",
    );
}

#[test]
fn multiple_paragraphs_fill_three_columns_in_one_group() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc{width:160px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:120px;top:0'>E<br>F</div><div style='position:absolute;left:0;top:40px'>G</div>",
    );
}

#[test]
fn multiple_paragraphs_balance_with_two_orphans_and_widows() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D<br>E<br>F</p><p>G<br>H</p></div><p>I</p>",
        ".mc{orphans:2;widows:2}",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C<br>D</div><div style='position:absolute;left:60px;top:0'>E<br>F<br>G<br>H</div><div style='position:absolute;left:0;top:80px'>I</div>",
    );
}

#[test]
fn paragraph_break_minima_override_the_column_container() {
    compare(
        "<div class=mc><p style='orphans:3;widows:3'>A<br>B<br>C<br>D<br>E<br>F</p><p>G<br>H</p></div><p>I</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>E<br>F<br>G<br>H</div><div style='position:absolute;left:0;top:100px'>I</div>",
    );
}

#[test]
fn paragraph_bottom_margins_participate_in_balancing() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D</div><div style='position:absolute;left:60px;top:28px'>E<br>F</div><div style='position:absolute;left:0;top:68px'>G</div>",
    );
}

#[test]
fn adjoining_paragraph_margins_collapse_within_a_column() {
    compare(
        "<div class=mc><p>A<br>B</p><p>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}.mc p+p{margin-top:12px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:52px'>C</div><div style='position:absolute;left:60px;top:0'>D</div><div style='position:absolute;left:60px;top:32px'>E<br>F</div><div style='position:absolute;left:0;top:76px'>G</div>",
    );
}

#[test]
fn source_whitespace_and_parent_insets_preserve_group_continuations() {
    compare(
        "<p>@</p><div class=mc>\n <p>A<br>B<br>C<br>D</p>\n <p>E<br>F</p>\n</div><p>G</p>",
        ".mc{padding:4px 5px;border:1px solid black}",
        "<div style='position:absolute;left:0;top:0'>@</div><div style='position:absolute;left:0;top:20px;box-sizing:border-box;width:112px;height:70px;border:1px solid black'></div><div style='position:absolute;left:6px;top:25px'>A<br>B<br>C</div><div style='position:absolute;left:66px;top:25px'>D<br>E<br>F</div><div style='position:absolute;left:0;top:90px'>G</div>",
    );
}

#[test]
fn short_paragraphs_move_whole_and_truncate_adjoining_break_margins() {
    compare(
        "<div class=mc><p>A<br>B</p><p>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc{orphans:3;widows:3}.mc p{margin-bottom:8px}.mc p+p{margin-top:12px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:52px'>C<br>D</div><div style='position:absolute;left:60px;top:0'>E<br>F</div><div style='position:absolute;left:0;top:92px'>G</div>",
    );
}

#[test]
fn an_empty_paragraph_does_not_disable_group_balancing() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p><p></p><p>E<br>F</p></div><p>G</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>E<br>F</div><div style='position:absolute;left:0;top:60px'>G</div>",
    );
}

#[test]
fn one_nonempty_paragraph_balances_beside_empty_blocks() {
    for body in [
        "<div class=mc><p>A<br>B<br>C<br>D</p><p></p></div><p>E</p>",
        "<div class=mc><p></p><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        "<div class=mc><p>A<br>B<br>C<br>D</p><p> </p></div><p>E</p>",
        "<div class=mc><p> </p><p>A<br>B<br>C<br>D</p></div><p>E</p>",
    ] {
        compare(
            body,
            "",
            "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
        );
    }
}

#[test]
fn empty_paragraphs_preserve_positive_and_negative_collapsed_margins() {
    compare(
        "<div class=mc><p>A<br>B</p><p class=empty></p><p class=later>C<br>D</p><p class=last>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}.mc .empty{margin-top:12px;margin-bottom:-4px}.mc .later{margin-top:20px}.mc .last{margin-top:12px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:56px'>C</div><div style='position:absolute;left:60px;top:0'>D</div><div style='position:absolute;left:60px;top:32px'>E<br>F</div><div style='position:absolute;left:0;top:78px'>G</div>",
    );
}

#[test]
fn a_zero_line_height_paragraph_preserves_ordinary_following_flow() {
    compare(
        "<div class=mc><p class=zero>A</p><p>B</p></div><p>C</p>",
        ".mc .zero{line-height:0}",
        "<div style='position:absolute;left:0;top:0;line-height:0'>A</div><div style='position:absolute;left:0;top:0'>B</div><div style='position:absolute;left:0;top:20px'>C</div>",
    );
}

#[test]
fn a_leading_margin_cannot_skip_an_empty_column() {
    compare(
        "<div class=mc><p style='margin-top:100px'>A</p><p>B</p></div><p>C</p>",
        "",
        "<div style='position:absolute;left:0;top:100px'>A</div><div style='position:absolute;left:60px;top:0'>B</div><div style='position:absolute;left:0;top:120px'>C</div>",
    );
}

#[test]
fn an_inline_background_moves_with_its_continuation_glyphs() {
    compare(
        "<div class=mc><p>A<br>B</p><p><span style='background:lime'>C<br>D</span></p><p>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}.mc p+p{margin-top:12px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:52px'><span style='background:lime'>C</span></div><div style='position:absolute;left:60px;top:0'><span style='background:lime'>D</span></div><div style='position:absolute;left:60px;top:32px'>E<br>F</div><div style='position:absolute;left:0;top:76px'>G</div>",
    );
}

#[test]
fn an_unfinished_paragraph_background_fills_its_column_extent() {
    compare(
        "<div class=mc><p style='background:lime;orphans:3;widows:3'>A<br>B<br>C<br>D<br>E<br>F</p><p>G<br>H</p></div><p>I</p>",
        "body{color:transparent}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:100px;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:60px;background:lime'></div>",
    );
}

#[test]
fn a_relative_paragraph_in_a_column_is_painted_shifted_once() {
    for inset in ["left:5px", "right:5px", "left:10%", "left:5px;top:3px"] {
        let paragraph = format!("<p style='position:relative;{inset};background:lime'>A<br>B</p>");
        compare(
            &format!("<div class=mc>{paragraph}</div>"),
            ".mc{height:40px;column-fill:auto}",
            &format!("<div style='width:40px'>{paragraph}</div>"),
        );
    }
}

#[test]
fn a_relative_paragraph_in_a_later_column_is_painted_shifted_once() {
    for inset in ["left:5px", "right:5px"] {
        compare(
            &format!(
                "<div class=mc><p>A<br>B</p><p style='position:relative;{inset};background:lime'>C<br>D</p></div>"
            ),
            ".mc{height:40px;column-fill:auto}",
            &format!(
                "<div style='position:absolute;left:0;top:0;width:40px'><p>A<br>B</p></div><div style='position:absolute;left:60px;top:0;width:40px'><p style='position:relative;{inset};background:lime'>C<br>D</p></div>"
            ),
        );
    }
}

#[test]
fn relative_paragraphs_of_balanced_columns_are_painted_shifted_once() {
    compare(
        "<div class=mc><p style='position:relative;left:5px;background:lime'>A</p><p>B</p><p style='position:relative;left:3px;background:lime'>C</p><p>D</p></div>",
        "",
        "<div style='position:absolute;left:0;top:0;width:40px'><p style='position:relative;left:5px;background:lime'>A</p><p>B</p></div><div style='position:absolute;left:60px;top:0;width:40px'><p style='position:relative;left:3px;background:lime'>C</p><p>D</p></div>",
    );
}

#[test]
fn a_relative_block_holding_paragraphs_keeps_its_inset_across_columns() {
    let wrapper = |style: &str| {
        format!(
            "<div class=mc><div style='{style};background:lime'><p>A<br>B</p><p>C<br>D</p></div></div>"
        )
    };
    let css = ".mc{height:40px;column-fill:auto}";
    let relative = raster(&wrapper("position:relative;left:5px"), css);
    let margin = raster(&wrapper("margin-left:5px;margin-right:-5px"), css);
    assert_eq!(
        relative
            .chunks_exact(4)
            .zip(margin.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count(),
        0
    );
}

#[test]
fn a_relative_block_holding_a_split_paragraph_is_painted_shifted_once() {
    // Moving the wrapper by its inset moves the whole column container the
    // same way, since the wrapper fills the column.
    for (inset, container_margin) in [
        ("left:5px", "margin-left:5px"),
        ("right:-5px", "margin-left:5px"),
        ("right:5px", "margin-left:-5px"),
        ("left:5px;right:20px", "margin-left:5px"),
        ("left:10%", "margin-left:4px"),
        ("left:5px;top:3px", "margin-left:5px;margin-top:3px"),
    ] {
        let wrapper = |style: &str| {
            format!("<div style='{style}background:lime'><p>A<br>B<br>C<br>D</p></div>")
        };
        compare(
            &format!(
                "<div class=mc>{}</div>",
                wrapper(&format!("position:relative;{inset};"))
            ),
            ".mc{height:40px;column-fill:auto}",
            &format!(
                "<div class=mc style='height:40px;column-fill:auto;{container_margin}'>{}</div>",
                wrapper("")
            ),
        );
    }
}

#[test]
fn text_inside_a_relative_box_below_a_column_child_is_painted_shifted_once() {
    // The inner box sits in a wrapper that fills the column, so offsetting it
    // by its inset moves its text as the whole column container moves.
    for inset in ["left:5px", "right:-5px"] {
        compare(
            &format!(
                "<div class=mc><div><div style='position:relative;{inset}'><p>A<br>B<br>C<br>D</p></div></div></div>"
            ),
            ".mc{height:40px;column-fill:auto}",
            "<div class=mc style='height:40px;column-fill:auto;margin-left:5px'><div><div><p>A<br>B<br>C<br>D</p></div></div></div>",
        );
    }
}

#[test]
fn a_relative_wrapper_with_a_width_splits_its_paragraph_and_is_shifted_once() {
    // The width keeps the wrapper on the constrained chain, which slices the
    // paragraph at the column end like a wrapper without a width.
    for (inset, margin) in [
        ("left:5px", "margin-left:5px"),
        ("right:-5px", "margin-left:5px"),
        ("left:10%", "margin-left:4px"),
    ] {
        for decoration in ["", "background:red;"] {
            compare(
                &format!(
                    "<div class=mc><div style='{decoration}position:relative;{inset};width:40px'><p>A<br>B<br>C<br>D</p></div></div>"
                ),
                ".mc{height:40px;column-fill:auto}",
                &format!(
                    "<div class=mc style='height:40px;column-fill:auto'><div style='{decoration}{margin};width:40px'><p>A<br>B<br>C<br>D</p></div></div>"
                ),
            );
        }
    }
}

#[test]
fn a_relative_paragraph_with_a_width_splits_and_is_shifted_once() {
    compare(
        "<div class=mc><p style='position:relative;left:5px;width:40px'>A<br>B<br>C<br>D</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div class=mc style='height:40px;column-fill:auto'><p style='margin-left:5px;width:40px'>A<br>B<br>C<br>D</p></div>",
    );
}

#[test]
fn a_relative_box_below_a_wrapper_with_a_width_is_shifted_once() {
    compare(
        "<div class=mc><div style='width:40px'><div style='position:relative;left:5px'><p>A<br>B<br>C<br>D</p></div></div></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div class=mc style='height:40px;column-fill:auto'><div style='width:40px'><div style='margin-left:5px'><p>A<br>B<br>C<br>D</p></div></div></div>",
    );
}

#[test]
fn a_wrapper_with_a_width_splits_its_paragraph_in_a_right_to_left_container() {
    // The wrapper keeps the right edge of its column, which is the right
    // edge of the container for the first column and the left of it for the
    // second. `first` and `second` give the control's `left` in each column.
    for (style, width, first, second) in [
        ("width:30px", 30, 70, 10),
        ("width:30px;margin-right:5px", 30, 65, 5),
        ("width:30px;margin-right:10%", 30, 66, 6),
        ("width:30px;margin-left:auto", 30, 70, 10),
        ("width:30px;margin-left:auto;margin-right:auto", 30, 65, 5),
        ("width:30px;margin-right:auto", 30, 60, 0),
        ("width:30px;position:relative;left:5px", 30, 75, 15),
        // A box wider than its column overflows to the left, whichever
        // margin is auto.
        ("width:50px", 50, 50, -10),
        ("width:50px;margin-left:auto", 50, 50, -10),
        ("width:50px;margin-right:auto", 50, 50, -10),
    ] {
        compare(
            &format!("<div class=mc><div style='{style}'><p>A<br>B<br>C<br>D</p></div></div>"),
            ".mc{height:40px;column-fill:auto;direction:rtl}",
            &format!(
                "<div style='position:absolute;direction:rtl;left:{first}px;top:0;width:{width}px'><p>A<br>B</p></div><div style='position:absolute;direction:rtl;left:{second}px;top:0;width:{width}px'><p>C<br>D</p></div>"
            ),
        );
    }
}

#[test]
fn a_wrapper_with_a_width_fills_three_right_to_left_columns_from_the_right() {
    // 160px wide with 20px gaps leaves 40px columns at 120, 60 and 0.
    compare(
        "<div class=mc><div style='width:30px'><p>A<br>B<br>C<br>D<br>E<br>F</p></div></div>",
        ".mc{width:160px;column-count:3;height:40px;column-fill:auto;direction:rtl}",
        "<div style='position:absolute;direction:rtl;left:130px;top:0;width:30px'><p>A<br>B</p></div><div style='position:absolute;direction:rtl;left:70px;top:0;width:30px'><p>C<br>D</p></div><div style='position:absolute;direction:rtl;left:10px;top:0;width:30px'><p>E<br>F</p></div>",
    );
}

#[test]
fn a_narrow_box_below_a_wrapper_with_a_width_keeps_the_right_edge_in_a_right_to_left_container() {
    // The inner box fills the wrapper between its margins, so its text stays
    // at the right edge of the room they leave.
    for (inner, left, width) in [
        ("width:30px", 70, 30),
        ("margin-left:5px", 65, 35),
        ("margin-right:5px", 60, 35),
    ] {
        compare(
            &format!(
                "<div class=mc><div style='width:40px'><div style='{inner}'><p>A<br>B<br>C<br>D</p></div></div></div>"
            ),
            ".mc{height:40px;column-fill:auto;direction:rtl}",
            &format!(
                "<div style='position:absolute;direction:rtl;left:{left}px;top:0;width:{width}px'><p>A<br>B</p></div><div style='position:absolute;direction:rtl;left:{second}px;top:0;width:{width}px'><p>C<br>D</p></div>",
                second = left - 60
            ),
        );
    }
}

#[test]
fn a_relative_column_container_is_painted_shifted_once() {
    compare(
        "<div class=mc style='position:relative;left:5px;top:3px;background:lime'><p>A<br>B<br>C<br>D</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div class=mc style='margin:3px 0 0 5px;height:40px;column-fill:auto;background:lime'><p>A<br>B<br>C<br>D</p></div>",
    );
}

#[test]
fn a_relative_nested_column_container_is_painted_shifted_once() {
    for (inset, top) in [("left:5px", 0), ("right:-5px", 0), ("left:5px;top:3px", 3)] {
        compare(
            &format!(
                "<div class=mc><div class=in style='position:relative;{inset};background:lime'><p>A<br>B<br>C<br>D</p></div></div>"
            ),
            ".mc{width:160px}.in{column-count:2;column-gap:10px}",
            &format!(
                "<div style='position:absolute;left:5px;top:{top}px;width:70px;height:40px;background:lime'></div><div style='position:absolute;left:5px;top:{top}px'>A<br>B</div><div style='position:absolute;left:45px;top:{top}px'>C<br>D</div>"
            ),
        );
    }
}

#[test]
fn a_relative_block_holding_a_split_paragraph_in_an_rtl_container_is_painted_shifted_once() {
    compare(
        "<div class=mc><div style='position:relative;left:5px;background:lime'><p>A<br>B<br>C<br>D</p></div></div>",
        ".mc{height:40px;column-fill:auto;direction:rtl}.mc p{direction:ltr}",
        "<div class=mc style='height:40px;column-fill:auto;direction:rtl;margin-left:5px'><div style='background:lime'><p style='direction:ltr'>A<br>B<br>C<br>D</p></div></div>",
    );
}

#[test]
fn a_sticky_paragraph_in_a_column_keeps_its_layout_offset() {
    let paragraph = "<p style='position:sticky;left:5px;background:lime'>A<br>B</p>";
    compare(
        &format!("<div class=mc>{paragraph}</div>"),
        ".mc{height:40px;column-fill:auto}",
        &format!("<div style='width:40px'>{paragraph}</div>"),
    );
}

#[test]
fn a_relative_paragraph_in_a_padded_container_resolves_its_inset_against_the_column() {
    compare(
        "<div class=mc><p>A<br>B</p><p style='position:relative;left:10%;background:lime'>C<br>D</p></div>",
        ".mc{height:40px;column-fill:auto;padding:0 20px}",
        "<div style='position:absolute;left:20px;top:0;width:40px'><p>A<br>B</p></div><div style='position:absolute;left:80px;top:0;width:40px'><p style='position:relative;left:10%;background:lime'>C<br>D</p></div>",
    );
}

#[test]
fn a_floated_relative_paragraph_in_a_column_is_painted_shifted_once() {
    compare(
        "<div class=mc><p style='float:left;position:relative;left:5px;background:lime'>A<br>B</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<p style='position:absolute;left:5px;top:0;background:lime'>A<br>B</p>",
    );
}

#[test]
fn a_column_child_with_horizontal_margins_fills_the_column_between_them() {
    for margin in [
        "margin-left:7px",
        "margin-right:7px",
        "margin:0 5px 0 3px",
        "margin-left:10%",
    ] {
        let paragraph = format!("<p style='{margin};background:lime'>A<br>B</p>");
        compare(
            &format!("<div class=mc>{paragraph}</div>"),
            ".mc{height:40px;column-fill:auto}",
            &format!("<div style='width:40px'>{paragraph}</div>"),
        );
    }
}

#[test]
fn a_column_child_with_a_margin_in_a_later_column_fills_the_column_between_them() {
    compare(
        "<div class=mc><p>A<br>B</p><p style='margin-left:7px;background:lime'>C<br>D</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px'><p>A<br>B</p></div><div style='position:absolute;left:60px;top:0;width:40px'><p style='margin-left:7px;background:lime'>C<br>D</p></div>",
    );
}

#[test]
fn a_column_child_with_a_margin_in_a_balanced_container_fills_the_column_between_them() {
    compare(
        "<div class=mc><p style='margin-left:7px;background:lime'>A</p><p>B</p><p style='margin-right:7px;background:lime'>C</p><p>D</p></div>",
        "",
        "<div style='position:absolute;left:0;top:0;width:40px'><p style='margin-left:7px;background:lime'>A</p><p>B</p></div><div style='position:absolute;left:60px;top:0;width:40px'><p style='margin-right:7px;background:lime'>C</p><p>D</p></div>",
    );
}

#[test]
fn a_right_to_left_column_child_with_a_margin_fills_the_column_between_them() {
    for margin in [
        "margin-left:7px",
        "margin-right:7px",
        "margin-left:10%",
        "margin-right:10%",
    ] {
        let paragraph = format!("<p style='{margin};background:lime'>A<br>B</p>");
        compare(
            &format!("<div class=mc>{paragraph}</div>"),
            ".mc{height:40px;column-fill:auto;direction:rtl}",
            &format!(
                "<div style='position:absolute;left:60px;top:0;width:40px;direction:rtl'>{paragraph}</div>"
            ),
        );
    }
}

#[test]
fn a_column_child_with_calc_negative_or_oversized_margins_matches_block_flow() {
    for margin in [
        "margin-left:calc(2px + 5px)",
        "margin-left:-7px",
        "margin:0 -4px 0 6px",
        "margin-left:50px",
        "margin-left:30px;margin-right:30px",
    ] {
        let paragraph = format!("<p style='{margin};background:lime'>A<br>B</p>");
        compare(
            &format!("<div class=mc>{paragraph}</div>"),
            ".mc{height:40px;column-fill:auto}",
            &format!("<div style='width:40px'>{paragraph}</div>"),
        );
    }
}

#[test]
fn a_column_child_with_auto_margins_matches_block_flow() {
    for margin in ["margin:0 auto", "margin-left:auto", "margin-right:auto"] {
        let paragraph = format!("<p style='{margin};background:lime'>A<br>B</p>");
        compare(
            &format!("<div class=mc>{paragraph}</div>"),
            ".mc{height:40px;column-fill:auto}",
            &format!("<div style='width:40px'>{paragraph}</div>"),
        );
    }
}

#[test]
fn a_column_child_with_a_width_and_auto_margins_is_placed_within_the_column() {
    for margin in [
        "margin:0 auto",
        "margin-left:auto",
        "margin-right:auto",
        "margin:0 auto 0 4px",
        "margin:0 4px 0 auto",
    ] {
        let paragraph = format!("<p style='width:20px;{margin};background:lime'>A<br>B</p>");
        compare(
            &format!("<div class=mc>{paragraph}</div>"),
            ".mc{height:40px;column-fill:auto}",
            &format!("<div style='width:40px'>{paragraph}</div>"),
        );
    }
}

#[test]
fn a_right_to_left_column_child_with_a_width_and_auto_margins_is_placed_within_the_column() {
    for margin in [
        "margin:0 auto",
        "margin-left:auto",
        "margin-right:auto",
        "margin:0 auto 0 4px",
        "margin:0 4px 0 auto",
    ] {
        let paragraph = format!("<p style='width:20px;{margin};background:lime'>A<br>B</p>");
        compare(
            &format!("<div class=mc>{paragraph}</div>"),
            ".mc{height:40px;column-fill:auto;direction:rtl}",
            &format!(
                "<div style='position:absolute;left:60px;top:0;width:40px;direction:rtl'>{paragraph}</div>"
            ),
        );
    }
}

#[test]
fn a_floated_column_child_with_auto_margins_keeps_zero_margins() {
    compare(
        "<div class=mc><p style='float:left;width:20px;margin:0 auto;background:lime'>A<br>B</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<p style='position:absolute;left:0;top:0;width:20px;background:lime'>A<br>B</p>",
    );
}

#[test]
fn a_floated_column_child_shrinks_to_its_content() {
    compare(
        "<div class=mc><p style='float:left;background:lime'>A<br>B</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<p style='position:absolute;left:0;top:0;background:lime'>A<br>B</p>",
    );
}

#[test]
fn a_floated_column_child_in_a_later_column_shrinks_to_its_content() {
    compare(
        "<div class=mc><p>A<br>B</p><p style='float:left;background:lime'>C<br>D</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px'><p>A<br>B</p></div><p style='position:absolute;left:60px;top:0;background:lime'>C<br>D</p>",
    );
}

#[test]
fn a_floated_column_child_in_a_balanced_container_shrinks_to_its_content() {
    compare(
        "<div class=mc><p>A</p><p style='float:left;background:lime'>B<br>C</p></div>",
        "",
        "<div style='position:absolute;left:0;top:0;width:40px'><p>A</p></div><p style='position:absolute;left:60px;top:0;background:lime'>B<br>C</p>",
    );
}

#[test]
fn a_floated_column_child_with_a_margin_shrinks_to_its_content_inside_it() {
    for margin in ["margin-left:7px", "margin:0 5px 0 3px", "margin-right:7px"] {
        compare(
            &format!(
                "<div class=mc><p style='float:left;{margin};background:lime'>A<br>B</p></div>"
            ),
            ".mc{height:40px;column-fill:auto}",
            &format!(
                "<p style='position:absolute;left:0;top:0;{margin};background:lime'>A<br>B</p>"
            ),
        );
    }
}

#[test]
fn a_floated_column_child_that_would_overflow_the_column_is_limited_to_it() {
    compare(
        "<div class=mc><p style='float:left;background:lime'>A B<br>C</p></div>",
        ".mc{height:80px;column-fill:auto}",
        "<p style='position:absolute;left:0;top:0;width:40px;background:lime'>A B<br>C</p>",
    );
}

#[test]
fn a_floated_block_holding_a_paragraph_shrinks_to_its_content() {
    compare(
        "<div class=mc><div style='float:left;background:lime'><p>A<br>B</p></div></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;background:lime'><p>A<br>B</p></div>",
    );
}

#[test]
fn a_right_to_left_floated_column_child_shrinks_to_its_content() {
    compare(
        "<div class=mc><p style='float:left;background:lime'>A<br>B</p></div>",
        ".mc{height:40px;column-fill:auto;direction:rtl}",
        "<p style='position:absolute;left:60px;top:0;background:lime'>A<br>B</p>",
    );
}

#[test]
fn a_right_floated_column_child_sits_at_the_right_edge_of_its_column() {
    for (css, left) in [
        (".mc{height:40px;column-fill:auto}", 20),
        (".mc{height:40px;column-fill:auto;direction:rtl}", 80),
    ] {
        compare(
            "<div class=mc><p style='float:right;background:lime'>A<br>B</p></div>",
            css,
            &format!("<p style='position:absolute;left:{left}px;top:0;background:lime'>A<br>B</p>"),
        );
    }
}

#[test]
fn a_right_floated_column_child_in_a_later_column_sits_at_the_right_edge_of_it() {
    compare(
        "<div class=mc><p>A<br>B</p><p style='float:right;margin-right:5px;background:lime'>C<br>D</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px'><p>A<br>B</p></div><p style='position:absolute;left:75px;top:0;background:lime'>C<br>D</p>",
    );
}

#[test]
fn a_floated_box_without_lines_shrinks_to_its_content() {
    compare(
        "<div class=mc><div style='float:left;background:lime'><div style='width:20px;height:20px;background:blue'></div></div></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;background:lime'><div style='width:20px;height:20px;background:blue'></div></div>",
    );
}

#[test]
fn a_relative_right_floated_column_child_is_painted_shifted_once() {
    compare(
        "<div class=mc><p style='float:right;position:relative;left:5px;background:lime'>A<br>B</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<p style='position:absolute;left:25px;top:0;background:lime'>A<br>B</p>",
    );
}

#[test]
fn a_relative_floated_column_child_with_a_width_is_painted_shifted_once() {
    compare(
        "<div class=mc><p style='float:left;width:40px;position:relative;left:5px;background:lime'>A<br>B</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<p style='position:absolute;left:5px;top:0;width:40px;background:lime'>A<br>B</p>",
    );
}

#[test]
fn a_floated_column_child_fits_the_room_its_margin_leaves() {
    compare(
        "<div class=mc><p style='float:left;margin-left:10px;background:lime'>A B<br>C</p></div>",
        ".mc{height:80px;column-fill:auto}",
        "<p style='position:absolute;left:0;top:0;margin-left:10px;width:30px;background:lime'>A B<br>C</p>",
    );
}

#[test]
fn a_relative_box_with_an_inset_is_assigned_to_a_column_of_a_definite_height_container() {
    compare(
        "<div class=mc><div style='position:relative;left:5px;height:40px;background:lime'></div></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:5px;top:0;width:40px;height:40px;background:lime'></div>",
    );
}

#[test]
fn a_relative_paragraph_after_a_full_column_moves_to_the_next_column_of_a_definite_height_container()
 {
    compare(
        "<div class=mc><p style='position:relative;left:5px;color:lime'>A</p><p>B</p><p style='position:relative;left:5px;color:blue'>C</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px'><p style='margin-left:5px;margin-right:-5px;color:lime'>A</p><p>B</p></div><div style='position:absolute;left:60px;top:0;width:40px'><p style='margin-left:5px;margin-right:-5px;color:blue'>C</p></div>",
    );
}

#[test]
fn a_relative_box_with_each_inline_inset_is_painted_shifted_once_in_a_definite_height_container() {
    for (inset, shift) in [
        ("left:5px", "left:5px;top:0"),
        ("right:5px", "left:-5px;top:0"),
        ("left:5px;right:20px", "left:5px;top:0"),
    ] {
        compare(
            &format!(
                "<div class=mc><div style='position:relative;{inset};height:20px;background:lime'></div></div>"
            ),
            ".mc{height:40px;column-fill:auto}",
            &format!(
                "<div style='position:absolute;{shift};width:40px;height:20px;background:lime'></div>"
            ),
        );
    }
}

#[test]
fn a_relative_box_in_a_later_column_of_a_definite_height_container_is_painted_shifted_once() {
    compare(
        "<div class=mc><div style='height:40px;background:blue'></div><div style='position:relative;left:5px;height:20px;background:lime'></div></div>",
        ".mc{height:40px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:40px;height:40px;background:blue'></div><div style='position:absolute;left:65px;top:0;width:40px;height:20px;background:lime'></div>",
    );
}

#[test]
fn a_relative_box_in_a_right_to_left_definite_height_container_is_painted_shifted_once() {
    compare(
        "<div class=mc><div style='position:relative;left:5px;height:20px;background:lime'></div></div>",
        ".mc{height:40px;column-fill:auto;direction:rtl}",
        "<div style='position:absolute;left:65px;top:0;width:40px;height:20px;background:lime'></div>",
    );
}

#[test]
fn relative_paragraphs_with_each_inline_inset_are_painted_shifted_once_in_a_definite_height_container()
 {
    for (inset, shift) in [
        ("right:5px", "left:-5px;top:0"),
        ("left:5px", "left:5px;top:0"),
    ] {
        compare(
            &format!(
                "<div class=mc><p style='position:relative;{inset};color:lime'>A</p><p>B</p></div>"
            ),
            ".mc{height:40px;column-fill:auto}",
            &format!(
                "<div style='position:absolute;left:0;top:20px;width:40px'><p>B</p></div><div style='position:absolute;{shift};width:40px'><p style='color:lime'>A</p></div>"
            ),
        );
    }
}

#[test]
fn display_contents_children_of_a_definite_height_container_are_painted() {
    // An element with `display: contents` has no box, so it paints no
    // background, and the blocks inside it are painted where the columns put
    // them: the columns are balanced, so each paragraph gets its own.
    compare(
        "<div class=mc style='height:60px'><div style='display:contents;background:blue'><p>A</p></div><div style='display:contents'><div style='display:contents'><p>B</p></div></div></div>",
        "",
        "<div style='position:absolute;left:0;top:0'>A</div><div style='position:absolute;left:60px;top:0'>B</div>",
    );
}

#[test]
fn display_contents_children_of_a_balanced_container_are_painted() {
    compare(
        "<div class=mc><div style='display:contents'><p>A</p><p>B</p></div><div style='display:contents'><p>C</p><p>D</p></div></div>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

/// The number of pixels at which `body` and `plain` render differently.
fn differing_pixels(body: &str, plain: &str, css: &str) -> usize {
    let actual = raster(body, css);
    let expected = raster(plain, css);
    actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

fn compare_bodies(body: &str, plain: &str, css: &str) {
    assert_eq!(differing_pixels(body, plain, css), 0);
}

#[test]
fn a_display_contents_wrapper_keeps_the_column_clip_of_its_children() {
    // The blocks that columns clip to their height are clipped the same way
    // when an element without a box wraps them.
    let css = ".mc{height:40px;column-fill:auto}";
    for inner in [
        "<div style='display:grid;height:80px;background:blue'></div>",
        "<div style='display:table;background:blue'><div style='display:table-row'><div style='display:table-cell;width:20px;height:80px'></div></div></div>",
        "<div style='height:0'><div style='height:80px;background:blue'></div></div>",
    ] {
        compare_bodies(
            &format!("<div class=mc><div style='display:contents'>{inner}</div></div><p>X</p>"),
            &format!("<div class=mc>{inner}</div><p>X</p>"),
            css,
        );
        compare_bodies(
            &format!(
                "<div class=mc><div style='display:contents'><div style='display:contents'>{inner}</div></div></div><p>X</p>"
            ),
            &format!("<div class=mc>{inner}</div><p>X</p>"),
            css,
        );
    }
}

/// `inner` wrapped in `depth` boxless `display: contents` elements.
fn wrapped(inner: &str, depth: usize) -> String {
    let mut html = inner.to_string();
    for _ in 0..depth {
        html = format!("<div style='display:contents'>{html}</div>");
    }
    html
}

#[test]
fn display_contents_wrappers_leave_the_column_assignment_of_blocks_unchanged() {
    // Each wrapper holds one block: the columns break between the wrappers
    // exactly as they break between the blocks.
    for depth in 1..=2 {
        compare_bodies(
            &format!(
                "<div class=mc>{}{}</div><p>X</p>",
                wrapped("<p>A</p>", depth),
                wrapped("<p>B</p>", depth)
            ),
            "<div class=mc><p>A</p><p>B</p></div><p>X</p>",
            ".mc{height:20px;column-fill:auto}",
        );
    }
}

#[test]
fn display_contents_wrappers_leave_the_line_breaking_of_a_column_flow_unchanged() {
    for depth in 1..=2 {
        compare_bodies(
            &format!(
                "<div class=mc>{}</div><p>X</p>",
                wrapped("<p>A<br>B<br>C<br>D</p>", depth)
            ),
            "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>X</p>",
            ".mc{height:40px;column-fill:auto}",
        );
    }
}

#[test]
fn display_contents_wrappers_leave_the_balancing_of_a_column_flow_unchanged() {
    for depth in 1..=2 {
        compare_bodies(
            &format!(
                "<div class=mc>{}</div><p>X</p>",
                wrapped("<p>A<br>B<br>C<br>D</p>", depth)
            ),
            "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>X</p>",
            "",
        );
    }
}

/// Each case holds the blocks of a column container and the container's CSS.
/// The blocks are placed the same way whether they sit directly in the
/// container, all inside one boxless wrapper, or each in a wrapper of its own.
const WRAPPED_CASES: &[(&[&str], &str)] = &[
    (
        &[
            "<div style='height:60px;background:blue'></div>",
            "<p>X</p>",
        ],
        ".mc{height:40px;column-fill:auto}",
    ),
    (&["<div style='height:60px;background:blue'></div>"], ""),
    (
        &["<p>A</p>", "<p style='break-before:column'>B</p>"],
        ".mc{height:60px}",
    ),
    (&["<p>A</p>", "<p style='break-before:column'>B</p>"], ""),
    (
        &[
            "<p>A</p>",
            "<h2 style='column-span:all;margin:0'>S</h2>",
            "<p>B<br>C</p>",
        ],
        ".mc{width:140px}",
    ),
    (
        &["<p>A<br>B</p>", "<p>C<br>D</p>", "<p>E</p>"],
        ".mc{height:60px;column-fill:auto}",
    ),
    (
        &[
            "<p>A<br>B</p>",
            "<p style='break-inside:avoid'>C<br>D</p>",
            "<p>E</p>",
        ],
        ".mc{height:60px;column-fill:auto}",
    ),
    (
        &["<div class=mc style='width:40px'><p>A<br>B<br>C<br>D</p></div>"],
        "",
    ),
    (
        &["<div style='width:80px'><p>A<br>B<br>C<br>D</p></div>"],
        ".mc{height:40px;column-fill:auto}",
    ),
    (
        &["<p>A</p>", "<p></p>", "<p>B<br>C</p>"],
        ".mc{height:40px;column-fill:auto}",
    ),
    (
        &["<p>A<br>B<br>C<br>D</p>", "<p style='width:40px'>E</p>"],
        ".mc{direction:rtl}",
    ),
    (
        &[
            "<p style='position:relative;left:10px'>A<br>B</p>",
            "<p>C<br>D</p>",
        ],
        ".mc{height:40px;column-fill:auto}",
    ),
    (
        &[
            "<p style='float:left;width:20px'>F</p>",
            "<p>A<br>B<br>C<br>D</p>",
        ],
        "",
    ),
    (
        &[
            "<p>A</p>",
            "<div class=mc style='width:40px'><p>B<br>C<br>D</p></div>",
        ],
        ".mc{height:60px}",
    ),
    (
        &[
            "<p style='margin:10px 0'>A</p>",
            "<p style='margin:10px 0'>B</p>",
        ],
        ".mc{height:50px;column-fill:auto}",
    ),
    // Table rows outside a table stay blocks of the container, in a wrapper or not.
    (
        &[
            "<div style='display:table-row'><div style='display:table-cell'>A</div></div>",
            "<div style='display:table-row'><div style='display:table-cell'>B</div></div>",
        ],
        ".mc{height:40px;column-fill:auto}",
    ),
];

#[test]
fn display_contents_wrappers_leave_every_column_strategy_unchanged() {
    for (index, (blocks, css)) in WRAPPED_CASES.iter().enumerate() {
        let plain = format!("<div class=mc>{}</div><p>Z</p>", blocks.concat());
        let whole = format!(
            "<div class=mc>{}</div><p>Z</p>",
            wrapped(&blocks.concat(), 1)
        );
        let each = format!(
            "<div class=mc>{}</div><p>Z</p>",
            blocks
                .iter()
                .map(|block| wrapped(block, 1))
                .collect::<String>()
        );
        for (name, body) in [("whole", whole), ("each", each)] {
            assert_eq!(
                differing_pixels(&body, &plain, css),
                0,
                "case {index} wrapped as {name} differs from the unwrapped blocks"
            );
        }
    }
}

#[test]
fn a_floated_paragraph_without_a_line_break_wraps_at_the_column_width() {
    compare(
        "<div class=mc><p style='float:left;background:lime'>A B</p></div>",
        ".mc{height:40px;column-fill:auto}",
        "<p style='position:absolute;left:0;top:0;width:40px;background:lime'>A<br>B</p>",
    );
}

#[test]
fn a_right_floated_box_without_a_line_break_sits_at_the_right_edge_of_its_column() {
    for (css, left) in [
        (".mc{height:40px;column-fill:auto}", 20),
        (".mc{height:40px;column-fill:auto;direction:rtl}", 80),
    ] {
        compare(
            "<div class=mc><div style='float:right'><div style='width:20px;height:20px;background:blue'></div></div></div>",
            css,
            &format!(
                "<div style='position:absolute;left:{left}px;top:0;width:20px;height:20px;background:blue'></div>"
            ),
        );
    }
}

#[test]
fn a_floated_box_that_does_not_fit_the_first_column_goes_to_the_next_one() {
    compare(
        "<div class=mc><p>A</p><div style='float:right;margin-right:5px'><div style='width:20px;height:20px;background:blue'></div></div></div>",
        ".mc{height:20px;column-fill:auto}",
        "<p style='position:absolute;left:0;top:0'>A</p><div style='position:absolute;left:75px;top:0;width:20px;height:20px;background:blue'></div>",
    );
}

#[test]
fn adjoining_margins_before_a_trailing_float_still_collapse() {
    compare(
        "<div class=mc><p style='margin-bottom:20px'>A</p><p style='margin-top:20px'>B</p><div style='float:right;width:20px;height:20px'></div></div>",
        ".mc{height:100px;column-fill:auto}",
        "<p style='position:absolute;left:0;top:0'>A</p><p style='position:absolute;left:0;top:40px'>B</p>",
    );
}

/// An inline-block that sits on its line like a glyph: 15px wide, one line
/// tall, with a glyph inside when `glyph` is not empty.
fn atomic(glyph: &str, color: &str) -> String {
    format!(
        "<span style='display:inline-block;vertical-align:top;width:15px;height:20px;background:{color}'>{glyph}</span>"
    )
}

#[test]
fn an_atomic_inline_of_a_child_paragraph_goes_to_the_column_of_its_line() {
    let red = atomic("", "red");
    compare(
        &format!("<div class=mc><p>A<br>B<br>{red}C<br>D</p></div><p>E</p>"),
        "",
        &format!(
            "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>{red}C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>"
        ),
    );
}

#[test]
fn atomic_inlines_with_text_stay_with_their_lines_in_both_columns() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    compare(
        &format!("<div class=mc><p>A{blue}<br>B{red}<br>{blue}C<br>D{red}</p></div><p>F</p>"),
        "",
        &format!(
            "<div style='position:absolute;left:0;top:0'>A{blue}<br>B{red}</div><div style='position:absolute;left:60px;top:0'>{blue}C<br>D{red}</div><div style='position:absolute;left:0;top:40px'>F</div>"
        ),
    );
}

#[test]
fn a_definite_height_child_paragraph_with_atomics_breaks_between_its_lines() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    compare(
        &format!("<div class=mc><p>A{red}<br>B<br>{blue}C<br>D</p></div><p>E</p>"),
        ".mc{height:40px}",
        &format!(
            "<div style='position:absolute;left:0;top:0'>A{red}<br>B</div><div style='position:absolute;left:60px;top:0'>{blue}C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>"
        ),
    );
}

#[test]
fn atomic_inlines_of_a_child_paragraph_split_after_another_one_follow_their_lines() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    for css in ["", ".mc{height:60px}"] {
        compare(
            &format!("<div class=mc><p>A<br>B</p><p>C{red}<br>{blue}D<br>F</p></div><p>E</p>"),
            css,
            &format!(
                "<div style='position:absolute;left:0;top:0'>A<br>B<br>C{red}</div><div style='position:absolute;left:60px;top:0'>{blue}D<br>F</div><div style='position:absolute;left:0;top:60px'>E</div>"
            ),
        );
    }
}

#[test]
fn atomic_inlines_of_a_right_to_left_child_paragraph_follow_their_lines() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    compare(
        &format!("<div class=mc><p>A{red}<br>B<br>{blue}C<br>D</p></div><p>E</p>"),
        ".mc{direction:rtl}",
        &format!(
            "<div dir=rtl style='position:absolute;left:60px;top:0;width:40px'>A{red}<br>B</div><div dir=rtl style='position:absolute;left:0;top:0;width:40px'>{blue}C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>"
        ),
    );
}

#[test]
fn an_atomic_inline_in_a_ruby_annotation_follows_the_line_of_its_ruby() {
    let red = atomic("", "red");
    compare(
        &format!("<div class=mc><p>A<br>B<br><ruby>C<rt>{red}</rt></ruby><br>D</p></div>"),
        "",
        &format!(
            "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'><ruby>C<rt>{red}</rt></ruby><br>D</div>"
        ),
    );
}

#[test]
fn an_atomic_inline_taller_than_its_text_makes_its_line_taller_in_the_column() {
    let tall = "<span style='display:inline-block;vertical-align:top;width:15px;height:60px;background:red'></span>";
    compare(
        &format!("<div class=mc><p>A<br>{tall}B<br>C<br>D</p></div><p>E</p>"),
        "",
        &format!(
            "<div style='position:absolute;left:0;top:0'>A<br>{tall}B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:80px'>E</div>"
        ),
    );
}

#[test]
fn atomic_inlines_of_a_paragraph_in_wrappers_follow_their_lines() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    for (open, close) in [("<div>", "</div>"), ("<div><div>", "</div></div>")] {
        compare(
            &format!("<div class=mc>{open}<p>A{red}<br>B<br>{blue}C<br>D</p>{close}</div><p>E</p>"),
            ".mc{height:40px}",
            &format!(
                "<div style='position:absolute;left:0;top:0'>A{red}<br>B</div><div style='position:absolute;left:60px;top:0'>{blue}C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>"
            ),
        );
    }
}

#[test]
fn atomic_inlines_of_a_paragraph_in_a_wrapper_fill_three_columns() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    compare(
        &format!("<div class=mc><div><p>A{red}<br>B<br>{blue}C<br>D<br>F{red}</p></div></div>"),
        ".mc{width:160px;column-count:3;height:40px}",
        &format!(
            "<div style='position:absolute;left:0;top:0'>A{red}<br>B</div><div style='position:absolute;left:60px;top:0'>{blue}C<br>D</div><div style='position:absolute;left:120px;top:0'>F{red}</div>"
        ),
    );
}

#[test]
fn an_atomic_inline_moved_with_its_line_by_widows_is_not_left_in_the_earlier_column() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    compare(
        &format!("<div class=mc><div><p>A{red}<br>B<br>{blue}C<br>D</p></div></div>"),
        ".mc{height:60px;orphans:2;widows:2}",
        &format!(
            "<div style='position:absolute;left:0;top:0'>A{red}<br>B</div><div style='position:absolute;left:60px;top:0'>{blue}C<br>D</div>"
        ),
    );
}

#[test]
fn atomic_inlines_of_a_paragraph_in_a_right_to_left_wrapper_follow_their_lines() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    compare(
        &format!("<div class=mc><div><p>A{red}<br>B<br>{blue}C<br>D</p></div></div><p>E</p>"),
        ".mc{height:40px;direction:rtl}",
        &format!(
            "<div dir=rtl style='position:absolute;left:60px;top:0;width:40px'>A{red}<br>B</div><div dir=rtl style='position:absolute;left:0;top:0;width:40px'>{blue}C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>"
        ),
    );
}

/// `body` in `depth` plain wrapper blocks inside a column container.
fn in_wrappers(body: &str, depth: usize) -> String {
    let mut html = body.to_string();
    for _ in 0..depth {
        html = format!("<div>{html}</div>");
    }
    html
}

/// The number of pixels `body` leaves inked.
fn ink(body: &str, css: &str) -> usize {
    raster(body, css)
        .chunks_exact(4)
        .filter(|pixel| pixel[..3] != [255, 255, 255])
        .count()
}

#[test]
fn a_paragraph_in_wrappers_of_a_balanced_container_breaks_between_its_lines() {
    for depth in 1..=2 {
        compare(
            &format!(
                "<div class=mc>{}</div><p>E</p>",
                in_wrappers("<p>A<br>B<br>C<br>D</p>", depth)
            ),
            "",
            "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
        );
    }
}

#[test]
fn a_paragraph_in_a_wrapper_of_a_balanced_container_fills_three_columns() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D<br>F<br>G</p></div></div>",
        ".mc{width:160px;column-count:3}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:120px;top:0'>F<br>G</div>",
    );
}

#[test]
fn an_uneven_paragraph_in_a_wrapper_of_a_balanced_container_keeps_the_longer_first_column() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D<br>F</p></div></div><p>E</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>F</div><div style='position:absolute;left:0;top:60px'>E</div>",
    );
}

#[test]
fn widows_of_a_paragraph_in_a_wrapper_of_a_balanced_container_pull_lines_to_the_last_column() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D<br>F</p></div></div><p>E</p>",
        ".mc{widows:3}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D<br>F</div><div style='position:absolute;left:0;top:60px'>E</div>",
    );
}

#[test]
fn a_paragraph_in_a_wrapper_of_a_balanced_right_to_left_container_starts_at_the_right() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{direction:rtl}",
        "<div dir=rtl style='position:absolute;left:60px;top:0;width:40px'>A<br>B</div><div dir=rtl style='position:absolute;left:0;top:0;width:40px'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn wrapped_words_of_a_paragraph_in_a_wrapper_of_a_balanced_container_follow_their_lines() {
    compare(
        "<div class=mc><div><p>AA BB CC DD</p></div></div><p>E</p>",
        "",
        "<div style='position:absolute;left:0;top:0'>AA<br>BB</div><div style='position:absolute;left:60px;top:0'>CC<br>DD</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn atomic_inlines_of_a_paragraph_in_a_wrapper_of_a_balanced_container_follow_their_lines() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    for depth in 1..=2 {
        compare(
            &format!(
                "<div class=mc>{}</div><p>E</p>",
                in_wrappers(&format!("<p>A{red}<br>B<br>{blue}C<br>D</p>"), depth)
            ),
            "",
            &format!(
                "<div style='position:absolute;left:0;top:0'>A{red}<br>B</div><div style='position:absolute;left:60px;top:0'>{blue}C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>"
            ),
        );
    }
}

#[test]
fn a_paragraph_background_in_a_wrapper_of_a_balanced_container_covers_both_column_fragments() {
    let paragraph = "<p style='background:yellow'>A<br>B<br>C<br>D</p>";
    compare_bodies(
        &format!("<div class=mc>{}</div><p>E</p>", in_wrappers(paragraph, 1)),
        &format!("<div class=mc>{paragraph}</div><p>E</p>"),
        "",
    );
}

#[test]
fn column_rules_run_beside_a_paragraph_in_a_wrapper_around_a_spanning_box() {
    // Each side of the spanning box balances its own paragraph; the rule
    // belongs to the columns the lines occupy.
    let spanner = "<h1 style='column-span:all;margin:0;font-size:20px'>S</h1>";
    let (first, second) = ("<p>A<br>B<br>C<br>D</p>", "<p>F<br>G<br>H<br>I</p>");
    compare_bodies(
        &format!(
            "<div class=mc>{}{spanner}{}</div><p>E</p>",
            in_wrappers(first, 1),
            in_wrappers(second, 1)
        ),
        &format!("<div class=mc>{first}{spanner}{second}</div><p>E</p>"),
        ".mc{column-rule:4px solid red}",
    );
}

#[test]
fn a_clipping_wrapper_of_a_balanced_container_keeps_every_line_visible() {
    // The later column of a paragraph lies outside the box of its wrapper, so
    // a wrapper that clips would hide it.
    let plain = ink(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        "",
    );
    for overflow in ["hidden", "clip", "auto"] {
        assert_eq!(
            ink(
                &format!(
                    "<div class=mc><div style='overflow:{overflow}'><p>A<br>B<br>C<br>D</p></div></div><p>E</p>"
                ),
                ""
            ),
            plain,
            "{overflow}"
        );
    }
    // The paragraph itself clips at the column it is drawn in.
    assert_eq!(
        ink(
            "<div class=mc><div><p style='overflow:hidden'>A<br>B<br>C<br>D</p></div></div><p>E</p>",
            ""
        ),
        plain
    );
}

/// Whether every black glyph of `body` has `background` beside it, at its
/// right edge. The glyphs are narrower than the columns.
fn glyphs_have_the_background_beside_them(body: &str, background: [u8; 3]) -> bool {
    let pixels = raster(body, "");
    let at = |x: usize, y: usize| {
        let start = (y * 180 + x) * 4;
        [pixels[start], pixels[start + 1], pixels[start + 2]]
    };
    let glyphs = (0..160)
        .flat_map(|y| (0..180).map(move |x| (x, y)))
        .filter(|&(x, y)| at(x, y) == [0, 0, 0])
        .count();
    glyphs > 0
        && (0..160).all(|y| {
            (0..179).all(|x| {
                at(x, y) != [0, 0, 0] || at(x + 1, y) == [0, 0, 0] || at(x + 1, y) == background
            })
        })
}

#[test]
fn a_first_wrapper_with_a_background_color_has_a_box_in_each_column() {
    // The wrapper that is the container's child has a box of its own in every
    // column its paragraph reaches, so it paints as the paragraph would.
    let lines = "A<br>B<br>C<br>D";
    let six_lines = "A<br>B<br>C<br>D<br>E<br>F";
    let three_columns = ".mc{width:160px;column-count:3}";
    for (inner, css) in [
        (lines, ""),
        (lines, ".mc{height:40px}"),
        (lines, ".mc{direction:rtl}"),
        ("A<br>B<br>C", ""),
        (six_lines, three_columns),
        (six_lines, ".mc{width:160px;column-count:3;height:40px}"),
    ] {
        let painted = format!("<div style='background:yellow'><p>{inner}</p></div>");
        let paragraph = format!("<p style='background:yellow'>{inner}</p>");
        assert!(
            glyphs_have_the_background_beside_them(
                &format!("<div class=mc>{painted}</div>"),
                [255, 255, 0]
            ),
            "{inner} {css}"
        );
        compare_bodies(
            &format!("<div class=mc>{painted}</div><p>E</p>"),
            &format!("<div class=mc>{paragraph}</div><p>E</p>"),
            css,
        );
    }
    // Plain wrappers between the painted one and the paragraph change nothing.
    compare_bodies(
        &format!(
            "<div class=mc><div style='background:yellow'><div><p>{lines}</p></div></div></div><p>E</p>"
        ),
        &format!("<div class=mc><p style='background:yellow'>{lines}</p></div><p>E</p>"),
        "",
    );
}

#[test]
fn a_first_wrapper_that_paints_more_than_a_color_keeps_its_paragraph_whole() {
    // Only a background color can be cut at a column edge. A wrapper with an
    // image, shadow or border radius would paint it again in each column, so
    // its paragraph keeps to the strategy that its one box covers.
    for style in [
        "background-image:linear-gradient(yellow,yellow)",
        "background:yellow;border-radius:4px",
        "background:yellow;box-shadow:0 0 2px black",
    ] {
        let body =
            format!("<div class=mc><div style='{style}'><p>A<br>B<br>C<br>D</p></div></div>");
        assert!(
            glyphs_have_the_background_beside_them(&body, [255, 255, 0]),
            "{style}"
        );
    }
}

#[test]
fn a_first_wrapper_with_generated_content_beside_its_paragraph_keeps_it_whole() {
    // Generated content is no child of the wrapper, so the wrapper looks like
    // it holds the paragraph alone. Its box holds more than the pieces of the
    // paragraph in the columns, so a color is laid out as an image is: the
    // paragraph keeps to the strategy that the one box covers.
    for generated in [
        "::before{content:'X';position:absolute;left:100px;top:0}",
        "::after{content:'X';float:left}",
    ] {
        let wrapper = |style: &str| {
            format!(
                "<style>.w{generated}</style><div class=mc><div class=w style='{style}'><p>A<br>B<br>C<br>D</p></div></div><p>E</p>"
            )
        };
        compare_bodies(
            &wrapper("background:yellow"),
            &wrapper("background-image:linear-gradient(yellow,yellow)"),
            "",
        );
    }
}

#[test]
fn a_background_below_the_first_wrapper_covers_each_column_of_the_paragraph() {
    let (paragraph, painted) = (
        "<p style='background:yellow'>A<br>B<br>C<br>D</p>",
        "<div style='background:yellow'><p>A<br>B<br>C<br>D</p></div>",
    );
    assert!(glyphs_have_the_background_beside_them(
        &format!("<div class=mc>{}</div>", in_wrappers(painted, 1)),
        [255, 255, 0]
    ));
    compare_bodies(
        &format!("<div class=mc>{}</div><p>E</p>", in_wrappers(painted, 1)),
        &format!("<div class=mc>{paragraph}</div><p>E</p>"),
        "",
    );
}

#[test]
fn a_size_contained_wrapper_keeps_the_height_of_an_empty_box() {
    // Its content gives it no height, so the balanced height of the paragraph
    // is not its height either.
    let paragraph = "<p>A<br>B<br>C<br>D</p>";
    compare_bodies(
        &format!("<div class=mc><div style='contain:size'>{paragraph}</div></div><p>E</p>"),
        &format!("<div class=mc><div style='height:0'>{paragraph}</div></div><p>E</p>"),
        "",
    );
    // Containing only the inline size leaves the height to the content.
    compare_bodies(
        &format!("<div class=mc><div style='contain:inline-size'>{paragraph}</div></div><p>E</p>"),
        &format!("<div class=mc><div>{paragraph}</div></div><p>E</p>"),
        "",
    );
}

#[test]
fn a_column_spanning_paragraph_below_a_wrapper_keeps_its_full_width() {
    // A box that spans every column is not split over them.
    compare(
        "<div class=mc><div><p style='column-span:all'>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        "",
        "<div style='position:absolute;left:0;top:0;width:100px'>A<br>B<br>C<br>D</div><div style='position:absolute;left:0;top:80px'>E</div>",
    );
}

#[test]
fn a_relative_wrapper_moves_each_column_of_its_paragraph_once() {
    compare(
        "<div class=mc><div style='position:relative;top:3px'><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        "",
        "<div style='position:absolute;left:0;top:3px'>A<br>B</div><div style='position:absolute;left:60px;top:3px'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn the_shadow_of_a_wrapper_is_not_cut_at_the_column_of_its_paragraph() {
    // A slice of a wrapper cannot draw the shadow of its whole box. The
    // columns are 40 pixels wide and the shadow reaches 44.
    let shadowed = "<div style='box-shadow:4px 4px 0 red'><p>A<br>B<br>C<br>D</p></div>";
    for wrappers in 1..=2 {
        let pixels = raster(
            &format!(
                "<div class=mc>{}</div>",
                in_wrappers(shadowed, wrappers - 1)
            ),
            "",
        );
        let beyond_the_column = (0..160).any(|y| {
            (41..44).any(|x| {
                let start = (y * 180 + x) * 4;
                pixels[start..start + 3] == [255, 0, 0]
            })
        });
        assert!(beyond_the_column, "{wrappers} wrappers");
    }
}

/// Every page of `body` laid out in pages of 180x160 pixels.
fn pages(body: &str) -> Vec<Vec<u8>> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:180px 160px;margin:0}}body{{margin:0;background:white;font:20px/20px Ahem}}p{{margin:0}}.mc{{width:100px;column-count:2;column-gap:20px;orphans:1;widows:1}}</style>{body}"
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
    let count = document.page_count();
    (0..count)
        .map(|index| {
            let (document, cascade, page_box, origin_y) =
                document.page(index).unwrap().paint_inputs();
            let mut scene = Scene::new();
            raikiri_paint::paint_single_page_with_origin_and_page(
                &mut scene, document, cascade, page_box, origin_y, index, count, false,
            )
            .unwrap();
            anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |out| out.append_scene(scene, Affine::IDENTITY),
                180,
                160,
            )
        })
        .collect()
}

#[test]
fn a_paragraph_in_wrappers_of_a_balanced_container_flows_over_pages_whatever_their_depth() {
    // Each line has its own color, so a line lost, repeated or moved to
    // another page shows. A container taller than a page continues on the
    // next one, and no page is left empty.
    let lines: String = (0..30)
        .map(|line| {
            format!(
                "<span style='color:hsl({},100%,50%)'>L</span><br>",
                line * 12
            )
        })
        .collect();
    let direct = pages(&format!("<div class=mc><p>{lines}</p></div><p>X</p>"));
    assert_eq!(direct.len(), 2);
    // A paragraph in the container fills one page's columns before the next
    // page's; the columns of a container that holds wrappers stay whole and
    // the pages cut them, however deep the wrappers are.
    let wrapped: Vec<_> = (1..=2)
        .map(|depth| {
            pages(&format!(
                "<div class=mc>{}</div><p>X</p>",
                in_wrappers(&format!("<p>{lines}</p>"), depth)
            ))
        })
        .collect();
    assert_eq!(wrapped[0].len(), 2);
    assert!(wrapped[0] == wrapped[1]);
}

/// A column container 170px wide with two 80px columns and a fixed height,
/// holding `inner`.
fn in_fixed_outer(height: u32, inner: &str) -> String {
    format!(
        "<div style='width:170px;height:{height}px;column-count:2;column-gap:10px;column-fill:auto'>{inner}</div>"
    )
}

#[test]
fn a_balanced_nested_container_that_fits_an_outer_column_paints_one_box() {
    // Six lines balance into two columns of three, which fit the first
    // column of the outer container exactly. The box belongs to that column
    // alone; the second outer column holds nothing of it.
    compare(
        &in_fixed_outer(
            60,
            "<div style='column-count:2;column-gap:10px;background:green'>A<br>B<br>C<br>D<br>E<br>F</div>",
        ),
        "",
        "<div style='position:absolute;left:0;top:0;width:80px;height:60px;background:green'></div><div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:45px;top:0'>D<br>E<br>F</div>",
    );
}

#[test]
fn a_bordered_nested_container_that_fits_an_outer_column_paints_its_border_once() {
    compare(
        &in_fixed_outer(
            70,
            "<div style='column-count:2;column-gap:10px;border:2px solid red;background:green'>A<br>B<br>C<br>D<br>E<br>F</div>",
        ),
        "",
        "<div style='position:absolute;left:0;top:0;width:76px;height:60px;border:2px solid red;background:green'></div><div style='position:absolute;left:2px;top:2px'>A<br>B<br>C</div><div style='position:absolute;left:45px;top:2px'>D<br>E<br>F</div>",
    );
}

#[test]
fn a_balanced_nested_container_in_a_later_outer_column_paints_one_box() {
    // The container starts the second outer column, so the first one holds
    // nothing of it.
    compare(
        &in_fixed_outer(
            60,
            "<div style='height:60px'></div><div style='column-count:2;column-gap:10px;background:green'>A<br>B<br>C<br>D<br>E<br>F</div>",
        ),
        "",
        "<div style='position:absolute;left:90px;top:0;width:80px;height:60px;background:green'></div><div style='position:absolute;left:90px;top:0'>A<br>B<br>C</div><div style='position:absolute;left:135px;top:0'>D<br>E<br>F</div>",
    );
}

#[test]
fn a_nested_container_in_an_auto_height_container_paints_all_of_its_columns() {
    // Every column of the inner container's own text is drawn, not only the
    // first, whatever the outer container's height.
    for outer in ["", "height:100px;"] {
        compare(
            &format!(
                "<div style='width:170px;{outer}column-count:2;column-gap:10px;column-fill:auto'><div style='column-count:2;column-gap:10px;background:green'>A<br>B<br>C<br>D<br>E<br>F</div></div>"
            ),
            "",
            "<div style='position:absolute;left:0;top:0;width:80px;height:60px;background:green'></div><div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:45px;top:0'>D<br>E<br>F</div>",
        );
    }
}

#[test]
fn atomic_inlines_of_a_nested_container_follow_their_lines() {
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    for outer in ["", "height:100px;", "height:60px;"] {
        compare(
            &format!(
                "<div style='width:170px;{outer}column-count:2;column-gap:10px;column-fill:auto'><div style='column-count:2;column-gap:10px;background:green'>A{red}<br>B<br>{blue}C<br>D<br>E<br>F</div></div>"
            ),
            "",
            &format!(
                "<div style='position:absolute;left:0;top:0;width:80px;height:60px;background:green'></div><div style='position:absolute;left:0;top:0'>A{red}<br>B<br>{blue}C</div><div style='position:absolute;left:45px;top:0'>D<br>E<br>F</div>"
            ),
        );
    }
}

#[test]
fn a_nested_container_paints_like_the_same_container_on_its_own() {
    // Placed in a column of another container, the box and the lines of each
    // of its own columns are where they are without the outer container.
    let (red, blue) = (atomic("", "red"), atomic("X", "blue"));
    for direction in ["", "direction:rtl;"] {
        for text in [
            "A B C D E F G H I J K L".to_string(),
            format!("A{red}<br>B<br>{blue}C<br>D<br>E<br>F"),
        ] {
            let inner = format!(
                "<div style='{direction}border:3px solid red;padding:2px;column-count:2;column-gap:10px;background:green;width:80px;box-sizing:border-box'>{text}</div>"
            );
            for outer in ["", "height:120px;"] {
                compare_bodies(
                    &format!(
                        "<div style='width:170px;{outer}column-count:2;column-gap:10px;column-fill:auto'>{inner}</div>"
                    ),
                    &inner,
                    "",
                );
            }
        }
    }
}

/// The width of the yellow box that `body` paints.
fn yellow_width(body: &str, css: &str) -> usize {
    let columns: Vec<usize> = raster(body, css)
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[..3] == [255, 255, 0])
        .map(|(index, _)| index % 180)
        .collect();
    columns
        .iter()
        .max()
        .zip(columns.iter().min())
        .map_or(0, |(max, min)| max - min + 1)
}

/// A multicol container painted yellow; its text is not painted.
const YELLOW_COLUMNS: &str = ".f{columns:2;column-gap:10px;background:yellow;color:transparent}";

#[test]
fn a_content_keyword_width_makes_a_multicol_container_as_wide_as_its_columns() {
    // Two columns of the 30px block and the 10px gap between them.
    let block = "<div style='width:30px;height:20px'></div>";
    for keyword in ["min-content", "max-content", "fit-content"] {
        let body = format!("<div class=f style='width:{keyword}'>{block}</div>");
        assert_eq!(yellow_width(&body, YELLOW_COLUMNS), 70, "{keyword}");
    }
    // The padding and the border are around the columns.
    let body = format!(
        "<div class=f style='width:max-content;padding:0 5px;border:2px solid yellow'>{block}</div>"
    );
    assert_eq!(yellow_width(&body, YELLOW_COLUMNS), 84);
    // A flex or grid item is sized by the keyword as well.
    for display in ["flex", "grid"] {
        let body = format!(
            "<div style='display:{display}'><div class=f style='width:max-content'>{block}</div></div>"
        );
        assert_eq!(yellow_width(&body, YELLOW_COLUMNS), 70, "{display}");
    }
}

#[test]
fn a_shrink_to_fit_multicol_container_is_as_wide_as_its_columns() {
    let block = "<div style='width:30px;height:20px'></div>";
    for style in ["float:left", "display:inline-block", "position:absolute"] {
        let body = format!("<div class=f style='{style}'>{block}</div>");
        assert_eq!(yellow_width(&body, YELLOW_COLUMNS), 70, "{style}");
    }
    // 2 * 20px of columns and the padding around them.
    let body = "<div class=f style='float:left;column-gap:0;padding:10px'><div style='width:20px;height:20px'></div></div>";
    assert_eq!(yellow_width(body, YELLOW_COLUMNS), 60);
}

#[test]
fn a_fit_content_multicol_container_takes_the_room_between_its_content_widths() {
    // The line "A B" is 60px wide and the word is 20px wide: the container is
    // 50px wide at least and 130px wide at most.
    for (room, expected) in [(40, 50), (100, 100), (180, 130)] {
        let body = format!(
            "<div style='width:{room}px'><div class=f style='width:fit-content'><p>A B</p></div></div>"
        );
        assert_eq!(yellow_width(&body, YELLOW_COLUMNS), expected, "{room}px");
    }
}

#[test]
fn the_lines_of_a_multicol_container_set_its_content_widths() {
    // The container holds the text itself, in no block.
    for (style, expected) in [
        ("width:max-content", 130),
        ("width:min-content", 50),
        ("float:left", 130),
    ] {
        let body = format!("<div class=f style='{style}'>A B</div>");
        assert_eq!(yellow_width(&body, YELLOW_COLUMNS), expected, "{style}");
    }
}

#[test]
fn a_column_width_or_count_sets_the_widths_of_a_container_with_spanners() {
    // Blocks 30px wide in the columns and a spanner of the given style, with
    // a 10px gap and no column count other than the one given.
    let css = ".f{column-gap:10px;background:yellow;color:transparent}";
    for (keyword, columns, spanner, expected) in [
        ("max-content", "column-width:20px", "width:15px", 20),
        ("min-content", "column-width:20px", "width:15px", 20),
        ("max-content", "column-width:20px", "width:50px", 50),
        ("min-content", "column-width:20px", "width:50px", 50),
        ("max-content", "column-width:40px", "width:15px", 40),
        ("min-content", "column-width:40px", "width:15px", 30),
        ("max-content", "column-count:2", "width:15px", 70),
        ("min-content", "column-count:2", "width:15px", 70),
        ("max-content", "column-count:2", "width:100px", 100),
        ("min-content", "column-count:2", "width:100px", 100),
        (
            "max-content",
            "column-count:2;column-width:36px",
            "width:15px",
            82,
        ),
    ] {
        let body = format!(
            "<div class=f style='width:{keyword};{columns}'><div style='width:30px;height:20px'></div><div style='column-span:all;{spanner};height:20px'></div><div style='width:30px;height:20px'></div></div>"
        );
        assert_eq!(
            yellow_width(&body, css),
            expected,
            "{keyword} {columns} {spanner}"
        );
    }
}

#[test]
fn a_spanner_in_plain_blocks_spans_the_columns_of_a_fit_content_container() {
    let spanner = "<div style='column-span:all;width:60px;height:20px'></div>";
    let css = ".f{columns:2;column-gap:0;background:yellow;width:fit-content}";
    // Not repeated in the two columns, and the edges of the blocks around it
    // are around the spanner.
    for (wrapper, expected) in [
        ("<div><div>{}</div></div>", 60),
        ("<div style='padding:0 10px'><div>{}</div></div>", 80),
        ("<div style='margin:0 5px'>{}</div>", 70),
        // A width of the block sizes it as column content only.
        ("<div style='width:20px'>{}</div>", 60),
        ("<div style='width:20px'><div>{}</div></div>", 60),
    ] {
        let body = format!("<div class=f>{}</div>", wrapper.replace("{}", spanner));
        assert_eq!(yellow_width(&body, css), expected, "{wrapper}");
    }
    // A block that roots its own formatting context is column content, and
    // so is the spanner in it: it is repeated in the two columns.
    let body = format!("<div class=f><div style='display:flow-root'>{spanner}</div></div>");
    assert_eq!(yellow_width(&body, css), 120);
    // So does a nested multicol container: the spanner spans its columns
    // only, and the outer container repeats the inner one in its own.
    let body = format!("<div class=f><div style='columns:2;column-gap:0'>{spanner}</div></div>");
    assert_eq!(yellow_width(&body, css), 120);
}

/// Two lime boxes standing for the pieces of a block cut at the end of the
/// first of two columns (40px wide, the second one starting at 60px).
fn lime_pieces(first_height: u32, second_height: u32) -> String {
    format!(
        "<div style='position:absolute;left:0;top:0;width:40px;height:{first_height}px;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:{second_height}px;background:lime'></div>"
    )
}

#[test]
fn a_tall_block_in_the_wrappers_of_a_fixed_height_container_is_split_over_the_columns() {
    for wrapper in [
        "<div>{}</div>",
        "<div style='display:flow-root'>{}</div>",
        "<div><div style='display:flow-root'><div>{}</div></div></div>",
    ] {
        compare(
            &format!(
                "<div class=mc>{}</div>",
                wrapper.replace("{}", "<div style='height:150px;background:lime'></div>")
            ),
            ".mc{height:100px;column-fill:auto}",
            &lime_pieces(100, 50),
        );
    }
}

#[test]
fn the_margin_after_a_block_of_a_flow_root_wrapper_ends_at_the_column() {
    // The 50px margin does not fit in the 30px left of the first column, so
    // the next block starts the second one, and the wrapper's background
    // runs to the end of both columns.
    compare(
        "<div class=mc><div style='display:flow-root;background:lime'><div style='height:70px;margin-bottom:50px'></div><div style='height:100px'></div></div></div>",
        ".mc{height:100px;column-fill:auto}",
        &lime_pieces(100, 100),
    );
}

#[test]
fn a_wrapper_with_a_minimum_width_wider_than_its_column_is_that_wide() {
    compare(
        "<div class=mc><div style='min-width:60px'><div style='height:20px;background:lime'></div></div></div>",
        ".mc{height:100px;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:60px;height:20px;background:lime'></div>",
    );
}

#[test]
fn the_spanners_of_a_wrapper_keep_the_larger_of_their_margins_between_them() {
    // The spanners are not blocks of a column, and the margins between them
    // collapse into the larger one: the container is 10 + 80 + 10px high.
    compare(
        "<div class=mc><div><div style='column-span:all;height:10px;margin-bottom:50px'></div><div style='column-span:all;height:10px;margin-top:80px'></div></div></div>",
        ".mc{background:lime}",
        "<div style='position:absolute;left:0;top:0;width:100px;height:100px;background:lime'></div>",
    );
}

#[test]
fn a_wrapper_is_sliced_like_its_blocks_under_the_sizing_of_the_container() {
    let tall = "<div style='height:150px;margin:10px 0;background:lime'></div>";
    let pair = "<div style='height:70px;margin:10px 0;background:lime'></div><div style='height:70px;margin:10px 0;background:lime'></div>";
    let column = "<div style='display:flex;flex-direction:column;height:120px'>{}</div>";
    for (outer, css, blocks) in [
        ("{}", ".mc{min-height:120px}", tall),
        ("{}", ".mc{max-height:60px}", tall),
        ("{}", ".mc{height:100px}", tall),
        ("{}", ".mc{height:100px;column-fill:auto}", tall),
        (
            "{}",
            ".mc{height:100px;min-height:130px;column-fill:auto}",
            tall,
        ),
        ("{}", ".mc{padding:5px 3px;border:1px solid black}", tall),
        (
            "{}",
            ".mc{padding:5px 3px;border:1px solid black;height:100px;box-sizing:border-box}",
            tall,
        ),
        // A height that the parent gives the container.
        (column, ".mc{flex:0 1 auto}", pair),
        ("<div style='display:flex;height:120px'>{}</div>", "", pair),
        (
            "<div style='display:grid;grid-template-rows:120px'>{}</div>",
            "",
            pair,
        ),
        (
            "<div style='display:grid;grid-template-rows:120px'>{}</div>",
            ".mc{column-fill:auto}",
            pair,
        ),
    ] {
        let page = |content: &str| {
            let container = format!("<div class=mc>{content}</div>");
            raster(&outer.replace("{}", &container), css)
        };
        let direct = page(blocks);
        assert!(
            direct
                .chunks_exact(4)
                .any(|pixel| pixel[0] == 0 && pixel[1] > 200),
            "{outer} {css}"
        );
        for wrapper in ["<div>{}</div>", "<div style='display:flow-root'>{}</div>"] {
            assert!(
                direct == page(&wrapper.replace("{}", blocks)),
                "{outer} {css} {wrapper}"
            );
        }
    }
}

#[test]
fn a_block_after_a_spanner_keeps_its_first_row_of_ink_at_a_fractional_offset() {
    // The block after the spanner starts 0.3px into a device pixel, and the
    // rasterizer puts its glyphs on whole pixels, so its fragment clip must not
    // cut the first row of them.
    compare_at(
        "<div class=mc><p>A</p><div style='column-span:all;height:10.3px'></div><p>B</p></div>",
        "",
        "<div style='width:100px'><p>A</p><div style='height:10.3px'></div><p>B</p></div>",
        Some(1.0),
    );
}

#[test]
fn a_spanner_in_a_plain_wrapper_spans_the_columns_of_the_container() {
    // The wrapper has nothing to draw, so it makes no difference: the lines
    // before the spanner fill the two columns, the spanner crosses both, and
    // the lines after it fill them again.
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p><div style='column-span:all;height:20px;background:lime'></div><p>E<br>F<br>G<br>H</p></div></div>",
        "",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px;width:100px;height:20px;background:lime'></div><div style='position:absolute;left:0;top:60px'>E<br>F</div><div style='position:absolute;left:60px;top:60px'>G<br>H</div>",
    );
}

#[test]
fn plain_wrappers_of_a_spanner_are_not_told_apart_from_no_wrapper() {
    let blocks = "<p>A<br>B<br>C<br>D</p><div class=sp></div><p>E<br>F<br>G<br>H</p>";
    let spanner = "<div class=sp></div>";
    for css in [
        ".sp{column-span:all;height:20px;background:lime}",
        ".sp{column-span:all;height:20px;background:lime}.mc{height:100px;column-fill:auto}",
    ] {
        let plain = format!("<div class=mc>{blocks}</div>");
        assert!(
            raster(&plain, css)
                .chunks_exact(4)
                .any(|pixel| pixel[0] == 0 && pixel[1] > 200),
            "{css}"
        );
        for (wrapper, content) in [
            ("<div>{}</div>", blocks),
            ("<div><section>{}</section></div>", blocks),
            ("<div style='margin:0 auto'>{}</div>", blocks),
            ("<div style='display:contents'><div>{}</div></div>", blocks),
        ] {
            compare_bodies(
                &format!("<div class=mc>{}</div>", wrapper.replace("{}", content)),
                &plain,
                css,
            );
        }
        // Around the spanner only, and around every block in turn.
        compare_bodies(
            &format!(
                "<div class=mc><p>A<br>B<br>C<br>D</p><div>{spanner}</div><p>E<br>F<br>G<br>H</p></div>"
            ),
            &plain,
            css,
        );
        compare_bodies(
            &format!(
                "<div class=mc><div><p>A<br>B<br>C<br>D</p></div><div>{spanner}</div><div><p>E<br>F<br>G<br>H</p></div></div>"
            ),
            &plain,
            css,
        );
    }
}

#[test]
fn the_lines_of_a_plain_wrapper_fill_the_columns_around_its_spanner() {
    let css = ".sp{column-span:all;height:20px;background:lime}";
    compare_bodies(
        "<div class=mc><div>A<br>B<br>C<br>D<div class=sp></div>E<br>F<br>G</div></div>",
        "<div class=mc>A<br>B<br>C<br>D<div class=sp></div>E<br>F<br>G</div>",
        css,
    );
}

#[test]
fn a_spanner_in_a_wrapper_of_a_list_item_paints_in_the_order_of_one_in_the_item() {
    // The marker inside the item and the outline of the spanner overlap by a
    // row of pixels, so the order they are painted in shows.
    let css = "ul{margin:0;padding:0 0 0 20px}li{column-count:1;width:100px;outline:1px solid black;list-style-position:inside}.sp{column-span:all;outline:1px solid blue}";
    compare_bodies(
        "<ul><li><div><div class=sp>X</div></div></li></ul>",
        "<ul><li><div class=sp>X</div></li></ul>",
        css,
    );
}

#[test]
fn a_plain_wrapper_of_a_spanner_keeps_the_column_clip_of_its_children() {
    // The blocks that columns clip to their height are clipped the same way
    // when a wrapper that makes no box holds them.
    let css = ".mc{height:40px;column-fill:auto}.sp{column-span:all;height:0}";
    for inner in [
        "<div style='display:grid;height:80px;background:blue'></div>",
        "<div style='display:table;background:blue'><div style='display:table-row'><div style='display:table-cell;width:20px;height:80px'></div></div></div>",
        "<div style='height:0'><div style='height:80px;background:blue'></div></div>",
    ] {
        compare_bodies(
            &format!("<div class=mc><div>{inner}<div class=sp></div></div></div><p>X</p>"),
            &format!("<div class=mc>{inner}<div class=sp></div></div><p>X</p>"),
            css,
        );
        compare_bodies(
            &format!(
                "<div class=mc><div><div>{inner}</div><div class=sp></div></div></div><p>X</p>"
            ),
            &format!("<div class=mc><div>{inner}</div><div class=sp></div></div><p>X</p>"),
            css,
        );
    }
}

#[test]
fn an_absolute_box_in_a_block_with_no_height_is_not_cut_to_the_block() {
    // The box is placed from the initial containing block, so the empty
    // block it sits in does not clip it.
    compare(
        "<div class=mc><div><div style='position:absolute;width:100px;height:100px;background:red'></div></div></div>",
        ".mc{height:100px;column-gap:0;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:100px;height:100px;background:red'></div>",
    );
}

#[test]
fn an_absolute_box_under_an_opacity_layer_in_a_column_container_blends_with_it() {
    compare(
        "<div class=mc><div style='opacity:.5'><div style='position:absolute;width:100px;height:100px;background:red'></div></div></div>",
        ".mc{height:100px;column-gap:0;column-fill:auto;background:blue}",
        "<div style='width:100px;height:100px;background:blue'><div style='opacity:.5;width:100px;height:100px;background:red'></div></div>",
    );
}

#[test]
fn a_block_with_no_height_still_clips_its_overflowing_flow_to_the_column() {
    // Only boxes out of the block's flow escape its empty fragment area.
    compare(
        "<div class=mc><div><div style='height:200px;background:red'></div></div><div style='height:0'></div></div>",
        ".mc{height:100px;column-gap:0;column-fill:auto}",
        "<div style='position:absolute;left:0;top:0;width:50px;height:100px;background:red'></div><div style='position:absolute;left:50px;top:0;width:50px;height:100px;background:red'></div>",
    );
}

#[test]
fn a_positioned_block_with_no_height_still_cuts_the_boxes_it_holds() {
    // The block is the containing block of the box, so the box is not one
    // of those that escape the block's empty fragment area.
    let css = ".mc{height:100px;column-gap:0;column-fill:auto}";
    let spanner = "<div style='column-span:all;height:20px'></div>";
    for style in ["position:relative", "transform:translate(0)"] {
        compare_bodies(
            &format!(
                "<div class=mc>{spanner}<div style='{style}'><div style='position:absolute;width:100%;height:100px;background:red'></div></div></div>"
            ),
            &format!("<div class=mc>{spanner}<div style='{style}'></div></div>"),
            css,
        );
    }
}

#[test]
fn absolute_boxes_with_a_containing_block_outside_an_empty_block_all_escape_its_cut() {
    let css = ".mc{height:100px;column-gap:0;column-fill:auto}";
    let abs = "position:absolute;width:100px;height:100px;background:red";
    let literal = "<div style='position:absolute;left:0;top:0;width:100px;height:100px;background:red'></div>";
    for body in [
        // Nested empty wrappers.
        format!("<div class=mc><div><div><div style='{abs}'></div></div></div></div>"),
        // A fixed box has the page as its containing block.
        "<div class=mc><div><div style='position:fixed;left:0;top:0;width:100px;height:100px;background:red'></div></div></div>".to_string(),
        // A wrapper without a box is no containing block.
        format!("<div class=mc><div><span style='display:contents;position:relative'><div style='{abs}'></div></span></div></div>"),
        // A comment paints nothing.
        format!("<div class=mc><div><!-- note --><div style='{abs}'></div></div></div>"),
        // Hidden text and collapsible white space paint nothing.
        format!("<div class=mc><div> <span style='display:none'>hidden</span> <div style='{abs}'></div></div></div>"),
    ] {
        compare(&body, css, literal);
    }
}

#[test]
fn an_empty_block_with_painting_content_still_cuts_the_boxes_beside_it() {
    let css = ".mc{height:100px;column-gap:0;column-fill:auto}.g{height:0}.g::before{content:'X'}";
    let abs = "position:absolute;width:100px;height:100px;background:red";
    for inner in [
        "X",
        "&nbsp;",
        "<div class=g></div>",
        "<span style='position:relative'></span>",
        // A positioned wrapper without a box leaves its text in the flow.
        "<span style='display:contents;position:absolute'>X</span>",
    ] {
        compare_bodies(
            &format!(
                "<div class=mc><div style='height:0'>{inner}<div style='{abs}'></div></div></div>"
            ),
            &format!("<div class=mc><div style='height:0'>{inner}</div></div>"),
            css,
        );
    }
}
