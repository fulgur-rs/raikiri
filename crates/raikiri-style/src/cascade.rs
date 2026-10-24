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

use crate::computed::{ComputedValues, VerticalLogicalSize};
use crate::counter_style::CounterStyleRegistry;
use crate::error::CascadeError;
use crate::media::MediaContext;
use crate::page::{
    PageCascadeResult, PageContextQuery, PageInheritance, cascade_page_with_normal_line_height,
};
use crate::property::{
    Contain, CssColor, Direction, DisplayValue, PropertyKey, TextOrientation, WritingMode,
};
use crate::resolve::{ComputedLengthPercentageOrAuto, ResolveContext};
use crate::ruletree::RuleTree;
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};
use crate::{PseudoElem, PseudoValueArc};

static NEXT_CASCADE_GENERATION: AtomicU64 = AtomicU64::new(1);

/// An explicitly cascaded SVG property prepared for a vector consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgStyleProperty {
    /// Property whose winning declaration participates in SVG painting.
    pub property: PropertyKey,
    /// Keep literal inheritance instead of freezing a definition's value.
    /// This preserves inheritance when a definition is instantiated by `use`.
    pub inherited: bool,
    /// A relative font expression that must resolve in the SVG instance, or the
    /// validated value of a painting property (see
    /// [`PropertyKey::is_svg_presentation`]), which has no computed field.
    /// Other values use the node's computed value.
    pub expression: Option<String>,
    /// The ancestor whose declaration `expression` takes the place of, when the
    /// cascade resolved an `inherit` of a stroke length against the source
    /// tree. A `<use>` instance inherits from the `use` element instead, so a
    /// consumer that knows the instance falls back to the literal `inherit`
    /// when this ancestor is not part of the instantiated subtree.
    pub inherited_from: Option<StyleNodeId>,
}

/// Cascade result.
///
/// The future static-side GCPM (paged media generated content) implementation
/// is expected to store content / string_set / running_templates in per-node
/// ComputedValues as the canonical taxonomy. Responsibility for concatenating
/// CascadeResult-level `gcpm_directives` / `running_templates` per document
/// would move downstream to raikiri-dom. Keep `#[non_exhaustive]` for future
/// field additions.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CascadeResult {
    generation: u64,
    /// Identity of the element cascade alone: [`Self::replace_page`] keeps it.
    element_generation: u64,
    svg_style_properties: HashMap<StyleNodeId, Vec<SvgStyleProperty>>,
    svg_relative_presentation_attributes: HashMap<StyleNodeId, Vec<SvgStyleProperty>>,
    first_letter_inputs: HashMap<StyleNodeId, first_letter::FirstLetterInputs>,
    first_line_contexts: HashMap<StyleNodeId, ResolveContext>,
    typographic_inheritance: HashMap<(StyleNodeId, Option<PseudoElem>), OwnedCandidates>,
    /// Index into [`Self::computed`] of the `@page` inheritance parent: the
    /// root element, or the Document node when the tree has no element child.
    root_element_index: usize,
    /// Index into [`Self::computed`] of the principal element: the element
    /// whose `writing-mode`, `direction` and `text-orientation` the root
    /// element takes as its used values (CSS Writing Modes 4 §8). It is the
    /// root element itself when no value propagates.
    principal_element_index: usize,
    /// What the root element's entry of [`Self::computed`] replaced of its own
    /// values with the principal element's flow. `None` when nothing was
    /// replaced.
    root_own_flow: Option<RootOwnFlow>,
    /// Winning custom-highlight background colors keyed by highlight name.
    /// Contextual expressions use initial black in this inspection view. Use
    /// [`Self::custom_highlight_background`] with the originating foreground
    /// for painting.
    pub custom_highlight_styles: HashMap<String, CssColor>,
    custom_highlight_sources: HashMap<String, smol_str::SmolStr>,
    /// Per-node computed values (indexed by NodeId.0 as usize).
    /// Populated for Element / Text / Document kinds; out-of-range access
    /// panics and is the caller's responsibility.
    ///
    /// The root element's entry carries its *used* `writing-mode`,
    /// `direction` and `text-orientation`, which are the principal
    /// element's (CSS Writing Modes 4 §8; see [`Self::principal_element_index`]),
    /// so that layout and paint read one set of values for the root. Its
    /// descendants and the `@page` cascade inherit from the root's own
    /// values, and [`Self::cssom_computed`] reports them.
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
    /// changing ordinary element layout semantics. The root element's entry
    /// is the principal element's when that element propagates its
    /// `writing-mode` to the root (see [`Self::principal_element_index`]).
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
    ///
    /// Values are held through [`PseudoValueArc`]. Identical complete sibling inputs may
    /// share the same allocation. Cloning a map value clones the handle. Clone the
    /// dereferenced [`ComputedValues`] for independent ownership, or use
    /// [`PseudoValueArc::make_mut`] for copy-on-write mutation.
    pub pseudo: HashMap<(StyleNodeId, PseudoElem), PseudoValueArc<ComputedValues>>,
    /// The measurer of `line-height: normal` the cascade ran with, kept for
    /// the first-letter styles resolved after it.
    normal_line_height: Option<crate::resolve::NormalLineHeightSource>,
    /// Output scale used to resolve border widths in this result.
    border_width_device_scale: Option<f32>,
}

