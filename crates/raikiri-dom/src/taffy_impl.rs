//! Taffy layout trait implementations on Document.
//!
//! Production adaptation of the SpikeTree pattern from the original
//! `taffy-layout-modes` spike, equivalent in scope to that prototype:
//! - `TraversePartialTree`: children iterator
//! - `CacheTree`: per-node cache getter / setter
//! - `LayoutPartialTree`: dispatch block / flexbox / grid based on display
//! - `LayoutBlockContainer / LayoutFlexboxContainer / LayoutGridContainer`:
//!   style getter marker impls

use taffy::tree::{RequestedAxis, RunMode};
use taffy::{
    AvailableSpace, BlockContext, BoxGenerationMode, CacheTree, CoreStyle, DetailedGridInfo,
    Display, Layout, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, Position as TaffyPosition, Size, Style,
    TraversePartialTree, TraverseTree, compute_block_layout, compute_cached_layout,
    compute_flexbox_layout, compute_grid_layout, compute_leaf_layout,
};

use crate::document::Document;
use crate::node::{NodeData, NodeFlags};
use raikiri_style::property::DisplayValue;

/// Combines an `<img>`'s resolved intrinsic size (if any) with a text
/// node's shaped intrinsic size (if any) into the single `Option<Size<f32>>`
/// the leaf-measure closures fall back to when CSS gives no explicit size.
/// A given `Node` is either an element or a text node
/// ([`crate::node::NodeData`]), so at most one of
/// [`crate::node::Node::image_intrinsic_size`] /
/// [`crate::node::Node::text_layout`] ever returns `Some` here.
fn leaf_intrinsic_size(
    node: &mut crate::node::Node,
    available_width: Option<f32>,
) -> Option<Size<f32>> {
    node.image_intrinsic_size()
        .map(|(width, height)| Size { width, height })
        .or_else(|| {
            node.text_layout_size_for_width(available_width)
                .map(|(width, height)| Size { width, height })
        })
}

/// Resolve a `calc()` payload handle against `basis`.
///
/// The handle does not depend on the document, so leaf measure callbacks that
/// cannot borrow the tree call this directly.
#[allow(unsafe_code)]
pub(crate) fn resolve_calc(val: *const (), basis: f32) -> f32 {
    // `layout::apply_computed_to_style` owns the boxed payload for the
    // duration of this layout pass, so the Taffy handle is valid here.
    let value = unsafe { &*(val as *const raikiri_style::property::CalcLengthPercentage) };
    let resolved = basis * value.percent / 100.0 + value.px;
    if resolved.is_finite() { resolved } else { 0.0 }
}

/// Resolve a padding or border length of an ifc root against the parent's
/// width.
fn ifc_edge(value: taffy::LengthPercentage, parent_width: Option<f32>) -> f32 {
    let basis = parent_width.unwrap_or(0.0);
    let raw = value.into_raw();
    if raw.is_calc() {
        resolve_calc(raw.calc_value(), basis)
    } else {
        crate::layout::used_style_length_percentage(value, basis).unwrap_or(0.0)
    }
}

/// Padding-top plus border-top of an ifc root, resolved from its style.
///
/// The parent has not stored this node's layout yet when the root is
/// measured, so the insets come from the style and the parent's width.
fn ifc_top_inset(style: &Style, parent_width: Option<f32>) -> f32 {
    ifc_edge(style.padding.top, parent_width) + ifc_edge(style.border.top, parent_width)
}

/// Border plus padding on the left and right of an ifc root, resolved from
/// its style like [`ifc_top_inset`].
fn ifc_horizontal_edges(style: &Style, parent_width: Option<f32>) -> (f32, f32) {
    (
        ifc_edge(style.padding.left, parent_width) + ifc_edge(style.border.left, parent_width),
        ifc_edge(style.padding.right, parent_width) + ifc_edge(style.border.right, parent_width),
    )
}

/// Taffy child iterator: filter nodes with `is_in_document() == false`
/// (such as a detached `<template>` contents fragment) from the raw arena
/// children.
///
/// The taffy layout tree is the web-spec “flat tree,” so layout must treat
/// template contents as nonexistent. Skipping them only at paint time would
/// still let their layout sizes / positions shift sibling positions.
/// `raikiri_traits::Dom::child_ids` returns raw children by contract;
/// filter only on the taffy path rather than changing that contract.
pub struct TaffyChildIter<'a> {
    doc: &'a Document,
    parent: NodeId,
    inner: core::slice::Iter<'a, usize>,
}

