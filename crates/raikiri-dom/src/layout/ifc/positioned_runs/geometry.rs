use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;

use super::{
    CascadeResult, Document, LineContext, PhysicalSize, PositionedLine, WritingMode, block_shift,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct OffsetRange {
    start: usize,
    end: usize,
    offset: (f32, f32),
}

#[derive(Default)]
pub(crate) struct OffsetRanges(BTreeMap<usize, OffsetRange>);

impl OffsetRanges {
    pub(crate) fn insert(&mut self, range: Range<usize>, offset: (f32, f32)) {
        if range.start >= range.end {
            return;
        }
        let tail = self
            .0
            .range(..range.end)
            .next_back()
            .filter(|(_, old)| old.end > range.end)
            .map(|(_, old)| OffsetRange {
                start: range.end,
                ..*old
            });
        if let Some((_, prefix)) = self.0.range_mut(..range.start).next_back()
            && prefix.end > range.start
        {
            prefix.end = range.start;
        }
        let removed: Vec<_> = self
            .0
            .range(range.clone())
            .map(|(&start, _)| start)
            .collect();
        for start in removed {
            self.0.remove(&start);
        }
        self.0.insert(
            range.start,
            OffsetRange {
                start: range.start,
                end: range.end,
                offset,
            },
        );
        if let Some(tail) = tail {
            self.0.insert(tail.start, tail);
        }
    }

    pub(crate) fn finish(self) -> Arc<[OffsetRange]> {
        self.0.into_values().collect::<Vec<_>>().into()
    }
}

pub(crate) fn offset_at(ranges: &[OffsetRange], index: usize) -> Option<(f32, f32)> {
    let end = ranges.partition_point(|range| range.start <= index);
    let range = ranges.get(end.checked_sub(1)?)?;
    (index < range.end).then_some(range.offset)
}

// Only owned geometry is retained in a page snapshot. Glyphs and styles are
// borrowed from the current document and cascade when a line is requested.
#[derive(Debug)]
pub(crate) struct PositionedLineGeometry {
    root: usize,
    writing_mode: WritingMode,
    converter_size: PhysicalSize,
    shifts: Arc<[f32]>,
    all: Arc<[OffsetRange]>,
    columns: HashMap<usize, Arc<[OffsetRange]>>,
    unfragmented: bool,
}

impl PositionedLineGeometry {
    pub(crate) fn new(document: &Document, root: usize) -> Option<Self> {
        let node = document.ifc_layout_node(root)?;
        let lines = node.ifc_lines()?;
        #[cfg(test)]
        crate::page_projection::work_tests::geometry_lines(lines.len());
        let writing_mode = node.ifc_writing_mode().unwrap_or(WritingMode::HorizontalTb);
        let mut converter_size = node
            .ifc_physical_content_size()
            .unwrap_or(PhysicalSize::default());
        if writing_mode == WritingMode::HorizontalTb {
            converter_size.width = node.ifc_size().unwrap_or((0.0, 0.0)).0;
        }
        let mut all = OffsetRanges::default();
        let mut columns = HashMap::<usize, OffsetRanges>::new();
        let unfragmented = node.ifc_multicol_fragments().is_none();
        if let Some(fragments) = node.ifc_multicol_fragments() {
            let origin_recorded = node
                .ifc
                .as_ref()
                .is_some_and(|root| root.multicol_fragment_origins_recorded);
            for fragment in fragments {
                let Some(first) = lines.get(fragment.line_start) else {
                    continue;
                };
                let range = fragment.line_start..fragment.line_end.min(lines.len());
                all.insert(
                    range.clone(),
                    (fragment.x, fragment.y - first.block_offset()),
                );
                columns.entry(fragment.fragmentainer).or_default().insert(
                    range,
                    (
                        0.0,
                        if origin_recorded { 0.0 } else { fragment.y } - first.block_offset(),
                    ),
                );
            }
        } else {
            all.insert(0..lines.len(), (0.0, 0.0));
        }
        Some(Self {
            root,
            writing_mode,
            converter_size,
            shifts: node.ifc_line_shifts().into(),
            all: all.finish(),
            columns: columns
                .into_iter()
                .map(|(column, ranges)| (column, ranges.finish()))
                .collect(),
            unfragmented,
        })
    }

    pub(crate) fn view<'a>(
        &'a self,
        document: &'a Document,
        cascade: &'a CascadeResult,
        fragmentainer: Option<usize>,
    ) -> Option<PreparedPositionedLines<'a>> {
        let node = document.ifc_layout_node(self.root)?;
        let line_data = node.ifc.as_ref()?.lines.as_ref()?;
        let lines = line_data.lines.as_slice();
        let ranges = match fragmentainer {
            Some(column) if !self.unfragmented => self
                .columns
                .get(&column)
                .map_or(&[][..], |ranges| ranges.as_ref()),
            _ => &self.all,
        };
        Some(PreparedPositionedLines {
            context: LineContext {
                document,
                cascade,
                root_id: self.root,
                writing_mode: self.writing_mode,
                converter_size: self.converter_size,
                offsets: node.ifc_relative_offsets(),
                line_data,
            },
            geometry: self,
            lines,
            ranges,
        })
    }

    #[cfg(test)]
    pub(crate) fn range_count(&self) -> usize {
        self.all.len()
            + self
                .columns
                .values()
                .map(|ranges| ranges.len())
                .sum::<usize>()
    }
}

pub(crate) struct PreparedPositionedLines<'a> {
    context: LineContext<'a>,
    geometry: &'a PositionedLineGeometry,
    lines: &'a [shodo::Line],
    ranges: &'a [OffsetRange],
}

impl<'a> PreparedPositionedLines<'a> {
    pub(crate) fn line_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.ranges.iter().map(|range| range.start..range.end)
    }

    pub(crate) fn all_lines(&self) -> &'a [shodo::Line] {
        self.lines
    }

    pub(crate) fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.ranges.iter().flat_map(|range| range.start..range.end)
    }

    pub(crate) fn line_offset(&self, index: usize) -> Option<(f32, f32)> {
        let (x, y) = offset_at(self.ranges, index)?;
        let shift = self.geometry.shifts.get(index).copied().unwrap_or(0.0);
        let (dx, dy) = block_shift(self.geometry.writing_mode, shift);
        Some((x + dx, y + dy))
    }

    pub(crate) fn line_at(&self, index: usize) -> Option<PositionedLine<'a>> {
        Some(
            self.context
                .line(index, self.lines.get(index)?, self.line_offset(index)?),
        )
    }
}
