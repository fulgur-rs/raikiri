//! The quirks mode fill quirks: the root element fills the page area and the
//! body fills the root element
//! (<https://quirks.spec.whatwg.org/#the-html-element-fills-the-viewport-quirk>,
//! <https://quirks.spec.whatwg.org/#the-body-element-fills-the-html-element-quirk>).
//!
//! The expected boxes are the ones Chromium lays out in an 800x600 viewport.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// The border boxes of the elements with an `id`, page by page, for a
/// document of `style` and `body` (the rest of the body's start tag, and its
/// content) on an 800x600 page, after `doctype`.
fn boxes(doctype: &str, style: &str, body: &str) -> String {
    let html = format!(
        "{doctype}<html id=h><head><style>@page{{size:800px 600px;margin:0}}\
         body{{font:10px/10px Ahem}}{style}</style></head><body id=b{body}</body></html>"
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
        panic!("layout did not complete");
    };
    let mut out = Vec::new();
    for (n, page) in result.pages().enumerate() {
        let dom = page.dom();
        let mut items: Vec<String> = page
            .fragments()
            .filter_map(|fragment| {
                let id = dom.attr(fragment.node(), "id")?;
                let r = fragment.rect();
                Some(format!(
                    "{id}=({}, {}, {}, {})",
                    r.x, r.y, r.width, r.height
                ))
            })
            .collect();
        items.sort();
        out.push(format!("p{n}: {}", items.join(" ")));
    }
    out.join(" ")
}

fn quirks(style: &str, body: &str) -> String {
    boxes("", style, body)
}

const HALF: &str = "><div id=c style='height:50%'></div>";
const STRETCH: &str = "><div id=c style='height:stretch'></div>";

#[test]
fn root_and_body_fill_the_page_area() {
    assert_eq!(
        quirks("", "><div id=c style='height:10px'></div>"),
        "p0: b=(8, 8, 784, 584) c=(8, 8, 784, 10) h=(0, 0, 800, 600)"
    );
    // Without the quirk both take the height of their content.
    assert_eq!(
        boxes(
            "<!DOCTYPE html>",
            "",
            "><div id=c style='height:10px'></div>"
        ),
        "p0: b=(8, 8, 784, 10) c=(8, 8, 784, 10) h=(0, 0, 800, 26)"
    );
}

#[test]
fn children_resolve_percentages_and_stretch_against_the_fill() {
    assert_eq!(
        quirks("body{margin:0}", HALF),
        "p0: b=(0, 0, 800, 600) c=(0, 0, 800, 300) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("body{margin:0}", STRETCH),
        "p0: b=(0, 0, 800, 600) c=(0, 0, 800, 600) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks(
            "body{margin:0;display:flex}",
            "><div id=c style='height:stretch;width:40px'></div>"
        ),
        "p0: b=(0, 0, 800, 600) c=(0, 0, 40, 600) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks(
            "body{margin:0}",
            "><div id=c style='display:inline-block;height:stretch;width:40px'></div>"
        ),
        "p0: b=(0, 0, 800, 600) c=(0, 0, 40, 600) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("body{margin:0;padding:5px}", STRETCH),
        "p0: b=(0, 0, 800, 600) c=(5, 5, 790, 590) h=(0, 0, 800, 600)"
    );
}

#[test]
fn replaced_and_form_controls_stretch_to_the_fill() {
    assert_eq!(
        quirks(
            "body{margin:0}body>*{box-sizing:border-box;height:stretch;width:40px;\
             border:5px solid blue;vertical-align:top;margin:0 10px 0 0}",
            "><canvas id=c1></canvas><button id=c2>B</button><input id=c3 value=i>\
             <textarea id=c4></textarea>"
        ),
        "p0: b=(0, 0, 800, 600) c1=(0, 0, 40, 600) c2=(50, 0, 40, 600) \
         c3=(100, 0, 40, 600) c4=(150, 0, 40, 600) h=(0, 0, 800, 600)"
    );
}

