//! Paint-order and paint-eligibility predicates shared by the paint walker and
//! the page paint-order event generator.

use crate::Document;
use crate::layout::{BoxParentScope, box_parent};
use raikiri_style::property::{
    BorderCollapseValue, ColumnCountValue, ContentComponent, DisplayValue, EmptyCellsValue,
    FloatValue, OverflowValue, OverflowXY, PositionValue, Visibility, VisualBox,
    WhiteSpaceCollapse, WillChange, ZIndexValue,
};
use raikiri_style::resolve::ComputedOverflowClipMargin;
use raikiri_style::{CascadeResult, ComputedColumnWidth, ComputedValues, PseudoElem, StyleNodeId};
use raikiri_traits::{NodeKind, PaintClip, PaintInsets, PaintRect};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

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
                    && cascade.computed[node_id].display.is_inline_flow()
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
        || (cv.display.is_inline_flow()
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
/// widths; the padding comes from the node's layout. Percentages and overlap
/// are resolved on that box before insetting; padding-edge curves are never
/// normalized again after an opposite edge crops them. Open axes impose no
/// bound and do not form rounded corners.
///
/// The clip origin is floored to include pixel-snapped descendants. An
/// `overflow: clip` far edge is floored too. Viewport-propagated overflow and
/// ordinary inline boxes do not produce a local clip.
pub fn overflow_clip(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    border_box: PaintRect,
    border: PaintInsets,
) -> Option<PaintClip> {
    let clip = clips_element_overflow(document, cascade, node_id).then(|| {
        let padding = document.nodes[node_id].layout.padding;
        let padding = PaintInsets::new(padding.top, padding.right, padding.bottom, padding.left);
        OverflowClipGeometry::new(
            &cascade.computed[node_id],
            border_box,
            border,
            padding,
            is_replaced_element(document, node_id),
        )
        .at(border_box)
    });
    if !document
        .get_node(node_id)
        .is_some_and(|node| node.clips_collapsed_table_span)
    {
        return clip;
    }
    // A cell that spans a collapsed row keeps the rows that remain and clips
    // its contents to its border box (CSS Tables 3, "Visibility-collapse cell
    // rendering"). An axis the element already clips keeps its own edges
    // where they lie inside the border box; an `overflow-clip-margin` that
    // reaches past it is cut back to the border box.
    let Some(clip) = clip else {
        return Some(PaintClip::new(border_box));
    };
    let (x, width) = clamp_clip_axis(
        clip.clip_x,
        clip.rect.x,
        clip.rect.width,
        border_box.x,
        border_box.width,
    );
    let (y, height) = clamp_clip_axis(
        clip.clip_y,
        clip.rect.y,
        clip.rect.height,
        border_box.y,
        border_box.height,
    );
    let rect = PaintRect::new(x, y, width, height);
    if rect == clip.rect && clip.clip_x && clip.clip_y {
        return Some(clip);
    }
    // The element's curves were shaped for its own clip edge, so a trimmed
    // or newly bounded rectangle takes square corners.
    Some(PaintClip::new(rect))
}

/// One axis of a collapsed-span clip: the element's own clip edges intersected
/// with the border box, or the border box when that axis did not clip.
fn clamp_clip_axis(
    clips: bool,
    start: f32,
    size: f32,
    box_start: f32,
    box_size: f32,
) -> (f32, f32) {
    if !clips {
        return (box_start, box_size);
    }
    let lo = start.max(box_start);
    let hi = (start + size).min(box_start + box_size);
    (lo, (hi - lo).max(0.0))
}

/// Whether the element is a replaced element, whose overflow and
/// `overflow-clip-margin` resolve differently (CSS Overflow 4 §3.1, §3.2).
pub(crate) fn is_replaced_element(document: &Document, node_id: usize) -> bool {
    document.get_node(node_id).is_some_and(|node| {
        node.is_inline_svg_root()
            || matches!(
                node.tag_name(),
                Some("img" | "canvas" | "video" | "audio" | "iframe" | "embed" | "object")
            )
    })
}

/// Whether the element lays out as a replaced box with a natural size: it
/// keeps that size instead of stretching to fill an auto width or a
/// `normal`-aligned grid area (CSS 2.1 §10.3.4, CSS Grid 1 §6.2). Replaced
/// content without a resolved natural size (an `<iframe>`, an unloaded
/// `<video>`, an `<object>` showing fallback) and inline SVG roots, whose auto
/// width is 100%, keep the ordinary block sizing.
pub(crate) fn lays_out_as_replaced(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    is_replaced_element(document, node_id)
        && !node.is_inline_svg_root()
        && node.image_intrinsic_box().is_some()
}

/// Resolve the clip of a replaced element's own content (its image or canvas
/// bitmap) in paint coordinates, from its used border box, border widths and
/// padding. Each clipping axis clips at the overflow clip edge, the content
/// box under the UA default `overflow-clip-margin: content-box`; a visible
/// axis lets the content paint outside its box (CSS Overflow 3 §3.1). The
/// edges snap like [`overflow_clip`], whose clip the page paint order puts
/// around the same content.
pub fn replaced_content_clip(
    cv: &ComputedValues,
    border_box: PaintRect,
    border: PaintInsets,
    padding: PaintInsets,
) -> Option<PaintClip> {
    if cv.overflow.x == OverflowValue::Visible && cv.overflow.y == OverflowValue::Visible {
        return None;
    }
    Some(OverflowClipGeometry::new(cv, border_box, border, padding, true).at(border_box))
}

/// The largest distance an overflow clip edge may lie from the border edge;
/// far beyond any page, so clamping never changes what paints.
const MAX_CLIP_MARGIN: f32 = 1.0e7;

/// Position-independent clip geometry shared by every placement of a source.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OverflowClipGeometry {
    /// How far inside the border box each side of the clip edge lies; a
    /// negative inset lies outside it.
    edges: PaintInsets,
    overflow: OverflowXY,
    corner_radii: Option<[[f32; 2]; 4]>,
}

