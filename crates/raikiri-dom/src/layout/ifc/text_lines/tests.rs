use crate::layout::layout_single_page;
use crate::layout::test_support::{
    ahem_font_context, ahem_paragraph, ifc_ahem_fonts, page_box_800x600,
};

fn lay_out(doc: &mut crate::Document, cascade: &raikiri_style::CascadeResult) {
    doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(doc, cascade, page_box_800x600(), ahem_font_context()).expect("layout");
}

fn recascade(doc: &mut crate::Document) -> raikiri_style::CascadeResult {
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(doc);
    raikiri_style::cascade(doc, &rules).expect("cascade")
}

#[test]
fn a_text_node_owns_the_lines_it_shows_up_on() {
    // Ahem at 10px, width 40: "aaaa", "bbbb", "cccc" are one line each.
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:40px");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[root].children[0];
    let lines = doc
        .ifc_text_lines(text)
        .expect("an ifc text node has lines");
    assert_eq!(lines.root, root);
    assert_eq!(lines.width, 40.0);
    assert_eq!(
        lines
            .lines
            .iter()
            .map(|l| (l.line, l.top, l.bottom))
            .collect::<Vec<_>>(),
        [(0, 0.0, 10.0), (1, 10.0, 20.0), (2, 20.0, 30.0)]
    );
}

#[test]
fn a_line_shared_by_two_text_nodes_belongs_to_both() {
    // "aa" + <span>"bb"</span> + "cc" fit on one 60px line.
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:60px");
    let first = doc.append_text(root, "aa");
    let span = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline"),
    );
    let inner = doc.append_text(span, "bb");
    let last = doc.append_text(root, "cc");
    let cascade = recascade(&mut doc);
    lay_out(&mut doc, &cascade);
    for id in [first, inner, last] {
        let lines = doc.ifc_text_lines(id).expect("lines");
        assert_eq!(lines.lines.len(), 1, "node {id}");
        assert_eq!((lines.lines[0].top, lines.lines[0].bottom), (0.0, 10.0));
    }
}

#[test]
fn a_node_outside_an_ifc_paragraph_has_no_lines() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa", "width:40px");
    // Switch off: the text node is shaped by parley.
    layout_single_page(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
        .expect("layout");
    let text = doc.nodes[root].children[0];
    assert!(doc.ifc_text_lines(text).is_none());
    assert!(
        doc.ifc_text_lines(root).is_none(),
        "the root is not a text node"
    );
}

#[test]
fn an_inline_element_is_not_a_text_node() {
    // The span carries IN_IFC_SUBTREE but is not text, so it has no lines.
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:60px");
    doc.append_text(root, "aa ");
    let span = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline"),
    );
    doc.append_text(span, "bb");
    let cascade = recascade(&mut doc);
    lay_out(&mut doc, &cascade);
    assert!(doc.ifc_text_lines(span).is_none());
}

#[test]
fn a_line_belongs_only_to_the_text_nodes_that_have_glyphs_on_it() {
    // "aaaa " fills the 40px line, so the span's "bbbb" wraps to line 2.
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:40px");
    let first = doc.append_text(root, "aaaa ");
    let span = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline"),
    );
    let inner = doc.append_text(span, "bbbb");
    let cascade = recascade(&mut doc);
    lay_out(&mut doc, &cascade);
    let owned = |id: usize| {
        doc.ifc_text_lines(id)
            .expect("lines")
            .lines
            .iter()
            .map(|l| (l.line, l.top, l.bottom))
            .collect::<Vec<_>>()
    };
    assert_eq!(owned(first), [(0, 0.0, 10.0)]);
    assert_eq!(owned(inner), [(1, 10.0, 20.0)]);
}

#[test]
fn a_whitespace_only_text_node_between_words_shares_the_line() {
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:80px");
    let a = doc.append_text(root, "aa");
    let gap = doc.append_text(root, " ");
    let b = doc.append_text(root, "bb");
    let cascade = recascade(&mut doc);
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.ifc_text_lines(a).map(|l| l.lines.len()), Some(1));
    assert_eq!(doc.ifc_text_lines(b).map(|l| l.lines.len()), Some(1));
    // CSS keeps the single space between the words; the engine emits it as a
    // glyph of the node it came from, so that node owns the line.
    assert_eq!(doc.ifc_text_lines(gap).map(|l| l.lines.len()), Some(1));
}

#[test]
fn a_text_node_whose_spaces_collapse_away_has_no_lines() {
    // Spaces at the start of a line are removed, so the leading text node
    // has no glyph on any line.
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:80px");
    let leading = doc.append_text(root, "   ");
    let word = doc.append_text(root, "aa");
    let cascade = recascade(&mut doc);
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.ifc_text_lines(word).map(|l| l.lines.len()), Some(1));
    assert_eq!(doc.ifc_text_lines(leading), None);
}

#[test]
fn the_lines_follow_a_rebreak_for_another_width() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[root].children[0];
    assert_eq!(doc.ifc_text_lines(text).map(|l| l.lines.len()), Some(1));
    // No authored width anywhere: parley re-shapes at max_advance, and so does
    // the inline engine.
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    assert_eq!(doc.ifc_text_lines(text).map(|l| l.lines.len()), Some(3));
    relayout_text_for_width(&mut doc, &cascade, 800.0, 800.0, ahem_font_context());
    assert_eq!(doc.ifc_text_lines(text).map(|l| l.lines.len()), Some(1));
}
