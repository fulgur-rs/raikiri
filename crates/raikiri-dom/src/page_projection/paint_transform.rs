//! How the built-in painter moves an element's box and subtree away from
//! their layout location: by relative offsets that layout leaves to paint,
//! and by `transform`.
//!
//! The paint order lists such a move as a transform around the element's
//! events, and the text runs carry the same transforms, composed.

use super::records::PageFragment;
use super::text_runs::{RunContext, box_paint_shift, column_clip_owner};
use crate::Document;
use crate::image_geometry::position_offset;
use raikiri_style::resolve::{ComputedLengthPercentage, ComputedTransformFunction};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::PaintTransform;
use std::collections::HashMap;

/// Where the border box of an element with a transform that does more than
/// translate is laid out.
#[derive(Debug, Clone, Copy)]
pub(super) struct TransformBox {
    x: f32,
    y: f32,
    /// `y` is relative to the page's content box rather than in shared flow
    /// coordinates without page cuts.
    repeat: bool,
}

/// The layout boxes of the elements whose transforms do more than translate.
#[derive(Debug, Default, Clone)]
pub(super) struct TransformBoxes {
    /// Boxes in the flow, from their placement on any page.
    flow: HashMap<usize, TransformBox>,
    /// Repeated boxes, such as table headers, which may sit at a different
    /// place on each page, by page index.
    repeats: HashMap<(u32, usize), TransformBox>,
}

impl TransformBoxes {
    pub(super) fn clear(&mut self) {
        self.flow.clear();
        self.repeats.clear();
    }

    fn get(&self, page: &PageFragment, id: usize) -> Option<&TransformBox> {
        self.repeats
            .get(&(page.page_index, id))
            .or_else(|| self.flow.get(&id))
    }
}

/// The layout boxes of the elements whose transforms do more than translate,
/// from their placements.
pub(super) fn transform_boxes(
    document: &Document,
    cascade: &CascadeResult,
    pages: &[PageFragment],
) -> TransformBoxes {
    let mut boxes = TransformBoxes::default();
    for page in pages {
        for item in &page.items {
            let Ok(id) = usize::try_from(item.node_id.0) else {
                continue; // cov:ignore: placement node ids are arena indices
            };
            if item.kind == super::records::PageFragmentKind::Text
                || cascade.computed.get(id).is_none_or(translation_only)
            {
                continue;
            }
            // A table's placement is its grid box; the painter places the
            // element's own box.
            let grid = document
                .get_node(id)
                .and_then(|node| node.table_grid_box)
                .map_or((0.0, 0.0), |rect| (rect.x, rect.y));
            let x = item.rect.x - grid.0;
            if item.is_repeat {
                boxes
                    .repeats
                    .entry((page.page_index, id))
                    .or_insert(TransformBox {
                        x,
                        y: item.box_y - grid.1,
                        repeat: true,
                    });
            } else {
                boxes.flow.entry(id).or_insert(TransformBox {
                    x,
                    y: item.box_y - grid.1 + page.content_origin_y,
                    repeat: false,
                });
            }
        }
    }
    boxes
}

/// Whether every function of `cv`'s transform list translates.
pub(super) fn translation_only(cv: &ComputedValues) -> bool {
    cv.transform.iter().all(|function| {
        matches!(
            function,
            ComputedTransformFunction::Translate(..)
                | ComputedTransformFunction::TranslateX(..)
                | ComputedTransformFunction::TranslateY(..)
        )
    })
}

/// A 2D affine matrix in double precision, as the painter computes it.
#[derive(Debug, Clone, Copy)]
struct Affine([f64; 6]);

impl Affine {
    fn translate(x: f64, y: f64) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, x, y])
    }

    fn then(self, inner: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [ia, ib, ic, id, ie, iff] = inner.0;
        Self([
            a * ia + c * ib,
            b * ia + d * ib,
            a * ic + c * id,
            b * ic + d * id,
            a * ie + c * iff + e,
            b * ie + d * iff + f,
        ])
    }

    fn is_invertible(self) -> bool {
        let [a, b, c, d, _, _] = self.0;
        self.0.iter().all(|value| value.is_finite()) && a * d - b * c != 0.0
    }
}

