//! Paint-order and paint-eligibility predicates shared by the paint walker and
//! the page paint-order event generator.

use crate::Document;
use raikiri_style::property::{
    BorderCollapseValue, ColumnCountValue, ContentComponent, DisplayValue, EmptyCellsValue,
    FloatValue, OverflowValue, OverflowXY, PositionValue, Visibility, WhiteSpaceCollapse,
    ZIndexValue,
};
use raikiri_style::{CascadeResult, ComputedColumnWidth, ComputedValues, PseudoElem, StyleNodeId};
use raikiri_traits::{NodeKind, PaintClip, PaintInsets, PaintRect};

/// Whether `node_id` is painted on the page whose name is `active_page_name`.
///
/// `None` skips the check. An inline `<canvas>` and floated boxes stay on
/// the page layout placed them on even when their `page` value names
/// another page.
pub fn named_page_matches(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    active_page_name: Option<Option<&str>>,
) -> bool {
    match active_page_name {
        None => true,
        Some(active) => {
            // An inline canvas is a boundary marker for pagination, but its
            // `page` value does not assign the replaced inline box to the
            // named page. Keep painting it on the page selected by layout.
            let is_inline_canvas = document.get_node(node_id).is_some_and(|node| {
                node.tag_name()
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("canvas"))
                    && matches!(cascade.computed[node_id].display, DisplayValue::Inline)
            });
            if is_inline_canvas {
                true
            } else {
                match cascade.page_values.get(node_id) {
                    Some(raikiri_style::property::PageValue::Named(name)) => {
                        // Floats retain the preceding page in the page-name-float
                        // cases; do not hide the floated box merely because its
                        // inherited page value names the following page.
                        if matches!(
                            cascade.computed[node_id].float,
                            FloatValue::Left
                                | FloatValue::Right
                                | FloatValue::InlineStart
                                | FloatValue::InlineEnd
                                | FloatValue::Footnote
                        ) {
                            true
                        } else {
                            active.is_some_and(|page| name.0.as_str() == page)
                        }
                    }
                    _ => true,
                }
            }
        }
    }
}

/// Whether the element clips its descendants to its padding box.
pub fn clips_overflow(cv: &ComputedValues) -> bool {
    // Internal table tracks are not block, flex, or grid containers, so
    // overflow does not turn their materialized boxes into clipping boxes.
    if matches!(
        cv.display,
        DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
            | DisplayValue::TableColumn
            | DisplayValue::TableColumnGroup
    ) {
        return false;
    }
    !matches!(cv.overflow.x, OverflowValue::Visible)
        || !matches!(cv.overflow.y, OverflowValue::Visible)
}

pub(crate) fn clips_element_overflow(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let Some(node) = document.get_node(node_id) else {
        return false;
    };
    let Some(cv) = cascade.computed.get(node_id) else {
        return false;
    };
    if !node.is_in_document()
        || node.is_non_rendered_html_element()
        || node.kind() != NodeKind::Element
        || !clips_overflow(cv)
        || matches!(cv.display, DisplayValue::None | DisplayValue::Contents)
        // A non-replaced inline box has no overflow clip, but the cascade
        // keeps `inline` on elements that layout blockifies (flex and grid
        // items, absolutely positioned boxes) or lays out as atomic inlines.
        || (cv.display == DisplayValue::Inline
            && !node.is_inline_svg_root()
            && !matches!(node.tag_name(), Some("img" | "canvas"))
            && !crate::layout::ifc::assign::generates_own_box(document, cascade, node_id))
    {
        return false;
    }
    if node.tag_name() == Some("html") && node.parent == Some(document.root_index()) {
        return false;
    }
    if node.tag_name() == Some("body")
        && let Some(parent_id) = node.parent
        && let Some(parent) = document.get_node(parent_id)
        && parent.tag_name() == Some("html")
        && parent.parent == Some(document.root_index())
        && cascade
            .computed
            .get(parent_id)
            .is_some_and(|root| !clips_overflow(root))
    {
        return false;
    }
    true
}