/// The values of the root element that the principal element's flow replaces
/// in [`CascadeResult::computed`]: the flow itself, and the physical sizes
/// the root's own writing mode mapped its logical ones to.
#[derive(Clone, Debug)]
struct RootOwnFlow {
    writing_mode: WritingMode,
    cssom_writing_mode: WritingMode,
    direction: Direction,
    text_orientation: TextOrientation,
    width: ComputedLengthPercentageOrAuto,
    height: ComputedLengthPercentageOrAuto,
    vertical_logical_size: Option<VerticalLogicalSize>,
}

impl RootOwnFlow {
    fn of(values: &ComputedValues) -> Self {
        Self {
            writing_mode: values.writing_mode,
            cssom_writing_mode: values.cssom_writing_mode,
            direction: values.direction,
            text_orientation: values.text_orientation,
            width: values.width,
            height: values.height,
            vertical_logical_size: values.vertical_logical_size.clone(),
        }
    }

    /// Put the root's own values back into `values`.
    fn restore(&self, values: &mut ComputedValues) {
        values.writing_mode = self.writing_mode;
        values.cssom_writing_mode = self.cssom_writing_mode;
        values.direction = self.direction;
        values.text_orientation = self.text_orientation;
        values.width = self.width;
        values.height = self.height;
        values.vertical_logical_size = self.vertical_logical_size.clone();
    }
}

impl CascadeResult {
    /// The device pixels per CSS pixel the cascade snapped border widths to,
    /// `None` for an output-neutral or vector cascade, or for a scale that is
    /// not finite and positive, which the cascade ignores.
    pub fn border_width_device_scale(&self) -> Option<f32> {
        self.border_width_device_scale
            .filter(|scale| scale.is_finite() && *scale > 0.0)
    }

    /// The measurer of `line-height: normal` the cascade ran with.
    pub(crate) fn normal_line_height(&self) -> Option<&dyn crate::resolve::NormalLineHeight> {
        self.normal_line_height.as_ref().map(|source| &*source.0)
    }

    /// The `@page` cascade for `query`, inheriting from this result's root
    /// element and measuring `line-height: normal` with the measurer this
    /// cascade ran with.
    pub fn cascade_page_from_root(
        &self,
        rule_tree: &RuleTree,
        query: &PageContextQuery,
        media_context: &MediaContext,
    ) -> PageCascadeResult {
        cascade_page_with_normal_line_height(
            rule_tree,
            query,
            PageInheritance::FromRoot(&self.root_element_cssom_computed()),
            media_context,
            self.normal_line_height(),
            self.border_width_device_scale,
        )
    }

