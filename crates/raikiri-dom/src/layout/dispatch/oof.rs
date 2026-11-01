//! Public Taffy positioning at the immediate-container geometry boundary.
//!
//! The document's CSS containing-block pass owns viewport and ancestor
//! positioning. Resolve immediate-container candidates here so all existing
//! consumers keep locations relative to their layout parent.

use super::grid_oof;
use crate::document::OofCoordinate;
use crate::layout::context::{LayoutSession, RootOofCapture};
use crate::layout::protocol::{
    AvailableSpace, AxisStaticEdge, AxisStaticPosition, LayoutOutput, NodeId, OofCandidate,
    OofCandidates, OofLayoutResult, OofPosition, OofPositioningArea, StaticAlignSafety,
};
use crate::layout::style_views::ReadRole;
use crate::layout::taffy_adapter::algorithms::{
    claims_position, compute_oof_layout_for_area, compute_root_layout,
};
use crate::layout::taffy_adapter::{GridInfo, OofBatch, OofSpan};
use crate::node::FormattingContext;
use crate::node::arena::BoxKey;
use raikiri_layout_geometry::{Point, Size};
use raikiri_layout_style::values::{
    AlignItems, AlignmentSafety, BaselineSet, ContainingBlockClaims, Direction, ExpandedDimension,
};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};

impl LayoutSession<'_> {
    /// Forget the hoisted children recorded for `node` by an earlier layout.
    pub(crate) fn clear_hoisted_children(&mut self, node: NodeId) {
        let owner = usize::from(node);
        if let Some(previous) = self.context.taffy_hoisted_children.remove(&owner) {
            for child in previous.into_iter().map(usize::from) {
                self.context.safe_static_positions.remove(&child);
                if self
                    .oof_coordinates
                    .get(&child)
                    .is_some_and(|coordinate| coordinate.owner == owner)
                {
                    self.oof_coordinates.remove(&child);
                }
            }
        }
    }

    /// Record `children` as hoisted out of `node` to the box that is their
    /// containing block.
    pub(crate) fn add_hoisted_children<I>(&mut self, node: NodeId, children: I)
    where
        I: Iterator<Item = NodeId> + Clone,
    {
        if children.clone().next().is_some() {
            let owner = usize::from(node);
            self.context
                .taffy_hoisted_children
                .entry(owner)
                .or_default()
                .extend(children.clone());
            for child in children.clone().map(usize::from) {
                let static_owner = self.taffy_parent_of(child).unwrap_or(owner);
                let (follow_static_x, follow_static_y) = {
                    let inset = self.physical_style(child).inset();
                    (
                        inset.left.is_auto() && inset.right.is_auto(),
                        inset.top.is_auto() && inset.bottom.is_auto(),
                    )
                };
                self.oof_coordinates.insert(
                    child,
                    OofCoordinate {
                        owner,
                        static_owner,
                        follow_static_x,
                        follow_static_y,
                        initial_owner_origin: None,
                        initial_static_origin: None,
                    },
                );
            }
            super::align_hoisted_grid_abspos_children(self, node, children);
        }
    }

    /// Whether the container that holds `node` in its own layout is on vertical
    /// axes.
    pub(crate) fn container_is_vertical(&self, node: usize) -> bool {
        self.taffy_parent_of(node)
            .is_some_and(|parent| super::block_right_to_left(self, parent).is_some())
    }

    /// Whether the library reads the alignment of the out-of-flow box `node`
    /// along the physical vertical axis from `justify-self` and the one along
    /// the horizontal axis from `align-self`, rather than the other way round.
    ///
    /// The last baseline of the container that holds the box places it on the
    /// container's axes, so the swap applies only where the container is on
    /// vertical axes as well as the containing block.
    pub(crate) fn oof_alignment_swapped(&self, node: usize) -> bool {
        self.context.oof_axes_vertical && self.container_is_vertical(node)
    }

    /// The tracks and areas a grid container reported, which the library
    /// positions its out-of-flow children in.
    pub(crate) fn oof_grid_info(&self, node: NodeId) -> &GridInfo {
        let node = usize::from(node);
        // The library resolves a grid area on the physical axes of
        // `horizontal-tb`. The tracks of a vertical grid are on its layout
        // axes, so `grid_oof` resolves its grid areas instead.
        if grid_oof::right_to_left(self, node).is_some() {
            return &GridInfo::NONE;
        }
        self.context
            .grid_layout_info
            .get(&node)
            .unwrap_or(&GridInfo::NONE)
    }
}