/// Resolve the local overflow clip of a whole border box in paint coordinates.
///
/// The caller supplies the box before page cuts and its used physical border
/// widths. Percentages and overlap are resolved on that box before insetting;
/// padding-edge curves are never normalized again after an opposite edge crops
/// them. Open axes impose no bound and do not form rounded corners.
///
/// The padding origin is floored to include pixel-snapped descendants. An
/// `overflow: clip` far edge is floored too. Viewport-propagated overflow and
/// ordinary inline boxes do not produce a local clip.
pub fn overflow_clip(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    border_box: PaintRect,
    border: PaintInsets,
) -> Option<PaintClip> {
    if !clips_element_overflow(document, cascade, node_id) {
        return None;
    }
    Some(OverflowClipGeometry::new(&cascade.computed[node_id], border_box, border).at(border_box))
}

/// Position-independent clip geometry shared by every placement of a source.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OverflowClipGeometry {
    border: PaintInsets,
    overflow: OverflowXY,
    corner_radii: Option<[[f32; 2]; 4]>,
}

impl OverflowClipGeometry {
    pub(crate) fn new(cv: &ComputedValues, border_box: PaintRect, border: PaintInsets) -> Self {
        let corner_radii =
            if cv.overflow.x != OverflowValue::Visible && cv.overflow.y != OverflowValue::Visible {
                let mut radii = cv.border_radius.used(border_box.width, border_box.height);
                for (corner, [x, y]) in radii.iter_mut().zip([
                    [border.left, border.top],
                    [border.right, border.top],
                    [border.right, border.bottom],
                    [border.left, border.bottom],
                ]) {
                    let rx = (corner[0] - x).max(0.0);
                    let ry = (corner[1] - y).max(0.0);
                    *corner = if rx > 0.0 && ry > 0.0 {
                        [rx, ry]
                    } else {
                        [0.0; 2]
                    };
                }
                radii.iter().any(|corner| corner[0] > 0.0).then_some(radii)
            } else {
                None
            };
        Self {
            border,
            overflow: cv.overflow,
            corner_radii,
        }
    }

    /// Snap edges only after translating the whole border box to its page.
    pub(crate) fn at(self, border_box: PaintRect) -> PaintClip {
        let x0 = (border_box.x + self.border.left).floor();
        let y0 = (border_box.y + self.border.top).floor();
        let right = border_box.x + border_box.width - self.border.right;
        let bottom = border_box.y + border_box.height - self.border.bottom;
        let x1 = if self.overflow.x == OverflowValue::Clip {
            right.floor()
        } else {
            right
        };
        let y1 = if self.overflow.y == OverflowValue::Clip {
            bottom.floor()
        } else {
            bottom
        };
        let mut clip = PaintClip::new(PaintRect::new(
            x0,
            y0,
            (x1 - x0).max(0.0),
            (y1 - y0).max(0.0),
        ));
        clip.clip_x = self.overflow.x != OverflowValue::Visible;
        clip.clip_y = self.overflow.y != OverflowValue::Visible;
        clip.corner_radii = self.corner_radii;
        clip
    }
}

/// Whether the element has a box of its own to paint: its background,
/// borders and outline. A `display: contents` element has none (CSS
/// Display 3 §2.5), even where layout keeps a node for its contents.
pub fn generates_box(cv: &ComputedValues) -> bool {
    !matches!(cv.display, DisplayValue::Contents | DisplayValue::None)
}

/// The group opacity the element paints its subtree with, if any.
///
/// An element with `display: contents` generates no box (CSS Display 3
/// §2.5), so its opacity has nothing to apply to: its children paint as if
/// they were children of its parent.
pub fn opacity_layer(cv: &ComputedValues) -> Option<f32> {
    (cv.opacity < 1.0 && cv.display != DisplayValue::Contents).then_some(cv.opacity)
}

/// The group opacity a `::before`, `::after` or `::first-letter` laid out in
/// its paragraph paints its boxes, images and text with, by the generated id
/// of [`crate::generated_content::generated_node_id`]. Floated and
/// out-of-flow pseudo-elements are not in the paragraph and form no group
/// there.
pub fn pseudo_opacity_layer(cascade: &CascadeResult, id: usize) -> Option<f32> {
    let (_, pseudo) = crate::generated_content::generated_origin(id)?;
    if !matches!(
        pseudo,
        PseudoElem::Before | PseudoElem::After | PseudoElem::FirstLetter
    ) {
        return None;
    }
    let cv = crate::generated_content::computed_for_id(cascade, id)?;
    (matches!(cv.position, PositionValue::Static | PositionValue::Relative)
        && matches!(cv.float, FloatValue::None))
    .then(|| opacity_layer(cv))
    .flatten()
}

