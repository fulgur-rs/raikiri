//! Images and `<svg>` replaced elements that state one of their sizes in a flex
//! container: the other size follows the main size they end up with after
//! flexing, and a `max-width` on the width of an `<svg>` leaves its stated
//! height alone. The expected boxes are the ones Chromium lays out for the same
//! markup, with an image of 20x50 pixels and an 8px body margin: each is the
//! element's left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver, ResolveDisposition,
    ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use url::Url;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Every image is 20x50 pixels, a 2:5 aspect ratio.
struct Portrait;

impl ReplacedResolver for Portrait {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(20.0, 50.0),
            ResolveDisposition::Ok,
        ))
    }
}

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://flex.test/document.html").expect("url"))
        .replaced_resolver(&Portrait);
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
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            assert!(
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            );
        }
    }
}

/// An image that states its main size and leaves the other `auto` takes the other size from the main size it ends up with after the flex algorithm grew or shrank it, along its ratio, not from the size it states; a stretched cross size wins over the ratio.
#[test]
fn an_image_takes_its_cross_size_from_the_flexed_main_size() {
    let cases: &[Case<'_>] = &[
        (
            "a column that grows an image by its height",
            "<div id=a style='display:flex;flex-direction:column;align-items:flex-start;width:200px;height:100px'><img id=b src='https://flex.test/portrait.png' style='height:40px;flex-grow:1'></div>",
            &[("a", 8.0, 8.0, 200.0, 100.0), ("b", 8.0, 8.0, 40.0, 100.0)],
        ),
        (
            "a column that centers the image",
            "<div id=a style='display:flex;flex-direction:column;align-items:center;width:200px;height:100px'><img id=b src='https://flex.test/portrait.png' style='height:40px;flex-grow:1'></div>",
            &[("a", 8.0, 8.0, 200.0, 100.0), ("b", 88.0, 8.0, 40.0, 100.0)],
        ),
        (
            "a column that stretches the image",
            "<div id=a style='display:flex;flex-direction:column;width:200px;height:100px'><img id=b src='https://flex.test/portrait.png' style='height:40px;flex-grow:1'></div>",
            &[("a", 8.0, 8.0, 200.0, 100.0), ("b", 8.0, 8.0, 200.0, 100.0)],
        ),
        (
            "a column that shrinks an image by its height",
            "<div id=a style='display:flex;flex-direction:column;align-items:flex-start;width:200px;height:30px'><img id=b src='https://flex.test/portrait.png' style='height:80px;flex-shrink:1'></div>",
            &[("a", 8.0, 8.0, 200.0, 30.0), ("b", 8.0, 8.0, 20.0, 50.0)],
        ),
        (
            "a row that grows an image by its width",
            "<div id=a style='display:flex;align-items:flex-start;width:100px;height:300px'><img id=b src='https://flex.test/portrait.png' style='width:40px;flex-grow:1'></div>",
            &[("a", 8.0, 8.0, 100.0, 300.0), ("b", 8.0, 8.0, 100.0, 250.0)],
        ),
        (
            "a row that stretches the image",
            "<div id=a style='display:flex;width:100px;height:300px'><img id=b src='https://flex.test/portrait.png' style='width:40px;flex-grow:1'></div>",
            &[("a", 8.0, 8.0, 100.0, 300.0), ("b", 8.0, 8.0, 100.0, 300.0)],
        ),
        (
            "a row that shrinks an image by its width",
            "<div id=a style='display:flex;align-items:flex-start;width:30px;height:300px'><img id=b src='https://flex.test/portrait.png' style='width:80px;flex-shrink:1;min-width:0'></div>",
            &[("a", 8.0, 8.0, 30.0, 300.0), ("b", 8.0, 8.0, 30.0, 75.0)],
        ),
        (
            "a column whose image has a flex basis of zero",
            "<div id=a style='display:flex;flex-direction:column;align-items:flex-start;width:200px;height:100px'><img id=b src='https://flex.test/portrait.png' style='height:40px;flex:1 1 0'></div>",
            &[("a", 8.0, 8.0, 200.0, 100.0), ("b", 8.0, 8.0, 40.0, 100.0)],
        ),
        (
            "a row that stops at the maximum width",
            "<div id=a style='display:flex;align-items:flex-start;width:100px;height:300px'><img id=b src='https://flex.test/portrait.png' style='width:40px;flex-grow:1;max-width:60px'></div>",
            &[("a", 8.0, 8.0, 100.0, 300.0), ("b", 8.0, 8.0, 60.0, 150.0)],
        ),
        (
            "a column that stops at the maximum height",
            "<div id=a style='display:flex;flex-direction:column;align-items:flex-start;width:200px;height:100px'><img id=b src='https://flex.test/portrait.png' style='height:40px;flex-grow:1;max-height:80px'></div>",
            &[("a", 8.0, 8.0, 200.0, 100.0), ("b", 8.0, 8.0, 32.0, 80.0)],
        ),
        (
            "a row whose image shares the room with another",
            "<div id=a style='display:flex;align-items:flex-start;width:100px;height:300px'><img id=b src='https://flex.test/portrait.png' style='width:40px;flex-grow:1'><img id=c src='https://flex.test/portrait.png' style='width:20px'></div>",
            &[
                ("a", 8.0, 8.0, 100.0, 300.0),
                ("b", 8.0, 8.0, 80.0, 200.0),
                ("c", 88.0, 8.0, 20.0, 50.0),
            ],
        ),
    ];
    check_boxes(cases);
}

/// An inline `<svg>` that states its size by attributes keeps the height of its `height` attribute when a percentage `width` is cut by `max-width`: the maximum does not carry across a ratio to the height of an image that states one.
#[test]
fn an_svg_keeps_its_height_attribute_when_a_maximum_width_cuts_its_width() {
    let cases: &[Case<'_>] = &[
        (
            "an inline-flex container",
            "<div id=a style='display:inline-flex'><svg id=b width=100 height=50 style='width:50%;max-width:80px'></svg></div>",
            &[("a", 8.0, 8.0, 80.0, 50.0), ("b", 8.0, 8.0, 40.0, 50.0)],
        ),
        (
            "a flex container",
            "<div id=a style='display:flex;width:200px'><svg id=b width=100 height=50 style='width:50%;max-width:80px'></svg></div>",
            &[("a", 8.0, 8.0, 200.0, 50.0), ("b", 8.0, 8.0, 80.0, 50.0)],
        ),
        (
            "an inline-block container",
            "<div id=a style='display:inline-block'><svg id=b width=100 height=50 style='width:50%;max-width:80px'></svg></div>",
            &[("a", 8.0, 8.0, 80.0, 52.0), ("b", 8.0, 8.0, 40.0, 50.0)],
        ),
    ];
    check_boxes(cases);
}
