//! The automatic minimum width of a row flex item (CSS Flexbox §4.5) and how it
//! shows in the width of a shrink-to-fit flex container. An item whose height is
//! fixed before its width is measured takes its width from that height along its
//! aspect ratio, and that width is the minimum the item keeps whatever its flex
//! basis. The expected boxes are the ones Chromium lays out for the same markup,
//! with an image of 20x50 pixels, an 8px body margin and a green border of 25px
//! around the item.

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

/// An inline flex container holding one image with the given `image_css`.
fn image(container_css: &str, image_css: &str) -> String {
    format!(
        "<div id=o style=\"{container_css}\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"{image_css}\"></div>"
    )
}

/// An inline flex container holding one box of text with the given `item_css`.
fn text(item_css: &str) -> String {
    format!("<div id=o style=\"{ROW}\"><div id=i style=\"{item_css}\">AAAA BBBB</div></div>")
}

#[test]
fn an_image_with_a_basis_of_zero_keeps_the_width_its_height_gives_along_the_ratio() {
    // The test `flex-aspect-ratio-055`: the 50px high image is 50px wide along
    // its ratio of 1, and a flex basis of zero does not take that away.
    assert_sizes(
        &image(ROW, "height:50px;flex:0 0 0px;aspect-ratio:1/1"),
        [100.0, 100.0],
        [50.0, 50.0],
    );
    assert_sizes(
        &image(ROW, "height:50px;aspect-ratio:1/1"),
        [100.0, 100.0],
        [50.0, 50.0],
    );
    assert_sizes(
        &image(ROW, "height:50px;flex:0 0 auto;aspect-ratio:1/1"),
        [100.0, 100.0],
        [50.0, 50.0],
    );
}

#[test]
fn a_box_with_a_basis_of_zero_takes_its_width_from_its_height_too() {
    assert_sizes(
        &format!(
            "<div id=o style=\"{ROW}\"><div id=i style=\"height:50px;flex:0 0 0px;aspect-ratio:1/1\"></div></div>"
        ),
        [100.0, 100.0],
        [50.0, 50.0],
    );
    assert_sizes(
        &format!(
            "<div id=o style=\"{ROW}\"><div id=i style=\"height:50px;flex:0 0 0px;aspect-ratio:1/2\"></div></div>"
        ),
        [75.0, 100.0],
        [25.0, 50.0],
    );
}

#[test]
fn the_natural_ratio_of_an_image_gives_the_minimum_width_of_its_stated_height() {
    assert_sizes(
        &image(ROW, "height:50px;flex:0 0 0px"),
        [70.0, 100.0],
        [20.0, 50.0],
    );
    assert_sizes(
        &image(ROW, "height:50px;flex:0 1 0px"),
        [70.0, 100.0],
        [20.0, 50.0],
    );
    assert_sizes(&image(ROW, "flex:0 0 0px"), [70.0, 100.0], [20.0, 50.0]);
}

#[test]
fn another_aspect_ratio_gives_another_minimum_width() {
    assert_sizes(
        &image(ROW, "height:50px;flex:0 0 0px;aspect-ratio:2/1"),
        [150.0, 100.0],
        [100.0, 50.0],
    );
}

#[test]
fn a_stated_width_replaces_the_width_along_the_ratio() {
    assert_sizes(
        &image(ROW, "width:10px;height:50px;flex:0 0 0px;aspect-ratio:1/1"),
        [60.0, 100.0],
        [10.0, 50.0],
    );
}

#[test]
fn a_minimum_width_of_zero_lets_the_item_shrink_to_its_basis() {
    assert_sizes(
        &image(ROW, "height:50px;flex:0 0 0px;aspect-ratio:1/1;min-width:0"),
        [50.0, 100.0],
        [0.0, 50.0],
    );
}

#[test]
fn an_item_without_a_stated_height_has_the_minimum_of_its_content() {
    assert_sizes(
        &image(ROW, "flex:0 0 0px;aspect-ratio:1/1"),
        [70.0, 70.0],
        [20.0, 20.0],
    );
}

