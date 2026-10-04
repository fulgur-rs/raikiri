use super::super::*;

#[test]
fn non_element_animation_style_source_is_absent() {
    let mut document = Document::new();
    let text = document.append_text(document.root_index(), "text");
    let node = document.get_node(text).expect("text node exists");
    let element = ElementRef { node };

    assert_eq!(element.animation_style_source(), None);
}
