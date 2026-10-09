//! Producer-owned column rule geometry shared by native and page painters.

use crate::Document;
use crate::fragment::FragmentationContext;
use raikiri_style::property::{BorderColor, BorderStyle, CssColor, PositionValue};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::{LayoutError, NodeId, PaintClip, PaintRect};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// One column rule with producer-resolved geometry and used paint values.
///
/// Page events use page-local CSS pixels. The native layout seam returns a
/// rectangle relative to the owning container's border-box origin.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ColumnRule {
    /// Multicolumn container owning the rule and its clip/opacity context.
    pub owner: NodeId,
    /// Physical rule rectangle; the rule consumes no layout space.
    pub rect: PaintRect,
    /// Used style, including the collapsed-border mapping of inset/outset.
    pub style: BorderStyle,
    /// Used color, with `currentcolor` resolved against the owning container.
    pub color: CssColor,
    /// Block origin of the complete rule before page slicing, in the same space as `rect`.
    pub pattern_origin: f32,
    /// Complete rule height, preserving the pattern across page slices.
    pub pattern_height: f32,
    pub(crate) overflow_chain: Option<usize>,
    pub(crate) fragmentainer_clip: Option<PaintClip>,
}

impl ColumnRule {
    pub(crate) fn new(owner: usize, rect: PaintRect, values: &ComputedValues) -> Self {
        Self {
            owner: NodeId::new(owner as u64),
            pattern_origin: rect.y,
            pattern_height: rect.height,
            overflow_chain: None,
            fragmentainer_clip: None,
            rect,
            style: match values.column_rule.style() {
                BorderStyle::Inset => BorderStyle::Ridge,
                BorderStyle::Outset => BorderStyle::Groove,
                style => style,
            },
            color: match values.column_rule.color {
                BorderColor::Resolved(color) => color,
                _ => values.color,
            },
        }
    }
}

struct Group {
    origin: (f32, f32),
    height: f32,
    context: FragmentationContext,
    occupied: BTreeSet<usize>,
}

/// Retain rules from the final column placements without reshaping or balancing.
/// Source scans and retained rectangles share the layout work cap.
pub(crate) fn prepare(
    document: &Document,
    cascade: &CascadeResult,
) -> Result<BTreeMap<usize, Vec<PaintRect>>, LayoutError> {
    if !document.nodes.iter().enumerate().any(|(index, node)| {
        node.multicol.is_some() && cascade.computed[index].column_rule.width().px() > 0.0
    }) {
        return Ok(BTreeMap::new());
    }
    let limit = document.fragment_tree.limit;
    let mut remaining = limit.saturating_sub(document.fragment_tree.break_flow_work_used);
    let mut charge = |amount: usize| -> Result<(), LayoutError> {
        remaining = remaining
            .checked_sub(amount)
            .ok_or(LayoutError::FragmentLimitExceeded { limit })?;
        Ok(())
    };
    charge(
        document
            .nodes
            .len()
            .saturating_mul(2)
            .saturating_add(document.fragment_tree.fragments.len()),
    )?;
    let mut groups = BTreeMap::<usize, Group>::new();
    let mut parent_owners = vec![None; document.nodes.len()];
    let fragmented: HashSet<_> = document
        .fragment_tree
        .fragments
        .iter()
        .map(|fragment| fragment.node_id)
        .collect();
    let mut stack = vec![(document.root, None)];
    while let Some((node_id, parent_owner)) = stack.pop() {
        let node = &document.nodes[node_id];
        if !node.is_in_document() || node.is_display_none() {
            continue;
        }
        let values = &cascade.computed[node_id];
        let out_of_flow = matches!(
            values.position,
            PositionValue::Absolute | PositionValue::Fixed
        );
        let parent_owner = if out_of_flow { None } else { parent_owner };
        parent_owners[node_id] = parent_owner;
        let layout = node.unrounded_layout;
        let origin = (
            layout.border.left + layout.padding.left,
            layout.border.top + layout.padding.top,
        );
        let content_width =
            (layout.size.width - origin.0 - layout.border.right - layout.padding.right).max(0.0);
        let height =
            (layout.size.height - origin.1 - layout.border.bottom - layout.padding.bottom).max(0.0);
        let owner = if let Some(style) = node.multicol {
            if style.horizontal
                && values.column_rule.width().px() > 0.0
                && height > 0.0
                && let Some(context) =
                    FragmentationContext::resolve(content_width, Some(height), style)
            {
                groups.insert(
                    node_id,
                    Group {
                        origin,
                        height,
                        context,
                        occupied: BTreeSet::new(),
                    },
                );
                Some(node_id)
            } else {
                None
            }
        } else {
            parent_owner
        };
        if let Some(group) = owner.and_then(|owner| groups.get_mut(&owner))
            && let Some(ranges) = node.ifc_multicol_fragments()
        {
            charge(ranges.len())?;
            for range in ranges
                .iter()
                .filter(|range| range.line_start < range.line_end)
            {
                group.occupied.insert(range.fragmentainer);
            }
        }
        // Foundational direct boxes have final column offsets but no custom
        // fragment records. Read their resolved placement rather than count
        // declared columns or infer occupancy from glyph ink.
        if let Some(group) = parent_owner.and_then(|owner| groups.get_mut(&owner))
            && document.parent_of(node_id) == parent_owner
            && !fragmented.contains(&node_id)
            && layout.size.width > 0.0
            && layout.size.height > 0.0
        {
            let stride = group.context.column_width + group.context.column_gap;
            let column = ((layout.location.x - group.origin.0) / stride)
                .round()
                .max(0.0) as usize;
            group.occupied.insert(column);
        }
        stack.extend(node.children.iter().rev().map(|&child| (child, owner)));
    }
    for fragment in &document.fragment_tree.fragments {
        if fragment.line_start.is_none()
            && fragment.rect.height > 0.0
            && fragment.rect.width > 0.0
            && let Some(group) =
                parent_owners[fragment.node_id].and_then(|owner| groups.get_mut(&owner))
        {
            group.occupied.insert(fragment.fragmentainer);
        }
    }
    let mut result = BTreeMap::new();
    for (owner, group) in groups {
        let width = cascade.computed[owner].column_rule.width().px();
        let mut rules = Vec::new();
        for &column in &group.occupied {
            if group.occupied.contains(&column.saturating_add(1)) {
                charge(1)?;
                let center = group.origin.0
                    + group.context.column_offset_x(column)
                    + group.context.column_width
                    + group.context.column_gap / 2.0;
                rules.push(PaintRect::new(
                    center - width / 2.0,
                    group.origin.1,
                    width,
                    group.height,
                ));
            }
        }
        if !rules.is_empty() {
            result.insert(owner, rules);
        }
    }
    Ok(result)
}

