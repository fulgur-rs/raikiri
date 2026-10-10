use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use super::SvgStyleProperty;
use crate::PseudoElem;
use crate::computed::{
    ComputedValues, CustomPropertyEnvironment, RunningTemplate, empty_custom_properties,
};
use crate::property::{
    Border, BorderColor, BorderRadius, BorderStyle, CornerRadius, CssWideKeyword, FontWeightValue,
    GridAutoFlowValue, GridLineValue, GridTemplateAreasValue, Length, LengthOrAuto,
    LetterSpacingValue, PositionValue, PropertyValue, RelativeFontSize, Sides, TextIndentLength,
    TextWrapMode, WhiteSpace, WhiteSpaceCollapse, WordSpacingValue, WritingMode,
    initial_grid_auto_track_list, resolve_text_align_internal_center,
    resolve_text_align_match_parent,
};
use crate::resolve::{
    ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ResolveContext,
    calc_ch_factor, used_line_height_length,
};
use crate::rule::{
    expand_background, expand_border, expand_border_bottom, expand_border_bottom_css_wide,
    expand_border_color, expand_border_css_wide, expand_border_left, expand_border_left_css_wide,
    expand_border_right, expand_border_right_css_wide, expand_border_style, expand_border_top,
    expand_border_top_css_wide, expand_border_width, expand_flex, expand_flex_flow, expand_font,
    expand_gap, expand_grid_column, expand_grid_row, expand_margin, expand_margin_block,
    expand_margin_inline, expand_outline, expand_overflow, expand_padding, expand_padding_block,
    expand_padding_inline, expand_place_content, expand_place_items, expand_place_self,
    expand_text_decoration,
};
use crate::specified::{INITIAL_BORDER, SpecifiedValues};
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};

use super::candidate::{
    Candidate, CustomCandidates, ElementCandidates, ElementInput, OwnedCandidates,
    SharedDeclarations,
};
use super::collect::{
    CASCADED_PSEUDO_ELEMENTS, Collector, RankedDecl, STORED_BLOCK_BUDGET, pick_winners,
};
use super::custom_property::{resolve_custom_properties, resolve_deferred_value};
use super::first_line::first_line_property_applies;
use super::limits::{CascadeLimits, ResultBudget, WalkCounts, bytes_of, exhausted, try_filled};
use super::rule_index::AncestorFilter;
use super::selector_match::MatchCaches;
use crate::error::CascadeError;
use crate::media::MediaContext;
use crate::ruletree::RuleTree;

/// Number of recently resolved siblings remembered per parent as sharing
/// sources. Repeated structures usually alternate between a handful of
/// shapes (element, whitespace text, element, ...), so a few slots catch
/// them without making a miss expensive.
const SIBLING_SHARE_SLOTS: usize = 4;

/// The default bound on the heap the sharing sources of every depth hold at
/// once: each source's boxed input and its buffers' capacities, though not
/// what its declarations own beyond their inline size. A node that would pass
/// it is not remembered as a source: fewer nodes share, and every result
/// stays the same. The sizes depend only on the document, so the walk stays
/// deterministic.
pub(crate) const SHARE_RETENTION_BUDGET: usize = 8 << 20;

/// Cleared inputs kept for reuse by the next nodes, at most this many.
const SPARE_INPUTS: usize = 2 * SIBLING_SHARE_SLOTS;

/// The inline bytes of one node's slots in the result's per-node vectors.
const NODE_OUTPUT_BYTES: usize = std::mem::size_of::<ComputedValues>()
    + std::mem::size_of::<Option<WritingMode>>()
    + std::mem::size_of::<crate::property::PageValue>()
    + 2 * std::mem::size_of::<bool>();

/// A node this walk resolved, with the input it was resolved from. The input
/// is boxed so that a share cache, of which a deep document has one per
/// level, stays small and inputs move as pointers.
struct ShareSource {
    id: StyleNodeId,
    input: Box<ElementInput>,
    bytes: usize,
}

/// Recently resolved children of one parent (or of nodes that copied that
/// parent's results), with their inputs, reused as computed-value sources for
/// later siblings with identical cascade input.
#[derive(Default)]
struct SiblingShareCache {
    parent: Option<StyleNodeId>,
    sources: [Option<ShareSource>; SIBLING_SHARE_SLOTS],
    next: usize,
}

/// The inputs the walk holds besides the one it is resolving: the sharing
/// sources of every depth, within a budget, and spares.
struct InputStore {
    /// Indexed by tree depth. The walk is depth-first, so while a parent's
    /// children are being visited, deeper slots belong to their subtrees and
    /// this depth's slot stays bound to that parent.
    caches: Vec<SiblingShareCache>,
    /// The bytes the sources hold, counted by [`Self::cost`].
    held_bytes: usize,
    /// The most the sources may hold; see [`SHARE_RETENTION_BUDGET`].
    budget: usize,
    /// Boxed like the sources' inputs, so that moving one between the two
    /// allocates nothing.
    #[expect(
        clippy::vec_box,
        reason = "spares move into share sources, which box their inputs"
    )]
    spares: Vec<Box<ElementInput>>,
}

impl InputStore {
    fn new(budget: usize) -> Self {
        Self {
            caches: Vec::new(),
            held_bytes: 0,
            budget,
            spares: Vec::new(),
        }
    }

    /// What holding `input` as a source costs: its box and its buffers.
    fn cost(input: &ElementInput) -> usize {
        std::mem::size_of::<ElementInput>() + input.buffer_bytes()
    }

    /// An empty input, reusing a spare's buffers when there is one.
    fn take(&mut self) -> Box<ElementInput> {
        self.spares.pop().unwrap_or_default()
    }

    /// Keeps `input` for reuse, unless enough spares are kept already or its
    /// buffers alone would pass the budget, which would keep the next nodes
    /// from being remembered.
    fn give_back(&mut self, mut input: Box<ElementInput>) {
        if self.spares.len() < SPARE_INPUTS && Self::cost(&input) <= self.budget {
            input.clear();
            self.spares.push(input);
        }
    }

    /// The share cache of `depth`, bound to `parent`. A cache bound to another
    /// parent gives its sources back first.
    fn cache_for(&mut self, depth: usize, parent: StyleNodeId) -> &SiblingShareCache {
        if self.caches.len() <= depth {
            self.caches
                .resize_with(depth + 1, SiblingShareCache::default);
        }
        if self.caches[depth].parent != Some(parent) {
            self.caches[depth].parent = Some(parent);
            self.caches[depth].next = 0;
            for slot in 0..SIBLING_SHARE_SLOTS {
                if let Some(source) = self.caches[depth].sources[slot].take() {
                    self.held_bytes -= source.bytes;
                    self.give_back(source.input);
                }
            }
        }
        &self.caches[depth]
    }

    /// Remembers `input`, which `id` was resolved from, in the share cache of
    /// `depth`, which [`Self::cache_for`] bound to `share_parent`. The oldest
    /// of the cache's sources makes room; a source that would pass the budget
    /// is given back instead.
    fn remember(
        &mut self,
        depth: usize,
        share_parent: StyleNodeId,
        id: StyleNodeId,
        input: Box<ElementInput>,
    ) {
        debug_assert_eq!(self.caches[depth].parent, Some(share_parent));
        let bytes = Self::cost(&input);
        let cache = &self.caches[depth];
        let evicted_bytes = cache.sources[cache.next]
            .as_ref()
            .map_or(0, |source| source.bytes);
        if self.held_bytes - evicted_bytes + bytes > self.budget {
            self.give_back(input);
            return;
        }
        self.held_bytes = self.held_bytes - evicted_bytes + bytes;
        let cache = &mut self.caches[depth];
        let slot = cache.next;
        cache.next = (slot + 1) % SIBLING_SHARE_SLOTS;
        if let Some(evicted) = cache.sources[slot].replace(ShareSource { id, input, bytes }) {
            self.give_back(evicted.input);
        }
    }
}

/// One node waiting in the walk, with what its parent hands down.
struct WalkEntry {
    id: StyleNodeId,
    /// The parent whose computed values the node inherits; `None` for the
    /// walk's first node, which inherits the walk's `parent_computed`.
    parent: Option<StyleNodeId>,
    /// The `rem`/`rlh` context, `None` while the node has no element
    /// ancestor; see [`walk_from`].
    root_ctx: Option<ResolveContext>,
    parent_custom_properties: Arc<CustomPropertyEnvironment>,
    /// Tree depth, which picks the node's share cache.
    depth: usize,
    /// How many element ancestors the node has: the length `ancestor_path`
    /// is truncated to before the node is visited.
    ancestors: usize,
    /// The parent whose children the node is matched against for sharing.
    share_parent: Option<StyleNodeId>,
    /// Whether the node is in [`WalkOptions::retain_subtree`].
    in_retained_subtree: bool,
    /// Whether the node descends from an element with a `::first-line` style.
    in_first_line: bool,
}

/// How [`walk`] runs.
#[derive(Clone, Copy)]
pub(crate) struct WalkOptions {
    /// Whether siblings with the same cascade input copy each other's
    /// results. Tests turn it off for the reference the shared walk must
    /// reproduce exactly.
    pub(crate) sibling_sharing: bool,
    /// The root of a subtree whose elements' candidates the walk keeps, in
    /// [`WalkOutputs::retained_subtree`].
    pub(crate) retain_subtree: Option<StyleNodeId>,
    /// The most the sharing sources may hold; see [`SHARE_RETENTION_BUDGET`].
    /// Passing it only shares less, so it is not one of the `limits`.
    pub(crate) share_retention_budget: usize,
    /// The most the stored `style`-attribute blocks may hold; see
    /// [`STORED_BLOCK_BUDGET`]. Passing it only parses more, so it is not
    /// one of the `limits` either.
    pub(crate) stored_block_budget: usize,
    /// Limits on the walk's work and on what its outputs hold.
    pub(crate) limits: CascadeLimits,
    /// The basis of the viewport-percentage lengths; see
    /// [`crate::CascadeOptions::viewport`].
    pub(crate) viewport: Option<(f32, f32)>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            sibling_sharing: true,
            retain_subtree: None,
            share_retention_budget: SHARE_RETENTION_BUDGET,
            stored_block_budget: STORED_BLOCK_BUDGET,
            limits: CascadeLimits::default(),
            viewport: None,
        }
    }
}

/// What one [`walk`] produces, indexed by node like
/// [`super::CascadeResult::computed`].
pub(crate) struct WalkOutputs {
    pub(crate) computed: Vec<ComputedValues>,
    pub(crate) authored_writing_modes: Vec<Option<WritingMode>>,
    pub(crate) page_values: Vec<crate::property::PageValue>,
    pub(crate) pseudo: HashMap<(StyleNodeId, PseudoElem), ComputedValues>,
    pub(crate) svg_properties: HashMap<StyleNodeId, Vec<SvgStyleProperty>>,
    pub(crate) first_letter_inputs: HashMap<StyleNodeId, super::first_letter::FirstLetterInputs>,
    /// The candidates the first-letter methods recompute first-line text from:
    /// those of the elements under an element with a `::first-line` style,
    /// and of their `::before`, `::after` and `::first-line`. Kept only when
    /// some element has first-letter inputs.
    pub(crate) typographic_inheritance: HashMap<(StyleNodeId, Option<PseudoElem>), OwnedCandidates>,
    pub(crate) opacity_specified: Vec<bool>,
    pub(crate) background_color_specified: Vec<bool>,
    /// The candidates of the elements of [`WalkOptions::retain_subtree`].
    pub(crate) retained_subtree: HashMap<StyleNodeId, OwnedCandidates>,
    /// How many nodes copied their results from a sibling.
    pub(crate) shared_nodes: usize,
    /// What the walk counted against [`WalkOptions::limits`].
    pub(crate) counts: WalkCounts,
}

impl WalkOutputs {
    /// Empty outputs with room for `node_count` nodes, counted against
    /// `budget` before they are allocated. Kept out of [`walk_from`], whose
    /// loop is the cascade's hot path.
    #[inline(never)]
    fn allocate(node_count: usize, budget: &mut ResultBudget) -> Result<Self, CascadeError> {
        budget.output((node_count as u64).saturating_mul(NODE_OUTPUT_BYTES as u64))?;
        // Reserve capacity only, rather than filling every slot with
        // initial() up front: `ComputedValues` is large and cloning it is not
        // cheap, and the walk overwrites almost every slot.
        let mut computed = Vec::new();
        computed
            .try_reserve_exact(node_count)
            .map_err(|_| exhausted::<ComputedValues>(node_count))?;
        Ok(Self {
            computed,
            authored_writing_modes: try_filled(node_count, None)?,
            page_values: try_filled(node_count, crate::property::PageValue::Auto)?,
            pseudo: HashMap::new(),
            svg_properties: HashMap::new(),
            first_letter_inputs: HashMap::new(),
            typographic_inheritance: HashMap::new(),
            opacity_specified: try_filled(node_count, false)?,
            background_color_specified: try_filled(node_count, false)?,
            retained_subtree: HashMap::new(),
            shared_nodes: 0,
            counts: WalkCounts::default(),
        })
    }
}

/// Cascades the whole document: [`walk_from`] its root, which inherits the
/// initial values.
pub(crate) fn walk<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
    options: WalkOptions,
) -> Result<WalkOutputs, CascadeError> {
    walk_from(
        dom,
        rule_tree,
        media_context,
        dom.root_id(),
        &ComputedValues::initial(),
        options,
    )
}

