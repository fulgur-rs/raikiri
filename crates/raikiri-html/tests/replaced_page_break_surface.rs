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
                .filter(|fragment| dom.attr(fragment.node(), "id") == Some("i"))
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
