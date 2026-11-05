//! Placement ownership for atomic-only lines containing a leading-edge box.

use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct LeadingInlineLine {
    pub(super) root: usize,
    pub(super) line: usize,
}

pub(super) fn assign(
    document: &Document,
    candidates: &mut Vec<PageCandidate>,
    edges_breaks: &mut [candidates::EdgesBreak],
    control: &PageLayoutControl<'_>,
) -> Result<(), LayoutError> {
    let indexes: HashMap<usize, usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.node_id, index))
        .collect();
    let qualified: HashSet<usize> = candidates
        .iter()
        .filter(|candidate| {
            (candidate.context.leading_block_piece_end().is_some()
                && document.nodes[candidate.node_id].display() == DisplayValue::InlineBlock)
                || (candidate.context.is_direct_body_element()
                    && (document.nodes[candidate.node_id]
                        .display()
                        .is_atomic_inline_level()
                        || (document.nodes[candidate.node_id]
                            .display()
                            .is_inline_level()
                            && is_replaced_for_pagination(document, candidate.node_id))))
        })
        .map(|candidate| candidate.node_id)
        .collect();
    let roots: HashSet<usize> = qualified
        .iter()
        .filter_map(|&node| {
            document
                .layout_ancestors(node)
                .find(|&parent| document.nodes[parent].is_ifc_root())
        })
        .collect();
    let mut removed = HashSet::new();
    let mut delegated = HashSet::new();
    for root in roots {
        let ifc = document
            .ifc_layout_node(root)
            .and_then(|node| node.ifc.as_ref())
            .expect("completed IFC root of a qualified atomic");
        if ifc.multicol_fragments.is_some() {
            continue;
        }
        let lines = ifc
            .lines
            .as_ref()
            .expect("qualified atomic has shaped lines");
        for (line, shaped) in lines.lines.iter().enumerate() {
            control.check_aborted()?;
            let atoms: Vec<usize> = shaped
                .fragments()
                .filter_map(|fragment| match fragment {
                    shodo::Fragment::Atomic(atomic) => usize::try_from(atomic.node.0).ok(),
                    _ => None,
                })
                .collect();
            if !atoms.iter().any(|node| qualified.contains(node)) {
                continue;
            }
            // Real text already owns its line; an atomic must not move it again.
            let text_owned = crate::layout::ifc::text_lines::glyph_owners(shaped)
                .into_iter()
                .any(|owner| {
                    document.nodes.get(owner).is_some_and(|node| {
                        matches!(&node.data, crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty())
                    })
                });
            // A paragraph already places its first atomic-only line whole.
            // Delegate only when that owner is absent or earlier blocks stay.
            let paragraph_owned = indexes.get(&root).is_some_and(|&index| {
                candidates[index]
                    .context
                    .unplaced_line()
                    .is_some_and(|unplaced| unplaced.atomic)
                    && !lines
                        .block_line_starts
                        .iter()
                        .any(|&(_, after)| after <= line)
            });
            if text_owned || paragraph_owned {
                for atom in &atoms {
                    if let Some(&index) = indexes.get(atom)
                        && let candidates::BoxContext::Element { attributes, .. } =
                            &mut candidates[index].context
                    {
                        attributes.leading_block_piece_end = None;
                    }
                }
                continue;
            }
            let index = atoms
                .iter()
                .filter_map(|node| indexes.get(node))
                .min()
                .copied()
                .expect("a qualified atomic is an existing candidate");
            if let candidates::BoxContext::Element { attributes, .. } =
                &mut candidates[index].context
            {
                attributes.leading_inline_line = Some(LeadingInlineLine { root, line });
            }
            for atom in atoms {
                if indexes.get(&atom).is_some_and(|&other| other != index) {
                    removed.insert(atom);
                }
            }
            delegated.insert(root);
        }
    }
    for candidate in candidates.iter_mut() {
        if delegated.contains(&candidate.node_id)
            && let candidates::BoxContext::Element { attributes, .. } = &mut candidate.context
        {
            attributes.unplaced_line = None;
        }
    }
    if !removed.is_empty() {
        // The breaks that fall before a candidate follow it to its new place.
        let mut kept = Vec::with_capacity(candidates.len() + 1);
        let mut count = 0;
        for candidate in candidates.iter() {
            kept.push(count);
            count += usize::from(!removed.contains(&candidate.node_id));
        }
        kept.push(count);
        for edges_break in edges_breaks.iter_mut() {
            edges_break.before = kept[edges_break.before];
        }
        candidates.retain(|candidate| !removed.contains(&candidate.node_id));
    }
    Ok(())
}

pub(super) fn bounds(document: &Document, owner: LeadingInlineLine) -> (f32, f32) {
    let node = document
        .ifc_layout_node(owner.root)
        .expect("delegated paragraph");
    let lines = node
        .ifc
        .as_ref()
        .and_then(|ifc| ifc.lines.as_ref())
        .expect("delegated lines");
    let top = current_abs_y(document, owner.root)
        + node.layout.border.top
        + node.layout.padding.top
        + lines.line_top(owner.line)
        - crate::layout::ifc::outside_marker::fit_leading(document, owner.root, owner.line);
    let height = crate::layout::ifc::outside_marker::fit_height(&lines.lines[owner.line]);
    (top, height)
}

pub(super) fn atomic_top(document: &Document, owner: LeadingInlineLine) -> f32 {
    let lines = document
        .ifc_layout_node(owner.root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .expect("delegated lines");
    lines.lines[owner.line]
        .fragments()
        .filter_map(|fragment| match fragment {
            shodo::Fragment::Atomic(atomic) => usize::try_from(atomic.node.0)
                .ok()
                .filter(|&node| node < document.nodes.len()),
            _ => None,
        })
        .map(|node| current_abs_y(document, node) - document.nodes[node].layout.margin.top)
        .fold(f32::INFINITY, f32::min)
}

pub(super) fn move_line(
    document: &mut super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    owner: LeadingInlineLine,
    shift: f32,
    shifts: &mut SubtreeShifts,
    page: u32,
) -> f32 {
    let lines = document
        .ifc_layout_node(owner.root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .expect("delegated lines");
    let boxes: HashSet<usize> = lines
        .block_line_starts
        .iter()
        .filter_map(|&(node, after)| (after > owner.line).then_some(node))
        .collect();
    let growth = move_paragraph_lines(
        document,
        cascade,
        body_id,
        &LineMove {
            block_id: owner.root,
            flex_line: None,
            line: owner.line,
            shift,
        },
        shifts,
        page,
    );
    for node in boxes {
        document.translate_pagination_y(node, shift);
    }
    growth
}