impl TaffyChildIter<'_> {
    fn children(doc: &Document, parent: NodeId) -> &[usize] {
        doc.nodes[usize::from(parent)].layout_children()
    }

    fn includes(doc: &Document, parent: NodeId, child: usize) -> bool {
        if !doc.nodes[child].is_in_document() {
            return false;
        }
        // CSS Flexbox §4: anonymous flex items are not generated for
        // whitespace-only text nodes. The same filtering is needed for Grid,
        // whose item collection also excludes inter-element source whitespace.
        // A synthetic inline root is the one exception: its whitespace nodes
        // are explicit zero-content inline items whose collapsed advance was
        // measured by `preshape_text` and stored in their style.
        let parent_node = &doc.nodes[usize::from(parent)];
        if !matches!(parent_node.style.display, Display::Flex | Display::Grid)
            || parent_node.flags.contains(NodeFlags::IS_INLINE_ROOT)
        {
            return true;
        }
        !matches!(
            &doc.nodes[child].data,
            NodeData::Text(text)
                if text
                    .text_content
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
        )
    }
}

impl Iterator for TaffyChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        let doc = self.doc;
        let parent = self.parent;
        for &child in self.inner.by_ref() {
            if Self::includes(doc, parent, child) {
                return Some(NodeId::from(child));
            }
        }
        None
    }
}

impl TraversePartialTree for Document {
    type ChildIter<'a> = TaffyChildIter<'a>;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        TaffyChildIter {
            doc: self,
            parent: node_id,
            inner: TaffyChildIter::children(self, node_id).iter(),
        }
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        // Must match the filter: count only in-document children.
        TaffyChildIter::children(self, node_id)
            .iter()
            .filter(|&&c| TaffyChildIter::includes(self, node_id, c))
            .count()
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        // Filtered index: return the nth child in the same view as child_ids.
        let idx = TaffyChildIter::children(self, node_id)
            .iter()
            .copied()
            .filter(|&c| TaffyChildIter::includes(self, node_id, c))
            .nth(index)
            .expect("get_child_id: index out of range");
        NodeId::from(idx)
    }
}

impl TraverseTree for Document {}

impl CacheTree for Document {
    fn cache_get(&mut self, node_id: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        self.nodes[usize::from(node_id)].cache.get(inputs)
    }

    fn cache_store(&mut self, node_id: NodeId, inputs: &LayoutInput, layout_output: LayoutOutput) {
        self.nodes[usize::from(node_id)]
            .cache
            .store(inputs, layout_output)
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        self.nodes[usize::from(node_id)].cache.clear();
    }
}

impl LayoutPartialTree for Document {
    type CustomIdent = String;
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    /// The **only** path by which taffy writes layout into the arena.
    ///
    /// Passing through [`crate::layout::sanitize_taffy_layout`] here structurally
    /// guarantees that `Node.unrounded_layout` contains no non-finite f32 values.
    /// The input guard in the bridge (`layout::sanitize_taffy`) alone cannot
    /// close the gap: nested percentages compound at the used-value layer
    /// and can become non-finite again. See the documentation for
    /// `layout::sanitize_taffy_layout` for the reasoning and measurements.
    ///
    /// Record fields actually clamped in `self.layout_warnings` (an owned buffer).
    /// The `taffy` crate fixes this trait-method signature, so diagnostic
    /// arguments cannot be added. Instead, write to the owned buffer through
    /// `self` without changing the signature. At pass end, `layout_single_page`
    /// drains the buffer to an observer or eprintln (see the documentation for
    /// the diagnostics buffer below).
    /// See [`Document::layout_warnings`](crate::document::Document).
    ///
    /// Bind the RHS to `sanitized` before assigning the LHS. This deliberately
    /// avoids borrowing two disjoint fields of `self` (`self.nodes[..]` via
    /// IndexMut and `self.layout_warnings`) in the same assignment expression.
    /// Besides satisfying borrowck, it makes evaluation order and dependencies
    /// explicit to readers.
    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        let sanitized = crate::layout::sanitize_taffy_layout(layout, &mut self.layout_warnings);
        self.nodes[usize::from(node_id)].unrounded_layout = sanitized;
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        resolve_calc(val, basis)
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        self.compute_child_layout_with_block_ctx(node_id, inputs, None)
    }
}

