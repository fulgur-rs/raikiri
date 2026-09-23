use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use smol_str::SmolStr;

use crate::Atom;
use crate::cascade::{
    ResolvedAgainstInherited, cascade_rank, resolve_against_inherited,
    resolve_custom_property_environment, resolve_deferred_value, resolve_relative_font_size,
};
use crate::computed::{ComputedValues, CustomPropertyEnvironment, empty_custom_properties};
#[allow(unused_imports)]
use crate::property::{
    BackgroundShorthand, BackgroundSize, Border, BorderCollapseValue, BorderColor, BorderRadius,
    BorderSpacingValue, BorderStyle, BoxShadowItem, CaptionSideValue, ColumnCountValue,
    ColumnWidthValue, ColumnsShorthand, CssPosition, CssPositionOffset, CustomProperty,
    EmptyCellsValue, FlexBasisValue, FlexFlow, FlexShorthand, FontShorthand, FontShorthandSize,
    GapShorthand, GridInflexibleBreadth, GridTemplateTracks, GridTrackBreadth, GridTrackList,
    GridTrackListComponent, GridTrackRepeat, GridTrackSize, Length, LengthOrAuto, LengthOrNormal,
    Outline, OutlineColor, OutlineStyle, OverflowValue, OverflowXY, PageValue, PropertyKey,
    PropertyValue, Sides, TableLayoutValue, TextCombineUpright, TextDecorationInset,
    TextDecorationShorthand, TextDecorationSkipInk, TextDecorationSkipSpaces,
    TextDecorationThickness, TextEmphasisHEdge, TextEmphasisPosition, TextEmphasisVEdge,
    TextIndentValue, TextOrientation, TextShadowItem, TextUnderlinePosition, TextWrapMode,
    TransformFunction, UnicodeBidi, parse_length_allow_negative, parse_non_negative_length,
    parse_value, resolve_overflow, resolve_writing_mode,
};
use crate::resolve::{
    ComputedBackgroundSize, ComputedCssPositionOffset, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackList,
    ComputedGridTrackListComponent, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ResolveContext, lift_border_spacing, lift_length_or_normal, lift_line_height, lift_tab_size,
    resolve_background_size, resolve_border, resolve_border_radius, resolve_border_spacing,
    resolve_box_shadow_item, resolve_css_position, resolve_flex_basis,
    resolve_grid_auto_track_list, resolve_grid_template_tracks, resolve_length,
    resolve_length_or_normal, resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_line_height, resolve_margin_length_or_auto,
    resolve_outline, resolve_tab_size, resolve_vertical_align, used_line_height_length,
};
use crate::rule::{Declaration, expand_shorthand_into};
use crate::ruletree::{Origin, RuleTree};
use crate::specified::INITIAL_BORDER;

use super::types::*;

// ---------------------------------------------------------------------------
// @page cascade order 本実装
//
// CSS Paged Media Level 3, §"Cascading and page context" —
//   <https://www.w3.org/TR/css-page-3/#cascading-and-page-context>
// CSS Cascading and Inheritance Level 4, §"Cascade Origin" —
//   <https://www.w3.org/TR/css-cascade-4/#cascade-origin>
//
// # Sibling arm convention
//
// Follows the sibling convention established by
// `crate::cascade::collect_cascaded` + `crate::cascade::pick_winners`:
// per-candidate `(value, important, origin, specificity, source_order)` tuple,
// group by `PropertyKey`, pick winner by `(rank, specificity, source_order)`
// where higher tuples beat lower. `rank` reuses `cascade_rank` verbatim —
// `@page` rules and style rules share the same origin ordering (spec §6.2).
// The only diverging element is the specificity type: `PageSpecificity` is a
// derived-`Ord` `(f, g, h)` triple per L3 §"Cascading and page context",
// whereas style rules use the `selectors` crate's 32-bit packed specificity.
// ---------------------------------------------------------------------------

/// Query describing which `@page` selectors apply to the current page.
///
/// The consumer (raikiri umbrella, dom-level page loop) supplies this bag
/// per page it is about to lay out. All fields are declarative — the caller
/// pre-computes `is_left` / `is_right` from `page_index` parity, `is_first`
/// from the page number, and `is_blank` from the fragmentation state
/// (design specification §9.1 "page name 遷移ルール"). raikiri-style does
/// not know about page indexes, only about which pseudo-page states are
/// currently true.
///
/// Fields default to `None` / `false`, i.e. an unnamed page with no
/// pseudo-page state — matches only `@page { … }`.
///
/// `#[non_exhaustive]`: future pseudo-pages (e.g. spec-outside extensions)
/// or additional context (media query state, forced-orientation flags) can be
/// added without a semver break. Consumers construct via
/// `PageContextQuery { page_name: …, is_first: …, ..Default::default() }` per
/// the standard `#[non_exhaustive]` pattern used throughout raikiri-style.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageContextQuery {
    /// Named-page ident from the `page:` property. `None` = unnamed page
    /// (only `@page` and `@page :<pseudo>` rules can match).
    ///
    /// Compared case-sensitively per CSS Values L4
    /// [`#custom-idents`](https://www.w3.org/TR/css-values-4/#custom-idents)
    /// `<custom-ident>` — "fully case-sensitive … even in the ASCII range"
    /// (mirrors the parse-time invariant on [`PageSelectorEntry::ident`]).
    pub page_name: Option<Atom>,
    /// Whether this is the first page (`:first` matches).
    pub is_first: bool,
    /// Whether this is a left / verso page (`:left` matches).
    pub is_left: bool,
    /// Whether this is a right / recto page (`:right` matches).
    pub is_right: bool,
    /// Whether this is a blank page inserted by a forced page break
    /// (`:blank` matches).
    pub is_blank: bool,
}

/// Cascaded declarations belonging to one page-margin box slot.
///
/// The declaration list is kept in source order. The page-context cascade and
/// the margin-context inheritance/used-value pass are separate operations, so
/// this type deliberately carries one parsed nested at-rule rather than
/// pretending that a margin box is an ordinary element node.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct PageMarginBoxCascadeResult {
    /// The margin-box slot named by the nested at-rule.
    pub slot: PageMarginBoxSlot,
    /// Declarations from this nested margin-box rule, in source order.
    pub declarations: Vec<Declaration>,
    /// Enclosing `@page` rule source order.
    pub source_order: u32,
    /// Enclosing `@page` rule cascade origin.
    pub origin: Origin,
    /// Enclosing page-selector specificity `(f, g, h)`.
    pub specificity: (u32, u32, u32),
}

/// Winning values from an `@page` cascade pass.
///
/// Ordinary declarations are exposed through [`Self::declarations`]. The
/// dedicated `size`, `marks`, and `bleed` descriptors are cascaded with the
/// same origin, page-selector specificity, importance, and source-order rules
/// and are exposed through their typed accessors. Matching margin-box at-rules
/// are exposed in source order; resolving their inherited computed values and
/// used geometry remains a downstream layout operation.
///
/// The ordinary box properties are **not** in that category: `margin` /
/// `padding` / `border-*` / `width` / `height` are parsed (the `margin`
/// shorthand is expanded to longhands) and go through both resolution phases
/// below. Downstream page-layout code consumes the typed descriptor accessors
/// for the concrete page box; margin-box inheritance and used geometry remain
/// downstream responsibilities.
///
/// Iteration order over `declarations` is `HashMap`-random; consumers that
/// need a deterministic order should sort or look up by [`PropertyKey`]
/// (Tests here look up by key rather than iterating).
/// Custom-property declarations are cascaded separately by their
/// case-sensitive names and are used only while resolving `var()`. They are
/// never emitted under `PropertyKey::Custom`, and a winning deferred value is
/// either fully reparsed/projected before emission or omitted as invalid at
/// computed-value time; this map never exposes raw specified-layer values.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageCascadeResult {
    // Private so the "resolved against the inheritance parent" contract
    // documented on `declarations()` is enforced by construction: only
    // `cascade_page` can populate this map. Read-only
    // access is public API — see the `declarations()` accessor below, in
    // particular its "Why this is a method, not a field" section.
    declarations: HashMap<PropertyKey, PropertyValue>,
    /// Winning `size` descriptor, if one of the matching rules declared it.
    size: Option<PageSize>,
    /// Winning `marks` descriptor, if one of the matching rules declared it.
    marks: Option<PageMarks>,
    /// Winning `bleed` descriptor, if one of the matching rules declared it.
    bleed: Option<PageBleed>,
    /// Matching margin-box at-rules in source order.
    margin_boxes: Vec<PageMarginBoxCascadeResult>,
}

