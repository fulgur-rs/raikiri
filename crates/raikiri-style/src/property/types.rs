use std::sync::{Arc, OnceLock};

use cssparser::{
    BasicParseError, BasicParseErrorKind, ParseError, Parser, ParserInput, SourcePosition, Token,
};
use smol_str::SmolStr;

use crate::Atom;

// A handful of items in this file are not truly type definitions — they
// were interspersed with them in the original monolithic property.rs — and
// reference functions/types that (until the parse.rs/serialize.rs
// extraction tasks finish) still live directly in property.rs itself.
// `use super::*` bridges that temporarily; it also naturally resolves once
// those items move to their own modules and get re-exported the same way.
use super::*;

/// Implements keyword serialization, and optionally parsing, for a CSS
/// keyword enum from one variant-to-keyword table, so the two directions
/// cannot drift apart.
///
/// `css_keywords!(Type { Variant => "keyword", ... })` adds `as_css_str`
/// and an ASCII case-insensitive `from_css_ident`. The `@serialize` form adds
/// only `as_css_str`, for enums whose grammar spans several tokens.
macro_rules! css_keywords {
    (@serialize $ty:ident { $($variant:ident => $css:literal),+ $(,)? }) => {
        impl $ty {
            /// Every variant, in table order.
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// This variant's own keyword spelling. Computed-value remaps, such as
            /// a legacy keyword computing to another one, are the caller's job.
            pub const fn as_css_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $css,)+
                }
            }
        }
    };
    ($ty:ident { $($variant:ident => $css:literal),+ $(,)? }) => {
        css_keywords!(@serialize $ty { $($variant => $css),+ });

        impl $ty {
            /// Parses one keyword, ASCII case-insensitively.
            pub(crate) fn from_css_ident(ident: &str) -> Option<Self> {
                cssparser::match_ignore_ascii_case! { ident,
                    $($css => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

/// A shared Arc for an empty `<content-list>`. It lets all nodes share the
/// default value used by initial / inherit_from in the cascade without
/// allocating a new heap slot per node; cloning only bumps the reference count.
///
/// As a defense against cascade memory DoS, `ComputedValues.content` and
/// `.string_set` are wrapped in `Arc<Vec<..>>`. Calling
/// `Arc::new(Vec::new())` for every node would add 2N small heap allocations
/// for an N-node document. `OnceLock` keeps **one empty Arc per process**,
/// and [`empty_content_list`] / [`empty_string_set_entries`] clone it at each
/// initial-value site (incrementing only the Arc reference count).
///
/// An empty `Vec::new()` allocates no buffer, but its 24-byte `Vec` struct
/// still exists per node. The Arc replaces it with an 8-byte pointer to shared data.
pub(crate) fn empty_content_list() -> Arc<Vec<ContentComponent>> {
    static EMPTY: OnceLock<Arc<Vec<ContentComponent>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// One `string-set` entry: a `(<custom-ident> name, content-list)` pair
/// stored in an owned Vec. This alias satisfies clippy::type_complexity and
/// keeps the `Arc<Vec<StringSetEntry>>` shape local to this code.
pub(crate) type StringSetEntry = (SmolStr, Vec<ContentComponent>);

/// Shared Arc for empty `string-set` entries, following the same pattern as
/// [`empty_content_list`] to avoid per-node empty-allocation regressions.
pub(crate) fn empty_string_set_entries() -> Arc<Vec<StringSetEntry>> {
    static EMPTY: OnceLock<Arc<Vec<StringSetEntry>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Shared Arc for empty `counter-*` entries. All three properties
/// (`counter-reset` / `counter-increment` / `counter-set`) share one slot
/// because [`Vec<(SmolStr, i32)>`] has the same type for all three.
///
/// This helper follows from the cascade memory DoS mitigation.
/// All three counter-* properties are non-inherited (CSS Lists 3 §4, including
/// `counter-reset`). `SpecifiedValues::inherit_from` therefore initializes
/// each child stack entry to empty values. Using plain `Vec::new()` would
/// create three 24-byte `Vec` structs per node, adding O(N) overhead to
/// an N-node document. Use a `OnceLock`-held shared Arc, as in
/// [`empty_content_list`] / [`empty_string_set_entries`].
pub(crate) fn empty_counter_entries() -> Arc<Vec<(SmolStr, i32)>> {
    static EMPTY: OnceLock<Arc<Vec<(SmolStr, i32)>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Shared Arc for empty `quotes` entries. Both parsing `none` and the
/// initial value (when there is no declaration) use this one slot,
/// following the `OnceLock` shared-slot pattern of [`empty_counter_entries`].
///
/// The spec says the initial value of `quotes` "depends on user agent"
/// (CSS2 §12.3.1), without prescribing specific quote marks. Under our
/// independent-implementation policy (no imported defaults from another UA),
/// we also use this empty list when no declaration exists (see [`PropertyValue::Quotes`]).
pub(crate) fn empty_quotes_entries() -> Arc<Vec<(SmolStr, SmolStr)>> {
    static EMPTY: OnceLock<Arc<Vec<(SmolStr, SmolStr)>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Whether a [`FontFamilyName`] is a generic CSS family keyword or a named family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFamilyKind {
    /// A named family, including quoted strings and unquoted identifier sequences.
    Named,
    /// An unquoted single-identifier generic family keyword.
    Generic,
}

/// One family in a CSS `font-family` list.
///
/// The `kind` is significant: a quoted family name such as `"serif"` is not
/// the generic `serif` family, even though both have the same [`Atom`] text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontFamilyName(
    /// The family text without CSS quotes.
    pub Atom,
    /// Whether this was parsed as a generic family keyword or a named family.
    pub FontFamilyKind,
);

impl FontFamilyName {
    /// Construct a named family.
    pub fn named(name: impl Into<Atom>) -> Self {
        Self(name.into(), FontFamilyKind::Named)
    }

    /// Construct a generic family keyword.
    pub fn generic(name: impl Into<Atom>) -> Self {
        Self(name.into(), FontFamilyKind::Generic)
    }

    /// The family text without CSS quotes.
    pub fn as_str(&self) -> &str {
        self.0.0.as_str()
    }
}

impl std::fmt::Display for FontFamilyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Shared Arc for the `font-family` initial value, using the same `OnceLock`
/// shared-slot pattern as [`empty_content_list`] (another memory DoS mitigation).
///
/// CSS Fonts 4 §2.1 "Font Family: the font-family property"
/// (<https://www.w3.org/TR/css-fonts-4/#font-family-prop>) specifies an initial
/// value that "depends on user agent"; it does not prescribe a family name.
/// This implementation uses the browser-default generic `serif` family.
///
/// `font-family` is **inherited**. Unlike the non-inherited counter-* / content /
/// string-set properties, its initial value is not an empty list but
/// `[FontFamilyName::generic("serif")]`, chosen by this implementation.
/// Thus this helper does not share an empty `Vec` like [`empty_content_list`];
/// it shares the **initial value itself**. Both `SpecifiedValues::initial()`
/// and `ComputedValues::initial()` on the root node bump-share this single
/// heap slot.
///
/// Its per-node cost differs from the other five fields (counter_reset, etc.):
/// those non-inherited properties reset to their initial values at every node,
/// whereas `font-family` is inherited, so each node primarily pays for
/// carrying the parent value through the inheritance walk
/// ([`crate::specified::SpecifiedValues::inherit_from`] calls
/// `parent.font_family.clone()`). Whether the value is the initial `serif` or
/// any author-specified list, the `Arc` makes `.clone()` bump an existing Arc.
/// This helper shares the one creation site for the initial value; it does not force non-initial inherited values into this slot.
pub(crate) fn initial_font_family() -> Arc<Vec<FontFamilyName>> {
    static INITIAL: OnceLock<Arc<Vec<FontFamilyName>>> = OnceLock::new();
    INITIAL
        .get_or_init(|| Arc::new(vec![FontFamilyName::generic("serif")]))
        .clone()
}

/// Shared Arc for the empty `text-shadow` list (`none`), following the same
/// `OnceLock` shared-slot pattern as [`empty_content_list`] to avoid per-node
/// allocation regressions. Representing `none` as an empty list follows
/// `parse_content` / `parse_counter_property`
/// (see the [`TextShadowItem`] documentation).
pub(crate) fn empty_text_shadow_list() -> Arc<Vec<TextShadowItem>> {
    static EMPTY: OnceLock<Arc<Vec<TextShadowItem>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Shared Arc for the empty `box-shadow` list (`none`).
pub(crate) fn empty_box_shadow_list() -> Arc<Vec<BoxShadowItem>> {
    static EMPTY: OnceLock<Arc<Vec<BoxShadowItem>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Shared Arc for the empty `transform` list (`none`), using the same shared-slot pattern.
pub(crate) fn empty_transform_list() -> Arc<Vec<TransformFunction>> {
    static EMPTY: OnceLock<Arc<Vec<TransformFunction>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Shared Arc for the empty `filter` list (`none`), using the same shared-slot pattern.
pub(crate) fn empty_filter_list() -> Arc<Vec<FilterFunction>> {
    static EMPTY: OnceLock<Arc<Vec<FilterFunction>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// RGBA color (0–255 per channel; `a` = 255 means fully opaque).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CssColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl CssColor {
    /// Opaque black: equivalent to the `<color>` initial value.
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    /// Fully transparent: equivalent to the `background-color` initial value.
    ///
    /// CSS Color 4 §6.3 "The transparent keyword"
    /// <https://www.w3.org/TR/css-color-4/#transparent-color>:
    /// "The keyword `transparent` specifies a transparent black; it is a
    /// shorthand for `rgba(0, 0, 0, 0)`". The initial value of `background-color`
    /// CSS Backgrounds 3 §2.2 <https://www.w3.org/TR/css-backgrounds-3/#background-color>
    /// is defined as `transparent` there.
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// Parse a hex-notation payload (the digit sequence without the leading `#`).
    ///
    /// CSS Color 4 §5.2 "The RGB Hexadecimal Notations: `#RRGGBB`"
    /// <https://www.w3.org/TR/css-color-4/#hex-notation> accepts four forms:
    ///
    /// - **3-digit** `rgb`     → duplicate each nibble → `#RRGGBB`, `a = 255`
    /// - **4-digit** `rgba`    → duplicate as above, including the 4-bit alpha nibble
    /// - **6-digit** `rrggbb`  → `a = 255` (fully opaque)
    /// - **8-digit** `rrggbbaa` → final byte is alpha (0..=255)
    ///
    /// The "digit duplication" for short forms (3/4 digits) is verbatim from §5.2:
    ///
    /// > This syntax is often explained by saying that it’s identical to a
    /// > 6-digit notation obtained by "duplicating" all of the digits. For
    /// > example, the notation #123 specifies the same color as the notation
    /// > #112233.
    ///
    /// The same applies to four digits; §5.2 says:
    ///
    /// > This is a shorter variant of the 8-digit notation, "expanded" in the
    /// > same way as the 3-digit notation is.
    ///
    /// Expand nibble `n` (0..=15) as `(n << 4) | n = n * 17`.
    ///
    /// # Case
    ///
    /// Accept `0-9` / `a-f` / `A-F` (ASCII case-insensitive). §5.2 says:
    ///
    /// > the case of the letters doesn’t matter - #00ff00 is identical to
    /// > #00FF00
    ///
    /// # Invalid input
    ///
    /// Return `None` for other lengths (0/1/2/5/7/9+) or non-hex bytes
    /// (invalid under the spec, so drop the value). The tokenizer
    /// (`Token::Hash`) removes the leading `#` before calling this helper.
    /// Direct callers outside the parser must remove the `#` themselves.
    pub fn from_hex(payload: &str) -> Option<Self> {
        let hex = payload.as_bytes();
        match hex.len() {
            3 => {
                let r = expand_hex_nibble(hex_digit(hex[0])?);
                let g = expand_hex_nibble(hex_digit(hex[1])?);
                let b = expand_hex_nibble(hex_digit(hex[2])?);
                Some(Self { r, g, b, a: 255 })
            }
            4 => {
                let r = expand_hex_nibble(hex_digit(hex[0])?);
                let g = expand_hex_nibble(hex_digit(hex[1])?);
                let b = expand_hex_nibble(hex_digit(hex[2])?);
                let a = expand_hex_nibble(hex_digit(hex[3])?);
                Some(Self { r, g, b, a })
            }
            6 => {
                let r = hex_byte(hex[0], hex[1])?;
                let g = hex_byte(hex[2], hex[3])?;
                let b = hex_byte(hex[4], hex[5])?;
                Some(Self { r, g, b, a: 255 })
            }
            8 => {
                let r = hex_byte(hex[0], hex[1])?;
                let g = hex_byte(hex[2], hex[3])?;
                let b = hex_byte(hex[4], hex[5])?;
                let a = hex_byte(hex[6], hex[7])?;
                Some(Self { r, g, b, a })
            }
            // A 0/1/2/5/7/9+-digit form is invalid under the §5.2 hex-notation grammar.
            _ => None,
        }
    }
}

/// Convert an ASCII hex digit (`0-9` / `a-f` / `A-F`) to a nibble in 0..=15.
/// Case-insensitive per CSS Color 4 §5.2; non-hex returns `None`.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Assemble a two-digit hex byte. Validate the `hi` and `lo` nibbles with [`hex_digit`]
/// and combine them as `(hi << 4) | lo`; return `None` if either is non-hex.
fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_digit(hi)? << 4) | hex_digit(lo)?)
}

/// Expand a 4-bit nibble `n` (`0..=15`) to an 8-bit channel `nn`.
/// `(n << 4) | n = n * 17` implements the short-form expansion by
/// "duplicating" all digits (CSS Color 4 §5.2; `#f` → `0xff`, `#8` → `0x88`).
/// For verbatim quotes, see the two excerpts in [`CssColor::from_hex`].
fn expand_hex_nibble(n: u8) -> u8 {
    (n << 4) | n
}

/// CSS length or length-percentage value (an author-CSS type for implementing the box model).
///
/// Each variant retains the authored value (the raw number as written). The
/// sibling-arm convention starts with [`Length::Px`]: `Px(16.0)` = `16px`.
/// Other variants likewise retain the authored number (`Em(1.2)` = `1.2em`,
/// `Percent(50.0)` = `50%`, storing the literal number).
///
/// # This type does not imply the specified layer; origin determines the layer
///
/// In the element path, absolutization yields the `Computed*` types in
/// [`crate::resolve`], so this type represents specified values there,
/// whereas [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// is a bag of `PropertyValue` entries that can carry computed values
/// represented by this type. Do not assume a `Length` is unresolved.
///
/// The type does not identify the cascade layer. In the page path,
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// can contain computed values represented by these same variants. Consumers
/// must use the cascade contract rather than infer resolution from the variant
/// name.
///
/// Downstream matches should include a wildcard arm because this type is
/// `#[non_exhaustive]`.
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §6.1.1 "Font-relative Lengths":
///   [`em`](https://www.w3.org/TR/css-values-4/#em) —
///   "Equal to the computed value of the font-size property of the element on
///   which it is used." /
///   [`rem`](https://www.w3.org/TR/css-values-4/#rem) —
///   "Equal to the computed value of the em unit on the root element."
/// - CSS Values 4 §5.5 "Percentages":
///   [`<percentage>`](https://www.w3.org/TR/css-values-4/#percentages) —
///   "Percentage values are denoted by &lt;percentage&gt;, and indicates a value
///   that is some fraction of another reference value."
/// - CSS Values 4 §6.2 "Absolute Lengths":
///   [`pt`](https://www.w3.org/TR/css-values-4/#absolute-lengths) —
///   `1pt = 1/72 in`, with `1in = 96px` in CSS: a pixel-relative absolute unit.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Absolute pixel length. `10px` → `Px(10.0)`.
    Px(f32),
    /// Font-relative length: `em`, relative to the element’s computed `font-size`.
    /// `1.2em` → `Em(1.2)`. Downstream resolves it using the font-size stack.
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#em>).
    Em(f32),
    /// Font-relative length: `rem`, relative to the root element’s computed `font-size`.
    /// `1rem` → `Rem(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rem>).
    Rem(f32),
    /// Percentage: a fraction of a reference value in a `<length-percentage>` context.
    /// `50%` → `Percent(50.0)` (stores the authored number without dividing by 100).
    ///
    /// Spec: CSS Values 4 §5.5 Percentages
    /// (<https://www.w3.org/TR/css-values-4/#percentages>).
    Percent(f32),
    /// Absolute length: `pt` (1pt = 1/72 in; in CSS, 1in = 96px).
    /// `12pt` → `Pt(12.0)`, equivalent to `12 * 96 / 72 = 16px` when resolved.
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>).
    Pt(f32),
    /// Font-relative length: `ex`, relative to the used font’s x-height.
    /// `1ex` → `Ex(1.0)`.
    ///
    /// raikiri-style has no real font metrics in the style layer (shaping occurs
    /// downstream), so the spec’s unknown-metric fallback always applies:
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ex>) verbatim: "In the cases
    /// where it is impossible or impractical to determine the x-height, a
    /// value of 0.5em must be assumed." Resolve as `0.5 * font-size`.
    Ex(f32),
    /// Font-relative length: `rex`, relative to the root element’s `ex`
    /// (root-font x-height fallback). `1rex` → `Rex(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rex>) — "Equal to the value of
    /// the ex unit on the root element." Apply the same fallback as [`Length::Ex`]
    /// (`0.5em`), using the root font size.
    Rex(f32),
    /// Font-relative length: `ch`, relative to the advance measure of the
    /// used font’s "0" (U+0030) glyph. `1ch` → `Ch(1.0)`.
    ///
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ch>) verbatim: "In the cases
    /// where it is impossible or impractical to determine the measure of the
    /// '0' glyph, it must be assumed to be 0.5em wide by 1em tall. Thus, the ch
    /// unit falls back to 0.5em in the general case, and to 1em when it
    /// would be typeset upright (i.e. writing-mode is vertical-rl or
    /// vertical-lr and text-orientation is upright)." raikiri-style has not
    /// implemented vertical rendering for `writing-mode` (or the
    /// `text-orientation` property), so computed values always normalize to
    /// `HorizontalTb` and the upright branch is unreachable. Resolve as
    /// `0.5 * font-size`; revisit this when vertical rendering is implemented.
    Ch(f32),
    /// Font-relative length: `rch`, relative to the root element’s `ch`.
    /// `1rch` → `Rch(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rch>) — "Equal to the value of
    /// the ch unit on the root element." Apply the same fallback as [`Length::Ch`]
    /// (`0.5em`; the upright branch is likewise unreachable), using the root font size.
    Rch(f32),
    /// Font-relative length: `ic`, relative to the advance measure of the
    /// used font’s CJK water ideograph (U+6C34) glyph. `1ic` → `Ic(1.0)`.
    ///
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ic>) verbatim: "In the cases
    /// where it is impossible or impractical to determine the ideographic
    /// advance measure, it must be assumed to be 1em." Resolve as
    /// `1.0 * font-size` (always use the fallback for the same lack of real
    /// metrics; see the [`Length::Ex`] documentation).
    Ic(f32),
    /// Font-relative length: `ric`, relative to the root element’s `ic`.
    /// `1ric` → `Ric(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ric>) — "Equal to the value of
    /// the ic unit on the root element." Apply the same fallback as [`Length::Ic`]
    /// (`1em`), using the root font size.
    Ric(f32),
    /// Absolute length: `cm` (centimeter). `1cm` → `Cm(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) conversion table
    /// verbatim: "1cm = 96px/2.54".
    Cm(f32),
    /// Absolute length: `mm` (millimeter). `1mm` → `Mm(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) conversion table
    /// verbatim: "1mm = 1/10th of 1cm".
    Mm(f32),
    /// Absolute length: `Q` (quarter-millimeter). `1Q` → `Q(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) conversion table
    /// verbatim: "1Q = 1/40th of 1cm".
    Q(f32),
    /// Absolute length: `in` (inch). `1in` → `In(1.0)`, equivalent to `96px`.
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) conversion table
    /// verbatim: "1in = 2.54cm = 96px".
    In(f32),
    /// Absolute length: `pc` (pica). `1pc` → `Pc(1.0)`, equivalent to `16px`.
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) conversion table
    /// verbatim: "1pc = 1/6th of 1in".
    Pc(f32),
    /// Font-relative length: `lh`, relative to the used element’s computed
    /// `line-height`. `1lh` → `Lh(1.0)`.
    ///
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#lh>) verbatim: "Equal to the
    /// computed value of the line-height property of the element on which it
    /// is used, converting normal to an absolute length by using only the
    /// metrics of the first available font."
    ///
    /// # Resolving `normal`: the same obstacle as `cap`/`rcap`
    ///
    /// `normal` is the **initial value** of `line-height`, so the
    /// unknown-metric branch is common, not an edge case. For the need for
    /// a real font instance, see the [`crate::resolve::used_line_height_length`]
    /// documentation (the same obstacle as `cap`/`rcap`). Unlike `ex`/`ch`/`ic`,
    /// the spec gives no font-size-ratio fallback. Under our independent-
    /// implementation policy, we do not invent one: resolution instead falls
    /// back per property to a value equivalent to that property’s initial value
    /// (see the `resolve_*` function docs in [`crate::resolve`]).
    ///
    /// # Self-reference (when used as the value of `line-height` itself)
    ///
    /// `line-height: 1lh` is self-referential: the definition of `lh` says
    /// "the element on which it is used", which means the used element itself
    /// **on every element**. For the spec text and the reason `rlh` differs,
    /// see the canonical [`crate::resolve::resolve_line_height`] documentation.
    /// This section summarizes rather than repeats that reasoning to avoid
    /// drift between duplicate explanations.
    /// `font-size: 1lh` is also self-referential under that provision (`font-size`
    /// is a font-\* property). It resolves against the parent’s used line-height
    /// (the "`lh` / `rlh` self-reference" section of
    /// [`crate::resolve::resolve_font_size`] is canonical).
    Lh(f32),
    /// Font-relative length: `rlh`, relative to the root element’s `lh`.
    /// `1rlh` → `Rlh(1.0)`.
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rlh>) — "Equal to the value of
    /// the lh unit on the root element."
    ///
    /// # The `normal` obstacle, shared with [`Length::Lh`]
    ///
    /// If the root element’s computed line-height is unresolved `normal`,
    /// `rlh` is unresolved too. As with the `normal` case described in the
    /// [`Length::Lh`] docs (and `cap`/`rcap`), a real font instance is needed.
    ///
    /// # Unlike `Length::Lh`, do not treat this as self-referential
    ///
    /// The definition of `rlh` is a tree-global constant independent of the
    /// declaration element’s position: it always refers to the root value.
    /// **Unlike `lh`, it is not self-referential** except when the declaring
    /// element is itself the root. In that case,
    /// [`crate::specified::SpecifiedValues::finalize_as_root`] handles the
    /// parentless case according to the spec’s "if the element has no parent"
    /// clause: use initial values (`line-height: normal`), always unresolved.
    /// On a non-root element, `line-height: 1rlh` only refers to another,
    /// already-resolved node (the root). It uses the same tree-global basis
    /// (`ResolveContext::root_line_height`) as `rlh` on other box properties.
    /// See [`crate::resolve::resolve_line_height`] for the full reasoning.
    ///
    /// `font-size: 1rlh` is also covered by the same section as [`Length::Lh`],
    /// but the asymmetry above means `rlh` is not self-referential on `font-size`
    /// either. Unless the declaring element is the root, use the tree-global
    /// `ResolveContext::root_line_height` directly
    /// (see [`crate::resolve::resolve_font_size`]).
    Rlh(f32),
    /// Viewport-percentage length: `vw`, 1% of the viewport's width.
    /// `50vw` → `Vw(50.0)`.
    ///
    /// Spec: CSS Values 4 §6.1.2 Viewport-percentage Lengths
    /// (<https://www.w3.org/TR/css-values-4/#viewport-relative-lengths>).
    /// The small (`svw`), large (`lvw`) and dynamic (`dvw`) variants parse to
    /// [`Length::SizedViewport`]. The viewport itself is supplied
    /// by [`crate::resolve::ResolveContext::viewport_width`] and
    /// [`crate::resolve::ResolveContext::viewport_height`].
    Vw(f32),
    /// Viewport-percentage length: `vh`, 1% of the
    /// viewport's height. See [`Length::Vw`].
    Vh(f32),
    /// Viewport-percentage length: `vi`, 1% of the
    /// viewport's size in the root element's inline axis. Only horizontal
    /// writing modes are implemented, so it resolves as [`Length::Vw`].
    Vi(f32),
    /// Viewport-percentage length: `vb`, 1% of the
    /// viewport's size in the root element's block axis. Only horizontal
    /// writing modes are implemented, so it resolves as [`Length::Vh`].
    Vb(f32),
    /// Viewport-percentage length: `vmin`,
    /// the smaller of [`Length::Vw`] and [`Length::Vh`].
    Vmin(f32),
    /// Viewport-percentage length: `vmax`,
    /// the larger of [`Length::Vw`] and [`Length::Vh`].
    Vmax(f32),
    /// A small (`s`), large (`l`) or dynamic (`d`) viewport-percentage
    /// length, such as `10svh` → `SizedViewport(Small, Vh, 10.0)`.
    ///
    /// Spec: CSS Values 4 §6.1.2.1
    /// (<https://www.w3.org/TR/css-values-4/#viewport-variants>). Paged media
    /// has one fixed viewport, so each resolves as its plain unit (see
    /// [`Length::default_viewport`]); the variant is kept so the specified
    /// value serializes as written.
    SizedViewport(ViewportSize, ViewportUnit, f32),
}

/// The viewport size a sized viewport-percentage unit refers to.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportSize {
    /// `s`: the small viewport.
    Small,
    /// `l`: the large viewport.
    Large,
    /// `d`: the dynamic viewport.
    Dynamic,
}

impl ViewportSize {
    /// The unit prefix: `s`, `l` or `d`.
    pub fn prefix(self) -> &'static str {
        match self {
            ViewportSize::Small => "s",
            ViewportSize::Large => "l",
            ViewportSize::Dynamic => "d",
        }
    }
}

/// The plain viewport-percentage unit of a [`Length::SizedViewport`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportUnit {
    /// `vw`.
    Vw,
    /// `vh`.
    Vh,
    /// `vi`.
    Vi,
    /// `vb`.
    Vb,
    /// `vmin`.
    Vmin,
    /// `vmax`.
    Vmax,
}

impl ViewportUnit {
    /// The unit name without a size prefix, such as `vh`.
    pub fn name(self) -> &'static str {
        match self {
            ViewportUnit::Vw => "vw",
            ViewportUnit::Vh => "vh",
            ViewportUnit::Vi => "vi",
            ViewportUnit::Vb => "vb",
            ViewportUnit::Vmin => "vmin",
            ViewportUnit::Vmax => "vmax",
        }
    }

    /// `value` in this unit as a plain viewport-percentage [`Length`].
    pub fn length(self, value: f32) -> Length {
        match self {
            ViewportUnit::Vw => Length::Vw(value),
            ViewportUnit::Vh => Length::Vh(value),
            ViewportUnit::Vi => Length::Vi(value),
            ViewportUnit::Vb => Length::Vb(value),
            ViewportUnit::Vmin => Length::Vmin(value),
            ViewportUnit::Vmax => Length::Vmax(value),
        }
    }
}

impl Length {
    /// A [`Length::SizedViewport`] as its plain unit, which is the same size
    /// in paged media; any other length unchanged.
    pub fn default_viewport(self) -> Length {
        match self {
            Length::SizedViewport(_, unit, value) => unit.length(value),
            other => other,
        }
    }

    /// Whether this is a viewport-percentage length (`vw`, `vh`, `vi`, `vb`,
    /// `vmin`, `vmax`).
    pub(crate) fn is_viewport_relative(self) -> bool {
        matches!(
            self,
            Length::Vw(_)
                | Length::Vh(_)
                | Length::Vi(_)
                | Length::Vb(_)
                | Length::Vmin(_)
                | Length::Vmax(_)
                | Length::SizedViewport(..)
        )
    }

    /// The numeric value regardless of unit.
    pub(crate) fn payload(self) -> f32 {
        match self {
            Length::Px(v)
            | Length::Em(v)
            | Length::Rem(v)
            | Length::Percent(v)
            | Length::Pt(v)
            | Length::Ex(v)
            | Length::Rex(v)
            | Length::Ch(v)
            | Length::Rch(v)
            | Length::Ic(v)
            | Length::Ric(v)
            | Length::Cm(v)
            | Length::Mm(v)
            | Length::Q(v)
            | Length::In(v)
            | Length::Pc(v)
            | Length::Lh(v)
            | Length::Rlh(v)
            | Length::Vw(v)
            | Length::Vh(v)
            | Length::Vi(v)
            | Length::Vb(v)
            | Length::Vmin(v)
            | Length::Vmax(v)
            | Length::SizedViewport(_, _, v) => v,
        }
    }
}

/// `<length-percentage> | auto`: an author-CSS seed shared by margin and width
/// (introduced for margin longhands, later reused by `width`).
///
/// Per the spec, margin properties take `<length-percentage> | auto` (CSS Box 3
/// §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>). Since `auto` is
/// a top-level grammar alternative distinct from `<length-percentage>`, this
/// is a sum type wrapping [`Length`]. CSS Sizing 3 §3.1.1 also uses this
/// grammar shape for `width` / `height` preferred sizes (`auto |
/// <length-percentage [0,∞]> | …`), so they share this type.
/// **The `Auto` variant has different meanings by property**: margins
/// "distribute available space", while width / height use "automatic size
/// calculation". Keep the variant property-agnostic and leave the
/// property-specific interpretation to downstream layout consumers.
///
/// Padding (CSS Box 3 §4) accepts only `<length-percentage>`, not `auto`,
/// so it **does not use** this type; it uses [`Sides<Length>`] directly.
/// Only `Sides<T>` is shared (see the reuse section of the [`Sides`] docs).
///
/// `#[non_exhaustive]` allows future variants (for example, CSS Box 4
/// extensions such as `<flex>` or `auto-vs-fill-available`, and sizing
/// keywords such as `min-content` / `max-content`) without breaking users.
/// This follows the pattern of sibling [`Length`] / [`CounterStyle`].
///
/// Implement [`Copy`] because the underlying [`Length`] is `Copy` (each
/// Px/Em/Rem/Percent/Pt variant holds a single f32). A `Sides<LengthOrAuto>`
/// is four values of about 8 bytes each, so per-node copies are cheap.
///
/// # Primary sources
///
/// - CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties":
///   [`margin-*`](https://www.w3.org/TR/css-box-3/#margin-physical) —
///   "Value: `<length-percentage> | auto`" (same for top/right/bottom/left).
///   Downstream layout resolves `auto` for margins by distributing
///   available space.
/// - CSS Sizing 3 §3.1.1 "Preferred Size Properties":
///   [`width`](https://www.w3.org/TR/css-sizing-3/#preferred-size-properties)
///   — "Value: `auto | <length-percentage [0,∞]> | …`". `auto` means
///   automatic size calculation, handled downstream (unlike margin space distribution).
///
/// A simple `calc()` expression containing a percentage term and an absolute
/// length term. The percentage is kept in authored percent units; Taffy
/// resolves it against the used containing-block basis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalcLengthPercentage {
    /// Percentage coefficient (`100%` is `100.0`).
    pub percent: f32,
    /// Absolute-length offset in CSS px.
    pub px: f32,
}

/// `<length-percentage>` used by `text-indent`, including deferred mixed-unit math.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextIndentLength {
    /// A plain authored length-percentage.
    Length(Length),
    /// A mixed `calc()` whose `em` component waits for the element's computed font size.
    Calc(LengthPercentageCalc),
}

/// Linear `<length-percentage>` terms retained until the computed font size is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthPercentageCalc {
    /// Percentage coefficient (`100%` is `100.0`).
    pub percent: f32,
    /// Absolute-length offset in CSS px.
    pub px: f32,
    /// `em` coefficient resolved against the element's computed font size.
    pub em: f32,
    /// `ch` coefficient, measured against the declaring font's `0` advance by
    /// the layout consumer; style computes only a `0.5em` fallback for it.
    pub ch: f32,
}
/// Specified `text-underline-offset` value.
///
/// A mixed calculation keeps percentage, absolute-pixel, and `em` terms until
/// the declaring element's computed font size is available. Computed values
/// resolve `em` but keep percentage terms relative for inheritance.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextUnderlineOffset {
    /// `auto` lets the user agent choose the underline offset.
    Auto,
    /// A plain authored length or percentage.
    Length(Length),
    /// An additive `calc()` retained until computed font-size resolution.
    Calc(LengthPercentageCalc),
}

/// Specified `letter-spacing` value.
///
/// Percentages stay deferred until line layout, and a mixed calc keeps its
/// percentage and length terms for computed-style serialization.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LetterSpacingValue {
    /// `normal` computes to zero.
    Normal,
    /// A simple length or percentage.
    Length(Length),
    /// A mixed length-percentage calc with any `em` term still deferred.
    Calc(LengthPercentageCalc),
}

/// Specified `word-spacing` value. CSS Text 4 uses the same
/// `normal | <length-percentage>` value shape as [`LetterSpacingValue`].
/// Sharing the representation preserves the same mixed calc terms until the
/// element's computed font size is known.
pub type WordSpacingValue = LetterSpacingValue;

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthOrAuto {
    /// Authored length-percentage (`10px` / `1em` / `50%` / etc.).
    Length(Length),
    /// The meaning of `auto` depends on the consuming property: margins
    /// "distribute available space" (CSS Box 3 §3.1), while width / height use
    /// "automatic size calculation" (CSS Sizing 3 §3.1.1). Keep this variant
    /// property-agnostic; downstream layout resolves it for each property.
    Auto,
    /// A deferred mixed-unit `calc()` expression.
    Calc(CalcLengthPercentage),
    /// The `min-content` sizing keyword (CSS Sizing 3 §3.1.1). Only the
    /// `width` / `inline-size` parser produces it. Block-level boxes sized by
    /// Taffy honor it; inline-block shrink-to-fit, block children laid out
    /// inside an inline formatting context, and a vertical box's block axis
    /// still size it like `auto`.
    MinContent,
}

/// `column-count` value from CSS Multi-column Layout.
///
/// The `auto` keyword leaves the used count to the paired `column-width` and
/// available inline size. Positive integer counts are preserved as authored.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnCountValue {
    /// Automatic column count.
    Auto,
    /// A positive integer column count.
    Count(u32),
}

/// `column-fill` value from CSS Multi-column Layout Module Level 1 §7.1
/// (<https://www.w3.org/TR/css-multicol-1/#propdef-column-fill>).
///
/// The value is non-inherited and initially `balance`. Layout currently uses
/// `auto` to suppress balancing when the multicol block-size is indefinite;
/// the two balance values retain the existing balancing behavior.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnFillValue {
    /// Fill columns sequentially.
    Auto,
    /// Balance content across columns.
    Balance,
    /// Balance all columns.
    BalanceAll,
}

css_keywords!(ColumnFillValue {
    Auto => "auto",
    Balance => "balance",
    BalanceAll => "balance-all",
});

/// `column-span` from CSS Multi-column Layout 1 section 6.1.
/// The non-inherited computed value is the specified keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnSpanValue {
    /// Stay in the ordinary column flow.
    None,
    /// Span all columns of the applicable multicolumn container.
    All,
}

css_keywords!(ColumnSpanValue {
    None => "none",
    All => "all",
});

/// `column-width` value from CSS Multi-column Layout.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidthValue {
    /// Automatic column width.
    Auto,
    /// A non-negative authored length.
    Length(Length),
}

/// The `columns` shorthand, before expansion into its two longhands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnsShorthand {
    /// The `column-width` component.
    pub width: ColumnWidthValue,
    /// The `column-count` component.
    pub count: ColumnCountValue,
}

/// Generic `normal` or length value used by gap properties.
/// Each property's parser controls which `Length` units its grammar accepts.
/// Text spacing properties use [`LetterSpacingValue`] so mixed calc terms can
/// remain available to computed-style serialization.
///
/// `#[non_exhaustive]` allows future variants without breaking downstream
/// exhaustive matches.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthOrNormal {
    /// Authored length or percentage. The property's parser enforces its
    /// grammar and value range.
    Length(Length),
    /// `normal` keyword. Its semantics are property-specific.
    Normal,
}

/// Four-sided box-model value. Field order matches CSS Box 3 §4.2’s
/// four-value shorthand `top right bottom left` (clockwise from top).
///
/// The padding shorthand was the first consumer; its sibling margin reuses
/// this as `Sides<LengthOrAuto>`. The type parameter accommodates the
/// different value types needed by each property.
///
/// The `Copy` derive propagates the conditional `where T: Copy` bound, making a
/// per-node write of `Sides<Length>` a bitwise copy.
/// The `Eq` derive likewise propagates conditionally with `T: Eq`.
/// Both `Sides<Length>` and `Sides<LengthOrAuto>` contain f32 values and
/// therefore are not actually `Eq`; their callers use `PartialEq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sides<T> {
    /// Top side (corresponds to `padding-top` / `margin-top`).
    pub top: T,
    /// Right side.
    pub right: T,
    /// Bottom side.
    pub bottom: T,
    /// Left side.
    pub left: T,
}

impl<T: Clone> Sides<T> {
    /// Construct four identical sides, for the one-value shorthand expansion
    /// of `padding: 10px` / `margin: 10px` (CSS Box 3 §3.2 / §4.2:
    /// "If there is only one component value, it applies to all sides.")
    /// and for the initial value of `Sides` fields (`0` on every side).
    pub fn all(v: T) -> Self {
        Self {
            top: v.clone(),
            right: v.clone(),
            bottom: v.clone(),
            left: v,
        }
    }
}

impl<T> Sides<T> {
    /// Map all four sides independently.
    ///
    /// Used by phase 3 ([`crate::specified::SpecifiedValues::finalize`]) to
    /// absolutize specified `Sides<Length>` / `Sides<LengthOrAuto>` /
    /// `Sides<Border>` into their corresponding computed types. This is
    /// equivalent to mapping four sides manually but structurally prevents
    /// mistakes such as writing `bottom` to `right`.
    ///
    /// `pub(crate)` because only in-crate absolutization currently uses it.
    pub(crate) fn map<U>(self, mut f: impl FnMut(T) -> U) -> Sides<U> {
        Sides {
            top: f(self.top),
            right: f(self.right),
            bottom: f(self.bottom),
            left: f(self.left),
        }
    }
}

/// Pair of start/end values shared by the flow-relative two-value
/// shorthands (`margin-inline` / `margin-block` / `padding-inline` /
/// `padding-block`, each with grammar `<'*-top'>{1,2}`). This is the
/// two-value counterpart of [`Sides<T>`] (the four-value box-model
/// shorthand). It is generic for the same reason: margin uses
/// `LengthOrAuto`, while padding uses `Length`.
///
/// Field names match the spec’s flow-relative `-start` / `-end` suffixes,
/// rather than physical names such as `top`/`right`/`bottom`/`left`.
/// The mapping to physical sides is fixed here: Raikiri assumes
/// writing-mode: horizontal-tb and direction: ltr. For details, see the
/// Non-goal section in the [`PropertyValue::MarginInline`] documentation.
///
/// The `Eq` derive conditionally propagates `T: Eq`, as in [`Sides<T>`].
/// Both `StartEnd<Length>` and `StartEnd<LengthOrAuto>` contain f32 values,
/// so neither is actually `Eq`; callers use `PartialEq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartEnd<T> {
    /// `*-start` component (`margin-inline-start` / `margin-block-start` /
    /// corresponding to `padding-inline-start` / `padding-block-start`.
    pub start: T,
    /// `*-end` component (`margin-inline-end` / `margin-block-end` /
    /// corresponding to `padding-inline-end` / `padding-block-end`.
    pub end: T,
}

impl<T: Clone> StartEnd<T> {
    /// Construct equal start/end values: the two-value counterpart of
    /// [`Sides::all`], used for one-value shorthand expansions such as
    /// `margin-inline: <value>` / `margin-block: <value>` (CSS Logical
    /// Properties and Values 1 §4.2/§4.4, grammar
    /// `<'margin-top'>{1,2}` / `<'padding-top'>{1,2}`:
    /// "If only one value is given, it applies to both the start and end edges").
    pub fn both(v: T) -> Self {
        Self {
            start: v.clone(),
            end: v,
        }
    }
}

impl<T> StartEnd<T> {
    /// Map start and end independently: the two-value version of [`Sides::map`].
    /// Used by phase 3 absolutization in the `@page` cascade
    /// (`absolutize_in_page_context` in `page.rs` is module-private and
    /// cannot be an intra-doc link). In the element cascade, the shorthand
    /// `PropertyValue` has no corresponding field in
    /// [`crate::specified::SpecifiedValues`],
    /// so this shorthand type is not used there. See the
    /// "this variant is not observed during element cascade" section of the
    /// [`PropertyValue::MarginInline`] documentation.
    ///
    /// `pub(crate)` because only in-crate absolutization currently uses it,
    /// as with [`Sides::map`].
    pub(crate) fn map<U>(self, mut f: impl FnMut(T) -> U) -> StartEnd<U> {
        StartEnd {
            start: f(self.start),
            end: f(self.end),
        }
    }
}

/// The `border-style` value: ten keywords in the spec’s `<line-style>` production.
///
/// CSS Backgrounds 3 §3.2 "Line Patterns: the border-style properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>:
/// Represents `<line-style> = none | hidden | dotted | dashed | solid | double |
/// groove | ridge | inset | outset` for borders. Initially `none` and not
/// inherited (§3.2).
///
/// UA stylesheets vary, but this crate is the implementation boundary and
/// accepts only the ten spec-defined alternatives. `parse_border_style_side`
/// compares identifiers ASCII case-insensitively (CSS Values 3 §3.1,
/// "Pre-defined Keywords", <https://www.w3.org/TR/css-values-3/#keywords>).
///
/// # Non-goals
///
/// - **(b) Unsupported**: Visual differences in painting (double strokes
///   and 3D shading for groove/ridge/inset/outset) are paint-scope work.
///   The static cascade only preserves the spec value.
/// - **(a) Invalid under the spec**: Unknown keywords (`wavy` / `wave`,
///   for example, belong to [`TextDecorationStyle`], not this property) cause
///   `parse_border_style_side` to return `None` and drop the declaration.
///
/// Do not derive `Default`: this crate’s convention is "derive `Default` iff
/// `.default()` is called" (as with sibling [`DisplayValue`] / [`TextAlign`]).
/// Initialization instead specifies the spec default [`BorderStyle::None`]
/// directly in [`crate::computed::ComputedValues::initial`].
///
/// `#[non_exhaustive]` allows future variants (for example, draft CSS
/// `border-image` styles) without breaking users, as with sibling
/// [`DisplayValue`] / [`TextAlign`] / [`Length`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    /// `none`: initial value. CSS Backgrounds 3 §3.2 says: "No border.
    /// Color and width are ignored (i.e., the border has width 0)."
    /// (<https://www.w3.org/TR/css-backgrounds-3/#valdef-line-style-none>)
    None,
    /// `hidden` — §3.2 verbatim: "Same as none, but has different behavior in
    /// the border conflict resolution rules for border-collapsed tables
    /// \[CSS2\]."
    Hidden,
    /// `dotted` — §3.2 verbatim: "A series of round dots."
    Dotted,
    /// `dashed` — §3.2 verbatim: "A series of square-ended dashes."
    Dashed,
    /// `solid` — §3.2 verbatim: "A single line segment."
    Solid,
    /// `double` — §3.2 verbatim: "Two parallel solid lines with some space
    /// between them."
    Double,
    /// `groove` — §3.2 verbatim: "Looks as if it were carved in the canvas."
    Groove,
    /// `ridge` — §3.2 verbatim: "Looks as if it were coming out of the
    /// canvas."
    Ridge,
    /// `inset` — §3.2 verbatim: "Looks as if the content on the inside of the
    /// border is sunken into the canvas."
    Inset,
    /// `outset` — §3.2 verbatim: "Looks as if the content on the inside of the
    /// border is raised out of the canvas."
    Outset,
}

css_keywords!(BorderStyle {
    None => "none",
    Hidden => "hidden",
    Dotted => "dotted",
    Dashed => "dashed",
    Solid => "solid",
    Double => "double",
    Groove => "groove",
    Ridge => "ridge",
    Inset => "inset",
    Outset => "outset",
});

/// Keyword payload for `outline-style`.
///
/// CSS Basic User Interface Module Level 3 §4.3
/// The outline style grammar in <https://www.w3.org/TR/css-ui-3/#outline-style>
/// (`auto | <border-style>`) is represented separately from the border-specific [`BorderStyle`].
/// `auto` only makes sense for outlines and is not part of [`BorderStyle`].
///
/// This crate's existing outline scope does not accept `hidden`, so the parser
/// drops the declaration instead of constructing [`Self::Hidden`]. However, the enum
/// can still hold every `<border-style>` keyword so the computed/specified value
/// carrier does not lose the authored value.
///
/// Do not derive `Default`. The spec initial value (`none`) is set explicitly by
/// [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineStyle {
    /// `none` — initial value.
    None,
    /// `hidden` — an outline value rejected by the parser in this scope.
    Hidden,
    /// `dotted`.
    Dotted,
    /// `dashed`.
    Dashed,
    /// `solid`.
    Solid,
    /// `double`.
    Double,
    /// `groove`.
    Groove,
    /// `ridge`.
    Ridge,
    /// `inset`.
    Inset,
    /// `outset`.
    Outset,
    /// `auto` — UA-dependent automatic outline rendering.
    Auto,
}

css_keywords!(OutlineStyle {
    None => "none",
    Hidden => "hidden",
    Dotted => "dotted",
    Dashed => "dashed",
    Solid => "solid",
    Double => "double",
    Groove => "groove",
    Ridge => "ridge",
    Inset => "inset",
    Outset => "outset",
    Auto => "auto",
});

/// Computed value for `border-*-color`: the static cascade preserves the
/// distinction between the specified `currentcolor` keyword and resolved `<color>`.
///
/// CSS Backgrounds 3 §3.1 "Line Colors: the border-color properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-color> — "Initial:
/// currentcolor" (the initial value is the `currentcolor` keyword, not a
/// literal `<color>` such as `black`).
///
/// CSS Color 3 §4.4 "currentColor color keyword"
/// <https://www.w3.org/TR/css-color-3/#currentColor-def> — "The used value of
/// the `currentColor` keyword is the computed value of the `color` property".
/// Resolving the used value (currentcolor → looking up the same node's computed `color` property)
/// is the paint scope's responsibility (with end-to-end integration
/// when border painting is implemented).
///
/// # Why retain an enum on the static cascade side? (Rationale for Option A over B)
///
/// `SpecifiedValues::initial` and `SpecifiedValues::inherit_from`
/// (crate::specified module) build every border side **before** cascade
/// `apply_value` writes the node’s own `color` declaration. With author CSS
/// border-color omitted (going directly to its initial value), the hazard is that border-color never
/// passes through `apply_value`; the cascade therefore cannot capture the node's own color
/// (only the parent's color is input to inherit_from). Option B (storing it directly
/// in advance during cascade) requires a post-cascade resolution pass and sentinel detection;
/// the sentinel is itself equivalent to this enum. It would also violate this crate's invariant that
/// "per-longhand cascade is independent of declaration order" (`apply_value` arm docs and the margin / padding
/// precedent). Option A retains the specified value
/// and delegates resolution to the paint scope, satisfying both constraints
/// (initial-path correctness and per-key determinism) at once.
///
/// # `#[non_exhaustive]`
///
/// The same pattern as sibling [`Length`] / [`LengthOrAuto`] / [`BorderStyle`] / [`Border`]
/// — a forward-compatibility contract for adding future variants (e.g., system-color keywords from CSS Color 4 §6.2 "System Colors"
/// <https://www.w3.org/TR/css-color-4/#css-system-colors>),
/// following the sibling convention.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderColor {
    /// `currentcolor` keyword — the spec-mandated initial value of border-*-color
    /// (CSS Backgrounds 3 §3.1). The paint scope determines the used value by looking up the node's computed
    /// `color` property.
    CurrentColor,
    /// Resolved `<color>` value — payload when the author explicitly specifies a hex / named / `rgb(a)` /
    /// `transparent` color, or when the value is expanded from a `border` / `border-color`
    /// shorthand.
    Resolved(CssColor),
}

/// `border` — an intermediate type grouping three sub-properties for one side.
///
/// Holds the three sub-properties for one side from CSS Backgrounds 3 §3 "Borders":
/// - `width`: [`Length`] — `parse_border_width_side` handles px keyword conversion (thin/
///   medium/thick → 1/3/5 px) and validates that lengths are non-negative.
/// - `style`: [`BorderStyle`] — `parse_border_style_side` accepts all 10
///   alternatives.
/// - `color`: [`BorderColor`] — the enum returned by `parse_border_color` (which parses spec §3.1's `<color>`
///   grammar and also accepts the `currentcolor` keyword). The initial value,
///   [`BorderColor::CurrentColor`], is resolved against the `color` property by the paint scope
///   (replacing the `CssColor::BLACK` placeholder to honor CSS Backgrounds 3
///   §3.1's initial-value contract).
///
/// # `<line-width>` keyword mapping (§3.3)
///
/// Spec §3.3 "Line Thickness: the border-width properties" defines
/// `<line-width> = <length [0,∞]> | thin | medium | thick`: thin=1px,
/// medium=3px and thick=5px as spec-defined values (verbatim: "are equivalent to 1px,
/// 3px, and 5px, respectively"). See the `parse_border_width_side` docs for details.
///
/// # `#[non_exhaustive]`
///
/// Allows future fields (e.g., integration of CSS Backgrounds 4 `border-image-*` into the cascade, or
/// per-side gradient support) without breaking callers — the same pattern as sibling
/// [`Length`] / [`LengthOrAuto`] / [`BorderStyle`].
///
/// # Representing borders as `Sides<Border>`
///
/// Holds all four sides as [`Sides<Border>`] (using the generic [`Sides`] type also used for margin
/// `Sides<LengthOrAuto>` / padding `Sides<Length>`). The cascade
/// writes each side's longhands directly, making application order irrelevant; the shorthand
/// `border: ...` expands into 12 longhands (4 sides × 3 sub-properties) at
/// parse time (as specified by CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand>, following the margin
/// precedent).
///
/// # `Eq` non-derive rationale
///
/// [`Length`] contains an f32 payload (`Px(f32)`, etc.) and thus cannot implement `Eq`;
/// Border also only implements PartialEq (`Sides<Border>` effectively uses PartialEq; `Sides`'s
/// derive transparently propagates the conditional `where T: Eq` bound).
/// The same constraint applies to sibling [`Length`] / [`LengthOrAuto`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// border-width (CSS Backgrounds 3 §3.3); initial `medium` = `Length::Px(3.0)`.
    /// The grammar is `<length [0,∞]>` — its non-negative constraint is
    /// enforced by `parse_border_width_side` at parse time. The spec does not include `<percentage>`
    /// (unlike the padding grammar).
    pub width: Length,
    /// border-style (CSS Backgrounds 3 §3.2); initial `none`.
    pub style: BorderStyle,
    /// border-color (CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>).
    /// The initial value is the `currentcolor` keyword — retained as the [`BorderColor::CurrentColor`]
    /// enum variant. The paint scope resolves its used value (currentcolor →
    /// the same node's computed `color` property).
    /// Promoted from `CssColor` to the [`BorderColor`] enum to faithfully preserve the spec's initial value.
    pub color: BorderColor,
}

/// `Border` is non-exhaustive but provides a default constructor and public
/// fields so downstream callers can construct and then customize a value.
impl Border {
    /// Zero-argument constructor returning the CSS Backgrounds 3 initial values
    /// (`width` = medium = 3px §3.3 / `style` = `none` §3.2 / `color` = `currentcolor` §3.1)
    /// as a `Border`. A thin wrapper around `Self::default()`, with the same shape
    /// as `raikiri_traits::page::PageBox::new`.
    ///
    /// Callers can customize the public fields after construction.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for Border {
    /// CSS Backgrounds 3 initial value: medium width, no border style, and
    /// currentcolor.
    fn default() -> Self {
        Self {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: BorderStyle::None,
            color: BorderColor::CurrentColor,
        }
    }
}

/// A CSS-wide keyword (CSS Cascading 4 §7.3 "Explicit Defaulting"
/// <https://www.w3.org/TR/css-cascade-4/#defaulting-keywords> and CSS Cascading 5
/// §7.3.5 "Rolling Back Cascade Layers: the revert-layer keyword"
/// <https://www.w3.org/TR/css-cascade-5/#revert-layer>).
///
/// Every border longhand and the `border` / `border-<side>` shorthands accept exactly
/// these five keywords as a single-token value. The shorthand forms expand to their
/// longhands with the same keyword preserved (see [`PropertyValue::Border`] and
/// [`PropertyValue::BorderRight`]).
///
/// Resolution follows CSS Cascading 4 §7.3 for the non-inherited border properties
/// (all `border-*` are `Inherited: no` per CSS Backgrounds 3 §3):
/// - `Inherit` takes the parent's computed value for that longhand.
/// - `Initial` takes the property's initial value (see [`crate::specified::INITIAL_BORDER`]).
/// - `Unset` behaves as `Initial` for these non-inherited properties.
/// - `Revert` rolls back to the previous origin's winner (see [`crate::cascade::cascade_rank`]).
/// - `RevertLayer` removes the current layer. For important declarations, it also
///   removes the interval between that layer's normal and important levels.
///   Important element-attached declarations preserve stylesheet important values.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssWideKeyword {
    /// `inherit`.
    Inherit,
    /// `initial`.
    Initial,
    /// `unset`.
    Unset,
    /// `revert`.
    Revert,
    /// `revert-layer`.
    RevertLayer,
}

css_keywords!(CssWideKeyword {
    Inherit => "inherit",
    Initial => "initial",
    Unset => "unset",
    Revert => "revert",
    RevertLayer => "revert-layer",
});

/// The **specified** value of the `font-weight` property.
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// (<https://www.w3.org/TR/css-fonts-4/#font-weight-prop>); value grammar:
/// `<font-weight-absolute> | bolder | lighter`,
/// `<font-weight-absolute> = [ normal | bold | <number [1,1000]> ]`.
///
/// # Why specified and computed values have separate types
///
/// `bolder` / `lighter` are **relative weights**. The table in spec §2.2.1 computes
/// the absolute weight from the **inherited value (the parent's computed font-weight)**. This
/// resolution needs cascade context (the parent node's computed value), so it cannot occur at
/// parse time. The value is therefore split into two types:
///
/// - **specified side** ([`FontWeightValue`], this type) — keeps `bolder` / `lighter` as
///   sentinel variants.
/// - **computed side** ([`crate::computed::ComputedValues::font_weight`], `f32`)
///   — holds only resolved absolute weights. `bolder` / `lighter` are
///   stored only after resolution by [`crate::cascade::apply_value`].
///
/// This distinction also follows the spec: the property table in §2.2 says
/// `Computed value: a number, see below`, while §2.2.1 "Relative Weights"
/// (<https://www.w3.org/TR/css-fonts-4/#relative-weights>) states "Specified values
/// of `bolder` and `lighter` indicate weights relative to the weight of the
/// parent element. The computed weight is calculated based on the inherited
/// `font-weight` value". Relative keywords therefore never
/// remain on the computed side.
///
/// # Primary source
///
/// - CSS Fonts 4 §2.2 (<https://www.w3.org/TR/css-fonts-4/#font-weight-prop>)
/// - CSS Values 3 §3.1 "Pre-defined Keywords"
///   (<https://www.w3.org/TR/css-values-3/#keywords>) — keywords are matched
///   ASCII case-insensitively.
///
/// Downstream matches must include a wildcard arm (`#[non_exhaustive]`;
/// adding variants is a forward-compatibility guarantee that does not break existing pattern matches,
/// as with sibling [`LineHeight`] / [`DisplayValue`]).
///
/// # Why `Eq` is not derived
///
/// The [`Absolute`](Self::Absolute) payload is now `f32` (formerly `u16`), so
/// `Eq` cannot be derived (`f32: !Eq`, because NaN is not reflexive). Comparing with
/// `PartialEq` (`==`) suffices: out-of-range values outside `[1, 1000]` are rejected, and NaN /
/// ±inf cannot reach this variant, making practical comparisons well-defined.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontWeightValue {
    /// `<font-weight-absolute>` — `normal` (400) / `bold` (700) /
    /// `<number [1,1000]>` map to one absolute weight. CSS Fonts 4 §2.2.2
    /// "Missing weights" (<https://www.w3.org/TR/css-fonts-4/#missing-weights>)
    /// Stores fractions as `f32` as required by "Fractional weights are valid" (formerly
    /// `u16`, promoted to retain fractions — see the [`parse_font_weight`] docs).
    Absolute(f32),
    /// `bolder` — a weight one step heavier than the inherited value. During cascade,
    /// [`crate::cascade::resolve_relative_weight`] uses the spec §2.2.1 table to resolve it
    /// to an absolute value.
    Bolder,
    /// `lighter` — a weight one step lighter than the inherited value. Resolved at the same time
    /// as [`Bolder`](Self::Bolder).
    Lighter,
}

/// Keywords for `font-size: larger | smaller` (`<relative-size>`).
/// Payload of [`PropertyValue::FontSizeRelative`].
///
/// CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop>.
/// Resolution happens in [`crate::cascade::resolve_relative_font_size`] — like [`FontWeightValue::Bolder`]
/// / [`FontWeightValue::Lighter`], it performs a parent computed font-size
/// read-modify-write.
///
/// Unlike [`FontWeightValue`], this type is not added to the `raikiri` (umbrella) `pub use` list
/// (see the [`PropertyValue::FontSizeRelative`] docs for why `Self::FontSize` was not reused
/// for this new variant).
///
/// Downstream matches must include a wildcard arm (`#[non_exhaustive]`,
/// following sibling [`FontWeightValue`]).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeFontSize {
    /// `larger` — one step larger than the parent's computed font-size.
    Larger,
    /// `smaller` — one step smaller than the parent's computed font-size.
    Smaller,
}

/// `font-kerning` property values from CSS Fonts Module Level 3.
/// <https://www.w3.org/TR/css-fonts-3/#font-kerning-prop>
///
/// The property is inherited, has initial value `auto`, and its computed value
/// is the specified keyword. This stores CSSOM data only; it does not change
/// glyph shaping or painting.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontKerning {
    /// `auto` — initial value.
    Auto,
    /// `normal`.
    Normal,
    /// `none`.
    None,
}

css_keywords!(FontKerning {
    Auto => "auto",
    Normal => "normal",
    None => "none",
});

/// `font-optical-sizing` property keywords from CSS Fonts Module Level 4.
/// <https://www.w3.org/TR/css-fonts-4/#font-optical-sizing-def>
///
/// The property is inherited, has initial value `auto`, and its computed value
/// is the specified keyword. This stores CSSOM data only; optical-size
/// selection and glyph shaping remain out of scope.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontOpticalSizing {
    /// `auto` — initial value.
    Auto,
    /// `none`.
    None,
}

css_keywords!(FontOpticalSizing {
    Auto => "auto",
    None => "none",
});

/// `font-variant-emoji` keywords from CSS Fonts Module Level 4.
/// <https://www.w3.org/TR/css-fonts-4/#font-variant-emoji-prop>
///
/// The property is inherited, has initial value `normal`, and its computed
/// value is the specified keyword. This stores CSSOM data only; emoji
/// presentation and glyph selection remain out of scope.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariantEmoji {
    /// `normal` — initial value.
    Normal,
    /// `text`.
    Text,
    /// `emoji`.
    Emoji,
    /// `unicode`.
    Unicode,
}

css_keywords!(FontVariantEmoji {
    Normal => "normal",
    Text => "text",
    Emoji => "emoji",
    Unicode => "unicode",
});

/// `font-language-override` keyword or string from CSS Fonts Module Level 4.
/// <https://www.w3.org/TR/css-fonts-4/#font-language-override-prop>
///
/// The property is inherited, has initial value `normal`, and is stored only
/// for computed-value/CSSOM exposure. It does not select a language system or
/// alter glyph shaping.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontLanguageOverride {
    /// `normal` — initial value.
    Normal,
    /// A quoted language-system string. Trailing spaces are removed during parsing.
    String(SmolStr),
}

/// Individual `font-variant-ligatures` keywords from CSS Fonts Module Level 4.
/// <https://www.w3.org/TR/css-fonts-4/#font-variant-ligatures-prop>
///
/// This narrow representation covers the individual values in the pinned
/// computed-value test. The property's combined component grammar can be
/// added when a selected case requires it. Values are stored for CSSOM only;
/// ligature shaping and painting are unchanged.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariantLigatures {
    /// `normal` — initial value.
    Normal,
    /// `none`.
    None,
    /// `common-ligatures`.
    CommonLigatures,
    /// `no-common-ligatures`.
    NoCommonLigatures,
    /// `discretionary-ligatures`.
    DiscretionaryLigatures,
    /// `no-discretionary-ligatures`.
    NoDiscretionaryLigatures,
    /// `historical-ligatures`.
    HistoricalLigatures,
    /// `no-historical-ligatures`.
    NoHistoricalLigatures,
    /// `contextual`.
    Contextual,
    /// `no-contextual`.
    NoContextual,
}

css_keywords!(FontVariantLigatures {
    Normal => "normal",
    None => "none",
    CommonLigatures => "common-ligatures",
    NoCommonLigatures => "no-common-ligatures",
    DiscretionaryLigatures => "discretionary-ligatures",
    NoDiscretionaryLigatures => "no-discretionary-ligatures",
    HistoricalLigatures => "historical-ligatures",
    NoHistoricalLigatures => "no-historical-ligatures",
    Contextual => "contextual",
    NoContextual => "no-contextual",
});

/// Individual `font-variant-position` computed keywords from CSS Fonts Module Level 3 §6.5.
/// <https://www.w3.org/TR/css-fonts-3/#font-variant-position-prop>
///
/// This data-only representation preserves the specified keyword for CSSOM.
/// It does not enable subscript/superscript glyph shaping or synthesis.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariantPosition {
    /// `normal` — initial value.
    Normal,
    /// `sub`.
    Sub,
    /// `super`.
    Super,
}

css_keywords!(FontVariantPosition {
    Normal => "normal",
    Sub => "sub",
    Super => "super",
});

/// The `font-palette` values exercised by the pinned computed-value case.
/// CSS Fonts 4 §9.1 <https://drafts.csswg.org/css-fonts/#font-palette-prop>.
///
/// Palette selection is retained as CSSOM data only; this type does not select
/// font palettes or change glyph rendering. `palette-mix()` is outside this
/// case's supported subset.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontPaletteValue {
    /// `normal` — initial value.
    Normal,
    /// `light`.
    Light,
    /// `dark`.
    Dark,
    /// A named dashed palette identifier, such as `--pitchfork`.
    Palette(SmolStr),
}

/// `font-variant-numeric` keyword set retained for computed-style exposure.
/// CSS Fonts Module Level 3 §6.7
/// <https://www.w3.org/TR/css-fonts-3/#font-variant-numeric-prop>.
///
/// Values are preserved as CSSOM data only; this type does not enable OpenType
/// features or change shaping/rendering.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontVariantNumeric {
    /// `lining-nums`.
    pub lining_nums: bool,
    /// `oldstyle-nums`.
    pub oldstyle_nums: bool,
    /// `proportional-nums`.
    pub proportional_nums: bool,
    /// `tabular-nums`.
    pub tabular_nums: bool,
    /// `diagonal-fractions`.
    pub diagonal_fractions: bool,
    /// `stacked-fractions`.
    pub stacked_fractions: bool,
    /// `ordinal`.
    pub ordinal: bool,
    /// `slashed-zero`.
    pub slashed_zero: bool,
}

impl FontVariantNumeric {
    /// Initial value: no numeric features enabled (`normal`).
    pub const fn initial() -> Self {
        Self {
            lining_nums: false,
            oldstyle_nums: false,
            proportional_nums: false,
            tabular_nums: false,
            diagonal_fractions: false,
            stacked_fractions: false,
            ordinal: false,
            slashed_zero: false,
        }
    }
}

/// East Asian text variant used by `font-variant-east-asian`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariantEastAsianVariant {
    /// `jis78`.
    Jis78,
    /// `jis83`.
    Jis83,
    /// `jis90`.
    Jis90,
    /// `jis04`.
    Jis04,
    /// `simplified`.
    Simplified,
    /// `traditional`.
    Traditional,
}

/// East Asian width value used by `font-variant-east-asian`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariantEastAsianWidth {
    /// `full-width`.
    FullWidth,
    /// `proportional-width`.
    ProportionalWidth,
}

/// `font-variant-east-asian` value retained for computed-style exposure.
/// CSS Fonts Module Level 3 §6.8
/// <https://www.w3.org/TR/css-fonts-3/#font-variant-east-asian-prop>.
///
/// This is CSSOM data only; it does not perform glyph substitution or sizing.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontVariantEastAsian {
    /// One East Asian character-form variant, if specified.
    pub variant: Option<FontVariantEastAsianVariant>,
    /// One East Asian width form, if specified.
    pub width: Option<FontVariantEastAsianWidth>,
    /// Whether the `ruby` value is specified.
    pub ruby: bool,
}

impl FontVariantEastAsian {
    /// Initial value (`normal`).
    pub const fn initial() -> Self {
        Self {
            variant: None,
            width: None,
            ruby: false,
        }
    }
}

/// A list of tagged font settings, shared between the nodes that inherit
/// it, that records whether it is in computed order.
macro_rules! tagged_settings_list {
    ($(#[$doc:meta])* $name:ident, $item:ty) => {
        $(#[$doc])*
        ///
        /// It dereferences to its entries. Whether they are already sorted by
        /// tag with one entry per tag is found once, when the list is made,
        /// so computing the value of a list that is already in that order,
        /// such as one every descendant inherits, costs nothing per node.
        #[derive(Clone)]
        pub struct $name {
            items: Arc<[$item]>,
            canonical: bool,
        }

        impl $name {
            fn new(items: Arc<[$item]>) -> Self {
                let canonical = items.windows(2).all(|pair| pair[0].tag < pair[1].tag);
                Self { items, canonical }
            }

            /// Whether `self` and `other` share their entries.
            pub fn shares(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.items, &other.items)
            }

            /// The list in computed order: the last entry per tag, sorted by
            /// tag. A list already in that order is returned as it is.
            fn canonicalized(self) -> Self {
                if self.canonical {
                    return self;
                }
                let mut items = self.items.to_vec();
                items.sort_by(|left, right| left.tag.cmp(&right.tag));
                let mut canonical: Vec<$item> = Vec::with_capacity(items.len());
                for item in items {
                    if let Some(last) = canonical.last_mut()
                        && last.tag == item.tag
                    {
                        *last = item;
                        continue;
                    }
                    canonical.push(item);
                }
                Self {
                    items: canonical.into(),
                    canonical: true,
                }
            }
        }

        impl From<Vec<$item>> for $name {
            fn from(items: Vec<$item>) -> Self {
                Self::new(items.into())
            }
        }

        impl std::ops::Deref for $name {
            type Target = [$item];

            fn deref(&self) -> &[$item] {
                &self.items
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.items == other.items
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.items.fmt(f)
            }
        }
    };
}

/// `font-feature-settings` values from CSS Fonts 4 §6.12
/// (<https://www.w3.org/TR/2026/WD-css-fonts-4-20260906/#font-feature-settings-prop>).
///
/// Specified values retain authored order and duplicates. Computed values
/// keep the last value for each case-sensitive tag and sort tags by code unit.
/// The resulting settings are passed to Shodo for OpenType shaping.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontFeatureSettings {
    /// `normal` — the initial value, with no author-specified feature changes.
    Normal,
    /// A non-empty list of specified feature tag/value pairs, shared so that
    /// the computed value every descendant inherits is not copied per node.
    Features(FontFeatureList),
}

/// One `<feature-tag-value>` pair in `font-feature-settings`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontFeatureSetting {
    /// A case-sensitive, four-byte printable ASCII OpenType feature tag.
    pub tag: [u8; 4],
    /// A non-negative feature value (`on` maps to 1 and `off` to 0).
    pub value: u32,
}

tagged_settings_list!(
    /// The `<feature-tag-value>` pairs of a `font-feature-settings` list.
    FontFeatureList,
    FontFeatureSetting
);

impl Eq for FontFeatureList {}

impl FontFeatureSettings {
    /// Return the computed representation: last entry per tag, sorted by tag.
    /// A list already in that form, such as an inherited computed value, is
    /// returned as it is, sharing its entries.
    pub(crate) fn canonicalized(self) -> Self {
        match self {
            Self::Normal => Self::Normal,
            Self::Features(settings) => Self::Features(settings.canonicalized()),
        }
    }
}

/// `font-variation-settings` value used for specified and computed style.
/// CSS Fonts 4 §8.2 <https://www.w3.org/TR/css-fonts-4/#font-variation-settings-def>.
///
/// The specified value retains authored order and duplicate tags. The computed
/// value keeps the last value for each case-sensitive tag and sorts by tag. This
/// is CSSOM data only; no variation axis, font selection, shaping, or rendering
/// behavior is applied.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum FontVariationSettings {
    /// `normal` — initial value.
    Normal,
    /// Non-empty setting list. It keeps authored order when specified and
    /// canonical order when computed, and is shared so that the computed
    /// value every descendant inherits is not copied per node.
    Settings(FontVariationList),
}

/// One `<opentype-tag> <number>` pair in `font-variation-settings`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct FontVariationSetting {
    /// Case-sensitive, four-printable-ASCII-character axis tag.
    pub tag: SmolStr,
    /// The numeric axis coordinate.
    pub value: f32,
}

tagged_settings_list!(
    /// The `<opentype-tag> <number>` pairs of a `font-variation-settings` list.
    FontVariationList,
    FontVariationSetting
);

impl FontVariationSettings {
    /// Return the computed representation: last entry per tag, sorted by tag.
    /// A list already in that form, such as an inherited computed value, is
    /// returned as it is, sharing its entries.
    pub(crate) fn canonicalized(self) -> Self {
        match self {
            Self::Normal => Self::Normal,
            Self::Settings(settings) => Self::Settings(settings.canonicalized()),
        }
    }
}

/// `font-synthesis` shorthand value preserved for computed-style exposure.
/// CSS Fonts 4 §2.8.5 <https://drafts.csswg.org/css-fonts/#font-synthesis>.
///
/// The WPT-driven representation stores the tested synthesis keywords only.
/// It does not request font synthesis or alter font selection or shaping.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontSynthesisValue {
    /// Synthesize bold when enabled.
    pub weight: bool,
    /// Synthesize italic/oblique when enabled, or permit oblique-only fallback.
    pub style: FontSynthesisStyle,
    /// Synthesize small caps when enabled.
    pub small_caps: bool,
    /// Synthesize super/subscript position when enabled.
    pub position: bool,
}

/// Style component retained by [`FontSynthesisValue`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontSynthesisStyle {
    /// Synthesis of italic/oblique is disabled.
    None,
    /// The `style` keyword.
    Auto,
    /// The pinned case's `oblique-only` keyword.
    ObliqueOnly,
}

impl FontSynthesisValue {
    /// CSS Fonts 4 initial value: `weight style small-caps position`.
    pub const fn initial() -> Self {
        Self {
            weight: true,
            style: FontSynthesisStyle::Auto,
            small_caps: true,
            position: true,
        }
    }

    /// The `none` value, with every synthesis component disabled.
    pub const fn none() -> Self {
        Self {
            weight: false,
            style: FontSynthesisStyle::None,
            small_caps: false,
            position: false,
        }
    }
}

/// The value of the `font-style` property.
///
/// CSS Fonts Module Level 4 §2.4 "Font style: the font-style property"
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>.
///
/// propdef (spec verbatim): Value: `normal | italic | left | right |
/// oblique <angle [-90deg,90deg]>?`; Initial: `normal`; Applies to: all
/// elements and text; Inherited: **yes**; Computed value: "the keyword
/// specified, plus angle in degrees if specified".
///
/// # Scope carving
///
/// - **Implemented as a bare keyword only**: `oblique` — accepted as a
///   standalone ident, [`FontStyle::Oblique`]. The optional `<angle
///   [-90deg,90deg]>` parameter that may follow it in the propdef grammar
///   quoted above is a **non-goal**: it needs its own payload-carrying
///   variant, range clamping, and the "plus angle in degrees" half of the
///   computed-value rule quoted above; deferred as a follow-up. `oblique`
///   with a trailing angle (e.g. `oblique 14deg`) is therefore rejected the
///   same as any other declaration `DeclParser` can't fully consume ([`mod@crate::rule`]'s
///   exhaustive-consumption check — same general mechanism noted on
///   [`TextTransform`]'s doc for its `||` combinator case).
/// - **Non-goal**: `left` / `right` — additional slant-direction keywords
///   in the same propdef grammar quoted above. Not implemented here;
///   silent drop like any other unhandled ident (below).
/// - **Partially supported**: The CSS-wide `inherit` keyword needed for an
///   inherited property is accepted and resolved to the parent's value at the
///   computed layer. Other CSS-wide keywords (`initial` / `unset` /
///   `revert` / `revert-layer`) are not implemented and are silently dropped
///   (see the canonical "CSS-wide keyword" section in the [`PropertyValue`] docs).
/// - **(a) Spec-invalid**: Idents other than the five keywords above are
///   silently dropped as `None` (`left` / `right` are covered by the Non-goal above).
///
/// With `oblique` accepted only as a bare keyword (no `<angle>` payload),
/// the angle-bearing branch of the spec's "Computed value" row stays
/// unreachable — so for this crate's scope, computed value = specified
/// keyword, no relative resolution needed (as in the [`Direction`] docs).
///
/// As with [`Direction`] / [`BoxSizing`], `Default` is intentionally not
/// derived: initialization sites ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) directly choose
/// [`FontStyle::Normal`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontStyle {
    /// `normal` — spec initial value.
    Normal,
    /// `italic`.
    Italic,
    /// `oblique` — bare keyword only, no `<angle>` payload (see the "Scope
    /// carving" section above).
    Oblique,
}

css_keywords!(FontStyle {
    Normal => "normal",
    Italic => "italic",
    Oblique => "oblique",
});

/// The value of the `font-variant-caps` property.
///
/// CSS Fonts Module Level 3 §6.6 "Capitalization: the font-variant-caps
/// property" <https://www.w3.org/TR/css-fonts-3/#font-variant-caps-prop>.
///
/// propdef (spec verbatim): Value: `normal | small-caps | all-small-caps |
/// petite-caps | all-petite-caps | unicase | titling-caps`; Initial:
/// `normal`; Applies to: all elements; Inherited: **yes**; Percentages: N/A;
/// Computed value: "as specified".
///
/// # Meaning of the seven keywords (verified against the spec verbatim)
///
/// - [`Normal`](Self::Normal) — "None of the features listed below are
///   enabled." Spec initial value.
/// - [`SmallCaps`](Self::SmallCaps) — "Enables display of small capitals
///   (OpenType feature: smcp). Small-caps glyphs typically use the form of
///   uppercase letters but are reduced to the size of lowercase letters."
/// - [`AllSmallCaps`](Self::AllSmallCaps) — "Enables display of small
///   capitals for both upper and lowercase letters (OpenType features:
///   c2sc, smcp)."
/// - [`PetiteCaps`](Self::PetiteCaps) — "Enables display of petite
///   capitals (OpenType feature: pcap)."
/// - [`AllPetiteCaps`](Self::AllPetiteCaps) — "Enables display of petite
///   capitals for both upper and lowercase letters (OpenType features:
///   c2pc, pcap)."
/// - [`Unicase`](Self::Unicase) — "Enables display of mixture of small
///   capitals for uppercase letters with normal lowercase letters
///   (OpenType feature: unic)."
/// - [`TitlingCaps`](Self::TitlingCaps) — "Enables display of titling
///   capitals (OpenType feature: titl). Uppercase letter glyphs are often
///   designed for use with lowercase letters. When used in all uppercase
///   titling sequences they can appear too strong. Titling capitals are
///   designed specifically for this situation."
///
/// # Scope carving
///
/// - **Non-goal**: the `font-variant` shorthand (CSS Fonts 3 §6.9 "Overall
///   shorthand for font rendering: the font-variant property"). Spec
///   verbatim: "Like other shorthands, using 'font-variant' resets
///   unspecified 'font-variant' subproperties to their initial values."
///   Its `||`-combinator grammar spans the value spaces of 5 subproperties
///   (`font-variant-ligatures` / `font-variant-caps` /
///   `font-variant-numeric` / `font-variant-east-asian` /
///   `font-variant-position`) — a reset behavior this crate can't
///   represent correctly while it has no longhand for the other 4.
///   Implementing only the `-caps` half of the shorthand would silently
///   drop that reset, which is worse than not accepting the shorthand name
///   at all — so `"font-variant"` has no [`parse_value`] dispatch arm, the
///   same reasoning [`WordBreak`]'s doc applies to the cross-property
///   `word-break: break-word` case.
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped (see the canonical "CSS-wide keyword" section in the
///   [`PropertyValue`] docs for the five-keyword list and reasons).
/// - **(a) Spec-invalid**: Idents other than the seven keywords above are silently dropped as `None`.
///
/// This crate's scope carries no length, so computed value = specified
/// keyword, with no relative resolution (as in the [`Direction`] docs).
///
/// # Downstream handoff
///
/// Replacing glyphs with the OpenType features named by these keywords (`smcp` /
/// `c2sc` / `pcap` / `c2pc` / `unic` / `titl`) belongs to text shaping and paint.
/// This crate only carries the keyword through the static cascade for that layer
/// (as in the "Downstream handoff" section in the [`TextTransform`] docs). Fallback
/// for fonts without the feature is likewise a downstream responsibility. Section
/// 6.6 specifies this fallback for all six non-`normal` keywords; the behavior
/// differs by keyword:
///
/// - `small-caps` / `all-small-caps`: a SHOULD-level synthesis fallback
///   (spec verbatim: "if 'small-caps' or 'all-small-caps' is specified but
///   small-caps glyphs are not available for a given font, user agents
///   should simulate a small-caps font").
/// - `petite-caps` / `all-petite-caps`: if the font does not support them,
///   each behaves as if `small-caps` / `all-small-caps`, respectively, were specified (spec
///   verbatim: "If either 'petite-caps' or 'all-petite-caps' is specified
///   for a font that doesn't support these features, the property behaves
///   as if 'small-caps' or 'all-small-caps', respectively, had been
///   specified").
/// - `unicase`: if the font does not support it, the behavior is as if
///   `small-caps` applied only to lowercased uppercase letters (spec verbatim: "If 'unicase'
///   is specified for a font that doesn't support that feature, the
///   property behaves as if 'small-caps' was applied only to lowercased
///   uppercase letters").
/// - `titling-caps`: if the font does not support it, there is no visible effect (spec verbatim: "If
///   'titling-caps' is specified with a font that does not support this
///   feature, this property has no visible effect").
///
/// As with [`FontStyle`] / [`Direction`], `Default` is intentionally not
/// derived: initialization sites ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) directly choose
/// [`FontVariantCaps::Normal`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariantCaps {
    /// `normal` — spec initial value.
    Normal,
    /// `small-caps`.
    SmallCaps,
    /// `all-small-caps`.
    AllSmallCaps,
    /// `petite-caps`.
    PetiteCaps,
    /// `all-petite-caps`.
    AllPetiteCaps,
    /// `unicase`.
    Unicase,
    /// `titling-caps`.
    TitlingCaps,
}

css_keywords!(FontVariantCaps {
    Normal => "normal",
    SmallCaps => "small-caps",
    AllSmallCaps => "all-small-caps",
    PetiteCaps => "petite-caps",
    AllPetiteCaps => "all-petite-caps",
    Unicase => "unicase",
    TitlingCaps => "titling-caps",
});

/// The value of the `text-transform` property.
///
/// CSS Text Module Level 4 property definition:
/// <https://www.w3.org/TR/css-text-4/#propdef-text-transform>.
///
/// propdef (spec verbatim): Value: `none | [capitalize | uppercase |
/// lowercase] || full-width || full-size-kana | math-auto`; Initial: `none`;
/// Applies to: text; Inherited: **yes**; Computed value: "specified keyword".
///
/// # Scope
///
/// Existing case and width keywords are preserved as computed values.
/// `full-width` maps ASCII characters to their full-width forms;
/// `full-size-kana` uses the CSS small kana mapping table, with
/// language-specific tailoring left to the layout consumer.
///
/// `math-auto` is likewise preserved as a standalone computed keyword only;
/// MathML Core's downstream behavior and any glyph or text transformation are
/// outside this crate's scope.
///
/// `Default` is intentionally not derived; initialization sites explicitly
/// choose [`TextTransform::None`] as the initial value.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    /// `none` — spec initial value.
    None,
    /// `math-auto` — a computed keyword; downstream math-text behavior is deferred.
    MathAuto,
    /// `capitalize` — first typographic letter of each word is titlecased.
    Capitalize,
    /// `uppercase` — all letters are uppercased.
    Uppercase,
    /// `lowercase` — all letters are lowercased.
    Lowercase,
    /// `full-width`.
    FullWidth,
    /// `full-size-kana`.
    FullSizeKana,
    /// Case transform combined with `full-width`.
    CapitalizeFullWidth,
    UppercaseFullWidth,
    LowercaseFullWidth,
    /// Case transform combined with `full-size-kana`.
    CapitalizeFullSizeKana,
    UppercaseFullSizeKana,
    LowercaseFullSizeKana,
    /// Both width transforms without a case transform.
    FullWidthFullSizeKana,
    /// A case transform combined with both width transforms.
    CapitalizeFullWidthFullSizeKana,
    UppercaseFullWidthFullSizeKana,
    LowercaseFullWidthFullSizeKana,
}

css_keywords!(@serialize TextTransform {
    None => "none",
    MathAuto => "math-auto",
    Capitalize => "capitalize",
    Uppercase => "uppercase",
    Lowercase => "lowercase",
    FullWidth => "full-width",
    FullSizeKana => "full-size-kana",
    CapitalizeFullWidth => "capitalize full-width",
    UppercaseFullWidth => "uppercase full-width",
    LowercaseFullWidth => "lowercase full-width",
    CapitalizeFullSizeKana => "capitalize full-size-kana",
    UppercaseFullSizeKana => "uppercase full-size-kana",
    LowercaseFullSizeKana => "lowercase full-size-kana",
    FullWidthFullSizeKana => "full-width full-size-kana",
    CapitalizeFullWidthFullSizeKana => "capitalize full-width full-size-kana",
    UppercaseFullWidthFullSizeKana => "uppercase full-width full-size-kana",
    LowercaseFullWidthFullSizeKana => "lowercase full-width full-size-kana",
});

/// The value of the `visibility` property.
///
/// CSS Display 3 §4 "Invisibility: the visibility property"
/// <https://www.w3.org/TR/css-display-3/#visibility>.
///
/// propdef (spec verbatim): Value: `visible | hidden | collapse`;
/// Initial: `visible`; Applies to: all elements; Inherited: **yes**;
/// Computed value: "as specified".
///
/// # Scope carving
///
/// - **`collapse`**: The spec says this keyword "can cause it to
///   take up less space than otherwise in a formatting-context–specific
///   way" and specifies that space-saving behavior only for table rows, columns,
///   row groups, and column groups (CSS2 dynamic row and column effects), and
///   flex items (CSS Flexbox 1 collapsed flex items). Otherwise, the spec says
///   "this simply makes the box invisible, just like
///   `visibility: hidden`". raikiri-style does not implement these space-saving
///   layout effects in any formatting context: this crate carries only the bare
///   computed keyword; context-specific behavior belongs downstream in layout.
///   Keep `collapse` distinct from `Hidden` so future layout code can tell them
///   apart rather than folding them into the same variant.
/// - **(b) Unsupported**: CSS-wide keywords are not implemented and are silently dropped
///   (see the canonical "CSS-wide keyword" section in the [`PropertyValue`] docs).
/// - **(a) Spec-invalid**: Idents other than the three keywords above are
///   silently dropped as `None`.
///
/// Computed value = specified keyword (per the spec's "Computed value: as
/// specified"; no relative resolution, as in the [`Direction`] docs).
///
/// As with [`Direction`] / [`FontStyle`], `Default` is intentionally not
/// derived: initialization sites ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) directly choose
/// [`Visibility::Visible`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// `visible` — spec initial value.
    Visible,
    /// `hidden`.
    Hidden,
    /// `collapse` — see the "Scope carving" section in the [`Visibility`] docs.
    Collapse,
}

css_keywords!(Visibility {
    Visible => "visible",
    Hidden => "hidden",
    Collapse => "collapse",
});

/// The value of the `line-height` property (an author CSS type that lays
/// the groundwork for inline layout).
///
/// CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#line-height-property>) defines the grammar
/// `normal | <number [0,∞]> | <length-percentage [0,∞]>`.
///
/// Three variants preserve the top-level alternatives in the spec:
///
/// - [`Normal`](Self::Normal) — the initial value. Downstream paint uses a line
///   height based on font metrics (roughly ascent + descent).
/// - [`Number`](Self::Number) — a unitless multiplier. `line-height: 1.5` is
///   1.5 times the used element's computed `font-size`. **Special spec behavior**:
///   a child inherits the **specified value** of a unitless number (the raw
///   multiplier, not a resolved length). The static cascade keeps that raw
///   value in a separate variant from Length so downstream paint can preserve
///   the number-versus-length distinction (§5.1 "When a child element inherits...").
/// - [`Length`](Self::Length) — a `<length-percentage>` payload. `Length::Percent`
///   means a **percentage of the element's own font-size** (§5.1).
///   `Length::Em`/`Rem`/`Px`/`Pt` follow the usual length-resolution context.
///
/// # Non-negative constraint
///
/// The spec grammars `<number [0,∞]>` and `<length-percentage [0,∞]>` exclude
/// negative values. The parser drops them in the `parse_line_height` post-filter
/// (spec-invalid → drop). Rejecting them at parse time matches the spec's
/// range constraint; it is not a stricter restriction.
///
/// # Primary source
///
/// - CSS Inline 3 §5.1 "Line Spacing: the line-height property"
///   (<https://www.w3.org/TR/css-inline-3/#line-height-property>) —
///   "specifies the box's preferred line height, which is used in calculating
///   its layout bounds"
///
/// Downstream matches must have a wildcard arm: `#[non_exhaustive]` allows new
/// variants without breaking existing pattern matches, as with sibling types
/// [`Length`] and [`DisplayValue`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    /// `normal` — the initial value. Paint uses a default line height based
    /// roughly on the font metrics' ascent and descent.
    Normal,
    /// `<number [0,∞]>` — a unitless multiplier. `1.5` → `Number(1.5)`.
    /// At resolution time, multiply the element's computed `font-size` by this
    /// value. The spec says children inherit the number's **specified value**;
    /// this distinction from the Length variant must be preserved.
    Number(f32),
    /// `<length-percentage [0,∞]>` — a length or percentage.
    /// `24px` → `Length(Length::Px(24.0))`; `150%` → `Length(Length::Percent(150.0))`.
    /// Per spec §5.1, `Length::Percent` is relative to the element's own font size.
    Length(Length),
}

/// The parsed result of `<counter-style>`.
///
/// It occurs as the optional third argument of `counter()` / `counters()` in
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>,
/// and as the optional final argument of `target-counter()` /
/// `target-counters()` in CSS Content 3 §2.6. When `counter-style?` is omitted,
/// the spec defaults to `decimal`.
///
/// The static side passes named styles through as SmolStr. Downstream code
/// interprets names such as `decimal-leading-zero`, `upper-alpha`, and
/// `lower-roman` when formatting the counter tree at runtime.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CounterStyle {
    /// `decimal` — the spec default, also used when `<counter-style>?` is omitted.
    #[default]
    Decimal,
    /// A named counter style other than `decimal`, with its case preserved in SmolStr.
    Named(SmolStr),
}

/// The computed value of `list-style-type`.
///
/// CSS Lists 3 §3.1 <https://www.w3.org/TR/css-lists-3/#list-style-type>.
/// Built-in counter styles and author-defined `@counter-style` names are kept
/// as an identifier so the layout/paint side can resolve them at marker time.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ListStyleType {
    /// `disc` — the initial value.
    #[default]
    Disc,
    /// `none` — suppress the marker box.
    None,
    /// A named built-in or author-defined counter style.
    Named(SmolStr),
    /// An author-supplied marker string (`<string>`).
    String(SmolStr),
}

/// The three longhands set by the `list-style` shorthand.
#[derive(Clone, Debug, PartialEq)]
pub struct ListStyleShorthand {
    /// Marker text or counter style.
    pub kind: ListStyleType,
    /// Marker position relative to the principal box.
    pub position: ListStylePosition,
    /// Image used instead of marker text when available.
    pub image: BackgroundImage,
}

/// The computed value of `list-style-position`.
///
/// CSS Lists 3 §3.2 <https://www.w3.org/TR/css-lists-3/#list-style-position>.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListStylePosition {
    /// Marker is laid out outside the principal block.
    #[default]
    Outside,
    /// Marker participates in the first line of the principal block.
    Inside,
}

css_keywords!(ListStylePosition {
    Outside => "outside",
    Inside => "inside",
});

/// The optional second argument of `string()`:
/// `[ first | start | last | first-except ]?`.
///
/// CSS Content 3 §2.7.2 "Inserting Named Strings: the string() function"
/// <https://www.w3.org/TR/css-content-3/#string-function>.
/// The spec defaults to `first`; the `first` definition says: "If no second
/// argument is provided, this is the default value."
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StringFetchMode {
    /// `first` — the spec default.
    #[default]
    First,
    /// `start`.
    Start,
    /// `last`.
    Last,
    /// `first-except`.
    FirstExcept,
}

css_keywords!(StringFetchMode {
    First => "first",
    Start => "start",
    Last => "last",
    FirstExcept => "first-except",
});

/// The second argument of `target-text()`:
/// `[ content | before | after | first-letter ]?`.
///
/// CSS Content 3 §2.6.3 "The target-text() function"
/// <https://www.w3.org/TR/css-content-3/#target-text> gives this production:
/// `target-text() = target-text( [ <string> | <url> ] , [ content | before |
/// after | first-letter ]? )`. The `?` makes the second argument syntactically
/// optional, unlike the `content()` argument in GCPM 3 §1.1.1.1. Only these
/// two sentences describe the keywords: "The target-text() function retrieves
/// the text value of the element referred to by the URL. An optional second
/// argument specifies what content is retrieved, using the same values as
/// the string-set property above." There is no separate definition or "if
/// omitted" sentence for the second argument's keywords, unlike those of
/// `string()`. The first sentence describes "the text value of the element",
/// which matches [`Content`](Self::Content): the target element's own string
/// value. That semantic correspondence, rather than an explicit spec statement
/// of a "default", motivates using [`Content`](Self::Content) when the keyword
/// is omitted. An earlier claim of an explicit default at this site was
/// corrected after the discrepancy was found.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentPart {
    /// `content` — the target element's own string value. It corresponds to
    /// "the text value of the element" in the `target-text()` description.
    /// It is used when the keyword is omitted, but the spec does not explicitly
    /// declare it the default (see the type-level documentation).
    #[default]
    Content,
    /// `before` — the string value of the `::before` pseudo-element.
    Before,
    /// `after` — the string value of the `::after` pseudo-element.
    After,
    /// `first-letter` — the string of the `::first-letter` pseudo-element.
    FirstLetter,
}

css_keywords!(ContentPart {
    Content => "content",
    Before => "before",
    After => "after",
    FirstLetter => "first-letter",
});

/// The argument of `content()`: `[ text | before | after | first-letter ]?`.
/// The `?` describes what Raikiri accepts, consistent with the spec's bare
/// `content()` example, `h2 { string-set: heading content() }`. It is not a
/// formal optional marker in the GCPM 3 grammar itself.
///
/// CSS GCPM 3 §1.1.1.1 "The content() function"
/// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> gives this production:
/// `content() = content([text | before | after | first-letter])`. Using
/// [`Text`](Self::Text) when the keyword is omitted is not based on a stable
/// spec declaration of a default. The grammar in that section has no `?`,
/// so the sole argument is not syntactically optional. Although the `text`
/// definition says "This is the default value", the same section still has
/// an unresolved WG issue about how to define that default. An earlier
/// overclaim at this site was corrected after it was found.
///
/// NB: The keywords overlap with sibling [`ContentPart`] (for `target-text()`),
/// but the spec spells `text` and `content` differently. Keep separate types,
/// as with the per-function StringFetchMode and ContentPart enums; do not
/// silently accept `content(content)`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentTextKeyword {
    /// `text` — the element's full string value, determined as if
    /// `white-space: normal` applied. Used when the keyword is omitted, but
    /// not on the basis of a stable spec declaration of a default (see above).
    #[default]
    Text,
    /// `before` — the string value of the `::before` pseudo-element.
    Before,
    /// `after` — the string value of the `::after` pseudo-element.
    After,
    /// `first-letter` — the string of the `::first-letter` pseudo-element.
    FirstLetter,
}

css_keywords!(ContentTextKeyword {
    Text => "text",
    Before => "before",
    After => "after",
    FirstLetter => "first-letter",
});

/// The four keywords of the `<quote>` production.
///
/// CSS Content 3 §2.4.2 "Inserting Quotation Marks: the *-quote keywords"
/// <https://www.w3.org/TR/css-content-3/#quote-values> gives this production:
/// `<quote> = open-quote | close-quote | no-open-quote | no-close-quote`.
///
/// Per the spec, [`OpenQuote`](Self::OpenQuote) / [`CloseQuote`](Self::CloseQuote)
/// are "replaced by the appropriate string as defined by the `quotes`
/// property" and change the nesting depth. [`NoOpenQuote`](Self::NoOpenQuote) /
/// [`NoCloseQuote`](Self::NoCloseQuote) "Inserts nothing (as in none)", but
/// still change that depth. Looking up the actual string in
/// [`PropertyValue::Quotes`] by nesting depth is outside this crate's static-side
/// scope. Downstream raikiri-dom resolves it at runtime using the computed
/// `quotes` value, just as downstream resolves [`CounterStyle`] and
/// [`StringFetchMode`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteKeyword {
    /// `open-quote` — increment the nesting depth and insert the matching
    /// opening quote string (resolved downstream).
    OpenQuote,
    /// `close-quote` — decrement the nesting depth and insert the matching
    /// closing quote string.
    CloseQuote,
    /// `no-open-quote` — insert nothing, but increment the nesting depth as
    /// `open-quote` does.
    NoOpenQuote,
    /// `no-close-quote` — insert nothing, but decrement the nesting depth as
    /// `close-quote` does.
    NoCloseQuote,
}

css_keywords!(QuoteKeyword {
    OpenQuote => "open-quote",
    CloseQuote => "close-quote",
    NoOpenQuote => "no-open-quote",
    NoCloseQuote => "no-close-quote",
});

/// The `<leader-type> = dotted | solid | space | <string>` argument of `leader()`.
///
/// CSS Content 3 §2.5.1 "The leader() function"
/// <https://www.w3.org/TR/css-content-3/#leader-function>.
///
/// The spec calls `dotted` "equivalent to `leader(".")`", `solid`
/// "equivalent to `leader("_")`", and `space` "equivalent to `leader(" ")`".
/// These equivalences describe **keyword semantics, not a normalization of
/// their spelling**. This follows [`counter_style_from_ident`], which retains
/// `decimal` in its own [`CounterStyle::Decimal`] variant instead of folding
/// it into `Named("decimal")`. Keep the three keywords as distinct variants;
/// downstream paint resolves their actual leader glyph strings (`Dotted` →
/// `"."`, for example) during rendering. The [`String`](Self::String) variant
/// stores custom leader strings in [`SmolStr`], following the SmolStr conversion
/// of [`ContentComponent::Literal`] to make short-lived clones cheap.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaderType {
    /// `dotted` — equivalent to `leader(".")` in the spec; rendered downstream.
    Dotted,
    /// `solid` — equivalent to `leader("_")` in the spec.
    Solid,
    /// `space` — equivalent to `leader(" ")` in the spec.
    Space,
    /// `<string>` — an author-specified custom leader string.
    String(SmolStr),
}

/// Selects the list vocabulary used by `parse_content_list_items`.
///
/// CSS Content 3 §2 <https://www.w3.org/TR/css-content-3/#content-values> and
/// CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list> both
/// define `<content-list>`, but GCPM 3 defines a narrower local production.
/// Its `Link defaults` do not include CSS Content 3, and §1.1.1 L82 defines
/// `<content-list> = [ <string> | <counter()> | <counters()> | <content()> |
/// <attr()> ]+` independently. The accepted functions differ by property,
/// so dispatch selects a mode, following the per-context enum convention of
/// [`StringFetchMode`] / [`ContentPart`] / [`ContentTextKeyword`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentListMode {
    /// The broad CSS Content 3 §2 `<content-list>` for the `content` property.
    /// Accepts: bare `<string>` literals / `counter()` / `counters()` / `string()` /
    /// `attr()` / `target-counter()` / `target-counters()` / `target-text()` /
    /// `content()` / `<image>` (only the `url()` alternative) /
    /// the `contents` keyword / `<quote>` (such as `open-quote`) / `leader()`.
    /// This full set of ten alternatives adds image/contents/quote/leader,
    /// fixing the former implementation's under-acceptance.
    CssContent3,
    /// The narrow local CSS GCPM 3 §1.1.1 `<content-list>` for `string-set`.
    /// Accepts bare `<string>` literals / `counter()` / `counters()` / `content()` /
    /// `attr()`. **Explicitly rejects** `string()` (distinct from a bare
    /// `<string>` literal), `target-counter()`, `target-counters()`, and
    /// `target-text()`. This follows the verbatim grammar at GCPM 3 §1.1.1 L82;
    /// dropping the declaration during cascade yields the spec-compliant
    /// shadowing behavior.
    GcpmStringSet,
}

/// A `content` property value item — an intermediate representation on the static side of the cascade.
///
/// Maps 1:1 to `raikiri_traits::ContentValueItem` from design doc §7.1
/// (translated by downstream raikiri-dom during runtime resolution). Because
/// raikiri-style is a leaf crate that does not depend on raikiri-traits, it
/// stores a **local** intermediate type, following the counter-* wire-through
/// pattern, and maps it to the shared trait type downstream.
///
/// Variants follow the order of the functions in the specifications:
/// - Literal: bare `<string>` (§2.1)
/// - Counter / Counters: CSS Lists 3 §4.7
///   <https://www.w3.org/TR/css-lists-3/#counter-functions>
/// - String: CSS Content 3 §2.7.2 <https://www.w3.org/TR/css-content-3/#string-function>
/// - Element: CSS GCPM 3 §1.2.2 <https://www.w3.org/TR/css-gcpm-3/#element-syntax>
/// - Attr: CSS Content 3 §2.1 <https://www.w3.org/TR/css-content-3/#strings>
/// - Target*: CSS Content 3 §2.6.1-3
///   <https://www.w3.org/TR/css-content-3/#target-counter>,
///   <https://www.w3.org/TR/css-content-3/#target-counters>,
///   <https://www.w3.org/TR/css-content-3/#target-text>
/// - Content: CSS GCPM 3 §1.1.1.1
///   <https://www.w3.org/TR/css-gcpm-3/#funcdef-content>
/// - Image / Contents / Quote / Leader: CSS Content 3 §2.2 / §2.3 / §2.4.2 /
///   §2.5.1 (appended as an under-acceptance fix, preserving the order of
///   existing variants for compatibility)
///
/// URLs are kept as raw `String` values (raikiri-style does not depend on the
/// `url` crate; consumers parse them as `url::Url` during runtime resolution).
///
/// # `#[non_exhaustive]` semantics (verbatim guidance for fulgur / downstream consumers)
///
/// Enum-level `#[non_exhaustive]` makes new variants forward-compatible by
/// requiring an `_ =>` arm in downstream `match` expressions, but **does not
/// block calls to existing variants' tuple constructors**. Changing an
/// existing variant's payload **type** therefore breaks downstream constructors
/// at compile time.
///
/// As part of the cascade memory DoS mitigation, the payload of
/// [`Literal`](Self::Literal) changed from `String` to [`SmolStr`]. Because
/// SmolStr provides `Deref<Target = str>`, consumers that **read** the payload
/// by pattern matching can still use `&str` APIs such as
/// `match cc { ContentComponent::Literal(s) => &*s, .. }`, `s.as_str()`, and
/// `s.len()` unchanged. Only consumers that **construct** it must replace
/// `ContentComponent::Literal("foo".into())` with
/// `ContentComponent::Literal(SmolStr::new("foo"))` (or use the appropriate
/// `From` impl where `.into()` is valid).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentComponent {
    /// Bare `<string>` literal (`content: "hello"`).
    ///
    /// [`SmolStr`] stores up to 22 bytes inline; longer values use an internal
    /// `Arc<str>`, making a clone an O(1) bump. This is a secondary defense
    /// against the direct DoS vector `content: "<large>"`; the primary defense
    /// is the [`Arc<Vec<..>>`] wrapper on the outer [`PropertyValue::Content`].
    ///
    /// **For consumers**:
    /// The previous payload type was `String`. SmolStr provides
    /// `Deref<Target = str>`, so read-side operations (`&*s` / `s.as_str()` /
    /// `s.len()` / `for c in s.chars()`) continue to work unchanged. Only the
    /// construct-side needs to use `SmolStr::new("foo")` (or
    /// `SmolStr::from(String)`). See also the enum-level documentation on
    /// §`#[non_exhaustive]` semantics.
    Literal(SmolStr),
    /// `counter(<counter-name>, <counter-style>?)`.
    Counter { name: SmolStr, style: CounterStyle },
    /// `counters(<counter-name>, <string>, <counter-style>?)`.
    Counters {
        name: SmolStr,
        separator: String,
        style: CounterStyle,
    },
    /// `string(<custom-ident>, [ first | start | last | first-except ]?)`.
    String {
        name: SmolStr,
        fetch: StringFetchMode,
    },
    /// `element(<custom-ident>, [ first | start | last | first-except ]?)` —
    /// CSS GCPM 3 §1.2.2 <https://www.w3.org/TR/css-gcpm-3/#element-syntax>.
    ///
    /// The running-element name and the page-relative selection keyword are
    /// retained for the downstream margin-box resolver. The keywords are the
    /// same as `string()`'s, with the same `first` default.
    Element {
        name: SmolStr,
        fetch: StringFetchMode,
    },
    /// `attr(<attribute-name>)` (§2.1).
    ///
    /// The legacy untyped form resolves a missing attribute to an empty
    /// string. Typed values and fallbacks use [`Self::AttrFallback`].
    Attr { name: SmolStr },
    /// Untyped `attr(<attribute-name>, <fallback>)` with a narrow static
    /// fallback subset: a quoted string is retained, while any other
    /// fallback token is represented as invalid and therefore contributes no
    /// generated text when the attribute is absent.
    AttrFallback {
        name: SmolStr,
        fallback: Option<SmolStr>,
    },
    /// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`.
    /// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>.
    ///
    /// The second argument is `<custom-ident>`, not `<counter-name>` — the
    /// specification's verbatim value definition (§2.6.1) is:
    ///
    /// ```text
    /// target-counter() = target-counter( [ <string> | <url> ] , <custom-ident> , <counter-style>? )
    /// ```
    ///
    /// Unlike `counter()` / `counters()` (CSS Lists 3 §4 `<counter-name>`
    /// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>), this does
    /// not additionally exclude `none`.
    TargetCounter {
        url: String,
        name: SmolStr,
        style: CounterStyle,
    },
    /// `target-counters([<string>|<url>], <custom-ident>, <string>, <counter-style>?)`.
    /// CSS Content 3 §2.6.2 <https://www.w3.org/TR/css-content-3/#target-counters>.
    ///
    /// The second argument is `<custom-ident>`, not `<counter-name>` — the
    /// specification's verbatim value definition (§2.6.2) is:
    ///
    /// ```text
    /// target-counters() = target-counters( [ <string> | <url> ] , <custom-ident> , <string> , <counter-style>? )
    /// ```
    ///
    /// Unlike `counter()` / `counters()` (CSS Lists 3 §4 `<counter-name>`
    /// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>), this does
    /// not additionally exclude `none`.
    TargetCounters {
        url: String,
        name: SmolStr,
        separator: String,
        style: CounterStyle,
    },
    /// `target-text([<string>|<url>], [ content | before | after | first-letter ]?)`.
    TargetText { url: String, part: ContentPart },
    /// `content([ text | before | after | first-letter ]?)` — GCPM 3 §1.1.1.1
    /// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> (`?` is part of the
    /// syntax accepted by raikiri, not the specification's grammar).
    /// Inserts the string value of the current element (or pseudo-element)
    /// into a named string. As a member of `<content-list>`, it is accepted in
    /// both `string-set` and the `content` property's content-list. When the
    /// keyword is omitted, [`ContentTextKeyword::Text`] is used as the fallback
    /// (not because the specification declares it the "default" — see the
    /// [`ContentTextKeyword`] documentation). Runtime resolution belongs to
    /// raikiri-dom (the wire-through pattern).
    Content { keyword: ContentTextKeyword },
    /// `<image>` (CSS Images 3 <https://www.w3.org/TR/css-images-3/#typedef-image>
    /// `<image> = <url> | <gradient>`) — CSS Content 3 §2.2 "2D Images: the
    /// `<image>` values" <https://www.w3.org/TR/css-content-3/#content-uri>.
    /// Specification verbatim: "Represents an anonymous inline replaced element filled
    /// with the specified `<image>`. If the `<image>` represents an invalid
    /// image, this value instead represents nothing" (rendering fallback is
    /// the downstream consumer's responsibility).
    ///
    /// **Relationship to `<content-replacement>` (not implemented; future task)**:
    /// The value definition for the whole `content` property (CSS Content 3 §1
    /// <https://www.w3.org/TR/css-content-3/#content-property>) is `normal |
    /// none | [ <content-replacement> | <content-list> ] […]?`, and
    /// `<content-replacement> = <image>` is a separate top-level alternative
    /// to `<content-list>` — specification verbatim: "Makes the element or
    /// pseudo-element a replaced element, filled with the specified
    /// `<image>`". It has different semantics from the list-item `<image>`
    /// above (an anonymous inline replaced element), such as suppressing
    /// `::before`/`::after` generation. The specification also states: "If the value of `<content-list>` is a
    /// single `<image>`, it must instead be interpreted as a
    /// `<content-replacement>`". This variant's shape
    /// (whether the sole item in `Vec<ContentComponent>` is `Image`) retains
    /// enough information for downstream consumers to reconstruct that
    /// distinction. Implementing replacement semantics, including suppressing
    /// pseudo-elements, is outside this crate's static-side scope.
    ///
    /// **(b) Unsupported (spec-valid)**: Only the `<url>` alternative is
    /// implemented (`url(...)` / `url("...")`). `<gradient>` (`linear-gradient()` /
    /// `repeating-linear-gradient()` / `radial-gradient()` /
    /// `repeating-radial-gradient()`, CSS Images 3 §3.1-2) is deferred because
    /// this crate lacks gradient-stop and color-interpolation infrastructure
    /// (out of scope; tracked as a follow-up task). `image()` / `image-set()` /
    /// `element()` / `cross-fade()` / `paint()`, added in CSS Images 4, are
    /// absent from the referenced CSS Images **3** `<image>` production and
    /// therefore spec-invalid (at Level 3). Their function names do not match
    /// any `parse_content_function` match arm, so they are dropped without
    /// additional code.
    ///
    /// URLs are kept as raw `String` values (the same convention as sibling
    /// [`TargetCounter`](Self::TargetCounter) and others; no dependency on the
    /// `url` crate).
    Image { url: String },
    /// `contents` keyword — CSS Content 3 §2.3 "Elemental Content: the
    /// `contents` keyword" <https://www.w3.org/TR/css-content-3/#element-content>.
    /// Specification verbatim: "The element's descendants". Resolving whether
    /// pseudo-elements are generated and the consumption order (do nothing if
    /// already used by another pseudo-element) is outside this crate's
    /// static-side scope. As with `normal`/`none` in the parse_content
    /// documentation, downstream consumers decide whether to generate content.
    ///
    /// **Asymmetry with `normal` (intentional)**: The specification states
    /// verbatim (§2.3) that "the initial
    /// value of content is `normal` and `normal` computes to `contents` on an
    /// element". However,
    /// [`parse_content`] stores `normal` as an empty `Vec`; expanding `normal`
    /// to `contents` at computed-value time is outside this crate's static-side
    /// (specified-value layer) scope. In contrast, explicit `content: none` is
    /// retained as a [`ContentComponent::None`] sentinel so pseudo-element
    /// consumers can suppress box generation. Therefore explicitly authored
    /// `content: contents` returns `[Contents]`, while `content: normal`
    /// (equivalent to the initial value) returns `[]`.
    Contents,
    /// Internal sentinel for an explicit `content: none` declaration.
    ///
    /// `normal` remains the empty list used for the initial value. Keeping
    /// `none` distinct lets downstream pseudo-element consumers suppress
    /// generated content without changing the public `PropertyValue` shape.
    None,
    /// `<quote>` (`open-quote` / `close-quote` / `no-open-quote` /
    /// `no-close-quote`) — CSS Content 3 §2.4.2
    /// <https://www.w3.org/TR/css-content-3/#quote-values>. See the
    /// [`QuoteKeyword`] documentation for details. Downstream consumers
    /// resolve the actual quotation marks alongside the computed value of
    /// [`PropertyValue::Quotes`].
    Quote(QuoteKeyword),
    /// `leader(<leader-type>)` — CSS Content 3 §2.5.1 "The leader() function"
    /// <https://www.w3.org/TR/css-content-3/#leader-function>. See the
    /// [`LeaderType`] documentation for details. The specification's production
    /// `leader( <leader-type> )` has no `?`, so the argument is required (bare
    /// `leader()` is spec-invalid → parsing fails and the declaration is dropped).
    Leader(LeaderType),
}

/// The value of the `display` property.
///
/// CSS Display 3 §2 "Box Layout Modes: the display property"
/// <https://www.w3.org/TR/css-display-3/#propdef-display>:
/// The value grammar is
/// `[ <display-outside> || <display-inside> ] | <display-listitem> |
/// <display-internal> | <display-box> | <display-legacy>`; the initial value
/// is `inline`, and the property is not inherited.
///
/// Currently accepts 19 keywords: `block` / `inline` / `inline-block`
/// / `none` / `flex` / `grid` / `list-item` / `contents` / `table`
/// / `inline-table` / `table-row-group` / `table-header-group`
/// / `table-footer-group` / `table-row` / `table-column-group`
/// / `table-column` / `table-cell` / `table-caption` / `flow-root`.
/// `flow-root` establishes a standalone block formatting context.
/// For other (future) keywords, `parse_display` returns `None` and the
/// declaration is silently dropped (the invalid-value drop path in rule.rs).
///
/// For `list-item`, only keyword acceptance of `<display-listitem> =
/// <display-outside>? && [ flow | flow-root ]? && list-item` is implemented
/// (the outer-defaulting rule makes an omitted `<display-outside>` block).
/// Generating a list-item box (a principal box plus a marker box under CSS
/// Lists 3 §2.2) and resolving the `::marker` pseudo-element are separate
/// layout work, dependent on the marker/list-style-type generated-content
/// machinery. Thus this crate only retains `display: list-item` as a keyword.
/// Approximating it as a block-level principal box without a marker box is
/// compatible with CSS2.1 §12.5.1, "a list-item's principal
/// box is block-level"
/// (that sentence does not make its claim conditional on the presence of a
/// marker box).
///
/// `#[non_exhaustive]` makes adding variants non-breaking (the additions of
/// InlineBlock / None / Flex / Grid / ListItem / Contents are forward-compatible
/// through this attribute).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayValue {
    /// `block` — CSS Display 3 §2 `<display-outside>` short form for
    /// "block flow" (block-level box containing block flow layout).
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-outside>
    Block,
    /// `inline` — CSS Display 3 §2 `<display-outside>` short form for
    /// "inline flow" (inline-level box containing inline flow layout).
    /// The specification's default (initial value).
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-outside>
    Inline,
    /// `inline-block` — CSS Display 3 §2 `<display-legacy>` short form for
    /// "inline flow-root" (an inline-level block container, primarily for
    /// button-like layout).
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-legacy>
    InlineBlock,
    /// `flow-root` — CSS Display 3 §2.5, a block-level box that establishes
    /// an independent block formatting context.
    /// <https://www.w3.org/TR/css-display-3/#valdef-display-flow-root>
    FlowRoot,
    /// `none` — CSS Display 3 §2 `<display-box>`: omits the element
    /// (including its subtree) from the box tree (like hidden).
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-box>
    None,
    /// `flex` — CSS Display 3 §2.2 "Inner Display Layout Models"
    /// `<display-inside>` short form for a flex formatting context.
    /// When `<display-outside>` is omitted, the outer display type defaults
    /// to block (the outer-defaulting rule in §2.2), so `display: flex` is
    /// equivalent to `display: block flex` (a block-level box containing a
    /// flex formatting context). The informative summary table in §2 also
    /// states this equivalence explicitly.
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-inside>
    /// <https://www.w3.org/TR/css-display-3/#the-display-properties>
    Flex,
    /// `inline-flex` — inline-level outer box establishing a flex formatting
    /// context. The DOM bridge uses this distinction for shrink-to-fit sizing.
    InlineFlex,
    /// `grid` — CSS Display 3 §2.2 "Inner Display Layout Models"
    /// `<display-inside>` short form for a grid formatting context.
    /// When `<display-outside>` is omitted, the outer display type defaults
    /// to block (the outer-defaulting rule in §2.2), so `display: grid` is
    /// equivalent to `display: block grid` (a block-level box containing a
    /// grid formatting context). The informative summary table in §2 also
    /// states this equivalence explicitly.
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-inside>
    /// <https://www.w3.org/TR/css-display-3/#the-display-properties>
    Grid,
    /// `inline-grid` — inline-level outer box establishing a grid formatting
    /// context. The DOM bridge uses this distinction for shrink-to-fit sizing.
    InlineGrid,
    /// `list-item` — CSS Display 3 §2 `<display-listitem>`
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-listitem>,
    /// HTML Living Standard's default UA stylesheet rule for `li`
    /// (`li { display: list-item; text-align: match-parent; }`)
    /// <https://html.spec.whatwg.org/multipage/rendering.html#lists>.
    /// When `<display-outside>` is omitted, the outer display type defaults
    /// to block (analogous to the outer-defaulting rule in §2.2), so
    /// `display: list-item` is equivalent to `display: block flow list-item`.
    ///
    /// Only keyword acceptance is implemented here. Adding the marker box
    /// to the list-item principal box (CSS Lists 3 §2.2) and resolving the
    /// `::marker` pseudo-element are outside this variant's scope (separate
    /// layout work dependent on generated-content/list machinery).
    ListItem,
    /// `contents` — CSS Display Module Level 3 §2.5 "Box Generation: the
    /// none and contents keywords"
    /// <https://www.w3.org/TR/css-display-3/#valdef-display-contents>: the
    /// element itself generates no box at all — as if it had been replaced
    /// in the document tree by its children (and any pseudo-elements it
    /// generates). Contrast with [`DisplayValue::None`], which suppresses
    /// box generation for the element **and** its whole subtree.
    ///
    /// # Consumer-side box generation gap (known, not worked around here)
    ///
    /// Parsing, cascading, and inheritance treat `contents` like any other
    /// keyword — none of that requires knowing the element's position in
    /// the box tree, so this crate's side is complete. Actually
    /// **generating** the correct box tree for `contents` is a different
    /// problem: the layout tree builder has to promote the element's
    /// children up to take its own place, skipping its own box while its
    /// children still lay out as normal. That is a tree transformation, not
    /// a value mapping, and this crate has no layout tree to transform (it
    /// only produces per-element [`crate::computed::ComputedValues`]) — so
    /// it is out of scope here by construction.
    ///
    /// At the time this variant was added, raikiri-dom's `bridge_display`
    /// (the function that maps [`DisplayValue`] to `taffy::Style::display`)
    /// has a catch-all arm that maps any display type it does not
    /// specifically recognize to a plain block box, and `taffy`'s own
    /// `Display`/`BoxGenerationMode` types have no "generate no box, but
    /// still lay out children" mode to map `contents` onto correctly
    /// either way. So a `display: contents` element will incorrectly still
    /// generate a box there (a spurious box, not merely an approximation)
    /// until real children-promotion support lands on the consumer side.
    /// That gap is intentionally not papered over here: mapping `Contents`
    /// to `Block` at the cascade layer would make the wrong behavior
    /// unobservable instead of fixing it.
    ///
    /// A second, independent consumer reads this field directly rather
    /// than through the taffy bridge above: raikiri-paint's
    /// `vertical_align_shift_px` (`walk.rs`) gates its `vertical-align`
    /// `sub`/`super` shift on `matches!(display, Inline | InlineBlock)`.
    /// Before this variant existed, `display: contents` failed to parse
    /// and the declaration was dropped, so an element that specified it
    /// kept whatever `display` its other declarations (or the `Inline`
    /// initial value) produced — which could satisfy that gate. Now that
    /// `display: contents` parses and computes to `Contents`, such an
    /// element no longer matches the gate and the shift is not applied.
    /// This happens to move the element closer to spec-correct (per CSS2
    /// §10.8 "Line height calculations: the 'line-height' and
    /// 'vertical-align' properties" and CSS Inline 3, the property applies
    /// to inline-level and table-cell boxes, and a `contents` element has
    /// no box of its own to shift) — but it is a real, previously-untested
    /// paint-visible behavior change introduced by this crate's cascade
    /// output, worth knowing about independently of the box-generation gap
    /// above.
    ///
    /// # Root element (not yet implemented)
    ///
    /// CSS Display Module Level 3 §2.8 "The Root Element's Principal Box"
    /// <https://www.w3.org/TR/css-display-3/#root>, verbatim: "a `display`
    /// of `contents` computes to `block` on the root element." This crate
    /// **does** track which element is the root during the inheritance walk
    /// ([`crate::cascade::walk_from`]'s root-element detection,
    /// used today to pick the `rem`/`rlh` resolution basis — see that
    /// function's doc) — but that tracking is not wired into `display`
    /// computation for any variant, so this root-element blockification
    /// rule for `contents` is not implemented (nor is any other
    /// `display`-specific root transformation).
    Contents,
    /// `table` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2 "The CSS table model"
    /// <https://www.w3.org/TR/css-display-3/#propdef-display>
    /// <https://www.w3.org/TR/CSS2/tables.html#table-display>: block-level table wrapper box.
    Table,
    /// `inline-table` — CSS Display 3 §2 / CSS 2.1 §17.2: inline-level table wrapper box
    /// (inline-outside, table-inside). Floated `inline-table` computes to `table` per CSS2 §9.7.
    InlineTable,
    /// `table-row-group` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: groups rows (`<tbody>`).
    TableRowGroup,
    /// `table-header-group` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: header rows (`<thead>`).
    TableHeaderGroup,
    /// `table-footer-group` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: footer rows (`<tfoot>`).
    TableFooterGroup,
    /// `table-row` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: single row (`<tr>`).
    TableRow,
    /// `table-column-group` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: groups columns (`<colgroup>`).
    TableColumnGroup,
    /// `table-column` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: single column (`<col>`).
    TableColumn,
    /// `table-cell` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: cell (`<td>`, `<th>`).
    TableCell,
    /// `table-caption` — CSS Display 3 §2 `<display-internal>` / CSS 2.1 §17.2: caption (`<caption>`).
    TableCaption,
}

css_keywords!(DisplayValue {
    Block => "block",
    Inline => "inline",
    InlineBlock => "inline-block",
    FlowRoot => "flow-root",
    None => "none",
    Flex => "flex",
    InlineFlex => "inline-flex",
    Grid => "grid",
    InlineGrid => "inline-grid",
    ListItem => "list-item",
    Contents => "contents",
    Table => "table",
    InlineTable => "inline-table",
    TableRowGroup => "table-row-group",
    TableHeaderGroup => "table-header-group",
    TableFooterGroup => "table-footer-group",
    TableRow => "table-row",
    TableColumnGroup => "table-column-group",
    TableColumn => "table-column",
    TableCell => "table-cell",
    TableCaption => "table-caption",
});

/// The value of the `flex-direction` property.
///
/// CSS Flexible Box Layout Module Level 1 §5.1 "Flex Flow Direction: the
/// flex-direction property"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>: value
/// grammar `row | row-reverse | column | column-reverse`; the property definition
/// lists "Initial: row", "Inherited: no", "Applies to: flex containers", and
/// "Computed value: specified keyword".
///
/// `#[non_exhaustive]` — the same forward-compatibility contract as [`DisplayValue`].
/// Like its siblings, this type does not derive `Default`: the initialization
/// paths ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) select [`Self::Row`] directly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirectionValue {
    /// `row` — the spec's initial value. The main axis follows the container's
    /// inline axis. Resolving the physical direction according to writing mode
    /// is outside this crate's scope and is delegated to taffy's mapping of
    /// the same keyword.
    Row,
    /// `row-reverse` — the main axis runs opposite to `row`.
    RowReverse,
    /// `column` — the main axis follows the container's block axis.
    Column,
    /// `column-reverse` — the main axis runs opposite to `column`.
    ColumnReverse,
}

css_keywords!(FlexDirectionValue {
    Row => "row",
    RowReverse => "row-reverse",
    Column => "column",
    ColumnReverse => "column-reverse",
});

/// The value of the `flex-wrap` property.
///
/// CSS Flexible Box Layout Module Level 1 §5.2 "Flex Line Wrapping: the
/// flex-wrap property"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>: value grammar
/// `nowrap | wrap | wrap-reverse`; "Initial: nowrap", "Inherited: no",
/// "Applies to: flex containers", and "Computed value: specified keyword".
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexWrapValue {
    /// `nowrap` — the spec's initial value; a single line.
    NoWrap,
    /// `wrap` — multiple lines, stacked from cross-start to cross-end.
    Wrap,
    /// `wrap-reverse` — multiple lines, stacked in reverse order from `wrap`.
    WrapReverse,
}

css_keywords!(FlexWrapValue {
    NoWrap => "nowrap",
    Wrap => "wrap",
    WrapReverse => "wrap-reverse",
});

/// The specified value of the `flex-basis` property.
///
/// CSS Flexible Box Layout Module Level 1 §7.2.3 "The flex-basis property"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>: value
/// grammar `content | <'width'>`; "Initial: auto", "Inherited: no",
/// "Applies to: flex items", and "Computed value: specified keyword or a
/// computed `<length-percentage>` value".
///
/// `<'width'>` is spec notation for reusing the grammar of the `width` property
/// (CSS Sizing 3 §3.1.1), `auto | <length-percentage [0,∞]>`. This has nearly
/// the same shape as this crate's [`LengthOrAuto`], but `flex-basis` also has
/// the `content` keyword ("plus the content keyword" — from the shorthand
/// discussion in spec §7.1). Therefore, it uses a dedicated three-variant enum
/// rather than reusing [`LengthOrAuto`] directly.
///
/// # Difference between `content` and `auto` (summary of spec §7.1) — preserved unresolved
///
/// - `auto`: uses the declared element's main-size property (`width`/`height`).
///   If that value is also `auto`, the used flex-basis becomes `content`
///   ("If that value is itself auto, then the used value is content.").
/// - `content`: ignores the main-size property's value and always uses
///   content-based sizing (typically equivalent to max-content).
///
/// The computed layer also preserves this distinction (spec "Computed value: specified
/// keyword … " — `content` is not collapsed into `auto`). **Actually resolving
/// this distinction is outside this crate's scope**: the downstream
/// (raikiri-dom) taffy bridge has to map both to `taffy::Dimension::AUTO`
/// (taffy 0.12's `Dimension` has no variant equivalent to `content`).
/// See the `bridge_flex` doc in `crates/raikiri-dom/src/layout.rs` for the
/// bridge's scope boundary.
///
/// `#[non_exhaustive]` — the same forward-compatibility contract as sibling
/// [`DisplayValue`] / [`LengthOrAuto`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlexBasisValue {
    /// `auto` — the spec's initial value.
    Auto,
    /// `content` — spec §7.1 "plus the content keyword".
    Content,
    /// `min-content` — a CSS Sizing 3 intrinsic keyword required by the WPT
    /// `flex-basis-valid.html`. Maps to the identically named `Dimension`
    /// variant in taffy 0.14 (see the `bridge_flex` doc).
    MinContent,
    /// `max-content` — same as above.
    MaxContent,
    /// Bare `fit-content` keyword — same as above. The `fit-content()` function
    /// with a `<length-percentage>` argument is outside scope (absent from the WPT vector).
    FitContent,
    /// `<length-percentage [0,∞]>` — the same non-negative constraint as `width`
    /// (see the [`parse_flex_basis`] doc).
    Length(Length),
}

/// The specified value of the `flex` shorthand.
///
/// CSS Flexible Box Layout Module Level 1 §7.1 "The flex Shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-property>: value grammar
/// `none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]`;
/// "Initial: 0 1 auto", "Inherited: no", "Applies to: flex items".
///
/// `none` is a separate exclusive keyword (equivalent to `0 0 auto`), represented
/// by this struct after expansion to three fields (see the "`none`" section
/// of the [`parse_flex_shorthand`] doc). It is a separate grammar branch, but
/// after structuring it need not be distinguished from other three-value forms.
///
/// # Omitted-component defaults **differ** from longhand initial values
///
/// Verbatim spec text (the Note immediately after "The flex property specifies…" in §7.1):
///
/// > The initial values of the flex longhands are equivalent to
/// > `flex: 0 1 auto`. This differs from their defaults when omitted in the
/// > flex shorthand (effectively `1 1 0px`) so that the flex shorthand can
/// > better accommodate the most common cases.
///
/// Thus, omitted components in the shorthand default to
/// **grow=1 / shrink=1 / basis=0px**, unlike the initial values (0 / 1 / auto)
/// of the `flex-grow`/`flex-shrink`/`flex-basis` longhands themselves.
/// [`parse_flex_shorthand`] applies these shorthand-local defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlexShorthand {
    /// See the [`Self`] doc — `<'flex-grow'>` component; defaults to 1.0 when omitted.
    pub grow: f32,
    /// See the [`Self`] doc — `<'flex-shrink'>` component; defaults to 1.0 when omitted.
    pub shrink: f32,
    /// See the [`Self`] doc — `<'flex-basis'>` component; defaults to `Length(Length::Px(0.0))` when omitted.
    pub basis: FlexBasisValue,
}

/// The specified value of the `flex-flow` shorthand.
///
/// CSS Flexible Box Layout Module Level 1 §5.3 "Flex Direction and Wrap: the
/// flex-flow shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-flow-property>: value grammar
/// `<'flex-direction'> || <'flex-wrap'>`;
/// "Initial: see individual properties", "Inherited: no",
/// "Applies to: flex containers", "Computed value: see individual properties".
///
/// `||` means any order, each component at most once, with at least one required.
/// An omitted component expands to its longhand's initial value
/// (direction=row / wrap=nowrap); [`parse_flex_flow`] applies this rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlexFlow {
    /// See the [`Self`] doc — `<'flex-direction'>` component; Row when omitted.
    pub direction: FlexDirectionValue,
    /// See the [`Self`] doc — `<'flex-wrap'>` component; NoWrap when omitted.
    pub wrap: FlexWrapValue,
}

/// Shared `justify-content` / `align-content` value ("content-distribution"
/// alignment).
///
/// CSS Box Alignment Module Level 3 §5.1 "The justify-content and
/// align-content Properties" propdef `justify-content`
/// <https://www.w3.org/TR/css-align-3/#propdef-justify-content> / propdef
/// `align-content` <https://www.w3.org/TR/css-align-3/#propdef-align-content>
/// (both property definitions occur in the same §5.1; the spec's choice to
/// define both properties in one section supports this crate's use of a
/// shared type): both have "Initial: normal", "Inherited: no", and
/// "Computed value: specified
/// keyword(s)". Apart from the scope limits below, their grammars have the
/// same shape (`<content-distribution>` = §4.3 `space-between | space-around |
/// space-evenly | stretch`, `<content-position>` = §4.1
/// `center | start | end | flex-start | flex-end`), so they share one
/// type. This is the reverse of the precedent set by separate
/// `AlignItemsKeyword`/`AlignContentKeyword` types, but applies the same
/// principle: share a type when the grammars are effectively identical.
///
/// # Scope carving
///
/// - **(b) unsupported**: `<overflow-position>` (`safe`/`unsafe` prefixes, §4.4)
///   is not implemented. The parser does not accept these prefixes and drops
///   the entire prefixed declaration (a two-token sequence such as `safe center`
///   naturally produces `None` because it does not match the single keyword
///   expected by `parse_content_alignment`).
/// - **(b) unsupported**: `<baseline-position>` (optional `first`/`last` plus
///   `baseline`, a grammar branch only for `align-content`) is not implemented.
///   Taffy 0.12's `AlignContent`/`JustifyContent` (both based on
///   `alignment::AlignContentKeyword`) have no `Baseline` variant, so taffy
///   cannot represent it.
/// - **(a) spec-invalid for this pair**: `justify-content`'s separate
///   `left`/`right` extension (`<content-position> | left | right`, keywords
///   relative to writing mode) is not implemented; taffy has no matching variant.
///
/// Following this crate's rule against inventing values, all three cases
/// **silently drop** the entire declaration on parse failure.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentAlignmentValue {
    /// `normal` — the spec's initial value. Box alignment's "default that
    /// depends on the context" has different meanings for `align-content` and
    /// `justify-content`: flex `align-content: normal` behaves like [`Self::Stretch`]
    /// (multiple lines fill the cross axis), whereas `justify-content: normal`
    /// behaves like `flex-start` (packed along the main axis, not a stretching axis).
    /// The taffy bridge maps both to `None`, leaving resolution to taffy's
    /// per-field defaults (see `content_alignment_to_taffy` in
    /// `crates/raikiri-dom/src/layout.rs`).
    Normal,
    /// `stretch` — CSS Box Alignment 3 §4.3 `<content-distribution>`.
    Stretch,
    /// `space-between` — `<content-distribution>`.
    SpaceBetween,
    /// `space-evenly` — `<content-distribution>`.
    SpaceEvenly,
    /// `space-around` — `<content-distribution>`.
    SpaceAround,
    /// `center` — CSS Box Alignment 3 §4.1 `<content-position>`.
    Center,
    /// `start` — `<content-position>`.
    Start,
    /// `end` — `<content-position>`.
    End,
    /// `flex-start` — `<content-position>`.
    FlexStart,
    /// `flex-end` — `<content-position>`.
    FlexEnd,
}

css_keywords!(ContentAlignmentValue {
    Normal => "normal",
    Stretch => "stretch",
    SpaceBetween => "space-between",
    SpaceEvenly => "space-evenly",
    SpaceAround => "space-around",
    Center => "center",
    Start => "start",
    End => "end",
    FlexStart => "flex-start",
    FlexEnd => "flex-end",
});

/// `align-items` value (a "self-alignment" keyword set based on CSS Box
/// Alignment 3 §4.1 `<self-position>`).
///
/// CSS Box Alignment Module Level 3 §7.2 "Block-Axis (or Cross-Axis)
/// Default Alignment: the align-items property" propdef `align-items`
/// <https://www.w3.org/TR/css-align-3/#propdef-align-items>: value grammar
/// `normal | stretch | <baseline-position> | <overflow-position>?
/// <self-position>`; "Initial: normal", "Inherited: no", "Applies to: all
/// elements", "Computed value: specified keyword(s)".
///
/// `align-self` (§6.2) reuses this enum through [`AlignSelfValue::Value`]:
/// its grammar adds `auto` to all the `align-items` keywords. This shared
/// type plus wrapper goes in the opposite direction from the decision to give
/// [`FlexBasisValue`] its own enum rather than reuse `LengthOrAuto`, but both
/// apply the rule of extracting only the shared grammar into one type.
///
/// # Scope carving
///
/// - **(b) unsupported**: `<overflow-position>` (`safe`/`unsafe` prefix) is
///   not implemented, as with [`ContentAlignmentValue`].
/// - **(b) unsupported**: `self-start`/`self-end` (part of `<self-position>`,
///   keywords relative to writing mode) are not implemented because taffy's
///   `AlignItemsKeyword` has no matching variants.
/// - **(b) unsupported**: the `first`/`last` prefixes for `<baseline-position>`
///   are not implemented: taffy's `AlignItemsKeyword::Baseline` does not
///   distinguish prefixes ("first" is the default; refinements corresponding
///   to spec §9 "Fallback Alignment" are unsupported). Only the bare
///   `baseline` keyword is accepted.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfAlignmentValue {
    /// `normal` — the spec's initial value.
    Normal,
    /// `stretch`.
    Stretch,
    /// `center` — `<self-position>`.
    Center,
    /// `start` — `<self-position>`.
    Start,
    /// `end` — `<self-position>`.
    End,
    /// `flex-start` — `<self-position>`.
    FlexStart,
    /// `flex-end` — `<self-position>`.
    FlexEnd,
    /// `baseline` — `<baseline-position>` (prefixes unsupported; see [`Self`] doc).
    Baseline,
}

/// The value of the `align-self` property.
///
/// CSS Box Alignment Module Level 3 §6.2 "Block-Axis (or Cross-Axis)
/// Self-Alignment: the align-self property" propdef `align-self`
/// <https://www.w3.org/TR/css-align-3/#propdef-align-self>: value grammar
/// `auto | <overflow-position>? [ normal | <self-position> ] | stretch |
/// <baseline-position>`; "Initial: auto", "Inherited: no", "Applies to:
/// flex items, grid items, and absolutely-positioned boxes", "Computed
/// value: specified keyword(s)".
///
/// All keywords except `auto` match the grammar of [`SelfAlignmentValue`]
/// (= `align-items`); see the sharing rationale in the [`Self`] doc.
///
/// `#[non_exhaustive]` — the same forward-compatibility contract as sibling
/// [`SelfAlignmentValue`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignSelfValue {
    /// `auto` — the spec's initial value. CSS Box Alignment 3 §6.2 `valdef-align-
    /// self-auto` (paraphrased): behaves like the parent's computed `align-items`
    /// value (excluding legacy keywords). Actual fallback resolution is outside
    /// this crate's scope and delegated to taffy (`Option<AlignSelf> = None`
    /// falls back to the parent's `align_items`).
    Auto,
    /// An explicit keyword other than `auto` — reuses [`SelfAlignmentValue`] unchanged.
    Value(SelfAlignmentValue),
}

/// The specified value of the `gap` shorthand.
///
/// CSS Box Alignment Module Level 3 §8.2 "Gap Shorthand: the gap property"
/// propdef `gap`
/// <https://www.w3.org/TR/css-align-3/#propdef-gap>: value grammar
/// `<'row-gap'> <'column-gap'>?`; "Initial: see individual properties",
/// "Inherited: no". When the second component is omitted, it copies the
/// first component's value (spec text: "If column-gap is omitted, it's set to the same value as
/// row-gap.").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GapShorthand {
    /// The `row-gap` component.
    pub row: LengthOrNormal,
    /// The `column-gap` component — equals `row` when omitted (see [`parse_gap_shorthand`]).
    pub column: LengthOrNormal,
}

/// The specified value of the `place-content` shorthand.
///
/// CSS Box Alignment Module Level 3 §5.2 "Content-Distribution Shorthand:
/// the place-content property" propdef `place-content`
/// <https://www.w3.org/TR/css-align-3/#propdef-place-content>: value
/// grammar `<'align-content'> <'justify-content'>?`; "Initial: normal",
/// "Inherited: no". When the second component is omitted, it copies the
/// first component's value. The exception to this spec rule ("unless that value is a `<baseline-position>` in which
/// case it is defaulted to `start`") is unreachable in this crate because
/// [`ContentAlignmentValue`] has no `<baseline-position>` variant (see the
/// [`ContentAlignmentValue`] doc's scope-carving section). Thus the "copy from
/// first value" branch always applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceContentShorthand {
    /// The `align-content` component.
    pub align: ContentAlignmentValue,
    /// The `justify-content` component — equals `align` when omitted
    /// (see [`parse_place_content_shorthand`] and the exception noted in the [`Self`] doc).
    pub justify: ContentAlignmentValue,
}

// ─────────────────────────────────────────────────────────────────────────
// CSS Grid Layout Module Level 1 (<https://www.w3.org/TR/css-grid-1/>) —
// grid-template-columns/-rows/-areas, grid-auto-columns/-rows/-flow,
// grid-row/-column (longhands + shorthand), and (from CSS Box Alignment
// Module Level 3) justify-items/justify-self/place-items/place-self.
// ─────────────────────────────────────────────────────────────────────────

/// `<track-breadth>` — one operand of a `<track-size>` (bare, or inside
/// `minmax()`'s second argument).
///
/// CSS Grid Layout Module Level 1 §7.2.1 "Track Sizes"
/// (<https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-track-breadth>),
/// grammar (verbatim):
///
/// `<track-breadth> = <length-percentage [0,∞]> | <flex [0,∞]> | min-content
/// | max-content | auto`
///
/// The `<flex>` unit (`fr`) is defined in §7.2.4 "Flexible Lengths: the fr
/// unit" (<https://www.w3.org/TR/css-grid-1/#fr-unit>).
///
/// `#[non_exhaustive]` — the same forward-compatibility contract as [`DisplayValue`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackBreadth {
    /// `<length-percentage [0,∞]>`.
    Length(Length),
    /// `<flex [0,∞]>` — the `fr` unit (§7.2.4). Authored non-negative number
    /// (`1fr` → `Flex(1.0)`).
    Flex(f32),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `auto` — in a track-sizing context, `auto` is "as `max-content`, but
    /// clamped to fit within the grid container" (spec §7.2.1). Resolution
    /// belongs to the used-value layer (raikiri-dom / taffy).
    Auto,
}

/// `<inflexible-breadth>` — grammar for `minmax()`'s first argument (min side).
/// This is `<track-breadth>` without `<flex>` (a grammar-level exclusion
/// corresponding to the spec: "A minmax() function
/// takes exactly two arguments... If the first argument is a `<flex>`
/// value... the declaration is invalid").
///
/// CSS Grid Layout Module Level 1 §7.2.1
/// (<https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-inflexible-breadth>):
///
/// `<inflexible-breadth> = <length-percentage [0,∞]> | min-content |
/// max-content | auto`
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridInflexibleBreadth {
    /// `<length-percentage [0,∞]>` — this variant also serves as
    /// `<fixed-breadth>` (see the "fixed-size constraint" section of the
    /// [`GridTrackSize`] doc).
    Length(Length),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `auto`.
    Auto,
}

/// `<track-size>` — sizing function for one grid track.
///
/// CSS Grid Layout Module Level 1 §7.2.1 "Track Sizes"
/// (<https://www.w3.org/TR/css-grid-1/#typedef-track-size>), grammar
/// (verbatim):
///
/// `<track-size> = <track-breadth> | minmax( <inflexible-breadth> ,
/// <track-breadth> ) | fit-content( <length-percentage [0,∞]> )`
///
/// # `<fixed-size>` — additional constraint in auto-repeat / fixed-repeat
///
/// Some forms of `repeat(auto-fill|auto-fit, …)` / `repeat(<integer>, …)`
/// (`<auto-repeat>` / `<fixed-repeat>`; see the [`GridTrackRepeat`] doc) require
/// the narrower `<fixed-size>` (§7.2.1
/// <https://www.w3.org/TR/css-grid-1/#typedef-fixed-size>), not `<track-size>`:
///
/// `<fixed-size> = <fixed-breadth> | minmax( <fixed-breadth> , <track-breadth>
/// ) | minmax( <inflexible-breadth> , <fixed-breadth> )` where `<fixed-breadth>
/// = <length-percentage [0,∞]>`
///
/// This crate does not model `<track-size>` and `<fixed-size>` as separate types
/// (both fit the same [`Self`] shape). Instead, [`grid_track_size_is_fixed`]
/// applies the `<fixed-size>` constraint during post-parse validation
/// (no `fr`, bare `min-content`/`max-content`/`auto`, or `fit-content()`),
/// called from [`parse_grid_template_tracks`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackSize {
    /// Bare `<track-breadth>`.
    Breadth(GridTrackBreadth),
    /// `minmax( <inflexible-breadth>, <track-breadth> )`.
    MinMax(GridInflexibleBreadth, GridTrackBreadth),
    /// `fit-content( <length-percentage [0,∞]> )` — spec §7.2.1: "represents
    /// the formula `max(minimum, min(limit, max-content))`". The limit is a
    /// non-negative length-percentage.
    FitContent(Length),
}

/// The first argument of `repeat()` (the repetition count).
///
/// CSS Grid Layout Module Level 1 §7.2.3.1 "Syntax of repeat()"
/// (<https://www.w3.org/TR/css-grid-1/#typedef-track-repeat>) allows only
/// `<integer [1,∞]>` for `<track-repeat>`, whereas `<auto-repeat>` (§7.2.3.2,
/// <https://www.w3.org/TR/css-grid-1/#typedef-auto-repeat>) allows only
/// `auto-fill | auto-fit`. This crate represents both in one enum;
/// [`parse_grid_repeat`] accepts only the appropriate alternative for each
/// context (see "Allowed counts" in the [`GridTrackRepeat`] docs).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridRepeatCount {
    /// `<integer [1,∞]>` — a fixed repetition count (`<track-repeat>` or
    /// `<fixed-repeat>`).
    Count(u32),
    /// `auto-fill` — repeats to fill the available space and retains empty
    /// tracks (§7.2.3.2 <https://www.w3.org/TR/css-grid-1/#auto-fill>).
    /// Computing the actual repetition count belongs to the used-value layer
    /// (raikiri-dom / taffy).
    AutoFill,
    /// `auto-fit` — like `auto-fill`, but collapses empty tracks
    /// (§7.2.3.2 <https://www.w3.org/TR/css-grid-1/#auto-fit>).
    AutoFit,
}

/// One `repeat( <count>, <tracks> )` component in a track list.
///
/// CSS Grid Layout Module Level 1 §7.2.3.1
/// (<https://www.w3.org/TR/css-grid-1/#funcdef-repeat>), grammar
/// (verbatim, three alternative forms):
///
/// ```text
/// <track-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <track-size> ]+ <line-names>? )
/// <auto-repeat>  = repeat( [ auto-fill | auto-fit ] , [ <line-names>? <fixed-size> ]+ <line-names>? )
/// <fixed-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <fixed-size> ]+ <line-names>? )
/// ```
///
/// [`Self::line_names`] follows the same interleaving convention as
/// [`GridTrackList::line_names`]: `line_names.len() == tracks.len() + 1`,
/// `line_names[i]` precedes `tracks[i]`, and `line_names[tracks.len()]` is the
/// trailing set. This matches the shape of taffy 0.12's
/// `GridTemplateRepetition.line_names`. It contains the line names for one
/// iteration of `repeat()`; merging names between iterations belongs to the
/// used-value layer. The §7.2.3.1 rule "If a repeat() function ends up
/// placing two `<line-names>` adjacent to each other, the name lists are
/// merged" is outside this crate's scope.
///
/// # Allowed counts and `<fixed-size>` constraints
///
/// - [`GridRepeatCount::Count`] (`<track-repeat>`) permits any full
///   `<track-size>` in [`Self::tracks`], including `fr` and every
///   [`GridTrackSize`] variant. This form may occur any number of times in a
///   track list, independently of the [`GridTrackList`] restriction of at
///   most one auto-repeat.
/// - [`GridRepeatCount::AutoFill`] / [`GridRepeatCount::AutoFit`]
///   (`<auto-repeat>`) requires `<fixed-size>` in [`Self::tracks`] (see the
///   [`GridTrackSize`] docs). It excludes `fr`, bare `min-content`,
///   `max-content`, `auto`, and `fit-content()`. The spec says: "It can only
///   appear once in the track list, but the same track list can also
///   contain `<fixed-repeat>`s." [`parse_grid_template_tracks`] checks the
///   entire track list for this at-most-once restriction.
/// - This crate has no separate `<fixed-repeat>` variant. It represents one
///   with [`GridRepeatCount::Count`] plus a `<fixed-size>` restriction,
///   post-validated by [`grid_track_size_is_fixed`] only when the track list
///   contains an auto-repeat. See the [`parse_grid_template_tracks`] docs.
///
/// `repeat()` cannot be nested (the spec says "The repeat() notation can't
/// be nested."). The element type of [`Self::tracks`] is [`GridTrackSize`],
/// not [`GridTrackList`], so nesting is structurally impossible.
#[derive(Clone, Debug, PartialEq)]
pub struct GridTrackRepeat {
    /// Number of repetitions.
    pub count: GridRepeatCount,
    /// Interleaved line names; see the [`Self`] docs for their shape.
    pub line_names: Vec<Vec<SmolStr>>,
    /// Track sizing functions to repeat.
    pub tracks: Vec<GridTrackSize>,
}

/// One component of a `<track-list>` or `<auto-track-list>`: a single track
/// or `repeat()`.
///
/// CSS Grid Layout Module Level 1 §7.2 "Explicit Track Sizing"
/// (<https://www.w3.org/TR/css-grid-1/#track-sizing>) defines the `<track-list>`
/// grammar as `[ <line-names>? [ <track-size> | <track-repeat> ] ]+
/// <line-names>?`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackListComponent {
    /// One track sizing function.
    Size(GridTrackSize),
    /// A `repeat()` component.
    Repeat(GridTrackRepeat),
}

/// The non-`none` specified value of `grid-template-columns` or
/// `grid-template-rows`: the entire track list, with interleaved line names
/// and components.
///
/// CSS Grid Layout Module Level 1 §7.2 gives the `<track-list>` grammar as
/// `[ <line-names>? [ <track-size> | <track-repeat> ] ]+ <line-names>?`.
///
/// [`Self::line_names`] interleaves with `Self::components`:
/// `line_names.len() == components.len() + 1`. The named line set before
/// `components[i]` is `line_names[i]`; the set after the final component is
/// `line_names[components.len()]`. This shape directly matches taffy 0.12's
/// `Style::grid_template_column_names` / `grid_template_row_names`, which
/// are consumed in lockstep with `components` by the internal
/// `NamedLineResolver`. The raikiri-dom bridge can zip them without reshaping.
///
/// This crate represents both `<track-list>` (no auto-repeat; all components
/// may use full `<track-size>`) and `<auto-track-list>` (exactly one
/// auto-repeat; all other tracks require `<fixed-size>`) as `GridTrackList`.
/// [`parse_grid_template_tracks`] checks two constraints after parsing: at
/// most one auto-repeat, and `<fixed-size>` for every other track when an
/// auto-repeat is present (see the [`GridTrackRepeat`] docs).
#[derive(Clone, Debug, PartialEq)]
pub struct GridTrackList {
    /// Interleaved line names; see the [`Self`] docs for their shape.
    pub line_names: Vec<Vec<SmolStr>>,
    /// Components of the track list.
    pub components: Vec<GridTrackListComponent>,
}

/// The specified value of `grid-template-columns` or `grid-template-rows`.
///
/// CSS Grid Layout Module Level 1 §7.2 "Explicit Track Sizing: the
/// grid-template-rows and grid-template-columns properties"
/// (<https://www.w3.org/TR/css-grid-1/#track-sizing>): value grammar `none |
/// <track-list> | <auto-track-list>`; "Initial: none"; "Inherited: no";
/// "Percentages: refer to corresponding dimension of the content area";
/// "Computed value: the keyword `none` or a computed track list".
///
/// Wrapping the list in `Arc` follows the memory-DoS mitigation described
/// in the [`PropertyValue`] docs, under "cascade memory DoS countermeasures".
/// Like the [`ContentComponent`] list's `Content(Arc<Vec<..>>)`, a track
/// list is a heap payload that may contain arbitrarily many `repeat()` calls
/// and may be cloned whenever a cascade winner is selected.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTemplateTracks {
    /// `none` — the spec initial value; no explicit tracks are defined.
    None,
    /// A `<track-list>` or `<auto-track-list>`.
    List(Arc<GridTrackList>),
}

/// The `grid` shorthand's explicit row/column track lists.
#[derive(Clone, Debug, PartialEq)]
pub struct GridShorthand {
    pub rows: GridTemplateTracks,
    pub columns: GridTemplateTracks,
}

/// The four-line `grid-area` placement shorthand.
#[derive(Clone, Debug, PartialEq)]
pub struct GridAreaShorthand {
    pub row_start: GridLineValue,
    pub column_start: GridLineValue,
    pub row_end: GridLineValue,
    pub column_end: GridLineValue,
}

/// One named `grid-template-areas` area, with 1-based grid line coordinates
/// and exclusive ends. These coordinates match taffy 0.12's
/// `GridTemplateArea` and the rectangle-to-lines conversion in CSS Grid
/// Layout Module Level 1 §9.2, "Line-based Placement: the
/// grid-template-areas shorthand".
#[derive(Clone, Debug, PartialEq)]
pub struct GridTemplateAreaEntry {
    /// Name of the area.
    pub name: SmolStr,
    /// Starting row grid line (1-based).
    pub row_start: u32,
    /// Ending row grid line (1-based, exclusive); `row_end - row_start`
    /// gives the row span.
    pub row_end: u32,
    /// Starting column grid line (1-based).
    pub column_start: u32,
    /// Ending column grid line (1-based, exclusive).
    pub column_end: u32,
}

/// The non-`none` specified value of `grid-template-areas`.
///
/// CSS Grid Layout Module Level 1 §7.3 "Named Areas: the
/// grid-template-areas property"
/// (<https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>):
/// "Computed value: the keyword `none` or a **list of string values**".
/// Unlike grid-template-columns/-rows, its computed value is the **list of
/// authored strings itself**, not the parsed area rectangles.
/// [`Self::row_strings`] meets this computed-value requirement.
/// [`Self::areas`] / [`Self::row_count`] / [`Self::column_count`] cache the
/// parsing results once, so the raikiri-dom bridge can convert them directly
/// to taffy `GridTemplateArea` without parsing them again.
#[derive(Clone, Debug, PartialEq)]
pub struct GridTemplateAreas {
    /// Authored strings (the computed value specified by the spec).
    pub row_strings: Vec<SmolStr>,
    /// Parsed named areas that passed the rectangle validation in
    /// [`parse_grid_template_areas`].
    pub areas: Vec<GridTemplateAreaEntry>,
    /// Number of rows in the string grid (= `row_strings.len()`).
    pub row_count: u32,
    /// Number of columns in the string grid, equal in every row as required
    /// by the spec (see the [`parse_grid_template_areas`] docs).
    pub column_count: u32,
}

/// The specified value of the `grid-template-areas` property.
///
/// See the [`GridTemplateAreas`] docs. Like [`GridTemplateTracks::List`],
/// the `Arc` reduces the cost of cloning a heap payload during cascade
/// winner selection.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTemplateAreasValue {
    /// `none` — the spec initial value.
    None,
    /// `<string>+` — a collection of parsed named areas.
    Areas(Arc<GridTemplateAreas>),
}

/// The value of the `grid-auto-flow` property.
///
/// CSS Grid Layout Module Level 1 §7.7 "Automatic Placement: the
/// grid-auto-flow property"
/// (<https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-flow>): value
/// grammar `[ row | column ] || dense`; "Initial: row"; "Inherited: no";
/// "Computed value: specified keyword(s)".
///
/// `#[non_exhaustive]` provides the same forward-compatibility guarantee as
/// [`DisplayValue`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridAutoFlowValue {
    /// `row` without `dense` — the spec initial value.
    Row,
    /// `column` without `dense`.
    Column,
    /// `row dense`.
    RowDense,
    /// `column dense`.
    ColumnDense,
}

/// `<grid-line>` is the shared grammar for `grid-row-start`, `grid-row-end`,
/// `grid-column-start`, and `grid-column-end`. The §8.4 `grid-row` and
/// `grid-column` shorthands reuse it through [`GridLineShorthand`].
///
/// CSS Grid Layout Module Level 1 §8.3 "Line-based Placement: the
/// grid-row-start, grid-column-start, grid-row-end, and grid-column-end
/// properties"
/// (<https://www.w3.org/TR/css-grid-1/#line-placement>): value grammar
/// (verbatim)
///
/// ```text
/// <grid-line> =
///   auto |
///   <custom-ident> |
///   [ [ <integer [-∞,-1]> | <integer [1,∞]> ] && <custom-ident>? ] |
///   [ span && [ <integer [1,∞]> || <custom-ident> ] ]
/// ```
///
/// "Initial: auto"; "Inherited: no"; "Percentages: n/a"; "Computed value:
/// specified keyword, identifier, and/or integer".
///
/// The spec states: "In all the above productions, the `<custom-ident>`
/// additionally excludes the keywords `span` and `auto`", and "If the
/// `<integer>` is omitted, it defaults to 1. Negative integers or zero are
/// invalid." [`is_reserved_grid_line_name`] and [`parse_grid_line`] enforce
/// the respective constraints.
///
/// The bare `<custom-ident>` alternative (without an index) is folded into
/// [`Self::NamedLine`] with an explicit index of `1`. Taffy 0.12 treats `0`
/// in `GridPlacement::NamedLine` as an "unspecified" sentinel and normalizes
/// it to `1` internally (`NamedLineResolver::find_line_index` checks
/// `if idx == 0 { idx = 1; }`). Supplying `1` in this crate preserves that
/// meaning.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridLineValue {
    /// `auto` — the spec initial value. It requests auto-placement or a
    /// default span of one when combined with a span.
    Auto,
    /// `<integer>` — a 1-based grid line index (negative values count from
    /// the end). `0` is invalid under the spec (see the [`parse_grid_line`] docs).
    Line(i32),
    /// Bare `<custom-ident>` — the top-level grammar alternative without an
    /// accompanying `<integer>`. Only this variant triggers the copy rule
    /// for an omitted second component of [`GridLineShorthand`] (the spec
    /// says "if the first value is a `<custom-ident>`"). [`Self::NamedLine`]
    /// does not: it is a separate `<integer> && <custom-ident>` alternative,
    /// in which an omitted `<integer>` is treated as an explicit `1`.
    /// See the [`parse_grid_line`] docs.
    Named(SmolStr),
    /// `[ [ <integer> ] && <custom-ident>? ]` — a named-line reference with
    /// a required `<integer>` and an optional `<custom-ident>`.
    NamedLine(SmolStr, i32),
    /// `span <integer>` — an explicit span.
    Span(u32),
    /// `span <custom-ident>` — a span up to a named line. An `<integer>`
    /// may accompany it and defaults to `1` when omitted.
    SpanNamed(SmolStr, u32),
}

/// The specified value of the `grid-row` or `grid-column` shorthand.
///
/// CSS Grid Layout Module Level 1 §8.4 "Placement Shorthands: the
/// grid-column, grid-row, and grid-area properties"
/// (<https://www.w3.org/TR/css-grid-1/#placement-shorthands>): value grammar
/// `<grid-line> [ / <grid-line> ]?`; "Initial: auto"; "Inherited: no".
///
/// The spec states: "If two `<grid-line>` values are specified, the
/// grid-row-start / grid-column-start longhand is set to the value before
/// the slash, and the grid-row-end / grid-column-end longhand is set to the
/// value after the slash. When the second value is omitted, if the first
/// value is a `<custom-ident>`, the grid-row-end / grid-column-end longhand
/// is also set to that `<custom-ident>`; otherwise, it is set to `auto`."
/// [`parse_grid_line_shorthand`] applies this rule.
#[derive(Clone, Debug, PartialEq)]
pub struct GridLineShorthand {
    /// The `-start` longhand component.
    pub start: GridLineValue,
    /// The `-end` longhand component; see the [`Self`] docs for its
    /// omission rule.
    pub end: GridLineValue,
}

/// The specified value of the `place-items` shorthand.
///
/// CSS Box Alignment Module Level 3 §7.3 "Default Alignment Shorthand: the
/// place-items property"
/// (<https://www.w3.org/TR/css-align-3/#propdef-place-items>): value grammar
/// `<'align-items'> <'justify-items'>?`; "Initial: see individual
/// properties"; "Inherited: no". When the second component is omitted,
/// it copies the first component unchanged (see the analogous note in the
/// [`PlaceContentShorthand`] docs). This crate's [`SelfAlignmentValue`] has
/// no additional `justify-items` scope carve-out for `legacy` (see the
/// [`PropertyValue::JustifyItems`] docs), so the exceptional branch of the
/// copy rule cannot be reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceItemsShorthand {
    /// The `align-items` component.
    pub align: SelfAlignmentValue,
    /// The `justify-items` component; it equals `align` when omitted
    /// (see [`parse_place_items_shorthand`]).
    pub justify: SelfAlignmentValue,
}

/// The specified value of the `place-self` shorthand.
///
/// CSS Box Alignment Module Level 3 §6.3 "Self-Alignment Shorthand: the
/// place-self property"
/// (<https://www.w3.org/TR/css-align-3/#propdef-place-self>): value grammar
/// `<'align-self'> <'justify-self'>?`; "Initial: `auto`"; "Inherited: no".
/// Omitting the second component follows the same copy rule as
/// [`PlaceItemsShorthand`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceSelfShorthand {
    /// The `align-self` component.
    pub align: AlignSelfValue,
    /// The `justify-self` component; it equals `align` when omitted
    /// (see [`parse_place_self_shorthand`]).
    pub justify: AlignSelfValue,
}

/// The shared `Arc` for the `grid-auto-columns` / `grid-auto-rows` spec
/// initial value (`auto`, the one-element list
/// `[GridTrackSize::Breadth(GridTrackBreadth::Auto)]`). Like
/// [`empty_content_list`], it avoids a per-node allocation by sharing one
/// heap slot throughout the process.
pub(crate) fn initial_grid_auto_track_list() -> Arc<Vec<GridTrackSize>> {
    static INITIAL: OnceLock<Arc<Vec<GridTrackSize>>> = OnceLock::new();
    INITIAL
        .get_or_init(|| Arc::new(vec![GridTrackSize::Breadth(GridTrackBreadth::Auto)]))
        .clone()
}

/// The value of the `box-sizing` property.
///
/// CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
/// <https://www.w3.org/TR/css-sizing-3/#box-sizing>: value grammar
/// `content-box | border-box`; initial value `content-box`; **not inherited**;
/// computed value equals the specified keyword.
///
/// The §3.3 note says: "The definition of the box-sizing property in this module
/// supersedes the one in [CSS-UI-3]". CSS Sizing 3 therefore supersedes the
/// CSS-UI-3 definition and is the authoritative source.
///
/// # Semantics (summary of the spec)
///
/// - [`ContentBox`](Self::ContentBox) — the spec initial value. The specified
///   `width` / `height` applies to the content box; padding and border add
///   outside it (the legacy CSS 2.1 box model).
/// - [`BorderBox`](Self::BorderBox) — the specified `width` / `height` applies
///   to the border box; padding and border shrink the content box within the
///   specified size ("The specified padding and border of the element are
///   laid out and drawn inside this specified width and height").
///
/// # Scope carving
///
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped. The [`PropertyValue`] docs under "CSS-wide keywords"
///   list all five and explain why.
/// - **(a) Invalid under the spec**: unknown keywords such as `padding-box`
///   (in a CSS UI 3 draft but removed from CSS Sizing 3) and `margin-box`
///   are silently dropped (`None`).
///
/// # Downstream handoff (future work confined to style scope)
///
/// [`ComputedValues.box_sizing`] stores only the cascade's static seed. A
/// future cross-scope task must implement the `apply_computed_to_style` bridge
/// in the DOM to translate it to `taffy::Style::box_sizing`.
///
/// `#[non_exhaustive]` provides the same forward-compatibility guarantee as
/// its siblings [`DisplayValue`], [`TextAlign`], and [`PositionValue`].
///
/// [`ComputedValues.box_sizing`]: crate::computed::ComputedValues::box_sizing
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxSizing {
    /// `content-box` — the spec initial value. `width` / `height` applies
    /// to the content box; padding and border add outside it.
    ContentBox,
    /// `border-box` — `width` / `height` applies to the border box; padding
    /// and border shrink the content box within the specified size.
    BorderBox,
}

css_keywords!(BoxSizing {
    ContentBox => "content-box",
    BorderBox => "border-box",
});

/// The inherited `hanging-punctuation` keyword set (CSS Text 3 §8.2.1).
///
/// Grammar: `none | [ first || [ force-end | allow-end ] || last ]`.
/// Each variant represents a valid set; the two end modes cannot coexist.
/// CSS serialization uses the canonical first/end/last order.
/// <https://drafts.csswg.org/css-text-3/#hanging-punctuation-property>.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HangingPunctuation {
    /// No punctuation hangs. This is the initial value.
    None,
    /// Hang an opening mark, quote, or U+3000 IDEOGRAPHIC SPACE on the
    /// first formatted line.
    First,
    /// Hang a closing mark or quote on the last formatted line.
    Last,
    /// Always hang a qualifying stop or comma at the line end.
    ForceEnd,
    /// Hang a qualifying stop or comma only when needed to fit the line.
    AllowEnd,
    /// `first last`.
    FirstLast,
    /// `first force-end`.
    FirstForceEnd,
    /// `first allow-end`.
    FirstAllowEnd,
    /// `force-end last`.
    ForceEndLast,
    /// `allow-end last`.
    AllowEndLast,
    /// `first force-end last`.
    FirstForceEndLast,
    /// `first allow-end last`.
    FirstAllowEndLast,
}

css_keywords!(@serialize HangingPunctuation {
    None => "none",
    First => "first",
    Last => "last",
    ForceEnd => "force-end",
    AllowEnd => "allow-end",
    FirstLast => "first last",
    FirstForceEnd => "first force-end",
    FirstAllowEnd => "first allow-end",
    ForceEndLast => "force-end last",
    AllowEndLast => "allow-end last",
    FirstForceEndLast => "first force-end last",
    FirstAllowEndLast => "first allow-end last",
});

impl HangingPunctuation {
    /// Whether the first-line opening-punctuation rule is selected.
    pub const fn first(self) -> bool {
        matches!(
            self,
            Self::First
                | Self::FirstLast
                | Self::FirstForceEnd
                | Self::FirstAllowEnd
                | Self::FirstForceEndLast
                | Self::FirstAllowEndLast
        )
    }

    /// Whether the last-line closing-punctuation rule is selected.
    pub const fn last(self) -> bool {
        matches!(
            self,
            Self::Last
                | Self::FirstLast
                | Self::ForceEndLast
                | Self::AllowEndLast
                | Self::FirstForceEndLast
                | Self::FirstAllowEndLast
        )
    }

    /// Whether stops and commas must hang at the line end.
    pub const fn force_end(self) -> bool {
        matches!(
            self,
            Self::ForceEnd | Self::FirstForceEnd | Self::ForceEndLast | Self::FirstForceEndLast
        )
    }

    /// Whether stops and commas may hang to fit the line.
    pub const fn allow_end(self) -> bool {
        matches!(
            self,
            Self::AllowEnd | Self::FirstAllowEnd | Self::AllowEndLast | Self::FirstAllowEndLast
        )
    }
}

/// The value of the `text-align` property.
///
/// CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
/// <https://www.w3.org/TR/css-text-3/#text-align-property>. **The spec defines
/// it as a shorthand** setting the two longhands `text-align-all` and
/// `text-align-last` (`Initial: start` / `Inherited: yes`).
///
/// Value grammar: `start | end | left | right | center | justify | match-parent | justify-all`.
///
/// # Scope carving
///
/// - **(b) Unsupported**: This crate does not expand the shorthand. It keeps
///   it in one `ComputedValues.text_align` field, following the "shorthand as
///   a single field" convention used for margin (`Sides<T>`) and `content`
///   (`normal`/`none` → empty list). Splitting out the text-align-all and
///   text-align-last longhands (§6.2 / §6.3) is future work.
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped. The [`PropertyValue`] docs under "CSS-wide keywords"
///   list all five and explain why.
/// - **Implemented**: resolution of `match-parent` at computed-value time.
///   The spec (§6.1 `#valdef-text-align-match-parent`) says: "This value behaves
///   the same as inherit (computes to its parent's computed value) except that
///   an inherited value of start or end is interpreted against the parent's
///   direction value and results in a computed value of either left or right.
///   Computes to start when specified on the root element."
///   [`MatchParent`](Self::MatchParent) passes through
///   [`SpecifiedValues::text_align`](crate::specified::SpecifiedValues::text_align)
///   as the specified cascade winner but is resolved **before reaching the
///   computed layer**. Both the element paths
///   [`crate::specified::SpecifiedValues::finalize`] and
///   [`crate::specified::SpecifiedValues::finalize_as_root`], and the page
///   path [`crate::cascade::resolve_against_inherited`], funnel through
///   [`resolve_text_align_match_parent`]. Resolution needs the parent's
///   computed `direction` ([`Direction`], CSS Writing Modes 4 §2.1).
///   [`crate::computed::ComputedValues::text_align`] therefore always holds
///   a resolved value. `MatchParent` cannot be observed as a computed value;
///   the `resolve_text_align_match_parent` debug assertion checks this
///   invariant.
/// - **(a) Invalid under the spec**: the CSS Text 3 §6.1 grammar includes
///   only the eight keywords above. Other identifiers, such as `middle` and
///   `baseline`, are silently dropped (`None`). `<string>` character
///   alignment from a CSS Text 4 draft is likewise not defined by the CSS
///   Text 3 spec cited here.
///
/// Like [`DisplayValue`], this enum does not derive `Default`: its
/// `.default()` is never called. Instead,
/// [`crate::computed::ComputedValues::initial`] explicitly sets
/// [`TextAlign::Start`]. Likewise, [`DisplayValue`] and [`PositionValue`] do
/// not derive `Default` because the spec gives no "omitted → default" rule;
/// [`CounterStyle`], [`StringFetchMode`], [`ContentPart`], and
/// [`ContentTextKeyword`] do derive it because their specs do.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    /// `start` — "Inline-level content is aligned to the start edge of the line
    /// box" (§6.1, verbatim). This is the spec initial value. Writing mode and
    /// direction determine the physical edge (physical left for horizontal-tb
    /// and LTR).
    Start,
    /// `end` — "Inline-level content is aligned to the end edge of the line
    /// box" (§6.1, verbatim). It is the opposite of `start`.
    End,
    /// `left` — "Inline-level content is aligned to the line-left edge of the
    /// line box" (§6.1, verbatim). This is **line-left, not physical left**.
    /// In vertical writing modes, it may map to physical top or bottom
    /// depending on the writing mode (the spec says: "In vertical writing
    /// modes, this can be either the physical top or bottom, depending on
    /// writing-mode").
    Left,
    /// `right` — "Inline-level content is aligned to the line-right edge of the
    /// line box" (§6.1, verbatim). Like [`Left`](Self::Left), it may map to
    /// physical top or bottom in vertical writing modes.
    Right,
    /// `center` — "Inline-level content is centered within the line box"
    /// (§6.1, verbatim).
    Center,
    /// `justify` — "Text is justified according to the method specified by the
    /// text-justify property, in order to exactly fill the line box" (§6.1,
    /// verbatim). Without a text-align-last setting, the last line (before a
    /// forced line break) aligns to the start edge.
    Justify,
    /// `match-parent` — matches the parent's computed text alignment. It
    /// resolves the parent's `start`/`end` against the parent's direction to
    /// `left`/`right`, then inherits the resolved value. On the root element,
    /// it falls back to `start` (§6.1, verbatim).
    MatchParent,
    /// CSS-wide `inherit` for the inherited `text-align` property. It resolves
    /// directly to the parent computed value before layout.
    Inherit,
    /// `-internal-center` in the HTML UA stylesheet. It resolves to `center`
    /// only when the parent's computed alignment is the initial `start`;
    /// otherwise, it inherits the parent's value. This is not part of the
    /// author-facing CSS Text grammar.
    InternalCenter,
    /// `justify-all` — sets both text-align-all and text-align-last to
    /// `justify`, forcing the last line to justify too (§6.1, verbatim).
    JustifyAll,
}

css_keywords!(TextAlign {
    Start => "start",
    End => "end",
    Left => "left",
    Right => "right",
    Center => "center",
    Justify => "justify",
    MatchParent => "match-parent",
    Inherit => "inherit",
    InternalCenter => "-internal-center",
    JustifyAll => "justify-all",
});

/// The value of the `direction` property.
///
/// CSS Writing Modes 4 §2.1 "Specifying Directionality: the direction property"
/// <https://www.w3.org/TR/css-writing-modes-4/#direction>.
///
/// Property definition (verbatim): Value: `ltr | rtl`; Initial: `ltr`;
/// Applies to: all elements; Inherited: **yes**; Computed value: specified
/// value (the keyword is retained without resolving it against other
/// properties).
///
/// # Why this property is needed
///
/// Previously, this crate had no `direction` field in `ComputedValues`.
/// Resolving `text-align: match-parent` under CSS Text 3 §6.1,
/// `#valdef-text-align-match-parent`, requires it: "an inherited value of
/// start or end is interpreted against the parent's direction value".
/// It also matters beyond [`TextAlign::MatchParent`]. CSS Paged Media 3
/// Appendix A, "CSS 2.1 Properties that apply within the page context",
/// <https://www.w3.org/TR/css-page-3/#page-property-list>, lists `direction`
/// itself first (verified against the spec). Even `@page { direction: rtl }`
/// alone therefore has a meaningful computed value in the page context.
///
/// # Scope carving
///
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped. The [`PropertyValue`] docs under "CSS-wide keywords"
///   list all five and explain why, as for the sibling [`TextAlign`].
/// - **(a) Invalid under the spec**: identifiers other than `ltr` and `rtl`
///   are silently dropped (`None`). The current §2.1 grammar has only these
///   two keywords, not `auto` from an older draft.
/// - **Non-goal**: mapping the HTML `dir` attribute to a UA-level `direction`
///   (the presentational hint behind the spec's recommendation "we recommend
///   HTML authors to use the HTML dir attribute") is outside this crate's
///   parsing and cascade scope. Bringing in UA CSS defaults is a separate
///   task outside the independent implementation.
///
/// Like [`DisplayValue`] and [`TextAlign`], this enum does not derive
/// `Default`. [`crate::computed::ComputedValues::initial`] explicitly sets
/// [`Direction::Ltr`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `ltr` — left-to-right; the spec initial value.
    Ltr,
    /// `rtl` — right-to-left.
    Rtl,
}

css_keywords!(Direction {
    Ltr => "ltr",
    Rtl => "rtl",
});

/// The value of the `writing-mode` property.
///
/// CSS Writing Modes 4 §3.2 "Block Flow Direction: the writing-mode property"
/// <https://www.w3.org/TR/css-writing-modes-4/#propdef-writing-mode>.
///
/// Property definition (verbatim): Value: `horizontal-tb | vertical-rl | vertical-lr
/// | sideways-rl | sideways-lr`; Initial: `horizontal-tb`; Applies to: "All
/// elements except table row groups, table column groups, table rows, table
/// columns, ruby base container, ruby annotation container"; Inherited:
/// **yes**; Percentages: n/a; Computed value: specified value; Animation
/// type: not animatable.
///
/// CSS Writing Modes **Level 3** <https://www.w3.org/TR/css-writing-modes-3/#propdef-writing-mode>
/// defines only three of these keywords (`horizontal-tb | vertical-rl |
/// vertical-lr`). Its changelog says: "Deferred the sideways-lr and
/// sideways-rl values of writing-mode to Level 4." `sideways-rl` and
/// `sideways-lr` occur only in the Level 4 property definition cited here.
/// This crate also uses Level 4 for the sibling `direction` property
/// (see the [`Direction`] docs).
///
/// # `@page` context — intentionally supported beyond Appendix A
///
/// Unlike `direction` (see the [`Direction`] docs), `writing-mode` itself
/// does **not** appear in the CSS 2.1-derived table in CSS Paged Media 3
/// Appendix A's page-property-list
/// <https://www.w3.org/TR/css-page-3/#page-property-list> (verified against
/// the raw table). Raikiri treats Appendix A as a floor, not a ceiling.
/// Alongside `overflow`/`overflow-x`/`overflow-y`, [`DisplayValue`],
/// [`PositionValue`], `box-sizing`, `counter-reset`, `counter-increment`,
/// `content`, and `string-set`, it intentionally supports `writing-mode` in
/// `@page` even though Appendix A does not list it. The
/// [`crate::page::PageCascadeResult::declarations`] docs contain the canonical
/// list and explain the "positive minimum, not a ceiling" interpretation of
/// CSS Paged Media 3 §6.
///
/// Definitions of the five keywords (spec verbatim, §3.2):
///
/// - [`HorizontalTb`](Self::HorizontalTb) — "Top-to-bottom block flow
///   direction. Both the writing mode and the typographic mode are
///   horizontal." This is the spec initial value.
/// - [`VerticalRl`](Self::VerticalRl) — "Right-to-left block flow direction.
///   Both the writing mode and the typographic mode are vertical."
/// - [`VerticalLr`](Self::VerticalLr) — "Left-to-right block flow direction.
///   Both the writing mode and the typographic mode are vertical."
/// - [`SidewaysRl`](Self::SidewaysRl) — "Right-to-left block flow direction.
///   The writing mode is vertical, while the typographic mode is
///   horizontal."
/// - [`SidewaysLr`](Self::SidewaysLr) — "Left-to-right block flow direction.
///   The writing mode is vertical, while the typographic mode is
///   horizontal."
///
/// # Scope carving
///
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped. The [`PropertyValue`] docs under "CSS-wide keywords"
///   list all five and explain why.
/// - **(a) Invalid under the spec**: identifiers other than the five listed
///   keywords are silently dropped (`None`).
/// - **Non-goal — rendering vertical writing modes is not implemented**:
///   `vertical-rl`, `vertical-lr`, `sideways-rl`, and `sideways-lr` are
///   **accepted syntactically** as the spec grammar requires; they do not
///   return `None`. This resembles CSS 2.1's established "accepted without
///   a rendered effect" pattern and the limited handling of the deprecated
///   `break-word` described in the sibling [`WordBreak`] docs. The specified
///   CSS keyword remains in
///   [`crate::computed::ComputedValues::cssom_writing_mode`]. Because vertical
///   rendering is unsupported, [`resolve_writing_mode`] normalizes the
///   renderer-facing [`crate::computed::ComputedValues::writing_mode`] to
///   [`HorizontalTb`](Self::HorizontalTb). This fallback separates the CSSOM
///   computed keyword from unsupported layout behavior; it does not diverge
///   from the CSSOM computed-value spec. Revisit the renderer-facing fallback
///   when vertical writing-mode rendering lands, while retaining the
///   spec-defined CSSOM computed keyword.
///
/// Like [`DisplayValue`], [`TextAlign`], and [`Direction`], this enum does not
/// derive `Default`. [`crate::computed::ComputedValues::initial`] explicitly
/// sets [`WritingMode::HorizontalTb`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritingMode {
    /// `horizontal-tb` — the spec initial value.
    HorizontalTb,
    /// `vertical-rl` — accepted CSS computed keyword. The renderer-facing
    /// [`crate::computed::ComputedValues::writing_mode`] fallback normalizes to
    /// [`HorizontalTb`](Self::HorizontalTb); CSSOM preserves this variant in
    /// [`crate::computed::ComputedValues::cssom_writing_mode`].
    VerticalRl,
    /// `vertical-lr` — the same non-goal handling as [`VerticalRl`](Self::VerticalRl).
    VerticalLr,
    /// `sideways-rl` — the same non-goal handling as [`VerticalRl`](Self::VerticalRl).
    SidewaysRl,
    /// `sideways-lr` — the same non-goal handling as [`VerticalRl`](Self::VerticalRl).
    SidewaysLr,
}

css_keywords!(WritingMode {
    HorizontalTb => "horizontal-tb",
    VerticalRl => "vertical-rl",
    VerticalLr => "vertical-lr",
    SidewaysRl => "sideways-rl",
    SidewaysLr => "sideways-lr",
});

/// The shared value type of `overflow-x` and `overflow-y`.
///
/// CSS Overflow Module Level 3 §3.1 "Overflow: the overflow-x, overflow-y,
/// overflow-block, overflow-inline, and overflow properties"
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>.
///
/// Property definition (verbatim): Value: `visible |
/// hidden | clip | scroll | auto`; Initial: `visible`; Inherited: **no**;
/// Computed value: "usually specified value, but see text". This crate
/// implements the cross-axis coupling described by that text in
/// [`resolve_overflow`] (see its docs for details).
///
/// # Meanings of the five keywords (verified verbatim against the spec)
///
/// - [`Visible`](Self::Visible) — "There is no special handling of overflow,
///   that is, the box's content is rendered outside the box if positioned
///   there." This is the spec initial value.
/// - [`Hidden`](Self::Hidden) — "The box's content is clipped to its padding
///   box and the UA must not provide any scrolling user interface to view
///   content outside the clipping region."
/// - [`Clip`](Self::Clip) — "The box's content is clipped to its overflow
///   clip edge and no scrolling user interface should be provided. Unlike
///   hidden, overflow: clip forbids scrolling entirely."
/// - [`Scroll`](Self::Scroll) — "The content is clipped to the padding box,
///   but can be scrolled into view and the box is a scroll container."
/// - [`Auto`](Self::Auto) — "Like scroll when the box has scrollable
///   overflow; like hidden otherwise."
///
/// # Scope carving
///
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped. The [`PropertyValue`] docs under "CSS-wide keywords"
///   list all five and explain why.
/// - **(a) Invalid under the spec**: unknown keywords are silently dropped
///   (`None`).
/// - **Non-goal**: the `overflow-block` and `overflow-inline` logical
///   longhands are also defined in the §3.1 property definition. This crate
///   maps only to the physical x/y axes while writing-mode support is
///   incomplete, following the same scope decision described for
///   `margin-block`/`margin-inline` by the `hr` rule comment in
///   `crates/raikiri-html/src/ua/minimal.css`.
///
/// Like siblings [`BoxSizing`] and [`Direction`], this enum does not derive
/// `Default`. Its initialization sites
/// ([`crate::specified::SpecifiedValues::initial`] and
/// [`crate::computed::ComputedValues::initial`]) explicitly set
/// [`OverflowValue::Visible`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowValue {
    /// `visible` — the spec initial value.
    Visible,
    /// `hidden`.
    Hidden,
    /// `clip`.
    Clip,
    /// `scroll`.
    Scroll,
    /// `auto`.
    ///
    /// The legacy `overlay` spelling is an alias for `auto` and therefore has no
    /// separate computed-value variant.
    Auto,
}

css_keywords!(OverflowValue {
    Visible => "visible",
    Hidden => "hidden",
    Clip => "clip",
    Scroll => "scroll",
    Auto => "auto",
});

/// A pair holding `overflow-x` and `overflow-y`.
///
/// This is the two-axis sibling of the four-side box-model holder
/// [`Sides<T>`]. Both expansion of the `overflow` shorthand's one or two
/// values (CSS Overflow 3 §3.1, `<'overflow-block'>{1,2}`, mapped directly
/// to physical axes as explained in the [`OverflowValue`] non-goal section)
/// and computed-value coupling across axes ([`resolve_overflow`]) read and
/// write x and y together. They therefore belong in one struct rather than
/// two independent scalar fields on [`SpecifiedValues`] / [`ComputedValues`].
/// This mirrors bundling `border-*-style` and `border-*-width` for joint use
/// in [`ComputedValues::border`] as `Sides<Border>` (see the
/// [`crate::resolve::resolve_border`] docs).
///
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`ComputedValues::border`]: crate::computed::ComputedValues::border
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverflowXY {
    /// The `overflow-x` axis.
    pub x: OverflowValue,
    /// The `overflow-y` axis.
    pub y: OverflowValue,
}

impl OverflowXY {
    /// Constructs x and y with the same value: the two-axis counterpart of
    /// [`Sides::all`]. Used for both one-value `overflow: <value>` shorthand
    /// expansion (CSS Overflow 3 §3.1: "If the second value is omitted, it is
    /// copied from the first.") and the spec initial value (`visible` on both
    /// axes).
    pub const fn both(v: OverflowValue) -> Self {
        Self { x: v, y: v }
    }
}

/// Resolves cross-axis computed-value coupling between `overflow-x` and
/// `overflow-y` (CSS Overflow 3 §3.1, verbatim):
///
/// > The visible/clip values of overflow compute to auto/hidden
/// > (respectively) if one of overflow-x or overflow-y is neither visible nor
/// > clip.
///
/// # Per-axis reading vs the spec's whole-pair phrasing
///
/// The quoted rule is phrased over the whole pair ("if **one of**
/// overflow-x or overflow-y is neither visible nor clip"), but this function
/// implements it as two independent per-axis checks (`axis` below: rewrite
/// `this` iff `other` is neither visible nor clip). The two readings agree:
/// the whole-pair condition is "the axis being tested is visible/clip, AND
/// the *other* axis is neither" (if the axis under test is itself neither
/// visible nor clip, `axis` has nothing to rewrite regardless of what the
/// condition evaluates to) — which is exactly the per-axis check applied
/// independently to each of the two axes.
///
/// The two properties on one node (`overflow-x` and `overflow-y`) determine
/// each other's computed values: a **same-node cross-field dependency**.
/// Unlike [`resolve_text_align_match_parent`], which depends on a parent's
/// computed value, this depends only on the *other axis* of the same node.
/// It resembles the style→width gating in
/// [`crate::resolve::resolve_border`], where `border-*-style` determines
/// `border-*-width`'s computed value. Both are resolved in **phase 3**
/// (absolutization). The element path calls
/// [`crate::specified::SpecifiedValues::finalize`] (inside `absolutize_with`).
/// The page path collects both axes' winners in `page_context_overflow_pair`
/// before passing them here during phase 3 of [`crate::page::cascade_page`].
///
/// This function is `pub(crate)` for its callers in `specified` and `page`.
pub(crate) fn resolve_overflow(specified: OverflowXY) -> OverflowXY {
    /// Resolves one axis. If `other` is neither visible nor clip (that is,
    /// it is hidden, scroll, or auto), rewrite `this` from `visible` to `auto`
    /// or from `clip` to `hidden`. The `matches!` allowlist of
    /// `Visible | Clip` makes this safe if [`OverflowValue`] gains an unknown
    /// variant through `#[non_exhaustive]`: it falls on the "neither visible
    /// nor clip" side and triggers the fallback. This mirrors the fail-safe
    /// choice of treating an unknown `BorderStyle` variant as visible in
    /// `resolve_border`.
    fn axis(this: OverflowValue, other: OverflowValue) -> OverflowValue {
        if matches!(other, OverflowValue::Visible | OverflowValue::Clip) {
            return this;
        }
        match this {
            OverflowValue::Visible => OverflowValue::Auto,
            OverflowValue::Clip => OverflowValue::Hidden,
            same => same,
        }
    }
    OverflowXY {
        x: axis(specified.x, specified.y),
        y: axis(specified.y, specified.x),
    }
}

/// `ruby-position` controls whether ruby annotations are placed above or below
/// their base. It is inherited and defaults to `over`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RubyPosition {
    /// Place annotations above the base.
    Over,
    /// Place annotations below the base.
    Under,
    /// Place annotations between vertical glyphs.
    InterCharacter,
}

css_keywords!(RubyPosition {
    Over => "over",
    Under => "under",
    InterCharacter => "inter-character",
});

/// `WritingMode`'s renderer-facing fallback normalization ([`WritingMode`] doc's
/// Non-goal section). This is not the CSS computed keyword: element
/// [`crate::computed::ComputedValues::cssom_writing_mode`] preserves the
/// specified value for CSSOM.
///
/// Since vertical writing-mode rendering is not implemented, the four
/// non-horizontal keywords are mapped to [`WritingMode::HorizontalTb`] for
/// renderer-facing [`crate::computed::ComputedValues::writing_mode`] and page
/// layout. The normalization depends only on this property's keyword; it does
/// not inspect the parent or other properties.
///
/// # Future work — vertical writing-mode implementation
///
/// When a vertical renderer is added, reevaluate this renderer-facing fallback
/// and the existing layout tests. Keep the separate CSSOM computed keyword in
/// sync with the spec's "Computed value: specified value" rule.
///
/// # Callers
///
/// - Element path: [`crate::specified::SpecifiedValues::absolutize_with`].
///   Both `finalize` and `finalize_as_root` funnel through it. The spec does
///   not give this property a root-specific rule, unlike the "Computes to
///   start when specified on the root element" branch of
///   [`resolve_text_align_match_parent`].
/// - Page path: `absolutize_in_page_context` in [`crate::page`]. The page
///   context receives the same unconditional normalization.
///
/// This function is `pub(crate)` for its callers in `specified` and `page`.
pub(crate) fn resolve_writing_mode(specified: WritingMode) -> WritingMode {
    match specified {
        WritingMode::HorizontalTb
        | WritingMode::VerticalRl
        | WritingMode::VerticalLr
        | WritingMode::SidewaysRl
        | WritingMode::SidewaysLr => WritingMode::HorizontalTb,
    }
}

/// Resolves `text-align: match-parent` (CSS Text 3 §6.1,
/// `#valdef-text-align-match-parent`,
/// <https://www.w3.org/TR/css-text-3/#valdef-text-align-match-parent>, verbatim):
///
/// > This value behaves the same as inherit (computes to its parent's computed
/// > value) except that an inherited value of start or end is interpreted
/// > against the parent's direction value and results in a computed value of
/// > either left or right. Computes to start when specified on the root
/// > element.
///
/// This function handles only the first case, **with an actual parent**.
/// Unless `specified` is [`TextAlign::MatchParent`], it does nothing: the
/// other keywords need no resolution and their computed value equals their
/// specified value. `parent_text_align` and `parent_direction` are computed
/// values of the actual parent element (or, for `@page`, its inheritance
/// parent `root_style`).
///
/// The **second case ("Computes to start when specified on the root
/// element") is outside this function**. The caller
/// [`crate::specified::SpecifiedValues::finalize_as_root`] handles the
/// parentless root separately. A page context is **not** a parentless case:
/// "The page context inherits from the root element" does not make it the
/// root element. The page path always uses this function's with-parent branch
/// (see the caution in the [`crate::page::PageCascadeResult::declarations`] docs).
///
/// # Callers (the same contract as resolve_relative_weight)
///
/// - Element path: [`crate::specified::SpecifiedValues::finalize`] and
///   [`crate::specified::SpecifiedValues::finalize_as_root`].
/// - Page path: [`crate::cascade::resolve_against_inherited`].
///
/// Both paths funnel through this function, so the table mapping `start`/`end`
/// to `left`/`right` using the parent's direction is implemented only once.
/// This follows the precedent of collecting the CSS Fonts 4 bolder/lighter
/// table in one `resolve_relative_weight` function.
///
/// **Why the element path does not call from `apply_value`**: that function
/// writes other winners on the same node (including `direction`) into
/// [`crate::specified::SpecifiedValues`] in [`PropertyKey`] declaration order.
/// Resolution needs the **parent's** direction and must not depend on the
/// order in which the current node's `direction` winner is applied; order
/// independence is an invariant of [`crate::cascade::walk_from`].
/// After all winners are applied, `finalize` / `finalize_as_root` receive the
/// parent's [`crate::computed::ComputedValues`] explicitly, avoiding that trap.
///
/// This function is `pub(crate)` for its callers in `specified` and `cascade`.
pub(crate) fn resolve_text_align_match_parent(
    specified: TextAlign,
    parent_text_align: TextAlign,
    parent_direction: Direction,
) -> TextAlign {
    match specified {
        TextAlign::MatchParent => {
            // cov:ignore: defensive invariant guard — the message literal is
            // only formatted if a caller passes an unresolved parent value,
            // which doesn't happen from either call site (specified.rs /
            // cascade.rs both pass an already-resolved parent).
            debug_assert_ne!(
                parent_text_align,
                TextAlign::MatchParent,
                "invariant violated: 親の computed text-align が MatchParent のまま — 親側の解決が漏れている"
            );
            match parent_text_align {
                TextAlign::Start => match parent_direction {
                    Direction::Ltr => TextAlign::Left,
                    Direction::Rtl => TextAlign::Right,
                },
                TextAlign::End => match parent_direction {
                    Direction::Ltr => TextAlign::Right,
                    Direction::Rtl => TextAlign::Left,
                },
                // `left` / `right` / `center` / `justify` / `justify-all`: inherit
                // the parent’s resolved value unchanged ("behaves the same as
                // inherit" in the spec). For safety, an unresolved `MatchParent`
                // also reaches this branch: the debug assertion is absent in
                // release builds, so pass it through rather than panic.
                other => other,
            }
        }
        // Other values compute "as specified" and need no resolution.
        other => other,
    }
}

/// Resolves `text-align: -internal-center` from the HTML UA stylesheet and
/// CSS-wide `inherit` against the parent's computed value. Like Blink/WebKit's
/// table-header default, internal center chooses `center` only if the parent
/// has the initial `start` alignment. If an author sets another alignment on
/// the table, it inherits that value instead. `inherit` always inherits the
/// parent's value. The regular CSS parser does not expose the internal value
/// through its author-facing grammar, but the UA stylesheet uses the same
/// declaration pipeline.
pub(crate) fn resolve_text_align_internal_center(
    specified: TextAlign,
    parent_text_align: TextAlign,
) -> TextAlign {
    match specified {
        TextAlign::InternalCenter => {
            if parent_text_align == TextAlign::Start {
                TextAlign::Center
            } else {
                parent_text_align
            }
        }
        TextAlign::Inherit => parent_text_align,
        other => other,
    }
}

/// The value of the `position` property. Static-side support accepts
/// `static` (the default), GCPM `running(<custom-ident>)`, and CSS Positioned
/// Layout `sticky`.
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// removes an element from normal flow and registers it as a template that
/// `element()` can place in a page margin box.
///
/// CSS Positioned Layout Module Level 3 §3 "Sticky positioning"
/// <https://www.w3.org/TR/css-position-3/#sticky-pos>: `position: sticky`
/// lays out in normal flow but behaves as sticky relative to a scroll
/// container. Parsing preserves it as [`PositionValue::Sticky`]; layout
/// integration is future work. Currently, `apply_value` treats it like
/// `static`, doing nothing and emitting no running template. `relative`,
/// `absolute`, and `fixed` are not implemented yet and are silently dropped
/// (`None`).
///
/// `static` is an explicit variant so a later cascade winner can cancel an
/// earlier `position: running(x)`. If `.foo.reset { position: static }` wins
/// over `.foo { position: running(hdr) }`, `apply_value` does nothing and the
/// empty `running_templates` from `inherit_from` remains empty.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionValue {
    /// `static` — the spec default; suppresses `running()`.
    ///
    /// Do not derive `Default`: this crate derives it only when `.default()`
    /// is called (as with [`DisplayValue`]). The initialization site
    /// [`crate::computed::ComputedValues::initial`] explicitly sets the
    /// spec default.
    Static,
    /// `relative` — CSS Positioned Layout Module Level 3 §3 relative
    /// positioning. It stays in normal flow and shifts at paint time using
    /// inset offsets.
    Relative,
    /// `absolute` — CSS Positioned Layout Module Level 3 §3 absolute
    /// positioning. Currently parsed only; layout treats it like `static`
    /// until a future implementation.
    Absolute,
    /// `fixed` — CSS Positioned Layout Module Level 3 §3 fixed positioning.
    /// Currently parsed only; layout treats it like `static` until a future
    /// implementation.
    Fixed,
    /// `sticky` — CSS Positioned Layout Module Level 3 §3 sticky positioning
    /// (<https://www.w3.org/TR/css-position-3/#sticky-pos>). It stays in
    /// normal flow but behaves as sticky relative to scrolling. Parsing
    /// preserves [`PositionValue::Sticky`]; layout integration is future
    /// work. For now, `apply_value` treats it like `static`: no running
    /// template is emitted. `relative`, `absolute`, and `fixed` are not yet
    /// implemented and are silently dropped (`None`).
    Sticky,
    /// `running(<custom-ident>)`. The `<custom-ident>` is a case-preserved
    /// smol str.
    Running(SmolStr),
}

/// The keyword payload for `text-decoration-line`.
///
/// CSS Text Decoration Module Level 3 §2.1 "Text Decoration Lines: the
/// text-decoration-line property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>
/// value grammar: `none | [ underline || overline || line-through || blink ]`;
/// Initial: `none`; Inherited: **no** (separate prose describes propagation when
/// drawing decorations, but that does not change cascade inheritance); Computed
/// value: specified keyword(s).
///
/// # `||` (any-order) grammar and representation with bool flags
///
/// The `||` semantics of spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> require at least
/// one component, allow any order, and allow each component at most once. They
/// match [`parse_border_shorthand`]'s `||` (width || style || color); see that
/// function's docs for the detailed rationale. The four keywords independently
/// turn on or off, giving 16 combinations. CSS Values 4 `||` forbids only a
/// second occurrence of the same component (each alternative occurs at most
/// once); all 16 combinations themselves are valid under the grammar. Therefore,
/// use a struct with four independent `bool` fields rather than a dedicated
/// enum with 16 variants. `none` has every flag `false` (the spec treats `none`
/// and the absence of all four keywords as the same state).
///
/// CSS Text Decoration 4 places `spelling-error` / `grammar-error` outside the
/// `||` group as top-level alternatives (`none | [ ... ] |
/// spelling-error | grammar-error`). Keep them in the other two `bool` fields
/// ([`Self::spelling_error`] / [`Self::grammar_error`]); the parser rejects
/// combining either with the `||` group (see [`parse_text_decoration_line`]).
///
/// Do not add `#[non_exhaustive]`, following sibling [`OverflowXY`]. This type
/// is not re-exported by the umbrella (`raikiri` crate), and CSS Text Decoration
/// Level 4 defines six keywords (`||` group 4 + top-level 2), so future fields
/// are less likely than for [`Border`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextDecorationLine {
    /// `underline` — draws a decoration line along the text's under edge.
    pub underline: bool,
    /// `overline` — draws a decoration line along the text's over edge.
    pub overline: bool,
    /// `line-through` — draws a decoration line through the middle of the text.
    pub line_through: bool,
    /// `blink` — blinks the decoration line (the spec notes that UAs may ignore
    /// this keyword; painting behavior is decided by the paint implementation).
    pub blink: bool,
    /// `spelling-error` — a UA-defined spelling-error decoration (CSS Text Decoration 4 §2.1
    /// <https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-line>
    /// defines `none | [ underline || overline || line-through || blink ] |
    /// spelling-error | grammar-error`; as a top-level alternative, it cannot
    /// appear together with the `||` group).
    pub spelling_error: bool,
    /// `grammar-error` — likewise, a grammar-error decoration.
    pub grammar_error: bool,
}

impl TextDecorationLine {
    /// `none` — the spec initial value. No decoration line (all six flags are `false`).
    pub const NONE: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `underline` alone.
    pub const UNDERLINE: Self = Self {
        underline: true,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `overline` alone.
    pub const OVERLINE: Self = Self {
        underline: false,
        overline: true,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `line-through` alone.
    pub const LINE_THROUGH: Self = Self {
        underline: false,
        overline: false,
        line_through: true,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `blink` alone.
    pub const BLINK: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: true,
        spelling_error: false,
        grammar_error: false,
    };
    /// `spelling-error` alone.
    pub const SPELLING_ERROR: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: true,
        grammar_error: false,
    };
    /// `grammar-error` alone.
    pub const GRAMMAR_ERROR: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: true,
    };
}

/// The keyword payload for `text-decoration-style`.
///
/// CSS Text Decoration Module Level 3 §2.2 "Text Decoration Style: the
/// text-decoration-style property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>;
/// value grammar: `solid | double | dotted | dashed | wavy`; Initial: `solid`;
/// Inherited: no; Computed value: specified keyword.
///
/// Do not derive `Default`, following sibling [`DisplayValue`] / [`Direction`]:
/// the initialization code [`crate::computed::ComputedValues::initial`] specifies
/// the spec default directly.
///
/// Mark `#[non_exhaustive]` as for sibling [`BorderStyle`]: this is the convention
/// for line-style keyword enums and allows future variants without a breaking change.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationStyle {
    /// `solid` — the spec initial value.
    Solid,
    /// `double`.
    Double,
    /// `dotted`.
    Dotted,
    /// `dashed`.
    Dashed,
    /// `wavy`.
    Wavy,
}

css_keywords!(TextDecorationStyle {
    Solid => "solid",
    Double => "double",
    Dotted => "dotted",
    Dashed => "dashed",
    Wavy => "wavy",
});

/// The computed value of `text-decoration-color`. Like [`BorderColor`], it
/// distinguishes the `currentcolor` keyword from a resolved `<color>`.
///
/// CSS Text Decoration Module Level 3 §2.3 "Text Decoration Color: the
/// text-decoration-color property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>;
/// value grammar: `<color>`; Initial: `currentcolor`; Inherited: no;
/// Computed value: computed color. Resolving the used value (currentcolor →
/// this node's computed `color` property) belongs to the paint scope. This
/// follows the rationale in [`BorderColor`]'s "why keep an enum on the static
/// cascade side" section: a `text-decoration` shorthand can be built before
/// the winning `color` declaration is known.
///
/// Mark `#[non_exhaustive]` as for sibling [`BorderColor`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationColor {
    /// `currentcolor` keyword — the spec-mandated initial value.
    CurrentColor,
    /// Resolved `<color>` value — the payload for an author's explicit hex /
    /// named / `rgb(a)` / `transparent` specification.
    Resolved(CssColor),
}

/// A temporary carrier for the parsed `text-decoration` shorthand.
///
/// CSS Text Decoration Module Level 3 §2.4 "Text Decoration Shorthand: the
/// text-decoration property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-property> verbatim:
/// "This property is a shorthand for setting text-decoration-line,
/// text-decoration-color, and text-decoration-style in one declaration.
/// Omitted values are set to their initial values." — grammar
/// `<'text-decoration-line'> || <'text-decoration-thickness'> ||
/// `<'text-decoration-style'> || <'text-decoration-color'>`
/// (Level 4 adds thickness to the three Level 3 §2.4 components;
/// ED <https://drafts.csswg.org/css-text-decor-4/#text-decoration-property>).
///
/// This type exists only as the payload of [`PropertyValue::TextDecoration`].
/// [`crate::rule::expand_shorthand_into`] expands it into the four longhands
/// [`PropertyValue::TextDecorationLine`] / [`PropertyValue::TextDecorationThickness`] /
/// [`PropertyValue::TextDecorationStyle`] / [`PropertyValue::TextDecorationColor`]
/// and discards it. As with margin/padding/border/overflow shorthand, expansion
/// happens at parse time and this value never reaches the cascade (see that
/// function's docs). [`ComputedValues`] / [`SpecifiedValues`] do **not** hold
/// this type as a field: the four longhands have no computed-value coupling,
/// unlike [`OverflowXY`] (see its "cross-axis coupling" section). See the docs
/// for each of the four longhand fields for details.
///
/// Do not add `#[non_exhaustive]`, following sibling [`TextDecorationLine`]:
/// this shorthand-only carrier is not re-exported by the umbrella crate.
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDecorationShorthand {
    /// The `text-decoration-line` component; defaults to [`TextDecorationLine::NONE`]
    /// (the spec initial value) when omitted.
    pub line: TextDecorationLine,
    /// The `text-decoration-style` component; defaults to [`TextDecorationStyle::Solid`]
    /// (the spec initial value) when omitted.
    pub style: TextDecorationStyle,
    /// The `text-decoration-color` component; defaults to [`TextDecorationColor::CurrentColor`]
    /// (the spec initial value) when omitted.
    pub color: TextDecorationColor,
    /// The `text-decoration-thickness` component; defaults to
    /// [`TextDecorationThickness::Auto`] (the ED §2.4.1 initial value) when omitted.
    pub thickness: TextDecorationThickness,
}

/// The value of `text-decoration-skip-ink: auto | none | all`.
///
/// CSS Text Decoration 4
/// (<https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-skip-ink>).
/// Value: `auto | none | all`; Initial: `auto`; Inherited: **yes**;
/// Computed value: specified keyword. This style slice preserves the value but
/// does not implement skip-ink decoration geometry or painting.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationSkipInk {
    /// `auto` — the spec initial value.
    Auto,
    /// `none`.
    None,
    /// `all`.
    All,
}

css_keywords!(TextDecorationSkipInk {
    Auto => "auto",
    None => "none",
    All => "all",
});

/// The value of `text-decoration-skip-spaces: none | all | [ start || end ]`.
///
/// CSS Text Decoration 4
/// (<https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-skip-spaces>).
/// Value: `none | all | [ start || end ]`; Initial: `start end`;
/// Inherited: **yes**; Computed value: specified keyword(s).
/// Keep `all` distinct from `start end`: the initial value is `start end`, not
/// `all`. Represent the possible values with an enum of five variants.
/// The element style preserves the keyword set; decoration painting is out of scope.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationSkipSpaces {
    /// `none`.
    None,
    /// `all`.
    All,
    /// `start` alone.
    Start,
    /// `end` alone.
    End,
    /// `start end` / `end start` (either order; the spec initial value).
    StartEnd,
}

/// The value of `text-decoration-thickness: auto | from-font | <length-percentage>`.
///
/// CSS Text Decoration 4
/// (<https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-thickness>;
/// ED §2.4.1 also includes `<line-width>`; see scope carve-out below).
/// Value: `auto | from-font | <length-percentage>`; Initial: `auto`;
/// Inherited: **no**; Percentages: N/A; Computed value: specified keyword
/// or absolute length.
/// The ED grammar also includes `<line-width>` (`thin`/`medium`/`thick`), but this
/// implementation drops it as out of scope (it does not appear in the WPT vector).
/// The parser accepts `<percentage>`, but this computed-value slice does not handle it.
/// `Length` is staged in [`crate::specified::SpecifiedValues`] and converted to
/// an absolute length by [`crate::resolve::resolve_text_decoration_thickness`]
/// on the element path. No font-metric measurements such as `ch` are added for
/// this property.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextDecorationThickness {
    /// `auto` — the spec initial value.
    Auto,
    /// `from-font`.
    FromFont,
    /// `<length-percentage>` (within [`parse_length_value`]'s `allow_percentage=true`
    /// range, including [`Length::Percent`]).
    Length(Length),
}

/// The value of `text-decoration-inset: <length>{1,2} | auto`.
///
/// CSS Text Decoration 4 ED §2.9.1 (<https://drafts.csswg.org/css-text-decor-4/#text-decoration-inset-property>).
/// The ED grammar is `<length-percentage>{1,2} | auto`, but the WPT vector
/// (`text-decoration-inset-invalid.html` rejects `10%`) does not permit `%`.
/// This implementation therefore narrows it to `<length>{1,2} | auto`; this
/// records the discrepancy because the vector is the ground truth here.
/// Initial: `0`; Inherited: **no**. When omitted, the second value copies the
/// first, as in the margin/padding two-value rule.
/// The cascade and paint pipeline carries this value through computed style;
/// [`PropertyValue::TextDecorationInset`] is the specified-stage representation.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextDecorationInset {
    /// `auto`.
    Auto,
    /// One or two `<length>` values. If `end` is omitted, it equals `start`.
    Lengths {
        /// Start endpoint offset.
        start: Length,
        /// End endpoint offset.
        end: Length,
    },
}

/// The vertical component of `text-emphasis-position` (`[ over | under ]`).
///
/// Rather than CSS Text Decoration 3 §3.4 (<https://www.w3.org/TR/css-text-decor-3/#text-emphasis-position-property>),
/// this follows the first part of the ED grammar
/// <https://drafts.csswg.org/css-text-decor-4/#text-emphasis-position-property>:
/// `[ over | under ] && [ right | left ]?` (the vertical component is required).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisVEdge {
    /// `over`.
    Over,
    /// `under`.
    Under,
}

/// The horizontal component of `text-emphasis-position` (`[ right | left ]?`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisHEdge {
    /// Specifies `right`.
    Right,
    /// Specifies `left`.
    Left,
}

/// The value of `text-emphasis-position: auto | ([ over | under ] && [ right | left ]?)`.
///
/// See ED §3.4 (<https://drafts.csswg.org/css-text-decor-4/#text-emphasis-position-property>).
/// Value: `[ over | under ] && [ right | left ]?` (plus `auto`);
/// Initial: `over right`; Inherited: **yes**. The vertical component is required,
/// the horizontal component is optional, and their order is unrestricted
/// (`right under` is valid; `left over right` is invalid).
/// The element cascade stages this inherited keyword value as computed-equivalent;
/// emphasis placement and painting remain out of scope.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisPosition {
    /// Specifies `auto`.
    Auto,
    /// Specifies a vertical component with an optional horizontal component.
    Position {
        /// Specifies the required `[ over | under ]` component.
        vertical: TextEmphasisVEdge,
        /// Specifies the optional `[ right | left ]?` component.
        horizontal: Option<TextEmphasisHEdge>,
    },
}

/// Fill component of `text-emphasis-style`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisFill {
    /// The filled mark (the initial fill).
    Filled,
    /// The open mark.
    Open,
}

/// Shape component of `text-emphasis-style`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisShape {
    /// `dot`.
    Dot,
    /// `circle` (the initial shape).
    Circle,
    /// `double-circle`.
    DoubleCircle,
    /// `triangle`.
    Triangle,
    /// `sesame`.
    Sesame,
}

/// Parsed and computed `text-emphasis-style` value.
///
/// The value is inherited and has initial value `none`. Shape/fill components
/// are retained so computed style can serialize the canonical keyword form.
/// [`Self::DefaultShape`] is a transient
/// specified-value marker and is resolved before it reaches computed style.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextEmphasisStyle {
    /// `none`.
    None,
    /// A mark with its fill and shape. The initial fill and shape are
    /// [`TextEmphasisFill::Filled`] and [`TextEmphasisShape::Circle`].
    Shape {
        /// Filled or open mark.
        fill: TextEmphasisFill,
        /// Mark shape.
        shape: TextEmphasisShape,
    },
    /// A fill-only specified value whose shape depends on writing mode.
    ///
    /// This is resolved to [`Self::Shape`] before computed style is stored.
    DefaultShape {
        /// Filled or open mark.
        fill: TextEmphasisFill,
    },
    /// A custom string mark.
    String(SmolStr),
}

/// `text-emphasis` shorthand components before cascade expansion.
///
/// Omitted components use initial values: `none` for style and `currentColor`
/// for color. The rule layer expands this carrier to the two longhands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEmphasisShorthand {
    /// `text-emphasis-style` component.
    pub style: TextEmphasisStyle,
    /// `text-emphasis-color` component.
    pub color: TextDecorationColor,
}

/// The value of `text-underline-position: auto | [ from-font | under ] || [ left | right ]`.
///
/// See CSS Text Decoration 4 ED §2.7 (<https://drafts.csswg.org/css-text-decor-4/#text-underline-position-property>).
/// Value: `auto | [ from-font | under ] || [ left | right ]`;
/// Initial: `auto`; Inherited: **yes**.
/// `from-font` and `under` are mutually exclusive (`under from-font` is invalid),
/// as are `left` and `right` (`left right` is invalid). `auto` must stand alone
/// (`auto under` is invalid). This uses the same Boolean flags plus parser
/// enforcement as [`TextDecorationLine`].
/// The element cascade preserves this inherited keyword set as the computed value;
/// underline placement remains out of scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextUnderlinePosition {
    /// Specifies `from-font` (mutually exclusive with `under`).
    pub from_font: bool,
    /// Specifies `under` (mutually exclusive with `from-font`).
    pub under: bool,
    /// Specifies `left` (mutually exclusive with `right`).
    pub left: bool,
    /// Specifies `right` (mutually exclusive with `left`).
    pub right: bool,
}

impl TextUnderlinePosition {
    /// The spec initial value `auto`, with all flags set to `false`.
    pub const AUTO: Self = Self {
        from_font: false,
        under: false,
        left: false,
        right: false,
    };
}

/// The value of `page: auto | <custom-ident>`.
///
/// See CSS Paged Media 3 §8.1 "Using named pages: page"
/// (<https://www.w3.org/TR/css-page-3/#using-named-pages>).
/// Value: `auto | <custom-ident>`; Initial: `auto`;
/// Applies to: boxes that create class A break points; Inherited: **no**;
/// Computed value: specified value.
/// A `<custom-ident>` is a single ident other than a CSS-wide keyword.
/// WPT (`page-invalid.html`) also rejects `default`, so it is excluded here.
/// The general mechanism (single-ident parse plus the caller's
/// `expect_exhausted`) drops `not valid` (two idents), `123px`, and `calc()`
/// because they do not match the grammar.
/// This value is parsing-only (see the [`PropertyValue::Page`] documentation).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageValue {
    /// Specifies `auto`, the spec initial value.
    Auto,
    /// Specifies a named page (`<custom-ident>`).
    Named(Atom),
}

/// The value of the `vertical-align` property.
///
/// CSS 2.1 §10.8.1 "Vertical alignment: the 'vertical-align' property"
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>.
///
/// propdef (spec verbatim): Value: `baseline | sub | super | top | text-top
/// | middle | bottom | text-bottom | <percentage> | <length> | inherit`;
/// Initial: `baseline`; Applies to: inline-level and 'table-cell' elements;
/// Inherited: **no**; Percentages: refer to the 'line-height' of the element
/// itself; Computed value: "for `<percentage>` and `<length>` the absolute
/// length, otherwise as specified".
///
/// # Why CSS 2.1 is the primary source
///
/// CSS Inline Layout Module Level 3
/// <https://www.w3.org/TR/css-inline-3/#vertical-align> redefines `vertical-align`
/// as a shorthand for the three longhands `alignment-baseline`,
/// `baseline-source`, and `baseline-shift`, but it has no compatibility mapping
/// for the full classic keyword grammar (`baseline` / `sub` / `super` / `top` /
/// ...). CSS 2.1 §10.8.1 alone provides a complete, consistent definition
/// of that grammar, so this crate uses it as the primary source.
///
/// # Scope carving
///
/// - **Implemented**: The `baseline` / `sub` / `super` / `top` / `bottom`
///   keywords and edge placement within the minimal line-box scope.
///   Of these, `baseline` (the spec initial value), `sub`, and `super` do not
///   carry a percentage or length, so their computed value is the specified
///   keyword without relative resolution. raikiri-paint applies these three
///   keywords to the glyph's actual drawing position (see "Calculating the
///   baseline shift is raikiri-paint's responsibility" below).
/// - **Implemented**: The three `middle` / `text-top` / `text-bottom` keywords
///   (§10.8.1 spec verbatim):
///   - `middle`: "Align the vertical midpoint of the box with the
///     baseline of the parent box plus half the x-height of the parent."
///   - `text-top`: "Align the top of the box with the top of the
///     parent's content area."
///   - `text-bottom`: "Align the bottom of the box with the bottom of
///     the parent's content area."
///
///   Like `sub` and `super`, these values depend only on **the parent's font
///   metrics** (its baseline, x-height, and the top or bottom of its content
///   area); they do not need the extent of other boxes in the line box.
///   `top` and `bottom` are handled separately in the minimal line-box scope
///   below. Since these keywords carry neither a percentage nor a length,
///   the computed value remains the specified keyword.
/// - **Implemented**: A `<length>` value (§10.8.1 spec verbatim: "Raise
///   (positive value) or lower (negative value) the box by this
///   distance. The value `0cm` means the same as `baseline`."). An absolute
///   value does not need a reference (line-height or font metrics), so the
///   existing length resolver ([`crate::resolve::resolve_length`], also used
///   for the `<length>` components of `letter-spacing` and `word-spacing`)
///   can absolutize it without approximation. There is no sign restriction:
///   the spec explicitly permits negative values, as with `letter-spacing`
///   and `margin-*`. `<percentage>` is a separate grammar alternative and
///   is not included in this variant (see the "Unsupported: `<percentage>`"
///   section below).
///
///   The computed value keeps the same [`VerticalAlign`] type as the
///   specified value. See "Why the type is unchanged for computed values" in
///   the [`crate::computed::ComputedValues::vertical_align`] documentation.
///   The `@page` phase-3 pipeline ([`crate::page`]'s
///   `absolutize_in_page_context` / `specified_layer_residue`) also has a
///   dedicated match arm for this variant.
/// - **Implemented**: The `top` / `bottom` keywords are parsed and cascaded
///   as line-box edge alignment per CSS 2.1 §10.8.1; the inline engine
///   aligns the box with the line box edge.
/// - **Implemented**: A `<percentage>` value (§10.8.1 propdef
///   "Percentages: refer to the 'line-height' of the element itself").
///   The value is absolutized as a percentage of the element's own used
///   `line-height` (see [`crate::resolve::resolve_vertical_align`]). For
///   example, `50%` → `Length::Percent(50.0)` at parse time, then resolves to
///   `used_line_height_length * p / 100` in phase 3. Under the spec, `0%`
///   means the same as `baseline` (as does "`0cm` means the same as
///   `baseline`" in the `<length>` section above). Negative values are also
///   accepted as spec-valid (the same "Raise/lower" semantics as `<length>`).
///
/// - **Implemented**: Mixed-unit `calc()` values flow through the shared
///   deferred CSS math path as [`CalcLengthPercentage`]. The px term is kept,
///   and the percentage term resolves against this element's used
///   `line-height`; the result is stored as [`Length::Px`] before paint.
///
///   **Spec-deviation fallback for `line-height: normal`**: When
///   `line-height: normal` applies (the spec initial value and the default
///   for an element with no declaration),
///   [`crate::resolve::used_line_height_length`] returns `None`. The style
///   layer has no real font metrics, so it cannot convert "normal" to an
///   absolute length; the "normal" wall in that function's documentation
///   is canonical. Normally, used line-height has an absolute length from
///   font metrics, which would allow the percentage to resolve naturally.
///   This crate has no source of font metrics, so it falls back to `0px` (equivalent to `baseline`). This
///   independent fallback does not invent a ratio. It differs from the
///   staging used for `<percentage>` in `padding` and `margin`, which leaves
///   the value unresolved in computed style for the consumer to resolve.
///   This per-property fallback was chosen because those consumers
///   (raikiri-dom and raikiri-paint) do not currently read
///   [`crate::computed::ComputedValues::line_height`]. It is a documented
///   deviation like the `None` → `0px` fallback for `Length::Lh` and
///   `Length::Rlh` (see [`crate::resolve::resolve_length`]).
/// - **Implemented**: CSS-wide defaulting and cascade rollback use deferred
///   longhand markers. Explicit `inherit` copies the parent's computed value;
///   `initial` and `unset` use `baseline`, since this property is not inherited.
/// - **(a) Invalid under the spec**: Any other ident is silently dropped
///   as `None`.
/// - **Calculating the baseline shift is raikiri-paint's responsibility**:
///   raikiri-paint calculates the actual shifts for `sub`, `super`, `middle`,
///   `text-top`, and `text-bottom` (px offsets based on the parent's used
///   font-size or font metrics) and applies them to glyph placement. This
///   crate only carries the five bare keywords and the absolutized px values
///   of `<length>`; it does not calculate the shift. Currently, raikiri-paint
///   explicitly calculates shifts only for `sub` and `super`. Other values,
///   including `middle`, `text-top`, `text-bottom`, and `<length>`, temporarily
///   get a 0px shift through the `#[non_exhaustive]` wildcard fallback (see
///   "Accepting cascade-regression risk" below).
///
/// # Accepting cascade-regression risk (`Middle`/`TextTop`/`TextBottom`)
///
/// This section records a deliberate departure from a principle documented
/// in an earlier version. That version did not accept a keyword at parse time
/// unless raikiri-paint implemented its shift. The reason was that an
/// unimplemented keyword declared by the UA or author at higher cascade
/// priority than the implemented `sub` or `super` could correctly win the
/// cascade, but raikiri-paint would treat it as shift 0. **An element that
/// previously shifted correctly would silently revert to shift 0.** This
/// was qualitatively different from an unsupported value merely having no
/// effect: it displaced a supported value and caused a regression.
///
/// `Middle`, `TextTop`, and `TextBottom` were added as explicit exceptions
/// to that principle. The reasons are: (1) all three keywords depend only
/// on the parent's font metrics, like `sub` and `super` (see "Implemented"
/// above), and are equally calculable, unlike `top` and `bottom`, which
/// depend on the whole line box. (2) raikiri-paint's shift-calculation arm
/// already treated every keyword other than `sub` and `super` uniformly as
/// a 0px shift through a `#[non_exhaustive]` wildcard. Adding these three
/// keywords therefore introduces no new kind of failure. It merely extends
/// the existing, accepted form of regression risk to more keywords.
/// `top` and `bottom` are accepted in the style layer and passed to minimal
/// line-box layout. `<percentage>` is implemented with the
/// `line-height: normal` fallback described under "Implemented:
/// `<percentage>`" above. Unlike `top` and `bottom`, it can be fully
/// absolutized within raikiri-style. It represented a different gap,
/// unrelated to cascade-regression risk from unimplemented keywords
/// displacing implemented values; checking the fallback in this change
/// resolved that gap.
///
/// `Default` is not derived, following sibling [`TextDecorationShorthand`].
/// The initializer [`crate::computed::ComputedValues::initial`] specifies
/// the spec default directly.
///
/// `Eq` is not derived because the [`Length`] carried by [`Self::Length`]
/// has an `f32` field (the same constraint as [`FlexBasisValue`]).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VerticalAlign {
    /// `baseline` is the spec initial value. It aligns the box's baseline
    /// with the parent's baseline, without an additional shift (§10.8.1 spec
    /// verbatim: "Align the baseline of the box with the baseline of the parent box.").
    Baseline,
    /// Specifies `sub`, per §10.8.1 (spec verbatim): "Lower the baseline of
    /// the box to the proper position for subscripts of the parent's box.
    /// (This value has no effect on the font size of the element's text.)"
    Sub,
    /// Specifies `super`, per §10.8.1 (spec verbatim): "Raise the baseline of
    /// the box to the proper position for superscripts of the parent's box.
    /// (This value has no effect on the font size of the element's text.)"
    Super,
    /// Specifies `middle`, per §10.8.1 (spec verbatim): "Align the vertical
    /// midpoint of the box with the baseline of the parent box plus half
    /// the x-height of the parent."
    Middle,
    /// Specifies `text-top`, per §10.8.1 (spec verbatim): "Align the top of
    /// the box with the top of the parent's content area."
    TextTop,
    /// Specifies `text-bottom`, per §10.8.1 (spec verbatim): "Align the bottom
    /// of the box with the bottom of the parent's content area."
    TextBottom,
    /// `top` — align the top of the aligned subtree with the top of the line box
    /// (CSS 2.1 §10.8.1). Layout consumes this keyword in the minimal line-box
    /// bridge; the style layer carries it unchanged.
    Top,
    /// `bottom` — align the bottom of the aligned subtree with the bottom of the
    /// line box (CSS 2.1 §10.8.1). Layout consumes this keyword in the minimal
    /// line-box bridge; the style layer carries it unchanged.
    Bottom,
    /// `<length>` / `<percentage>` — "Raise (positive value) or lower
    /// (negative value) the box by this distance. The value `0cm` means the
    /// same as `baseline`." (§10.8.1 spec verbatim). The `<percentage>` value
    /// resolves to an absolute length against `line-height`, as specified by
    /// "Percentages: refer to the 'line-height' of the element itself" in
    /// the same propdef. Computed style carries the absolutized `Length::Px`.
    /// See the "Implemented: `<percentage>`" section in the [`Self`]
    /// documentation and [`crate::resolve::resolve_vertical_align`]. The
    /// fallback for `normal` is `0px`. This variant also includes `calc()`
    /// values reducible to a single length or percentage.
    Length(Length),
    /// Deferred mixed `<length-percentage>` `calc()` value. The shared CSS
    /// math path preserves its px and percentage coefficients; phase 3 resolves
    /// the percentage against this element's used `line-height` and replaces
    /// this variant with [`Self::Length`] before paint.
    Calc(CalcLengthPercentage),
}

/// The value of the `z-index` property.
///
/// CSS Positioned Layout Module Level 3 does not itself formally define this
/// property — it states only "The [z-index] property applies to all
/// positioned boxes" and defers detail to CSS2
/// (<https://drafts.csswg.org/css-position-3/#z-index-property>: "See CSS2 §
/// 9.9 Layered presentation ... for details about z-index"). The propdef
/// therefore lives in CSS2 §9.9.1 "Specifying the stack level: the 'z-index'
/// property" <https://www.w3.org/TR/CSS2/visuren.html#z-index>.
///
/// propdef (CSS2 spec verbatim): Value: `auto | <integer> | inherit`,
/// Initial: `auto`, Applies to: positioned elements, Inherited: **no**,
/// Computed value: "as specified".
///
/// Meanings of values (CSS2 §9.9.1 spec verbatim):
/// - `<integer>`: "This integer is the stack level of the generated box in
///   the current stacking context. The box also establishes a new stacking
///   context."
/// - `auto`: "The stack level of the generated box in the current stacking
///   context is 0. The box does not establish a new stacking context unless
///   it is the root element."
///
/// # Scope carving
///
/// This crate's `position` property implementation ([`PositionValue`] doc)
/// only recognizes `static`, `sticky` (CSS Positioned Layout Module Level 3 §3)
/// and CSS GCPM 3's `running(<custom-ident>)` —
/// the CSS2 `relative` / `absolute` / `fixed` keywords that the
/// propdef's "Applies to: positioned elements" clause presupposes are not
/// implemented yet (`sticky` is parsed but has no layout consumer yet).
/// Stacking-context construction and paint-order
/// consumption of this value are therefore also out of scope here: this
/// type only carries the cascaded value through to
/// [`crate::computed::ComputedValues::z_index`], mirroring how
/// [`FontStyle`] / [`VerticalAlign`] are cascaded and stored before any
/// layout-side consumer exists for them.
///
/// `Default` is not derived, following [`Direction`] and [`BoxSizing`].
/// The initializers [`crate::specified::SpecifiedValues::initial`] and
/// [`crate::computed::ComputedValues::initial`] specify the spec default
/// directly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZIndexValue {
    /// `auto` — spec initial value. "The stack level of the generated box
    /// in the current stacking context is 0. The box does not establish a
    /// new stacking context unless it is the root element." (CSS2 §9.9.1
    /// verbatim) Note: css-position-3 §2.2 overrides this for fixed and
    /// sticky positioned boxes — they nonetheless form a stacking context
    /// even when `z-index` is `auto`
    /// (<https://www.w3.org/TR/css-position-3/#stacking-context>).
    Auto,
    /// `<integer>` — "the stack level of the generated box in the current
    /// stacking context. The box also establishes a new stacking context."
    /// (CSS2 §9.9.1 verbatim)
    Integer(i32),
}

/// The value of the `word-break` property.
///
/// CSS Text Module Level 3 §5.1 "Breaking Rules for Letters: the word-break
/// property" <https://www.w3.org/TR/css-text-3/#word-break-property>.
///
/// propdef (spec verbatim): Value: `normal | keep-all | break-all |
/// break-word`; Initial: `normal`; Applies to: text; Inherited: **yes**;
/// Computed value: specified keyword.
///
/// # Meaning of the three keywords (verified verbatim against the spec)
///
/// - [`Normal`](Self::Normal) — "Words break according to their customary
///   rules, as described above. Korean, which commonly exhibits two
///   different behaviors, allows breaks between any two consecutive
///   Hangul/Hanja. For Ethiopic, which also exhibits two different
///   behaviors, such breaks within words are not allowed." This is the
///   spec initial value.
/// - [`KeepAll`](Self::KeepAll) — "Breaking is forbidden within 'words':
///   implicit soft wrap opportunities between typographic letter units (or
///   other typographic character units belonging to the NU, AL, AI, or ID
///   Unicode line breaking classes) are suppressed, i.e. breaks are
///   prohibited between pairs of such characters (regardless of line-break
///   settings other than anywhere) except where opportunities exist due to
///   dictionary-based breaking."
/// - [`BreakAll`](Self::BreakAll) — "Breaking is allowed within 'words':
///   specifically, in addition to soft wrap opportunities allowed for
///   normal, any typographic letter units (and any typographic character
///   units resolving to the NU ('numeric'), AL ('alphabetic'), or SA
///   ('Southeast Asian') line breaking classes) are instead treated as ID
///   ('ideographic characters') for the purpose of line-breaking.
///   Hyphenation is not applied."
///
/// # Scope carving
///
/// - **Non-goal**: the spec's 4th keyword, a deprecated `break-word` value
///   on `word-break` itself — spec verbatim: "For compatibility with legacy
///   content, the word-break property also supports a deprecated
///   break-word keyword. When specified, this has the same effect as
///   word-break: normal and overflow-wrap: anywhere, regardless of the
///   actual value of the overflow-wrap property." Representing that would
///   mean one property's parsed value forcing a *different* property
///   (`overflow-wrap`) to a specific value — a cross-property override this
///   crate's per-property parse/cascade model has no slot for. `word-break:
///   break-word` is silent-dropped like any other unhandled ident, the same
///   way [`FontStyle`]'s unimplemented `left`/`right` keywords are. It is
///   [`OverflowWrap::BreakWord`] — the non-deprecated
///   `overflow-wrap: break-word` value — that this crate represents
///   instead.
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped. See the "CSS-wide keywords" section in the
///   [`PropertyValue`] documentation for the canonical list of five keywords
///   and the reason.
/// - **(a) Invalid under the spec**: Any ident other than the three keywords
///   above (`normal` / `keep-all` / `break-all`; see the non-goal section above
///   for `break-word`) is silently dropped as `None`.
///
/// This crate does not carry lengths in this scope, so the computed value
/// remains the specified keyword without relative resolution, as with
/// [`Direction`].
///
/// `Default` is not derived, following [`Direction`] and [`FontStyle`].
/// The initializers [`crate::specified::SpecifiedValues::initial`] and
/// [`crate::computed::ComputedValues::initial`] specify [`WordBreak::Normal`]
/// directly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordBreak {
    /// Specifies `normal`, the spec initial value.
    Normal,
    /// Specifies `keep-all`.
    KeepAll,
    /// Specifies `break-all`.
    BreakAll,
    /// `manual` — CSS Text 4 / WPT word-break-valid.
    Manual,
    /// `auto-phrase` — CSS Text 4 / WPT word-break-valid.
    AutoPhrase,
    /// `break-word` — deprecated but WPT expects valid (word-break-valid.html).
    BreakWord,
}

css_keywords!(WordBreak {
    Normal => "normal",
    KeepAll => "keep-all",
    BreakAll => "break-all",
    Manual => "manual",
    AutoPhrase => "auto-phrase",
    BreakWord => "break-word",
});

/// The value of the `overflow-wrap` property. Its legacy name alias
/// `word-wrap` refers to the same property (see "Legacy alias" below).
///
/// CSS Text Module Level 3 §5.4 "Overflow Wrapping: the overflow-wrap
/// (word-wrap) property"
/// <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>.
///
/// propdef (spec verbatim): Value: `normal | break-word | anywhere`;
/// Initial: `normal`; Applies to: text; Inherited: **yes**; Computed value:
/// specified keyword.
///
/// # Legacy alias (`word-wrap`)
///
/// spec verbatim: "For legacy reasons, UAs must treat word-wrap as a legacy
/// name alias of the overflow-wrap property." — `parse_value` dispatches
/// both the `"overflow-wrap"` and `"word-wrap"` property names to the same
/// [`PropertyValue::OverflowWrap`] variant / [`PropertyKey::OverflowWrap`]
/// key, so the two names cascade against each other as one property (a
/// declaration under either name can win over a declaration under the
/// other), not as two independently-winning properties.
///
/// # Meaning of the three keywords (verified verbatim against the spec)
///
/// - [`Normal`](Self::Normal) — "Lines may break only at allowed break
///   points. However, the restrictions introduced by word-break: keep-all
///   may be relaxed to match word-break: normal if there are no
///   otherwise-acceptable break points in the line." This is the spec initial value.
/// - [`BreakWord`](Self::BreakWord) — "As for anywhere except that soft
///   wrap opportunities introduced by break-word are not considered when
///   calculating min-content intrinsic sizes."
/// - [`Anywhere`](Self::Anywhere) — "An otherwise unbreakable sequence of
///   characters may be broken at an arbitrary point if there are no
///   otherwise-acceptable break points in the line. Shaping characters are
///   still shaped as if the word were not broken, and grapheme clusters
///   must stay together as one unit. No hyphenation character is inserted
///   at the break point. Soft wrap opportunities introduced by anywhere are
///   considered when calculating min-content intrinsic sizes."
///
/// # Scope carving
///
/// - **Non-goal**: the `BreakWord` / `Anywhere` distinction quoted above
///   (whether the soft wrap opportunity counts toward min-content intrinsic
///   size) is a layout-time distinction this crate does not compute
///   intrinsic sizes for yet. Both keywords are still represented as
///   distinct variants here (unlike `word-break`'s deprecated `break-word`
///   value, which [`WordBreak`]'s doc explains is not represented at all)
///   so the distinction survives for a future layout consumer even though
///   nothing reads it yet.
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet and are
///   silently dropped. See the "CSS-wide keywords" section in the
///   [`PropertyValue`] documentation for the canonical list of five keywords
///   and the reason.
/// - **(a) Invalid under the spec**: Any ident other than the three keywords
///   above is silently dropped as `None`.
///
/// This crate does not carry lengths in this scope, so the computed value
/// remains the specified keyword without relative resolution, as with
/// [`Direction`].
///
/// `Default` is not derived, following [`Direction`] and [`WordBreak`].
/// The initializers [`crate::specified::SpecifiedValues::initial`] and
/// [`crate::computed::ComputedValues::initial`] specify [`OverflowWrap::Normal`]
/// directly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowWrap {
    /// Specifies `normal`, the spec initial value.
    Normal,
    /// Specifies `break-word`.
    BreakWord,
    /// Specifies `anywhere`.
    Anywhere,
}

css_keywords!(OverflowWrap {
    Normal => "normal",
    BreakWord => "break-word",
    Anywhere => "anywhere",
});

/// The shared value of the `break-before` and `break-after` properties
/// ([`PropertyValue::BreakBefore`] and [`PropertyValue::BreakAfter`]); both
/// properties use the same grammar.
///
/// CSS Fragmentation Module Level 3 §3.1 "Breaks Between Boxes: the
/// break-before and break-after properties"
/// <https://www.w3.org/TR/css-break-3/#break-between>.
///
/// propdef (spec verbatim): Value: `auto | avoid | avoid-page | page | left
/// | right | recto | verso | avoid-column | column | avoid-region |
/// region`, Initial: `auto`, Applies to: "block-level boxes, grid items,
/// flex items, table row groups, table rows (but see prose)", Inherited:
/// **no**, Computed value: "specified keyword".
///
/// # Scope carving
///
/// This type preserves generic, page and column break values. `always`
/// follows CSS Fragmentation Level 4 and forces a break in the immediately
/// containing fragmentation context:
/// <https://drafts.csswg.org/css-break-4/#break-between>.
///
/// Page-spread values (`left`, `right`, `recto`, `verso`), CSS Regions
/// values (`avoid-region`, `region`), and the Level 4 `all` value remain
/// unsupported. Column-specific values stay distinct from generic values
/// so pagination does not apply column-only constraints.
///
/// # `page-break-before` / `page-break-after` (CSS2.1 legacy shorthand)
///
/// CSS Fragmentation Module Level 3 §3.4 "Page Break Aliases"
/// <https://www.w3.org/TR/css-break-3/#page-break-properties> defines the
/// CSS2.1 `page-break-before` / `page-break-after` properties as **legacy
/// shorthands** (the spec's own term, linking CSS Cascading Level 4's
/// <https://www.w3.org/TR/css-cascade-4/#legacy-shorthand> "legacy
/// shorthand" definition — not a plain name alias the way CSS Text 3
/// words `word-wrap` / `overflow-wrap`, see [`OverflowWrap`] doc) for
/// `break-before` / `break-after`, with an explicit non-identity value
/// mapping (spec's own table, §3.4):
///
/// | `page-break-*` value | `break-*` value |
/// |---|---|
/// | `auto` | `auto` |
/// | `avoid` | `avoid` |
/// | `always` | `page` |
///
/// (The table's own first row also lists `left` / `right` as
/// identity-mapped — out of scope here per the "Scope carving" section
/// above, so `page-break-before: left` / `: right` are rejected the same
/// way `break-before: left` is.) Unlike the `padding` / `border` /
/// `text-decoration` shorthands elsewhere in this crate, this legacy
/// shorthand expands to exactly **one** longhand with a 1:1 value mapping
/// — there is no multi-longhand fan-out, so `parse_value` dispatches
/// `page-break-before` / `page-break-after` directly to
/// [`PropertyValue::BreakBefore`] / [`PropertyValue::BreakAfter`] (via a
/// dedicated remapping parser) rather than going through
/// [`crate::rule::expand_shorthand_into`]'s longhand-expansion machinery,
/// which exists for shorthands that fan out to multiple independent
/// cascade winners.
///
/// Do not derive `Default` — following the convention of [`ZIndexValue`] (the spec
/// initial value is set directly by [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakBetween {
    /// `auto` — spec initial value. "Neither force nor forbid a break
    /// before/after the principal box." (§3.1 verbatim)
    Auto,
    /// `avoid` — "Avoid a break before/after the principal box." (§3.1
    /// verbatim)
    Avoid,
    /// `avoid-page` — "Avoid a page break before/after the principal box."
    /// (§3.1 verbatim, "Page Break Values" — only has an effect in
    /// paginated contexts)
    AvoidPage,
    /// `page` — "Always force a page break before/after the principal
    /// box." (§3.1 verbatim) — also the `page-break-before` /
    /// `page-break-after` legacy shorthand's remap target for `always`
    /// (this type's doc's "legacy shorthand" section).
    Page,
    /// Forces a break in the immediately containing fragmentation context.
    Always,
    /// Avoids a column break before or after the principal box.
    AvoidColumn,
    /// Forces a column break before or after the principal box.
    Column,
}

css_keywords!(BreakBetween {
    Auto => "auto",
    Avoid => "avoid",
    AvoidPage => "avoid-page",
    Page => "page",
    Always => "always",
    AvoidColumn => "avoid-column",
    Column => "column",
});

/// The value of the `break-inside` property.
///
/// CSS Fragmentation Module Level 3 §3.2 "Breaks Within Boxes: the
/// break-inside property" <https://www.w3.org/TR/css-break-3/#break-within>.
///
/// propdef (spec verbatim): Value: `auto | avoid | avoid-page |
/// avoid-column | avoid-region`, Initial: `auto`, Applies to: "all elements
/// except inline-level boxes, internal ruby boxes, table column boxes,
/// table column group boxes, absolutely-positioned boxes", Inherited:
/// **no**, Computed value: "specified keyword".
///
/// This is a **smaller, disjoint** value set from [`BreakBetween`] — only
/// `avoid`-flavored keywords exist ("breaking within" has no start/end
/// edge to force a break relative to, so the forced-break value `page`
/// that [`BreakBetween`] carries has no `break-inside` counterpart at
/// all). Sharing [`BreakBetween`] for both properties would silently
/// over-accept `break-inside: page`, which the spec grammar above does not
/// have — hence this separate type.
///
/// # Scope carving
///
/// This type implements `auto`, generic `avoid`, and the context-specific
/// `avoid-page` and `avoid-column` values. `avoid-region` remains unsupported
/// because CSS Regions fragmentation is not modeled.
///
/// # `page-break-inside` (CSS2.1 legacy shorthand)
///
/// CSS Fragmentation Module Level 3 §3.4 "Page Break Aliases"
/// <https://www.w3.org/TR/css-break-3/#page-break-properties> defines the
/// CSS2.1 `page-break-inside` property as a legacy shorthand ([`BreakBetween`]
/// doc's "legacy shorthand" section explains the spec's "legacy shorthand"
/// vs. "legacy name alias" distinction) for `break-inside`. Unlike
/// `page-break-before` / `page-break-after`, this shorthand's value
/// mapping is **identity** — CSS2.1's own `page-break-inside` propdef
/// grammar is just `auto | avoid` (no `always` / `left` / `right`), and
/// both keywords already exist unchanged on `break-inside`. `parse_value`
/// dispatches `page-break-inside` to a dedicated parser that only accepts
/// this 2-keyword CSS2.1 grammar (not the fuller `break-inside` grammar
/// above) — `page-break-inside: avoid-page` is rejected, since it is not
/// valid CSS2.1 `page-break-inside` syntax.
///
/// Do not derive `Default` — following the convention of [`ZIndexValue`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakInside {
    /// `auto` — spec initial value. "Impose no additional breaking
    /// constraints within the box." (§3.2 verbatim)
    Auto,
    /// `avoid` — "Avoid breaks within the box." (§3.2 verbatim)
    Avoid,
    /// `avoid-page` — "Avoid a page break within the box." (§3.2 verbatim)
    AvoidPage,
    /// Avoids a column break within the box.
    AvoidColumn,
}

css_keywords!(BreakInside {
    Auto => "auto",
    Avoid => "avoid",
    AvoidPage => "avoid-page",
    AvoidColumn => "avoid-column",
});

/// The value of the `float` property.
///
/// CSS2 §9.5.1 "Positioning the float: the 'float' property"
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>. Value: `left |
/// right | none | inherit`, Initial: `none`, Applies to: "all, but see 9.7",
/// Inherited: **no**, Percentages: N/A, Media: visual, Computed value: "as
/// specified".
///
/// Meanings of values (CSS2 §9.5.1 spec verbatim):
/// - `left`: "The element generates a block box that is floated to the
///   left. Content flows on the right side of the box, starting at the top
///   (subject to the 'clear' property)."
/// - `right`: "Similar to 'left', except the box is floated to the right,
///   and content flows on the left side of the box, starting at the top."
/// - `none`: "The box is not floated."
///
/// # Scope carving
///
/// - `position`'s `absolute` / `fixed` values are not implemented by this
///   crate yet ([`PositionValue`] doc's "not implemented" note). CSS2 §9.7's
///   `display`/`position`/`float` algorithm forces the computed value of
///   `float` to `none` on an absolutely positioned box, so that interaction
///   currently has no observable effect on any element this crate can
///   style.
/// - CSS2 §9.7's mandated `display` recomputation when this value is not
///   `none` **is** implemented (unlike most Scope carving notes in this
///   file, this is not a cut) — see [`resolve_display_for_float`] doc.
/// - Actual float positioning, shrink-to-fit width, and line-box
///   shortening (CSS2 §9.5's exclusion-area algorithm) are layout-time
///   behavior (raikiri-dom scope). This crate only carries the cascaded
///   keyword through to [`crate::computed::ComputedValues::float`].
///
/// Do not derive `Default` — following the convention of [`ZIndexValue`] (the spec initial value
/// is set directly by [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatValue {
    /// `none` — spec initial value. "The box is not floated." (CSS2 §9.5.1
    /// verbatim)
    None,
    /// `left` — "The element generates a block box that is floated to the
    /// left. Content flows on the right side of the box, starting at the
    /// top (subject to the 'clear' property)." (CSS2 §9.5.1 verbatim)
    Left,
    /// `right` — "Similar to 'left', except the box is floated to the
    /// right, and content flows on the left side of the box, starting at
    /// the top." (CSS2 §9.5.1 verbatim)
    Right,
    /// `inline-start` — logical equivalent of `left`/`right` (CSS Logical Properties §3).
    InlineStart,
    /// `inline-end` — logical equivalent of `left`/`right` (CSS Logical Properties §3).
    InlineEnd,
    /// `footnote` — removes the box from normal flow and places it in the
    /// footnote area of the page containing its anchor (CSS Generated Content
    /// for Paged Media).
    Footnote,
}

css_keywords!(FloatValue {
    None => "none",
    Left => "left",
    Right => "right",
    InlineStart => "inline-start",
    InlineEnd => "inline-end",
    Footnote => "footnote",
});

/// The value of the `clear` property.
///
/// CSS2 §9.5.2 "Controlling flow next to floats: the 'clear' property"
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>. Value: `none |
/// left | right | both | inherit`, Initial: `none`, Applies to: block-level
/// elements, Inherited: **no**, Percentages: N/A, Media: visual, Computed
/// value: "as specified".
///
/// Meanings of values (CSS2 §9.5.2 spec verbatim, "Values have the
/// following meanings when applied to non-floating block-level boxes"):
/// - `left`: "Requires that the top border edge of the box be below the
///   bottom outer edge of any left-floating boxes that resulted from
///   elements earlier in the source document."
/// - `right`: "Requires that the top border edge of the box be below the
///   bottom outer edge of any right-floating boxes that resulted from
///   elements earlier in the source document."
/// - `both`: "Requires that the top border edge of the box be below the
///   bottom outer edge of any right-floating and left-floating boxes that
///   resulted from elements earlier in the source document."
/// - `none`: "No constraint on the box's position with respect to floats."
///
/// # Scope carving
///
/// - The propdef's "Applies to: block-level elements" clause is not
///   enforced by the parser — applicability gating by computed `display`
///   is layout-time / consumer scope in this crate, the same split
///   [`VerticalAlign`] doc's "Applies to: inline-level ... table-cell"
///   note describes for that property.
/// - Clearance computation (CSS2 §9.5.2's "Computing the clearance of an
///   element on which 'clear' is set") and the vertical displacement it
///   produces are layout-time behavior (raikiri-dom scope). This crate
///   only carries the cascaded keyword through to
///   [`crate::computed::ComputedValues::clear`].
///
/// Do not derive `Default` — following the convention of [`FloatValue`] (the spec initial value
/// is set directly by [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearValue {
    /// `none` — spec initial value. "No constraint on the box's position
    /// with respect to floats." (CSS2 §9.5.2 verbatim)
    None,
    /// `left` — "Requires that the top border edge of the box be below the
    /// bottom outer edge of any left-floating boxes that resulted from
    /// elements earlier in the source document." (CSS2 §9.5.2 verbatim)
    Left,
    /// `right` — "Requires that the top border edge of the box be below
    /// the bottom outer edge of any right-floating boxes that resulted
    /// from elements earlier in the source document." (CSS2 §9.5.2
    /// verbatim)
    Right,
    /// `both` — "Requires that the top border edge of the box be below
    /// the bottom outer edge of any right-floating and left-floating
    /// boxes that resulted from elements earlier in the source document."
    /// (CSS2 §9.5.2 verbatim)
    Both,
    /// `inline-start` — logical equivalent of `left`/`right` depending on
    /// writing direction (CSS Logical Properties §4). Maps to `left` in LTR.
    InlineStart,
    /// `inline-end` — logical equivalent of `left`/`right` depending on
    /// writing direction (CSS Logical Properties §4). Maps to `right` in LTR.
    InlineEnd,
}

css_keywords!(ClearValue {
    None => "none",
    Left => "left",
    Right => "right",
    Both => "both",
    InlineStart => "inline-start",
    InlineEnd => "inline-end",
});

/// When `float` is not `none`, resolve the computed-value transformation of
/// `display` required by CSS2 §9.7 "Relationships between
/// 'display', 'position', and 'float'"
/// <https://www.w3.org/TR/CSS2/visuren.html#dis-pos-flo>. Spec verbatim:
///
/// > Otherwise, if 'float' has a value other than 'none', the box is
/// > floated and 'display' is set according to the table below.
///
/// The table in the same section (verbatim):
///
/// | Specified value | Computed value |
/// |---|---|
/// | `inline-table` | `table` |
/// | `inline`, `table-row-group`, `table-column`, `table-column-group`, `table-header-group`, `table-footer-group`, `table-row`, `table-cell`, `table-caption`, `inline-block` | `block` |
/// | others | same as specified |
///
/// This crate's [`DisplayValue`] scope has 18 variants, including table types
/// (`block` / `inline` / `inline-block` / `none` / `flex` / `grid`
/// / `list-item` / `contents` / `table` / `inline-table` / `table-row-group`
/// / `table-header-group` / `table-footer-group` / `table-row`
/// / `table-column-group` / `table-column` / `table-cell` / `table-caption`).
/// Under the CSS2 §9.7 table:
/// - `inline-table` → `table` (first row)
/// - `inline`, `table-row-group`, `table-column`, `table-column-group`,
///   `table-header-group`, `table-footer-group`, `table-row`, `table-cell`,
///   `table-caption`, `inline-block` → `block` (second row)
/// - `block`, `table`, `flex`, `grid`, `list-item`, `none`, `contents` fall under
///   "others" (same as specified), even when floated. The treatment of
///   `flex`/`grid`/`list-item` as "others" is unchanged.
///
/// # `display: none` is handled by a separate branch before this function is called
///
/// §9.7 opens with: "If 'display' has the value 'none', then
/// 'position' and 'float' do not apply." This branch precedes the table
/// and does not pass through it. Therefore, guard [`DisplayValue::None`]
/// with an explicit early return rather than rely on the general rule for
/// variants absent from the table ("same as specified"). That `None`
/// happens to produce the same result under the general rule is coincidental;
/// the distinction could matter under future fail-safe extensions like
/// [`resolve_overflow`].
///
/// # `display: contents` is also exempt from forced transformation (on separate spec grounds)
///
/// CSS2 §9.7's table does not cover `contents`, a keyword absent from CSS2.
/// Instead, CSS Display Module Level 3 §2.7 "Automatic
/// Box Type Transformations"
/// <https://www.w3.org/TR/css-display-3/#transformations> says verbatim of
/// blockification in general, including this table: "This has no effect on
/// display types that generate no box at all, such as `display: none` or
/// `display: contents`." A floated `contents` element generates no box,
/// so the float-induced transformation itself does not apply (the same
/// conclusion as [`DisplayValue::None`], but from a different spec passage).
///
/// # Cross-field dependency on the same node
///
/// Like [`resolve_overflow`], this couples two properties on the same node
/// (it depends only on the other property on that node, not on a parent's
/// value). It is also called at the same place in phase 3 (absolutization),
/// inside [`crate::specified::SpecifiedValues::finalize`]'s
/// `absolutize_with`.
///
/// # Not called for pages
///
/// An `@page` box is not an "element in the visual formatting context" as
/// contemplated by §9.7 (CSS has no mechanism to float a page box).
/// Thus, phase 3 of [`crate::page::cascade_page`] does not perform this
/// resolution: it treats `Float` / `Clear` as opaque pass-through values,
/// like [`ZIndexValue`] (see the corresponding `absolutize_in_page_context`
/// arm in [`crate::page`]).
///
/// `pub(crate)` — only the `specified` module calls this function.
pub(crate) fn resolve_display_for_float(display: DisplayValue, float: FloatValue) -> DisplayValue {
    if matches!(float, FloatValue::None) {
        return display;
    }
    match display {
        DisplayValue::None => DisplayValue::None,
        // As explained in the function docs under `display: contents`,
        // blockification does not apply to display types that generate no box
        // (CSS Display Module Level 3 §2.7 verbatim).
        DisplayValue::Contents => DisplayValue::Contents,
        // CSS2 §9.7, first table row: `inline-table` → `table`
        DisplayValue::InlineTable => DisplayValue::Table,
        // CSS2 §9.7, second table row: `inline`, `table-row-group`, `table-column`,
        // `table-column-group`, `table-header-group`, `table-footer-group`,
        // `table-row`, `table-cell`, `table-caption`, `inline-block` → `block`
        DisplayValue::Inline
        | DisplayValue::InlineBlock
        | DisplayValue::InlineFlex
        | DisplayValue::InlineGrid
        | DisplayValue::TableRowGroup
        | DisplayValue::TableColumn
        | DisplayValue::TableColumnGroup
        | DisplayValue::TableHeaderGroup
        | DisplayValue::TableFooterGroup
        | DisplayValue::TableRow
        | DisplayValue::TableCell
        | DisplayValue::TableCaption => DisplayValue::Block,
        // The remaining variants are "others" — same as specified. Explicitly list
        // `Block` / `Table` / `Flex` / `Grid` / `ListItem` / `FlowRoot` so
        // adding a future variant causes a non-exhaustive-match compile error
        // (`#[non_exhaustive]` applies outside this crate, not to this match
        // inside the defining crate).
        same @ (DisplayValue::Block
        | DisplayValue::Table
        | DisplayValue::Flex
        | DisplayValue::Grid
        | DisplayValue::ListItem
        | DisplayValue::FlowRoot) => same,
    }
}

/// The value of the `white-space` property.
///
/// CSS Text Module Level 3 §3 "White Space and Wrapping: the white-space
/// property" <https://www.w3.org/TR/css-text-3/#white-space-property>.
///
/// propdef (spec verbatim): Value: `normal | pre | nowrap | pre-wrap |
/// break-spaces | pre-line`, Initial: `normal`, Applies to: text, Inherited:
/// **yes**, Computed value: "specified keyword".
///
/// # Meaning of the five keywords (verified against the spec verbatim)
///
/// - [`Normal`](WhiteSpace::Normal) — "This value directs user agents to collapse
///   sequences of white space into a single character (or in some cases, no
///   character). Lines may wrap at allowed soft wrap opportunities." The spec
///   initial value.
/// - [`Pre`](WhiteSpace::Pre) — "This value prevents user agents from collapsing
///   sequences of white space. Segment breaks such as line feeds are
///   preserved as forced line breaks. Lines only break at forced line
///   breaks."
/// - [`Nowrap`](WhiteSpace::Nowrap) — "Like normal, this value collapses white
///   space; but like pre, it does not allow wrapping."
/// - [`PreWrap`](WhiteSpace::PreWrap) — "Like pre, this value preserves white
///   space; but like normal, it allows wrapping."
/// - [`PreLine`](WhiteSpace::PreLine) — "Like normal, this value collapses
///   consecutive white space characters and allows wrapping, but it
///   preserves segment breaks in the source as forced line breaks."
///
/// As the spec's informative summary table shows, the five keywords combine
/// two independent behaviors: (a) whether white space collapses
/// (`normal`/`nowrap`/`pre-line`) or is preserved (`pre`/`pre-wrap`), and
/// (b) whether lines wrap (`normal`/`pre-wrap`/`pre-line`) or do not wrap
/// (`pre`/`nowrap`).
///
/// # Scope carving
///
/// - **Non-goal**: the spec's sixth keyword, `break-spaces` — spec verbatim:
///   "The behavior is identical to that of pre-wrap, except that any sequence of
///   preserved white space always takes up space, including at the end of
///   the line." The distinction from `pre-wrap` is a used-value line-breaking
///   calculation: whether preserved end-of-line spaces actually occupy space.
///   This crate has no line boxes yet (the same carve-out as the deprecated
///   `break-word` non-goal in the [`WordBreak`] docs). Drop it silently as
///   `None`, like other unknown identifiers.
/// - **Downstream handoff**: The actual white-space collapsing and line-wrapping
///   algorithms (applying the two axes summarized by the spec's table) belong
///   to a text-layout/line-breaking consumer that this crate does not yet have
///   (in raikiri-dom / raikiri-paint). This property carries only the cascade's
///   static-side keyword, as in the "Downstream handoff" section of the
///   [`TextTransform`] docs.
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet (future
///   work), and are silently dropped. The "CSS-wide keywords" section of the
///   [`PropertyValue`] docs is authoritative for the five keywords and rationale.
/// - **(a) Spec-invalid**: Any identifier other than the five keywords above
///   (`break-spaces` is covered under Non-goal) is silently dropped as `None`.
///
/// This crate carries no lengths for this property, so computed value equals
/// specified keyword, with no relative resolution (as for [`Direction`]).
///
/// Following the [`Direction`] / [`WordBreak`] convention, do not derive
/// `Default`: [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] set [`WhiteSpace::Normal`]
/// directly.
/// `text-wrap-mode` value (CSS Text 4 §5.1), also used by the `text-wrap`
/// shorthand's wrapping component.
///
/// The `text-wrap` shorthand is expanded into separate mode and style values
/// before cascade. Inherited, initial `wrap`, computed value = specified
/// keyword. `nowrap` suppresses soft wrapping in the inline engine's line
/// breaking.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextWrapMode {
    /// `wrap` — spec initial value.
    Wrap,
    /// `nowrap`.
    Nowrap,
}

css_keywords!(TextWrapMode {
    Wrap => "wrap",
    Nowrap => "nowrap",
});

/// CSS Text 4 §5.4 `text-wrap-style` computed keyword (initial `auto`, inherited).
/// <https://drafts.csswg.org/css-text-4/#text-wrap-style>
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextWrapStyle {
    /// `auto` — spec initial value.
    Auto,
    /// `balance`.
    Balance,
    /// `pretty`.
    Pretty,
    /// `stable`.
    Stable,
}

css_keywords!(TextWrapStyle {
    Auto => "auto",
    Balance => "balance",
    Pretty => "pretty",
    Stable => "stable",
});

/// The normalized components of the CSS Text 4 `text-wrap` shorthand.
/// Omitted components are stored with their initial values before cascade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextWrapShorthand {
    /// `text-wrap-mode` component.
    pub mode: TextWrapMode,
    /// `text-wrap-style` component.
    pub style: TextWrapStyle,
}

/// `white-space` property value (CSS Text 3 §4).
///
/// All six keyword values are preserved through parsing and cascade. The
/// downstream text shaper collapses source whitespace for `normal`, `nowrap`,
/// and `pre-line`; `pre`, `pre-wrap`, and `break-spaces` preserve it, with
/// `break-spaces` end-of-line occupancy still owned by line breaking.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhiteSpace {
    /// `normal` — spec initial value.
    Normal,
    /// `pre`.
    Pre,
    /// `nowrap`.
    Nowrap,
    /// `pre-wrap`.
    PreWrap,
    /// `pre-line`.
    PreLine,
    /// `break-spaces`.
    BreakSpaces,
}

css_keywords!(WhiteSpace {
    Normal => "normal",
    Pre => "pre",
    Nowrap => "nowrap",
    PreWrap => "pre-wrap",
    PreLine => "pre-line",
    BreakSpaces => "break-spaces",
});

impl WhiteSpace {
    /// The `white-space-collapse` and `text-wrap-mode` values this legacy
    /// keyword stands for (CSS Text 4 §3, the `white-space` shorthand
    /// mapping table), or `None` for a keyword added later.
    pub fn collapse_and_wrap(self) -> Option<(WhiteSpaceCollapse, TextWrapMode)> {
        match self {
            WhiteSpace::Normal => Some((WhiteSpaceCollapse::Collapse, TextWrapMode::Wrap)),
            WhiteSpace::Pre => Some((WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap)),
            WhiteSpace::Nowrap => Some((WhiteSpaceCollapse::Collapse, TextWrapMode::Nowrap)),
            WhiteSpace::PreWrap => Some((WhiteSpaceCollapse::Preserve, TextWrapMode::Wrap)),
            WhiteSpace::PreLine => Some((WhiteSpaceCollapse::PreserveBreaks, TextWrapMode::Wrap)),
            WhiteSpace::BreakSpaces => Some((WhiteSpaceCollapse::BreakSpaces, TextWrapMode::Wrap)),
        }
    }
}

/// `white-space-collapse` property value.
///
/// CSS Text Module Level 4 property definition:
/// <https://www.w3.org/TR/css-text-4/#propdef-white-space-collapse>.
/// The six values are `collapse | discard | preserve | preserve-breaks |
/// preserve-spaces | break-spaces`; initial value is `collapse`, the property
/// is inherited, and its computed value is the specified keyword.
///
/// This type carries the value through style computation only. It does not
/// perform whitespace transformation or line layout.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhiteSpaceCollapse {
    /// `collapse` — spec initial value.
    Collapse,
    /// `discard`.
    Discard,
    /// `preserve`.
    Preserve,
    /// `preserve-breaks`.
    PreserveBreaks,
    /// `preserve-spaces`.
    PreserveSpaces,
    /// `break-spaces`.
    BreakSpaces,
}

css_keywords!(WhiteSpaceCollapse {
    Collapse => "collapse",
    Discard => "discard",
    Preserve => "preserve",
    PreserveBreaks => "preserve-breaks",
    PreserveSpaces => "preserve-spaces",
    BreakSpaces => "break-spaces",
});

/// The value of the `hyphens` property.
///
/// CSS Text Module Level 3 §5.3 "Hyphenation: the hyphens property"
/// <https://www.w3.org/TR/css-text-3/#hyphens-property>.
///
/// propdef (spec verbatim): Value: `none | manual | auto`, Initial: `manual`,
/// Applies to: text, Inherited: **yes**, Computed value: specified keyword.
///
/// # Meaning of the three keywords (verified against the spec verbatim)
///
/// - [`None`](Self::None) — "Words are not hyphenated, even if characters
///   inside the word explicitly define hyphenation opportunities."
/// - [`Manual`](Self::Manual) — "Words are only hyphenated where there are
///   characters inside the word that explicitly suggest hyphenation
///   opportunities." The spec initial value. A common explicit hyphenation
///   opportunity is the soft hyphen (`U+00AD`, `&shy;` in HTML); the same
///   section says "In Unicode, U+00AD is a conditional 'soft hyphen'".
/// - [`Auto`](Self::Auto) — "Words may be broken at hyphenation
///   opportunities determined automatically by a language-appropriate
///   hyphenation resource in addition to those indicated explicitly by a
///   conditional hyphen."
///
/// # Scope carving
///
/// - **Non-goal**: The `auto` behavior "determined automatically by a
///   language-appropriate hyphenation resource" (dictionary-based automatic
///   hyphenation) is outside this crate's scope. This crate has neither content
///   language detection nor language-specific hyphenation resources. The spec
///   clearly distinguishes `auto` and `manual`, and this type represents both
///   as distinct variants. This follows the decision for `BreakWord`/`Anywhere`
///   in the "Scope carving" section of the [`OverflowWrap`] docs: preserve the
///   variants for a future layout/hyphenation consumer even though their
///   difference cannot be observed at this crate's layer.
/// - **(b) Unsupported**: CSS-wide keywords are not implemented yet (future
///   work), and are silently dropped. The "CSS-wide keywords" section of the
///   [`PropertyValue`] docs is authoritative for the five keywords and rationale.
/// - **(a) Spec-invalid**: Any identifier other than the three keywords above
///   is silently dropped as `None`.
///
/// # Downstream handoff
///
/// Computing the actual positions of hyphenation opportunities within words
/// (scanning for soft hyphens, dictionary lookup for `auto`) is outside this
/// crate's scope. This property carries only the cascade's static-side
/// keyword, which a text-shaping/paint consumer reads (as in the "Downstream
/// handoff" section of the [`TextTransform`] docs). **A downstream consumer
/// without dictionary-based automatic hyphenation can safely handle
/// [`Auto`](Self::Auto) only by breaking at soft hyphens (`U+00AD`), exactly
/// as for [`Manual`](Self::Manual).** Document this choice explicitly in the
/// downstream consumer rather than silently omitting the specified behavior.
/// The computed value retains the spec's three distinct keywords, for CSSOM
/// round-tripping and for future dictionary resources that can distinguish
/// `Auto` from `Manual` again.
///
/// This crate carries no lengths for this property, so computed value equals
/// specified keyword, with no relative resolution (as for [`Direction`]).
///
/// Following the [`Direction`] / [`WordBreak`] convention, do not derive
/// `Default`: [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] set [`Hyphens::Manual`]
/// directly.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hyphens {
    /// `none`.
    None,
    /// `manual` — spec initial value.
    Manual,
    /// `auto`. In this crate's scope, downstream consumers are expected to handle
    /// it by breaking only at soft hyphens, as for [`Manual`](Self::Manual).
    /// See the type docs' "Downstream handoff" section for details.
    Auto,
}

css_keywords!(Hyphens {
    None => "none",
    Manual => "manual",
    Auto => "auto",
});

/// The specified/computed value of CSS Text 4 `hyphenate-character`.
///
/// Grammar: `auto | <string>`. The property is inherited and initially `auto`.
/// A string is stored as decoded Unicode text; choosing or inserting the string
/// at a hyphenation opportunity remains a text-layout consumer responsibility.
/// <https://drafts.csswg.org/css-text-4/#propdef-hyphenate-character>
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HyphenateCharacter {
    /// The UA-selected character for hyphenation opportunities.
    Auto,
    /// An explicit CSS string, decoded by the CSS parser.
    String(SmolStr),
}

/// One component of CSS Text 4 `hyphenate-limit-chars`.
///
/// A direct `<integer>` token is stored as a non-negative integer. A math
/// expression that computes to a number is rounded to the nearest integer
/// when an integer is required; exact half values round toward positive
/// infinity, per CSS Values 4. This value model does not implement
/// hyphenation or text breaking.
/// <https://drafts.csswg.org/css-text-4/#propdef-hyphenate-limit-chars>
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HyphenateLimitCharsValue {
    /// UA-selected limit.
    Auto,
    /// A computed, non-negative integer.
    Integer(u32),
}

/// The expanded computed value of CSS Text 4 `hyphenate-limit-chars`.
///
/// The property is inherited and initially `auto`. It computes to three
/// components: an omitted second component becomes `auto`, and an omitted
/// third component copies the second component. The style layer only carries
/// these values; line breaking and hyphenation remain consumer behavior.
/// <https://drafts.csswg.org/css-text-4/#propdef-hyphenate-limit-chars>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HyphenateLimitChars {
    /// Maximum hyphenated word length.
    pub total: HyphenateLimitCharsValue,
    /// Minimum characters before a hyphenation point.
    pub before: HyphenateLimitCharsValue,
    /// Minimum characters after a hyphenation point.
    pub after: HyphenateLimitCharsValue,
}

impl HyphenateLimitChars {
    /// CSS initial value: all three computed components are `auto`.
    pub const INITIAL: Self = Self {
        total: HyphenateLimitCharsValue::Auto,
        before: HyphenateLimitCharsValue::Auto,
        after: HyphenateLimitCharsValue::Auto,
    };
}

/// The value of the `tab-size` property.
///
/// CSS Text Module Level 3 §4.2 "Tab Character Size: the tab-size property"
/// (<https://www.w3.org/TR/css-text-3/#tab-size-property>), value grammar
/// `<number [0,∞]> | <length [0,∞]>`. Initial: `8`. Inherited: yes.
/// Percentages: N/A.
///
/// This has the number-vs-length shape of [`LineHeight`], but only two
/// grammar branches: there is no `normal` keyword.
///
/// - [`Number`](Self::Number) — `<number [0,∞]>`. The spec says "A `<number>`
///   represents the measure as a multiple of the advance width of the space
///   character (U+0020) of the nearest block container ancestor of the
///   preserved tab, including its associated letter-spacing and
///   word-spacing." Resolving this font metric belongs to a text-layout
///   consumer (raikiri-dom / raikiri-paint) that this crate does not yet have.
///   This crate passes through the unitless multiplier (as for
///   [`LineHeight::Number`]).
/// - [`Length`](Self::Length) — `<length [0,∞]>`. Unlike
///   [`LineHeight::Length`] (`<length-percentage>`), this has no percentage
///   form: the spec propdef says "Percentages: N/A".
/// - [`Calc`](Self::Calc) — the `calc()` form of the `<length>` alternative
///   (e.g. `calc(10px + 0.5em)`). Reject percentage terms at parse time
///   ("Percentages: N/A"); unsupported `calc()` forms containing `sign()`
///   or container-relative units (e.g. `cqw`) are dropped.
///
/// # Non-negative constraint
///
/// The spec's `[0,∞]` grammar applies to both branches. The spec says "Negative values are not
/// allowed."
/// Reject negative values in the parser (`parse_tab_size` post-filter:
/// spec-invalid → drop).
///
/// # CR status of the `<length>` alternative
///
/// The spec marks the `<length>` alternative "at risk" (it could be dropped
/// during the Candidate Recommendation process). This implementation follows
/// the current TR grammar; revisit it if the alternative is actually dropped.
///
/// Downstream matches must have a wildcard arm (`#[non_exhaustive]` is the
/// forward-compatibility contract that adding a variant must not break existing
/// pattern matches, as with sibling [`LineHeight`] / [`Length`]).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TabSize {
    /// `<number [0,∞]>` — multiplier of the advance width of the space character
    /// (U+0020). Remains a number at the computed-value layer; resolving the
    /// font metric belongs to the downstream consumer (see the type docs).
    Number(f32),
    /// `<length [0,∞]>` — absolute tab size. No percentages are allowed
    /// (see "Percentages: N/A" in the type docs).
    Length(Length),
    /// Deferred length from additive `calc()` — retains the same
    /// [`LengthPercentageCalc`] representation (`px` + `em`) as
    /// [`LetterSpacingValue::Calc`]. [`crate::resolve::resolve_tab_size`]
    /// resolves it using this element's computed font size. Percentage terms
    /// cannot reach this point: `parse_tab_size` already rejects them
    /// (spec propdef: "Percentages: N/A").
    Calc(LengthPercentageCalc),
}

/// The value of the `line-break` property (CSS Text 3 §5.2).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineBreak {
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}

css_keywords!(LineBreak {
    Auto => "auto",
    Loose => "loose",
    Normal => "normal",
    Strict => "strict",
    Anywhere => "anywhere",
});

/// The value of the `text-justify` property (CSS Text 3 §6.2).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextJustify {
    Auto,
    None,
    InterWord,
    InterCharacter,
    /// Legacy `distribute` (an older CSS Text 3 §6.2 value treated as an alias
    /// for `inter-character`, used by WPT text-justify-distribute-001).
    /// The inline engine lays it out as `inter-character`.
    Distribute,
}

css_keywords!(TextJustify {
    Auto => "auto",
    None => "none",
    InterWord => "inter-word",
    InterCharacter => "inter-character",
    Distribute => "distribute",
});

/// CSS Text 4 `word-space-transform` computed value.
///
/// The inline text layout consumes `space` and `ideographic-space` for
/// interior U+200B and in-flow inline `<wbr>` in the single-line spacing
/// subset. Other values are data-only.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WordSpaceTransform {
    /// `none` — the initial value.
    None,
    /// `space`.
    Space,
    /// `ideographic-space`.
    IdeographicSpace,
    /// `space auto-phrase`.
    SpaceAutoPhrase,
    /// `ideographic-space auto-phrase`.
    IdeographicSpaceAutoPhrase,
}

css_keywords!(@serialize WordSpaceTransform {
    None => "none",
    Space => "space",
    IdeographicSpace => "ideographic-space",
    SpaceAutoPhrase => "space auto-phrase",
    IdeographicSpaceAutoPhrase => "ideographic-space auto-phrase",
});

/// `text-autospace` property value (CSS Text Module Level 4).
///
/// The keyword forms are kept distinct because `auto` and `normal` are
/// distinct computed values, even though this layout slice currently uses
/// `normal` as the only automatic-spacing mode.  The long form stores the
/// three independent boundary classes and the optional `insert`/`replace`
/// behavior from the current grammar.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAutospace {
    /// The initial value.  Automatic spacing is enabled for the default
    /// ideograph/letter and ideograph/number boundaries.
    Normal,
    /// The legacy `auto` keyword, preserved as a computed keyword.
    Auto,
    /// Disable automatic spacing.
    NoAutospace,
    /// An explicit set of boundary classes.
    Custom {
        ideograph_alpha: bool,
        ideograph_numeric: bool,
        punctuation: bool,
        mode: TextAutospaceMode,
    },
}

/// Optional behavior modifier of an explicit [`TextAutospace`] value.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAutospaceMode {
    /// No explicit modifier was specified.
    None,
    /// Insert an inter-character space at matching boundaries.
    Insert,
    /// Replace an existing separator at matching boundaries.
    Replace,
}

/// `text-spacing-trim` property values from CSS Text 4.
/// <https://drafts.csswg.org/css-text-4/#text-spacing-trim-property>
///
/// The specified keyword is retained as the computed value. This is data-only:
/// it does not enable text-spacing or punctuation-trimming behavior in layout.
/// The property is inherited and its initial value is `normal`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextSpacingTrim {
    /// `auto`.
    Auto,
    /// `normal` — initial value.
    Normal,
    /// `space-all`.
    SpaceAll,
    /// `trim-both`.
    TrimBoth,
    /// `trim-all`.
    TrimAll,
    /// `trim-start`.
    TrimStart,
    /// `space-first`.
    SpaceFirst,
}

css_keywords!(TextSpacingTrim {
    Auto => "auto",
    Normal => "normal",
    SpaceAll => "space-all",
    TrimBoth => "trim-both",
    TrimAll => "trim-all",
    TrimStart => "trim-start",
    SpaceFirst => "space-first",
});

/// The normalized longhand values of the CSS Text 4 `text-spacing` shorthand.
///
/// The shorthand expands to [`TextSpacingTrim`] and [`TextAutospace`]. Omitted
/// components use their longhand initial value before cascade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSpacingShorthand {
    /// `text-spacing-trim` component.
    pub trim: TextSpacingTrim,
    /// `text-autospace` component.
    pub autospace: TextAutospace,
}

/// The value of the `text-align-all` property (CSS Text 3 §6.1 longhand).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlignAll {
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
    MatchParent,
}

css_keywords!(TextAlignAll {
    Start => "start",
    End => "end",
    Left => "left",
    Right => "right",
    Center => "center",
    Justify => "justify",
    MatchParent => "match-parent",
});

/// The value of the `text-align-last` property (CSS Text 3 §6.1 longhand).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlignLast {
    Auto,
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
    MatchParent,
}

css_keywords!(TextAlignLast {
    Auto => "auto",
    Start => "start",
    End => "end",
    Left => "left",
    Right => "right",
    Center => "center",
    Justify => "justify",
    MatchParent => "match-parent",
});

/// The value of the `text-combine-upright` property (CSS Writing Modes 3 §9.1).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCombineUpright {
    None,
    All,
}

css_keywords!(TextCombineUpright {
    None => "none",
    All => "all",
});

/// The value of the `text-orientation` property (CSS Writing Modes 3 §5.1).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOrientation {
    Mixed,
    Upright,
    Sideways,
}

css_keywords!(TextOrientation {
    Mixed => "mixed",
    Upright => "upright",
    Sideways => "sideways",
});

/// The value of the `unicode-bidi` property (CSS Writing Modes 3 §2.2).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnicodeBidi {
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}

css_keywords!(UnicodeBidi {
    Normal => "normal",
    Embed => "embed",
    Isolate => "isolate",
    BidiOverride => "bidi-override",
    IsolateOverride => "isolate-override",
    Plaintext => "plaintext",
});

/// The value of the `table-layout` property.
///
/// CSS Tables 3 §4 "Table Layout Algorithm"
/// <https://www.w3.org/TR/css-tables-3/#table-layout-property>
/// (formerly CSS 2.1 §17.5.2 "Table width algorithms: the 'table-layout'
/// property" <https://www.w3.org/TR/CSS2/tables.html#width-layout>).
/// Value: `auto | fixed`; Initial: `auto`; Applies to: `table` /
/// `inline-table`; Inherited: **no**; Computed value: "as specified".
///
/// - `auto` — automatic table layout (content-driven column sizing,
///   CSS Tables 3 §5).
/// - `fixed` — fixed table layout (table width and widths specified on
///   the first row / `col` drive column sizing; content may overflow;
///   the fixed branch of CSS Tables 3 §5).
///
/// Column sizing at layout time is handled by raikiri-dom
/// (see [`crate::computed::ComputedValues::table_layout`]); this crate only
/// carries the cascaded keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableLayoutValue {
    /// `auto` — spec initial value (automatic table layout).
    Auto,
    /// `fixed` — fixed table layout.
    Fixed,
}

css_keywords!(TableLayoutValue {
    Auto => "auto",
    Fixed => "fixed",
});

/// The value of the `text-overflow` property.
///
/// CSS Overflow 3 §5.1 "Inline Overflow Ellipsis: the text-overflow property"
/// <https://www.w3.org/TR/css-overflow-3/#text-overflow>.
/// Value: `clip | ellipsis`; Initial: `clip`; Applies to: block containers;
/// Inherited: **no**; Computed value: "as specified".
///
/// Only the one-value keyword form is accepted. The `<string>` and two-value
/// forms (CSS Overflow 4) are rejected at parse time, so such a declaration
/// does not apply.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOverflowValue {
    /// `clip` — spec initial value: overflowing inline content is clipped.
    Clip,
    /// `ellipsis` — overflowing inline content is replaced by an ellipsis.
    Ellipsis,
}

css_keywords!(TextOverflowValue {
    Clip => "clip",
    Ellipsis => "ellipsis",
});

/// The value of the `border-collapse` property.
///
/// CSS Tables 3 §6 "Borders"
/// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>
/// (formerly CSS 2.1 §17.6 "Borders"
/// <https://www.w3.org/TR/CSS2/tables.html#borders>).
/// Value: `collapse | separate`; Initial: `separate`; Applies to: `table` /
/// `inline-table`; Inherited: **yes**; Computed value: "as specified".
///
/// - `separate` — separated borders model (with cell spacing).
/// - `collapse` — collapsing borders model (adjacent borders collapse into
///   one border through conflict resolution).
///
/// Conflict resolution itself is handled by raikiri-dom
/// (see [`crate::computed::ComputedValues::border_collapse`]); this crate only
/// carries the cascaded keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderCollapseValue {
    /// `separate` — spec initial value (separated borders model).
    Separate,
    /// `collapse` — collapsing borders model.
    Collapse,
}

css_keywords!(BorderCollapseValue {
    Separate => "separate",
    Collapse => "collapse",
});

/// The value of the `caption-side` property.
///
/// CSS Tables 3 §7 "Caption Position: the caption-side property"
/// <https://www.w3.org/TR/css-tables-3/#caption-side-property>
/// (formerly CSS 2.1 §17.4 "Tables in the visual formatting model ...
/// Caption position and alignment"
/// <https://www.w3.org/TR/CSS2/tables.html#caption-position>).
/// Value: `top | bottom`; Initial: `top`; Applies to: `table-caption`;
/// Inherited: **yes**; Computed value: "as specified".
///
/// - `top` — places the caption box above the table box (block-start side).
/// - `bottom` — places the caption box below the table box (block-end side).
///
/// Positioning the caption box itself is handled by raikiri-dom
/// (see [`crate::computed::ComputedValues::caption_side`]); this crate only
/// carries the cascaded keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionSideValue {
    /// `top` — spec initial value.
    Top,
    /// `bottom` — caption below the table box.
    Bottom,
}

css_keywords!(CaptionSideValue {
    Top => "top",
    Bottom => "bottom",
});

/// The value of the `empty-cells` property.
///
/// CSS Tables 3 §8 "Empty Cells: the empty-cells property"
/// <https://www.w3.org/TR/css-tables-3/#empty-cells-property>
/// (formerly CSS 2.1 §17.5.1 "Table layers and transparency"
/// <https://www.w3.org/TR/CSS2/tables.html#empty-cells>).
/// Value: `show | hide`; Initial: `show`; Inherited: **yes**;
/// Computed value: "as specified".
///
/// - `show` — paints an empty cell's border and background (effective only in
///   the separated borders model).
/// - `hide` — does not paint an empty cell's border or background.
///
/// The decision to paint cell backgrounds and borders belongs to
/// raikiri-dom / raikiri-paint
/// (see [`crate::computed::ComputedValues::empty_cells`]); this crate only
/// carries the cascaded keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyCellsValue {
    /// `show` — spec initial value.
    Show,
    /// `hide` — hides an empty cell's border and background.
    Hide,
}

css_keywords!(EmptyCellsValue {
    Show => "show",
    Hide => "hide",
});

/// The specified value of `border-spacing`.
///
/// CSS Tables 3 §6.1 "Separated borders: the border-spacing property"
/// <https://www.w3.org/TR/css-tables-3/#border-spacing-property>
/// (formerly CSS 2.1 §17.6.1 "The separated borders model"
/// <https://www.w3.org/TR/CSS2/tables.html#separated-borders>).
/// Value grammar: `<length>{1,2}`; Initial: `0`; Applies to: `table` /
/// `inline-table`; Inherited: **yes**; Computed value:
/// "two absolute lengths"; Percentages: N/A; "Negative lengths are illegal"
/// (the spec rejects them during parsing; for negative computed calc results,
/// [`crate::resolve::resolve_border_spacing`] clamps to `0`).
///
/// If the second component is omitted, copy the first component (per the spec:
/// "If only one value is specified, it applies to both the horizontal and
/// vertical spacing"). This is the same single-value-doubles form as [`GapShorthand`].
/// The two components are ordered horizontal, then vertical.
///
/// Because the computed value is "two absolute lengths", phase 3
/// absolutizes each component with [`crate::resolve::resolve_border_spacing`];
/// unlike its sibling keyword-only types [`CaptionSideValue`] / [`EmptyCellsValue`],
/// the values are not simply passed through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderSpacingValue {
    /// Horizontal (inline-axis) spacing — the first component.
    pub horizontal: Length,
    /// Vertical (block-axis) spacing — the second component, or `horizontal` if omitted.
    pub vertical: Length,
}

/// The `<color>` component of a `text-shadow` entry: like [`BorderColor`] /
/// [`TextDecorationColor`], it distinguishes the `currentcolor` keyword from resolved
/// `<color>` values.
///
/// CSS Text Decoration Module Level 3 §4 "Text Shadows: the text-shadow
/// property" <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>:
/// "Values are interpreted as for box-shadow." In the `<shadow>` grammar
/// (CSS Backgrounds 3 §6.1 "Drop Shadows: the box-shadow property"
/// <https://www.w3.org/TR/css-backgrounds-3/#box-shadow>),
/// an omitted `<color>` component has the used value `currentcolor`
/// (CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>).
/// Used-value resolution (currentcolor → the same node's computed `color`
/// property) belongs to paint; the rationale matches the [`BorderColor`] docs on why
/// the static side of the cascade keeps the value in an enum.
///
/// `#[non_exhaustive]` follows the pattern of [`BorderColor`] / [`TextDecorationColor`]:
/// it allows future variants (for example, CSS Color 4 §6.2 system-color
/// keywords) to be added without breaking users.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextShadowColor {
    /// `currentcolor` keyword — the specification requires this when `<color>` is omitted
    /// (inherited from box-shadow; see the docs above).
    CurrentColor,
    /// Resolved `<color>` value — the payload when the author explicitly specifies a hex, named, or `rgb(a)`
    /// color, or `transparent`.
    Resolved(CssColor),
}

/// One `text-shadow` entry (one item in a comma-separated list).
///
/// CSS Text Decoration Module Level 3 §4
/// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property> —
/// "Values are interpreted as for box-shadow. (But note that spread values
/// and the inset keyword are not allowed.)" The box-shadow `<shadow>` syntax
/// (CSS Backgrounds 3 §6.1 "Drop Shadows: the box-shadow property") is
/// narrowed by excluding `inset` and the fourth length (spread-distance);
/// its grammar is `<color>? && <length>{2,3}`
/// (offset-x, offset-y, and an optional blur-radius).
///
/// # Filling in initial values for omitted components
///
/// - Omitted `blur_radius` → `Length::Px(0.0)` — the specification's computed-value definition
///   ("a list, each item consisting of three absolute lengths plus a
///   computed color") always requires blur-radius as the third length.
///   Thus, following the precedent in [`parse_border_shorthand`] of filling omitted
///   components with their initial values, the parser eagerly fills in the radius without `Option<Length>`
///   surviving into the specified-value layer.
/// - Omitted `color` → [`TextShadowColor::CurrentColor`] — analogous to the [`BorderColor`]
///   `color.unwrap_or(BorderColor::CurrentColor)` precedent
///   (see [`parse_border_shorthand`]).
///
/// # Non-negative blur-radius
///
/// The blur-radius (third length) must be non-negative. CSS Backgrounds 3 §6.1
/// "Drop Shadows: the box-shadow property" defines the shared `<shadow>`
/// syntax for box-shadow and text-shadow and says "Negative values are invalid"
/// for blur-radius and spread distance. This restriction does not apply to
/// (the first two lengths); negative offsets are allowed, as with box-shadow.
/// Literal negative lengths are rejected during parsing; negative computed calc results are clamped to `0px`.
/// See [`crate::property::parse`] for the text-shadow parser path.
///
/// # `#[non_exhaustive]`
///
/// This allows non-breaking addition of future fields (for example, `spread`/`inset`-like
/// fields shared with a future box-shadow), following the pattern of sibling [`Border`].
/// A `<length>` used by `text-shadow`, with a mixed `calc()` kept until
/// computed font-size resolution. The parser rejects percentages and retains
/// only absolute-pixel and `em` terms.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextShadowLength {
    /// A plain authored length.
    Length(Length),
    /// A `calc()` with an absolute-pixel component and an `em` coefficient.
    Calc {
        /// Absolute-pixel component.
        px: f32,
        /// `em` component resolved against the element's computed font size.
        em: f32,
    },
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadowItem {
    /// `offset-x` — `<length>` (no percentages; CSS Text Decoration Module
    /// Level 3 §4 "Percentages: N/A"). Negative values are allowed.
    pub offset_x: TextShadowLength,
    /// `offset-y` — the same grammar as [`Self::offset_x`].
    pub offset_y: TextShadowLength,
    /// `blur-radius` — `<length [0,∞]>`; when omitted, defaults to
    /// [`TextShadowLength::Length`] with `Length::Px(0.0)` (see the docs
    /// above). A negative calculated result resolves to `0px`; a literal
    /// negative length remains invalid.
    pub blur_radius: TextShadowLength,
    /// `<color>` component — defaults to [`TextShadowColor::CurrentColor`]
    /// when omitted (see the docs above).
    pub color: TextShadowColor,
}

/// A CSS Custom Properties Level 1 declaration retained as raw tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomProperty {
    pub(crate) name: SmolStr,
    pub(crate) value: SmolStr,
}

/// A known property whose value must wait for computed-value substitution.
#[derive(Clone, Debug, PartialEq)]
pub struct DeferredValue {
    pub(crate) property: SmolStr,
    pub(crate) value: SmolStr,
    pub(crate) key: PropertyKey,
}

/// A validated element color that retains `currentcolor` until its color basis is known.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextualColor {
    pub(crate) source: SmolStr,
    pub(crate) key: PropertyKey,
}

impl DeferredValue {
    /// Return a canonical defaulting marker for the supported deferred longhands.
    pub(crate) fn css_wide_keyword(&self) -> Option<CssWideKeyword> {
        if !matches!(
            self.key,
            PropertyKey::Color
                | PropertyKey::Display
                | PropertyKey::Opacity
                | PropertyKey::Visibility
                | PropertyKey::FontFamily
                | PropertyKey::FontWeight
                | PropertyKey::FontStyle
                | PropertyKey::BackgroundColor
                | PropertyKey::FontSize
                | PropertyKey::VerticalAlign
                | PropertyKey::BorderRadiusTopLeft
                | PropertyKey::BorderRadiusTopRight
                | PropertyKey::BorderRadiusBottomRight
                | PropertyKey::BorderRadiusBottomLeft
                | PropertyKey::ListStyleType
                | PropertyKey::ListStylePosition
                | PropertyKey::ListStyleImage
                | PropertyKey::ColumnRule
                | PropertyKey::ColumnRuleWidth
                | PropertyKey::ColumnRuleStyle
                | PropertyKey::ColumnRuleColor
                | PropertyKey::ColumnSpan
        ) {
            return None;
        }
        CssWideKeyword::from_css_ident(self.value.as_ref())
    }
}

/// Horizontal and vertical radii of one physical corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerRadius<T> {
    /// Radius along the horizontal edge.
    pub horizontal: T,
    /// Radius along the vertical edge.
    pub vertical: T,
}

impl<T> CornerRadius<T> {
    /// Build an elliptical corner from independent axes.
    pub const fn new(horizontal: T, vertical: T) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }

    /// Convert both axes without dropping their independent values.
    pub fn map<U>(self, mut convert: impl FnMut(T) -> U) -> CornerRadius<U> {
        CornerRadius::new(convert(self.horizontal), convert(self.vertical))
    }
}

impl<T: Copy> CornerRadius<T> {
    /// Use the same value on both axes.
    pub const fn circular(value: T) -> Self {
        Self::new(value, value)
    }
}

impl<T: Copy> From<T> for CornerRadius<T> {
    fn from(value: T) -> Self {
        Self::circular(value)
    }
}

/// Four physical corners of a `border-radius` shorthand.
///
/// This is the shorthand from CSS Backgrounds and Borders Level 3 §4.1
/// <https://www.w3.org/TR/css-backgrounds-3/#border-radius> expanded to
/// the four corners at parse time. Corners are ordered top-left, top-right,
/// bottom-right, bottom-left (clockwise). Both axes retain their independent
/// `<length-percentage>` values. Omitted vertical values copy the horizontal
/// values; one-to-four expansion applies separately to each axis.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderRadius {
    /// top-left corner radius.
    pub top_left: CornerRadius<Length>,
    /// top-right corner radius.
    pub top_right: CornerRadius<Length>,
    /// bottom-right corner radius.
    pub bottom_right: CornerRadius<Length>,
    /// bottom-left corner radius.
    pub bottom_left: CornerRadius<Length>,
}

impl BorderRadius {
    /// Expand four horizontal and four vertical radii in clockwise order.
    pub const fn elliptical(horizontal: [Length; 4], vertical: [Length; 4]) -> Self {
        Self {
            top_left: CornerRadius::new(horizontal[0], vertical[0]),
            top_right: CornerRadius::new(horizontal[1], vertical[1]),
            bottom_right: CornerRadius::new(horizontal[2], vertical[2]),
            bottom_left: CornerRadius::new(horizontal[3], vertical[3]),
        }
    }

    /// Expand four scalar corner values, using each value on both axes.
    pub const fn corners(
        top_left: Length,
        top_right: Length,
        bottom_right: Length,
        bottom_left: Length,
    ) -> Self {
        Self::elliptical(
            [top_left, top_right, bottom_right, bottom_left],
            [top_left, top_right, bottom_right, bottom_left],
        )
    }
}

/// One entry in the comma-separated `box-shadow` list.
///
/// Retains the offsets, optional blur, optional spread, optional color, and
/// optional `inset` of CSS Backgrounds and Borders Level 3 §6.1
/// <https://www.w3.org/TR/css-backgrounds-3/#box-shadow>. Offsets and spread
/// may be negative; blur must be non-negative.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadowItem {
    /// horizontal offset.
    pub offset_x: Length,
    /// vertical offset.
    pub offset_y: Length,
    /// Blur radius. Defaults to `0px` when omitted.
    pub blur_radius: Length,
    /// Spread distance. Defaults to `0px` when omitted.
    pub spread_radius: Length,
    /// Color. Defaults to `currentcolor` when omitted.
    pub color: TextShadowColor,
    /// Whether the shadow is painted inside the border box (`inset`).
    pub inset: bool,
}

/// The keyword/color payload of `outline-color`.
///
/// Retains `invert | <color>` from CSS Basic User Interface Module Level 3 §4.4
/// <https://www.w3.org/TR/css-ui-3/#outline-color>. Retain `currentcolor`,
/// which is included in `<color>`, separately from a resolved color on the
/// cascade's static side. `invert` applies only to outlines; do not add it
/// to the border's [`BorderColor`].
///
/// Do not derive `Default`. [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] explicitly set the spec initial
/// value (`invert`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineColor {
    /// `invert` — the CSS UI 3 §4.4 spec initial value.
    Invert,
    /// `currentcolor` keyword. Used-value resolution belongs to painting.
    CurrentColor,
    /// Resolved `<color>` value.
    Resolved(CssColor),
}

/// The specified value of the `outline` shorthand.
///
/// Retains width/style/color in any order, as defined by CSS Basic User
/// Interface Module Level 3 §4
/// <https://www.w3.org/TR/css-ui-3/#outline-props>. Unlike a border, an
/// outline does not affect the box model's dimensions.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    /// Outline width. Defaults to `medium` when omitted (CSS UI 3 §4.2).
    pub width: Length,
    /// Outline style. Defaults to [`OutlineStyle::None`] when omitted (CSS UI 3 §4.3).
    pub style: OutlineStyle,
    /// Outline color. Defaults to [`OutlineColor::Invert`] when omitted (CSS UI 3 §4.4).
    pub color: OutlineColor,
}

/// Offset along one axis of a `<position>` value type (CSS Values and Units 4
/// §8.3 <https://www.w3.org/TR/css-values-4/#typedef-position>). CSS
/// Backgrounds and Borders 3 §2.6 extends that grammar with `<bg-position>`
/// <https://www.w3.org/TR/css-backgrounds-3/#typedef-bg-position> for
/// `background-position`; this shared type generalizes it for other properties.
///
/// # Grammar (CSS Backgrounds 3 §2.6 `<bg-position>` — a superset of the
/// general `<position>` in CSS Values 4 §8.3)
///
/// ```text
/// <position> =
///   [ left | center | right | top | bottom | <length-percentage> ]
/// |
///   [ left | center | right | <length-percentage> ]
///   [ top | center | bottom | <length-percentage> ]
/// |
///   [ center | [ left | right ] <length-percentage>? ] &&
///   [ center | [ top | bottom ] <length-percentage>? ]
/// ```
///
/// **The code block above describes `<bg-position>`, the grammar implemented for
/// `background-position` (see [`parse_bg_position`]), not `<position>`
/// itself.** Its final alternative independently permits omission of
/// `<length-percentage>?` in both groups. This is an extension specific to
/// `<bg-position>`: a three-value edge-offset form with an authored offset
/// on only one axis. CSS Values 4 §8.3's plain `<position>` has no such
/// intermediate form. Its corresponding alternative (`<position-four>`) is
/// `[[left|right] <length-percentage>] && [[top|bottom]
/// <length-percentage>]`: offsets are required, not optional (`?`). It
/// permits either authored offsets on both axes (four values) or a bare
/// keyword pair without `<length-percentage>` on either axis (the `&&`
/// form of `<position-two>`). Properties requiring `<position>` (such as
/// `object-position`) use [`parse_position_strict`] to enforce this
/// restriction (see the [`parse_position_branch3_strict`] docs).
/// `background-position` remains on `<bg-position>` and still allows the
/// three-value form.
///
/// The last of the three alternatives (`&&` allows its two groups in either
/// order) covers reordered keywords such as `top left`, as well as edge
/// offsets: `bottom 10px right 20px` (four values, also valid for
/// `<position>`) and `right 10px top` (three values, specific to
/// `<bg-position>`).
///
/// # Why two variants (`Start`/`End`)? A single `<length-percentage>` cannot represent both.
///
/// The horizontal component of `right 10px top` (a three-value edge offset;
/// bare `right 10px` alone is a *different*, ambiguous two-value form, as
/// pinned by `background_position_parse_right_10px_is_not_an_edge_offset`)
/// measures 10px from the right edge. This is equivalent to `100% - 10px`
/// from the left edge, but this crate does not implement `calc()` (see
/// [`DEFERRED_FUNCTIONS`]), so it cannot represent that as a single
/// `<length-percentage>` (a linear combination of px and %). The type
/// therefore retains which edge the offset starts from. Downstream layout,
/// which knows the background positioning area's size (also required to
/// resolve `<length-percentage>` itself), converts it to a final pixel
/// position. This consistently applies the same "absolutize at used-value
/// time" design as other `<length-percentage>` properties (`padding` /
/// `width`, etc.), whose percentages this crate passes through unresolved.
///
/// An `End` with a `Percent` payload is normalized immediately after
/// construction to the equivalent `Start` with percentage `100.0 - p`
/// (see [`normalize_css_position`]). Whenever a percentage can express
/// `right`/`bottom`, it is folded into `Start`. `End` occurs only with
/// non-percentage offsets (such as the horizontal component of
/// `right 10px top`) that would require an equivalent `calc()` expression.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CssPositionOffset {
    /// Offset from the start edge (horizontal: `left`, vertical: `top`).
    Start(Length),
    /// Offset from the end edge (horizontal: `right`, vertical: `bottom`) —
    /// see "Why two variants?" in the type docs above.
    End(Length),
}

/// The `<position>` value type (CSS Backgrounds and Borders 3 §2.6;
/// see the [`CssPositionOffset`] docs). Used by `background-position` /
/// `object-position` in this crate; defined generically for future reuse,
/// such as `transform-origin`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CssPosition {
    /// Offset along the horizontal axis.
    pub horizontal: CssPositionOffset,
    /// Offset along the vertical axis.
    pub vertical: CssPositionOffset,
}

/// The specified value of `background-image`.
///
/// CSS Backgrounds and Borders 3 §2.3 "Image Sources: the background-image
/// property" <https://www.w3.org/TR/css-backgrounds-3/#the-background-image>.
/// Grammar: `<bg-image>#`, `<bg-image> = <image> | none`. This property permits
/// a comma-separated list (`#` multiplier) of multiple background layers,
/// but this implementation accepts only one layer (the same "multiple-layer
/// compositing is follow-up work" scope carve-out as in the
/// [`BackgroundRepeat`] docs). To support comma lists later, wrap this enum
/// in `Arc<Vec<BackgroundImage>>`.
///
/// Both alternatives of `<image> = <url> | <gradient>` (CSS Images 4
/// <https://www.w3.org/TR/css-images-4/#typedef-image>) are implemented.
/// `<url>` reuses [`parse_url_value`] (the same two forms as
/// [`ContentComponent::Image`]: unquoted `url(...)` / quoted `url("...")`).
/// `<gradient>` (`linear-gradient()` / `repeating-linear-gradient()` /
/// `radial-gradient()` / `repeating-radial-gradient()` /
/// `conic-gradient()` / `repeating-conic-gradient()`, CSS Images 4
/// §3.1-§3.4) stores its payload in [`Gradient`].
///
/// Scope carving for `<gradient>` (features intentionally omitted from the
/// Level 4 grammar; see also the [`GradientColorStop`] docs):
///
/// - Linear and radial color stop positions support only
///   `<color> <length-percentage>?` (the CSS Images 3 §3.4.1 baseline
///   grammar). The Level 4 addition `<color-stop-length> =
///   <length-percentage>{1,2}` (two positions for one stop to form a band
///   of the same color) is unsupported there. Conic stops instead accept
///   `<color> <color-stop-angle>{1,2}?`: a double angle expands at parse
///   time into two single-position stops sharing the color (see
///   [`AngularColorStop`] docs), which preserves the solid-band meaning
///   without a two-position type.
/// - `<linear-color-hint>` (a transition hint between two stops) is
///   unsupported: `<color-stop-list>` is parsed as `<linear-color-stop>#`
///   with no hint elements. **This is already part of the CSS Images 3
///   §3.4.1 baseline grammar (`<color-stop-list> = <linear-color-stop> ,
///   [ <linear-color-hint>? , <linear-color-stop> ]#`), not a Level 4
///   addition.** Level 4 §3.5.1 carries over that same production. Thus,
///   this crate does not fully implement the Level 3 baseline while merely
///   deferring Level 4 extensions; it also defers the baseline hint.
/// - A color stop list requires at least two stops (CSS Images 3 §3.4.1
///   baseline grammar `<linear-color-stop> , [ … ]#`: the first stop plus
///   a `#` group containing at least one, not three or more). The
///   single-stop gradient introduced by Level 4's `]#?` relaxation
///   (`gradient-single-stop-*.html` WPT tests) is unsupported.
/// - `<color-interpolation-method>` accepts only the six `<color-space>`
///   variants of [`MixColorSpace`] (`srgb`/`srgb-linear`/`lab`/`lch`/
///   `oklab`/`oklch`). The `hsl`/`hwb`/`xyz` families and `display-p3`
///   family are not supported: this crate lacks their corresponding
///   `<color>` function parsers (see [`parse_color`]), so gradients do
///   not accept them either.
/// - `radial-gradient()`/`repeating-radial-gradient()` support only the
///   CSS Images 3 §3.2.1 baseline `<radial-size>` grammar (`<radial-extent> |
///   <length [0,∞]> | <length-percentage [0,∞]>{2}`). The two-keyword
///   `<radial-extent>{1,2}` form added by CSS Images 4 §3.2.2 is
///   unsupported (see the [`RadialSize`] docs).
///
/// Linear and radial filling remains solid-first-stop in raikiri-paint, so
/// expanding their grammars now would increase validation costs before it
/// provides a benefit. Conic filling is implemented (see
/// [`Gradient::Conic`] paint), which is why its double-angle form is
/// accepted while the linear and radial counterparts stay deferred.
///
/// The comma-separated list (`#` multiplier) for multiple background layers
/// is also restricted to a single layer by this enum (the same "multiple-layer
/// compositing is follow-up work" scope carve-out as in the
/// [`BackgroundRepeat`] docs). To extend it to comma lists later, wrap
/// this enum in `Arc<Vec<BackgroundImage>>`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum BackgroundImage {
    /// `none` — the spec initial value. Draw no background image.
    None,
    /// `<url>` — the URL for one image layer, kept as its raw text without
    /// depending on the `url` crate. A [`SmolStr`] so that the inherited
    /// `list-style-image` every descendant copies shares a long URL instead
    /// of copying it per node.
    Url(SmolStr),
    /// `<gradient>` — one of six gradient functions. Conic filling is
    /// implemented in the paint layer; linear and radial still paint as
    /// their solid first stop (see the type docs). This variant retains
    /// the parsed result in
    /// [`ComputedValues`](crate::computed::ComputedValues); the font-relative
    /// part of `<length-percentage>` is absolutized to `Px` in the computed
    /// layer, and only `<percentage>` is deferred to painting (see the
    /// `resolve_background_image` docs). Shared so that copying the value,
    /// as every descendant does with an inherited `list-style-image`, does not
    /// copy its color stops.
    Gradient(Arc<Gradient>),
}

/// `<angle>` (CSS Values 4 §7.1 "Angle Units: the &lt;angle&gt; type and
/// deg, grad, rad, turn units"
/// <https://www.w3.org/TR/css-values-4/#angles>).
///
/// All four units (`deg`/`grad`/`rad`/`turn`) convert purely between units.
/// Unlike `em`/`%`, they have no content-relative context (no dependence on
/// font size or a percentage base). Thus, unlike [`Length`], there is no
/// reason to keep a separate variant per unit in the specified layer. The
/// spec itself says "All `<angle>` units are compatible, and `deg` is their
/// canonical unit"; this crate likewise folds them into one value in `deg`
/// units at parse time.
///
/// `degrees` retains the authored value without normalization modulo 360
/// (`rem_euclid`): `810deg` remains `810.0`. Such normalization would not
/// change the rotation, and the specified layer has no reason to do it.
/// Interpreting a gradient's direction/angle is outside this crate's scope
/// (paint is responsible), so downstream code also decides whether to
/// normalize.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Angle(pub f32);

/// `<angle-percentage>` (CSS Values 4 §5.6 "Mixing Percentages and
/// Dimensions" <https://www.w3.org/TR/css-values-4/#mixed-percentages>) —
/// the `<color-stop-angle>` payload used for angular color stop positions
/// in `conic-gradient()` ([`AngularColorStop`]).
/// This is the angle counterpart of `<length-percentage>` ([`Length`]).
/// `Percent` has the same semantics: a ratio to a base, retaining the
/// authored number unchanged.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnglePercentage {
    /// `<angle>` alternative.
    Angle(Angle),
    /// `<percentage>` alternative — authored number (`50%` → `50.0`,
    /// following the same convention as [`Length::Percent`]).
    Percent(f32),
}

/// `<gradient>` (CSS Images Module Level 4 §3 "Gradients"
/// <https://www.w3.org/TR/css-images-4/#gradients>) — one of the 6 gradient
/// functions. Payload of [`BackgroundImage::Gradient`].
///
/// The `repeating-*` variants use the same grammar as their non-repeating
/// counterparts (spec verbatim, CSS Images 4 §3.4: "These notations take the same values
/// and are interpreted the same as their respective non-repeating
/// siblings"). Each struct therefore has a `repeating: bool` field
/// ([`LinearGradient::repeating`] etc.) rather than a separate repeating variant.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Gradient {
    /// `linear-gradient()` / `repeating-linear-gradient()` (CSS Images 4
    /// §3.1).
    Linear(LinearGradient),
    /// `radial-gradient()` / `repeating-radial-gradient()` (CSS Images 4
    /// §3.2).
    Radial(RadialGradient),
    /// `conic-gradient()` / `repeating-conic-gradient()` (CSS Images 4
    /// §3.3).
    Conic(ConicGradient),
}

/// `linear-gradient()` / `repeating-linear-gradient()` (CSS Images 4 §3.1
/// "Linear Gradients: the linear-gradient() notation"
/// <https://www.w3.org/TR/css-images-4/#linear-gradients>).
///
/// Grammar: `<linear-gradient-syntax> = [ [ <angle> | <zero> | to
/// <side-or-corner> ] || <color-interpolation-method> ]? , <color-stop-list>`.
/// The omitted defaults for `direction` and `interpolation` are stored directly
/// ("defaults to to bottom" / the spec-mandated default of
/// [`GradientColorInterpolation`]). This follows the same policy as other
/// keyword defaults, such as the "center" default of [`CssPosition`]:
/// resolve an omitted value immediately rather than encoding omission in the type.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    /// Whether this is `repeating-linear-gradient()` (see [`Gradient`] docs).
    pub repeating: bool,
    /// Direction of the gradient line. Defaults to `to bottom` if omitted
    /// ([`LinearGradientDirection::Side`] with `vertical: Some(Bottom)`).
    pub direction: LinearGradientDirection,
    /// `in <color-space> <hue-interpolation-method>?` clause. Defaults to
    /// `Oklab` if omitted (CSS Images 4 §3.5.2 "Coloring the Gradient Line" — this
    /// subsection defines interpolation for all 3 gradient shapes, not just
    /// linear — "If no `<color-interpolation-method>` is specified in the
    /// gradient function, the color space used for gradient interpolation
    /// is the default interpolation color space, Oklab").
    pub interpolation: GradientColorInterpolation,
    /// Color stop list. Contains at least 2 stops (see the scope-carving
    /// section of [`BackgroundImage`] docs). `Arc` follows the same cheap-clone
    /// pattern as other comma-separated list payloads (`Content(Arc<Vec<..>>)` etc.).
    pub stops: Arc<Vec<GradientColorStop>>,
}

/// The 2 alternatives for [`LinearGradient::direction`]
/// (`<angle> | <zero> | to <side-or-corner>`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinearGradientDirection {
    /// `<angle>` alternative (including `<zero>`).
    Angle(Angle),
    /// `to <side-or-corner>` alternative.
    Side(SideOrCorner),
}

/// `<side-or-corner> = [left | right] || [top | bottom]` (CSS Images 4
/// §3.1). At least one component must be `Some`; the parser rejects two `None`s.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideOrCorner {
    /// Horizontal component (`left`/`right`); optional.
    pub horizontal: Option<HorizontalSide>,
    /// Vertical component (`top`/`bottom`); optional.
    pub vertical: Option<VerticalSide>,
}

/// Keyword for [`SideOrCorner::horizontal`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizontalSide {
    /// `left`.
    Left,
    /// `right`.
    Right,
}

css_keywords!(HorizontalSide {
    Left => "left",
    Right => "right",
});

/// Keyword for [`SideOrCorner::vertical`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalSide {
    /// `top`.
    Top,
    /// `bottom`.
    Bottom,
}

css_keywords!(VerticalSide {
    Top => "top",
    Bottom => "bottom",
});

/// `radial-gradient()` / `repeating-radial-gradient()` (CSS Images 4 §3.2
/// "Radial Gradients: the radial-gradient() notation"
/// <https://www.w3.org/TR/css-images-4/#radial-gradients>).
///
/// Grammar: `<radial-gradient-syntax> = [ [ [ <radial-shape> ||
/// <radial-size> ]? [ at <position> ]? ] || <color-interpolation-method> ]?
/// , <color-stop-list>`. Omitted defaults for `shape` and `size` are stored
/// directly (the shape-inference rules in CSS Images 3 §3.2.1; the same
/// policy as the [`LinearGradient`] docs).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    /// Whether this is `repeating-radial-gradient()`.
    pub repeating: bool,
    /// Ending shape. If omitted, defaults to `Circle` when `size` is
    /// `Circle(_)`, or to `Ellipse` otherwise (including an omitted size).
    /// CSS Images 3 §3.2.1: "the ending shape
    /// defaults to a circle if the `<radial-size>` is a single `<length>`,
    /// and to an ellipse otherwise".
    pub shape: RadialShape,
    /// Size of the ending shape. Defaults to `Extent(FarthestCorner)` if omitted.
    pub size: RadialSize,
    /// Center of the gradient. Defaults to `center` if omitted.
    pub position: CssPosition,
    /// `in <color-space> <hue-interpolation-method>?` clause. Defaults to
    /// `Oklab` if omitted, as in [`LinearGradient::interpolation`].
    pub interpolation: GradientColorInterpolation,
    /// Color stop list (same shape as [`LinearGradient::stops`]).
    pub stops: Arc<Vec<GradientColorStop>>,
}

/// [`RadialGradient::shape`] — `<radial-shape> = circle | ellipse`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialShape {
    /// `circle`.
    Circle,
    /// `ellipse`.
    Ellipse,
}

css_keywords!(RadialShape {
    Circle => "circle",
    Ellipse => "ellipse",
});

/// [`RadialGradient::size`] (CSS Images 3 §3.2.1 baseline grammar
/// `<radial-size> = <radial-extent> | <length [0,∞]> |
/// <length-percentage [0,∞]>{2}`).
///
/// The two-keyword `<radial-extent>{1,2}` form added in CSS Images 4 §3.2.2
/// (an extension from the circle()/ellipse() `<basic-shape>` grammar)
/// is not supported; see the scope-carving section in [`BackgroundImage`] docs.
/// The parser enforces the non-negative constraint (`[0,∞]`) on `Circle` and
/// `Ellipse` variants. The type does not encode this constraint, following
/// the convention for other non-negative contexts of [`Length`], such as
/// `border-width`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialSize {
    /// `<radial-extent>` keyword — usable with either [`RadialShape::Circle`]
    /// or [`RadialShape::Ellipse`].
    Extent(RadialExtent),
    /// Explicit `<length [0,∞]>` — usable only with [`RadialShape::Circle`]
    /// (enforced by the parser).
    Circle(Length),
    /// Explicit `<length-percentage [0,∞]>{2}` (horizontal and vertical radii) —
    /// usable only with [`RadialShape::Ellipse`] (enforced by the parser).
    Ellipse(Length, Length),
}

/// Keyword for [`RadialSize::Extent`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialExtent {
    /// `closest-side`.
    ClosestSide,
    /// `closest-corner`.
    ClosestCorner,
    /// `farthest-side`.
    FarthestSide,
    /// `farthest-corner` — the spec-mandated default for [`RadialSize`].
    FarthestCorner,
}

css_keywords!(RadialExtent {
    ClosestSide => "closest-side",
    ClosestCorner => "closest-corner",
    FarthestSide => "farthest-side",
    FarthestCorner => "farthest-corner",
});

/// `conic-gradient()` / `repeating-conic-gradient()` (CSS Images 4 §3.3
/// "Conic Gradients: the conic-gradient() notation"
/// <https://www.w3.org/TR/css-images-4/#conic-gradients>).
///
/// Grammar: `<conic-gradient-syntax> = [ [ [ from [ <angle> | <zero> ] ]?
/// [ at <position> ]? ] || <color-interpolation-method> ]? ,
/// <angular-color-stop-list>`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradient {
    /// Whether this is `repeating-conic-gradient()`.
    pub repeating: bool,
    /// `from <angle>`. Defaults to `0deg` if omitted.
    pub angle: Angle,
    /// `at <position>`. Defaults to `center` if omitted.
    pub position: CssPosition,
    /// `in <color-space> <hue-interpolation-method>?` clause. Defaults to
    /// `Oklab` if omitted, as in [`LinearGradient::interpolation`].
    pub interpolation: GradientColorInterpolation,
    /// Angular color stop list (same shape as [`LinearGradient::stops`],
    /// but with [`AngularColorStop`] as the position type).
    pub stops: Arc<Vec<AngularColorStop>>,
}

/// `in <color-space> <hue-interpolation-method>?` (CSS Color 4 §13.2;
/// see [`MixColorSpace`] docs) — the color-interpolation clause shared by
/// gradient functions.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientColorInterpolation {
    /// Color space used for interpolation.
    pub color_space: MixColorSpace,
    /// Hue interpolation direction. Has no meaning unless `color_space` is
    /// polar (`Lch`/`Oklch`); otherwise `Shorter` is always stored (see
    /// [`parse_gradient_color_interpolation`]).
    pub hue_method: HueInterpolationMethod,
}

/// Color payload of a gradient stop containing `<color>`: distinguishes
/// the `currentcolor` keyword from a resolved `<color>` (same shape as
/// [`TextShadowColor`], but a separate type for gradient stops).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientStopColor {
    /// `currentcolor` keyword. Used-value resolution belongs to the paint layer.
    CurrentColor,
    /// Resolved `<color>` value.
    Resolved(CssColor),
}

/// Entry in the color-stop list of `<linear-color-stop>` /
/// `<radial-gradient-syntax>` (CSS Images 3 §3.4.1 "Color Stop Lists";
/// Level 4 §3.5.1 retains the same production).
///
/// This crate implements only the CSS Images 3 baseline grammar
/// `<linear-color-stop> =
/// <color> <length-percentage>?`. It does not
/// support the Level 4 addition `<color-stop-length> = <length-percentage>{1,2}`
/// (two positions for one stop, producing a band of the same color).
/// It **also** does not support `<linear-color-hint>` (a transition hint
/// between stops). This is not a Level 4 extension; it is already present
/// in the Level 3 §3.4.1 baseline grammar (`<color-stop-list> = <linear-color-stop> , [
/// <linear-color-hint>? , <linear-color-stop> ]#`). This crate defers that
/// part of the baseline too (see the scope-carving section in
/// [`BackgroundImage`] docs).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientColorStop {
    /// Color of the stop.
    pub color: GradientStopColor,
    /// Position of the stop (`<length-percentage>`). If omitted, fixup
    /// determines it automatically (CSS Images 3 §3.4.3 "Color Stop \"Fixup\"").
    /// That decision requires the length of the gradient line, so it belongs
    /// to the used-value layer (paint); this crate retains `None`.
    pub position: Option<Length>,
}

/// `<angular-color-stop>` — the conic-gradient counterpart to
/// [`GradientColorStop`] (only the position type changes to
/// `<angle-percentage>`; CSS Images 4 §3.5.1). One stop carries a single
/// optional position; an authored double position such as `red 0 25%`
/// expands at parse time into two stops (`red 0`, `red 25%`) with the same
/// color, which keeps the solid-band meaning while this type stays
/// single-position. `<angular-color-hint>` remains unsupported, as for
/// [`GradientColorStop`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AngularColorStop {
    /// Color of the stop.
    pub color: GradientStopColor,
    /// Position of the stop (`<angle-percentage>`). Omission is handled
    /// as for [`GradientColorStop::position`].
    pub position: Option<AnglePercentage>,
}

/// Keyword for one axis of `<repeat-style>` (CSS Backgrounds and Borders 3 §2.4
/// <https://www.w3.org/TR/css-backgrounds-3/#typedef-repeat-style>).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundRepeatKeyword {
    /// `repeat` — the spec initial value (on both axes). Repeats tiles,
    /// clipping the last tile if necessary.
    Repeat,
    /// `space` — repeats tiles and distributes any remaining space evenly
    /// between them.
    Space,
    /// `round` — repeats tiles, resizing them to fit an integral number.
    Round,
    /// `no-repeat` — places only one tile.
    NoRepeat,
}

css_keywords!(BackgroundRepeatKeyword {
    Repeat => "repeat",
    Space => "space",
    Round => "round",
    NoRepeat => "no-repeat",
});

/// Specified value of `background-repeat`.
///
/// CSS Backgrounds and Borders 3 §2.4 "Tiling Images: the
/// background-repeat property". Grammar: `<repeat-style>#` — this
/// property allows a comma-separated list (`#` multiplier) for multiple
/// background layers, but this implementation accepts only a single layer.
/// `background-image` itself is not implemented yet (a separate task), so
/// there is no way to observe multiple layers. Once `background-image`
/// supports comma-separated lists, this struct can be extended simply by
/// wrapping it in `Arc<Vec<BackgroundRepeat>>` (the same shape as converting
/// `BoxShadowItem` to `Arc<Vec<..>>`).
///
/// `repeat-x` = `{x: Repeat, y: NoRepeat}`, `repeat-y` = `{x: NoRepeat, y:
/// Repeat}` (the computed values defined by the spec for these two-keyword
/// shorthands; see [`parse_background_repeat`] docs). A single keyword
/// applies to both axes (`repeat` = `repeat repeat`, etc.).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackgroundRepeat {
    /// Repeat mode on the horizontal axis.
    pub x: BackgroundRepeatKeyword,
    /// Repeat mode on the vertical axis.
    pub y: BackgroundRepeatKeyword,
}

/// Specified value of `background-attachment`.
///
/// CSS Backgrounds and Borders 3 §2.5 "Affixing Images: the
/// background-attachment property". Grammar: `<attachment>#` — see
/// [`BackgroundRepeat`] docs for why comma-separated lists are out of scope.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundAttachment {
    /// `scroll` — the spec initial value. The background is fixed relative
    /// to the block containing the element (the containing-block chain).
    /// It does not scroll with the element's content, but does follow
    /// scrolling of the entire page.
    Scroll,
    /// `fixed` — the background is fixed relative to the viewport.
    Fixed,
    /// `local` — the background scrolls with the element's own content.
    Local,
}

css_keywords!(BackgroundAttachment {
    Scroll => "scroll",
    Fixed => "fixed",
    Local => "local",
});

/// Box keyword shared by `background-clip` and `background-origin`
/// (CSS Backgrounds and Borders 3 §2.7 "Painting Area: the
/// background-clip property" / "Positioning Area: the
/// background-origin property"). Both properties have the grammar
/// `<visual-box>#`; see [`BackgroundRepeat`] docs for why comma-separated
/// lists are out of scope.
///
/// The spec initial value differs by property: `background-clip` uses
/// `border-box`, while `background-origin` uses `padding-box`
/// (see [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualBox {
    /// `border-box` — the outside edge of the border.
    BorderBox,
    /// `padding-box` — the inside edge of the border, outside edge of the padding.
    PaddingBox,
    /// `content-box` — the inside edge of the padding, edge of the content box.
    ContentBox,
    /// `border-area` — the edge of the border area (CSS Backgrounds 4 §2.6).
    BorderArea,
    /// `text` — clips to the shape of the text (CSS Backgrounds 4 §2.6, `background-clip: text`).
    Text,
}

css_keywords!(VisualBox {
    BorderBox => "border-box",
    PaddingBox => "padding-box",
    ContentBox => "content-box",
    BorderArea => "border-area",
    Text => "text",
});

/// Specified value of `background-size`.
///
/// CSS Backgrounds and Borders 3 §2.9 "Sizing Images: the
/// background-size property". Grammar: `<bg-size>#` — see
/// [`BackgroundRepeat`] docs for why comma-separated lists are out of scope.
///
/// `<bg-size> = [ <length-percentage [0,∞]> | auto ]{1,2} | cover |
/// contain`. If only one value is specified, the second axis defaults to
/// **`auto`** (spec verbatim: "If only one value is given the second is assumed to be
/// auto."). This differs from the fill rule of [`BorderRadius`], which also
/// accepts 1-2 values but copies the first when the second is omitted
/// (see [`parse_background_size`] docs).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackgroundSize {
    /// `[ <length-percentage [0,∞]> | auto ]{1,2}` — each axis independently
    /// takes a length or `auto`.
    Explicit {
        /// Size on the horizontal axis.
        width: LengthOrAuto,
        /// Size on the vertical axis.
        height: LengthOrAuto,
    },
    /// `cover` — scales the background to cover the entire background
    /// positioning area while preserving its aspect ratio.
    Cover,
    /// `contain` — scales the background to the largest size that fits
    /// inside the background positioning area while preserving its aspect ratio.
    Contain,
}

/// Temporary carrier for the parse result of the `background` shorthand.
///
/// CSS Backgrounds and Borders 3 §2.10 "Backgrounds Shorthand: the
/// background property"
/// <https://www.w3.org/TR/css-backgrounds-3/#the-background>. Spec text
/// verbatim: "The background property is a shorthand property for setting
/// most background properties at the same place in the style sheet. […]
/// Given a valid declaration, for each layer the shorthand first sets the
/// corresponding value of each of background-image, background-position,
/// background-size, background-repeat, background-origin, background-clip
/// and background-attachment to that property's initial value, then assigns
/// any explicit values specified for this layer in the declaration. Finally
/// background-color is set to the specified color, if any, else set to its
/// initial value."
///
/// Grammar (single layer — this crate's scope carving, see below):
///
/// ```text
/// <final-bg-layer> = <bg-image> || <bg-position> [ / <bg-size> ]? ||
///                     <repeat-style> || <attachment> || <visual-box> ||
///                     <visual-box> || <'background-color'>
/// ```
///
/// `<bg-position> [ / <bg-size> ]?` is a single `||` alternative, not two
/// independently-orderable ones — `<bg-size>` may only follow a position,
/// separated by a literal `/` ([`parse_background_position_and_size`], same
/// fixed-pair shape as `<grid-line> [ / <grid-line> ]?`,
/// [`parse_grid_line_shorthand`]).
///
/// `<visual-box>` appears **twice** as independent `||` alternatives. Spec
/// verbatim: "If one `<visual-box>` value is present then it sets both
/// background-origin and background-clip to that value. If two values are
/// present, then the first sets background-origin and the second
/// background-clip." — i.e. up to 2 occurrences are accepted (in document
/// order, interleaved freely with the other components), not 2
/// independently-named slots.
///
/// # Single layer only (Non-goal: comma-separated multi-layer)
///
/// The full grammar is `<bg-layer>#? , <final-bg-layer>` — a comma-separated
/// list of layers, where every layer but the last is a `<bg-layer>` (same as
/// `<final-bg-layer>` minus the `<'background-color'>` alternative — spec
/// verbatim: "A color is permitted in `<final-bg-layer>`, but not in
/// `<bg-layer>`."). This crate accepts only a single layer, matching the
/// existing single-layer scope carving already in place on
/// [`BackgroundImage`] / [`BackgroundRepeat`] / [`BackgroundSize`] /
/// [`CssPosition`] etc. Since there is only ever one layer, it is always the
/// *final* one, so `<'background-color'>` is always a valid component —
/// there is no separate "non-final" grammar to support.
///
/// A comma anywhere in the value is therefore **not** consumed by
/// [`parse_background_shorthand`] — the parser stops at the first
/// unrecognized token (the comma) after parsing everything before it, and
/// the leftover comma (plus any further layers) makes the whole declaration
/// invalid at the [`crate::rule::parse_declaration_block_within`] call site's
/// `expect_exhausted` check, so it is dropped entirely. This is a deliberate
/// divergence from the spec's per-layer compositing: this crate never
/// renders a first-layer-only approximation of a multi-layer declaration
/// (which would be wrong-but-plausible), it rejects the declaration outright
/// (silently, per this crate's general "invalid declaration → drop" policy).
///
/// # Initial value fill (omitted components)
///
/// Spec §2.10 verbatim: "the shorthand first sets the corresponding value of
/// each of background-image, background-position, background-size,
/// background-repeat, background-origin, background-clip and
/// background-attachment to that property's initial value, then assigns any
/// explicit values specified for this layer" — and "background-color is set
/// to the specified color, if any, else set to its initial value."
/// [`parse_background_shorthand`] applies this fill for every component
/// omitted from the declaration, using the same initial values as the 8
/// standalone longhands ([`crate::specified::SpecifiedValues::initial`]'s
/// `background_*` fields are the canonical source for each).
///
/// [`crate::rule::expand_shorthand_into`] expands
/// [`PropertyValue::Background`] into the 8 longhand
/// [`PropertyValue::BackgroundColor`] / [`PropertyValue::BackgroundImage`] /
/// [`PropertyValue::BackgroundRepeat`] / [`PropertyValue::BackgroundAttachment`] /
/// [`PropertyValue::BackgroundPosition`] / [`PropertyValue::BackgroundSize`] /
/// [`PropertyValue::BackgroundClip`] / [`PropertyValue::BackgroundOrigin`]
/// declarations — margin/padding/border/outline shorthand precedent, same
/// "parse-time expansion, never reaches cascade" design (details on that
/// function's doc).
///
/// `#[non_exhaustive]` is intentionally omitted, matching sibling
/// shorthand-only carriers [`GridLineShorthand`] / [`TextDecorationShorthand`]:
/// this type exists only as [`PropertyValue::Background`]'s payload, is
/// never held by [`ComputedValues`] / [`SpecifiedValues`], and is not
/// re-exported by the `raikiri` umbrella crate.
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundShorthand {
    /// Symbolic color component, when it contains `currentcolor`.
    pub(crate) color_expression: Option<SmolStr>,
    /// `background-color` component: [`CssColor::TRANSPARENT`] if omitted (the spec
    /// initial value).
    pub color: CssColor,
    /// `background-image` component: [`BackgroundImage::None`] if omitted (the spec
    /// initial value).
    pub image: BackgroundImage,
    /// `background-repeat` component: [`BackgroundRepeatKeyword::Repeat`] on both axes
    /// if omitted (the spec initial value).
    pub repeat: BackgroundRepeat,
    /// `background-attachment` component: [`BackgroundAttachment::Scroll`]
    /// if omitted (the spec initial value).
    pub attachment: BackgroundAttachment,
    /// `background-position` component — defaults to `0% 0%` (the spec's initial value).
    pub position: CssPosition,
    /// `background-size` component: `auto` on both axes if omitted. Size
    /// can occur only directly after position, separated by `/` (see the
    /// position+size section in the [`Self`] documentation).
    pub size: BackgroundSize,
    /// `background-clip` component — defaults to [`VisualBox::BorderBox`] (the spec's
    /// initial value). The number of `<visual-box>` occurrences and the rules for assigning them to origin/clip
    /// are described in the [`Self`] docs.
    pub clip: VisualBox,
    /// `background-origin` component — defaults to [`VisualBox::PaddingBox`] (the spec's
    /// initial value).
    pub origin: VisualBox,
}

/// Specified value of `object-fit`.
///
/// CSS Images Module Level 3 §5.1 "Sizing the replaced element: the
/// object-fit property"
/// <https://www.w3.org/TR/css-images-3/#the-object-fit>. Grammar: `fill |
/// contain | cover | none | scale-down`. Applies to: replaced elements
/// only. **Non-inherited**. Computed value = specified keyword — no length
/// payload (the same shape as `BackgroundAttachment`).
///
/// It pairs with `object-position` (CSS Images 3 §5.2; reuses [`CssPosition`]) as a
/// property, but these five keywords describe how the concrete object size of a replaced element
/// is determined by a layout-time algorithm (in §5.3 of the same spec,
/// "Sizing the replaced element", the default-object-size / concrete-object-size
/// procedure). This crate does not implement that layout application algorithm —
/// this variant stores only the cascade/computed-value keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFit {
    /// `fill` — the spec's initial value. Stretch replaced content to fit the content box
    /// (without preserving its aspect ratio).
    Fill,
    /// `contain` — preserve the aspect ratio and scale to the largest size that fits
    /// within the content box.
    Contain,
    /// `cover` — preserve the aspect ratio and scale to the smallest size that covers
    /// the content box (possibly overflowing the box on one axis).
    Cover,
    /// `none` — do not resize the content. The concrete object size is the
    /// intrinsic size (or, if absent, the result of the spec's default object size
    /// algorithm), used unchanged.
    None,
    /// `scale-down` — of the concrete object sizes produced by `none` and `contain`,
    /// choose the smaller one.
    ScaleDown,
}

css_keywords!(ObjectFit {
    Fill => "fill",
    Contain => "contain",
    Cover => "cover",
    None => "none",
    ScaleDown => "scale-down",
});

/// Specified value of `isolation`.
///
/// CSS Compositing and Blending Level 1 §3.4.2 "Isolation: the isolation
/// property" <https://www.w3.org/TR/compositing-1/#isolation>. Grammar:
/// `auto | isolate`. **Non-inherited**. Computed value = specified keyword —
/// no length payload (the same shape as `ObjectFit`).
///
/// The spec gives detailed conditions for whether `isolation` creates a
/// stacking context / group (element types, SVG containers, etc.), but
/// this crate does not implement that application algorithm. This variant
/// stores only the cascaded/computed keyword. Creating compositing groups
/// belongs to raikiri-paint; the `ObjectFit` documentation similarly
/// leaves the property’s application algorithm to layout.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    /// `auto` — the spec's initial value. The element itself does not force an independent stacking context /
    /// group.
    Auto,
    /// `isolate` — make the element an independent stacking context and confine `mix-blend-mode`
    /// blending to its subtree.
    Isolate,
}

css_keywords!(Isolation {
    Auto => "auto",
    Isolate => "isolate",
});

/// Specified value of `mix-blend-mode`.
///
/// CSS Compositing and Blending Level 1 §3.4.1 "Mix Blend Mode: the
/// mix-blend-mode property"
/// <https://www.w3.org/TR/compositing-1/#mix-blend-mode>. Grammar:
/// `<blend-mode> = normal | multiply | screen | overlay | darken | lighten |
/// color-dodge | color-burn | hard-light | soft-light | difference |
/// exclusion | hue | saturation | color | luminosity` (the `<blend-mode>` grammar comes from
/// CSS Compositing and Blending Level 1 §2 "Compositing and Blending"
/// which defines it; the grammar is also shared with the `background-blend-mode` property,
/// which this crate does not implement). **Non-inherited**. Computed value = specified
/// keyword — no length payload.
///
/// The actual blending operations (the equations for each mode, CSS Compositing and Blending
/// Level 1 §3.2 "Blending") require separate compositing support in raikiri-paint —
/// this variant stores only the cascade/computed-value keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixBlendMode {
    /// `normal` — the spec's initial value. Normal compositing that passes through the backdrop.
    Normal,
    /// `multiply` — CSS Compositing and Blending Level 1 §3.2.1.
    Multiply,
    /// `screen` — see §3.2.2.
    Screen,
    /// `overlay` — see §3.2.3.
    Overlay,
    /// `darken` — see §3.2.4.
    Darken,
    /// `lighten` — see §3.2.5.
    Lighten,
    /// `color-dodge` — see §3.2.6.
    ColorDodge,
    /// `color-burn` — see §3.2.7.
    ColorBurn,
    /// `hard-light` — see §3.2.8.
    HardLight,
    /// `soft-light` — see §3.2.9.
    SoftLight,
    /// `difference` — see §3.2.10.
    Difference,
    /// `exclusion` — see §3.2.11.
    Exclusion,
    /// `hue` — non-separable blend mode; CSS Compositing and Blending
    /// Level 1 §3.2.12.
    Hue,
    /// `saturation` — see §3.2.13.
    Saturation,
    /// `color` — see §3.2.14.
    Color,
    /// `luminosity` — see §3.2.15.
    Luminosity,
}

css_keywords!(MixBlendMode {
    Normal => "normal",
    Multiply => "multiply",
    Screen => "screen",
    Overlay => "overlay",
    Darken => "darken",
    Lighten => "lighten",
    ColorDodge => "color-dodge",
    ColorBurn => "color-burn",
    HardLight => "hard-light",
    SoftLight => "soft-light",
    Difference => "difference",
    Exclusion => "exclusion",
    Hue => "hue",
    Saturation => "saturation",
    Color => "color",
    Luminosity => "luminosity",
});

/// `clip-path`'s `<geometry-box>` component (CSS Masking Level 1 §5.1,
/// "Basic Shapes: the clip-path property"
/// <https://www.w3.org/TR/css-masking-1/#the-clip-path>).
///
/// Grammar: `<geometry-box> = <shape-box> | fill-box | stroke-box |
/// view-box`; `<shape-box> = <box> | margin-box`; `<box> = border-box |
/// padding-box | content-box`. `fill-box`/`stroke-box`/`view-box` are
/// not part of `<shape-box>`; they are direct alternatives in `<geometry-box>` —
/// the two productions have no overlapping keywords (their union is border-box /
/// padding-box / content-box / margin-box / fill-box / stroke-box /
/// view-box: seven keywords).
///
/// Resolving SVG contexts (`fill-box`/`stroke-box`/`view-box`) belongs to
/// raikiri-paint's SVG rendering implementation (currently nonexistent; see the [`ClipPath`] docs) — this variant
/// only stores the keyword.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryBox {
    /// `border-box`.
    BorderBox,
    /// `padding-box`.
    PaddingBox,
    /// `content-box`.
    ContentBox,
    /// `margin-box`.
    MarginBox,
    /// `fill-box` — the SVG bounding box.
    FillBox,
    /// `stroke-box` — the SVG stroke bounding box.
    StrokeBox,
    /// `view-box` — the nearest SVG viewport.
    ViewBox,
}

css_keywords!(GeometryBox {
    BorderBox => "border-box",
    PaddingBox => "padding-box",
    ContentBox => "content-box",
    MarginBox => "margin-box",
    FillBox => "fill-box",
    StrokeBox => "stroke-box",
    ViewBox => "view-box",
});

/// `fill-rule` for [`BasicShape::Polygon`] / [`BasicShape::Path`].
///
/// CSS Shapes Module Level 1 §3.1 "Supported Shapes"
/// <https://www.w3.org/TR/css-shapes-1/#supported-basic-shapes> defines
/// `<polygon()>` and `<path()>` as `polygon( <'fill-rule'>? … )` and
/// `path( <'fill-rule'>? , <string> )`, where `<fill-rule>` is the SVG
/// `fill-rule` property (`nonzero | evenodd`, CSS Masking Level 1 §5.1
/// delegates to this definition). Defaults to `nonzero` when omitted.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    /// `nonzero` — spec default.
    NonZero,
    /// `evenodd`.
    EvenOdd,
}

css_keywords!(FillRule {
    NonZero => "nonzero",
    EvenOdd => "evenodd",
});

/// `<shape-radius>` for [`BasicShape::Circle`] / [`BasicShape::Ellipse`].
///
/// CSS Shapes Module Level 1 §3.1
/// <https://www.w3.org/TR/css-shapes-1/#supported-basic-shapes> defines
/// `circle()` / `ellipse()` as `circle( <radial-size>? [ at <position> ]? )`
/// where `<radial-size>` is `<length-percentage [0,∞]> | closest-side |
/// farthest-side` (CSS Images 3 §3.2 `<radial-size>` repurposed for the
/// reference box). Negative lengths are invalid and cause the whole
/// `clip-path` declaration to drop.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeRadius {
    /// `<length-percentage [0,∞]>` — authored value stored as [`Length`].
    Length(Length),
    /// `closest-side`.
    ClosestSide,
    /// `farthest-side`.
    FarthestSide,
}

/// `inset()` shape — [`BasicShape::Inset`].
///
/// CSS Shapes Module Level 1 §3.1
/// <https://www.w3.org/TR/css-shapes-1/#supported-basic-shapes>:
/// `inset( <length-percentage>{1,4} [ round <'border-radius'> ]? )`.
/// The 1-4 inset values expand like `margin` shorthand (1→all, 2→vertical/horizontal,
/// 3→top/horizontal/bottom, 4→top/right/bottom/left). `round` introduces
/// an optional border-radius for the inset rectangle (slash-separated
/// elliptical radii are supported).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct InsetShape {
    /// Inset from top edge.
    pub top: Length,
    /// Inset from right edge.
    pub right: Length,
    /// Inset from bottom edge.
    pub bottom: Length,
    /// Inset from left edge.
    pub left: Length,
    /// Optional `round` border radius.
    pub border_radius: Option<InsetBorderRadius>,
}

/// Border radius for [`InsetShape`] `round` clause.
///
/// CSS Backgrounds and Borders Level 3 §5 `<border-radius>` grammar
/// (used via CSS Shapes `round <'border-radius'>`). Supports 1-4
/// `<length-percentage [0,∞]>` values optionally followed by `/` and a
/// second 1-4 group for elliptical radii. Values expand to four corners
/// clockwise (top-left, top-right, bottom-right, bottom-left).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct InsetBorderRadius {
    /// Horizontal radii for four corners (top-left, top-right, bottom-right, bottom-left).
    pub horizontal: [Length; 4],
    /// Vertical radii if slash-separated, otherwise `None` (circular).
    pub vertical: Option<[Length; 4]>,
}

/// `circle()` shape — [`BasicShape::Circle`].
///
/// CSS Shapes Module Level 1 §3.1: `circle( <shape-radius>? [ at <position> ]? )`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircleShape {
    /// Optional radius (`<shape-radius>`).
    pub radius: Option<ShapeRadius>,
    /// Optional position (`at <position>`).
    pub position: Option<CssPosition>,
}

/// `ellipse()` shape — [`BasicShape::Ellipse`].
///
/// CSS Shapes Module Level 1 §3.1: `ellipse( <shape-radius>{2}? [ at <position> ]? )`
/// (0-2 radii — spec `<radial-size>` for ellipse expands to two values, but
/// 0/1/2 are all valid; single radius leaves the other defaulting per
/// browser behavior and is accepted as valid here).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EllipseShape {
    /// Optional first radius (horizontal).
    pub radius_x: Option<ShapeRadius>,
    /// Optional second radius (vertical).
    pub radius_y: Option<ShapeRadius>,
    /// Optional position (`at <position>`).
    pub position: Option<CssPosition>,
}

/// `polygon()` shape — [`BasicShape::Polygon`].
///
/// CSS Shapes Module Level 1 §3.1:
/// `polygon( <fill-rule>? [ round <length> ]? , [<length-percentage> <length-percentage>]# )`.
/// The optional `round` length enables rounded vertices (CSS Shapes
/// Level 1 extension, `<length>` is non-negative). Points are stored as
/// pairs of `<length-percentage>` values.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct PolygonShape {
    /// Fill rule (defaults to `nonzero`).
    pub fill_rule: FillRule,
    /// Optional `round` length for rounded polygon.
    pub round: Option<Length>,
    /// Vertices — each `(x, y)` is `<length-percentage>`.
    pub points: Vec<(Length, Length)>,
}

/// `path()` shape — [`BasicShape::Path`].
///
/// CSS Shapes Module Level 1 §3.1: `path( <fill-rule>? , <string> )`.
/// The `<string>` is raw SVG path data; structured segment parsing is
/// deferred to raikiri-paint (no SVG path parser is reused here, per task
/// scope note). Stored without surrounding quotes.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct PathShape {
    /// Fill rule (defaults to `nonzero`).
    pub fill_rule: FillRule,
    /// Raw SVG path data string.
    pub path: String,
}

/// `<basic-shape>` for `clip-path`.
///
/// CSS Shapes Module Level 1 §3 "Basic Shapes"
/// <https://www.w3.org/TR/css-shapes-1/#basic-shape-functions> (primary
/// source for this type) as referenced by CSS Masking Level 1 §5.1
/// <https://www.w3.org/TR/css-masking-1/#the-clip-path>. Covers
/// `circle()` / `ellipse()` / `inset()` / `polygon()` / `path()`
/// — `rect()` / `xywh()` / `shape()` (Level 1 additions / newer drafts) are
/// intentionally out of scope for this task.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum BasicShape {
    /// `inset()` — see [`InsetShape`].
    Inset(InsetShape),
    /// `circle()` — see [`CircleShape`].
    Circle(CircleShape),
    /// `ellipse()` — see [`EllipseShape`].
    Ellipse(EllipseShape),
    /// `polygon()` — see [`PolygonShape`].
    Polygon(PolygonShape),
    /// `path()` — see [`PathShape`].
    Path(PathShape),
}

/// Specified value of `clip-path`.
///
/// CSS Masking Level 1 §5.1 "Basic Shapes: the clip-path property"
/// <https://www.w3.org/TR/css-masking-1/#the-clip-path>. Full grammar:
/// `<clip-source> | [ <basic-shape> || <geometry-box> ] | none`,
/// `<clip-source> = <url>`. **Non-inherited**.
///
/// The `<basic-shape>` grammar is defined by CSS Shapes Module Level 1 §3
/// <https://www.w3.org/TR/css-shapes-1/#basic-shape-functions>, not by
/// CSS Masking Level 1 itself. That spec is the primary source.
/// This crate implements five functions: `circle()`, `ellipse()`, `inset()`,
/// `polygon()`, and `path()`. `rect()`, `xywh()`, and `shape()` are outside
/// the scope of this task.
///
/// # Computed value
///
/// That section specifies "Computed value: as specified, but with `<url>` values made
/// absolute". This crate does not make `<url>` values absolute (resolve them
/// against a base URL), because it has no URL-resolution environment (base URL
/// or fetch facility). This has the same scope limit as [`BackgroundImage::Url`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ClipPath {
    /// `none` — the spec's initial value; performs no clipping.
    None,
    /// `<clip-source>` = `<url>` — a reference to an SVG `<clipPath>` element or similar resource.
    Url(String),
    /// `<geometry-box>` alone, without an accompanying `<basic-shape>`.
    GeometryBox(GeometryBox),
    /// `[ <basic-shape> || <geometry-box> ]` — shape alone, or shape
    /// paired with a reference box (either order in source).
    BasicShape {
        /// The `<basic-shape>` function.
        shape: Box<BasicShape>,
        /// Optional reference box (`<geometry-box>`).
        geometry_box: Option<GeometryBox>,
    },
}

/// Specified value of `mask-image` — a type alias for [`BackgroundImage`].
///
/// CSS Masking Level 1 §7.1 "Image Masking: the mask-image property"
/// <https://www.w3.org/TR/css-masking-1/#the-mask-image>. Full grammar:
/// `<mask-reference>#`, `<mask-reference> = none | <image> | <mask-source>`,
/// `<mask-source> = <url>`, `<image> = <url> | <gradient>`.
/// **Non-inherited**.
///
/// # Scope limit — only one layer
///
/// Comma-separated lists of multiple `<mask-reference>#` layers are not
/// supported. This follows the same scope decision and leaves the same room
/// for extension as the [`BackgroundImage`] docs: "comma-separated lists for
/// multiple background layers are unsupported; future support needs only an
/// `Arc<Vec<..>>` wrapper".
///
/// # `<mask-source>` and the `<image>` `url` alternative share concrete syntax
///
/// The `<mask-source>` (`<url>`) and `<image>`'s `<url>` alternative cannot
/// be distinguished by their concrete syntax: a parser cannot tell which
/// one `url(#foo)` was meant to represent. Thus this alias has the same
/// concrete syntax as [`BackgroundImage`]'s `<bg-image> = <url> | <gradient>`
/// (see the [`parse_mask_image`] docs).
///
/// # Reuse convention — reuse [`BackgroundImage`] verbatim
///
/// This alias follows convention (b): reuse a property-specific type for
/// another property. An existing example is [`FilterFunction::DropShadow`],
/// which reuses [`TextShadowItem`] verbatim. `MaskImage` and [`BackgroundImage`]
/// have identical variant shapes (`None` / `Url(String)` / `Gradient(Gradient)`),
/// so this is a type alias rather than a duplicate enum. It will also pick
/// up any future multi-layer support or other extension to [`BackgroundImage`]
/// automatically, avoiding drift.
pub type MaskImage = BackgroundImage;

/// One `transform` function (CSS Transforms Level 1 §9.1 "Two-dimensional
/// Subset" <https://www.w3.org/TR/css-transforms-1/#two-d-transform-functions>).
///
/// V2 (3D transforms such as `translate3d()`/`rotate3d()`/`matrix3d()`/
/// `perspective()`) is unsupported. Those functions have a separate spec
/// section (§10 "3D Transform Functions"); this crate explicitly supports
/// only the 2D functions in §9.1. The unrecognized-name path in
/// [`parse_transform_function`] silently drops 3D function names. This is
/// the same decision to defer a whole separate section as the scope limit
/// for `<basic-shape>` (see the [`ClipPath`] docs).
///
/// See the docs for [`parse_transform_number`]/
/// [`parse_transform_length_percentage`]/[`parse_angle_reject_nan`] for
/// how each numeric payload handles NaN. NaN caused by cssparser's exponent
/// overflow (`0 * Infinity` collapse) is rejected, while `+Inf`/`-Inf`
/// caused by magnitude overflow is retained where the spec does not restrict
/// the argument's range. This matches the `!is_nan()` guard on
/// [`PropertyValue::Opacity`].
///
/// # Absolutization — length half is absolutized, percent stays symbolic
///
/// The Computed value of the `transform` property itself is "as specified,
/// **but with lengths made absolute**"
/// (<https://www.w3.org/TR/css-transforms-1/#transform-property>). Unlike
/// `filter`'s simple "as specified" (which needs no absolutization), discussed
/// in the "Range restriction" section of the [`FilterFunction`] docs,
/// `transform` must absolutize `<length>` payloads under the spec: here,
/// the non-percentage part of the `Length` carried by [`Self::Translate`]/
/// [`Self::TranslateX`]/[`Self::TranslateY`].
///
/// On the element path, [`crate::resolve::ComputedTransformFunction`]/
/// [`crate::resolve::resolve_transform_function`] implements this distinction;
/// on the `page` path, phase 3 of [`crate::page::cascade_page`] does so through
/// the `Transform` arm of `absolutize_in_page_context`. They absolutize only
/// the length part of `<length-percentage>` against font-size/root-font-size,
/// leaving the percentage part symbolic. This is the same treatment that
/// [`crate::resolve::resolve_css_position`]/
/// [`crate::resolve::resolve_length_percentage`] applies to
/// `background-position`/`object-position`. The six `<number>` slots in
/// `Matrix` and the `<angle>` slots in `Rotate`/`Skew`/`SkewX`/`SkewY` do
/// not need this conversion: the former are already fully resolved
/// `<number>` values and the latter are `<angle>` values the spec does not
/// normalize. Neither involves percentages or box sizes.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformFunction {
    /// `matrix(<number>{6})` — six coefficients a, b, c, d, e, f of the homogeneous
    /// 2D affine matrix `[[a, c, e], [b, d, f], [0, 0, 1]]`.
    Matrix([f32; 6]),
    /// `translate(<length-percentage>, <length-percentage>?)` — the second
    /// argument defaults to `0` (see the [`parse_translate_args`] docs).
    Translate(Length, Length),
    /// `translateX(<length-percentage>)`.
    TranslateX(Length),
    /// `translateY(<length-percentage>)`.
    TranslateY(Length),
    /// `scale(<number>, <number>?)` — when omitted, the second argument copies
    /// the first (see the [`parse_scale_args`] docs).
    Scale(f32, f32),
    /// `scaleX(<number>)`.
    ScaleX(f32),
    /// `scaleY(<number>)`.
    ScaleY(f32),
    /// `rotate([<angle> | <zero>])`.
    Rotate(Angle),
    /// `skew([<angle> | <zero>], [<angle> | <zero>]?)` — the second argument
    /// defaults to `0deg` (see the [`parse_skew_args`] docs).
    Skew(Angle, Angle),
    /// `skewX([<angle> | <zero>])`.
    SkewX(Angle),
    /// `skewY([<angle> | <zero>])`.
    SkewY(Angle),
}

/// One `filter` function/reference (CSS Filter Effects Level 1 §6
/// "Filter Functions" <https://www.w3.org/TR/filter-effects-1/#filter-functions>
/// + the `<url>` alternative in §5).
///
/// # Range restriction: reject rather than clamp
///
/// §6.1 says "Negative values are not allowed" for each `<number-percentage>`
/// argument. CSS Color 4 §3.3 explicitly carves out an exception for the
/// `opacity` property, "retain the specified value, clamp the computed value",
/// but there is no
/// such exception here. The `filter` property's Computed value is "as
/// specified" (see the [`parse_filter_amount`] docs), so out-of-range values
/// are invalid under the CSS Values 4 §5 default. Reject them at parse time,
/// as with [`parse_nonneg_finite_number`] (flex-grow/flex-shrink).
///
/// For `grayscale()`/`invert()`/`opacity()`/`sepia()`, "values over 100%
/// allowed but UAs **must** clamp the values to 1" imposes a duty on the
/// user agent **during rendering**, not a change to the specified/computed
/// value. Since the `filter` property's Computed value is "as specified",
/// the value itself must remain unchanged. Leave this clamp to **paint**;
/// this crate carries the original value. These functions can therefore
/// share a payload type with `brightness()`/`contrast()`/`saturate()`, whose
/// "over 100% allowed" does not require clamping.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum FilterFunction {
    /// `blur(<length>?)` — defaults to `0px`; the standard deviation must be
    /// non-negative (reuses [`parse_non_negative_length`]).
    Blur(Length),
    /// `brightness(<number-percentage>?)` — defaults to `1`. Values above 100% need no
    /// clamping (see the enum documentation above).
    Brightness(f32),
    /// `contrast(<number-percentage>?)` — defaults to `1`. Values above 100% need no
    /// clamping.
    Contrast(f32),
    /// `grayscale(<number-percentage>?)` — defaults to `1`. The UA must clamp values
    /// above 100% to 1 **at rendering time**, but we preserve the original value
    /// (see the enum documentation above).
    Grayscale(f32),
    /// `hue-rotate([<angle> | <zero>]?)` — defaults to `0deg`; there is no range restriction.
    HueRotate(Angle),
    /// `invert(<number-percentage>?)` — defaults to `1`. Values above 100% are
    /// handled like `grayscale()`.
    Invert(f32),
    /// `opacity(<number-percentage>?)` — defaults to `1`. Values above 100% are
    /// handled like `grayscale()` (this namesake filter function is unrelated to the
    /// [`PropertyValue::Opacity`] property).
    Opacity(f32),
    /// `saturate(<number-percentage>?)` — defaults to `1`. Values above 100% need
    /// no clamping.
    Saturate(f32),
    /// `sepia(<number-percentage>?)` — defaults to `1`. Values above 100% are
    /// handled like `grayscale()`.
    Sepia(f32),
    /// `drop-shadow(<color>? && <length>{2,3})` — "Values are interpreted
    /// as for box-shadow but with the optional 3rd `<length>` value being
    /// the standard deviation instead of blur radius." Spread, inset, and multiple
    /// shadows are not allowed. Its grammar exactly matches [`TextShadowItem`], so
    /// we reuse that type (see the [`parse_drop_shadow_args`] documentation).
    DropShadow(TextShadowItem),
    /// `<url>` — a reference to an SVG `<filter>` element or similar target
    /// (the `[ <filter-function> | <url> ]+` grammar in §5).
    Url(String),
}

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
/// implemented in this crate, except for `color`, `background-color`, `font-size`,
/// `font-family`, `font-weight`, `font-style`, `display`, `opacity`, `visibility`,
/// `vertical-align`,
/// the border longhands and the `border` /
/// `border-right` shorthands, which accept all five keywords through
/// [`CssWideKeyword`] (see that type's docs for resolution). The `all` shorthand
/// additionally accepts literal `revert-layer` through [`PropertyValue::AllRevertLayer`];
/// its other CSS-wide values and `var()` form are not supported. Custom
/// properties interpret literal `revert` and `revert-layer` during cascading;
/// their other CSS-wide defaults are not supported.
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
/// The payload of `text-indent` ([`TextIndentValue`]).
///
/// It preserves all components of the CSS Text 3 §8.1 grammar
/// `<length-percentage> && hanging? && each-line?`. `Copy` (all fields are `Copy`)
/// keeps by-value cascade assignment and inheritance copies straightforward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextIndentValue {
    /// The `<length-percentage>` component.
    pub length: TextIndentLength,
    /// Whether the `hanging` keyword is present.
    pub hanging: bool,
    /// Whether the `each-line` keyword is present.
    pub each_line: bool,
}
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// Roll back all ordinary properties except direction and unicode-bidi by layer.
    AllRevertLayer,
    /// A `--<ident>` custom property. The value remains token-preserving until
    /// the computed-value stage, where `var()` references are resolved.
    CustomProperty(CustomProperty),
    /// A known property value containing `var()` or a math function.
    Deferred(DeferredValue),
    /// A `color` or `background-color` expression containing `currentcolor`.
    ContextualColor(ContextualColor),
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
    /// `list-style` sets type, position and image, including omitted defaults.
    /// Boxed so that it does not set the size of every `PropertyValue`: it
    /// is expanded into its longhands when parsed (see
    /// [`crate::rule::expand_shorthand_into`]), so few values hold it.
    ListStyle(Box<ListStyleShorthand>),
    /// `counter-reset: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited. The spec's initial value is `none` (CSS Lists 3 §4.1), which
    /// this implementation represents as an empty list.
    /// The integer defaults to 0 if omitted (as specified).
    ///
    /// The [`Arc<Vec<..>>`] wrapper makes cloning a cascade winner (the
    /// `value.clone()` in `apply_winners`'s drain) and cloning during an inheritance
    /// walk (`stack.push((child, computed.clone()))` and
    /// `out[idx] = computed.clone()` in `walk_from`) **shallow (only an
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
    /// The inherited `hanging-punctuation` keyword set, initially `none`.
    /// First/last and the mutually exclusive end mode are retained for layout.
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
    /// (the inline engine's line options in raikiri-dom) applies them to the
    /// first line or to every line.
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
    /// when the declaration is parsed, and the element cascade only ever reads
    /// those expanded declarations. This gives the 1/2/3/4 expansion and
    /// follows CSS Cascading L4 §3 "Shorthand Properties"
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
    /// [`Self::PaddingLeft`]/[`Self::PaddingRight`] longhands when the
    /// declaration is parsed, and the element cascade only reads those. If it
    /// does reach
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
    MarginTopInherit,
    /// `margin-right: <length-percentage> | auto` — non-inherited, initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>).
    MarginRight(LengthOrAuto),
    /// Page-context-only marker for `margin-right: inherit`.
    MarginRightInherit,
    /// `margin-bottom: <length-percentage> | auto` — non-inherited, initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>).
    MarginBottom(LengthOrAuto),
    /// Page-context-only marker for `margin-bottom: inherit`.
    MarginBottomInherit,
    /// `margin-left: <length-percentage> | auto` — non-inherited, initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>).
    MarginLeft(LengthOrAuto),
    /// Page-context-only marker for `margin-left: inherit`.
    MarginLeftInherit,
    /// Page-context-only marker for the `margin: inherit` shorthand. It is
    /// expanded into four side markers before page-context cascade.
    MarginInherit,
    /// `margin: <'margin-top'>{1,4}` shorthand — sets all four sides
    /// (CSS Box 3 §3.2 <https://www.w3.org/TR/css-box-3/#margin-shorthand>).
    ///
    /// **This variant is not observed during element cascade**:
    /// [`crate::rule::expand_shorthand_into`] expands it into the four longhand
    /// variants ([`MarginTop`](Self::MarginTop) / [`MarginRight`](Self::MarginRight) /
    /// [`MarginBottom`](Self::MarginBottom) / [`MarginLeft`](Self::MarginLeft))
    /// when the declaration is parsed, and the element cascade only reads
    /// those expanded declarations. This follows the 1/2/3/4-value expansion
    /// in spec §3.2 and CSS Cascading L4 §3, "Shorthand Properties"
    /// <https://www.w3.org/TR/css-cascade-4/#shorthand>:
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
    /// variants (four sides × three sub-properties) when the declaration is
    /// parsed, and the element cascade only reads
    /// those expanded declarations. This follows CSS Cascading
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
    /// `border-top: <line-width> || <line-style> || <color>` — single-side shorthand
    /// for the three top-side longhands (CSS Backgrounds 3 §3.4
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>).
    ///
    /// The `||` grammar, omitted-component initial fill, and declaration-ordering
    /// contract match [`Self::Border`]; only the expansion target differs (three
    /// top-side longhands instead of twelve). See [`crate::rule::expand_border_top`].
    BorderTop(Border),
    /// `border-right: <line-width> || <line-style> || <color>` — single-side shorthand
    /// for the three right-side longhands (CSS Backgrounds 3 §3.4
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>).
    ///
    /// The `||` grammar, omitted-component initial fill, and declaration-ordering
    /// contract match [`Self::Border`]; only the expansion target differs (three
    /// right-side longhands instead of twelve). See [`crate::rule::expand_border_right`].
    BorderRight(Border),
    /// `border-bottom: <line-width> || <line-style> || <color>` — single-side shorthand
    /// for the three bottom-side longhands (CSS Backgrounds 3 §3.4
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>).
    ///
    /// The `||` grammar, omitted-component initial fill, and declaration-ordering
    /// contract match [`Self::Border`]; only the expansion target differs (three
    /// bottom-side longhands instead of twelve). See [`crate::rule::expand_border_bottom`].
    BorderBottom(Border),
    /// `border-left: <line-width> || <line-style> || <color>` — single-side shorthand
    /// for the three left-side longhands (CSS Backgrounds 3 §3.4
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>).
    ///
    /// The `||` grammar, omitted-component initial fill, and declaration-ordering
    /// contract match [`Self::Border`]; only the expansion target differs (three
    /// left-side longhands instead of twelve). See [`crate::rule::expand_border_left`].
    BorderLeft(Border),
    /// `border: <css-wide-keyword>` — expands to twelve longhand CSS-wide markers
    /// (see [`CssWideKeyword`]).
    BorderCssWide(CssWideKeyword),
    /// `border-top: <css-wide-keyword>` — expands to three top-side longhand
    /// CSS-wide markers (see [`CssWideKeyword`]).
    BorderTopCssWide(CssWideKeyword),
    /// `border-right: <css-wide-keyword>` — expands to three right-side longhand
    /// CSS-wide markers (see [`CssWideKeyword`]).
    BorderRightCssWide(CssWideKeyword),
    /// `border-bottom: <css-wide-keyword>` — expands to three bottom-side longhand
    /// CSS-wide markers (see [`CssWideKeyword`]).
    BorderBottomCssWide(CssWideKeyword),
    /// `border-left: <css-wide-keyword>` — expands to three left-side longhand
    /// CSS-wide markers (see [`CssWideKeyword`]).
    BorderLeftCssWide(CssWideKeyword),
    /// `border-top-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderTopWidthCssWide(CssWideKeyword),
    /// `border-right-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderRightWidthCssWide(CssWideKeyword),
    /// `border-bottom-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderBottomWidthCssWide(CssWideKeyword),
    /// `border-left-width: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderLeftWidthCssWide(CssWideKeyword),
    /// `border-top-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderTopStyleCssWide(CssWideKeyword),
    /// `border-right-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderRightStyleCssWide(CssWideKeyword),
    /// `border-bottom-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderBottomStyleCssWide(CssWideKeyword),
    /// `border-left-style: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderLeftStyleCssWide(CssWideKeyword),
    /// `border-top-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderTopColorCssWide(CssWideKeyword),
    /// `border-right-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderRightColorCssWide(CssWideKeyword),
    /// `border-bottom-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
    BorderBottomColorCssWide(CssWideKeyword),
    /// `border-left-color: <css-wide-keyword>` (see [`CssWideKeyword`]).
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
    /// [`OverflowY`](Self::OverflowY) when the declaration is parsed, and the
    /// element cascade only reads those (CSS Cascading L4 §3 "Shorthand Properties"
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
    /// four longhands [`TextDecorationLine`](Self::TextDecorationLine),
    /// [`TextDecorationThickness`](Self::TextDecorationThickness),
    /// [`TextDecorationStyle`](Self::TextDecorationStyle), and
    /// [`TextDecorationColor`](Self::TextDecorationColor) when the declaration
    /// is parsed, and the element cascade only reads those (per CSS Cascading
    /// L4 §3, "Shorthand Properties", <https://www.w3.org/TR/css-cascade-4/#shorthand>).
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
    /// retained independently on both axes through computed-value processing.
    BorderRadius(BorderRadius),
    /// `border-radius: inherit` — resolved from the parent computed corners.
    BorderRadiusInherit,
    /// `border-top-left-radius` with one or two `<length-percentage>` values.
    BorderRadiusTopLeft(CornerRadius<Length>),
    /// `border-top-right-radius` with one or two `<length-percentage>` values.
    BorderRadiusTopRight(CornerRadius<Length>),
    /// `border-bottom-right-radius` with one or two `<length-percentage>` values.
    BorderRadiusBottomRight(CornerRadius<Length>),
    /// `border-bottom-left-radius` with one or two `<length-percentage>` values.
    BorderRadiusBottomLeft(CornerRadius<Length>),
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
    /// `grid-area` shorthand — placement for one grid item. Shared behind an
    /// [`Arc`] so that it does not set the size of every `PropertyValue`: it
    /// reaches the cascade unexpanded, where a winning value is cloned for
    /// every element it applies to, and cloning an `Arc` only counts a
    /// reference.
    GridArea(Arc<GridAreaShorthand>),
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
    /// declaration-order section of [`PropertyKey`]). Boxed so that it does
    /// not set the size of every `PropertyValue`; being expanded when parsed,
    /// few values hold it.
    Background(Box<BackgroundShorthand>),
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
    /// `isolation` — **non-inherited**, initial: [`Isolation::Auto`] (CSS
    /// Compositing and Blending Level 1 §3.4.2; see [`Isolation`]). Appended
    /// as a new 1:1 disjoint field under the placement rule in [`PropertyKey`].
    Isolation(Isolation),
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
    /// into six grammar longhands and ten modeled reset-only subproperties:
    /// `font-kerning` / `font-language-override` / `font-optical-sizing` /
    /// `font-variant-east-asian` / `font-variant-emoji` /
    /// `font-variant-ligatures` / `font-variant-numeric` /
    /// `font-variant-position` / `font-variation-settings` /
    /// `font-feature-settings`.
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
    /// Marker for `text-decoration-thickness: inherit` on the element path.
    /// CSS Cascading 4 section 7.3 takes the parent computed value even for this
    /// non-inherited longhand when `inherit` is specified explicitly
    /// (CSS Text Decoration 4 `text-decoration-thickness` is non-inherited,
    /// initial `auto`). The cascade resolves this against the parent computed
    /// thickness before staging (see [`crate::cascade::apply_winners`]).
    TextDecorationThicknessInherit,
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
    TextSpacingShorthand(TextSpacingShorthand),
    /// CSS Text 4 `word-space-transform` — inherited, initial `none`.
    /// The value is preserved without adding text transformation behavior.
    /// Appended to preserve existing variant discriminants.
    WordSpaceTransform(WordSpaceTransform),
    /// `text-emphasis-style` — inherited, initial `none` (CSS Text Decoration 4).
    /// Shape/fill and string values are preserved as computed data.
    /// Appended to preserve existing variant tags.
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
    /// `column-fill: auto | balance | balance-all` — non-inherited, initial `balance`.
    /// CSS Multi-column Layout Module Level 1 §7.1
    /// (<https://www.w3.org/TR/css-multicol-1/#propdef-column-fill>).
    ColumnFill(ColumnFillValue),
    /// `column-span` — non-inherited, initial `none`.
    ColumnSpan(ColumnSpanValue),
    /// `font-feature-settings: normal | <feature-tag-value>#` — inherited,
    /// initial `normal`; carries explicit OpenType feature settings into the
    /// Shodo text shaper. Appended to preserve existing variant discriminants.
    FontFeatureSettings(FontFeatureSettings),
    /// `inline-size: auto | <length-percentage [0,∞]>` — logical preferred
    /// inline size (CSS Logical Properties 1 §4.1). The cascade resolves it to
    /// `width` or `height` according to the specified `writing-mode`.
    /// Appended to preserve existing variant discriminants.
    InlineSize(LengthOrAuto),
    /// `block-size: auto | <length-percentage [0,∞]>` — logical preferred
    /// block size (CSS Logical Properties 1 §4.1), resolved like
    /// [`Self::InlineSize`] onto the perpendicular physical axis.
    /// Appended to preserve existing variant discriminants.
    BlockSize(LengthOrAuto),
    /// `text-overflow: clip | ellipsis` — **non-inherited**, initial:
    /// [`TextOverflowValue::Clip`] (CSS Overflow 3 §5.1; see
    /// [`TextOverflowValue`]). Computed value: the specified keyword.
    /// Appended to preserve existing variant discriminants.
    TextOverflow(TextOverflowValue),
    /// Non-inherited `column-rule` shorthand; omitted components use their initial values.
    ColumnRule(Border),
    /// Non-inherited `column-rule-width`; resolves like a border width.
    ColumnRuleWidth(Length),
    /// Non-inherited `column-rule-style`; retains the specified border style.
    ColumnRuleStyle(BorderStyle),
    /// Non-inherited `column-rule-color`; initially `currentcolor`.
    ColumnRuleColor(BorderColor),
}

// Every declaration a rule tree holds carries one `PropertyValue`, and the
// cascade clones each winning value for the element it applies to, so its
// size is paid per declaration and per element. A larger variant payload
// should be boxed, or this bound raised together with a measurement.
const _: () = assert!(
    std::mem::size_of::<PropertyValue>() <= 72,
    "PropertyValue grew past 72 bytes: box the new variant's payload, or raise the bound with a cascade measurement"
);

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
    /// The all-properties rollback shorthand.
    All,
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
    // `border-top` single-side shorthand key (semantics on the matching
    // PropertyValue::BorderTop variant).
    BorderTop,
    // `border-right` single-side shorthand key (semantics on the matching
    // PropertyValue::BorderRight variant).
    BorderRight,
    // `border-bottom` single-side shorthand key (semantics on the matching
    // PropertyValue::BorderBottom variant).
    BorderBottom,
    // `border-left` single-side shorthand key (semantics on the matching
    // PropertyValue::BorderLeft variant).
    BorderLeft,
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
    // isolation / mix-blend-mode (CSS Compositing and Blending Level 1
    // §3.4.1/§3.4.2, semantics on the matching PropertyValue::Isolation /
    // PropertyValue::MixBlendMode variants; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). Appended for the same
    // reason as background-repeat: new fields disjoint one-to-one from
    // existing fields.
    Isolation,
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
    // CSS Multi-column Layout Module Level 1 column-fill; appended to preserve existing key slots.
    ColumnFill,
    // CSS Fonts 4 font-feature-settings; appended to preserve existing key slots.
    FontFeatureSettings,
    // Logical preferred sizes; appended to preserve existing key slots.
    InlineSize,
    BlockSize,
    // CSS Overflow 3 text-overflow; appended to preserve existing key slots.
    TextOverflow,
    ListStyle,
    // Column rule keys are appended to preserve existing key slots.
    ColumnRule,
    ColumnRuleWidth,
    ColumnRuleStyle,
    ColumnRuleColor,
    // Appended to preserve existing property key slots.
    ColumnSpan,
}

impl PropertyValue {
    /// Returns the property key for this value.
    ///
    /// Used as the discriminant for selecting one winner per property in the
    /// cascade, and as the key in the `@page` cascade result map.
    pub fn key(&self) -> PropertyKey {
        match self {
            PropertyValue::AllRevertLayer => PropertyKey::All,
            PropertyValue::CustomProperty(_) => PropertyKey::Custom,
            PropertyValue::Deferred(value) => value.key,
            PropertyValue::ContextualColor(value) => value.key,
            PropertyValue::Grid(_) => PropertyKey::Grid,
            PropertyValue::GridArea(_) => PropertyKey::GridArea,
            PropertyValue::Color(_) => PropertyKey::Color,
            PropertyValue::BackgroundColor(_) => PropertyKey::BackgroundColor,
            PropertyValue::FontFamily(_) => PropertyKey::FontFamily,
            PropertyValue::FontSize(_) => PropertyKey::FontSize,
            // `larger` / `smaller` belong to the same property as `font-size`.
            // Map them to the same `PropertyKey` so cascade winner selection
            // correctly makes `font-size: 12px` and `font-size: larger` compete.
            // Separate keys would let both win, which the spec does not allow.
            PropertyValue::FontSizeRelative(_) => PropertyKey::FontSize,
            PropertyValue::FontWeight(_) => PropertyKey::FontWeight,
            PropertyValue::LineHeight(_) => PropertyKey::LineHeight,
            PropertyValue::Display(_) => PropertyKey::Display,
            PropertyValue::ListStyleType(_) => PropertyKey::ListStyleType,
            PropertyValue::ListStylePosition(_) => PropertyKey::ListStylePosition,
            PropertyValue::ListStyleImage(_) => PropertyKey::ListStyleImage,
            PropertyValue::ListStyle(_) => PropertyKey::ListStyle,
            PropertyValue::CounterReset(_) | PropertyValue::CounterResetInherit => {
                PropertyKey::CounterReset
            }
            PropertyValue::CounterIncrement(_) => PropertyKey::CounterIncrement,
            PropertyValue::CounterSet(_) => PropertyKey::CounterSet,
            PropertyValue::Content(_) => PropertyKey::Content,
            PropertyValue::StringSet(_) => PropertyKey::StringSet,
            PropertyValue::Position(_) => PropertyKey::Position,
            PropertyValue::Top(_) => PropertyKey::Top,
            PropertyValue::Right(_) => PropertyKey::Right,
            PropertyValue::Bottom(_) => PropertyKey::Bottom,
            PropertyValue::Left(_) => PropertyKey::Left,
            PropertyValue::TextAlign(_) => PropertyKey::TextAlign,
            PropertyValue::HangingPunctuation(_) => PropertyKey::HangingPunctuation,
            PropertyValue::TextIndent(_) => PropertyKey::TextIndent,
            PropertyValue::PaddingTop(_) => PropertyKey::PaddingTop,
            PropertyValue::PaddingRight(_) => PropertyKey::PaddingRight,
            PropertyValue::PaddingBottom(_) => PropertyKey::PaddingBottom,
            PropertyValue::PaddingLeft(_) => PropertyKey::PaddingLeft,
            PropertyValue::Padding(_) => PropertyKey::Padding,
            PropertyValue::PaddingInline(_) => PropertyKey::PaddingInline,
            PropertyValue::PaddingBlock(_) => PropertyKey::PaddingBlock,
            PropertyValue::MarginTop(_) | PropertyValue::MarginTopInherit => PropertyKey::MarginTop,
            PropertyValue::MarginRight(_) | PropertyValue::MarginRightInherit => {
                PropertyKey::MarginRight
            }
            PropertyValue::MarginBottom(_) | PropertyValue::MarginBottomInherit => {
                PropertyKey::MarginBottom
            }
            PropertyValue::MarginLeft(_) | PropertyValue::MarginLeftInherit => {
                PropertyKey::MarginLeft
            }
            PropertyValue::Margin(_) | PropertyValue::MarginInherit => PropertyKey::Margin,
            PropertyValue::MarginInline(_) => PropertyKey::MarginInline,
            PropertyValue::MarginBlock(_) => PropertyKey::MarginBlock,
            PropertyValue::BorderTopWidth(_) | PropertyValue::BorderTopWidthCssWide(_) => {
                PropertyKey::BorderTopWidth
            }
            PropertyValue::BorderRightWidth(_) | PropertyValue::BorderRightWidthCssWide(_) => {
                PropertyKey::BorderRightWidth
            }
            PropertyValue::BorderBottomWidth(_) | PropertyValue::BorderBottomWidthCssWide(_) => {
                PropertyKey::BorderBottomWidth
            }
            PropertyValue::BorderLeftWidth(_) | PropertyValue::BorderLeftWidthCssWide(_) => {
                PropertyKey::BorderLeftWidth
            }
            PropertyValue::BorderTopStyle(_) | PropertyValue::BorderTopStyleCssWide(_) => {
                PropertyKey::BorderTopStyle
            }
            PropertyValue::BorderRightStyle(_) | PropertyValue::BorderRightStyleCssWide(_) => {
                PropertyKey::BorderRightStyle
            }
            PropertyValue::BorderBottomStyle(_) | PropertyValue::BorderBottomStyleCssWide(_) => {
                PropertyKey::BorderBottomStyle
            }
            PropertyValue::BorderLeftStyle(_) | PropertyValue::BorderLeftStyleCssWide(_) => {
                PropertyKey::BorderLeftStyle
            }
            PropertyValue::BorderTopColor(_) | PropertyValue::BorderTopColorCssWide(_) => {
                PropertyKey::BorderTopColor
            }
            PropertyValue::BorderRightColor(_) | PropertyValue::BorderRightColorCssWide(_) => {
                PropertyKey::BorderRightColor
            }
            PropertyValue::BorderBottomColor(_) | PropertyValue::BorderBottomColorCssWide(_) => {
                PropertyKey::BorderBottomColor
            }
            PropertyValue::BorderLeftColor(_) | PropertyValue::BorderLeftColorCssWide(_) => {
                PropertyKey::BorderLeftColor
            }
            PropertyValue::Border(_) | PropertyValue::BorderCssWide(_) => PropertyKey::Border,
            PropertyValue::BorderTop(_) | PropertyValue::BorderTopCssWide(_) => {
                PropertyKey::BorderTop
            }
            PropertyValue::BorderRight(_) | PropertyValue::BorderRightCssWide(_) => {
                PropertyKey::BorderRight
            }
            PropertyValue::BorderBottom(_) | PropertyValue::BorderBottomCssWide(_) => {
                PropertyKey::BorderBottom
            }
            PropertyValue::BorderLeft(_) | PropertyValue::BorderLeftCssWide(_) => {
                PropertyKey::BorderLeft
            }
            PropertyValue::BorderStyle(_) => PropertyKey::BorderStyle,
            PropertyValue::BorderWidth(_) => PropertyKey::BorderWidth,
            PropertyValue::BorderColor(_) => PropertyKey::BorderColor,
            PropertyValue::Width(_) => PropertyKey::Width,
            PropertyValue::Height(_) => PropertyKey::Height,
            PropertyValue::MaxWidth(_) => PropertyKey::MaxWidth,
            PropertyValue::MaxHeight(_) => PropertyKey::MaxHeight,
            PropertyValue::MinWidth(_) => PropertyKey::MinWidth,
            PropertyValue::MinHeight(_) => PropertyKey::MinHeight,
            PropertyValue::MinBlockSize(_) => PropertyKey::MinBlockSize,
            PropertyValue::TextUnderlineOffset(_) => PropertyKey::TextUnderlineOffset,
            PropertyValue::TextAutospace(_) => PropertyKey::TextAutospace,
            PropertyValue::BoxSizing(_) => PropertyKey::BoxSizing,
            PropertyValue::Direction(_) => PropertyKey::Direction,
            PropertyValue::OverflowX(_) => PropertyKey::OverflowX,
            PropertyValue::OverflowY(_) => PropertyKey::OverflowY,
            PropertyValue::Overflow(_) => PropertyKey::Overflow,
            PropertyValue::TextDecorationLine(_) => PropertyKey::TextDecorationLine,
            PropertyValue::TextDecorationStyle(_) => PropertyKey::TextDecorationStyle,
            PropertyValue::TextDecorationColor(_) => PropertyKey::TextDecorationColor,
            PropertyValue::TextDecoration(_) => PropertyKey::TextDecoration,
            PropertyValue::VerticalAlign(_) => PropertyKey::VerticalAlign,
            PropertyValue::FontStyle(_) => PropertyKey::FontStyle,
            PropertyValue::TextTransform(_) => PropertyKey::TextTransform,
            PropertyValue::Visibility(_) => PropertyKey::Visibility,
            PropertyValue::ZIndex(_) => PropertyKey::ZIndex,
            PropertyValue::WordBreak(_) => PropertyKey::WordBreak,
            PropertyValue::OverflowWrap(_) => PropertyKey::OverflowWrap,
            PropertyValue::LetterSpacing(_) => PropertyKey::LetterSpacing,
            PropertyValue::WordSpacing(_) => PropertyKey::WordSpacing,
            PropertyValue::BreakBefore(_) => PropertyKey::BreakBefore,
            PropertyValue::BreakAfter(_) => PropertyKey::BreakAfter,
            PropertyValue::BreakInside(_) => PropertyKey::BreakInside,
            PropertyValue::Float(_) => PropertyKey::Float,
            PropertyValue::Clear(_) => PropertyKey::Clear,
            PropertyValue::WhiteSpace(_) => PropertyKey::WhiteSpace,
            PropertyValue::WhiteSpaceCollapse(_) => PropertyKey::WhiteSpaceCollapse,
            PropertyValue::TextWrap(_) => PropertyKey::TextWrap,
            PropertyValue::TextWrapShorthand(_) => PropertyKey::TextWrap,
            PropertyValue::TextWrapStyle(_) => PropertyKey::TextWrapStyle,
            PropertyValue::FlexDirection(_) => PropertyKey::FlexDirection,
            PropertyValue::FlexWrap(_) => PropertyKey::FlexWrap,
            PropertyValue::FlexGrow(_) => PropertyKey::FlexGrow,
            PropertyValue::FlexShrink(_) => PropertyKey::FlexShrink,
            PropertyValue::FlexBasis(_) => PropertyKey::FlexBasis,
            PropertyValue::Flex(_) => PropertyKey::Flex,
            PropertyValue::FlexFlow(_) => PropertyKey::FlexFlow,
            PropertyValue::Order(_) => PropertyKey::Order,
            PropertyValue::JustifyContent(_) => PropertyKey::JustifyContent,
            PropertyValue::AlignContent(_) => PropertyKey::AlignContent,
            PropertyValue::AlignItems(_) => PropertyKey::AlignItems,
            PropertyValue::AlignSelf(_) => PropertyKey::AlignSelf,
            PropertyValue::RowGap(_) => PropertyKey::RowGap,
            PropertyValue::ColumnGap(_) => PropertyKey::ColumnGap,
            PropertyValue::Gap(_) => PropertyKey::Gap,
            PropertyValue::PlaceContent(_) => PropertyKey::PlaceContent,
            PropertyValue::Hyphens(_) => PropertyKey::Hyphens,
            PropertyValue::TabSize(_) => PropertyKey::TabSize,
            PropertyValue::LineBreak(_) => PropertyKey::LineBreak,
            PropertyValue::TextJustify(_) => PropertyKey::TextJustify,
            PropertyValue::TextAlignAll(_) => PropertyKey::TextAlignAll,
            PropertyValue::TextAlignLast(_) => PropertyKey::TextAlignLast,
            PropertyValue::TextCombineUpright(_) => PropertyKey::TextCombineUpright,
            PropertyValue::TextOrientation(_) => PropertyKey::TextOrientation,
            PropertyValue::UnicodeBidi(_) => PropertyKey::UnicodeBidi,
            PropertyValue::FontVariantCaps(_) => PropertyKey::FontVariantCaps,
            PropertyValue::Quotes(_) => PropertyKey::Quotes,
            PropertyValue::TextShadow(_) => PropertyKey::TextShadow,
            PropertyValue::GridTemplateColumns(_) => PropertyKey::GridTemplateColumns,
            PropertyValue::GridTemplateRows(_) => PropertyKey::GridTemplateRows,
            PropertyValue::GridTemplateAreas(_) => PropertyKey::GridTemplateAreas,
            PropertyValue::GridAutoColumns(_) => PropertyKey::GridAutoColumns,
            PropertyValue::GridAutoRows(_) => PropertyKey::GridAutoRows,
            PropertyValue::GridAutoFlow(_) => PropertyKey::GridAutoFlow,
            PropertyValue::GridRowStart(_) => PropertyKey::GridRowStart,
            PropertyValue::GridRowEnd(_) => PropertyKey::GridRowEnd,
            PropertyValue::GridColumnStart(_) => PropertyKey::GridColumnStart,
            PropertyValue::GridColumnEnd(_) => PropertyKey::GridColumnEnd,
            PropertyValue::GridRow(_) => PropertyKey::GridRow,
            PropertyValue::GridColumn(_) => PropertyKey::GridColumn,
            PropertyValue::JustifyItems(_) => PropertyKey::JustifyItems,
            PropertyValue::JustifySelf(_) => PropertyKey::JustifySelf,
            PropertyValue::PlaceItems(_) => PropertyKey::PlaceItems,
            PropertyValue::PlaceSelf(_) => PropertyKey::PlaceSelf,
            PropertyValue::Orphans(_) => PropertyKey::Orphans,
            PropertyValue::Widows(_) => PropertyKey::Widows,
            PropertyValue::BorderRadius(_) | PropertyValue::BorderRadiusInherit => {
                PropertyKey::BorderRadius
            }
            PropertyValue::BorderRadiusTopLeft(_) => PropertyKey::BorderRadiusTopLeft,
            PropertyValue::BorderRadiusTopRight(_) => PropertyKey::BorderRadiusTopRight,
            PropertyValue::BorderRadiusBottomRight(_) => PropertyKey::BorderRadiusBottomRight,
            PropertyValue::BorderRadiusBottomLeft(_) => PropertyKey::BorderRadiusBottomLeft,
            PropertyValue::BoxShadow(_) => PropertyKey::BoxShadow,
            PropertyValue::Outline(_) => PropertyKey::Outline,
            PropertyValue::OutlineWidth(_) => PropertyKey::OutlineWidth,
            PropertyValue::OutlineStyle(_) => PropertyKey::OutlineStyle,
            PropertyValue::OutlineColor(_) => PropertyKey::OutlineColor,
            PropertyValue::OutlineOffset(_) => PropertyKey::OutlineOffset,
            PropertyValue::WritingMode(_) => PropertyKey::WritingMode,
            PropertyValue::RubyPosition(_) => PropertyKey::RubyPosition,
            PropertyValue::BackgroundRepeat(_) => PropertyKey::BackgroundRepeat,
            PropertyValue::BackgroundAttachment(_) => PropertyKey::BackgroundAttachment,
            PropertyValue::BackgroundClip(_) => PropertyKey::BackgroundClip,
            PropertyValue::BackgroundOrigin(_) => PropertyKey::BackgroundOrigin,
            PropertyValue::BackgroundSize(_) => PropertyKey::BackgroundSize,
            PropertyValue::BackgroundPosition(_) => PropertyKey::BackgroundPosition,
            PropertyValue::BackgroundImage(_) => PropertyKey::BackgroundImage,
            PropertyValue::Background(_) => PropertyKey::Background,
            PropertyValue::ObjectFit(_) => PropertyKey::ObjectFit,
            PropertyValue::ObjectPosition(_) => PropertyKey::ObjectPosition,
            PropertyValue::Opacity(_) => PropertyKey::Opacity,
            PropertyValue::Isolation(_) => PropertyKey::Isolation,
            PropertyValue::MixBlendMode(_) => PropertyKey::MixBlendMode,
            PropertyValue::MaskImage(_) => PropertyKey::MaskImage,
            PropertyValue::ClipPath(_) => PropertyKey::ClipPath,
            PropertyValue::Transform(_) => PropertyKey::Transform,
            PropertyValue::TransformOrigin(..) => PropertyKey::TransformOrigin,
            PropertyValue::Filter(_) => PropertyKey::Filter,
            PropertyValue::TableLayout(_) => PropertyKey::TableLayout,
            PropertyValue::BorderCollapse(_) => PropertyKey::BorderCollapse,
            PropertyValue::BorderSpacing(_) => PropertyKey::BorderSpacing,
            PropertyValue::CaptionSide(_) => PropertyKey::CaptionSide,
            PropertyValue::EmptyCells(_) => PropertyKey::EmptyCells,
            PropertyValue::Font(_) => PropertyKey::Font,
            PropertyValue::TextDecorationSkipInk(_) => PropertyKey::TextDecorationSkipInk,
            PropertyValue::TextDecorationSkipSpaces(_) => PropertyKey::TextDecorationSkipSpaces,
            PropertyValue::TextDecorationThickness(_)
            | PropertyValue::TextDecorationThicknessInherit => PropertyKey::TextDecorationThickness,
            PropertyValue::TextDecorationInset(_) => PropertyKey::TextDecorationInset,
            PropertyValue::TextEmphasisPosition(_) => PropertyKey::TextEmphasisPosition,
            PropertyValue::TextUnderlinePosition(_) => PropertyKey::TextUnderlinePosition,
            PropertyValue::Page(_) => PropertyKey::Page,
            PropertyValue::ColumnCount(_) => PropertyKey::ColumnCount,
            PropertyValue::ColumnWidth(_) => PropertyKey::ColumnWidth,
            PropertyValue::Columns(_) => PropertyKey::Columns,
            PropertyValue::ColumnFill(_) => PropertyKey::ColumnFill,
            PropertyValue::ColumnSpan(_) => PropertyKey::ColumnSpan,
            PropertyValue::HyphenateCharacter(_) => PropertyKey::HyphenateCharacter,
            PropertyValue::HyphenateLimitChars(_) => PropertyKey::HyphenateLimitChars,
            PropertyValue::TextSpacingTrim(_) => PropertyKey::TextSpacingTrim,
            PropertyValue::TextSpacingShorthand(_) => PropertyKey::TextSpacing,
            PropertyValue::WordSpaceTransform(_) => PropertyKey::WordSpaceTransform,
            PropertyValue::TextEmphasisStyle(_) => PropertyKey::TextEmphasisStyle,
            PropertyValue::TextEmphasisColor(_) => PropertyKey::TextEmphasisColor,
            PropertyValue::TextEmphasis(_) => PropertyKey::TextEmphasis,
            PropertyValue::FontKerning(_) => PropertyKey::FontKerning,
            PropertyValue::FontOpticalSizing(_) => PropertyKey::FontOpticalSizing,
            PropertyValue::FontVariantEmoji(_) => PropertyKey::FontVariantEmoji,
            PropertyValue::FontLanguageOverride(_) => PropertyKey::FontLanguageOverride,
            PropertyValue::FontVariantLigatures(_) => PropertyKey::FontVariantLigatures,
            PropertyValue::FontSynthesis(_) => PropertyKey::FontSynthesis,
            PropertyValue::FontVariantPosition(_) => PropertyKey::FontVariantPosition,
            PropertyValue::FontPalette(_) => PropertyKey::FontPalette,
            PropertyValue::FontVariantNumeric(_) => PropertyKey::FontVariantNumeric,
            PropertyValue::FontVariantEastAsian(_) => PropertyKey::FontVariantEastAsian,
            PropertyValue::FontVariationSettings(_) => PropertyKey::FontVariationSettings,
            PropertyValue::FontFeatureSettings(_) => PropertyKey::FontFeatureSettings,
            PropertyValue::InlineSize(_) => PropertyKey::InlineSize,
            PropertyValue::BlockSize(_) => PropertyKey::BlockSize,
            PropertyValue::TextOverflow(_) => PropertyKey::TextOverflow,
            PropertyValue::ColumnRule(_) => PropertyKey::ColumnRule,
            PropertyValue::ColumnRuleWidth(_) => PropertyKey::ColumnRuleWidth,
            PropertyValue::ColumnRuleStyle(_) => PropertyKey::ColumnRuleStyle,
            PropertyValue::ColumnRuleColor(_) => PropertyKey::ColumnRuleColor,
        }
    }
}

const DEFERRED_FUNCTIONS: [&str; 5] = ["var", "calc", "min", "max", "clamp"];
const MATH_FUNCTIONS: [&str; 4] = ["calc", "min", "max", "clamp"];

/// Shared upper bound for a deferred declaration value and every intermediate
/// string produced while substituting variables or simplifying math functions.
/// CSS Variables 1 §3.3 permits a UA-defined expansion limit; keeping the
/// bound in this module lets the declaration capture enforce it before making
/// an owned `SmolStr`.
pub(crate) const MAX_SUBSTITUTED_VALUE_BYTES: usize = 64 * 1024;

/// Maximum component-value nesting accepted by the deferred-value scanners.
/// The recursive parser paths use the same bound as a stack guard.
pub(crate) const MAX_DEFERRED_VALUE_NESTING_DEPTH: usize = 128;

// CSS Color 5's color endpoints and relative origins are recursive. Share
// this limit across every color function, not just color-mix(), to bound
// native recursion in declarations and direct color-parser callers.
pub(crate) const MAX_COLOR_MIX_NESTING_DEPTH: usize = 128;

fn is_deferred_function(name: &str) -> bool {
    DEFERRED_FUNCTIONS
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

/// Validate that math functions (calc/min/max/clamp) contain only syntactically
/// plausible inner tokens. `calc(foo)` has inner Ident(foo) which is not a
/// length/percentage/dimension, so it should be rejected as invalid parsing
/// rather than deferred. Valid examples like `calc(2em + 3ex)` or
/// `min(20px, 10px)` contain only Dimension/Percentage/Number and operators.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorMathType {
    Number,
    Percentage,
    Angle,
    Length,
    LengthPercentage,
    OtherDimension,
    Unknown,
    Invalid,
}

#[derive(Clone, Copy)]
pub(crate) enum MathTerm {
    Value(ColorMathType),
    Operator(char),
    Comma,
}

pub(crate) fn color_math_dimension_type(unit: &str) -> ColorMathType {
    if is_angle_unit(unit) {
        return ColorMathType::Angle;
    }
    if matches!(
        unit.to_ascii_lowercase().as_str(),
        "px" | "em"
            | "rem"
            | "ex"
            | "rex"
            | "ch"
            | "rch"
            | "cm"
            | "mm"
            | "q"
            | "in"
            | "pt"
            | "pc"
            | "vw"
            | "vh"
            | "vi"
            | "vb"
            | "vmin"
            | "vmax"
            | "lvw"
            | "lvh"
            | "lvi"
            | "lvb"
            | "lvmin"
            | "lvmax"
            | "svw"
            | "svh"
            | "svi"
            | "svb"
            | "svmin"
            | "svmax"
            | "dvw"
            | "dvh"
            | "dvi"
            | "dvb"
            | "dvmin"
            | "dvmax"
            | "lh"
            | "rlh"
            | "cap"
            | "rcap"
            | "ic"
            | "ric"
            | "cqw"
            | "cqh"
            | "cqi"
            | "cqb"
            | "cqmin"
            | "cqmax"
    ) {
        ColorMathType::Length
    } else {
        ColorMathType::OtherDimension
    }
}

fn color_math_add(left: ColorMathType, right: ColorMathType) -> ColorMathType {
    if matches!(left, ColorMathType::Invalid) || matches!(right, ColorMathType::Invalid) {
        return ColorMathType::Invalid;
    }
    if matches!(left, ColorMathType::Unknown) {
        return right;
    }
    if matches!(right, ColorMathType::Unknown) {
        return left;
    }
    if left == right {
        return left;
    }
    match (left, right) {
        (ColorMathType::Length, ColorMathType::Percentage)
        | (ColorMathType::Percentage, ColorMathType::Length)
        | (ColorMathType::LengthPercentage, ColorMathType::Length)
        | (ColorMathType::Length, ColorMathType::LengthPercentage)
        | (ColorMathType::LengthPercentage, ColorMathType::Percentage)
        | (ColorMathType::Percentage, ColorMathType::LengthPercentage) => {
            ColorMathType::LengthPercentage
        }
        _ => ColorMathType::Invalid,
    }
}

fn color_math_multiply(left: ColorMathType, right: ColorMathType) -> ColorMathType {
    if matches!(left, ColorMathType::Invalid) || matches!(right, ColorMathType::Invalid) {
        return ColorMathType::Invalid;
    }
    if matches!(left, ColorMathType::Unknown) {
        return right;
    }
    if matches!(right, ColorMathType::Unknown) {
        return left;
    }
    if left == ColorMathType::Number {
        return right;
    }
    if right == ColorMathType::Number {
        return left;
    }
    ColorMathType::Invalid
}

fn color_math_divide(left: ColorMathType, right: ColorMathType) -> ColorMathType {
    if matches!(left, ColorMathType::Invalid) || matches!(right, ColorMathType::Invalid) {
        return ColorMathType::Invalid;
    }
    if matches!(left, ColorMathType::Unknown) || matches!(right, ColorMathType::Unknown) {
        return ColorMathType::Unknown;
    }
    if right == ColorMathType::Number {
        return left;
    }
    ColorMathType::Invalid
}

pub(crate) struct MathTermsParser {
    terms: Vec<MathTerm>,
    index: usize,
}

impl MathTermsParser {
    pub(crate) fn new(terms: Vec<MathTerm>) -> Self {
        Self { terms, index: 0 }
    }

    fn peek(&self) -> Option<MathTerm> {
        self.terms.get(self.index).copied()
    }

    fn take(&mut self) -> Option<MathTerm> {
        let term = self.peek()?;
        self.index += 1;
        Some(term)
    }

    fn parse_primary(&mut self) -> ColorMathType {
        if matches!(self.peek(), Some(MathTerm::Operator('+' | '-'))) {
            self.take();
        }
        match self.take() {
            Some(MathTerm::Value(value)) => value,
            _ => ColorMathType::Invalid,
        }
    }

    fn parse_product(&mut self) -> ColorMathType {
        let mut value = self.parse_primary();
        while let Some(MathTerm::Operator(operator)) = self.peek() {
            if !matches!(operator, '*' | '/') {
                break;
            }
            self.take();
            let right = self.parse_primary();
            value = if operator == '*' {
                color_math_multiply(value, right)
            } else {
                color_math_divide(value, right)
            };
        }
        value
    }

    fn parse_sum(&mut self) -> ColorMathType {
        let mut value = self.parse_product();
        while let Some(MathTerm::Operator(operator)) = self.peek() {
            if !matches!(operator, '+' | '-') {
                break;
            }
            self.take();
            let right = self.parse_product();
            value = color_math_add(value, right);
        }
        value
    }

    pub(crate) fn parse_all(&mut self, allow_comma: bool) -> ColorMathType {
        if self.terms.is_empty() {
            return ColorMathType::Invalid;
        }
        let mut value = self.parse_sum();
        if matches!(self.peek(), Some(MathTerm::Comma)) {
            if !allow_comma {
                return ColorMathType::Invalid;
            }
            while matches!(self.peek(), Some(MathTerm::Comma)) {
                self.take();
                let right = self.parse_sum();
                value = color_math_add(value, right);
            }
        }
        if self.index != self.terms.len() {
            ColorMathType::Invalid
        } else {
            value
        }
    }
}

pub(crate) fn consume_math_component_values<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(), ParseError<'i, ()>> {
    loop {
        let token = match input.next() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        match token {
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                input.parse_nested_block(|nested| consume_math_component_values(nested))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn color_math_expression_type<'i>(
    input: &mut Parser<'i, '_>,
    allow_comma: bool,
) -> Result<ColorMathType, ParseError<'i, ()>> {
    let mut terms = Vec::new();
    loop {
        let token = match input.next() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        match token {
            Token::Number { .. } => terms.push(MathTerm::Value(ColorMathType::Number)),
            Token::Percentage { .. } => terms.push(MathTerm::Value(ColorMathType::Percentage)),
            Token::Dimension { ref unit, .. } => {
                terms.push(MathTerm::Value(color_math_dimension_type(unit.as_ref())))
            }
            Token::Ident(ref name) if is_math_constant(name.as_ref()) => {
                terms.push(MathTerm::Value(ColorMathType::Number));
            }
            Token::Ident(_) => terms.push(MathTerm::Value(ColorMathType::Invalid)),
            Token::Function(ref name) => {
                let name_lower = name.as_ref().to_ascii_lowercase();
                let function_type = if name_lower == "var" || name_lower == "env" {
                    input.parse_nested_block(|nested| {
                        consume_math_component_values(nested)?;
                        Ok::<_, ParseError<'_, ()>>(ColorMathType::Unknown)
                    })?
                } else {
                    let nested_allows_comma = matches!(
                        name_lower.as_str(),
                        "min" | "max" | "clamp" | "round" | "mod" | "rem" | "atan2"
                    );
                    let nested = input
                        .parse_nested_block(|nested| {
                            color_math_expression_type(nested, nested_allows_comma)
                        })
                        .unwrap_or(ColorMathType::Invalid);
                    if !relative_math_function(name_lower.as_ref()) {
                        ColorMathType::Invalid
                    } else {
                        match name_lower.as_str() {
                            "calc" | "min" | "max" | "clamp" | "abs" | "round" | "mod" | "rem" => {
                                nested
                            }
                            "sign" | "pow" | "sqrt" | "hypot" | "log" | "exp" | "sin" | "cos"
                            | "tan" | "asin" | "acos" | "atan" | "atan2" => {
                                if nested == ColorMathType::Invalid {
                                    ColorMathType::Invalid
                                } else {
                                    ColorMathType::Number
                                }
                            }
                            _ => ColorMathType::Invalid,
                        }
                    }
                };
                terms.push(MathTerm::Value(function_type));
            }
            Token::ParenthesisBlock => {
                let nested = input
                    .parse_nested_block(|nested| color_math_expression_type(nested, false))
                    .unwrap_or(ColorMathType::Invalid);
                terms.push(MathTerm::Value(nested));
            }
            Token::Delim('+' | '-' | '*' | '/') => {
                if let Token::Delim(operator) = token {
                    terms.push(MathTerm::Operator(operator));
                }
            }
            Token::Comma if allow_comma => terms.push(MathTerm::Comma),
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            _ => terms.push(MathTerm::Value(ColorMathType::Invalid)),
        }
    }
    Ok(MathTermsParser::new(terms).parse_all(allow_comma))
}

#[derive(Clone, Copy)]
pub(crate) enum ColorMathContext {
    Number,
    Percentage,
    NumberOrPercentage,
    NumberOrAngle,
}

fn color_math_type_allowed(value: ColorMathType, context: ColorMathContext) -> bool {
    matches!(value, ColorMathType::Unknown)
        || match context {
            ColorMathContext::Number => value == ColorMathType::Number,
            ColorMathContext::Percentage => value == ColorMathType::Percentage,
            ColorMathContext::NumberOrPercentage => {
                matches!(value, ColorMathType::Number | ColorMathType::Percentage)
            }
            ColorMathContext::NumberOrAngle => {
                matches!(value, ColorMathType::Number | ColorMathType::Angle)
            }
        }
}

pub(crate) fn parse_color_math_value<'i>(
    input: &mut Parser<'i, '_>,
    context: ColorMathContext,
) -> Result<(ColorMathType, f32), ParseError<'i, ()>> {
    input.skip_whitespace();
    let token = input.next()?.clone();
    let Token::Function(ref name) = token else {
        return Err(input.new_custom_error(()));
    };
    if !name.eq_ignore_ascii_case("calc") {
        return Err(input.new_custom_error(()));
    }
    let value = input.parse_nested_block(|nested| color_math_expression_type(nested, false))?;
    if color_math_type_allowed(value, context) {
        Ok((value, 0.0))
    } else {
        Err(input.new_custom_error(()))
    }
}

pub(crate) fn color_value_with_math_is_valid(value: &str) -> bool {
    let mut parser_input = ParserInput::new(value);
    let mut parser = Parser::new(&mut parser_input);
    parser
        .parse_entirely(|input| {
            parse_color(input).ok_or_else(|| input.new_custom_error::<(), ()>(()))
        })
        .is_ok()
}

pub(crate) fn math_function_syntax_is_valid(input: &str) -> bool {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    math_syntax_valid_in_parser(&mut parser, 0)
}

/// CSS Values 4 mathematical constants are identifiers inside a math
/// function, rather than ordinary `<number>` tokens. They still form valid
/// numeric expressions (`calc(infinity)`, `calc(-infinity)`, and
/// `calc(NaN)` are used by the CSS Color WPT), so the deferred syntax scanner
/// must distinguish them from an arbitrary invalid identifier such as `foo`.
pub(crate) fn is_math_constant(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "e" | "pi" | "infinity" | "-infinity" | "nan"
    )
}

fn math_syntax_valid_in_parser(parser: &mut Parser<'_, '_>, depth: usize) -> bool {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return false;
    }
    loop {
        let token = match parser.next() {
            Ok(t) => t.clone(),
            Err(_) => break,
        };
        match token {
            Token::Function(name) => {
                let is_math = MATH_FUNCTIONS.iter().any(|m| name.eq_ignore_ascii_case(m));
                let valid = parser
                    .parse_nested_block(|nested| {
                        if is_math {
                            // Inside math, check inner tokens are valid calc expression
                            Ok::<_, ParseError<'_, ()>>(math_calc_inner_is_valid(
                                nested,
                                depth.saturating_add(1),
                            ))
                        } else {
                            // Non-math function: recursively check inside
                            Ok::<_, ParseError<'_, ()>>(math_syntax_valid_in_parser(
                                nested,
                                depth.saturating_add(1),
                            ))
                        }
                    })
                    .unwrap_or(false);
                if !valid {
                    return false;
                }
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let valid = parser
                    .parse_nested_block(|nested| {
                        Ok::<_, ParseError<'_, ()>>(math_syntax_valid_in_parser(
                            nested,
                            depth.saturating_add(1),
                        ))
                    })
                    .unwrap_or(true);
                if !valid {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

fn math_calc_inner_is_valid(parser: &mut Parser<'_, '_>, depth: usize) -> bool {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return false;
    }
    let mut has_content = false;
    loop {
        let token = match parser.next() {
            Ok(t) => t.clone(),
            Err(_) => break,
        };
        has_content = true;
        match token {
            Token::Dimension { .. } | Token::Percentage { .. } | Token::Number { .. } => {}
            Token::Delim('+' | '-' | '*' | '/' | ',' | '(' | ')') => {}
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let valid = parser
                    .parse_nested_block(|nested| {
                        Ok::<_, ParseError<'_, ()>>(math_calc_inner_is_valid(
                            nested,
                            depth.saturating_add(1),
                        ))
                    })
                    .unwrap_or(false);
                if !valid {
                    return false;
                }
            }
            Token::Function(name) => {
                // Nested math functions are allowed (e.g., calc(calc(...)))
                let is_math = MATH_FUNCTIONS.iter().any(|m| name.eq_ignore_ascii_case(m));
                let valid = parser
                    .parse_nested_block(|nested| {
                        if is_math || !name.eq_ignore_ascii_case("var") {
                            // CSS math functions such as sign() have a
                            // numeric expression as their argument. `var()`
                            // is the exception: its custom-property syntax is
                            // not a math expression, but it is still valid in
                            // a deferred calc and must be consumed in full.
                            Ok::<_, ParseError<'_, ()>>(math_calc_inner_is_valid(
                                nested,
                                depth.saturating_add(1),
                            ))
                        } else {
                            while nested.next().is_ok() {}
                            Ok::<_, ParseError<'_, ()>>(true)
                        }
                    })
                    .unwrap_or(false);
                if !is_math && !valid {
                    return false;
                }
                // For math nested, if valid is false, fail
                if is_math && !valid {
                    return false;
                }
            }
            Token::Ident(ref name) if is_math_constant(name) => {}
            Token::Ident(_)
            | Token::IDHash(_)
            | Token::Hash(_)
            | Token::AtKeyword(_)
            | Token::UnquotedUrl(_) => {
                // Bare ident like `foo` inside calc is invalid
                return false;
            }
            Token::QuotedString(_)
            | Token::BadString(_)
            | Token::BadUrl(_)
            | Token::Colon
            | Token::Semicolon
            | Token::Comma
            | Token::IncludeMatch
            | Token::DashMatch
            | Token::PrefixMatch
            | Token::SuffixMatch
            | Token::SubstringMatch => {
                // These inside calc are invalid
                if !matches!(token, Token::Comma) {
                    return false;
                }
            }
            _ => {
                // Any other token considered invalid for calc inner
                return false;
            }
        }
    }
    has_content
}

/// Whether `value` contains any dimension or percentage token inside a
/// math function (`calc`/`min`/`max`/`clamp`).
///
/// CSS Values 4 calc type resolution: a math expression built only from
/// `<number>`s has type `<number>` and must not satisfy
/// `<length>`-expecting positions — WPT `flex: 1 2 calc(0)` (invalid,
/// number-typed calc as basis) vs `flex: calc(-1) calc(-1) 0` (valid,
/// number-typed calc as factors). [`deferred_dummy_is_valid_for_property`]
/// picks the dummy from this: `"1px"` when dimensions are present (current
/// behavior), `"1"` for pure-number math (so only `<number>` positions
/// validate).
pub(crate) fn math_source_has_dimension_or_percentage(input: &str) -> bool {
    // NOTE: no early `return true` anywhere in this walk — `parse_nested_block`
    // runs its closure via `parse_entirely`, which fails when the closure
    // leaves input unconsumed (e.g. returning at the first dimension of
    // `min(20px, 10px)` leaves `, 10px)` behind and the whole block scores
    // false). Accumulate into `found` and always walk to exhaustion.
    fn scan(parser: &mut Parser<'_, '_>, in_math: bool) -> bool {
        let mut found = false;
        loop {
            let token = match parser.next() {
                Ok(token) => token.clone(),
                Err(_) => break,
            };
            match token {
                Token::Dimension { .. } | Token::Percentage { .. } if in_math => {
                    found = true;
                }
                Token::Function(name) => {
                    let is_math = MATH_FUNCTIONS.iter().any(|m| name.eq_ignore_ascii_case(m));
                    let inner = parser
                        .parse_nested_block(|nested| {
                            Ok::<_, ParseError<'_, ()>>(scan(nested, in_math || is_math))
                        })
                        .unwrap_or(false);
                    found = found || inner;
                }
                Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                    let inner = parser
                        .parse_nested_block(|nested| {
                            Ok::<_, ParseError<'_, ()>>(scan(nested, in_math))
                        })
                        .unwrap_or(false);
                    found = found || inner;
                }
                _ => {}
            }
        }
        found
    }

    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    scan(&mut parser, false)
}

pub(crate) fn math_source_has_percentage(input: &str) -> bool {
    fn scan(parser: &mut Parser<'_, '_>, in_math: bool) -> bool {
        let mut found = false;
        loop {
            let token = match parser.next() {
                Ok(token) => token.clone(),
                Err(_) => break,
            };
            match token {
                Token::Percentage { .. } if in_math => found = true,
                Token::Function(name) => {
                    let is_math = MATH_FUNCTIONS.iter().any(|m| name.eq_ignore_ascii_case(m));
                    let inner = parser
                        .parse_nested_block(|nested| {
                            Ok::<_, ParseError<'_, ()>>(scan(nested, in_math || is_math))
                        })
                        .unwrap_or(false);
                    found = found || inner;
                }
                Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                    let inner = parser
                        .parse_nested_block(|nested| {
                            Ok::<_, ParseError<'_, ()>>(scan(nested, in_math))
                        })
                        .unwrap_or(false);
                    found = found || inner;
                }
                _ => {}
            }
        }
        found
    }

    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    scan(&mut parser, false)
}

pub(crate) fn deferred_dummy_is_valid_for_property(value: &str, prop: &str) -> bool {
    // Replace math functions with a dummy and check if the resulting value parses for the property.
    // This validates overall structure (e.g. `margin-top: calc(...) auto` has 2 tokens, invalid for longhand).
    //
    // Color components intentionally use more than one dummy type. A color
    // math expression can be a number, percentage, or angle depending on its
    // position. Looking only for dimensions anywhere in the expression is
    // incorrect for expressions such as `sign(1em - 10px) * 10%`: the nested
    // comparison has dimensions, but the result is a percentage. The ordinary
    // property path keeps the stricter type-aware check used by the rest of the
    // declaration parser; the color path tries representatives for the three
    // scalar forms accepted by the color grammars.
    let color_property = matches!(
        prop.to_ascii_lowercase().as_str(),
        "color" | "background-color"
    );
    let dummies: &[&str] = if color_property {
        &["1", "50%", "1deg"]
    } else if math_source_has_dimension_or_percentage(value) {
        &["1px"]
    } else {
        &["1"]
    };

    for dummy_value in dummies {
        let Some(dummy) = replace_math_with_dummy_and_number(value, dummy_value) else {
            continue;
        };
        let mut input = ParserInput::new(&dummy);
        let mut parser = Parser::new(&mut input);
        // Never re-enter deferred validation if an unsupported function remains.
        if contains_deferred_function(&mut parser) {
            continue;
        }
        if parse_value(prop, &mut parser).is_some() && parser.expect_exhausted().is_ok() {
            return true;
        }
    }
    false
}

/// Replace real math function tokens while preserving all other source bytes.
fn replace_math_with_dummy_and_number(input: &str, dummy: &str) -> Option<String> {
    fn replace<'i>(
        parser: &mut Parser<'i, '_>,
        dummy: &str,
        depth: usize,
        copied_until: &mut SourcePosition,
        result: &mut String,
    ) -> Result<(), ParseError<'i, ()>> {
        if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
            return Err(parser.new_custom_error(()));
        }
        loop {
            let start = parser.position();
            let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
                break;
            };
            match token {
                Token::Function(name)
                    if MATH_FUNCTIONS.iter().any(|f| name.eq_ignore_ascii_case(f)) =>
                {
                    result.push_str(parser.slice(*copied_until..start));
                    // Separators keep adjacent identifiers and numbers from merging
                    // with the replacement into a different CSS token.
                    result.push(' ');
                    result.push_str(dummy);
                    result.push(' ');
                    parser.parse_nested_block(|nested| {
                        while nested.next_including_whitespace_and_comments().is_ok() {}
                        Ok::<_, ParseError<'_, ()>>(())
                    })?;
                    *copied_until = parser.position();
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    parser.parse_nested_block(|nested| {
                        replace(nested, dummy, depth + 1, copied_until, result)
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    let mut copied_until = parser.position();
    let mut result = String::with_capacity(input.len());
    replace(&mut parser, dummy, 0, &mut copied_until, &mut result).ok()?;
    result.push_str(parser.slice(copied_until..parser.position()));
    Some(result)
}

pub(crate) fn contains_deferred_function(input: &mut Parser<'_, '_>) -> bool {
    let start = input.state();
    let source_start = input.position();
    let found = parser_contains_deferred_function(input, source_start, 0).unwrap_or(true);
    input.reset(&start);
    found
}

#[cfg(test)]
pub(crate) fn contains_deferred_function_in_source(input: &str) -> bool {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return true;
    }
    let source_start = parser.position();
    parser_contains_deferred_function(&mut parser, source_start, 0).unwrap_or(true)
}

/// Inspect CSS component-value tokens, including nested blocks, so a deferred
/// function is recognized only when cssparser emitted a real `Function` token.
/// Raw substring matching would mistake `#var(--x)` for a variable function.
fn parser_contains_deferred_function<'i>(
    input: &mut Parser<'i, '_>,
    source_start: SourcePosition,
    depth: usize,
) -> Result<bool, ParseError<'i, ()>> {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return Err(input.new_custom_error(()));
    }
    let mut found = false;
    loop {
        let token = match input.next() {
            Ok(token) => token.clone(),
            Err(_) => {
                if input.slice(source_start..input.position()).len() > MAX_SUBSTITUTED_VALUE_BYTES {
                    found = true;
                }
                break;
            }
        };
        if input.slice(source_start..input.position()).len() > MAX_SUBSTITUTED_VALUE_BYTES {
            found = true;
        }
        match token {
            Token::Function(name) => {
                if is_deferred_function(name.as_ref()) {
                    found = true;
                }
                if input.parse_nested_block(|nested| {
                    parser_contains_deferred_function(nested, source_start, depth.saturating_add(1))
                })? {
                    found = true;
                }
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                if input.parse_nested_block(|nested| {
                    parser_contains_deferred_function(nested, source_start, depth.saturating_add(1))
                })? {
                    found = true;
                }
            }
            _ => {}
        }
    }
    Ok(found)
}

#[cfg(test)]
pub(crate) fn skip_deferred_string(input: &str, start: usize) -> Option<usize> {
    let quote = input.as_bytes()[start];
    let bytes = input.as_bytes();
    let mut position = start + 1;
    while position < bytes.len() {
        match bytes[position] {
            b'\\' => position = position.checked_add(2)?,
            byte if byte == quote => return Some(position + 1),
            _ => position += 1,
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn skip_deferred_comment(input: &str, start: usize) -> Option<usize> {
    input[start + 2..]
        .find("*/")
        .map(|offset| start + 2 + offset + 2)
}

/// Bound CSS component-value nesting using cssparser's token boundaries.
///
/// In particular, an unquoted `url-token` is one token: brackets and braces
/// in its payload are URL data, not nested component values.
pub(crate) fn css_component_values_are_bounded(input: &str) -> bool {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    css_component_values_are_bounded_in_parser(&mut parser, 0)
}

pub(crate) fn css_component_values_are_bounded_in_parser(
    input: &mut Parser<'_, '_>,
    depth: usize,
) -> bool {
    loop {
        let token = match input.next() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        if token.is_parse_error() {
            return false;
        }
        match token {
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                if depth >= MAX_DEFERRED_VALUE_NESTING_DEPTH {
                    return false;
                }
                let nested_bounded = input
                    .parse_nested_block(|nested| {
                        Ok::<_, ParseError<'_, ()>>(css_component_values_are_bounded_in_parser(
                            nested,
                            depth + 1,
                        ))
                    })
                    .ok()
                    .unwrap_or(false);
                if !nested_bounded {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// Consume a value containing a substitution/math function while leaving a
/// trailing `!important` for the declaration parser.
pub(crate) fn consume_deferred_value(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let start_state = input.state();
    let start = input.position();
    loop {
        if input.slice(start..input.position()).len() > MAX_SUBSTITUTED_VALUE_BYTES {
            input.reset(&start_state);
            return None;
        }
        let before_token = input.state();
        match input.next() {
            Ok(Token::Delim('!')) => {
                // `next` first skips the unread body of a preceding function or
                // block (the `--x)` of `var(--x) !important`), so the position
                // recorded before the call can still be inside that body. The
                // value is everything consumed up to and excluding this
                // one-byte `!`.
                let through_bang = input.slice_from(start);
                let value_source = through_bang.strip_suffix('!').unwrap_or(through_bang);
                input.reset(&before_token);
                input.skip_whitespace();
                let bang_state = input.state();
                let is_important = input
                    .try_parse(|parser| {
                        cssparser::parse_important(parser)?;
                        parser.expect_exhausted()
                    })
                    .is_ok();
                if is_important {
                    input.reset(&bang_state);
                    if value_source.len() > MAX_SUBSTITUTED_VALUE_BYTES {
                        input.reset(&start_state);
                        return None;
                    }
                    let value = crate::cascade::trim_css_whitespace_and_comments(value_source);
                    if !css_component_values_are_bounded(value) {
                        input.reset(&start_state);
                        return None;
                    }
                    return Some(value.into());
                }
                input.reset(&start_state);
                return None;
            }
            Ok(_) => {}
            Err(BasicParseError {
                kind: BasicParseErrorKind::EndOfInput,
                ..
            }) => {
                if input.slice(start..input.position()).len() > MAX_SUBSTITUTED_VALUE_BYTES {
                    input.reset(&start_state);
                    return None;
                }
                let value = crate::cascade::trim_css_whitespace_and_comments(
                    input.slice(start..input.position()),
                );
                if !css_component_values_are_bounded(value) {
                    input.reset(&start_state);
                    return None;
                }
                return Some(value.into());
            }
            // cov:ignore: cssparser's `Parser::next` only reports
            // EndOfInput as a basic error for a valid token stream.
            Err(_) => {
                input.reset(&start_state);
                return None;
            }
        }
    }
}

pub(crate) fn is_custom_property_name(name: &str) -> bool {
    name.len() > 2 && name.as_bytes().starts_with(b"--")
}

/// Return the known property key without parsing its value.
pub(crate) fn property_key_for_name(name: &str) -> Option<PropertyKey> {
    let normalized_name = name.to_ascii_lowercase();
    Some(match normalized_name.as_str() {
        "color" => PropertyKey::Color,
        "background-color" => PropertyKey::BackgroundColor,
        "font-family" => PropertyKey::FontFamily,
        "font-size" => PropertyKey::FontSize,
        "font-weight" => PropertyKey::FontWeight,
        "line-height" => PropertyKey::LineHeight,
        "display" => PropertyKey::Display,
        "list-style-type" => PropertyKey::ListStyleType,
        "list-style-position" => PropertyKey::ListStylePosition,
        "list-style-image" => PropertyKey::ListStyleImage,
        "list-style" => PropertyKey::ListStyle,
        "counter-reset" => PropertyKey::CounterReset,
        "counter-increment" => PropertyKey::CounterIncrement,
        "counter-set" => PropertyKey::CounterSet,
        "content" => PropertyKey::Content,
        "string-set" => PropertyKey::StringSet,
        "position" => PropertyKey::Position,
        "top" => PropertyKey::Top,
        "right" => PropertyKey::Right,
        "bottom" => PropertyKey::Bottom,
        "left" => PropertyKey::Left,
        "text-align" => PropertyKey::TextAlign,
        "hanging-punctuation" => PropertyKey::HangingPunctuation,
        "text-autospace" => PropertyKey::TextAutospace,
        "text-spacing-trim" => PropertyKey::TextSpacingTrim,
        "text-spacing" => PropertyKey::TextSpacing,
        "word-space-transform" => PropertyKey::WordSpaceTransform,
        "text-indent" => PropertyKey::TextIndent,
        "padding-top" => PropertyKey::PaddingTop,
        "padding-right" => PropertyKey::PaddingRight,
        "padding-bottom" => PropertyKey::PaddingBottom,
        "padding-left" => PropertyKey::PaddingLeft,
        "padding" => PropertyKey::Padding,
        // CSS Logical Properties and Values 1 §4.4 padding-inline-start/-end /
        // padding-block-start/-end — physically fixed-mapped onto the
        // matching padding-{left,right,top,bottom} key (`PropertyValue::PaddingInline`
        // doc's "Why the 8 longhands have no dedicated `PropertyValue` variants" section).
        "padding-inline-start" => PropertyKey::PaddingLeft,
        "padding-inline-end" => PropertyKey::PaddingRight,
        "padding-block-start" => PropertyKey::PaddingTop,
        "padding-block-end" => PropertyKey::PaddingBottom,
        "padding-inline" => PropertyKey::PaddingInline,
        "padding-block" => PropertyKey::PaddingBlock,
        "margin-top" => PropertyKey::MarginTop,
        "margin-right" => PropertyKey::MarginRight,
        "margin-bottom" => PropertyKey::MarginBottom,
        "margin-left" => PropertyKey::MarginLeft,
        "margin" => PropertyKey::Margin,
        // CSS Logical Properties and Values 1 §4.2 margin-inline-start/-end /
        // margin-block-start/-end — same physically fixed-mapped pattern as
        // padding-inline-*/padding-block-* above.
        "margin-inline-start" => PropertyKey::MarginLeft,
        "margin-inline-end" => PropertyKey::MarginRight,
        "margin-block-start" => PropertyKey::MarginTop,
        "margin-block-end" => PropertyKey::MarginBottom,
        "margin-inline" => PropertyKey::MarginInline,
        "margin-block" => PropertyKey::MarginBlock,
        "border-top-width" => PropertyKey::BorderTopWidth,
        "border-right-width" => PropertyKey::BorderRightWidth,
        "border-bottom-width" => PropertyKey::BorderBottomWidth,
        "border-left-width" => PropertyKey::BorderLeftWidth,
        "border-top-style" => PropertyKey::BorderTopStyle,
        "border-right-style" => PropertyKey::BorderRightStyle,
        "border-bottom-style" => PropertyKey::BorderBottomStyle,
        "border-left-style" => PropertyKey::BorderLeftStyle,
        "border-top-color" => PropertyKey::BorderTopColor,
        "border-right-color" => PropertyKey::BorderRightColor,
        "border-bottom-color" => PropertyKey::BorderBottomColor,
        "border-left-color" => PropertyKey::BorderLeftColor,
        "border-radius" => PropertyKey::BorderRadius,
        "border-top-left-radius" => PropertyKey::BorderRadiusTopLeft,
        "border-top-right-radius" => PropertyKey::BorderRadiusTopRight,
        "border-bottom-right-radius" => PropertyKey::BorderRadiusBottomRight,
        "border-bottom-left-radius" => PropertyKey::BorderRadiusBottomLeft,
        "border" => PropertyKey::Border,
        "border-top" => PropertyKey::BorderTop,
        "border-right" => PropertyKey::BorderRight,
        "border-bottom" => PropertyKey::BorderBottom,
        "border-left" => PropertyKey::BorderLeft,
        "border-style" => PropertyKey::BorderStyle,
        "border-width" => PropertyKey::BorderWidth,
        "border-color" => PropertyKey::BorderColor,
        "width" => PropertyKey::Width,
        "height" => PropertyKey::Height,
        "inline-size" => PropertyKey::InlineSize,
        "block-size" => PropertyKey::BlockSize,
        "text-overflow" => PropertyKey::TextOverflow,
        "max-width" => PropertyKey::MaxWidth,
        "max-height" => PropertyKey::MaxHeight,
        "min-width" => PropertyKey::MinWidth,
        "min-height" => PropertyKey::MinHeight,
        "min-block-size" => PropertyKey::MinBlockSize,
        "text-underline-offset" => PropertyKey::TextUnderlineOffset,
        "box-sizing" => PropertyKey::BoxSizing,
        "direction" => PropertyKey::Direction,
        "overflow-x" => PropertyKey::OverflowX,
        "overflow-y" => PropertyKey::OverflowY,
        "overflow" => PropertyKey::Overflow,
        "text-decoration-line" => PropertyKey::TextDecorationLine,
        "text-decoration-style" => PropertyKey::TextDecorationStyle,
        "text-decoration-color" => PropertyKey::TextDecorationColor,
        "text-decoration" => PropertyKey::TextDecoration,
        "vertical-align" => PropertyKey::VerticalAlign,
        "font-style" => PropertyKey::FontStyle,
        "font-kerning" => PropertyKey::FontKerning,
        "font-optical-sizing" => PropertyKey::FontOpticalSizing,
        "font-variant-emoji" => PropertyKey::FontVariantEmoji,
        "font-language-override" => PropertyKey::FontLanguageOverride,
        "font-variant-ligatures" => PropertyKey::FontVariantLigatures,
        "font-synthesis" => PropertyKey::FontSynthesis,
        "font-variant-position" => PropertyKey::FontVariantPosition,
        "font-palette" => PropertyKey::FontPalette,
        "font-variant-numeric" => PropertyKey::FontVariantNumeric,
        "font-variant-east-asian" => PropertyKey::FontVariantEastAsian,
        "font-variation-settings" => PropertyKey::FontVariationSettings,
        "font-feature-settings" => PropertyKey::FontFeatureSettings,
        "text-transform" => PropertyKey::TextTransform,
        "visibility" => PropertyKey::Visibility,
        "z-index" => PropertyKey::ZIndex,
        "word-break" => PropertyKey::WordBreak,
        "overflow-wrap" | "word-wrap" => PropertyKey::OverflowWrap,
        "letter-spacing" => PropertyKey::LetterSpacing,
        "word-spacing" => PropertyKey::WordSpacing,
        "break-before" | "page-break-before" => PropertyKey::BreakBefore,
        "break-after" | "page-break-after" => PropertyKey::BreakAfter,
        "break-inside" | "page-break-inside" => PropertyKey::BreakInside,
        "float" => PropertyKey::Float,
        "clear" => PropertyKey::Clear,
        "white-space" => PropertyKey::WhiteSpace,
        "white-space-collapse" => PropertyKey::WhiteSpaceCollapse,
        "text-wrap" | "text-wrap-mode" => PropertyKey::TextWrap,
        "text-wrap-style" => PropertyKey::TextWrapStyle,
        "flex-direction" => PropertyKey::FlexDirection,
        "flex-wrap" => PropertyKey::FlexWrap,
        "flex-grow" => PropertyKey::FlexGrow,
        "flex-shrink" => PropertyKey::FlexShrink,
        "flex-basis" => PropertyKey::FlexBasis,
        "flex" => PropertyKey::Flex,
        "flex-flow" => PropertyKey::FlexFlow,
        "order" => PropertyKey::Order,
        "justify-content" => PropertyKey::JustifyContent,
        "align-content" => PropertyKey::AlignContent,
        "align-items" => PropertyKey::AlignItems,
        "align-self" => PropertyKey::AlignSelf,
        "row-gap" => PropertyKey::RowGap,
        "column-gap" => PropertyKey::ColumnGap,
        "gap" => PropertyKey::Gap,
        "column-count" => PropertyKey::ColumnCount,
        "column-fill" => PropertyKey::ColumnFill,
        "column-span" => PropertyKey::ColumnSpan,
        "column-width" => PropertyKey::ColumnWidth,
        "columns" => PropertyKey::Columns,
        "column-rule" => PropertyKey::ColumnRule,
        "column-rule-width" => PropertyKey::ColumnRuleWidth,
        "column-rule-style" => PropertyKey::ColumnRuleStyle,
        "column-rule-color" => PropertyKey::ColumnRuleColor,
        "place-content" => PropertyKey::PlaceContent,
        "hyphens" => PropertyKey::Hyphens,
        "hyphenate-character" => PropertyKey::HyphenateCharacter,
        "hyphenate-limit-chars" => PropertyKey::HyphenateLimitChars,
        "tab-size" => PropertyKey::TabSize,
        "line-break" => PropertyKey::LineBreak,
        "text-justify" => PropertyKey::TextJustify,
        "text-align-all" => PropertyKey::TextAlignAll,
        "text-align-last" => PropertyKey::TextAlignLast,
        "text-combine-upright" => PropertyKey::TextCombineUpright,
        "text-orientation" => PropertyKey::TextOrientation,
        "unicode-bidi" => PropertyKey::UnicodeBidi,
        "table-layout" => PropertyKey::TableLayout,
        "border-collapse" => PropertyKey::BorderCollapse,
        "border-spacing" => PropertyKey::BorderSpacing,
        "caption-side" => PropertyKey::CaptionSide,
        "empty-cells" => PropertyKey::EmptyCells,
        "font" => PropertyKey::Font,
        "text-decoration-skip-ink" => PropertyKey::TextDecorationSkipInk,
        "text-decoration-skip-spaces" => PropertyKey::TextDecorationSkipSpaces,
        "text-decoration-thickness" => PropertyKey::TextDecorationThickness,
        "text-decoration-inset" => PropertyKey::TextDecorationInset,
        "text-emphasis-position" => PropertyKey::TextEmphasisPosition,
        "text-underline-position" => PropertyKey::TextUnderlinePosition,
        "text-emphasis-style" => PropertyKey::TextEmphasisStyle,
        "text-emphasis-color" => PropertyKey::TextEmphasisColor,
        "text-emphasis" => PropertyKey::TextEmphasis,
        "page" => PropertyKey::Page,
        "font-variant-caps" => PropertyKey::FontVariantCaps,
        "quotes" => PropertyKey::Quotes,
        "text-shadow" => PropertyKey::TextShadow,
        "box-shadow" => PropertyKey::BoxShadow,
        "outline" => PropertyKey::Outline,
        "outline-width" => PropertyKey::OutlineWidth,
        "outline-style" => PropertyKey::OutlineStyle,
        "outline-color" => PropertyKey::OutlineColor,
        "outline-offset" => PropertyKey::OutlineOffset,
        "grid-template-columns" => PropertyKey::GridTemplateColumns,
        "grid-template-rows" => PropertyKey::GridTemplateRows,
        "grid-template-areas" => PropertyKey::GridTemplateAreas,
        "grid-auto-columns" => PropertyKey::GridAutoColumns,
        "grid-auto-rows" => PropertyKey::GridAutoRows,
        "grid-auto-flow" => PropertyKey::GridAutoFlow,
        "grid-row-start" => PropertyKey::GridRowStart,
        "grid-row-end" => PropertyKey::GridRowEnd,
        "grid-row" => PropertyKey::GridRow,
        "grid-column-start" => PropertyKey::GridColumnStart,
        "grid-column-end" => PropertyKey::GridColumnEnd,
        "grid-column" => PropertyKey::GridColumn,
        "justify-items" => PropertyKey::JustifyItems,
        "justify-self" => PropertyKey::JustifySelf,
        "place-items" => PropertyKey::PlaceItems,
        "place-self" => PropertyKey::PlaceSelf,
        "orphans" => PropertyKey::Orphans,
        "widows" => PropertyKey::Widows,
        "writing-mode" => PropertyKey::WritingMode,
        "ruby-position" => PropertyKey::RubyPosition,
        "background-repeat" => PropertyKey::BackgroundRepeat,
        "background-attachment" => PropertyKey::BackgroundAttachment,
        "background-clip" => PropertyKey::BackgroundClip,
        "background-origin" => PropertyKey::BackgroundOrigin,
        "background-size" => PropertyKey::BackgroundSize,
        "background-position" => PropertyKey::BackgroundPosition,
        "background-image" => PropertyKey::BackgroundImage,
        "background" => PropertyKey::Background,
        "object-fit" => PropertyKey::ObjectFit,
        "object-position" => PropertyKey::ObjectPosition,
        "opacity" => PropertyKey::Opacity,
        "isolation" => PropertyKey::Isolation,
        "mix-blend-mode" => PropertyKey::MixBlendMode,
        "mask-image" => PropertyKey::MaskImage,
        "clip-path" => PropertyKey::ClipPath,
        "transform" => PropertyKey::Transform,
        "transform-origin" => PropertyKey::TransformOrigin,
        "filter" => PropertyKey::Filter,
        _ => return None,
    })
}
