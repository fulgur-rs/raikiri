//! A `min-height` or `max-height` of `min-content`, `max-content` or
//! `fit-content` on a block container resolves to the height its content
//! takes before it is fragmented, so a box that a page break cuts grows with
//! the content that the break pushes down, as a box of `auto` height does, and
//! a box in columns takes the height of its lines (CSS Sizing 3, section 5.1;
//! CSS Fragmentation 3, section 4.4). The pixels are the ones Chromium prints
//! for the same documents.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const BLUE: [u8; 3] = [0, 0, 255];
const RED: [u8; 3] = [255, 0, 0];
const GRAY: [u8; 3] = [204, 204, 204];
const LIME: [u8; 3] = [0, 255, 0];
const WHITE: [u8; 3] = [255, 255, 255];

/// The pixels down the column `x` of each page of `body`, laid out in Ahem on
/// 200×200 pages, as runs of `(first row, color)`.
fn runs_per_page(body: &str, x: usize) -> Vec<Vec<(usize, [u8; 3])>> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}html,body{{margin:0}}\
         body{{font:20px/20px Ahem}}p{{margin:0;color:transparent}}\
         .b{{border:6px solid blue;border-bottom-color:red;background:#ccc}}</style>{body}"
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
            raikiri_paint::paint_single_page_with_origin(&mut scene, doc, cascade, page_box, origin)
                .unwrap();
            let pixels = anyrender::render_to_buffer::<
                anyrender_vello_cpu::VelloCpuImageRenderer,
                _,
            >(|out| out.append_scene(scene, Affine::IDENTITY), 200, 200);
            let mut runs: Vec<(usize, [u8; 3])> = Vec::new();
            for y in 0..200 {
                let index = (y * 200 + x) * 4;
                let color = [pixels[index], pixels[index + 1], pixels[index + 2]];
                if runs.last().is_none_or(|run| run.1 != color) {
                    runs.push((y, color));
                }
            }
            runs
        })
        .collect()
}

/// Lines L1 through L`count`.
fn lines(count: u32) -> String {
    let lines: Vec<_> = (1..=count).map(|n| format!("L{n}")).collect();
    format!("<p>{}</p>", lines.join("<br>"))
}

#[test]
fn a_min_content_minimum_grows_a_set_height_box_with_its_lines_on_the_next_page() {
    assert_eq!(
        runs_per_page(
            &format!(
                "<div class=b style='height:50px;min-height:min-content'>{}</div>",
                lines(15)
            ),
            50
        ),
        [
            vec![(0, BLUE), (6, GRAY)],
            vec![(0, GRAY), (120, RED), (126, WHITE)],
        ]
    );
}

#[test]
fn a_fit_content_maximum_shrinks_a_set_height_box_to_its_lines_across_pages() {
    assert_eq!(
        runs_per_page(
            &format!(
                "<div class=b style='height:600px;max-height:fit-content'>{}</div>",
                lines(15)
            ),
            50
        ),
        [
            vec![(0, BLUE), (6, GRAY)],
            vec![(0, GRAY), (120, RED), (126, WHITE)],
        ]
    );
}

#[test]
fn the_flow_after_a_box_grown_by_its_minimum_follows_it_on_the_next_page() {
    assert_eq!(
        runs_per_page(
            &format!(
                "<div class=b style='height:50px;min-height:max-content'>{}</div>\
                 <div style='height:20px;background:lime'></div>",
                lines(12)
            ),
            50
        ),
        [
            vec![(0, BLUE), (6, GRAY)],
            vec![(0, GRAY), (60, RED), (66, LIME), (86, WHITE)],
        ]
    );
}

#[test]
fn a_min_content_minimum_sizes_a_paragraph_in_balanced_columns() {
    let body = "<div style='columns:2;column-gap:0'><p style='height:20px;\
                min-height:min-content;background:#ccc'>L1<br>L2<br>L3</p></div>";
    assert_eq!(runs_per_page(body, 50), [vec![(0, GRAY), (40, WHITE)]]);
    assert_eq!(runs_per_page(body, 150), [vec![(0, GRAY), (20, WHITE)]]);
}

#[test]
fn a_max_content_maximum_above_the_height_keeps_a_set_height_box_whole() {
    assert_eq!(
        runs_per_page(
            &format!(
                "<div class=b style='height:50px;max-height:max-content'>{}</div>\
                 <div style='height:20px;background:lime'></div>",
                lines(15)
            ),
            50
        ),
        [
            vec![(0, BLUE), (6, GRAY), (56, RED), (62, LIME), (82, WHITE)],
            vec![(0, WHITE)],
        ]
    );
}

#[test]
fn a_min_content_minimum_below_the_height_does_not_grow_the_box_with_its_lines() {
    assert_eq!(
        runs_per_page(
            &format!(
                "<div style='height:7px'></div>\
                 <div class=b style='height:400px;min-height:min-content'>{}</div>\
                 <div style='height:20px;background:lime'></div>",
                lines(15)
            ),
            50
        ),
        [
            vec![(0, WHITE), (7, BLUE), (13, GRAY)],
            vec![(0, GRAY)],
            vec![(0, GRAY), (13, RED), (19, LIME), (39, WHITE)],
        ]
    );
}

#[test]
fn a_max_content_maximum_below_the_height_grows_the_box_with_its_lines() {
    assert_eq!(
        runs_per_page(
            &format!(
                "<div style='height:7px'></div>\
                 <div class=b style='height:500px;max-height:max-content'>{}</div>\
                 <div style='height:20px;background:lime'></div>",
                lines(15)
            ),
            50
        ),
        [
            vec![(0, WHITE), (7, BLUE), (13, GRAY)],
            vec![(0, GRAY), (120, RED), (126, LIME), (146, WHITE)],
        ]
    );
}
