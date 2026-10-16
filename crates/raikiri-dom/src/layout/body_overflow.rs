//! CSS body overflow propagation to the viewport.

use super::*;

/// When the root overflow is visible, propagate body overflow to the viewport
/// and give the body its visible used overflow value (CSS Overflow 3 §3.3).
pub(crate) fn propagate_body_overflow_to_viewport(
    doc: &mut Document,
    cascade: &CascadeResult,
    body_id: usize,
) {
    let Some(html_id) = doc.parent_of(body_id) else {
        return; // cov:ignore: layout finds the body below the document root, so it has a parent.
    };
    // Only a `body` propagates its overflow, not a `frameset` (CSS Overflow
    // 3, 3.3).
    if doc.nodes[html_id].tag_name() != Some("html")
        || doc.nodes[body_id].tag_name() != Some("body")
    {
        return;
    }
    let html = cascade.computed[html_id].overflow;
    if html.x == OverflowValue::Visible && html.y == OverflowValue::Visible {
        doc.nodes[body_id].style.overflow = Point {
            x: TaffyOverflow::Visible,
            y: TaffyOverflow::Visible,
        };
    }
}

#[cfg(test)]
mod tests;
