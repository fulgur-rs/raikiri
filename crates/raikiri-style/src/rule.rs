//! CSS rule と declaration の shape。type/universal selector を含む
//! qualified rule (StyleRule) のみサポート。at-rule (@page / @media 等) は
//! ruletree.rs 側で skip。

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use selectors::parser::SelectorList;

use crate::RaikiriSelectorImpl;
use crate::property::{
    Border, Length, LengthOrAuto, OverflowXY, PropertyValue, Sides, TextDecorationShorthand,
    parse_value,
};

/// 1 property declaration = value + `!important` flag。
///
/// `value` が私有なので、crate 外からは struct literal / functional-update
/// のいずれでも構築できない。
///
/// # write 経路が無いことの compile-fail pin
///
/// `value` は `pub(crate)` に絞ってあるが、そのことは prose の主張のまま
/// だった — read 経路 (`value()`) が届くことは
/// `crates/raikiri/tests/external_consumer.rs` の実行時 test が pin するが、
/// 「write 経路が無い」ことは compile する code では表現できないので、それだけ
/// では pin されない。以下は struct literal 構築が external crate から reject
/// されることの compile-fail pin:
///
/// ```compile_fail
/// use raikiri_style::{CssColor, Declaration, PropertyValue};
///
/// let _ = Declaration {
///     value: PropertyValue::Color(CssColor::BLACK),
///     important: false,
/// };
/// ```
///
/// functional-update (`..base`) 経由の構築も同じ理由 (`value` が private) で
/// reject される。
///
/// ⚠️ **上 2 fence (struct literal / functional-update) は `value` 単独の
/// visibility を独立には discriminate できない** — 実測で判明した点である。
/// 今 `Declaration` に
/// `#[non_exhaustive]` が付いておらず `important` が `pub` だから、この 2
/// fence の失敗理由はたまたま「`value` が private」だけになっている。だが
/// 将来 (a) `Declaration` に `#[non_exhaustive]` が付く、または (b)
/// `important` が narrowing されると、この 2 fence は **`value` が `pub` に
/// 戻っても** (a) は non_exhaustive 由来の `E0639` で、(b) は `important` 由来
/// の `E0451` で、compile-fail し**続ける** — つまりその時点で `value` を
/// 保護する力を失っているのに「compile fail ... ok」のまま気付けない。
/// この 2 fence だけを見て `#[non_exhaustive]` 追加や `important`
/// narrowing の影響を判断しないこと。この risk の drift 検知は下の
/// non-vacuous control の `decl.important` 行が担う (`important` が
/// narrowing されればそちらが先に落ちる — ただし `#[non_exhaustive]` 追加の
/// 影響までは non-vacuous control も検知しない、追加時は本 doc を
/// 書き直すこと)。
///
/// `value` の visibility だけを独立に discriminate するのは下の 3 番目の
/// fence (`.clone()` 後の field 代入) だけである:
///
/// ```compile_fail
/// use raikiri_style::{CssColor, Declaration, Origin, PropertyValue, RuleTree};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("p { color: red; }", Origin::Author);
/// let base = tree.style_rules()[0].declarations()[0].clone();
///
/// let _ = Declaration {
///     value: PropertyValue::Color(CssColor::BLACK),
///     ..base
/// };
/// ```
///
/// 最後に、`Declaration` は `Clone` を derive しているので、external crate は
/// `declarations()` 経由で得た `&Declaration` を `.clone()` して**所有権のある
/// 可変値**を手に入れられる — このとき `value` への代入が reject されることが
/// 唯一の実効的な write-pin である (issue が挙げた
/// `tree.style_rules()[0].declarations()[0].value = ...;` は `style_rules()` /
/// `declarations()` が両方とも `&[_]` を返す ので、`value` の visibility に
/// 関係なく常に `E0594` (immutable な参照への代入) で reject される — つまり
/// これは「常に compile-fail」であり `value` の可視性が将来 `pub` に戻っても
/// 検出できない vacuous な pin になってしまう。下は `.clone()` を挟むことで
/// `value` の visibility だけを discriminate する版):
///
/// ```compile_fail
/// use raikiri_style::{CssColor, Origin, PropertyValue, RuleTree};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("p { color: red; }", Origin::Author);
/// let mut decl = tree.style_rules()[0].declarations()[0].clone();
/// decl.value = PropertyValue::Color(CssColor::BLACK);
/// ```
///
/// # 上の 3 fence の non-visibility 部分の non-vacuous control
///
/// (`raikiri-paint`
/// の `draw_glyphs_at_natural_font_size_is_a_non_vacuous_control` /
/// `crate::page` の `specified_layer_residue_detector_is_not_vacuous` と同じ
/// 語彙 — 「非 vacuous であることを示す control」。)
///
/// 上 3 fence はいずれも `.clone()` / `CssColor::BLACK` / `PropertyValue::Color`
/// の payload 形といった、可視性とは無関係な ingredient に依存している。
/// これらが将来 drift (rename / shape 変更) すると、fence は「意図した理由」
/// ではなく「単に construct できない」で compile-fail し続け、`value` の
/// 可視性が緩んでも検出できない silent vacuity になる。以下は同じ
/// ingredient を使い**かつ compile が通る**ことを assert するので、drift が
/// 起きれば真っ先にここが (compile_fail ではなく) 通常の doctest として
/// 落ちる。
///
/// `decl.important` の直接 field 読み出しも含める — 上 2 fence
/// (struct literal / functional-update) が `value` の visibility を独立に
/// discriminate できているという前提は「`important` が今 `pub` であること」
/// に依存している (上の ⚠️ 節)。`important` が narrowing されれば真っ先に
/// このassertが (compile_fail ではなく通常の doctest として) 壊れるので、
/// 上 2 fence の前提が崩れたことをここで検知する:
///
/// ```
/// use raikiri_style::{CssColor, Origin, PropertyValue, RuleTree};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("p { color: red; }", Origin::Author);
/// let decl = tree.style_rules()[0].declarations()[0].clone();
/// assert_eq!(
///     decl.value(),
///     &PropertyValue::Color(CssColor {
///         r: 255,
///         g: 0,
///         b: 0,
///         a: 255
///     })
/// );
/// // `important` field への直接アクセス (今 `pub`) — narrowing されれば
/// // ここが真っ先に壊れる (上の ⚠️ 節参照)。
/// assert!(!decl.important, "`color: red` に `!important` は付かない");
/// let _ = PropertyValue::Color(CssColor::BLACK);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// resolved property value。
    pub(crate) value: PropertyValue,
    /// `!important` flag (true なら importance 上げ)。
    pub important: bool,
}

