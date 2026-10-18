//! The frames of the stack that the paint walk keeps instead of recursing,
//! and the layers of deferred content that wait for the end of a block.

use crate::text;
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use raikiri_dom::paint_rules::{self, LeavingParagraphs};
use raikiri_dom::{Document, FragmentRect};
use raikiri_style::CascadeResult;

#[derive(Clone)]
pub(super) enum PaintFrame<'a> {
    Visit(VisitFrame),
    AnonymousParagraph(AnonymousParagraphFrame<'a>),
    RestoreTransform(Affine),
    Lines(LinesFrame),
    /// Start recording the paint of a box laid out beside the held lines
    /// of its paragraph. An `early` box paints before the lines, which
    /// place it once they are drawn.
    BeginBox {
        root: usize,
        node_id: usize,
        early: bool,
    },
    /// Place the recorded paint of a box in the held lines of `root`.
    EndBox {
        root: usize,
        node_id: usize,
        early: bool,
    },
    /// Trace the text of paragraph `root` positioned below its blocks,
    /// which its held lines paint below the blocks placed in them.
    TraceBelowBlocks {
        root: usize,
    },
    /// Composite the held lines of `root` and the boxes placed in them.
    FinishLines {
        root: usize,
    },
    /// The group of a positioned inline element that forms a stacking
    /// context in its paragraph's stacking context (see
    /// [`raikiri_dom::paint_rules::leaves_paragraph_content`]), which
    /// waits for the end of the layer like the positioned boxes. Its
    /// lines leave its paint in `left_groups`.
    InlineGroup {
        root: usize,
        node_id: usize,
    },
    /// Close a layer of deferred inline content: paint what its blocks
    /// deferred, then drop the layer.
    EndDeferral,
    /// Reopen the overflow clip of a block around the content its layer
    /// handed to the enclosing one (see
    /// [`raikiri_dom::paint_rules::merges_clipped_inline_content`]), in
    /// the transform the block was painted in.
    PushClip {
        node_id: usize,
        clip: raikiri_traits::PaintClip,
        transform: Affine,
    },
    PopClip,
    /// Open the opacity group of a translucent inline element around a
    /// box laid out beside its paragraph's lines.
    PushOpacity {
        node_id: usize,
        alpha: f32,
    },
    /// Close an element opacity group after its complete subtree.
    PopOpacity,
    /// Paint an originating element's `::after` pseudo after its real
    /// children, while still inside any overflow clip pushed for the
    /// originating element.
    PaintAfter {
        node_id: usize,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        before_advance: f32,
        content_advance: f32,
    },
}

#[derive(Clone)]
pub(super) struct VisitFrame {
    pub(super) node_id: usize,
    pub(super) parent_abs_x: f32,
    pub(super) parent_abs_y: f32,
    pub(super) parent_font_size: f32,
    pub(super) shift_y: f32,
    pub(super) transform_x: f32,
    pub(super) transform_y: f32,
    /// How far the relative offsets that paint applies to the
    /// ancestors move `parent_abs_y` from where layout placed it.
    pub(super) paint_offset_y: f32,
    pub(super) fragment_clip_height: Option<f32>,
    /// On a `display: contents` child of a multicolumn container, the
    /// column height that container clips its blocks to. The element
    /// has no box, so its children are the ones the columns hold.
    pub(super) column_clip_height: Option<f32>,
    pub(super) fragment_id: Option<usize>,
    pub(super) fragment_origin_x: f32,
    pub(super) fragment_origin_y: f32,
    pub(super) fragmentainer: Option<usize>,
    pub(super) fragment_rect: Option<FragmentRect>,
    pub(super) fragment_clip: Option<FragmentRect>,
    pub(super) is_fragment_visit: bool,
    pub(super) inside_fixed: bool,
    pub(super) inside_fixed_containing_block: bool,
    pub(super) decorations: text::DecorationContext,
}

