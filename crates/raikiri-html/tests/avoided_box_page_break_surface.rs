//! A box that avoids a break inside it and meets the end of a page moves to the
//! next one. When it is the first thing in the blocks around it, those blocks
//! move with it, as no page breaks between the start of a block and its first
//! child. The positions are the ones Chromium prints for the same documents
//! (`page.pdf` with the page size taken from `@page`, the extents read from the
//! rendered pages by colour).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The vertical extent `(top, bottom)` of the first fragment of the element
/// with the id `id` on each page, `None` for a page without any.
fn extents(body: &str, id: &str) -> Vec<Option<(i32, i32)>> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:8px;font:10px/10px Ahem}}div{{box-sizing:border-box}}</style>{body}"
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
    result
        .pages()
        .map(|page| {
            let dom = page.dom();
            page.fragments()
                .filter(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (rect.y.round() as i32, (rect.y + rect.height).round() as i32)
                })
                .next()
        })
        .collect()
}

const SPACER: &str = "<div style='height:300px'></div>";

fn small_pages(body: &str, id: &str) -> Vec<Option<(i32, i32)>> {
    extents(
        &format!(
            "<style>@page{{size:200px 200px;margin:0}}body{{margin:0;font:8px/10px Ahem}}</style>{body}"
        ),
        id,
    )
}

#[test]
fn an_avoided_chain_moves_before_a_cards_closing_edges_even_when_the_run_is_too_tall() {
    for (card, content, after) in [
        (
            "padding-bottom:30px",
            "<div id=c style='height:60px'>C</div>",
            90,
        ),
        (
            "border-bottom:30px solid",
            "<div id=c style='height:60px'>C</div>",
            90,
        ),
        (
            "padding-bottom:30px;break-inside:avoid",
            "<div id=c style='height:60px'>C</div>",
            90,
        ),
        (
            "padding-bottom:15px",
            "<div style='padding-bottom:15px'><div id=c style='height:60px'>C</div></div>",
            90,
        ),
        (
            "padding-bottom:30px;margin-bottom:20px",
            "<div id=c style='height:60px'>C</div>",
            110,
        ),
        (
            "padding-bottom:10px",
            "<div id=c style='height:60px;margin-bottom:20px'>C</div>",
            90,
        ),
    ] {
        let body = format!(
            "<div style='height:10px'>T</div>\
            <div id=s style='height:130px;break-after:avoid'>S</div>\
            <div id=o style='{card}'>{content}</div>\
            <div id=z style='height:20px'>Z</div>"
        );
        assert_eq!(
            small_pages(&body, "s"),
            [None, Some((0, 130)), None],
            "{card} / {content}"
        );
        assert_eq!(
            small_pages(&body, "c"),
            [None, None, Some((0, 60))],
            "{card} / {content}"
        );
        assert_eq!(
            small_pages(&body, "z"),
            [None, None, Some((after, after + 20))],
            "{card} / {content}"
        );
    }
}

#[test]
fn closing_edges_do_not_make_every_oversized_run_move() {
    for (head, card, content, top, height) in [
        (
            "height:130px",
            "",
            "<div style='height:60px'>C</div>",
            10,
            130,
        ),
        (
            "height:130px",
            "padding-bottom:30px",
            "<div style='height:100px'>C</div>",
            10,
            130,
        ),
        (
            "height:100px",
            "padding-bottom:30px",
            "<div style='height:60px'>C</div>",
            10,
            100,
        ),
        (
            "height:130px",
            "padding-top:10px;padding-bottom:30px",
            "<div style='height:60px'>C</div>",
            10,
            130,
        ),
        (
            "height:130px;margin-top:20px",
            "padding-bottom:30px",
            "<div style='height:60px'>C</div>",
            30,
            130,
        ),
        (
            "height:150px;position:relative;top:10px",
            "padding-bottom:30px",
            "<div style='height:60px'>C</div>",
            20,
            150,
        ),
        (
            "height:150px",
            "padding-bottom:30px;position:relative;top:-10px",
            "<div style='height:60px'>C</div>",
            10,
            150,
        ),
        (
            "height:130px",
            "padding-bottom:30px",
            "<div style='height:30px'>C</div><div style='height:30px'>D</div>",
            10,
            130,
        ),
        (
            "height:130px",
            "padding-bottom:30px",
            "<div style='padding-bottom:20px'><div style='float:left;width:10px;height:70px'></div><div style='height:60px'>C</div></div>",
            10,
            130,
        ),
    ] {
        let body = format!(
            "<div style='height:10px'>T</div>\
            <div id=s style='{head};break-after:avoid'>S</div>\
            <div style='{card}'>{content}</div><div style='height:20px'>Z</div>"
        );
        assert_eq!(
            small_pages(&body, "s")[0],
            Some((top, top + height)),
            "{head} / {card} / {content}"
        );
    }
}

