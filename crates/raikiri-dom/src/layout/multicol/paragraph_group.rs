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
    let (paragraphs, paragraph_count, has_empty, mut high) = collect(tree, parent, entries)?;
    if paragraph_count == 0
        || (paragraph_count == 1 && !has_empty)
        || !high.is_finite()
        || context.column_count == 0
    {
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

fn collect(
    tree: &Document,
    parent: usize,
    entries: &[Measurement],
) -> Option<(Vec<Option<Paragraph>>, usize, bool, f32)> {
    let mut paragraphs = Vec::with_capacity(entries.len());
    let mut paragraph_count = 0;
    let mut has_empty = false;
    let mut high = 0.0;
    for &(child, output, _, margin_top, margin_bottom, _) in entries {
        let node = &tree.nodes[child];
        // Source whitespace between block elements has no formatting box.
        if node.kind() == NodeKind::Text && output.size.height == 0.0 && node.ifc.is_none() {
            paragraphs.push(None);
            continue;
        }
        let empty = output.size.height == 0.0
            && node.ifc.as_ref().is_none_or(|root| {
                root.lines
                    .as_ref()
                    .is_none_or(|lines| lines.lines.is_empty())
            })
            && node.children.iter().all(|&child| {
                tree.nodes[child].kind() == NodeKind::Text
                    && tree.nodes[child].unrounded_layout.size.height == 0.0
            })
            && node
                .authored_writing_mode
                .is_none_or(|mode| mode == raikiri_style::property::WritingMode::HorizontalTb)
            && node.style.overflow.x == taffy::Overflow::Visible
            && node.style.overflow.y == taffy::Overflow::Visible;
        if !can_balance_paragraph_box(tree, parent, child)
            || (!empty && !can_balance_single_paragraph(tree, parent, child))
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
        if empty {
            has_empty = true;
            high += margin_top.abs() + margin_bottom.abs();
            paragraphs.push(Some(Paragraph {
                extents: Vec::new(),
                margin_top,
                margin_bottom,
                orphans: node.paragraph_orphans,
                widows: node.paragraph_widows,
            }));
            continue;
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
    Some((paragraphs, paragraph_count, has_empty, high))
}

#[derive(Clone, Copy, Default)]
struct Margins {
    positive: f32,
    negative: f32,
}

impl Margins {
    fn with(self, value: f32) -> Self {
        Self {
            positive: self.positive.max(value),
            negative: self.negative.min(value),
        }
    }

    fn value(self) -> f32 {
        self.positive + self.negative
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct FlowCursor {
    paragraph: usize,
    line: usize,
}

struct Chunk {
    group: Group,
    next: Option<FlowCursor>,
}

fn fill(
    paragraphs: &[Option<Paragraph>],
    context: FragmentationContext,
    height: f32,
) -> Option<Group> {
    let chunk = fill_chunk(paragraphs, context, height, FlowCursor::default())?;
    chunk.next.is_none().then_some(chunk.group)
}

fn fill_chunk(
    paragraphs: &[Option<Paragraph>],
    context: FragmentationContext,
    height: f32,
    resume: FlowCursor,
) -> Option<Chunk> {
    let mut placements: Vec<Option<Placement>> = (0..paragraphs.len()).map(|_| None).collect();
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut pending_margin = Margins::default();
    let mut maximum = 0.0f32;
    let mut column_has_content = false;
    for (paragraph_index, paragraph) in paragraphs.iter().enumerate().skip(resume.paragraph) {
        let Some(paragraph) = paragraph else {
            continue;
        };
        let initial_line = if paragraph_index == resume.paragraph {
            resume.line
        } else {
            0
        };
        let adjoining = pending_margin.with(if initial_line == 0 {
            paragraph.margin_top
        } else {
            0.0
        });
        let mut top = cursor + adjoining.value();
        if paragraph.extents.is_empty() {
            // Retain both extrema across empty boxes: collapsing their summed
            // margins again would lose the negative member of the chain.
            pending_margin = adjoining.with(paragraph.margin_bottom);
            placements[paragraph_index] = Some(Placement {
                first_column: column,
                first_y: top,
                first_height: 0.0,
                last_column: column,
                last_y: cursor + pending_margin.value(),
                fragments: Vec::new(),
            });
            continue;
        }
        let mut start = initial_line;
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
                if start != 0 || !column_has_content {
                    return None;
                }
                maximum = maximum.max(cursor);
                column = column.checked_add(1)?;
                if column >= context.column_count {
                    return Some(Chunk {
                        group: finish_group(placements, height),
                        next: Some(FlowCursor {
                            paragraph: paragraph_index,
                            line: start,
                        }),
                    });
                }
                top = 0.0;
                column_has_content = false;
                first_column = column;
                first_y = top;
                continue;
            }
            let used = paragraph.extents[end - 1].1 - origin;
            if fragments.is_empty() {
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
            column_has_content = true;
            maximum = maximum.max(cursor);
            start = end;
            if start < paragraph.extents.len() {
                column = column.checked_add(1)?;
                if column >= context.column_count {
                    placements[paragraph_index] = Some(Placement {
                        first_column,
                        first_y,
                        first_height,
                        last_column: column - 1,
                        last_y: cursor,
                        fragments,
                    });
                    return Some(Chunk {
                        group: finish_group(placements, height),
                        next: Some(FlowCursor {
                            paragraph: paragraph_index,
                            line: start,
                        }),
                    });
                }
                top = 0.0;
                column_has_content = false;
            }
        }
        pending_margin = Margins::default().with(paragraph.margin_bottom);
        placements[paragraph_index] = Some(Placement {
            first_column,
            first_y,
            first_height,
            last_column: column,
            last_y: cursor + pending_margin.value(),
            fragments,
        });
    }
    let used = maximum.max(cursor + pending_margin.value());
    (used <= height + f32::EPSILON).then(|| Chunk {
        group: finish_group(placements, used),
        next: None,
    })
}

fn finish_group(mut placements: Vec<Option<Placement>>, used: f32) -> Group {
    for placement in placements.iter_mut().flatten() {
        if placement.fragments.len() > 1 {
            placement.first_height = (used - placement.first_y).max(0.0);
        }
    }
    Group {
        placements,
        height: used,
    }
}

pub(super) struct PagedFragment {
    pub(super) child: usize,
    pub(super) line_start: usize,
    pub(super) line_end: usize,
    pub(super) fragmentainer: usize,
    pub(super) rect: crate::fragment::FragmentRect,
    pub(super) clip: crate::fragment::FragmentRect,
}

pub(super) struct PagedGroup {
    pub(super) fragments: Vec<PagedFragment>,
    pub(super) groups: Vec<crate::fragment::MulticolGroup>,
    pub(super) end: f32,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paginate(
    tree: &Document,
    parent: usize,
    entries: &[Measurement],
    context: FragmentationContext,
    owner_y: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<Option<PagedGroup>, LayoutError> {
    let line_count = entries
        .iter()
        .map(|(child, ..)| {
            tree.nodes[*child]
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .map_or(0, |lines| lines.lines.len())
        })
        .sum::<usize>();
    work.charge(entries.len().saturating_add(line_count))?;
    let Some((paragraphs, count, _, _)) = collect(tree, parent, entries) else {
        return Ok(None);
    };
    if count == 0 || context.column_count == 0 {
        return Ok(None);
    }
    let mut result = PagedGroup {
        fragments: Vec::new(),
        groups: Vec::new(),
        end: context.origin_y,
    };
    let mut resume = FlowCursor::default();
    let mut absolute_y = owner_y + context.origin_y;
    loop {
        let (page_index, page_origin, page_height) = page_for(absolute_y);
        if !work.page_available(page_index)? {
            break;
        }
        work.charge(entries.len().saturating_add(line_count))?;
        let capacity = page_origin + page_height - absolute_y;
        if !capacity.is_finite() || capacity <= 0.0 {
            return Ok(None);
        }
        let Some(mut chunk) = fill_chunk(&paragraphs, context, capacity, resume) else {
            // A short remainder can contain fewer lines than the paragraph's
            // break minima. Retry at the next complete page, without a soft
            // break inside the paragraph or a zero-progress page loop.
            if absolute_y > page_origin + 0.001 {
                absolute_y = page_origin + page_height;
                continue;
            }
            return Ok(None);
        };
        if chunk.next.is_none() && context.column_fill != ColumnFillValue::Auto {
            let mut low = 0.0;
            let mut high = capacity;
            for _ in 0..32 {
                if high - low <= 0.001 {
                    break;
                }
                work.charge(entries.len().saturating_add(line_count))?;
                let trial = (low + high) * 0.5;
                if let Some(candidate) = fill_chunk(&paragraphs, context, trial, resume)
                    && candidate.next.is_none()
                {
                    high = trial;
                    chunk = candidate;
                } else {
                    low = trial;
                }
            }
        }
        let height = if chunk.next.is_some() {
            capacity
        } else {
            chunk.group.height
        };
        let group_y = absolute_y - owner_y;
        let mut occupied = std::collections::BTreeSet::new();
        for (index, placement) in chunk.group.placements.into_iter().enumerate() {
            let Some(placement) = placement else {
                continue;
            };
            let Some(paragraph) = &paragraphs[index] else {
                continue;
            };
            for range in placement.fragments {
                work.charge(1)?;
                let y = placement.first_y + range.y;
                let natural_height =
                    paragraph.extents[range.line_end - 1].1 - paragraph.extents[range.line_start].0;
                let box_height = if range.line_end < paragraph.extents.len() {
                    (height - y).max(0.0)
                } else {
                    natural_height
                };
                let column = range.fragmentainer;
                let fragmentainer = (page_index as usize)
                    .checked_mul(context.column_count)
                    .and_then(|first| first.checked_add(column))
                    .ok_or(LayoutError::FragmentLimitExceeded {
                        limit: tree.fragment_tree.limit,
                    })?;
                let x = context.origin_x + context.column_offset_x(column);
                occupied.insert(column);
                result.fragments.push(PagedFragment {
                    child: entries[index].0,
                    line_start: range.line_start,
                    line_end: range.line_end,
                    fragmentainer,
                    rect: crate::fragment::FragmentRect {
                        x: x + entries[index].2.margin.left,
                        y: group_y + y,
                        width: entries[index].2.size.width,
                        height: box_height,
                    },
                    clip: crate::fragment::FragmentRect {
                        x,
                        y: group_y,
                        width: context.column_width,
                        height,
                    },
                });
            }
        }
        result.groups.push(crate::fragment::MulticolGroup {
            context: FragmentationContext {
                origin_y: group_y,
                available_height: Some(height),
                ..context
            },
            height,
            occupied,
        });
        result.end = group_y + height;
        let Some(next) = chunk.next else {
            break;
        };
        if next == resume {
            return Ok(None);
        }
        resume = next;
        absolute_y = page_origin + page_height;
    }
    Ok(Some(result))
}
