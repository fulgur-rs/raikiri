use super::*;

#[test]
fn fragment_root_with_zero_or_multiple_children_is_not_flattened() {
    let mut empty = raikiri_dom::Document::new();
    flatten_fragment_root(&mut empty);
    assert!(
        empty
            .get_node(empty.root_index())
            .unwrap()
            .children
            .is_empty()
    );

    let mut multiple = raikiri_dom::Document::new();
    let root = multiple.root_index();
    let first = multiple.append_element(Some(root), "div", Default::default(), None::<&str>);
    let second = multiple.append_element(Some(root), "span", Default::default(), None::<&str>);
    flatten_fragment_root(&mut multiple);
    assert_eq!(
        multiple.get_node(root).unwrap().children,
        vec![first, second]
    );
}

#[test]
fn fragment_root_with_non_html_or_foreign_html_child_is_not_flattened() {
    let mut non_html = raikiri_dom::Document::new();
    let root = non_html.root_index();
    let div = non_html.append_element(Some(root), "div", Default::default(), None::<&str>);
    flatten_fragment_root(&mut non_html);
    assert_eq!(non_html.get_node(root).unwrap().children, vec![div]);

    let mut foreign_html = raikiri_dom::Document::new();
    let root = foreign_html.root_index();
    let html = foreign_html.append_element(Some(root), "html", Default::default(), None::<&str>);
    foreign_html.set_element_namespace(html, Some("urn:foreign".into()));
    let child = foreign_html.append_element(Some(html), "g", Default::default(), None::<&str>);
    flatten_fragment_root(&mut foreign_html);
    assert_eq!(foreign_html.get_node(root).unwrap().children, vec![html]);
    assert_eq!(foreign_html.get_node(html).unwrap().children, vec![child]);
}
