use super::*;

#[test]
fn body_candidates_include_detached_case_and_namespace_variants() {
    for namespace in [ns!(html), ns!(svg), ns!()] {
        for local in ["body", "BODY", "BoDy", "frameset", "FRAMESET", "FrameSet"] {
            let sink = RaikiriTreeSink::default();
            assert!(!sink.has_created_body_candidate());
            sink.make_element(
                QualName::new(None, namespace.clone(), "head".into()),
                vec![],
            );
            assert!(!sink.has_created_body_candidate());
            let id =
                sink.make_element(QualName::new(None, namespace.clone(), local.into()), vec![]);
            assert!(sink.has_created_body_candidate());
            let document = sink.document.borrow();
            assert!(
                !document
                    .preorder(document.root_index())
                    .any(|node| node == id)
            );
            drop(document);
            sink.make_element(QualName::new(None, namespace.clone(), "div".into()), vec![]);
            assert!(sink.has_created_body_candidate());
        }
    }
}

#[test]
fn a_rejected_body_allocation_does_not_set_the_candidate_flag() {
    let sink = RaikiriTreeSink::with_limits(None, Some(1));
    let id = sink.make_element(QualName::new(None, ns!(html), "body".into()), vec![]);
    assert_eq!(id, OVERFLOW_HANDLE);
    assert!(!sink.has_created_body_candidate());
    assert_eq!(sink.dom_node_limit_error(), Some((1, 2)));
}
