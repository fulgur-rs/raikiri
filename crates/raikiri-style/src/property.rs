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
/// `raikiri-spike-d9y.1` (SEC HIGH cascade memory DoS fix) の副作用として
/// `ComputedValues.content` / `.string_set` は `Arc<Vec<..>>` に wrap したが、
/// `Arc::new(Vec::new())` を every node で呼ぶと N-node document あたり
/// 2N の small heap allocation regression になる (advisor calibration)。
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
/// pattern (per-node empty allocation regression 回避、d9y.1)。
pub(crate) fn empty_string_set_entries() -> Arc<Vec<StringSetEntry>> {
    static EMPTY: OnceLock<Arc<Vec<StringSetEntry>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// 空 `counter-*` entries を表す shared Arc — 3 property
/// (`counter-reset` / `counter-increment` / `counter-set`) 全てで単一 slot を
/// 共有する ([`Vec<(SmolStr, i32)>`] は同一型のため helper を分ける必要無し)。
///
/// `raikiri-spike-d9y.2` (SEC HIGH cascade memory DoS fix) の副作用 helper。
/// counter-* は non-inherited (CSS Lists 3 §4、`counter-reset` を含む全 3 property)
/// のため、`SpecifiedValues::inherit_from` が child stack entry のたびに empty 値で
/// 初期化する。生 `Vec::new()` を使うと per-node で 3 個の `Vec` struct
/// (24 bytes × 3) が生まれ N-node document あたり O(N) の overhead になるため、
/// [`empty_content_list`] / [`empty_string_set_entries`] と同じ `OnceLock` 保持の
/// shared Arc を使う (advisor calibration precedent)。
pub(crate) fn empty_counter_entries() -> Arc<Vec<(SmolStr, i32)>> {
    static EMPTY: OnceLock<Arc<Vec<(SmolStr, i32)>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// `font-family` の initial value を表す shared Arc — [`empty_content_list`]
/// 等と同じ `OnceLock` 保持の shared-slot pattern (raikiri-spike-no7b、d9y.1 /
/// d9y.2 pattern の踏襲)。
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
    /// (g04 category (a) spec-invalid → drop)。leading `#` は tokenizer
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
/// `(n << 4) | n = n * 17` — CSS Color 4 §5.2 の「"duplicating" all of the
/// digits」を実装した short-form 展開 helper (`#f` → `0xff`, `#8` → `0x88`)。
fn expand_hex_nibble(n: u8) -> u8 {
    (n << 4) | n
}

/// CSS length or length-percentage value (Author CSS seed for m4+ box model).
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
/// は `PropertyValue` の bag なので computed 値も本型で運ばれる (bd
/// raikiri-spike-sshp)。したがって「`Length` が見えたから未解決」と判断しては
/// ならない。
///
/// **本節が「型は層を表明しない」規則の canonical な記述である。**
/// 一方、page 経路が具体的に何を保証するか (どの値が computed 層に居るのか、
/// 例外は何か) は
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// の doc が canonical であり、その内容は `page::tests` の
/// `page_declarations_carry_no_specified_layer_residue` (raikiri-spike-l3wg
/// 以前の名前は `page_declarations_carry_exactly_one_specified_layer_residue`)
/// が機械的に pin している。**ここに保証の中身を書き足して重複させないこと**
/// — 手で 2 site を揃える運用は既に 2 度 drift した (bd raikiri-spike-awjx)。
///
/// Downstream match は必ず wildcard arm を持つこと (`#[non_exhaustive]` 属性、
/// 変数追加が既存 pattern-match を break しない forward-compat 契約)。
///
/// **訂正 (bd raikiri-spike-2x8)**: 本節は以前 `crates/raikiri-dom/src/
/// layout.rs:143` の `preshape_text` を「wildcard arm を持つ既存 sibling」と
/// して挙げていたが、これは bd raikiri-spike-zls8 の Option A 層分離
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
/// 現時点でこの契約を exercise している既存 site は無い (bd raikiri-spike-2x8
/// で `crates/` 全体を再 grep して確認)。
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
    /// (<https://www.w3.org/TR/css-values-4/#ex>) 原文: "In the cases where
    /// it is impossible or impractical to determine the x-height, [...] a
    /// value of 0.5em must be assumed." Resolve は `0.5 * font-size`
    /// (bd raikiri-spike-2x8)。
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
    /// (<https://www.w3.org/TR/css-values-4/#ch>) 原文: "In the cases where
    /// it is impossible or impractical to determine the measure of the '0'
    /// glyph, it must be assumed to be 0.5em wide by 1em tall. Thus, the ch
    /// unit falls back to 0.5em in the general case, and to 1em when it
    /// would be typeset upright (i.e. writing-mode is vertical-rl or
    /// vertical-lr and text-orientation is upright)." raikiri-style は
    /// `writing-mode` / `text-orientation` を未実装 (horizontal-tb 前提のみ)
    /// なので upright 分岐は到達不能 — resolve は常に `0.5 * font-size`。
    /// `writing-mode` 実装時に本判断の見直しが要る (bd raikiri-spike-2x8)。
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
    /// (<https://www.w3.org/TR/css-values-4/#ic>) 原文: "In the cases where
    /// it is impossible or impractical to determine the measure of the CJK
    /// water ideograph glyph, the ic unit must fall back to 1em." resolve は
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
    /// (<https://www.w3.org/TR/css-values-4/#lh>) 原文: "Equal to the computed
    /// value of the line-height property of the element on which it is used,
    /// converting normal to an absolute length by using only the metrics of
    /// the first available font."
    ///
    /// # `normal` の resolve — `cap`/`rcap` と同じ wall (bd raikiri-spike-vxha)
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
    /// (roborev-refine iter 1 quality lens 1、bd raikiri-spike-awjx の
    /// drift 前例により、本節では要約に留め全文を再掲しない)。
    /// `font-size: 1lh` も同条項の対象で自己参照になる (font-size は
    /// font-\* property) — 親の used line-height を基準に解決する
    /// (bd raikiri-spike-yh3w、[`crate::resolve::resolve_font_size`] doc の
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
    /// # `Length::Lh` と非対称 — 自己参照として扱わない (bd raikiri-spike-vxha)
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
    /// (bd raikiri-spike-yh3w、[`crate::resolve::resolve_font_size`] doc 参照)。
    Rlh(f32),
}

/// `<length-percentage> | auto` — margin / width で共有される Author CSS seed
/// (最初は 0vv.5 で margin longhand 用に導入、0vv.10 で `width` からも reuse)。
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
/// を含まないため、0vv.6 padding は本 type を **使わず** [`Sides<Length>`] を
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

/// 4-side box-model value holder。field 順は CSS Box 3 §4.2 shorthand の
/// 4-value form `top right bottom left` に一致 (clockwise from top)。
///
/// Sprint 12 で padding shorthand が最初の consumer (raikiri-spike-0vv.6)、
/// sibling raikiri-spike-0vv.5 (margin) は `Sides<LengthOrAuto>` として reuse
/// する — 型パラメータで per-property の value type 差を吸収する。
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
/// # Non-goals (g04 3-category labels)
///
/// - **(b) milestone subset**: paint side での visual 差 (double stroke / 3D
///   groove/ridge/inset/outset の shading) は paint scope で defer、cascade
///   static side では spec value を保持するのみ。
/// - **(a) spec-invalid**: 未知 keyword (`wavy` / `wave` 等 CSS Text Decoration
///   4 の `<text-decoration-style>` 由来 keyword は本 property では invalid) は
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
///
/// (raikiri-spike-0vv.12)
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
/// lookup) は paint scope 責務 (bd raikiri-spike-q7qf、border 描画実装との
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
///
/// (raikiri-spike-0vv.17)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderColor {
    /// `currentcolor` keyword — border-*-color の spec-mandated initial value
    /// (CSS Backgrounds 3 §3.1)。used-value は paint scope で node の computed
    /// `color` property を lookup して確定する (bd raikiri-spike-q7qf)。
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
///   resolve する (raikiri-spike-0vv.17 で `CssColor::BLACK` placeholder から
///   格上げ、CSS Backgrounds 3 §3.1 の initial 契約準拠)。
///
/// # `<line-width>` keyword mapping (§3.3)
///
/// spec §3.3 "Line Thickness: the border-width properties" は
/// `<line-width> = <length [0,∞]> | thin | medium | thick`。thin=1px、
/// medium=3px、thick=5px は spec 規定値 (verbatim "are equivalent to 1px,
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
/// 展開される (spec CSS Cascading L5 §"Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-5/#shorthand> 準拠、raikiri-spike-0vv.5
/// margin precedent の踏襲)。
///
/// (raikiri-spike-0vv.12)
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
    /// に含まれない (padding とは違う grammar、advisor calibration)。
    pub width: Length,
    /// border-style (CSS Backgrounds 3 §3.2)。initial `none`。
    pub style: BorderStyle,
    /// border-color (CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>)。
    /// initial は `currentcolor` keyword — [`BorderColor::CurrentColor`] を
    /// enum variant として保持し、used-value resolution (currentcolor →
    /// 同 node の computed `color` property) は paint scope で確定する
    /// (bd raikiri-spike-q7qf)。raikiri-spike-0vv.17 で `CssColor` から
    /// [`BorderColor`] enum へ格上げ (spec initial 契約 fidelity)。
    pub color: BorderColor,
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
/// # `Eq` を derive しない (bd raikiri-spike-e52s)
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
    /// `u16`、bd raikiri-spike-e52s で格上げ — 詳細は [`parse_font_weight`] doc)。
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

/// `line-height` property の value (Author CSS seed for m4+ inline layout)。
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
/// invalid → parser 側で drop (`parse_line_height` の post-filter)。g04
/// category (a) spec-invalid → drop: spec grammar が range を parse-time で
/// 制約しているため、reject 自体が spec 準拠 (stricter ではなく match)。
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
/// M5 static-side scope では named style を SmolStr で pass-through する
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
/// spec default = `first` (per §2.7.2 "if the second argument is omitted").
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
/// <https://www.w3.org/TR/css-content-3/#target-text>。
/// spec default = `content` (per §2.6.3 "The default value is `content`").
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentPart {
    /// `content` — spec default (element の string value)。
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
/// 未解決の WG issue として残っており、TR 上安定した根拠ではない (bd
/// raikiri-spike-x8i6 で発見された同一 overclaim class、bd
/// raikiri-spike-83r2 で本 site を訂正)。
///
/// NB: sibling [`ContentPart`] (target-text() 用) と keyword 集合が重なるが、
/// `text` vs `content` の spec spelling divergence があるため型を分ける
/// (StringFetchMode / ContentPart と同じ per-function 専用 enum 慣行、
/// reviewer:spec: `content(content)` を silently accept してはならない)。
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
/// spec 原文: [`OpenQuote`](Self::OpenQuote) / [`CloseQuote`](Self::CloseQuote)
/// は "replaced by the appropriate string from the `quotes` property" かつ
/// nesting depth を増減する。[`NoOpenQuote`](Self::NoOpenQuote) /
/// [`NoCloseQuote`](Self::NoCloseQuote) は "insert nothing (as in none)" だが
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
/// spec 原文: `dotted` は "equivalent to `leader(".")`"、`solid` は
/// "equivalent to `leader("_")`"、`space` は "equivalent to `leader(" ")`"。
/// この等価性は **keyword の意味論の説明であって spelling の正規化指示ではない**
/// ([`counter_style_from_ident`] が `decimal` keyword を `Named("decimal")` に
/// 畳まず [`CounterStyle::Decimal`] という別 variant で保持するのと同じ
/// precedent) — 3 keyword を個別 variant に保持し、実際の leader glyph
/// 文字列への解決 (`Dotted` → `"."` 等) は downstream (paint) の rendering
/// 責務とする。[`String`](Self::String) variant の custom leader 文字列は
/// [`SmolStr`] で保持 ([`ContentComponent::Literal`] の d9y.1 SmolStr 化
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
/// [`ContentTextKeyword`] と同じ per-context 専用 enum 慣行、raikiri-spike-6s1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentListMode {
    /// CSS Content 3 §2 broad `<content-list>` — `content` property 用。
    /// 受理: `<string>` bare literal / `counter()` / `counters()` / `string()` /
    /// `attr()` / `target-counter()` / `target-counters()` / `target-text()` /
    /// `content()` / `<image>` (`url()` alternative のみ、raikiri-spike-1us) /
    /// `contents` keyword / `<quote>` (`open-quote` 等) / `leader()`。10 alt
    /// full set (raikiri-spike-1us で image/contents/quote/leader を追加、
    /// m5.1 由来の under-accept を解消)。
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
/// に依存しない leaf crate = 94e/3ps Phase B により、counter-* wire-through
/// pattern (raikiri-spike-s85) と同様に **local** な intermediate type で保持し、
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
///   <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> (raikiri-spike-5ri)
/// - Image / Contents / Quote / Leader: CSS Content 3 §2.2 / §2.3 / §2.4.2 /
///   §2.5.1 (raikiri-spike-1us、m5.1 由来 under-accept の fix — 末尾に追加、
///   既存 variant の並びは互換性のため保持)
///
/// URL は raw `String` として保持 (raikiri-style は `url` crate に依存しない —
/// runtime resolve 段で `url::Url` へ parse する consumer 責務)。
///
/// (raikiri-spike-m5.1)
///
/// # `#[non_exhaustive]` semantics (fulgur / downstream consumer 向け verbatim)
///
/// enum-level `#[non_exhaustive]` は downstream の `match` に `_ =>` arm を
/// 強制することで新 variant 追加を forward-compatible にするが、**既存 variant
/// の tuple constructor 呼び出しは block しない**。ゆえに既存 variant の
/// payload **type** 変更は downstream の constructor を compile-break させる。
///
/// Sprint 9 hardening (bd raikiri-spike-d9y.1、SEC HIGH cascade memory DoS fix)
/// では [`Literal`](Self::Literal) の payload を `String` → [`SmolStr`] に
/// 変更した (bd raikiri-spike-q3f wall/umbrella formal declare)。SmolStr は
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
    /// clone が O(1) bump になる (raikiri-spike-d9y.1 の DoS 直系 attack vector
    /// `content: "<large>"` に対する secondary defense、primary は outer
    /// [`PropertyValue::Content`] の [`Arc<Vec<..>>`] wrap)。
    ///
    /// **Consumer 向け** (bd raikiri-spike-q3f wall/umbrella formal declare):
    /// pre-d9y.1 の payload は `String` だった。SmolStr は `Deref<Target = str>`
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
    /// `attr(<attribute-name>)` (§2.1、type/fallback は M5+ scope)。
    Attr { name: SmolStr },
    /// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`。
    /// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>。
    ///
    /// 第 2 引数は `<counter-name>` ではなく `<custom-ident>` — spec 原文
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
    /// 第 2 引数は `<counter-name>` ではなく `<custom-ident>` — spec 原文
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
    /// [`ContentTextKeyword`] の doc comment 参照、bd raikiri-spike-x8i6 /
    /// raikiri-spike-83r2)。runtime resolve は raikiri-dom 責務 (m5.1
    /// wire-through pattern)。
    Content { keyword: ContentTextKeyword },
    /// `<image>` (CSS Images 3 <https://www.w3.org/TR/css-images-3/#typedef-image>
    /// `<image> = <url> | <gradient>`) — CSS Content 3 §2.2 "2D Images: the
    /// `<image>` values" <https://www.w3.org/TR/css-content-3/#content-uri>。
    /// spec 原文: "Represents an anonymous inline replaced element filled with
    /// the specified `<image>`. If the `<image>` represents an invalid image,
    /// this value instead represents nothing" (rendering 側の fallback は
    /// downstream 責務)。
    ///
    /// **`<content-replacement>` との関係 (未反映、raikiri-spike-5hp8 送り)**:
    /// `content` property 全体の value definition (CSS Content 3 §1
    /// <https://www.w3.org/TR/css-content-3/#content-property>) は `normal |
    /// none | [ <content-replacement> | <content-list> ] […]?` で、
    /// `<content-replacement> = <image>` は `<content-list>` とは別の
    /// top-level alternative — spec 原文 "Represents a *replaced element*"
    /// で `::before`/`::after` 生成を抑制する等、上の list-item 版
    /// `<image>` (anonymous inline replaced element) とは異なる semantics
    /// を持つ。spec 原文は続けて "If the value of `<content-list>` is a
    /// single `<image>`, it must instead be interpreted as a
    /// `<content-replacement>`" とも述べており、本 variant の shape
    /// (`Vec<ContentComponent>` の 1 要素が `Image` かどうか) は downstream
    /// がこの区別を再構成するのに十分な情報を保持している — replacement
    /// semantics 自体の実装 (pseudo-element 抑制含む) は本 crate の
    /// static-side scope 外。
    ///
    /// **milestone subset (g04 category (b))**: `<url>` alternative のみ実装
    /// (`url(...)` / `url("...")`)。`<gradient>` (`linear-gradient()` /
    /// `repeating-linear-gradient()` / `radial-gradient()` /
    /// `repeating-radial-gradient()`、CSS Images 3 §3.1-2) は gradient stop /
    /// color-interpolation infra が本 crate に無く defer — raikiri-spike-1us
    /// scope 外、追跡は follow-up task。CSS Images 4 で追加された `image()` /
    /// `image-set()` / `element()` / `cross-fade()` / `paint()` は参照した
    /// CSS Images **3** の `<image>` production に含まれないため g04 category
    /// (a) spec-invalid (Level 3 準拠) — これらは function 名が
    /// `parse_content_function` の match arm と一致せず自動的に drop される
    /// ため追加コード不要。
    ///
    /// URL は raw `String` として保持 (sibling [`TargetCounter`](Self::TargetCounter)
    /// 等と同じ convention、`url` crate 非依存)。
    Image { url: String },
    /// `contents` keyword — CSS Content 3 §2.3 "Elemental Content: the
    /// `contents` keyword" <https://www.w3.org/TR/css-content-3/#element-content>。
    /// spec 原文: "The element's descendants" — pseudo-element の生成有無や
    /// 「既に他の pseudo-element で使用済みなら何もしない」という消費順序の
    /// 解決は本 crate の static-side scope 外 (parse_content の docstring の
    /// `normal`/`none` と同じ「生成判断は下流に委ねる」方針)。
    ///
    /// **`normal` との非対称性 (意図的)**: spec 原文 (§2.3) は "the initial
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
/// 現状受理する keyword は Sprint 12 scope: `block` / `inline` / `inline-block`
/// / `none` の 4 値 (raikiri-spike-0vv.4)。`flex` / `grid` / `table*` /
/// `list-item` / `flow-root` (standalone) / `contents` 等 spec-valid だが
/// milestone defer 対象の keyword は `parse_display` が `None` を返し、
/// declaration が silent drop される (rule.rs 側 invalid-value drop path)。
///
/// `#[non_exhaustive]`: variant 追加を non-breaking にする (Sprint 12 の
/// InlineBlock / None 追加は本 attribute 経由で forward-compatible)。
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
/// # Scope carving (g04 3-category)
///
/// - **(b) milestone subset**: CSS-wide keyword (`inherit` / `initial` /
///   `unset` / `revert` / `revert-layer`) は Epic 7、silent drop。
/// - **(a) spec-invalid**: 未知 keyword (`padding-box` — CSS UI 3 draft 相当
///   だが css-sizing-3 では削除、`margin-box` 等) は silent drop = `None`。
///
/// # Downstream handoff (future scope、style-scope confined)
///
/// [`ComputedValues.box_sizing`] は cascade static side seed のみ保持し、
/// `apply_computed_to_style` bridge (dom scope、`taffy::Style::box_sizing`
/// への翻訳) は future cross-scope task に defer (bd raikiri-spike-0vv.13
/// Non-goals)。
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
/// # Scope carving (g04 3-category)
///
/// - **(b) milestone subset**: 本 crate は Sprint 12 seed として shorthand を expand
///   せず、`ComputedValues.text_align` 単一 field に保持する — margin (`Sides<T>`) や
///   `content` (`normal`/`none` → 空 list) と同じ「shorthand as single field」
///   convention。text-align-all / text-align-last longhand 分離 (§6.2 / §6.3) は
///   future task (Epic 5 or Epic 7 相当) で拡張。
/// - **(b) milestone subset**: CSS-wide keyword (`inherit` / `initial` / `unset` /
///   `revert` / `revert-layer`) は Epic 7、silent drop。
/// - **実装済み** (raikiri-spike-l3wg): `match-parent` の **computed-value 時解決**
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
///   computed `direction` ([`Direction`]、CSS Writing Modes 4 §2.1) を要する
///   — この 2 property の組がなぜ resolve 出来なかったかは
///   raikiri-spike-l3wg の origin (raikiri-spike-ygl0 §8.2 spec lens F1) 参照。
///   [`crate::computed::ComputedValues::text_align`] に残る値は常に解決済 —
///   `MatchParent` が computed 値として観測されることは無い (bd
///   raikiri-spike-l3wg 以降の invariant、`resolve_text_align_match_parent`
///   の debug_assert が pin する)。
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
/// # なぜこの property が要るか (raikiri-spike-l3wg)
///
/// 本 crate は raikiri-spike-ygl0 まで `direction` を computed 層に持たなかった
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
/// # Scope carving (g04 3-category)
///
/// - **(b) milestone subset**: CSS-wide keyword (`inherit` / `initial` /
///   `unset` / `revert` / `revert-layer`) は Epic 7、silent drop (37n sibling
///   [`TextAlign`] と同 convention)。
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
/// # 呼び出し元 (raikiri-spike-l3wg、resolve_relative_weight と同型の contract)
///
/// - Element 経路: [`crate::specified::SpecifiedValues::finalize`] /
///   [`crate::specified::SpecifiedValues::finalize_as_root`]。
/// - Page 経路: [`crate::cascade::resolve_against_inherited`]。
///
/// 両経路とも本関数へ funnel するので、"start/end を親の direction で
/// left/right に解決する" table の実装は 1 箇所にしか無い (CSS Fonts 4
/// bolder/lighter table を `resolve_relative_weight` 1 箇所に集約した
/// raikiri-spike-ygl0 の precedent を踏襲)。
///
/// **なぜ element 経路の呼び手が `apply_value` ではないか**: `apply_value`
/// は同一 node 上の他 winner (`direction` 自身を含む) が [`PropertyKey`]
/// 宣言順に順次 [`crate::specified::SpecifiedValues`] へ書き込まれる場所であり、
/// この関数が要る「**親の** direction」は自 node の `direction` winner の
/// 適用順序に左右されてはならない (適用順に依存しないことが
/// [`crate::cascade::resolve_inheritance`] の invariant)。`finalize` /
/// `finalize_as_root` は全 winner 適用後に**明示的に親の [`ComputedValues`]
/// を受け取って**呼ばれるため、この罠を構造的に避けられる。
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
                "invariant violated: 親の computed text-align が MatchParent のまま — 親側の解決が漏れている (raikiri-spike-l3wg invariant)"
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

/// `position` property の value — M5 static-side scope では `static` (default) と
/// GCPM `running(<custom-ident>)` のみ受理する。
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// は element を normal flow から取り除き、`element()` 経由で page margin box に
/// 配置可能な template として登録する。
///
/// `relative` / `absolute` / `fixed` / `sticky` は M5+ scope 外、silent drop
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

/// 現サポート property の resolved value (variant 一覧は下記、
/// property name → variant mapping は `parse_value` 参照)。
///
/// 認識できない property (例: `float` — 現行 milestone subset 外) や
/// invalid value (例: `font-size: math` — MathML scaling algorithm 未実装
/// (bd raikiri-spike-0vv.18) / `margin-top: 1cm` — `cm` unit 未対応) は
/// parser 段で `None` に落として rule から silently 除外される。
///
/// **box property は「認識できない」側ではない** — `margin` / `padding` /
/// `border-*` / `width` / `height` はいずれも認識対象で、下記に variant を持つ
/// (bd raikiri-spike-0vv.5 / .6 / .10 / .11 / .12)。`font-size: 1em` /
/// `font-size: medium` / `font-size: larger` も bd raikiri-spike-zls8 /
/// raikiri-spike-4rmu 以降は valid である。
/// **例を差し替えるときは sibling の [`crate::rule`] の
/// `drops_invalid_property_and_value` と揃えること** — 両者は同じ milestone
/// subset を説明しており、あちらだけ更新されて本 doc が取り残される drift が
/// 実際に起きた (bd raikiri-spike-sshp §8.3)。
///
/// # `#[non_exhaustive]` semantics (fulgur / downstream consumer 向け verbatim)
///
/// enum-level `#[non_exhaustive]` は downstream の `match` を forward-compatible
/// にする (新 variant 追加時 `_ =>` arm が必ず求められる) が、**既存 variant の
/// tuple constructor 呼び出しは block しない**。したがって variant の payload
/// **type** が変わると constructor 側は普通に compile-break する。
///
/// Sprint 9 hardening (bd raikiri-spike-d9y.1、SEC HIGH cascade memory DoS fix)
/// では正にこの break が発生し、Sprint 10 wall/umbrella formal declare
/// (bd raikiri-spike-q3f) で以下を fulgur consumer 向け migration 対象として
/// 表明する:
///
/// - [`Content`](Self::Content): `Content(Vec<ContentComponent>)` →
///   `Content(Arc<Vec<ContentComponent>>)`
/// - [`StringSet`](Self::StringSet): payload の outer `Vec<..>` を `Arc<Vec<..>>` に
///
/// Sprint 9 Wave 2 hardening (bd raikiri-spike-d9y.2、同 SEC HIGH の counter-*
/// 拡張) は Sprint 10 時点で consumer live impact 0 だが同 pattern:
///
/// - [`CounterReset`](Self::CounterReset) / [`CounterIncrement`](Self::CounterIncrement) /
///   [`CounterSet`](Self::CounterSet): `Vec<(SmolStr, i32)>` → `Arc<Vec<(SmolStr, i32)>>`
///
/// Sprint 18 hardening (bd raikiri-spike-no7b、d9y.1/d9y.2 pattern の踏襲 — perf
/// lens、out-of-diff pre-existing finding、SEC 分類ではない) は同 pattern を
/// 最後の non-Arc `Vec` payload に適用する:
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
/// 詳細は bd raikiri-spike-q3f (`Content`/`StringSet` の `Arc<Vec<_>>` 化 +
/// Deref chain 透過性の wall/umbrella declare) を参照。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// `color: <color>` — inherited、initial: black。
    Color(CssColor),
    /// `background-color: <color>` — **non-inherited**、initial: `transparent`。
    /// CSS Backgrounds 3 §2.2 "Base Color: the background-color property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#background-color>。
    /// (raikiri-spike-0vv.7)
    BackgroundColor(CssColor),
    /// `font-family: <family-name>#` — inherited。CSS Fonts 4 §2.1
    /// <https://www.w3.org/TR/css-fonts-4/#font-family-prop> の spec 上の
    /// initial は "depends on user agent"。本実装は `[Atom::from("serif")]`
    /// を採る ([`crate::property::initial_font_family`] doc 参照)。
    ///
    /// [`Arc<Vec<..>>`] wrap (raikiri-spike-no7b、d9y.1/d9y.2 pattern踏襲):
    /// cascade winner move (`apply_value`) と inheritance walk clone
    /// (`SpecifiedValues::inherit_from` の `parent.font_family.clone()`) が
    /// **shallow (Arc bump)** になる。`font-family` は inherited property なので
    /// non-inherited な counter-* / content / string-set とはコストの形が違う —
    /// 「毎 node で initial にリセットする」コストではなく「inheritance walk が
    /// 毎 node で親の値を運ぶ」コストで、N-node document あたり O(N) の
    /// 1-element `Vec` malloc になっていた (origin: raikiri-spike-zpui §8.2
    /// perf lens、out-of-diff pre-existing)。`Arc<Vec<T>>: Deref<Target = Vec<T>>`
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
    /// 時点で `medium` (16px) 基準の `Length::Px` へ解決し尽くす
    /// (raikiri-spike-4rmu)。`<relative-size>` (`larger` / `smaller`) は
    /// 継承先依存のため別 variant ([`Self::FontSizeRelative`]) を持つ —
    /// 理由は同 variant の doc を参照。`math` keyword は g04 category (b)
    /// milestone subset (bd raikiri-spike-0vv.18、MathML scaling algorithm が
    /// 丸ごと未実装) として `parse_font_size` が `None` に落とす。
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
    /// `crates/raikiri` 側の修正を要求する = wall/umbrella 相当の破壊的変更に
    /// なる (bd raikiri-spike-q3f の `Content`/`StringSet` payload 変更が
    /// 同種の前例)。
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
    /// 格納する (raikiri-spike-17s8)。
    ///
    /// **page context 側も解決される** (raikiri-spike-ygl0)。
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
    /// Sprint 12 scope: `block` / `inline` / `inline-block` / `none`
    /// (raikiri-spike-0vv.4、詳細は [`DisplayValue`] doc)。
    Display(DisplayValue),
    /// `counter-reset: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited。spec initial は `none` (CSS Lists 3 §4.1)、本 impl はそれを
    /// 空 list で表現する。
    /// missing integer は 0 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`) + inheritance walk clone (`resolve_inheritance` の
    /// `stack.push((child, computed.clone()))` + `out[idx] = computed.clone()`)
    /// が **shallow (Arc bump only)** になる。counter-* は non-inherited のため
    /// child は inherit_from で shared empty slot に落ちるが、winner までの経路
    /// (parent stack entry + cascaded candidates 蓄積) は deep-clone 経由だった。
    /// `* { counter-reset: c0 c1 ... cN }` × M element で O(N × M) → O(N + M)
    /// (raikiri-spike-d9y.2 SEC HIGH、d9y.1 Content/StringSet pattern の踏襲)。
    CounterReset(Arc<Vec<(SmolStr, i32)>>),
    /// `counter-increment: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited。spec initial は `none` (CSS Lists 3 §4.2)、本 impl はそれを
    /// 空 list で表現する。
    /// missing integer は 1 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::CounterReset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    CounterIncrement(Arc<Vec<(SmolStr, i32)>>),
    /// `counter-set: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited。spec initial は `none` (CSS Lists 3 §4.2)、本 impl はそれを
    /// 空 list で表現する。
    /// missing integer は 0 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::CounterReset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    CounterSet(Arc<Vec<(SmolStr, i32)>>),
    /// `content: normal | none | <content-list>` — non-inherited。spec initial は
    /// `normal`、本 impl は `normal` / `none` をどちらも空 list で表現する
    /// (pseudo-element 生成判断は下流 layer)。M5 gcpm-directive-emit static-side
    /// (raikiri-spike-m5.1)、CSS Content 3 §1
    /// <https://www.w3.org/TR/css-content-3/#content-property>。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone + inheritance walk stack
    /// entry clone + per-node write が **shallow (Arc bump only)** になる。
    /// `* { content: "<large>" }` × N element の O(N × M) memory blow-up を
    /// 単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.1 SEC HIGH)。
    ///
    /// **Consumer 向け** (bd raikiri-spike-q3f wall/umbrella formal declare):
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
    /// `<content-list>` は CSS Content 3 §2 (m5.1 で parser 実装済)。
    /// 名前解決と runtime string() 参照は下流 (raikiri-dom) 責務。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::Content`] と同じ理由 —
    /// `* { string-set: name "<large>" }` × N element 経路の同種 DoS を塞ぐ
    /// (raikiri-spike-d9y.1)。
    ///
    /// **Consumer 向け** (bd raikiri-spike-q3f wall/umbrella formal declare):
    /// [`Self::Content`] と同じく outer `Arc` は read-side は deref 透過、
    /// construct-side (`PropertyValue::StringSet(vec![(name, items)])`) のみ
    /// `PropertyValue::StringSet(Arc::new(vec![..]))` に書き換える。inner
    /// `Vec<ContentComponent>` は Arc 化しない (per-entry share の hit率 が
    /// 想定できないため、outer 単段で d9y.1 の攻撃経路を塞ぐ設計)。
    StringSet(Arc<Vec<(SmolStr, Vec<ContentComponent>)>>),
    /// `position: static | running(<custom-ident>)` — non-inherited、initial:
    /// `static`。M5 static-side ε (raikiri-spike-m5.4)。
    /// CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>。
    /// M5 scope では `running()` seed emit のみが下流に伝わる —
    /// `Static` は `apply_value` で no-op (先行 `running()` を上書き suppress
    /// する discriminant 用途、spec default に相当)。
    /// `relative` / `absolute` / `fixed` / `sticky` は M5+ scope 外、parser 段で drop。
    Position(PositionValue),
    /// `text-align: start | end | left | right | center | justify | match-parent
    /// | justify-all` — **inherited**、initial: [`TextAlign::Start`]
    /// (CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
    /// <https://www.w3.org/TR/css-text-3/#text-align-property>)。
    /// spec 上 shorthand (text-align-all + text-align-last) だが Sprint 12 seed
    /// では単一 field に保持 (**g04 (b) milestone subset**、longhand 分離は
    /// 後続 task で defer)。詳細は [`TextAlign`] doc-comment。
    TextAlign(TextAlign),
    /// `padding-top: <length-percentage [0,∞]>` — non-inherited、initial: `0`。
    /// CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical>。
    /// spec grammar `<length-percentage [0,∞]>` の non-negative constraint は
    /// `parse_padding_side` が parse-time enforce (負値は None 返し → declaration drop)、
    /// `auto` keyword は grammar に含まれないため `parse_length_value` の
    /// Dimension / Percentage arm fall-through で自然 reject。
    /// (raikiri-spike-0vv.6)
    PaddingTop(Length),
    /// `padding-right: <length-percentage [0,∞]>` — [`Self::PaddingTop`] と同 grammar。
    /// (raikiri-spike-0vv.6)
    PaddingRight(Length),
    /// `padding-bottom: <length-percentage [0,∞]>` — [`Self::PaddingTop`] と同 grammar。
    /// (raikiri-spike-0vv.6)
    PaddingBottom(Length),
    /// `padding-left: <length-percentage [0,∞]>` — [`Self::PaddingTop`] と同 grammar。
    /// (raikiri-spike-0vv.6)
    PaddingLeft(Length),
    /// `padding: <'padding-top'>{1,4}` shorthand — non-inherited、initial:
    /// `Sides::all(Length::Px(0.0))`。CSS Box 3 §4.2
    /// <https://www.w3.org/TR/css-box-3/#padding-shorthand>。
    ///
    /// 1-4 value expansion (spec-verbatim):
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
    /// に展開するため (1/2/3/4 expansion + CSS Cascading L5
    /// "Shorthand Properties" <https://www.w3.org/TR/css-cascade-5/#shorthand>
    /// verbatim "A shorthand property sets all of its longhand sub-properties,
    /// exactly as if expanded in place." 準拠、cascade の per-side 勝ち抜けが自然に
    /// 成立する)。expansion 経路の safety net として [`crate::cascade::apply_value`]
    /// は本 variant を受けたときも `ComputedValues.padding` field 全 4 side を
    /// 上書きする実装を持つ (regression 時 panic 回避)。
    /// raikiri-spike-5nc (margin 0vv.5 の parse-time expansion model に migrate)。
    Padding(Sides<Length>),
    /// `margin-top: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    /// raikiri-spike-0vv.5。
    MarginTop(LengthOrAuto),
    /// `margin-right: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    /// raikiri-spike-0vv.5。
    MarginRight(LengthOrAuto),
    /// `margin-bottom: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    /// raikiri-spike-0vv.5。
    MarginBottom(LengthOrAuto),
    /// `margin-left: <length-percentage> | auto` — non-inherited、initial: 0
    /// (CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>)。
    /// raikiri-spike-0vv.5。
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
    /// verbatim "A shorthand property sets all of its longhand sub-properties,
    /// exactly as if expanded in place." 準拠、cascade の per-side 勝ち抜けが自然に
    /// 成立する)。expansion 経路の safety net として [`crate::cascade::apply_value`]
    /// は本 variant を受けたときも `ComputedValues.margin` field 全 4 side を
    /// 上書きする実装を持つ (regression 時 panic 回避)。
    /// raikiri-spike-0vv.5。
    Margin(Sides<LengthOrAuto>),
    /// `border-top-width: <line-width>` — non-inherited、initial: `medium`
    /// = `Length::Px(3.0)` (CSS Backgrounds 3 §3.3
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。
    /// `<line-width>` = `<length [0,∞]> | thin | medium | thick`。
    /// `<percentage>` は grammar に含まれない (padding とは違う点、advisor
    /// calibration)。keyword mapping は spec 規定値: thin=1px、medium=3px、
    /// thick=5px (`parse_border_width_side` doc 参照)。
    /// (raikiri-spike-0vv.12)
    BorderTopWidth(Length),
    /// `border-right-width: <line-width>` — [`Self::BorderTopWidth`] と同 grammar。
    /// (raikiri-spike-0vv.12)
    BorderRightWidth(Length),
    /// `border-bottom-width: <line-width>` — [`Self::BorderTopWidth`] と同 grammar。
    /// (raikiri-spike-0vv.12)
    BorderBottomWidth(Length),
    /// `border-left-width: <line-width>` — [`Self::BorderTopWidth`] と同 grammar。
    /// (raikiri-spike-0vv.12)
    BorderLeftWidth(Length),
    /// `border-top-style: <line-style>` — non-inherited、initial: `none`
    /// (CSS Backgrounds 3 §3.2
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-style>)。10 keyword は
    /// [`BorderStyle`] variant を参照。
    /// (raikiri-spike-0vv.12)
    BorderTopStyle(BorderStyle),
    /// `border-right-style: <line-style>` — [`Self::BorderTopStyle`] と同 grammar。
    /// (raikiri-spike-0vv.12)
    BorderRightStyle(BorderStyle),
    /// `border-bottom-style: <line-style>` — [`Self::BorderTopStyle`] と同 grammar。
    /// (raikiri-spike-0vv.12)
    BorderBottomStyle(BorderStyle),
    /// `border-left-style: <line-style>` — [`Self::BorderTopStyle`] と同 grammar。
    /// (raikiri-spike-0vv.12)
    BorderLeftStyle(BorderStyle),
    /// `border-top-color: <color>` — non-inherited、initial: `currentcolor`
    /// keyword ([`BorderColor::CurrentColor`]、CSS Backgrounds 3 §3.1
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-color> "Initial:
    /// currentcolor")。cascade static side は [`BorderColor`] enum で
    /// specified value (currentcolor vs. resolved `<color>`) を保持し、
    /// used-value resolution (currentcolor → 同 node computed `color` property
    /// lookup、CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>)
    /// は paint scope 責務 (bd raikiri-spike-q7qf)。
    /// (raikiri-spike-0vv.12 initial seed、raikiri-spike-0vv.17 で
    /// `CssColor` から [`BorderColor`] へ格上げ)
    BorderTopColor(BorderColor),
    /// `border-right-color: <color>` — [`Self::BorderTopColor`] と同 grammar。
    /// (raikiri-spike-0vv.12、raikiri-spike-0vv.17)
    BorderRightColor(BorderColor),
    /// `border-bottom-color: <color>` — [`Self::BorderTopColor`] と同 grammar。
    /// (raikiri-spike-0vv.12、raikiri-spike-0vv.17)
    BorderBottomColor(BorderColor),
    /// `border-left-color: <color>` — [`Self::BorderTopColor`] と同 grammar。
    /// (raikiri-spike-0vv.12、raikiri-spike-0vv.17)
    BorderLeftColor(BorderColor),
    /// `border: <line-width> || <line-style> || <color>` shorthand — 4 side
    /// 全てに同一の [`Border`] を配る (CSS Backgrounds 3 §3.4
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>)。
    ///
    /// spec grammar は `||` (any-order、each component at most once、at least
    /// 1 必須) — `parse_border_shorthand` が unfilled slot loop で peel する。
    /// 省略成分は initial: width=`Length::Px(3.0)` (medium)、style=`BorderStyle::None`、
    /// color=[`BorderColor::CurrentColor`] (spec §3.1 initial、raikiri-spike-0vv.17)。
    ///
    /// **element cascade 段でこの variant は観測されない**:
    /// [`crate::rule::expand_shorthand_into`] が parse 出口
    /// (`parse_declaration_block`) と element cascade 入口
    /// ([`mod@crate::cascade`] の `collect_cascaded`) の両方で 12 longhand variant
    /// (4 side × 3 sub-property)
    /// に展開するため (spec CSS Cascading L5 §"Shorthand Properties"
    /// <https://www.w3.org/TR/css-cascade-5/#shorthand> verbatim "A shorthand
    /// property sets all of its longhand sub-properties, exactly as if expanded
    /// in place." 準拠、cascade の per-side / per-sub-property 勝ち抜けが自然に
    /// 成立する — margin / padding shorthand precedent 踏襲)。expansion 経路の
    /// safety net として [`crate::cascade::apply_value`] は本 variant を受けたときも
    /// `ComputedValues.border` field 全 4 side × 3 sub-property を上書きする
    /// 実装を持つ (regression 時 panic 回避)。
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
    /// border-image を milestone defer (未実装、bd raikiri-spike-0vv Epic の
    /// (b) milestone subset)。future 統合 task で border-image longhand と併せて
    /// 対応。
    /// (raikiri-spike-0vv.12)
    Border(Sides<Border>),
    /// `width: auto | <length-percentage [0,∞]>` — non-inherited、initial: `auto`
    /// (CSS Sizing 3 §3.1.1 <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
    ///
    /// spec value grammar は `auto | <length-percentage [0,∞]> | min-content |
    /// max-content | fit-content(<length-percentage>)` だが、min-content /
    /// max-content / fit-content() は g04 (b) milestone subset として本 milestone
    /// では silent drop、`auto` と non-negative `<length-percentage>` のみ受理。
    /// 負値は spec grammar `[0,∞]` violation として drop。
    ///
    /// `auto` の resolution は下流 layout (raikiri-dom apply_computed_to_style
    /// bridge、taffy::Style::size.width 反映) 責務。raikiri-spike-0vv.10。
    Width(LengthOrAuto),
    /// `height: <length-percentage [0,∞]> | auto` — **non-inherited**、initial:
    /// `auto` (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
    ///
    /// Sprint 17 seed scope (raikiri-spike-0vv.11) は `auto` + 非負
    /// `<length-percentage>` の 2 分岐のみ受理。`min-content` / `max-content` /
    /// `fit-content(<length-percentage>)` は spec-valid だが milestone subset
    /// (g04 (b)) として parser 段で silent drop する — `parse_height` doc 参照。
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
    /// (raikiri-spike-0vv.13)
    BoxSizing(BoxSizing),
    /// `direction: ltr | rtl` — **inherited**、initial: [`Direction::Ltr`]
    /// (CSS Writing Modes 4 §2.1 "Specifying Directionality: the direction
    /// property" <https://www.w3.org/TR/css-writing-modes-4/#direction>)。
    /// computed value = specified value (相対解決なし、[`Direction`] doc 参照)。
    /// 唯一の consumer は [`resolve_text_align_match_parent`] だが、property
    /// 自体は CSS Paged Media 3 Appendix A page-property-list にも独立に
    /// 現れる ([`Direction`] doc の verbatim 確認済み引用参照)。
    /// (raikiri-spike-l3wg、末尾に追加 — 既存 variant の discriminant を
    /// shift させないための配置、[`PropertyKey`] doc の「宣言順は load-bearing」
    /// 節参照)
    Direction(Direction),
}

