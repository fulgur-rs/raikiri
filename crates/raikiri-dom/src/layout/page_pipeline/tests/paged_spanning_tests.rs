use super::*;

fn long_spanning_document() -> (Document, CascadeResult) {
    spanning_document("display:block;column-span:all")
}

fn spanning_document(heading_css: &str) -> (Document, CascadeResult) {
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
    let heading = document.append_element(Some(owner), "div", Style::default(), Some(heading_css));
    document.append_text(heading, "B");
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    with_ahem(&mut document);
    (document, cascade)
}

#[test]
fn spanning_pagination_completes_with_the_heading_after_the_column_tail() {
    let (mut document, cascade) = long_spanning_document();
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    let slices = layout_pages(&mut document, &cascade, page).unwrap();
    assert!(slices.len() > 1);
    document
        .project_pages(&cascade, page, &slices, &[])
        .unwrap();
    let tail = document.page_text_runs(&cascade, slices.last().unwrap().page_index);
    assert!(tail.iter().any(|run| run.text == "B"));
}

#[test]
fn spanning_pagination_propagates_an_exhausted_ancestor_growth_budget() {
    let (mut document, cascade) =
        spanning_document("display:block;column-span:all;height:40px;break-inside:avoid");
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    layout_single_page_with_table_projection(
        &mut crate::layout::context::LayoutSession::with_cascade(&mut document, &cascade),
        &cascade,
        page,
        true,
        true,
    )
    .unwrap();
    let owner = document
        .nodes
        .iter()
        .position(|n| !n.multicol_groups.is_empty())
        .unwrap();
    let parent = document.layout_parent_of(owner).unwrap();
    let root = crate::layout::initial_containing_block::root_element(&document).unwrap();
    let margins = page_margins(&cascade, page);
    let insets = page_content_insets(&cascade, page);
    let step = fixed_page_step(insets.page_area_height(margins, page), page.height);
    let origins = PageOrigins::new(&[], step);
    let control = PageLayoutControl::default();
    let limit = document.fragment_tree.work_limit(document.nodes.len());
    // Measure the preceding phase so only ancestor propagation runs out of work.
    let mut probe = document.clone();
    let mut edits = FragmentEdits::new(&probe.fragment_tree);
    let mut work = ProjectionWork::new(&control, limit);
    let owner_y = current_abs_y(&probe, owner);
    let delta = paginate_spanning_columns(
        &mut probe,
        &mut edits,
        owner,
        owner_y,
        &|y| {
            let index = origins.page_index_for_y(y);
            (index, origins.origin(index), origins.step_at(index))
        },
        &mut work,
    )
    .unwrap();
    assert_eq!(delta, 20.0);
    let planning_cost = limit - work.remaining();
    document.fragment_tree.break_flow_work_used = limit - planning_cost;
    let parent_height = document.nodes[parent].unrounded_layout.size.height;
    let result = paginate_flow(
        &mut crate::layout::context::LayoutSession::new(&mut document),
        &cascade,
        root,
        PageFlowGeometry {
            page_box: page,
            margins,
            insets,
        },
        &[],
        &[],
        &control,
    );
    assert!(
        matches!(result, Err(LayoutError::FragmentLimitExceeded { limit: actual }) if actual == limit)
    );
    assert_eq!(
        document.nodes[parent].unrounded_layout.size.height,
        parent_height
    );
    assert_eq!(
        document.nodes[owner].unrounded_layout.size.height,
        probe.nodes[owner].unrounded_layout.size.height
    );
}

#[test]
fn spanning_page_limit_is_checked_before_planning_the_unbounded_tail() {
    let (mut document, cascade) = long_spanning_document();
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    let control = PageLayoutControl::new(Some(1));
    let result = layout_pages_with_options(
        &mut document,
        &cascade,
        page,
        crate::PageLayoutOptions {
            page_steps: &[],
            page_widths: &[],
            control: Some(&control),
            ..Default::default()
        },
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
    let result = layout_pages_with_options(
        &mut document,
        &cascade,
        page,
        crate::PageLayoutOptions {
            page_steps: &[],
            page_widths: &[],
            control: Some(&control),
            ..Default::default()
        },
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