/// The group opacity one fragment of a `::first-letter` laid out in its
/// paragraph, by the generated id `id` of its box, paints its boxes and
/// letters with. A fragment keeps the style it was given for the text it
/// wraps, `retained`, and composites with that style's opacity; without
/// one, it takes the pseudo-element's (see [`pseudo_opacity_layer`]).
pub fn letter_opacity_layer(
    cascade: &CascadeResult,
    id: usize,
    retained: Option<&ComputedValues>,
) -> Option<f32> {
    let Some(retained) = retained else {
        return pseudo_opacity_layer(cascade, id);
    };
    let cv = crate::generated_content::computed_for_id(cascade, id)?;
    (matches!(cv.position, PositionValue::Static | PositionValue::Relative)
        && matches!(cv.float, FloatValue::None))
    .then(|| opacity_layer(retained))
    .flatten()
}

/// The group opacity the `::first-line` boxes around the first line of
/// paragraph `root` paint that line's content with, if any.
///
/// A `::first-line` is a fictional inline box wrapping the content of its
/// block's first formatted line, inside the block and outside the inline
/// elements on the line, which it splits at the line's end (CSS Pseudo 4
/// §2.1.1). Its opacity composites that content as one group (§2.1.2, CSS
/// Color 4 §3.3). The first line of a paragraph is also the first formatted
/// line of each ancestor block it starts, so their `::first-line` boxes
/// nest around the same content, and their opacities multiply. A paragraph
/// that starts with a block child has no first line of its own.
pub fn first_line_opacity_layer(document: &Document, root: usize) -> Option<f32> {
    let node = document.get_node(root)?;
    if node.kind() != NodeKind::Element || !node.is_ifc_root() {
        return None;
    }
    let ifc = node.ifc.as_ref()?;
    let lines = ifc.lines.as_ref()?;
    if lines.lines.is_empty() || lines.block_line_starts.iter().any(|&(_, line)| line == 0) {
        return None;
    }
    ifc.first_line_opacity
}

/// Whether `id` is an atomic inline on the first line of paragraph `root`,
/// so the paragraph's `::first-line` wraps it (see
/// [`first_line_opacity_layer`]). Floats and absolutely positioned boxes are
/// not on any line: the `::first-line` box wraps only the inline-level
/// content of the line (CSS Pseudo 4 §2.1.1).
pub fn atomic_on_first_line(document: &Document, root: usize, id: usize) -> bool {
    document
        .get_node(root)
        .and_then(|node| node.ifc_lines())
        .and_then(|lines| lines.first())
        .is_some_and(|line| {
            line.fragments().any(|fragment| {
                matches!(fragment, shodo::Fragment::Atomic(atomic) if atomic.node.0 as usize == id)
            })
        })
}

/// The stack level a pseudo-element group of [`pseudo_opacity_layer`]
/// composites at among the other groups of its paragraph: the `z-index` of a
/// relatively positioned pseudo-element, and 0 otherwise, the level of the
/// non-positioned groups (CSS 2.1 §9.9.1, Appendix E steps 8 and 9).
pub fn pseudo_stack_level(cascade: &CascadeResult, id: usize) -> i32 {
    crate::generated_content::computed_for_id(cascade, id).map_or(0, |cv| {
        match (&cv.position, cv.z_index) {
            (PositionValue::Relative, ZIndexValue::Integer(level)) => level,
            _ => 0,
        }
    })
}

/// Whether an in-flow block's inline content (its lines and the atomic
/// inlines and floats laid out beside them) waits for the enclosing layer.
///
/// CSS 2.1 Appendix E paints the backgrounds of all in-flow, non-positioned
/// block-level descendants of a stacking context (step 4) before any of
/// their inline content (step 7), so a later sibling's background never
/// covers an earlier sibling's overflowing text. The painters defer the
/// inline content of a plain block in a block container until the end of
/// the nearest ancestor that does not defer, which closes the layer.
///
/// The parts of a table in such a block defer too: the table's, rows' and
/// cells' backgrounds are block backgrounds of the layer, below the text of
/// every cell.
///
/// A block that groups, transforms, fragments into columns, floats or is
/// positioned paints its content in place: deferring past it would move the
/// content out of its group. A block that clips defers its content in a
/// layer of its own instead (see [`merges_clipped_inline_content`]).
pub fn defers_inline_content(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    cascade
        .computed
        .get(node_id)
        .is_some_and(|cv| !clips_overflow(cv))
        && defers_unless_clipped(document, cascade, node_id)
}