/// The cascade's single walk: one pre-order depth-first traversal from `id`
/// that collects each element's cascade input, chooses and applies its
/// winners, and resolves its pseudo-elements before moving on.
///
/// # Visit order
///
/// Selector matching reads `ancestor_path`, the element ancestors of the
/// node being visited, so the walk must visit ancestors before descendants;
/// visiting a child before its parent would leave the parent absent and
/// falsely reject a descendant or child combinator. Children also need their
/// parent's computed values, which the walk writes before pushing them. The
/// traversal is iterative with an explicit stack to avoid overflow. `stack`
/// interleaves the pending work of several subtrees (sibling branches,
/// cousins, ...), so each entry records how many element ancestors its node
/// has, and truncating `ancestor_path` to that length undoes whatever a
/// since-finished sibling subtree appended, reconstructing exactly the
/// root..parent chain. Children are pushed in reverse so they are visited in
/// document order, which also keeps the inline style cache and the share
/// caches seeing nodes in document order.
///
/// Only `Element`-kind nodes join `ancestor_path`: CSS Selectors L4
/// descendant and child combinators are defined in terms of element ancestry
/// (<https://www.w3.org/TR/selectors-4/#descendant-combinators>: "an element
/// B that is an arbitrary descendant of some ancestor element A").
///
/// # Threading the `rem` context (design document §6.3)
///
/// The `root_ctx` of each entry, `Option<ResolveContext>`, indicates
/// **whether this node has an element ancestor**:
///
/// - `None` — there is no element ancestor. If this node is an element, it is
///   the **root element** and goes through [`SpecifiedValues::finalize_as_root`].
///   That function's docs quote the parent-metrics clause of CSS Values 4 §6.1.1
///   (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) verbatim.
///   They explain why `rem` uses different bases for `font-size` (initial 16px)
///   and box properties (the element's own font-size). **This prevents the
///   incorrect self-reference in `html { font-size: 2rem }` that would result
///   from giving the entire tree one context.**
/// - `Some(ctx)` — there is an element ancestor. The computed font-size of the
///   highest such ancestor (= root element) is `ctx.root_font_size`; its `lh`
///   value (the `rlh` basis, absolutized by `used_line_height_length`, or `None`
///   when `normal` cannot be resolved) is `ctx.root_line_height`.
///
/// [`StyleDom::root_id`] identifies the Document node, not the root element
/// (see the Contract section of [`crate::style_dom`]). Non-element nodes such as
/// Document / Comment / Text therefore pass `None` through to their children.
///
/// A well-formed HTML document has one root element, but `StyleDom` does not
/// enforce this. In a synthetic DOM with several elements directly under the
/// Document, **each acts as a root element** (providing the `rem` basis for its
/// own subtree). This deliberately follows the §6.1.1 rule that elements
/// without an element parent use initial values.
///
/// # Sibling sharing
///
/// A node's results depend only on its parent's computed values and custom
/// properties, the `rem`/`rlh` context, and its own cascade input (see
/// [`ElementInput::same_input`]). Siblings with identical cascade input
/// therefore get identical computed values, pseudo-element values, `page`
/// values, non-UA margin flags, and authored writing modes. Repeated
/// structures (list items, table cells, paragraphs, whitespace text between
/// them) hit this constantly, so each depth keeps a few recently resolved
/// children of the current parent, with their inputs, and copies a match's
/// results instead of redoing winner selection and absolutization. A node
/// that copied its results from `source` hands its children the same parent
/// context as `source`'s children, so those children are matched against
/// `source`'s children: cousins under repeated parents share too. Sharing is
/// limited to nodes with an element ancestor (`root_ctx` is `Some`), because a
/// root element derives its own `rem` context from its computed values. The
/// walk holds no candidates beyond the node it resolves, those sharing
/// sources (within [`WalkOptions::share_retention_budget`]), and what the options and the
/// first-letter methods ask it to keep.
///
/// # Limits
///
/// The walk counts its work and its outputs against [`WalkOptions::limits`]
/// (see [`CascadeLimits`]): the collector counts each element's candidates
/// and selector tests, and the walk counts the per-node outputs before
/// allocating them, then every pseudo-element value, SVG property list and
/// kept candidate copy as it adds it. A node that copies a sibling's results
/// adds the same entries as resolving them would, so the counts do not depend
/// on sharing.
///
/// # Errors
///
/// Fails when the input passes one of [`WalkOptions::limits`], when the
/// allocator refuses an output's buffer, or when the rule index cannot number
/// the active rules or a node's candidates or the candidates kept in the
/// result cannot be numbered (see [`Collector::collect`] and
/// [`OwnedCandidates::copy`]).
pub(crate) fn walk_from<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
    id: StyleNodeId,
    parent_computed: &ComputedValues,
    options: WalkOptions,
) -> Result<WalkOutputs, CascadeError> {
    let viewport = options.viewport.unwrap_or((
        media_context.viewport_width() as f32,
        media_context.viewport_height() as f32,
    ));
    let match_caches = MatchCaches::default();
    let mut collector = Collector::new(
        dom,
        rule_tree,
        media_context,
        &match_caches,
        &options.limits,
        options.stored_block_budget,
    )?; // cov:ignore: the error branch needs a u32 handle overflow
    // Recomputing first-line text only matters to the first-letter methods.
    let keep_typographic = collector.targets(PseudoElem::FirstLetter);
    let node_count = dom.node_count();
    let mut budget = ResultBudget::new(&options.limits);
    let mut out = WalkOutputs::allocate(node_count, &mut budget)?;
    let mut stack = vec![WalkEntry {
        id,
        parent: None,
        root_ctx: None,
        parent_custom_properties: empty_custom_properties(),
        depth: 0,
        ancestors: 0,
        share_parent: None,
        in_retained_subtree: options.retain_subtree == Some(id),
        in_first_line: false,
    }];
    // Ancestor **element** ids, root-most first / immediate-parent last, and
    // a Bloom filter over them, truncated and pushed in lockstep.
    let mut ancestor_path: Vec<StyleNodeId> = Vec::new();
    let mut ancestor_filter = AncestorFilter::new();
    let mut store = InputStore::new(options.share_retention_budget);
    // Scratch buffer for `apply_winners`, allocated **outside** the walk loop
    // and reused for every node. Two per-node `HashMap`s previously accounted
    // for 3,667 allocations / 3.0 MB at n=1000 nodes, or 56.7% of all cascade
    // heap traffic. The buffer grows to the maximum `PropertyKey` index over
    // the first few nodes, then incurs no allocations. `apply_winners` owns its
    // fill and drain; the walk loop only lends it a reusable container.
    let mut winners: Vec<Option<RankedDecl>> = Vec::new();
    // Scratch buffer for the candidates that apply to `::first-line`, reused
    // the same way.
    let mut first_line_scratch: Vec<Candidate> = Vec::new();
    while let Some(entry) = stack.pop() {
        let WalkEntry {
            id,
            parent,
            root_ctx,
            parent_custom_properties,
            depth,
            ancestors,
            share_parent,
            in_retained_subtree,
            in_first_line,
        } = entry;
        ancestor_path.truncate(ancestors);
        ancestor_filter.truncate(ancestors);
        // Skip the entire subtree when is_in_document()==false: detached and
        // template descendants keep initial() instead of inheriting from their
        // parent (nodes under `<template style="color:red">` do not inherit
        // red), and selectors never match inside them. Unknown NodeIds
        // (dom.node returns None) are skipped the same way.
        let Some(node) = dom.node(id).filter(|n| n.is_in_document()) else {
            continue;
        };
        let is_element = node.kind() == StyleNodeKind::Element;
        let is_svg = node
            .as_element()
            .is_some_and(|element| element.namespace_uri() == Some("http://www.w3.org/2000/svg"));
        let mut node_svg_properties = Vec::new();
        let parent_computed =
            parent.map_or(parent_computed, |parent| &out.computed[parent.0 as usize]);
        let idx = id.0 as usize;

        // Collect this element's input. It then joins `ancestor_path` for its
        // children.
        let mut input = store.take();
        if is_element && let Some(elem) = node.as_element() {
            collector.collect(id, &elem, &ancestor_path, &ancestor_filter, &mut input)?;
            // Whether the element's own candidates include `opacity` and
            // `background-color`, whichever declaration wins.
            if let (Some(candidates), _) = input.element(collector.shared()) {
                for candidate in candidates.decls() {
                    match candidate.key() {
                        crate::property::PropertyKey::Opacity => out.opacity_specified[idx] = true,
                        crate::property::PropertyKey::BackgroundColor => {
                            out.background_color_specified[idx] = true;
                        }
                        _ => {}
                    }
                }
            }
            ancestor_path.push(id);
            ancestor_filter.push(&elem);
        }

        let shared = collector.shared();
        let share_source = match (share_parent, root_ctx) {
            (Some(parent), Some(_)) if options.sibling_sharing => store
                .cache_for(depth, parent)
                .sources
                .iter()
                .flatten()
                .find(|source| {
                    dom.node(source.id).is_some_and(|source_node| {
                        source_node.kind() == node.kind()
                            && source_node.as_element().is_some_and(|element| {
                                element.namespace_uri() == Some("http://www.w3.org/2000/svg")
                            }) == is_svg
                    }) && input.same_input(&source.input, shared)
                })
                .map(|source| source.id),
            _ => None,
        };

        let (computed, custom_properties, child_ctx, children_share_parent) = if let Some(source) =
            share_source
        {
            out.shared_nodes += 1;
            let src = source.0 as usize;
            out.page_values[idx] = out.page_values[src].clone();
            out.authored_writing_modes[idx] = out.authored_writing_modes[src];
            if let Some(properties) = out.svg_properties.get(&source) {
                node_svg_properties = properties.clone();
            }
            // The pseudo-element and first-letter copies are counted as
            // resolving the node would count its own entries, before they are
            // made. The SVG property list, one entry per SVG paint property at
            // most, is counted when it is added below, after it exists, as on
            // the resolving path.
            for pseudo in CASCADED_PSEUDO_ELEMENTS {
                if let Some(values) = out.pseudo.get(&(source, pseudo)) {
                    budget.output(entry_bytes(&out.pseudo, 0))?;
                    let values = values.clone();
                    try_insert(&mut out.pseudo, (id, pseudo), values)?;
                }
            }
            if let Some(inputs) = out.first_letter_inputs.get(&source) {
                budget.retained(entry_bytes(
                    &out.first_letter_inputs,
                    inputs.candidates.bytes(),
                ))?;
                let inputs = inputs.clone();
                try_insert(&mut out.first_letter_inputs, id, inputs)?;
            }
            let computed = out.computed[src].clone();
            let custom_properties = computed.custom_properties.clone();
            // `source` was resolved by this walk (only freshly resolved
            // nodes are remembered), and this node now has exactly its
            // results, so this node's children see the same parent
            // context as `source`'s children and may share with them.
            (computed, custom_properties, root_ctx, source)
        } else {
            let (element_candidates, element_custom) = input.element(shared);
            let local_custom_properties = element_custom
                .map(|candidates| resolve_custom_properties(&parent_custom_properties, candidates));
            let custom_properties = local_custom_properties
                .clone()
                .unwrap_or_else(|| parent_custom_properties.clone());
            let local_custom_properties =
                local_custom_properties.unwrap_or_else(empty_custom_properties);

            // Phase 1: apply this node's cascade winners to the initial state of
            // the inheritance walk (inherited fields copied from the parent's
            // computed values; non-inherited fields initialized). The target is a
            // staging representation, so winner application order does not matter.
            let mut specified = SpecifiedValues::inherit_from(parent_computed);
            if let Some(candidates) = element_candidates {
                apply_winners(
                    candidates,
                    &mut winners,
                    &mut specified,
                    parent_computed,
                    &custom_properties,
                    Some(&mut out.page_values[idx]),
                    Some(&mut out.authored_writing_modes[idx]),
                    is_svg.then_some(&mut node_svg_properties),
                );
            }

            // Phases 2 + 3: absolutize. A root element (no element ancestor) uses
            // different `rem` bases in these phases, so it needs a dedicated entry
            // point (see the spec quote in `SpecifiedValues::finalize_as_root` docs).
            let mut computed = match &root_ctx {
                Some(ctx) => specified.finalize(parent_computed, ctx),
                None => {
                    // `finalize_as_root` fixes the phase-2 basis to the initial
                    // value (§6.1.1: "if the element has no parent"). This is
                    // correct because **`root_ctx == None` means this node has no
                    // element parent**. Check this caller-side invariant:
                    //
                    // - `walk` always starts at `dom.root_id()` (= Document
                    //   node) and passes `ComputedValues::initial()` there.
                    // - Only elements have candidates, so the Document node
                    //   retains its initial computed values.
                    // - Only the Document itself and its direct children still
                    //   have `root_ctx == None` (`Some` follows the first element).
                    //
                    // Any future entry point that starts `walk_from` partway
                    // through a subtree (e.g. incremental restyle) must therefore
                    // supply `root_ctx` itself. This assertion catches an omission
                    // in debug builds.
                    debug_assert_eq!(
                        parent_computed.font_size,
                        ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
                        "root_ctx == None は element 親が居ないことを意味するので、\
                         親の computed font-size は initial でなければならない \
                         (subtree の途中から walk を開始していないか?)"
                    );
                    specified.finalize_as_root_in_viewport(viewport.0, viewport.1)
                }
            };
            computed.custom_properties = custom_properties.clone();
            computed.local_custom_properties = local_custom_properties;

            // The rem/rlh context passed to children. Only after phases 2 + 2.5
            // for the root element are `root_font_size` / `root_line_height` known,
            // so this is the first point where the context becomes `Some`.
            // `used_line_height_length` uses the same derivation as
            // `crate::specified::SpecifiedValues::finalize_as_root` uses to build
            // its own `ctx`; `rlh_on_root_element_matches_child_root_line_height_basis`
            // checks that the two agree.
            let child_ctx = match root_ctx {
                Some(ctx) => Some(ctx),
                None if is_element => Some(
                    ResolveContext::with_root_line_height(
                        computed.font_size,
                        used_line_height_length(computed.line_height, computed.font_size),
                    )
                    .with_viewport(viewport.0, viewport.1),
                ),
                None => None,
            };

            // `::before`/`::after` — CSS Pseudo-Elements Module Level 4 §4
            // <https://drafts.csswg.org/css-pseudo-4/#treelike> (tree-abiding
            // pseudo-elements, of which `::before`/`::after` are a subcase):
            // "They inherit any inheritable properties from their originating
            // element; non-inheritable properties take their initial values as
            // usual." `::first-line` is not itself tree-abiding (§2.1
            // <https://drafts.csswg.org/css-pseudo-4/#first-line-pseudo>, see
            // `PseudoElem` doc), but this crate resolves it with the same
            // inherit-then-cascade step below since both are single-originating-
            // element pseudo-elements; §4's `content`-conditioned box-generation
            // rule two paragraphs down does not apply to it (`::first-line` has
            // no `content`-driven box-generation model), only the plain
            // inheritance sentence quoted above. This is computed inline here,
            // right where `id`'s own real
            // children would be, rather than in a separate pass after this
            // whole walk finishes, for one specific reason: `child_ctx` (the
            // `rem`/`rlh` basis `id`'s real children get) is *not* a
            // document-wide constant — a document can have multiple top-level
            // elements directly under the `Document` node, each independently
            // becoming its own `root_ctx == None` root with its own `rem` basis
            // (see `walk_from`'s `root_ctx` doc above). A pseudo's
            // `rem`/`rlh` basis must match whichever one its own real
            // originating element actually resolved against, and that value
            // only exists as this loop's *local* `child_ctx` at this exact
            // point — reconstructing it after the fact would require redoing
            // this same top-level-root bookkeeping in a second pass. Using
            // `computed` (not `parent_computed`) as the inherited-from base and
            // `child_ctx` (not `root_ctx`) as the absolutization context both
            // follow directly from that §4 inheritance framing — a pseudo is
            // one more entry alongside `id`'s real children in every sense this
            // cascade cares about, just one this function itself resolves
            // instead of pushing onto `stack` (there is no `StyleNodeId` for a
            // pseudo-element to push). Whether a box is actually generated from
            // the result — §4.1 <https://drafts.csswg.org/css-pseudo-4/#generated-content>'s
            // separate, `content`-conditioned rule, "When their computed
            // 'content' value is not 'none', these pseudo-elements generate
            // boxes as if they were immediate children of their originating
            // element" — is a downstream (`raikiri-dom`) decision this crate
            // does not make; see `CascadeResult::pseudo`'s doc.
            if is_element {
                for pseudo in CASCADED_PSEUDO_ELEMENTS {
                    let (candidates, custom_candidates) = input.pseudo(pseudo, shared);
                    if candidates.is_none() && custom_candidates.is_none() {
                        continue;
                    }
                    let pseudo_local_custom_properties = custom_candidates.map(|candidates| {
                        resolve_custom_properties(&custom_properties, candidates)
                    });
                    let pseudo_custom_properties = pseudo_local_custom_properties
                        .clone()
                        .unwrap_or_else(|| custom_properties.clone());
                    let pseudo_local_custom_properties =
                        pseudo_local_custom_properties.unwrap_or_else(empty_custom_properties);

                    let mut pseudo_specified = if pseudo == PseudoElem::Marker {
                        SpecifiedValues::inherit_marker_from(&computed)
                    } else {
                        SpecifiedValues::inherit_from(&computed)
                    };
                    // Filter by longhand key before choosing winners, including
                    // deferred var() values and expanded shorthand candidates.
                    let candidates = if pseudo == PseudoElem::FirstLine {
                        candidates.map(|values| {
                            values.filtered(&mut first_line_scratch, |candidate| {
                                first_line_property_applies(candidate.key())
                            })
                        })
                    } else {
                        candidates
                    };
                    if let Some(candidates) = candidates {
                        apply_winners(
                            candidates,
                            &mut winners,
                            &mut pseudo_specified,
                            &computed,
                            &pseudo_custom_properties,
                            None,
                            None,
                            None,
                        );
                    }

                    let ctx = child_ctx.expect(
                        "a generated pseudo-element candidate only exists for a \
                         StyleNodeKind::Element (is_element == true here, since \
                         only elements are ever matched by a selector — the \
                         collector's pseudo-element pass runs only for nodes \
                         `node.as_element()` returns an element for), and \
                         `child_ctx` is `Some` for every element by this point \
                         in the match above",
                    );
                    let mut pseudo_computed = pseudo_specified.finalize(&computed, &ctx);
                    if pseudo == PseudoElem::FirstLetter {
                        let decls = candidates.unwrap_or(ElementCandidates::EMPTY);
                        let custom = custom_candidates.unwrap_or(CustomCandidates::EMPTY);
                        budget.retained(entry_bytes(
                            &out.first_letter_inputs,
                            OwnedCandidates::copy_bytes(decls, custom),
                        ))?;
                        let candidates = OwnedCandidates::copy(decls, custom)?; // cov:ignore: the error branch needs a u32 handle overflow
                        let inputs = super::first_letter::FirstLetterInputs {
                            candidates,
                            context: ctx,
                        };
                        try_insert(&mut out.first_letter_inputs, id, inputs)?;
                    }
                    pseudo_computed.custom_properties = pseudo_custom_properties;
                    pseudo_computed.local_custom_properties = pseudo_local_custom_properties;
                    budget.output(entry_bytes(&out.pseudo, 0))?;
                    try_insert(&mut out.pseudo, (id, pseudo), pseudo_computed)?;
                }
            }
            (computed, custom_properties, child_ctx, id)
        };

        // Keep the candidates the options and the first-letter methods ask for.
        let has_first_line = out.pseudo.contains_key(&(id, PseudoElem::FirstLine));
        if keep_typographic && (in_first_line || has_first_line) {
            keep_typographic_inputs(
                &input,
                id,
                shared,
                &mut out.typographic_inheritance,
                &mut budget,
            )?;
        }
        if in_retained_subtree && let (Some(candidates), _) = input.element(shared) {
            keep_copy(&mut out.retained_subtree, id, candidates, &mut budget)?;
        }
        match (share_parent, root_ctx) {
            (Some(parent), Some(_)) if options.sibling_sharing && share_source.is_none() => {
                store.remember(depth, parent, id, input);
            }
            _ => store.give_back(input),
        }

        if !node_svg_properties.is_empty() {
            budget.output(entry_bytes(
                &out.svg_properties,
                bytes_of::<SvgStyleProperty>(node_svg_properties.len()),
            ))?;
            try_insert(&mut out.svg_properties, id, node_svg_properties)?;
        }

        // `computed` may be shorter than node_count(): only capacity was
        // reserved, so a slot is created the first time the walk reaches it.
        // Ids usually arrive in increasing order, which makes this a plain
        // push; any gap left by an unvisited id is filled with initial().
        if out.computed.len() < idx {
            out.computed.resize(idx, ComputedValues::initial());
        }
        if out.computed.len() == idx {
            out.computed.push(computed);
        } else {
            out.computed[idx] = computed;
        }

        // Push children onto the stack, looking up their already-written
        // parent's computed value by ID. The stack is LIFO, so reverse the
        // children to visit them in document order. Extend `stack` directly
        // from the `child_ids` iterator, then reverse only the newly appended
        // suffix in place; no throwaway intermediate `Vec` is needed.
        // `child_ctx` is `Copy` (via `ResolveContext`) and can be reused inside
        // the closure.
        //
        // Document order also preserves the leak-detection direction
        // documented by `winner_does_not_leak_into_next_sibling`: earlier
        // `<p>` then later `<span>` in document order.
        let start = stack.len();
        let ancestors = ancestor_path.len();
        let in_first_line = in_first_line || has_first_line;
        stack.extend(dom.child_ids(id).map(|child| WalkEntry {
            id: child,
            parent: Some(id),
            root_ctx: child_ctx,
            parent_custom_properties: custom_properties.clone(),
            depth: depth + 1,
            ancestors,
            share_parent: Some(children_share_parent),
            in_retained_subtree: in_retained_subtree || options.retain_subtree == Some(child),
            in_first_line,
        }));
        stack[start..].reverse();
    }
    // Every node gets a slot, visited or not.
    if out.computed.len() < node_count {
        out.computed.resize(node_count, ComputedValues::initial());
    }
    if out.first_letter_inputs.is_empty() {
        out.typographic_inheritance.clear();
    }
    collector.count_into(&mut out.counts);
    budget.count_into(&mut out.counts);
    Ok(out)
}

