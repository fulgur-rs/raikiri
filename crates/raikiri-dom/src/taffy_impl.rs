//! Taffy layout trait implementations on Document.
//!
//! 初期スパイク実装 (`taffy-layout-modes`) の SpikeTree pattern を production
//! 化したもの。実装内容は spike と等価:
//! - `TraversePartialTree`: children iterator
//! - `CacheTree`: per-node cache getter / setter
//! - `LayoutPartialTree`: display に応じて block / flexbox / grid をdispatch
//! - `LayoutBlockContainer / LayoutFlexboxContainer / LayoutGridContainer`:
//!   style getter marker impls

use taffy::tree::{RequestedAxis, RunMode};
use taffy::{
    AvailableSpace, BlockContext, CacheTree, Display, Layout, LayoutBlockContainer,
    LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    NodeId, Size, Style, TraversePartialTree, TraverseTree, compute_block_layout,
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_leaf_layout,
};

use crate::document::Document;
use crate::node::NodeData;
use raikiri_style::property::DisplayValue;

/// Combines an `<img>`'s resolved intrinsic size (if any) with a text
/// node's shaped intrinsic size (if any) into the single `Option<Size<f32>>`
/// the leaf-measure closures fall back to when CSS gives no explicit size.
/// At most one of the two is ever `Some` for a given leaf in this scope
/// (a node is either an `<img>` element or a text node, never both).
fn leaf_intrinsic_size(node: &crate::node::Node) -> Option<Size<f32>> {
    node.image_intrinsic_size()
        .map(|(width, height)| Size { width, height })
        .or_else(|| {
            node.text_layout()
                .map(|l| Size { width: l.width(), height: l.height() })
        })
}

/// Taffy child iterator。raw arena children から `is_in_document() == false`
/// (`<template>` descendants など) を filter する。
///
/// Taffy の layout tree = web spec の "flat tree" なので、layout traversal
/// では template contents を "存在しない" ものとして扱う必要がある (paint 側で
/// skip しても layout 側で size / position が計算されると sibling の位置に
/// 影響してしまう)。`raikiri_traits::Dom::child_ids` は raw children を返す
/// 契約なので、そちらは変更せず、taffy 経路でのみ filter する。
pub struct TaffyChildIter<'a> {
    doc: &'a Document,
    parent: NodeId,
    inner: core::slice::Iter<'a, usize>,
}

