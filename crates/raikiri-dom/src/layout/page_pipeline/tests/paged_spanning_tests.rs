use super::*;

fn long_spanning_document() -> (Document, CascadeResult) {
    spanning_document("display:block;column-span:all")
}

fn spanning_document(heading_css: &str) -> (Document, CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    document.append_element(Some(html), "head", None::<&str>);
    let body = document.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let owner = document.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let paragraph = document.append_element(
        Some(owner),
        "p",
        Some("display:block;white-space:pre;margin:0"),
    );
    document.append_text(
        paragraph,
        std::iter::repeat_n("A", 1000)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let heading = document.append_element(Some(owner), "div", Some(heading_css));
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
        &mut crate::layout::context::LayoutSession::bridged(&mut probe, &cascade),
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
    let parent_height = document.nodes[parent].layout.size.height;
    let result = paginate_flow(
        &mut crate::layout::context::LayoutSession::bridged(&mut document, &cascade),
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
    assert_eq!(document.nodes[parent].layout.size.height, parent_height);
    assert_eq!(
        document.nodes[owner].layout.size.height,
        probe.nodes[owner].layout.size.height
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

#[test]
fn a_multicol_owner_is_monolithic_when_pagination_is_transposed() {
    let mut document = Document::new();
    let html = document.append_element(
        Some(0),
        "html",
        Some("display:block;writing-mode:vertical-rl"),
    );
    document.append_element(Some(html), "head", None::<&str>);
    let body = document.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let owner = document.append_element(
        Some(body),
        "div",
        Some(
            "display:block;writing-mode:horizontal-tb;width:100px;column-count:2;\
             column-gap:20px;orphans:1;widows:1",
        ),
    );
    let paragraph = document.append_element(
        Some(owner),
        "p",
        Some("display:block;white-space:pre;margin:0"),
    );
    document.append_text(
        paragraph,
        std::iter::repeat_n("A", 200).collect::<Vec<_>>().join("\n"),
    );
    let heading =
        document.append_element(Some(owner), "div", Some("display:block;column-span:all"));
    document.append_text(heading, "B");
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    with_ahem(&mut document);
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    let slices = layout_pages(&mut document, &cascade, page).unwrap();
    assert!(!slices.is_empty());
    // The owner still holds the groups of its source layout.
    assert_eq!(document.nodes[owner].multicol_groups.len(), 1);
}

#[test]
fn a_multicol_owner_without_a_spanner_keeps_no_groups_when_pagination_is_transposed() {
    let mut document = Document::new();
    let html = document.append_element(
        Some(0),
        "html",
        Some("display:block;writing-mode:vertical-rl"),
    );
    document.append_element(Some(html), "head", None::<&str>);
    let body = document.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let owner = document.append_element(
        Some(body),
        "div",
        Some(
            "display:block;writing-mode:horizontal-tb;width:100px;column-count:2;\
             column-gap:20px;orphans:1;widows:1",
        ),
    );
    let paragraph = document.append_element(
        Some(owner),
        "p",
        Some("display:block;white-space:pre;margin:0"),
    );
    document.append_text(
        paragraph,
        std::iter::repeat_n("A", 200).collect::<Vec<_>>().join("\n"),
    );
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    with_ahem(&mut document);
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 60.0;
    let slices = layout_pages(&mut document, &cascade, page).unwrap();
    assert!(!slices.is_empty());
    // Its columns stay as the source layout made them, for the pages to cut.
    assert!(document.nodes[owner].multicol_groups.is_empty());
}

/// A two-column owner whose paragraph fills the first 200px, followed by a
/// spanner of 350px that holds no text: a tall first page that the spanner
/// starts on, and a tail of 20px pages for it to run into.
fn spanner_over_a_short_tail() -> (Document, CascadeResult, PageBox) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    document.append_element(Some(html), "head", None::<&str>);
    let body = document.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let owner = document.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let paragraph = document.append_element(
        Some(owner),
        "p",
        Some("display:block;white-space:pre;margin:0"),
    );
    document.append_text(
        paragraph,
        std::iter::repeat_n("A", 20).collect::<Vec<_>>().join("\n"),
    );
    document.append_element(
        Some(owner),
        "div",
        Some("display:block;column-span:all;height:350px"),
    );
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    with_ahem(&mut document);
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 20.0;
    (document, cascade, page)
}

fn paginate_spanner_over_a_short_tail(
    control: &PageLayoutControl<'_>,
) -> (
    Result<Vec<PageSlice>, LayoutError>,
    Document,
    CascadeResult,
    PageBox,
) {
    let (mut document, cascade, page) = spanner_over_a_short_tail();
    let result = layout_pages_with_options(
        &mut document,
        &cascade,
        page,
        crate::PageLayoutOptions {
            page_steps: &[400.0],
            page_widths: &[],
            control: Some(control),
            ..Default::default()
        },
    );
    (result, document, cascade, page)
}

#[test]
fn a_spanner_that_runs_into_a_short_tail_is_cut_on_every_page_it_crosses() {
    let control = PageLayoutControl::new(None);
    let (result, ..) = paginate_spanner_over_a_short_tail(&control);
    // The spanner crosses the ends of the tall page and of seven short ones.
    assert_eq!(result.unwrap().len(), 9);
}

#[test]
fn a_spanner_over_a_short_tail_reports_the_page_limit_of_a_strict_control() {
    let control = PageLayoutControl::new(Some(3));
    let (result, ..) = paginate_spanner_over_a_short_tail(&control);
    assert!(
        matches!(
            result,
            Err(LayoutError::PageLimitExceeded {
                limit: 3,
                actual: 4
            })
        ),
        "{result:?}"
    );
}

#[test]
fn geometry_discovery_keeps_the_pages_a_spanner_reaches_before_the_page_limit() {
    let control = PageLayoutControl::for_geometry_discovery(Some(3));
    let (result, ..) = paginate_spanner_over_a_short_tail(&control);
    assert_eq!(result.unwrap().len(), 3);
    assert!(control.page_limit_reached());
}