#[derive(Clone)]
pub(super) struct AnonymousParagraphFrame<'a> {
    pub(super) key: usize,
    pub(super) cell: &'a raikiri_dom::Node,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) shift_y: f32,
    pub(super) transform_x: f32,
    pub(super) transform_y: f32,
    /// How far the relative offsets that paint applies move `y`
    /// from where layout placed it.
    pub(super) paint_offset_y: f32,
    pub(super) fragmentainer: Option<usize>,
    pub(super) inside_fixed: bool,
    pub(super) decorations: text::DecorationContext,
    /// The page area that `fixed` backgrounds of the paragraph's
    /// inline boxes are positioned in.
    pub(super) fixed_viewport: Option<Rect>,
    /// Whether the paragraph's lines wait for the end of the layer,
    /// as those of the explicit cells beside it do.
    pub(super) defer: bool,
}

/// The lines of one paragraph, drawn where this frame pops: in place
/// for most paragraphs, or at the end of the enclosing layer for a
/// block whose inline content is deferred (see
/// [`raikiri_dom::paint_rules::defers_inline_content`]).
#[derive(Clone)]
pub(super) struct LinesFrame {
    pub(super) node_id: usize,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) shift_y: f32,
    /// How far the translations around the paragraph move its lines
    /// along y.
    pub(super) translation_y: f32,
    pub(super) decorations: text::DecorationContext,
    pub(super) fragmentainer: Option<usize>,
    pub(super) inside_fixed: bool,
    pub(super) paint_transform: Affine,
    /// The page area that `fixed` backgrounds of the paragraph's
    /// inline boxes are positioned in.
    pub(super) fixed_viewport: Option<Rect>,
    /// Keep the paragraph's groups open until
    /// [`PaintFrame::FinishLines`] when it has any, so the boxes laid
    /// out beside the lines join them.
    pub(super) hold: bool,
    /// Whether a box beside the lines is inside a translucent inline
    /// element, so the groups are kept open even if no text of the
    /// element allocated one.
    pub(super) boxes_in_groups: bool,
}

/// How an open layer ends once its subtree is done.
#[derive(Clone, Copy)]
pub(super) enum LayerClose {
    /// Paint what the layer collected.
    Paint,
    /// Hand what the layer collected to the enclosing layer, inside the
    /// overflow clip of the block that opened it.
    Merge {
        node_id: usize,
        clip: raikiri_traits::PaintClip,
        transform: Affine,
    },
    /// Paint the deferred inline content, and hand the positioned boxes
    /// to the enclosing layer.
    Hoist,
}

/// An open layer: the inline content its plain blocks defer and the
/// positioned boxes that wait for its end, each in tree order, whether
/// blocks inside it may defer (the content of a block in columns stays
/// with its column clip), and how it ends.
pub(super) struct DeferralLayer<'a> {
    pub(super) pending: Vec<PaintFrame<'a>>,
    pub(super) positioned: Vec<PaintFrame<'a>>,
    pub(super) allowed: bool,
    pub(super) close: LayerClose,
    /// Whether the layer's end is painting its positioned boxes: the
    /// boxes it pops then paint where they pop rather than waiting again.
    pub(super) flushing: bool,
    /// The positioned boxes still to paint once the layer's end is
    /// painting them.
    pub(super) queue: Option<raikiri_dom::paint_rules::PositionedQueue<PaintFrame<'a>>>,
    /// Whether the standalone `::after` of the block that opened the
    /// layer waits right below the layer's end frame. It follows the
    /// block's content wherever that paints.
    pub(super) after: bool,
}

impl<'a> DeferralLayer<'a> {
    pub(super) fn new(allowed: bool, close: LayerClose) -> Self {
        Self {
            pending: Vec::new(),
            positioned: Vec::new(),
            allowed,
            close,
            flushing: false,
            queue: None,
            after: false,
        }
    }
}

/// How the queue of a layer's positioned boxes reads its frames.
pub(super) fn positioned_frame(
    frame: &PaintFrame<'_>,
) -> raikiri_dom::paint_rules::PositionedFrame {
    match frame {
        PaintFrame::Visit(VisitFrame { node_id, .. }) | PaintFrame::InlineGroup { node_id, .. } => {
            raikiri_dom::paint_rules::PositionedFrame::Box(*node_id)
        }
        PaintFrame::PushClip { node_id, .. } => {
            raikiri_dom::paint_rules::PositionedFrame::Open(*node_id)
        }
        _ => raikiri_dom::paint_rules::PositionedFrame::Close,
    }
}