#[test]
fn a_closing_edge_exception_requires_the_content_and_its_floats_to_fit_the_current_page() {
    for (lead, head, floated, moved) in [
        (40, 130, false, false),
        (10, 140, false, false),
        (10, 120, true, true),
    ] {
        let content = if floated {
            "<div style='padding-bottom:20px'><div style='float:left;width:10px;height:70px'></div><div id=c style='height:60px'>C</div></div>"
        } else {
            "<div id=c style='height:60px'>C</div>"
        };
        let body = format!(
            "<div style='height:{lead}px'>T</div>\
            <div id=s style='height:{head}px;break-after:avoid'>S</div>\
            <div style='padding-bottom:30px'>{content}</div><div style='height:20px'>Z</div>"
        );
        if moved {
            assert_eq!(small_pages(&body, "s"), [None, Some((0, head)), None]);
            assert_eq!(small_pages(&body, "c"), [None, None, Some((0, 60))]);
        } else {
            assert_eq!(small_pages(&body, "s")[0], Some((lead, lead + head)));
        }
    }
}

#[test]
fn a_trailing_margin_or_a_forced_child_break_does_not_move_the_chain() {
    for (card, child) in [
        ("padding-bottom:10px;margin-bottom:100px", "height:40px"),
        ("padding-bottom:30px", "height:60px;break-before:page"),
    ] {
        let body = format!(
            "<div style='height:10px'>T</div>\
            <div id=s style='height:130px;break-after:avoid'>S</div>\
            <div style='{card}'><div style='{child}'>C</div></div><div style='height:20px'>Z</div>"
        );
        assert_eq!(
            small_pages(&body, "s")[0],
            Some((10, 140)),
            "{card} / {child}"
        );
    }
}

#[test]
fn the_head_of_the_direct_chain_can_hold_an_avoided_run_of_its_own() {
    let body = "<div style='height:10px'>T</div>\
        <div id=s style='height:130px;break-after:avoid'>\
        <div style='height:60px;break-after:avoid'>A</div><div style='height:70px'>B</div></div>\
        <div style='padding-bottom:30px'><div id=c style='height:60px'>C</div></div>\
        <div style='height:20px'>Z</div>";
    assert_eq!(small_pages(body, "s"), [None, Some((0, 130)), None]);
    assert_eq!(small_pages(body, "c"), [None, None, Some((0, 60))]);
}

#[test]
fn a_chain_at_the_page_start_still_releases_the_card_alone() {
    let body = "<div id=s style='height:130px;break-after:avoid'>S</div>\
        <div style='padding-bottom:30px'><div id=c style='height:60px'>C</div></div>\
        <div id=z style='height:20px'>Z</div>";
    assert_eq!(small_pages(body, "s"), [Some((0, 130)), None]);
    assert_eq!(small_pages(body, "c"), [None, Some((0, 60))]);
    assert_eq!(small_pages(body, "z"), [None, Some((90, 110))]);
    let shifted = body.replace("height:130px", "height:130px;position:relative;top:10px");
    assert_eq!(small_pages(&shifted, "s"), [Some((10, 140)), None]);
}

