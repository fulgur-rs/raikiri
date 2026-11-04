//! Column clipping keeps the ink overflow of boxes that fit their column.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

fn raster(display: &str, height: u32, outer: &str) -> Vec<u8> {
    raster_with_content(display, height, outer, "")
}

fn raster_with_content(display: &str, height: u32, outer: &str, content: &str) -> Vec<u8> {
    let body = format!(
        "<!doctype html><style>@page{{size:180px 100px;margin:0}}html,body{{margin:0;background:white}}</style>\
         <article style='column-count:2;column-gap:10px;column-fill:auto;width:170px;height:40px;{outer}'>\
         <div style='display:{display};width:80px;height:{height}px;background:red;box-shadow:0 6px 0 0 blue'>{content}</div></article>"
    );
    let resources = RenderResources::new();
    let parsed = parse_html_with_resources(body.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    assert_eq!(result.page_count(), 1);
    let page = result.page(0).unwrap();
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, document, cascade, page_box, origin)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        180,
        100,
    )
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> &[u8] {
    &pixels[(y * 180 + x) * 4..(y * 180 + x) * 4 + 4]
}

#[test]
fn a_grid_that_fits_keeps_its_shadow_below_the_column() {
    let pixels = raster("grid", 40, "");
    assert_eq!(pixel(&pixels, 10, 10), &[255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 10, 42), &[0, 0, 255, 255]);
}

#[test]
fn a_table_that_fits_keeps_its_shadow_below_the_column() {
    let pixels = raster("table", 40, "");
    assert_eq!(pixel(&pixels, 10, 10), &[255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 10, 42), &[0, 0, 255, 255]);
}

#[test]
fn shorter_boxes_keep_the_part_of_the_shadow_past_the_column_bottom() {
    for display in ["grid", "table"] {
        let pixels = raster(display, 35, "");
        assert_eq!(pixel(&pixels, 10, 40), &[0, 0, 255, 255], "{display}");
    }
}

#[test]
fn explicit_overflow_hidden_still_cuts_a_fitting_box_shadow() {
    for display in ["grid", "table"] {
        let pixels = raster(display, 40, "overflow:hidden");
        assert_eq!(pixel(&pixels, 10, 42), &[255, 255, 255, 255], "{display}");
    }
}

#[test]
fn a_box_too_tall_for_the_column_keeps_its_bottom_cut() {
    for display in ["grid", "table"] {
        let pixels = raster(display, 80, "");
        assert_eq!(pixel(&pixels, 10, 10), &[255, 0, 0, 255], "{display}");
        assert_eq!(pixel(&pixels, 10, 42), &[255, 255, 255, 255], "{display}");
        assert_eq!(pixel(&pixels, 10, 86), &[255, 255, 255, 255], "{display}");
    }
}

#[test]
fn a_transformed_column_keeps_the_fitting_box_shadow() {
    for display in ["grid", "table"] {
        let pixels = raster(display, 40, "transform:scale(.5);transform-origin:0 0");
        assert_eq!(pixel(&pixels, 10, 21), &[0, 0, 255, 255], "{display}");
    }
}

#[test]
fn a_fitting_grid_keeps_its_fitting_childs_ink_overflow() {
    let pixels = raster_with_content(
        "grid",
        40,
        "",
        "<div style='height:20px;box-shadow:0 30px lime'></div>",
    );
    assert_eq!(pixel(&pixels, 10, 48), &[0, 255, 0, 255]);
}

#[test]
fn a_taller_flow_child_still_keeps_its_column_bottom_cut() {
    let pixels = raster_with_content(
        "grid",
        40,
        "",
        "<div style='height:50px;background:lime'></div>",
    );
    // A taller flow box can fragment; it is not merely ink overflow.
    assert_eq!(pixel(&pixels, 10, 48), &[255, 255, 255, 255]);
}

#[test]
fn a_zero_height_parallel_flow_keeps_its_existing_column_clip() {
    let pixels = raster_with_content(
        "block",
        0,
        "",
        "<div style='height:80px;background:lime'></div>",
    );
    assert_eq!(pixel(&pixels, 10, 42), &[255, 255, 255, 255]);
}

#[test]
fn a_fitting_box_still_cuts_a_nested_flow_box_that_reaches_past_the_column() {
    // The wrapper keeps the table inside the column, but the box in it does
    // not end there: it is flow that continues in the next column, so the
    // column bottom cuts it, wherever in the column it starts.
    let content = "<div style='height:10px'>\
         <div style='height:20px'></div>\
         <div style='height:60px;background:lime'></div></div>";
    for display in ["grid", "table"] {
        let pixels = raster_with_content(display, 20, "", content);
        assert_eq!(pixel(&pixels, 10, 30), &[0, 255, 0, 255], "{display}");
        assert_eq!(pixel(&pixels, 10, 48), &[255, 255, 255, 255], "{display}");
    }
}

#[test]
fn flow_boxes_inside_a_display_contents_element_count_toward_the_column() {
    let content = "<div style='height:10px'><div style='display:contents'>\
         <div style='height:20px'></div>\
         <div style='height:60px;background:lime'></div></div></div>";
    for display in ["grid", "table"] {
        let pixels = raster_with_content(display, 20, "", content);
        assert_eq!(pixel(&pixels, 10, 48), &[255, 255, 255, 255], "{display}");
    }
}

#[test]
fn a_fitting_box_keeps_the_ink_overflow_of_a_nested_box() {
    let content = "<div style='height:30px'>\
         <div style='height:20px;box-shadow:0 30px lime'></div></div>";
    for display in ["grid", "table"] {
        let pixels = raster_with_content(display, 40, "", content);
        assert_eq!(pixel(&pixels, 10, 48), &[0, 255, 0, 255], "{display}");
    }
}

#[test]
fn a_nested_box_that_clips_its_overflow_ends_the_search_at_its_own_box() {
    let content = "<div style='height:20px;overflow:hidden'><div style='height:100px'></div></div>";
    for display in ["grid", "table"] {
        let pixels = raster_with_content(display, 40, "", content);
        assert_eq!(pixel(&pixels, 10, 42), &[0, 0, 255, 255], "{display}");
    }
}

#[test]
fn an_out_of_flow_box_does_not_count_toward_the_column() {
    let content = "<div style='position:absolute;left:120px;top:0;width:20px;height:100px;background:lime'></div>";
    for display in ["grid", "table"] {
        let pixels = raster_with_content(display, 40, "", content);
        assert_eq!(pixel(&pixels, 10, 42), &[0, 0, 255, 255], "{display}");
    }
}

#[test]
fn a_fitting_grid_that_clips_its_overflow_keeps_its_own_shadow() {
    // Its tall child is cut by the grid's own overflow clip, so the grid ends
    // in the column and its shadow below the column shows. A table grows to
    // hold its content, so only the grid stays small.
    let content = "<div style='height:100px'></div>";
    let pixels = raster_with_content("grid;overflow:hidden", 40, "", content);
    assert_eq!(pixel(&pixels, 10, 42), &[0, 0, 255, 255]);
}

#[test]
fn a_nested_multicol_container_keeps_its_fitting_parents_shadow() {
    let content = "<div style='columns:2;column-gap:0;column-fill:auto;height:20px'>\
         <div style='height:30px'></div><div style='height:30px'></div></div>";
    for display in ["grid", "table"] {
        let pixels = raster_with_content(display, 40, "", content);
        assert_eq!(pixel(&pixels, 10, 42), &[0, 0, 255, 255], "{display}");
    }
}
