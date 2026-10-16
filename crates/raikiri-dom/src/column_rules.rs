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

/// One rule rectangle centered in the gap after `column`: the gap midpoint
/// is the column's inline offset plus a full column width and half the gap.
/// Shared by retained groups, fresh groups, and page projection so the three
/// stay in agreement.
fn column_rule_rect(
    origin_x: f32,
    origin_y: f32,
    context: &FragmentationContext,
    column: usize,
    width: f32,
    height: f32,
) -> PaintRect {
    let center = origin_x
        + context.column_offset_x(column)
        + context.column_width
        + context.column_gap / 2.0;
    PaintRect::new(center - width / 2.0, origin_y, width, height)
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
    let limit = document.fragment_tree.work_limit(document.nodes.len());
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
        let content_width = layout.content_box_width().max(0.0);
        let height = layout.content_box_height().max(0.0);
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
        // IFC ranges already include atomic content. Only the foundational
        // box flow needs a fallback for placements without fragment records.
        if let Some(parent_owner) = parent_owner
            && let Some(group) = groups.get_mut(&parent_owner)
            && document.parent_of(node_id) == Some(parent_owner)
            && !document.nodes[parent_owner].is_ifc_root()
            && !fragmented.contains(&node_id)
            && layout.size.width > 0.0
            && layout.size.height > 0.0
        {
            // An ordinary, unfragmented block flow only occupies its first
            // column.
            group.occupied.insert(0);
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
        // A nested container that continues in later outer columns keeps one
        // row of its columns per outer column; its rules follow those rows.
        let retained_groups = if document.nodes[owner].multicol_groups.is_empty() {
            &document.nodes[owner].multicol_rows
        } else {
            &document.nodes[owner].multicol_groups
        };
        if !retained_groups.is_empty() {
            let width = cascade.computed[owner].column_rule.width().px();
            let mut rules = Vec::new();
            for retained in retained_groups {
                charge(retained.occupied.len().saturating_add(1))?;
                if retained.height <= 0.0 {
                    continue;
                }
                for &column in &retained.occupied {
                    if retained.occupied.contains(&column.saturating_add(1)) {
                        charge(1)?;
                        rules.push(column_rule_rect(
                            retained.context.origin_x,
                            retained.context.origin_y,
                            &retained.context,
                            column,
                            width,
                            retained.height,
                        ));
                    }
                }
            }
            if !rules.is_empty() {
                result.insert(owner, rules);
            }
            continue;
        }
        let width = cascade.computed[owner].column_rule.width().px();
        let mut rules = Vec::new();
        for &column in &group.occupied {
            if group.occupied.contains(&column.saturating_add(1)) {
                charge(1)?;
                rules.push(column_rule_rect(
                    group.origin.0,
                    group.origin.1,
                    &group.context,
                    column,
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
    /// The rules of the row of a nested column container's columns in outer
    /// column `fragmentainer`, relative to that row's box.
    fn row_column_rules(&self, owner: usize, width: f32, fragmentainer: usize) -> Vec<PaintRect> {
        self.nodes[owner]
            .multicol_rows
            .iter()
            .filter(|row| row.context.column_index == fragmentainer)
            .flat_map(|row| {
                row.occupied
                    .iter()
                    .filter(|&&column| row.occupied.contains(&(column + 1)))
                    .map(move |&column| {
                        column_rule_rect(
                            row.context.origin_x,
                            row.context.origin_y,
                            &row.context,
                            column,
                            width,
                            row.height,
                        )
                    })
            })
            .collect()
    }

    /// The rules painted in the box of `owner` recorded by layout fragment
    /// `fragment`, relative to that box. A nested container continued in
    /// later outer columns has a box per row of its columns, and each box
    /// draws the rules of its own row.
    #[doc(hidden)]
    pub fn fragment_column_rules(
        &self,
        cascade: &CascadeResult,
        owner: usize,
        fragment: Option<usize>,
    ) -> Vec<ColumnRule> {
        let rows = &self.nodes[owner].multicol_rows;
        let Some(first) = rows.first() else {
            return self.column_rules(cascade, owner).collect();
        };
        // A box painted without a record of its own is the first row's.
        let fragmentainer = fragment
            .and_then(|index| self.fragment_tree.fragments.get(index))
            .filter(|fragment| fragment.node_id == owner)
            .map_or(first.context.column_index, |fragment| {
                fragment.fragmentainer
            });
        let values = &cascade.computed[owner];
        self.row_column_rules(owner, values.column_rule.width().px(), fragmentainer)
            .into_iter()
            .map(|rect| ColumnRule::new(owner, rect, values))
            .collect()
    }

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
    // Whether each owner's rules run down the container in order, so that a
    // page box only visits the rules that reach its page. A long container
    // has a box on every page, and visiting all of its rules on each one
    // would cost the page count times the rule count.
    let mut ordered = std::collections::HashMap::<usize, bool>::new();
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
            // A nested container continued in later outer columns has one
            // box per row; each box draws the rules of its own row.
            let row_rules: Vec<PaintRect>;
            let source = if document.nodes[owner].multicol_rows.is_empty() {
                source.as_slice()
            } else {
                row_rules = document.row_column_rules(
                    owner,
                    values.column_rule.width().px(),
                    item.fragmentainer as usize,
                );
                row_rules.as_slice()
            };
            let in_order = document.nodes[owner].multicol_rows.is_empty()
                && match ordered.entry(owner) {
                    std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        // The ordering check reads every rule once per owner.
                        work.charge(source.len())?;
                        *entry.insert(source.windows(2).all(|pair| {
                            pair[0].y <= pair[1].y
                                && pair[0].y + pair[0].height <= pair[1].y + pair[1].height
                        }))
                    }
                };
            let page_end = page.content_box.y + page.content_box.height;
            let source = if in_order {
                // Rules ending above the page and rules starting below it
                // are skipped; the ones between are clipped as usual.
                let top = |rect: &PaintRect| rect.y + item.box_y + page.content_box.y;
                let first =
                    source.partition_point(|rect| top(rect) + rect.height <= page.content_box.y);
                let source = &source[first..];
                &source[..source.partition_point(|rect| top(rect) < page_end)]
            } else {
                source
            };
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
