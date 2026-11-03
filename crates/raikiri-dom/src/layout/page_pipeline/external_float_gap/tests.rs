use super::*;
use crate::layout::test_support::with_ahem;

fn finish(doc: &mut Document) -> CascadeResult {
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(doc, &raikiri_style::build_rule_tree(doc)).unwrap();
    doc.set_font_collection_with_limits(
        crate::layout::test_support::ifc_ahem_fonts(),
        shodo::limits::Limits::default(),
    );
    layout_single_page(with_ahem(doc), &cascade, PageBox::A4).unwrap();
    cascade
}

#[test]
fn retained_float_ends_are_read_once_when_many_roots_share_a_moving_holder() {
    for count in [8, 32] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block"));
        let holder = doc.append_element(
            Some(body),
            "section",
            Some("display:block;width:40px;min-height:60px"),
        );
        for _ in 0..count {
            doc.append_element(
                Some(holder),
                "div",
                Some("display:block;float:left;width:40px;height:10px"),
            );
            let root = doc.append_element(Some(holder), "div", Some("display:block;width:40px;max-height:0;font-family:Ahem;font-size:10px;line-height:10px"));
            doc.append_text(root, "Y");
        }
        let cascade = finish(&mut doc);
        let mut session = crate::layout::context::LayoutSession::resumed(&mut doc, &cascade);
        let mut index = Index::default();
        let consumed = index.discard_in_holder(&mut session, &cascade, holder, 40.0, body);
        assert!(consumed.is_empty());
        assert_eq!(index.roots.len(), count);
        assert_eq!(index.float_reads, count);
    }
}

#[test]
fn absolute_children_that_need_resizing_reject_uniform_gap_removal() {
    for insets in ["top:0;bottom:0", "top:0;height:100%", "top:10px;bottom:5px"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block"));
        doc.append_element(
            Some(body),
            "div",
            Some("display:block;float:left;width:40px;height:20px"),
        );
        let root = doc.append_element(Some(body), "div", Some("display:block;position:relative;width:40px;font-family:Ahem;font-size:20px;line-height:30px"));
        doc.append_text(root, "YY");
        let child = doc.append_element(
            Some(root),
            "div",
            Some(&format!(
                "display:block;position:absolute;left:50px;width:10px;{insets}"
            )),
        );
        let cascade = finish(&mut doc);
        let before = (
            doc.nodes[root].layout.size.height,
            doc.nodes[child].layout.size.height,
        );
        assert_eq!(before.0, 50.0);
        let mut session = crate::layout::context::LayoutSession::resumed(&mut doc, &cascade);
        let mut index = Index::default();
        let consumed = index.discard_in_holder(&mut session, &cascade, root, 40.0, body);
        assert!(consumed.is_empty());
        assert_eq!(
            (
                session.nodes[root].layout.size.height,
                session.nodes[child].layout.size.height
            ),
            before
        );
    }
}

#[test]
fn owners_with_flow_extent_but_no_candidate_do_not_offer_a_removable_prefix() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    doc.append_element(
        Some(body),
        "div",
        Some("display:block;float:left;width:40px;height:20px"),
    );
    let root = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:40px;font-family:Ahem;font-size:20px;line-height:30px"),
    );
    doc.append_text(root, "Y");
    let cascade = finish(&mut doc);
    let height = doc.nodes[root].layout.size.height;
    let mut session = crate::layout::context::LayoutSession::resumed(&mut doc, &cascade);
    let mut index = Index::default();
    index.prepare(&mut session, &cascade, body);
    assert_eq!(index.roots.len(), 1);
    index.assign_visits(&mut session, &[]);
    assert!(index.roots.is_empty());
    let node = session.ifc_layout_node(root).unwrap();
    assert_eq!(
        node.ifc
            .as_ref()
            .unwrap()
            .lines
            .as_ref()
            .unwrap()
            .external_float_gap,
        0.0
    );
    assert_eq!(session.nodes[root].layout.size.height, height);
}

#[test]
fn descendant_candidates_schedule_their_owners_in_visit_order() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    doc.append_element(
        Some(body),
        "div",
        Some("display:block;float:left;width:40px;height:20px"),
    );
    let mut owners = Vec::new();
    for word in ["Y", "Z"] {
        let root = doc.append_element(Some(body), "div", Some("display:block;width:40px;height:0;font-family:Ahem;font-size:20px;line-height:30px"));
        let text = doc.append_text(root, word);
        owners.push((root, text));
    }
    let cascade = finish(&mut doc);
    let candidates: Vec<_> = owners
        .iter()
        .rev()
        .map(|&(root, text)| PageCandidate {
            node_id: text,
            raw_y: 0.0,
            height: 30.0,
            context: candidates::BoxContext::text(candidates::CandidateFlowContext::default()),
            page_name: None,
            named_flex_context: None,
            ifc_root: Some((root, 30.0)),
        })
        .collect();
    let mut session = crate::layout::context::LayoutSession::resumed(&mut doc, &cascade);
    let mut index = Index::default();
    index.prepare(&mut session, &cascade, body);
    assert_eq!(index.roots.len(), 2);
    index.assign_visits(&mut session, &candidates);
    assert_eq!(index.first_visit(owners[0].0), Some(1));
    assert_eq!(index.first_visit(owners[1].0), Some(0));
}
