//! An image that states no size and has an aspect ratio in a row flex container
//! keeps its natural width within the `min-*` and `max-*` sizes it states: the
//! sizes of one axis carry over to the other along the ratio (CSS 2.1 §10.4),
//! but a `max-width` above the natural width does not widen the image, and a
//! container whose height is not definite stretches nothing for the ratio to
//! turn into a width. The expected boxes are the ones Chromium lays out for the
//! same markup, with an image of 20x50 pixels, an 8px body margin and a green
//! border of 25px around the items.

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

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page with an 8px margin.
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

/// Check that the container `o` of `body` has the given border box size,
/// starting at the body margin, and that each of `items` is laid out with the
/// given border box `(x, y, width, height)`.
fn assert_boxes(body: &str, container: [f32; 2], items: &[(&str, [f32; 4])]) {
    let laid_out = boxes(body);
    let find = |id: &str| {
        laid_out
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("no box with id {id} in {body}"))
            .1
    };
    let container_box = [8.0, 8.0, container[0], container[1]];
    for (id, want) in std::iter::once(&("o", container_box)).chain(items) {
        let got = find(id);
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// An inline flex container with a green border of 25px.
const ROW: &str = "display:inline-flex;border:solid 25px green";

/// An image with the given `css`, named `id`.
fn image(id: &str, css: &str) -> String {
    format!("<img id={id} src=\"https://flex.test/portrait.png\" style=\"{css}\">")
}

/// A container with the given `container_css` that holds `items`.
fn container(container_css: &str, items: &str) -> String {
    format!("<div id=o style=\"{container_css}\">{items}</div>")
}

#[test]
fn an_image_with_a_ratio_and_a_maximum_width_keeps_its_natural_width() {
    // The ratio of 1 gives 20px for the natural width of 20px, whatever the
    // maximum width is, as long as it is not below it.
    for (item, size, item_box) in [
        (
            "aspect-ratio:1/1;max-width:30px",
            [70.0, 70.0],
            [20.0, 20.0],
        ),
        (
            "aspect-ratio:1/1;max-width:100px",
            [70.0, 70.0],
            [20.0, 20.0],
        ),
        (
            "aspect-ratio:1/1;max-width:calc(10px + 20px)",
            [70.0, 70.0],
            [20.0, 20.0],
        ),
        (
            "aspect-ratio:2/1;max-width:30px",
            [70.0, 60.0],
            [20.0, 10.0],
        ),
        (
            "aspect-ratio:1/2;max-width:30px",
            [70.0, 90.0],
            [20.0, 40.0],
        ),
        (
            "aspect-ratio:2/5;max-width:100px",
            [70.0, 100.0],
            [20.0, 50.0],
        ),
        (
            "aspect-ratio:1/1;max-width:30px;flex:0 0 auto",
            [70.0, 70.0],
            [20.0, 20.0],
        ),
        (
            "aspect-ratio:1/1;max-width:30px;flex:0 0 0px",
            [70.0, 70.0],
            [20.0, 20.0],
        ),
        // A basis of a length is the size the image has.
        (
            "aspect-ratio:1/1;max-width:30px;flex:0 0 25px",
            [75.0, 75.0],
            [25.0, 25.0],
        ),
        // A maximum width below the natural width wins.
        (
            "aspect-ratio:1/1;max-width:10px",
            [60.0, 60.0],
            [10.0, 10.0],
        ),
        ("aspect-ratio:1/1;max-width:0px", [50.0, 50.0], [0.0, 0.0]),
        // A minimum width above it too, and wins over the maximum.
        (
            "aspect-ratio:1/1;min-width:30px",
            [80.0, 80.0],
            [30.0, 30.0],
        ),
        (
            "aspect-ratio:1/1;min-width:40px;max-width:30px",
            [90.0, 90.0],
            [40.0, 40.0],
        ),
        // Both maximums bring the size within the smaller of the two.
        (
            "aspect-ratio:1/1;max-width:30px;max-height:10px",
            [60.0, 60.0],
            [10.0, 10.0],
        ),
        // The maximum and minimum height carry across to the width.
        (
            "aspect-ratio:1/1;max-height:30px",
            [70.0, 70.0],
            [20.0, 20.0],
        ),
        (
            "aspect-ratio:1/1;min-height:30px",
            [80.0, 80.0],
            [30.0, 30.0],
        ),
        // The padding is outside the content box that the sizes bound.
        (
            "padding:5px;aspect-ratio:1/1;max-width:30px",
            [80.0, 80.0],
            [30.0, 30.0],
        ),
        (
            "padding:5px;aspect-ratio:1/1;max-width:30px;box-sizing:border-box",
            [80.0, 80.0],
            [30.0, 30.0],
        ),
        // So are the same sizes of an image with its natural ratio.
        ("max-height:30px", [62.0, 80.0], [12.0, 30.0]),
        ("max-width:10px;max-height:30px", [60.0, 75.0], [10.0, 25.0]),
        ("min-height:100px", [90.0, 150.0], [40.0, 100.0]),
    ] {
        assert_boxes(
            &container(ROW, &image("i", item)),
            size,
            &[("i", [33.0, 33.0, item_box[0], item_box[1]])],
        );
    }
}

#[test]
fn an_image_with_a_ratio_and_a_maximum_width_keeps_its_natural_width_in_any_container() {
    let item = "aspect-ratio:1/1;max-width:30px";
    for (container_css, size, item_box) in [
        // A container that wraps, runs in reverse, or has a width.
        (
            "display:inline-flex;border:solid 25px green;flex-wrap:wrap",
            [70.0, 70.0],
            [33.0, 33.0, 20.0, 20.0],
        ),
        (
            "display:inline-flex;border:solid 25px green;flex-direction:row-reverse",
            [70.0, 70.0],
            [33.0, 33.0, 20.0, 20.0],
        ),
        (
            "display:inline-flex;border:solid 25px green;width:100px",
            [100.0, 70.0],
            [33.0, 33.0, 20.0, 20.0],
        ),
        // A container too narrow for it shrinks the image.
        (
            "display:flex;border:solid 25px green;width:10px",
            [50.0, 70.0],
            [33.0, 33.0, 20.0, 20.0],
        ),
        // A column.
        (
            "display:inline-flex;border:solid 25px green;flex-direction:column",
            [70.0, 70.0],
            [33.0, 33.0, 20.0, 20.0],
        ),
        // A container that has a height of its own, as the minimum height does
        // not give the image a height to take its width from.
        (
            "display:inline-flex;border:solid 25px green;min-height:100px",
            [70.0, 100.0],
            [33.0, 33.0, 20.0, 50.0],
        ),
        (
            "display:inline-flex;border:solid 25px green;height:100px;align-items:flex-start",
            [70.0, 100.0],
            [33.0, 33.0, 20.0, 20.0],
        ),
    ] {
        assert_boxes(
            &container(container_css, &image("i", item)),
            size,
            &[("i", item_box)],
        );
    }
    // A margin is outside the image.
    assert_boxes(
        &container(
            ROW,
            &image("i", "aspect-ratio:1/1;max-width:30px;margin:5px"),
        ),
        [80.0, 80.0],
        &[("i", [38.0, 38.0, 20.0, 20.0])],
    );
    // Several images each keep their width.
    assert_boxes(
        &container(
            ROW,
            &format!(
                "{}{}",
                image("i", "aspect-ratio:1/1;max-width:30px"),
                image("j", "aspect-ratio:2/1;max-width:60px")
            ),
        ),
        [90.0, 70.0],
        &[
            ("i", [33.0, 33.0, 20.0, 20.0]),
            ("j", [53.0, 33.0, 20.0, 20.0]),
        ],
    );
    // An image that grows takes its maximum width, and none beyond it.
    assert_boxes(
        &container(
            "display:flex;width:200px;border:solid 25px green",
            &image("i", "aspect-ratio:1/1;max-width:30px;flex:1 1 auto"),
        ),
        [200.0, 80.0],
        &[("i", [33.0, 33.0, 30.0, 30.0])],
    );
    assert_boxes(
        &container(
            "display:flex;width:200px;border:solid 25px green",
            &image("i", "aspect-ratio:1/1;flex:1 1 auto"),
        ),
        [200.0, 200.0],
        &[("i", [33.0, 33.0, 150.0, 150.0])],
    );
}

#[test]
fn an_image_that_the_container_stretches_takes_a_width_from_a_height_that_is_definite_only() {
    let item = "aspect-ratio:1/1;max-width:30px";
    let block = "<div id=s style=\"width:10px;height:70px\"></div>";
    // A sibling that makes the line 70px high stretches the image, which keeps
    // the width it has: the height of the line is not known before.
    assert_boxes(
        &container(ROW, &format!("{}{block}", image("i", item))),
        [80.0, 120.0],
        &[
            ("i", [33.0, 33.0, 20.0, 70.0]),
            ("s", [53.0, 33.0, 10.0, 70.0]),
        ],
    );
    assert_boxes(
        &container(
            &format!("{ROW};align-items:center"),
            &format!("{}{block}", image("i", item)),
        ),
        [80.0, 120.0],
        &[
            ("i", [33.0, 58.0, 20.0, 20.0]),
            ("s", [53.0, 33.0, 10.0, 70.0]),
        ],
    );
    // A container with a height of its own gives the stretched image that
    // height, and the ratio gives the width within its bounds.
    for (css, size, item_box) in [
        (item, [80.0, 100.0], [30.0, 50.0]),
        (
            "aspect-ratio:1/1;max-height:30px",
            [80.0, 100.0],
            [30.0, 30.0],
        ),
        (
            "aspect-ratio:1/1;min-height:20%",
            [100.0, 100.0],
            [50.0, 50.0],
        ),
    ] {
        assert_boxes(
            &container(&format!("{ROW};height:100px"), &image("i", css)),
            size,
            &[("i", [33.0, 33.0, item_box[0], item_box[1]])],
        );
    }
    // The line of a vertical writing mode runs the other way, and stays as it
    // was.
    assert_boxes(
        &container(
            &format!("{ROW};writing-mode:vertical-rl"),
            &image("i", item),
        ),
        [80.0, 80.0],
        &[("i", [33.0, 33.0, 30.0, 30.0])],
    );
}

#[test]
fn an_image_with_a_ratio_keeps_its_width_beside_a_sibling_and_inside_another_container() {
    let sibling = "<div id=s style=\"width:10px;height:70px\"></div>";
    // A sibling that makes the line 70px high stretches the image whatever
    // its flex basis and maximum width are, and leaves its width as it is.
    for item in [
        "aspect-ratio:1/1",
        "aspect-ratio:1/1;flex:0 0 0px",
        "aspect-ratio:1/1;flex:0 0 0px;max-width:30px",
        "",
    ] {
        assert_boxes(
            &container(ROW, &format!("{}{sibling}", image("i", item))),
            [80.0, 120.0],
            &[
                ("i", [33.0, 33.0, 20.0, 70.0]),
                ("s", [53.0, 33.0, 10.0, 70.0]),
            ],
        );
    }
    assert_boxes(
        &container(
            ROW,
            &format!(
                "{}{sibling}",
                image("i", "aspect-ratio:1/1;max-width:30px;align-self:flex-start")
            ),
        ),
        [80.0, 120.0],
        &[
            ("i", [33.0, 33.0, 20.0, 20.0]),
            ("s", [53.0, 33.0, 10.0, 70.0]),
        ],
    );
    // A box of intrinsic size that is contained is sized the same way.
    let contained = "<div id=i style=\"contain:size;contain-intrinsic-size:20px 50px;\
                     aspect-ratio:1/1\"></div>";
    assert_boxes(
        &container(ROW, contained),
        [70.0, 100.0],
        &[("i", [33.0, 33.0, 20.0, 50.0])],
    );
    assert_boxes(
        &container(ROW, &format!("{contained}{sibling}")),
        [80.0, 120.0],
        &[
            ("i", [33.0, 33.0, 20.0, 70.0]),
            ("s", [53.0, 33.0, 10.0, 70.0]),
        ],
    );
    // A flex basis that is a length, or a percentage of the container.
    assert_boxes(
        &container(
            ROW,
            &image("i", "aspect-ratio:1/1;flex:1 1 100px;max-width:30px"),
        ),
        [70.0, 70.0],
        &[("i", [33.0, 33.0, 20.0, 20.0])],
    );
    assert_boxes(
        &container(
            "display:inline-flex;width:100px;border:solid 25px green",
            &image("i", "aspect-ratio:1/1;flex:0 0 50%"),
        ),
        [100.0, 75.0],
        &[("i", [33.0, 33.0, 25.0, 25.0])],
    );
    // A percentage maximum or minimum width.
    assert_boxes(
        &container(ROW, &image("i", "aspect-ratio:1/1;max-width:50%")),
        [70.0, 60.0],
        &[("i", [33.0, 33.0, 10.0, 10.0])],
    );
    assert_boxes(
        &container(
            "display:flex;width:200px;border:solid 25px green",
            &image("i", "aspect-ratio:1/1;min-width:50%"),
        ),
        [200.0, 125.0],
        &[("i", [33.0, 33.0, 75.0, 75.0])],
    );
    // The inner container is an item of the outer one. It is stretched to the
    // height of the outer one, which is definite for what is inside it, only
    // when the outer one has a height.
    let inner = |css: &str| {
        format!(
            "<div id=m style=\"display:inline-flex\">{}</div>",
            image("i", css)
        )
    };
    assert_boxes(
        &container(
            &format!("{ROW};height:100px"),
            &inner("aspect-ratio:1/1;flex:0 0 0px"),
        ),
        [100.0, 100.0],
        &[
            ("m", [33.0, 33.0, 50.0, 50.0]),
            ("i", [33.0, 33.0, 50.0, 50.0]),
        ],
    );
    assert_boxes(
        &container(
            &format!("{ROW};height:100px"),
            &inner("aspect-ratio:1/1;max-width:30px"),
        ),
        [80.0, 100.0],
        &[
            ("m", [33.0, 33.0, 30.0, 50.0]),
            ("i", [33.0, 33.0, 30.0, 50.0]),
        ],
    );
    assert_boxes(
        &container(ROW, &inner("aspect-ratio:1/1;max-width:30px")),
        [70.0, 70.0],
        &[
            ("m", [33.0, 33.0, 20.0, 20.0]),
            ("i", [33.0, 33.0, 20.0, 20.0]),
        ],
    );
    // A container that wraps and has a height stretches the image to it, but
    // as that is no definite height, the width stays.
    for item in [
        "aspect-ratio:1/1;max-width:30px",
        "aspect-ratio:1/1;min-height:20%",
    ] {
        assert_boxes(
            &container(
                &format!("{ROW};height:100px;flex-wrap:wrap"),
                &image("i", item),
            ),
            [70.0, 100.0],
            &[("i", [33.0, 33.0, 20.0, 50.0])],
        );
    }
    // A percentage height of the container gives the width along the ratio.
    assert_boxes(
        &container(
            &format!("{ROW};height:100px"),
            &image("i", "aspect-ratio:1/1;height:50%"),
        ),
        [75.0, 100.0],
        &[("i", [33.0, 33.0, 25.0, 25.0])],
    );
}