/// The product of `cv`'s transform functions, with percentages resolved
/// against a `width` by `height` box.
fn function_matrix(cv: &ComputedValues, width: f64, height: f64) -> Affine {
    let resolve = |value: ComputedLengthPercentage, basis: f64| match value {
        ComputedLengthPercentage::Px(px) => f64::from(px),
        ComputedLengthPercentage::Percent(percent) => basis * f64::from(percent) / 100.0,
    };
    let skew = |x: f32, y: f32| {
        Affine([
            1.0,
            f64::from(y).to_radians().tan(),
            f64::from(x).to_radians().tan(),
            1.0,
            0.0,
            0.0,
        ])
    };
    let mut matrix = Affine::translate(0.0, 0.0);
    for function in cv.transform.iter() {
        matrix = matrix.then(match *function {
            ComputedTransformFunction::Matrix(values) => Affine(values.map(f64::from)),
            ComputedTransformFunction::Translate(x, y) => {
                Affine::translate(resolve(x, width), resolve(y, height))
            }
            ComputedTransformFunction::TranslateX(x) => Affine::translate(resolve(x, width), 0.0),
            ComputedTransformFunction::TranslateY(y) => Affine::translate(0.0, resolve(y, height)),
            ComputedTransformFunction::Scale(x, y) => {
                Affine([f64::from(x), 0.0, 0.0, f64::from(y), 0.0, 0.0])
            }
            ComputedTransformFunction::ScaleX(x) => Affine([f64::from(x), 0.0, 0.0, 1.0, 0.0, 0.0]),
            ComputedTransformFunction::ScaleY(y) => Affine([1.0, 0.0, 0.0, f64::from(y), 0.0, 0.0]),
            ComputedTransformFunction::Rotate(angle) => {
                let (sin, cos) = f64::from(angle.0).to_radians().sin_cos();
                Affine([cos, sin, -sin, cos, 0.0, 0.0])
            }
            ComputedTransformFunction::Skew(x, y) => skew(x.0, y.0),
            ComputedTransformFunction::SkewX(x) => skew(x.0, 0.0),
            ComputedTransformFunction::SkewY(y) => skew(0.0, y.0),
        });
    }
    matrix
}

/// Whether `cv`'s transform maps its box to a degenerate shape, so that
/// nothing of the element is painted (CSS Transforms 1 §5).
pub(super) fn is_singular(cv: &ComputedValues) -> bool {
    // The linear part does not depend on the box, and a percentage
    // translation of a finite box is finite.
    !function_matrix(cv, 1.0, 1.0).is_invertible()
}

