//! The lines of text in an absolutely positioned box around the end of a page:
//! a box breaks only between its lines, and a line that does not fit the page
//! it starts on starts the next one. The positions are the ones Chromium
//! prints for the same documents, on pages 100px tall with 30px lines of 20px
//! Ahem text.

use raikiri_html::{
    FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus, Page, RenderError,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, LimitKind, PageDefaults, RenderLimits};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// Lays `body` out on pages 100px tall and hands the result to `read`.
fn with_pages<T>(body: &str, read: impl Fn(u32, &Page<'_>) -> Vec<T>) -> Vec<T> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let mut found = Vec::new();
    for index in 0..8 {
        if let Some(page) = result.page(index) {
            found.extend(read(index, &page));
        }
    }
    found
}

/// The page and top of every line whose first run `wanted` accepts.
fn lines_of(body: &str, wanted: impl Fn(&str) -> bool) -> Vec<(u32, f32)> {
    let mut lines = with_pages(body, |index, page| {
        page.text_runs()
            .into_iter()
            .filter(|run| wanted(run.text.trim_end()))
            .map(|run| (index, run.origin.1 - ASCENT))
            .collect()
    });
    let mut seen = Vec::new();
    lines.retain(|line| {
        let new = !seen.contains(line);
        seen.push(*line);
        new
    });
    lines
}

/// The page, top and height of every border box 20px wide.
fn boxes_20px_wide(body: &str) -> Vec<(u32, f32, f32)> {
    with_pages(body, |index, page| {
        page.fragments()
            .filter(|fragment| fragment.kind() == FragmentKind::Box)
            .map(|fragment| fragment.rect())
            .filter(|rect| (rect.width - 20.0).abs() < 0.01)
            .map(|rect| (index, rect.y, rect.height))
            .collect()
    })
}

/// The tops of the lines of all the text in `body` on each page from the
/// first to the last with a line.
fn tops_per_page(body: &str) -> Vec<Vec<f32>> {
    let found = lines_of(body, |_| true);
    let pages = found.last().map_or(0, |line| line.0 as usize + 1);
    let mut tops = vec![Vec::new(); pages];
    for (page, top) in found {
        tops[page as usize].push(top);
    }
    tops
}

/// `content` in a box placed by `style` inside a relative box 300px tall.
fn positioned(style: &str, content: &str) -> String {
    format!(
        "<div style='position:relative;height:300px'>\
         <div style='position:absolute;left:0;{style}'>{content}</div></div>"
    )
}

#[test]
fn a_line_that_does_not_fit_the_page_moves_its_box_to_the_next_page() {
    // The line is 30px tall, so it fits the first page from 70px down. From
    // 75px it crosses the page end, and the box starts the next page, wherever
    // in the last 25px it was put.
    for (top, expected) in [
        (70, vec![vec![75.0]]),
        (75, vec![vec![], vec![5.0]]),
        (80, vec![vec![], vec![5.0]]),
        (90, vec![vec![], vec![5.0]]),
        (100, vec![vec![], vec![5.0]]),
        (105, vec![vec![], vec![10.0]]),
        (170, vec![vec![], vec![75.0]]),
        (175, vec![vec![], vec![], vec![5.0]]),
        (185, vec![vec![], vec![], vec![5.0]]),
    ] {
        assert_eq!(
            tops_per_page(&positioned(&format!("top:{top}px"), "Y")),
            expected,
            "top:{top}px"
        );
    }
}

#[test]
fn a_box_with_a_set_height_breaks_between_its_lines_not_at_its_height() {
    // The box is 40px tall but its one line is 30px: only the line decides.
    for (top, expected) in [
        (70, vec![vec![75.0]]),
        (75, vec![vec![], vec![5.0]]),
        (95, vec![vec![], vec![5.0]]),
    ] {
        assert_eq!(
            tops_per_page(&positioned(&format!("top:{top}px;height:40px"), "Y")),
            expected,
            "top:{top}px"
        );
    }
}

#[test]
fn the_lines_of_a_box_break_without_regard_to_orphans_and_widows() {
    // Two lines whose second crosses the page end: the second starts the next
    // page alone, although the default orphans and widows are 2. With the
    // first line crossing it, both lines go.
    for (top, expected) in [
        (40, vec![vec![45.0, 75.0]]),
        (45, vec![vec![50.0], vec![5.0]]),
        (70, vec![vec![75.0], vec![5.0]]),
        (75, vec![vec![], vec![5.0, 35.0]]),
        (145, vec![vec![], vec![50.0], vec![5.0]]),
    ] {
        assert_eq!(
            tops_per_page(&positioned(&format!("top:{top}px;width:20px"), "Y Y")),
            expected,
            "top:{top}px"
        );
    }
    // Three lines: the break falls after the first, or after the second.
    for (top, expected) in [
        (10, vec![vec![15.0, 45.0, 75.0]]),
        (15, vec![vec![20.0, 50.0], vec![5.0]]),
        (45, vec![vec![50.0], vec![5.0, 35.0]]),
        (75, vec![vec![], vec![5.0, 35.0, 65.0]]),
        (115, vec![vec![], vec![20.0, 50.0], vec![5.0]]),
    ] {
        assert_eq!(
            tops_per_page(&positioned(&format!("top:{top}px;width:20px"), "Y Y Y")),
            expected,
            "top:{top}px"
        );
    }
}

#[test]
fn a_box_placed_by_its_bottom_breaks_where_it_lands() {
    // The bottom edge of a box 190px above the bottom of the 300px box puts
    // the line at 80px, across the page end.
    assert_eq!(
        tops_per_page(&positioned("bottom:190px", "Y")),
        [vec![], vec![5.0]]
    );
    // 200px puts it at 70px, where it fits.
    assert_eq!(tops_per_page(&positioned("bottom:200px", "Y")), [[75.0]]);
}

#[test]
fn a_box_at_the_static_position_breaks_with_the_other_lines_of_its_paragraph() {
    // The box sits in a paragraph of six lines, three to a page. A top put in
    // the last 25px of a page starts the next page, and so does a top put in
    // the last 25px of the second.
    let child = |top: u32| {
        format!(
            "<div style='position:relative;width:20px'>X X X X X X\
             <span style='position:absolute;left:40px;top:{top}px'>Y</span></div>"
        )
    };
    for (top, expected) in [
        (70, vec![(0, 75.0)]),
        (75, vec![(1, 5.0)]),
        (95, vec![(1, 5.0)]),
        (170, vec![(1, 75.0)]),
        (175, vec![(2, 5.0)]),
        (195, vec![(2, 5.0)]),
    ] {
        assert_eq!(
            lines_of(&child(top), |text| text == "Y"),
            expected,
            "top:{top}px"
        );
    }
}

#[test]
fn a_box_after_a_spacer_breaks_at_the_page_end_not_at_its_container() {
    // The containing block starts 30px down the first page, so a top of 50px
    // puts the line at 80px, across the page end.
    let body = |top: u32| {
        format!(
            "<div style='height:30px'></div><div style='position:relative;height:200px'>\
             <span style='position:absolute;left:0;top:{top}px'>Y</span></div>"
        )
    };
    for (top, expected) in [
        (40, vec![vec![75.0]]),
        (50, vec![vec![], vec![5.0]]),
        (65, vec![vec![], vec![5.0]]),
    ] {
        assert_eq!(tops_per_page(&body(top)), expected, "top:{top}px");
    }
}

#[test]
fn the_padding_above_the_first_line_stays_where_the_box_is() {
    // The padding of 10px above the line leaves the line at 80px from a top
    // of 70px: it crosses the page end, and the next page starts with the
    // line, at the top of the page. A line that fits keeps the padding above
    // it.
    for (top, expected) in [
        (60, vec![vec![75.0]]),
        (65, vec![vec![], vec![5.0]]),
        (80, vec![vec![], vec![5.0]]),
    ] {
        assert_eq!(
            tops_per_page(&positioned(&format!("top:{top}px;padding:10px 0"), "Y")),
            expected,
            "top:{top}px"
        );
    }
    for (top, expected) in [
        (60, vec![vec![75.0], vec![5.0]]),
        (65, vec![vec![], vec![5.0, 35.0]]),
    ] {
        assert_eq!(
            tops_per_page(&positioned(
                &format!("top:{top}px;padding:10px 0;width:20px"),
                "Y Y"
            )),
            expected,
            "top:{top}px with two lines"
        );
    }
}

#[test]
fn a_box_with_nothing_above_its_first_line_moves_whole() {
    // Its background starts the next page with the line, instead of leaving a
    // strip of it on the first page.
    for (top, expected) in [
        (70, vec![(0, 70.0, 30.0)]),
        (75, vec![(1, 0.0, 30.0)]),
        (95, vec![(1, 0.0, 30.0)]),
    ] {
        assert_eq!(
            boxes_20px_wide(&positioned(&format!("top:{top}px;background:#ccc"), "Y")),
            expected,
            "top:{top}px"
        );
    }
}

#[test]
fn a_box_in_a_fixed_box_stays_with_the_page_the_fixed_box_repeats_on() {
    // Chromium prints the text at 80px on every page.
    let found = lines_of(
        "<div style='position:fixed;top:0;left:0;height:300px'>\
         <div style='position:absolute;left:0;top:75px'>Y</div></div>\
         <div style='height:250px'></div>",
        |text| text == "Y",
    );
    assert!(!found.is_empty());
    assert!(found.iter().all(|&(_, top)| top == 80.0), "{found:?}");
}

#[test]
fn a_last_line_of_an_atomic_inline_breaks_like_a_line_of_text() {
    // Three lines of text and, on the fourth, an inline-block that no text
    // owns: it crosses the page end from 90px, so it starts the next page.
    let content =
        "Y<br>Y<br>Y<br><span style='display:inline-block;width:20px;height:20px'></span>";
    // The inline-block is the box 20px tall; those of the box around it are
    // not.
    let inline_blocks: Vec<_> = boxes_20px_wide(&positioned("top:0", content))
        .into_iter()
        .filter(|&(_, _, height)| (height - 20.0).abs() < 0.01)
        .collect();
    assert_eq!(inline_blocks.len(), 1, "{inline_blocks:?}");
    let (page, top, _) = inline_blocks[0];
    assert_eq!(page, 1, "{inline_blocks:?}");
    assert!((0.0..30.0).contains(&top), "{inline_blocks:?}");
}

#[test]
fn a_box_with_blocks_among_its_lines_is_left_to_the_blocks() {
    // The blocks would stay behind when the lines of the box move, so these
    // lines are not moved on their own: all three are printed, and none lands
    // on another.
    for top in [40, 70, 75, 95] {
        let mut tops = tops_per_page(&positioned(&format!("top:{top}px"), "A<div>B</div>C"));
        assert_eq!(tops.iter().map(Vec::len).sum::<usize>(), 3, "top:{top}px");
        for page in &mut tops {
            page.sort_by(f32::total_cmp);
            assert!(
                page.windows(2).all(|pair| pair[1] - pair[0] >= 30.0),
                "top:{top}px {page:?}"
            );
        }
    }
}

#[test]
fn breaks_that_push_a_line_past_the_page_limit_fail_the_layout() {
    // Four lines from 75px end at 195px, inside two pages. The first crosses
    // the first page end and starts the second page, which pushes the last
    // one across the second page end: the box needs three pages.
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = "<!doctype html><style>@page{size:100px 100px;margin:0}\
                body{margin:0;font:20px/30px Ahem}</style><body>\
                <div style='position:relative;height:100px'>\
                <div style='position:absolute;left:0;top:75px;width:20px'>Y Y Y Y</div>\
                </div></body>";
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let layout_with = |pages: u32| {
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::builder()
                .limits(
                    RenderLimits::builder()
                        .max_document_pages(Some(pages))
                        .build(),
                )
                .build(),
            LayoutOptions::new().resources(&resources),
        )
    };
    assert!(matches!(
        layout_with(2),
        Err(RenderError::LimitExceeded {
            kind: LimitKind::Pages,
            limit: 2,
            actual: 3,
        })
    ));
    assert!(matches!(layout_with(3), Ok(LayoutStatus::Completed(_))));
}