impl Document {
    /// Unified `compute_child_layout` implementation that both
    /// [`LayoutPartialTree::compute_child_layout`] (called with
    /// `block_ctx: None`) and [`LayoutBlockContainer::compute_block_child_layout`]
    /// (called with the caller's [`BlockContext`]) delegate to.
    ///
    /// Threading `block_ctx` through to `compute_block_layout`'s `Display::Block`
    /// arm is what keeps a normal-flow block descendant in the same Block
    /// Formatting Context as its ancestor instead of each one starting a fresh,
    /// independent `BlockFormattingContext` — per CSS2 §9.4.1
    /// <https://www.w3.org/TR/CSS2/visuren.html#block-formatting> a box is only
    /// in the *same* BFC as its parent when it doesn't itself establish a new
    /// one (taffy's own `is_in_same_bfc` gate in `compute_block_layout`:
    /// in-flow, not floated, not absolutely positioned, not a table, not a
    /// scroll container). Without this, floats placed by one child would not
    /// be visible to (and would not cause wraparound in) that child's own
    /// nested block descendants, only its direct siblings.
    ///
    /// Overflow is bridged to `taffy::Style::overflow`; the remaining
    /// overflow-area details are handled by the paint and block-layout paths.
    fn compute_child_layout_with_block_ctx(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        // Lazy layout cache invalidation — mutations only set a flag; we
        // clear all node caches on the first compute after the flag flips
        // (amortized O(1) per mutation over N-node batches).
        if self.layout_dirty {
            for node in &mut self.nodes {
                node.cache.clear();
            }
            self.layout_dirty = false;
        }
        let mut output = compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let idx = usize::from(node_id);
            // Table dispatch uses preserved DisplayValue (not taffy's collapsed Display::Block).
            // Taffy 0.12 has no table layout; we route to native table engine in parallel with
            // Block/Flex/Grid (spec requires parallel Display::Block/Flex/Grid handling).
            {
                use crate::node::NodeFlags;
                use raikiri_style::property::DisplayValue;
                let dv = tree.nodes[idx].display;
                // Pure-inline table boxes carry IS_INLINE_ROOT (set by
                // establish_minimal_line_boxes for all-inline children) and
                // flow as flex instead: they are exactly one anonymous
                // cell's content, which the grid collector cannot represent.
                let inline_flow = tree.nodes[idx].flags.contains(NodeFlags::IS_INLINE_ROOT);
                if (dv == DisplayValue::Table || dv == DisplayValue::InlineTable) && !inline_flow {
                    return crate::layout::table::compute_table_layout(tree, node_id, inputs);
                }
            }
            let display = tree.nodes[idx].style.display;
            if display == Display::None {
                return LayoutOutput::HIDDEN;
            }
            // Inline-block shrink-wrap: bridge_display collapses InlineBlock to
            // taffy Block, so without this a width:auto inline-block would
            // stretch-fill like a plain block. Explicit widths keep the normal
            // block path below; only width:auto sizes to fit-content here.
            {
                use raikiri_style::property::DisplayValue;
                let inline_shrink_wrap = matches!(
                    tree.nodes[idx].display,
                    DisplayValue::InlineBlock | DisplayValue::InlineFlex | DisplayValue::InlineGrid
                );
                if inline_shrink_wrap
                    && tree.nodes[idx].style.size.width.is_auto()
                    && (inputs.known_dimensions.width.is_none()
                        || inputs.sizing_mode != taffy::SizingMode::ContentSize)
                {
                    return compute_inline_block_shrink_wrap(tree, node_id, inputs, block_ctx);
                }
            }
            // CSS multicol is not a native Taffy display mode. Dispatch at
            // this seam so nested containers receive a local fragmentation
            // context instead of being repaired by a global post-pass.
            // Keep malformed/deeply recursive DOM from exhausting the layout
            // stack; the ordinary Taffy path remains the bounded fallback.
            const MAX_NESTED_MULTICOL_DEPTH: usize = 64;
            // cov:ignore: exercised by ignored nested multicol WPT reftests
            if tree.nodes[idx].multicol.is_some()
                && tree.fragmentation_stack.len() < MAX_NESTED_MULTICOL_DEPTH // cov:ignore: exercised by ignored nested multicol WPT reftests
            // cov:ignore: exercised by ignored nested multicol WPT reftests
                && matches!(display, Display::Block | Display::FlowRoot)
            {
                return crate::layout::compute_multicol_layout(tree, node_id, inputs, block_ctx); // cov:ignore: exercised by ignored nested multicol WPT reftests
            }
            let is_leaf = tree.nodes[idx].children.is_empty();
            if tree.nodes[idx].is_inline_svg_root()
                && tree.nodes[idx].attribute("width").is_none()
                && tree.nodes[idx].attribute("height").is_none()
                && let Some(intrinsic) = tree.nodes[idx].image_intrinsic_box()
                && let Some(ratio) = intrinsic.aspect_ratio
            {
                // CSS Sizing 3, intrinsic sizes: ratio-only replaced content
                // uses the definite available inline size, not a 300px default.
                let mut style = tree.nodes[idx].style.clone();
                // Transfer the ratio in measurement only. Taffy's leaf ratio
                // floor would otherwise override explicit/max heights.
                style.aspect_ratio = None;
                return compute_leaf_layout(
                    inputs,
                    &style,
                    |val, basis| tree.resolve_calc_value(val, basis),
                    |known, available| {
                        // Taffy's known dimensions include padding and border;
                        // its available space has already removed those insets.
                        let known = Size {
                            width: known.width.and(available.width.into_option()),
                            height: known.height.and(available.height.into_option()),
                        };
                        let width = known
                            .width
                            .or(known.height.map(|height| height * ratio))
                            .unwrap_or(match available.width {
                                AvailableSpace::Definite(width) => width,
                                AvailableSpace::MinContent => 0.0,
                                AvailableSpace::MaxContent => intrinsic.width,
                            });
                        Size {
                            width,
                            height: known.height.unwrap_or(width / ratio),
                        }
                    },
                );
            }
            if tree.nodes[idx].flags.contains(NodeFlags::IS_IFC_ROOT) {
                let style = tree.nodes[idx].style.clone();
                // Border-box top inset, resolved here: the parent has not
                // stored this node's layout yet.
                let top_inset = ifc_top_inset(&style, inputs.parent_size.width);
                // A known width on the input means the parent stretched or
                // fixed the box; otherwise the box shrinks to fit.
                let stretched = inputs.known_dimensions.width.is_some();
                let measure = IfcMeasure {
                    run_mode: inputs.run_mode,
                    stretched,
                    width_bounds: ifc_content_width_bounds(&style, inputs.parent_size.width),
                    edges: ifc_horizontal_edges(&style, inputs.parent_size.width),
                    top_inset,
                };
                let mut content_baseline = None;
                let mut output =
                    compute_leaf_layout(inputs, &style, resolve_calc, |known, available| {
                        let (size, baseline) = measure_ifc_root(
                            tree,
                            idx,
                            known.height,
                            available,
                            measure,
                            block_ctx,
                        );
                        content_baseline = baseline;
                        size
                    });
                output.baselines.first = content_baseline.map(|baseline| baseline + top_inset);
                return output;
            }
            if is_leaf {
                let style = tree.nodes[idx].style.clone();
                if tree.nodes[idx].has_pre_taffy_text_indent() {
                    compute_leaf_layout(
                        inputs,
                        &style,
                        |_val, _basis| 0.0,
                        |known, available| {
                            // Rebreak ch-aware text at the width Taffy
                            // passes to this measure callback, so its
                            // intrinsic height participates in the current
                            // pass.
                            let leaf_intrinsic = leaf_intrinsic_size(
                                &mut tree.nodes[idx],
                                available.width.into_option(),
                            );
                            Size {
                                width: known
                                    .width
                                    .or(leaf_intrinsic.map(|s| s.width))
                                    .unwrap_or(0.0),
                                height: known
                                    .height
                                    .or(leaf_intrinsic.map(|s| s.height))
                                    .unwrap_or(0.0),
                            }
                        },
                    )
                } else {
                    compute_leaf_layout(
                        inputs,
                        &style,
                        |_val, _basis| 0.0,
                        |known, available| {
                            // Nested fragmentainers may probe a narrower width
                            // than the page-level preshape pass. Keep ordinary
                            // text behavior unchanged, but rebreak text while
                            // a recursive multicol context is active.
                            // cov:ignore: exercised by ignored nested multicol WPT reftests
                            let probe_width = if !tree.fragmentation_stack.is_empty() {
                                available.width.into_option()
                            } else {
                                None
                            };
                            let leaf_intrinsic =
                                leaf_intrinsic_size(&mut tree.nodes[idx], probe_width);
                            // If taffy derives Some known.width / .height from style, prefer
                            // that explicit size; otherwise use parley or image intrinsic sizes;
                            // if neither is available, use zero.
                            Size {
                                width: known
                                    .width
                                    .or(leaf_intrinsic.map(|s| s.width))
                                    .unwrap_or(0.0),
                                height: known
                                    .height
                                    .or(leaf_intrinsic.map(|s| s.height))
                                    .unwrap_or(0.0),
                            }
                        },
                    )
                }
            } else {
                match display {
                    Display::Block | Display::FlowRoot => {
                        compute_block_layout(tree, node_id, inputs, block_ctx)
                    }
                    Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
                    Display::Grid => compute_grid_layout(tree, node_id, inputs),
                    Display::None => unreachable!("Display::None handled above"),
                }
            }
        });
        if let Some(baseline) = first_inline_baseline(self, usize::from(node_id)) {
            output.baselines.first = Some(baseline);
        }
        output
    }
}

