use super::{
    BoxRect, CascadeResult, Document, HashMap, LayoutError, PageLayoutControl, PositionedLines,
};
use crate::IfcTextLines;
use std::collections::{BTreeSet, HashSet};

pub(crate) struct ProjectionWork<'a, 'b> {
    control: &'a PageLayoutControl<'b>,
    remaining: usize,
    limit: usize,
}

impl<'a, 'b> ProjectionWork<'a, 'b> {
    pub(crate) fn new(control: &'a PageLayoutControl<'b>, limit: usize) -> Self {
        Self {
            control,
            remaining: limit,
            limit,
        }
    }
    pub(crate) fn remaining(&self) -> usize {
        self.remaining
    }
    pub(crate) fn with_remaining(
        control: &'a PageLayoutControl<'b>,
        limit: usize,
        remaining: usize,
    ) -> Self {
        Self {
            control,
            limit,
            remaining,
        }
    }
    pub(crate) fn check(&self) -> Result<(), LayoutError> {
        self.control.check_aborted()
    }
    pub(crate) fn charge(&mut self, amount: usize) -> Result<(), LayoutError> {
        self.check()?;
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or(LayoutError::FragmentLimitExceeded { limit: self.limit })?;
        Ok(())
    }
}

pub(crate) type ParagraphCache = HashMap<usize, ParagraphProjection>;
pub(crate) type PreparedGeneratedPiece = (
    usize,
    bool,
    crate::layout::InlineBoxPiece,
    (f32, f32),
    f32,
    f32,
    f32,
);

type TextGroups = Vec<(f32, usize, u32, IfcTextLines)>;

#[derive(Clone, Default)]
pub(super) struct ColumnProjection {
    pub(super) bounds: HashMap<usize, Vec<(BoxRect, u32)>>,
    pub(super) fragmented: HashSet<usize>,
    pub(super) owners: HashMap<usize, TextGroups>,
    pub(super) record_count: usize,
}

pub(crate) struct ParagraphProjection {
    pub(super) combined: ColumnProjection,
    pub(super) columns: HashMap<usize, ColumnProjection>,
    pub(super) children: Option<HashMap<(usize, usize), Vec<usize>>>,
    pub(super) multicol: bool,
    generated: Vec<PreparedGeneratedPiece>,
    generated_columns: HashMap<usize, Vec<PreparedGeneratedPiece>>,
    markers: Vec<(f32, f32, crate::PositionedMarker)>,
    marker_columns: HashMap<usize, Vec<(f32, f32, crate::PositionedMarker)>>,
}

fn add_bounds(data: &mut ColumnProjection, node: usize, rect: BoxRect, column: u32, split: bool) {
    data.record_count += 1;
    let pieces = data.bounds.entry(node).or_default();
    if !pieces.is_empty() {
        data.fragmented.insert(node);
    }
    if split || pieces.is_empty() {
        pieces.push((rect, column));
    } else {
        let bounds = &mut pieces[0].0;
        let x = bounds.x.min(rect.x);
        let y = bounds.y.min(rect.y);
        *bounds = BoxRect {
            x,
            y,
            width: (bounds.x + bounds.width).max(rect.x + rect.width) - x,
            height: (bounds.y + bounds.height).max(rect.y + rect.height) - y,
        };
    }
}

fn add_line(
    data: &mut ColumnProjection,
    owner: usize,
    owned: &IfcTextLines,
    source_index: usize,
    line: crate::IfcTextLine,
    x: f32,
    column: u32,
) {
    data.record_count += 1;
    let groups = data.owners.entry(owner).or_default();
    if let Some((previous_x, _, previous_column, previous)) = groups.last_mut()
        && *previous_x == x
        && *previous_column == column
    {
        previous.lines.push(line);
    } else {
        groups.push((
            x,
            source_index,
            column,
            IfcTextLines {
                root: owned.root,
                width: owned.width,
                lines: vec![line],
            },
        ));
    }
}