    /// The computed values of the margin box of `page` whose cascaded rule
    /// is `rule`, measuring `line-height: normal` with the measurer this
    /// cascade ran with. See [`PageCascadeResult::margin_box_style`].
    pub fn margin_box_style(
        &self,
        page: &PageCascadeResult,
        rule: &crate::PageMarginBoxCascadeResult,
    ) -> ComputedValues {
        page.margin_box_style(rule, self.normal_line_height())
    }

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
    /// Winning relative-unit SVG presentation attributes that need conversion
    /// to the declaring element's font size before an SVG renderer handles
    /// inheritance. Absolute presentation attributes remain in the SVG source.
    pub fn svg_relative_presentation_attributes(&self, node: StyleNodeId) -> &[SvgStyleProperty] {
        self.svg_relative_presentation_attributes
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

    /// Opaque identity for the cascade run that produced the element
    /// results: unlike [`Self::generation`], it survives
    /// [`Self::replace_page`], which changes only the page context.
    ///
    /// Layout caches that read only element results use it.
    #[doc(hidden)]
    pub fn element_generation(&self) -> u64 {
        self.element_generation
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
    /// [`Self::cascade_page_from_root`] instead of rerunning the whole cascade.
    ///
    /// Like [`Self::computed`], the values carry the used `writing-mode`,
    /// `direction` and `text-orientation` of the root element, the principal
    /// element's. The `@page` cascade inherits from the root's own values
    /// instead, and [`Self::cssom_computed`] reads them.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::computed`] was shrunk after the cascade produced it.
    pub fn root_element_computed(&self) -> &ComputedValues {
        &self.computed[self.root_element_index]
    }

    /// Index into [`Self::computed`] of the principal element, the element
    /// whose `writing-mode` and `direction` the root element takes as its
    /// used values (CSS Writing Modes 4 §8).
    ///
    /// That is the first `body` child of an HTML `html` root whose `display`
    /// is not `none`. Containment on the root or on that `body` stops the
    /// propagation (CSS Containment 2 §2), and a root of another kind has no
    /// principal child. The index is then the root element's own.
    pub fn principal_element_index(&self) -> usize {
        self.principal_element_index
    }

    /// The computed values of node `index` as CSSOM reports them.
    ///
    /// Equal to `computed[index]`, except for the root element when the
    /// principal element's `writing-mode`, `direction` or `text-orientation`
    /// replaced the root's own in [`Self::computed`]: the propagation changes
    /// the used values and leaves the computed ones alone (CSS Writing Modes
    /// 4 §8), which this returns. It is borrowed unless it has to put those
    /// back.
    ///
    /// # Panics
    ///
    /// Panics if `index` is outside [`Self::computed`].
    pub fn cssom_computed(&self, index: usize) -> std::borrow::Cow<'_, ComputedValues> {
        match &self.root_own_flow {
            Some(own) if index == self.root_element_index => {
                let mut values = self.computed[index].clone();
                own.restore(&mut values);
                std::borrow::Cow::Owned(values)
            }
            _ => std::borrow::Cow::Borrowed(&self.computed[index]),
        }
    }

    /// The `writing-mode` the root element computed to, when the principal
    /// element's mode replaced it in [`Self::computed`]; `None` when the
    /// root's entry holds it already.
    ///
    /// What inherits from the root element, other than the principal element
    /// and the text directly in the root, inherits this one, as the
    /// propagation changes the used value and leaves the computed one alone
    /// (CSS Writing Modes 4 §8).
    pub fn root_element_own_writing_mode(&self) -> Option<WritingMode> {
        self.root_own_flow
            .as_ref()
            .map(|own| own.cssom_writing_mode)
    }

    /// The computed values of the root element as CSSOM reports them, the
    /// inheritance parent of the `@page` cascade; see [`Self::cssom_computed`].
    ///
    /// # Panics
    ///
    /// Panics if [`Self::computed`] was shrunk after the cascade produced it.
    pub fn root_element_cssom_computed(&self) -> std::borrow::Cow<'_, ComputedValues> {
        self.cssom_computed(self.root_element_index)
    }

    /// The computed values of every node as CSSOM reports them, in place of
    /// [`Self::computed`]; see [`Self::cssom_computed`].
    pub fn into_cssom_computed(mut self) -> Vec<ComputedValues> {
        if let Some(own) = self.root_own_flow.take() {
            own.restore(&mut self.computed[self.root_element_index]);
        }
        self.computed
    }

