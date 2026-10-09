//! Native multicolumn placements match independently positioned literal controls.

use anyrender::recording::RenderCommand;
use anyrender::{PaintScene, Scene};
use kurbo::{Affine, Shape};
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

fn lay_out(body: &str, css: &str) -> raikiri_html::DocumentLayout {
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
    document
}

fn raster(body: &str, css: &str) -> Vec<u8> {
    let document = lay_out(body, css);
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
fn relative_atomic_content_does_not_occupy_an_additional_column() {
    let body = "<div class=mc><span style='display:inline-block;width:20px;height:20px;position:relative;left:60px;background:blue'></span></div>";
    assert_eq!(
        raster(body, ".mc{height:40px;column-rule:2px solid red}"),
        raster(body, ".mc{height:40px}"),
    );
}

#[test]
fn wide_rule_is_below_the_owners_standalone_before_content() {
    let actual = raster(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div>",
        ".mc{height:40px;column-rule:160px solid red}.mc::before{content:'XX';position:absolute;color:blue}",
    );
    // The first black glyph covers the first blue X. The second X must
    // remain above the rule, independently of the paragraph's legacy offsets.
    let blue = actual
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| *pixel == [0, 0, 255, 255])
        .map(|(pixel, _)| (pixel % 180, pixel / 180))
        .collect::<Vec<_>>();
    let expected = (0..20)
        .flat_map(|y| (20..40).map(move |x| (x, y)))
        .collect::<Vec<_>>();
    assert_eq!(blue, expected);
}

#[test]
fn wide_rule_is_below_the_owners_outside_text_marker() {
    for overflow in ["", "overflow:hidden"] {
        let actual = raster(
            "<div class=mc><p>A<br>B<br>C<br>D</p></div>",
            &format!(
                ".mc{{display:list-item;list-style-position:outside;margin-left:40px;height:40px;column-rule:200px solid red;{overflow}}}.mc::marker{{content:'X';color:blue}}"
            ),
        );
        let blue = actual
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| *pixel == [0, 0, 255, 255])
            .map(|(pixel, _)| (pixel % 180, pixel / 180))
            .collect::<Vec<_>>();
        let expected = (0..20)
            .flat_map(|y| (16..36).map(move |x| (x, y)))
            .collect::<Vec<_>>();
        assert_eq!(blue, expected);
    }
}

#[test]
fn huge_dotted_rules_bound_scene_commands_without_truncating_the_pattern() {
    let document = lay_out(
        "<div class=mc><div style='height:20px;break-after:column'></div><div style='height:20px'></div></div>",
        "@page{size:180px 40000px}.mc{height:40000px;column-rule:1px dotted red}",
    );
    assert_eq!(document.page_count(), 1);
    let page = document.page(0).unwrap();
    let (source, cascade, page_box, _) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(
        &mut scene,
        source,
        cascade,
        page_box,
        &NoPixels,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .unwrap();
    let fills = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count();
    assert!(fills > 0);
    assert!(fills <= 4100, "unbounded rule primitive count: {fills}");
    let dots = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) if fill.shape.bounding_box().width() == 1.0 => {
                Some(fill.shape.bounding_box())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(dots.first().unwrap().y0, -0.5);
    assert_eq!(dots.last().unwrap().y1, 40000.5);
}

#[test]
fn huge_dashed_rules_bound_backend_segments_and_keep_the_full_span() {
    let document = lay_out(
        "<div class=mc><div style='height:20px;break-after:column'></div><div style='height:20px'></div></div>",
        "@page{size:180px 40000px}.mc{height:40000px;column-rule:1px dashed red}",
    );
    let page = document.page(0).unwrap();
    let (source, cascade, page_box, _) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(
        &mut scene,
        source,
        cascade,
        page_box,
        &NoPixels,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .unwrap();
    let strokes = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Stroke(stroke) => Some(stroke),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].shape.bounding_box().y0, 0.0);
    assert_eq!(strokes[0].shape.bounding_box().y1, 40000.0);
    assert!(strokes[0].style.dash_pattern.iter().sum::<f64>() >= 40000.0 / 4096.0);
}