impl PageCascadeResult {
    /// Winning `(key, value)` per property, after the two resolution passes
    /// described below.
    ///
    /// This accessor returns the computed declaration bag produced by the page
    /// cascade. CSS Paged Media Level 3 §6 requires a computed value for every
    /// property in the page context.
    ///
    /// **These are computed values, with no documented exception** (see
    /// the phase 2 bullet below for the property
    /// that used to be the one exception). CSS Paged Media 3 §6 "Page Properties"
    /// (<https://www.w3.org/TR/css-page-3/#page-properties>) states that "both
    /// the page context and the margin context have a computed value for every
    /// property" and that "The page context inherits from the root element";
    /// the inheritance parent is the [`PageInheritance`] argument of
    /// [`cascade_page`].
    ///
    /// **Phase 2** — resolution against the inheritance parent
    /// ([`crate::cascade::resolve_against_inherited`]):
    ///
    /// - `font-weight` — `bolder` / `lighter` are resolved against the
    ///   inherited weight, so **no relative font-weight sentinel** reaches the
    ///   consumer.
    /// - `font-size` — `em` / `rem` / `%` are absolutized against the root
    ///   element's computed font-size, so the value is always
    ///   [`Length::Px`]. §6 verbatim: "When used on
    ///   the font-size property in the page context, they are relative to the
    ///   font-size of the root element." `larger` /
    ///   `smaller` (`<relative-size>`) are resolved the same way against the
    ///   root element's computed font-size, so **no relative font-size
    ///   sentinel** reaches the consumer either — same guarantee as the
    ///   `font-weight` bullet above.
    /// - `text-align` — [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent)
    ///   is resolved against the inheritance parent's computed `text-align`
    ///   **and** `direction`. CSS Text 3 §6.1
    ///   `#valdef-text-align-match-parent`
    ///   (<https://www.w3.org/TR/css-text-3/#valdef-text-align-match-parent>)
    ///   verbatim: "This value behaves the same as inherit (computes to its
    ///   parent's computed value) except that an inherited value of start or
    ///   end is interpreted against the parent's direction value and results
    ///   in a computed value of either left or right." Implementation:
    ///   [`crate::property::resolve_text_align_match_parent`], shared with the
    ///   element path's [`crate::specified::SpecifiedValues::finalize`].
    ///
    ///   ⚠️ **Trap avoided here, stated for anyone touching this arm again**:
    ///   the same value definition's second sentence — "Computes to `start`
    ///   when specified on the root element" — does **not** apply to the page
    ///   context. §6 says "The page context *inherits from* the root
    ///   element" — inheriting from the root element does not make the page
    ///   context *be* the root element, so `@page { text-align: match-parent }`
    ///   is always resolved via the parent-direction table, never
    ///   short-circuited to `start`. (The element path's analogous
    ///   short-circuit lives in
    ///   [`crate::specified::SpecifiedValues::finalize_as_root`], which is a
    ///   different entry point entirely — `cascade_page` never calls it.)
    ///
    /// **Phase 3** — absolutization against the page context's *own*
    /// `font-size` (`absolutize_in_page_context`):
    ///
    /// - `padding` / `margin` / `width` / `height` / `border-*-width` /
    ///   `line-height` — [`Length::Em`] / `Rem` /
    ///   `Pt` become [`Length::Px`]. `em` is
    ///   relative to "the font associated with their context" (§6 above), which
    ///   is the page context's own `font-size` — possibly declared by a sibling
    ///   `@page` declaration, hence a phase of its own. `rem` stays relative to
    ///   the root element (CSS Values 4 §6.1.1
    ///   <https://www.w3.org/TR/css-values-4/#rem>).
    /// - `border-*-width` is additionally **gated to `0px` when the side's
    ///   computed `border-*-style` is `none` or `hidden`** — CSS Backgrounds 3
    ///   §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width> makes that
    ///   part of the *computed* value ("Computed value: absolute length, snapped
    ///   as a border width; zero if the border style is `none` or `hidden`").
    ///   An **undeclared** `border-*-style` counts as its initial value `none`
    ///   — CSS Backgrounds 3 §3.2
    ///   <https://www.w3.org/TR/css-backgrounds-3/#border-style> propdef gives
    ///   `border-*-style`'s `Initial: none`, and §6 (a *different* sentence
    ///   from the ones quoted above for `font-size` and `em`) is why that
    ///   initial value is there to begin with even though the property
    ///   is undeclared — verbatim: "both the page context and the margin
    ///   context have a computed value for every property, even if that
    ///   property does not apply to the page or page-margin box." So
    ///   `@page { border-top-width: 5px }` alone computes to `0px`, matching
    ///   the element path.
    /// - `overflow-x` / `overflow-y` apply the CSS
    ///   Overflow 3 §3.1 cross-axis coupling
    ///   ([`crate::property::resolve_overflow`]) the same way the element
    ///   path does, with one representational gap this map does not close:
    ///   the coupling can rewrite an axis based on the *other* axis's value
    ///   even when the other axis is **undeclared** (e.g. `@page { overflow-x:
    ///   hidden }` alone should compute `overflow-y` to `auto`, not its raw
    ///   initial `visible`, on the element path). Because this map only
    ///   contains *declared* properties (see the "not a full computed-value
    ///   bag" note below), an undeclared `overflow-y` never becomes a
    ///   `PropertyKey::OverflowY` entry here, so that computed value is not
    ///   representable in this map at all — unlike the `border-*-width` /
    ///   `border-*-style` gate above, which only ever needs the *declared*
    ///   property's own computed value plus an *undeclared* input's initial
    ///   value (never the reverse). `overflow`/`overflow-x`/`overflow-y` are
    ///   **not** in CSS Paged Media 3 Appendix A's page-property-list
    ///   (checked directly against the raw Appendix A table) — same as
    ///   [`crate::property::DisplayValue`], [`crate::property::PositionValue`],
    ///   `box-sizing`, `counter-reset`/`counter-increment`, `content`,
    ///   `string-set`, and [`crate::property::WritingMode`] (`writing-mode`),
    ///   all of which this crate already wires into the page
    ///   cascade beyond Appendix A's CSS 2.1 floor (§6's wording is a
    ///   positive minimum, not a ceiling, and does not prohibit extending
    ///   further). So this integration is intentional, not a scope question.
    ///   The representational gap itself remains open, tracked separately:
    ///   closing it needs [`page_context_overflow_pair`]
    ///   (or its caller) to synthesize the missing axis's entry rather than
    ///   silently omitting it.
    /// - `<percentage>` on `padding` / `margin` / `width` / `height` **stays**
    ///   [`Length::Percent`]. The governing rule is the general one — CSS
    ///   Values 4 §5.5.1 "Computation and Combination of `<percentage>`"
    ///   (<https://www.w3.org/TR/css-values-4/#combine-percentages>): "Unless
    ///   otherwise specified (such as in font-size, which computes its
    ///   `<percentage>` values to `<length>`), the computed value of a
    ///   percentage is the specified percentage." For `padding` / `margin`
    ///   specifically, §6 adds that it is relative to "the dimensions of the
    ///   containing block", i.e. a used-value input (§6 treats `width` /
    ///   `height` in a *separate* sentence about used-value computation rules,
    ///   so do not attribute those two to the margin/padding sentence). Either
    ///   way that is the computed value, not an unresolved one; the element
    ///   path resolves it the same way
    ///   ([`crate::resolve::resolve_length_percentage`]).
    ///
    ///   ⚠️ **`font-size` and `line-height` are the opposite case** — their
    ///   `<percentage>` does *not* survive into the computed layer (§5.5.1's
    ///   own parenthetical names `font-size`; CSS Inline 3 §5.1
    ///   <https://www.w3.org/TR/css-inline-3/#line-height-property> gives
    ///   `line-height` "Percentages: computed relative to 1em"). They are
    ///   handled by phase 2 and phase 3 respectively.
    ///
    /// See [`crate::cascade::resolve_against_inherited`] and
    /// [`absolutize_in_page_context`] for the exact per-phase contracts.
    ///
    /// **This map is not a full computed-value bag.** It contains only the
    /// properties that some matching `@page` rule actually *declared*;
    /// properties whose value would come purely from inheritance or from the
    /// initial value are **absent**. The spec sentence quoted above ("a computed
    /// value for every property") describes the page context as a whole, not
    /// this map — materialising the complete bag is the downstream page-layout
    /// consumer's job, and it starts from the resolved [`PageInheritance`]
    /// plus these declarations.
    ///
    /// # Non-finite values pass through unguarded
    ///
    /// The absolutization in phase 2 / phase 3 above is IEEE 754 `f32`
    /// arithmetic over untrusted author input. CSS Values 4 §5 "Numeric Data
    /// Types" (<https://www.w3.org/TR/css-values-4/#numeric-types>) requires
    /// out-of-range values to be "converted to the closest value supported by
    /// the implementation" — it does **not** require the result to be finite.
    /// A declaration such as `html { font-size: 0px }` with
    /// `@page { font-size: 1e40em }` overflows the literal `1e40` to
    /// `f32::INFINITY` at parse time; phase 2's `parent_font_size.0 * v`
    /// (`0.0 * inf`) then yields `f32::NAN` per IEEE 754. See
    /// `page::tests::cascade_page_font_size_can_carry_nan_from_pathological_em`
    /// for the pinned reproducer.
    ///
    /// That `1e40em` example's *parse-time* overflow-to-`f32::INFINITY` step
    /// is itself disputed: it is an open question whether CSS Values 4 §5's
    /// "closest value" wording may require
    /// saturating to `f32::MAX` instead, which is a question about
    /// cssparser's `f64`→`f32` cast, not about this crate. The hazard this
    /// section documents does not depend on how that resolves: two already
    /// finite operands can still overflow to infinity on the `*` in phase 2
    /// alone (e.g. a `1e30px` root font-size times a `1e20em` page
    /// declaration), with no contested cast anywhere in the chain.
    ///
    /// **This map does not filter that out**, and neither does
    /// [`ComputedValues`] — the element path's equivalent computed-value bag —
    /// which documents no finiteness contract either. That is not an
    /// oversight: `raikiri-style` applies no `is_finite` / `is_nan` check
    /// anywhere in parsing, cascade, or resolution (grep the crate to
    /// confirm), and the element path's guard against non-finite geometry
    /// lives entirely outside this crate, in `raikiri-dom`'s layout module
    /// (`sanitize_taffy`, per a deliberate design decision:
    /// **the guard belongs at the sink that consumes the computed-value bag,
    /// not at the parse/resolve layer that produces it**). This map is the
    /// same kind of computed-value bag, so the same precedent applies to it.
    ///
    /// Any downstream consumer that converts these values to geometry must
    /// apply the appropriate finite-value and layout-safety checks at that
    /// sink; this layer preserves the computed result without filtering it.
    ///
    /// # Why this is a method, not a field
    ///
    /// Keeping the map private prevents callers from inserting unresolved
    /// specified-layer values without going through [`cascade_page`]. The
    /// accessor exposes the resolved declarations read-only.
    ///
    /// ## Compile-fail check
    ///
    /// "The field is private" is a claim about what does *not* compile, which
    /// no ordinary (must-pass) doctest can check — the same gap was
    /// closed for `Declaration::value` /
    /// `StyleRule::declarations` / `RuleTree::style_rules` at the same time.
    /// `PageCascadeResult::default()` returns an owned, mutable value (the
    /// type derives [`Default`]), so unlike `Declaration::value` (reached only
    /// through a `&Declaration` behind an accessor) this needs no `.clone()`
    /// detour to discriminate the field's own visibility — direct field
    /// access on an owned binding is exactly what `pub`/private gates, with no
    /// second field's visibility able to confound the result:
    ///
    /// ```compile_fail
    /// use raikiri_style::{CssColor, PageCascadeResult, PropertyKey, PropertyValue};
    ///
    /// let mut r = PageCascadeResult::default();
    /// r.declarations.insert(PropertyKey::Color, PropertyValue::Color(CssColor::BLACK));
    /// ```
    ///
    /// ### Accessor control
    ///
    /// The following doctest exercises the public accessor independently of
    /// the compile-fail privacy check.
    ///
    /// ```
    /// use raikiri_style::{
    ///     CssColor, Origin, PageContextQuery, PageInheritance, PropertyKey, PropertyValue,
    ///     RuleTree, cascade_page,
    /// };
    ///
    /// let mut tree = RuleTree::empty();
    /// tree.add_stylesheet("@page { color: red }", Origin::Author);
    /// let result = cascade_page(
    ///     &tree,
    ///     &PageContextQuery::default(),
    ///     PageInheritance::LegacyInitialValues,
    /// );
    /// assert_eq!(
    ///     result.declarations().get(&PropertyKey::Color),
    ///     Some(&PropertyValue::Color(CssColor {
    ///         r: 255,
    ///         g: 0,
    ///         b: 0,
    ///         a: 255
    ///     })),
    /// );
    /// // Ingredient used by the compile_fail fence above, confirmed still
    /// // valid here.
    /// let _ = PropertyValue::Color(CssColor::BLACK);
    /// ```
    pub fn declarations(&self) -> &HashMap<PropertyKey, PropertyValue> {
        &self.declarations
    }

    /// The winning `size` descriptor for this page context, if declared.
    pub fn size(&self) -> Option<PageSize> {
        self.size
    }

    /// The winning `marks` descriptor for this page context, if declared.
    pub fn marks(&self) -> Option<PageMarks> {
        self.marks
    }

    /// The winning `bleed` descriptor for this page context, if declared.
    pub fn bleed(&self) -> Option<PageBleed> {
        self.bleed
    }

    /// Matching margin-box declarations, in source order.
    pub fn margin_boxes(&self) -> &[PageMarginBoxCascadeResult] {
        &self.margin_boxes
    }
}

/// Selects the page context's inheritance parent for [`cascade_page`] — CSS
/// Paged Media 3 §6 "Page Properties"
/// (<https://www.w3.org/TR/css-page-3/#page-properties>) states: "As with
/// elements in the document, both the page context and the margin context have
/// a computed value for every property … The normal rules for CSS properties
/// apply with the following exceptions: page-margin boxes inherit from the page
/// context. The page context inherits from the root element."
///
/// The named variants make the choice between the root element and the L3
/// legacy initial-values exception explicit at each invocation.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum PageInheritance<'a> {
    /// Resolve against the root element's own [`ComputedValues`] — the §6
    /// rule quoted above. **Prefer this variant** wherever a root style is
    /// available:
    ///
    /// ```ignore
    /// // ignore: `dom` / `rule_tree` / `query` are the caller's real values,
    /// // not constructible in a doc-test in isolation; shape only, see
    /// // `cascade_page`'s own doc-tests for a version that actually runs.
    /// let cascaded = cascade(dom, rule_tree)?;
    /// let result = cascade_page(
    ///     rule_tree,
    ///     &query,
    ///     PageInheritance::FromRoot(&cascaded.computed[dom.root_id().0 as usize]),
    /// );
    /// ```
    FromRoot(&'a ComputedValues),
    /// The L3 *legacy exception* the same §6 paragraph grants: "since the
    /// previous revision of CSS Paged Media Level 3 did not specify this
    /// point, an implementation that sets inherited properties in the page
    /// context to their initial values (as for the root element) is also
    /// conformant to CSS Paged Media Level 3." Resolves against
    /// [`ComputedValues::initial()`] — for `font-weight` that is `400`, not
    /// the root element's computed weight.
    ///
    /// The spec states this exception will be removed in Level 4. Choosing
    /// this variant where a root style could have been supplied instead
    /// silently resolves `bolder` / `lighter` against `400` instead of the
    /// root's weight. Since phase 3 is wired through the
    /// same `font-size` basis, it also affects `em` / `rem` lengths in the
    /// page box (`padding` / `margin` / `width` / `height` /
    /// `border-*-width` / `line-height`) — but not uniformly: the `rem`
    /// basis is always the initial `16px` under this variant, and so is the
    /// `em` basis *only when the page context declares no `font-size` of its
    /// own* ([`page_context_font_size`]). An explicit
    /// `@page { font-size: … }` still determines the `em` basis regardless
    /// of this variant (`cascade_page_padding_em_uses_own_font_size_over_inherited`
    /// pins this).
    LegacyInitialValues,
}

/// Cascade all `@page` rules in `rule_tree` against `query` and return the
/// resolved winning declarations.
///
/// # Algorithm (spec §"Cascading and page context" + §"Cascade Origin")
///
/// 1. For each rule in [`RuleTree::page_rules`], find the highest-specificity
///    matching entry in its comma-separated `<page-selector-list>` prelude
///    (each entry is an OR alternative; the entries within an entry — an
///    optional ident + zero-or-more pseudo-pages — are combined AND-wise).
///    A rule contributes its declarations if any entry matched, tagged with
///    that entry's `(f, g, h)` specificity triple (see the crate-internal
///    `PageSpecificity` type for the exact shape).
/// 2. Group candidate declarations by [`PropertyKey`], pick the winner per
///    `(rank, specificity, source_order)` tuple where higher beats lower.
///    `rank` comes from the crate-internal `cascade_rank` (shared with the
///    style-rule cascade) and encodes the full L4 §6.2 / L5 §6.5 origin
///    ordering across all 4 [`Origin`] variants — see that function's doc
///    for the exact rank table, which this module reuses rather than
///    duplicating.
/// 3. **Phase 2** — resolve each winner against the page context's inheritance
///    parent ([`PageInheritance`]) through the crate-internal
///    [`crate::cascade::resolve_against_inherited`], the sibling of
///    [`crate::cascade::apply_value`] that
///    keeps the `PropertyValue` shape. This covers the two properties that are
///    determined by the inheritance parent alone: `font-weight: bolder` /
///    `lighter` (CSS Fonts 4 §2.2.1 "Relative Weights") and `font-size`.
/// 4. **Phase 3** — absolutize the remaining lengths through the crate-internal
///    `absolutize_in_page_context`, using the page context's own `font-size`
///    (step 3's output, or the inherited value when `@page` declared none) as
///    the `em` basis and the root element's font-size as the `rem` basis, and
///    apply the `border-*-width` style gate. Mirrors the element path's
///    [`crate::specified::SpecifiedValues::finalize`]; the split into two phases
///    is required because `padding: 2em` depends on a sibling declaration whose
///    winner is only known after step 2.
///
/// The result is a bag of **computed** values —
/// [`PageCascadeResult::declarations`] documents the one remaining exception
/// (`text-align: match-parent`) and the per-phase citations.
///
/// # The inheritance parent
///
/// See [`PageInheritance`] for the two named choices, their spec basis, and
/// which one to prefer — that is what step 3 above resolves winners against.
///
/// # Typed inheritance choice
///
/// The [`PageInheritance`] parameter is explicit and has no default, so a
/// caller must choose the inheritance source.
///
/// ```compile_fail
/// use raikiri_style::{Origin, PageContextQuery, RuleTree, cascade_page};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("@page { color: red }", Origin::Author);
/// let _ = cascade_page(&tree, &PageContextQuery::default(), None);
/// ```
///
/// A valid [`PageInheritance`] value is accepted by the ordinary example
/// below.
///
/// # Ties on equal `(rank, specificity, source_order)`
///
/// The spec text at the anchor says @page cascade follows normal cascade
/// tie-breaking. Within a single rule, later declarations of the same
/// property win per CSS Cascading L4 §6.1
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> "Order of Appearance";
/// the `>=` in the crate-internal `page_beats` mirrors the style-rule
/// sibling's tie-break (`cascade::beats` uses `>=` on the same triple).
///
/// # Example
///
/// ```ignore
/// use raikiri_style::{Origin, PageContextQuery, PageInheritance, RuleTree, cascade_page};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("@page :first { color: red }", Origin::Author);
/// let query = PageContextQuery { is_first: true, ..Default::default() };
/// // Real callers pass `PageInheritance::FromRoot(root_style)` — see
/// // `PageInheritance`'s doc for the copy-pasteable form. The legacy variant
/// // appears here only because this snippet has no document to cascade.
/// let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
/// // result.declarations() contains one entry: PropertyKey::Color -> red
/// ```
fn page_layer_rank(rule: &PageRule, important: bool) -> u32 {
    if important {
        u32::MAX.saturating_sub(rule.layer_order)
    } else {
        rule.layer_order
    }
}

