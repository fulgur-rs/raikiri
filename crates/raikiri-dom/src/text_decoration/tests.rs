use super::*;

#[test]
fn an_out_of_range_root_has_no_decoration_context() {
    let document = Document::new();
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    assert!(
        context_for_root(&document, &cascade, usize::MAX)
            .specs()
            .is_empty()
    );
}

#[test]
fn mixed_inset_copy_overflow_falls_back_to_a_finite_endpoint_span() {
    let mut values = ComputedValues::initial();
    values.text_decoration_line.underline = true;
    let mut spec = element_decoration(NodeId::new(1), &values, 0.0).unwrap();
    spec.inset_start = 1.0e308;
    spec.inset_end = -1.0e306;
    let (spans, count) = decoration_spans(-1.0e308, 1.5e308, &spec).unwrap();
    assert_eq!(count, 1);
    assert_eq!(spans[0].0, 0.0);
    assert!(spans[0].1.is_finite());
    assert!(spans[0].1 > 1.5e308);
}

#[test]
fn an_uncascaded_descendant_keeps_the_existing_decoration_context() {
    let mut document = Document::new();
    let root = document.append_element(
        Some(0),
        "p",
        taffy::Style::default(),
        Some("text-decoration:underline"),
    );
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    let base = context_for_root(&document, &cascade, root);
    assert_eq!(base.specs().len(), 1);
    let span = document.append_element(Some(root), "span", taffy::Style::default(), None::<&str>);
    let text = document.append_text(span, "ab");
    let result = context_for_text(&document, &cascade, root, text, &base, &HashMap::new());
    assert_eq!(result.specs(), base.specs());
}

#[test]
fn inline_baseline_shifts_require_the_documents_font_collection() {
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{ahem_paragraph_with, page_box_800x600, with_ahem};

    let mut span = 0;
    let (mut document, cascade, root) =
        ahem_paragraph_with("font-size:20px;line-height:20px", |document, root| {
            span = document.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("vertical-align:super"),
            );
            document.append_text(span, "ab");
        });
    layout_single_page(with_ahem(&mut document), &cascade, page_box_800x600()).unwrap();
    let line = &document.nodes[root].ifc_lines().unwrap()[0];
    let shifts = baseline_shifts(&document, line);
    assert!(shifts[&span] < 0.0);
    assert!(baseline_shifts(&Document::new(), line).is_empty());
}
