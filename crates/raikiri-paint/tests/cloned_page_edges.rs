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

#[test]
fn a_size_contained_cloned_block_paints_as_one_box_cut_at_the_pages() {
    // Size containment makes the box monolithic: it does not repeat its
    // edges at the break, and stays one box cut at the page edge.
    assert_eq!(
        runs_per_page("<div class=b style='contain:size;contain-intrinsic-height:300px'></div>"),
        [
            vec![(0, BLUE), (6, GRAY)],
            vec![(0, GRAY), (106, RED), (112, WHITE)],
        ]
    );
}

#[test]
fn an_absolutely_positioned_cloned_box_on_one_page_paints_as_one_box() {
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b style='position:absolute;top:0;left:0;right:0'>{}</div>",
            lines(3)
        )),
        [vec![(0, BLUE), (6, GRAY), (66, RED), (72, WHITE)]]
    );
}

#[test]
fn a_cloned_block_that_fits_its_page_paints_as_one_box() {
    assert_eq!(
        runs_per_page(&format!("<div class=b>{}</div>", lines(3))),
        [vec![(0, BLUE), (6, GRAY), (66, RED), (72, WHITE)]]
    );
}

#[test]
fn a_cloned_block_under_a_max_height_it_does_not_reach_repeats_its_edges() {
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b style='max-height:500px'>{}</div>",
            lines(15)
        )),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (126, RED), (132, WHITE)],
        ]
    );
}

#[test]
fn a_cloned_paragraph_keeps_room_for_its_own_edges_once() {
    let text: Vec<_> = (1..=15).map(|n| format!("L{n}")).collect();
    assert_eq!(
        runs_per_page(&format!("<p class=b>{}</p>", text.join("<br>"))),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (126, RED), (132, WHITE)],
        ]
    );
}

#[test]
fn a_block_a_forced_break_moves_goes_below_the_repeated_edges() {
    assert_eq!(
        runs_per_page(
            "<div class=b><p>L1<br>L2<br>L3</p>\
             <p style='break-before:page'>M1<br>M2</p></div>"
        ),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (46, RED), (52, WHITE)],
        ]
    );
}

#[test]
fn a_relatively_offset_cloned_block_breaks_where_its_place_in_the_flow_does() {
    let pages = runs_per_page(&format!(
        "<div class=b style='position:relative;top:10px'>{}</div>",
        lines(15)
    ));
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0], [(0, WHITE), (10, BLUE), (16, GRAY)]);
    // The first fragment, offset past its page, runs on into the top of the
    // next page; only the rows below that are compared.
    let second: Vec<_> = pages[1]
        .iter()
        .copied()
        .filter(|&(row, _)| row >= 10)
        .collect();
    assert_eq!(second, [(10, BLUE), (16, GRAY), (136, RED), (142, WHITE)]);
}

#[test]
fn a_cloned_block_inside_an_absolutely_positioned_box_on_one_page_paints_as_one_box() {
    assert_eq!(
        runs_per_page(&format!(
            "<div style='position:absolute;top:0;left:0;right:0'><div class=b>{}</div></div>",
            lines(3)
        )),
        [vec![(0, BLUE), (6, GRAY), (66, RED), (72, WHITE)]]
    );
}

#[test]
fn a_nested_cloned_block_breaks_inside_the_edges_its_ancestor_repeats() {
    const LIME: [u8; 3] = [0, 255, 0];
    const YELLOW: [u8; 3] = [255, 255, 0];
    const BLACK: [u8; 3] = [0, 0, 0];
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b><div style='box-decoration-break:clone;border:4px solid lime;\
             border-bottom-color:black;background:yellow;margin:0 20px'>{}</div></div>",
            lines(15)
        )),
        [
            vec![(0, BLUE), (6, LIME), (10, YELLOW), (190, BLACK), (194, RED)],
            vec![
                (0, BLUE),
                (6, LIME),
                (10, YELLOW),
                (130, BLACK),
                (134, RED),
                (140, WHITE)
            ],
        ]
    );
}

#[test]
fn a_sliced_paragraph_inside_a_cloned_block_keeps_room_for_its_own_closing_edges() {
    const LIME: [u8; 3] = [0, 255, 0];
    let text: Vec<_> = (1..=18).map(|n| format!("L{n}")).collect();
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b><p style='padding-bottom:10px;border-bottom:4px solid lime'>{}</p></div>",
            text.join("<br>")
        )),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (56, LIME), (60, RED), (66, WHITE)],
        ]
    );
}

/// Lines `prefix`1 through `prefix``count` in a paragraph.
fn named_lines(prefix: &str, count: u32) -> String {
    let lines: Vec<_> = (1..=count).map(|n| format!("{prefix}{n}")).collect();
    format!("<p>{}</p>", lines.join("<br>"))
}

#[test]
fn a_paragraph_that_starts_in_the_room_kept_for_the_closing_edges_moves_on() {
    // The second paragraph starts where the repeated padding and border of
    // the box around it begin at the page end.
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b style='padding:4px 0'>{}{}</div>",
            named_lines("L", 9),
            named_lines("M", 3)
        )),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (74, RED), (80, WHITE)],
        ]
    );
}

#[test]
fn a_paragraph_moved_whole_for_its_orphans_goes_below_the_repeated_edges() {
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b style='padding:4px 0'>{}{}</div>",
            named_lines("L", 8),
            named_lines("M", 3)
        )),
        [
            vec![(0, BLUE), (6, GRAY), (194, RED)],
            vec![(0, BLUE), (6, GRAY), (74, RED), (80, WHITE)],
        ]
    );
}

#[test]
fn a_sliced_box_inside_a_cloned_block_stays_inside_the_repeated_edges() {
    const YELLOW: [u8; 3] = [255, 255, 0];
    let text: Vec<_> = (1..=15).map(|n| format!("L{n}")).collect();
    assert_eq!(
        runs_per_page(&format!(
            "<div class=b><p style='background:yellow'>{}</p></div>",
            text.join("<br>")
        )),
        [
            vec![(0, BLUE), (6, YELLOW), (194, RED)],
            vec![(0, BLUE), (6, YELLOW), (126, RED), (132, WHITE)],
        ]
    );
}
