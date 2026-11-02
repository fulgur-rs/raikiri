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
fn a_column_flex_container_moves_with_its_first_replaced_item() {
    let body = format!(
        "<div style='height:300px'></div>\
         <div id=o style='display:flex;flex-direction:column;width:300px;height:400px'>\
         {}</div>",
        block_image(400)
    );
    assert_eq!(extents_of("o", &body), [None, Some((0, 400))]);
    assert_eq!(image_extents(&body), [None, Some((0, 400))]);
}

fn column_image(height: u32, css: &str) -> String {
    format!("{IMAGE} style='display:block;width:300px;height:{height}px;flex-shrink:0;{css}'>")
}

fn column_after_lead(items: &str, css: &str) -> String {
    format!(
        "<div style='height:300px'></div>\
         <div id=o style='display:flex;flex-direction:column;width:300px;{css}'>{items}</div>"
    )
}

#[test]
fn a_leading_column_image_moves_following_items_and_outer_flow_once() {
    for tail in [
        "<div id=t style='height:20px;flex-shrink:0'></div>",
        "<div id=t style='height:20px;flex-shrink:0'>T</div>",
    ] {
        let items = format!("{}{tail}", column_image(400, ""));
        let body = format!(
            "{}<div id=a style='height:10px'></div>",
            column_after_lead(&items, "")
        );
        assert_eq!(extents_of("o", &body), [None, Some((0, 420))]);
        assert_eq!(image_extents(&body), [None, Some((0, 400))]);
        assert_eq!(extents_of("t", &body), [None, Some((400, 420))]);
        assert_eq!(extents_of("a", &body), [None, Some((420, 430))]);
    }
}

#[test]
fn column_image_placement_uses_order_and_reverse_visual_order() {
    let early_image = format!(
        "<div style='height:20px;order:1;flex-shrink:0'></div>{}",
        column_image(400, "order:0")
    );
    let reverse_image = format!(
        "<div style='height:20px;flex-shrink:0'></div>{}",
        column_image(400, "")
    );
    for (items, css) in [
        (early_image, ""),
        (reverse_image, "flex-direction:column-reverse"),
    ] {
        let body = column_after_lead(&items, css);
        assert_eq!(extents_of("o", &body), [None, Some((0, 420))]);
        assert_eq!(image_extents(&body), [None, Some((0, 400))]);
    }
    let late_image = format!(
        "{}<div style='height:20px;order:0;flex-shrink:0'></div>",
        column_image(400, "order:1")
    );
    let reverse_late = format!(
        "{}<div style='height:20px;flex-shrink:0'></div>",
        column_image(400, "")
    );
    for (items, css) in [
        (late_image, ""),
        (reverse_late, "flex-direction:column-reverse"),
    ] {
        let body = column_after_lead(&items, css);
        assert_eq!(extents_of("o", &body)[0], Some((308, 600)));
        assert_eq!(image_extents(&body), [None, Some((0, 400))]);
    }
}

#[test]
fn a_column_image_can_begin_plain_blocks_and_contents_wrappers() {
    for items in [
        format!("<div>{}</div>", column_image(400, "")),
        format!(
            "<section style='display:contents'>{}</section>",
            column_image(400, "")
        ),
    ] {
        let body = column_after_lead(&items, "");
        assert_eq!(extents_of("o", &body), [None, Some((0, 400))]);
        assert_eq!(image_extents(&body), [None, Some((0, 400))]);
    }
    let body = format!(
        "<div style='height:300px'></div><section id=w><div id=o style='display:flex;flex-direction:column;width:300px'>{}</div></section>",
        column_image(400, "")
    );
    assert_eq!(extents_of("w", &body), [None, Some((0, 400))]);
    assert_eq!(extents_of("o", &body), [None, Some((0, 400))]);
}