impl Declaration {
    /// resolved property value への read-only accessor。
    pub fn value(&self) -> &PropertyValue {
        &self.value
    }
}

/// Qualified style rule (`selectors { declarations }`)。
///
/// `source_order` は同一 `RuleTree` 内で 0 から通し番号。cascade tie-break
/// (同 specificity 時に「後勝ち」) に使う。
/// `origin` は CSS Cascading L4 §6.2 の origin。cascade tuple
/// の rank 化 (`!important` 反転扱い) に使用。
/// Future field (specificity cache / invalidation hint 等) は将来追加予定、
/// `#[non_exhaustive]` の恩恵で non-breaking。
#[non_exhaustive]
pub struct StyleRule {
    /// Parse 済 selector list。type + universal のみ受理 (他は build 段で drop)。
    pub selectors: SelectorList<RaikiriSelectorImpl>,
    /// このルールの declaration list (invalid は含まない)。
    pub(crate) declarations: Vec<Declaration>,
    /// RuleTree 全体を通した 0-indexed source order。
    pub source_order: u32,
    /// この rule が属する cascade origin。
    pub origin: crate::ruletree::Origin,
}

impl StyleRule {
    /// このルールの declaration list への read-only accessor
    /// (invalid は含まない)。
    ///
    /// # `declarations` field 自体への到達不能性
    ///
    /// `declarations` field は `pub(crate)` — external crate から届くのは
    /// この accessor だけである。`StyleRule` は `Clone` を derive していない
    /// ので、external crate は `.clone()` で所有値の `StyleRule` を得る経路が
    /// そもそも無い。加えて `#[non_exhaustive]` が struct literal /
    /// functional-update による新規構築も塞いでいる — この 2 つは独立な
    /// gate であり、`StyleRule` を external crate が所有値として保持する
    /// 経路はどちらの意味でも存在しない。触れられるのはこの accessor が返す
    /// `&[Declaration]` だけである。したがって「write 経路」を意味のある形で
    /// discriminate する pin は存在しない (どんな可視性でも `&StyleRule` から
    /// は書けない) が、
    /// `declarations` という field 名そのものが private であることは以下で
    /// 直接 pin できる — `pub` に戻れば以下は compile が通るようになる:
    ///
    /// ```compile_fail
    /// use raikiri_style::{Origin, RuleTree};
    ///
    /// let mut tree = RuleTree::empty();
    /// tree.add_stylesheet("p { color: red; }", Origin::Author);
    /// let _ = &tree.style_rules()[0].declarations;
    /// ```
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }
}

/// declaration-list を消費して `Vec<Declaration>` を produce。
/// 認識できない property name / invalid value は silently drop。
///
/// # Shorthand expansion
///
/// spec CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim: "A shorthand
/// property sets all of its longhand sub-properties, exactly as if expanded
/// in place." に準拠して、[`PropertyValue::Margin`] 系の shorthand declaration
/// は本関数の出口で 4 longhand declaration に展開される。cascade 段の
/// per-side winner selection が自然に成立することを担保するための spec-correct
/// な expansion — 詳細は [`expand_shorthand_into`] doc 参照。
pub(crate) fn parse_declaration_block(input: &mut Parser<'_, '_>) -> Vec<Declaration> {
    let mut parser = DeclParser;
    let mut out = Vec::new();
    for decl in RuleBodyParser::new(input, &mut parser).flatten() {
        expand_shorthand_into(&decl, |d| out.push(d));
    }
    out
}

