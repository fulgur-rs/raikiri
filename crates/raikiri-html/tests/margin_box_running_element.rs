//! External-consumer coverage for page-margin boxes that show a running
//! element (`content: element(<name>)`, CSS GCPM 3 §1.2.2).

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, MarginBox, Page,
    PageMarginBoxSlot, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn lay_out(margin_boxes: &str, body: &str) -> DocumentLayout {
    let html = format!(
        r#"<!doctype html><style>
          @page {{ size: 300px 200px; margin: 40px; {margin_boxes} }}
          body {{ margin: 0; font: 10px/10px Ahem }}
          p {{ margin: 0 }}
          .hdr {{ position: running(hdr) }}
        </style>{body}"#
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    result
}

fn slot(page: &Page<'_>, slot: PageMarginBoxSlot) -> MarginBox {
    page.margin_boxes()
        .into_iter()
        .find(|margin_box| margin_box.slot == slot)
        .expect("margin box")
}

/// The text of the running element shown in `slot_name` on `page`.
fn shown(page: &Page<'_>, slot_name: PageMarginBoxSlot) -> Option<String> {
    let margin_box = slot(page, slot_name);
    let placed = page
        .margin_box_running_element(&margin_box)
        .expect("layout")?;
    let runs = placed.layout.page().text_runs();
    Some(
        runs.iter()
            .map(|run| run.text.trim().to_owned())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[test]
fn a_margin_box_shows_the_running_element_of_each_page() {
    let result = lay_out(
        "@top-center { content: element(hdr) }",
        r#"<h1 class="hdr">One</h1><p>A</p>
           <h1 class="hdr" style="break-before: page">Two</h1><p style="break-before: page">B</p>
           <p style="break-before: page">C</p>"#,
    );
    let pages: Vec<_> = result.pages().collect();
    assert_eq!(pages.len(), 3);
    let shown: Vec<_> = pages
        .iter()
        .map(|page| shown(page, PageMarginBoxSlot::TopCenter))
        .collect();
    // Page 3 has no assignment and keeps the last element of page 2.
    assert_eq!(
        shown,
        vec![Some("One".into()), Some("Two".into()), Some("Two".into())]
    );
    let margin_box = slot(&pages[0], PageMarginBoxSlot::TopCenter);
    // The box text stays as a flat fallback.
    assert!(margin_box.text.is_some());
    let running = margin_box.running.as_ref().expect("running");
    let placed = pages[0]
        .margin_box_running_element(&margin_box)
        .expect("layout")
        .expect("placed");
    assert_eq!(placed.layout.width(), running.content_box.width);
    // The same element at the same width is laid out once.
    let again = pages[0]
        .margin_box_running_element(&margin_box)
        .expect("layout")
        .expect("placed");
    assert!(std::ptr::eq(placed.layout, again.layout));
}

#[test]
fn the_element_is_aligned_in_the_content_box() {
    let body = r#"<div class="hdr">H</div><p>A</p>"#;
    let origin = |align: &str| {
        let result = lay_out(
            &format!("@top-center {{ content: element(hdr); vertical-align: {align} }}"),
            body,
        );
        let page = result.pages().next().expect("page");
        let margin_box = slot(&page, PageMarginBoxSlot::TopCenter);
        let content = margin_box.running.as_ref().expect("running").content_box;
        let placed = page
            .margin_box_running_element(&margin_box)
            .expect("layout")
            .expect("placed");
        (
            placed.origin.0 - content.x,
            placed.origin.1 - content.y,
            content.height - placed.layout.height(),
        )
    };
    let (x, top, free) = origin("text-top");
    assert_eq!((x, top), (0.0, 0.0));
    assert!(free > 0.0);
    assert_eq!(origin("middle").1, free / 2.0);
    assert_eq!(origin("text-bottom").1, free);
}

#[test]
fn boxes_without_an_element_or_without_an_applicable_one_show_nothing() {
    let result = lay_out(
        r#"@top-left { content: "Title" }
           @top-right { content: element(hdr, first-except) }
           @bottom-center { content: element(ftr) }"#,
        r#"<div class="hdr">H</div><p>A</p><p style="break-before: page">B</p>"#,
    );
    let pages: Vec<_> = result.pages().collect();
    let top_left = slot(&pages[0], PageMarginBoxSlot::TopLeft);
    assert!(top_left.running.is_none());
    assert!(top_left.text.is_some());
    assert!(
        pages[0]
            .margin_box_running_element(&top_left)
            .expect("layout")
            .is_none()
    );
    // `first-except` shows nothing on the page the element is assigned on,
    // and the element on the pages after it.
    assert_eq!(shown(&pages[0], PageMarginBoxSlot::TopRight), None);
    assert_eq!(
        shown(&pages[1], PageMarginBoxSlot::TopRight),
        Some("H".into())
    );
    // No element is named `ftr`.
    let bottom = slot(&pages[0], PageMarginBoxSlot::BottomCenter);
    assert!(bottom.running.is_some());
    assert_eq!(shown(&pages[0], PageMarginBoxSlot::BottomCenter), None);
}

#[test]
fn a_running_layout_page_has_no_running_elements_of_its_own() {
    let result = lay_out(
        "@top-center { content: element(hdr) }",
        r#"<div class="hdr">H</div><p>A</p>"#,
    );
    let page = result.pages().next().expect("page");
    let margin_box = slot(&page, PageMarginBoxSlot::TopCenter);
    let placed = page
        .margin_box_running_element(&margin_box)
        .expect("layout")
        .expect("placed");
    assert!(
        placed
            .layout
            .page()
            .margin_box_running_element(&margin_box)
            .expect("layout")
            .is_none()
    );
}

#[test]
fn an_element_combined_with_other_content_keeps_the_flat_text() {
    let result = lay_out(
        r#"@top-center { content: "Chapter: " element(hdr) }"#,
        r#"<div class="hdr">H</div><p>A</p>"#,
    );
    let page = result.pages().next().expect("page");
    let margin_box = slot(&page, PageMarginBoxSlot::TopCenter);
    assert!(margin_box.running.is_none());
    assert!(
        page.margin_box_running_element(&margin_box)
            .expect("layout")
            .is_none()
    );
    let text: String = margin_box
        .text_runs()
        .iter()
        .map(|run| run.text.to_string())
        .collect();
    assert_eq!(text.trim(), "Chapter: H");
}
