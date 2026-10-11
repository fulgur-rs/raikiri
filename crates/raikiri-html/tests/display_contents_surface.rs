//! A `display: contents` element generates no box of its own (CSS Display 3
//! §2.5): its children lay out and paint as children of its parent.

use raikiri_html::{
    DocumentLayout, LayoutOptions, LayoutStatus, PaintEvent, PaintRect, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str, style: &str) -> DocumentLayout {
    let fonts = raikiri_html::FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page {{size:200px 150px;margin:0}} body {{margin:0;font:10px/10px Ahem}} \
         .contents {{display:contents;width:5px;height:5px;padding:20px;margin:20px;\
         border:5px solid red;background:red;overflow:hidden;column-count:2}} {style}</style><body>{body}</body>"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap();
    let LayoutStatus::Completed(result) = status else {
        panic!("layout must complete")
    };
    result
}

fn rect_of(result: &DocumentLayout, id: &str) -> PaintRect {
    let page = result.page(0).unwrap();
    page.fragments()
        .find(|fragment| page.dom().attr(fragment.node(), "id") == Some(id))
        .unwrap_or_else(|| panic!("no fragment for {id}"))
        .rect()
}

#[test]
fn block_children_ignore_the_contents_element_box() {
    let result = lay_out(
        "<div class=contents id=contents><p id=child style='margin:0'>X</p></div>",
        "",
    );
    let child = rect_of(&result, "child");
    // Neither the width, nor the padding, border or margin of the contents
    // element reaches its child.
    assert_eq!((child.x, child.y, child.width), (0.0, 0.0, 200.0));
}

#[test]
fn flex_and_grid_containers_take_the_children_as_items() {
    let result = lay_out(
        "<div style='display:flex'><div class=contents>\
         <i id=a></i><i id=b></i></div></div>\
         <div style='display:grid;grid-template-columns:repeat(2,50px)'>\
         <div class=contents><i id=c></i><i id=d></i></div></div>",
        "i {display:block;width:30px;height:10px}",
    );
    let (a, b) = (rect_of(&result, "a"), rect_of(&result, "b"));
    assert_eq!((a.x, b.x), (0.0, 30.0));
    assert_eq!(a.y, b.y);
    let (c, d) = (rect_of(&result, "c"), rect_of(&result, "d"));
    assert_eq!((c.x, d.x), (0.0, 50.0));
    assert_eq!((c.y, d.y), (10.0, 10.0));
}

#[test]
fn contents_elements_paint_no_box() {
    let result = lay_out(
        "<div class=contents id=block><p>X</p></div>\
         <p>A<span class=contents id=inline>B</span>C</p>\
         <div style='display:flex'><span class=contents id=flex><b>F</b></span></div>",
        "",
    );
    let page = result.page(0).unwrap();
    let dom = page.dom();
    let runs = page.text_runs();
    for events in [page.paint_order(), page.paint_order_for_text_runs(&runs)] {
        for event in events {
            if let PaintEvent::Box(fragment) = event {
                assert!(
                    !matches!(
                        dom.attr(fragment.node(), "id"),
                        Some("block" | "inline" | "flex")
                    ),
                    "a box for {:?}",
                    dom.attr(fragment.node(), "id")
                );
            }
        }
    }
}

#[test]
fn unusual_elements_compute_contents_to_none() {
    let result = lay_out(
        "<div><img class=contents id=img><input class=contents id=input>\
         <textarea class=contents id=textarea></textarea>\
         <svg class=contents id=svg width=50 height=50></svg></div>\
         <p id=after style='margin:0'>X</p>",
        "",
    );
    let page = result.page(0).unwrap();
    for id in ["img", "input", "textarea", "svg"] {
        assert!(
            page.fragments()
                .all(|fragment| page.dom().attr(fragment.node(), "id") != Some(id)),
            "{id} must generate no box"
        );
    }
    // Nothing of theirs takes up space before the following paragraph.
    assert_eq!(rect_of(&result, "after").y, 0.0);
}

#[test]
fn elements_sharing_style_with_an_unusual_sibling_still_unbox() {
    let result = lay_out(
        "<div><br class=c><x-a class=c>PASS</x-a></div>",
        ".c {display:contents}",
    );
    let page = result.page(0).unwrap();
    assert!(page.text_runs().iter().any(|run| run.text.contains("PASS")));
}
