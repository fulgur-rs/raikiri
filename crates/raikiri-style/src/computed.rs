//! Per-node computed CSS values.
//!
//! Populated by the cascade and inheritance walk. A future layer will extract
//! `taffy::Style` and paint color information from `ComputedValues`.
//!
//! For supported properties and their inheritance behavior, see the field
//! documentation on [`ComputedValues`].

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use smol_str::SmolStr;

use crate::property::{
    AlignSelfValue, BackgroundAttachment, BackgroundImage, BackgroundRepeat,
    BackgroundRepeatKeyword, BorderCollapseValue, BorderColor, BorderStyle, BoxSizing,
    BreakBetween, BreakInside, CaptionSideValue, ClearValue, ClipPath, ColumnCountValue,
    ColumnFillValue, ContentAlignmentValue, ContentComponent, CssColor, Direction, DisplayValue,
    EmptyCellsValue, FilterFunction, FlexDirectionValue, FlexWrapValue, FloatValue, FontFamilyName,
    FontFeatureSettings, FontKerning, FontLanguageOverride, FontOpticalSizing, FontPaletteValue,
    FontStyle, FontSynthesisValue, FontVariantCaps, FontVariantEastAsian, FontVariantEmoji,
    FontVariantLigatures, FontVariantNumeric, FontVariantPosition, FontVariationSettings,
    GridAutoFlowValue, GridLineValue, GridTemplateAreasValue, HangingPunctuation,
    HyphenateCharacter, HyphenateLimitChars, Hyphens, Isolation, LineBreak, ListStylePosition,
    ListStyleType, MaskImage, MixBlendMode, ObjectFit, OutlineColor, OutlineStyle, OverflowValue,
    OverflowWrap, OverflowXY, PositionValue, RubyPosition, SelfAlignmentValue, Sides,
    TableLayoutValue, TextAlign, TextAlignLast, TextAutospace, TextCombineUpright,
    TextDecorationColor, TextDecorationLine, TextDecorationSkipInk, TextDecorationSkipSpaces,
    TextDecorationStyle, TextEmphasisHEdge, TextEmphasisPosition, TextEmphasisStyle,
    TextEmphasisVEdge, TextJustify, TextOrientation, TextOverflowValue, TextSpacingTrim,
    TextTransform, TextUnderlinePosition, TextWrapMode, TextWrapStyle, UnicodeBidi, VerticalAlign,
    Visibility, VisualBox, WhiteSpace, WhiteSpaceCollapse, WordBreak, WordSpaceTransform,
    WritingMode, ZIndexValue, empty_content_list, empty_counter_entries, empty_filter_list,
    empty_quotes_entries, empty_string_set_entries, initial_font_family,
};
use crate::resolve::{
    ComputedBackgroundSize, ComputedBorder, ComputedBorderRadius, ComputedBorderSpacing,
    ComputedBoxShadowItem, ComputedColumnWidth, ComputedCssPosition, ComputedCssPositionOffset,
    ComputedFlexBasis, ComputedGridTemplateTracks, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ComputedLetterSpacing, ComputedLineHeight, ComputedOutline, ComputedTabSize,
    ComputedTextDecorationInset, ComputedTextDecorationThickness, ComputedTextIndent,
    ComputedTextShadow, ComputedTextUnderlineOffset, ComputedTransformFunction,
    ComputedWordSpacing, empty_computed_box_shadow_list, empty_computed_text_shadow_list,
    empty_computed_transform_list, initial_computed_grid_auto_track_list,
};

/// Pixel value corresponding to the CSS initial `font-size` value (`medium`).
///
/// CSS Fonts 4 §2.5, "Font size: the font-size property"
/// (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>), specifies "Initial:
/// medium" but leaves the actual pixel size of `medium` to the user agent.
/// This implementation uses the browser default of 16px.
///
pub(crate) const INITIAL_FONT_SIZE_PX: f32 = 16.0;

/// Persistent custom-property environment used by computed values.
///
/// CSS Variables 1 §2 makes custom properties inherited. Each environment
/// therefore stores only the declarations resolved at one element and points
/// at the parent's environment. Cloning an environment is an `Arc` bump;
/// adding a declaration allocates only the local delta instead of copying the
/// complete inherited map.
#[derive(Clone)]
pub(crate) struct CustomPropertyEnvironment {
    parent: Option<Arc<CustomPropertyEnvironment>>,
    /// `Some(value)` is a resolved declaration. `None` is a local
    /// guaranteed-invalid tombstone: it must hide an inherited value while
    /// allowing `var()` fallback at this element.
    local: HashMap<SmolStr, Option<SmolStr>>,
}

impl CustomPropertyEnvironment {
    /// Create an environment containing one element's local resolved values.
    pub(crate) fn from_local(
        parent: &Arc<CustomPropertyEnvironment>,
        local: HashMap<SmolStr, Option<SmolStr>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            parent: Some(parent.clone()),
            local,
        })
    }

    /// Create a root environment from an already resolved flat map.
    #[cfg(test)]
    pub(crate) fn from_map(values: HashMap<SmolStr, SmolStr>) -> Arc<Self> {
        let local = values
            .into_iter()
            .map(|(name, value)| (name, Some(value)))
            .collect();
        Self::from_local(&empty_custom_properties(), local)
    }

    /// Look up the effective value, walking shared ancestor environments.
    pub(crate) fn get(&self, name: &str) -> Option<SmolStr> {
        let mut current = Some(self);
        while let Some(environment) = current {
            if let Some(value) = environment.local.get(name) {
                return value.clone();
            }
            current = environment.parent.as_deref();
        }
        None
    }

    /// Look up a value declared on this node, without walking inherited
    /// environments.  This is used by the consumer-property boundary, whose
    /// registrations are local by default.
    pub(crate) fn get_local(&self, name: &str) -> Option<SmolStr> {
        self.local.get(name).cloned().flatten()
    }

    /// Number of entries owned by this environment, for bounded regression
    /// tests and diagnostics.
    #[cfg(test)]
    pub(crate) fn local_entry_count(&self) -> usize {
        self.local.len()
    }

    /// Parent environment, if this is not the shared empty root.
    #[cfg(test)]
    pub(crate) fn parent_environment(&self) -> Option<&Arc<Self>> {
        self.parent.as_ref()
    }
}

impl Drop for CustomPropertyEnvironment {
    fn drop(&mut self) {
        // A linked environment normally drops its parent recursively when the
        // parent Arc is unique. Detach and consume a unique chain iteratively
        // so a hostile deep element tree cannot exhaust the native stack while
        // its computed values are torn down.
        let mut parent = self.parent.take();
        while let Some(parent_arc) = parent {
            match Arc::try_unwrap(parent_arc) {
                Ok(mut parent_environment) => {
                    parent = parent_environment.parent.take();
                }
                Err(_) => break,
            }
        }
    }
}

impl fmt::Debug for CustomPropertyEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CustomPropertyEnvironment")
            .field("local", &self.local)
            // Do not recursively format the parent chain: a deep document's
            // debug representation must remain bounded by this environment.
            .field("has_parent", &self.parent.is_some())
            .finish()
    }
}

impl PartialEq for CustomPropertyEnvironment {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other) || self.effective_bindings() == other.effective_bindings()
    }
}

impl Eq for CustomPropertyEnvironment {}

impl CustomPropertyEnvironment {
    fn effective_bindings(&self) -> HashMap<SmolStr, SmolStr> {
        let mut environments = Vec::new();
        let mut current = Some(self);
        while let Some(environment) = current {
            environments.push(environment);
            current = environment.parent.as_deref();
        }

        let mut values = HashMap::new();
        for environment in environments.into_iter().rev() {
            for (name, value) in &environment.local {
                match value {
                    Some(value) => {
                        values.insert(name.clone(), value.clone());
                    }
                    None => {
                        values.remove(name);
                    }
                }
            }
        }
        values
    }
}

/// Shared empty custom-property environment for computed values that have no
/// local custom-property declarations. The environment is crate-private
/// bookkeeping for the `@page` cascade; it is not part of the public
/// computed-style API.
pub(crate) fn empty_custom_properties() -> Arc<CustomPropertyEnvironment> {
    static EMPTY: OnceLock<Arc<CustomPropertyEnvironment>> = OnceLock::new();
    EMPTY
        .get_or_init(|| {
            Arc::new(CustomPropertyEnvironment {
                parent: None,
                local: HashMap::new(),
            })
        })
        .clone()
}

/// Cascade-time seed for a template registered by `position: running(<custom-ident>)`.
///
/// CSS GCPM 3 §1.2.1, "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: an element with
/// `position: running(name)` is removed from the body flow and can be referenced
/// by `content: element(name)` in an `@page` margin box.
///
/// This struct is the static-side seed for the **two-tier cache** in design doc
/// §7.3. The cascade captures `name` per node; downstream `raikiri-dom` then
/// associates the subtree root, pre-cascaded style, and dynamic flags
/// (`ParsedRunningTemplate`). This crate is a leaf and has no DOM node identity.
///
/// `#[non_exhaustive]` allows future fields, such as a per-name override of
/// `alternative_hint`, to be added without breaking callers.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningTemplate {
    /// The case-preserved smol-str name in `running(<name>)`.
    pub name: SmolStr,
}

/// Font matching data captured when an authored `ch` value is computed.
///
/// Inherited values retain the ancestor's key, so a descendant with a different
/// font does not re-measure the inherited value against the wrong glyph
/// advance. Non-inherited box values use the declaring node's own key. The
/// key currently covers family, size, weight, and style; variation axes and
/// vertical text orientation remain outside this layout's horizontal metric
/// scope.
#[derive(Clone, Debug, PartialEq)]
pub struct ChFontKey {
    /// Font family list used by the shaping resolver.
    pub family: Arc<Vec<FontFamilyName>>,
    /// Computed font size in CSS px.
    pub size: ComputedLength,
    /// Computed numeric font weight.
    pub weight: f32,
    /// Computed font style.
    pub style: FontStyle,
}

/// Physical preferred sizes of a `vertical-rl` / `vertical-lr` box whose
/// `inline-size` or `block-size` was declared; see
/// [`ComputedValues::vertical_logical_size`].
#[derive(Clone, Debug, PartialEq)]
pub struct VerticalLogicalSize {
    /// Physical width: the block size, or the authored `width`.
    pub width: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance of [`Self::width`].
    pub width_ch: Option<ChLengthProvenance>,
    /// Physical height: the inline size, or the authored `height`.
    pub height: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance of [`Self::height`].
    pub height_ch: Option<ChLengthProvenance>,
}

/// Authored `ch` provenance retained until the layout sink can probe the
/// declaring font's U+0030 advance.
#[derive(Clone, Debug, PartialEq)]
pub struct ChLengthProvenance {
    /// Authored multiplier, for example `2` in `width: 2ch`.
    pub factor: f32,
    /// Font matching data from the declaring node for this value; inherited
    /// values keep the ancestor's key.
    pub font: ChFontKey,
}

