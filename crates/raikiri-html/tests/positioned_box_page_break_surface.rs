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
fn static_absolute_children_follow_moved_lines_inside_positioned_blocks() {
    for content in [
        "<p style='position:relative;margin:0'>A<span style='position:absolute;left:40px'>T</span><br>B</p><p style='margin:0'>C</p>",
        "<div style='position:relative'>A<span style='position:absolute;left:40px'>T</span><div>B</div>C</div><p style='margin:0'>D</p>",
    ] {
        assert_eq!(
            lines_of(&positioned("top:75px", content), |text| text == "T"),
            [(1, 5.0)],
            "{content}"
        );
    }
}

#[test]
fn a_fixed_clip_limits_the_growth_passed_to_the_following_block() {
    let body = positioned(
        "top:75px",
        "<div style='height:30px;overflow:hidden'><p style='margin:0'>A</p><p style='margin:0'>B</p></div><p style='margin:0'>C</p>",
    );
    assert_eq!(lines_of(&body, |text| text == "C"), [(1, 10.0)]);
}

#[test]
fn hidden_and_contents_wrappers_keep_the_positioned_block_sequence() {
    for content in [
        "<p style='margin:0'>A</p><div style='display:none'>X</div><p style='margin:0'>B</p>",
        "<div style='display:contents'><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
    ] {
        assert_eq!(
            tops_per_page(&positioned("top:75px", content)),
            [vec![], vec![5.0, 35.0]]
        );
    }
}

#[test]
fn a_block_inside_an_inline_wrapper_grows_its_positioned_flow() {
    let body = "<div style='position:relative;height:300px'><div id=o style='position:absolute;top:75px;width:80px;background:blue'><span><div>A<br>B</div></span>C</div></div>";
    for (text, top) in [("A", 5.0), ("B", 35.0), ("C", 65.0)] {
        assert_eq!(lines_of(body, |found| found == text), [(1, top)]);
    }
    let boxes = with_pages(body, |page_index, page| {
        page.fragments()
            .filter(|fragment| page.dom().attr(fragment.node(), "id") == Some("o"))
            .map(|fragment| {
                let rect = fragment.rect();
                (page_index, rect.y, rect.height)
            })
            .collect()
    });
    assert_eq!(boxes, [(0, 75.0, 25.0), (1, 0.0, 90.0)]);
}

#[test]
fn inline_wrapper_offsets_do_not_grow_the_positioned_content_twice() {
    for (style, offset) in [("", 0.0), ("position:relative;top:10px", 10.0)] {
        let body = format!(
            "<div style='position:relative;height:300px'><div id=o style='position:absolute;top:75px;width:80px;background:blue'>P<br><span style='{style}'><div>A<br>B</div></span>C</div></div>"
        );
        for (text, page, top) in [
            ("P", 1, 5.0),
            ("A", 1, 35.0 + offset),
            ("B", 1, 65.0 + offset),
            ("C", 2, 5.0),
        ] {
            assert_eq!(
                lines_of(&body, |found| found == text),
                [(page, top)],
                "{style} {text}"
            );
        }
        let boxes = with_pages(&body, |page_index, page| {
            page.fragments()
                .filter(|fragment| page.dom().attr(fragment.node(), "id") == Some("o"))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (page_index, rect.y, rect.height)
                })
                .collect()
        });
        assert_eq!(
            boxes,
            [(0, 75.0, 25.0), (1, 0.0, 100.0), (2, 0.0, 30.0)],
            "{style}"
        );
    }
}

#[test]
fn relative_block_offsets_are_applied_after_the_internal_page_breaks() {
    for content in [
        "<div style='position:relative;top:35px'>A</div><div>B</div>",
        "<div style='position:relative;top:20px'><div style='position:relative;top:15px'>A</div></div><div>B</div>",
    ] {
        let body = positioned("top:40px", content);
        assert_eq!(lines_of(&body, |text| text == "A"), [(0, 80.0)]);
        assert_eq!(lines_of(&body, |text| text == "B"), [(0, 75.0)]);
    }
}

#[test]
fn relative_offsets_preserve_the_painted_clip_and_whole_box_positions() {
    for content in [
        "<div style='height:60px;overflow:hidden'><div style='position:relative;top:35px'>A</div></div><div>B</div>",
        "<div style='position:relative;top:35px;contain:size;height:60px'>A</div><div>B</div>",
    ] {
        let body = positioned("top:40px", content);
        assert_eq!(lines_of(&body, |text| text == "A"), [(0, 80.0)]);
        assert_eq!(lines_of(&body, |text| text == "B"), [(1, 5.0)]);
    }
}

