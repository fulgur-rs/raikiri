//! Baseline alignment in flex and grid containers in vertical writing modes.
//!
//! The items of a row (or of a grid row) share a baseline across the line:
//! the dominant baseline of the container, which is the central baseline in a
//! vertical writing mode unless the text is set sideways, and the alphabetic
//! one otherwise (CSS Box Alignment 3 §9, CSS Writing Modes 4 §4.2). An item
//! without a baseline of that kind takes one from its border box.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

#[test]
fn vertical_rl_row_flex_aligns_the_central_baselines() {
    // A row flex container in `vertical-rl` aligns its items by the central baseline
    // of their first lines, which lies half way across the line (CSS Writing
    // Modes 4 §4.2): the middles of the 20px, 10px and 40px lines coincide.
    // The item with the most space before its baseline sits on the cross start
    // (the right edge), the others are pushed towards the cross end.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:40px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (175.0, 11.0, 20.0, 40.0)),
            ("b", (180.0, 51.0, 10.0, 30.0)),
            ("d", (165.0, 81.0, 40.0, 20.0)),
        ],
    );
}

#[test]
fn vertical_lr_row_flex_aligns_the_central_baselines() {
    // The same in `vertical-lr`, where the cross start is the left edge.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-lr;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:40px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (21.0, 11.0, 20.0, 40.0)),
            ("b", (26.0, 51.0, 10.0, 30.0)),
            ("d", (11.0, 81.0, 40.0, 20.0)),
        ],
    );
}

#[test]
fn sideways_text_aligns_the_alphabetic_baselines() {
    // Text set sideways has the alphabetic baseline as the dominant one: it lies
    // at the line-under edge of the font box, and the lines are not centred on
    // each other.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;text-orientation:sideways;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:40px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (178.0, 11.0, 20.0, 40.0)),
            ("b", (180.0, 51.0, 10.0, 30.0)),
            ("d", (165.0, 81.0, 40.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-lr;text-orientation:sideways;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:40px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (24.0, 11.0, 20.0, 40.0)),
            ("b", (26.0, 51.0, 10.0, 30.0)),
            ("d", (11.0, 81.0, 40.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:sideways-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:30px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (183.0, 11.0, 20.0, 40.0)),
            ("b", (185.0, 51.0, 10.0, 30.0)),
            ("d", (175.0, 81.0, 30.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:sideways-lr;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:30px;writing-mode:horizontal-tb;width:20px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (15.0, 65.0, 20.0, 40.0)),
            ("b", (23.0, 35.0, 10.0, 30.0)),
            ("d", (11.0, 5.0, 20.0, 30.0)),
        ],
    );
}

#[test]
fn an_orthogonal_item_sits_on_the_line_under_edge_of_sideways_text() {
    // With the alphabetic baseline the synthesized one lies on the line-under edge
    // of the container's writing mode: the left edge in `vertical-rl` and
    // `vertical-lr` text set sideways, where the block axis starts at the left in
    // the second, and the right edge in `sideways-rl`.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;text-orientation:sideways;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=h style="writing-mode:horizontal-tb;width:30px;height:40px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (171.0, 11.0, 20.0, 40.0)),
            ("h", (175.0, 51.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-lr;text-orientation:sideways;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=h style="writing-mode:horizontal-tb;width:30px;height:40px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (11.0, 11.0, 20.0, 40.0)),
            ("h", (15.0, 51.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:sideways-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=h style="writing-mode:horizontal-tb;width:30px;height:40px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (171.0, 11.0, 20.0, 40.0)),
            ("h", (175.0, 51.0, 30.0, 40.0)),
        ],
    );
}

#[test]
fn an_item_is_aligned_on_the_dominant_baseline_of_its_own_text() {
    // An item whose text sits on the other kind of baseline than the container's
    // contributes the baseline of its own first line.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="text-orientation:sideways;font-size:10px;line-height:30px">XX</div><div id=d style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (177.0, 11.0, 20.0, 40.0)),
            ("b", (175.0, 51.0, 30.0, 20.0)),
            ("d", (182.0, 71.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;text-orientation:sideways;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="text-orientation:upright;font-size:10px;line-height:30px">XX</div><div id=d style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (185.0, 11.0, 20.0, 40.0)),
            ("b", (174.0, 51.0, 30.0, 20.0)),
            ("d", (187.0, 71.0, 10.0, 30.0)),
        ],
    );
}

#[test]
fn upright_text_keeps_the_central_baseline() {
    // `text-orientation: upright` keeps the central baseline.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;text-orientation:upright;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (185.0, 11.0, 20.0, 40.0)),
            ("b", (190.0, 51.0, 10.0, 30.0)),
        ],
    );
}