/// Per-node computed style. See the field documentation below for supported
/// properties and their inheritance behavior (inherited: color / font-family / font-size / font-weight / text_align / hanging_punctuation / direction / writing_mode / cssom_writing_mode / line_height / font_style / font_kerning / font_optical_sizing / font_variant_emoji / font_language_override / font_variant_ligatures / font_synthesis / font_variant_position / font_palette / font_variant_numeric / font_variant_east_asian / font_variation_settings / font_feature_settings / font_variant_caps / text_transform / text_combine_upright / text_orientation / visibility / text_indent / word_break / overflow_wrap / letter_spacing / word_spacing / white_space / white_space_collapse / text_wrap_style / hyphens / hyphenate_character / hyphenate_limit_chars / tab_size / quotes / text_shadow / orphans / widows / list_style_type / list_style_position;
/// non-inherited: background-color / display / counter-* / content / string-set /
/// running_templates / padding / margin / border / border_radius / box_shadow / outline / width / height / box_sizing /
/// overflow / text_decoration / unicode_bidi / vertical_align / z_index / float / clear).
///
/// # Layers
///
/// This struct stores only **computed values**. Fields carrying lengths use
/// `Computed*` types from [`crate::resolve`]; `em`, `rem`, and `pt` have already
/// been converted to absolute px. CSS Cascade 5 §7.2
/// <https://www.w3.org/TR/css-cascade-5/#inheriting> specifies that inheritance
/// carries computed values, so conversion must precede inheritance. Treatment
/// of `<percentage>` varies by property; see each field's documentation.
///
/// The staging representation before conversion is
/// [`crate::specified::SpecifiedValues`]. Cascade winners are applied there;
/// [`SpecifiedValues::finalize`] produces this struct.
///
/// [`SpecifiedValues::finalize`]: crate::specified::SpecifiedValues::finalize
///
/// `#[non_exhaustive]` allows new properties without breaking callers.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedValues {
    /// `color`. Inherited; initial: opaque black.
    pub color: CssColor,
    /// `background-color`. **Not inherited**; initial: `transparent`
    /// (= [`CssColor::TRANSPARENT`]). CSS Backgrounds 3 §2.2, "Base Color:
    /// the background-color property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#background-color>.
    pub background_color: CssColor,
    /// Symbolic background color retained for explicit inheritance.
    pub(crate) background_color_expression: Option<SmolStr>,
    /// `font-family`, in priority order. Inherited. CSS Fonts 4 §2.1
    /// <https://www.w3.org/TR/css-fonts-4/#font-family-prop> specifies an initial
    /// value of "depends on user agent" rather than a particular family name
    /// (see [`crate::property::initial_font_family`]). This implementation uses
    /// the generic family `[FontFamilyName::generic("serif")]`.
    ///
    pub font_family: Arc<Vec<FontFamilyName>>,
    /// `font-size`. **Inherited**; initial: 16px (the spec says `medium`, whose
    /// actual pixel size depends on the user agent). CSS Fonts 4 §2.5, "Font
    /// size: the font-size property"
    /// <https://www.w3.org/TR/css-fonts-4/#propdef-font-size> specifies
    /// "Computed value: an absolute length".
    ///
    /// Its [`ComputedLength`] type contains absolute px: `em` (relative to the
    /// parent), `rem` (relative to the root), `<percentage>` (relative to the
    /// parent), and `pt` are converted in cascade phase 2 by
    /// [`crate::resolve::resolve_font_size`].
    pub font_size: ComputedLength,
    /// `font-weight`. Inherited; initial: 400.0 (normal).
    ///
    /// **Always a resolved absolute weight** in `[1, 1000]`. The specified-value
    /// `bolder` / `lighter` sentinels ([`crate::property::FontWeightValue`]) are
    /// resolved by [`crate::cascade::apply_value`] against the parent's computed
    /// weight and the CSS Fonts 4 §2.2 table before this field is written. No
    /// relative keyword remains here. The §2.2 property table specifies
    /// "Computed value: a number, see below"; §2.2.1, "Relative Weights", says
    /// "Specified values of `bolder` and `lighter` indicate weights relative
    /// to the weight of the parent element. The computed weight is calculated
    /// based on the inherited `font-weight` value"
    /// (<https://www.w3.org/TR/css-fonts-4/#relative-weights>).
    ///
    /// The `f32` type (formerly `u16`) preserves fractional computed weights,
    /// as required by §2.2.2, "Missing weights"
    /// <https://www.w3.org/TR/css-fonts-4/#missing-weights>: "Fractional weights
    /// are valid". The former `u16` representation required round-half-away-
    /// from-zero conversion, which could also change the chosen row of the
    /// §2.2.1 relative-weight table. Changing this field's type removed that
    /// known divergence.
    ///
    /// The `raikiri-dom` layout passes this field directly to the inline
    /// engine's font query without a cast.
    ///
    /// # Caller contract: finite and within `[1, 1000]` (no field-level guard)
    ///
    /// Every field of this struct is `pub`, so callers can construct
    /// `ComputedValues { font_weight: ..., .. }` without going through the
    /// cascade (for example, the inherited root argument to
    /// [`crate::page::cascade_page`]). The old `u16` type could not represent
    /// non-finite values. With `f32`, validation shifts from the type to the
    /// caller. The normal cascade path is always finite because
    /// `parse_font_weight` (private; see [`crate::property::FontWeightValue`])
    /// enforces `[1, 1000]`, but direct construction bypasses that guard. The
    /// behavior of [`crate::cascade::resolve_relative_weight`] (which resolves
    /// `bolder` / `lighter`) for `NaN` / `±Inf` is documented on that function.
    ///
    /// **Neither this field nor `resolve_relative_weight` adds a guard.** The
    /// design puts guards at sink boundaries rather than sanitizing the public
    /// resolve/computed layer (carve-out 2). Other direct consumers of this
    /// field, including external consumers through the umbrella crate, must
    /// guard their own sink boundaries for the same reason.
    pub font_weight: f32,
    /// `line-height`. **Inherited**; initial: [`ComputedLineHeight::Normal`].
    /// CSS Inline 3 §5.1, "Line Spacing: the line-height property"
    /// <https://www.w3.org/TR/css-inline-3/#line-height-property>.
    ///
    /// The three variants of [`ComputedLineHeight`] directly correspond to the
    /// spec's "Computed value: the specified keyword, a number, or a computed
    /// `<length>` value". **The computed layer contains no percentages**:
    /// `<percentage>` is resolved against the declaring element's computed font
    /// size in cascade phase 3 (§5.1, "Percentages: computed relative to 1em").
    ///
    /// [`ComputedLineHeight::Number`] remains a unitless multiplier in the
    /// computed layer. This matches the §5.1 special behavior "child inherits
    /// the specified value" and leaves multiplication by the child's font size
    /// to downstream paint. By contrast, [`ComputedLineHeight::Length`] is px
    /// fixed at the declaring element, which children **inherit unchanged**.
    pub line_height: ComputedLineHeight,
    /// `display`. **Not inherited**; initial: `DisplayValue::Inline`
    /// (CSS Display 3 §2 <https://www.w3.org/TR/css-display-3/#propdef-display>).
    /// Currently supports 18 keywords (`block` / `inline` / `inline-block` /
    /// `none` / `flex` / `grid` / `list-item` / `contents` / `table` /
    /// `inline-table` / `table-row-group` / `table-header-group` /
    /// `table-footer-group` / `table-row` / `table-column-group` /
    /// `table-column` / `table-cell` / `table-caption`; see [`DisplayValue`]).
    pub display: DisplayValue,
    /// `list-style-type`. **Inherited**; initial: `disc` (CSS Lists 3 §3.1).
    pub list_style_type: ListStyleType,
    /// `list-style-position`. **Inherited**; initial: `outside` (CSS Lists 3 §3.2).
    pub list_style_position: ListStylePosition,
    /// `list-style-image`. **Inherited**; initial: `none` (CSS Lists 3 §3.3).
    pub list_style_image: BackgroundImage,
    /// `counter-reset`. **Not inherited**. The spec's initial value is `none`
    /// (CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#counter-reset>),
    /// represented here by an empty list. Holds counter-name / initial-value
    /// pairs in preparation for resolving the counter tree; actual resolution
    /// will be implemented separately.
    ///
    /// Wrapping in [`Arc<Vec<..>>`] makes cascade winner cloning (`value.clone()`
    /// during `apply_winners` drain; move in `apply_value`) and inheritance-walk
    /// cloning (stack push in `resolve_inheritance`, then
    /// `out[idx] = computed.clone()`) **shallow (Arc reference-count increments)**.
    /// Sharing one heap allocation prevents O(N × M) memory growth for
    /// `* { counter-reset: c0 c1 ... cN }` across M elements, a security-relevant
    /// DoS defense following the Content/StringSet pattern. Because
    /// `Arc<Vec<T>>: Deref<Target = Vec<T>>`, downstream `.iter()`, `.len()`, and
    /// `.is_empty()` calls remain unchanged (no dom/paint consumer changes).
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-increment`. **Not inherited**. The spec's initial value is
    /// `none` (CSS Lists 3 §4.2
    /// <https://www.w3.org/TR/css-lists-3/#increment-set>), represented here
    /// by an empty list. Holds counter-name / increment pairs in preparation
    /// for resolving the counter tree; actual resolution comes later.
    ///
    /// The [`Arc<Vec<..>>`] wrapper has the same rationale as [`Self::counter_reset`].
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-set`. **Not inherited**. The spec's initial value is `none`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#increment-set>),
    /// represented here by an empty list. Holds counter-name / value pairs in
    /// preparation for resolving the counter tree; actual resolution comes later.
    ///
    /// The [`Arc<Vec<..>>`] wrapper has the same rationale as [`Self::counter_reset`].
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// Resolved intermediate representation of `content`. **Not inherited**.
    /// The spec's initial value is `normal` (CSS Content 3 §1 propdef-content,
    /// "Initial: normal"). Here `normal` is an empty list, while explicit `none`
    /// is a [`ContentComponent::None`] sentinel. Pseudo-element consumers can
    /// therefore suppress an explicit `content: none` declaration. This is
    /// groundwork for emitting future GCPM directives. Downstream `raikiri-dom`
    /// maps it to `raikiri_traits::ContentValueItem`: `raikiri-style` is a leaf
    /// crate with no dependency on `raikiri-traits`, following the counter-*
    /// wire-through pattern.
    /// See <https://www.w3.org/TR/css-content-3/#content-property>.
    ///
    /// The [`Arc<Vec<..>>`] wrapper makes cascade winner cloning (`value.clone()`
    /// during `apply_winners` drain; move in `apply_value`) and inheritance-walk
    /// cloning (stack push in `resolve_inheritance`, then
    /// `out[idx] = computed.clone()`) **shallow (Arc reference-count increments)**.
    /// Sharing one heap allocation prevents O(N × M) memory growth for
    /// `* { content: "<large>" }` across N elements, a security-relevant DoS
    /// defense. Because `Arc<Vec<T>>: Deref<Target = Vec<T>>`, downstream
    /// `.iter()`, `.len()`, and `.is_empty()` calls remain unchanged (no
    /// dom/paint consumer changes).
    pub content: Arc<Vec<ContentComponent>>,
    /// Parsed `string-set` entries as `(name, content-list)` pairs.
    /// **Not inherited**. The spec's initial value is `none` (CSS GCPM 3 §1.1.1,
    /// propdef-string-set, "Initial: none"), represented here by an empty list.
    /// Downstream `raikiri-dom` handles name resolution and runtime `string()`
    /// references. See <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>.
    ///
    /// The [`Arc<Vec<..>>`] wrapper has the same rationale as [`Self::content`]:
    /// it prevents a similar DoS path for `* { string-set: name "<large>" }`
    /// across N elements.
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// Seed for `position: running(<custom-ident>)`. **Not inherited**. The
    /// spec's initial `position: static` has no running() seed (CSS GCPM 3
    /// §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>), represented
    /// here by an empty list.
    ///
    /// This field always has **zero or one entry per node**: `position` is a
    /// single-valued property, so an element can have at most one `running(name)`:
    /// - `position: static`, another keyword, or no rule → empty
    /// - `position: running(name)` → `[RunningTemplate { name }]`
    ///
    /// The `Vec` shape follows the SmolStr wire-through pattern used by
    /// `content` / `string_set` (reuse of an established precedent). Downstream
    /// `raikiri-dom` concatenates these per-node seeds when building a
    /// per-document `Vec<RunningTemplate>`. This is the static side of the
    /// two-tier cache described in design doc §7.3.
    pub running_templates: Vec<RunningTemplate>,
    /// `position` — **non-inherited**, initial: `static` (CSS Positioned Layout Module Level 3 §3).
    pub position: PositionValue,
    /// `text-align`. **Inherited**; initial: [`TextAlign::Start`]
    /// (CSS Text 3 §6.1, "Text Alignment: the text-align shorthand"
    /// <https://www.w3.org/TR/css-text-3/#text-align-property>).
    ///
    /// The spec defines a shorthand that sets the `text-align-all` and
    /// `text-align-last` longhands. For now, this uses the **shorthand as one
    /// field** convention (as with margin `Sides<T>` and the empty-list
    /// representation of content `normal` / `none`); splitting the longhands
    /// from §6.2 / §6.3 is deferred. See [`TextAlign`].
    ///
    /// Like sibling fields [`color`](Self::color) /
    /// [`font_family`](Self::font_family) / [`font_size`](Self::font_size) /
    /// [`font_weight`](Self::font_weight), this is **inherited**. It belongs in
    /// the inherited block of `inherit_from` and copies the parent's value
    /// (`TextAlign` is `Copy`).
    ///
    /// **`MatchParent` never appears as this field's value.** Even when the
    /// cascade winner is `match-parent`, before reaching the computed layer
    /// [`crate::property::resolve_text_align_match_parent`] resolves it using
    /// the parent's [`Self::text_align`] and [`Self::direction`] to `Left`,
    /// `Right`, or a copy of the parent's value. Both the element path
    /// ([`crate::specified::SpecifiedValues::finalize`] / `finalize_as_root`)
    /// and page path ([`crate::cascade::resolve_against_inherited`]) use that
    /// same resolver.
    pub text_align: TextAlign,
    /// `hanging-punctuation`. **inherited**, initial `none` (CSS Text 3
    /// §8.2.1). The full keyword set is preserved for the layout consumer.
    pub hanging_punctuation: HangingPunctuation,
    /// `text-autospace` (CSS Text 4). **Inherited**; initial: `normal`.
    /// The keyword/flag set is preserved for the inline text layout consumer.
    pub text_autospace: TextAutospace,
    /// `word-space-transform` (CSS Text 4). **Inherited**, initial `none`;
    /// the inline text layout handles interior U+200B and in-flow inline
    /// `<wbr>` in the `space`/`ideographic-space` single-line subset. Other
    /// values remain data-only.
    pub word_space_transform: WordSpaceTransform,
    /// `text-spacing-trim` (CSS Text 4). **Inherited**, initial `normal`;
    /// computed value is the specified keyword. The value is data-only and
    /// does not enable text-spacing layout or rendering behavior.
    pub text_spacing_trim: TextSpacingTrim,
    /// `text-justify` (CSS Text 3 §6.2). **Inherited**; initial: `auto`.
    /// This keyword's computed value equals its specified value (copied by
    /// value; `Copy`). Legacy `distribute` is accepted; the inline engine
    /// lays it out as `inter-character`.
    pub text_justify: TextJustify,
    /// `text-align-last` (CSS Text 3 §6.1). **Inherited**; initial: `auto`.
    /// This keyword's computed value equals its specified value (copied by
    /// value; `Copy`). The consumer (the inline engine in `raikiri-dom`) resolves `auto`
    /// (`justify` → `start`; otherwise follow `text-align`) rather than
    /// coupling the two properties in the cascade layer.
    pub text_align_last: TextAlignLast,
    /// `direction`. **Inherited**; initial: [`Direction::Ltr`]
    /// (CSS Writing Modes 4 §2.1, "Specifying Directionality: the direction
    /// property" <https://www.w3.org/TR/css-writing-modes-4/#direction>).
    /// The computed value equals the specified value (no relative resolution;
    /// see [`Direction`]).
    ///
    /// Like sibling [`text_align`](Self::text_align), this is **inherited**.
    /// It belongs in the inherited block of `inherit_from` and copies the
    /// parent's value (`Direction` is `Copy`).
    ///
    /// It was added because resolving [`text_align`](Self::text_align)
    /// `match-parent` (CSS Text 3 §6.1) requires the parent's computed
    /// `direction`. This property also appears independently in CSS Paged
    /// Media 3 Appendix A's page-property list
    /// <https://www.w3.org/TR/css-page-3/#page-property-list>, so it matters
    /// in `@page` contexts too.
    pub direction: Direction,
    /// `writing-mode`. **Inherited**; initial: [`WritingMode::HorizontalTb`]
    /// (CSS Writing Modes 4 §3.2, "Block Flow Direction: the writing-mode
    /// property" <https://www.w3.org/TR/css-writing-modes-4/#propdef-writing-mode>).
    ///
    /// Like sibling [`direction`](Self::direction), this is **inherited**.
    /// It belongs in the inherited block of `inherit_from` and copies the
    /// parent's value (`WritingMode` is `Copy`).
    ///
    /// CSS Paged Media 3 Appendix A's page-property list, cited by
    /// [`direction`](Self::direction), does **not** include `writing-mode`
    /// <https://www.w3.org/TR/css-page-3/#page-property-list>. Nevertheless,
    /// Raikiri deliberately wires `writing-mode` into the `@page` context like
    /// the other properties noted in the "Appendix A is a floor, not a ceiling"
    /// section of [`crate::page::PageCascadeResult::declarations`]:
    /// `overflow-x` / `overflow-y` / `DisplayValue` / `PositionValue` /
    /// `box-sizing` / `counter-reset` / `counter-increment` / `content` /
    /// `string-set` (see the same section in [`WritingMode`]).
    ///
    /// This renderer-facing field remains [`WritingMode::HorizontalTb`] because
    /// vertical writing-mode layout is not implemented. The CSS computed keyword
    /// is preserved separately in [`Self::cssom_writing_mode`]; CSSOM does not
    /// share this layout fallback. [`crate::property::resolve_writing_mode`]
    /// performs the normalization. See [`WritingMode`] doc's Non-goal section.
    pub writing_mode: WritingMode,
    /// CSSOM-facing computed `writing-mode` keyword. **inherited**, initial:
    /// [`WritingMode::HorizontalTb`] (CSS Writing Modes 4 §3.2). This retains the
    /// specified keyword while [`Self::writing_mode`] stays renderer-facing.
    pub cssom_writing_mode: WritingMode,
    /// `ruby-position` — inherited annotation placement.
    pub ruby_position: RubyPosition,
    /// `text-indent` — first-line indentation of a block container.
    /// **Inherited**; initial: [`ComputedTextIndent::Px`]`(0.0)` (CSS
    /// Text 3 §8.1 "First Line Indentation: the text-indent property"
    /// <https://www.w3.org/TR/css-text-3/#text-indent-property>: "Initial:
    /// 0", "Applies to: block containers", "Inherited: yes", "Percentages:
    /// refers to block container's own inline-axis inner size", "Computed
    /// value: computed `<length-percentage>` value, plus any specified
    /// keywords"). The full grammar is
    /// `<length-percentage> && hanging? && each-line?`.
    ///
    /// This field carries **only** the `<length-percentage>` component of
    /// the grammar — `hanging`/`each-line` are not implemented at this
    /// crate's scope ([`crate::property::PropertyValue::TextIndent`] doc's
    /// "Scope carving" section).
    ///
    /// A dedicated computed representation keeps mixed `calc()` terms for
    /// CSSOM serialization while leaving the percentage basis for used-value
    /// layout. `text-indent` percentages use the block container's inline-axis
    /// inner size, unlike [`Self::padding`]'s containing-block width. This
    /// property is inherited; [`crate::specified::SpecifiedValues::inherit_from`]
    /// lifts the parent's computed value with [`crate::resolve::lift_text_indent`].
    pub text_indent: ComputedTextIndent,
    /// Authored `ch` factor retained for the font-metric-aware layout sink.
    pub text_indent_ch_factor: Option<f32>,
    /// Absolute part (px) of a `ch`-bearing `calc()` for `text-indent`, paired with
    /// the factor: a font-aware consumer resolves it as
    /// `factor * advance + offset`. Zero for a plain `Nch` value.
    pub text_indent_ch_offset: f32,
    /// Source font for an inherited `ch` value.
    pub text_indent_ch_font: Option<ChFontKey>,
    /// Whether the `ch` value was inherited from an ancestor.
    pub text_indent_ch_inherited: bool,
    /// `text-indent`'s `hanging` flag. Inherited, initial `false`.
    pub text_indent_hanging: bool,
    /// `text-indent`'s `each-line` flag. Inherited, initial `false`.
    pub text_indent_each_line: bool,
    /// `padding`: four-sided box-model padding. **Not inherited**; initial:
    /// `Sides::all(ComputedLengthPercentage::Px(0.0))` (CSS Box 3 §4.1
    /// <https://www.w3.org/TR/css-box-3/#padding-physical>, initial "0").
    ///
    /// - Physical longhands: [`padding-top`](https://www.w3.org/TR/css-box-3/#propdef-padding-top) /
    ///   `padding-right` / `padding-bottom` / `padding-left`.
    /// - Shorthand: [`padding` (§4.2)](https://www.w3.org/TR/css-box-3/#padding-shorthand).
    ///
    /// Grammar: `<length-percentage [0,∞]>`. Non-negativity is enforced when
    /// parsing (see [`crate::property::PropertyValue::PaddingTop`]). As required
    /// by "Computed value: a computed `<length-percentage>` value", this uses
    /// [`ComputedLengthPercentage`] in its `Px` or `Percent` form.
    /// **`<percentage>` survives into the computed layer** because its basis
    /// (containing-block width) is known only at the used-value layer (CSS
    /// Cascade 5 §4.5 <https://www.w3.org/TR/css-cascade-5/#used>); in Raikiri,
    /// this is delegated to Taffy.
    pub padding: Sides<ComputedLengthPercentage>,
    /// Authored `ch` provenance for each padding side.
    pub padding_ch: Sides<Option<ChLengthProvenance>>,
    /// `margin`: four-sided box-model margin (top / right / bottom / left).
    /// **Not inherited**; initial: `0` on each side
    /// (`Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))`).
    ///
    /// A core author-CSS box-model property. Cascade phase 3 converts `em`,
    /// `rem`, and `pt` to px. `<percentage>` and `auto` remain at the computed
    /// layer (the spec says "Computed value: the keyword `auto` or a computed
    /// `<length-percentage>` value"). Resolving against the containing block
    /// and distributing free space are used-value tasks for downstream Taffy.
    ///
    /// # Primary sources (§ title + anchor)
    ///
    /// - CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties":
    ///   [`margin-top` / `margin-right` / `margin-bottom` / `margin-left`](https://www.w3.org/TR/css-box-3/#margin-physical)
    ///   — "Value: `<length-percentage> | auto`", "Initial: 0", "Inherited: no",
    ///   "Applies to: all elements except internal table elements".
    /// - CSS Box 3 §3.2 "Margin Shorthand: the margin property":
    ///   [`margin`](https://www.w3.org/TR/css-box-3/#margin-shorthand) —
    ///   "Value: `<'margin-top'>{1,4}`", 1/2/3/4 value expansion rules.
    pub margin: Sides<ComputedLengthPercentageOrAuto>,
    /// Authored `ch` provenance for each margin side.
    pub margin_ch: Sides<Option<ChLengthProvenance>>,
    /// `border`: four-sided box-model border (width / style / color × 4 sides).
    /// **Not inherited**; each side initially has width [`ComputedLength::ZERO`],
    /// style [`BorderStyle::None`], and color [`BorderColor::CurrentColor`].
    /// Width and style are crate-internal fields; consumers read them through
    /// [`ComputedBorder::width`] / [`ComputedBorder::style`] accessors.
    ///
    /// The specified initial width is `medium` (3px), but at the **computed
    /// layer it is 0px**: CSS Backgrounds 3 §3.3
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-width> specifies
    /// "Computed value: absolute length, snapped as a border width; zero if the
    /// border style is `none` or `hidden`". The initial border style is `none`,
    /// so [`crate::resolve::resolve_border`] gates the width to zero.
    ///
    /// - Physical longhands: [`border-top-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   `border-top-style` / `border-top-color` × 4 sides.
    /// - Shorthand: [`border` (§3.4)](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands).
    ///
    /// # Value semantics
    ///
    /// - `Sides<ComputedBorder>: Copy` makes each per-node write a bitwise copy
    ///   (`ComputedBorder` contains only f32/enum/BorderColor payloads).
    /// - `width` is a [`ComputedLength`] in px. Since `<percentage>` is not in
    ///   the spec grammar, a length suffices even at the computed layer
    ///   (see [`crate::property::PropertyValue::BorderTopWidth`]).
    /// - `color` stays a [`BorderColor`] enum on the static cascade side, retaining
    ///   the specified-value distinction between the spec's `currentcolor`
    ///   keyword and an explicit `<color>` (CSS Backgrounds 3 §3.1
    ///   <https://www.w3.org/TR/css-backgrounds-3/#border-color>, initial:
    ///   currentcolor). Resolving the used value (currentcolor → the same
    ///   node's computed `color` property, CSS Color 3 §4.4) belongs to paint.
    ///
    /// # Non-goals
    ///
    /// - `border-image-*` subproperties (source/slice/width/outset/repeat)
    ///   are not implemented; `border:` does not reset border-image either
    ///   (an explicit spec deviation to address in future integration work).
    /// - The four single-side `border-{top,right,bottom,left}` shorthands (for
    ///   example, `border-top: 1px solid red`) are not supported yet.
    /// - Retaining `currentcolor` as an enum in the cascade is complete;
    ///   used-value resolution in paint is deferred.
    ///
    /// # Primary sources
    ///
    /// - CSS Backgrounds 3 §3 "Borders":
    ///   [`border-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   [`border-style`](https://www.w3.org/TR/css-backgrounds-3/#border-style) /
    ///   [`border-color`](https://www.w3.org/TR/css-backgrounds-3/#border-color) /
    ///   [`border shorthand`](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands).
    pub border: Sides<ComputedBorder>,
    /// Computed lengths at the four `border-radius` corners. **Not inherited**;
    /// initial: `0px` at every corner. Percentages and elliptical forms are
    /// outside the specified parser's scope.
    pub border_radius: ComputedBorderRadius,
    /// Computed `box-shadow` list. **Not inherited**; initial: empty list
    /// (`none`). Painting the shadows is the paint layer's responsibility.
    pub box_shadow: Arc<Vec<ComputedBoxShadowItem>>,
    /// Computed width/style/color for `outline`. **Not inherited**; initial:
    /// `0px` / `none` / `invert`. Outlines do not affect box-model dimensions.
    /// Specified `medium` width contributes to computed width only if the
    /// outline style is visible.
    pub outline: ComputedOutline,
    /// Computed length for `outline-offset`. **Not inherited**; initial: `0px`
    /// (CSS UI 3 §4.5 <https://www.w3.org/TR/css-ui-3/#outline-offset>).
    /// A negative value insets the outline within the border edge.
    pub outline_offset: ComputedLength,
    /// `width`: preferred physical horizontal size (a writing-mode-neutral
    /// physical property corresponding to the block axis in vertical writing modes).
    /// **Not inherited**; initial: [`ComputedLengthPercentageOrAuto::Auto`]
    /// (CSS Sizing 3 §3.1.1, "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>).
    ///
    /// The spec grammar is `auto | <length-percentage [0,∞]> | min-content |
    /// max-content | fit-content(<length-percentage>)`, but currently only
    /// `auto` and non-negative `<length-percentage>` are accepted. The other
    /// options are not implemented. Negative values violate `[0,∞]` and are
    /// discarded at parse time (see [`crate::property::PropertyValue::Width`]
    /// and `parse_width`).
    ///
    /// As specified by "Computed value: as specified, with `<length-percentage>`
    /// values computed", lengths become absolute px while percentages remain
    /// ([`ComputedLengthPercentageOrAuto`], also used by margin). Downstream
    /// layout resolves `Auto` through CSS Sizing 3 automatic sizing: the used
    /// size is the containing-block width minus margin, border, and padding.
    /// This differs from distributing free space for `margin: auto`.
    pub width: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance for the preferred width.
    pub width_ch: Option<ChLengthProvenance>,
    /// `height`: preferred vertical size. **Not inherited**; initial:
    /// `ComputedLengthPercentageOrAuto::Auto` (CSS Sizing 3 §3.1.1, "Preferred
    /// Size Properties" <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>;
    /// the spec explicitly says "Initial: auto", "Inherited: no").
    ///
    /// Currently accepts only `auto` and non-negative `<length-percentage>`.
    /// The spec also permits `min-content`, `max-content`, and
    /// `fit-content(<length-percentage>)`, but the parser silently drops these
    /// unsupported values (see `parse_height`).
    ///
    /// [`ComputedLengthPercentageOrAuto`] reuses the same shape and payload
    /// type as sibling [`Self::width`]. Resolving percentages against the
    /// containing block and computing the actual layout height of `Auto`
    /// belong to downstream Taffy at the used-value layer.
    ///
    /// # Primary source
    ///
    /// - CSS Sizing 3 §3.1.1 "Preferred Size Properties":
    ///   [`height`](https://www.w3.org/TR/css-sizing-3/#preferred-size-properties)
    ///   — "Value: auto | `<length-percentage [0,∞]>` | min-content |
    ///   max-content | fit-content(`<length-percentage [0,∞]>`)",
    ///   "Initial: auto", "Applies to: all elements except non-replaced
    ///   inlines", "Inherited: no", "Percentages: relative to containing block".
    pub height: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance for the preferred height.
    pub height_ch: Option<ChLengthProvenance>,
    /// `max-width` — **non-inherited**, initial `none` (mapped to Auto as placeholder).
    pub max_width: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance of [`Self::max_width`].
    pub max_width_ch: Option<ChLengthProvenance>,
    /// `max-height` — **non-inherited**, initial `none` (mapped to Auto as placeholder).
    pub max_height: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance of [`Self::max_height`].
    pub max_height_ch: Option<ChLengthProvenance>,
    /// `min-width` — **non-inherited**, initial `auto` (CSS Sizing 3 §4
    /// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
    pub min_width: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance of [`Self::min_width`], after logical mapping.
    pub min_width_ch: Option<ChLengthProvenance>,
    /// `min-height` — **non-inherited**, initial `auto` (CSS Sizing 3 §4
    /// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
    pub min_height: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance of [`Self::min_height`], after logical mapping.
    pub min_height_ch: Option<ChLengthProvenance>,
    /// Authored logical `min-block-size`, retained so layout can distinguish
    /// its fragmentation behavior from a physical `min-height` declaration.
    /// The used value is already mapped into `min_width`/`min_height`.
    pub min_block_size: Option<ComputedLengthPercentageOrAuto>,
    /// Authored `ch` provenance of [`Self::min_block_size`].
    pub min_block_size_ch: Option<ChLengthProvenance>,
    /// Physical `(width, height)` of the authored `inline-size` /
    /// `block-size` mapped through a `vertical-rl` or `vertical-lr`
    /// writing mode (CSS Logical Properties 1 §4.1). `None` without a logical
    /// preferred size or outside those two modes. [`Self::width`] and
    /// [`Self::height`] keep the horizontal mapping for layout that runs
    /// vertical content on horizontal axes.
    pub vertical_logical_size: Option<VerticalLogicalSize>,
    /// `top`. **Not inherited**; initial: `auto`.
    pub top: ComputedLengthPercentageOrAuto,
    /// `right`. **Not inherited**; initial: `auto`.
    pub right: ComputedLengthPercentageOrAuto,
    /// `bottom`. **Not inherited**; initial: `auto`.
    pub bottom: ComputedLengthPercentageOrAuto,
    /// `left`. **Not inherited**; initial: `auto`.
    pub left: ComputedLengthPercentageOrAuto,
    /// `box-sizing`. **Not inherited**; initial: [`BoxSizing::ContentBox`]
    /// (CSS Sizing 3 §3.3, "Box Edges for Sizing: the box-sizing property"
    /// <https://www.w3.org/TR/css-sizing-3/#box-sizing>, "Initial: `content-box`"
    /// / "Inherited: no"). The computed value is the specified keyword.
    ///
    /// The spec notes in §3.3 that "The definition of the box-sizing property
    /// in this module supersedes the one in [CSS-UI-3]". CSS Sizing 3 is thus
    /// the authoritative source, not CSS UI 3.
    ///
    /// Like [`display`](Self::display) and
    /// [`background_color`](Self::background_color), this is **not inherited**:
    /// `inherit_from` sets the initial value rather than copying the parent.
    ///
    /// # Downstream handoff (future scope, confined to style)
    ///
    /// This field holds only the static cascade-side seed. Translating it to
    /// `taffy::Style::box_sizing` in the dom-scope `apply_computed_to_style`
    /// bridge is deferred to a future cross-scope task (non-goal).
    pub box_sizing: BoxSizing,
    /// `overflow-x` + `overflow-y`. **non-inherited**, initial:
    /// [`OverflowXY::both`]`(`[`OverflowValue::Visible`]`)` (CSS Overflow
    /// Module Level 3 §3.1 "Overflow: the overflow-x, overflow-y,
    /// overflow-block, overflow-inline, and overflow properties"
    /// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>, "Initial:
    /// visible" / "Inherited: no").
    ///
    /// Computed value is **not** simply the specified keyword — CSS Overflow
    /// 3 §3.1 defines a cross-axis coupling ("The visible/clip values of
    /// overflow compute to auto/hidden (respectively) if one of overflow-x
    /// or overflow-y is neither visible nor clip"), applied by
    /// [`resolve_overflow`](crate::property::resolve_overflow) in phase 3.
    /// Bundled into one [`OverflowXY`] field (rather than two independent
    /// scalar fields) for the same reason [`Self::border`] bundles
    /// `border-*-width`/`border-*-style` into [`Sides<ComputedBorder>`] — the
    /// coupling needs both axes at once ([`OverflowXY`] doc).
    ///
    /// # Downstream handoff (future scope, style-scope confined)
    ///
    /// This field carries the cascade static side seed only, mirroring
    /// [`Self::box_sizing`] — the block-formatting-context establishment and
    /// float-clearing consequences of `overflow != visible` (CSS 2.1 §9.4.1 /
    /// §9.5) are dom/paint scope and deferred to a follow-up task (see the
    /// `overflow` UA rule comment in
    /// `crates/raikiri-html/src/ua/minimal.css`).
    pub overflow: OverflowXY,
    /// `text-decoration-line`. **non-inherited**, initial:
    /// [`TextDecorationLine::NONE`] (CSS Text Decoration Module Level 3
    /// §2.1 "Text Decoration Lines: the text-decoration-line property"
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>,
    /// "Initial: none" / "Inherited: no"). Computed value = specified
    /// keyword(s) ([`TextDecorationLine`] doc — no length payload, so no
    /// relative resolution is needed).
    ///
    /// The paint walker consumes this field as the originating line and
    /// propagates it to descendant text runs per CSS Text Decoration 3.
    pub text_decoration_line: TextDecorationLine,
    /// `text-decoration-style`. **non-inherited**, initial:
    /// [`TextDecorationStyle::Solid`] (CSS Text Decoration Module Level 3
    /// §2.2 "Text Decoration Style: the text-decoration-style property"
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>,
    /// "Initial: solid" / "Inherited: no"). Computed value = specified
    /// keyword ([`TextDecorationStyle`] doc).
    ///
    /// The paint walker consumes this field when it draws the originating
    /// line, including `double`, `dotted`, `dashed`, and `wavy` styles.
    pub text_decoration_style: TextDecorationStyle,
    /// `text-decoration-color`. **non-inherited**, initial:
    /// [`TextDecorationColor::CurrentColor`] (CSS Text Decoration Module
    /// Level 3 §2.3 "Text Decoration Color: the text-decoration-color
    /// property"
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>,
    /// "Initial: currentcolor" / "Inherited: no"). Computed value =
    /// computed color ([`TextDecorationColor`] doc — used-value resolution
    /// of `currentcolor` is paint scope responsibility, mirroring
    /// [`BorderColor`]).
    ///
    /// The paint walker resolves `currentcolor` against the originating
    /// element's computed [`Self::color`] before drawing the line.
    pub text_decoration_color: TextDecorationColor,
    /// `text-decoration-thickness`. **non-inherited**, initial: `auto` (CSS
    /// Text Decoration 4, [`crate::property::TextDecorationThickness`]). Computed lengths are
    /// absolute CSS pixels; `auto` and `from-font` remain keywords. No thickness
    /// painting behavior is implemented by this field.
    pub text_decoration_thickness: ComputedTextDecorationThickness,
    /// `text-decoration-skip-ink` (CSS Text Decoration 4). **Inherited**,
    /// initial: [`TextDecorationSkipInk::Auto`]. Stored for computed-style output;
    /// no skip-ink drawing behavior is implemented.
    pub text_decoration_skip_ink: TextDecorationSkipInk,
    /// `text-decoration-skip-spaces` (CSS Text Decoration 4). **Inherited**,
    /// initial: [`TextDecorationSkipSpaces::StartEnd`]. Stored for computed-style
    /// output; no decoration drawing behavior is implemented.
    pub text_decoration_skip_spaces: TextDecorationSkipSpaces,
    /// `text-decoration-inset`. **non-inherited**, initial: `0` (CSS Text
    /// Decoration 4 §2.9.1). Lengths are absolute in the computed layer so
    /// paint can trim or extend each decoration segment without re-resolving
    /// against the originating font.
    pub text_decoration_inset: ComputedTextDecorationInset,
    /// Authored `ch` provenance for the inset's inline-start length.
    pub text_decoration_inset_start_ch: Option<ChLengthProvenance>,
    /// Authored `ch` provenance for the inset's inline-end length.
    pub text_decoration_inset_end_ch: Option<ChLengthProvenance>,
    /// `text-underline-offset`. **inherited**, initial: `auto` (CSS Text
    /// Decoration 4 §2.8). Length values are fixed computed offsets and are
    /// carried with the decoration origin; mixed calcs retain that resolved
    /// length while percentages stay relative to the font size of the element
    /// the value is used on.
    pub text_underline_offset: ComputedTextUnderlineOffset,
    /// `text-underline-position`. **inherited**, initial: `auto` (CSS Text
    /// Decoration 4 §2.7). The keyword set is preserved as the computed value;
    /// this field does not implement underline placement or painting.
    pub text_underline_position: TextUnderlinePosition,
    /// `text-emphasis-position`. **inherited**, initial: `over right` (CSS
    /// Text Decoration 4 §3.4). The keyword value is preserved; emphasis
    /// placement and painting are out of scope.
    pub text_emphasis_position: TextEmphasisPosition,
    /// `text-emphasis-style`. **inherited**, initial: `none`. Shape/fill and
    /// string values are retained in computed style; emphasis painting is out
    /// of scope.
    pub text_emphasis_style: TextEmphasisStyle,
    /// `text-emphasis-color`. **inherited**, initial: `currentColor`. The
    /// current-color sentinel is resolved against [`Self::color`] when a
    /// computed shorthand string is exposed; no emphasis painting is added.
    pub text_emphasis_color: TextDecorationColor,
    /// `vertical-align`. **non-inherited**, initial:
    /// [`VerticalAlign::Baseline`] (CSS 2.1 §10.8.1 "Vertical alignment: the
    /// 'vertical-align' property"
    /// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>,
    /// "Initial: baseline" / "Inherited: no"). Computed value: the 6
    /// keywords (`baseline`/`sub`/`super`/`middle`/`text-top`/`text-bottom`)
    /// stay the specified keyword; [`VerticalAlign::Length`] and mixed
    /// [`VerticalAlign::Calc`] values absolutize to `Length::Px`
    /// ([`crate::resolve::resolve_vertical_align`] doc, percentages use the
    /// element's used line-height, with the existing `normal` fallback).
    ///
    /// # Scope carving
    ///
    /// This field holds the values described on [`VerticalAlign`]'s own
    /// "Scope carving" doc. `top` / `bottom` stay as explicit keyword
    /// variants; lengths, percentages, and mixed `calc()` values are
    /// absolutized to `Length::Px` before paint.
    ///
    /// # Same type at both the specified and computed layer
    ///
    /// Unlike most length-bearing fields in this crate (`flex_basis` /
    /// `letter_spacing` / `border`, which all use a dedicated `Computed*`
    /// type distinct from their specified-layer type), this field keeps the
    /// **same** [`VerticalAlign`] type [`crate::specified::SpecifiedValues::vertical_align`]
    /// carries. This is forced by raikiri-paint: its
    /// `vertical_align_shift_px` function takes this crate's
    /// [`VerticalAlign`] by value directly, so introducing a separate
    /// computed-only type here would require a raikiri-paint signature
    /// change this crate's scope does not include. See
    /// [`crate::resolve::resolve_vertical_align`] doc for how the
    /// [`VerticalAlign::Length`] and [`VerticalAlign::Calc`] variants are
    /// absolutized without changing this field's type.
    ///
    /// # Downstream handoff
    ///
    /// This field carries the cascade static side value only, mirroring
    /// [`Self::text_decoration_line`] — the baseline-shift amount
    /// calculation and glyph rendering is raikiri-paint scope, not this
    /// crate's. raikiri-paint consumes this field for `sub`/`super` (a
    /// used-font-size-relative pixel offset applied at glyph draw time);
    /// `baseline` continues to contribute no offset by definition. The
    /// other 3 keywords (`middle`/`text-top`/`text-bottom`) and the
    /// absolutized `Length` payload fall through raikiri-paint's own
    /// `#[non_exhaustive]` wildcard fallback to the same 0px shift until
    /// that crate's own shift-calculation work lands — this crate's job
    /// ends at carrying the (now, for `Length`, absolutized) value through.
    /// This does not by itself make `sub`/`super` content appear inline
    /// with its surrounding text — raikiri-dom has no inline formatting
    /// context yet (every element, `inline` included, lays out as its own
    /// block row), which is a separate, larger, pre-existing gap this
    /// field's integration does not close.
    pub vertical_align: VerticalAlign,
    /// `font-style`. **inherited**, initial: [`FontStyle::Normal`] (CSS
    /// Fonts Module Level 4 §2.4 "Font style: the font-style property"
    /// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>, "Initial:
    /// normal" / "Inherited: yes"). Computed value = specified keyword —
    /// see [`FontStyle`] doc's "Scope carving" section (the spec's
    /// angle-bearing computed-value branch is unreachable at this crate's
    /// scope, since `oblique`'s optional `<angle>` argument is not
    /// implemented).
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `normal | italic | oblique` subset of the
    /// property's full `normal | italic | left | right | oblique <angle
    /// [-90deg,90deg]>?` grammar — see [`FontStyle`] doc.
    pub font_style: FontStyle,
    /// `font-kerning` — inherited, initial `auto`; preserved for CSSOM only.
    /// No glyph-shaping or painting behavior is changed.
    pub font_kerning: FontKerning,
    /// `font-optical-sizing` — inherited, initial `auto`; preserved for CSSOM only.
    /// No optical-size selection or glyph-shaping behavior is changed.
    pub font_optical_sizing: FontOpticalSizing,
    /// `font-variant-emoji` — inherited, initial `normal`; preserved for CSSOM only.
    /// No emoji presentation or glyph-selection behavior is changed.
    pub font_variant_emoji: FontVariantEmoji,
    /// `font-language-override` — inherited, initial `normal`; preserved for CSSOM only.
    /// No language-system selection or glyph-shaping behavior is changed.
    pub font_language_override: FontLanguageOverride,
    /// `font-variant-ligatures` — inherited, initial `normal`; retained for CSSOM only.
    /// This field does not change ligature shaping or painting.
    pub font_variant_ligatures: FontVariantLigatures,
    /// `font-synthesis` — inherited, initial `weight style small-caps position`.
    /// Preserved as CSSOM data only; no synthetic fonts or shaping changes are applied.
    pub font_synthesis: FontSynthesisValue,
    /// `font-variant-position` — inherited, initial `normal`, computed as specified.
    /// Kept as CSSOM data only; sub/sup glyph shaping and synthesis are unchanged.
    pub font_variant_position: FontVariantPosition,
    /// `font-palette` — inherited, initial `normal`, computed as specified.
    /// Preserved as CSSOM data only; no palette selection or glyph rendering is performed.
    pub font_palette: FontPaletteValue,
    /// `font-variant-numeric` — inherited, initial `normal`, computed as specified.
    /// Stored as CSSOM data only; OpenType shaping is unchanged.
    pub font_variant_numeric: FontVariantNumeric,
    /// `font-variant-east-asian` — inherited, initial `normal`, computed as specified.
    /// Stored as CSSOM data only; glyph substitution is unchanged.
    pub font_variant_east_asian: FontVariantEastAsian,
    /// `font-variation-settings` — inherited, initial `normal`, computed as a deduplicated sorted list.
    /// Stored as CSSOM data only; font axes and shaping are not applied.
    pub font_variation_settings: FontVariationSettings,
    /// `font-feature-settings` — inherited, initial `normal`, computed as a deduplicated sorted list.
    /// Explicit OpenType feature settings are forwarded to the IFC shaping style.
    pub font_feature_settings: FontFeatureSettings,
    /// `font-variant-caps`. **inherited**, initial:
    /// [`FontVariantCaps::Normal`] (CSS Fonts Module Level 3 §6.6
    /// "Capitalization: the font-variant-caps property"
    /// <https://www.w3.org/TR/css-fonts-3/#font-variant-caps-prop>,
    /// "Initial: normal" / "Inherited: yes"). Computed value = specified
    /// keyword. See [`FontVariantCaps`] for the seven supported keywords and
    /// the scope limit excluding the `font-variant` shorthand.
    pub font_variant_caps: FontVariantCaps,
    /// `text-transform`. **inherited**, initial: [`TextTransform::None`]
    /// (CSS Text 4 property definition:
    /// <https://www.w3.org/TR/css-text-4/#propdef-text-transform>,
    /// "Initial: none" / "Inherited: yes"). Computed value = specified
    /// keyword.
    ///
    /// Case and width combinations and standalone `math-auto` are preserved
    /// as specified keywords; see [`TextTransform`] for the grammar. The
    /// downstream math-text behavior of `math-auto` is outside this crate.
    pub text_transform: TextTransform,
    /// `text-combine-upright`. **inherited**, initial: [`TextCombineUpright::None`]
    /// (CSS Writing Modes 3 §9.1). Computed value = specified keyword; text
    /// composition is outside this field.
    pub text_combine_upright: TextCombineUpright,
    /// `text-orientation`. **inherited**, initial: [`TextOrientation::Mixed`]
    /// (CSS Writing Modes 3 §5.1, initial mixed and inherited yes). Computed
    /// value = specified keyword; no writing-mode layout is performed here.
    pub text_orientation: TextOrientation,
    /// `unicode-bidi`. **non-inherited**, initial: [`UnicodeBidi::Normal`]
    /// (CSS Writing Modes 3 §2.2). Computed value = specified value; bidi
    /// layout behavior is outside this field.
    pub unicode_bidi: UnicodeBidi,
    /// `visibility`. **inherited**, initial: [`Visibility::Visible`] (CSS
    /// Display Module Level 3 §4 "Invisibility: the visibility property"
    /// <https://www.w3.org/TR/css-display-3/#visibility>, "Initial: visible"
    /// / "Inherited: yes"). Computed value = specified keyword — see
    /// [`Visibility`] doc's "Scope carving" section (the spec's
    /// formatting-context-specific space-saving effect for `collapse` is
    /// downstream layout scope, not represented by this field).
    pub visibility: Visibility,
    /// `z-index`. **non-inherited**, initial: [`ZIndexValue::Auto`] (CSS2
    /// §9.9.1 "Specifying the stack level: the 'z-index' property"
    /// <https://www.w3.org/TR/CSS2/visuren.html#z-index>, "Initial: auto" /
    /// "Inherited: no"). Computed value = specified value ([`ZIndexValue`]
    /// doc — no length payload, so no relative resolution is needed).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only — no consumer reads it
    /// yet. [`ZIndexValue`] doc's "Scope carving" section explains why:
    /// this crate's `position` property does not implement the CSS2
    /// `relative`/`absolute`/`fixed` keywords that "positioned
    /// elements" (the propdef's "Applies to" clause) presupposes ( `sticky`
    /// is parsed as [`crate::property::PositionValue::Sticky`] but has no
    /// layout consumer yet, `relative`/`absolute`/`fixed` remain silent drop),
    /// so there is no stacking-context/paint-order consumer to wire up yet.
    pub z_index: ZIndexValue,
    /// `word-break`. **inherited**, initial: [`WordBreak::Normal`] (CSS
    /// Text Module Level 3 §5.1 "Breaking Rules for Letters: the
    /// word-break property"
    /// <https://www.w3.org/TR/css-text-3/#word-break-property>, "Initial:
    /// normal" / "Inherited: yes"). Computed value = specified keyword
    /// ([`WordBreak`] doc — no length payload, so no relative resolution
    /// is needed).
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `normal | keep-all | break-all` subset of
    /// the property's full `normal | keep-all | break-all | break-word`
    /// grammar — the deprecated `break-word` value is not represented, see
    /// [`WordBreak`] doc's "Scope carving" section.
    pub word_break: WordBreak,
    /// `line-break`. **inherited**, initial: [`LineBreak::Auto`] (CSS Text
    /// Module Level 3 §5.3). The layout sink uses `anywhere` as an
    /// emergency opportunity between typographic units.
    pub line_break: LineBreak,
    /// `overflow-wrap` (legacy name alias: `word-wrap`). **inherited**,
    /// initial: [`OverflowWrap::Normal`] (CSS Text Module Level 3 §5.4
    /// "Overflow Wrapping: the overflow-wrap (word-wrap) property"
    /// <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>,
    /// "Initial: normal" / "Inherited: yes"). Computed value = specified
    /// keyword ([`OverflowWrap`] doc — no length payload, so no relative
    /// resolution is needed).
    ///
    /// `word-wrap` is not a separate field — `parse_value` dispatches both
    /// names to this same field's [`PropertyKey::OverflowWrap`]
    /// ([`OverflowWrap`] doc's "legacy alias" section).
    ///
    /// [`PropertyKey::OverflowWrap`]: crate::property::PropertyKey::OverflowWrap
    pub overflow_wrap: OverflowWrap,
    /// Renderer-facing absolute fallback for `letter-spacing`. This preserves
    /// the existing layout contract; percentage and calc values use zero here
    /// until text layout consumes [`Self::letter_spacing_computed`].
    pub letter_spacing: ComputedLength,
    /// Computed `letter-spacing` value retained for inheritance and CSSOM
    /// serialization. This carries percentages and mixed length-percentage
    /// calcs separately from the absolute renderer fallback.
    pub letter_spacing_computed: ComputedLetterSpacing,
    /// Authored `ch` factor for `letter-spacing`, retained so the text-layout
    /// sink can replace the style fallback with the shaping font's `0` advance.
    pub letter_spacing_ch_factor: Option<f32>,
    /// Absolute part (px) of a `ch`-bearing `calc()` for `letter-spacing`, paired with
    /// the factor: a font-aware consumer resolves it as
    /// `factor * advance + offset`. Zero for a plain `Nch` value.
    pub letter_spacing_ch_offset: f32,
    /// Font that declared [`Self::letter_spacing_ch_factor`]. The computed
    /// value is an absolute length, so an inherited `ch` measures with the
    /// declaring element's font, not the inheriting element's.
    pub letter_spacing_ch_font: Option<ChFontKey>,
    /// Absolute renderer/layout fallback for `word-spacing`. The inherited CSSOM
    /// computed form, including percentages and mixed calc terms, is retained in
    /// [`Self::word_spacing_computed`]. Initial fallback: zero.
    pub word_spacing: ComputedLength,
    /// CSS Text 4 computed `word-spacing` value. Percentages and mixed calc
    /// terms remain distinct from the absolute renderer/layout fallback.
    pub word_spacing_computed: ComputedWordSpacing,
    /// Authored `ch` factor for `word-spacing`, when the winning declaration
    /// used that font-metric-relative unit. The factor is retained through
    /// inheritance so the text-layout sink can replace the style-layer
    /// fallback with the shaping font's `0` glyph advance.
    pub word_spacing_ch_factor: Option<f32>,
    /// Absolute part (px) of a `ch`-bearing `calc()` for `word-spacing`, paired with
    /// the factor: a font-aware consumer resolves it as
    /// `factor * advance + offset`. Zero for a plain `Nch` value.
    pub word_spacing_ch_offset: f32,
    /// Font that declared [`Self::word_spacing_ch_factor`]; see
    /// [`Self::letter_spacing_ch_font`].
    pub word_spacing_ch_font: Option<ChFontKey>,
    /// `tab-size`. **inherited**, initial: [`ComputedTabSize::Number`]`(8.0)`
    /// (CSS Text Module Level 3 §4.2 "Tab Character Size: the tab-size
    /// property" <https://www.w3.org/TR/css-text-3/#tab-size-property>,
    /// "Initial: 8" / "Inherited: yes"). Computed value: the specified
    /// number, or an absolutized length ([`ComputedTabSize`] doc).
    ///
    /// # Scope carving (nothing reads this field yet)
    ///
    /// This field carries the cascaded/absolutized value only — no consumer
    /// reads it yet, same as [`Self::white_space`] doc's "Scope carving"
    /// section: the actual tab-stop advance calculation (§4.2's "multiple of
    /// the advance width of the space character... of the nearest block
    /// container ancestor") is font-metric-dependent text layout behavior
    /// (raikiri-dom / raikiri-paint scope) this crate does not implement.
    pub tab_size: ComputedTabSize,
    /// `break-before` (legacy shorthand: `page-break-before`).
    /// **non-inherited**, initial: [`BreakBetween::Auto`] (CSS
    /// Fragmentation Module Level 3 §3.1 "Breaks Between Boxes: the
    /// break-before and break-after properties"
    /// <https://www.w3.org/TR/css-break-3/#break-between>, "Initial: auto"
    /// / "Inherited: no"). Computed value = specified keyword
    /// ([`BreakBetween`] doc — no length payload, so no relative
    /// resolution is needed).
    ///
    /// `page-break-before` is not a separate field — `parse_value`
    /// dispatches both names to this same field's
    /// [`PropertyKey::BreakBefore`] ([`BreakBetween`] doc's "legacy
    /// shorthand" section).
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `auto | avoid | avoid-page | page`
    /// subset of the property's full 12-keyword grammar — see
    /// [`BreakBetween`] doc's "Scope carving" section.
    ///
    /// [`PropertyKey::BreakBefore`]: crate::property::PropertyKey::BreakBefore
    pub break_before: BreakBetween,
    /// `break-after` (legacy shorthand: `page-break-after`). Same shape as
    /// [`Self::break_before`] — see that field's doc (CSS Fragmentation
    /// Module Level 3 §3.1, [`BreakBetween`] doc).
    ///
    /// [`PropertyKey::BreakAfter`]: crate::property::PropertyKey::BreakAfter
    pub break_after: BreakBetween,
    /// `break-inside` (legacy shorthand: `page-break-inside`).
    /// **non-inherited**, initial: [`BreakInside::Auto`] (CSS Fragmentation
    /// Module Level 3 §3.2 "Breaks Within Boxes: the break-inside
    /// property" <https://www.w3.org/TR/css-break-3/#break-within>,
    /// "Initial: auto" / "Inherited: no"). Computed value = specified
    /// keyword ([`BreakInside`] doc — no length payload, so no relative
    /// resolution is needed; a smaller, disjoint value set from
    /// [`Self::break_before`]/[`Self::break_after`]'s [`BreakBetween`]).
    ///
    /// `page-break-inside` is not a separate field — same dispatch shape
    /// as [`Self::break_before`]'s doc describes.
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `auto | avoid | avoid-page` subset of the
    /// property's full 5-keyword grammar — see [`BreakInside`] doc's
    /// "Scope carving" section.
    pub break_inside: BreakInside,
    /// `float`. **non-inherited**, initial: [`FloatValue::None`] (CSS2
    /// §9.5.1 "Positioning the float: the 'float' property"
    /// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>, "Initial:
    /// none" / "Inherited: no"). Computed value = specified value
    /// ([`FloatValue`] doc — no length payload, so no relative resolution
    /// is needed).
    ///
    /// CSS2 §9.7's forced `display` recomputation when this field is not
    /// [`FloatValue::None`] has already been applied to [`Self::display`]
    /// by the time this field is populated — see
    /// [`crate::property::resolve_display_for_float`] doc.
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. Actual float
    /// positioning, shrink-to-fit width, and line-box shortening (CSS2
    /// §9.5's exclusion-area algorithm) are layout-time behavior
    /// (raikiri-dom scope) — [`FloatValue`] doc's "Scope carving" section.
    pub float: FloatValue,
    /// `clear`. **non-inherited**, initial: [`ClearValue::None`] (CSS2
    /// §9.5.2 "Controlling flow next to floats: the 'clear' property"
    /// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>, "Initial:
    /// none" / "Inherited: no"). Computed value = specified value
    /// ([`ClearValue`] doc — no length payload, so no relative resolution
    /// is needed).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. Clearance computation
    /// and the vertical displacement it produces are layout-time behavior
    /// (raikiri-dom scope) — [`ClearValue`] doc's "Scope carving" section.
    pub clear: ClearValue,
    /// `white-space`. **inherited**, initial: [`WhiteSpace::Normal`] (CSS
    /// Text Module Level 3 §3 "White Space and Wrapping: the white-space
    /// property" <https://www.w3.org/TR/css-text-3/#white-space-property>,
    /// "Initial: normal" / "Inherited: yes"). Computed value = specified
    /// keyword.
    ///
    /// # Scope carving
    ///
    /// This field carries the full `normal | pre | nowrap | pre-wrap |
    /// break-spaces | pre-line` keyword set — see [`WhiteSpace`] doc. The
    /// text-layout consumer in `raikiri-dom` applies the phase-1 behavior:
    /// collapsible whitespace is merged for `normal`/`nowrap`/`pre-line`,
    /// segment breaks are preserved as forced breaks only for `pre-line`, and
    /// the `pre` family preserves source whitespace. The additional
    /// end-of-line occupancy rules for `break-spaces` remain a downstream
    /// line-breaking detail.
    pub white_space: WhiteSpace,
    /// `white-space-collapse`. **Inherited**, initial:
    /// [`WhiteSpaceCollapse::Collapse`] (CSS Text 4 property definition:
    /// <https://www.w3.org/TR/css-text-4/#propdef-white-space-collapse>).
    /// Computed value = specified keyword.
    ///
    /// This field carries the cascaded keyword only. It does not transform
    /// whitespace or change text-layout behavior.
    pub white_space_collapse: WhiteSpaceCollapse,
    /// `text-wrap` wrapping component. Inherited, initial `wrap`.
    pub text_wrap: TextWrapMode,
    /// `text-wrap-style`. **Inherited**, initial [`TextWrapStyle::Auto`]. The
    /// computed value is the specified keyword; this field does not affect
    /// line wrapping or text layout.
    pub text_wrap_style: TextWrapStyle,
    /// `white-space-collapse` after the legacy `white-space` keyword and the
    /// longhand are settled by cascade order. Inherited. The CSSOM does not
    /// read it; it serializes the declared fields above.
    pub effective_white_space_collapse: WhiteSpaceCollapse,
    /// `text-wrap-mode` after the legacy `white-space` keyword and the
    /// longhands are settled by cascade order. Inherited. The CSSOM does not
    /// read it.
    pub effective_text_wrap_mode: TextWrapMode,
    /// `hyphens`. **inherited**, initial: [`Hyphens::Manual`] (CSS Text
    /// Module Level 3 §5.3 "Hyphenation: the hyphens property"
    /// <https://www.w3.org/TR/css-text-3/#hyphens-property>, "Initial:
    /// manual" / "Inherited: yes"). Computed value = specified keyword.
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only — no consumer reads it
    /// yet, same as [`Self::white_space`] doc's note: the actual
    /// hyphenation-opportunity computation belongs to a text layout /
    /// line-breaking consumer this crate does not implement yet. `auto` is
    /// stored as its own distinct value, not collapsed to `manual`, even
    /// though dictionary-based automatic hyphenation is out of this crate's
    /// scope — see [`Hyphens`] doc's "Downstream handoff" section for why
    /// the collapse (to soft-hyphen-only splitting) is a consumer-side
    /// decision rather than something this field's computed value encodes.
    pub hyphens: Hyphens,
    /// `hyphenate-character`. Inherited, initial `auto` (CSS Text 4).
    /// The explicit string remains decoded Unicode text; this field does not
    /// perform hyphenation or insert characters into lines.
    pub hyphenate_character: HyphenateCharacter,
    /// `hyphenate-limit-chars`. **Inherited**, initial [`HyphenateLimitChars::INITIAL`].
    /// The triple is a computed style value only; this crate does not apply
    /// hyphenation or line-breaking behavior.
    pub hyphenate_limit_chars: HyphenateLimitChars,
    /// `flex-direction`. **non-inherited**, initial:
    /// [`FlexDirectionValue::Row`] (CSS Flexible Box Layout Module Level 1
    /// §5.1 <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>,
    /// "Inherited: no"). Computed value = specified keyword ([`FlexDirectionValue`]
    /// doc — no length payload).
    pub flex_direction: FlexDirectionValue,
    /// `flex-wrap`. **non-inherited**, initial: [`FlexWrapValue::NoWrap`]
    /// (CSS Flexible Box Layout Module Level 1 §5.2
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>, "Inherited:
    /// no"). Computed value = specified keyword.
    pub flex_wrap: FlexWrapValue,
    /// `flex-grow`. **non-inherited**, initial: `0.0` (CSS Flexible Box
    /// Layout Module Level 1 §7.2.1
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-grow-property>, "Inherited:
    /// no"). Computed value = specified number ("`<number [0,∞]>`") —
    /// [`crate::property`]'s parser enforces `[0,∞]` **and** finiteness at
    /// parse time (this field has no downstream sink guard between here and
    /// `taffy::Style::flex_grow`, unlike geometry fields that pass through
    /// `raikiri-dom`'s `sanitize_taffy`).
    pub flex_grow: f32,
    /// `flex-shrink`. **non-inherited**, initial: `1.0` (CSS Flexible Box
    /// Layout Module Level 1 §7.2.2
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-shrink-property>,
    /// "Inherited: no"). Same `[0,∞]` + finite parse-time enforcement as
    /// [`Self::flex_grow`].
    pub flex_shrink: f32,
    /// `flex-basis`. **non-inherited**, initial:
    /// [`ComputedFlexBasis::Auto`] (CSS Flexible Box Layout Module Level 1
    /// §7.2.3 <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>,
    /// "Inherited: no"). Computed value: specified keyword (`auto` /
    /// `content`) or a computed `<length-percentage>` value
    /// ([`ComputedFlexBasis`] doc).
    pub flex_basis: ComputedFlexBasis,
    /// `order`. **non-inherited**, initial: `0` (CSS Flexible Box Layout
    /// Module Level 1 §4.2
    /// <https://www.w3.org/TR/css-flexbox-1/#order-property>, "Inherited:
    /// no"). Computed value = specified integer (no length payload —
    /// same opaque pass-through as [`Self::flex_grow`]).
    pub order: i32,
    /// `justify-content`. **non-inherited**, initial:
    /// [`ContentAlignmentValue::Normal`] (CSS Box Alignment Module Level 3
    /// §5.1 <https://www.w3.org/TR/css-align-3/#propdef-justify-content>,
    /// "Inherited: no"). Computed value = specified keyword(s)
    /// ([`ContentAlignmentValue`] doc — shared with [`Self::align_content`]).
    pub justify_content: ContentAlignmentValue,
    /// `align-content`. **non-inherited**, initial:
    /// [`ContentAlignmentValue::Normal`] (CSS Box Alignment Module Level 3
    /// §5.1 <https://www.w3.org/TR/css-align-3/#propdef-align-content>,
    /// "Inherited: no"). Computed value = specified keyword(s).
    pub align_content: ContentAlignmentValue,
    /// `align-items`. **non-inherited**, initial:
    /// [`SelfAlignmentValue::Normal`] (CSS Box Alignment Module Level 3
    /// §7.2 <https://www.w3.org/TR/css-align-3/#propdef-align-items>,
    /// "Inherited: no"). Computed value = specified keyword(s).
    pub align_items: SelfAlignmentValue,
    /// `align-self`. **non-inherited**, initial: [`AlignSelfValue::Auto`]
    /// (CSS Box Alignment Module Level 3 §6.2
    /// <https://www.w3.org/TR/css-align-3/#propdef-align-self>, "Inherited:
    /// no"). Computed value = specified keyword(s).
    pub align_self: AlignSelfValue,
    /// `row-gap`. **non-inherited**, initial:
    /// [`ComputedLengthPercentageOrNormal::Normal`] (CSS Box Alignment
    /// Module Level 3 §8.1
    /// <https://www.w3.org/TR/css-align-3/#propdef-row-gap>, "Inherited:
    /// no"). Computed value: specified keyword, else a computed
    /// `<length-percentage>` value ([`ComputedLengthPercentageOrNormal`] doc).
    pub row_gap: ComputedLengthPercentageOrNormal,
    /// `column-gap`. **non-inherited**, initial:
    /// [`ComputedLengthPercentageOrNormal::Normal`] (CSS Box Alignment
    /// Module Level 3 §8.1
    /// <https://www.w3.org/TR/css-align-3/#propdef-column-gap>, "Inherited:
    /// no"). Same shape as [`Self::row_gap`].
    pub column_gap: ComputedLengthPercentageOrNormal,
    /// `quotes` — nesting-level `(open, close)` string pairs. **inherited**.
    /// CSS Content Module Level 3 §2.4.1
    /// <https://www.w3.org/TR/css-content-3/#quotes-property>, legacy CSS2
    /// §12.3.1 <https://www.w3.org/TR/CSS21/generate.html#quotes-specify>.
    /// Spec initial is "depends on user agent" — no concrete string table is
    /// specified. This implementation represents both the explicit `none`
    /// keyword and the (independent-implementation) unspecified-initial case as an
    /// empty list — no other implementation's UA default is imported (see
    /// [`crate::property::PropertyValue::Quotes`] doc for the full
    /// rationale, including the `auto`/`match-parent` keywords this crate
    /// does not implement).
    ///
    /// Resolving nesting depth to an actual quote-mark string (consulted
    /// when [`Self::content`] carries a [`ContentComponent::Quote`]) is
    /// downstream (raikiri-dom) responsibility — this crate stops at
    /// cascade + inheritance wire-through, same split as [`Self::content`]
    /// itself.
    ///
    /// [`Arc<Vec<..>>`] wrap is the same DoS-mitigation shape as
    /// [`Self::counter_reset`] — cascade winner clone / inheritance walk
    /// clone become a shallow Arc reference-count increment instead of a per-node `Vec` copy.
    pub quotes: Arc<Vec<(SmolStr, SmolStr)>>,
    /// Whether an empty `quotes` list represents the initial `auto` value.
    /// Explicit `quotes: none` keeps the list empty but clears this marker.
    pub quotes_auto: bool,
    /// `text-shadow`. **inherited**, initial: empty list (= `none`) (CSS
    /// Text Decoration Module Level 3 §4
    /// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>,
    /// "Initial: none" / "Inherited: yes"). Computed value: "a list, each
    /// item consisting of three absolute lengths plus a computed color"
    /// ([`ComputedTextShadow`] doc — `currentcolor` stays symbolic, used-value
    /// resolution is paint scope responsibility, mirroring
    /// [`Self::text_decoration_color`]).
    ///
    /// # Downstream handoff (future scope, style-scope confined)
    ///
    /// This field carries the cascade static side seed only, mirroring
    /// [`Self::text_decoration_line`] — actually painting the shadow
    /// (including the blur approximation) is raikiri-paint scope and not yet
    /// wired.
    pub text_shadow: Arc<Vec<ComputedTextShadow>>,
    /// `grid-template-columns`. **non-inherited**, initial:
    /// [`ComputedGridTemplateTracks::None`] (CSS Grid Layout Module Level 1
    /// §7.2 <https://www.w3.org/TR/css-grid-1/#track-sizing>, "Inherited:
    /// no"). Computed value: the keyword `none` or a computed track list
    /// ([`ComputedGridTemplateTracks`] doc).
    pub grid_template_columns: ComputedGridTemplateTracks,
    /// `grid-template-rows`. **non-inherited**, initial:
    /// [`ComputedGridTemplateTracks::None`]. Same shape as
    /// [`Self::grid_template_columns`].
    pub grid_template_rows: ComputedGridTemplateTracks,
    /// `grid-template-areas`. **non-inherited**, initial:
    /// [`GridTemplateAreasValue::None`] (CSS Grid Layout Module Level 1 §7.3
    /// <https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>,
    /// "Inherited: no"). Computed value: the keyword `none` or a list of
    /// string values — no length payload, so this field stays the
    /// specified-layer type ([`GridTemplateAreasValue`] doc).
    pub grid_template_areas: GridTemplateAreasValue,
    /// `grid-auto-columns`. **non-inherited**, initial: `auto` (CSS Grid
    /// Layout Module Level 1 §7.6
    /// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-columns>,
    /// "Inherited: no").
    pub grid_auto_columns: Arc<Vec<ComputedGridTrackSize>>,
    /// `grid-auto-rows`. **non-inherited**, initial: `auto`. Same shape as
    /// [`Self::grid_auto_columns`].
    pub grid_auto_rows: Arc<Vec<ComputedGridTrackSize>>,
    /// `grid-auto-flow`. **non-inherited**, initial: [`GridAutoFlowValue::Row`]
    /// (CSS Grid Layout Module Level 1 §7.7
    /// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-flow>,
    /// "Inherited: no"). Computed value = specified keyword(s) — no length
    /// payload.
    pub grid_auto_flow: GridAutoFlowValue,
    /// `grid-row-start`. **non-inherited**, initial: [`GridLineValue::Auto`]
    /// (CSS Grid Layout Module Level 1 §8.3
    /// <https://www.w3.org/TR/css-grid-1/#line-placement>, "Inherited:
    /// no"). Computed value: specified keyword, identifier, and/or integer
    /// — no length payload.
    pub grid_row_start: GridLineValue,
    /// `grid-row-end`. **non-inherited**, initial: [`GridLineValue::Auto`].
    /// Same shape as [`Self::grid_row_start`].
    pub grid_row_end: GridLineValue,
    /// `grid-column-start`. **non-inherited**, initial: [`GridLineValue::Auto`].
    /// Same shape as [`Self::grid_row_start`].
    pub grid_column_start: GridLineValue,
    /// `grid-column-end`. **non-inherited**, initial: [`GridLineValue::Auto`].
    /// Same shape as [`Self::grid_row_start`].
    pub grid_column_end: GridLineValue,
    /// `justify-items`. **non-inherited**, initial:
    /// [`SelfAlignmentValue::Normal`] (CSS Box Alignment Module Level 3 §7.1
    /// <https://www.w3.org/TR/css-align-3/#propdef-justify-items>,
    /// "Inherited: no". The "legacy is unsupported" section of
    /// [`crate::property::PropertyValue::JustifyItems`] explains why this
    /// crate's initial value differs from the spec's literal `legacy`). Computed value =
    /// specified keyword(s).
    pub justify_items: SelfAlignmentValue,
    /// `justify-self`. **non-inherited**, initial: [`AlignSelfValue::Auto`]
    /// (CSS Box Alignment Module Level 3 §6.1
    /// <https://www.w3.org/TR/css-align-3/#propdef-justify-self>,
    /// "Inherited: no").
    pub justify_self: AlignSelfValue,
    /// `orphans`. **inherited**, initial: `2` (CSS Fragmentation Module
    /// Level 3 §3.3 "Breaks Between Lines: orphans, widows"
    /// <https://www.w3.org/TR/css-break-3/#widows-orphans>, which
    /// supersedes CSS 2.1 §13.3.2's original definition of this property
    /// with the same grammar/initial/inheritance). Value: `<integer>`,
    /// computed value = specified integer. The spec restricts this to
    /// positive integers ("Negative values and zero are invalid and must
    /// cause the declaration to be ignored") — enforced at parse time
    /// ([`crate::property::PropertyValue::Orphans`] doc).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded/inherited value only — the
    /// minimum-line-count enforcement this property describes (keeping at
    /// least this many line boxes together before a fragmentation break) is
    /// a pagination/layout-time algorithm this crate does not implement.
    pub orphans: i32,
    /// `widows`. Same shape as [`Self::orphans`] — see that field's doc
    /// (CSS Fragmentation Module Level 3 §3.3, same propdef table, `<integer>`
    /// / initial `2` / inherited / positive-only). The only difference is
    /// which side of a fragmentation break the minimum line count applies to
    /// (after the break, vs. `orphans`'s before).
    pub widows: i32,
    /// `background-repeat`. **non-inherited**, initial:
    /// [`BackgroundRepeat`]`{x: Repeat, y: Repeat}` (CSS Backgrounds and
    /// Borders 3 §2.4 "Tiling Images: the background-repeat
    /// property" <https://www.w3.org/TR/css-backgrounds-3/#the-background-repeat>,
    /// "Value: `<repeat-style>#`", "Inherited: no"). Computed value =
    /// specified keyword pair (no length payload).
    pub background_repeat: BackgroundRepeat,
    /// `background-attachment`. **non-inherited**, initial:
    /// [`BackgroundAttachment::Scroll`] (CSS Backgrounds and Borders 3 §2.5
    /// "Affixing Images: the background-attachment property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#the-background-attachment>,
    /// "Value: `<attachment>#`", "Inherited: no").
    pub background_attachment: BackgroundAttachment,
    /// `background-clip`. **non-inherited**, initial:
    /// [`VisualBox::BorderBox`] (CSS Backgrounds and Borders 3 §2.7
    /// "Painting Area: the background-clip property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#the-background-clip>,
    /// "Value: `<visual-box>#`", "Initial: border-box", "Inherited: no" —
    /// sibling [`Self::background_origin`] has a *different* initial, see
    /// that field's doc).
    pub background_clip: VisualBox,
    /// `background-origin`. **non-inherited**, initial:
    /// [`VisualBox::PaddingBox`] (CSS Backgrounds and Borders 3 §2.8
    /// "Positioning Area: the background-origin property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#the-background-origin>,
    /// "Value: `<visual-box>#`", "Initial: padding-box", "Inherited: no" —
    /// sibling [`Self::background_clip`] has a *different* initial, see
    /// that field's doc).
    pub background_origin: VisualBox,
    /// `background-size`. **non-inherited**, initial:
    /// [`crate::property::BackgroundSize::Explicit`]`{width: Auto, height: Auto}` (CSS
    /// Backgrounds and Borders 3 §2.9 "Sizing Images: the
    /// background-size property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#the-background-size>,
    /// "Value: `<bg-size>#`", "Initial: auto", "Inherited: no"). Contains a
    /// `<length-percentage>` per axis, absolutized against this node's own
    /// font-size/line-height (`em`/`rem`/`lh` resolved to px, `%` passed
    /// through — same layering as [`Self::width`]).
    pub background_size: ComputedBackgroundSize,
    /// `background-position`. **non-inherited**, initial: `0% 0%` (CSS
    /// Backgrounds and Borders 3 §2.6 "Positioning the Background Image:
    /// the background-position property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#the-background-position>,
    /// "Value: `<bg-position>#`", "Inherited: no"). See
    /// [`crate::property::CssPositionOffset`] doc for why edge-relative
    /// offsets (`right 10px` etc.) stay symbolic through this layer too.
    pub background_position: ComputedCssPosition,
    /// `background-image`. **non-inherited**, initial: [`BackgroundImage::None`]
    /// (CSS Backgrounds and Borders 3 §2.3 "Image Sources: the
    /// background-image property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#the-background-image>,
    /// "Value: `<bg-image>#`", "Inherited: no"). This crate parses only a
    /// single layer (`<bg-image> = <image> | none`, `<image> = <url> |
    /// <gradient>` — both alternatives implemented, see [`BackgroundImage`]
    /// doc's scope-carving section for what's deferred within `<gradient>`
    /// itself); comma-separated multi-layer `#` support is a follow-up.
    /// Per CSS Values and Units 4 §4.5.1, a `<url>`'s computed value is
    /// technically the *resolved absolute URL*, not the specified text
    /// verbatim — this crate holds the raw `String` unresolved and defers
    /// resolution to the runtime consumer, the same boundary
    /// [`crate::property::PropertyValue::Content`]'s `Image` component
    /// documents (raikiri-style doesn't depend on the `url` crate).
    /// `Gradient(..)`'s `<length-percentage>` payloads are partially
    /// absolutized (font-relative lengths → `Px`, `<percentage>` stays
    /// symbolic — `resolve_background_image` doc) while `<angle>` stays
    /// unresolved; the `<percentage>` half still needs the gradient box's
    /// own dimensions (paint/used-value layer, see
    /// [`crate::specified::SpecifiedValues::background_image`] doc).
    pub background_image: BackgroundImage,
    /// `object-fit`. **non-inherited**, initial: [`ObjectFit::Fill`] (CSS
    /// Images Module Level 3 §5.1 "Sizing the replaced element: the
    /// object-fit property"
    /// <https://www.w3.org/TR/css-images-3/#the-object-fit>, "Value: `fill |
    /// contain | cover | none | scale-down`", "Inherited: no"). Computed
    /// value = specified keyword.
    pub object_fit: ObjectFit,
    /// `object-position`. **non-inherited**, initial: `50% 50%` (CSS Images
    /// Module Level 3 §5.2 "Positioning the replaced element: the
    /// object-position property"
    /// <https://www.w3.org/TR/css-images-3/#the-object-position>, "Value:
    /// `<position>`", "Inherited: no"). Computed value = "as for
    /// background-position" per that same propdef table — this crate reuses
    /// [`crate::property::CssPosition`]/[`ComputedCssPosition`] verbatim
    /// ([`crate::property::CssPosition`] doc's reuse note). Percentages
    /// resolve against the replaced element's own content box at used-value
    /// time (a layout-time input this crate's cascade/computed layer does
    /// not have), so — same as [`Self::background_position`] — the
    /// `<length-percentage>` payload is absolutized against font-size/
    /// line-height only, not against a box size.
    pub object_position: ComputedCssPosition,
    /// `opacity`. **non-inherited**, initial: `1` (CSS Color 4 §3.3
    /// "Transparency: the opacity property"
    /// <https://www.w3.org/TR/css-color-4/#transparency>, "Value:
    /// `<opacity-value>`", "Inherited: no"). Grammar: `<opacity-value> =
    /// <number> | <percentage>`.
    ///
    /// # Specified preserves, computed clamps
    ///
    /// Same §, verbatim: "Opacity values outside the range \[0, 1\] are not
    /// invalid, and are preserved in specified values, but are clamped to
    /// the range \[0, 1\] in computed values." A value produced by the
    /// ordinary parse -> cascade pipeline through this field always lands
    /// in `[0.0, 1.0]`: this crate's `property` module numeric-token
    /// acquisition (`expect_number_stable`/`expect_percentage_stable`)
    /// corrects the one cssparser tokenizer artifact that could otherwise
    /// produce a NaN parse (a huge-exponent, zero-mantissa literal like
    /// `opacity: 0e999`), and `parse_opacity_value`'s `!is_nan()` guard
    /// remains on top as defense-in-depth; the `[0, 1]` clamp for every
    /// other out-of-range value (including `+Inf`/`-Inf`) happens in
    /// [`crate::specified::SpecifiedValues::absolutize_with`] (phase 3) and
    /// its page-context sibling in [`crate::page`]; the corresponding
    /// specified-layer field
    /// ([`crate::specified::SpecifiedValues::opacity`]) is the one that
    /// preserves an out-of-range parse (still NaN-free), per
    /// [`crate::property::PropertyValue::Opacity`]'s doc.
    ///
    /// This is **not** a type-level invariant this public field enforces
    /// against direct construction — [`Self`] has no private state guarding
    /// it, so code in this crate (or, via a future non-`#[non_exhaustive]`
    /// bump, outside it) that builds a `ComputedValues` by struct literal
    /// and assigns this field directly (bypassing `finalize`/
    /// `absolutize_in_page_context`) can still put a NaN or out-of-range
    /// `f32` here; it only describes what the pipeline itself guarantees.
    ///
    /// The actual alpha-blend application of this value against a node's
    /// paint output is out of this crate's scope — `raikiri-paint`
    /// consumes it as plain data.
    pub opacity: f32,
    /// `isolation`. **non-inherited**, initial: [`Isolation::Auto`] (CSS
    /// Compositing and Blending Level 1 §3.4.2 "Isolation: the isolation
    /// property" <https://www.w3.org/TR/compositing-1/#isolation>). Always
    /// a keyword — no phase-3 transform, identity pass-through from
    /// [`crate::specified::SpecifiedValues::isolation`] (same shape as
    /// [`Self::object_fit`]).
    pub isolation: Isolation,
    /// `mix-blend-mode`. **non-inherited**, initial:
    /// [`MixBlendMode::Normal`] (CSS Compositing and Blending Level 1
    /// §3.4.1 "Mix Blend Mode: the mix-blend-mode property"
    /// <https://www.w3.org/TR/compositing-1/#mix-blend-mode>). Same shape
    /// as [`Self::isolation`] above — actual blending compositing is
    /// `raikiri-paint`'s responsibility, this field is plain data.
    pub mix_blend_mode: MixBlendMode,
    /// `mask-image`. **non-inherited**, initial: [`MaskImage::None`] (CSS
    /// Masking Level 1 §7.1 "Image Masking: the mask-image property"
    /// <https://www.w3.org/TR/css-masking-1/#the-mask-image>). Identity
    /// pass-through from [`crate::specified::SpecifiedValues::mask_image`]
    /// — this crate never absolutizes a `<gradient>` payload's embedded
    /// lengths, same reasoning as [`Self::background_image`].
    pub mask_image: MaskImage,
    /// `clip-path`. **non-inherited**, initial: [`ClipPath::None`] (CSS
    /// Masking Level 1 §5.1 "Basic Shapes: the clip-path property"
    /// <https://www.w3.org/TR/css-masking-1/#the-clip-path>). Same shape
    /// as [`Self::mask_image`] above.
    pub clip_path: ClipPath,
    /// `transform`. **non-inherited**, initial: `none` (empty list,
    /// [`crate::property::empty_transform_list`]) (CSS Transforms Level 1 §4 "The transform
    /// property" <https://www.w3.org/TR/css-transforms-1/#transform-property>).
    /// Computed value is "as specified, but with lengths made absolute" — the
    /// length half of each `<length-percentage>` slot (`translate()`/
    /// `translateX()`/`translateY()`'s [`crate::property::Length`] payload) is
    /// absolutized against font-size/root-font-size while leaving
    /// [`crate::property::Length::Percent`] symbolic
    /// ([`crate::resolve::ComputedLengthPercentage::Percent`]) for a later
    /// box-size-relative resolution. Same split as
    /// [`crate::resolve::resolve_css_position`] for
    /// `background-position`/`object-position`.
    pub transform: Arc<Vec<ComputedTransformFunction>>,
    /// Border-box origin with absolute lengths and symbolic percentages.
    pub transform_origin: ComputedCssPosition,
    /// Absolute Z origin (unused by 2D rendering).
    pub transform_origin_z: ComputedLength,
    /// `filter`. **non-inherited**, initial: `none` (empty list,
    /// [`empty_filter_list`]) (CSS Filter Effects Level 1 §5 "The filter
    /// property" <https://www.w3.org/TR/filter-effects-1/#FilterProperty>,
    /// whose own Computed value is "as specified" — unlike
    /// [`Self::transform`] above, identity pass-through here is this
    /// property's actual, spec-correct computed-value definition, not an
    /// implementation gap). Same `Arc<Vec<..>>` payload shape as
    /// [`Self::transform`] otherwise.
    pub filter: Arc<Vec<FilterFunction>>,
    /// `table-layout`. **non-inherited**, initial:
    /// [`TableLayoutValue::Auto`] (CSS Tables 3 §4 "Table Layout Algorithm"
    /// <https://www.w3.org/TR/css-tables-3/#table-layout-property>,
    /// "Initial: auto" / "Inherited: no"). Computed value = specified
    /// keyword ([`TableLayoutValue`] doc — no length payload, so no relative
    /// resolution is needed).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. The fixed/auto column
    /// sizing algorithm it drives is layout-time behavior (raikiri-dom
    /// scope — the downstream table engine reads this field via the `Node`
    /// bridge) — same split [`Self::float`] doc describes for float
    /// positioning.
    pub table_layout: TableLayoutValue,
    /// `text-overflow`. **non-inherited**, initial: [`TextOverflowValue::Clip`]
    /// (CSS Overflow 3 §5.1). Computed value = specified keyword. Inline
    /// layout reads it on block containers whose `overflow` is not `visible`.
    pub text_overflow: TextOverflowValue,
    /// `border-collapse`. **inherited**, initial:
    /// [`BorderCollapseValue::Separate`] (CSS Tables 3 §6 "Borders"
    /// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>,
    /// "Initial: separate" / "Inherited: yes"). Computed value = specified
    /// keyword ([`BorderCollapseValue`] doc — no length payload).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. Border conflict
    /// resolution (collapsing borders model) is layout-time behavior
    /// (raikiri-dom scope) — [`Self::table_layout`] doc's split applies
    /// here as well.
    pub border_collapse: BorderCollapseValue,
    /// `border-spacing`. **inherited**, initial: `0` (both axes `0px`)
    /// (CSS Tables 3 §6.1 "Separated borders: the border-spacing property"
    /// <https://www.w3.org/TR/css-tables-3/#border-spacing-property>,
    /// "Initial: 0" / "Inherited: yes"). Computed value = two absolute
    /// lengths ([`ComputedBorderSpacing`] doc — phase 3 resolves each
    /// axis against the node's own font metrics).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded + absolutized value only. Whether
    /// cell separation actually gaps the table grid at layout time is
    /// layout-time behavior (raikiri-dom scope — parse + cascade + compute
    /// only in this task, no table layout integration) — [`Self::table_layout`]
    /// doc's split applies here as well.
    pub border_spacing: ComputedBorderSpacing,
    /// `caption-side`. **inherited**, initial:
    /// [`CaptionSideValue::Top`] (CSS Tables 3 §7 "Caption Position"
    /// <https://www.w3.org/TR/css-tables-3/#caption-side-property>,
    /// "Initial: top" / "Inherited: yes"). Computed value = specified
    /// keyword ([`CaptionSideValue`] doc — no length payload).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. Caption box placement
    /// is layout-time behavior (raikiri-dom scope) — [`Self::table_layout`]
    /// doc's split applies here as well.
    pub caption_side: CaptionSideValue,
    /// `empty-cells`. **inherited**, initial:
    /// [`EmptyCellsValue::Show`] (CSS Tables 3 §8 "Empty Cells"
    /// <https://www.w3.org/TR/css-tables-3/#empty-cells-property>,
    /// "Initial: show" / "Inherited: yes"). Computed value = specified
    /// keyword ([`EmptyCellsValue`] doc — no length payload).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. Empty-cell border /
    /// background painting is layout/paint-time behavior (raikiri-dom /
    /// raikiri-paint scope) — [`Self::table_layout`] doc's split applies
    /// here as well.
    pub empty_cells: EmptyCellsValue,
    /// `column-count` — non-inherited multicol container setting.
    pub column_count: ColumnCountValue,
    /// `column-fill` — non-inherited, initial `balance` (CSS Multi-column
    /// Layout Module Level 1 §7.1
    /// <https://www.w3.org/TR/css-multicol-1/#propdef-column-fill>).
    pub column_fill: ColumnFillValue,
    /// Computed `column-width` — non-inherited multicol container setting.
    pub column_width: ComputedColumnWidth,
    /// Resolved custom properties for the page-context inheritance bridge.
    ///
    /// This is deliberately crate-private: `ComputedValues`' public property
    /// bag remains unchanged, while `cascade_page` can faithfully implement
    /// CSS Variables' inherited custom-property semantics without adding a
    /// public API or a second root-style argument.
    pub(crate) custom_properties: Arc<CustomPropertyEnvironment>,
    /// Resolved custom properties declared on this node only. This is kept
    /// separately because nodes without declarations share their inherited
    /// environment for performance.
    pub(crate) local_custom_properties: Arc<CustomPropertyEnvironment>,
}