#[test]
fn a_wrapper_does_not_gain_the_direct_sibling_closing_edge_exception() {
    let body = "<div style='height:40px'>X</div>\
        <div style='padding-bottom:20px'>\
        <div id=s style='height:130px;break-after:avoid'>S</div>\
        <div style='padding-bottom:30px'><div style='height:60px'>C</div></div></div>\
        <div style='height:20px'>Z</div>";
    assert_eq!(small_pages(body, "s")[0], Some((40, 170)));
}

/// A parent `o` with the style `parent`, and a child `i` that is `height`
/// tall and avoids a break inside it, after a spacer.
fn avoiding_child(parent: &str, height: u32) -> String {
    format!(
        "{SPACER}<div id=o style='width:300px;{parent}'>\
         <div id=i style='height:{height}px;break-inside:avoid'></div></div>"
    )
}

#[test]
fn an_avoiding_first_child_takes_its_parent_to_the_next_page() {
    // The child does not fit below the spacer. Chromium prints no part of the
    // parent on the first page, whether or not it has a height of its own.
    for parent in ["height:400px", ""] {
        let body = avoiding_child(parent, 400);
        assert_eq!(
            extents(&body, "o"),
            [None, Some((0, 400))],
            "parent {parent:?}"
        );
        assert_eq!(extents(&body, "i"), [None, Some((0, 400))], "{parent:?}");
    }
}

#[test]
fn white_space_between_the_parent_and_the_child_does_not_hold_the_parent_back() {
    let body = format!(
        "{SPACER}\n<div id=o style='width:300px'>\n  \
         <div id=i style='height:400px;break-inside:avoid'></div>\n</div>\n"
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 400))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 400))]);
}

#[test]
fn every_plain_block_that_the_child_begins_moves_with_it() {
    // The wrapper between them has no edge of its own, so it begins the child
    // too and starts the next page with it.
    let body = format!(
        "{SPACER}<div id=o style='width:300px'>\
         <div style='background:#00f'>{}</div></div>",
        "<div id=i style='height:400px;break-inside:avoid'></div>"
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 400))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 400))]);
    // A margin above the child collapses through the parent and is cut at the
    // top of the page.
    let body =
        avoiding_child("", 400).replace("break-inside:avoid", "break-inside:avoid;margin-top:30px");
    assert_eq!(extents(&body, "o"), [None, Some((0, 400))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 400))]);
}

#[test]
fn the_flow_after_the_parent_follows_it_to_the_next_page() {
    let body = format!(
        "{}<div id=s style='height:50px;width:100px'></div>",
        avoiding_child("", 400)
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 400))]);
    assert_eq!(extents(&body, "s"), [None, Some((400, 450))]);
    // What follows the child inside the parent stays with it.
    let body = format!(
        "{SPACER}<div id=o style='width:300px'>\
         <div id=i style='height:400px;break-inside:avoid'></div>\
         <div id=s style='height:50px'></div></div>"
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 450))]);
    assert_eq!(extents(&body, "s"), [None, Some((400, 450))]);
}

#[test]
fn a_parent_that_something_precedes_the_child_in_stays_on_its_page() {
    let body = format!(
        "{SPACER}<div id=o style='width:300px'><div style='height:20px'></div>\
         <div id=i style='height:400px;break-inside:avoid'></div></div>"
    );
    assert_eq!(extents(&body, "o"), [Some((308, 600)), Some((0, 400))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 400))]);
}

#[test]
fn a_child_taller_than_a_page_starts_the_next_one_and_takes_its_parent() {
    // It does not fit the next page either, but it starts that page rather
    // than being cut at the end of this one: Chromium prints no part of the
    // parent on the first page.
    let body = avoiding_child("", 700);
    assert_eq!(extents(&body, "o")[0], None);
    assert_eq!(extents(&body, "i"), [None, Some((0, 600)), Some((0, 100))]);
}