/// Whether a block that clips its overflow would otherwise defer its inline
/// content (see [`defers_inline_content`]).
///
/// The painters collect the block's inline content, and the positioned boxes
/// inside it, in a layer of its own, and hand that layer to the enclosing
/// one inside the block's clip once the block's subtree is done. The text
/// stays clipped, and still paints above the backgrounds of the blocks that
/// follow (CSS 2.1 Appendix E steps 4 and 7).
pub fn merges_clipped_inline_content(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    cascade.computed.get(node_id).is_some_and(clips_overflow)
        && defers_unless_clipped(document, cascade, node_id)
}

/// [`defers_inline_content`] without its overflow condition.
fn defers_unless_clipped(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let Some(cv) = cascade.computed.get(node_id) else {
        return false;
    };
    let parent_display = box_parent(document, cascade, node_id)
        .and_then(|parent| cascade.computed.get(parent))
        .map(|parent| parent.display);
    let fits_parent = match cv.display {
        DisplayValue::Block
        | DisplayValue::FlowRoot
        | DisplayValue::ListItem
        | DisplayValue::Table => parent_display.is_some_and(is_block_container),
        DisplayValue::TableCaption
        | DisplayValue::TableRowGroup
        | DisplayValue::TableHeaderGroup
        | DisplayValue::TableFooterGroup => parent_display == Some(DisplayValue::Table),
        DisplayValue::TableRow => matches!(
            parent_display,
            Some(
                DisplayValue::Table
                    | DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
            )
        ),
        DisplayValue::TableCell => parent_display == Some(DisplayValue::TableRow),
        _ => false,
    };
    fits_parent
        && matches!(cv.position, PositionValue::Static)
        && matches!(cv.float, FloatValue::None)
        && !establishes_stacking_context(cv)
        && matches!(cv.column_count, ColumnCountValue::Auto)
        && matches!(cv.column_width, ComputedColumnWidth::Auto)
}

/// Whether boxes of `display` hold the block-level boxes they contain in
/// a block formatting context of their own or of their parent.
fn is_block_container(display: DisplayValue) -> bool {
    matches!(
        display,
        DisplayValue::Block
            | DisplayValue::FlowRoot
            | DisplayValue::ListItem
            | DisplayValue::InlineBlock
            | DisplayValue::TableCell
            | DisplayValue::TableCaption
    )
}

/// Whether a non-positioned box still forms a group of its own: an opacity
/// group, a transform or a filter.
fn establishes_stacking_context(cv: &ComputedValues) -> bool {
    opacity_layer(cv).is_some() || !cv.transform.is_empty() || !cv.filter.is_empty()
}

/// Whether a positioned box, or a box that forms a stacking context, waits
/// for the end of the enclosing layer.
///
/// Such boxes with a stack level of zero or more paint after the in-flow
/// blocks of their stacking context and after its inline content (CSS 2.1
/// Appendix E steps 8 and 9), so a block that follows one in the tree still
/// paints its background below it. The painters keep these boxes in tree
/// order at the end of the layer of the nearest ancestor that closes one.
///
/// Boxes laid out beside the lines of a paragraph keep their place among the
/// paragraph's content, as do boxes with a negative stack level, which
/// paint below the blocks of the layer.
pub fn defers_to_positioned_layer(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    cascade.computed.get(node_id).is_some_and(|cv| {
        let positioned = !matches!(cv.position, PositionValue::Static);
        let negative = positioned && matches!(cv.z_index, ZIndexValue::Integer(level) if level < 0);
        document
            .get_node(node_id)
            .is_some_and(|node| node.kind() == NodeKind::Element)
            && generates_box(cv)
            && (positioned || establishes_stacking_context(cv))
            && !negative
            && box_parent(document, cascade, node_id).is_some_and(|parent| {
                document
                    .get_node(parent)
                    .is_some_and(|node| !node.is_ifc_root())
                    && cascade.computed.get(parent).is_some_and(|parent| {
                        // A child of a table part sits in an anonymous cell;
                        // the children of flex and grid containers join the
                        // layer the same way.
                        is_block_container(parent.display)
                            || matches!(
                                parent.display,
                                DisplayValue::Flex
                                    | DisplayValue::InlineFlex
                                    | DisplayValue::Grid
                                    | DisplayValue::InlineGrid
                                    | DisplayValue::Table
                                    | DisplayValue::InlineTable
                                    | DisplayValue::TableRowGroup
                                    | DisplayValue::TableHeaderGroup
                                    | DisplayValue::TableFooterGroup
                                    | DisplayValue::TableRow
                            )
                    })
            })
    })
}

