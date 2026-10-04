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

#[test]
fn counter_snapshot_limit_names_the_failed_budget() {
    let error: IfcError = crate::target::CounterSnapshotLimitExceeded {
        limit: 32,
        actual: 33,
    }
    .into();
    assert_eq!(
        error.to_string(),
        "counter snapshot memory limit exceeded: 33 bytes (limit 32)"
    );
}