/// Keeps copies of `id`'s candidates, and of those of its `::before`,
/// `::after` and `::first-line`, for recomputing first-line text later,
/// counting them against `budget`.
fn keep_typographic_inputs(
    input: &ElementInput,
    id: StyleNodeId,
    shared: SharedDeclarations<'_>,
    kept: &mut HashMap<(StyleNodeId, Option<PseudoElem>), OwnedCandidates>,
    budget: &mut ResultBudget,
) -> Result<(), CascadeError> {
    if let (Some(candidates), _) = input.element(shared) {
        keep_copy(kept, (id, None), candidates, budget)?;
    }
    for pseudo in [PseudoElem::Before, PseudoElem::After, PseudoElem::FirstLine] {
        if let (Some(candidates), _) = input.pseudo(pseudo, shared) {
            keep_copy(kept, (id, Some(pseudo)), candidates, budget)?;
        }
    }
    Ok(())
}

/// Keeps a copy of `candidates` in `kept`, counting it against `budget`
/// before making it.
fn keep_copy<K: Eq + Hash>(
    kept: &mut HashMap<K, OwnedCandidates>,
    key: K,
    candidates: ElementCandidates<'_>,
    budget: &mut ResultBudget,
) -> Result<(), CascadeError> {
    budget.retained(entry_bytes(
        kept,
        OwnedCandidates::copy_bytes(candidates, CustomCandidates::EMPTY),
    ))?;
    let candidates = OwnedCandidates::copy(candidates, CustomCandidates::EMPTY)?; // cov:ignore: the error branch needs a u32 handle overflow
    try_insert(kept, key, candidates)
}

/// The bytes one more entry of `map` adds to the result: its inline size and
/// the `heap` bytes its value brings. The walk adds each key of an output map
/// once, so no entry is counted twice.
fn entry_bytes<K, V>(_map: &HashMap<K, V>, heap: u64) -> u64 {
    bytes_of::<(K, V)>(1).saturating_add(heap)
}

/// Inserts into `map`, failing instead when the allocator refuses to grow it.
fn try_insert<K: Eq + Hash, V>(
    map: &mut HashMap<K, V>,
    key: K,
    value: V,
) -> Result<(), CascadeError> {
    map.try_reserve(1)
        .map_err(|_| exhausted::<(K, V)>(map.len().saturating_add(1)))?;
    map.insert(key, value);
    Ok(())
}

/// Find a longhand's surviving value after origin or layer rollback.
fn find_rollback(
    candidates: ElementCandidates<'_>,
    key: crate::property::PropertyKey,
    winner_index: usize,
    keyword: CssWideKeyword,
    custom_properties: &CustomPropertyEnvironment,
) -> Option<PropertyValue> {
    let index = super::rollback::select_layered_winner(candidates.decls(), |idx, candidate| {
        if candidate.key() != key && !candidate.is_all_revert_layer() {
            return None;
        }
        let rollback = if idx == winner_index {
            if keyword == CssWideKeyword::Revert {
                super::rollback::Rollback::Origin
            } else {
                super::rollback::Rollback::Layer
            }
        } else if let PropertyValue::Deferred(deferred) = candidates.value(idx) {
            resolve_deferred_value(deferred, custom_properties)
                .as_ref()
                .map_or(
                    super::rollback::Rollback::None,
                    super::rollback::rollback_kind,
                )
        } else {
            candidate.rollback()
        };
        Some(candidate.precedence().layered(idx, rollback))
    })?;
    Some(candidates.value(index).clone())
}

/// Resolve one border longhand CSS-wide marker to its concrete specified value.
///
/// `inherited` supplies the parent computed border for `Inherit`. `INITIAL_BORDER`
/// supplies `Initial` and, because all `border-*` are non-inherited, `Unset`.
/// `Revert` / `RevertLayer` use [`find_rollback`]; when no surviving winner
/// exists they fall back to [`INITIAL_BORDER`]. A rollback winner that is
/// [`PropertyValue::Deferred`] is resolved through `custom_properties`; a rollback
/// winner that is itself a CSS-wide marker (only `Inherit` / `Initial` / `Unset`
/// can survive [`find_rollback`]'s rollback filters) is resolved recursively one
/// level without further rollback.
fn resolve_border_css_wide(
    keyword: CssWideKeyword,
    key: crate::property::PropertyKey,
    inherited: &ComputedValues,
    candidates: ElementCandidates<'_>,
    winner: RankedDecl,
    custom_properties: &CustomPropertyEnvironment,
) -> Option<PropertyValue> {
    let parent_side = |side: &crate::resolve::ComputedBorder| -> Border {
        Border {
            width: inherited_border_width(side.width()),
            style: side.style(),
            color: side.color,
        }
    };
    let parent_border = || -> Border {
        // Dispatch by key to the matching side; unreachable keys fall back to initial
        // (defensive: every border longhand key maps to one side below).
        match key {
            crate::property::PropertyKey::BorderTopWidth
            | crate::property::PropertyKey::BorderTopStyle
            | crate::property::PropertyKey::BorderTopColor => parent_side(&inherited.border.top),
            crate::property::PropertyKey::BorderRightWidth
            | crate::property::PropertyKey::BorderRightStyle
            | crate::property::PropertyKey::BorderRightColor => {
                parent_side(&inherited.border.right)
            }
            crate::property::PropertyKey::BorderBottomWidth
            | crate::property::PropertyKey::BorderBottomStyle
            | crate::property::PropertyKey::BorderBottomColor => {
                parent_side(&inherited.border.bottom)
            }
            _ => parent_side(&inherited.border.left),
        }
    };
    let initial_border = || -> Border { INITIAL_BORDER };
    let pick_field = |border: Border| -> PropertyValue {
        match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidth(border.width)
            }
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidth(border.width)
            }
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidth(border.width)
            }
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidth(border.width)
            }
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyle(border.style)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyle(border.style)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyle(border.style)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyle(border.style)
            }
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColor(border.color)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColor(border.color)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColor(border.color)
            }
            _ => PropertyValue::BorderLeftColor(border.color),
        }
    };
    match keyword {
        CssWideKeyword::Inherit => Some(pick_field(parent_border())),
        CssWideKeyword::Initial | CssWideKeyword::Unset => Some(pick_field(initial_border())),
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => {
            let rollback = find_rollback(candidates, key, winner.idx, keyword, custom_properties)?;
            // Resolve one level: Deferred needs custom-property substitution;
            // a surviving Inherit/Initial/Unset marker resolves without further rollback.
            match rollback {
                PropertyValue::Deferred(deferred) => {
                    let resolved = resolve_deferred_value(&deferred, custom_properties)?;
                    // A deferred var() may itself substitute to a CSS-wide keyword
                    // (e.g. `--x: inherit`). Resolve that single level directly
                    // against parent/initial to avoid rollback recursion.
                    match resolved {
                        PropertyValue::BorderTopWidthCssWide(kw)
                        | PropertyValue::BorderRightWidthCssWide(kw)
                        | PropertyValue::BorderBottomWidthCssWide(kw)
                        | PropertyValue::BorderLeftWidthCssWide(kw)
                        | PropertyValue::BorderTopStyleCssWide(kw)
                        | PropertyValue::BorderRightStyleCssWide(kw)
                        | PropertyValue::BorderBottomStyleCssWide(kw)
                        | PropertyValue::BorderLeftStyleCssWide(kw)
                        | PropertyValue::BorderTopColorCssWide(kw)
                        | PropertyValue::BorderRightColorCssWide(kw)
                        | PropertyValue::BorderBottomColorCssWide(kw)
                        | PropertyValue::BorderLeftColorCssWide(kw) => match kw {
                            CssWideKeyword::Inherit => Some(pick_field(parent_border())),
                            CssWideKeyword::Initial | CssWideKeyword::Unset => {
                                Some(pick_field(initial_border()))
                            }
                            CssWideKeyword::Revert | CssWideKeyword::RevertLayer => {
                                Some(pick_field(initial_border()))
                            }
                        },
                        v => Some(v),
                    }
                }
                PropertyValue::BorderTopWidthCssWide(kw)
                | PropertyValue::BorderRightWidthCssWide(kw)
                | PropertyValue::BorderBottomWidthCssWide(kw)
                | PropertyValue::BorderLeftWidthCssWide(kw)
                | PropertyValue::BorderTopStyleCssWide(kw)
                | PropertyValue::BorderRightStyleCssWide(kw)
                | PropertyValue::BorderBottomStyleCssWide(kw)
                | PropertyValue::BorderLeftStyleCssWide(kw)
                | PropertyValue::BorderTopColorCssWide(kw)
                | PropertyValue::BorderRightColorCssWide(kw)
                | PropertyValue::BorderBottomColorCssWide(kw)
                | PropertyValue::BorderLeftColorCssWide(kw) => match kw {
                    CssWideKeyword::Inherit => Some(pick_field(parent_border())),
                    CssWideKeyword::Initial | CssWideKeyword::Unset => {
                        Some(pick_field(initial_border()))
                    }
                    // Unreachable via find_rollback's skip, defensive fallback.
                    CssWideKeyword::Revert | CssWideKeyword::RevertLayer => {
                        Some(pick_field(initial_border()))
                    }
                },
                v => Some(v),
            }
            .or(Some(pick_field(initial_border())))
        }
    }
}

/// Choose the cascade winners for one node and apply them to the staging
/// representation (**phase 1**).
///
/// The caller allocates `winners` outside the walk loop as a scratch buffer.
/// This function pairs filling it ([`pick_winners`]) with draining it, leaving
/// every slot at `None` on return.
///
/// # Application order
///
/// Iterate slots by ascending index, i.e. [`PropertyKey`] **declaration order**.
/// [`Option::take`] restores each slot to `None` during this walk, also resetting
/// the buffer for the next node.
///
/// Shorthand keys must not reach this phase. CSS Cascading Level 4 §3
/// requires shorthand declarations to behave as if expanded in place, and
/// §6.1 determines the winner by order of appearance. Parsed entry points
/// expand shorthands before collecting candidates; the exhaustive expansion
/// match requires an explicit decision for new property variants.
///
/// # The unchecked `winner.idx` position
///
/// `winner.idx` is in bounds because the immediately preceding [`pick_winners`]
/// produced it from **the same `candidates`**. Filling and draining occur next
/// to each other in this function body; no path swaps out `candidates` between
/// them.
///
/// The index is valid only because [`pick_winners`] produced it from the
/// same candidate slice immediately before this drain. A stale slot could
/// otherwise select another declaration, so the winner buffer is reset for
/// every node.
///
/// # Origin of `candidates`
///
/// [`walk_from`] obtains `candidates` through [`ElementInput::element`] and
/// [`ElementInput::pseudo`], whose views pair **exactly this node's
/// candidates** with **exactly this node's own declarations** (private fields
/// prevent other pairings). Its `::first-line` filter,
/// [`super::first_line::cascade_with_first_line`] and the first-letter
/// methods pass filtered copies of such views or views of [`OwnedCandidates`],
/// which resolve to the same declarations. Thus neither `winner.idx` nor a
/// local handle can resolve to **another node's declaration**. The only
/// remaining risk described above is a leaked slot that was not drained.
///
/// [`PropertyKey`]: crate::property::PropertyKey
// The winner application already groups several optional cascade side channels;
// the inherited computed values add one more required input for `inherit`
// resolution without changing that staging boundary.
#[allow(clippy::too_many_arguments)] // winner application writes independent cascade metadata outputs
pub(crate) fn apply_winners(
    candidates: ElementCandidates<'_>,
    winners: &mut Vec<Option<RankedDecl>>,
    specified: &mut SpecifiedValues,
    inherited: &ComputedValues,
    custom_properties: &CustomPropertyEnvironment,
    mut page_value: Option<&mut crate::property::PageValue>,
    mut authored_writing_mode: Option<&mut Option<WritingMode>>,
    mut svg_properties: Option<&mut Vec<SvgStyleProperty>>,
) {
    pick_winners(candidates, winners);
    // The drain below takes every slot, so the white-space winners are read
    // first; their applied values are collected during the drain.
    let white_space_winners = WhiteSpaceWinners::read(winners);
    let mut white_space_applied = WhiteSpaceApplied::default();
    let radius_winners = BorderRadiusWinners::read(winners);
    let initial_radius = specified.border_radius;
    let mut radius_applied = BorderRadiusApplied::default();
    for slot in winners.iter_mut() {
        if let Some(winner) = slot.take() {
            let value = candidates.value(winner.idx);
            let winner_key = candidates.decls()[winner.idx].key();
            let mut literal_inherit = false;
            let mut default_value = |value: PropertyValue| {
                if let PropertyValue::Deferred(marker) = &value {
                    literal_inherit = marker.css_wide_keyword() == Some(CssWideKeyword::Inherit)
                        || (marker.css_wide_keyword() == Some(CssWideKeyword::Unset)
                            && svg_property_is_inherited(winner_key));
                }
                resolve_defaulting_value(value, inherited)
            };
            let value = match value {
                PropertyValue::BorderRadiusInherit => Some(PropertyValue::BorderRadius(
                    inherited_border_radius_value(inherited),
                )),
                PropertyValue::TextDecorationThicknessInherit => {
                    Some(PropertyValue::TextDecorationThickness(
                        inherited_text_decoration_thickness(inherited),
                    ))
                }
                PropertyValue::Deferred(deferred) => {
                    let resolved = resolve_deferred_value(deferred, custom_properties);
                    let resolved = match resolved {
                        Some(PropertyValue::Deferred(marker)) => match marker.css_wide_keyword() {
                            Some(
                                keyword @ (CssWideKeyword::Revert | CssWideKeyword::RevertLayer),
                            ) => {
                                match find_rollback(
                                    candidates,
                                    winner_key,
                                    winner.idx,
                                    keyword,
                                    custom_properties,
                                ) {
                                    Some(PropertyValue::Deferred(fallback)) => {
                                        resolve_deferred_value(&fallback, custom_properties)
                                            .map(&mut default_value)
                                    }
                                    Some(value) => Some(default_value(value)),
                                    None => None,
                                }
                            }
                            _ => Some(default_value(PropertyValue::Deferred(marker))),
                        },
                        value => value,
                    };
                    match resolved {
                        None => None,
                        Some(PropertyValue::BorderRadiusInherit) => Some(
                            PropertyValue::BorderRadius(inherited_border_radius_value(inherited)),
                        ),
                        Some(
                            PropertyValue::BorderTopWidthCssWide(kw)
                            | PropertyValue::BorderRightWidthCssWide(kw)
                            | PropertyValue::BorderBottomWidthCssWide(kw)
                            | PropertyValue::BorderLeftWidthCssWide(kw)
                            | PropertyValue::BorderTopStyleCssWide(kw)
                            | PropertyValue::BorderRightStyleCssWide(kw)
                            | PropertyValue::BorderBottomStyleCssWide(kw)
                            | PropertyValue::BorderLeftStyleCssWide(kw)
                            | PropertyValue::BorderTopColorCssWide(kw)
                            | PropertyValue::BorderRightColorCssWide(kw)
                            | PropertyValue::BorderBottomColorCssWide(kw)
                            | PropertyValue::BorderLeftColorCssWide(kw),
                        ) => resolve_border_css_wide(
                            kw,
                            winner_key,
                            inherited,
                            candidates,
                            winner,
                            custom_properties,
                        ),
                        Some(PropertyValue::TextDecorationThicknessInherit) => {
                            Some(PropertyValue::TextDecorationThickness(
                                inherited_text_decoration_thickness(inherited),
                            ))
                        }
                        Some(v) => Some(v),
                    }
                }
                PropertyValue::BorderTopWidthCssWide(kw)
                | PropertyValue::BorderRightWidthCssWide(kw)
                | PropertyValue::BorderBottomWidthCssWide(kw)
                | PropertyValue::BorderLeftWidthCssWide(kw)
                | PropertyValue::BorderTopStyleCssWide(kw)
                | PropertyValue::BorderRightStyleCssWide(kw)
                | PropertyValue::BorderBottomStyleCssWide(kw)
                | PropertyValue::BorderLeftStyleCssWide(kw)
                | PropertyValue::BorderTopColorCssWide(kw)
                | PropertyValue::BorderRightColorCssWide(kw)
                | PropertyValue::BorderBottomColorCssWide(kw)
                | PropertyValue::BorderLeftColorCssWide(kw) => resolve_border_css_wide(
                    *kw,
                    winner_key,
                    inherited,
                    candidates,
                    winner,
                    custom_properties,
                ),
                _ => Some(value.clone()),
            };
            // Record the precedence even for a value invalid at computed-value
            // time: it still decides which of a physical and a logical size wins.
            let rank = Some((
                winner.rank,
                winner.layer_priority,
                winner.specificity,
                winner.source_order,
                winner.idx,
            ));
            let sizes = &mut specified.preferred_size_precedence;
            match winner_key {
                crate::property::PropertyKey::Width => sizes.width = rank,
                crate::property::PropertyKey::Height => sizes.height = rank,
                crate::property::PropertyKey::InlineSize => sizes.inline_size = rank,
                crate::property::PropertyKey::BlockSize => sizes.block_size = rank,
                _ => {}
            }
            if let Some(properties) = svg_properties.as_deref_mut()
                && svg_property_is_exported(winner_key)
            {
                properties.push(SvgStyleProperty {
                    property: winner_key,
                    inherited: literal_inherit
                        || matches!(&value, Some(PropertyValue::ContextualColor(color))
                            if color.key == crate::property::PropertyKey::Color
                                && svg_color_is_current_color(&color.source))
                        || (value.is_none() && svg_property_is_inherited(winner_key)),
                    expression: value.as_ref().and_then(svg_property_expression),
                });
            }
            if let Some(value) = value {
                if let PropertyValue::WritingMode(mode) = &value
                    && let Some(slot) = authored_writing_mode.as_deref_mut()
                {
                    *slot = Some(*mode);
                }
                if let crate::property::PropertyValue::Page(page) = &value
                    && let Some(page_slot) = page_value.as_deref_mut()
                {
                    *page_slot = page.clone();
                }
                white_space_applied.note(&value);
                radius_applied.note(&value);
                apply_value(value, specified);
            }
        }
    }
    let (collapse, wrap) = white_space_winners.settle(&white_space_applied);
    specified.border_radius = radius_winners.settle(&radius_applied, initial_radius);
    // A half no declaration on this element decides keeps the value
    // `inherit_from` seeded, which is the parent's effective value.
    if let Some(collapse) = collapse {
        specified.effective_white_space_collapse = collapse;
    }
    if let Some(wrap) = wrap {
        specified.effective_text_wrap_mode = wrap;
    }
}

