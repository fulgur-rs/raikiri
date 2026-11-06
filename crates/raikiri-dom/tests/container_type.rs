//! Container size containment and independent formatting contexts.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }}</style>{body}"
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
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

fn rect(body: &str, id: &str) -> [f32; 4] {
    boxes(body)
        .into_iter()
        .find(|(name, _)| name == id)
        .expect("named box")
        .1
}

#[test]
fn container_type_size_ignores_child_height_for_preferred_ratio() {
    for (kind, height) in [("normal", 200.0), ("inline-size", 200.0), ("size", 100.0)] {
        let body = format!(
            "<div id=b style='container-type:{kind};width:100px;aspect-ratio:1'><img height=200 style='display:block'></div>"
        );
        assert_eq!(rect(&body, "b"), [8.0, 8.0, 100.0, height], "{kind}");
    }
}

#[test]
fn container_type_size_containment_tracks_the_own_inline_axis() {
    for (style, size) in [
        ("container-type:normal", [200.0, 50.0]),
        ("container-type:size", [0.0, 0.0]),
        ("container-type:inline-size", [0.0, 50.0]),
        (
            "container-type:inline-size;writing-mode:vertical-rl",
            [200.0, 0.0],
        ),
        (
            "container-type:inline-size;writing-mode:vertical-lr",
            [200.0, 0.0],
        ),
        (
            "container-type:size;contain:none;contain-intrinsic-size:80px 30px",
            [80.0, 30.0],
        ),
    ] {
        let body = format!(
            "<div id=b style='display:inline-block;{style}'><div style='width:200px;height:50px'></div></div>"
        );
        let actual = rect(&body, "b");
        assert_eq!([actual[2], actual[3]], size, "{style}");
    }
}

#[test]
fn container_type_inline_size_contains_its_floats() {
    for (kind, height) in [("normal", 0.0), ("inline-size", 40.0), ("size", 0.0)] {
        let body = format!(
            "<div id=b style='container-type:{kind};width:100px'><div style='float:left;width:30px;height:40px'></div></div>"
        );
        assert_eq!(rect(&body, "b"), [8.0, 8.0, 100.0, height], "{kind}");
    }
}

#[test]
fn container_type_context_stays_clear_of_external_floats() {
    for (kind, x) in [("normal", 8.0), ("inline-size", 38.0), ("size", 38.0)] {
        let body = format!(
            "<div style='float:left;width:30px;height:40px'></div><div id=b style='container-type:{kind};width:100px;height:10px'></div>"
        );
        assert_eq!(rect(&body, "b"), [x, 8.0, 100.0, 10.0], "{kind}");
    }
}

#[test]
fn container_type_context_stops_parent_child_margin_collapse() {
    for (kind, y, height) in [
        ("normal", 29.0, 10.0),
        ("inline-size", 9.0, 30.0),
        ("size", 9.0, 0.0),
    ] {
        let body = format!(
            "<div style='border:1px solid transparent'><div id=b style='container-type:{kind};width:100px'><div id=c style='margin-top:20px;height:10px'></div></div></div>"
        );
        assert_eq!(rect(&body, "b"), [9.0, y, 100.0, height], "{kind}");
        assert_eq!(rect(&body, "c")[1], 29.0, "{kind} child");
    }
}

#[test]
fn container_type_does_not_create_positioned_containing_blocks() {
    for kind in ["normal", "inline-size", "size"] {
        for (position, expected) in [
            ("absolute", [8.0, 8.0, 5.0, 5.0]),
            ("fixed", [0.0, 0.0, 5.0, 5.0]),
        ] {
            let body = format!(
                "<div style='position:relative;padding:40px'><div style='container-type:{kind};width:100px;height:30px'><div id=c style='position:{position};left:0;top:0;width:5px;height:5px'></div></div></div>"
            );
            assert_eq!(rect(&body, "c"), expected, "{kind} {position}");
        }
    }
}

#[test]
fn container_type_preserves_flex_and_grid_layout_algorithms() {
    for display in ["flex", "grid"] {
        for (kind, height) in [("normal", 200.0), ("inline-size", 200.0), ("size", 0.0)] {
            let body = format!(
                "<div id=b style='display:{display};container-type:{kind};width:100px'><div id=c style='height:200px;width:30px;flex-shrink:0'></div></div>"
            );
            assert_eq!(
                rect(&body, "b"),
                [8.0, 8.0, 100.0, height],
                "{display} {kind}"
            );
            assert_eq!(
                rect(&body, "c")[2..],
                [30.0, 200.0],
                "{display} {kind} child"
            );
        }
    }
}

#[test]
fn container_type_does_not_blockify_plain_inline_or_contents_boxes() {
    for kind in ["normal", "inline-size", "size"] {
        let body = format!("A<span style='container-type:{kind}'>BC</span><span id=c>D</span>");
        assert_eq!(rect(&body, "c"), [38.0, 8.0, 10.0, 10.0], "{kind} inline");
        let body = format!(
            "<div style='display:contents;container-type:{kind}'><div style='width:30px;height:20px'></div></div><div id=c style='height:10px'></div>"
        );
        assert_eq!(rect(&body, "c")[1], 28.0, "{kind} contents");
        let body = format!(
            "<div style='display:none;container-type:{kind}'><div id=hidden style='height:20px'></div></div><div id=c style='height:10px'></div>"
        );
        assert_eq!(rect(&body, "c")[1], 8.0, "{kind} hidden");
        assert!(!boxes(&body).iter().any(|(id, _)| id == "hidden"));
    }
}

#[test]
fn container_type_does_not_size_contain_internal_table_boxes() {
    for kind in ["normal", "inline-size", "size"] {
        let body = format!(
            "<table style='border-spacing:0'><tr><td id=b style='padding:0;container-type:{kind}'><div style='width:200px;height:50px'></div></td></tr></table>"
        );
        assert_eq!(rect(&body, "b"), [8.0, 8.0, 200.0, 50.0], "{kind}");
    }
}

#[test]
fn container_type_keeps_inline_block_baselines() {
    let normal = rect(
        "<div id=b style='display:inline-block;width:100px;container-type:normal'>A</div><span id=c>B</span>",
        "c",
    );
    let contained = rect(
        "<div id=b style='display:inline-block;width:100px;container-type:inline-size'>A</div><span id=c>B</span>",
        "c",
    );
    assert_eq!(contained, normal);
}

#[test]
fn container_type_keeps_column_spans_inside_its_formatting_context() {
    let body = "<div style='width:100px;columns:2;column-gap:0'><div id=w style='CONTAINER'><div id=c style='column-span:all;height:10px'></div></div></div>";
    let expected = boxes(&body.replace("CONTAINER", "display:flow-root"));
    assert_eq!(
        expected,
        vec![
            ("w".to_owned(), [8.0, 8.0, 50.0, 5.0]),
            ("w".to_owned(), [58.0, 8.0, 50.0, 5.0]),
            ("c".to_owned(), [8.0, 8.0, 50.0, 5.0]),
            ("c".to_owned(), [58.0, 8.0, 50.0, 5.0]),
        ]
    );
    assert_eq!(
        boxes(&body.replace("CONTAINER", "container-type:inline-size")),
        expected
    );
}