/// The node's own `min-width` and `max-width`, as content-box widths.
///
/// A shrink-to-fit box is clamped by these after it is measured, so the lines
/// have to be broken at the clamped width to match the box taffy returns.
/// `auto` gives `0.0` and `f32::INFINITY`.
fn ifc_content_width_bounds(style: &Style, parent_width: Option<f32>) -> (f32, f32) {
    let basis = parent_width.unwrap_or(0.0);
    let resolve_lp = |value: taffy::LengthPercentage| {
        let raw = value.into_raw();
        if raw.is_calc() {
            resolve_calc(raw.calc_value(), basis)
        } else {
            crate::layout::used_style_length_percentage(value, basis).unwrap_or(0.0)
        }
    };
    let insets = if style.box_sizing == taffy::BoxSizing::BorderBox {
        resolve_lp(style.padding.left)
            + resolve_lp(style.padding.right)
            + resolve_lp(style.border.left)
            + resolve_lp(style.border.right)
    } else {
        0.0
    };
    let resolve_dim = |value: taffy::LengthPercentageAuto, auto: f32| {
        let raw = value.into_raw();
        // Calc pointers carry address bits in `tag()`, so test them first.
        let resolved = if raw.is_calc() {
            resolve_calc(raw.calc_value(), basis)
        } else {
            match raw.tag() {
                taffy::CompactLength::LENGTH_TAG => raw.value(),
                taffy::CompactLength::PERCENT_TAG => raw.value() * basis,
                _ => return auto,
            }
        };
        if resolved.is_finite() {
            (resolved - insets).max(0.0)
        } else {
            auto
        }
    };
    (
        resolve_dim(style.min_size.width, 0.0),
        resolve_dim(style.max_size.width, f32::INFINITY),
    )
}

