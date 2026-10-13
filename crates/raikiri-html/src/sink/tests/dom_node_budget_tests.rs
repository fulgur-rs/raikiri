use super::super::{OVERFLOW_HANDLE, RaikiriTreeSink, TextTarget};
use html5ever::interface::{ElementFlags, QualName, TreeSink};
use html5ever::tendril::StrTendril;
use markup5ever::{LocalName, ns};

fn element_name(local_name: &str) -> QualName {
    QualName::new(None, ns!(html), LocalName::from(local_name))
}

#[test]
fn comment_node_limit_is_charged_before_allocation() {
    let sink = RaikiriTreeSink::with_limits(None, Some(1));

    let handle = TreeSink::create_comment(&sink, StrTendril::from("comment"));

    assert_eq!(handle, OVERFLOW_HANDLE);
    assert_eq!(sink.document.borrow().node_count(), 1);
    assert_eq!(sink.dom_node_limit_error(), Some((1, 2)));
}

#[test]
fn processing_instruction_node_limit_is_charged_before_allocation() {
    let sink = RaikiriTreeSink::with_limits(None, Some(1));

    let handle = TreeSink::create_pi(&sink, StrTendril::from("target"), StrTendril::from("data"));

    assert_eq!(handle, OVERFLOW_HANDLE);
    assert_eq!(sink.document.borrow().node_count(), 1);
    assert_eq!(sink.dom_node_limit_error(), Some((1, 2)));
}

#[test]
fn template_fragment_root_uses_the_dom_node_budget() {
    let sink = RaikiriTreeSink::with_limits(None, Some(2));
    let mut flags = ElementFlags::default();
    flags.template = true;

    let template = TreeSink::create_element(&sink, element_name("template"), Vec::new(), flags);

    assert_eq!(template, 1);
    assert_eq!(sink.document.borrow().node_count(), 2);
    assert_eq!(sink.dom_node_limit_error(), Some((2, 3)));
}

#[test]
fn text_buffer_reserves_a_node_before_copying_character_data() {
    let sink = RaikiriTreeSink::with_limits(None, Some(1));

    sink.buffer_text(TextTarget::Append(0), "text");

    assert!(sink.pending_text.borrow().is_none());
    assert_eq!(sink.document.borrow().node_count(), 1);
    assert_eq!(sink.dom_node_limit_error(), Some((1, 2)));
}
