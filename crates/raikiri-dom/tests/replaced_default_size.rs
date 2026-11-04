//! Replaced content without a natural size, and the width it takes where its
//! `width` is `auto`.
//!
//! An `<iframe>`, a `<video>` and an `<embed>` take the 300x150 default object
//! size (CSS 2.1 section 10.3.2), and none of them has a natural ratio, so a
//! stated height does not give them a width. Their `width` and `height` attributes size them. An inline
//! `<svg>` with no ratio takes that size too, while one with only a `viewBox`
//! fills the room its parent offers, which beside a float is the room the
//! float leaves, and an `<img>` with no source is a box of no width and no
//! height.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body: a block
//! beside a 100px by 100px float in a 200px container, with no float, and as
//! an inline-block in a 400px container. The elements carry no padding or
//! border of their own unless a row says so.

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

/// Where the element with the `id` `c` is put.
#[derive(Clone, Copy, Debug)]
enum Ctx {
    /// After a 100px left float, in a 200px container.
    Float,
    /// In a 200px container.
    Plain,
    /// As an inline-block, in a 400px container.
    Inline,
    /// After a 60px right float and a 40px left one, in a 200px container.
    TwoFloats,
}

fn page(element: &str, ctx: Ctx) -> String {
    match ctx {
        Ctx::Float => format!(
            "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\">\
             </div>{element}</div>"
        ),
        Ctx::Plain => format!("<div id=o style=\"width:200px\">{element}</div>"),
        Ctx::Inline => format!(
            "<div id=o style=\"width:400px\">{}</div>",
            element.replace("display:block", "display:inline-block")
        ),
        Ctx::TwoFloats => format!(
            "<div id=o style=\"width:200px\"><div style=\"float:right;width:60px;height:30px\">\
             </div><div style=\"float:left;width:40px;height:100px\"></div>{element}</div>"
        ),
    }
}

