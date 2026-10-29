use super::*;
use raikiri_style::{build_rule_tree, cascade};

fn fixture(html_style: &str, body_style: &str) -> (Document, CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some(html_style));
    let body = document.append_element(Some(html), "body", Some(body_style));
    document.append_text(body, "hi");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    (document, cascade)
}

fn canvas_use(html_style: &str) -> CanvasBoxUse {
    let (document, cascade) = fixture(html_style, "");
    canvas_box_use(&document, &cascade)
}

/// An image of a size of its own, tiled from the top left of the root's box.
const TILE: &str = "background-image: url(a.png); background-size: 20px 30px;";

#[test]
fn a_canvas_without_images_takes_nothing_from_the_root_box() {
    assert_eq!(canvas_use(""), CanvasBoxUse::None);
    assert_eq!(canvas_use("background-color: red"), CanvasBoxUse::None);
}

#[test]
fn an_image_that_fills_the_root_box_takes_its_extent() {
    for style in [
        "background-image: linear-gradient(red, blue)",
        "background-image: url(a.png)",
        "background-image: url(a.png); background-size: cover",
        "background-image: url(a.png); background-size: contain",
        "background-image: url(a.png); background-size: 20px 50%",
        "background-image: url(a.png); background-size: 20px auto",
        "background-image: url(a.png); background-size: 20px",
    ] {
        assert_eq!(canvas_use(style), CanvasBoxUse::Extent, "{style}");
    }
}

#[test]
fn tiles_of_their_own_height_from_the_top_take_only_the_start() {
    for style in [
        TILE.to_owned(),
        format!("{TILE} background-repeat: no-repeat"),
        format!("{TILE} background-repeat: repeat-x"),
        format!("{TILE} background-position: 10px 0"),
        format!("{TILE} background-position: 10px 12px"),
        format!("{TILE} background-position: 50% 0%"),
    ] {
        assert_eq!(canvas_use(&style), CanvasBoxUse::Start, "{style}");
    }
}

#[test]
fn tiles_placed_or_spread_by_the_extent_take_it() {
    for style in [
        format!("{TILE} background-position: 0 50%"),
        format!("{TILE} background-position: 0 100%"),
        format!("{TILE} background-position: 0 bottom"),
        format!("{TILE} background-position: left 0 bottom 10px"),
        format!("{TILE} background-repeat: repeat space"),
        format!("{TILE} background-repeat: round"),
    ] {
        assert_eq!(canvas_use(&style), CanvasBoxUse::Extent, "{style}");
    }
}

#[test]
fn an_image_fixed_to_the_page_area_takes_nothing() {
    assert_eq!(
        canvas_use("background-image: linear-gradient(red, blue); background-attachment: fixed"),
        CanvasBoxUse::None
    );
}

#[test]
fn the_layers_take_the_most_any_of_them_does() {
    let fixed = "linear-gradient(red, blue)";
    let style = |attachments: &str, images: &str, sizes: &str| {
        format!(
            "background-image: {images}; background-attachment: {attachments}; \
             background-size: {sizes}"
        )
    };
    assert_eq!(
        canvas_use(&style(
            "fixed, scroll",
            &format!("{fixed}, url(a.png)"),
            "auto, 20px 30px"
        )),
        CanvasBoxUse::Start
    );
    assert_eq!(
        canvas_use(&style(
            "scroll, scroll",
            &format!("{fixed}, url(a.png)"),
            "auto, 20px 30px"
        )),
        CanvasBoxUse::Extent
    );
}

#[test]
fn pages_that_run_across_take_the_extent_of_every_image() {
    assert_eq!(
        canvas_use(&format!("{TILE} display: block; writing-mode: vertical-rl")),
        CanvasBoxUse::Extent
    );
}

#[test]
fn the_body_gives_the_canvas_its_background_when_the_root_has_none() {
    let (document, cascade) = fixture("", &format!("{TILE} background-color: red"));
    let body = canvas_background_owner(&document, &cascade).expect("an owner");
    assert_eq!(document.get_node(body).unwrap().tag_name(), Some("body"));
    assert_eq!(canvas_box_use(&document, &cascade), CanvasBoxUse::Start);

    let (document, cascade) = fixture("", "background-image: linear-gradient(red, blue)");
    assert_eq!(canvas_box_use(&document, &cascade), CanvasBoxUse::Extent);
}

#[test]
fn a_root_with_a_background_keeps_it_from_the_body() {
    let (document, cascade) = fixture(
        "background-color: red",
        "background-image: linear-gradient(red, blue)",
    );
    let owner = canvas_background_owner(&document, &cascade).expect("an owner");
    assert_eq!(document.get_node(owner).unwrap().tag_name(), Some("html"));
    assert_eq!(canvas_box_use(&document, &cascade), CanvasBoxUse::None);
}

#[test]
fn a_document_without_backgrounds_has_no_owner() {
    let (document, cascade) = fixture("", "");
    assert_eq!(canvas_background_owner(&document, &cascade), None);
    assert_eq!(canvas_box_use(&document, &cascade), CanvasBoxUse::None);

    let empty = Document::new();
    let rules = build_rule_tree(&empty);
    let cascade = raikiri_style::cascade(&empty, &rules).expect("cascade Ok");
    assert_eq!(canvas_background_owner(&empty, &cascade), None);
    assert_eq!(canvas_box_use(&empty, &cascade), CanvasBoxUse::None);
}