    /// Replace the `@page` cascade with one for another page query.
    ///
    /// The element results are unchanged; `page` is expected to come from
    /// [`Self::cascade_page_from_root`] with the same [`RuleTree`] and
    /// [`MediaContext`]. The page context is a layout input, so the result
    /// gets a fresh [`Self::generation`] and generation-keyed caches treat it
    /// as a new cascade.
    pub fn replace_page(&mut self, page: PageCascadeResult) {
        self.page = page;
        self.generation = NEXT_CASCADE_GENERATION.fetch_add(1, Ordering::Relaxed);
    }

    /// The results for part of the tree, renumbered: node `i` of the new
    /// result is node `old_of_new[i]` of this one. Every index in
    /// `old_of_new` must be in range.
    ///
    /// This pairs with a document copy that keeps only some nodes; the
    /// result gets a fresh [`Self::generation`]. When the `@page` inheritance
    /// parent is not kept, the new result's is its first node.
    pub fn extract(&self, old_of_new: &[usize]) -> Self {
        // Every field is named so that a new one has to be considered here.
        let Self {
            generation: _,
            element_generation: _,
            svg_style_properties,
            svg_relative_presentation_attributes,
            first_letter_inputs,
            first_line_contexts,
            typographic_inheritance,
            root_element_index,
            principal_element_index,
            root_own_flow,
            custom_highlight_styles,
            custom_highlight_sources,
            computed,
            opacity_specified,
            background_color_specified,
            authored_writing_modes,
            page,
            counter_styles,
            page_values,
            pseudo,
            normal_line_height,
            border_width_device_scale,
        } = self;
        let new_of_old: HashMap<u64, u64> = old_of_new
            .iter()
            .enumerate()
            .map(|(new, &old)| (old as u64, new as u64))
            .collect();
        let node = |id: &StyleNodeId| new_of_old.get(&id.0).map(|&new| StyleNodeId(new));
        let root_kept = new_of_old.contains_key(&(*root_element_index as u64));
        let new_root = new_of_old
            .get(&(*root_element_index as u64))
            .map_or(0, |&new| new as usize);
        let generation = NEXT_CASCADE_GENERATION.fetch_add(1, Ordering::Relaxed);
        Self {
            generation,
            element_generation: generation,
            svg_style_properties: svg_style_properties
                .iter()
                .filter_map(|(id, value)| Some((node(id)?, value.clone())))
                .collect(),
            svg_relative_presentation_attributes: svg_relative_presentation_attributes
                .iter()
                .filter_map(|(id, value)| Some((node(id)?, value.clone())))
                .collect(),
            first_letter_inputs: first_letter_inputs
                .iter()
                .filter_map(|(id, value)| Some((node(id)?, value.clone())))
                .collect(),
            first_line_contexts: first_line_contexts
                .iter()
                .filter_map(|(id, context)| Some((node(id)?, *context)))
                .collect(),
            typographic_inheritance: typographic_inheritance
                .iter()
                .filter_map(|((id, pseudo), value)| Some(((node(id)?, *pseudo), value.clone())))
                .collect(),
            root_element_index: new_root,
            // A root that is kept keeps the flow it was given, whether or not
            // the principal element is kept with it, and stands in for it
            // otherwise. A root that was left out takes nothing.
            principal_element_index: if root_kept {
                new_of_old
                    .get(&(*principal_element_index as u64))
                    .map_or(new_root, |&new| new as usize)
            } else {
                new_root
            },
            root_own_flow: root_own_flow.clone().filter(|_| root_kept),
            custom_highlight_styles: custom_highlight_styles.clone(),
            custom_highlight_sources: custom_highlight_sources.clone(),
            computed: renumber(computed, old_of_new),
            opacity_specified: renumber(opacity_specified, old_of_new),
            background_color_specified: renumber(background_color_specified, old_of_new),
            authored_writing_modes: renumber(authored_writing_modes, old_of_new),
            page: page.clone(),
            counter_styles: counter_styles.clone(),
            page_values: renumber(page_values, old_of_new),
            pseudo: pseudo
                .iter()
                .filter_map(|((id, kind), value)| Some(((node(id)?, *kind), value.clone())))
                .collect(),
            normal_line_height: normal_line_height.clone(),
            border_width_device_scale: *border_width_device_scale,
        }
    }
}

