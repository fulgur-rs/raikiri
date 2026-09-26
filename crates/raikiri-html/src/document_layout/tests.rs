use super::*;
use raikiri_traits::{
    NodeId, NodeKind, PageFragmentItem, PageFragmentKind, PageFragmentLineRange, PageFragmentRect,
};

fn rect(x: f32, y: f32, w: f32, h: f32) -> PageFragmentRect {
    let mut r = PageFragmentRect::default();
    r.x = x;
    r.y = y;
    r.width = w;
    r.height = h;
    r
}

#[test]
fn fragment_rect_is_moved_to_the_page_box_origin() {
    let item = PageFragmentItem::new(
        NodeId(7),
        rect(5.0, 8.0, 20.0, 10.0),
        PageFragmentKind::Text,
        0,
        2,
        false,
    )
    .with_page_index(0)
    .with_line_range(PageFragmentLineRange::new(0, 3));
    let f = Fragment::new(&item, rect(30.0, 40.0, 100.0, 100.0));
    let r = f.rect();
    assert_eq!((r.x, r.y, r.width, r.height), (35.0, 48.0, 20.0, 10.0));
    assert_eq!(f.node(), NodeId(7));
    assert_eq!(f.kind(), FragmentKind::Text);
    assert_eq!(f.line_range(), Some(0..3));
    assert_eq!(f.is_first_fragment(), Some(true));
    assert_eq!(f.is_last_fragment(), Some(false));
    assert_eq!(f.repeat(), None);
    assert!(!f.continuation());
}

#[test]
fn repeated_fragment_reports_every_page() {
    let item = PageFragmentItem::new(
        NodeId(3),
        rect(0.0, 0.0, 1.0, 1.0),
        PageFragmentKind::Box,
        1,
        2,
        true,
    );
    let item = item.with_page_index(1);
    let f = Fragment::new(&item, rect(0.0, 0.0, 10.0, 10.0));
    assert_eq!(f.repeat(), Some(RepeatKind::EveryPage));
    assert_eq!(f.is_last_fragment(), Some(true));
}

fn dom(html: &str) -> crate::HtmlDocument {
    crate::parse_html_with_resources(html.as_bytes(), &crate::RenderResources::new())
        .expect("parse")
}

fn find(view: &DomView<'_>, node: NodeId, name: &str) -> Option<NodeId> {
    if view.local_name(node) == Some(name) {
        return Some(node);
    }
    view.children(node)
        .find_map(|child| find(view, child, name))
}

#[test]
fn dom_view_walks_structure_and_attributes() {
    let doc = dom(r#"<p id="x" class="c">He<b>ll</b>o</p><svg><g></g></svg>"#);
    let view = DomView::new(doc.dom());
    let p = find(&view, view.root(), "p").expect("p");
    assert_eq!(view.kind(p), Some(NodeKind::Element));
    assert_eq!(view.attr(p, "id"), Some("x"));
    assert_eq!(
        view.namespace(p),
        None,
        "HTML namespace uses the None fast path"
    );
    let b = find(&view, p, "b").expect("b");
    assert_eq!(view.parent(b), Some(p));
    assert_eq!(view.text_content(p), "Hello");
    let g = find(&view, view.root(), "g").expect("g");
    assert_eq!(view.namespace(g), Some("http://www.w3.org/2000/svg"));
}

#[test]
fn dom_view_text_content_inserts_spaces_at_block_boundaries() {
    let doc = dom("<a href=x><div>foo</div><div>bar</div></a>");
    let view = DomView::new(doc.dom());
    let a = find(&view, view.root(), "a").expect("a");
    assert_eq!(view.text_content(a), "foo bar");
}

#[test]
fn dom_view_is_total_on_out_of_range_ids() {
    let doc = dom("<p>x</p>");
    let view = DomView::new(doc.dom());
    let bad = NodeId(1_000_000);
    assert_eq!(view.kind(bad), None);
    assert_eq!(view.parent(bad), None);
    assert_eq!(view.children(bad).count(), 0);
    assert_eq!(view.local_name(bad), None);
    assert_eq!(view.attr(bad, "id"), None);
    assert_eq!(view.text(bad), None);
    assert_eq!(view.text_content(bad), "");
}