pub(super) fn layout_local_oof(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    output: &mut LayoutOutput,
) {
    if tree.nodes[usize::from(node)]
        .flags
        .contains(crate::node::NodeFlags::FINAL_VERTICAL_TABLE_CELL)
    {
        return;
    }
    layout_local_oof_now(tree, node, output);
}

/// Complete a deferred cell's positioning after its flow alignment.
pub(super) fn layout_local_oof_now(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    output: &mut LayoutOutput,
) {
    let Some(area) = output.oof_positioning_area else {
        return;
    };
    let candidates = output.oof_candidates.take();
    let grid = grid_oof::GridAxes::of(tree, usize::from(node), output.size.width);
    let result = layout_candidates_of(tree, node, candidates.iter(), area, grid);
    output.oof_candidates = result.unclaimed;
    output.scrollable_overflow_rect = output
        .scrollable_overflow_rect
        .union(result.scrollable_overflow_rect);
}

/// Position the `candidates` that `node` contains against `area`, its
/// positioning area, and return what it does not claim. The direction and
/// the claims are those of `node`'s own style.
pub(crate) fn layout_container_candidates(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    candidates: &OofCandidates,
    area: OofPositioningArea,
) -> OofLayoutResult {
    layout_candidates_of(tree, node, candidates.iter(), area, None)
}

/// [`layout_container_candidates`] for `node` that is a grid container laid out
/// on the axes `grid` describes, when it is.
fn layout_candidates_of(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    candidates: impl Iterator<Item = OofCandidate>,
    area: OofPositioningArea,
    grid: Option<grid_oof::GridAxes>,
) -> OofLayoutResult {
    let batch = oof_batch(tree, candidates);
    let style = tree.query_style(usize::from(node), ReadRole::CoreContainer);
    let direction = style.direction();
    let claims = style.is_containing_block();
    layout_candidates(tree, node, &batch, area, direction, claims, grid)
}

/// The `candidates` as a batch, a box that is fixed by its style or by its
/// containing block counting as fixed.
fn oof_batch(tree: &LayoutSession<'_>, candidates: impl Iterator<Item = OofCandidate>) -> OofBatch {
    OofBatch::collect(candidates.map(|mut candidate| {
        let node = usize::from(candidate.node);
        let css_is_fixed =
            tree.nodes[node].position() == &raikiri_style::property::PositionValue::Fixed;
        if tree.nodes[node].fixed_position || css_is_fixed {
            candidate.position = OofPosition::Fixed;
        }
        candidate
    }))
}

/// Compute the root layout while resolving intrinsic constraints on the
/// fixed and otherwise unclaimed out-of-flow boxes against the initial
/// containing block.
///
/// The library positions those boxes in a pass of its own once the root is laid
/// out, so the overrides a box that `stretch` sizes from its static position
/// needs (see [`static_position_overrides`]) are in place for that pass alone:
/// [`LayoutSession::install_root_static_position_overrides`] pushes them as the
/// root layout is stored, and
/// [`LayoutSession::remove_root_static_position_overrides`] pops them when the
/// library records the boxes it positioned. The scope here restores whatever
/// is left, which is all of them when the library reads the root from its
/// cache and records nothing.
pub(crate) fn compute_root_layout_with_content_limits(
    tree: &mut LayoutSession<'_>,
    root: NodeId,
    available_space: Size<AvailableSpace>,
) {
    // The initial containing block takes the writing mode of the root element.
    with_oof_axes(tree, root, |tree| {
        super::content_limits::with_oof_content_limits(tree, None, &[root], |tree| {
            tree.context.root_oof_capture = Some(RootOofCapture {
                root,
                available_space,
                candidates: OofCandidates::new(),
                checkpoint: None,
            });
            tree.with_overrides(Vec::new(), |tree| {
                compute_root_layout(tree, root, available_space);
            });
            tree.context.root_oof_capture = None;
        });
    });
}

