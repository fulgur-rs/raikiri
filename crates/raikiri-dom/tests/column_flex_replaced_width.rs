//! An image that is an item of a column flex container takes its width from a
//! height along its aspect ratio (CSS Flexbox §9.8 and CSS Sizing 4 §5.1).
//! While the container's own width is measured, that is the height the image
//! states. Once the width of the container is fixed, it is the height the
//! container flexed the image to. The expected boxes are the ones Chromium lays
//! out for the same markup, with an image of 20x50 pixels.

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
/// `body` is laid out on a page of the given height with an 8px margin.
fn boxes_on_page(body: &str, page_height: u32) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px {page_height}px; margin: 0 }} \
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

/// Check that every `(id, expected box)` of `expected` is laid out as given.
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    assert_boxes_on_page(body, 600, expected);
}

/// Like `assert_boxes`, on a page of the given height. A page taller than the
/// content keeps an image taller than 600px whole on the first page.
fn assert_boxes_on_page(body: &str, page_height: u32, expected: &[(&str, [f32; 4])]) {
    let laid_out = boxes_on_page(body, page_height);
    for (id, want) in expected {
        let got = laid_out
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("no box with id {id} in {body}"))
            .1;
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// An inline column flex container with the given `container_css`, holding one
/// image with the given `image_css`.
fn column(container_css: &str, image_css: &str) -> String {
    format!(
        "<div id=o style=\"display:inline-flex;flex-direction:column;{container_css}\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"{image_css}\"></div>"
    )
}

#[test]
fn a_height_the_image_states_gives_its_width_along_the_natural_ratio() {
    assert_boxes(
        &column("", "height:100px"),
        &[
            ("o", [8.0, 8.0, 40.0, 100.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn a_height_the_image_states_gives_its_width_along_a_stated_ratio() {
    assert_boxes(
        &column("", "aspect-ratio:1/1;height:100px"),
        &[
            ("o", [8.0, 8.0, 100.0, 100.0]),
            ("i", [8.0, 8.0, 100.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("", "aspect-ratio:2/1;height:100px"),
        &[
            ("o", [8.0, 8.0, 200.0, 100.0]),
            ("i", [8.0, 8.0, 200.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("", "aspect-ratio:1/1;height:100px;align-self:flex-start"),
        &[
            ("o", [8.0, 8.0, 100.0, 100.0]),
            ("i", [8.0, 8.0, 100.0, 100.0]),
        ],
    );
}

#[test]
fn the_height_bounds_of_the_image_bound_its_width_too() {
    assert_boxes(
        &column("", "height:100px;max-height:60px"),
        &[("o", [8.0, 8.0, 24.0, 60.0]), ("i", [8.0, 8.0, 24.0, 60.0])],
    );
    assert_boxes(
        &column("", "min-height:150px"),
        &[
            ("o", [8.0, 8.0, 60.0, 150.0]),
            ("i", [8.0, 8.0, 60.0, 150.0]),
        ],
    );
}

#[test]
fn the_widest_image_sets_the_width_of_the_container() {
    assert_boxes(
        "<div id=o style=\"display:inline-flex;flex-direction:column\">\
         <img id=i src=\"https://flex.test/a.png\" style=\"height:100px\">\
         <img id=j src=\"https://flex.test/b.png\" style=\"height:60px\"></div>",
        &[
            ("o", [8.0, 8.0, 40.0, 160.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
            ("j", [8.0, 108.0, 40.0, 60.0]),
        ],
    );
}

#[test]
fn the_padding_of_the_image_stays_outside_the_height_its_width_follows() {
    assert_boxes(
        &column("", "height:100px;padding:5px;border:3px solid"),
        &[
            ("o", [8.0, 8.0, 56.0, 116.0]),
            ("i", [8.0, 8.0, 56.0, 116.0]),
        ],
    );
}

#[test]
fn an_image_that_grows_keeps_the_width_of_its_container_when_it_stretches() {
    assert_boxes(
        &column("height:200px", "flex-grow:1"),
        &[
            ("o", [8.0, 8.0, 20.0, 200.0]),
            ("i", [8.0, 8.0, 20.0, 200.0]),
        ],
    );
    assert_boxes(
        &column("height:200px", "aspect-ratio:1/1;flex-grow:1"),
        &[
            ("o", [8.0, 8.0, 20.0, 200.0]),
            ("i", [8.0, 8.0, 20.0, 200.0]),
        ],
    );
}

#[test]
fn an_image_that_grows_and_does_not_stretch_takes_its_width_from_the_grown_height() {
    assert_boxes(
        &column(
            "height:200px;align-items:flex-start",
            "aspect-ratio:1/1;flex-grow:1",
        ),
        &[
            ("o", [8.0, 8.0, 20.0, 200.0]),
            ("i", [8.0, 8.0, 200.0, 200.0]),
        ],
    );
}

/// The `flex-aspect-ratio-024` markup of web-platform-tests: a column container
/// that wraps takes the width of the height the image states, but the image,
/// alone on its line, takes the width of the height it grew to.
#[test]
fn a_wrapping_container_is_as_wide_as_the_stated_height_and_the_image_as_the_grown_one() {
    let image = "aspect-ratio:1/1;min-height:0;height:50px;flex:1";
    assert_boxes(
        &column("flex-wrap:wrap;height:100px", image),
        &[
            ("o", [8.0, 8.0, 50.0, 100.0]),
            ("i", [8.0, 8.0, 100.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("flex-wrap:wrap;height:200px", image),
        &[
            ("o", [8.0, 8.0, 50.0, 200.0]),
            ("i", [8.0, 8.0, 200.0, 200.0]),
        ],
    );
    // Without a wrap the image stretches to the width of the container.
    assert_boxes(
        &column("height:100px", image),
        &[
            ("o", [8.0, 8.0, 50.0, 100.0]),
            ("i", [8.0, 8.0, 50.0, 100.0]),
        ],
    );
}

#[test]
fn a_wrapping_container_follows_the_natural_ratio_of_the_grown_image() {
    assert_boxes(
        &column(
            "flex-wrap:wrap;height:100px",
            "min-height:0;height:50px;flex:1",
        ),
        &[
            ("o", [8.0, 8.0, 20.0, 100.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn a_wrapping_container_with_an_image_that_states_no_height_is_as_wide_as_the_image() {
    assert_boxes(
        &column("flex-wrap:wrap;height:100px", "aspect-ratio:1/1;flex:1"),
        &[
            ("o", [8.0, 8.0, 20.0, 100.0]),
            ("i", [8.0, 8.0, 100.0, 100.0]),
        ],
    );
}

#[test]
fn an_image_of_a_row_container_or_one_with_a_stated_width_is_unchanged() {
    assert_boxes(
        "<div id=o style=\"display:inline-flex\">\
         <img id=i src=\"https://flex.test/a.png\" style=\"height:100px\"></div>",
        &[
            ("o", [8.0, 8.0, 40.0, 100.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("", "width:100px"),
        &[
            ("o", [8.0, 8.0, 100.0, 250.0]),
            ("i", [8.0, 8.0, 100.0, 250.0]),
        ],
    );
    assert_boxes(
        &column("", ""),
        &[("o", [8.0, 8.0, 20.0, 50.0]), ("i", [8.0, 8.0, 20.0, 50.0])],
    );
}

#[test]
fn a_percentage_height_gives_no_width_while_the_height_of_the_container_follows_its_items() {
    // The percentage has no height to refer to, so each size is `auto`.
    for image in ["height:50%", "max-height:50%", "min-height:50%"] {
        assert_boxes(
            &column("", image),
            &[("o", [8.0, 8.0, 20.0, 50.0]), ("i", [8.0, 8.0, 20.0, 50.0])],
        );
    }
    assert_boxes(
        &column("", "height:50%;aspect-ratio:1/1"),
        &[("o", [8.0, 8.0, 20.0, 20.0]), ("i", [8.0, 8.0, 20.0, 20.0])],
    );
}

#[test]
fn a_percentage_height_gives_a_width_when_the_container_has_a_height_of_its_own() {
    assert_boxes(
        &column("height:200px", "height:50%"),
        &[
            ("o", [8.0, 8.0, 40.0, 200.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("height:200px", "min-height:50%"),
        &[
            ("o", [8.0, 8.0, 40.0, 200.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("height:200px", "max-height:50%;flex-grow:1"),
        &[
            ("o", [8.0, 8.0, 20.0, 200.0]),
            ("i", [8.0, 8.0, 20.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("", "height:calc(50px + 50px)"),
        &[
            ("o", [8.0, 8.0, 40.0, 100.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn an_image_of_a_column_container_in_a_vertical_writing_mode_is_unchanged() {
    assert_boxes(
        &column("writing-mode:vertical-rl", "height:100px"),
        &[
            ("o", [8.0, 8.0, 40.0, 100.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn a_width_bound_of_the_image_carries_to_its_height_through_the_ratio() {
    assert_boxes(
        &column("", "max-width:10px"),
        &[("o", [8.0, 8.0, 10.0, 25.0]), ("i", [8.0, 8.0, 10.0, 25.0])],
    );
    assert_boxes(
        &column("", "min-width:50px"),
        &[
            ("o", [8.0, 8.0, 50.0, 125.0]),
            ("i", [8.0, 8.0, 50.0, 125.0]),
        ],
    );
    assert_boxes(
        &column("", "max-height:25px"),
        &[("o", [8.0, 8.0, 10.0, 25.0]), ("i", [8.0, 8.0, 10.0, 25.0])],
    );
    assert_boxes(
        &column("", "aspect-ratio:1/1;max-width:10px"),
        &[("o", [8.0, 8.0, 10.0, 10.0]), ("i", [8.0, 8.0, 10.0, 10.0])],
    );
    assert_boxes(
        &column(
            "height:100px",
            "aspect-ratio:1/1;min-width:60px;height:80px",
        ),
        &[
            ("o", [8.0, 8.0, 80.0, 100.0]),
            ("i", [8.0, 8.0, 80.0, 80.0]),
        ],
    );
    assert_boxes(
        &column("height:300px", "max-width:10px;flex-grow:1"),
        &[
            ("o", [8.0, 8.0, 10.0, 300.0]),
            ("i", [8.0, 8.0, 10.0, 300.0]),
        ],
    );
}

#[test]
fn a_shrunk_image_or_one_in_a_reversed_container_takes_its_width_from_the_height_it_states() {
    assert_boxes(
        &column("height:60px", "aspect-ratio:1/1;height:100px"),
        &[
            ("o", [8.0, 8.0, 100.0, 60.0]),
            ("i", [8.0, 8.0, 100.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("flex-direction:column-reverse", "height:100px"),
        &[
            ("o", [8.0, 8.0, 40.0, 100.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn a_percentage_height_refers_to_the_height_of_the_container_that_wraps_or_grows_it() {
    assert_boxes(
        &column("height:300px;flex-wrap:wrap", "height:50%"),
        &[
            ("o", [8.0, 8.0, 60.0, 300.0]),
            ("i", [8.0, 8.0, 60.0, 150.0]),
        ],
    );
    assert_boxes(
        &column("height:300px", "height:50%;flex-grow:1"),
        &[
            ("o", [8.0, 8.0, 60.0, 300.0]),
            ("i", [8.0, 8.0, 60.0, 300.0]),
        ],
    );
    assert_boxes(
        "<div id=o style=\"display:inline-flex;flex-direction:column;height:300px\">\
         <div id=m style=\"display:flex;flex-direction:column;height:200px\">\
         <img id=i src=\"https://flex.test/a.png\" style=\"height:50%\"></div></div>",
        &[
            ("o", [8.0, 8.0, 40.0, 300.0]),
            ("m", [8.0, 8.0, 40.0, 200.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn a_stretched_height_gives_the_width_along_the_ratio_of_the_container_height() {
    // `height: stretch` states the height of the container, 200px here, so the
    // image is 80px wide along its natural ratio, and so is the container.
    assert_boxes(
        &column("height:200px", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
    assert_boxes(
        &column("height:200px", "height:stretch;aspect-ratio:1/1"),
        &[
            ("o", [8.0, 8.0, 200.0, 200.0]),
            ("i", [8.0, 8.0, 200.0, 200.0]),
        ],
    );
    assert_boxes(
        &column("height:200px;align-items:flex-start", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_stretched_height_is_kept_within_the_height_bounds_of_the_image() {
    assert_boxes(
        &column("height:200px", "height:stretch;max-height:100px"),
        &[
            ("o", [8.0, 8.0, 40.0, 200.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn a_stretched_height_needs_a_height_of_the_container_to_stretch_to() {
    assert_boxes(
        &column("", "height:stretch"),
        &[("o", [8.0, 8.0, 20.0, 50.0]), ("i", [8.0, 8.0, 20.0, 50.0])],
    );
}

#[test]
fn an_image_that_does_not_state_a_stretched_height_keeps_the_width_it_had() {
    assert_boxes(
        &column("height:200px", ""),
        &[
            ("o", [8.0, 8.0, 20.0, 200.0]),
            ("i", [8.0, 8.0, 20.0, 50.0]),
        ],
    );
    assert_boxes(
        &column("height:200px", "height:100px"),
        &[
            ("o", [8.0, 8.0, 40.0, 200.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("height:200px", "flex:1 1 0px"),
        &[
            ("o", [8.0, 8.0, 20.0, 200.0]),
            ("i", [8.0, 8.0, 20.0, 200.0]),
        ],
    );
}

#[test]
fn the_padding_and_margin_of_a_stretched_image_stay_inside_the_height_it_fills() {
    // The image fills the height of the container with its margin box, and the
    // padding is part of its border box whatever `box-sizing` is.
    for image_css in [
        "height:stretch;padding:10px",
        "height:stretch;padding:10px;box-sizing:border-box",
    ] {
        assert_boxes(
            &column("height:200px", image_css),
            &[
                ("o", [8.0, 8.0, 92.0, 200.0]),
                ("i", [8.0, 8.0, 92.0, 200.0]),
            ],
        );
    }
    assert_boxes(
        &column("height:200px", "height:stretch;margin:5px"),
        &[
            ("o", [8.0, 8.0, 86.0, 200.0]),
            ("i", [13.0, 13.0, 76.0, 190.0]),
        ],
    );
}

#[test]
fn a_stretched_image_that_grows_or_wraps_is_as_wide_as_the_container_height_gives() {
    assert_boxes(
        &column("height:200px", "height:stretch;flex-grow:1"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
    assert_boxes(
        &column("height:200px;flex-wrap:wrap", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_container_whose_height_follows_its_items_leaves_a_stretched_image_at_its_natural_size() {
    // Without a height of its own to stretch to, `height: stretch` is `auto`,
    // whether a sibling makes the container taller or a bound limits it.
    assert_boxes(
        "<div id=o style=\"display:inline-flex;flex-direction:column\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"height:stretch\">\
         <div id=j style=\"height:100px;width:10px\"></div></div>",
        &[
            ("o", [8.0, 8.0, 20.0, 150.0]),
            ("i", [8.0, 8.0, 20.0, 50.0]),
            ("j", [8.0, 58.0, 10.0, 100.0]),
        ],
    );
    assert_boxes(
        &column("max-height:100px", "height:stretch"),
        &[("o", [8.0, 8.0, 20.0, 50.0]), ("i", [8.0, 8.0, 20.0, 50.0])],
    );
    assert_boxes(
        &column("min-height:100px", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 20.0, 100.0]),
            ("i", [8.0, 8.0, 20.0, 50.0]),
        ],
    );
}

/// A block-level column flex container of the given size with one image.
fn block_column(width: u32, height: u32, container_css: &str, image_css: &str) -> String {
    format!(
        "<div id=o style=\"display:flex;flex-direction:column;width:{width}px;height:{height}px;{container_css}\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"{image_css}\"></div>"
    )
}

#[test]
fn an_image_stretched_to_the_width_of_a_block_column_takes_its_height_from_that_width() {
    // The image is 20x50, so a width of 300px along its ratio is 750px high,
    // more than the 200px of the container, and its automatic minimum height
    // keeps it from shrinking to fit. The page is tall enough to hold it.
    let tall = 3000;
    for image_css in ["flex-grow:1", "", "flex:1 1 0px"] {
        assert_boxes_on_page(
            &block_column(300, 200, "", image_css),
            tall,
            &[
                ("o", [8.0, 8.0, 300.0, 200.0]),
                ("i", [8.0, 8.0, 300.0, 750.0]),
            ],
        );
    }
    assert_boxes_on_page(
        &block_column(100, 200, "", "flex-grow:1"),
        tall,
        &[("i", [8.0, 8.0, 100.0, 250.0])],
    );
    assert_boxes_on_page(
        &block_column(200, 100, "", "flex-grow:1"),
        tall,
        &[("i", [8.0, 8.0, 200.0, 500.0])],
    );
    assert_boxes_on_page(
        &block_column(300, 200, "", "flex-grow:1;aspect-ratio:1/1"),
        tall,
        &[("i", [8.0, 8.0, 300.0, 300.0])],
    );
}

#[test]
fn an_image_that_grows_beyond_its_stretched_height_or_does_not_stretch() {
    let tall = 3000;
    // A container taller than the image makes it grow to the container.
    assert_boxes_on_page(
        &block_column(300, 1000, "", "flex-grow:1"),
        tall,
        &[
            ("o", [8.0, 8.0, 300.0, 1000.0]),
            ("i", [8.0, 8.0, 300.0, 1000.0]),
        ],
    );
    // One that is shorter than the image does not shrink it, whatever its grow.
    assert_boxes_on_page(
        &block_column(300, 600, "", "flex-grow:1"),
        tall,
        &[
            ("o", [8.0, 8.0, 300.0, 600.0]),
            ("i", [8.0, 8.0, 300.0, 750.0]),
        ],
    );
    // Without a stretch the image keeps its width and grows in height only.
    assert_boxes_on_page(
        &block_column(300, 200, "align-items:flex-start", "flex-grow:1"),
        tall,
        &[("i", [8.0, 8.0, 80.0, 200.0])],
    );
    // A stated height is the flex basis, and the grow takes the image to 200px.
    assert_boxes_on_page(
        &block_column(300, 200, "", "flex-grow:1;height:50px"),
        tall,
        &[("i", [8.0, 8.0, 300.0, 200.0])],
    );
    // In a row the container height is what the image stretches to.
    assert_boxes_on_page(
        "<div id=o style=\"display:flex;width:300px;height:200px\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"flex-grow:1\"></div>",
        tall,
        &[("i", [8.0, 8.0, 300.0, 200.0])],
    );
}

#[test]
fn two_images_that_cannot_shrink_follow_each_other_at_their_full_height() {
    assert_boxes_on_page(
        "<div id=o style=\"display:flex;flex-direction:column;width:300px;height:200px\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"flex-grow:1\">\
         <img id=j src=\"https://flex.test/portrait.png\" style=\"flex-grow:1\"></div>",
        3000,
        &[
            ("i", [8.0, 8.0, 300.0, 750.0]),
            ("j", [8.0, 758.0, 300.0, 750.0]),
        ],
    );
}

/// An inline column flex container with the given `container_css`, holding two
/// images `i` and `j` with the given `image_css`.
fn column_of_two(container_css: &str, image_css: &str) -> String {
    format!(
        "<div id=o style=\"display:inline-flex;flex-direction:column;{container_css}\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"{image_css}\">\
         <img id=j src=\"https://flex.test/portrait.png\" style=\"{image_css}\"></div>"
    )
}

#[test]
fn stretched_images_that_overflow_their_container_keep_the_width_its_height_gives() {
    // Each image stretches to the 200px of the container, so the container is
    // 80px wide whatever the container flexes the images to. The images are
    // 80px wide and 200px high, so the second one overflows.
    assert_boxes(
        &column_of_two("height:200px", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [8.0, 208.0, 80.0, 200.0]),
        ],
    );
    // A container that is 80px wide already lays the same images out alike.
    assert_boxes(
        &column_of_two("height:200px;width:80px", "height:stretch"),
        &[
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [8.0, 208.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_flex_basis_does_not_shrink_the_stretched_width_of_the_container() {
    assert_boxes(
        &column("height:200px", "height:stretch;flex-basis:50px"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
    assert_boxes(
        &column_of_two("height:200px", "height:stretch;flex-grow:1"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [8.0, 208.0, 80.0, 200.0]),
        ],
    );
    assert_boxes(
        &column_of_two("height:200px;flex-wrap:wrap", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 160.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [88.0, 8.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_stretched_image_beside_an_item_with_a_stated_height_is_not_shrunk_by_it() {
    assert_boxes(
        "<div id=o style=\"display:inline-flex;flex-direction:column;height:200px\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"height:stretch\">\
         <img id=j src=\"https://flex.test/portrait.png\" style=\"height:100px\"></div>",
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [8.0, 208.0, 80.0, 100.0]),
        ],
    );
}

#[test]
fn the_margins_padding_and_border_of_stretched_images_stay_inside_the_height_they_fill() {
    // 200px less the 10px margins above and below.
    assert_boxes(
        &column_of_two("height:200px", "height:stretch;margin:10px 0"),
        &[
            ("o", [8.0, 8.0, 72.0, 200.0]),
            ("i", [8.0, 18.0, 72.0, 180.0]),
            ("j", [8.0, 218.0, 72.0, 180.0]),
        ],
    );
    assert_boxes(
        &column_of_two("height:200px", "height:stretch;padding:5px"),
        &[
            ("o", [8.0, 8.0, 86.0, 200.0]),
            ("i", [8.0, 8.0, 86.0, 200.0]),
            ("j", [8.0, 208.0, 86.0, 200.0]),
        ],
    );
    // 200px less the 10px border of the container above and below: 180px.
    assert_boxes(
        &column_of_two("height:200px;border:solid 10px green", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 92.0, 200.0]),
            ("i", [18.0, 18.0, 72.0, 180.0]),
            ("j", [18.0, 198.0, 72.0, 180.0]),
        ],
    );
}

#[test]
fn a_height_bound_of_stretched_images_bounds_the_width_of_the_container() {
    assert_boxes(
        &column_of_two("height:200px", "height:stretch;max-height:100px"),
        &[
            ("o", [8.0, 8.0, 40.0, 200.0]),
            ("i", [8.0, 8.0, 40.0, 100.0]),
            ("j", [8.0, 108.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn images_that_are_not_stretched_to_the_container_keep_the_height_they_are_flexed_to() {
    // The images fit their content when they do not stretch, so they are 40px
    // wide at the 100px each of them gets, in a container 80px wide.
    assert_boxes(
        &column_of_two("height:200px;align-items:center", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [28.0, 8.0, 40.0, 100.0]),
            ("j", [28.0, 108.0, 40.0, 100.0]),
        ],
    );
}

#[test]
fn a_container_without_a_definite_height_leaves_stretched_images_at_their_natural_size() {
    assert_boxes(
        &column_of_two("", "height:stretch"),
        &[
            ("o", [8.0, 8.0, 20.0, 100.0]),
            ("i", [8.0, 8.0, 20.0, 50.0]),
            ("j", [8.0, 58.0, 20.0, 50.0]),
        ],
    );
}

#[test]
fn the_container_height_a_percentage_gives_is_what_stretched_images_fill() {
    assert_boxes(
        &format!(
            "<div style=\"height:400px\">{}</div>",
            column_of_two("height:50%", "height:stretch")
        ),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [8.0, 208.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn stretched_images_of_a_row_container_are_unchanged() {
    assert_boxes(
        "<div id=o style=\"display:inline-flex;height:200px\">\
         <img id=i src=\"https://flex.test/portrait.png\" style=\"height:stretch\">\
         <img id=j src=\"https://flex.test/portrait.png\" style=\"height:stretch\"></div>",
        &[
            ("o", [8.0, 8.0, 160.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [88.0, 8.0, 80.0, 200.0]),
        ],
    );
}
