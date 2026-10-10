use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use smol_str::SmolStr;

use crate::Atom;
use crate::cascade::{
    ResolvedAgainstInherited, cascade_rank, resolve_against_inherited, resolve_css_wide_color_font,
    resolve_custom_property_environment, resolve_page_deferred_value,
};
use crate::computed::{ComputedValues, CustomPropertyEnvironment, empty_custom_properties};
use crate::media::MediaContext;
use crate::property::{
    BackgroundImage, BorderStyle, CssColor, CustomProperty, Length, OutlineStyle, OverflowValue,
    OverflowXY, PropertyKey, PropertyValue, Sides,
};
use crate::resolve::{
    ComputedLength, ResolveContext, resolve_length, resolve_line_height, used_line_height_length,
};
use crate::rule::{Declaration, ParsedDeclaration, expand_shorthand_into};
use crate::ruletree::{Origin, RuleTree};
use crate::specified::INITIAL_BORDER;

use super::absolutize::absolutize_in_page_context;
use super::types::*;

// ---------------------------------------------------------------------------
// Actual @page cascade order implementation
//
// CSS Paged Media Level 3, §"Cascading and page context" —
//   <https://www.w3.org/TR/css-page-3/#cascading-and-page-context>
// CSS Cascading and Inheritance Level 4, §"Cascade Origin" —
//   <https://www.w3.org/TR/css-cascade-4/#cascade-origin>
//
// # Sibling arm convention
//
// Follows the convention of the element cascade's winner selection
// (`crate::cascade::pick_winners`): per candidate a value with its importance,
// origin, specificity and source order, grouped by `PropertyKey`, the winner
// picked by `(rank, specificity, source_order)`
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
/// (design specification §9.1 "page name transition rules"). raikiri-style does
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

#[derive(Debug, Clone, PartialEq)]
struct MarginBoxInheritance {
    color: CssColor,
    background_color: CssColor,
    background_color_expression: Option<SmolStr>,
    font_size: ComputedLength,
    custom_properties: Arc<CustomPropertyEnvironment>,
}

impl Default for MarginBoxInheritance {
    fn default() -> Self {
        Self {
            color: CssColor::BLACK,
            background_color: CssColor::TRANSPARENT,
            background_color_expression: None,
            font_size: ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
            custom_properties: empty_custom_properties(),
        }
    }
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
    /// Normal layer rank evaluated for the enclosing page media context.
    layer_order: u32,
}

impl PageMarginBoxCascadeResult {
    /// Cascade matching rules for one margin-box slot, preserving declaration
    /// importance, origin, layer, page-selector specificity and source order.
    /// Uses initial inheritance values for supported defaulting longhands.
    /// Use [`PageCascadeResult::cascade_margin_box`] to inherit from an actual page.
    /// Returns `None` when no rule targets the slot.
    pub fn cascade_matching(rules: &[Self], slot: PageMarginBoxSlot) -> Option<Self> {
        Self::cascade_with_inheritance(rules, slot, &MarginBoxInheritance::default())
    }