#[test]
fn earlier_content_and_leading_gaps_leave_the_column_in_place() {
    for items in [
        format!(
            "<div style='height:20px;flex-shrink:0'></div>{}",
            column_image(400, "")
        ),
        format!(
            "<div><div style='height:0'></div>{}</div>",
            column_image(400, "")
        ),
        format!("<div>A{}</div>", column_image(400, "")),
        column_image(400, "margin-top:10px"),
        column_image(400, "margin-top:-10px"),
    ] {
        let body = column_after_lead(&items, "");
        assert_eq!(extents_of("o", &body)[0], Some((308, 600)), "{items}");
    }
    let body = column_after_lead(
        &column_image(400, ""),
        "height:420px;justify-content:center",
    );
    assert_eq!(extents_of("o", &body)[0], Some((308, 600)));
}

#[test]
fn an_oversized_column_image_moves_once_and_keeps_the_declared_container_height() {
    let image = column_image(750, "");
    let body = column_after_lead(&image, "");
    let split = [None, Some((0, 600)), Some((0, 150))];
    assert_eq!(extents_of("o", &body), split);
    assert_eq!(image_extents(&body), split);
    let body = format!(
        "{}<div id=a style='height:10px'></div>",
        column_after_lead(&image, "height:400px")
    );
    assert_eq!(extents_of("o", &body), [None, Some((0, 400)), None]);
    assert_eq!(image_extents(&body), split);
    assert_eq!(extents_of("a", &body), [None, Some((400, 410)), None]);
}

#[test]
fn a_column_image_that_fits_or_already_starts_the_page_stays() {
    let body = format!(
        "<div style='height:192px'></div><div id=o style='display:flex;flex-direction:column'>{}</div>",
        column_image(400, "")
    );
    assert_eq!(extents_of("o", &body), [Some((200, 600))]);
    assert_eq!(image_extents(&body), [Some((200, 600))]);
    let body = format!(
        "<style>body{{margin:0}}</style><div id=o style='display:flex;flex-direction:column'>{}</div>",
        column_image(750, "")
    );
    assert_eq!(extents_of("o", &body), [Some((0, 600)), Some((0, 150))]);
    assert_eq!(image_extents(&body), [Some((0, 600)), Some((0, 150))]);
}

#[test]
fn relative_visual_offsets_do_not_change_a_column_images_break_position() {
    let body = column_after_lead(&column_image(400, "position:relative;top:30px"), "");
    assert_eq!(extents_of("o", &body), [None, Some((0, 400))]);
    assert_eq!(image_extents(&body), [None, Some((30, 430))]);
}

#[test]
fn column_image_candidates_respect_parallel_and_clipping_ancestors() {
    let column = format!(
        "<div id=o style='display:flex;flex-direction:column;width:300px'>{}</div>",
        column_image(400, "")
    );
    for wrapper in [
        format!(
            "<div style='display:flex;break-before:avoid'><div style='width:20px;height:20px'></div>{column}</div>"
        ),
        format!("<div style='height:400px;overflow:hidden'>{column}</div>"),
    ] {
        let body = format!("<div style='height:300px'></div>{wrapper}");
        assert_eq!(extents_of("o", &body)[0], Some((308, 600)), "{wrapper}");
    }
}

#[test]
fn an_initial_column_keeps_its_start_while_the_image_moves_past_a_margin() {
    let column = format!(
        "<div id=o style='display:flex;flex-direction:column;width:300px'>{}</div>",
        column_image(750, "")
    );
    for body in [column.clone(), format!("<div>{column}</div>")] {
        assert_eq!(
            extents_of("o", &body),
            [Some((8, 600)), Some((0, 600)), Some((0, 150))]
        );
        assert_eq!(image_extents(&body), [None, Some((0, 600)), Some((0, 150))]);
    }
    let body = format!(
        "<style>body{{margin:0}}</style><div style='margin-top:20px'><div id=o style='display:flex;flex-direction:column'>{}</div></div>",
        column_image(590, "")
    );
    assert_eq!(extents_of("o", &body), [Some((20, 600)), Some((0, 590))]);
    assert_eq!(image_extents(&body), [None, Some((0, 590))]);
}