/// Whether the lines drawn now leave the groups that form stacking
/// contexts in the paragraph's stacking context to the end of the layer,
/// where the positioned boxes paint.
pub(super) fn leaves_groups(deferral_scopes: &[DeferralLayer<'_>]) -> bool {
    deferral_scopes
        .last()
        .is_some_and(|layer| layer.allowed && !layer.flushing)
}

/// Composites held lines into `scene` like
/// [`crate::ifc_text::HeldLines::finish`], apart from the groups they leave to
/// the end of the layer, which it returns.
pub(super) fn finish_lines_leaving_groups(
    scene: &mut impl PaintScene,
    lines: crate::ifc_text::HeldLines<'_>,
) -> Vec<(usize, anyrender::Scene)> {
    let (below, blocks, above, left) = lines.finish_leaving_groups(true);
    scene.append_scene(below, Affine::IDENTITY);
    for block in blocks {
        scene.append_scene(block, Affine::IDENTITY);
    }
    scene.append_scene(above, Affine::IDENTITY);
    left
}

/// Lets the groups a paragraph's lines left wait for the end of the
/// layer, inside `clip`, the clip of the lines, if any.
pub(super) fn leave_groups<'a>(
    document: &Document,
    deferral_scopes: &mut [DeferralLayer<'a>],
    left_groups: &mut std::collections::HashMap<usize, anyrender::Scene>,
    root: usize,
    left: Vec<(usize, anyrender::Scene)>,
    clip: Option<(Affine, Rect)>,
) {
    for (node_id, content) in left {
        let entry = left_groups.entry(node_id);
        let waiting = matches!(entry, std::collections::hash_map::Entry::Occupied(_));
        let paint = entry.or_default();
        if let Some((transform, clip)) = clip {
            paint.push_clip_layer(transform, &clip);
            paint.append_scene(content, Affine::IDENTITY);
            paint.pop_layer();
        } else {
            paint.append_scene(content, Affine::IDENTITY);
        }
        if !waiting && let Some(layer) = deferral_scopes.last_mut() {
            raikiri_dom::paint_rules::insert_positioned_frame(
                document,
                &mut layer.positioned,
                PaintFrame::InlineGroup { root, node_id },
                positioned_frame,
            );
        }
    }
}

/// Set `frame` aside when it waits for the end of its layer or for the deferred
/// inline content before it, and give it back when it is painted now.
pub(super) fn set_aside<'a>(
    mut frame: PaintFrame<'a>,
    document: &Document,
    cascade: &CascadeResult,
    paragraphs: &LeavingParagraphs,
    deferral_scopes: &mut [DeferralLayer<'a>],
    stack: &mut Vec<PaintFrame<'a>>,
) -> Option<PaintFrame<'a>> {
    // A positioned box waits for the end of its layer, after the
    // backgrounds of the blocks that follow it.
    if let PaintFrame::Visit(VisitFrame { node_id, .. }) = &frame
        && let Some(layer) = deferral_scopes.last_mut()
        && layer.allowed
        && !layer.flushing
        && paint_rules::defers_to_positioned_layer(document, cascade, *node_id, paragraphs)
    {
        if paint_rules::leaves_paragraph_lines(document, cascade, *node_id, paragraphs) {
            paint_rules::insert_positioned_frame(
                document,
                &mut layer.positioned,
                frame,
                positioned_frame,
            );
        } else {
            layer.positioned.push(frame);
        }
        return None;
    }
    // The lines of an anonymous cell in a table part that defers wait
    // for the end of the layer, in tree order with the other cells'.
    if let PaintFrame::AnonymousParagraph(AnonymousParagraphFrame { defer, .. }) = &mut frame
        && std::mem::take(defer)
    {
        if let Some(layer) = deferral_scopes.last_mut() {
            layer.pending.push(frame);
        }
        return None;
    }
    // A box that does not defer, such as a flex container, paints after
    // the inline content deferred before it.
    if let PaintFrame::Visit(VisitFrame { node_id, .. }) = &frame
        && deferral_scopes
            .last()
            .is_some_and(|layer| !layer.pending.is_empty())
        && paint_rules::closes_deferred_inline_content(document, cascade, *node_id)
    {
        let pending = deferral_scopes
            .last_mut()
            .map(|layer| std::mem::take(&mut layer.pending))
            .unwrap_or_default();
        stack.push(frame);
        stack.extend(pending.into_iter().rev());
        return None;
    }
    Some(frame)
}