/// Index of the element whose `writing-mode` and `direction` the root
/// element takes: the first HTML `body` child of an `html` root whose
/// `display` is not `none`, else the root (CSS Writing Modes 4 §8, principal
/// writing mode). Containment on the root or on that `body` stops the
/// propagation (CSS Containment 2 §2).
fn find_principal_element<D: StyleDom>(dom: &D, computed: &[ComputedValues], root: usize) -> usize {
    const XHTML: &str = "http://www.w3.org/1999/xhtml";
    let is_html = |id: StyleNodeId, name: &str| {
        dom.node(id)
            .and_then(|node| {
                node.as_element().map(|e| {
                    (
                        e.tag_name().eq_ignore_ascii_case(name),
                        e.namespace_uri().is_none_or(|ns| ns == XHTML),
                    )
                })
            })
            .is_some_and(|(named, html)| named && html)
    };
    // An index outside the arena has no values to read, so it never propagates.
    let contained = |index: usize| {
        computed
            .get(index)
            .is_none_or(|v| v.contain != Contain::NONE)
    };
    let root_id = StyleNodeId::new(root as u64);
    if !is_html(root_id, "html") || contained(root) {
        return root;
    }
    dom.child_ids(root_id)
        .map(|id| id.0 as usize)
        .find(|&index| {
            is_html(StyleNodeId::new(index as u64), "body")
                && computed
                    .get(index)
                    .is_some_and(|values| values.display != DisplayValue::None)
        })
        .filter(|&body| !contained(body))
        .unwrap_or(root)
}

/// Give the root element the principal element's used `writing-mode`,
/// `direction` and `text-orientation`. Returns what the root had of its own
/// when any of them differs, for CSSOM and the `@page` cascade, which keep
/// the root's own values (CSS Writing Modes 4 §8: the used values change, not
/// the computed ones).
///
/// The root's authored `writing-mode` takes the principal's too, as the
/// authored value is what layout resolves the used writing mode of a node
/// from. Its physical sizes stay what its own writing mode mapped them to.
fn propagate_principal_flow(
    outputs: &mut WalkOutputs,
    root: usize,
    principal: usize,
) -> Option<RootOwnFlow> {
    if root == principal {
        return None;
    }
    let (writing_mode, cssom_writing_mode, direction, text_orientation) = {
        let values = &outputs.computed[principal];
        (
            values.writing_mode,
            values.cssom_writing_mode,
            values.direction,
            values.text_orientation,
        )
    };
    let authored =
        outputs.authored_writing_modes[principal].or(outputs.authored_writing_modes[root]);
    let own = &outputs.computed[root];
    if own.writing_mode == writing_mode
        && own.cssom_writing_mode == cssom_writing_mode
        && own.direction == direction
        && own.text_orientation == text_orientation
        && authored == outputs.authored_writing_modes[root]
    {
        return None;
    }
    let own = RootOwnFlow::of(own);
    let used = &mut outputs.computed[root];
    used.writing_mode = writing_mode;
    used.cssom_writing_mode = cssom_writing_mode;
    used.direction = direction;
    used.text_orientation = text_orientation;
    // `inline-size` and `block-size` map onto the physical axes by the root's
    // own writing mode. Where that one is vertical, the mapping is kept
    // beside the horizontal one that layout reads for a horizontal used mode.
    if !cssom_writing_mode.is_vertical()
        && let Some(size) = used.vertical_logical_size.take()
    {
        used.width = size.width;
        used.height = size.height;
    }
    outputs.authored_writing_modes[root] = authored;
    Some(own)
}

