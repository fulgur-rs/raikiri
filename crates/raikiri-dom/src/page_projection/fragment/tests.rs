use super::*;
use crate::page_projection::records::PageFragmentLineRange;

fn rect(x: f32, y: f32, w: f32, h: f32) -> PageFragmentRect {
    PageFragmentRect::new(x, y, w, h)
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