impl LayoutSession<'_> {
    /// Push the overrides that give the out-of-flow boxes the library is about
    /// to position against the initial containing block their static
    /// positions, when `root` is the root whose candidates are being recorded.
    ///
    /// The library lays the root out, stores its layout, and only then positions
    /// the boxes it handed back, so this runs once its layout is stored.
    pub(crate) fn install_root_static_position_overrides(&mut self, root: NodeId) {
        let Some((handed, available_space)) = self
            .context
            .root_oof_capture
            .as_mut()
            .filter(|capture| capture.root == root && capture.checkpoint.is_none())
            .map(|capture| {
                (
                    std::mem::take(&mut capture.candidates),
                    capture.available_space,
                )
            })
        else {
            return;
        };
        let overrides = root_static_position_overrides(self, root, &handed, available_space);
        let checkpoint = self.override_checkpoint();
        self.push_overrides(overrides);
        if let Some(capture) = self.context.root_oof_capture.as_mut() {
            // The measures that size the tables among `handed` lay out boxes
            // inside the root, never the root itself, which is the only layout
            // that records candidates.
            debug_assert!(capture.candidates.is_empty());
            capture.candidates = handed;
            capture.checkpoint = Some(checkpoint);
        }
    }

    /// Pop the overrides [`LayoutSession::install_root_static_position_overrides`]
    /// pushed, when `root` is the root they were pushed for.
    ///
    /// The library records the boxes it positioned right after the pass, and
    /// what it records of a box, whether its static position moves it with
    /// its parent, is read from the style the author gave it.
    pub(crate) fn remove_root_static_position_overrides(&mut self, root: NodeId) {
        let checkpoint = self
            .context
            .root_oof_capture
            .as_mut()
            .filter(|capture| capture.root == root)
            .and_then(|capture| capture.checkpoint.take());
        if let Some(checkpoint) = checkpoint {
            self.restore_overrides_to(checkpoint);
        }
    }
}

/// [`static_position_overrides`] for the `handed` candidates that the root
/// positions against the initial containing block: the page area, the available
/// space less the scrollbars of the root, anchored at the origin of the canvas
/// as the library anchors it. The page area is definite in both axes.
///
/// The tables among them keep the `max-width` floor of
/// [`super::content_limits::table_size_overrides`], as the tables a box
/// contains do.
fn root_static_position_overrides(
    tree: &mut LayoutSession<'_>,
    root: NodeId,
    handed: &OofCandidates,
    available_space: Size<AvailableSpace>,
) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    if handed.is_empty() {
        return Vec::new();
    }
    let layout = tree.nodes[usize::from(root)].layout;
    let area = available_space
        .width
        .into_option()
        .zip(available_space.height.into_option())
        .map(|(width, height)| OofPositioningArea {
            size: Size {
                width: (width - layout.scrollbar_size.width).max(0.0),
                height: (height - layout.scrollbar_size.height).max(0.0),
            },
            offset: Point {
                x: -layout.location.x,
                y: -layout.location.y,
            },
        });
    area.map_or_else(Vec::new, |area| {
        let batch = oof_batch(tree, handed.iter());
        let mut overrides = static_position_overrides(
            tree,
            root,
            &batch,
            area,
            ContainingBlockClaims {
                absolute: true,
                fixed: true,
            },
        );
        let nodes = batch
            .iter()
            .map(|candidate| candidate.node)
            .collect::<Vec<_>>();
        overrides.extend(table_size_overrides(tree, root, &nodes, area, None));
        overrides
    })
}