/// Shorthand declaration を対応する longhand declaration 列に展開して sink
/// `push` に流す。non-shorthand はそのまま 1 個 push される。
///
/// # どの variant が shorthand かの列挙はここ 1 箇所だけ (call site は 4 つ)
///
/// 呼ぶのは 4 箇所だが、**「どの `PropertyValue` variant が shorthand か」を
/// 列挙する match は本関数だけ**である (per-family の展開表は
/// [`expand_margin`] / [`expand_padding`] / [`expand_border`] に、non-shorthand
/// path は [`expand_none`] に分離してあるが、これは perf 上の hot/cold split
/// であって分岐の追加ではない — **4 helper のいずれも `match` を持たない**)。
/// 本関数の match は exhaustive であり (下の「wildcard arm を置かない理由
/// (契約)」節)、下の 4 call site が同時にその
/// compile-time 保証を継承する:
///
/// 1. [`parse_declaration_block`] — 通常の qualified rule (style rule) の
///    parse 出口。author CSS / inline style / UA stylesheet が通る。
/// 2. [`mod@crate::cascade`] の `collect_cascaded` — **[`crate::ruletree::RuleTree`]
///    の declaration が element cascade candidate になる境界**。
/// 3. [`crate::page::cascade_page`] — **`RuleTree::page_rules` の declaration が
///    `@page` cascade candidate になる境界**。
/// 4. [`crate::page::parse_page_declaration_block`] — `@page` block の parse
///    出口。1 の `@page` 版で、こちらは author CSS の `@page { … }` body が通る
///    (`parse_declaration_block` そのものは `@page` body の parse には使われない
///    — [`crate::page::parse_page_declaration_block`] の doc の「Not a reuse
///    of [`parse_declaration_block`]」節参照)。
///
/// 2 と 3 が要るのは、`add_stylesheet` の**後**に declaration を shorthand
/// variant へ書き戻す post-parse mutation 経路が存在するからである。
/// parse 出口の guard (1 と 4) は parse
/// 出口しか見ないのでこの経路を守らない。両 cascade 入口 (2 と 3) で同じ等価変換を
/// 通すことで、**declaration がどこから来たかに依らず** 下の不変が成立する。
///
/// [`crate::ruletree::RuleTree::style_rules()`] /
/// [`StyleRule::declarations()`] / [`Declaration::value()`] は `pub(crate)` +
/// read-only accessor に絞ってあるので、2 / 3 は現在 **crate 内 invariant guard**
/// である。ただし **2 と 3 で閉じている範囲が違う**:
///
/// - **2 (element)** は可視性だけで閉じる。Consumer が到達できるのは read-only な
///   `&[StyleRule]` / `&[Declaration]` までで、declaration を書き戻す `&mut`
///   経路が public に存在しない。
/// - **3 (`@page`)** は `RuleTree::page_rules` / [`crate::page::PageRule::declarations`]
///   が `pub` のままなので、Consumer は既存 `Declaration` の複製 / 削除 /
///   並べ替え / `important` 書き換え、および既存 `PageRule` を clone して中身を
///   差し替えたものの push を依然できる (`PageRule` は `#[non_exhaustive]` なので
///   新規 literal 構築はできず、seed に既存 `@page` rule が 1 つ要る)。
///   閉じているのは **shorthand を載せた `Declaration` を新規に作れない**点だけで、
///   その根拠は 2 段ある: (a) `value` が私有なので struct literal /
///   functional-update 構築が不可、(b) `Clone` 元になりうる declaration が
///   longhand しか無いのは、parse 直後の時点では [`crate::page::parse_page_declaration_block`]
///   が (element 側の [`parse_declaration_block`] と同じく) shorthand key を
///   emit しないため (`tests::declaration_block_never_emits_shorthand_keys`
///   と `ruletree::tests::page_declaration_block_never_emits_shorthand_keys`、
///   および本関数の exhaustive match が pin)。**(a) か (b) が
///   破れると 3 の経路は crate 外から再び開く** — `Declaration` に公開 ctor を
///   足す変更は本節を再導出してから行うこと。
///
/// docs.rs 読者向けの summary (本節の結論だけを抜いたもの) は
/// [`crate::page::PageRule::declarations`] の doc の「Why this field ... is
/// still `pub`」節にある。本関数は `pub(crate)` なので
/// この doc 自体は docs.rs に出ない — 全 4 call site を跨ぐ完全な導出はここが
/// canonical のまま。
///
/// ⚠️ **2 と 3 で破れ方が違う**。2 (element) の winner は
/// [`crate::specified::SpecifiedValues`] の longhand と同じ field に畳まれるので
/// shorthand が**誤って後勝ちする**。3 (`@page`) の出力は
/// `HashMap<PropertyKey, PropertyValue>` のままなので、shorthand winner は
/// `PropertyKey::Margin` 等の**独立 key に park** する。3 の破れ方はさらに
/// 2 つの症状に分かれる:
///
/// - **silent drop** — shorthand が担うはずだった side が longhand key に
///   一切現れず、`.get(&PropertyKey::MarginRight)` が `None` を返す。
/// - **present-but-wrong** — longhand key は存在するのに値が spec と違う。
///   `@page` の `border` では特に鋭く、style longhand が 1 つも現れないため
///   `page_context_border_styles` が initial `none` と判定し、生き残った
///   `border-top-width` まで CSS Backgrounds 3 §3.3 の style gate で 0 に
///   潰される。
///
/// どちらも下の spec 違反である。silent drop のほうが検出が難しいが、
/// **「@page の破れは silent drop だけ」ではない**。上の 2 症状を family ごとの
/// 具体形に落としたもの、および「どの test が展開の有無を実際に区別するか」の
/// hunk-revert 実測は [`crate::page`] の `post_parse_page_*` test 群の comment 側に
/// ある。
///
/// [`crate::page::PageCascadeResult`] の `declarations` を private + accessor に
/// 絞ってあるのは cascade の**出力**側の話であり、3 (入力側) とは
/// 別 struct・別 gap である。
///
/// sink を取る形にしてあるのは call site 2 / 3 の受け皿が
/// `Vec<(PropertyValue, bool, Origin, _, u32)>` (2 は `selectors` crate の
/// `Specificity`、3 は [`crate::page`] の `PageSpecificity`) であって
/// `Vec<Declaration>` ではないためで、`Vec` 返しにすると declaration ごとの
/// 一時 alloc か scratch buffer の状態管理を強いられる。call site 1 と
/// call site 4 ([`crate::page::parse_page_declaration_block`]) はどちらも
/// `Vec<Declaration>` へ push するだけの closure を渡す (4 は
/// `PageBodyItem::Property` arm のみ、`Size` arm は別 `Vec` へ)。sink 化
/// そのものは alloc 中立〜改善と実測されている — hot loop の regression 要因は
/// sink ではなく引数の受け方だった。下の「signature は perf 要件である」節を参照。
///
/// # Rationale (per-key cascade determinism)
///
/// 既存 property は全て `PropertyKey` と [`crate::specified::SpecifiedValues`]
/// の field が 1:1 disjoint なので winner の適用順に依存しない。ところが
/// `margin` shorthand + `margin-*` longhand は **cross-key dependency** を持ち
/// (`margin: 0; margin-top: 10px` は spec 上 top=10、他=0)、両者が別 key の
/// winner として cascade 段に届くと **適用順が結果を左右してしまう**。
///
/// spec CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> は shorthand を "sets all
/// of its longhand sub-properties, exactly as if expanded in place" と定義し
/// shorthand を longhand の syntactic sugar と扱う。本関数は上記 4 つの境界で
/// spec のこの等価変換を実行することで、**cascade 段には longhand のみが伝わる**
/// 不変を確立する。cross-key dependency 自体が消えるので、適用順は結果に
/// 影響しなくなる (`static ordering after cascade` の deterministic な source
/// of truth)。
///
/// 展開後は同一 property key の longhand が複数 candidate になるが、勝者は
/// `(rank, specificity, source_order)` を `>=` で比較して決まる — call site 2 は
/// [`mod@crate::cascade`] の `beats`、call site 3 は [`crate::page`] の `page_beats` が
/// **同じ `>=` semantics** を持つ。全 key が同値なら**後に現れたほうが勝つ**ので、
/// CSS Cascading L4 §6.1 <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
/// の "the last declaration in document order wins" が
/// "expanded in place" と組み合わさって自然に成立する。
///
/// つまり本関数の rationale は「cascade 段の適用順が具体的に何であるか」には
/// **依存しない** — 適用順に賭けないことそのものが目的である。参考までに現在の
/// 適用順は `PropertyKey` discriminant 昇順であり、shorthand variant が longhand
/// より後ろに置かれている都合で shorthand key が cascade 段に届いたら必ず後勝ち
/// する。これは上の不変により**到達不能**だが、**variant の並び順は本 rationale
/// の根拠ではない**ので、並べ替えでこの gap を塞ごうとしないこと (並べ替えは
/// `margin: 0; margin-top: 10px` と `margin-top: 10px; margin: 0` の鏡像 2 例の
/// うち片方を必ず壊す。詳細は [`mod@crate::cascade`] の `apply_winners` doc)。
///
/// 本関数の exhaustive match (下の「wildcard arm を置かない理由 (契約)」節) に
/// 対する defense-in-depth の runtime guard は本 module の
/// `tests::declaration_block_never_emits_shorthand_keys` (call site 1)、
/// [`mod@crate::cascade`] の `post_parse_*` test 群 (call site 2、6 本 — うち展開の
/// 有無を実際に区別するのは 4 本。残り 2 本は `PropertyKey` 宣言順のせいで
/// 展開しない実装でも偶然 pass する弱い guard で、各 test の comment に
/// その旨を開示してある)、[`crate::page`] の `post_parse_page_*` test 群
/// (call site 3、6 本 — こちらは shorthand が独立 key に park するかどうかを
/// 直接 assert するので **6 本とも展開の有無を区別する**。hunk-revert 実測で
/// 6/6 fail を確認済)、そして `crate::ruletree::tests::page_declaration_block_never_emits_shorthand_keys`
/// (call site 4 — `@page` block の parse 出口自体が shorthand key を emit しない
/// ことを、call site 1 の guard と同じ形で直接 assert する)。
///
/// # wildcard arm を置かない理由 (契約)
///
/// non-shorthand 側は全 variant を明示列挙し `_ => expand_none(d, push)` を
/// 使わない。これは意図的な compile-time guard である: **`_` があると新しい
/// shorthand variant の展開 arm を書き忘れても compile error にならず、その
/// shorthand が黙って cascade 段へ流れる**。上の不変 (「cascade 段には longhand
/// のみが伝わる」) は per-key cascade determinism の前提なので、黙って破れる形に
/// はしない。同 crate の [`mod@crate::cascade`] `resolve_against_inherited` が同じ理由
/// で同じ契約を持つ (『`_` があると素通りして未解決値が
/// public な結果に漏れた』regression が precedent)。
///
/// ## この guard が守らない範囲 (明示)
///
/// 強制されるのは **arm を書くこと**だけである。正しい側に書くことでも、本関数が
/// 呼ばれることでもない:
///
/// 1. **新 variant を or-pattern 側へ足した場合** — 展開先の longhand を持つ
///    variant を「展開先が無い」側の末尾に足せば compile error は黙る。
///    **or-pattern への追加は「展開先が無い」という主張として書くこと。**
///    [`PropertyValue::TextAlign`] が実在の境界例で、spec 上 shorthand だが
///    展開先 variant を本 crate が持たないので or-pattern 側に居る (spec citation
///    と milestone carve-out は同 variant の doc が canonical)。longhand 分離が
///    入ったら本 match の arm も移すこと。
/// 2. **本関数を呼ばない経路** — exhaustive match が強制するのは *arm を書く
///    こと*であって *本関数が呼ばれること*ではない。既知の境界は上の 4 call site
///    に列挙してあり、そこから呼び出しを消しても compile は通る。
///
/// # `important` flag propagation
///
/// shorthand の `!important` は各 longhand にそのまま copy される (spec §3
/// verbatim: "Declaring a shorthand property to be !important is equivalent to
/// declaring all of its sub-properties to be !important.")。
///
/// # Allocation shape
///
/// caller の sink に直接 push する — 従来の
/// `flat_map + vec![d].into_iter()` は non-shorthand path で per-decl の 1-slot
/// heap Vec を alloc していた (common case regression)、
/// in-place push で除去。shorthand path は 4 longhand を 4 回 push (同 alloc
/// budget、shape のみ変更)。sink 化で call site 2 / 3 (受け皿が
/// `Vec<Declaration>` ではない両 cascade 入口) も中間 buffer 無しになるので、
/// non-shorthand の common case でも追加の heap alloc は発生しない。
///
/// # ⚠️ signature は perf 要件である (単純化しないこと)
///
/// 以下の 3 点は好みではなく **cascade hot loop の実測に基づく要件**である:
///
/// 1. **`d: &Declaration` (by-value にしないこと)** — call site 2 は
///    [`mod@crate::cascade`] の `collect_cascaded` の per-declaration loop
///    (72 byte = `size_of::<Declaration>()` stride) の中にあり、by-value 受けは
///    declaration ごとに 72 byte の stack temp + memcpy を強制する
///    (objdump で materialize → `lea` byval ポインタ渡しを確認)。
///    by-value 版は cascade を **heavy config +22% / light config +16%**
///    (どちらも ≈4 ns/declaration) 遅くしていた。
/// 2. **`#[inline]` + per-family helper への分割** — 展開 arm を全て本体に置くと
///    ~4KB の body になり inline 対象にならない。`#[inline(never)]` helper に
///    逃がして hot path を「discriminant を見て 4 way 分岐するだけ」に保つ。
///    分割せずに `#[inline]` だけ付けるのは「展開表の全 `push` を hot loop に
///    展開せよ」という誤った要求になる。禁じているのは展開を本体に置くことで
///    あって、variant を列挙すること自体ではない。
/// 3. **helper に `#[cold]` は付けないこと** — `margin:` / `padding:` /
///    `border:` は author CSS では普通に頻出するので、call site 1 (parse) 側で
///    誤った branch hint になる。
///
/// 切り分け実測: `#[inline]` + split のみ (by-value 維持) では 1/3 しか回復せず、
/// `&Declaration` 化のみ (split 無し) で 2/3 回復、**両方で完全回復** (展開前と
/// 同等〜やや高速)。
///
/// ## 未計測の trade-off
///
/// call site 1 (parse) の non-shorthand path は owned move から `d.clone()` に
/// 変わったので、declaration あたり `PropertyValue::clone()` が 1 回増える。
/// 本 comment 執筆時点では `FontFamily(Vec<Atom>)` がここでの実 heap alloc の
/// 具体例だったが、その後 `Arc<Vec<Atom>>` 化されたため、現時点で
/// `PropertyValue` に生 `Vec` payload
/// を持つ variant は残っていない (`Arc::clone` は bump のみ)。parse は
/// stylesheet あたり 1 回 = `O(declaration 数)`、cascade は毎回
/// `O(element × match した rule × declaration)` なので trade は cascade 側に
/// 倒すのが正しいという分析自体は不変 (将来 variant が生 heap payload を
/// 持てば同じ trade-off が再発しうる) だが、**parse 側の delta 自体は
/// 計測していない**。
#[inline]
pub(crate) fn expand_shorthand_into(d: &Declaration, push: impl FnMut(Declaration)) {
    match d.value {
        PropertyValue::Margin(sides) => expand_margin(sides, d.important, push),
        PropertyValue::Padding(sides) => expand_padding(sides, d.important, push),
        PropertyValue::Border(sides) => expand_border(sides, d.important, push),
        PropertyValue::Overflow(pair) => expand_overflow(pair, d.important, push),
        PropertyValue::TextDecoration(shorthand) => {
            expand_text_decoration(shorthand, d.important, push)
        }
        // 展開先の longhand variant を持たない — そのまま 1 個 push。
        // `_` に潰さないこと (上の「wildcard arm を置かない理由 (契約)」節)。
        // ここへ variant を足すことは「展開先が無い」という主張である。
        PropertyValue::Color(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::FontSize(_)
        | PropertyValue::FontSizeRelative(_)
        | PropertyValue::FontWeight(_)
        | PropertyValue::LineHeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::TextAlign(_)
        | PropertyValue::TextIndent(_)
        | PropertyValue::PaddingTop(_)
        | PropertyValue::PaddingRight(_)
        | PropertyValue::PaddingBottom(_)
        | PropertyValue::PaddingLeft(_)
        | PropertyValue::MarginTop(_)
        | PropertyValue::MarginRight(_)
        | PropertyValue::MarginBottom(_)
        | PropertyValue::MarginLeft(_)
        | PropertyValue::BorderTopWidth(_)
        | PropertyValue::BorderRightWidth(_)
        | PropertyValue::BorderBottomWidth(_)
        | PropertyValue::BorderLeftWidth(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::Width(_)
        | PropertyValue::Height(_)
        | PropertyValue::BoxSizing(_)
        | PropertyValue::Direction(_)
        | PropertyValue::OverflowX(_)
        | PropertyValue::OverflowY(_)
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        | PropertyValue::VerticalAlign(_)
        | PropertyValue::FontStyle(_)
        | PropertyValue::TextTransform(_)
        | PropertyValue::Visibility(_)
        | PropertyValue::ZIndex(_)
        | PropertyValue::WordBreak(_)
        | PropertyValue::OverflowWrap(_)
        | PropertyValue::LetterSpacing(_)
        | PropertyValue::WordSpacing(_) => expand_none(d, push),
    }
}

/// non-shorthand の共通 path。`expand_shorthand_into` から分離してあるのは
/// hot path の code size を最小に保つため (下の per-family helper と対)。
#[inline(always)]
fn expand_none(d: &Declaration, mut push: impl FnMut(Declaration)) {
    push(d.clone());
}

/// `margin` shorthand を 4 longhand に展開する cold helper。
#[inline(never)]
fn expand_margin(sides: Sides<LengthOrAuto>, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::MarginTop(sides.top),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginRight(sides.right),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginBottom(sides.bottom),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginLeft(sides.left),
        important,
    });
}

