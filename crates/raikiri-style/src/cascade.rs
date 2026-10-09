//! CSS cascade and inheritance walk.
//!
//! One top-down, pre-order walk of the document ([`walk_from`]). For each node
//! it collects the candidate declarations of the element and its
//! pseudo-elements (matching rules, inline style, presentational hints),
//! selects the winners by origin, `!important`, layer, specificity and source
//! order, then inherits the parent's computed values and overrides them with
//! the node's own winners.
//!
//! # Four stages of resolving one node
//!
//! Resolving a node's winners has four stages (named phases 1 / 2 / 2.5 / 3,
//! in order):
//!
//! - **phase 1: stage winners** — seed [`crate::specified::SpecifiedValues`] from
//!   the parent's [`ComputedValues`] and apply all the node's winners with
//!   `apply_value`. Lengths remain in specified form here.
//! - **phase 2: absolutize font-size** — relative to the **parent's** computed font-size.
//! - **phase 2.5: absolutize line-height** — relative to the node's **own**
//!   (now determined) font-size. See the canonical [`crate::resolve::resolve_line_height`]
//!   documentation for the asymmetry of the `lh`/`rlh` self-reference basis.
//! - **phase 3: absolutize all remaining lengths** — relative to the node's
//!   **own** computed font-size / line-height.
//!
//! Phases 2 / 2.5 / 3 are encapsulated in [`crate::specified::SpecifiedValues::finalize`].
//! See the [`crate::specified`] module docs for why these stages must be separate:
//! the `font-size` that serves as the basis for `padding: 2em` is not known until
//! **all** the node's winners have been applied, so lengths cannot be absolutized
//! while winners are being applied.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::PseudoElem;
use crate::computed::ComputedValues;
use crate::counter_style::CounterStyleRegistry;
use crate::error::CascadeError;
use crate::media::MediaContext;
use crate::page::{
    PageCascadeResult, PageContextQuery, PageInheritance, cascade_page_with_media_context,
};
use crate::property::{CssColor, PropertyKey, WritingMode};
use crate::ruletree::RuleTree;
use crate::style_dom::{StyleDom, StyleNode, StyleNodeId, StyleNodeKind};

static NEXT_CASCADE_GENERATION: AtomicU64 = AtomicU64::new(1);

/// An explicitly cascaded SVG property prepared for a vector consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgStyleProperty {
    /// Property whose winning declaration participates in SVG painting.
    pub property: PropertyKey,
    /// Keep literal inheritance instead of freezing a definition's value.
    /// This preserves inheritance when a definition is instantiated by `use`.
    pub inherited: bool,
    /// A relative font expression that must resolve in the SVG instance.
    /// Other values use the node's computed value.
    pub expression: Option<String>,
}