#[test]
fn a_top_edge_on_the_parent_does_not_keep_it_on_the_page_the_child_leaves() {
    // The page breaks before the parent rather than between its edge and the
    // child. The child stays below the edge on the next page and splits there
    // when it is taller than the room that is left.
    for parent in ["padding-top:20px", "border-top:20px solid"] {
        let body = avoiding_child(parent, 700);
        assert_eq!(
            extents(&body, "o"),
            [None, Some((0, 600)), Some((0, 120))],
            "{parent}"
        );
        assert_eq!(
            extents(&body, "i"),
            [None, Some((20, 600)), Some((0, 120))],
            "{parent}"
        );
    }
    // A child that fits a page but not the room below the edge splits too.
    let body = avoiding_child("padding-top:200px", 500);
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 100))]);
    assert_eq!(
        extents(&body, "i"),
        [None, Some((200, 600)), Some((0, 100))]
    );
    // One that fits below the edge is not cut.
    let body = avoiding_child("padding-top:20px", 500);
    assert_eq!(extents(&body, "o"), [None, Some((0, 520))]);
    assert_eq!(extents(&body, "i"), [None, Some((20, 520))]);
    // Nor is one that fits where it is.
    let body = avoiding_child("padding-top:20px", 250);
    assert_eq!(extents(&body, "o"), [Some((308, 578))]);
    assert_eq!(extents(&body, "i"), [Some((328, 578))]);
}

#[test]
fn the_offset_of_the_child_below_the_top_edge_of_its_moved_parent_adds_up() {
    // A margin above the child stays between the edge and the child.
    let body = avoiding_child("padding-top:20px", 700)
        .replace("break-inside:avoid", "break-inside:avoid;margin-top:10px");
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 130))]);
    assert_eq!(extents(&body, "i"), [None, Some((30, 600)), Some((0, 130))]);
    // So does the edge of a wrapper between them.
    let body = format!(
        "{SPACER}<div id=o style='width:300px;padding-top:20px'>\
         <div style='padding-top:10px'>\
         <div id=i style='height:700px;break-inside:avoid'></div></div></div>"
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 130))]);
    assert_eq!(extents(&body, "i"), [None, Some((30, 600)), Some((0, 130))]);
    // An edge below the child is no reason to keep the parent in place.
    let body = avoiding_child("padding:20px 0 30px", 700);
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 150))]);
    assert_eq!(extents(&body, "i"), [None, Some((20, 600)), Some((0, 120))]);
}

#[test]
fn the_margins_of_the_wrappers_between_the_parent_and_the_child_add_to_the_offset_too() {
    // A margin on a wrapper without edges collapses with the one on the child,
    // the larger of them stays below the edge of the parent; the edge of a
    // wrapper separates its own margin from the child's.
    let wrapped = |wrapper: &str, child: &str| {
        format!(
            "{SPACER}<div id=o style='width:300px;padding-top:20px'>\
             <div style='{wrapper}'>\
             <div id=i style='height:700px;break-inside:avoid;{child}'></div></div></div>"
        )
    };
    for (wrapper, child, top) in [
        ("margin-top:10px", "", 30),
        ("margin-top:10px", "margin-top:15px", 35),
        ("margin-top:10px", "margin-top:4px", 30),
        ("margin-top:10px;padding-top:5px", "", 35),
        ("margin-top:10px;border-top:5px solid", "", 35),
        // A block that establishes a formatting context keeps the margin of
        // its first child inside it, apart from its own.
        ("margin-top:10px;display:flow-root", "margin-top:15px", 45),
        ("margin-top:10px;overflow:hidden", "margin-top:15px", 45),
        ("margin-top:10px;display:flow-root", "", 30),
    ] {
        let body = wrapped(wrapper, child);
        let name = format!("wrapper {wrapper:?} child {child:?}");
        assert_eq!(
            extents(&body, "o"),
            [None, Some((0, 600)), Some((0, top - 20 + 120))],
            "{name}"
        );
        assert_eq!(
            extents(&body, "i"),
            [None, Some((top, 600)), Some((0, top - 20 + 120))],
            "{name}"
        );
    }
    // Two wrappers collapse their margins into the larger.
    let body = format!(
        "{SPACER}<div id=o style='width:300px;padding-top:20px'>\
         <div style='margin-top:10px'><div style='margin-top:20px'>\
         <div id=i style='height:700px;break-inside:avoid'></div></div></div></div>"
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 140))]);
    assert_eq!(extents(&body, "i"), [None, Some((40, 600)), Some((0, 140))]);
}