#[test]
fn stretch_keeps_margins_that_do_not_collapse_through_the_body() {
    // The body has padding at the top and bottom, so both margins stay
    // inside it; the maximum height then wins.
    assert_eq!(
        quirks(
            "body{margin:0;padding:10px 0}",
            "><div id=c style='height:stretch;margin:5px 0;padding:4px;max-height:100px'></div>"
        ),
        "p0: b=(0, 0, 800, 600) c=(0, 15, 800, 108) h=(0, 0, 800, 600)"
    );
    // Only the top margin stays inside a body with a top border, and a
    // percentage minimum resolves against the fill.
    assert_eq!(
        quirks(
            "body{margin:0;border-top:3px solid}",
            "><div id=c style='height:stretch;margin:5px 0;min-height:50%'></div>"
        ),
        "p0: b=(0, 0, 800, 600) c=(0, 8, 800, 592) h=(0, 0, 800, 600)"
    );
}

#[test]
fn the_body_fills_a_stated_root_height() {
    assert_eq!(
        quirks("html{height:300px}body{margin:0}", HALF),
        "p0: b=(0, 0, 800, 300) c=(0, 0, 800, 150) h=(0, 0, 800, 300)"
    );
    assert_eq!(
        quirks("html{height:50%}body{margin:0}", HALF),
        "p0: b=(0, 0, 800, 300) c=(0, 0, 800, 150) h=(0, 0, 800, 300)"
    );
    assert_eq!(
        quirks(
            "html{height:300px;box-sizing:border-box;padding:10px}body{margin:0}",
            HALF
        ),
        "p0: b=(10, 10, 780, 280) c=(10, 10, 780, 140) h=(0, 0, 800, 300)"
    );
}

#[test]
fn edges_of_the_root_and_body_narrow_the_fill() {
    assert_eq!(
        quirks(
            "html{margin:6px;border:4px solid}body{margin:10px;border:5px solid;padding:3px}",
            HALF
        ),
        "p0: b=(20, 20, 760, 560) c=(28, 28, 744, 272) h=(6, 6, 788, 588)"
    );
    // Percentage margins and `calc()` padding resolve against the width.
    assert_eq!(
        quirks("body{margin:5% 0;padding:calc(10px + 1%) 0}", HALF),
        "p0: b=(0, 40, 800, 520) c=(0, 58, 800, 242) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("body{margin:0;box-sizing:border-box;padding:10px}", HALF),
        "p0: b=(0, 0, 800, 600) c=(10, 10, 780, 290) h=(0, 0, 800, 600)"
    );
}

#[test]
fn minimum_and_maximum_heights_clamp_the_fill() {
    assert_eq!(
        quirks("body{margin:0;min-height:50%}", HALF),
        "p0: b=(0, 0, 800, 600) c=(0, 0, 800, 300) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("html{min-height:50%}body{margin:0}", HALF),
        "p0: b=(0, 0, 800, 600) c=(0, 0, 800, 300) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("body{margin:0;max-height:200px}", HALF),
        "p0: b=(0, 0, 800, 200) c=(0, 0, 800, 100) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("body{margin:0;max-height:min(200px, 50%)}", HALF),
        "p0: b=(0, 0, 800, 200) c=(0, 0, 800, 100) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("body{margin:0;max-height:50%}", HALF),
        "p0: b=(0, 0, 800, 300) c=(0, 0, 800, 150) h=(0, 0, 800, 600)"
    );
    // The children of a body with a fixed height take theirs from it.
    assert_eq!(
        quirks("body{margin:0;height:200px}", STRETCH),
        "p0: b=(0, 0, 800, 200) c=(0, 0, 800, 200) h=(0, 0, 800, 600)"
    );
}

#[test]
fn the_root_element_fills_whatever_its_display() {
    for display in ["inline", "contents"] {
        assert_eq!(
            quirks(&format!("html{{display:{display}}}body{{margin:0}}"), HALF),
            "p0: b=(0, 0, 800, 600) c=(0, 0, 800, 300) h=(0, 0, 800, 600)",
            "{display}"
        );
    }
}

