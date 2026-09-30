use super::*;

#[test]
fn unsupported_names_the_node_and_the_reason() {
    let error = IfcError::Unsupported {
        node: 7,
        reason: "float",
    };
    assert_eq!(
        error.to_string(),
        "node 7 is not supported by the inline path: float"
    );
}

#[test]
fn invalid_node_names_the_node() {
    assert_eq!(
        IfcError::InvalidNode(3).to_string(),
        "node 3 is not part of the document"
    );
}