pub fn cascade_page(
    rule_tree: &RuleTree,
    query: &PageContextQuery,
    inheritance: PageInheritance<'_>,
) -> PageCascadeResult {
    // Candidate: (value, important, origin, specificity, source_order).
    // Shape mirrors `cascade::CascadedDecl` per the sibling convention, with
    // `PageSpecificity` in place of `selectors`-crate `Specificity`.
    let mut candidates: Vec<(PropertyValue, bool, Origin, u32, PageSpecificity, u32)> = Vec::new();
    let mut custom_candidates: Vec<PageCustomCascadedDecl> = Vec::new();
    // Descriptor candidates use the same origin/specificity/source-order tuple
    // as ordinary page declarations. The extra declaration index is needed
    // because descriptor declarations are stored in dedicated vectors and two
    // declarations in one rule share the rule's source order.
    let mut size_best: Option<(u8, u32, PageSpecificity, u32, u32, PageSize)> = None;
    let mut marks_best: Option<(u8, u32, PageSpecificity, u32, u32, PageMarks)> = None;
    let mut bleed_best: Option<(u8, u32, PageSpecificity, u32, u32, PageBleed)> = None;
    let mut margin_boxes: Vec<PageMarginBoxCascadeResult> = Vec::new();
    for rule in &rule_tree.page_rules {
        // Comma-separated list = OR: rule contributes if any entry matches.
        // Take the highest-specificity matching entry within this rule (spec
        // examples in §"Cascading and page context" show the (f,g,h) triple
        // determines the effective specificity of the whole rule).
        let mut best_spec: Option<PageSpecificity> = None;
        for entry in &rule.selector.entries {
            if let Some(spec) = match_page_entry(entry, query) {
                best_spec = Some(match best_spec {
                    Some(prev) => prev.max(spec),
                    None => spec,
                });
            }
        }
        if let Some(spec) = best_spec {
            for (index, decl) in rule.size_declarations.iter().enumerate() {
                let candidate = (
                    cascade_rank(rule.origin, decl.important),
                    page_layer_rank(rule, decl.important),
                    spec,
                    rule.source_order,
                    index as u32,
                    decl.value(),
                );
                if size_best
                    .as_ref()
                    .is_none_or(|existing| descriptor_beats(&candidate, existing))
                {
                    size_best = Some(candidate);
                }
            }
            for (index, decl) in rule.marks_declarations.iter().enumerate() {
                let candidate = (
                    cascade_rank(rule.origin, decl.important),
                    page_layer_rank(rule, decl.important),
                    spec,
                    rule.source_order,
                    index as u32,
                    decl.value(),
                );
                if marks_best
                    .as_ref()
                    .is_none_or(|existing| descriptor_beats(&candidate, existing))
                {
                    marks_best = Some(candidate);
                }
            }
            for (index, decl) in rule.bleed_declarations.iter().enumerate() {
                let candidate = (
                    cascade_rank(rule.origin, decl.important),
                    page_layer_rank(rule, decl.important),
                    spec,
                    rule.source_order,
                    index as u32,
                    decl.value(),
                );
                if bleed_best
                    .as_ref()
                    .is_none_or(|existing| descriptor_beats(&candidate, existing))
                {
                    bleed_best = Some(candidate);
                }
            }
            for margin_box in &rule.margin_box_rules {
                margin_boxes.push(PageMarginBoxCascadeResult {
                    slot: margin_box.slot,
                    declarations: margin_box.declarations.clone(),
                    source_order: rule.source_order,
                    origin: rule.origin,
                    specificity: (spec.f, spec.g, spec.h),
                });
            }
            for decl in &rule.declarations {
                if let PropertyValue::CustomProperty(custom) = &decl.value {
                    custom_candidates.push((
                        custom.clone(),
                        decl.important,
                        rule.origin,
                        page_layer_rank(rule, decl.important),
                        spec,
                        rule.source_order,
                    ));
                    continue;
                }
                // Expand shorthands into longhands before pushing candidates —
                // the parse-time expansion alone does not cover the post-parse
                // mutation path through the `pub` field `PageRule::declarations`
                // (the element-path sibling is
                // `crate::cascade`'s `collect_cascaded`).
                // Rationale is consolidated in `crate::rule::expand_shorthand_into`.
                expand_shorthand_into(decl, |d| {
                    candidates.push((
                        d.value,
                        d.important,
                        rule.origin,
                        page_layer_rank(rule, d.important),
                        spec,
                        rule.source_order,
                    ));
                });
            }
        }
    }

    // Custom properties have case-sensitive name keys and therefore must not
    // enter the ordinary PropertyKey winner table (which contains the
    // `PropertyKey::Custom` sentinel only for the specified-layer shape).
    let mut custom_best: HashMap<SmolStr, (u8, u32, PageSpecificity, u32, CustomProperty)> =
        HashMap::new();
    for (value, important, origin, layer, spec, order) in custom_candidates {
        let name = value.name.clone();
        let rank = cascade_rank(origin, important);
        let replace = custom_best.get(&name).is_none_or(|existing| {
            (rank, layer, spec, order) >= (existing.0, existing.1, existing.2, existing.3)
        });
        if replace {
            custom_best.insert(name, (rank, layer, spec, order, value));
        }
    }
    let custom_local: HashMap<SmolStr, SmolStr> = custom_best
        .into_iter()
        .map(|(name, (_, _, _, _, value))| (name, value.value))
        .collect();

    // Winner selection — sibling arm to `cascade::pick_winners`.
    let mut best: HashMap<PropertyKey, (u8, u32, PageSpecificity, u32, PropertyValue)> =
        HashMap::new();
    for (value, important, origin, layer, spec, order) in candidates {
        let rank = cascade_rank(origin, important);
        let key = value.key();
        let candidate = (rank, layer, spec, order, value);
        match best.get(&key) {
            Some(existing) => {
                if page_beats(&candidate, existing) {
                    best.insert(key, candidate);
                }
            }
            None => {
                best.insert(key, candidate);
            }
        }
    }

    // Step 3 (phase 2): resolve winners against the page context's inheritance
    // parent. `PageInheritance::LegacyInitialValues` falls back to the initial
    // values, which the L3 legacy exception in CSS Paged Media 3 §6 "Page
    // Properties" permits explicitly (see `PageInheritance`'s doc). Shared
    // static: the fallback is immutable and `initial()` costs a heap allocation,
    // which the `LegacyInitialValues` path would otherwise take on every call
    // (`property.rs` の `empty_counter_entries` と同じ前例)。
    static INITIAL_PAGE_PARENT: LazyLock<ComputedValues> = LazyLock::new(ComputedValues::initial);
    let empty_custom_property_environment = empty_custom_properties();
    let (inherited, inherited_custom_properties): (
        &ComputedValues,
        &Arc<CustomPropertyEnvironment>,
    ) = match inheritance {
        PageInheritance::FromRoot(root) => (root, &root.custom_properties),
        PageInheritance::LegacyInitialValues => {
            (&INITIAL_PAGE_PARENT, &empty_custom_property_environment)
        }
    };
    let custom_properties =
        resolve_custom_property_environment(inherited_custom_properties, &custom_local);
    // `ctx` carries `inherited`'s used line-height as `root_line_height` —
    // needed by *both* step 3 below (`resolve_against_inherited`'s
    // `FontSize` arm, for `font-size: 1lh`/`1rlh`'s self-reference basis) and
    // step 4 (phase 3, for `padding: 1lh` etc.'s basis via `own_line_height`).
    // Built once here rather than separately in each step: `inherited` is
    // immutable for the whole function, so the two steps would otherwise
    // compute the exact same value twice.
    //
    // `rlh` always refers to the *root element's* own
    // `lh`, never the page context's — CSS Paged Media 3 §6 "The page context
    // inherits from the root element", so `inherited` (the root element's
    // `ComputedValues`, or the L3 legacy initial-values fallback) is the right
    // source, unconditionally, regardless of what the page context's own
    // `line-height` declares. `resolve_against_inherited`'s `FontSize` arm
    // relies on the same fact for `font-size: 1lh`'s self-reference basis —
    // for the page context specifically, "parent" and "root" coincide (both
    // are `inherited`), so the one `ctx.root_line_height` serves both.
    let ctx = ResolveContext::with_root_line_height(
        inherited.font_size,
        used_line_height_length(inherited.line_height, inherited.font_size),
    );
    let resolved: HashMap<PropertyKey, ResolvedAgainstInherited> = best
        .into_iter()
        .filter_map(|(k, (_, _, _, _, value))| {
            let value = match value {
                PropertyValue::Deferred(deferred) => {
                    resolve_deferred_value(&deferred, custom_properties.as_ref())
                }
                value => Some(value),
            }?;
            Some((k, resolve_against_inherited(value, inherited, &ctx)))
        })
        .collect();

    // Step 4 (phase 3): absolutize the remaining lengths against the page
    // context's own font-size and apply the `border-*-width` style gate.
    // Splitting this out of step 3 is forced by the same constraint the element
    // path has: `padding: 2em` needs the page
    // context's font-size, which step 3 only finalises once *all* winners have
    // been seen — `best` iteration order is `HashMap`-random.
    let font_size = page_context_font_size(&resolved, inherited);
    // `rem` = "the computed value of the em unit on the root element"
    // (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>). The page
    // context is *not* the root element, so this stays the inheritance parent's
    // font-size even after `font_size` above diverges from it — the element-path
    // sibling is `SpecifiedValues::finalize`, not `finalize_as_root`. `ctx`
    // (built above, before step 3) already carries this basis.
    let border_styles = page_context_border_styles(&resolved);
    let outline_style = page_context_outline_style(&resolved);
    // The page context's raw `overflow-x`/`overflow-y` winners
    // — mirrors `border_styles` above: the CSS Overflow
    // 3 §3.1 cross-axis coupling needs both axes at once, and `resolved`'s
    // iteration below only ever sees one winner at a time.
    let overflow_pair = page_context_overflow_pair(&resolved);
    // The page context's own `lh` basis — mirrors
    // `font_size` above: `1lh` in `padding`/`margin`/`border-*-width` needs
    // the page context's *own* resolved line-height, not the root's.
    let own_line_height = page_context_line_height_basis(&resolved, inherited, font_size, &ctx);
    PageCascadeResult {
        declarations: resolved
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    absolutize_in_page_context(
                        v,
                        font_size,
                        own_line_height,
                        &ctx,
                        border_styles,
                        outline_style,
                        overflow_pair,
                    ),
                )
            })
            .collect(),
        size: size_best
            .map(|candidate| absolutize_page_size(candidate.5, font_size, own_line_height, &ctx)),
        marks: marks_best.map(|candidate| candidate.5),
        bleed: bleed_best
            .map(|candidate| absolutize_page_bleed(candidate.5, font_size, own_line_height, &ctx)),
        margin_boxes,
    }
}

/// The page context's own computed `font-size` — the `em` basis for phase 3.
///
/// CSS Paged Media 3 §6 "Page Properties"
/// (<https://www.w3.org/TR/css-page-3/#page-properties>) verbatim: "Values in
/// units of em and ex are interpreted relative to the font associated with
/// their context." The font associated with the page context is the one its own
/// `font-size` declaration establishes; with no such declaration the same §
/// gives the inherited value ("The page context inherits from the root
/// element", plus "both the page context and the margin context have a computed
/// value for every property").
///
/// `declarations` must already have been through phase 2
/// ([`crate::cascade::resolve_against_inherited`]) —
/// this is enforced by the parameter type itself, not merely documented; see
/// [`crate::cascade::ResolvedAgainstInherited`] for the exact scope of that
/// guarantee. Its `FontSize` arm always wraps its result in [`Length::Px`].
/// **That invariant is what makes the read below total**: any other `Length` variant under
/// `PropertyKey::FontSize` is unreachable, and the catch-all falls back to the
/// inherited value rather than panicking (crate policy: no panic surface in
/// the cascade — see the shorthand fall-through note in
/// [`crate::cascade::apply_value`]'s doc). If that arm ever stops normalising to
/// `Px`, this function silently starts using the wrong basis, so the two must
/// be changed together.
fn page_context_font_size(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
    inherited: &ComputedValues,
) -> ComputedLength {
    match declarations
        .get(&PropertyKey::FontSize)
        .map(ResolvedAgainstInherited::as_property_value)
    {
        Some(PropertyValue::FontSize(Length::Px(px))) => ComputedLength(*px),
        other => {
            // cov:ignore: the `Some(non-`Px`)` half is unreachable — phase 2's
            // `FontSize` arm wraps its result in `Length::Px` unconditionally.
            // The `None` half *is* covered, by
            // `page_context_font_size_falls_back_to_inherited`.
            //
            // The assert exists because the silent-fallback behaviour must not
            // be relied on: if someone adds `font-size: larger | smaller` and
            // stops normalising to `Px`, the page context would quietly use the
            // root's font-size as the `em` basis and every length in the page
            // box would be wrong, with every test still green.
            debug_assert!(
                other.is_none(),
                "phase 2 must normalise `font-size` to `Length::Px`; got {other:?}"
            );
            inherited.font_size
        }
    }
}

/// The page context's own `lh` basis — the resolve basis for `1lh` used in
/// `padding`/`margin`/`border-*-width` in phase 3.
///
/// Mirrors [`page_context_font_size`]'s shape: an undeclared `line-height`
/// falls back to `inherited.line_height` (CSS Paged Media 3 §6 "The page
/// context inherits from the root element", plus "both the page context and
/// the margin context have a computed value for every property" — line-height
/// is an inherited property so the fallback is the inheritance parent's
/// computed value, not the property's own initial `normal`). A declared
/// `line-height` is resolved through [`resolve_line_height`], passing the
/// root element as the page context's self-reference "parent" (`self_reference_parent`
/// local below — CSS Values 4 §6.1.1's self-reference clause, canonically
/// documented on [`resolve_line_height`]; only its `Length::Lh` arm actually
/// reads this argument, `Length::Rlh` reads `ctx.root_line_height` directly
/// regardless of what is passed here, so the two happen to coincide for the
/// page context specifically).
///
/// # Which font-size does an inherited `<number>` multiply by?
///
/// Both branches — declared and undeclared — convert the resulting
/// [`crate::resolve::ComputedLineHeight`] with **this call's own `font_size`
/// argument** (the page context's own, from [`page_context_font_size`]),
/// never the root's — ordinary CSS inheritance semantics for the unitless
/// multiplier ([`crate::resolve::ComputedLineHeight`] doc, "子は number を
/// inherit して自分の font-size に掛ける"), not a page-context-specific
/// carve-out — pinned by
/// `cascade_page_padding_lh_uses_page_context_own_font_size_for_inherited_number`.
///
/// The result is then converted to an absolute length via
/// [`used_line_height_length`], returning `None` when unresolvable (`normal`
/// with no font metrics — the same wall as `cap`/`rcap`).
fn page_context_line_height_basis(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
    inherited: &ComputedValues,
    font_size: ComputedLength,
    ctx: &ResolveContext,
) -> Option<ComputedLength> {
    // CSS Paged Media 3 §6: the page context's self-reference "parent" (CSS
    // Values 4 §6.1.1) is the root element.
    let self_reference_parent = ctx.root_line_height;
    let line_height = match declarations
        .get(&PropertyKey::LineHeight)
        .map(ResolvedAgainstInherited::as_property_value)
    {
        Some(PropertyValue::LineHeight(lh)) => {
            resolve_line_height(*lh, font_size, self_reference_parent, ctx)
        }
        _ => inherited.line_height,
    };
    used_line_height_length(line_height, font_size)
}