#[test]
fn a_percentage_margin_below_the_top_edge_is_resolved_against_the_parent() {
    // The parent is wider than the page, so the 5% of its width (CSS 2
    // section 8.3) is more than 5% of the page. Following that rule rather
    // than a Chromium print, which scales a page that wide to fit.
    let body = format!(
        "{SPACER}<div id=o style='width:1600px;padding-top:20px'>\
         <div id=i style='height:700px;break-inside:avoid;margin-top:5%'></div></div>"
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 200))]);
    assert_eq!(
        extents(&body, "i"),
        [None, Some((100, 600)), Some((0, 200))]
    );
}

/// A parent `o` with the style `parent` after a spacer, holding a paragraph
/// `i` of `lines` lines that avoids a break inside it.
fn avoiding_paragraph(parent: &str, own: &str, lines: usize) -> String {
    format!(
        "{SPACER}<div id=o style='width:300px;{parent}'>\
         <p id=i style='margin:0;break-inside:avoid;{own}'>{}</p></div>",
        vec!["L"; lines].join("<br>")
    )
}

#[test]
fn an_avoiding_paragraph_that_fits_a_page_without_the_edges_below_it_gives_up_its_last_lines() {
    // The paragraph moves to the next page, where its 58 lines fit the page
    // but the padding below them does not. The page breaks between its lines
    // rather than between the last line and the padding, so the padding is
    // not left alone: the last two lines go on with it. The first fragment
    // runs to the page end.
    for parent in ["padding-bottom:30px", "border-bottom:30px solid"] {
        let body = avoiding_paragraph(parent, "", 58);
        assert_eq!(
            extents(&body, "o"),
            [None, Some((0, 600)), Some((0, 50))],
            "{parent}"
        );
        assert_eq!(
            extents(&body, "i"),
            [None, Some((0, 600)), Some((0, 20))],
            "{parent}"
        );
    }
    // The edges of the paragraph itself, and of two blocks around it.
    let body = avoiding_paragraph("", "padding-bottom:30px", 58);
    assert_eq!(extents(&body, "i"), [None, Some((0, 600)), Some((0, 50))]);
    let body = format!(
        "{SPACER}<div id=o style='width:300px;padding-bottom:10px'>\
         <div style='padding-bottom:20px'><p id=i style='margin:0;break-inside:avoid'>{}</p>\
         </div></div>",
        vec!["L"; 58].join("<br>")
    );
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 50))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 600)), Some((0, 20))]);
    // A padding above the paragraph goes with it to the next page.
    let body = avoiding_paragraph("padding:20px 0 30px", "", 58);
    assert_eq!(extents(&body, "o"), [None, Some((0, 600)), Some((0, 50))]);
    assert_eq!(extents(&body, "i"), [None, Some((20, 600)), Some((0, 20))]);
    // One more line gives up the same two.
    let body = avoiding_paragraph("padding-bottom:30px", "", 59);
    assert_eq!(extents(&body, "i"), [None, Some((0, 600)), Some((0, 20))]);
}

#[test]
fn an_avoiding_paragraph_stays_whole_where_the_edges_fit_a_page_with_it() {
    // Together they fit the next page, so they move there as a whole.
    let body = avoiding_paragraph("padding-bottom:30px", "", 30);
    assert_eq!(extents(&body, "o"), [None, Some((0, 330))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 300))]);
    // Exactly a page.
    let body = avoiding_paragraph("padding-bottom:30px", "", 57);
    assert_eq!(extents(&body, "o"), [None, Some((0, 600))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 570))]);
    // Without edges below it, the paragraph is not cut.
    let body = avoiding_paragraph("", "", 58);
    assert_eq!(extents(&body, "i"), [None, Some((0, 580))]);
}