/// Whether a box hands the positioned boxes it holds to the enclosing layer.
///
/// Positioned descendants belong to the nearest stacking context, where they
/// paint after its inline content (CSS 2.1 Appendix E step 8), whatever box
/// between holds them. A float or a flex or grid item paints as if it formed
/// a stacking context, but passes them on too. A box that clips or forms a
/// stacking context of its own keeps them, as does a box laid out beside the
/// lines of a paragraph, which paints among the paragraph's content.
pub fn hoists_positioned_content(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    cascade.computed.get(node_id).is_some_and(|cv| {
        matches!(cv.position, PositionValue::Static)
            && !establishes_stacking_context(cv)
            && !is_z_indexed_item(document, cascade, node_id, cv)
            && !clips_overflow(cv)
            && (!matches!(cv.float, FloatValue::None)
                || box_parent(document, cascade, node_id).is_some_and(|parent| {
                    document
                        .get_node(parent)
                        .is_some_and(|node| !node.is_ifc_root())
                }))
    })
}

/// Whether a flex or grid item with computed values `cv` has an integer
/// `z-index`, which makes it a stacking context even when it is not
/// positioned (CSS Flexbox 1 §4.3, CSS Grid 1 §9.5).
fn is_z_indexed_item(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    cv: &ComputedValues,
) -> bool {
    matches!(cv.z_index, ZIndexValue::Integer(_))
        && box_parent(document, cascade, node_id)
            .and_then(|parent| cascade.computed.get(parent))
            .is_some_and(|parent| {
                matches!(
                    parent.display,
                    DisplayValue::Flex
                        | DisplayValue::InlineFlex
                        | DisplayValue::Grid
                        | DisplayValue::InlineGrid
                )
            })
}

/// Whether box `id`, laid out in paragraph `root`, is an in-flow block
/// inside the paragraph's inline content (CSS 2.1 §9.2.1.1) that paints
/// before the paragraph's lines.
///
/// Its background is a block background of the layer (CSS 2.1 Appendix E
/// step 4), below the lines around it (step 7), and its own inline content
/// defers like any other block's. Blocks inside an inline element, or on
/// the first line, stay with the paragraph's content and its groups. The
/// painters keep all blocks of a paragraph with the paragraph's content when
/// [`holds_negative_stack_level`] says it holds content that paints below
/// the blocks.
pub fn paints_before_paragraph_lines(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
) -> bool {
    cascade.computed.get(id).is_some_and(|cv| {
        matches!(
            cv.display,
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::Table
        ) && matches!(cv.position, PositionValue::Static)
            && matches!(cv.float, FloatValue::None)
            && !establishes_stacking_context(cv)
    }) && document
        .get_node(id)
        .is_some_and(|node| node.kind() == NodeKind::Element)
        && box_parent(document, cascade, id) == Some(root)
        && !atomic_on_first_line(document, root, id)
}