#[test]
fn plain_leading_edges_move_with_the_column_after_earlier_content() {
    let column = format!(
        "<div id=o style='display:flex;flex-direction:column;width:300px'>{}</div>",
        column_image(400, "")
    );
    let body =
        format!("<div style='height:300px'></div><div id=w style='padding-top:6px'>{column}</div>");
    assert_eq!(extents_of("w", &body), [None, Some((0, 406))]);
    assert_eq!(extents_of("o", &body), [None, Some((6, 406))]);
    assert_eq!(image_extents(&body), [None, Some((6, 406))]);
    let body =
        format!("<div style='height:300px'></div><div id=w style='margin-top:20px'>{column}</div>");
    assert_eq!(extents_of("w", &body), [None, Some((0, 400))]);
    assert_eq!(extents_of("o", &body), [None, Some((0, 400))]);
}

#[test]
fn column_holder_fit_uses_the_image_piece_instead_of_the_whole_column() {
    let items = format!(
        "{}<div style='height:500px;flex-shrink:0'></div>",
        column_image(400, "")
    );
    let body = format!(
        "<div style='height:300px'></div><div id=w style='padding-top:6px'><div id=o style='display:flex;flex-direction:column'>{items}</div></div>"
    );
    assert_eq!(
        extents_of("w", &body),
        [None, Some((0, 600)), Some((0, 306))]
    );
    assert_eq!(
        extents_of("o", &body),
        [None, Some((6, 600)), Some((0, 306))]
    );
    assert_eq!(image_extents(&body), [None, Some((6, 406)), None]);
}

#[test]
fn a_column_stays_when_only_its_later_content_crosses_the_page() {
    let items = format!(
        "{}<div style='height:500px;flex-shrink:0'></div>",
        column_image(100, "")
    );
    let body = column_after_lead(&items, "");
    assert_eq!(extents_of("o", &body), [Some((308, 600)), Some((0, 308))]);
    assert_eq!(image_extents(&body), [Some((308, 408)), None]);
}

#[test]
fn contents_chains_pass_only_their_first_free_boundary_to_a_column() {
    let column = format!(
        "<div id=o style='display:flex;flex-direction:column;width:300px'>{}</div>",
        column_image(400, "")
    );
    for wrapped in [
        format!("<section style='display:contents;break-before:avoid'>{column}</section>"),
        format!(
            "<section style='display:contents'> \n<section style='display:contents'>{column}</section></section>"
        ),
    ] {
        let body = format!("<div style='height:300px'></div><div id=w>{wrapped}</div>");
        assert_eq!(extents_of("w", &body), [None, Some((0, 400))]);
        assert_eq!(extents_of("o", &body), [None, Some((0, 400))]);
    }
    for before in ["A", "<div style='height:0'></div>"] {
        let body = format!(
            "<div style='height:300px'></div><div><section style='display:contents'>{before}{column}</section></div>"
        );
        assert!(extents_of("o", &body)[0].is_some(), "{before}");
    }
}

#[test]
fn column_boundary_inheritance_keeps_existing_avoided_breaks() {
    // These native controls preserve the existing conservative avoided-break
    // path; they are not an additional Chromium conformance claim.
    for (lead, wrapper, image) in [
        ("", "break-before:avoid", ""),
        ("break-after:avoid", "", ""),
        ("", "", "break-before:avoid"),
    ] {
        let body = format!(
            "<div style='height:300px;{lead}'></div><div style='{wrapper}'><div id=o style='display:flex;flex-direction:column;{image}'>{}</div></div>",
            column_image(400, "")
        );
        assert!(
            extents_of("o", &body)[0].is_some(),
            "{lead} / {wrapper} / {image}"
        );
    }
}

#[test]
fn earlier_float_or_generated_content_stops_a_leading_column_chain() {
    let items = format!(
        "<div><div style='float:left;width:10px;height:10px'></div>{}</div>",
        column_image(400, "")
    );
    assert!(extents_of("o", &column_after_lead(&items, ""))[0].is_some());
    let body = format!(
        "<style>#w::before{{content:'A';display:block;height:10px}}</style><div style='height:300px'></div><div id=w><div id=o style='display:flex;flex-direction:column'>{}</div></div>",
        column_image(400, "")
    );
    assert!(extents_of("o", &body)[0].is_some());
}