/// A parent `o` with the style `parent` that holds a spacer of `before` tall
/// and then the box `i`, which avoids a break inside it.
fn avoiding_box_after(parent: &str, before: u32, inner: &str) -> String {
    format!(
        "<div id=o style='width:300px;{parent}'><div style='height:{before}px'></div>\
         <div id=i style='break-inside:avoid'>{inner}</div></div>"
    )
}

#[test]
fn an_avoiding_box_after_another_box_goes_to_the_next_page_when_the_edges_run_past_it() {
    // The box fits its page and the padding below it does not, so the page
    // breaks before the box, not between the box and the padding, and not
    // inside the box. Box and padding fill the next page exactly.
    let lines = |count: usize| vec!["L"; count].join("<br>");
    let body = avoiding_box_after("padding-bottom:30px", 10, &lines(57));
    assert_eq!(extents(&body, "o"), [Some((8, 600)), Some((0, 600))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 570))]);
    // The same box when it does not fit a page together with the padding:
    // it goes to the next page and gives up its last lines there.
    let body = avoiding_box_after("padding-bottom:30px", 10, &lines(58));
    assert_eq!(
        extents(&body, "o"),
        [Some((8, 600)), Some((0, 600)), Some((0, 50))]
    );
    assert_eq!(extents(&body, "i"), [None, Some((0, 600)), Some((0, 20))]);
    // A box made of blocks keeps them together the same way.
    let body = avoiding_box_after(
        "padding-bottom:30px",
        300,
        "<div style='height:200px'></div><div style='height:90px'></div>",
    );
    assert_eq!(extents(&body, "o"), [Some((8, 600)), Some((0, 320))]);
    assert_eq!(extents(&body, "i"), [None, Some((0, 290))]);
    // A box taller than a page is no better off on the next page, but the
    // page still breaks before it; its last lines go on with the padding.
    let body = avoiding_box_after("padding-bottom:30px", 10, &lines(65));
    assert_eq!(
        extents(&body, "o"),
        [Some((8, 600)), Some((0, 600)), Some((0, 80))]
    );
    assert_eq!(extents(&body, "i"), [None, Some((0, 600)), Some((0, 50))]);
    // Without a padding below it, the box stays.
    let body = avoiding_box_after("", 10, &lines(58));
    assert_eq!(extents(&body, "i"), [Some((18, 598))]);
}

#[test]
fn the_sizes_of_the_parent_do_not_stop_it_from_moving() {
    // A min-height shows below the child, a fixed height or a max-height
    // shorter than the child lets the child overflow, and a clip cuts at the
    // parent's own box.
    for (parent, parent_extent) in [
        ("min-height:450px", (0, 450)),
        ("height:200px", (0, 200)),
        ("max-height:300px", (0, 300)),
        ("overflow:hidden", (0, 400)),
    ] {
        let body = avoiding_child(parent, 400);
        assert_eq!(
            extents(&body, "o"),
            [None, Some(parent_extent)],
            "parent {parent:?}"
        );
        assert_eq!(extents(&body, "i"), [None, Some((0, 400))], "{parent:?}");
    }
}

#[test]
fn a_child_on_a_later_page_takes_its_parent_to_the_page_after_it() {
    // The spacer fills the first page and 308px of the second, so the child
    // crosses the end of the second page.
    let body = "<div style='height:900px'></div>\
         <div id=o style='width:300px'>\
         <div id=i style='height:400px;break-inside:avoid'></div></div>";
    assert_eq!(extents(body, "o"), [None, None, Some((0, 400))]);
    assert_eq!(extents(body, "i"), [None, None, Some((0, 400))]);
}

