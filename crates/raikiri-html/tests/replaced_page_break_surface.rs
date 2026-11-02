//! Replaced elements and lines of atomic inlines at the end of a page. No page
//! break goes through them: one that would cross the end of a page starts the
//! next one, unless it starts the page it is on. The positions are the ones
//! Chromium prints for the same documents (`page.pdf` with the page size taken
//! from `@page`).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver, ResolveDisposition,
    ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use url::Url;

/// Every picture is 20px wide and 50px tall.
struct Portrait;

impl ReplacedResolver for Portrait {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(20.0, 50.0),
            ResolveDisposition::Ok,
        ))
    }
}

const IMAGE: &str = "<img id=i src='portrait.png'";

/// The vertical extent `(top, bottom)` of the fragments of the element with
/// the id `i` on each page, `None` for a page without any.
fn image_extents(body: &str) -> Vec<Option<(i32, i32)>> {
    extents_of("i", body)
}

/// The vertical extent `(top, bottom)` of the first fragment of the element
/// with the id `id` on each page, `None` for a page without any.
fn extents_of(id: &str, body: &str) -> Vec<Option<(i32, i32)>> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://example.test/document.html").unwrap())
        .replaced_resolver(&Portrait);
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

/// An image that is `height` tall and a block.
fn block_image(height: u32) -> String {
    format!("{IMAGE} style='display:block;width:300px;height:{height}px'>")
}

#[test]
fn an_image_taller_than_the_page_starts_the_second_page_when_a_margin_keeps_it_from_the_first() {
    // The body's margin is kept at the top of the first page, so the image
    // does not start it: the first page is left blank, and the image is cut
    // into pages from the top of the second.
    let wrapped = format!("<div id=o style='width:300px'>{}</div>", block_image(750));
    for body in [wrapped, block_image(750)] {
        assert_eq!(
            image_extents(&body),
            [None, Some((0, 600)), Some((0, 150))],
            "{body}"
        );
    }
}

#[test]
fn an_image_that_crosses_the_page_end_by_a_few_pixels_moves_whole() {
    assert_eq!(image_extents(&block_image(590)), [Some((8, 598))]);
    assert_eq!(image_extents(&block_image(595)), [None, Some((0, 595))]);
}

#[test]
fn an_image_after_a_box_starts_the_next_page() {
    let spacer = "<div style='height:300px'></div>";
    assert_eq!(
        image_extents(&format!("{spacer}{}", block_image(400))),
        [None, Some((0, 400))]
    );
    // A taller image than a page is cut from the top of the next one.
    assert_eq!(
        image_extents(&format!("{spacer}{}", block_image(750))),
        [None, Some((0, 600)), Some((0, 150))]
    );
    // Nested in a list item, it moves all the same.
    assert_eq!(
        image_extents(&format!(
            "{spacer}<ul style='margin:0;padding:0;list-style:none'><li>{}</li></ul>",
            block_image(400)
        )),
        [None, Some((0, 400))]
    );
}

#[test]
fn an_image_alone_in_its_line_follows_the_same_rule() {
    let spacer = "<div style='height:300px'></div>";
    assert_eq!(
        image_extents(&format!(
            "{spacer}<p style='margin:0'>{IMAGE} style='width:300px;height:400px'></p>"
        )),
        [None, Some((0, 400))]
    );
    assert_eq!(
        image_extents(&format!(
            "<p style='margin:0'>{IMAGE} style='width:300px;height:750px'></p>"
        )),
        [None, Some((0, 600)), Some((0, 150))]
    );
}

#[test]
fn a_block_that_only_avoids_a_break_inside_it_stays_where_its_margin_starts_the_page() {
    // The block starts the page apart from the margin of the body, so it is
    // cut at the page end like any other block, where an image would move.
    let avoiding = "<div id=i style='width:300px;height:595px;break-inside:avoid'></div>";
    assert_eq!(
        image_extents(avoiding),
        [Some((8, 600)), Some((0, 3))],
        "{avoiding}"
    );
}

/// An image `height` tall with a 30px margin above and below it.
fn margined_image(height: u32, extra: &str) -> String {
    format!("{IMAGE} style='display:block;width:300px;height:{height}px;margin:30px 0;{extra}'>")
}