impl OverflowClipGeometry {
    /// An `overflow: clip` axis clips at the overflow clip edge: the box and
    /// outset that `overflow-clip-margin` names (CSS Overflow 4 §3.2). A
    /// scroll container axis clips at the padding box, or at a `padding-box` or
    /// `content-box` edge that lies inside it. On a `replaced`
    /// element every non-`visible` axis is used as `clip` (CSS Overflow 4
    /// §3.1), and an omitted box is the content box.
    pub(crate) fn new(
        cv: &ComputedValues,
        border_box: PaintRect,
        border: PaintInsets,
        padding: PaintInsets,
        replaced: bool,
    ) -> Self {
        let (overflow, edges) = resolve_overflow_clip_edges(
            cv.overflow,
            cv.overflow_clip_margin,
            border,
            padding,
            replaced,
        );
        let corner_radii =
            if overflow.x != OverflowValue::Visible && overflow.y != OverflowValue::Visible {
                let mut radii = cv.border_radius.used(border_box.width, border_box.height);
                for (corner, [x, y]) in radii.iter_mut().zip([
                    [edges.left, edges.top],
                    [edges.right, edges.top],
                    [edges.right, edges.bottom],
                    [edges.left, edges.bottom],
                ]) {
                    let rx = adjusted_radius(corner[0], x);
                    let ry = adjusted_radius(corner[1], y);
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
            edges,
            overflow,
            corner_radii,
        }
    }

    /// Clip geometry with square corners, for boxes whose painter draws no
    /// `border-radius` (page-margin boxes).
    pub(crate) fn square(
        overflow: OverflowXY,
        margin: ComputedOverflowClipMargin,
        border: PaintInsets,
        padding: PaintInsets,
    ) -> Self {
        let (overflow, edges) =
            resolve_overflow_clip_edges(overflow, margin, border, padding, false);
        Self {
            edges,
            overflow,
            corner_radii: None,
        }
    }

    /// How far the block-end clip edge lies past the border box's block-end
    /// edge: the outset that `overflow-clip-margin` gives an `overflow: clip`
    /// axis, and zero when the edge lies on or inside the border box.
    pub(crate) fn block_end_outset(&self) -> f32 {
        (-self.edges.bottom).max(0.0)
    }

    /// Snap edges only after translating the whole border box to its page.
    pub(crate) fn at(self, border_box: PaintRect) -> PaintClip {
        let x0 = (border_box.x + self.edges.left).floor();
        let y0 = (border_box.y + self.edges.top).floor();
        let right = border_box.x + border_box.width - self.edges.right;
        let bottom = border_box.y + border_box.height - self.edges.bottom;
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

fn resolve_overflow_clip_edges(
    overflow: OverflowXY,
    margin: ComputedOverflowClipMargin,
    border: PaintInsets,
    padding: PaintInsets,
    replaced: bool,
) -> (OverflowXY, PaintInsets) {
    let used = |overflow| {
        if replaced && overflow != OverflowValue::Visible {
            OverflowValue::Clip
        } else {
            overflow
        }
    };
    let overflow = OverflowXY::new(used(overflow.x), used(overflow.y));
    // An omitted box is the content box on a replaced element and the
    // padding box otherwise (CSS Overflow 4 §3.2).
    let visual_box = margin.visual_box.unwrap_or(if replaced {
        VisualBox::ContentBox
    } else {
        VisualBox::PaddingBox
    });
    let margin_edge = |border: f32, padding: f32| {
        let inset = match visual_box {
            VisualBox::BorderBox => 0.0,
            VisualBox::ContentBox => border + padding,
            _ => border,
        };
        // Keep an extreme or infinite offset finite, so the clip
        // rectangle built from these insets stays finite.
        (inset - margin.offset.px()).clamp(-MAX_CLIP_MARGIN, MAX_CLIP_MARGIN)
    };
    // A `clip` axis uses the overflow clip edge as is. A scroll container
    // axis keeps it within the padding box: a `border-box` value is ignored
    // with its offset, and an outward offset stops at the padding box.
    let edge = |overflow, border: f32, padding: f32| match overflow {
        OverflowValue::Clip => margin_edge(border, padding),
        OverflowValue::Visible => border,
        _ if visual_box == VisualBox::BorderBox => border,
        _ => margin_edge(border, padding).max(border),
    };
    let edges = PaintInsets::new(
        edge(overflow.y, border.top, padding.top),
        edge(overflow.x, border.right, padding.right),
        edge(overflow.y, border.bottom, padding.bottom),
        edge(overflow.x, border.left, padding.left),
    );
    (overflow, edges)
}

/// A border-edge corner radius moved `inset` inside the border edge, or
/// outside it when `inset` is negative. Inside, the radius shrinks by the
/// inset. Outside, it grows by the outset, less for a radius smaller than the
/// outset so that a sharp corner stays sharp (CSS Backgrounds 3 §7.1.1, the
/// rule for spread shadows).
fn adjusted_radius(radius: f32, inset: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    if inset >= 0.0 {
        return (radius - inset).max(0.0);
    }
    let outset = -inset;
    let ratio = radius / outset;
    if ratio < 1.0 {
        radius + outset * (1.0 + (ratio - 1.0).powi(3))
    } else {
        radius + outset
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

/// The `display` the box of element `id` is painted with.
///
/// The document element has no parent to take its contents, so a
/// `display: contents` one computes to `block` (CSS Display 3 §2.8), as layout
/// treats it. Every other element is painted with the `display` it computed.
pub fn painted_display(document: &Document, id: usize, cv: &ComputedValues) -> DisplayValue {
    if cv.display == DisplayValue::Contents && document.document_element_index() == Some(id) {
        DisplayValue::Block
    } else {
        cv.display
    }
}

/// [`generates_box`] for the element `id`, counting a `display: contents`
/// document element, which paints the box of a block (see [`painted_display`]).
pub fn element_generates_box(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    !matches!(
        painted_display(document, id, cv),
        DisplayValue::Contents | DisplayValue::None
    )
}

/// [`opacity_layer`] for the element `id`, counting a `display: contents`
/// document element, which paints the box of a block (see [`painted_display`]).
pub fn element_opacity_layer(document: &Document, id: usize, cv: &ComputedValues) -> Option<f32> {
    (cv.opacity < 1.0 && painted_display(document, id, cv) != DisplayValue::Contents)
        .then_some(cv.opacity)
}

/// The group opacity a `::before`, `::after` or `::first-letter` laid out in
/// its paragraph paints its boxes, images and text with, by the generated id
/// of `generated_node_id`. Floated and
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
    (matches!(
        cv.position,
        PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
    ) && matches!(cv.float, FloatValue::None))
    .then(|| opacity_layer(cv))
    .flatten()
}

/// The style of a `::before` or `::after` laid out in its paragraph, by its
/// generated id, when its box can take a `transform`. Only an atomic
/// inline-level pseudo-element can, or one that an image alone in its
/// `content` replaces (CSS Content 3 §2.1): a non-replaced inline box is not
/// transformable (CSS Transforms 1 §3). Floated and out-of-flow
/// pseudo-elements are not in the paragraph.
fn transformable_pseudo(cascade: &CascadeResult, id: usize) -> Option<&ComputedValues> {
    let (_, pseudo) = crate::generated_content::generated_origin(id)?;
    if !matches!(pseudo, PseudoElem::Before | PseudoElem::After) {
        return None;
    }
    let cv = crate::generated_content::computed_for_id(cascade, id)?;
    (matches!(
        cv.position,
        PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
    ) && matches!(cv.float, FloatValue::None)
        && (cv.display.is_atomic_inline_level()
            || matches!(cv.content.as_slice(), [ContentComponent::Image { .. }])))
    .then_some(cv)
}

/// The style of a `::before` or `::after` laid out in its paragraph, by its
/// generated id, when it turns its boxes, images and text by its own
/// `transform` (see [`transformable_pseudo`] for the boxes that can).
pub fn pseudo_transform(cascade: &CascadeResult, id: usize) -> Option<&ComputedValues> {
    transformable_pseudo(cascade, id).filter(|cv| !cv.transform.is_empty())
}

/// Whether the `::before` or `::after` with generated id `id`, laid out in
/// its paragraph, is by its own style the containing block of fixed-position
/// descendants: it has a filter, a transform that applies to it (see
/// [`transformable_pseudo`]), or layout or paint containment, which a
/// non-replaced inline box ignores (CSS Containment 2 §3).
pub fn pseudo_establishes_fixed_containing_block(cascade: &CascadeResult, id: usize) -> bool {
    let (Some((_, PseudoElem::Before | PseudoElem::After)), Some(cv)) = (
        crate::generated_content::generated_origin(id),
        crate::generated_content::computed_for_id(cascade, id),
    ) else {
        return false;
    };
    let transform = !cv.transform.is_empty() || cv.will_change.contains(WillChange::TRANSFORM);
    let filter = !cv.filter.is_empty() || cv.will_change.contains(WillChange::FILTER);
    let contain = cv.contain.contains_positioned_descendants()
        || cv.will_change.contains(WillChange::CONTAIN);
    // An image alone in `content` replaces the box, which makes it atomic
    // however its `display` reads (CSS Content 3 §2.1).
    let replaced = matches!(cv.content.as_slice(), [ContentComponent::Image { .. }]);
    let containable = replaced
        || (!cv.display.is_inline_flow()
            && !matches!(
                cv.display,
                DisplayValue::Contents
                    | DisplayValue::TableRow
                    | DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
                    | DisplayValue::TableColumn
                    | DisplayValue::TableColumnGroup
            ));
    filter || (transform && transformable_pseudo(cascade, id).is_some()) || (contain && containable)
}

/// How the `::before` or `::after` with generated id `id` transforms its box
/// at `rect`, as the painter turns its group: `None` when it has no
/// transform of its own (see [`pseudo_transform`]), `Some(None)` when that
/// transform is singular and the pseudo-element paints nothing (CSS
/// Transforms 1 §5), else the transform about its own box, in the
/// coordinates `rect` is in.
pub fn pseudo_box_transform(
    cascade: &CascadeResult,
    id: usize,
    rect: PaintRect,
) -> Option<Option<raikiri_traits::PaintTransform>> {
    use crate::page_projection::paint_transform::{box_transform, is_singular};
    let cv = pseudo_transform(cascade, id)?;
    Some((!is_singular(cv)).then(|| box_transform(cv, rect)))
}

/// Whether the pseudo-element with generated id `id` composites its boxes,
/// images and text as one group in its paragraph: one with an opacity below
/// one (see [`pseudo_opacity_layer`]), or one with a transform of its own
/// (see [`pseudo_transform`]), which is a stacking context too (CSS
/// Transforms 1 §3).
pub fn pseudo_forms_group(cascade: &CascadeResult, id: usize) -> bool {
    pseudo_opacity_layer(cascade, id).is_some() || pseudo_transform(cascade, id).is_some()
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

/// The node a line fragment lays out, if any.
fn fragment_node(fragment: shodo::Fragment<'_>) -> Option<usize> {
    match fragment {
        shodo::Fragment::GlyphRun(run) => run.node().map(|node| node.0 as usize),
        shodo::Fragment::Atomic(atomic) => Some(atomic.node.0 as usize),
        shodo::Fragment::InlineBox(inline) => Some(inline.node.0 as usize),
        shodo::Fragment::RubyAnnotation(_) | shodo::Fragment::OutOfFlowAnchor(_) => None,
    }
}

/// Whether paragraph `root` has a translucent first line (see
/// [`first_line_opacity_layer`]) and `id`, an inline element or a `::before`
/// or `::after` laid out in its lines, has no content on the lines after it.
///
/// The `::first-line` box wraps such content (CSS Pseudo 4 §2.1.1), and its
/// opacity group is a stacking context that holds it whatever its stack
/// level (CSS Color 4 §3.3), so the content does not paint below the
/// paragraph's blocks.
fn within_translucent_first_line(document: &Document, root: usize, id: usize) -> bool {
    if first_line_opacity_layer(document, root).is_none() {
        return false;
    }
    let Some(node) = document.get_node(root) else {
        return false; // cov:ignore: a translucent first line belongs to a paragraph node
    };
    let cache = &document.page_projection.later_line_membership;
    if let Some(members) = cache.borrow().get(&root) {
        return !members.contains(&id);
    }
    let members = collect_later_line_membership(document, node);
    let within = !members.contains(&id);
    cache.borrow_mut().insert(root, members);
    within
}

fn collect_later_line_membership(document: &Document, node: &crate::Node) -> HashSet<usize> {
    #[cfg(test)]
    tests::record_first_line_work(0);
    let mut members = HashSet::new();
    let mut include = |id| {
        let origin =
            crate::generated_content::generated_origin(id).map_or(id, |(element, _)| element);
        // An existing ancestor already has its full ancestor chain recorded.
        // Add the fragment ID only after closing its origin's ancestor chain.
        for ancestor in document.inclusive_ancestors(origin) {
            #[cfg(test)]
            tests::record_first_line_work(2);
            if !members.insert(ancestor) {
                break;
            }
        }
        members.insert(id);
    };
    for line in node.ifc_lines().into_iter().flatten().skip(1) {
        for fragment in line.fragments() {
            #[cfg(test)]
            tests::record_first_line_work(1);
            if let Some(id) = fragment_node(fragment) {
                include(id);
            }
        }
    }
    // Empty generated blocks still paint their boxes on their recorded line.
    for piece in node.ifc_generated_block_pieces() {
        #[cfg(test)]
        tests::record_first_line_work(3);
        if piece.line > 0 {
            include(piece.node);
        }
    }
    members
}

/// The stack level a pseudo-element group of [`pseudo_opacity_layer`]
/// composites at among the other groups of its paragraph: the `z-index` of a
/// relatively or sticky positioned pseudo-element, and 0 otherwise, the
/// level of the non-positioned groups (CSS 2.1 §9.9.1, Appendix E steps 8
/// and 9). A sticky one stays in its lines like a relative one (CSS
/// Positioned Layout 3 §3.4).
pub fn pseudo_stack_level(cascade: &CascadeResult, id: usize) -> i32 {
    crate::generated_content::computed_for_id(cascade, id).map_or(0, |cv| {
        match (&cv.position, cv.z_index) {
            (PositionValue::Relative | PositionValue::Sticky, ZIndexValue::Integer(level)) => level,
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

/// Whether a box that clips its overflow collects the boxes that wait for the
/// end of its layer (see [`defers_to_positioned_layer`]) in a layer of its
/// own, and hands that layer to the enclosing one inside its clip.
///
/// A plain block does (see [`merges_clipped_inline_content`]), and so does a
/// block-level flex or grid container: it is no stacking context, so its
/// items with a stack level paint in the enclosing stacking context, by level
/// among the boxes outside the container and in tree order within a level (CSS
/// 2.1 Appendix E steps 8 and 9; CSS Flexbox 1 §4.3; CSS Grid 1 §9.5). Its
/// clip still covers them.
pub fn merges_clipped_layer(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    merges_clipped_inline_content(document, cascade, node_id)
        || merges_clipped_container(document, cascade, node_id)
}

/// Whether `node_id` is a block-level flex or grid container that clips its
/// overflow and is no stacking context, placed in a box that leaves its
/// positioned boxes to the layer around it.
fn merges_clipped_container(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let Some(cv) = cascade.computed.get(node_id) else {
        return false;
    };
    cv.display.is_block_flex_or_grid_container()
        && clips_overflow(cv)
        && matches!(cv.position, PositionValue::Static)
        && matches!(cv.float, FloatValue::None)
        && !establishes_stacking_context(document, node_id, cv)
        && matches!(cv.column_count, ColumnCountValue::Auto)
        && matches!(cv.column_width, ComputedColumnWidth::Auto)
        && box_parent(document, cascade, node_id, BoxParentScope::Document)
            .and_then(|parent| cascade.computed.get(parent))
            .is_some_and(|parent| {
                // A container that is a flex or grid item with an integer
                // `z-index` forms a stacking context of its own.
                is_block_container(parent.display)
                    || (parent.display.is_flex_or_grid_container()
                        && !is_z_indexed_item(document, cascade, node_id, cv))
            })
}

/// [`defers_inline_content`] without its overflow condition.
fn defers_unless_clipped(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let Some(cv) = cascade.computed.get(node_id) else {
        return false;
    };
    let parent_display = box_parent(document, cascade, node_id, BoxParentScope::Document)
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
        && !establishes_stacking_context(document, node_id, cv)
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
/// group, a transform or a filter. A transform on a box that is not
/// transformable does not apply, so it forms no group (CSS Transforms 1 §3).
fn establishes_stacking_context(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    element_opacity_layer(document, id, cv).is_some()
        || !cv.filter.is_empty()
        || crate::layout::has_applied_transform(document, id, cv)
}

/// Whether a positioned box, a flex or grid item with an integer `z-index`,
/// or a box that forms a stacking context, waits for the end of the enclosing
/// layer.
///
/// Such boxes with a stack level of zero or more paint after the in-flow
/// blocks of their stacking context and after its inline content (CSS 2.1
/// Appendix E steps 8 and 9), so a block that follows one in the tree still
/// paints its background below it. The painters keep these boxes in tree
/// order at the end of the layer of the nearest ancestor that closes one.
///
/// Boxes laid out beside the lines of a paragraph keep their place among the
/// paragraph's content, apart from those [`leaves_paragraph_lines`] finds, as
/// do boxes with a negative stack level, which paint below the blocks of the
/// layer.
pub fn defers_to_positioned_layer(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    paragraphs: &LeavingParagraphs,
) -> bool {
    cascade.computed.get(node_id).is_some_and(|cv| {
        let leveled = takes_stack_level(document, cascade, node_id, cv);
        let negative = leveled && matches!(cv.z_index, ZIndexValue::Integer(level) if level < 0);
        document
            .get_node(node_id)
            .is_some_and(|node| node.kind() == NodeKind::Element)
            && element_generates_box(document, node_id, cv)
            && (leveled || establishes_stacking_context(document, node_id, cv))
            && !negative
            && (leaves_paragraph_lines(document, cascade, node_id, paragraphs)
                || box_parent(document, cascade, node_id, BoxParentScope::Document).is_some_and(
                    |parent| {
                        document
                            .get_node(parent)
                            .is_some_and(|node| !node.is_ifc_root())
                            && cascade.computed.get(parent).is_some_and(|parent| {
                                // A child of a table part sits in an anonymous cell;
                                // the children of flex and grid containers join the
                                // layer the same way.
                                is_block_container(parent.display)
                                    || (parent.display.is_flex_or_grid_container()
                                        || matches!(
                                            parent.display,
                                            DisplayValue::Table
                                                | DisplayValue::InlineTable
                                                | DisplayValue::TableRowGroup
                                                | DisplayValue::TableHeaderGroup
                                                | DisplayValue::TableFooterGroup
                                                | DisplayValue::TableRow
                                        ))
                            })
                    },
                ))
    })
}

/// Whether box `node_id`, laid out beside the lines of a paragraph, is
/// positioned with an integer `z-index` of zero or more outside every
/// inline box of the paragraph that forms a stacking context, beyond the
/// paragraph's translucent first line, if it has one.
///
/// It forms a stacking context in the stacking context of the paragraph,
/// where it paints by stack level among the positioned boxes outside the
/// paragraph, in tree order within a level (CSS 2.1 Appendix E steps 8 and
/// 9), so it waits for the end of the layer like them.
pub fn leaves_paragraph_lines(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    paragraphs: &LeavingParagraphs,
) -> bool {
    cascade.computed.get(node_id).is_some_and(|cv| {
        !cv.display.is_inline_flow()
            && !matches!(cv.position, PositionValue::Static)
            && matches!(cv.z_index, ZIndexValue::Integer(level) if level >= 0)
    }) && paragraph_of_line_box(document, cascade, node_id).is_some_and(|root| {
        document.parent_of(node_id).is_some_and(|parent| {
            parent == root || outside_inline_stacking_contexts(document, cascade, root, parent)
        }) && !within_translucent_first_line(document, root, node_id)
            && paragraphs.can_leave(document, cascade, root)
    })
}

/// What [`content_can_leave_paragraph`] tells of each paragraph a painter
/// asks about, found once per paragraph, as it scans the paragraph's lines.
#[derive(Debug, Default)]
pub struct LeavingParagraphs(std::cell::RefCell<std::collections::HashMap<usize, bool>>);

impl LeavingParagraphs {
    /// Whether the content of paragraph `root` can leave its lines (see
    /// [`content_can_leave_paragraph`]).
    pub fn can_leave(&self, document: &Document, cascade: &CascadeResult, root: usize) -> bool {
        *self
            .0
            .borrow_mut()
            .entry(root)
            .or_insert_with(|| content_can_leave_paragraph(document, cascade, root))
    }
}

/// Whether the positioned content of paragraph `root` that forms a stacking
/// context in the paragraph's stacking context can wait for the end of the
/// layer, apart from the paragraph's lines.
///
/// It cannot when a translucent first line wraps part of it, as the group
/// of the `::first-line` box holds that part (CSS Color 4 §3.3), or when a
/// `::before` or `::after` forms such a stacking context too, as the lines
/// order those groups by stack level among themselves, or when such an
/// inline element holds a block, which paints with the paragraph's blocks;
/// the content then paints with the lines, as all of it did before.
pub fn content_can_leave_paragraph(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> bool {
    if first_line_opacity_layer(document, root).is_some() {
        return false;
    }
    // A block-level `::before` or `::after` with no content has no line
    // fragment, only its piece.
    document.get_node(root).is_none_or(|node| {
        // An in-flow block inside such an inline element paints apart from
        // the element's group.
        !node.ifc_boxes().into_iter().any(|id| {
            cascade.computed.get(id).is_some_and(|cv| {
                !cv.display.is_inline_level()
                    && matches!(cv.position, PositionValue::Static)
                    && matches!(cv.float, FloatValue::None)
            }) && document
                .ancestors(id)
                .take_while(|&ancestor| ancestor != root)
                .any(|ancestor| inline_stacking_group(document, cascade, root, ancestor))
        }) && !node
            .ifc_lines()
            .into_iter()
            .flatten()
            .flat_map(|line| line.fragments())
            .filter_map(fragment_node)
            .chain(
                node.ifc_generated_block_pieces()
                    .iter()
                    .map(|piece| piece.node),
            )
            .any(|id| {
                crate::generated_content::generated_origin(id).is_some_and(|(element, _)| {
                    pseudo_stacking_group(document, cascade, root, id)
                        && outside_inline_stacking_contexts(document, cascade, root, element)
                })
            })
    })
}

/// The paragraph whose lines lay out box `node_id`: the paragraph root
/// reached through the inline elements around the box.
fn paragraph_of_line_box(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<usize> {
    document.ancestors(node_id).find_map(|ancestor| {
        if document
            .get_node(ancestor)
            .is_some_and(|node| node.is_ifc_root())
        {
            return Some(Some(ancestor));
        }
        let inline = document
            .get_node(ancestor)
            .is_some_and(|node| node.kind() == NodeKind::Element)
            && cascade.computed.get(ancestor).is_some_and(|cv| {
                cv.display.is_inline_flow() || cv.display == DisplayValue::Contents
            });
        (!inline).then_some(None)
    })?
}

/// Tree order and ancestor intervals shared by one immutable document walk.
///
/// Construction is constant-time. The first comparison in a source tree indexes
/// that whole tree, including detached trees and template-content fragments.
/// Other arena entries are neither visited nor allocated an index entry. Each
/// queried tree costs O(V + E) preparation and O(V) space; subsequent comparisons
/// use average O(1) hash lookups. The document borrow keeps the index valid.
/// Generated layout boxes are not source nodes and remain incomparable.
pub struct DocumentOrder<'a> {
    document: &'a Document,
    entries: rustc_hash::FxHashMap<usize, OrderEntry>,
    #[cfg(test)]
    index_work: usize,
}

#[derive(Clone, Copy)]
struct OrderEntry {
    component: usize,
    start: usize,
    end: usize,
}

impl<'a> DocumentOrder<'a> {
    /// Creates an empty index to share across all layers of one document walk.
    pub fn new(document: &'a Document) -> Self {
        Self {
            document,
            entries: rustc_hash::FxHashMap::default(),
            #[cfg(test)]
            index_work: 0,
        }
    }

    fn entry(&mut self, id: usize) -> OrderEntry {
        if let Some(&entry) = self.entries.get(&id) {
            return entry;
        }
        let mut root = id;
        while let Some(parent) = self.document.parent_of(root) {
            root = parent;
        }
        // Source parent links form a forest, even after layout creates its
        // separate anonymous boxes. Walk iteratively to support deep documents.
        let mut pending = vec![(root, false)];
        while let Some((node_id, leaving)) = pending.pop() {
            if leaving {
                let end = self.entries.len();
                self.entries.get_mut(&node_id).unwrap().end = end;
                continue;
            }
            #[cfg(test)]
            {
                self.index_work += 1;
            }
            let start = self.entries.len();
            self.entries.insert(
                node_id,
                OrderEntry {
                    component: root,
                    start,
                    end: start,
                },
            );
            pending.push((node_id, true));
            let node = self.document.get_node(node_id).unwrap();
            for &child in node.children.iter().rev() {
                #[cfg(test)]
                {
                    self.index_work += 1;
                }
                // Layout-only children must not become source-tree edges.
                if self.document.get_node(child).is_some()
                    && self.document.parent_of(child) == Some(node_id)
                {
                    pending.push((child, false));
                }
            }
        }
        self.entries[&id]
    }

    fn precedes(&mut self, a: usize, b: usize) -> bool {
        if a == b || self.document.get_node(a).is_none() || self.document.get_node(b).is_none() {
            return false;
        }
        let (a, b) = (self.entry(a), self.entry(b));
        a.component == b.component && a.start < b.start
    }

    fn is_ancestor_or_self(&mut self, ancestor: usize, node: usize) -> bool {
        let (ancestor, node) = (self.entry(ancestor), self.entry(node));
        ancestor.component == node.component
            && ancestor.start <= node.start
            && node.start < ancestor.end
    }
}

/// Adds `frame` to `frames`, the list a layer keeps in tree order of the
/// boxes that wait for its end (see [`PositionedFrame`]), at its place in
/// tree order.
///
/// The painters list most boxes as they reach them, in paint order, which
/// is tree order but for the order-modified document order of flex and grid
/// items (CSS Flexbox 1 §5.4), so they push those at the end. A paragraph
/// whose inline content waits for the end of the layer reaches its boxes
/// (see [`leaves_paragraph_lines`] and [`leaves_paragraph_content`]) after
/// the boxes that follow it, so its boxes go back past them with this, past
/// the clips around them as a whole.
///
/// Share `order` across the walk so tree comparisons do not revisit ancestors
/// or siblings. Reverse insertion still shifts the vector; closed clips still
/// scan back to their matching open frame.
pub fn insert_positioned_frame<F>(
    order: &mut DocumentOrder<'_>,
    frames: &mut Vec<F>,
    frame: F,
    kind: impl Fn(&F) -> PositionedFrame,
) {
    let PositionedFrame::Box(id) = kind(&frame) else {
        frames.push(frame);
        return;
    };
    let mut at = frames.len();
    while at > 0 {
        let first = match kind(&frames[at - 1]) {
            PositionedFrame::Box(other) => Some((at - 1, other)),
            PositionedFrame::Close => {
                // The clip around the frames back to the matching open.
                let mut depth = 0;
                (0..at).rev().find_map(|index| match kind(&frames[index]) {
                    PositionedFrame::Close => {
                        depth += 1;
                        None
                    }
                    PositionedFrame::Open(owner) => {
                        depth -= 1;
                        (depth == 0).then_some((index, owner))
                    }
                    PositionedFrame::Box(_) => None,
                })
            }
            PositionedFrame::Open(_) => None,
        };
        match first {
            Some((index, other))
                if order.precedes(id, other) && !order.is_ancestor_or_self(other, id) =>
            {
                at = index;
            }
            _ => break,
        }
    }
    frames.insert(at, frame);
}

/// Whether a box hands the positioned boxes it holds to the enclosing layer.
///
/// Positioned descendants belong to the nearest stacking context, where they
/// paint after its inline content (CSS 2.1 Appendix E step 8), whatever box
/// between holds them. A float, a flex or grid item, and an atomic inline
/// such as an inline-block paint as if they formed a stacking context, but
/// pass them on too (Appendix E, the step for inline-block and inline-table
/// elements, and CSS Flexbox 1 §4.3). So does a positioned box with
/// `z-index: auto`, which forms no stacking context (CSS 2.1 §9.9.1), when
/// the boxes with a non-zero stack level it holds can all join the enclosing
/// layer; the boxes inside it of level zero paint right after it either way. A box that clips or forms a
/// stacking context of its own keeps them, as does a box inside an inline
/// opacity group, which the painters paint as a whole. A clipping plain
/// block, flex container or grid container keeps them clipped but hands them
/// on in its own layer (see [`merges_clipped_layer`]).
pub fn hoists_positioned_content(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    cascade.computed.get(node_id).is_some_and(|cv| {
        (matches!(cv.position, PositionValue::Static)
            || held_levels(document, cascade, node_id) == HeldLevels::HandedOn)
            && !establishes_stacking_context(document, node_id, cv)
            && !is_z_indexed_item(document, cascade, node_id, cv)
            && !clips_overflow(cv)
            && !inside_inline_opacity_group(document, cascade, node_id)
    })
}

/// Whether element `node_id` sits inside an opacity group of its paragraph:
/// an inline element with an opacity below one, or a translucent
/// `::first-line` when the box is an atomic inline on the first line (see
/// [`first_line_opacity_layer`]). The painters paint such a group's content
/// as a whole; other inline stacking contexts form no group yet, so the
/// positioned boxes inside them still join the enclosing layer.
fn inside_inline_opacity_group(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let mut current = node_id;
    while let Some(parent) = box_parent(document, cascade, current, BoxParentScope::Document) {
        if document
            .get_node(parent)
            .is_some_and(|node| node.is_ifc_root())
        {
            return first_line_opacity_layer(document, parent).is_some()
                && atomic_on_first_line(document, parent, node_id);
        }
        match cascade.computed.get(parent) {
            Some(cv) if cv.display.is_inline_flow() => {
                if opacity_layer(cv).is_some() {
                    return true;
                }
                current = parent;
            }
            _ => break,
        }
    }
    false
}

/// One frame of the list a layer keeps of the boxes that wait for its end
/// (see [`defers_to_positioned_layer`]), as [`PositionedQueue`] reads
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionedFrame {
    /// The box of element `id`.
    Box(usize),
    /// Opens the clip of element `owner` around the frames up to the
    /// matching [`PositionedFrame::Close`].
    Open(usize),
    /// Closes the innermost open clip.
    Close,
}

/// The stack level a box that waits for the end of its layer paints at:
/// its integer `z-index` when it is positioned or a flex or grid item, else
/// zero (CSS 2.1 Appendix E steps 8 and 9).
fn positioned_layer_level(document: &Document, cascade: &CascadeResult, node_id: usize) -> i32 {
    cascade
        .computed
        .get(node_id)
        .map_or(0, |cv| match cv.z_index {
            ZIndexValue::Integer(level) if takes_stack_level(document, cascade, node_id, cv) => {
                level
            }
            _ => 0,
        })
}

/// The boxes that wait for the end of a layer, in the order its end paints
/// them.
///
/// The boxes of a stacking context with a stack level of zero or more paint
/// in stack level order, whatever their depth in the tree (CSS 2.1 Appendix
/// E step 9), and in tree order within a level (step 8). The layer lists
/// them in tree order, inside the clips of the boxes between that keep them
/// clipped. Each box keeps its clips: a clip opens again around a box moved
/// out of it, and boxes that stay next to each other share it.
///
/// The queue hands out one box at a time, so that the positioned boxes a box
/// hands on as it paints (see [`hoists_positioned_content`]) can join the
/// queue, at the box's place in the tree, before the next box is taken.
pub struct PositionedQueue<F> {
    /// The frames that open a clip, each with the clip it opens inside.
    clips: Vec<(Option<usize>, F)>,
    /// The boxes, each with the innermost clip open around it.
    boxes: Vec<(Option<usize>, F)>,
    /// The boxes still to paint, by index into `boxes`, smallest key first.
    order: BinaryHeap<Reverse<(PositionedKey, usize)>>,
    /// The clips open around the box taken last, outermost first.
    open: Vec<usize>,
    /// The box taken last, by key and index into `boxes`.
    last: Option<(PositionedKey, usize)>,
}

/// The order of a box in a [`PositionedQueue`]: the run of boxes it sorts
/// in, its stack level, and its place in the tree, as the index of each box
/// that handed it on among the boxes listed with that box, outermost first,
/// followed by its own. Compared as a whole, the places follow tree order:
/// the boxes a box hands on come after it and before the boxes after it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PositionedKey {
    run: usize,
    level: i32,
    place: Vec<usize>,
}

impl<F: Clone> PositionedQueue<F> {
    /// The queue of the boxes in `frames`, the list a layer kept in tree
    /// order.
    pub fn new(
        document: &Document,
        cascade: &CascadeResult,
        frames: Vec<F>,
        kind: impl Fn(&F) -> PositionedFrame,
    ) -> Self {
        let mut queue = Self {
            clips: Vec::new(),
            boxes: Vec::new(),
            order: BinaryHeap::new(),
            open: Vec::new(),
            last: None,
        };
        // A positioned box with `z-index: auto` is no stacking context. When
        // it keeps the positioned boxes inside it, as one that clips does,
        // the painters paint them with it. Moving it as a whole could put a
        // descendant of another level on the wrong side of the boxes it
        // passes, so such a box stays in place and the boxes on either side
        // of it are sorted apart.
        let mut run = 0;
        let mut position = 0;
        queue.push(frames, None, &kind, |id| {
            let at = position;
            position += 1;
            let key = |run, level| PositionedKey {
                run,
                level,
                place: vec![at],
            };
            if held_levels(document, cascade, id) != HeldLevels::None
                && !hoists_positioned_content(document, cascade, id)
            {
                run += 2;
                key(run - 1, 0)
            } else {
                key(run, positioned_layer_level(document, cascade, id))
            }
        });
        queue
    }

    /// Adds the boxes in `frames`, which the box taken last handed on, at
    /// its place in the tree and inside its clips.
    pub fn hand_on(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        frames: Vec<F>,
        kind: impl Fn(&F) -> PositionedFrame,
    ) {
        // Only a box the queue handed out has a place to hand boxes on at.
        let Some((last, taken)) = self.last.clone() else {
            return;
        };
        let clip = self.boxes[taken].0;
        let mut index = 0;
        self.push(frames, clip, &kind, |id| {
            let mut place = last.place.clone();
            place.push(index);
            index += 1;
            PositionedKey {
                run: last.run,
                level: positioned_layer_level(document, cascade, id),
                place,
            }
        });
    }

    /// The frames that paint the next box, with the clips that open and
    /// close around it, or the frames that close the clips still open once
    /// no box is left. `close` is the frame that closes a clip.
    pub fn next(&mut self, close: F) -> Vec<F> {
        let mut frames = Vec::new();
        let Some(Reverse((key, index))) = self.order.pop() else {
            frames.extend(self.open.drain(..).map(|_| close.clone()));
            return frames;
        };
        let mut path = Vec::new();
        let mut clip = self.boxes[index].0;
        while let Some(current) = clip {
            path.push(current);
            clip = self.clips[current].0;
        }
        path.reverse();
        let shared = self
            .open
            .iter()
            .zip(&path)
            .take_while(|(a, b)| a == b)
            .count();
        frames.extend(self.open.drain(shared..).map(|_| close.clone()));
        for &clip in &path[shared..] {
            frames.push(self.clips[clip].1.clone());
            self.open.push(clip);
        }
        frames.push(self.boxes[index].1.clone());
        self.last = Some((key, index));
        frames
    }

    /// Adds the boxes and clips in `frames`, a list in tree order, inside
    /// clip `parent`, keying each box by `key`.
    fn push(
        &mut self,
        frames: Vec<F>,
        parent: Option<usize>,
        kind: &impl Fn(&F) -> PositionedFrame,
        mut key: impl FnMut(usize) -> PositionedKey,
    ) {
        let mut open = parent;
        for frame in frames {
            match kind(&frame) {
                PositionedFrame::Box(id) => {
                    self.order.push(Reverse((key(id), self.boxes.len())));
                    self.boxes.push((open, frame));
                }
                PositionedFrame::Open(_) => {
                    self.clips.push((open, frame));
                    open = Some(self.clips.len() - 1);
                }
                PositionedFrame::Close => open = open.and_then(|clip| self.clips[clip].0),
            }
        }
    }
}

/// The boxes with a non-zero stack level in the enclosing stacking context
/// that a positioned box with `z-index: auto` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeldLevels {
    /// None: the box is no such box, or holds none.
    None,
    /// Only boxes with a positive `z-index` (positioned ones, and flex or
    /// grid items) that wait for the end of the enclosing layer and reach it
    /// through the boxes between: the painters can hand them on to sort with
    /// the boxes around the box.
    HandedOn,
    /// Some that the painters paint along with the box: a negative level, a
    /// box in a paragraph's lines, or one behind a box that keeps its
    /// positioned boxes.
    Kept,
}

/// What boxes with a non-zero stack level element `id` holds when it is a
/// positioned box with `z-index: auto` that forms no stacking context.
fn held_levels(document: &Document, cascade: &CascadeResult, id: usize) -> HeldLevels {
    // Fixed and sticky boxes form a stacking context whatever their
    // `z-index` (CSS Positioned Layout 3 §4).
    let forms_context = |id: usize, cv: &ComputedValues| {
        establishes_stacking_context(document, id, cv)
            || is_z_indexed_item(document, cascade, id, cv)
            || matches!(cv.position, PositionValue::Fixed | PositionValue::Sticky)
            || (!matches!(cv.position, PositionValue::Static)
                && matches!(cv.z_index, ZIndexValue::Integer(_)))
    };
    let auto = cascade
        .computed
        .get(id)
        .is_some_and(|cv| !matches!(cv.position, PositionValue::Static) && !forms_context(id, cv));
    let children = |id: usize, kept: bool| {
        document
            .get_node(id)
            .into_iter()
            .flat_map(move |node| node.children.iter().map(move |&child| (child, kept)))
    };
    // A box between keeps the positioned boxes inside it when it clips (a
    // plain block, a flex or a grid container hands them on inside its
    // clip), lays out columns, or sits in an inline opacity group.
    let keeps = |id: usize, cv: &ComputedValues| {
        (clips_overflow(cv) && !merges_clipped_layer(document, cascade, id))
            || document
                .get_node(id)
                .is_some_and(|node| node.is_multicol_container())
            || inside_inline_opacity_group(document, cascade, id)
    };
    let mut held = HeldLevels::None;
    let paragraphs = LeavingParagraphs::default();
    let mut stack: Vec<(usize, bool)> = if auto {
        let kept = cascade.computed.get(id).is_some_and(|cv| keeps(id, cv));
        children(id, kept).collect()
    } else {
        Vec::new()
    };
    while let Some((child, kept)) = stack.pop() {
        // Boxes that paint nothing, and the content of a nested stacking
        // context, take no part in the enclosing context's order. The
        // children of a `display: contents` element still generate boxes in
        // this context.
        let Some(cv) = cascade
            .computed
            .get(child)
            .filter(|cv| cv.display != DisplayValue::None)
        else {
            continue;
        };
        if !generates_box(cv) {
            stack.extend(children(child, kept));
            continue;
        }
        let level = positioned_layer_level(document, cascade, child);
        if level != 0 {
            // A non-zero level only comes from a positioned box or a z-indexed
            // flex or grid item, so a static child needs no guard of its own
            // here: `defers_to_positioned_layer` decides for both.
            if kept
                || level < 0
                || !defers_to_positioned_layer(document, cascade, child, &paragraphs)
            {
                return HeldLevels::Kept;
            }
            held = HeldLevels::HandedOn;
        }
        if !forms_context(child, cv) {
            stack.extend(children(child, kept || keeps(child, cv)));
        }
    }
    held
}

/// Whether a box takes a stack level from its `z-index`: a positioned box, or
/// a flex or grid item with an integer `z-index` (see [`is_z_indexed_item`]).
fn takes_stack_level(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    cv: &ComputedValues,
) -> bool {
    !matches!(cv.position, PositionValue::Static)
        || is_z_indexed_item(document, cascade, node_id, cv)
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
        && box_parent(document, cascade, node_id, BoxParentScope::Document)
            .and_then(|parent| cascade.computed.get(parent))
            .is_some_and(|parent| parent.display.is_flex_or_grid_container())
}

/// Whether box `id`, laid out in paragraph `root`, is an in-flow block
/// inside the paragraph's inline content (CSS 2.1 §9.2.1.1) that paints
/// before the paragraph's lines.
///
/// Its background is a block background of the layer (CSS 2.1 Appendix E
/// step 4), below the lines around it (step 7), and its own inline content
/// defers like any other block's. Blocks inside an inline element, or on
/// the first line, stay with the paragraph's content and its groups. When
/// [`holds_negative_stack_level`] says the paragraph holds content that
/// paints below the blocks, the painters paint the blocks with the
/// paragraph's content, after that content and before the rest.
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
            && !establishes_stacking_context(document, id, cv)
    }) && document
        .get_node(id)
        .is_some_and(|node| node.kind() == NodeKind::Element)
        && box_parent(document, cascade, id, BoxParentScope::Document) == Some(root)
        && !atomic_on_first_line(document, root, id)
}

/// Whether the inline content of paragraph `root` holds a box, an inline
/// element or a `::before` or `::after` positioned with a negative
/// `z-index`.
///
/// Such content paints below the in-flow blocks of the layer (CSS 2.1
/// Appendix E step 3), and the painters list it with the paragraph, so the
/// blocks inside the paragraph (see [`paints_before_paragraph_lines`]) are
/// listed there too, between that content and the rest. Only the
/// paragraph's own inline content is searched, not the blocks inside it,
/// the content of inline boxes that form stacking contexts, or content a
/// translucent first line wraps whole.
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
    // Content wrapped by a translucent `::first-line` paints in its group.
    let below = |key: usize| negative(key) && !within_translucent_first_line(document, root, key);
    if pseudos(root).into_iter().any(below) {
        return true;
    }
    let mut stack = Vec::new();
    if let Some(node) = document.get_node(root) {
        stack.extend(node.children.iter().copied());
    }
    while let Some(id) = stack.pop() {
        #[cfg(test)]
        crate::page_projection::work_tests::paragraph_node();
        let Some(node) = document
            .get_node(id)
            .filter(|node| node.kind() == NodeKind::Element)
        else {
            continue;
        };
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: elements in a laid-out paragraph have computed values
        };
        // A `display: contents` element has no box, so it neither paints at
        // a level of its own nor forms a stacking context (CSS Display 3
        // §2.5).
        let contents = cv.display == DisplayValue::Contents;
        if !contents && below(id) {
            return true;
        }
        // Content inside an inline box that forms a stacking context paints
        // within that box, not below the paragraph's blocks.
        let stacking = !contents
            && (establishes_stacking_context(document, id, cv)
                || (!matches!(cv.position, PositionValue::Static)
                    && matches!(cv.z_index, ZIndexValue::Integer(_))));
        if stacking {
            continue;
        }
        if pseudos(id).into_iter().any(below) {
            return true;
        }
        if cv.display.is_inline_flow() || cv.display == DisplayValue::Contents {
            stack.extend(node.children.iter().copied());
        }
    }
    false
}

/// Whether inline element `id` of paragraph `root` is positioned with a
/// negative `z-index` outside every inline box of the paragraph that forms
/// a stacking context, with content beyond the paragraph's translucent
/// first line, if it has one (see [`within_translucent_first_line`]).
///
/// Its boxes and text paint as a group below the in-flow blocks inside the
/// paragraph and the rest of its lines (CSS 2.1 Appendix E steps 3, 4 and
/// 7). Such an element is part of the content
/// [`holds_negative_stack_level`] finds; so is a `::before` or `::after`
/// that [`negative_pseudo_group`] finds.
pub fn negative_inline_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
) -> bool {
    document
        .get_node(id)
        .is_some_and(|node| node.kind() == NodeKind::Element)
        && cascade.computed.get(id).is_some_and(|cv| {
            cv.display.is_inline_flow()
                && !matches!(cv.position, PositionValue::Static)
                && matches!(cv.z_index, ZIndexValue::Integer(level) if level < 0)
        })
        && document
            .parent_of(id)
            .is_some_and(|parent| outside_inline_stacking_contexts(document, cascade, root, parent))
        && !within_translucent_first_line(document, root, id)
}

/// Whether inline element `id` of paragraph `root` is positioned with an
/// integer `z-index` and paints as a group: one with a stack level of zero
/// or more outside every inline box of the paragraph that forms a stacking
/// context, beyond the paragraph's translucent first line, if it has one,
/// or one with any stack level inside an inline element of the paragraph
/// that forms a stacking context and paints as a group (a translucent one,
/// one that [`negative_inline_group`] finds, or another one this finds).
///
/// It forms a stacking context of its own, whose boxes and text paint as a
/// group at its stack level among the content of the enclosing one: below
/// it with a negative level, after it otherwise (CSS 2.1 Appendix E steps
/// 3, 8 and 9). Outside every inline stacking context, that is after the
/// rest of the paragraph's inline content, as for the positioned boxes laid
/// out on its lines.
pub fn inline_stacking_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
) -> bool {
    let Some(level) = document
        .get_node(id)
        .filter(|node| node.kind() == NodeKind::Element)
        .and_then(|_| cascade.computed.get(id))
        .filter(|cv| cv.display.is_inline_flow() && !matches!(cv.position, PositionValue::Static))
        .and_then(|cv| match cv.z_index {
            ZIndexValue::Integer(level) => Some(level),
            _ => None,
        })
    else {
        return false;
    };
    let owner = document.ifc_source_owner(root);
    match document
        .inclusive_ancestors(id)
        .skip(1)
        .take_while(|&ancestor| ancestor != root && ancestor != owner)
        .find(|&ancestor| !forms_no_inline_stacking_context(document, cascade, ancestor))
    {
        None => level >= 0 && !within_translucent_first_line(document, root, id),
        Some(context) => paints_as_inline_group(document, cascade, root, context),
    }
}

/// Whether the group of inline element `id` of paragraph `root`, which
/// [`inline_stacking_group`] finds, forms a stacking context in the stacking
/// context of the paragraph: one with a stack level of zero or more outside
/// every inline box of the paragraph that forms a stacking context.
///
/// Such a group paints by stack level among the positioned boxes outside the
/// paragraph, in tree order within a level (CSS 2.1 Appendix E steps 8 and
/// 9), so the painters let it wait for the end of the layer like them, when
/// `can_leave`: what [`content_can_leave_paragraph`] tells of the paragraph,
/// which callers find once for all its groups.
pub fn leaves_paragraph_content(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
    can_leave: bool,
) -> bool {
    can_leave
        && inline_stacking_group(document, cascade, root, id)
        && document
            .parent_of(id)
            .is_some_and(|parent| outside_inline_stacking_contexts(document, cascade, root, parent))
}

/// Whether inline element `context` of paragraph `root`, which forms a
/// stacking context, paints as a group: a translucent one, one that
/// [`negative_inline_group`] finds, or one that [`inline_stacking_group`]
/// finds.
fn paints_as_inline_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    context: usize,
) -> bool {
    cascade
        .computed
        .get(context)
        .is_some_and(|cv| cv.display.is_inline_flow() && cv.opacity < 1.0)
        || negative_inline_group(document, cascade, root, context)
        || inline_stacking_group(document, cascade, root, context)
}

/// Whether the `::before` or `::after` with generated id `id`, laid out in
/// the lines of paragraph `root`, is relatively or sticky positioned with a
/// negative `z-index` outside every inline box of the paragraph that forms a
/// stacking context, with content beyond the paragraph's translucent first
/// line, if it has one.
///
/// Like the elements [`negative_inline_group`] finds, its boxes, images and
/// text paint as a group below the in-flow blocks inside the paragraph and
/// the rest of its lines.
pub fn negative_pseudo_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
) -> bool {
    crate::generated_content::generated_origin(id).is_some_and(|(element, pseudo)| {
        matches!(pseudo, PseudoElem::Before | PseudoElem::After)
            && pseudo_stack_level(cascade, id) < 0
            && outside_inline_stacking_contexts(document, cascade, root, element)
            && !within_translucent_first_line(document, root, id)
    })
}

