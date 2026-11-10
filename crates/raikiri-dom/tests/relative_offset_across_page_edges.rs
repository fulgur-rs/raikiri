//! A relative offset paints a box away from its place in the flow without
//! moving the flow (CSS 2.1 §9.4.3), and fragmentation happens before it
//! (CSS Fragmentation 3 §5.5): the page breaks fall where the flow puts the
//! box and its lines, and the offset then paints each piece that far from
//! there, onto the next page when it reaches past the page edge.
//!
//! The pages are 100px by 100px with no margin, the lines are 20px Ahem, and
//! the expected extents of `#o` on each page are the ones Chromium 153 prints
//! for the same markup.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The block-axis extent of `#o` on each page, `None` where it has no box.
fn extents(body: &str) -> Vec<Option<(f32, f32)>> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 100px 100px; margin: 0 }} \
         body {{ margin: 0; font: 20px/20px Ahem }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    result
        .pages()
        .map(|page| {
            let dom = page.dom();
            page.fragments()
                .filter(|fragment| dom.attr(fragment.node(), "id") == Some("o"))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (rect.y, rect.y + rect.height)
                })
                .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
        })
        .collect()
}

/// The extent of `#o` on each page, as [`extents`] gives it.
type Extents<'a> = &'a [Option<(f32, f32)>];

