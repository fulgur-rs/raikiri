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
