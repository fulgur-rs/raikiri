use super::*;

#[test]
fn preserves_inheritance_nullable_results_and_required_arguments() {
    let interfaces = parse(include_str!("../idl/dom.webidl")).unwrap();
    assert_eq!(interfaces.len(), 2);
    assert_eq!(interfaces[1].parent.as_deref(), Some("Node"));
    assert_eq!(interfaces[0].members[1].result, Ty::NullableNode);
    assert_eq!(interfaces[0].members[2].arguments, vec![Ty::NullableNode]);
    assert_eq!(interfaces[1].members[0].result, Ty::NullableString);
    assert_eq!(
        interfaces[1].members[1].arguments,
        vec![Ty::String, Ty::String]
    );
}

#[test]
fn rejects_unsupported_features_instead_of_dropping_them() {
    for source in [
        "[Exposed=Worker] interface Node {};",
        "[Exposed=Window] interface Node { undefined f(optional DOMString s); };",
        "[Exposed=Window] interface Node { undefined f(DOMString... s); };",
        "[Exposed=Window] interface Node { undefined f(); undefined f(DOMString s); };",
        "[Exposed=Window] interface Node { attribute DOMString value; };",
        "[Exposed=Window] interface Node { readonly attribute DOMString value; };",
        "[Exposed=Window] interface Node { [SameObject] readonly attribute Node? parentNode; };",
        "[Exposed=Window] interface Node {}; garbage",
        "[Exposed=Window] interface Node : Unknown {};",
        "[Exposed=Window] interface Node {}; [Exposed=Window] interface Node {};",
        "dictionary Options {};",
    ] {
        assert!(
            parse(source).is_err(),
            "unsupported IDL was accepted: {source}"
        );
    }
}

#[test]
fn converts_camel_case_native_operation_names() {
    assert_eq!(snake("setAttribute"), "set_attribute");
    assert_eq!(snake("isSameNode"), "is_same_node");
}