/// `padding` shorthand を 4 longhand に展開する cold helper。
#[inline(never)]
fn expand_padding(sides: Sides<Length>, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::PaddingTop(sides.top),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingRight(sides.right),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingBottom(sides.bottom),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingLeft(sides.left),
        important,
    });
}

/// `border` shorthand (CSS Backgrounds 3 §3.4 "Border Shorthand Properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>) を 12 longhand
/// (4 side × 3 sub-property = width / style / color) に展開する。
/// margin / padding shorthand precedent と同 pattern。
/// spec `border` grammar は 4 side 共通 (`Sides::all(border)`) だが、cascade
/// 段では per-side longhand として書き込むことで、`border: 1px solid red;
/// border-top-color: blue;` のような longhand override が per-side
/// determinism で解決する。
#[inline(never)]
fn expand_border(sides: Sides<Border>, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::BorderTopWidth(sides.top.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderTopStyle(sides.top.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderTopColor(sides.top.color),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightWidth(sides.right.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightStyle(sides.right.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightColor(sides.right.color),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomWidth(sides.bottom.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomStyle(sides.bottom.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomColor(sides.bottom.color),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftWidth(sides.left.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftStyle(sides.left.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftColor(sides.left.color),
        important,
    });
}

/// `overflow` shorthand (CSS Overflow 3 §3.1
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>) を
/// `overflow-x` / `overflow-y` の 2 longhand に展開する cold helper。
/// margin / padding / border shorthand precedent と
/// 同 pattern — 2-axis なので push は 2 回のみ。
#[inline(never)]
fn expand_overflow(pair: OverflowXY, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::OverflowX(pair.x),
        important,
    });
    push(Declaration {
        value: PropertyValue::OverflowY(pair.y),
        important,
    });
}

/// `text-decoration` shorthand (CSS Text Decoration Module Level 3 §2.4
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-property>) を
/// `text-decoration-line` / `-style` / `-color` の 3 longhand に展開する cold
/// helper。margin / padding / border / overflow shorthand precedent と同
/// pattern — 3 longhand は互いに 1:1 disjoint field (`TextDecorationShorthand`
/// doc の「cross-axis coupling が無い」節参照) なので push は 3 回のみ。
///
/// shorthand parser (`property.rs` の `parse_text_decoration_shorthand`、
/// private fn のため直接 link 不可) が既に省略成分を spec initial value で
/// 埋めているため ([`TextDecorationShorthand`] doc の "Initial value fill"
/// 節)、本関数は 3 field をそのまま 3 declaration に分配するだけでよい —
/// margin/padding/border の各 side にも既に initial fill 済みの値が入って
/// いるのと同じ形。
#[inline(never)]
fn expand_text_decoration(
    shorthand: TextDecorationShorthand,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::TextDecorationLine(shorthand.line),
        important,
    });
    push(Declaration {
        value: PropertyValue::TextDecorationStyle(shorthand.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::TextDecorationColor(shorthand.color),
        important,
    });
}

/// Per-declaration parser for cssparser::RuleBodyParser。
struct DeclParser;

impl<'i> DeclarationParser<'i> for DeclParser {
    type Declaration = Declaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<Declaration, ParseError<'i, Self::Error>> {
        let value = parse_value(name.as_ref(), input).ok_or_else(|| input.new_custom_error(()))?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        // Exhaustive consumption: trailing garbage after the value (and optional
        // `!important`) must reject the whole declaration rather than silently
        // accepting a prefix (e.g. `color: red garbage` / `font-size: 16px 20px`).
        input.expect_exhausted().map_err(
            |e: cssparser::BasicParseError<'i>| -> ParseError<'i, Self::Error> { e.into() },
        )?;
        Ok(Declaration { value, important })
    }
}

