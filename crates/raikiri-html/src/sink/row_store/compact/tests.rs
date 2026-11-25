use super::*;

#[test]
fn live_reference_to_a_closed_row_cancels_compaction_before_mutating_the_arena() {
    let sink = CompactSink(RaikiriTreeSink::new(None));
    let state = sink.capture_table_rows().unwrap();
    let row = sink.create_element(
        QualName::new(None, ns!(html), "tr".into()),
        vec![],
        Default::default(),
    );
    let root = sink.get_document();
    sink.append(&root, NodeOrText::AppendNode(row.clone()));
    {
        let RowCapture::Recording(store) = &mut *state.borrow_mut() else {
            panic!("capture disabled")
        };
        store.closed.push(row.get());
    }
    let before = row.get();
    compact(&sink, &state, vec![row.clone()]);
    assert!(matches!(*state.borrow(), RowCapture::Unsupported));
    compact(&sink, &state, vec![row.clone()]);
    assert_eq!(row.get(), before);
    assert_eq!(sink.document.borrow().parent_of(before), Some(root.get()));
}

#[test]
fn parser_can_insert_a_processing_instruction_before_a_retained_sibling() {
    let sink = CompactSink(RaikiriTreeSink::new(None));
    let root = sink.get_document();
    let sibling = sink.create_comment("after".into());
    sink.append(&root, NodeOrText::AppendNode(sibling.clone()));
    let instruction = sink.create_pi("target".into(), "data".into());
    sink.append_before_sibling(&sibling, NodeOrText::AppendNode(instruction.clone()));
    let dom = sink.document.borrow();
    assert_eq!(
        dom.get_node(root.get()).unwrap().children.as_slice(),
        &[instruction.get(), sibling.get()]
    );
    assert_eq!(dom.parent_of(instruction.get()), Some(root.get()));
}