/// Whether the inline content of paragraph `root` holds a box, an inline
/// element or a `::before` or `::after` positioned with a negative
/// `z-index`.
///
/// Such content paints below the in-flow blocks of the layer (CSS 2.1
/// Appendix E step 3), and the painters list it with the paragraph, so the
/// blocks inside the paragraph (see [`paints_before_paragraph_lines`]) stay
/// there too. Only the paragraph's own inline content is searched, not the
/// blocks inside it or the content of inline boxes that form stacking
/// contexts.
pub fn holds_negative_stack_level(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> bool {
    let negative = |key: usize| {
        crate::generated_content::computed_for_id(cascade, key).is_some_and(|cv| {
            !matches!(cv.position, PositionValue::Static)
                && matches!(cv.z_index, ZIndexValue::Integer(level) if level < 0)
        })
    };
    let pseudos = |id: usize| {
        [PseudoElem::Before, PseudoElem::After]
            .map(|pseudo| crate::generated_content::generated_node_id(id, pseudo))
    };
    if pseudos(root).into_iter().any(negative) {
        return true;
    }
    let mut stack = document
        .get_node(root)
        .map(|node| node.children.clone())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(node) = document
            .get_node(id)
            .filter(|node| node.kind() == NodeKind::Element)
        else {
            continue;
        };
        if negative(id) {
            return true;
        }
        // Content inside an inline box that forms a stacking context paints
        // within that box, not below the paragraph's blocks.
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: elements in a laid-out paragraph have computed values
        };
        let stacking = establishes_stacking_context(cv)
            || (!matches!(cv.position, PositionValue::Static)
                && matches!(cv.z_index, ZIndexValue::Integer(_)));
        if stacking {
            continue;
        }
        if pseudos(id).into_iter().any(negative) {
            return true;
        }
        if matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents) {
            stack.extend(node.children.iter().copied());
        }
    }
    false
}

/// The nearest ancestor that generates a box, skipping `display: contents`
/// elements, whose children take their place (CSS Display 3 §2.5).
fn box_parent(document: &Document, cascade: &CascadeResult, node_id: usize) -> Option<usize> {
    let mut parent = document.parent_of(node_id)?;
    while cascade
        .computed
        .get(parent)
        .is_some_and(|cv| cv.display == DisplayValue::Contents)
    {
        parent = document.parent_of(parent)?;
    }
    Some(parent)
}

/// Whether the inline content deferred in front of an element paints
/// before it.
///
/// Positioned boxes and other stacking contexts paint above inline content
/// (CSS 2.1 Appendix E steps 8 and 9); most of them wait for the end of the
/// layer instead (see [`defers_to_positioned_layer`]). Flex containers,
/// atomic inlines, tables that do not defer and other boxes that are not
/// block containers keep the inline content before them in tree order. Floats (step 5) and in-flow
/// blocks that clip or fragment into columns (step 4) paint below it, so
/// the earlier text stays above them. An element without a box, such as a
/// `display: none` or `display: contents` one, closes nothing.
pub fn closes_deferred_inline_content(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let Some(cv) = cascade.computed.get(node_id) else {
        return false;
    };
    let below_inline_content = matches!(cv.position, PositionValue::Static)
        && !establishes_stacking_context(cv)
        && (!matches!(cv.float, FloatValue::None)
            || matches!(
                cv.display,
                DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
            ));
    document
        .get_node(node_id)
        .is_some_and(|node| node.kind() == NodeKind::Element)
        && generates_box(cv)
        && !below_inline_content
        && !defers_inline_content(document, cascade, node_id)
        && !merges_clipped_inline_content(document, cascade, node_id)
}

/// A `visibility: hidden` table does not paint its own box.
pub fn is_visibility_hidden_table(cv: &ComputedValues) -> bool {
    cv.visibility == Visibility::Hidden
        && matches!(cv.display, DisplayValue::Table | DisplayValue::InlineTable)
}

/// A hidden table row or row group does not paint its own decorations.
pub fn is_visibility_hidden_table_part(cv: &ComputedValues) -> bool {
    cv.visibility == Visibility::Hidden
        && matches!(
            cv.display,
            DisplayValue::TableRow
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
        )
}

/// Whether a table row or row group has a background or border to paint.
///
/// A part without one paints nothing inside its cells, so it needs no
/// per-cell clip.
pub fn paints_table_part_decoration(cv: &ComputedValues) -> bool {
    cv.background_color.a > 0
        || cv.has_background_image()
        || [
            &cv.border.top,
            &cv.border.right,
            &cv.border.bottom,
            &cv.border.left,
        ]
        .into_iter()
        .any(|side| side.width().px() > 0.0)
}

/// Stable-sorts a parent's children into paint order.
pub fn sort_paint_children(
    children: &mut [usize],
    parent_display: DisplayValue,
    cascade: &CascadeResult,
) {
    // Stable sorting keeps DOM order for equal keys.
    children.sort_by_key(|&child| paint_child_key(child, parent_display, cascade));
}

