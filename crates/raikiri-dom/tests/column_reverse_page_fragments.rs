//! A reversed column can put an unshrunk item above the first page.
//! Page fragments expose the part of its border box on that page; the
//! content rectangle retains the whole content box. The same item fits whole after
//! moving its container down, without changing its flex sizing.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults, PaintRect};

fn boxes(children: &str, offset: f32) -> Vec<(String, PaintRect, PaintRect)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size:800px 600px;margin:0 }} \
         body {{ margin:0 }}</style><div style='height:{offset}px'></div>\
         <div style='display:flex;width:300px;flex-direction:column-reverse;height:100px'>{children}</div>"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("incomplete layout")
    };
    let page = result.pages().next().expect("page");
    page.fragments()
        .filter_map(|fragment| {
            let id = page.dom().attr(fragment.node(), "id")?;
            Some((
                id.to_owned(),
                fragment.rect(),
                fragment.content_rect().expect("whole content box"),
            ))
        })
        .collect()
}

#[test]
fn reversed_column_page_cuts_do_not_shrink_the_whole_item() {
    for (children, expected) in [
        (
            "<div id=i style='height:300px;flex:none'></div>",
            vec![("i", -200.0, 300.0)],
        ),
        (
            "<div id=i style='height:80px;flex:none'></div><div id=j style='height:80px;flex:none'></div>",
            vec![("i", 20.0, 80.0), ("j", -60.0, 80.0)],
        ),
        (
            "<div id=i style='aspect-ratio:1/1;height:300px;flex:1 1 0'></div>",
            vec![("i", -200.0, 300.0)],
        ),
        (
            "<div id=i style='height:60px;flex:none'></div>",
            vec![("i", 40.0, 60.0)],
        ),
    ] {
        for offset in [0.0_f32, 200.0] {
            let actual = boxes(children, offset);
            assert_eq!(actual.len(), expected.len());
            for &(id, y, height) in &expected {
                let (_, rect, whole) = actual
                    .iter()
                    .find(|(node_id, _, _)| node_id == id)
                    .expect("item fragment");
                assert_eq!(
                    (whole.x, whole.y, whole.width, whole.height),
                    (0.0, y + offset, 300.0, height)
                );
                let visible_top = (y + offset).max(0.0);
                let visible_bottom = y + offset + height;
                assert_eq!(
                    (rect.x, rect.y, rect.width, rect.height),
                    (0.0, visible_top, 300.0, visible_bottom - visible_top)
                );
            }
        }
    }
}

#[test]
fn page_cuts_preserve_content_insets_without_calling_the_content_box_a_border_box() {
    let child = "<div id=i style='height:300px;flex:none;padding:10px;border:5px solid'></div>";
    for offset in [0.0_f32, 300.0] {
        let actual = boxes(child, offset);
        assert_eq!(actual.len(), 1);
        let (_, fragment, content) = &actual[0];
        assert_eq!(*content, PaintRect::new(15.0, offset - 215.0, 270.0, 300.0));
        let top = (offset - 230.0).max(0.0);
        assert_eq!(
            *fragment,
            PaintRect::new(0.0, top, 300.0, offset + 100.0 - top)
        );
    }
}

#[test]
fn an_item_entirely_above_the_first_page_has_no_fragment_on_that_page() {
    let children = "<div id=i style='height:300px;flex:none'></div><div id=j style='height:300px;flex:none'></div>";
    let actual = boxes(children, 0.0);
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].0, "i");
    assert_eq!(actual[0].1, PaintRect::new(0.0, 0.0, 300.0, 100.0));
    assert_eq!(actual[0].2, PaintRect::new(0.0, -200.0, 300.0, 300.0));
}
