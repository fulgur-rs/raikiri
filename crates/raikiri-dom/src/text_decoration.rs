//! Backend-independent propagation and used geometry of text decorations.

use crate::generated_content::{computed_for_id, generated_origin};
use crate::{Document, PositionedLine};
use raikiri_style::property::{
    CssColor, Direction, DisplayValue, FloatValue, PositionValue, TextDecorationColor,
    TextDecorationLine, TextDecorationStyle,
};
use raikiri_style::{
    CascadeResult, ComputedTextDecorationInset, ComputedTextUnderlineOffset, ComputedValues,
};
use raikiri_traits::NodeId;
use shodo::Fragment;
use shodo::geometry::BaselineKind;
use std::collections::HashMap;
use std::sync::Arc;

/// The shape of a text decoration pattern.
pub use raikiri_style::property::TextDecorationStyle as DecorationStyle;

/// Which line of a text decoration is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecorationKind {
    /// A line below the text.
    Underline,
    /// A line above the text.
    Overline,
    /// A line through the text, above its glyphs.
    LineThrough,
}

/// A used decoration segment in CSS px in page paint coordinates.
///
/// An ancestor's decoration retains its color, thickness and baseline at
/// the originating element. Segments of the same line share a pattern origin
/// and end, so font and color run boundaries cannot restart the pattern or
/// change a period chosen to bound drawing complexity.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DecorationLine {
    /// The line's position relative to the text.
    pub kind: DecorationKind,
    /// The line pattern.
    pub style: DecorationStyle,
    /// Used color, including alpha.
    pub color: CssColor,
    /// Left endpoint in page coordinates.
    pub x_start: f32,
    /// Right endpoint in page coordinates.
    pub x_end: f32,
    /// Left endpoint of the unsplit line, for pattern phase.
    pub pattern_origin_x: f32,
    /// Right endpoint of the unsplit line, for a shared pattern complexity budget.
    pub pattern_end_x: f32,
    /// Vertical center in page coordinates, positive downwards.
    pub y: f32,
    /// Used line thickness.
    pub thickness: f32,
    /// The element originating this line.
    pub origin: NodeId,
}

/// A decoration line carried from the element that originated it.
///
/// `text-decoration-line` is not an inherited property, but CSS Text
/// Decoration propagates a line from an element to its in-flow descendants.
/// The paint walker therefore carries these values separately from the
/// computed-value inheritance walk. The origin metrics are retained so a
/// descendant with a different font size cannot change the line's thickness
/// or vertical offsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationSpec {
    /// The element originating the decoration.
    pub origin: NodeId,
    pub line: TextDecorationLine,
    pub style: TextDecorationStyle,
    pub color: CssColor,
    pub origin_thickness: f64,
    pub origin_ascent: f64,
    pub origin_descent: f64,
    /// Cumulative vertical-align shift at the decorating box.
    ///
    /// Descendant shifts must not change the decoration's initial position.
    pub origin_shift_y: f32,
    /// Inline-start/end endpoint offsets from `text-decoration-inset`.
    pub inset_start: f64,
    pub inset_end: f64,
    /// Fixed offset for underlines originating at this element.
    pub underline_offset: f64,
    pub origin_rtl: bool,
}

/// Return whether an element is a boundary for decoration propagation.
///
/// CSS Text Decoration propagates through in-flow descendants, but not into
/// out-of-flow boxes, floats, or atomic inline-level boxes. The boundary box
/// may still originate its own decoration, which is added after the ancestor
/// context has been cleared.
fn is_decoration_propagation_boundary(cv: &ComputedValues) -> bool {
    let out_of_flow = matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        || !matches!(cv.float, FloatValue::None);
    let atomic_inline = matches!(
        cv.display,
        DisplayValue::InlineBlock
            | DisplayValue::InlineFlex
            | DisplayValue::InlineGrid
            | DisplayValue::InlineTable
    );
    out_of_flow || atomic_inline
}

/// Persistent paint-only context for propagated decorations.
///
/// Each originating element adds one linked node. Cloning the context for a
/// child frame only clones the outer `Arc`; ancestor specifications are never
/// copied into a new vector, even for deeply nested decorated elements.
#[derive(Clone, Debug, Default)]
pub struct DecorationContext(Option<Arc<DecorationLink>>);

#[derive(Debug)]
struct DecorationLink {
    spec: DecorationSpec,
    parent: DecorationContext,
}