#[test]
fn a_margin_kept_after_a_forced_break_moves_an_image_that_crosses_the_page_end_on() {
    // A forced break keeps the margin above the box after it, so the image
    // does not start its page: one that fits below the margin stays, one that
    // crosses the page end moves to the page after, where the margin is cut.
    let forced = "<div style='height:100px;break-after:page'></div>";
    let after = |height| image_extents(&format!("{forced}{}", margined_image(height, "")));
    assert_eq!(after(500), [None, Some((30, 530))]);
    assert_eq!(after(570), [None, Some((30, 600))]);
    assert_eq!(after(575), [None, None, Some((0, 575))]);
    assert_eq!(after(750), [None, None, Some((0, 600)), Some((0, 150))]);
}

#[test]
fn a_forced_break_leaves_an_image_that_starts_its_page_or_floats_where_it_is() {
    let forced = "<div style='height:100px;break-after:page'></div>";
    // Without a margin the image starts its page and is cut there.
    assert_eq!(
        image_extents(&format!(
            "{forced}{IMAGE} style='display:block;width:300px;height:750px'>"
        )),
        [None, Some((0, 600)), Some((0, 150))]
    );
    // A float is cut at its margin like any other box that crosses the edge.
    assert_eq!(
        image_extents(&format!("{forced}{}", margined_image(575, "float:left"))),
        [None, Some((30, 600)), Some((0, 5))]
    );
}

#[test]
fn a_forced_break_before_an_image_moves_it_on_the_same_way() {
    let spacer = "<div style='height:100px'></div>";
    // On the image itself.
    assert_eq!(
        image_extents(&format!(
            "{spacer}{}",
            margined_image(575, "break-before:page")
        )),
        [None, None, Some((0, 575))]
    );
    // In a wrapper, after a forced break inside it or before the image.
    assert_eq!(
        image_extents(&format!(
            "<div><div style='height:100px;break-after:page'></div>{}</div>",
            margined_image(575, "")
        )),
        [None, None, Some((0, 575))]
    );
    assert_eq!(
        image_extents(&format!(
            "{spacer}<div>{}</div>",
            margined_image(575, "break-before:page")
        )),
        [None, None, Some((0, 575))]
    );
}

#[test]
fn a_wrapper_with_a_height_moves_to_the_next_page_with_the_image_that_starts_it() {
    // The wrapper begins with the image, so the page breaks before both: it
    // keeps its own height on the next page rather than leaving a stub of
    // itself at the foot of the first.
    let spacer = "<div style='height:300px'></div>";
    for (wrapper_style, expected_wrapper) in [
        ("height:400px", (0, 400)),
        ("height:500px", (0, 500)),
        ("", (0, 400)),
    ] {
        let body = format!(
            "{spacer}<div id=o style='width:300px;{wrapper_style}'>{}</div>",
            block_image(400)
        );
        assert_eq!(
            extents_of("o", &body),
            [None, Some(expected_wrapper)],
            "{body}"
        );
        assert_eq!(image_extents(&body), [None, Some((0, 400))], "{body}");
    }
}

#[test]
fn the_flow_after_a_wrapper_that_moves_with_its_image_follows_the_wrapper() {
    let spacer = "<div style='height:300px'></div>";
    let trailer = "<div id=t style='width:100px;height:50px'></div>";
    let cases = [
        // The next box starts below the wrapper at its place on the new page.
        (
            format!(
                "<div style='width:300px;height:400px'>{}</div>",
                block_image(400)
            ),
            (400, 450),
        ),
        // The margin below the wrapper is kept, the one above it is cut.
        (
            format!(
                "<div style='width:300px;height:400px;margin:20px 0'>{}</div>",
                block_image(400)
            ),
            (420, 470),
        ),
        // Both wrappers begin with the image, and the outer one goes on past it.
        (
            format!(
                "<div style='width:300px'><div style='height:400px'>{}</div>\
                 <div style='height:30px'></div></div>",
                block_image(400)
            ),
            (430, 480),
        ),
    ];
    for (wrapper, trailer_extent) in cases {
        let body = format!("{spacer}{wrapper}{trailer}");
        assert_eq!(
            extents_of("t", &body),
            [None, Some(trailer_extent)],
            "{body}"
        );
        assert_eq!(image_extents(&body), [None, Some((0, 400))], "{body}");
    }
}

#[test]
fn a_wrapper_with_something_before_the_image_stays_on_the_page_and_continues_after_it() {
    // The ten pixels before the image render on the first page, so the wrapper
    // starts there and runs on to the next with the height it has left.
    let spacer = "<div style='height:300px'></div>";
    let body = format!(
        "{spacer}<div id=o style='width:300px;height:500px'>\
         <div style='height:10px'></div>{}</div>",
        block_image(400)
    );
    assert_eq!(
        extents_of("o", &body),
        [Some((308, 600)), Some((0, 208))],
        "{body}"
    );
    assert_eq!(image_extents(&body), [None, Some((0, 400))], "{body}");
}

