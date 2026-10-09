use crate::column_rules::project;
use crate::layout::test_support::ahem_paragraph;
use crate::{PageContentInsets, PageMargins, PageSlice, layout_single_page};
use raikiri_traits::{LayoutError, NodeId, PageBox};

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