impl DecorationContext {
    /// The specifications in paint order (ancestor to descendant).
    pub fn specs(&self) -> Vec<&DecorationSpec> {
        self.iter().collect()
    }

    fn push(&self, spec: DecorationSpec) -> Self {
        Self(Some(Arc::new(DecorationLink {
            spec,
            parent: self.clone(),
        })))
    }

    pub fn iter(&self) -> impl Iterator<Item = &DecorationSpec> {
        // The linked context is newest-first, while CSS paint order follows
        // the originating elements from ancestor to descendant. This small
        // per-text-node stack reverses traversal without copying contexts or
        // their specifications during the tree walk.
        let mut stack = Vec::new();
        let mut next = self.0.as_deref();
        while let Some(link) = next {
            stack.push(link);
            next = link.parent.0.as_deref();
        }
        DecorationContextIter { stack }
    }
}

struct DecorationContextIter<'a> {
    stack: Vec<&'a DecorationLink>,
}

impl<'a> Iterator for DecorationContextIter<'a> {
    type Item = &'a DecorationSpec;

    fn next(&mut self) -> Option<Self::Item> {
        self.stack.pop().map(|link| &link.spec)
    }
}

fn element_decoration(
    origin: NodeId,
    cv: &ComputedValues,
    origin_shift_y: f32,
) -> Option<DecorationSpec> {
    if !has_paintable_line(cv.text_decoration_line) {
        return None;
    }
    let color = match cv.text_decoration_color {
        TextDecorationColor::CurrentColor => cv.color,
        TextDecorationColor::Resolved(color) => color,
        // `TextDecorationColor` is non-exhaustive so a future value must not
        // make the paint path silently lose an otherwise valid decoration.
        // cov:ignore: `TextDecorationColor` has no future variant in this
        // pinned implementation; this arm is a non-exhaustive forward guard.
        _ => cv.color,
    };
    let (inset_start, inset_end) = match cv.text_decoration_inset {
        // `auto` remains distinct in computed style. The current per-text-node
        // paint segmentation already supplies the automatic boundary behavior,
        // so no additional endpoint offset is needed here.
        ComputedTextDecorationInset::Auto => (0.0, 0.0),
        ComputedTextDecorationInset::Lengths { start, end } => (start.px() as f64, end.px() as f64),
    };
    let underline_offset = match cv.text_underline_offset {
        ComputedTextUnderlineOffset::Auto => 0.0,
        ComputedTextUnderlineOffset::Length(value) => value.px() as f64,
        // A percentage is relative to 1em of the decorating element itself,
        // not of the ancestor that declared it.
        ComputedTextUnderlineOffset::Percent(percent) => {
            cv.font_size.px() as f64 * percent as f64 / 100.0
        }
        ComputedTextUnderlineOffset::Calc(value) => {
            value.px as f64 + cv.font_size.px() as f64 * value.percent as f64 / 100.0
        }
    };
    let underline_offset = if underline_offset.is_finite() {
        underline_offset
    } else {
        // cov:ignore: computed style values are sanitized before paint.
        0.0
    };
    let raw_font_size = cv.font_size.px() as f64;
    let origin_font_size = if raw_font_size.is_finite() {
        raw_font_size.max(1.0)
    } else {
        // cov:ignore: computed font sizes are sanitized before paint; retain a
        // finite fallback at this sink boundary for hostile/future inputs.
        1.0
    };
    Some(DecorationSpec {
        origin,
        line: cv.text_decoration_line,
        style: cv.text_decoration_style,
        color,
        origin_thickness: (origin_font_size / 16.0).max(1.0),
        // These normalized metrics keep the position tied to the decorating
        // element even when a descendant uses a different font size.
        origin_ascent: origin_font_size * 0.8,
        origin_descent: origin_font_size * 0.2,
        origin_shift_y,
        inset_start,
        inset_end,
        underline_offset,
        origin_rtl: matches!(cv.direction, Direction::Rtl),
    })
}

/// Build the decoration context for an element's children.
///
/// A boundary drops ancestor lines first, then retains a line originated by
/// the boundary element itself. This models the CSS rule that a child cannot
/// cancel an ancestor decoration while atomic/out-of-flow boxes do not receive
/// that ancestor decoration.
pub fn decorations_for_element(
    decorations: &DecorationContext,
    origin: NodeId,
    cv: &ComputedValues,
    origin_shift_y: f32,
) -> DecorationContext {
    // `display: contents` generates no box, so its own decoration has no
    // effect. It also must not block a decoration propagated through it.
    if matches!(cv.display, DisplayValue::Contents) {
        return decorations.clone();
    }
    let base = if is_decoration_propagation_boundary(cv) {
        DecorationContext::default()
    } else {
        decorations.clone()
    };
    match element_decoration(origin, cv, origin_shift_y) {
        Some(spec) => base.push(spec),
        None => base,
    }
}