// At-rule parser は no-op (block 内で @rule が現れた場合は drop)。
impl<'i> AtRuleParser<'i> for DeclParser {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = ();
}

// Qualified-rule parser (nested rule) も no-op — block 内 nested rule は drop。
impl<'i> QualifiedRuleParser<'i> for DeclParser {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Declaration, ()> for DeclParser {
    fn parse_qualified(&self) -> bool {
        false
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::property::{
        BorderColor, BorderStyle, CssColor, FontWeightValue, Length, LengthOrAuto, OverflowValue,
    };
    use cssparser::ParserInput;

    fn parse_block(source: &str) -> Vec<Declaration> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_declaration_block(&mut parser)
    }

    /// `parse_declaration_block` の出口に shorthand key が 1 つも残らないこと。
    ///
    /// この不変は cascade 段の正しさに load-bearing である
    /// (`crate::cascade` の `apply_winners` doc): shorthand key が cascade に // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// 届くと `PropertyKey` 宣言順で longhand より後に適用され、declaration の
    /// 並び方によっては longhand winner を潰して spec と食い違う。
    ///
    /// 「展開 arm の書き忘れ」形の壊れ方は `expand_shorthand_into` の
    /// **exhaustive match** が compile error にするので、本 test が走るより前に
    /// 落ちる。本 test は一次 guard ではなく
    /// defense-in-depth である — 同関数 doc の「この guard が守らない範囲」節を
    /// 参照。
    ///
    /// ⚠️ 本 test が見るのは `expand_shorthand_into` の **call site 1 (element
    /// 側の parse 出口) だけ**である。post-parse mutation 経路 (crate 内からのみ
    /// 到達可能) は本 test を素通りする。call site 2 (element cascade 入口) の
    /// guard は `crate::cascade` の `post_parse_*` test 群 (6 本) が、call site 3 // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// (`@page` cascade 入口) の guard は `crate::page` の `post_parse_page_*` // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// test 群 (6 本) が、call site 4 (`@page` 側の parse 出口) の guard は
    /// `crate::ruletree::tests::page_declaration_block_never_emits_shorthand_keys`
    /// が持つ。
    #[test]
    fn declaration_block_never_emits_shorthand_keys() {
        use crate::property::PropertyKey;

        let decls = parse_block(
            "margin: 1px; padding: 2px; border: 3px solid red; \
             margin-top: 4px; padding-left: 5px; border-top-width: 6px; \
             text-decoration: underline overline; color: red; font-size: 10px",
        );
        assert!(
            !decls.is_empty(),
            "parse が空 — test corpus 側の regression"
        );

        for decl in &decls {
            let key = decl.value.key();
            assert!(
                !matches!(
                    key,
                    PropertyKey::Margin
                        | PropertyKey::Padding
                        | PropertyKey::Border
                        | PropertyKey::TextDecoration
                ),
                "shorthand key {key:?} が cascade 段へ漏れている — \
                 `expand_shorthand_into` の展開 arm は exhaustive match により \
                 存在するはずなので、疑うのは `parse_declaration_block` が \
                 同関数を通さなくなったか、当該 variant が展開 arm ではなく \
                 non-shorthand 側の or-pattern に書かれているか (どちらも \
                 compile は通る)"
            );
        }
    }