#[test]
fn rule_widths_snap_to_whole_pixels_before_centering() {
    for (specified, expected) in [(0.3, 1.0), (0.9, 1.0), (1.9, 1.0), (3.9, 3.0)] {
        compare(
            "<div class=mc>A<br>B<br>C<br>D</div>",
            &format!(".mc{{column-rule:{specified}px solid red}}"),
            &format!(
                "<div style='position:absolute;left:{}px;top:0;width:{expected}px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
                50.0 - expected / 2.0
            ),
        );
    }
}

#[test]
fn rule_page_slices_do_not_leak_into_page_margins() {
    let letters = (b'A'..=b'Z')
        .map(|letter| (letter as char).to_string())
        .collect::<Vec<_>>()
        .join("<br>");
    let document = lay_out(
        &format!("<div class=mc>{letters}</div>"),
        "@page{margin:20px}.mc{height:260px;column-rule:2px solid red}",
    );
    assert_eq!(document.page_count(), 3);
    for (index, height) in [(0, 120), (1, 120), (2, 20)] {
        let page = document.page(index).unwrap();
        let (source, cascade, page_box, origin) = page.paint_inputs();
        let mut scene = Scene::new();
        raikiri_paint::paint_single_page_with_origin_and_page_context(
            &mut scene,
            source,
            cascade,
            page_box,
            origin,
            index,
            3,
            false,
            None,
            &mut raikiri_dom::CounterSnapshotBudget::default(),
        )
        .unwrap();
        let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            180,
            160,
        );
        let red = pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| *pixel == [255, 0, 0, 255])
            .map(|(pixel, _)| (pixel % 180, pixel / 180))
            .collect::<Vec<_>>();
        let expected = (20..20 + height)
            .flat_map(|y| [(69, y), (70, y)])
            .collect::<Vec<_>>();
        assert_eq!(red, expected);
    }
}

#[test]
fn two_column_rules_center_in_the_gap_without_moving_text() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div><p>E</p>",
        ".mc{column-rule:2px solid red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:0;top:40px'>E</div>",
    );
}

#[test]
fn three_columns_have_two_independently_positioned_rules() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D<br>E<br>F</div>",
        ".mc{width:160px;column-count:3;column-rule:2px solid blue}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:40px;background:blue'></div><div style='position:absolute;left:109px;top:0;width:2px;height:40px;background:blue'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div><div style='position:absolute;left:120px;top:0'>E<br>F</div>",
    );
}

#[test]
fn padded_column_rules_use_the_content_origin() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div><p>E</p>",
        ".mc{padding:4px 5px;border:1px solid black;background:lime;column-rule:2px solid red}",
        "<div style='position:absolute;left:0;top:0;box-sizing:border-box;width:112px;height:50px;border:1px solid black;background:lime'></div><div style='position:absolute;left:55px;top:5px;width:2px;height:40px;background:red'></div><div style='position:absolute;left:6px;top:5px'>A<br>B</div><div style='position:absolute;left:66px;top:5px'>C<br>D</div><div style='position:absolute;left:0;top:50px'>E</div>",
    );
}

#[test]
fn no_rule_is_painted_beside_an_empty_column() {
    compare(
        "<div class=mc>A</div>",
        ".mc{height:60px;column-rule:2px solid red}",
        "<div style='position:absolute;left:0;top:0'>A</div>",
    );
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{height:40px;width:160px;column-count:3;column-rule:2px solid red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

#[test]
fn rule_longhands_resolve_currentcolor() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        "body{color:blue}.mc{column-rule-width:2px;column-rule-style:solid;column-rule-color:currentcolor}",
        "<style>body{color:blue}</style><div style='position:absolute;left:49px;top:0;width:2px;height:40px;background:blue'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

#[test]
fn rules_cover_the_full_column_height_even_when_the_last_column_is_short() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{height:60px;column-rule:2px solid red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:60px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D</div>",
    );
}

#[test]
fn rules_wider_than_the_gap_paint_below_column_text() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{column-rule:40px solid red}",
        "<div style='position:absolute;left:30px;top:0;width:40px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

#[test]
fn column_rules_share_the_container_opacity_group() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{background:lime;opacity:.5;column-rule:2px solid red}",
        "<div style='position:absolute;left:0;top:0;width:100px;height:40px;background:lime;opacity:.5'><div style='position:absolute;left:49px;top:0;width:2px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div></div>",
    );
}

