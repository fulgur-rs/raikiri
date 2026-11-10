//! The automatic cross size of a wrapping column flex container comes from
//! the cross-size contributions of its items, measured with the room the
//! container's main size gives them rather than with their flex base sizes
//! (CSS Flexbox 1 §9.9.2). An orthogonal item can then be larger than the
//! line the container makes room for, and overflows it.
//!
//! The lines the contributions are summed over break against the
//! container's definite main size or its maximum; with neither, the items
//! share one line. The expected boxes are those Chromium lays out in an
//! 800x600 viewport, the page area here, with Ahem loaded.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// One case: the container style, each item as `(style, content)`, and the
/// border boxes Chromium gives the container and then each item, as
/// `[x, y, width, height]`.
type Case = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static [[f32; 4]],
);

/// The border boxes of the container `c` and its items `i0`, `i1`, ... laid
/// out on one 800x600 page, in a body `body_width` pixels wide, and in a
/// parent styled `parent` unless that is empty.
fn boxes(
    resources: &RenderResources,
    body_width: f32,
    parent: &str,
    container: &str,
    items: &[(&str, &str)],
) -> Vec<[f32; 4]> {
    let items: String = items
        .iter()
        .enumerate()
        .map(|(i, (style, content))| format!("<div id=i{i} style=\"{style}\">{content}</div>"))
        .collect();
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;width:{body_width}px;font:10px/10px Ahem}}div{{box-sizing:border-box}}\
         </style></head><body>{open}<div id=c style=\"{container}\">{items}</div>{close}</body></html>",
        open = if parent.is_empty() {
            String::new()
        } else {
            format!("<div style=\"{parent}\">")
        },
        close = if parent.is_empty() { "" } else { "</div>" },
    );
    let doc = parse_html_with_resources(html.as_bytes(), resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("layout did not complete");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let count = items.matches("<div id=i").count();
    std::iter::once("c".to_string())
        .chain((0..count).map(|i| format!("i{i}")))
        .map(|id| {
            let fragment = page
                .fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(id.as_str()))
                .unwrap_or_else(|| panic!("no box for {id}"));
            let r = fragment.rect();
            [r.x, r.y, r.width, r.height]
        })
        .collect()
}

/// Asserts every case lays out as Chromium does, within 1/32px.
#[track_caller]
fn assert_cases(cases: &[Case]) {
    assert_cases_in(800.0, "", cases);
}

/// Asserts every case lays out as Chromium does, within 1/32px, in a body
/// `body_width` pixels wide and in a parent styled `parent`, if any.
#[track_caller]
fn assert_cases_in(body_width: f32, parent: &str, cases: &[Case]) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    for &(container, items, expected) in cases {
        let actual = boxes(&resources, body_width, parent, container, items);
        let close = actual.len() == expected.len()
            && actual
                .iter()
                .flatten()
                .zip(expected.iter().flatten())
                .all(|(a, e)| (a - e).abs() <= 1.0 / 32.0);
        assert!(close, "{container}: {actual:?} != {expected:?}");
    }
}