#[test]
fn block_paragraphs_inside_an_absolute_box_move_with_the_following_blocks() {
    for content in [
        "<p style='margin:0'>A</p><p style='margin:0'>B</p>",
        "<div><p style='margin:0'>A</p></div><p style='margin:0'>B</p>",
    ] {
        assert_eq!(
            tops_per_page(&positioned("top:75px", content)),
            [vec![], vec![5.0, 35.0]],
            "{content}"
        );
    }
}

#[test]
fn a_multiline_block_inside_an_absolute_box_keeps_its_widows_and_following_block() {
    assert_eq!(
        tops_per_page(&positioned(
            "top:75px",
            "<p style='margin:0'>A<br>B<br>C<br>D</p><p style='margin:0'>E</p>"
        )),
        [vec![], vec![5.0, 35.0], vec![5.0, 35.0, 65.0]]
    );
}

#[test]
fn a_fixed_height_block_inside_an_absolute_box_keeps_its_following_block_at_its_end() {
    assert_eq!(
        tops_per_page(&positioned(
            "top:75px",
            "<p style='margin:0;height:60px'>A<br>B</p><p style='margin:0'>C</p>"
        )),
        [vec![], vec![5.0, 35.0, 40.0]]
    );
}

#[test]
fn mixed_text_and_blocks_inside_an_absolute_box_keep_their_flow_order() {
    let body = positioned("top:75px", "A<div>B</div>C");
    for (text, top) in [("A", 5.0), ("B", 35.0), ("C", 65.0)] {
        assert_eq!(lines_of(&body, |found| found == text), [(1, top)], "{text}");
    }
}

#[test]
fn an_atomic_inline_inside_an_absolute_box_moves_once_with_its_line() {
    let body = positioned(
        "top:75px",
        "<div style='display:inline-block'>A<br>B</div><p style='margin:0'>C</p>",
    );
    for (text, top) in [("A", 5.0), ("B", 35.0), ("C", 65.0)] {
        assert_eq!(lines_of(&body, |found| found == text), [(1, top)], "{text}");
    }
}

#[test]
fn nested_absolute_block_flow_waits_for_its_containing_lines_to_settle() {
    let body = positioned(
        "top:75px",
        "O<span style='position:absolute;left:40px;top:0'><p style='margin:0'>A</p><p style='margin:0'>B</p></span>",
    );
    for (text, top) in [("O", 5.0), ("A", 5.0), ("B", 35.0)] {
        assert_eq!(lines_of(&body, |found| found == text), [(1, top)], "{text}");
    }
}

#[test]
fn a_fixed_height_ancestor_inside_an_absolute_box_stops_growth_at_its_end() {
    let body = positioned(
        "top:75px",
        "<div style='height:60px'><p style='margin:0'>A</p><p style='margin:0'>B</p></div><p style='margin:0'>C</p>",
    );
    for (text, top) in [("A", 5.0), ("B", 35.0), ("C", 40.0)] {
        assert_eq!(lines_of(&body, |found| found == text), [(1, top)], "{text}");
    }
}

#[test]
fn a_clip_before_the_page_end_keeps_a_partially_visible_line_in_place() {
    let body = positioned(
        "top:75px",
        "<div style='height:15px;overflow:hidden'><p style='margin:0'>A</p></div><p style='margin:0'>B</p>",
    );
    assert_eq!(lines_of(&body, |text| text == "A"), [(0, 80.0)]);
    assert_eq!(lines_of(&body, |text| text == "B"), [(1, 5.0)]);
}

#[test]
fn the_flow_outside_an_absolute_box_does_not_follow_its_moved_blocks() {
    let body = format!(
        "{}<p style='margin:0'>Z</p>",
        positioned(
            "top:75px",
            "<p style='margin:0'>A</p><p style='margin:0'>B</p>"
        )
    );
    assert_eq!(lines_of(&body, |text| text == "A"), [(1, 5.0)]);
    assert_eq!(lines_of(&body, |text| text == "B"), [(1, 35.0)]);
    assert_eq!(lines_of(&body, |text| text == "Z"), [(3, 5.0)]);
}