/// How the grid areas that boxes name resolve in a grid container: the inline
/// direction of the container and, when it is laid out on the axes of a
/// vertical writing mode, those axes.
#[derive(Clone, Copy)]
struct GridAreaBasis {
    direction: Direction,
    axes: Option<grid_oof::GridAxes>,
}

/// [`super::content_limits::table_size_overrides`] for the `candidates` that
/// `owner` positions against `area`, its positioning area.
///
/// A grid container places a box that names a grid area in that area. Width
/// constraints use the area's physical width; percentage padding uses its
/// inline size. `basis` resolves both dimensions of that area. Without it,
/// or without the tracks, placed boxes keep their authored values until a
/// positioning area is available.
fn table_size_overrides(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    candidates: &[NodeId],
    area: OofPositioningArea,
    basis: Option<GridAreaBasis>,
) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    if tree.nodes[usize::from(owner)].formatting_context != FormattingContext::Grid {
        return super::content_limits::table_size_overrides(
            tree,
            candidates,
            area.size,
            tree.context.oof_axes_vertical,
        );
    }
    let mut floors = Vec::new();
    let mut in_containing_block = Vec::new();
    for &candidate in candidates {
        let style = tree.query_style(usize::from(candidate), ReadRole::Oof);
        if !style.has_grid_placement() {
            in_containing_block.push(candidate);
            continue;
        }
        let containing_size = basis.and_then(|basis| {
            let info = tree.context.grid_layout_info.get(&usize::from(owner));
            grid_oof::GridAreas::of(info, basis.axes)
                .map(|areas| areas.positioning_area(&style, basis.direction, area).size)
        });
        if let Some(size) = containing_size {
            floors.extend(super::content_limits::table_size_overrides(
                tree,
                &[candidate],
                size,
                tree.context.oof_axes_vertical,
            ));
        }
    }
    floors.extend(super::content_limits::table_size_overrides(
        tree,
        &in_containing_block,
        area.size,
        tree.context.oof_axes_vertical,
    ));
    floors
}

/// Run `layout` with the boxes it positions aligned on the axes of `container`,
/// their containing block: the library aligns a box by `justify-self` along the
/// physical horizontal axis and by `align-self` along the vertical one, and on
/// the axes of a vertical writing mode it is the other way round (CSS Position 3
/// §5.2). Boxes laid out inside this one are positioned by a call of their own.
fn with_oof_axes<R>(
    tree: &mut LayoutSession<'_>,
    container: NodeId,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let vertical = super::block_right_to_left(tree, usize::from(container)).is_some();
    let outer_axes = std::mem::replace(&mut tree.context.oof_axes_vertical, vertical);
    let result = layout(tree);
    tree.context.oof_axes_vertical = outer_axes;
    result
}

pub(super) fn layout_candidates(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    candidates: &OofBatch,
    area: OofPositioningArea,
    direction: Direction,
    claims: ContainingBlockClaims,
    grid: Option<grid_oof::GridAxes>,
) -> OofLayoutResult {
    let candidate_ids = candidates
        .iter()
        .filter(|candidate| claims_position(claims, candidate.position))
        .map(|candidate| candidate.node)
        .collect::<Vec<_>>();
    let result = with_oof_axes(tree, owner, |tree| {
        super::content_limits::with_oof_content_limits(tree, Some(&candidate_ids), &[], |tree| {
            let span: OofSpan<'_> = candidates.all();
            let basis = Some(GridAreaBasis {
                direction,
                axes: grid,
            });
            let floors = table_size_overrides(tree, owner, &candidate_ids, area, basis);
            match grid {
                Some(axes) => tree.with_overrides(floors, |tree| {
                    grid_oof::compute_oof_layout(tree, owner, span, area, direction, claims, axes)
                }),
                None => {
                    let mut overrides =
                        static_position_overrides(tree, owner, candidates, area, claims);
                    overrides.extend(floors);
                    if overrides.is_empty() {
                        compute_oof_layout_for_area(tree, owner, span, area, direction, claims)
                    } else {
                        tree.with_overrides(overrides, |tree| {
                            compute_oof_layout_for_area(tree, owner, span, area, direction, claims)
                        })
                    }
                }
            }
        })
    });
    tree.clear_hoisted_children(owner);
    tree.add_hoisted_children(owner, result.hoisted.iter().copied());
    if super::block_right_to_left(tree, usize::from(owner)).is_some() {
        record_safe_static_positions(tree, candidates, &result.hoisted);
    }
    result
}

