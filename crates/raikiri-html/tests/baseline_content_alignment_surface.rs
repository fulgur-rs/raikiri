//! `align-content: baseline` (and `first baseline`) on a block container
//! takes its fallback alignment `start` (CSS Box Alignment 3 §5.1), and, not
//! being `normal`, makes the block a formatting context root, so the top
//! margin of its first child stays inside it. Chromium does not accept `last
//! baseline` for `align-content`, though the grammar allows it, so the
//! declaration is dropped and the `normal` layout stays. The positions are the
//! ones Chromium 153 gives for the same documents.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The top of the aligned block and the offset of its child's top from it.
fn tops(alignment: &str) -> (f32, f32) {
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}</style>\
         <body style='margin:0'><div style='height:5px'></div>\
         <div id=r style='height:100px;align-content:{alignment}'>\
         <div id=c style='margin-top:10px;width:40px;height:20px'></div></div></body>"
    );
    let resources = RenderResources::new();
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let page = result.pages().next().unwrap();
    let dom = page.dom();
    let (mut root, mut child) = (None, None);
    for fragment in page.fragments() {
        match dom.attr(fragment.node(), "id") {
            Some("r") => root = Some(fragment.rect().y),
            Some("c") => child = Some(fragment.rect().y),
            _ => {}
        }
    }
    let (root, child) = (root.unwrap(), child.unwrap());
    (root, child - root)
}

#[test]
fn a_baseline_content_alignment_starts_the_content_in_its_own_formatting_context() {
    assert_eq!(tops("normal"), (15.0, 0.0));
    assert_eq!(tops("baseline"), (5.0, 10.0));
    assert_eq!(tops("first baseline"), (5.0, 10.0));
    assert_eq!(tops("start"), (5.0, 10.0));
}

#[test]
fn a_last_baseline_content_alignment_is_not_accepted() {
    assert_eq!(tops("last baseline"), (15.0, 0.0));
}