#[test]
fn the_block_backgrounds_keep_their_start_when_the_positioned_lines_move() {
    let body = positioned(
        "top:75px;width:50px;background:blue",
        "<p id=a style='margin:0;width:30px;background:red'>A</p><p id=b style='margin:0;width:30px;background:lime'>B</p>",
    );
    let boxes = |id: &str| {
        with_pages(&body, |page_index, page| {
            page.fragments()
                .filter(|fragment| page.dom().attr(fragment.node(), "id") == Some(id))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (page_index, rect.y, rect.height)
                })
                .collect()
        })
    };
    assert_eq!(boxes("a"), [(0, 75.0, 25.0), (1, 0.0, 30.0)]);
    assert_eq!(boxes("b"), [(1, 30.0, 30.0)]);
}

#[test]
fn positioned_children_of_a_growing_mixed_box_follow_their_insets() {
    for (inset, top) in [("top:90px", 70.0), ("bottom:0", 35.0), ("top:40%", 14.0)] {
        let body = positioned(
            "top:75px",
            &format!(
                "<p style='margin:0'>A</p>X<span style='position:absolute;left:40px;{inset}'>T</span>"
            ),
        );
        assert_eq!(lines_of(&body, |text| text == "T"), [(1, top)], "{inset}");
    }
}

#[test]
fn a_bottom_child_follows_the_growth_of_a_block_only_absolute_box() {
    let body = positioned(
        "top:75px",
        "<p style='margin:0'>A</p><p style='margin:0'>B</p><span style='position:absolute;left:40px;bottom:0'>T</span>",
    );
    assert_eq!(lines_of(&body, |text| text == "T"), [(1, 35.0)]);
}

#[test]
fn bounds_and_held_floats_absorb_the_growth_inside_an_absolute_box() {
    for (style, extra, page, top) in [
        ("min-height:120px", "", 2, 5.0),
        ("max-height:60px", "", 1, 40.0),
        (
            "display:flow-root",
            "<div style='float:left;width:10px;height:150px'></div>",
            2,
            30.0,
        ),
    ] {
        let margin = if extra.is_empty() {
            "margin:0"
        } else {
            "margin:0;margin-left:10px"
        };
        let body = positioned(
            "top:75px",
            &format!(
                "<div style='{style}'>{extra}<p style='{margin}'>A</p><p style='{margin}'>B</p></div><p style='margin:0'>C</p>"
            ),
        );
        assert_eq!(lines_of(&body, |text| text == "A"), [(1, 5.0)], "{style}");
        assert_eq!(lines_of(&body, |text| text == "B"), [(1, 35.0)], "{style}");
        assert_eq!(
            lines_of(&body, |text| text == "C"),
            [(page, top)],
            "{style}"
        );
    }
}

#[test]
fn height_dependent_descendants_follow_the_growth_of_their_actual_containing_block() {
    for (inset, top) in [("bottom:0", 65.0), ("top:40%", 26.0)] {
        let body = positioned(
            "top:75px",
            &format!("A<div>B<span style='position:absolute;left:40px;{inset}'>T</span></div>C"),
        );
        assert_eq!(lines_of(&body, |text| text == "T"), [(1, top)], "{inset}");
    }
}

#[test]
fn a_size_contained_block_moves_as_a_unit_in_the_positioned_flow() {
    let body = positioned(
        "top:75px",
        "<div style='contain:size;height:60px'><p style='margin:0'>A</p><p style='margin:0'>B</p></div><p style='margin:0'>C</p>",
    );
    for (text, top) in [("A", 5.0), ("B", 35.0), ("C", 65.0)] {
        assert_eq!(lines_of(&body, |found| found == text), [(1, top)], "{text}");
    }
}

#[test]
fn a_size_contained_block_that_fits_keeps_its_page_position() {
    let body = positioned(
        "top:40px",
        "<div style='contain:size;height:30px'>A</div><p style='margin:0'>B</p>",
    );
    assert_eq!(lines_of(&body, |text| text == "A"), [(0, 45.0)]);
    assert_eq!(lines_of(&body, |text| text == "B"), [(0, 75.0)]);
}