impl Document {
    /// Producer-computed rules relative to the owner's border-box origin.
    #[doc(hidden)]
    pub fn column_rules<'a>(
        &'a self,
        cascade: &'a CascadeResult,
        owner: usize,
    ) -> impl Iterator<Item = ColumnRule> + 'a {
        self.column_rules
            .get(&owner)
            .into_iter()
            .flatten()
            .copied()
            .map(move |rect| ColumnRule::new(owner, rect, &cascade.computed[owner]))
    }
}

pub(crate) type PageColumnRules = BTreeMap<u32, BTreeMap<NodeId, Vec<ColumnRule>>>;

/// Prepare bounded page slices atomically with the rest of page projection.
pub(crate) fn project(
    document: &Document,
    cascade: &CascadeResult,
    pages: &[crate::page_projection::records::PageFragment],
    work: &mut crate::layout::ProjectionWork<'_, '_>,
) -> Result<PageColumnRules, LayoutError> {
    let mut result = PageColumnRules::new();
    if document.column_rules.is_empty() {
        return Ok(result);
    }
    for page in pages {
        work.charge(page.items.len())?;
        for item in &page.items {
            if item.kind == crate::page_projection::records::PageFragmentKind::Text {
                continue;
            }
            let owner = item.node_id.0 as usize;
            let Some(source) = document.column_rules.get(&owner) else {
                continue;
            };
            let values = &cascade.computed[owner];
            if values.visibility != raikiri_style::property::Visibility::Visible {
                continue;
            }
            work.charge(source.len())?;
            for &rect in source {
                let mut rule = ColumnRule::new(
                    owner,
                    PaintRect::new(
                        rect.x + item.rect.x + page.content_box.x,
                        rect.y + item.box_y + page.content_box.y,
                        rect.width,
                        rect.height,
                    ),
                    values,
                );
                let start = rule.rect.y.max(page.content_box.y);
                let end = (rule.rect.y + rule.rect.height)
                    .min(page.content_box.y + page.content_box.height);
                if end <= start {
                    continue;
                }
                rule.rect.y = start;
                rule.rect.height = end - start;
                rule.overflow_chain = item.own_overflow_source.or(item.overflow_chain);
                rule.fragmentainer_clip =
                    crate::Fragment::new(item, page.content_box).fragmentainer_clip();
                result
                    .entry(page.page_index)
                    .or_default()
                    .entry(item.node_id)
                    .or_default()
                    .push(rule);
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
