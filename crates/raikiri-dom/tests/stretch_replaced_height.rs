//! An image that states `height: stretch` in a block container whose height is
//! definite fills the content height of that container, and its `auto` width
//! follows from that height along its aspect ratio (CSS Sizing 4 §5.1). The
//! container that shrinks to fit its content, an inline-block, a float, an
//! absolutely positioned box or a flow-root of `min-content` width, measures
//! that width as well.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport with the 8px body margin, with every
//! image 20x50 pixels, a 2:5 aspect ratio.

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

/// Every image is 20x50 pixels.
struct Portrait;

impl ReplacedResolver for Portrait {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(20.0, 50.0),
            ResolveDisposition::Ok,
        ))
    }
}

/// A border box `(x, y, width, height)`.
type Rect = [f32; 4];

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    border_boxes_with_device_scale(body, None)
}

fn border_boxes_with_device_scale(body: &str, scale: Option<f32>) -> Vec<(String, Rect)> {
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
        .border_width_device_scale(scale)
        .base_url(Url::parse("https://stretch.test/document.html").expect("url"))
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

/// Check that each element of `expected` is laid out with the given border box.
fn check(body: &str, expected: &[(&str, Rect)]) {
    let laid_out = border_boxes(body);
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

/// `container_css` around one image with `item_css`, which states
/// `height: stretch` itself.
fn image(container_css: &str, item_css: &str) -> String {
    format!(
        "<div id=o style=\"{container_css}\">\
         <img id=i src=\"https://stretch.test/portrait.png\" style=\"height:stretch;{item_css}\">\
         </div>"
    )
}

#[test]
fn the_width_follows_the_stretched_height_along_the_ratio() {
    // The 200px of the container give 80px of width, not the natural 20px.
    check(
        &image("height:200px", ""),
        &[
            ("o", [8.0, 8.0, 784.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn the_height_a_percentage_gives_is_filled_too() {
    check(
        &format!(
            "<div style=\"height:400px\">{}</div>",
            image("height:50%", "")
        ),
        &[
            ("o", [8.0, 8.0, 784.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_container_without_a_definite_height_leaves_the_natural_size() {
    // `stretch` has nothing to fill, so the image is as it would be with
    // `auto`; an unresolved percentage height is not definite either.
    check(
        &image("", ""),
        &[
            ("o", [8.0, 8.0, 784.0, 52.0]),
            ("i", [8.0, 8.0, 20.0, 50.0]),
        ],
    );
    check(
        &image("height:50%", ""),
        &[
            ("o", [8.0, 8.0, 784.0, 52.0]),
            ("i", [8.0, 8.0, 20.0, 50.0]),
        ],
    );
}

#[test]
fn the_border_of_the_container_is_not_filled() {
    // 200px less the 10px borders on both sides: 180px high, 72px wide.
    check(
        &image("height:200px;border:solid 10px green", ""),
        &[("i", [18.0, 18.0, 72.0, 180.0])],
    );
}

#[test]
fn the_margins_and_the_padding_of_the_image_are_inside_the_stretched_height() {
    check(
        &image("height:200px", "margin:5px"),
        &[("i", [13.0, 13.0, 76.0, 190.0])],
    );
    check(
        &image("height:200px", "padding:5px"),
        &[("i", [8.0, 8.0, 86.0, 200.0])],
    );
    check(
        &image("height:200px", "border:solid 5px red"),
        &[("i", [8.0, 8.0, 86.0, 200.0])],
    );
}

#[test]
fn the_limits_of_the_image_still_apply() {
    // A maximum height clamps the stretched height, and the width follows the
    // clamped one.
    check(
        &image("height:200px", "max-height:100px"),
        &[("i", [8.0, 8.0, 40.0, 100.0])],
    );
    // A width limit does not move the stretched height.
    check(
        &image("height:200px", "max-width:30px"),
        &[("i", [8.0, 8.0, 30.0, 200.0])],
    );
    check(
        &image("height:200px", "min-width:100px"),
        &[("i", [8.0, 8.0, 100.0, 200.0])],
    );
    check(
        &image("height:200px;width:100px", "max-width:50%"),
        &[("i", [8.0, 8.0, 50.0, 200.0])],
    );
}

#[test]
fn an_aspect_ratio_the_image_states_is_the_one_that_gives_the_width() {
    check(
        &image("height:200px", "aspect-ratio:1/1"),
        &[("i", [8.0, 8.0, 200.0, 200.0])],
    );
}

#[test]
fn an_image_in_a_line_of_text_is_stretched_too() {
    check(
        "<div id=o style=\"height:200px\">abc\
         <img id=i src=\"https://stretch.test/portrait.png\" style=\"height:stretch\">def</div>",
        &[
            ("o", [8.0, 8.0, 784.0, 200.0]),
            ("i", [38.0, 8.0, 80.0, 200.0]),
        ],
    );
    check(
        "<div id=o style=\"height:200px\">\
         <img id=i src=\"https://stretch.test/portrait.png\" style=\"height:stretch\">\
         <img id=j src=\"https://stretch.test/portrait.png\" style=\"height:stretch\"></div>",
        &[
            ("i", [8.0, 8.0, 80.0, 200.0]),
            ("j", [88.0, 8.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_container_that_shrinks_to_fit_is_as_wide_as_the_stretched_image() {
    for container in [
        "display:inline-block;height:200px",
        "float:left;height:200px",
        "display:flow-root;height:200px;width:min-content",
    ] {
        check(
            &image(container, ""),
            &[
                ("o", [8.0, 8.0, 80.0, 200.0]),
                ("i", [8.0, 8.0, 80.0, 200.0]),
            ],
        );
    }
    check(
        &image("position:absolute;left:0;top:0;height:200px", ""),
        &[
            ("o", [0.0, 0.0, 80.0, 200.0]),
            ("i", [0.0, 0.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_container_that_shrinks_to_fit_follows_the_limits_of_the_image() {
    for container in [
        "display:inline-block;height:200px",
        "float:left;height:200px",
    ] {
        check(
            &image(container, "display:block;max-height:100px"),
            &[
                ("o", [8.0, 8.0, 40.0, 200.0]),
                ("i", [8.0, 8.0, 40.0, 100.0]),
            ],
        );
    }
    check(
        &image(
            "display:inline-block;height:200px",
            "display:block;min-width:100px",
        ),
        &[
            ("o", [8.0, 8.0, 100.0, 200.0]),
            ("i", [8.0, 8.0, 100.0, 200.0]),
        ],
    );
    check(
        &image(
            "display:inline-block;height:200px",
            "display:block;padding:5px",
        ),
        &[
            ("o", [8.0, 8.0, 86.0, 200.0]),
            ("i", [8.0, 8.0, 86.0, 200.0]),
        ],
    );
}

#[test]
fn a_container_that_shrinks_to_fit_leaves_the_margins_of_the_image_inside_the_height() {
    // 200px less the 10px margins above and below: 180px, 72px wide, and 5px of
    // margin on either side of it.
    check(
        &image(
            "display:inline-block;height:200px",
            "display:block;margin:10px 5px",
        ),
        &[
            ("o", [8.0, 8.0, 82.0, 200.0]),
            ("i", [13.0, 18.0, 72.0, 180.0]),
        ],
    );
    check(
        &image("display:inline-block;height:200px", "margin:10px 0"),
        &[
            ("o", [8.0, 8.0, 72.0, 200.0]),
            ("i", [8.0, 18.0, 72.0, 180.0]),
        ],
    );
}

#[test]
fn a_block_image_in_a_container_that_shrinks_to_fit_is_stretched() {
    check(
        &image("display:inline-block;height:200px", "display:block"),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
    check(
        &format!(
            "<div style=\"height:300px\">{}</div>",
            image("display:inline-block;height:50%", "display:block")
        ),
        &[
            ("o", [8.0, 8.0, 60.0, 150.0]),
            ("i", [8.0, 8.0, 60.0, 150.0]),
        ],
    );
}

#[test]
fn a_container_that_shrinks_to_fit_without_a_height_stays_as_wide_as_the_image() {
    check(
        &image("display:inline-block", "display:block"),
        &[("o", [8.0, 8.0, 20.0, 50.0]), ("i", [8.0, 8.0, 20.0, 50.0])],
    );
}

#[test]
fn the_other_kinds_of_container_stretch_the_image_as_well() {
    check(
        &image("display:grid;height:200px", ""),
        &[("i", [8.0, 8.0, 80.0, 200.0])],
    );
    check(
        &image("display:inline-flex;height:200px", ""),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
    check(
        &image("display:inline-flex;flex-direction:column;height:200px", ""),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 8.0, 80.0, 200.0]),
        ],
    );
}

#[test]
fn a_stretched_image_in_a_table_cell_is_aligned_at_its_used_height() {
    let body = "<table id=t style='height:200px;border-spacing:0'><tr><td id=c>\
        <img id=i src='https://stretch.test/portrait.png' style='height:stretch'>\
        </td></tr></table>";
    let found = border_boxes(body);
    // Chromium quantizes the ratio width to 1/64px (79.1875px).
    for (id, expected) in [
        ("t", [8.0, 8.0, 22.0, 200.0]),
        ("c", [8.0, 8.0, 22.0, 200.0]),
        ("i", [9.0, 9.0, 79.1875, 198.0]),
    ] {
        let (_, actual) = found.iter().find(|(name, _)| name == id).expect("box");
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 0.04),
            "#{id} is {actual:?}, expected {expected:?}"
        );
    }
}

#[test]
fn table_cell_image_alignment_follows_final_constraints() {
    check_cell_image_oracle(include_str!("data/table-cell-stretch-image.json"));
}

#[test]
fn a_cell_does_not_stretch_its_height_determining_image_again() {
    check_cell_image_oracle(include_str!("data/table-cell-stretch-min-height.json"));
}

fn check_cell_image_oracle(source: &str) {
    let oracle: serde_json::Value = serde_json::from_str(source).expect("Chromium measurements");
    let mut mismatches = Vec::new();
    for case in oracle["cases"].as_array().expect("cases") {
        let body = case["html"].as_str().expect("markup");
        let scale = case["device_scale"].as_f64().map(|value| value as f32);
        let boxes = border_boxes_with_device_scale(body, scale);
        for entry in case["expected"].as_array().expect("expected boxes") {
            let id = entry[0].as_str().expect("id");
            let (_, actual) = boxes.iter().find(|(name, _)| name == id).expect("box");
            let expected: Vec<_> = (1..5)
                .map(|i| entry[i].as_f64().expect("coordinate") as f32)
                .collect();
            if !actual
                .iter()
                .zip(&expected)
                .all(|(actual, expected)| (actual - expected).abs() < 0.04)
            {
                mismatches.push(format!(
                    "{}: #{id} is {actual:?}, expected {expected:?}",
                    case["label"]
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
