//! External-consumer coverage for the paint order a painter reads.
//!
//! Every type below is named through `raikiri_html` only, the way a
//! downstream painter that does not depend on the DOM crate would write it.

use raikiri_html::{
    ClipKind, DocumentLayout, LayoutOptions, LayoutStatus, PaintEvent, RenderResources,
    WarningKind, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str, style: &str) -> DocumentLayout {
    let html = format!("<!doctype html><style>{style}</style><body>{body}</body>");
    let resources = RenderResources::new();
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
    result
}

#[test]
fn absolute_box_paints_after_preceding_text() {
    let result = lay_out(
        "<p id=p>text</p><div id=a style='position:absolute;top:0;left:0;\
         width:50px;height:50px;background:red'></div>",
        "",
    );
    let page = result.page(0).unwrap();
    let dom = page.dom();
    let events = page.paint_order();
    let pos = |pred: &dyn Fn(&PaintEvent<'_>) -> bool| events.iter().position(pred).unwrap();
    let text = pos(&|e| {
        matches!(e, PaintEvent::Text(f)
            if dom.parent(f.node()).and_then(|n| dom.attr(n, "id")) == Some("p"))
    });
    let abs = pos(&|e| matches!(e, PaintEvent::Box(f) if dom.attr(f.node(), "id") == Some("a")));
    assert!(text < abs);
}

#[test]
fn paint_order_fragments_belong_to_page() {
    let result = lay_out(
        "<div style='opacity:.5'><p style='height:150px'>a</p></div><img style='width:10px;height:10px'>\
         <div style='position:fixed;top:0'>header</div><p>b</p>",
        "@page { size: 300px 100px; margin: 0 }",
    );
    let mut checked = 0;
    for page in result.pages() {
        let all: Vec<_> = page
            .fragments()
            .map(|f| (f.node(), f.kind(), f.fragment_index()))
            .collect();
        for event in page.paint_order() {
            if let PaintEvent::Box(f) | PaintEvent::Text(f) | PaintEvent::Replaced(f) = event {
                assert!(
                    all.contains(&(f.node(), f.kind(), f.fragment_index())),
                    "page {} event fragment not in fragments()",
                    page.index()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no event carries a fragment");
}

#[test]
fn spanning_overflow_box_clips_on_each_page() {
    let result = lay_out(
        "<div style='overflow:hidden'><p style='height:150px'>a</p></div>",
        "@page { size: 300px 100px; margin: 0 }",
    );
    for index in 0..2 {
        let events = result.page(index).unwrap().paint_order();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PaintEvent::PushClip(_, ClipKind::Overflow))),
            "page {index}"
        );
    }
}

#[test]
fn multicol_records_an_approximation_warning() {
    let result = lay_out("<div style='columns:2'><p>a</p><p>b</p></div>", "");
    let approximated: Vec<_> = result
        .warnings()
        .iter()
        .filter(|w| matches!(w.kind, WarningKind::PaintOrderApproximated))
        .collect();
    assert_eq!(approximated.len(), 1, "{approximated:?}");
    assert!(approximated[0].node_id.is_some());
}
