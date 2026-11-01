//! An image that states `height: stretch` in a column flex container whose
//! height is definite fills that height less its margins, whatever its width
//! gives it along its aspect ratio: a stretch height is the size the item
//! states, so its automatic minimum size is the smaller of that and its content
//! size (CSS Flexbox 1 §4.5), and the item does not grow past it to the height
//! its width asks for.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport with the 8px body margin, with every
//! image 20x50 pixels, a 2:5 aspect ratio. Layout units are 1/64px, so a margin
//! that Chromium rounds (8.19 for 8.2) differs a little.

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

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// Check that each element of `expected` is laid out with the given border box.
fn check(label: &str, body: &str, expected: &[(&str, Rect)]) {
    let laid_out = border_boxes(body);
    for (id, want) in expected {
        let got = laid_out
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("{label}: no box with id {id} in {body}"))
            .1;
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < TOLERANCE),
            "{label}: #{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// An image `id` that states `height: stretch` and `css`.
fn img(id: &str, css: &str) -> String {
    format!(
        "<img id={id} src=\"https://stretch.test/portrait.png\" style=\"height:stretch;{css}\">"
    )
}

/// `container_css` around `items`.
fn column(container_css: &str, items: &str) -> String {
    format!("<div id=o style=\"{container_css}\">{items}</div>")
}

/// A column flex container that is as wide as its image asks and 200px high.
const SHRINK_WRAPPED: &str = "display: inline-flex; flex-direction: column; height: 200px";

/// A column flex container that is wider than its image asks and 200px high.
const WIDE: &str = "display: flex; flex-direction: column; height: 200px; width: 300px";

#[test]
fn a_percentage_vertical_margin_comes_off_the_stretch_height() {
    // 10% of the container's 80px is 8px on each side: 200 - 16.
    check(
        "both sides",
        &column(SHRINK_WRAPPED, &img("i", "margin: 10% 0")),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 16.0, 80.0, 184.0]),
        ],
    );
    check(
        "top only",
        &column(SHRINK_WRAPPED, &img("i", "margin-top: 10%")),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 16.0, 80.0, 192.0]),
        ],
    );
    // The same container with a width of its own.
    check(
        "stated width",
        &column(
            "display: flex; flex-direction: column; height: 200px; width: 80px",
            &img("i", "margin: 10% 0"),
        ),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 16.0, 80.0, 184.0]),
        ],
    );
    // A container that lays its items out in reverse, or in lines.
    check(
        "reverse",
        &column(
            "display: inline-flex; flex-direction: column-reverse; height: 200px",
            &img("i", "margin: 10% 0"),
        ),
        &[("i", [8.0, 16.0, 80.0, 184.0])],
    );
    check(
        "wrapping",
        &column(
            &format!("{SHRINK_WRAPPED}; flex-wrap: wrap"),
            &img("i", "margin: 10% 0"),
        ),
        &[("i", [8.0, 16.0, 80.0, 184.0])],
    );
}

#[test]
fn a_percentage_margin_on_the_sides_too_still_comes_off() {
    // The width of the item shrinks by the side margins, so its ratio height is
    // below the stretch height.
    check(
        "left and vertical",
        &column(SHRINK_WRAPPED, &img("i", "margin: 10% 0 10% 10%")),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [16.0, 16.0, 72.0, 184.0]),
        ],
    );
    check(
        "all sides",
        &column(SHRINK_WRAPPED, &img("i", "margin: 10%")),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [16.0, 16.0, 64.0, 184.0]),
        ],
    );
}

#[test]
fn a_wide_container_does_not_raise_the_stretch_height() {
    // The item is as wide as the 300px container, and 300px along the ratio is
    // 750px high; its automatic minimum size is the stretch height, 200 - 60.
    check(
        "300px wide",
        &column(WIDE, &img("i", "margin: 10% 0")),
        &[
            ("o", [8.0, 8.0, 300.0, 200.0]),
            ("i", [8.0, 38.0, 300.0, 140.0]),
        ],
    );
    check(
        "100px wide",
        &column(
            "display: flex; flex-direction: column; height: 200px; width: 100px",
            &img("i", "margin: 10% 0"),
        ),
        &[
            ("o", [8.0, 8.0, 100.0, 200.0]),
            ("i", [8.0, 18.0, 100.0, 180.0]),
        ],
    );
    // Two images with no margin, as wide as the container.
    check(
        "two without margins",
        &column(
            "display: flex; flex-direction: column; height: 200px; width: 100px",
            &format!("{}{}", img("i", ""), img("j", "")),
        ),
        &[
            ("i", [8.0, 8.0, 100.0, 200.0]),
            ("j", [8.0, 208.0, 100.0, 200.0]),
        ],
    );
    // Neither shrinking nor a basis of its own changes it.
    check(
        "no shrinking",
        &column(WIDE, &img("i", "margin: 10% 0; flex-shrink: 0")),
        &[("i", [8.0, 38.0, 300.0, 140.0])],
    );
    check(
        "a basis",
        &column(WIDE, &img("i", "margin: 10% 0; flex-basis: 50px")),
        &[("i", [8.0, 38.0, 300.0, 140.0])],
    );
    check(
        "not stretched across",
        &column(
            "display: flex; flex-direction: column; align-items: flex-start; height: 200px; \
             width: 300px",
            &img("i", "margin: 10% 0"),
        ),
        &[("i", [8.0, 38.0, 56.0, 140.0])],
    );
}