#[test]
fn an_orthogonal_item_takes_a_baseline_from_its_border_box() {
    // An item in a horizontal writing mode has no baseline across the line of a
    // vertical container, so one is synthesized from its border box: the middle
    // of the box for the central baseline.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=h style="writing-mode:horizontal-tb;width:30px;height:40px"></div><div id=d style="font-size:10px;line-height:40px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (175.0, 11.0, 20.0, 40.0)),
            ("h", (170.0, 51.0, 30.0, 34.0)),
            ("d", (165.0, 85.0, 40.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-lr;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=h style="writing-mode:horizontal-tb;width:30px;height:40px"></div><div id=d style="font-size:10px;line-height:40px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (21.0, 11.0, 20.0, 40.0)),
            ("h", (16.0, 51.0, 30.0, 34.0)),
            ("d", (11.0, 85.0, 40.0, 20.0)),
        ],
    );
}

#[test]
fn baseline_alignment_uses_the_margin_boxes_of_the_items() {
    // Margins, padding and borders on the block sides of an item move its baseline
    // inside the margin box, and the container sizes itself around the result.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px;margin:0 4px 0 6px;padding:2px 3px 2px 7px;border-left:2px solid">XX</div><div id=b style="font-size:10px;line-height:10px;margin:0 9px 0 1px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (169.0, 11.0, 32.0, 44.0)),
            ("b", (183.0, 55.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:inline-flex;writing-mode:vertical-rl;align-items:baseline;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:40px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 46.0, 100.0)),
            ("a", (21.0, 11.0, 20.0, 40.0)),
            ("b", (26.0, 51.0, 10.0, 30.0)),
            ("d", (11.0, 81.0, 40.0, 20.0)),
        ],
    );
}