/// What the measure callback of an ifc root needs besides the available
/// space.
#[derive(Clone, Copy)]
struct IfcMeasure {
    run_mode: RunMode,
    /// The parent stretched or fixed the box's width.
    stretched: bool,
    /// The box's own content-box `min-width` and `max-width`.
    width_bounds: (f32, f32),
    /// Border plus padding on the left and right.
    edges: (f32, f32),
    /// Border plus padding above the content box.
    top_inset: f32,
}

/// Measure an ifc root's paragraph for taffy's leaf measure callback.
///
/// The width comes from `available.width`, which taffy has already reduced to
/// the content box. `known` is not used for the width: it is `NONE` when
/// performing layout and border-box when only computing a size. A box the
/// parent stretched or fixed breaks at the available width; a shrink-to-fit
/// box is kept between its min- and max-content widths. The lines wrap around
/// the floats of `block_ctx`, the parent's float context, when there is one.
/// Every performed layout stores its lines, so the last one wins after a
/// parent's probes.
fn measure_ifc_root(
    tree: &mut Document,
    idx: usize,
    known_height: Option<f32>,
    available: Size<AvailableSpace>,
    measure: IfcMeasure,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> (Size<f32>, Option<f32>) {
    use crate::layout::ifc::{flow, root::with_state};
    let Some(root) = tree.nodes[idx].ifc.as_ref() else {
        return (Size::ZERO, None);
    };
    // Take the engine state only around the calls into the engine: laying
    // out another node in between may need the state for that node.
    let probe = root.without_lines();
    let has_boxes = !probe.boxes.is_empty();
    // The boxes are measured as nodes of their own, outside the state scope,
    // and only when the intrinsic widths decide the width.
    let needs_intrinsics =
        !(measure.stretched && matches!(available.width, AvailableSpace::Definite(_)));
    let box_intrinsics = if has_boxes && needs_intrinsics {
        let basis = match available.width {
            AvailableSpace::Definite(width) => width,
            _ => 0.0,
        };
        crate::layout::ifc::boxes::intrinsics_of_boxes(tree, idx, basis)
    } else {
        crate::layout::ifc::boxes::BoxIntrinsics::EMPTY
    };
    let Some((min_content, max_content)) = with_state(tree, |state| {
        flow::intrinsic_widths_with(&probe, &mut state.layout_cx, &box_intrinsics.engine)
    }) else {
        return (Size::ZERO, None);
    };
    let (min_content, max_content) = (
        min_content.max(box_intrinsics.blocks.0),
        max_content.max(box_intrinsics.blocks.1),
    );
    let width = match available.width {
        AvailableSpace::Definite(width) if measure.stretched => width,
        AvailableSpace::Definite(width) => width.max(min_content).min(max_content),
        AvailableSpace::MinContent => min_content,
        AvailableSpace::MaxContent => max_content,
    };
    // A stretched or fixed box already carries its clamped width; a
    // shrink-to-fit box is clamped by its own min/max after measurement.
    let width = if measure.stretched {
        width
    } else {
        width
            .min(measure.width_bounds.1)
            .max(measure.width_bounds.0)
    };
    let geometry = flow::FlowGeometry {
        width,
        edges: measure.edges,
        top_edge: measure.top_inset,
    };
    // Lines, and the paragraph's own floats, are laid out by one loop; it
    // takes the engine state only around each call into the engine.
    let perform = measure.run_mode == RunMode::PerformLayout;
    let lines =
        crate::layout::ifc::boxes::layout_with_boxes(tree, idx, geometry, block_ctx, perform);
    let size = Size {
        width,
        height: known_height.unwrap_or(lines.height),
    };
    let baseline = flow::first_baseline(&lines);
    if measure.run_mode == RunMode::PerformLayout
        && let Some(root) = tree.nodes[idx].ifc.as_mut()
    {
        root.lines = Some(lines);
    }
    (size, baseline)
}

/// Return the first baseline for text leaves from Parley's first line. Taffy's
/// block algorithm propagates that baseline through ordinary inline wrappers;
/// inline-blocks use the baseline of their last in-flow line box, so recover
/// the last in-flow text line from the descendant layout tree. See CSS 2.1
/// §10.8.1: <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
fn first_inline_baseline(doc: &Document, root: usize) -> Option<f32> {
    use raikiri_style::property::DisplayValue;
    let root_node = doc.nodes.get(root)?;
    if root_node.style.display == Display::None {
        return None;
    }
    if let NodeData::Text(text) = &root_node.data {
        let baseline = text
            .text_layout
            .as_ref()?
            .lines()
            .next()?
            .metrics()
            .baseline;
        return baseline.is_finite().then_some(baseline);
    }
    if root_node.display != DisplayValue::InlineBlock {
        return None;
    }

    let mut stack = Vec::new();
    for &child in root_node.children.iter().rev() {
        if doc.nodes[child].is_in_document()
            && doc.nodes[child].style.display != Display::None
            && doc.nodes[child].style.position != TaffyPosition::Absolute
        {
            stack.push((child, doc.nodes[child].unrounded_layout.location.y));
        }
    }
    let mut last_baseline = None;
    while let Some((idx, offset_y)) = stack.pop() {
        let node = &doc.nodes[idx];
        if node.flags.contains(NodeFlags::IS_IFC_ROOT) {
            if let Some(line_baseline) = node
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .and_then(crate::layout::ifc::flow::last_baseline)
            {
                let layout = &node.unrounded_layout;
                let baseline = offset_y + layout.padding.top + layout.border.top + line_baseline;
                if baseline.is_finite() {
                    last_baseline = Some(baseline);
                }
            }
            continue;
        }
        if let NodeData::Text(text) = &node.data {
            if let Some(line) = text
                .text_layout
                .as_ref()
                .and_then(|layout| layout.lines().last())
            {
                let baseline = offset_y + line.metrics().baseline;
                if baseline.is_finite() {
                    last_baseline = Some(baseline);
                }
            }
            continue;
        }
        for &child in node.children.iter().rev() {
            if doc.nodes[child].is_in_document()
                && doc.nodes[child].style.display != Display::None
                && doc.nodes[child].style.position != TaffyPosition::Absolute
            {
                stack.push((
                    child,
                    offset_y + doc.nodes[child].unrounded_layout.location.y,
                ));
            }
        }
    }
    last_baseline
}

/// Shrink-to-fit layout for a width:auto inline-block.
///
/// A width:auto inline-block sizes to its fit-content size: the preferred
/// max-content size clamped by the available width with the min-content size
/// as the floor (CSS 2.1 section 10.3.7 shrink-to-fit, CSS Sizing 3
/// fit-content). This compensates for the bridge mapping inline-block to the
/// taffy Block display, which would otherwise stretch-fill the containing
/// block like a plain block-level box.
///
/// The intrinsic min/max-content widths are measured by running the block (or
/// leaf, for childless boxes) algorithm directly in size-computation mode, not
/// through the cached entry point, so this same width:auto branch is not
/// re-entered for the node being measured. Measurement uses a fresh formatting
/// context, which is the correct shape here because an inline-block
/// establishes an independent context for its contents. The final pass fixes
/// the fitted width as a known dimension and runs the normal algorithm, so
/// style min/max clamps and child positioning behave exactly as they do for
/// an explicitly sized block of the same width.
///
/// Nodes with an explicit width never reach this function (the caller only
/// routes width:auto boxes here), and intrinsic-constraint callers
/// (min/max-content available space) short-circuit to their own bound, which
/// is what the fit-content formula reduces to under such a constraint.
fn compute_inline_block_shrink_wrap(
    tree: &mut Document,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let display = tree.nodes[idx].display;
    let is_leaf = tree.nodes[idx].children.is_empty();
    // Clone what the leaf path needs before any exclusive tree use below.
    let leaf_style = tree.nodes[idx].style.clone();
    let leaf_intrinsic = leaf_intrinsic_size(&mut tree.nodes[idx], None);
    // Scalar copies so the measure closure below captures no large state.
    let sizing_mode = inputs.sizing_mode;
    let known_height = inputs.known_dimensions.height;
    let avail_height = inputs.available_space.height;

    // Intrinsic outer width under the given width constraint.
    let intrinsic_width = |tree: &mut Document, width: AvailableSpace| -> f32 {
        if is_leaf {
            compute_leaf_layout(
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    sizing_mode,
                    axis: RequestedAxis::Horizontal,
                    known_dimensions: Size::NONE,
                    available_space: Size {
                        width,
                        height: avail_height,
                    },
                    ..inputs
                },
                &leaf_style,
                |_val, _basis| 0.0,
                |known, _avail| Size {
                    width: known
                        .width
                        .or(leaf_intrinsic.map(|s| s.width))
                        .unwrap_or(0.0),
                    height: known
                        .height
                        .or(leaf_intrinsic.map(|s| s.height))
                        .unwrap_or(0.0),
                },
            )
            .size
            .width
        } else {
            let intrinsic_inputs = LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode,
                axis: RequestedAxis::Horizontal,
                known_dimensions: Size {
                    width: None,
                    height: known_height,
                },
                available_space: Size {
                    width,
                    height: avail_height,
                },
                ..inputs
            };
            match display {
                DisplayValue::InlineFlex => {
                    compute_flexbox_layout(tree, node_id, intrinsic_inputs)
                        .size
                        .width
                }
                DisplayValue::InlineGrid => {
                    compute_grid_layout(tree, node_id, intrinsic_inputs)
                        .size
                        .width
                }
                _ => {
                    compute_block_layout(tree, node_id, intrinsic_inputs, None)
                        .size
                        .width
                }
            }
        }
    };

    let min_w = intrinsic_width(tree, AvailableSpace::MinContent);
    let max_w = intrinsic_width(tree, AvailableSpace::MaxContent);
    let (lo, hi) = if min_w <= max_w {
        (min_w, max_w)
    } else {
        (max_w, min_w)
    };
    let fit = match inputs.available_space.width {
        AvailableSpace::Definite(avail) => avail.clamp(lo, hi),
        AvailableSpace::MinContent => lo,
        AvailableSpace::MaxContent => hi,
    }
    .max(0.0);

    let final_inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(fit),
            height: inputs.known_dimensions.height,
        },
        available_space: Size {
            width: AvailableSpace::Definite(fit),
            height: inputs.available_space.height,
        },
        ..inputs
    };
    if is_leaf {
        compute_leaf_layout(
            final_inputs,
            &leaf_style,
            |_val, _basis| 0.0,
            |known, _avail| Size {
                width: known
                    .width
                    .or(leaf_intrinsic.map(|s| s.width))
                    .unwrap_or(0.0),
                height: known
                    .height
                    .or(leaf_intrinsic.map(|s| s.height))
                    .unwrap_or(0.0),
            },
        )
    } else {
        match display {
            DisplayValue::InlineFlex => compute_flexbox_layout(tree, node_id, final_inputs),
            DisplayValue::InlineGrid => compute_grid_layout(tree, node_id, final_inputs),
            _ => compute_block_layout(tree, node_id, final_inputs, block_ctx),
        }
    }
}