/// Property key (cascade で "同一 property を勝ち取る" ための discriminant)。
///
/// cascade.rs の per-node winner selection、および page.rs の
/// [`cascade_page`](crate::page::cascade_page) が [`PageCascadeResult`] の
/// map key に使う。`PropertyValue` の variant tag を stateless に抜き出したもので
/// 追加情報を持たないため public に露出する (raikiri-spike-m4.1、[`PageCascadeResult`]
/// が `pub` 型を要求するため — clippy `private_interfaces` 対応)。
///
/// # ⚠️ variant の**宣言順は load-bearing** (bd raikiri-spike-8kn8)
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
///   cascade 段には到達しない** (bd raikiri-spike-nqkj)。`@page` cascade
///   ([`crate::page::cascade_page`]) も入口側で同じ展開を通すので、`PageRule` の
///   `pub declarations` を post-parse mutation された場合の同 shape の gap も
///   塞がっている (bd raikiri-spike-3svx)。
///
/// **並び順を「直す」ことで shorthand/longhand の cascade を修正しようとしない
/// こと** — 順序任せの解は `margin: 0; margin-top: 10px` と
/// `margin-top: 10px; margin: 0` という鏡像 2 例のうち必ず片方を壊す
/// (詳細は `apply_winners` の doc)。正しい解は既に採られている
/// 「shorthand を cascade 段に到達させない」方向であり、その展開 arm の
/// 書き忘れは [`crate::rule::expand_shorthand_into`] の exhaustive match により
/// compile-time に排除されている (bd raikiri-spike-ez7b)。
///
/// 新しい variant を足すときは、それが既存 variant と同じ `SpecifiedValues`
/// field に書くかどうかを確認すること。書かないなら (= 1:1 disjoint なら)
/// 位置は自由でよい。
///
/// [`Direction`] / [`TextAlign`] は 1:1 disjoint (`SpecifiedValues::direction`
/// / `SpecifiedValues::text_align` の別 field) — `text-align: match-parent`
/// が `direction` の**親**の computed 値を要する件 (raikiri-spike-l3wg) は
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
    /// 到達せず (bd raikiri-spike-nqkj)、observable な divergence は無い
    /// (`@page` 経路の同 shape gap も [`crate::page::cascade_page`] の入口側展開で
    /// 塞がれている — bd raikiri-spike-3svx)。その担保のうち「展開 arm の
    /// 書き忘れ」は [`crate::rule::expand_shorthand_into`] の exhaustive match により
    /// compile-time に排除されている (bd raikiri-spike-ez7b)。
    Padding,
    // margin longhand + shorthand — raikiri-spike-0vv.5 (semantics on the
    // matching PropertyValue::Margin* variants; sibling PropertyKey variants
    // carry no per-variant docs per crate convention).
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Margin,
    // border longhand + shorthand — raikiri-spike-0vv.12 (semantics on the
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
    // width — raikiri-spike-0vv.10 (CSS Sizing 3 §3.1.1)。
    Width,
    // height — raikiri-spike-0vv.11 (CSS Sizing 3 §3.1.1、semantics on the
    // matching PropertyValue::Height variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention).
    Height,
    // box-sizing — raikiri-spike-0vv.13 (CSS Sizing 3 §3.3、semantics on the
    // matching PropertyValue::BoxSizing variant; sibling PropertyKey variants
    // carry no per-variant docs per crate convention).
    BoxSizing,
    // direction — raikiri-spike-l3wg (CSS Writing Modes 4 §2.1、semantics on
    // the matching PropertyValue::Direction variant; sibling PropertyKey
    // variants carry no per-variant docs per crate convention). 末尾配置の
    // 理由は PropertyValue::Direction の doc 参照。
    Direction,
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
        // (raikiri-spike-0vv.7)
        "background-color" => parse_color(input).map(PropertyValue::BackgroundColor),
        // Arc wrap は raikiri-spike-no7b の cascade memory 削減 (d9y.1/d9y.2
        // pattern の踏襲、SEC 分類ではない perf lens)。`parse_font_family` は
        // grammar 上 empty Vec を返さない (`<family-name>#` は 1 要素以上必須、
        // 同関数の `if families.is_empty() { None }` 参照) ため、counter-* /
        // content / string-set と異なり shared-empty-slot 分岐は不要。
        "font-family" => parse_font_family(input).map(|v| PropertyValue::FontFamily(Arc::new(v))),
        "font-size" => parse_font_size(input),
        "font-weight" => parse_font_weight(input).map(PropertyValue::FontWeight),
        // CSS Inline 3 §5.1 line-height (raikiri-spike-0vv.9)。
        // `normal` / `<number [0,∞]>` / `<length-percentage [0,∞]>` を受理、
        // 負値と其他 keyword は spec grammar 違反として drop。
        "line-height" => parse_line_height(input).map(PropertyValue::LineHeight),
        "display" => parse_display(input).map(PropertyValue::Display),
        // CSS Lists 3 §4 counter properties (raikiri-spike-s85、M5 pre-work)。
        // spec default: reset = 0、increment = 1、set = 0。
        // Arc wrap は raikiri-spike-d9y.2 の cascade memory DoS fix (per-element
        // clone を shallow bump 化)、空 list は 3 property 共通 shared Arc slot
        // (`empty_counter_entries`) に落として per-node allocation regression を
        // 避ける (d9y.1 Content/StringSet precedent と同 pattern)。
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
        // CSS Content 3 §1 content property (raikiri-spike-m5.1、M5 gcpm-directive-emit static side)。
        // Arc wrap は raikiri-spike-d9y.1 の cascade memory DoS fix (per-element clone を
        // shallow bump 化)、empty list は shared Arc slot に落として per-node allocation
        // regression を避ける (advisor calibration)。
        "content" => parse_content(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Content(empty_content_list())
            } else {
                PropertyValue::Content(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.1.1 string-set (raikiri-spike-m5.3、M5 static-side β)。
        // Arc wrap は raikiri-spike-d9y.1、同 rationale。
        "string-set" => parse_string_set(input).map(|v| {
            if v.is_empty() {
                PropertyValue::StringSet(empty_string_set_entries())
            } else {
                PropertyValue::StringSet(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.2.1 position: running() (raikiri-spike-m5.4、M5 static-side ε)。
        // M5 scope では `static` + `running(<custom-ident>)` のみ受理、
        // `relative` / `absolute` / `fixed` / `sticky` は silent drop (M5+ scope 外)。
        "position" => parse_position(input).map(PropertyValue::Position),
        // CSS Text 3 §6.1 text-align (raikiri-spike-0vv.8、Sprint 12 seed)。
        // spec 上 shorthand (text-align-all + text-align-last) だが単一 field で保持
        // (g04 (b) milestone subset、[`TextAlign`] doc-comment 参照)。
        "text-align" => parse_text_align(input).map(PropertyValue::TextAlign),
        // CSS Box 3 §4.1 padding physical longhand (raikiri-spike-0vv.6)。
        // grammar: <length-percentage [0,∞]> — non-negative constraint は
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
        // CSS Box 3 §3.1 margin-* physical longhand (raikiri-spike-0vv.5).
        // <length-percentage> | auto の grammar、negative 許容 (spec 準拠、layout
        // 側で負値の意味付け)。
        "margin-top" => parse_margin_side(input).map(PropertyValue::MarginTop),
        "margin-right" => parse_margin_side(input).map(PropertyValue::MarginRight),
        "margin-bottom" => parse_margin_side(input).map(PropertyValue::MarginBottom),
        "margin-left" => parse_margin_side(input).map(PropertyValue::MarginLeft),
        // CSS Box 3 §3.2 margin shorthand (raikiri-spike-0vv.5). 1-4 value
        // expansion。cascade 段では `PropertyValue::Margin` は `parse_declaration_block`
        // 内で 4 longhand に展開されるため通常観測しない (詳細は
        // `PropertyValue::Margin` doc + `crate::rule::expand_shorthand_into`)。
        "margin" => parse_margin_shorthand(input).map(PropertyValue::Margin),
        // CSS Backgrounds 3 §3.3 border-width physical longhand
        // (raikiri-spike-0vv.12)。grammar: `<line-width>` = `<length [0,∞]> |
        // thin | medium | thick`。`<percentage>` は含まれない (advisor
        // calibration、padding とは違う点)。keyword mapping は spec 規定値:
        // thin=1px、medium=3px、thick=5px。負値は spec grammar 違反 → drop
        // (`parse_border_width_side` が enforce)。
        "border-top-width" => parse_border_width_side(input).map(PropertyValue::BorderTopWidth),
        "border-right-width" => parse_border_width_side(input).map(PropertyValue::BorderRightWidth),
        "border-bottom-width" => {
            parse_border_width_side(input).map(PropertyValue::BorderBottomWidth)
        }
        "border-left-width" => parse_border_width_side(input).map(PropertyValue::BorderLeftWidth),
        // CSS Backgrounds 3 §3.2 border-style physical longhand
        // (raikiri-spike-0vv.12)。grammar: `<line-style>` = 10 alternative
        // (none / hidden / dotted / dashed / solid / double / groove / ridge /
        // inset / outset)。他 keyword は silent drop。
        "border-top-style" => parse_border_style_side(input).map(PropertyValue::BorderTopStyle),
        "border-right-style" => parse_border_style_side(input).map(PropertyValue::BorderRightStyle),
        "border-bottom-style" => {
            parse_border_style_side(input).map(PropertyValue::BorderBottomStyle)
        }
        "border-left-style" => parse_border_style_side(input).map(PropertyValue::BorderLeftStyle),
        // CSS Backgrounds 3 §3.1 border-color physical longhand
        // (raikiri-spike-0vv.12 initial seed、raikiri-spike-0vv.17 で
        // `parse_border_color` 経由に格上げ)。grammar: `<color>` に加え
        // `currentcolor` keyword を先取り (CSS Color 3 §4.4)。`BorderColor` enum
        // で specified value distinction を保持し、used-value resolution は
        // paint scope 責務 (bd raikiri-spike-q7qf)。
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
        // CSS Sizing 3 §3.1.1 preferred size property (raikiri-spike-0vv.10)。
        // grammar: `auto | <length-percentage [0,∞]> | min-content | max-content
        // | fit-content(<length-percentage>)` のうち `auto` + non-negative
        // `<length-percentage>` のみ受理、min-content / max-content / fit-content()
        // は g04 (b) milestone subset として silent drop、負値は spec `[0,∞]`
        // violation として drop (parse_width が enforce)。
        "width" => parse_width(input).map(PropertyValue::Width),
        // CSS Sizing 3 §3.1.1 preferred size — height (raikiri-spike-0vv.11)。
        // grammar: `auto | <length-percentage [0,∞]>` + spec-valid だが本 milestone
        // scope 外の `min-content` / `max-content` / `fit-content()` は silent drop
        // (parse_height 内で ident branch が auto のみ受理して他 keyword 落とし)。
        "height" => parse_height(input).map(PropertyValue::Height),
        // CSS Sizing 3 §3.3 box-sizing (raikiri-spike-0vv.13)。
        // value grammar `content-box | border-box`、initial `content-box`、
        // not inherited、computed value = specified keyword。
        "box-sizing" => parse_box_sizing(input).map(PropertyValue::BoxSizing),
        // CSS Writing Modes 4 §2.1 direction (raikiri-spike-l3wg)。
        // value grammar `ltr | rtl`、initial `ltr`、inherited、
        // computed value = specified keyword ([`Direction`] doc 参照)。
        "direction" => parse_direction(input).map(PropertyValue::Direction),
        _ => None,
    }
}

/// `<color>` を parse する。
///
/// cssparser 0.37 は (0.36 までと異なり) 汎用 `Color` enum / `Color::parse` を
/// 提供しない — それは別 crate `cssparser-color` 側に移った。ここでは
/// 各 form の parse を自前 (cleanroom) で組み立て、hex / named / rgb() の
/// 3 形式をカバーする (m1.4 scope):
///
/// - **Hex** (`#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa`) は
///   [`CssColor::from_hex`] を呼び出す — CSS Color 4 §5.2 準拠の cleanroom 実装。
///   `Token::Hash` / `Token::IDHash` の payload は leading `#` を含まないため
///   そのまま渡す (raikiri-spike-0vv.14)。
/// - **Named color** は `parse_named_color` (Sprint 12 style-7 precedent、
///   Non-goals defer 対象の 140+ CSS Color L3 keyword table を再実装しない
///   ため cssparser の table を暫定利用)。
/// - **`rgb()` / `rgba()` function form** は [`parse_rgb_function`] で
///   `parse_nested_block` 経由の手動 parse。
///
/// `transparent` keyword は CSS Color 4 §6.3 "The transparent keyword"
/// <https://www.w3.org/TR/css-color-4/#transparent-color> で
/// `rgba(0, 0, 0, 0)` の shorthand と規定される — `parse_named_color` の
/// (r, g, b) は alpha を返さないため、Ident arm 手前で明示 branch して
/// [`CssColor::TRANSPARENT`] を返す (raikiri-spike-0vv.7)。
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
/// cssparser の `parse_named_color` (RGB triple mapping、Sprint 12 precedent)
/// 経由のため `currentcolor` は named-color table 未収載として `None` 側に
/// 落ちる — 本 helper が Ident 段で先取りする必要がある。resolution 委譲の
/// rationale は [`BorderColor`] enum doc 参照 (bd raikiri-spike-q7qf paint
/// scope 責務)。
///
/// 5 call site (4 longhand + [`parse_border_shorthand`] color slot) が本
/// helper を経由する (37n sibling-arm convention consistency)。
///
/// (raikiri-spike-0vv.17)
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
/// # Milestone subset (g04 category (b))
///
/// - Modern (space + slash) syntax `rgb(R G B / A)` は本 task 対象外。legacy
///   と modern の mix は spec で禁止だが、本 helper は最初の channel の直後で
///   `expect_comma` を要求するため modern syntax は fall-through で reject。
/// - Fractional number channel (`rgb(127.5, 0, 0)`) は spec grammar 上 valid
///   だが、`expect_integer` (整数 `int_value` 必須) を採用しているため drop
///   — category (b) milestone subset、future task で `<number>` に緩める余地。
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
/// `allow_percentage` で受理集合を分岐:
/// - `false` → `<length>` mode: dimension unit のみ受理 (`px` / `em` / `rem` / `pt`)。
/// - `true` → `<length-percentage>` mode: 上記 4 unit + `%` token を受理。
///
/// 未対応 unit (`vw` / `vh` / `ch` / `ex` / `cm` / `mm` / `in` / `pc` / `Q` /
/// `cap` / `rcap` / `ic` / `ric` / `lh` / `rlh`) は spec-valid だが本 milestone
/// scope 外 (g04 category (b) milestone subset、defer 先 Sprint 13+ style backlog、
/// 未起票 — Epic 1 planner 判定)。
///
/// # Unitless zero
///
/// CSS Values 3 §5 "Distance Units: the `<length>` type"
/// <https://www.w3.org/TR/css-values-3/#lengths> 原文: "For zero lengths the
/// unit identifier is optional (i.e. can be syntactically represented as the
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
/// forward-provisioning として導入した mode だが、現在は 6 caller が使用する:
/// [`parse_margin_side`] / [`parse_padding_side`] / [`parse_width`] /
/// [`parse_height`] / [`parse_line_height`] / [`parse_font_size`]。いずれも
/// grammar が spec で `<length-percentage>` を含む
/// (bd raikiri-spike-0vv.5 / .6 / .9 / .10 / .11、`font-size` は
/// bd raikiri-spike-zls8 で `<length>` 限定から拡張)。共通 helper 化により
/// 重複 dimension unit dispatch を回避している。
///
/// `allow_percentage=false` (= `<length>` mode) の caller は
/// [`parse_border_width_side`] のみ — CSS Backgrounds 3 §3.3 の
/// `<line-width>` grammar が `<percentage>` を含まないため。
///
/// # Percentage overflow (bd raikiri-spike-3gee)
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
/// bd raikiri-spike-2ui0 の sink-guard precedent (「guard は sink 境界に
/// 置く、parse/resolve 層には置かない」) はここには適用しない —
/// bd raikiri-spike-3gee の判断: 本件は guard ではなく変換の正確さの問題
/// (specified 層の値そのものが CSS Values 4 §5 の要求から外れている)
/// であり、precedent とは別軸。`raikiri-dom::layout::sanitize_finite`
/// (resolve 後の geometry に対する sink guard) は本変更後も引き続き必要。
///
/// **`NaN` はこの saturation の対象外**(reviewer:spec 指摘、agent
/// a4beb897ae3457dcd, CONFIRMED medium)。`is_finite()` は `NaN` に対しても
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
            // Additional font-relative units (CSS Values 4 §6.1.1) —
            // bd raikiri-spike-2x8. `ex`/`ch`/`ic` の real-metric variant は
            // style 層に font metrics が無いため常に spec fallback を使う
            // ([`Length::Ex`] / [`Length::Ch`] / [`Length::Ic`] の doc 参照)。
            "ex" => Some(Length::Ex(*value)),
            "rex" => Some(Length::Rex(*value)),
            "ch" => Some(Length::Ch(*value)),
            "rch" => Some(Length::Rch(*value)),
            "ic" => Some(Length::Ic(*value)),
            "ric" => Some(Length::Ric(*value)),
            // Additional absolute units (CSS Values 4 §6.2) —
            // bd raikiri-spike-2x8. `unit` は `to_ascii_lowercase()` 済 —
            // `Q` トークンも `"q"` として届く。
            "cm" => Some(Length::Cm(*value)),
            "mm" => Some(Length::Mm(*value)),
            "q" => Some(Length::Q(*value)),
            "in" => Some(Length::In(*value)),
            "pc" => Some(Length::Pc(*value)),
            // `lh` / `rlh` (CSS Values 4 §6.1.1) — bd raikiri-spike-vxha.
            // Accepted generally here for every consumer, `font-size` included
            // (bd raikiri-spike-yh3w lifted `parse_font_size`'s former
            // post-filter — see that function's doc "`lh` / `rlh` は受理し、
            // 親基準で解決する" section for the self-reference resolution).
            "lh" => Some(Length::Lh(*value)),
            "rlh" => Some(Length::Rlh(*value)),
            // (b) milestone subset — viewport-relative unit (`vw`/`vh`/…) と
            // `cap`/`rcap` は未対応、silent drop。両者とも specified 層だけ
            // では正しく resolve できない (viewport size / font ascent が
            // style 層に存在しない) ため follow-up bd issue へ spinout 済
            // (bd raikiri-spike-wnpb、raikiri-spike-2x8 discovered-from)。
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
/// `parse_border_width_side` / `parse_height` / `parse_line_height` は grammar
/// の `[0,∞]` non-negative constraint を "全 variant の payload を取り出して
/// `>= 0.0` を確認" という同一 pattern で parse-time enforce する
/// (`parse_length_value` 自体は sign check しない仕様 — 同関数の "Sign / range"
/// doc 参照)。
///
/// 本 helper 導入前は 6 call site それぞれが `Length::Px(v) | Length::Em(v) |
/// … => v` の OR-pattern を個別に持っていた。bd raikiri-spike-2x8 で
/// [`Length`] が 5 → 16 variant に増える際、6 site 全てを手で拡張すると
/// 1 か所でも変数を書き漏らした variant が非負チェックを素通りする
/// (実際 2 site — `parse_border_width_side` / `parse_line_height` — は
/// `_ => None` catch-all を持っていたため、拡張漏れは compile error にならず
/// 黙って新 unit を reject し続ける fail-quiet になっていた)。本 helper は
/// **exhaustive match を 1 か所に集約**することで、新 variant 追加時に
/// compile error で全 call site の見直しを強制する — bd raikiri-spike-ier4
/// §4.5 が指摘した「拡張のたびに N site 分の負債が乗る」パターンをこの関数の
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
/// [`crate::rule::DeclParser`] の [`cssparser::DeclarationParser::parse_value`]
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
/// # Milestone subset (g04 (b))
///
/// `min-content` / `max-content` / `fit-content()` は intrinsic sizing keyword
/// で Epic 未着手 — 本 helper では受理せず自然に `None` に落ちる (`auto` ident
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
    // (padding と同 pattern、[`length_payload`] 経由、raikiri-spike-0vv.6
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
/// - `math` — g04 category (b) milestone subset (bd raikiri-spike-0vv.18、
///   MathML scaling algorithm が丸ごと未実装) として `None` に落とす。
///
/// # ident 分岐を先に `try_parse` する理由
///
/// `<absolute-size>` / `<relative-size>` / `math` はいずれも単一 ident token。
/// [`parse_margin_side`] の `auto` 分岐と同じ pattern — [`parse_length_value`]
/// は内部で `input.next()` を unconditional に消費するため、ident 分岐は
/// checkpoint 経由の rewind (`try_parse`) で先に試す必要がある。
///
/// # `em` / `rem` / `%` / `pt` を受理するようになった経緯 (bd raikiri-spike-zls8)
///
/// Sprint 18 までは `px` 以外を post-filter で drop していた。理由は「font-size
/// context resolve 未実装」であり、その resolve が decision raikiri-spike-082k
/// Phase 2 で実装された — cascade が phase 2 で
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
/// # `lh` / `rlh` は受理し、親基準で解決する (bd raikiri-spike-yh3w)
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
/// bd raikiri-spike-vxha の時点では、この解決 (「親の computed line-height」を
/// font-size 解決の基準として渡す) が `line-height`
/// (`finalize`/`finalize_as_root` が既に持つ `parent: &ComputedValues` を
/// そのまま使える) より高コストに見えたため drop していたが、実際に実装した
/// ところコストは局所的だった — [`crate::resolve::resolve_font_size`] の
/// `self_reference_basis` 引数、および [`crate::specified::SpecifiedValues::finalize`]
/// 内の 2, 3 行の並べ替えで足りる (`parent` は本関数の呼び出しに入る前に
/// tree walk で既に確定済みのため、cross-node な phase 順序の変更は不要 —
/// [`mod@crate::resolve`] module doc の「想定される 4 段階」節、
/// bd raikiri-spike-yh3w 完了報告参照)。
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
/// <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping> 原文の表を
/// **そのまま**写す (`resolve_relative_weight` の "算術式で書いてはいけない"
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
/// bd raikiri-spike-0vv.18 (g04 category (b))。
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
        // `math` はここに落ちる (spec-valid だが g04 (b) 未対応、bd raikiri-spike-0vv.18)。
        // 未知 ident も同じく drop。
        _ => return None,
    };
    Some(PropertyValue::FontSize(Length::Px(px)))
}

/// `padding-{top,right,bottom,left}` の single-side value を parse する。
///
/// grammar: `<length-percentage [0,∞]>` (CSS Box 3 §4.1
/// <https://www.w3.org/TR/css-box-3/#padding-physical>)。spec 原文:
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
/// identical)。`parse_font_size` は raikiri-spike-4rmu で `<absolute-size>` /
/// `<relative-size>` / `math` の ident 分岐 (`parse_font_size_keyword`) が
/// 前段に付いたため関数全体としては同形ではなくなったが、この tail 部分の
/// ロジックは identical。
///
/// 両者が非対称だった時期 (font-size が `<length>` px-only milestone で、padding
/// だけが `<length-percentage>` の 5 variant を受けていた頃) の記述は
/// bd raikiri-spike-zls8 の font-relative unit 対応で解消済み。
fn parse_padding_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid."。
    (length_payload(length) >= 0.0).then_some(length)
}

/// `padding: <'padding-top'>{1,4}` shorthand を [`Sides<Length>`] に expand する。
///
/// CSS Box 3 §4.2 <https://www.w3.org/TR/css-box-3/#padding-shorthand>: spec-verbatim
/// 1-4 value expansion:
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
    // spec (CSS Box 3) §4.2 1-4 value expansion (verbatim):
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

/// `border-{top,right,bottom,left}-width` の single-side value を parse する。
///
/// Grammar: `<line-width>` = `<length [0,∞]> | thin | medium | thick`
/// (CSS Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。
/// **`<percentage>` は含まれない** — padding とは違う (advisor calibration、
/// `parse_length_value(input, false)` = `<length>` mode を渡す)。
///
/// # Keyword mapping (spec 規定値)
///
/// spec §3.3 は 3 keyword を normative に規定する — verbatim "The thin,
/// medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively":
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
/// # Non-goals (g04 3-category labels)
///
/// - **(a) spec-invalid → drop**: 負値 (`-1px`)、未知 keyword (`fat` 等)、
///   spec-invalid unit (`%` は grammar に含まれない → drop)。
/// - **(b) milestone subset**: CSS-wide keyword (`inherit` / `initial` /
///   `unset` / `revert` / `revert-layer`) は Epic 7、silent drop。
/// - **(b) milestone subset**: `calc()` / `var()` は Epic 5、silent drop。
///
/// (raikiri-spike-0vv.12)
fn parse_border_width_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    // 1. keyword branch (thin / medium / thick) を先に try — `parse_length_value`
    //    は unconditional に token を consume するため、`try_parse` で rewind を
    //    確保する必要がある (sibling `parse_margin_side` の `auto` branch と同
    //    pattern)。
    let keyword = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.clone();
        match ident.to_ascii_lowercase().as_str() {
            "thin" => Ok(Length::Px(1.0)),
            "medium" => Ok(Length::Px(3.0)),
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
    // spec `<length [0,∞]>` の non-negative constraint — [`length_payload`] は
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
/// # Non-goals (g04 3-category labels)
///
/// - **(a) spec-invalid → drop**: 未知 keyword (`wavy` 等) は silent drop。
/// - **(b) milestone subset**: CSS-wide keyword は Epic 7、silent drop。
///
/// (raikiri-spike-0vv.12)
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
/// spec §3.4 verbatim "Omitted values are set to their initial values":
/// - width 省略 → `Length::Px(3.0)` (medium initial)
/// - style 省略 → `BorderStyle::None` (initial、spec §3.2)
/// - color 省略 → [`BorderColor::CurrentColor`] (spec §3.1 initial、used-value
///   resolution は paint scope 責務、raikiri-spike-0vv.17)
///
/// # Non-goals (spec deviation 明示)
///
/// spec §3.4 では border shorthand が **border-image-* も reset** する (spec
/// verbatim "The border shorthand also resets border-image to its initial
/// value.") が、本 crate は border-image を milestone defer で実装しないため
/// reset side effect を省略。
/// bd raikiri-spike-0vv (Epic) の border-image longhand 実装時に統合する。
///
/// # Sibling pattern
///
/// [`parse_margin_shorthand`] / [`parse_padding_shorthand`] は `{1,4}`
/// multiplier (順序固定、side ごとに違う値) だが、本 shorthand は `||` (any-order、
/// side は 4 side 共通) — 別 pattern。sibling は `try_parse` 経由の rewind と
/// initial fill の点で共通 principle を持つ。
///
/// (raikiri-spike-0vv.12)
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

        // color slot — [`parse_border_color`] を reuse。hex / named / rgb(a) /
        // transparent の全 alternative + `currentcolor` keyword (CSS Color 3
        // §4.4) を受理。4 longhand parse site (border-{top,right,bottom,left}-color)
        // と同じ helper を経由することで 37n sibling convention consistency を
        // 担保 (raikiri-spike-0vv.17)。
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
        width: width.unwrap_or(Length::Px(3.0)), // medium
        style: style.unwrap_or(BorderStyle::None),
        // §3.1 initial "currentcolor" — used-value resolution は paint scope
        // 責務 (bd raikiri-spike-q7qf、raikiri-spike-0vv.17)。
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
/// # Scope carving (g04 3-category)
///
/// - **(a) spec-invalid → drop**: 負値 (`height: -10px`) は grammar `[0,∞]` 違反、
///   全 [`Length`] variant の payload に対し `>= 0.0` post-filter で reject
///   ([`parse_padding_side`] の非負フィルタ pattern と同 shape)。
/// - **(b) milestone subset — 未対応 sizing keyword**: `min-content` /
///   `max-content` / `fit-content(<length-percentage>)` は Sprint 17 seed scope
///   外、silent drop (auto ident branch から外れる他 keyword は
///   `expect_ident_matching("auto")` が失敗 → length parser の Dimension /
///   Percentage arm でも受理されず None に落ちる)。
/// - **(b) milestone subset — CSS-wide keyword**: `inherit` / `initial` /
///   `unset` / `revert` / `revert-layer` / `all` は Epic 7、silent drop
///   (同 ident 経路で他 keyword と同じく落ちる)。
/// - **calc() / var()**: Epic 5 対象、本 task scope 外
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
    // spec §3.1.1: <length-percentage [0,∞]>。負値 → drop (parse_padding_side
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
/// g04 category (a) spec-invalid → drop: spec grammar が range を parse-time
/// で制約するため、reject 自体が spec 準拠 (下段 Non-goals arm と同 label)。
///
/// # Non-goals (g04 3-category labels)
///
/// - **(b) milestone subset**: global keyword (`inherit` / `initial` / `unset` /
///   `revert` / `revert-layer`) は Epic 7 対象、silent drop
/// - **(b) milestone subset**: `calc()` / `var()` は Epic 5 (css-variables-and-math)
///   対象、silent drop
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
    //    (spec-invalid CSS を通す correctness bug、raikiri-spike-0vv.9 quality)。
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
///   が親の computed weight から解決する (raikiri-spike-17s8)。
///
/// ASCII case-insensitive matching は CSS Values 3 §3.1 "Pre-defined Keywords"
/// <https://www.w3.org/TR/css-values-3/#keywords> 準拠 (37n sibling
/// `parse_display` / `parse_content_*` と同 convention)。
///
/// # Range (g04 category (a) — spec grammar)
///
/// spec §2.2: "Only values greater than or equal to 1, and less than or equal
/// to 1000, are valid, and all other values are invalid"。したがって `0` /
/// `1001` / `-100` の reject は **spec grammar そのもの** であり、stricter
/// policy ではない。範囲判定は **丸める前の指定値** に対して行う (spec の
/// "values" は author が書いた `<number>` を指すため、`0.6` や `1000.4` は
/// 丸めれば範囲内になるが invalid)。
///
/// # Fractional weight は丸めずそのまま保持する (bd raikiri-spike-e52s で解消)
///
/// **spec は fraction を落としてよいとは述べていない。** §2.2 の property table
/// は `Computed value: a number, see below` と規定し、§2.2.2 "Missing weights"
/// <https://www.w3.org/TR/css-fonts-4/#missing-weights> は "Fractional weights
/// are valid" と明言する。WPT `css/css-fonts/parsing/font-weight-computed.html`
/// の `test_computed_value('font-weight', '150.25')` (2-arg 形 = computed ==
/// specified) がこれを直接 pin している。
///
/// payload ([`FontWeightValue::Absolute`]) と
/// [`crate::computed::ComputedValues::font_weight`] は共に `f32` (bd
/// raikiri-spike-e52s で `u16` から格上げ) なので、parse 時に整数化する必要が
/// ない — `<number>` の `value` をそのまま保持する。旧実装は computed side が
/// `u16` だったため round-half-away-from-zero で整数化しており、その丸めが
/// §2.2.1 "Relative Weights" relative-weight table の*行選択*を変える 2 次被害
/// があった (親 `font-weight: 349.5` + 子 `bolder` が旧実装では 350 への丸め後
/// `350 <= w < 550` 行 → 700 に化け、spec の `100 <= w < 350` 行 → 400
/// と食い違う。`549.5` + `bolder`、`749.5` + `lighter` も同型 — pin:
/// [`crate::cascade::tests::bolder_lighter_resolve_against_unrounded_fractional_parent_weight`])。
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

/// `display: <ident>` を parse する。
///
/// CSS Display 3 §2 "Box Layout Modes: the display property"
/// <https://www.w3.org/TR/css-display-3/#propdef-display>。Sprint 12 scope
/// (raikiri-spike-0vv.4) では 4 keyword を受理:
///
/// - `block` — `<display-outside>` (block flow)
/// - `inline` — `<display-outside>` (inline flow、initial value)
/// - `inline-block` — `<display-legacy>` (inline flow-root)
/// - `none` — `<display-box>` (subtree omitted from box tree)
///
/// 他 keyword (`flex` / `grid` / `table*` / `list-item` / `flow-root` /
/// `contents` 等) は spec-valid だが milestone defer で silent drop
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
/// # Scope carving (g04 3-category、[`BoxSizing`] doc-comment に詳述)
///
/// - **(b) milestone subset**: CSS-wide keyword (`inherit` / `initial` /
///   `unset` / `revert` / `revert-layer`) は Epic 7 で silent drop。
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
/// # Scope carving (g04 3-category、[`TextAlign`] doc-comment に詳述)
///
/// - **(b) milestone subset**: `<string>` value は silent drop。CSS Text 3
///   §6.1 の grammar には無く、CSS Text 4 §7.1
///   <https://www.w3.org/TR/css-text-4/#text-align-property> で追加された
///   alternative (semantics は同 §7.2 "Character-based Alignment in a Table
///   Column")。CSS-wide keyword (`inherit` 等) も Epic 7 で silent drop。
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
/// # Scope carving (g04 3-category、[`Direction`] doc-comment に詳述)
///
/// - **(b) milestone subset**: CSS-wide keyword (`inherit` 等) は Epic 7、
///   silent drop。
/// - **(a) spec-invalid**: `ltr` / `rtl` 以外の ident は silent drop = `None`。
fn parse_direction(input: &mut Parser<'_, '_>) -> Option<Direction> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "ltr" => Some(Direction::Ltr),
        "rtl" => Some(Direction::Rtl),
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
/// 変異させる)。counter-* precedent (raikiri-spike-s85) と同じ shape。
///
/// items+ loop は `<string>` literal と function token (`counter(...)` 等) を
/// 順次 peel する。認識できない token に当たった時点で loop を break、caller
/// の `expect_exhausted` (rule.rs) が leftover を検知して declaration ごと drop。
///
/// `alt text` (spec `... [/ <string>...]?`) は M5 pre-work scope 外、`/` 以降は
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
///   を受理 (raikiri-spike-1us で `<image>` / `contents` / `<quote>` /
///   `leader()` を追加)。
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
/// (raikiri-spike-m5.1 で導入、raikiri-spike-6s1 で mode-parameterize、
/// raikiri-spike-1us で `<image>` / `contents` / `<quote>` / `leader()` 追加)
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
        // (`url(...)` / `url("...")`) のみ (`<gradient>` は milestone subset
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
/// ## Entry separator の strict 化 (raikiri-spike-1ll)
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
/// [`parse_optional_counter_style`] (raikiri-spike-zik) と同 principle。
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
        // top-level alternative の `none` reject を組み合わせる (m5.1 の
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
        // を受理しない (raikiri-spike-6s1、詳細は `ContentListMode` doc)。
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
/// `none` はここでは除外しない。spec 原文が "Specifications using `<custom-ident>`
/// must specify clearly what other keywords are excluded from `<custom-ident>`,
/// if any…" と述べるとおり、より狭い grammar (`<counter-name>` 等) の追加除外は
/// 個別の predicate (例 [`is_reserved_counter_name`]) 側の責務。
/// [`is_reserved_custom_ident`] の docstring も参照。
///
/// **呼び出し元は当初 3 箇所**: `string()` の name 引数 ([`parse_string_fn`])、
/// `target-counter()` / `target-counters()` の第 2 引数
/// ([`parse_target_counter_fn`] / [`parse_target_counters_fn`])。いずれも spec 上
/// `<custom-ident>` を取り `none` は valid。
///
/// bd raikiri-spike-r7r1 で `pub(crate)` に広げ、`counter_style` module が
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
/// `pub(crate)`: bd raikiri-spike-r7r1 の `counter_style` module が
/// `<counter-style-name>` 系 production (rule name / `fallback` / `system:
/// extends`) の除外 predicate を組み立てる際にこの base list を再利用する
/// ([`parse_custom_ident`] の doc 参照)。
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
/// spec 原文: "A `<counter-name>` name cannot match the keyword `none`; such an
/// identifier is invalid as a `<counter-name>`"。
///
/// counter() / counters() (§4.7) の first argument、および
/// counter-reset / counter-increment / counter-set property
/// (§4.1 / §4.2) の name 引数で使う。後者は既に [`parse_counter_property`] が
/// [`is_reserved_counter_name`]
/// 経由で reject 済 — 本 helper は前者を同じ predicate に揃えるための wrapper
/// (raikiri-spike-afv — codex final for m5.1)。
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
/// `?` propagation、raikiri-spike-zik で silent Decimal fallback を除去)。
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
/// M5 static-side scope: type / fallback (attr(x string, "default") 等) は defer。
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
/// M5 static-side ε (raikiri-spike-m5.4) の scope:
/// - `static` — [`PositionValue::Static`]、`inherit_from` の初期状態と一致するため
///   apply_value が no-op でも問題ない。cascade winner selection では
///   先行 `running(...)` を上書き suppress する identity 用途
///   (advisor calibration: standalone-static test だけでは実効性が問えない)。
/// - `running(<custom-ident>)` — [`PositionValue::Running`]、apply_value が
///   1-item `RunningTemplate` を computed.running_templates に seed する。
/// - 他 keyword (`relative` / `absolute` / `fixed` / `sticky`) は M5+ scope 外、
///   silent drop = `None`。
///
/// `<custom-ident>` の除外は m5.3 string-set と同じ規約:
/// [`is_reserved_custom_ident`] (CSS-wide keyword + `default`) に加えて
/// `none` を弾く。`none` は position property の他 spec-defined keyword
/// では無いが、custom-ident としては予約 alternative の慣行を残しつつ、
/// runtime resolve で `element(none)` 参照を誤って matching させないためのガード
/// (reviewer:spec interpretation point、m5.3 の `none` reject と同じ扱い)。
fn parse_position(input: &mut Parser<'_, '_>) -> Option<PositionValue> {
    // `static` は M5 scope で唯一受理する non-running keyword。
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

/// `content([ text | before | after | first-letter ]?)`。
/// CSS GCPM 3 §1.1.1.1 <https://www.w3.org/TR/css-gcpm-3/#funcdef-content>。
///
/// bare `content()` (spec 例 `h2 { string-set: heading content() }`) は
/// spec default `text` を意味する。target-text() の第 2 引数と違い、keyword は
/// paren 直下に置かれる (comma を先行させない)。
///
/// GCPM 3 §1.1.1 の narrow `<content-list>` (string-set 側) と CSS Content 3
/// §2 の broad `<content-list>` (content property 側) の **両方** に含まれる
/// 5 alt の 1 つのため、[`ContentListMode`] mode gate なし = 両 property 共通で
/// 受理される (raikiri-spike-6s1 で mode dispatch を導入した後もこの arm は
/// unconditional のまま)。
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
        // spec-invalid: `g` は hex digit ではない (g04 category (a) → drop)。
        assert_eq!(parse("#gggggg", "color"), None);
    }

    #[test]
    fn color_parse_hex_invalid_length_returns_none() {
        // spec-invalid: hex-notation grammar は 3/4/6/8 digit のみ。
        // 5-digit は spec に無い (g04 category (a) → drop)。
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

    // ── rgb() / rgba() function form (raikiri-spike-0vv.15) ──
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
        // 対象外 (Non-goals category (b) milestone subset)。1 番目 channel
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
        // regression pin (raikiri-spike-0vv.7、advisor calibration)。
        assert_eq!(
            parse("transparent", "color"),
            Some(PropertyValue::Color(CssColor::TRANSPARENT))
        );
    }

    // ── CssColor::from_hex (raikiri-spike-0vv.14) direct helper contract ──
    //
    // parse_color 経由の integration test は上で網羅済み。以下は helper 自体の
    // API contract を pin する direct call test — 0vv.15 (rgb() function form)
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

    // ── background-color (CSS Backgrounds 3 §2.2、raikiri-spike-0vv.7) ──
    //
    // 5-sample accept pin (task description Verification #4):
    // named / hex / rgb() / rgba() / transparent が
    // `Some(PropertyValue::BackgroundColor(<exact RGBA>))` を返す。
    //
    // exact RGBA assert は advisor calibration: `Some(_)` の loose form だと
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
        // Non-goals category (a) spec-invalid → drop)。
        assert_eq!(parse("none", "background-color"), None);
        // hsl() は CSS Color 4 spec-valid だが Sprint 12 では未対応 (task
        // Non-goals category (b) milestone subset、CSS Color 4 拡張は defer)。
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
    /// unit と percentage を含む。bd raikiri-spike-zls8 で cascade の phase 2
    /// (絶対化) が入ったので、これらを parse 段で drop しなくなった。
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

    /// `math` は spec-valid だが g04 category (b) milestone subset
    /// (bd raikiri-spike-0vv.18、MathML scaling algorithm 未実装) として drop。
    /// `<absolute-size>` / `<relative-size>` は raikiri-spike-4rmu で受理済み —
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
    /// case-insensitive。sibling [`font_weight_keyword_case_insensitive`] と同 pattern。
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
    /// [`PropertyValue::FontSizeRelative`] をそのまま返す — 解決 (親の
    /// computed font-size に対する read-modify-write) は
    /// [`crate::cascade`] の責務 (`bolder` / `lighter` と同型、
    /// bd raikiri-spike-4rmu)。
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
    /// ([`PropertyValue::FontSizeRelative`] doc 参照) — 別 key だと両方が
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
        // bd raikiri-spike-2x8 で追加した unit も `length_payload` 経由で同じ
        // non-negative check を通ることを pin。
        assert_eq!(parse("-1ex", "font-size"), None);
        assert_eq!(parse("-1cm", "font-size"), None);
    }

    #[test]
    fn font_size_accepts_additional_units() {
        // CSS Fonts 4 §2.5 `<length-percentage [0,∞]>` — bd raikiri-spike-2x8 で
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
        // bd raikiri-spike-yh3w: CSS Fonts 4's `font-size` grammar
        // (`<absolute-size> | <relative-size> | <length-percentage [0,∞]>`)
        // has no carve-out excluding `lh`/`rlh` from `<length-percentage>`'s
        // `<length>` component (CSS Values 4 §6.1.1) — `font-size: 1lh` /
        // `font-size: 1rlh` are spec-valid and must survive parsing so the
        // cascade can pick them as a winner (dropping at parse time, as this
        // crate previously did per bd raikiri-spike-vxha, can change *which
        // declaration wins* the cascade — a stronger effect than an
        // incorrectly-resolved value). Resolution against the parent's used
        // line-height is [`crate::resolve::resolve_font_size`]'s concern, not
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
        // The grammar's `[0,∞]` non-negative constraint ([`parse_font_size`]
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
        // CSS Values 3 §5 unitless-zero clause の Number arm を通し (raikiri-spike-fnqx)、
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
    /// `OnceLock` 経由の shared slot であることの直接 pin (raikiri-spike-no7b)。
    ///
    /// この pin は cascade level の test (`mod@crate::cascade` の
    /// `initial_font_family_shares_arc_slot_across_independent_cascade_runs`
    /// 等) では**代替できない** — `font-family` は inherited なので、単一
    /// document 内の兄弟 element は `SpecifiedValues::inherit_from` の
    /// 「親の Arc を bump」経路で共有される。これは同 document 内で
    /// `initial_font_family()` が実質 1 回しか呼ばれないことを意味し、
    /// ここで `OnceLock` を外して per-call `Arc::new(..)` に戻す regression を
    /// 混入させても、その cascade level test は green のままになる
    /// (実際に perturbation で確認済み — bd raikiri-spike-no7b 実装ログ)。
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
        // <number [1,1000]> ]`。旧実装は [100, 900] に絞っていたが spec は
        // [1, 1000] (raikiri-spike-5iy)。bd Verification #1 / #2 / #3。
        assert_eq!(parse("1", "font-weight"), fw(1.0));
        assert_eq!(parse("1000", "font-weight"), fw(1000.0));
        assert_eq!(parse("50", "font-weight"), fw(50.0));
        // 旧 range の両端も当然 valid のまま (regression guard)。
        assert_eq!(parse("100", "font-weight"), fw(100.0));
        assert_eq!(parse("900", "font-weight"), fw(900.0));
    }

    #[test]
    fn font_weight_rejects_out_of_range_number() {
        // g04 category (a) — spec grammar。§2.2 "Only values greater than or
        // equal to 1, and less than or equal to 1000, are valid, and all other
        // values are invalid"。bd Verification #5 / #6 / #7。
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
        // だった (bd raikiri-spike-e52s)。payload / `ComputedValues.font_weight`
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
        // `crate::cascade::tests::font_weight_wpt_font_weight_computed_150_25`
        // (bd raikiri-spike-e52s)。
        assert_eq!(parse("150.25", "font-weight"), fw(150.25));
    }

    #[test]
    fn font_weight_accepts_scientific_notation_number() {
        // `int_value` matcher から `value` (f32) 参照に変えた副次効果。
        // `1e3` は CSS Values 3 の `<number>` production として spec-valid
        // なので受理が正しい (g04 category (a))。
        assert_eq!(parse("1e3", "font-weight"), fw(1000.0));
    }

    #[test]
    fn font_weight_parses_relative_keywords_as_sentinels() {
        // CSS Fonts 4 §2.2: `bolder` / `lighter` は継承値依存の relative
        // weight。parse 段では解けないので sentinel variant を返し、cascade が
        // 親の computed weight から解決する (raikiri-spike-17s8)。
        // bd 17s8 Verification #1 / #2。
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
        // g04 category (a) — spec-invalid keyword → declaration drop。
        assert_eq!(parse("normal-ish", "font-weight"), None);
        assert_eq!(parse("super-bold", "font-weight"), None);
    }

    #[test]
    fn unknown_property_returns_none() {
        // `background-color` (0vv.7)、`padding` (0vv.6)、`margin` (0vv.5)、
        // `width` (0vv.10)、`height` (0vv.11) が順次実装済 = ここから除外。
        // `float` は現時点で parse_value dispatch に未登録 → fall-through で
        // None が返る canonical unknown-property canary (Epic 7 future)。
        assert_eq!(parse("left", "float"), None);
    }

    // ── Display (CSS Display 3 §2、raikiri-spike-m1.22) ─────────────

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
    fn display_rejects_unknown_ident() {
        // Sprint 12 scope: block / inline / inline-block / none 以外は spec-valid
        // でも milestone defer で silent drop (raikiri-spike-0vv.4)。
        // flex / grid / table* / list-item / flow-root / contents は M6+ layout
        // epic で対応予定。
        assert_eq!(parse("flex", "display"), None);
        assert_eq!(parse("grid", "display"), None);
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
    }

    // ── box-sizing (CSS Sizing 3 §3.3、raikiri-spike-0vv.13) ────────
    //
    // Verification anchors (bd raikiri-spike-0vv.13):
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
        // spec-invalid (category (a) → drop):
        // - `padding-box` は CSS-UI 3 draft 相当だが css-sizing-3 では削除済み
        //   (spec note "supersedes the one in [CSS-UI-3]")、
        // - `margin-box` は grammar 外の任意 ident。
        assert_eq!(parse("padding-box", "box-sizing"), None);
        assert_eq!(parse("margin-box", "box-sizing"), None);
        assert_eq!(parse("bogus", "box-sizing"), None);
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

    // ── counter-* (CSS Lists 3 §4、raikiri-spike-s85 M5 pre-work) ──

    // d9y.2: `PropertyValue::Counter*(Arc<Vec<..>>)` に wrap したため、
    // literal test 比較用に Arc<Vec<..>> を返す helper に切り替え
    // (d9y.1 content/string_set helper と同 pattern)。
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
        // d9y.2: empty case は shared Arc slot (`empty_counter_entries`) を使う。
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
        // d9y.2: empty case は shared Arc slot を使う。
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
        // d9y.2: empty case は shared Arc slot を使う。
        assert_eq!(
            parse("none", "counter-set"),
            Some(PropertyValue::CounterSet(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_reset_is_case_insensitive_on_none() {
        // CSS spec: keyword `none` は ASCII case-insensitive
        // d9y.2: empty case は shared Arc slot を使う。
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

    // ── content property (CSS Content 3 §2、raikiri-spike-m5.1) ──
    //
    // task 9 verification items = spec-derived (9y9(a))。task 記述の
    // `raikiri_traits::ContentValueItem` は下流 (raikiri-dom) mapping 先。
    // raikiri-style は raikiri-traits に依存しない leaf crate (94e/3ps Phase B)
    // のため、s85 counter-* precedent に倣い local `ContentComponent` を emit
    // する (原則 1: 前例主義)。Symbol → SmolStr、Url → String へ substitution。

    fn content_items(source: &str) -> Vec<ContentComponent> {
        match parse(source, "content") {
            // d9y.1: PropertyValue::Content(Arc<Vec<..>>) を expose するため
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
        // spec-correct な `first-letter` を採用 (reviewer:spec 9y9(c) の
        // task-own-claim verification で task 側の書き振りが訂正対象)。
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
    // §2.2 / §2.3 / §2.4.2 / §2.5.1、raikiri-spike-1us — m5.1 由来の
    // under-accept fix、6s1 CssContent3 mode arm) ──

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
    // leader() (CSS GCPM 3 §1.1.1 L82、raikiri-spike-1us) ──
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
        // spec §2.6.3: 第 2 引数省略時 default は `content`。
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
        // (raikiri-spike-afv — codex final for m5.1)
        assert_eq!(parse("counter(none)", "content"), None);
    }

    #[test]
    fn content_counters_rejects_none_name() {
        // spec CSS Lists 3 §4 / §4.7: counters() の first argument も
        // <counter-name> production、`none` は invalid。
        // (raikiri-spike-afv — codex final for m5.1)
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

    // ── parse_optional_counter_style trailing-comma strict reject (raikiri-spike-zik) ──
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

    // ── string-set (CSS GCPM 3 §1.1.1、raikiri-spike-m5.3) ──
    //
    // grammar: `none | [ <custom-ident> <content-list> ]#` — 各 entry は
    // (name, content-list) pair、m5.1 の `ContentComponent` + `parse_content_list_items`
    // を reuse。task description の "4-item Vec" は entry name の分を content 側に
    // 誤って含めた結果、実態は 3-item (name は tuple の第 1 要素)。

    fn string_set_entries(source: &str) -> Vec<(SmolStr, Vec<ContentComponent>)> {
        match parse(source, "string-set") {
            // d9y.1: PropertyValue::StringSet(Arc<Vec<..>>)、content_items と同 pattern。
            Some(PropertyValue::StringSet(v)) => (*v).clone(),
            other => panic!("expected PropertyValue::StringSet, got {other:?}"),
        }
    }

    #[test]
    fn string_set_single_entry_with_literal() {
        // Verification 1: string-set: my_str "hello"
        // → [(SmolStr("my_str"), [Literal("hello")])]
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
        // Verification 2 (raikiri-spike-6s1 で adjust):
        // string-set: chapter_title counter(chapter) ": " attr(title)
        //
        // 先頭 `chapter_title` は entry name (tuple 第 1 要素)。content-list は
        // 残りの `counter(chapter) ": " attr(title)` = 3 items。
        //
        // NB: m5.3 の原 test は末尾に `string(chapter_title)` を置いていたが、
        // GCPM 3 §1.1.1 narrow list は `string()` function を含まないため
        // raikiri-spike-6s1 で `attr()` (GCPM narrow list の 5 alt の 1 つ) に
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
        // 検証しない — advisor calibration。
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

    // ── string-set trailing-comma strict reject (raikiri-spike-1ll) ──
    //
    // `#` (comma-separated multiplier、CSS Values 4 §2.3
    // <https://www.w3.org/TR/css-values-4/#mult-comma>) は trailing comma を
    // 許容しない。GCPM 3 §1.1.1 <string-set-value> = `[ <custom-ident>
    // <content-list> ]#` は entry 間 comma 必須 + trailing comma 禁止。
    //
    // m5.3 の初期実装は separator loop で `try_parse(expect_comma).is_err() {
    // break }` していたため、trailing comma を silently 受理していた (comma を
    // consume 後 next iteration で name parse fail → break → 既存 entries を
    // Some で返す)。zik と同 principle の `.ok()?` propagation で strict 化。

    #[test]
    fn string_set_rejects_trailing_comma_single_entry() {
        // `string-set: a "x",` → trailing comma → declaration drop。
        // pre-fix は Some([(a, [Literal("x")])]) を silently 返していた。
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
        // (a, ["x"]) push 後、bottom expect_comma fail → break、leftover
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

    // ── string-set narrow <content-list> gate (CSS GCPM 3 §1.1.1、raikiri-spike-6s1) ──
    //
    // GCPM 3 §1.1.1 L82 verbatim: <content-list> = [ <string> | <counter()> |
    // <counters()> | <content()> | <attr()> ]+ — CSS Content 3 §2 broad list を
    // string-set 用に narrower 再定義。`string()` (function、bare literal とは別)
    // および `target-counter()` / `target-counters()` / `target-text()` は
    // spec grammar に含まれず、`ContentListMode::GcpmStringSet` mode dispatch で
    // reject する (parse_content_list_items が 0 items → parse_string_set →
    // None → declaration drop、bd description の cascade shadow 例
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
        // bd description の cascade shadow 主要例、declaration drop → cascade で
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

    // ── content() function (CSS GCPM 3 §1.1.1.1、raikiri-spike-5ri) ──
    //
    // grammar (spec verbatim, line 758 of TR/css-gcpm-3/):
    //   content() = content([text | before | after | first-letter])
    // 4 keyword、default `text`。GCPM 3 §1.1.1 の narrow `<content-list>` と
    // CSS Content 3 §2 の broad `<content-list>` の両方に含まれるため、string-set
    // および content property 双方の content-list 内で受理される
    // (raikiri-spike-6s1 で `ContentListMode` mode dispatch を導入した後も
    // `content()` arm は両 mode で unconditional accept)。
    //
    // pre-fix reproduction: `string-set: title content(text)` は m5.1/m5.3 で
    // silent drop していた (parse_content_function match arm 欠如 →
    // parse_content_list_items break → 0 items → parse_string_set None →
    // declaration drop)。arm 追加で Some を返すことを pin する。

    #[test]
    fn string_set_content_text_reproduces_pre_fix_drop() {
        // bd raikiri-spike-5ri description の主要 repro case:
        // pre-fix では declaration drop = None、post-fix では
        // (title, [Content{keyword: Text}]) を含む Some を返す。
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
        // §1.1.1.1: `content(text)` は element の string value (default と同義だが
        // 明示的 keyword 保持で downstream の分岐余地を残す)。
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
        // §1.1.1.1 の spec 例 `h2 { string-set: heading content() }` — bare
        // `content()` は default `text` を意味する。
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
        // §1.1.1.1 の grammar は `[text | before | after | first-letter]` の 4
        // alternative のみ。それ以外の ident (spec 上存在しない `marker` 等) は
        // parse_content_text_keyword が None を返し、上位伝播で
        // parse_content_list_items が break、declaration drop = None。
        // (`marker` は list-item pseudo に関する別 concept、content() には出現しない)
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
        // → (header, [Content{Before}, Literal(":"), Content{Text}]) 3 items。
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

    // ── position: running() (CSS GCPM 3 §1.2.1、raikiri-spike-m5.4) ──
    //
    // Verification items 1-6 は task description 由来 (bd raikiri-spike-m5.4)、
    // canonical shape は bd raikiri-spike-376 amended。sibling は s85 (counter)
    // / m5.1 (content) / m5.3 (string-set) の SmolStr wire-through pattern。

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
        // silent match するのを避けるため custom-ident としても弾く (m5.3 string-set
        // と同じ規約、reviewer:spec interpretation point)。
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
        // M5+ scope: relative / absolute / fixed / sticky は本 crate では
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

    // ── parse_length_value helper (raikiri-spike-0vv.3) ────────────────────
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
        // bd raikiri-spike-3gee: `1e40%` は cssparser tokenizer 側で
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
        // reviewer:spec finding (agent a4beb897ae3457dcd, CONFIRMED medium):
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
        // (b) milestone subset — bd raikiri-spike-wnpb の spinout
        // (viewport-relative unit / `cap` / `rcap`) は本 helper で引き続き
        // silent drop。`lh` / `rlh` は bd raikiri-spike-vxha で受理側へ移った
        // (下記 `parse_length_value_accepts_lh` / `_rlh` を参照)。
        assert_eq!(parse_length("10vw", false), None);
        assert_eq!(parse_length("1cap", true), None);
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

    // ── 追加 font-relative unit (CSS Values 4 §6.1.1、bd raikiri-spike-2x8) ──

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

    // ── 追加 absolute unit (CSS Values 4 §6.2、bd raikiri-spike-2x8) ──

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
        // 個別に確認する (bd raikiri-spike-2x8、`Q` は特に取り違えやすい)。
        assert_eq!(parse_length("10IN", false), Some(Length::In(10.0)));
        assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
        assert_eq!(parse_length("2CM", false), Some(Length::Cm(2.0)));
        assert_eq!(parse_length("2EX", false), Some(Length::Ex(2.0)));
        assert_eq!(parse_length("2CH", false), Some(Length::Ch(2.0)));
        assert_eq!(parse_length("2IC", false), Some(Length::Ic(2.0)));
    }

    // ── padding (CSS Box 3 §4.1 physical + §4.2 shorthand、raikiri-spike-0vv.6) ──
    //
    // Primary sources (WebFetch verified 2026-07-20):
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
        // の Number arm を通し (raikiri-spike-fnqx)、非負 filter を pass。
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

    // Verification #6 — spec grammar 外 unit の drop (vw / cap 等 milestone subset)。
    #[test]
    fn padding_top_rejects_unsupported_unit() {
        // (b) milestone subset — vw / cap 等は spec-valid だが bd raikiri-spike-wnpb
        // の spinout follow-up で未対応、parse_length_value 側で drop、`None`
        // propagate → declaration drop。`ch` は bd raikiri-spike-2x8 で、
        // `lh`/`rlh` は bd raikiri-spike-vxha でそれぞれ受理側へ移った
        // (`padding_top_accepts_ch` / `padding_top_accepts_lh` 参照)。
        assert_eq!(parse("10vw", "padding-top"), None);
        assert_eq!(parse("5cap", "padding-top"), None);
    }

    #[test]
    fn padding_top_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh` — bd raikiri-spike-vxha。
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
        // bd raikiri-spike-vxha — `length_payload` の OR-pattern に `Lh`/`Rlh`
        // を足し忘れていないことの直接 pin)。
        assert_eq!(parse("-1lh", "padding-top"), None);
        assert_eq!(parse("-1rlh", "padding-top"), None);
    }

    /// bd raikiri-spike-2x8 で追加した残り unit (`rex` / `rch` / `ic` / `ric` /
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

    // ── line-height (CSS Inline 3 §5.1、raikiri-spike-0vv.9) ────────────────
    //
    // Verification 5/6/7 の spec-derived (9y9(a) + wzj): grammar `normal |
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
        // helper との integration を pin (Task 0vv.3 helper 経由の em/rem/pt)。
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
        // clause の Length 経路 raikiri-spike-fnqx が導入した Px(0.0) route ではない)。
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
        // Regression (reviewer:quality raikiri-spike-0vv.9): Number branch は
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
        // spec `normal` 以外の ident (Epic 7 global keyword 含む) は本 milestone
        // scope 外、silent drop (`auto` / `medium` は spec-invalid、CSS-wide
        // keyword `inherit` 等は milestone subset で defer)。
        assert_eq!(parse("auto", "line-height"), None);
        assert_eq!(parse("medium", "line-height"), None);
        assert_eq!(parse("inherit", "line-height"), None);
    }

    #[test]
    fn line_height_rejects_unsupported_unit() {
        // parse_length_value が silent drop する unit (`vw` / `cap` 等、
        // bd raikiri-spike-wnpb の spinout follow-up) は helper 側で `None` →
        // line-height parse も declaration drop。`ch` は bd raikiri-spike-2x8
        // で、`lh`/`rlh` は bd raikiri-spike-vxha でそれぞれ受理側へ移った
        // (`line_height_accepts_ch` / `line_height_accepts_lh` 参照)。
        assert_eq!(parse("10vw", "line-height"), None);
        assert_eq!(parse("10cap", "line-height"), None);
    }

    #[test]
    fn line_height_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh` — bd raikiri-spike-vxha.
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

    // ── text-align (CSS Text 3 §6.1、raikiri-spike-0vv.8) ──
    //
    // Value grammar (§6.1 spec verbatim):
    //   start | end | left | right | center | justify | match-parent | justify-all
    // Initial: start / Inherited: yes / spec 上 shorthand (text-align-all +
    // text-align-last、Sprint 12 seed は単一 field で保持 = g04 (b) milestone
    // subset)。inheritance test は cascade.rs 側 (parent → child コピー、display
    // non-inherited との対比)。

    #[test]
    fn text_align_parse_all_eight_keywords() {
        // Verification 5: 8 keyword が全て正しく TextAlign variant にマップされる。
        // 1 test で全 arm coverage (patch coverage 100% 目標、§8.1.1)。
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
        // spec §6.1 grammar に含まれない keyword は silent drop (g04 (a) spec-invalid)。
        // `middle` は typo/俗称、`text-align` spec に存在しない。
        assert_eq!(parse("middle", "text-align"), None);
        assert_eq!(parse("baseline", "text-align"), None);
        assert_eq!(parse("top", "text-align"), None);
    }

    #[test]
    fn text_align_rejects_css_wide_keyword() {
        // g04 (b) milestone subset: `inherit` / `initial` / `unset` / `revert` /
        // `revert-layer` は Epic 7、現状は silent drop = None
        // (parse_text_align の `_ => None` arm 経由)。
        assert_eq!(parse("inherit", "text-align"), None);
        assert_eq!(parse("initial", "text-align"), None);
        assert_eq!(parse("unset", "text-align"), None);
        assert_eq!(parse("revert", "text-align"), None);
        assert_eq!(parse("revert-layer", "text-align"), None);
    }

    #[test]
    fn text_align_rejects_string_value() {
        // CSS Text 3 §6.1 grammar は 8 keyword のみ、`<string>` value は本 crate
        // が引用する level では未定義 → g04 (a) spec-invalid、silent drop。
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

    // ── direction (CSS Writing Modes 4 §2.1、raikiri-spike-l3wg) ──
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
        // g04 (b) milestone subset: CSS-wide keyword は Epic 7、silent drop。
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

    // ── resolve_text_align_match_parent (CSS Text 3 §6.1
    // `#valdef-text-align-match-parent`、raikiri-spike-l3wg) ──
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

    // ── margin longhand + shorthand (CSS Box 3 §3.1/§3.2、raikiri-spike-0vv.5) ──
    //
    // Primary source (WebFetch verified 2026-07-20):
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
        // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す
        // (raikiri-spike-fnqx)。margin は non-negative filter を持たないため素通り。
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
        // `cap` (§6.1.1 font-relative lengths) は bd raikiri-spike-wnpb の
        // spinout follow-up で未対応 (parse_length_value 側で drop)。`cm` は
        // bd raikiri-spike-2x8 で、`lh`/`rlh` は bd raikiri-spike-vxha で
        // それぞれ受理側へ移った — margin-side helper に非依存で波及ドロップを pin。
        assert_eq!(parse("1cap", "margin-top"), None);
    }

    #[test]
    fn margin_side_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh` — bd raikiri-spike-vxha。
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
        // (raikiri-spike-fnqx で `parse_length_value` に arm 追加、`parse_length_value_accepts_unitless_zero_only`
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

    // ── border longhand + shorthand (CSS Backgrounds 3 §3、raikiri-spike-0vv.12) ──

    #[test]
    fn border_top_width_parse_px() {
        // Verification #1: parse("1px", "border-top-width") =
        // Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))。
        assert_eq!(
            parse("1px", "border-top-width"),
            Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))
        );
    }

    // ── width (CSS Sizing 3 §3.1.1、raikiri-spike-0vv.10) ────────────────────
    //
    // Primary source (WebFetch verified 2026-07-23):
    // https://www.w3.org/TR/css-sizing-3/#preferred-size-properties
    // Value: `auto | <length-percentage [0,∞]> | min-content | max-content |
    //         fit-content(<length-percentage>)`
    // Initial: auto、Inherited: no。
    //
    // 本 task では `auto` + non-negative `<length-percentage>` のみ受理、
    // min-content / max-content / fit-content() は g04 (b) milestone subset。

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
        // 実 variant を返すようになった transition pin (canary は `float` に
        // 移設済み)。
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
        // Number arm を通す (raikiri-spike-fnqx)。parse_border_width_side の
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
        // Follow-on coverage (raikiri-spike-fnqx bd comment 2026-07-23): `0 solid`
        // は shorthand の width slot を bare-zero で埋めた canonical form。
        // parse_border_shorthand の width slot が parse_border_width_side_res 経由で
        // parse_length_value Number arm を通して Length::Px(0.0) を取り、
        // style slot は Solid、color slot は省略で spec initial =
        // [`BorderColor::CurrentColor`] (CSS Backgrounds 3 §3.1、raikiri-spike-0vv.17)。
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
        // 違う点、advisor calibration)。`parse_length_value(input, false)` の
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
    fn border_width_accepts_absolute_unit() {
        // `<line-width>` の `<length [0,∞]>` half は `<percentage>` を含まないが
        // 他 absolute unit は含む — bd raikiri-spike-2x8 で追加した `pc` を
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
        // CSS Values 4 §6.1.1 `lh`/`rlh` — bd raikiri-spike-vxha。`<line-width>`
        // grammar (`<length [0,∞]> | thin | medium | thick`) has no
        // self-reference concern the way `font-size` / `line-height` do
        // ([`Length::Lh`] doc), so `border-*-width` accepts them unfiltered.
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
        // unitless-zero clause 経由 (raikiri-spike-fnqx): bare `0` も同 Px(0.0)
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
    fn border_style_case_insensitive() {
        // CSS Values 3 §3.1: keyword は ASCII case-insensitive。
        assert_eq!(
            parse("SOLID", "border-top-style"),
            Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
        );
    }

    #[test]
    fn width_rejects_min_content_keyword() {
        // Verification #5: (b) milestone subset — intrinsic sizing keyword は
        // Epic 未着手、silent drop。auto ident 分岐は expect_ident_matching("auto")
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
        // (b) milestone subset — vw / cap 等は spec-valid だが bd raikiri-spike-wnpb
        // の spinout follow-up で未対応、parse_length_value 側で drop、None
        // propagate。`ch` は bd raikiri-spike-2x8 で、`lh`/`rlh` は
        // bd raikiri-spike-vxha でそれぞれ受理側へ移った
        // (`width_accepts_absolute_unit` / `width_accepts_lh` 参照)。
        assert_eq!(parse("10vw", "width"), None);
        assert_eq!(parse("5cap", "width"), None);
    }

    #[test]
    fn width_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh` — bd raikiri-spike-vxha。
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
        // が [`BorderColor::Resolved`] で wrap して cascade static side に届く
        // (raikiri-spike-0vv.17)。
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
        // raikiri-spike-0vv.17: `BorderColor::Resolved` wrap。
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
        // [`BorderColor::CurrentColor`] variant として保持されることを pin する
        // (0vv.17 hazard case 1 の cascade-side coverage、used-value resolution は
        // bd raikiri-spike-q7qf の paint scope 責務)。
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
        // raikiri-spike-0vv.17: color slot は `BorderColor::Resolved` に wrap。
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
        // `currentcolor` keyword ([`BorderColor::CurrentColor`]、spec §3.1
        // initial、raikiri-spike-0vv.17)。
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
    fn border_shorthand_color_slot_accepts_currentcolor() {
        // 37n sibling: border shorthand の color slot は 4 longhand と同じ
        // [`parse_border_color`] を経由するため、`currentcolor` keyword も
        // shorthand から受理される (0vv.17)。
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
            color: BorderColor::CurrentColor, // spec §3.1 initial (0vv.17)
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
            color: BorderColor::CurrentColor, // spec §3.1 initial (0vv.17)
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

    // ── height (CSS Sizing 3 §3.1.1、raikiri-spike-0vv.11) ─────────────
    //
    // Primary source (WebFetch verified 2026-07-23):
    // - #preferred-size-properties: `auto | <length-percentage [0,∞]> |
    //   min-content | max-content | fit-content(<length-percentage>)`,
    //   initial `auto`, Inheritance `No`.
    //
    // Sprint 17 seed scope は `auto` + 非負 `<length-percentage>` の 2 分岐のみ、
    // 他 sizing keyword / global keyword / calc() / var() は silent drop
    // (parse_height doc の g04 3-category 参照)。
    //
    // 37n sibling: sibling `width` (0vv.10) と同 shape の非負 `<length-percentage>` +
    // `auto` grammar、payload 型は共通 [`LengthOrAuto`]。

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
        // 非負制約により `-10px` は spec-invalid → drop (g04 (a))。sibling
        // padding の非負フィルタ pattern と同 shape、margin の `-10px` 受理
        // (§3.1) との対称的な reject を pin。
        assert_eq!(parse("-10px", "height"), None);
    }

    #[test]
    fn height_accepts_zero() {
        // spec `<length-percentage [0,∞]>` — 0 は閉区間下端。`0px` は Dimension arm、
        // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す
        // (raikiri-spike-fnqx)。parse_height の `>= 0.0` 非負 filter を pass。
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
        // Non-goal (b) milestone subset: `min-content` / `max-content` /
        // `fit-content()` は spec-valid だが本 milestone scope 外、silent drop。
        // ident branch は `auto` matching のみ、length parser の Dimension /
        // Percentage arm でも受理されず None に落ちる pin。
        assert_eq!(parse("min-content", "height"), None);
        assert_eq!(parse("max-content", "height"), None);
        assert_eq!(parse("fit-content(50%)", "height"), None);
    }

    #[test]
    fn height_rejects_global_keyword() {
        // Non-goal (b): CSS-wide keyword (`inherit` / `initial` / `unset` /
        // `revert` / `revert-layer`) は Epic 7、silent drop。auto 以外の ident
        // は length parser でも受理されず None。
        assert_eq!(parse("inherit", "height"), None);
        assert_eq!(parse("initial", "height"), None);
        assert_eq!(parse("unset", "height"), None);
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
        // `cap` (§6.1.1 font-relative lengths) は bd raikiri-spike-wnpb の
        // spinout follow-up で未対応 (parse_length_value 側で drop)。`cm` は
        // bd raikiri-spike-2x8 で、`lh`/`rlh` は bd raikiri-spike-vxha で
        // それぞれ受理側へ移った (`height_accepts_absolute_unit` /
        // `height_accepts_lh` 参照)。sibling
        // `margin_side_rejects_unsupported_unit` と同 pattern。
        assert_eq!(parse("1cap", "height"), None);
    }

    #[test]
    fn height_accepts_lh() {
        // CSS Values 4 §6.1.1 `lh`/`rlh` — bd raikiri-spike-vxha。
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
        // CSS Values 4 §6.2 absolute lengths — bd raikiri-spike-2x8。
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
