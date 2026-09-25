//! `Node`, `Element`, and `Document` members.

use boa_engine::{Context, JsResult, JsValue};
use raikiri_dom::NodeKind;

use super::interfaces::Members;
use super::webidl::{this_node, with_state};

/// `Node.nodeType` (DOM §4.4).
fn node_type(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let kind = with_state(context, |s| {
        s.host.document().get_node(index).map(|n| n.kind())
    })?;
    Ok(JsValue::from(match kind {
        Some(NodeKind::Element) => 1,
        Some(NodeKind::Text) => 3,
        Some(NodeKind::ProcessingInstruction) => 7,
        Some(NodeKind::Comment) => 8,
        Some(NodeKind::Document) => 9,
        Some(NodeKind::DocumentFragment) => 11,
        _ => 0,
    }))
}

pub(crate) const NODE_MEMBERS: Members = Members {
    getters: &[("nodeType", node_type)],
    accessors: &[],
    methods: &[],
};
pub(crate) const ELEMENT_MEMBERS: Members = super::interfaces::NO_MEMBERS;
pub(crate) const DOCUMENT_MEMBERS: Members = super::interfaces::NO_MEMBERS;