#[test]
fn two_stretched_items_keep_their_stretch_height_and_overflow() {
    // Each is 200 - 30 high: the automatic minimum size stops them shrinking.
    let items = format!("{}{}", img("i", "margin: 5% 0"), img("j", "margin: 5% 0"));
    check(
        "two",
        &column(WIDE, &items),
        &[
            ("o", [8.0, 8.0, 300.0, 200.0]),
            ("i", [8.0, 23.0, 300.0, 170.0]),
            ("j", [8.0, 223.0, 300.0, 170.0]),
        ],
    );
    // With a minimum of zero they shrink to share the container.
    let items = format!(
        "{}{}",
        img("i", "margin: 5% 0; min-height: 0"),
        img("j", "margin: 5% 0; min-height: 0")
    );
    check(
        "two that may shrink",
        &column(WIDE, &items),
        &[
            ("i", [8.0, 23.0, 300.0, 70.0]),
            ("j", [8.0, 123.0, 300.0, 70.0]),
        ],
    );
}

#[test]
fn the_other_sizes_of_the_item_still_apply() {
    // A length margin, as before.
    check(
        "length margins",
        &column(SHRINK_WRAPPED, &img("i", "margin: 8px 0")),
        &[
            ("o", [8.0, 8.0, 73.59, 200.0]),
            ("i", [8.0, 16.0, 73.59, 184.0]),
        ],
    );
    // Padding sits inside the stretched border box.
    check(
        "padding",
        &column(SHRINK_WRAPPED, &img("i", "margin: 10% 0; padding: 5px")),
        &[
            ("o", [8.0, 8.0, 86.0, 200.0]),
            ("i", [8.0, 16.59, 86.0, 182.81]),
        ],
    );
    // A maximum clamps the stretch height; a minimum raises it.
    check(
        "max-height",
        &column(
            SHRINK_WRAPPED,
            &img("i", "margin: 10% 0; max-height: 100px"),
        ),
        &[
            ("o", [8.0, 8.0, 40.0, 200.0]),
            ("i", [8.0, 12.0, 40.0, 100.0]),
        ],
    );
    check(
        "max-height in a wide container",
        &column(WIDE, &img("i", "margin: 10% 0; max-height: 120px")),
        &[("i", [8.0, 38.0, 300.0, 120.0])],
    );
    check(
        "min-height",
        &column(WIDE, &img("i", "margin: 10% 0; min-height: 150px")),
        &[("i", [8.0, 38.0, 300.0, 150.0])],
    );
    // A content flex basis, a scroll container (its automatic minimum size is
    // zero) and a vertical writing mode: the margins are the same 8px, and the
    // item fills the room that is left.
    for (label, css) in [
        ("flex-basis: content", "flex-basis: content"),
        ("overflow", "overflow: hidden"),
        ("vertical item", "writing-mode: vertical-rl"),
    ] {
        check(
            label,
            &column(SHRINK_WRAPPED, &img("i", &format!("margin: 10% 0; {css}"))),
            &[("i", [8.0, 16.0, 80.0, 184.0])],
        );
    }
    // Margins that leave nothing of the container's height.
    check(
        "no room",
        &column(
            "display: flex; flex-direction: column; height: 60px; width: 300px",
            &img("i", "margin: 10% 0"),
        ),
        &[
            ("o", [8.0, 8.0, 300.0, 60.0]),
            ("i", [8.0, 38.0, 300.0, 0.0]),
        ],
    );
}

#[test]
fn an_inline_svg_with_only_a_ratio_is_held_to_the_stretch_height_too() {
    let svg = |css: &str| {
        format!("<svg id=i viewBox=\"0 0 20 50\" style=\"height:stretch;{css}\"></svg>")
    };
    check(
        "margin",
        &column(SHRINK_WRAPPED, &svg("margin: 10% 0")),
        &[
            ("o", [8.0, 8.0, 80.0, 200.0]),
            ("i", [8.0, 16.0, 80.0, 184.0]),
        ],
    );
    check(
        "wide container",
        &column(WIDE, &svg("margin: 10% 0")),
        &[
            ("o", [8.0, 8.0, 300.0, 200.0]),
            ("i", [8.0, 38.0, 300.0, 140.0]),
        ],
    );
    check(
        "wide container, no margin",
        &column(WIDE, &svg("")),
        &[
            ("o", [8.0, 8.0, 300.0, 200.0]),
            ("i", [8.0, 8.0, 300.0, 200.0]),
        ],
    );
}

