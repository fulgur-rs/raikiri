//! A block that clones its decorations draws its whole border and background
//! around each fragment a page break cuts it into, and the lines inside it
//! leave room for the repeated edges (CSS Fragmentation 3, section 5.4). The
//! pixels are the ones Chromium prints for the same documents.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const BLUE: [u8; 3] = [0, 0, 255];
const RED: [u8; 3] = [255, 0, 0];
const GRAY: [u8; 3] = [204, 204, 204];
const WHITE: [u8; 3] = [255, 255, 255];

/// The pixels down the middle column of each page of `body`, laid out in
/// Ahem on 200×200 pages, as runs of `(first row, color)`.
fn runs_per_page(body: &str) -> Vec<Vec<(usize, [u8; 3])>> {
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
         .b{{box-decoration-break:clone;border:6px solid blue;border-bottom-color:red;\
         background:#ccc}}</style>{body}"
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
                let index = (y * 200 + 100) * 4;
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
fn a_cloned_block_of_lines_closes_and_opens_each_page_with_its_border() {
    assert_eq!(
        runs_per_page(&format!("<div class=b>{}</div>", lines(15))),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (126, RED), (132, WHITE)],
        ]
    );
}

#[test]
fn a_cloned_multicol_container_closes_and_opens_each_page_with_its_border() {
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b style='columns:2;column-gap:0'>{}</div>",
            lines(30)
        )),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (126, RED), (132, WHITE)],
        ]
    );
}