impl LayoutBlockContainer for Document {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }

    /// Overrides the default (which forwards to
    /// [`LayoutPartialTree::compute_child_layout`] and discards `block_ctx`,
    /// always starting a fresh Block Formatting Context) so that a
    /// same-BFC block child — the case `compute_block_layout` calls this
    /// for — actually continues the caller's [`BlockContext`]. See
    /// [`Document::compute_child_layout_with_block_ctx`]'s doc for why this
    /// matters once floats are involved.
    fn compute_block_child_layout(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        self.compute_child_layout_with_block_ctx(node_id, inputs, block_ctx)
    }
}

impl LayoutFlexboxContainer for Document {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }
}

impl LayoutGridContainer for Document {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }

    fn set_detailed_grid_info(
        &mut self,
        node_id: NodeId,
        detailed_grid_info: DetailedGridInfo<Self::CustomIdent>,
    ) {
        let parent = usize::from(node_id);
        let children = TaffyChildIter::children(self, node_id)
            .iter()
            .copied()
            .filter(|&child| TaffyChildIter::includes(self, node_id, child))
            .filter(|&child| {
                let style = &self.nodes[child].style;
                style.box_generation_mode() != BoxGenerationMode::None
                    && style.position != TaffyPosition::Absolute
            })
            .collect::<Vec<_>>();

        // Taffy builds `DetailedGridInfo.items` from its in-flow children in
        // source order. Pair those resolved rows with the same child projection
        // that Taffy received, so pagination can order by actual placement rather
        // than trying to reconstruct auto-placement or relative offsets.
        let grid_column_count = detailed_grid_info.columns.positions.len();
        debug_assert_eq!(children.len(), detailed_grid_info.items.len());
        self.nodes[parent].grid_item_row_starts = children
            .into_iter()
            .zip(detailed_grid_info.items.iter())
            .map(|(child, item)| (child, item.row_start))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.nodes[parent].grid_column_count = grid_column_count;
    }
}