    fn cascade_with_inheritance(
        rules: &[Self],
        slot: PageMarginBoxSlot,
        inherited: &MarginBoxInheritance,
    ) -> Option<Self> {
        let mut merged = rules.iter().rev().find(|rule| rule.slot == slot)?.clone();
        let candidates: Vec<_> = rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.slot == slot)
            .flat_map(|(rule_index, rule)| {
                rule.declarations
                    .iter()
                    .enumerate()
                    .map(move |(decl_index, decl)| (rule_index, decl_index, rule, decl))
            })
            .collect();
        let local_custom =
            crate::cascade::select_custom_rollback_values(candidates.iter().filter_map(
                |&(rule_index, decl_index, rule, decl)| {
                    let PropertyValue::CustomProperty(custom) = &decl.value else {
                        return None;
                    };
                    let layer = crate::layer::LayerPosition {
                        attached: false,
                        rank: rule.layer_order,
                    };
                    Some((
                        custom,
                        (
                            cascade_rank(rule.origin, decl.important),
                            layer.priority(decl.important),
                            rule.specificity,
                            rule.source_order,
                            rule_index,
                            decl_index,
                        ),
                        rule.origin,
                        layer,
                        decl.important,
                    ))
                },
            ));
        let custom_properties =
            resolve_custom_property_environment(&inherited.custom_properties, &local_custom);
        let resolved_value = |value: &PropertyValue| {
            if let PropertyValue::Deferred(deferred) = value
                && matches!(
                    deferred.key,
                    PropertyKey::Color | PropertyKey::BackgroundColor | PropertyKey::FontSize
                )
            {
                resolve_page_deferred_value(deferred, custom_properties.as_ref())
            } else {
                Some(value.clone())
            }
        };
        let rollback = |value: &PropertyValue| {
            resolved_value(value).as_ref().map_or(
                crate::cascade::rollback::Rollback::None,
                crate::cascade::rollback::rollback_kind,
            )
        };
        let mut winners = HashMap::new();
        let mut has_rollback = false;
        for (index, &(rule_index, decl_index, rule, decl)) in candidates.iter().enumerate() {
            if matches!(decl.value, PropertyValue::AllRevertLayer) {
                has_rollback = true;
                continue;
            }
            has_rollback |= matches!(&decl.value, PropertyValue::Deferred(_))
                && rollback(&decl.value) != crate::cascade::rollback::Rollback::None;
            let name = match &decl.value {
                PropertyValue::CustomProperty(custom) => {
                    has_rollback |= crate::cascade::custom_property_rollback(&custom.value)
                        != crate::cascade::rollback::Rollback::None;
                    Some(custom.name.clone())
                }
                _ => None,
            };
            let layer = crate::layer::LayerPosition {
                attached: false,
                rank: rule.layer_order,
            };
            let priority = (
                cascade_rank(rule.origin, decl.important),
                layer.priority(decl.important),
                rule.specificity,
                rule.source_order,
                rule_index,
                decl_index,
            );
            let entry = winners
                .entry((decl.value.key(), name))
                .or_insert((priority, index));
            if priority >= entry.0 {
                *entry = (priority, index);
            }
        }
        if has_rollback {
            winners.retain(|(key, name), winner| {
                if matches!(key, PropertyKey::Direction | PropertyKey::UnicodeBidi) {
                    return true;
                }
                let selected = crate::cascade::rollback::select_layered_winner(
                    &candidates,
                    |_, &(rule_index, decl_index, rule, decl)| {
                        let same_property = decl.value.key() == *key
                            && match (&decl.value, name) {
                                (PropertyValue::CustomProperty(custom), Some(name)) => {
                                    custom.name == *name
                                }
                                _ => true,
                            };
                        let reset_all = *key != PropertyKey::Custom
                            && matches!(decl.value, PropertyValue::AllRevertLayer);
                        if !same_property && !reset_all {
                            return None;
                        }
                        let layer = crate::layer::LayerPosition {
                            attached: false,
                            rank: rule.layer_order,
                        };
                        Some((
                            (
                                cascade_rank(rule.origin, decl.important),
                                layer.priority(decl.important),
                                rule.specificity,
                                rule.source_order,
                                rule_index,
                                decl_index,
                            ),
                            rule.origin,
                            layer,
                            decl.important,
                            rollback(&decl.value),
                        ))
                    },
                );
                if let Some(index) = selected {
                    winner.1 = index;
                    true
                } else {
                    false
                }
            });
        }
        let mut winners: Vec<_> = winners
            .into_values()
            .map(|(_, index)| candidates[index])
            .collect();
        winners.sort_unstable_by_key(|(rule_index, decl_index, _, _)| (*rule_index, *decl_index));
        merged.declarations = winners
            .into_iter()
            .filter_map(|(_, _, _, decl)| {
                let value = resolved_value(&decl.value)?;
                Some(Declaration::new(
                    resolve_css_wide_color_font(
                        value,
                        inherited.color,
                        inherited.background_color,
                        inherited.background_color_expression.as_ref(),
                        inherited.font_size,
                    ),
                    decl.important,
                ))
            })
            .collect();
        let own_color = merged
            .declarations
            .iter()
            .find_map(|declaration| match &declaration.value {
                PropertyValue::Color(color) => Some(*color),
                PropertyValue::ContextualColor(color) if color.key == PropertyKey::Color => {
                    crate::property::resolve_contextual_color(&color.source, inherited.color)
                }
                _ => None,
            })
            .unwrap_or(inherited.color);
        for declaration in &mut merged.declarations {
            if let PropertyValue::ContextualColor(expression) = &declaration.value {
                let is_color = expression.key == PropertyKey::Color;
                let color = crate::property::resolve_contextual_color(
                    &expression.source,
                    if is_color { inherited.color } else { own_color },
                )
                .unwrap_or(if is_color {
                    inherited.color
                } else {
                    CssColor::TRANSPARENT
                });
                let resolved = if is_color {
                    PropertyValue::Color(color)
                } else {
                    PropertyValue::BackgroundColor(color)
                };
                declaration.update_value(|value| *value = resolved);
            }
        }
        Some(merged)
    }
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
#[derive(Debug, Clone, Default, PartialEq)]
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
    margin_box_inheritance: MarginBoxInheritance,
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

    /// Visits URL-bearing page and page-margin-box background images mutably.
    ///
    /// These backgrounds are cascaded before an embedder resolves relative
    /// resource URLs. This narrow visitor lets the embedder replace those URLs
    /// without exposing the declaration maps for unrestricted mutation.
    pub fn for_each_background_image_mut(&mut self, mut visitor: impl FnMut(&mut BackgroundImage)) {
        if let Some(PropertyValue::BackgroundImage(image)) =
            self.declarations.get_mut(&PropertyKey::BackgroundImage)
        {
            visitor(image);
        }
        for margin_box in &mut self.margin_boxes {
            for declaration in &mut margin_box.declarations {
                if matches!(declaration.value, PropertyValue::BackgroundImage(_)) {
                    declaration.update_value(|value| {
                        if let PropertyValue::BackgroundImage(image) = value {
                            visitor(image);
                        }
                    });
                }
            }
        }
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

    /// Cascade a margin-box slot against this page's computed inheritance values.
    ///
    /// Color, background-color and font-size defaulting and variable substitution
    /// are resolved before returning declarations to the layout/paint consumer.
    pub fn cascade_margin_box(
        &self,
        slot: PageMarginBoxSlot,
    ) -> Option<PageMarginBoxCascadeResult> {
        PageMarginBoxCascadeResult::cascade_with_inheritance(
            &self.margin_boxes,
            slot,
            &self.margin_box_inheritance,
        )
    }

    /// Matching parsed margin-box declarations, in source order.
    /// Use [`Self::cascade_margin_box`] to resolve a slot for layout or painting.
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
fn page_layer_rank(rule: &PageRule, important: bool, layers: &crate::layer::LayerOrder<'_>) -> u32 {
    layers.priority(rule.layer, rule.origin, important)
}

/// Page and margin boxes are laid out in horizontal-tb, so their logical
/// preferred sizes are `width` / `height` (CSS Logical Properties 1 §4.1)
/// and compete with them in one winner slot.
fn horizontal_preferred_size(value: PropertyValue) -> PropertyValue {
    match value {
        PropertyValue::InlineSize(size) => PropertyValue::Width(size),
        PropertyValue::BlockSize(size) => PropertyValue::Height(size),
        // A `var()` value is re-parsed by its property name once resolved.
        PropertyValue::Deferred(mut deferred) => {
            let physical = match deferred.key {
                PropertyKey::InlineSize => Some((PropertyKey::Width, "width")),
                PropertyKey::BlockSize => Some((PropertyKey::Height, "height")),
                _ => None,
            };
            if let Some((key, property)) = physical {
                deferred.key = key;
                deferred.property = SmolStr::new_static(property);
            }
            PropertyValue::Deferred(deferred)
        }
        value => value,
    }
}

pub fn cascade_page(
    rule_tree: &RuleTree,
    query: &PageContextQuery,
    inheritance: PageInheritance<'_>,
) -> PageCascadeResult {
    cascade_page_with_media_context(rule_tree, query, inheritance, &MediaContext::default())
}

/// Cascade all `@page` rules in `rule_tree` against `query` and `media_context`.
///
/// This is the media-aware sibling of [`cascade_page`]. Rules with no media
/// condition always apply. Rules nested in `@media` apply only when their
/// intersected condition matches [`crate::media::MediaContext`], mirroring the
/// element cascade media prefilter. Paper-dimension conditions suppress `size`
/// descriptors only, as required by CSS Paged Media 3 §7.1.
pub fn cascade_page_with_media_context(
    rule_tree: &RuleTree,
    query: &PageContextQuery,
    inheritance: PageInheritance<'_>,
    media_context: &MediaContext,
) -> PageCascadeResult {
    let layers = rule_tree.layer_order(media_context);
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
        if rule
            .media_condition
            .as_ref()
            .is_some_and(|condition| !condition.matches(media_context))
        {
            continue;
        }
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
            let ignore_size = rule
                .media_condition
                .as_ref()
                .is_some_and(|condition| condition.depends_on_paper_size());
            for (index, decl) in rule
                .size_declarations
                .iter()
                .enumerate()
                .filter(|_| !ignore_size)
            {
                let candidate = (
                    cascade_rank(rule.origin, decl.important),
                    page_layer_rank(rule, decl.important, &layers),
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
                    page_layer_rank(rule, decl.important, &layers),
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
                    page_layer_rank(rule, decl.important, &layers),
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
                    declarations: margin_box
                        .declarations
                        .iter()
                        .map(|decl| {
                            Declaration::new(
                                horizontal_preferred_size(decl.value.clone()),
                                decl.important,
                            )
                        })
                        .collect(),
                    source_order: rule.source_order,
                    origin: rule.origin,
                    specificity: (spec.f, spec.g, spec.h),
                    layer_order: layers.rank(rule.layer, rule.origin),
                });
            }
            for decl in &rule.declarations {
                if let PropertyValue::CustomProperty(custom) = &decl.value {
                    custom_candidates.push((
                        custom.clone(),
                        decl.important,
                        rule.origin,
                        page_layer_rank(rule, decl.important, &layers),
                        spec,
                        rule.source_order,
                    ));
                    continue;
                }
                // Expand again before pushing candidates. The page parser
                // already expanded every declaration, so this is the identity
                // for them; `PageRule::declarations` is a `pub` field, and
                // re-running the expansion keeps the page cascade independent
                // of how a consumer reassembled the list.
                // Rationale is consolidated in `crate::rule::expand_shorthand_into`.
                let parsed = ParsedDeclaration {
                    value: decl.value.clone(),
                    important: decl.important,
                };
                expand_shorthand_into(&parsed, |d| {
                    candidates.push((
                        horizontal_preferred_size(d.value),
                        d.important,
                        rule.origin,
                        page_layer_rank(rule, d.important, &layers),
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
    for (value, important, origin, layer, spec, order) in &custom_candidates {
        let name = value.name.clone();
        let rank = cascade_rank(*origin, *important);
        let replace = custom_best.get(&name).is_none_or(|existing| {
            (rank, *layer, *spec, *order) >= (existing.0, existing.1, existing.2, existing.3)
        });
        if replace {
            custom_best.insert(name, (rank, *layer, *spec, *order, value.clone()));
        }
    }
    let custom_local = if custom_best.values().any(|(_, _, _, _, value)| {
        crate::cascade::custom_property_rollback(&value.value)
            != crate::cascade::rollback::Rollback::None
    }) {
        crate::cascade::select_custom_rollback_values(custom_candidates.iter().enumerate().map(
            |(index, (value, important, origin, priority, spec, order))| {
                (
                    value,
                    (
                        cascade_rank(*origin, *important),
                        *priority,
                        *spec,
                        *order,
                        index,
                    ),
                    *origin,
                    crate::layer::LayerPosition {
                        attached: false,
                        rank: if *important {
                            u32::MAX - *priority
                        } else {
                            *priority
                        },
                    },
                    *important,
                )
            },
        ))
    } else {
        custom_best
            .into_iter()
            .map(|(name, (_, _, _, _, value))| (name, value.value))
            .collect()
    };

    // Resolve custom properties and defaulting against the page context's inheritance
    // parent. `PageInheritance::LegacyInitialValues` falls back to the initial
    // values, which the L3 legacy exception in CSS Paged Media 3 §6 "Page
    // Properties" permits explicitly (see `PageInheritance`'s doc). Shared
    // static: the fallback is immutable and `initial()` costs a heap allocation,
    // which the `LegacyInitialValues` path would otherwise take on every call
    // (following the `empty_counter_entries` precedent in `property.rs`).
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
    // Defaulting after substitution participates in rollback before selecting
    // ordinary winners. Invalid computed values still win their own slot.
    let candidate_rollback = |value: &PropertyValue| {
        if let PropertyValue::Deferred(deferred) = value {
            resolve_page_deferred_value(deferred, custom_properties.as_ref())
                .as_ref()
                .map_or(
                    crate::cascade::rollback::Rollback::None,
                    crate::cascade::rollback::rollback_kind,
                )
        } else {
            crate::cascade::rollback::rollback_kind(value)
        }
    };

    // Winner selection — sibling arm to `cascade::pick_winners`.
    let mut best: HashMap<PropertyKey, (u8, u32, PageSpecificity, u32, PropertyValue)> =
        HashMap::new();
    let mut winner_indices = HashMap::new();
    let mut has_rollback = false;
    let mut has_property_rollback = false;
    for (index, (value, important, origin, layer, spec, order)) in candidates.iter().enumerate() {
        if matches!(value, PropertyValue::AllRevertLayer) {
            has_rollback = true;
            continue;
        }
        has_property_rollback |= matches!(value, PropertyValue::Deferred(_))
            && candidate_rollback(value) != crate::cascade::rollback::Rollback::None;
        let rank = cascade_rank(*origin, *important);
        let key = value.key();
        let candidate = (rank, *layer, *spec, *order, value.clone());
        match best.get(&key) {
            Some(existing) => {
                if page_beats(&candidate, existing) {
                    best.insert(key, candidate);
                    winner_indices.insert(key, index);
                }
            }
            None => {
                best.insert(key, candidate);
                winner_indices.insert(key, index);
            }
        }
    }

    if has_rollback || has_property_rollback {
        best.retain(|key, winner| {
            if !has_rollback && !matches!(&winner.4, PropertyValue::Deferred(_)) {
                return true;
            }
            if matches!(key, PropertyKey::Direction | PropertyKey::UnicodeBidi) {
                return true;
            }
            let selected = crate::cascade::rollback::select_layered_winner(
                &candidates,
                |idx, (value, important, origin, priority, spec, order)| {
                    if value.key() != *key && !matches!(value, PropertyValue::AllRevertLayer) {
                        return None;
                    }
                    let layer = crate::layer::LayerPosition {
                        attached: false,
                        rank: if *important {
                            u32::MAX - *priority
                        } else {
                            *priority
                        },
                    };
                    Some((
                        (
                            cascade_rank(*origin, *important),
                            *priority,
                            *spec,
                            *order,
                            idx,
                        ),
                        *origin,
                        layer,
                        *important,
                        candidate_rollback(value),
                    ))
                },
            );
            if let Some(index) = selected {
                winner_indices.insert(*key, index);
                let (value, important, origin, layer, spec, order) = &candidates[index];
                *winner = (
                    cascade_rank(*origin, *important),
                    *layer,
                    *spec,
                    *order,
                    value.clone(),
                );
                true
            } else {
                false
            }
        });
    }

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
    let radius_keys = [
        PropertyKey::BorderRadiusTopLeft,
        PropertyKey::BorderRadiusTopRight,
        PropertyKey::BorderRadiusBottomRight,
        PropertyKey::BorderRadiusBottomLeft,
    ];
    let radius_precedence: HashMap<_, _> = best
        .iter()
        .filter(|(key, _)| **key == PropertyKey::BorderRadius || radius_keys.contains(key))
        .map(|(&key, winner)| {
            (
                key,
                (winner.0, winner.1, winner.2, winner.3, winner_indices[&key]),
            )
        })
        .collect();
    let mut resolved: HashMap<PropertyKey, ResolvedAgainstInherited> = best
        .into_iter()
        .filter_map(|(k, (_, _, _, _, value))| {
            let value = match value {
                PropertyValue::Deferred(deferred) => {
                    resolve_page_deferred_value(&deferred, custom_properties.as_ref())
                }
                value => Some(value),
            }?;
            Some((k, resolve_against_inherited(value, inherited, &ctx)))
        })
        .collect();

    // Settle the retained shorthand and corner winners before exposing a map
    // that no longer carries declaration order. Invalid winning values reset
    // their corners instead of revealing an older declaration.
    if !radius_precedence.is_empty() {
        let zero = crate::property::CornerRadius::circular(Length::Px(0.0));
        let shorthand = match resolved
            .get(&PropertyKey::BorderRadius)
            .map(ResolvedAgainstInherited::as_property_value)
        {
            Some(PropertyValue::BorderRadius(radius)) => *radius,
            _ => crate::property::BorderRadius::elliptical(
                [Length::Px(0.0); 4],
                [Length::Px(0.0); 4],
            ),
        };
        let mut corners = [
            shorthand.top_left,
            shorthand.top_right,
            shorthand.bottom_right,
            shorthand.bottom_left,
        ];
        let values: [fn(crate::property::CornerRadius<Length>) -> PropertyValue; 4] = [
            PropertyValue::BorderRadiusTopLeft,
            PropertyValue::BorderRadiusTopRight,
            PropertyValue::BorderRadiusBottomRight,
            PropertyValue::BorderRadiusBottomLeft,
        ];
        for (index, key) in radius_keys.into_iter().enumerate() {
            let longhand_wins = radius_precedence.get(&key).is_some_and(|rank| {
                radius_precedence
                    .get(&PropertyKey::BorderRadius)
                    .is_none_or(|shorthand| rank > shorthand)
            });
            if longhand_wins {
                corners[index] = match resolved
                    .get(&key)
                    .map(ResolvedAgainstInherited::as_property_value)
                {
                    Some(
                        PropertyValue::BorderRadiusTopLeft(corner)
                        | PropertyValue::BorderRadiusTopRight(corner)
                        | PropertyValue::BorderRadiusBottomRight(corner)
                        | PropertyValue::BorderRadiusBottomLeft(corner),
                    ) => *corner,
                    _ => zero,
                };
            }
            if radius_precedence.contains_key(&key) {
                resolved.insert(
                    key,
                    resolve_against_inherited(values[index](corners[index]), inherited, &ctx),
                );
            }
        }
        let [top_left, top_right, bottom_right, bottom_left] = corners;
        resolved.insert(
            PropertyKey::BorderRadius,
            resolve_against_inherited(
                PropertyValue::BorderRadius(crate::property::BorderRadius {
                    top_left,
                    top_right,
                    bottom_right,
                    bottom_left,
                }),
                inherited,
                &ctx,
            ),
        );
    }

    // Page winner iteration has no ordering guarantee. Settle its color first,
    // then use that same color for every contextual background expression.
    let own_color = match resolved
        .get(&PropertyKey::Color)
        .map(ResolvedAgainstInherited::as_property_value)
    {
        Some(PropertyValue::Color(color)) => *color,
        Some(PropertyValue::ContextualColor(color)) => {
            crate::property::resolve_contextual_color(&color.source, inherited.color)
                .unwrap_or(inherited.color)
        }
        _ => inherited.color,
    };
    let background_color_expression = match resolved
        .get(&PropertyKey::BackgroundColor)
        .map(ResolvedAgainstInherited::as_property_value)
    {
        Some(PropertyValue::ContextualColor(expression)) => Some(expression.source.clone()),
        _ => None,
    };
    for (key, value) in &mut resolved {
        if let PropertyValue::ContextualColor(color) = value.as_property_value()
            && let Some(color) = crate::property::resolve_contextual_color(
                &color.source,
                if *key == PropertyKey::Color {
                    inherited.color
                } else {
                    own_color
                },
            )
        {
            let value_resolved = if *key == PropertyKey::Color {
                PropertyValue::Color(color)
            } else {
                PropertyValue::BackgroundColor(color)
            };
            *value = resolve_against_inherited(value_resolved, inherited, &ctx);
        }
    }

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
    let margin_box_inheritance = MarginBoxInheritance {
        background_color_expression,
        color: match resolved
            .get(&PropertyKey::Color)
            .map(ResolvedAgainstInherited::as_property_value)
        {
            Some(PropertyValue::Color(color)) => *color,
            _ => inherited.color,
        },
        background_color: match resolved
            .get(&PropertyKey::BackgroundColor)
            .map(ResolvedAgainstInherited::as_property_value)
        {
            Some(PropertyValue::BackgroundColor(color)) => *color,
            _ => CssColor::TRANSPARENT,
        },
        font_size,
        custom_properties,
    };
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
        margin_box_inheritance,
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
/// multiplier ([`crate::resolve::ComputedLineHeight`] doc, "children inherit the number and
/// multiply it by their own font-size"), not a page-context-specific
/// carve-out — pinned by
/// `cascade_page_padding_lh_uses_page_context_own_font_size_for_inherited_number`.
///
/// The result is then converted to an absolute length via
/// [`used_line_height_length`], returning `None` when unresolvable (`normal`
/// with no font metrics — the same wall as `cap`/`rcap`).
pub(super) fn page_context_line_height_basis(
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

/// The page context's raw (pre-[`resolve_overflow`](crate::property::resolve_overflow)) `overflow-x` +
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
pub(super) fn page_context_overflow_pair(
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
