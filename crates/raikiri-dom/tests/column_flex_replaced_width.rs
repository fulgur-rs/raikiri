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

/// Check that every `(id, expected box)` of `expected` is laid out as given.
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let laid_out = boxes(body);
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