impl ParagraphProjection {
    pub(crate) fn generated(&self, column: Option<usize>) -> &[PreparedGeneratedPiece] {
        if self.multicol
            && let Some(column) = column
        {
            self.generated_columns
                .get(&column)
                .map_or(&[], Vec::as_slice)
        } else {
            &self.generated
        }
    }
    pub(crate) fn markers(&self, column: Option<usize>) -> &[(f32, f32, crate::PositionedMarker)] {
        if self.multicol
            && let Some(column) = column
        {
            self.marker_columns.get(&column).map_or(&[], Vec::as_slice)
        } else {
            &self.markers
        }
    }

    pub(crate) fn prepare(
        document: &Document,
        cascade: &CascadeResult,
        root: usize,
        fallback_column: usize,
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<Self, LayoutError> {
        let node = document
            .ifc_layout_node(root)
            .expect("projected roots retain their paragraph");
        let ranges = node.ifc_multicol_fragments();
        let multicol = ranges.is_some();
        let count = node.ifc_lines().map_or(0, |lines| lines.len());
        // Charge the complete offset preparation before allocation, including
        // repeated ranges, so malformed overlapping ranges cannot amplify work.
        if let Some(ranges) = ranges {
            let cost = ranges.iter().fold(count, |cost, range| {
                cost.saturating_add(
                    range
                        .line_end
                        .min(count)
                        .saturating_sub(range.line_start.min(count)),
                )
            });
            work.charge(cost)?;
        }
        work.check()?;
        let positioned = PositionedLines::new(document, cascade, root, None);
        let mut line_columns = vec![fallback_column; count];
        let mut column_origins = vec![0.0; count];
        let origins_recorded = document.nodes[root]
            .ifc
            .as_ref()
            .is_some_and(|root| root.multicol_fragment_origins_recorded);
        if let Some(ranges) = ranges {
            let mut assigned = vec![false; count];
            for range in ranges {
                for index in range.line_start.min(count)..range.line_end.min(count) {
                    if !assigned[index] {
                        line_columns[index] = range.fragmentainer;
                        if origins_recorded {
                            column_origins[index] = range.y;
                        }
                        assigned[index] = true;
                    }
                }
            }
        }
        let mut result = Self {
            combined: ColumnProjection::default(),
            columns: HashMap::new(),
            children: None,
            multicol,
            generated: Vec::new(),
            generated_columns: HashMap::new(),
            markers: Vec::new(),
            marker_columns: HashMap::new(),
        };
        for piece in node.ifc_inline_boxes().unwrap_or_default() {
            work.check()?;
            if multicol {
                work.charge(1)?;
            }
            let Some(offset) = positioned
                .as_ref()
                .and_then(|lines| lines.line_offset(piece.line))
            else {
                continue;
            };
            let shift = positioned
                .as_ref()
                .and_then(|lines| lines.shifts.get(piece.line))
                .copied()
                .unwrap_or(0.0);
            let column = line_columns[piece.line];
            if let Some((owner, pseudo)) = crate::generated_content::generated_origin(piece.node)
                && matches!(
                    pseudo,
                    raikiri_style::PseudoElem::Before | raikiri_style::PseudoElem::After
                )
                && piece.border_box.width > 0.0
                && piece.border_box.height > 0.0
                && crate::generated_content::computed_for_id(cascade, piece.node).is_some_and(
                    |style| style.visibility == raikiri_style::property::Visibility::Visible,
                )
            {
                let line = &positioned.as_ref().unwrap().all_lines()[piece.line];
                let data = (
                    owner,
                    pseudo == raikiri_style::PseudoElem::After,
                    piece,
                    offset,
                    shift,
                    line.block_offset(),
                    line.block_size(),
                );
                result.generated.push(data);
                result.generated_columns.entry(column).or_default().push((
                    owner,
                    data.1,
                    piece,
                    (0.0, offset.1 - column_origins[piece.line]),
                    shift,
                    data.5,
                    data.6,
                ));
            }
            let rect = BoxRect {
                x: piece.border_box.x + offset.0,
                y: piece.border_box.y + offset.1 - shift,
                ..piece.border_box
            };
            add_bounds(
                &mut result.combined,
                piece.node,
                rect,
                column as u32,
                multicol,
            );
            if multicol {
                let rect = BoxRect {
                    x: piece.border_box.x,
                    y: rect.y - column_origins[piece.line],
                    ..rect
                };
                add_bounds(
                    result.columns.entry(column).or_default(),
                    piece.node,
                    rect,
                    column as u32,
                    true,
                );
            }
        }
        for (owner, owned) in document.ifc_text_lines_by_node(root) {
            work.check()?;
            if multicol {
                work.charge(1)?;
            }
            result.combined.record_count += 1;
            result.combined.owners.entry(owner).or_default();
            for (index, raw) in owned.lines.iter().enumerate() {
                work.check()?;
                if multicol {
                    work.charge(1)?;
                }
                let Some(offset) = positioned
                    .as_ref()
                    .and_then(|lines| lines.line_offset(raw.line))
                else {
                    continue;
                };
                let shift = positioned
                    .as_ref()
                    .and_then(|lines| lines.shifts.get(raw.line))
                    .copied()
                    .unwrap_or(0.0);
                let column = line_columns[raw.line];
                let line = crate::IfcTextLine {
                    top: raw.top + offset.1 - shift,
                    bottom: raw.bottom + offset.1 - shift,
                    ..*raw
                };
                add_line(
                    &mut result.combined,
                    owner,
                    &owned,
                    index,
                    line,
                    offset.0,
                    column as u32,
                );
                if multicol {
                    add_line(
                        result.columns.entry(column).or_default(),
                        owner,
                        &owned,
                        index,
                        crate::IfcTextLine {
                            top: line.top - column_origins[raw.line],
                            bottom: line.bottom - column_origins[raw.line],
                            ..line
                        },
                        0.0,
                        column as u32,
                    );
                }
            }
        }
        if crate::generated_content::inside_marker_in_flow(cascade, root)
            && document.list_marker_image(root).is_some()
            && let Some(positioned) = &positioned
        {
            for line in positioned.lines() {
                work.charge(
                    line.runs
                        .len()
                        .saturating_add(line.markers.len())
                        .saturating_add(1),
                )?;
                let top = line.offset.1 + line.line.block_offset();
                let height = line.line.block_size();
                for marker in line.markers {
                    let global = crate::PositionedMarker {
                        owner: marker.owner,
                        rect: raikiri_traits::PaintRect::new(
                            line.offset.0 + marker.rect.x,
                            line.offset.1 + marker.rect.y,
                            marker.rect.width,
                            marker.rect.height,
                        ),
                    };
                    let local = crate::PositionedMarker {
                        rect: raikiri_traits::PaintRect::new(
                            marker.rect.x,
                            global.rect.y - column_origins[line.index],
                            global.rect.width,
                            global.rect.height,
                        ),
                        ..global
                    };
                    result.markers.push((top, height, global));
                    result
                        .marker_columns
                        .entry(line_columns[line.index])
                        .or_default()
                        .push((top - column_origins[line.index], height, local));
                }
            }
        }
        // Pure inline paragraphs have no independent block/atomic subtrees.
        // Index their source edges by actual column ownership once, retaining
        // unowned empty elements in the first placement only. Atomic subtrees
        // keep the existing traversal and remain bounded by the work cap.
        if multicol && node.ifc_boxes().is_empty() {
            let mut membership = HashMap::<usize, BTreeSet<usize>>::new();
            for (&column, data) in &result.columns {
                for &source in data.owners.keys().chain(data.bounds.keys()) {
                    let mut current = source;
                    while current != root {
                        work.charge(1)?;
                        if !membership.entry(current).or_default().insert(column) {
                            break;
                        }
                        let Some(parent) = document.parent_of(current) else {
                            break;
                        };
                        current = parent;
                    }
                }
            }
            let first_column = ranges
                .and_then(|ranges| ranges.first())
                .map_or(fallback_column, |range| range.fragmentainer);
            let mut children = HashMap::<(usize, usize), Vec<usize>>::new();
            let mut pending = vec![root];
            while let Some(parent) = pending.pop() {
                for &child in &document.nodes[parent].children {
                    work.charge(1)?;
                    let columns = membership
                        .get(&child)
                        .cloned()
                        .unwrap_or_else(|| BTreeSet::from([first_column]));
                    for column in columns {
                        work.charge(1)?;
                        children.entry((parent, column)).or_default().push(child);
                    }
                    pending.push(child);
                }
            }
            result.children = Some(children);
        }
        Ok(result)
    }
}
