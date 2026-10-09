use crate::column_rules::project;
use crate::layout::test_support::ahem_paragraph;
use crate::{PageContentInsets, PageMargins, PageSlice, layout_single_page};
use raikiri_traits::{LayoutError, NodeId, PageBox, PaintRect};

#[test]
fn page_rule_projection_preserves_previous_snapshot_on_budget_or_abort() {
    let (mut document, cascade, owner) = ahem_paragraph(
        "A\nB\nC\nD",
        "width:100px;column-count:2;column-gap:20px;column-rule:2px solid red;orphans:1;widows:1",
    );
    let page_box = PageBox::default();
    layout_single_page(&mut document, &cascade, page_box).unwrap();
    let slices = [PageSlice {
        page_index: 0,
        content_origin_y: 0.0,
        page_name: None,
    }];
    let geometry = [(
        page_box,
        PageMargins::default(),
        PageContentInsets::default(),
    )];
    document
        .project_pages(&cascade, page_box, &slices, &geometry)
        .unwrap();
    let before = document.page_projection.column_rules.clone();
    let pages = document.page_projection.pages.clone();
    let control = crate::PageLayoutControl::default();
    let mut work = crate::layout::ProjectionWork::new(&control, 0);
    assert!(matches!(
        project(&document, &cascade, &pages, &mut work),
        Err(LayoutError::FragmentLimitExceeded { limit: 0 })
    ));
    let aborted = || true;
    let control = crate::PageLayoutControl::default().with_abort_check(&aborted);
    let mut work = crate::layout::ProjectionWork::new(&control, 100_000);
    assert!(matches!(
        project(&document, &cascade, &pages, &mut work),
        Err(LayoutError::Aborted)
    ));
    assert_eq!(document.page_projection.column_rules, before);
    assert_eq!(before[&0][&NodeId::new(owner as u64)].len(), 1);
}

#[test]
fn padding_only_page_does_not_publish_a_column_rule() {
    let (mut document, cascade, owner) = ahem_paragraph(
        "A\nB\nC\nD",
        "padding-top:100px;width:100px;column-count:2;column-gap:20px;column-rule:2px solid red;orphans:1;widows:1",
    );
    let mut page_box = PageBox::default();
    page_box.width = 180.0;
    page_box.height = 80.0;
    layout_single_page(&mut document, &cascade, page_box).unwrap();
    let slices = [0.0, 80.0].map(|content_origin_y| PageSlice {
        page_index: (content_origin_y / 80.0) as u32,
        content_origin_y,
        page_name: None,
    });
    document
        .project_pages(&cascade, page_box, &slices, &[])
        .unwrap();
    assert!(!document.page_projection.column_rules.contains_key(&0));
    let rule = &document.page_projection.column_rules[&1][&NodeId::new(owner as u64)][0];
    assert_eq!(rule.rect, PaintRect::new(49.0, 20.0, 2.0, 20.0));
    assert_eq!(rule.pattern_origin, 20.0);
    assert_eq!(rule.pattern_height, 20.0);
}
