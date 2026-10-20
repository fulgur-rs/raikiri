//! CSS body overflow propagation to the viewport.

use super::*;

/// The element supplying viewport overflow: the document element, or the
/// first body child of an HTML root whose overflow is visible in both axes.
pub(crate) fn viewport_overflow_source(doc: &Document, cascade: &CascadeResult) -> Option<usize> {
    let root = doc.document_element_index()?;
    let values = cascade.computed.get(root)?;
    if values.display == DisplayValue::None {
        return None;
    }
    if doc.nodes[root].tag_name() == Some("html")
        && values.overflow.x == OverflowValue::Visible
        && values.overflow.y == OverflowValue::Visible
        && let Some(body) = find_body(doc)
        && doc.parent_of(body) == Some(root)
        && doc.nodes[body].tag_name() == Some("body")
        && cascade.computed[body].display != DisplayValue::None
    {
        return Some(body);
    }
    Some(root)
}

/// When the root overflow is visible, propagate body overflow to the viewport
/// and give the body its visible used overflow value (CSS Overflow 3 §3.3).
/// Return whether the body's used value was installed.
pub(crate) fn propagate_body_overflow_to_viewport(
    doc: &mut Document,
    cascade: &CascadeResult,
    body_id: usize,
) -> bool {
    if viewport_overflow_source(doc, cascade) == Some(body_id)
        && doc.parent_of(body_id) == doc.document_element_index()
        && doc.nodes[body_id].tag_name() == Some("body")
    {
        doc.layout_node_with_style_mut(body_id)
            .unwrap()
            .style
            .overflow = Point {
            x: TaffyOverflow::Visible,
            y: TaffyOverflow::Visible,
        };
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests;