#[test]
fn a_float_in_a_following_block_extends_the_positioned_owner_when_it_moves() {
    let body = "<div style='position:relative;height:300px'><div id=o style='position:absolute;left:0;top:75px;width:80px;background:blue'><div style='width:80px'>A<div style='height:10px'><div style='float:left;width:10px;height:150px'></div></div>C</div><p style='margin:0'>D</p></div></div>";
    for (text, top) in [("A", 5.0), ("C", 45.0), ("D", 75.0)] {
        assert_eq!(lines_of(body, |found| found == text), [(1, top)], "{text}");
    }
    let boxes = with_pages(body, |page_index, page| {
        page.fragments()
            .filter(|fragment| page.dom().attr(fragment.node(), "id") == Some("o"))
            .map(|fragment| {
                let rect = fragment.rect();
                (page_index, rect.y, rect.height)
            })
            .collect()
    });
    assert_eq!(boxes, [(0, 75.0, 25.0), (1, 0.0, 100.0), (2, 0.0, 80.0)]);
}

#[test]
fn a_size_contained_absolute_owner_keeps_its_specified_position() {
    let body = positioned(
        "top:75px;contain:size;height:60px",
        "<p style='margin:0'>A</p><p style='margin:0'>B</p>",
    );
    assert_eq!(lines_of(&body, |text| text == "A"), [(0, 80.0)]);
    assert_eq!(lines_of(&body, |text| text == "B"), [(1, 10.0)]);
}

#[test]
fn a_paragraph_holding_its_float_keeps_the_growth_of_its_moved_lines() {
    let body = positioned(
        "top:75px",
        "<div style='display:flow-root;width:80px'><div style='float:left;width:10px;height:150px'></div>A<br>B</div><p style='margin:0'>C</p>",
    );
    assert_eq!(lines_of(&body, |text| text == "A"), [(1, 5.0)]);
    assert_eq!(lines_of(&body, |text| text == "B"), [(1, 35.0)]);
    assert_eq!(lines_of(&body, |text| text == "C"), [(2, 55.0)]);
}

#[test]
fn a_fixed_clip_hides_later_breaks_while_an_auto_clip_grows_with_them() {
    for (style, page, top) in [
        ("height:120px;overflow:hidden", 1, 65.0),
        ("overflow:hidden", 2, 5.0),
    ] {
        let body = positioned(
            "top:75px",
            &format!("<div style='{style}'><p style='margin:0'>A<br>B<br>C<br>D</p></div>"),
        );
        assert_eq!(
            lines_of(&body, |text| text == "C"),
            [(page, top)],
            "{style}"
        );
    }
}

#[test]
fn a_whole_box_move_keeps_a_negative_margin_follower_in_source_order() {
    let body = positioned(
        "top:40px",
        "A<div style='contain:size;height:60px'>B</div><div style='margin-top:-50px'>C</div>D",
    );
    for (text, page, top) in [
        ("A", 0, 45.0),
        ("B", 1, 5.0),
        ("C", 1, 15.0),
        ("D", 1, 45.0),
    ] {
        assert_eq!(
            lines_of(&body, |found| found == text),
            [(page, top)],
            "{text}"
        );
    }
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
fn a_box_with_blocks_among_its_lines_keeps_each_piece_in_flow_order() {
    // Lines and blocks follow the preceding piece: all three are printed,
    // and none lands on another.
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
    for (content, needs_three) in [
        ("Y Y Y Y", true),
        (
            "<p style='margin:0'>A</p><p style='margin:0'>B</p><p style='margin:0'>C</p><p style='margin:0'>D</p>",
            true,
        ),
        (
            "<div style='height:120px;overflow:hidden'><p style='margin:0'>A<br>B<br>C<br>D</p></div>",
            false,
        ),
        (
            "<div style='overflow:hidden'><p style='margin:0'>A<br>B<br>C<br>D</p></div>",
            true,
        ),
    ] {
        let html = format!(
            "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
                body{{margin:0;font:20px/30px Ahem}}</style><body>\
                <div style='position:relative;height:100px'>\
                <div style='position:absolute;left:0;top:75px;width:20px'>{content}</div>\
                </div></body>"
        );
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
        if needs_three {
            assert!(matches!(
                layout_with(2),
                Err(RenderError::LimitExceeded {
                    kind: LimitKind::Pages,
                    limit: 2,
                    actual: 3,
                })
            ));
        } else {
            assert!(matches!(layout_with(2), Ok(LayoutStatus::Completed(_))));
        }
        assert!(matches!(layout_with(3), Ok(LayoutStatus::Completed(_))));
    }
}