#[test]
fn wide_rules_obey_the_container_overflow_clip() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{height:40px;overflow:hidden;column-rule:160px solid red}",
        "<div style='position:absolute;left:0;top:0;width:100px;height:40px;overflow:hidden'><div style='position:absolute;left:-30px;top:0;width:160px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div></div>",
    );
}

#[test]
fn dashed_rule_uses_separate_dashes_along_the_column_height() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{height:42px;column-rule:2px dashed red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:6px;background:red'></div><div style='position:absolute;left:49px;top:12px;width:2px;height:6px;background:red'></div><div style='position:absolute;left:49px;top:24px;width:2px;height:6px;background:red'></div><div style='position:absolute;left:49px;top:36px;width:2px;height:6px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

#[test]
fn double_rule_has_a_transparent_middle_third() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{column-rule:6px double red}",
        "<div style='position:absolute;left:47px;top:0;width:2px;height:40px;background:red'></div><div style='position:absolute;left:51px;top:0;width:2px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

#[test]
fn ridge_groove_inset_outset_use_collapsed_border_shading() {
    for (style, left, right) in [
        ("ridge", "#b1c5d9", "#32465a"),
        ("inset", "#b1c5d9", "#32465a"),
        ("groove", "#32465a", "#b1c5d9"),
        ("outset", "#32465a", "#b1c5d9"),
    ] {
        let css = format!(".mc{{column-rule:4px {style} rgb(100, 140, 180)}}");
        let reference = format!(
            "<div style='position:absolute;left:48px;top:0;width:2px;height:40px;background:{left}'></div><div style='position:absolute;left:50px;top:0;width:2px;height:40px;background:{right}'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>"
        );
        compare("<div class=mc>A<br>B<br>C<br>D</div>", &css, &reference);
    }
}

#[test]
fn dotted_rule_has_round_dots_clipped_to_the_column_height() {
    let dots = (0..=10).map(|index| format!("<div style='position:absolute;left:49px;top:{}px;width:2px;height:2px;border-radius:1px;background:red'></div>", index * 4 - 1)).collect::<String>();
    let reference = format!(
        "<div style='position:absolute;width:100px;height:40px;overflow:hidden'>{dots}<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div></div>"
    );
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{column-rule:2px dotted red}",
        &reference,
    );
}

#[test]
fn rules_follow_a_group_of_independent_paragraphs() {
    compare(
        "<div class=mc><p>A<br>B<br>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc{column-rule:2px solid red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:60px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B<br>C</div><div style='position:absolute;left:60px;top:0'>D<br>E<br>F</div><div style='position:absolute;left:0;top:60px'>G</div>",
    );
}

#[test]
fn rules_follow_a_fixed_height_fragmented_wrapper() {
    compare(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div>",
        ".mc{height:40px;column-rule:2px solid red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:40px;background:red'></div><div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
}

#[test]
fn rule_occupancy_includes_non_text_block_boxes() {
    compare(
        "<div class=mc><div style='height:20px;background:lime;break-after:column'></div><div style='height:20px;background:blue'></div></div>",
        ".mc{height:20px;column-rule:2px solid red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:20px;background:red'></div><div style='position:absolute;left:0;top:0;width:40px;height:20px;background:lime'></div><div style='position:absolute;left:60px;top:0;width:40px;height:20px;background:blue'></div>",
    );
}

#[test]
fn transparent_rules_and_short_dashes_preserve_literal_native_pixels() {
    compare(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{column-rule:2px solid transparent}",
        "<div style='position:absolute;left:0;top:0'>A<br>B</div><div style='position:absolute;left:60px;top:0'>C<br>D</div>",
    );
    compare(
        "<div class=mc><div style='height:2px;break-after:column'></div><div style='height:2px'></div></div>",
        ".mc{height:2px;column-rule:2px dashed red}",
        "<div style='position:absolute;left:49px;top:0;width:2px;height:2px;background:red'></div>",
    );
    assert_eq!(
        raster(
            "<div class=mc><div style='height:.4px;break-after:column'></div><div style='height:.4px'></div></div>",
            ".mc{height:.4px;column-rule:2px solid red}",
        ),
        raster("", ""),
    );
}
