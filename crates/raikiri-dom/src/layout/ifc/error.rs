//! Errors produced while turning raikiri DOM state into shodo input.

use std::fmt;

/// Why a subtree could not be handed to shodo.
#[derive(Clone, Debug)]
pub(crate) enum IfcError {
    /// The node id is out of range or the node is not part of the document.
    InvalidNode(usize),
    /// The subtree uses something the inline path does not map yet. It is
    /// rejected instead of being approximated.
    Unsupported {
        /// Node the unsupported input belongs to.
        node: usize,
        /// What is not supported.
        reason: &'static str,
    },
    /// A shodo resource limit was exceeded while building the paragraph.
    Limit(shodo::limits::LimitExceeded),
}

impl fmt::Display for IfcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IfcError::InvalidNode(node) => write!(f, "node {node} is not part of the document"),
            IfcError::Unsupported { node, reason } => {
                write!(
                    f,
                    "node {node} is not supported by the inline path: {reason}"
                )
            }
            IfcError::Limit(limit) => write!(f, "shodo limit exceeded: {limit}"),
        }
    }
}

impl std::error::Error for IfcError {}

#[cfg(test)]
mod tests;
