use super::super::*;

#[test]
fn node_flags_default_is_empty() {
    let f = NodeFlags::default();
    assert!(!f.contains(NodeFlags::IS_IN_DOCUMENT));
}

#[test]
fn node_new_document_has_is_in_document_set_by_default() {
    // Node::new_document() は Document root 用、常に flat tree の一員。
    let n = Node::new_document();
    assert!(n.is_in_document());
}

#[test]
fn node_new_element_has_is_in_document_set_by_default() {
    let n = Node::new_element(SmolStr::new("p"), taffy::Style::default(), None);
    assert!(n.is_in_document());
}

#[test]
fn node_new_text_has_is_in_document_set_by_default() {
    let n = Node::new_text(SmolStr::new("hi"));
    assert!(n.is_in_document());
}

#[test]
fn set_in_document_toggles_bit() {
    let mut n = Node::new_document();
    n.set_in_document(false);
    assert!(!n.is_in_document());
    n.set_in_document(true);
    assert!(n.is_in_document());
}

#[test]
fn node_new_comment_kind_and_default_flag_state() {
    // Comment constructor は kind = NodeKind::Comment、
    // IS_IN_DOCUMENT は default true (mark_in_document_flags で後段 clear
    // される optimistic 初期値、Element / Text と同じ posture)。
    let n = Node::new_comment(SmolStr::new("hello"));
    assert_eq!(n.kind(), NodeKind::Comment);
    assert!(
        n.is_in_document(),
        "constructor default follows Element/Text pattern"
    );
    // tag_name accessor は Comment に対して None を返す (Element でない)。
    assert_eq!(n.tag_name(), None);
    // text_layout accessor は Comment に対して None を返す (Text でない)。
    assert!(n.text_layout().is_none());
}

#[test]
fn node_new_processing_instruction_kind_and_default_flag_state() {
    let n = Node::new_processing_instruction(
        SmolStr::new("xml-stylesheet"),
        SmolStr::new("href='x.css'"),
    );
    assert_eq!(n.kind(), NodeKind::ProcessingInstruction);
    assert!(n.is_in_document());
    assert_eq!(n.tag_name(), None);
    assert!(n.text_layout().is_none());
}

#[test]
fn node_new_document_fragment_kind_and_default_flag_state() {
    let n = Node::new_document_fragment();
    assert_eq!(n.kind(), NodeKind::DocumentFragment);
    // Fragment root は使用時 detached 状態で作られるため、mark 後に
    // false に落ちる。constructor 単体では default true。
    assert!(n.is_in_document());
    assert_eq!(n.tag_name(), None);
    assert!(n.text_layout().is_none());
}

#[test]
fn node_flags_bit_values_match_blitz_raw() {
    // Regression pin。blitz `NodeFlags`
    // (blitz-dom/src/node/node.rs:50-58) と raw bit 値まで一致:
    //   IS_INLINE_ROOT = 0b001, IS_TABLE_ROOT = 0b010, IS_IN_DOCUMENT = 0b100
    //
    // これにより将来の blitz-compat の変換が `NodeFlags::from_bits(x)` の
    // trivial cast で成立する。将来 bit を追加する際は blitz と同 bit
    // 位置に揃えること。
    assert_eq!(NodeFlags::IS_INLINE_ROOT.bits(), 0b001);
    assert_eq!(NodeFlags::IS_TABLE_ROOT.bits(), 0b010);
    assert_eq!(NodeFlags::IS_IN_DOCUMENT.bits(), 0b100);
}