/// Cascade result.
///
/// The future static-side GCPM (paged media generated content) implementation
/// is expected to store content / string_set / running_templates in per-node
/// ComputedValues as the canonical taxonomy. Responsibility for concatenating
/// CascadeResult-level `gcpm_directives` / `running_templates` per document
/// would move downstream to raikiri-dom. Keep `#[non_exhaustive]` for future
/// field additions.
#[derive(Debug)]
#[non_exhaustive]
pub struct CascadeResult {
    generation: u64,
    svg_style_properties: HashMap<StyleNodeId, Vec<SvgStyleProperty>>,
    first_letter_inputs: HashMap<StyleNodeId, first_letter::FirstLetterInputs>,
    typographic_inheritance: HashMap<(StyleNodeId, Option<PseudoElem>), OwnedCandidates>,
    /// Index into [`Self::computed`] of the `@page` inheritance parent: the
    /// root element, or the Document node when the tree has no element child.
    root_element_index: usize,
    /// Winning custom-highlight background colors keyed by highlight name.
    /// Contextual expressions use initial black in this inspection view. Use
    /// [`Self::custom_highlight_background`] with the originating foreground
    /// for painting.
    pub custom_highlight_styles: HashMap<String, CssColor>,
    custom_highlight_sources: HashMap<String, smol_str::SmolStr>,
    /// Per-node computed values (indexed by NodeId.0 as usize).
    /// Populated for Element / Text / Document kinds; out-of-range access
    /// panics and is the caller's responsibility.
    pub computed: Vec<ComputedValues>,
    /// Per-node flags indicating whether the cascade had an explicit
    /// `opacity` declaration for that node. Inline SVG painting uses this to
    /// decide whether source-root opacity belongs in the host paint group.
    pub opacity_specified: Vec<bool>,
    /// Per-node flags indicating whether the cascade had an explicit
    /// `background-color` declaration. Inline SVG painting uses this to omit
    /// a source-root background when the host declaration computes transparent.
    pub background_color_specified: Vec<bool>,
    /// Authored `writing-mode` winners before the computed-value normalization
    /// that currently collapses vertical modes to `horizontal-tb`.  This keeps
    /// paged consumers able to apply physical page-context mapping without
    /// changing ordinary element layout semantics.
    pub authored_writing_modes: Vec<Option<WritingMode>>,
    /// The `@page` cascade for the page query supplied to the cascade entry
    /// point. The compatibility entry point uses the unnamed/default query;
    /// paged consumers should use [`cascade_with_media_context_for_page`].
    ///
    /// The page context is a layout input, so a caller deriving another page
    /// query's result from this one should use [`Self::replace_page`] rather
    /// than assigning this field: assignment keeps the old
    /// [`Self::generation`], and generation-keyed layout caches would treat
    /// the changed result as the cascade they last saw.
    pub page: PageCascadeResult,
    /// The winning `@counter-style` registry captured from the rule tree.
    ///
    /// Counter styles are stylesheet-level rules, not per-node computed
    /// values, but marker and generated-content paint runs only receive the
    /// cascade result. Cloning this small registry at cascade time keeps the
    /// rule-tree ownership boundary while making custom counter formatting
    /// available to those consumers.
    pub counter_styles: CounterStyleRegistry,
    /// Per-node computed page type (`page: auto | <custom-ident>`).
    ///
    /// This is kept beside, rather than inside, `ComputedValues` because the
    /// page property is consumed by the pagination driver and is not part of
    /// the element-to-taffy computed style bag.  The vector has the same arena
    /// indexing contract as [`Self::computed`].
    pub page_values: Vec<crate::property::PageValue>,
    /// `::before` / `::after` — sparse, keyed by `(originating element's own
    /// NodeId, which pseudo)`. An entry exists **iff** at least one
    /// stylesheet rule's selector targets that pseudo-element and matches
    /// that element (CSS Pseudo-Elements Module Level 4 §4.1
    /// <https://drafts.csswg.org/css-pseudo-4/#generated-content>) — most
    /// elements have neither `::before` nor `::after` rules, so this stays
    /// empty for them (no wasted entry).
    ///
    /// Each value is a **full** [`ComputedValues`], computed the same way a
    /// real element's is — cascaded winners from matching `::before`/
    /// `::after` selectors applied over inherited-from-the-real-element
    /// values, then absolutized — not just a lone `content` value. Two
    /// distinct spec statements justify this, and neither alone would:
    /// §4 `#treelike` (tree-abiding pseudo-elements, which `::before`/
    /// `::after` are a subcase of) states the *unconditioned* inheritance
    /// model this crate actually implements — "They inherit any inheritable
    /// properties from their originating element; non-inheritable
    /// properties take their initial values as usual" — independent of what
    /// `content` computes to. §4.1 `#generated-content` separately states
    /// the *content-conditioned* box-generation model — "When their
    /// computed 'content' value is not 'none', these pseudo-elements
    /// generate boxes as if they were immediate children of their
    /// originating element" — which is a downstream (`raikiri-dom`)
    /// decision, not something this crate's cascade computes. So a pseudo
    /// inherits its `color`/`font-*`/etc. from the real element exactly
    /// like a real child would (§4, unconditionally), and can have those
    /// (and any other property) overridden by its own `::before`/`::after`
    /// rule; whether a box is actually generated from the result (§4.1,
    /// conditioned on `content`) is left to the consumer, per this doc's own
    /// content representation note below. See [`walk_from`]'s
    /// pseudo-element section for the derivation.
    ///
    /// This crate represents `content: normal` as an empty `content` list and
    /// explicit `content: none` with [`crate::property::ContentComponent::None`] inside the
    /// list ([`ComputedValues::content`] doc) — a present
    /// map entry with an empty `content` list means "a `::before`/`::after`
    /// rule matched, but its (or the initial) `content` value is `normal`";
    /// an explicit `none` carries the sentinel. CSS Content Module Level 3
    /// <https://www.w3.org/TR/css-content-3/#content-property> is the
    /// primary source for what a non-empty `content` value implies for box
    /// generation — this crate stops at exposing the computed value; the
    /// consumer decides box generation from whether the list is empty or
    /// carries [`crate::property::ContentComponent::None`], not from
    /// map presence (map presence only means "some `::before`/`::after`
    /// rule matched this element", independent of what that rule set
    /// `content` to). This content representation ⇒ suppress-box reading is
    /// scoped to **this** (`pseudo`) map's entries specifically — it does
    /// not carry over to [`Self::computed`] (real elements). CSS Content 3
    /// §1 <https://www.w3.org/TR/css-content-3/#content-property> draws
    /// exactly this real-element/pseudo-element split itself: "For
    /// elements, \[`content`\] has only one purpose: specifying that the
    /// element renders as normal, or replacing the element with an image
    /// [...]. For pseudo-elements [...], it is more powerful. It controls
    /// whether the element renders at all, can replace the element with an
    /// image, or replace it with arbitrary inline content" — an empty
    /// `content` list on a **real** element's own [`Self::computed`] entry
    /// means "renders as normal" (no box-suppression meaning at all), while
    /// the same empty list on a **pseudo**'s [`Self::pseudo`] entry means
    /// "no box" (§4.1's `content: not none` condition above).
    ///
    /// `::marker` (CSS Lists 3 §3.7) shares this same map and the same full-
    /// `ComputedValues` treatment. `::first-line` (CSS Pseudo-Elements
    /// Module Level 4 §2.1 `#first-line-pseudo`) also shares it, but map
    /// presence for a `::first-line` entry carries none of the `content`-
    /// based box-suppression reading above — `::first-line` has no
    /// `content`-driven box-generation model at all (see [`PseudoElem`]
    /// doc), so presence means only "some `::first-line` rule matched this
    /// element". §2.1.2 `#first-line-styling` also restricts which
    /// properties may apply through `::first-line`; this crate
    /// accepts font, background, color, opacity, text decoration and
    /// supported inline typesetting/layout properties, while excluding
    /// box geometry, writing-mode, direction and text-orientation.
    pub pseudo: HashMap<(StyleNodeId, PseudoElem), ComputedValues>,
}