impl ComputedValues {
    /// Return an effective resolved CSS custom-property value as an owned
    /// string.  This accessor deliberately exposes only primitive text, not
    /// the style engine's internal property environment.
    pub fn resolved_custom_property(&self, name: &str) -> Option<String> {
        self.custom_properties
            .get(name)
            .map(|value| value.to_string())
    }

    /// Return a custom-property value declared locally on this node.
    pub fn local_resolved_custom_property(&self, name: &str) -> Option<String> {
        self.local_custom_properties
            .get_local(name)
            .map(|value| value.to_string())
    }

    /// Initial values according to the CSS spec. Used for a root node with no
    /// matching cascade declarations and to terminate the inheritance chain.
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            // CSS Backgrounds 3 §2.2: initial background-color is `transparent`.
            background_color: CssColor::TRANSPARENT,
            background_color_expression: None,
            // Shared Arc slot avoids a per-node allocation (see the
            // documentation for `initial_font_family`).
            font_family: initial_font_family(),
            font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
            font_weight: 400.0,
            // CSS Inline 3 §5.1: initial line-height is `normal`; paint resolves
            // it using font metrics equivalent to ascent + descent.
            line_height: ComputedLineHeight::Normal,
            display: DisplayValue::Inline,
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            list_style_image: BackgroundImage::None,
            // CSS Lists 3 §4: the spec initializes counter-* to `none`, here
            // represented by an empty list (see the field documentation).
            // A shared empty Arc avoids per-node allocations (see the
            // documentation for `empty_counter_entries` in property.rs).
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // CSS Content 3 §1: the spec initializes content to `normal`. Here
            // an empty list means "no generated content" to downstream code.
            // A shared empty Arc avoids per-node allocations (see the
            // documentation for `empty_content_list` in property.rs).
            content: empty_content_list(),
            // CSS GCPM 3 §1.1.1: the spec initializes string-set to `none`,
            // represented here by an empty list. Use the same shared-empty-Arc pattern.
            string_set: empty_string_set_entries(),
            // CSS GCPM 3 §1.2.1: the initial position is `static`, so there
            // is no running(name) seed; the initial list is empty.
            running_templates: Vec::new(),
            position: PositionValue::Static,
            // CSS Text 3 §6.1: text-align initial is `start`
            text_align: TextAlign::Start,
            // CSS Text 3 §8.2.1: hanging-punctuation initial is `none`.
            hanging_punctuation: HangingPunctuation::None,
            // CSS Text 4: text-autospace initial is `normal`.
            text_autospace: TextAutospace::Normal,
            // CSS Text 4: word-space-transform initial is `none`.
            word_space_transform: WordSpaceTransform::None,
            // CSS Text 4: text-spacing-trim initial is `normal`.
            text_spacing_trim: TextSpacingTrim::Normal,
            // CSS Text 3 §6.2: text-justify initial is `auto`.
            text_justify: TextJustify::Auto,
            // CSS Text 3 §6.1: text-align-last initial is `auto`.
            text_align_last: TextAlignLast::Auto,
            // CSS Writing Modes 4 §2.1: initial direction is `ltr`.
            direction: Direction::Ltr,
            // CSS Writing Modes 4 §3.2: writing-mode initial is
            // `horizontal-tb` (`WritingMode` doc's Non-goal section — the
            // other 4 keywords never appear in this field regardless).
            writing_mode: WritingMode::HorizontalTb,
            cssom_writing_mode: WritingMode::HorizontalTb,
            ruby_position: RubyPosition::Over,
            // CSS Text 3 §8.1: initial text-indent is `0`.
            text_indent: ComputedTextIndent::Px(0.0),
            text_indent_ch_factor: None,
            text_indent_ch_offset: 0.0,
            text_indent_ch_font: None,
            text_indent_ch_inherited: false,
            text_indent_hanging: false,
            text_indent_each_line: false,
            // CSS Box 3 §4.1: initial padding is 0 on all four sides.
            padding: Sides::all(ComputedLengthPercentage::Px(0.0)),
            padding_ch: Sides::all(None),
            // CSS Box 3 §3.1: the physical margin-* properties initially have
            // value `0`, spread to all four sides by `Sides::all(0)`.
            margin: Sides::all(ComputedLengthPercentageOrAuto::Px(0.0)),
            margin_ch: Sides::all(None),
            // CSS Backgrounds 3 §3.3/§3.2/§3.1: on every side, the initial
            // border style is none and the color is the `currentcolor` keyword.
            // `BorderColor::CurrentColor` replaced a `CssColor::BLACK` placeholder
            // to match the CSS Backgrounds 3 §3.1 initial-value contract. Do not
            // confuse this §3.1 with the CSS Box 3 §3.1 margin rule above.
            //
            // The specified width is `medium` (3px), but the **computed width
            // is 0px**: §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
            // says "Computed value: … zero if the border style is `none` or
            // `hidden`". The gate is `crate::resolve::resolve_border`; the
            // specified initial value lives in
            // `crate::specified::SpecifiedValues::initial`.
            border: Sides::all(ComputedBorder {
                width: ComputedLength::ZERO,
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            }),
            // CSS Backgrounds and Borders 3 §5: border-radius initial is 0.
            border_radius: ComputedBorderRadius::all(ComputedLength::ZERO),
            // CSS Backgrounds and Borders 3 §6.1: box-shadow initial is none.
            box_shadow: empty_computed_box_shadow_list(),
            // CSS UI 3 §4.1/§4.2/§4.3/§4.4: outline specified initial values
            // are medium (3px), none, and invert. The computed width is 0px when
            // the computed style is none.
            outline: ComputedOutline {
                width: ComputedLength::ZERO,
                style: OutlineStyle::None,
                color: OutlineColor::Invert,
            },
            // CSS UI 3 §4.5: initial outline-offset is `0`.
            outline_offset: ComputedLength::ZERO,
            // CSS Sizing 3 §3.1.1: initial width is `auto`.
            width: ComputedLengthPercentageOrAuto::Auto,
            width_ch: None,
            // CSS Sizing 3 §3.1.1: initial height is `auto`.
            height: ComputedLengthPercentageOrAuto::Auto,
            height_ch: None,
            max_width: ComputedLengthPercentageOrAuto::Auto,
            max_width_ch: None,
            max_height: ComputedLengthPercentageOrAuto::Auto,
            max_height_ch: None,
            min_width: ComputedLengthPercentageOrAuto::Auto,
            min_width_ch: None,
            min_height: ComputedLengthPercentageOrAuto::Auto,
            min_height_ch: None,
            min_block_size: None,
            min_block_size_ch: None,
            vertical_logical_size: None,
            top: ComputedLengthPercentageOrAuto::Auto,
            right: ComputedLengthPercentageOrAuto::Auto,
            bottom: ComputedLengthPercentageOrAuto::Auto,
            left: ComputedLengthPercentageOrAuto::Auto,
            // CSS Sizing 3 §3.3: initial box-sizing is `content-box`.
            box_sizing: BoxSizing::ContentBox,
            // CSS Overflow 3 §3.1: both overflow axes initially are `visible`,
            // so cross-axis coupling (`resolve_overflow`) is a no-op initially.
            overflow: OverflowXY::both(OverflowValue::Visible),
            // CSS Text Decoration 3 §2.1/§2.2/§2.3: initial values are
            // `none`, `solid`, and `currentcolor`, respectively.
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_style: TextDecorationStyle::Solid,
            text_decoration_color: TextDecorationColor::CurrentColor,
            text_decoration_thickness: ComputedTextDecorationThickness::Auto,
            // CSS Text Decoration 4: text-decoration-skip-ink initial is `auto`.
            text_decoration_skip_ink: TextDecorationSkipInk::Auto,
            // CSS Text Decoration 4: text-decoration-skip-spaces initial is `start end`.
            text_decoration_skip_spaces: TextDecorationSkipSpaces::StartEnd,
            text_decoration_inset: ComputedTextDecorationInset::Lengths {
                start: ComputedLength::ZERO,
                end: ComputedLength::ZERO,
            },
            text_decoration_inset_start_ch: None,
            text_decoration_inset_end_ch: None,
            text_underline_offset: ComputedTextUnderlineOffset::Auto,
            text_underline_position: TextUnderlinePosition::AUTO,
            text_emphasis_position: TextEmphasisPosition::Position {
                vertical: TextEmphasisVEdge::Over,
                horizontal: Some(TextEmphasisHEdge::Right),
            },
            text_emphasis_style: TextEmphasisStyle::None,
            text_emphasis_color: TextDecorationColor::CurrentColor,
            // CSS 2.1 §10.8.1: initial vertical-align is `baseline`.
            vertical_align: VerticalAlign::Baseline,
            // CSS Fonts 4 §2.4: initial font-style is `normal`.
            font_style: FontStyle::Normal,
            font_kerning: FontKerning::Auto,
            font_optical_sizing: FontOpticalSizing::Auto,
            font_variant_emoji: FontVariantEmoji::Normal,
            font_language_override: FontLanguageOverride::Normal,
            font_variant_ligatures: FontVariantLigatures::Normal,
            font_synthesis: FontSynthesisValue::initial(),
            font_variant_position: FontVariantPosition::Normal,
            font_palette: FontPaletteValue::Normal,
            font_variant_numeric: FontVariantNumeric::initial(),
            font_variant_east_asian: FontVariantEastAsian::initial(),
            font_variation_settings: FontVariationSettings::Normal,
            font_feature_settings: FontFeatureSettings::Normal,
            // CSS Fonts 3 §6.6: initial font-variant-caps is `normal`.
            font_variant_caps: FontVariantCaps::Normal,
            // CSS Text 3 §2.1: initial text-transform is `none`.
            text_transform: TextTransform::None,
            // CSS Writing Modes 3 §9.1: text-combine-upright initial is `none`.
            text_combine_upright: TextCombineUpright::None,
            // CSS Writing Modes 3 §5.1: text-orientation initial is `mixed`.
            text_orientation: TextOrientation::Mixed,
            // CSS Writing Modes 3 §2.2: unicode-bidi initial is `normal`.
            unicode_bidi: UnicodeBidi::Normal,
            // CSS Display 3 §4: initial visibility is `visible`.
            visibility: Visibility::Visible,
            // CSS2 §9.9.1: initial z-index is `auto`.
            z_index: ZIndexValue::Auto,
            // CSS Text 3 §5.1: initial word-break is `normal`.
            word_break: WordBreak::Normal,
            // CSS Text 3 §5.3: line-break initial is `auto`.
            line_break: LineBreak::Auto,
            // CSS Text 3 §5.4: initial overflow-wrap is `normal`.
            overflow_wrap: OverflowWrap::Normal,
            // CSS Text 3 §7.2 / §7.1: initial `normal` letter-spacing and
            // word-spacing compute to `0` (`ComputedLength::ZERO`).
            letter_spacing: ComputedLength::ZERO,
            letter_spacing_computed: ComputedLetterSpacing::Px(0.0),
            letter_spacing_ch_factor: None,
            letter_spacing_ch_offset: 0.0,
            letter_spacing_ch_font: None,
            word_spacing: ComputedLength::ZERO,
            word_spacing_computed: ComputedWordSpacing::Px(0.0),
            word_spacing_ch_factor: None,
            word_spacing_ch_offset: 0.0,
            word_spacing_ch_font: None,
            // CSS Text 3 §4.2: initial tab-size is `8`.
            tab_size: ComputedTabSize::Number(8.0),
            // CSS Fragmentation 3 §3.1 / §3.2: break-before, break-after,
            // and break-inside all initially are `auto`.
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            // CSS2 §9.5.1 / §9.5.2: float and clear both initially are `none`.
            float: FloatValue::None,
            clear: ClearValue::None,
            // CSS Text 3 §3: initial white-space is `normal`.
            white_space: WhiteSpace::Normal,
            // CSS Text 4: white-space-collapse initial is `collapse`.
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            text_wrap: TextWrapMode::Wrap,
            text_wrap_style: TextWrapStyle::Auto,
            effective_white_space_collapse: WhiteSpaceCollapse::Collapse,
            effective_text_wrap_mode: TextWrapMode::Wrap,
            // CSS Text 3 §5.3: initial hyphens is `manual`.
            hyphens: Hyphens::Manual,
            // CSS Text 4: hyphenate-character initial is `auto`.
            hyphenate_character: HyphenateCharacter::Auto,
            // CSS Text 4: hyphenate-limit-chars initial is `auto`.
            hyphenate_limit_chars: HyphenateLimitChars::INITIAL,
            // CSS Flexbox 1 §5.1: initial flex-direction is `row`.
            flex_direction: FlexDirectionValue::Row,
            // CSS Flexbox 1 §5.2: initial flex-wrap is `nowrap`.
            flex_wrap: FlexWrapValue::NoWrap,
            // CSS Flexbox 1 §7.2.1/§7.2.2: initial flex-grow is `0`;
            // initial flex-shrink is `1`.
            flex_grow: 0.0,
            flex_shrink: 1.0,
            // CSS Flexbox 1 §7.2.3: initial flex-basis is `auto`.
            flex_basis: ComputedFlexBasis::Auto,
            // CSS Flexbox 1 §4.2: initial order is `0`.
            order: 0,
            // CSS Box Alignment 3 §5.1: justify-content and align-content
            // both initially are `normal`.
            justify_content: ContentAlignmentValue::Normal,
            align_content: ContentAlignmentValue::Normal,
            // CSS Box Alignment 3 §7.2: initial align-items is `normal`.
            align_items: SelfAlignmentValue::Normal,
            // CSS Box Alignment 3 §6.2: initial align-self is `auto`.
            align_self: AlignSelfValue::Auto,
            // CSS Box Alignment 3 §8.1: row-gap and column-gap
            // both initially are `normal`.
            row_gap: ComputedLengthPercentageOrNormal::Normal,
            column_gap: ComputedLengthPercentageOrNormal::Normal,
            // CSS Content 3 §2.4.1 leaves the initial quotes value to the
            // user agent. This independent implementation represents it with an
            // empty list (the same shared empty Arc slot as `none`; see the
            // documentation for `Self::quotes` and `empty_quotes_entries`).
            quotes: empty_quotes_entries(),
            quotes_auto: true,
            // CSS Text Decoration 3 §4: initial text-shadow is `none`,
            // using a shared empty Arc slot (see the documentation for
            // `empty_computed_text_shadow_list`).
            text_shadow: empty_computed_text_shadow_list(),
            // CSS Grid Layout 1 §7.2/§7.3: both grid-template-* properties
            // initially are `none`.
            grid_template_columns: ComputedGridTemplateTracks::None,
            grid_template_rows: ComputedGridTemplateTracks::None,
            grid_template_areas: GridTemplateAreasValue::None,
            // CSS Grid Layout 1 §7.6: grid-auto-columns and grid-auto-rows
            // both initially are `auto`.
            grid_auto_columns: initial_computed_grid_auto_track_list(),
            grid_auto_rows: initial_computed_grid_auto_track_list(),
            // CSS Grid Layout 1 §7.7: initial grid-auto-flow is `row`.
            grid_auto_flow: GridAutoFlowValue::Row,
            // CSS Grid Layout 1 §8.3: grid-row-start/-end and
            // grid-column-start/-end all initially are `auto`.
            grid_row_start: GridLineValue::Auto,
            grid_row_end: GridLineValue::Auto,
            grid_column_start: GridLineValue::Auto,
            grid_column_end: GridLineValue::Auto,
            // CSS Box Alignment 3 §7.1/§6.1: initial justify-items and
            // justify-self (see the "legacy is unsupported" section in the
            // documentation for `crate::property::PropertyValue::JustifyItems`;
            // justify-self initially is `auto`).
            justify_items: SelfAlignmentValue::Normal,
            justify_self: AlignSelfValue::Auto,
            // CSS Fragmentation 3 §3.3: orphans and widows both initially
            // are `2`.
            orphans: 2,
            widows: 2,
            // CSS Backgrounds and Borders 3 §2.4–§2.9: spec initial
            // values (see the field documentation for `Self::background_repeat`
            // and others; `background_clip` and `background_origin` have
            // different initial values).
            background_repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            },
            background_attachment: BackgroundAttachment::Scroll,
            background_clip: VisualBox::BorderBox,
            background_origin: VisualBox::PaddingBox,
            background_size: ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Auto,
                height: ComputedLengthPercentageOrAuto::Auto,
            },
            background_position: ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    0.0,
                )),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(0.0)),
            },
            // CSS Backgrounds and Borders 3 §2.3: initial background-image
            // is `none`.
            background_image: BackgroundImage::None,
            // CSS Images 3 §5.1: initial object-fit is `fill`.
            object_fit: ObjectFit::Fill,
            // CSS Images 3 §5.2: initial object-position is `50% 50%`,
            // unlike the `0% 0%` initial background-position.
            object_position: ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    50.0,
                )),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(50.0)),
            },
            // CSS Color 4 §3.3: initial opacity is `1`.
            opacity: 1.0,
            // CSS Compositing and Blending 1 §3.4.2: initial isolation
            // is `auto`.
            isolation: Isolation::Auto,
            // CSS Compositing and Blending 1 §3.4.1: initial
            // mix-blend-mode is `normal`.
            mix_blend_mode: MixBlendMode::Normal,
            // CSS Masking 1 §7.1: initial mask-image is `none`.
            mask_image: MaskImage::None,
            // CSS Masking 1 §5.1: initial clip-path is `none`.
            clip_path: ClipPath::None,
            // CSS Transforms 1 §4: initial transform is `none` (empty list).
            transform: empty_computed_transform_list(),
            transform_origin: ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    50.0,
                )),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(50.0)),
            },
            transform_origin_z: ComputedLength(0.0),
            // CSS Filter Effects Level 1 §5: initial filter is `none`
            // (an empty list).
            filter: empty_filter_list(),
            // CSS Tables 3 §4: initial table-layout is `auto`
            // (not inherited).
            table_layout: TableLayoutValue::Auto,
            // CSS Overflow 3 §5.1: initial text-overflow is `clip` (not inherited).
            text_overflow: TextOverflowValue::Clip,
            // CSS Tables 3 §6: initial border-collapse is `separate`
            // (inherited; used to seed the root).
            border_collapse: BorderCollapseValue::Separate,
            // CSS Tables 3 §6.1: initial border-spacing is `0`
            // (inherited; seeds the root at 0px on both axes).
            border_spacing: ComputedBorderSpacing {
                horizontal: ComputedLength::ZERO,
                vertical: ComputedLength::ZERO,
            },
            // CSS Tables 3 §7: initial caption-side is `top`
            // (inherited; used to seed the root).
            caption_side: CaptionSideValue::Top,
            // CSS Tables 3 §8: initial empty-cells is `show`
            // (inherited; used to seed the root).
            empty_cells: EmptyCellsValue::Show,
            // CSS Multi-column Layout 1: initial column-count and column-width are `auto`;
            // column-fill is `balance`.
            column_count: ColumnCountValue::Auto,
            column_fill: ColumnFillValue::Balance,
            column_width: ComputedColumnWidth::Auto,
            custom_properties: empty_custom_properties(),
            local_custom_properties: empty_custom_properties(),
        }
    }

    /// Construct a child's computed values when it has no declarations, using
    /// the parent's computed values.
    ///
    /// - Copy **inherited** properties from the parent.
    /// - Keep **non-inherited** properties at their `initial()` values.
    ///
    /// The field documentation on [`Self`] is the canonical source for each
    /// property's inheritance behavior (currently inherited: color / font-family / font-size / font-weight / text_align / hanging_punctuation / direction / writing_mode / cssom_writing_mode / line_height / font_style / font_kerning / font_optical_sizing / font_variant_emoji / font_language_override / font_variant_ligatures / font_synthesis / font_variant_position / font_palette / font_variant_numeric / font_variant_east_asian / font_variation_settings / font_feature_settings / font_variant_caps / text_transform / text_combine_upright / text_orientation / visibility / text_indent / word_break / overflow_wrap / letter_spacing / word_spacing / white_space / white_space_collapse / text_wrap_style / hyphens / hyphenate_character / hyphenate_limit_chars / tab_size / quotes / text_shadow / text_underline_offset / orphans / widows / border_collapse / border_spacing / caption_side / empty_cells;
    /// non-inherited: background-color / display / counter-* / content /
    /// string-set / running_templates / padding / margin / border / border_radius / box_shadow / outline / width / height / box_sizing / overflow / text_decoration_line / text_decoration_style / text_decoration_color / text_decoration_inset / unicode_bidi / vertical_align / z_index / break_before / break_after / break_inside / background_repeat / background_attachment / background_clip / background_origin / background_size / background_position / background_image / object_fit / object_position / opacity / isolation / mix_blend_mode / mask_image / clip_path / transform / filter / table_layout / column_count / column_fill / column_width).
    ///
    /// The inherited/non-inherited classification is defined by each field's
    /// documentation. Inherited fields are copied from the parent's computed
    /// values; non-inherited fields retain their initial values.
    ///
    pub fn inherit_from(parent: &Self) -> Self {
        let mut child = crate::specified::SpecifiedValues::inherit_from(parent).finalize(
            parent,
            &crate::resolve::ResolveContext::new(parent.font_size),
        );
        // CSS Variables 1 §2: custom properties are inherited. `finalize`
        // starts with the ordinary computed initial state, so carry this
        // crate-private bridge explicitly through the public helper too.
        child.custom_properties = parent.custom_properties.clone();
        child.local_custom_properties = empty_custom_properties();
        child.quotes_auto = parent.quotes_auto;
        child
    }

    /// Inherit marker text styles, including the CSS Lists 3 §3.1.1 UA defaults.
    /// The cascade applies author declarations after these defaults.
    pub fn inherit_marker_from(parent: &Self) -> Self {
        let mut marker = crate::specified::SpecifiedValues::inherit_marker_from(parent).finalize(
            parent,
            &crate::resolve::ResolveContext::new(parent.font_size),
        );
        marker.custom_properties = parent.custom_properties.clone();
        marker.quotes_auto = parent.quotes_auto;
        marker
    }
}

mod cssom;
pub use cssom::ComputedProperty;

#[cfg(test)]
mod tests;