#[test]
fn an_item_that_grows_keeps_what_the_container_gives_it() {
    assert_sizes(
        &image(
            "display:flex;width:200px;border:solid 25px green",
            "height:50px;flex:1 1 0px;aspect-ratio:1/1",
        ),
        [200.0, 100.0],
        [150.0, 50.0],
    );
    assert_sizes(&text("flex:1 0 0px"), [140.0, 60.0], [90.0, 10.0]);
}

#[test]
fn an_item_stretched_to_a_definite_height_takes_its_minimum_width_from_it() {
    const TALL: &str = "display:flex;height:100px;width:200px;border:solid 25px green";
    // The container is 50px high inside its border, and so is the stretched
    // item; along the ratio of 1 it keeps 50px of width.
    assert_sizes(
        &image(TALL, "flex:0 0 0px;aspect-ratio:1/1"),
        [200.0, 100.0],
        [50.0, 50.0],
    );
    // Along its natural ratio it is 20px wide, which it is without the stretch.
    assert_sizes(&image(TALL, "flex:0 0 0px"), [200.0, 100.0], [20.0, 50.0]);
    assert_sizes(&image(TALL, ""), [200.0, 100.0], [20.0, 50.0]);
}

#[test]
fn the_height_an_item_states_is_kept_within_its_minimum_and_maximum_height() {
    assert_sizes(
        &image(
            ROW,
            "height:200px;max-height:50px;flex:0 0 0px;aspect-ratio:2/1",
        ),
        [150.0, 100.0],
        [100.0, 50.0],
    );
    assert_sizes(
        &image(
            ROW,
            "height:10px;min-height:50px;flex:0 0 0px;aspect-ratio:1/1",
        ),
        [100.0, 100.0],
        [50.0, 50.0],
    );
}

#[test]
fn the_padding_of_an_item_is_inside_or_outside_the_height_it_states() {
    // The content box is 40px high and 40px wide with `content-box`, and 10px
    // of padding makes the box 50px by 50px.
    assert_sizes(
        &image(
            ROW,
            "box-sizing:content-box;padding:5px;height:40px;flex:0 0 0px;aspect-ratio:1/1",
        ),
        [100.0, 100.0],
        [50.0, 50.0],
    );
    assert_sizes(
        &image(
            ROW,
            "box-sizing:border-box;padding:5px;height:50px;flex:0 0 0px;aspect-ratio:1/1",
        ),
        [100.0, 100.0],
        [50.0, 50.0],
    );
    assert_sizes(
        &image(
            ROW,
            "box-sizing:content-box;padding:5px;height:40px;flex:0 0 0px",
        ),
        [76.0, 100.0],
        [26.0, 50.0],
    );
}

#[test]
fn a_container_narrower_than_the_minimum_width_of_its_item_lets_it_overflow() {
    assert_sizes(
        &image(
            "display:flex;width:30px;border:solid 25px green",
            "height:50px;flex:0 1 auto;aspect-ratio:1/1",
        ),
        [50.0, 100.0],
        [50.0, 50.0],
    );
}

#[test]
fn text_with_a_basis_of_zero_is_as_wide_as_its_longest_word() {
    // Two words of four letters, 10px each: the item keeps the 40px of the
    // longest word and wraps, so the container is as wide as that.
    assert_sizes(&text("flex:0 0 0px"), [90.0, 70.0], [40.0, 20.0]);
    assert_sizes(&text("flex:0 1 0px"), [90.0, 70.0], [40.0, 20.0]);
}

#[test]
fn text_with_a_minimum_width_of_zero_shrinks_to_nothing() {
    assert_sizes(&text("flex:0 0 0px;min-width:0"), [50.0, 70.0], [0.0, 20.0]);
}

#[test]
fn text_with_a_basis_wider_than_its_words_keeps_the_basis() {
    assert_sizes(&text("flex:0 0 100px"), [150.0, 60.0], [100.0, 10.0]);
}