impl CascadeResult {
    /// Explicit SVG paint properties after winner selection and defaulting.
    ///
    /// Read concrete values from [`Self::computed`]. Properties marked
    /// inherited must remain literal `inherit` in exported vector source;
    /// relative expressions must resolve in the SVG instance.
    /// Nodes without SVG declarations return an empty slice.
    pub fn svg_style_properties(&self, node: StyleNodeId) -> &[SvgStyleProperty] {
        self.svg_style_properties
            .get(&node)
            .map_or(&[], Vec::as_slice)
    }
    /// Resolve a named highlight background against its originating foreground.
    /// Literal backgrounds use the same winning declaration as
    /// [`Self::custom_highlight_styles`]; contextual expressions remain deferred
    /// until the caller supplies the foreground of the highlighted text.
    pub fn custom_highlight_background(
        &self,
        name: &str,
        foreground: CssColor,
    ) -> Option<CssColor> {
        if let Some(source) = self.custom_highlight_sources.get(name) {
            crate::property::resolve_contextual_color(source, foreground)
        } else {
            self.custom_highlight_styles.get(name).copied()
        }
    }

    /// Opaque identity for the cascade run that produced this result.
    ///
    /// Layout caches use it to avoid reusing placement data after a new cascade.
    #[doc(hidden)]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Computed values of the root element, the inheritance parent of the
    /// `@page` cascade.
    ///
    /// CSS Paged Media 3 §6 (<https://www.w3.org/TR/css-page-3/#page-properties>)
    /// states that the page context inherits from the root element. This is
    /// the first element child of the Document node; a tree without one falls
    /// back to the Document node's own computed values. The element cascade
    /// does not depend on the page query, so a caller holding the same
    /// [`RuleTree`] and [`MediaContext`] can cascade another page query with
    /// [`cascade_page_with_media_context`] and
    /// `PageInheritance::FromRoot(result.root_element_computed())` instead of
    /// rerunning the whole cascade.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::computed`] was shrunk after the cascade produced it.
    pub fn root_element_computed(&self) -> &ComputedValues {
        &self.computed[self.root_element_index]
    }