/// The overrides that make Taffy size a `stretch` box from its static position.
///
/// A box whose insets are both `auto` in an axis sits at its static position,
/// and the room it can fill reaches from that position to the far edge of the
/// containing block (CSS Position 3 §5.2, CSS Sizing 4 §5.1). Taffy resolves
/// `stretch` against the containing block less the insets that are not `auto`,
/// so it fills the whole block and the box overflows by the offset of its
/// static position. A box with an inset on the side it is aligned to, equal to
/// that position, is sized and placed as the box with `auto` insets would be
/// by the rule, so each qualifying box gets that inset: the left or top one for
/// a box aligned to the start of its static area, the right or bottom one for
/// a box aligned to its end. A box that overflows stays anchored to that edge
/// whatever the overflow position of the alignment says, as in Chromium.
///
/// A box centered in its static area gets both insets instead. They bound the
/// inset-modified containing block, which is centered on the static position
/// and reaches no further than the nearer edge of the containing block, and a
/// center alignment keeps a size cut short by `max-width` centered in it. Only
/// an overflow position the box asks for itself is kept: that of the container
/// decides where the static position is, not how the box falls back. An `auto`
/// margin stays zero, as it is beside an `auto` inset.
///
/// A grid container places its boxes in the grid area they name, which is not
/// the area the static position is measured in.
fn static_position_overrides(
    tree: &LayoutSession<'_>,
    owner: NodeId,
    candidates: &OofBatch,
    area: OofPositioningArea,
    claims: ContainingBlockClaims,
) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    if tree.nodes[usize::from(owner)].formatting_context == FormattingContext::Grid {
        return Vec::new();
    }
    let mut entries = Vec::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| claims_position(claims, candidate.position))
    {
        let idx = usize::from(candidate.node);
        let style = tree.query_style(idx, ReadRole::Oof);
        let (size, inset, margin) = (style.size(), style.inset(), style.margin());
        let key = tree
            .nodes
            .box_key(idx)
            .expect("an out-of-flow candidate has a box key");
        let px_override = |property, value| {
            (
                key,
                OwnedPropertyOverride::length(
                    property,
                    raikiri_style::ComputedLengthPercentageOrAuto::Px(value),
                    false,
                )
                .expect("static position override"),
            )
        };
        let axes = [
            (
                size.width.expand() == ExpandedDimension::Stretch
                    && inset.left.is_auto()
                    && inset.right.is_auto(),
                candidate.static_position.x,
                area.offset.x,
                area.size.width,
                (LengthProperty::Left, LengthProperty::Right),
                [
                    (LengthProperty::MarginLeft, margin.left.is_auto()),
                    (LengthProperty::MarginRight, margin.right.is_auto()),
                ],
            ),
            (
                size.height.expand() == ExpandedDimension::Stretch
                    && inset.top.is_auto()
                    && inset.bottom.is_auto(),
                candidate.static_position.y,
                area.offset.y,
                area.size.height,
                (LengthProperty::Top, LengthProperty::Bottom),
                [
                    (LengthProperty::MarginTop, margin.top.is_auto()),
                    (LengthProperty::MarginBottom, margin.bottom.is_auto()),
                ],
            ),
        ];
        for (qualifies, position, offset, extent, (start, end), margins) in axes {
            if !qualifies {
                continue;
            }
            match position.align.keyword {
                AxisStaticEdge::Start => {
                    entries.push(px_override(start, position.area.start - offset));
                }
                AxisStaticEdge::End => {
                    entries.push(px_override(end, extent - (position.area.end - offset)));
                }
                AxisStaticEdge::Center => {
                    // The library reads the alignment along the physical x
                    // axis from `justify-self` and the one along y from
                    // `align-self`, unless the axes are swapped.
                    let (alignment, own): (fn(_) -> _, _) =
                        if (start == LengthProperty::Left) != tree.oof_alignment_swapped(idx) {
                            (
                                OwnedPropertyOverride::justify_self,
                                style.bridge_justify_self(),
                            )
                        } else {
                            (OwnedPropertyOverride::align_self, style.bridge_align_self())
                        };
                    // Either the box asks for the center itself, or a flex
                    // container centers it and ignores its `justify-self`
                    // along the main axis, so the override replaces no
                    // placement the box needs. Chromium centers a box that
                    // overflows the room on both sides unless the box asks
                    // for `safe`, where the library would keep it inside the
                    // containing block.
                    let safety = if own.is_some_and(|own| own.safety == AlignmentSafety::Safe) {
                        AlignmentSafety::Safe
                    } else {
                        AlignmentSafety::Unsafe
                    };
                    // A static position on or past an edge of the containing
                    // block leaves no room: the box is centered on it.
                    let center = (position.area.start + position.area.end) / 2.0 - offset;
                    let half = center.min(extent - center).max(0.0);
                    // With both insets stated the library gives free space to
                    // an `auto` margin, while with an `auto` inset it counts
                    // as zero, as the box asked for.
                    entries.extend(
                        margins
                            .into_iter()
                            .filter(|&(_, auto)| auto)
                            .map(|(property, _)| px_override(property, 0.0)),
                    );
                    entries.extend([
                        px_override(start, center - half),
                        px_override(end, extent - (center + half)),
                        (
                            key,
                            alignment(Some(AlignItems {
                                safety,
                                ..AlignItems::CENTER
                            })),
                        ),
                    ]);
                }
            }
        }
    }
    entries
}