#[test]
fn a_columns_own_clip_keeps_an_image_whose_visible_part_fits() {
    for overflow in [
        "overflow:hidden",
        "overflow:clip",
        "overflow-x:hidden;overflow-y:visible",
    ] {
        let body = format!(
            "{}<div id=a style='height:10px'></div>",
            column_after_lead(&column_image(400, ""), &format!("height:100px;{overflow}"))
        );
        assert_eq!(extents_of("o", &body), [Some((308, 408))], "{overflow}");
        assert_eq!(extents_of("a", &body), [Some((408, 418))], "{overflow}");
    }
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

#[test]
fn a_wrapper_with_a_top_edge_moves_to_the_next_page_with_the_image_below_it() {
    // The page breaks before the wrapper, not between its padding or border
    // and the image: the wrapper starts the next page and the image lands
    // below those edges.
    let spacer = "<div style='height:300px'></div>";
    let trailer = "<div id=t style='width:100px;height:50px'></div>";
    let image = block_image(400);
    let cases = [
        (
            format!("<div id=o style='width:340px;padding:16px;border:1px solid'>{image}</div>"),
            ((0, 434), (17, 417), (434, 484)),
        ),
        (
            format!("<div id=o style='width:340px;padding-top:10px'>{image}</div>"),
            ((0, 410), (10, 410), (410, 460)),
        ),
        (
            format!("<div id=o style='width:340px;border-top:5px solid'>{image}</div>"),
            ((0, 405), (5, 405), (405, 455)),
        ),
        // A clipping wrapper keeps its height on the next page.
        (
            format!(
                "<div id=o style='width:340px;padding:10px;overflow:hidden;height:440px'>\
                 {image}</div>"
            ),
            ((0, 440), (10, 410), (440, 490)),
        ),
        // The margin above the wrapper is cut by the break, the one below it kept.
        (
            format!("<div id=o style='width:340px;margin:20px 0;padding:16px'>{image}</div>"),
            ((0, 432), (16, 416), (452, 502)),
        ),
        // Wrappers nested around the image add their edges.
        (
            format!(
                "<div id=o style='width:340px;padding:10px'>\
                 <div style='padding:5px'>{image}</div></div>"
            ),
            ((0, 430), (15, 415), (430, 480)),
        ),
    ];
    for (wrapper, (wrapper_extent, image_extent, trailer_extent)) in cases {
        let body = format!("{spacer}{wrapper}{trailer}");
        assert_eq!(
            extents_of("o", &body),
            [None, Some(wrapper_extent)],
            "{body}"
        );
        assert_eq!(image_extents(&body), [None, Some(image_extent)], "{body}");
        assert_eq!(
            extents_of("t", &body),
            [None, Some(trailer_extent)],
            "{body}"
        );
    }
}

#[test]
fn a_wrapper_that_keeps_the_margin_of_the_image_inside_moves_with_it_and_the_margin_stays() {
    // `overflow: hidden` keeps the margin of the image from collapsing through
    // the wrapper, so the margin is part of the wrapper and is not cut.
    let spacer = "<div style='height:300px'></div>";
    let trailer = "<div id=t style='width:100px;height:50px'></div>";
    let image = format!("{IMAGE} style='display:block;width:300px;height:400px;margin-top:20px'>");
    for (wrapper_style, wrapper_extent, image_extent, trailer_extent) in [
        ("overflow:hidden", (0, 420), (20, 420), (420, 470)),
        (
            "overflow:hidden;padding-top:10px",
            (0, 430),
            (30, 430),
            (430, 480),
        ),
        ("display:flow-root", (0, 420), (20, 420), (420, 470)),
    ] {
        let body =
            format!("{spacer}<div id=o style='width:340px;{wrapper_style}'>{image}</div>{trailer}");
        assert_eq!(
            extents_of("o", &body),
            [None, Some(wrapper_extent)],
            "{body}"
        );
        assert_eq!(image_extents(&body), [None, Some(image_extent)], "{body}");
        assert_eq!(
            extents_of("t", &body),
            [None, Some(trailer_extent)],
            "{body}"
        );
    }
    // A margin that collapses through the wrapper is cut by the break.
    let body = format!("{spacer}<div id=o style='width:340px'>{image}</div>{trailer}");
    assert_eq!(image_extents(&body), [None, Some((0, 400))], "{body}");
}

#[test]
fn a_wrapper_with_a_top_edge_stays_when_the_image_does_not_begin_it_or_it_starts_the_page() {
    let trailer = "<div id=t style='width:100px;height:50px'></div>";
    // Something before the image stays on this page with the wrapper.
    let body = format!(
        "<div style='height:300px'></div>\
         <div id=o style='width:340px;padding:10px'>\
         <div style='height:10px'></div>{}</div>{trailer}",
        block_image(400)
    );
    assert_eq!(
        extents_of("o", &body),
        [Some((308, 600)), Some((0, 410))],
        "{body}"
    );
    assert_eq!(image_extents(&body), [None, Some((0, 400))], "{body}");
    // A wrapper that starts the second page gains nothing by moving.
    let body = format!(
        "<div style='height:592px'></div>\
         <div id=o style='width:340px;padding:16px'>{}</div>{trailer}",
        block_image(300)
    );
    assert_eq!(extents_of("o", &body), [None, Some((0, 332))], "{body}");
    assert_eq!(image_extents(&body), [None, Some((16, 316))], "{body}");
    // The image fits the next page below the padding, down to the last pixel,
    // and the box after the wrapper goes on from there.
    let body = format!(
        "<div style='height:300px'></div>\
         <div id=o style='width:340px;padding:16px'>{}</div>{trailer}",
        block_image(560)
    );
    assert_eq!(
        extents_of("o", &body),
        [None, Some((0, 592)), None],
        "{body}"
    );
    assert_eq!(
        image_extents(&body),
        [None, Some((16, 576)), None],
        "{body}"
    );
    assert_eq!(
        extents_of("t", &body),
        [None, Some((592, 600)), Some((0, 42))],
        "{body}"
    );
}

#[test]
fn a_margin_above_the_wrapper_is_cut_by_the_break_like_one_above_the_image() {
    // The spacer ends where the page does, so the margin that collapses
    // through the wrapper to the image is the one a break cuts, whichever of
    // the two carries it.
    let spacer = "<div style='height:592px'></div>";
    let image =
        |style: &str| format!("{IMAGE} style='display:block;width:300px;height:590px;{style}'>");
    let on_wrapper = format!(
        "{spacer}<div style='width:340px;margin-top:30px'>{}</div>",
        image("")
    );
    let on_image = format!(
        "{spacer}<div style='width:340px'>{}</div>",
        image("margin-top:30px")
    );
    let on_both = format!(
        "{spacer}<div style='width:340px;margin-top:30px'>{}</div>",
        image("margin-top:20px")
    );
    let expected = image_extents(&on_image);
    assert_eq!(expected, [None, Some((0, 590))], "{on_image}");
    assert_eq!(image_extents(&on_wrapper), expected, "{on_wrapper}");
    assert_eq!(image_extents(&on_both), expected, "{on_both}");
}

#[test]
fn a_wrapper_moves_with_an_image_taller_than_the_page() {
    // The image is cut into pages from the top of the second, and the wrapper
    // that it begins starts there with it.
    let body = format!("<div id=o style='width:300px'>{}</div>", block_image(750));
    assert_eq!(
        extents_of("o", &body),
        [None, Some((0, 600)), Some((0, 150))],
        "{body}"
    );
}

#[test]
fn an_image_that_does_not_fit_below_the_padding_takes_only_the_wrappers_that_start_with_it() {
    // The image fills the page it moves to, so the padded wrapper cannot
    // land it below its padding there: it moves with the wrapper that starts
    // where it does, and the padded one stays.
    let body = format!(
        "<div id=o style='width:340px;padding:16px'>\
         <div id=w style='width:300px'>{}</div></div>",
        block_image(590)
    );
    assert_eq!(
        extents_of("w", &body),
        [None, Some((0, 590)), None],
        "{body}"
    );
    assert_eq!(image_extents(&body), [None, Some((0, 590)), None], "{body}");
    // The padded wrapper stays where it starts, and grows to hold what moved.
    assert_eq!(extents_of("o", &body)[0], Some((8, 600)), "{body}");
}
