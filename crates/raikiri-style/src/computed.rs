//! Per-node computed CSS values.
//!
//! Cascade + inheritance walk が populate。将来 ComputedValues → taffy::Style
//! + paint 用色情報の抽出 layer が入る予定。
//!
//! 現サポート property の一覧と inherited / non-inherited 分類は
//! [`ComputedValues`] 定義の field doc comment を参照。

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use smol_str::SmolStr;

use crate::Atom;
use crate::property::{
    AlignSelfValue, BackgroundAttachment, BackgroundImage, BackgroundRepeat,
    BackgroundRepeatKeyword, BorderCollapseValue, BorderColor, BorderStyle, BoxSizing,
    BreakBetween, BreakInside, CaptionSideValue, ClearValue, ClipPath, ColumnCountValue,
    ContentAlignmentValue, ContentComponent, CssColor, Direction, DisplayValue, EmptyCellsValue,
    FilterFunction, FlexDirectionValue, FlexWrapValue, FloatValue, FontKerning,
    FontLanguageOverride, FontOpticalSizing, FontPaletteValue, FontStyle, FontSynthesisValue,
    FontVariantCaps, FontVariantEastAsian, FontVariantEmoji, FontVariantLigatures,
    FontVariantNumeric, FontVariantPosition, FontVariationSettings, GridAutoFlowValue,
    GridLineValue, GridTemplateAreasValue, HangingPunctuation, HyphenateCharacter,
    HyphenateLimitChars, Hyphens, Isolation, LineBreak, ListStylePosition, ListStyleType,
    MaskImage, MixBlendMode, ObjectFit, OutlineColor, OutlineStyle, OverflowValue, OverflowWrap,
    OverflowXY, PositionValue, RubyPosition, SelfAlignmentValue, Sides, TableLayoutValue,
    TextAlign, TextAlignLast, TextAutospace, TextCombineUpright, TextDecorationColor,
    TextDecorationLine, TextDecorationSkipInk, TextDecorationSkipSpaces, TextDecorationStyle,
    TextEmphasisHEdge, TextEmphasisPosition, TextEmphasisStyle, TextEmphasisVEdge, TextJustify,
    TextOrientation, TextSpacingTrim, TextTransform, TextUnderlinePosition, TextWrapMode,
    TextWrapStyle, UnicodeBidi, VerticalAlign, Visibility, VisualBox, WhiteSpace,
    WhiteSpaceCollapse, WordBreak, WordSpaceTransform, WritingMode, ZIndexValue,
    empty_content_list, empty_counter_entries, empty_filter_list, empty_quotes_entries,
    empty_string_set_entries, initial_font_family,
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

/// CSS spec 上の `font-size` initial value (`medium`) に対応する px 値。
///
/// CSS Fonts 4 §2.5 "Font size: the font-size property"
/// (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>) は "Initial: medium"
/// と規定し、`medium` の実 px は UA 依存。本実装は browser default の 16px を
/// 採る。
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

/// `position: running(<custom-ident>)` により登録された template の cascade-time seed。
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// された element は body flow から除去、`@page` margin box の
/// `content: element(name)` から参照される。
///
/// 本 struct は design doc §7.3 の **2-tier キャッシュ** の static side seed —
/// cascade で per-node に `name` を捕捉し、下流 (raikiri-dom) 側が subtree_root /
/// pre-cascaded style / dynamic flags を association する
/// (`ParsedRunningTemplate` — 本 crate は leaf、DOM node identity を持たない)。
///
/// `#[non_exhaustive]` により future field (e.g. `alternative_hint` 等の per-name
/// override) を non-breaking で追加可能。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningTemplate {
    /// `running(<name>)` の name (case-preserved smol str)。
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
    pub family: Arc<Vec<Atom>>,
    /// Computed font size in CSS px.
    pub size: ComputedLength,
    /// Computed numeric font weight.
    pub weight: f32,
    /// Computed font style.
    pub style: FontStyle,
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

/// Per-node computed style。現サポート property と inheritance 分類は下記 field
/// doc を参照 (inherited: color / font-family / font-size / font-weight / text_align / hanging_punctuation / direction / writing_mode / cssom_writing_mode / line_height / font_style / font_kerning / font_optical_sizing / font_variant_emoji / font_language_override / font_variant_ligatures / font_synthesis / font_variant_position / font_palette / font_variant_numeric / font_variant_east_asian / font_variation_settings / font_variant_caps / text_transform / text_combine_upright / text_orientation / visibility / text_indent / word_break / overflow_wrap / letter_spacing / word_spacing / white_space / white_space_collapse / text_wrap_style / hyphens / hyphenate_character / hyphenate_limit_chars / tab_size / quotes / text_shadow / orphans / widows / list_style_type / list_style_position、
/// non-inherited: background-color / display / counter-* / content / string-set /
/// running_templates / padding / margin / border / border_radius / box_shadow / outline / width / height / box_sizing /
/// overflow / text_decoration / unicode_bidi / vertical_align / z_index / float / clear)。
///
/// # 層
///
/// 本 struct が保持するのは **computed value 層**の値だけである。length を運ぶ
/// field は [`crate::resolve`] の `Computed*` 型で、`em` / `rem` / `pt` は既に
/// px へ絶対化されている (CSS Cascade 5 §7.2
/// <https://www.w3.org/TR/css-cascade-5/#inheriting> が「inheritance が運ぶのは
/// computed value である」と規定するため、この絶対化は inheritance より前に
/// 済んでいなければならない)。`<percentage>` は property ごとに扱いが異なる —
/// 各 field doc を参照。
///
/// 絶対化前の staging 表現は [`crate::specified::SpecifiedValues`]。cascade
/// winner の適用はそちらに対して行い、[`SpecifiedValues::finalize`] が本 struct
/// を produce する。
///
/// [`SpecifiedValues::finalize`]: crate::specified::SpecifiedValues::finalize
///
/// `#[non_exhaustive]` により future property の追加が non-breaking。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedValues {
    /// `color`。inherited、initial: opaque black。
    pub color: CssColor,
    /// `background-color`。**non-inherited**、initial: `transparent`
    /// (= [`CssColor::TRANSPARENT`])。CSS Backgrounds 3 §2.2 "Base Color:
    /// the background-color property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#background-color>。
    pub background_color: CssColor,
    /// `font-family` — 優先順位順。inherited。CSS Fonts 4 §2.1
    /// <https://www.w3.org/TR/css-fonts-4/#font-family-prop> の spec 上の
    /// initial は "depends on user agent" — spec は具体的な family name を
    /// 規定しない ([`crate::property::initial_font_family`] doc 参照)。
    /// 本実装は `[Atom::from("serif")]` を採る。
    ///
    pub font_family: Arc<Vec<Atom>>,
    /// `font-size`。**inherited**、initial: 16px (spec は `medium`、実 px は
    /// UA 依存)。CSS Fonts 4 §2.5 "Font size: the font-size property"
    /// <https://www.w3.org/TR/css-fonts-4/#propdef-font-size> は
    /// "Computed value: an absolute length" と規定する。
    ///
    /// 型は [`ComputedLength`] — `em` (親基準) / `rem` (root 基準) / `<percentage>`
    /// (親基準) / `pt` は cascade の phase 2
    /// ([`crate::resolve::resolve_font_size`]) で px へ絶対化済み。
    pub font_size: ComputedLength,
    /// `font-weight`。inherited、initial: 400.0 (normal)。
    ///
    /// **常に resolve 済みの absolute weight** (`[1, 1000]`)。specified value 側の
    /// `bolder` / `lighter` sentinel ([`crate::property::FontWeightValue`]) は
    /// [`crate::cascade::apply_value`] が親の computed weight と CSS Fonts 4 §2.2
    /// の table から絶対値に解決してから書き込むため、この field に relative
    /// keyword が残ることはない。これは spec とも一致する — §2.2 の property
    /// table は `Computed value: a number, see below` と規定し、§2.2.1
    /// "Relative Weights" が "Specified values of `bolder` and `lighter`
    /// indicate weights relative to the weight of the parent element. The
    /// computed weight is calculated based on the inherited `font-weight`
    /// value" と規定している
    /// (<https://www.w3.org/TR/css-fonts-4/#relative-weights>)。
    ///
    /// 型は `f32` (旧 `u16` から格上げ) — §2.2.2 "Missing
    /// weights" <https://www.w3.org/TR/css-fonts-4/#missing-weights> "Fractional
    /// weights are valid" どおり computed value の fractional 精度を保持する。
    /// `u16` だった当時は整数化のため round-half-away-from-zero を要し、その丸めが
    /// §2.2.1 relative-weight table の行選択を変える 2 次被害を伴う既知
    /// divergence だった (本 field の型変更で解消)。
    ///
    /// `raikiri-dom` の layout はこの field を直接 `parley::FontWeight::new(f32)`
    /// に渡す (キャスト不要) —
    /// **渡す直前** に `sanitize_font_weight` (`crates/raikiri-dom/src/
    /// layout.rs`) が sink 境界 guard を掛ける。
    ///
    /// # `[1, 1000]` / finite は caller が維持する contract (本 field 自体に guard なし)
    ///
    /// 本 struct の field は全て `pub` であり、cascade を経由せず直接
    /// `ComputedValues { font_weight: ..., .. }` を構築することを妨げない
    /// (例: [`crate::page::cascade_page`] の継承元 root 引数)。`u16` だった頃は
    /// 非有限値がそもそも型で構成不可能だったが、`f32` 化
    /// でこの保証は「型」から「呼び出し元の値検証」に変わった — cascade を
    /// 経由する通常経路は `parse_font_weight` (private、
    /// [`crate::property::FontWeightValue`] の doc 参照) の `[1, 1000]` range
    /// guard により常に finite だが、直接構築はその guard を経ない。
    /// `NaN` / `±Inf` が渡った場合の [`crate::cascade::resolve_relative_weight`]
    /// (`bolder`/`lighter` 解決) の挙動は同関数の doc で characterize 済み。
    ///
    /// **本 field / `resolve_relative_weight` のどちらにも guard は追加しない**
    /// (guard は sink 境界に置く、resolve/computed 層の public surface は
    /// sanitize しない、という設計判断による — carve-out 2)。
    /// この field を直接読む他の consumer (raikiri-dom 以外、例:
    /// umbrella 経由の外部 consumer) は自分の sink 境界で同様の guard を
    /// 持つ責務を負う (同じ carve-out 2 の理屈)。
    pub font_weight: f32,
    /// `line-height`。**inherited**、initial: [`ComputedLineHeight::Normal`]。
    /// CSS Inline 3 §5.1 "Line Spacing: the line-height property"
    /// <https://www.w3.org/TR/css-inline-3/#line-height-property>。
    ///
    /// spec の "Computed value: the specified keyword, a number, or a computed
    /// `<length>` value" に 1:1 対応する 3 variant
    /// ([`ComputedLineHeight`])。**computed 層に percentage は存在しない** —
    /// `<percentage>` は宣言要素の computed font-size に対して cascade の
    /// phase 3 で絶対化される (§5.1 "Percentages: computed relative to 1em")。
    ///
    /// [`ComputedLineHeight::Number`] (unitless multiplier) は computed 層でも
    /// number のまま残る。これは §5.1 の "child inherits the specified value"
    /// special behavior に対応し、child の font-size で再乗算する責務を下流
    /// (paint) に残す。逆に
    /// [`ComputedLineHeight::Length`] は宣言要素で確定した px であり、
    /// child は**再解決せずそのまま継承する**。
    pub line_height: ComputedLineHeight,
    /// `display`。**non-inherited**、initial: `DisplayValue::Inline`
    /// (CSS Display 3 §2 <https://www.w3.org/TR/css-display-3/#propdef-display>)。
    /// 現状 18 keyword (`block` / `inline` / `inline-block` / `none` / `flex`
    /// / `grid` / `list-item` / `contents` / `table` / `inline-table`
    /// / `table-row-group` / `table-header-group` / `table-footer-group`
    /// / `table-row` / `table-column-group` / `table-column` / `table-cell`
    /// / `table-caption` — 詳細は [`DisplayValue`] doc)。
    pub display: DisplayValue,
    /// `list-style-type`。**inherited**、initial: `disc` (CSS Lists 3 §3.1)。
    pub list_style_type: ListStyleType,
    /// `list-style-position`。**inherited**、initial: `outside` (CSS Lists 3 §3.2)。
    pub list_style_position: ListStylePosition,
    /// `list-style-image`。**inherited**、initial: `none` (CSS Lists 3 §3.3)。
    pub list_style_image: BackgroundImage,
    /// `counter-reset`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#counter-reset>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + initial value pairs。counter tree の実 resolve に向けた
    /// pre-work であり、resolve 本体は将来別途実装する。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`、`apply_value` move) と inheritance walk clone
    /// (`resolve_inheritance` の stack push + `out[idx] = computed.clone()`) が
    /// **shallow (Arc reference-count increment)** になる。
    /// `* { counter-reset: c0 c1 ... cN }` × M element の O(N × M) memory
    /// blow-up を単一 heap slot 共有で塞ぐ (security-relevant な DoS 対策、
    /// Content/StringSet と同 pattern の踏襲)。`Arc<Vec<T>>: Deref<Target = Vec<T>>`
    /// により downstream の `.iter()` / `.len()` / `.is_empty()` は既存 pattern
    /// そのままで通る (dom/paint consumer 波及 0)。
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-increment`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#increment-set>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + increment pairs。counter tree の実 resolve に向けた
    /// pre-work であり、resolve 本体は将来別途実装する。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale。
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-set`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#increment-set>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + value pairs。counter tree の実 resolve に向けた
    /// pre-work であり、resolve 本体は将来別途実装する。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale。
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// `content` の resolved 中間表現。**non-inherited**。spec initial は
    /// `normal` (CSS Content 3 §1 propdef-content "Initial: normal") で、本 impl
    /// は `normal` を空 list、明示的な `none` を
    /// [`ContentComponent::None`] sentinel として表現する。pseudo-element
    /// consumers can therefore suppress an explicit `content: none` declaration.
    /// 将来の GCPM directive emit に向けた pre-work。
    /// 下流 (raikiri-dom) が `raikiri_traits::ContentValueItem` に mapping する
    /// (raikiri-style は raikiri-traits に依存しない leaf crate、counter-* の
    /// wire-through pattern を踏襲)。
    /// See <https://www.w3.org/TR/css-content-3/#content-property>.
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`、`apply_value` move) と inheritance walk clone
    /// (`resolve_inheritance` の stack push + `out[idx] = computed.clone()`) が
    /// **shallow (Arc reference-count increment)** になる。
    /// `* { content: "<large>" }` × N element の O(N × M) memory blow-up
    /// を単一 heap slot 共有で塞ぐ (security-relevant な DoS 対策)。
    /// `Arc<Vec<T>>: Deref<Target = Vec<T>>` により downstream の `.iter()` /
    /// `.len()` / `.is_empty()` は既存 pattern そのままで通る (dom/paint
    /// consumer 波及 0)。
    pub content: Arc<Vec<ContentComponent>>,
    /// `string-set` の parse 結果 — `(name, content-list)` entry の列。
    /// **non-inherited**。spec initial は `none` (CSS GCPM 3 §1.1.1
    /// propdef-string-set "Initial: none")、本 impl はそれを空 list で表現する。
    /// 名前解決と runtime `string()` 参照は下流 (raikiri-dom) 責務。
    /// See <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>.
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::content`] と同 rationale
    /// (`* { string-set: name "<large>" }` × N element の同種 DoS 経路を塞ぐ)。
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// `position: running(<custom-ident>)` の seed。**non-inherited**。spec
    /// initial は `position: static` (running() seed 無し、CSS GCPM 3 §1.2.1
    /// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>)、本 impl はそれを
    /// 空 list で表現する。
    ///
    /// 本 field は **per-node で常に 0 または 1 要素** (`position` は spec 上
    /// 単一値の property、element は最大 1 つの `running(name)` しか持たない):
    /// - `position: static` / 他 keyword / rule 無し → empty
    /// - `position: running(name)` → `[RunningTemplate { name }]`
    ///
    /// `Vec` shape を採るのは `content` / `string_set` と同じ
    /// SmolStr wire-through pattern の踏襲 (原則 1 前例主義)。下流 (raikiri-dom)
    /// が per-document `Vec<RunningTemplate>` を組み立てる際に per-node seed を
    /// concatenate する。design doc §7.3 の 2-tier キャッシュ static side に相当。
    pub running_templates: Vec<RunningTemplate>,
    /// `position` — **non-inherited**, initial: `static` (CSS Positioned Layout Module Level 3 §3).
    pub position: PositionValue,
    /// `text-align`。**inherited**、initial: [`TextAlign::Start`]
    /// (CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
    /// <https://www.w3.org/TR/css-text-3/#text-align-property>)。
    ///
    /// spec 上 shorthand (text-align-all + text-align-last の 2 longhand を set)
    /// だが、現状は **shorthand as single field** convention (margin
    /// `Sides<T>` / content-normal-none-as-empty-list precedent) を踏襲して単一
    /// field に保持 (longhand 分離 §6.2 / §6.3 は将来 defer)。詳細は
    /// [`TextAlign`] doc-comment。
    ///
    /// sibling field: [`color`](Self::color) / [`font_family`](Self::font_family) /
    /// [`font_size`](Self::font_size) / [`font_weight`](Self::font_weight) と同じ
    /// **inherited** 系 — `inherit_from` の inherited block に配置し親から by-value
    /// copy (`TextAlign` は `Copy`)。
    ///
    /// **`MatchParent` が本 field の値として観測されることは無い** —
    /// cascade winner が `match-parent` でも、computed 層に届く前に
    /// [`crate::property::resolve_text_align_match_parent`] が親の
    /// [`Self::text_align`] + [`Self::direction`] を使って `Left` / `Right` /
    /// (親値のコピー) に解決する。解決は 2 箇所 — element 経路
    /// ([`crate::specified::SpecifiedValues::finalize`] /
    /// `finalize_as_root`) と page 経路
    /// ([`crate::cascade::resolve_against_inherited`]) — から同じ関数へ
    /// funnel する。
    pub text_align: TextAlign,
    /// `hanging-punctuation`. **inherited**, initial `none` (CSS Text 3
    /// §8.2.1). The consumer currently uses the `first` subset.
    pub hanging_punctuation: HangingPunctuation,
    /// `text-autospace` (CSS Text 4)。**inherited**、initial: `normal`。
    /// The keyword/flag set is preserved for the inline text layout consumer.
    pub text_autospace: TextAutospace,
    /// `word-space-transform` (CSS Text 4). **Inherited**, initial `none`;
    /// the specified keyword combination is preserved without text transformation.
    pub word_space_transform: WordSpaceTransform,
    /// `text-spacing-trim` (CSS Text 4). **Inherited**, initial `normal`;
    /// computed value is the specified keyword. The value is data-only and
    /// does not enable text-spacing layout or rendering behavior.
    pub text_spacing_trim: TextSpacingTrim,
    /// `text-justify` (CSS Text 3 §6.2)。**inherited**、initial: `auto`。
    /// keyword のため computed = specified (by-value copy、`Copy`)。
    /// `distribute` は legacy 値として受理し parley 側では `Justify` と
    /// 同扱い (parley に inter-word/inter-character/distribute の区別無し)。
    pub text_justify: TextJustify,
    /// `text-align-last` (CSS Text 3 §6.1)。**inherited**、initial: `auto`。
    /// keyword のため computed = specified (by-value copy、`Copy`)。
    /// `auto` の解決 (`justify`→`start`、他は `text-align` 値通り) は
    /// consumer (raikiri-dom realign) 側で行う — cascade 層に
    /// `text-align` との結合解決を持ち込まない。
    pub text_align_last: TextAlignLast,
    /// `direction`。**inherited**、initial: [`Direction::Ltr`]
    /// (CSS Writing Modes 4 §2.1 "Specifying Directionality: the direction
    /// property" <https://www.w3.org/TR/css-writing-modes-4/#direction>)。
    /// Computed value = specified value (相対解決なし、[`Direction`] doc 参照)。
    ///
    /// sibling field: [`text_align`](Self::text_align) と同じ **inherited** 系 —
    /// `inherit_from` の inherited block に配置し親から by-value copy
    /// (`Direction` は `Copy`)。
    ///
    /// 追加理由: [`text_align`](Self::text_align) の
    /// `match-parent` 解決 (CSS Text 3 §6.1) が親の computed `direction` を
    /// 要求する。property 自体は CSS Paged Media 3 Appendix A
    /// page-property-list <https://www.w3.org/TR/css-page-3/#page-property-list>
    /// にも独立に載っており、`@page` context でも意味を持つ。
    pub direction: Direction,
    /// `writing-mode`。**inherited**、initial: [`WritingMode::HorizontalTb`]
    /// (CSS Writing Modes 4 §3.2 "Block Flow Direction: the writing-mode
    /// property" <https://www.w3.org/TR/css-writing-modes-4/#propdef-writing-mode>)。
    ///
    /// sibling field: [`direction`](Self::direction) と同じ **inherited** 系 —
    /// `inherit_from` の inherited block に配置し親から by-value copy
    /// (`WritingMode` は `Copy`)。
    ///
    /// [`direction`](Self::direction) の field doc が引用する CSS Paged Media 3
    /// Appendix A page-property-list <https://www.w3.org/TR/css-page-3/#page-property-list>
    /// に、`writing-mode` 自体は **載っていない**。ただし
    /// [`crate::page::PageCascadeResult::declarations`] doc の「Appendix A は
    /// 床であって天井ではない」節が挙げる他の property 群 (`overflow-x`/
    /// `overflow-y` / `DisplayValue` / `PositionValue` / `box-sizing` /
    /// `counter-reset`/`counter-increment` / `content` / `string-set`) と
    /// 同じ扱いで、raikiri は `writing-mode` も意図的に `@page` context へ
    /// 拡張配線している ([`WritingMode`] doc の同節参照)。
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
    /// **inherited**、initial: [`ComputedTextIndent::Px`]`(0.0)` (CSS
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
    /// Source font for an inherited `ch` value.
    pub text_indent_ch_font: Option<ChFontKey>,
    /// Whether the `ch` value was inherited from an ancestor.
    pub text_indent_ch_inherited: bool,
    /// `text-indent`'s `hanging` flag. Inherited, initial `false`.
    pub text_indent_hanging: bool,
    /// `text-indent`'s `each-line` flag. Inherited, initial `false`.
    pub text_indent_each_line: bool,
    /// `padding` — 4-side box-model padding。**non-inherited**、initial:
    /// `Sides::all(ComputedLengthPercentage::Px(0.0))` — CSS Box 3 §4.1
    /// <https://www.w3.org/TR/css-box-3/#padding-physical> initial "0"。
    ///
    /// - Physical longhands: [`padding-top`](https://www.w3.org/TR/css-box-3/#propdef-padding-top) /
    ///   `padding-right` / `padding-bottom` / `padding-left`。
    /// - Shorthand: [`padding` (§4.2)](https://www.w3.org/TR/css-box-3/#padding-shorthand)。
    ///
    /// Value grammar: `<length-percentage [0,∞]>` — non-negative constraint は
    /// parse-time enforce ([`crate::property::PropertyValue::PaddingTop`] doc 参照)。
    /// "Computed value: a computed `<length-percentage>` value" のとおり
    /// `Px` / `Percent` の 2 形態 ([`ComputedLengthPercentage`]) を取る。
    /// **`<percentage>` は computed 層に残る** — 参照値 (containing block width)
    /// が used value 層でしか決まらないため (CSS Cascade 5 §4.5
    /// <https://www.w3.org/TR/css-cascade-5/#used>、raikiri では taffy 委譲)。
    pub padding: Sides<ComputedLengthPercentage>,
    /// Authored `ch` provenance for each padding side.
    pub padding_ch: Sides<Option<ChLengthProvenance>>,
    /// `margin` 4-side quad (top / right / bottom / left)。**non-inherited**、
    /// initial: `0` on each side (`Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))`).
    ///
    /// Author CSS の box model 中核 property。`em` / `rem` / `pt` は cascade の
    /// phase 3 で px に絶対化済み。`<percentage>` と `auto` は computed 層に残り
    /// (spec "Computed value: the keyword `auto` or a computed
    /// `<length-percentage>` value")、containing block に対する解決と余白分配は
    /// used value 層 = 下流 (taffy) の責務。
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
    /// `border` — 4-side box-model border (width / style / color × 4 side)。
    /// **non-inherited**、initial: 各 side が `width` =
    /// [`ComputedLength::ZERO`] / `style` = [`BorderStyle::None`] / `color` =
    /// [`BorderColor::CurrentColor`] (`width` / `style` は crate 内部 field、
    /// consumer からは [`ComputedBorder::width`] / [`ComputedBorder::style`]
    /// accessor 経由で読む)。
    ///
    /// specified の initial は width = `medium` (= 3px) だが、**computed 層では
    /// 0px** になる — CSS Backgrounds 3 §3.3
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-width> の propdef が
    /// "Computed value: absolute length, snapped as a border width; zero if the
    /// border style is `none` or `hidden`" と規定し、initial の border-style が
    /// `none` であるため ([`crate::resolve::resolve_border`] が gate する)。
    ///
    /// - Physical longhands: [`border-top-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   `border-top-style` / `border-top-color` × 4 side。
    /// - Shorthand: [`border` (§3.4)](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands)。
    ///
    /// # Value semantics
    ///
    /// - `Sides<ComputedBorder>: Copy` により per-node write は bit-copy
    ///   (ComputedBorder は f32/enum/BorderColor payload の POD 集合)。
    /// - `width` は [`ComputedLength`] (px)。`<percentage>` は spec grammar に
    ///   含まれないため computed 層でも length のみで足りる
    ///   ([`crate::property::PropertyValue::BorderTopWidth`] doc 参照)。
    /// - `color` は cascade static side で [`BorderColor`] enum として保持し、
    ///   spec `currentcolor` keyword vs. 明示 `<color>` の specified-value
    ///   distinction を preserve する (CSS Backgrounds 3 §3.1
    ///   <https://www.w3.org/TR/css-backgrounds-3/#border-color> initial:
    ///   currentcolor)。used-value resolution (currentcolor → 同 node の
    ///   computed `color` property lookup、CSS Color 3 §4.4) は paint scope
    ///   責務。
    ///
    /// # Non-goals
    ///
    /// - `border-image-*` sub-property (source/slice/width/outset/repeat) は
    ///   未着手、shorthand `border:` も border-image を reset しない
    ///   (spec deviation 明示、future 統合 task で対応)。
    /// - `border-{top,right,bottom,left}` 4-side single-side shorthand (例:
    ///   `border-top: 1px solid red`) は現状未対応、future 追加。
    /// - `currentcolor` の cascade-side enum 保持は解消済み。
    ///   used-value resolution (paint scope) は defer。
    ///
    /// # Primary sources
    ///
    /// - CSS Backgrounds 3 §3 "Borders":
    ///   [`border-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   [`border-style`](https://www.w3.org/TR/css-backgrounds-3/#border-style) /
    ///   [`border-color`](https://www.w3.org/TR/css-backgrounds-3/#border-color) /
    ///   [`border shorthand`](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands)。
    pub border: Sides<ComputedBorder>,
    /// `border-radius` の four-corner computed lengths。**non-inherited**、
    /// initial は全 corner `0px`。percentage/elliptical form は specified
    /// parser の scope 外。
    pub border_radius: ComputedBorderRadius,
    /// `box-shadow` の computed shadow list。**non-inherited**、initial は
    /// empty list (`none`)。描画そのものは paint 層の責務。
    pub box_shadow: Arc<Vec<ComputedBoxShadowItem>>,
    /// `outline` の computed width/style/color。**non-inherited**、initial は
    /// `0px` / `none` / `invert`。outline は box model 寸法へ影響しない。
    /// specified width の `medium` は style が visible の場合だけ computed width に
    /// 反映される。
    pub outline: ComputedOutline,
    /// `outline-offset` の computed length。**non-inherited**、initial は `0px`
    /// (CSS UI 3 §4.5 <https://www.w3.org/TR/css-ui-3/#outline-offset>)。
    /// 負値は border edge より内側への inset を示す。
    pub outline_offset: ComputedLength,
    /// `width` — preferred physical horizontal size (writing-mode neutral な
    /// physical property、vertical writing mode では block axis に対応)。
    /// **non-inherited**、initial: [`ComputedLengthPercentageOrAuto::Auto`] (CSS Sizing 3 §3.1.1
    /// "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
    ///
    /// spec value grammar は `auto | <length-percentage [0,∞]> | min-content |
    /// max-content | fit-content(<length-percentage>)` だが、現状は
    /// `auto` + non-negative `<length-percentage>` のみ受理する (min-content /
    /// max-content / fit-content() は未着手)。
    /// 負値は spec grammar `[0,∞]` violation として parse-time drop
    /// ([`crate::property::PropertyValue::Width`] doc + `parse_width` 参照)。
    ///
    /// spec の "Computed value: as specified, with `<length-percentage>` values
    /// computed" のとおり、length は px へ絶対化され percentage は残る
    /// ([`ComputedLengthPercentageOrAuto`]、margin と同型)。`Auto` は CSS
    /// Sizing 3 の automatic size calculation (containing block width から
    /// margin/border/padding を差し引いた値を used-value に採る) として下流
    /// layout で解決する — margin `auto` の余白分配とは意味が異なる。
    pub width: ComputedLengthPercentageOrAuto,
    /// Authored `ch` provenance for the preferred width.
    pub width_ch: Option<ChLengthProvenance>,
    /// `height` — preferred vertical size。**non-inherited**、initial:
    /// `ComputedLengthPercentageOrAuto::Auto` (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>、
    /// spec 明記 "Initial: auto", "Inherited: no")。
    ///
    /// 現状は `auto` + 非負
    /// `<length-percentage>` の 2 分岐のみ受理 — `min-content` / `max-content`
    /// / `fit-content(<length-percentage>)` は spec-valid だが未サポートとして
    /// parser 段で silent drop
    /// (`parse_height` doc 参照)。
    ///
    /// [`ComputedLengthPercentageOrAuto`] は sibling [`Self::width`] と同 shape を
    /// reuse (sibling field、payload 型は共通)。percentage の containing block 換算と
    /// `Auto` の実 layout 高さ計算は used value 層 = 下流 (taffy) 責務。
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
    /// `max-height` — **non-inherited**, initial `none` (mapped to Auto as placeholder).
    pub max_height: ComputedLengthPercentageOrAuto,
    /// `min-width` — **non-inherited**, initial `auto` (CSS Sizing 3 §4
    /// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
    pub min_width: ComputedLengthPercentageOrAuto,
    /// `min-height` — **non-inherited**, initial `auto` (CSS Sizing 3 §4
    /// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
    pub min_height: ComputedLengthPercentageOrAuto,
    /// Authored logical `min-block-size`, retained so layout can distinguish
    /// its fragmentation behavior from a physical `min-height` declaration.
    /// The used value is already mapped into `min_width`/`min_height`.
    pub min_block_size: Option<ComputedLengthPercentageOrAuto>,
    /// `top`。**non-inherited**、initial: `auto`.
    pub top: ComputedLengthPercentageOrAuto,
    /// `right`。**non-inherited**、initial: `auto`.
    pub right: ComputedLengthPercentageOrAuto,
    /// `bottom`。**non-inherited**、initial: `auto`.
    pub bottom: ComputedLengthPercentageOrAuto,
    /// `left`。**non-inherited**、initial: `auto`.
    pub left: ComputedLengthPercentageOrAuto,
    /// `box-sizing`。**non-inherited**、initial: [`BoxSizing::ContentBox`]
    /// (CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
    /// <https://www.w3.org/TR/css-sizing-3/#box-sizing>、"Initial: `content-box`"
    /// / "Inherited: no")。computed value = specified keyword。
    ///
    /// spec note (§3.3): "The definition of the box-sizing property in this
    /// module supersedes the one in [CSS-UI-3]" — CSS-UI-3 の box-sizing
    /// 定義は本 module により supersede されるため、css-sizing-3 が authoritative
    /// source。
    ///
    /// sibling field: [`display`](Self::display) / [`background_color`](Self::background_color)
    /// と同じ **non-inherited** 系 — `inherit_from` の non-inherited block に
    /// 配置し initial 値を直接指定 (親からコピーしない)。
    ///
    /// # Downstream handoff (future scope、style-scope confined)
    ///
    /// 本 field は cascade static side seed のみ保持し、
    /// `apply_computed_to_style` bridge (dom scope、`taffy::Style::box_sizing`
    /// への翻訳) は future cross-scope task に defer (Non-goals)。
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
    /// `font-variant-caps`. **inherited**, initial:
    /// [`FontVariantCaps::Normal`] (CSS Fonts Module Level 3 §6.6
    /// "Capitalization: the font-variant-caps property"
    /// <https://www.w3.org/TR/css-fonts-3/#font-variant-caps-prop>,
    /// "Initial: normal" / "Inherited: yes"). Computed value = specified
    /// keyword — see [`FontVariantCaps`] doc's "7 keyword の意味" section
    /// (all 7 property keywords implemented) and its "Scope carving"
    /// section (the `font-variant` shorthand is not).
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
    /// "Inherited: no" — [`crate::property::PropertyValue::JustifyItems`]
    /// doc's "legacy は未対応" section explains why this crate's initial
    /// differs from the spec's literal `legacy`). Computed value =
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

    /// CSS spec に沿った initial value。cascade で何も matching しなかった root
    /// node と、inheritance chain の terminate に使う。
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            // CSS Backgrounds 3 §2.2: background-color initial は `transparent`。
            background_color: CssColor::TRANSPARENT,
            // shared Arc slot — per-node allocation 回避 (`initial_font_family`
            // doc 参照)。
            font_family: initial_font_family(),
            font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
            font_weight: 400.0,
            // CSS Inline 3 §5.1: line-height initial は `normal` (font metrics
            // ascent+descent 相当を paint 側で resolve)。
            line_height: ComputedLineHeight::Normal,
            display: DisplayValue::Inline,
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            list_style_image: BackgroundImage::None,
            // CSS Lists 3 §4: counter-* の spec initial は `none`、本 impl は
            // 空 list で表現する (anchor は field doc 参照)。
            // shared empty Arc slot — per-node allocation 回避
            // (property.rs `empty_counter_entries` doc 参照)。
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // CSS Content 3 §1: content の spec initial は `normal`。本 impl は下流に
            // とって「no generated content」= 空 list で表現する。
            // shared empty Arc slot — per-node allocation 回避
            // (property.rs `empty_content_list` doc 参照)。
            content: empty_content_list(),
            // CSS GCPM 3 §1.1.1: string-set の spec initial は `none`。本 impl は
            // それを空 list で表現する。
            // same shared-empty-Arc pattern。
            string_set: empty_string_set_entries(),
            // CSS GCPM 3 §1.2.1: position: running() seed initial は empty
            // (position の initial は `static`、running(name) 無し)。
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
            // CSS Writing Modes 4 §2.1: direction initial is `ltr`。
            direction: Direction::Ltr,
            // CSS Writing Modes 4 §3.2: writing-mode initial is
            // `horizontal-tb` (`WritingMode` doc's Non-goal section — the
            // other 4 keywords never appear in this field regardless).
            writing_mode: WritingMode::HorizontalTb,
            cssom_writing_mode: WritingMode::HorizontalTb,
            ruby_position: RubyPosition::Over,
            // CSS Text 3 §8.1: text-indent initial is `0`。
            text_indent: ComputedTextIndent::Px(0.0),
            text_indent_ch_factor: None,
            text_indent_ch_font: None,
            text_indent_ch_inherited: false,
            text_indent_hanging: false,
            text_indent_each_line: false,
            // CSS Box 3 §4.1: padding initial = 0 (all 4 sides)。
            padding: Sides::all(ComputedLengthPercentage::Px(0.0)),
            padding_ch: Sides::all(None),
            // CSS Box 3 §3.1: margin-* physical の initial は `0` (`Sides::all(0)`
            // で全 4 side に spread)。
            margin: Sides::all(ComputedLengthPercentageOrAuto::Px(0.0)),
            margin_ch: Sides::all(None),
            // CSS Backgrounds 3 §3.3/§3.2/§3.1: border initial は各 side で
            // style=none、color=`currentcolor` keyword
            // (`BorderColor::CurrentColor`、`CssColor::BLACK` placeholder から
            // enum variant へ格上げ、CSS Backgrounds 3 §3.1 initial 契約
            // fidelity — 上の margin 行の CSS Box 3 §3.1 とは別 spec の同番号
            // なので注意)。
            //
            // width は specified では `medium` (3px) だが **computed 層では 0px** —
            // §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width> の
            // "Computed value: … zero if the border style is `none` or `hidden`"
            // による (gate 実装は
            // `crate::resolve::resolve_border`)。specified 側の initial は
            // `crate::specified::SpecifiedValues::initial` が持つ。
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
            // CSS UI 3 §4.5: outline-offset initial は `0`.
            outline_offset: ComputedLength::ZERO,
            // CSS Sizing 3 §3.1.1: width initial は `auto`。
            width: ComputedLengthPercentageOrAuto::Auto,
            width_ch: None,
            // CSS Sizing 3 §3.1.1: height initial は `auto`。
            height: ComputedLengthPercentageOrAuto::Auto,
            height_ch: None,
            max_width: ComputedLengthPercentageOrAuto::Auto,
            max_height: ComputedLengthPercentageOrAuto::Auto,
            min_width: ComputedLengthPercentageOrAuto::Auto,
            min_height: ComputedLengthPercentageOrAuto::Auto,
            min_block_size: None,
            top: ComputedLengthPercentageOrAuto::Auto,
            right: ComputedLengthPercentageOrAuto::Auto,
            bottom: ComputedLengthPercentageOrAuto::Auto,
            left: ComputedLengthPercentageOrAuto::Auto,
            // CSS Sizing 3 §3.3: box-sizing initial は `content-box`.
            box_sizing: BoxSizing::ContentBox,
            // CSS Overflow 3 §3.1: overflow-x/overflow-y initial は `visible`。
            // 両 axis が `visible` なので cross-axis
            // coupling (`resolve_overflow`) は initial state では no-op。
            overflow: OverflowXY::both(OverflowValue::Visible),
            // CSS Text Decoration Module Level 3 §2.1/§2.2/§2.3: initial は
            // それぞれ `none` / `solid` / `currentcolor`。
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
            // CSS 2.1 §10.8.1: vertical-align initial は `baseline`。
            vertical_align: VerticalAlign::Baseline,
            // CSS Fonts 4 §2.4: font-style initial は `normal`。
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
            // CSS Fonts Module Level 3 §6.6: font-variant-caps initial は
            // `normal`。
            font_variant_caps: FontVariantCaps::Normal,
            // CSS Text Module Level 3 §2.1: text-transform initial は `none`。
            text_transform: TextTransform::None,
            // CSS Writing Modes 3 §9.1: text-combine-upright initial is `none`.
            text_combine_upright: TextCombineUpright::None,
            // CSS Writing Modes 3 §5.1: text-orientation initial is `mixed`.
            text_orientation: TextOrientation::Mixed,
            // CSS Writing Modes 3 §2.2: unicode-bidi initial is `normal`.
            unicode_bidi: UnicodeBidi::Normal,
            // CSS Display 3 §4: visibility initial は `visible`。
            visibility: Visibility::Visible,
            // CSS2 §9.9.1: z-index initial は `auto`。
            z_index: ZIndexValue::Auto,
            // CSS Text 3 §5.1: word-break initial は `normal`。
            word_break: WordBreak::Normal,
            // CSS Text 3 §5.3: line-break initial is `auto`.
            line_break: LineBreak::Auto,
            // CSS Text 3 §5.4: overflow-wrap initial は `normal`。
            overflow_wrap: OverflowWrap::Normal,
            // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing の
            // initial `normal` は computed 層で `0` (`ComputedLength::ZERO`)。
            letter_spacing: ComputedLength::ZERO,
            letter_spacing_computed: ComputedLetterSpacing::Px(0.0),
            letter_spacing_ch_factor: None,
            letter_spacing_ch_font: None,
            word_spacing: ComputedLength::ZERO,
            word_spacing_computed: ComputedWordSpacing::Px(0.0),
            word_spacing_ch_factor: None,
            word_spacing_ch_font: None,
            // CSS Text Module Level 3 §4.2: tab-size initial は `8`.
            tab_size: ComputedTabSize::Number(8.0),
            // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
            // break-after / break-inside initial は共に `auto`。
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            // CSS2 §9.5.1 / §9.5.2: float / clear の initial は共に `none`。
            float: FloatValue::None,
            clear: ClearValue::None,
            // CSS Text 3 §3: white-space initial は `normal`。
            white_space: WhiteSpace::Normal,
            // CSS Text 4: white-space-collapse initial is `collapse`.
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            text_wrap: TextWrapMode::Wrap,
            text_wrap_style: TextWrapStyle::Auto,
            // CSS Text 3 §5.3: hyphens initial は `manual`。
            hyphens: Hyphens::Manual,
            // CSS Text 4: hyphenate-character initial is `auto`.
            hyphenate_character: HyphenateCharacter::Auto,
            // CSS Text 4: hyphenate-limit-chars initial is `auto`.
            hyphenate_limit_chars: HyphenateLimitChars::INITIAL,
            // CSS Flexible Box Layout Module Level 1 §5.1: flex-direction
            // initial は `row`。
            flex_direction: FlexDirectionValue::Row,
            // CSS Flexible Box Layout Module Level 1 §5.2: flex-wrap initial
            // は `nowrap`。
            flex_wrap: FlexWrapValue::NoWrap,
            // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2:
            // flex-grow initial は `0`、flex-shrink initial は `1`。
            flex_grow: 0.0,
            flex_shrink: 1.0,
            // CSS Flexible Box Layout Module Level 1 §7.2.3: flex-basis
            // initial は `auto`。
            flex_basis: ComputedFlexBasis::Auto,
            // CSS Flexible Box Layout Module Level 1 §4.2: order initial
            // は `0`。
            order: 0,
            // CSS Box Alignment Module Level 3 §5.1: justify-content /
            // align-content initial は `normal`。
            justify_content: ContentAlignmentValue::Normal,
            align_content: ContentAlignmentValue::Normal,
            // CSS Box Alignment Module Level 3 §7.2: align-items initial は
            // `normal`。
            align_items: SelfAlignmentValue::Normal,
            // CSS Box Alignment Module Level 3 §6.2: align-self initial は
            // `auto`。
            align_self: AlignSelfValue::Auto,
            // CSS Box Alignment Module Level 3 §8.1: row-gap / column-gap
            // initial は `normal`。
            row_gap: ComputedLengthPercentageOrNormal::Normal,
            column_gap: ComputedLengthPercentageOrNormal::Normal,
            // CSS Content 3 §2.4.1: quotes の spec initial は "depends on
            // user agent"、本 impl は 独立実装方針によりそれを空 list で
            // 表現する (`none` と同じ shared empty Arc slot、`Self::quotes`
            // field doc / `empty_quotes_entries` doc 参照)。
            quotes: empty_quotes_entries(),
            quotes_auto: true,
            // CSS Text Decoration Module Level 3 §4: text-shadow initial
            // は `none` — shared empty Arc slot
            // (`empty_computed_text_shadow_list` doc 参照)。
            text_shadow: empty_computed_text_shadow_list(),
            // CSS Grid Layout Module Level 1 §7.2/§7.3: grid-template-*
            // initial は共に `none`。
            grid_template_columns: ComputedGridTemplateTracks::None,
            grid_template_rows: ComputedGridTemplateTracks::None,
            grid_template_areas: GridTemplateAreasValue::None,
            // CSS Grid Layout Module Level 1 §7.6: grid-auto-columns /
            // grid-auto-rows initial は `auto`。
            grid_auto_columns: initial_computed_grid_auto_track_list(),
            grid_auto_rows: initial_computed_grid_auto_track_list(),
            // CSS Grid Layout Module Level 1 §7.7: grid-auto-flow initial は
            // `row`。
            grid_auto_flow: GridAutoFlowValue::Row,
            // CSS Grid Layout Module Level 1 §8.3: grid-row-start/-end /
            // grid-column-start/-end initial は共に `auto`。
            grid_row_start: GridLineValue::Auto,
            grid_row_end: GridLineValue::Auto,
            grid_column_start: GridLineValue::Auto,
            grid_column_end: GridLineValue::Auto,
            // CSS Box Alignment Module Level 3 §7.1/§6.1: justify-items /
            // justify-self initial (`crate::property::PropertyValue::JustifyItems`
            // doc の "legacy は未対応" 節参照、justify-self は `auto`)。
            justify_items: SelfAlignmentValue::Normal,
            justify_self: AlignSelfValue::Auto,
            // CSS Fragmentation Module Level 3 §3.3: orphans / widows
            // initial は共に `2`。
            orphans: 2,
            widows: 2,
            // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.6/§2.7/§2.8/§2.9 —
            // spec initial values (`Self::background_repeat` 等の field doc
            // に spec 根拠あり; `background_clip`/`background_origin` は
            // initial が異なる点に注意)。
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
            // CSS Backgrounds and Borders 3 §2.3: background-image initial
            // は `none`。
            background_image: BackgroundImage::None,
            // CSS Images Module Level 3 §5.1: object-fit initial は `fill`。
            object_fit: ObjectFit::Fill,
            // CSS Images Module Level 3 §5.2: object-position initial は
            // `50% 50%` — `background_position` の `0% 0%` とは異なる点に
            // 注意。
            object_position: ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    50.0,
                )),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(50.0)),
            },
            // CSS Color 4 §3.3: opacity initial は `1`。
            opacity: 1.0,
            // CSS Compositing and Blending Level 1 §3.4.2: isolation
            // initial は `auto`。
            isolation: Isolation::Auto,
            // CSS Compositing and Blending Level 1 §3.4.1: mix-blend-mode
            // initial は `normal`。
            mix_blend_mode: MixBlendMode::Normal,
            // CSS Masking Level 1 §7.1: mask-image initial は `none`。
            mask_image: MaskImage::None,
            // CSS Masking Level 1 §5.1: clip-path initial は `none`。
            clip_path: ClipPath::None,
            // CSS Transforms Level 1 §4: transform initial は `none`
            // (空 list)。
            transform: empty_computed_transform_list(),
            // CSS Filter Effects Level 1 §5: filter initial は `none`
            // (空 list)。
            filter: empty_filter_list(),
            // CSS Tables 3 §4: table-layout initial は `auto`
            // (non-inherited)。
            table_layout: TableLayoutValue::Auto,
            // CSS Tables 3 §6: border-collapse initial は `separate`
            // (inherited — root seed 用)。
            border_collapse: BorderCollapseValue::Separate,
            // CSS Tables 3 §6.1: border-spacing initial は `0`
            // (inherited — root seed 用、両軸 0px)。
            border_spacing: ComputedBorderSpacing {
                horizontal: ComputedLength::ZERO,
                vertical: ComputedLength::ZERO,
            },
            // CSS Tables 3 §7: caption-side initial は `top`
            // (inherited — root seed 用)。
            caption_side: CaptionSideValue::Top,
            // CSS Tables 3 §8: empty-cells initial は `show`
            // (inherited — root seed 用)。
            empty_cells: EmptyCellsValue::Show,
            // CSS Multi-column Layout 1: both longhands initially `auto`.
            column_count: ColumnCountValue::Auto,
            column_width: ComputedColumnWidth::Auto,
            custom_properties: empty_custom_properties(),
            local_custom_properties: empty_custom_properties(),
        }
    }

    /// 親 node の computed values から child node の「declaration が 1 つも無い
    /// 場合の computed values」を生成する。
    ///
    /// - **inherited** property は親からコピー
    /// - **non-inherited** property は `initial()` と同じ値を保持
    ///
    /// 各 property の inherited / non-inherited 分類は [`Self`] 定義の field
    /// doc comment を canonical source として参照する
    /// (現状 inherited: color / font-family / font-size / font-weight / text_align / hanging_punctuation / direction / writing_mode / cssom_writing_mode / line_height / font_style / font_kerning / font_optical_sizing / font_variant_emoji / font_language_override / font_variant_ligatures / font_synthesis / font_variant_position / font_palette / font_variant_numeric / font_variant_east_asian / font_variant_caps / text_transform / text_combine_upright / text_orientation / visibility / text_indent / word_break / overflow_wrap / letter_spacing / word_spacing / white_space / white_space_collapse / text_wrap_style / hyphens / hyphenate_character / hyphenate_limit_chars / tab_size / quotes / text_shadow / text_underline_offset / orphans / widows / border_collapse / border_spacing / caption_side / empty_cells、
    /// non-inherited: background-color / display / counter-* / content /
    /// string-set / running_templates / padding / margin / border / border_radius / box_shadow / outline / width / height / box_sizing / overflow / text_decoration_line / text_decoration_style / text_decoration_color / text_decoration_inset / unicode_bidi / vertical_align / z_index / break_before / break_after / break_inside / background_repeat / background_attachment / background_clip / background_origin / background_size / background_position / background_image / object_fit / object_position / opacity / isolation / mix_blend_mode / mask_image / clip_path / transform / filter / table_layout)。
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
}

mod cssom;
pub use cssom::ComputedProperty;

#[cfg(test)]
mod tests;
