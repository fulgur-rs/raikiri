//! A flex item's whole content box can begin before the first page, while
//! its fragment exposes only the part on that page (RAIKIRI-1477).
//! Chromium DOMRect and PDF measurements are in `data/flex-cross-overflow.json`.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

type Rect = (f32, f32, f32, f32);

fn item_boxes(html: &str) -> (Rect, Rect) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
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
    let pages = result.pages().collect::<Vec<_>>();
    assert_eq!(pages.len(), 1);
    let page = &pages[0];
    let fragments = page
        .fragments()
        .filter(|fragment| page.dom().attr(fragment.node(), "id") == Some("a"))
        .collect::<Vec<_>>();
    assert_eq!(fragments.len(), 1);
    let fragment = &fragments[0];
    let rect = fragment.rect();
    let whole = fragment.content_rect().expect("whole content box");
    (
        (rect.x, rect.y, rect.width, rect.height),
        (whole.x, whole.y, whole.width, whole.height),
    )
}

#[test]
fn overflowing_cross_aligned_items_keep_their_whole_box_and_visible_page_extent() {
    for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for sizing in [
            "height:170px",
            "inline-size:max-content",
            "inline-size:fit-content",
        ] {
            if mode == "horizontal-tb" && sizing != "height:170px" {
                continue;
            }
            for (alignment, relative_top) in [
                ("flex-end", -90.0),
                ("center", -45.0),
                ("flex-start", 0.0),
                ("stretch", 0.0),
            ] {
                for (offset, sibling) in [(0, false), (100, false), (0, true), (100, true)] {
                    let before = if sibling {
                        r#"<div style="width:10px;height:50px"></div>"#
                    } else {
                        ""
                    };
                    // Seventeen unbreakable Ahem glyphs measure 170px at
                    // both min-content and max-content inline sizes.
                    let html = format!(
                        r#"<!doctype html><style>@page {{ size:800px 600px;margin:0 }}
                        body {{ margin:0;font:10px/10px Ahem }} div {{ box-sizing:border-box }}</style>
                        <div style="height:{offset}px"></div>
                        <div style="display:flex;align-items:{alignment};height:80px">
                        {before}<div id=a style="writing-mode:{mode};width:10px;{sizing}">xxxxxxxxxxxxxxxxx</div></div>"#
                    );
                    let (visible, whole) = item_boxes(&html);
                    let top: f32 = relative_top + offset as f32;
                    let x = if sibling { 10.0 } else { 0.0 };
                    let context = format!("{mode} | {sizing} | {alignment} | {offset} | {sibling}");
                    assert_eq!(whole, (x, top, 10.0, 170.0), "{context}");
                    assert_eq!(
                        visible,
                        (x, top.max(0.0), 10.0, 170.0 + top.min(0.0)),
                        "{context}"
                    );
                }
            }
        }
    }
}