    /// Replace the `@page` cascade with one for another page query.
    ///
    /// The element results are unchanged; `page` is expected to come from
    /// [`cascade_page_with_media_context`] inheriting from
    /// [`Self::root_element_computed`] with the same [`RuleTree`] and
    /// [`MediaContext`]. The page context is a layout input, so the result
    /// gets a fresh [`Self::generation`] and generation-keyed caches treat it
    /// as a new cascade.
    pub fn replace_page(&mut self, page: PageCascadeResult) {
        self.page = page;
        self.generation = NEXT_CASCADE_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Index of the node whose computed values the `@page` cascade inherits from.
///
/// `StyleDom::root_id()` is the Document node.  Page properties inherit from
/// the first direct element child (the root element), not from that Document
/// node's initial values.  `computed_len` guards against an element id outside
/// the computed arena, in which case the Document node is used instead.
fn page_inheritance_root_index<D: StyleDom>(dom: &D, computed_len: usize) -> usize {
    dom.child_ids(dom.root_id())
        .find(|id| {
            dom.node(*id)
                .is_some_and(|node| node.kind() == StyleNodeKind::Element)
        })
        .map(|id| id.0 as usize)
        .filter(|index| *index < computed_len)
        .unwrap_or(dom.root_id().0 as usize)
}

/// Produce per-node ComputedValues from the DOM and RuleTree.
///
/// Currently always returns `Ok` (invalid CSS is silently dropped during
/// build_rule_tree, so cascade errors cannot arise). Keep the `Result`
/// signature for a future fail-hard mode.
///
/// # Example
///
/// ```ignore
/// use raikiri_style::{build_rule_tree, cascade, ComputedValues};
///
/// # fn demo<D: raikiri_style::StyleDom>(dom: &D) {
/// let rule_tree = build_rule_tree(dom);
/// let result = cascade(dom, &rule_tree).expect("現時点では常に Ok");
/// let root_style: &ComputedValues = &result.computed[0];
/// # }
/// ```
pub fn cascade<D: StyleDom>(dom: &D, rule_tree: &RuleTree) -> Result<CascadeResult, CascadeError> {
    cascade_with_media_context(dom, rule_tree, &MediaContext::default())
}

/// Run the cascade for an explicit media context.
///
/// [`cascade`] remains the compatibility entry point and uses the default
/// paged (`print`) context.
pub fn cascade_with_media_context<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
) -> Result<CascadeResult, CascadeError> {
    cascade_with_media_context_for_page(dom, rule_tree, media_context, &PageContextQuery::default())
}

/// Run the element and `@page` cascades for one page-context query.
///
/// This is the page-aware sibling of [`cascade_with_media_context`]. It keeps
/// the ordinary element cascade and the page-context cascade in one result so
/// a paged consumer cannot accidentally render with a page box selected from a
/// different stylesheet or root inheritance context.
pub fn cascade_with_media_context_for_page<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
) -> Result<CascadeResult, CascadeError> {
    let outputs = walk(dom, rule_tree, media_context, WalkOptions::default())?;
    Ok(finish(dom, rule_tree, page_query, media_context, outputs))
}

/// The cascade result of one walk, with the `@page` cascade for `page_query`.
///
/// The walk visits only nodes reachable from the root, so detached or
/// unreachable nodes (foster-parenting transients, orphans after stripping,
/// etc.) keep their initial values. The contract `computed.len() ==
/// document.node_count()` still covers every node, since raikiri-dom's layout
/// and raikiri-paint's walker index `computed[idx]` directly by node id; the
/// walk gives every node a slot.
fn finish<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    page_query: &PageContextQuery,
    media_context: &MediaContext,
    outputs: WalkOutputs,
) -> CascadeResult {
    let root_element_index = page_inheritance_root_index(dom, outputs.computed.len());
    let page = cascade_page_with_media_context(
        rule_tree,
        page_query,
        PageInheritance::FromRoot(&outputs.computed[root_element_index]),
        media_context,
    );
    CascadeResult {
        generation: NEXT_CASCADE_GENERATION.fetch_add(1, Ordering::Relaxed),
        svg_style_properties: outputs.svg_properties,
        first_letter_inputs: outputs.first_letter_inputs,
        typographic_inheritance: outputs.typographic_inheritance,
        root_element_index,
        custom_highlight_styles: rule_tree.custom_highlight_styles().clone(),
        custom_highlight_sources: rule_tree.custom_highlight_sources().clone(),
        computed: outputs.computed,
        opacity_specified: outputs.opacity_specified,
        background_color_specified: outputs.background_color_specified,
        authored_writing_modes: outputs.authored_writing_modes,
        page,
        counter_styles: rule_tree.counter_styles_for(media_context),
        page_values: outputs.page_values,
        pseudo: outputs.pseudo,
    }
}

mod candidate;
use candidate::OwnedCandidates;
mod collect;
pub(crate) mod rollback;
pub(crate) use collect::*;
mod directionality;
pub(crate) mod lang;
mod query;
mod rule_index;
mod selector_match;
pub(crate) use directionality::*;
pub use query::{SelectorMatcher, SelectorQuery};
mod custom_property;
mod html_quirks;
mod svg_hints;
mod table_hints;
pub(crate) use custom_property::*;
mod first_letter;
mod first_line;
pub use first_line::{FirstLineCascade, FirstLineStyles, cascade_with_first_line};
mod inherit;
pub(crate) use inherit::*;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;