/// The transform the painter applies to element `id` and its subtree on
/// `page`, in the coordinates the element's parent paints its content in.
///
/// `offset` is the element's own relative offset and translation, which
/// [`box_paint_shift`] reports; `general` says whether its transform does
/// more than translate. `ancestors` is the sum of the offsets of its
/// ancestors. `None` when the element does not move.
pub(super) fn element_transform(
    document: &Document,
    cascade: &CascadeResult,
    page: &PageFragment,
    id: usize,
    offset: (f32, f32),
    general: bool,
    ancestors: (f32, f32),
) -> Option<PaintTransform> {
    let shift = PaintTransform::translate(offset.0, offset.1);
    if !general {
        return (offset != (0.0, 0.0)).then_some(shift);
    }
    let cv = cascade.computed.get(id)?;
    let node = document.get_node(id)?;
    let Some(placed) = document.page_projection.transform_boxes.get(page, id) else {
        return (offset != (0.0, 0.0)).then_some(shift); // cov:ignore: every laid-out transformed element has a placement
    };
    // spec: https://www.w3.org/TR/css-transforms-1/#transform-rendering
    // The painter turns the box about its transform origin, on the box it
    // paints: the layout box moved by every offset, in page coordinates and
    // snapped to whole pixels.
    let size = node.unrounded_layout.size;
    let x = page.content_box.x + placed.x + ancestors.0 + offset.0;
    let y = page.content_box.y + placed.y + ancestors.1 + offset.1
        - if placed.repeat {
            0.0
        } else {
            page.content_origin_y
        };
    let paint_x = f64::from(x).round();
    let paint_y = f64::from(y).round();
    let width = f64::from(x + size.width).round() - paint_x;
    let height = f64::from(y + size.height).round() - paint_y;
    let origin = (
        paint_x + position_offset(cv.transform_origin.horizontal, width),
        paint_y + position_offset(cv.transform_origin.vertical, height),
    );
    // The painter applies the matrix in page coordinates, after every
    // offset; the ancestors' offsets are already applied around this one.
    let matrix = Affine::translate(
        origin.0 - f64::from(ancestors.0),
        origin.1 - f64::from(ancestors.1),
    )
    .then(function_matrix(cv, width, height))
    .then(Affine::translate(
        f64::from(ancestors.0) - origin.0 + f64::from(offset.0),
        f64::from(ancestors.1) - origin.1 + f64::from(offset.1),
    ));
    let [a, b, c, d, e, f] = matrix.0.map(|value| value as f32);
    Some(PaintTransform::new(a, b, c, d, e, f))
}

/// What the paint order does with element `id` on `page`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Motion {
    /// The element is painted where layout put it.
    Still,
    /// The element and its subtree are painted through this transform.
    Moved(PaintTransform),
    /// A singular transform hides the element and its subtree.
    Hidden,
}

/// How the painter moves element `id` on `page`, as the paint order lists it.
///
/// Inside columns, the column clips are listed at the content they clip,
/// not at the element that moves; there the boxes keep their layout
/// location, and the runs carry the offsets instead (see
/// [`super::text_runs`]).
pub(super) fn element_motion(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    page: &PageFragment,
    id: usize,
    cv: &ComputedValues,
) -> Motion {
    if cv.transform.is_empty() && cv.position != raikiri_style::property::PositionValue::Relative {
        return Motion::Still;
    }
    let fragmented = context.fragmented.contains(&id);
    let Ok(shift) = box_paint_shift(document, cascade, context.body, id, cv, fragmented) else {
        return Motion::Still;
    };
    if shift.general && is_singular(cv) {
        return Motion::Hidden;
    }
    if (shift.offset == (0.0, 0.0) && !shift.general) || in_columns(document, cascade, context, id)
    {
        return Motion::Still;
    }
    let mut ancestors = (0.0, 0.0);
    if shift.general {
        let mut current = (Some(id) != context.body)
            .then(|| document.parent_of(id))
            .flatten();
        while let Some(ancestor) = current {
            if let Some(cv) = cascade.computed.get(ancestor)
                && let Ok(shift) = box_paint_shift(
                    document,
                    cascade,
                    context.body,
                    ancestor,
                    cv,
                    context.fragmented.contains(&ancestor),
                )
            {
                ancestors.0 += shift.offset.0;
                ancestors.1 += shift.offset.1;
            }
            if Some(ancestor) == context.body {
                break;
            }
            current = document.parent_of(ancestor);
        }
    }
    element_transform(
        document,
        cascade,
        page,
        id,
        shift.offset,
        shift.general,
        ancestors,
    )
    .map_or(Motion::Still, Motion::Moved)
}

/// Whether a box above element `id`, up to the body, clips it to a column.
pub(super) fn in_columns(
    document: &Document,
    cascade: &CascadeResult,
    context: &RunContext,
    id: usize,
) -> bool {
    if Some(id) == context.body {
        return false;
    }
    let mut current = document.parent_of(id);
    while let Some(ancestor) = current {
        if column_clip_owner(document, cascade, context, ancestor) {
            return true;
        }
        if Some(ancestor) == context.body {
            break;
        }
        current = document.parent_of(ancestor);
    }
    false
}