/// The border box of the element `c` on the first page.
fn element_box(body: &str) -> Option<Rect> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    page.fragments().find_map(|fragment| {
        (dom.attr(fragment.node(), "id")? == "c").then(|| {
            let rect = fragment.rect();
            (rect.x, rect.y, rect.width, rect.height)
        })
    })
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(rows: &[(&str, Ctx, Rect)]) {
    let mut failures = Vec::new();
    for &(element, ctx, expected) in rows {
        let body = page(element, ctx);
        let found = element_box(&body);
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        let matches = found.is_some_and(|found| {
            close(found.0, expected.0)
                && close(found.1, expected.1)
                && close(found.2, expected.2)
                && close(found.3, expected.3)
        });
        if !matches {
            failures.push(format!(
                "{ctx:?} {element}: is {found:?}, expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const ROWS_EMBEDDED: &[(&str, Ctx, Rect)] = &[
    (
        r#"<iframe id=c style="display:block;border:0"></iframe>"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 150.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0"></iframe>"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0"></iframe>"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0;height:20px"></iframe>"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 20.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0;height:20px"></iframe>"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 20.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0;height:20px"></iframe>"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 20.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0;width:40px"></iframe>"#,
        Ctx::Float,
        (100.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0;width:40px"></iframe>"#,
        Ctx::Plain,
        (0.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<iframe id=c style="display:block;border:0;width:40px"></iframe>"#,
        Ctx::Inline,
        (0.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<iframe id=c width=60 height=30 style="display:block;border:0"></iframe>"#,
        Ctx::Float,
        (100.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<iframe id=c width=60 height=30 style="display:block;border:0"></iframe>"#,
        Ctx::Plain,
        (0.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<iframe id=c width=60 height=30 style="display:block;border:0"></iframe>"#,
        Ctx::Inline,
        (0.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<video id=c style="display:block"></video>"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 150.0),
    ),
    (
        r#"<video id=c style="display:block"></video>"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<video id=c style="display:block"></video>"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<video id=c style="display:block;height:20px"></video>"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 20.0),
    ),
    (
        r#"<video id=c style="display:block;height:20px"></video>"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 20.0),
    ),
    (
        r#"<video id=c style="display:block;height:20px"></video>"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 20.0),
    ),
    (
        r#"<video id=c style="display:block;width:40px"></video>"#,
        Ctx::Float,
        (100.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<video id=c style="display:block;width:40px"></video>"#,
        Ctx::Plain,
        (0.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<video id=c style="display:block;width:40px"></video>"#,
        Ctx::Inline,
        (0.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<video id=c width=60 height=30 style="display:block"></video>"#,
        Ctx::Float,
        (100.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<video id=c width=60 height=30 style="display:block"></video>"#,
        Ctx::Plain,
        (0.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<video id=c width=60 height=30 style="display:block"></video>"#,
        Ctx::Inline,
        (0.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<embed id=c src="x.pdf" style="display:block">"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 150.0),
    ),
    (
        r#"<embed id=c src="x.pdf" style="display:block">"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<embed id=c src="x.pdf" style="display:block">"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<embed id=c src="x.pdf" style="display:block;height:20px">"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 20.0),
    ),
    (
        r#"<embed id=c src="x.pdf" style="display:block;height:20px">"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 20.0),
    ),
    (
        r#"<embed id=c src="x.pdf" style="display:block;height:20px">"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 20.0),
    ),
];

const ROWS_SVG: &[(&str, Ctx, Rect)] = &[
    (
        r#"<svg id=c style="display:block"></svg>"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 150.0),
    ),
    (
        r#"<svg id=c style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<svg id=c style="display:block"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 150.0),
    ),
    (
        r#"<svg id=c style="display:block;height:20px"></svg>"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 20.0),
    ),
    (
        r#"<svg id=c style="display:block;height:20px"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 20.0),
    ),
    (
        r#"<svg id=c style="display:block;height:20px"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 20.0),
    ),
    (
        r#"<svg id=c style="display:block;width:40px"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<svg id=c style="display:block;width:40px"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<svg id=c style="display:block;width:40px"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 40.0, 150.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 200.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 400.0, 400.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;height:20px"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 20.0, 20.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;height:20px"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 20.0, 20.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;height:20px"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 20.0, 20.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;width:40px"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 40.0, 40.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;width:40px"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 40.0, 40.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;width:40px"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 40.0, 40.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 20 10" style="display:block"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 100.0, 50.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 20 10" style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 20 10" style="display:block"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 400.0, 200.0),
    ),
    (
        r#"<svg id=c width=50 style="display:block"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 50.0, 150.0),
    ),
    (
        r#"<svg id=c width=50 style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 50.0, 150.0),
    ),
    (
        r#"<svg id=c width=50 style="display:block"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 50.0, 150.0),
    ),
    (
        r#"<svg id=c width=50 height=30 style="display:block"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 50.0, 30.0),
    ),
    (
        r#"<svg id=c width=50 height=30 style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 50.0, 30.0),
    ),
    (
        r#"<svg id=c width=50 height=30 style="display:block"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 50.0, 30.0),
    ),
    (
        r#"<svg id=c height=30 style="display:block"></svg>"#,
        Ctx::Float,
        (0.0, 100.0, 300.0, 30.0),
    ),
    (
        r#"<svg id=c height=30 style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 300.0, 30.0),
    ),
    (
        r#"<svg id=c height=30 style="display:block"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 300.0, 30.0),
    ),
    (
        r#"<svg id=c width=50 viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 50.0, 50.0),
    ),
    (
        r#"<svg id=c width=50 viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 50.0, 50.0),
    ),
    (
        r#"<svg id=c width=50 viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Inline,
        (0.0, 0.0, 50.0, 50.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin:0 10px"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 90.0, 90.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin:0 10px"></svg>"#,
        Ctx::Plain,
        (10.0, 0.0, 180.0, 180.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin:0 10px"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin-left:30px"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin-left:30px"></svg>"#,
        Ctx::Plain,
        (30.0, 0.0, 170.0, 170.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin-left:30px"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;padding:0 10px"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 100.0, 80.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;padding:0 10px"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 180.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;padding:0 10px"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 0.0, 100.0, 80.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;padding:0 10px;box-sizing:content-box"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 100.0, 80.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;padding:0 10px;box-sizing:content-box"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 180.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;padding:0 10px;box-sizing:content-box"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 0.0, 100.0, 80.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;max-width:60px"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 60.0, 60.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;max-width:60px"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 60.0, 60.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;max-width:60px"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 0.0, 60.0, 60.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;min-width:150px"></svg>"#,
        Ctx::Float,
        (0.0, 100.0, 200.0, 200.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;min-width:150px"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 200.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;min-width:150px"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 30.0, 160.0, 160.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin:0 auto;max-width:60px"></svg>"#,
        Ctx::Float,
        (120.0, 0.0, 60.0, 60.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin:0 auto;max-width:60px"></svg>"#,
        Ctx::Plain,
        (70.0, 0.0, 60.0, 60.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;margin:0 auto;max-width:60px"></svg>"#,
        Ctx::TwoFloats,
        (60.0, 0.0, 60.0, 60.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;width:50%"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;width:50%"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block;width:50%"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Float,
        (100.0, 0.0, 100.0, 100.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 200.0),
    ),
    (
        r#"<svg id=c viewBox="0 0 10 10" style="display:block"></svg>"#,
        Ctx::TwoFloats,
        (40.0, 0.0, 100.0, 100.0),
    ),
];

const ROWS_IMG: &[(&str, Ctx, Rect)] = &[
    (
        r#"<img id=c style="display:block">"#,
        Ctx::Float,
        (100.0, 0.0, 0.0, 0.0),
    ),
    (
        r#"<img id=c style="display:block">"#,
        Ctx::Plain,
        (0.0, 0.0, 0.0, 0.0),
    ),
    (
        r#"<img id=c style="display:block">"#,
        Ctx::Inline,
        (0.0, 8.0, 0.0, 0.0),
    ),
    (
        r#"<img id=c style="display:block;height:20px">"#,
        Ctx::Float,
        (100.0, 0.0, 0.0, 20.0),
    ),
    (
        r#"<img id=c style="display:block;height:20px">"#,
        Ctx::Plain,
        (0.0, 0.0, 0.0, 20.0),
    ),
    (
        r#"<img id=c style="display:block;height:20px">"#,
        Ctx::Inline,
        (0.0, 0.0, 0.0, 20.0),
    ),
    (
        r#"<img id=c style="display:block;width:40px">"#,
        Ctx::Float,
        (100.0, 0.0, 40.0, 0.0),
    ),
    (
        r#"<img id=c style="display:block;width:40px">"#,
        Ctx::Plain,
        (0.0, 0.0, 40.0, 0.0),
    ),
    (
        r#"<img id=c style="display:block;width:40px">"#,
        Ctx::Inline,
        (0.0, 8.0, 40.0, 0.0),
    ),
    (
        r#"<img id=c width=60 height=30 style="display:block">"#,
        Ctx::Float,
        (100.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<img id=c width=60 height=30 style="display:block">"#,
        Ctx::Plain,
        (0.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<img id=c width=60 height=30 style="display:block">"#,
        Ctx::Inline,
        (0.0, 0.0, 60.0, 30.0),
    ),
    (
        r#"<img id=c alt="ab" style="display:block;height:20px">"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 20.0),
    ),
    (
        r#"<img id=c alt="" style="display:block;height:20px">"#,
        Ctx::Plain,
        (0.0, 0.0, 0.0, 20.0),
    ),
    (
        r#"<img id=c src="" style="display:block;height:20px">"#,
        Ctx::Plain,
        (0.0, 0.0, 200.0, 20.0),
    ),
];

#[test]
fn embedded_content_takes_the_default_object_size() {
    check(ROWS_EMBEDDED);
}

#[test]
fn an_svg_takes_its_size_from_its_ratio_or_the_default_object_size() {
    check(ROWS_SVG);
}

#[test]
fn an_image_without_a_source_has_no_size() {
    check(ROWS_IMG);
}