impl TaffyChildIter<'_> {
    fn includes(doc: &Document, parent: NodeId, child: usize) -> bool {
        if !doc.nodes[child].is_in_document() {
            return false;
        }
        // CSS Flexbox §4: anonymous flex items are not generated for
        // whitespace-only text nodes. The same filtering is needed for Grid,
        // whose item collection also excludes inter-element source whitespace.
        if !matches!(
            doc.nodes[usize::from(parent)].style.display,
            Display::Flex | Display::Grid
        ) {
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
            inner: self.nodes[usize::from(node_id)].children.iter(),
        }
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        // Filter に一致する必要あり (is_in_document children のみ数える)。
        self.nodes[usize::from(node_id)]
            .children
            .iter()
            .filter(|&&c| TaffyChildIter::includes(self, node_id, c))
            .count()
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        // Filtered index — child_ids iterator と同じ view で n 番目を返す。
        let idx = self.nodes[usize::from(node_id)]
            .children
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

    /// taffy が arena へ layout を書き戻す**唯一の**経路。
    ///
    /// ここで [`crate::layout::sanitize_taffy_layout`] を通すことで
    /// 「`Node.unrounded_layout` は決して非有限 f32 を含まない」を構造的に
    /// 保証する。bridge 側の入力 guard
    /// (`layout::sanitize_taffy`) だけでは nested percentage が used value 層で
    /// 複利して非有限に戻るため閉じない — 理由と実測は
    /// `layout::sanitize_taffy_layout` の doc を参照。
    ///
    /// clamp が実際に発火した field は `self.layout_warnings` (owned buffer)
    /// に積む。この trait method の signature は
    /// `taffy` crate が固定しているため観測用の引数を追加できない —
    /// `self` 経由で書ける owned buffer に積むことで signature を変えずに
    /// 診断を残す。`layout_single_page` がパス終了時にこの buffer を drain
    /// して observer-or-eprintln へ流す (詳細は
    /// [`Document::layout_warnings`](crate::document::Document) の doc)。
    ///
    /// RHS を先に `sanitized` へ束縛してから LHS へ代入する — `self.nodes[..]`
    /// (IndexMut 経由) と `self.layout_warnings` という `self` の 2 つの
    /// disjoint field を 1 文の代入式内で同時に borrow させないための意図的な
    /// 分割 (borrowck を通すためだけでなく、evaluation order を明示して
    /// 読み手に依存関係を隠さない狙いもある)。
    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        let sanitized = crate::layout::sanitize_taffy_layout(layout, &mut self.layout_warnings);
        self.nodes[usize::from(node_id)].unrounded_layout = sanitized;
    }

    #[allow(unsafe_code)]
    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        // `layout::apply_computed_to_style` owns the boxed payload for the
        // duration of this layout pass, so the Taffy handle is valid here.
        let value = unsafe { &*(val as *const raikiri_style::property::CalcLengthPercentage) };
        let resolved = basis * value.percent / 100.0 + value.px;
        if resolved.is_finite() { resolved } else { 0.0 }
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
        compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let idx = usize::from(node_id);
            // Table dispatch uses preserved DisplayValue (not taffy's collapsed Display::Block).
            // Taffy 0.12 has no table layout; we route to native table engine in parallel with
            // Block/Flex/Grid (spec requirement: Display::Block/Flex/Grid並列).
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
                if inline_shrink_wrap && tree.nodes[idx].style.size.width.is_auto() {
                    return compute_inline_block_shrink_wrap(tree, node_id, inputs, block_ctx);
                }
            }
            let is_leaf = tree.nodes[idx].children.is_empty();
            if is_leaf {
                let style = tree.nodes[idx].style.clone();
                let leaf_intrinsic = leaf_intrinsic_size(&tree.nodes[idx]);
                compute_leaf_layout(
                    inputs,
                    &style,
                    |_val, _basis| 0.0,
                    |known, _avail| {
                        // taffy が style から算出した known.width / .height が Some なら
                        // それを優先 (explicit size)、None なら parley intrinsic / 画像
                        // intrinsic を使う、両方無ければ 0。
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
                match display {
                    Display::Block | Display::FlowRoot => {
                        compute_block_layout(tree, node_id, inputs, block_ctx)
                    }
                    Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
                    Display::Grid => compute_grid_layout(tree, node_id, inputs),
                    Display::None => unreachable!("Display::None handled above"),
                }
            }
        })
    }
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
    let leaf_intrinsic = leaf_intrinsic_size(&tree.nodes[idx]);
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
}

// SAFETY: taffy の `calc` feature が enable の場合、`Style::Dimension` は
// `CompactLength` 経由で `*const ()` (caller-owned calc expression arena
// pointer) を保持する。Raw pointer は !Send のため `Style: !Send`、そこから
// `Document: !Send` が導出される。
//
// **Invariant (self-contained arena approach)**:
// raikiri-dom 内で `Style` を保持する任意の型 (Document、将来追加予定の
// LayoutBuffer 等) に格納される全 `CompactLength::calc(ptr)` の `ptr` は、
// 同じ Document が own する calc arena (将来 raikiri-dom 内に追加予定) を
// 指す。この invariant が守られる限り、Document 全体を別 thread へ move
// しても pointer target が follow するため validity は保たれる。
//
// **Sync は付けない**: raikiri の (将来実装予定の) parallel layout 経路
// (16 margin box slot + column-count 並列化) は `Arc<GcpmSnapshot>`
// (owned deep copy、design doc
// §5.4.1) 又は `&Style` の read-only borrow 経由で動作。`&Document` を
// 複数 thread から同時 read する path は無いため Sync は不要。blitz-dom は
// stylo parallel style traversal のため `unsafe impl Sync for Node` を追加
// しているが、raikiri は stylo 非依存で該当 path なし。
//
// **Precedent**: blitz-dom `Node` にも同種の `unsafe impl Send` があり
// (`blitz-dom-0.3.0-beta.1/src/node/node.rs:136`、無注釈)、taffy + calc
// feature 上で確立された pattern。ただし blitz は stylo `Arc<ComputedValues>`
// chain が calc data を own する外部 arena モデル、raikiri は self-contained
// arena モデルで invariant の依存対象が異なる。
//
// **Current state**: calc pointer を populate する path は不在 (Node.style は
// `length(px)` / `percent` / `auto` のみ)。将来 sandboxed resolver で CSS
// calc() を実装する際、calc arena を raikiri-dom 側に配置し、
// `CompactLength::calc(...)` の唯一の callsite が arena allocation と同一
// site に閉じるよう API を絞る (structural enforcement)。calc() 実装前に
// 混入を防ぐ custom lint (`raikiri-lints::no_calc_construction`、design
// §5.4.1) を raikiri-dom crate に導入する予定。
#[allow(unsafe_code)]
unsafe impl Send for Document {}
