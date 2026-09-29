//! The hand-written [`PropertyValue`] and [`PropertyKey`] enums and the
//! [`PropertyValue::key`] projection between them.

use std::sync::Arc;

use raikiri_style_macros::longhands;
use smol_str::SmolStr;

// Not `use super::*`: that glob would also import this file's own module name
// `decl`, which would then be ambiguous with the macro-expanded inline `decl`.
#[allow(unused_imports)]
use super::parse::*;
use super::types::*;

// `#[longhands]` can only read an inline module, hence the nested `decl`.
#[longhands]
#[allow(clippy::module_inception)]
mod decl {
    use super::*;

    /// The resolved values of currently supported properties (variants are listed below;
    /// see `parse_value` for the property-name-to-variant mapping).
    ///
    /// The parser silently drops unrecognized properties (for example, `cursor`, which
    /// is outside the current scope) and invalid values (for example, `font-size: math`,
    /// whose MathML scaling algorithm is not implemented) from rules by returning `None`.
    ///
    /// This section does not list examples of unsupported units: such examples become
    /// stale when support for those units is added (this happened once with `cm`).
    /// The canonical source is the `Token::Dimension` match arm in
    /// [`parse_length_value`] (the comment immediately before its `_` arm), together
    /// with the `parse_length_value_rejects_unsupported_unit` test.
    ///
    /// # CSS-wide keyword (canonical)
    ///
    /// Support for CSS-wide keywords (`inherit` / `initial` / `unset` / `revert` — CSS
    /// Cascade 4 §7.3 "Explicit Defaulting"
    /// <https://www.w3.org/TR/css-cascade-4/#defaulting-keywords>; `revert-layer`
    /// — CSS Cascade 5 §7.3.5 "Rolling Back Cascade Layers: the revert-layer
    /// keyword" <https://www.w3.org/TR/css-cascade-5/#revert-layer>) is not yet
    /// implemented in this crate, except for the border longhands and the `border` /
    /// `border-right` shorthands, which accept all five keywords through
    /// [`CssWideKeyword`] (see that type's docs for resolution).
    ///
    /// Unlike unsupported units, this missing support has no single code arm: each
    /// `parse_*` function simply does not recognize these idents and rejects them
    /// through the same "unknown keyword" path as other spec-invalid keywords
    /// (for example, its own `_ => None` arm). This is a **coincidence caused by an
    /// absence of implementation**. Neither this section nor individual property
    /// docs repeat the list of five keywords: repeated lists drift when properties
    /// are added. Indeed, `property.rs` once repeated the list at 16 separate sites,
    /// and three of those (`border-width`, `border-style`, and `box-sizing`) had no
    /// pinning tests. The canonical sources are this paragraph and the representative
    /// tests named `rejects_css_wide_keyword` (grep the crate to find all of them,
    /// including `text_align_rejects_css_wide_keyword`).
    ///
    /// Grammars based on `<custom-ident>` (names in `counter-name` / `string-set` and
    /// arguments to `position: running()`) instead have **explicit** reject lists
    /// ([`is_reserved_counter_name`] / [`is_reserved_custom_ident`]). These enforce
    /// permanent exclusions specified in CSS Values 4 §4.2
    /// <https://www.w3.org/TR/css-values-4/#custom-idents>, regardless of whether
    /// CSS-wide keywords are implemented. Do not confuse them with the accidental
    /// matching behavior described above.
    ///
    /// **Box properties are not "unrecognized"**: `margin` / `padding` / `border-*` /
    /// `width` / `height` are all recognized and have variants below.
    /// `font-size: 1em` / `font-size: medium` / `font-size: larger` are also valid.
    /// **When changing the examples, keep the sibling [`crate::rule`] test
    /// `drops_invalid_property_and_value` in sync.** Both describe the same behavior;
    /// the test once changed while this documentation was left stale.
    ///
    /// # `#[non_exhaustive]` semantics (verbatim for fulgur / downstream consumers)
    ///
    /// Enum-level `#[non_exhaustive]` makes downstream `match` statements
    /// forward-compatible (adding a variant requires an `_ =>` arm), but **does not
    /// block tuple constructors of existing variants**. Changing a variant's payload
    /// **type** therefore still breaks callers that construct it.
    ///
    /// A cascade-memory DoS mitigation caused precisely this break. Downstream
    /// fulgur consumers need to migrate the following:
    ///
    /// - [`Content`](PropertyValue::Content): `Content(Vec<ContentComponent>)` →
    ///   `Content(Arc<Vec<ContentComponent>>)`
    /// - [`StringSet`](PropertyValue::StringSet): wrap the outer payload `Vec<..>` in `Arc<Vec<..>>`
    ///
    /// The same cascade-memory DoS mitigation was extended to counter-*. There was
    /// no live consumer impact at the time, but the change follows the same pattern:
    ///
    /// - [`CounterReset`](PropertyValue::CounterReset) / [`CounterIncrement`](PropertyValue::CounterIncrement) /
    ///   [`CounterSet`](PropertyValue::CounterSet): `Vec<(SmolStr, i32)>` → `Arc<Vec<(SmolStr, i32)>>`
    ///
    /// A later Arc-wrap follows the pattern for the last non-Arc `Vec` payload. Its
    /// goal is better performance, not security:
    ///
    /// - [`FontFamily`](PropertyValue::FontFamily): `FontFamily(Vec<FontFamilyName>)` →
    ///   `FontFamily(Arc<Vec<FontFamilyName>>)`
    ///
    /// Consumers that **read** payloads through pattern matching continue to work
    /// **unchanged**: `Arc<Vec<T>>` dereferences through `Deref<Target = Vec<T>>` →
    /// `Deref<Target = [T]>`, so a `match` arm such as
    /// `PropertyValue::Content(components) => components.iter()` still works
    /// (`&Arc<Vec<T>>` autoderefs to `&[T]`). Consumers that **construct** payloads,
    /// however, must change `PropertyValue::Content(vec![...])` to
    /// `PropertyValue::Content(Arc::new(vec![...]))`.
    #[non_exhaustive]
    #[derive(Clone, Debug, PartialEq)]
    pub enum PropertyValue {
        /// A `--<ident>` custom property. The value remains token-preserving until
        /// the computed-value stage, where `var()` references are resolved.
        #[key(Custom)]
        CustomProperty(CustomProperty),
        /// A known property value containing `var()` or a math function.
        #[key(with = |value| value.key)]
        Deferred(DeferredValue),
        /// `color: <color>` — inherited, initial: black.
        Color(CssColor),
        /// `background-color: <color>` — **non-inherited**, initial: `transparent`.
        /// CSS Backgrounds 3 §2.2 "Base Color: the background-color property"
        /// <https://www.w3.org/TR/css-backgrounds-3/#background-color>.
        BackgroundColor(CssColor),
        /// `font-family: <family-name>#` — inherited. According to CSS Fonts 4 §2.1
        /// <https://www.w3.org/TR/css-fonts-4/#font-family-prop>, the spec's initial
        /// value "depends on user agent". This implementation uses
        /// `[FontFamilyName::generic("serif")]` (see [`crate::property::initial_font_family`]).
        ///
        /// [`Arc<Vec<..>>`] wraps the payload, following the pattern of related DoS
        /// mitigations. Moving a cascade winner (`apply_value`) and cloning during an
        /// inheritance walk (`parent.font_family.clone()` in
        /// `SpecifiedValues::inherit_from`) become **shallow (Arc reference-count increments)**.
        /// Unlike non-inherited counter-* / content / string-set, `font-family` is
        /// inherited. The cost is not "reset to the initial value at every node", but
        /// "carry the parent's value at every node during the inheritance walk". This
        /// previously required O(N) one-element `Vec` allocations for an N-node document
        /// (an existing performance problem). `Arc<Vec<T>>: Deref<Target = Vec<T>>` keeps
        /// existing downstream `.iter()` / `.len()` / `.is_empty()` calls working without
        /// changes (no impact on dom/paint consumers, including `cv.font_family.iter()`
        /// in `crates/raikiri-dom/src/layout.rs`).
        FontFamily(Arc<Vec<FontFamilyName>>),
        /// `font-size: <absolute-size> | <length-percentage [0,∞]>` — inherited,
        /// initial: 16px (= `medium`). CSS Fonts 4 §2.5
        /// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>.
        ///
        /// `<absolute-size>` (`xx-small` … `xxx-large`, `medium`) does not depend on the
        /// parent. [`parse_font_size`] fully resolves these fixed sizes to `Length::Px`
        /// relative to `medium` (16px) at parse time, using the scaling-factor table in
        /// §2.5.1 (<https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping>).
        /// `<relative-size>` (`larger` / `smaller`) depends on the inherited size, so it
        /// has a separate variant ([`Self::FontSizeRelative`]); see that variant's docs
        /// for the reason. The `math` keyword is spec-valid but not implemented (the
        /// entire MathML scaling algorithm is unsupported), so `parse_font_size` returns
        /// `None` for it.
        FontSize(Length),
        /// `font-size: <relative-size>` (`larger` / `smaller`) — inherited,
        /// part of the same `font-size` property as [`Self::FontSize`]. CSS Fonts 4 §2.5
        /// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>.
        ///
        /// # Why a new variant instead of reusing `Self::FontSize(Length)`
        ///
        /// Like `bolder` / `lighter` (`font-weight`), this requires a parent-dependent
        /// read-modify-write operation. A symmetric design would change the `FontSize`
        /// payload into a keyword-bearing enum, as with [`FontWeightValue`]. But the
        /// `raikiri` (umbrella) crate re-exports `PropertyValue`, and
        /// `umbrella_re_exports_cover_computed_value_types_and_parse_options_fields`
        /// in `crates/raikiri/tests/build_cascaded.rs` **intentionally pins** the
        /// construction of `PropertyValue::FontSize(Length::Px(12.0))` at compile time
        /// (see the rationale for the umbrella re-export list in the relevant comment
        /// in `crates/raikiri/src/lib.rs`). Changing the `FontSize` payload type would
        /// break that check and require a change in `crates/raikiri`: a breaking change
        /// for the umbrella crate (as with the `Content`/`StringSet` payload changes).
        ///
        /// Because `PropertyValue` is `#[non_exhaustive]`, **adding a new variant** does
        /// not break existing tuple constructors (see the enum-level `#[non_exhaustive]`
        /// documentation). We therefore keep the `FontSize` payload type and add this
        /// variant for `larger` / `smaller`. Unlike [`FontWeightValue`],
        /// [`RelativeFontSize`] is **not** in the `raikiri` (umbrella) `pub use` list.
        /// This follows the existing asymmetry for [`FontWeightValue`], which is also
        /// absent from that list: only consumers depending directly on raikiri-style
        /// can name it.
        ///
        /// # Resolution timing
        ///
        /// As with `bolder` / `lighter`, [`crate::cascade::apply_value`] resolves the
        /// relative size against the parent's computed font-size (the previous value of
        /// `SpecifiedValues::font_size` during staging, always
        /// `Length::Px(the parent's px value)` by the D5 invariant). It stores the result
        /// in `target.font_size` as [`Self::FontSize`] (`Length::Px`). This variant is only
        /// a temporary representation of a cascade winner and does not survive into
        /// [`crate::specified::SpecifiedValues`] or later stages. For pages,
        /// [`crate::cascade::resolve_against_inherited`] performs the same resolution;
        /// by the time a value reaches [`crate::page::PageCascadeResult::declarations`],
        /// it has likewise become [`Self::FontSize`] (`Length::Px`).
        // `larger` / `smaller` belong to the same property as `font-size`.
        // Map them to the same `PropertyKey` so cascade winner selection
        // correctly makes `font-size: 12px` and `font-size: larger` compete.
        // Separate keys would let both win, which the spec does not allow.
        #[key(FontSize)]
        FontSizeRelative(RelativeFontSize),
        /// `font-weight: <font-weight-absolute> | bolder | lighter` — inherited,
        /// initial: `Absolute(400.0)`. CSS Fonts 4 §2.2
        /// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop>.
        ///
        /// The payload is a **specified value** ([`FontWeightValue`]). `bolder` and
        /// `lighter` are relative weights that depend on the inherited value, so they
        /// cannot be resolved while parsing. [`crate::cascade::apply_value`] resolves
        /// them against the parent's computed weight and stores an absolute value in
        /// [`crate::computed::ComputedValues::font_weight`] (`f32`).
        ///
        /// **Page contexts also resolve them.** [`crate::page::cascade_page`] passes the
        /// winner through [`crate::cascade::resolve_against_inherited`] (a sibling of
        /// `apply_value` sharing the same relative-weight table), then stores it in
        /// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations).
        /// Thus `@page { font-weight: bolder }` appears in the public result only as
        /// `Absolute`. According to CSS Paged Media 3 §6 "Page Properties"
        /// <https://www.w3.org/TR/css-page-3/#page-properties>, "The page context
        /// inherits from the root element": the inherited value comes from the root
        /// element's computed weight. If unavailable, it falls back to initial 400 under
        /// the legacy exception in the same section.
        FontWeight(FontWeightValue),
        /// `line-height: normal | <number> | <length-percentage>` — inherited,
        /// initial: [`LineHeight::Normal`]. CSS Inline 3 §5.1
        /// <https://www.w3.org/TR/css-inline-3/#line-height-property>.
        /// The number-versus-length distinction is essential information for downstream
        /// paint to resolve in context: unitless numbers have special specified-value
        /// inheritance behavior (see the [`LineHeight`] documentation).
        LineHeight(LineHeight),
        /// `display: <ident>` — non-inherited, initial: `inline` (CSS Display
        /// 3 §2 <https://www.w3.org/TR/css-display-3/#propdef-display>).
        /// Currently accepted keywords: `block` / `inline` / `inline-block` / `none`
        /// (see the [`DisplayValue`] documentation).
        Display(DisplayValue),
        /// `list-style-type: none | <counter-style-name> | <string>` — inherited,
        /// initial: `disc` (CSS Lists 3 §3.1).
        ListStyleType(ListStyleType),
        /// `list-style-position: inside | outside` — inherited, initial: `outside`
        /// (CSS Lists 3 §3.2).
        ListStylePosition(ListStylePosition),
        /// `list-style-image: none | <url>` — inherited, initial: `none`.
        ListStyleImage(BackgroundImage),
        /// `counter-reset: [ <counter-name> <integer>? ]+ | none` —
        /// non-inherited. The spec's initial value is `none` (CSS Lists 3 §4.1), which
        /// this implementation represents as an empty list.
        /// The integer defaults to 0 if omitted (as specified).
        ///
        /// The [`Arc<Vec<..>>`] wrapper makes cloning a cascade winner (the
        /// `value.clone()` in `apply_winners`'s drain) and cloning during an inheritance
        /// walk (`stack.push((child, computed.clone()))` and
        /// `out[idx] = computed.clone()` in `resolve_inheritance`) **shallow (only an
        /// Arc reference-count increment)**. Because counter-* is non-inherited,
        /// `inherit_from` puts children in the shared empty slot, but the path to the
        /// winner (parent stack entry and accumulated cascade candidates) previously
        /// deep-cloned the payload. For `* { counter-reset: c0 c1 ... cN }` across M
        /// elements, the cost falls from O(N × M) to O(N + M) (a cascade-memory DoS
        /// mitigation following the Content/StringSet pattern).
        CounterReset(Arc<Vec<(SmolStr, i32)>>),
        /// CSS-wide `counter-reset: inherit` retained for page-context resolution.
        /// The page-margin used-value pass resolves this marker against the
        /// enclosing page counter scope.
        #[key(CounterReset)]
        CounterResetInherit,
        /// `counter-increment: [ <counter-name> <integer>? ]+ | none` —
        /// non-inherited. The spec's initial value is `none` (CSS Lists 3 §4.2), which
        /// this implementation represents as an empty list.
        /// The integer defaults to 1 if omitted (as specified).
        ///
        /// The [`Arc<Vec<..>>`] wrapper has the same rationale as [`Self::CounterReset`].
        CounterIncrement(Arc<Vec<(SmolStr, i32)>>),
        /// `counter-set: [ <counter-name> <integer>? ]+ | none` —
        /// non-inherited. The spec's initial value is `none` (CSS Lists 3 §4.2), which
        /// this implementation represents as an empty list.
        /// The integer defaults to 0 if omitted (as specified).
        ///
        /// The [`Arc<Vec<..>>`] wrapper has the same rationale as [`Self::CounterReset`].
        CounterSet(Arc<Vec<(SmolStr, i32)>>),
        /// `content: normal | none | <content-list>` — non-inherited. The spec's
        /// initial value is `normal`, represented here by an empty list; `none` is
        /// represented by the [`ContentComponent::None`] sentinel.
        /// (A downstream layer decides whether to generate pseudo-elements.) CSS Content 3 §1
        /// <https://www.w3.org/TR/css-content-3/#content-property>.
        ///
        /// The [`Arc<Vec<..>>`] wrapper makes cloning a cascade winner, cloning an
        /// inheritance-walk stack entry, and writing each node **shallow (only an Arc
        /// reference-count increment)**. Sharing one heap slot prevents the O(N × M)
        /// memory blow-up from `* { content: "<large>" }` across N elements (a
        /// cascade-memory DoS mitigation).
        ///
        /// **For consumers:**
        /// Consumers that **read** payloads through pattern matching can continue to
        /// use `PropertyValue::Content(components) =>
        /// components.iter().for_each(..)` unchanged, thanks to the `Arc<Vec<T>>` deref
        /// chain (Vec → slice). Only consumers that **construct** payloads must change
        /// to `PropertyValue::Content(Arc::new(vec![..]))`.
        /// See also the enum-level `#[non_exhaustive]` semantics documentation.
        Content(Arc<Vec<ContentComponent>>),
        /// `string-set: none | [ <custom-ident> <content-list> ]#` — non-inherited.
        /// The spec's initial value is `none`, represented here by an empty list. Each
        /// entry is a `(name, content-list)` pair.
        /// CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>;
        /// `<content-list>` follows CSS Content 3 §2 (parser implemented).
        /// Name resolution and runtime `string()` references belong to the downstream
        /// layer (raikiri-dom).
        ///
        /// The [`Arc<Vec<..>>`] wrapper has the same rationale as [`Self::Content`]: it
        /// prevents the same kind of DoS along the `* { string-set: name "<large>" }`
        /// path across N elements.
        ///
        /// **For consumers:**
        /// As with [`Self::Content`], the outer `Arc` is transparent to readers thanks
        /// to dereferencing. Only consumers that construct values
        /// (`PropertyValue::StringSet(vec![(name, items)])`) must switch to
        /// `PropertyValue::StringSet(Arc::new(vec![..]))`. The inner
        /// `Vec<ContentComponent>` is not Arc-wrapped: per-entry sharing has no
        /// predictable benefit, and wrapping only the outer vector blocks the attack.
        StringSet(Arc<Vec<(SmolStr, Vec<ContentComponent>)>>),
        /// `position: static | sticky | running(<custom-ident>)` — non-inherited,
        /// initial: `static`.
        /// CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax> and
        /// CSS Positioned Layout Module Level 3 §3
        /// <https://www.w3.org/TR/css-position-3/#sticky-pos>.
        /// In the current scope, only `running()` seed emission reaches downstream.
        /// `Static` / `Sticky` are no-ops in `apply_value`: their discriminants suppress
        /// a preceding `running()` (matching the spec default). `Sticky` is also kept
        /// for future layout integration.
        /// `relative` / `absolute` / `fixed` are not yet implemented; parsing drops them.
        Position(PositionValue),
        /// `top: auto | <length-percentage>` — **non-inherited**, initial: `auto`
        /// (CSS Positioned Layout Module Level 3 §3 <https://www.w3.org/TR/css-position-3/>).
        /// Used for `position: relative` offset (paint-time shift) and future absolute/fixed.
        Top(LengthOrAuto),
        /// `right: auto | <length-percentage>` — **non-inherited**, initial: `auto`.
        Right(LengthOrAuto),
        /// `bottom: auto | <length-percentage>` — **non-inherited**, initial: `auto`.
        Bottom(LengthOrAuto),
        /// `left: auto | <length-percentage>` — **non-inherited**, initial: `auto`.
        Left(LengthOrAuto),
        /// `text-align: start | end | left | right | center | justify | match-parent
        /// | justify-all` — **inherited**, initial: [`TextAlign::Start`]
        /// (CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
        /// <https://www.w3.org/TR/css-text-3/#text-align-property>).
        /// The spec defines a shorthand (text-align-all + text-align-last), but this
        /// implementation stores it in a single field (**part (b) unsupported**;
        /// splitting into longhands is deferred to a later task). See [`TextAlign`] docs.
        TextAlign(TextAlign),
        /// `hanging-punctuation: none | first` — inherited, initial `none`.
        /// The line-layout consumer currently implements only a leading U+3000
        /// hang for `first`; other valid grammar arms remain outside this slice.
        HangingPunctuation(HangingPunctuation),
        /// `text-indent` — the full grammar is
        /// `<length-percentage> && hanging? && each-line?`; this variant covers
        /// **only** the `<length-percentage>` component (`hanging` / `each-line`
        /// are the other two, unimplemented — see "Scope carving" below).
        /// **inherited**, initial: `0` (CSS Text 3 §8.1 "First Line Indentation:
        /// the text-indent property"
        /// <https://www.w3.org/TR/css-text-3/#text-indent-property>: "Initial:
        /// 0", "Applies to: block containers", "Inherited: yes", "Percentages:
        /// refers to block container's own inline-axis inner size", "Computed
        /// value: computed `<length-percentage>` value, plus any specified
        /// keywords" — the "plus any specified keywords" clause covers
        /// `hanging`/`each-line`, which this variant's bare [`Length`] payload
        /// does not carry at all, per "Scope carving" below).
        ///
        /// Unlike [`Self::PaddingTop`] / [`Self::Width`], the grammar carries no
        /// `[0,∞]` restriction — negative indents are spec-valid.
        /// `parse_text_indent` therefore applies no non-negative filter, the
        /// same shape as [`Self::MarginTop`]'s `<length-percentage> | auto`
        /// (minus the `auto` alternative, which `text-indent` does not have).
        ///
        /// The payload is [`TextIndentValue`] (length plus hanging/each-line flags).
        /// The cascade distributes the length to `text_indent` and the flags to
        /// `text_indent_hanging` / `text_indent_each_line`. The consumer
        /// (raikiri-dom realign) maps them to parley's `IndentOptions`.
        TextIndent(TextIndentValue),
        /// `padding-top: <length-percentage [0,∞]>` — non-inherited, initial: `0`.
        /// CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical>.
        /// `parse_padding_side` enforces the spec grammar's non-negative constraint
        /// `<length-percentage [0,∞]>` at parse time (negative values return `None`,
        /// dropping the declaration). Because the grammar does not include `auto`, the
        /// Dimension / Percentage arms of `parse_length_value` reject it naturally
        /// through fall-through.
        PaddingTop(Length),
        /// `padding-right: <length-percentage [0,∞]>` — same grammar as [`Self::PaddingTop`].
        PaddingRight(Length),
        /// `padding-bottom: <length-percentage [0,∞]>` — same grammar as [`Self::PaddingTop`].
        PaddingBottom(Length),
        /// `padding-left: <length-percentage [0,∞]>` — same grammar as [`Self::PaddingTop`].
        PaddingLeft(Length),
        /// `padding: <'padding-top'>{1,4}` shorthand — non-inherited, initial:
        /// `Sides::all(Length::Px(0.0))`. CSS Box 3 §4.2
        /// <https://www.w3.org/TR/css-box-3/#padding-shorthand>.
        ///
        /// Expand one to four values according to CSS Box 3 §4.2 (this is a paraphrase,
        /// not a verbatim quotation, so do not label it `verbatim`):
        /// - 1 value: all 4 sides
        /// - 2 values: top/bottom = first, left/right = second
        /// - 3 values: top = first, left/right = second, bottom = third
        /// - 4 values: top / right / bottom / left (clockwise from top)
        ///
        /// **This variant is never observed by the element cascade**:
        /// [`crate::rule::expand_shorthand_into`] expands it into four longhand variants
        /// ([`PaddingTop`](Self::PaddingTop) / [`PaddingRight`](Self::PaddingRight) /
        /// [`PaddingBottom`](Self::PaddingBottom) / [`PaddingLeft`](Self::PaddingLeft))
        /// both at the parser exit (`parse_declaration_block`) and at the element-cascade
        /// entry (the `collect_cascaded` function in [`mod@crate::cascade`]). This gives
        /// the 1/2/3/4 expansion and follows CSS Cascading L4 §3 "Shorthand Properties"
        /// <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim: "A shorthand
        /// property sets all of its longhand sub-properties, exactly as if expanded in
        /// place." Each side can then win the cascade independently. These expansion
        /// guarantees explain why the variant is unreachable. If it nevertheless reaches
        /// [`crate::cascade::apply_value`], that behavior is **not a safety net**
        /// (correcting an earlier characterization): it unconditionally overwrites all
        /// four sides of `ComputedValues.padding` and necessarily destroys four winning
        /// longhands. Reaching it is already a bug and does not degrade gracefully (see
        /// the canonical documentation for [`crate::cascade::apply_value`] and
        /// [`crate::rule::expand_shorthand_into`]).
        /// Consider migrating to the margin-style parse-time expansion model (follow-up task).
        Padding(Sides<Length>),
        /// `padding-inline: <'padding-top'>{1,2}` shorthand — CSS Logical
        /// Properties and Values 1 §4.4 "Flow-Relative Padding: the
        /// padding-block-start, padding-block-end, padding-inline-start,
        /// padding-inline-end properties and padding-block and padding-inline
        /// shorthands" <https://www.w3.org/TR/css-logical-1/#propdef-padding-inline>.
        /// With two values, the first is `padding-inline-start` and the second is
        /// `padding-inline-end` (if the second is omitted, copy the first via
        /// [`StartEnd::both`]).
        ///
        /// # Physical mapping (Non-goal: flow-relative mapping based on writing-mode / direction)
        ///
        /// The spec determines which physical side (`padding-top`/`padding-right`/
        /// `padding-bottom`/`padding-left`) corresponds to
        /// `padding-inline-start`/`padding-inline-end` (and
        /// `padding-block-start`/`padding-block-end`) from the element's **own computed
        /// `writing-mode` / `direction` / `text-orientation`** (CSS Logical Properties
        /// and Values 1 §4, opening section). Raikiri cuts off this dependency in two
        /// ways and always maps to a fixed physical side. Note that these two cases
        /// are **asymmetric**:
        ///
        /// - **Block axis** (via [`PaddingBlock`](Self::PaddingBlock),
        ///   `padding-block-start`/`-end` → `padding-top`/`padding-bottom`):
        ///   Raikiri does not implement a vertical writing rendering pipeline, so the
        ///   renderer-facing writing-mode fallback is always [`WritingMode::HorizontalTb`]
        ///   (see the Non-goal section in [`resolve_writing_mode`]). Under
        ///   `horizontal-tb`, the block axis is always vertical (block-start = top),
        ///   and `direction` never affects its mapping (the spec likewise always maps
        ///   block-start to top under `horizontal-tb` for any `direction`). This case is
        ///   therefore **exact, not an approximation**: within Raikiri's scope (the
        ///   renderer-facing writing mode is always `horizontal-tb`), it fully matches
        ///   the spec.
        /// - **Inline axis** (this variant / `padding-inline-start`/`-end` →
        ///   `padding-left`/`padding-right`): this also **assumes `direction: ltr`**.
        ///   The `direction` property itself is implemented in this crate
        ///   (see [`PropertyValue::Direction`]), but the mapping **does not consult it**.
        ///   As the section below, "Why the eight longhands have no dedicated variant",
        ///   explains, this PR chooses a fixed mapping at parse time, before the cascade
        ///   winner is known. Once the winner is known, the computed `direction` is
        ///   available in this crate: [`resolve_text_align_match_parent`] already
        ///   performs similar post-cascade resolution from `SpecifiedValues::finalize`.
        ///   Direction-aware resolution is intentionally deferred beyond this scope;
        ///   it is not inherently impossible in this architecture. **This is an
        ///   approximation that disagrees with the spec for elements with
        ///   `direction: rtl`**: `padding-inline-start` should then map to
        ///   `padding-right`, but Raikiri always maps it to `padding-left`.
        ///
        /// This asymmetry (exact block axis, approximate inline axis) is one level more
        /// complex than the physical x/y mapping for `overflow-inline`/`overflow-block`
        /// (the two components of the `overflow` shorthand) explained in the Non-goal
        /// section of [`OverflowValue`]. Each overflow inline/block axis has a single
        /// value (the `overflow-x`/`overflow-y` pair), with no `start`/`end` distinction,
        /// so `direction` did not matter there. This property has two sides, `start` and
        /// `end`, on each axis, making an additional direction-dependent approximation
        /// necessary on the inline axis alone.
        ///
        /// # Why the 8 longhands have no dedicated `PropertyValue` variants
        ///
        /// These are `padding-inline-start`/`-end`, `padding-block-start`/`-end`,
        /// and the corresponding four margin longhands.
        ///
        /// The fixed mapping above does not depend on any cascade-time state (such as
        /// an inherited computed `direction` or a `direction` winner on the same node):
        /// it is already settled at parse time. Consequently,
        /// `padding-inline-start: <value>` produces exactly the same `PropertyValue`
        /// as [`Self::PaddingLeft`], and `padding-block-start: <value>` produces exactly
        /// the same `PropertyValue` as [`Self::PaddingTop`] (see the relevant arms of
        /// `parse_value` and `property_key_for_name`).
        /// Following the existing `word-wrap`/`overflow-wrap` legacy alias
        /// pattern (folding names with the same grammar into the same
        /// `PropertyValue`/[`PropertyKey`]; see the `"overflow-wrap" | "word-wrap"`
        /// arm of `parse_value`), no separate variant is needed. Cascade winner
        /// selection likewise treats them as competing for the same physical
        /// property, matching the spec's actual cascade behavior (CSS Logical
        /// Properties and Values 1 §4: "corresponding flow-relative and physical
        /// properties are paired").
        ///
        /// # This variant is not observed during element cascade
        ///
        /// As with [`Self::Padding`] / [`Self::Margin`],
        /// [`crate::rule::expand_shorthand_into`] expands it into the two
        /// [`Self::PaddingLeft`]/[`Self::PaddingRight`] longhands both when parsing
        /// exits and when element cascade begins. If it does reach
        /// [`crate::cascade::apply_value`], that behavior is **not a safety net**
        /// (the same framing as the `Padding`/`Margin` arms).
        PaddingInline(StartEnd<Length>),
        /// `padding-block: <'padding-top'>{1,2}` shorthand — the same grammar,
        /// expansion, and physical-mapping rationale as [`Self::PaddingInline`]
        /// (the [`Self::PaddingInline`] docs are canonical), but for the block axis
        /// (`padding-block-start`/`-end` → `padding-top`/`padding-bottom`: exact
        /// mapping, not an approximation). CSS Logical Properties and Values 1 §4.4
        /// <https://www.w3.org/TR/css-logical-1/#propdef-padding-block>.
        PaddingBlock(StartEnd<Length>),
        /// `margin-top: <length-percentage> | auto` — non-inherited, initial: 0
        /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>).
        MarginTop(LengthOrAuto),
        /// Page-context-only marker for `margin-top: inherit`. The page parser
        /// resolves this against the root element before exposing declarations.
        #[key(MarginTop)]
        MarginTopInherit,
        /// `margin-right: <length-percentage> | auto` — non-inherited, initial: 0
        /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>).
        MarginRight(LengthOrAuto),
        /// Page-context-only marker for `margin-right: inherit`.
        #[key(MarginRight)]
        MarginRightInherit,
        /// `margin-bottom: <length-percentage> | auto` — non-inherited, initial: 0
        /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>).
        MarginBottom(LengthOrAuto),
        /// Page-context-only marker for `margin-bottom: inherit`.
        #[key(MarginBottom)]
        MarginBottomInherit,
        /// `margin-left: <length-percentage> | auto` — non-inherited, initial: 0
        /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>).
        MarginLeft(LengthOrAuto),
        /// Page-context-only marker for `margin-left: inherit`.
        #[key(MarginLeft)]
        MarginLeftInherit,
        /// Page-context-only marker for the `margin: inherit` shorthand. It is
        /// expanded into four side markers before page-context cascade.
        #[key(Margin)]
        MarginInherit,
        /// `margin: <'margin-top'>{1,4}` shorthand — sets all four sides
        /// (CSS Box 3 §3.2 <https://www.w3.org/TR/css-box-3/#margin-shorthand>).
        ///
        /// **This variant is not observed during element cascade**:
        /// [`crate::rule::expand_shorthand_into`] expands it into the four longhand
        /// variants ([`MarginTop`](Self::MarginTop) / [`MarginRight`](Self::MarginRight) /
        /// [`MarginBottom`](Self::MarginBottom) / [`MarginLeft`](Self::MarginLeft))
        /// both when parsing exits (`parse_declaration_block`) and when element
        /// cascade begins ([`mod@crate::cascade`]'s `collect_cascaded`). This follows
        /// the 1/2/3/4-value expansion in spec §3.2 and CSS Cascading L4 §3,
        /// "Shorthand Properties" <https://www.w3.org/TR/css-cascade-4/#shorthand>:
        /// "A shorthand property sets all of its longhand sub-properties,
        /// exactly as if expanded in place." Per-side cascade winners therefore
        /// work naturally. The expansion guarantee prevents this variant from
        /// reaching cascade. If it does reach [`crate::cascade::apply_value`], the
        /// behavior is **not a safety net** (corrected framing): it unconditionally
        /// overwrites all four sides of `ComputedValues.margin`, destroying the
        /// four longhand winners. Reaching it is already a bug; it cannot degrade
        /// gracefully. See the canonical descriptions in the docs for
        /// [`crate::cascade::apply_value`] and [`crate::rule::expand_shorthand_into`].
        Margin(Sides<LengthOrAuto>),
        /// `margin-inline: <'margin-top'>{1,2}` shorthand — CSS Logical
        /// Properties and Values 1 §4.2 "Flow-Relative Margins: the
        /// margin-block-start, margin-block-end, margin-inline-start,
        /// margin-inline-end properties and margin-block and margin-inline
        /// shorthands" <https://www.w3.org/TR/css-logical-1/#propdef-margin-inline>.
        /// The [`Self::PaddingInline`] docs provide the canonical grammar,
        /// expansion, and physical-mapping rationale. The only difference from
        /// padding is that the payload is `<length-percentage> | auto`, as with
        /// [`Self::Margin`] (`auto` passes through [`parse_margin_side`]). The
        /// inline-axis mapping (this variant and `margin-inline-start`/`-end` →
        /// `margin-left`/`margin-right`) assumes `direction: ltr`, as explained
        /// in the "asymmetry" section of [`Self::PaddingInline`]. This is an
        /// approximation that disagrees with the spec under `direction: rtl`.
        MarginInline(StartEnd<LengthOrAuto>),
        /// `margin-block: <'margin-top'>{1,2}` shorthand — the same grammar,
        /// expansion, and physical-mapping rationale as [`Self::MarginInline`],
        /// but for the block axis (`margin-block-start`/`-end` →
        /// `margin-top`/`margin-bottom`: exact mapping, not an approximation;
        /// see the "asymmetry" section of [`Self::PaddingInline`]).
        /// CSS Logical Properties and Values 1 §4.2
        /// <https://www.w3.org/TR/css-logical-1/#propdef-margin-block>.
        MarginBlock(StartEnd<LengthOrAuto>),
        /// `border-top-width: <line-width>` — non-inherited, initial: `medium`
        /// = `Length::Px(3.0)` (CSS Backgrounds 3 §3.3
        /// <https://www.w3.org/TR/css-backgrounds-3/#border-width>).
        /// `<line-width>` = `<length [0,∞]> | thin | medium | thick`.
        /// `<percentage>` is not part of the grammar (unlike padding). Keyword
        /// mappings use the values specified by the spec: thin=1px, medium=3px,
        /// thick=5px (see the `parse_border_width_side` docs).
        BorderTopWidth(Length),
        /// `border-right-width: <line-width>` — same grammar as [`Self::BorderTopWidth`].
        BorderRightWidth(Length),
        /// `border-bottom-width: <line-width>` — same grammar as [`Self::BorderTopWidth`].
        BorderBottomWidth(Length),
        /// `border-left-width: <line-width>` — same grammar as [`Self::BorderTopWidth`].
        BorderLeftWidth(Length),
        /// `border-top-style: <line-style>` — non-inherited, initial: `none`
        /// (CSS Backgrounds 3 §3.2
        /// <https://www.w3.org/TR/css-backgrounds-3/#border-style>).
        /// See the [`BorderStyle`] variants for the ten keywords.
        BorderTopStyle(BorderStyle),
        /// `border-right-style: <line-style>` — same grammar as [`Self::BorderTopStyle`].
        BorderRightStyle(BorderStyle),
        /// `border-bottom-style: <line-style>` — same grammar as [`Self::BorderTopStyle`].
        BorderBottomStyle(BorderStyle),
        /// `border-left-style: <line-style>` — same grammar as [`Self::BorderTopStyle`].
        BorderLeftStyle(BorderStyle),
        /// `border-top-color: <color>` — non-inherited, initial: the
        /// `currentcolor` keyword ([`BorderColor::CurrentColor`]; CSS Backgrounds
        /// 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>,
        /// "Initial: currentcolor"). The cascade's static side stores the specified
        /// value (currentcolor versus resolved `<color>`) in the [`BorderColor`]
        /// enum. Resolving the used value (currentcolor → this node's computed
        /// `color` property; CSS Color 3 §4.4
        /// <https://www.w3.org/TR/css-color-3/#currentColor-def>) is the paint
        /// scope's responsibility.
        /// (`CssColor` has been promoted to [`BorderColor`].)
        BorderTopColor(BorderColor),
        /// `border-right-color: <color>` — same grammar as [`Self::BorderTopColor`].
        BorderRightColor(BorderColor),
        /// `border-bottom-color: <color>` — same grammar as [`Self::BorderTopColor`].
        BorderBottomColor(BorderColor),
        /// `border-left-color: <color>` — same grammar as [`Self::BorderTopColor`].
        BorderLeftColor(BorderColor),
        /// `border: <line-width> || <line-style> || <color>` shorthand —
        /// assigns the same [`Border`] to all four sides (CSS Backgrounds 3 §3.4
        /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>).
        ///
        /// The spec grammar uses `||` (any order, each component at most once,
        /// at least one required). `parse_border_shorthand` peels components with
        /// an unfilled-slot loop. Omitted components use their initial values:
        /// width=`Length::Px(3.0)` (medium), style=`BorderStyle::None`, and
        /// color=[`BorderColor::CurrentColor`] (spec §3.1 initial).
        ///
        /// **This variant is not observed during element cascade**:
        /// [`crate::rule::expand_shorthand_into`] expands it into twelve longhand
        /// variants (four sides × three sub-properties) both when parsing exits
        /// (`parse_declaration_block`) and when element cascade begins
        /// ([`mod@crate::cascade`]'s `collect_cascaded`). This follows CSS Cascading
        /// L4 §3 "Shorthand Properties"
        /// <https://www.w3.org/TR/css-cascade-4/#shorthand>: "A shorthand
        /// property sets all of its longhand sub-properties, exactly as if expanded
        /// in place." Per-side, per-sub-property cascade winners therefore work
        /// naturally, following the margin/padding shorthand precedent. The
        /// expansion guarantee prevents this variant from reaching cascade. If it
        /// does reach [`crate::cascade::apply_value`], the behavior is **not a safety
        /// net** (corrected framing): it unconditionally overwrites all four sides
        /// and three sub-properties of `ComputedValues.border`, destroying all
        /// twelve longhand winners. Reaching it is already a bug; it cannot degrade
        /// gracefully. See the canonical descriptions in the docs for
        /// [`crate::cascade::apply_value`] and [`crate::rule::expand_shorthand_into`].
        ///
        /// ⚠️ The spec's "all of its longhand sub-properties" includes the five
        /// reset-only `border-image-*` properties (CSS Backgrounds 3 §3.4: the
        /// `border` shorthand also resets `border-image` to its initial value).
        /// They are not implemented, so expansion covers only twelve longhands;
        /// see "Non-goals" below.
        ///
        /// # Non-goals (explicit spec deviation)
        ///
        /// Spec §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
        /// says the border shorthand **also resets border-image-*** (the spec's
        /// verbatim wording occurs only in the `parse_border_shorthand` docs).
        /// This crate does not implement border-image, so the reset is unsupported
        /// (spec-valid but outside this crate's scope). A future integration task
        /// must handle it alongside the border-image longhands.
        Border(Sides<Border>),
        /// `border-style: <line-style>{1,4}` — non-inherited (CSS Backgrounds 3
        /// §3.2 `<line-style>` × §3.4 shorthands
        /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>).
        /// The 1–4-value expansion follows the margin precedent
        /// (the same shape as `parse_margin_shorthand`): 1 → all sides,
        /// 2 → vertical/horizontal, 3 → top/horizontal/bottom,
        /// 4 → clockwise. `rule.rs` expands it to four longhands
        /// (including `BorderTopStyle`).
        BorderStyle(Sides<BorderStyle>),
        /// `border-width: <line-width>{1,4}` — non-inherited (CSS Backgrounds 3
        /// §3.3 × §3.4). Each side has the same grammar as `border-*-width`
        /// (`parse_border_width_side`: thin/medium/thick keywords and non-negative
        /// `<length>`). The 1–4-value expansion follows the margin precedent.
        BorderWidth(Sides<Length>),
        /// `border-color: <color>{1,4}` — non-inherited (CSS Backgrounds 3 §3.1
        /// × §3.4). Each side has the same grammar as `border-*-color`
        /// (`parse_border_color`: `currentcolor`, named, hash, or function).
        /// The 1–4-value expansion follows the margin precedent.
        BorderColor(Sides<BorderColor>),
        /// `border-right: <line-width> || <line-style> || <color>` — single-side shorthand
        /// for the three right-side longhands (CSS Backgrounds 3 §3.4
        /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>).
        ///
        /// The `||` grammar, omitted-component initial fill, and declaration-ordering
        /// contract match [`Self::Border`]; only the expansion target differs (three
        /// right-side longhands instead of twelve). See [`crate::rule::expand_border_right`].
        BorderRight(Border),
        /// `border: <css-wide-keyword>` — expands to twelve longhand CSS-wide markers
        /// (see [`CssWideKeyword`]).
        #[key(Border)]
        BorderCssWide(CssWideKeyword),
        /// `border-right: <css-wide-keyword>` — expands to three right-side longhand
        /// CSS-wide markers (see [`CssWideKeyword`]).
        #[key(BorderRight)]
        BorderRightCssWide(CssWideKeyword),
        /// `border-top-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderTopWidth)]
        BorderTopWidthCssWide(CssWideKeyword),
        /// `border-right-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderRightWidth)]
        BorderRightWidthCssWide(CssWideKeyword),
        /// `border-bottom-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderBottomWidth)]
        BorderBottomWidthCssWide(CssWideKeyword),
        /// `border-left-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderLeftWidth)]
        BorderLeftWidthCssWide(CssWideKeyword),
        /// `border-top-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderTopStyle)]
        BorderTopStyleCssWide(CssWideKeyword),
        /// `border-right-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderRightStyle)]
        BorderRightStyleCssWide(CssWideKeyword),
        /// `border-bottom-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderBottomStyle)]
        BorderBottomStyleCssWide(CssWideKeyword),
        /// `border-left-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderLeftStyle)]
        BorderLeftStyleCssWide(CssWideKeyword),
        /// `border-top-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderTopColor)]
        BorderTopColorCssWide(CssWideKeyword),
        /// `border-right-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderRightColor)]
        BorderRightColorCssWide(CssWideKeyword),
        /// `border-bottom-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderBottomColor)]
        BorderBottomColorCssWide(CssWideKeyword),
        /// `border-left-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
        #[key(BorderLeftColor)]
        BorderLeftColorCssWide(CssWideKeyword),
        /// `width: auto | <length-percentage [0,∞]>` — non-inherited,
        /// initial: `auto` (CSS Sizing 3 §3.1.1
        /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>).
        ///
        /// The spec's value grammar also includes `min-content`, `max-content`,
        /// and `fit-content(<length-percentage>)`. These are not yet implemented;
        /// the parser currently silently drops them and accepts only `auto` and
        /// non-negative `<length-percentage>`. Negative values violate the spec's
        /// `[0,∞]` grammar and are dropped.
        ///
        /// Resolving `auto` belongs to downstream layout (the raikiri-dom
        /// `apply_computed_to_style` bridge into `taffy::Style::size.width`).
        Width(LengthOrAuto),
        /// `height: <length-percentage [0,∞]> | auto` — **non-inherited**,
        /// initial: `auto` (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
        /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>).
        ///
        /// The current scope accepts only `auto` and non-negative
        /// `<length-percentage>`. `min-content`, `max-content`, and
        /// `fit-content(<length-percentage>)` are spec-valid but unimplemented;
        /// the parser silently drops them (see the `parse_height` docs).
        ///
        /// The grammar differs from margin (`<length-percentage> | auto`) only in
        /// its non-negative constraint, so the payload reuses sibling
        /// [`Self::Width`]'s [`LengthOrAuto`] type. It combines the non-negative
        /// filter from `parse_padding_side` and the `auto` branch from
        /// `parse_margin_side` (see the `parse_height` docs).
        ///
        /// Resolving percentages against the containing block and computing the
        /// actual layout height for `LengthOrAuto::Auto` are downstream tasks
        /// (the raikiri-dom `apply_computed_to_style` bridge, in a future task).
        /// This crate stays on the cascade's static side and retains the raw
        /// specified value.
        Height(LengthOrAuto),
        /// `max-width: none | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**, initial: `none`
        /// (CSS Sizing 3 §3.2 <https://www.w3.org/TR/css-sizing-3/#max-size-properties>).
        /// `none` maps to `LengthOrAuto::Auto` as placeholder (no max).
        /// Intrinsic keywords map similarly to Auto (WPT parsing valid, layout pending).
        MaxWidth(LengthOrAuto),
        /// `max-height: none | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**, initial: `none`
        /// (CSS Sizing 3 §3.2 <https://www.w3.org/TR/css-sizing-3/#max-size-properties>).
        MaxHeight(LengthOrAuto),
        /// `min-width: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**, initial: `auto`
        /// (CSS Sizing 3 §4 <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
        /// `auto` maps to [`LengthOrAuto::Auto`] (no minimum). Intrinsic keywords map
        /// similarly to Auto (WPT parsing valid, layout pending) — sibling
        /// [`Self::Width`] arms use the same placeholder shape.
        MinWidth(LengthOrAuto),
        /// `min-height: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**, initial: `auto`
        /// (CSS Sizing 3 §4 <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
        /// Same placeholder shape as sibling [`Self::MinWidth`].
        MinHeight(LengthOrAuto),
        /// `box-sizing: content-box | border-box` — **non-inherited**,
        /// initial: `content-box` (CSS Sizing 3 §3.3 "Box Edges for Sizing: the
        /// box-sizing property" <https://www.w3.org/TR/css-sizing-3/#box-sizing>).
        BoxSizing(BoxSizing),
        /// `direction: ltr | rtl` — **inherited**, initial: [`Direction::Ltr`]
        /// (CSS Writing Modes 4 §2.1 "Specifying Directionality: the direction
        /// property" <https://www.w3.org/TR/css-writing-modes-4/#direction>).
        /// The computed value equals the specified value (no relative resolution;
        /// see [`Direction`] docs). The sole consumer is
        /// [`resolve_text_align_match_parent`], but this property also appears
        /// independently in CSS Paged Media 3 Appendix A's page-property-list
        /// (see the verified verbatim quotation in the [`Direction`] docs).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// "declaration order is load-bearing" section of [`PropertyKey`] docs.)
        Direction(Direction),
        /// `overflow-x: visible | hidden | clip | scroll | auto` (legacy
        /// `overlay` aliases `auto`) — **non-inherited**, initial:
        /// [`OverflowValue::Visible`] (CSS Overflow 3 §3.1
        /// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>).
        /// Its computed value may depend on this node's `overflow-y`; see
        /// [`resolve_overflow`] (this is not a simple assignment; also see the
        /// docs for this variant's arm in [`crate::cascade::apply_value`]).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// "declaration order is load-bearing" section of [`PropertyKey`] docs.)
        OverflowX(OverflowValue),
        /// `overflow-y: visible | hidden | clip | scroll | auto` (legacy
        /// `overlay` aliases `auto`) — same grammar, initial value, and
        /// non-inheritance as [`Self::OverflowX`], but on the opposite axis.
        /// (Appended for the same reason as [`Self::OverflowX`].)
        OverflowY(OverflowValue),
        /// `overflow: <'overflow-block'>{1,2}` shorthand — CSS Overflow 3 §3.1
        /// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>. One value
        /// sets both axes; with two values, the first is x and the second is y
        /// (see [`OverflowXY`] docs). The spec maps to logical `overflow-block`/
        /// `overflow-inline`, but raikiri-style does not implement writing-mode,
        /// so it maps directly to physical axes. This is the same kind of carve-out
        /// as the Non-goal section of [`OverflowValue`] docs.
        ///
        /// **This variant is not observed during element cascade**: as with
        /// [`Self::Padding`], [`crate::rule::expand_shorthand_into`] expands it into
        /// the two longhands [`OverflowX`](Self::OverflowX) /
        /// [`OverflowY`](Self::OverflowY) both when parsing exits and when element
        /// cascade begins (CSS Cascading L4 §3 "Shorthand Properties"
        /// <https://www.w3.org/TR/css-cascade-4/#shorthand>). If it does reach
        /// [`crate::cascade::apply_value`], the behavior is **not a safety net**;
        /// see the same framing in [`Self::Padding`] docs.
        /// (Appended for the same reason as [`Self::OverflowX`].)
        Overflow(OverflowXY),
        /// `text-decoration-line` — **non-inherited**, initial:
        /// [`TextDecorationLine::NONE`] (CSS Text Decoration Module Level 3 §2.1
        /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>,
        /// "Inherited: no"). The computed value equals the specified keyword(s)
        /// (see [`TextDecorationLine`] docs; there are no lengths to resolve).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// "declaration order is load-bearing" section of [`PropertyKey`] docs.
        /// Placement is free for a new, disjoint 1:1 field; see the rule at the
        /// end of that section.)
        TextDecorationLine(TextDecorationLine),
        /// `text-decoration-style` — **non-inherited**, initial:
        /// [`TextDecorationStyle::Solid`] (CSS Text Decoration Module Level 3
        /// §2.2 <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>,
        /// "Inherited: no"). The computed value equals the specified keyword
        /// (see [`TextDecorationStyle`] docs). (Appended for the same reason as
        /// [`Self::TextDecorationLine`].)
        TextDecorationStyle(TextDecorationStyle),
        /// `text-decoration-color` — **non-inherited**, initial:
        /// [`TextDecorationColor::CurrentColor`] (CSS Text Decoration Module
        /// Level 3 §2.3
        /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>,
        /// "Inherited: no"). The computed value is the computed color
        /// (see the [`TextDecorationColor`] docs; used-value resolution belongs to the
        /// paint scope). Its position at the end has the same reason as
        /// [`Self::TextDecorationLine`].
        TextDecorationColor(TextDecorationColor),
        /// `text-decoration` shorthand (see [`TextDecorationShorthand`]).
        ///
        /// **This variant is not observed during the element cascade**: as with
        /// [`Self::Padding`], [`crate::rule::expand_shorthand_into`] expands it into the
        /// three longhands [`TextDecorationLine`](Self::TextDecorationLine),
        /// [`TextDecorationStyle`](Self::TextDecorationStyle), and
        /// [`TextDecorationColor`](Self::TextDecorationColor) both at the parser exit
        /// and at the element-cascade entry (per CSS Cascading L4 §3, "Shorthand
        /// Properties", <https://www.w3.org/TR/css-cascade-4/#shorthand>).
        /// If it reaches [`crate::cascade::apply_value`], that path is **not a safety
        /// net**; see the same framing and details in the [`Self::Padding`] docs.
        /// (The shorthand key follows the longhands, as for [`Self::Padding`],
        /// [`Self::Margin`], [`Self::Border`], and [`Self::Overflow`].)
        TextDecoration(TextDecorationShorthand),
        /// `vertical-align: baseline | sub | super | middle | text-top |
        /// text-bottom | <length> | <percentage>` — **non-inherited**, initial:
        /// [`VerticalAlign::Baseline`] (CSS 2.1 §10.8.1
        /// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>).
        /// Computed value: keywords stay unchanged; lengths, percentages, and mixed
        /// `calc()` values are absolutized (see "Scope carving" in the [`VerticalAlign`]
        /// docs).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        VerticalAlign(VerticalAlign),
        /// `font-style: normal | italic | oblique` — **inherited**, initial:
        /// [`FontStyle::Normal`] (see the [`FontStyle`] docs for CSS Fonts 4 §2.4).
        /// The computed value is the specified keyword (see "Scope carving" in the
        /// [`FontStyle`] docs); the angle in `oblique <angle>` and `left`/`right` are
        /// not implemented.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        FontStyle(FontStyle),
        /// `text-transform: none | [capitalize | uppercase | lowercase] ||
        /// full-width || full-size-kana | math-auto` — **inherited**, initial:
        /// [`TextTransform::None`] (CSS Text 4 [`TextTransform`] docs).
        /// The computed value is the specified keyword. `math-auto` is a keyword only;
        /// downstream math-text behavior is outside this crate's scope.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        TextTransform(TextTransform),
        /// `visibility: visible | hidden | collapse` — **inherited**, initial:
        /// [`Visibility::Visible`] (see the [`Visibility`] docs for CSS Display 3 §4).
        /// The computed value is the specified keyword (see "Scope carving" in the
        /// [`Visibility`] docs); the formatting-context-specific space-saving effect
        /// of `collapse` is not implemented.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        Visibility(Visibility),
        /// `z-index: auto | <integer>` — **non-inherited**, initial:
        /// [`ZIndexValue::Auto`] (see the [`ZIndexValue`] docs and CSS2 §9.9.1,
        /// "Inherited: no"). The computed value is the specified value (see the
        /// [`ZIndexValue`] docs; it carries no length requiring relative resolution).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        ZIndex(ZIndexValue),
        /// `word-break: normal | keep-all | break-all` — **inherited**, initial:
        /// [`WordBreak::Normal`] (see the [`WordBreak`] docs for CSS Text 3 §5.1).
        /// The computed value is the specified keyword (see "Scope carving" in the
        /// [`WordBreak`] docs); the deprecated `break-word` value is not implemented.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        WordBreak(WordBreak),
        /// `overflow-wrap: normal | break-word | anywhere` (legacy alias
        /// `word-wrap`) — **inherited**, initial: [`OverflowWrap::Normal`]
        /// (see the [`OverflowWrap`] docs for CSS Text 3 §5.4).
        /// The computed value is the specified keyword (see the [`OverflowWrap`] docs).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        OverflowWrap(OverflowWrap),
        /// `letter-spacing: normal | <length-percentage>` — **inherited**,
        /// initial: [`LetterSpacingValue::Normal`]. The computed representation retains
        /// percentages and mixed calc terms; `normal` resolves to zero. The WPT CSSOM
        /// adapter serializes a zero computed value as `normal`.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        LetterSpacing(LetterSpacingValue),
        /// `word-spacing: normal | <length-percentage>` — **inherited**, initial:
        /// [`WordSpacingValue::Normal`] (CSS Text 4 §8.1 "Word Spacing: the
        /// word-spacing property" <https://drafts.csswg.org/css-text-4/#propdef-word-spacing>).
        /// Computed value: an absolute length and/or percentage; the CSSOM form is
        /// retained in [`crate::computed::ComputedValues::word_spacing_computed`] while
        /// the existing [`crate::computed::ComputedValues::word_spacing`] remains the
        /// renderer/layout fallback.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        WordSpacing(WordSpacingValue),
        /// `break-before: auto | avoid | avoid-page | page` (legacy shorthand
        /// `page-break-before`; see "legacy shorthand" in the [`BreakBetween`] docs)
        /// — **non-inherited**, initial: [`BreakBetween::Auto`] (see the [`BreakBetween`]
        /// docs for CSS Fragmentation Module Level 3 §3.1). The computed value is the
        /// specified keyword (see "Scope carving" in the [`BreakBetween`] docs).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        BreakBefore(BreakBetween),
        /// `break-after: auto | avoid | avoid-page | page` (legacy shorthand
        /// `page-break-after`; see "legacy shorthand" in the [`BreakBetween`] docs)
        /// — **non-inherited**, initial: [`BreakBetween::Auto`] (see the [`BreakBetween`]
        /// docs for CSS Fragmentation Module Level 3 §3.1). The computed value is the
        /// specified keyword (see "Scope carving" in the [`BreakBetween`] docs).
        /// (Appended for the same reason as [`Self::BreakBefore`].)
        BreakAfter(BreakBetween),
        /// `break-inside: auto | avoid | avoid-page` (legacy shorthand
        /// `page-break-inside`; see "legacy shorthand" in the [`BreakInside`] docs)
        /// — **non-inherited**, initial: [`BreakInside::Auto`] (see the [`BreakInside`]
        /// docs for CSS Fragmentation Module Level 3 §3.2). The computed value is the
        /// specified keyword (see "Scope carving" in the [`BreakInside`] docs; this
        /// is a separate type with a smaller value set than [`BreakBetween`]).
        /// (Appended for the same reason as [`Self::BreakBefore`].)
        BreakInside(BreakInside),
        /// `float: none | left | right` — **non-inherited**, initial:
        /// [`FloatValue::None`] (CSS2 §9.5.1, "Inherited: no"; see the [`FloatValue`]
        /// docs). The computed value is the specified value (see the [`FloatValue`]
        /// docs; it carries no length requiring relative resolution). For values other
        /// than `none`, [`resolve_display_for_float`] separately resolves the forced
        /// `display` conversion; this variant carries only the cascaded `float` value.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        Float(FloatValue),
        /// `clear: none | left | right | both` — **non-inherited**, initial:
        /// [`ClearValue::None`] (CSS2 §9.5.2, "Inherited: no"; see the [`ClearValue`]
        /// docs). The computed value is the specified value (see the [`ClearValue`]
        /// docs; it carries no length requiring relative resolution).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        Clear(ClearValue),
        /// `white-space: normal | pre | nowrap | pre-wrap | pre-line` —
        /// **inherited**, initial: [`WhiteSpace::Normal`] (see the [`WhiteSpace`]
        /// docs for CSS Text 3 §3). The computed value is the specified keyword
        /// (see "Scope carving" in the [`WhiteSpace`] docs); the sixth keyword
        /// `break-spaces` is not implemented.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        WhiteSpace(WhiteSpace),
        /// `white-space-collapse: collapse | discard | preserve | preserve-breaks |
        /// preserve-spaces | break-spaces` — inherited, initial:
        /// [`WhiteSpaceCollapse::Collapse`], computed value = specified keyword
        /// (CSS Text 4 property definition:
        /// <https://www.w3.org/TR/css-text-4/#propdef-white-space-collapse>).
        /// This carries the cascade value only; text processing is outside this
        /// crate's scope.
        WhiteSpaceCollapse(WhiteSpaceCollapse),
        /// CSS Text 4 `text-wrap-mode: wrap | nowrap` longhand value. The `text-wrap`
        /// shorthand expands to this value and [`PropertyValue::TextWrapStyle`].
        TextWrap(TextWrapMode),
        /// CSS Text 4 `text-wrap` shorthand, expanded into its two longhands before
        /// cascade.
        #[key(TextWrap)]
        TextWrapShorthand(TextWrapShorthand),
        /// CSS Text 4 `text-wrap-style` computed keyword; no wrapping behavior is
        /// attached to this value in the current style slice.
        TextWrapStyle(TextWrapStyle),
        /// `flex-direction: row | row-reverse | column | column-reverse` —
        /// non-inherited, initial: [`FlexDirectionValue::Row`]
        /// (see the [`FlexDirectionValue`] docs).
        FlexDirection(FlexDirectionValue),
        /// `flex-wrap: nowrap | wrap | wrap-reverse` — non-inherited, initial:
        /// [`FlexWrapValue::NoWrap`] (see the [`FlexWrapValue`] docs).
        FlexWrap(FlexWrapValue),
        /// `flex-grow: <number [0,∞]>` — non-inherited, initial: `0.0`
        /// (CSS Flexible Box Layout Module Level 1 §7.2.1
        /// <https://www.w3.org/TR/css-flexbox-1/#flex-grow-property>).
        /// [`parse_nonneg_finite_number`] enforces both the `[0,∞]` range **and**
        /// finiteness at parse time (see the sink-guard note in the
        /// [`ComputedValues::flex_grow`] docs).
        ///
        /// [`ComputedValues::flex_grow`]: crate::computed::ComputedValues::flex_grow
        FlexGrow(f32),
        /// `flex-shrink: <number [0,∞]>` — non-inherited, initial: `1.0`
        /// (CSS Flexible Box Layout Module Level 1 §7.2.2
        /// <https://www.w3.org/TR/css-flexbox-1/#flex-shrink-property>).
        /// The same parse-time enforcement as [`Self::FlexGrow`] applies.
        FlexShrink(f32),
        /// `flex-basis: content | <'width'>` — non-inherited, initial:
        /// [`FlexBasisValue::Auto`] (see the [`FlexBasisValue`] docs).
        FlexBasis(FlexBasisValue),
        /// `flex: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]`
        /// shorthand — non-inherited, initial: `0 1 auto`
        /// (see the [`FlexShorthand`] docs). [`crate::rule::expand_shorthand_into`]
        /// expands it into the three longhands [`Self::FlexGrow`], [`Self::FlexShrink`],
        /// and [`Self::FlexBasis`], so it normally does not reach the element cascade
        /// (the same pattern as other shorthands such as `Self::Margin`).
        Flex(FlexShorthand),
        /// `flex-flow: <'flex-direction'> || <'flex-wrap'>` shorthand —
        /// non-inherited, initial: `row nowrap` (see the [`FlexFlow`] docs).
        /// [`crate::rule::expand_shorthand_into`] expands it into the two longhands
        /// [`Self::FlexDirection`] and [`Self::FlexWrap`], so it normally does not reach
        /// the element cascade (the same pattern as [`Self::Flex`]).
        FlexFlow(FlexFlow),
        /// `order: <integer>` — non-inherited, initial: `0`
        /// (CSS Flexible Box Layout Module Level 1 §4.2 "Display Order: the order
        /// property" <https://www.w3.org/TR/css-flexbox-1/#order-property>).
        /// The computed value is the specified integer (an opaque pass-through like
        /// [`Self::ZIndex`], since no length requires relative resolution).
        Order(i32),
        /// `justify-content` — non-inherited, initial:
        /// [`ContentAlignmentValue::Normal`] (see the [`ContentAlignmentValue`] docs).
        JustifyContent(ContentAlignmentValue),
        /// `align-content` — non-inherited, initial:
        /// [`ContentAlignmentValue::Normal`]. It shares the same payload type as
        /// [`Self::JustifyContent`] (see the [`ContentAlignmentValue`] docs).
        AlignContent(ContentAlignmentValue),
        /// `align-items` — non-inherited, initial:
        /// [`SelfAlignmentValue::Normal`] (see the [`SelfAlignmentValue`] docs).
        AlignItems(SelfAlignmentValue),
        /// `align-self` — non-inherited, initial: [`AlignSelfValue::Auto`]
        /// (see the [`AlignSelfValue`] docs).
        AlignSelf(AlignSelfValue),
        /// `row-gap: normal | <length-percentage [0,∞]>` — non-inherited,
        /// initial: [`LengthOrNormal::Normal`] (CSS Box Alignment Module Level 3
        /// §8.1 <https://www.w3.org/TR/css-align-3/#propdef-row-gap>).
        /// It reuses [`LengthOrNormal`] (see the [`parse_gap_value`] docs; note that
        /// unlike `letter-spacing`/`word-spacing`, it accepts percentages).
        RowGap(LengthOrNormal),
        /// `column-gap: normal | <length-percentage [0,∞]>` — non-inherited,
        /// initial: [`LengthOrNormal::Normal`]. Its grammar matches [`Self::RowGap`].
        ColumnGap(LengthOrNormal),
        /// `gap: <'row-gap'> <'column-gap'>?` shorthand — non-inherited, initial:
        /// "see individual properties" (see the [`GapShorthand`] docs).
        /// [`crate::rule::expand_shorthand_into`] expands it into the two longhands
        /// [`Self::RowGap`] and [`Self::ColumnGap`].
        Gap(GapShorthand),
        /// `place-content: <'align-content'> <'justify-content'>?` shorthand —
        /// non-inherited, initial: `normal` (see the [`PlaceContentShorthand`] docs).
        /// [`crate::rule::expand_shorthand_into`] expands it into the two longhands
        /// [`Self::AlignContent`] and [`Self::JustifyContent`].
        PlaceContent(PlaceContentShorthand),
        /// `hyphens: none | manual | auto` — **inherited**, initial:
        /// [`Hyphens::Manual`] (see the [`Hyphens`] docs for CSS Text 3 §5.3).
        /// The computed value is the specified keyword (see "Scope carving" and
        /// "Downstream handoff" in the [`Hyphens`] docs; `auto` remains a distinct
        /// variant rather than implementing dictionary-based hyphenation).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        Hyphens(Hyphens),
        /// `tab-size: <number [0,∞]> | <length [0,∞]>` — **inherited**, initial:
        /// [`TabSize::Number`]`(8.0)` (CSS Text Module Level 3 §4.2 "Tab
        /// Character Size: the tab-size property"
        /// <https://www.w3.org/TR/css-text-3/#tab-size-property>).
        /// Computed value: the specified number or an absolutized length (see the
        /// [`TabSize`] docs).
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        TabSize(TabSize),
        /// `line-break: auto | loose | normal | strict | anywhere` — **inherited** (CSS Text 3 §5.2).
        LineBreak(LineBreak),
        /// `text-justify: auto | none | inter-word | inter-character` — **inherited** (CSS Text 3 §6.2).
        TextJustify(TextJustify),
        /// `text-align-all: start | end | left | right | center | justify | match-parent` — **inherited** (CSS Text 3 §6.1).
        TextAlignAll(TextAlignAll),
        /// `text-align-last: auto | start | end | left | right | center | justify | match-parent` — **inherited** (CSS Text 3 §6.1).
        TextAlignLast(TextAlignLast),
        /// `text-combine-upright: none | all` — (CSS Writing Modes 3 §9.1).
        TextCombineUpright(TextCombineUpright),
        /// `text-orientation: mixed | upright | sideways` — (CSS Writing Modes 3 §5.1).
        TextOrientation(TextOrientation),
        /// `unicode-bidi: normal | embed | isolate | bidi-override | isolate-override | plaintext` — (CSS Writing Modes 3 §2.2).
        UnicodeBidi(UnicodeBidi),
        /// `font-variant-caps: normal | small-caps | all-small-caps |
        /// petite-caps | all-petite-caps | unicase | titling-caps` —
        /// **inherited**, initial: [`FontVariantCaps::Normal`] (see the
        /// [`FontVariantCaps`] docs for CSS Fonts 3 §6.6). The computed value is the
        /// specified keyword (see "Scope carving" in the [`FontVariantCaps`] docs);
        /// the `font-variant` shorthand is not implemented.
        /// (Appended to avoid shifting existing variant discriminants; see the
        /// [`PropertyKey`] docs on declaration order. This new field maps 1:1 to a
        /// disjoint field, so its placement is otherwise flexible; see the rule at the
        /// end of that section.)
        FontVariantCaps(FontVariantCaps),
        /// `quotes: none | [ <string> <string> ]+` — **inherited**.
        ///
        /// This implementation follows CSS Content Module Level 3 §2.4.1
        /// "Quotation Mark System: the quotes
        /// property" <https://www.w3.org/TR/css-content-3/#quotes-property>,
        /// and its predecessor CSS2 §12.3.1
        /// <https://www.w3.org/TR/CSS21/generate.html#quotes-specify> by implementing
        /// the same legacy grammar `[ <string> <string> ]+ | none`. The `auto` and
        /// `match-parent` keyword alternatives added by CSS Content 3 are outside
        /// this crate's scope (unimplemented and spec-valid, but rejected at parse
        /// time; see "Unsupported" below).
        ///
        /// Each pair contains the (open, close) quote strings for one nesting level
        /// (quote depth). The definition of quote depth and the pair selection rule
        /// come from the `<quote>` keyword ([`QuoteKeyword`]) section, not this
        /// property definition: CSS Content 3 §2.4.2
        /// <https://www.w3.org/TR/css-content-3/#quote-values> (also CSS2 §12.3.2
        /// <https://www.w3.org/TR/CSS21/generate.html#quotes-insert>). Verbatim
        /// (§2.4.2): "the number of occurrences of open-quote in all generated
        /// text before the current occurrence, minus the number of occurrences of
        /// close-quote […]. If the depth is 0, the first pair is used, if the depth
        /// is 1, the second pair is used, etc. […] If the depth is greater than the
        /// number of pairs, the last pair is repeated." Depth is **0-indexed**
        /// (depth 0 selects the first pair), and excess depth reuses the last pair.
        ///
        /// **Relation to the `content` property's `open-quote` / `close-quote`
        /// keywords ([`QuoteKeyword`])**: A `<quote>` keyword represents only a
        /// nesting-depth change and whether a quote is inserted; it does not choose
        /// the actual string (see [`QuoteKeyword`]). Resolving depth to a quote
        /// string requires this variant's pairs by nesting level. Resolution itself
        /// is outside this crate's static-side scope: downstream raikiri-dom resolves
        /// the [`ContentComponent::Quote`] components of `content` against this
        /// property's computed value at runtime (the same division of responsibility
        /// as the downstream-resolution section of [`QuoteKeyword`]).
        ///
        /// The spec's initial value is "depends on user agent" (CSS2 §12.3.1); it
        /// does not prescribe concrete quote strings. To keep the implementation
        /// independent of other user agents' defaults, this implementation represents
        /// an undeclared initial value as an empty list, just like `none` (see
        /// [`empty_quotes_entries`]).
        ///
        /// **Unsupported (spec-valid)**: `auto` / `match-parent` are keyword
        /// alternatives that CSS Content 3 §2.4.1 added to the legacy CSS2 §12.3.1
        /// grammar. Neither is implemented; both are rejected at parse time (the
        /// `parse_quotes_property` grammar does not accept them, so their
        /// declarations are silently dropped).
        ///
        /// The [`Arc<Vec<..>>`] wrapper has the same rationale as
        /// [`Self::CounterReset`]: for `* { quotes: "«" "»" }` across N elements,
        /// cascade-winner and inheritance-walk clones become shallow Arc reference
        /// count increments, mitigating DoS risk.
        Quotes(Arc<Vec<(SmolStr, SmolStr)>>),
        /// `text-shadow: none | <shadow>#` — **inherited**, initial: `none`
        /// (CSS Text Decoration Module Level 3 §4
        /// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>,
        /// "Initial: none" / "Inherited: yes"). `none` is represented by an empty
        /// list (see [`TextShadowItem`]; the same precedent as
        /// [`Self::CounterReset`]). The computed value is a list whose lengths are
        /// absolute (spec: "a list, each item consisting of three absolute lengths
        /// plus a computed color"). Length absolutization is deferred to phase 3,
        /// as with [`Self::LetterSpacing`], which stores the specified form.
        /// (Appended so existing variant discriminants do not shift; see the
        /// declaration-order section of [`PropertyKey`]. This is a new 1:1
        /// disjoint field, so its position is otherwise unrestricted.)
        TextShadow(Arc<Vec<TextShadowItem>>),
        /// `border-radius` — non-inherited. Expands the four-corner
        /// `<length-percentage>` shorthand into [`BorderRadius`]. Percentages are
        /// retained through computed-value processing; the slash-separated
        /// elliptical form is unsupported.
        BorderRadius(BorderRadius),
        /// `border-radius: inherit` — resolved from the parent computed corners.
        #[key(BorderRadius)]
        BorderRadiusInherit,
        /// `border-top-left-radius` longhand (circular `<length>` subset).
        BorderRadiusTopLeft(Length),
        /// `border-top-right-radius` longhand (circular `<length>` subset).
        BorderRadiusTopRight(Length),
        /// `border-bottom-right-radius` longhand (circular `<length>` subset).
        BorderRadiusBottomRight(Length),
        /// `border-bottom-left-radius` longhand (circular `<length>` subset).
        BorderRadiusBottomLeft(Length),
        /// `box-shadow: none | <shadow>#` — non-inherited. Retains multiple entries.
        /// Each entry retains `inset` and omitted colors.
        BoxShadow(Arc<Vec<BoxShadowItem>>),
        /// `outline` shorthand — non-inherited. Retains width/style/color;
        /// ensuring that outlines do not affect layout is downstream layout's
        /// responsibility. [`crate::rule::expand_shorthand_into`] expands it into
        /// [`Self::OutlineWidth`] / [`Self::OutlineStyle`] / [`Self::OutlineColor`].
        Outline(Outline),
        /// `outline-width: <line-width>` — non-inherited, initial: `medium`.
        OutlineWidth(Length),
        /// `outline-style: auto | <border-style>` — non-inherited, initial: `none`.
        /// The existing outline parser rejects `hidden` as out of scope.
        OutlineStyle(OutlineStyle),
        /// `outline-color: invert | <color>` — non-inherited, initial: `invert`
        /// (CSS UI 3 §4.4). [`OutlineColor`] keeps `invert`, `currentcolor`, and
        /// resolved colors distinct through computed-value processing.
        OutlineColor(OutlineColor),
        /// `outline-offset: <length>` — non-inherited, initial: `0` (CSS UI 3 §4.5
        /// <https://www.w3.org/TR/css-ui-3/#outline-offset>). Negative values are
        /// accepted; the offset from the border edge is absolutized. `<percentage>`
        /// is outside the grammar.
        OutlineOffset(Length),
        /// `grid-area` shorthand — placement for one grid item.
        GridArea(GridAreaShorthand),
        /// `grid` shorthand — the supported explicit `rows / columns` form.
        Grid(GridShorthand),
        /// `grid-template-columns` — non-inherited, initial:
        /// [`GridTemplateTracks::None`] (see [`GridTemplateTracks`]).
        GridTemplateColumns(GridTemplateTracks),
        /// `grid-template-rows` — non-inherited, initial:
        /// [`GridTemplateTracks::None`]. Same grammar and shape as
        /// [`Self::GridTemplateColumns`].
        GridTemplateRows(GridTemplateTracks),
        /// `grid-template-areas` — non-inherited, initial:
        /// [`GridTemplateAreasValue::None`] (see [`GridTemplateAreasValue`]).
        GridTemplateAreas(GridTemplateAreasValue),
        /// `grid-auto-columns: <track-size>+` — non-inherited, initial: `auto`
        /// (one `[GridTrackSize::Breadth(GridTrackBreadth::Auto)]` element; CSS
        /// Grid Layout Module Level 1 §7.6
        /// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-columns>). The `Arc`
        /// wrapper has the same rationale as [`Self::GridTemplateColumns`].
        GridAutoColumns(Arc<Vec<GridTrackSize>>),
        /// `grid-auto-rows` — non-inherited, initial: `auto`.
        /// Same grammar and shape as [`Self::GridAutoColumns`].
        GridAutoRows(Arc<Vec<GridTrackSize>>),
        /// `grid-auto-flow` — non-inherited, initial: [`GridAutoFlowValue::Row`]
        /// (see [`GridAutoFlowValue`]).
        GridAutoFlow(GridAutoFlowValue),
        /// `grid-row-start` — non-inherited, initial: [`GridLineValue::Auto`]
        /// (see [`GridLineValue`]).
        GridRowStart(GridLineValue),
        /// `grid-row-end` — non-inherited, initial: [`GridLineValue::Auto`].
        GridRowEnd(GridLineValue),
        /// `grid-column-start` — non-inherited, initial: [`GridLineValue::Auto`].
        GridColumnStart(GridLineValue),
        /// `grid-column-end` — non-inherited, initial: [`GridLineValue::Auto`].
        GridColumnEnd(GridLineValue),
        /// `grid-row: <grid-line> [ / <grid-line> ]?` shorthand — non-inherited,
        /// initial: `auto` (see [`GridLineShorthand`]).
        /// [`crate::rule::expand_shorthand_into`] expands it into two longhands:
        /// [`Self::GridRowStart`] / [`Self::GridRowEnd`].
        GridRow(GridLineShorthand),
        /// `grid-column` shorthand — non-inherited, initial: `auto`.
        /// [`crate::rule::expand_shorthand_into`] expands it into two longhands:
        /// [`Self::GridColumnStart`] / [`Self::GridColumnEnd`].
        GridColumn(GridLineShorthand),
        /// `justify-items` — non-inherited. Reuses [`SelfAlignmentValue`] from
        /// `align-items` (the same payload type as [`Self::AlignItems`]).
        ///
        /// # Scope carving — `legacy` is unsupported
        ///
        /// The spec grammar in CSS Box Alignment Module Level 3 §7.1
        /// (<https://www.w3.org/TR/css-align-3/#propdef-justify-items>) is
        /// `normal | stretch | <baseline-position> |
        /// <overflow-position>? [ <self-position> | left | right ] | legacy |
        /// legacy && [ left | right | center ]`, with "Initial: `legacy`". Beyond
        /// the alternatives outside `<self-position>` (the
        /// `<overflow-position>`/`left`/`right` scope carve-out in
        /// [`SelfAlignmentValue`]), the `legacy` keyword and its special
        /// "effectively inherit into descendants" behavior are unsupported.
        /// The spec says verbatim: "if the inherited value of justify-items
        /// includes the legacy keyword, this value computes to the inherited
        /// value; otherwise it computes to normal". This behavior exists for
        /// legacy alignment of the HTML `<center>` element and `align` attribute.
        /// taffy 0.12's `justify_items: Option<AlignItems>` also has no
        /// representation for `legacy`.
        ///
        /// As the spec's "otherwise it computes to normal" branch indicates,
        /// without a `legacy` mechanism (and hence with no way to inherit
        /// `legacy`), the effective result in this crate is always `normal`.
        /// [`SelfAlignmentValue::Normal`] rather than the spec's `legacy` as the
        /// initial value directly represents this result.
        JustifyItems(SelfAlignmentValue),
        /// `justify-self` — non-inherited, initial: [`AlignSelfValue::Auto`].
        /// Reuses [`AlignSelfValue`] from `align-self` (the same payload type as
        /// [`Self::AlignSelf`]). CSS Box Alignment Module Level 3 §6.1
        /// <https://www.w3.org/TR/css-align-3/#propdef-justify-self>.
        JustifySelf(AlignSelfValue),
        /// `place-items: <'align-items'> <'justify-items'>?` shorthand —
        /// non-inherited, initial: "see individual properties" (see
        /// [`PlaceItemsShorthand`]). [`crate::rule::expand_shorthand_into`] expands
        /// it into two longhands: [`Self::AlignItems`] / [`Self::JustifyItems`].
        PlaceItems(PlaceItemsShorthand),
        /// `place-self: <'align-self'> <'justify-self'>?` shorthand —
        /// non-inherited, initial: `auto` (see [`PlaceSelfShorthand`]).
        /// [`crate::rule::expand_shorthand_into`] expands it into two longhands:
        /// [`Self::AlignSelf`] / [`Self::JustifySelf`].
        PlaceSelf(PlaceSelfShorthand),
        /// `orphans` — **inherited**, initial: `2` (CSS Fragmentation Module
        /// Level 3 §3.3 "Breaks Between Lines: orphans, widows"
        /// <https://www.w3.org/TR/css-break-3/#widows-orphans>; this supersedes
        /// the CSS 2.1 §13.3.2 definition without changing its grammar). Value:
        /// `<integer>`. Computed value: the specified integer.
        ///
        /// The spec permits only positive integers: "Only positive integers are
        /// allowed as values of orphans and widows. Negative values and zero are
        /// invalid and must cause the declaration to be ignored." This is
        /// enforced at parse time (see `parse_positive_integer`), so this payload
        /// is always `> 0`.
        ///
        /// This crate implements parsing and inherited storage only. Enforcement
        /// of the minimum line count during pagination is not implemented.
        Orphans(i32),
        /// `widows` — the same grammar, initial value, inheritance, and
        /// positive-integer constraint as [`Self::Orphans`] (CSS Fragmentation
        /// Module Level 3 §3.3, the same property-definition table). It differs
        /// only in which side of a fragmentation break needs the minimum line
        /// count: after the break, rather than before it as with `orphans`.
        Widows(i32),
        /// `writing-mode: horizontal-tb | vertical-rl | vertical-lr | sideways-rl
        /// | sideways-lr` — **inherited**, initial: [`WritingMode::HorizontalTb`]
        /// (CSS Writing Modes 4 §3.2; see [`WritingMode`]). All five keywords are
        /// accepted syntactically, but the computed value is always normalized to
        /// [`WritingMode::HorizontalTb`] (see the Non-goal section of
        /// [`WritingMode`] and [`resolve_writing_mode`]; raikiri has no vertical
        /// writing rendering pipeline).
        /// (Appended so existing variant discriminants do not shift; see the
        /// declaration-order section of [`PropertyKey`]. This is a new 1:1
        /// disjoint field, so its position is otherwise unrestricted.)
        WritingMode(WritingMode),
        /// `ruby-position` — inherited, initial: [`RubyPosition::Over`].
        RubyPosition(RubyPosition),
        /// `background-repeat` — **non-inherited**, initial:
        /// [`BackgroundRepeat`]`{x: Repeat, y: Repeat}` (CSS Backgrounds 3 §2.4;
        /// see [`BackgroundRepeat`]). Appended as a new 1:1 disjoint field under
        /// the placement rule in [`PropertyKey`].
        BackgroundRepeat(BackgroundRepeat),
        /// `background-attachment` — **non-inherited**, initial:
        /// [`BackgroundAttachment::Scroll`] (CSS Backgrounds 3 §2.5; see
        /// [`BackgroundAttachment`]).
        BackgroundAttachment(BackgroundAttachment),
        /// `background-clip` — **non-inherited**, initial:
        /// [`VisualBox::BorderBox`] (CSS Backgrounds 3 §2.7; see [`VisualBox`]).
        /// Note that sibling [`Self::BackgroundOrigin`] has a different initial value.
        BackgroundClip(VisualBox),
        /// `background-origin` — **non-inherited**, initial:
        /// [`VisualBox::PaddingBox`] (CSS Backgrounds 3 §2.8; see [`VisualBox`]).
        /// Note that sibling [`Self::BackgroundClip`] has a different initial value.
        BackgroundOrigin(VisualBox),
        /// `background-size` — **non-inherited**, initial:
        /// [`BackgroundSize::Explicit`]`{width: Auto, height: Auto}` (CSS
        /// Backgrounds 3 §2.9; see [`BackgroundSize`]). Since this contains
        /// `<length-percentage>`, absolutization is deferred to phase 3 (as with
        /// `padding` and `width`).
        BackgroundSize(BackgroundSize),
        /// `background-position` — **non-inherited**, initial:
        /// [`CssPosition`]`{horizontal: Start(Percent(0.0)), vertical:
        /// Start(Percent(0.0))}` (CSS Backgrounds 3 §2.6 "Initial: 0% 0%"; see
        /// [`CssPosition`]). Since this contains `<length-percentage>`,
        /// absolutization is deferred to phase 3.
        BackgroundPosition(CssPosition),
        /// `background-image` — **non-inherited**, initial: [`BackgroundImage::None`]
        /// (CSS Backgrounds 3 §2.3; see [`BackgroundImage`]). Appended as a new 1:1
        /// disjoint field under the placement rule in [`PropertyKey`].
        BackgroundImage(BackgroundImage),
        /// `background` shorthand — non-inherited. Retains eight components
        /// (color/image/repeat/attachment/position/size/clip/origin; see
        /// [`BackgroundShorthand`]). Only one layer is supported (see its Non-goal
        /// section). [`crate::rule::expand_shorthand_into`] expands it into eight
        /// longhands: [`Self::BackgroundColor`] / [`Self::BackgroundImage`] /
        /// [`Self::BackgroundRepeat`] / [`Self::BackgroundAttachment`] /
        /// [`Self::BackgroundPosition`] / [`Self::BackgroundSize`] /
        /// [`Self::BackgroundClip`] / [`Self::BackgroundOrigin`]. Appended despite
        /// not being 1:1 disjoint (the eight existing longhands have their own
        /// fields): shorthands do not reach the cascade stage (see
        /// [`crate::rule::expand_shorthand_into`]), so discriminant order does not
        /// matter. Avoiding shifts of existing variants takes priority (see the
        /// declaration-order section of [`PropertyKey`]).
        Background(BackgroundShorthand),
        /// `object-fit` — **non-inherited**, initial: [`ObjectFit::Fill`] (CSS
        /// Images 3 §5.1; see [`ObjectFit`]). Appended as a new 1:1 disjoint field
        /// under the placement rule in [`PropertyKey`].
        ObjectFit(ObjectFit),
        /// `object-position` — **non-inherited**, initial: `50% 50%` (CSS Images
        /// 3 §5.2 "Initial: 50% 50%"; see [`CssPosition`]). It reuses the same
        /// [`CssPosition`] type as `background-position` (see the section about
        /// `background-position` / `object-position` in [`CssPosition`]), but the
        /// grammars differ. It requires strict `<position>` (CSS Values 4 §8.3),
        /// which disallows the `<bg-position>`-specific three-value edge-offset
        /// syntax. Therefore it is parsed with [`parse_position_strict`] instead
        /// of [`parse_bg_position`] (see [`parse_position_branch3_strict`]).
        /// Since it contains `<length-percentage>`, absolutization is deferred to
        /// phase 3.
        ObjectPosition(CssPosition),
        /// `opacity` — **non-inherited**, initial: `1` (CSS Color 4 §3.3
        /// "Transparency: the opacity property"
        /// <https://www.w3.org/TR/css-color-4/#transparency>, "Value:
        /// `<opacity-value>`", "Inherited: no"). Grammar: `<opacity-value> =
        /// <number> | <percentage>`.
        ///
        /// This payload **retains the specified value without clamping**. The spec
        /// says: "Opacity values outside the range \[0, 1\] are not invalid, and
        /// are preserved in specified values, but are clamped to the range
        /// \[0, 1\] in computed values." Clamping belongs to phase 3
        /// ([`crate::specified::SpecifiedValues::absolutize_with`] /
        /// [`crate::page`]'s `absolutize_in_page_context`); this variant carries
        /// even out-of-range values (for example, `opacity: 2`). Appended as a new
        /// 1:1 disjoint field under the placement rule in [`PropertyKey`].
        Opacity(f32),
        /// `mix-blend-mode` — **non-inherited**, initial:
        /// [`MixBlendMode::Normal`] (CSS Compositing and Blending Level 1 §3.4.1;
        /// see [`MixBlendMode`]). Appended as a new 1:1 disjoint field under the
        /// placement rule in [`PropertyKey`].
        MixBlendMode(MixBlendMode),
        /// `mask-image` — **non-inherited**, initial: [`MaskImage::None`] (CSS
        /// Masking Level 1 §7.1; see [`MaskImage`]). Appended as a new 1:1
        /// disjoint field under the placement rule in [`PropertyKey`].
        MaskImage(MaskImage),
        /// `clip-path` — **non-inherited**, initial: [`ClipPath::None`] (CSS
        /// Masking Level 1 §5.1; see [`ClipPath`]). Appended as a new 1:1
        /// disjoint field under the placement rule in [`PropertyKey`].
        ClipPath(ClipPath),
        /// `transform` — **non-inherited**, initial: `none` (CSS Transforms
        /// Level 1 §4; see [`TransformFunction`]). `none` is represented by an
        /// empty list ([`empty_transform_list`]), as with `BoxShadow`'s `none`
        /// represented by an empty `Vec`. Appended as a new 1:1 disjoint field
        /// under the placement rule in [`PropertyKey`].
        Transform(Arc<Vec<TransformFunction>>),
        /// Origin on the border box plus a Z length (CSS Transforms 1 §5).
        /// Z is retained for computed values; the painter currently uses 2D transforms.
        TransformOrigin(CssPosition, Length),
        /// `filter` — **non-inherited**, initial: `none` (CSS Filter Effects
        /// Level 1 §5; see [`FilterFunction`]). Uses the same empty-list-means-none
        /// convention as `Transform` ([`empty_filter_list`]). Appended as a new
        /// 1:1 disjoint field under the placement rule in [`PropertyKey`].
        Filter(Arc<Vec<FilterFunction>>),
        /// `table-layout: auto | fixed` — **non-inherited**, initial:
        /// [`TableLayoutValue::Auto`] (CSS Tables 3 §4; see [`TableLayoutValue`]).
        /// Computed value: the specified keyword (no relative lengths to resolve).
        /// (Appended so existing variant discriminants do not shift; see the
        /// declaration-order section of [`PropertyKey`]. This is a new 1:1
        /// disjoint field, so its position is otherwise unrestricted.)
        TableLayout(TableLayoutValue),
        /// `border-collapse: collapse | separate` — **inherited**, initial:
        /// [`BorderCollapseValue::Separate`] (CSS Tables 3 §6; see
        /// [`BorderCollapseValue`]). Computed value: the specified keyword
        /// (no relative lengths to resolve).
        /// (Appended for the same reason as [`Self::TableLayout`].)
        BorderCollapse(BorderCollapseValue),
        /// `border-spacing: <length>{1,2}` — **inherited**, initial: `0`
        /// (`0px` on both axes; CSS Tables 3 §6.1; see [`BorderSpacingValue`]).
        /// The computed value has two absolute lengths. Phase 3 absolutizes them
        /// via [`crate::resolve::resolve_border_spacing`] (the same staging pattern
        /// for length-bearing values as [`Self::TabSize`]).
        /// (Appended for the same reason as [`Self::TableLayout`].)
        BorderSpacing(BorderSpacingValue),
        /// `caption-side: top | bottom` — **inherited**, initial:
        /// [`CaptionSideValue::Top`] (CSS Tables 3 §7; see [`CaptionSideValue`]).
        /// Computed value: the specified keyword (no relative lengths to resolve).
        /// (Appended for the same reason as [`Self::TableLayout`].)
        CaptionSide(CaptionSideValue),
        /// `empty-cells: show | hide` — **inherited**, initial:
        /// [`EmptyCellsValue::Show`] (CSS Tables 3 §8; see [`EmptyCellsValue`]).
        /// Computed value: the specified keyword (no relative lengths to resolve).
        /// (Appended for the same reason as [`Self::TableLayout`].)
        EmptyCells(EmptyCellsValue),
        /// `font` shorthand — **inherited**. Retains six grammar components
        /// (style/variant-caps/weight/size/line-height/family; see
        /// [`FontShorthand`]). Its subset of CSS Fonts 4 §2.1
        /// <https://www.w3.org/TR/css-fonts-4/#font-prop> uses
        /// `[ <'font-style'> || <font-variant-css2> || <'font-weight'> ]? <'font-size'> [ / <'line-height'> ]? <'font-family'>#`.
        /// System-font keywords, non-`normal` `font-stretch`, and `font-variant`
        /// values outside CSS2 are out of scope (see Scope carving in
        /// [`FontShorthand`]). [`crate::rule::expand_shorthand_into`] expands it
        /// into six grammar longhands and nine modeled reset-only subproperties:
        /// `font-kerning` / `font-language-override` / `font-optical-sizing` /
        /// `font-variant-east-asian` / `font-variant-emoji` /
        /// `font-variant-ligatures` / `font-variant-numeric` /
        /// `font-variant-position` / `font-variation-settings`.
        /// Appended because shorthands do not reach the cascade stage (see
        /// [`crate::rule::expand_shorthand_into`]), so discriminant order does not
        /// matter. Avoiding shifts of existing variants takes priority (see the
        /// declaration-order section of [`PropertyKey`]).
        Font(FontShorthand),
        /// `text-decoration-skip-ink` — **inherited**, initial:
        /// [`TextDecorationSkipInk::Auto`] (see [`TextDecorationSkipInk`]).
        /// The element cascade preserves this keyword in the staging and computed
        /// values. No skip-ink decoration geometry or painting is implemented.
        /// (Appended for the same reason as [`Self::TableLayout`].)
        TextDecorationSkipInk(TextDecorationSkipInk),
        /// `text-decoration-skip-spaces` — **inherited**, initial: `start end`
        /// ([`TextDecorationSkipSpaces::StartEnd`]). The element cascade carries
        /// this inherited keyword set into computed style; decoration painting is
        /// out of scope.
        /// (Appended for the same reason as [`Self::TableLayout`].)
        TextDecorationSkipSpaces(TextDecorationSkipSpaces),
        /// `text-decoration-thickness` — **non-inherited**, initial:
        /// [`TextDecorationThickness::Auto`] (see [`TextDecorationThickness`]).
        /// The element path stages it in [`crate::specified::SpecifiedValues`]
        /// and absolutizes its length to computed CSS px via
        /// [`crate::resolve::resolve_text_decoration_thickness`]. The `@page`
        /// path absolutizes it separately via `absolutize_in_page_context` in
        /// [`crate::page`].
        /// (Appended for the same reason as [`Self::TableLayout`].)
        TextDecorationThickness(TextDecorationThickness),
        /// `text-decoration-inset` — **non-inherited**, initial: `0`
        /// (ED). The element path stages it in
        /// [`crate::specified::SpecifiedValues`] and absolutizes it against the
        /// declaring node's font metrics via
        /// [`crate::resolve::resolve_text_decoration_inset`] before painting.
        /// (Appended for the same reason as [`Self::TableLayout`].)
        TextDecorationInset(TextDecorationInset),
        /// `text-emphasis-position` — **inherited**, initial: `over right`
        /// (ED). The element cascade stages this computed-equivalent keyword value
        /// in [`crate::specified::SpecifiedValues`]; emphasis placement is out of
        /// scope.
        /// (Appended for the same reason as [`Self::TableLayout`].)
        TextEmphasisPosition(TextEmphasisPosition),
        /// `text-underline-position` — **inherited**, initial:
        /// [`TextUnderlinePosition::AUTO`]. The element cascade stages this
        /// computed-equivalent keyword set in [`crate::specified::SpecifiedValues`].
        /// Underline placement is out of scope.
        /// (Appended for the same placement reason as [`Self::TableLayout`].)
        TextUnderlinePosition(TextUnderlinePosition),
        /// `page: auto | <custom-ident>` — **non-inherited**, initial:
        /// [`PageValue::Auto`] (see the [`PageValue`] docs and CSS Paged Media 3 §8.1).
        /// Parsing only: the cascade drops the winner instead of storing it in a
        /// staging field (as with `TextCombineUpright` in E/F/G).
        /// (Appended for the same placement reason as [`Self::TableLayout`].)
        Page(PageValue),
        /// `column-count` — non-inherited, positive integer or `auto`.
        ColumnCount(ColumnCountValue),
        /// `column-width` — non-inherited, non-negative length or `auto`.
        ColumnWidth(ColumnWidthValue),
        /// `columns` shorthand for `column-width` and `column-count`.
        Columns(ColumnsShorthand),
        /// `min-block-size: auto | <length-percentage [0,∞]>` — logical
        /// minimum block size. The cascade resolves it to the physical axis
        /// selected by the specified `writing-mode` before layout bridging.
        /// Appended to preserve existing variant discriminants.
        MinBlockSize(LengthOrAuto),
        /// `text-underline-offset` — inherited, initial: `auto` (CSS Text
        /// Decoration 4 §2.8). Lengths are absolutized at the declaring element;
        /// percentages stay relative so they scale with the font as they inherit.
        TextUnderlineOffset(TextUnderlineOffset),
        /// `text-autospace` — inherited, initial: `normal` (CSS Text 4).
        TextAutospace(TextAutospace),
        /// `hyphenate-character` — inherited, initial: `auto` (CSS Text 4).
        /// The style value retains the specified keyword or decoded string; it
        /// does not perform hyphenation.
        /// Appended to preserve existing variant discriminants.
        HyphenateCharacter(HyphenateCharacter),
        /// `hyphenate-limit-chars` — inherited, initial `auto` (CSS Text 4).
        /// The parser expands one-to-three authored components to the three
        /// computed components. Math expressions are rounded to an integer;
        /// direct fractional number tokens are invalid. No hyphenation behavior
        /// is implemented here.
        HyphenateLimitChars(HyphenateLimitChars),
        /// `text-spacing-trim` — inherited, initial `normal` (CSS Text 4).
        /// The computed value is the specified keyword; this value is data-only
        /// and does not enable layout or rendering behavior.
        TextSpacingTrim(TextSpacingTrim),
        /// CSS Text 4 `text-spacing` shorthand; expanded into its two longhands.
        #[key(TextSpacing)]
        TextSpacingShorthand(TextSpacingShorthand),
        /// CSS Text 4 `word-space-transform` — inherited, initial `none`.
        /// The value is preserved without adding text transformation behavior.
        /// Appended to preserve existing variant discriminants.
        WordSpaceTransform(WordSpaceTransform),
        /// `text-emphasis-style` — inherited, initial `none` (CSS Text Decoration 4).
        /// Shape/fill and string values are preserved as computed data; emphasis
        /// painting is out of scope. Appended to preserve existing variant tags.
        TextEmphasisStyle(TextEmphasisStyle),
        /// `text-emphasis-color` — inherited, initial `currentColor`.
        /// Appended to preserve existing variant tags.
        TextEmphasisColor(TextDecorationColor),
        /// `text-emphasis` shorthand, expanded into style and color longhands.
        /// Appended to preserve existing variant tags.
        TextEmphasis(TextEmphasisShorthand),
        /// `font-kerning: auto | normal | none` — inherited, initial `auto`.
        /// The computed value is the specified keyword; this value is data-only
        /// and does not enable shaping or painting behavior.
        FontKerning(FontKerning),
        /// `font-optical-sizing: auto | none` — inherited, initial `auto`.
        /// The computed value is the specified keyword; no font selection or
        /// shaping behavior is enabled by this data-only value.
        FontOpticalSizing(FontOpticalSizing),
        /// `font-variant-emoji: normal | text | emoji | unicode` — inherited, initial `normal`.
        /// This data-only value does not alter emoji presentation or glyph selection.
        FontVariantEmoji(FontVariantEmoji),
        /// `font-language-override: normal | <string>` — inherited, initial `normal`.
        /// This data-only value does not select a language system or alter shaping.
        FontLanguageOverride(FontLanguageOverride),
        /// An individual `font-variant-ligatures` keyword, inherited with initial `normal`.
        /// This data-only value does not change ligature shaping or painting.
        FontVariantLigatures(FontVariantLigatures),
        /// `font-synthesis` tested keyword set; data-only, with no synthesis behavior.
        FontSynthesis(FontSynthesisValue),
        /// `font-variant-position` keyword; data-only, with no glyph shaping or synthesis.
        FontVariantPosition(FontVariantPosition),
        /// `font-palette` keyword or dashed identifier; CSSOM data only.
        FontPalette(FontPaletteValue),
        /// `font-variant-numeric` keyword set; data only, with no shaping behavior.
        FontVariantNumeric(FontVariantNumeric),
        /// `font-variant-east-asian` value; data only, with no glyph substitution.
        FontVariantEastAsian(FontVariantEastAsian),
        /// `font-variation-settings` value; data only, with no font-axis application.
        FontVariationSettings(FontVariationSettings),
    }

    /// Property key: the discriminant used to select a winner for each property in
    /// the cascade.
    ///
    /// Used by the per-node winner selection in cascade.rs and as the map key in
    /// [`PageCascadeResult`] by [`cascade_page`](crate::page::cascade_page) in page.rs.
    /// This extracts only the variant tag from `PropertyValue`, with no additional
    /// state. It is public because [`PageCascadeResult`] requires a `pub` type
    /// (to satisfy clippy's `private_interfaces` lint).
    ///
    /// # ⚠️ Variant **declaration order is load-bearing**
    ///
    /// The element cascade uses this enum's discriminant (`key as usize`) as a
    /// scratch-buffer slot index, then **visits slots in ascending index order to
    /// apply their winners** (`apply_winners` in [`mod@crate::cascade`]). Thus:
    ///
    /// - **Where a variant is added** and **the order of existing variants** can
    ///   change the **final value** when multiple winners write to the same field
    ///   in [`crate::specified::SpecifiedValues`] for one node.
    /// - Currently, only shorthand keys (`Padding` / `Margin` / `Border`) could be
    ///   affected, and each is after its longhands. However,
    ///   [`crate::rule::expand_shorthand_into`] expands shorthands into longhands
    ///   both after parsing and on entry to the element cascade, so **shorthand
    ///   keys never reach the element cascade stage**. The `@page` cascade
    ///   ([`crate::page::cascade_page`]) performs the same expansion on entry,
    ///   closing the analogous gap if `PageRule`'s `pub declarations` are mutated
    ///   after parsing.
    ///
    /// **Do not try to fix shorthand/longhand cascading by "fixing" variant
    /// order**: an order-dependent solution must break one of the two mirrored
    /// cases, `margin: 0; margin-top: 10px` and `margin-top: 10px; margin: 0`
    /// (see the `apply_winners` docs). The existing, correct solution is to keep
    /// shorthands out of the cascade stage. The exhaustive match in
    /// [`crate::rule::expand_shorthand_into`] prevents an expansion arm from being
    /// forgotten at compile time.
    ///
    /// When adding a variant, check whether it writes to the same `SpecifiedValues`
    /// field as an existing variant. If not (the fields are disjoint one-to-one),
    /// it can go anywhere.
    ///
    /// [`Direction`] and [`TextAlign`] write to different fields one-to-one
    /// (`SpecifiedValues::direction` and `SpecifiedValues::text_align`). The need
    /// for the **parent's** computed `direction` when resolving
    /// `text-align: match-parent` is **unrelated** to this `PropertyKey` order.
    /// After all winners have been applied, the caller of
    /// [`crate::property::resolve_text_align_match_parent`],
    /// [`crate::specified::SpecifiedValues::finalize`], receives the explicit
    /// parent [`crate::computed::ComputedValues`] and resolves it there. Thus it
    /// does not depend on the `apply_winners` slot iteration order (the declaration
    /// order of this enum).
    ///
    /// [`PageCascadeResult`]: crate::page::PageCascadeResult
    #[non_exhaustive]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum PropertyKey {
        Color,
        BackgroundColor,
        FontFamily,
        FontSize,
        FontWeight,
        LineHeight,
        Display,
        /// `grid` shorthand, which resets and sets the grid template/auto values.
        Grid,
        /// `grid-area` placement shorthand.
        GridArea,
        CounterReset,
        CounterIncrement,
        CounterSet,
        Content,
        StringSet,
        Position,
        Top,
        Right,
        Bottom,
        Left,
        TextAlign,
        TextIndent,
        PaddingTop,
        PaddingRight,
        PaddingBottom,
        PaddingLeft,
        /// Discriminant subject to the "shorthand vs longhand cascade" constraint
        /// documented on [`PropertyValue::Padding`]: shorthand and longhand each
        /// select a separate winner. CSS Cascading L4 §3
        /// <https://www.w3.org/TR/css-cascade-4/#shorthand> says "exactly as if
        /// expanded in place". This treats a shorthand as syntactic sugar for its
        /// longhands, so separate winners would be a deviation. However,
        /// [`crate::rule::expand_shorthand_into`] expands shorthands both after
        /// parsing and on entry to the element cascade. This variant therefore
        /// never reaches that cascade stage, so there is no observable divergence.
        /// Entry-side expansion in [`crate::page::cascade_page`] closes the analogous
        /// gap for `@page`. The exhaustive match in
        /// [`crate::rule::expand_shorthand_into`] prevents a missing expansion arm
        /// at compile time.
        Padding,
        // padding-inline / padding-block logical 2-value shorthand (CSS Logical
        // Properties and Values 1 §4.4, semantics on the matching
        // PropertyValue::PaddingInline / PropertyValue::PaddingBlock variants).
        // Placed after `Padding` under the convention that shorthand keys follow
        // any longhands they can compete with (see the "declaration order is
        // load-bearing" section in the `PropertyKey` docs). Both expand into a
        // subset of the same four padding longhands as `Padding`.
        PaddingInline,
        PaddingBlock,
        // margin longhand + shorthand (semantics on the
        // matching PropertyValue::Margin* variants; sibling PropertyKey variants
        // carry no per-variant docs per crate convention).
        MarginTop,
        MarginRight,
        MarginBottom,
        MarginLeft,
        Margin,
        // margin-inline / margin-block logical 2-value shorthand — same
        // placement rationale as `PaddingInline`/`PaddingBlock` above
        // (semantics on the matching PropertyValue::MarginInline /
        // PropertyValue::MarginBlock variants).
        MarginInline,
        MarginBlock,
        // border longhand + shorthand (semantics on the
        // matching PropertyValue::Border* variants; sibling PropertyKey variants
        // carry no per-variant docs per crate convention).
        BorderTopWidth,
        BorderRightWidth,
        BorderBottomWidth,
        BorderLeftWidth,
        BorderTopStyle,
        BorderRightStyle,
        BorderBottomStyle,
        BorderLeftStyle,
        BorderTopColor,
        BorderRightColor,
        BorderBottomColor,
        BorderLeftColor,
        Border,
        // `border-right` single-side shorthand key (semantics on the matching
        // PropertyValue::BorderRight variant).
        BorderRight,
        // `border-style` / `border-width` / `border-color` shorthand keys
        // (semantics on the matching PropertyValue variants above).
        BorderStyle,
        BorderWidth,
        BorderColor,
        // width (CSS Sizing 3 §3.1.1).
        Width,
        // height (CSS Sizing 3 §3.1.1, semantics on the
        // matching PropertyValue::Height variant; sibling PropertyKey variants
        // carry no per-variant docs per crate convention).
        Height,
        MaxWidth,
        MaxHeight,
        MinWidth,
        MinHeight,
        // box-sizing (CSS Sizing 3 §3.3, semantics on the
        // matching PropertyValue::BoxSizing variant; sibling PropertyKey variants
        // carry no per-variant docs per crate convention).
        BoxSizing,
        // direction (CSS Writing Modes 4 §2.1, semantics on
        // the matching PropertyValue::Direction variant; sibling PropertyKey
        // variants carry no per-variant docs per crate convention).
        // See the PropertyValue::Direction docs for why this key is appended.
        Direction,
        // overflow-x / overflow-y longhand + overflow shorthand
        // (CSS Overflow 3 §3.1, semantics on the matching PropertyValue::Overflow*
        // variants; sibling PropertyKey variants carry no per-variant docs per
        // crate convention).
        // See the PropertyValue::OverflowX docs for why this key is appended.
        OverflowX,
        OverflowY,
        Overflow,
        // text-decoration-line / -style / -color longhand + text-decoration
        // shorthand (CSS Text Decoration Module Level 3 §2.1-§2.4, semantics on
        // the matching PropertyValue::TextDecoration* variants; sibling
        // PropertyKey variants carry no per-variant docs per crate convention).
        // See the PropertyValue::TextDecorationLine docs for why this key is appended.
        TextDecorationLine,
        TextDecorationStyle,
        TextDecorationColor,
        TextDecoration,
        // vertical-align (CSS 2.1 §10.8.1, semantics on the matching
        // PropertyValue::VerticalAlign variant; sibling PropertyKey variants
        // carry no per-variant docs per crate convention). Appended for the same
        // reason given in the PropertyValue::TextDecoration docs: a new field
        // disjoint one-to-one from existing fields.
        VerticalAlign,
        // font-style (CSS Fonts 4 §2.4, semantics on the matching
        // PropertyValue::FontStyle variant; sibling PropertyKey variants carry
        // no per-variant docs per crate convention).
        // See the PropertyValue::FontStyle docs for why this key is appended.
        FontStyle,
        // text-transform (CSS Text 4, semantics on the matching
        // PropertyValue::TextTransform variant; sibling PropertyKey
        // variants carry no per-variant docs per crate convention).
        // See the PropertyValue::TextTransform docs for why this key is appended.
        TextTransform,
        // visibility (CSS Display 3 §4, semantics on the matching
        // PropertyValue::Visibility variant; sibling PropertyKey variants carry
        // no per-variant docs per crate convention).
        // See the PropertyValue::Visibility docs for why this key is appended.
        Visibility,
        // z-index (CSS2 §9.9.1, semantics on the matching PropertyValue::ZIndex
        // variant; sibling PropertyKey variants carry no per-variant docs per
        // crate convention).
        // See the PropertyValue::FontStyle docs for why this key is appended.
        ZIndex,
        // word-break (CSS Text 3 §5.1, semantics on the matching
        // PropertyValue::WordBreak variant; sibling PropertyKey variants carry
        // no per-variant docs per crate convention).
        // See the PropertyValue::WordBreak docs for why this key is appended.
        WordBreak,
        // overflow-wrap / legacy alias word-wrap (CSS Text 3 §5.4, semantics on
        // the matching PropertyValue::OverflowWrap variant; sibling PropertyKey
        // variants carry no per-variant docs per crate convention).
        // See the PropertyValue::OverflowWrap docs for why this key is appended.
        OverflowWrap,
        // letter-spacing / word-spacing (CSS Text 3 §7.2 / §7.1, semantics on
        // the matching PropertyValue::LetterSpacing / PropertyValue::WordSpacing
        // variants; sibling PropertyKey variants carry no per-variant docs per
        // crate convention).
        // See the PropertyValue::FontStyle docs for why this key is appended.
        LetterSpacing,
        WordSpacing,
        // break-before / break-after / break-inside + legacy shorthand
        // page-break-* (CSS Fragmentation Module Level 3 §3.1 / §3.2 / §3.4,
        // semantics on the matching PropertyValue::BreakBefore /
        // PropertyValue::BreakAfter / PropertyValue::BreakInside variants;
        // sibling PropertyKey variants carry no per-variant docs per crate
        // convention).
        // See the PropertyValue::BreakBefore docs for why this key is appended.
        BreakBefore,
        BreakAfter,
        BreakInside,
        // float (CSS2 §9.5.1, semantics on the matching PropertyValue::Float
        // variant; sibling PropertyKey variants carry no per-variant docs per
        // crate convention).
        // See the PropertyValue::Float docs for why this key is appended.
        Float,
        // clear (CSS2 §9.5.2, semantics on the matching PropertyValue::Clear
        // variant; sibling PropertyKey variants carry no per-variant docs per
        // crate convention).
        // See the PropertyValue::Clear docs for why this key is appended.
        Clear,
        // white-space (CSS Text 3 §3, semantics on the matching
        // PropertyValue::WhiteSpace variant; sibling PropertyKey variants carry
        // no per-variant docs per crate convention).
        // See the PropertyValue::WhiteSpace docs for why this key is appended.
        WhiteSpace,
        // text-wrap/text-wrap-mode key; the shorthand expands to both longhands.
        TextWrap,
        // flex-* container/item longhands + `flex` shorthand (CSS Flexible Box
        // Layout Module Level 1 §5.1/§5.2/§7.2.1/§7.2.2/§7.2.3/§7.1, semantics
        // on the matching PropertyValue::Flex* variants; sibling PropertyKey
        // variants carry no per-variant docs per crate convention). Shorthand
        // key (`Flex`) placed after its 3 longhands, same convention as
        // `Margin`/`Padding`/`Border`.
        FlexDirection,
        FlexWrap,
        FlexGrow,
        FlexShrink,
        FlexBasis,
        Flex,
        FlexFlow,
        Order,
        // justify-content / align-content (CSS Box Alignment Module Level 3
        // §5.1) / align-items (§7.2) / align-self (§6.2), semantics on the
        // matching PropertyValue::* variants; sibling PropertyKey variants
        // carry no per-variant docs per crate convention.
        JustifyContent,
        AlignContent,
        AlignItems,
        AlignSelf,
        // row-gap / column-gap longhands (CSS Box Alignment Module Level 3
        // §8.1) + `gap` shorthand (§8.2). Shorthand key (`Gap`) placed after
        // its 2 longhands, same convention as `Margin`/`Padding`/`Border`.
        RowGap,
        ColumnGap,
        Gap,
        // `place-content` shorthand (CSS Box Alignment Module Level 3 §5.2) —
        // both longhands (`AlignContent`/`JustifyContent`) declared above.
        PlaceContent,
        // `hyphens` (CSS Text Module Level 3 §5.3), semantics on the matching
        // PropertyValue::Hyphens variant; sibling PropertyKey variants carry no
        // per-variant docs per crate convention.
        Hyphens,
        // tab-size (CSS Text Module Level 3 §4.2, semantics on the matching
        // PropertyValue::TabSize variant; sibling PropertyKey variants carry no
        // per-variant docs per crate convention).
        // See the PropertyValue::TabSize docs for why this key is appended.
        TabSize,
        // font-variant-caps (CSS Fonts Module Level 3 §6.6, semantics on the
        // matching PropertyValue::FontVariantCaps variant; sibling PropertyKey
        // variants carry no per-variant docs per crate convention).
        // See the PropertyValue::FontVariantCaps docs for why this key is appended.
        FontVariantCaps,
        // quotes (CSS Content Module Level 3 §2.4.1, semantics on the matching
        // PropertyValue::Quotes variant; sibling PropertyKey variants carry no
        // per-variant docs per crate convention). For the reason this key is
        // appended, see the PropertyValue::Quotes docs (a new disjoint field).
        Quotes,
        // text-shadow (CSS Text Decoration Module Level 3 §4, semantics on
        // the matching PropertyValue::TextShadow variant; sibling PropertyKey
        // variants carry no per-variant docs per crate convention).
        // See the PropertyValue::TextShadow docs for why this key is appended.
        TextShadow,
        // grid-template-columns / grid-template-rows / grid-template-areas
        // (CSS Grid Layout Module Level 1 §7.2 / §7.3, semantics on the matching
        // PropertyValue::GridTemplate* variants; sibling PropertyKey variants
        // carry no per-variant docs per crate convention).
        GridTemplateColumns,
        GridTemplateRows,
        GridTemplateAreas,
        // grid-auto-columns / grid-auto-rows / grid-auto-flow (CSS Grid Layout
        // Module Level 1 §7.6 / §7.7).
        GridAutoColumns,
        GridAutoRows,
        GridAutoFlow,
        // grid-row-start / grid-row-end / grid-column-start / grid-column-end
        // longhands (CSS Grid Layout Module Level 1 §8.3) + grid-row /
        // grid-column shorthands (§8.4). Shorthand keys (`GridRow`/`GridColumn`)
        // placed after their 2 longhands each, same convention as
        // `Margin`/`Padding`/`Border`.
        GridRowStart,
        GridRowEnd,
        GridRow,
        GridColumnStart,
        GridColumnEnd,
        GridColumn,
        // justify-items (CSS Box Alignment Module Level 3 §7.1) / justify-self
        // (§6.1), semantics on the matching PropertyValue::* variants.
        JustifyItems,
        JustifySelf,
        // `place-items` shorthand (§7.3) — both longhands (`AlignItems`/
        // `JustifyItems`) are declared above; AlignItems is in the existing
        // flex/alignment section. `place-self` shorthand (§6.3) — both longhands
        // (`AlignSelf`/`JustifySelf`) are declared above.
        PlaceItems,
        PlaceSelf,
        // orphans / widows (CSS Fragmentation Module Level 3 §3.3, semantics on
        // the matching PropertyValue::Orphans / PropertyValue::Widows variants;
        // sibling PropertyKey variants carry no per-variant docs per crate
        // convention).
        Orphans,
        Widows,
        /// Internal sentinel for a custom property. Custom properties are
        /// selected by their case-sensitive name, not by this key.
        Custom,
        // border-radius / box-shadow / outline (CSS Backgrounds and Borders 3 §5
        // / §6.1 and CSS UI 3 §4; semantics on matching PropertyValue variants).
        BorderRadius,
        BorderRadiusTopLeft,
        BorderRadiusTopRight,
        BorderRadiusBottomRight,
        BorderRadiusBottomLeft,
        BoxShadow,
        Outline,
        OutlineWidth,
        OutlineStyle,
        OutlineColor,
        OutlineOffset,
        // writing-mode (CSS Writing Modes 4 §3.2, semantics on the matching
        // PropertyValue::WritingMode variant; sibling PropertyKey variants carry
        // no per-variant docs per crate convention).
        // See the PropertyValue::WritingMode docs for why this key is appended.
        WritingMode,
        /// `ruby-position` inherited annotation placement.
        RubyPosition,
        // background-repeat / background-attachment / background-clip /
        // background-origin / background-size / background-position (CSS
        // Backgrounds and Borders 3 §2.4-§2.9, semantics on the matching
        // PropertyValue::Background* variants; sibling PropertyKey variants
        // carry no per-variant docs per crate convention). For the reason these
        // keys are appended, see the preceding writing-mode section: each has a
        // new field disjoint one-to-one from existing fields.
        BackgroundRepeat,
        BackgroundAttachment,
        BackgroundClip,
        BackgroundOrigin,
        BackgroundSize,
        BackgroundPosition,
        // background-image (CSS Backgrounds and Borders 3 §2.3, semantics on the
        // matching PropertyValue::BackgroundImage variant; sibling PropertyKey
        // variants carry no per-variant docs per crate convention). Appended for
        // the same reason as the preceding background-repeat group: a new field
        // disjoint one-to-one from existing fields.
        BackgroundImage,
        // `background` shorthand (CSS Backgrounds and Borders 3 §2.10, semantics
        // on the matching PropertyValue::Background variant). For the reason this
        // key is appended, see the PropertyValue::Background docs: shorthands do
        // not reach the cascade stage, so discriminant order does not matter.
        Background,
        // object-fit / object-position (CSS Images Module Level 3 §5.1/§5.2,
        // semantics on the matching PropertyValue::ObjectFit /
        // PropertyValue::ObjectPosition variants; sibling PropertyKey variants
        // carry no per-variant docs per crate convention). Appended for the same
        // reason as background-repeat: new fields disjoint one-to-one from
        // existing fields.
        ObjectFit,
        ObjectPosition,
        // opacity (CSS Color 4 §3.3, semantics on the matching
        // PropertyValue::Opacity variant; sibling PropertyKey variants carry no
        // per-variant docs per crate convention). Appended for the same reason as
        // background-repeat: a new field disjoint one-to-one from existing fields.
        Opacity,
        // mix-blend-mode (CSS Compositing and Blending Level 1 §3.4.1,
        // semantics on the matching PropertyValue::MixBlendMode variant; sibling
        // PropertyKey variants carry no per-variant docs per crate convention).
        // Appended for the same reason as background-repeat: a new field
        // disjoint one-to-one from existing fields.
        MixBlendMode,
        // mask-image / clip-path (CSS Masking Level 1 §7.1/§5.1, semantics on
        // the matching PropertyValue::MaskImage / PropertyValue::ClipPath
        // variants; sibling PropertyKey variants carry no per-variant docs per
        // crate convention). Appended for the same reason as background-repeat:
        // new fields disjoint one-to-one from existing fields.
        MaskImage,
        ClipPath,
        // transform / filter (CSS Transforms Level 1 §4, CSS Filter Effects
        // Level 1 §5, semantics on the matching PropertyValue::Transform /
        // PropertyValue::Filter variants; sibling PropertyKey variants carry
        // no per-variant docs per crate convention). Appended for the same reason
        // as background-repeat: new fields disjoint one-to-one from existing fields.
        Transform,
        TransformOrigin,
        Filter,
        // line-break (CSS Text 3 §5.2, semantics on PropertyValue::LineBreak).
        LineBreak,
        // text-justify (CSS Text 3 §6.2, semantics on PropertyValue::TextJustify).
        TextJustify,
        // text-align-all / text-align-last (CSS Text 3 §6.1 longhands)
        TextAlignAll,
        TextAlignLast,
        // text-combine-upright (CSS Writing Modes 3 §9.1)
        TextCombineUpright,
        // text-orientation (CSS Writing Modes 3 §5.1)
        TextOrientation,
        // unicode-bidi (CSS Writing Modes 3 §2.2)
        UnicodeBidi,
        // table-layout (CSS Tables 3 §4, semantics on the matching
        // PropertyValue::TableLayout variant; sibling PropertyKey variants carry
        // no per-variant docs per crate convention). Appended for the same reason
        // as background-repeat: a new field disjoint one-to-one from existing fields.
        TableLayout,
        // border-collapse (CSS Tables 3 §6, semantics on the matching
        // PropertyValue::BorderCollapse variant; sibling PropertyKey variants
        // carry no per-variant docs per crate convention). Appended for the same
        // reason as background-repeat: a new field disjoint one-to-one from
        // existing fields.
        BorderCollapse,
        // border-spacing (CSS Tables 3 §6.1, semantics on the matching
        // PropertyValue::BorderSpacing variant; sibling PropertyKey variants
        // carry no per-variant docs per crate convention). Appended for the same
        // reason as background-repeat: a new field disjoint one-to-one from
        // existing fields.
        BorderSpacing,
        // caption-side (CSS Tables 3 §7, semantics on the matching
        // PropertyValue::CaptionSide variant; sibling PropertyKey variants
        // carry no per-variant docs per crate convention). Appended for the same
        // reason as background-repeat: a new field disjoint one-to-one from
        // existing fields.
        CaptionSide,
        // empty-cells (CSS Tables 3 §8, semantics on the matching
        // PropertyValue::EmptyCells variant; sibling PropertyKey variants
        // carry no per-variant docs per crate convention). Appended for the same
        // reason as background-repeat: a new field disjoint one-to-one from
        // existing fields.
        EmptyCells,
        // font shorthand (CSS Fonts 4 §2.1, semantics on the matching
        // PropertyValue::Font variant; sibling PropertyKey variants carry no
        // per-variant docs per crate convention). For this appended placement, see the background-repeat
        // section above. Since `crate::rule::expand_shorthand_into` expands this
        // shorthand before the cascade stage, discriminant order does not matter.
        Font,
        // text-decoration-skip-ink (ED §2.10.4, semantics on the matching
        // PropertyValue::TextDecorationSkipInk variant; sibling PropertyKey
        // variants carry no per-variant docs per crate convention). Appended for
        // the same reason as background-repeat: a new disjoint field; element
        // computed state is stored in `ComputedValues::text_decoration_skip_ink`.
        TextDecorationSkipInk,
        // text-decoration-skip-spaces (ED §2.10.3, same convention and placement rationale as above).
        TextDecorationSkipSpaces,
        // text-decoration-thickness (ED §2.4.1, TR §2.4, same convention and placement rationale as above).
        TextDecorationThickness,
        // text-decoration-inset (ED §2.9.1, same convention and placement rationale as above).
        TextDecorationInset,
        // text-emphasis-position (ED §3.4, same convention and placement rationale as above).
        TextEmphasisPosition,
        // text-underline-position (ED §2.7, same convention and placement rationale as above).
        TextUnderlinePosition,
        // page (CSS Paged Media 3 §8.1, semantics on the matching
        // PropertyValue::Page variant; sibling PropertyKey variants carry no
        // per-variant docs per crate convention). Appended for the same reason as
        // background-repeat. This key is parsing-only and has no staging field,
        // but its discriminant can likewise be placed freely.
        Page,
        // CSS Lists 3 §3 list-style longhands. Appended to preserve the
        // discriminants of existing keys used by the winner scratch slots.
        ListStyleType,
        ListStylePosition,
        ListStyleImage,
        // CSS Multi-column Layout Module Level 1.
        ColumnCount,
        ColumnWidth,
        Columns,
        // Logical minimum block size; appended to preserve existing key slots.
        MinBlockSize,
        // CSS Text Decoration 4 §2.8; appended to preserve existing key slots.
        TextUnderlineOffset,
        // CSS Text 3 §8.2.1; appended to preserve existing key slots.
        HangingPunctuation,
        // CSS Text 4 text-autospace; appended to preserve existing key slots.
        TextAutospace,
        // CSS Text 4 white-space-collapse; appended to preserve existing key slots.
        WhiteSpaceCollapse,
        // CSS Text 4 text-wrap-style; appended to preserve existing key slots.
        TextWrapStyle,
        // CSS Text 4 hyphenate-character; appended to preserve existing key slots.
        HyphenateCharacter,
        // CSS Text 4 hyphenate-limit-chars; appended to preserve existing key slots.
        HyphenateLimitChars,
        // CSS Text 4 text-spacing-trim; appended to preserve existing key slots.
        TextSpacingTrim,
        // CSS Text 4 text-spacing shorthand; appended to preserve existing key slots.
        TextSpacing,
        // CSS Text 4 word-space-transform; appended to preserve existing key slots.
        WordSpaceTransform,
        // CSS Text Decoration 4 text-emphasis-style; appended to preserve key slots.
        TextEmphasisStyle,
        // CSS Text Decoration 4 text-emphasis-color; appended to preserve key slots.
        TextEmphasisColor,
        // CSS Text Decoration 3 text-emphasis shorthand; appended to preserve key slots.
        TextEmphasis,
        // CSS Fonts 3 font-kerning; appended to preserve existing key slots.
        FontKerning,
        // CSS Fonts 4 font-optical-sizing; appended to preserve existing key slots.
        FontOpticalSizing,
        // CSS Fonts 4 font-variant-emoji; appended to preserve existing key slots.
        FontVariantEmoji,
        // CSS Fonts 4 font-language-override; appended to preserve existing key slots.
        FontLanguageOverride,
        // CSS Fonts 4 font-variant-ligatures; appended to preserve existing key slots.
        FontVariantLigatures,
        // CSS Fonts 4 font-synthesis; appended to preserve existing key slots.
        FontSynthesis,
        // CSS Fonts 3 font-variant-position; appended to preserve existing key slots.
        FontVariantPosition,
        // CSS Fonts 4 font-palette; appended to preserve existing key slots.
        FontPalette,
        // CSS Fonts 3 font-variant-numeric; appended to preserve existing key slots.
        FontVariantNumeric,
        // CSS Fonts 3 font-variant-east-asian; appended to preserve existing key slots.
        FontVariantEastAsian,
        // CSS Fonts 4 font-variation-settings; appended to preserve existing key slots.
        FontVariationSettings,
    }

    properties! {
        /// CSS Compositing and Blending Level 1 §3.4.2 "Isolation: the isolation
        /// property" <https://www.w3.org/TR/compositing-1/#isolation>. Grammar:
        /// `auto | isolate`. **Non-inherited**, initial `auto`. The computed value
        /// is the specified keyword (no length payload).
        ///
        /// The spec gives detailed conditions for whether `isolation` creates a
        /// stacking context / group (element types, SVG containers, etc.), but
        /// this crate does not implement that application algorithm: it stores
        /// only the cascaded/computed keyword. Creating compositing groups
        /// belongs to raikiri-paint.
        "isolation" => Isolation {
            keywords: [
                /// `auto` — the spec's initial value. The element itself does not
                /// force an independent stacking context / group.
                Auto,
                /// `isolate` — make the element an independent stacking context and
                /// confine `mix-blend-mode` blending to its subtree.
                Isolate,
            ],
            initial: Auto,
            inherited: no,
        },
    }
}
pub use decl::*;