#[test]
fn a_clipping_wrapper_with_a_height_moves_to_the_next_page_with_the_image_that_starts_it() {
    // The clipping gives the wrapper a height of its own, but the image is
    // not cut at the page end any more than in a wrapper that grows: the page
    // breaks before both.
    let spacer = "<div style='height:300px'></div>";
    let trailer = "<div id=t style='width:100px;height:50px'></div>";
    for (wrapper_style, wrapper_extent, trailer_extent) in [
        ("height:400px;overflow:hidden", (0, 400), (400, 450)),
        ("height:500px;overflow:hidden", (0, 500), (500, 550)),
        ("height:400px;overflow:clip", (0, 400), (400, 450)),
        ("height:400px;overflow-x:hidden", (0, 400), (400, 450)),
        ("overflow:hidden", (0, 400), (400, 450)),
    ] {
        let body = format!(
            "{spacer}<div id=o style='width:300px;{wrapper_style}'>{}</div>{trailer}",
            block_image(400)
        );
        assert_eq!(
            extents_of("o", &body),
            [None, Some(wrapper_extent)],
            "{body}"
        );
        assert_eq!(
            extents_of("t", &body),
            [None, Some(trailer_extent)],
            "{body}"
        );
        assert_eq!(image_extents(&body), [None, Some((0, 400))], "{body}");
    }
}

#[test]
fn clipping_wrappers_nested_around_an_image_move_with_it() {
    let spacer = "<div style='height:300px'></div>";
    let trailer = "<div id=t style='width:100px;height:50px'></div>";
    for (wrapper, wrapper_extent, trailer_extent) in [
        (
            format!(
                "<div id=o style='width:300px;height:450px;overflow:hidden'>\
                 <div style='height:420px'>{}</div></div>",
                block_image(400)
            ),
            (0, 450),
            (450, 500),
        ),
        (
            format!(
                "<div id=o style='width:300px;height:450px;overflow:hidden'>\
                 <div style='height:420px;overflow:hidden'>{}</div></div>",
                block_image(400)
            ),
            (0, 450),
            (450, 500),
        ),
    ] {
        let body = format!("{spacer}{wrapper}{trailer}");
        assert_eq!(
            extents_of("o", &body),
            [None, Some(wrapper_extent)],
            "{body}"
        );
        assert_eq!(
            extents_of("t", &body),
            [None, Some(trailer_extent)],
            "{body}"
        );
        assert_eq!(image_extents(&body), [None, Some((0, 400))], "{body}");
    }
}

#[test]
fn a_clipping_wrapper_that_moves_continues_on_the_page_after_with_what_follows_the_image() {
    let spacer = "<div style='height:300px'></div>";
    let body = format!(
        "{spacer}<div id=o style='width:300px;height:700px;overflow:hidden'>{}\
         <div style='height:100px'></div></div>\
         <div id=t style='width:100px;height:50px'></div>",
        block_image(400)
    );
    assert_eq!(
        extents_of("o", &body),
        [None, Some((0, 600)), Some((0, 100))],
        "{body}"
    );
    assert_eq!(image_extents(&body), [None, Some((0, 400)), None], "{body}");
    assert_eq!(
        extents_of("t", &body),
        [None, None, Some((100, 150))],
        "{body}"
    );
}

#[test]
fn an_image_that_the_clipping_wrapper_cuts_short_moves_only_when_the_wrapper_crosses_the_page_end()
{
    let trailer = "<div id=t style='width:100px;height:50px'></div>";
    let wrapper = format!(
        "<div id=o style='width:300px;height:200px;overflow:hidden'>{}</div>{trailer}",
        block_image(400)
    );
    // The 200px that show end above the page end: the image is cut by the
    // wrapper there and nothing moves.
    let body = format!("<div style='height:300px'></div>{wrapper}");
    assert_eq!(extents_of("o", &body), [Some((308, 508))], "{body}");
    assert_eq!(extents_of("t", &body), [Some((508, 558))], "{body}");
    // They cross it, so the wrapper goes to the next page whole.
    let body = format!("<div style='height:480px'></div>{wrapper}");
    assert_eq!(extents_of("o", &body), [None, Some((0, 200))], "{body}");
    assert_eq!(extents_of("t", &body), [None, Some((200, 250))], "{body}");
}