/// The page context's computed `border-*-style` per side — the gate input for
/// `border-*-width` in phase 3.
///
/// An **absent** declaration means the initial value: CSS Paged Media 3 §6
/// (<https://www.w3.org/TR/css-page-3/#page-properties>) gives the page context
/// "a computed value for every property", and `border-style` is not inherited
/// (CSS Backgrounds 3 §3.2
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>), so the computed
/// value of an undeclared side is [`INITIAL_BORDER`]`.style` = `none`.
///
/// The consequence is load-bearing and matches the element path: `@page {
/// border-top-width: 5px }` **on its own** computes to `0px`, exactly as
/// `ComputedValues::initial().border.top.width()` is `0px` for an element that
/// declares no `border-style`.
fn page_context_border_styles(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
) -> Sides<BorderStyle> {
    // **Dispatch on the variant, never on the key.** A `PropertyKey` lookup
    // followed by a payload match would make correctness depend on the map's
    // keying invariant (`PropertyValue::key()` agreeing with the key it is
    // stored under) — an invariant that lives only in prose, and whose
    // violation would degrade silently to the initial value. Scanning the
    // values makes the invariant irrelevant: each arm names the side it writes.
    //
    // Order-independence: `cascade_page` keeps at most one winner per
    // `PropertyKey`, so at most one value matches each arm and the
    // `HashMap`-random iteration order cannot change the result.
    //
    // The `_` arm here is a *filter*, not a classification — it does not weaken
    // the variant-addition tripwire, which lives in
    // `absolutize_in_page_context`: a new `border-*-width`-like variant fails to
    // compile there first, and fixing it forces the author back through this
    // function.
    let mut sides = Sides::all(INITIAL_BORDER.style);
    for value in declarations.values() {
        match value.as_property_value() {
            PropertyValue::BorderTopStyle(s) => sides.top = *s,
            PropertyValue::BorderRightStyle(s) => sides.right = *s,
            PropertyValue::BorderBottomStyle(s) => sides.bottom = *s,
            PropertyValue::BorderLeftStyle(s) => sides.left = *s,
            _ => {}
        }
    }
    sides
}

/// The page context's computed `outline-style`, used to gate the
/// `outline-width` longhand in phase 3. An absent declaration means the
/// initial `none` (CSS UI 3 §4.3); unlike border sides, outline has one shared
/// style rather than four physical sides.
fn page_context_outline_style(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
) -> OutlineStyle {
    declarations
        .values()
        .find_map(|value| match value.as_property_value() {
            PropertyValue::OutlineStyle(style) => Some(*style),
            _ => None,
        })
        .unwrap_or(OutlineStyle::None)
}

/// The page context's raw (pre-[`resolve_overflow`]) `overflow-x` +
/// `overflow-y` winners — the input to the CSS Overflow 3 §3.1 cross-axis
/// coupling gate in phase 3.
///
/// Sibling of [`page_context_border_styles`] — same shape and same rationale
/// (dispatch on the variant, never on the key; an **absent** declaration
/// means the initial value, CSS Paged Media 3 §6 "both the page context and
/// the margin context have a computed value for every property"; `overflow`
/// is not inherited, CSS Overflow 3 §3.1 "Inherited: no", so the fallback for
/// an undeclared axis is the property's own initial `visible`, not
/// `inherited`'s value).
fn page_context_overflow_pair(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
) -> OverflowXY {
    let mut pair = OverflowXY::both(OverflowValue::Visible);
    for value in declarations.values() {
        match value.as_property_value() {
            PropertyValue::OverflowX(v) => pair.x = *v,
            PropertyValue::OverflowY(v) => pair.y = *v,
            _ => {}
        }
    }
    pair
}