fn svg_color_is_current_color(source: &str) -> bool {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    parser.expect_ident_matching("currentcolor").is_ok() && parser.expect_exhausted().is_ok()
}

fn svg_property_is_exported(key: crate::property::PropertyKey) -> bool {
    use crate::property::PropertyKey::*;
    matches!(
        key,
        Color | FontFamily | FontSize | FontWeight | FontStyle | Visibility | Display | Opacity
    )
}

fn svg_property_is_inherited(key: crate::property::PropertyKey) -> bool {
    use crate::property::PropertyKey::*;
    matches!(
        key,
        Color | FontFamily | FontSize | FontWeight | FontStyle | Visibility
    )
}

fn svg_property_expression(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::FontSize(Length::Em(_) | Length::Percent(_) | Length::Ex(_)) => {
            crate::property::serialize_value(value)
        }
        // SVG 1.1 lengths, as SVG renderers such as usvg parse them, have no
        // `ch` or `ic` unit. Export the em multiple of the fallback these units
        // resolve with (see `Length::Ch` and `Length::Ic`), which stays
        // relative to the font size of each instance.
        PropertyValue::FontSize(Length::Ch(value)) => {
            crate::property::serialize_value(&PropertyValue::FontSize(Length::Em(value * 0.5)))
        }
        PropertyValue::FontSize(Length::Ic(value)) => {
            crate::property::serialize_value(&PropertyValue::FontSize(Length::Em(*value)))
        }
        PropertyValue::FontSizeRelative(RelativeFontSize::Larger) => Some("larger".into()),
        PropertyValue::FontSizeRelative(RelativeFontSize::Smaller) => Some("smaller".into()),
        PropertyValue::FontWeight(FontWeightValue::Bolder) => Some("bolder".into()),
        PropertyValue::FontWeight(FontWeightValue::Lighter) => Some("lighter".into()),
        _ => None,
    }
}

/// Whether `a` comes after `b` in cascade order: rank, specificity and source
/// order first, then the position in the candidate list. Declarations of one
/// rule (and of one inline style) share a source order, so the candidate
/// position is what orders them.
fn declared_later(a: RankedDecl, b: RankedDecl) -> bool {
    (
        a.rank,
        a.layer_priority,
        a.specificity,
        a.source_order,
        a.idx,
    ) > (
        b.rank,
        b.layer_priority,
        b.specificity,
        b.source_order,
        b.idx,
    )
}

/// Winners of the retained radius shorthand and its four corner longhands.
struct BorderRadiusWinners {
    shorthand: Option<RankedDecl>,
    corners: [Option<RankedDecl>; 4],
}

#[derive(Default)]
struct BorderRadiusApplied {
    shorthand: Option<BorderRadius>,
    corners: [Option<CornerRadius<Length>>; 4],
}

impl BorderRadiusApplied {
    fn note(&mut self, value: &PropertyValue) {
        match value {
            PropertyValue::BorderRadius(radius) => self.shorthand = Some(*radius),
            PropertyValue::BorderRadiusTopLeft(corner) => self.corners[0] = Some(*corner),
            PropertyValue::BorderRadiusTopRight(corner) => self.corners[1] = Some(*corner),
            PropertyValue::BorderRadiusBottomRight(corner) => self.corners[2] = Some(*corner),
            PropertyValue::BorderRadiusBottomLeft(corner) => self.corners[3] = Some(*corner),
            _ => {}
        }
    }
}

impl BorderRadiusWinners {
    fn read(winners: &[Option<RankedDecl>]) -> Self {
        use crate::property::PropertyKey as K;
        let at = |key: K| winners.get(key as usize).copied().flatten();
        Self {
            shorthand: at(K::BorderRadius),
            corners: [
                K::BorderRadiusTopLeft,
                K::BorderRadiusTopRight,
                K::BorderRadiusBottomRight,
                K::BorderRadiusBottomLeft,
            ]
            .map(at),
        }
    }

    fn settle(&self, applied: &BorderRadiusApplied, initial: BorderRadius) -> BorderRadius {
        let corners = |radius: BorderRadius| {
            [
                radius.top_left,
                radius.top_right,
                radius.bottom_right,
                radius.bottom_left,
            ]
        };
        let initial = corners(initial);
        let shorthand = applied.shorthand.map(corners).unwrap_or(initial);
        let [top_left, top_right, bottom_right, bottom_left] =
            std::array::from_fn(|index| match (self.corners[index], self.shorthand) {
                (Some(longhand), Some(shorthand_winner))
                    if declared_later(longhand, shorthand_winner) =>
                {
                    applied.corners[index].unwrap_or(initial[index])
                }
                (_, Some(_)) => shorthand[index],
                (Some(_), None) => applied.corners[index].unwrap_or(initial[index]),
                (None, None) => initial[index],
            });
        BorderRadius {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }
}

/// The cascade winners of the legacy `white-space` shorthand and of the
/// longhands it expands to.
///
/// The cascade keeps one winner per property and applies them in property
/// order, so the order in which the shorthand and a longhand were declared is
/// not visible to `apply_value`. CSS Text 4 §3 makes `white-space` a shorthand
/// of `white-space-collapse` and `text-wrap-mode`, so each longhand takes the
/// value of whichever of the two declarations comes later in cascade order.
struct WhiteSpaceWinners {
    legacy: Option<RankedDecl>,
    collapse: Option<RankedDecl>,
    wrap: Option<RankedDecl>,
}

/// The values the white-space winners resolved to (after `var()`
/// substitution), or `None` when a winner was invalid at computed-value time.
#[derive(Default)]
struct WhiteSpaceApplied {
    legacy: Option<WhiteSpace>,
    collapse: Option<WhiteSpaceCollapse>,
    wrap: Option<TextWrapMode>,
}

impl WhiteSpaceApplied {
    fn note(&mut self, value: &PropertyValue) {
        match value {
            PropertyValue::WhiteSpace(keyword) => self.legacy = Some(*keyword),
            PropertyValue::WhiteSpaceCollapse(collapse) => self.collapse = Some(*collapse),
            PropertyValue::TextWrap(wrap) => self.wrap = Some(*wrap),
            PropertyValue::TextWrapShorthand(shorthand) => self.wrap = Some(shorthand.mode),
            _ => {}
        }
    }
}

impl WhiteSpaceWinners {
    fn read(winners: &[Option<RankedDecl>]) -> Self {
        use crate::property::PropertyKey as K;
        let at = |key: K| winners.get(key as usize).copied().flatten();
        Self {
            legacy: at(K::WhiteSpace),
            collapse: at(K::WhiteSpaceCollapse),
            // A `text-wrap` shorthand kept as a `var()` value shares this key.
            wrap: at(K::TextWrap),
        }
    }

    /// The effective `white-space-collapse` and `text-wrap-mode` this
    /// element declares, each `None` when no declaration decides that half
    /// (or the deciding one was invalid at computed-value time, which makes
    /// the half inherit).
    fn settle(
        &self,
        applied: &WhiteSpaceApplied,
    ) -> (Option<WhiteSpaceCollapse>, Option<TextWrapMode>) {
        let legacy_pair = applied.legacy.and_then(WhiteSpace::collapse_and_wrap);
        let collapse = match (self.collapse, self.legacy) {
            (Some(longhand), Some(legacy)) if !declared_later(longhand, legacy) => {
                legacy_pair.map(|pair| pair.0)
            }
            (Some(_), _) => applied.collapse,
            (None, Some(_)) => legacy_pair.map(|pair| pair.0),
            (None, None) => None,
        };
        let wrap = match (self.wrap, self.legacy) {
            (Some(longhand), Some(legacy)) if !declared_later(longhand, legacy) => {
                legacy_pair.map(|pair| pair.1)
            }
            (Some(_), _) => applied.wrap,
            (None, Some(_)) => legacy_pair.map(|pair| pair.1),
            (None, None) => None,
        };
        (collapse, wrap)
    }
}

/// Resolve a specified `font-weight` to a computed absolute weight.
///
/// CSS Fonts 4 §2.2.1 "Relative Weights"
/// <https://www.w3.org/TR/css-fonts-4/#relative-weights> supplies the
/// bolder / lighter table below, reproduced **exactly in six rows**
/// (`inherited` corresponds to the table's `w`):
///
/// | inherited value | `bolder` | `lighter` |
/// |---|---|---|
/// | `w < 100`       | 400 | `w` (no change) |
/// | `100 <= w < 350`| 400 | 100 |
/// | `350 <= w < 550`| 700 | 100 |
/// | `550 <= w < 750`| 900 | 400 |
/// | `750 <= w < 900`| 900 | 700 |
/// | `900 <= w`      | `w` (no change) | 700 |
///
/// # Why arithmetic expressions are incorrect
///
/// Approximations such as `min(w + 300, 900)` / `max(w - 300, 100)` omit the
/// two "no change" rows at the ends of the table. Authors can now specify
/// the full `<number [1,1000]>` range, making those rows reachable:
///
/// - Parent `font-weight: 1000` + child `bolder` → **1000** (no change in the
///   `900 <= w` row). `min(1300, 900)` would incorrectly return 900.
/// - Parent `font-weight: 50` + child `lighter` → **50** (no change in the
///   `w < 100` row). `max(-250, 100)` would incorrectly return 100.
///
/// The boundaries are half-open intervals (e.g. `350 <= w < 550`). The match
/// arms run top to bottom, with **only an upper bound** (`w < N`) in each
/// guard; the previous arm's failure supplies the lower bound. Thus **arm
/// order must match spec row order exactly**. Do not reorder these arms.
///
/// `pub(crate)` permits intra-doc links from other modules; making this
/// private fails the documentation gate (repository rule 3).
///
/// `inherited` and the result are `f32` (formerly `u16`). All table boundaries
/// (100 / 350 / 550 / 750 / 900) are integers, but `inherited` retains
/// fractional weights (e.g. `349.5`). Comparing without rounding selects the
/// precise row prescribed by spec §2.2.1. The old `u16` implementation rounded
/// `349.5` to `350` while parsing and incorrectly selected the `350 <= w < 550`
/// row (details in the `parse_font_weight` docs of [`crate::property`]).
///
/// # Non-finite `inherited` (`NaN` / `±Inf`) — not guarded here
///
/// The `u16` type structurally excluded non-finite values. With `f32`, the
/// caller must validate finiteness rather than relying on the type. The usual
/// cascade path always supplies finite values because `parse_font_weight` in
/// [`crate::property`] checks the `[1, 1000]` range. But every field of
/// `ComputedValues` is `pub`, and [`crate::page::cascade_page`] accepts a
/// caller-provided [`crate::page::PageInheritance`]`::FromRoot` as its inherited
/// root. A direct construction that bypasses the cascade
/// (`ComputedValues { font_weight: f32::NAN, .. }`) can theoretically reach here.
///
/// In both match arms, `<` against NaN is always false. Their different
/// catch-all arms make their behavior asymmetric: `Bolder` falls through to
/// `w => w` (the `900 <= w` no-change row), propagating `NaN` / `+Inf`;
/// `-Inf` matches the first `w < 100.0` guard and resolves to 400.0.
/// `Lighter` falls through to `_ => 700.0`, mapping `NaN` / `+Inf` to **700.0**;
/// `-Inf` matches the first guard and propagates unchanged.
///
/// The resolver leaves non-finite and out-of-range values to the sink
/// boundary, where the value is normalized before it reaches layout.
pub(crate) fn resolve_relative_weight(specified: FontWeightValue, inherited: f32) -> f32 {
    match specified {
        FontWeightValue::Absolute(w) => w,
        FontWeightValue::Bolder => match inherited {
            w if w < 100.0 => 400.0,
            w if w < 350.0 => 400.0,
            w if w < 550.0 => 700.0,
            w if w < 750.0 => 900.0,
            w if w < 900.0 => 900.0,
            // `900 <= w`: no change (return inherited values above 900, such as 1000)
            w => w,
        },
        FontWeightValue::Lighter => match inherited {
            // `w < 100`: no change (return inherited values below 100, such as 50)
            w if w < 100.0 => w,
            w if w < 350.0 => 100.0,
            w if w < 550.0 => 100.0,
            w if w < 750.0 => 400.0,
            w if w < 900.0 => 700.0,
            _ => 700.0,
        },
    }
}

/// Resolve `<relative-size>` (`larger` / `smaller`) for `font-size` against
/// the parent's computed font-size. This is the font-size counterpart of
/// [`resolve_relative_weight`] (same pattern as bolder/lighter).
///
/// CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop> verbatim:
///
/// > A `<relative-size>` keyword is interpreted relative to the computed
/// > font-size of the parent element and possibly the table of font sizes.
/// > \[…\] If the parent element has a keyword font size in the absolute size
/// > keyword mapping table, larger may compute the font size to the next
/// > entry in the table, and smaller may compute the font size to the
/// > previous entry in the table. \[…\] Instead of using next and previous
/// > items in the previous keyword table, User agents may instead use a
/// > simple ratio to increase or decrease the font size relative to the
/// > parent element. The specific ratio is unspecified, but should be around
/// > 1.2–1.5.
///
/// # Why the next/previous table-entry branch is not implemented
///
/// The spec offers both branches with "may" (both are permitted, neither is
/// required). Raikiri implements **only** the simple-ratio branch. The parser
/// (`parse_font_size_keyword`) already resolves `<absolute-size>` keywords to
/// `Length::Px` relative to `medium` at parse time and discards the keyword
/// variant. The inherited computed font-size cannot reveal whether the parent
/// specified a keyword, so we cannot safely check the table branch's premise
/// ("if the parent element has a keyword font size in the ... table").
///
/// # Why ratio = 1.2
///
/// The spec quote's "should be around 1.2–1.5" gives the lower end of the
/// allowed range. **This 1.2 has a different basis from the note in §2.5.1**
/// (which says CSS2's 1.2 scaling factor between adjacent indices was too
/// small at small sizes). Do not cite that note as the basis: it concerns the
/// table branch, while this function uses the simple-ratio branch.
pub(crate) fn resolve_relative_font_size(keyword: RelativeFontSize, inherited_px: f32) -> f32 {
    const RATIO: f32 = 1.2;
    match keyword {
        RelativeFontSize::Larger => inherited_px * RATIO,
        RelativeFontSize::Smaller => inherited_px / RATIO,
    }
}

/// Resolve a specified value against inherited computed values and return a
/// computed-equivalent value **still represented as `PropertyValue`**, wrapped
/// in [`ResolvedAgainstInherited`].
///
/// # Why this is separate from [`apply_value`]
///
/// [`apply_value`] writes results directly into fields of [`SpecifiedValues`];
/// callers needing a `PropertyValue` result cannot reuse it. The result of
/// [`crate::page::cascade_page`] exposes
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// publicly as a `HashMap<PropertyKey, PropertyValue>`, so values must pass
/// through this function before insertion. The CSS Fonts 4 §2.2.1 relative-
/// weight table exists only once in `resolve_relative_weight`; this function
/// and [`apply_value`] both call it rather than duplicating the table.
///
/// # Why there is no wildcard arm (contract)
///
/// The pass-through side lists every variant instead of using `_ => value`.
/// This is an intentional compile-time guard: **if a new property needs
/// inherited-value resolution, a wildcard would silently pass its unresolved
/// value into the public result**.
/// The pass-through side is exhaustive rather than using a wildcard. Adding a
/// property variant therefore requires an explicit decision about whether it
/// depends on inherited values. This compile-time guard does not detect new
/// entry points or new payload semantics inside an existing variant.
///
/// # Pass-through here does not mean "resolved" (phase 3 is still required)
///
/// This function performs only resolutions possible using **inherited computed
/// values alone**: **phase 2**. Values passed through still include specified
/// [`Length`] `Em` / `Rem` / `Pt` in box properties
/// ([`padding`](PropertyValue::PaddingTop) /
/// [`margin`](PropertyValue::MarginTop) / [`width`](PropertyValue::Width) /
/// [`height`](PropertyValue::Height) / `border-*-width`) and `line-height`.
/// CSS Paged Media 3 §6 "Page Properties"
/// <https://www.w3.org/TR/css-page-3/#page-properties> says, "Values in units of
/// em and ex are interpreted relative to the font associated with their
/// context". Thus `Em` uses the page context's own font, whose size **may come
/// from a sibling declaration in the same cascade** and cannot be determined
/// from `inherited` alone. `border-*-width` style gating (CSS Backgrounds 3 §3.3)
/// likewise requires a sibling `border-*-style` declaration.
///
/// **The caller must perform that resolution.** The sole caller,
/// [`crate::page::cascade_page`], immediately runs **phase 3**
/// (`absolutize_in_page_context` in [`crate::page`]) to absolutize against the
/// page context's font-size and apply style gating. Values are therefore
/// computed by the time they enter
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations).
/// **Do not write a new caller that exposes this function's result directly.**
///
/// The element counterpart is [`apply_value`] → [`SpecifiedValues::finalize`]:
/// phase 2 determines font-size, then phase 3 absolutizes remaining values
/// against the element's own font-size. Both phase-3 paths share the same
/// functions in [`crate::resolve`], so each spec rule (`em` / `rem` bases,
/// unchanged percentages, border style gating) has one implementation.
///
/// # This function resolves `TextAlign::MatchParent`
///
/// [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent) can be
/// resolved using `inherited` alone: this covers the actual-parent case of
/// CSS Text 3 §6.1 `#valdef-text-align-match-parent`, not the root element's
/// "computes to start" case (see below). The `TextAlign` arm passes
/// `inherited.text_align` and `inherited.direction` to
/// [`crate::property::resolve_text_align_match_parent`]. Raikiri previously
/// could not implement this because it lacked computed `direction`.
///
/// ⚠️ **Trap**: "The page context inherits from the root element" in CSS Paged
/// Media 3 §6 **does not** mean the page context has no parent. `inherited`
/// always denotes the actual parent (or initial values under the L3 legacy
/// exception). The "Computes to start when specified on the root element"
/// exception quoted in the [`TextAlign`](crate::property::TextAlign) docs is
/// **not handled here**. A page context is not itself the root element, so it
/// never receives that exception. [`SpecifiedValues::finalize_as_root`] handles
/// this exception on the element path.
///
/// The public contract is [`crate::PageCascadeResult::declarations`].
///
/// `Percent` is not "unresolved": the computed value of a box property
/// retains its percentage. CSS Paged Media 3 §6 says "Percentage values on the
/// margin and padding properties are relative to the dimensions of the
/// containing block"; that is an input to the used-value stage (the quote is
/// on the canonical side). This matches the element path's
/// [`crate::resolve::resolve_length_percentage`].
///
/// # Why the result is [`ResolvedAgainstInherited`], not a bare [`PropertyValue`]
///
/// This type narrows the gap described in §2 of "What this guard does not
/// protect" above (a new entry point that does not call this function).
/// See the [`ResolvedAgainstInherited`] docs for details.
///
/// # Caller contract for `ctx`
///
/// Pass `ctx.root_line_height` derived from `inherited` via
/// (`used_line_height_length(inherited.line_height, inherited.font_size)`).
/// The `FontSize` arm's `lh`/`rlh` resolution (see below) requires that match.
/// The sole caller, [`crate::page::cascade_page`], computes the value once and
/// reuses it here and in phase 3 (`absolutize_in_page_context` in
/// [`crate::page`]). Because `inherited` is unchanged throughout the function,
/// computing it twice would yield the same result (see the caller's docs).
pub(crate) fn inherited_border_radius(
    value: crate::property::CornerRadius<ComputedLengthPercentage>,
) -> crate::property::CornerRadius<Length> {
    value.map(|axis| match axis {
        ComputedLengthPercentage::Px(px) => Length::Px(px),
        ComputedLengthPercentage::Percent(percent) => Length::Percent(percent),
    })
}

