use super::*;

fn long_spanning_document() -> (Document, CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let owner = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:100px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let paragraph = document.append_element(
        Some(owner),
        "p",
        Style::default(),
        Some("display:block;white-space:pre;margin:0"),
    );
    document.append_text(
        paragraph,
        std::iter::repeat_n("A", 1000)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let heading = document.append_element(
        Some(owner),
        "div",
        Style::default(),
        Some("display:block;column-span:all"),
    );
    document.append_text(heading, "B");
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    with_ahem(&mut document);
    (document, cascade)
}

#[test]
fn spanning_page_limit_is_checked_before_planning_the_unbounded_tail() {
    let (mut document, cascade) = long_spanning_document();
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    let control = PageLayoutControl::new(Some(1));
    let result = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page,
        &[],
        &[],
        &control,
    );
    assert!(
        matches!(
            result,
            Err(LayoutError::PageLimitExceeded {
                limit: 1,
                actual: 2
            })
        ),
        "{result:?}"
    );
}

#[test]
fn spanning_geometry_discovery_retains_a_page_prefix_without_planning_the_tail() {
    let (mut document, cascade) = long_spanning_document();
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    let control = PageLayoutControl::for_geometry_discovery(Some(1));
    let result = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page,
        &[],
        &[],
        &control,
    );
    let slices = result.expect("bounded spanning page discovery");
    assert_eq!(slices.len(), 1);
    assert!(control.page_limit_reached());
    document
        .project_pages(&cascade, page, &slices, &[])
        .unwrap();
    let runs = document.page_text_runs(&cascade, 0);
    assert_eq!(runs.len(), 6);
    assert!(runs.iter().all(|run| run.text == "A"));
}

#[test]
fn spanning_source_fragment_limit_returns_a_layout_error() {
    let (mut document, cascade) = long_spanning_document();
    document.fragment_tree.limit = 2;
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    assert!(matches!(
        layout_single_page(&mut document, &cascade, page),
        Err(LayoutError::FragmentLimitExceeded { limit: 2 })
    ));
    assert!(document.fragment_tree.limit_exceeded);
    assert_eq!(document.fragment_tree.fragments.len(), 2);
}