#[test]
fn a_float_moves_with_its_first_avoiding_child() {
    for side in ["left", "right"] {
        let body = format!(
            "{SPACER}<div id=o style='width:300px;float:{side};background:blue'><div id=i style='height:400px;background:green;break-inside:avoid'></div></div>"
        );
        for id in ["o", "i"] {
            assert_eq!(extents(&body, id), [None, Some((0, 400))], "{side} / {id}");
        }
    }
}

#[test]
fn a_nested_float_moves_with_its_first_avoiding_child() {
    let body = format!(
        "{SPACER}<section><div id=o style='width:300px;float:left'><div id=i style='height:400px;break-inside:avoid'></div></div></section>"
    );
    for id in ["o", "i"] {
        assert_eq!(extents(&body, id), [None, Some((0, 400))], "{id}");
    }
}

#[test]
fn a_float_stays_when_only_the_tail_after_its_first_avoiding_piece_crosses_the_page() {
    let body = format!(
        "{SPACER}<div id=o style='width:300px;float:left'><div id=i style='height:200px;break-inside:avoid'></div><div style='height:400px'></div></div>"
    );
    assert_eq!(extents(&body, "o"), [Some((308, 600)), Some((0, 308))]);
    assert_eq!(extents(&body, "i"), [Some((308, 508)), None]);
}

#[test]
fn a_float_with_splittable_content_keeps_its_existing_page_break() {
    let body = format!(
        "{SPACER}<div id=o style='width:300px;float:left'><div id=i style='height:400px'></div></div>"
    );
    for id in ["o", "i"] {
        assert_eq!(
            extents(&body, id),
            [Some((308, 600)), Some((0, 108))],
            "{id}"
        );
    }
}

#[test]
fn a_float_does_not_leave_a_page_its_first_piece_already_starts() {
    for prefix in [
        "<style>body{margin:0}</style>",
        "<div style='height:300px;break-after:page'></div>",
    ] {
        let body = format!(
            "{prefix}<div id=o style='width:300px;float:left'><div id=i style='height:400px;break-inside:avoid'></div></div>"
        );
        let expected = if prefix.starts_with("<style>") {
            vec![Some((0, 400))]
        } else {
            vec![None, Some((0, 400))]
        };
        for id in ["o", "i"] {
            assert_eq!(extents(&body, id), expected, "{prefix} / {id}");
        }
    }
}

#[test]
fn an_oversized_first_avoiding_piece_starts_its_float_on_the_next_page() {
    let body = format!(
        "{SPACER}<div id=o style='width:300px;float:left'><div id=i style='height:750px;break-inside:avoid'></div></div>"
    );
    for id in ["o", "i"] {
        assert_eq!(
            extents(&body, id),
            [None, Some((0, 600)), Some((0, 150))],
            "{id}"
        );
    }
}

#[test]
fn a_cards_closing_edges_follow_the_float_that_extends_below_its_last_box() {
    for (outer, card, child, float, prefix, c_top, f_top, z_top) in [
        ("", "padding-bottom:30px", "", "float:left", "", 0, 20, 90),
        (
            "",
            "border-bottom:30px solid",
            "",
            "float:right",
            "",
            0,
            20,
            90,
        ),
        (
            "padding-bottom:10px",
            "padding-bottom:20px",
            "",
            "float:left",
            "",
            0,
            20,
            90,
        ),
        (
            "",
            "padding-bottom:30px",
            "",
            "float:left",
            "<div style='height:10px'>S</div>",
            0,
            20,
            90,
        ),
        (
            "",
            "padding-bottom:30px;position:relative;top:7px",
            "",
            "float:left",
            "",
            7,
            27,
            90,
        ),
        (
            "",
            "padding-bottom:30px",
            "position:relative;top:7px",
            "float:left",
            "",
            7,
            20,
            90,
        ),
        (
            "",
            "padding-bottom:30px",
            "",
            "float:left;position:relative;top:7px",
            "",
            0,
            // A float's layout rectangle is in flow coordinates; paint adds
            // its 7px relative offset, as on the unfragmented control.
            20,
            90,
        ),
        (
            "",
            "padding-bottom:30px",
            "",
            "float:left;margin-bottom:5px",
            "",
            0,
            20,
            95,
        ),
        (
            "",
            "padding-bottom:40px",
            "margin-bottom:-10px",
            "float:left",
            "",
            0,
            10,
            90,
        ),
    ] {
        let body = format!(
            "{prefix}<div style='{outer}'><div id=o style='display:flow-root;{card}'>\
            <div style='height:60px'>A</div><div style='height:60px'>B</div>\
            <div id=c style='height:20px;{child}'>C</div>\
            <div id=f style='{float};width:20px;height:40px'>f</div></div></div>\
            <div id=z style='height:20px'>Z</div>"
        );
        for (id, top, height) in [("c", c_top, 20), ("f", f_top, 40), ("z", z_top, 20)] {
            assert_eq!(
                small_pages(&body, id),
                [None, Some((top, top + height))],
                "{id}: {body}"
            );
        }
    }
}

