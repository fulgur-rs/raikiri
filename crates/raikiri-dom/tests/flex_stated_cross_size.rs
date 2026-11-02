//! A flex item that states its cross size keeps it. The `min-*` and `max-*` sizes
//! of the main axis are carried across the aspect ratio only into a cross size
//! that is `auto`: a `max-width` of 30px on an image that states a `height` of
//! 50px and has a ratio of 1 leaves the image 50px high, though 30px wide. The
//! expected boxes are the ones Chromium lays out for the same markup, with an
//! image of 20x50 pixels, an 8px body margin and a green border of 25px around
//! the item.

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
        "<!DOCTYPE html><style>@page {{ size: 800px 3000px; margin: 0 }} \
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

/// Check that the container `o` and the item `i` of `body` are laid out with
/// the given border boxes `(width, height)`; the container starts at the body
/// margin and the item inside its 25px border.
fn assert_sizes(body: &str, container: [f32; 2], item: [f32; 2]) {
    let laid_out = boxes(body);
    let find = |id: &str| {
        laid_out
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("no box with id {id} in {body}"))
            .1
    };
    for (id, want) in [
        ("o", [8.0, 8.0, container[0], container[1]]),
        ("i", [33.0, 33.0, item[0], item[1]]),
    ] {
        let got = find(id);
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

const ROW: &str = "display:inline-flex;border:solid 25px green";
const COLUMN: &str = "display:inline-flex;flex-direction:column;border:solid 25px green";

/// A flex container with the given `container_css` holding one image with the
/// given `item_css`.
fn image(container_css: &str, item_css: &str) -> String {
    format!(
        "<div id=o style=\"{container_css}\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"{item_css}\"></div>"
    )
}

/// A flex container with the given `container_css` holding one empty box with
/// the given `item_css`.
fn block(container_css: &str, item_css: &str) -> String {
    format!("<div id=o style=\"{container_css}\"><div id=i style=\"{item_css}\"></div></div>")
}

#[test]
fn a_maximum_width_leaves_the_height_an_image_states() {
    // The ratio of 1 would give a 30px high image for a 30px wide one.
    assert_sizes(
        &image(
            ROW,
            "height:50px;flex:0 0 0px;aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
    assert_sizes(
        &image(ROW, "height:50px;aspect-ratio:1/1;max-width:30px"),
        [80.0, 100.0],
        [30.0, 50.0],
    );
    assert_sizes(
        &image(
            ROW,
            "height:50px;aspect-ratio:2/1;max-width:30px;flex:0 1 auto",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
}

#[test]
fn a_maximum_width_leaves_the_height_along_the_natural_ratio_too() {
    // The natural ratio of the image is 2:5: 10px of width would be 25px high.
    assert_sizes(
        &image(ROW, "height:50px;max-width:10px"),
        [60.0, 100.0],
        [10.0, 50.0],
    );
}

#[test]
fn a_minimum_width_leaves_the_height_an_image_states() {
    // The ratio of 1/2 would give a 60px high image for a 30px wide one.
    assert_sizes(
        &image(ROW, "height:50px;aspect-ratio:1/2;min-width:30px"),
        [80.0, 100.0],
        [30.0, 50.0],
    );
    assert_sizes(
        &image(
            ROW,
            "height:50px;flex:0 0 0px;aspect-ratio:1/2;min-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
}

#[test]
fn a_box_without_content_keeps_the_height_it_states_too() {
    assert_sizes(
        &block(ROW, "height:50px;aspect-ratio:1/1;max-width:30px"),
        [80.0, 100.0],
        [30.0, 50.0],
    );
    assert_sizes(
        &block(
            ROW,
            "height:50px;flex:0 0 0px;aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
    assert_sizes(
        &block(
            ROW,
            "height:50px;flex:0 0 100px;aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
    assert_sizes(
        &block(ROW, "height:50px;aspect-ratio:1/1;min-width:80px"),
        [130.0, 100.0],
        [80.0, 50.0],
    );
}

#[test]
fn an_item_that_grows_to_its_maximum_width_keeps_its_height() {
    assert_sizes(
        &block(
            "display:flex;width:200px;border:solid 25px green",
            "height:50px;flex:1 1 0px;aspect-ratio:1/1;max-width:30px",
        ),
        [200.0, 100.0],
        [30.0, 50.0],
    );
}

#[test]
fn a_box_with_text_keeps_the_height_it_states() {
    let body = format!(
        "<div id=o style=\"{ROW}\"><div id=i style=\"height:50px;aspect-ratio:1/1;max-width:30px\">AAAA BBBB</div></div>"
    );
    assert_sizes(&body, [80.0, 100.0], [30.0, 50.0]);
}

#[test]
fn a_height_in_any_unit_is_a_height_the_item_states() {
    // A percentage of the 50px the container has inside its border.
    assert_sizes(
        &image(
            "display:inline-flex;height:100px;border:solid 25px green",
            "height:100%;aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
    assert_sizes(
        &image(
            ROW,
            "height:calc(20px + 30px);aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
}

#[test]
fn the_padding_of_an_item_is_inside_or_outside_the_height_it_states() {
    assert_sizes(
        &image(
            ROW,
            "box-sizing:content-box;padding:5px;height:40px;aspect-ratio:1/1;max-width:30px",
        ),
        [90.0, 100.0],
        [40.0, 50.0],
    );
    assert_sizes(
        &image(
            ROW,
            "box-sizing:border-box;padding:5px;height:50px;aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
}

#[test]
fn every_item_of_a_container_keeps_the_height_it_states() {
    let body = format!(
        "<div id=o style=\"{ROW}\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"height:50px;aspect-ratio:1/1;max-width:30px\">\
         <img src=\"https://flex.test/portrait.png\" style=\"height:10px;aspect-ratio:1/1;max-width:5px\"></div>"
    );
    assert_sizes(&body, [85.0, 100.0], [30.0, 50.0]);
    assert_sizes(
        &image(
            &format!("{ROW};align-items:center"),
            "height:50px;aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
}

#[test]
fn a_maximum_height_leaves_the_width_an_item_of_a_column_states() {
    // The ratio of 1 would give a 30px wide image for a 30px high one.
    assert_sizes(
        &image(COLUMN, "width:50px;aspect-ratio:1/1;max-height:30px"),
        [100.0, 80.0],
        [50.0, 30.0],
    );
    assert_sizes(
        &block(COLUMN, "width:50px;aspect-ratio:1/1;max-height:30px"),
        [100.0, 80.0],
        [50.0, 30.0],
    );
    // The natural ratio of 2:5: 20px of height would be 8px wide.
    assert_sizes(
        &image(COLUMN, "width:10px;max-height:20px"),
        [60.0, 70.0],
        [10.0, 20.0],
    );
}

#[test]
fn a_minimum_height_leaves_the_width_an_item_of_a_column_states() {
    // The ratio of 2 would give a 160px wide image for an 80px high one.
    assert_sizes(
        &image(COLUMN, "width:50px;aspect-ratio:2/1;min-height:80px"),
        [100.0, 130.0],
        [50.0, 80.0],
    );
}

#[test]
fn a_cross_size_that_is_auto_still_follows_the_bounds_of_the_main_axis() {
    // Without a height the image is as high as the ratio gives for its width,
    // which is 25px for the 10px of its maximum width.
    assert_sizes(&image(ROW, "max-width:10px"), [60.0, 75.0], [10.0, 25.0]);
    assert_sizes(
        &image(ROW, "aspect-ratio:1/2;min-width:80px"),
        [130.0, 210.0],
        [80.0, 160.0],
    );
    // A container that is 50px high inside its border stretches the item.
    assert_sizes(
        &image(
            "display:inline-flex;height:100px;border:solid 25px green",
            "aspect-ratio:1/1;max-width:30px",
        ),
        [80.0, 100.0],
        [30.0, 50.0],
    );
}