/// Resolve one border width longhand CSS-wide marker for the page path
/// ([`resolve_against_inherited`]), which has no rollback candidates.
///
/// `is_top` / `is_right` / `is_bottom` select the side (all false = left).
/// `Inherit` lifts the parent computed side's gated width (see
/// [`inherited_border_width`]); `Initial` / `Unset` use [`INITIAL_BORDER`];
/// `Revert` / `RevertLayer` fall back to [`INITIAL_BORDER`] here because this
/// function has no candidate list — the page winner-selection in
/// [`crate::page::cascade_page`] replaces revert winners with their rollback
/// before calling this function, so reaching this arm via the normal page path
/// means no lower-origin winner existed.
fn resolve_border_page_longhand(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> Length {
    let side = if is_top {
        &inherited.border.top
    } else if is_right {
        &inherited.border.right
    } else if is_bottom {
        &inherited.border.bottom
    } else {
        &inherited.border.left
    };
    match kw {
        CssWideKeyword::Inherit => inherited_border_width(side.width()),
        CssWideKeyword::Initial | CssWideKeyword::Unset => INITIAL_BORDER.width,
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => INITIAL_BORDER.width,
    }
}

/// Page-path companion of [`resolve_border_page_longhand`] for `border-*-style`.
fn resolve_border_page_style(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> BorderStyle {
    let side = if is_top {
        &inherited.border.top
    } else if is_right {
        &inherited.border.right
    } else if is_bottom {
        &inherited.border.bottom
    } else {
        &inherited.border.left
    };
    match kw {
        CssWideKeyword::Inherit => side.style(),
        CssWideKeyword::Initial | CssWideKeyword::Unset => INITIAL_BORDER.style,
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => INITIAL_BORDER.style,
    }
}

/// Page-path companion for `border-*-color`.
fn resolve_border_page_color(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> BorderColor {
    let side = if is_top {
        &inherited.border.top
    } else if is_right {
        &inherited.border.right
    } else if is_bottom {
        &inherited.border.bottom
    } else {
        &inherited.border.left
    };
    match kw {
        CssWideKeyword::Inherit => side.color,
        CssWideKeyword::Initial | CssWideKeyword::Unset => INITIAL_BORDER.color,
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => INITIAL_BORDER.color,
    }
}

/// Page-path single-side resolver for `border-right: <css-wide>` (and the per-side
/// helper for [`resolve_border_page_sides`]).
fn resolve_border_page_side(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> Border {
    Border {
        width: resolve_border_page_longhand(kw, inherited, is_top, is_right, is_bottom),
        style: resolve_border_page_style(kw, inherited, is_top, is_right, is_bottom),
        color: resolve_border_page_color(kw, inherited, is_top, is_right, is_bottom),
    }
}

/// Page-path resolver for `border: <css-wide>` — all four sides.
fn resolve_border_page_sides(kw: CssWideKeyword, inherited: &ComputedValues) -> Sides<Border> {
    Sides {
        top: resolve_border_page_side(kw, inherited, true, false, false),
        right: resolve_border_page_side(kw, inherited, false, true, false),
        bottom: resolve_border_page_side(kw, inherited, false, false, true),
        left: resolve_border_page_side(kw, inherited, false, false, false),
    }
}

fn inherited_border_radius_value(inherited: &ComputedValues) -> BorderRadius {
    BorderRadius {
        top_left: inherited_border_radius(inherited.border_radius.top_left),
        top_right: inherited_border_radius(inherited.border_radius.top_right),
        bottom_right: inherited_border_radius(inherited.border_radius.bottom_right),
        bottom_left: inherited_border_radius(inherited.border_radius.bottom_left),
    }
}

/// Lift a parent computed border width into specified form for `inherit`.
///
/// CSS Cascading 4 §7.3 takes the parent's computed value. For `border-*-width`
/// the computed value is already gated to zero when the parent's style is `none`
/// or `hidden` (see [`crate::resolve::resolve_border`]); inheriting that gated
/// zero is spec-correct. `ComputedLength` absolutizes to px (see [`crate::resolve`]),
/// so representing it as [`Length::Px`] is lossless.
fn inherited_border_width(computed: ComputedLength) -> Length {
    Length::Px(computed.0)
}

/// Lift a parent computed `text-decoration-thickness` into specified form for `inherit`.
///
/// CSS Cascading 4 section 7.3 takes the parent computed value even though
/// CSS Text Decoration 4 marks this longhand non-inherited. The computed
/// length is already absolute CSS px (see [`crate::resolve::resolve_text_decoration_thickness`]),
/// so representing it as `Length::Px` is lossless. Keywords pass through unchanged.
fn inherited_text_decoration_thickness(
    inherited: &ComputedValues,
) -> crate::property::TextDecorationThickness {
    match inherited.text_decoration_thickness {
        crate::resolve::ComputedTextDecorationThickness::Auto => {
            crate::property::TextDecorationThickness::Auto
        }
        crate::resolve::ComputedTextDecorationThickness::FromFont => {
            crate::property::TextDecorationThickness::FromFont
        }
        crate::resolve::ComputedTextDecorationThickness::Length(px) => {
            crate::property::TextDecorationThickness::Length(Length::Px(px.0))
        }
    }
}

fn inherited_margin_length(value: ComputedLengthPercentageOrAuto) -> LengthOrAuto {
    match value {
        ComputedLengthPercentageOrAuto::Px(px) => LengthOrAuto::Length(Length::Px(px)),
        ComputedLengthPercentageOrAuto::Percent(percent) => {
            LengthOrAuto::Length(Length::Percent(percent))
        }
        ComputedLengthPercentageOrAuto::Calc(calc) => LengthOrAuto::Calc(calc),
        ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
        // Margins never compute to `min-content`; fall back to the initial 0.
        ComputedLengthPercentageOrAuto::MinContent => LengthOrAuto::Length(Length::Px(0.0)), // cov:ignore: only the width / inline-size parser produces min-content
    }
}

/// Resolve defaulting markers shared by the element and page inheritance paths.
fn resolve_defaulting_value(value: PropertyValue, inherited: &ComputedValues) -> PropertyValue {
    if let PropertyValue::Deferred(marker) = &value
        && let Some(keyword) = marker.css_wide_keyword()
    {
        let radius = if keyword == CssWideKeyword::Inherit {
            inherited_border_radius_value(inherited)
        } else {
            BorderRadius::elliptical([Length::Px(0.0); 4], [Length::Px(0.0); 4])
        };
        let initial = keyword == CssWideKeyword::Initial;
        let inherit_svg = keyword == CssWideKeyword::Inherit
            || (keyword != CssWideKeyword::Initial && svg_property_is_inherited(marker.key));
        match marker.key {
            crate::property::PropertyKey::Opacity => {
                return PropertyValue::Opacity(if inherit_svg { inherited.opacity } else { 1.0 });
            }
            crate::property::PropertyKey::Display => {
                return PropertyValue::Display(if inherit_svg {
                    inherited.display
                } else {
                    crate::property::DisplayValue::Inline
                });
            }
            crate::property::PropertyKey::Visibility => {
                return PropertyValue::Visibility(if inherit_svg {
                    inherited.visibility
                } else {
                    crate::property::Visibility::Visible
                });
            }
            crate::property::PropertyKey::FontFamily => {
                return PropertyValue::FontFamily(if inherit_svg {
                    inherited.font_family.clone()
                } else {
                    crate::property::initial_font_family()
                });
            }
            crate::property::PropertyKey::FontWeight => {
                return PropertyValue::FontWeight(FontWeightValue::Absolute(if inherit_svg {
                    inherited.font_weight
                } else {
                    400.0
                }));
            }
            crate::property::PropertyKey::FontStyle => {
                return PropertyValue::FontStyle(if inherit_svg {
                    inherited.font_style
                } else {
                    crate::property::FontStyle::Normal
                });
            }
            crate::property::PropertyKey::VerticalAlign => {
                return PropertyValue::VerticalAlign(if keyword == CssWideKeyword::Inherit {
                    inherited.vertical_align
                } else {
                    crate::property::VerticalAlign::Baseline
                });
            }
            crate::property::PropertyKey::BorderRadiusTopLeft => {
                return PropertyValue::BorderRadiusTopLeft(radius.top_left);
            }
            crate::property::PropertyKey::BorderRadiusTopRight => {
                return PropertyValue::BorderRadiusTopRight(radius.top_right);
            }
            crate::property::PropertyKey::BorderRadiusBottomRight => {
                return PropertyValue::BorderRadiusBottomRight(radius.bottom_right);
            }
            crate::property::PropertyKey::BorderRadiusBottomLeft => {
                return PropertyValue::BorderRadiusBottomLeft(radius.bottom_left);
            }
            crate::property::PropertyKey::ColumnRuleWidth => {
                return PropertyValue::ColumnRuleWidth(if keyword == CssWideKeyword::Inherit {
                    Length::Px(inherited.column_rule.width().px())
                } else {
                    INITIAL_BORDER.width
                });
            }
            crate::property::PropertyKey::ColumnRuleStyle => {
                return PropertyValue::ColumnRuleStyle(if keyword == CssWideKeyword::Inherit {
                    inherited.column_rule.style()
                } else {
                    INITIAL_BORDER.style
                });
            }
            crate::property::PropertyKey::ColumnSpan => {
                return PropertyValue::ColumnSpan(if keyword == CssWideKeyword::Inherit {
                    inherited.column_span
                } else {
                    crate::property::ColumnSpanValue::None
                });
            }
            crate::property::PropertyKey::ColumnRuleColor => {
                return PropertyValue::ColumnRuleColor(if keyword == CssWideKeyword::Inherit {
                    inherited.column_rule.color
                } else {
                    INITIAL_BORDER.color
                });
            }
            crate::property::PropertyKey::ListStyleType => {
                return PropertyValue::ListStyleType(if initial {
                    Default::default()
                } else {
                    inherited.list_style_type.clone()
                });
            }
            crate::property::PropertyKey::ListStylePosition => {
                return PropertyValue::ListStylePosition(if initial {
                    Default::default()
                } else {
                    inherited.list_style_position
                });
            }
            crate::property::PropertyKey::ListStyleImage => {
                return PropertyValue::ListStyleImage(if initial {
                    crate::property::BackgroundImage::None
                } else {
                    inherited.list_style_image.clone()
                });
            }
            _ => {}
        }
    }
    resolve_css_wide_color_font(
        value,
        inherited.color,
        inherited.background_color,
        inherited.background_color_expression.as_ref(),
        inherited.font_size,
    )
}

/// Share color/background/font defaulting with the page-margin cascade.
pub(crate) fn resolve_css_wide_color_font(
    value: PropertyValue,
    inherited_color: crate::property::CssColor,
    inherited_background: crate::property::CssColor,
    inherited_background_expression: Option<&smol_str::SmolStr>,
    inherited_font_size: ComputedLength,
) -> PropertyValue {
    let PropertyValue::Deferred(marker) = &value else {
        return value;
    };
    if !matches!(
        marker.key,
        crate::property::PropertyKey::Color
            | crate::property::PropertyKey::BackgroundColor
            | crate::property::PropertyKey::FontSize
    ) {
        return value;
    }
    let Some(keyword) = marker.css_wide_keyword() else {
        return value;
    };
    if marker.key == crate::property::PropertyKey::BackgroundColor
        && keyword == CssWideKeyword::Inherit
        && let Some(source) = inherited_background_expression
    {
        return PropertyValue::ContextualColor(crate::property::ContextualColor {
            source: source.clone(),
            key: marker.key,
        });
    }
    let inherit = keyword == CssWideKeyword::Inherit
        || (keyword != CssWideKeyword::Initial
            && marker.key != crate::property::PropertyKey::BackgroundColor);
    if marker.key == crate::property::PropertyKey::Color {
        PropertyValue::Color(if inherit {
            inherited_color
        } else {
            crate::property::CssColor::BLACK
        })
    } else if marker.key == crate::property::PropertyKey::BackgroundColor {
        PropertyValue::BackgroundColor(if inherit {
            inherited_background
        } else {
            crate::property::CssColor::TRANSPARENT
        })
    } else if marker.key == crate::property::PropertyKey::FontSize {
        PropertyValue::FontSize(Length::Px(if inherit {
            inherited_font_size.0
        } else {
            crate::computed::INITIAL_FONT_SIZE_PX
        }))
    } else {
        value
    }
}

pub(crate) fn resolve_against_inherited(
    value: PropertyValue,
    inherited: &ComputedValues,
    ctx: &ResolveContext,
) -> ResolvedAgainstInherited {
    ResolvedAgainstInherited(match resolve_defaulting_value(value, inherited) {
        PropertyValue::BorderRadiusInherit => {
            PropertyValue::BorderRadius(inherited_border_radius_value(inherited))
        },
        PropertyValue::MarginTopInherit => {
            PropertyValue::MarginTop(inherited_margin_length(inherited.margin.top))
        }
        PropertyValue::MarginRightInherit => {
            PropertyValue::MarginRight(inherited_margin_length(inherited.margin.right))
        }
        PropertyValue::MarginBottomInherit => {
            PropertyValue::MarginBottom(inherited_margin_length(inherited.margin.bottom))
        }
        PropertyValue::MarginLeftInherit => {
            PropertyValue::MarginLeft(inherited_margin_length(inherited.margin.left))
        }
        PropertyValue::MarginInherit => PropertyValue::Margin(Sides {
            top: inherited_margin_length(inherited.margin.top),
            right: inherited_margin_length(inherited.margin.right),
            bottom: inherited_margin_length(inherited.margin.bottom),
            left: inherited_margin_length(inherited.margin.left),
        }),
        PropertyValue::TextDecorationThicknessInherit => {
            PropertyValue::TextDecorationThickness(inherited_text_decoration_thickness(inherited))
        }
        PropertyValue::BorderTopWidthCssWide(kw) => {
            PropertyValue::BorderTopWidth(resolve_border_page_longhand(kw, inherited, true, false, false))
        }
        PropertyValue::BorderRightWidthCssWide(kw) => {
            PropertyValue::BorderRightWidth(resolve_border_page_longhand(kw, inherited, false, true, false))
        }
        PropertyValue::BorderBottomWidthCssWide(kw) => {
            PropertyValue::BorderBottomWidth(resolve_border_page_longhand(kw, inherited, false, false, true))
        }
        PropertyValue::BorderLeftWidthCssWide(kw) => {
            PropertyValue::BorderLeftWidth(resolve_border_page_longhand(kw, inherited, false, false, false))
        }
        PropertyValue::BorderTopStyleCssWide(kw) => {
            PropertyValue::BorderTopStyle(resolve_border_page_style(kw, inherited, true, false, false))
        }
        PropertyValue::BorderRightStyleCssWide(kw) => {
            PropertyValue::BorderRightStyle(resolve_border_page_style(kw, inherited, false, true, false))
        }
        PropertyValue::BorderBottomStyleCssWide(kw) => {
            PropertyValue::BorderBottomStyle(resolve_border_page_style(kw, inherited, false, false, true))
        }
        PropertyValue::BorderLeftStyleCssWide(kw) => {
            PropertyValue::BorderLeftStyle(resolve_border_page_style(kw, inherited, false, false, false))
        }
        PropertyValue::BorderTopColorCssWide(kw) => {
            PropertyValue::BorderTopColor(resolve_border_page_color(kw, inherited, true, false, false))
        }
        PropertyValue::BorderRightColorCssWide(kw) => {
            PropertyValue::BorderRightColor(resolve_border_page_color(kw, inherited, false, true, false))
        }
        PropertyValue::BorderBottomColorCssWide(kw) => {
            PropertyValue::BorderBottomColor(resolve_border_page_color(kw, inherited, false, false, true))
        }
        PropertyValue::BorderLeftColorCssWide(kw) => {
            PropertyValue::BorderLeftColor(resolve_border_page_color(kw, inherited, false, false, false))
        }
        PropertyValue::BorderCssWide(kw) => {
            PropertyValue::Border(resolve_border_page_sides(kw, inherited))
        }
        PropertyValue::BorderTopCssWide(kw) => {
            PropertyValue::BorderTop(resolve_border_page_side(kw, inherited, true, false, false))
        }
        PropertyValue::BorderRightCssWide(kw) => {
            PropertyValue::BorderRight(resolve_border_page_side(kw, inherited, false, true, false))
        }
        PropertyValue::BorderBottomCssWide(kw) => {
            PropertyValue::BorderBottom(resolve_border_page_side(kw, inherited, false, false, true))
        }
        PropertyValue::BorderLeftCssWide(kw) => {
            PropertyValue::BorderLeft(resolve_border_page_side(kw, inherited, false, false, false))
        }
        // CSS Fonts 4 §2.2.1 "Relative Weights"
        // <https://www.w3.org/TR/css-fonts-4/#relative-weights>: resolve `bolder` /
        // `lighter` against the inherited computed weight. Convert to
        // `Absolute` here so no relative keyword remains in the result.
        // The `Absolute(f32)` → `f32` → `Absolute(f32)` round-trip loses nothing.
        PropertyValue::FontWeight(fw) => PropertyValue::FontWeight(FontWeightValue::Absolute(
            resolve_relative_weight(fw, inherited.font_weight),
        )),
        // `font-size` needs only the inherited computed font-size. Delegate to
        // `crate::resolve::resolve_font_size` and wrap the result in `Length::Px`
        // to make it computed-equivalent (as the `FontWeight` arm returns
        // `Absolute(f32)`).
        //
        // Why `inherited.font_size` is the right basis:
        //
        // - `em`: CSS Paged Media 3 §6 "Page Properties"
        //   <https://www.w3.org/TR/css-page-3/#page-properties> verbatim —
        //   "When used on the font-size property in the page context, they are
        //   relative to the font-size of the root element."
        //   `cascade_page` receives the root element's `ComputedValues` as
        //   `inherited` (or initial values for
        //   `PageInheritance::LegacyInitialValues`). This is the §6 basis.
        // - `rem`: CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>
        //   "Equal to the computed value of the em unit on the root element." —
        //   again, `inherited.font_size`.
        // - `%`: **§6 does not specify `%` for page-context `font-size`.**
        //   CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop>
        //   says "Percentages: refer to parent element's font size"; together
        //   with §6's "The page context inherits from the root element.", this
        //   implies the root element's font-size. This is an **inference**, not
        //   an explicit statement in §6.
        // - `px` / `pt`: absolute units, independent of context.
        // - `lh` / `rlh`: §6 does not specify `lh`/`rlh` either. By the same
        //   **inference** as for `%`, "the page context inherits from the root
        //   element" plus CSS Values 4 §6.1.1's self-reference clause makes
        //   the root element's used line-height the page context's `lh`
        //   self-reference basis. For this page context, "root" and "parent"
        //   are the same node (= `inherited`), so `rlh` has the same basis.
        //   The docs of `crate::page::page_context_line_height_basis` make
        //   this argument for `line-height` itself and note that the two bases
        //   coincide **only for this page context**. This lets us pass
        //   `ctx.root_line_height` as the `lh` self-reference basis too (as
        //   guaranteed by the caller contract for `ctx` above).
        //
        // This arm handles only `font-size`: `em` in box properties (`padding` /
        // `margin` / `width` / `height` / `border-*-width`) needs the page
        // context's own font-size, which may come from a sibling declaration
        // in this cascade and is not determined by `inherited` alone. The
        // caller (`cascade_page`) handles these in phase 3 after this function
        // (see "Pass-through here does not mean resolved" above).
        // **Always returning `Length::Px` here is essential**: phase 3 reads
        // it back as the page context's font-size (the `em` basis) via
        // `crate::page::page_context_font_size`. The next `FontSizeRelative`
        // arm also produces `PropertyValue::FontSize(Length::Px(_))`.
        PropertyValue::FontSize(len) => PropertyValue::FontSize(Length::Px(
            crate::resolve::resolve_font_size(len, inherited.font_size, ctx.root_line_height, ctx)
                .px(),
        )),
        // CSS Text 3 §6.1 `#valdef-text-align-match-parent`.
        // `inherited` is the page context's inheritance parent (the root
        // element, or initial values under the L3 legacy exception). Resolve
        // it as an **actual parent**; the page context is not itself the root
        // element and never receives its "computes to start" exception (see
        // the trap in the docs above). Other keywords are no-ops (see docs).
        PropertyValue::TextAlign(t) => PropertyValue::TextAlign(
            resolve_text_align_internal_center(
                resolve_text_align_match_parent(t, inherited.text_align, inherited.direction),
                inherited.text_align,
            ),
        ),
        // CSS Fonts 4 §2.5 `<relative-size>` (`larger` / `smaller`): like
        // `bolder` / `lighter`, resolve against the inherited computed
        // font-size. Convert to the `FontSize` variant. As the "resolution
        // timing" section of the `PropertyValue::FontSizeRelative` docs says,
        // this variant exists only temporarily for cascade winners, never in
        // the public result (`crate::page::PageCascadeResult::declarations`).
        PropertyValue::FontSizeRelative(rel) => PropertyValue::FontSize(Length::Px(
            resolve_relative_font_size(rel, inherited.font_size.px()),
        )),
        // Pass through properties not resolved here. The "Pass-through here
        // does not mean resolved (phase 3 is still required)" section above
        // explains how the caller's phase 3 absolutizes them. Do not use `_`.
        //
        // `Direction` belongs here: computed value = specified value (no
        // relative resolution; see `crate::property::Direction` docs), just
        // like `Color` / `FontFamily`.
        v @ (PropertyValue::Color(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::LineHeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::ListStyleType(_)
        | PropertyValue::ListStyle(_)
        | PropertyValue::ListStyleImage(_)
        | PropertyValue::ListStylePosition(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterResetInherit
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::HangingPunctuation(_)
        | PropertyValue::TextAutospace(_)
        | PropertyValue::WordSpaceTransform(_)
        | PropertyValue::TextSpacingTrim(_)
        | PropertyValue::Direction(_)
        | PropertyValue::TextIndent(_)
        | PropertyValue::PaddingTop(_)
        | PropertyValue::PaddingRight(_)
        | PropertyValue::PaddingBottom(_)
        | PropertyValue::PaddingLeft(_)
        | PropertyValue::Padding(_)
        // `padding-inline`/`padding-block` shorthand — same "nothing for
        // phase 2 to resolve" shape as `Padding`/`Margin` above (the
        // `<length-percentage>` font-relative absolutization happens later,
        // in phase 3).
        | PropertyValue::PaddingInline(_)
        | PropertyValue::PaddingBlock(_)
        | PropertyValue::MarginTop(_)
        | PropertyValue::MarginRight(_)
        | PropertyValue::MarginBottom(_)
        | PropertyValue::MarginLeft(_)
        | PropertyValue::Margin(_)
        | PropertyValue::MarginInline(_)
        | PropertyValue::MarginBlock(_)
        | PropertyValue::BorderTopWidth(_)
        | PropertyValue::BorderRightWidth(_)
        | PropertyValue::BorderBottomWidth(_)
        | PropertyValue::BorderLeftWidth(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::Border(_)
        | PropertyValue::BorderTop(_)
        | PropertyValue::BorderRight(_)
        | PropertyValue::BorderBottom(_)
        | PropertyValue::BorderLeft(_)
        // `border-style` / `border-width` / `border-color` shorthands —
        // same "nothing for phase 2 to resolve" shape as `Border` above
        // (non-inherited keywords/lengths; structurally unreachable here
        // since rule.rs expands them first).
        | PropertyValue::BorderStyle(_)
        | PropertyValue::BorderWidth(_)
        | PropertyValue::BorderColor(_)
        | PropertyValue::Width(_)
        | PropertyValue::Height(_)
        | PropertyValue::MaxWidth(_)
        | PropertyValue::MaxHeight(_)
        | PropertyValue::MinWidth(_)
        | PropertyValue::MinHeight(_)
        | PropertyValue::MinBlockSize(_)
        | PropertyValue::InlineSize(_)
        | PropertyValue::BlockSize(_)
        | PropertyValue::Top(_)
        | PropertyValue::Right(_)
        | PropertyValue::Bottom(_)
        | PropertyValue::Left(_)
        | PropertyValue::BoxSizing(_)
        // `overflow-x`/`overflow-y`/`overflow` join this arm — CSS Overflow 3
        // §3.1's cross-axis coupling (`resolve_overflow`) depends only on the
        // *other axis of the same node*, never on the inheritance parent, so
        // there is nothing for this function (phase 2) to resolve. It is
        // applied in phase 3 instead (`crate::page::absolutize_in_page_context`,
        // mirroring the element path's `SpecifiedValues::absolutize_with`).
        | PropertyValue::OverflowX(_)
        | PropertyValue::OverflowY(_)
        | PropertyValue::Overflow(_)
        // `writing-mode` joins this arm for the same reason `overflow-x`/
        // `overflow-y`/`overflow` do — the renderer-facing `HorizontalTb`
        // fallback (`resolve_writing_mode`, CSS Writing Modes 4 §3.2) depends
        // only on its own specified value, never on the inheritance parent.
        // The raw CSSOM computed keyword is retained separately in the element
        // computed values; this page/layout path still applies the fallback in
        // phase 3 (`crate::page::absolutize_in_page_context`).
        | PropertyValue::WritingMode(_)
        | PropertyValue::RubyPosition(_)
        // `text-decoration-line`/`-style`/`-color` (and the `text-decoration`
        // shorthand, structurally unreachable here per
        // `crate::rule::expand_shorthand_into`) carry no length. The inherited
        // `text-decoration-skip-ink`/`-skip-spaces` keywords also need no
        // phase-2 resolution because none of these values has a relative part.
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextEmphasisStyle(_)
        | PropertyValue::TextEmphasisColor(_)
        | PropertyValue::TextEmphasis(_)
        | PropertyValue::TextUnderlinePosition(_)
        | PropertyValue::TextDecoration(_)
        // `text-decoration-thickness` / `text-decoration-inset` carry a
        // `<length-percentage>` that needs the *declaring node's own*
        // font-size (phase 3), not the inheritance parent's — same shape
        // as `Padding`/`Margin`/`Width` above, nothing for phase 2 to
        // resolve here. (`TextDecoration` shorthand itself joins the group
        // above since expansion removes it before this function runs.)
        | PropertyValue::TextDecorationThickness(_)
        | PropertyValue::TextDecorationInset(_)
        | PropertyValue::TextUnderlineOffset(_)
        // `vertical-align`'s 6 keywords (`baseline`/`sub`/`super`/`middle`/
        // `text-top`/`text-bottom`) describe a shift *relative to the
        // parent's font metrics*, but that relation is a used-value/layout
        // concern (raikiri-paint scope, `VerticalAlign` doc's "baseline
        // shift calculation is raikiri-paint's scope" note) — CSS 2.1 §10.8.1's
        // computed value for these keywords is still the bare specified
        // keyword, so there is nothing for this function (phase 2,
        // inheritance-parent-relative resolution) to resolve. The
        // `<length>` variant needs *own-node* font-size resolution (`em`/
        // `rem`), which is phase 3's job
        // (`crate::specified::SpecifiedValues::absolutize_with`'s
        // `resolve_vertical_align` call, same split `FlexBasis` below
        // uses) — not this function's, since it depends on the declaring
        // node's own winners rather than the inheritance parent.
        | PropertyValue::VerticalAlign(_)
        // `font-style` carries no length at this crate's scope
        // (`normal`/`italic`/`oblique` implemented, `oblique`'s `<angle>`
        // argument is not — `FontStyle` doc) and does not depend on the
        // inheritance parent — nothing for phase 2 to resolve.
        | PropertyValue::FontStyle(_)
        | PropertyValue::FontKerning(_)
        | PropertyValue::FontOpticalSizing(_)
        | PropertyValue::FontVariantEmoji(_)
        | PropertyValue::FontLanguageOverride(_)
        | PropertyValue::FontVariantLigatures(_)
        | PropertyValue::FontSynthesis(_)
        | PropertyValue::FontVariantPosition(_)
        | PropertyValue::FontPalette(_)
        | PropertyValue::FontVariantNumeric(_)
        | PropertyValue::FontVariantEastAsian(_)
        | PropertyValue::FontVariationSettings(_)
        | PropertyValue::FontFeatureSettings(_)
        // `text-transform` carries no length (`TextTransform` doc) and
        // does not depend on the inheritance parent — nothing for phase 2
        // to resolve.
        | PropertyValue::TextTransform(_)
        // `visibility` carries no length (`Visibility` doc) and does not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::Visibility(_)
        // `z-index` carries no length (`ZIndexValue` doc) and does not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::ZIndex(_)
        // `word-break` (CSS Text 3 §5.1) carries no length (`WordBreak`
        // doc) and does not depend on the inheritance parent — nothing for
        // phase 2 to resolve.
        | PropertyValue::WordBreak(_)
        // `overflow-wrap`/`word-wrap` (CSS Text 3 §5.4) carries no length
        // (`OverflowWrap` doc) either — same as `WordBreak` above.
        | PropertyValue::OverflowWrap(_)
        // `letter-spacing` / `word-spacing` carry a `<length>` that needs
        // the *declaring node's own* font-size (phase 3), not the
        // inheritance parent's — same shape as `Padding`/`Margin`/`Width`/
        // `Height` above, nothing for phase 2 to resolve here.
        | PropertyValue::LetterSpacing(_)
        | PropertyValue::WordSpacing(_)
        // `tab-size`'s `<length>` alternative needs the same *declaring
        // node's own* font-size basis (phase 3) as `LetterSpacing`/
        // `WordSpacing` above; its `<number>` alternative carries no length
        // at all. Either way, nothing for phase 2 to resolve here.
        | PropertyValue::TabSize(_)
        // `break-before`/`break-after` (CSS Fragmentation Module Level 3
        // §3.1, legacy shorthand `page-break-before`/`page-break-after`
        // included) carry no length (`BreakBetween` doc) and do not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::BreakBefore(_)
        | PropertyValue::BreakAfter(_)
        // `break-inside` (CSS Fragmentation Module Level 3 §3.2, legacy
        // shorthand `page-break-inside` included) carries no length
        // either (`BreakInside` doc) — same as `BreakBefore`/`BreakAfter`
        // above.
        | PropertyValue::BreakInside(_)
        // `float` (CSS2 §9.5.1) carries no length and does not depend on
        // the inheritance parent — nothing for phase 2 to resolve. The
        // §9.7 `display` coupling this value drives is a same-node
        // dependency, resolved in phase 3
        // (`crate::specified::SpecifiedValues::absolutize_with`'s
        // `resolve_display_for_float` call), not here.
        | PropertyValue::Float(_)
        // `clear` (CSS2 §9.5.2) carries no length either — same as
        // `Float` above.
        | PropertyValue::Clear(_)
        // `white-space` (CSS Text 3 §3) carries no length (`WhiteSpace`
        // doc) and does not depend on the inheritance parent — nothing for
        // phase 2 to resolve.
        | PropertyValue::WhiteSpace(_)
        // `white-space-collapse` is a keyword-only value; nothing for phase 2
        // to resolve.
        | PropertyValue::WhiteSpaceCollapse(_)
        // `text-wrap` and `text-wrap-style` carry keyword values only; nothing
        // for phase 2 to resolve.
        | PropertyValue::TextWrap(_)
        | PropertyValue::TextWrapStyle(_)
        | PropertyValue::TextWrapShorthand(_)
        | PropertyValue::TextSpacingShorthand(_)
        // `flex-*` / alignment / `row-gap`/`column-gap` (and their
        // shorthands) — same "nothing for phase 2 to resolve" shape as
        // `Padding`/`Margin`/`Width`/`Height` above for the length-bearing
        // ones (`FlexBasis`/`RowGap`/`ColumnGap`/`Flex`/`Gap`; phase 3
        // (`absolutize_in_page_context`) does the declaring node's own
        // font-size resolution), and no length payload at all for the rest
        // (`FlexDirection`/`FlexWrap`/`FlexGrow`/`FlexShrink`/
        // `JustifyContent`/`AlignContent`/`AlignItems`/`AlignSelf`/
        // `PlaceContent`).
        | PropertyValue::FlexDirection(_)
        | PropertyValue::FlexWrap(_)
        | PropertyValue::FlexGrow(_)
        | PropertyValue::FlexShrink(_)
        | PropertyValue::FlexBasis(_)
        | PropertyValue::Flex(_)
        | PropertyValue::FlexFlow(_)
        | PropertyValue::Order(_)
        | PropertyValue::JustifyContent(_)
        | PropertyValue::AlignContent(_)
        | PropertyValue::AlignItems(_)
        | PropertyValue::AlignSelf(_)
        | PropertyValue::RowGap(_)
        | PropertyValue::ColumnGap(_)
        | PropertyValue::Gap(_)
        | PropertyValue::PlaceContent(_)
        // `hyphens` (CSS Text 3 §5.3) carries no length (`Hyphens` doc) and
        // does not depend on the inheritance parent — nothing for phase 2
        // to resolve.
        | PropertyValue::Hyphens(_)
        // `hyphenate-character` carries only `auto` or a decoded string; it
        // has no relative length for phase 2 to resolve.
        | PropertyValue::HyphenateCharacter(_)
        // `hyphenate-limit-chars` is a three-component integer/`auto` value;
        // no relative length or inheritance-parent resolution is needed.
        | PropertyValue::HyphenateLimitChars(_)
        // `font-variant-caps` (CSS Fonts Module Level 3 §6.6) carries no
        // length (`FontVariantCaps` doc) and does not depend on the
        // inheritance parent — nothing for phase 2 to resolve, same shape
        // as `FontStyle` above.
        | PropertyValue::FontVariantCaps(_)
        // `quotes` (CSS Content 3 §2.4.1) carries no length and does not
        // depend on the inheritance parent (computed value = specified
        // value, `ComputedValues::quotes` doc) — nothing for phase 2 to
        // resolve.
        | PropertyValue::Quotes(_)
        // `text-shadow`'s per-item lengths (offset-x/offset-y/blur-radius)
        // need the *declaring node's own* font-size (phase 3), not the
        // inheritance parent's — same shape as `LetterSpacing`/`WordSpacing`
        // above, nothing for phase 2 to resolve here. The `<color>`
        // component carries no length either (`TextShadowColor` doc).
        | PropertyValue::TextShadow(_)
        // F7/F8/F9 lengths need the declaring node/page context and therefore
        // remain for phase 3.
        | PropertyValue::BorderRadius(_)
        | PropertyValue::BorderRadiusTopLeft(_)
        | PropertyValue::BorderRadiusTopRight(_)
        | PropertyValue::BorderRadiusBottomRight(_)
        | PropertyValue::BorderRadiusBottomLeft(_)
        | PropertyValue::BoxShadow(_)
        | PropertyValue::Outline(_)
        | PropertyValue::OutlineWidth(_)
        | PropertyValue::OutlineStyle(_)
        | PropertyValue::OutlineColor(_)
        | PropertyValue::OutlineOffset(_)
        // CSS Grid Layout Module Level 1 grid-template-columns/-rows/-areas
        // (§7.2/§7.3) + grid-auto-columns/-rows/-flow (§7.6/§7.7) +
        // grid-row-start/-end/grid-column-start/-end (+ their `grid-row`/
        // `grid-column` shorthands, §8.3/§8.4) — same "nothing for phase 2
        // to resolve" shape as `FlexBasis`/`RowGap` above for the
        // length-bearing track-sizing ones (`GridTemplateColumns`/
        // `GridTemplateRows`/`GridAutoColumns`/`GridAutoRows`; phase 3 does
        // the declaring node's own font-size resolution over the track
        // list), and no length payload at all for the rest
        // (`GridTemplateAreas`/`GridAutoFlow`/`GridRowStart`/`GridRowEnd`/
        // `GridColumnStart`/`GridColumnEnd`/`GridRow`/`GridColumn`).
        | PropertyValue::GridTemplateColumns(_)
        | PropertyValue::GridTemplateRows(_)
        | PropertyValue::GridTemplateAreas(_)
        | PropertyValue::GridAutoColumns(_)
        | PropertyValue::GridAutoRows(_)
        | PropertyValue::GridAutoFlow(_)
        | PropertyValue::GridRowStart(_)
        | PropertyValue::GridRowEnd(_)
        | PropertyValue::GridColumnStart(_)
        | PropertyValue::GridColumnEnd(_)
        | PropertyValue::GridRow(_)
        | PropertyValue::GridColumn(_)
        // justify-items (CSS Box Alignment Module Level 3 §7.1) /
        // justify-self (§6.1) / place-items (§7.3) / place-self (§6.3) —
        // same "no length payload" shape as `AlignItems`/`AlignSelf` above.
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        | PropertyValue::PlaceItems(_)
        | PropertyValue::PlaceSelf(_)
        // `orphans`/`widows` (CSS Fragmentation Module Level 3 §3.3) carry a
        // bare positive `<integer>`, not a length — nothing for phase 2 to
        // resolve, same shape as `FlexGrow`/`FlexShrink` above.
        | PropertyValue::Orphans(_)
        | PropertyValue::Widows(_)
        // background-repeat / background-attachment / background-clip /
        // background-origin / background-size / background-position (CSS
        // Backgrounds and Borders 3 §2.4-§2.9) — nothing for phase 2 to
        // resolve against the inheritance parent (all 6 are non-inherited,
        // `background-color`'s sibling shape); `background-size`/
        // `background-position`'s `<length-percentage>` absolutization is
        // phase 3's job (`crate::specified::SpecifiedValues::absolutize_with`
        // on the element path, `crate::page`'s `absolutize_in_page_context`
        // on the page path), same as `Padding`/`Width` above.
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)
        | PropertyValue::BackgroundSize(_)
        | PropertyValue::BackgroundPosition(_)
        // background-image (CSS Backgrounds and Borders 3 §2.3) — `None` /
        // `Url(String)` are opaque to font-size/line-height resolution, same
        // as its 4 keyword-only siblings just above. `Gradient(..)` *does*
        // carry `<length-percentage>`/`<angle>` payloads (CSS Images 4 §3),
        // but this crate deliberately never resolves them against the
        // inheritance parent (or at all, pre-paint) — resolving a gradient's
        // lengths needs the gradient box's own dimensions, an input phase 2
        // doesn't have, so the whole variant is left as specified-layer data
        // all the way into `ComputedValues` (`BackgroundImage` doc's scope
        // note). Nothing for phase 2 to do here either way.
        | PropertyValue::BackgroundImage(_)
        // `background` shorthand — same "nothing for phase 2 to resolve"
        // shape as `Padding`/`Margin`/`Border`/`Flex`/`Gap` above: its
        // `<length-percentage>` components (`position`/`size`) need the
        // *declaring node's own* font-size (phase 3), not the inheritance
        // parent's, and it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::Background(_)
        // `font` shorthand — same "nothing for phase 2 to resolve" shape as
        // `Padding`/`Margin`/`Border`/`Flex`/`Gap`/`Background` above: its
        // length-bearing components (`size` as <length-percentage>,
        // `line-height` as <number>/<length-percentage>) need the
        // *declaring node's own* font-size (phase 3), not the inheritance
        // parent's, and it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::Font(_)
        // object-fit (CSS Images Module Level 3 §5.1) — non-inherited,
        // keyword-only, same "nothing for phase 2 to resolve" shape as
        // `BackgroundRepeat` above.
        | PropertyValue::ObjectFit(_)
        // object-position (CSS Images Module Level 3 §5.2) — non-inherited,
        // reuses `CssPosition` (`background-position`'s type); its
        // `<length-percentage>` absolutization is phase 3's job, same as
        // `BackgroundPosition` above.
        | PropertyValue::ObjectPosition(_)
        // opacity (CSS Color 4 §3.3) — non-inherited, carries a bare
        // `<number>`/`<percentage>`-derived `f32`, not a length, and does
        // not depend on the inheritance parent — nothing for phase 2 to
        // resolve here. The `[0,1]` clamp (CSS Color 4 §3.3's "computed
        // value: … clamped" rule) is phase 3's job
        // (`crate::specified::SpecifiedValues::absolutize_with`), same
        // split `FlexGrow`/`ZIndex` use for their own phase-3-only work.
        | PropertyValue::Opacity(_)
        // isolation (CSS Compositing and Blending Level 1 §3.4.2) /
        // mix-blend-mode (§3.4.1) — both non-inherited, bare keyword
        // payloads with no phase-2 dependency, same shape as `ObjectFit`
        // above.
        | PropertyValue::Isolation(_)
        | PropertyValue::MixBlendMode(_)
        // mask-image (CSS Masking Level 1 §7.1) / clip-path (§5.1) —
        // non-inherited, `<url>`/`<gradient>`/`<geometry-box>` payloads
        // that don't depend on the inheritance parent — nothing for phase
        // 2 to resolve here, same as `BackgroundImage` above (neither
        // crate absolutizes their embedded lengths at all — see
        // `MaskImage`/`ClipPath` doc's scope notes).
        | PropertyValue::MaskImage(_)
        | PropertyValue::ClipPath(_)
        // transform (CSS Transforms Level 1 §4) / filter (CSS Filter
        // Effects Level 1 §5) — non-inherited, embedded `Length`/`Angle`/
        // `f32` payloads that don't depend on the inheritance parent —
        // nothing for phase 2 to resolve here, same as `MaskImage`/
        // `ClipPath` above (neither absolutizes at all, see
        // `TransformFunction`/`FilterFunction` doc's scope notes).
        | PropertyValue::Transform(_)
        | PropertyValue::TransformOrigin(..)
        | PropertyValue::Filter(_)
        | PropertyValue::TableLayout(_)
        | PropertyValue::TextOverflow(_)
        | PropertyValue::BorderCollapse(_)
        // `border-spacing` (CSS Tables 3 §6.1): absolutizing `<length>{1,2}`
        // needs the declaring node's own font-size, so it belongs in phase 3
        // (like the "nothing for phase 2" `Padding`/`Margin` arms).
        // `caption-side` (§7) / `empty-cells` (§8) hold bare keywords and have
        // no phase-2 dependency (like `BorderCollapse`).
        | PropertyValue::BorderSpacing(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)
        | PropertyValue::AllRevertLayer
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Deferred(_)
        | PropertyValue::ContextualColor(_)
        | PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        | PropertyValue::Page(_)
        | PropertyValue::ColumnCount(_)
        | PropertyValue::ColumnWidth(_)
        | PropertyValue::ColumnFill(_)
        | PropertyValue::ColumnSpan(_)
        | PropertyValue::Columns(_)
        | PropertyValue::ColumnRule(_)
        | PropertyValue::ColumnRuleWidth(_)
        | PropertyValue::ColumnRuleStyle(_)
        | PropertyValue::ColumnRuleColor(_)) => v,
    })
}

/// Wrapper that **encodes in its type** passage through phase 2
/// ([`resolve_against_inherited`]). Its tuple field is private to the
/// `cascade` module: another module cannot construct this value without
/// calling [`resolve_against_inherited`].
///
/// Phase 3's `absolutize_in_page_context` in [`crate::page`] requires this
/// type. Thus any caller that reuses phase 3 on the page path must pass through
/// [`resolve_against_inherited`], regardless of its module. Even `page` itself
/// is foreign to the tuple field, since the type is defined in `cascade`:
/// there is no special access between these two modules.
///
/// # narrowed, not closed
///
/// A new path inside this module (`cascade.rs`) could access the tuple field
/// directly. A margin-box cascade that implements its own absolutization
/// instead of reusing phase 3 also bypasses this type. That remaining inability
/// to enumerate all paths is tracked separately in the margin-box cascade's
/// acceptance criteria. See §2 of "What this guard does not protect" in the
/// [`resolve_against_inherited`] docs.
///
/// This wrapper marks values that have passed phase 2 before phase 3.
/// The direct constructor exists only in test builds; production code uses
/// [`resolve_against_inherited`].
#[derive(Debug)]
pub(crate) struct ResolvedAgainstInherited(PropertyValue);

impl ResolvedAgainstInherited {
    /// Extract a value that has passed phase 2, taking ownership.
    pub(crate) fn into_property_value(self) -> PropertyValue {
        self.0
    }

    /// Inspect a value that has passed phase 2 without taking ownership.
    /// `page_context_font_size` / `page_context_border_styles` in [`crate::page`]
    /// use this to read `font-size` / `border-*-style` before phase 3.
    pub(crate) fn as_property_value(&self) -> &PropertyValue {
        &self.0
    }

    /// Test-only constructor for phase-3 inputs.
    #[cfg(test)]
    pub(crate) fn for_test(value: PropertyValue) -> Self {
        Self(value)
    }
}

/// Write one cascade winner to the staging representation
/// ([`SpecifiedValues`]) (**phase 1**).
///
/// Keep length-bearing properties in specified form and let
/// [`SpecifiedValues::finalize`] absolutize them: the font-size used for `em`
/// cannot be known until all of this node's winners have been applied.
/// Winners can be applied in any order, so resolutions depending on other
/// properties (such as `text-align: match-parent`) also do not happen here.
///
/// Exceptions are `bolder` / `lighter` in `font-weight` and `larger` /
/// `smaller` in `font-size`. [`SpecifiedValues::inherit_from`] has already
/// seeded `target` with the parent's computed values. These arms read the
/// inherited values from `target` before replacing them with absolute ones.
///
/// Arms for the shorthands that [`crate::rule::expand_shorthand_into`]
/// expands before the cascade ([`crate::rule::KeyClass::Shorthand`]) are
/// only reached when this is called directly; they delegate to the same
/// per-family expanders in `crate::rule` (e.g. [`crate::rule::expand_border`]).
/// The shorthand forms the cascade keeps whole
/// ([`crate::rule::KeyClass::Retained`]) do arrive here and set their fields
/// directly: `grid`, `grid-area` and `white-space` values, and the
/// `text-wrap` / `text-spacing` shorthand a `var()` value of one substitutes
/// into after winner selection.
///
/// `pub(crate)` permits intra-doc links from other modules.
pub(crate) fn apply_value(value: PropertyValue, target: &mut SpecifiedValues) {
    match value {
        PropertyValue::Color(c) => target.color = c,
        PropertyValue::BackgroundColor(c) => {
            target.background_color = c;
            target.background_color_expression = None;
        }
        PropertyValue::ContextualColor(value) => {
            if value.key == crate::property::PropertyKey::Color {
                // Color is applied while the staging value still holds its inherited basis.
                if let Some(color) =
                    crate::property::resolve_contextual_color(&value.source, target.color)
                {
                    target.color = color;
                }
            } else {
                target.background_color_expression = Some(value.source);
            }
        }
        PropertyValue::FontFamily(f) => target.font_family = f,
        PropertyValue::FontSize(s) => target.font_size = s,
        PropertyValue::FontSizeRelative(rel) => {
            // Always the `Length::Px` parent seed: `FontSize` shares this
            // key's single winner slot, so it has not overwritten the seed.
            let inherited_px = target.font_size.payload();
            target.font_size = Length::Px(resolve_relative_font_size(rel, inherited_px));
        }
        PropertyValue::FontWeight(fw) => {
            let inherited = target.font_weight;
            target.font_weight = resolve_relative_weight(fw, inherited);
        }
        PropertyValue::LineHeight(lh) => target.line_height = lh,
        PropertyValue::Display(d) => target.display = d,
        PropertyValue::ListStyle(v) => {
            target.list_style_type = v.kind;
            target.list_style_position = v.position;
            target.list_style_image = v.image;
        }
        PropertyValue::ListStyleType(v) => target.list_style_type = v,
        PropertyValue::ListStyleImage(v) => target.list_style_image = v,
        PropertyValue::ListStylePosition(v) => target.list_style_position = v,
        PropertyValue::CounterReset(v) => target.counter_reset = v,
        PropertyValue::CounterResetInherit => {
            target.counter_reset = crate::property::empty_counter_entries();
        }
        PropertyValue::CounterIncrement(v) => target.counter_increment = v,
        PropertyValue::CounterSet(v) => target.counter_set = v,
        PropertyValue::Content(v) => target.content = v,
        PropertyValue::StringSet(v) => target.string_set = v,
        PropertyValue::Position(pv) => match pv {
            PositionValue::Static => {
                target.position = PositionValue::Static;
            }
            PositionValue::Sticky => {
                target.position = PositionValue::Sticky;
            }
            PositionValue::Relative => {
                target.position = PositionValue::Relative;
            }
            PositionValue::Absolute => {
                target.position = PositionValue::Absolute;
            }
            PositionValue::Fixed => {
                target.position = PositionValue::Fixed;
            }
            PositionValue::Running(name) => {
                target.running_templates.push(RunningTemplate { name });
            }
        },
        PropertyValue::TextAlign(t) => target.text_align = t,
        PropertyValue::HangingPunctuation(v) => target.hanging_punctuation = v,
        // CSS Text 4: inherited keyword/flag set, simple by-value assignment.
        PropertyValue::TextAutospace(v) => target.text_autospace = v,
        PropertyValue::WordSpaceTransform(v) => target.word_space_transform = v,
        PropertyValue::TextSpacingTrim(v) => target.text_spacing_trim = v,
        PropertyValue::TextSpacingShorthand(v) => {
            target.text_spacing_trim = v.trim;
            target.text_autospace = v.autospace;
        }
        PropertyValue::TextJustify(v) => target.text_justify = v,
        PropertyValue::TextAlignLast(v) => target.text_align_last = v,
        PropertyValue::TextIndent(v) => {
            target.text_indent = v.length;
            target.text_indent_ch_factor = match v.length {
                TextIndentLength::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                TextIndentLength::Calc(calc) => calc_ch_factor(calc),
                _ => None,
            };
            target.text_indent_ch_offset = 0.0;
            target.text_indent_ch_font = None;
            target.text_indent_ch_inherited = false;
            target.text_indent_hanging = v.hanging;
            target.text_indent_each_line = v.each_line;
        }
        PropertyValue::Direction(d) => target.direction = d,
        PropertyValue::PaddingTop(v) => target.padding.top = v,
        PropertyValue::PaddingRight(v) => target.padding.right = v,
        PropertyValue::PaddingBottom(v) => target.padding.bottom = v,
        PropertyValue::PaddingLeft(v) => target.padding.left = v,
        PropertyValue::Padding(sides) => expand_padding(sides, |v| apply_value(v, target)),
        PropertyValue::PaddingInline(pair) => {
            expand_padding_inline(pair, |v| apply_value(v, target))
        }
        PropertyValue::PaddingBlock(pair) => expand_padding_block(pair, |v| apply_value(v, target)),
        // Page-cascade-only inherit markers; the element parser never produces them.
        PropertyValue::BorderRadiusInherit
        | PropertyValue::MarginTopInherit
        | PropertyValue::MarginRightInherit
        | PropertyValue::MarginBottomInherit
        | PropertyValue::MarginLeftInherit
        | PropertyValue::MarginInherit => {}
        PropertyValue::MarginTop(v) => target.margin.top = v,
        PropertyValue::MarginRight(v) => target.margin.right = v,
        PropertyValue::MarginBottom(v) => target.margin.bottom = v,
        PropertyValue::MarginLeft(v) => target.margin.left = v,
        PropertyValue::Margin(sides) => expand_margin(sides, |v| apply_value(v, target)),
        PropertyValue::MarginInline(pair) => expand_margin_inline(pair, |v| apply_value(v, target)),
        PropertyValue::MarginBlock(pair) => expand_margin_block(pair, |v| apply_value(v, target)),
        PropertyValue::BorderTopWidth(v) => target.border.top.width = v,
        PropertyValue::BorderRightWidth(v) => target.border.right.width = v,
        PropertyValue::BorderBottomWidth(v) => target.border.bottom.width = v,
        PropertyValue::BorderLeftWidth(v) => target.border.left.width = v,
        PropertyValue::BorderTopStyle(v) => target.border.top.style = v,
        PropertyValue::BorderRightStyle(v) => target.border.right.style = v,
        PropertyValue::BorderBottomStyle(v) => target.border.bottom.style = v,
        PropertyValue::BorderLeftStyle(v) => target.border.left.style = v,
        PropertyValue::BorderTopColor(v) => target.border.top.color = v,
        PropertyValue::BorderRightColor(v) => target.border.right.color = v,
        PropertyValue::BorderBottomColor(v) => target.border.bottom.color = v,
        PropertyValue::BorderLeftColor(v) => target.border.left.color = v,
        // Border longhand CSS-wide markers are resolved in `apply_winners` before
        // reaching here (they need the parent computed value or rollback candidates).
        // Keep them panic-free for direct callers that bypass that phase.
        PropertyValue::BorderTopWidthCssWide(_)
        | PropertyValue::BorderRightWidthCssWide(_)
        | PropertyValue::BorderBottomWidthCssWide(_)
        | PropertyValue::BorderLeftWidthCssWide(_)
        | PropertyValue::BorderTopStyleCssWide(_)
        | PropertyValue::BorderRightStyleCssWide(_)
        | PropertyValue::BorderBottomStyleCssWide(_)
        | PropertyValue::BorderLeftStyleCssWide(_)
        | PropertyValue::BorderTopColorCssWide(_)
        | PropertyValue::BorderRightColorCssWide(_)
        | PropertyValue::BorderBottomColorCssWide(_)
        | PropertyValue::BorderLeftColorCssWide(_) => {}
        // `text-decoration-thickness: inherit` is resolved in `apply_winners`
        // before reaching here (it needs the parent computed value).
        // Keep it panic-free for direct callers that bypass that phase.
        PropertyValue::TextDecorationThicknessInherit => {}
        PropertyValue::Border(sides) => expand_border(sides, |v| apply_value(v, target)),
        PropertyValue::BorderTop(border) => expand_border_top(border, |v| apply_value(v, target)),
        PropertyValue::BorderRight(border) => {
            expand_border_right(border, |v| apply_value(v, target))
        }
        PropertyValue::BorderBottom(border) => {
            expand_border_bottom(border, |v| apply_value(v, target))
        }
        PropertyValue::BorderLeft(border) => expand_border_left(border, |v| apply_value(v, target)),
        PropertyValue::BorderCssWide(kw) => expand_border_css_wide(kw, |v| apply_value(v, target)),
        PropertyValue::BorderTopCssWide(kw) => {
            expand_border_top_css_wide(kw, |v| apply_value(v, target))
        }
        PropertyValue::BorderRightCssWide(kw) => {
            expand_border_right_css_wide(kw, |v| apply_value(v, target))
        }
        PropertyValue::BorderBottomCssWide(kw) => {
            expand_border_bottom_css_wide(kw, |v| apply_value(v, target))
        }
        PropertyValue::BorderLeftCssWide(kw) => {
            expand_border_left_css_wide(kw, |v| apply_value(v, target))
        }
        PropertyValue::Width(v) => target.width = v,
        PropertyValue::Height(v) => target.height = v,
        PropertyValue::MaxWidth(v) => target.max_width = v,
        PropertyValue::MaxHeight(v) => target.max_height = v,
        PropertyValue::MinWidth(v) => target.min_width = v,
        PropertyValue::MinHeight(v) => target.min_height = v,
        PropertyValue::MinBlockSize(v) => target.min_block_size = Some(v),
        PropertyValue::InlineSize(v) => target.inline_size = Some(v),
        PropertyValue::BlockSize(v) => target.block_size = Some(v),
        PropertyValue::Top(v) => target.top = v,
        PropertyValue::Right(v) => target.right = v,
        PropertyValue::Bottom(v) => target.bottom = v,
        PropertyValue::Left(v) => target.left = v,
        PropertyValue::BoxSizing(bs) => target.box_sizing = bs,
        PropertyValue::OverflowX(v) => target.overflow.x = v,
        PropertyValue::OverflowY(v) => target.overflow.y = v,
        PropertyValue::Overflow(pair) => expand_overflow(pair, |v| apply_value(v, target)),
        PropertyValue::TextDecorationLine(v) => target.text_decoration_line = v,
        PropertyValue::TextDecorationStyle(v) => target.text_decoration_style = v,
        PropertyValue::TextDecorationColor(v) => target.text_decoration_color = v,
        PropertyValue::TextDecorationThickness(v) => target.text_decoration_thickness = v,
        PropertyValue::TextDecorationSkipInk(v) => target.text_decoration_skip_ink = v,
        PropertyValue::TextDecorationSkipSpaces(v) => target.text_decoration_skip_spaces = v,
        PropertyValue::TextDecorationInset(v) => target.text_decoration_inset = v,
        PropertyValue::TextUnderlineOffset(v) => target.text_underline_offset = v,
        PropertyValue::TextUnderlinePosition(v) => target.text_underline_position = v,
        PropertyValue::TextEmphasisPosition(v) => target.text_emphasis_position = v,
        PropertyValue::TextEmphasisStyle(v) => target.text_emphasis_style = v,
        PropertyValue::TextEmphasisColor(v) => target.text_emphasis_color = v,
        PropertyValue::TextEmphasis(shorthand) => {
            crate::rule::expand_text_emphasis(&shorthand, |value| apply_value(value, target))
        }
        PropertyValue::TextDecoration(shorthand) => {
            expand_text_decoration(shorthand, |v| apply_value(v, target))
        }
        PropertyValue::VerticalAlign(va) => target.vertical_align = va,
        PropertyValue::FontStyle(fs) => target.font_style = fs,
        PropertyValue::FontKerning(value) => target.font_kerning = value,
        PropertyValue::FontOpticalSizing(value) => target.font_optical_sizing = value,
        PropertyValue::FontVariantEmoji(value) => target.font_variant_emoji = value,
        PropertyValue::FontLanguageOverride(value) => target.font_language_override = value,
        PropertyValue::FontVariantLigatures(value) => target.font_variant_ligatures = value,
        PropertyValue::FontSynthesis(value) => target.font_synthesis = value,
        PropertyValue::FontVariantPosition(value) => target.font_variant_position = value,
        PropertyValue::FontPalette(value) => target.font_palette = value,
        PropertyValue::FontVariantNumeric(value) => target.font_variant_numeric = value,
        PropertyValue::FontVariantEastAsian(value) => target.font_variant_east_asian = value,
        PropertyValue::FontVariationSettings(value) => target.font_variation_settings = value,
        PropertyValue::FontFeatureSettings(value) => target.font_feature_settings = value,
        PropertyValue::TextTransform(tt) => target.text_transform = tt,
        PropertyValue::TextCombineUpright(value) => target.text_combine_upright = value,
        PropertyValue::TextOrientation(value) => target.text_orientation = value,
        PropertyValue::UnicodeBidi(value) => target.unicode_bidi = value,
        PropertyValue::Visibility(v) => target.visibility = v,
        PropertyValue::ZIndex(z) => target.z_index = z,
        PropertyValue::WordBreak(wb) => target.word_break = wb,
        PropertyValue::LineBreak(lb) => target.line_break = lb,
        PropertyValue::OverflowWrap(ow) => target.overflow_wrap = ow,
        PropertyValue::LetterSpacing(ls) => {
            target.letter_spacing = ls;
            target.letter_spacing_ch_factor = match ls {
                LetterSpacingValue::Length(Length::Ch(factor)) if factor.is_finite() => {
                    Some(factor)
                }
                LetterSpacingValue::Calc(calc) => calc_ch_factor(calc),
                _ => None,
            };
            target.letter_spacing_ch_offset = 0.0;
            target.letter_spacing_ch_font = None;
        }
        PropertyValue::WordSpacing(ws) => {
            target.word_spacing = ws;
            target.word_spacing_ch_factor = match ws {
                WordSpacingValue::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                WordSpacingValue::Calc(calc) => calc_ch_factor(calc),
                _ => None,
            };
            target.word_spacing_ch_offset = 0.0;
            target.word_spacing_ch_font = None;
        }
        PropertyValue::TabSize(ts) => target.tab_size = ts,
        PropertyValue::BreakBefore(bb) => target.break_before = bb,
        PropertyValue::BreakAfter(bb) => target.break_after = bb,
        PropertyValue::BreakInside(bi) => target.break_inside = bi,
        PropertyValue::Float(f) => target.float = f,
        PropertyValue::Clear(c) => target.clear = c,
        PropertyValue::WhiteSpace(ws) => target.white_space = ws,
        PropertyValue::WhiteSpaceCollapse(value) => target.white_space_collapse = value,
        PropertyValue::TextWrap(v) => target.text_wrap = v,
        PropertyValue::TextWrapStyle(v) => target.text_wrap_style = v,
        PropertyValue::TextWrapShorthand(v) => {
            target.text_wrap = v.mode;
            target.text_wrap_style = v.style;
        }
        PropertyValue::Hyphens(h) => target.hyphens = h,
        PropertyValue::HyphenateCharacter(value) => target.hyphenate_character = value,
        PropertyValue::HyphenateLimitChars(value) => target.hyphenate_limit_chars = value,
        PropertyValue::FlexDirection(fd) => target.flex_direction = fd,
        PropertyValue::FlexWrap(fw) => target.flex_wrap = fw,
        PropertyValue::FlexGrow(g) => target.flex_grow = g,
        PropertyValue::FlexShrink(s) => target.flex_shrink = s,
        PropertyValue::FlexBasis(fb) => target.flex_basis = fb,
        PropertyValue::Flex(f) => expand_flex(f, |v| apply_value(v, target)),
        PropertyValue::FlexFlow(f) => expand_flex_flow(f, |v| apply_value(v, target)),
        PropertyValue::Order(o) => target.order = o,
        PropertyValue::JustifyContent(jc) => target.justify_content = jc,
        PropertyValue::AlignContent(ac) => target.align_content = ac,
        PropertyValue::AlignItems(ai) => target.align_items = ai,
        PropertyValue::AlignSelf(as_) => target.align_self = as_,
        PropertyValue::RowGap(rg) => target.row_gap = rg,
        PropertyValue::ColumnGap(cg) => target.column_gap = cg,
        PropertyValue::Gap(g) => expand_gap(g, |v| apply_value(v, target)),
        PropertyValue::PlaceContent(p) => expand_place_content(p, |v| apply_value(v, target)),
        PropertyValue::FontVariantCaps(fvc) => target.font_variant_caps = fvc,
        PropertyValue::Quotes(v) => {
            target.quotes_auto = false;
            target.quotes = v;
        }
        PropertyValue::TextShadow(shadows) => target.text_shadow = shadows,
        PropertyValue::BorderRadius(v) => target.border_radius = v,
        PropertyValue::BorderRadiusTopLeft(v) => target.border_radius.top_left = v,
        PropertyValue::BorderRadiusTopRight(v) => target.border_radius.top_right = v,
        PropertyValue::BorderRadiusBottomRight(v) => target.border_radius.bottom_right = v,
        PropertyValue::BorderRadiusBottomLeft(v) => target.border_radius.bottom_left = v,
        PropertyValue::BoxShadow(shadows) => target.box_shadow = shadows,
        PropertyValue::Outline(outline) => expand_outline(outline, |v| apply_value(v, target)),
        PropertyValue::OutlineWidth(v) => target.outline.width = v,
        PropertyValue::OutlineStyle(v) => target.outline.style = v,
        PropertyValue::OutlineColor(v) => target.outline.color = v,
        PropertyValue::OutlineOffset(v) => target.outline_offset = v,
        PropertyValue::GridArea(area) => {
            target.grid_row_start = area.row_start;
            target.grid_column_start = area.column_start;
            target.grid_row_end = area.row_end;
            target.grid_column_end = area.column_end;
        }
        PropertyValue::Grid(shorthand) => {
            target.grid_template_rows = shorthand.rows;
            target.grid_template_columns = shorthand.columns;
            target.grid_template_areas = GridTemplateAreasValue::None;
            target.grid_auto_columns = initial_grid_auto_track_list();
            target.grid_auto_rows = initial_grid_auto_track_list();
            target.grid_auto_flow = GridAutoFlowValue::Row;
            target.grid_row_start = GridLineValue::Auto;
            target.grid_row_end = GridLineValue::Auto;
            target.grid_column_start = GridLineValue::Auto;
            target.grid_column_end = GridLineValue::Auto;
        }
        PropertyValue::GridTemplateColumns(v) => target.grid_template_columns = v,
        PropertyValue::GridTemplateRows(v) => target.grid_template_rows = v,
        PropertyValue::GridTemplateAreas(v) => target.grid_template_areas = v,
        PropertyValue::GridAutoColumns(v) => target.grid_auto_columns = v,
        PropertyValue::GridAutoRows(v) => target.grid_auto_rows = v,
        PropertyValue::GridAutoFlow(v) => target.grid_auto_flow = v,
        PropertyValue::GridRowStart(v) => target.grid_row_start = v,
        PropertyValue::GridRowEnd(v) => target.grid_row_end = v,
        PropertyValue::GridColumnStart(v) => target.grid_column_start = v,
        PropertyValue::GridColumnEnd(v) => target.grid_column_end = v,
        PropertyValue::GridRow(shorthand) => {
            expand_grid_row(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::GridColumn(shorthand) => {
            expand_grid_column(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::JustifyItems(v) => target.justify_items = v,
        PropertyValue::JustifySelf(v) => target.justify_self = v,
        PropertyValue::PlaceItems(p) => expand_place_items(p, |v| apply_value(v, target)),
        PropertyValue::PlaceSelf(p) => expand_place_self(p, |v| apply_value(v, target)),
        PropertyValue::Orphans(n) => target.orphans = n,
        PropertyValue::Widows(n) => target.widows = n,
        PropertyValue::WritingMode(v) => target.writing_mode = v,
        PropertyValue::RubyPosition(v) => target.ruby_position = v,
        PropertyValue::BackgroundRepeat(v) => target.background_repeat = v,
        PropertyValue::BackgroundAttachment(v) => target.background_attachment = v,
        PropertyValue::BackgroundClip(v) => target.background_clip = v,
        PropertyValue::BackgroundOrigin(v) => target.background_origin = v,
        PropertyValue::BackgroundSize(v) => target.background_size = v,
        PropertyValue::BackgroundPosition(v) => target.background_position = v,
        PropertyValue::BackgroundImage(v) => target.background_image = v,
        PropertyValue::Background(shorthand) => {
            expand_background(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::BorderStyle(sides) => expand_border_style(sides, |v| apply_value(v, target)),
        PropertyValue::BorderWidth(sides) => expand_border_width(sides, |v| apply_value(v, target)),
        PropertyValue::BorderColor(sides) => expand_border_color(sides, |v| apply_value(v, target)),
        PropertyValue::Font(shorthand) => expand_font(&shorthand, |v| apply_value(v, target)),
        PropertyValue::ObjectFit(v) => target.object_fit = v,
        PropertyValue::ObjectPosition(v) => target.object_position = v,
        PropertyValue::Opacity(v) => target.opacity = v,
        PropertyValue::Isolation(v) => target.isolation = v,
        PropertyValue::MixBlendMode(v) => target.mix_blend_mode = v,
        PropertyValue::MaskImage(v) => target.mask_image = v,
        PropertyValue::ClipPath(v) => target.clip_path = v,
        PropertyValue::Transform(v) => target.transform = v,
        PropertyValue::TransformOrigin(position, z) => {
            target.transform_origin = position;
            target.transform_origin_z = z;
        }
        PropertyValue::Filter(v) => target.filter = v,
        PropertyValue::TableLayout(v) => target.table_layout = v,
        PropertyValue::TextOverflow(v) => target.text_overflow = v,
        PropertyValue::BorderCollapse(v) => target.border_collapse = v,
        PropertyValue::BorderSpacing(v) => target.border_spacing = v,
        PropertyValue::CaptionSide(v) => target.caption_side = v,
        PropertyValue::EmptyCells(v) => target.empty_cells = v,
        // Parsed but not yet staged for elements.
        PropertyValue::TextAlignAll(_) => {}
        PropertyValue::Page(value) => target.page = value,
        PropertyValue::ColumnCount(value) => target.column_count = value,
        PropertyValue::ColumnFill(value) => target.column_fill = value,
        PropertyValue::ColumnSpan(value) => target.column_span = value,
        PropertyValue::ColumnWidth(value) => target.column_width = value,
        PropertyValue::ColumnRuleWidth(value) => target.column_rule.width = value,
        PropertyValue::ColumnRuleStyle(value) => target.column_rule.style = value,
        PropertyValue::ColumnRuleColor(value) => target.column_rule.color = value,
        PropertyValue::ColumnRule(value) => target.column_rule = value,
        // cov:ignore: direct unexpanded shorthand callers are defensive-only.
        PropertyValue::Columns(value) => {
            target.column_count = value.count;
            target.column_width = value.width;
        }
        // Resolved before ordinary winners reach this function.
        PropertyValue::AllRevertLayer
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Deferred(_) => {}
    }
}

// cov:ignore: pure test-code relocation (no logic changed). patch-coverage's git-diff-based line classifier treats every moved
// line as newly added, and cargo-llvm-cov does not record hits for
// multi-line string-literal continuation lines inside assert!/panic!
// messages even though the containing statement executes in a
// passing test. Verified against every flagged line in this move:
// all are string-literal fragments or trivial format-arg
// expressions inside already-passing tests, none of them
// production code.
#[cfg(test)]
mod tests;