/// The key [`sort_paint_children`] sorts a child by: its stacking bucket and
/// z-index, then floats after other boxes of the same level, then the flex or
/// grid `order`. A stacking context that is neither positioned nor a flex or
/// grid item, such as an element with `opacity` below 1, paints at
/// [`STACKING_CONTEXT_PAINT_KEY`].
pub(crate) fn paint_child_key(
    child: usize,
    parent_display: DisplayValue,
    cascade: &CascadeResult,
) -> PaintChildKey {
    let order_sensitive_container = matches!(
        parent_display,
        DisplayValue::Flex
            | DisplayValue::InlineFlex
            | DisplayValue::Grid
            | DisplayValue::InlineGrid
    );
    // Direct abspos/fixed children are not flex/grid items: paint them as if
    // their order were zero, while in-flow (including relative) items use the
    // computed `order` value. Stacking buckets stay primary.
    // Anonymous paragraph keys are outside the source DOM arena. Their
    // non-inherited position, float and order use initial values.
    let Some(computed) = cascade.computed.get(child) else {
        return ((1, 0), 0, 0);
    };
    let stack = if order_sensitive_container && matches!(computed.position, PositionValue::Static) {
        // A flex/grid item can use integer z-index even when static.
        // Match positioned items' stack levels so order only breaks
        // ties within the same z-index level.
        match computed.z_index {
            ZIndexValue::Auto => (1, 0),
            ZIndexValue::Integer(value) if value < 0 => (0, value),
            ZIndexValue::Integer(value) => (2, value),
            _ => paint_order_key(cascade, child), // cov:ignore: defensive fallback for future non-exhaustive z-index variants.
        }
    } else {
        paint_order_key(cascade, child)
    };
    let order = if order_sensitive_container
        && !matches!(
            computed.position,
            PositionValue::Absolute | PositionValue::Fixed
        ) {
        computed.order
    } else {
        0
    };
    let float_paint_order = if !order_sensitive_container
        && matches!(
            computed.float,
            FloatValue::Left | FloatValue::Right | FloatValue::InlineStart | FloatValue::InlineEnd
        ) {
        1
    } else {
        0
    };
    (stack, float_paint_order, order)
}

/// See [`paint_child_key`].
pub(crate) type PaintChildKey = ((u8, i32), u8, i32);

/// The paint key of an element with `opacity` below 1 that is not
/// positioned. CSS Color 4 §3.3 paints it on the same layer, within its
/// parent stacking context, as positioned elements with stack level 0, and
/// CSS 2.1 Appendix E step 8 paints that layer in tree order.
pub(crate) const STACKING_CONTEXT_PAINT_KEY: PaintChildKey = ((2, 0), 0, 0);

/// The paint key of a stacking context that is not a flex or grid item and
/// has computed values `cv`, such as a pseudo-element with `opacity` below
/// one: [`STACKING_CONTEXT_PAINT_KEY`] when it is not positioned, else its
/// z-index level.
pub(crate) fn stacking_context_key(cv: &ComputedValues) -> PaintChildKey {
    match cv.position {
        PositionValue::Static => STACKING_CONTEXT_PAINT_KEY,
        _ => (computed_paint_order_key(cv), 0, 0),
    }
}

fn paint_order_key(cascade: &CascadeResult, node_id: usize) -> (u8, i32) {
    computed_paint_order_key(&cascade.computed[node_id])
}

fn computed_paint_order_key(computed: &ComputedValues) -> (u8, i32) {
    match (&computed.position, computed.z_index) {
        // In-flow boxes paint before positioned descendants with an auto
        // z-index. Keep ordinary static boxes in the normal bucket, but
        // place flex containers with positioned descendants alongside
        // auto-z siblings so their later absolute child paints in tree order.
        (PositionValue::Static, _)
            if matches!(
                computed.display,
                DisplayValue::Flex | DisplayValue::InlineFlex
            ) =>
        {
            (2, 0)
        }
        (PositionValue::Static, _) => (1, 0),
        (_, ZIndexValue::Auto) => (2, 0),
        (_, ZIndexValue::Integer(value)) if value < 0 => (0, value),
        (_, ZIndexValue::Integer(value)) => (2, value),
        // PositionValue and ZIndexValue are non-exhaustive.  New variants
        // retain the default/source-order bucket until stacking support grows.
        _ => (1, 0),
    }
}