/// Whether the `::before` or `::after` with generated id `id`, laid out in
/// the lines of paragraph `root`, generates a box that forms a stacking
/// context with a stack level of zero or more: a sticky one, which always
/// forms one (CSS Positioned Layout 3 §3.4), or a relatively positioned one
/// with an integer `z-index`. It paints as a group outside every inline box
/// of the paragraph that forms a stacking context, beyond the paragraph's
/// translucent first line, if it has one, or inside an inline element of
/// the paragraph that forms one and paints as a group (a translucent one,
/// one that [`negative_inline_group`] finds, or one that
/// [`inline_stacking_group`] finds).
///
/// Like the elements [`inline_stacking_group`] finds, its boxes, images and
/// text paint as a group after the rest of the inline content of the
/// enclosing stacking context (CSS 2.1 Appendix E steps 8 and 9).
pub fn pseudo_stacking_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
) -> bool {
    let Some((element, _)) = crate::generated_content::generated_origin(id)
        .filter(|&(_, pseudo)| matches!(pseudo, PseudoElem::Before | PseudoElem::After))
        .filter(|_| {
            crate::generated_content::computed_for_id(cascade, id).is_some_and(|cv| {
                // A `display: contents` pseudo-element generates no box to
                // form a stacking context with (CSS Display 3 §2.5).
                cv.display != DisplayValue::Contents
                    && match cv.position {
                        PositionValue::Sticky => true,
                        PositionValue::Relative => {
                            matches!(cv.z_index, ZIndexValue::Integer(_))
                        }
                        _ => false,
                    }
            })
        })
        .filter(|_| pseudo_stack_level(cascade, id) >= 0)
    else {
        return false;
    };
    let owner = document.ifc_source_owner(root);
    match document
        .inclusive_ancestors(element)
        .take_while(|&ancestor| ancestor != root && ancestor != owner)
        .find(|&ancestor| !forms_no_inline_stacking_context(document, cascade, ancestor))
    {
        None => !within_translucent_first_line(document, root, id),
        Some(context) => paints_as_inline_group(document, cascade, root, context),
    }
}

