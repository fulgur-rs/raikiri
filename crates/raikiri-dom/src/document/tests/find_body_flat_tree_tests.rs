// Regression check: find_body (both layout and paint impls)
// must not select a <body> that lives inside an inert subtree
// (<template>...<body>ghost</body>...</template>).
use super::*;
use crate::layout::find_body as layout_find_body;

#[test]
fn layout_find_body_skips_body_inside_template() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let html = doc.append_element(Some(root), "html", Style::default(), None::<&str>);
    // Ghost body in the template's detached contents fragment (should NOT be
    // picked). The fragment is unreachable from the Document root, matching
    // the parser's shape; an ordinary light-DOM child of the template
    // element would stay in-document instead.
    let tmpl = doc.append_element(Some(html), "template", Style::default(), None::<&str>);
    let frag = doc.allocate_template_fragment_root(tmpl);
    let _ghost = doc.append_element(Some(frag), "body", Style::default(), None::<&str>);
    // Real body under html (should be picked)
    let real = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    doc.mark_in_document_flags();

    assert_eq!(
        layout_find_body(&doc),
        Some(real),
        "find_body must skip inert body inside <template> and select the real body"
    );
}