fn has_paintable_line(line: TextDecorationLine) -> bool {
    line.underline || line.overline || line.line_through
}

#[derive(Clone, Copy, Debug)]
pub struct DecorationGeometry {
    /// Pattern origin before this segment is split.
    pub pattern_origin_x: Option<f64>,
    /// Pattern end before this segment is split.
    pub pattern_end_x: Option<f64>,
    pub x0: f64,
    pub x1: f64,
    pub abs_y: f64,
    pub line_top: f32,
    /// The line's baseline in page coordinates, when the layout engine
    /// supplies it; each decoration adds its element's `origin_shift_y`.
    /// `None` derives the baseline from the decorating element's metrics.
    pub baseline: Option<f64>,
}

pub fn decoration_span(x0: f64, x1: f64, decoration: &DecorationSpec) -> Option<(f64, f64)> {
    let (start, end) = if decoration.origin_rtl {
        (decoration.inset_end, decoration.inset_start)
    } else {
        (decoration.inset_start, decoration.inset_end)
    };
    let line_x0 = x0 + start;
    let line_x1 = x1 - end;
    if !start.is_finite()
        || !end.is_finite()
        || !line_x0.is_finite()
        || !line_x1.is_finite()
        || line_x1 <= line_x0
    {
        None
    } else {
        Some((line_x0, line_x1))
    }
}

/// Return the paint spans for one decoration segment.
///
/// A mixed-sign inset can be represented by two translated copies of the
/// originating segment. Keeping those copies separate preserves the endpoint
/// overlap produced by the CSS Text Decoration reference rendering. Equal
/// translations collapse to the ordinary single span.
pub fn decoration_spans(
    x0: f64,
    x1: f64,
    decoration: &DecorationSpec,
) -> Option<([(f64, f64); 2], usize)> {
    let span = decoration_span(x0, x1, decoration)?;
    if !decoration.origin_rtl
        && decoration.inset_start > 0.0
        && decoration.inset_end < 0.0
        && (decoration.inset_start + decoration.inset_end).abs() > f64::EPSILON
    {
        let first = (x0 + decoration.inset_start, x1 + decoration.inset_start);
        let second = (x0 - decoration.inset_end, x1 - decoration.inset_end);
        if first.0.is_finite()
            && first.1.is_finite()
            && second.0.is_finite()
            && second.1.is_finite()
            && first.1 > first.0
            && second.1 > second.0
        {
            return Some(([first, second], 2));
        }
    }
    Some(([span, (0.0, 0.0)], 1))
}

/// Resolve the segments of each line in CSS painting order.
pub fn resolve_decoration_lines(
    decorations: &[&DecorationSpec],
    geometry: DecorationGeometry,
) -> Vec<DecorationLine> {
    let mut lines = Vec::new();
    for kind in [
        DecorationKind::Underline,
        DecorationKind::Overline,
        DecorationKind::LineThrough,
    ] {
        for decoration in decorations {
            let enabled = match kind {
                DecorationKind::Underline => decoration.line.underline,
                DecorationKind::Overline => decoration.line.overline,
                DecorationKind::LineThrough => decoration.line.line_through,
            };
            if !enabled {
                continue;
            }
            let Some((spans, count)) = decoration_spans(geometry.x0, geometry.x1, decoration)
            else {
                continue;
            };
            let baseline = geometry.baseline.unwrap_or(
                geometry.abs_y + f64::from(geometry.line_top) + decoration.origin_ascent,
            ) + f64::from(decoration.origin_shift_y);
            let thickness = decoration.origin_thickness;
            let y = match kind {
                DecorationKind::Underline => {
                    baseline + decoration.origin_descent * 0.5 + decoration.underline_offset
                }
                DecorationKind::Overline => baseline - decoration.origin_ascent + thickness * 0.5,
                DecorationKind::LineThrough => baseline - decoration.origin_ascent * 0.35,
            };
            for &(x_start, x_end) in spans.iter().take(count) {
                let line = DecorationLine {
                    kind,
                    style: decoration.style,
                    color: decoration.color,
                    x_start: x_start as f32,
                    x_end: x_end as f32,
                    pattern_origin_x: geometry.pattern_origin_x.unwrap_or(x_start) as f32,
                    pattern_end_x: geometry.pattern_end_x.unwrap_or(x_end) as f32,
                    y: y as f32,
                    thickness: thickness as f32,
                    origin: decoration.origin,
                };
                if line.x_start.is_finite()
                    && line.x_end.is_finite()
                    && line.pattern_origin_x.is_finite()
                    && line.pattern_end_x.is_finite()
                    && line.pattern_end_x > line.pattern_origin_x
                    && line.y.is_finite()
                    && line.thickness.is_finite()
                    && line.x_end > line.x_start
                    && line.thickness > 0.0
                {
                    lines.push(line);
                }
            }
        }
    }
    lines
}

