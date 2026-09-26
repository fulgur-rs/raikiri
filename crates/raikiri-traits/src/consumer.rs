//! Neutral resolved consumer-property events.

use crate::NodeId;

/// Owned value delivered for a registered consumer property.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConsumerPropertyValue {
    /// A signed CSS integer.
    Integer(i32),
    /// A resolved text value owned by the event.
    Text(String),
    /// The registered grammar's explicit `none` keyword.
    None,
}

/// One resolved consumer-property declaration in document order.
///
/// The event contains only neutral data.  `node_id` and `parent_id` can be
/// joined with fragments by source identity without exposing renderer-specific
/// objects.  `source_order` is the producer's
/// preorder index among source nodes and is stable for one parsed document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConsumerPropertyEvent {
    /// Opaque source node identity.
    pub node_id: NodeId,
    /// Opaque source parent identity. Top-level nodes use the document-root
    /// identity; the root itself is not emitted as a property event.
    pub parent_id: Option<NodeId>,
    /// Stable preorder source-node index. In-document text and comment nodes
    /// also consume indices, even though only element nodes can emit events.
    pub source_order: u32,
    /// Registered property name without a leading `--`.
    pub property_name: String,
    /// Owned resolved value.
    pub value: ConsumerPropertyValue,
}

impl ConsumerPropertyEvent {
    /// Construct an owned neutral property event.
    pub fn new(
        node_id: NodeId,
        parent_id: Option<NodeId>,
        source_order: u32,
        property_name: impl Into<String>,
        value: ConsumerPropertyValue,
    ) -> Self {
        Self {
            node_id,
            parent_id,
            source_order,
            property_name: property_name.into(),
            value,
        }
    }
}

/// Optional receiver for resolved consumer-owned properties.
///
/// The render driver invokes `observe_event` in deterministic document order.
// cov:ignore: observer trait declaration has no executable body
pub trait ConsumerPropertyObserver: Send {
    /// Receive one resolved property event.
    fn observe_event(&mut self, event: ConsumerPropertyEvent) -> std::io::Result<()>;
}

impl<F> ConsumerPropertyObserver for F
where
    F: FnMut(ConsumerPropertyEvent) -> std::io::Result<()> + Send,
{
    fn observe_event(&mut self, event: ConsumerPropertyEvent) -> std::io::Result<()> {
        self(event)
    }
}