#[test]
fn wrapping_columns_take_the_cross_size_their_items_contribute() {
    assert_cases(&[
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[(
                "writing-mode:vertical-rl;flex:0 0 60px;min-height:10%",
                "xxxx xxxx xx",
            )],
            &[[0.0, 0.0, 10.0, 60.0], [0.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[
                (
                    "writing-mode:vertical-rl;flex:0 0 60px;min-height:calc(10% + 1px)",
                    "xxxx xxxx xx",
                ),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 10.0, 101.0],
                [0.0, 0.0, 30.0, 61.0],
                [0.0, 61.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[(
                "writing-mode:vertical-rl;flex:0 0 60px;margin-top:5%",
                "xxxx xxxx xx",
            )],
            &[[0.0, 0.0, 10.0, 60.5], [0.0, 0.5, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 10.0, 60.0], [0.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[("writing-mode:vertical-lr;flex:0 0 60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 10.0, 60.0], [0.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap-reverse",
            &[("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 10.0, 60.0], [-20.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column",
            &[("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 10.0, 60.0], [0.0, 0.0, 10.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[("writing-mode:vertical-rl", "xxxx xxxx xx")],
            &[[0.0, 0.0, 10.0, 120.0], [0.0, 0.0, 10.0, 120.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[("writing-mode:vertical-rl;height:60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 30.0, 60.0], [0.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("", "xxx"),
            ],
            &[
                [0.0, 0.0, 30.0, 70.0],
                [0.0, 0.0, 30.0, 60.0],
                [0.0, 60.0, 30.0, 10.0],
            ],
        ),
        (
            "display:block;width:max-content",
            &[(
                "display:inline-flex;flex-direction:column;flex-wrap:wrap",
                "<div style='writing-mode:vertical-rl;flex:0 0 60px'>xxxx xxxx xx</div>",
            )],
            &[[0.0, 0.0, 10.0, 62.0], [0.0, 0.0, 10.0, 60.0]],
        ),
        (
            "float:left;display:flex;flex-direction:column;flex-wrap:wrap",
            &[("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 10.0, 60.0], [0.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 10.0, 100.0],
                [0.0, 0.0, 30.0, 60.0],
                [0.0, 60.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:70px",
            &[("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 20.0, 70.0], [0.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:100px",
            &[("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx")],
            &[[0.0, 0.0, 20.0, 100.0], [0.0, 0.0, 30.0, 60.0]],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:50px",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 50.0, 50.0],
                [0.0, 0.0, 30.0, 60.0],
                [30.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:70px",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 40.0, 70.0],
                [0.0, 0.0, 30.0, 60.0],
                [30.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:80px",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 30.0, 80.0],
                [0.0, 0.0, 30.0, 60.0],
                [30.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:100px",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 20.0, 100.0],
                [0.0, 0.0, 30.0, 60.0],
                [0.0, 60.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:50px",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 50.0, 50.0],
                [0.0, 0.0, 30.0, 60.0],
                [30.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:80px",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 30.0, 60.0],
                [0.0, 0.0, 30.0, 60.0],
                [30.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:120px",
            &[
                ("writing-mode:vertical-rl;flex:0 0 60px", "xxxx xxxx xx"),
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx xx xx"),
            ],
            &[
                [0.0, 0.0, 10.0, 100.0],
                [0.0, 0.0, 30.0, 60.0],
                [0.0, 60.0, 30.0, 40.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:50px",
            &[
                ("flex:0 0 30px", "xx"),
                ("flex:0 0 30px", "xxxx"),
                ("flex:0 0 30px", "xxx"),
            ],
            &[
                [0.0, 0.0, 90.0, 50.0],
                [0.0, 0.0, 20.0, 30.0],
                [20.0, 0.0, 40.0, 30.0],
                [60.0, 0.0, 30.0, 30.0],
            ],
        ),
        (
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;height:70px",
            &[
                ("flex:0 0 30px", "xx"),
                ("flex:0 0 30px", "xxxx"),
                ("flex:0 0 30px", "xxx"),
            ],
            &[
                [0.0, 0.0, 70.0, 70.0],
                [0.0, 0.0, 40.0, 30.0],
                [0.0, 30.0, 40.0, 30.0],
                [40.0, 0.0, 30.0, 30.0],
            ],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap",
            &[("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx")],
            &[[0.0, 0.0, 80.0, 10.0], [0.0, 0.0, 80.0, 30.0]],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap",
            &[("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx")],
            &[[0.0, 0.0, 80.0, 10.0], [0.0, 0.0, 80.0, 30.0]],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 0.0, 130.0, 10.0],
                [50.0, 0.0, 80.0, 30.0],
                [0.0, 0.0, 50.0, 30.0],
            ],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 0.0, 90.0, 40.0],
                [10.0, 0.0, 80.0, 30.0],
                [40.0, 30.0, 50.0, 20.0],
            ],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;width:100px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 0.0, 100.0, 40.0],
                [0.0, 0.0, 80.0, 30.0],
                [0.0, 30.0, 50.0, 20.0],
            ],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:140px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 0.0, 130.0, 10.0],
                [50.0, 0.0, 80.0, 30.0],
                [0.0, 0.0, 50.0, 30.0],
            ],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;width:90px;margin-top:50px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 40.0],
                [10.0, 60.0, 80.0, 30.0],
                [40.0, 40.0, 50.0, 20.0],
            ],
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;margin-top:50px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 130.0, 10.0],
                [0.0, 30.0, 80.0, 30.0],
                [80.0, 30.0, 50.0, 30.0],
            ],
        ),
    ]);
}

/// A vertical column container's main size is its width. Its lines break
/// against its width when it states one, else against its `max-width` and
/// its `min-width`, and its width is then its longest line (CSS Flexbox 1
/// §9.3). The items of each line flex in that width. The containers start
/// 50px down the page, so that `wrap-reverse` lines above them stay on it.
#[test]
fn vertical_wrapping_columns_break_their_lines_along_their_width() {
    assert_cases(&[
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:50px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 50.0, 50.0],
                [-30.0, 50.0, 80.0, 30.0],
                [0.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [30.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:100px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [30.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px;padding:5px;border:2px solid",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 64.0],
                [3.0, 57.0, 80.0, 30.0],
                [33.0, 87.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 40.0],
                [10.0, 50.0, 80.0, 30.0],
                [40.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 130.0, 10.0],
                [50.0, 50.0, 80.0, 30.0],
                [0.0, 50.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;min-width:150px;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 150.0, 10.0],
                [70.0, 50.0, 80.0, 30.0],
                [20.0, 50.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:50px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 50.0, 50.0],
                [-30.0, 70.0, 80.0, 30.0],
                [0.0, 50.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 60.0, 80.0, 30.0],
                [30.0, 40.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:100px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 60.0, 80.0, 30.0],
                [30.0, 40.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px;padding:5px;border:2px solid",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 64.0],
                [3.0, 77.0, 80.0, 30.0],
                [33.0, 57.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 40.0],
                [10.0, 60.0, 80.0, 30.0],
                [40.0, 40.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 130.0, 10.0],
                [50.0, 30.0, 80.0, 30.0],
                [0.0, 30.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;min-width:150px;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 150.0, 10.0],
                [70.0, 30.0, 80.0, 30.0],
                [20.0, 30.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;max-width:50px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 50.0, 50.0],
                [0.0, 50.0, 80.0, 30.0],
                [0.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [0.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;max-width:100px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [0.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;max-width:90px;padding:5px;border:2px solid",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 64.0],
                [7.0, 57.0, 80.0, 30.0],
                [7.0, 87.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [0.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 130.0, 10.0],
                [0.0, 50.0, 80.0, 30.0],
                [80.0, 50.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;min-width:150px;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 150.0, 10.0],
                [0.0, 50.0, 80.0, 30.0],
                [80.0, 50.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;max-width:50px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 50.0, 50.0],
                [0.0, 70.0, 80.0, 30.0],
                [0.0, 50.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 60.0, 80.0, 30.0],
                [0.0, 40.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;max-width:100px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 60.0, 80.0, 30.0],
                [0.0, 40.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;max-width:90px;padding:5px;border:2px solid",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 64.0],
                [7.0, 77.0, 80.0, 30.0],
                [7.0, 57.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 40.0],
                [0.0, 60.0, 80.0, 30.0],
                [0.0, 40.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 130.0, 10.0],
                [0.0, 30.0, 80.0, 30.0],
                [80.0, 30.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;min-width:150px;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 150.0, 10.0],
                [0.0, 30.0, 80.0, 30.0],
                [80.0, 30.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                ("flex:0 0 80px", "xxxx xxxx xxxx"),
                ("flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 250.0],
                [0.0, 50.0, 80.0, 140.0],
                [30.0, 190.0, 50.0, 110.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                (
                    "writing-mode:horizontal-tb;flex:0 0 auto;width:50%",
                    "xxxx xxxx xxxx",
                ),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 50.0, 50.0],
                [5.0, 50.0, 45.0, 30.0],
                [0.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 60%", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 54.0, 40.0],
                [0.0, 50.0, 54.0, 30.0],
                [4.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                (
                    "writing-mode:horizontal-tb;flex:0 1 120px",
                    "xxxx xxxx xxxx",
                ),
                ("writing-mode:horizontal-tb;flex:0 1 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 40.0],
                [0.0, 50.0, 90.0, 20.0],
                [40.0, 70.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 40px", "xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 40px", "xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 40px", "xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 20.0],
                [40.0, 50.0, 40.0, 20.0],
                [0.0, 50.0, 40.0, 20.0],
                [40.0, 70.0, 40.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 90.0, 40.0],
                [0.0, 50.0, 90.0, 20.0],
                [0.0, 70.0, 90.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;align-content:center;justify-content:space-between;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 40px", "xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 40px", "xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 40px", "xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 20.0],
                [40.0, 40.0, 40.0, 20.0],
                [0.0, 40.0, 40.0, 20.0],
                [40.0, 60.0, 40.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
            ],
            &[
                [0.0, 50.0, 120.0, 10.0],
                [80.0, 50.0, 40.0, 30.0],
                [30.0, 50.0, 50.0, 30.0],
                [0.0, 50.0, 30.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
            &[
                (
                    "writing-mode:horizontal-tb;flex:0 1 120px",
                    "xxxx xxxx xxxx",
                ),
                ("writing-mode:horizontal-tb;flex:0 1 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 170.0, 10.0],
                [50.0, 50.0, 120.0, 20.0],
                [0.0, 50.0, 50.0, 20.0],
            ],
        ),
    ]);
}

/// In a body narrower than its lines, a vertical wrapping column container
/// still takes the width of its longest line, as its width is its block
/// size, which the room around it does not bound.
#[test]
fn vertical_wrapping_columns_keep_their_lines_in_a_narrow_body() {
    assert_cases_in(
        60.0,
        "",
        &[
            (
                "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                    ("writing-mode:horizontal-tb;flex:0 0 30px;width:20%", "xx"),
                ],
                &[
                    [0.0, 50.0, 160.0, 10.0],
                    [80.0, 50.0, 80.0, 30.0],
                    [30.0, 50.0, 50.0, 30.0],
                    [0.0, 50.0, 30.0, 30.0],
                ],
            ),
            (
                "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                    ("writing-mode:horizontal-tb;flex:0 0 30px;width:20%", "xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 60.0, 80.0, 30.0],
                    [30.0, 40.0, 50.0, 20.0],
                    [0.0, 40.0, 30.0, 20.0],
                ],
            ),
            (
                "margin-top:50px;display:flex;float:left;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;min-width:40px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                    ("writing-mode:horizontal-tb;flex:0 0 30px;width:20%", "xx"),
                ],
                &[
                    [0.0, 50.0, 160.0, 10.0],
                    [80.0, 50.0, 80.0, 30.0],
                    [30.0, 50.0, 50.0, 30.0],
                    [0.0, 50.0, 30.0, 30.0],
                ],
            ),
            (
                "margin-top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                    ("writing-mode:horizontal-tb;flex:0 0 30px;width:20%", "xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 50.0, 80.0, 30.0],
                    [30.0, 80.0, 50.0, 20.0],
                    [0.0, 80.0, 30.0, 20.0],
                ],
            ),
        ],
    );
}

/// A vertical wrapping column container finds its width from its lines
/// whatever sizes it: an intrinsic `width` keyword, a flex or grid container
/// it is an item of, or the shrink-to-fit width of an absolutely positioned
/// box. Its items then contribute to its height with the room its
/// `max-width` gives them, or their max-content size without one.
#[test]
fn vertical_wrapping_columns_find_their_width_from_their_lines_wherever_they_sit() {
    assert_cases(&[
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [30.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [30.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [30.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content;max-width:90px",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 80.0, 40.0],
                [0.0, 50.0, 80.0, 30.0],
                [30.0, 80.0, 50.0, 20.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 130.0, 10.0],
                [50.0, 50.0, 80.0, 30.0],
                [0.0, 50.0, 50.0, 30.0],
            ],
        ),
        (
            "margin-top:50px;display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;",
            &[
                ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
            ],
            &[
                [0.0, 50.0, 130.0, 10.0],
                [50.0, 50.0, 80.0, 30.0],
                [0.0, 50.0, 50.0, 30.0],
            ],
        ),
    ]);
    assert_cases_in(
        800.0,
        "margin-top:50px;display:flex;flex-direction:column;align-items:start",
        &[
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 50.0, 80.0, 30.0],
                    [30.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 10.0],
                    [50.0, 50.0, 80.0, 30.0],
                    [0.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 60.0, 80.0, 30.0],
                    [30.0, 40.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 10.0],
                    [50.0, 30.0, 80.0, 30.0],
                    [0.0, 30.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    assert_cases_in(
        800.0,
        "margin-top:50px;display:flex;flex-direction:column",
        &[
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 50.0, 80.0, 30.0],
                    [40.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 800.0, 10.0],
                    [720.0, 50.0, 80.0, 30.0],
                    [670.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 60.0, 80.0, 30.0],
                    [40.0, 40.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 800.0, 10.0],
                    [720.0, 30.0, 80.0, 30.0],
                    [670.0, 30.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    assert_cases_in(
        800.0,
        "margin-top:50px;display:flex",
        &[
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 50.0],
                    [0.0, 50.0, 80.0, 30.0],
                    [30.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 20.0],
                    [50.0, 50.0, 80.0, 30.0],
                    [0.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 50.0],
                    [0.0, 70.0, 80.0, 30.0],
                    [30.0, 50.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 20.0],
                    [50.0, 40.0, 80.0, 30.0],
                    [0.0, 40.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    assert_cases_in(
        800.0,
        "margin-top:50px;display:flex;align-items:start",
        &[
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 50.0],
                    [0.0, 50.0, 80.0, 30.0],
                    [30.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 20.0],
                    [50.0, 50.0, 80.0, 30.0],
                    [0.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 50.0],
                    [0.0, 70.0, 80.0, 30.0],
                    [30.0, 50.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 20.0],
                    [50.0, 40.0, 80.0, 30.0],
                    [0.0, 40.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    assert_cases_in(
        800.0,
        "margin-top:50px;display:grid;justify-items:start",
        &[
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 50.0, 80.0, 30.0],
                    [30.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 10.0],
                    [50.0, 50.0, 80.0, 30.0],
                    [0.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 60.0, 80.0, 30.0],
                    [30.0, 40.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 10.0],
                    [50.0, 30.0, 80.0, 30.0],
                    [0.0, 30.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    assert_cases_in(
        800.0,
        "margin-top:50px;display:grid",
        &[
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 50.0, 80.0, 30.0],
                    [40.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 800.0, 10.0],
                    [720.0, 50.0, 80.0, 30.0],
                    [670.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 60.0, 80.0, 30.0],
                    [40.0, 40.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 800.0, 10.0],
                    [720.0, 30.0, 80.0, 30.0],
                    [670.0, 30.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    assert_cases_in(
        800.0,
        "margin-top:50px;position:relative;height:300px",
        &[
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 50.0, 80.0, 30.0],
                    [30.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "position:absolute;top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 100.0, 80.0, 40.0],
                    [0.0, 100.0, 80.0, 30.0],
                    [30.0, 130.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 10.0],
                    [50.0, 50.0, 80.0, 30.0],
                    [0.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "position:absolute;top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 100.0, 130.0, 10.0],
                    [50.0, 100.0, 80.0, 30.0],
                    [0.0, 100.0, 50.0, 30.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 80.0, 40.0],
                    [0.0, 60.0, 80.0, 30.0],
                    [30.0, 40.0, 50.0, 20.0],
                ],
            ),
            (
                "position:absolute;top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 100.0, 80.0, 40.0],
                    [0.0, 110.0, 80.0, 30.0],
                    [30.0, 90.0, 50.0, 20.0],
                ],
            ),
            (
                "display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 130.0, 10.0],
                    [50.0, 30.0, 80.0, 30.0],
                    [0.0, 30.0, 50.0, 30.0],
                ],
            ),
            (
                "position:absolute;top:50px;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 100.0, 130.0, 10.0],
                    [50.0, 80.0, 80.0, 30.0],
                    [0.0, 80.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    assert_cases_in(
        800.0,
        "margin-top:50px;display:flex;width:400px",
        &[
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 50.0, 80.0, 30.0],
                    [40.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 400.0, 10.0],
                    [320.0, 50.0, 80.0, 30.0],
                    [270.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;max-width:90px",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 60.0, 80.0, 30.0],
                    [40.0, 40.0, 50.0, 20.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 400.0, 10.0],
                    [320.0, 30.0, 80.0, 30.0],
                    [270.0, 30.0, 50.0, 30.0],
                ],
            ),
        ],
    );
    // A row flex container flexes the width it assigns its item, whatever
    // keyword sizes the item.
    assert_cases_in(
        800.0,
        "margin-top:50px;display:flex;width:400px",
        &[
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 400.0, 10.0],
                    [320.0, 50.0, 80.0, 30.0],
                    [270.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px;width:min-content",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 50.0, 80.0, 30.0],
                    [40.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 400.0, 10.0],
                    [320.0, 50.0, 80.0, 30.0],
                    [270.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px;width:max-content",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 50.0, 80.0, 30.0],
                    [40.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 400.0, 10.0],
                    [320.0, 50.0, 80.0, 30.0],
                    [270.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px;width:fit-content",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 50.0, 80.0, 30.0],
                    [40.0, 80.0, 50.0, 20.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 400.0, 10.0],
                    [320.0, 50.0, 80.0, 30.0],
                    [270.0, 50.0, 50.0, 30.0],
                ],
            ),
            (
                "flex:1 0 auto;display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:90px;",
                &[
                    ("writing-mode:horizontal-tb;flex:0 0 80px", "xxxx xxxx xxxx"),
                    ("writing-mode:horizontal-tb;flex:0 0 50px", "xx xx xx xx"),
                ],
                &[
                    [0.0, 50.0, 90.0, 40.0],
                    [10.0, 50.0, 80.0, 30.0],
                    [40.0, 80.0, 50.0, 20.0],
                ],
            ),
        ],
    );
}