/// `values[old_of_new[i]]` for each `i`.
fn renumber<T: Clone>(values: &[T], old_of_new: &[usize]) -> Vec<T> {
    old_of_new.iter().map(|&old| values[old].clone()).collect()
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
/// Invalid CSS is silently dropped while the rule tree is built, so it never
/// makes the cascade fail. The cascade runs within the default
/// [`CascadeLimits`]; [`cascade_with_options`] takes others.
///
/// # Errors
///
/// See [`cascade_with_options`].
///
/// # Example
///
/// ```ignore
/// use raikiri_style::{build_rule_tree, cascade, ComputedValues};
///
/// # fn demo<D: raikiri_style::StyleDom>(dom: &D) {
/// let rule_tree = build_rule_tree(dom);
/// let result = cascade(dom, &rule_tree).expect("the document is within the default limits");
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
///
/// # Errors
///
/// See [`cascade_with_options`].
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
///
/// # Errors
///
/// See [`cascade_with_options`].
pub fn cascade_with_media_context_for_page<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
) -> Result<CascadeResult, CascadeError> {
    cascade_with_options(
        dom,
        rule_tree,
        media_context,
        page_query,
        &CascadeOptions::default(),
    )
}

/// Run the element and `@page` cascades for one page-context query, as
/// [`cascade_with_media_context_for_page`] does, within `options.limits`.
///
/// # Errors
///
/// - [`CascadeError::LimitExceeded`] when the document and its stylesheets
///   need more of the cascade's work or of its result's memory than
///   `options.limits` allows. The counts depend only on the input, so the same
///   input fails the same way every time, before the cascade allocates what
///   the count stands for, or, for the heap one node's values hold, once that
///   node is resolved.
/// - [`CascadeError::ResourceExhausted`] when the allocator refuses one of the
///   result's buffers.
/// - [`CascadeError::Internal`] when the stylesheets have more rules,
///   selectors or declarations than the cascade can number.
pub fn cascade_with_options<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
    options: &CascadeOptions,
) -> Result<CascadeResult, CascadeError> {
    let normal_line_height = options.normal_line_height_for(dom);
    let measurer = normal_line_height.as_ref().map(|source| &*source.0);
    let outputs = walk(
        dom,
        rule_tree,
        media_context,
        WalkOptions {
            limits: options.limits,
            viewport: options.viewport,
            border_width_device_scale: options.border_width_device_scale,
            normal_line_height: measurer,
            ..WalkOptions::default()
        },
    )?;
    let mut result = finish(
        dom,
        rule_tree,
        page_query,
        media_context,
        outputs,
        measurer,
        options.border_width_device_scale,
    );
    result.normal_line_height = normal_line_height;
    Ok(result)
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
    mut outputs: WalkOutputs,
    normal_line_height: Option<&dyn crate::resolve::NormalLineHeight>,
    border_width_device_scale: Option<f32>,
) -> CascadeResult {
    let root_element_index = page_inheritance_root_index(dom, outputs.computed.len());
    let page = cascade_page_with_normal_line_height(
        rule_tree,
        page_query,
        PageInheritance::FromRoot(&outputs.computed[root_element_index]),
        media_context,
        normal_line_height,
        border_width_device_scale,
    );
    // The page context and every descendant have inherited from the root's
    // own values by now; only the root's entry takes the principal flow.
    let principal_element_index =
        find_principal_element(dom, &outputs.computed, root_element_index);
    let root_own_flow =
        propagate_principal_flow(&mut outputs, root_element_index, principal_element_index);
    let generation = NEXT_CASCADE_GENERATION.fetch_add(1, Ordering::Relaxed);
    CascadeResult {
        generation,
        element_generation: generation,
        svg_style_properties: outputs.svg_properties,
        svg_relative_presentation_attributes: outputs.svg_relative_presentation_attributes,
        first_letter_inputs: outputs.first_letter_inputs,
        first_line_contexts: outputs.first_line_contexts,
        typographic_inheritance: outputs.typographic_inheritance,
        root_element_index,
        principal_element_index,
        root_own_flow,
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
        normal_line_height: None,
        border_width_device_scale,
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
mod list_hints;
mod svg_hints;
mod table_hints;
pub(crate) use custom_property::*;
mod first_letter;
mod first_line;
pub use first_line::{
    FirstLineCascade, FirstLineStyles, cascade_with_first_line, cascade_with_first_line_options,
};
mod inherit;
pub(crate) use inherit::*;
pub(crate) mod keyword_defaults;
mod limits;
mod value_heap;
pub(crate) use limits::Counter;
pub use limits::{CascadeLimits, CascadeOptions};

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;
