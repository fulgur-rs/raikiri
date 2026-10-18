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

#[test]
fn a_fixed_height_column_container_splits_a_nested_plain_block() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
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
        "<div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D</div><div style='position:absolute;left:60px;top:28px'>E<br>F</div><div style='position:absolute;left:0;top:76px'>G</div>",
    );
}

#[test]
fn adjoining_paragraph_margins_collapse_within_a_column() {
    compare(
        "<div class=mc><p>A<br>B</p><p>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}.mc p+p{margin-top:12px}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:52px'>C</div><div style='position:absolute;left:60px;top:0'>D</div><div style='position:absolute;left:60px;top:32px'>E<br>F</div><div style='position:absolute;left:0;top:80px'>G</div>",
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
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:56px'>C</div><div style='position:absolute;left:60px;top:0'>D</div><div style='position:absolute;left:60px;top:32px'>E<br>F</div><div style='position:absolute;left:0;top:80px'>G</div>",
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
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:0;top:52px'><span style='background:lime'>C</span></div><div style='position:absolute;left:60px;top:0'><span style='background:lime'>D</span></div><div style='position:absolute;left:60px;top:32px'>E<br>F</div><div style='position:absolute;left:0;top:80px'>G</div>",
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
