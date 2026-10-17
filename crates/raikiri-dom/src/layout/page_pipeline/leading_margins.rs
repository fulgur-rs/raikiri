//! Truncate leading margins using the positions produced by normal block layout.

use super::*;
use crate::layout::context::LayoutSession;
use taffy::TraversePartialTree;

/// Truncate the root and descendant leading margins for one pagination pass.
pub(super) fn truncate_flow(
    document: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    pages: &PageOrigins,
    control: &PageLayoutControl<'_>,
) -> Result<bool, LayoutError> {
    let root_margin = document.nodes[root].unrounded_layout.margin.top;
    let root_margin_page = pages.page_index_for_y(root_margin);
    if root_margin_page > 0 {
        control.check_discovery_page_index(root_margin_page)?;
        document.nodes[root].unrounded_layout.location.y -=
            root_margin - pages.origin(root_margin_page);
    }

    Ok(truncate(document, cascade, root, pages, control)? || root_margin_page > 0)
}

/// Truncate the leading margin of a flow child when it crosses a page edge.
/// The parent's content edge and the child's used location already include
/// ordinary margin collapse; no CSS margin set is recomputed here.
pub(super) fn truncate(
    document: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    pages: &PageOrigins,
    control: &PageLayoutControl<'_>,
) -> Result<bool, LayoutError> {
    let root_flow_y = current_abs_y(document, root)
        - accumulated_relative_offset_y(document, cascade, root, root);
    let mut stack = vec![(root, root_flow_y, true)];
    let mut moved = false;
    while let Some((parent, flow_y, leading_flow)) = stack.pop() {
        control.check_aborted()?;
        let parent_node = &document.nodes[parent];
        let block_flow = parent_node.formatting_context == FormattingContext::Block
            && parent_node.style.align_content == taffy::AlignContent::NORMAL
            && (matches!(
                parent_node.display(),
                DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
            ) || parent_node.flags.contains(NodeFlags::IS_ROOT_ELEMENT));
        // The rows an anonymous table wraps are placed relative to the
        // table's parent, which holds them in the DOM.
        let content_y = if document.is_anonymous_table(parent) {
            flow_y - parent_node.unrounded_layout.location.y
        } else {
            flow_y
                + parent_node.unrounded_layout.border.top
                + parent_node.unrounded_layout.padding.top
        };
        let children: Vec<usize> = document
            .child_ids(TaffyNodeId::from(parent))
            .map(usize::from)
            .collect();
        let mut first_in_flow = true;
        let mut preceding_float = false;
        for child in children {
            let node = &document.nodes[child];
            if node.is_display_none() || node.is_non_rendered_html_element() {
                continue;
            }
            if node.style.float != TaffyFloat::None {
                preceding_float = true;
                continue;
            }
            if node.taffy_position() == TaffyPosition::Absolute {
                continue;
            }
            let relative = if document.is_source_node(child) {
                layout_relative_offset_y(document, cascade, root, child)
            } else {
                0.0
            };
            let mut child_y = flow_y + node.unrounded_layout.location.y - relative;
            let page = pages.page_index_for_y(child_y);
            let leading_child = leading_flow && first_in_flow && !preceding_float;
            if block_flow
                && leading_child
                && node.style.clear == TaffyClear::None
                && child_y > content_y
                && page > pages.page_index_for_y(content_y)
                && child_y > pages.origin(page) + 0.001
            {
                control.check_discovery_page_index(page)?;
                let delta = pages.origin(page) - child_y;
                document.nodes[child].unrounded_layout.location.y += delta;
                follow_moved_ifc_block(document, child, delta);
                shrink_ancestors_after_margin_truncation(document, cascade, child, delta, root);
                child_y += delta;
                moved = true;
            }
            first_in_flow = false;
            stack.push((child, child_y, leading_child));
        }
    }
    Ok(moved)
}