/// The propagated decoration context at an IFC root.
pub fn context_for_root(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> DecorationContext {
    let mut ancestors = Vec::new();
    let mut current = Some(root);
    while let Some(id) = current {
        ancestors.push(id);
        current = document.parent_of(id);
    }
    ancestors
        .into_iter()
        .rev()
        .fold(
            DecorationContext::default(),
            |context, id| match computed_for_id(cascade, id) {
                Some(cv) => decorations_for_element(&context, NodeId::new(id as u64), cv, 0.0),
                None => context,
            },
        )
}

/// Build one context per text owner and reuse it across that owner's font slices.
fn contexts_for_text_owners<K: Copy + Eq + std::hash::Hash>(
    owners: impl IntoIterator<Item = K>,
    mut context: impl FnMut(K) -> DecorationContext,
) -> Vec<DecorationContext> {
    let mut cache = HashMap::new();
    owners
        .into_iter()
        .map(|owner| cache.entry(owner).or_insert_with(|| context(owner)).clone())
        .collect()
}

/// The decoration context of a text node: the context after the ifc root,
/// folded through the elements between the root and the text. Each element's
/// decoration sits at that element's baseline, `shifts` below the line's.
fn context_for_text(
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    text_node: usize,
    base: &DecorationContext,
    shifts: &HashMap<usize, f32>,
) -> DecorationContext {
    // Anonymous flex/grid text roots already carry their complete ancestor
    // context. Starting at their parent would fold those ancestors twice.
    if root_id == text_node {
        return base.clone();
    }
    let mut chain = Vec::new();
    // The text of a pseudo-element is owned by the pseudo-element's own box,
    // which is decorated like an inline element child of its element.
    let mut current = match generated_origin(text_node) {
        Some((element, _)) => {
            chain.push(text_node);
            (element != root_id).then_some(element)
        }
        None => document.parent_of(text_node),
    };
    while let Some(id) = current {
        if id == root_id {
            break;
        }
        chain.push(id);
        current = document.parent_of(id);
    }
    chain.iter().rev().fold(base.clone(), |context, &id| {
        let Some(cv) = computed_for_id(cascade, id) else {
            return context;
        };
        decorations_for_element(
            &context,
            NodeId::new(id as u64),
            cv,
            shifts.get(&id).copied().unwrap_or(0.0),
        )
    })
}

/// How far each inline element's baseline lies below the line's baseline, on
/// this line. An element with no fragment on the line is not in the map.
///
/// A fragment's content area starts at the top of its font's ascent, so the
/// element's baseline is that top plus the ascent.
fn baseline_shifts(document: &Document, line: &shodo::Line) -> HashMap<usize, f32> {
    let line_baseline = line.baseline(BaselineKind::Alphabetic);
    let mut shifts = HashMap::new();
    for fragment in line.fragments() {
        let Fragment::InlineBox(inline_box) = fragment else {
            continue;
        };
        let Some(metrics) = document.ifc_font_metrics(inline_box.font, inline_box.font_size) else {
            continue;
        };
        let baseline = inline_box.content_rect.block_start + metrics.ascent;
        shifts.insert(inline_box.node.0 as usize, baseline - line_baseline);
    }
    shifts
}

/// Used decoration segments corresponding to each positioned run of a line.
pub fn positioned_line_decorations(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    base: &DecorationContext,
    line: &PositionedLine<'_>,
    origin: (f32, f32),
) -> Vec<Vec<DecorationLine>> {
    let shifts = baseline_shifts(document, line.line);
    let mut bounds: Vec<(f64, f64)> = line
        .runs
        .iter()
        .map(|placed| {
            placed.glyphs.iter().zip(placed.run.glyphs()).fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(left, right), (glyph, shaped)| {
                    (
                        left.min(f64::from(origin.0 + glyph.x)),
                        right.max(f64::from(origin.0 + glyph.x) + f64::from(shaped.advance)),
                    )
                },
            )
        })
        .collect();
    let content = f64::from(line.line.inline_size() + line.line.hang_start());
    if line.line.used_direction() == shodo::geometry::Direction::Rtl {
        let right = bounds.iter().map(|b| b.1).fold(f64::NEG_INFINITY, f64::max);
        for bound in &mut bounds {
            bound.0 = bound.0.max(right - content);
        }
    } else {
        let left = bounds.iter().map(|b| b.0).fold(f64::INFINITY, f64::min);
        for bound in &mut bounds {
            bound.1 = bound.1.min(left + content);
        }
    }
    let baseline = f64::from(origin.1)
        + f64::from(line.line.block_offset())
        + f64::from(line.line.baseline(BaselineKind::Alphabetic));
    let contexts = contexts_for_text_owners(
        line.runs.iter().map(|run| (run.style_owner, run.owner)),
        |(style_owner, owner)| {
            let context = context_for_text(document, cascade, root, owner, base, &shifts);
            if style_owner == owner {
                return context;
            }
            let Some(root_node) = document.get_node(root) else {
                return context; // cov:ignore: PositionedLines validated this root before producing any styled run.
            };
            let mut chain = Vec::new();
            let mut current = Some(style_owner);
            while let Some(id) = current {
                chain.push(id);
                current = root_node.ifc_typographic_parent(id);
            }
            chain.iter().rev().fold(context, |context, &id| {
                let Some(style) = root_node.ifc_typographic_style_for_owner(id, owner) else {
                    return context; // cov:ignore: style owner and parent IDs come from this root's retained letter styles.
                };
                decorations_for_element(
                    &context,
                    NodeId::new(id as u64),
                    style,
                    shifts.get(&id).copied().unwrap_or(0.0),
                )
            })
        },
    );
    let geometries: Vec<_> = line
        .runs
        .iter()
        .zip(bounds)
        .map(|(run, (x0, x1))| DecorationGeometry {
            x0: x0 + f64::from(run.offset.0),
            x1: x1 + f64::from(run.offset.0),
            abs_y: baseline,
            line_top: 0.0,
            baseline: Some(baseline + f64::from(run.offset.1)),
            pattern_origin_x: None,
            pattern_end_x: None,
        })
        .collect();
    // Insets belong to the decorating line, before font/color run slicing.
    let mut extents: HashMap<NodeId, (f64, f64)> = HashMap::new();
    for (context, geometry) in contexts.iter().zip(&geometries) {
        if !geometry.x0.is_finite() || !geometry.x1.is_finite() || geometry.x1 <= geometry.x0 {
            continue;
        }
        for spec in context.iter() {
            extents
                .entry(spec.origin)
                .and_modify(|range| {
                    range.0 = range.0.min(geometry.x0);
                    range.1 = range.1.max(geometry.x1);
                })
                .or_insert((geometry.x0, geometry.x1));
        }
    }
    contexts
        .iter()
        .zip(geometries)
        .map(|(context, geometry)| {
            let mut lines = Vec::new();
            for spec in context.iter() {
                let Some(&(left, right)) = extents.get(&spec.origin) else {
                    continue;
                };
                let Some((spans, count)) = decoration_spans(left, right, spec) else {
                    continue;
                };
                let without_insets = DecorationSpec {
                    inset_start: 0.0,
                    inset_end: 0.0,
                    ..*spec
                };
                for &(start, end) in spans.iter().take(count) {
                    let x0 = if geometry.x0 == left {
                        start
                    } else {
                        geometry.x0.max(start)
                    };
                    let x1 = if geometry.x1 == right {
                        end
                    } else {
                        geometry.x1.min(end)
                    };
                    lines.extend(resolve_decoration_lines(
                        &[&without_insets],
                        DecorationGeometry {
                            x0,
                            x1,
                            pattern_origin_x: Some(start),
                            pattern_end_x: Some(end),
                            ..geometry
                        },
                    ));
                }
            }
            lines.sort_by_key(|line| match line.kind {
                DecorationKind::Underline => 0,
                DecorationKind::Overline => 1,
                DecorationKind::LineThrough => 2,
            });
            lines
        })
        .collect()
}

#[cfg(test)]
mod tests;