#[test]
fn closing_edges_distinguish_escaping_fitting_and_crossing_floats() {
    for (display, padding, float_height, child_style, expected_c, expected_f, expected_z) in [
        (
            "block",
            30,
            40,
            "",
            vec![Some((120, 140))],
            vec![Some((140, 180))],
            vec![Some((170, 190))],
        ),
        (
            "flow-root",
            20,
            40,
            "",
            vec![Some((120, 140)), None],
            vec![Some((140, 180)), None],
            vec![None, Some((0, 20))],
        ),
        (
            "flow-root",
            30,
            70,
            "",
            vec![Some((120, 140)), None],
            vec![Some((140, 200)), Some((0, 10))],
            vec![None, Some((40, 60))],
        ),
        (
            "flow-root",
            30,
            40,
            "margin-bottom:-10px",
            vec![Some((120, 140)), None],
            vec![Some((130, 170)), None],
            vec![None, Some((0, 20))],
        ),
    ] {
        let body = format!(
            "<div style='display:{display};padding-bottom:{padding}px'>\
            <div style='height:60px'>A</div><div style='height:60px'>B</div>\
            <div id=c style='height:20px;{child_style}'>C</div>\
            <div id=f style='float:left;width:20px;height:{float_height}px'>f</div></div>\
            <div id=z style='height:20px'>Z</div>"
        );
        for (id, expected) in [("c", expected_c), ("f", expected_f), ("z", expected_z)] {
            assert_eq!(small_pages(&body, id), expected, "{id}: {body}");
        }
    }
}

#[test]
fn a_preceding_float_stays_while_the_last_box_moves_with_closing_edges() {
    let body = "<div style='display:flow-root;padding-bottom:30px'>\
        <div id=f style='float:left;width:20px;height:180px'>f</div>\
        <div style='height:60px'>A</div><div style='height:60px'>B</div>\
        <div id=c style='height:20px'>C</div></div>\
        <div id=z style='height:20px'>Z</div>";
    assert_eq!(small_pages(body, "f"), [Some((0, 180)), None]);
    assert_eq!(small_pages(body, "c"), [None, Some((0, 20))]);
    assert_eq!(small_pages(body, "z"), [None, Some((50, 70))]);
}

#[test]
fn an_avoided_chain_moves_with_the_enclosed_float_and_closing_edges() {
    let body = "<div style='display:flow-root;padding-bottom:30px'>\
        <div style='height:60px'>A</div><div id=b style='height:60px'>B</div>\
        <div id=c style='height:20px;break-before:avoid'>C</div>\
        <div id=f style='float:left;width:20px;height:40px'>f</div></div>\
        <div id=z style='height:20px'>Z</div>";
    for (id, top, height) in [("b", 0, 60), ("c", 60, 20), ("f", 80, 40), ("z", 150, 20)] {
        assert_eq!(
            small_pages(body, id),
            [None, Some((top, top + height))],
            "{id}"
        );
    }
}
