use super::*;

type Measurement = (usize, LayoutOutput, TaffyLayout, f32, f32, f32);

pub(super) struct Group {
    pub(super) placements: Vec<Option<Placement>>,
    pub(super) height: f32,
}

pub(super) struct Placement {
    pub(super) first_column: usize,
    pub(super) first_y: f32,
    pub(super) first_height: f32,
    pub(super) last_column: usize,
    pub(super) last_y: f32,
    pub(super) fragments: Vec<MulticolTextFragment>,
}

struct Paragraph {
    extents: Vec<(f32, f32)>,
    margin_top: f32,
    margin_bottom: f32,
    orphans: usize,
    widows: usize,
}

pub(super) fn balance(
    tree: &Document,
    parent: usize,
    entries: &[Measurement],
    context: FragmentationContext,
) -> Option<Group> {
    let mut paragraphs = Vec::with_capacity(entries.len());
    let mut paragraph_count = 0;
    let mut high = 0.0;
    for &(child, output, _, margin_top, margin_bottom, _) in entries {
        let node = &tree.nodes[child];
        // Source whitespace between block elements has no formatting box.
        if node.kind() == NodeKind::Text && output.size.height == 0.0 && node.ifc.is_none() {
            paragraphs.push(None);
            continue;
        }
        if !can_balance_single_paragraph(tree, parent, child)
            || node.display != DisplayValue::Block
            || node.style.direction != TaffyDirection::Ltr
            || node.break_before != BreakBetween::Auto
            || node.break_after != BreakBetween::Auto
            || node.has_before_or_after_content
            || multicol_subtree_has_float(tree, child)
            || !margin_top.is_finite()
            || !margin_bottom.is_finite()
        {
            return None;
        }
        let lines = node.ifc.as_ref()?.lines.as_ref()?;
        let extents: Vec<_> = lines
            .lines
            .iter()
            .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
            .collect();
        let first = extents.first()?;
        let last = extents.last()?;
        let height = last.1 - first.0;
        if !height.is_finite() || height <= 0.0 {
            return None;
        }
        high += height + margin_top.abs() + margin_bottom.abs();
        paragraphs.push(Some(Paragraph {
            extents,
            margin_top,
            margin_bottom,
            orphans: node.paragraph_orphans,
            widows: node.paragraph_widows,
        }));
        paragraph_count += 1;
    }
    if paragraph_count < 2 || !high.is_finite() || context.column_count == 0 {
        return None;
    }
    // Trial work and storage depend on shaped lines, never the declared count.
    let mut low = 0.0;
    for _ in 0..32 {
        if high - low <= 0.001 {
            break;
        }
        let trial = (low + high) * 0.5;
        if fill(&paragraphs, context, trial).is_some() {
            high = trial;
        } else {
            low = trial;
        }
    }
    fill(&paragraphs, context, high)
}

fn collapsed_margin(before: f32, after: f32) -> f32 {
    before.max(after).max(0.0) + before.min(after).min(0.0)
}

fn fill(
    paragraphs: &[Option<Paragraph>],
    context: FragmentationContext,
    height: f32,
) -> Option<Group> {
    let mut placements = Vec::with_capacity(paragraphs.len());
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut pending_margin = 0.0f32;
    let mut maximum = 0.0f32;
    for paragraph in paragraphs {
        let Some(paragraph) = paragraph else {
            placements.push(None);
            continue;
        };
        let mut top = cursor + collapsed_margin(pending_margin, paragraph.margin_top);
        let mut start = 0;
        let mut fragments = Vec::new();
        let mut first_column = column;
        let mut first_y = top;
        let mut first_height = 0.0;
        while start < paragraph.extents.len() {
            let origin = paragraph.extents[start].0;
            let available = height - top;
            let mut end = start
                + paragraph.extents[start..]
                    .partition_point(|extent| extent.1 - origin <= available + f32::EPSILON);
            let remaining = paragraph.extents.len() - end;
            if remaining > 0 {
                end = end.min(paragraph.extents.len().saturating_sub(paragraph.widows));
                let minimum = if start == 0 {
                    paragraph.orphans
                } else {
                    paragraph.orphans.max(paragraph.widows)
                };
                if end.saturating_sub(start) < minimum {
                    end = start;
                }
            }
            if end <= start {
                // An unforced break at a block boundary discards both adjoining
                // margins. A too-tall paragraph at an empty column fails this trial.
                if start != 0 || top <= 0.0 {
                    return None;
                }
                maximum = maximum.max(cursor);
                column = column.checked_add(1)?;
                if column >= context.column_count {
                    return None;
                }
                top = 0.0;
                first_column = column;
                first_y = top;
                continue;
            }
            let used = paragraph.extents[end - 1].1 - origin;
            if start == 0 {
                first_height = used;
            }
            fragments.push(MulticolTextFragment {
                line_start: start,
                line_end: end,
                fragmentainer: column,
                x: context.column_offset_x(column) - context.column_offset_x(first_column),
                y: top - first_y,
            });
            cursor = top + used;
            maximum = maximum.max(cursor);
            start = end;
            if start < paragraph.extents.len() {
                column = column.checked_add(1)?;
                if column >= context.column_count {
                    return None;
                }
                top = 0.0;
            }
        }
        pending_margin = paragraph.margin_bottom;
        placements.push(Some(Placement {
            first_column,
            first_y,
            first_height,
            last_column: column,
            last_y: cursor + pending_margin,
            fragments,
        }));
    }
    let used = maximum.max(cursor + pending_margin);
    (used <= height + f32::EPSILON).then_some(Group {
        placements,
        height: used,
    })
}
