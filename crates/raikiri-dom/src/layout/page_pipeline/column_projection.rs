use super::{
    BoxRect, CascadeResult, Document, HashMap, LayoutError, PageLayoutControl, PositionedLines,
};
use crate::IfcTextLines;
use std::collections::{BTreeSet, HashSet};

pub(super) struct ProjectionWork<'a, 'b> {
    control: &'a PageLayoutControl<'b>,
    remaining: usize,
    limit: usize,
}

impl<'a, 'b> ProjectionWork<'a, 'b> {
    pub(super) fn new(control: &'a PageLayoutControl<'b>, limit: usize) -> Self {
        Self {
            control,
            remaining: limit,
            limit,
        }
    }
    pub(super) fn check(&self) -> Result<(), LayoutError> {
        self.control.check_aborted()
    }
    pub(super) fn charge(&mut self, amount: usize) -> Result<(), LayoutError> {
        self.check()?;
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or(LayoutError::FragmentLimitExceeded { limit: self.limit })?;
        Ok(())
    }
}

type TextGroups = Vec<(f32, usize, u32, IfcTextLines)>;

#[derive(Clone, Default)]
pub(super) struct ColumnProjection {
    pub(super) bounds: HashMap<usize, Vec<(BoxRect, u32)>>,
    pub(super) fragmented: HashSet<usize>,
    pub(super) owners: HashMap<usize, TextGroups>,
}

pub(super) struct ParagraphProjection {
    pub(super) combined: ColumnProjection,
    pub(super) columns: HashMap<usize, ColumnProjection>,
    pub(super) children: Option<HashMap<(usize, usize), Vec<usize>>>,
    pub(super) multicol: bool,
}

fn add_bounds(data: &mut ColumnProjection, node: usize, rect: BoxRect, column: u32, split: bool) {
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
    pub(super) fn prepare(
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
        if let Some(ranges) = ranges {
            let mut assigned = vec![false; count];
            for range in ranges {
                for index in range.line_start.min(count)..range.line_end.min(count) {
                    if !assigned[index] {
                        line_columns[index] = range.fragmentainer;
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
                        line,
                        0.0,
                        column as u32,
                    );
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
