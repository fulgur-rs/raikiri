use super::*;

fn page_tree(width: usize, depth: usize, named: bool) -> (Document, CascadeResult, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    for _ in 0..width {
        let mut parent = body;
        for _ in 0..depth {
            parent = doc.append_element(
                Some(parent),
                "div",
                Style::default(),
                Some(if named {
                    "display:block;page:chapter"
                } else {
                    "display:block"
                }),
            );
        }
        doc.append_text(parent, "x");
    }
    doc.mark_in_document_flags();
    crate::layout::test_support::with_ahem(&mut doc);
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    (doc, cascade, body)
}

#[test]
fn unnamed_layout_skips_page_name_propagation() {
    let mut observations = Vec::new();
    for (width, depth) in [(16, 1), (64, 1), (128, 1), (64, 4), (128, 4)] {
        let (mut doc, cascade, _) = page_tree(width, depth, false);
        PAGE_NAME_WORK.with(|work| work.set([0; 6]));
        let start = std::time::Instant::now();
        assert_eq!(first_page_name(&doc, &cascade), None);
        let pages = layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
        assert!(pages.iter().all(|page| page.page_name.is_none()));
        let elapsed = start.elapsed().as_micros();
        let work = PAGE_NAME_WORK.with(|work| work.get());
        eprintln!("width={width} depth={depth} work={work:?} layout-us={elapsed}");
        observations.push(work);
    }
    assert!(observations.iter().all(|work| *work == [0; 6]));
}

#[test]
fn selected_name_borrows_the_cascade_storage() {
    let (_doc, cascade, body) = page_tree(1, 1, true);
    let id = body + 1;
    let raikiri_style::property::PageValue::Named(name) = &cascade.page_values[id] else {
        panic!("named value");
    };
    let selected = selected_page_name(&cascade, id).unwrap();
    assert_eq!(selected.as_ptr(), name.0.as_str().as_ptr());
}

#[test]
fn unnamed_content_preserves_borrowed_inherited_names_and_child_order() {
    let (mut doc, cascade, body) = page_tree(2, 2, false);
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert!(!has_named_pages(&cascade));
    assert!(matches!(
        initial_page_child_order(&doc, &cascade, body),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        pagination_child_order_borrowed(&doc, &cascade, body),
        Cow::Borrowed(_)
    ));
    let inherited = String::from("borrowed-inherited-page");
    let (has_content, name) = propagated_start_page_name(&doc, &cascade, body, Some(&inherited));
    assert!(has_content);
    assert_eq!(name, Some(inherited.as_str()));
    assert_eq!(name.unwrap().as_ptr(), inherited.as_ptr());
    assert_eq!(
        propagated_start_page_name(&doc, &cascade, body, None),
        (true, None)
    );
}

#[test]
fn named_presence_is_recomputed_after_recascade() {
    let (mut doc, _, body) = page_tree(2, 2, false);
    let first = doc.nodes[body].children[0];
    for named in [false, true, false] {
        doc.set_element_inline_style(
            first,
            Some(
                if named {
                    "display:block;page:chapter"
                } else {
                    "display:block;page:auto"
                }
                .into(),
            ),
        );
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        PAGE_NAME_WORK.with(|work| work.set([0; 6]));
        assert_eq!(
            first_page_name(&doc, &cascade).as_deref(),
            named.then_some("chapter")
        );
        let pages = layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
        assert_eq!(pages[0].page_name.as_deref(), named.then_some("chapter"));
        let work = PAGE_NAME_WORK.with(|work| work.get());
        if named {
            assert!(work[0] > 0);
            assert!(work[2] > 0);
        } else {
            assert_eq!(work, [0; 6]);
        }
    }
}

#[test]
fn unnamed_flex_skips_named_subtree_search() {
    let (mut doc, _, body) = page_tree(16, 2, false);
    doc.nodes[body].data.as_element_mut().unwrap().inline_style = Some("display:flex".into());
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    PAGE_NAME_WORK.with(|work| work.set([0; 6]));
    layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
    assert_eq!(PAGE_NAME_WORK.with(|work| work.get()), [0; 6]);
}

#[test]
fn unnamed_first_child_stops_before_a_later_named_sibling() {
    let (mut doc, _, body) = page_tree(2, 1, false);
    let later = doc.nodes[body].children[1];
    doc.set_element_inline_style(later, Some("display:block;page:chapter".into()));
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    assert!(has_named_pages(&cascade));
    assert_eq!(first_page_name(&doc, &cascade), None);
    layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
    assert_eq!(first_page_name(&doc, &cascade), None);
    assert_eq!(
        propagated_start_page_name(&doc, &cascade, body, None),
        (true, None)
    );
}

#[test]
fn named_flex_keeps_owned_reordered_children() {
    let (mut doc, _, body) = page_tree(2, 1, true);
    let first = doc.nodes[body].children[0];
    let last = doc.nodes[body].children[1];
    doc.set_element_inline_style(body, Some("display:flex".into()));
    doc.set_element_inline_style(first, Some("display:block;order:1;page:first".into()));
    doc.set_element_inline_style(last, Some("display:block;order:-1;page:last".into()));
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let initial = initial_page_child_order(&doc, &cascade, body);
    assert!(matches!(initial, Cow::Owned(_)));
    assert_eq!(&*initial, &[last, first]);
    layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
    let resolved = pagination_child_order_borrowed(&doc, &cascade, body);
    assert!(matches!(resolved, Cow::Owned(_)));
    assert_eq!(&*resolved, &[last, first]);
}

#[test]
fn propagation_resumes_after_hidden_and_whitespace_children() {
    for display in ["block", "flex"] {
        for named_child in [false, true] {
            let mut doc = Document::new();
            let root = doc.append_element(
                Some(0),
                "div",
                Style::default(),
                Some(&format!("display:{display};page:outer")),
            );
            doc.append_text(root, "   ");
            doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:none;page:hidden"),
            );
            doc.append_text(root, "\n");
            if named_child {
                doc.append_element(
                    Some(root),
                    "div",
                    Style::default(),
                    Some("display:block;page:inner"),
                );
            }
            doc.mark_in_document_flags();
            let rules = raikiri_style::build_rule_tree(&doc);
            let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
            crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).unwrap();
            assert_eq!(
                propagated_start_page_name_with_order(&doc, &cascade, root, None, false, false),
                (true, Some(if named_child { "inner" } else { "outer" })),
            );
        }
    }
}