    #[test]
    fn parses_single_declaration() {
        let decls = parse_block("color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
        assert!(!decls[0].important);
    }

    #[test]
    fn captures_important_flag() {
        let decls = parse_block("color: red !important;");
        assert_eq!(decls.len(), 1);
        assert!(decls[0].important);
    }

    #[test]
    fn drops_invalid_property_and_value() {
        // float: 未対応 property → drop (`margin` / `width` は以前 dropped 例に
        // 使っていたが、その後認識対象になったため差し替え。`float` は現状
        // unsupported)。
        // font-size: math → MathML scaling algorithm が未実装のため drop
        // (`medium` を以前 dropped 例に使っていたが、`<absolute-size>` /
        // `<relative-size>` keyword が認識対象になったため差し替え —
        // property.rs `PropertyValue` doc の「例を差し替えるときは…揃えること」
        // 節参照)。
        // color: red → 残す
        let decls = parse_block("float: left; font-size: math; color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
    }

    #[test]
    fn multiple_declarations_in_order() {
        let decls = parse_block("color: red; font-size: 16px; font-weight: 700;");
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].value, PropertyValue::Color(_)));
        assert_eq!(decls[1].value, PropertyValue::FontSize(Length::Px(16.0)));
        assert_eq!(
            decls[2].value,
            PropertyValue::FontWeight(FontWeightValue::Absolute(700.0))
        );
    }

    #[test]
    fn empty_block_returns_empty() {
        assert!(parse_block("").is_empty());
        assert!(parse_block("   ").is_empty());
    }