/// Whether `start` and the elements around it, up to paragraph `root`, are
/// inline elements of the paragraph that form no stacking context.
fn outside_inline_stacking_contexts(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    start: usize,
) -> bool {
    let owner = document.ifc_source_owner(root);
    document
        .inclusive_ancestors(start)
        .take_while(|&ancestor| ancestor != root && ancestor != owner)
        .all(|ancestor| forms_no_inline_stacking_context(document, cascade, ancestor))
}

/// Whether `id` is an inline element, or a `display: contents` one, that
/// forms no stacking context.
fn forms_no_inline_stacking_context(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
) -> bool {
    document
        .get_node(id)
        .is_some_and(|node| node.kind() == NodeKind::Element)
        && cascade.computed.get(id).is_some_and(|cv| {
            // A `display: contents` element has no box to form a
            // stacking context with (CSS Display 3 §2.5).
            cv.display == DisplayValue::Contents
                || (cv.display.is_inline_flow()
                    && !establishes_stacking_context(document, id, cv)
                    && (matches!(cv.position, PositionValue::Static)
                        || !matches!(cv.z_index, ZIndexValue::Integer(_))))
        })
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
        && !establishes_stacking_context(document, node_id, cv)
        && (!matches!(cv.float, FloatValue::None)
            || matches!(
                cv.display,
                DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
            ));
    document
        .get_node(node_id)
        .is_some_and(|node| node.kind() == NodeKind::Element)
        && element_generates_box(document, node_id, cv)
        && !below_inline_content
        && !defers_inline_content(document, cascade, node_id)
        && !merges_clipped_inline_content(document, cascade, node_id)
}