#[test]
fn out_of_flow_and_floated_bodies_do_not_fill() {
    assert_eq!(
        quirks("body{margin:0;position:absolute}", HALF),
        "p0: b=(0, 0, 0, 0) c=(0, 0, 0, 0) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks(
            "body{float:left}",
            "><div id=c style='height:10px;width:10px'></div>"
        ),
        "p0: b=(8, 8, 10, 10) c=(8, 8, 10, 10) h=(0, 0, 800, 600)"
    );
}

#[test]
fn a_vertical_root_keeps_its_usual_size() {
    // Chromium fills the root's block axis, its width, here; the quirk is
    // left out of vertical writing modes.
    assert_eq!(
        quirks("html{writing-mode:vertical-rl}body{margin:0}", HALF),
        "p0: b=(800, 0, 0, 600) c=(800, 0, 0, 300) h=(800, 0, 0, 600)"
    );
}

#[test]
fn stretch_keeps_margins_that_cannot_collapse_through_the_parent() {
    // The root element keeps its children's margins inside.
    assert_eq!(
        quirks("body{height:stretch}", HALF),
        "p0: b=(8, 8, 784, 584) c=(8, 8, 784, 292) h=(0, 0, 800, 600)"
    );
    // So do a parent that roots a formatting context and a scroll
    // container, and a float keeps its own.
    let child = "><div id=c style='height:stretch;margin:10px 0'></div>";
    assert_eq!(
        quirks("body{margin:0;display:flow-root}", child),
        "p0: b=(0, 0, 800, 600) c=(0, 10, 800, 580) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks("body{margin:0;overflow:hidden}html{overflow:hidden}", child),
        "p0: b=(0, 0, 800, 600) c=(0, 10, 800, 580) h=(0, 0, 800, 600)"
    );
    assert_eq!(
        quirks(
            "body{margin:0}",
            "><div id=c style='float:left;height:stretch;margin:10px;width:10px'></div>"
        ),
        "p0: b=(0, 0, 800, 600) c=(10, 10, 10, 580) h=(0, 0, 800, 600)"
    );
}

#[test]
fn absolutely_positioned_children_size_against_their_containing_block() {
    assert_eq!(
        quirks(
            "",
            "><div id=c style='position:absolute;height:stretch;width:10px'></div>\
             <div id=d style='position:absolute;height:50%;width:10px'></div>"
        ),
        "p0: b=(8, 8, 784, 584) c=(8, 8, 10, 592) d=(8, 8, 10, 300) h=(0, 0, 800, 600)"
    );
}

#[test]
fn body_margins_resolve_against_the_root_width() {
    assert_eq!(
        quirks("html{width:400px}body{margin:5% 0}", HALF),
        "p0: b=(0, 20, 400, 560) c=(0, 20, 400, 280) h=(0, 0, 400, 600)"
    );
    assert_eq!(
        quirks("html{margin-left:10%;width:300px}body{margin:10%}", HALF),
        "p0: b=(110, 30, 240, 540) c=(110, 30, 240, 270) h=(80, 0, 300, 600)"
    );
    assert_eq!(
        quirks(
            "html{width:50%;box-sizing:border-box;padding:0 20px}body{margin:10% 0}",
            HALF
        ),
        "p0: b=(20, 36, 360, 528) c=(20, 36, 360, 264) h=(0, 0, 400, 600)"
    );
}

#[test]
fn the_body_fills_the_clamped_root_height() {
    assert_eq!(
        quirks("html{height:300px;max-height:100px}body{margin:0}", HALF),
        "p0: b=(0, 0, 800, 100) c=(0, 0, 800, 50) h=(0, 0, 800, 100)"
    );
    assert_eq!(
        quirks("html{height:300px;min-height:400px}body{margin:0}", HALF),
        "p0: b=(0, 0, 800, 400) c=(0, 0, 800, 200) h=(0, 0, 800, 400)"
    );
}
