//! Computed `border-image-*` values as a painter reads them through
//! `raikiri_html`.

use raikiri_html::computed::{
    BorderImageOutsetSide, BorderImageRepeatKeyword, BorderImageSliceOffset, BorderImageWidthSide,
    ComputedBackgroundImage, ComputedBorderImage, ComputedLengthPercentage, Sides,
};
use raikiri_html::{
    LayoutOptions, LayoutStatus, NodeId, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, NodeKind, PageDefaults};

fn find_by_id(node: NodeId, dom: &raikiri_html::DomView<'_>, id: &str) -> Option<NodeId> {
    if dom.kind(node) == Some(NodeKind::Element) && dom.attr(node, "id") == Some(id) {
        return Some(node);
    }
    dom.children(node)
        .find_map(|child| find_by_id(child, dom, id))
}

/// The computed border image of each `id`, in order.
fn border_images(css: &str, body: &str, ids: &[&str]) -> Vec<ComputedBorderImage> {
    let html = format!("<!doctype html><style>{css}</style>{body}");
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(summary) = status else {
        panic!("expected a complete layout");
    };
    let page = summary.pages().next().expect("one page");
    let dom = page.dom();
    ids.iter()
        .map(|id| {
            let node = find_by_id(dom.root(), &dom, id).expect("element");
            page.computed(node).expect("computed").border_image.clone()
        })
        .collect()
}

fn url(image: &ComputedBorderImage) -> Option<&str> {
    match &image.source {
        ComputedBackgroundImage::Url(url) => Some(url.as_str()),
        _ => None,
    }
}

#[test]
fn shorthand_sets_every_longhand() {
    let [image] = &border_images(
        "html { font-size: 10px }
         #a { border-image: url(frame.png) 50% 10 fill / 2em 3 auto / 1em 2 round space }",
        "<div id=a></div>",
        &["a"],
    )[..] else {
        unreachable!()
    };
    assert_eq!(url(image), Some("frame.png"));
    assert!(image.slice.fill);
    assert_eq!(
        image.slice.offsets.top,
        BorderImageSliceOffset::Percent(50.0)
    );
    assert_eq!(
        image.slice.offsets.right,
        BorderImageSliceOffset::Number(10.0)
    );
    assert_eq!(
        image.slice.offsets.bottom,
        BorderImageSliceOffset::Percent(50.0)
    );
    assert_eq!(
        image.width,
        Sides {
            top: BorderImageWidthSide::LengthPercentage(ComputedLengthPercentage::Px(20.0)),
            right: BorderImageWidthSide::Number(3.0),
            bottom: BorderImageWidthSide::Auto,
            left: BorderImageWidthSide::Number(3.0),
        }
    );
    assert_eq!(image.outset.top, BorderImageOutsetSide::Length(10.0));
    assert_eq!(image.outset.right, BorderImageOutsetSide::Number(2.0));
    assert_eq!(image.repeat.horizontal, BorderImageRepeatKeyword::Round);
    assert_eq!(image.repeat.vertical, BorderImageRepeatKeyword::Space);
}

#[test]
fn longhands_cascade_and_border_resets_them() {
    let images = border_images(
        "#a { border-image: url(a.png) 10; border-image-slice: 5 fill; border-image-repeat: repeat }
         #b { border-image: url(b.png) 10; border: 2px solid }
         #c { border-image-source: url(c.png) }
         #d { border-image-width: inherit; border-image-outset: inherit }
         #c { border-image-width: 10%; border-image-outset: 4px }",
        "<div id=a></div><div id=b></div><div id=c><p id=d></p></div>",
        &["a", "b", "c", "d"],
    );
    assert_eq!(url(&images[0]), Some("a.png"));
    assert_eq!(
        images[0].slice.offsets.left,
        BorderImageSliceOffset::Number(5.0)
    );
    assert!(images[0].slice.fill);
    assert_eq!(images[0].repeat.vertical, BorderImageRepeatKeyword::Repeat);
    // `border` resets the whole border image.
    assert_eq!(images[1], ComputedBorderImage::initial());
    assert_eq!(url(&images[2]), Some("c.png"));
    // Not inherited, except where asked for.
    assert_eq!(images[3].source, ComputedBackgroundImage::None);
    assert_eq!(
        images[3].width.top,
        BorderImageWidthSide::LengthPercentage(ComputedLengthPercentage::Percent(10.0))
    );
    assert_eq!(images[3].outset.left, BorderImageOutsetSide::Length(4.0));
}

#[test]
fn invalid_declarations_are_dropped() {
    let images = border_images(
        "div { border-image-slice: 7 }
         #a { border-image-slice: -1 }
         #b { border-image-slice: 1 2 3 4 5 }
         #c { border-image-width: -2px }
         #d { border-image-repeat: stretch round space }
         #e { border-image: 10 / / }
         #f { border-image: url(x.png) / 2 }",
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div>\
         <div id=e></div><div id=f></div>",
        &["a", "b", "c", "d", "e", "f"],
    );
    for image in &images[..2] {
        assert_eq!(image.slice.offsets.top, BorderImageSliceOffset::Number(7.0));
    }
    assert_eq!(images[2].width.top, BorderImageWidthSide::Number(1.0));
    assert_eq!(
        images[3].repeat.horizontal,
        BorderImageRepeatKeyword::Stretch
    );
    for image in &images[4..] {
        assert_eq!(image.source, ComputedBackgroundImage::None);
        assert_eq!(image.slice.offsets.top, BorderImageSliceOffset::Number(7.0));
    }
}