/// Whether a box with these values paints none of its own decorations: its
/// background, borders, shadows and outline (CSS Display 3 §4). Its content
/// and descendants decide their visibility for themselves.
///
/// `collapse` counts as `hidden`, as it does for text. Table layout does not
/// remove collapsed rows and columns, so a collapsed table part keeps its
/// space and, like a cell that inherits `collapse` from its row, paints
/// nothing.
pub fn hides_own_box(cv: &ComputedValues) -> bool {
    cv.visibility != Visibility::Visible
}

/// A table that is not visible does not paint its own box.
pub fn is_visibility_hidden_table(cv: &ComputedValues) -> bool {
    hides_own_box(cv) && matches!(cv.display, DisplayValue::Table | DisplayValue::InlineTable)
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
    let order_sensitive_container = parent_display.is_flex_or_grid_container();
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
        (PositionValue::Static, _) if computed.display.is_flex_container() => (2, 0),
        (PositionValue::Static, _) => (1, 0),
        (_, ZIndexValue::Auto) => (2, 0),
        (_, ZIndexValue::Integer(value)) if value < 0 => (0, value),
        (_, ZIndexValue::Integer(value)) => (2, value),
        // PositionValue and ZIndexValue are non-exhaustive.  New variants
        // retain the default/source-order bucket until stacking support grows.
        _ => (1, 0),
    }
}

/// The body element paint starts at: the same element layout starts at
/// (see [`crate::layout::find_body`]), so a `<body>` inside an inert subtree
/// such as `<template>` contents is never selected. Return `None` for
/// fragments (no `<body>`).
pub fn find_paint_root(doc: &Document) -> Option<usize> {
    crate::layout::find_body(doc)
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