/// **phase 3** for the page context — absolutize one winner against the page
/// context's own `font-size` and apply the `border-*-width` style gate.
///
/// Sibling of the element path's [`crate::specified::SpecifiedValues::finalize`]
/// second half (`absolutize_with`). Both funnel into the *same*
/// [`crate::resolve`] functions, so the spec rules (`em` / `rem` basis,
/// percentage pass-through, border style gating) have a single implementation
/// per rule; only the integration logic differs, because the page path carries a
/// `PropertyValue` bag instead of a typed struct.
///
/// # What this function does not resolve, and why that is fine
///
/// `<percentage>` on the box properties stays a percentage — it is a
/// used-value-layer input (CSS Values 4 §5.5.1), not a phase-3 concern. The
/// spec citation lives in [`PageCascadeResult::declarations`] (canonical).
///
/// `text-align: match-parent` is **not** handled here either, but for a
/// different reason than it used to be: it is now fully
/// resolved by **phase 2**
/// ([`crate::cascade::resolve_against_inherited`], which runs before this
/// function) — inherited-value dependence is phase 2's shape, not phase 3's,
/// so by the time a value reaches this function `TextAlign::MatchParent`
/// should never appear. It stays in the pass-through arm below (alongside
/// `Color` / `Display` / …) simply because, once resolved, `text-align`'s
/// computed value carries no length for phase 3 to touch — same as before,
/// just for a different underlying reason. `direction` joins the same arm as
/// a new, ordinary computed-equivalent keyword (CSS Writing Modes 4 §2.1,
/// computed value = specified value, no phase-2 or phase-3 work at all).
///
/// # No wildcard arm
///
/// The match is exhaustive without `_`, like its two siblings
/// ([`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]).
/// A new [`PropertyValue`] variant that carries a length must be classified
/// here explicitly; a catch-all would let it reach the public `declarations`
/// map as a specified value — the exact shape of a regression this crate has
/// already hit once. (What this guard does *not* catch is a new **payload**
/// case inside an existing variant — gap (a).)
///
/// # The `value` parameter is phase-2 output, enforced by its type
///
/// `value` is a
/// [`crate::cascade::ResolvedAgainstInherited`]
/// rather than a raw [`PropertyValue`] — see that type's doc for what this
/// does and does not guarantee ("narrowed, not closed").
///
/// # `own_line_height`
///
/// The page context's own `lh` basis — [`page_context_line_height_basis`]'s
/// output, threaded alongside `font_size` for the same reason: `1lh` in
/// `padding`/`margin`/`border-*-width` needs this context's *own* resolved
/// line-height (not the root's — that is `ctx.root_line_height`, used only
/// for `rlh`).
///
/// # `overflow_pair`
///
/// [`page_context_overflow_pair`]'s output — the page context's raw
/// `overflow-x`/`overflow-y` winners, threaded in for the same reason
/// `border_styles` is: CSS Overflow 3 §3.1's cross-axis coupling
/// ([`resolve_overflow`]) needs *both* axes at once, and this function
/// otherwise only sees one [`PropertyValue`] winner at a time.
fn absolutize_in_page_context(
    value: ResolvedAgainstInherited,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
    border_styles: Sides<BorderStyle>,
    outline_style: OutlineStyle,
    overflow_pair: OverflowXY,
) -> PropertyValue {
    let value = value.into_property_value();
    /// `<length-percentage>` → computed, mapped back into the specified-layer
    /// `Length` shape that `PropertyValue` carries (`Px` / `Percent` only —
    /// `Em` / `Rem` / `Pt` are gone after this).
    fn lp(
        specified: Length,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Length {
        match resolve_length_percentage(specified, font_size, own_line_height, ctx) {
            ComputedLengthPercentage::Px(v) => Length::Px(v),
            ComputedLengthPercentage::Percent(p) => Length::Percent(p),
        }
    }
    /// `<length-percentage> | auto` — same mapping, `auto` preserved.
    fn lpa(
        specified: LengthOrAuto,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> LengthOrAuto {
        match resolve_length_percentage_or_auto(specified, font_size, own_line_height, ctx) {
            ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
            ComputedLengthPercentageOrAuto::Percent(p) => LengthOrAuto::Length(Length::Percent(p)),
            ComputedLengthPercentageOrAuto::Calc(value) => LengthOrAuto::Calc(value),
        }
    }
    /// `<length-percentage> | auto` for `margin-*` specifically —
    /// same mapping as [`lpa`] but
    /// routed through [`resolve_margin_length_or_auto`], whose unresolvable-`lh`/
    /// `rlh` fallback is `Px(0.0)` (margin's true spec initial), not `Auto`
    /// (which is `width`/`height`'s initial, and which would trigger real
    /// auto-margin layout behaviour if used here — see that function's doc).
    fn margin_lpa(
        specified: LengthOrAuto,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> LengthOrAuto {
        match resolve_margin_length_or_auto(specified, font_size, own_line_height, ctx) {
            ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
            ComputedLengthPercentageOrAuto::Percent(p) => LengthOrAuto::Length(Length::Percent(p)),
            ComputedLengthPercentageOrAuto::Calc(value) => LengthOrAuto::Calc(value),
        }
    }
    /// `flex-basis: content | <'width'>` — same mapping as [`lpa`], with
    /// `content` preserved as its own keyword (`ComputedFlexBasis::Content`
    /// has no `LengthOrAuto` equivalent, so it maps back to
    /// `FlexBasisValue::Content` directly rather than routing through the
    /// `Auto`/`Px`/`Percent` cases `lpa` shares).
    fn fb(
        specified: FlexBasisValue,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> FlexBasisValue {
        match resolve_flex_basis(specified, font_size, own_line_height, ctx) {
            ComputedFlexBasis::Auto => FlexBasisValue::Auto,
            ComputedFlexBasis::Content => FlexBasisValue::Content,
            ComputedFlexBasis::MinContent => FlexBasisValue::MinContent,
            ComputedFlexBasis::MaxContent => FlexBasisValue::MaxContent,
            ComputedFlexBasis::FitContent => FlexBasisValue::FitContent,
            ComputedFlexBasis::Px(v) => FlexBasisValue::Length(Length::Px(v)),
            ComputedFlexBasis::Percent(p) => FlexBasisValue::Length(Length::Percent(p)),
        }
    }
    /// `background-size: <bg-size>` — same round-trip as [`lpa`], applied
    /// per axis via [`resolve_background_size`]; `cover`/`contain` carry no
    /// length and pass straight through.
    fn background_size(
        specified: BackgroundSize,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> BackgroundSize {
        fn lift(computed: ComputedLengthPercentageOrAuto) -> LengthOrAuto {
            match computed {
                ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
                ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
                ComputedLengthPercentageOrAuto::Percent(p) => {
                    LengthOrAuto::Length(Length::Percent(p))
                }
                ComputedLengthPercentageOrAuto::Calc(value) => LengthOrAuto::Calc(value),
            }
        }
        match resolve_background_size(specified, font_size, own_line_height, ctx) {
            ComputedBackgroundSize::Cover => BackgroundSize::Cover,
            ComputedBackgroundSize::Contain => BackgroundSize::Contain,
            ComputedBackgroundSize::Explicit { width, height } => BackgroundSize::Explicit {
                width: lift(width),
                height: lift(height),
            },
        }
    }
    /// `background-position: <position>` — same round-trip as [`lp`],
    /// applied to each edge/offset via [`resolve_css_position`].
    fn css_position(
        specified: CssPosition,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> CssPosition {
        fn lift(computed: ComputedLengthPercentage) -> Length {
            match computed {
                ComputedLengthPercentage::Px(v) => Length::Px(v),
                ComputedLengthPercentage::Percent(p) => Length::Percent(p),
            }
        }
        fn lift_offset(computed: ComputedCssPositionOffset) -> CssPositionOffset {
            match computed {
                ComputedCssPositionOffset::Start(l) => CssPositionOffset::Start(lift(l)),
                ComputedCssPositionOffset::End(l) => CssPositionOffset::End(lift(l)),
            }
        }
        let computed = resolve_css_position(specified, font_size, own_line_height, ctx);
        CssPosition {
            horizontal: lift_offset(computed.horizontal),
            vertical: lift_offset(computed.vertical),
        }
    }
    /// `row-gap` / `column-gap`: `normal | <length-percentage [0,∞]>` —
    /// same mapping shape as [`lp`], with `normal` preserved (unlike
    /// `letter-spacing`/`word-spacing`'s `lift_length_or_normal`, which
    /// collapses `normal` to a resolved `ComputedLength` — gap's `normal`
    /// stays a keyword at the computed layer, see
    /// [`ComputedLengthPercentageOrNormal`] doc).
    fn lpn(
        specified: LengthOrNormal,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> LengthOrNormal {
        match resolve_length_percentage_or_normal(specified, font_size, own_line_height, ctx) {
            ComputedLengthPercentageOrNormal::Normal => LengthOrNormal::Normal,
            ComputedLengthPercentageOrNormal::Px(v) => LengthOrNormal::Length(Length::Px(v)),
            ComputedLengthPercentageOrNormal::Percent(p) => {
                LengthOrNormal::Length(Length::Percent(p))
            }
        }
    }
    /// [`ComputedGridTrackBreadth`] → specified [`GridTrackBreadth`] —
    /// round-trip half of [`gtt`]/[`gatl`], same "computed-equivalent
    /// specified representation" convention as [`fb`]/[`lpn`] (`Px`/
    /// `Percent` only, no other unit ever appears post-absolutization).
    fn grid_track_breadth_from_computed(b: ComputedGridTrackBreadth) -> GridTrackBreadth {
        match b {
            ComputedGridTrackBreadth::Px(v) => GridTrackBreadth::Length(Length::Px(v)),
            ComputedGridTrackBreadth::Percent(p) => GridTrackBreadth::Length(Length::Percent(p)),
            ComputedGridTrackBreadth::Flex(f) => GridTrackBreadth::Flex(f),
            ComputedGridTrackBreadth::MinContent => GridTrackBreadth::MinContent,
            ComputedGridTrackBreadth::MaxContent => GridTrackBreadth::MaxContent,
            ComputedGridTrackBreadth::Auto => GridTrackBreadth::Auto,
        }
    }
    /// [`ComputedGridTrackBreadth`] → specified [`GridInflexibleBreadth`] —
    /// sibling of [`grid_track_breadth_from_computed`] for `minmax()`'s min
    /// side. `ComputedGridTrackBreadth::Flex` never reaches
    /// here (`ComputedGridTrackBreadth`'s doc "collapse" note: the min side
    /// is only ever produced by [`crate::resolve::resolve_grid_inflexible_breadth`],
    /// which has no `Flex` variant to produce it from) — the arm is a
    /// defensive fallback to keep the match exhaustive, not a reachable case.
    fn grid_inflexible_breadth_from_computed(b: ComputedGridTrackBreadth) -> GridInflexibleBreadth {
        match b {
            ComputedGridTrackBreadth::Px(v) => GridInflexibleBreadth::Length(Length::Px(v)),
            ComputedGridTrackBreadth::Percent(p) => {
                GridInflexibleBreadth::Length(Length::Percent(p))
            }
            ComputedGridTrackBreadth::MinContent => GridInflexibleBreadth::MinContent,
            ComputedGridTrackBreadth::MaxContent => GridInflexibleBreadth::MaxContent,
            // cov:ignore: unreachable per this fn's doc; kept for match
            // exhaustiveness.
            ComputedGridTrackBreadth::Flex(_) | ComputedGridTrackBreadth::Auto => {
                GridInflexibleBreadth::Auto
            }
        }
    }
    /// [`ComputedGridTrackSize`] → specified [`GridTrackSize`] — round-trip
    /// half of [`gtt`]/[`gatl`].
    fn grid_track_size_from_computed(s: ComputedGridTrackSize) -> GridTrackSize {
        match s {
            ComputedGridTrackSize::Breadth(b) => {
                GridTrackSize::Breadth(grid_track_breadth_from_computed(b))
            }
            ComputedGridTrackSize::MinMax(min, max) => GridTrackSize::MinMax(
                grid_inflexible_breadth_from_computed(min),
                grid_track_breadth_from_computed(max),
            ),
            ComputedGridTrackSize::FitContent(lp) => GridTrackSize::FitContent(match lp {
                ComputedLengthPercentage::Px(v) => Length::Px(v),
                ComputedLengthPercentage::Percent(p) => Length::Percent(p),
            }),
        }
    }
    /// [`ComputedGridTrackList`] → specified [`GridTrackList`] — round-trip
    /// half of [`gtt`]. `line_names` carry no length so they clone through
    /// unchanged.
    fn grid_track_list_from_computed(list: &ComputedGridTrackList) -> GridTrackList {
        GridTrackList {
            line_names: list.line_names.clone(),
            components: list
                .components
                .iter()
                .cloned()
                .map(|c| match c {
                    ComputedGridTrackListComponent::Size(s) => {
                        GridTrackListComponent::Size(grid_track_size_from_computed(s))
                    }
                    ComputedGridTrackListComponent::Repeat(r) => {
                        GridTrackListComponent::Repeat(GridTrackRepeat {
                            count: r.count,
                            line_names: r.line_names,
                            tracks: r
                                .tracks
                                .into_iter()
                                .map(grid_track_size_from_computed)
                                .collect(),
                        })
                    }
                })
                .collect(),
        }
    }
    /// `grid-template-columns` / `grid-template-rows` — round-trips through
    /// [`resolve_grid_template_tracks`] and maps the `Computed*` mirror
    /// types back to the specified-layer shape (`none` preserved as a
    /// keyword, same as [`fb`]'s `Auto`/`Content` handling).
    fn gtt(
        specified: GridTemplateTracks,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> GridTemplateTracks {
        match resolve_grid_template_tracks(specified, font_size, own_line_height, ctx) {
            ComputedGridTemplateTracks::None => GridTemplateTracks::None,
            ComputedGridTemplateTracks::List(list) => {
                GridTemplateTracks::List(Arc::new(grid_track_list_from_computed(&list)))
            }
        }
    }
    /// `grid-auto-columns` / `grid-auto-rows` — same round-trip shape as
    /// [`gtt`], for the `<track-size>+` (no `none`, no `<line-names>`)
    /// grammar.
    fn gatl(
        specified: &[GridTrackSize],
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Arc<Vec<GridTrackSize>> {
        let computed = resolve_grid_auto_track_list(specified, font_size, own_line_height, ctx);
        Arc::new(
            computed
                .iter()
                .cloned()
                .map(grid_track_size_from_computed)
                .collect(),
        )
    }
    /// One `border-*` side: absolutize the width and apply the style gate.
    ///
    /// Routed through [`resolve_border`] rather than re-testing
    /// `none` / `hidden` locally — that function's doc calls itself the single
    /// source of the gate and forbids re-implementing the rule elsewhere.
    fn border(
        specified: Border,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Border {
        let computed = resolve_border(specified, font_size, own_line_height, ctx);
        Border {
            width: Length::Px(computed.width.px()),
            style: computed.style,
            color: computed.color,
        }
    }
    /// A `border-*-width` longhand, gated by the side's computed style.
    ///
    /// `color` is a placeholder — [`resolve_border`] never reads it, and the
    /// longhand carries no colour. Building the `Border` here (instead of
    /// branching on `style` locally) is what keeps the gate single-sourced.
    fn border_width(
        width: Length,
        style: BorderStyle,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Length {
        border(
            Border {
                width,
                style,
                color: BorderColor::CurrentColor,
            },
            font_size,
            own_line_height,
            ctx,
        )
        .width
    }
    /// An `outline-width` longhand, gated by the winning `outline-style`.
    ///
    /// Routing through [`resolve_outline`] keeps the element and page paths
    /// on the same CSS UI 3 §4.2 computed-value rule: width is zero when the
    /// style is `none` (or the retained-but-parser-rejected `hidden` keyword).
    fn outline_width(
        width: Length,
        style: OutlineStyle,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Length {
        Length::Px(
            resolve_outline(
                Outline {
                    width,
                    style,
                    color: OutlineColor::Invert,
                },
                font_size,
                own_line_height,
                ctx,
            )
            .width()
            .px(),
        )
    }
    /// One `text-shadow` item: absolutize the 3 lengths (`offset-x`/
    /// `offset-y`/`blur-radius`), round-tripped back into the specified-layer
    /// `Length::Px` shape (`resolve_length` is the percentage-less resolver —
    /// text-shadow's lengths don't allow `<percentage>`, `TextShadowItem`
    /// doc). `color` carries no length (`TextShadowColor` doc) — passed
    /// through unchanged, same as `Border`'s `color` field above.
    fn text_shadow_item(
        specified: TextShadowItem,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> TextShadowItem {
        TextShadowItem {
            offset_x: Length::Px(
                resolve_length(specified.offset_x, font_size, own_line_height, ctx).px(),
            ),
            offset_y: Length::Px(
                resolve_length(specified.offset_y, font_size, own_line_height, ctx).px(),
            ),
            blur_radius: Length::Px(
                resolve_length(specified.blur_radius, font_size, own_line_height, ctx).px(),
            ),
            color: specified.color,
        }
    }

    /// `border-radius` の 4 corner を computed 長から `PropertyValue` が
    /// 運ぶ `Length::Px` 表現へ戻す。page bag は element path の
    /// `ComputedValues` と同じ computed-value 契約を持つが、既存の bag の
    /// API を壊さないため payload 型は specified 側を再利用する。
    fn border_radius_value(
        specified: BorderRadius,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> BorderRadius {
        let computed = resolve_border_radius(specified, font_size, own_line_height, ctx);
        let length = |value: ComputedLengthPercentage| match value {
            ComputedLengthPercentage::Px(px) => Length::Px(px),
            ComputedLengthPercentage::Percent(percent) => Length::Percent(percent),
        };
        BorderRadius {
            top_left: length(computed.top_left),
            top_right: length(computed.top_right),
            bottom_right: length(computed.bottom_right),
            bottom_left: length(computed.bottom_left),
        }
    }

    /// `box-shadow` の 1 item を computed 長から page bag の
    /// `BoxShadowItem` へ戻す。`color` は resolve 層で長さを持たないため
    /// そのまま保持する。
    fn box_shadow_item(
        specified: BoxShadowItem,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> BoxShadowItem {
        let computed = resolve_box_shadow_item(specified, font_size, own_line_height, ctx);
        BoxShadowItem {
            offset_x: Length::Px(computed.offset_x.px()),
            offset_y: Length::Px(computed.offset_y.px()),
            blur_radius: Length::Px(computed.blur_radius.px()),
            spread_radius: Length::Px(computed.spread_radius.px()),
            color: computed.color,
            inset: computed.inset,
        }
    }

    /// `outline` の width を絶対化した computed value を
    /// `PropertyValue` の payload に戻す。outline は box model の寸法へ
    /// 影響しないため、page phase では値の解決だけを行う。
    fn outline_value(
        specified: Outline,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Outline {
        let computed = resolve_outline(specified, font_size, own_line_height, ctx);
        Outline {
            width: Length::Px(computed.width().px()),
            style: computed.style(),
            color: computed.color,
        }
    }

    /// `transform` の `<length-percentage>` slot を絶対化し page bag の
    /// `TransformFunction` へ戻す。`lp` と同じ round-trip — length 側は
    /// `Px` へ、percentage 側は `Percent` のまま残す。`matrix` の 6
    /// `<number>` slot と `rotate`/`skew` 系の `<angle>` slot はそのまま。
    fn transform_function(
        specified: TransformFunction,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> TransformFunction {
        match specified {
            TransformFunction::Matrix(m) => TransformFunction::Matrix(m),
            TransformFunction::Translate(tx, ty) => TransformFunction::Translate(
                lp(tx, font_size, own_line_height, ctx),
                lp(ty, font_size, own_line_height, ctx),
            ),
            TransformFunction::TranslateX(v) => {
                TransformFunction::TranslateX(lp(v, font_size, own_line_height, ctx))
            }
            TransformFunction::TranslateY(v) => {
                TransformFunction::TranslateY(lp(v, font_size, own_line_height, ctx))
            }
            TransformFunction::Scale(x, y) => TransformFunction::Scale(x, y),
            TransformFunction::ScaleX(v) => TransformFunction::ScaleX(v),
            TransformFunction::ScaleY(v) => TransformFunction::ScaleY(v),
            TransformFunction::Rotate(a) => TransformFunction::Rotate(a),
            TransformFunction::Skew(ax, ay) => TransformFunction::Skew(ax, ay),
            TransformFunction::SkewX(a) => TransformFunction::SkewX(a),
            TransformFunction::SkewY(a) => TransformFunction::SkewY(a),
        }
    }

    match value {
        // ── already computed-equivalent after phase 2 ──────────────────────
        // `font-size` is phase 2's output (`Length::Px`); re-absolutizing it
        // here would be a second application against the *wrong* basis (its
        // own value instead of the inheritance parent's).
        v @ (PropertyValue::Color(_)
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Deferred(_)
        | PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::CalcLengthPercentage { .. }
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::FontSize(_)
        | PropertyValue::FontWeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::ListStyleType(_)
        | PropertyValue::ListStyleImage(_)
        | PropertyValue::ListStylePosition(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterResetInherit
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::TextAlign(_)
        | PropertyValue::HangingPunctuation(_)
        | PropertyValue::Direction(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::OutlineStyle(_)
        | PropertyValue::OutlineColor(_)
        | PropertyValue::BoxSizing(_)
        // `text-decoration-line`/`-style`/`-color` carry no length and
        // computed value = specified keyword(s)/color (see
        // `TextDecorationLine`/`TextDecorationStyle`/`TextDecorationColor`
        // docs) — nothing for phase 3 to absolutize. The `text-decoration`
        // shorthand itself does NOT join this bucket: its `-thickness`
        // component carries `<length-percentage>` (ED §2.4.1), so it gets
        // its own fall-through transform arm below (same "structurally
        // unreachable, pinned directly" shape as `Margin`/`Border`/
        // `Background` above).
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        // `text-decoration-skip-ink`/`-skip-spaces`/`text-emphasis-position`/
        // `text-underline-position` carry no length and computed value =
        // specified keyword(s) (see each type's doc) — nothing for phase 3
        // to absolutize, same bucket shape as the 3 `text-decoration`
        // longhands above.
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextUnderlinePosition(_)
        // `font-style` carries no length at this crate's scope
        // (`normal`/`italic`/`oblique` implemented, `oblique`'s `<angle>`
        // argument is not — see `FontStyle`'s doc) and computed value =
        // specified keyword — nothing for phase 3 to absolutize.
        | PropertyValue::FontStyle(_)
        // `font-variant-caps` carries no length either (see
        // `FontVariantCaps`'s doc) and computed value = specified keyword —
        // nothing for phase 3 to absolutize.
        | PropertyValue::FontVariantCaps(_)
        // `text-transform` carries no length (see `TextTransform`'s doc)
        // and computed value = specified keyword — nothing for phase 3 to
        // absolutize.
        | PropertyValue::TextTransform(_)
        // `visibility` carries no length (see `Visibility`'s doc) and
        // computed value = specified keyword — nothing for phase 3 to
        // absolutize.
        | PropertyValue::Visibility(_)
        // `z-index` carries no length (see `ZIndexValue`'s doc) and computed
        // value = specified value — nothing for phase 3 to absolutize.
        | PropertyValue::ZIndex(_)
        // `word-break` (CSS Text 3 §5.1) carries no length either and
        // computed value = specified keyword (see `WordBreak`'s doc) —
        // nothing for phase 3 to absolutize.
        | PropertyValue::WordBreak(_)
        // `overflow-wrap` (legacy alias `word-wrap`, CSS Text 3 §5.4)
        // carries no length either (see `OverflowWrap`'s doc) — same as
        // `WordBreak` above.
        | PropertyValue::OverflowWrap(_)
        // `break-before`/`break-after` (CSS Fragmentation Module Level 3
        // §3.1, legacy shorthand `page-break-before`/`page-break-after`
        // included) carry no length either (see `BreakBetween`'s doc) —
        // nothing for phase 3 to absolutize.
        | PropertyValue::BreakBefore(_)
        | PropertyValue::BreakAfter(_)
        // `break-inside` (CSS Fragmentation Module Level 3 §3.2, legacy
        // shorthand `page-break-inside` included) carries no length
        // either (see `BreakInside`'s doc) — same as `BreakBefore`/
        // `BreakAfter` above.
        | PropertyValue::BreakInside(_)
        // `float`/`clear` (CSS2 §9.5.1/§9.5.2) carry no length either. The
        // §9.7 `display` recomputation `float` drives on the element path
        // (`crate::property::resolve_display_for_float`) is **not**
        // applied here — a page box is not an element in a visual
        // formatting context, so §9.7's clause has no subject in this
        // path; `Float`/`Clear` are opaque pass-through values here, the
        // same treatment `ZIndex` gets (see that arm's doc).
        | PropertyValue::Float(_)
        | PropertyValue::Clear(_)
        // `white-space` (CSS Text 3 §3) carries no length either and
        // computed value = specified keyword (see `WhiteSpace`'s doc) —
        // nothing for phase 3 to absolutize.
        | PropertyValue::WhiteSpace(_)
        // `text-wrap` (CSS Text 4 §5 subset) carries no length either —
        // same as `WhiteSpace` above.
        | PropertyValue::TextWrap(_)
        // `hyphens` (CSS Text 3 §5.3) carries no length either and computed
        // value = specified keyword (see `Hyphens`'s doc) — same as
        // `WhiteSpace` above.
        | PropertyValue::Hyphens(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        // `page` (CSS Paged Media 3 §8.1) carries no length and computed
        // value = specified value — nothing for phase 3 to absolutize.
        | PropertyValue::Page(_)
        // `column-count` carries only a keyword/integer and has no page-side
        // length to absolutize. `column-width:auto` is likewise already
        // computed-equivalent; the length-bearing forms have dedicated arms
        // below so relative units do not leak into the page declaration bag.
        | PropertyValue::ColumnCount(_)
        | PropertyValue::ColumnWidth(ColumnWidthValue::Auto)
        // `flex-direction`/`flex-wrap` (CSS Flexible Box Layout Module
        // Level 1 §5.1/§5.2) carry no length and computed value = specified
        // keyword — nothing for phase 3 to absolutize.
        | PropertyValue::FlexDirection(_)
        | PropertyValue::FlexWrap(_)
        // `flex-grow`/`flex-shrink` (§7.2.1/§7.2.2) carry a bare
        // `<number>`, not a length — nothing for phase 3 to absolutize
        // (unlike `FlexBasis` below, which does carry a
        // `<length-percentage>` and gets its own transform arm).
        | PropertyValue::FlexGrow(_)
        | PropertyValue::FlexShrink(_)
        // `flex-flow` shorthand — neither component carries a length (see
        // `place-content` below for the same shape); structurally
        // unreachable here regardless (`expand_shorthand_into` expands it
        // before this function runs).
        | PropertyValue::FlexFlow(_)
        // `order` (§4.2) carries a bare `<integer>`, not a length —
        // nothing for phase 3 to absolutize.
        | PropertyValue::Order(_)
        // `justify-content`/`align-content` (CSS Box Alignment Module Level
        // 3 §5.1) / `align-items` (§7.2) / `align-self` (§6.2) carry no
        // length — same shape as `WordBreak` above.
        | PropertyValue::JustifyContent(_)
        | PropertyValue::AlignContent(_)
        | PropertyValue::AlignItems(_)
        | PropertyValue::AlignSelf(_)
        // `place-content` shorthand joins the same bucket as identity
        // pass-through — neither of its 2 components carries a length (see
        // `TextDecoration` above for the same shorthand-with-no-length-
        // components shape); it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::PlaceContent(_)
        // `quotes` (CSS Content 3 §2.4.1) carries no length and computed
        // value = specified value (`ComputedValues::quotes` doc) — nothing
        // for phase 3 to absolutize.
        | PropertyValue::Quotes(_)
        // `grid-template-areas` (CSS Grid Layout Module Level 1 §7.3) —
        // computed value is the keyword `none` or the authored string list
        // itself (`GridTemplateAreasValue` doc), not a parsed/absolutized
        // form — nothing for phase 3 to absolutize, same shape as
        // `GridTemplateAreasValue`'s specified/computed-equivalent layering
        // (unlike `GridTemplateColumns`/`GridTemplateRows` below, which do
        // carry `<length-percentage>` in their track sizes and get their
        // own transform arm).
        | PropertyValue::GridTemplateAreas(_)
        // `grid-auto-flow` (§7.7) carries no length and computed value =
        // specified keyword(s) — same shape as `WordBreak` above.
        | PropertyValue::GridAutoFlow(_)
        // `grid-row-start`/`-end`/`grid-column-start`/`-end` (§8.3) carry no
        // length (`GridLineValue` doc) — same shape as `WordBreak` above.
        | PropertyValue::GridRowStart(_)
        | PropertyValue::GridRowEnd(_)
        | PropertyValue::GridColumnStart(_)
        | PropertyValue::GridColumnEnd(_)
        // `grid-row`/`grid-column` shorthands (§8.4) join the same bucket as
        // identity pass-through — neither longhand they expand to carries a
        // length either, and they are structurally unreachable here
        // regardless (same `expand_shorthand_into` reasoning as
        // `PlaceContent` above).
        | PropertyValue::GridRow(_)
        | PropertyValue::GridColumn(_)
        // `justify-items` (CSS Box Alignment Module Level 3 §7.1) /
        // `justify-self` (§6.1) carry no length — same shape as
        // `AlignItems`/`AlignSelf` above.
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        // `place-items`/`place-self` shorthands (§7.3/§6.3) join the same
        // bucket as identity pass-through, same reasoning as
        // `PlaceContent`/`GridRow` above.
        | PropertyValue::PlaceItems(_)
        | PropertyValue::PlaceSelf(_)
        // `orphans`/`widows` (CSS Fragmentation Module Level 3 §3.3) carry a
        // bare positive `<integer>`, not a length, and computed value =
        // specified integer — nothing for phase 3 to absolutize, same shape
        // as `FlexGrow`/`FlexShrink` above.
        | PropertyValue::Orphans(_)
        | PropertyValue::Widows(_)
        // `background-repeat`/`background-attachment`/`background-clip`/
        // `background-origin` (CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8)
        // carry no length — computed value = specified keyword(s), same
        // shape as `WordBreak` above. `background-size`/`background-position`
        // do carry `<length-percentage>` and get their own transform arms
        // below (next to `Width`/`Height`).
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)

        // `object-fit` (CSS Images Module Level 3 §5.1) carries no length
        // either — keyword-only payload, same shape as `WordBreak` above.
        // `object-position` (§5.2) does carry `<length-percentage>` (reuses
        // `CssPosition`, `background-position`'s type) and gets its own
        // transform arm below, next to `BackgroundPosition`.
        | PropertyValue::ObjectFit(_)
        // `isolation` (CSS Compositing and Blending Level 1 §3.4.2) /
        // `mix-blend-mode` (§3.4.1) — keyword-only payloads, same shape as
        // `ObjectFit` above.
        | PropertyValue::Isolation(_)
        | PropertyValue::MixBlendMode(_)

        // `clip-path` (§5.1) — `None`/`Url(String)`/`GeometryBox(..)` all
        // carry no length payload (`ClipPath` doc's scope note — no
        // `<basic-shape>` support, so no embedded length at all).
        | PropertyValue::ClipPath(_)
        // `filter` (§5) — Computed value is plain "as specified": no
        // absolutization is spec-required at all, so passing every
        // embedded `Length`/`Angle` through untouched (including
        // `drop-shadow()`'s, reused from `TextShadowItem`) is not a gap,
        // just this property's actual computed-value definition
        // (`FilterFunction` doc's "Range restriction is reject, not clamp"
        // section establishes the same "as specified" fact for a different
        // purpose).
        | PropertyValue::Filter(_)
        // `table-layout` (CSS Tables 3 §4) / `border-collapse` (CSS Tables
        // 3 §6) / `caption-side` (§7) / `empty-cells` (§8) carry no length
        // and computed value = specified keyword
        // (`TableLayoutValue`/`BorderCollapseValue` docs) — nothing for
        // phase 3 to absolutize. A page box is not a table wrapper box, so
        // none of these properties has layout meaning in this path; all are
        // opaque pass-through values here, the same treatment `ZIndex` gets
        // (see that arm's doc). (`border-spacing` §6.1 *does* carry
        // `<length>` and gets its own transform arm next to `TabSize`.)
        | PropertyValue::TableLayout(_)
        | PropertyValue::BorderCollapse(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)) => v,
        // `column-width` and the width component of `columns` are
        // length-bearing. Resolve relative units against the page context,
        // matching the other single-length properties above.
        PropertyValue::ColumnWidth(ColumnWidthValue::Length(length)) => {
            PropertyValue::ColumnWidth(ColumnWidthValue::Length(Length::Px(
                resolve_length(length, font_size, own_line_height, ctx).px(),
            )))
        }
        // cov:ignore: page-context shorthand absolutization is defensive and not part of the focused layout path.
        PropertyValue::Columns(shorthand) => PropertyValue::Columns(ColumnsShorthand {
            width: match shorthand.width {
                ColumnWidthValue::Auto => ColumnWidthValue::Auto,
                ColumnWidthValue::Length(length) => ColumnWidthValue::Length(Length::Px(
                    resolve_length(length, font_size, own_line_height, ctx).px(),
                )),
            },
            count: shorthand.count,
        }),
        // ── background-image / mask-image ───────────────────────────────
        // `None`/`Url(String)` are computed-equivalent. `Gradient(..)`'s
        // `<length-percentage>` payloads (`GradientColorStop::position`,
        // `RadialSize::Circle`/`Ellipse`, `RadialGradient`/`ConicGradient`'s
        // `CssPosition`) are partially absolutized: font-relative lengths
        // (`em`/`rem`/`ex`/`ch`/`ic`/`pt`/`cm`/`mm`/`q`/`in`/`pc`/`lh`/`rlh`)
        // resolve against this page context's own `font-size`/
        // `own_line_height` basis, while `<percentage>` stays symbolic for
        // paint-time box-size resolution — same split as
        // `background-position`/`object-position` (`resolve_css_position`/
        // `resolve_length_percentage`), see `resolve_background_image` doc.
        PropertyValue::BackgroundImage(img) => PropertyValue::BackgroundImage(
            crate::resolve::resolve_background_image(img, font_size, own_line_height, ctx),
        ),
        PropertyValue::MaskImage(img) => PropertyValue::MaskImage(
            crate::resolve::resolve_background_image(img, font_size, own_line_height, ctx),
        ),
        // ── font-size: larger / smaller ──────────────────────────────────
        // ⚠️ **structurally unreachable through `cascade_page`, not a "safety
        // net"** — step 3 (phase 2) in `cascade_page` maps *every* winner
        // through `resolve_against_inherited` before this function ever runs,
        // and that function's `FontSizeRelative` arm always converges to
        // `PropertyValue::FontSize(Length::Px(_))`. There is no second entry
        // point that could skip step 3 (unlike the shorthand fall-throughs
        // below, which *are* reachable via a direct internal call).
        //
        // The arm still exists — not folded into the `Color`/`FontSize`/…
        // bucket above, and not `unreachable!` — for the same two reasons the
        // shorthand fall-throughs keep real arms: the crate keeps the cascade
        // panic-free (a deliberate design policy, see the shorthand
        // fall-through note in `crate::cascade::apply_value`'s doc), and this
        // function's `pub(crate)` visibility means test code *can* call it
        // directly with an
        // unresolved `FontSizeRelative`, bypassing step 3 (as
        // `absolutize_in_page_context_font_size_relative_safety_net` does).
        //
        // The basis is deliberately `font_size` (this context's own,
        // phase-2-resolved value) rather than the inheritance parent's — this
        // function has no parent basis available, and the arm is bug-only
        // regardless, so spec correctness here is not a design goal. What
        // matters is: no panic, and convergence to the same `FontSize(Px(_))`
        // shape the real (phase 2) path produces, so a caller reading
        // `declarations` never observes an unresolved `FontSizeRelative`.
        PropertyValue::FontSizeRelative(rel) => {
            PropertyValue::FontSize(Length::Px(resolve_relative_font_size(rel, font_size.px())))
        }
        // ── line-height ───────────────────────────────────────────────────
        // CSS Inline 3 §5.1 <https://www.w3.org/TR/css-inline-3/#propdef-line-height>:
        // `<percentage>` is "computed relative to 1em" of the declaring
        // context. `normal` / `<number>` survive as keywords by spec.
        //
        // `lh`/`rlh` used *within* this declaration's own value (self-reference)
        // use `ctx.root_line_height` — the page context's
        // CSS Values 4 §6.1.1 self-reference "parent" is the root element
        // (`page_context_line_height_basis` doc), the same source
        // `ctx.root_line_height` already carries.
        PropertyValue::LineHeight(lh) => PropertyValue::LineHeight(lift_line_height(
            resolve_line_height(lh, font_size, ctx.root_line_height, ctx),
        )),
        // ── text-indent ───────────────────────────────────────────────────
        // CSS Text 3 §8.1 — same `<length-percentage>` absolutization shape
        // as `padding-top` (`lp` helper above). The fact that this property
        // is *inherited* at the element cascade layer doesn't change this
        // function's job: `value` already arrived through
        // `resolve_against_inherited` (this arm is phase 3, run after phase
        // 2), so by this point it is this page context's own winner (or a
        // value already carried through inheritance) — either way, just a
        // `Length` that needs absolutizing against this context's own
        // `font_size`/`own_line_height` basis, same as every other box
        // property here.
        PropertyValue::TextIndent(v) => {
            PropertyValue::TextIndent(TextIndentValue {
                length: lp(v.length, font_size, own_line_height, ctx),
                hanging: v.hanging,
                each_line: v.each_line,
            })
        }
        // ── padding ───────────────────────────────────────────────────────
        PropertyValue::PaddingTop(v) => {
            PropertyValue::PaddingTop(lp(v, font_size, own_line_height, ctx))
        }
        PropertyValue::PaddingRight(v) => {
            PropertyValue::PaddingRight(lp(v, font_size, own_line_height, ctx))
        }
        PropertyValue::PaddingBottom(v) => {
            PropertyValue::PaddingBottom(lp(v, font_size, own_line_height, ctx))
        }
        PropertyValue::PaddingLeft(v) => {
            PropertyValue::PaddingLeft(lp(v, font_size, own_line_height, ctx))
        }
        // Shorthand fall-through — **unreachable through `cascade_page`**.
        // `crate::rule::expand_shorthand_into` runs at both boundaries that feed
        // this function: the parse exit (`parse_page_declaration_block`, this
        // module's own `@page`-block parser) and the `@page` cascade entry
        // (the candidate loop in `cascade_page` itself — the sibling of
        // `crate::cascade`'s `collect_cascaded`). The
        // entry-side expansion is what covers the post-parse mutation path
        // through the `pub` field `PageRule::declarations`.
        //
        // ⚠️ **これは "safety" net ではない — 到達したら既に bug である**
        // (element 側の同種の framing 訂正と同旨)。到達した
        // winner は `PropertyKey::Margin` 等の独立 key に park したまま
        // absolutize され、well-formed に見える値のまま `MarginTop` を読む
        // consumer から黙って消える — まさにこの fall-through が引き起こす
        // 典型的な failure mode である。degraded ではなく deterministic に
        // CSS Cascading
        // L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand> 違反であり、
        // 本 arm はそれを穏当に見せない。
        //
        // The arms are kept rather than folded into `unreachable!` for the same
        // reason `cascade::apply_value` keeps its shorthand arms: the guarantee
        // above is only partly compile-time enforced — the carve-outs are in the
        // expansion `match`'s own doc — and the crate
        // keeps the cascade panic-free as a deliberate design policy. Behaviour
        // is pinned directly
        // by `tests::absolutize_in_page_context_shorthand_fall_throughs`.
        PropertyValue::Padding(sides) => {
            PropertyValue::Padding(sides.map(|l| lp(l, font_size, own_line_height, ctx)))
        }
        // `padding-inline`/`padding-block` shorthand fall-through — same
        // shape and same unreachability rationale as `Padding` above
        // (`crate::property::PropertyValue::PaddingInline` doc covers the
        // physical-mapping choice). Pinned directly by
        // `tests::absolutize_in_page_context_logical_shorthand_fall_throughs`.
        PropertyValue::PaddingInline(pair) => {
            PropertyValue::PaddingInline(pair.map(|l| lp(l, font_size, own_line_height, ctx)))
        }
        PropertyValue::PaddingBlock(pair) => {
            PropertyValue::PaddingBlock(pair.map(|l| lp(l, font_size, own_line_height, ctx)))
        }
        // ── margin ────────────────────────────────────────────────────────
        PropertyValue::MarginTop(v) => {
            PropertyValue::MarginTop(margin_lpa(v, font_size, own_line_height, ctx))
        }
        PropertyValue::MarginRight(v) => {
            PropertyValue::MarginRight(margin_lpa(v, font_size, own_line_height, ctx))
        }
        PropertyValue::MarginBottom(v) => {
            PropertyValue::MarginBottom(margin_lpa(v, font_size, own_line_height, ctx))
        }
        PropertyValue::MarginLeft(v) => {
            PropertyValue::MarginLeft(margin_lpa(v, font_size, own_line_height, ctx))
        }
        // Inherit markers are normally resolved in phase 2. Keep them
        // panic-free if an internal caller bypasses that phase.
        v @ (PropertyValue::MarginTopInherit
        | PropertyValue::MarginRightInherit
        | PropertyValue::MarginBottomInherit
        | PropertyValue::MarginLeftInherit
        | PropertyValue::MarginInherit) => v,
        // Shorthand fall-through (see `Padding` above).
        PropertyValue::Margin(sides) => {
            PropertyValue::Margin(sides.map(|l| margin_lpa(l, font_size, own_line_height, ctx)))
        }
        // `margin-inline`/`margin-block` shorthand fall-through — same shape
        // as `PaddingInline`/`PaddingBlock` above.
        PropertyValue::MarginInline(pair) => {
            PropertyValue::MarginInline(pair.map(|l| margin_lpa(l, font_size, own_line_height, ctx)))
        }
        PropertyValue::MarginBlock(pair) => {
            PropertyValue::MarginBlock(pair.map(|l| margin_lpa(l, font_size, own_line_height, ctx)))
        }
        // ── border-*-width (absolutized **and** style-gated) ──────────────
        PropertyValue::BorderTopWidth(w) => PropertyValue::BorderTopWidth(border_width(
            w,
            border_styles.top,
            font_size,
            own_line_height,
            ctx,
        )),
        PropertyValue::BorderRightWidth(w) => PropertyValue::BorderRightWidth(border_width(
            w,
            border_styles.right,
            font_size,
            own_line_height,
            ctx,
        )),
        PropertyValue::BorderBottomWidth(w) => PropertyValue::BorderBottomWidth(border_width(
            w,
            border_styles.bottom,
            font_size,
            own_line_height,
            ctx,
        )),
        PropertyValue::BorderLeftWidth(w) => PropertyValue::BorderLeftWidth(border_width(
            w,
            border_styles.left,
            font_size,
            own_line_height,
            ctx,
        )),
        // Shorthand fall-through (see `Padding` above). Each side gates on the
        // style it carries itself, which is where a `border` shorthand's style
        // lives.
        PropertyValue::Border(sides) => {
            PropertyValue::Border(sides.map(|b| border(b, font_size, own_line_height, ctx)))
        }
        // `border-style` / `border-width` / `border-color` shorthand
        // fall-throughs (same unreachability rationale — rule.rs expands
        // them first). Styles and colors carry no lengths (passthrough);
        // widths absolutize against the companion side styles, exactly
        // like the `BorderTopWidth` longhand arms above.
        PropertyValue::BorderStyle(sides) => PropertyValue::BorderStyle(sides),
        PropertyValue::BorderWidth(sides) => {
            PropertyValue::BorderWidth(Sides {
                top: border_width(sides.top, border_styles.top, font_size, own_line_height, ctx),
                right: border_width(
                    sides.right,
                    border_styles.right,
                    font_size,
                    own_line_height,
                    ctx,
                ),
                bottom: border_width(
                    sides.bottom,
                    border_styles.bottom,
                    font_size,
                    own_line_height,
                    ctx,
                ),
                left: border_width(
                    sides.left,
                    border_styles.left,
                    font_size,
                    own_line_height,
                    ctx,
                ),
            })
        }
        PropertyValue::BorderColor(sides) => PropertyValue::BorderColor(sides),
        // ── border-radius / box-shadow / outline ─────────────────────────
        // CSS Backgrounds and Borders 3 §5/§6.1 and CSS UI 3 §4: all
        // length components are computed against this page context's own
        // font-size/line-height. Percentages are intentionally outside this
        // task's parser contract, so every surviving component round-trips
        // as `Length::Px` after phase 3.
        PropertyValue::BorderRadius(v) => PropertyValue::BorderRadius(border_radius_value(
            v,
            font_size,
            own_line_height,
            ctx,
        )),
        PropertyValue::BorderRadiusInherit => PropertyValue::BorderRadiusInherit,
        PropertyValue::BorderRadiusTopLeft(v) => PropertyValue::BorderRadiusTopLeft(Length::Px(
            resolve_length(v, font_size, own_line_height, ctx).px(),
        )),
        PropertyValue::BorderRadiusTopRight(v) => PropertyValue::BorderRadiusTopRight(Length::Px(
            resolve_length(v, font_size, own_line_height, ctx).px(),
        )),
        PropertyValue::BorderRadiusBottomRight(v) => {
            PropertyValue::BorderRadiusBottomRight(Length::Px(
                resolve_length(v, font_size, own_line_height, ctx).px(),
            ))
        }
        PropertyValue::BorderRadiusBottomLeft(v) => PropertyValue::BorderRadiusBottomLeft(Length::Px(
            resolve_length(v, font_size, own_line_height, ctx).px(),
        )),
        PropertyValue::BoxShadow(items) => PropertyValue::BoxShadow(if items.is_empty() {
            items
        } else {
            Arc::new(
                items
                    .iter()
                    .map(|item| box_shadow_item(*item, font_size, own_line_height, ctx))
                    .collect(),
            )
        }),
        PropertyValue::Outline(v) => {
            PropertyValue::Outline(outline_value(v, font_size, own_line_height, ctx))
        }
        PropertyValue::OutlineWidth(v) => {
            PropertyValue::OutlineWidth(outline_width(
                v,
                outline_style,
                font_size,
                own_line_height,
                ctx,
            ))
        }
        PropertyValue::OutlineOffset(v) => {
            PropertyValue::OutlineOffset(Length::Px(
                resolve_length(v, font_size, own_line_height, ctx).px(),
            ))
        }
        // ── width / height / max-* / min-* ──────────────────────────────
        PropertyValue::Width(v) => PropertyValue::Width(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::Height(v) => PropertyValue::Height(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::MaxWidth(v) => PropertyValue::MaxWidth(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::MaxHeight(v) => PropertyValue::MaxHeight(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::MinWidth(v) => PropertyValue::MinWidth(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::MinHeight(v) => PropertyValue::MinHeight(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::MinBlockSize(v) => PropertyValue::MinBlockSize(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::Top(v) => PropertyValue::Top(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::Right(v) => PropertyValue::Right(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::Bottom(v) => PropertyValue::Bottom(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::Left(v) => PropertyValue::Left(lpa(v, font_size, own_line_height, ctx)),
        // ── background-size / background-position ───────────────────────
        // CSS Backgrounds and Borders 3 §2.9/§2.6: both carry
        // `<length-percentage>` components, absolutized against this page
        // context's own font-size/line-height (same basis as `Width`/
        // `Height` above).
        PropertyValue::BackgroundSize(v) => {
            PropertyValue::BackgroundSize(background_size(v, font_size, own_line_height, ctx))
        }
        PropertyValue::BackgroundPosition(v) => {
            PropertyValue::BackgroundPosition(css_position(v, font_size, own_line_height, ctx))
        }
        // `background` shorthand fall-through (see `Padding` above for the
        // unreachability rationale) — unreachable in practice
        // (`expand_shorthand_into` expands it before this function ever sees
        // a winner). `position`/`size`/`image` are the length-bearing
        // components this function absolutizes here — same basis as the
        // `BackgroundPosition`/`BackgroundSize`/`BackgroundImage` longhand
        // arms just above (`Flex`/`Gap` below use the same "only the
        // length-bearing fields get transformed" shape). `image`'s
        // `Gradient` payload's font-relative `<length-percentage>` values
        // are absolutized while `<percentage>` stays symbolic (same split as
        // `BackgroundImage` above). Pinned directly by
        // `tests::absolutize_in_page_context_shorthand_fall_throughs`'s
        // `Background` case.
        PropertyValue::Background(shorthand) => PropertyValue::Background(BackgroundShorthand {
            image: crate::resolve::resolve_background_image(
                shorthand.image,
                font_size,
                own_line_height,
                ctx,
            ),
            position: css_position(shorthand.position, font_size, own_line_height, ctx),
            size: background_size(shorthand.size, font_size, own_line_height, ctx),
            ..shorthand
        }),
        // ── text-decoration-thickness / text-decoration-inset ────────────
        // ED §2.4.1/§2.9.1: both carry `<length-percentage>` components,
        // absolutized against this page context's own font-size/line-height
        // (same basis and same `Length::Px(resolve_length(..).px())` shape
        // as `OutlineOffset` above). `auto`/`from-font` survive as keywords
        // by spec, same as `LineHeight::Normal`.
        PropertyValue::TextDecorationThickness(t) => {
            PropertyValue::TextDecorationThickness(match t {
                TextDecorationThickness::Auto | TextDecorationThickness::FromFont => t,
                TextDecorationThickness::Length(l) => {
                    TextDecorationThickness::Length(Length::Px(
                        resolve_length(l, font_size, own_line_height, ctx).px(),
                    ))
                }
            })
        }
        PropertyValue::TextDecorationInset(inset) => {
            PropertyValue::TextDecorationInset(match inset {
                TextDecorationInset::Auto => TextDecorationInset::Auto,
                TextDecorationInset::Lengths { start, end } => {
                    TextDecorationInset::Lengths {
                        start: Length::Px(resolve_length(start, font_size, own_line_height, ctx).px()),
                        end: Length::Px(resolve_length(end, font_size, own_line_height, ctx).px()),
                    }
                }
            })
        }
        // CSS Text Decoration 4 §2.8: fixed underline offsets resolve
        // against the declaring page context's font-size; percentages stay
        // relative. Deferred mixed math falls back to `auto`.
        PropertyValue::TextUnderlineOffset(value) => {
            PropertyValue::TextUnderlineOffset(match value {
                LengthOrAuto::Auto => LengthOrAuto::Auto,
                // Percentages inherit as relative values.
                LengthOrAuto::Length(Length::Percent(percent)) => {
                    LengthOrAuto::Length(Length::Percent(percent))
                }
                LengthOrAuto::Length(length) => LengthOrAuto::Length(Length::Px(
                    resolve_length(length, font_size, own_line_height, ctx).px(),
                )),
                LengthOrAuto::Calc(_) => LengthOrAuto::Auto,
            })
        }
        // `text-decoration` shorthand fall-through (see `Padding` above for
        // the unreachability rationale) — unreachable in practice
        // (`expand_shorthand_into` expands it before this function ever sees
        // a winner). `thickness` is the only length-bearing component this
        // function absolutizes here (same basis as the
        // `TextDecorationThickness` longhand arm just above); the other 3
        // pass through via the struct-update `..shorthand`. Pinned directly
        // by `tests::absolutize_in_page_context_shorthand_fall_throughs`'s
        // `TextDecoration` case.
        PropertyValue::TextDecoration(shorthand) => {
            PropertyValue::TextDecoration(TextDecorationShorthand {
                thickness: match shorthand.thickness {
                    TextDecorationThickness::Auto | TextDecorationThickness::FromFont => {
                        shorthand.thickness
                    }
                    TextDecorationThickness::Length(l) => {
                        TextDecorationThickness::Length(Length::Px(
                            resolve_length(l, font_size, own_line_height, ctx).px(),
                        ))
                    }
                },
                ..shorthand
            })
        }
        // `font` shorthand fall-through (see `Padding` above for the
        // unreachability rationale) — unreachable in practice
        // (`expand_shorthand_into` expands it before this function ever sees
        // a winner). `size`/`line-height` are the length-bearing components
        // this function absolutizes here — same basis as the `FontSize`/
        // `LineHeight` longhand arms just above (`Flex`/`Gap`/`Background`
        // above use the same "only the length-bearing fields get
        // transformed" shape). Pinned directly by
        // `tests::absolutize_in_page_context_shorthand_fall_throughs`'s
        // `Font` case.
        PropertyValue::Font(shorthand) => PropertyValue::Font(FontShorthand {
            size: match shorthand.size {
                FontShorthandSize::Absolute(length) => FontShorthandSize::Absolute(Length::Px(
                    resolve_length(length, font_size, own_line_height, ctx).px(),
                )),
                FontShorthandSize::Relative(relative) => FontShorthandSize::Absolute(Length::Px(
                    resolve_relative_font_size(relative, font_size.px()),
                )),
            },
            line_height: lift_line_height(resolve_line_height(
                shorthand.line_height,
                font_size,
                ctx.root_line_height,
                ctx,
            )),
            ..shorthand
        }),
        // ── object-position ──────────────────────────────────────────────
        // CSS Images Module Level 3 §5.2: carries `<length-percentage>`
        // components via the reused `CssPosition` type (same shape and same
        // `css_position` helper as `BackgroundPosition` above).
        PropertyValue::ObjectPosition(v) => {
            PropertyValue::ObjectPosition(css_position(v, font_size, own_line_height, ctx))
        }
        // ── opacity ───────────────────────────────────────────────────────
        // CSS Color 4 §3.3: "Opacity values outside the range `[0, 1]` are
        // not invalid, and are preserved in specified values, but are
        // clamped to the range `[0, 1]` in computed values." This is the
        // page-context sibling of
        // `crate::specified::SpecifiedValues::absolutize_with`'s opacity
        // clamp — a real phase-3 transform, but not a length
        // absolutization, so it does not need `font_size`/`own_line_height`/
        // `ctx` the way the arms above do.
        PropertyValue::Opacity(o) => PropertyValue::Opacity(o.clamp(0.0, 1.0)),
        // ── overflow-x / overflow-y ──────────────────────────────────────────
        // CSS Overflow 3 §3.1 cross-axis coupling — this axis's own winner
        // (`v`) paired with the *other* axis's winner (`overflow_pair`,
        // `page_context_overflow_pair`'s output), same shape as
        // `border_width` pairing `w` with `border_styles.top` above.
        PropertyValue::OverflowX(v) => PropertyValue::OverflowX(
            resolve_overflow(OverflowXY {
                x: v,
                y: overflow_pair.y,
            })
            .x,
        ),
        PropertyValue::OverflowY(v) => PropertyValue::OverflowY(
            resolve_overflow(OverflowXY {
                x: overflow_pair.x,
                y: v,
            })
            .y,
        ),
        // `overflow` shorthand fall-through (see `Border` above) — unreachable
        // in practice (`expand_shorthand_into` expands it before this
        // function ever sees a winner), not a safety net if it were.
        PropertyValue::Overflow(pair) => PropertyValue::Overflow(resolve_overflow(pair)),
        // ── writing-mode ─────────────────────────────────────────────────
        // CSS Writing Modes 4 §3.2 — same-node-only normalization (no length,
        // no cross-property/parent dependency), applied here rather than in
        // `resolve_against_inherited` for the same reason `overflow-x`/
        // `overflow-y`'s cross-axis coupling is: it does not depend on the
        // inheritance parent, only phase 3 runs it. See `WritingMode` doc's
        // Non-goal section and `resolve_writing_mode` doc for why every
        // non-`horizontal-tb` keyword collapses here.
        PropertyValue::WritingMode(v) => PropertyValue::WritingMode(resolve_writing_mode(v)),
        PropertyValue::RubyPosition(v) => PropertyValue::RubyPosition(v),
        // ── letter-spacing / word-spacing ───────────────────────────────────
        // CSS Text 3 §7.2 / §7.1: `normal | <length>`, absolutized the same
        // way `crate::specified::SpecifiedValues::absolutize_with` does
        // (`resolve_length_or_normal`), then mapped back into the
        // specified-layer `LengthOrNormal` shape (`lift_length_or_normal`)
        // that `PropertyValue` carries — same round-trip as `lp`/`lpa` above,
        // reusing the shared element-path functions directly since neither
        // needs page-context-specific integration logic.
        PropertyValue::LetterSpacing(v) => PropertyValue::LetterSpacing(lift_length_or_normal(
            resolve_length_or_normal(v, font_size, own_line_height, ctx),
        )),
        PropertyValue::WordSpacing(v) => PropertyValue::WordSpacing(lift_length_or_normal(
            resolve_length_or_normal(v, font_size, own_line_height, ctx),
        )),
        // ── tab-size ─────────────────────────────────────────────────────
        // CSS Text Module Level 3 §4.2: `<number [0,∞]> | <length [0,∞]>`,
        // absolutized the same way
        // `crate::specified::SpecifiedValues::absolutize_with` does
        // (`resolve_tab_size`), then mapped back into the specified-layer
        // `TabSize` shape (`lift_tab_size`) that `PropertyValue` carries —
        // same round-trip as `LetterSpacing`/`WordSpacing` above, reusing
        // the shared element-path functions directly since neither needs
        // page-context-specific integration logic.
        PropertyValue::TabSize(v) => {
            PropertyValue::TabSize(lift_tab_size(resolve_tab_size(v, font_size, own_line_height, ctx)))
        }
        // ── border-spacing ─────────────────────────────────────────────────
        // CSS Tables 3 §6.1: `<length>{1,2}`, absolutized the same way
        // `crate::specified::SpecifiedValues::absolutize_with` does
        // (`resolve_border_spacing`), then mapped back into the
        // specified-layer `BorderSpacingValue` shape (`lift_border_spacing`)
        // that `PropertyValue` carries — same round-trip as
        // `LetterSpacing`/`WordSpacing`/`TabSize` above, reusing the shared
        // element-path functions directly since neither needs
        // page-context-specific integration logic.
        PropertyValue::BorderSpacing(v) => {
            PropertyValue::BorderSpacing(lift_border_spacing(resolve_border_spacing(
                v,
                font_size,
                own_line_height,
                ctx,
            )))
        }
        // ── text-shadow ─────────────────────────────────────────────────────
        // CSS Text Decoration Module Level 3 §4: each item's 3 lengths
        // (`offset-x`/`offset-y`/`blur-radius`) are absolutized against this
        // context's own `font_size`/`own_line_height` basis (`text_shadow_item`
        // helper above); `<color>` carries no length and passes through.
        // `none` (empty list) needs no allocation — `items` is reused as-is.
        PropertyValue::TextShadow(items) => PropertyValue::TextShadow(if items.is_empty() {
            items
        } else {
            Arc::new(
                items
                    .iter()
                    .map(|item| text_shadow_item(*item, font_size, own_line_height, ctx))
                    .collect(),
            )
        }),
        // ── vertical-align ───────────────────────────────────────────────
        // CSS 2.1 §10.8.1 — the bare keywords are preserved as-is,
        // `<length>` / `<percentage>` absolutized (`<percentage>` is
        // `used_line_height_length` basis with `0px` fallback when
        // `line-height: normal` — `resolve_vertical_align` doc).
        // `resolve_vertical_align` already returns the same
        // `VerticalAlign` shape `PropertyValue::VerticalAlign` carries (no
        // separate `Computed*` type exists for this property — see that
        // function's doc for why — so unlike `lp`/`fb`/`lift_length_or_normal`
        // above there is no round-trip conversion needed here).
        PropertyValue::VerticalAlign(v) => {
            PropertyValue::VerticalAlign(resolve_vertical_align(v, font_size, own_line_height, ctx))
        }
        // ── flex-basis ────────────────────────────────────────────────────
        // CSS Flexible Box Layout Module Level 1 §7.2.3 — `content`/`auto`
        // preserved as keywords, `<length-percentage>` absolutized (`fb`
        // helper above).
        PropertyValue::FlexBasis(v) => {
            PropertyValue::FlexBasis(fb(v, font_size, own_line_height, ctx))
        }
        // Shorthand fall-through (see `Padding` above) — unreachable in
        // practice (`expand_shorthand_into` expands it before this function
        // ever sees a winner). `grow`/`shrink` carry no length; `basis`
        // gets the same `fb` treatment as the `FlexBasis` longhand above.
        PropertyValue::Flex(f) => PropertyValue::Flex(FlexShorthand {
            grow: f.grow,
            shrink: f.shrink,
            basis: fb(f.basis, font_size, own_line_height, ctx),
        }),
        // ── row-gap / column-gap ─────────────────────────────────────────
        // CSS Box Alignment Module Level 3 §8.1 — `normal` preserved as a
        // keyword (`lpn` helper above, unlike `letter-spacing`/
        // `word-spacing`'s `normal → 0` collapse).
        PropertyValue::RowGap(v) => {
            PropertyValue::RowGap(lpn(v, font_size, own_line_height, ctx))
        }
        PropertyValue::ColumnGap(v) => {
            PropertyValue::ColumnGap(lpn(v, font_size, own_line_height, ctx))
        }
        // Shorthand fall-through (see `Padding` above) — unreachable in
        // practice, same shape as `Flex` above.
        PropertyValue::Gap(g) => PropertyValue::Gap(GapShorthand {
            row: lpn(g.row, font_size, own_line_height, ctx),
            column: lpn(g.column, font_size, own_line_height, ctx),
        }),
        // ── grid-template-columns / grid-template-rows ──────────────────────
        // CSS Grid Layout Module Level 1 §7.2 — `none` preserved as a
        // keyword, `<length-percentage>` inside the track list absolutized
        // (`gtt` helper above).
        PropertyValue::GridTemplateColumns(v) => {
            PropertyValue::GridTemplateColumns(gtt(v, font_size, own_line_height, ctx))
        }
        PropertyValue::GridTemplateRows(v) => {
            PropertyValue::GridTemplateRows(gtt(v, font_size, own_line_height, ctx))
        }
        // ── grid-auto-columns / grid-auto-rows ───────────────────────────────
        // CSS Grid Layout Module Level 1 §7.6 — same track-size
        // absolutization as `GridTemplateColumns` above (`gatl` helper).
        PropertyValue::GridAutoColumns(v) => {
            PropertyValue::GridAutoColumns(gatl(&v, font_size, own_line_height, ctx))
        }
        PropertyValue::GridAutoRows(v) => {
            PropertyValue::GridAutoRows(gatl(&v, font_size, own_line_height, ctx))
        }
        // ── transform ────────────────────────────────────────────────────
        // CSS Transforms Level 1 §4: Computed value is "as specified, but
        // with lengths made absolute" — `translate()`/`translateX()`/
        // `translateY()`'s `<length-percentage>` slot の length 側だけを
        // `lp` と同じ split で絶対化し percentage 側は `Percent` のまま残す
        // (`resolve_length_percentage` と同じ、`background-position` の
        // `css_position` helper と同型)。`matrix` の 6 `<number>` slot と
        // `rotate`/`skew` 系の `<angle>` slot はそのまま。
        PropertyValue::Transform(items) => {
            if items.is_empty() {
                PropertyValue::Transform(items)
            } else {
                PropertyValue::Transform(Arc::new(
                    items
                        .iter()
                        .map(|f| transform_function(*f, font_size, own_line_height, ctx))
                        .collect(),
                ))
            }
        }
    }
}

/// Page-selector specificity triple `(f, g, h)` per CSS Paged Media L3
/// §"Cascading and page context":
///
/// - `f` = count of page type names (named ident, syntactically 0 or 1)
/// - `g` = count of `:first` or `:blank` pseudo-classes
/// - `h` = count of `:left` or `:right` pseudo-classes
///
/// Compared lexicographically (`f` beats `g` beats `h`), via
/// `#[derive(Ord)]` on the field order.
///
/// Selected examples from the spec:
///
/// - `@page { }`        → `(0, 0, 0)`
/// - `@page :left { }`  → `(0, 0, 1)`
/// - `@page :first { }` → `(0, 1, 0)`
/// - `@page artsy { }`  → `(1, 0, 0)`
///
/// A `u32` per component is more than enough — `f` is bounded to 1 by the
/// grammar, and `g` / `h` in practice count `Vec<PagePseudo>` entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PageSpecificity {
    f: u32,
    g: u32,
    h: u32,
}

/// One page custom-property candidate. Custom names are cascaded separately
/// from the ordinary `PropertyKey` table because CSS Variables names compare
/// case-sensitively.
type PageCustomCascadedDecl = (CustomProperty, bool, Origin, u32, PageSpecificity, u32);

/// Match a single compound `<page-selector>` entry against `query`.
///
/// Returns the entry's [`PageSpecificity`] on a match, `None` otherwise.
/// Matching semantics per L3 §"@page rule grammar":
///
/// - Optional ident: if the entry has one, `query.page_name` must be `Some`
///   and byte-equal to it (`<custom-ident>` case-sensitivity, CSS Values L4
///   §4.2). An entry with no ident matches every page.
/// - Reserved-keyword exception: an ident that is `auto` under ASCII
///   case-insensitive comparison never matches. CSS Paged Media L3
///   §"Page selectors" states: "A page type name of auto (ASCII
///   case-insensitive) does not make the rule invalid, but must never
///   match." The parser therefore accepts `@page auto { … }` (the rule
///   is spec-valid syntax) and the exclusion lives here, on the cascade
///   side; see
///   <https://www.w3.org/TR/css-page-3/#page-selectors>.
/// - Zero-or-more pseudo-pages: **all** must match (compound = AND).
fn match_page_entry(
    entry: &PageSelectorEntry,
    query: &PageContextQuery,
) -> Option<PageSpecificity> {
    if let Some(ident) = &entry.ident {
        // Reserved-keyword exclusion: `auto` (ASCII case-insensitive) is
        // spec-valid but must never match — CSS Paged Media L3
        // §"Page selectors", `#page-selectors`. Applied *before* the
        // name compare so no `query.page_name` value (including a
        // literal `"auto"` custom-ident from an author `page` property)
        // can bypass the rule.
        if ident.0.eq_ignore_ascii_case("auto") {
            return None;
        }
        match &query.page_name {
            Some(name) if ident == name => {}
            _ => return None,
        }
    }
    for pseudo in &entry.pseudos {
        let ok = match pseudo {
            PagePseudo::First => query.is_first,
            PagePseudo::Left => query.is_left,
            PagePseudo::Right => query.is_right,
            PagePseudo::Blank => query.is_blank,
        };
        if !ok {
            return None;
        }
    }
    let f = if entry.ident.is_some() { 1 } else { 0 };
    let g = entry
        .pseudos
        .iter()
        .filter(|p| matches!(p, PagePseudo::First | PagePseudo::Blank))
        .count() as u32;
    let h = entry
        .pseudos
        .iter()
        .filter(|p| matches!(p, PagePseudo::Left | PagePseudo::Right))
        .count() as u32;
    Some(PageSpecificity { f, g, h })
}

/// Winner tie-break: `candidate` beats `existing` when its
/// `(rank, specificity, source_order)` triple is `>=` the existing's.
///
/// Sibling arm to `cascade::beats`. The `>=` is intentional and mirrors the
/// style-rule tie-break: within a single `@page` rule, later declarations of
/// the same property beat earlier ones (CSS Cascading L4 §6.1
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> "Order of Appearance");
/// across rules, `source_order` is monotonically increasing so `>` and `>=`
/// coincide.
fn page_beats(
    candidate: &(u8, u32, PageSpecificity, u32, PropertyValue),
    existing: &(u8, u32, PageSpecificity, u32, PropertyValue),
) -> bool {
    (candidate.0, candidate.1, candidate.2, candidate.3)
        >= (existing.0, existing.1, existing.2, existing.3)
}

/// Compare a descriptor candidate, including declaration order within one rule.
fn descriptor_beats<T>(
    candidate: &(u8, u32, PageSpecificity, u32, u32, T),
    existing: &(u8, u32, PageSpecificity, u32, u32, T),
) -> bool {
    (
        candidate.0,
        candidate.1,
        candidate.2,
        candidate.3,
        candidate.4,
    ) >= (existing.0, existing.1, existing.2, existing.3, existing.4)
}

/// Resolve descriptor lengths using the computed page-context font bases.
fn absolutize_page_size(
    size: PageSize,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> PageSize {
    match size {
        PageSize::Lengths { width, height } => PageSize::Lengths {
            width: Length::Px(resolve_length(width, font_size, own_line_height, ctx).px()),
            height: Length::Px(resolve_length(height, font_size, own_line_height, ctx).px()),
        },
        other => other,
    }
}

fn absolutize_page_bleed(
    bleed: PageBleed,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> PageBleed {
    match bleed {
        PageBleed::Length(length) => PageBleed::Length(Length::Px(
            resolve_length(length, font_size, own_line_height, ctx).px(),
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests;