/// Walk the Document arena in DFS order; return the first `<body>` element index.
///
/// Use an iterative `Vec` stack (as in cascade §deep_nesting) to avoid
/// stack overflow on deep DOMs. Return `None` for fragments (no `<body>`).
///
/// Skip subtrees with `!is_in_document()` (such as `<template>` descendants)
/// so a hypothetical `<body>` in an inert subtree is not selected. The
/// paint-side and layout-side body lookups are separate to avoid crossing
/// module boundaries, but share the same contract.
pub fn find_paint_root(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && crate::layout::is_body_element(doc, id) {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// Whether separate-border `empty-cells: hide` suppresses this cell's box ink.
/// Out-of-flow content does not make a cell nonempty; floated and in-flow
/// elements do, even when their own box has no content (CSS 2.2 section 17.6.1.1).
pub fn hides_empty_table_cell(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let cv = &cascade.computed[node_id];
    if cv.display != DisplayValue::TableCell
        || cv.empty_cells != EmptyCellsValue::Hide
        || cv.border_collapse != BorderCollapseValue::Separate
    {
        return false;
    }
    if cv.visibility == Visibility::Hidden {
        return true;
    }
    lacks_visible_table_cell_content(
        document,
        cascade,
        Some(node_id),
        &document.nodes[node_id].children,
    )
}

/// Classify a layout-only cell using its inherited values and its own content
/// group. Pseudo-elements on the source row belong to that row, not this box.
pub(crate) fn hides_anonymous_table_cell(
    document: &Document,
    cascade: &CascadeResult,
    owner: usize,
    children: &[usize],
) -> bool {
    let cv = &cascade.computed[owner];
    cv.empty_cells == EmptyCellsValue::Hide
        && cv.border_collapse == BorderCollapseValue::Separate
        && (cv.visibility == Visibility::Hidden
            || lacks_visible_table_cell_content(document, cascade, None, children))
}

fn lacks_visible_table_cell_content(
    document: &Document,
    cascade: &CascadeResult,
    generated_owner: Option<usize>,
    children: &[usize],
) -> bool {
    let has_generated_box = |id| {
        [PseudoElem::Before, PseudoElem::After]
            .iter()
            .any(|pseudo| {
                cascade
                    .pseudo
                    .get(&(StyleNodeId::new(id as u64), *pseudo))
                    .is_some_and(|cv| {
                        cv.display != DisplayValue::None
                            && cv.visibility != Visibility::Hidden
                            && !matches!(
                                cv.position,
                                PositionValue::Absolute | PositionValue::Fixed
                            )
                            && !cv.content.is_empty()
                            && !cv
                                .content
                                .iter()
                                .any(|component| matches!(component, ContentComponent::None))
                    })
            })
    };
    // Generated boxes count as in-flow or floating content, including a
    // generated empty inline box, just like an authored empty inline element.
    if generated_owner.is_some_and(has_generated_box) {
        return false;
    }
    let mut stack = children.to_vec();
    while let Some(id) = stack.pop() {
        let node = &document.nodes[id];
        let cv = &cascade.computed[id];
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || cv.display == DisplayValue::None
            || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        {
            continue;
        }
        if cv.visibility == Visibility::Hidden {
            if has_generated_box(id) {
                return false;
            }
            stack.extend(&node.children);
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                let text = node.text_content().unwrap_or("");
                if text.chars().any(|c| {
                    !matches!(c, ' ' | '\t' | '\n' | '\r')
                        || match cv.effective_white_space_collapse {
                            WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard => false,
                            WhiteSpaceCollapse::PreserveBreaks => matches!(c, '\n' | '\r'),
                            _ => true,
                        }
                }) {
                    return false;
                }
            }
            NodeKind::Element if cv.display == DisplayValue::Contents => {
                if has_generated_box(id) {
                    return false;
                }
                stack.extend(&node.children)
            }
            NodeKind::Element => return false,
            _ => {} // cov:ignore: Comment/PI membership is cleared by mark_in_document_flags; document/fragment roots cannot occur in a valid cell subtree.
        }
    }
    true
}

#[cfg(test)]
mod tests;
