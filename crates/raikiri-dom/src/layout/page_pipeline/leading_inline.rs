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
            candidate.context.textless_line()
                || (candidate.context.leading_block_piece_end().is_some()
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

/// The first atomic inline of each line of the paragraph `root`, when it
/// has more lines than one, no block among them, and no text places any of
/// them: no text
/// candidate moves those lines, so each of them moves with its first atomic
/// inline, as a line of text does. Empty for any other paragraph.
pub(super) fn textless_first_atoms(document: &Document, root: usize) -> HashSet<usize> {
    let Some(lines) = document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .filter(|ifc| ifc.multicol_fragments.is_none())
        .and_then(|ifc| ifc.lines.as_ref())
        .filter(|lines| lines.lines.len() > 1 && lines.block_line_starts.is_empty())
    else {
        return HashSet::new();
    };
    if lines.lines.iter().any(|line| places_text(document, line)) {
        return HashSet::new();
    }
    lines
        .lines
        .iter()
        .filter_map(|line| {
            line.fragments().find_map(|fragment| match fragment {
                shodo::Fragment::Atomic(atomic) => usize::try_from(atomic.node.0).ok(),
                _ => None,
            })
        })
        .collect()
}

/// Whether some glyph on `line`, a ruby annotation's included, belongs to
/// text that is not blank, or to generated content.
fn places_text(document: &Document, line: &shodo::Line) -> bool {
    line.fragments().any(|fragment| match fragment {
        shodo::Fragment::GlyphRun(run) => run.node().is_some_and(|node| {
            document.nodes.get(node.0 as usize).is_some_and(|node| {
                !matches!(&node.data, crate::node::NodeData::Text(text) if text.text_content.trim().is_empty())
            })
        }),
        shodo::Fragment::RubyAnnotation(annotation) => places_text(document, annotation.line()),
        _ => false,
    })
}

/// The lines of the paragraph `root`.
fn paragraph_lines(document: &Document, root: usize) -> Option<&[shodo::Line]> {
    document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .map(|lines| lines.lines.as_slice())
}

/// The line that a page break before the line `owner` of a paragraph that
/// no text places falls before once the `orphans` and `widows` of the
/// paragraph are kept (CSS Fragmentation 3 section 3.3), with how far that
/// line moves to start the next page at `next_start`. `page_start` is where
/// the page of `owner` starts in the flow and `relative` the relative offset
/// of the paragraph. `None` when the break stays before `owner`: enough lines
/// follow it, or the paragraph continues from the page before. The widows
/// move lines even when they do not fit on the next page together, as in
/// Chromium.
pub(super) fn kept_lines_break(
    document: &Document,
    cascade: &CascadeResult,
    owner: LeadingInlineLine,
    page_start: f32,
    relative: f32,
    next_start: f32,
) -> Option<KeptBreak> {
    let root = owner.root;
    let total = paragraph_lines(document, root)?.len();
    let (orphans, widows) = super::orphans_and_widows(cascade, root);
    let top = |line: usize| bounds(document, LeadingInlineLine { root, line }).0 - relative;
    // The first line of the paragraph on the page of `owner`; the tops of
    // the lines only grow.
    let (mut first, mut past) = (0, owner.line);
    while first < past {
        let middle = first + (past - first) / 2;
        if top(middle) < page_start - 0.001 {
            first = middle + 1;
        } else {
            past = middle;
        }
    }
    if first == owner.line {
        // The first line of the paragraph breaks before its box, when content
        // comes before it on the page.
        return (owner.line == 0 && top(0) > page_start + 0.001).then(|| KeptBreak {
            line: owner,
            shift: next_start - top(0),
            whole: true,
        });
    }
    if owner.line - first < orphans {
        // Fewer lines than the orphans fit: the paragraph starts the next
        // page, unless it starts this one already. A paragraph continued from
        // the page before keeps its natural break, as moving all its lines
        // would leave the page without them.
        if first > 0 || top(0) <= page_start + 0.001 {
            return None;
        }
        return Some(KeptBreak {
            line: LeadingInlineLine { root, line: 0 },
            shift: next_start - top(0),
            whole: true,
        });
    }
    // Orphans win over widows, as in columns.
    let line = crate::layout::multicol::piece_end(first, owner.line, total, (orphans, widows));
    if line == owner.line {
        return None;
    }
    Some(KeptBreak {
        line: LeadingInlineLine { root, line },
        shift: next_start - top(line),
        whole: false,
    })
}

/// Where [`kept_lines_break`] moves the break of a paragraph that no text
/// places.
pub(super) struct KeptBreak {
    /// The first line that starts the next page.
    pub(super) line: LeadingInlineLine,
    /// How far that line moves.
    pub(super) shift: f32,
    /// Whether that line is the first of the paragraph: the break then falls
    /// before the paragraph's box, which moves with its padding and border.
    pub(super) whole: bool,
}

/// The first line without atomic inlines between the line of `owner` and
/// the one with atomic inlines before it, in a paragraph that no text places,
/// that crosses the end of its page or starts a page exactly: such a line,
/// left by `<br>`, has no candidate of its own, so the break before it moves
/// with the line after it. `relative` is the relative offset of the
/// paragraph.
pub(super) fn crossing_blank_line(
    document: &Document,
    owner: LeadingInlineLine,
    relative: f32,
    page_origins: &PageOrigins,
) -> Option<LeadingInlineLine> {
    let lines = paragraph_lines(document, owner.root)?;
    let has_atom = |line: usize| {
        lines[line]
            .fragments()
            .any(|fragment| matches!(fragment, shodo::Fragment::Atomic(_)))
    };
    let after_atoms = (0..owner.line)
        .rev()
        .find(|&line| has_atom(line))
        .map_or(0, |line| line + 1);
    (after_atoms..owner.line)
        .map(|line| LeadingInlineLine {
            root: owner.root,
            line,
        })
        .find(|&line| {
            let (top, height) = bounds(document, line);
            let top = top - relative;
            let page = page_origins.page_index_for_y(top);
            let start = page_origins.origin(page);
            top + height > start + page_origins.step_at(page) + 0.001
                || page > 0 && (top - start).abs() <= 0.001
        })
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

pub(super) fn atomic_flow_top(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    owner: LeadingInlineLine,
) -> f32 {
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
        .map(|node| {
            current_abs_y(document, node)
                - accumulated_relative_offset_y(document, cascade, body_id, node)
                - document.nodes[node].layout.margin.top
        })
        .fold(f32::INFINITY, f32::min)
}

pub(super) fn move_line(
    document: &mut super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    owner: LeadingInlineLine,
    (shift, growth): (f32, Option<f32>),
    shifts: &mut SubtreeShifts,
    (flow_page, page): (u32, u32),
) -> f32 {
    // The block children after the line move with it in
    // `shift_paragraph_lines`.
    move_paragraph_lines(
        document,
        cascade,
        body_id,
        &LineMove {
            block_id: owner.root,
            flex_line: None,
            line: owner.line,
            shift,
            page,
            with_block_before: false,
            growth,
        },
        shifts,
        flow_page,
    )
}