/// Remember the static positions of the `hoisted` boxes among `candidates`
/// that align `safe` in an axis, for the vertical containing block that
/// positioned them to align them again.
fn record_safe_static_positions(
    tree: &mut LayoutSession<'_>,
    candidates: &OofBatch,
    hoisted: &[NodeId],
) {
    let safe = |axis: AxisStaticPosition| axis.align.safety == StaticAlignSafety::Safe;
    for candidate in candidates.iter() {
        let node = usize::from(candidate.node);
        let position = without_last_baseline_safety(tree, node, candidate.static_position);
        if (safe(position.x) || safe(position.y)) && hoisted.contains(&candidate.node) {
            tree.context.safe_static_positions.insert(node, position);
        }
    }
}

/// `position`, the static position of `node`, without the overflow position of
/// the axes in which it aligns to the last baseline. The `safe self-end`
/// fallback of such an alignment keeps the box at the end edge when it
/// overflows in the web-platform-tests
/// (`grid-abspos-staticpos-align-self-vertWM-last-baseline-*`), which Taffy's
/// fallback to the left or top edge matches.
fn without_last_baseline_safety(
    tree: &LayoutSession<'_>,
    node: usize,
    position: Point<AxisStaticPosition>,
) -> Point<AxisStaticPosition> {
    tree.taffy_parent_of(node).map_or(position, |parent| {
        let (style, parent_style) = (tree.physical_style(node), tree.physical_style(parent));
        // The parent aligns the box along its block axis by `align-self`, which
        // is the physical horizontal axis of a vertical parent.
        let parent_vertical = super::block_right_to_left(tree, parent).is_some();
        let last_baseline =
            |justify| style.self_baseline_set(&parent_style, justify) == Some(BaselineSet::Last);
        let mut position = position;
        if last_baseline(!parent_vertical) {
            position.x.align.safety = StaticAlignSafety::Default;
        }
        if last_baseline(parent_vertical) {
            position.y.align.safety = StaticAlignSafety::Default;
        }
        position
    })
}