#[test]
fn a_nested_block_gives_its_first_line_to_the_item() {
    // An item whose first line is in a block child has the baseline of that line,
    // measured through the padding and borders of the block and the item. A
    // flex container as an item has the baseline of its first item.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px;"><div id=a1 style="padding-right:4px">XX</div></div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (181.0, 11.0, 24.0, 40.0)),
            ("a1", (181.0, 11.0, 24.0, 40.0)),
            ("b", (186.0, 51.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px;"><div id=a1 style="margin-right:4px">XX</div></div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (181.0, 11.0, 24.0, 40.0)),
            ("a1", (181.0, 11.0, 20.0, 40.0)),
            ("b", (186.0, 51.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px;padding:0 5px"><div id=a1 style="padding-right:4px">XX</div></div><div id=f style="display:flex;font-size:10px;line-height:40px"><div id=f1>XX</div><div id=f2 style="font-size:30px;line-height:30px">X</div></div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (170.0, 11.0, 34.0, 40.0)),
            ("a1", (175.0, 11.0, 24.0, 40.0)),
            ("f", (165.0, 51.0, 40.0, 50.0)),
            ("f1", (165.0, 51.0, 40.0, 20.0)),
            ("f2", (165.0, 71.0, 40.0, 30.0)),
            ("b", (180.0, 101.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-lr;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px"><div id=a1 style="padding-left:4px">XX</div></div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (11.0, 11.0, 24.0, 40.0)),
            ("a1", (11.0, 11.0, 24.0, 40.0)),
            ("b", (20.0, 51.0, 10.0, 30.0)),
        ],
    );
}

#[test]
fn a_nested_block_skips_the_children_without_a_baseline() {
    // Children that are out of flow, not rendered, empty, in an orthogonal flow or
    // white space do not give the item its baseline: the first block with a
    // line does.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px"> <div id=e></div> <div style="position:absolute;width:5px;height:5px"></div> <div style="display:none">X</div> <div id=o style="writing-mode:horizontal-tb;height:10px;width:30px">XX</div> <div id=a1 style="padding-right:4px"><span>XX</span></div></div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (151.0, 11.0, 54.0, 40.0)),
            ("e", (205.0, 11.0, 0.0, 40.0)),
            ("o", (175.0, 11.0, 30.0, 10.0)),
            ("a1", (151.0, 11.0, 24.0, 40.0)),
            ("b", (156.0, 51.0, 10.0, 30.0)),
        ],
    );
}

#[test]
fn a_wrapper_gives_the_item_the_baseline_its_child_exported() {
    // A block around a box passes on the baseline that box exported, not one
    // found again in its content. A scroll container exports a baseline clamped
    // to its border box: the 40px line of `s` is centred 20px across, but `s` is
    // 10px wide, so the item takes 10px with or without the wrapper.
    for scroller in [
        r#"<div id=s style="overflow:hidden;width:10px;line-height:40px">XX</div>"#,
        r#"<div id=w><div id=s style="overflow:hidden;width:10px;line-height:40px">XX</div></div>"#,
    ] {
        check(
            &format!(
                r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px"><div id=a style="line-height:10px">X</div>{scroller}<div id=d style="line-height:30px">X</div></div>"#
            ),
            &[
                ("c", (8.0, 8.0, 200.0, 100.0)),
                ("a", (188.0, 8.0, 10.0, 10.0)),
                ("s", (193.0, 18.0, 10.0, 20.0)),
                ("d", (178.0, 38.0, 30.0, 10.0)),
            ],
        );
    }
    // A flex container exports the baseline of its first item in layout order,
    // so `order` decides it, and a wrapper does not change that.
    for flex in [
        r#"<div id=f style="display:flex"><div id=f1 style="line-height:10px">X</div><div id=f2 style="line-height:40px;order:-1">X</div></div>"#,
        r#"<div id=w><div id=f style="display:flex"><div id=f1 style="line-height:10px">X</div><div id=f2 style="line-height:40px;order:-1">X</div></div></div>"#,
    ] {
        check(
            &format!(
                r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px"><div id=a style="line-height:10px">X</div>{flex}<div id=d style="line-height:30px">X</div></div>"#
            ),
            &[
                ("c", (8.0, 8.0, 200.0, 100.0)),
                ("a", (183.0, 8.0, 10.0, 10.0)),
                ("f", (168.0, 18.0, 40.0, 20.0)),
                ("f1", (168.0, 28.0, 40.0, 10.0)),
                ("f2", (168.0, 18.0, 40.0, 10.0)),
                ("d", (173.0, 38.0, 30.0, 10.0)),
            ],
        );
    }
}

/// A row flex container with the item `a`, the item under test `item`, and the
/// item `d`, which sit on the container's baseline.
fn baseline_row(item: &str) -> String {
    format!(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px"><div id=a style="line-height:10px">X</div>{item}<div id=d style="line-height:30px">X</div></div>"#
    )
}

#[test]
fn the_item_takes_the_first_baseline_in_content_order() {
    // A block child ahead of the text, or text ahead of the block child, gives
    // the item its baseline, whichever it is: the 40px line of `p` is centred
    // 20px across, the 10px line of the text 45px or 5px.
    check(
        &baseline_row(r#"<div id=w><div id=p style="line-height:40px">X</div>X</div>"#),
        &[
            ("a", (183.0, 8.0, 10.0, 10.0)),
            ("w", (158.0, 18.0, 50.0, 10.0)),
            ("p", (168.0, 18.0, 40.0, 10.0)),
            ("d", (173.0, 28.0, 30.0, 10.0)),
        ],
    );
    check(
        &baseline_row(r#"<div id=w>X<div id=p style="line-height:40px">X</div></div>"#),
        &[
            ("a", (188.0, 8.0, 10.0, 10.0)),
            ("w", (148.0, 18.0, 50.0, 10.0)),
            ("p", (148.0, 18.0, 40.0, 10.0)),
            ("d", (178.0, 28.0, 30.0, 10.0)),
        ],
    );
    // A negative margin can put the later box ahead of the first line along the
    // block axis, but not ahead of it in the content: the text still decides.
    check(
        &baseline_row(
            r#"<div id=w>X<div id=p style="line-height:40px;margin-right:-40px">X</div></div>"#,
        ),
        &[
            ("a", (188.0, 8.0, 10.0, 10.0)),
            ("w", (188.0, 18.0, 10.0, 10.0)),
            ("p", (188.0, 18.0, 40.0, 10.0)),
            ("d", (178.0, 28.0, 30.0, 10.0)),
        ],
    );
    check(
        &baseline_row(
            r#"<div id=w><div id=p style="line-height:40px;margin-left:-5px">X</div>X</div>"#,
        ),
        &[
            ("a", (183.0, 8.0, 10.0, 10.0)),
            ("w", (163.0, 18.0, 45.0, 10.0)),
            ("p", (168.0, 18.0, 40.0, 10.0)),
            ("d", (173.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_contents_element_passes_the_baseline_of_its_children() {
    // The children of a `display: contents` element count as the item's own.
    check(
        &baseline_row(
            r#"<div id=w><div style="display:contents"><div id=p style="line-height:40px">X</div></div><div id=q style="line-height:10px">X</div></div>"#,
        ),
        &[
            ("a", (183.0, 8.0, 10.0, 10.0)),
            ("w", (158.0, 18.0, 50.0, 10.0)),
            ("p", (168.0, 18.0, 40.0, 10.0)),
            ("q", (158.0, 18.0, 10.0, 10.0)),
            ("d", (173.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_relative_offset_does_not_move_the_baseline() {
    // The block moves from its place in flow, not the baseline of the item
    // around it, with a length or a percentage of the item's width.
    check(
        &baseline_row(
            r#"<div id=w><div id=p style="line-height:40px;position:relative;left:10px">X</div></div>"#,
        ),
        &[
            ("a", (183.0, 8.0, 10.0, 10.0)),
            ("w", (168.0, 18.0, 40.0, 10.0)),
            ("p", (178.0, 18.0, 40.0, 10.0)),
            ("d", (173.0, 28.0, 30.0, 10.0)),
        ],
    );
    check(
        &baseline_row(
            r#"<div id=w style="width:50px"><div id=p style="line-height:40px;position:relative;left:10%">X</div></div>"#,
        ),
        &[
            ("a", (183.0, 8.0, 10.0, 10.0)),
            ("w", (158.0, 18.0, 50.0, 10.0)),
            ("p", (173.0, 18.0, 40.0, 10.0)),
            ("d", (173.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn baseline_alignment_works_on_each_flex_line() {
    // Each flex line shares a baseline of its own, and `direction` does not move it.
    check(
        r#"<div id=c style="display:flex;flex-wrap:wrap;writing-mode:vertical-rl;align-items:baseline;width:200px;height:50px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:30px">XX</div><div id=e style="font-size:20px;line-height:20px">XX</div><div id=g style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 50.0)),
            ("a", (185.0, 11.0, 20.0, 40.0)),
            ("b", (154.2, 11.0, 10.0, 30.0)),
            ("d", (103.41, 11.0, 30.0, 20.0)),
            ("e", (62.61, 11.0, 20.0, 40.0)),
            ("g", (31.81, 11.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;direction:rtl;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:30px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (180.0, 65.0, 20.0, 40.0)),
            ("b", (185.0, 35.0, 10.0, 30.0)),
            ("d", (175.0, 15.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn a_scroll_container_exports_a_baseline_within_its_border_box() {
    // A scroll container exports a baseline clamped to its border box (CSS Box
    // Alignment 3 §9.1): the line sits 20px from the block start, the box is only
    // 10px wide.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:baseline;width:200px;height:100px;border:3px solid"><div id=a style="overflow:hidden;width:10px;font-size:10px;line-height:40px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (195.0, 11.0, 10.0, 20.0)),
            ("b", (190.0, 31.0, 10.0, 30.0)),
        ],
    );
}

#[test]
fn align_self_baseline_overrides_the_container() {
    // `align-self: baseline` and `first baseline` join the baseline group while the
    // other items follow `align-items`.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:center;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px;align-self:baseline">XX</div><div id=b style="font-size:10px;line-height:10px;align-self:first baseline">XXX</div><div id=d style="font-size:10px;line-height:30px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (185.0, 11.0, 20.0, 40.0)),
            ("b", (190.0, 51.0, 10.0, 30.0)),
            ("d", (93.0, 81.0, 30.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;align-items:first baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px;">XX</div><div id=b style="font-size:10px;line-height:10px;">XXX</div><div id=d style="font-size:10px;line-height:30px;">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (180.0, 11.0, 20.0, 40.0)),
            ("b", (185.0, 51.0, 10.0, 30.0)),
            ("d", (175.0, 81.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn a_vertical_flex_container_exposes_its_baseline() {
    // The baseline of a vertical flex container is the one of its first item, which
    // an inline-level container takes onto the baseline of a horizontal line and
    // onto the central baseline of a vertical one.
    check(
        r#"<div style="font-size:10px;line-height:10px">A<div id=c style="display:inline-flex;vertical-align:baseline;writing-mode:vertical-rl;align-items:baseline;width:60px;height:60px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>B</div>"#,
        &[
            ("c", (18.0, 8.0, 60.0, 60.0)),
            ("a", (55.0, 11.0, 20.0, 40.0)),
            ("b", (60.0, 51.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=p style="writing-mode:vertical-rl;width:200px;height:100px;font-size:10px;line-height:10px">A<div id=c style="display:inline-flex;writing-mode:vertical-rl;align-items:baseline;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div></div>B</div>"#,
        &[
            ("p", (8.0, 8.0, 200.0, 100.0)),
            ("c", (182.0, 18.0, 26.0, 76.0)),
            ("a", (185.0, 21.0, 20.0, 40.0)),
            ("b", (190.0, 61.0, 10.0, 30.0)),
        ],
    );
}

#[test]
fn vertical_grid_rows_share_a_baseline() {
    // Items in the same row of a grid share a baseline along the block axis; an
    // orthogonal item takes one from its border box.
    check(
        r#"<div id=c style="display:grid;grid-auto-flow:column;writing-mode:vertical-rl;align-items:baseline;width:260px;height:120px;border:3px solid;padding:4px 2px"><div id=a style="font-size:20px;line-height:20px;border:2px solid;padding:3px 1px 5px 6px;margin:2px 4px 1px 7px">XX</div><div id=b style="font-size:10px;line-height:30px;border:1px solid;padding:2px;margin:3px">XXX</div><div id=d style="font-size:30px;line-height:10px;writing-mode:horizontal-tb;padding:1px">X</div></div>"#,
        &[
            ("c", (8.0, 8.0, 260.0, 120.0)),
            ("a", (224.0, 17.0, 31.0, 52.0)),
            ("b", (224.0, 73.0, 36.0, 36.0)),
            ("d", (226.0, 112.0, 32.0, 12.0)),
        ],
    );
    check(
        r#"<div id=c style="display:grid;grid-auto-flow:column;writing-mode:vertical-lr;align-items:baseline;width:260px;height:120px;border:3px solid;padding:4px 2px"><div id=a style="font-size:20px;line-height:20px;border:2px solid;padding:3px 1px 5px 6px;margin:2px 4px 1px 7px">XX</div><div id=b style="font-size:10px;line-height:30px;border:1px solid;padding:2px;margin:3px">XXX</div><div id=d style="font-size:30px;line-height:10px;writing-mode:horizontal-tb;padding:1px">X</div></div>"#,
        &[
            ("c", (8.0, 8.0, 260.0, 120.0)),
            ("a", (20.0, 17.0, 31.0, 52.0)),
            ("b", (20.0, 73.0, 36.0, 36.0)),
            ("d", (22.0, 112.0, 32.0, 12.0)),
        ],
    );
}

#[test]
fn first_baseline_is_baseline_in_a_horizontal_container() {
    // `first baseline` is the same alignment as `baseline`.
    check(
        r#"<div id=c style="display:flex;align-items:first baseline;width:200px;height:100px;border:3px solid"><div id=a style="font-size:20px;line-height:20px">XX</div><div id=b style="font-size:10px;line-height:10px">XXX</div><div id=d style="font-size:10px;line-height:30px;align-self:first baseline">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (11.0, 13.0, 40.0, 20.0)),
            ("b", (51.0, 21.0, 30.0, 10.0)),
            ("d", (81.0, 11.0, 20.0, 30.0)),
        ],
    );
}