    #[test]
    fn rejects_trailing_garbage_after_value() {
        // "red garbage" — value 後に余計な token があるので declaration ごと drop。
        let decls = parse_block("color: red garbage;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_extra_length_after_font_size() {
        // "16px 20px" — 2 つ目の length は exhaust しない garbage 扱いで drop。
        let decls = parse_block("font-size: 16px 20px;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_extra_keyword_after_text_transform() {
        // CSS Text Module Level 3 §2.1's `||` combinator makes `uppercase
        // full-width` spec-valid grammar (case keyword co-occurring with
        // `full-width`), but this crate only implements the case-keyword
        // group (`TextTransform` doc's "Scope carving" section). The
        // unimplemented trailing `full-width` ident is exhaust-check
        // garbage the same as any other unconsumed token, so the whole
        // declaration is dropped rather than silently applying just
        // `uppercase`. `parse_text_transform` succeeds on the leading
        // `uppercase` ident here, so this exercises the
        // `DeclParser::expect_exhausted` path specifically.
        let decls = parse_block("text-transform: uppercase full-width;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_text_transform_with_unimplemented_keyword_first() {
        // Same `||` grammar as `rejects_extra_keyword_after_text_transform`,
        // but with the unimplemented `full-width` ident first (`||` allows
        // either order). This takes a *different* code path —
        // `parse_text_transform` itself fails on the unrecognized leading
        // ident, so `parse_value` returns `None` and the declaration is
        // dropped before `DeclParser::expect_exhausted` is ever reached —
        // but reaches the same outcome: whole declaration dropped, not a
        // partial `uppercase` application.
        let decls = parse_block("text-transform: full-width uppercase;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_extra_ident_after_text_indent() {
        // "2em hanging" — `hanging` is spec-valid (CSS Text 3 §8.1) but this
        // crate's `PropertyValue::TextIndent` only carries the
        // `<length-percentage>` component, so `hanging` is unconsumed
        // garbage from `expect_exhausted`'s point of view and the whole
        // declaration drops — mirrors `rejects_extra_length_after_font_size`.
        let decls = parse_block("text-indent: 2em hanging;");
        assert!(decls.is_empty());
    }

    #[test]
    fn still_accepts_important_after_value() {
        // regression guard: !important は exhaustive-consumption check の後でも
        // 引き続き受理されなければならない。
        let decls = parse_block("color: red !important;");
        assert_eq!(decls.len(), 1);
        assert!(decls[0].important);
    }

    #[test]
    fn font_family_leaves_important_alone() {
        // parse_font_family の loop が `!` (from `!important`) を garbage として
        // 拒否してしまうと、declaration ごと drop される (Finding 3)。
        let decls = parse_block("font-family: Arial !important;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::FontFamily(Arc::new(vec![crate::Atom::from("Arial")]))
        );
        assert!(decls[0].important);
    }

    // ── margin shorthand expansion (CSS Cascading L4 §3) ──
    //
    // `parse_declaration_block` は shorthand `margin` を 4 longhand
    // (`MarginTop` / `MarginRight` / `MarginBottom` / `MarginLeft`) に展開する。
    // spec §3 "Shorthand Properties"
    // <https://www.w3.org/TR/css-cascade-4/#shorthand> の "sets all of its
    // longhand sub-properties, exactly as if expanded in place" 準拠、cascade 段
    // に shorthand key を届かせない不変を parse-time で担保する。

    #[test]
    fn margin_shorthand_expands_into_four_longhand_declarations() {
        // `margin: 10px 20px` → 4 longhand (top=10, right=20, bottom=10, left=20)。
        let decls = parse_block("margin: 10px 20px;");
        assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
        assert_eq!(
            decls[0].value,
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(10.0)))
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(20.0)))
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(10.0)))
        );
        assert_eq!(
            decls[3].value,
            PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(20.0)))
        );
    }

    #[test]
    fn margin_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L4 §3 "Shorthand Properties"
        // <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim:
        // "Declaring a shorthand property to be !important is equivalent to
        // declaring all of its sub-properties to be !important." — shorthand の
        // `!important` は全 longhand に copy される。
        let decls = parse_block("margin: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn margin_shorthand_five_values_declaration_dropped() {
        // 5+ value shorthand: parse_margin_shorthand は 4 value 消費、5th 残り
        // token は expect_exhausted で declaration drop。end-to-end で 0 decl
        // になることを pin (property.rs の
        // `margin_shorthand_leaves_extra_values_for_caller_exhausted_check` と complementary)。
        let decls = parse_block("margin: 10px 20px 30px 40px 50px;");
        assert!(
            decls.is_empty(),
            "5-value shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn margin_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl のまま)。
        // shorthand-only expansion の scope を pin する negative test。
        let decls = parse_block("margin-top: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(10.0)))
        );
    }

    // ── padding shorthand expansion (CSS Cascading L4 §3) ──

    #[test]
    fn padding_shorthand_expands_into_four_longhand_declarations() {
        // `padding: 10px 20px` → 4 longhand (top=10, right=20, bottom=10, left=20)。
        // (margin の parse-time expansion model を padding に migrate)
        let decls = parse_block("padding: 10px 20px;");
        assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
        assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
        assert_eq!(
            decls[1].value,
            PropertyValue::PaddingRight(Length::Px(20.0))
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::PaddingBottom(Length::Px(10.0))
        );
        assert_eq!(decls[3].value, PropertyValue::PaddingLeft(Length::Px(20.0)));
    }

    #[test]
    fn padding_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L4 §3: shorthand の `!important` は全 longhand に
        // copy される (margin important 拡張と同じ)。
        let decls = parse_block("padding: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn padding_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl のまま)。
        let decls = parse_block("padding-top: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
    }

    // ── border shorthand expansion (CSS Cascading L4 §3) ──
    //
    // `parse_declaration_block` は shorthand `border` を 12 longhand
    // (4 side × 3 sub-property: width / style / color) に展開する。
    // spec §3 "Shorthand Properties" の "sets all of its longhand sub-properties,
    // exactly as if expanded in place" 準拠、cascade 段に shorthand key を
    // 届かせない不変を parse-time で担保する。margin / padding
    // precedent を 12 longhand shape に拡張。

    #[test]
    fn border_shorthand_expands_into_twelve_longhand_declarations() {
        // `border: 1px solid red` → 12 longhand (4 side × {width, style, color})。
        // order: top-w / top-s / top-c / right-w / right-s / right-c / bottom-* /
        // left-* (`expand_shorthand_into` の hand-written
        // order を pin することでcopy-paste regression を検知)。
        let decls = parse_block("border: 1px solid red;");
        assert_eq!(
            decls.len(),
            12,
            "border shorthand must expand to 12 longhand decls"
        );
        let red = CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        // color longhand は `BorderColor::Resolved(red)`
        // で cascade に届く (shorthand の author-specified color slot は
        // Resolved variant を渡す — hazard case 3 の pin)。
        let red_bc = BorderColor::Resolved(red);
        assert_eq!(
            decls[0].value,
            PropertyValue::BorderTopWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::BorderTopStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[2].value, PropertyValue::BorderTopColor(red_bc));
        assert_eq!(
            decls[3].value,
            PropertyValue::BorderRightWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[4].value,
            PropertyValue::BorderRightStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[5].value, PropertyValue::BorderRightColor(red_bc));
        assert_eq!(
            decls[6].value,
            PropertyValue::BorderBottomWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[7].value,
            PropertyValue::BorderBottomStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[8].value, PropertyValue::BorderBottomColor(red_bc));
        assert_eq!(
            decls[9].value,
            PropertyValue::BorderLeftWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[10].value,
            PropertyValue::BorderLeftStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[11].value, PropertyValue::BorderLeftColor(red_bc));
    }

    #[test]
    fn border_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L4 §3: shorthand `!important` は全 longhand に copy
        // される (margin / padding important 拡張と同 pattern、12 longhand 全て
        // 検証)。
        let decls = parse_block("border: 5px dashed blue !important;");
        assert_eq!(decls.len(), 12);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn border_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl のまま)。
        // shorthand-only expansion の scope を pin する negative test (margin /
        // padding sibling と同 pattern)。
        let decls = parse_block("border-top-width: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::BorderTopWidth(Length::Px(10.0))
        );
    }

    #[test]
    fn border_shorthand_two_widths_declaration_dropped() {
        // property.rs `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
        // の end-to-end 側 pin: `border: 1px 2px` は shorthand helper が 1px を
        // width slot に置いた後 2px は他 slot (style/color) に match しないため
        // fall-through 到達で leftover になり、caller の `expect_exhausted` が
        // declaration 全体を drop する (0 decl)。
        let decls = parse_block("border: 1px 2px;");
        assert!(
            decls.is_empty(),
            "border shorthand with leftover token must be dropped, got {decls:?}"
        );
    }

    // ── overflow shorthand expansion (CSS Overflow 3 §3.1) ──

    #[test]
    fn overflow_shorthand_expands_into_two_longhand_declarations() {
        // `overflow: hidden scroll` → 2 longhand (x=hidden, y=scroll), spec
        // order per §3.1 "sets the specified values of overflow-x and
        // overflow-y in that order".
        let decls = parse_block("overflow: hidden scroll;");
        assert_eq!(decls.len(), 2, "shorthand must expand to 2 longhand decls");
        assert_eq!(
            decls[0].value,
            PropertyValue::OverflowX(OverflowValue::Hidden)
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::OverflowY(OverflowValue::Scroll)
        );
    }

    #[test]
    fn overflow_shorthand_one_value_expands_to_both_axes() {
        // §3.1 "If the second value is omitted, it is copied from the first."
        let decls = parse_block("overflow: auto;");
        assert_eq!(decls.len(), 2);
        assert_eq!(
            decls[0].value,
            PropertyValue::OverflowX(OverflowValue::Auto)
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::OverflowY(OverflowValue::Auto)
        );
    }

    #[test]
    fn overflow_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L4 §3: shorthand `!important` は全 longhand に copy
        // される (margin / padding / border important 拡張と同 pattern)。
        let decls = parse_block("overflow: hidden !important;");
        assert_eq!(decls.len(), 2);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn overflow_shorthand_three_values_declaration_dropped() {
        // property.rs
        // `overflow_shorthand_leaves_extra_values_for_caller_exhausted_check`
        // の end-to-end 側 pin: `parse_overflow_shorthand` は 2 value 消費、3rd
        // 残り token は expect_exhausted で declaration 全体を drop する
        // (0 decl、margin 5-value sibling と同 pattern)。
        let decls = parse_block("overflow: hidden scroll auto;");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            decls.is_empty(),
            "3-value shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn overflow_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl
        // のまま)。shorthand-only expansion の scope を pin する negative test
        // (margin / padding / border sibling と同 pattern)。
        let decls = parse_block("overflow-x: hidden;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::OverflowX(OverflowValue::Hidden)
        );
    }

    // ── text-decoration shorthand expansion (CSS Text Decoration Module
    // Level 3 §2.4) ──
    //
    // `parse_declaration_block` は shorthand `text-decoration` を 3 longhand
    // (`TextDecorationLine` / `TextDecorationStyle` / `TextDecorationColor`)
    // に展開する。margin / padding / border / overflow precedent と同じ
    // parse-time expansion model。

    #[test]
    fn text_decoration_shorthand_expands_into_three_longhand_declarations() {
        use crate::property::{TextDecorationColor, TextDecorationLine, TextDecorationStyle};

        // `text-decoration: underline` → 3 longhand、省略成分
        // (style/color) は spec initial で埋まる (property.rs
        // `parse_text_decoration_shorthand` の "Initial value fill" 節)。
        let decls = parse_block("text-decoration: underline;");
        assert_eq!(decls.len(), 3, "shorthand must expand to 3 longhand decls");
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE)
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Solid)
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor)
        );
    }

    #[test]
    fn text_decoration_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L4 §3: shorthand `!important` は全 longhand に copy
        // される (margin / padding / border / overflow important 拡張と同
        // pattern)。
        let decls = parse_block("text-decoration: underline !important;");
        assert_eq!(decls.len(), 3);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn text_decoration_shorthand_two_style_components_declaration_dropped() {
        // property.rs
        // `text_decoration_shorthand_two_style_components_leaves_leftover_for_caller_exhausted_check`
        // の end-to-end 側 pin: 2nd style keyword は leftover token として
        // expect_exhausted に検知され、declaration 全体が drop される (0 decl)。
        let decls = parse_block("text-decoration: solid wavy;");
        assert!(
            decls.is_empty(),
            "2 style components must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn text_decoration_longhand_declarations_not_expanded() {
        use crate::property::{TextDecorationColor, TextDecorationLine, TextDecorationStyle};

        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl
        // のまま)。shorthand-only expansion の scope を pin する negative test
        // (margin / padding / border / overflow sibling と同 pattern)。
        let decls = parse_block("text-decoration-line: underline;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE)
        );

        let decls = parse_block("text-decoration-style: wavy;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy)
        );

        let decls = parse_block("text-decoration-color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationColor(TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn text_decoration_shorthand_always_overwrites_all_three_longhand() {
        // Shorthand-resets-omitted-longhands: per CSS Cascading L4 §3's
        // "exactly as if expanded in place", `text-decoration: underline`
        // (style/color omitted) still emits a `TextDecorationStyle::Solid` /
        // `TextDecorationColor::CurrentColor` declaration alongside the
        // line one — it does not "leave the other two alone". This is what
        // lets a later bare `text-decoration: underline` reset an earlier
        // `text-decoration-style: wavy` back to `solid` through ordinary
        // cascade order-of-appearance (see
        // `crate::cascade::tests::text_decoration_shorthand_resets_earlier_longhand_declarations` // doc-pointer-lint:ignore: opt-out-3, #[test]-item body (test doc) — rustdoc-blind, confirmed via わざと壊して確かめる
        // for the end-to-end cascade pin).
        use crate::property::{TextDecorationColor, TextDecorationStyle};

        let decls = parse_block("text-decoration-style: wavy; text-decoration: underline;");
        assert_eq!(decls.len(), 4, "1 longhand + 3 expanded, in source order");
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy)
        );
        // The 2nd declaration is the shorthand's TextDecorationLine — the
        // 3rd is the discriminating one: the shorthand's own Solid,
        // appearing *after* the earlier explicit Wavy.
        assert_eq!(
            decls[2].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Solid)
        );
        assert_eq!(
            decls[3].value,
            PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor)
        );
    }

    #[test]
    fn text_decoration_line_duplicate_and_none_combination_declarations_dropped() {
        // End-to-end pin for the leftover-token cases property.rs's
        // `text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check`
        // and `text_decoration_line_none_combined_with_a_keyword_leaves_leftover`
        // exercise at the `parse_text_decoration_line` helper level: the
        // leftover token they leave unconsumed is caught here by
        // `expect_exhausted` (`rule.rs`'s `DeclParser`), dropping the whole
        // declaration (0 decl), same shape as
        // `rejects_trailing_garbage_after_value`.
        assert!(parse_block("text-decoration-line: underline underline;").is_empty());
        assert!(parse_block("text-decoration-line: none underline;").is_empty());
        assert!(parse_block("text-decoration-line: underline none;").is_empty());
    }
}
