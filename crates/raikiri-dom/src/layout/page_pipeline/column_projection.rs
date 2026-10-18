use super::{
    BoxRect, CascadeResult, Document, HashMap, LayoutError, PageLayoutControl, PositionedLines,
};
use crate::IfcTextLines;
use crate::node::MulticolTextFragment;
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
    pub(crate) fn page_available(&self, page_index: u32) -> Result<bool, LayoutError> {
        self.control.check_discovery_page_index(page_index)?;
        Ok(self
            .control
            .max_pages
            .is_none_or(|limit| page_index < limit))
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
/// A line piece of a generated box, or of an inline element when the kind is
/// `None`, with its owner, line offset, shift, and line top and height. A
/// piece of a block-level generated box gives its own top and height
/// instead, and whether it holds the block-start and block-end edges of
/// its box.
pub(crate) type PreparedGeneratedPiece = (
    usize,
    Option<crate::GeneratedKind>,
    crate::layout::InlineBoxPiece,
    (f32, f32),
    f32,
    f32,
    f32,
    Option<(bool, bool)>,
);

/// The kind of generated box a `::before` or `::after` id names.
fn generated_kind(node: usize) -> Option<(usize, crate::GeneratedKind)> {
    let (owner, pseudo) = crate::generated_content::generated_origin(node)?;
    Some((
        owner,
        match pseudo {
            raikiri_style::PseudoElem::Before => crate::GeneratedKind::Before,
            raikiri_style::PseudoElem::After => crate::GeneratedKind::After,
            raikiri_style::PseudoElem::FirstLetter => crate::GeneratedKind::FirstLetter,
            _ => return None,
        },
    ))
}

/// A content image with the top and height of its line, its line index, and
/// its rectangle from the root's content-box origin.
pub(crate) type PreparedContentImage = (f32, f32, usize, crate::PositionedContentImage);

pub(super) type TextGroups = Vec<(f32, usize, u32, IfcTextLines)>;

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
    content_images: Vec<PreparedContentImage>,
    content_image_columns: HashMap<usize, Vec<PreparedContentImage>>,
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
    pub(crate) fn content_images(&self, column: Option<usize>) -> &[PreparedContentImage] {
        if self.multicol
            && let Some(column) = column
        {
            self.content_image_columns
                .get(&column)
                .map_or(&[], Vec::as_slice)
        } else {
            &self.content_images
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
        charge_column_ranges(ranges, count, work)?;
        work.check()?;
        let positioned = PositionedLines::new(document, cascade, root, None);
        let origins_recorded = node
            .ifc
            .as_ref()
            .is_some_and(|root| root.multicol_fragment_origins_recorded);
        let LineColumns {
            column: line_columns,
            origin: column_origins,
        } = LineColumns::new(ranges, count, fallback_column, origins_recorded);
        let mut result = Self {
            combined: ColumnProjection::default(),
            columns: HashMap::new(),
            children: None,
            multicol,
            generated: Vec::new(),
            generated_columns: HashMap::new(),
            markers: Vec::new(),
            marker_columns: HashMap::new(),
            content_images: Vec::new(),
            content_image_columns: HashMap::new(),
        };
        let pieces = node.ifc_inline_boxes().unwrap_or_default();
        let split = split_inline_elements(document, cascade, root, node, &pieces);
        // A block-level `::before` or `::after` paints its own box; its
        // pieces on the lines only hold its text.
        let blocks = node.ifc_generated_block_pieces();
        let block_nodes: HashSet<usize> = blocks.iter().map(|block| block.node).collect();
        // Only `::before` and `::after` are laid out as blocks.
        let kinds = blocks
            .into_iter()
            .filter_map(|block| generated_kind(block.node).map(|kind| (block, kind)));
        for (block, (owner, kind)) in kinds {
            work.check()?;
            let visible = block.border_box.width > 0.0
                && block.border_box.height > 0.0
                && crate::generated_content::computed_for_id(cascade, block.node).is_some_and(
                    |style| style.visibility == raikiri_style::property::Visibility::Visible,
                );
            if !visible {
                continue;
            }
            result.generated.push((
                owner,
                Some(kind),
                crate::layout::InlineBoxPiece {
                    node: block.node,
                    line: block.line,
                    border_box: block.border_box,
                    content_box: block.content_box,
                    has_start_edge: true,
                    has_end_edge: true,
                    parent: None,
                    source_container: None,
                    source_owner: None,
                },
                (0.0, block.shift),
                block.shift,
                block.border_box.y - block.shift,
                block.border_box.height,
                Some((block.has_block_start_edge, block.has_block_end_edge)),
            ));
        }
        for piece in pieces {
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
            let generated = crate::generated_content::generated_origin(piece.node);
            if let Some((owner, kind)) = generated_kind(piece.node)
                .map(|(owner, kind)| (owner, Some(kind)))
                .or_else(|| {
                    (generated.is_none() && split.contains(&piece.node))
                        .then_some((piece.node, None))
                })
                && !block_nodes.contains(&piece.node)
                && piece.border_box.width > 0.0
                && piece.border_box.height > 0.0
                && generated_piece_style(
                    node,
                    cascade,
                    piece.node,
                    piece.source_container,
                    piece.source_owner,
                    piece.line,
                )
                .is_some_and(|style| {
                    style.visibility == raikiri_style::property::Visibility::Visible
                })
            {
                let line = &positioned.as_ref().unwrap().all_lines()[piece.line];
                let data = (
                    owner,
                    kind,
                    piece,
                    offset,
                    shift,
                    line.block_offset(),
                    line.block_size(),
                    None,
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
                    data.7,
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
        if node
            .ifc
            .as_ref()
            .is_some_and(|ifc| !ifc.generated_atomics.is_empty())
            && let Some(positioned) = &positioned
        {
            for line in positioned.lines() {
                work.charge(
                    line.runs
                        .len()
                        .saturating_add(line.content_images.len())
                        .saturating_add(1),
                )?;
                let top = line.offset.1 + line.line.block_offset();
                let height = line.line.block_size();
                for image in line.content_images {
                    let global = crate::PositionedContentImage {
                        rect: raikiri_traits::PaintRect::new(
                            line.offset.0 + image.rect.x,
                            line.offset.1 + image.rect.y,
                            image.rect.width,
                            image.rect.height,
                        ),
                        ..image
                    };
                    let local = crate::PositionedContentImage {
                        rect: raikiri_traits::PaintRect::new(
                            image.rect.x,
                            global.rect.y - column_origins[line.index],
                            global.rect.width,
                            global.rect.height,
                        ),
                        ..global
                    };
                    result
                        .content_images
                        .push((top, height, line.index, global));
                    result
                        .content_image_columns
                        .entry(line_columns[line.index])
                        .or_default()
                        .push((top - column_origins[line.index], height, line.index, local));
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
                    for current in document
                        .inclusive_ancestors(source)
                        .take_while(|&current| current != root)
                    {
                        work.charge(1)?;
                        if !membership.entry(current).or_default().insert(column) {
                            break;
                        }
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

/// Charge `work` for preparing the lines of `count` over the `ranges` of a
/// paragraph in columns, before anything is allocated for them. Ranges that
/// overlap are charged for each line again, so malformed ranges cannot amplify
/// the work.
pub(super) fn charge_column_ranges(
    ranges: Option<&[MulticolTextFragment]>,
    count: usize,
    work: &mut ProjectionWork<'_, '_>,
) -> Result<(), LayoutError> {
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
    Ok(())
}

/// The column each line of a paragraph is in, and where that column starts.
pub(super) struct LineColumns {
    /// The column (fragmentainer) of each line.
    pub(super) column: Vec<usize>,
    /// The offset along the block axis at which the column of each line
    /// starts. Zero unless the column layout recorded where its fragments
    /// start.
    pub(super) origin: Vec<f32>,
}

impl LineColumns {
    /// The columns of `count` lines in the `ranges` a paragraph in columns is
    /// split into. A line that several ranges hold belongs to the first of
    /// them; a line that none holds, and every line of a paragraph that is
    /// not in columns, belongs to `fallback_column`.
    pub(super) fn new(
        ranges: Option<&[MulticolTextFragment]>,
        count: usize,
        fallback_column: usize,
        origins_recorded: bool,
    ) -> Self {
        let mut line_columns = vec![fallback_column; count];
        let mut column_origins = vec![0.0; count];
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
        Self {
            column: line_columns,
            origin: column_origins,
        }
    }
}

/// The inline elements of the paragraph `root` that are listed by their line
/// pieces, and not as a whole.
pub(super) fn split_inline_elements(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    node: &crate::Node,
    pieces: &[crate::layout::InlineBoxPiece],
) -> HashSet<usize> {
    // A translucent `::first-line` composites the first line apart from
    // the others, so the paint order lists the line pieces of the inline
    // elements on the first line and on later ones. An element on the
    // first line that the `::first-line` gives other values is listed by
    // its pieces too, so that the piece on the first line carries them.
    let mut split: HashSet<usize> =
        if crate::paint_rules::first_line_opacity_layer(document, root).is_some() {
            let first: HashSet<usize> = pieces
                .iter()
                .filter(|piece| piece.line == 0)
                .map(|piece| piece.node)
                .collect();
            pieces
                .iter()
                .filter(|piece| piece.line != 0 && first.contains(&piece.node))
                .map(|piece| piece.node)
                .collect()
        } else {
            HashSet::new()
        };
    split.extend(
        pieces
            .iter()
            .filter(|piece| {
                piece.line == 0
                    && crate::generated_content::generated_origin(piece.node).is_none()
                    && first_line_restyles(node, cascade, piece.node)
            })
            .map(|piece| piece.node),
    );
    split
}

/// Whether the `::first-line` around the inline element `element` of the
/// paragraph `root` gives it a color or font size other than its own, which
/// are the inherited values that change how its box paints (`currentColor`
/// and lengths in `em`), so that its box on the first line is painted with
/// other values than its box on later lines.
pub(crate) fn first_line_restyles(
    root: &crate::Node,
    cascade: &CascadeResult,
    element: usize,
) -> bool {
    root.ifc_first_line_element_style(element)
        .is_some_and(|style| {
            crate::generated_content::computed_for_id(cascade, element)
                .is_some_and(|own| style.color != own.color || style.font_size != own.font_size)
        })
}

/// The style of a generated inline box piece on paragraph line `line`: the
/// retained style of a typographic pseudo-element such as `::first-letter`,
/// which depends on the text it wraps; on the first line, the values the
/// `::first-line` around the box gives it; or else the pseudo-element's
/// computed style.
pub(crate) fn generated_piece_style<'a>(
    root: &'a crate::Node,
    cascade: &'a CascadeResult,
    node: usize,
    source_container: Option<usize>,
    source_owner: Option<usize>,
    line: usize,
) -> Option<&'a raikiri_style::ComputedValues> {
    root.ifc_typographic_fragment(node, source_container, source_owner)
        .map(|(style, _)| style)
        .or_else(|| {
            root.ifc_first_line_element_style(node)
                .filter(|_| line == 0)
        })
        .or_else(|| crate::generated_content::computed_for_id(cascade, node))
}