// SAFETY: with taffy calc support enabled, taffy length types hold
// a raw pointer to a caller-owned calc payload through a compact tagged
// representation. Raw pointers are not Send, making `Style` not Send and
// thus `Document` not Send without this impl.
//
// Invariant, self-contained arena approach:
// Every calc pointer stored in the arena points into the calc payload
// storage owned by the same Document. That storage keeps pointees alive
// with heap allocations behind reference counting, so moving the Document
// to another thread moves the pointer targets with it, preserving validity.
// The layout bridge and the inline text path are the sole constructors of
// owned handles: each one pushes the payload into the same Document's
// storage and then builds the handle from the stored entry.
//
// Enforcement at the boundary:
// The public element constructor sanitizes any externally supplied style
// before storage, replacing foreign calc handles with safe keyword
// fallbacks. Cross-document copies go through the same constructor, so the
// target never adopts the source arena's pointers. The bridge rebuilds
// owned handles from computed values before each layout pass, so valid
// styles without calc pass through untouched.
//
// Do not implement Sync: the planned parallel layout path uses an owned
// deep copy or a read-only borrow of style data. No path reads a shared
// Document reference concurrently across threads, so Sync is unnecessary.
// blitz-dom added an unsafe Sync impl for its node type to serve a parallel
// style traversal that raikiri does not have.
//
// Precedent: blitz-dom Node has a similar unsafe Send impl without
// annotation, an established taffy plus calc pattern. blitz uses an external
// arena model where a separate computed-values chain owns calc data;
// raikiri uses a self-contained arena model with different invariants.
#[allow(unsafe_code)]
unsafe impl Send for Document {}