#[test]
fn a_container_stretched_after_it_is_measured_gives_its_item_the_final_stretch_height() {
    // The row container stretches the column one to 300px after it measured
    // the column at its content height, and the image fills 300px, less its
    // margins, not the 500px its 200px width gives it along its ratio.
    let stretched = |item_css: &str| {
        format!(
            "<div style=\"display: flex; height: 300px; width: 200px\">{}</div>",
            column(
                "display: flex; flex-direction: column; width: 200px",
                &img("i", item_css)
            )
        )
    };
    check(
        "no margin",
        &stretched(""),
        &[
            ("o", [8.0, 8.0, 200.0, 300.0]),
            ("i", [8.0, 8.0, 200.0, 300.0]),
        ],
    );
    check(
        "margins",
        &stretched("margin: 10px 0"),
        &[
            ("o", [8.0, 8.0, 200.0, 300.0]),
            ("i", [8.0, 18.0, 200.0, 280.0]),
        ],
    );
}

#[test]
fn the_stretch_height_is_not_below_the_padding_and_border() {
    // The margins take all the room, so the box is left with its padding and
    // border: 5px above and below, and 4px of border above and below.
    check(
        "padding",
        &column(
            "display: inline-flex; flex-direction: column; height: 60px",
            &img("i", "margin: 100% 0; padding: 5px"),
        ),
        &[("i", [8.0, 38.0, 30.0, 10.0])],
    );
    check(
        "border",
        &column(
            "display: flex; flex-direction: column; height: 60px; width: 100px",
            &img("i", "margin: 30% 0; border: 4px solid"),
        ),
        &[("i", [8.0, 38.0, 100.0, 8.0])],
    );
}

#[test]
fn inline_svgs_with_stretch_heights_can_shrink_inside_a_column() {
    let items = "<svg id=i viewBox=\"0 0 20 50\" style=\"height:stretch\"></svg>\
                 <svg id=j viewBox=\"0 0 20 50\" style=\"height:stretch\"></svg>";
    check(
        "default SVG overflow",
        &column(
            "display:flex;flex-direction:column;height:200px;width:100px",
            items,
        ),
        &[
            ("i", [8.0, 8.0, 100.0, 100.0]),
            ("j", [8.0, 108.0, 100.0, 100.0]),
        ],
    );
}

#[test]
fn explicit_svg_overflow_controls_the_automatic_column_minimum() {
    for (overflow, height) in [
        ("hidden", 100.0),
        ("auto", 100.0),
        ("visible", 200.0),
        ("clip", 200.0),
    ] {
        let items = format!(
            "<svg id=i viewBox=\"0 0 20 50\" style=\"height:stretch;overflow:{overflow}\"></svg>\
                            <svg id=j viewBox=\"0 0 20 50\" style=\"height:stretch;overflow:{overflow}\"></svg>"
        );
        check(
            overflow,
            &column(
                "display:flex;flex-direction:column;height:200px;width:100px",
                &items,
            ),
            &[
                ("i", [8.0, 8.0, 100.0, height]),
                ("j", [8.0, 8.0 + height, 100.0, height]),
            ],
        );
    }
}

#[test]
fn svg_and_image_stretch_minima_follow_their_computed_overflow() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("data/inline-svg-stretch-overflow.json")).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        let expected = case["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry[0].as_str().unwrap(),
                    [
                        entry[1].as_f64().unwrap() as f32,
                        entry[2].as_f64().unwrap() as f32,
                        entry[3].as_f64().unwrap() as f32,
                        entry[4].as_f64().unwrap() as f32,
                    ],
                )
            })
            .collect::<Vec<_>>();
        check(
            case["label"].as_str().unwrap(),
            case["html"].as_str().unwrap(),
            &expected,
        );
    }
}

#[test]
fn svg_overflow_attributes_yield_to_author_stylesheets() {
    for attribute in ["visible", "hidden", "clip", "auto"] {
        for author in [None, Some("visible"), Some("hidden")] {
            let overflow = author.unwrap_or(attribute);
            let height = if matches!(overflow, "hidden" | "auto") {
                100.0
            } else {
                200.0
            };
            let css = author
                .map(|value| format!("<style>svg{{overflow:{value}}}</style>"))
                .unwrap_or_default();
            let items = format!(
                "<svg id=i viewBox=\"0 0 20 50\" overflow=\"{attribute}\" style=\"height:stretch\"></svg>\
                                <svg id=j viewBox=\"0 0 20 50\" overflow=\"{attribute}\" style=\"height:stretch\"></svg>"
            );
            check(
                attribute,
                &format!(
                    "{css}{}",
                    column(
                        "display:flex;flex-direction:column;height:200px;width:100px",
                        &items
                    )
                ),
                &[
                    ("i", [8.0, 8.0, 100.0, height]),
                    ("j", [8.0, 8.0 + height, 100.0, height]),
                ],
            );
        }
    }
}
