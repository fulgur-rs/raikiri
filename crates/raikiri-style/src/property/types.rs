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

/// 空 `<content-list>` を表す shared Arc — cascade で全 node が持ちうる
/// initial / inherit_from の default 値を per-node 新規 allocate せず、
/// 単一 heap slot を bump-share するための helper。
///
/// cascade memory DoS 対策として `ComputedValues.content` / `.string_set` は
/// `Arc<Vec<..>>` に wrap したが、`Arc::new(Vec::new())` を every node で呼ぶと
/// N-node document あたり 2N の small heap allocation regression になる。
/// `OnceLock` で **process 全体で 1 個** の empty Arc を保持し、
/// [`empty_content_list`] / [`empty_string_set_entries`] が各 initial spot で
/// clone (Arc reference-count increment only) する。
///
/// 空 `Vec::new()` は allocation 0 だが `Vec` struct 自体の 24 bytes が per-node
/// に生まれる — Arc 化により 8-byte pointer に置き換わり、指す先は shared。
pub(crate) fn empty_content_list() -> Arc<Vec<ContentComponent>> {
    static EMPTY: OnceLock<Arc<Vec<ContentComponent>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// `string-set` entry の 1 要素 — `(<custom-ident> name, content-list)` pair
/// を owned Vec で保持。alias 化により clippy::type_complexity を satisfy し、
/// 下段の `Arc<Vec<StringSetEntry>>` shape を局所化する。
pub(crate) type StringSetEntry = (SmolStr, Vec<ContentComponent>);

/// 空 `string-set` entries を表す shared Arc — [`empty_content_list`] と同じ
/// pattern (per-node empty allocation regression 回避)。
pub(crate) fn empty_string_set_entries() -> Arc<Vec<StringSetEntry>> {
    static EMPTY: OnceLock<Arc<Vec<StringSetEntry>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// 空 `counter-*` entries を表す shared Arc — 3 property
/// (`counter-reset` / `counter-increment` / `counter-set`) 全てで単一 slot を
/// 共有する ([`Vec<(SmolStr, i32)>`] は同一型のため helper を分ける必要無し)。
///
/// cascade memory DoS 対策の副作用 helper。
/// counter-* は non-inherited (CSS Lists 3 §4、`counter-reset` を含む全 3 property)
/// のため、`SpecifiedValues::inherit_from` が child stack entry のたびに empty 値で
/// 初期化する。生 `Vec::new()` を使うと per-node で 3 個の `Vec` struct
/// (24 bytes × 3) が生まれ N-node document あたり O(N) の overhead になるため、
/// [`empty_content_list`] / [`empty_string_set_entries`] と同じ `OnceLock` 保持の
/// shared Arc を使う。
pub(crate) fn empty_counter_entries() -> Arc<Vec<(SmolStr, i32)>> {
    static EMPTY: OnceLock<Arc<Vec<(SmolStr, i32)>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// 空 `quotes` entries を表す shared Arc — `none` の parse 結果、および
/// (宣言なしの) initial value の両方がこの 1 slot を共有する
/// ([`empty_counter_entries`] と同じ `OnceLock` 保持の shared-slot pattern)。
///
/// spec 上 `quotes` の initial value は "depends on user agent" (CSS2 §12.3.1)
/// — 具体的な引用符文字列を規定しない。本実装は 独立実装方針 (他実装の UA
/// 既定値を持ち込まない) により、宣言が無い場合もこの空 list を initial 値として
/// 採る ([`PropertyValue::Quotes`] doc 参照)。
pub(crate) fn empty_quotes_entries() -> Arc<Vec<(SmolStr, SmolStr)>> {
    static EMPTY: OnceLock<Arc<Vec<(SmolStr, SmolStr)>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// `font-family` の initial value を表す shared Arc — [`empty_content_list`]
/// 等と同じ `OnceLock` 保持の shared-slot pattern (同種の DoS 対策 fix の踏襲)。
///
/// CSS Fonts 4 §2.1 "Font Family: the font-family property"
/// (<https://www.w3.org/TR/css-fonts-4/#font-family-prop>) の spec 上の
/// initial は "depends on user agent" — spec は具体的な family name を規定
/// しない (`font-size` の initial `medium` の実 px が UA 依存であるのと同型、
/// [`crate::computed::INITIAL_FONT_SIZE_PX`] の doc 参照)。本実装は browser
/// default の `[Atom::from("serif")]` を採る。
///
/// `font-family` は **inherited** property であり、非 inherited な counter-* /
/// content / string-set と違って initial 値は空 list ではなく本実装が選んだ
/// `[Atom::from("serif")]` である。したがって本 helper は [`empty_content_list`]
/// のような「空 `Vec` を共有する」ものではなく、「initial 値そのものを共有する」
/// もの — root node の `SpecifiedValues::initial()` / `ComputedValues::initial()`
/// がこの単一 heap slot を bump-share する。
///
/// per-node cost の形は他 5 field (counter_reset 等) とは異なる —
/// あちらは「non-inherited property が毎 node で initial にリセットされる」
/// コストだったが、`font-family` は inherited なので「inheritance walk が
/// 毎 node で親の値を運ぶ」コスト
/// ([`crate::specified::SpecifiedValues::inherit_from`] の
/// `parent.font_family.clone()`) が主。値が initial の `serif` であろうと author
/// 指定の任意 list であろうと、`Arc` 化により `.clone()` は既存 Arc の bump に
/// なる — 本 helper は「initial 値を作る 1 箇所」を shared にするための slot
/// であって、inherit chain 上の非 initial 値までこの slot に強制する訳ではない。
pub(crate) fn initial_font_family() -> Arc<Vec<Atom>> {
    static INITIAL: OnceLock<Arc<Vec<Atom>>> = OnceLock::new();
    INITIAL
        .get_or_init(|| Arc::new(vec![Atom::from("serif")]))
        .clone()
}

/// 空 `text-shadow` list (`none`) を表す shared Arc — [`empty_content_list`]
/// 等と同じ `OnceLock` 保持の shared-slot pattern (per-node allocation
/// regression 回避)。`none` = 空 list という表現は `parse_content` /
/// `parse_counter_property` と同じ precedent
/// ([`TextShadowItem`] doc 参照)。
pub(crate) fn empty_text_shadow_list() -> Arc<Vec<TextShadowItem>> {
    static EMPTY: OnceLock<Arc<Vec<TextShadowItem>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// 空 `box-shadow` list (`none`) を表す shared Arc。
pub(crate) fn empty_box_shadow_list() -> Arc<Vec<BoxShadowItem>> {
    static EMPTY: OnceLock<Arc<Vec<BoxShadowItem>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// 空 `transform` list (`none`) を表す shared Arc — 同じ shared-slot pattern。
pub(crate) fn empty_transform_list() -> Arc<Vec<TransformFunction>> {
    static EMPTY: OnceLock<Arc<Vec<TransformFunction>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// 空 `filter` list (`none`) を表す shared Arc — 同じ shared-slot pattern。
pub(crate) fn empty_filter_list() -> Arc<Vec<FilterFunction>> {
    static EMPTY: OnceLock<Arc<Vec<FilterFunction>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// RGBA color (0-255 per channel、`a` は 255 = fully opaque)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CssColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl CssColor {
    /// Opaque black — `<color>` initial value に相当。
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    /// Fully transparent — `background-color` initial value に相当。
    ///
    /// CSS Color 4 §6.3 "The transparent keyword"
    /// <https://www.w3.org/TR/css-color-4/#transparent-color>:
    /// "The keyword `transparent` specifies a transparent black; it is a
    /// shorthand for `rgba(0, 0, 0, 0)`". `background-color` の initial value は
    /// CSS Backgrounds 3 §2.2 <https://www.w3.org/TR/css-backgrounds-3/#background-color>
    /// で `transparent` と規定される。
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// hex-notation payload (leading `#` を除いた digit 列) を parse する。
    ///
    /// CSS Color 4 §5.2 "The RGB Hexadecimal Notations: `#RRGGBB`"
    /// <https://www.w3.org/TR/css-color-4/#hex-notation> の 4 form を受理:
    ///
    /// - **3-digit** `rgb`     → 各 nibble を duplicate → `#RRGGBB`, `a = 255`
    /// - **4-digit** `rgba`    → 3-digit と同じ duplicate + alpha 4-bit nibble
    /// - **6-digit** `rrggbb`  → `a = 255` (fully opaque)
    /// - **8-digit** `rrggbbaa` → 末尾 byte が alpha (0..=255)
    ///
    /// 短縮形 (3/4-digit) の "digit duplicate" は §5.2 verbatim:
    ///
    /// > This syntax is often explained by saying that it’s identical to a
    /// > 6-digit notation obtained by "duplicating" all of the digits. For
    /// > example, the notation #123 specifies the same color as the notation
    /// > #112233.
    ///
    /// 4-digit も同様 — §5.2 verbatim:
    ///
    /// > This is a shorter variant of the 8-digit notation, "expanded" in the
    /// > same way as the 3-digit notation is.
    ///
    /// 実装上は nibble `n` (0..=15) を `(n << 4) | n = n * 17` に展開する。
    ///
    /// # Case
    ///
    /// `0-9` / `a-f` / `A-F` を受理 (ASCII case-insensitive)。§5.2 verbatim:
    ///
    /// > the case of the letters doesn’t matter - #00ff00 is identical to
    /// > #00FF00
    ///
    /// # Invalid input
    ///
    /// 他 length (0/1/2/5/7/9+) や non-hex byte を含む場合は `None` を返す
    /// (spec-invalid → drop)。leading `#` は tokenizer
    /// (`Token::Hash`) 側で剥がされて渡ってくるため、本 helper は expect しない
    /// (parser 経由でない直接呼び出しは caller 責務で `#` を落とすこと)。
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
            // 0/1/2/5/7/9+ digit は §5.2 hex-notation grammar に無い spec-invalid。
            _ => None,
        }
    }
}

/// ASCII hex digit (`0-9` / `a-f` / `A-F`) を 0..=15 の nibble へ変換。
/// case-insensitive per CSS Color 4 §5.2。non-hex → `None`。
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 2 桁 hex byte を組み立てる。`hi` / `lo` それぞれの nibble を [`hex_digit`]
/// で validate し、`(hi << 4) | lo` に合成する。どちらか non-hex なら `None`。
fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_digit(hi)? << 4) | hex_digit(lo)?)
}

/// 4-bit nibble `n` (`0..=15`) を 8-bit channel `nn` に展開する。
/// `(n << 4) | n = n * 17` — CSS Color 4 §5.2 の "duplicating" all of the
/// digits を実装した short-form 展開 helper (`#f` → `0xff`, `#8` → `0x88`、
/// verbatim 引用は [`CssColor::from_hex`] doc の 2 件を参照)。
fn expand_hex_nibble(n: u8) -> u8 {
    (n << 4) | n
}

/// CSS length or length-percentage value (box model 実装の足がかりとなる author CSS 型).
///
/// 各 variant は authored value (raw number as written) を保持する。sibling arm
/// convention: [`Length::Px`] が `Px(16.0)` = `16px` の pattern を確立、
/// 他 variant も authored value をそのまま保持する (`Em(1.2)` = `1.2em`、
/// `Percent(50.0)` = `50%` の literal 数字を格納)。
///
/// # 本型は「specified 層」を意味しない — 層は出所で決まる
///
/// element 経路では絶対化の結果が [`crate::resolve`] の `Computed*` 型になるので
/// 本型 = specified 層で読んでよい。**page 経路は違う** —
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// は `PropertyValue` の bag なので computed 値も本型で運ばれる。したがって
/// 「`Length` が見えたから未解決」と判断してはならない。
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
///   `1pt = 1/72 in`, CSS で `1in = 96px` の pixel-relative absolute unit。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Absolute pixel length。`10px` → `Px(10.0)`。
    Px(f32),
    /// Font-relative length: `em` — 使用要素の computed `font-size` に対する倍率。
    /// `1.2em` → `Em(1.2)`。Resolve は下流 (font-size stack を辿る)。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#em>).
    Em(f32),
    /// Font-relative length: `rem` — root element の computed `font-size` に対する倍率。
    /// `1rem` → `Rem(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rem>).
    Rem(f32),
    /// Percentage — `<length-percentage>` 文脈で reference value に対する比率。
    /// `50%` → `Percent(50.0)` (authored number をそのまま格納、divide-by-100 なし)。
    ///
    /// Spec: CSS Values 4 §5.5 Percentages
    /// (<https://www.w3.org/TR/css-values-4/#percentages>).
    Percent(f32),
    /// Absolute length: `pt` — 1pt = 1/72 in, CSS で 1in = 96px。
    /// `12pt` → `Pt(12.0)`、resolve 時 `12 * 96 / 72 = 16px` 相当。
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>).
    Pt(f32),
    /// Font-relative length: `ex` — 使用要素の font の x-height に対する倍率。
    /// `1ex` → `Ex(1.0)`。
    ///
    /// raikiri-style は style 層で実 font metrics を持たない (font shaping は
    /// downstream) ため、spec の unknown-metric fallback が常に適用される —
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ex>) verbatim: "In the cases
    /// where it is impossible or impractical to determine the x-height, a
    /// value of 0.5em must be assumed." Resolve は `0.5 * font-size`。
    Ex(f32),
    /// Font-relative length: `rex` — root element の `ex` (root font の
    /// x-height fallback) に対する倍率。`1rex` → `Rex(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rex>) — "Equal to the value of
    /// the ex unit on the root element." [`Length::Ex`] と同じ fallback
    /// (`0.5em`) を root font-size 基準で適用する。
    Rex(f32),
    /// Font-relative length: `ch` — 使用要素の font の "0" (U+0030) glyph の
    /// advance measure に対する倍率。`1ch` → `Ch(1.0)`。
    ///
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ch>) verbatim: "In the cases
    /// where it is impossible or impractical to determine the measure of the
    /// '0' glyph, it must be assumed to be 0.5em wide by 1em tall. Thus, the ch
    /// unit falls back to 0.5em in the general case, and to 1em when it
    /// would be typeset upright (i.e. writing-mode is vertical-rl or
    /// vertical-lr and text-orientation is upright)." raikiri-style は
    /// `writing-mode` の縦書きレンダリングパイプライン (と `text-orientation`
    /// 自体) を未実装なので、computed 値は常に `HorizontalTb` に正規化され
    /// upright 分岐は到達不能 — resolve は常に `0.5 * font-size`。縦書き
    /// レンダリング実装時に本判断の見直しが要る。
    Ch(f32),
    /// Font-relative length: `rch` — root element の `ch` に対する倍率。
    /// `1rch` → `Rch(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rch>) — "Equal to the value of
    /// the ch unit on the root element." [`Length::Ch`] と同じ fallback
    /// (`0.5em`、upright 分岐は同様に到達不能) を root font-size 基準で適用する。
    Rch(f32),
    /// Font-relative length: `ic` — 使用要素の font の CJK water ideograph
    /// (U+6C34) glyph の advance measure に対する倍率。`1ic` → `Ic(1.0)`。
    ///
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ic>) verbatim: "In the cases
    /// where it is impossible or impractical to determine the ideographic
    /// advance measure, it must be assumed to be 1em." resolve は
    /// `1.0 * font-size` (real metrics 同様の理由で常に fallback、
    /// [`Length::Ex`] doc 参照)。
    Ic(f32),
    /// Font-relative length: `ric` — root element の `ic` に対する倍率。
    /// `1ric` → `Ric(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#ric>) — "Equal to the value of
    /// the ic unit on the root element." [`Length::Ic`] と同じ fallback
    /// (`1em`) を root font-size 基準で適用する。
    Ric(f32),
    /// Absolute length: `cm` — centimeter。`1cm` → `Cm(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) 換算表
    /// verbatim: "1cm = 96px/2.54"。
    Cm(f32),
    /// Absolute length: `mm` — millimeter。`1mm` → `Mm(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) 換算表
    /// verbatim: "1mm = 1/10th of 1cm"。
    Mm(f32),
    /// Absolute length: `Q` — quarter-millimeter。`1Q` → `Q(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) 換算表
    /// verbatim: "1Q = 1/40th of 1cm"。
    Q(f32),
    /// Absolute length: `in` — inch。`1in` → `In(1.0)`、`96px` 相当。
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) 換算表
    /// verbatim: "1in = 2.54cm = 96px"。
    In(f32),
    /// Absolute length: `pc` — pica。`1pc` → `Pc(1.0)`、`16px` 相当。
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) 換算表
    /// verbatim: "1pc = 1/6th of 1in"。
    Pc(f32),
    /// Font-relative length: `lh` — 使用要素の computed `line-height` に対する
    /// 倍率。`1lh` → `Lh(1.0)`。
    ///
    /// CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#lh>) verbatim: "Equal to the
    /// computed value of the line-height property of the element on which it
    /// is used, converting normal to an absolute length by using only the
    /// metrics of the first available font."
    ///
    /// # `normal` の resolve — `cap`/`rcap` と同じ wall
    ///
    /// `normal` は `line-height` の **initial value** なので、この
    /// unknown-metric branch は edge case ではなく common case — real font
    /// instance が要る点は [`crate::resolve::used_line_height_length`] doc
    /// (cap/rcap と同じ wall) を参照。`ex`/`ch`/`ic` と違い spec は
    /// font-size 比のフォールバックを与えない (根拠なく比率を捏造しない、
    /// 独立実装方針) ため、resolve 側は「解決不能 → 消費 property の
    /// initial 相当」という per-property fallback を取る
    /// ([`crate::resolve`] の各 `resolve_*` 関数 doc 参照)。
    ///
    /// # 自己参照 (`line-height` 自身の値として使われる場合)
    ///
    /// `line-height: 1lh` は自己参照 (`lh` の素の定義 "the element on which
    /// it is used" が常に使用要素自身を指すため、**あらゆる要素**で自己参照
    /// になる) — spec 原文と `rlh` との非対称の判断根拠は
    /// [`crate::resolve::resolve_line_height`] doc が canonical
    /// (内容の重複による drift を避けるため、本節では要約に留め全文を
    /// 再掲しない)。
    /// `font-size: 1lh` も同条項の対象で自己参照になる (font-size は
    /// font-\* property) — 親の used line-height を基準に解決する
    /// ([`crate::resolve::resolve_font_size`] doc の
    /// 「`lh` / `rlh` の自己参照」節が canonical)。
    Lh(f32),
    /// Font-relative length: `rlh` — root element の `lh` に対する倍率。
    /// `1rlh` → `Rlh(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rlh>) — "Equal to the value of
    /// the lh unit on the root element."
    ///
    /// # `normal` wall — [`Length::Lh`] と共通
    ///
    /// root element の computed line-height が `normal` で解決不能なら
    /// `rlh` も解決不能になる — [`Length::Lh`] doc の「`normal` の resolve」
    /// 節と同じ wall (`cap`/`rcap` と同じ、real font instance が要る)。
    ///
    /// # `Length::Lh` と非対称 — 自己参照として扱わない
    ///
    /// `rlh` の素の定義は宣言要素の位置に依存しない tree-global な定数
    /// (root element の値を常に指す) であり、**`lh` と違って自己参照には
    /// ならない** — 循環が起こり得るのは宣言要素自身が root element の
    /// ときだけ ([`crate::specified::SpecifiedValues::finalize_as_root`] が
    /// カバーする「親が居ない」ケース、spec の "if the element has no
    /// parent" 節どおり initial values (`line-height: normal`) 基準になり
    /// 常に unresolved になる)。root **ではない**要素の `line-height: 1rlh`
    /// は既に確定済みの別 node (root) の値を参照するだけで自己参照では
    /// ないため、他の box property 上の `rlh` と同じ tree-global 基準
    /// (`ResolveContext::root_line_height`) を使う — 判断根拠の全文は
    /// [`crate::resolve::resolve_line_height`] doc 参照。
    ///
    /// `font-size: 1rlh` も [`Length::Lh`] doc の同節の対象だが、`rlh` は
    /// 上記の非対称により `font-size` 上でも自己参照として扱わない —
    /// 宣言要素が root element のとき以外は tree-global な
    /// `ResolveContext::root_line_height` を直接使う
    /// ([`crate::resolve::resolve_font_size`] doc 参照)。
    Rlh(f32),
}

impl Length {
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
            | Length::Rlh(v) => v,
        }
    }
}

/// `<length-percentage> | auto` — margin / width で共有される Author CSS seed
/// (margin longhand 用に導入し、後に `width` からも reuse)。
///
/// margin property は spec で `<length-percentage> | auto` を取る (CSS Box 3
/// §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。`auto` は spec
/// grammar top-level alternative として `<length-percentage>` と disjoint に
/// 現れるため、[`Length`] を包む sum type にする。同じ grammar shape は CSS
/// Sizing 3 §3.1.1 `width` / `height` の preferred-size にも現れる (`auto |
/// <length-percentage [0,∞]> | …`) ため、本 type を型 alias 相当で共有する。
/// **`Auto` variant の意味は property ごとに異なる** — margin は "distribute
/// available space"、width / height は "automatic size calculation" — variant
/// 側では意図的に property-agnostic に保ち、下流 layout / consumer 側で
/// property-specific に解釈する。
///
/// NB: padding (CSS Box 3 §4) の grammar は `<length-percentage>` のみで `auto`
/// を含まないため、padding は本 type を **使わず** [`Sides<Length>`] を
/// 直接使う (`Sides<T>` のみ reuse、詳細は [`Sides`] doc の再利用先 section)。
///
/// `#[non_exhaustive]` は future variant (例: `<flex>` `auto-vs-fill-available`
/// 系 CSS Box 4 拡張、または `min-content` / `max-content` 系 sizing keyword) の
/// non-breaking 追加のため — sibling [`Length`] / [`CounterStyle`] と同じ
/// pattern。
///
/// [`Copy`] 導入は underlying [`Length`] が `Copy` (Px/Em/Rem/Percent/Pt は
/// 全て単一 f32 payload) で、`Sides<LengthOrAuto>` = 4 × ~8 bytes に収まり
/// per-node copy が cheap なため。
///
/// # Primary sources
///
/// - CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties":
///   [`margin-*`](https://www.w3.org/TR/css-box-3/#margin-physical) —
///   "Value: `<length-percentage> | auto`" (top / right / bottom / left 共通)。
///   `auto` の resolution は下流 layout 責務 (margin auto = distribute
///   available space)。
/// - CSS Sizing 3 §3.1.1 "Preferred Size Properties":
///   [`width`](https://www.w3.org/TR/css-sizing-3/#preferred-size-properties)
///   — "Value: `auto | <length-percentage [0,∞]> | …`"。`auto` は automatic
///   size calculation (下流 layout 責務、margin の余白分配とは別意味)。
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

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthOrAuto {
    /// authored length-percentage (`10px` / `1em` / `50%` / etc.)。
    Length(Length),
    /// `auto` keyword。意味は consumer property 依存 — margin では
    /// "distribute available space" (CSS Box 3 §3.1)、width / height では
    /// "automatic size calculation" (CSS Sizing 3 §3.1.1)。variant 自体は
    /// property-agnostic に保ち、下流 layout が property-specific に解決する。
    Auto,
    /// A deferred mixed-unit `calc()` expression.
    Calc(CalcLengthPercentage),
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

/// `normal | <length>` を取る property の specified value —
/// [`letter-spacing`](PropertyValue::LetterSpacing) と
/// [`word-spacing`](PropertyValue::WordSpacing) で共有する
/// ([`LengthOrAuto`] が margin / width / height を横断して共有されるのと同じ
/// reuse pattern、同 type の doc 参照)。
///
/// 両 property とも spec 上 percentage を持たない (`Percentages: N/A`) ため、
/// `Length` 側の unit set はそのまま percentage を含む — percentage token は
/// **parse 段で reject** する ([`parse_letter_or_word_spacing`] が
/// `parse_length_value(input, false)` を使う)。
///
/// # Primary sources
///
/// - CSS Text 3 §7.1 "Word Spacing: the word-spacing property"
///   (<https://www.w3.org/TR/css-text-3/#word-spacing-property>): "Value:
///   `normal | <length>`"、"Percentages: N/A"。
/// - CSS Text 3 §7.2 "Tracking: the letter-spacing property"
///   (<https://www.w3.org/TR/css-text-3/#letter-spacing-property>): 同じ
///   `normal | <length>` grammar、同じく percentage 非対応。
///
/// `#[non_exhaustive]` は [`LengthOrAuto`] と同じ判断 — future variant を
/// non-breaking で追加できるようにする。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthOrNormal {
    /// authored length (`2px` / `-0.05em` / etc.) — 両 property とも spec が
    /// "Values may be negative, but there may be implementation-dependent
    /// limits." と明記するため、parse 段では sign を制限しない
    /// ([`margin-*`](LengthOrAuto) と同じ扱い、`padding` / `border-width` の
    /// non-negative constraint とは異なる)。
    Length(Length),
    /// `normal` keyword。CSS Text 3 §7.1 / §7.2 いずれも "No additional
    /// spacing is applied. Computes to zero." と定める — 絶対化
    /// ([`crate::resolve::resolve_length_or_normal`]) は常に
    /// [`crate::resolve::ComputedLength::ZERO`] に潰す。
    Normal,
}

/// 4-side box-model value holder。field 順は CSS Box 3 §4.2 shorthand の
/// 4-value form `top right bottom left` に一致 (clockwise from top)。
///
/// padding shorthand が最初の consumer、sibling の margin は
/// `Sides<LengthOrAuto>` として reuse する — 型パラメータで per-property の
/// value type 差を吸収する。
///
/// `Copy` は `where T: Copy` conditional bound として transparent に伝わり、
/// `Sides<Length>` の per-node write は bit-copy になる。
/// `Eq` derive は `T: Eq` conditional に伝わる (`Sides<Length>` / `Sides<LengthOrAuto>`
/// は共に inner が f32 を含むため実質 Eq にはならない — bound-伝播のみ、実 usage
/// は PartialEq)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sides<T> {
    /// Top side (`padding-top` / `margin-top` に相当)。
    pub top: T,
    /// Right side.
    pub right: T,
    /// Bottom side.
    pub bottom: T,
    /// Left side.
    pub left: T,
}

impl<T: Clone> Sides<T> {
    /// 4 side を全て同じ値で埋める constructor — `padding: 10px` / `margin: 10px`
    /// の 1-value shorthand expansion (CSS Box 3 §3.2 / §4.2 "If there is only
    /// one component value, it applies to all sides.") + `Sides` 系 field の
    /// spec initial value (`0` を全 side に配る) の両方で使う共通 helper。
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
    /// 4 side を独立に写像する。
    ///
    /// specified 層の `Sides<Length>` / `Sides<LengthOrAuto>` / `Sides<Border>` を
    /// computed 層の対応型へ絶対化する phase 3
    /// ([`crate::specified::SpecifiedValues::finalize`]) で使う。side ごとに
    /// 4 行書き下すのと等価だが、side の取り違え (`right` に `bottom` を書く等)
    /// を構造的に防ぐ。
    ///
    /// `pub(crate)` — 現状 consumer は crate 内の絶対化のみ。
    pub(crate) fn map<U>(self, mut f: impl FnMut(T) -> U) -> Sides<U> {
        Sides {
            top: f(self.top),
            right: f(self.right),
            bottom: f(self.bottom),
            left: f(self.left),
        }
    }
}

/// CSS Logical Properties and Values Level 1 の flow-relative 2-value
/// shorthand (`margin-inline` / `margin-block` / `padding-inline` /
/// `padding-block`、いずれも grammar `<'*-top'>{1,2}`) が共有する
/// start/end pair holder。[`Sides<T>`] (4-value box-model shorthand) の
/// 2-value sibling — 同じ理由 (型パラメータで margin の `LengthOrAuto` と
/// padding の `Length` の value type 差を吸収する) で generic 化する。
///
/// field 名は spec の `-start` / `-end` suffix (flow-relative、`top`/`right`/
/// `bottom`/`left` のような物理名ではない) にそのまま合わせる。本 crate での
/// 実際の物理 side への写像は固定 (raikiri は writing-mode: horizontal-tb +
/// direction: ltr を仮定する) — 詳細は
/// [`PropertyValue::MarginInline`] doc の Non-goal 節参照。
///
/// `Eq` derive は [`Sides<T>`] と同じく `T: Eq` conditional に伝わるのみ
/// (`StartEnd<Length>` / `StartEnd<LengthOrAuto>` は共に inner が f32 を含む
/// ため実質 Eq にはならない — 実 usage は `PartialEq`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartEnd<T> {
    /// `*-start` component (`margin-inline-start` / `margin-block-start` /
    /// `padding-inline-start` / `padding-block-start` に相当)。
    pub start: T,
    /// `*-end` component (`margin-inline-end` / `margin-block-end` /
    /// `padding-inline-end` / `padding-block-end` に相当)。
    pub end: T,
}

impl<T: Clone> StartEnd<T> {
    /// start/end を同じ値で埋める constructor — `margin-inline: <value>` /
    /// `margin-block: <value>` 等の 1-value shorthand expansion (CSS Logical
    /// Properties and Values 1 §4.2/§4.4 の 2-value grammar
    /// `<'margin-top'>{1,2}` / `<'padding-top'>{1,2}`... の "If only one
    /// value is given, it applies to both the start and end edges" 相当) で
    /// 使う共通 helper — [`Sides::all`] の 2-value 版。
    pub fn both(v: T) -> Self {
        Self {
            start: v.clone(),
            end: v,
        }
    }
}

impl<T> StartEnd<T> {
    /// start/end を独立に写像する — [`Sides::map`] の 2-value 版。
    /// `@page` cascade の phase 3 絶対化
    /// (`page.rs` の `absolutize_in_page_context`、module-private のため
    /// intra-doc link 不可) で使う (element cascade
    /// 側は shorthand PropertyValue が
    /// [`crate::specified::SpecifiedValues`] 上の
    /// field を持たないため、この shorthand 型自体には触れない — 詳細は
    /// [`PropertyValue::MarginInline`] doc の
    /// "element cascade 段でこの variant は観測されない" 節)。
    ///
    /// `pub(crate)` — 現状 consumer は crate 内の絶対化のみ ([`Sides::map`]
    /// と同じ可視性)。
    pub(crate) fn map<U>(self, mut f: impl FnMut(T) -> U) -> StartEnd<U> {
        StartEnd {
            start: f(self.start),
            end: f(self.end),
        }
    }
}

/// `border-style` の value — spec `<line-style>` production の 10 keyword。
///
/// CSS Backgrounds 3 §3.2 "Line Patterns: the border-style properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>:
/// Border の `<line-style> = none | hidden | dotted | dashed | solid | double |
/// groove | ridge | inset | outset` を表す。initial value は `none`、not
/// inherited (§3.2)。
///
/// UA stylesheet 差はあるが本 crate は implementation boundary のため
/// spec-defined 10 alternative のみ受理する。ASCII case-insensitive で
/// `parse_border_style_side` が ident と照合する (CSS Values 3 §3.1 "Pre-defined
/// Keywords" <https://www.w3.org/TR/css-values-3/#keywords>)。
///
/// # Non-goals
///
/// - **(b) 非対応**: paint side での visual 差 (double stroke / 3D
///   groove/ridge/inset/outset の shading) は paint scope の責務、cascade
///   static side では spec value を保持するのみ。
/// - **(a) spec-invalid**: 未知 keyword (`wavy` / `wave` 等 [`TextDecorationStyle`]
///   由来 keyword は本 property では invalid) は
///   `parse_border_style_side` が `None` を返し、declaration ごと drop。
///
/// `Default` は derive しない — 本 crate の convention は "derive `Default` iff
/// `.default()` が call される" (sibling [`DisplayValue`] / [`TextAlign`] と
/// 同じ、spec default は初期化側 [`crate::computed::ComputedValues::initial`]
/// が [`BorderStyle::None`] を直接指定する)。
///
/// `#[non_exhaustive]` は future variant (Draft CSS Backgrounds 4 拡張、または
/// author-defined `border-image` 相当の new line style) の non-breaking 追加のため —
/// sibling [`DisplayValue`] / [`TextAlign`] / [`Length`] と同 pattern。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    /// `none` — initial value。CSS Backgrounds 3 §3.2 verbatim: "No border.
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

/// `outline-style` の keyword payload。
///
/// CSS Basic User Interface Module Level 3 §4.3
/// <https://www.w3.org/TR/css-ui-3/#outline-style> の outline style grammar
/// (`auto | <border-style>`) を、border 用の [`BorderStyle`] から分離して表す。
/// `auto` は outline にだけ意味があり、[`BorderStyle`] には含まれない。
///
/// 本 crate の既存 outline scope は `hidden` を受理しないため、parser は
/// [`Self::Hidden`] を構築せず declaration を drop する。ただし enum には
/// `<border-style>` の全 keyword を保持できる形を残し、computed/specified の値
/// carrier が authored value を失わないようにする。
///
/// `Default` は derive しない。spec initial (`none`) は
/// [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] が明示的に設定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineStyle {
    /// `none` — initial value。
    None,
    /// `hidden` — parser scope では reject する outline value。
    Hidden,
    /// `dotted`。
    Dotted,
    /// `dashed`。
    Dashed,
    /// `solid`。
    Solid,
    /// `double`。
    Double,
    /// `groove`。
    Groove,
    /// `ridge`。
    Ridge,
    /// `inset`。
    Inset,
    /// `outset`。
    Outset,
    /// `auto` — UA-dependent automatic outline rendering.
    Auto,
}

/// `border-*-color` computed value — spec `currentcolor` keyword と resolved
/// `<color>` の specified-value distinction を cascade static side で保持する。
///
/// CSS Backgrounds 3 §3.1 "Line Colors: the border-color properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-color> — "Initial:
/// currentcolor" (initial value は `currentcolor` keyword、literal `<color>`
/// (`black` を含む) とは区別される)。
///
/// CSS Color 3 §4.4 "currentColor color keyword"
/// <https://www.w3.org/TR/css-color-3/#currentColor-def> — "The used value of
/// the `currentColor` keyword is the computed value of the `color` property"。
/// used-value resolution (currentcolor → 同 node の computed `color` property
/// lookup) は paint scope 責務 (border 描画実装との
/// 合流で end-to-end 疎通)。
///
/// # なぜ cascade static side で enum 保持するか (Option A / B の A 採用理由)
///
/// `SpecifiedValues::initial` と `SpecifiedValues::inherit_from`
/// (crate::specified module) は node の自 `color` declaration が cascade `apply_value` で書き込まれる
/// **前** に border 全 side を構築する。author `<div style="color:red">` で
/// border-color 省略 (initial 直行) の hazard case では、border-color が
/// `apply_value` を一切通らないため cascade 段で node 自 color を捕捉できない
/// (parent の color のみが inherit_from の入力になる)。Option B (cascade 段で
/// 事前 stored directly) は post-cascade resolution pass + sentinel 判別を要求し、
/// sentinel 自体が本 enum と等価になる — 本 crate の "per-longhand cascade は
/// declaration 順非依存" invariant (margin / padding precedent、`apply_value`
/// arm doc 群参照) も同時に破ることになる。Option A は specified value を
/// preserve して paint scope に resolution を委譲することで、両制約
/// (initial-path correctness + per-key determinism) を同時に満たす。
///
/// # `#[non_exhaustive]`
///
/// Sibling [`Length`] / [`LengthOrAuto`] / [`BorderStyle`] / [`Border`] と
/// 同 pattern — future variant 追加 (例: CSS Color 4 §6.2 "System Colors"
/// <https://www.w3.org/TR/css-color-4/#css-system-colors> の system-color keyword)
/// の forward-compat 契約 (sibling convention)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderColor {
    /// `currentcolor` keyword — border-*-color の spec-mandated initial value
    /// (CSS Backgrounds 3 §3.1)。used-value は paint scope で node の computed
    /// `color` property を lookup して確定する。
    CurrentColor,
    /// Resolved `<color>` value — author が hex / named / `rgb(a)` /
    /// `transparent` で明示指定した場合、または `border` / `border-color`
    /// shorthand から expand された場合の payload。
    Resolved(CssColor),
}

/// `border` — 3 sub-property を単一 side 分にまとめた intermediate 型。
///
/// CSS Backgrounds 3 §3 "Borders" の 3 sub-property を 1 side 分保持する:
/// - `width`: [`Length`] — `parse_border_width_side` が px keyword 変換 (thin/
///   medium/thick → 1/3/5 px) と length の non-negative check を担う。
/// - `style`: [`BorderStyle`] — `parse_border_style_side` が 10 alternative を
///   受理。
/// - `color`: [`BorderColor`] — `parse_border_color` (spec §3.1 の `<color>`
///   grammar に加え `currentcolor` keyword を先取り) が返す enum。initial
///   [`BorderColor::CurrentColor`] は paint scope が `color` property で
///   resolve する (`CssColor::BLACK` placeholder から格上げ、CSS Backgrounds 3
///   §3.1 の initial 契約準拠)。
///
/// # `<line-width>` keyword mapping (§3.3)
///
/// spec §3.3 "Line Thickness: the border-width properties" は
/// `<line-width> = <length [0,∞]> | thin | medium | thick`。thin=1px、
/// medium=3px、thick=5px は spec 規定値 (verbatim: "are equivalent to 1px,
/// 3px, and 5px, respectively")。詳細は `parse_border_width_side` doc 参照。
///
/// # `#[non_exhaustive]`
///
/// future field (例: CSS Backgrounds 4 の `border-image-*` cascade 統合、あるいは
/// per-side gradient support) の non-breaking 追加のため — sibling
/// [`Length`] / [`LengthOrAuto`] / [`BorderStyle`] と同 pattern。
///
/// # `Sides<Border>` 化
///
/// [`Sides<Border>`] として 4 side を保持する ([`Sides`] 型パラメータ — margin
/// `Sides<LengthOrAuto>` / padding `Sides<Length>` と同じ再利用先)。cascade は
/// per-side longhand を direct-write するため apply 順に依存せず、shorthand
/// `border: ...` は parse-time で 12 longhand (4 side × 3 sub-property) に
/// 展開される (spec CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> 準拠、margin precedent
/// の踏襲)。
///
/// # `Eq` non-derive rationale
///
/// [`Length`] は f32 payload (`Px(f32)` etc.) を持つため `Eq` を実装できず、
/// Border も PartialEq のみ (`Sides<Border>`: PartialEq が実質的 usage、`Sides` の
/// derive は `where T: Eq` conditional bound として transparent に伝わる)。
/// sibling [`Length`] / [`LengthOrAuto`] と同じ制約。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// border-width (CSS Backgrounds 3 §3.3)。initial `medium` = `Length::Px(3.0)`。
    /// grammar は `<length [0,∞]>` — non-negative 制約は
    /// `parse_border_width_side` が parse-time で enforce。`<percentage>` は spec
    /// に含まれない (padding とは違う grammar)。
    pub width: Length,
    /// border-style (CSS Backgrounds 3 §3.2)。initial `none`。
    pub style: BorderStyle,
    /// border-color (CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>)。
    /// initial は `currentcolor` keyword — [`BorderColor::CurrentColor`] を
    /// enum variant として保持し、used-value resolution (currentcolor →
    /// 同 node の computed `color` property) は paint scope で確定する。
    /// `CssColor` から [`BorderColor`] enum へ格上げ (spec initial 契約 fidelity)。
    pub color: BorderColor,
}

/// `Border` is non-exhaustive but provides a default constructor and public
/// fields so downstream callers can construct and then customize a value.
impl Border {
    /// CSS Backgrounds 3 の初期値
    /// (`width` = medium = 3px §3.3 / `style` = `none` §3.2 / `color` = `currentcolor` §3.1)
    /// を持つ `Border` を返す zero-arg constructor。`Self::default()` の thin
    /// wrapper — `raikiri_traits::page::PageBox::new` と同じ shape。
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

/// `font-weight` property の **specified** value。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// (<https://www.w3.org/TR/css-fonts-4/#font-weight-prop>)、value grammar
/// `<font-weight-absolute> | bolder | lighter`、
/// `<font-weight-absolute> = [ normal | bold | <number [1,1000]> ]`。
///
/// # なぜ specified / computed で型が分かれるか
///
/// `bolder` / `lighter` は **relative weight** で、spec §2.2.1 の table により
/// **継承値 (親の computed font-weight)** から絶対 weight を算出する。この
/// resolution は cascade context (親 node の computed value) を要するため
/// parse 段では解けない。そこで型を 2 段に分ける:
///
/// - **specified side** ([`FontWeightValue`]、本型) — `bolder` / `lighter` を
///   sentinel variant として保持する。
/// - **computed side** ([`crate::computed::ComputedValues::font_weight`]、`f32`)
///   — resolution 済みの absolute weight のみ。`bolder` / `lighter` は
///   [`crate::cascade::apply_value`] で解決されてから格納される。
///
/// この分離は spec 準拠でもある: §2.2 の property table は
/// `Computed value: a number, see below` と規定し、§2.2.1 "Relative Weights"
/// (<https://www.w3.org/TR/css-fonts-4/#relative-weights>) が "Specified values
/// of `bolder` and `lighter` indicate weights relative to the weight of the
/// parent element. The computed weight is calculated based on the inherited
/// `font-weight` value" と規定している。computed 側に relative keyword が
/// 残ることはない。
///
/// # Primary source
///
/// - CSS Fonts 4 §2.2 (<https://www.w3.org/TR/css-fonts-4/#font-weight-prop>)
/// - CSS Values 3 §3.1 "Pre-defined Keywords"
///   (<https://www.w3.org/TR/css-values-3/#keywords>) — keyword は ASCII
///   case-insensitive で照合
///
/// Downstream match は必ず wildcard arm を持つこと (`#[non_exhaustive]` 属性、
/// variant 追加が既存 pattern-match を break しない forward-compat 契約、sibling
/// [`LineHeight`] / [`DisplayValue`] と同 pattern)。
///
/// # `Eq` を derive しない
///
/// [`Absolute`](Self::Absolute) の payload が `f32` になった (旧 `u16`) ため
/// `Eq` は derive できない (`f32: !Eq`、NaN が反射性を満たさないため)。比較は
/// `PartialEq` (`==`) のみで足りる — `[1, 1000]` 範囲外に reject 済みで NaN /
/// ±inf は本 variant に到達しないので、実用上の比較は常に well-defined。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontWeightValue {
    /// `<font-weight-absolute>` — `normal` (400) / `bold` (700) /
    /// `<number [1,1000]>` を単一の絶対 weight に畳んだもの。CSS Fonts 4 §2.2.2
    /// "Missing weights" (<https://www.w3.org/TR/css-fonts-4/#missing-weights>)
    /// "Fractional weights are valid" どおり fraction を保持する `f32` (旧
    /// `u16`、fraction 保持のため格上げ — 詳細は [`parse_font_weight`] doc)。
    Absolute(f32),
    /// `bolder` — 継承値より 1 段太い weight。cascade 時に
    /// [`crate::cascade::resolve_relative_weight`] が spec §2.2.1 table で
    /// 絶対値に解決する。
    Bolder,
    /// `lighter` — 継承値より 1 段細い weight。解決タイミングは
    /// [`Bolder`](Self::Bolder) と同じ。
    Lighter,
}

/// `font-size: larger | smaller` (`<relative-size>`) の keyword。
/// [`PropertyValue::FontSizeRelative`] の payload。
///
/// CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop>。
/// 解決は [`crate::cascade::resolve_relative_font_size`] — [`FontWeightValue::Bolder`]
/// / [`FontWeightValue::Lighter`] と同型の、親の computed font-size に対する
/// read-modify-write。
///
/// [`FontWeightValue`] と異なり `raikiri` (umbrella) の `pub use` list には
/// 追加しない ([`PropertyValue::FontSizeRelative`] doc の「`Self::FontSize`
/// を再利用せず新 variant にした理由」節を参照)。
///
/// Downstream match は必ず wildcard arm を持つこと (`#[non_exhaustive]` 属性、
/// sibling [`FontWeightValue`] と同 pattern)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeFontSize {
    /// `larger` — 親の computed font-size より 1 段大きいサイズ。
    Larger,
    /// `smaller` — 親の computed font-size より 1 段小さいサイズ。
    Smaller,
}

/// `font-style` property の value。
///
/// CSS Fonts Module Level 4 §2.4 "Font style: the font-style property"
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>。
///
/// propdef (spec verbatim): Value: `normal | italic | left | right |
/// oblique <angle [-90deg,90deg]>?`、Initial: `normal`、Applies to: all
/// elements and text、Inherited: **yes**、Computed value: "the keyword
/// specified, plus angle in degrees if specified"。
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
/// - **部分対応**: 継承 property に必要な CSS-wide `inherit` は受理し、computed
///   層で親の値へ解決する。他の CSS-wide keyword (`initial` / `unset` /
///   `revert` / `revert-layer`) は未実装で silent drop (一覧・理由は
///   [`PropertyValue`] doc の「CSS-wide keyword」節が canonical)。
/// - **(a) spec-invalid**: 上記 5 keyword (`left` / `right` は上記 Non-goal
///   節参照) 以外の ident は silent drop = `None`。
///
/// With `oblique` accepted only as a bare keyword (no `<angle>` payload),
/// the angle-bearing branch of the spec's "Computed value" row stays
/// unreachable — so for this crate's scope, computed value = specified
/// keyword, no relative resolution needed ([`Direction`] doc と同型)。
///
/// [`Direction`] / [`BoxSizing`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`FontStyle::Normal`]
/// を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontStyle {
    /// `normal` — spec initial value。
    Normal,
    /// `italic`。
    Italic,
    /// `oblique` — bare keyword only, no `<angle>` payload (「Scope
    /// carving」節参照)。
    Oblique,
}

/// `font-variant-caps` property の value。
///
/// CSS Fonts Module Level 3 §6.6 "Capitalization: the font-variant-caps
/// property" <https://www.w3.org/TR/css-fonts-3/#font-variant-caps-prop>。
///
/// propdef (spec verbatim): Value: `normal | small-caps | all-small-caps |
/// petite-caps | all-petite-caps | unicase | titling-caps`、Initial:
/// `normal`、Applies to: all elements、Inherited: **yes**、Percentages: N/A、
/// Computed value: "as specified"。
///
/// # 7 keyword の意味 (spec 確認済み verbatim)
///
/// - [`Normal`](Self::Normal) — "None of the features listed below are
///   enabled." spec initial value。
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
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 7 keyword 以外の ident は silent drop = `None`。
///
/// この crate の scope では length を運ばないため、computed value = specified
/// keyword、相対解決なし ([`Direction`] doc と同型)。
///
/// # Downstream handoff
///
/// 各 keyword が指す OpenType feature (`smcp` / `c2sc` / `pcap` / `c2pc` /
/// `unic` / `titl`) の実際の glyph 差し替えは text-shaping/paint 層の責務であり、
/// 本 crate はそこへ渡す cascade static side の keyword を運ぶだけ
/// ([`TextTransform`] doc の「Downstream handoff」節と同型)。フォント側の
/// feature 非対応時のフォールバックも同じく downstream の責務。§6.6 は
/// `normal` 以外の 6 keyword 全てに対してこのフォールバックを規定しており、
/// 内容は keyword ごとに異なる:
///
/// - `small-caps` / `all-small-caps`: SHOULD-level の synthesis fallback
///   (spec verbatim: "if 'small-caps' or 'all-small-caps' is specified but
///   small-caps glyphs are not available for a given font, user agents
///   should simulate a small-caps font")。
/// - `petite-caps` / `all-petite-caps`: 対応 font が無い場合、それぞれ
///   `small-caps` / `all-small-caps` が指定されたのと同じ挙動になる (spec
///   verbatim: "If either 'petite-caps' or 'all-petite-caps' is specified
///   for a font that doesn't support these features, the property behaves
///   as if 'small-caps' or 'all-small-caps', respectively, had been
///   specified")。
/// - `unicase`: 対応 font が無い場合、小文字化された大文字にのみ
///   `small-caps` が適用されたのと同じ挙動になる (spec verbatim: "If 'unicase'
///   is specified for a font that doesn't support that feature, the
///   property behaves as if 'small-caps' was applied only to lowercased
///   uppercase letters")。
/// - `titling-caps`: 対応 font が無い場合、可視効果なし (spec verbatim: "If
///   'titling-caps' is specified with a font that does not support this
///   feature, this property has no visible effect")。
///
/// [`FontStyle`] / [`Direction`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が
/// [`FontVariantCaps::Normal`] を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontVariantCaps {
    /// `normal` — spec initial value。
    Normal,
    /// `small-caps`。
    SmallCaps,
    /// `all-small-caps`。
    AllSmallCaps,
    /// `petite-caps`。
    PetiteCaps,
    /// `all-petite-caps`。
    AllPetiteCaps,
    /// `unicase`。
    Unicase,
    /// `titling-caps`。
    TitlingCaps,
}

/// `text-transform` property の value。
///
/// CSS Text Module Level 3 §2.1 "Case Transforms: the text-transform
/// property" <https://www.w3.org/TR/css-text-3/#text-transform-property>。
///
/// propdef (spec verbatim): Value: `none | [capitalize | uppercase |
/// lowercase] || full-width || full-size-kana`、Initial: `none`、Applies to:
/// text、Inherited: **yes**、Computed value: "specified keyword"。
///
/// # Scope
///
/// The parser and layout pipeline preserve the optional width transforms and
/// the case transform as one computed keyword value. `full-width` maps ASCII
/// characters to their full-width forms; `full-size-kana` uses the CSS small
/// kana mapping table. Language-specific tailoring is applied by the layout
/// consumer where the document language is available.
///
/// `Default` is intentionally not derived; initialization sites explicitly
/// choose [`TextTransform::None`] as the initial value.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    /// `none` — spec initial value。
    None,
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

/// `visibility` property の value。
///
/// CSS Display 3 §4 "Invisibility: the visibility property"
/// <https://www.w3.org/TR/css-display-3/#visibility>。
///
/// propdef (spec verbatim): Value: `visible | hidden | collapse`、
/// Initial: `visible`、Applies to: all elements、Inherited: **yes**、
/// Computed value: "as specified"。
///
/// # Scope carving
///
/// - **`collapse`**: spec 本文はこの keyword について "can cause it to
///   take up less space than otherwise in a formatting-context–specific
///   way" と述べ、その space-saving 効果を table 行/列/行グループ/列グループ
///   (CSS2 dynamic row and column effects) と flex item
///   (CSS Flexbox 1 collapsed flex items) にだけ specific に定める。それ以外
///   では spec 自身が "this simply makes the box invisible, just like
///   `visibility: hidden`" と明記する。raikiri-style はこの space-saving 側の
///   layout 効果をどの formatting context に対しても実装しない — 本 crate が
///   運ぶのは computed value としての bare keyword のみで、上記の
///   formatting-context 固有な仕様は下流 (layout) の scope。将来その実装が
///   加わったときに `Hidden` との判別が要るため、`collapse` は `Hidden` に
///   畳み込まず独立 variant として保持する。
/// - **(b) 非対応**: CSS-wide keyword は未実装、silent drop ([`PropertyValue`]
///   doc の「CSS-wide keyword」節が canonical)。
/// - **(a) spec-invalid**: 上記 3 keyword 以外の ident は silent drop = `None`。
///
/// computed value = specified keyword (spec の "Computed value: as
/// specified" のとおり、相対解決なし、[`Direction`] doc と同型)。
///
/// [`Direction`] / [`FontStyle`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`Visibility::Visible`]
/// を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// `visible` — spec initial value。
    Visible,
    /// `hidden`。
    Hidden,
    /// `collapse` — [`Visibility`] doc の「Scope carving」節参照。
    Collapse,
}

/// `line-height` property の value (inline layout 実装の足がかりとなる author CSS 型)。
///
/// CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#line-height-property>)、value grammar
/// `normal | <number [0,∞]> | <length-percentage [0,∞]>`。
///
/// 3 variant で spec の top-level alternative を保持する:
///
/// - [`Normal`](Self::Normal) — spec initial value。resolve は下流 (paint) が
///   font metrics ascent+descent 相当を採用 (`parley` の default line-height 挙動)。
/// - [`Number`](Self::Number) — unitless multiplier。`line-height: 1.5` は
///   使用要素の computed `font-size` × 1.5。**spec special behavior**: unitless
///   number は **specified value を child が inherit する** (資源 resolve せず
///   raw multiplier を伝える) — cascade static side では raw value を保持し、
///   Length variant と別 variant にすることで number-vs-length semantics 差を
///   下流 (paint) が復元可能にする (§5.1 "When a child element inherits...")。
/// - [`Length`](Self::Length) — `<length-percentage>` payload。`Length::Percent`
///   の semantics は **percentage of the element's own font-size** (§5.1)。
///   `Length::Em`/`Rem`/`Px`/`Pt` は通常の length resolve context に従う。
///
/// # Non-negative constraint
///
/// spec grammar `<number [0,∞]>` / `<length-percentage [0,∞]>` により負値は
/// invalid → parser 側で drop (`parse_line_height` の post-filter、
/// spec-invalid → drop)。spec grammar が range を parse-time で制約している
/// ため、reject 自体が spec 準拠 (stricter ではなく match)。
///
/// # Primary source
///
/// - CSS Inline 3 §5.1 "Line Spacing: the line-height property"
///   (<https://www.w3.org/TR/css-inline-3/#line-height-property>) —
///   "specifies the box's preferred line height, which is used in calculating
///   its layout bounds"
///
/// Downstream match は必ず wildcard arm を持つこと (`#[non_exhaustive]` 属性、
/// 変数追加が既存 pattern-match を break しない forward-compat 契約、sibling
/// [`Length`] / [`DisplayValue`] と同 pattern)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    /// `normal` — spec initial value。paint 側が font metrics ascent+descent
    /// 相当の default line-height を採用する。
    Normal,
    /// `<number [0,∞]>` — unitless multiplier。`1.5` → `Number(1.5)`。
    /// resolve 時 使用要素の computed `font-size` × 本 value。
    /// number variant は spec 上 child が **specified value** を inherit する
    /// (Length variant と別扱いの load-bearing distinction)。
    Number(f32),
    /// `<length-percentage [0,∞]>` — length or percentage。
    /// `24px` → `Length(Length::Px(24.0))`、`150%` → `Length(Length::Percent(150.0))`。
    /// `Length::Percent` は spec §5.1 で「element's own font-size に対する比率」。
    Length(Length),
}

/// `<counter-style>` の parse 結果。
///
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>
/// で `counter()` / `counters()` の optional 第 3 引数、CSS Content 3 §2.6
/// で `target-counter()` / `target-counters()` の optional 末尾引数として現れる。
/// spec default = `decimal` (`counter-style?` omitted 時)。
///
/// static-side scope では named style を SmolStr で pass-through する
/// (`decimal-leading-zero`, `upper-alpha`, `lower-roman` 等の解釈は下流責務、
/// runtime resolve で counter tree を format する際に効く)。
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CounterStyle {
    /// `decimal` — spec default (`<counter-style>?` omitted も同一 variant)。
    #[default]
    Decimal,
    /// `decimal` 以外の named counter-style。値は case-preserved の smol str。
    Named(SmolStr),
}

/// `list-style-type` の computed value。
///
/// CSS Lists 3 §3.1 <https://www.w3.org/TR/css-lists-3/#list-style-type>。
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

/// `list-style-position` の computed value。
///
/// CSS Lists 3 §3.2 <https://www.w3.org/TR/css-lists-3/#list-style-position>。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListStylePosition {
    /// Marker is laid out outside the principal block.
    #[default]
    Outside,
    /// Marker participates in the first line of the principal block.
    Inside,
}

/// `string()` の第 2 引数 `[ first | start | last | first-except ]?`。
///
/// CSS Content 3 §2.7.2 "Inserting Named Strings: the string() function"
/// <https://www.w3.org/TR/css-content-3/#string-function>。
/// spec default = `first` — `first` dt/dd 本文 verbatim: "If no second
/// argument is provided, this is the default value."
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StringFetchMode {
    /// `first` — spec default。
    #[default]
    First,
    /// `start`。
    Start,
    /// `last`。
    Last,
    /// `first-except`。
    FirstExcept,
}

/// `target-text()` の第 2 引数 `[ content | before | after | first-letter ]?`。
///
/// CSS Content 3 §2.6.3 "The target-text() function"
/// <https://www.w3.org/TR/css-content-3/#target-text> の verbatim production:
/// `target-text() = target-text( [ <string> | <url> ] , [ content | before |
/// after | first-letter ]? )`。第 2 引数には `?` があり (GCPM 3 §1.1.1.1 の
/// `content()` とは異なり、構文上そのものが optional)。keyword の意味を
/// 述べる prose はこの 2 文のみ: "The target-text() function retrieves the
/// text value of the element referred to by the URL. An optional second
/// argument specifies what content is retrieved, using the same values as
/// the string-set property above." — 第 2 引数の keyword に対する dt/dd や "if
/// omitted" 文は存在しない (string() 関数の keyword 定義とは違う)。ただし
/// 第 1 文が述べる base behavior ("the text value of the element") は
/// [`Content`](Self::Content) の意味 (対象要素自身の string value) と一致
/// する。keyword 省略時に [`Content`](Self::Content) を採用する根拠はこの
/// semantic correspondence であり、spec が "default" と明言した文の
/// verbatim quote ではない (同種の overclaim を後で発見し、本 site を訂正済み)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentPart {
    /// `content` — 対象要素自身の string value。target-text() の prose が
    /// base behavior として述べる "the text value of the element" と対応
    /// する keyword (type-level doc 参照)。keyword 省略時のフォールバック値
    /// だが、根拠は spec の "default" 宣言ではない。
    #[default]
    Content,
    /// `before` — `::before` pseudo-element の string value。
    Before,
    /// `after` — `::after` pseudo-element の string value。
    After,
    /// `first-letter` — `::first-letter` pseudo-element の string。
    FirstLetter,
}

/// `content()` function の引数 `[ text | before | after | first-letter ]?`
/// (`?` は raikiri の受理済み記法 — spec 自身の bare `content()` 例
/// `h2 { string-set: heading content() }` に対応する省略可能性の注記であり、
/// GCPM 3 の grammar 自体の formal optional marker ではない)。
///
/// CSS GCPM 3 §1.1.1.1 "The content() function"
/// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> の verbatim production:
/// `content() = content([text | before | after | first-letter])`。keyword
/// 省略時に [`Text`](Self::Text) を採用する根拠は spec の "default" 宣言には
/// 依らない — 同 section の grammar には `?` が無く (content() の唯一の
/// 引数が構文上 optional でない)、`text` dt/dd は "This is the default
/// value" と述べるものの、同じ section に "default をどう定義するか" 自体が
/// 未解決の WG issue として残っており、TR 上安定した根拠ではない (同種の
/// overclaim を後で発見し、本 site を訂正済み)。
///
/// NB: sibling [`ContentPart`] (target-text() 用) と keyword 集合が重なるが、
/// `text` vs `content` の spec spelling divergence があるため型を分ける
/// (StringFetchMode / ContentPart と同じ per-function 専用 enum 慣行 —
/// `content(content)` を silently accept してはならない)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentTextKeyword {
    /// `text` — 要素の string value 全体 (`white-space: normal` 相当で決定)。
    /// keyword 省略時のフォールバック値だが、根拠は spec の "default" 宣言
    /// ではない (type-level doc 参照)。
    #[default]
    Text,
    /// `before` — `::before` pseudo-element の string value。
    Before,
    /// `after` — `::after` pseudo-element の string value。
    After,
    /// `first-letter` — `::first-letter` pseudo-element の string。
    FirstLetter,
}

/// `<quote>` production の 4 keyword。
///
/// CSS Content 3 §2.4.2 "Inserting Quotation Marks: the *-quote keywords"
/// <https://www.w3.org/TR/css-content-3/#quote-values> verbatim production:
/// `<quote> = open-quote | close-quote | no-open-quote | no-close-quote`。
///
/// verbatim: [`OpenQuote`](Self::OpenQuote) / [`CloseQuote`](Self::CloseQuote)
/// は "replaced by the appropriate string as defined by the `quotes`
/// property" かつ nesting depth を増減する。[`NoOpenQuote`](Self::NoOpenQuote) /
/// [`NoCloseQuote`](Self::NoCloseQuote) は "Inserts nothing (as in none)" だが
/// depth 増減のみ行う。実際の [`PropertyValue::Quotes`] 引き (nesting depth
/// → 文字列) は本 crate の static-side scope 外 — 下流 (raikiri-dom) が
/// `quotes` の computed value と併せて runtime resolve する ([`CounterStyle`] /
/// [`StringFetchMode`] と同じ「resolve は downstream 責務」の分担)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteKeyword {
    /// `open-quote` — nesting depth を increment、対応する開き引用符 string
    /// を挿入 (実際の文字列解決は downstream)。
    OpenQuote,
    /// `close-quote` — nesting depth を decrement、対応する閉じ引用符 string
    /// を挿入。
    CloseQuote,
    /// `no-open-quote` — 何も挿入しないが nesting depth は `open-quote` と
    /// 同様に increment する。
    NoOpenQuote,
    /// `no-close-quote` — 何も挿入しないが nesting depth は `close-quote` と
    /// 同様に decrement する。
    NoCloseQuote,
}

/// `leader()` の引数 `<leader-type> = dotted | solid | space | <string>`。
///
/// CSS Content 3 §2.5.1 "The leader() function"
/// <https://www.w3.org/TR/css-content-3/#leader-function>。
///
/// spec verbatim: `dotted` は "equivalent to `leader(".")`"、`solid` は
/// "equivalent to `leader("_")`"、`space` は "equivalent to `leader(" ")`"。
/// この等価性は **keyword の意味論の説明であって spelling の正規化指示ではない**
/// ([`counter_style_from_ident`] が `decimal` keyword を `Named("decimal")` に
/// 畳まず [`CounterStyle::Decimal`] という別 variant で保持するのと同じ
/// precedent) — 3 keyword を個別 variant に保持し、実際の leader glyph
/// 文字列への解決 (`Dotted` → `"."` 等) は downstream (paint) の rendering
/// 責務とする。[`String`](Self::String) variant の custom leader 文字列は
/// [`SmolStr`] で保持 ([`ContentComponent::Literal`] の SmolStr 化
/// precedent と同じ、短寿命 clone を bump にする)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaderType {
    /// `dotted` — spec 上 `leader(".")` と等価 (rendering 解決は downstream)。
    Dotted,
    /// `solid` — spec 上 `leader("_")` と等価。
    Solid,
    /// `space` — spec 上 `leader(" ")` と等価。
    Space,
    /// `<string>` — author 指定の custom leader 文字列。
    String(SmolStr),
}

/// `parse_content_list_items` の list vocabulary mode selector。
///
/// CSS Content 3 §2 <https://www.w3.org/TR/css-content-3/#content-values> と
/// CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list> は同名
/// `<content-list>` production を持つが、後者は前者の narrower な local 再定義
/// (GCPM 3 は `Link defaults` に CSS Content 3 を含めず、§1.1.1 L82 で自前に
/// `<content-list> = [ <string> | <counter()> | <counters()> | <content()> |
/// <attr()> ]+` を dfn する)。property ごとに受理される function 集合が違うため、
/// dispatch 時に mode で分岐する ([`StringFetchMode`] / [`ContentPart`] /
/// [`ContentTextKeyword`] と同じ per-context 専用 enum 慣行)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentListMode {
    /// CSS Content 3 §2 broad `<content-list>` — `content` property 用。
    /// 受理: `<string>` bare literal / `counter()` / `counters()` / `string()` /
    /// `attr()` / `target-counter()` / `target-counters()` / `target-text()` /
    /// `content()` / `<image>` (`url()` alternative のみ) /
    /// `contents` keyword / `<quote>` (`open-quote` 等) / `leader()`。10 alt
    /// full set (image/contents/quote/leader を追加し、旧実装の
    /// under-accept を解消)。
    CssContent3,
    /// CSS GCPM 3 §1.1.1 narrow local `<content-list>` — `string-set` 用。
    /// 受理: `<string>` bare literal / `counter()` / `counters()` / `content()` /
    /// `attr()`。**明示 reject**: `string()` (bare `<string>` literal とは別),
    /// `target-counter()`, `target-counters()`, `target-text()` (GCPM 3 §1.1.1
    /// L82 verbatim grammar より導出、cascade で declaration drop → shadow 効果を
    /// spec 準拠に一致させる)。
    GcpmStringSet,
}

/// `content` property の value item — cascade static side の中間表現。
///
/// design doc §7.1 の `raikiri_traits::ContentValueItem` に 1:1 mapping する
/// (下流 raikiri-dom が runtime resolve 時に翻訳)。raikiri-style は raikiri-traits
/// に依存しない leaf crate であるため、counter-* wire-through
/// pattern と同様に **local** な intermediate type で保持し、
/// downstream 側で shared trait type にマッピングする。
///
/// Variants は spec の function grammar 順:
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
///   §2.5.1 (under-accept fix として末尾に追加、既存 variant の並びは
///   互換性のため保持)
///
/// URL は raw `String` として保持 (raikiri-style は `url` crate に依存しない —
/// runtime resolve 段で `url::Url` へ parse する consumer 責務)。
///
/// # `#[non_exhaustive]` semantics (fulgur / downstream consumer 向け verbatim)
///
/// enum-level `#[non_exhaustive]` は downstream の `match` に `_ =>` arm を
/// 強制することで新 variant 追加を forward-compatible にするが、**既存 variant
/// の tuple constructor 呼び出しは block しない**。ゆえに既存 variant の
/// payload **type** 変更は downstream の constructor を compile-break させる。
///
/// cascade memory DoS 対策の一環として、[`Literal`](Self::Literal) の
/// payload を `String` → [`SmolStr`] に変更した。SmolStr は
/// `Deref<Target = str>` を提供するため、pattern-match で payload を **読む**
/// consumer は `match cc { ContentComponent::Literal(s) => &*s, .. }` や
/// `s.as_str()` / `s.len()` などの `&str` API がそのまま動作する。**construct**
/// する consumer のみ `ContentComponent::Literal("foo".into())` を
/// `ContentComponent::Literal(SmolStr::new("foo"))` (または `.into()` が有効な
/// context では対応する `From` impl) に書き換える。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentComponent {
    /// `<string>` bare literal (`content: "hello"`)。
    ///
    /// [`SmolStr`] は 22 bytes 以下を inline、超過分は内部 `Arc<str>` 保存で
    /// clone が O(1) bump になる (DoS 直系 attack vector
    /// `content: "<large>"` に対する secondary defense、primary は outer
    /// [`PropertyValue::Content`] の [`Arc<Vec<..>>`] wrap)。
    ///
    /// **Consumer 向け**:
    /// 変更前の payload は `String` だった。SmolStr は `Deref<Target = str>`
    /// を提供するので、read-side (`&*s` / `s.as_str()` / `s.len()` / `for c in s.chars()`)
    /// は透過的に継続動作する。construct-side のみ `SmolStr::new("foo")` (または
    /// `SmolStr::from(String)`) へ書き換える。enum-level docstring
    /// §`#[non_exhaustive]` semantics も参照。
    Literal(SmolStr),
    /// `counter(<counter-name>, <counter-style>?)`。
    Counter { name: SmolStr, style: CounterStyle },
    /// `counters(<counter-name>, <string>, <counter-style>?)`。
    Counters {
        name: SmolStr,
        separator: String,
        style: CounterStyle,
    },
    /// `string(<custom-ident>, [ first | start | last | first-except ]?)`。
    String {
        name: SmolStr,
        fetch: StringFetchMode,
    },
    /// `element(<custom-ident>)` — CSS GCPM 3 §1.2.2.
    ///
    /// The running-element name is retained for the downstream margin-box
    /// resolver. Page-scoped first/start/last selection remains outside this
    /// minimal single-page bridge.
    Element { name: SmolStr },
    /// `attr(<attribute-name>)` (§2.1)。
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
    /// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`。
    /// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>。
    ///
    /// 第 2 引数は `<counter-name>` ではなく `<custom-ident>` — spec verbatim
    /// (§2.6.1 の value definition):
    ///
    /// ```text
    /// target-counter() = target-counter( [ <string> | <url> ] , <custom-ident> , <counter-style>? )
    /// ```
    ///
    /// `counter()` / `counters()` (CSS Lists 3 §4 `<counter-name>`
    /// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) と異なり
    /// `none` を追加除外しない点に注意。
    TargetCounter {
        url: String,
        name: SmolStr,
        style: CounterStyle,
    },
    /// `target-counters([<string>|<url>], <custom-ident>, <string>, <counter-style>?)`。
    /// CSS Content 3 §2.6.2 <https://www.w3.org/TR/css-content-3/#target-counters>。
    ///
    /// 第 2 引数は `<counter-name>` ではなく `<custom-ident>` — spec verbatim
    /// (§2.6.2 の value definition):
    ///
    /// ```text
    /// target-counters() = target-counters( [ <string> | <url> ] , <custom-ident> , <string> , <counter-style>? )
    /// ```
    ///
    /// `counter()` / `counters()` (CSS Lists 3 §4 `<counter-name>`
    /// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) と異なり
    /// `none` を追加除外しない点に注意。
    TargetCounters {
        url: String,
        name: SmolStr,
        separator: String,
        style: CounterStyle,
    },
    /// `target-text([<string>|<url>], [ content | before | after | first-letter ]?)`。
    TargetText { url: String, part: ContentPart },
    /// `content([ text | before | after | first-letter ]?)` — GCPM 3 §1.1.1.1
    /// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> (`?` は raikiri の
    /// 受理済み記法であり spec の grammar 自体の表記ではない)。
    /// 現要素 (または擬似要素) の string value を named string に挿入する用途で、
    /// `<content-list>` の一員として `string-set` および `content` property の
    /// content-list 内で受理される。keyword 省略時は [`ContentTextKeyword::Text`]
    /// をフォールバック値として使う (根拠は spec の "default" 宣言ではない —
    /// [`ContentTextKeyword`] の doc comment 参照)。runtime resolve は
    /// raikiri-dom 責務 (wire-through pattern)。
    Content { keyword: ContentTextKeyword },
    /// `<image>` (CSS Images 3 <https://www.w3.org/TR/css-images-3/#typedef-image>
    /// `<image> = <url> | <gradient>`) — CSS Content 3 §2.2 "2D Images: the
    /// `<image>` values" <https://www.w3.org/TR/css-content-3/#content-uri>。
    /// spec verbatim: "Represents an anonymous inline replaced element filled
    /// with the specified `<image>`. If the `<image>` represents an invalid
    /// image, this value instead represents nothing" (rendering 側の
    /// fallback は downstream 責務)。
    ///
    /// **`<content-replacement>` との関係 (未反映、将来 task 送り)**:
    /// `content` property 全体の value definition (CSS Content 3 §1
    /// <https://www.w3.org/TR/css-content-3/#content-property>) は `normal |
    /// none | [ <content-replacement> | <content-list> ] […]?` で、
    /// `<content-replacement> = <image>` は `<content-list>` とは別の
    /// top-level alternative — spec verbatim: "Makes the element or
    /// pseudo-element a replaced element, filled with the specified
    /// `<image>`" で `::before`/`::after` 生成を抑制する等、上の list-item 版
    /// `<image>` (anonymous inline replaced element) とは異なる semantics
    /// を持つ。spec verbatim は続けて "If the value of `<content-list>` is a
    /// single `<image>`, it must instead be interpreted as a
    /// `<content-replacement>`" とも述べており、本 variant の shape
    /// (`Vec<ContentComponent>` の 1 要素が `Image` かどうか) は downstream
    /// がこの区別を再構成するのに十分な情報を保持している — replacement
    /// semantics 自体の実装 (pseudo-element 抑制含む) は本 crate の
    /// static-side scope 外。
    ///
    /// **(b) 非対応 (spec-valid)**: `<url>` alternative のみ実装
    /// (`url(...)` / `url("...")`)。`<gradient>` (`linear-gradient()` /
    /// `repeating-linear-gradient()` / `radial-gradient()` /
    /// `repeating-radial-gradient()`、CSS Images 3 §3.1-2) は gradient stop /
    /// color-interpolation infra が本 crate に無く defer (scope 外、追跡は
    /// follow-up task)。CSS Images 4 で追加された `image()` /
    /// `image-set()` / `element()` / `cross-fade()` / `paint()` は参照した
    /// CSS Images **3** の `<image>` production に含まれないため spec-invalid
    /// (Level 3 準拠) — これらは function 名が
    /// `parse_content_function` の match arm と一致せず自動的に drop される
    /// ため追加コード不要。
    ///
    /// URL は raw `String` として保持 (sibling [`TargetCounter`](Self::TargetCounter)
    /// 等と同じ convention、`url` crate 非依存)。
    Image { url: String },
    /// `contents` keyword — CSS Content 3 §2.3 "Elemental Content: the
    /// `contents` keyword" <https://www.w3.org/TR/css-content-3/#element-content>。
    /// spec verbatim: "The element's descendants" — pseudo-element の生成有無や
    /// 「既に他の pseudo-element で使用済みなら何もしない」という消費順序の
    /// 解決は本 crate の static-side scope 外 (parse_content の docstring の
    /// `normal`/`none` と同じ「生成判断は下流に委ねる」方針)。
    ///
    /// **`normal` との非対称性 (意図的)**: spec verbatim (§2.3) は "the initial
    /// value of content is `normal` and `normal` computes to `contents` on an
    /// element" と述べるが、[`parse_content`] は `normal` を空 `Vec` に畳んで
    /// 保持する — computed-value 時の `normal` → `contents` 展開は本 crate の
    /// static-side (specified 層) scope 外。一方、明示的な `content: none` は
    /// [`ContentComponent::None`] sentinel として保持され、pseudo-element
    /// consumers が box generation を抑制できる。したがって author が明示的に
    /// 書いた `content: contents` は `[Contents]` を返し、`content: normal`
    /// (initial value 相当) は `[]` を返す。
    Contents,
    /// Internal sentinel for an explicit `content: none` declaration.
    ///
    /// `normal` remains the empty list used for the initial value. Keeping
    /// `none` distinct lets downstream pseudo-element consumers suppress
    /// generated content without changing the public `PropertyValue` shape.
    None,
    /// `<quote>` (`open-quote` / `close-quote` / `no-open-quote` /
    /// `no-close-quote`) — CSS Content 3 §2.4.2
    /// <https://www.w3.org/TR/css-content-3/#quote-values>。詳細は
    /// [`QuoteKeyword`] の doc を参照 (実際の引用符文字列解決は
    /// [`PropertyValue::Quotes`] の computed value と合わせて downstream が
    /// 行う)。
    Quote(QuoteKeyword),
    /// `leader(<leader-type>)` — CSS Content 3 §2.5.1 "The leader() function"
    /// <https://www.w3.org/TR/css-content-3/#leader-function>。詳細は
    /// [`LeaderType`] の doc を参照。spec production `leader( <leader-type> )`
    /// に `?` が無いため引数は必須 (bare `leader()` は spec-invalid → parse 失敗
    /// = declaration drop)。
    Leader(LeaderType),
}

/// `display` property の value。
///
/// CSS Display 3 §2 "Box Layout Modes: the display property"
/// <https://www.w3.org/TR/css-display-3/#propdef-display>:
/// value grammar は
/// `[ <display-outside> || <display-inside> ] | <display-listitem> |
/// <display-internal> | <display-box> | <display-legacy>`、initial value
/// は `inline`、not inherited。
///
/// 現状受理する keyword は `block` / `inline` / `inline-block`
/// / `none` / `flex` / `grid` / `list-item` / `contents` / `table`
/// / `inline-table` / `table-row-group` / `table-header-group`
/// / `table-footer-group` / `table-row` / `table-column-group`
/// / `table-column` / `table-cell` / `table-caption` / `flow-root` の 19 値。
/// `flow-root` は standalone の block formatting context として実装する。
/// (将来対応) の keyword は `parse_display` が `None` を返し、
/// declaration が silent drop される (rule.rs 側 invalid-value drop path)。
///
/// `list-item` は `<display-listitem> = <display-outside>? && [ flow |
/// flow-root ]? && list-item` (outer-defaulting rule により省略された
/// `<display-outside>` は block になる) の keyword-acceptance のみを
/// 実装する — list-item box の生成 (principal box に加えて marker box を
/// 追加で作る CSS Lists 3 §2.2 の挙動) と `::marker` 擬似要素の解決は
/// 別 scope (marker/list-style-type 系の generated-content 機構に依存する
/// 別途 layout work)。したがって `display: list-item` は本 crate では
/// 単なる keyword として保持されるのみ。marker box 抜きの block-level
/// principal box という近似は CSS2.1 §12.5.1 "a list-item's principal
/// box is block-level" と spec-compatible (§12.5.1 のこの一文自体が
/// marker box の有無を条件にしていない)。
///
/// `#[non_exhaustive]`: variant 追加を non-breaking にする (InlineBlock /
/// None / Flex / Grid / ListItem / Contents 追加は本 attribute 経由で
/// forward-compatible)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayValue {
    /// `block` — CSS Display 3 §2 `<display-outside>` short form for
    /// "block flow" (block-level box containing block flow layout).
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-outside>
    Block,
    /// `inline` — CSS Display 3 §2 `<display-outside>` short form for
    /// "inline flow" (inline-level box containing inline flow layout).
    /// spec default (initial value)。
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-outside>
    Inline,
    /// `inline-block` — CSS Display 3 §2 `<display-legacy>` short form for
    /// "inline flow-root" (inline-level block container、button 相当 layout
    /// の primary)。
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-legacy>
    InlineBlock,
    /// `flow-root` — CSS Display 3 §2.5, a block-level box that establishes
    /// an independent block formatting context.
    /// <https://www.w3.org/TR/css-display-3/#valdef-display-flow-root>
    FlowRoot,
    /// `none` — CSS Display 3 §2 `<display-box>`: element (含 subtree) を
    /// box tree から omit する (hidden 相当)。
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-box>
    None,
    /// `flex` — CSS Display 3 §2.2 "Inner Display Layout Models" の
    /// `<display-inside>` short form for a flex formatting context。
    /// `<display-outside>` を省略した場合 outer display type は block に
    /// デフォルトするため (§2.2 の outer-defaulting rule)、`display: flex`
    /// は `display: block flex` と等価 (block-level box containing a
    /// flex formatting context)。この等価性は §2 の informative summary
    /// table にも明記されている。
    /// <https://www.w3.org/TR/css-display-3/#typedef-display-inside>
    /// <https://www.w3.org/TR/css-display-3/#the-display-properties>
    Flex,
    /// `inline-flex` — inline-level outer box establishing a flex formatting
    /// context. The DOM bridge uses this distinction for shrink-to-fit sizing.
    InlineFlex,
    /// `grid` — CSS Display 3 §2.2 "Inner Display Layout Models" の
    /// `<display-inside>` short form for a grid formatting context。
    /// `<display-outside>` を省略した場合 outer display type は block に
    /// デフォルトするため (§2.2 の outer-defaulting rule)、`display: grid`
    /// は `display: block grid` と等価 (block-level box containing a
    /// grid formatting context)。この等価性は §2 の informative summary
    /// table にも明記されている。
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
    /// `<display-outside>` を省略した場合 outer display type は block に
    /// デフォルトするため (§2.2 の outer-defaulting rule と同型)、
    /// `display: list-item` は `display: block flow list-item` と等価。
    ///
    /// keyword acceptance のみ — list-item principal box への marker box
    /// 追加 (CSS Lists 3 §2.2) と `::marker` 擬似要素の解決は本 variant の
    /// scope 外 (別途 generated-content/list 機構に依存する layout work)。
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
    /// ([`crate::cascade::resolve_inheritance`]'s root-element detection,
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

/// `flex-direction` property の value。
///
/// CSS Flexible Box Layout Module Level 1 §5.1 "Flex Flow Direction: the
/// flex-direction property"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>: value
/// grammar `row | row-reverse | column | column-reverse`、propdef table
/// "Initial: row"、"Inherited: no"、"Applies to: flex containers"、
/// "Computed value: specified keyword"。
///
/// `#[non_exhaustive]` — [`DisplayValue`] と同じ forward-compat 契約。
/// sibling と同じ convention で `Default` を derive しない — 初期化側
/// ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`Self::Row`] を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirectionValue {
    /// `row` — spec initial value。main axis はコンテナの inline axis と
    /// 同方向 (writing-mode 依存の物理方向解決は本 crate scope 外、
    /// taffy 側の同 keyword mapping に委譲)。
    Row,
    /// `row-reverse` — main axis は `row` の逆方向。
    RowReverse,
    /// `column` — main axis はコンテナの block axis と同方向。
    Column,
    /// `column-reverse` — main axis は `column` の逆方向。
    ColumnReverse,
}

/// `flex-wrap` property の value。
///
/// CSS Flexible Box Layout Module Level 1 §5.2 "Flex Line Wrapping: the
/// flex-wrap property"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>: value grammar
/// `nowrap | wrap | wrap-reverse`、"Initial: nowrap"、"Inherited: no"、
/// "Applies to: flex containers"、"Computed value: specified keyword"。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexWrapValue {
    /// `nowrap` — spec initial value。single-line。
    NoWrap,
    /// `wrap` — multi-line、cross-start から cross-end へ積む。
    Wrap,
    /// `wrap-reverse` — multi-line、`wrap` と逆順に積む。
    WrapReverse,
}

/// `flex-basis` property の specified value。
///
/// CSS Flexible Box Layout Module Level 1 §7.2.3 "The flex-basis property"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>: value
/// grammar `content | <'width'>`、"Initial: auto"、"Inherited: no"、
/// "Applies to: flex items"、"Computed value: specified keyword or a
/// computed `<length-percentage>` value"。
///
/// `<'width'>` は `width` property (CSS Sizing 3 §3.1.1) と同じ grammar
/// (`auto | <length-percentage [0,∞]>`) を再利用する旨の spec 記法 —
/// 本 crate の [`LengthOrAuto`] とほぼ同じ shape だが、`flex-basis` は
/// それに加え `content` keyword を持つ ("plus the content keyword" —
/// spec §7.1 の shorthand 解説部より) ため、[`LengthOrAuto`] をそのまま
/// 再利用せず専用 3-variant enum にする。
///
/// # `content` と `auto` の意味差 (spec §7.1 verbatim 要約) — 未解決のまま保持
///
/// - `auto`: 宣言要素の main-size property (`width`/`height`) の値を使う。
///   その値自体も `auto` なら used flex-basis は `content` になる
///   ("If that value is itself auto, then the used value is content.")。
/// - `content`: main-size property の値を無視し、常に content-based sizing
///   (typically max-content 相当) を使う。
///
/// 両者は computed 層でも区別を保つ (spec "Computed value: specified
/// keyword … " — `content` は `auto` に畳まない)。**この区別の実際の
/// 解決は本 crate の scope 外** — 下流 (raikiri-dom) の taffy bridge は
/// 両方とも `taffy::Dimension::AUTO` に写像せざるを得ない
/// (taffy 0.12 の `Dimension` に `content` 相当の variant が無いため)。
/// bridge 側の scope carving は `crates/raikiri-dom/src/layout.rs` の
/// `bridge_flex` doc を参照。
///
/// `#[non_exhaustive]` — sibling [`DisplayValue`] / [`LengthOrAuto`] と
/// 同じ forward-compat 契約。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlexBasisValue {
    /// `auto` — spec initial value。
    Auto,
    /// `content` — spec §7.1 "plus the content keyword"。
    Content,
    /// `min-content` — CSS Sizing 3 の intrinsic keyword (WPT
    /// `flex-basis-valid.html` が要求)。taffy 0.14 の同名 `Dimension`
    /// variant に写像する (`bridge_flex` doc 参照)。
    MinContent,
    /// `max-content` — 同上。
    MaxContent,
    /// bare `fit-content` keyword — 同上。`<length-percentage>` 引数付きの
    /// `fit-content()` function 形は scope 外 (WPT vector に現れない)。
    FitContent,
    /// `<length-percentage [0,∞]>` — `width` と同じ non-negative constraint
    /// ([`parse_flex_basis`] doc 参照)。
    Length(Length),
}

/// `flex` shorthand の specified value。
///
/// CSS Flexible Box Layout Module Level 1 §7.1 "The flex Shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-property>: value grammar
/// `none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]`、
/// "Initial: 0 1 auto"、"Inherited: no"、"Applies to: flex items"。
///
/// `none` は独立した exclusive keyword (`0 0 auto` と等価) であり、本 struct
/// では 3-field 展開後の値として表現する ([`parse_flex_shorthand`] doc の
/// "`none`" 節参照) — grammar 上 別 branch だが構造化後は他の 3-value 形と
/// 区別する必要がない。
///
/// # Omitted-component defaults は longhand の initial 値と**異なる**
///
/// spec 本文 verbatim (§7.1 "The flex property specifies…" 直後の Note):
///
/// > The initial values of the flex longhands are equivalent to
/// > `flex: 0 1 auto`. This differs from their defaults when omitted in the
/// > flex shorthand (effectively `1 1 0px`) so that the flex shorthand can
/// > better accommodate the most common cases.
///
/// すなわち shorthand 内で成分を省略した場合の default は
/// **grow=1 / shrink=1 / basis=0px** であり、`flex-grow`/`flex-shrink`/
/// `flex-basis` 各 longhand 自身の initial 値 (0 / 1 / auto) とは異なる。
/// [`parse_flex_shorthand`] がこの shorthand-local default を適用する。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlexShorthand {
    /// [`Self`] doc 参照 — `<'flex-grow'>` 成分、省略時 1.0。
    pub grow: f32,
    /// [`Self`] doc 参照 — `<'flex-shrink'>` 成分、省略時 1.0。
    pub shrink: f32,
    /// [`Self`] doc 参照 — `<'flex-basis'>` 成分、省略時 `Length(Length::Px(0.0))`。
    pub basis: FlexBasisValue,
}

/// `flex-flow` shorthand の specified value.
///
/// CSS Flexible Box Layout Module Level 1 §5.3 "Flex Direction and Wrap: the
/// flex-flow shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-flow-property>: value grammar
/// `<'flex-direction'> || <'flex-wrap'>`、
/// "Initial: see individual properties"、"Inherited: no"、
/// "Applies to: flex containers"、"Computed value: see individual properties".
///
/// `||` (any-order、each component at most once、at least 1 必須) —
/// 省略成分は対応 longhand の initial (direction=row / wrap=nowrap) に
/// 展開される ([`parse_flex_flow`] が適用)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlexFlow {
    /// [`Self`] doc 参照 — `<'flex-direction'>` 成分、省略時 Row。
    pub direction: FlexDirectionValue,
    /// [`Self`] doc 参照 — `<'flex-wrap'>` 成分、省略時 NoWrap。
    pub wrap: FlexWrapValue,
}

/// `justify-content` / `align-content` 共有 value ("content-distribution"
/// alignment)。
///
/// CSS Box Alignment Module Level 3 §5.1 "The justify-content and
/// align-content Properties" propdef `justify-content`
/// <https://www.w3.org/TR/css-align-3/#propdef-justify-content> / propdef
/// `align-content` <https://www.w3.org/TR/css-align-3/#propdef-align-content>
/// (両 propdef とも同じ §5.1 に同居する — 2 property を 1 節で定義する spec の
/// 構成そのものが、本 crate が両者に 1 型を共有する判断を後押しする):
/// 両者とも "Initial: normal"、"Inherited: no"、"Computed value: specified
/// keyword(s)"。両 grammar は下記の scope carving を除き同型
/// (`<content-distribution>` = §4.3 `space-between | space-around |
/// space-evenly | stretch`、`<content-position>` = §4.1
/// `center | start | end | flex-start | flex-end`) なので 1 型を共有する
/// (`AlignItemsKeyword`/`AlignContentKeyword` を分けた precedent の逆 —
/// grammar が実質同一なら共有する、という同じ判断原則の適用)。
///
/// # Scope carving
///
/// - **(b) 非対応**: `<overflow-position>` (`safe`/`unsafe` prefix、§4.4) は
///   未実装 — parser はそれらの prefix を受理せず、prefix 付き宣言全体を
///   drop する (`safe center` のような 2-token 列は `parse_content_alignment`
///   の単一 keyword match に一致しないため自然に `None`)。
/// - **(b) 非対応**: `<baseline-position>` (`first`?/`last`? `baseline`、
///   `align-content` のみの grammar 分岐) は未実装 — taffy 0.12 の
///   `AlignContent`/`JustifyContent` (共に `alignment::AlignContentKeyword`
///   ベース) に `Baseline` variant が無く、taffy 側で表現不可能なため。
/// - **(a) spec-invalid for this pair**: `justify-content` 独自の
///   `left`/`right` 拡張 (`<content-position> | left | right`、writing-mode
///   相対 keyword) は未実装 — taffy に対応 variant が無い。
///
/// 3 点とも「本 crate が値を捏造しない」原則により **silent drop** (parse
/// failure → declaration 全体 drop) で扱う。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentAlignmentValue {
    /// `normal` — spec initial value。box alignment の "context に応じた
    /// default" — その意味は `align-content` と `justify-content` とで
    /// 異なる: flex の `align-content: normal` は [`Self::Stretch`] 相当
    /// (複数行が cross axis を埋める) だが、`justify-content: normal` は
    /// `flex-start` 相当 (main axis 上で詰めて配置、stretch する軸ではない)。
    /// taffy bridge 側はどちらも `None` へ mapping し、taffy 自身の
    /// field 別 default resolution に委ねる (`crates/raikiri-dom/src/layout.rs`
    /// の `content_alignment_to_taffy` 参照)。
    Normal,
    /// `stretch` — CSS Box Alignment 3 §4.3 `<content-distribution>`。
    Stretch,
    /// `space-between` — `<content-distribution>`。
    SpaceBetween,
    /// `space-evenly` — `<content-distribution>`。
    SpaceEvenly,
    /// `space-around` — `<content-distribution>`。
    SpaceAround,
    /// `center` — CSS Box Alignment 3 §4.1 `<content-position>`。
    Center,
    /// `start` — `<content-position>`。
    Start,
    /// `end` — `<content-position>`。
    End,
    /// `flex-start` — `<content-position>`。
    FlexStart,
    /// `flex-end` — `<content-position>`。
    FlexEnd,
}

/// `align-items` value ("self-alignment" — CSS Box Alignment 3 §4.1
/// `<self-position>` を軸にした keyword set)。
///
/// CSS Box Alignment Module Level 3 §7.2 "Block-Axis (or Cross-Axis)
/// Default Alignment: the align-items property" propdef `align-items`
/// <https://www.w3.org/TR/css-align-3/#propdef-align-items>: value grammar
/// `normal | stretch | <baseline-position> | <overflow-position>?
/// <self-position>`、"Initial: normal"、"Inherited: no"、"Applies to: all
/// elements"、"Computed value: specified keyword(s)"。
///
/// `align-self` (§6.2) はこの enum を [`AlignSelfValue::Value`] 経由で再利用する
/// — grammar は `align-items` の全 keyword を含んだ上で `auto` を追加するため
/// (共有型 + wrapper の precedent、[`FlexBasisValue`] が `LengthOrAuto` を
/// 再利用せず専用 enum にしたのとは逆方向の判断だが、いずれも「共有できる
/// grammar 部分だけを 1 型に切り出す」原則の適用)。
///
/// # Scope carving
///
/// - **(b) 非対応**: `<overflow-position>` (`safe`/`unsafe` prefix) は
///   未実装、[`ContentAlignmentValue`] と同じ scope carving。
/// - **(b) 非対応**: `self-start`/`self-end` (`<self-position>` の一部、
///   writing-mode 相対 keyword) は未実装 — taffy `AlignItemsKeyword` に
///   対応 variant が無いため。
/// - **(b) 非対応**: `<baseline-position>` の `first`/`last` prefix は
///   未実装 — taffy `AlignItemsKeyword::Baseline` は prefix 区別を持たない
///   ("first" が既定、spec §9 "Fallback Alignment" 相当の細分化は非対応)。
///   bare `baseline` keyword のみ受理する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfAlignmentValue {
    /// `normal` — spec initial value。
    Normal,
    /// `stretch`。
    Stretch,
    /// `center` — `<self-position>`。
    Center,
    /// `start` — `<self-position>`。
    Start,
    /// `end` — `<self-position>`。
    End,
    /// `flex-start` — `<self-position>`。
    FlexStart,
    /// `flex-end` — `<self-position>`。
    FlexEnd,
    /// `baseline` — `<baseline-position>` (prefix 非対応、[`Self`] doc 参照)。
    Baseline,
}

/// `align-self` property の value。
///
/// CSS Box Alignment Module Level 3 §6.2 "Block-Axis (or Cross-Axis)
/// Self-Alignment: the align-self property" propdef `align-self`
/// <https://www.w3.org/TR/css-align-3/#propdef-align-self>: value grammar
/// `auto | <overflow-position>? [ normal | <self-position> ] | stretch |
/// <baseline-position>`、"Initial: auto"、"Inherited: no"、"Applies to:
/// flex items, grid items, and absolutely-positioned boxes"、"Computed
/// value: specified keyword(s)"。
///
/// `auto` 以外の全 keyword は [`SelfAlignmentValue`] (= `align-items` の
/// grammar) と同一 — [`Self`] doc の共有 rationale 参照。
///
/// `#[non_exhaustive]` — sibling [`SelfAlignmentValue`] と同じ
/// forward-compat 契約。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignSelfValue {
    /// `auto` — spec initial value。CSS Box Alignment 3 §6.2 `valdef-align-
    /// self-auto` (verbatim ではない要約): 親の computed `align-items` 値
    /// (legacy keyword 除く) として振る舞う — 実際の fallback 解決は本 crate
    /// scope 外、taffy 側 (`Option<AlignSelf> = None` → 親の `align_items`
    /// へ fallback) に委譲する。
    Auto,
    /// `auto` 以外の明示 keyword — [`SelfAlignmentValue`] をそのまま再利用。
    Value(SelfAlignmentValue),
}

/// `gap` shorthand の specified value。
///
/// CSS Box Alignment Module Level 3 §8.2 "Gap Shorthand: the gap property"
/// propdef `gap`
/// <https://www.w3.org/TR/css-align-3/#propdef-gap>: value grammar
/// `<'row-gap'> <'column-gap'>?`、"Initial: see individual properties"、
/// "Inherited: no"。第 2 成分省略時は第 1 成分の値をそのまま copy する
/// (spec 本文: "If column-gap is omitted, it's set to the same value as
/// row-gap.")。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GapShorthand {
    /// `row-gap` 成分。
    pub row: LengthOrNormal,
    /// `column-gap` 成分 — 省略時は `row` と同値 ([`parse_gap_shorthand`] 参照)。
    pub column: LengthOrNormal,
}

/// `place-content` shorthand の specified value。
///
/// CSS Box Alignment Module Level 3 §5.2 "Content-Distribution Shorthand:
/// the place-content property" propdef `place-content`
/// <https://www.w3.org/TR/css-align-3/#propdef-place-content>: value
/// grammar `<'align-content'> <'justify-content'>?`、"Initial: normal"、
/// "Inherited: no"。第 2 成分省略時は第 1 成分の値をそのまま copy する
/// spec 規則の例外 ("unless that value is a `<baseline-position>` in which
/// case it is defaulted to `start`") は本 crate では到達不能 — 本 crate の
/// [`ContentAlignmentValue`] は `<baseline-position>` variant 自体を持たない
/// ([`ContentAlignmentValue`] doc の scope carving 節参照) ため、"copy from
/// first value" 分岐のみが常に成立する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceContentShorthand {
    /// `align-content` 成分。
    pub align: ContentAlignmentValue,
    /// `justify-content` 成分 — 省略時は `align` と同値
    /// ([`parse_place_content_shorthand`] 参照、[`Self`] doc の例外注記も参照)。
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
/// `#[non_exhaustive]` — [`DisplayValue`] と同じ forward-compat 契約。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackBreadth {
    /// `<length-percentage [0,∞]>`。
    Length(Length),
    /// `<flex [0,∞]>` — the `fr` unit (§7.2.4)。authored non-negative number
    /// (`1fr` → `Flex(1.0)`)。
    Flex(f32),
    /// `min-content`。
    MinContent,
    /// `max-content`。
    MaxContent,
    /// `auto` — track-sizing 文脈での `auto` は "as `max-content`, but
    /// clamped to fit within the grid container" (spec §7.2.1) — resolve は
    /// used-value layer (raikiri-dom / taffy) 責務。
    Auto,
}

/// `<inflexible-breadth>` — `minmax()` の第 1 引数 (min side) の grammar。
/// `<track-breadth>` から `<flex>` を除いたもの (spec: "A minmax() function
/// takes exactly two arguments... If the first argument is a `<flex>`
/// value... the declaration is invalid" 相当の grammar-level 除外)。
///
/// CSS Grid Layout Module Level 1 §7.2.1
/// (<https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-inflexible-breadth>):
///
/// `<inflexible-breadth> = <length-percentage [0,∞]> | min-content |
/// max-content | auto`
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridInflexibleBreadth {
    /// `<length-percentage [0,∞]>` — この variant はそのまま `<fixed-breadth>`
    /// ([`GridTrackSize`] doc の "fixed-size 制約" 節参照) にも相当する。
    Length(Length),
    /// `min-content`。
    MinContent,
    /// `max-content`。
    MaxContent,
    /// `auto`。
    Auto,
}

/// `<track-size>` — 1 grid track の sizing function。
///
/// CSS Grid Layout Module Level 1 §7.2.1 "Track Sizes"
/// (<https://www.w3.org/TR/css-grid-1/#typedef-track-size>), grammar
/// (verbatim):
///
/// `<track-size> = <track-breadth> | minmax( <inflexible-breadth> ,
/// <track-breadth> ) | fit-content( <length-percentage [0,∞]> )`
///
/// # `<fixed-size>` — auto-repeat / fixed-repeat 内で追加される制約
///
/// `repeat(auto-fill|auto-fit, …)` / `repeat(<integer>, …)` の一部の形
/// (`<auto-repeat>` / `<fixed-repeat>`、[`GridTrackRepeat`] doc 参照) は
/// `<track-size>` ではなく、より狭い `<fixed-size>` (§7.2.1
/// <https://www.w3.org/TR/css-grid-1/#typedef-fixed-size>) を要求する:
///
/// `<fixed-size> = <fixed-breadth> | minmax( <fixed-breadth> , <track-breadth>
/// ) | minmax( <inflexible-breadth> , <fixed-breadth> )` where `<fixed-breadth>
/// = <length-percentage [0,∞]>`
///
/// この crate は `<track-size>` と `<fixed-size>` を型として分けず (両者は
/// [`Self`] の同じ shape で表現可能)、代わりに [`grid_track_size_is_fixed`]
/// が post-parse validation として `<fixed-size>` 制約 (`fr` / bare
/// `min-content`/`max-content`/`auto` を許さない、`fit-content()` も不可) を
/// [`parse_grid_template_tracks`] から適用する。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackSize {
    /// bare `<track-breadth>`。
    Breadth(GridTrackBreadth),
    /// `minmax( <inflexible-breadth>, <track-breadth> )`。
    MinMax(GridInflexibleBreadth, GridTrackBreadth),
    /// `fit-content( <length-percentage [0,∞]> )` — spec §7.2.1: "represents
    /// the formula `max(minimum, min(limit, max-content))`"。limit は非負
    /// length-percentage。
    FitContent(Length),
}

/// `repeat()` の第 1 引数 (repetition count)。
///
/// CSS Grid Layout Module Level 1 §7.2.3.1 "Syntax of repeat()"
/// (<https://www.w3.org/TR/css-grid-1/#typedef-track-repeat>): `<track-repeat>`
/// は `<integer [1,∞]>` のみ、`<auto-repeat>` (§7.2.3.2
/// <https://www.w3.org/TR/css-grid-1/#typedef-auto-repeat>) は `auto-fill |
/// auto-fit` のみ。この crate は両方を 1 つの enum で表現し、
/// [`parse_grid_repeat`] が context ごとに正しい alternative のみ受理する
/// ([`GridTrackRepeat`] doc の "許可される count" 節参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridRepeatCount {
    /// `<integer [1,∞]>` — 固定回数の repeat (`<track-repeat>` /
    /// `<fixed-repeat>`)。
    Count(u32),
    /// `auto-fill` — repeat-to-fill、空きトラックは残す
    /// (§7.2.3.2 <https://www.w3.org/TR/css-grid-1/#auto-fill>)。実際の
    /// repetition 回数の算出は used-value layer (raikiri-dom / taffy) 責務。
    AutoFill,
    /// `auto-fit` — `auto-fill` と同じだが、空きトラックを collapse する
    /// (§7.2.3.2 <https://www.w3.org/TR/css-grid-1/#auto-fit>)。
    AutoFit,
}

/// `repeat( <count>, <tracks> )` — track list 中の 1 repeat() component。
///
/// CSS Grid Layout Module Level 1 §7.2.3.1
/// (<https://www.w3.org/TR/css-grid-1/#funcdef-repeat>), grammar
/// (verbatim, 3 alternative forms):
///
/// ```text
/// <track-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <track-size> ]+ <line-names>? )
/// <auto-repeat>  = repeat( [ auto-fill | auto-fit ] , [ <line-names>? <fixed-size> ]+ <line-names>? )
/// <fixed-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <fixed-size> ]+ <line-names>? )
/// ```
///
/// [`Self::line_names`] は [`GridTrackList::line_names`] と同じ interleave
/// 規約 (`line_names.len() == tracks.len() + 1`、`tracks[i]` の直前が
/// `line_names[i]`、末尾の trailing set が `line_names[tracks.len()]`) —
/// taffy 0.12 `GridTemplateRepetition.line_names` の shape と一致する
/// (repeat() 1 巡分の line name を表す、繰り返しの巡ごとの名前 merge は
/// used-value layer 責務、spec §7.2.3.1 の "If a repeat() function ends up
/// placing two `<line-names>` adjacent to each other, the name lists are
/// merged" は本 crate の scope 外)。
///
/// # 許可される count と `<fixed-size>` 制約
///
/// - [`GridRepeatCount::Count`] (`<track-repeat>`) — [`Self::tracks`] は
///   full `<track-size>` ([`GridTrackSize`] のいずれの variant も可、`fr`
///   含む)。ただし [`GridTrackList`] level の "at most one auto-repeat"
///   制約とは無関係に、この形自体は無制限に track list 中へ現れてよい。
/// - [`GridRepeatCount::AutoFill`] / [`GridRepeatCount::AutoFit`]
///   (`<auto-repeat>`) — [`Self::tracks`] は `<fixed-size>` 制約下
///   ([`GridTrackSize`] doc 参照、`fr`/bare `min-content`/`max-content`/
///   `auto`/`fit-content()` は不可)。spec §7.2.3.1 verbatim: "It can only
///   appear once in the track list, but the same track list can also
///   contain `<fixed-repeat>`s." — [`parse_grid_template_tracks`] が
///   track list 全体を通して 1 回まで constraint を検査する。
/// - `<fixed-repeat>` は本 crate では別 variant を持たず、
///   [`GridRepeatCount::Count`] + `<fixed-size>` 制約 (auto-repeat が
///   track list 中に存在する場合のみ [`grid_track_size_is_fixed`] で
///   post-validate) として扱う — [`parse_grid_template_tracks`] doc 参照。
///
/// `repeat()` はネストしない (spec §7.2.3.1 verbatim: "The repeat() notation
/// can't be nested.") — [`Self::tracks`] の要素型が [`GridTrackSize`] で
/// あり [`GridTrackList`] を含まないため、構造的にネスト不可能。
#[derive(Clone, Debug, PartialEq)]
pub struct GridTrackRepeat {
    /// 繰り返し回数。
    pub count: GridRepeatCount,
    /// interleaved line name。[`Self`] doc の shape 参照。
    pub line_names: Vec<Vec<SmolStr>>,
    /// 繰り返される track sizing function 列。
    pub tracks: Vec<GridTrackSize>,
}

/// `<track-list>` / `<auto-track-list>` 中の 1 component — 単独 track か
/// `repeat()`。
///
/// CSS Grid Layout Module Level 1 §7.2 "Explicit Track Sizing"
/// (<https://www.w3.org/TR/css-grid-1/#track-sizing>) の `<track-list>`
/// grammar: `[ <line-names>? [ <track-size> | <track-repeat> ] ]+
/// <line-names>?`。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackListComponent {
    /// 単独 track sizing function。
    Size(GridTrackSize),
    /// `repeat()`。
    Repeat(GridTrackRepeat),
}

/// `grid-template-columns` / `grid-template-rows` の `none` 以外の specified
/// value — track list 全体 (interleaved line names + component 列)。
///
/// CSS Grid Layout Module Level 1 §7.2, `<track-list>` grammar:
/// `[ <line-names>? [ <track-size> | <track-repeat> ] ]+ <line-names>?`。
///
/// [`Self::line_names`] は `Self::components` と 1 対 1 で interleave する:
/// `line_names.len() == components.len() + 1`、`components[i]` の直前の
/// named line set が `line_names[i]`、track list 全体の末尾 (最後の
/// component の後) が `line_names[components.len()]`。この shape は taffy
/// 0.12 の `Style::grid_template_column_names` / `grid_template_row_names`
/// (`components` と lock-step で consume される、taffy `NamedLineResolver`
/// 内部 iteration の shape) と直接対応し、raikiri-dom 側 bridge が
/// 変換なしで zip できる。
///
/// この crate は `<track-list>` (auto-repeat なし、全 component が full
/// `<track-size>` を使える) と `<auto-track-list>` (ちょうど 1 つの
/// auto-repeat を含み、他の全 track は `<fixed-size>` 制約下)
/// を型として分けず、1 つの `GridTrackList` で両方を表現する —
/// [`parse_grid_template_tracks`] が post-parse validation として
/// "at most one auto-repeat" と "auto-repeat 存在時、他の全 track は
/// `<fixed-size>`" の 2 制約を検査する ([`GridTrackRepeat`] doc 参照)。
#[derive(Clone, Debug, PartialEq)]
pub struct GridTrackList {
    /// interleaved line name。[`Self`] doc の shape 参照。
    pub line_names: Vec<Vec<SmolStr>>,
    /// track list の component 列。
    pub components: Vec<GridTrackListComponent>,
}

/// `grid-template-columns` / `grid-template-rows` の specified value。
///
/// CSS Grid Layout Module Level 1 §7.2 "Explicit Track Sizing: the
/// grid-template-rows and grid-template-columns properties"
/// (<https://www.w3.org/TR/css-grid-1/#track-sizing>): value grammar `none |
/// <track-list> | <auto-track-list>`、"Initial: none"、"Inherited: no"、
/// "Percentages: refer to corresponding dimension of the content area"、
/// "Computed value: the keyword `none` or a computed track list"。
///
/// `Arc` wrap は [`PropertyValue`] doc の "cascade memory DoS 対策" 節と同じ
/// perf pattern ([`ContentComponent`] list の `Content(Arc<Vec<..>>)` と同じ
/// 理由 — track list は任意個の repeat() を含みうる heap payload で、
/// cascade winner 選定のたび clone されうる)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTemplateTracks {
    /// `none` — spec initial value。explicit track が定義されない。
    None,
    /// `<track-list>` / `<auto-track-list>`。
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

/// `grid-template-areas` の 1 named area — 1-based, exclusive-end な grid
/// line 座標 (taffy 0.12 `GridTemplateArea` と同じ座標系、CSS Grid Layout
/// Module Level 1 §9.2 "Line-based Placement: the grid-template-areas
/// shorthand" の rectangle-to-lines 変換規則)。
#[derive(Clone, Debug, PartialEq)]
pub struct GridTemplateAreaEntry {
    /// area 名。
    pub name: SmolStr,
    /// row 開始 grid line (1-based)。
    pub row_start: u32,
    /// row 終了 grid line (1-based, exclusive — `row_end - row_start` が
    /// row span)。
    pub row_end: u32,
    /// column 開始 grid line (1-based)。
    pub column_start: u32,
    /// column 終了 grid line (1-based, exclusive)。
    pub column_end: u32,
}

/// `grid-template-areas` の `none` 以外の specified value。
///
/// CSS Grid Layout Module Level 1 §7.3 "Named Areas: the
/// grid-template-areas property"
/// (<https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>):
/// "Computed value: the keyword `none` or a **list of string values**" —
/// grid-template-columns/-rows と異なり、computed value は解析済み area
/// 矩形ではなく **authored string のリストそのもの**。[`Self::row_strings`]
/// がこの computed-value 要件を満たす。[`Self::areas`] /
/// [`Self::row_count`] / [`Self::column_count`] は parse 時に一度だけ
/// 算出する解析結果 (raikiri-dom bridge が再解析せず直接 taffy
/// `GridTemplateArea` へ変換できるようにするための cache)。
#[derive(Clone, Debug, PartialEq)]
pub struct GridTemplateAreas {
    /// authored string 列 (spec の computed value そのもの)。
    pub row_strings: Vec<SmolStr>,
    /// 解析済み named area — [`parse_grid_template_areas`] の rectangle
    /// validation を通過したもののみ。
    pub areas: Vec<GridTemplateAreaEntry>,
    /// string grid の row 数 (= `row_strings.len()`)。
    pub row_count: u32,
    /// string grid の column 数 (全 row で同数、spec 制約
    /// [`parse_grid_template_areas`] doc 参照)。
    pub column_count: u32,
}

/// `grid-template-areas` property の specified value。
///
/// [`GridTemplateAreas`] doc 参照。`Arc` wrap は [`GridTemplateTracks::List`]
/// と同じ理由 (heap payload、cascade winner 選定での clone コスト削減)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridTemplateAreasValue {
    /// `none` — spec initial value。
    None,
    /// `<string>+` — 解析済み named area の集合。
    Areas(Arc<GridTemplateAreas>),
}

/// `grid-auto-flow` property の value。
///
/// CSS Grid Layout Module Level 1 §7.7 "Automatic Placement: the
/// grid-auto-flow property"
/// (<https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-flow>): value
/// grammar `[ row | column ] || dense`、"Initial: row"、"Inherited: no"、
/// "Computed value: specified keyword(s)"。
///
/// `#[non_exhaustive]` — [`DisplayValue`] と同じ forward-compat 契約。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridAutoFlowValue {
    /// `row` (dense なし) — spec initial value。
    Row,
    /// `column` (dense なし)。
    Column,
    /// `row dense`。
    RowDense,
    /// `column dense`。
    ColumnDense,
}

/// `<grid-line>` — `grid-row-start` / `grid-row-end` / `grid-column-start` /
/// `grid-column-end` の共有 grammar (§8.4 の `grid-row` / `grid-column`
/// shorthand は [`GridLineShorthand`] 経由でこれを再利用する)。
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
/// "Initial: auto"、"Inherited: no"、"Percentages: n/a"、"Computed value:
/// specified keyword, identifier, and/or integer"。
///
/// spec 本文 verbatim: "In all the above productions, the `<custom-ident>`
/// additionally excludes the keywords `span` and `auto`"、および "If the
/// `<integer>` is omitted, it defaults to 1. Negative integers or zero are
/// invalid." — [`is_reserved_grid_line_name`] と [`parse_grid_line`] が
/// それぞれの制約を enforce する。
///
/// bare `<custom-ident>` alternative (index 省略) は
/// [`Self::NamedLine`] に index `1` を明示的に埋めて畳む — taffy 0.12
/// `GridPlacement::NamedLine` は index `0` を「未指定」sentinel として扱い、
/// 内部で `0` を `1` に正規化する
/// (`NamedLineResolver::find_line_index` の `if idx == 0 { idx = 1; }`) ため、
/// この crate 側で先に `1` を埋めても意味は変わらない。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum GridLineValue {
    /// `auto` — spec initial value。auto-placement、または (span と併用時)
    /// default span of one。
    Auto,
    /// `<integer>` — 1-based grid line index (負数は末尾から)。`0` は spec
    /// 上 invalid ([`parse_grid_line`] doc 参照)。
    Line(i32),
    /// bare `<custom-ident>` — grammar top-level alternative (`<integer>`
    /// 併記なし)。[`GridLineShorthand`] の第 2 成分省略時 copy 規則
    /// (spec 本文 verbatim "if the first value is a `<custom-ident>`") は
    /// **この variant のみ**を対象とする — [`Self::NamedLine`]
    /// (`<integer> && <custom-ident>` compound、`<integer>` 省略時も spec 上
    /// `1` を明示指定した扱いになる別の grammar alternative) は対象外
    /// ([`parse_grid_line`] doc 参照)。
    Named(SmolStr),
    /// `[ [ <integer> ] && <custom-ident>? ]` — `<integer>` 必須の named
    /// line 参照 compound (`<custom-ident>` 側は省略可)。
    NamedLine(SmolStr, i32),
    /// `span <integer>` — 明示 span。
    Span(u32),
    /// `span <custom-ident>` (`<integer>` 併記可、省略時は `1`) — named line
    /// までの span。
    SpanNamed(SmolStr, u32),
}

/// `grid-row` / `grid-column` shorthand の specified value。
///
/// CSS Grid Layout Module Level 1 §8.4 "Placement Shorthands: the
/// grid-column, grid-row, and grid-area properties"
/// (<https://www.w3.org/TR/css-grid-1/#placement-shorthands>): value grammar
/// `<grid-line> [ / <grid-line> ]?`、"Initial: auto"、"Inherited: no"。
///
/// spec 本文 verbatim: "If two `<grid-line>` values are specified, the
/// grid-row-start / grid-column-start longhand is set to the value before
/// the slash, and the grid-row-end / grid-column-end longhand is set to the
/// value after the slash. When the second value is omitted, if the first
/// value is a `<custom-ident>`, the grid-row-end / grid-column-end longhand
/// is also set to that `<custom-ident>`; otherwise, it is set to `auto`." —
/// [`parse_grid_line_shorthand`] がこの規則を適用する。
#[derive(Clone, Debug, PartialEq)]
pub struct GridLineShorthand {
    /// `-start` longhand 成分。
    pub start: GridLineValue,
    /// `-end` longhand 成分 — 省略時の規則は [`Self`] doc 参照。
    pub end: GridLineValue,
}

/// `place-items` shorthand の specified value。
///
/// CSS Box Alignment Module Level 3 §7.3 "Default Alignment Shorthand: the
/// place-items property"
/// (<https://www.w3.org/TR/css-align-3/#propdef-place-items>): value grammar
/// `<'align-items'> <'justify-items'>?`、"Initial: see individual
/// properties"、"Inherited: no"。第 2 成分省略時は第 1 成分の値をそのまま
/// copy する ([`PlaceContentShorthand`] doc の同型注記参照 — 本 crate の
/// [`SelfAlignmentValue`] は `justify-items` 側の追加 scope carve-out
/// (`legacy`、[`PropertyValue::JustifyItems`] doc 参照) を持たないため、
/// copy 規則の例外分岐は到達不能)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceItemsShorthand {
    /// `align-items` 成分。
    pub align: SelfAlignmentValue,
    /// `justify-items` 成分 — 省略時は `align` と同値
    /// ([`parse_place_items_shorthand`] 参照)。
    pub justify: SelfAlignmentValue,
}

/// `place-self` shorthand の specified value。
///
/// CSS Box Alignment Module Level 3 §6.3 "Self-Alignment Shorthand: the
/// place-self property"
/// (<https://www.w3.org/TR/css-align-3/#propdef-place-self>): value grammar
/// `<'align-self'> <'justify-self'>?`、"Initial: `auto`"、"Inherited: no"。
/// 第 2 成分省略時の copy 規則は [`PlaceItemsShorthand`] と同じ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceSelfShorthand {
    /// `align-self` 成分。
    pub align: AlignSelfValue,
    /// `justify-self` 成分 — 省略時は `align` と同値
    /// ([`parse_place_self_shorthand`] 参照)。
    pub justify: AlignSelfValue,
}

/// `grid-auto-columns` / `grid-auto-rows` の spec initial value
/// (`auto`、単一要素 `[GridTrackSize::Breadth(GridTrackBreadth::Auto)]`) の
/// shared `Arc` — [`empty_content_list`] と同じ perf pattern (per-node
/// allocation を避け、process 全体で 1 heap slot を bump-share する)。
pub(crate) fn initial_grid_auto_track_list() -> Arc<Vec<GridTrackSize>> {
    static INITIAL: OnceLock<Arc<Vec<GridTrackSize>>> = OnceLock::new();
    INITIAL
        .get_or_init(|| Arc::new(vec![GridTrackSize::Breadth(GridTrackBreadth::Auto)]))
        .clone()
}

/// `box-sizing` property の value。
///
/// CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
/// <https://www.w3.org/TR/css-sizing-3/#box-sizing>: value grammar
/// `content-box | border-box`、initial value `content-box`、**not inherited**、
/// computed value = specified keyword。
///
/// spec note (§3.3): "The definition of the box-sizing property in this module
/// supersedes the one in [CSS-UI-3]" — CSS-UI-3 の box-sizing 定義は本 module
/// により supersede されるため、css-sizing-3 が authoritative source。
///
/// # Semantics (spec verbatim summary)
///
/// - [`ContentBox`](Self::ContentBox) — spec initial value。指定した `width` /
///   `height` は content box を対象とし、padding / border は content box の
///   外側に加算される (legacy CSS 2.1 box model)。
/// - [`BorderBox`](Self::BorderBox) — 指定した `width` / `height` は border
///   box を対象とし、padding / border は指定 size 内で content box を縮める
///   ("The specified padding and border of the element are laid out and drawn
///   inside this specified width and height").
///
/// # Scope carving
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 未知 keyword (`padding-box` — CSS UI 3 draft 相当
///   だが css-sizing-3 では削除、`margin-box` 等) は silent drop = `None`。
///
/// # Downstream handoff (future scope、style-scope confined)
///
/// [`ComputedValues.box_sizing`] は cascade static side seed のみ保持し、
/// `apply_computed_to_style` bridge (dom scope、`taffy::Style::box_sizing`
/// への翻訳) は future cross-scope task に defer。
///
/// `#[non_exhaustive]` は sibling [`DisplayValue`] / [`TextAlign`] /
/// [`PositionValue`] と同じ forward-compat 契約。
///
/// [`ComputedValues.box_sizing`]: crate::computed::ComputedValues::box_sizing
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxSizing {
    /// `content-box` — spec initial value。`width` / `height` は content box
    /// を対象とし、padding / border は content box の外側に加算される。
    ContentBox,
    /// `border-box` — `width` / `height` は border box を対象とし、padding /
    /// border は指定 size 内で content box を縮める。
    BorderBox,
}

/// `hanging-punctuation` property value.
///
/// CSS Text 3 §8.2.1
/// <https://drafts.csswg.org/css-text-3/#hanging-punctuation-property>.
/// The full grammar also has `last`, `force-end`, and `allow-end`; this
/// milestone carries only the inherited `none | first` subset needed by the
/// leading U+3000 WPT slice. Unsupported valid keywords are dropped until a
/// matching line-layout implementation lands.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HangingPunctuation {
    /// No punctuation hangs. This is the initial value.
    None,
    /// A leading opening mark, quote, or U+3000 IDEOGRAPHIC SPACE hangs on
    /// the first formatted line.
    First,
}

/// `text-align` property の value。
///
/// CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
/// <https://www.w3.org/TR/css-text-3/#text-align-property>。**spec 上 shorthand** —
/// `text-align-all` + `text-align-last` の 2 longhand を set する
/// (`Initial: start` / `Inherited: yes`)。
///
/// Value grammar: `start | end | left | right | center | justify | match-parent | justify-all`。
///
/// # Scope carving
///
/// - **(b) 非対応**: 本 crate は shorthand を expand
///   せず、`ComputedValues.text_align` 単一 field に保持する — margin (`Sides<T>`) や
///   `content` (`normal`/`none` → 空 list) と同じ「shorthand as single field」
///   convention。text-align-all / text-align-last longhand 分離 (§6.2 / §6.3) は
///   future task で拡張。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **実装済み**: `match-parent` の **computed-value 時解決**
///   (spec §6.1 `#valdef-text-align-match-parent` verbatim: "This value behaves
///   the same as inherit (computes to its parent's computed value) except that
///   an inherited value of start or end is interpreted against the parent's
///   direction value and results in a computed value of either left or right.
///   Computes to start when specified on the root element.")。[`MatchParent`](Self::MatchParent)
///   は cascade winner としては specified value のまま
///   [`SpecifiedValues::text_align`](crate::specified::SpecifiedValues::text_align)
///   を経由するが、**computed 層に届く前に解決される** — element 経路は
///   [`crate::specified::SpecifiedValues::finalize`] /
///   [`crate::specified::SpecifiedValues::finalize_as_root`]、page 経路は
///   [`crate::cascade::resolve_against_inherited`] が、どちらも
///   [`resolve_text_align_match_parent`] へ funnel する。解決には親要素の
///   computed `direction` ([`Direction`]、CSS Writing Modes 4 §2.1) を要する。
///   [`crate::computed::ComputedValues::text_align`] に残る値は常に解決済 —
///   `MatchParent` が computed 値として観測されることは無い
///   (`resolve_text_align_match_parent` の debug_assert が check する不変条件)。
/// - **(a) spec-invalid**: CSS Text 3 §6.1 grammar は上記 8 keyword のみ。それ以外
///   の ident (`middle`, `baseline` 等、および CSS Text 4 draft 相当の `<string>`
///   character alignment は本 crate が引用する CSS Text 3 では未定義) は silent
///   drop = `None`。
///
/// [`DisplayValue`] と同じ convention で `Default` を derive しない — 本 enum の
/// `.default()` は呼ばれず、初期化側 [`crate::computed::ComputedValues::initial`]
/// が [`TextAlign::Start`] を直接指定する (sibling pattern:
/// [`DisplayValue`] / [`PositionValue`] は spec に "omitted → default" が無いため
/// non-derive、[`CounterStyle`] / [`StringFetchMode`] / [`ContentPart`] /
/// [`ContentTextKeyword`] は spec に omitted-default があるため derive)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    /// `start` — "Inline-level content is aligned to the start edge of the line
    /// box" (§6.1 spec verbatim)。spec initial value。writing-mode + direction
    /// で physical edge が決まる (horizontal-tb + LTR で physical left)。
    Start,
    /// `end` — "Inline-level content is aligned to the end edge of the line
    /// box" (§6.1 spec verbatim)。`start` の反対側。
    End,
    /// `left` — "Inline-level content is aligned to the line-left edge of the
    /// line box" (§6.1 spec verbatim)。**physical left ではなく line-left** —
    /// vertical writing modes では writing-mode に応じて physical top / bottom
    /// に写像され得る (spec 注記: "In vertical writing modes, this can be either
    /// the physical top or bottom, depending on writing-mode")。
    Left,
    /// `right` — "Inline-level content is aligned to the line-right edge of the
    /// line box" (§6.1 spec verbatim)。[`Left`](Self::Left) と同様、vertical
    /// writing modes では physical top / bottom に写像され得る。
    Right,
    /// `center` — "Inline-level content is centered within the line box"
    /// (§6.1 spec verbatim)。
    Center,
    /// `justify` — "Text is justified according to the method specified by the
    /// text-justify property, in order to exactly fill the line box" (§6.1 spec
    /// verbatim)。末行 (forced line break 前) は text-align-last の指定が無ければ
    /// start-aligned。
    Justify,
    /// `match-parent` — 親要素の text-align 計算値と一致させる (`start`/`end` を
    /// 親の direction で `left`/`right` に解決した後、その解決値を継承)。
    /// root element では `start` に fallback (§6.1 spec verbatim)。
    MatchParent,
    /// CSS-wide `inherit` for the inherited `text-align` property. It resolves
    /// directly to the parent computed value before layout.
    Inherit,
    /// HTML UA stylesheet の `-internal-center`。親の computed alignment が
    /// initial `start` のときだけ `center` に解決し、それ以外では親の値を
    /// 継承する。これは author-facing CSS Text grammar の値ではない。
    InternalCenter,
    /// `justify-all` — text-align-all と text-align-last の両方を justify に set、
    /// 末行にも justify を強制する (§6.1 spec verbatim)。
    JustifyAll,
}

/// `direction` property の value。
///
/// CSS Writing Modes 4 §2.1 "Specifying Directionality: the direction property"
/// <https://www.w3.org/TR/css-writing-modes-4/#direction>。
///
/// propdef (spec verbatim): Value: `ltr | rtl`、Initial: `ltr`、Applies to:
/// all elements、Inherited: **yes**、Computed value: specified value (= keyword
/// をそのまま保持、他 property に対する相対解決は無い)。
///
/// # なぜこの property が要るか
///
/// 本 crate は以前 `direction` を computed 層に持たなかった
/// (`ComputedValues` に field が無い)。CSS Text 3 §6.1
/// `#valdef-text-align-match-parent` の `text-align: match-parent` 解決 — "an
/// inherited value of start or end is interpreted against the parent's
/// direction value" — がこの property を要求するため追加した。用途は
/// [`TextAlign::MatchParent`] の解決に留まらない — CSS Paged Media 3 Appendix A
/// "CSS 2.1 Properties that apply within the page context"
/// <https://www.w3.org/TR/css-page-3/#page-property-list> の list 先頭に
/// `direction` 自体が挙げられている (verbatim 確認済) ので、`@page { direction:
/// rtl }` 単体でも page context の computed value として意味を持つ。
///
/// # Scope carving
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical。sibling [`TextAlign`] と同 convention)。
/// - **(a) spec-invalid**: `ltr` / `rtl` 以外の ident は silent drop = `None`。
///   spec には旧 draft 相当の `auto` 値は無い (現行 §2.1 grammar は 2 keyword のみ)。
/// - **Non-goal**: HTML `dir` attribute → UA-level `direction` mapping
///   (spec が "we recommend HTML authors to use the HTML dir attribute" と述べる
///   presentational hint) は本 crate の parse/cascade scope に無い — UA CSS
///   default 値の持ち込みは 独立実装 対象外の別 task。
///
/// [`DisplayValue`] / [`TextAlign`] と同じ convention で `Default` を derive
/// しない — 初期化側 [`crate::computed::ComputedValues::initial`] が
/// [`Direction::Ltr`] を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `ltr` — left-to-right。spec initial value。
    Ltr,
    /// `rtl` — right-to-left。
    Rtl,
}

/// `writing-mode` property の value。
///
/// CSS Writing Modes 4 §3.2 "Block Flow Direction: the writing-mode property"
/// <https://www.w3.org/TR/css-writing-modes-4/#propdef-writing-mode>。
///
/// propdef (spec verbatim): Value: `horizontal-tb | vertical-rl | vertical-lr
/// | sideways-rl | sideways-lr`、Initial: `horizontal-tb`、Applies to: "All
/// elements except table row groups, table column groups, table rows, table
/// columns, ruby base container, ruby annotation container"、Inherited:
/// **yes**、Percentages: n/a、Computed value: specified value、Animation
/// type: not animatable。
///
/// CSS Writing Modes **Level 3** <https://www.w3.org/TR/css-writing-modes-3/#propdef-writing-mode>
/// defines only 3 of these keywords (`horizontal-tb | vertical-rl |
/// vertical-lr`) — its own changelog records "Deferred the sideways-lr and
/// sideways-rl values of writing-mode to Level 4." `sideways-rl` /
/// `sideways-lr` only exist in the Level 4 propdef this doc cites, which is
/// also this crate's existing precedent for the sibling `direction` property
/// ([`Direction`] doc cites the same Level 4 document).
///
/// # `@page` context — Appendix A 非掲載、しかし意図的に拡張配線
///
/// [`Direction`] doc が引用する `direction` とは対照的に、`writing-mode`
/// 自体は CSS Paged Media 3 Appendix A page-property-list
/// <https://www.w3.org/TR/css-page-3/#page-property-list> の CSS 2.1 由来
/// table には **載っていない** (Appendix A の raw table を直接確認済)。ただし
/// raikiri は Appendix A を「床」であって「天井」ではないものとして扱う —
/// `overflow`/`overflow-x`/`overflow-y` / [`DisplayValue`] /
/// [`PositionValue`] / `box-sizing` / `counter-reset` / `counter-increment` /
/// `content` / `string-set` と同じ「Appendix A 非掲載だが意図的に `@page`
/// context へ拡張配線している」property の並びに `writing-mode` も加わる —
/// canonical な列挙と根拠 (CSS Paged Media 3 §6 の "positive minimum, not a
/// ceiling" の性質) は
/// [`crate::page::PageCascadeResult::declarations`] doc 参照。
///
/// 5 keyword の prose 定義 (spec verbatim、§3.2):
///
/// - [`HorizontalTb`](Self::HorizontalTb) — "Top-to-bottom block flow
///   direction. Both the writing mode and the typographic mode are
///   horizontal." spec initial value。
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
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 5 keyword 以外の ident は silent drop = `None`。
/// - **Non-goal — vertical writing-mode rendering pipeline は未実装**:
///   `vertical-rl` / `vertical-lr` / `sideways-rl` / `sideways-lr` は spec
///   grammar どおり **構文としては受理する** (`None` を返さない、CSS 2.1 の
///   「受理するが視覚効果は未実装」established pattern — sibling
///   [`WordBreak`] doc の deprecated `break-word` scope-limited と同じ精神)。ただし
///   raikiri は縦書きレンダリングパイプラインを持たないため、この 4 keyword の
///   **computed value はすべて [`HorizontalTb`](Self::HorizontalTb) と同じ表現に
///   正規化する** — [`resolve_writing_mode`] が実装する。これは spec の
///   "Computed value: specified value" (= computed 値は specified keyword を
///   そのまま保持する) からの意図的な divergence であり、spec 解釈の誤りでは
///   ない — 縦書き非対応という scope cut を正直に表現したもの。
///   将来 vertical writing-mode レンダリングを実装する際は、この collapse と
///   [`resolve_writing_mode`] を削除し、spec どおり "specified value" を保持
///   する computed value へ戻すこと — 将来の縦書き対応時に棚卸しする。
///
/// [`DisplayValue`] / [`TextAlign`] / [`Direction`] と同じ convention で
/// `Default` を derive しない — 初期化側
/// [`crate::computed::ComputedValues::initial`] が
/// [`WritingMode::HorizontalTb`] を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritingMode {
    /// `horizontal-tb` — spec initial value。
    HorizontalTb,
    /// `vertical-rl` — 構文としては受理するが、computed value は
    /// [`HorizontalTb`](Self::HorizontalTb) に正規化される
    /// ([`WritingMode`] doc の Non-goal 節参照)。
    VerticalRl,
    /// `vertical-lr` — [`VerticalRl`](Self::VerticalRl) と同じ Non-goal 扱い。
    VerticalLr,
    /// `sideways-rl` — [`VerticalRl`](Self::VerticalRl) と同じ Non-goal 扱い。
    SidewaysRl,
    /// `sideways-lr` — [`VerticalRl`](Self::VerticalRl) と同じ Non-goal 扱い。
    SidewaysLr,
}

/// `overflow-x` / `overflow-y` の共通 value type。
///
/// CSS Overflow Module Level 3 §3.1 "Overflow: the overflow-x, overflow-y,
/// overflow-block, overflow-inline, and overflow properties"
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>。
///
/// propdef (spec verbatim): Value: `visible |
/// hidden | clip | scroll | auto`、Initial: `visible`、Inherited: **no**、
/// Computed value: "usually specified value, but see text" — 本 crate は
/// text が指す cross-axis coupling を [`resolve_overflow`] で実装する
/// (詳細は同関数 doc)。
///
/// # 5 keyword の意味 (spec 確認済み verbatim)
///
/// - [`Visible`](Self::Visible) — "There is no special handling of overflow,
///   that is, the box's content is rendered outside the box if positioned
///   there." spec initial value。
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
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 未知 keyword は silent drop = `None`。
/// - **Non-goal**: `overflow-block` / `overflow-inline` logical longhand
///   (spec §3.1 propdef が同時に定義するが、raikiri-style は writing-mode
///   未実装のため物理 axis (x/y) にのみ写像する —
///   `crates/raikiri-html/src/ua/minimal.css` の `hr` rule comment が
///   `margin-block`/`margin-inline` について述べる carve out と同型の判断)。
///
/// sibling [`BoxSizing`] / [`Direction`] と同じ convention で `Default`
/// を derive しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`OverflowValue::Visible`]
/// を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowValue {
    /// `visible` — spec initial value。
    Visible,
    /// `hidden`。
    Hidden,
    /// `clip`。
    Clip,
    /// `scroll`。
    Scroll,
    /// `auto`。
    Auto,
}

/// `overflow-x` + `overflow-y` の pair holder。
///
/// [`Sides<T>`] (4-side box-model holder) の 2-axis sibling。`overflow`
/// shorthand の 1-2 value expansion (CSS Overflow 3 §3.1
/// `<'overflow-block'>{1,2}`、[`OverflowValue`] doc の Non-goal 節が説明する
/// とおり本 crate は物理 axis にそのまま写像する) と、cross-axis の
/// computed-value coupling ([`resolve_overflow`]) の両方が x/y を同時に
/// 読み書きするため、独立した 2 field ([`SpecifiedValues`]/[`ComputedValues`]
/// 直下の scalar field 2 つ) ではなく 1 struct に bundle する —
/// [`ComputedValues::border`] が `border-*-style` / `border-*-width` の
/// 同時参照のため `Sides<Border>` に bundle しているのと同型の設計判断
/// ([`crate::resolve::resolve_border`] doc 参照)。
///
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`ComputedValues::border`]: crate::computed::ComputedValues::border
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverflowXY {
    /// `overflow-x` に相当する axis。
    pub x: OverflowValue,
    /// `overflow-y` に相当する axis。
    pub y: OverflowValue,
}

impl OverflowXY {
    /// x/y を同じ値で埋める constructor — [`Sides::all`] の 2-axis 版。
    /// `overflow: <value>` の 1-value shorthand expansion (CSS Overflow 3
    /// §3.1 "If the second value is omitted, it is copied from the first.")
    /// と spec initial value (両 axis に `visible` を配る) の両方で使う。
    pub const fn both(v: OverflowValue) -> Self {
        Self { x: v, y: v }
    }
}

/// `overflow-x` / `overflow-y` の cross-axis computed-value coupling を解決する
/// (CSS Overflow 3 §3.1 verbatim):
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
/// 同一 node の 2 property (`overflow-x` / `overflow-y`) が互いの computed
/// value を決める **same-node cross-field dependency** —
/// [`resolve_text_align_match_parent`] (親の computed 値に依存) とは異なり、
/// 依存先は自 node 内の**もう一方の axis**のみ。
/// [`crate::resolve::resolve_border`] の style→width gating
/// (`border-*-style` が `border-*-width` の computed value を決める) と同型の
/// 「同一 node 内の sibling property が computed value を決める」パターンで
/// あり、両方とも **phase 3** (絶対化) で解決する — element 経路は
/// [`crate::specified::SpecifiedValues::finalize`] (内部の `absolutize_with`)、
/// page 経路は [`crate::page::cascade_page`] の phase 3 (`page_context_overflow_pair`
/// で両 axis の winner を先に集めてから本関数へ渡す)。
///
/// `pub(crate)` — 呼び手は `specified` / `page` の 2 module のみ。
pub(crate) fn resolve_overflow(specified: OverflowXY) -> OverflowXY {
    /// 1 axis 分の解決。`other` が "neither visible nor clip" (= hidden /
    /// scroll / auto のいずれか) なら `this` の `visible`→`auto` /
    /// `clip`→`hidden` を適用する。`other` の判定を `Visible | Clip` の
    /// allowlist に対する `matches!` で書いているのは、[`OverflowValue`] が
    /// `#[non_exhaustive]` なため将来 variant が増えても、その未知 variant は
    /// allowlist に一致せず自動的に「neither visible nor clip」側 (=
    /// fallback 適用) に倒れる fail-safe な形にするため — `resolve_border`
    /// の未知 `BorderStyle` variant を「visible 側」に倒す fail-safe と同じ
    /// 判断。
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

/// [`WritingMode`]'s specified→computed collapse ([`WritingMode`] doc の
/// Non-goal 節参照)。
///
/// raikiri は縦書きレンダリングパイプラインを実装しないため、5 keyword の
/// うち [`WritingMode::HorizontalTb`] 以外の 4 つ (`vertical-rl` /
/// `vertical-lr` / `sideways-rl` / `sideways-lr`) は **常に**
/// [`WritingMode::HorizontalTb`] と同じ computed value に正規化する。
/// [`resolve_text_align_match_parent`] / [`resolve_overflow`] とは異なり
/// **他 field (親の computed 値・同 node の他 property) に一切依存しない** —
/// 引数の keyword に関わらず戻り値は固定 (`self` すら実質不要だが、他の
/// `resolve_*` 関数と同じ signature shape を保つため受け取る)。
///
/// # Future work — vertical writing-mode 実装時の棚卸し
///
/// 本関数は CSS Writing Modes 4 "Computed value: specified value" からの
/// 意図的な divergence である。将来 vertical writing を実装する際は、本関数
/// 自体を削除し、`specified.rs` の
/// `finalize_collapses_all_non_horizontal_writing_modes` /
/// `inherit_from_then_finalize_still_collapses_writing_mode`、`page.rs` の
/// `absolutize_in_page_context_collapses_writing_mode_to_horizontal_tb` /
/// `KEYWORD_TRANSFORMED_WITHOUT_RAW_RESIDUE` / `WritingMode(VerticalRl)` corpus
/// sample、`cascade.rs` の `writing_mode_wired_through_cascade_from_inline_style`、
/// `computed.rs` の `non_initial_parent` fixture + `HorizontalTb` assertion と
/// lockstep で revert/rewrite すること。
///
/// # 呼び出し元
///
/// - Element 経路: [`crate::specified::SpecifiedValues::absolutize_with`]
///   (`finalize` / `finalize_as_root` の両方がここへ funnel する — root か
///   どうかで分岐する必要が無いのは、spec がこの property に root 固有の
///   特別扱いを定めていないため、[`resolve_text_align_match_parent`] の
///   "Computes to start when specified on the root element" 分岐と異なる点)。
/// - Page 経路: [`crate::page`] の `absolutize_in_page_context`
///   (page context 自身も同じ無条件正規化を受ける)。
///
/// `pub(crate)` は `specified` / `page` の 2 module から呼ぶため。
pub(crate) fn resolve_writing_mode(specified: WritingMode) -> WritingMode {
    match specified {
        WritingMode::HorizontalTb
        | WritingMode::VerticalRl
        | WritingMode::VerticalLr
        | WritingMode::SidewaysRl
        | WritingMode::SidewaysLr => WritingMode::HorizontalTb,
    }
}

/// `text-align: match-parent` の解決 (CSS Text 3 §6.1
/// `#valdef-text-align-match-parent`
/// <https://www.w3.org/TR/css-text-3/#valdef-text-align-match-parent> verbatim):
///
/// > This value behaves the same as inherit (computes to its parent's computed
/// > value) except that an inherited value of start or end is interpreted
/// > against the parent's direction value and results in a computed value of
/// > either left or right. Computes to start when specified on the root
/// > element.
///
/// 本関数が扱うのは前半 (**実の親を持つ場合**) だけ — `specified` が
/// [`TextAlign::MatchParent`] でなければ no-op (他 keyword は解決不要、
/// computed value = specified value)。`parent_text_align` / `parent_direction`
/// は実の親要素 (または `@page` の場合は inheritance parent = root_style) の
/// computed 値。
///
/// **後半 ("Computes to start when specified on the root element") はこの
/// 関数の対象外** — root element (親を持たない element) の特別扱いは呼び出し側
/// ([`crate::specified::SpecifiedValues::finalize_as_root`]) が別途行う。
/// `@page` の "The page context inherits from the root element" は「親を持た
/// ない」ケースでは**ない** — page context が root element そのものになるわけ
/// ではないので、page 経路は常に本関数 (親あり分岐) を通る
/// ([`crate::page::PageCascadeResult::declarations`] doc の trap 注記参照)。
///
/// # 呼び出し元 (resolve_relative_weight と同型の contract)
///
/// - Element 経路: [`crate::specified::SpecifiedValues::finalize`] /
///   [`crate::specified::SpecifiedValues::finalize_as_root`]。
/// - Page 経路: [`crate::cascade::resolve_against_inherited`]。
///
/// 両経路とも本関数へ funnel するので、"start/end を親の direction で
/// left/right に解決する" table の実装は 1 箇所にしか無い (CSS Fonts 4
/// bolder/lighter table を `resolve_relative_weight` 1 箇所に集約した
/// precedent を踏襲)。
///
/// **なぜ element 経路の呼び手が `apply_value` ではないか**: `apply_value`
/// は同一 node 上の他 winner (`direction` 自身を含む) が [`PropertyKey`]
/// 宣言順に順次 [`crate::specified::SpecifiedValues`] へ書き込まれる場所であり、
/// この関数が要る「**親の** direction」は自 node の `direction` winner の
/// 適用順序に左右されてはならない (適用順に依存しないことが
/// [`crate::cascade::resolve_inheritance`] の invariant)。`finalize` /
/// `finalize_as_root` は全 winner 適用後に**明示的に親の
/// [`crate::computed::ComputedValues`] を受け取って**呼ばれるため、この罠を
/// 構造的に避けられる。
///
/// `pub(crate)` は `specified` / `cascade` の 2 module から呼ぶため。
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
                // `left` / `right` / `center` / `justify` / `justify-all` — 親の
                // 解決済み値をそのまま継承 (spec の「behaves the same as
                // inherit」)。防御的に `MatchParent` もここへ落ちるが、上の
                // debug_assert が release では消えるため fallback として
                // そのまま伝播する (パニックしない crate policy)。
                other => other,
            }
        }
        // match-parent 以外は spec 上 "as specified" — 解決不要。
        other => other,
    }
}

/// HTML UA stylesheet の `text-align: -internal-center` と CSS-wide `inherit` を
/// 親の computed 値に対して解決する。Blink/WebKit の table-header default と同じく、
/// internal center は親が initial `start` の場合だけ中央寄せを選び、author が
/// table 側で別の alignment を指定した場合はその値を継承する。`inherit` は常に
/// 親の値を継承する。通常の CSS parser から author-facing grammar としては
/// internal 値を公開しないが、UA stylesheet は同じ declaration pipeline を通る。
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

/// `position` property の value — static-side scope では `static` (default) と
/// GCPM `running(<custom-ident>)` および CSS Positioned Layout `sticky` を受理する。
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// は element を normal flow から取り除き、`element()` 経由で page margin box に
/// 配置可能な template として登録する。
///
/// CSS Positioned Layout Module Level 3 §3 "Sticky positioning"
/// <https://www.w3.org/TR/css-position-3/#sticky-pos>: `position: sticky` は
/// normal flow 内でレイアウトされつつ、scroll container に対して sticky に
/// 振る舞う。本 crate では parse 段階で [`PositionValue::Sticky`] として保持し、
/// layout 連携は将来対応 — 現状は `static` 同様に `apply_value` で no-op
/// (running template を emit しない) として扱う。`relative` / `absolute` /
/// `fixed` は未実装 (将来対応)、silent drop (`None`)。
///
/// `static` を明示的に variant 化しているのは、
/// 先行の `position: running(x)` を later cascade で上書き無効化する用途
/// (`.foo { position: running(hdr) } .foo.reset { position: static }` の
/// 後者が winner になったとき、`apply_value` は no-op、`inherit_from` 起点で
/// 空 `running_templates` が残る)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionValue {
    /// `static` — spec default、running() を suppress。
    ///
    /// `Default` は derive しない — 本 crate の convention は "derive `Default`
    /// iff `.default()` が call される" (sibling [`DisplayValue`] と同じ、
    /// spec default は初期化側 [`crate::computed::ComputedValues::initial`] が
    /// 直接指定する)。
    Static,
    /// `relative` — CSS Positioned Layout Module Level 3 §3 relative positioning.
    /// Normal flow 内でレイアウトされ、inset offsets で paint 時に shift。
    Relative,
    /// `absolute` — CSS Positioned Layout Module Level 3 §3 absolute positioning.
    /// 現状 parse のみ、layout では static と同様 (future work)。
    Absolute,
    /// `fixed` — CSS Positioned Layout Module Level 3 §3 fixed positioning.
    /// 現状 parse のみ、layout では static と同様 (future work)。
    Fixed,
    /// `sticky` — CSS Positioned Layout Module Level 3 §3 sticky positioning
    /// (<https://www.w3.org/TR/css-position-3/#sticky-pos>)。normal flow 内で
    /// レイアウトされつつ scroll に対して sticky に振る舞う。本 crate では parse 段階で [`PositionValue::Sticky`] として保持し、
    /// layout 連携は将来対応 — 現状は `static` 同様に `apply_value` で no-op
    /// (running template を emit しない) として扱う。`relative` / `absolute` /
    /// `fixed` は未実装 (将来対応)、silent drop (`None`)。
    Sticky,
    /// `running(<custom-ident>)`。`<custom-ident>` は case-preserved の smol str。
    Running(SmolStr),
}

/// `text-decoration-line` の keyword payload。
///
/// CSS Text Decoration Module Level 3 §2.1 "Text Decoration Lines: the
/// text-decoration-line property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>
/// value grammar: `none | [ underline || overline || line-through || blink ]`、
/// Initial: `none`、Inherited: **no** (draw 段の伝播規則は別途 prose にあるが、
/// cascade の inherited/non-inherited 分類には効かない)、Computed value:
/// specified keyword(s)。
///
/// # `||` (any-order) grammar と bool flag 表現
///
/// spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> の `||`
/// semantics (each component 最大 1 回、at least 1 個必須、順序自由) は
/// [`parse_border_shorthand`] の `||` (width || style || color) と同型 —
/// 詳細な rationale は同関数 doc 参照。4 keyword が独立に on/off なので
/// 16 通りの組み合わせを持つが、CSS Values 4 の `||` は「同じ component の
/// 2 回目の出現」を許さない (各 alternative は集合として高々 1 回) だけで
/// あり、16 通りの组み合わせ自体は grammar 上すべて valid。よって専用
/// enum (16 variant) ではなく 4 independent `bool` field の struct で表現する
/// — `none` は全 flag `false` (spec 上 `none` と「4 keyword とも
/// 不使用」は同じ状態)。
///
/// CSS Text Decoration 4 の `spelling-error` / `grammar-error` は `||`
/// group の外側の top-level alternative (`none | [ ... ] |
/// spelling-error | grammar-error`) のため、残り 2 `bool` field
/// ([`Self::spelling_error`] / [`Self::grammar_error`]) として保持し、
/// `||` group との併記は parser 側で reject する
/// ([`parse_text_decoration_line`] 参照)。
///
/// `#[non_exhaustive]` を付けない — sibling [`OverflowXY`] と同じ判断
/// (umbrella (`raikiri` crate) へ再 export されておらず、CSS spec が
/// 定める keyword は Level 4 時点で 6 (`||` group 4 + top-level 2) のため、
/// 将来 field 追加の蓋然性が [`Border`] ほど高くない)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextDecorationLine {
    /// `underline` — テキストの under edge に沿って装飾線を引く。
    pub underline: bool,
    /// `overline` — テキストの over edge に沿って装飾線を引く。
    pub overline: bool,
    /// `line-through` — テキストの中央を貫く装飾線を引く。
    pub line_through: bool,
    /// `blink` — 装飾線を点滅させる (spec note: UA は本 keyword を無視してよい、
    /// paint 側の実装判断)。
    pub blink: bool,
    /// `spelling-error` — UA 定義の綴り誤り装飾 (CSS Text Decoration 4 §2.1
    /// <https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-line>
    /// の `none | [ underline || overline || line-through || blink ] |
    /// spelling-error | grammar-error` — top-level alternative のため `||`
    /// group とは併記不可)。
    pub spelling_error: bool,
    /// `grammar-error` — 同上、文法誤り装飾。
    pub grammar_error: bool,
}

impl TextDecorationLine {
    /// `none` — spec initial value。装飾線なし (6 flag 全て `false`)。
    pub const NONE: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `underline` 単独。
    pub const UNDERLINE: Self = Self {
        underline: true,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `overline` 単独。
    pub const OVERLINE: Self = Self {
        underline: false,
        overline: true,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `line-through` 単独。
    pub const LINE_THROUGH: Self = Self {
        underline: false,
        overline: false,
        line_through: true,
        blink: false,
        spelling_error: false,
        grammar_error: false,
    };
    /// `blink` 単独。
    pub const BLINK: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: true,
        spelling_error: false,
        grammar_error: false,
    };
    /// `spelling-error` 単独。
    pub const SPELLING_ERROR: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: true,
        grammar_error: false,
    };
    /// `grammar-error` 単独。
    pub const GRAMMAR_ERROR: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: false,
        spelling_error: false,
        grammar_error: true,
    };
}

/// `text-decoration-style` の keyword payload。
///
/// CSS Text Decoration Module Level 3 §2.2 "Text Decoration Style: the
/// text-decoration-style property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>、
/// value grammar: `solid | double | dotted | dashed | wavy`、Initial: `solid`、
/// Inherited: no、Computed value: specified keyword。
///
/// `Default` は derive しない — sibling [`DisplayValue`] / [`Direction`] と
/// 同じ convention (spec default は初期化側
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
///
/// `#[non_exhaustive]` — sibling [`BorderStyle`] と同じ判断 (line-style 系
/// keyword enum の慣行、future variant の non-breaking 追加)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationStyle {
    /// `solid` — spec initial value。
    Solid,
    /// `double`。
    Double,
    /// `dotted`。
    Dotted,
    /// `dashed`。
    Dashed,
    /// `wavy`。
    Wavy,
}

/// `text-decoration-color` computed value — [`BorderColor`] と同型の
/// `currentcolor` keyword / resolved `<color>` distinction。
///
/// CSS Text Decoration Module Level 3 §2.3 "Text Decoration Color: the
/// text-decoration-color property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>、
/// value grammar: `<color>`、Initial: `currentcolor`、Inherited: no、
/// Computed value: computed color。used-value resolution (currentcolor →
/// 同 node の computed `color` property) は paint scope 責務 — rationale は
/// [`BorderColor`] doc の「なぜ cascade static side で enum 保持するか」節と
/// 同型 (`text-decoration` shorthand も `color` winner 確定前に構築されうる)。
///
/// `#[non_exhaustive]` — sibling [`BorderColor`] と同じ判断。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationColor {
    /// `currentcolor` keyword — spec-mandated initial value。
    CurrentColor,
    /// Resolved `<color>` value — author が hex / named / `rgb(a)` /
    /// `transparent` で明示指定した場合の payload。
    Resolved(CssColor),
}

/// `text-decoration` shorthand の parse 結果を一時的に保持する carrier。
///
/// CSS Text Decoration Module Level 3 §2.4 "Text Decoration Shorthand: the
/// text-decoration property"
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-property> verbatim:
/// "This property is a shorthand for setting text-decoration-line,
/// text-decoration-color, and text-decoration-style in one declaration.
/// Omitted values are set to their initial values." — grammar
/// `<'text-decoration-line'> || <'text-decoration-thickness'> ||
/// `<'text-decoration-style'> || <'text-decoration-color'>`
/// (Level 3 §2.4 の 3 成分に Level 4 の thickness が追加、
/// ED <https://drafts.csswg.org/css-text-decor-4/#text-decoration-property>)。
///
/// [`PropertyValue::TextDecoration`] の payload としてのみ存在し、
/// [`crate::rule::expand_shorthand_into`] が
/// [`PropertyValue::TextDecorationLine`] / [`PropertyValue::TextDecorationThickness`] /
/// [`PropertyValue::TextDecorationStyle`] / [`PropertyValue::TextDecorationColor`] の 4 longhand へ展開した後は捨てられる
/// — margin/padding/border/overflow shorthand precedent と同じ「parse-time
/// expansion, never reaches cascade」設計 (詳細は同関数 doc)。[`ComputedValues`]
/// / [`SpecifiedValues`] は本型を **field として持たない** — 3 longhand が
/// 互いに computed-value coupling を持たないため、[`OverflowXY`] のような
/// bundling の根拠 (同型 doc の「cross-axis coupling」節) が本 shorthand には
/// 無い (詳細は 3 longhand 各 field の doc)。
///
/// `#[non_exhaustive]` を付けない — sibling [`TextDecorationLine`] と同じ判断
/// (shorthand-only carrier で umbrella 再 export 対象外)。
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDecorationShorthand {
    /// `text-decoration-line` 成分 — 省略時は [`TextDecorationLine::NONE`]
    /// (spec initial)。
    pub line: TextDecorationLine,
    /// `text-decoration-style` 成分 — 省略時は [`TextDecorationStyle::Solid`]
    /// (spec initial)。
    pub style: TextDecorationStyle,
    /// `text-decoration-color` 成分 — 省略時は [`TextDecorationColor::CurrentColor`]
    /// (spec initial)。
    pub color: TextDecorationColor,
    /// `text-decoration-thickness` 成分 — 省略時は
    /// [`TextDecorationThickness::Auto`] (ED §2.4.1 initial)。
    pub thickness: TextDecorationThickness,
}

/// `text-decoration-skip-ink: auto | none | all` の value.
///
/// CSS Text Decoration 4
/// (<https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-skip-ink>)。
/// Value: `auto | none | all`、Initial: `auto`、Inherited: **yes**、
/// Computed value: specified keyword。
/// parsing-only ([`PropertyValue::TextDecorationSkipInk`] doc 参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationSkipInk {
    /// `auto` — spec initial value。
    Auto,
    /// `none`。
    None,
    /// `all`。
    All,
}

/// `text-decoration-skip-spaces: none | all | [ start || end ]` の value.
///
/// CSS Text Decoration 4
/// (<https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-skip-spaces>)。
/// Value: `none | all | [ start || end ]`、Initial: `start end`、
/// Inherited: **yes**、Computed value: specified keyword(s)。
/// `all` は `start end` と区別する (initial が `start end` であって
/// `all` ではないため) — 5 variant enum で表現する。
/// parsing-only ([`PropertyValue::TextDecorationSkipSpaces`] doc 参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationSkipSpaces {
    /// `none`。
    None,
    /// `all`。
    All,
    /// `start` のみ。
    Start,
    /// `end` のみ。
    End,
    /// `start end` / `end start` (順序自由、spec initial value)。
    StartEnd,
}

/// `text-decoration-thickness: auto | from-font | <length-percentage>` の value.
///
/// CSS Text Decoration 4
/// (<https://www.w3.org/TR/css-text-decor-4/#propdef-text-decoration-thickness>,
/// ED §2.4.1 は `<line-width>` も含む、下の Scope carving 参照)。
/// Value: `auto | from-font | <length-percentage>`、Initial: `auto`、
/// Inherited: **no**、Percentages: N/A、Computed value: specified keyword
/// or absolute length。
/// ED grammar は `<line-width>` (`thin`/`medium`/`thick`) も含むが、本実装は
/// scope 外として drop する (WPT vector に現れない — scope carving)。
/// `<percentage>` は受理して保持する (同)。
/// parsing-only ([`PropertyValue::TextDecorationThickness`] doc 参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextDecorationThickness {
    /// `auto` — spec initial value。
    Auto,
    /// `from-font`。
    FromFont,
    /// `<length-percentage>` ([`parse_length_value`] の `allow_percentage=true`
    /// 範囲 — [`Length::Percent`] を含む)。
    Length(Length),
}

/// `text-decoration-inset: <length>{1,2} | auto` の value.
///
/// CSS Text Decoration 4 ED §2.9.1 (<https://drafts.csswg.org/css-text-decor-4/#text-decoration-inset-property>)。
/// ED grammar は `<length-percentage>{1,2} | auto` だが、WPT
/// (`text-decoration-inset-invalid.html` の `10%` reject) が `%` を認めない
/// ため、本実装は `<length>{1,2} | auto` に絞る (vector が ground truth —
/// 乖離としてここに記録する)。
/// Initial: `0`、Inherited: **no**。2 値目は省略時に 1 値目を複製する
/// (margin/padding の 2-value 規則と同型)。
/// The cascade and paint pipeline carries this value through computed style;
/// [`PropertyValue::TextDecorationInset`] is the specified-stage representation.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextDecorationInset {
    /// `auto`。
    Auto,
    /// 1-2 `<length>`。`end` 省略時は `start` と同値。
    Lengths {
        /// start endpoint offset。
        start: Length,
        /// end endpoint offset。
        end: Length,
    },
}

/// `text-emphasis-position` の vertical 成分 (`[ over | under ]`)。
///
/// CSS Text Decoration 3 §3.4 (<https://www.w3.org/TR/css-text-decor-3/#text-emphasis-position-property>)
/// …ではなく ED <https://drafts.csswg.org/css-text-decor-4/#text-emphasis-position-property>
/// の `[ over | under ] && [ right | left ]?` の前半 (vertical は必須)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisVEdge {
    /// `over`。
    Over,
    /// `under`。
    Under,
}

/// `text-emphasis-position` の horizontal 成分 (`[ right | left ]?`)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisHEdge {
    /// `right`。
    Right,
    /// `left`。
    Left,
}

/// `text-emphasis-position: auto | ([ over | under ] && [ right | left ]?)` の value.
///
/// ED §3.4 (<https://drafts.csswg.org/css-text-decor-4/#text-emphasis-position-property>)。
/// Value: `[ over | under ] && [ right | left ]?` (+ `auto`)、
/// Initial: `over right`、Inherited: **yes**。vertical 必須・horizontal
/// 任意・順序自由 (`right under` valid、`left over right` invalid)。
/// parsing-only ([`PropertyValue::TextEmphasisPosition`] doc 参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEmphasisPosition {
    /// `auto`。
    Auto,
    /// vertical (+ optional horizontal)。
    Position {
        /// `[ over | under ]` (必須)。
        vertical: TextEmphasisVEdge,
        /// `[ right | left ]?` (任意)。
        horizontal: Option<TextEmphasisHEdge>,
    },
}

/// `text-underline-position: auto | [ from-font | under ] || [ left | right ]` の value.
///
/// CSS Text Decoration 4 ED §2.7 (<https://drafts.csswg.org/css-text-decor-4/#text-underline-position-property>)。
/// Value: `auto | [ from-font | under ] || [ left | right ]`、
/// Initial: `auto`、Inherited: **yes**。
/// `from-font` と `under` は排他 (`under from-font` invalid)、`left` と
/// `right` も排他 (`left right` invalid)、`auto` は単独
/// (`auto under` invalid) — [`TextDecorationLine`] と同じ bool-flag +
/// parser-enforcement 表現。
/// parsing-only ([`PropertyValue::TextUnderlinePosition`] doc 参照)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextUnderlinePosition {
    /// `from-font` (`under` と排他)。
    pub from_font: bool,
    /// `under` (`from-font` と排他)。
    pub under: bool,
    /// `left` (`right` と排他)。
    pub left: bool,
    /// `right` (`left` と排他)。
    pub right: bool,
}

impl TextUnderlinePosition {
    /// `auto` — spec initial value (全 flag `false`)。
    pub const AUTO: Self = Self {
        from_font: false,
        under: false,
        left: false,
        right: false,
    };
}

/// `page: auto | <custom-ident>` の value。
///
/// CSS Paged Media 3 §8.1 "Using named pages: page"
/// (<https://www.w3.org/TR/css-page-3/#using-named-pages>)。
/// Value: `auto | <custom-ident>`、Initial: `auto`、
/// Applies to: boxes that create class A break points、Inherited: **no**、
/// Computed value: specified value。
/// `<custom-ident>` は CSS-wide keyword を除く単一 ident —
/// WPT (`page-invalid.html`) が `default` も reject するため
/// 同様に除外する。`not valid` (2 ident) / `123px` /
/// `calc()` は grammar 外のため一般 mechanism
/// (single-ident parse + caller `expect_exhausted`) で drop される。
/// parsing-only ([`PropertyValue::Page`] doc 参照)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageValue {
    /// `auto` — spec initial value。
    Auto,
    /// 名前付きページ (`<custom-ident>`)。
    Named(Atom),
}

/// `vertical-align` property の value.
///
/// CSS 2.1 §10.8.1 "Vertical alignment: the 'vertical-align' property"
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>。
///
/// propdef (spec verbatim): Value: `baseline | sub | super | top | text-top
/// | middle | bottom | text-bottom | <percentage> | <length> | inherit`、
/// Initial: `baseline`、Applies to: inline-level and 'table-cell' elements、
/// Inherited: **no**、Percentages: refer to the 'line-height' of the element
/// itself、Computed value: "for `<percentage>` and `<length>` the absolute
/// length, otherwise as specified".
///
/// # なぜ CSS 2.1 を primary source に採るか
///
/// CSS Inline Layout Module Level 3
/// <https://www.w3.org/TR/css-inline-3/#vertical-align> は `vertical-align`
/// を `alignment-baseline` / `baseline-source` / `baseline-shift` 3
/// longhand の shorthand として再定義するが、classic keyword grammar
/// (`baseline` / `sub` / `super` / `top` / ... ) 全体への互換 mapping 節を
/// 持たない。classic keyword grammar の完全かつ一貫した定義を持つのは
/// CSS 2.1 §10.8.1 のみであるため、本 crate はそちらを primary source に
/// 採る。
///
/// # Scope carving
///
/// - **実装済み**: `baseline` / `sub` / `super` / `top` / `bottom` の
///   keyword と、minimal line-box scope の edge placement。
///   `baseline` (spec initial value) / `sub` / `super` の 3
///   keyword。いずれも percentage / length を運ばないため、computed value =
///   specified keyword そのまま (相対解決なし)。raikiri-paint がこの 3
///   keyword を実際の glyph 描画位置へ反映する (下記「baseline shift 量の
///   計算は raikiri-paint scope」節)。
/// - **実装済み**: `middle` / `text-top` / `text-bottom` の 3 keyword
///   (§10.8.1 spec verbatim):
///   - `middle`: "Align the vertical midpoint of the box with the
///     baseline of the parent box plus half the x-height of the parent."
///   - `text-top`: "Align the top of the box with the top of the
///     parent's content area."
///   - `text-bottom`: "Align the bottom of the box with the bottom of
///     the parent's content area."
///
///   いずれも `sub`/`super` と同じ基準 — **親の font metric だけ**
///   (baseline / x-height / content area の top・bottom) で定まり、line
///   box 内の他 box の extent を必要としない。`top`/`bottom` は下記の
///   minimal line-box scope で別途扱う。percentage /
///   length を運ばないため computed value = specified keyword そのまま。
/// - **実装済み**: `<length>` value (§10.8.1 spec verbatim: "Raise
///   (positive value) or lower (negative value) the box by this
///   distance. The value `0cm` means the same as `baseline`.")。基準
///   (line-height / font metrics) を必要としない絶対値であり、既存の
///   length resolver ([`crate::resolve::resolve_length`]、
///   `letter-spacing`/`word-spacing` の `<length>` 成分と同じ経路) で
///   そのまま近似なしに絶対化できる。sign 制限なし (spec が明示的に負値を
///   許容、`letter-spacing`/`margin-*` と同じ扱い)。`<percentage>` は
///   grammar 上の別の alternative であり本 variant には含まれない (下記
///   「非対応: `<percentage>`」節)。
///
///   Computed value の型は specified と同じ [`VerticalAlign`] のまま —
///   [`crate::computed::ComputedValues::vertical_align`] doc の「computed
///   でも型を分けない理由」節参照。`@page` 側の phase-3 pipeline
///   ([`crate::page`] の `absolutize_in_page_context` /
///   `specified_layer_residue`) もこの variant 専用の match arm を持つ。
/// - **実装済み (minimal line-box scope)**: `top` / `bottom` keyword は
///   CSS 2.1 §10.8.1 の line-box edge alignment として parse/cascade される。
///   `establish_minimal_line_boxes` の taffy bridge は direct
///   inline-level child の `bottom` を `flex-end` へ写像し、nested inline
///   wrapper の block-axis padding が edge-aligned subtree をずらさないよう
///   その padding をこの narrow slice では除外する。これは full baseline/
///   strut/nested-inline flattening の実装ではなく、単一 minimal line box の
///   focused behavior である。
/// - **実装済み**: `<percentage>` value (§10.8.1 propdef
///   "Percentages: refer to the 'line-height' of the element itself")。
///   要素自身の used `line-height` に対する比率として絶対化する
///   ([`crate::resolve::resolve_vertical_align`] doc 参照)。`50%` →
///   `Length::Percent(50.0)` を parse し、phase 3 で
///   `used_line_height_length * p / 100` に解決する。`0%` は spec 上
///   `baseline` と同義 (上記 `<length>` 節の "`0cm` means the same as
///   `baseline`" と同型)。負値も spec-valid として受理する (`<length>`
///   と同じ "Raise/lower"  semantics)。
///
///   **`line-height: normal` 時の spec-deviation fallback**: `line-height:
///   normal` (spec initial value、宣言が無い要素の既定) の下では
///   [`crate::resolve::used_line_height_length`] が `None` を返す —
///   real font metrics を style 層に持たないため "normal" を絶対長化できない
///   (同関数 doc の "normal" wall が canonical)。本来は used line-height
///   が font metrics 由来の絶対長を持つため percentage も自然に解決するが、
///   本 crate が font-metrics source を持つまで (parley
///   統合 milestone) は `0px` (= `baseline` 相当) に倒す — これは比率を
///   捏造しない independent fallback であり、`padding` / `margin` が
///   `<percentage>` に対して採る「絶対化せず computed 層まで素通しし、
///   使用先で解決する」staging とは異なり、素通し先の consumer
///   (raikiri-dom / raikiri-paint) が今日時点で
///   [`crate::computed::ComputedValues::line_height`] を読まないため
///   選択した per-property fallback である。`Length::Lh` / `Length::Rlh`
///   の `None` → `0px` fallback ([`crate::resolve::resolve_length`] doc)
///   と同型の documented deviation。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記以外の ident は silent drop = `None`。
/// - **baseline shift 量の計算は raikiri-paint scope**: `sub` / `super` /
///   `middle` / `text-top` / `text-bottom` が指す実際の shift 量計算
///   (parent's used font-size ないし font metrics を基準にした px offset)
///   と glyph 描画位置への反映は raikiri-paint 側の責務。本 crate はこの
///   5 keyword の bare keyword、および `<length>` の絶対化済み px 値を
///   運ぶだけで、shift 量の算出は行わない。raikiri-paint は現状 `sub`/
///   `super` の 2 keyword だけを明示的な match arm で shift 計算しており、
///   他 (`middle`/`text-top`/`text-bottom`/`<length>` を含む) は
///   `#[non_exhaustive]` wildcard fallback 経由の 0px shift で暫定着地する
///   (下記「cascade-regression risk の受け入れ」節)。
///
/// # cascade-regression risk の受け入れ (`Middle`/`TextTop`/`TextBottom`)
///
/// この節は本 doc の以前の版が明文化していた原則からの意図的な逸脱を記録
/// する。以前の版は「raikiri-paint が shift を実装していない keyword は
/// parse 段でも受理しない」方針を採っていた — 理由: UA/author が (実装済み
/// の) `sub`/`super` より高い cascade priority で (未実装の) keyword を
/// 宣言した場合、その宣言は cascade 上正当に winner になるが raikiri-paint
/// は shift 0 として扱うため、**それまで正しく shift していた要素が
/// silent に shift 0 へ後退する** — 「未対応の値が単に無効果」ではなく
/// 「対応済みの値が押しのけられて後退する」という質的に異なるリスクだった
/// ためである。
///
/// `Middle`/`TextTop`/`TextBottom` はこの原則の明示的な例外として追加した。
/// 根拠: (1) 3 keyword とも `sub`/`super` と同じ「親の font metric だけで
/// 定まる」基準を持ち (上記「実装済み」節)、計算可能性の質は `sub`/`super`
/// と同等 — `top`/`bottom` (line box 全体依存) とは異なる。(2)
/// raikiri-paint 側の shift 計算 arm はもともと `#[non_exhaustive]`
/// wildcard で「`sub`/`super` 以外の全 keyword」を一様に 0px shift として
/// 扱う設計だったため、この 3 keyword が増えても新種の failure mode は
/// 生じない — 追加される regression risk の形は `sub`/`super` が既に
/// 許容しているものと同型であり、対象 keyword が増えるだけである。
/// `top`/`bottom` は style 層では受理し、minimal line-box layout へ渡す。
/// `<percentage>` は上記「実装済み: `<percentage>`」節の
/// `line-height: normal` fallback を伴い実装済み — `top`/`bottom` とは異なり
/// raikiri-style 内部で完結して絶対化できるため、cascade-regression risk
/// (未実装 keyword が cascade 上で実装済み値を押しのける) とは無関係な
/// 別種の gap だったが、本対応で fallback を check して解消した。
///
/// `Default` は derive しない — sibling [`TextDecorationShorthand`] と
/// 同じ convention (spec default は初期化側
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
///
/// `Eq` は derive しない — [`Self::Length`] が運ぶ [`Length`] が `f32`
/// field を持つため ([`FlexBasisValue`] と同じ制約)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VerticalAlign {
    /// `baseline` — spec initial value。box の baseline を親の baseline に
    /// 揃える (追加のシフトなし、§10.8.1 spec verbatim: "Align the baseline
    /// of the box with the baseline of the parent box.")。
    Baseline,
    /// `sub` — "Lower the baseline of the box to the proper position for
    /// subscripts of the parent's box. (This value has no effect on the
    /// font size of the element's text.)" (§10.8.1 spec verbatim)。
    Sub,
    /// `super` — "Raise the baseline of the box to the proper position for
    /// superscripts of the parent's box. (This value has no effect on the
    /// font size of the element's text.)" (§10.8.1 spec verbatim)。
    Super,
    /// `middle` — "Align the vertical midpoint of the box with the
    /// baseline of the parent box plus half the x-height of the parent."
    /// (§10.8.1 spec verbatim)。
    Middle,
    /// `text-top` — "Align the top of the box with the top of the
    /// parent's content area." (§10.8.1 spec verbatim)。
    TextTop,
    /// `text-bottom` — "Align the bottom of the box with the bottom of
    /// the parent's content area." (§10.8.1 spec verbatim)。
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
    /// same as `baseline`." (§10.8.1 spec verbatim、`<percentage>` は同 propdef
    /// の "Percentages: refer to the 'line-height' of the element itself"
    /// により `line-height` 基準で絶対長へ解決)。computed 層では絶対化済みの
    /// `Length::Px` を運ぶ ([`Self`] doc の「実装済み: `<percentage>`」節および
    /// [`crate::resolve::resolve_vertical_align`] doc 参照 — `normal` 時は
    /// `0px` fallback)。
    Length(Length),
}

/// `z-index` property の value。
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
/// `Default` は derive しない — [`Direction`] / [`BoxSizing`] と同じ
/// convention (spec default は初期化側
/// [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
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

/// `word-break` property の value。
///
/// CSS Text Module Level 3 §5.1 "Breaking Rules for Letters: the word-break
/// property" <https://www.w3.org/TR/css-text-3/#word-break-property>。
///
/// propdef (spec verbatim): Value: `normal | keep-all | break-all |
/// break-word`、Initial: `normal`、Applies to: text、Inherited: **yes**、
/// Computed value: specified keyword。
///
/// # 3 keyword の意味 (spec 確認済み verbatim)
///
/// - [`Normal`](Self::Normal) — "Words break according to their customary
///   rules, as described above. Korean, which commonly exhibits two
///   different behaviors, allows breaks between any two consecutive
///   Hangul/Hanja. For Ethiopic, which also exhibits two different
///   behaviors, such breaks within words are not allowed." spec initial
///   value。
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
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 3 keyword (`normal`/`keep-all`/`break-all`、
///   `break-word` は上記 Non-goal 節参照) 以外の ident は silent drop =
///   `None`。
///
/// この crate の scope では length を運ばないため、computed value = specified
/// keyword、相対解決なし ([`Direction`] doc と同型)。
///
/// [`Direction`] / [`FontStyle`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`WordBreak::Normal`]
/// を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordBreak {
    /// `normal` — spec initial value。
    Normal,
    /// `keep-all`。
    KeepAll,
    /// `break-all`。
    BreakAll,
    /// `manual` — CSS Text 4 / WPT word-break-valid.
    Manual,
    /// `auto-phrase` — CSS Text 4 / WPT word-break-valid.
    AutoPhrase,
    /// `break-word` — deprecated but WPT expects valid (word-break-valid.html).
    BreakWord,
}

/// `overflow-wrap` property の value (legacy name alias `word-wrap` は同一
/// property を指す — 下記「legacy alias」節参照)。
///
/// CSS Text Module Level 3 §5.4 "Overflow Wrapping: the overflow-wrap
/// (word-wrap) property"
/// <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>。
///
/// propdef (spec verbatim): Value: `normal | break-word | anywhere`、
/// Initial: `normal`、Applies to: text、Inherited: **yes**、Computed value:
/// specified keyword。
///
/// # legacy alias (`word-wrap`)
///
/// spec verbatim: "For legacy reasons, UAs must treat word-wrap as a legacy
/// name alias of the overflow-wrap property." — `parse_value` dispatches
/// both the `"overflow-wrap"` and `"word-wrap"` property names to the same
/// [`PropertyValue::OverflowWrap`] variant / [`PropertyKey::OverflowWrap`]
/// key, so the two names cascade against each other as one property (a
/// declaration under either name can win over a declaration under the
/// other), not as two independently-winning properties.
///
/// # 3 keyword の意味 (spec 確認済み verbatim)
///
/// - [`Normal`](Self::Normal) — "Lines may break only at allowed break
///   points. However, the restrictions introduced by word-break: keep-all
///   may be relaxed to match word-break: normal if there are no
///   otherwise-acceptable break points in the line." spec initial value。
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
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 3 keyword 以外の ident は silent drop =
///   `None`。
///
/// この crate の scope では length を運ばないため、computed value = specified
/// keyword、相対解決なし ([`Direction`] doc と同型)。
///
/// [`Direction`] / [`WordBreak`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`OverflowWrap::Normal`]
/// を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowWrap {
    /// `normal` — spec initial value。
    Normal,
    /// `break-word`。
    BreakWord,
    /// `anywhere`。
    Anywhere,
}

/// `break-before` / `break-after` property の value ([`PropertyValue::BreakBefore`]
/// / [`PropertyValue::BreakAfter`] が共有する — 両 property は同一 grammar を
/// 持つ)。
///
/// CSS Fragmentation Module Level 3 §3.1 "Breaks Between Boxes: the
/// break-before and break-after properties"
/// <https://www.w3.org/TR/css-break-3/#break-between>。
///
/// propdef (spec verbatim): Value: `auto | avoid | avoid-page | page | left
/// | right | recto | verso | avoid-column | column | avoid-region |
/// region`, Initial: `auto`, Applies to: "block-level boxes, grid items,
/// flex items, table row groups, table rows (but see prose)", Inherited:
/// **no**, Computed value: "specified keyword".
///
/// # Scope carving
///
/// This type implements only 4 of the propdef's 12 keywords:
///
/// - `auto` / `avoid` — the "Generic Break Values" (§3.1): apply regardless
///   of fragmentation context.
/// - `avoid-page` / `page` — the "Page Break Values" (§3.1): the only
///   fragmentation context this crate models is pagination, not
///   multi-column or CSS Regions.
///
/// Excluded:
///
/// - `avoid-column` / `column` ("Column Break Values") and `avoid-region` /
///   `region` ("Region Break Values") — this crate has no multi-column or
///   CSS Regions fragmentation context to break within.
/// - `left` / `right` / `recto` / `verso` — page-spread-parity forced
///   breaks; this crate has no page-spread concept.
/// - `always` / `all` — **not part of Level 3's spec grammar at all**.
///   Level 3's own change log records "Dropped `any` and `always` values of
///   `break-*`" (removed between the January 2015 Working Draft and the
///   current text) — that entry does not mention `all`; both `always` and
///   `all` instead reappear as forced-break values in CSS Fragmentation
///   Module Level 4 <https://www.w3.org/TR/css-break-4/> (a First Public
///   Working Draft as of this writing, with `all` itself marked at-risk
///   there), a spec version this crate does not target. Of the two, only
///   `always` has any path into this crate at all, and only indirectly:
///   the CSS2.1 `page-break-before` / `page-break-after` legacy shorthand
///   grammar below still has it, remapped to `page` — it is never a valid
///   `break-before` / `break-after` value on its own. `all` has no path
///   into this crate.
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
/// `Default` は derive しない — [`ZIndexValue`] と同じ convention (spec
/// default は初期化側 [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
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
}

/// `break-inside` property の value.
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
/// This type implements only 3 of the propdef's 5 keywords — `auto` /
/// `avoid` (apply regardless of fragmentation context) and `avoid-page`
/// (the only fragmentation context this crate models). Excluded, same
/// rationale as [`BreakBetween`] doc's "Scope carving" section: `avoid-column`
/// (no multi-column fragmentation context) and `avoid-region` (no CSS
/// Regions fragmentation context).
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
/// `Default` は derive しない — [`ZIndexValue`] と同じ convention。
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
}

/// `float` property の value。
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
///   crate yet ([`PositionValue`] doc's "未実装" note). CSS2 §9.7's
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
/// `Default` は derive しない — [`ZIndexValue`] と同じ convention (spec
/// default は初期化側 [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
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

/// `clear` property の value。
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
/// `Default` は derive しない — [`FloatValue`] と同じ convention (spec
/// default は初期化側 [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
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

/// `float` が `none` 以外のときに CSS2 §9.7 "Relationships between
/// 'display', 'position', and 'float'"
/// <https://www.w3.org/TR/CSS2/visuren.html#dis-pos-flo> が強制する
/// `display` の computed-value 変換を解決する。spec verbatim:
///
/// > Otherwise, if 'float' has a value other than 'none', the box is
/// > floated and 'display' is set according to the table below.
///
/// 同 §の表 (verbatim):
///
/// | Specified value | Computed value |
/// |---|---|
/// | `inline-table` | `table` |
/// | `inline`, `table-row-group`, `table-column`, `table-column-group`, `table-header-group`, `table-footer-group`, `table-row`, `table-cell`, `table-caption`, `inline-block` | `block` |
/// | others | same as specified |
///
/// この crate の [`DisplayValue`] scope は table 系も含む 18 variant
/// (`block` / `inline` / `inline-block` / `none` / `flex` / `grid`
/// / `list-item` / `contents` / `table` / `inline-table` / `table-row-group`
/// / `table-header-group` / `table-footer-group` / `table-row`
/// / `table-column-group` / `table-column` / `table-cell` / `table-caption`)
/// を持つ。CSS2 §9.7 の表に照らすと:
/// - `inline-table` → `table` (表 1 行目)
/// - `inline`, `table-row-group`, `table-column`, `table-column-group`,
///   `table-header-group`, `table-footer-group`, `table-row`, `table-cell`,
///   `table-caption`, `inline-block` → `block` (表 2 行目)
/// - `block`, `table`, `flex`, `grid`, `list-item`, `none`, `contents` は
///   "others" (same as specified) — floated でもそのまま。`flex`/`grid`/
///   `list-item` が "others" に落ちるのは従来通り。
///
/// # `display: none` は本関数の呼び出し前に別枝で処理される
///
/// §9.7 冒頭の verbatim: "If 'display' has the value 'none', then
/// 'position' and 'float' do not apply." — この分岐は表より**前**にあり、
/// 表を経由しない。したがって [`DisplayValue::None`] は明示的な
/// early-return で守る (他の未知 variant と同じ「表に登場しない ==
/// same as specified」の一般ルールには**委ねない** — `None` がその一般
/// ルールと同じ結果になるのは偶然の一致であり、将来 [`resolve_overflow`]
/// 型の fail-safe 拡張で意味が変わりうる区別を明示するため)。
///
/// # `display: contents` も強制変換の対象外 (§9.7 とは別の spec 根拠)
///
/// CSS2 §9.7 の表自体は `contents` を扱わない (`contents` は CSS2 に無い
/// 新しい keyword)。代わりに CSS Display Module Level 3 §2.7 "Automatic
/// Box Type Transformations"
/// <https://www.w3.org/TR/css-display-3/#transformations> がこの表を含む
/// blockification 全般について verbatim で述べる: "This has no effect on
/// display types that generate no box at all, such as `display: none` or
/// `display: contents`." — floated `contents` 要素はそもそも box を
/// 生成しないため、float によるこの強制変換自体が適用されない
/// ([`DisplayValue::None`] と同じ結論だが、根拠となる spec 文は別)。
///
/// # 同一 node の cross-field dependency
///
/// [`resolve_overflow`] と同型の same-node coupling (依存先は自 node 内の
/// もう一方の property のみ、親の値には依存しない) — 呼び出し箇所も
/// phase 3 (絶対化) の同じ場所
/// ([`crate::specified::SpecifiedValues::finalize`] 内部の
/// `absolutize_with`)。
///
/// # page 経路では呼ばれない
///
/// `@page` box は §9.7 が想定する「visual formatting context 内の
/// element」ではない (page box 自体を float させる CSS 機構は存在しない)
/// ため、[`crate::page::cascade_page`] の phase 3 はこの解決を行わず、
/// `Float` / `Clear` を [`ZIndexValue`] と同じ opaque pass-through として
/// 扱う ([`crate::page`] の `absolutize_in_page_context` の該当 arm 参照)。
///
/// `pub(crate)` — 呼び手は `specified` module のみ。
pub(crate) fn resolve_display_for_float(display: DisplayValue, float: FloatValue) -> DisplayValue {
    if matches!(float, FloatValue::None) {
        return display;
    }
    match display {
        DisplayValue::None => DisplayValue::None,
        // 関数 doc の「`display: contents` も強制変換の対象外」節 —
        // box を生成しない display type には blockification 自体が
        // 適用されない (CSS Display Module Level 3 §2.7 verbatim)。
        DisplayValue::Contents => DisplayValue::Contents,
        // CSS2 §9.7 表 1 行目: `inline-table` → `table`
        DisplayValue::InlineTable => DisplayValue::Table,
        // CSS2 §9.7 表 2 行目: `inline`, `table-row-group`, `table-column`,
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
        // 残りは "others" — same as specified。`Block` / `Table` / `Flex` /
        // `Grid` / `ListItem` / `FlowRoot` を明示列挙し、将来 variant 追加時に非網羅で
        // compile error にする (`#[non_exhaustive]` は crate 外部向け、
        // 定義 crate 内部のこの match には適用されない)。
        same @ (DisplayValue::Block
        | DisplayValue::Table
        | DisplayValue::Flex
        | DisplayValue::Grid
        | DisplayValue::ListItem
        | DisplayValue::FlowRoot) => same,
    }
}

/// `white-space` property の value。
///
/// CSS Text Module Level 3 §3 "White Space and Wrapping: the white-space
/// property" <https://www.w3.org/TR/css-text-3/#white-space-property>。
///
/// propdef (spec verbatim): Value: `normal | pre | nowrap | pre-wrap |
/// break-spaces | pre-line`、Initial: `normal`、Applies to: text、Inherited:
/// **yes**、Computed value: "specified keyword"。
///
/// # 5 keyword の意味 (spec 確認済み verbatim)
///
/// - [`Normal`](WhiteSpace::Normal) — "This value directs user agents to collapse
///   sequences of white space into a single character (or in some cases, no
///   character). Lines may wrap at allowed soft wrap opportunities." spec
///   initial value。
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
/// spec の informative summary table (collapsing 有無 / wrapping 有無の 2 軸)
/// が示すとおり、5 keyword は独立な 2 behavior の組み合わせで決まる —
/// (a) white space の collapse 有無 (`normal`/`nowrap`/`pre-line` は
/// collapse、`pre`/`pre-wrap` は preserve)、(b) line wrap の有無
/// (`normal`/`pre-wrap`/`pre-line` は wrap、`pre`/`nowrap` は no wrap)。
///
/// # Scope carving
///
/// - **Non-goal**: spec の 6th keyword `break-spaces` — spec verbatim: "The
///   behavior is identical to that of pre-wrap, except that any sequence of
///   preserved white space always takes up space, including at the end of
///   the line." `pre-wrap` との差は行末の保存済み space が実際に space を
///   占有するかどうかという line-breaking の used-value 計算に属する差
///   であり、本 crate はまだ line box を持たない ([`WordBreak`] doc の
///   deprecated `break-word` non-goal と同型の carve-out)。他の未知 ident
///   と同じく silent drop = `None` とする。
/// - **Downstream handoff**: white space の実際の collapsing / line
///   wrapping algorithm 自体 (spec 冒頭の summary table が要約する 2 axis
///   の適用) は、この crate がまだ持たない text layout / line-breaking
///   consumer (raikiri-dom / raikiri-paint 側) の仕事であり、この property
///   は cascade static-side keyword しか運ばない ([`TextTransform`] doc の
///   "Downstream handoff" 節と同型)。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 5 keyword (`break-spaces` は上記 Non-goal 節
///   参照) 以外の ident は silent drop = `None`。
///
/// この crate の scope では length を運ばないため、computed value = specified
/// keyword、相対解決なし ([`Direction`] doc と同型)。
///
/// [`Direction`] / [`WordBreak`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`WhiteSpace::Normal`]
/// を直接指定する。
/// `text-wrap-mode` value (CSS Text 4 §5.1), carried as the `text-wrap`
/// shorthand's wrapping component.
///
/// Only the single-keyword `wrap | nowrap` subset is parsed; the full
/// `text-wrap` shorthand (wrap-style `auto | balance | pretty | stable`)
/// is deferred. Inherited, initial `wrap`, computed value = specified
/// keyword. `nowrap` suppresses soft wrapping in raikiri-dom preshape and
/// realign.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextWrapMode {
    /// `wrap` — spec initial value.
    Wrap,
    /// `nowrap`.
    Nowrap,
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
    /// `normal` — spec initial value。
    Normal,
    /// `pre`。
    Pre,
    /// `nowrap`。
    Nowrap,
    /// `pre-wrap`。
    PreWrap,
    /// `pre-line`。
    PreLine,
    /// `break-spaces`。
    BreakSpaces,
}

/// `hyphens` property の value。
///
/// CSS Text Module Level 3 §5.3 "Hyphenation: the hyphens property"
/// <https://www.w3.org/TR/css-text-3/#hyphens-property>。
///
/// propdef (spec verbatim): Value: `none | manual | auto`、Initial: `manual`、
/// Applies to: text、Inherited: **yes**、Computed value: specified keyword。
///
/// # 3 keyword の意味 (spec 確認済み verbatim)
///
/// - [`None`](Self::None) — "Words are not hyphenated, even if characters
///   inside the word explicitly define hyphenation opportunities."
/// - [`Manual`](Self::Manual) — "Words are only hyphenated where there are
///   characters inside the word that explicitly suggest hyphenation
///   opportunities." spec initial value。explicit な hyphenation opportunity
///   の代表例が soft hyphen (`U+00AD`、HTML では `&shy;`) — 同 §
///   "In Unicode, U+00AD is a conditional 'soft hyphen'" 参照。
/// - [`Auto`](Self::Auto) — "Words may be broken at hyphenation
///   opportunities determined automatically by a language-appropriate
///   hyphenation resource in addition to those indicated explicitly by a
///   conditional hyphen."
///
/// # Scope carving
///
/// - **Non-goal**: `auto` の "determined automatically by a
///   language-appropriate hyphenation resource" (辞書ベースの自動
///   hyphenation) は本 crate の scope 外 — content language の検出も
///   言語別 hyphenation resource もこの crate は持たない。`auto` と
///   `manual` は spec 上明確に区別される 2 keyword であり、本 type は両方を
///   distinct variant として represent する — [`OverflowWrap`] doc の
///   Scope carving 節が `BreakWord`/`Anywhere` について述べる判断と同型
///   (この crate の layer では区別を観測できなくても、将来の
///   layout/hyphenation consumer のために variant 自体は残す)。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 3 keyword 以外の ident は silent drop =
///   `None`。
///
/// # Downstream handoff
///
/// 実際に word のどこで hyphenation opportunity が発生するかの計算
/// (soft hyphen 位置の走査、`auto` の dictionary lookup) は本 crate の
/// scope 外 — text-shaping/paint 層の consumer が読む cascade static-side
/// keyword しか本 property は運ばない ([`TextTransform`] doc の
/// 「Downstream handoff」節と同型)。**dictionary-based automatic
/// hyphenation を持たない downstream consumer が [`Auto`](Self::Auto) を
/// 安全に扱う唯一の方法は [`Manual`](Self::Manual) と同じ soft hyphen
/// (`U+00AD`) のみの分割** — この対応は downstream consumer 側の実装判断
/// として明示的に文書化する (silent な仕様省略にしない)。computed value
/// 自体は spec どおり 3 keyword を区別したまま保持する — CSSOM
/// round-trip、および将来 dictionary-based hyphenation resource を追加した
/// ときに `Auto`/`Manual` を再び分岐できる forward-compat のため。
///
/// この crate の scope では length を運ばないため、computed value = specified
/// keyword、相対解決なし ([`Direction`] doc と同型)。
///
/// [`Direction`] / [`WordBreak`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`Hyphens::Manual`]
/// を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hyphens {
    /// `none`。
    None,
    /// `manual` — spec initial value。
    Manual,
    /// `auto`。この crate の scope では [`Manual`](Self::Manual) と同じ
    /// soft-hyphen-only 分割として downstream consumer が扱う想定 — 詳細は
    /// 上記型 doc の「Downstream handoff」節参照。
    Auto,
}

/// `tab-size` property の value。
///
/// CSS Text Module Level 3 §4.2 "Tab Character Size: the tab-size property"
/// (<https://www.w3.org/TR/css-text-3/#tab-size-property>), value grammar
/// `<number [0,∞]> | <length [0,∞]>`. Initial: `8`. Inherited: yes.
/// Percentages: N/A.
///
/// [`LineHeight`] と同じ number-vs-length split の shape だが 2 branch のみ —
/// `normal` keyword を持たない点が異なる:
///
/// - [`Number`](Self::Number) — `<number [0,∞]>`。spec 本文 "A `<number>`
///   represents the measure as a multiple of the advance width of the space
///   character (U+0020) of the nearest block container ancestor of the
///   preserved tab, including its associated letter-spacing and
///   word-spacing." — この font metric 依存の解決は、本 crate がまだ持たない
///   text layout consumer (raikiri-dom / raikiri-paint) の仕事であり、本
///   crate は unitless multiplier を素通しするだけ ([`LineHeight::Number`]
///   と同じ scope carving)。
/// - [`Length`](Self::Length) — `<length [0,∞]>`。percentage を持たない点が
///   [`LineHeight::Length`] (`<length-percentage>`) と異なる — spec propdef
///   の "Percentages: N/A" が根拠。
///
/// # Non-negative constraint
///
/// spec grammar `[0,∞]` (両 branch とも) — 本文 "Negative values are not
/// allowed." により負値は invalid → parser 側で drop (`parse_tab_size` の
/// post-filter、spec-invalid → drop)。
///
/// # `<length>` alternative の CR status
///
/// spec は `<length>` alternative を "at risk" (CR プロセス中に取り下げ
/// られる可能性がある feature) とマークしている。本実装は TR に記載の現行
/// grammar をそのまま実装する — 取り下げが実際に発生したら別途対応する。
///
/// Downstream match は必ず wildcard arm を持つこと (`#[non_exhaustive]`
/// 属性、変数追加が既存 pattern-match を break しない forward-compat 契約、
/// sibling [`LineHeight`] / [`Length`] と同 pattern)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TabSize {
    /// `<number [0,∞]>` — advance width of the space character (U+0020) の
    /// multiplier。computed 層でも number のまま (font metric 依存の解決は
    /// downstream consumer の仕事、type doc 参照)。
    Number(f32),
    /// `<length [0,∞]>` — absolute tab size。percentage は持たない (type doc
    /// の "Percentages: N/A" 節参照)。
    Length(Length),
}

/// `line-break` property の value (CSS Text 3 §5.2).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineBreak {
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}

/// `text-justify` property の value (CSS Text 3 §6.2).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextJustify {
    Auto,
    None,
    InterWord,
    InterCharacter,
    /// legacy `distribute` (CSS Text 3 §6.2 で `inter-character` の別名扱い
    /// だった旧値 — WPT text-justify-distribute-001 が使用)。
    /// parley 側に区別が無いため consumer では `Justify` と同扱い。
    Distribute,
}

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

/// `text-align-all` property の value (CSS Text 3 §6.1 longhand).
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

/// `text-align-last` property の value (CSS Text 3 §6.1 longhand).
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

/// `text-combine-upright` property の value (CSS Writing Modes 3 §9.1).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCombineUpright {
    None,
    All,
}

/// `text-orientation` property の value (CSS Writing Modes 3 §5.1).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOrientation {
    Mixed,
    Upright,
    Sideways,
}

/// `unicode-bidi` property の value (CSS Writing Modes 3 §2.2).
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

/// `table-layout` property の value.
///
/// CSS Tables 3 §4 "Table Layout Algorithm"
/// <https://www.w3.org/TR/css-tables-3/#table-layout-property>
/// (前身 CSS 2.1 §17.5.2 "Table width algorithms: the 'table-layout'
/// property" <https://www.w3.org/TR/CSS2/tables.html#width-layout>).
/// Value: `auto | fixed`、Initial: `auto`、Applies to: `table` /
/// `inline-table`、Inherited: **no**、Computed value: "as specified".
///
/// - `auto` — automatic table layout (content-driven column sizing、
///   CSS Tables 3 §5)。
/// - `fixed` — fixed table layout (table width + first-row / `col`
///   specified widths drive column sizing、content は overflow しうる、
///   CSS Tables 3 §5 の fixed branch)。
///
/// Layout-time の column sizing 自体は raikiri-dom scope
/// ([`crate::computed::ComputedValues::table_layout`] 参照) — 本 crate は
/// cascaded keyword を運ぶのみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableLayoutValue {
    /// `auto` — spec initial value (automatic table layout)。
    Auto,
    /// `fixed` — fixed table layout。
    Fixed,
}

/// `border-collapse` property の value.
///
/// CSS Tables 3 §6 "Borders"
/// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>
/// (前身 CSS 2.1 §17.6 "Borders"
/// <https://www.w3.org/TR/CSS2/tables.html#borders>).
/// Value: `collapse | separate`、Initial: `separate`、Applies to: `table` /
/// `inline-table`、Inherited: **yes**、Computed value: "as specified".
///
/// - `separate` — separated borders model (cell spacing あり)。
/// - `collapse` — collapsing borders model (隣接 border は conflict
///   resolution で 1 本に潰れる)。
///
/// Conflict resolution 自体は raikiri-dom scope
/// ([`crate::computed::ComputedValues::border_collapse`] 参照) — 本 crate は
/// cascaded keyword を運ぶのみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderCollapseValue {
    /// `separate` — spec initial value (separated borders model)。
    Separate,
    /// `collapse` — collapsing borders model。
    Collapse,
}

/// `caption-side` property の value.
///
/// CSS Tables 3 §7 "Caption Position: the caption-side property"
/// <https://www.w3.org/TR/css-tables-3/#caption-side-property>
/// (前身 CSS 2.1 §17.4 "Tables in the visual formatting model ...
/// Caption position and alignment"
/// <https://www.w3.org/TR/CSS2/tables.html#caption-position>).
/// Value: `top | bottom`、Initial: `top`、Applies to: `table-caption`,
/// Inherited: **yes**、Computed value: "as specified".
///
/// - `top` — caption box を table box の上 (block-start 側) に置く。
/// - `bottom` — caption box を table box の下 (block-end 側) に置く。
///
/// Caption box の配置自体は raikiri-dom scope
/// ([`crate::computed::ComputedValues::caption_side`] 参照) — 本 crate は
/// cascaded keyword を運ぶのみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionSideValue {
    /// `top` — spec initial value。
    Top,
    /// `bottom` — caption below the table box。
    Bottom,
}

/// `empty-cells` property の value.
///
/// CSS Tables 3 §8 "Empty Cells: the empty-cells property"
/// <https://www.w3.org/TR/css-tables-3/#empty-cells-property>
/// (前身 CSS 2.1 §17.5.1 "Table layers and transparency"
/// <https://www.w3.org/TR/CSS2/tables.html#empty-cells>).
/// Value: `show | hide`、Initial: `show`、Inherited: **yes**、
/// Computed value: "as specified".
///
/// - `show` — 空 cell の border / background を描く (separated borders
///   model でのみ効果を持つ)。
/// - `hide` — 空 cell の border / background を描かない。
///
/// Cell background / border の paint 判定自体は raikiri-dom /
/// raikiri-paint scope
/// ([`crate::computed::ComputedValues::empty_cells`] 参照) — 本 crate は
/// cascaded keyword を運ぶのみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyCellsValue {
    /// `show` — spec initial value。
    Show,
    /// `hide` — 空 cell の border / background を隠す。
    Hide,
}

/// `border-spacing` の specified value。
///
/// CSS Tables 3 §6.1 "Separated borders: the border-spacing property"
/// <https://www.w3.org/TR/css-tables-3/#border-spacing-property>
/// (前身 CSS 2.1 §17.6.1 "The separated borders model"
/// <https://www.w3.org/TR/CSS2/tables.html#separated-borders>).
/// Value grammar: `<length>{1,2}`、Initial: `0`、Applies to: `table` /
/// `inline-table`、Inherited: **yes**、Computed value:
/// "two absolute lengths"、Percentages: N/A、"Negative lengths are illegal"
/// (spec 本文 — parse 時に reject、calc 由来の computed-time 負値は
/// [`crate::resolve::resolve_border_spacing`] が `0` に clamp する)。
///
/// 第 2 成分省略時は第 1 成分の値をそのまま copy する (spec 本文:
/// "If only one value is specified, it applies to both the horizontal and
/// vertical spacing") — [`GapShorthand`] と同じ single-doubles 形。
/// 2 成分は horizontal / vertical の順。
///
/// Computed value が "two absolute lengths" のため、phase 3 で
/// [`crate::resolve::resolve_border_spacing`] が各成分を絶対化する —
/// keyword 素通しの sibling [`CaptionSideValue`] / [`EmptyCellsValue`]
/// とはこの点で異なる。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderSpacingValue {
    /// Horizontal (inline-axis) spacing — 第 1 成分。
    pub horizontal: Length,
    /// Vertical (block-axis) spacing — 第 2 成分、省略時は `horizontal`。
    pub vertical: Length,
}

/// `text-shadow`/// `text-shadow` の 1 shadow entry が運ぶ `<color>` 成分 — [`BorderColor`] /
/// [`TextDecorationColor`] と同型の `currentcolor` keyword / resolved
/// `<color>` distinction。
///
/// CSS Text Decoration Module Level 3 §4 "Text Shadows: the text-shadow
/// property" <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>:
/// "Values are interpreted as for box-shadow." — box-shadow の `<shadow>`
/// syntax (CSS Backgrounds 3 §6.1 "Drop Shadows: the box-shadow property"
/// <https://www.w3.org/TR/css-backgrounds-3/#box-shadow>)
/// で `<color>` が省略された場合、used-value は `currentcolor` と同じ
/// (CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>)。
/// used-value resolution (currentcolor → 同 node の computed `color`
/// property) は paint scope 責務 — rationale は [`BorderColor`] doc の「なぜ
/// cascade static side で enum 保持するか」節と同型。
///
/// `#[non_exhaustive]` — sibling [`BorderColor`] / [`TextDecorationColor`]
/// と同じ判断 (future variant、例: CSS Color 4 §6.2 system-color keyword の
/// non-breaking 追加)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextShadowColor {
    /// `currentcolor` keyword — `<color>` 省略時の spec-mandated 扱い
    /// (box-shadow 経由の継承、上記 doc 参照)。
    CurrentColor,
    /// Resolved `<color>` value — author が hex / named / `rgb(a)` /
    /// `transparent` で明示指定した場合の payload。
    Resolved(CssColor),
}

/// `text-shadow` の 1 shadow entry (comma-separated list の 1 要素)。
///
/// CSS Text Decoration Module Level 3 §4
/// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property> —
/// "Values are interpreted as for box-shadow. (But note that spread values
/// and the inset keyword are not allowed.)" box-shadow の `<shadow>` syntax
/// (CSS Backgrounds 3 §6.1 "Drop Shadows: the box-shadow property") を
/// `inset` と 4 番目の length (spread-distance)
/// を除いた形で narrow したもの — grammar は `<color>? && <length>{2,3}`
/// (offset-x, offset-y, 省略可能な blur-radius)。
///
/// # 各成分の初期値埋め (省略成分)
///
/// - `blur_radius` 省略 → `Length::Px(0.0)` — spec の computed value 定義
///   ("a list, each item consisting of three absolute lengths plus a
///   computed color") が blur-radius を常に 3 番目の length として要求する
///   ため、[`parse_border_shorthand`] の「省略成分は spec の initial value で
///   埋める」precedent に倣い parse 時点で eager に埋める (`Option<Length>`
///   を specified 層まで持ち越さない)。
/// - `color` 省略 → [`TextShadowColor::CurrentColor`] — [`BorderColor`] の
///   `color.unwrap_or(BorderColor::CurrentColor)` precedent と同型
///   ([`parse_border_shorthand`] 参照)。
///
/// # Non-negative blur-radius
///
/// blur-radius (3 番目の length) は non-negative — CSS Backgrounds 3 §6.1
/// "Drop Shadows: the box-shadow property" の `<shadow>` syntax (box-shadow /
/// text-shadow 共通) が blur-radius / spread
/// distance に "Negative values are invalid" を課す。offset-x / offset-y
/// (1・2 番目の length) にこの制約は無い (負値可、box-shadow の offset と
/// 同型)。parse 側の実装は [`parse_text_shadow_lengths`] 参照。
///
/// # `#[non_exhaustive]`
///
/// future field (例: box-shadow 導入時に共有する `spread`/`inset` 相当の
/// 拡張余地) の non-breaking 追加のため — sibling [`Border`] と同 pattern。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadowItem {
    /// `offset-x` — `<length>` (percentage 不可、CSS Text Decoration Module
    /// Level 3 §4 "Percentages: N/A")。負値可。
    pub offset_x: Length,
    /// `offset-y` — [`Self::offset_x`] と同じ grammar。
    pub offset_y: Length,
    /// `blur-radius` — `<length [0,∞]>`。省略時 `Length::Px(0.0)` (上記
    /// doc 参照)。
    pub blur_radius: Length,
    /// `<color>` 成分 — 省略時は [`TextShadowColor::CurrentColor`] (上記
    /// doc 参照)。
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

/// `border-radius` の four-corner `<length>` value。
///
/// CSS Backgrounds and Borders Level 3 §5
/// <https://www.w3.org/TR/css-backgrounds-3/#border-radius> の shorthand を
/// parse-time に四隅へ展開した形。corner の順序は top-left, top-right,
/// bottom-right, bottom-left (clockwise) で、`1`/`2`/`3` value の省略規則も
/// `parse_border_radius` が適用する。`<percentage>`、slash で指定する楕円形状、
/// longhand は本 task の scope 外である。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderRadius {
    /// top-left corner radius。
    pub top_left: Length,
    /// top-right corner radius。
    pub top_right: Length,
    /// bottom-right corner radius。
    pub bottom_right: Length,
    /// bottom-left corner radius。
    pub bottom_left: Length,
}

/// `box-shadow` の comma-separated list の 1 entry。
///
/// CSS Backgrounds and Borders Level 3 §6.1
/// <https://www.w3.org/TR/css-backgrounds-3/#box-shadow> の offset、optional
/// blur、optional spread、optional color、optional `inset` を保持する。offset
/// と spread は負値を許し、blur は non-negative に制限する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadowItem {
    /// horizontal offset。
    pub offset_x: Length,
    /// vertical offset。
    pub offset_y: Length,
    /// blur radius。省略時は `0px`。
    pub blur_radius: Length,
    /// spread distance。省略時は `0px`。
    pub spread_radius: Length,
    /// color。省略時は `currentcolor`。
    pub color: TextShadowColor,
    /// Whether the shadow is painted inside the border box (`inset`).
    pub inset: bool,
}

/// `outline-color` の keyword/color payload。
///
/// CSS Basic User Interface Module Level 3 §4.4
/// <https://www.w3.org/TR/css-ui-3/#outline-color> の `invert | <color>` を
/// 保持する。`<color>` に含まれる `currentcolor` も、resolved color と区別して
/// cascade static side に残す。`invert` は outline 専用であり、border の
/// [`BorderColor`] には追加しない。
///
/// `Default` は derive しない。spec initial (`invert`) は
/// [`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`] が明示的に設定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineColor {
    /// `invert` — CSS UI 3 §4.4 の spec initial value。
    Invert,
    /// `currentcolor` keyword。used-value 解決は paint scope の責務。
    CurrentColor,
    /// Resolved `<color>` value。
    Resolved(CssColor),
}

/// `outline` shorthand の specified value。
///
/// CSS Basic User Interface Module Level 3 §4
/// <https://www.w3.org/TR/css-ui-3/#outline-props> の width/style/color を
/// any-order で保持する。outline は border と異なり box model の寸法を変えない。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    /// outline width。省略時は `medium` (CSS UI 3 §4.2)。
    pub width: Length,
    /// outline style。省略時は [`OutlineStyle::None`] (CSS UI 3 §4.3)。
    pub style: OutlineStyle,
    /// outline color。省略時は [`OutlineColor::Invert`] (CSS UI 3 §4.4)。
    pub color: OutlineColor,
}

/// `<position>` value type の 1 軸分の offset (CSS Values and Units 4 §8.3
/// <https://www.w3.org/TR/css-values-4/#typedef-position>、CSS Backgrounds
/// and Borders 3 §2.6 `<bg-position>` <https://www.w3.org/TR/css-backgrounds-3/#typedef-bg-position>
/// が `background-position` 向けにこの grammar を拡張したものを、他
/// property 向けに一般化して再利用する共通 value type)。
///
/// # Grammar (CSS Backgrounds 3 §2.6 `<bg-position>` — 汎用 `<position>`
/// (CSS Values 4 §8.3) の superset)
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
/// **上記コード片は `<bg-position>` (この crate が `background-position`
/// 向けに実装している grammar、[`parse_bg_position`] 参照) であって、
/// `<position>` 自体ではない点に注意** — 最後の alternative の
/// `<length-percentage>?` が両 group で独立に optional なのは
/// `<bg-position>` 固有の拡張 (3-value edge-offset 構文、offset がどちらか
/// 片方の軸にだけ authored される中間形) であり、CSS Values 4 §8.3 の
/// `<position>` 自体にこの中間形は存在しない。plain `<position>` 側の
/// 対応する alternative (`<position-four>`) は `[[left|right]
/// <length-percentage>] && [[top|bottom] <length-percentage>]` — offset は
/// `?` ではなく必須で、両軸とも authored されているか (4-value)、
/// どちらも `<length-percentage>` を伴わない bare keyword pair
/// (`<position-two>` の `&&` 形) かのどちらかしか許さない。`<position>`
/// 型を要求する property (`object-position` 等) はこの制約を課す
/// [`parse_position_strict`] を使う ([`parse_position_branch3_strict`]
/// doc参照) — `background-position` 自身は 3-value 形式を許す
/// `<bg-position>` のままで変わらない。
///
/// 3 alternative のうち最後 (`&&`、2 group が任意順で出現可能) が `top left`
/// のような keyword 並び替えと、`bottom 10px right 20px` (4-value、
/// `<position>` 自体にも存在) / `right 10px top` (3-value、`<bg-position>`
/// 固有の拡張) の edge-offset 構文をカバーする。
///
/// # なぜ 2 variant (`Start`/`End`) か — `<length-percentage>` 単体では表現不能
///
/// `right 10px top` (3-value edge-offset — bare `right 10px` alone is a
/// *different*, ambiguous 2-value form, see the well-known gotcha pinned
/// by `background_position_parse_right_10px_is_not_an_edge_offset`)の
/// 水平成分 (右 edge から 10px) は「左 edge から `100% - 10px`」と等価だが、
/// この crate は `calc()` を実装していない ([`DEFERRED_FUNCTIONS`] 参照) ため、
/// 単一の `<length-percentage>` (px と % の線形結合) としては表現できない。
/// そのため offset がどちらの edge から測られているかを型で保持し、edge から
/// 実 pixel 位置への最終変換は (`<length-percentage>` の percentage 解決自体が
/// 元々必要とする) background positioning area のサイズを持つ downstream layout
/// に委ねる — この crate の他の `<length-percentage>` (`padding` / `width` 等)
/// が percentage を解決せず素通しするのと同じ「絶対化は used value 層」の設計
/// 方針を、edge 情報にも一貫して適用したもの。
///
/// `Percent` payload の `End` は構築直後に等価な `Start` (`100.0 - p` を
/// percentage とする) へ正規化される ([`normalize_css_position`] 参照) —
/// `right`/`bottom` を percentage で表現できる場合は常に `Start` 基準に畳み、
/// `End` が実際に現れるのは非 percentage な offset (`right 10px top` の
/// 水平成分等、`calc()` 相当が必要なケース) に限られる。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CssPositionOffset {
    /// Start edge (horizontal: `left`、vertical: `top`) からの offset。
    Start(Length),
    /// End edge (horizontal: `right`、vertical: `bottom`) からの offset —
    /// 上記 doc の「なぜ 2 variant か」節参照。
    End(Length),
}

/// `<position>` value type (CSS Backgrounds and Borders 3 §2.6、[`CssPositionOffset`]
/// doc 参照)。`background-position` / `object-position` (本crate) で使われる。
/// 将来の `transform-origin` 等の再利用も見込んで汎用的に定義する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CssPosition {
    /// 水平軸の offset。
    pub horizontal: CssPositionOffset,
    /// 垂直軸の offset。
    pub vertical: CssPositionOffset,
}

/// `background-image` の specified value。
///
/// CSS Backgrounds and Borders 3 §2.3 "Image Sources: the background-image
/// property" <https://www.w3.org/TR/css-backgrounds-3/#the-background-image>。
/// Grammar: `<bg-image>#`、`<bg-image> = <image> | none` — この property は
/// 複数 background layer 用の comma-separated list (`#` multiplier) を許すが、
/// この実装は単一 layer のみを受理する ([`BackgroundRepeat`] doc と同じ
/// 「複数 layer compositing は follow-up」scope carving。将来 comma-list へ
/// 拡張する際は本 enum を `Arc<Vec<BackgroundImage>>` へ wrap するだけでよい)。
///
/// `<image> = <url> | <gradient>` (CSS Images 4
/// <https://www.w3.org/TR/css-images-4/#typedef-image>) の両 alternative を
/// 実装する — `<url>` は [`parse_url_value`] を再利用
/// ([`ContentComponent::Image`] と同じ 2 形式、unquoted `url(...)` / quoted
/// `url("...")`)、`<gradient>` (`linear-gradient()` /
/// `repeating-linear-gradient()` / `radial-gradient()` /
/// `repeating-radial-gradient()` / `conic-gradient()` /
/// `repeating-conic-gradient()`、CSS Images 4 §3.1-§3.4) は [`Gradient`] に
/// payload を持つ。
///
/// `<gradient>` の scope carving (Level 4 grammar から意図的に落とした部分。
/// [`GradientColorStop`] doc も参照):
///
/// - Color stop position は `<color> <length-percentage>?` (CSS Images 3
///   §3.4.1 の baseline grammar) のみ — Level 4 が追加した
///   `<color-stop-length> = <length-percentage>{1,2}` (1 stop に 2 position
///   を与え、同色の帯を作る記法) は未対応。
///   `<angular-color-stop>`/`<color-stop-angle>` (conic 版) も同様。
/// - `<linear-color-hint>` (2 stop 間の transition hint) は未対応 —
///   `<color-stop-list>` は hint 要素を挟まない `<linear-color-stop>#`
///   として parse する。**これは Level 4 の追加機能ではなく CSS Images 3
///   §3.4.1 の baseline grammar (`<color-stop-list> = <linear-color-stop> ,
///   [ <linear-color-hint>? , <linear-color-stop> ]#`) に既に含まれる** —
///   Level 4 §3.5.1 は同じ production をそのまま引き継ぐ。つまり本 crate は
///   「Level 3 baseline を完全実装し Level 4 の拡張のみ defer」ではなく、
///   baseline 自体の一部 (hint) も defer している。
/// - Color stop list は 2 個以上必須 (CSS Images 3 §3.4.1 の
///   `<linear-color-stop> , [ … ]#` baseline grammar — 3 個以上ではなく
///   「1 個目 + `#` group (1 個以上)」なので実質 2 個以上)。Level 4 が
///   `]#?` へ緩和した single-stop gradient (`gradient-single-stop-*.html`
///   系 WPT) は未対応。
/// - `<color-interpolation-method>` の `<color-space>` は
///   [`MixColorSpace`] が持つ 6 種 (`srgb`/`srgb-linear`/`lab`/`lch`/
///   `oklab`/`oklch`) のみ — `hsl`/`hwb`/`xyz`系/`display-p3`系は
///   対応する `<color>` function parser 自体が本 crate に無いため
///   ([`parse_color`] doc)、gradient 側でも受理しない。
/// - `radial-gradient()`/`repeating-radial-gradient()`の`<radial-size>`は
///   CSS Images 3 §3.2.1 の baseline grammar (`<radial-extent> |
///   <length [0,∞]> | <length-percentage [0,∞]>{2}`) のみ — CSS Images 4
///   §3.2.2 が追加した `<radial-extent>{1,2}` の 2-keyword 形は未対応
///   ([`RadialSize`] doc参照)。
///
/// これらは全て、gradient を実際に fill する raikiri-paint 側の描画実装が
/// まだ存在しないため使用実績が無く、grammar を広げるほど検証コストだけが
/// 先行する箇所 — 描画実装が着手される時点で個別に再評価する。
///
/// 複数 background layer 用の comma-separated list (`#` multiplier、
/// [`BackgroundRepeat`] doc と同じ「複数 layer compositing は follow-up」
/// scope carving) は本 enum 自体も単一 layer のみ受理する。将来 comma-list
/// へ拡張する際は本 enum を `Arc<Vec<BackgroundImage>>` へ wrap するだけで
/// よい。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum BackgroundImage {
    /// `none` — spec initial value。背景に image を描画しない。
    None,
    /// `<url>` — 単一 image layer の URL。raw `String` として保持
    /// ([`ContentComponent::Image`] 等の sibling `url` field と同じ
    /// convention、`url` crate 非依存)。
    Url(String),
    /// `<gradient>` — 6 gradient function のいずれか。paint 側での実際の
    /// fill は未実装 (上記 doc 参照) — この variant は parse 結果を
    /// [`ComputedValues`](crate::computed::ComputedValues) に保持するが、
    /// `<length-percentage>` の font-relative 側は computed 層で `Px` へ
    /// 絶対化され、`<percentage>` のみが paint 層へ defer される
    /// (`resolve_background_image` doc参照)。
    Gradient(Gradient),
}

/// `<angle>` (CSS Values 4 §7.1 "Angle Units: the &lt;angle&gt; type and
/// deg, grad, rad, turn units"
/// <https://www.w3.org/TR/css-values-4/#angles>)。
///
/// 4 単位 (`deg`/`grad`/`rad`/`turn`) は全て純粋な unit 変換であり、
/// `em`/`%` と異なり content-relative context を持たない (font-size や
/// percentage base に依存しない) ため、[`Length`] のように単位ごとに
/// variant を分けて specified 層に残す理由が無い — spec 自身が
/// "All `<angle>` units are compatible, and `deg` is their canonical unit"
/// と定める通り、本 crate も parse 時点で `deg` 単位の 1 値へ畳む。
///
/// `degrees` は authored 値をそのまま保持し、360 で正規化 (`rem_euclid`)
/// しない — `810deg` は `810.0` のまま。回転として意味が変わらない
/// 正規化を specified 層で行う理由が無く、gradient の direction/angle
/// 解釈は本 crate の scope 外 (paint 側の責務) なので、正規化するかどうかの
/// 判断も含めて downstream に委ねる。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Angle(pub f32);

/// `<angle-percentage>` (CSS Values 4 §5.6 "Mixing Percentages and
/// Dimensions" <https://www.w3.org/TR/css-values-4/#mixed-percentages>) —
/// `conic-gradient()` の angular color stop position
/// ([`AngularColorStop`]) が使う `<color-stop-angle>` の payload。
/// `<length-percentage>` ([`Length`]) の angle 版で、`Percent` の意味論は
/// 同じ (base に対する比率、authored number をそのまま保持)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnglePercentage {
    /// `<angle>` alternative。
    Angle(Angle),
    /// `<percentage>` alternative — authored number (`50%` → `50.0`、
    /// [`Length::Percent`] と同じ convention)。
    Percent(f32),
}

/// `<gradient>` (CSS Images Module Level 4 §3 "Gradients"
/// <https://www.w3.org/TR/css-images-4/#gradients>) — 6 gradient function
/// のいずれか。[`BackgroundImage::Gradient`] の payload。
///
/// `repeating-*` variant は非 repeating 版と同じ grammar を持つため
/// (spec verbatim、CSS Images 4 §3.4 "These notations take the same values
/// and are interpreted the same as their respective non-repeating
/// siblings")、6 variant に分けず各 struct に `repeating: bool` field を
/// 持たせる形にした ([`LinearGradient::repeating`] 等)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Gradient {
    /// `linear-gradient()` / `repeating-linear-gradient()` (CSS Images 4
    /// §3.1)。
    Linear(LinearGradient),
    /// `radial-gradient()` / `repeating-radial-gradient()` (CSS Images 4
    /// §3.2)。
    Radial(RadialGradient),
    /// `conic-gradient()` / `repeating-conic-gradient()` (CSS Images 4
    /// §3.3)。
    Conic(ConicGradient),
}

/// `linear-gradient()` / `repeating-linear-gradient()` (CSS Images 4 §3.1
/// "Linear Gradients: the linear-gradient() notation"
/// <https://www.w3.org/TR/css-images-4/#linear-gradients>)。
///
/// Grammar: `<linear-gradient-syntax> = [ [ <angle> | <zero> | to
/// <side-or-corner> ] || <color-interpolation-method> ]? , <color-stop-list>`。
/// `direction`/`interpolation` は共に省略時 default を stored directly する
/// (省略時は "defaults to to bottom" / [`GradientColorInterpolation`] の
/// spec-mandated default) — [`CssPosition`] の "center" default 等、他の
/// keyword default と同じ「省略パターンを型に残さず即座に解決する」方針。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    /// `repeating-linear-gradient()` かどうか ([`Gradient`] doc参照)。
    pub repeating: bool,
    /// 勾配線の方向。省略時 default は `to bottom`
    /// ([`LinearGradientDirection::Side`] with `vertical: Some(Bottom)`)。
    pub direction: LinearGradientDirection,
    /// `in <color-space> <hue-interpolation-method>?` 節。省略時 default は
    /// `Oklab` (CSS Images 4 §3.5.2 "Coloring the Gradient Line" — this
    /// subsection defines interpolation for all 3 gradient shapes, not just
    /// linear — "If no `<color-interpolation-method>` is specified in the
    /// gradient function, the color space used for gradient interpolation
    /// is the default interpolation color space, Oklab")。
    pub interpolation: GradientColorInterpolation,
    /// Color stop list。2 個以上 ([`BackgroundImage`] doc の scope carving
    /// 節参照)。`Arc` は他の comma-separated list payload
    /// (`Content(Arc<Vec<..>>)` 等) と同じ cheap-clone pattern。
    pub stops: Arc<Vec<GradientColorStop>>,
}

/// [`LinearGradient::direction`] の 2 alternative
/// (`<angle> | <zero> | to <side-or-corner>`)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinearGradientDirection {
    /// `<angle>` (`<zero>` を含む) alternative。
    Angle(Angle),
    /// `to <side-or-corner>` alternative。
    Side(SideOrCorner),
}

/// `<side-or-corner> = [left | right] || [top | bottom]` (CSS Images 4
/// §3.1)。少なくとも一方は `Some` — 両方 `None` は parser が reject する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideOrCorner {
    /// 水平成分 (`left`/`right`)。省略可。
    pub horizontal: Option<HorizontalSide>,
    /// 垂直成分 (`top`/`bottom`)。省略可。
    pub vertical: Option<VerticalSide>,
}

/// [`SideOrCorner::horizontal`] の keyword。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizontalSide {
    /// `left`。
    Left,
    /// `right`。
    Right,
}

/// [`SideOrCorner::vertical`] の keyword。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalSide {
    /// `top`。
    Top,
    /// `bottom`。
    Bottom,
}

/// `radial-gradient()` / `repeating-radial-gradient()` (CSS Images 4 §3.2
/// "Radial Gradients: the radial-gradient() notation"
/// <https://www.w3.org/TR/css-images-4/#radial-gradients>)。
///
/// Grammar: `<radial-gradient-syntax> = [ [ [ <radial-shape> ||
/// <radial-size> ]? [ at <position> ]? ] || <color-interpolation-method> ]?
/// , <color-stop-list>`。`shape`/`size` は共に省略時 default を stored directly
/// する (CSS Images 3 §3.2.1 の shape-inference 規則 — [`LinearGradient`]
/// doc と同じ方針)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    /// `repeating-radial-gradient()` かどうか。
    pub repeating: bool,
    /// Ending shape。省略時、`size` が `Circle(_)` なら `Circle`、それ以外
    /// (省略含む) は `Ellipse` (CSS Images 3 §3.2.1 "the ending shape
    /// defaults to a circle if the `<radial-size>` is a single `<length>`,
    /// and to an ellipse otherwise")。
    pub shape: RadialShape,
    /// Ending shape のサイズ。省略時 default は `Extent(FarthestCorner)`。
    pub size: RadialSize,
    /// Gradient の中心。省略時 default は `center`。
    pub position: CssPosition,
    /// `in <color-space> <hue-interpolation-method>?` 節。省略時 default は
    /// [`LinearGradient::interpolation`] と同じ `Oklab`。
    pub interpolation: GradientColorInterpolation,
    /// Color stop list ([`LinearGradient::stops`] と同じ shape)。
    pub stops: Arc<Vec<GradientColorStop>>,
}

/// [`RadialGradient::shape`] — `<radial-shape> = circle | ellipse`。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialShape {
    /// `circle`。
    Circle,
    /// `ellipse`。
    Ellipse,
}

/// [`RadialGradient::size`] (CSS Images 3 §3.2.1 baseline grammar
/// `<radial-size> = <radial-extent> | <length [0,∞]> |
/// <length-percentage [0,∞]>{2}`)。
///
/// CSS Images 4 §3.2.2 が追加した 2-keyword `<radial-extent>{1,2}` 形
/// (circle()/ellipse() `<basic-shape>` 由来の拡張) は未対応 —
/// [`BackgroundImage`] doc の scope carving 節参照。`Circle`/`Ellipse`
/// variant の非負制約 (`[0,∞]`) は parser 側で enforce する (型には
/// 反映しない、[`Length`] の他の non-negative context — `border-width` 等
/// — と同じ convention)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialSize {
    /// `<radial-extent>` keyword — [`RadialShape::Circle`]/[`RadialShape::Ellipse`]
    /// どちらとも組み合わせ可。
    Extent(RadialExtent),
    /// 明示的な `<length [0,∞]>` — [`RadialShape::Circle`] とのみ組み合わせ可
    /// (parser が enforce)。
    Circle(Length),
    /// 明示的な `<length-percentage [0,∞]>{2}` (水平・垂直半径) —
    /// [`RadialShape::Ellipse`] とのみ組み合わせ可 (parser が enforce)。
    Ellipse(Length, Length),
}

/// [`RadialSize::Extent`] の keyword。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialExtent {
    /// `closest-side`。
    ClosestSide,
    /// `closest-corner`。
    ClosestCorner,
    /// `farthest-side`。
    FarthestSide,
    /// `farthest-corner` — [`RadialSize`] の spec-mandated default。
    FarthestCorner,
}

/// `conic-gradient()` / `repeating-conic-gradient()` (CSS Images 4 §3.3
/// "Conic Gradients: the conic-gradient() notation"
/// <https://www.w3.org/TR/css-images-4/#conic-gradients>)。
///
/// Grammar: `<conic-gradient-syntax> = [ [ [ from [ <angle> | <zero> ] ]?
/// [ at <position> ]? ] || <color-interpolation-method> ]? ,
/// <angular-color-stop-list>`。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradient {
    /// `repeating-conic-gradient()` かどうか。
    pub repeating: bool,
    /// `from <angle>`。省略時 default は `0deg`。
    pub angle: Angle,
    /// `at <position>`。省略時 default は `center`。
    pub position: CssPosition,
    /// `in <color-space> <hue-interpolation-method>?` 節。省略時 default は
    /// [`LinearGradient::interpolation`] と同じ `Oklab`。
    pub interpolation: GradientColorInterpolation,
    /// Angular color stop list ([`LinearGradient::stops`] と同じ shape、
    /// position の型のみ [`AngularColorStop`] に差し替え)。
    pub stops: Arc<Vec<AngularColorStop>>,
}

/// `in <color-space> <hue-interpolation-method>?` (CSS Color 4 §13.2、
/// [`MixColorSpace`] doc参照) — gradient 関数群共通の color-interpolation
/// 節。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientColorInterpolation {
    /// Interpolation を行う color space。
    pub color_space: MixColorSpace,
    /// Hue の補間方向 — `color_space` が polar (`Lch`/`Oklch`) でない場合は
    /// 意味を持たない (常に `Shorter` を格納、[`parse_gradient_color_interpolation`]
    /// 参照)。
    pub hue_method: HueInterpolationMethod,
}

/// `<color>` を持つ gradient stop の color payload — `currentcolor`
/// keyword と resolved `<color>` の区別 ([`TextShadowColor`] と同型、
/// gradient stop 専用の別 type)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientStopColor {
    /// `currentcolor` keyword。used-value 解決は paint scope の責務。
    CurrentColor,
    /// Resolved `<color>` value。
    Resolved(CssColor),
}

/// `<linear-color-stop>` / `<radial-gradient-syntax>` の color-stop-list
/// entry (CSS Images 3 §3.4.1 "Color Stop Lists" — Level 4 §3.5.1 は同じ
/// production を引き継ぐ)。
///
/// この crate は CSS Images 3 の baseline grammar `<linear-color-stop> =
/// <color> <length-percentage>?` のみを実装する — Level 4 が追加した
/// `<color-stop-length> = <length-percentage>{1,2}` (1 stop に 2 position、
/// 同色の帯を作る記法) は未対応。加えて、`<linear-color-hint>` (stop 間の
/// transition hint) **も**未対応 — こちらは Level 4 の拡張ではなく Level 3
/// §3.4.1 の baseline grammar (`<color-stop-list> = <linear-color-stop> , [
/// <linear-color-hint>? , <linear-color-stop> ]#`) に既に含まれる production
/// で、本 crate は baseline のこの部分も defer している
/// ([`BackgroundImage`] doc の scope carving 節参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientColorStop {
    /// Stop の color。
    pub color: GradientStopColor,
    /// Stop の position (`<length-percentage>`)。省略時は fixup で自動決定
    /// される (CSS Images 3 §3.4.3 "Color Stop \"Fixup\"") — その決定は
    /// gradient line の長さを要するため used-value 層 (paint) の責務、
    /// この crate は `None` のまま保持する。
    pub position: Option<Length>,
}

/// `<angular-color-stop>` — [`GradientColorStop`] の conic-gradient 版
/// (position の型のみ `<angle-percentage>` に差し替え、CSS Images 4
/// §3.5.1)。scope carving は [`GradientColorStop`] と同じ
/// (`<color-stop-angle>{1,2}`/`<angular-color-hint>` 未対応)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AngularColorStop {
    /// Stop の color。
    pub color: GradientStopColor,
    /// Stop の position (`<angle-percentage>`)。省略時の扱いは
    /// [`GradientColorStop::position`] と同じ。
    pub position: Option<AnglePercentage>,
}

/// `<repeat-style>` の 1 軸分の keyword (CSS Backgrounds and Borders 3 §2.4
/// <https://www.w3.org/TR/css-backgrounds-3/#typedef-repeat-style>)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundRepeatKeyword {
    /// `repeat` — spec initial value (両軸)。tile を繰り返し、必要なら最後の
    /// tile を clip する。
    Repeat,
    /// `space` — tile を繰り返しつつ、割り切れない余白を tile 間の均等な
    /// 間隔として分配する。
    Space,
    /// `round` — tile を繰り返しつつ、割り切れるよう tile を伸縮する。
    Round,
    /// `no-repeat` — tile を 1 個だけ配置する。
    NoRepeat,
}

/// `background-repeat` の specified value。
///
/// CSS Backgrounds and Borders 3 §2.4 "Tiling Images: the
/// background-repeat property"。Grammar: `<repeat-style>#` — この
/// property は複数 background layer 用の comma-separated list
/// (`#` multiplier) を許すが、この実装は `background-image` 自体が
/// (別 task の scope として) 未実装で複数 layer を observe する経路が無いため、
/// 単一 layer のみを受理する — 将来 `background-image` が comma-list を
/// 持つようになった時点で、本 struct を `Arc<Vec<BackgroundRepeat>>` へ
/// wrap するだけで拡張できる (`BoxShadowItem` の `Arc<Vec<..>>` 化と
/// 同じ shape)。
///
/// `repeat-x` = `{x: Repeat, y: NoRepeat}`、`repeat-y` = `{x: NoRepeat, y:
/// Repeat}` (2 keyword shorthand として spec が定義する computed value —
/// [`parse_background_repeat`] doc 参照)。1 keyword 指定時は両軸に適用する
/// (`repeat` = `repeat repeat` 等)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackgroundRepeat {
    /// 水平軸の repeat 方式。
    pub x: BackgroundRepeatKeyword,
    /// 垂直軸の repeat 方式。
    pub y: BackgroundRepeatKeyword,
}

/// `background-attachment` の specified value。
///
/// CSS Backgrounds and Borders 3 §2.5 "Affixing Images: the
/// background-attachment property"。Grammar: `<attachment>#` — comma-list
/// の scope 外理由は [`BackgroundRepeat`] doc と同じ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundAttachment {
    /// `scroll` — spec initial value。background は element を含む block
    /// (containing block chain) に対して固定され、element 自身の内容と
    /// ともにスクロールしない一方、page 全体のスクロールには追従する。
    Scroll,
    /// `fixed` — background は viewport に対して固定される。
    Fixed,
    /// `local` — background は element 自身の内容とともにスクロールする。
    Local,
}

/// `background-clip` / `background-origin` が共有する box keyword
/// (CSS Backgrounds and Borders 3 §2.7 "Painting Area: the
/// background-clip property" / "Positioning Area: the
/// background-origin property")。両 property とも grammar は
/// `<visual-box>#` — comma-list の scope 外理由は [`BackgroundRepeat`] doc
/// と同じ。
///
/// spec initial は property ごとに異なる — `background-clip` は
/// `border-box`、`background-origin` は `padding-box`
/// ([`crate::specified::SpecifiedValues::initial`] / [`crate::computed::ComputedValues::initial`]
/// 参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualBox {
    /// `border-box` — border の外側の縁。
    BorderBox,
    /// `padding-box` — border の内側、padding の外側の縁。
    PaddingBox,
    /// `content-box` — padding の内側、content box の縁。
    ContentBox,
    /// `border-area` — border area の縁 (CSS Backgrounds 4 §2.6)。
    BorderArea,
    /// `text` — text の形にクリップ (CSS Backgrounds 4 §2.6, `background-clip: text`)。
    Text,
}

/// `background-size` の specified value。
///
/// CSS Backgrounds and Borders 3 §2.9 "Sizing Images: the
/// background-size property"。Grammar: `<bg-size>#` — comma-list の scope
/// 外理由は [`BackgroundRepeat`] doc と同じ。
///
/// `<bg-size> = [ <length-percentage [0,∞]> | auto ]{1,2} | cover |
/// contain`。1 value のみ指定時、2 個目の axis は **`auto`** になる (spec
/// verbatim: "If only one value is given the second is assumed to be
/// auto.") — 同じく 1-2 value を取る [`BorderRadius`] (省略値は 1 個目を
/// 複製) とは fill 規則が異なる点に注意 ([`parse_background_size`] doc
/// 参照)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackgroundSize {
    /// `[ <length-percentage [0,∞]> | auto ]{1,2}` — 各軸独立に長さまたは
    /// `auto` を取る。
    Explicit {
        /// 水平軸のサイズ。
        width: LengthOrAuto,
        /// 垂直軸のサイズ。
        height: LengthOrAuto,
    },
    /// `cover` — background positioning area 全体を覆うよう、アスペクト比を
    /// 保ったまま拡大縮小する。
    Cover,
    /// `contain` — background positioning area に収まる最大サイズまで、
    /// アスペクト比を保ったまま拡大縮小する。
    Contain,
}

/// `background` shorthand の parse 結果を一時的に保持する carrier。
///
/// CSS Backgrounds and Borders 3 §2.10 "Backgrounds Shorthand: the
/// background property"
/// <https://www.w3.org/TR/css-backgrounds-3/#the-background>。spec 本文
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
/// invalid at the [`crate::rule::parse_declaration_block`] call site's
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
    /// `background-color` 成分 — 省略時は [`CssColor::TRANSPARENT`] (spec
    /// initial)。
    pub color: CssColor,
    /// `background-image` 成分 — 省略時は [`BackgroundImage::None`] (spec
    /// initial)。
    pub image: BackgroundImage,
    /// `background-repeat` 成分 — 省略時は両軸 [`BackgroundRepeatKeyword::Repeat`]
    /// (spec initial)。
    pub repeat: BackgroundRepeat,
    /// `background-attachment` 成分 — 省略時は [`BackgroundAttachment::Scroll`]
    /// (spec initial)。
    pub attachment: BackgroundAttachment,
    /// `background-position` 成分 — 省略時は `0% 0%` (spec initial)。
    pub position: CssPosition,
    /// `background-size` 成分 — 省略時は両軸 `auto` (spec initial)。size は
    /// position の直後、`/` 区切りでのみ出現しうる ([`Self`] doc の
    /// position+size 節参照)。
    pub size: BackgroundSize,
    /// `background-clip` 成分 — 省略時は [`VisualBox::BorderBox`] (spec
    /// initial)。`<visual-box>` の出現回数と origin/clip への割り当て規則は
    /// [`Self`] doc 参照。
    pub clip: VisualBox,
    /// `background-origin` 成分 — 省略時は [`VisualBox::PaddingBox`] (spec
    /// initial)。
    pub origin: VisualBox,
}

/// `object-fit` の specified value。
///
/// CSS Images Module Level 3 §5.1 "Sizing the replaced element: the
/// object-fit property"
/// <https://www.w3.org/TR/css-images-3/#the-object-fit>。Grammar: `fill |
/// contain | cover | none | scale-down`。Applies to: replaced elements
/// only。**non-inherited**。Computed value = specified keyword — no length
/// payload (`BackgroundAttachment` と同じ shape)。
///
/// `object-position` (CSS Images 3 §5.2、[`CssPosition`] 再利用) と対になる
/// property だが、この 5 keyword の意味自体は replaced element の concrete
/// object size をどう決めるかという layout-time algorithm (同 spec §5.3
/// "Sizing the replaced element" の default-object-size / concrete-object-size
/// 手順) であり、本 crate はそのレイアウト適用アルゴリズム自体を実装しない —
/// 本 variant が保持するのは cascade/computed value の keyword のみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFit {
    /// `fill` — spec initial value。replaced content を content box に
    /// 合わせて (aspect ratio を保持せず) 引き伸ばす。
    Fill,
    /// `contain` — aspect ratio を保持したまま、content box に収まる最大
    /// サイズへ縮小/拡大する。
    Contain,
    /// `cover` — aspect ratio を保持したまま、content box を覆う最小
    /// サイズへ縮小/拡大する (どちらかの軸で box をはみ出しうる)。
    Cover,
    /// `none` — content を resize しない。concrete object size は
    /// intrinsic size (無ければ spec の default object size algorithm の
    /// 結果) をそのまま使う。
    None,
    /// `scale-down` — `none` と `contain` それぞれの concrete object size
    /// のうち小さい方。
    ScaleDown,
}

/// `isolation` の specified value。
///
/// CSS Compositing and Blending Level 1 §3.4.2 "Isolation: the isolation
/// property" <https://www.w3.org/TR/compositing-1/#isolation>。Grammar:
/// `auto | isolate`。**non-inherited**。Computed value = specified keyword —
/// no length payload (`ObjectFit` と同じ shape)。
///
/// spec 本文は `isolation` が実際に stacking context / group を作るかどうかの
/// 適用条件 (要素の種類、SVG container 等) を細かく規定するが、本 crate は
/// その適用アルゴリズムを実装しない — 本 variant が保持するのは
/// cascade/computed value の keyword のみ (実際に compositing group を
/// 構築する処理は raikiri-paint 側の責務、`ObjectFit` doc の「レイアウト
/// 適用アルゴリズム自体は実装しない」節と同じ scope carving)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    /// `auto` — spec initial value。要素自身は独立した stacking context /
    /// group を強制しない。
    Auto,
    /// `isolate` — 要素を独立した stacking context にし、`mix-blend-mode` の
    /// blending をその subtree 内に隔離する。
    Isolate,
}

/// `mix-blend-mode` の specified value。
///
/// CSS Compositing and Blending Level 1 §3.4.1 "Mix Blend Mode: the
/// mix-blend-mode property"
/// <https://www.w3.org/TR/compositing-1/#mix-blend-mode>。Grammar:
/// `<blend-mode> = normal | multiply | screen | overlay | darken | lighten |
/// color-dodge | color-burn | hard-light | soft-light | difference |
/// exclusion | hue | saturation | color | luminosity` (`<blend-mode>` 自体は
/// CSS Compositing and Blending Level 1 §2 "Compositing and Blending"
/// で定義され、本 crate が未実装の `background-blend-mode` property とも
/// 共有される grammar)。**non-inherited**。Computed value = specified
/// keyword — no length payload。
///
/// 実際の blending 演算 (各 mode の合成式、CSS Compositing and Blending
/// Level 1 §3.2 "Blending") は raikiri-paint 側の compositing 実装が別途
/// 必要 — 本 variant が保持するのは cascade/computed value の keyword のみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixBlendMode {
    /// `normal` — spec initial value。backdrop を素通しする通常合成。
    Normal,
    /// `multiply` — CSS Compositing and Blending Level 1 §3.2.1。
    Multiply,
    /// `screen` — 同 §3.2.2。
    Screen,
    /// `overlay` — 同 §3.2.3。
    Overlay,
    /// `darken` — 同 §3.2.4。
    Darken,
    /// `lighten` — 同 §3.2.5。
    Lighten,
    /// `color-dodge` — 同 §3.2.6。
    ColorDodge,
    /// `color-burn` — 同 §3.2.7。
    ColorBurn,
    /// `hard-light` — 同 §3.2.8。
    HardLight,
    /// `soft-light` — 同 §3.2.9。
    SoftLight,
    /// `difference` — 同 §3.2.10。
    Difference,
    /// `exclusion` — 同 §3.2.11。
    Exclusion,
    /// `hue` — non-separable blend mode、CSS Compositing and Blending
    /// Level 1 §3.2.12。
    Hue,
    /// `saturation` — 同 §3.2.13。
    Saturation,
    /// `color` — 同 §3.2.14。
    Color,
    /// `luminosity` — 同 §3.2.15。
    Luminosity,
}

/// `clip-path` の `<geometry-box>` component (CSS Masking Level 1 §5.1
/// "Basic Shapes: the clip-path property"
/// <https://www.w3.org/TR/css-masking-1/#the-clip-path>)。
///
/// Grammar: `<geometry-box> = <shape-box> | fill-box | stroke-box |
/// view-box`、`<shape-box> = <box> | margin-box`、`<box> = border-box |
/// padding-box | content-box`。`fill-box`/`stroke-box`/`view-box` は
/// `<shape-box>` には含まれず、`<geometry-box>` 自身の直接 alternative —
/// 両 production 間で重複する keyword は無い (union は border-box /
/// padding-box / content-box / margin-box / fill-box / stroke-box /
/// view-box の 7 keyword)。
///
/// SVG 文脈 (`fill-box`/`stroke-box`/`view-box`) の解決は raikiri-paint 側の
/// SVG レンダリング実装 (現状皆無、[`ClipPath`] doc 参照) の責務 — 本 variant
/// は keyword を保持するのみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryBox {
    /// `border-box`。
    BorderBox,
    /// `padding-box`。
    PaddingBox,
    /// `content-box`。
    ContentBox,
    /// `margin-box`。
    MarginBox,
    /// `fill-box` — SVG の bounding box。
    FillBox,
    /// `stroke-box` — SVG の stroke bounding box。
    StrokeBox,
    /// `view-box` — 最も近い SVG viewport。
    ViewBox,
}

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

/// `clip-path` の specified value.
///
/// CSS Masking Level 1 §5.1 "Basic Shapes: the clip-path property"
/// <https://www.w3.org/TR/css-masking-1/#the-clip-path>。Full grammar:
/// `<clip-source> | [ <basic-shape> || <geometry-box> ] | none`、
/// `<clip-source> = <url>`。**non-inherited**。
///
/// `<basic-shape>` の grammar は CSS Shapes Module Level 1 §3
/// <https://www.w3.org/TR/css-shapes-1/#basic-shape-functions> が定義する
/// (CSS Masking Level 1 自身ではなく同 spec が primary source)。
/// 本 crate は `circle()` / `ellipse()` / `inset()` / `polygon()` /
/// `path()` の 5 function を実装する — `rect()` / `xywh()` / `shape()`
/// は本 task の scope 外。
///
/// # Computed value
///
/// 同 § "Computed value: as specified, but with `<url>` values made
/// absolute" — `<url>` の絶対化 (base URL 解決) は本 crate が URL 解決の
/// 実行環境 (base URL、fetch) を持たないため未対応、[`BackgroundImage::Url`]
/// と同じ scope carving。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ClipPath {
    /// `none` — spec initial value。clipping を行わない。
    None,
    /// `<clip-source>` = `<url>` — SVG `<clipPath>` element 等への参照。
    Url(String),
    /// `<geometry-box>` 単体 (`<basic-shape>` 併記なし)。
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

/// `mask-image` の specified value — [`BackgroundImage`] の type alias.
///
/// CSS Masking Level 1 §7.1 "Image Masking: the mask-image property"
/// <https://www.w3.org/TR/css-masking-1/#the-mask-image>。Full grammar:
/// `<mask-reference>#`、`<mask-reference> = none | <image> | <mask-source>`、
/// `<mask-source> = <url>`、`<image> = <url> | <gradient>`。
/// **non-inherited**。
///
/// # Scope carving — single layer のみ
///
/// `<mask-reference>#` の comma-separated multi-layer list は未対応 —
/// [`BackgroundImage`] doc の「複数 background layer 用の comma-separated
/// list は未対応、将来 `Arc<Vec<..>>` へ wrap するだけで拡張できる」scope
/// carving と同じ判断・同じ拡張余地。
///
/// # `<mask-source>` と `<image>` の `url` alternative は同じ具象構文
///
/// `<mask-source>` (`<url>`) と `<image>`'s `<url>` alternative は
/// concrete syntax 上区別不能 (`url(#foo)` はどちらのつもりで書かれたかを
/// パーサーが判別する情報を持たない) — したがって本 alias の concrete
/// syntax は [`BackgroundImage`] の `<bg-image> = <url> | <gradient>` と
/// 同一 ([`parse_mask_image`] doc 参照)。
///
/// # Reuse convention — [`BackgroundImage`] を verbatim 再利用
///
/// 本 alias は property 固有名の型を別 property でそのまま再利用する
/// convention (b) に従う — 既存例: [`FilterFunction::DropShadow`] が
/// [`TextShadowItem`] を verbatim 再利用。`MaskImage` と
/// [`BackgroundImage`] は variant shape (`None` / `Url(String)` /
/// `Gradient(Gradient)`) が完全一致するため、重複 enum ではなく type
/// alias とする。将来 [`BackgroundImage`] に multi-layer 対応等の拡張が
/// 入った場合も自動的に追従し drift を防ぐ。
pub type MaskImage = BackgroundImage;

/// `transform` の 1 function (CSS Transforms Level 1 §9.1 "Two-dimensional
/// Subset" <https://www.w3.org/TR/css-transforms-1/#two-d-transform-functions>)。
///
/// V2 (3D transform: `translate3d()`/`rotate3d()`/`matrix3d()`/
/// `perspective()` 等) は非対応 — 別 spec section (§10 "3D Transform
/// Functions") であり、本 crate の対応 scope は明示的に §9.1 の 2D
/// function のみ。3D function 名は [`parse_transform_function`] の
/// unrecognized-name path で silent drop される (`<basic-shape>` の
/// scope carving — [`ClipPath`] doc参照 — と同型の「別 section 丸ごと
/// defer」判断)。
///
/// 各 numeric payload の NaN 扱いは [`parse_transform_number`]/
/// [`parse_transform_length_percentage`]/[`parse_angle_reject_nan`] の doc
/// を参照 — cssparser の exponent overflow (`0 * Infinity` collapse) 由来の
/// NaN を reject し、magnitude overflow 由来の `+Inf`/`-Inf` は (spec が
/// range を制限しない引数である限り) 保持する、[`PropertyValue::Opacity`]
/// の `!is_nan()` guard と同じ判断。
///
/// # Absolutization — length half is absolutized, percent stays symbolic
///
/// `transform` property 自体の Computed value は "as specified, **but with
/// lengths made absolute**"
/// (<https://www.w3.org/TR/css-transforms-1/#transform-property>) —
/// [`FilterFunction`] doc の "Range restriction" 節が述べる `filter` の
/// 単純な "as specified" (絶対化不要) とは異なり、`transform` は
/// `<length>` payload (この enum では [`Self::Translate`]/
/// [`Self::TranslateX`]/[`Self::TranslateY`] が運ぶ `Length` の
/// non-percentage 側) を spec 上絶対化する義務を負う.
///
/// 本 crate は element 経路では
/// [`crate::resolve::ComputedTransformFunction`]/
/// [`crate::resolve::resolve_transform_function`] が、`page` 経路では
/// [`crate::page::cascade_page`] の phase 3 (`absolutize_in_page_context`
/// の `Transform` arm) が、それぞれこの分割を実装する:
/// `<length-percentage>` の length 側だけを font-size/root-font-size に対して
/// 絶対化し percentage 側は symbolic なまま残す — 既に
/// [`crate::resolve::resolve_css_position`]/
/// [`crate::resolve::resolve_length_percentage`] が
/// `background-position`/`object-position` に対して行っているのと同型
/// (`Matrix` の 6 `<number>` slot と `Rotate`/`Skew`/`SkewX`/
/// `SkewY` の `<angle>` slot にはこの変換は不要 — 前者は既に fully
/// resolved な `<number>`、後者は spec 上正規化されない `<angle>` で
/// あり、どちらも percentage/box-size の話に関わらない)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformFunction {
    /// `matrix(<number>{6})` — a, b, c, d, e, f の 6 係数、homogeneous
    /// 2D affine matrix `[[a, c, e], [b, d, f], [0, 0, 1]]`。
    Matrix([f32; 6]),
    /// `translate(<length-percentage>, <length-percentage>?)` — 2 番目省略時
    /// `0` ([`parse_translate_args`] doc参照)。
    Translate(Length, Length),
    /// `translateX(<length-percentage>)`。
    TranslateX(Length),
    /// `translateY(<length-percentage>)`。
    TranslateY(Length),
    /// `scale(<number>, <number>?)` — 2 番目省略時は 1 番目を複製
    /// ([`parse_scale_args`] doc参照)。
    Scale(f32, f32),
    /// `scaleX(<number>)`。
    ScaleX(f32),
    /// `scaleY(<number>)`。
    ScaleY(f32),
    /// `rotate([<angle> | <zero>])`。
    Rotate(Angle),
    /// `skew([<angle> | <zero>], [<angle> | <zero>]?)` — 2 番目省略時
    /// `0deg` ([`parse_skew_args`] doc参照)。
    Skew(Angle, Angle),
    /// `skewX([<angle> | <zero>])`。
    SkewX(Angle),
    /// `skewY([<angle> | <zero>])`。
    SkewY(Angle),
}

/// `filter` の 1 function/reference (CSS Filter Effects Level 1 §6
/// "Filter Functions" <https://www.w3.org/TR/filter-effects-1/#filter-functions>
/// + §5 の `<url>` alternative)。
///
/// # Range restriction は reject、clamp ではない
///
/// §6.1 の各 `<number-percentage>` 引数は "Negative values are not
/// allowed" と規定する — CSS Color 4 §3.3 が `opacity` property に対して
/// 明示した「specified では保持、computed で clamp」carve-out はここには
/// 無く (`filter` property 自体の Computed value は "as specified"、
/// [`parse_filter_amount`] doc参照)、CSS Values 4 §5 の既定通り range 外は
/// invalid — [`parse_nonneg_finite_number`] (flex-grow/flex-shrink) と
/// 同じ reject-at-parse 判断。
///
/// `grayscale()`/`invert()`/`opacity()`/`sepia()` の "values over 100%
/// allowed but UAs **must** clamp the values to 1" は user-agent の
/// **rendering 時**の義務であり、specified/computed value 自体を変形する
/// 規定ではない (`filter` property の Computed value が "as specified" で
/// ある以上、値そのものを変形する余地が無い) — よってこの clamp は
/// **paint 側**の責務として保持し、本 crate 側では値をそのまま運ぶ
/// (`brightness()`/`contrast()`/`saturate()` の "over 100% allowed" — 明示的に
/// clamp 不要 — と同じ payload 型を共有できる)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum FilterFunction {
    /// `blur(<length>?)` — 省略時 `0px`。standard deviation、non-negative
    /// ([`parse_non_negative_length`] を再利用)。
    Blur(Length),
    /// `brightness(<number-percentage>?)` — 省略時 `1`。over-100% は
    /// clamp 不要 (上記 doc参照)。
    Brightness(f32),
    /// `contrast(<number-percentage>?)` — 省略時 `1`。over-100% は
    /// clamp 不要。
    Contrast(f32),
    /// `grayscale(<number-percentage>?)` — 省略時 `1`。over-100% は
    /// **rendering 時に** UA が 1 へ clamp する義務があるが、値自体は
    /// そのまま運ぶ (上記 doc参照)。
    Grayscale(f32),
    /// `hue-rotate([<angle> | <zero>]?)` — 省略時 `0deg`。range 制限無し。
    HueRotate(Angle),
    /// `invert(<number-percentage>?)` — 省略時 `1`。`grayscale()` と同じ
    /// over-100% 扱い。
    Invert(f32),
    /// `opacity(<number-percentage>?)` — 省略時 `1`。`grayscale()` と同じ
    /// over-100% 扱い ([`PropertyValue::Opacity`] property とは無関係の
    /// 同名 filter function)。
    Opacity(f32),
    /// `saturate(<number-percentage>?)` — 省略時 `1`。over-100% は clamp
    /// 不要。
    Saturate(f32),
    /// `sepia(<number-percentage>?)` — 省略時 `1`。`grayscale()` と同じ
    /// over-100% 扱い。
    Sepia(f32),
    /// `drop-shadow(<color>? && <length>{2,3})` — "Values are interpreted
    /// as for box-shadow but with the optional 3rd `<length>` value being
    /// the standard deviation instead of blur radius." spread/inset/複数
    /// shadow は不可 — [`TextShadowItem`] と grammar が完全一致するため
    /// その型を再利用する ([`parse_drop_shadow_args`] doc参照)。
    DropShadow(TextShadowItem),
    /// `<url>` — SVG `<filter>` element 等への参照 (§5 の
    /// `[ <filter-function> | <url> ]+` grammar)。
    Url(String),
}

/// 現サポート property の resolved value (variant 一覧は下記、
/// property name → variant mapping は `parse_value` 参照)。
///
/// 認識できない property (例: `cursor` — 現行 scope 外) や
/// invalid value (例: `font-size: math` — MathML scaling algorithm 未実装) は
/// parser 段で `None` に落として rule から
/// silently 除外される。
///
/// unit 側の「現在何が未対応か」は本節では例示しない — 具体例を挙げると
/// その unit が受理側へ移った時点で本節だけが取り残される (実際に `cm`
/// の例がこの経路で 1 度 drift した)。
/// canonical は [`parse_length_value`] の `Token::Dimension` match arm
/// (`_` arm 直前 comment) と `parse_length_value_rejects_unsupported_unit`
/// test。
///
/// # CSS-wide keyword (canonical)
///
/// CSS-wide keyword (`inherit` / `initial` / `unset` / `revert` — CSS
/// Cascade 4 §7.3 "Explicit Defaulting"
/// <https://www.w3.org/TR/css-cascade-4/#defaulting-keywords>、`revert-layer`
/// — CSS Cascade 5 §7.3.5 "Rolling Back Cascade Layers: the revert-layer
/// keyword" <https://www.w3.org/TR/css-cascade-5/#revert-layer>) の support は
/// 本 crate ではまだ実装されていない。
///
/// unit 側と違い、この不対応には単一の code arm が無い — 各 `parse_*` 関数は
/// これらの ident を単に認識せず、他の spec-invalid keyword と同じ「未知
/// keyword」rejection 経路 (各関数自身の `_ => None` 等) へ落ちる、という
/// **実装しないことによる不作為の一致**。したがって本節でも個々の property
/// doc でも 5 keyword の enumeration を反復しない — 反復は property が増える
/// たびに drift する (実際に property.rs 内 16 箇所で
/// 独立に再記述され、うち border-width / border-style / box-sizing の 3 箇所は
/// pinning test を伴わずに存在していた)。canonical はこの 1 段落と、
/// `rejects_css_wide_keyword` 命名の代表 pinning test 群
/// (`text_align_rejects_css_wide_keyword` 等、crate 内で grep すれば全件
/// 見つかる)。
///
/// `<custom-ident>` ベースの grammar (`counter-name` / `string-set` の name /
/// `position: running()` の引数) は不作為ではなく **明示的な** reject list
/// ([`is_reserved_counter_name`] / [`is_reserved_custom_ident`]) を持つ — CSS
/// Values 4 §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents> の
/// permanent な spec 除外規定であり、CSS-wide keyword の実装状況とは無関係
/// (CSS-wide keyword が実装されても変わらない) — 上の「不作為の一致」と
/// 混同しないこと。
///
/// **box property は「認識できない」側ではない** — `margin` / `padding` /
/// `border-*` / `width` / `height` はいずれも認識対象で、下記に variant を持つ。
/// `font-size: 1em` / `font-size: medium` / `font-size: larger` も valid で
/// ある。
/// **例を差し替えるときは sibling の [`crate::rule`] の
/// `drops_invalid_property_and_value` と揃えること** — 両者は同じ内容を
/// 説明しており、あちらだけ更新されて本 doc が取り残される drift が
/// 実際に起きた。
///
/// # `#[non_exhaustive]` semantics (fulgur / downstream consumer 向け verbatim)
///
/// enum-level `#[non_exhaustive]` は downstream の `match` を forward-compatible
/// にする (新 variant 追加時 `_ =>` arm が必ず求められる) が、**既存 variant の
/// tuple constructor 呼び出しは block しない**。したがって variant の payload
/// **type** が変わると constructor 側は普通に compile-break する。
///
/// cascade memory DoS 対策の一環でまさにこの break が発生し、以下を
/// fulgur consumer 向け migration 対象として表明する:
///
/// - [`Content`](PropertyValue::Content): `Content(Vec<ContentComponent>)` →
///   `Content(Arc<Vec<ContentComponent>>)`
/// - [`StringSet`](PropertyValue::StringSet): payload の outer `Vec<..>` を `Arc<Vec<..>>` に
///
/// 同じ cascade memory DoS 対策の counter-* への拡張は、当時 consumer への
/// live impact 0 だったが同 pattern:
///
/// - [`CounterReset`](PropertyValue::CounterReset) / [`CounterIncrement`](PropertyValue::CounterIncrement) /
///   [`CounterSet`](PropertyValue::CounterSet): `Vec<(SmolStr, i32)>` → `Arc<Vec<(SmolStr, i32)>>`
///
/// 同種の Arc-wrap パターンの踏襲 (目的は perf 改善であり、security 対策では
/// ない) は同 pattern を最後の non-Arc `Vec` payload に適用する:
///
/// - [`FontFamily`](PropertyValue::FontFamily): `FontFamily(Vec<Atom>)` →
///   `FontFamily(Arc<Vec<Atom>>)`
///
/// Pattern-match で payload を **読む** consumer は `Arc<Vec<T>>` の
/// `Deref<Target = Vec<T>>` → `Deref<Target = [T]>` chain により、`match` arm
/// で `PropertyValue::Content(components) => components.iter()` のような使い方が
/// **透過的に継続動作** する (`&Arc<Vec<T>>` は autoderef で `&[T]` として使える)。
/// 一方、`PropertyValue::Content(vec![...])` のように payload を **construct** する
/// 場合は `PropertyValue::Content(Arc::new(vec![...]))` への書き換えが必要。
/// `text-indent` の payload ([`TextIndentValue`])。
///
/// CSS Text 3 §8.1 grammar `<length-percentage> && hanging? && each-line?`
/// の全成分を保持する。`Copy` (全 field が `Copy`) のため cascade の
/// by-value 代入・inheritance copy が素朴に書ける。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextIndentValue {
    /// `<length-percentage>` 成分。
    pub length: Length,
    /// `hanging` keyword の有無。
    pub hanging: bool,
    /// `each-line` keyword の有無。
    pub each_line: bool,
}
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// A `--<ident>` custom property. The value remains token-preserving until
    /// the computed-value stage, where `var()` references are resolved.
    CustomProperty(CustomProperty),
    /// A known property value containing `var()` or a math function.
    Deferred(DeferredValue),
    /// `color: <color>` — inherited、initial: black。
    Color(CssColor),
    /// `background-color: <color>` — **non-inherited**、initial: `transparent`。
    /// CSS Backgrounds 3 §2.2 "Base Color: the background-color property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#background-color>。
    BackgroundColor(CssColor),
    /// `font-family: <family-name>#` — inherited。CSS Fonts 4 §2.1
    /// <https://www.w3.org/TR/css-fonts-4/#font-family-prop> の spec 上の
    /// initial は "depends on user agent"。本実装は `[Atom::from("serif")]`
    /// を採る ([`crate::property::initial_font_family`] doc 参照)。
    ///
    /// [`Arc<Vec<..>>`] wrap (同種の DoS 対策 fix の pattern 踏襲):
    /// cascade winner move (`apply_value`) と inheritance walk clone
    /// (`SpecifiedValues::inherit_from` の `parent.font_family.clone()`) が
    /// **shallow (Arc reference-count increment)** になる。`font-family` は inherited property なので
    /// non-inherited な counter-* / content / string-set とはコストの形が違う —
    /// 「毎 node で initial にリセットする」コストではなく「inheritance walk が
    /// 毎 node で親の値を運ぶ」コストで、N-node document あたり O(N) の
    /// 1-element `Vec` malloc になっていた (以前から存在した perf 上の課題)。
    /// `Arc<Vec<T>>: Deref<Target = Vec<T>>`
    /// により downstream の `.iter()` / `.len()` / `.is_empty()` は既存 pattern
    /// そのままで通る (dom/paint consumer 波及 0、`crates/raikiri-dom/src/layout.rs`
    /// の `cv.font_family.iter()` 含む)。
    FontFamily(Arc<Vec<Atom>>),
    /// `font-size: <absolute-size> | <length-percentage [0,∞]>` — inherited、
    /// initial: 16px (= `medium`)。CSS Fonts 4 §2.5
    /// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>。
    ///
    /// `<absolute-size>` (`xx-small` … `xxx-large`、`medium`) は親に依存しない
    /// 固定値なので、[`parse_font_size`] が §2.5.1 の scaling-factor table
    /// (<https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping>) を parse
    /// 時点で `medium` (16px) 基準の `Length::Px` へ解決し尽くす。
    /// `<relative-size>` (`larger` / `smaller`) は
    /// 継承先依存のため別 variant ([`Self::FontSizeRelative`]) を持つ —
    /// 理由は同 variant の doc を参照。`math` keyword は spec-valid だが
    /// 未実装 (MathML scaling algorithm が丸ごと未対応) として
    /// `parse_font_size` が `None` に落とす。
    FontSize(Length),
    /// `font-size: <relative-size>` (`larger` / `smaller`) — inherited、
    /// [`Self::FontSize`] と同じ `font-size` property の一部。CSS Fonts 4 §2.5
    /// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>。
    ///
    /// # `Self::FontSize(Length)` を再利用せず新 variant にした理由
    ///
    /// `bolder` / `lighter` (`font-weight`) と同型の親依存 read-modify-write が
    /// 必要 — 素朴には [`FontWeightValue`] 同様「`FontSize` の payload 型を
    /// keyword を持てる enum に差し替える」設計が対称だが、`PropertyValue` は
    /// `raikiri` (umbrella) crate が re-export しており、
    /// `crates/raikiri/tests/build_cascaded.rs` の
    /// `umbrella_re_exports_cover_computed_value_types_and_parse_options_fields`
    /// が `PropertyValue::FontSize(Length::Px(12.0))` の construction を
    /// **意図的に compile-time pin** している (umbrella re-export list の
    /// rationale、`crates/raikiri/src/lib.rs` 該当 comment 参照)。
    /// `FontSize` の payload 型を変えるとこの check が割れ、
    /// `crates/raikiri` 側の修正を要求する = umbrella crate に対する破壊的変更に
    /// なる (`Content`/`StringSet` payload 変更が同種の前例)。
    ///
    /// `PropertyValue` は `#[non_exhaustive]` なので **新 variant の追加**は
    /// 既存 tuple constructor 呼び出しを一切壊さない (enum-level
    /// `#[non_exhaustive]` の doc 参照) — そのため `FontSize` の型はそのまま
    /// 残し、`larger` / `smaller` 用に本 variant を追加する。[`RelativeFontSize`]
    /// は [`FontWeightValue`] と異なり `raikiri` (umbrella) の `pub use` list
    /// には**含めない** — 同 list に無い [`FontWeightValue`] と同じ非対称を
    /// 踏襲する (raikiri-style へ直接 dep する consumer のみ名指し可能)。
    ///
    /// # 解決タイミング
    ///
    /// `bolder` / `lighter` と同じく [`crate::cascade::apply_value`] が
    /// 親の computed font-size (staging 上は上書き前の
    /// `SpecifiedValues::font_size`、D5 invariant により常に
    /// `Length::Px(親の px)`) から絶対値へ解決し、結果を
    /// [`Self::FontSize`] 形 (`Length::Px`) で `target.font_size` に格納する —
    /// variant 自体は cascade winner の一時的な表現に留まり、
    /// [`crate::specified::SpecifiedValues`] 以降には残らない。page 経路は
    /// [`crate::cascade::resolve_against_inherited`] が同じ解決を行い、
    /// [`crate::page::PageCascadeResult::declarations`] に届く時点では
    /// 同じく [`Self::FontSize`] (`Length::Px`) に収束している。
    FontSizeRelative(RelativeFontSize),
    /// `font-weight: <font-weight-absolute> | bolder | lighter` — inherited、
    /// initial: `Absolute(400.0)`。CSS Fonts 4 §2.2
    /// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop>。
    ///
    /// payload は **specified value** ([`FontWeightValue`])。`bolder` /
    /// `lighter` は継承値依存の relative weight なので parse 段では解けず、
    /// [`crate::cascade::apply_value`] が親の computed weight から絶対値に
    /// 解決して [`crate::computed::ComputedValues::font_weight`] (`f32`) に
    /// 格納する。
    ///
    /// **page context 側も解決される**。
    /// [`crate::page::cascade_page`] は winner を
    /// [`crate::cascade::resolve_against_inherited`] (`apply_value` の sibling、
    /// 同じ relative-weight table を共有) に通してから
    /// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
    /// に格納するので、`@page { font-weight: bolder }` も `Absolute` に
    /// 落ちた形でしか public な結果に現れない。継承元は
    /// CSS Paged Media 3 §6 "Page Properties"
    /// <https://www.w3.org/TR/css-page-3/#page-properties> の "The page context
    /// inherits from the root element" どおり root element の computed weight
    /// (未供給時は同 §の legacy exception により initial 値 400)。
    FontWeight(FontWeightValue),
    /// `line-height: normal | <number> | <length-percentage>` — inherited、
    /// initial: [`LineHeight::Normal`]。CSS Inline 3 §5.1
    /// <https://www.w3.org/TR/css-inline-3/#line-height-property>。
    /// number-vs-length distinction は下流 (paint) が resolve context に落とす
    /// ための load-bearing 情報 (unitless number は specified-value inherit の
    /// spec special behavior、[`LineHeight`] doc 参照)。
    LineHeight(LineHeight),
    /// `display: <ident>` — non-inherited、initial: `inline` (CSS Display
    /// 3 §2 <https://www.w3.org/TR/css-display-3/#propdef-display>)。
    /// 現状受理する keyword: `block` / `inline` / `inline-block` / `none`
    /// (詳細は [`DisplayValue`] doc)。
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
    /// non-inherited。spec initial は `none` (CSS Lists 3 §4.1)、本 impl はそれを
    /// 空 list で表現する。
    /// missing integer は 0 に default (spec default)。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`) + inheritance walk clone (`resolve_inheritance` の
    /// `stack.push((child, computed.clone()))` + `out[idx] = computed.clone()`)
    /// が **shallow (Arc reference-count increment only)** になる。counter-* は non-inherited のため
    /// child は inherit_from で shared empty slot に落ちるが、winner までの経路
    /// (parent stack entry + cascaded candidates 蓄積) は deep-clone 経由だった。
    /// `* { counter-reset: c0 c1 ... cN }` × M element で O(N × M) → O(N + M)
    /// (cascade memory DoS 対策、Content/StringSet pattern の踏襲)。
    CounterReset(Arc<Vec<(SmolStr, i32)>>),
    /// CSS-wide `counter-reset: inherit` retained for page-context resolution.
    /// The page-margin used-value pass resolves this marker against the
    /// enclosing page counter scope.
    CounterResetInherit,
    /// `counter-increment: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited。spec initial は `none` (CSS Lists 3 §4.2)、本 impl はそれを
    /// 空 list で表現する。
    /// missing integer は 1 に default (spec default)。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::CounterReset`] と同 rationale。
    CounterIncrement(Arc<Vec<(SmolStr, i32)>>),
    /// `counter-set: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited。spec initial は `none` (CSS Lists 3 §4.2)、本 impl はそれを
    /// 空 list で表現する。
    /// missing integer は 0 に default (spec default)。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::CounterReset`] と同 rationale。
    CounterSet(Arc<Vec<(SmolStr, i32)>>),
    /// `content: normal | none | <content-list>` — non-inherited。spec initial は
    /// `normal`、本 impl は `normal` を空 list、`none` を
    /// [`ContentComponent::None`] sentinel で表現する。
    /// (pseudo-element 生成判断は下流 layer)。CSS Content 3 §1
    /// <https://www.w3.org/TR/css-content-3/#content-property>。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone + inheritance walk stack
    /// entry clone + per-node write が **shallow (Arc reference-count increment only)** になる。
    /// `* { content: "<large>" }` × N element の O(N × M) memory blow-up を
    /// 単一 heap slot 共有で塞ぐ (cascade memory DoS 対策)。
    ///
    /// **Consumer 向け**:
    /// pattern-match で payload を **読む** 場合は `Arc<Vec<T>>` の deref chain
    /// (Vec → slice) により従来の `PropertyValue::Content(components) =>
    /// components.iter().for_each(..)` がそのまま動作する。**construct** する
    /// 場合のみ `PropertyValue::Content(Arc::new(vec![..]))` の書き換えが必要。
    /// enum-level docstring §`#[non_exhaustive]` semantics も参照。
    Content(Arc<Vec<ContentComponent>>),
    /// `string-set: none | [ <custom-ident> <content-list> ]#` — non-inherited。
    /// spec initial は `none`、本 impl はそれを空 list で表現する。各 entry は
    /// `(name, content-list)` pair。
    /// CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>、
    /// `<content-list>` は CSS Content 3 §2 (parser 実装済)。
    /// 名前解決と runtime string() 参照は下流 (raikiri-dom) 責務。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::Content`] と同じ理由 —
    /// `* { string-set: name "<large>" }` × N element 経路の同種 DoS を塞ぐ。
    ///
    /// **Consumer 向け**:
    /// [`Self::Content`] と同じく outer `Arc` は read-side は deref 透過、
    /// construct-side (`PropertyValue::StringSet(vec![(name, items)])`) のみ
    /// `PropertyValue::StringSet(Arc::new(vec![..]))` に書き換える。inner
    /// `Vec<ContentComponent>` は Arc 化しない (per-entry share の hit率 が
    /// 想定できないため、outer 単段で攻撃経路を塞ぐ設計)。
    StringSet(Arc<Vec<(SmolStr, Vec<ContentComponent>)>>),
    /// `position: static | sticky | running(<custom-ident>)` — non-inherited、initial:
    /// `static`。
    /// CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax> および
    /// CSS Positioned Layout Module Level 3 §3
    /// <https://www.w3.org/TR/css-position-3/#sticky-pos>。
    /// 現状 scope では `running()` seed emit のみが下流に伝わる —
    /// `Static` / `Sticky` は `apply_value` で no-op (先行 `running()` を上書き suppress
    /// する discriminant 用途、spec default に相当; `Sticky` は将来の layout 連携まで
    /// 保持するだけで将来の layout 連携に備える)。
    /// `relative` / `absolute` / `fixed` は未実装 (将来対応)、parser 段で drop。
    Position(PositionValue),
    /// `top: auto | <length-percentage>` — **non-inherited**、initial: `auto`
    /// (CSS Positioned Layout Module Level 3 §3 <https://www.w3.org/TR/css-position-3/>).
    /// Used for `position: relative` offset (paint-time shift) and future absolute/fixed.
    Top(LengthOrAuto),
    /// `right: auto | <length-percentage>` — **non-inherited**、initial: `auto`.
    Right(LengthOrAuto),
    /// `bottom: auto | <length-percentage>` — **non-inherited**、initial: `auto`.
    Bottom(LengthOrAuto),
    /// `left: auto | <length-percentage>` — **non-inherited**、initial: `auto`.
    Left(LengthOrAuto),
    /// `text-align: start | end | left | right | center | justify | match-parent
    /// | justify-all` — **inherited**、initial: [`TextAlign::Start`]
    /// (CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
    /// <https://www.w3.org/TR/css-text-3/#text-align-property>)。
    /// spec 上 shorthand (text-align-all + text-align-last) だが
    /// 単一 field に保持 (**(b) 非対応**、longhand 分離は
    /// 後続 task で defer)。詳細は [`TextAlign`] doc-comment。
    TextAlign(TextAlign),
    /// `hanging-punctuation: none | first` — inherited, initial `none`.
    /// The line-layout consumer currently implements only a leading U+3000
    /// hang for `first`; other valid grammar arms remain outside this slice.
    HangingPunctuation(HangingPunctuation),
    /// `text-indent` — the full grammar is
    /// `<length-percentage> && hanging? && each-line?`; this variant covers
    /// **only** the `<length-percentage>` component (`hanging` / `each-line`
    /// are the other two, unimplemented — see "Scope carving" below).
    /// **inherited**、initial: `0` (CSS Text 3 §8.1 "First Line Indentation:
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
    /// payload は [`TextIndentValue`] (length + hanging/each-line flags)。
    /// cascade 層は length を `text_indent` へ、flags を `text_indent_hanging` /
    /// `text_indent_each_line` へ分配する。consumer (raikiri-dom realign) は
    /// parley の `IndentOptions` へ写像する。
    TextIndent(TextIndentValue),
    /// `padding-top: <length-percentage [0,∞]>` — non-inherited、initial: `0`。
    /// CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical>。
    /// spec grammar `<length-percentage [0,∞]>` の non-negative constraint は
    /// `parse_padding_side` が parse-time enforce (負値は None 返し → declaration drop)、
    /// `auto` keyword は grammar に含まれないため `parse_length_value` の
    /// Dimension / Percentage arm fall-through で自然 reject。
    PaddingTop(Length),
    /// `padding-right: <length-percentage [0,∞]>` — [`Self::PaddingTop`] と同 grammar。
    PaddingRight(Length),
    /// `padding-bottom: <length-percentage [0,∞]>` — [`Self::PaddingTop`] と同 grammar。
    PaddingBottom(Length),
    /// `padding-left: <length-percentage [0,∞]>` — [`Self::PaddingTop`] と同 grammar。
    PaddingLeft(Length),
    /// `padding: <'padding-top'>{1,4}` shorthand — non-inherited、initial:
    /// `Sides::all(Length::Px(0.0))`。CSS Box 3 §4.2
    /// <https://www.w3.org/TR/css-box-3/#padding-shorthand>。
    ///
    /// 1-4 value expansion (CSS Box 3 §4.2 の規定どおり — 逐語引用ではないため
    /// `verbatim` 表記は使わない):
    /// - 1 value: 全 4 side
    /// - 2 values: top/bottom = first, left/right = second
    /// - 3 values: top = first, left/right = second, bottom = third
    /// - 4 values: top / right / bottom / left (clockwise from top)
    ///
    /// **element cascade 段でこの variant は観測されない**:
    /// [`crate::rule::expand_shorthand_into`] が parse 出口
    /// (`parse_declaration_block`) と element cascade 入口
    /// ([`mod@crate::cascade`] の `collect_cascaded`) の両方で 4 longhand variant
    /// ([`PaddingTop`](Self::PaddingTop) / [`PaddingRight`](Self::PaddingRight) /
    /// [`PaddingBottom`](Self::PaddingBottom) / [`PaddingLeft`](Self::PaddingLeft))
    /// に展開するため (1/2/3/4 expansion + CSS Cascading L4 §3
    /// "Shorthand Properties" <https://www.w3.org/TR/css-cascade-4/#shorthand>
    /// verbatim: "A shorthand property sets all of its longhand sub-properties,
    /// exactly as if expanded in place." 準拠、cascade の per-side 勝ち抜けが自然に
    /// 成立する)。到達経路が無いのは上記の展開保証によるものであり、万一到達
    /// した場合の [`crate::cascade::apply_value`] の挙動は **safety net ではない**
    /// (framing を訂正済み) — `ComputedValues.padding` field
    /// 全 4 side を無条件に上書きし、4 longhand winner を必ず破壊する。到達した
    /// 時点で既に bug であり、穏当に degrade はしない (canonical な記述は
    /// [`crate::cascade::apply_value`] doc、および
    /// [`crate::rule::expand_shorthand_into`] doc 参照)。
    /// margin と同じ parse-time expansion model への migrate を検討 (follow-up task)。
    Padding(Sides<Length>),
    /// `padding-inline: <'padding-top'>{1,2}` shorthand — CSS Logical
    /// Properties and Values 1 §4.4 "Flow-Relative Padding: the
    /// padding-block-start, padding-block-end, padding-inline-start,
    /// padding-inline-end properties and padding-block and padding-inline
    /// shorthands" <https://www.w3.org/TR/css-logical-1/#propdef-padding-inline>。
    /// 2-value expansion: 1st value = `padding-inline-start`、2nd value =
    /// `padding-inline-end` (2nd 省略時は 1st を copy — [`StartEnd::both`])。
    ///
    /// # 物理写像 (Non-goal: writing-mode / direction 依存の flow-relative mapping)
    ///
    /// spec は `padding-inline-start`/`padding-inline-end` (および
    /// `padding-block-start`/`padding-block-end`) がどの物理 side
    /// (`padding-top`/`padding-right`/`padding-bottom`/`padding-left`) に
    /// 対応するかを、**その element 自身の computed `writing-mode` /
    /// `direction` / `text-orientation` に依存して決まる**と定める
    /// (CSS Logical Properties and Values 1 §4 冒頭)。raikiri は次の 2 点で
    /// この依存を切り、常に固定の物理 side へ写像する — 2 点の性質は
    /// **非対称**であることに注意:
    ///
    /// - **block axis** ([`PaddingBlock`](Self::PaddingBlock) 経由の
    ///   `padding-block-start`/`-end` → `padding-top`/`padding-bottom`):
    ///   raikiri は縦書きレンダリングパイプラインを実装しないため computed
    ///   writing-mode は常に [`WritingMode::HorizontalTb`] に潰れる
    ///   ([`resolve_writing_mode`] doc の Non-goal 節)。`horizontal-tb` の下
    ///   では block axis は常に vertical (block-start = top) であり、
    ///   `direction` は block axis の写像に一切関与しない (spec 上も
    ///   `horizontal-tb` + 任意の `direction` で block-start は常に top)。
    ///   したがってこちらは **近似ではなく厳密** — raikiri の scope
    ///   (computed writing-mode が常に `horizontal-tb`) の下では spec と
    ///   完全に一致する。
    /// - **inline axis** (本 variant 自身 / `padding-inline-start`/`-end` →
    ///   `padding-left`/`padding-right`): 上記に加えて **`direction: ltr`
    ///   を仮定**する。`direction` property 自体はこの crate に実装済み
    ///   ([`PropertyValue::Direction`] doc) だが、この写像はそれを
    ///   **参照しない** — 下記「なぜ 8 longhand が専用 variant を持たないか」
    ///   節が説明するとおり、本 PR はこの写像を parse 時点で (cascade winner
    ///   が確定する前に) 固定的に決める設計を選んだため。`direction` の
    ///   computed 値自体は cascade winner 確定後であれば this crate 内で
    ///   参照可能 ([`resolve_text_align_match_parent`] が
    ///   `SpecifiedValues::finalize` から同種の post-cascade 解決を既に行う
    ///   precedent) — direction-aware な解決はこの scope では意図的に
    ///   defer しているのであって、このアーキテクチャで原理的に不可能な
    ///   わけではない。**これは近似であり、
    ///   `direction: rtl` の element では spec と食い違う** —
    ///   `direction: rtl` では `padding-inline-start` は本来
    ///   `padding-right` に対応するが、raikiri は常に `padding-left` に
    ///   写像する。
    ///
    /// この非対称 (block axis は厳密、inline axis は近似) は [`OverflowValue`]
    /// doc の Non-goal 節が説明する `overflow-inline`/`overflow-block`
    /// (`overflow` shorthand の 2 component) の物理 x/y 写像より 1 段階
    /// 複雑 — overflow の inline/block はそれぞれ 1 axis に付き 1 value
    /// (`overflow-x`/`overflow-y` の pair) であり `start`/`end` の区別が
    /// 無いため direction は最初から無関係だった。本 property は axis
    /// ごとに `start`/`end` の 2 side を持つため、inline axis に限り
    /// direction 依存の近似が追加で必要になる。
    ///
    /// # なぜ 8 longhand (`padding-inline-start`/`-end`/`padding-block-start`/
    /// `-end` および margin 側の対応 4 つ) が専用 `PropertyValue` variant を
    /// 持たないか
    ///
    /// 上記の固定写像は cascade 時点の任意の状態 (親から継承した
    /// `direction` の computed 値、同 node の `direction` winner など) に
    /// 一切依存しない — parse 時点で既に確定する。したがって
    /// `padding-inline-start: <value>` は [`Self::PaddingLeft`]・
    /// `padding-block-start: <value>` は [`Self::PaddingTop`] と**全く同じ**
    /// `PropertyValue` を produce する (`parse_value` の該当 arm、
    /// `property_key_for_name` の該当 arm)。別 variant を新設しないのは、
    /// 既存の `word-wrap`/`overflow-wrap` legacy alias 化 (同 grammar の
    /// 別名を同じ `PropertyValue`/[`PropertyKey`] へ畳む、`parse_value` の
    /// `"overflow-wrap" | "word-wrap"` arm 参照) と同型の判断であり、
    /// cascade winner selection 上も「同じ物理 property を取り合う」という
    /// spec の実際の cascade 挙動 (CSS Logical Properties and Values 1 §4
    /// 冒頭の "corresponding flow-relative and physical properties are
    /// paired" — 対応する論理/物理 property は同じ物理 target を取り合う)
    /// と一致する。
    ///
    /// # element cascade 段でこの variant 自身は観測されない
    ///
    /// [`Self::Padding`] / [`Self::Margin`] と同じ理由 —
    /// [`crate::rule::expand_shorthand_into`] が parse 出口と element
    /// cascade 入口の両方で [`Self::PaddingLeft`]/[`Self::PaddingRight`]
    /// の 2 longhand に展開するため。到達した場合の
    /// [`crate::cascade::apply_value`] の挙動は **safety net ではない**
    /// (`Padding`/`Margin` arm と同じ framing)。
    PaddingInline(StartEnd<Length>),
    /// `padding-block: <'padding-top'>{1,2}` shorthand — [`Self::PaddingInline`]
    /// と同じ grammar/expansion/物理写像 rationale ([`Self::PaddingInline`]
    /// doc が canonical)、block axis 側 (`padding-block-start`/`-end` →
    /// `padding-top`/`padding-bottom` — 厳密写像、近似ではない)。CSS Logical
    /// Properties and Values 1 §4.4
    /// <https://www.w3.org/TR/css-logical-1/#propdef-padding-block>。
    PaddingBlock(StartEnd<Length>),
    /// `margin-top: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginTop(LengthOrAuto),
    /// Page-context-only marker for `margin-top: inherit`. The page parser
    /// resolves this against the root element before exposing declarations.
    MarginTopInherit,
    /// `margin-right: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginRight(LengthOrAuto),
    /// Page-context-only marker for `margin-right: inherit`.
    MarginRightInherit,
    /// `margin-bottom: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginBottom(LengthOrAuto),
    /// Page-context-only marker for `margin-bottom: inherit`.
    MarginBottomInherit,
    /// `margin-left: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginLeft(LengthOrAuto),
    /// Page-context-only marker for `margin-left: inherit`.
    MarginLeftInherit,
    /// Page-context-only marker for the `margin: inherit` shorthand. It is
    /// expanded into four side markers before page-context cascade.
    MarginInherit,
    /// `margin: <'margin-top'>{1,4}` shorthand — 4-side quad の一括指定
    /// (CSS Box 3 §3.2 <https://www.w3.org/TR/css-box-3/#margin-shorthand>)。
    ///
    /// **element cascade 段でこの variant は観測されない**:
    /// [`crate::rule::expand_shorthand_into`] が parse 出口
    /// (`parse_declaration_block`) と element cascade 入口
    /// ([`mod@crate::cascade`] の `collect_cascaded`) の両方で 4 longhand variant
    /// ([`MarginTop`](Self::MarginTop) / [`MarginRight`](Self::MarginRight) /
    /// [`MarginBottom`](Self::MarginBottom) / [`MarginLeft`](Self::MarginLeft))
    /// に展開するため (spec §3.2 の 1/2/3/4 expansion + CSS Cascading L4 §3
    /// "Shorthand Properties" <https://www.w3.org/TR/css-cascade-4/#shorthand>
    /// verbatim: "A shorthand property sets all of its longhand sub-properties,
    /// exactly as if expanded in place." 準拠、cascade の per-side 勝ち抜けが自然に
    /// 成立する)。到達経路が無いのは上記の展開保証によるものであり、万一到達
    /// した場合の [`crate::cascade::apply_value`] の挙動は **safety net ではない**
    /// (framing を訂正済み) — `ComputedValues.margin` field
    /// 全 4 side を無条件に上書きし、4 longhand winner を必ず破壊する。到達した
    /// 時点で既に bug であり、穏当に degrade はしない (canonical な記述は
    /// [`crate::cascade::apply_value`] doc、および
    /// [`crate::rule::expand_shorthand_into`] doc 参照)。
    Margin(Sides<LengthOrAuto>),
    /// `margin-inline: <'margin-top'>{1,2}` shorthand — CSS Logical
    /// Properties and Values 1 §4.2 "Flow-Relative Margins: the
    /// margin-block-start, margin-block-end, margin-inline-start,
    /// margin-inline-end properties and margin-block and margin-inline
    /// shorthands" <https://www.w3.org/TR/css-logical-1/#propdef-margin-inline>。
    /// grammar/expansion/物理写像の rationale は [`Self::PaddingInline`] doc
    /// が canonical — payload が `<length-percentage> | auto` である点のみ
    /// [`Self::Margin`] と同じく padding と異なる (`auto` は
    /// [`parse_margin_side`] がそのまま通す)。inline axis (本 variant 自身 /
    /// `margin-inline-start`/`-end` → `margin-left`/`margin-right`) 側 —
    /// [`Self::PaddingInline`] doc の「非対称」節が述べるとおり
    /// `direction: ltr` を仮定する近似 (`direction: rtl` では spec と食い違う)。
    MarginInline(StartEnd<LengthOrAuto>),
    /// `margin-block: <'margin-top'>{1,2}` shorthand — [`Self::MarginInline`]
    /// と同じ grammar/expansion/物理写像 rationale、block axis 側
    /// (`margin-block-start`/`-end` → `margin-top`/`margin-bottom` — 厳密
    /// 写像、近似ではない、[`Self::PaddingInline`] doc の「非対称」節参照)。
    /// CSS Logical Properties and Values 1 §4.2
    /// <https://www.w3.org/TR/css-logical-1/#propdef-margin-block>。
    MarginBlock(StartEnd<LengthOrAuto>),
    /// `border-top-width: <line-width>` — non-inherited、initial: `medium`
    /// = `Length::Px(3.0)` (CSS Backgrounds 3 §3.3
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。
    /// `<line-width>` = `<length [0,∞]> | thin | medium | thick`。
    /// `<percentage>` は grammar に含まれない (padding とは違う点)。keyword
    /// mapping は spec 規定値: thin=1px、medium=3px、
    /// thick=5px (`parse_border_width_side` doc 参照)。
    BorderTopWidth(Length),
    /// `border-right-width: <line-width>` — [`Self::BorderTopWidth`] と同 grammar。
    BorderRightWidth(Length),
    /// `border-bottom-width: <line-width>` — [`Self::BorderTopWidth`] と同 grammar。
    BorderBottomWidth(Length),
    /// `border-left-width: <line-width>` — [`Self::BorderTopWidth`] と同 grammar。
    BorderLeftWidth(Length),
    /// `border-top-style: <line-style>` — non-inherited、initial: `none`
    /// (CSS Backgrounds 3 §3.2
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-style>)。10 keyword は
    /// [`BorderStyle`] variant を参照。
    BorderTopStyle(BorderStyle),
    /// `border-right-style: <line-style>` — [`Self::BorderTopStyle`] と同 grammar。
    BorderRightStyle(BorderStyle),
    /// `border-bottom-style: <line-style>` — [`Self::BorderTopStyle`] と同 grammar。
    BorderBottomStyle(BorderStyle),
    /// `border-left-style: <line-style>` — [`Self::BorderTopStyle`] と同 grammar。
    BorderLeftStyle(BorderStyle),
    /// `border-top-color: <color>` — non-inherited、initial: `currentcolor`
    /// keyword ([`BorderColor::CurrentColor`]、CSS Backgrounds 3 §3.1
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-color> "Initial:
    /// currentcolor")。cascade static side は [`BorderColor`] enum で
    /// specified value (currentcolor vs. resolved `<color>`) を保持し、
    /// used-value resolution (currentcolor → 同 node computed `color` property
    /// lookup、CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>)
    /// は paint scope 責務。
    /// (`CssColor` から [`BorderColor`] へ格上げ済み)
    BorderTopColor(BorderColor),
    /// `border-right-color: <color>` — [`Self::BorderTopColor`] と同 grammar。
    BorderRightColor(BorderColor),
    /// `border-bottom-color: <color>` — [`Self::BorderTopColor`] と同 grammar。
    BorderBottomColor(BorderColor),
    /// `border-left-color: <color>` — [`Self::BorderTopColor`] と同 grammar。
    BorderLeftColor(BorderColor),
    /// `border: <line-width> || <line-style> || <color>` shorthand — 4 side
    /// 全てに同一の [`Border`] を配る (CSS Backgrounds 3 §3.4
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>)。
    ///
    /// spec grammar は `||` (any-order、each component at most once、at least
    /// 1 必須) — `parse_border_shorthand` が unfilled slot loop で peel する。
    /// 省略成分は initial: width=`Length::Px(3.0)` (medium)、style=`BorderStyle::None`、
    /// color=[`BorderColor::CurrentColor`] (spec §3.1 initial)。
    ///
    /// **element cascade 段でこの variant は観測されない**:
    /// [`crate::rule::expand_shorthand_into`] が parse 出口
    /// (`parse_declaration_block`) と element cascade 入口
    /// ([`mod@crate::cascade`] の `collect_cascaded`) の両方で 12 longhand variant
    /// (4 side × 3 sub-property)
    /// に展開するため (spec CSS Cascading L4 §3 "Shorthand Properties"
    /// <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim: "A shorthand
    /// property sets all of its longhand sub-properties, exactly as if expanded
    /// in place." 準拠、cascade の per-side / per-sub-property 勝ち抜けが自然に
    /// 成立する — margin / padding shorthand precedent 踏襲)。到達経路が無いのは
    /// 上記の展開保証によるものであり、万一到達した場合の
    /// [`crate::cascade::apply_value`] の挙動は **safety net ではない**
    /// (framing を訂正済み) — `ComputedValues.border` field 全
    /// 4 side × 3 sub-property を無条件に上書きし、12 longhand winner を必ず
    /// 破壊する。到達した時点で既に bug であり、穏当に degrade はしない
    /// (canonical な記述は [`crate::cascade::apply_value`] doc、
    /// および [`crate::rule::expand_shorthand_into`] doc 参照)。
    ///
    /// ⚠️ spec の "all of its longhand sub-properties" には reset-only の
    /// `border-image-*` (5 本) も含まれる (CSS Backgrounds 3 §3.4: the `border`
    /// shorthand also resets `border-image` to its initial value) が、それらは
    /// 未実装なので本展開は 12 longhand に留まる — 下の `# Non-goals` 節参照。
    ///
    /// # Non-goals (spec deviation 明示)
    ///
    /// spec §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
    /// では border shorthand が **border-image-* も reset** する (spec verbatim は
    /// `parse_border_shorthand` doc に 1 site だけ置く) が、本 crate は
    /// border-image を実装しておらず、未対応 (spec-valid だが本 crate の
    /// scope 外)。future 統合 task で border-image longhand と併せて
    /// 対応。
    Border(Sides<Border>),
    /// `border-style: <line-style>{1,4}` — non-inherited (CSS Backgrounds 3
    /// §3.2 `<line-style>` × §3.4 shorthands
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>)。
    /// 1-4 value expansion は margin precedent
    /// (`parse_margin_shorthand` と同型): 1 → all、2 → vertical/horizontal、
    /// 3 → top/horizontal/bottom、4 → clockwise。rule.rs で 4 longhand
    /// (`BorderTopStyle` 等) へ展開される。
    BorderStyle(Sides<BorderStyle>),
    /// `border-width: <line-width>{1,4}` — non-inherited (CSS Backgrounds 3
    /// §3.3 × §3.4)。各 side の grammar は `border-*-width` と同一
    /// (`parse_border_width_side`: thin/medium/thick keyword + 非負
    /// `<length>`)。1-4 value expansion は margin precedent と同型。
    BorderWidth(Sides<Length>),
    /// `border-color: <color>{1,4}` — non-inherited (CSS Backgrounds 3 §3.1
    /// × §3.4)。各 side の grammar は `border-*-color` と同一
    /// (`parse_border_color`: `currentcolor` / named / hash / function)。
    /// 1-4 value expansion は margin precedent と同型。
    BorderColor(Sides<BorderColor>),
    /// `width: auto | <length-percentage [0,∞]>` — non-inherited、initial: `auto`
    /// (CSS Sizing 3 §3.1.1 <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
    ///
    /// spec value grammar は `auto | <length-percentage [0,∞]> | min-content |
    /// max-content | fit-content(<length-percentage>)` だが、min-content /
    /// max-content / fit-content() は未実装 (将来対応) として現状
    /// silent drop、`auto` と non-negative `<length-percentage>` のみ受理。
    /// 負値は spec grammar `[0,∞]` violation として drop。
    ///
    /// `auto` の resolution は下流 layout (raikiri-dom apply_computed_to_style
    /// bridge、taffy::Style::size.width 反映) 責務。
    Width(LengthOrAuto),
    /// `height: <length-percentage [0,∞]> | auto` — **non-inherited**、initial:
    /// `auto` (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
    ///
    /// 現状 scope では `auto` + 非負
    /// `<length-percentage>` の 2 分岐のみ受理。`min-content` / `max-content` /
    /// `fit-content(<length-percentage>)` は spec-valid だが未実装
    /// (将来対応) として parser 段で silent drop する — `parse_height` doc 参照。
    ///
    /// margin (`<length-percentage> | auto`) の non-negative constraint が違うだけの
    /// grammar のため、payload 型は sibling [`Self::Width`] と同じ
    /// [`LengthOrAuto`] を reuse (sibling: `parse_padding_side` の非負フィルタ +
    /// `parse_margin_side` の auto 分岐を合成、`parse_height` doc 参照)。
    ///
    /// resolve (percentage → containing block, `LengthOrAuto::Auto` の実 layout
    /// 高さ計算) は下流 (raikiri-dom `apply_computed_to_style` bridge、future task)
    /// 責務 — 本 crate は cascade static side に留まり raw specified value を保持。
    Height(LengthOrAuto),
    /// `max-width: none | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**、initial: `none`
    /// (CSS Sizing 3 §3.2 <https://www.w3.org/TR/css-sizing-3/#max-size-properties>).
    /// `none` maps to `LengthOrAuto::Auto` as placeholder (no max).
    /// Intrinsic keywords map similarly to Auto (WPT parsing valid, layout pending).
    MaxWidth(LengthOrAuto),
    /// `max-height: none | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**、initial: `none`
    /// (CSS Sizing 3 §3.2 <https://www.w3.org/TR/css-sizing-3/#max-size-properties>).
    MaxHeight(LengthOrAuto),
    /// `min-width: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**、initial: `auto`
    /// (CSS Sizing 3 §4 <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
    /// `auto` maps to [`LengthOrAuto::Auto`] (no minimum). Intrinsic keywords map
    /// similarly to Auto (WPT parsing valid, layout pending) — sibling
    /// [`Self::Width`] arms use the same placeholder shape.
    MinWidth(LengthOrAuto),
    /// `min-height: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` — **non-inherited**、initial: `auto`
    /// (CSS Sizing 3 §4 <https://www.w3.org/TR/css-sizing-3/#min-size-properties>).
    /// Same placeholder shape as sibling [`Self::MinWidth`].
    MinHeight(LengthOrAuto),
    /// `box-sizing: content-box | border-box` — **non-inherited**、initial:
    /// `content-box` (CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing
    /// property" <https://www.w3.org/TR/css-sizing-3/#box-sizing>)。
    BoxSizing(BoxSizing),
    /// `direction: ltr | rtl` — **inherited**、initial: [`Direction::Ltr`]
    /// (CSS Writing Modes 4 §2.1 "Specifying Directionality: the direction
    /// property" <https://www.w3.org/TR/css-writing-modes-4/#direction>)。
    /// computed value = specified value (相対解決なし、[`Direction`] doc 参照)。
    /// 唯一の consumer は [`resolve_text_align_match_parent`] だが、property
    /// 自体は CSS Paged Media 3 Appendix A page-property-list にも独立に
    /// 現れる ([`Direction`] doc の verbatim 確認済み引用参照)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照)
    Direction(Direction),
    /// `overflow-x: visible | hidden | clip | scroll | auto` —
    /// **non-inherited**、initial: [`OverflowValue::Visible`] (CSS Overflow 3
    /// §3.1 <https://www.w3.org/TR/css-overflow-3/#overflow-properties>)。
    /// computed value は同一 node の `overflow-y` に依存しうる —
    /// [`resolve_overflow`] 参照 (単純代入ではない、[`crate::cascade::apply_value`]
    /// の本 variant arm doc も参照)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照)
    OverflowX(OverflowValue),
    /// `overflow-y: visible | hidden | clip | scroll | auto` —
    /// [`Self::OverflowX`] と同 grammar / initial / non-inherited、逆 axis。
    /// (末尾配置は [`Self::OverflowX`] と同理由)
    OverflowY(OverflowValue),
    /// `overflow: <'overflow-block'>{1,2}` shorthand — CSS Overflow 3 §3.1
    /// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>。1 value
    /// は両 axis、2 value は 1st=x, 2nd=y ([`OverflowXY`] doc 参照、spec は
    /// logical `overflow-block`/`overflow-inline` に写像するが raikiri-style
    /// は writing-mode 未実装のため物理 axis にそのまま写像する —
    /// [`OverflowValue`] doc の Non-goal 節と同型の carve out)。
    ///
    /// **element cascade 段でこの variant は観測されない**:
    /// [`Self::Padding`] と同型、[`crate::rule::expand_shorthand_into`] が
    /// parse 出口と element cascade 入口の両方で
    /// [`OverflowX`](Self::OverflowX) / [`OverflowY`](Self::OverflowY) の 2
    /// longhand に展開するため (CSS Cascading L4 §3 "Shorthand Properties"
    /// <https://www.w3.org/TR/css-cascade-4/#shorthand> 準拠)。万一到達した
    /// 場合の [`crate::cascade::apply_value`] の挙動は **safety net ではない**
    /// — [`Self::Padding`] doc と同じ framing、詳細は同 doc 参照。
    /// (末尾配置は [`Self::OverflowX`] と同理由)
    Overflow(OverflowXY),
    /// `text-decoration-line` — **non-inherited**、initial:
    /// [`TextDecorationLine::NONE`] (CSS Text Decoration Module Level 3 §2.1
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>、
    /// "Inherited: no")。computed value = specified keyword(s)
    /// ([`TextDecorationLine`] doc 参照、length を運ばないため相対解決なし)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    TextDecorationLine(TextDecorationLine),
    /// `text-decoration-style` — **non-inherited**、initial:
    /// [`TextDecorationStyle::Solid`] (CSS Text Decoration Module Level 3
    /// §2.2 <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>、
    /// "Inherited: no")。computed value = specified keyword
    /// ([`TextDecorationStyle`] doc 参照)。(末尾配置は [`Self::TextDecorationLine`]
    /// と同理由)
    TextDecorationStyle(TextDecorationStyle),
    /// `text-decoration-color` — **non-inherited**、initial:
    /// [`TextDecorationColor::CurrentColor`] (CSS Text Decoration Module
    /// Level 3 §2.3
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>、
    /// "Inherited: no")。computed value = computed color
    /// ([`TextDecorationColor`] doc 参照、used-value resolution は paint
    /// scope 責務)。(末尾配置は [`Self::TextDecorationLine`] と同理由)
    TextDecorationColor(TextDecorationColor),
    /// `text-decoration` shorthand ([`TextDecorationShorthand`] 参照)。
    ///
    /// **element cascade 段でこの variant は観測されない**:
    /// [`Self::Padding`] と同型、[`crate::rule::expand_shorthand_into`] が
    /// parse 出口と element cascade 入口の両方で
    /// [`TextDecorationLine`](Self::TextDecorationLine) /
    /// [`TextDecorationStyle`](Self::TextDecorationStyle) /
    /// [`TextDecorationColor`](Self::TextDecorationColor) の 3 longhand に
    /// 展開するため (CSS Cascading L4 §3 "Shorthand Properties"
    /// <https://www.w3.org/TR/css-cascade-4/#shorthand> 準拠)。万一到達した
    /// 場合の [`crate::cascade::apply_value`] の挙動は **safety net ではない**
    /// — [`Self::Padding`] doc と同じ framing、詳細は同 doc 参照。
    /// (shorthand key は longhand の後に置く既存 convention — [`Self::Padding`] /
    /// [`Self::Margin`] / [`Self::Border`] / [`Self::Overflow`] と同じ並び)
    TextDecoration(TextDecorationShorthand),
    /// `vertical-align: baseline | sub | super | middle | text-top |
    /// text-bottom | <length>` — **non-inherited**、initial:
    /// [`VerticalAlign::Baseline`] (CSS 2.1 §10.8.1
    /// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>)。
    /// computed value = keyword はそのまま、`<length>` は絶対化済み
    /// ([`VerticalAlign`] doc の "Scope carving" 節参照)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    VerticalAlign(VerticalAlign),
    /// `font-style: normal | italic | oblique` — **inherited**、initial:
    /// [`FontStyle::Normal`] (CSS Fonts 4 §2.4 [`FontStyle`] doc 参照)。
    /// computed value = specified keyword ([`FontStyle`] doc の Scope
    /// carving 節参照、`oblique <angle>` の angle 引数 / `left` / `right` は
    /// 未実装)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    FontStyle(FontStyle),
    /// `text-transform: none | capitalize | uppercase | lowercase` —
    /// **inherited**、initial: [`TextTransform::None`] (CSS Text Module
    /// Level 3 §2.1 [`TextTransform`] doc 参照)。computed value = specified
    /// keyword ([`TextTransform`] doc の Scope carving 節参照、`full-width`
    /// / `full-size-kana` は未実装)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    TextTransform(TextTransform),
    /// `visibility: visible | hidden | collapse` — **inherited**、initial:
    /// [`Visibility::Visible`] (CSS Display 3 §4 [`Visibility`]
    /// doc 参照)。computed value = specified keyword ([`Visibility`] doc の
    /// Scope carving 節参照、`collapse` の formatting-context 固有な
    /// space-saving 効果は未実装)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    Visibility(Visibility),
    /// `z-index: auto | <integer>` — **non-inherited**、initial:
    /// [`ZIndexValue::Auto`] ([`ZIndexValue`] doc 参照、CSS2 §9.9.1
    /// "Inherited: no")。computed value = specified value ([`ZIndexValue`]
    /// doc 参照、length を運ばないため相対解決なし)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    ZIndex(ZIndexValue),
    /// `word-break: normal | keep-all | break-all` — **inherited**、initial:
    /// [`WordBreak::Normal`] (CSS Text 3 §5.1 [`WordBreak`] doc 参照)。
    /// computed value = specified keyword ([`WordBreak`] doc の Scope
    /// carving 節参照、deprecated `break-word` value は未実装)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    WordBreak(WordBreak),
    /// `overflow-wrap: normal | break-word | anywhere` (legacy alias
    /// `word-wrap`) — **inherited**、initial: [`OverflowWrap::Normal`]
    /// (CSS Text 3 §5.4 [`OverflowWrap`] doc 参照)。computed value =
    /// specified keyword ([`OverflowWrap`] doc 参照)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    OverflowWrap(OverflowWrap),
    /// `letter-spacing: normal | <length>` — **inherited**、initial:
    /// [`LengthOrNormal::Normal`] (CSS Text 3 §7.2 "Tracking: the
    /// letter-spacing property"
    /// <https://www.w3.org/TR/css-text-3/#letter-spacing-property>).
    /// computed value: an absolute length (`normal` computes to zero —
    /// [`LengthOrNormal`] doc 参照。[`LineHeight::Normal`] とは異なり、
    /// `letter-spacing: normal` は font metrics に依存せず常に `0` へ絶対化
    /// できるため、computed 層で keyword を保持する必要が無い)。
    ///
    /// **Non-goal**: §7.2 の "For legacy reasons, a computed letter-spacing
    /// of zero yields a resolved value (`getComputedStyle()` return value)
    /// of `normal`." は CSSOM の resolved-value serialization 規則であり、
    /// この crate に CSSOM surface が無いため対象外。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    LetterSpacing(LengthOrNormal),
    /// `word-spacing: normal | <length>` — **inherited**、initial:
    /// [`LengthOrNormal::Normal`] (CSS Text 3 §7.1 "Word Spacing: the
    /// word-spacing property"
    /// <https://www.w3.org/TR/css-text-3/#word-spacing-property>).
    /// computed value: an absolute length ([`Self::LetterSpacing`] doc の
    /// "computes to zero" 節と同じ扱い)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    WordSpacing(LengthOrNormal),
    /// `break-before: auto | avoid | avoid-page | page` (legacy shorthand
    /// `page-break-before`, [`BreakBetween`] doc の「legacy shorthand」節
    /// 参照) — **non-inherited**、initial: [`BreakBetween::Auto`] (CSS
    /// Fragmentation Module Level 3 §3.1 [`BreakBetween`] doc 参照)。
    /// computed value = specified keyword ([`BreakBetween`] doc の Scope
    /// carving 節参照)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    BreakBefore(BreakBetween),
    /// `break-after: auto | avoid | avoid-page | page` (legacy shorthand
    /// `page-break-after`, [`BreakBetween`] doc の「legacy shorthand」節
    /// 参照) — **non-inherited**、initial: [`BreakBetween::Auto`] (CSS
    /// Fragmentation Module Level 3 §3.1 [`BreakBetween`] doc 参照)。
    /// computed value = specified keyword ([`BreakBetween`] doc の Scope
    /// carving 節参照)。
    /// (末尾に追加、[`Self::BreakBefore`] と同じ配置理由)
    BreakAfter(BreakBetween),
    /// `break-inside: auto | avoid | avoid-page` (legacy shorthand
    /// `page-break-inside`, [`BreakInside`] doc の「legacy shorthand」節
    /// 参照) — **non-inherited**、initial: [`BreakInside::Auto`] (CSS
    /// Fragmentation Module Level 3 §3.2 [`BreakInside`] doc 参照)。
    /// computed value = specified keyword ([`BreakInside`] doc の Scope
    /// carving 節参照 — [`BreakBetween`] とは disjoint な、より小さい value
    /// set を持つ別 type)。
    /// (末尾に追加、[`Self::BreakBefore`] と同じ配置理由)
    BreakInside(BreakInside),
    /// `float: none | left | right` — **non-inherited**、initial:
    /// [`FloatValue::None`] (CSS2 §9.5.1 "Inherited: no"、[`FloatValue`]
    /// doc 参照)。computed value = specified value ([`FloatValue`] doc
    /// 参照、length を運ばないため相対解決なし)。この値が `none` 以外の
    /// ときの `display` 強制変換は別途 [`resolve_display_for_float`] が
    /// 解決する — 本 variant 自体は `float` の cascaded value のみを運ぶ。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    Float(FloatValue),
    /// `clear: none | left | right | both` — **non-inherited**、initial:
    /// [`ClearValue::None`] (CSS2 §9.5.2 "Inherited: no"、[`ClearValue`]
    /// doc 参照)。computed value = specified value ([`ClearValue`] doc
    /// 参照、length を運ばないため相対解決なし)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    Clear(ClearValue),
    /// `white-space: normal | pre | nowrap | pre-wrap | pre-line` —
    /// **inherited**、initial: [`WhiteSpace::Normal`] (CSS Text 3 §3
    /// [`WhiteSpace`] doc 参照)。computed value = specified keyword
    /// ([`WhiteSpace`] doc の Scope carving 節参照、6th keyword
    /// `break-spaces` は未実装)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    WhiteSpace(WhiteSpace),
    /// `text-wrap: wrap | nowrap` (subset — see [`TextWrapMode`] doc).
    TextWrap(TextWrapMode),
    /// `flex-direction: row | row-reverse | column | column-reverse` —
    /// non-inherited、initial: [`FlexDirectionValue::Row`]
    /// ([`FlexDirectionValue`] doc 参照)。
    FlexDirection(FlexDirectionValue),
    /// `flex-wrap: nowrap | wrap | wrap-reverse` — non-inherited、initial:
    /// [`FlexWrapValue::NoWrap`] ([`FlexWrapValue`] doc 参照)。
    FlexWrap(FlexWrapValue),
    /// `flex-grow: <number [0,∞]>` — non-inherited、initial: `0.0`
    /// (CSS Flexible Box Layout Module Level 1 §7.2.1
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-grow-property>)。
    /// [`parse_nonneg_finite_number`] が `[0,∞]` **と** finiteness を parse
    /// 時に enforce する ([`ComputedValues::flex_grow`] doc の sink-guard
    /// 注記参照)。
    ///
    /// [`ComputedValues::flex_grow`]: crate::computed::ComputedValues::flex_grow
    FlexGrow(f32),
    /// `flex-shrink: <number [0,∞]>` — non-inherited、initial: `1.0`
    /// (CSS Flexible Box Layout Module Level 1 §7.2.2
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-shrink-property>)。
    /// [`Self::FlexGrow`] と同じ parse-time enforcement。
    FlexShrink(f32),
    /// `flex-basis: content | <'width'>` — non-inherited、initial:
    /// [`FlexBasisValue::Auto`] ([`FlexBasisValue`] doc 参照)。
    FlexBasis(FlexBasisValue),
    /// `flex: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]`
    /// shorthand — non-inherited、initial: `0 1 auto`
    /// ([`FlexShorthand`] doc 参照)。[`crate::rule::expand_shorthand_into`]
    /// が [`Self::FlexGrow`] / [`Self::FlexShrink`] / [`Self::FlexBasis`] の
    /// 3 longhand に展開するため、element cascade 段には通常到達しない
    /// (`Self::Margin` 等の shorthand precedent と同じ shape)。
    Flex(FlexShorthand),
    /// `flex-flow: <'flex-direction'> || <'flex-wrap'>` shorthand —
    /// non-inherited、initial: `row nowrap`
    /// ([`FlexFlow`] doc 参照)。[`crate::rule::expand_shorthand_into`]
    /// が [`Self::FlexDirection`] / [`Self::FlexWrap`] の 2 longhand に
    /// 展開するため、element cascade 段には通常到達しない
    /// ([`Self::Flex`] と同じ shape)。
    FlexFlow(FlexFlow),
    /// `order: <integer>` — non-inherited、initial: `0`
    /// (CSS Flexible Box Layout Module Level 1 §4.2 "Display Order: the order
    /// property" <https://www.w3.org/TR/css-flexbox-1/#order-property>)。
    /// Computed value = specified integer (相対解決なし、length を運ばない
    /// ため [`Self::ZIndex`] と同じ opaque pass-through)。
    Order(i32),
    /// `justify-content` — non-inherited、initial:
    /// [`ContentAlignmentValue::Normal`] ([`ContentAlignmentValue`] doc 参照)。
    JustifyContent(ContentAlignmentValue),
    /// `align-content` — non-inherited、initial:
    /// [`ContentAlignmentValue::Normal`]。[`Self::JustifyContent`] と同じ
    /// payload 型を共有する ([`ContentAlignmentValue`] doc 参照)。
    AlignContent(ContentAlignmentValue),
    /// `align-items` — non-inherited、initial:
    /// [`SelfAlignmentValue::Normal`] ([`SelfAlignmentValue`] doc 参照)。
    AlignItems(SelfAlignmentValue),
    /// `align-self` — non-inherited、initial: [`AlignSelfValue::Auto`]
    /// ([`AlignSelfValue`] doc 参照)。
    AlignSelf(AlignSelfValue),
    /// `row-gap: normal | <length-percentage [0,∞]>` — non-inherited、
    /// initial: [`LengthOrNormal::Normal`] (CSS Box Alignment Module Level 3
    /// §8.1 <https://www.w3.org/TR/css-align-3/#propdef-row-gap>)。
    /// [`LengthOrNormal`] を再利用する ([`parse_gap_value`] doc 参照 —
    /// `letter-spacing`/`word-spacing` とは異なり percentage を受理する点に
    /// 注意)。
    RowGap(LengthOrNormal),
    /// `column-gap: normal | <length-percentage [0,∞]>` — non-inherited、
    /// initial: [`LengthOrNormal::Normal`]。[`Self::RowGap`] と同じ grammar。
    ColumnGap(LengthOrNormal),
    /// `gap: <'row-gap'> <'column-gap'>?` shorthand — non-inherited、initial:
    /// "see individual properties" ([`GapShorthand`] doc 参照)。
    /// [`crate::rule::expand_shorthand_into`] が [`Self::RowGap`] /
    /// [`Self::ColumnGap`] の 2 longhand に展開する。
    Gap(GapShorthand),
    /// `place-content: <'align-content'> <'justify-content'>?` shorthand —
    /// non-inherited、initial: `normal` ([`PlaceContentShorthand`] doc 参照)。
    /// [`crate::rule::expand_shorthand_into`] が [`Self::AlignContent`] /
    /// [`Self::JustifyContent`] の 2 longhand に展開する。
    PlaceContent(PlaceContentShorthand),
    /// `hyphens: none | manual | auto` — **inherited**、initial:
    /// [`Hyphens::Manual`] (CSS Text 3 §5.3 [`Hyphens`] doc 参照)。computed
    /// value = specified keyword ([`Hyphens`] doc の Scope carving /
    /// Downstream handoff 節参照 — `auto` は dictionary-based hyphenation を
    /// 実装せず、distinct variant のまま残す)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    Hyphens(Hyphens),
    /// `tab-size: <number [0,∞]> | <length [0,∞]>` — **inherited**、initial:
    /// [`TabSize::Number`]`(8.0)` (CSS Text Module Level 3 §4.2 "Tab
    /// Character Size: the tab-size property"
    /// <https://www.w3.org/TR/css-text-3/#tab-size-property>). computed
    /// value: the specified number or an absolutized length ([`TabSize`]
    /// doc 参照)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
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
    /// **inherited**、initial: [`FontVariantCaps::Normal`] (CSS Fonts 3
    /// §6.6 [`FontVariantCaps`] doc 参照)。computed value = specified
    /// keyword ([`FontVariantCaps`] doc の Scope carving 節参照、
    /// `font-variant` shorthand は未実装)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    FontVariantCaps(FontVariantCaps),
    /// `quotes: none | [ <string> <string> ]+` — **inherited**。
    ///
    /// CSS Content Module Level 3 §2.4.1 "Quotation Mark System: the quotes
    /// property" <https://www.w3.org/TR/css-content-3/#quotes-property>、
    /// 前身の CSS2 §12.3.1
    /// <https://www.w3.org/TR/CSS21/generate.html#quotes-specify> と同じ
    /// legacy grammar `[ <string> <string> ]+ | none` を実装。CSS Content 3 が
    /// 追加した `auto` / `match-parent` keyword alternative は本 crate の
    /// scope 外 (未実装、spec-valid だが parse 時に reject — 下記 "非対応" 節)。
    ///
    /// 各 pair は nesting level (quote depth) ごとの (open, close) 引用符
    /// 文字列。quote depth の定義と pair 選択規則は本 propdef 自体ではなく
    /// `<quote>` keyword ([`QuoteKeyword`]) 側の section — CSS Content 3
    /// §2.4.2 <https://www.w3.org/TR/css-content-3/#quote-values> (前身
    /// CSS2 §12.3.2 <https://www.w3.org/TR/CSS21/generate.html#quotes-insert>
    /// も同旨) — が定める。verbatim (§2.4.2): "the number of occurrences of
    /// open-quote in all generated text before the current occurrence,
    /// minus the number of occurrences of close-quote […]. If the depth is
    /// 0, the first pair is used, if the depth is 1, the second pair is
    /// used, etc. […] If the depth is greater than the number of pairs, the
    /// last pair is repeated." — depth は **0-indexed** (depth 0 が 1 pair
    /// 目) であり、depth が pair 数を超えたら最終 pair を再利用する。
    ///
    /// **`content` property の `open-quote` / `close-quote` keyword
    /// ([`QuoteKeyword`]) との関係**: `<quote>` keyword 自体は nesting depth
    /// の増減と「挿入するかどうか」だけを表現し、実際の文字列は決めない
    /// ([`QuoteKeyword`] doc 参照)。depth → 実際の引用符文字列への解決は本
    /// variant の値 (nesting level ごとの pair 列) を要するが、その解決自体は
    /// 本 crate の static-side scope 外 — 下流 (raikiri-dom) が `content` の
    /// [`ContentComponent::Quote`] component 列と本 property の computed
    /// value を併せて runtime resolve する ([`QuoteKeyword`] doc の「resolve
    /// は downstream 責務」節と同じ分担)。
    ///
    /// spec 上 initial value は "depends on user agent" (CSS2 §12.3.1) —
    /// 具体的な引用符文字列を規定しない。本実装は 独立実装方針 (他実装の UA
    /// 既定値を持ち込まない) により、宣言が無い場合の初期値も `none` と同じ
    /// 空 list で表現する ([`empty_quotes_entries`] 参照)。
    ///
    /// **非対応 (spec-valid)**: `auto` / `match-parent` — CSS Content 3
    /// §2.4.1 が legacy grammar (CSS2 §12.3.1) に追加した keyword
    /// alternative。本 crate は未実装で、どちらも parse 時に reject する
    /// (`parse_quotes_property` の grammar が受理しないため、declaration が
    /// silent drop される)。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::CounterReset`] と同 rationale
    /// (`* { quotes: "«" "»" }` × N element の cascade winner clone /
    /// inheritance walk clone を shallow Arc reference-count increment にする、DoS 対策)。
    Quotes(Arc<Vec<(SmolStr, SmolStr)>>),
    /// `text-shadow: none | <shadow>#` — **inherited**、initial: `none`
    /// (CSS Text Decoration Module Level 3 §4
    /// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>,
    /// "Initial: none" / "Inherited: yes")。`none` は空 list で表現する
    /// ([`TextShadowItem`] doc 参照、[`Self::CounterReset`] 等と同じ
    /// precedent)。computed value = 各要素の length を絶対化した list
    /// (spec: "a list, each item consisting of three absolute lengths plus a
    /// computed color") — 絶対化は phase 3 に委ねる ([`Self::LetterSpacing`]
    /// と同じ「specified 表現のまま格納」handling)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    TextShadow(Arc<Vec<TextShadowItem>>),
    /// `border-radius` — non-inherited。four-corner `<length-percentage>` shorthand を
    /// [`BorderRadius`] に展開する。percentage は computed 層まで保持し、
    /// slash-separated elliptical form は未対応。
    BorderRadius(BorderRadius),
    /// `border-radius: inherit` — resolved from the parent computed corners.
    BorderRadiusInherit,
    /// `border-top-left-radius` longhand (circular `<length>` subset).
    BorderRadiusTopLeft(Length),
    /// `border-top-right-radius` longhand (circular `<length>` subset).
    BorderRadiusTopRight(Length),
    /// `border-bottom-right-radius` longhand (circular `<length>` subset).
    BorderRadiusBottomRight(Length),
    /// `border-bottom-left-radius` longhand (circular `<length>` subset).
    BorderRadiusBottomLeft(Length),
    /// `box-shadow: none | <shadow>#` — non-inherited。複数 entry を保持する。
    /// `inset` and omitted colors are retained on each entry.
    BoxShadow(Arc<Vec<BoxShadowItem>>),
    /// `outline` shorthand — non-inherited。width/style/color を保持するが、
    /// outline の layout 非干渉性そのものは下流 layout の責務である。
    /// [`crate::rule::expand_shorthand_into`] が
    /// [`Self::OutlineWidth`] / [`Self::OutlineStyle`] / [`Self::OutlineColor`]
    /// に展開する。
    Outline(Outline),
    /// `outline-width: <line-width>` — non-inherited、initial: `medium`。
    OutlineWidth(Length),
    /// `outline-style: auto | <border-style>` — non-inherited、initial: `none`。
    /// `hidden` は既存 outline parser の scope 外として reject する。
    OutlineStyle(OutlineStyle),
    /// `outline-color: invert | <color>` — non-inherited、initial: `invert`
    /// (CSS UI 3 §4.4)。[`OutlineColor`] keeps `invert`, `currentcolor`, and
    /// resolved colors distinct through computed-value processing.
    OutlineColor(OutlineColor),
    /// `outline-offset: <length>` — non-inherited、initial: `0` (CSS UI 3 §4.5
    /// <https://www.w3.org/TR/css-ui-3/#outline-offset>)。負値も受理し、border edge
    /// からの offset を絶対化する。`<percentage>` は grammar 外。
    OutlineOffset(Length),
    /// `grid-area` shorthand — placement for one grid item.
    GridArea(GridAreaShorthand),
    /// `grid` shorthand — the supported explicit `rows / columns` form.
    Grid(GridShorthand),
    /// `grid-template-columns` — non-inherited、initial:
    /// [`GridTemplateTracks::None`] ([`GridTemplateTracks`] doc 参照)。
    GridTemplateColumns(GridTemplateTracks),
    /// `grid-template-rows` — non-inherited、initial:
    /// [`GridTemplateTracks::None`]。[`Self::GridTemplateColumns`] と同じ
    /// grammar/shape。
    GridTemplateRows(GridTemplateTracks),
    /// `grid-template-areas` — non-inherited、initial:
    /// [`GridTemplateAreasValue::None`] ([`GridTemplateAreasValue`] doc 参照)。
    GridTemplateAreas(GridTemplateAreasValue),
    /// `grid-auto-columns: <track-size>+` — non-inherited、initial: `auto`
    /// (単一要素 `[GridTrackSize::Breadth(GridTrackBreadth::Auto)]`、CSS
    /// Grid Layout Module Level 1 §7.6
    /// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-columns>)。`Arc`
    /// wrap は [`Self::GridTemplateColumns`] と同じ理由。
    GridAutoColumns(Arc<Vec<GridTrackSize>>),
    /// `grid-auto-rows` — non-inherited、initial: `auto`。
    /// [`Self::GridAutoColumns`] と同じ grammar/shape。
    GridAutoRows(Arc<Vec<GridTrackSize>>),
    /// `grid-auto-flow` — non-inherited、initial: [`GridAutoFlowValue::Row`]
    /// ([`GridAutoFlowValue`] doc 参照)。
    GridAutoFlow(GridAutoFlowValue),
    /// `grid-row-start` — non-inherited、initial: [`GridLineValue::Auto`]
    /// ([`GridLineValue`] doc 参照)。
    GridRowStart(GridLineValue),
    /// `grid-row-end` — non-inherited、initial: [`GridLineValue::Auto`]。
    GridRowEnd(GridLineValue),
    /// `grid-column-start` — non-inherited、initial: [`GridLineValue::Auto`]。
    GridColumnStart(GridLineValue),
    /// `grid-column-end` — non-inherited、initial: [`GridLineValue::Auto`]。
    GridColumnEnd(GridLineValue),
    /// `grid-row: <grid-line> [ / <grid-line> ]?` shorthand — non-inherited、
    /// initial: `auto` ([`GridLineShorthand`] doc 参照)。
    /// [`crate::rule::expand_shorthand_into`] が [`Self::GridRowStart`] /
    /// [`Self::GridRowEnd`] の 2 longhand に展開する。
    GridRow(GridLineShorthand),
    /// `grid-column` shorthand — non-inherited、initial: `auto`。
    /// [`crate::rule::expand_shorthand_into`] が [`Self::GridColumnStart`] /
    /// [`Self::GridColumnEnd`] の 2 longhand に展開する。
    GridColumn(GridLineShorthand),
    /// `justify-items` — non-inherited。[`SelfAlignmentValue`] を
    /// `align-items` と共有再利用する ([`Self::AlignItems`] と同じ payload
    /// 型)。
    ///
    /// # Scope carving — `legacy` は未対応
    ///
    /// CSS Box Alignment Module Level 3 §7.1
    /// (<https://www.w3.org/TR/css-align-3/#propdef-justify-items>) の spec
    /// grammar は `normal | stretch | <baseline-position> |
    /// <overflow-position>? [ <self-position> | left | right ] | legacy |
    /// legacy && [ left | right | center ]`、"Initial: `legacy`" —
    /// `<self-position>` 以外の carve-out ([`SelfAlignmentValue`] doc の
    /// scope carving 節と同じ、`<overflow-position>`/`left`/`right`) に加え、
    /// `legacy` keyword とその特殊な "effectively inherit into descendants"
    /// 継承 (spec 本文 verbatim: "if the inherited value of justify-items
    /// includes the legacy keyword, this value computes to the inherited
    /// value; otherwise it computes to normal" — HTML `<center>` element /
    /// `align` 属性の legacy alignment 実装専用機構) は未対応。taffy 0.12 の
    /// `justify_items: Option<AlignItems>` にも `legacy` 相当の表現が無い。
    ///
    /// spec の "otherwise it computes to normal" 分岐が示すとおり、`legacy`
    /// 機構が未実装の本 crate では (誰も `legacy` を継承させられないため)
    /// 実効的に常に `normal` へ収束する — この crate の initial value を
    /// spec の `legacy` ではなく [`SelfAlignmentValue::Normal`] とするのは
    /// この収束先を直接表現したもの。
    JustifyItems(SelfAlignmentValue),
    /// `justify-self` — non-inherited、initial: [`AlignSelfValue::Auto`]。
    /// [`AlignSelfValue`] を `align-self` と共有再利用する
    /// ([`Self::AlignSelf`] と同じ payload 型)。CSS Box Alignment Module
    /// Level 3 §6.1 <https://www.w3.org/TR/css-align-3/#propdef-justify-self>。
    JustifySelf(AlignSelfValue),
    /// `place-items: <'align-items'> <'justify-items'>?` shorthand —
    /// non-inherited、initial: "see individual properties"
    /// ([`PlaceItemsShorthand`] doc 参照)。
    /// [`crate::rule::expand_shorthand_into`] が [`Self::AlignItems`] /
    /// [`Self::JustifyItems`] の 2 longhand に展開する。
    PlaceItems(PlaceItemsShorthand),
    /// `place-self: <'align-self'> <'justify-self'>?` shorthand —
    /// non-inherited、initial: `auto` ([`PlaceSelfShorthand`] doc 参照)。
    /// [`crate::rule::expand_shorthand_into`] が [`Self::AlignSelf`] /
    /// [`Self::JustifySelf`] の 2 longhand に展開する。
    PlaceSelf(PlaceSelfShorthand),
    /// `orphans` — **inherited**、initial: `2` (CSS Fragmentation Module
    /// Level 3 §3.3 "Breaks Between Lines: orphans, widows"
    /// <https://www.w3.org/TR/css-break-3/#widows-orphans>。CSS 2.1
    /// §13.3.2 の原定義を supersede するが grammar は不変)。Value:
    /// `<integer>`。computed value = specified integer。
    ///
    /// spec は正の整数のみを許容する: "Only positive integers are allowed
    /// as values of orphans and widows. Negative values and zero are
    /// invalid and must cause the declaration to be ignored." — parse 時に
    /// enforce される (`parse_positive_integer` 参照) ため、この payload は
    /// 常に `> 0`。
    ///
    /// この crate が実装するのは parsing と inherited storage のみ —
    /// このプロパティが記述する pagination 時の最小行数 enforcement 自体は
    /// 未実装。
    Orphans(i32),
    /// `widows` — [`Self::Orphans`] と同じ grammar/initial/inheritance・
    /// 正数限定の制約 (CSS Fragmentation Module Level 3 §3.3、同じ propdef
    /// table)。違いは最小行数を fragmentation break のどちら側に適用するか
    /// だけ (break 後 — `orphans` は break 前)。
    Widows(i32),
    /// `writing-mode: horizontal-tb | vertical-rl | vertical-lr | sideways-rl
    /// | sideways-lr` — **inherited**、initial: [`WritingMode::HorizontalTb`]
    /// (CSS Writing Modes 4 §3.2 [`WritingMode`] doc 参照)。5 keyword とも
    /// 構文としては受理するが、computed value は常に
    /// [`WritingMode::HorizontalTb`] に正規化する ([`WritingMode`] doc の
    /// Non-goal 節、[`resolve_writing_mode`] doc 参照 — raikiri は縦書き
    /// レンダリングパイプラインを実装しない)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    WritingMode(WritingMode),
    /// `ruby-position` — inherited, initial: [`RubyPosition::Over`].
    RubyPosition(RubyPosition),
    /// `background-repeat` — **non-inherited**、initial:
    /// [`BackgroundRepeat`]`{x: Repeat, y: Repeat}` (CSS Backgrounds 3 §2.4
    /// [`BackgroundRepeat`] doc 参照)。末尾に追加 (1:1 disjoint な新 field、
    /// [`PropertyKey`] doc の判断規則)。
    BackgroundRepeat(BackgroundRepeat),
    /// `background-attachment` — **non-inherited**、initial:
    /// [`BackgroundAttachment::Scroll`] (CSS Backgrounds 3 §2.5
    /// [`BackgroundAttachment`] doc 参照)。
    BackgroundAttachment(BackgroundAttachment),
    /// `background-clip` — **non-inherited**、initial:
    /// [`VisualBox::BorderBox`] (CSS Backgrounds 3 §2.7 [`VisualBox`] doc 参照
    /// — sibling [`Self::BackgroundOrigin`] と initial が異なる点に注意)。
    BackgroundClip(VisualBox),
    /// `background-origin` — **non-inherited**、initial:
    /// [`VisualBox::PaddingBox`] (CSS Backgrounds 3 §2.8 [`VisualBox`] doc
    /// 参照 — sibling [`Self::BackgroundClip`] と initial が異なる点に注意)。
    BackgroundOrigin(VisualBox),
    /// `background-size` — **non-inherited**、initial:
    /// [`BackgroundSize::Explicit`]`{width: Auto, height: Auto}` (CSS
    /// Backgrounds 3 §2.9 [`BackgroundSize`] doc 参照)。`<length-percentage>`
    /// を含むため絶対化は phase 3 に委ねる (`padding`/`width` と同型)。
    BackgroundSize(BackgroundSize),
    /// `background-position` — **non-inherited**、initial:
    /// [`CssPosition`]`{horizontal: Start(Percent(0.0)), vertical:
    /// Start(Percent(0.0))}` (CSS Backgrounds 3 §2.6 "Initial: 0% 0%"、
    /// [`CssPosition`] doc 参照)。`<length-percentage>` を含むため絶対化は
    /// phase 3 に委ねる。
    BackgroundPosition(CssPosition),
    /// `background-image` — **non-inherited**、initial: [`BackgroundImage::None`]
    /// (CSS Backgrounds 3 §2.3 [`BackgroundImage`] doc 参照)。末尾に追加
    /// (1:1 disjoint な新 field、[`PropertyKey`] doc の判断規則)。
    BackgroundImage(BackgroundImage),
    /// `background` shorthand — non-inherited。8 成分 (color/image/repeat/
    /// attachment/position/size/clip/origin) を保持する
    /// ([`BackgroundShorthand`] doc 参照)。単一 layer のみ対応 (同 doc の
    /// Non-goal 節)。[`crate::rule::expand_shorthand_into`] が
    /// [`Self::BackgroundColor`] / [`Self::BackgroundImage`] /
    /// [`Self::BackgroundRepeat`] / [`Self::BackgroundAttachment`] /
    /// [`Self::BackgroundPosition`] / [`Self::BackgroundSize`] /
    /// [`Self::BackgroundClip`] / [`Self::BackgroundOrigin`] の 8 longhand
    /// に展開する。末尾に追加 (既存 8 longhand は既に別 field を持つため
    /// 1:1 disjoint ではないが、shorthand は cascade 段に到達しない
    /// ([`crate::rule::expand_shorthand_into`] doc) ので discriminant 順は
    /// 意味を持たない — 既存 variant を shift させない配置を優先する、
    /// [`PropertyKey`] doc の「宣言順は load-bearing」節参照)。
    Background(BackgroundShorthand),
    /// `object-fit` — **non-inherited**、initial: [`ObjectFit::Fill`] (CSS
    /// Images 3 §5.1 [`ObjectFit`] doc 参照)。末尾に追加 (1:1 disjoint な
    /// 新 field、[`PropertyKey`] doc の判断規則)。
    ObjectFit(ObjectFit),
    /// `object-position` — **non-inherited**、initial: `50% 50%` (CSS Images
    /// 3 §5.2 "Initial: 50% 50%"、[`CssPosition`] doc 参照)。
    /// `background-position` と同じ [`CssPosition`] 型を再利用する
    /// ([`CssPosition`] doc の「`background-position` / `object-position` で
    /// 使われる」節) が、grammar は同一ではない — `<bg-position>` 固有の
    /// 3-value edge-offset 構文を許さない strict な `<position>` (CSS
    /// Values 4 §8.3) を要求するため、`background-position` が使う
    /// [`parse_bg_position`] ではなく [`parse_position_strict`] で parse
    /// する ([`parse_position_branch3_strict`] doc参照)。
    /// `<length-percentage>` を含むため絶対化は phase 3 に委ねる。
    ObjectPosition(CssPosition),
    /// `opacity` — **non-inherited**、initial: `1` (CSS Color 4 §3.3
    /// "Transparency: the opacity property"
    /// <https://www.w3.org/TR/css-color-4/#transparency>, "Value:
    /// `<opacity-value>`", "Inherited: no")。grammar: `<opacity-value> =
    /// <number> | <percentage>`。
    ///
    /// この payload は **specified value をそのまま保持し、clamp しない**
    /// — 同 § 本文: "Opacity values outside the range \[0, 1\] are not
    /// invalid, and are preserved in specified values, but are clamped to
    /// the range \[0, 1\] in computed values."。clamp は phase 3
    /// ([`crate::specified::SpecifiedValues::absolutize_with`] /
    /// [`crate::page`] の `absolutize_in_page_context`) の仕事であり、この
    /// variant 自体は範囲外の値 (例: `opacity: 2`) をそのまま運ぶ。末尾に
    /// 追加 (1:1 disjoint な新 field、[`PropertyKey`] doc の判断規則)。
    Opacity(f32),
    /// `isolation` — **non-inherited**、initial: [`Isolation::Auto`] (CSS
    /// Compositing and Blending Level 1 §3.4.2 [`Isolation`] doc 参照)。
    /// 末尾に追加 (1:1 disjoint な新 field、[`PropertyKey`] doc の判断規則)。
    Isolation(Isolation),
    /// `mix-blend-mode` — **non-inherited**、initial:
    /// [`MixBlendMode::Normal`] (CSS Compositing and Blending Level 1
    /// §3.4.1 [`MixBlendMode`] doc 参照)。末尾に追加 (1:1 disjoint な新
    /// field、[`PropertyKey`] doc の判断規則)。
    MixBlendMode(MixBlendMode),
    /// `mask-image` — **non-inherited**、initial: [`MaskImage::None`] (CSS
    /// Masking Level 1 §7.1 [`MaskImage`] doc 参照)。末尾に追加 (1:1
    /// disjoint な新 field、[`PropertyKey`] doc の判断規則)。
    MaskImage(MaskImage),
    /// `clip-path` — **non-inherited**、initial: [`ClipPath::None`] (CSS
    /// Masking Level 1 §5.1 [`ClipPath`] doc 参照)。末尾に追加 (1:1
    /// disjoint な新 field、[`PropertyKey`] doc の判断規則)。
    ClipPath(ClipPath),
    /// `transform` — **non-inherited**、initial: `none` (CSS Transforms
    /// Level 1 §4 [`TransformFunction`] doc 参照)。`none` は空 list
    /// ([`empty_transform_list`]) で表現する (`BoxShadow` の `none` = 空
    /// `Vec` と同じ convention)。末尾に追加 (1:1 disjoint な新 field、
    /// [`PropertyKey`] doc の判断規則)。
    Transform(Arc<Vec<TransformFunction>>),
    /// `filter` — **non-inherited**、initial: `none` (CSS Filter Effects
    /// Level 1 §5 [`FilterFunction`] doc 参照)。`Transform` と同じ
    /// 空-list-means-none convention ([`empty_filter_list`])。末尾に追加
    /// (1:1 disjoint な新 field、[`PropertyKey`] doc の判断規則)。
    Filter(Arc<Vec<FilterFunction>>),
    /// `table-layout: auto | fixed` — **non-inherited**、initial:
    /// [`TableLayoutValue::Auto`] (CSS Tables 3 §4 [`TableLayoutValue`] doc
    /// 参照)。computed value = specified keyword (length を運ばないため
    /// 相対解決なし)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    TableLayout(TableLayoutValue),
    /// `border-collapse: collapse | separate` — **inherited**、initial:
    /// [`BorderCollapseValue::Separate`] (CSS Tables 3 §6
    /// [`BorderCollapseValue`] doc 参照)。computed value = specified
    /// keyword (length を運ばないため相対解決なし)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    BorderCollapse(BorderCollapseValue),
    /// `border-spacing: <length>{1,2}` — **inherited**、initial: `0`
    /// (両軸 `0px`、CSS Tables 3 §6.1 [`BorderSpacingValue`] doc 参照)。
    /// computed value = two absolute lengths のため phase 3 で
    /// [`crate::resolve::resolve_border_spacing`] が絶対化する
    /// ([`Self::TabSize`] と同じ length-bearing staging 形)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    BorderSpacing(BorderSpacingValue),
    /// `caption-side: top | bottom` — **inherited**、initial:
    /// [`CaptionSideValue::Top`] (CSS Tables 3 §7 [`CaptionSideValue`] doc
    /// 参照)。computed value = specified keyword (length を運ばないため
    /// 相対解決なし)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    CaptionSide(CaptionSideValue),
    /// `empty-cells: show | hide` — **inherited**、initial:
    /// [`EmptyCellsValue::Show`] (CSS Tables 3 §8 [`EmptyCellsValue`] doc
    /// 参照)。computed value = specified keyword (length を運ばないため
    /// 相対解決なし)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    EmptyCells(EmptyCellsValue),
    /// `font` shorthand — **inherited**。6 成分 (style/variant-caps/weight/
    /// size/line-height/family) を保持する ([`FontShorthand`] doc 参照)。
    /// CSS Fonts 4 §2.1
    /// <https://www.w3.org/TR/css-fonts-4/#font-prop> の
    /// `[ <'font-style'> || <font-variant-css2> || <'font-weight'> ]? <'font-size'> [ / <'line-height'> ]? <'font-family'>#`
    /// subset (system-font keyword・`font-stretch` 非 `normal`・CSS2 外の
    /// `font-variant` は scope 外、[`FontShorthand`] doc の Scope carving 節
    /// 参照)。[`crate::rule::expand_shorthand_into`] が
    /// [`Self::FontStyle`] / [`Self::FontVariantCaps`] /
    /// [`Self::FontWeight`] / [`Self::FontSize`]・[`Self::FontSizeRelative`] /
    /// [`Self::LineHeight`] / [`Self::FontFamily`] の 6 longhand に展開する。
    /// 末尾に追加 (shorthand は cascade 段に到達しない
    /// ([`crate::rule::expand_shorthand_into`] doc) ので discriminant 順は
    /// 意味を持たない — 既存 variant を shift させない配置を優先する、
    /// [`PropertyKey`] doc の「宣言順は load-bearing」節参照)。
    Font(FontShorthand),
    /// `text-decoration-skip-ink` — **inherited**、initial:
    /// [`TextDecorationSkipInk::Auto`] ([`TextDecorationSkipInk`] doc 参照)。
    /// parsing-only: cascade は winner を staging field に載せず drop する
    /// (E/F/G fa04ac3 の `TextCombineUpright` 等と同 pattern —
    /// [`crate::cascade::apply_value`] の同名 no-op arm 参照)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    TextDecorationSkipInk(TextDecorationSkipInk),
    /// `text-decoration-skip-spaces` — **inherited**、initial は `start end`
    /// ([`TextDecorationSkipSpaces::StartEnd`])。parsing-only (同上)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    TextDecorationSkipSpaces(TextDecorationSkipSpaces),
    /// `text-decoration-thickness` — **non-inherited**、initial:
    /// [`TextDecorationThickness::Auto`] ([`TextDecorationThickness`] doc
    /// 参照)。parsing-only (同上 — `<length-percentage>` を運ぶが phase 3
    /// の絶対化対象にはしない。`@page` 経路の
    /// [`crate::page`] の `absolutize_in_page_context` の同名 arm だけが
    /// length 成分を absolutize する)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    TextDecorationThickness(TextDecorationThickness),
    /// `text-decoration-inset` — **non-inherited**、initial: `0`
    /// (ED)。element 経路は [`crate::specified::SpecifiedValues`] へ staging
    /// し、[`crate::resolve::resolve_text_decoration_inset`] で declaring
    /// node の font metrics に対して絶対化して paint へ渡す。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    TextDecorationInset(TextDecorationInset),
    /// `text-emphasis-position` — **inherited**、initial: `over right`
    /// (ED)。parsing-only (同上)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    TextEmphasisPosition(TextEmphasisPosition),
    /// `text-underline-position` — **inherited**、initial:
    /// [`TextUnderlinePosition::AUTO`]。parsing-only (同上)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
    TextUnderlinePosition(TextUnderlinePosition),
    /// `page: auto | <custom-ident>` — **non-inherited**、initial:
    /// [`PageValue::Auto`] (CSS Paged Media 3 §8.1 [`PageValue`] doc 参照)。
    /// parsing-only: cascade は winner を staging field に載せず drop する
    /// (E/F/G の `TextCombineUpright` 等と同 pattern)。
    /// (末尾に追加 — 配置理由は [`Self::TableLayout`] と同じ)
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
    TextUnderlineOffset(LengthOrAuto),
    /// `text-autospace` — inherited, initial: `normal` (CSS Text 4).
    TextAutospace(TextAutospace),
}

/// Property key (cascade で "同一 property を勝ち取る" ための discriminant)。
///
/// cascade.rs の per-node winner selection、および page.rs の
/// [`cascade_page`](crate::page::cascade_page) が [`PageCascadeResult`] の
/// map key に使う。`PropertyValue` の variant tag を stateless に抜き出したもので
/// 追加情報を持たないため public に露出する ([`PageCascadeResult`]
/// が `pub` 型を要求するため — clippy `private_interfaces` 対応)。
///
/// # ⚠️ variant の**宣言順は load-bearing**
///
/// element cascade は本 enum の discriminant (`key as usize`) を scratch buffer
/// の slot index に使い、**slot を index 昇順に走査して winner を適用する**
/// ([`mod@crate::cascade`] の `apply_winners`)。したがって:
///
/// - **variant を追加する位置**と**既存 variant の並び順**が、同一 node で
///   複数の winner が同じ [`crate::specified::SpecifiedValues`] field に書く
///   場合の**最終値を変えうる**。
/// - 現状これが効きうるのは shorthand key (`Padding` / `Margin` / `Border`)
///   だけで、いずれも longhand より後ろに置かれている。ただし
///   [`crate::rule::expand_shorthand_into`] が parse 出口と element cascade 入口の
///   両方で shorthand を longhand に展開するため、**shorthand key は element
///   cascade 段には到達しない**。`@page` cascade
///   ([`crate::page::cascade_page`]) も入口側で同じ展開を通すので、`PageRule` の
///   `pub declarations` を post-parse mutation された場合の同 shape の gap も
///   塞がっている。
///
/// **並び順を「直す」ことで shorthand/longhand の cascade を修正しようとしない
/// こと** — 順序任せの解は `margin: 0; margin-top: 10px` と
/// `margin-top: 10px; margin: 0` という鏡像 2 例のうち必ず片方を壊す
/// (詳細は `apply_winners` の doc)。正しい解は既に採られている
/// 「shorthand を cascade 段に到達させない」方向であり、その展開 arm の
/// 書き忘れは [`crate::rule::expand_shorthand_into`] の exhaustive match により
/// compile-time に排除されている。
///
/// 新しい variant を足すときは、それが既存 variant と同じ `SpecifiedValues`
/// field に書くかどうかを確認すること。書かないなら (= 1:1 disjoint なら)
/// 位置は自由でよい。
///
/// [`Direction`] / [`TextAlign`] は 1:1 disjoint (`SpecifiedValues::direction`
/// / `SpecifiedValues::text_align` の別 field) — `text-align: match-parent`
/// が `direction` の**親**の computed 値を要する件は
/// この `PropertyKey` の並び順とは**無関係**。その解決は
/// [`crate::property::resolve_text_align_match_parent`] の呼び手
/// ([`crate::specified::SpecifiedValues::finalize`]) が全 winner 適用後に
/// 明示的な親 [`crate::computed::ComputedValues`] を受け取って行うため、
/// `apply_winners` の slot 走査順 (= 本 enum の宣言順) には触れない。
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
    /// [`PropertyValue::Padding`] doc の "shorthand vs longhand cascade" 制約に
    /// 該当する discriminant — shorthand と longhand それぞれ独立 winner が
    /// pick される。CSS Cascading L4 §3
    /// <https://www.w3.org/TR/css-cascade-4/#shorthand> の
    /// "exactly as if expanded in place" は本来 shorthand を longhand の
    /// syntactic sugar として畳むことを意味するので、独立 winner を持つこと自体は
    /// deviation。[`crate::rule::expand_shorthand_into`] が parse 出口と element
    /// cascade 入口の両方で shorthand を畳むため本 variant は element cascade 段に
    /// 到達せず、observable な divergence は無い
    /// (`@page` 経路の同 shape gap も [`crate::page::cascade_page`] の入口側展開で
    /// 塞がれている)。その担保のうち「展開 arm の
    /// 書き忘れ」は [`crate::rule::expand_shorthand_into`] の exhaustive match により
    /// compile-time に排除されている。
    Padding,
    // padding-inline / padding-block logical 2-value shorthand (CSS Logical
    // Properties and Values 1 §4.4, semantics on the matching
    // PropertyValue::PaddingInline / PropertyValue::PaddingBlock variants).
    // Placed after `Padding` for the same "shorthand key comes after the
    // longhands it can compete with" convention (`PropertyKey` doc's
    // "宣言順は load-bearing" section) — both expand into a subset of the
    // same 4 padding longhands `Padding` does.
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
    // `border-style` / `border-width` / `border-color` shorthand keys
    // (semantics on the matching PropertyValue variants above).
    BorderStyle,
    BorderWidth,
    BorderColor,
    // width (CSS Sizing 3 §3.1.1)。
    Width,
    // height (CSS Sizing 3 §3.1.1、semantics on the
    // matching PropertyValue::Height variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention).
    Height,
    MaxWidth,
    MaxHeight,
    MinWidth,
    MinHeight,
    // box-sizing (CSS Sizing 3 §3.3、semantics on the
    // matching PropertyValue::BoxSizing variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention).
    BoxSizing,
    // direction (CSS Writing Modes 4 §2.1、semantics on
    // the matching PropertyValue::Direction variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は PropertyValue::Direction の doc 参照。
    Direction,
    // overflow-x / overflow-y longhand + overflow shorthand
    // (CSS Overflow 3 §3.1、semantics on the matching PropertyValue::Overflow*
    // variants; sibling PropertyKey variants carry no per-variant docs per
    // crate convention). 末尾配置の理由は PropertyValue::OverflowX の doc 参照。
    OverflowX,
    OverflowY,
    Overflow,
    // text-decoration-line / -style / -color longhand + text-decoration
    // shorthand (CSS Text Decoration Module Level 3 §2.1-§2.4、semantics on
    // the matching PropertyValue::TextDecoration* variants; sibling
    // PropertyKey variants carry no per-variant docs per crate convention).
    // 末尾配置の理由は PropertyValue::TextDecorationLine の doc 参照。
    TextDecorationLine,
    TextDecorationStyle,
    TextDecorationColor,
    TextDecoration,
    // vertical-align (CSS 2.1 §10.8.1、semantics on the matching
    // PropertyValue::VerticalAlign variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::TextDecoration の doc と同じ (1:1 disjoint な新 field)。
    VerticalAlign,
    // font-style (CSS Fonts 4 §2.4、semantics on the matching
    // PropertyValue::FontStyle variant; sibling PropertyKey variants carry
    // no per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::FontStyle の doc 参照。
    FontStyle,
    // text-transform (CSS Text Module Level 3 §2.1、semantics on the
    // matching PropertyValue::TextTransform variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は PropertyValue::TextTransform の doc 参照。
    TextTransform,
    // visibility (CSS Display 3 §4、semantics on the matching
    // PropertyValue::Visibility variant; sibling PropertyKey variants carry
    // no per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::Visibility の doc 参照。
    Visibility,
    // z-index (CSS2 §9.9.1、semantics on the matching PropertyValue::ZIndex
    // variant; sibling PropertyKey variants carry no per-variant docs per
    // crate convention). 末尾配置の理由は PropertyValue::FontStyle の doc 参照。
    ZIndex,
    // word-break (CSS Text 3 §5.1、semantics on the matching
    // PropertyValue::WordBreak variant; sibling PropertyKey variants carry
    // no per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::WordBreak の doc 参照。
    WordBreak,
    // overflow-wrap / legacy alias word-wrap (CSS Text 3 §5.4、semantics on
    // the matching PropertyValue::OverflowWrap variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は PropertyValue::OverflowWrap の doc 参照。
    OverflowWrap,
    // letter-spacing / word-spacing (CSS Text 3 §7.2 / §7.1、semantics on
    // the matching PropertyValue::LetterSpacing / PropertyValue::WordSpacing
    // variants; sibling PropertyKey variants carry no per-variant docs per
    // crate convention). 末尾配置の理由は PropertyValue::FontStyle の doc
    // 参照。
    LetterSpacing,
    WordSpacing,
    // break-before / break-after / break-inside + legacy shorthand
    // page-break-* (CSS Fragmentation Module Level 3 §3.1 / §3.2 / §3.4、
    // semantics on the matching PropertyValue::BreakBefore /
    // PropertyValue::BreakAfter / PropertyValue::BreakInside variants;
    // sibling PropertyKey variants carry no per-variant docs per crate
    // convention). 末尾配置の理由は PropertyValue::BreakBefore の doc 参照。
    BreakBefore,
    BreakAfter,
    BreakInside,
    // float (CSS2 §9.5.1、semantics on the matching PropertyValue::Float
    // variant; sibling PropertyKey variants carry no per-variant docs per
    // crate convention). 末尾配置の理由は PropertyValue::Float の doc 参照。
    Float,
    // clear (CSS2 §9.5.2、semantics on the matching PropertyValue::Clear
    // variant; sibling PropertyKey variants carry no per-variant docs per
    // crate convention). 末尾配置の理由は PropertyValue::Clear の doc 参照。
    Clear,
    // white-space (CSS Text 3 §3、semantics on the matching
    // PropertyValue::WhiteSpace variant; sibling PropertyKey variants carry
    // no per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::WhiteSpace の doc 参照。
    WhiteSpace,
    // text-wrap (CSS Text 4 §5、semantics on PropertyValue::TextWrap).
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
    // tab-size (CSS Text Module Level 3 §4.2、semantics on the matching
    // PropertyValue::TabSize variant; sibling PropertyKey variants carry no
    // per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::TabSize の doc 参照。
    TabSize,
    // font-variant-caps (CSS Fonts Module Level 3 §6.6, semantics on the
    // matching PropertyValue::FontVariantCaps variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は PropertyValue::FontVariantCaps の doc 参照。
    FontVariantCaps,
    // quotes (CSS Content Module Level 3 §2.4.1, semantics on the matching
    // PropertyValue::Quotes variant; sibling PropertyKey variants carry no
    // per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::Quotes の doc 参照 (1:1 disjoint な新 field)。
    Quotes,
    // text-shadow (CSS Text Decoration Module Level 3 §4, semantics on
    // the matching PropertyValue::TextShadow variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は PropertyValue::TextShadow の doc 参照。
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
    // `JustifyItems`) declared above (AlignItems 側は既存 flex/alignment
    // 節)。`place-self` shorthand (§6.3) — both longhands
    // (`AlignSelf`/`JustifySelf`) declared above.
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
    // writing-mode (CSS Writing Modes 4 §3.2、semantics on the matching
    // PropertyValue::WritingMode variant; sibling PropertyKey variants carry
    // no per-variant docs per crate convention). 末尾配置の理由は
    // PropertyValue::WritingMode の doc 参照。
    WritingMode,
    /// `ruby-position` inherited annotation placement.
    RubyPosition,
    // background-repeat / background-attachment / background-clip /
    // background-origin / background-size / background-position (CSS
    // Backgrounds and Borders 3 §2.4-§2.9、semantics on the matching
    // PropertyValue::Background* variants; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // WritingMode 直前の同節参照 (1:1 disjoint な新 field)。
    BackgroundRepeat,
    BackgroundAttachment,
    BackgroundClip,
    BackgroundOrigin,
    BackgroundSize,
    BackgroundPosition,
    // background-image (CSS Backgrounds and Borders 3 §2.3、semantics on the
    // matching PropertyValue::BackgroundImage variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は直前の background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    BackgroundImage,
    // `background` shorthand (CSS Backgrounds and Borders 3 §2.10、semantics
    // on the matching PropertyValue::Background variant). 末尾配置の理由は
    // PropertyValue::Background の doc 参照 — shorthand は cascade 段に
    // 到達しないため discriminant 順は意味を持たない。
    Background,
    // object-fit / object-position (CSS Images Module Level 3 §5.1/§5.2、
    // semantics on the matching PropertyValue::ObjectFit /
    // PropertyValue::ObjectPosition variants; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    ObjectFit,
    ObjectPosition,
    // opacity (CSS Color 4 §3.3、semantics on the matching
    // PropertyValue::Opacity variant; sibling PropertyKey variants carry no
    // per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    Opacity,
    // isolation / mix-blend-mode (CSS Compositing and Blending Level 1
    // §3.4.1/§3.4.2、semantics on the matching PropertyValue::Isolation /
    // PropertyValue::MixBlendMode variants; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    Isolation,
    MixBlendMode,
    // mask-image / clip-path (CSS Masking Level 1 §7.1/§5.1、semantics on
    // the matching PropertyValue::MaskImage / PropertyValue::ClipPath
    // variants; sibling PropertyKey variants carry no per-variant docs per
    // crate convention). 末尾配置の理由は background-repeat 等と同節参照
    // (1:1 disjoint な新 field)。
    MaskImage,
    ClipPath,
    // transform / filter (CSS Transforms Level 1 §4、CSS Filter Effects
    // Level 1 §5、semantics on the matching PropertyValue::Transform /
    // PropertyValue::Filter variants; sibling PropertyKey variants carry
    // no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    Transform,
    Filter,
    // line-break (CSS Text 3 §5.2、semantics on PropertyValue::LineBreak).
    LineBreak,
    // text-justify (CSS Text 3 §6.2、semantics on PropertyValue::TextJustify).
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
    // table-layout (CSS Tables 3 §4、semantics on the matching
    // PropertyValue::TableLayout variant; sibling PropertyKey variants carry
    // no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    TableLayout,
    // border-collapse (CSS Tables 3 §6、semantics on the matching
    // PropertyValue::BorderCollapse variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    BorderCollapse,
    // border-spacing (CSS Tables 3 §6.1、semantics on the matching
    // PropertyValue::BorderSpacing variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    BorderSpacing,
    // caption-side (CSS Tables 3 §7、semantics on the matching
    // PropertyValue::CaptionSide variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    CaptionSide,
    // empty-cells (CSS Tables 3 §8、semantics on the matching
    // PropertyValue::EmptyCells variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (1:1 disjoint な新 field)。
    EmptyCells,
    // font shorthand (CSS Fonts 4 §2.1、semantics on the matching
    // PropertyValue::Font variant; sibling PropertyKey variants carry no
    // per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 — shorthand は cascade 段に到達しない
    // (`crate::rule::expand_shorthand_into` が展開する) ため discriminant
    // 順は意味を持たない。
    Font,
    // text-decoration-skip-ink (ED §2.10.4、semantics on the matching
    // PropertyValue::TextDecorationSkipInk variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は background-repeat 等と同節参照 (1:1 disjoint な新 field…
    // parsing-only のため field 自体は無いが discriminant 順は同様に自由)。
    TextDecorationSkipInk,
    // text-decoration-skip-spaces (ED §2.10.3、同上)。
    TextDecorationSkipSpaces,
    // text-decoration-thickness (ED §2.4.1、TR §2.4、同上)。
    TextDecorationThickness,
    // text-decoration-inset (ED §2.9.1、同上)。
    TextDecorationInset,
    // text-emphasis-position (ED §3.4、同上)。
    TextEmphasisPosition,
    // text-underline-position (ED §2.7、同上)。
    TextUnderlinePosition,
    // page (CSS Paged Media 3 §8.1、semantics on the matching
    // PropertyValue::Page variant; sibling PropertyKey variants carry no
    // per-variant docs per crate convention). 末尾配置の理由は
    // background-repeat 等と同節参照 (staging field なしの parsing-only
    // だが discriminant 順は同様に自由)。
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
}

impl PropertyValue {
    /// この value が属する property key を返す。
    ///
    /// cascade winner selection で "同一 property を勝ち取る" ための discriminant として、
    /// また `@page` cascade 結果 map の key として使う。
    pub fn key(&self) -> PropertyKey {
        match self {
            PropertyValue::CustomProperty(_) => PropertyKey::Custom,
            PropertyValue::Deferred(value) => value.key,
            PropertyValue::Grid(_) => PropertyKey::Grid,
            PropertyValue::GridArea(_) => PropertyKey::GridArea,
            PropertyValue::Color(_) => PropertyKey::Color,
            PropertyValue::BackgroundColor(_) => PropertyKey::BackgroundColor,
            PropertyValue::FontFamily(_) => PropertyKey::FontFamily,
            PropertyValue::FontSize(_) => PropertyKey::FontSize,
            // `larger` / `smaller` は `font-size` と同じ property — 同じ
            // `PropertyKey` に落とすことで cascade winner selection が
            // `font-size: 12px` と `font-size: larger` を正しく競合させる
            // (別 key にすると spec 上ありえない「両方勝つ」が起きる)。
            PropertyValue::FontSizeRelative(_) => PropertyKey::FontSize,
            PropertyValue::FontWeight(_) => PropertyKey::FontWeight,
            PropertyValue::LineHeight(_) => PropertyKey::LineHeight,
            PropertyValue::Display(_) => PropertyKey::Display,
            PropertyValue::ListStyleType(_) => PropertyKey::ListStyleType,
            PropertyValue::ListStylePosition(_) => PropertyKey::ListStylePosition,
            PropertyValue::ListStyleImage(_) => PropertyKey::ListStyleImage,
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
            PropertyValue::BorderTopWidth(_) => PropertyKey::BorderTopWidth,
            PropertyValue::BorderRightWidth(_) => PropertyKey::BorderRightWidth,
            PropertyValue::BorderBottomWidth(_) => PropertyKey::BorderBottomWidth,
            PropertyValue::BorderLeftWidth(_) => PropertyKey::BorderLeftWidth,
            PropertyValue::BorderTopStyle(_) => PropertyKey::BorderTopStyle,
            PropertyValue::BorderRightStyle(_) => PropertyKey::BorderRightStyle,
            PropertyValue::BorderBottomStyle(_) => PropertyKey::BorderBottomStyle,
            PropertyValue::BorderLeftStyle(_) => PropertyKey::BorderLeftStyle,
            PropertyValue::BorderTopColor(_) => PropertyKey::BorderTopColor,
            PropertyValue::BorderRightColor(_) => PropertyKey::BorderRightColor,
            PropertyValue::BorderBottomColor(_) => PropertyKey::BorderBottomColor,
            PropertyValue::BorderLeftColor(_) => PropertyKey::BorderLeftColor,
            PropertyValue::Border(_) => PropertyKey::Border,
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
            PropertyValue::TextWrap(_) => PropertyKey::TextWrap,
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
            PropertyValue::Filter(_) => PropertyKey::Filter,
            PropertyValue::TableLayout(_) => PropertyKey::TableLayout,
            PropertyValue::BorderCollapse(_) => PropertyKey::BorderCollapse,
            PropertyValue::BorderSpacing(_) => PropertyKey::BorderSpacing,
            PropertyValue::CaptionSide(_) => PropertyKey::CaptionSide,
            PropertyValue::EmptyCells(_) => PropertyKey::EmptyCells,
            PropertyValue::Font(_) => PropertyKey::Font,
            PropertyValue::TextDecorationSkipInk(_) => PropertyKey::TextDecorationSkipInk,
            PropertyValue::TextDecorationSkipSpaces(_) => PropertyKey::TextDecorationSkipSpaces,
            PropertyValue::TextDecorationThickness(_) => PropertyKey::TextDecorationThickness,
            PropertyValue::TextDecorationInset(_) => PropertyKey::TextDecorationInset,
            PropertyValue::TextEmphasisPosition(_) => PropertyKey::TextEmphasisPosition,
            PropertyValue::TextUnderlinePosition(_) => PropertyKey::TextUnderlinePosition,
            PropertyValue::Page(_) => PropertyKey::Page,
            PropertyValue::ColumnCount(_) => PropertyKey::ColumnCount,
            PropertyValue::ColumnWidth(_) => PropertyKey::ColumnWidth,
            PropertyValue::Columns(_) => PropertyKey::Columns,
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

// CSS Color 5's `<color>` endpoint grammar is recursive because it includes
// `<color-mix()>`. Bound this parser's recursive descent to keep untrusted
// declarations from exhausting the native stack.
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
            | "ch"
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
            | "ic"
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
        let dummy = replace_math_with_dummy_and_number(value, dummy_value);
        let mut input = ParserInput::new(&dummy);
        let mut parser = Parser::new(&mut input);
        // Avoid recursion into deferred path: dummy contains no deferred
        // function, so parse_value goes to normal dispatch.
        if parse_value(prop, &mut parser).is_some() && parser.expect_exhausted().is_ok() {
            return true;
        }
    }
    false
}

/// [`replace_math_with_dummy_and_number`] generalized over the dummy payload.
fn replace_math_with_dummy_and_number(input: &str, dummy: &str) -> String {
    let mut result = String::new();
    let mut i = 0;
    let lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    while i < bytes.len() {
        let mut matched = None;
        for func in MATH_FUNCTIONS.iter() {
            if lower[i..].starts_with(func) {
                let after = i + func.len();
                if after < bytes.len() && bytes[after] == b'(' {
                    matched = Some(*func);
                    break;
                }
            }
        }
        if let Some(func) = matched {
            let mut depth = 0;
            let mut j = i + func.len();
            let mut found_end = None;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            found_end = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if let Some(end) = found_end {
                result.push_str(dummy);
                i = end + 1;
                continue;
            } else {
                result.push_str(&input[i..]);
                break;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

pub(crate) fn contains_deferred_function(input: &mut Parser<'_, '_>) -> bool {
    let start = input.state();
    let source_start = input.position();
    let found = parser_contains_deferred_function(input, source_start, 0);
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
    parser_contains_deferred_function(&mut parser, source_start, 0)
}

/// Inspect CSS component-value tokens, including nested blocks, so a deferred
/// function is recognized only when cssparser emitted a real `Function` token.
/// Raw substring matching would mistake `#var(--x)` for a variable function.
fn parser_contains_deferred_function(
    input: &mut Parser<'_, '_>,
    source_start: SourcePosition,
    depth: usize,
) -> bool {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return true;
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
                if input
                    .parse_nested_block(|nested| {
                        Ok::<_, ParseError<'_, ()>>(parser_contains_deferred_function(
                            nested,
                            source_start,
                            depth.saturating_add(1),
                        ))
                    })
                    .unwrap_or(false)
                {
                    found = true;
                }
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                if input
                    .parse_nested_block(|nested| {
                        Ok::<_, ParseError<'_, ()>>(parser_contains_deferred_function(
                            nested,
                            source_start,
                            depth.saturating_add(1),
                        ))
                    })
                    .unwrap_or(false)
                {
                    found = true;
                }
            }
            _ => {}
        }
    }
    found
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
fn css_component_values_are_bounded(input: &str) -> bool {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    css_component_values_are_bounded_in_parser(&mut parser, 0)
}

fn css_component_values_are_bounded_in_parser(input: &mut Parser<'_, '_>, depth: usize) -> bool {
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
        let token_start = input.position();
        match input.next() {
            Ok(Token::Delim('!')) => {
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
                    if input.slice(start..input.position()).len() > MAX_SUBSTITUTED_VALUE_BYTES {
                        input.reset(&start_state);
                        return None;
                    }
                    let value = input.slice(start..token_start).trim();
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
                let value = input.slice(start..input.position()).trim();
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
        "text-indent" => PropertyKey::TextIndent,
        "padding-top" => PropertyKey::PaddingTop,
        "padding-right" => PropertyKey::PaddingRight,
        "padding-bottom" => PropertyKey::PaddingBottom,
        "padding-left" => PropertyKey::PaddingLeft,
        "padding" => PropertyKey::Padding,
        // CSS Logical Properties and Values 1 §4.4 padding-inline-start/-end /
        // padding-block-start/-end — physically fixed-mapped onto the
        // matching padding-{left,right,top,bottom} key (`PropertyValue::PaddingInline`
        // doc's "なぜ 8 longhand が専用 variant を持たないか" section).
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
        "border" => PropertyKey::Border,
        "border-style" => PropertyKey::BorderStyle,
        "border-width" => PropertyKey::BorderWidth,
        "border-color" => PropertyKey::BorderColor,
        "width" | "inline-size" => PropertyKey::Width,
        "height" | "block-size" => PropertyKey::Height,
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
        "text-wrap" => PropertyKey::TextWrap,
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
        "column-width" => PropertyKey::ColumnWidth,
        "columns" => PropertyKey::Columns,
        "place-content" => PropertyKey::PlaceContent,
        "hyphens" => PropertyKey::Hyphens,
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
        "filter" => PropertyKey::Filter,
        _ => return None,
    })
}
