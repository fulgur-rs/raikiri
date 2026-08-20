//! CSS property value 型と per-property parser。
//!
//! 現サポート property の canonical 一覧は `parse_value` の match arm を参照
//! (該 arm を single source of truth として扱う)。認識できない property name /
//! invalid value は `parse_value` が `None` を返す (spec 準拠の silent drop、
//! caller である rule.rs で declaration ごと drop)。
//!
//! `parse_value` は rule.rs の `DeclParser::parse_value` から呼ばれる。

use std::sync::{Arc, OnceLock};

use cssparser::color::{clamp_unit_f32, parse_named_color};
use cssparser::{ParseError, Parser, Token};
use smol_str::SmolStr;

use crate::Atom;

/// 空 `<content-list>` を表す shared Arc — cascade で全 node が持ちうる
/// initial / inherit_from の default 値を per-node 新規 allocate せず、
/// 単一 heap slot を bump-share するための helper。
///
/// cascade memory DoS 対策として `ComputedValues.content` / `.string_set` は
/// `Arc<Vec<..>>` に wrap したが、`Arc::new(Vec::new())` を every node で呼ぶと
/// N-node document あたり 2N の small heap allocation regression になる。
/// `OnceLock` で **process 全体で 1 個** の empty Arc を保持し、
/// [`empty_content_list`] / [`empty_string_set_entries`] が各 initial spot で
/// clone (Arc bump only) する。
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
/// convention (37n): [`Length::Px`] が `Px(16.0)` = `16px` の pattern を確立、
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
/// **本節が「型は層を表明しない」規則の canonical な記述である。**
/// 一方、page 経路が具体的に何を保証するか (どの値が computed 層に居るのか、
/// 例外は何か) は
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// の doc が canonical であり、その内容は `page::tests` の
/// `page_declarations_carry_no_specified_layer_residue` (以前の名前は
/// `page_declarations_carry_exactly_one_specified_layer_residue`)
/// が機械的に pin している。**ここに保証の中身を書き足して重複させないこと**
/// — 手で 2 site を揃える運用は既に 2 度 drift した。
///
/// Downstream match は必ず wildcard arm を持つこと (`#[non_exhaustive]` 属性、
/// 変数追加が既存 pattern-match を break しない forward-compat 契約)。
///
/// **訂正**: 本節は以前 `crates/raikiri-dom/src/
/// layout.rs:143` の `preshape_text` を「wildcard arm を持つ既存 sibling」と
/// して挙げていたが、これは Option A 層分離
/// (`crate::resolve` 参照) で崩れた — `preshape_text` が消費する
/// `cv.font_size` は現在 [`crate::resolve::ComputedLength`] (px scalar) で
/// あり、`Length` を直接 match しないため wildcard arm ごと削除済
/// (`layout.rs` の `preshape_text` doc "失敗しない" 節に経緯あり)。
/// 実際 `crates/raikiri-dom` / `raikiri-paint` / `raikiri-html` /
/// `raikiri-traits` は現状どこも `Length` を直接 match しない — 層分離後は
/// すべて `crate::resolve` の `Computed*` 型 (`ComputedLength` /
/// `ComputedLengthPercentage` / `ComputedLengthPercentageOrAuto` /
/// `ComputedBorder`) を経由するため。上記の wildcard-arm 契約は
/// **`Length` を直接 match する将来の downstream code に対して有効**であり、
/// 現時点でこの契約を exercise している既存 site は無い (`crates/` 全体を
/// 再 grep して確認済み)。
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
    /// `writing-mode` / `text-orientation` を未実装 (horizontal-tb 前提のみ)
    /// なので upright 分岐は到達不能 — resolve は常に `0.5 * font-size`。
    /// `writing-mode` 実装時に本判断の見直しが要る。
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
    /// cleanroom 方針) ため、resolve 側は「解決不能 → 消費 property の
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
/// non-breaking 追加のため — 37n sibling [`Length`] / [`CounterStyle`] と同じ
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

/// `border-style` の value — spec `<line-style>` production の 10 keyword。
///
/// CSS Backgrounds 3 §3.2 "Line Patterns: the border-style properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>:
/// `<line-style> = none | hidden | dotted | dashed | solid | double | groove |
/// ridge | inset | outset`。initial value は `none`、not inherited (§3.2)。
///
/// UA stylesheet 差はあるが本 crate は cleanroom scope (`walls.md` §1) のため
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
/// `.default()` が call される" (37n sibling [`DisplayValue`] / [`TextAlign`] と
/// 同じ、spec default は初期化側 [`crate::computed::ComputedValues::initial`]
/// が [`BorderStyle::None`] を直接指定する)。
///
/// `#[non_exhaustive]` は future variant (Draft CSS Backgrounds 4 拡張、または
/// author-defined `border-image` 相当の new line style) の non-breaking 追加のため —
/// 37n sibling [`DisplayValue`] / [`TextAlign`] / [`Length`] と同 pattern。
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
/// 事前 baked-in) は post-cascade resolution pass + sentinel 判別を要求し、
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
/// の forward-compat 契約 (37n sibling convention)。
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
/// per-side gradient support) の non-breaking 追加のため — 37n sibling
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
/// 37n sibling [`Length`] / [`LengthOrAuto`] と同じ制約。
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

/// `#[non_exhaustive]` は crate 外からの struct-literal 構築を `E0639` で塞ぐ
/// (umbrella (`raikiri` crate) へ [`Border`]
/// 自体を re-export した際に踏んだ制約 — その時点では埋め合わせの
/// constructor が無く、型は名指しできても値を得る public な経路が無かった)。
///
/// `raikiri-traits::page::PageBox` / `PageDefaults` 等、本 workspace で
/// `#[non_exhaustive]` かつ umbrella re-export 対象の struct が共通して使う
/// 「zero-arg `new()` (= `Default::default()`) + 全 field `pub` による
/// mutation」の 2-pattern 構築契約 (`crates/raikiri/tests/external_consumer.rs`
/// の "3 pattern" acceptance criteria の pattern 1 + pattern 2) を
/// [`Border`] にも適用する。3 field のみの単純な値なので builder
/// (pattern 3) は他の類似 struct (`PageBox` / `PageContext` 等) と同様に
/// 見送り — 複数 setter を持つ多 field config struct 向けの pattern であり、
/// このためだけの builder は無駄な surface になる。
impl Border {
    /// CSS Backgrounds 3 の初期値
    /// (`width` = medium = 3px §3.3 / `style` = `none` §3.2 / `color` = `currentcolor` §3.1)
    /// を持つ `Border` を返す zero-arg constructor。`Self::default()` の thin
    /// wrapper — `raikiri_traits::page::PageBox::new` と同じ shape。
    ///
    /// 全 field が `pub` なので、initial 以外の値が要る呼び手は
    /// `let mut b = Border::new(); b.width = Length::Px(5.0);` の mutation
    /// pattern で組み立てる (`external_consumer_can_mutate_pub_fields_via_default_shorthand`
    /// が umbrella 経由でこの経路を pin する)。
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for Border {
    /// CSS Backgrounds 3 initial value。[`crate::specified::INITIAL_BORDER`]
    /// (cascade の internal fast-path 用 `pub(crate) const`) と同じ値を保つ —
    /// 単体テスト `border_default_matches_initial_border`
    /// ([`crate::specified::INITIAL_BORDER`] と `assert_eq!` で突き合わせ) が
    /// drift を検知する。2 つを 1 本化しない理由: `INITIAL_BORDER` は `const`
    /// (cascade hot path で使う compile-time 値) だが、trait method
    /// (`Default::default`) は stable Rust では `const fn` にできないため。
    ///
    /// # sibling [`BorderStyle`] / [`BorderColor`] の "Default は derive しない"
    /// 注記との関係
    ///
    /// [`BorderStyle`] の doc は「`Default` は derive しない — 本 crate の
    /// convention は "derive `Default` iff `.default()` が call される"」と
    /// 述べている。これは **`#[derive(Default)]`** (呼ばれない Default を
    /// タダだから足す) の話であり、本 impl はそれとは逆で「呼ばれるから
    /// 手書きで足す」— 内部からは [`Border::new`]、外部からは umbrella
    /// (`raikiri` crate) の pattern-1/pattern-2 construction pin
    /// (`external_consumer_can_construct_all_non_exhaustive_types` /
    /// `external_consumer_can_mutate_pub_fields_via_default_shorthand`) が
    /// 実際に呼ぶ。手書き `impl Default` を `#[non_exhaustive]` struct に
    /// 足す in-crate precedent は `counter_style.rs` の
    /// [`NegativeDescriptor`](crate::counter_style::NegativeDescriptor) /
    /// [`PadDescriptor`](crate::counter_style::PadDescriptor) (どちらも
    /// hand-written `impl Default`、derive ではない) — 同じ判断基準
    /// (「呼ばれるかどうか」) の適用であり、本 impl はその convention への
    /// 違反ではなくむしろ一致。
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
/// - **Non-goal**: `oblique <angle [-90deg,90deg]>?` — the optional
///   `<angle>` argument needs its own payload-carrying variant, range
///   clamping, and the "plus angle in degrees" half of the computed-value
///   rule quoted above; deferred as a follow-up.
/// - **Non-goal**: `left` / `right` — additional slant-direction keywords
///   in the same propdef grammar quoted above. Not implemented here;
///   silent drop like any other unhandled ident (below).
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 4 keyword (`normal` / `italic` / `left` /
///   `right`、`oblique` は未実装) 以外の ident は silent drop = `None`。
///
/// With only `normal` / `italic` implemented, the angle-bearing branch of
/// the spec's "Computed value" row is unreachable — so for this crate's
/// scope, computed value = specified keyword, no relative resolution
/// needed ([`Direction`] doc と同型)。
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
/// # Scope carving
///
/// - **Non-goal**: `full-width` / `full-size-kana` — the propdef's `||`
///   double-bar combinator lets either keyword co-occur alongside a case
///   keyword (`capitalize | uppercase | lowercase`) **in either order**;
///   this crate implements only the case-keyword group (below) and does
///   not recognize `full-width` / `full-size-kana` as idents. A single
///   ident such as `full-width` on its own is rejected the same as any
///   other unknown ident (below). A two-ident combination is rejected
///   regardless of which order the case keyword and the unimplemented
///   keyword appear in, but by two different mechanisms depending on
///   which ident comes first ([`parse_text_transform`]'s doc has the
///   per-arm detail):
///   - unimplemented-first (e.g. `full-width uppercase`) —
///     [`parse_text_transform`] itself fails on the first (unrecognized)
///     ident, so `parse_value` already returns `None` before `DeclParser`'s
///     exhaustive-consumption check is even reached.
///   - case-keyword-first (e.g. `uppercase full-width`) —
///     [`parse_text_transform`] succeeds on `uppercase` and leaves
///     `full-width` unconsumed; the whole declaration is then dropped by
///     `DeclParser` (in [`mod@crate::rule`])'s exhaustive-consumption
///     check (the same general mechanism that rejects `font-size: 16px
///     20px`) — no property-specific lookahead is needed here.
///
///   Either path lands on the same outcome (whole declaration dropped),
///   so this crate does not need to special-case `||` order.
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical。sibling [`FontStyle`] と同 convention)。
/// - **(a) spec-invalid**: 上記 4 keyword (`none`/`capitalize`/`uppercase`/
///   `lowercase`、`full-width`/`full-size-kana` は未実装) 以外の ident は
///   silent drop = `None`。
///
/// # Downstream handoff
///
/// Actually applying the case transform (the Unicode default-case
/// algorithm, and word segmentation for `capitalize`'s titlecase rule) is
/// out of this crate's scope — it belongs to the text-shaping/paint layer
/// that consumes the computed value. This property carries only the
/// cascade static-side keyword, mirroring
/// [`crate::computed::ComputedValues::vertical_align`]'s "Downstream
/// handoff" doc note.
///
/// [`FontStyle`] / [`Direction`] と同じ convention で `Default` を derive
/// しない — 初期化側 ([`crate::specified::SpecifiedValues::initial`] /
/// [`crate::computed::ComputedValues::initial`]) が [`TextTransform::None`]
/// を直接指定する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    /// `none` — spec initial value。"No effects."
    None,
    /// `capitalize` — "Puts the first typographic letter unit of each word,
    /// if lowercase, in titlecase; other characters are unaffected."
    Capitalize,
    /// `uppercase` — "Puts all letters in uppercase."
    Uppercase,
    /// `lowercase` — "Puts all letters in lowercase."
    Lowercase,
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
/// depth 増減のみ行う。実際の `quotes` property 引き (nesting depth → 文字列)
/// は本 crate の static-side scope 外 — 下流 (raikiri-dom) が `quotes` の
/// computed value と併せて runtime resolve する ([`CounterStyle`] /
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

/// [`parse_content_list_items`] の list vocabulary mode selector。
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
enum ContentListMode {
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
    /// `attr(<attribute-name>)` (§2.1、type/fallback は未実装、将来対応)。
    Attr { name: SmolStr },
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
    /// element" と述べるが、[`parse_content`] は `normal` を (既存 `none` と
    /// 同様) 空 `Vec` に畳んで保持する — computed-value 時の `normal` →
    /// `contents` 展開は本 crate の static-side (specified 層) scope 外。した
    /// がって author が明示的に書いた `content: contents` は `[Contents]` を
    /// 返す一方、`content: normal` (initial value 相当) は `[]` を返す —
    /// specified 層での「明示 vs 省略」の区別を保つための意図的非対称性で、
    /// spec 違反ではない (computed-value 展開は downstream 責務)。
    Contents,
    /// `<quote>` (`open-quote` / `close-quote` / `no-open-quote` /
    /// `no-close-quote`) — CSS Content 3 §2.4.2
    /// <https://www.w3.org/TR/css-content-3/#quote-values>。詳細は
    /// [`QuoteKeyword`] の doc を参照 (実際の引用符文字列解決は `quotes`
    /// property の computed value と合わせて downstream が行う)。
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
/// / `none` / `flex` / `grid` の 6 値。`table*` / `list-item` /
/// `flow-root` (standalone) / `contents` 等 spec-valid だが未実装
/// (将来対応) の keyword は `parse_display` が `None` を返し、
/// declaration が silent drop される (rule.rs 側 invalid-value drop path)。
///
/// `#[non_exhaustive]`: variant 追加を non-breaking にする (InlineBlock /
/// None / Flex / Grid 追加は本 attribute 経由で forward-compatible)。
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
/// `#[non_exhaustive]` は 37n sibling [`DisplayValue`] / [`TextAlign`] /
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
///   (`resolve_text_align_match_parent` の debug_assert が pin する不変条件)。
/// - **(a) spec-invalid**: CSS Text 3 §6.1 grammar は上記 8 keyword のみ。それ以外
///   の ident (`middle`, `baseline` 等、および CSS Text 4 draft 相当の `<string>`
///   character alignment は本 crate が引用する CSS Text 3 では未定義) は silent
///   drop = `None`。
///
/// [`DisplayValue`] と同じ convention で `Default` を derive しない — 本 enum の
/// `.default()` は呼ばれず、初期化側 [`crate::computed::ComputedValues::initial`]
/// が [`TextAlign::Start`] を直接指定する (37n sibling pattern:
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
///   が canonical。37n sibling [`TextAlign`] と同 convention)。
/// - **(a) spec-invalid**: `ltr` / `rtl` 以外の ident は silent drop = `None`。
///   spec には旧 draft 相当の `auto` 値は無い (現行 §2.1 grammar は 2 keyword のみ)。
/// - **Non-goal**: HTML `dir` attribute → UA-level `direction` mapping
///   (spec が "we recommend HTML authors to use the HTML dir attribute" と述べる
///   presentational hint) は本 crate の parse/cascade scope に無い — UA CSS
///   default 値の持ち込みは cleanroom 対象外の別 task。
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
/// 37n sibling [`BoxSizing`] / [`Direction`] と同じ convention で `Default`
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

/// `position` property の value — static-side scope では `static` (default) と
/// GCPM `running(<custom-ident>)` のみ受理する。
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// は element を normal flow から取り除き、`element()` 経由で page margin box に
/// 配置可能な template として登録する。
///
/// `relative` / `absolute` / `fixed` / `sticky` は未実装 (将来対応)、silent drop
/// (parse_position が `None`)。`static` を明示的に variant 化しているのは、
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
    /// iff `.default()` が call される" (37n sibling [`DisplayValue`] と同じ、
    /// spec default は初期化側 [`crate::computed::ComputedValues::initial`] が
    /// 直接指定する)。
    Static,
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
/// `#[non_exhaustive]` を付けない — sibling [`OverflowXY`] と同じ判断
/// (umbrella (`raikiri` crate) へ再 export されておらず、CSS spec が
/// 定める 4 keyword は Level 4 時点でも増えていないため、将来 field 追加の
/// 蓋然性が [`Border`] ほど高くない)。
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
}

impl TextDecorationLine {
    /// `none` — spec initial value。装飾線なし (4 flag 全て `false`)。
    pub const NONE: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: false,
    };
    /// `underline` 単独。
    pub const UNDERLINE: Self = Self {
        underline: true,
        overline: false,
        line_through: false,
        blink: false,
    };
    /// `overline` 単独。
    pub const OVERLINE: Self = Self {
        underline: false,
        overline: true,
        line_through: false,
        blink: false,
    };
    /// `line-through` 単独。
    pub const LINE_THROUGH: Self = Self {
        underline: false,
        overline: false,
        line_through: true,
        blink: false,
    };
    /// `blink` 単独。
    pub const BLINK: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
        blink: true,
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
/// `Default` は derive しない — 37n sibling [`DisplayValue`] / [`Direction`] と
/// 同じ convention (spec default は初期化側
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
///
/// `#[non_exhaustive]` — sibling [`BorderStyle`] と同じ判断 (line-style 系
/// keyword enum の 37n 慣行、future variant の non-breaking 追加)。
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
/// `<'text-decoration-line'> || <'text-decoration-style'> ||
/// <'text-decoration-color'>`。
///
/// [`PropertyValue::TextDecoration`] の payload としてのみ存在し、
/// [`crate::rule::expand_shorthand_into`] が
/// [`PropertyValue::TextDecorationLine`] / [`PropertyValue::TextDecorationStyle`] /
/// [`PropertyValue::TextDecorationColor`] の 3 longhand へ展開した後は捨てられる
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

/// `vertical-align` property の value。
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
/// - **実装済み**: `baseline` (spec initial value) / `sub` / `super` の 3
///   keyword。いずれも percentage / length を運ばないため、computed value =
///   specified keyword そのまま (相対解決なし)。raikiri-paint がこの 3
///   keyword を実際の glyph 描画位置へ反映する (下記「baseline shift 量の
///   計算は raikiri-paint scope」節)。
/// - **(b) 非対応、silent drop 継続**: `top` / `text-top` / `middle` /
///   `bottom` / `text-bottom` keyword は spec-valid だが未実装。box
///   alignment (line box 内の他 box との高さ比較) を要し、inline
///   formatting context / line box model 抜きには計算できないため未実装
///   — この 5 keyword を「parse は通すが raikiri-paint は shift 0 として
///   扱う」形に緩めることは**しない**: そうすると、UA/author が `sub` /
///   `super` (raikiri-paint が実際に shift する) より高い cascade priority
///   で `top` 等を宣言した場合、cascade 上は正当に winner になるが
///   raikiri-paint は無視するため、**現在は正しく shift している要素が
///   silent に shift 0 へ後退する** — 今 silent-reject (`None`) のままなら
///   その宣言自体が cascade に参加せず起きない regression である。5
///   keyword 全てが raikiri-paint 側でも実装されるまでは、parse 側で先に
///   受理しない方が安全。
/// - **(b) 非対応**: `<percentage>` value は spec-valid だが未実装、silent
///   drop (`None`)。CSS 2.1 §10.8.1 はこの percentage を要素自身の
///   `line-height` 基準で定義する (propdef の "Percentages: refer to the
///   'line-height' of the element itself")。`line-height: normal` (spec
///   initial value、宣言が無い要素の既定) の下では
///   [`crate::resolve::used_line_height_length`] が `None` を返す —
///   real font metrics を style 層に持たないため "normal" を絶対長化できない
///   (同関数 doc の "normal" wall が canonical)。`sub`/`super` の shift 量
///   (下記「baseline shift 量の計算は raikiri-paint scope」節) と違い、この
///   percentage には spec 側の UA-default fallback (CSS Inline
///   Layout Module Level 3 §4.2.3 の `baseline-shift` 相当記述) が存在しない
///   ため、`line-height: normal` という最も一般的なケースを誠実に近似する
///   手段が無い。汎用 length resolver
///   ([`crate::resolve::resolve_length`]、`Length::Percent` を "grammar 上
///   到達しない" 前提で `0px` に落とす) へそのまま通す実装は誤り —
///   `0%` は spec 上 `baseline` と同義だが、非 0 の percentage まで一律
///   `0px` に潰すのは近似ではなく誤変換になる。`padding` / `margin` が
///   `<percentage>` に対して採る「絶対化せず computed 層まで素通しし、
///   使用先で解決する」staging pattern もここでは借用先が無い —
///   raikiri-dom / raikiri-paint のいずれも今日時点で
///   [`crate::computed::ComputedValues::line_height`] を読む consumer を
///   持たず (line-height 自体、`normal` を実解決する行き先が現状存在
///   しない)、percentage 残滓を素通しして渡す先が無い。
/// - **(b) 非対応**: `<length>` value も spec-valid だが未実装、silent drop
///   (`None`)。percentage と異なりこちらは基準 (line-height / font
///   metrics) を必要としない絶対値であり、既存の汎用 length resolver
///   ([`crate::resolve::resolve_length`]、`border-*-width` /
///   `line-height` の `<length>` 成分が既に使う) でそのまま近似なしに
///   絶対化できる — 単体では tractable。それでも percentage と同じ bucket
///   に留めて未実装のままにしているのは、この enum が
///   [`PropertyValue::VerticalAlign`] の payload として `@page` 側の
///   phase-3 pipeline ([`crate::page`] の `absolutize_in_page_context` /
///   `specified_layer_residue`) にもそのまま流れるため。**両関数とも現状
///   `PropertyValue::VerticalAlign(_)` を内側の `VerticalAlign` enum に
///   対する wildcard として match している** — 新しい top-level
///   `PropertyValue` variant の追加はどちらの関数の (それ自体は網羅的な)
///   外側 match でも compile error として強制されるが、`VerticalAlign`
///   内部への variant 追加だけではこの wildcard がそれを黙って吸収し、
///   どちらの site も compile error を出さない。すなわち length を運ぶ
///   variant を追加しても、この 2 箇所の分類更新を忘れたままビルドも既存
///   test も素通りしうる — [`length_payload`] の doc が言う fail-quiet
///   (catch-all が拡張漏れを compile error にせず黙って吸収する class) と
///   同じ risk であり、単なる bookkeeping コストの話ではない
///   ([`crate::page`] のテスト側 corpus 分類、詳細は同 module 参照)。
///   `raikiri-style` 側の staging 追加 (既存の `padding` / `margin` /
///   `width` / `height` と同型) 自体は raikiri-paint 側の変更と独立に
///   着手できるが、この `@page` 側の fail-quiet gap を塞ぐ作業は独立ではなく
///   同じ変更に付随する — `<percentage>` が単独では着手できない以上、
///   `<length>` だけを切り出して先に landing するより、両者をまとめて
///   1 つの follow-up として着手する方がこの gap を確実に塞ぎやすい。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記以外の ident は silent drop = `None`。
/// - **baseline shift 量の計算は raikiri-paint scope**: `sub` / `super` が
///   指す実際の shift 量計算 (parent's used font-size を基準にした px offset)
///   と glyph 描画位置への反映は raikiri-paint 側の責務。本 crate はこの
///   computed value (`Sub` / `Super` の bare keyword) を運ぶだけで、shift
///   量の算出は行わない。
///
/// `Default` は derive しない — 37n sibling [`TextDecorationShorthand`] と
/// 同じ convention (spec default は初期化側
/// [`crate::computed::ComputedValues::initial`] が直接指定する)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
/// only recognizes `static` and CSS GCPM 3's `running(<custom-ident>)` —
/// the CSS2 `relative` / `absolute` / `fixed` / `sticky` keywords that the
/// propdef's "Applies to: positioned elements" clause presupposes are not
/// implemented yet. Stacking-context construction and paint-order
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
    /// verbatim)
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
///   way [`FontStyle`]'s unimplemented `oblique`/`left`/`right` keywords
///   are. It is [`OverflowWrap::BreakWord`] — the non-deprecated
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
/// この crate の [`DisplayValue`] scope (`block` / `inline` / `inline-block`
/// / `none` / `flex` / `grid`) に絞ると、表の中段に該当するのは
/// [`DisplayValue::Inline`] と [`DisplayValue::InlineBlock`] の 2 variant
/// だけ ([`DisplayValue`] は table 系 keyword を実装していない)。
/// [`DisplayValue::Flex`] / [`DisplayValue::Grid`] は表に**登場しない**
/// ("others" 側、same as specified) — floated flex/grid container は
/// float してもそのまま `flex`/`grid` の computed value を保つ。
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
/// 扱う (`crate::page::absolutize_in_page_context` の該当 arm 参照)。
///
/// `pub(crate)` — 呼び手は `specified` module のみ。
pub(crate) fn resolve_display_for_float(display: DisplayValue, float: FloatValue) -> DisplayValue {
    if matches!(float, FloatValue::None) {
        return display;
    }
    match display {
        DisplayValue::None => DisplayValue::None,
        DisplayValue::Inline | DisplayValue::InlineBlock => DisplayValue::Block,
        // `DisplayValue::None` の doc 直上の rationale と同じ理由で、この
        // arm もあえて `_` に潰さない — `Block` / `Flex` / `Grid` を明示
        // 列挙することで、将来 `DisplayValue` に table-family variant
        // (CSS2 §9.7 表の `inline-table` / `table-row-group` 等) が
        // 追加された時、この match が非網羅になり compile error で
        // 呼び出し元に再考を強制する (`#[non_exhaustive]` は crate 外部
        // consumer 向けの属性であり、定義 crate 内部のこの match には
        // 適用されない)。silent に「specified のまま」へ pass-through
        // させてしまうと §9.7 表を under-apply する。
        same @ (DisplayValue::Block | DisplayValue::Flex | DisplayValue::Grid) => same,
    }
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
/// - [`Content`](Self::Content): `Content(Vec<ContentComponent>)` →
///   `Content(Arc<Vec<ContentComponent>>)`
/// - [`StringSet`](Self::StringSet): payload の outer `Vec<..>` を `Arc<Vec<..>>` に
///
/// 同じ cascade memory DoS 対策の counter-* への拡張は、当時 consumer への
/// live impact 0 だったが同 pattern:
///
/// - [`CounterReset`](Self::CounterReset) / [`CounterIncrement`](Self::CounterIncrement) /
///   [`CounterSet`](Self::CounterSet): `Vec<(SmolStr, i32)>` → `Arc<Vec<(SmolStr, i32)>>`
///
/// 同種の Arc-wrap パターンの踏襲 (目的は perf 改善であり、security 対策では
/// ない) は同 pattern を最後の non-Arc `Vec` payload に適用する:
///
/// - [`FontFamily`](Self::FontFamily): `FontFamily(Vec<Atom>)` →
///   `FontFamily(Arc<Vec<Atom>>)`
///
/// Pattern-match で payload を **読む** consumer は `Arc<Vec<T>>` の
/// `Deref<Target = Vec<T>>` → `Deref<Target = [T]>` chain により、`match` arm
/// で `PropertyValue::Content(components) => components.iter()` のような使い方が
/// **透過的に継続動作** する (`&Arc<Vec<T>>` は autoderef で `&[T]` として使える)。
/// 一方、`PropertyValue::Content(vec![...])` のように payload を **construct** する
/// 場合は `PropertyValue::Content(Arc::new(vec![...]))` への書き換えが必要。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
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
    /// **shallow (Arc bump)** になる。`font-family` は inherited property なので
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
    /// `FontSize` の payload 型を変えるとこの pin が割れ、
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
    /// `counter-reset: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited。spec initial は `none` (CSS Lists 3 §4.1)、本 impl はそれを
    /// 空 list で表現する。
    /// missing integer は 0 に default (spec default)。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`) + inheritance walk clone (`resolve_inheritance` の
    /// `stack.push((child, computed.clone()))` + `out[idx] = computed.clone()`)
    /// が **shallow (Arc bump only)** になる。counter-* は non-inherited のため
    /// child は inherit_from で shared empty slot に落ちるが、winner までの経路
    /// (parent stack entry + cascaded candidates 蓄積) は deep-clone 経由だった。
    /// `* { counter-reset: c0 c1 ... cN }` × M element で O(N × M) → O(N + M)
    /// (cascade memory DoS 対策、Content/StringSet pattern の踏襲)。
    CounterReset(Arc<Vec<(SmolStr, i32)>>),
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
    /// `normal`、本 impl は `normal` / `none` をどちらも空 list で表現する
    /// (pseudo-element 生成判断は下流 layer)。CSS Content 3 §1
    /// <https://www.w3.org/TR/css-content-3/#content-property>。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone + inheritance walk stack
    /// entry clone + per-node write が **shallow (Arc bump only)** になる。
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
    /// `position: static | running(<custom-ident>)` — non-inherited、initial:
    /// `static`。
    /// CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>。
    /// 現状 scope では `running()` seed emit のみが下流に伝わる —
    /// `Static` は `apply_value` で no-op (先行 `running()` を上書き suppress
    /// する discriminant 用途、spec default に相当)。
    /// `relative` / `absolute` / `fixed` / `sticky` は未実装 (将来対応)、
    /// parser 段で drop。
    Position(PositionValue),
    /// `text-align: start | end | left | right | center | justify | match-parent
    /// | justify-all` — **inherited**、initial: [`TextAlign::Start`]
    /// (CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
    /// <https://www.w3.org/TR/css-text-3/#text-align-property>)。
    /// spec 上 shorthand (text-align-all + text-align-last) だが
    /// 単一 field に保持 (**(b) 非対応**、longhand 分離は
    /// 後続 task で defer)。詳細は [`TextAlign`] doc-comment。
    TextAlign(TextAlign),
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
    /// # Scope carving ((b) 非対応)
    ///
    /// The `hanging` and `each-line` keywords are not implemented — this
    /// variant's payload is a bare [`Length`], not a struct that could also
    /// carry them. `hanging` reverses which lines an indent applies to
    /// (normally the first line only; with `hanging`, every line *except*
    /// the first) and `each-line` extends the indent to every line after a
    /// forced break; both require multi-line layout state this crate's
    /// cascade static side does not have. `parse_text_indent` itself only
    /// consumes the leading `<length-percentage>` and does not check for
    /// leftover tokens — `text-indent: 2em hanging` is still rejected as a
    /// whole declaration (not silently truncated to `2em`), but that rejection
    /// happens one layer up, at the `expect_exhausted` check [`crate::rule`]'s
    /// `DeclParser` runs on every declaration's leftover tokens (same
    /// mechanism the `rejects_extra_length_after_font_size` test in
    /// `crate::rule` pins for an unrelated property).
    TextIndent(Length),
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
    /// [`crate::cascade::apply_value`] の `Margin`/`Border` arm doc、および
    /// [`crate::rule::expand_shorthand_into`] doc 参照)。
    /// margin と同じ parse-time expansion model への migrate を検討 (follow-up task)。
    Padding(Sides<Length>),
    /// `margin-top: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginTop(LengthOrAuto),
    /// `margin-right: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginRight(LengthOrAuto),
    /// `margin-bottom: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginBottom(LengthOrAuto),
    /// `margin-left: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    MarginLeft(LengthOrAuto),
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
    /// [`crate::cascade::apply_value`] の `Margin` arm doc、および
    /// [`crate::rule::expand_shorthand_into`] doc 参照)。
    Margin(Sides<LengthOrAuto>),
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
    /// (canonical な記述は [`crate::cascade::apply_value`] の `Border` arm doc、
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
    /// [`LengthOrAuto`] を reuse (37n sibling: `parse_padding_side` の非負フィルタ +
    /// `parse_margin_side` の auto 分岐を合成、`parse_height` doc 参照)。
    ///
    /// resolve (percentage → containing block, `LengthOrAuto::Auto` の実 layout
    /// 高さ計算) は下流 (raikiri-dom `apply_computed_to_style` bridge、future task)
    /// 責務 — 本 crate は cascade static side に留まり raw specified value を保持。
    Height(LengthOrAuto),
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
    /// `vertical-align: baseline | sub | super` — **non-inherited**、initial:
    /// [`VerticalAlign::Baseline`] (CSS 2.1 §10.8.1
    /// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>)。
    /// computed value = specified keyword ([`VerticalAlign`] doc 参照、
    /// percentage/length を運ばないため相対解決なし)。
    /// (末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照。1:1 disjoint な新 field なので配置は自由 — 同節末尾の判断規則)
    VerticalAlign(VerticalAlign),
    /// `font-style: normal | italic` — **inherited**、initial:
    /// [`FontStyle::Normal`] (CSS Fonts 4 §2.4 [`FontStyle`] doc 参照)。
    /// computed value = specified keyword ([`FontStyle`] doc の Scope
    /// carving 節参照、`oblique <angle>?` / `left` / `right` は未実装)。
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
    CounterReset,
    CounterIncrement,
    CounterSet,
    Content,
    StringSet,
    Position,
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
    // margin longhand + shorthand (semantics on the
    // matching PropertyValue::Margin* variants; sibling PropertyKey variants
    // carry no per-variant docs per crate convention).
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Margin,
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
    // width (CSS Sizing 3 §3.1.1)。
    Width,
    // height (CSS Sizing 3 §3.1.1、semantics on the
    // matching PropertyValue::Height variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention).
    Height,
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
}

impl PropertyValue {
    /// この value が属する property key を返す。
    ///
    /// cascade winner selection で "同一 property を勝ち取る" ための discriminant として、
    /// また `@page` cascade 結果 map の key として使う。
    pub fn key(&self) -> PropertyKey {
        match self {
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
            PropertyValue::CounterReset(_) => PropertyKey::CounterReset,
            PropertyValue::CounterIncrement(_) => PropertyKey::CounterIncrement,
            PropertyValue::CounterSet(_) => PropertyKey::CounterSet,
            PropertyValue::Content(_) => PropertyKey::Content,
            PropertyValue::StringSet(_) => PropertyKey::StringSet,
            PropertyValue::Position(_) => PropertyKey::Position,
            PropertyValue::TextAlign(_) => PropertyKey::TextAlign,
            PropertyValue::TextIndent(_) => PropertyKey::TextIndent,
            PropertyValue::PaddingTop(_) => PropertyKey::PaddingTop,
            PropertyValue::PaddingRight(_) => PropertyKey::PaddingRight,
            PropertyValue::PaddingBottom(_) => PropertyKey::PaddingBottom,
            PropertyValue::PaddingLeft(_) => PropertyKey::PaddingLeft,
            PropertyValue::Padding(_) => PropertyKey::Padding,
            PropertyValue::MarginTop(_) => PropertyKey::MarginTop,
            PropertyValue::MarginRight(_) => PropertyKey::MarginRight,
            PropertyValue::MarginBottom(_) => PropertyKey::MarginBottom,
            PropertyValue::MarginLeft(_) => PropertyKey::MarginLeft,
            PropertyValue::Margin(_) => PropertyKey::Margin,
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
            PropertyValue::Width(_) => PropertyKey::Width,
            PropertyValue::Height(_) => PropertyKey::Height,
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
        }
    }
}

/// Property name + Parser から `PropertyValue` を produce。
/// 認識できない name / invalid value は `None`。
pub(crate) fn parse_value(name: &str, input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    // ascii-lowercase 比較で property name を dispatch。
    let normalized_name = name.to_ascii_lowercase();
    match normalized_name.as_str() {
        "color" => parse_color(input).map(PropertyValue::Color),
        // CSS Backgrounds 3 §2.2 <https://www.w3.org/TR/css-backgrounds-3/#background-color>
        // "Base Color: the background-color property"。value grammar は `<color>`、
        // 直上 sibling `color` arm と同じ parse_color reuse pattern。
        "background-color" => parse_color(input).map(PropertyValue::BackgroundColor),
        // Arc wrap は cascade memory 削減 (同種の DoS 対策 fix の
        // pattern 踏襲、perf 目的で security 対策ではない)。`parse_font_family` は
        // grammar 上 empty Vec を返さない (`<family-name>#` は 1 要素以上必須、
        // 同関数の `if families.is_empty() { None }` 参照) ため、counter-* /
        // content / string-set と異なり shared-empty-slot 分岐は不要。
        "font-family" => parse_font_family(input).map(|v| PropertyValue::FontFamily(Arc::new(v))),
        "font-size" => parse_font_size(input),
        "font-weight" => parse_font_weight(input).map(PropertyValue::FontWeight),
        // CSS Inline 3 §5.1 line-height。
        // `normal` / `<number [0,∞]>` / `<length-percentage [0,∞]>` を受理、
        // 負値と其他 keyword は spec grammar 違反として drop。
        "line-height" => parse_line_height(input).map(PropertyValue::LineHeight),
        "display" => parse_display(input).map(PropertyValue::Display),
        // CSS Lists 3 §4 counter properties。
        // spec default: reset = 0、increment = 1、set = 0。
        // Arc wrap は cascade memory DoS 対策 (per-element
        // clone を shallow bump 化)、空 list は 3 property 共通 shared Arc slot
        // (`empty_counter_entries`) に落として per-node allocation regression を
        // 避ける (Content/StringSet の precedent と同 pattern)。
        "counter-reset" => parse_counter_property(input, 0).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterReset(empty_counter_entries())
            } else {
                PropertyValue::CounterReset(Arc::new(v))
            }
        }),
        "counter-increment" => parse_counter_property(input, 1).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterIncrement(empty_counter_entries())
            } else {
                PropertyValue::CounterIncrement(Arc::new(v))
            }
        }),
        "counter-set" => parse_counter_property(input, 0).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterSet(empty_counter_entries())
            } else {
                PropertyValue::CounterSet(Arc::new(v))
            }
        }),
        // CSS Content 3 §1 content property。
        // Arc wrap は cascade memory DoS 対策 (per-element clone を
        // shallow bump 化)、empty list は shared Arc slot に落として per-node allocation
        // regression を避ける。
        "content" => parse_content(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Content(empty_content_list())
            } else {
                PropertyValue::Content(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.1.1 string-set。
        // Arc wrap は同種の DoS 対策 fix と同 rationale。
        "string-set" => parse_string_set(input).map(|v| {
            if v.is_empty() {
                PropertyValue::StringSet(empty_string_set_entries())
            } else {
                PropertyValue::StringSet(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.2.1 position: running()。
        // 現状 scope では `static` + `running(<custom-ident>)` のみ受理、
        // `relative` / `absolute` / `fixed` / `sticky` は未実装 (将来対応) につき silent drop。
        "position" => parse_position(input).map(PropertyValue::Position),
        // CSS Text 3 §6.1 text-align。
        // spec 上 shorthand (text-align-all + text-align-last) だが単一 field で保持
        // ((b) 非対応、`TextAlign` doc-comment 参照)。
        "text-align" => parse_text_align(input).map(PropertyValue::TextAlign),
        // CSS Text 3 §8.1 text-indent — `<length-percentage>` component only
        // (`hanging`/`each-line` out of scope, `PropertyValue::TextIndent` doc).
        "text-indent" => parse_text_indent(input).map(PropertyValue::TextIndent),
        // CSS Box 3 §4.1 padding physical longhand。
        // grammar: <length-percentage `[0,∞]`> — non-negative constraint は
        // parse_padding_side が enforce (parse-time drop、spec-invalid → None)。
        // `auto` keyword は spec grammar に含まれず parse_length_value の Dimension /
        // Percentage arm fall-through で自然 reject。
        "padding-top" => parse_padding_side(input).map(PropertyValue::PaddingTop),
        "padding-right" => parse_padding_side(input).map(PropertyValue::PaddingRight),
        "padding-bottom" => parse_padding_side(input).map(PropertyValue::PaddingBottom),
        "padding-left" => parse_padding_side(input).map(PropertyValue::PaddingLeft),
        // CSS Box 3 §4.2 padding shorthand: `<'padding-top'>{1,4}`
        // <https://www.w3.org/TR/css-box-3/#padding-shorthand>。
        // 1-4 value expansion は parse_padding_shorthand が spec verbatim で適用。
        "padding" => parse_padding_shorthand(input).map(PropertyValue::Padding),
        // CSS Box 3 §3.1 margin-* physical longhand.
        // <length-percentage> | auto の grammar、negative 許容 (spec 準拠、layout
        // 側で負値の意味付け)。
        "margin-top" => parse_margin_side(input).map(PropertyValue::MarginTop),
        "margin-right" => parse_margin_side(input).map(PropertyValue::MarginRight),
        "margin-bottom" => parse_margin_side(input).map(PropertyValue::MarginBottom),
        "margin-left" => parse_margin_side(input).map(PropertyValue::MarginLeft),
        // CSS Box 3 §3.2 margin shorthand. 1-4 value
        // expansion。cascade 段では `PropertyValue::Margin` は `parse_declaration_block`
        // 内で 4 longhand に展開されるため通常観測しない (詳細は
        // `PropertyValue::Margin` doc + `crate::rule::expand_shorthand_into`)。
        "margin" => parse_margin_shorthand(input).map(PropertyValue::Margin),
        // CSS Backgrounds 3 §3.3 border-width physical longhand。grammar:
        // `<line-width>` = `<length [0,∞]> |
        // thin | medium | thick`。`<percentage>` は含まれない (padding とは違う点)。
        // keyword mapping は spec 規定値:
        // thin=1px、medium=3px、thick=5px。負値は spec grammar 違反 → drop
        // (`parse_border_width_side` が enforce)。
        "border-top-width" => parse_border_width_side(input).map(PropertyValue::BorderTopWidth),
        "border-right-width" => parse_border_width_side(input).map(PropertyValue::BorderRightWidth),
        "border-bottom-width" => {
            parse_border_width_side(input).map(PropertyValue::BorderBottomWidth)
        }
        "border-left-width" => parse_border_width_side(input).map(PropertyValue::BorderLeftWidth),
        // CSS Backgrounds 3 §3.2 border-style physical longhand。grammar:
        // `<line-style>` = 10 alternative
        // (none / hidden / dotted / dashed / solid / double / groove / ridge /
        // inset / outset)。他 keyword は silent drop。
        "border-top-style" => parse_border_style_side(input).map(PropertyValue::BorderTopStyle),
        "border-right-style" => parse_border_style_side(input).map(PropertyValue::BorderRightStyle),
        "border-bottom-style" => {
            parse_border_style_side(input).map(PropertyValue::BorderBottomStyle)
        }
        "border-left-style" => parse_border_style_side(input).map(PropertyValue::BorderLeftStyle),
        // CSS Backgrounds 3 §3.1 border-color physical longhand
        // (`parse_border_color` 経由)。grammar: `<color>` に加え
        // `currentcolor` keyword を先取り (CSS Color 3 §4.4)。`BorderColor` enum
        // で specified value distinction を保持し、used-value resolution は
        // paint scope 責務。
        "border-top-color" => parse_border_color(input).map(PropertyValue::BorderTopColor),
        "border-right-color" => parse_border_color(input).map(PropertyValue::BorderRightColor),
        "border-bottom-color" => parse_border_color(input).map(PropertyValue::BorderBottomColor),
        "border-left-color" => parse_border_color(input).map(PropertyValue::BorderLeftColor),
        // CSS Backgrounds 3 §3.4 border shorthand: `<line-width> || <line-style>
        // || <color>` (any-order、each component at most once、at least 1 present)。
        // 4 side 全てに同一 Border を配る。cascade 段では
        // `PropertyValue::Border` は `parse_declaration_block` 内で 12 longhand
        // (4 side × 3 sub-property) に展開されるため通常観測しない (詳細は
        // `PropertyValue::Border` doc + `crate::rule::expand_shorthand_into`)。
        "border" => parse_border_shorthand(input).map(PropertyValue::Border),
        // CSS Sizing 3 §3.1.1 preferred size property。
        // grammar: `auto | <length-percentage [0,∞]> | min-content | max-content
        // | fit-content(<length-percentage>)` のうち `auto` + non-negative
        // `<length-percentage>` のみ受理、min-content / max-content / fit-content()
        // は未実装 (将来対応) として silent drop、負値は spec `[0,∞]`
        // violation として drop (parse_width が enforce)。
        "width" => parse_width(input).map(PropertyValue::Width),
        // CSS Sizing 3 §3.1.1 preferred size — height。
        // grammar: `auto | <length-percentage [0,∞]>` + spec-valid だが現状
        // scope 外の `min-content` / `max-content` / `fit-content()` は silent drop
        // (parse_height 内で ident branch が auto のみ受理して他 keyword 落とし)。
        "height" => parse_height(input).map(PropertyValue::Height),
        // CSS Sizing 3 §3.3 box-sizing。
        // value grammar `content-box | border-box`、initial `content-box`、
        // not inherited、computed value = specified keyword。
        "box-sizing" => parse_box_sizing(input).map(PropertyValue::BoxSizing),
        // CSS Writing Modes 4 §2.1 direction。
        // value grammar `ltr | rtl`、initial `ltr`、inherited、
        // computed value = specified keyword (`Direction` doc 参照)。
        "direction" => parse_direction(input).map(PropertyValue::Direction),
        // CSS Overflow 3 §3.1 overflow-x/overflow-y physical longhand.
        // grammar: visible | hidden | clip | scroll |
        // auto, initial visible, not inherited. cross-axis computed-value
        // coupling is applied in phase 3 (`resolve_overflow`), not here —
        // this only carries the specified keyword.
        "overflow-x" => parse_overflow_value(input).map(PropertyValue::OverflowX),
        "overflow-y" => parse_overflow_value(input).map(PropertyValue::OverflowY),
        // CSS Overflow 3 §3.1 overflow shorthand: `<'overflow-block'>{1,2}`.
        // 1-2 value expansion via
        // parse_overflow_shorthand (mapped to physical x/y — `OverflowValue`
        // doc's Non-goal note).
        "overflow" => parse_overflow_shorthand(input).map(PropertyValue::Overflow),
        // CSS Text Decoration Module Level 3 §2.1 text-decoration-line
        // grammar: `none | [ underline || overline || line-through || blink ]`.
        "text-decoration-line" => {
            parse_text_decoration_line(input).map(PropertyValue::TextDecorationLine)
        }
        // §2.2 text-decoration-style grammar: `solid | double | dotted |
        // dashed | wavy`.
        "text-decoration-style" => {
            parse_text_decoration_style(input).map(PropertyValue::TextDecorationStyle)
        }
        // §2.3 text-decoration-color grammar: `<color>`.
        "text-decoration-color" => {
            parse_text_decoration_color(input).map(PropertyValue::TextDecorationColor)
        }
        // §2.4 text-decoration shorthand: `<'text-decoration-line'> ||
        // <'text-decoration-style'> || <'text-decoration-color'>`.
        "text-decoration" => {
            parse_text_decoration_shorthand(input).map(PropertyValue::TextDecoration)
        }
        // CSS 2.1 §10.8.1 vertical-align, restricted to `baseline` / `sub` /
        // `super` (minimal scope — `VerticalAlign` doc's "Scope carving"
        // section). initial `baseline`, not inherited, computed value =
        // specified keyword.
        "vertical-align" => parse_vertical_align(input).map(PropertyValue::VerticalAlign),
        // CSS Fonts 4 §2.4 font-style. grammar: `normal | italic | left |
        // right | oblique <angle [-90deg,90deg]>?`, restricted here to
        // `normal` / `italic` (`FontStyle` doc's "Scope carving" section —
        // `oblique <angle>?` and `left`/`right` are spec-valid but
        // unimplemented). initial `normal`, inherited, computed value =
        // specified keyword (angle-bearing branch unreachable at this
        // scope).
        "font-style" => parse_font_style(input).map(PropertyValue::FontStyle),
        // CSS Text Module Level 3 §2.1 text-transform. grammar: `none |
        // [capitalize | uppercase | lowercase] || full-width ||
        // full-size-kana`, restricted here to `none` / `capitalize` /
        // `uppercase` / `lowercase` (`TextTransform` doc's "Scope carving"
        // section — `full-width` / `full-size-kana` are spec-valid but
        // unimplemented). initial `none`, inherited, computed value =
        // specified keyword.
        "text-transform" => parse_text_transform(input).map(PropertyValue::TextTransform),
        // CSS Display 3 §4 visibility. grammar: `visible |
        // hidden | collapse`. initial `visible`, inherited, computed value =
        // specified keyword (`Visibility` doc's "Scope carving" section —
        // `collapse`'s formatting-context-specific space-saving effect is
        // unimplemented, the keyword itself is fully accepted).
        "visibility" => parse_visibility(input).map(PropertyValue::Visibility),
        // CSS2 §9.9.1 z-index. grammar: `auto | <integer>` (`inherit` — the
        // propdef's third alternative — is the CSS-wide keyword, unhandled
        // here per the "CSS-wide keyword (canonical)" section above).
        // initial `auto`, not inherited, computed value = specified value.
        "z-index" => parse_z_index(input).map(PropertyValue::ZIndex),
        // CSS Text 3 §5.1 word-break. grammar (this crate's scope):
        // `normal | keep-all | break-all` — the spec's 4th, deprecated
        // `break-word` keyword is not implemented (`WordBreak` doc's
        // "Scope carving" section). initial `normal`, inherited, computed
        // value = specified keyword.
        "word-break" => parse_word_break(input).map(PropertyValue::WordBreak),
        // CSS Text 3 §5.4 overflow-wrap, grammar: `normal | break-word |
        // anywhere`. `word-wrap` is the spec's mandated legacy name alias
        // for this same property (`OverflowWrap` doc's "legacy alias"
        // section) — both names parse to the same `PropertyValue` variant /
        // `PropertyKey`. initial `normal`, inherited, computed value =
        // specified keyword.
        "overflow-wrap" | "word-wrap" => {
            parse_overflow_wrap(input).map(PropertyValue::OverflowWrap)
        }
        // CSS Text 3 §7.2 "Tracking: the letter-spacing property"
        // <https://www.w3.org/TR/css-text-3/#letter-spacing-property>.
        // grammar: `normal | <length>`, initial `normal`, inherited,
        // percentage NOT supported ("Percentages: n/a"), negative lengths
        // allowed ("Values may be negative, but there may be
        // implementation-dependent limits.") — see `parse_letter_or_word_spacing`.
        "letter-spacing" => parse_letter_or_word_spacing(input).map(PropertyValue::LetterSpacing),
        // CSS Text 3 §7.1 "Word Spacing: the word-spacing property"
        // <https://www.w3.org/TR/css-text-3/#word-spacing-property>. Same
        // `normal | <length>` grammar as `letter-spacing` above.
        "word-spacing" => parse_letter_or_word_spacing(input).map(PropertyValue::WordSpacing),
        // CSS Fragmentation Module Level 3 §3.1 break-before / break-after.
        // grammar (this crate's scope): `auto | avoid | avoid-page | page`
        // (`BreakBetween` doc's "Scope carving" section). initial `auto`,
        // not inherited, computed value = specified keyword.
        "break-before" => parse_break_between(input).map(PropertyValue::BreakBefore),
        "break-after" => parse_break_between(input).map(PropertyValue::BreakAfter),
        // CSS Fragmentation Module Level 3 §3.2 break-inside. grammar (this
        // crate's scope): `auto | avoid | avoid-page` (`BreakInside` doc's
        // "Scope carving" section — a smaller, disjoint set from
        // `break-before`/`break-after`). initial `auto`, not inherited,
        // computed value = specified keyword.
        "break-inside" => parse_break_inside(input).map(PropertyValue::BreakInside),
        // CSS Fragmentation Module Level 3 §3.4 "Page Break Aliases" —
        // CSS2.1 legacy shorthands for break-before / break-after, with a
        // non-identity value remap (`BreakBetween` doc's "legacy
        // shorthand" section: `always` -> `page`, `auto`/`avoid` identity).
        // Both dispatch to the same `PropertyValue`/`PropertyKey` as
        // break-before/break-after (one cascade winner, not two).
        "page-break-before" => {
            parse_legacy_page_break_between(input).map(PropertyValue::BreakBefore)
        }
        "page-break-after" => parse_legacy_page_break_between(input).map(PropertyValue::BreakAfter),
        // CSS Fragmentation Module Level 3 §3.4 — CSS2.1 legacy shorthand
        // for break-inside, identity value mapping (`BreakInside` doc's
        // "legacy shorthand" section: CSS2.1's own `page-break-inside`
        // grammar is just `auto | avoid`).
        "page-break-inside" => {
            parse_legacy_page_break_inside(input).map(PropertyValue::BreakInside)
        }
        // CSS2 §9.5.1 float. grammar: `left | right | none` (`inherit` —
        // the propdef's fourth alternative — is the CSS-wide keyword,
        // unhandled here per the "CSS-wide keyword (canonical)" section
        // above). initial `none`, not inherited, computed value =
        // specified value; §9.7's forced `display` recomputation is
        // applied separately at phase 3 (`resolve_display_for_float` doc).
        "float" => parse_float(input).map(PropertyValue::Float),
        // CSS2 §9.5.2 clear. grammar: `none | left | right | both`
        // (`inherit` unhandled, same reason as `float` above). initial
        // `none`, not inherited, computed value = specified value.
        "clear" => parse_clear(input).map(PropertyValue::Clear),
        _ => None,
    }
}

/// `<color>` を parse する。
///
/// cssparser 0.37 は (0.36 までと異なり) 汎用 `Color` enum / `Color::parse` を
/// 提供しない — それは別 crate `cssparser-color` 側に移った。ここでは
/// 各 form の parse を自前 (cleanroom) で組み立て、hex / named / rgb() の
/// 3 形式をカバーする:
///
/// - **Hex** (`#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa`) は
///   [`CssColor::from_hex`] を呼び出す — CSS Color 4 §5.2 準拠の cleanroom 実装。
///   `Token::Hash` / `Token::IDHash` の payload は leading `#` を含まないため
///   そのまま渡す。
/// - **Named color** は `parse_named_color` (140+ CSS Color L3 keyword table を
///   再実装しない方針のため cssparser の table を暫定利用)。
/// - **`rgb()` / `rgba()` function form** は [`parse_rgb_function`] で
///   `parse_nested_block` 経由の手動 parse。
///
/// `transparent` keyword は CSS Color 4 §6.3 "The transparent keyword"
/// <https://www.w3.org/TR/css-color-4/#transparent-color> で
/// `rgba(0, 0, 0, 0)` の shorthand と規定される — `parse_named_color` の
/// (r, g, b) は alpha を返さないため、Ident arm 手前で明示 branch して
/// [`CssColor::TRANSPARENT`] を返す。
fn parse_color(input: &mut Parser<'_, '_>) -> Option<CssColor> {
    let token = input.next().ok()?.clone();
    match token {
        Token::Hash(ref value) | Token::IDHash(ref value) => CssColor::from_hex(value),
        Token::Ident(ref name) if name.eq_ignore_ascii_case("transparent") => {
            Some(CssColor::TRANSPARENT)
        }
        Token::Ident(ref name) => {
            let (r, g, b) = parse_named_color(name).ok()?;
            Some(CssColor { r, g, b, a: 255 })
        }
        Token::Function(ref name)
            if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") =>
        {
            input.parse_nested_block(parse_rgb_function).ok()
        }
        _ => None,
    }
}

/// `border-*-color` の value parser — `currentcolor` keyword を先取りしてから
/// 既存 [`parse_color`] に委譲する。
///
/// CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
/// の border-*-color grammar は `<color>` そのもの、`<color>` production は
/// CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>
/// `currentcolor` keyword を含む。しかし本 crate の [`parse_color`] は
/// cssparser の `parse_named_color` (RGB triple mapping)
/// 経由のため `currentcolor` は named-color table 未収載として `None` 側に
/// 落ちる — 本 helper が Ident 段で先取りする必要がある。resolution 委譲の
/// rationale は [`BorderColor`] enum doc 参照 (paint scope 責務)。
///
/// 5 call site (4 longhand + [`parse_border_shorthand`] color slot) が本
/// helper を経由する (37n sibling-arm convention consistency)。
fn parse_border_color(input: &mut Parser<'_, '_>) -> Option<BorderColor> {
    // `expect_ident_matching` は ASCII case-insensitive (cssparser 慣行、
    // sibling `parse_margin_side` line 1892 と同 shape の keyword intercept)。
    // 失敗時 `try_parse` が rewind、続く `parse_color` が Ident (named /
    // transparent) / Hash / Function の全 alternative を担当。
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(BorderColor::CurrentColor);
    }
    parse_color(input).map(BorderColor::Resolved)
}

/// `rgb()` / `rgba()` legacy comma syntax の中身 (関数呼び出しの括弧内) を
/// parse する。`parse_nested_block` の caller 側で `rgb(` / `rgba(` の function
/// token は既に consume 済み。`rgb` / `rgba` の function name は spec 上 alias
/// (CSS Color 4 §5.1: "rgb() and rgba() are now aliases for each other")
/// — alpha 省略は両者で許容し、name-based branching は行わない。
///
/// # Grammar (CSS Color 4 §5.1)
///
/// <https://www.w3.org/TR/css-color-4/#rgb-functions>
///
/// ```text
/// legacy-rgb-syntax  = rgb(  <legacy-rgb-channel>#{3} , <alpha-value>? )
/// legacy-rgba-syntax = rgba( <legacy-rgb-channel>#{3} , <alpha-value>? )
/// legacy-rgb-channel = <number> | <percentage>
/// alpha-value        = <number> | <percentage>
/// ```
///
/// legacy form の 3 channel は **all-number** or **all-percentage** の同一種で
/// なければならず、mix (`rgb(255, 50%, 0)`) は spec-invalid (§5.1:
/// "In the legacy form, the color channels can only be either all `<number>`s
/// or all `<percentage>`s — mixing types isn't allowed.")。
///
/// # Clamping
///
/// §5.1: "Values outside these ranges are not invalid, but are clamped to the
/// ranges defined here at parsed-value time" — 負値 / >255 (number) や
/// 100% 超も spec-valid、clamp only。
///
/// - `<number>` 0..=255 → `clamp_channel` で `i32.clamp(0, 255) as u8`
/// - `<percentage>` 0%..=100% → `expect_percentage` は `0%`→0.0 / `100%`→1.0
///   の unit_value を返すため [`clamp_unit_f32`] (`round(v * 255).clamp(0, 255)`)
///   で `u8` へ mapping
/// - `<alpha-value>` は `<number>` 0..=1 または `<percentage>` 0%..=100% —
///   どちらも [`clamp_unit_f32`] で単一 formula に統合
///
/// # Non-goals
///
/// - Modern (space + slash) syntax `rgb(R G B / A)` は本 task 対象外。legacy
///   と modern の mix は spec で禁止だが、本 helper は最初の channel の直後で
///   `expect_comma` を要求するため modern syntax は fall-through で reject。
/// - Fractional number channel (`rgb(127.5, 0, 0)`) は spec grammar 上 valid
///   だが、`expect_integer` (整数 `int_value` 必須) を採用しているため drop
///   — 未実装 (将来対応)、future task で `<number>` に緩める余地。
/// - Alpha の `none` component は modern syntax でのみ許容 — 本 task 対象外。
fn parse_rgb_function<'i>(input: &mut Parser<'i, '_>) -> Result<CssColor, ParseError<'i, ()>> {
    // 1st channel: try percentage first、fail → integer number。
    // 成功した variant が以降 2 channel の kind を固定する。
    let (r, is_pct) = if let Ok(pct) = input.try_parse(|i| i.expect_percentage()) {
        (clamp_unit_f32(pct), true)
    } else {
        (clamp_channel(input.expect_integer()?), false)
    };
    input.expect_comma()?;
    let g = parse_rgb_channel(input, is_pct)?;
    input.expect_comma()?;
    let b = parse_rgb_channel(input, is_pct)?;
    // 4 番目 comma がある場合のみ alpha を parse。無ければ opaque (a=255)。
    // `rgba(...)` name 側で alpha 必須にしない (spec §5.1 alias 規定)。
    let a = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_alpha_value(input)?
    } else {
        255
    };
    Ok(CssColor { r, g, b, a })
}

/// legacy rgb() の 2 番目 / 3 番目 channel を parse する。1 番目 channel で
/// 決定した `is_pct` kind に沿って `<number>` / `<percentage>` のどちらかを
/// hard-expect し、mix (`rgb(255, 50%, 0)` / `rgb(50%, 255, 0)`) は Err で
/// 弾く (spec §5.1: "mixing types isn't allowed")。
fn parse_rgb_channel<'i>(
    input: &mut Parser<'i, '_>,
    is_pct: bool,
) -> Result<u8, ParseError<'i, ()>> {
    if is_pct {
        Ok(clamp_unit_f32(input.expect_percentage()?))
    } else {
        Ok(clamp_channel(input.expect_integer()?))
    }
}

/// `<alpha-value>` (CSS Color 4 §5.1 grammar: `<number> | <percentage>`)。
/// `<number>` は 0..=1、`<percentage>` は 0%..=100% で、どちらも clamp 後
/// [`clamp_unit_f32`] で 0..=255 の `u8` に mapping する
/// (`expect_percentage` の unit_value は既に 0..=1 化されているため同一 formula)。
///
/// try_parse で percentage を先行させる — `<percentage>` は Token::Percentage、
/// `<number>` は Token::Number で orthogonal だが、percentage-first は
/// [`parse_rgb_function`] の 1 番目 channel と対称の順序 (mix reject と同じ
/// pattern で読める)。
fn parse_alpha_value<'i>(input: &mut Parser<'i, '_>) -> Result<u8, ParseError<'i, ()>> {
    if let Ok(pct) = input.try_parse(|i| i.expect_percentage()) {
        Ok(clamp_unit_f32(pct))
    } else {
        Ok(clamp_unit_f32(input.expect_number()?))
    }
}

fn clamp_channel(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// `font-family: <family-name>#` を parse する。
///
/// comma-separated な family-name の list。各 family-name は quoted string
/// (`"Times New Roman"`) か、unquoted identifier の連続 (`Times New Roman` =
/// 3 ident が空白区切りで 1 family、CSS4 で有効) のいずれか。
///
/// 末尾で comma が続かなければ loop を止め、残り input (`!important` 等) は
/// 手を付けずに downstream (caller の `parse_important` / `expect_exhausted`)
/// に委ねる — `!` を garbage として拒否しないための Finding 3 対応。
fn parse_font_family(input: &mut Parser<'_, '_>) -> Option<Vec<Atom>> {
    let mut families = Vec::new();
    loop {
        // Try quoted string first (e.g. "Times New Roman")
        let family = if let Ok(s) = input.try_parse(|i| i.expect_string().cloned()) {
            Atom::from(s.as_ref())
        } else if let Ok(first) = input.try_parse(|i| i.expect_ident().cloned()) {
            // Unquoted ident sequence: `Times New Roman` = 3 idents joined by space
            let mut buf = first.as_ref().to_string();
            while let Ok(next) = input.try_parse(|i| i.expect_ident().cloned()) {
                buf.push(' ');
                buf.push_str(next.as_ref());
            }
            Atom::from(buf.as_str())
        } else {
            return None;
        };
        families.push(family);
        // Consume comma or stop (leaves remaining input alone)
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }
    if families.is_empty() {
        None
    } else {
        Some(families)
    }
}

/// `<length>` / `<length-percentage>` の共通 parser。1 token を consume する。
///
/// Grammar reference: CSS Values 4 §6 <https://www.w3.org/TR/css-values-4/#lengths>
/// / §5.5 <https://www.w3.org/TR/css-values-4/#percentages>.
///
/// # Mode selector
///
/// `allow_percentage` は `%` (`Token::Percentage`) token の受理有無のみを
/// 分岐する — dimension unit (`px` 等) の受理集合は分岐に依存しない (下の
/// `Token::Dimension` match arm 参照、両 mode で同一集合を受理する)。
/// - `false` → `<length>` mode: `%` を受理しない。
/// - `true` → `<length-percentage>` mode: `%` も受理する。
///
/// **受理 / 未対応 unit の一覧は本節では列挙しない** — 下の
/// `Token::Dimension` match arm (module doc 冒頭の「該 arm を single source
/// of truth として扱う」と同じ convention、`_` arm 直前 comment が未対応側の
/// 代表例を持つ) と `parse_length_value_rejects_unsupported_unit` test が
/// canonical。**ここに一覧を書き足す運用は受理 unit が増えるたびに drift
/// した** (実際に `cm` を筆頭に、`ch` / `ex` / `ic` /
/// `mm` / `in` / `pc` / `Q` / `lh` / `rlh` の一括拡張のたびに本節の一覧全体が
/// stale 化していた)。
///
/// # Unitless zero
///
/// CSS Values 3 §5 "Distance Units: the `<length>` type"
/// <https://www.w3.org/TR/css-values-3/#lengths> verbatim: "For zero lengths
/// the unit identifier is optional (i.e. can be syntactically represented as the
/// `<number>` 0)." — bare `0` (Token::Number, value == 0.0) を [`Length::Px`]
/// `(0.0)` として受理する (mode 非依存: `<length>` / `<length-percentage>` 両方)。
/// 非零 unitless number (`5`, `-1` etc.) は grammar 上 `<length>` にならないため
/// 引き続き drop する (`== 0.0` guard で判定)。
///
/// 同 spec §5 clause 2: "if a 0 could be parsed as either a `<number>` or a
/// `<length>` in a property (such as line-height), it must parse as a `<number>`"
/// — [`parse_line_height`] は本 helper より先に `expect_number` branch を試すため
/// 該当分岐は `LineHeight::Number(0.0)` を返し、本 helper 経由の `Length::Px(0.0)`
/// には落ちない (spec-required disambiguation)。
///
/// # Sign / range
///
/// 本 helper は sign / range check を行わない — property ごとに要件が異なるため
/// (padding は non-negative、margin は negative 許容、etc.)。caller 側で
/// post-filter する ([`parse_font_size`] は **全 [`Length`] variant** の payload に
/// 対して `>= 0.0` を確認する)。
///
/// # `allow_percentage=true` の caller
///
/// forward-provisioning として導入した mode だが、現在は 7 caller が使用する:
/// [`parse_margin_side`] / [`parse_padding_side`] / [`parse_width`] /
/// [`parse_height`] / [`parse_line_height`] / [`parse_font_size`] /
/// [`parse_text_indent`]。いずれも
/// grammar が spec で `<length-percentage>` を含む
/// (`font-size` は元は `<length>` 限定だったが後に拡張)。共通 helper 化により
/// 重複 dimension unit dispatch を回避している。
///
/// `allow_percentage=false` (= `<length>` mode) の caller は
/// [`parse_border_width_side`] / [`parse_letter_or_word_spacing`] —
/// 前者は CSS Backgrounds 3 §3.3 の `<line-width>` grammar が `<percentage>`
/// を含まないため、後者は CSS Text 3 §7.1/§7.2 の `letter-spacing` /
/// `word-spacing` grammar が共に "Percentages: N/A" と明記するため。
///
/// # Percentage overflow
///
/// `Token::Percentage.unit_value` は f64→f32 変換済 (cssparser 0.37
/// tokenizer が `value / 100.0` を emit) だが、[`Length::Percent`] は
/// authored number (`50%` → `50.0`) を保持する設計のため、本 helper 側で
/// `unit_value * 100.0` の逆変換を行う。`unit_value` 自体が f32 有限範囲に
/// 収まっていても (例 `1e40%` → cssparser 側は `1e38` で有限)、この
/// ×100.0 の逆変換それ自体が f32 overflow を起こしうる (`1e38 * 100.0` は
/// f32 の有限範囲 `3.4028235e38` を超えて `+Inf`)。CSS Values 4 §5 "Range
/// Checking and Precision for Numeric Types"
/// <https://www.w3.org/TR/css-values-4/#numeric-types> の "it must be
/// converted to the closest value supported by the implementation" に従い、
/// `±Inf` になった場合のみ、符号を保持しつつ `f32::MAX` へ寄せる。
///
/// 既存の sink-guard precedent (「guard は sink 境界に
/// 置く、parse/resolve 層には置かない」) はここには適用しない —
/// 本件は guard ではなく変換の正確さの問題
/// (specified 層の値そのものが CSS Values 4 §5 の要求から外れている)
/// であり、precedent とは別軸。`raikiri-dom::layout::sanitize_finite`
/// (resolve 後の geometry に対する sink guard) は本変更後も引き続き必要。
///
/// **`NaN` はこの saturation の対象外**。`is_finite()` は `NaN` に対しても
/// `false` を返すため、当初の実装は `NaN` も `±f32::MAX` へ saturate して
/// いたが、それは誤り: 例えば `0e999%` は cssparser 側の `0.0 * 10^999`
/// (`f64::powf` が `+Inf` を返す) で `NaN` になる、**真の数学的値は 0**
/// の入力であり、"closest value" は `f32::MAX` ではなく `0.0` である。
/// `sanitize_finite` (`raikiri-dom/src/layout.rs`) は `NaN` を既に `0.0`
/// として扱うため、ここで saturate せず `NaN` のまま通せば sink 側の
/// 既存契約と整合する。よって saturation の条件は `is_infinite()` に
/// 限定し、`NaN` は無変換で通す。
fn parse_length_value(input: &mut Parser<'_, '_>, allow_percentage: bool) -> Option<Length> {
    match input.next().ok()? {
        Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
            "px" => Some(Length::Px(*value)),
            "em" => Some(Length::Em(*value)),
            "rem" => Some(Length::Rem(*value)),
            "pt" => Some(Length::Pt(*value)),
            // Additional font-relative units (CSS Values 4 §6.1.1).
            // `ex`/`ch`/`ic` の real-metric variant は
            // style 層に font metrics が無いため常に spec fallback を使う
            // (`Length::Ex` / `Length::Ch` / `Length::Ic` の doc 参照)。
            "ex" => Some(Length::Ex(*value)),
            "rex" => Some(Length::Rex(*value)),
            "ch" => Some(Length::Ch(*value)),
            "rch" => Some(Length::Rch(*value)),
            "ic" => Some(Length::Ic(*value)),
            "ric" => Some(Length::Ric(*value)),
            // Additional absolute units (CSS Values 4 §6.2).
            // `unit` は `to_ascii_lowercase()` 済 —
            // `Q` トークンも `"q"` として届く。
            "cm" => Some(Length::Cm(*value)),
            "mm" => Some(Length::Mm(*value)),
            "q" => Some(Length::Q(*value)),
            "in" => Some(Length::In(*value)),
            "pc" => Some(Length::Pc(*value)),
            // `lh` / `rlh` (CSS Values 4 §6.1.1).
            // Accepted generally here for every consumer, `font-size` included
            // (moved out of `parse_font_size`'s former
            // post-filter — see that function's doc "`lh` / `rlh` は受理し、
            // 親基準で解決する" section for the self-reference resolution).
            "lh" => Some(Length::Lh(*value)),
            "rlh" => Some(Length::Rlh(*value)),
            // (b) 非対応 — viewport-relative unit (`vw`/`vh`/…) と
            // `cap`/`rcap` は未対応、silent drop。両者とも specified 層だけ
            // では正しく resolve できない (viewport size / font ascent が
            // style 層に存在しない) ため follow-up task へ切り出し済。
            //
            // この arm はそれ以外の全 unrecognized unit (例:
            // container-query unit `cqw`/`cqh`/`cqi`/`cqb`/`cqmin`/`cqmax` —
            // CSS Contain 3 §6 <https://www.w3.org/TR/css-contain-3/#container-lengths>、
            // container size も viewport size 同様 style 層に存在しない)
            // も等しく drop する。本 comment が「未対応 unit の一覧」の
            // canonical source になった以上、この一覧を書き足す形の
            // 重複記述はしないこと。
            _ => None,
        },
        Token::Percentage { unit_value, .. } if allow_percentage => {
            // authored-number 逆変換 + overflow saturation: 上の
            // "# Percentage overflow" section 参照。
            let percent = *unit_value * 100.0;
            Some(Length::Percent(if percent.is_infinite() {
                f32::MAX.copysign(percent)
            } else {
                percent
            }))
        }
        // CSS Values 3 §5 unitless-zero clause (doc "# Unitless zero" 参照)。
        Token::Number { value, .. } if *value == 0.0 => Some(Length::Px(0.0)),
        _ => None,
    }
}

/// [`Length`] の authored payload (`f32`) を variant によらず取り出す。
///
/// `parse_width` / `parse_font_size` / `parse_padding_side` /
/// `parse_border_width_side` / `parse_height` / `parse_line_height` /
/// [`parse_non_negative_length`] は grammar
/// の `[0,∞]` non-negative constraint を "全 variant の payload を取り出して
/// `>= 0.0` を確認" という同一 pattern で parse-time enforce する
/// (`parse_length_value` 自体は sign check しない仕様 — 同関数の "Sign / range"
/// doc 参照)。
///
/// 本 helper 導入前は 6 call site それぞれが `Length::Px(v) | Length::Em(v) |
/// … => v` の OR-pattern を個別に持っていた。
/// [`Length`] が 5 → 16 variant に増える際、6 site 全てを手で拡張すると
/// 1 か所でも変数を書き漏らした variant が非負チェックを素通りする
/// (実際 2 site — `parse_border_width_side` / `parse_line_height` — は
/// `_ => None` catch-all を持っていたため、拡張漏れは compile error にならず
/// 黙って新 unit を reject し続ける fail-quiet になっていた)。本 helper は
/// **exhaustive match を 1 か所に集約**することで、新 variant 追加時に
/// compile error で全 call site の見直しを強制する —
/// 「拡張のたびに N site 分の負債が乗る」パターンをこの関数の
/// 内側だけに閉じ込める。
fn length_payload(length: Length) -> f32 {
    match length {
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

/// `<length [0,∞]>` — [`parse_length_value`] with `allow_percentage=false`
/// (no `<percentage>` alternative), then the same `[0,∞]` non-negative
/// filter [`length_payload`]'s doc describes (`(length_payload(length) >=
/// 0.0).then_some(length)`), so [`crate::page`]'s `size` descriptor parser
/// (the 7th caller in [`length_payload`]'s roster) doesn't have to
/// re-enumerate [`Length`] variants by hand.
///
/// `pub(crate)` for the one caller outside this module: [`crate::page`]'s
/// `size` descriptor parser. CSS Paged Media Level 3 §7.1 "Page size: the
/// size property" (<https://www.w3.org/TR/css-page-3/#page-size-prop>)
/// grammar is `<length>{1,2} | auto | …` — `<length>`, not
/// `<length-percentage>` — and states "Negative lengths are illegal", the
/// same `[0,∞]` shape this crate's box properties already enforce via
/// [`length_payload`].
pub(crate) fn parse_non_negative_length(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, false)?;
    (length_payload(length) >= 0.0).then_some(length)
}

/// `text-indent`'s `<length-percentage>` component.
///
/// grammar reference: CSS Text 3 §8.1
/// <https://www.w3.org/TR/css-text-3/#text-indent-property>, whose full
/// grammar is `<length-percentage> && hanging? && each-line?` — this helper
/// covers only the `<length-percentage>` part ([`PropertyValue::TextIndent`]
/// doc's "Scope carving" section).
///
/// No non-negative filter, unlike [`parse_padding_side`] — the spec places no
/// `[0,∞]` restriction on this grammar (negative indents are valid, sibling
/// [`parse_margin_side`] applies the same "no filter" treatment for the same
/// reason its own grammar allows negative values).
fn parse_text_indent(input: &mut Parser<'_, '_>) -> Option<Length> {
    parse_length_value(input, true)
}

/// `<length-percentage> | auto` の共通 parser — margin longhand 1 side 分。
///
/// grammar reference: CSS Box 3 §3.1
/// <https://www.w3.org/TR/css-box-3/#margin-physical> "Value:
/// `<length-percentage> | auto`"。
///
/// # Order of alternative
///
/// `auto` ident branch を **先に** try_parse する — [`parse_length_value`] は内部で
/// `input.next()` を unconditional に消費 (fail 時も token を戻さない) するため、
/// naive な "try length first, then auto" だと `margin: auto` の `auto` ident
/// が length parser で drop され後段の auto match が届かない。try_parse で
/// checkpoint 経由の rewind を確保する (sibling: [`parse_content_list_items`] の
/// bare `<string>` literal 分岐と同 pattern)。
///
/// `expect_ident_matching` は ASCII case-insensitive (cssparser 慣行、既存
/// `counter_reset_is_case_insensitive_on_none` test が挙動を pin) なので
/// `AUTO` / `Auto` も透過的に受理される。
fn parse_margin_side(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    parse_length_value(input, true).map(LengthOrAuto::Length)
}

/// `margin: <'margin-top'>{1,4}` shorthand — 1-4 value expansion 実装。
///
/// grammar reference: CSS Box 3 §3.2
/// <https://www.w3.org/TR/css-box-3/#margin-shorthand>。
///
/// # Expansion rules (spec verbatim, §3.2)
///
/// "If there is only one component value, it applies to all sides. If there
/// are two values, the top and bottom margins are set to the first value and
/// the right and left margins are set to the second. If there are three
/// values, the top is set to the first value, the left and right are set to
/// the second, and the bottom is set to the third. If there are four values
/// they apply to the top, right, bottom, and left, respectively."
///
/// # Trailing garbage handling
///
/// 5+ value (`margin: 10px 20px 30px 40px 50px`) は本 helper では 4 value 消費
/// して残り 1 token を unconsumed で return する。caller の
/// [`mod@crate::rule`] の `DeclParser` の
/// [`cssparser::DeclarationParser::parse_value`]
/// impl が `expect_exhausted` で余剰 token を
/// 検知して declaration ごと drop する (既存 [`parse_font_family`] 系と同じ
/// 責務分担、`rejects_extra_length_after_font_size` 系 test で pattern を pin)。
fn parse_margin_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<LengthOrAuto>> {
    let v1 = parse_margin_side(input)?;
    // 2nd value 不在 → 1 value case: 全 4 side に spread (§3.2 "If there is only
    // one component value, it applies to all sides")。
    let Some(v2) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides::all(v1));
    };
    // 3rd 不在 → 2 value case: top/bottom = 1st, right/left = 2nd。
    let Some(v3) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v1,
            left: v2,
        });
    };
    // 4th 不在 → 3 value case: top = 1st, right/left = 2nd, bottom = 3rd。
    let Some(v4) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v3,
            left: v2,
        });
    };
    // 4 values: clockwise from top (top, right, bottom, left)。5th 以降は
    // 本 helper では消費せず、caller の `expect_exhausted` で drop される
    // (property.rs test `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`
    //  で parse_value 単体挙動、rule.rs test `margin_shorthand_five_values_declaration_dropped`
    //  で end-to-end drop を pin)。
    Some(Sides {
        top: v1,
        right: v2,
        bottom: v3,
        left: v4,
    })
}

/// `width: auto | <length-percentage [0,∞]>` を parse する。
///
/// grammar reference: CSS Sizing 3 §3.1.1
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties> "Value:
/// `auto | <length-percentage [0,∞]> | min-content | max-content |
/// fit-content(<length-percentage>)`"、"Initial: auto"、"Inherited: no"。
///
/// # 非対応 (spec-valid、将来対応)
///
/// `min-content` / `max-content` / `fit-content()` は intrinsic sizing keyword
/// で未実装 — 本 helper では受理せず自然に `None` に落ちる (`auto` ident
/// 分岐で `expect_ident_matching("auto")` が fail、続く `parse_length_value` が
/// keyword / function token を Dimension / Percentage arm fall-through で drop)。
/// 負値 (`width: -10px`) は spec grammar `[0,∞]` violation として drop する。
///
/// # Order of alternatives
///
/// [`parse_margin_side`] と同 pattern の "auto ident branch 先行 try_parse":
/// [`parse_length_value`] は内部で `input.next()` を unconditional に消費する
/// (fail 時も token を戻さない) ため、naive な "try length first, then auto"
/// だと `width: auto` の `auto` ident が length parser で drop され後段の auto
/// match が届かない。`try_parse` で checkpoint 経由の rewind を確保する。
///
/// # Non-negative constraint
///
/// [`parse_padding_side`] と同 pattern の全 [`Length`] variant OR-pattern check —
/// spec `[0,∞]` の closed interval を parse-time enforce (Verification #4:
/// `width: -10px` → `None` → declaration drop)。padding と shape は同じだが
/// `auto` keyword 分岐が先行する (padding は `auto` を受理しない grammar
/// `<length-percentage [0,∞]>` のみ)。
fn parse_width(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    // spec §3.1.1 grammar `<length-percentage [0,∞]>` の non-negative constraint
    // (padding と同 pattern、`length_payload` 経由の
    // precedent)。
    (length_payload(length) >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `font-size: <absolute-size> | <relative-size> | <length-percentage [0,∞]> |
/// math` を parse する。
///
/// Grammar (CSS Fonts 4 §2.5 "Font size: the font-size property"
/// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>):
/// `<absolute-size> | <relative-size> | <length-percentage [0,∞]> | math`。
///
/// - `<absolute-size>` (`xx-small` … `xxx-large`、`medium`) — [`parse_font_size_keyword`]
///   が §2.5.1 の scaling-factor table を `medium` = 16px 基準で解決し、
///   [`PropertyValue::FontSize`] (`Length::Px`) を返す。
/// - `<relative-size>` (`larger` / `smaller`) — 継承先依存のため
///   [`PropertyValue::FontSizeRelative`] を返し、解決は
///   [`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]
///   に委ねる (詳細は同 variant の doc)。
/// - `<length-percentage [0,∞]>` — 本関数の後半、[`parse_length_value`] 経由。
/// - `math` — 未実装 (MathML scaling algorithm が丸ごと未対応) として
///   `None` に落とす。
///
/// # ident 分岐を先に `try_parse` する理由
///
/// `<absolute-size>` / `<relative-size>` / `math` はいずれも単一 ident token。
/// [`parse_margin_side`] の `auto` 分岐と同じ pattern — [`parse_length_value`]
/// は内部で `input.next()` を unconditional に消費するため、ident 分岐は
/// checkpoint 経由の rewind (`try_parse`) で先に試す必要がある。
///
/// # `em` / `rem` / `%` / `pt` を受理するようになった経緯
///
/// 以前は `px` 以外を post-filter で drop していた。理由は「font-size
/// context resolve 未実装」であり、その resolve が後に実装された —
/// cascade が phase 2 で
/// [`crate::resolve::resolve_font_size`] を呼び、`em` は**親の** computed
/// font-size、`rem` は root element の computed font-size、`%` は同 §2.5
/// "Percentages: refer to parent element's font size" に従って絶対化する。
/// したがって drop の理由が消えたので受理する。
///
/// # Non-negative constraint
///
/// grammar の `[0,∞]` を parse-time enforce する。[`parse_padding_side`] /
/// [`parse_width`] と同じ [`length_payload`] 経由の全 [`Length`] variant check
/// — `-5px` だけでなく `-50%` / `-1em` も drop する。`<absolute-size>` /
/// `<relative-size>` は grammar 上そもそも符号を持たないので本 constraint の
/// 対象外 (ident 分岐は `parse_length_value` に達する前に return する)。
///
/// # `lh` / `rlh` は受理し、親基準で解決する
///
/// [`Length::Lh`] doc の「自己参照」節: CSS Values 4 §6.1.1 は `lh`/`rlh` が
/// `line-height` **または font-\* property** の値として、それが指す要素自身に
/// 使われたときは親 (または「親が無ければ initial values」) の line-height /
/// font metrics を基準にする、と規定する。`font-size` はまさにその
/// font-\* property であり、grammar 上 `lh`/`rlh` を排除する根拠は無い
/// (CSS Fonts 4 の `font-size` grammar `<absolute-size> | <relative-size> |
/// <length-percentage [0,∞]>` の `<length-percentage>` は `<length>` を含み、
/// CSS Values 4 §6.1.1 の `<length>` production は `lh`/`rlh` を除外しない)。
///
/// 当初は、この解決 (「親の computed line-height」を
/// font-size 解決の基準として渡す) が `line-height`
/// (`finalize`/`finalize_as_root` が既に持つ `parent: &ComputedValues` を
/// そのまま使える) より高コストに見えたため drop していたが、実際に実装した
/// ところコストは局所的だった — [`crate::resolve::resolve_font_size`] の
/// `self_reference_basis` 引数、および [`crate::specified::SpecifiedValues::finalize`]
/// 内の 2, 3 行の並べ替えで足りる (`parent` は本関数の呼び出しに入る前に
/// tree walk で既に確定済みのため、cross-node な phase 順序の変更は不要 —
/// [`mod@crate::resolve`] module doc の「想定される 4 段階」節参照)。
fn parse_font_size(input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    if let Ok(ident) = input.try_parse(|i| i.expect_ident().cloned()) {
        return parse_font_size_keyword(&ident);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(PropertyValue::FontSize(length))
}

/// `<absolute-size>` / `<relative-size>` / `math` の ident 部分を parse する
/// ([`parse_font_size`] の helper)。
///
/// # `<absolute-size>` scaling-factor table
///
/// CSS Fonts 4 §2.5.1 "Absolute Size Keyword Mapping Table"
/// <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping> の表をそのまま
/// 写す (`resolve_relative_weight` の "算術式で書いてはいけない"
/// 方針と同じ理由 — 分数のまま持つことで丸め誤差の議論を spec 引用だけで
/// 閉じられる)。`medium` は raikiri の固定基準
/// ([`crate::computed::INITIAL_FONT_SIZE_PX`] = 16px、
/// [`crate::specified::SpecifiedValues::initial`] doc 参照) を再利用する:
///
/// | keyword | xx-small | x-small | small | medium | large | x-large | xx-large | xxx-large |
/// |---|---|---|---|---|---|---|---|---|
/// | factor | 3/5 | 3/4 | 8/9 | 1 | 6/5 | 3/2 | 2/1 | 3/1 |
///
/// 同 §の "an UA applying these guidelines should nevertheless avoid creating
/// font sizes of less than 9 device pixels per EM unit" は "should" (RFC 2119
/// 弱勧告)。本 table の最小値は `xx-small` = `16 * 3/5 = 9.6px` で、9px の
/// 下限を上回るため clamp は不要 (実装しない理由は「未対応」ではなく
/// 「`medium` = 16px 基準ではこの guideline を最初から満たす」こと)。
///
/// # `<relative-size>`
///
/// [`RelativeFontSize`] doc 参照。
///
/// # `math`
///
/// 未実装 (spec-valid だが対応外)。
fn parse_font_size_keyword(ident: &str) -> Option<PropertyValue> {
    const MEDIUM_PX: f32 = crate::computed::INITIAL_FONT_SIZE_PX;
    let px = match ident.to_ascii_lowercase().as_str() {
        "xx-small" => MEDIUM_PX * (3.0 / 5.0),
        "x-small" => MEDIUM_PX * (3.0 / 4.0),
        "small" => MEDIUM_PX * (8.0 / 9.0),
        "medium" => MEDIUM_PX,
        "large" => MEDIUM_PX * (6.0 / 5.0),
        "x-large" => MEDIUM_PX * (3.0 / 2.0),
        "xx-large" => MEDIUM_PX * (2.0 / 1.0),
        "xxx-large" => MEDIUM_PX * (3.0 / 1.0),
        "larger" => return Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger)),
        "smaller" => return Some(PropertyValue::FontSizeRelative(RelativeFontSize::Smaller)),
        // `math` はここに落ちる (spec-valid だが未対応)。
        // 未知 ident も同じく drop。
        _ => return None,
    };
    Some(PropertyValue::FontSize(Length::Px(px)))
}

/// `padding-{top,right,bottom,left}` の single-side value を parse する。
///
/// grammar: `<length-percentage [0,∞]>` (CSS Box 3 §4.1
/// <https://www.w3.org/TR/css-box-3/#padding-physical>)。spec verbatim:
/// "Negative values for padding properties are invalid." — 負値は grammar 違反
/// として declaration ごと drop する。
///
/// # 実装 note
///
/// 1. [`parse_length_value`] を `allow_percentage=true` で呼ぶ (grammar が
///    `<length-percentage>`)。dimension 未対応 unit / `auto` keyword / non-numeric
///    token は同 helper が `None` に落とす (font-size 経路と同 pattern)。
/// 2. 全 [`Length`] variant の payload ([`length_payload`] 経由) に対し
///    `>= 0.0` を確認、負値は `None` 返し (`Percent(-10.0)` = `-10%` も含む —
///    Verification #5 で pin)。
///
/// # Sibling pattern
///
/// [`parse_font_size`] の `<length-percentage>` 分岐 (ident 分岐で `None` に
/// なった後の tail) と同形 — どちらも `allow_percentage=true` で
/// [`parse_length_value`] を呼び、[`length_payload`] で全 [`Length`] variant の
/// payload を抽出して `>= 0.0` を post-filter する (tail 部分の body は
/// identical)。`parse_font_size` は後に `<absolute-size>` /
/// `<relative-size>` / `math` の ident 分岐 (`parse_font_size_keyword`) が
/// 前段に付いたため関数全体としては同形ではなくなったが、この tail 部分の
/// ロジックは identical。
///
/// 両者が非対称だった時期 (font-size が `<length>` px-only scope で、padding
/// だけが `<length-percentage>` の 5 variant を受けていた頃) の記述は
/// font-relative unit 対応で解消済み。
fn parse_padding_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid."。
    (length_payload(length) >= 0.0).then_some(length)
}

/// `padding: <'padding-top'>{1,4}` shorthand を [`Sides<Length>`] に expand する。
///
/// CSS Box 3 §4.2 <https://www.w3.org/TR/css-box-3/#padding-shorthand> の
/// 1-4 value expansion (逐語引用ではないので `verbatim` 表記は使わない):
///
/// - 1 value: all 4 sides = value
/// - 2 values: top/bottom = 1st, left/right = 2nd
/// - 3 values: top = 1st, left/right = 2nd, bottom = 3rd
/// - 4 values: top / right / bottom / left (clockwise from top)
///
/// # Robustness
///
/// - 5 個目以降の value は本関数では consume せず leftover として残す →
///   caller (`rule.rs::DeclParser`) の `expect_exhausted` が declaration
///   ごと drop する (`padding: 1px 2px 3px 4px 5px` → invalid, drop)。
/// - 0 value (input が empty) は 1st `parse_padding_side` が `None` を返し
///   全体 `None` propagate。
/// - 各 value の non-negative constraint は [`parse_padding_side`] が個別に
///   enforce (負値混じり `padding: 10px -5px` → 2nd で `None`、全体 drop)。
fn parse_padding_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Length>> {
    // 1st value 必須。無ければ全体 drop (0-value form は grammar 違反)。
    let v1 = parse_padding_side(input)?;
    // 2-4 value は sequential `try_parse` で optional 取得。`try_parse` は
    // 失敗時に parser position を rewind するため、前段 None 時にも下段の
    // try_parse は同 token を再 read → 同 fail、guard 不要 (自然 short-circuit)。
    let v2 = input.try_parse(parse_padding_side_res).ok();
    let v3 = input.try_parse(parse_padding_side_res).ok();
    let v4 = input.try_parse(parse_padding_side_res).ok();
    // spec (CSS Box 3) §4.2 1-4 value expansion (code, not a spec quote):
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// [`parse_padding_side`] の `Result` 版 — `try_parse` は closure 内で
/// `Result` を要求するため wrapper 化。
fn parse_padding_side_res<'i>(input: &mut Parser<'i, '_>) -> Result<Length, ParseError<'i, ()>> {
    parse_padding_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `border-width` の `medium` keyword (= spec 上の initial value) に対応する
/// px 値。
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) 本文 verbatim:
/// "The thin, medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." — `font-size` の `medium` (UA 裁量、
/// [`crate::computed::INITIAL_FONT_SIZE_PX`] 参照) とは異なり、こちらは
/// **spec が規範的に定める厳密値**であり、raikiri の選択ではない。
///
/// **非 test code で `3.0` (border-width `medium`) を書く単一 source**
/// ([`INITIAL_FONT_SIZE_PX`](crate::computed::INITIAL_FONT_SIZE_PX)
/// と同じ pattern) — [`parse_border_width_side`] の `medium` keyword 分岐と、
/// [`parse_border_shorthand`] の width 省略成分デフォルトが参照する。
/// [`crate::specified::INITIAL_BORDER`] の `width` field も本 const を参照する
/// (property → specified の既存依存方向 — `specified` は既に
/// `use crate::property::{..}` で本 module の型を import している。逆方向の
/// edge を作らないこと)。
///
/// 一方「initial の border-width が **3px そのものである**」ことの pin は
/// test 側が literal で持つ。**これらを「一貫性のため」本 const への参照に
/// 書き換えてはならない** — 全体が自己参照になり、const の誤編集を何も
/// 検出できなくなる ([`INITIAL_FONT_SIZE_PX`](crate::computed::INITIAL_FONT_SIZE_PX)
/// doc と同じ理由)。該当 test は本 const を `5.0` 等に摂動すれば列挙できる
/// (lib test が fail-fast して doctest section まで到達しないので、
/// `cargo test -p raikiri-style` と `--doc` を別々に走らせること)。
///
/// **`thin` (1px) / `thick` (5px) は const 化しない** — 同じ規範文の 3 keyword
/// の残り 2 つだが、[`parse_border_width_side`] の keyword match 内 1 箇所ずつ
/// にしか現れず (border shorthand の省略成分デフォルトは spec 上も `medium`
/// のみが initial value)、複数 site 間の drift 余地がない。const 化するのは
/// 独立 literal が 2 箇所以上に分散している `medium` のみで十分
/// (`medium` の重複を解消する scope、thin/thick への一般化は
/// non-goal)。
pub(crate) const BORDER_WIDTH_MEDIUM_PX: f32 = 3.0;

/// `border-{top,right,bottom,left}-width` の single-side value を parse する。
///
/// Grammar: `<line-width>` = `<length [0,∞]> | thin | medium | thick`
/// (CSS Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。
/// **`<percentage>` は含まれない** — padding とは違う (
/// `parse_length_value(input, false)` = `<length>` mode を渡す)。
///
/// # Keyword mapping (spec 規定値)
///
/// spec §3.3 は 3 keyword を normative に規定する — verbatim: "The thin,
/// medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." 対応表:
/// - `thin`   → `Length::Px(1.0)`
/// - `medium` → `Length::Px(3.0)` (initial value)
/// - `thick`  → `Length::Px(5.0)`
///
/// UA 裁量ではなく spec 規定の equivalence なので、cleanroom 制約下でも
/// そのまま採用できる (Chromium / Firefox / WebKit の実装とも一致)。
///
/// # Sign / range
///
/// spec `<length [0,∞]>` の non-negative 制約は本 helper が enforce する
/// (負値 → `None` = declaration drop)。sibling [`parse_padding_side`] と同じ
/// post-filter pattern だが、`Length::Percent` variant は生成されない
/// (`allow_percentage=false` により Percentage token 自体が reject される)。
///
/// # Non-goals
///
/// - **(a) spec-invalid → drop**: 負値 (`-1px`)、未知 keyword (`fat` 等)、
///   spec-invalid unit (`%` は grammar に含まれない → drop)。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (将来対応)、silent drop。
fn parse_border_width_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    // 1. keyword branch (thin / medium / thick) を先に try — `parse_length_value`
    //    は unconditional に token を consume するため、`try_parse` で rewind を
    //    確保する必要がある (sibling `parse_margin_side` の `auto` branch と同
    //    pattern)。
    let keyword = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.clone();
        match ident.to_ascii_lowercase().as_str() {
            "thin" => Ok(Length::Px(1.0)),
            "medium" => Ok(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
            "thick" => Ok(Length::Px(5.0)),
            _ => Err(i.new_custom_error(())),
        }
    });
    if let Ok(l) = keyword {
        return Some(l);
    }
    // 2. `<length [0,∞]>` — allow_percentage=false で `<length>` mode
    //    (Percentage token は reject される、`<percentage>` は grammar 外)。
    let length = parse_length_value(input, false)?;
    // spec `<length [0,∞]>` の non-negative constraint — `length_payload` は
    // `Percent` も含む全 variant に対して定義されているが、`Percent` は
    // `allow_percentage=false` により本関数へは到達し得ない (unreachable、
    // dead value であって dead code ではない — helper 自体は border-width
    // 専用ではないため分岐を割ることはしない)。
    (length_payload(length) >= 0.0).then_some(length)
}

/// [`parse_border_width_side`] の `Result` 版 — `try_parse` は closure 内で
/// `Result` を要求するため wrapper 化 ([`parse_padding_side_res`] と同 pattern)。
fn parse_border_width_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_border_width_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `border-{top,right,bottom,left}-style` の single-side value を parse する。
///
/// Grammar: `<line-style>` = `none | hidden | dotted | dashed | solid | double
/// | groove | ridge | inset | outset` (CSS Backgrounds 3 §3.2
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>)。
/// ASCII case-insensitive で ident と照合 (37n sibling
/// [`parse_display`] / [`parse_text_align`] と同 flavor)。
///
/// # Non-goals
///
/// - **(a) spec-invalid → drop**: 未知 keyword (`wavy` 等) は silent drop。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
fn parse_border_style_side(input: &mut Parser<'_, '_>) -> Option<BorderStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(BorderStyle::None),
        "hidden" => Some(BorderStyle::Hidden),
        "dotted" => Some(BorderStyle::Dotted),
        "dashed" => Some(BorderStyle::Dashed),
        "solid" => Some(BorderStyle::Solid),
        "double" => Some(BorderStyle::Double),
        "groove" => Some(BorderStyle::Groove),
        "ridge" => Some(BorderStyle::Ridge),
        "inset" => Some(BorderStyle::Inset),
        "outset" => Some(BorderStyle::Outset),
        _ => None,
    }
}

/// `border: <line-width> || <line-style> || <color>` shorthand を parse する。
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>。
/// 4 side 全てに同一 [`Border`] を配る (`Sides::all`)。
///
/// # `||` (any-order) grammar semantics
///
/// spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> verbatim:
/// "A double bar (||) separates two or more options: one or more of them must
/// occur, in any order." — 本 shorthand では:
/// - each component は最大 1 回 (2 回目の同 slot ident は spec-invalid = drop)
/// - at least 1 component が必須 (0 component の empty `border:` は drop)
/// - order は自由 (`1px solid red` / `red 1px solid` / `solid 1px` 全て valid)
///
/// # Loop 実装
///
/// unfilled slot (width / style / color) を loop で peel:
/// 1. `try_parse` で order-independent に各 slot の parser を試す
/// 2. 埋まっている slot に match する token に当たったら stop (spec 準拠、caller
///    の `expect_exhausted` が leftover を drop する — 例: `border: 1px 2px` は
///    `1px` を width に置いた後 `2px` は既に埋まっている width slot に match して
///    stop、caller が leftover を検出して declaration ごと drop)
/// 3. 全 slot が埋まった or どの parser も match しなくなったら break
/// 4. 少なくとも 1 slot が埋まっていれば `Some`、0 slot なら `None`
///
/// # Initial value fill (省略成分)
///
/// spec §3.4 verbatim: "Omitted values are set to their initial values."
/// 各成分の initial:
/// - width 省略 → `Length::Px(3.0)` (medium initial)
/// - style 省略 → `BorderStyle::None` (initial、spec §3.2)
/// - color 省略 → [`BorderColor::CurrentColor`] (spec §3.1 initial、used-value
///   resolution は paint scope 責務)
///
/// # Non-goals (spec deviation 明示)
///
/// spec §3.4 では border shorthand が **border-image-* も reset** する (spec
/// verbatim: "The border shorthand also resets border-image to its initial
/// value.") が、本 crate は border-image を実装していないため
/// reset side effect を省略。
/// border-image longhand 実装時に統合する。
///
/// # Sibling pattern
///
/// [`parse_margin_shorthand`] / [`parse_padding_shorthand`] は `{1,4}`
/// multiplier (順序固定、side ごとに違う値) だが、本 shorthand は `||` (any-order、
/// side は 4 side 共通) — 別 pattern。sibling は `try_parse` 経由の rewind と
/// initial fill の点で共通 principle を持つ。
fn parse_border_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Border>> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    // `||` grammar: at least 1 component 必須、each component 最大 1 回、
    // order 自由。全 slot 満了 or 未 match token 到達で break。
    //
    // 各 iteration は "unfilled slot を順に try_parse、成功したら continue、
    // どの slot にも match しなかったら break" の shape。`continue` の前に slot
    // 満了 check を置くことで、埋まっている slot に対する 2 回目 (`border: 1px
    // 2px`) は自動的に fall-through して break (caller の `expect_exhausted` が
    // 残 token を検知して declaration drop)。
    loop {
        // 全 slot 満了 → break (leftover token は caller `expect_exhausted` が drop)
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }

        // width slot (unfilled のみ試行) — keyword (thin/medium/thick) と length
        // の両方を扱う helper を direct 呼ぶ。`try_parse` で失敗時 rewind。
        // `let Ok(..) = ..` の nested-if は clippy::collapsible-if を回避するため
        // let-chain (rust 1.88+) で 1 段化。
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }

        // style slot — ident が 10 keyword に match すれば埋める。`try_parse` で
        // 失敗時 rewind (width keyword `thin` / `medium` / `thick` を先に試すため
        // style keyword `none` / `solid` などとの間の ambiguity は無い、ident 集合が
        // disjoint)。
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }

        // color slot — `parse_border_color` を reuse。hex / named / rgb(a) /
        // transparent の全 alternative + `currentcolor` keyword (CSS Color 3
        // §4.4) を受理。4 longhand parse site (border-{top,right,bottom,left}-color)
        // と同じ helper を経由することで 37n sibling convention consistency を
        // 担保。
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }

        // どの unfilled slot にも match しなかった → 埋まっている slot に対する
        // 2 回目の指定 or 未知 token。break で loop 終了、caller の
        // `expect_exhausted` が leftover を drop する (`border: 1px 2px` →
        // `2px` は width slot 満了で本 fall-through 到達、declaration ごと drop)。
        break;
    }

    // spec `||` grammar: at least 1 component 必須。0 component (empty `border:`
    // or 未知 keyword only) は `None` = declaration drop。
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    // 省略成分は spec §3.4 の initial value で埋める。
    let border = Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)), // medium
        style: style.unwrap_or(BorderStyle::None),
        // §3.1 initial "currentcolor" — used-value resolution は paint scope
        // 責務。
        color: color.unwrap_or(BorderColor::CurrentColor),
    };
    Some(Sides::all(border))
}

/// `height: <length-percentage [0,∞]> | auto` を parse する。
///
/// grammar reference: CSS Sizing 3 §3.1.1 "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>。value
/// grammar は `auto | <length-percentage [0,∞]> | min-content | max-content |
/// fit-content(<length-percentage>)`、initial value `auto`、Inheritance `No`。
///
/// # Scope carving
///
/// - **(a) spec-invalid → drop**: 負値 (`height: -10px`) は grammar `[0,∞]` 違反、
///   全 [`Length`] variant の payload に対し `>= 0.0` post-filter で reject
///   ([`parse_padding_side`] の非負フィルタ pattern と同 shape)。
/// - **(b) 非対応 — 未対応 sizing keyword**: `min-content` /
///   `max-content` / `fit-content(<length-percentage>)` は現状 scope
///   外、silent drop (auto ident branch から外れる他 keyword は
///   `expect_ident_matching("auto")` が失敗 → length parser の Dimension /
///   Percentage arm でも受理されず None に落ちる)。
/// - **(b) 非対応 — CSS-wide keyword**: 未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical。同 ident 経路で他 keyword と同じく
///   落ちる。旧稿は `all` を CSS-wide keyword の一つとして誤って列挙していた
///   — `all` は shorthand property 名であって値ではなく、この訂正も
///   consolidation の一部)。
/// - **calc() / var()**: 未実装、本 task scope 外
///   (`Token::Function` は `parse_length_value` が Dimension / Percentage 以外を
///   silent drop)。
///
/// # Order of alternative (sibling: [`parse_margin_side`])
///
/// `auto` ident branch を **先に** try_parse する — [`parse_length_value`] は内部
/// で `input.next()` を unconditional に消費するため、naive な "try length first,
/// then auto" だと `height: auto` の `auto` ident が length parser で drop され
/// 後段の auto match が届かない。`try_parse` で checkpoint 経由の rewind を
/// 確保する ([`parse_margin_side`] と同 pattern — margin の grammar `<length-
/// percentage> | auto` と同 shape を LengthOrAuto payload で共有)。
///
/// `expect_ident_matching` は ASCII case-insensitive (cssparser 慣行、既存
/// `counter_reset_is_case_insensitive_on_none` test が挙動を pin) なので
/// `AUTO` / `Auto` も透過的に受理される。
///
/// # Non-negative filter (sibling: [`parse_padding_side`])
///
/// spec `<length-percentage [0,∞]>` (§3.1.1) の非負制約は [`length_payload`]
/// 経由で全 [`Length`] variant の payload に対し `>= 0.0` を確認 —
/// [`parse_padding_side`] の同名 pattern を踏襲 (`<length-percentage [0,∞]>`
/// grammar と非負フィルタが対応する 37n sibling)。`Percent(-10.0)` = `-10%` も
/// 含めて全 variant 経由で reject する。
fn parse_height(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    // spec §3.1.1: <length-percentage `[0,∞]`>。負値 → drop (parse_padding_side
    // の同 pattern)。
    (length_payload(length) >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `line-height: normal | <number> | <length-percentage>` を parse する。
///
/// Grammar: CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#line-height-property>) — value
/// alternative は 3 branch:
///
/// 1. `normal` keyword → [`LineHeight::Normal`]
/// 2. `<number [0,∞]>` bare number (Token::Number、unit なし) → [`LineHeight::Number`]
/// 3. `<length-percentage [0,∞]>` → [`LineHeight::Length`] with reused Length variant
///
/// # Number vs Length grammar distinction
///
/// spec は `<number>` と `<length-percentage>` を別 alternative として持つため
/// 1 token レベルで区別が要る (Token::Number = unitless / Token::Dimension =
/// unit-bearing / Token::Percentage)。unitless `1.5` と dimensioned `1.5em` を
/// 別 variant に mapping することで、下流 (paint) が unitless number の
/// spec special behavior "specified value を child が inherit する"
/// (§5.1 "When a child element inherits a computed value...") と、length の
/// 通常 resolve context を区別できる。
///
/// # Ordering
///
/// `normal` (`try_parse` + `expect_ident_matching`) → bare number
/// (`try_parse(|i| i.expect_number())` — Dimension/Percentage に対しては rewind
/// して失敗) → [`parse_length_value`] (`allow_percentage = true`)。この順で
/// `1.5` は Number branch、`1.5em` / `1.5px` / `150%` は Length branch に確定分岐。
///
/// # Non-negative
///
/// spec `[0,∞]` により全 branch で negative reject:
/// - Number branch: `n >= 0.0` guard、負なら `None` = declaration drop
/// - Length branch: 全 payload の inner f32 に `>= 0.0` guard、負なら drop
///
/// spec-invalid → drop: spec grammar が range を parse-time
/// で制約するため、reject 自体が spec 準拠。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (css-variables-and-math)、
///   silent drop
/// - **(a) spec-invalid → drop**: `<number>` / `<length-percentage>` の負値、
///   `auto` / `medium` 等 spec-invalid keyword は spec grammar 違反、drop
fn parse_line_height(input: &mut Parser<'_, '_>) -> Option<LineHeight> {
    // 1. `normal` keyword — spec initial value。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LineHeight::Normal);
    }
    // 2. bare `<number [0,∞]>` — Token::Number (unit なし)。
    //    Dimension (`1.5em`) / Percentage (`150%`) に対しては `expect_number` が
    //    Err を返し `try_parse` が rewind するため、Length branch へフォールスルー。
    //    Number token を commit した後は必ずここで確定させる (accept か drop):
    //    `try_parse` は `Ok` の path で cursor を戻さないため、外側 `&& n >= 0.0`
    //    で reject すると consumed cursor のまま Length branch に落ち、
    //    `line-height: -0.5 20px` が `20px` として silently accept される
    //    (spec-invalid CSS を通す correctness bug)。
    if let Ok(n) = input.try_parse(|i| i.expect_number()) {
        // spec `<number [0,∞]>` 違反 → declaration drop (Length branch へ落とさない)。
        return (n >= 0.0).then_some(LineHeight::Number(n));
    }
    // 3. `<length-percentage [0,∞]>` — helper で全 unit + `%` を受理、
    //    negative は post-filter で drop (helper 自体は sign check しない仕様、
    //    parse_length_value doc "Sign / range" 参照)。
    let l = parse_length_value(input, true)?;
    // spec `[0,∞]`: 負値は grammar 違反 → declaration drop。
    (length_payload(l) >= 0.0).then_some(LineHeight::Length(l))
}

/// `letter-spacing: normal | <length>` / `word-spacing: normal | <length>`
/// を parse する。両 property は grammar が完全に同型 (CSS Text 3 §7.2
/// <https://www.w3.org/TR/css-text-3/#letter-spacing-property> / §7.1
/// <https://www.w3.org/TR/css-text-3/#word-spacing-property>) なので 1
/// 関数を共有する ([`LengthOrNormal`] doc の reuse pattern 節参照)。
///
/// # Ordering
///
/// `normal` (`try_parse` + `expect_ident_matching`) → [`parse_length_value`]
/// (`allow_percentage = false`) — [`parse_line_height`] と同じ 2-branch
/// shape だが、`<number>` branch が無い (grammar 自体に `<number>`
/// alternative が無いため、CSS Values 3 §5 の number-vs-length
/// disambiguation は本 property には適用されない — bare `0` はそのまま
/// [`parse_length_value`] の unitless-zero clause 経由で `Length::Px(0.0)`
/// になる)。
///
/// # Percentage は非対応
///
/// 両 property とも spec が "Percentages: N/A" (word-spacing) /
/// "Percentages: n/a" (letter-spacing) と明記する — `allow_percentage =
/// false` により `5%` は `_ => None` (Percentage token に対する
/// `allow_percentage` guard 不成立) で drop される。
///
/// # Negative length は許容 (non-negative filter を掛けない)
///
/// [`parse_line_height`] / [`parse_font_size`] 等の `[0,∞]` callers とは
/// 異なり、本関数は [`length_payload`] による `>= 0.0` post-filter を
/// **意図的に行わない**。CSS Text 3 §7.2 (letter-spacing) / §7.1
/// (word-spacing) がいずれも "Values may be negative, but there may be
/// implementation-dependent limits." と明記するため — spec 自身が sign を
/// 制限していない ([`LengthOrAuto`] を使う `margin-*` と同じ扱い、`padding`
/// / `border-width` の non-negative constraint とは対照的)。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (css-variables-and-math)、
///   silent drop
/// - **(a) spec-invalid → drop**: `<percentage>`、`auto` 等 spec-invalid
///   keyword は spec grammar 違反、drop
fn parse_letter_or_word_spacing(input: &mut Parser<'_, '_>) -> Option<LengthOrNormal> {
    // 1. `normal` keyword — spec initial value、"Computes to zero"。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LengthOrNormal::Normal);
    }
    // 2. `<length>` — percentage 非対応 (`allow_percentage = false`)、sign は
    //    制限しない (上記 doc "Negative length は許容" 節)。
    parse_length_value(input, false).map(LengthOrNormal::Length)
}

/// `font-weight: <font-weight-absolute> | bolder | lighter` を parse する。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop>:
///
/// ```text
/// <font-weight-absolute> = [ normal | bold | <number [1,1000]> ]
/// ```
///
/// - `normal` = 400 / `bold` = 700 (spec §2.2 の keyword 定義)
/// - `bolder` / `lighter` は継承値依存の relative weight。parse 段では解けない
///   ため sentinel variant ([`FontWeightValue::Bolder`] /
///   [`FontWeightValue::Lighter`]) で保持し、[`crate::cascade::apply_value`]
///   が親の computed weight から解決する。
///
/// ASCII case-insensitive matching は CSS Values 3 §3.1 "Pre-defined Keywords"
/// <https://www.w3.org/TR/css-values-3/#keywords> 準拠 (37n sibling
/// `parse_display` / `parse_content_*` と同 convention)。
///
/// # Range (spec grammar)
///
/// spec §2.2: "Only values greater than or equal to 1, and less than or equal
/// to 1000, are valid, and all other values are invalid"。したがって `0` /
/// `1001` / `-100` の reject は **spec grammar そのもの** であり、stricter
/// policy ではない。範囲判定は **丸める前の指定値** に対して行う (spec の
/// "values" は author が書いた `<number>` を指すため、`0.6` や `1000.4` は
/// 丸めれば範囲内になるが invalid)。
///
/// # Fractional weight は丸めずそのまま保持する
///
/// **spec は fraction を落としてよいとは述べていない。** §2.2 の property table
/// は `Computed value: a number, see below` と規定し、§2.2.2 "Missing weights"
/// <https://www.w3.org/TR/css-fonts-4/#missing-weights> は "Fractional weights
/// are valid" と明言する。WPT `css/css-fonts/parsing/font-weight-computed.html`
/// の `test_computed_value('font-weight', '150.25')` (2-arg 形 = computed ==
/// specified) がこれを直接 pin している。
///
/// payload ([`FontWeightValue::Absolute`]) と
/// [`crate::computed::ComputedValues::font_weight`] は共に `f32` (以前は
/// `u16` だったが格上げ) なので、parse 時に整数化する必要が
/// ない — `<number>` の `value` をそのまま保持する。旧実装は computed side が
/// `u16` だったため round-half-away-from-zero で整数化しており、その丸めが
/// §2.2.1 "Relative Weights" relative-weight table の*行選択*を変える 2 次被害
/// があった (親 `font-weight: 349.5` + 子 `bolder` が旧実装では 350 への丸め後
/// `350 <= w < 550` 行 → 700 に化け、spec の `100 <= w < 350` 行 → 400
/// と食い違う。`549.5` + `bolder`、`749.5` + `lighter` も同型 — pin:
/// `crate::cascade::tests::bolder_lighter_resolve_against_unrounded_fractional_parent_weight`)。
/// `f32` 格上げにより丸めそのものが不要になったため、この 2 次被害も解消される。
fn parse_font_weight(input: &mut Parser<'_, '_>) -> Option<FontWeightValue> {
    match input.next().ok()? {
        // `<number [1,1000]>`。`value` field (f32) を見るので `1e3` のような
        // scientific notation や fractional もそのまま受理される (どちらも
        // CSS Values 3 の `<number>` production として spec-valid)。fraction は
        // 丸めずそのまま computed value まで運ぶ (上記 doc 参照)。
        // NaN は両比較が false、±inf は片方のみ false — いずれも guard が成立
        // しないため reject される (`1e400` → None、§2.2 "all other values are
        // invalid" と一致)。
        Token::Number { value, .. } if *value >= 1.0 && *value <= 1000.0 => {
            Some(FontWeightValue::Absolute(*value))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("normal") => {
            Some(FontWeightValue::Absolute(400.0))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("bold") => {
            Some(FontWeightValue::Absolute(700.0))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("bolder") => Some(FontWeightValue::Bolder),
        Token::Ident(name) if name.eq_ignore_ascii_case("lighter") => {
            Some(FontWeightValue::Lighter)
        }
        _ => None,
    }
}

/// `font-style: <ident>` を parse する (CSS Fonts 4 §2.4
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>)。
///
/// Value grammar (§2.4, full property grammar): `normal | italic | left |
/// right | oblique <angle [-90deg,90deg]>?`。本 parser は `normal` /
/// `italic` の 2 keyword のみ受理する ([`FontStyle`] doc の Scope carving
/// 節参照) — `oblique` (angle の有無を問わず) と `left` / `right` は
/// spec-valid だが未実装のため、他の未知 ident と同じく silent drop = `None`
/// とする。ASCII case-insensitive で ident を比較する (37n sibling
/// [`parse_direction`] と同 flavor)。
fn parse_font_style(input: &mut Parser<'_, '_>) -> Option<FontStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(FontStyle::Normal),
        "italic" => Some(FontStyle::Italic),
        _ => None,
    }
}

/// `text-transform: <ident>` を parse する (CSS Text Module Level 3 §2.1
/// <https://www.w3.org/TR/css-text-3/#text-transform-property>)。
///
/// Value grammar (§2.1, full property grammar): `none | [capitalize |
/// uppercase | lowercase] || full-width || full-size-kana`。本 parser は
/// `none` / `capitalize` / `uppercase` / `lowercase` の 4 keyword のみ受理
/// する ([`TextTransform`] doc の Scope carving 節参照) — `full-width` /
/// `full-size-kana` は spec-valid だが未実装のため、他の未知 ident と同じく
/// silent drop = `None` とする。1 ident しか consume しないため、
/// `full-width` / `full-size-kana` を伴う `||` 併記は **どちらの ident が
/// 先に来ても** declaration 全体が drop されるが、drop される場所は ident の
/// 順序で変わる ([`TextTransform`] doc の Scope carving 節が両 case の
/// canonical な記述):
///
/// - 未実装 ident が先 (例: `full-width uppercase`) — 本関数自体が最初の
///   ident で `None` を返す。caller の `parse_value` はこの時点で declaration
///   を drop するので、`DeclParser` の exhaustive-consumption check にすら
///   到達しない。
/// - case keyword が先 (例: `uppercase full-width`) — 本関数は `uppercase`
///   を consume して成功で返るが、`full-width` が未消費のまま残る。
///   declaration 全体は caller ([`mod@crate::rule`] の `DeclParser`) の
///   exhaustive-consumption check で drop される (`font-size: 16px 20px` を
///   拒否するのと同じ一般 mechanism)。
///
/// ASCII case-insensitive で ident を比較する (sibling [`parse_font_style`]
/// と同 flavor)。
fn parse_text_transform(input: &mut Parser<'_, '_>) -> Option<TextTransform> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "capitalize" => Some(TextTransform::Capitalize),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        _ => None,
    }
}

/// `visibility: <ident>` を parse する (CSS Display 3 §4
/// <https://www.w3.org/TR/css-display-3/#visibility>)。
///
/// Value grammar (spec verbatim): `visible | hidden | collapse`。全 3
/// keyword を受理する ([`Visibility`] doc の Scope carving 節参照 —
/// `collapse` の formatting-context 固有な space-saving 効果は未実装だが、
/// keyword 自体は spec-valid として受理する)。ASCII case-insensitive で
/// ident を比較する ([`parse_font_style`] と同 flavor)。
fn parse_visibility(input: &mut Parser<'_, '_>) -> Option<Visibility> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        "hidden" => Some(Visibility::Hidden),
        "collapse" => Some(Visibility::Collapse),
        _ => None,
    }
}

/// `word-break: <ident>` を parse する (CSS Text 3 §5.1
/// <https://www.w3.org/TR/css-text-3/#word-break-property>)。
///
/// Value grammar (§5.1, full property grammar): `normal | keep-all |
/// break-all | break-word`。本 parser は `normal` / `keep-all` /
/// `break-all` の 3 keyword のみ受理する ([`WordBreak`] doc の Scope
/// carving 節参照) — 4th keyword `break-word` (deprecated,
/// `word-break: normal` + `overflow-wrap: anywhere` の compound 相当) は
/// spec-valid だが未実装のため、他の未知 ident と同じく silent drop =
/// `None` とする。ASCII case-insensitive で ident を比較する (sibling
/// `parse_font_style` と同 flavor)。
fn parse_word_break(input: &mut Parser<'_, '_>) -> Option<WordBreak> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(WordBreak::Normal),
        "keep-all" => Some(WordBreak::KeepAll),
        "break-all" => Some(WordBreak::BreakAll),
        _ => None,
    }
}

/// `overflow-wrap: <ident>` (`word-wrap` legacy alias 名でも呼ばれる、
/// [`OverflowWrap`] doc の「legacy alias」節参照) を parse する (CSS Text 3
/// §5.4 <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>)。
///
/// Value grammar (§5.4): `normal | break-word | anywhere` — 3 keyword とも
/// 受理する (`WordBreak` の deprecated `break-word` とは異なり、
/// `overflow-wrap` 自身の `break-word` は deprecated ではない spec-valid
/// keyword、[`OverflowWrap`] doc 参照)。ASCII case-insensitive で ident を
/// 比較する (sibling `parse_word_break` と同 flavor)。
fn parse_overflow_wrap(input: &mut Parser<'_, '_>) -> Option<OverflowWrap> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(OverflowWrap::Normal),
        "break-word" => Some(OverflowWrap::BreakWord),
        "anywhere" => Some(OverflowWrap::Anywhere),
        _ => None,
    }
}

/// `float: <ident>` を parse する (CSS2 §9.5.1
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>, [`FloatValue`]
/// doc 参照)。
///
/// Value grammar: `left | right | none` (`inherit` は上記 "CSS-wide
/// keyword (canonical)" 節により未対応)。ASCII case-insensitive matching
/// は sibling `parse_word_break` と同 flavor。
fn parse_float(input: &mut Parser<'_, '_>) -> Option<FloatValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(FloatValue::None),
        "left" => Some(FloatValue::Left),
        "right" => Some(FloatValue::Right),
        _ => None,
    }
}

/// `clear: <ident>` を parse する (CSS2 §9.5.2
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>, [`ClearValue`]
/// doc 参照)。
///
/// Value grammar: `none | left | right | both` (`inherit` は上記
/// "CSS-wide keyword (canonical)" 節により未対応)。ASCII case-insensitive
/// matching は sibling `parse_float` と同 flavor。
fn parse_clear(input: &mut Parser<'_, '_>) -> Option<ClearValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(ClearValue::None),
        "left" => Some(ClearValue::Left),
        "right" => Some(ClearValue::Right),
        "both" => Some(ClearValue::Both),
        _ => None,
    }
}

/// `display: <ident>` を parse する。
///
/// CSS Display 3 §2 "Box Layout Modes: the display property"
/// <https://www.w3.org/TR/css-display-3/#propdef-display>。現状受理する
/// keyword は 6 つ:
///
/// - `block` — `<display-outside>` (block flow)
/// - `inline` — `<display-outside>` (inline flow、initial value)
/// - `inline-block` — `<display-legacy>` (inline flow-root)
/// - `none` — `<display-box>` (subtree omitted from box tree)
/// - `flex` — `<display-inside>` (§2.2) keyword、outer-defaulting rule
///   により `block flex` と等価
/// - `grid` — `<display-inside>` (§2.2) keyword、outer-defaulting rule
///   により `block grid` と等価
///
/// 他 keyword (`inline-flex` / `inline-grid` / `table*` / `list-item` /
/// `flow-root` / `contents` 等) は spec-valid だが未実装のため silent drop
/// (`None`)。ASCII case-insensitive で ident を比較する (CSS Values 3
/// §3.1 "Pre-defined Keywords" <https://www.w3.org/TR/css-values-3/#keywords>:
/// keyword は ASCII case-insensitive)。
fn parse_display(input: &mut Parser<'_, '_>) -> Option<DisplayValue> {
    // 37n sibling multi-keyword idiom (parse_string_fetch / parse_content_part /
    // parse_content_text_keyword) に揃える。ASCII case-insensitive matching は
    // to_ascii_lowercase() 経由 (parse-time allocation は一 declaration 一回)。
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "block" => Some(DisplayValue::Block),
        "inline" => Some(DisplayValue::Inline),
        "inline-block" => Some(DisplayValue::InlineBlock),
        "none" => Some(DisplayValue::None),
        "flex" => Some(DisplayValue::Flex),
        "grid" => Some(DisplayValue::Grid),
        _ => None,
    }
}

/// `box-sizing: <ident>` を parse する
/// (CSS Sizing 3 §3.3 <https://www.w3.org/TR/css-sizing-3/#box-sizing>)。
///
/// Spec value grammar (§3.3): `content-box | border-box`。ASCII
/// case-insensitive で ident を比較する (CSS Values 3 §3.1 "Pre-defined
/// Keywords"、37n sibling [`parse_display`] / [`parse_text_align`] と同 flavor)。
///
/// # Scope carving ([`BoxSizing`] doc-comment に詳述)
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 他 keyword (`padding-box` — CSS-UI 3 draft 相当
///   だが css-sizing-3 では削除、`margin-box` 等) は silent drop = `None`。
fn parse_box_sizing(input: &mut Parser<'_, '_>) -> Option<BoxSizing> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

/// `text-align: <ident>` を parse する
/// (CSS Text 3 §6.1 <https://www.w3.org/TR/css-text-3/#text-align-property>)。
///
/// Spec value grammar (§6.1): `start | end | left | right | center | justify |
/// match-parent | justify-all`。ASCII case-insensitive で ident を比較する
/// (CSS spec 慣行、37n sibling [`parse_string_fetch`] / [`parse_content_part`] /
/// [`parse_content_text_keyword`] と同 flavor)。
///
/// # Scope carving ([`TextAlign`] doc-comment に詳述)
///
/// - **(b) 非対応**: `<string>` value は silent drop。CSS Text 3
///   §6.1 の grammar には無く、CSS Text 4 §7.1
///   <https://www.w3.org/TR/css-text-4/#text-align-property> で追加された
///   alternative (semantics は同 §7.2 "Character-based Alignment in a Table
///   Column")。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 未知 keyword (`middle` 等) は silent drop = `None`。
fn parse_text_align(input: &mut Parser<'_, '_>) -> Option<TextAlign> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "start" => Some(TextAlign::Start),
        "end" => Some(TextAlign::End),
        "left" => Some(TextAlign::Left),
        "right" => Some(TextAlign::Right),
        "center" => Some(TextAlign::Center),
        "justify" => Some(TextAlign::Justify),
        "match-parent" => Some(TextAlign::MatchParent),
        "justify-all" => Some(TextAlign::JustifyAll),
        _ => None,
    }
}

/// `direction: <ident>` を parse する
/// (CSS Writing Modes 4 §2.1 <https://www.w3.org/TR/css-writing-modes-4/#direction>)。
///
/// Spec value grammar (§2.1): `ltr | rtl`。ASCII case-insensitive で ident を
/// 比較する (37n sibling [`parse_text_align`] と同 flavor)。
///
/// # Scope carving ([`Direction`] doc-comment に詳述)
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: `ltr` / `rtl` 以外の ident は silent drop = `None`。
fn parse_direction(input: &mut Parser<'_, '_>) -> Option<Direction> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "ltr" => Some(Direction::Ltr),
        "rtl" => Some(Direction::Rtl),
        _ => None,
    }
}

/// `overflow-x` / `overflow-y: <ident>` を parse する (
/// CSS Overflow 3 §3.1 <https://www.w3.org/TR/css-overflow-3/#overflow-properties>)。
///
/// Spec value grammar (§3.1): `visible | hidden | clip | scroll | auto`。
/// ASCII case-insensitive で ident を比較する (37n sibling [`parse_box_sizing`] /
/// [`parse_direction`] と同 flavor)。
///
/// # Scope carving ([`OverflowValue`] doc-comment に詳述)
///
/// - **(a) spec-invalid**: 上記 5 keyword 以外の ident は silent drop = `None`。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
fn parse_overflow_value(input: &mut Parser<'_, '_>) -> Option<OverflowValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "visible" => Some(OverflowValue::Visible),
        "hidden" => Some(OverflowValue::Hidden),
        "clip" => Some(OverflowValue::Clip),
        "scroll" => Some(OverflowValue::Scroll),
        "auto" => Some(OverflowValue::Auto),
        _ => None,
    }
}

/// `overflow: <'overflow-block'>{1,2}` shorthand — 1-2 value expansion。
///
/// grammar reference: CSS Overflow 3 §3.1
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>。
///
/// # Expansion rule (spec verbatim, §3.1)
///
/// "The overflow property is a shorthand property that sets the specified
/// values of overflow-x and overflow-y in that order. If the second value is
/// omitted, it is copied from the first."
///
/// [`parse_margin_shorthand`] と同じ try_parse 積み上げ pattern の 2-value
/// 版 (1-4 value ではなく 1-2 value であること以外は同型)。
///
/// # Trailing garbage handling
///
/// 3rd value (`overflow: hidden scroll auto`) は本 helper では 2 value 消費
/// して残り 1 token を unconsumed で return する。caller の
/// [`mod@crate::rule`] の `DeclParser` の
/// [`cssparser::DeclarationParser::parse_value`] impl が `expect_exhausted`
/// で余剰 token を検知して declaration ごと drop する
/// ([`parse_margin_shorthand`] doc の「Trailing garbage handling」節と同じ
/// 責務分担)。
fn parse_overflow_shorthand(input: &mut Parser<'_, '_>) -> Option<OverflowXY> {
    let v1 = parse_overflow_value(input)?;
    // 2nd value 不在 → 1 value case: 両 axis に spread (§3.1 "If the second
    // value is omitted, it is copied from the first.")。
    let Some(v2) = input.try_parse(|i| parse_overflow_value(i).ok_or(())).ok() else {
        return Some(OverflowXY::both(v1));
    };
    Some(OverflowXY { x: v1, y: v2 })
}

/// `text-decoration-line: none | [ underline || overline || line-through ||
/// blink ]` を parse する (CSS Text Decoration Module Level 3 §2.1
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>)。
///
/// # top-level alternative (`none` vs. `||` combination)
///
/// grammar は `none | [ ... ]` — `none` は他 4 keyword と併記不可能な
/// **別 alternative** (`none underline` は spec-invalid) であり、`none` 自体が
/// `||` combination の一員ではない。よって `none` を最初に単独で試し、
/// 一致すれば即 return する。
///
/// # `||` (any-order, each-at-most-once) loop
///
/// `none` に一致しなければ、[`parse_border_shorthand`] の per-slot
/// `try_parse` loop と同じ shape で 4 keyword を順不同・重複無しに peel する
/// (詳細な rationale は同関数 doc 参照)。4 keyword の ident 集合は互いに
/// disjoint (border shorthand の width/style/color 3 slot が disjoint なのと
/// 同じ理由 — 単純に別々の語)。
///
/// - unfilled flag (未 true の bool field) のみ試行
/// - 埋まっている flag に対する 2 回目の同一 keyword は、その flag の
///   `try_parse` を試さない (falls through) ので match せず loop を抜ける —
///   caller ([`mod@crate::rule`] の `DeclParser`) の `expect_exhausted` が
///   leftover token を検知して declaration ごと drop する
///   (`text-decoration-line: underline underline` は 0 decl になる)
/// - 4 flag とも埋まった、またはどの keyword にも match しなくなったら break
/// - 1 個も flag が立たなければ (`none` でもなく、`||` combination も 0 個)
///   `None` — spec `||` grammar の "one or more of them must occur" 違反
fn parse_text_decoration_line(input: &mut Parser<'_, '_>) -> Option<TextDecorationLine> {
    // top-level alternative: `none`。`||` combination とは併記不可 (上記 doc)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(TextDecorationLine::NONE);
    }

    let mut line = TextDecorationLine::NONE;
    loop {
        if !line.underline
            && input
                .try_parse(|i| i.expect_ident_matching("underline"))
                .is_ok()
        {
            line.underline = true;
            continue;
        }
        if !line.overline
            && input
                .try_parse(|i| i.expect_ident_matching("overline"))
                .is_ok()
        {
            line.overline = true;
            continue;
        }
        if !line.line_through
            && input
                .try_parse(|i| i.expect_ident_matching("line-through"))
                .is_ok()
        {
            line.line_through = true;
            continue;
        }
        if !line.blink
            && input
                .try_parse(|i| i.expect_ident_matching("blink"))
                .is_ok()
        {
            line.blink = true;
            continue;
        }
        break;
    }

    if line == TextDecorationLine::NONE {
        // `none` は上で既に処理済み — ここに来るのは 0 keyword しか
        // match しなかった場合のみ (未知 ident、または value 自体が空)。
        return None;
    }
    Some(line)
}

/// `text-decoration-style: solid | double | dotted | dashed | wavy` を
/// parse する (CSS Text Decoration Module Level 3 §2.2
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>)。
/// ASCII case-insensitive で ident を比較する (37n sibling
/// [`parse_border_style_side`] と同 flavor)。
fn parse_text_decoration_style(input: &mut Parser<'_, '_>) -> Option<TextDecorationStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "solid" => Some(TextDecorationStyle::Solid),
        "double" => Some(TextDecorationStyle::Double),
        "dotted" => Some(TextDecorationStyle::Dotted),
        "dashed" => Some(TextDecorationStyle::Dashed),
        "wavy" => Some(TextDecorationStyle::Wavy),
        _ => None,
    }
}

/// `text-decoration-color: <color>` を parse する (CSS Text Decoration Module
/// Level 3 §2.3
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>)。
///
/// [`parse_border_color`] と同型 — `currentcolor` keyword (CSS Color 3 §4.4)
/// を先取りしてから [`parse_color`] (hex / named / `rgb(a)` / `transparent`)
/// に委譲する。独立した helper にしてあるのは、両 property が異なる
/// payload 型 ([`TextDecorationColor`] / [`BorderColor`]) を持つため —
/// [`parse_border_color`] 自体は border-*-color 専用のまま変更しない。
fn parse_text_decoration_color(input: &mut Parser<'_, '_>) -> Option<TextDecorationColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(TextDecorationColor::CurrentColor);
    }
    parse_color(input).map(TextDecorationColor::Resolved)
}

/// `text-decoration: <'text-decoration-line'> || <'text-decoration-style'> ||
/// <'text-decoration-color'>` shorthand を parse する (CSS Text Decoration
/// Module Level 3 §2.4
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-property>)。
///
/// [`parse_border_shorthand`] と同じ 3-slot `||` loop (line / style / color)
/// — 詳細な rationale・loop 構造・initial value fill の判断根拠は同関数 doc
/// 参照。3 slot の ident/token 集合は互いに disjoint: line keyword
/// (`none`/`underline`/`overline`/`line-through`/`blink`) と style keyword
/// (`solid`/`double`/`dotted`/`dashed`/`wavy`) はどちらも named CSS color
/// ではなく ([`parse_named_color`] のテーブルに無い)、[`parse_color`] の
/// Ident 分岐に誤って吸われることはない。
///
/// # Initial value fill (省略成分)
///
/// spec §2.4 verbatim: "Omitted values are set to their initial values."
/// - line 省略 → [`TextDecorationLine::NONE`] (§2.1 initial)
/// - style 省略 → [`TextDecorationStyle::Solid`] (§2.2 initial)
/// - color 省略 → [`TextDecorationColor::CurrentColor`] (§2.3 initial)
fn parse_text_decoration_shorthand(input: &mut Parser<'_, '_>) -> Option<TextDecorationShorthand> {
    let mut line: Option<TextDecorationLine> = None;
    let mut style: Option<TextDecorationStyle> = None;
    let mut color: Option<TextDecorationColor> = None;

    loop {
        if line.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if line.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationLine, ParseError<'_, ()>> {
                parse_text_decoration_line(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            line = Some(v);
            continue;
        }
        if style.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationStyle, ParseError<'_, ()>> {
                parse_text_decoration_style(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(v);
            continue;
        }
        if color.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationColor, ParseError<'_, ()>> {
                parse_text_decoration_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(v);
            continue;
        }
        break;
    }

    // spec `||` grammar: at least 1 component 必須。0 component は `None` =
    // declaration drop (`parse_border_shorthand` と同じ判断)。
    if line.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    Some(TextDecorationShorthand {
        line: line.unwrap_or(TextDecorationLine::NONE),
        style: style.unwrap_or(TextDecorationStyle::Solid),
        color: color.unwrap_or(TextDecorationColor::CurrentColor),
    })
}

/// `vertical-align: <ident>` を parse する (CSS 2.1 §10.8.1
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>)。
///
/// ASCII case-insensitive で ident を比較する (37n sibling [`parse_direction`]
/// / [`parse_text_decoration_style`] と同 flavor)。
///
/// # Scope carving (minimal scope、[`VerticalAlign`] doc-comment に詳述)
///
/// - **(b) 非対応**: `top` / `text-top` / `middle` / `bottom` /
///   `text-bottom` keyword、`<percentage>` / `<length>` value は silent
///   drop = `None` — [`VerticalAlign`] doc 参照。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: `baseline` / `sub` / `super` 以外の ident は
///   silent drop = `None`。
fn parse_vertical_align(input: &mut Parser<'_, '_>) -> Option<VerticalAlign> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "baseline" => Some(VerticalAlign::Baseline),
        "sub" => Some(VerticalAlign::Sub),
        "super" => Some(VerticalAlign::Super),
        _ => None,
    }
}

/// `counter-reset` / `counter-increment` / `counter-set` の value を parse する。
///
/// Grammar (CSS Lists 3 §4):
///   `<counter-name> = <custom-ident>` — CSS-wide keyword (inherit / initial /
///   unset / revert / revert-layer) + `default` + `none` を除く任意 ident。
///   `[ <counter-name> <integer>? ]+ | none`。
///
/// `default_number`: 各 property の spec default (reset=0、increment=1、set=0)。
///
/// `none` を top-level alternative として先に処理。以降は ident + optional
/// integer を LL(1) で peel。ident が reserved keyword、または最初の token が
/// ident でない (`counter-reset: 123 abc` 等) 場合は None を返し、rule.rs 側の
/// silent-drop で declaration が丸ごと落ちる。
///
/// 途中 ident (`chapter none`) が reserved の場合は `try_parse` の rewind で
/// 未消費のまま loop を抜け、caller の `expect_exhausted` (rule.rs)
/// が leftover token を検出して declaration を drop する。
fn parse_counter_property(
    input: &mut Parser<'_, '_>,
    default_number: i32,
) -> Option<Vec<(SmolStr, i32)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        // reserved keyword を counter-name として受理しない (spec §4、`<custom-ident>`
        // の除外リスト)。try_parse の rewind で reserved 検出時は unconsumed に戻す。
        let name = match input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = i.expect_ident()?.clone();
            if is_reserved_counter_name(&ident) {
                Err(i.new_custom_error(()))
            } else {
                Ok(SmolStr::new(ident.as_ref()))
            }
        }) {
            Ok(name) => name,
            Err(_) => break,
        };
        // optional trailing `<integer>` (missing → property-specific default)。
        let value = input
            .try_parse(|i| i.expect_integer())
            .unwrap_or(default_number);
        result.push((name, value));
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// `<counter-name>` = `<custom-ident>` の除外リスト (CSS Lists 3 §4 + CSS Values 4
/// §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>)。
///
/// CSS-wide keyword + `default` (Counter Styles L3) + `none` (top-level alternative)
/// を弾く。case-insensitive 比較。
///
/// これは CSS Values 4 §4.2 の permanent な spec 除外規定であり、**CSS-wide
/// keyword の実装状況とは無関係** — [`PropertyValue`] doc の「CSS-wide keyword」節
/// が説明する「property value としては未実装」claim
/// とは別の話なので混同しないこと。
fn is_reserved_counter_name(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" | "none"
    )
}

/// `content: normal | none | <content-list>` を parse する
/// (CSS Content 3 §1 <https://www.w3.org/TR/css-content-3/#content-property>)。
///
/// `normal` / `none` は spec で意味が異なる (pseudo-element の生成/非生成) が、
/// 本 crate は cascade static side に留まり生成判断は下流に委ねるため、両者を
/// 空 `Vec` に落として区別を持たない (§7.1 downstream mapping で必要になれば
/// 変異させる)。counter-* precedent と同じ shape。
///
/// items+ loop は `<string>` literal と function token (`counter(...)` 等) を
/// 順次 peel する。認識できない token に当たった時点で loop を break、caller
/// の `expect_exhausted` (rule.rs) が leftover を検知して declaration ごと drop。
///
/// `alt text` (spec `... [/ <string>...]?`) は現状 scope 外、`/` 以降は
/// unconsumed のまま caller に返す (現状 rule.rs の `expect_exhausted` により
/// declaration drop、alt text 対応時に本関数を extend)。
fn parse_content(input: &mut Parser<'_, '_>) -> Option<Vec<ContentComponent>> {
    // `normal` / `none` = 空 list (top-level alternative)。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(Vec::new());
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let items = parse_content_list_items(input, ContentListMode::CssContent3);
    if items.is_empty() { None } else { Some(items) }
}

/// `<content-list>` の items+ loop 部分。
///
/// `<string>` bare literal、`<image>` の `<url>` alternative、bare keyword
/// (`contents` / `<quote>`)、function token (`counter(...)` / `string(...)` /
/// `target-*()` / `attr(...)` / `content(...)` / `leader(...)`) を順次 peel。
/// 認識できない token に当たった時点で break — 呼び出し側が leftover を検知
/// して drop する。
///
/// `content` property (`parse_content`) と `string-set` property
/// (`parse_string_set`) の両方から call されるが、GCPM 3 §1.1.1 は string-set
/// 向けに CSS Content 3 §2 の broad list を narrower に再定義しているため、
/// `mode` パラメータで受理 alternative 集合を分岐する:
/// - [`ContentListMode::CssContent3`] — content property (CSS Content 3 §2
///   <https://www.w3.org/TR/css-content-3/#content-values>)。10 alt full set
///   を受理 (`<image>` / `contents` / `<quote>` /
///   `leader()` は後に追加)。
/// - [`ContentListMode::GcpmStringSet`] — string-set property (CSS GCPM 3
///   §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list>)。`string()` /
///   `target-counter()` / `target-counters()` / `target-text()` / `<image>` /
///   `contents` / `<quote>` / `leader()` は GCPM 3 §1.1.1 L82 narrow grammar
///   に含まれず reject (bare `<string>` literal は両 mode で受理)。
///
/// bare literal 分岐は spec 上両 mode で共通 (どちらの `<content-list>` grammar
/// も `<string>` を top-level alternative に含む) なので mode 判定なし。他の
/// 分岐は各 branch 内で mode guard を掛ける ([`parse_content_function`] の
/// match arm guard と同じ pattern)。`Parser::try_parse` は失敗時に読んだ token
/// を必ず rewind するため、branch の試行順序は正しさに影響しない
/// (どの順で並べても等価)。
///
/// (導入後、mode-parameterize を経て `<image>` / `contents` / `<quote>` /
/// `leader()` を追加)
fn parse_content_list_items(
    input: &mut Parser<'_, '_>,
    mode: ContentListMode,
) -> Vec<ContentComponent> {
    let mut items = Vec::new();
    loop {
        // bare `<string>` literal — 両 mode 共通 (mode gate 不要)。
        if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
            items.push(ContentComponent::Literal(SmolStr::new(s.as_ref())));
            continue;
        }
        // `<image>` の `<url>` alternative — CSS Images 3 <url> production
        // (`url(...)` / `url("...")`) のみ (`<gradient>` は未実装として
        // defer、`ContentComponent::Image` doc 参照)。`expect_url` は bare
        // quoted string を受理しない (`<url> = <url()> | <src()>`) ので上の
        // literal 分岐との誤 overlap は無い。CssContent3 mode 限定
        // (GCPM 3 §1.1.1 L82 narrow list に `<image>` は含まれない)。
        if mode == ContentListMode::CssContent3
            && let Ok(url) = input.try_parse(|i| i.expect_url())
        {
            items.push(ContentComponent::Image {
                url: url.as_ref().to_string(),
            });
            continue;
        }
        // bare keyword alternative — `contents` / `<quote>` (function でも
        // `<string>` でもない ident-only alternative)。CssContent3 mode 限定。
        if mode == ContentListMode::CssContent3
            && let Ok(c) = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
                let ident = i.expect_ident()?.clone();
                parse_content_bare_keyword(ident.as_ref()).ok_or_else(|| i.new_custom_error(()))
            })
        {
            items.push(c);
            continue;
        }
        // function — mode に応じて `string()` / `target-*()` / `leader()` を
        // reject する判定は `parse_content_function` の match arm side で実施。
        let parsed = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
            let name = i.expect_function()?.clone();
            i.parse_nested_block(|inner| {
                parse_content_function(name.as_ref(), mode, inner)
                    .ok_or_else(|| inner.new_custom_error(()))
            })
        });
        match parsed {
            Ok(c) => items.push(c),
            Err(_) => break,
        }
    }
    items
}

/// `contents` keyword と `<quote>` (`open-quote` / `close-quote` /
/// `no-open-quote` / `no-close-quote`) の bare-ident alternative をまとめて
/// 判定する ([`parse_content_list_items`] 専用 helper)。CSS Content 3 §2.3
/// <https://www.w3.org/TR/css-content-3/#element-content> および §2.4.2
/// <https://www.w3.org/TR/css-content-3/#quote-values>。
fn parse_content_bare_keyword(ident: &str) -> Option<ContentComponent> {
    match ident.to_ascii_lowercase().as_str() {
        "contents" => Some(ContentComponent::Contents),
        "open-quote" => Some(ContentComponent::Quote(QuoteKeyword::OpenQuote)),
        "close-quote" => Some(ContentComponent::Quote(QuoteKeyword::CloseQuote)),
        "no-open-quote" => Some(ContentComponent::Quote(QuoteKeyword::NoOpenQuote)),
        "no-close-quote" => Some(ContentComponent::Quote(QuoteKeyword::NoCloseQuote)),
        _ => None,
    }
}

/// `string-set: none | [ <custom-ident> <content-list> ]#` を parse する
/// (CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>)。
///
/// `none` を top-level alternative として先に処理し、以降は
/// `(name, content-list)` entry を comma-separated で peel する。
///
/// `<custom-ident>` は CSS-wide keyword + `default` (css-values-4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents> が将来の CSS-wide
/// keyword 用に予約) + `none` (top-level alt、gcpm-3 §1.1.1) を弾く。
///
/// ## Entry separator の strict 化
///
/// `#` (comma-separated multiplier、CSS Values 4 §2.3
/// <https://www.w3.org/TR/css-values-4/#mult-comma>) は entry 間に comma を
/// 要求する一方、**trailing comma を許容しない**。従って comma を consume した
/// 直後の loop iteration では次 entry の name parse **必須** — 失敗すれば
/// `#` production 全体が spec-invalid、declaration drop = `None`。
///
/// 初回 iteration で name parse が失敗する case (`string-set: ,`,
/// `string-set: "x"` 等 name 不在) も含めて `.ok()?` で一律に `None` 上位伝播
/// する。この strict `?` propagation は sibling
/// [`parse_optional_counter_style`] と同 principle。
///
/// `<content-list>` は 1+ items 必須 (CSS Content 3 §2)。name の後に 1 item も
/// peel できなければ malformed → `None` (declaration drop)。
fn parse_string_set(input: &mut Parser<'_, '_>) -> Option<Vec<(SmolStr, Vec<ContentComponent>)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut entries = Vec::new();
    loop {
        // <custom-ident> — CSS-wide keyword + `default` + `none` を弾く。
        // 既存 `is_reserved_custom_ident` (css-wide + default) と、property-specific
        // top-level alternative の `none` reject を組み合わせる (
        // `is_reserved_custom_ident` docstring の想定 usage)。
        //
        // `.ok()?` で strict 上位伝播: (a) 初回 iteration で name 不在 = `#`
        // production 0 entries、(b) 直前 iteration で bottom `expect_comma` が
        // succeed した直後 = trailing comma、の 2 case を一律 `None` に落とす。
        let name = input
            .try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
                let ident = i.expect_ident()?.clone();
                if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                    Err(i.new_custom_error(()))
                } else {
                    Ok(SmolStr::new(ident.as_ref()))
                }
            })
            .ok()?;
        // <content-list> は 1+ items 必須。0 items → declaration drop。
        // GCPM 3 §1.1.1 narrow local <content-list> = `string()` と `target-*()`
        // を受理しない (詳細は `ContentListMode` doc)。
        let items = parse_content_list_items(input, ContentListMode::GcpmStringSet);
        if items.is_empty() {
            return None;
        }
        entries.push((name, items));
        // 次 entry の separator: comma で継続、他 token で loop を抜ける
        // (caller `expect_exhausted` が leftover token を drop)。break 到達時は
        // 直前の push で entries 非空 — なので tail は無条件 `Some(entries)`。
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }

    // break 到達 = 直前の push を経ている、`.ok()?` 経路以外で loop を抜ける
    // 唯一の exit なので `entries` は必ず 1+。
    Some(entries)
}

/// Dispatch on function name (ASCII-case-insensitive、spec identifier 慣行)。
/// 未知の function name または引数 parse 失敗は `None` — caller の
/// `parse_nested_block` が custom error に変換する。
///
/// `mode` は property ごとの `<content-list>` 語彙を選ぶ (詳細は
/// [`ContentListMode`] doc):
/// - [`ContentListMode::CssContent3`] (`content` property, CSS Content 3 §2)
///   では全 arm を許可。
/// - [`ContentListMode::GcpmStringSet`] (`string-set` property, CSS GCPM 3
///   §1.1.1) では `string` / `target-counter` / `target-counters` /
///   `target-text` / `leader` arm を match guard で外し fall-through で `None`
///   を返す (= declaration drop、caller の `parse_string_set` が
///   `<content-list>` 0 items → `None`)。`counter` / `counters` / `content` /
///   `attr` は両 mode で spec grammar に含まれるため gate なし。
///
/// `<image>` (`url()`) / `contents` / `<quote>` は function 名 dispatch では
/// なく [`parse_content_list_items`] 側の bare-token branch で扱う (`<image>`
/// は url token、`contents`/`<quote>` は bare ident であり `expect_function`
/// にヒットしないため)。
///
/// 各 `parse_*_fn` は自身では `expect_exhausted` を呼ばない —
/// [`parse_content`] 側の `parse_nested_block` が内部で
/// [`Parser::parse_entirely`] を経由し、closure 成功後の余剰 token を
/// exhaustion check で拒否する ([`parse_rgb_function`] と同じ規約)。
fn parse_content_function(
    name: &str,
    mode: ContentListMode,
    input: &mut Parser<'_, '_>,
) -> Option<ContentComponent> {
    match name.to_ascii_lowercase().as_str() {
        "string" if matches!(mode, ContentListMode::CssContent3) => parse_string_fn(input),
        "counter" => parse_counter_fn(input),
        "counters" => parse_counters_fn(input),
        "attr" => parse_attr_fn(input),
        "target-counter" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_counter_fn(input)
        }
        "target-counters" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_counters_fn(input)
        }
        "target-text" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_text_fn(input)
        }
        "content" => parse_content_fn(input),
        "leader" if matches!(mode, ContentListMode::CssContent3) => parse_leader_fn(input),
        _ => None,
    }
}

/// `<custom-ident>` (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>): CSS-wide keyword と
/// `default` を除いた任意 ident。case-preserving、smol str で保持。
///
/// `none` はここでは除外しない。spec verbatim: "Specifications using
/// `<custom-ident>` must specify clearly what other keywords are excluded
/// from `<custom-ident>`, if any…" と述べるとおり、より狭い grammar
/// (`<counter-name>` 等) の追加除外は個別の predicate (例
/// [`is_reserved_counter_name`]) 側の責務。[`is_reserved_custom_ident`] の
/// docstring も参照。
///
/// **呼び出し元は当初 3 箇所**: `string()` の name 引数 ([`parse_string_fn`])、
/// `target-counter()` / `target-counters()` の第 2 引数
/// ([`parse_target_counter_fn`] / [`parse_target_counters_fn`])。いずれも spec 上
/// `<custom-ident>` を取り `none` は valid。
///
/// 後に `pub(crate)` に広げ、`counter_style` module が
/// `<counter-style-name>` (CSS Counter Styles L3 §3
/// <https://www.w3.org/TR/css-counter-styles-3/#typedef-counter-style-name> —
/// `<custom-ident>` に `none` 追加除外を足した production、`<symbol>` の
/// `<custom-ident>` alternative 等) の base として同じ CSS-wide keyword 除外
/// list を再利用する 4 箇所目の呼び出し元になった (`is_reserved_custom_ident`
/// の list を二重管理しないため — 本 crate の drift 回避規約、
/// [`crate::page::PageCascadeResult::declarations`] doc 同旨)。
///
/// `<counter-name>` を取る `counter()` / `counters()` および counter-* property は
/// **本関数を経由しない** — [`parse_counter_name`] / [`parse_counter_property`] が
/// [`is_reserved_counter_name`] で `none` を追加除外する。したがって本関数に
/// `none` 除外を足してはならない (足すと `target-counter(url(#a), none)` と
/// `string(none)` を spec に反して reject する)。
pub(crate) fn parse_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_custom_ident(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `<custom-ident>` 除外リスト (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>)。
///
/// CSS-wide keyword (`inherit` / `initial` / `unset` / `revert` /
/// `revert-layer`) と `default` のみを弾く。`none` はここでは除外せず、
/// より狭い grammar (`<counter-name>` 等) の追加除外は個別の predicate
/// (例 [`is_reserved_counter_name`]) で行う。case-insensitive 比較。
///
/// `pub(crate)`: `counter_style` module が
/// `<counter-style-name>` 系 production (rule name / `fallback` / `system:
/// extends`) の除外 predicate を組み立てる際にこの base list を再利用する
/// ([`parse_custom_ident`] の doc 参照)。
///
/// これは CSS Values 4 §4.2 の permanent な spec 除外規定であり、**CSS-wide
/// keyword の実装状況とは無関係** — [`PropertyValue`] doc の「CSS-wide keyword」節
/// が説明する「property value としては未実装」claim
/// とは別の話なので混同しないこと。
pub(crate) fn is_reserved_custom_ident(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
    )
}

/// `string(<custom-ident> [, [ first | start | last | first-except ]? ])`。
/// CSS Content 3 §2.7.2 <https://www.w3.org/TR/css-content-3/#string-function>。
fn parse_string_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_custom_ident(input)?;
    let fetch = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_string_fetch(input)?
    } else {
        StringFetchMode::default()
    };
    Some(ContentComponent::String { name, fetch })
}

fn parse_string_fetch(input: &mut Parser<'_, '_>) -> Option<StringFetchMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "first" => Some(StringFetchMode::First),
        "start" => Some(StringFetchMode::Start),
        "last" => Some(StringFetchMode::Last),
        "first-except" => Some(StringFetchMode::FirstExcept),
        _ => None,
    }
}

/// `<counter-name>` (CSS Lists 3 §4
/// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>):
/// `<custom-ident>` から `none` を追加除外した production。
/// spec verbatim: "A `<counter-name>` name cannot match the keyword `none`;
/// such an identifier is invalid as a `<counter-name>`"。
///
/// counter() / counters() (§4.7) の first argument、および
/// counter-reset / counter-increment / counter-set property
/// (§4.1 / §4.2) の name 引数で使う。後者は既に [`parse_counter_property`] が
/// [`is_reserved_counter_name`]
/// 経由で reject 済 — 本 helper は前者を同じ predicate に揃えるための wrapper。
fn parse_counter_name(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_counter_name(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `counter(<counter-name>, <counter-style>?)`。
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>。
/// first argument grammar は §4 `<counter-name>`
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) —
/// `<custom-ident>` から `none` を追加除外。
fn parse_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_counter_name(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::Counter { name, style })
}

/// `counters(<counter-name>, <string>, <counter-style>?)`。
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>。
/// first argument grammar は §4 `<counter-name>`
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) —
/// `<custom-ident>` から `none` を追加除外。
fn parse_counters_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_counter_name(input)?;
    input.expect_comma().ok()?;
    let separator = input.expect_string().ok()?.as_ref().to_string();
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::Counters {
        name,
        separator,
        style,
    })
}

/// optional trailing `, <counter-style>`。省略時は spec default `decimal`
/// (CSS Lists 3 §4.7 `counter()` / `counters()` の末尾引数
/// <https://www.w3.org/TR/css-lists-3/#counter-functions>、CSS Content 3 §2.6.1-2
/// `target-counter()` / `target-counters()` の末尾引数
/// <https://www.w3.org/TR/css-content-3/#target-counter>)。
///
/// grammar は `<counter-style>?` — `,` を先行させる時は ident 必須。
/// `,` を consume 後に ident 不在 (`counter(chapter,)` 等の trailing-comma)
/// は spec-invalid、`None` 上位伝播で declaration ごと drop する
/// (sibling [`parse_string_fetch`] / [`parse_content_part`] と同じ strict
/// `?` propagation、silent Decimal fallback は撤去済み)。
fn parse_optional_counter_style(input: &mut Parser<'_, '_>) -> Option<CounterStyle> {
    if input.try_parse(|i| i.expect_comma()).is_ok() {
        // comma consumed — ident 必須。失敗は None として上位伝播。
        let ident = input.expect_ident().ok()?.clone();
        Some(counter_style_from_ident(ident.as_ref()))
    } else {
        Some(CounterStyle::default())
    }
}

fn counter_style_from_ident(ident: &str) -> CounterStyle {
    if ident.eq_ignore_ascii_case("decimal") {
        CounterStyle::Decimal
    } else {
        CounterStyle::Named(SmolStr::new(ident))
    }
}

/// `attr(<attribute-name>)`。CSS Content 3 §2.1
/// <https://www.w3.org/TR/css-content-3/#strings>。
///
/// static-side scope: type / fallback (attr(x string, "default") 等) は defer。
fn parse_attr_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = input.expect_ident().ok()?.clone();
    Some(ContentComponent::Attr {
        name: SmolStr::new(name.as_ref()),
    })
}

/// target-* の第 1 引数 `[ <string> | <url> ]` を raw String として抽出。
/// `url("...")` / `url(...)` / bare `"..."` を統一的に受ける
/// (cssparser の `expect_url_or_string` を使用)。
fn parse_target_url(input: &mut Parser<'_, '_>) -> Option<String> {
    input
        .expect_url_or_string()
        .ok()
        .map(|s| s.as_ref().to_string())
}

/// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`。
/// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>。
fn parse_target_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounter { url, name, style })
}

/// `target-counters([<string>|<url>], <custom-ident>, <string>, <counter-style>?)`。
/// CSS Content 3 §2.6.2 <https://www.w3.org/TR/css-content-3/#target-counters>。
fn parse_target_counters_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    input.expect_comma().ok()?;
    let separator = input.expect_string().ok()?.as_ref().to_string();
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounters {
        url,
        name,
        separator,
        style,
    })
}

/// `target-text([<string>|<url>], [ content | before | after | first-letter ]?)`。
/// CSS Content 3 §2.6.3 <https://www.w3.org/TR/css-content-3/#target-text>。
fn parse_target_text_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    let part = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_content_part(input)?
    } else {
        ContentPart::default()
    };
    Some(ContentComponent::TargetText { url, part })
}

/// `position: static | running(<custom-ident>)` を parse する
/// (CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>)。
///
/// 現状 scope:
/// - `static` — [`PositionValue::Static`]、`inherit_from` の初期状態と一致するため
///   apply_value が no-op でも問題ない。cascade winner selection では
///   先行 `running(...)` を上書き suppress する identity 用途
///   (standalone-static test だけでは実効性が問えない点に注意)。
/// - `running(<custom-ident>)` — [`PositionValue::Running`]、apply_value が
///   1-item `RunningTemplate` を computed.running_templates に seed する。
/// - 他 keyword (`relative` / `absolute` / `fixed` / `sticky`) は未実装、
///   silent drop = `None`。
///
/// `<custom-ident>` の除外は string-set と同じ規約:
/// [`is_reserved_custom_ident`] (CSS-wide keyword + `default`) に加えて
/// `none` を弾く。`none` は position property の他 spec-defined keyword
/// では無いが、custom-ident としては予約 alternative の慣行を残しつつ、
/// runtime resolve で `element(none)` 参照を誤って matching させないためのガード
/// (string-set の `none` reject と同じ扱い)。
fn parse_position(input: &mut Parser<'_, '_>) -> Option<PositionValue> {
    // `static` は現状 scope で唯一受理する non-running keyword。
    if input
        .try_parse(|i| i.expect_ident_matching("static"))
        .is_ok()
    {
        return Some(PositionValue::Static);
    }
    // `running(<custom-ident>)`。function name は ASCII case-insensitive、
    // 中身の custom-ident は case-preserving で SmolStr に格納。
    let running = input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
        let fn_name = i.expect_function()?.clone();
        if !fn_name.eq_ignore_ascii_case("running") {
            return Err(i.new_custom_error(()));
        }
        i.parse_nested_block(|inner| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = inner.expect_ident()?.clone();
            if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                return Err(inner.new_custom_error(()));
            }
            Ok(SmolStr::new(ident.as_ref()))
        })
    });
    running.ok().map(PositionValue::Running)
}

/// `z-index: auto | <integer>` を parse する (CSS2 §9.9.1
/// <https://www.w3.org/TR/CSS2/visuren.html#z-index>, [`ZIndexValue`] doc
/// 参照)。
///
/// `auto` ident branch を先に try_parse する — [`parse_margin_side`] と同じ
/// order-of-alternative 理由 (同関数 doc 参照)、ここでは the two branches
/// (`auto` ident と integer token) の token kind が既に不連続なので必須では
/// ないが、既存 sibling と同じ並びに揃える。
///
/// integer 本体は [`parse_counter_property`] の `<integer>` 抽出と同じ
/// `expect_integer` 直接呼び出し — CSS Values 3 §4.2 "Integers: the
/// `<integer>` type" により符号付き (負値含む) を許容し、range 制限は無い。
fn parse_z_index(input: &mut Parser<'_, '_>) -> Option<ZIndexValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ZIndexValue::Auto);
    }
    input
        .try_parse(|i| i.expect_integer())
        .ok()
        .map(ZIndexValue::Integer)
}

/// `break-before: <ident>` / `break-after: <ident>` を parse する (CSS
/// Fragmentation Module Level 3 §3.1
/// <https://www.w3.org/TR/css-break-3/#break-between>)。
///
/// この crate の scope で受理する 4 keyword ([`BreakBetween`] doc の Scope
/// carving 節参照): `auto` / `avoid` / `avoid-page` / `page`。propdef の
/// 残り 8 keyword (`left` / `right` / `recto` / `verso` / `avoid-column` /
/// `column` / `avoid-region` / `region`) と、現行 spec grammar に無い
/// `always` / `all` は他の未知 ident と同じく silent drop (`None`)。ASCII
/// case-insensitive で ident を比較する ([`parse_word_break`] 等 sibling と
/// 同 flavor)。
fn parse_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "avoid-page" => Some(BreakBetween::AvoidPage),
        "page" => Some(BreakBetween::Page),
        _ => None,
    }
}

/// `break-inside: <ident>` を parse する (CSS Fragmentation Module Level 3
/// §3.2 <https://www.w3.org/TR/css-break-3/#break-within>)。
///
/// この crate の scope で受理する 3 keyword ([`BreakInside`] doc の Scope
/// carving 節参照): `auto` / `avoid` / `avoid-page`。propdef の残り 2
/// keyword (`avoid-column` / `avoid-region`) は他の未知 ident と同じく
/// silent drop (`None`)。ASCII case-insensitive で ident を比較する
/// ([`parse_break_between`] と同 flavor)。
fn parse_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        "avoid-page" => Some(BreakInside::AvoidPage),
        _ => None,
    }
}

/// `page-break-before: <ident>` / `page-break-after: <ident>` — CSS2.1
/// legacy shorthand for `break-before` / `break-after` — を parse し、
/// [`BreakBetween`] へ remap する (CSS Fragmentation Module Level 3 §3.4
/// <https://www.w3.org/TR/css-break-3/#page-break-properties>,
/// [`BreakBetween`] doc の「legacy shorthand」節の mapping table 参照)。
///
/// CSS2.1 自身の `page-break-before` / `page-break-after` propdef grammar
/// (verbatim, <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-before>)
/// は `auto | always | avoid | left | right`。本 parser はそのうち
/// `auto` / `avoid` / `always` の 3 keyword のみ受理する — `left` /
/// `right` は `break-before`/`break-after` 側で未実装 ([`BreakBetween`] doc
/// の Scope carving 節) の値へ remap されるため、この legacy shorthand
/// 経由でも同じく受理しない。`always` は spec の mapping table どおり
/// [`BreakBetween::Page`] へ remap する (identity ではない — `auto` /
/// `avoid` は identity)。
fn parse_legacy_page_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "always" => Some(BreakBetween::Page),
        _ => None,
    }
}

/// `page-break-inside: <ident>` — CSS2.1 legacy shorthand for
/// `break-inside` — を parse する (CSS Fragmentation Module Level 3 §3.4,
/// [`BreakInside`] doc の「legacy shorthand」節参照)。
///
/// CSS2.1 自身の `page-break-inside` propdef grammar (verbatim,
/// <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-inside>) は
/// `avoid | auto` のみ (`always` / `left` / `right` は無い) — この 2
/// keyword を [`BreakInside`] へ identity mapping する。`break-inside`
/// 自身が持つ `avoid-page` は CSS2.1 の `page-break-inside` grammar には
/// 無いため受理しない。
fn parse_legacy_page_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        _ => None,
    }
}

fn parse_content_part(input: &mut Parser<'_, '_>) -> Option<ContentPart> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "content" => Some(ContentPart::Content),
        "before" => Some(ContentPart::Before),
        "after" => Some(ContentPart::After),
        "first-letter" => Some(ContentPart::FirstLetter),
        _ => None,
    }
}

/// `content([ text | before | after | first-letter ]?)` (`?` は raikiri の
/// 受理済み記法であり、GCPM 3 の grammar 自体には無い formal optional
/// marker ではない)。CSS GCPM 3 §1.1.1.1 "The content() function"
/// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> の keyword 集合
/// (`text | before | after | first-letter` の 4 種) をそのまま実装する。
///
/// **grammar 選択の根拠**: `content()` は GCPM 3 §1.1.1.1 と CSS Content 3
/// §2.7.3 <https://www.w3.org/TR/css-content-3/#funcdef-content> の 2 つの
/// spec に別々に定義されており、2 つの軸で食い違う。(1) keyword 集合 — GCPM 3
/// は 4 keyword のみ、CSS Content 3 はそこに `marker` を加えた 5 keyword。
/// (2) 引数の省略可否 — GCPM 3 の production 自体には `?` が無く引数は形式上
/// 必須だが、CSS Content 3 は `?` 付きで、省略時は `text` を暗黙採用すると
/// 明記する。この実装は (1) の keyword 集合では GCPM 3 §1.1.1.1 に従い、
/// `marker` を意図的に reject する。(2) の引数省略可否については逆に
/// CSS Content 3 §2.7.3 の `?` 付き grammar と同じ挙動 (省略時 `text`
/// フォールバック) を採用しており、GCPM 3 の厳密な grammar (引数必須) には
/// 従っていない — 「GCPM 3 に従う」と言えるのは keyword 集合の軸のみである。
/// これは spec 間の grammar 相反を軸ごとに解決した結果の選択であり、
/// 実装漏れではない。
///
/// **既知の feature gap**: `content` property 側の `<content-list>` は
/// CSS Content 3 §2 governance (broad grammar、[`ContentListMode::CssContent3`]
/// 参照) だが、この `content()` 内部の keyword 集合だけは両 property 呼び出し
/// 元で GCPM 3 §1.1.1.1 の 4-keyword 版のまま unconditional に適用される
/// (下記 mode dispatch の節参照)。引数省略時の `text` フォールバック挙動は
/// 既に CSS Content 3 §2.7.3 の記述と一致しているため、CSS Content 3 §2.7.3
/// の広い grammar を優先実装する必要が生じた場合、残る差分は `marker`
/// keyword の受理のみ。
///
/// bare `content()` (spec 例 `h2 { string-set: heading content() }`、
/// string-set/GCPM3 側の文脈) では [`ContentTextKeyword::Text`] を
/// フォールバック値として使う (根拠は GCPM 3 側の spec "default" 宣言では
/// ない — 詳細は [`ContentTextKeyword`] の doc comment 参照)。target-text()
/// の第 2 引数と
/// 違い、keyword は paren 直下に置かれる (comma を先行させない)。
///
/// GCPM 3 §1.1.1 の narrow `<content-list>` (string-set 側) と CSS Content 3
/// §2 の broad `<content-list>` (content property 側) の **両方** に対し
/// unconditional に受理される ([`ContentListMode`] mode gate なし、
/// mode dispatch 導入後もこの arm は両 mode で unconditional のまま、
/// [`parse_content_function`] の match arm 参照) — 上記の通り、この
/// unconditional な適用自体が「受理 keyword 集合は両 property とも
/// GCPM 3 §1.1.1.1 の 4 種」という選択の実装箇所である。
fn parse_content_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let keyword = if input.is_exhausted() {
        ContentTextKeyword::default()
    } else {
        parse_content_text_keyword(input)?
    };
    Some(ContentComponent::Content { keyword })
}

fn parse_content_text_keyword(input: &mut Parser<'_, '_>) -> Option<ContentTextKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "text" => Some(ContentTextKeyword::Text),
        "before" => Some(ContentTextKeyword::Before),
        "after" => Some(ContentTextKeyword::After),
        "first-letter" => Some(ContentTextKeyword::FirstLetter),
        _ => None,
    }
}

/// `leader(<leader-type>)`。CSS Content 3 §2.5.1 "The leader() function"
/// <https://www.w3.org/TR/css-content-3/#leader-function>。spec production
/// `leader( <leader-type> )` に `?` が無いため引数は必須
/// (`parse_leader_type` 失敗 = declaration drop、`leader()` 単体は spec-invalid)。
fn parse_leader_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let leader_type = parse_leader_type(input)?;
    Some(ContentComponent::Leader(leader_type))
}

/// `<leader-type> = dotted | solid | space | <string>`。[`LeaderType`] の doc
/// も参照 (keyword を正規化せず個別 variant で保持する rationale)。
fn parse_leader_type(input: &mut Parser<'_, '_>) -> Option<LeaderType> {
    if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
        return Some(LeaderType::String(SmolStr::new(s.as_ref())));
    }
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "dotted" => Some(LeaderType::Dotted),
        "solid" => Some(LeaderType::Solid),
        "space" => Some(LeaderType::Space),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssparser::ParserInput;

    fn parse(source: &str, name: &str) -> Option<PropertyValue> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_value(name, &mut parser)
    }

    #[test]
    fn color_parse_hex() {
        assert_eq!(
            parse("#ff0000", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_hex_3digit_duplicates_nibbles() {
        // CSS Color 4 §5.2 verbatim: "This syntax is often explained by saying
        // that it’s identical to a 6-digit notation obtained by "duplicating"
        // all of the digits." `#f00` == `#ff0000`.
        assert_eq!(
            parse("#f00", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_hex_4digit_duplicates_alpha_nibble() {
        // CSS Color 4 §5.2: `#rgba` becomes `#rrggbbaa`。alpha nibble `8`
        // → `0x88` = 136 (8 * 17)。
        assert_eq!(
            parse("#f008", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 136
            }))
        );
    }

    #[test]
    fn color_parse_hex_8digit_alpha_byte() {
        // CSS Color 4 §5.2 8-digit form: 末尾 byte が alpha (0..=255)。
        // `#ff000080` → alpha = 0x80 = 128。
        assert_eq!(
            parse("#ff000080", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 128
            }))
        );
    }

    #[test]
    fn color_parse_hex_case_insensitive() {
        // CSS Color 4 §5.2 verbatim: "the case of the letters doesn’t matter -
        // #00ff00 is identical to #00FF00" — `#FF0000` == `#ff0000`。
        assert_eq!(
            parse("#FF0000", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_hex_invalid_char_returns_none() {
        // spec-invalid: `g` は hex digit ではない (→ drop)。
        assert_eq!(parse("#gggggg", "color"), None);
    }

    #[test]
    fn color_parse_hex_invalid_length_returns_none() {
        // spec-invalid: hex-notation grammar は 3/4/6/8 digit のみ。
        // 5-digit は spec に無い (→ drop)。
        assert_eq!(parse("#12345", "color"), None);
        // 7-digit も同様に spec-invalid。
        assert_eq!(parse("#1234567", "color"), None);
    }

    #[test]
    fn color_parse_named() {
        assert_eq!(
            parse("red", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_rgb() {
        assert_eq!(
            parse("rgb(255, 0, 0)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    // ── rgb() / rgba() function form ──
    //
    // CSS Color 4 §5.1 legacy comma syntax の追加 form covers。
    // 1 sample あたり CssColor 値まで pin (loose `Some(_)` は mix reject 系
    // regression が silent pass するため避ける、既存 background_color assert
    // pattern に揃える)。

    #[test]
    fn color_parse_rgb_percentage_form() {
        // §5.1: `<percentage>` 0%/100% は `<number>` 0/255 と等価。
        // 100% → 1.0 unit_value → clamp_unit_f32(1.0) = round(255) = 255。
        assert_eq!(
            parse("rgb(100%, 0%, 0%)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }))
        );
    }

    #[test]
    fn color_parse_rgba_number_alpha() {
        // §5.1 alpha-value = <number> 0..=1。0.5 → clamp_unit_f32(0.5) =
        // round(127.5) = 128 (cssparser convention)。
        assert_eq!(
            parse("rgba(255, 0, 0, 0.5)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 128,
            }))
        );
    }

    #[test]
    fn color_parse_rgba_percentage_alpha() {
        // §5.1 alpha-value = <percentage> 0%..=100% は <number> 0..=1 と
        // 同じ mapping (50% → unit_value 0.5 → 128)。
        assert_eq!(
            parse("rgba(255, 0, 0, 50%)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 128,
            }))
        );
    }

    #[test]
    fn color_parse_rgb_clamps_overflow() {
        // §5.1: "Values outside these ranges are not invalid, but are
        // clamped to the ranges defined here at parsed-value time"。300 → 255。
        assert_eq!(
            parse("rgb(300, 0, 0)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }))
        );
    }

    #[test]
    fn color_parse_rgb_clamps_negative() {
        // §5.1 同上、負値も spec-valid で clamp のみ。-10 → 0。
        assert_eq!(
            parse("rgb(-10, 0, 0)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            }))
        );
    }

    #[test]
    fn color_parse_rgb_mix_number_percentage_returns_none() {
        // §5.1: legacy form は "all-number or all-percentage"、mix は禁止。
        // 1 番目 = <number> 255 → is_pct=false 固定、2 番目 `50%` は
        // expect_integer が Percentage token を reject → Err → None。
        assert_eq!(parse("rgb(255, 50%, 0)", "color"), None);
    }

    #[test]
    fn color_parse_rgb_mix_percentage_number_returns_none() {
        // 逆方向 mix (percentage → number): 1 番目 = <pct> 50% → is_pct=true
        // 固定、2 番目 `255` は expect_percentage が Number token を reject。
        assert_eq!(parse("rgb(50%, 255, 0)", "color"), None);
    }

    #[test]
    fn color_parse_rgb_modern_syntax_returns_none() {
        // §5.1 modern (space + slash) syntax `rgb(R G B / A)` は本 task
        // 対象外 (Non-goals、非対応)。1 番目 channel
        // (255) の後で `expect_comma` を要求するため、space separator は
        // fall-through で reject。
        assert_eq!(parse("rgb(255 0 0)", "color"), None);
        assert_eq!(parse("rgb(255 0 0 / 0.5)", "color"), None);
    }

    #[test]
    fn color_parse_rgb_too_few_args_returns_none() {
        // §5.1 legacy grammar は 3 channel 必須。2 個 (`rgb(255, 0)`) は
        // 3 番目 channel 手前で `)` (block 終端) に達し、expect_integer が
        // Err → None。
        assert_eq!(parse("rgb(255, 0)", "color"), None);
    }

    #[test]
    fn color_parse_rgb_too_many_args_returns_none() {
        // §5.1 legacy grammar は最大 4 slot (3 channel + optional alpha)。
        // 5 個目は `parse_nested_block` 内部の `parse_entirely` (cssparser
        // 0.37 parser.rs:1149) が exhaustion check で Err → None。
        assert_eq!(parse("rgb(255, 0, 0, 0.5, 99)", "color"), None);
    }

    #[test]
    fn color_parse_rgb_name_accepts_alpha() {
        // §5.1 alias 規定 cross-cover: rgb() name でも alpha を受理。
        // parse_rgb_function は function name に依存せず、4 番目 comma の有無
        // だけで alpha slot を判定するため、`rgb(R, G, B, A)` は valid。
        // name-based branching が retro で入った場合の regression guard。
        assert_eq!(
            parse("rgb(255, 0, 0, 0.5)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 128,
            }))
        );
    }

    #[test]
    fn color_parse_rgba_name_accepts_no_alpha() {
        // §5.1 alias 規定 cross-cover: rgba() name でも alpha を省略できる
        // (opaque と等価)。`rgba(R, G, B)` は spec grammar 上 valid で、
        // parse_rgb_function は name に依存せず 4 番目 comma 無し → a=255。
        assert_eq!(
            parse("rgba(255, 0, 0)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }))
        );
    }

    #[test]
    fn color_parse_invalid_returns_none() {
        assert_eq!(parse("bogus", "color"), None);
        assert_eq!(parse("", "color"), None);
    }

    #[test]
    fn color_parse_transparent_keyword_returns_zero_alpha() {
        // CSS Color 4 §6.3 "The transparent keyword": `transparent`
        // = rgba(0, 0, 0, 0)。Ident arm hardcodes a=255、明示 branch が無ければ
        // transparent が到達しても opaque black (`{0,0,0,255}`) になる bug の
        // regression pin。
        assert_eq!(
            parse("transparent", "color"),
            Some(PropertyValue::Color(CssColor::TRANSPARENT))
        );
    }

    // ── CssColor::from_hex direct helper contract ──
    //
    // parse_color 経由の integration test は上で網羅済み。以下は helper 自体の
    // API contract を pin する direct call test — rgb() function form
    // や future property (border-*-color 等) が同じ primitive を消費するため、
    // 内部形状の regression を早く捕まえる目的。

    #[test]
    fn css_color_from_hex_6digit_returns_channels() {
        // 6-digit form: `rrggbb` は各 2 桁を byte として解釈、alpha = 255。
        assert_eq!(
            CssColor::from_hex("336699"),
            Some(CssColor {
                r: 0x33,
                g: 0x66,
                b: 0x99,
                a: 255,
            })
        );
    }

    #[test]
    fn css_color_from_hex_3digit_expands_by_duplication() {
        // 3-digit form: 各 nibble を duplicate。`#369` == `#336699`。
        assert_eq!(CssColor::from_hex("369"), CssColor::from_hex("336699"));
    }

    #[test]
    fn css_color_from_hex_4digit_expands_alpha_nibble() {
        // 4-digit form: `#369c` == `#336699cc`。alpha nibble `c` (12) →
        // `0xcc` = 204。
        assert_eq!(CssColor::from_hex("369c"), CssColor::from_hex("336699cc"));
    }

    #[test]
    fn css_color_from_hex_8digit_carries_alpha_byte() {
        // 8-digit form: 末尾 byte がそのまま alpha (0..=255)。
        assert_eq!(
            CssColor::from_hex("336699cc"),
            Some(CssColor {
                r: 0x33,
                g: 0x66,
                b: 0x99,
                a: 0xcc,
            })
        );
    }

    #[test]
    fn css_color_from_hex_mixed_case_accepted() {
        // §5.2 case-insensitive: `#aBcDeF` == `#abcdef`。
        assert_eq!(CssColor::from_hex("aBcDeF"), CssColor::from_hex("abcdef"));
    }

    #[test]
    fn css_color_from_hex_invalid_length_returns_none() {
        // hex-notation grammar 外の length は spec-invalid → None。
        assert_eq!(CssColor::from_hex(""), None);
        assert_eq!(CssColor::from_hex("1"), None);
        assert_eq!(CssColor::from_hex("12"), None);
        assert_eq!(CssColor::from_hex("12345"), None);
        assert_eq!(CssColor::from_hex("1234567"), None);
        assert_eq!(CssColor::from_hex("123456789"), None);
    }

    #[test]
    fn css_color_from_hex_non_hex_char_returns_none() {
        // non-hex byte → None (nibble parse で早期 fail)。
        assert_eq!(CssColor::from_hex("gggggg"), None);
        assert_eq!(CssColor::from_hex("12x456"), None);
        // 3-digit 内の non-hex も同様。
        assert_eq!(CssColor::from_hex("f0z"), None);
    }

    // ── background-color (CSS Backgrounds 3 §2.2) ──
    //
    // 5-sample accept pin (task description Verification #4):
    // named / hex / rgb() / rgba() / transparent が
    // `Some(PropertyValue::BackgroundColor(<exact RGBA>))` を返す。
    //
    // exact RGBA assert が必要な理由: `Some(_)` の loose form だと
    // `parse_color` の Ident arm が transparent に a=255 を返す regression
    // (opaque black に落ちる bug) を silent pass してしまうため、
    // 5 sample 全て CssColor 値まで pin する。

    #[test]
    fn background_color_parse_named() {
        assert_eq!(
            parse("red", "background-color"),
            Some(PropertyValue::BackgroundColor(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn background_color_parse_hex() {
        assert_eq!(
            parse("#ff0000", "background-color"),
            Some(PropertyValue::BackgroundColor(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn background_color_parse_rgb() {
        assert_eq!(
            parse("rgb(255, 0, 0)", "background-color"),
            Some(PropertyValue::BackgroundColor(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn background_color_parse_rgba() {
        // rgba() alpha は number literal (0.0..=1.0)、clamp_unit_f32 で
        // 0..=255 に mapping。0.5 → 128 (rounding は cssparser 準拠)。
        assert_eq!(
            parse("rgba(0, 0, 0, 0.5)", "background-color"),
            Some(PropertyValue::BackgroundColor(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 128
            }))
        );
    }

    #[test]
    fn background_color_parse_transparent() {
        // CSS Color 4 §6.3 "The transparent keyword": shorthand for
        // rgba(0, 0, 0, 0)。spec initial value と一致 (CSS Backgrounds 3 §2.2)。
        assert_eq!(
            parse("transparent", "background-color"),
            Some(PropertyValue::BackgroundColor(CssColor::TRANSPARENT))
        );
    }

    #[test]
    fn background_color_parse_invalid_returns_none() {
        // `none` は <color> grammar に含まれない spec-invalid keyword (task
        // Non-goals: spec-invalid → drop)。
        assert_eq!(parse("none", "background-color"), None);
        // hsl() は CSS Color 4 spec-valid だが現状未対応 (task
        // Non-goals: 非対応、CSS Color 4 拡張は defer)。
        assert_eq!(parse("hsl(0, 100%, 50%)", "background-color"), None);
    }

    #[test]
    fn background_color_key_returns_background_color() {
        // PropertyValue::BackgroundColor → PropertyKey::BackgroundColor (cascade
        // winner 選択の discriminant 導線、sibling `Color` key() と対称)。
        let v = PropertyValue::BackgroundColor(CssColor::TRANSPARENT);
        assert_eq!(v.key(), PropertyKey::BackgroundColor);
    }

    #[test]
    fn font_size_parse_px() {
        assert_eq!(
            parse("16px", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(16.0)))
        );
    }

    /// CSS Fonts 4 §2.5 の grammar `<length-percentage [0,∞]>` は font-relative
    /// unit と percentage を含む。cascade の phase 2 (絶対化) が入ったので、
    /// これらを parse 段で drop しなくなった。
    #[test]
    fn font_size_accepts_font_relative_and_percentage() {
        assert_eq!(
            parse("1.5em", "font-size"),
            Some(PropertyValue::FontSize(Length::Em(1.5)))
        );
        assert_eq!(
            parse("2rem", "font-size"),
            Some(PropertyValue::FontSize(Length::Rem(2.0)))
        );
        assert_eq!(
            parse("12pt", "font-size"),
            Some(PropertyValue::FontSize(Length::Pt(12.0)))
        );
        assert_eq!(
            parse("150%", "font-size"),
            Some(PropertyValue::FontSize(Length::Percent(150.0)))
        );
    }

    /// `math` は spec-valid だが未実装
    /// (MathML scaling algorithm 未対応) として drop。
    /// `<absolute-size>` / `<relative-size>` は受理済み —
    /// 別 test (`font_size_accepts_absolute_size_keywords` /
    /// `font_size_accepts_relative_size_keywords`) 参照。
    #[test]
    fn font_size_rejects_math_keyword() {
        assert_eq!(parse("math", "font-size"), None);
    }

    /// CSS Fonts 4 §2.5.1 <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping>
    /// の scaling-factor table 全 8 keyword。`medium` = raikiri の固定基準
    /// (16px) そのもの、他は table の分数を掛けたもの
    /// (`resolve_relative_weight` 前例に倣い浮動小数 literal ではなく分数式で
    /// 期待値を書く — 丸め誤差の議論を spec 引用だけで閉じるため)。
    #[test]
    fn font_size_accepts_absolute_size_keywords() {
        const MEDIUM: f32 = 16.0;
        let cases: &[(&str, f32)] = &[
            ("xx-small", MEDIUM * (3.0 / 5.0)),
            ("x-small", MEDIUM * (3.0 / 4.0)),
            ("small", MEDIUM * (8.0 / 9.0)),
            ("medium", MEDIUM),
            ("large", MEDIUM * (6.0 / 5.0)),
            ("x-large", MEDIUM * (3.0 / 2.0)),
            ("xx-large", MEDIUM * (2.0 / 1.0)),
            ("xxx-large", MEDIUM * (3.0 / 1.0)),
        ];
        for (keyword, px) in cases {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                parse(keyword, "font-size"),
                Some(PropertyValue::FontSize(Length::Px(*px))),
                "keyword = {keyword}"
            );
        }
    }

    /// CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII
    /// case-insensitive。sibling `font_weight_keyword_case_insensitive` と同 pattern。
    #[test]
    fn font_size_absolute_size_keyword_case_insensitive() {
        assert_eq!(
            parse("MEDIUM", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(16.0)))
        );
        assert_eq!(
            parse("Large", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(16.0 * (6.0 / 5.0))))
        );
    }

    /// `<relative-size>` (`larger` / `smaller`) は parse 段では解決せず
    /// `PropertyValue::FontSizeRelative` をそのまま返す — 解決 (親の
    /// computed font-size に対する read-modify-write) は
    /// `crate::cascade` の責務 (`bolder` / `lighter` と同型)。 // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    #[test]
    fn font_size_accepts_relative_size_keywords() {
        assert_eq!(
            parse("larger", "font-size"),
            Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger))
        );
        assert_eq!(
            parse("smaller", "font-size"),
            Some(PropertyValue::FontSizeRelative(RelativeFontSize::Smaller))
        );
        assert_eq!(
            parse("LARGER", "font-size"),
            Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger))
        );
    }

    /// `font-size: 12px` と `font-size: larger` は同じ property を競合する
    /// (`PropertyValue::FontSizeRelative` doc 参照) — 別 key だと両方が
    /// cascade で「勝つ」事態が起き spec (1 property = 1 winner) と食い違う。
    #[test]
    fn font_size_relative_shares_property_key_with_font_size() {
        assert_eq!(
            PropertyValue::FontSize(Length::Px(12.0)).key(),
            PropertyKey::FontSize
        );
        assert_eq!(
            PropertyValue::FontSizeRelative(RelativeFontSize::Larger).key(),
            PropertyKey::FontSize
        );
    }

    #[test]
    fn font_size_rejects_negative() {
        // spec grammar `[0,∞]`: 負値は全 unit で drop (px だけではない)。
        assert_eq!(parse("-10px", "font-size"), None);
        assert_eq!(parse("-0.5px", "font-size"), None);
        assert_eq!(parse("-1em", "font-size"), None);
        assert_eq!(parse("-2rem", "font-size"), None);
        assert_eq!(parse("-12pt", "font-size"), None);
        assert_eq!(parse("-50%", "font-size"), None);
        // 追加した unit も `length_payload` 経由で同じ
        // non-negative check を通ることを pin。
        assert_eq!(parse("-1ex", "font-size"), None);
        assert_eq!(parse("-1cm", "font-size"), None);
    }

    #[test]
    fn font_size_accepts_additional_units() {
        // CSS Fonts 4 §2.5 `<length-percentage [0,∞]>` —
        // 追加した font-relative / absolute unit も `font-size` 上で受理される
        // (`parse_length_value` の dispatch に mode 差は無い)。
        assert_eq!(
            parse("2ex", "font-size"),
            Some(PropertyValue::FontSize(Length::Ex(2.0)))
        );
        assert_eq!(
            parse("1cm", "font-size"),
            Some(PropertyValue::FontSize(Length::Cm(1.0)))
        );
    }

    #[test]
    fn font_size_accepts_lh_and_rlh() {
        // CSS Fonts 4's `font-size` grammar
        // (`<absolute-size> | <relative-size> | <length-percentage [0,∞]>`)
        // has no carve-out excluding `lh`/`rlh` from `<length-percentage>`'s
        // `<length>` component (CSS Values 4 §6.1.1) — `font-size: 1lh` /
        // `font-size: 1rlh` are spec-valid and must survive parsing so the
        // cascade can pick them as a winner (dropping at parse time, as this
        // crate previously did, can change *which
        // declaration wins* the cascade — a stronger effect than an
        // incorrectly-resolved value). Resolution against the parent's used
        // line-height is `crate::resolve::resolve_font_size`'s concern, not
        // this parser's — pinned by that module's tests, not here.
        assert_eq!(
            parse("1lh", "font-size"),
            Some(PropertyValue::FontSize(Length::Lh(1.0)))
        );
        assert_eq!(
            parse("1rlh", "font-size"),
            Some(PropertyValue::FontSize(Length::Rlh(1.0)))
        );
    }

    #[test]
    fn font_size_rejects_negative_lh_and_rlh() {
        // The grammar's `[0,∞]` non-negative constraint (`parse_font_size`
        // doc "Non-negative constraint" 節) applies to `lh`/`rlh` the same as
        // every other `Length` variant — `length_payload` reads their inner
        // `f32` generically, so this falls out of the existing post-filter
        // without a dedicated branch.
        assert_eq!(parse("-1lh", "font-size"), None);
        assert_eq!(parse("-1rlh", "font-size"), None);
    }

    #[test]
    fn font_size_accepts_zero() {
        // spec `[0,∞]` の閉区間下端。`0px` は Dimension arm、bare `0` は
        // CSS Values 3 §5 unitless-zero clause の Number arm を通し、
        // parse_font_size の非負 Px post-filter を pass。
        assert_eq!(
            parse("0px", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(0.0)))
        );
        assert_eq!(
            parse("0", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(0.0)))
        );
    }

    #[test]
    fn font_family_parse_comma_list() {
        let got = parse(r#"Arial, "Times New Roman", serif"#, "font-family");
        let expected = Some(PropertyValue::FontFamily(Arc::new(vec![
            Atom::from("Arial"),
            Atom::from("Times New Roman"),
            Atom::from("serif"),
        ])));
        assert_eq!(got, expected);
    }

    #[test]
    fn font_family_unquoted_multi_word_single_family() {
        // CSS4: unquoted multi-word family name = ident sequence joined by space。
        let got = parse("Times New Roman", "font-family");
        let expected = Some(PropertyValue::FontFamily(Arc::new(vec![Atom::from(
            "Times New Roman",
        )])));
        assert_eq!(got, expected);
    }

    /// `initial_font_family()` は呼び出しごとに独立した call site でも
    /// **同一** underlying `Vec` allocation を指す (`Arc::ptr_eq` = true) —
    /// `OnceLock` 経由の shared slot であることの直接 pin。
    ///
    /// この pin は cascade level の test (`mod@crate::cascade` の
    /// `initial_font_family_shares_arc_slot_across_independent_cascade_runs`
    /// 等) では**代替できない** — `font-family` は inherited なので、単一
    /// document 内の兄弟 element は `SpecifiedValues::inherit_from` の
    /// 「親の Arc を bump」経路で共有される。これは同 document 内で
    /// `initial_font_family()` が実質 1 回しか呼ばれないことを意味し、
    /// ここで `OnceLock` を外して per-call `Arc::new(..)` に戻す regression を
    /// 混入させても、その cascade level test は green のままになる
    /// (実際に perturbation で確認済み)。
    /// 本 test は `initial_font_family()` を直接 2 回呼ぶことで、この
    /// inheritance-sharing の死角を回避する。
    #[test]
    fn initial_font_family_shares_arc_slot_across_calls() {
        assert!(Arc::ptr_eq(&initial_font_family(), &initial_font_family()));
    }

    /// `font-weight` の parse 期待値を組み立てる test-local helper。
    fn fw(w: f32) -> Option<PropertyValue> {
        Some(PropertyValue::FontWeight(FontWeightValue::Absolute(w)))
    }

    #[test]
    fn font_weight_parse_integer() {
        assert_eq!(parse("400", "font-weight"), fw(400.0));
        assert_eq!(parse("700", "font-weight"), fw(700.0));
    }

    #[test]
    fn font_weight_parse_keyword_normal() {
        // CSS Fonts 4 §2.2: normal = 400。
        assert_eq!(parse("normal", "font-weight"), fw(400.0));
    }

    #[test]
    fn font_weight_parse_keyword_bold() {
        // CSS Fonts 4 §2.2: bold = 700。
        assert_eq!(parse("bold", "font-weight"), fw(700.0));
    }

    #[test]
    fn font_weight_keyword_case_insensitive() {
        // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII
        // case-insensitive で照合する。
        assert_eq!(parse("NORMAL", "font-weight"), fw(400.0));
        assert_eq!(parse("Bold", "font-weight"), fw(700.0));
    }

    #[test]
    fn font_weight_accepts_full_spec_range() {
        // CSS Fonts 4 §2.2 `<font-weight-absolute> = [ normal | bold |
        // <number [1,1000]> ]`。旧実装は `[100, 900]` に絞っていたが spec は
        // `[1, 1000]`。
        assert_eq!(parse("1", "font-weight"), fw(1.0));
        assert_eq!(parse("1000", "font-weight"), fw(1000.0));
        assert_eq!(parse("50", "font-weight"), fw(50.0));
        // 旧 range の両端も当然 valid のまま (regression guard)。
        assert_eq!(parse("100", "font-weight"), fw(100.0));
        assert_eq!(parse("900", "font-weight"), fw(900.0));
    }

    #[test]
    fn font_weight_rejects_out_of_range_number() {
        // spec-invalid — spec grammar。§2.2 "Only values greater than or
        // equal to 1, and less than or equal to 1000, are valid, and all other
        // values are invalid"。
        assert_eq!(parse("0", "font-weight"), None);
        assert_eq!(parse("1001", "font-weight"), None);
        assert_eq!(parse("-100", "font-weight"), None);
        // 範囲判定は **丸める前の指定値** に対して行う: 丸めれば範囲内に入る
        // 値でも spec 上は invalid。
        assert_eq!(parse("0.6", "font-weight"), None);
        assert_eq!(parse("1000.4", "font-weight"), None);
        // 非有限値。`1e400` は f32 に収まらず ±inf に overflow するため、
        // `value <= 1000.0` (または `>= 1.0`) が成立せず reject される
        // (§2.2 "all other values are invalid" と一致)。NaN は両比較が false。
        // `nan` / `inf` は `<number>` production ではなく Ident token なので
        // keyword arm にも該当せず reject される。
        assert_eq!(parse("1e400", "font-weight"), None);
        assert_eq!(parse("-1e400", "font-weight"), None);
        assert_eq!(parse("nan", "font-weight"), None);
        assert_eq!(parse("inf", "font-weight"), None);
    }

    #[test]
    fn font_weight_computed_preserves_fractional_precision() {
        // **spec 準拠 pin。** §2.2 の computed value は "a number" であり、
        // §2.2.2 "Missing weights" <https://www.w3.org/TR/css-fonts-4/#missing-weights>
        // は "Fractional weights are valid" と明言する。旧実装 (computed side が
        // `u16`) は parse 時に round-half-away-from-zero で整数化しており、
        // これは spec 沈黙点の選択ではなく表現上の制約による既知 divergence
        // だった。payload / `ComputedValues.font_weight`
        // を `f32` に格上げしたことで丸め自体が不要になり、本 test はその
        // 解消を pin する — もはや丸めていないことの regression guard。
        // 全て 2 進数で厳密表現可能な小数 (`.5` / `.25`) — parse 側と期待値の
        // 独立な文字列→f32 変換が bit-for-bit 一致することを保証でき、
        // 丸め誤差を懸念せず `assert_eq!` で直接比較できる。
        assert_eq!(parse("100.5", "font-weight"), fw(100.5));
        assert_eq!(parse("250.75", "font-weight"), fw(250.75));
        assert_eq!(parse("399.5", "font-weight"), fw(399.5));
        assert_eq!(parse("999.5", "font-weight"), fw(999.5));
    }

    #[test]
    fn font_weight_wpt_font_weight_computed_150_25() {
        // WPT css/css-fonts/parsing/font-weight-computed.html:
        // `test_computed_value('font-weight', '150.25')` — 2-arg 形は
        // computed === specified を pin する。parse 結果 (specified-equivalent
        // な `PropertyValue`) がそのまま `150.25` を保持することを確認する。
        // cascade を経由した computed 側の同値 pin は
        // `crate::cascade::tests::font_weight_wpt_font_weight_computed_150_25`。
        assert_eq!(parse("150.25", "font-weight"), fw(150.25));
    }

    #[test]
    fn font_weight_accepts_scientific_notation_number() {
        // `int_value` matcher から `value` (f32) 参照に変えた副次効果。
        // `1e3` は CSS Values 3 の `<number>` production として spec-valid
        // なので受理が正しい。
        assert_eq!(parse("1e3", "font-weight"), fw(1000.0));
    }

    #[test]
    fn font_weight_parses_relative_keywords_as_sentinels() {
        // CSS Fonts 4 §2.2: `bolder` / `lighter` は継承値依存の relative
        // weight。parse 段では解けないので sentinel variant を返し、cascade が
        // 親の computed weight から解決する。
        assert_eq!(
            parse("bolder", "font-weight"),
            Some(PropertyValue::FontWeight(FontWeightValue::Bolder))
        );
        assert_eq!(
            parse("lighter", "font-weight"),
            Some(PropertyValue::FontWeight(FontWeightValue::Lighter))
        );
        // CSS Values 3 §3.1: relative keyword も ASCII case-insensitive。
        assert_eq!(
            parse("BOLDER", "font-weight"),
            Some(PropertyValue::FontWeight(FontWeightValue::Bolder))
        );
        assert_eq!(
            parse("Lighter", "font-weight"),
            Some(PropertyValue::FontWeight(FontWeightValue::Lighter))
        );
    }

    #[test]
    fn font_weight_rejects_unknown_ident() {
        // spec-invalid keyword → declaration drop。
        assert_eq!(parse("normal-ish", "font-weight"), None);
        assert_eq!(parse("super-bold", "font-weight"), None);
    }

    #[test]
    fn unknown_property_returns_none() {
        // `background-color` / `padding` / `margin` / `width` / `height` /
        // `float` が順次実装済 = ここから除外。
        // `cursor` (CSS Basic User Interface Module Level 3
        // <https://www.w3.org/TR/css-ui-3/#cursor>) は現時点で
        // parse_value dispatch に未登録 → fall-through で None が返る
        // canonical unknown-property canary。実装され次第、別の未実装
        // property 名へ再び移設すること。
        assert_eq!(parse("pointer", "cursor"), None);
    }

    // ── Display (CSS Display 3 §2) ─────────────

    #[test]
    fn display_parse_block() {
        assert_eq!(
            parse("block", "display"),
            Some(PropertyValue::Display(DisplayValue::Block))
        );
    }

    #[test]
    fn display_parse_inline() {
        assert_eq!(
            parse("inline", "display"),
            Some(PropertyValue::Display(DisplayValue::Inline))
        );
    }

    #[test]
    fn display_parse_inline_block() {
        // CSS Display 3 §2 <display-legacy>
        assert_eq!(
            parse("inline-block", "display"),
            Some(PropertyValue::Display(DisplayValue::InlineBlock))
        );
    }

    #[test]
    fn display_parse_none() {
        // CSS Display 3 §2 <display-box>
        assert_eq!(
            parse("none", "display"),
            Some(PropertyValue::Display(DisplayValue::None))
        );
    }

    #[test]
    fn display_parse_flex() {
        // CSS Display 3 §2.2 "Inner Display Layout Models" — `<display-inside>`
        // keyword, outer-defaulting rule makes it equivalent to `block flex`.
        assert_eq!(
            parse("flex", "display"),
            Some(PropertyValue::Display(DisplayValue::Flex))
        );
    }

    #[test]
    fn display_parse_grid() {
        // CSS Display 3 §2.2 "Inner Display Layout Models" — `<display-inside>`
        // keyword, outer-defaulting rule makes it equivalent to `block grid`.
        assert_eq!(
            parse("grid", "display"),
            Some(PropertyValue::Display(DisplayValue::Grid))
        );
    }

    #[test]
    fn display_rejects_unknown_ident() {
        // block / inline / inline-block / none / flex / grid 以外は
        // spec-valid でも未実装のため silent drop。
        // inline-flex / inline-grid / table* / list-item / flow-root /
        // contents は将来の layout 対応で扱う予定。
        assert_eq!(parse("inline-flex", "display"), None);
        assert_eq!(parse("inline-grid", "display"), None);
        assert_eq!(parse("table", "display"), None);
        assert_eq!(parse("table-row", "display"), None);
        assert_eq!(parse("list-item", "display"), None);
        assert_eq!(parse("flow-root", "display"), None);
        assert_eq!(parse("contents", "display"), None);
    }

    #[test]
    fn display_rejects_non_ident() {
        assert_eq!(parse("16px", "display"), None);
        assert_eq!(parse("100", "display"), None);
    }

    #[test]
    fn display_is_case_insensitive() {
        // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII case-insensitive
        assert_eq!(
            parse("BLOCK", "display"),
            Some(PropertyValue::Display(DisplayValue::Block))
        );
        assert_eq!(
            parse("Inline", "display"),
            Some(PropertyValue::Display(DisplayValue::Inline))
        );
        assert_eq!(
            parse("INLINE-BLOCK", "display"),
            Some(PropertyValue::Display(DisplayValue::InlineBlock))
        );
        assert_eq!(
            parse("Inline-Block", "display"),
            Some(PropertyValue::Display(DisplayValue::InlineBlock))
        );
        assert_eq!(
            parse("NONE", "display"),
            Some(PropertyValue::Display(DisplayValue::None))
        );
        assert_eq!(
            parse("None", "display"),
            Some(PropertyValue::Display(DisplayValue::None))
        );
        assert_eq!(
            parse("FLEX", "display"),
            Some(PropertyValue::Display(DisplayValue::Flex))
        );
        assert_eq!(
            parse("Grid", "display"),
            Some(PropertyValue::Display(DisplayValue::Grid))
        );
    }

    // ── box-sizing (CSS Sizing 3 §3.3) ────────
    //
    // Verification anchors:
    //   #1 content-box → Some(BoxSizing::ContentBox)
    //   #2 border-box  → Some(BoxSizing::BorderBox)
    //   #3 padding-box → None (spec 外、CSS UI 3 draft の削除済 keyword)
    //   #4 initial + #5 non-inheritance test は crate::computed 側
    //
    // 37n sibling: `display_*` / `text_align_*` の keyword parser test 群と同構造。

    #[test]
    fn box_sizing_parse_content_box() {
        assert_eq!(
            parse("content-box", "box-sizing"),
            Some(PropertyValue::BoxSizing(BoxSizing::ContentBox))
        );
    }

    #[test]
    fn box_sizing_parse_border_box() {
        assert_eq!(
            parse("border-box", "box-sizing"),
            Some(PropertyValue::BoxSizing(BoxSizing::BorderBox))
        );
    }

    #[test]
    fn box_sizing_rejects_unknown_ident() {
        // spec-invalid (→ drop):
        // - `padding-box` は CSS-UI 3 draft 相当だが css-sizing-3 では削除済み
        //   (spec note "supersedes the one in `[CSS-UI-3]`")、
        // - `margin-box` は grammar 外の任意 ident。
        assert_eq!(parse("padding-box", "box-sizing"), None);
        assert_eq!(parse("margin-box", "box-sizing"), None);
        assert_eq!(parse("bogus", "box-sizing"), None);
    }

    #[test]
    fn box_sizing_rejects_css_wide_keyword() {
        // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
        // canonical: PropertyValue doc「CSS-wide keyword」節。
        assert_eq!(parse("inherit", "box-sizing"), None);
        assert_eq!(parse("initial", "box-sizing"), None);
        assert_eq!(parse("unset", "box-sizing"), None);
        assert_eq!(parse("revert", "box-sizing"), None);
        assert_eq!(parse("revert-layer", "box-sizing"), None);
    }

    #[test]
    fn box_sizing_rejects_non_ident() {
        assert_eq!(parse("16px", "box-sizing"), None);
        assert_eq!(parse("100", "box-sizing"), None);
    }

    #[test]
    fn box_sizing_is_case_insensitive() {
        // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII case-insensitive
        // (37n sibling `display_is_case_insensitive` と同 flavor)。
        assert_eq!(
            parse("CONTENT-BOX", "box-sizing"),
            Some(PropertyValue::BoxSizing(BoxSizing::ContentBox))
        );
        assert_eq!(
            parse("Border-Box", "box-sizing"),
            Some(PropertyValue::BoxSizing(BoxSizing::BorderBox))
        );
    }

    #[test]
    fn box_sizing_key_returns_box_sizing() {
        // PropertyValue::BoxSizing → PropertyKey::BoxSizing (cascade winner
        // 選択の discriminant 導線、sibling `Display` / `TextAlign` key() と対称)。
        let v = PropertyValue::BoxSizing(BoxSizing::BorderBox);
        assert_eq!(v.key(), PropertyKey::BoxSizing);
    }

    // ── counter-* (CSS Lists 3 §4) ──

    // `PropertyValue::Counter*(Arc<Vec<..>>)` に wrap したため、
    // literal test 比較用に Arc<Vec<..>> を返す helper に切り替え
    // (content/string_set helper と同 pattern)。
    fn counter_pairs(pairs: &[(&str, i32)]) -> Arc<Vec<(SmolStr, i32)>> {
        Arc::new(
            pairs
                .iter()
                .map(|(name, value)| (SmolStr::new(name), *value))
                .collect(),
        )
    }

    #[test]
    fn counter_reset_single_name_defaults_to_zero() {
        // spec: reset の default は 0
        assert_eq!(
            parse("chapter", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[(
                "chapter", 0
            )])))
        );
    }

    #[test]
    fn counter_reset_multiple_names_with_mixed_ints() {
        // 2 番目に integer が付く → 1 番目は default 0、2 番目は 3
        assert_eq!(
            parse("chapter section 3", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[
                ("chapter", 0),
                ("section", 3)
            ])))
        );
    }

    #[test]
    fn counter_reset_none_returns_empty_vec() {
        // spec: `none` は空リストと同等 (top-level alternative)
        // empty case は shared Arc slot (`empty_counter_entries`) を使う。
        assert_eq!(
            parse("none", "counter-reset"),
            Some(PropertyValue::CounterReset(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_reset_rejects_number_first() {
        // 先頭が number → ident が来るまで peel できず empty → None (drop)
        // spec §4: `<counter-name> = <custom-ident>` (数値は counter-name ではない)
        assert_eq!(parse("123 abc", "counter-reset"), None);
    }

    #[test]
    fn counter_increment_single_name_defaults_to_one() {
        // spec: increment の default は 1
        assert_eq!(
            parse("chapter", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[(
                "chapter", 1
            )])))
        );
    }

    #[test]
    fn counter_increment_mixed_int_and_default() {
        // `chapter 2 section` → chapter=2、section=default(1)
        assert_eq!(
            parse("chapter 2 section", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[
                ("chapter", 2),
                ("section", 1)
            ])))
        );
    }

    #[test]
    fn counter_increment_accepts_negative_integer() {
        // spec §4: <integer> — negative も valid (counter を decrement する用途)
        assert_eq!(
            parse("chapter -1", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[(
                "chapter", -1
            )])))
        );
    }

    #[test]
    fn counter_increment_none_returns_empty_vec() {
        // empty case は shared Arc slot を使う。
        assert_eq!(
            parse("none", "counter-increment"),
            Some(PropertyValue::CounterIncrement(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_set_defaults_to_zero() {
        // spec: set の default は 0
        assert_eq!(
            parse("page 5 note", "counter-set"),
            Some(PropertyValue::CounterSet(counter_pairs(&[
                ("page", 5),
                ("note", 0)
            ])))
        );
    }

    #[test]
    fn counter_set_none_returns_empty_vec() {
        // empty case は shared Arc slot を使う。
        assert_eq!(
            parse("none", "counter-set"),
            Some(PropertyValue::CounterSet(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_reset_is_case_insensitive_on_none() {
        // CSS spec: keyword `none` は ASCII case-insensitive
        // empty case は shared Arc slot を使う。
        assert_eq!(
            parse("NONE", "counter-reset"),
            Some(PropertyValue::CounterReset(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_reset_rejects_reserved_css_wide_keyword_as_name() {
        // spec §4: <counter-name> excludes CSS-wide keywords + `default`。
        // 先頭 ident が `inherit` → try_parse rewind で empty result → None。
        assert_eq!(parse("inherit", "counter-reset"), None);
        assert_eq!(parse("initial", "counter-reset"), None);
        assert_eq!(parse("unset", "counter-reset"), None);
        assert_eq!(parse("revert", "counter-reset"), None);
        assert_eq!(parse("default", "counter-reset"), None);
    }

    #[test]
    fn counter_reset_accepts_negative_integer() {
        // CSS Values 3 §4.2 "Integers: the <integer> type"
        // (https://www.w3.org/TR/css-values-3/#integers): <integer> は負値を含む。
        // increment だけでなく reset / set も同一 grammar。
        assert_eq!(
            parse("chapter -5", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[(
                "chapter", -5
            )])))
        );
    }

    #[test]
    fn counter_set_accepts_negative_integer() {
        // 同上 (parity with reset/increment negative-integer coverage)。
        assert_eq!(
            parse("page -3", "counter-set"),
            Some(PropertyValue::CounterSet(counter_pairs(&[("page", -3)])))
        );
    }

    // ── content property (CSS Content 3 §2) ──
    //
    // task の verification items は spec-derived。task 記述の
    // `raikiri_traits::ContentValueItem` は下流 (raikiri-dom) mapping 先。
    // raikiri-style は raikiri-traits に依存しない leaf crate
    // のため、counter-* precedent に倣い local `ContentComponent` を emit
    // する (原則 1: 前例主義)。Symbol → SmolStr、Url → String へ substitution。

    fn content_items(source: &str) -> Vec<ContentComponent> {
        match parse(source, "content") {
            // PropertyValue::Content(Arc<Vec<..>>) を expose するため
            // (*v).clone() で Vec を deref-clone。tests は既存 shape のまま検証。
            Some(PropertyValue::Content(v)) => (*v).clone(),
            other => panic!("expected PropertyValue::Content, got {other:?}"),
        }
    }

    #[test]
    fn content_parse_string_function() {
        // Verification 1: content: string(my_str)
        // → ContentComponent::String { name: "my_str", fetch: default (First) }
        let items = content_items("string(my_str)");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::String {
                name: SmolStr::new("my_str"),
                fetch: StringFetchMode::First,
            }
        );
    }

    #[test]
    fn content_parse_counter_function() {
        // Verification 2: content: counter(chapter)
        // → ContentComponent::Counter { name: "chapter", style: default (Decimal) }
        let items = content_items("counter(chapter)");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_counters_function() {
        // Verification 3: content: counters(section, ".")
        // → ContentComponent::Counters { name, separator: ".", style: default }
        let items = content_items(r#"counters(section, ".")"#);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Counters {
                name: SmolStr::new("section"),
                separator: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_target_counter_function() {
        // Verification 4: content: target-counter(url("#anchor"), page)
        // → ContentComponent::TargetCounter { url: "#anchor", name: "page", style: default }
        let items = content_items(r##"target-counter(url("#anchor"), page)"##);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::TargetCounter {
                url: String::from("#anchor"),
                name: SmolStr::new("page"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_target_counters_function() {
        // Verification 5: content: target-counters(url("#anchor"), section, ".")
        // → ContentComponent::TargetCounters { url, name, separator, style: default }
        let items = content_items(r##"target-counters(url("#anchor"), section, ".")"##);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::TargetCounters {
                url: String::from("#anchor"),
                name: SmolStr::new("section"),
                separator: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_target_text_first_letter() {
        // Verification 6: content: target-text(url("#anchor"), first-letter)
        // → ContentComponent::TargetText { url, part: ContentPart::FirstLetter }
        //
        // NB: task description の "content-first-letter" は spec (§2.6.3
        // `[ content | before | after | first-letter ]?`) と食い違うため、
        // spec-correct な `first-letter` を採用 (task 側の記述が誤りと
        // 判明したための訂正)。
        let items = content_items(r##"target-text(url("#anchor"), first-letter)"##);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::TargetText {
                url: String::from("#anchor"),
                part: ContentPart::FirstLetter,
            }
        );
    }

    #[test]
    fn content_parse_attr_function() {
        // Verification 7: content: attr(href) → ContentComponent::Attr { name: "href" }
        let items = content_items("attr(href)");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Attr {
                name: SmolStr::new("href"),
            }
        );
    }

    #[test]
    fn content_parse_literal_string() {
        // Verification 8: content: "hello" → ContentComponent::Literal("hello")
        let items = content_items(r#""hello""#);
        assert_eq!(
            items,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    #[test]
    fn content_parse_mixed_sequence_preserves_order() {
        // Verification 9: content: "Chapter " counter(chapter) ": " string(chapter_title)
        // → 4-item Vec in order
        let items = content_items(r#""Chapter " counter(chapter) ": " string(chapter_title)"#);
        assert_eq!(items.len(), 4, "expected 4 items, got {items:?}");
        assert_eq!(
            items[0],
            ContentComponent::Literal(SmolStr::new("Chapter "))
        );
        assert_eq!(
            items[1],
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
        assert_eq!(items[2], ContentComponent::Literal(SmolStr::new(": ")));
        assert_eq!(
            items[3],
            ContentComponent::String {
                name: SmolStr::new("chapter_title"),
                fetch: StringFetchMode::First,
            }
        );
    }

    // ── content property: image / contents / <quote> / leader() (CSS Content 3
    // §2.2 / §2.3 / §2.4.2 / §2.5.1 — under-accept fix、CssContent3 mode arm) ──

    #[test]
    fn content_parse_image_url_quoted_form() {
        // `<image>` の `<url>` alternative、`url("...")` (quoted) form。
        let items = content_items(r#"url("cat.png")"#);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Image {
                url: String::from("cat.png"),
            }
        );
    }

    #[test]
    fn content_parse_image_url_unquoted_form() {
        // `<image>` の `<url>` alternative、`url(...)` (unquoted url-token) form。
        let items = content_items("url(cat.png)");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Image {
                url: String::from("cat.png"),
            }
        );
    }

    #[test]
    fn content_bare_string_is_still_literal_not_image() {
        // Regression pin: `<image>` production は `<url> | <gradient>` のみで
        // bare `<string>` を含まない (target-* の `[<string>|<url>]` とは別
        // grammar)。`expect_url` は quoted string 単体を受理しないため
        // `content: "cat.png"` は Literal のまま — Image への誤変換防止。
        let items = content_items(r#""cat.png""#);
        assert_eq!(
            items,
            vec![ContentComponent::Literal(SmolStr::new("cat.png"))]
        );
    }

    #[test]
    fn content_parse_contents_keyword() {
        // CSS Content 3 §2.3 "Elemental Content: the contents keyword"。
        let items = content_items("contents");
        assert_eq!(items, vec![ContentComponent::Contents]);
    }

    #[test]
    fn content_contents_keyword_is_case_insensitive() {
        let items = content_items("CoNtEnTs");
        assert_eq!(items, vec![ContentComponent::Contents]);
    }

    #[test]
    fn content_parse_quote_keywords() {
        // CSS Content 3 §2.4.2 `<quote> = open-quote | close-quote |
        // no-open-quote | no-close-quote` の 4 keyword 全数検証。
        assert_eq!(
            content_items("open-quote"),
            vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
        );
        assert_eq!(
            content_items("close-quote"),
            vec![ContentComponent::Quote(QuoteKeyword::CloseQuote)]
        );
        assert_eq!(
            content_items("no-open-quote"),
            vec![ContentComponent::Quote(QuoteKeyword::NoOpenQuote)]
        );
        assert_eq!(
            content_items("no-close-quote"),
            vec![ContentComponent::Quote(QuoteKeyword::NoCloseQuote)]
        );
    }

    #[test]
    fn content_quote_keyword_is_case_insensitive() {
        let items = content_items("OPEN-QUOTE");
        assert_eq!(
            items,
            vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
        );
    }

    #[test]
    fn content_parse_leader_dotted_solid_space_keywords() {
        // CSS Content 3 §2.5.1 `<leader-type> = dotted | solid | space | <string>`。
        assert_eq!(
            content_items("leader(dotted)"),
            vec![ContentComponent::Leader(LeaderType::Dotted)]
        );
        assert_eq!(
            content_items("leader(solid)"),
            vec![ContentComponent::Leader(LeaderType::Solid)]
        );
        assert_eq!(
            content_items("leader(space)"),
            vec![ContentComponent::Leader(LeaderType::Space)]
        );
    }

    #[test]
    fn content_parse_leader_custom_string() {
        let items = content_items(r#"leader(".~.")"#);
        assert_eq!(
            items,
            vec![ContentComponent::Leader(LeaderType::String(SmolStr::new(
                ".~."
            )))]
        );
    }

    #[test]
    fn content_leader_is_case_insensitive() {
        let items = content_items("LEADER(DOTTED)");
        assert_eq!(items, vec![ContentComponent::Leader(LeaderType::Dotted)]);
    }

    #[test]
    fn content_leader_rejects_missing_argument() {
        // spec production `leader( <leader-type> )` に `?` が無いため引数必須。
        // bare `leader()` は spec-invalid → declaration drop。
        assert_eq!(parse("leader()", "content"), None);
    }

    #[test]
    fn content_leader_rejects_unknown_keyword() {
        assert_eq!(parse("leader(bogus)", "content"), None);
    }

    #[test]
    fn content_rejects_unknown_bare_keyword() {
        // `parse_content_bare_keyword` の 5 keyword (`contents` / 4 `<quote>`)
        // いずれにも一致しない ident は catch-all `_ => None` に落ちる —
        // items 0 → declaration drop (単独 token の場合)。
        assert_eq!(parse("bogus", "content"), None);
    }

    #[test]
    fn content_unknown_bare_keyword_mid_list_stops_items_and_leaves_leftover() {
        // 認識済み item (`counter(chapter)`) の後に未知 ident が来た場合、
        // items+ loop は unknown token で break する (catch-all の break 経路)。
        // caller (rule.rs) の `expect_exhausted` 相当は `parse` helper では
        // 経由しないため、本 helper 経由では 1-item 到達で観測できる — leftover
        // 自体の drop 挙動は既存 `content_rejects_unknown_function` /
        // `string_set_accepts_missing_comma_single_leftover_entry` と同じ
        // break-then-leftover pattern の non-regression pin。
        let items = content_items("counter(chapter) bogus");
        assert_eq!(
            items,
            vec![ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }]
        );
    }

    #[test]
    fn content_parse_mixed_sequence_with_new_alternatives() {
        // image / contents / quote / leader を既存 alternative と混在させ、
        // 順序が保持されることを検証。
        let items = content_items(
            r#"open-quote "term" close-quote leader(dotted) url("icon.png") contents"#,
        );
        assert_eq!(
            items,
            vec![
                ContentComponent::Quote(QuoteKeyword::OpenQuote),
                ContentComponent::Literal(SmolStr::new("term")),
                ContentComponent::Quote(QuoteKeyword::CloseQuote),
                ContentComponent::Leader(LeaderType::Dotted),
                ContentComponent::Image {
                    url: String::from("icon.png"),
                },
                ContentComponent::Contents,
            ]
        );
    }

    // ── string-set narrow <content-list> gate: image / contents / quote /
    // leader() (CSS GCPM 3 §1.1.1 L82) ──
    //
    // GCPM 3 §1.1.1 narrow list には `<image>` / `contents` / `<quote>` /
    // `leader()` のいずれも含まれない (既存の string_set_rejects_* group と
    // 同じ rationale — sibling test 群と揃えて 1 declaration = 1 rejection の
    // pin にする)。

    #[test]
    fn string_set_rejects_image_url() {
        assert_eq!(parse(r#"title url("a.png")"#, "string-set"), None);
    }

    #[test]
    fn string_set_rejects_contents_keyword() {
        assert_eq!(parse("title contents", "string-set"), None);
    }

    #[test]
    fn string_set_rejects_quote_keyword() {
        assert_eq!(parse("title open-quote", "string-set"), None);
    }

    #[test]
    fn string_set_rejects_leader_fn() {
        assert_eq!(parse("title leader(dotted)", "string-set"), None);
    }

    // ── content property edge cases (spec-derived、guard rails) ──

    #[test]
    fn content_normal_returns_empty_list() {
        // spec §1: `normal` は「content が明示されない場合と同じ」= 空 list として保持。
        // pseudo-element generation 判断は下流で行う。
        assert_eq!(
            parse("normal", "content"),
            Some(PropertyValue::Content(empty_content_list()))
        );
    }

    #[test]
    fn content_none_returns_empty_list() {
        // spec §1: `none` — 本 crate では `normal` と同じく空 list に落とす。
        assert_eq!(
            parse("none", "content"),
            Some(PropertyValue::Content(empty_content_list()))
        );
    }

    #[test]
    fn content_string_with_fetch_last_keyword() {
        // spec §2.7.2 の string() 第 2 引数 keyword を全て受理することを smoke で pin。
        let items = content_items("string(head, last)");
        assert_eq!(
            items,
            vec![ContentComponent::String {
                name: SmolStr::new("head"),
                fetch: StringFetchMode::Last,
            }]
        );
    }

    #[test]
    fn content_target_text_default_part_is_content() {
        // target-text() の第 2 引数省略時、raikiri は ContentPart::Content を
        // フォールバック値として使う (根拠は spec の "default" 宣言ではない —
        // CSS Content 3 §2.6.3 は第 2 引数省略時の値を規定していない)。
        let items = content_items(r##"target-text(url("#a"))"##);
        assert_eq!(
            items,
            vec![ContentComponent::TargetText {
                url: String::from("#a"),
                part: ContentPart::Content,
            }]
        );
    }

    #[test]
    fn content_counter_rejects_none_name() {
        // spec CSS Lists 3 §4 <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>:
        // "A <counter-name> name cannot match the keyword `none`; such an identifier
        // is invalid as a <counter-name>". §4.7 counter() の first argument が
        // <counter-name> production のため `counter(none)` は declaration drop。
        // counter-reset/increment/set (property.rs 既存) と一貫、Chrome/FF と一致。
        assert_eq!(parse("counter(none)", "content"), None);
    }

    #[test]
    fn content_counters_rejects_none_name() {
        // spec CSS Lists 3 §4 / §4.7: counters() の first argument も
        // <counter-name> production、`none` は invalid。
        assert_eq!(parse(r#"counters(none, ".")"#, "content"), None);
    }

    #[test]
    fn content_counter_with_named_style_preserves_ident() {
        // spec CSS Lists 3 §4.7: 第 2 引数 `<counter-style>` は decimal 以外の
        // named style も受ける。下流 (raikiri-dom) が解釈するため raw ident 保持。
        let items = content_items("counter(chapter, upper-alpha)");
        assert_eq!(
            items,
            vec![ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Named(SmolStr::new("upper-alpha")),
            }]
        );
    }

    #[test]
    fn content_rejects_unknown_function() {
        // 未知 function は認識できず、items 開始 token として peel 失敗。
        // 先頭 token が unknown function だと empty items → None (drop)。
        assert_eq!(parse("bogus(x)", "content"), None);
    }

    #[test]
    fn content_case_insensitive_function_name() {
        // spec: function name は ASCII case-insensitive。
        let items = content_items("COUNTER(chapter)");
        assert_eq!(
            items,
            vec![ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }]
        );
    }

    #[test]
    fn content_key_maps_to_content_property_key() {
        // PropertyValue::Content → PropertyKey::Content (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* と同じ pattern)。
        let cv = PropertyValue::Content(empty_content_list());
        assert_eq!(cv.key(), PropertyKey::Content);
    }

    // ── parse_optional_counter_style trailing-comma strict reject ──
    //
    // CSS Lists 3 §4.7 `counter(<counter-name>, <counter-style>?)` /
    // CSS Content 3 §2.6.1-2 `target-counter()` / `target-counters()` は
    // `<counter-style>?` — `,` を先行させる時は ident 必須。trailing-comma
    // (`counter(chapter,)` 等) は spec-invalid → declaration ごと drop すべき。
    // sibling `parse_string_fetch` / `parse_content_part` は既に strict `?`
    // propagation、`parse_optional_counter_style` のみ silent Decimal fallback
    // していた regression を pin する。

    #[test]
    fn content_counter_rejects_trailing_comma() {
        // `counter(chapter,)` — comma 消費後に ident 不在。spec-invalid、
        // declaration drop = None (Chrome/Firefox と同挙動)。
        assert_eq!(parse("counter(chapter,)", "content"), None);
    }

    #[test]
    fn content_counters_rejects_trailing_comma() {
        // `counters(chapter, ".",)` — separator string 後の trailing comma。
        assert_eq!(parse(r#"counters(chapter, ".",)"#, "content"), None);
    }

    #[test]
    fn content_target_counter_rejects_trailing_comma() {
        // `target-counter(url("#a"), page,)` — name 後の trailing comma。
        // target-counter/target-counters は parse_optional_counter_style を
        // 経由 (parse_target_counter_fn / parse_target_counters_fn) するため同じ pattern で drop。
        assert_eq!(
            parse(r##"target-counter(url("#a"), page,)"##, "content"),
            None
        );
    }

    #[test]
    fn content_target_counters_rejects_trailing_comma() {
        // `target-counters(url("#a"), section, ".",)` — separator 後の trailing。
        assert_eq!(
            parse(r##"target-counters(url("#a"), section, ".",)"##, "content"),
            None
        );
    }

    #[test]
    fn content_string_rejects_trailing_comma() {
        // 対照実験 (現行 strict の維持確認): `string(foo,)` は
        // `parse_string_fetch` が `?` 経由で伝播、既に None。
        assert_eq!(parse("string(foo,)", "content"), None);
    }

    #[test]
    fn content_target_text_rejects_trailing_comma() {
        // 対照実験: `target-text(url("#a"),)` は `parse_content_part` が
        // `?` 経由で伝播、既に None。
        assert_eq!(parse(r##"target-text(url("#a"),)"##, "content"), None);
    }

    #[test]
    fn content_counter_accepts_bare_default() {
        // `counter(chapter)` — trailing comma 無しの正常 case、Decimal default
        // で Some を返す (silent fallback を strict にしても正常 path は変えない)。
        let items = content_items("counter(chapter)");
        assert_eq!(
            items,
            vec![ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }]
        );
    }

    // ── string-set (CSS GCPM 3 §1.1.1) ──
    //
    // grammar: `none | [ <custom-ident> <content-list> ]#` — 各 entry は
    // (name, content-list) pair、`ContentComponent` + `parse_content_list_items`
    // を reuse。task description の "4-item Vec" は entry name の分を content 側に
    // 誤って含めた結果、実態は 3-item (name は tuple の第 1 要素)。

    fn string_set_entries(source: &str) -> Vec<(SmolStr, Vec<ContentComponent>)> {
        match parse(source, "string-set") {
            // PropertyValue::StringSet(Arc<Vec<..>>)、content_items と同 pattern。
            Some(PropertyValue::StringSet(v)) => (*v).clone(),
            other => panic!("expected PropertyValue::StringSet, got {other:?}"),
        }
    }

    #[test]
    fn string_set_single_entry_with_literal() {
        // Verification 1: string-set: my_str "hello"
        // → `[(SmolStr("my_str"), [Literal("hello")])]`
        let entries = string_set_entries(r#"my_str "hello""#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("my_str"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    #[test]
    fn string_set_mixed_content_list_preserves_order() {
        // Verification 2:
        // string-set: chapter_title counter(chapter) ": " attr(title)
        //
        // 先頭 `chapter_title` は entry name (tuple 第 1 要素)。content-list は
        // 残りの `counter(chapter) ": " attr(title)` = 3 items。
        //
        // NB: 原 test は末尾に `string(chapter_title)` を置いていたが、
        // GCPM 3 §1.1.1 narrow list は `string()` function を含まないため
        // `attr()` (GCPM narrow list の 5 alt の 1 つ) に
        // swap。テストの主意 (mixed content-list の order 保持) は保つ。
        let entries = string_set_entries(r#"chapter_title counter(chapter) ": " attr(title)"#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("chapter_title"));
        assert_eq!(entries[0].1.len(), 3);
        assert_eq!(
            entries[0].1[0],
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
        assert_eq!(
            entries[0].1[1],
            ContentComponent::Literal(SmolStr::new(": "))
        );
        assert_eq!(
            entries[0].1[2],
            ContentComponent::Attr {
                name: SmolStr::new("title"),
            }
        );
    }

    #[test]
    fn string_set_comma_separated_multi_entry() {
        // Verification 3: string-set: a "x", b "y" → 2 entries
        let entries = string_set_entries(r#"a "x", b "y""#);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, SmolStr::new("a"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Literal(SmolStr::new("x"))]
        );
        assert_eq!(entries[1].0, SmolStr::new("b"));
        assert_eq!(
            entries[1].1,
            vec![ContentComponent::Literal(SmolStr::new("y"))]
        );
    }

    #[test]
    fn string_set_none_returns_empty_vec() {
        // spec §1.1.1: top-level `none` = empty list
        assert_eq!(
            parse("none", "string-set"),
            Some(PropertyValue::StringSet(empty_string_set_entries()))
        );
    }

    #[test]
    fn string_set_rejects_reserved_css_wide_keyword_as_name() {
        // spec §1.1.1 + CSS Values 4 §4.2
        // <https://www.w3.org/TR/css-values-4/#custom-idents>:
        // `<custom-ident>` は CSS-wide keyword 除外。
        // 先頭 ident が `inherit` → try_parse rewind で entries 空 → None。
        //
        // NB: 先頭が `none` の場合は top-level alternative の branch を先に
        // 通って `Some(empty)` を返し、leftover は下流 `expect_exhausted` で
        // declaration drop (rule.rs level)。この case は parse_value 単体では
        // 検証しない。
        assert_eq!(parse("inherit \"x\"", "string-set"), None);
        assert_eq!(parse("initial \"x\"", "string-set"), None);
        assert_eq!(parse("unset \"x\"", "string-set"), None);
        assert_eq!(parse("revert \"x\"", "string-set"), None);
        assert_eq!(parse("default \"x\"", "string-set"), None);
    }

    #[test]
    fn string_set_rejects_name_without_content_list() {
        // spec §1.1.1 + Content 3 §2: `<content-list>` は 1+ items 必須。
        // name だけで items 0 → declaration drop (None)。
        assert_eq!(parse("my_str", "string-set"), None);
    }

    #[test]
    fn string_set_is_case_insensitive_on_none() {
        // CSS spec: keyword `none` は ASCII case-insensitive
        assert_eq!(
            parse("NONE", "string-set"),
            Some(PropertyValue::StringSet(empty_string_set_entries()))
        );
    }

    #[test]
    fn string_set_key_maps_to_string_set_property_key() {
        // PropertyValue::StringSet → PropertyKey::StringSet (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* / content と同じ pattern)。
        let v = PropertyValue::StringSet(empty_string_set_entries());
        assert_eq!(v.key(), PropertyKey::StringSet);
    }

    // ── string-set trailing-comma strict reject ──
    //
    // `#` (comma-separated multiplier、CSS Values 4 §2.3
    // <https://www.w3.org/TR/css-values-4/#mult-comma>) は trailing comma を
    // 許容しない。GCPM 3 §1.1.1 <string-set-value> = `[ <custom-ident>
    // <content-list> ]#` は entry 間 comma 必須 + trailing comma 禁止。
    //
    // 初期実装は separator loop で `try_parse(expect_comma).is_err() {
    // break }` していたため、trailing comma を silently 受理していた (comma を
    // consume 後 next iteration で name parse fail → break → 既存 entries を
    // Some で返す)。同じ principle の `.ok()?` propagation で strict 化。

    #[test]
    fn string_set_rejects_trailing_comma_single_entry() {
        // `string-set: a "x",` → trailing comma → declaration drop。
        // pre-fix は Some(`[(a, [Literal("x")])]`) を silently 返していた。
        assert_eq!(parse(r#"a "x","#, "string-set"), None);
    }

    #[test]
    fn string_set_rejects_trailing_comma_two_entries() {
        // `string-set: a "x", b "y",` → trailing comma → declaration drop。
        // 内部 comma 1 個は valid separator、末尾 comma のみが `#` 違反。
        assert_eq!(parse(r#"a "x", b "y","#, "string-set"), None);
    }

    #[test]
    fn string_set_rejects_trailing_comma_three_entries() {
        // 3 entries + trailing comma — chain 越しの一貫 strict reject を pin。
        assert_eq!(parse(r#"a "x", b "y", c "z","#, "string-set"), None);
    }

    #[test]
    fn string_set_rejects_missing_entry_after_comma() {
        // `string-set: a "x", b` → comma 後 name は取れるが `<content-list>`
        // が 0 items (`parse_content_list_items` empty) → declaration drop。
        // trailing-comma 系とは reject 経路が異なる (items-empty) 独立 pin。
        assert_eq!(parse(r#"a "x", b"#, "string-set"), None);
    }

    #[test]
    fn string_set_accepts_missing_comma_single_leftover_entry() {
        // `string-set: a "x" b "y"` は separator comma 欠如。iter 1 で
        // (a, `["x"]`) push 後、bottom expect_comma fail → break、leftover
        // `b "y"` は本 helper (parse_value 直呼び、caller expect_exhausted
        // 経由なし) では drop されず 1 entry の Some として観測される。
        // 実 caller (rule.rs) は expect_exhausted で declaration drop する
        // — 本 test は parse_string_set の break exit が Some (`.ok()?`
        // 経路と混同しない) であることを pin する目的、trailing-comma fix の
        // non-regression coverage。
        let entries = string_set_entries(r#"a "x" b "y""#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("a"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Literal(SmolStr::new("x"))]
        );
    }

    // ── string-set narrow <content-list> gate (CSS GCPM 3 §1.1.1) ──
    //
    // GCPM 3 §1.1.1 L82 verbatim: <content-list> = [ <string> | <counter()> |
    // <counters()> | <content()> | <attr()> ]+ — CSS Content 3 §2 broad list を
    // string-set 用に narrower 再定義。`string()` (function、bare literal とは別)
    // および `target-counter()` / `target-counters()` / `target-text()` は
    // spec grammar に含まれず、`ContentListMode::GcpmStringSet` mode dispatch で
    // reject する (parse_content_list_items が 0 items → parse_string_set →
    // None → declaration drop、cascade shadow 例
    // `p.hi { string-set: title target-counter(url("#x"), page); }` の spec 準拠
    // 挙動 = .hi rule drop → parser layer で確認)。
    //
    // 一方 content property (CssContent3 mode) はこれら全てを引き続き受理する
    // (下の content_parse_* 系 pin test 群で non-regression 検証)。

    #[test]
    fn string_set_rejects_string_fn() {
        // GCPM 3 §1.1.1 L82 は `string()` function を narrow list から除外。
        // bare `<string>` literal (`"..."`) と混同しないよう function 側のみ reject。
        assert_eq!(parse("title string(x)", "string-set"), None);
    }

    #[test]
    fn string_set_rejects_target_counter_fn() {
        // GCPM 3 §1.1.1 L82 は `target-counter()` を narrow list から除外。
        // cascade shadow の主要例、declaration drop → cascade で
        // 先行の spec-valid rule が winner になる shape。
        assert_eq!(
            parse(r##"title target-counter(url("#a"), page)"##, "string-set"),
            None
        );
    }

    #[test]
    fn string_set_rejects_target_counters_fn() {
        // GCPM 3 §1.1.1 L82 は `target-counters()` を narrow list から除外。
        assert_eq!(
            parse(
                r##"title target-counters(url("#a"), section, ".")"##,
                "string-set"
            ),
            None
        );
    }

    #[test]
    fn string_set_rejects_target_text_fn() {
        // GCPM 3 §1.1.1 L82 は `target-text()` を narrow list から除外。
        assert_eq!(
            parse(r##"title target-text(url("#a"))"##, "string-set"),
            None
        );
    }

    // ── content() function (CSS GCPM 3 §1.1.1.1) ──
    //
    // grammar (spec verbatim, line 758 of TR/css-gcpm-3/, string-set/GCPM3側の
    // grammar):
    //   content() = content(`[text | before | after | first-letter]`)
    // 4 keyword。keyword 省略時は `text` をフォールバック値として使う (根拠は
    // GCPM 3 側の spec "default" 宣言ではない — grammar に `?` が無く、"default
    // をどう定義するか" 自体が未解決の WG issue として残っている)。GCPM 3
    // §1.1.1 の narrow `<content-list>` と CSS Content 3 §2 の broad
    // `<content-list>` の両方に対し unconditional に受理されるため、
    // string-set および content property 双方の content-list 内で受理される
    // (`ContentListMode` mode dispatch 導入後も `content()` arm は両
    // mode で unconditional accept)。
    //
    // CSS Content 3 §2.7.3 は content() を `?` 付き 5 keyword (`marker` 含む)
    // で別途定義しており、GCPM 3 §1.1.1.1 と keyword 集合が食い違う。この
    // 実装は keyword 集合について GCPM 3 §1.1.1.1 に従うと決めており、content
    // property 側でも `marker` は意図的に reject する (詳細・根拠は
    // `parse_content_fn` の doc comment 参照)。CSS Content 3 §2.7.3 の
    // `marker` keyword は既知の feature gap として残る。
    //
    // pre-fix reproduction: `string-set: title content(text)` は
    // silent drop していた (parse_content_function match arm 欠如 →
    // parse_content_list_items break → 0 items → parse_string_set None →
    // declaration drop)。arm 追加で Some を返すことを pin する。

    #[test]
    fn string_set_content_text_reproduces_pre_fix_drop() {
        // description の主要 repro case:
        // pre-fix では declaration drop = None、post-fix では
        // (title, `[Content{keyword: Text}]`) を含む Some を返す。
        let entries = string_set_entries("title content(text)");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("title"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_explicit_text_keyword() {
        // §1.1.1.1: `content(text)` は element の string value (bare `content()`
        // のフォールバック値と同じ keyword だが、明示的 keyword 保持で
        // downstream の分岐余地を残す)。
        let items = content_items("content(text)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_default_keyword_on_empty_parens() {
        // §1.1.1.1 の spec 例 `h2 { string-set: heading content() }` (string-set
        // /GCPM3側の文脈) — bare `content()` は `text` をフォールバック値として
        // 使う (根拠は GCPM 3 側の spec "default" 宣言ではない。
        // content property側でのgrammar相反は上記 parse_content_fn doc 参照)。
        let items = content_items("content()");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_before_keyword() {
        // §1.1.1.1 の spec 例 `h1 { string-set: header content(before) ':' content(text); }`
        // で使われる `before` keyword。
        let items = content_items("content(before)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            }]
        );
    }

    #[test]
    fn content_content_fn_after_keyword() {
        let items = content_items("content(after)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::After,
            }]
        );
    }

    #[test]
    fn content_content_fn_first_letter_keyword() {
        let items = content_items("content(first-letter)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::FirstLetter,
            }]
        );
    }

    #[test]
    fn content_content_fn_rejects_unknown_keyword() {
        // GCPM 3 §1.1.1.1 の grammar は `[text | before | after | first-letter]`
        // の 4 alternative のみ (string-set/GCPM3側の文脈)。それ以外の ident は
        // parse_content_text_keyword が None を返し、上位伝播で
        // parse_content_list_items が break、declaration drop = None。`marker`
        // はこの GCPM3 grammar には無い。CSS Content 3 §2.7.3 は独自に
        // content() を `marker` 含む 5 keyword で定義しているが、この実装は
        // keyword 集合について GCPM 3 §1.1.1.1 に従うと決めており
        // (parse_content_fn の doc comment 参照)、`marker` reject は意図した
        // 挙動であって未解決の問題ではない。
        // 本 test は現状の GCPM3-scoped 実装の挙動を
        // pin するものであり、`marker` が spec に一切存在しないという主張では
        // ない。
        assert_eq!(parse("content(marker)", "content"), None);
        assert_eq!(parse("content(bogus)", "content"), None);
    }

    #[test]
    fn content_content_fn_rejects_target_text_keyword() {
        // §1.1.1.1 は `text` alternative を持つ (target-text() §2.6.3 は `content`)。
        // spec spelling divergence — `content(content)` は spec-invalid、reject。
        // 混同 (sibling ContentPart 再利用) を防ぐ regression pin。
        assert_eq!(parse("content(content)", "content"), None);
    }

    #[test]
    fn content_content_fn_case_insensitive_keyword() {
        // spec 慣行: keyword は ASCII case-insensitive。
        let items = content_items("content(TEXT)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_case_insensitive_function_name() {
        // parse_content_function は既存 arm と同じく ASCII case-insensitive dispatch。
        let items = content_items("CONTENT(before)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            }]
        );
    }

    #[test]
    fn content_content_fn_rejects_extra_argument() {
        // grammar は single-argument。余剰 token は
        // parse_nested_block 内 parse_entirely が拒否し declaration drop。
        assert_eq!(parse("content(text, extra)", "content"), None);
        assert_eq!(parse("content(text before)", "content"), None);
    }

    #[test]
    fn string_set_content_fn_mixed_with_other_items() {
        // §1.1.1.1 の spec 例:
        //   h1 { string-set: header content(before) ':' content(text); }
        // → (header, `[Content{Before}, Literal(":"), Content{Text}]`) 3 items。
        let entries = string_set_entries(r#"header content(before) ":" content(text)"#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("header"));
        assert_eq!(
            entries[0].1,
            vec![
                ContentComponent::Content {
                    keyword: ContentTextKeyword::Before,
                },
                ContentComponent::Literal(SmolStr::new(":")),
                ContentComponent::Content {
                    keyword: ContentTextKeyword::Text,
                },
            ]
        );
    }

    // ── position: running() (CSS GCPM 3 §1.2.1) ──
    //
    // Verification items 1-6 は task description 由来、
    // canonical shape は後に amended。sibling は counter-* /
    // content / string-set の SmolStr wire-through pattern。

    #[test]
    fn position_parse_running_header() {
        // Verification 1: position: running(header)
        // → PropertyValue::Position(PositionValue::Running("header"))
        assert_eq!(
            parse("running(header)", "position"),
            Some(PropertyValue::Position(PositionValue::Running(
                SmolStr::new("header")
            )))
        );
    }

    #[test]
    fn position_parse_running_footer() {
        // Verification 2: 別 name の smoke — SmolStr::new が生きていることを pin。
        assert_eq!(
            parse("running(footer)", "position"),
            Some(PropertyValue::Position(PositionValue::Running(
                SmolStr::new("footer")
            )))
        );
    }

    #[test]
    fn position_parse_static() {
        // Verification 5 baseline: position: static → PositionValue::Static。
        // apply_value は no-op、running_templates は inherit_from の initial
        // (空 Vec) が残る = cascade winner が earlier running(...) を suppress する
        // ID 用途 (cascade.rs 側の `static_position_wins_over_running` で検証)。
        assert_eq!(
            parse("static", "position"),
            Some(PropertyValue::Position(PositionValue::Static))
        );
    }

    #[test]
    fn position_running_case_insensitive_function_name() {
        // Verification 4: function name は ASCII case-insensitive (CSS spec 慣行)、
        // custom-ident は case-preserving。
        assert_eq!(
            parse("RUNNING(header)", "position"),
            Some(PropertyValue::Position(PositionValue::Running(
                SmolStr::new("header")
            )))
        );
    }

    #[test]
    fn position_running_rejects_none_custom_ident() {
        // Verification 6: `running(none)` reject。`none` は position property
        // spec-defined keyword ではないが、runtime resolve で `element(none)` 参照が
        // silent match するのを避けるため custom-ident としても弾く (string-set
        // と同じ規約)。
        assert_eq!(parse("running(none)", "position"), None);
    }

    #[test]
    fn position_running_rejects_reserved_css_wide_keyword() {
        // spec CSS Values 4 §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>:
        // <custom-ident> は CSS-wide keyword + `default` 除外。
        // position: running(inherit) 等は declaration drop。
        assert_eq!(parse("running(inherit)", "position"), None);
        assert_eq!(parse("running(initial)", "position"), None);
        assert_eq!(parse("running(unset)", "position"), None);
        assert_eq!(parse("running(revert)", "position"), None);
        assert_eq!(parse("running(default)", "position"), None);
    }

    #[test]
    fn position_rejects_missing_custom_ident() {
        // spec §1.2.1: `running() = running( <custom-ident> )` — argument 必須。
        // 空 argument は malformed、declaration drop。
        assert_eq!(parse("running()", "position"), None);
    }

    #[test]
    fn position_rejects_out_of_scope_keywords() {
        // relative / absolute / fixed / sticky は本 crate では
        // 認識せず None を返す (spec-correct: invalid → drop)。
        assert_eq!(parse("relative", "position"), None);
        assert_eq!(parse("absolute", "position"), None);
        assert_eq!(parse("fixed", "position"), None);
        assert_eq!(parse("sticky", "position"), None);
    }

    #[test]
    fn position_rejects_running_with_extra_arg() {
        // `running(a, b)` — parse_nested_block が parse_entirely 経由で
        // 余剰 token を検知し、declaration drop になる。
        assert_eq!(parse("running(a, b)", "position"), None);
    }

    // ── parse_length_value helper ────────────────────
    //
    // helper 単体を叩く共通 fixture — property dispatcher (`parse_value`) を経由せず
    // 5 unit sample (`px` / `em` / `rem` / `%` / `pt`) の parse を直接 verify する。
    // `parse_font_size` 経由 test は上流に既存 (`font_size_parse_px` 等)、そちらは
    // px-only post-filter を verify するので分離する。

    fn parse_length(source: &str, allow_percentage: bool) -> Option<Length> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_length_value(&mut parser, allow_percentage)
    }

    #[test]
    fn parse_length_value_accepts_px() {
        assert_eq!(parse_length("10px", false), Some(Length::Px(10.0)));
        // length-percentage mode でも px 受理 (mode 非依存)。
        assert_eq!(parse_length("10px", true), Some(Length::Px(10.0)));
    }

    #[test]
    fn parse_length_value_accepts_em() {
        // CSS Values 4 §6.1.1 em (https://www.w3.org/TR/css-values-4/#em):
        // authored `1.2em` を Length::Em(1.2) にそのまま保持 (resolve は下流責務)。
        assert_eq!(parse_length("1.2em", false), Some(Length::Em(1.2)));
    }

    #[test]
    fn parse_length_value_accepts_rem() {
        // CSS Values 4 §6.1.1 rem (https://www.w3.org/TR/css-values-4/#rem):
        // root element の font-size 基準、authored value を Length::Rem に格納。
        assert_eq!(parse_length("1rem", false), Some(Length::Rem(1.0)));
    }

    #[test]
    fn parse_length_value_accepts_pt() {
        // CSS Values 4 §6.2 absolute lengths (https://www.w3.org/TR/css-values-4/#absolute-lengths):
        // 1pt = 1/72 in, 1in = 96px、resolve 側で 12pt → 16px 相当に変換。
        assert_eq!(parse_length("12pt", false), Some(Length::Pt(12.0)));
    }

    #[test]
    fn parse_length_value_accepts_percentage_when_allowed() {
        // CSS Values 4 §5.5 (https://www.w3.org/TR/css-values-4/#percentages):
        // `<length-percentage>` mode でのみ受理。cssparser `unit_value = 0.5` を
        // × 100.0 で authored `50` に戻して Length::Percent(50.0) に格納。
        assert_eq!(parse_length("50%", true), Some(Length::Percent(50.0)));
    }

    #[test]
    fn parse_length_value_extreme_percentage_saturates_to_f32_max_not_inf() {
        // `1e40%` は cssparser tokenizer 側で
        // `unit_value = 1e40 / 100.0 = 1e38` (f32 有限範囲 `3.4028235e38` 内)
        // になるが、authored number へ戻す本 helper の `× 100.0` 自体が
        // f32 overflow を起こし +Inf を作っていた (fix 前)。
        //
        // CSS Values 4 §5 "Range Checking and Precision for Numeric Types"
        // <https://www.w3.org/TR/css-values-4/#numeric-types>:
        // "When a value cannot be explicitly supported due to
        // range/precision limitations, it must be converted to the closest
        // value supported by the implementation" — 非有限は許容されないため、
        // 符号を保持しつつ f32::MAX に寄った有限値を pin する。
        assert_eq!(parse_length("1e40%", true), Some(Length::Percent(f32::MAX)));
        // 符号保持も合わせて pin (負の overflow は -f32::MAX へ)。
        assert_eq!(
            parse_length("-1e40%", true),
            Some(Length::Percent(-f32::MAX))
        );
    }

    #[test]
    fn parse_length_value_nan_percentage_passes_through_unsaturated() {
        // finding:
        // `is_finite()` also catches NaN, a different failure class than the
        // `1e40%` overflow above. `0e999%` triggers it: cssparser's exponent
        // handling computes `0.0 * 10f64.powf(999.0)`, and
        // `10f64.powf(999.0)` is `+Inf`, so the product is `NaN` per IEEE
        // 754 — even though `0e999`'s true mathematical value is `0`, not
        // "unrepresentable". Saturating this to `f32::MAX` would turn the
        // sink-side geometry (`raikiri-dom::layout::sanitize_finite`, which
        // already treats `NaN` as `0.0`) into a huge box instead of a
        // zero-sized one, so this class must pass through unsaturated and
        // rely on that existing `NaN -> 0.0` sink contract downstream.
        // cov:ignore: the panic-message literals in this match's arms only
        // execute on assertion/match failure, unreachable while this test
        // passes.
        match parse_length("0e999%", true) {
            Some(Length::Percent(v)) => assert!(
                v.is_nan(),
                "expected NaN (0 * Inf) to pass through unsaturated, got {v}"
            ),
            other => panic!("expected Some(Length::Percent(NaN)), got {other:?}"),
        }
    }

    #[test]
    fn parse_length_value_rejects_percentage_in_length_only_mode() {
        // `<length>` mode (font-size 等) では `%` は grammar 外、None を返す。
        assert_eq!(parse_length("50%", false), None);
    }

    #[test]
    fn parse_length_value_rejects_unsupported_unit() {
        // (b) 非対応 — viewport-relative unit / `cap` / `rcap` は本 helper で
        // 引き続き silent drop。`lh` / `rlh` は受理側へ移った
        // (下記 `parse_length_value_accepts_lh` / `_rlh` を参照)。
        assert_eq!(parse_length("10vw", false), None);
        assert_eq!(parse_length("1cap", true), None);
        // container-query unit (CSS Contain 3 §6) — `_` arm 直前 comment が
        // 挙げる `cq*` 一覧をこの assertion で pin する。comment のみで
        // test 未網羅だと、将来 `cq*` 対応 arm が誤って追加されても
        // どの test も落ちず canonical comment が silent に stale 化する
        // (spec-lens follow-up として追加)。
        assert_eq!(parse_length("10cqw", false), None);
    }

    #[test]
    fn parse_length_value_accepts_lh() {
        // https://www.w3.org/TR/css-values-4/#lh — authored value をそのまま保持。
        assert_eq!(parse_length("1.5lh", false), Some(Length::Lh(1.5)));
    }

    #[test]
    fn parse_length_value_accepts_rlh() {
        // https://www.w3.org/TR/css-values-4/#rlh
        assert_eq!(parse_length("2rlh", false), Some(Length::Rlh(2.0)));
    }

    // ── 追加 font-relative unit (CSS Values 4 §6.1.1) ──

    #[test]
    fn parse_length_value_accepts_ex() {
        // https://www.w3.org/TR/css-values-4/#ex — authored value をそのまま保持。
        assert_eq!(parse_length("2ex", false), Some(Length::Ex(2.0)));
    }

    #[test]
    fn parse_length_value_accepts_rex() {
        // https://www.w3.org/TR/css-values-4/#rex
        assert_eq!(parse_length("2rex", false), Some(Length::Rex(2.0)));
    }

    #[test]
    fn parse_length_value_accepts_ch() {
        // https://www.w3.org/TR/css-values-4/#ch
        assert_eq!(parse_length("3ch", false), Some(Length::Ch(3.0)));
    }

    #[test]
    fn parse_length_value_accepts_rch() {
        // https://www.w3.org/TR/css-values-4/#rch
        assert_eq!(parse_length("3rch", false), Some(Length::Rch(3.0)));
    }

    #[test]
    fn parse_length_value_accepts_ic() {
        // https://www.w3.org/TR/css-values-4/#ic
        assert_eq!(parse_length("1.5ic", false), Some(Length::Ic(1.5)));
    }

    #[test]
    fn parse_length_value_accepts_ric() {
        // https://www.w3.org/TR/css-values-4/#ric
        assert_eq!(parse_length("1.5ric", false), Some(Length::Ric(1.5)));
    }

    // ── 追加 absolute unit (CSS Values 4 §6.2) ──

    #[test]
    fn parse_length_value_accepts_cm() {
        assert_eq!(parse_length("2cm", false), Some(Length::Cm(2.0)));
    }

    #[test]
    fn parse_length_value_accepts_mm() {
        assert_eq!(parse_length("5mm", false), Some(Length::Mm(5.0)));
    }

    #[test]
    fn parse_length_value_accepts_q() {
        // `Q` — unit token は `to_ascii_lowercase()` を経て `"q"` として dispatch
        // される。case-insensitivity test でも uppercase `Q` を確認する。
        assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
    }

    #[test]
    fn parse_length_value_accepts_in() {
        assert_eq!(parse_length("1in", false), Some(Length::In(1.0)));
    }

    #[test]
    fn parse_length_value_accepts_pc() {
        assert_eq!(parse_length("6pc", false), Some(Length::Pc(6.0)));
    }

    #[test]
    fn parse_length_value_accepts_unitless_zero_only() {
        // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>:
        // "For zero lengths the unit identifier is optional (i.e. can be
        // syntactically represented as the `<number>` 0)." — bare `0` は
        // mode 非依存で Length::Px(0.0) 受理 (両 mode 網羅で mode-independence pin)。
        assert_eq!(parse_length("0", false), Some(Length::Px(0.0)));
        assert_eq!(parse_length("0", true), Some(Length::Px(0.0)));
        // 非零 unitless number は grammar 上 length ではない — `== 0.0` guard で
        // 分岐して下段 `_ => None` fallthrough で drop。drop 経路は mode 非依存
        // (guard を通らず fallthrough する path が両 mode 共通) のため 1 mode で pin。
        assert_eq!(parse_length("5", false), None);
        assert_eq!(parse_length("-1", false), None);
    }

    #[test]
    fn parse_length_value_rejects_non_numeric_token() {
        assert_eq!(parse_length("medium", false), None);
        assert_eq!(parse_length("", false), None);
    }

    #[test]
    fn parse_length_value_preserves_negative_sign() {
        // helper は sign check を行わない — property ごとに要件が異なるため
        // (font-size は non-negative post-filter、margin は negative 許容)。
        assert_eq!(parse_length("-5px", false), Some(Length::Px(-5.0)));
        assert_eq!(parse_length("-1em", false), Some(Length::Em(-1.0)));
    }

    #[test]
    fn parse_length_value_unit_dispatch_case_insensitive() {
        // CSS spec: unit identifier は ASCII case-insensitive。
        assert_eq!(parse_length("10PX", false), Some(Length::Px(10.0)));
        assert_eq!(parse_length("1.5EM", false), Some(Length::Em(1.5)));
        assert_eq!(parse_length("2Rem", false), Some(Length::Rem(2.0)));
        assert_eq!(parse_length("14Pt", false), Some(Length::Pt(14.0)));
        // `unit.to_ascii_lowercase()` の dispatch key はすべて lowercase
        // (`"q"` / `"in"` 等) — uppercase 単位が正しく畳み込まれることを
        // 個別に確認する (`Q` は特に取り違えやすい)。
        assert_eq!(parse_length("10IN", false), Some(Length::In(10.0)));
        assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
        assert_eq!(parse_length("2CM", false), Some(Length::Cm(2.0)));
        assert_eq!(parse_length("2EX", false), Some(Length::Ex(2.0)));
        assert_eq!(parse_length("2CH", false), Some(Length::Ch(2.0)));
        assert_eq!(parse_length("2IC", false), Some(Length::Ic(2.0)));
    }

    // ── padding (CSS Box 3 §4.1 physical + §4.2 shorthand) ──
    //
    // Primary sources:
    // - https://www.w3.org/TR/css-box-3/#padding-physical
    //   "Negative values for padding properties are invalid." — non-negative
    //   constraint を parse-time enforce (parse_padding_side が全 Length variant
    //   で >= 0.0 check、負値 = declaration drop)。
    // - https://www.w3.org/TR/css-box-3/#padding-shorthand
    //   `<'padding-top'>{1,4}` — 1-4 value expansion (top/right/bottom/left)。

    fn padding_sides(top: Length, right: Length, bottom: Length, left: Length) -> Sides<Length> {
        Sides {
            top,
            right,
            bottom,
            left,
        }
    }

    // Verification #3 — longhand parse 4 arm (px / % / em / pt の 5 unit)。
    #[test]
    fn padding_top_parses_px() {
        assert_eq!(
            parse("10px", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Px(10.0)))
        );
    }

    #[test]
    fn padding_right_parses_percentage() {
        // spec grammar `<length-percentage>` — % 受理。
        assert_eq!(
            parse("5%", "padding-right"),
            Some(PropertyValue::PaddingRight(Length::Percent(5.0)))
        );
    }

    #[test]
    fn padding_bottom_parses_em() {
        assert_eq!(
            parse("1em", "padding-bottom"),
            Some(PropertyValue::PaddingBottom(Length::Em(1.0)))
        );
    }

    #[test]
    fn padding_left_parses_pt() {
        assert_eq!(
            parse("12pt", "padding-left"),
            Some(PropertyValue::PaddingLeft(Length::Pt(12.0)))
        );
    }

    // Verification #4 — shorthand 1-4 value expansion (CSS Box 3 §4.2)。
    #[test]
    fn padding_shorthand_one_value_all_sides() {
        // 1 value → 4 sides = value
        let px10 = Length::Px(10.0);
        assert_eq!(
            parse("10px", "padding"),
            Some(PropertyValue::Padding(Sides::all(px10)))
        );
    }

    #[test]
    fn padding_shorthand_two_values_top_bottom_left_right() {
        // 2 values → top/bottom = 1st, left/right = 2nd
        let px10 = Length::Px(10.0);
        let px20 = Length::Px(20.0);
        assert_eq!(
            parse("10px 20px", "padding"),
            Some(PropertyValue::Padding(padding_sides(
                px10, px20, px10, px20
            )))
        );
    }

    #[test]
    fn padding_shorthand_three_values_top_horizontal_bottom() {
        // 3 values → top = 1st, left/right = 2nd, bottom = 3rd
        let px10 = Length::Px(10.0);
        let px20 = Length::Px(20.0);
        let px30 = Length::Px(30.0);
        assert_eq!(
            parse("10px 20px 30px", "padding"),
            Some(PropertyValue::Padding(padding_sides(
                px10, px20, px30, px20
            )))
        );
    }

    #[test]
    fn padding_shorthand_four_values_clockwise() {
        // 4 values → top / right / bottom / left (clockwise from top)
        assert_eq!(
            parse("10px 20px 30px 40px", "padding"),
            Some(PropertyValue::Padding(padding_sides(
                Length::Px(10.0),
                Length::Px(20.0),
                Length::Px(30.0),
                Length::Px(40.0),
            )))
        );
    }

    #[test]
    fn padding_shorthand_mixed_units() {
        // spec (CSS Box 3) §4.2 は per-value `<'padding-top'>` = `<length-percentage>` を許容 —
        // 混合 unit も spec-valid (padding: 10px 5% 1em 12pt)。
        assert_eq!(
            parse("10px 5% 1em 12pt", "padding"),
            Some(PropertyValue::Padding(padding_sides(
                Length::Px(10.0),
                Length::Percent(5.0),
                Length::Em(1.0),
                Length::Pt(12.0),
            )))
        );
    }

    // Verification #5 — non-negative constraint (spec-literal claim)。
    #[test]
    fn padding_top_rejects_negative_px() {
        // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid."。
        assert_eq!(parse("-5px", "padding-top"), None);
    }

    #[test]
    fn padding_top_rejects_negative_percentage() {
        // 負 percentage も同様に spec-invalid。
        assert_eq!(parse("-10%", "padding-top"), None);
    }

    #[test]
    fn padding_top_rejects_negative_em() {
        // 負 em (font-relative) も spec-invalid。
        assert_eq!(parse("-1em", "padding-top"), None);
    }

    #[test]
    fn padding_top_rejects_negative_rem() {
        // 全 Length variant 経路の non-negative check pin (rem)。
        assert_eq!(parse("-0.5rem", "padding-top"), None);
    }

    #[test]
    fn padding_top_rejects_negative_pt() {
        // 全 Length variant 経路の non-negative check pin (pt)。
        assert_eq!(parse("-3pt", "padding-top"), None);
    }

    #[test]
    fn padding_top_accepts_zero() {
        // zero (bound の下端) は spec grammar `[0,∞]` の閉区間で有効。
        // `0px` は Dimension arm、bare `0` は CSS Values 3 §5 unitless-zero clause
        // の Number arm を通し、非負 filter を pass。
        assert_eq!(
            parse("0px", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Px(0.0)))
        );
        assert_eq!(
            parse("0", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Px(0.0)))
        );
    }

    #[test]
    fn padding_shorthand_rejects_any_negative_value() {
        // `padding: 10px -5px` — spec (CSS Box 3) §4.2 の {1,4} multiplier は各 iteration が
        // 有効 `<'padding-top'>` であることを要求。2 番目 `-5px` は spec (CSS Box 3) §4.1
        // `[0,∞]` 制約違反で fail、try_parse rewind で 1-value form の Some を
        // parse_padding_shorthand が返す。ここで DeclParser の expect_exhausted
        // が leftover `-5px` を検知して declaration ごと drop する — 実 caller
        // 経路として rule.rs 経由で drop を pin (parse_value 単体では
        // Some(all(10px)) が観測されるが、それは leftover 込みで invalid)。
        let decls_2 = crate::rule::parse_declaration_block(&mut Parser::new(
            &mut ParserInput::new("padding: 10px -5px;"),
        ));
        assert!(
            decls_2.is_empty(),
            "`padding: 10px -5px` must drop via expect_exhausted leftover"
        );
        // 4 value form 内の 4 番目が負値 case — 同様 leftover 経由 drop。
        let decls_4 = crate::rule::parse_declaration_block(&mut Parser::new(
            &mut ParserInput::new("padding: 10px 20px 30px -40px;"),
        ));
        assert!(
            decls_4.is_empty(),
            "`padding: 10px 20px 30px -40px` must drop via expect_exhausted leftover"
        );
    }

    // Verification #6 — `auto` keyword reject (spec grammar に無い)。
    #[test]
    fn padding_top_rejects_auto_keyword() {
        // spec (CSS Box 3) §4.1 grammar = `<length-percentage>` のみ、`auto` は margin 側の
        // extension で padding には無い。parse_length_value の Dimension /
        // Percentage arm fall-through で自然 reject。
        assert_eq!(parse("auto", "padding-top"), None);
    }

    #[test]
    fn padding_shorthand_rejects_auto_keyword() {
        // shorthand も同様 auto reject (1st value で fail、全体 drop)。
        assert_eq!(parse("auto", "padding"), None);
    }

    #[test]
    fn padding_shorthand_mixed_with_auto_drops_via_leftover() {
        // `padding: 10px auto` — 1st 成功 (10px)、2nd で auto → try_parse rewind、
        // 1-value form の Some を parse_padding_shorthand が返す。ここまでは
        // parse_value 単体で観測可能だが、DeclParser の expect_exhausted が
        // leftover `auto` を検知して declaration drop する — 実 caller 経路の
        // pin として rule.rs 経由でも drop することを確認。
        let decls = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
            "padding: 10px auto;",
        )));
        assert!(
            decls.is_empty(),
            "`padding: 10px auto` must drop via expect_exhausted leftover"
        );
    }

    // Verification #6 — spec grammar 外 unit の drop (vw / cap 等、非対応)。
    #[test]
    fn padding_top_rejects_unsupported_unit() {
        // (b) 非対応 — vw / cap 等は spec-valid だが
        // 未対応、parse_length_value 側で drop、`None`
        // propagate → declaration drop。`ch` / `lh` / `rlh` はそれぞれ受理側へ移った
        // (`padding_top_accepts_ch` / `padding_top_accepts_lh` 参照)。
        assert_eq!(parse("10vw", "padding-top"), None);
        assert_eq!(parse("5cap", "padding-top"), None);
    }

    #[test]
    fn padding_top_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh`。
        assert_eq!(
            parse("5lh", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Lh(5.0)))
        );
        assert_eq!(
            parse("1rlh", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Rlh(1.0)))
        );
    }

    #[test]
    fn padding_top_accepts_ch() {
        assert_eq!(
            parse("2ch", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Ch(2.0)))
        );
    }

    #[test]
    fn padding_top_accepts_cm() {
        assert_eq!(
            parse("2cm", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Cm(2.0)))
        );
    }

    #[test]
    fn padding_top_rejects_negative_cm() {
        // 全 Length variant 経路の non-negative check pin (cm、新規 absolute unit)。
        assert_eq!(parse("-1cm", "padding-top"), None);
    }

    #[test]
    fn padding_top_rejects_negative_ex() {
        // 全 Length variant 経路の non-negative check pin (ex、新規 font-relative unit)。
        assert_eq!(parse("-1ex", "padding-top"), None);
    }

    #[test]
    fn padding_top_rejects_negative_lh() {
        // 全 Length variant 経路の non-negative check pin (`lh`/`rlh`、
        // `length_payload` の OR-pattern に `Lh`/`Rlh`
        // を足し忘れていないことの直接 pin)。
        assert_eq!(parse("-1lh", "padding-top"), None);
        assert_eq!(parse("-1rlh", "padding-top"), None);
    }

    /// 追加した残り unit (`rex` / `rch` / `ic` / `ric` /
    /// `mm` / `Q`) を `length_payload` 経由で直接 exercise する — 他 call site
    /// (font-size / width / height / margin / border-width / line-height) の
    /// テストは Ex / Ch / Cm / In / Pc しか通さないため、`length_payload` の
    /// OR-pattern 全 arm の patch coverage には本 test が要る。
    #[test]
    fn padding_top_accepts_remaining_additional_units() {
        assert_eq!(
            parse("1rex", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Rex(1.0)))
        );
        assert_eq!(
            parse("1rch", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Rch(1.0)))
        );
        assert_eq!(
            parse("1ic", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Ic(1.0)))
        );
        assert_eq!(
            parse("1ric", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Ric(1.0)))
        );
        assert_eq!(
            parse("1mm", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Mm(1.0)))
        );
        assert_eq!(
            parse("40Q", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Q(40.0)))
        );
    }

    // Verification — Sides::all constructor + PropertyKey mapping smoke。
    #[test]
    fn padding_key_maps_to_padding_property_keys() {
        // 5 discriminant (4 longhand + 1 shorthand) が個別 PropertyKey を返すこと。
        // cascade winner selection の discriminant integrity 確認。
        assert_eq!(
            PropertyValue::PaddingTop(Length::Px(0.0)).key(),
            PropertyKey::PaddingTop
        );
        assert_eq!(
            PropertyValue::PaddingRight(Length::Px(0.0)).key(),
            PropertyKey::PaddingRight
        );
        assert_eq!(
            PropertyValue::PaddingBottom(Length::Px(0.0)).key(),
            PropertyKey::PaddingBottom
        );
        assert_eq!(
            PropertyValue::PaddingLeft(Length::Px(0.0)).key(),
            PropertyKey::PaddingLeft
        );
        assert_eq!(
            PropertyValue::Padding(Sides::all(Length::Px(0.0))).key(),
            PropertyKey::Padding
        );
    }

    // ── line-height (CSS Inline 3 §5.1) ────────────────
    //
    // Verification 5/6/7 の spec-derived: grammar `normal |
    // <number [0,∞]> | <length-percentage [0,∞]>` — 4 accept branch + negative
    // reject + Number vs Length variant distinction を pin する。
    //
    // 37n sibling: parse_display (keyword accept)、parse_font_size (Length
    // post-filter for non-negative)、parse_length_value (unit dispatch)。

    #[test]
    fn line_height_parse_normal_keyword() {
        // Verification 5.1: `line-height: normal` → LineHeight::Normal
        assert_eq!(
            parse("normal", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Normal))
        );
    }

    #[test]
    fn line_height_parse_bare_number() {
        // Verification 5.2 + 6: `line-height: 1.5` (bare number, no unit) →
        // LineHeight::Number(1.5)。Token::Number arm を通り Length branch には
        // 落ちない (Number vs Length distinction load-bearing、下流 special
        // behavior "specified value inherit" のための variant tag)。
        assert_eq!(
            parse("1.5", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Number(1.5)))
        );
    }

    #[test]
    fn line_height_parse_length_px() {
        // Verification 5.3: `line-height: 24px` → LineHeight::Length(Px(24.0))
        assert_eq!(
            parse("24px", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Px(
                24.0
            ))))
        );
    }

    #[test]
    fn line_height_parse_length_percentage() {
        // Verification 5.4: `line-height: 150%` → LineHeight::Length(Percent(150.0))
        // parse_length_value(allow_percentage=true) が Percent branch を有効化。
        assert_eq!(
            parse("150%", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(
                Length::Percent(150.0)
            )))
        );
    }

    #[test]
    fn line_height_number_vs_em_are_distinct_variants() {
        // Verification 6: `1.5` (unitless) と `1.5em` (dimensioned) は同じ scalar
        // でも別 variant に mapping (Token::Number vs Token::Dimension で分岐)。
        // spec §5.1 unitless number は child が specified value を inherit する
        // special behavior、Length variant は通常 resolve — 下流が区別する必要。
        let number = parse("1.5", "line-height");
        let length_em = parse("1.5em", "line-height");
        assert_eq!(
            number,
            Some(PropertyValue::LineHeight(LineHeight::Number(1.5)))
        );
        assert_eq!(
            length_em,
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Em(
                1.5
            ))))
        );
        assert_ne!(number, length_em, "Number and Length must be distinct");
    }

    #[test]
    fn line_height_accepts_length_em_rem_pt() {
        // 5 unit sample の length-percentage branch smoke — parse_length_value
        // helper との integration を pin (em/rem/pt helper 経由)。
        assert_eq!(
            parse("1.2em", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Em(
                1.2
            ))))
        );
        assert_eq!(
            parse("1rem", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Rem(
                1.0
            ))))
        );
        assert_eq!(
            parse("12pt", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Pt(
                12.0
            ))))
        );
    }

    #[test]
    fn sides_all_constructor_replicates_value() {
        // Sides::all(v) は 4 field を全て v で埋める。
        let sides = Sides::all(Length::Px(7.5));
        assert_eq!(sides.top, Length::Px(7.5));
        assert_eq!(sides.right, Length::Px(7.5));
        assert_eq!(sides.bottom, Length::Px(7.5));
        assert_eq!(sides.left, Length::Px(7.5));
    }

    #[test]
    fn padding_shorthand_five_values_dropped_by_leftover() {
        // 5 個目以降は本 helper が consume せず leftover として残す。
        // parse_value 単体では 4-value form の Some を返すが、caller (rule.rs)
        // の expect_exhausted が leftover を検知して declaration drop するので、
        // rule.rs 経由で drop 確認。
        let source = "padding: 10px 20px 30px 40px 50px;";
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let decls = crate::rule::parse_declaration_block(&mut parser);
        assert!(
            decls.is_empty(),
            "5-value form must be dropped by expect_exhausted"
        );
    }

    #[test]
    fn line_height_accepts_zero_number_and_length() {
        // spec `[0,∞]`: 0 は境界の valid value。
        // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths> clause 2:
        // "if a 0 could be parsed as either a `<number>` or a `<length>` in a
        // property (such as line-height), it must parse as a `<number>`" —
        // parse_line_height は expect_number branch を parse_length_value より
        // 先に試すため、bare `0` は LineHeight::Number(0.0) として確定 (unitless-zero
        // clause の Length 経路が導入した Px(0.0) route ではない)。
        assert_eq!(
            parse("0", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Number(0.0)))
        );
        assert_eq!(
            parse("0px", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Px(
                0.0
            ))))
        );
    }

    #[test]
    fn padding_case_insensitive_unit() {
        // CSS spec: unit identifier は ASCII case-insensitive。
        assert_eq!(
            parse("10PX", "padding-top"),
            Some(PropertyValue::PaddingTop(Length::Px(10.0)))
        );
        assert_eq!(
            parse("2EM", "padding-bottom"),
            Some(PropertyValue::PaddingBottom(Length::Em(2.0)))
        );
    }

    #[test]
    fn line_height_rejects_negative_number() {
        // Verification 7: `<number [0,∞]>` — 負値は spec grammar 違反 → drop。
        assert_eq!(parse("-1.5", "line-height"), None);
    }

    #[test]
    fn line_height_rejects_negative_number_with_trailing_length() {
        // Regression: Number branch は
        // Token::Number を commit した後 fallthrough すべきでない。fallthrough
        // していた旧実装では `-0.5 20px` が Length branch で `20px` を拾い
        // silently accept されていた (spec-invalid → 本来 declaration drop)。
        // 現行: Number 到達 = 確定、`[0,∞]` 違反は declaration drop、
        // 後続 token は expect_exhausted なくとも parse_length_value 側で拾わない。
        assert_eq!(parse("-0.5 20px", "line-height"), None);
        // 対称: negative number + em / % も同じく drop。
        assert_eq!(parse("-0.5 1em", "line-height"), None);
        assert_eq!(parse("-1.0 50%", "line-height"), None);
    }

    #[test]
    fn line_height_rejects_negative_length() {
        // Verification 7: `<length-percentage [0,∞]>` — 負 length は drop。
        assert_eq!(parse("-10px", "line-height"), None);
        assert_eq!(parse("-1em", "line-height"), None);
    }

    #[test]
    fn line_height_rejects_negative_percentage() {
        // Verification 7: 負 percentage も spec `[0,∞]` 違反 → drop。
        assert_eq!(parse("-50%", "line-height"), None);
    }

    #[test]
    fn line_height_rejects_unknown_keyword() {
        // spec grammar 外の ident (`auto` / `medium` 等) は (a) spec-invalid、
        // silent drop。CSS-wide keyword は別 test
        // (`line_height_rejects_css_wide_keyword`) — (a) ではなく (b) の
        // 非対応なので混同しないこと。
        assert_eq!(parse("auto", "line-height"), None);
        assert_eq!(parse("medium", "line-height"), None);
    }

    #[test]
    fn line_height_rejects_css_wide_keyword() {
        // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
        // canonical: PropertyValue doc「CSS-wide keyword」節。
        assert_eq!(parse("inherit", "line-height"), None);
        assert_eq!(parse("initial", "line-height"), None);
        assert_eq!(parse("unset", "line-height"), None);
        assert_eq!(parse("revert", "line-height"), None);
        assert_eq!(parse("revert-layer", "line-height"), None);
    }

    #[test]
    fn line_height_rejects_unsupported_unit() {
        // parse_length_value が silent drop する unit (`vw` / `cap` 等、
        // 現状未対応) は helper 側で `None` →
        // line-height parse も declaration drop。`ch` / `lh` / `rlh` は
        // それぞれ受理側へ移った
        // (`line_height_accepts_ch` / `line_height_accepts_lh` 参照)。
        assert_eq!(parse("10vw", "line-height"), None);
        assert_eq!(parse("10cap", "line-height"), None);
    }

    #[test]
    fn line_height_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh`.
        // `line-height` itself is a valid context for `lh`/`rlh` at parse
        // time (unlike `font-size`, which `parse_font_size` post-filters —
        // see that function's doc for why) — the self-reference resolve
        // basis is handled downstream in `crate::resolve::resolve_line_height`.
        assert_eq!(
            parse("10lh", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Lh(
                10.0
            ))))
        );
        assert_eq!(
            parse("1rlh", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Rlh(
                1.0
            ))))
        );
    }

    #[test]
    fn line_height_accepts_ch() {
        assert_eq!(
            parse("2ch", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Length(Length::Ch(
                2.0
            ))))
        );
    }

    #[test]
    fn line_height_normal_is_case_insensitive() {
        // CSS spec: keyword ident は ASCII case-insensitive
        // (expect_ident_matching が case-insensitive)。
        assert_eq!(
            parse("NORMAL", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Normal))
        );
        assert_eq!(
            parse("Normal", "line-height"),
            Some(PropertyValue::LineHeight(LineHeight::Normal))
        );
    }

    #[test]
    fn line_height_key_maps_to_line_height_property_key() {
        // PropertyValue::LineHeight → PropertyKey::LineHeight (cascade winner 選択の
        // discriminant integrity、既存 sibling font_size / display と同じ pattern)。
        let v = PropertyValue::LineHeight(LineHeight::Normal);
        assert_eq!(v.key(), PropertyKey::LineHeight);
        let v = PropertyValue::LineHeight(LineHeight::Number(1.5));
        assert_eq!(v.key(), PropertyKey::LineHeight);
        let v = PropertyValue::LineHeight(LineHeight::Length(Length::Px(24.0)));
        assert_eq!(v.key(), PropertyKey::LineHeight);
    }

    #[test]
    fn position_key_maps_to_position_property_key() {
        // PropertyValue::Position → PropertyKey::Position (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* / content / string-set と
        // 同じ pattern)。
        let v = PropertyValue::Position(PositionValue::Static);
        assert_eq!(v.key(), PropertyKey::Position);
        let v = PropertyValue::Position(PositionValue::Running(SmolStr::new("hdr")));
        assert_eq!(v.key(), PropertyKey::Position);
    }

    // ── text-align (CSS Text 3 §6.1) ──
    //
    // Value grammar (§6.1 spec verbatim):
    //   start | end | left | right | center | justify | match-parent | justify-all
    // Initial: start / Inherited: yes / spec 上 shorthand (text-align-all +
    // text-align-last、単一 field で保持 = (b)
    // 非対応)。inheritance test は cascade.rs 側 (parent → child コピー、display
    // non-inherited との対比)。

    #[test]
    fn text_align_parse_all_eight_keywords() {
        // Verification 5: 8 keyword が全て正しく TextAlign variant にマップされる。
        // 1 test で全 arm coverage (patch coverage 100% 目標)。
        assert_eq!(
            parse("start", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::Start))
        );
        assert_eq!(
            parse("end", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::End))
        );
        assert_eq!(
            parse("left", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::Left))
        );
        assert_eq!(
            parse("right", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::Right))
        );
        assert_eq!(
            parse("center", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::Center))
        );
        assert_eq!(
            parse("justify", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::Justify))
        );
        assert_eq!(
            parse("match-parent", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::MatchParent))
        );
        assert_eq!(
            parse("justify-all", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::JustifyAll))
        );
    }

    #[test]
    fn text_align_is_case_insensitive() {
        // Verification 6: CSS spec 慣行 — property value keyword は ASCII case-insensitive。
        assert_eq!(
            parse("CENTER", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::Center))
        );
        assert_eq!(
            parse("Justify-All", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::JustifyAll))
        );
        assert_eq!(
            parse("Match-Parent", "text-align"),
            Some(PropertyValue::TextAlign(TextAlign::MatchParent))
        );
    }

    #[test]
    fn text_align_rejects_unknown_keyword() {
        // spec §6.1 grammar に含まれない keyword は silent drop (spec-invalid)。
        // `middle` は typo/俗称、`text-align` spec に存在しない。
        assert_eq!(parse("middle", "text-align"), None);
        assert_eq!(parse("baseline", "text-align"), None);
        assert_eq!(parse("top", "text-align"), None);
    }

    #[test]
    fn text_align_rejects_css_wide_keyword() {
        // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5 keyword
        // の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が canonical。
        assert_eq!(parse("inherit", "text-align"), None);
        assert_eq!(parse("initial", "text-align"), None);
        assert_eq!(parse("unset", "text-align"), None);
        assert_eq!(parse("revert", "text-align"), None);
        assert_eq!(parse("revert-layer", "text-align"), None);
    }

    #[test]
    fn text_align_rejects_string_value() {
        // CSS Text 3 §6.1 grammar は 8 keyword のみ、`<string>` value は本 crate
        // が引用する level では未定義 → spec-invalid、silent drop。
        // (Text 4 draft では tabular-data character alignment 用に `<string>` が
        // 検討されているが本 crate は Text 3 pin。expect_ident が String token を
        // reject する経路で `None` を返す。)
        assert_eq!(parse(r#""." "#, "text-align"), None);
    }

    #[test]
    fn text_align_rejects_non_ident() {
        // Number / dimension token は expect_ident で reject。
        assert_eq!(parse("16px", "text-align"), None);
        assert_eq!(parse("100", "text-align"), None);
    }

    #[test]
    fn text_align_key_maps_to_text_align_property_key() {
        // PropertyValue::TextAlign → PropertyKey::TextAlign (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* / content / string-set /
        // position と同じ pattern)。
        let v = PropertyValue::TextAlign(TextAlign::Start);
        assert_eq!(v.key(), PropertyKey::TextAlign);
        let v = PropertyValue::TextAlign(TextAlign::Center);
        assert_eq!(v.key(), PropertyKey::TextAlign);
    }

    // ── text-indent (CSS Text 3 §8.1) ──
    //
    // Full value grammar: `<length-percentage> && hanging? && each-line?` —
    // this crate implements only the `<length-percentage>` component.
    // Initial: 0 / Applies to: block containers / Inherited: yes /
    // Percentages: refers to block container's own inline-axis inner size /
    // Computed value: computed <length-percentage> value, plus any
    // specified keywords.

    #[test]
    fn text_indent_parse_px() {
        assert_eq!(
            parse("20px", "text-indent"),
            Some(PropertyValue::TextIndent(Length::Px(20.0)))
        );
    }

    #[test]
    fn text_indent_parse_percentage() {
        assert_eq!(
            parse("10%", "text-indent"),
            Some(PropertyValue::TextIndent(Length::Percent(10.0)))
        );
    }

    #[test]
    fn text_indent_parse_em() {
        assert_eq!(
            parse("2em", "text-indent"),
            Some(PropertyValue::TextIndent(Length::Em(2.0)))
        );
    }

    #[test]
    fn text_indent_accepts_negative_length() {
        // CSS Text 3 §8.1 places no `[0,∞]` restriction on this grammar
        // (unlike `padding-top` — `PropertyValue::TextIndent` doc).
        assert_eq!(
            parse("-2em", "text-indent"),
            Some(PropertyValue::TextIndent(Length::Em(-2.0)))
        );
    }

    #[test]
    fn text_indent_accepts_zero() {
        // CSS Values 3 §5 unitless-zero clause — bare `0` is a valid `<length>`.
        assert_eq!(
            parse("0", "text-indent"),
            Some(PropertyValue::TextIndent(Length::Px(0.0)))
        );
    }

    #[test]
    fn text_indent_rejects_unsupported_unit() {
        // `cap` (CSS Values 4 §6.1.1) is not implemented — dropped by
        // `parse_length_value`'s `Token::Dimension` fall-through, same as the
        // `margin_side_rejects_unsupported_unit` sibling.
        assert_eq!(parse("1cap", "text-indent"), None);
    }

    #[test]
    fn text_indent_rejects_auto() {
        // Unlike `margin` / `width`, `text-indent`'s grammar has no `auto`
        // alternative — the `hanging`/`each-line` keywords are the only
        // idents the full grammar accepts, and this crate does not parse
        // them either ((b) 非対応, `PropertyValue::TextIndent` doc's "Scope
        // carving" section). `parse_length_value` reads one token via
        // `input.next()` and only has match arms for `Token::Dimension` /
        // `Token::Percentage` / a zero `Token::Number` — an `Ident` token
        // (`auto` included) matches none of them and falls through to the
        // trailing `_ => None`.
        assert_eq!(parse("auto", "text-indent"), None);
    }

    #[test]
    fn text_indent_key_maps_to_text_indent_property_key() {
        let v = PropertyValue::TextIndent(Length::Px(20.0));
        assert_eq!(v.key(), PropertyKey::TextIndent);
    }

    // ── direction (CSS Writing Modes 4 §2.1) ──
    //
    // Value grammar (§2.1 spec verbatim): ltr | rtl
    // Initial: ltr / Inherited: yes / Computed value: specified value。

    #[test]
    fn direction_parse_both_keywords() {
        assert_eq!(
            parse("ltr", "direction"),
            Some(PropertyValue::Direction(Direction::Ltr))
        );
        assert_eq!(
            parse("rtl", "direction"),
            Some(PropertyValue::Direction(Direction::Rtl))
        );
    }

    #[test]
    fn direction_is_case_insensitive() {
        assert_eq!(
            parse("LTR", "direction"),
            Some(PropertyValue::Direction(Direction::Ltr))
        );
        assert_eq!(
            parse("Rtl", "direction"),
            Some(PropertyValue::Direction(Direction::Rtl))
        );
    }

    #[test]
    fn direction_rejects_unknown_keyword() {
        // 旧 draft 相当の `auto` は現行 §2.1 grammar に無い — 実 spec-invalid。
        assert_eq!(parse("auto", "direction"), None);
        assert_eq!(parse("horizontal-tb", "direction"), None);
    }

    #[test]
    fn direction_rejects_css_wide_keyword() {
        // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5 keyword
        // の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が canonical。
        assert_eq!(parse("inherit", "direction"), None);
        assert_eq!(parse("initial", "direction"), None);
        assert_eq!(parse("unset", "direction"), None);
        assert_eq!(parse("revert", "direction"), None);
        assert_eq!(parse("revert-layer", "direction"), None);
    }

    #[test]
    fn direction_rejects_non_ident() {
        assert_eq!(parse("16px", "direction"), None);
        assert_eq!(parse(r#""ltr""#, "direction"), None);
    }

    #[test]
    fn direction_key_maps_to_direction_property_key() {
        let v = PropertyValue::Direction(Direction::Ltr);
        assert_eq!(v.key(), PropertyKey::Direction);
        let v = PropertyValue::Direction(Direction::Rtl);
        assert_eq!(v.key(), PropertyKey::Direction);
    }

    // ── overflow-x / overflow-y / overflow (CSS Overflow 3 §3.1) ──
    //
    // Value grammar (§3.1 spec verbatim): visible | hidden | clip | scroll |
    // auto. Initial: visible / Inherited: no. `overflow` shorthand grammar:
    // `<'overflow-block'>{1,2}` (mapped to physical x/y — see `OverflowValue`
    // doc's Non-goal note).

    #[test]
    fn overflow_x_parse_all_five_keywords() {
        assert_eq!(
            parse("visible", "overflow-x"),
            Some(PropertyValue::OverflowX(OverflowValue::Visible))
        );
        assert_eq!(
            parse("hidden", "overflow-x"),
            Some(PropertyValue::OverflowX(OverflowValue::Hidden))
        );
        assert_eq!(
            parse("clip", "overflow-x"),
            Some(PropertyValue::OverflowX(OverflowValue::Clip))
        );
        assert_eq!(
            parse("scroll", "overflow-x"),
            Some(PropertyValue::OverflowX(OverflowValue::Scroll))
        );
        assert_eq!(
            parse("auto", "overflow-x"),
            Some(PropertyValue::OverflowX(OverflowValue::Auto))
        );
    }

    #[test]
    fn overflow_y_parse_all_five_keywords() {
        // Sibling of `overflow_x_parse_all_five_keywords` — same grammar,
        // separate `PropertyValue` variant / `PropertyKey`.
        assert_eq!(
            parse("visible", "overflow-y"),
            Some(PropertyValue::OverflowY(OverflowValue::Visible))
        );
        assert_eq!(
            parse("hidden", "overflow-y"),
            Some(PropertyValue::OverflowY(OverflowValue::Hidden))
        );
        assert_eq!(
            parse("clip", "overflow-y"),
            Some(PropertyValue::OverflowY(OverflowValue::Clip))
        );
        assert_eq!(
            parse("scroll", "overflow-y"),
            Some(PropertyValue::OverflowY(OverflowValue::Scroll))
        );
        assert_eq!(
            parse("auto", "overflow-y"),
            Some(PropertyValue::OverflowY(OverflowValue::Auto))
        );
    }

    #[test]
    fn overflow_is_case_insensitive() {
        assert_eq!(
            parse("HIDDEN", "overflow-x"),
            Some(PropertyValue::OverflowX(OverflowValue::Hidden))
        );
        assert_eq!(
            parse("Auto", "overflow-y"),
            Some(PropertyValue::OverflowY(OverflowValue::Auto))
        );
    }

    #[test]
    fn overflow_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "overflow-x"), None);
        assert_eq!(parse("collapse", "overflow-y"), None);
        // `padding-box` etc. are not part of this property's grammar.
        assert_eq!(parse("padding-box", "overflow-x"), None);
    }

    #[test]
    fn overflow_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "overflow-x"), None);
            assert_eq!(parse(kw, "overflow-y"), None);
            assert_eq!(parse(kw, "overflow"), None);
        }
    }

    #[test]
    fn overflow_rejects_non_ident() {
        assert_eq!(parse("16px", "overflow-x"), None);
        assert_eq!(parse(r#""hidden""#, "overflow-y"), None);
    }

    #[test]
    fn overflow_x_key_maps_to_overflow_x_property_key() {
        let v = PropertyValue::OverflowX(OverflowValue::Hidden);
        assert_eq!(v.key(), PropertyKey::OverflowX);
    }

    #[test]
    fn overflow_y_key_maps_to_overflow_y_property_key() {
        let v = PropertyValue::OverflowY(OverflowValue::Scroll);
        assert_eq!(v.key(), PropertyKey::OverflowY);
    }

    #[test]
    fn overflow_key_maps_to_overflow_property_key() {
        let v = PropertyValue::Overflow(OverflowXY::both(OverflowValue::Auto));
        assert_eq!(v.key(), PropertyKey::Overflow);
    }

    #[test]
    fn overflow_shorthand_one_value_spreads_to_both_axes() {
        // §3.1 "If there is only one component value, it applies to all
        // sides" (paraphrase of the shared `<'overflow-block'>{1,2}`
        // expansion rule this crate maps onto physical x/y).
        assert_eq!(
            parse("hidden", "overflow"),
            Some(PropertyValue::Overflow(OverflowXY::both(
                OverflowValue::Hidden
            )))
        );
    }

    #[test]
    fn overflow_shorthand_two_values_set_x_then_y() {
        // §3.1 verbatim: "The overflow property is a shorthand property that
        // sets the specified values of overflow-x and overflow-y in that
        // order."
        assert_eq!(
            parse("hidden scroll", "overflow"),
            Some(PropertyValue::Overflow(OverflowXY {
                x: OverflowValue::Hidden,
                y: OverflowValue::Scroll,
            }))
        );
    }

    #[test]
    fn overflow_shorthand_leaves_extra_values_for_caller_exhausted_check() {
        // Mirrors `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`
        // — this helper consumes only 2 values; a 3rd is left unconsumed for
        // the `expect_exhausted` caller in `rule.rs` to reject the whole
        // declaration. `parse_value` itself does not call `expect_exhausted`,
        // so this direct call only demonstrates the helper's own consumption,
        // not the end-to-end drop (that is `rule.rs`'s job, pinned by
        // `rule::tests::overflow_shorthand_three_values_declaration_dropped`).
        assert_eq!(
            parse("hidden scroll auto", "overflow"),
            Some(PropertyValue::Overflow(OverflowXY {
                x: OverflowValue::Hidden,
                y: OverflowValue::Scroll,
            }))
        );
    }

    // ── text-decoration-line (CSS Text Decoration Module Level 3 §2.1) ──

    #[test]
    fn text_decoration_line_parses_none() {
        assert_eq!(
            parse("none", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(TextDecorationLine::NONE))
        );
    }

    #[test]
    fn text_decoration_line_parses_each_single_keyword() {
        assert_eq!(
            parse("underline", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(
                TextDecorationLine::UNDERLINE
            ))
        );
        assert_eq!(
            parse("overline", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(
                TextDecorationLine::OVERLINE
            ))
        );
        assert_eq!(
            parse("line-through", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(
                TextDecorationLine::LINE_THROUGH
            ))
        );
        assert_eq!(
            parse("blink", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(TextDecorationLine::BLINK))
        );
    }

    #[test]
    fn text_decoration_line_parses_combination_in_any_order() {
        // `||` grammar: order-independent. Both orderings of the same pair
        // must produce the same flag set.
        assert_eq!(
            parse("underline overline", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(TextDecorationLine {
                underline: true,
                overline: true,
                line_through: false,
                blink: false,
            }))
        );
        assert_eq!(
            parse("overline underline", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(TextDecorationLine {
                underline: true,
                overline: true,
                line_through: false,
                blink: false,
            }))
        );
    }

    #[test]
    fn text_decoration_line_parses_all_four_combined() {
        assert_eq!(
            parse(
                "underline overline line-through blink",
                "text-decoration-line"
            ),
            Some(PropertyValue::TextDecorationLine(TextDecorationLine {
                underline: true,
                overline: true,
                line_through: true,
                blink: true,
            }))
        );
    }

    #[test]
    fn text_decoration_line_is_case_insensitive() {
        assert_eq!(
            parse("NONE", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(TextDecorationLine::NONE))
        );
        assert_eq!(
            parse("Underline", "text-decoration-line"),
            Some(PropertyValue::TextDecorationLine(
                TextDecorationLine::UNDERLINE
            ))
        );
    }

    #[test]
    fn text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check() {
        // Each `||` component at most once (CSS Values 4 §2.2). The 2nd
        // `underline` is left unconsumed by `parse_text_decoration_line`
        // (its flag is already set) — `parse_border_shorthand`'s sibling
        // `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
        // test pattern: the helper itself still returns `Some` (1st token
        // consumed), and rejection is the caller's (`rule.rs`'s
        // `DeclParser::parse_value`'s `expect_exhausted`) responsibility —
        // pinned end-to-end by `rule.rs`'s
        // `text_decoration_line_duplicate_and_none_combination_declarations_dropped`
        // sibling test.
        let mut input = ParserInput::new("underline underline");
        let mut parser = Parser::new(&mut input);
        assert_eq!(
            parse_text_decoration_line(&mut parser),
            Some(TextDecorationLine::UNDERLINE)
        );
        assert!(!parser.is_exhausted());
    }

    #[test]
    fn text_decoration_line_none_combined_with_a_keyword_leaves_leftover() {
        // `none | [ ... ]` — `none` is a separate top-level alternative, not
        // a member of the `||` combination, so it cannot co-occur with the
        // other keywords in either order. Same "helper returns `Some`,
        // leftover is the caller's `expect_exhausted` responsibility" shape
        // as the duplicate-keyword sibling test above.
        let mut input = ParserInput::new("none underline");
        let mut parser = Parser::new(&mut input);
        assert_eq!(
            parse_text_decoration_line(&mut parser),
            Some(TextDecorationLine::NONE)
        );
        assert!(!parser.is_exhausted());

        let mut input = ParserInput::new("underline none");
        let mut parser = Parser::new(&mut input);
        assert_eq!(
            parse_text_decoration_line(&mut parser),
            Some(TextDecorationLine::UNDERLINE)
        );
        assert!(!parser.is_exhausted());
    }

    #[test]
    fn text_decoration_line_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "text-decoration-line"), None);
    }

    #[test]
    fn text_decoration_line_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "text-decoration-line"), None);
        }
    }

    #[test]
    fn text_decoration_line_rejects_non_ident() {
        assert_eq!(parse("16px", "text-decoration-line"), None);
        assert_eq!(parse(r#""underline""#, "text-decoration-line"), None);
    }

    // ── text-decoration-style (CSS Text Decoration Module Level 3 §2.2) ──

    #[test]
    fn text_decoration_style_parses_all_five_keywords() {
        for (kw, expected) in [
            ("solid", TextDecorationStyle::Solid),
            ("double", TextDecorationStyle::Double),
            ("dotted", TextDecorationStyle::Dotted),
            ("dashed", TextDecorationStyle::Dashed),
            ("wavy", TextDecorationStyle::Wavy),
        ] {
            assert_eq!(
                parse(kw, "text-decoration-style"),
                Some(PropertyValue::TextDecorationStyle(expected))
            );
        }
    }

    #[test]
    fn text_decoration_style_is_case_insensitive() {
        assert_eq!(
            parse("WAVY", "text-decoration-style"),
            Some(PropertyValue::TextDecorationStyle(
                TextDecorationStyle::Wavy
            ))
        );
    }

    #[test]
    fn text_decoration_style_rejects_unknown_keyword() {
        // `underline` is a `text-decoration-line` keyword, not a
        // `text-decoration-style` one — the two properties' keyword sets are
        // disjoint.
        assert_eq!(parse("underline", "text-decoration-style"), None);
        assert_eq!(parse("bogus", "text-decoration-style"), None);
    }

    // ── text-decoration-color (CSS Text Decoration Module Level 3 §2.3) ──

    #[test]
    fn text_decoration_color_parses_currentcolor() {
        assert_eq!(
            parse("currentcolor", "text-decoration-color"),
            Some(PropertyValue::TextDecorationColor(
                TextDecorationColor::CurrentColor
            ))
        );
        assert_eq!(
            parse("CurrentColor", "text-decoration-color"),
            Some(PropertyValue::TextDecorationColor(
                TextDecorationColor::CurrentColor
            ))
        );
    }

    #[test]
    fn text_decoration_color_parses_resolved_color() {
        assert_eq!(
            parse("red", "text-decoration-color"),
            Some(PropertyValue::TextDecorationColor(
                TextDecorationColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                })
            ))
        );
        assert_eq!(
            parse("#00ff00", "text-decoration-color"),
            Some(PropertyValue::TextDecorationColor(
                TextDecorationColor::Resolved(CssColor {
                    r: 0,
                    g: 255,
                    b: 0,
                    a: 255,
                })
            ))
        );
    }

    #[test]
    fn text_decoration_color_rejects_unknown_ident() {
        assert_eq!(parse("bogus", "text-decoration-color"), None);
    }

    // ── text-decoration shorthand (CSS Text Decoration Module Level 3
    // §2.4) ──
    //
    // `<'text-decoration-line'> || <'text-decoration-style'> ||
    // <'text-decoration-color'>`. Omitted components fill with their
    // longhand's initial value (verbatim: "Omitted values are set to their
    // initial values.").

    #[test]
    fn text_decoration_shorthand_line_only_fills_other_two_with_initial() {
        assert_eq!(
            parse("underline", "text-decoration"),
            Some(PropertyValue::TextDecoration(TextDecorationShorthand {
                line: TextDecorationLine::UNDERLINE,
                style: TextDecorationStyle::Solid,
                color: TextDecorationColor::CurrentColor,
            }))
        );
    }

    #[test]
    fn text_decoration_shorthand_style_only_fills_other_two_with_initial() {
        // A bare style keyword is a spec-valid shorthand value under `||`
        // (`text-decoration: wavy;`) — this was previously unreachable
        // (pre-longhand-decomposition `text-decoration` only accepted
        // `none`/`underline`).
        assert_eq!(
            parse("wavy", "text-decoration"),
            Some(PropertyValue::TextDecoration(TextDecorationShorthand {
                line: TextDecorationLine::NONE,
                style: TextDecorationStyle::Wavy,
                color: TextDecorationColor::CurrentColor,
            }))
        );
    }

    #[test]
    fn text_decoration_shorthand_color_only_fills_other_two_with_initial() {
        // Likewise a bare color (`text-decoration: red;`) was rejected
        // wholesale pre-decomposition; `||` makes it valid on its own.
        assert_eq!(
            parse("red", "text-decoration"),
            Some(PropertyValue::TextDecoration(TextDecorationShorthand {
                line: TextDecorationLine::NONE,
                style: TextDecorationStyle::Solid,
                color: TextDecorationColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }))
        );
    }

    #[test]
    fn text_decoration_shorthand_parses_all_three_in_any_order() {
        let expected = Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        }));
        assert_eq!(parse("underline wavy red", "text-decoration"), expected);
        assert_eq!(parse("red wavy underline", "text-decoration"), expected);
        assert_eq!(parse("wavy red underline", "text-decoration"), expected);
    }

    #[test]
    fn text_decoration_shorthand_line_combination_plus_style_and_color() {
        assert_eq!(
            parse("underline overline wavy red", "text-decoration"),
            Some(PropertyValue::TextDecoration(TextDecorationShorthand {
                line: TextDecorationLine {
                    underline: true,
                    overline: true,
                    line_through: false,
                    blink: false,
                },
                style: TextDecorationStyle::Wavy,
                color: TextDecorationColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }))
        );
    }

    #[test]
    fn text_decoration_shorthand_rejects_empty_value() {
        assert_eq!(parse("", "text-decoration"), None);
    }

    #[test]
    fn text_decoration_shorthand_two_style_components_leaves_leftover_for_caller_exhausted_check() {
        // Each `||` component at most once — a 2nd style keyword ("wavy")
        // doesn't match any unfilled slot (style already filled by "solid";
        // it isn't a line keyword or a `<color>`) so it's left unconsumed.
        // Same "helper returns `Some`, caller's `expect_exhausted` drops the
        // whole declaration" shape as `parse_border_shorthand`'s
        // `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
        // — end-to-end rejection is pinned by `rule.rs`'s
        // `text_decoration_shorthand_two_style_components_declaration_dropped`.
        let mut input = ParserInput::new("solid wavy");
        let mut parser = Parser::new(&mut input);
        assert_eq!(
            parse_text_decoration_shorthand(&mut parser),
            Some(TextDecorationShorthand {
                line: TextDecorationLine::NONE,
                style: TextDecorationStyle::Solid,
                color: TextDecorationColor::CurrentColor,
            })
        );
        assert!(!parser.is_exhausted());
    }

    #[test]
    fn text_decoration_shorthand_rejects_css_wide_keyword() {
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "text-decoration"), None);
        }
    }

    #[test]
    fn text_decoration_key_maps_to_text_decoration_property_key() {
        let line = PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE);
        assert_eq!(line.key(), PropertyKey::TextDecorationLine);
        let style = PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy);
        assert_eq!(style.key(), PropertyKey::TextDecorationStyle);
        let color = PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor);
        assert_eq!(color.key(), PropertyKey::TextDecorationColor);
        let shorthand = PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
        });
        assert_eq!(shorthand.key(), PropertyKey::TextDecoration);
    }

    // ── vertical-align (CSS 2.1 §10.8.1) ──
    //
    // Value grammar (minimal scope — `VerticalAlign` doc's "Scope carving"
    // section): baseline | sub | super. Initial: baseline / Inherited: no /
    // Computed value: specified keyword.

    #[test]
    fn vertical_align_parse_all_keywords() {
        assert_eq!(
            parse("baseline", "vertical-align"),
            Some(PropertyValue::VerticalAlign(VerticalAlign::Baseline))
        );
        assert_eq!(
            parse("sub", "vertical-align"),
            Some(PropertyValue::VerticalAlign(VerticalAlign::Sub))
        );
        assert_eq!(
            parse("super", "vertical-align"),
            Some(PropertyValue::VerticalAlign(VerticalAlign::Super))
        );
    }

    #[test]
    fn vertical_align_is_case_insensitive() {
        assert_eq!(
            parse("BASELINE", "vertical-align"),
            Some(PropertyValue::VerticalAlign(VerticalAlign::Baseline))
        );
        assert_eq!(
            parse("Sub", "vertical-align"),
            Some(PropertyValue::VerticalAlign(VerticalAlign::Sub))
        );
        assert_eq!(
            parse("SUPER", "vertical-align"),
            Some(PropertyValue::VerticalAlign(VerticalAlign::Super))
        );
    }

    #[test]
    fn vertical_align_rejects_unimplemented_keywords() {
        // (b) not supported — `top` / `text-top` / `middle` / `bottom` /
        // `text-bottom` and the `<percentage>`/`<length>` value forms are
        // explicit follow-up (`VerticalAlign` doc's "Scope carving"
        // section), not (a) spec-invalid.
        for kw in ["top", "text-top", "middle", "bottom", "text-bottom"] {
            assert_eq!(parse(kw, "vertical-align"), None);
        }
    }

    #[test]
    fn vertical_align_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "vertical-align"), None);
    }

    #[test]
    fn vertical_align_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "vertical-align"), None);
        }
    }

    #[test]
    fn vertical_align_rejects_non_ident() {
        assert_eq!(parse("16px", "vertical-align"), None);
        assert_eq!(parse("50%", "vertical-align"), None);
    }

    #[test]
    fn vertical_align_key_maps_to_vertical_align_property_key() {
        let v = PropertyValue::VerticalAlign(VerticalAlign::Baseline);
        assert_eq!(v.key(), PropertyKey::VerticalAlign);
        let v = PropertyValue::VerticalAlign(VerticalAlign::Sub);
        assert_eq!(v.key(), PropertyKey::VerticalAlign);
        let v = PropertyValue::VerticalAlign(VerticalAlign::Super);
        assert_eq!(v.key(), PropertyKey::VerticalAlign);
    }

    // ── font-style (CSS Fonts 4 §2.4) ──
    //
    // Value grammar (§2.4 spec verbatim, full property grammar): `normal |
    // italic | left | right | oblique <angle [-90deg,90deg]>?`. This crate
    // implements only normal / italic (`FontStyle` doc's "Scope carving"
    // section). Initial: normal / Inherited: yes / Computed value:
    // specified keyword (angle-bearing branch unreachable at this scope).

    #[test]
    fn font_style_parse_both_keywords() {
        assert_eq!(
            parse("normal", "font-style"),
            Some(PropertyValue::FontStyle(FontStyle::Normal))
        );
        assert_eq!(
            parse("italic", "font-style"),
            Some(PropertyValue::FontStyle(FontStyle::Italic))
        );
    }

    #[test]
    fn font_style_is_case_insensitive() {
        assert_eq!(
            parse("NORMAL", "font-style"),
            Some(PropertyValue::FontStyle(FontStyle::Normal))
        );
        assert_eq!(
            parse("Italic", "font-style"),
            Some(PropertyValue::FontStyle(FontStyle::Italic))
        );
    }

    #[test]
    fn font_style_rejects_unimplemented_keywords() {
        // (b) not supported — `oblique` (with or without an angle) and the
        // `left` / `right` slant-direction keywords are spec-valid but
        // unimplemented (`FontStyle` doc's "Scope carving" section), not
        // (a) spec-invalid.
        assert_eq!(parse("oblique", "font-style"), None);
        assert_eq!(parse("left", "font-style"), None);
        assert_eq!(parse("right", "font-style"), None);
    }

    #[test]
    fn font_style_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "font-style"), None);
    }

    #[test]
    fn font_style_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "font-style"), None);
        }
    }

    #[test]
    fn font_style_rejects_non_ident() {
        assert_eq!(parse("16px", "font-style"), None);
        assert_eq!(parse(r#""italic""#, "font-style"), None);
    }

    #[test]
    fn font_style_key_maps_to_font_style_property_key() {
        let v = PropertyValue::FontStyle(FontStyle::Normal);
        assert_eq!(v.key(), PropertyKey::FontStyle);
        let v = PropertyValue::FontStyle(FontStyle::Italic);
        assert_eq!(v.key(), PropertyKey::FontStyle);
    }

    // ── text-transform (CSS Text Module Level 3 §2.1) ──
    //
    // Value grammar (§2.1 spec verbatim, full property grammar): `none |
    // [capitalize | uppercase | lowercase] || full-width || full-size-kana`.
    // This crate implements only none / capitalize / uppercase / lowercase
    // (`TextTransform` doc's "Scope carving" section). Initial: none /
    // Inherited: yes / Computed value: specified keyword.

    #[test]
    fn text_transform_parse_all_four_keywords() {
        assert_eq!(
            parse("none", "text-transform"),
            Some(PropertyValue::TextTransform(TextTransform::None))
        );
        assert_eq!(
            parse("capitalize", "text-transform"),
            Some(PropertyValue::TextTransform(TextTransform::Capitalize))
        );
        assert_eq!(
            parse("uppercase", "text-transform"),
            Some(PropertyValue::TextTransform(TextTransform::Uppercase))
        );
        assert_eq!(
            parse("lowercase", "text-transform"),
            Some(PropertyValue::TextTransform(TextTransform::Lowercase))
        );
    }

    #[test]
    fn text_transform_is_case_insensitive() {
        assert_eq!(
            parse("NONE", "text-transform"),
            Some(PropertyValue::TextTransform(TextTransform::None))
        );
        assert_eq!(
            parse("Uppercase", "text-transform"),
            Some(PropertyValue::TextTransform(TextTransform::Uppercase))
        );
    }

    #[test]
    fn text_transform_rejects_unimplemented_keywords() {
        // (b) not supported — `full-width` / `full-size-kana` are
        // spec-valid `||`-combinable keywords but unimplemented
        // (`TextTransform` doc's "Scope carving" section), not (a)
        // spec-invalid.
        assert_eq!(parse("full-width", "text-transform"), None);
        assert_eq!(parse("full-size-kana", "text-transform"), None);
    }

    #[test]
    fn text_transform_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "text-transform"), None);
    }

    #[test]
    fn text_transform_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "text-transform"), None);
        }
    }

    #[test]
    fn text_transform_rejects_non_ident() {
        assert_eq!(parse("16px", "text-transform"), None);
        assert_eq!(parse(r#""uppercase""#, "text-transform"), None);
    }

    #[test]
    fn text_transform_key_maps_to_text_transform_property_key() {
        let v = PropertyValue::TextTransform(TextTransform::None);
        assert_eq!(v.key(), PropertyKey::TextTransform);
        let v = PropertyValue::TextTransform(TextTransform::Capitalize);
        assert_eq!(v.key(), PropertyKey::TextTransform);
    }

    // ── visibility (CSS Display 3 §4) ──
    //
    // Value grammar (spec verbatim): `visible | hidden | collapse`. This
    // crate implements all 3 keywords (`Visibility` doc's "Scope carving"
    // section — `collapse`'s formatting-context-specific space-saving effect
    // is unimplemented, but the keyword itself is fully accepted). Initial:
    // visible / Inherited: yes / Computed value: as specified.

    #[test]
    fn visibility_parse_all_keywords() {
        assert_eq!(
            parse("visible", "visibility"),
            Some(PropertyValue::Visibility(Visibility::Visible))
        );
        assert_eq!(
            parse("hidden", "visibility"),
            Some(PropertyValue::Visibility(Visibility::Hidden))
        );
        assert_eq!(
            parse("collapse", "visibility"),
            Some(PropertyValue::Visibility(Visibility::Collapse))
        );
    }

    #[test]
    fn visibility_is_case_insensitive() {
        assert_eq!(
            parse("VISIBLE", "visibility"),
            Some(PropertyValue::Visibility(Visibility::Visible))
        );
        assert_eq!(
            parse("Hidden", "visibility"),
            Some(PropertyValue::Visibility(Visibility::Hidden))
        );
        assert_eq!(
            parse("Collapse", "visibility"),
            Some(PropertyValue::Visibility(Visibility::Collapse))
        );
    }

    #[test]
    fn visibility_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "visibility"), None);
    }

    #[test]
    fn visibility_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "visibility"), None);
        }
    }

    #[test]
    fn visibility_rejects_non_ident() {
        assert_eq!(parse("16px", "visibility"), None);
        assert_eq!(parse(r#""hidden""#, "visibility"), None);
    }

    #[test]
    fn visibility_key_maps_to_visibility_property_key() {
        let v = PropertyValue::Visibility(Visibility::Visible);
        assert_eq!(v.key(), PropertyKey::Visibility);
        let v = PropertyValue::Visibility(Visibility::Hidden);
        assert_eq!(v.key(), PropertyKey::Visibility);
        let v = PropertyValue::Visibility(Visibility::Collapse);
        assert_eq!(v.key(), PropertyKey::Visibility);
    }

    #[test]
    fn z_index_key_maps_to_z_index_property_key() {
        let v = PropertyValue::ZIndex(ZIndexValue::Auto);
        assert_eq!(v.key(), PropertyKey::ZIndex);
        let v = PropertyValue::ZIndex(ZIndexValue::Integer(-1));
        assert_eq!(v.key(), PropertyKey::ZIndex);
    }

    // ── float (CSS2 §9.5.1) ──
    //
    // Value grammar (§9.5.1 spec verbatim): `left | right | none | inherit`.
    // Initial: none / Inherited: no / Computed value: as specified.

    #[test]
    fn float_parse_all_keywords() {
        assert_eq!(
            parse("none", "float"),
            Some(PropertyValue::Float(FloatValue::None))
        );
        assert_eq!(
            parse("left", "float"),
            Some(PropertyValue::Float(FloatValue::Left))
        );
        assert_eq!(
            parse("right", "float"),
            Some(PropertyValue::Float(FloatValue::Right))
        );
    }

    #[test]
    fn float_is_case_insensitive() {
        assert_eq!(
            parse("NONE", "float"),
            Some(PropertyValue::Float(FloatValue::None))
        );
        assert_eq!(
            parse("Left", "float"),
            Some(PropertyValue::Float(FloatValue::Left))
        );
        assert_eq!(
            parse("RIGHT", "float"),
            Some(PropertyValue::Float(FloatValue::Right))
        );
    }

    #[test]
    fn float_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "float"), None);
        // `clear`'s `both` keyword is not valid on `float`.
        assert_eq!(parse("both", "float"), None);
    }

    #[test]
    fn float_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "float"), None);
        }
    }

    #[test]
    fn float_rejects_non_ident() {
        assert_eq!(parse("16px", "float"), None);
        assert_eq!(parse(r#""left""#, "float"), None);
    }

    #[test]
    fn float_key_maps_to_float_property_key() {
        let v = PropertyValue::Float(FloatValue::None);
        assert_eq!(v.key(), PropertyKey::Float);
        let v = PropertyValue::Float(FloatValue::Left);
        assert_eq!(v.key(), PropertyKey::Float);
        let v = PropertyValue::Float(FloatValue::Right);
        assert_eq!(v.key(), PropertyKey::Float);
    }

    // ── clear (CSS2 §9.5.2) ──
    //
    // Value grammar (§9.5.2 spec verbatim): `none | left | right | both |
    // inherit`. Initial: none / Inherited: no / Computed value: as
    // specified.

    #[test]
    fn clear_parse_all_keywords() {
        assert_eq!(
            parse("none", "clear"),
            Some(PropertyValue::Clear(ClearValue::None))
        );
        assert_eq!(
            parse("left", "clear"),
            Some(PropertyValue::Clear(ClearValue::Left))
        );
        assert_eq!(
            parse("right", "clear"),
            Some(PropertyValue::Clear(ClearValue::Right))
        );
        assert_eq!(
            parse("both", "clear"),
            Some(PropertyValue::Clear(ClearValue::Both))
        );
    }

    #[test]
    fn clear_is_case_insensitive() {
        assert_eq!(
            parse("NONE", "clear"),
            Some(PropertyValue::Clear(ClearValue::None))
        );
        assert_eq!(
            parse("Both", "clear"),
            Some(PropertyValue::Clear(ClearValue::Both))
        );
    }

    #[test]
    fn clear_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "clear"), None);
    }

    #[test]
    fn clear_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "clear"), None);
        }
    }

    #[test]
    fn clear_rejects_non_ident() {
        assert_eq!(parse("16px", "clear"), None);
        assert_eq!(parse(r#""left""#, "clear"), None);
    }

    #[test]
    fn clear_key_maps_to_clear_property_key() {
        let v = PropertyValue::Clear(ClearValue::None);
        assert_eq!(v.key(), PropertyKey::Clear);
        let v = PropertyValue::Clear(ClearValue::Both);
        assert_eq!(v.key(), PropertyKey::Clear);
    }

    // ── resolve_display_for_float (CSS2 §9.7) ──

    #[test]
    fn resolve_display_for_float_is_noop_when_float_is_none() {
        for display in [
            DisplayValue::Block,
            DisplayValue::Inline,
            DisplayValue::InlineBlock,
            DisplayValue::None,
            DisplayValue::Flex,
            DisplayValue::Grid,
        ] {
            assert_eq!(
                resolve_display_for_float(display, FloatValue::None),
                display
            );
        }
    }

    #[test]
    fn resolve_display_for_float_forces_inline_and_inline_block_to_block() {
        // §9.7 table: `inline` / `inline-block` → `block`.
        for float in [FloatValue::Left, FloatValue::Right] {
            assert_eq!(
                resolve_display_for_float(DisplayValue::Inline, float),
                DisplayValue::Block
            );
            assert_eq!(
                resolve_display_for_float(DisplayValue::InlineBlock, float),
                DisplayValue::Block
            );
        }
    }

    #[test]
    fn resolve_display_for_float_leaves_block_flex_grid_unchanged() {
        // §9.7 table's "others" row — not in the forced-to-block list.
        for float in [FloatValue::Left, FloatValue::Right] {
            assert_eq!(
                resolve_display_for_float(DisplayValue::Block, float),
                DisplayValue::Block
            );
            assert_eq!(
                resolve_display_for_float(DisplayValue::Flex, float),
                DisplayValue::Flex
            );
            assert_eq!(
                resolve_display_for_float(DisplayValue::Grid, float),
                DisplayValue::Grid
            );
        }
    }

    #[test]
    fn resolve_display_for_float_leaves_none_as_none() {
        // §9.7 leading clause: "If 'display' has the value 'none', then
        // 'position' and 'float' do not apply" — this precedes the table,
        // so `none` must not be affected even when `float` is not `none`.
        for float in [FloatValue::Left, FloatValue::Right] {
            assert_eq!(
                resolve_display_for_float(DisplayValue::None, float),
                DisplayValue::None
            );
        }
    }

    // ── word-break (CSS Text 3 §5.1) ──
    //
    // Value grammar (§5.1 spec verbatim, full property grammar): `normal |
    // keep-all | break-all | break-word`. This crate implements only
    // normal / keep-all / break-all (`WordBreak` doc's "Scope carving"
    // section) — the 4th, deprecated `break-word` keyword is excluded.
    // Initial: normal / Inherited: yes / Computed value: specified keyword.

    #[test]
    fn word_break_parse_all_implemented_keywords() {
        assert_eq!(
            parse("normal", "word-break"),
            Some(PropertyValue::WordBreak(WordBreak::Normal))
        );
        assert_eq!(
            parse("keep-all", "word-break"),
            Some(PropertyValue::WordBreak(WordBreak::KeepAll))
        );
        assert_eq!(
            parse("break-all", "word-break"),
            Some(PropertyValue::WordBreak(WordBreak::BreakAll))
        );
    }

    #[test]
    fn word_break_is_case_insensitive() {
        assert_eq!(
            parse("NORMAL", "word-break"),
            Some(PropertyValue::WordBreak(WordBreak::Normal))
        );
        assert_eq!(
            parse("Keep-All", "word-break"),
            Some(PropertyValue::WordBreak(WordBreak::KeepAll))
        );
        assert_eq!(
            parse("BREAK-ALL", "word-break"),
            Some(PropertyValue::WordBreak(WordBreak::BreakAll))
        );
    }

    #[test]
    fn word_break_rejects_deprecated_break_word() {
        // (b) not supported — the deprecated `break-word` value on
        // `word-break` itself is spec-valid but unimplemented (`WordBreak`
        // doc's "Scope carving" section — its compound `normal` +
        // `overflow-wrap: anywhere` cross-property semantics have no slot
        // in this crate's per-property model), not (a) spec-invalid.
        assert_eq!(parse("break-word", "word-break"), None);
    }

    #[test]
    fn word_break_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "word-break"), None);
    }

    #[test]
    fn word_break_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "word-break"), None);
        }
    }

    #[test]
    fn word_break_rejects_non_ident() {
        assert_eq!(parse("16px", "word-break"), None);
        assert_eq!(parse(r#""normal""#, "word-break"), None);
    }

    #[test]
    fn word_break_key_maps_to_word_break_property_key() {
        let v = PropertyValue::WordBreak(WordBreak::Normal);
        assert_eq!(v.key(), PropertyKey::WordBreak);
        let v = PropertyValue::WordBreak(WordBreak::KeepAll);
        assert_eq!(v.key(), PropertyKey::WordBreak);
        let v = PropertyValue::WordBreak(WordBreak::BreakAll);
        assert_eq!(v.key(), PropertyKey::WordBreak);
    }

    // ── overflow-wrap / word-wrap legacy alias (CSS Text 3 §5.4) ──
    //
    // Value grammar (§5.4 spec verbatim): `normal | break-word | anywhere`.
    // All 3 keywords are implemented — unlike `word-break`'s deprecated
    // `break-word`, `overflow-wrap: break-word` is not deprecated
    // (`OverflowWrap` doc). `word-wrap` is the spec-mandated legacy name
    // alias and must parse identically. Initial: normal / Inherited: yes /
    // Computed value: specified keyword.

    #[test]
    fn overflow_wrap_parse_all_keywords() {
        assert_eq!(
            parse("normal", "overflow-wrap"),
            Some(PropertyValue::OverflowWrap(OverflowWrap::Normal))
        );
        assert_eq!(
            parse("break-word", "overflow-wrap"),
            Some(PropertyValue::OverflowWrap(OverflowWrap::BreakWord))
        );
        assert_eq!(
            parse("anywhere", "overflow-wrap"),
            Some(PropertyValue::OverflowWrap(OverflowWrap::Anywhere))
        );
    }

    #[test]
    fn overflow_wrap_is_case_insensitive() {
        assert_eq!(
            parse("NORMAL", "overflow-wrap"),
            Some(PropertyValue::OverflowWrap(OverflowWrap::Normal))
        );
        assert_eq!(
            parse("Break-Word", "overflow-wrap"),
            Some(PropertyValue::OverflowWrap(OverflowWrap::BreakWord))
        );
        assert_eq!(
            parse("ANYWHERE", "overflow-wrap"),
            Some(PropertyValue::OverflowWrap(OverflowWrap::Anywhere))
        );
    }

    #[test]
    fn word_wrap_legacy_alias_parses_identically_to_overflow_wrap() {
        // CSS Text 3 §5.4 verbatim: "For legacy reasons, UAs must treat
        // word-wrap as a legacy name alias of the overflow-wrap property."
        for (kw, expected) in [
            ("normal", OverflowWrap::Normal),
            ("break-word", OverflowWrap::BreakWord),
            ("anywhere", OverflowWrap::Anywhere),
        ] {
            assert_eq!(
                parse(kw, "word-wrap"),
                Some(PropertyValue::OverflowWrap(expected))
            );
            assert_eq!(parse(kw, "word-wrap"), parse(kw, "overflow-wrap"));
        }
    }

    #[test]
    fn overflow_wrap_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "overflow-wrap"), None);
        assert_eq!(parse("bogus", "word-wrap"), None);
    }

    #[test]
    fn overflow_wrap_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "overflow-wrap"), None);
            assert_eq!(parse(kw, "word-wrap"), None);
        }
    }

    #[test]
    fn overflow_wrap_rejects_non_ident() {
        assert_eq!(parse("16px", "overflow-wrap"), None);
        assert_eq!(parse(r#""normal""#, "overflow-wrap"), None);
    }

    #[test]
    fn overflow_wrap_key_maps_to_overflow_wrap_property_key() {
        // `word-wrap` and `overflow-wrap` share one `PropertyKey` — the
        // legacy alias cascades as one property, not two independently
        // winning ones (`OverflowWrap` doc's "legacy alias" section).
        let v = PropertyValue::OverflowWrap(OverflowWrap::Normal);
        assert_eq!(v.key(), PropertyKey::OverflowWrap);
        let v = PropertyValue::OverflowWrap(OverflowWrap::BreakWord);
        assert_eq!(v.key(), PropertyKey::OverflowWrap);
        let v = PropertyValue::OverflowWrap(OverflowWrap::Anywhere);
        assert_eq!(v.key(), PropertyKey::OverflowWrap);
    }

    // ── break-before / break-after (CSS Fragmentation Module Level 3
    // §3.1) + page-break-before / page-break-after legacy shorthand (§3.4) ──
    //
    // Value grammar (this crate's scope, `BreakBetween` doc's "Scope
    // carving" section): `auto | avoid | avoid-page | page`. Initial:
    // `auto` / Inherited: no / Computed value: specified keyword.

    #[test]
    fn break_before_after_parse_all_implemented_keywords() {
        assert_eq!(
            parse("auto", "break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::Auto))
        );
        assert_eq!(
            parse("auto", "break-after"),
            Some(PropertyValue::BreakAfter(BreakBetween::Auto))
        );
        assert_eq!(
            parse("avoid", "break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::Avoid))
        );
        assert_eq!(
            parse("avoid-page", "break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::AvoidPage))
        );
        assert_eq!(
            parse("page", "break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::Page))
        );
        assert_eq!(
            parse("avoid", "break-after"),
            Some(PropertyValue::BreakAfter(BreakBetween::Avoid))
        );
        assert_eq!(
            parse("avoid-page", "break-after"),
            Some(PropertyValue::BreakAfter(BreakBetween::AvoidPage))
        );
        assert_eq!(
            parse("page", "break-after"),
            Some(PropertyValue::BreakAfter(BreakBetween::Page))
        );
    }

    #[test]
    fn break_before_is_case_insensitive() {
        assert_eq!(
            parse("AUTO", "break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::Auto))
        );
        assert_eq!(
            parse("Avoid-Page", "break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::AvoidPage))
        );
        assert_eq!(
            parse("PAGE", "break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::Page))
        );
    }

    #[test]
    fn break_before_after_rejects_out_of_scope_column_and_region_values() {
        // (b) not supported — this crate has no multi-column or CSS
        // Regions fragmentation context (`BreakBetween` doc's "Scope
        // carving" section), not (a) spec-invalid.
        for kw in ["avoid-column", "column", "avoid-region", "region"] {
            assert_eq!(parse(kw, "break-before"), None);
            assert_eq!(parse(kw, "break-after"), None);
        }
    }

    #[test]
    fn break_before_after_rejects_out_of_scope_page_spread_values() {
        // (b) not supported — no page-spread concept in this crate
        // (`BreakBetween` doc's "Scope carving" section).
        for kw in ["left", "right", "recto", "verso"] {
            assert_eq!(parse(kw, "break-before"), None);
            assert_eq!(parse(kw, "break-after"), None);
        }
    }

    #[test]
    fn break_before_after_rejects_always_and_all() {
        // `always`/`all` are not part of the current break-before/
        // break-after grammar at all (`BreakBetween` doc's "Scope carving"
        // section — Level 3's change log only names `always`, not `all`;
        // both live in Level 4 instead, which this crate does not target).
        // `always` is valid only as the `page-break-before`/
        // `page-break-after` legacy shorthand's own keyword, never
        // directly on `break-before`/`break-after`; `all` has no path in
        // at all.
        for kw in ["always", "all"] {
            assert_eq!(parse(kw, "break-before"), None);
            assert_eq!(parse(kw, "break-after"), None);
        }
    }

    #[test]
    fn break_before_after_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "break-before"), None);
        assert_eq!(parse("bogus", "break-after"), None);
    }

    #[test]
    fn break_before_after_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "break-before"), None);
            assert_eq!(parse(kw, "break-after"), None);
        }
    }

    #[test]
    fn break_before_after_rejects_non_ident() {
        assert_eq!(parse("16px", "break-before"), None);
        assert_eq!(parse(r#""auto""#, "break-after"), None);
    }

    #[test]
    fn break_before_after_key_maps_to_distinct_property_keys() {
        // `break-before` / `break-after` are 2 independent cascade winners
        // (unlike the `word-wrap`/`overflow-wrap` name alias, which shares
        // one `PropertyKey` — `BreakBetween` doc's "legacy shorthand"
        // section explains why this pair does too, just each with its
        // *own* longhand).
        let v = PropertyValue::BreakBefore(BreakBetween::Page);
        assert_eq!(v.key(), PropertyKey::BreakBefore);
        let v = PropertyValue::BreakAfter(BreakBetween::Page);
        assert_eq!(v.key(), PropertyKey::BreakAfter);
    }

    // ── page-break-before / page-break-after legacy shorthand value remap
    // (CSS Fragmentation Module Level 3 §3.4) ──

    #[test]
    fn page_break_before_after_legacy_shorthand_identity_values() {
        for prop in ["page-break-before", "page-break-after"] {
            let wrap = |v| {
                if prop == "page-break-before" {
                    PropertyValue::BreakBefore(v)
                } else {
                    PropertyValue::BreakAfter(v)
                }
            };
            assert_eq!(parse("auto", prop), Some(wrap(BreakBetween::Auto)));
            assert_eq!(parse("avoid", prop), Some(wrap(BreakBetween::Avoid)));
        }
    }

    #[test]
    fn page_break_before_after_legacy_shorthand_remaps_always_to_page() {
        // CSS Fragmentation Module Level 3 §3.4 mapping table verbatim:
        // `always` (page-break-*) -> `page` (break-*). Non-identity remap —
        // pin the exact equality with the longhand spelling, mirroring
        // `word_wrap_legacy_alias_parses_identically_to_overflow_wrap`'s
        // shape (there the two spellings are identical; here they are not,
        // which is exactly what this test must catch).
        assert_eq!(
            parse("always", "page-break-before"),
            Some(PropertyValue::BreakBefore(BreakBetween::Page))
        );
        assert_eq!(
            parse("always", "page-break-before"),
            parse("page", "break-before")
        );
        assert_eq!(
            parse("always", "page-break-after"),
            Some(PropertyValue::BreakAfter(BreakBetween::Page))
        );
        assert_eq!(
            parse("always", "page-break-after"),
            parse("page", "break-after")
        );
    }

    #[test]
    fn page_break_before_after_legacy_shorthand_rejects_new_property_only_values() {
        // `avoid-page` / `page` are valid on `break-before`/`break-after`
        // directly, but CSS2.1's own `page-break-before`/`page-break-after`
        // propdef grammar (`auto | always | avoid | left | right`) does not
        // have them — the legacy shorthand's grammar is CSS2.1's, not the
        // new property's (`BreakBetween` doc's "legacy shorthand" section).
        for kw in ["avoid-page", "page"] {
            assert_eq!(parse(kw, "page-break-before"), None);
            assert_eq!(parse(kw, "page-break-after"), None);
        }
    }

    #[test]
    fn page_break_before_after_legacy_shorthand_rejects_left_and_right() {
        // CSS2.1's own grammar has `left`/`right`, but they remap to
        // `break-before`/`break-after` values this crate does not
        // implement (`BreakBetween` doc's "Scope carving" section) — the
        // scope carve applies transitively through the legacy shorthand.
        for kw in ["left", "right"] {
            assert_eq!(parse(kw, "page-break-before"), None);
            assert_eq!(parse(kw, "page-break-after"), None);
        }
    }

    #[test]
    fn page_break_before_after_legacy_shorthand_key_maps_to_same_key_as_longhand() {
        // One cascade winner per property, whether declared via the new
        // name or the legacy shorthand name (`BreakBetween` doc's "legacy
        // shorthand" section — no `expand_shorthand_into` arm needed since
        // this is a 1:1, not a fan-out, shorthand).
        let v = parse("always", "page-break-before").unwrap();
        assert_eq!(v.key(), PropertyKey::BreakBefore);
        let v = parse("always", "page-break-after").unwrap();
        assert_eq!(v.key(), PropertyKey::BreakAfter);
    }

    // ── break-inside (CSS Fragmentation Module Level 3 §3.2) +
    // page-break-inside legacy shorthand (§3.4) ──
    //
    // Value grammar (this crate's scope, `BreakInside` doc's "Scope
    // carving" section): `auto | avoid | avoid-page` — a smaller, disjoint
    // set from `break-before`/`break-after`'s `BreakBetween` (no `page`).
    // Initial: `auto` / Inherited: no / Computed value: specified keyword.

    #[test]
    fn break_inside_parse_all_implemented_keywords() {
        assert_eq!(
            parse("auto", "break-inside"),
            Some(PropertyValue::BreakInside(BreakInside::Auto))
        );
        assert_eq!(
            parse("avoid", "break-inside"),
            Some(PropertyValue::BreakInside(BreakInside::Avoid))
        );
        assert_eq!(
            parse("avoid-page", "break-inside"),
            Some(PropertyValue::BreakInside(BreakInside::AvoidPage))
        );
    }

    #[test]
    fn break_inside_is_case_insensitive() {
        assert_eq!(
            parse("AUTO", "break-inside"),
            Some(PropertyValue::BreakInside(BreakInside::Auto))
        );
        assert_eq!(
            parse("Avoid-Page", "break-inside"),
            Some(PropertyValue::BreakInside(BreakInside::AvoidPage))
        );
    }

    #[test]
    fn break_inside_rejects_forced_break_values() {
        // `break-inside` has no forced-break values at all — `page` /
        // `column` / `region` are valid on `break-before`/`break-after`
        // (or would be, absent this crate's scope carve) but have no
        // `break-inside` counterpart in the spec grammar at all, not even
        // an excluded one (`BreakInside` doc: "breaking within has no
        // start/end edge to force a break relative to").
        for kw in ["page", "column", "region"] {
            assert_eq!(parse(kw, "break-inside"), None);
        }
    }

    #[test]
    fn break_inside_rejects_out_of_scope_avoid_values() {
        // Unlike `page`/`column`/`region` above, `avoid-column` and
        // `avoid-region` *are* in `break-inside`'s own propdef grammar
        // (`auto | avoid | avoid-page | avoid-column | avoid-region`,
        // `BreakInside` doc) — they are rejected here purely by this
        // crate's scope carve (no multi-column / CSS Regions fragmentation
        // context), not because the spec lacks them.
        for kw in ["avoid-column", "avoid-region"] {
            assert_eq!(parse(kw, "break-inside"), None);
        }
    }

    #[test]
    fn break_inside_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "break-inside"), None);
    }

    #[test]
    fn break_inside_rejects_css_wide_keyword() {
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "break-inside"), None);
        }
    }

    #[test]
    fn break_inside_rejects_non_ident() {
        assert_eq!(parse("16px", "break-inside"), None);
        assert_eq!(parse(r#""auto""#, "break-inside"), None);
    }

    #[test]
    fn break_inside_key_maps_to_break_inside_property_key() {
        let v = PropertyValue::BreakInside(BreakInside::AvoidPage);
        assert_eq!(v.key(), PropertyKey::BreakInside);
    }

    #[test]
    fn page_break_inside_legacy_shorthand_identity_values() {
        assert_eq!(
            parse("auto", "page-break-inside"),
            Some(PropertyValue::BreakInside(BreakInside::Auto))
        );
        assert_eq!(
            parse("avoid", "page-break-inside"),
            Some(PropertyValue::BreakInside(BreakInside::Avoid))
        );
        assert_eq!(
            parse("auto", "page-break-inside"),
            parse("auto", "break-inside")
        );
        assert_eq!(
            parse("avoid", "page-break-inside"),
            parse("avoid", "break-inside")
        );
    }

    #[test]
    fn page_break_inside_legacy_shorthand_rejects_new_property_only_value() {
        // `avoid-page` is valid on `break-inside` directly, but CSS2.1's
        // own `page-break-inside` propdef grammar is just `auto | avoid` —
        // the legacy shorthand's grammar is CSS2.1's, not the new
        // property's fuller one (`BreakInside` doc's "legacy shorthand"
        // section).
        assert_eq!(parse("avoid-page", "page-break-inside"), None);
    }

    #[test]
    fn page_break_inside_legacy_shorthand_key_maps_to_same_key_as_longhand() {
        let v = parse("avoid", "page-break-inside").unwrap();
        assert_eq!(v.key(), PropertyKey::BreakInside);
    }

    // ── letter-spacing / word-spacing (CSS Text 3 §7.2 / §7.1) ──
    //
    // Value grammar (spec verbatim, identical for both): `normal | <length>`.
    // Initial: `normal`. Inherited: yes. Percentages: N/A. Both share
    // `parse_letter_or_word_spacing` — see that function's doc for the
    // ordering / non-negative / percentage-rejection rationale.

    #[test]
    fn letter_spacing_parse_normal_keyword() {
        assert_eq!(
            parse("normal", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Normal))
        );
    }

    #[test]
    fn word_spacing_parse_normal_keyword() {
        assert_eq!(
            parse("normal", "word-spacing"),
            Some(PropertyValue::WordSpacing(LengthOrNormal::Normal))
        );
    }

    #[test]
    fn letter_spacing_is_case_insensitive_normal() {
        assert_eq!(
            parse("NORMAL", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Normal))
        );
        assert_eq!(
            parse("Normal", "word-spacing"),
            Some(PropertyValue::WordSpacing(LengthOrNormal::Normal))
        );
    }

    #[test]
    fn letter_spacing_parse_length_px() {
        assert_eq!(
            parse("2px", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
                Length::Px(2.0)
            )))
        );
    }

    #[test]
    fn word_spacing_parse_length_px() {
        assert_eq!(
            parse("4px", "word-spacing"),
            Some(PropertyValue::WordSpacing(LengthOrNormal::Length(
                Length::Px(4.0)
            )))
        );
    }

    #[test]
    fn letter_spacing_accepts_length_em_rem_pt() {
        assert_eq!(
            parse("0.1em", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
                Length::Em(0.1)
            )))
        );
        assert_eq!(
            parse("1rem", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
                Length::Rem(1.0)
            )))
        );
        assert_eq!(
            parse("2pt", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
                Length::Pt(2.0)
            )))
        );
    }

    // Spec verbatim (both §7.1 and §7.2): "Values may be negative, but there
    // may be implementation-dependent limits." — unlike `line-height` /
    // `font-size` / `padding` etc., this property does NOT reject negative
    // lengths at parse time (`parse_letter_or_word_spacing` doc's "Negative
    // length は許容" section).
    #[test]
    fn letter_spacing_accepts_negative_length() {
        assert_eq!(
            parse("-2px", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
                Length::Px(-2.0)
            )))
        );
        assert_eq!(
            parse("-0.05em", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
                Length::Em(-0.05)
            )))
        );
    }

    #[test]
    fn word_spacing_accepts_negative_length() {
        assert_eq!(
            parse("-1px", "word-spacing"),
            Some(PropertyValue::WordSpacing(LengthOrNormal::Length(
                Length::Px(-1.0)
            )))
        );
    }

    // Bare `0` has no `<number>` grammar alternative to disambiguate against
    // here (unlike `line-height`) — it goes through `parse_length_value`'s
    // unitless-zero clause and becomes `Length::Px(0.0)`, not `Normal`.
    #[test]
    fn letter_spacing_unitless_zero_is_length_not_normal() {
        assert_eq!(
            parse("0", "letter-spacing"),
            Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
                Length::Px(0.0)
            )))
        );
    }

    // Spec verbatim (both §7.1 and §7.2): "Percentages: N/A" / "Percentages:
    // n/a" — percentage is rejected at parse time, unlike `line-height`'s
    // `<length-percentage>`.
    #[test]
    fn letter_spacing_rejects_percentage() {
        assert_eq!(parse("5%", "letter-spacing"), None);
    }

    #[test]
    fn word_spacing_rejects_percentage() {
        assert_eq!(parse("5%", "word-spacing"), None);
    }

    #[test]
    fn letter_spacing_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "letter-spacing"), None);
        assert_eq!(parse("auto", "letter-spacing"), None);
    }

    #[test]
    fn word_spacing_rejects_unknown_keyword() {
        assert_eq!(parse("bogus", "word-spacing"), None);
    }

    #[test]
    fn letter_spacing_rejects_css_wide_keyword() {
        // (b) not supported — CSS-wide keyword is unimplemented (future work),
        // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "letter-spacing"), None);
        }
    }

    #[test]
    fn word_spacing_rejects_css_wide_keyword() {
        for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert_eq!(parse(kw, "word-spacing"), None);
        }
    }

    #[test]
    fn letter_spacing_rejects_non_length_non_ident() {
        assert_eq!(parse(r#""2px""#, "letter-spacing"), None);
    }

    #[test]
    fn letter_spacing_key_maps_to_letter_spacing_property_key() {
        let v = PropertyValue::LetterSpacing(LengthOrNormal::Normal);
        assert_eq!(v.key(), PropertyKey::LetterSpacing);
        let v = PropertyValue::LetterSpacing(LengthOrNormal::Length(Length::Px(2.0)));
        assert_eq!(v.key(), PropertyKey::LetterSpacing);
    }

    #[test]
    fn word_spacing_key_maps_to_word_spacing_property_key() {
        let v = PropertyValue::WordSpacing(LengthOrNormal::Normal);
        assert_eq!(v.key(), PropertyKey::WordSpacing);
        let v = PropertyValue::WordSpacing(LengthOrNormal::Length(Length::Px(2.0)));
        assert_eq!(v.key(), PropertyKey::WordSpacing);
    }

    // ── resolve_overflow (CSS Overflow 3 §3.1 cross-axis computed-value
    // coupling) ──
    //
    // Spec verbatim: "The visible/clip values of overflow compute to
    // auto/hidden (respectively) if one of overflow-x or overflow-y is
    // neither visible nor clip."

    #[test]
    fn resolve_overflow_both_visible_is_unaffected() {
        let pair = OverflowXY::both(OverflowValue::Visible);
        assert_eq!(resolve_overflow(pair), pair);
    }

    #[test]
    fn resolve_overflow_visible_x_computes_to_auto_when_y_is_hidden() {
        let pair = OverflowXY {
            x: OverflowValue::Visible,
            y: OverflowValue::Hidden,
        };
        assert_eq!(
            resolve_overflow(pair),
            OverflowXY {
                x: OverflowValue::Auto,
                y: OverflowValue::Hidden,
            }
        );
    }

    #[test]
    fn resolve_overflow_clip_x_computes_to_hidden_when_y_is_scroll() {
        let pair = OverflowXY {
            x: OverflowValue::Clip,
            y: OverflowValue::Scroll,
        };
        assert_eq!(
            resolve_overflow(pair),
            OverflowXY {
                x: OverflowValue::Hidden,
                y: OverflowValue::Scroll,
            }
        );
    }

    #[test]
    fn resolve_overflow_visible_and_clip_do_not_gate_each_other() {
        // The gate condition is "the *other* axis is neither visible nor
        // clip" — `visible` and `clip` are each themselves one of the two
        // values the gate exempts, so pairing them together never satisfies
        // the condition for either axis. Both stay as specified.
        let pair = OverflowXY {
            x: OverflowValue::Visible,
            y: OverflowValue::Clip,
        };
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            resolve_overflow(pair),
            pair,
            "visible/clip do not gate each other"
        );
    }

    #[test]
    fn resolve_overflow_non_visible_non_clip_values_pass_through_unchanged() {
        // `hidden`/`scroll`/`auto` are not rewritten by the coupling
        // regardless of the other axis's value (the rule only ever rewrites
        // `visible`/`clip`).
        for this in [
            OverflowValue::Hidden,
            OverflowValue::Scroll,
            OverflowValue::Auto,
        ] {
            for other in [
                OverflowValue::Visible,
                OverflowValue::Hidden,
                OverflowValue::Clip,
                OverflowValue::Scroll,
                OverflowValue::Auto,
            ] {
                let pair = OverflowXY { x: this, y: other };
                assert_eq!(resolve_overflow(pair).x, this);
            }
        }
    }

    #[test]
    fn resolve_overflow_both_non_visible_non_clip_is_unaffected() {
        let pair = OverflowXY {
            x: OverflowValue::Scroll,
            y: OverflowValue::Auto,
        };
        assert_eq!(resolve_overflow(pair), pair);
    }

    // ── resolve_text_align_match_parent (CSS Text 3 §6.1
    // `#valdef-text-align-match-parent`) ──
    //
    // This is the shared resolver both `SpecifiedValues::finalize` (element
    // path) and `cascade::resolve_against_inherited` (page path) funnel into
    // — see the function doc for why `apply_value` itself is deliberately
    // *not* a caller (the same-node winner-order hazard between `direction`
    // and `text-align`).

    #[test]
    fn match_parent_resolves_start_against_ltr_parent_to_left() {
        assert_eq!(
            resolve_text_align_match_parent(
                TextAlign::MatchParent,
                TextAlign::Start,
                Direction::Ltr
            ),
            TextAlign::Left
        );
    }

    #[test]
    fn match_parent_resolves_start_against_rtl_parent_to_right() {
        assert_eq!(
            resolve_text_align_match_parent(
                TextAlign::MatchParent,
                TextAlign::Start,
                Direction::Rtl
            ),
            TextAlign::Right
        );
    }

    #[test]
    fn match_parent_resolves_end_against_ltr_parent_to_right() {
        assert_eq!(
            resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::End, Direction::Ltr),
            TextAlign::Right
        );
    }

    #[test]
    fn match_parent_resolves_end_against_rtl_parent_to_left() {
        assert_eq!(
            resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::End, Direction::Rtl),
            TextAlign::Left
        );
    }

    #[test]
    fn match_parent_copies_non_start_end_parent_value_verbatim() {
        // "behaves the same as inherit" for the non-start/end half — direction
        // plays no role.
        for parent in [
            TextAlign::Left,
            TextAlign::Right,
            TextAlign::Center,
            TextAlign::Justify,
            TextAlign::JustifyAll,
        ] {
            assert_eq!(
                resolve_text_align_match_parent(TextAlign::MatchParent, parent, Direction::Ltr),
                parent
            );
            assert_eq!(
                resolve_text_align_match_parent(TextAlign::MatchParent, parent, Direction::Rtl),
                parent
            );
        }
    }

    #[test]
    fn non_match_parent_specified_values_pass_through_unchanged() {
        // Every other keyword's computed value is "as specified" — the parent
        // args must be ignored entirely.
        for specified in [
            TextAlign::Start,
            TextAlign::End,
            TextAlign::Left,
            TextAlign::Right,
            TextAlign::Center,
            TextAlign::Justify,
            TextAlign::JustifyAll,
        ] {
            assert_eq!(
                resolve_text_align_match_parent(specified, TextAlign::Center, Direction::Rtl),
                specified
            );
        }
    }

    // ── margin longhand + shorthand (CSS Box 3 §3.1/§3.2) ──
    //
    // Primary source:
    // - #margin-physical (§3.1): `<length-percentage> | auto`, initial 0, non-inherited.
    // - #margin-shorthand (§3.2): `<'margin-top'>{1,4}` with 1/2/3/4 value expansion.

    #[test]
    fn margin_top_parse_px() {
        // Verification 3-a: `margin-top: 10px` → MarginTop(Length(Px(10)))。
        assert_eq!(
            parse("10px", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                10.0
            ))))
        );
    }

    #[test]
    fn margin_right_parse_auto() {
        // Verification 3-b: `margin-right: auto` → MarginRight(Auto)。§3.1 の
        // `auto` alternative の受理を per-side longhand で pin。
        assert_eq!(
            parse("auto", "margin-right"),
            Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
        );
    }

    #[test]
    fn margin_bottom_parse_percentage() {
        // Verification 3-c: `margin-bottom: 50%` → MarginBottom(Length(Percent(50)))。
        assert_eq!(
            parse("50%", "margin-bottom"),
            Some(PropertyValue::MarginBottom(LengthOrAuto::Length(
                Length::Percent(50.0)
            )))
        );
    }

    #[test]
    fn margin_left_parse_em() {
        // Verification 3-d: `margin-left: 2em` → MarginLeft(Length(Em(2)))。
        // `<length-percentage>` mode 経由で em 受理 (parse_length_value の mode
        // arg = true)。
        assert_eq!(
            parse("2em", "margin-left"),
            Some(PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Em(
                2.0
            ))))
        );
    }

    #[test]
    fn margin_side_accepts_negative_length() {
        // Task Non-goals: negative margin は spec-valid (§3.1 "Negative values
        // for margin properties are allowed")。longhand も含めて受理を pin。
        assert_eq!(
            parse("-10px", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                -10.0
            ))))
        );
    }

    #[test]
    fn margin_top_accepts_zero() {
        // spec `<length-percentage> | auto` — 0 は valid length。`0px` は Dimension arm、
        // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す。
        // margin は non-negative filter を持たないため素通り。
        assert_eq!(
            parse("0px", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                0.0
            ))))
        );
        assert_eq!(
            parse("0", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                0.0
            ))))
        );
    }

    #[test]
    fn margin_side_case_insensitive_auto() {
        // CSS spec: ident keyword は ASCII case-insensitive。`AUTO` 受理を pin
        // (expect_ident_matching が case-insensitive の証拠、helper 変更で
        // regression した際の canary)。
        assert_eq!(
            parse("AUTO", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Auto))
        );
    }

    #[test]
    fn margin_side_rejects_unsupported_unit() {
        // `cap` (§6.1.1 font-relative lengths) は現状
        // 未対応 (parse_length_value 側で drop)。`cm` / `lh` / `rlh` は
        // それぞれ受理側へ移った — margin-side helper に非依存で波及ドロップを pin。
        assert_eq!(parse("1cap", "margin-top"), None);
    }

    #[test]
    fn margin_side_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh`。
        assert_eq!(
            parse("1lh", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Lh(
                1.0
            ))))
        );
        assert_eq!(
            parse("2rlh", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Rlh(
                2.0
            ))))
        );
    }

    #[test]
    fn margin_side_accepts_absolute_unit() {
        assert_eq!(
            parse("1cm", "margin-top"),
            Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Cm(
                1.0
            ))))
        );
    }

    #[test]
    fn margin_side_rejects_bogus_ident() {
        // `<length-percentage> | auto` grammar 外 ident は declaration drop。
        assert_eq!(parse("fill-available", "margin-top"), None);
        assert_eq!(parse("initial", "margin-top"), None);
    }

    #[test]
    fn margin_shorthand_one_value_spreads_all_sides() {
        // Verification 4-a (§3.2 "If there is only one component value, it
        // applies to all sides"): `margin: 10px` → 全 4 side = 10px。
        let want = Sides::all(LengthOrAuto::Length(Length::Px(10.0)));
        assert_eq!(parse("10px", "margin"), Some(PropertyValue::Margin(want)));
    }

    #[test]
    fn margin_shorthand_two_values_top_bottom_and_right_left() {
        // Verification 4-b (§3.2 "If there are two values, the top and bottom
        // margins are set to the first value and the right and left margins
        // are set to the second"): top/bottom = 10px, right/left = 20px。
        let want = Sides {
            top: LengthOrAuto::Length(Length::Px(10.0)),
            right: LengthOrAuto::Length(Length::Px(20.0)),
            bottom: LengthOrAuto::Length(Length::Px(10.0)),
            left: LengthOrAuto::Length(Length::Px(20.0)),
        };
        assert_eq!(
            parse("10px 20px", "margin"),
            Some(PropertyValue::Margin(want))
        );
    }

    #[test]
    fn margin_shorthand_three_values_top_horiz_bottom() {
        // Verification 4-c (§3.2 "If there are three values, the top is set to
        // the first value, the left and right are set to the second, and the
        // bottom is set to the third"): top = 10px, right/left = 20px,
        // bottom = 30px。
        let want = Sides {
            top: LengthOrAuto::Length(Length::Px(10.0)),
            right: LengthOrAuto::Length(Length::Px(20.0)),
            bottom: LengthOrAuto::Length(Length::Px(30.0)),
            left: LengthOrAuto::Length(Length::Px(20.0)),
        };
        assert_eq!(
            parse("10px 20px 30px", "margin"),
            Some(PropertyValue::Margin(want))
        );
    }

    #[test]
    fn margin_shorthand_four_values_clockwise() {
        // Verification 4-d (§3.2 "If there are four values they apply to the
        // top, right, bottom, and left, respectively"): clockwise from top。
        let want = Sides {
            top: LengthOrAuto::Length(Length::Px(10.0)),
            right: LengthOrAuto::Length(Length::Px(20.0)),
            bottom: LengthOrAuto::Length(Length::Px(30.0)),
            left: LengthOrAuto::Length(Length::Px(40.0)),
        };
        assert_eq!(
            parse("10px 20px 30px 40px", "margin"),
            Some(PropertyValue::Margin(want))
        );
    }

    #[test]
    fn margin_shorthand_all_auto() {
        // Verification 5-a: `margin: auto` (1 value auto) → 全 4 side = Auto。
        // browser の "block-level centering" 慣用の parse pin。
        let want = Sides::all(LengthOrAuto::Auto);
        assert_eq!(parse("auto", "margin"), Some(PropertyValue::Margin(want)));
    }

    #[test]
    fn margin_shorthand_zero_and_auto_horizontal_center() {
        // Verification 5-b: `margin: 0 auto` (2 value mixed) は block-level
        // horizontal centering の canonical form。top/bottom = 0px, right/left = auto。
        // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths> の
        // unitless-zero clause により bare `0` は Length::Px(0.0) 受理
        // (`parse_length_value_accepts_unitless_zero_only`
        // で pin)。
        let want = Sides {
            top: LengthOrAuto::Length(Length::Px(0.0)),
            right: LengthOrAuto::Auto,
            bottom: LengthOrAuto::Length(Length::Px(0.0)),
            left: LengthOrAuto::Auto,
        };
        assert_eq!(parse("0 auto", "margin"), Some(PropertyValue::Margin(want)));
    }

    #[test]
    fn margin_shorthand_mixed_units() {
        // grammar coverage: 4-value shorthand で unit / auto を全て混在させる。
        // 32n `<length-percentage> | auto` の grammar 網羅を単一 assertion に集約。
        let want = Sides {
            top: LengthOrAuto::Length(Length::Px(10.0)),
            right: LengthOrAuto::Auto,
            bottom: LengthOrAuto::Length(Length::Percent(50.0)),
            left: LengthOrAuto::Length(Length::Em(2.0)),
        };
        assert_eq!(
            parse("10px auto 50% 2em", "margin"),
            Some(PropertyValue::Margin(want))
        );
    }

    #[test]
    fn margin_shorthand_rejects_empty_input() {
        // 空 value: parse_margin_side 1st fail → parse_margin_shorthand `?`
        // 上位伝播で None (declaration drop)。
        assert_eq!(parse("", "margin"), None);
    }

    #[test]
    fn margin_shorthand_rejects_bogus_ident() {
        // 1st value 位置に grammar 外 ident → declaration drop。
        assert_eq!(parse("bogus", "margin"), None);
    }

    #[test]
    fn margin_shorthand_leaves_extra_values_for_caller_exhausted_check() {
        // 5+ value shorthand: 本 helper は 4 value 消費、5th 以降は unconsumed で
        // return。DeclParser::parse_value の expect_exhausted で最終的に
        // declaration drop されるので、rule.rs 側 test
        // (`margin_shorthand_five_values_declaration_dropped`) で end-to-end
        // 挙動を pin する。本 test は parse_value 単体 (caller expect_exhausted
        // 経由なし) では 4 value までは Some が返る shape の pin。
        let want = Sides {
            top: LengthOrAuto::Length(Length::Px(10.0)),
            right: LengthOrAuto::Length(Length::Px(20.0)),
            bottom: LengthOrAuto::Length(Length::Px(30.0)),
            left: LengthOrAuto::Length(Length::Px(40.0)),
        };
        assert_eq!(
            parse("10px 20px 30px 40px 50px", "margin"),
            Some(PropertyValue::Margin(want))
        );
    }

    #[test]
    fn margin_shorthand_case_insensitive_auto_and_units() {
        // shorthand path でも case-insensitive dispatch が生きている pin。
        let want = Sides {
            top: LengthOrAuto::Auto,
            right: LengthOrAuto::Length(Length::Px(20.0)),
            bottom: LengthOrAuto::Auto,
            left: LengthOrAuto::Length(Length::Px(20.0)),
        };
        assert_eq!(
            parse("AUTO 20PX", "margin"),
            Some(PropertyValue::Margin(want))
        );
    }

    #[test]
    fn margin_longhand_keys_map_correctly() {
        // 4 longhand + shorthand variant → 対応 key (cascade winner 選択の
        // discriminant integrity)。sibling `position_key_maps_to_position_property_key`
        // と同 pattern。shorthand `Margin` key も expansion 前の PropertyValue
        // 段で観測可能なため includes。
        let top = PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(1.0)));
        assert_eq!(top.key(), PropertyKey::MarginTop);
        let right = PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(1.0)));
        assert_eq!(right.key(), PropertyKey::MarginRight);
        let bottom = PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(1.0)));
        assert_eq!(bottom.key(), PropertyKey::MarginBottom);
        let left = PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(1.0)));
        assert_eq!(left.key(), PropertyKey::MarginLeft);
        let shorthand = PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Px(1.0))));
        assert_eq!(shorthand.key(), PropertyKey::Margin);
    }

    #[test]
    fn sides_all_spreads_value_to_all_four() {
        // Sides::all helper (37n reused by shorthand 1-value + initial value):
        // 1 value → top/right/bottom/left が全て同値、Clone 経路 (最後の side は
        // move 消費) が正しく動く pin。
        let s = Sides::all(LengthOrAuto::Length(Length::Px(3.5)));
        assert_eq!(s.top, LengthOrAuto::Length(Length::Px(3.5)));
        assert_eq!(s.right, LengthOrAuto::Length(Length::Px(3.5)));
        assert_eq!(s.bottom, LengthOrAuto::Length(Length::Px(3.5)));
        assert_eq!(s.left, LengthOrAuto::Length(Length::Px(3.5)));
    }

    // ── border longhand + shorthand (CSS Backgrounds 3 §3) ──

    #[test]
    fn border_top_width_parse_px() {
        // Verification #1: parse("1px", "border-top-width") =
        // Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))。
        assert_eq!(
            parse("1px", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))
        );
    }

    // ── width (CSS Sizing 3 §3.1.1) ────────────────────────
    //
    // Primary source:
    // https://www.w3.org/TR/css-sizing-3/#preferred-size-properties
    // Value: `auto | <length-percentage [0,∞]> | min-content | max-content |
    //         fit-content(<length-percentage>)`
    // Initial: auto、Inherited: no。
    //
    // 本 task では `auto` + non-negative `<length-percentage>` のみ受理、
    // min-content / max-content / fit-content() は (b) 非対応。

    #[test]
    fn width_parse_auto_keyword() {
        // Verification #1: `width: auto` → Width(Auto)。initial 値と同 shape で
        // grammar 上位優先分岐 (parse_width の try_parse ident branch) が生きて
        // いることを pin。
        assert_eq!(
            parse("auto", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Auto))
        );
    }

    #[test]
    fn border_top_width_parse_medium_keyword() {
        // Verification #2: parse("medium", "border-top-width") =
        // Some(PropertyValue::BorderTopWidth(Length::Px(3.0)))。
        // spec §3.3 規定値 (normative equivalence) の 1/3/5 px mapping。
        assert_eq!(
            parse("medium", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Px(3.0)))
        );
    }

    #[test]
    fn width_parse_length_px() {
        // Verification #2: `width: 100px` → Width(Length(Px(100)))。
        // 従来 `unknown_property_returns_none` canary で `None` だった箇所が
        // 実 variant を返すようになった transition pin (canary はその後
        // `float` を経て `cursor` に移設済み)。
        assert_eq!(
            parse("100px", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(
                100.0
            ))))
        );
    }

    #[test]
    fn border_width_thin_thick_keywords_map_to_1px_5px() {
        // spec §3.3 規定値: thin=1px、thick=5px。
        // 4 side 各 arm の smoke — arm cross-copy regression pin (`top` arm を
        // `right` arm に誤 wire しても本 test で fail する)。
        assert_eq!(
            parse("thin", "border-right-width"),
            Some(PropertyValue::BorderRightWidth(Length::Px(1.0)))
        );
        assert_eq!(
            parse("thick", "border-left-width"),
            Some(PropertyValue::BorderLeftWidth(Length::Px(5.0)))
        );
    }

    #[test]
    fn width_parse_length_percentage() {
        // Verification #3: `width: 50%` → Width(Length(Percent(50)))。
        // parse_length_value(allow_percentage=true) 経由の Percent branch。
        assert_eq!(
            parse("50%", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::Percent(
                50.0
            ))))
        );
    }

    #[test]
    fn border_width_rejects_negative() {
        // Verification #6: parse("-1px", "border-top-width") = None。
        // spec `<line-width>` = `<length [0,∞]>` — 負値は grammar 違反 → drop。
        assert_eq!(parse("-1px", "border-top-width"), None);
        // Em / Rem / Pt も同 constraint (unit-bearing variant 全て)。
        assert_eq!(parse("-1em", "border-bottom-width"), None);
    }

    #[test]
    fn border_top_width_accepts_zero() {
        // spec `<line-width>` = `<length [0,∞]>` — 0 は閉区間下端。`0px` は
        // Dimension arm、bare `0` は CSS Values 3 §5 unitless-zero clause の
        // Number arm を通す。parse_border_width_side の
        // `>= 0.0` 非負 filter を pass。
        assert_eq!(
            parse("0px", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Px(0.0)))
        );
        assert_eq!(
            parse("0", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Px(0.0)))
        );
    }

    #[test]
    fn border_shorthand_accepts_bare_zero_width() {
        // Follow-on coverage: `0 solid`
        // は shorthand の width slot を bare-zero で埋めた canonical form。
        // parse_border_shorthand の width slot が parse_border_width_side_res 経由で
        // parse_length_value Number arm を通して Length::Px(0.0) を取り、
        // style slot は Solid、color slot は省略で spec initial =
        // `BorderColor::CurrentColor` (CSS Backgrounds 3 §3.1)。
        let border = Border {
            width: Length::Px(0.0),
            style: BorderStyle::Solid,
            color: BorderColor::CurrentColor,
        };
        assert_eq!(
            parse("0 solid", "border"),
            Some(PropertyValue::Border(Sides::all(border)))
        );
    }

    #[test]
    fn border_width_rejects_percentage() {
        // `<line-width>` grammar は `<percentage>` を含まない (padding とは
        // 違う点)。`parse_length_value(input, false)` の
        // `<length>` mode で Percentage token 自体が reject される。
        assert_eq!(parse("50%", "border-top-width"), None);
    }

    #[test]
    fn border_width_rejects_unknown_keyword() {
        // spec §3.3 の keyword 集合外は drop (`auto` / `fat` / `bold` etc.)。
        assert_eq!(parse("auto", "border-top-width"), None);
        assert_eq!(parse("fat", "border-top-width"), None);
    }

    #[test]
    fn border_width_rejects_css_wide_keyword() {
        // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
        // canonical: PropertyValue doc「CSS-wide keyword」節。
        assert_eq!(parse("inherit", "border-top-width"), None);
        assert_eq!(parse("initial", "border-top-width"), None);
        assert_eq!(parse("unset", "border-top-width"), None);
        assert_eq!(parse("revert", "border-top-width"), None);
        assert_eq!(parse("revert-layer", "border-top-width"), None);
    }

    #[test]
    fn border_width_accepts_absolute_unit() {
        // `<line-width>` の `<length [0,∞]>` half は `<percentage>` を含まないが
        // 他 absolute unit は含む — 追加した `pc` を
        // border-width 経路 (`allow_percentage=false`) でも pin する。
        assert_eq!(
            parse("1pc", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Pc(1.0)))
        );
    }

    #[test]
    fn border_width_rejects_negative_absolute_unit() {
        // 全 unit-bearing variant の non-negative check pin (cm、新規 absolute unit)。
        assert_eq!(parse("-1cm", "border-top-width"), None);
    }

    #[test]
    fn border_width_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh`。`<line-width>`
        // grammar (`<length [0,∞]> | thin | medium | thick`) has no
        // self-reference concern the way `font-size` / `line-height` do
        // (`Length::Lh` doc), so `border-*-width` accepts them unfiltered.
        assert_eq!(
            parse("2lh", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Lh(2.0)))
        );
        assert_eq!(
            parse("1rlh", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Rlh(1.0)))
        );
    }

    #[test]
    fn border_top_style_parse_solid() {
        // Verification #3: parse("solid", "border-top-style") =
        // Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))。
        assert_eq!(
            parse("solid", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
        );
    }

    #[test]
    fn width_parse_length_em() {
        // font-relative unit 経路 pin — parse_length_value 経由で em を受理。
        assert_eq!(
            parse("2em", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::Em(2.0))))
        );
    }

    #[test]
    fn border_style_all_10_variants_accepted() {
        // spec §3.2 `<line-style>` の 10 alternative 全てを smoke (arm 削り
        // regression 検知)。
        assert_eq!(
            parse("none", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::None))
        );
        assert_eq!(
            parse("hidden", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Hidden))
        );
        assert_eq!(
            parse("dotted", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Dotted))
        );
        assert_eq!(
            parse("dashed", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Dashed))
        );
        assert_eq!(
            parse("double", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Double))
        );
        assert_eq!(
            parse("groove", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Groove))
        );
        assert_eq!(
            parse("ridge", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Ridge))
        );
        assert_eq!(
            parse("inset", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Inset))
        );
        assert_eq!(
            parse("outset", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Outset))
        );
    }

    #[test]
    fn width_rejects_negative_px() {
        // Verification #4: `width: -10px` → None (spec grammar `[0,∞]` violation)。
        // parse_width の post-filter が enforce (padding と同 pattern)。
        assert_eq!(parse("-10px", "width"), None);
    }

    #[test]
    fn width_rejects_negative_percentage() {
        // 全 Length variant OR-pattern check の pin (Percent 分岐)。
        assert_eq!(parse("-50%", "width"), None);
    }

    #[test]
    fn width_rejects_negative_em() {
        // 全 Length variant OR-pattern check の pin (Em 分岐)。
        assert_eq!(parse("-2em", "width"), None);
    }

    #[test]
    fn width_accepts_zero() {
        // spec `[0,∞]` の closed interval — 下端 0 は有効。
        assert_eq!(
            parse("0px", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
        );
        // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>
        // unitless-zero clause 経由: bare `0` も同 Px(0.0)
        // として受理 (width は `<length-percentage [0,∞]>`、helper が Number arm で
        // 拾い parse_width の非負 filter を pass)。
        assert_eq!(
            parse("0", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
        );
    }

    #[test]
    fn border_style_rejects_unknown_keyword() {
        // `<line-style>` grammar 外 (`wavy` は CSS Text Decoration 4 由来、
        // border-style では invalid) は drop。
        assert_eq!(parse("wavy", "border-top-style"), None);
    }

    #[test]
    fn border_style_rejects_css_wide_keyword() {
        // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
        // canonical: PropertyValue doc「CSS-wide keyword」節。
        assert_eq!(parse("inherit", "border-top-style"), None);
        assert_eq!(parse("initial", "border-top-style"), None);
        assert_eq!(parse("unset", "border-top-style"), None);
        assert_eq!(parse("revert", "border-top-style"), None);
        assert_eq!(parse("revert-layer", "border-top-style"), None);
    }

    #[test]
    fn border_style_case_insensitive() {
        // CSS Values 3 §3.1: keyword は ASCII case-insensitive。
        assert_eq!(
            parse("SOLID", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
        );
    }

    #[test]
    fn width_rejects_min_content_keyword() {
        // Verification #5: (b) 非対応 — intrinsic sizing keyword は
        // 未実装、silent drop。auto ident 分岐は expect_ident_matching("auto")
        // で fail → parse_length_value に落ちて Dimension/Percentage arm 外の
        // Ident token として drop。
        assert_eq!(parse("min-content", "width"), None);
    }

    #[test]
    fn width_rejects_max_content_keyword() {
        // 同上、max-content も silent drop。
        assert_eq!(parse("max-content", "width"), None);
    }

    #[test]
    fn width_rejects_fit_content_function() {
        // fit-content(<length-percentage>) は function token — 受理せず drop。
        assert_eq!(parse("fit-content(50%)", "width"), None);
    }

    #[test]
    fn width_rejects_unsupported_unit() {
        // (b) 非対応 — vw / cap 等は spec-valid だが
        // 未対応、parse_length_value 側で drop、None
        // propagate。`ch` / `lh` / `rlh` は
        // それぞれ受理側へ移った
        // (`width_accepts_absolute_unit` / `width_accepts_lh` 参照)。
        assert_eq!(parse("10vw", "width"), None);
        assert_eq!(parse("5cap", "width"), None);
    }

    #[test]
    fn width_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh`。
        assert_eq!(
            parse("5lh", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::Lh(5.0))))
        );
        assert_eq!(
            parse("1rlh", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::Rlh(1.0))))
        );
    }

    #[test]
    fn width_accepts_absolute_unit() {
        // `1in` = 96px 相当 (specified 層は authored unit をそのまま保持、
        // 絶対化は resolve.rs の責務 — pin: `resolve::tests::length_additional_absolute_units_convert_per_spec_table`)。
        assert_eq!(
            parse("1in", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Length(Length::In(1.0))))
        );
    }

    #[test]
    fn width_case_insensitive_auto() {
        // CSS spec: ident keyword は ASCII case-insensitive。`AUTO` 受理を pin
        // (sibling `margin_side_case_insensitive_auto` と同 pattern)。
        assert_eq!(
            parse("AUTO", "width"),
            Some(PropertyValue::Width(LengthOrAuto::Auto))
        );
    }

    #[test]
    fn border_top_color_parse_hex() {
        // border-*-color の hex form は `parse_color` (background-color と同じ
        // helper) が hex/named/rgb(a)/transparent を受理し、`parse_border_color`
        // が `BorderColor::Resolved` で wrap して cascade static side に届く。
        assert_eq!(
            parse("#ff0000", "border-top-color"),
            Some(PropertyValue::BorderTopColor(BorderColor::Resolved(
                CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255
                }
            )))
        );
    }

    #[test]
    fn border_color_named_and_rgb() {
        // 4 side 各 arm の smoke + 3 color form (named / rgb / transparent) を
        // 分散して cross-arm regression 検知 (background-color test の pattern)。
        // `BorderColor::Resolved` wrap。
        assert_eq!(
            parse("red", "border-right-color"),
            Some(PropertyValue::BorderRightColor(BorderColor::Resolved(
                CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255
                }
            )))
        );
        assert_eq!(
            parse("rgb(0, 0, 255)", "border-bottom-color"),
            Some(PropertyValue::BorderBottomColor(BorderColor::Resolved(
                CssColor {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255
                }
            )))
        );
        assert_eq!(
            parse("transparent", "border-left-color"),
            Some(PropertyValue::BorderLeftColor(BorderColor::Resolved(
                CssColor::TRANSPARENT
            )))
        );
    }

    #[test]
    fn border_top_color_parse_currentcolor() {
        // CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
        // "Initial: currentcolor" — author 明示 `border-*-color: currentcolor` が
        // `BorderColor::CurrentColor` variant として保持されることを pin する
        // (hazard case 1 の cascade-side coverage、used-value resolution は
        // paint scope 責務)。
        assert_eq!(
            parse("currentcolor", "border-top-color"),
            Some(PropertyValue::BorderTopColor(BorderColor::CurrentColor))
        );
        // CSS Color 3 §4.4 keyword は ASCII case-insensitive。
        assert_eq!(
            parse("CurrentColor", "border-right-color"),
            Some(PropertyValue::BorderRightColor(BorderColor::CurrentColor))
        );
        assert_eq!(
            parse("CURRENTCOLOR", "border-bottom-color"),
            Some(PropertyValue::BorderBottomColor(BorderColor::CurrentColor))
        );
    }

    #[test]
    fn border_shorthand_all_three_components() {
        // Verification #5: parse("1px solid red", "border") = shorthand 経由で
        // 全 4 side の Border {width: 1px, style: Solid, color: red} を expand。
        // color slot は `BorderColor::Resolved` に wrap。
        let border = Border {
            width: Length::Px(1.0),
            style: BorderStyle::Solid,
            color: BorderColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        };
        assert_eq!(
            parse("1px solid red", "border"),
            Some(PropertyValue::Border(Sides::all(border)))
        );
    }

    #[test]
    fn border_shorthand_any_order() {
        // spec §3.4 grammar は `||` (any-order)。全 6 permutation を pin する
        // 代わりに 3 order (color-first / style-first / mixed) を smoke。
        let expected = Border {
            width: Length::Px(2.0),
            style: BorderStyle::Dashed,
            color: BorderColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        };
        // color first
        assert_eq!(
            parse("red 2px dashed", "border"),
            Some(PropertyValue::Border(Sides::all(expected)))
        );
        // style first
        assert_eq!(
            parse("dashed 2px red", "border"),
            Some(PropertyValue::Border(Sides::all(expected)))
        );
    }

    #[test]
    fn border_shorthand_omitted_components_use_initial() {
        // spec §3.4 "Omitted values are set to their initial values" —
        // width 省略 → medium (3px)、style 省略 → None、color 省略 →
        // `currentcolor` keyword (`BorderColor::CurrentColor`、spec §3.1
        // initial)。
        // 1 component only (color) — width と style は initial:
        let with_only_color = Border {
            width: Length::Px(3.0), // medium initial
            style: BorderStyle::None,
            color: BorderColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        };
        assert_eq!(
            parse("red", "border"),
            Some(PropertyValue::Border(Sides::all(with_only_color)))
        );
        // 1 component only (style) — width と color は initial:
        let with_only_style = Border {
            width: Length::Px(3.0),
            style: BorderStyle::Solid,
            color: BorderColor::CurrentColor, // spec §3.1 initial
        };
        assert_eq!(
            parse("solid", "border"),
            Some(PropertyValue::Border(Sides::all(with_only_style)))
        );
    }

    #[test]
    fn border_width_medium_is_consistent_across_its_independent_call_sites() {
        // Before this fix, `medium` = 3px was written
        // as 3 independent `Length::Px(3.0)` literals — the `medium` keyword
        // branch in `parse_border_width_side`, the border shorthand's
        // omitted-width default in `parse_border_shorthand`, and
        // `crate::specified::INITIAL_BORDER`'s `width` field — with no test
        // tying them together, so they could silently drift apart. All 3 now
        // derive from `BORDER_WIDTH_MEDIUM_PX`; this test exercises all 3
        // through real behavior (not literal-vs-literal) and pins that they
        // still agree with each other and with the const, so a future edit
        // that touches only one of them fails loudly here instead of
        // drifting silently. The sibling tests
        // `border_top_width_parse_medium_keyword` and
        // `border_shorthand_omitted_components_use_initial` independently
        // pin the *absolute* value (`3.0`) as a literal — do not fold those
        // into a reference to the const, or nothing catches an accidental
        // edit to the const itself (see the const's doc).
        let via_keyword = parse("medium", "border-top-width");
        assert_eq!(
            via_keyword,
            Some(PropertyValue::BorderTopWidth(Length::Px(
                BORDER_WIDTH_MEDIUM_PX
            )))
        );

        let via_shorthand_omission = parse("solid", "border");
        // cov:ignore: this let-else panic branch is unreached as long as the
        // test passes — `parse("solid", "border")` always matches
        // `Some(PropertyValue::Border(_))`, so llvm-cov marks the panic-message
        // literal "uncovered" the same way it does for any other panic-only
        // branch (same false-positive class as r7r1).
        let Some(PropertyValue::Border(sides)) = via_shorthand_omission else {
            panic!("expected `border: solid` to parse to a Border shorthand value");
        };
        assert_eq!(sides.top.width, Length::Px(BORDER_WIDTH_MEDIUM_PX));

        assert_eq!(
            crate::specified::INITIAL_BORDER.width,
            Length::Px(BORDER_WIDTH_MEDIUM_PX)
        );
    }

    #[test]
    fn border_default_matches_initial_border() {
        // `Border::default()` (public, umbrella-facing
        // constructor) and `crate::specified::INITIAL_BORDER` (`pub(crate)`,
        // cascade-internal fast path) encode the same CSS Backgrounds 3
        // initial value. Precision on what this actually catches (the sibling
        // test just above, `border_width_medium_is_consistent_across_its_independent_call_sites`,
        // warns explicitly against a "vacuous pin" of this shape):
        //
        // - `style` / `color`: each side hardcodes `BorderStyle::None` /
        //   `BorderColor::CurrentColor` independently (no shared constant), so
        //   this assert is a real independent-literal drift check for those 2
        //   fields — same rationale as the sibling test.
        // - `width`: both sides already read `BORDER_WIDTH_MEDIUM_PX` (this
        //   fn's own body and `INITIAL_BORDER`'s definition), so an edit to
        //   that const moves both sides together and this assert alone would
        //   NOT catch it — that drift is what the sibling test's real,
        //   behavior-driven exercise of the const (plus
        //   `border_top_width_parse_medium_keyword`'s absolute-value literal
        //   pin) already covers. This test's width leg is a
        //   both-must-reference-the-same-const structural check, not an
        //   independent value pin — do not treat it as one.
        assert_eq!(Border::default(), crate::specified::INITIAL_BORDER);
    }

    #[test]
    fn border_new_is_default() {
        // `Border::new()` is documented as a thin
        // `Self::default()` wrapper (same shape as
        // `raikiri_traits::page::PageBox::new`) — pin that the two stay
        // equivalent.
        assert_eq!(Border::new(), Border::default());
    }

    #[test]
    fn border_shorthand_color_slot_accepts_currentcolor() {
        // 37n sibling: border shorthand の color slot は 4 longhand と同じ
        // `parse_border_color` を経由するため、`currentcolor` keyword も
        // shorthand から受理される。
        let expected = Border {
            width: Length::Px(1.0),
            style: BorderStyle::Solid,
            color: BorderColor::CurrentColor,
        };
        assert_eq!(
            parse("1px solid currentcolor", "border"),
            Some(PropertyValue::Border(Sides::all(expected)))
        );
        // 引数 order は自由。style first。
        assert_eq!(
            parse("solid currentcolor 1px", "border"),
            Some(PropertyValue::Border(Sides::all(expected)))
        );
    }

    #[test]
    fn border_shorthand_empty_returns_none() {
        // `||` grammar は at least 1 component 必須。0 component は None。
        assert_eq!(parse("", "border"), None);
    }

    #[test]
    fn border_shorthand_unknown_keyword_only_returns_none() {
        // 未知 keyword (width/style/color いずれの slot にも match しない) は
        // 1st iteration で全 slot None、`matched=false` で break、0-component
        // guard で None (declaration drop)。
        assert_eq!(parse("garbage", "border"), None);
    }

    #[test]
    fn border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check() {
        // `border: 1px 2px` — 1st iteration で width=1px、2nd iteration で
        // width slot 満了、`2px` は他 slot (style/color) に match しないため
        // fall-through break。leftover は caller の `expect_exhausted` 責務。
        // 本 helper 単体としては 1st を確保して Some を返す (parse_value 経路
        // では end-to-end で declaration drop する — rule.rs test で pin 予定)。
        let expected = Border {
            width: Length::Px(1.0),
            style: BorderStyle::None,
            color: BorderColor::CurrentColor, // spec §3.1 initial
        };
        let mut input = ParserInput::new("1px 2px");
        let mut parser = Parser::new(&mut input);
        let result = parse_border_shorthand(&mut parser);
        assert_eq!(result, Some(Sides::all(expected)));
        // 2px は unconsumed のまま — parser cursor は "2px" の直前を指す。
        assert!(!parser.is_exhausted());
    }

    #[test]
    fn border_longhand_keys_map_correctly() {
        // 12 longhand + 1 shorthand variant → 対応 key (cascade winner 選択の
        // discriminant integrity)。sibling `margin_longhand_keys_map_correctly`
        // と同 pattern。
        assert_eq!(
            PropertyValue::BorderTopWidth(Length::Px(1.0)).key(),
            PropertyKey::BorderTopWidth
        );
        assert_eq!(
            PropertyValue::BorderRightWidth(Length::Px(1.0)).key(),
            PropertyKey::BorderRightWidth
        );
        assert_eq!(
            PropertyValue::BorderBottomWidth(Length::Px(1.0)).key(),
            PropertyKey::BorderBottomWidth
        );
        assert_eq!(
            PropertyValue::BorderLeftWidth(Length::Px(1.0)).key(),
            PropertyKey::BorderLeftWidth
        );
        assert_eq!(
            PropertyValue::BorderTopStyle(BorderStyle::Solid).key(),
            PropertyKey::BorderTopStyle
        );
        assert_eq!(
            PropertyValue::BorderRightStyle(BorderStyle::Solid).key(),
            PropertyKey::BorderRightStyle
        );
        assert_eq!(
            PropertyValue::BorderBottomStyle(BorderStyle::Solid).key(),
            PropertyKey::BorderBottomStyle
        );
        assert_eq!(
            PropertyValue::BorderLeftStyle(BorderStyle::Solid).key(),
            PropertyKey::BorderLeftStyle
        );
        assert_eq!(
            PropertyValue::BorderTopColor(BorderColor::CurrentColor).key(),
            PropertyKey::BorderTopColor
        );
        assert_eq!(
            PropertyValue::BorderRightColor(BorderColor::CurrentColor).key(),
            PropertyKey::BorderRightColor
        );
        assert_eq!(
            PropertyValue::BorderBottomColor(BorderColor::CurrentColor).key(),
            PropertyKey::BorderBottomColor
        );
        assert_eq!(
            PropertyValue::BorderLeftColor(BorderColor::CurrentColor).key(),
            PropertyKey::BorderLeftColor
        );
        let default_border = Border {
            width: Length::Px(3.0),
            style: BorderStyle::None,
            color: BorderColor::CurrentColor, // spec §3.1 initial
        };
        assert_eq!(
            PropertyValue::Border(Sides::all(default_border)).key(),
            PropertyKey::Border
        );
    }

    #[test]
    fn width_key_maps_to_width_property_key() {
        // PropertyValue::Width → PropertyKey::Width (cascade winner 選択の
        // discriminant integrity、既存 sibling padding/margin と同じ pattern)。
        assert_eq!(
            PropertyValue::Width(LengthOrAuto::Auto).key(),
            PropertyKey::Width
        );
        assert_eq!(
            PropertyValue::Width(LengthOrAuto::Length(Length::Px(100.0))).key(),
            PropertyKey::Width
        );
    }

    // ── height (CSS Sizing 3 §3.1.1) ─────────────
    //
    // Primary source:
    // - #preferred-size-properties: `auto | <length-percentage [0,∞]> |
    //   min-content | max-content | fit-content(<length-percentage>)`,
    //   initial `auto`, Inheritance `No`.
    //
    // 現状 scope は `auto` + 非負 `<length-percentage>` の 2 分岐のみ、
    // 他 sizing keyword / global keyword / calc() / var() は silent drop
    // (parse_height doc の Scope carving 節参照)。
    //
    // 37n sibling: sibling `width` と同 shape の非負 `<length-percentage>` +
    // `auto` grammar、payload 型は共通 `LengthOrAuto`。

    #[test]
    fn height_parse_auto() {
        // Verification 1 (task doc): `auto` ident は spec initial value でもある
        // (§3.1.1 "Initial: auto") — cascade winner として declaration が到達
        // した場合の受理 pattern を pin。
        assert_eq!(
            parse("auto", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Auto))
        );
    }

    #[test]
    fn height_parse_px() {
        // Verification 2 (task doc): 非負 px は spec-valid `<length-percentage>`
        // (§3.1.1)。sibling `width_parse_length_px` と同 shape。
        assert_eq!(
            parse("100px", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(
                100.0
            ))))
        );
    }

    #[test]
    fn height_parse_percentage() {
        // Verification 3 (task doc): percentage 受理 (parse_length_value の
        // allow_percentage = true 経路)。resolve (containing block % → 実寸)
        // は下流責務。
        assert_eq!(
            parse("50%", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(
                Length::Percent(50.0)
            )))
        );
    }

    #[test]
    fn height_rejects_negative_length() {
        // Verification 4 (task doc): `<length-percentage [0,∞]>` (§3.1.1) の
        // 非負制約により `-10px` は spec-invalid → drop。sibling
        // padding の非負フィルタ pattern と同 shape、margin の `-10px` 受理
        // (§3.1) との対称的な reject を pin。
        assert_eq!(parse("-10px", "height"), None);
    }

    #[test]
    fn height_accepts_zero() {
        // spec `<length-percentage [0,∞]>` — 0 は閉区間下端。`0px` は Dimension arm、
        // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す。
        // parse_height の `>= 0.0` 非負 filter を pass。
        assert_eq!(
            parse("0px", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(0.0))))
        );
        assert_eq!(
            parse("0", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(0.0))))
        );
    }

    #[test]
    fn height_rejects_negative_percentage() {
        // 非負フィルタが Percent variant にも効く pin (parse_padding_side の
        // 同 pattern、Verification 4 の姉妹)。
        assert_eq!(parse("-10%", "height"), None);
    }

    #[test]
    fn height_rejects_unsupported_sizing_keyword() {
        // Non-goal (b) 非対応: `min-content` / `max-content` /
        // `fit-content()` は spec-valid だが現状 scope 外、silent drop。
        // ident branch は `auto` matching のみ、length parser の Dimension /
        // Percentage arm でも受理されず None に落ちる pin。
        assert_eq!(parse("min-content", "height"), None);
        assert_eq!(parse("max-content", "height"), None);
        assert_eq!(parse("fit-content(50%)", "height"), None);
    }

    #[test]
    fn height_rejects_css_wide_keyword() {
        // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
        // canonical: PropertyValue doc「CSS-wide keyword」節。
        assert_eq!(parse("inherit", "height"), None);
        assert_eq!(parse("initial", "height"), None);
        assert_eq!(parse("unset", "height"), None);
        assert_eq!(parse("revert", "height"), None);
        assert_eq!(parse("revert-layer", "height"), None);
    }

    #[test]
    fn height_case_insensitive_auto() {
        // CSS spec: ident keyword は ASCII case-insensitive
        // (`expect_ident_matching` の cssparser 慣行、sibling
        // `margin_side_case_insensitive_auto` と同 pattern)。
        assert_eq!(
            parse("AUTO", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Auto))
        );
    }

    #[test]
    fn height_rejects_unsupported_unit() {
        // `cap` (§6.1.1 font-relative lengths) は現状
        // 未対応 (parse_length_value 側で drop)。`cm` / `lh` / `rlh` は
        // それぞれ受理側へ移った (`height_accepts_absolute_unit` /
        // `height_accepts_lh` 参照)。sibling
        // `margin_side_rejects_unsupported_unit` と同 pattern。
        assert_eq!(parse("1cap", "height"), None);
    }

    #[test]
    fn height_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh`。
        assert_eq!(
            parse("1.5lh", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Lh(1.5))))
        );
        assert_eq!(
            parse("2rlh", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Rlh(
                2.0
            ))))
        );
    }

    #[test]
    fn height_accepts_absolute_unit() {
        // CSS Values 4 §6.2 absolute lengths。
        assert_eq!(
            parse("1cm", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Cm(1.0))))
        );
    }

    #[test]
    fn height_rejects_negative_absolute_unit() {
        assert_eq!(parse("-1cm", "height"), None);
    }

    #[test]
    fn height_parse_em_and_rem() {
        // grammar coverage: font-relative units (`em` / `rem`) も
        // `<length-percentage>` mode で受理される。resolve は下流
        // (font-size context / root font-size context) 責務。
        assert_eq!(
            parse("1.2em", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Em(1.2))))
        );
        assert_eq!(
            parse("2rem", "height"),
            Some(PropertyValue::Height(LengthOrAuto::Length(Length::Rem(
                2.0
            ))))
        );
    }

    #[test]
    fn height_key_maps_to_height_property_key() {
        // sibling `margin_longhand_keys_map_correctly` と同 pattern — cascade
        // winner selection の discriminant integrity を pin。
        let v = PropertyValue::Height(LengthOrAuto::Auto);
        assert_eq!(v.key(), PropertyKey::Height);
        let v = PropertyValue::Height(LengthOrAuto::Length(Length::Px(100.0)));
        assert_eq!(v.key(), PropertyKey::Height);
    }
}