/// Checks each case: its label, its markup and the extents Chromium gives
/// `#o`.
fn check(cases: &[(&str, &str, Extents<'_>)]) {
    for &(label, body, expected) in cases {
        let got = extents(body);
        let near = |a: f32, b: f32| (a - b).abs() <= 0.04;
        assert!(
            got.len() == expected.len()
                && got.iter().zip(expected).all(|(g, e)| match (g, e) {
                    (Some(g), Some(e)) => near(g.0, e.0) && near(g.1, e.1),
                    (None, None) => true,
                    _ => false,
                }),
            "{label}: got {got:?}, expected {expected:?}"
        );
    }
}

/// A block of `height` at the top of the page.
fn space(height: u32) -> String {
    format!("<div style=\"height:{height}px\"></div>")
}

/// A paragraph `#o` of `lines` lines with `style` added.
fn paragraph(style: &str, lines: usize) -> String {
    format!(
        "<div id=o style=\"position:relative;{style}\">{}</div>",
        vec!["x"; lines].join("<br>")
    )
}

/// The lines of a paragraph that the flow keeps on a page stay there,
/// wherever the offset paints them: the part that reaches past the page edge
/// goes on the next page.
#[test]
fn lines_break_where_the_flow_puts_them() {
    let s20 = space(20);
    let s70 = space(70);
    let s90 = space(90);
    let tail = space(10);
    check(&[
        (
            "a line painted across the edge",
            &format!("{s20}{}{tail}", paragraph("top:70px", 1)),
            &[Some((90.0, 100.0)), Some((0.0, 10.0))],
        ),
        (
            "two lines painted across the edge",
            &format!("{s20}{}{tail}", paragraph("top:70px", 2)),
            &[Some((90.0, 100.0)), Some((0.0, 30.0))],
        ),
        (
            "in a relatively positioned block",
            &format!(
                "{s20}<div style=\"position:relative;top:70px\"><div id=o>x<br>x</div></div>{tail}"
            ),
            &[Some((90.0, 100.0)), Some((0.0, 30.0))],
        ),
        // The second line crosses the edge in the flow, so the paragraph
        // moves to keep two lines together, and the offset paints it back
        // across the edge.
        (
            "painted up from the next page",
            &format!("{s70}{}{tail}", paragraph("top:-30px", 2)),
            &[Some((70.0, 100.0)), Some((0.0, 10.0))],
        ),
        (
            "first line pushed, painted up",
            &format!("{s90}{}{tail}", paragraph("top:-30px", 2)),
            &[Some((70.0, 100.0)), Some((0.0, 10.0))],
        ),
    ]);
}

/// `orphans`, `widows` and `break-inside: avoid` judge the lines where the
/// flow puts them.
#[test]
fn orphans_and_widows_judge_the_lines_in_the_flow() {
    let tail = space(10);
    check(&[
        (
            "flow fits, painted above",
            &format!("{}{}{tail}", space(40), paragraph("top:-30px", 4)),
            &[Some((10.0, 100.0)), Some((0.0, 10.0))],
        ),
        (
            "flow breaks for widows, painted below",
            &format!("{}{}{tail}", space(20), paragraph("top:30px", 4)),
            &[Some((50.0, 100.0)), Some((0.0, 30.0))],
        ),
        (
            "an avoided break that the flow does not reach",
            &format!(
                "{}{}{tail}",
                space(30),
                paragraph("top:20px;break-inside:avoid", 3)
            ),
            &[Some((50.0, 100.0)), Some((0.0, 10.0))],
        ),
        (
            "orphans three",
            &format!(
                "{}{}",
                space(30),
                paragraph("top:15px;orphans:3;widows:1", 5)
            ),
            &[Some((45.0, 100.0)), Some((0.0, 55.0))],
        ),
    ]);
}

/// A box moved to the next page, by a break it avoids or a forced one, keeps
/// its offset there, and the flow after it goes on from its place in the
/// flow.
#[test]
fn a_moved_box_keeps_its_offset_and_the_flow_after_it_its_place() {
    let s50 = space(50);
    let avoided = "<div style=\"position:relative;top:-40px;height:60px;break-inside:avoid\"";
    check(&[
        (
            "avoided break, the box",
            &format!("{s50}{avoided} id=o></div>{}", space(10)),
            &[Some((60.0, 100.0)), Some((0.0, 20.0))],
        ),
        (
            "avoided break, the block after it",
            &format!("{s50}{avoided}></div><div id=o style=\"height:10px\"></div>"),
            &[None, Some((60.0, 70.0))],
        ),
        (
            "forced break, painted up",
            &format!(
                "{s50}<div id=o style=\"position:relative;top:-40px;height:30px;break-before:page\"></div>{}",
                space(10)
            ),
            &[Some((60.0, 90.0)), None],
        ),
        (
            "forced break, the block after a box painted up",
            &format!(
                "{s50}<div style=\"position:relative;top:-40px;height:30px;break-before:page\"></div>\
                 <div id=o style=\"height:10px\"></div>"
            ),
            &[None, Some((30.0, 40.0))],
        ),
        (
            "forced break, painted down",
            &format!(
                "{s50}<div id=o style=\"position:relative;top:20px;height:30px;break-before:page\"></div>{}",
                space(10)
            ),
            &[None, Some((20.0, 50.0))],
        ),
        (
            "forced break, the block after a box painted down",
            &format!(
                "{s50}<div style=\"position:relative;top:20px;height:30px;break-before:page\"></div>\
                 <div id=o style=\"height:10px\"></div>"
            ),
            &[None, Some((30.0, 40.0))],
        ),
        // The box starts the page after the break, and the offset paints it
        // two pages on; the flow after it stays on that page.
        (
            "forced break, painted far down",
            &format!(
                "{s50}<div id=o style=\"position:relative;top:160px;height:20px;break-before:page\"></div>{}",
                space(10)
            ),
            &[None, None, Some((60.0, 80.0))],
        ),
        (
            "forced break, the block after a box painted far down",
            &format!(
                "{s50}<div style=\"position:relative;top:160px;height:20px;break-before:page\"></div>\
                 <div id=o style=\"height:10px\"></div>"
            ),
            &[None, Some((20.0, 30.0)), None],
        ),
    ]);
}

/// An offset of an inline box inside a paragraph moves its text within the
/// lines: a block of a set height whose lines run past the page edge still
/// moves whole.
#[test]
fn an_offset_inline_box_leaves_its_block_to_move_whole() {
    check(&[(
        "offset span in a block of a set height",
        &format!(
            "{}<div id=o style=\"height:40px\"><span style=\"position:relative;top:30px\">x<br>x</span></div>{}",
            space(70),
            space(10)
        ),
        &[None, Some((0.0, 40.0))],
    )]);
}

/// A block painted across a page edge from where the flow keeps it goes on
/// the next page past the edge, and the flow after it stays.
#[test]
fn a_block_painted_across_the_edge_continues_on_the_next_page() {
    check(&[
        (
            "painted down across",
            &format!(
                "{}<div id=o style=\"position:relative;top:70px;height:30px\"></div>{}",
                space(20),
                space(10)
            ),
            &[Some((90.0, 100.0)), Some((0.0, 20.0))],
        ),
        (
            "painted up across",
            &format!(
                "{}<div id=o style=\"position:relative;top:-15px;height:20px\"></div>{}",
                space(100),
                space(10)
            ),
            &[Some((85.0, 100.0)), Some((0.0, 5.0))],
        ),
    ]);
}
