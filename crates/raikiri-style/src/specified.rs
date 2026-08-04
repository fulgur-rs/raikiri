//! Cascade winner の **staging 表現** ([`SpecifiedValues`]) と、そこから
//! [`ComputedValues`] への絶対化 (phase 2 + phase 2.5 + phase 3 —
//! phase 2.5 は line-height の絶対化、bd raikiri-spike-vxha で追加)。
//!
//! bd decision raikiri-spike-082k (Option A) / bd task raikiri-spike-zls8
//! (Phase 2 = atomic swap)。
//!
//! # なぜ staging 表現が要るのか
//!
//! cascade winner (`PropertyValue`) が運ぶ length は **specified value** であり、
//! `em` / `rem` を含む。一方 [`ComputedValues`] は Phase 2 以降 **computed value 層**
//! の型だけを持つ。両者の間に位置し「winner を適用し終えたが、まだ絶対化して
//! いない」状態を表すのが本 module の [`SpecifiedValues`] である。
//!
//! この中間状態が独立に必要な理由は [`crate::resolve`] の module doc が述べる
//! とおり — `padding: 2em` の基準となる `font-size` は、その node の**全** winner を
//! 適用し終えた後にしか確定しない。winner を 1 つ適用するたびに絶対化する実装は
//! 適用順に依存してしまい成立しない (`font-size` が最後に適用されれば、それ以前に
//! 絶対化した `2em` は古い基準で焼き付いている)。**絶対化を winner 適用とは別
//! phase に分けることは decision 082k の拘束事項**である。
//!
//! なお cascade 段の winner 適用順そのものは **決定的**である —
//! `PropertyKey` discriminant を index にした slot 配列を昇順に走査するため
//! (bd raikiri-spike-8kn8 以前は `HashMap` iteration 順で非決定的だった)。
//! 上の拘束は適用順の決定性とは独立に成り立つ: どの順に適用しようと
//! 「全 winner 適用後」でなければ基準 `font-size` は確定しない。

use std::sync::Arc;

use smol_str::SmolStr;

use crate::Atom;
use crate::computed::{ComputedValues, RunningTemplate};
use crate::property::{
    Border, BorderColor, BorderStyle, BoxSizing, ContentComponent, CssColor, Direction,
    DisplayValue, Length, LengthOrAuto, LineHeight, Sides, TextAlign, empty_content_list,
    empty_counter_entries, empty_string_set_entries, initial_font_family,
    resolve_text_align_match_parent,
};
use crate::resolve::{
    ComputedLength, ComputedLineHeight, ResolveContext, lift_font_size, lift_line_height,
    resolve_border, resolve_font_size, resolve_length_percentage,
    resolve_length_percentage_or_auto, resolve_line_height, resolve_margin_length_or_auto,
    used_line_height_length,
};

/// Cascade winner を適用し終えたが、まだ絶対化していない per-node の値。
///
/// [`ComputedValues`] と同じ field 集合を持ち、**length を運ぶ field だけ**が
/// specified value 層の型 ([`Length`] / [`LengthOrAuto`] / [`LineHeight`] /
/// [`Border`]) のままになっている。
///
/// # property → 層の対応表 (bd raikiri-spike-ygl0 の帰結)
///
/// raikiri の cascade は property ごとに「どの層まで解決済か」が異なる。一律に
/// 「この struct は specified 型」と言えないので、対応表を型で表現したものが本
/// struct の field 型である:
///
/// | 層 | field |
/// |---|---|
/// | **specified 層のまま** (絶対化が phase 2 / phase 3 待ち) | `font_size` / `line_height` / `padding` / `margin` / `border` / `width` / `height` |
/// | **既に computed-equivalent** (絶対化する length を含まない) | `color` / `background_color` / `font_family` / `font_weight` / `display` / `counter_*` / `content` / `string_set` / `running_templates` / `text_align` / `direction` / `box_sizing` |
///
/// `font_weight` が後者にいるのは load-bearing な事実である —
/// `bolder` / `lighter` は [`crate::cascade::apply_value`] が**この struct へ書き込む
/// 時点で**親の computed weight に対して解決するので、`u16` で保持される
/// (下記「D5 invariant」節)。
///
/// `font_size` は「specified 層のまま」に留まるが、`larger` / `smaller`
/// (`<relative-size>`、raikiri-spike-4rmu) は同じ D5 invariant を使って
/// **同じ書き込み時点**で親基準の絶対値に解決される — 結果は `Length::Px`
/// (specified 層の型としては通常の author px 指定と区別できない値) になるので
/// 表の分類は変わらない。詳細は下記「D5 invariant」節と
/// [`crate::cascade::apply_value`] の `FontSizeRelative` arm を参照。
///
/// page 経路には本 struct に相当する staging 型が無い —
/// [`crate::page::cascade_page`] は同じ 2 phase を `PropertyValue` の bag の上で
/// 直接走らせるので、中間状態は関数 local に閉じており public には出ない
/// (bd raikiri-spike-sshp)。両経路の phase 3 は [`crate::resolve`] の同じ関数群へ
/// funnel する。
///
/// # D5 invariant — inherited field は**親の computed 値**で seed すること
///
/// [`Self::inherit_from`] は inherited property を親の [`ComputedValues`] から
/// seed する。これは単なる効率の話ではなく **正しさの要求**である:
/// [`crate::cascade::apply_value`] の `PropertyValue::FontWeight` arm は
/// `apply_value` 中の read-modify-write で、書き込み前の
/// `self.font_weight` が**親の computed font-weight である**ことに依拠して
/// `bolder` / `lighter` を解決する。[`Self::initial`] から seed すると
/// `bolder` が常に 400 起点になり、**compile error にも既存 test の失敗にも
/// ならずに**壊れる (bd raikiri-spike-i5bs §8.2 debt lens D5)。
/// pin: `bolder_resolves_against_parent_computed_weight_through_staging`。
///
/// `font_size` も同じ invariant に依拠する (raikiri-spike-4rmu で
/// `FontSizeRelative` arm が加わった) — [`Self::inherit_from`] は
/// `font_size` を [`crate::resolve::lift_font_size`] 経由で seed し、この
/// 関数は常に `Length::Px` を返す (`Px` は絶対化の不動点、同関数 doc 参照)。
/// [`Self::initial`] も `font_size: Length::Px(INITIAL_FONT_SIZE_PX)` で
/// 同じく `Px`。したがって `apply_value` の `FontSizeRelative` arm が
/// 書き込み前に読む `self.font_size` は**必ず「親 (または root では initial)
/// の computed font-size」の `Length::Px` 表現である** — `font_weight`
/// (`u16`、単位を持たない) とは表現型が違うが保証の形は同じ。
/// [`Self::initial`] から seed する実装に変えると `larger` が常に
/// `INITIAL_FONT_SIZE_PX` (16px) 起点になり、`bolder` と同じ壊れ方をする。
///
/// # `text_align: match-parent` は D5 と**同型ではない** (raikiri-spike-l3wg)
///
/// `text-align: match-parent` も継承元依存の解決を要する点は `bolder` /
/// `lighter` と同じだが、D5 の read-before-write パターンは**使わない**。
/// D5 が安全なのは `apply_value` の `FontWeight` arm が自 field
/// (`self.font_weight`) だけを読み書きするからである。`match-parent` の解決は
/// **他 property (`direction`) の親の値**を要するため、同じ trick を使うと
/// 「自 node が `direction` winner を持つ場合、その適用順序次第で親ではなく
/// **自分の** direction を読んでしまう」レースが生まれる
/// ([`crate::cascade::resolve_inheritance`] の「winner の適用順に依存しない」
/// invariant への違反)。したがって本 struct の `text_align` field は D5 と
/// 同じく素朴に親からコピーするだけ ([`Self::inherit_from`] 参照) —
/// `match-parent` の解決は [`Self::finalize`] / [`Self::finalize_as_root`] が
/// 全 winner 適用**後**に、明示的に親の [`ComputedValues`] を受け取って行う
/// ([`crate::property::resolve_text_align_match_parent`] のドキュメント参照)。
///
/// # `#[non_exhaustive]`
///
/// [`ComputedValues`] と同じ判断 — future property の追加を source 互換にする。
/// crate 外からの構築は [`Self::initial`] / [`Self::inherit_from`] を使う。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct SpecifiedValues {
    /// [`ComputedValues::color`] の staging。層は computed-equivalent。
    pub color: CssColor,
    /// [`ComputedValues::background_color`] の staging。層は computed-equivalent。
    pub background_color: CssColor,
    /// [`ComputedValues::font_family`] の staging。層は computed-equivalent。
    pub font_family: Arc<Vec<Atom>>,
    /// `font-size` の **specified** value。phase 2 ([`resolve_font_size`]) で
    /// **親の** computed font-size を基準に絶対化される。
    pub font_size: Length,
    /// [`ComputedValues::font_weight`] の staging。**既に computed-equivalent**
    /// — 上記 D5 invariant を参照。型は `f32` (bd raikiri-spike-e52s で `u16`
    /// から格上げ、fractional weight を保持する)。
    pub font_weight: f32,
    /// `line-height` の **specified** value。phase 3 ([`resolve_line_height`]) で
    /// 自 node の computed font-size を基準に絶対化される。
    pub line_height: LineHeight,
    /// [`ComputedValues::display`] の staging。層は computed-equivalent。
    pub display: DisplayValue,
    /// [`ComputedValues::counter_reset`] の staging。層は computed-equivalent。
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// [`ComputedValues::counter_increment`] の staging。層は computed-equivalent。
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// [`ComputedValues::counter_set`] の staging。層は computed-equivalent。
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// [`ComputedValues::content`] の staging。層は computed-equivalent。
    pub content: Arc<Vec<ContentComponent>>,
    /// [`ComputedValues::string_set`] の staging。層は computed-equivalent。
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// [`ComputedValues::running_templates`] の staging。層は computed-equivalent。
    pub running_templates: Vec<RunningTemplate>,
    /// [`ComputedValues::text_align`] の staging。層は computed-equivalent。
    /// `match-parent` はここでは**解決されない** — [`Self`] doc の
    /// "`text_align: match-parent` は D5 と同型ではない" 節参照。
    pub text_align: TextAlign,
    /// [`ComputedValues::direction`] の staging。層は computed-equivalent
    /// (raikiri-spike-l3wg)。
    pub direction: Direction,
    /// `padding` の **specified** value。phase 3
    /// ([`resolve_length_percentage`]) で絶対化される (percentage は素通し)。
    pub padding: Sides<Length>,
    /// `margin` の **specified** value。phase 3
    /// ([`resolve_margin_length_or_auto`]) で絶対化される。
    pub margin: Sides<LengthOrAuto>,
    /// `border` の **specified** value。phase 3 ([`resolve_border`]) で
    /// width が絶対化され、`none` / `hidden` の style gating も適用される。
    pub border: Sides<Border>,
    /// `width` の **specified** value。phase 3 で絶対化される。
    pub width: LengthOrAuto,
    /// `height` の **specified** value。phase 3 で絶対化される。
    pub height: LengthOrAuto,
    /// [`ComputedValues::box_sizing`] の staging。層は computed-equivalent。
    pub box_sizing: BoxSizing,
}

impl SpecifiedValues {
    /// 全 property が CSS spec の initial value である staging 値。
    ///
    /// 各値の spec 根拠は [`ComputedValues::initial`] の field 単位 comment と
    /// [`ComputedValues`] の field doc を canonical source として参照する
    /// (本関数はその **specified 表現**であり、絶対化を通すと
    /// [`ComputedValues::initial`] に一致する — pin:
    /// `initial_specified_finalizes_to_initial_computed`)。
    ///
    /// 親を持たない node (Document root / detached subtree の起点) の seed に
    /// 使う。
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            background_color: CssColor::TRANSPARENT,
            // d9y.1/d9y.2 pattern踏襲 (raikiri-spike-no7b): shared Arc slot —
            // per-node allocation 回避 (`initial_font_family` doc 参照)。
            font_family: initial_font_family(),
            // CSS Fonts 4 §2.5: initial は `medium` (本実装では 16px)。
            font_size: Length::Px(crate::computed::INITIAL_FONT_SIZE_PX),
            font_weight: 400.0,
            line_height: LineHeight::Normal,
            display: DisplayValue::Inline,
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            text_align: TextAlign::Start,
            // CSS Writing Modes 4 §2.1: direction initial は `ltr` (raikiri-spike-l3wg)。
            direction: Direction::Ltr,
            padding: Sides::all(Length::Px(0.0)),
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            // CSS Backgrounds 3 §3.3 / §3.2 / §3.1: width=medium (3px) /
            // style=none / color=currentcolor。computed 層では style gating に
            // より width が 0px に潰れる (`resolve_border`)。
            // (§ 番号は spec の `data-level` 実測値 — bd raikiri-spike-zls8
            // §8.2 spec lens SPEC-5 で 5.x から訂正。crate 全域の一括 sweep は
            // bd raikiri-spike-bcmu / raikiri-spike-xpd4 で完了しており、
            // Backgrounds 3 の §5.x は border-image の節なので
            // 「一貫性のため」本 file を 5.x に戻してはならない。)
            border: Sides::all(INITIAL_BORDER),
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
        }
    }

    /// 親 node の [`ComputedValues`] から child node の staging 開始値を作る。
    ///
    /// - **inherited** property は親の computed 値から seed する。length を運ぶ
    ///   `font_size` / `line_height` は [`lift_font_size`] / [`lift_line_height`]
    ///   で specified 表現に lift する (`Px` は絶対化の不動点なので、phase 2 /
    ///   phase 3 を通しても二重適用にならない — 両関数の doc 参照)。
    /// - **non-inherited** property は [`Self::initial`] と同じ値。
    ///
    /// 分類の canonical source は [`ComputedValues`] の field doc comment。
    /// 新 property を足すときは本関数と [`Self::initial`] と
    /// [`Self::finalize`] の 3 箇所を同時に更新する (前 2 者は field 網羅、
    /// 最後は絶対化の要否)。
    ///
    /// **分類の実装は本関数 1 箇所だけである** — public な
    /// [`ComputedValues::inherit_from`] は本関数 + [`Self::finalize`] へ delegate
    /// する thin wrapper なので、そちらに分類を写す必要はない
    /// (bd raikiri-spike-zls8)。
    ///
    /// [`ComputedValues::inherit_from`]: crate::computed::ComputedValues::inherit_from
    ///
    /// **inherited field を [`Self::initial`] から seed してはならない** —
    /// [`Self`] doc の D5 invariant を参照。
    pub fn inherit_from(parent: &ComputedValues) -> Self {
        // **直接 struct literal で初期化する** — `..Self::initial()` 経由に
        // 「簡約」すると `font_family` の `Vec` を 1 度 allocate → drop してから
        // parent から clone し直すことになり無駄 (roborev job 217 medium 対応。
        // 本 comment は `ComputedValues::inherit_from` から bd raikiri-spike-zls8
        // で移設したもの)。raikiri-spike-no7b で `font_family` は
        // `Arc<Vec<Atom>>` 化されたため、この特定の malloc→drop は解消済み
        // (`initial_font_family()` は shared slot の bump のみ) —
        // ただし本 directive (下記) はそれとは独立に立つ (per-field 網羅列挙が
        // 新規 property 追加時の audit friendliness を担う、将来 field が同種の
        // 生 heap payload を持てば再発しうる)。
        //
        // 本関数は per-node で走るので、この無駄は O(N) の alloc regression に
        // なる。**15 行削れるからと `..Self::initial()` に書き換えてはならない。**
        Self {
            // ── inherited: 親の computed 値から seed ────────────────────
            color: parent.color,
            font_family: parent.font_family.clone(),
            // computed `<length>` → specified `Px` の lift (lossless、不動点)。
            font_size: lift_font_size(parent.font_size),
            // D5: `bolder` / `lighter` はこの値を基準に解決される。
            font_weight: parent.font_weight,
            line_height: lift_line_height(parent.line_height),
            // `match-parent` はここでは解決しない (素朴なコピー) — 解決は
            // `finalize` / `finalize_as_root` が全 winner 適用後に親の
            // `ComputedValues` を明示的に受け取って行う ([`Self`] doc の
            // "D5 と同型ではない" 節、raikiri-spike-l3wg)。
            text_align: parent.text_align,
            direction: parent.direction,
            // ── non-inherited: initial 値 ───────────────────────────────
            background_color: CssColor::TRANSPARENT,
            display: DisplayValue::Inline,
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            padding: Sides::all(Length::Px(0.0)),
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            border: Sides::all(INITIAL_BORDER),
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
        }
    }

    /// **親を持つ** node を絶対化して [`ComputedValues`] にする
    /// (**phase 2 → phase 3**)。
    ///
    /// `parent` は親要素の [`ComputedValues`] (font-size は phase 2 の基準、
    /// `text_align` + `direction` は `match-parent` 解決の基準 —
    /// raikiri-spike-l3wg で `parent_font_size: ComputedLength` から `&ComputedValues`
    /// に広げた、下記「引数を広げた理由」節参照)。`ctx.root_font_size` は
    /// root element の computed font-size。root element 自身には
    /// [`Self::finalize_as_root`] を使うこと (`rem` の基準が違う上、
    /// `match-parent` も "computes to start" の別ルールになる)。
    ///
    /// # 引数を `&ComputedValues` に広げた理由 (raikiri-spike-l3wg)
    ///
    /// 当初 `parent_font_size: ComputedLength` だけを受け取っていたが、
    /// `text-align: match-parent` の解決 (CSS Text 3 §6.1) が親の
    /// `text_align` + `direction` も要求するようになった。3 つの scalar
    /// 引数に分割する案 (`parent_font_size, parent_text_align,
    /// parent_direction`) も検討したが、将来また別の inherited property が
    /// 「親の computed 値」を要求するたびに引数が増える形になるため、
    /// [`crate::cascade::resolve_against_inherited`] が既に取っている
    /// `inherited: &ComputedValues` の shape に揃えた —呼び手 (`cascade.rs` の
    /// `resolve_inheritance`) は `parent_computed` をそのまま渡すだけになる。
    ///
    /// この形は同一 node の winner 適用順序に関する懸念を持ち込まない —
    /// `parent` は呼び手が**この node の staging (`self`) とは別に**保持して
    /// いる、既に確定済みの親の [`ComputedValues`] であり、`self` (自 node の
    /// staging、`direction` winner が上書き済みかもしれない) とは無関係な
    /// 参照である。[`crate::property::resolve_text_align_match_parent`] の
    /// doc が説明する「なぜ `apply_value` ではなく `finalize` か」の根拠は
    /// まさにこの分離にある。
    ///
    /// # phase 順序の担保 (bd raikiri-spike-i5bs §8.2 quality lens F2 への回答)
    ///
    /// phase 2 と phase 3 は基準が違う (親の font-size vs. 自 node の font-size)
    /// にもかかわらず両方 [`ComputedLength`] なので、型検査だけでは取り違えを
    /// 防げない。本実装が実際に担保するのは次の 3 点であり、それ以上ではない:
    ///
    /// 1. **phase 3 の本体は `parent` を名前として持たない** — 後半は
    ///    private な `Self::absolutize_with` に閉じており、その signature に
    ///    `parent` が無いので、**その body の中では**取り違えが書けない。
    ///
    ///    **本関数の body については同じことが言えない** — `parent.font_size` と
    ///    `font_size` はどちらも [`ComputedLength`] として同一 scope に居るので
    ///    `self.absolutize_with(parent.font_size, ..., ctx)` のような取り違えは
    ///    compile する。「取り違えは書けない」という capability claim は本関数には
    ///    成り立たない (bd raikiri-spike-zls8 §8.2 quality lens Q6 で訂正。前例は
    ///    bd raikiri-spike-i5bs の F2)。`text_align` 解決も同型の risk を持つ —
    ///    `resolve_text_align_match_parent(self.text_align, parent.text_align,
    ///    parent.direction)` の 2 番目と 3 番目の引数は異なる型
    ///    (`TextAlign` / `Direction`) なので取り違えれば compile error になるが、
    ///    `self.text_align` と `parent.text_align` はどちらも `TextAlign` なので
    ///    その 2 つの取り違えは compile する。
    /// 2. **cascade pipeline から見た絶対化の入口は本関数と
    ///    [`Self::finalize_as_root`] の 2 つだけ** なので、phase 2 → phase 3 の
    ///    順序と基準の受け渡しは各 2 行に局所化されている。
    ///    **call site を実際に守っているのはこの局所性であって claim 1 ではない。**
    /// 3. **root / 非 root の `rem` 基準の違いが entry point の名前になっている**
    ///    ので、呼び出し側は `ResolveContext` を組み立てる判断をしない。同様に
    ///    `match-parent` の「親あり」/「親なし (root)」分岐も entry point の
    ///    選択そのもの (`finalize` vs `finalize_as_root`) に埋め込まれている。
    ///
    /// 逆に担保**していない**こと: [`crate::resolve`] の絶対化関数群は個別に
    /// public なので、本関数を経由せず誤った基準で呼ぶ code は依然として書ける。
    /// `OwnFontSize` / `ParentFontSize` newtype による型 level の enforcement は
    /// 採らなかった — 守る距離が各関数の 2 行しかない一方、public 関数 8 本と
    /// その doctest の signature churn を伴うため。
    pub fn finalize(self, parent: &ComputedValues, ctx: &ResolveContext) -> ComputedValues {
        // phase 2: font-size を **親基準** で絶対化する (CSS Values 4 §6.1.1
        // parent-metrics 条項)。
        let font_size = resolve_font_size(self.font_size, parent.font_size, ctx);
        // text-align: match-parent の解決 (CSS Text 3 §6.1)。親を持つ node の
        // 分岐 — root element の "computes to start" は `finalize_as_root` 側。
        let text_align =
            resolve_text_align_match_parent(self.text_align, parent.text_align, parent.direction);
        // phase 2.5 (bd raikiri-spike-vxha): line-height を絶対化する。
        // `lh` (自己参照、親基準) / `rlh` (tree-global、`ctx.root_line_height`
        // 基準) の判断根拠は `resolve_line_height` doc が canonical
        // (roborev-refine iter 1 quality lens 1)。この基準は自 node の
        // padding 等 (phase 3) には使わない — それらは `absolutize_with` 内で
        // 改めて**自 node の**基準 (`used_line_height_length(line_height,
        // font_size)`) を求める。
        let parent_line_height_basis =
            used_line_height_length(parent.line_height, parent.font_size);
        let line_height =
            resolve_line_height(self.line_height, font_size, parent_line_height_basis, ctx);
        // phase 3: 残りを **自 node の** font-size / line-height 基準で絶対化する。
        self.absolutize_with(font_size, line_height, text_align, ctx)
    }

    /// **親を持たない** node (root element) を絶対化する。
    ///
    /// root element では `rem` の基準が phase 2 と phase 3 で**異なる**。
    /// CSS Values 4 §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) の verbatim:
    ///
    /// > When used in the value of any font-\* property **on the element they
    /// > refer to**, the font-relative lengths resolve against the computed
    /// > metrics of the parent element—or against the computed metrics
    /// > corresponding to the initial values of the font and line-height
    /// > properties, if the element has no parent.
    ///
    /// - **phase 2** (`font-size`) — `font-size` は font-\* property であり、
    ///   `rem` は定義上 root element を指す (§6.1.1 `rem`: "Equal to the computed
    ///   value of the em unit on the root element."
    ///   <https://www.w3.org/TR/css-values-4/#rem>)。すなわち root element 上の
    ///   `font-size: Nrem` は「自分を指す font-relative length を font-\*
    ///   property に使う」case なので上記条項が発火し、**initial value (16px)
    ///   基準**になる。`em` も同じ条項で initial 基準 (親が無いため)。
    /// - **phase 3** (それ以外) — `padding` 等は font-\* property ではないので
    ///   条項は発火せず、`rem` は素の定義どおり **root element の computed
    ///   font-size** = phase 2 で確定した自分の font-size 基準になる。
    ///   (`em` も "the element on which it is used" の定義どおり自 font-size 基準
    ///   — 同 §の "The other font-relative lengths continue to resolve against
    ///   the element's own metrics when used in line-height." と整合。)
    ///
    /// この非対称は `html { font-size: 20px; padding: 2rem }` で観測できる —
    /// `font-size` は 20px、`padding` は 40px (16px × 2 = 32px では**ない**)。
    /// pin: [`mod@crate::cascade`] の
    /// `rem_on_root_element_box_property_uses_own_font_size`。
    ///
    /// `line-height` **自身の値**に現れる font-relative unit も同じ非対称を
    /// 持つ — 同 §の "The other font-relative lengths continue to resolve
    /// against the element's own metrics when used in line-height." により
    /// `em` 等は自 font-size 基準のまま (phase 2.5、`finalize` 側と同じ)。
    /// **`lh` / `rlh` だけが例外** — spec 原文と両者の非対称の判断根拠は
    /// [`resolve_line_height`] doc が canonical (roborev-refine iter 1
    /// quality lens 1、bd raikiri-spike-awjx の drift 前例により要約に留める)。
    /// 結論だけ述べると、root element には親が無いので `line-height: 1lh`
    /// / `1rlh` は常に「initial values」(`line-height: normal`) 基準に
    /// 帰着し、`normal` は font metrics が無い限り絶対長化できない
    /// (`cap`/`rcap` と同じ wall) — root element 上のこの自己参照は常に
    /// unresolved になる。
    ///
    /// root element の **box property** (`padding: 1rlh` 等) は上記の
    /// 自己参照条項の対象外 — こちらは phase 3 の話で、`rlh` の素の定義
    /// (「root element の `lh`」) どおり **自分の**確定済 line-height を
    /// 参照してよい。本関数はそのために phase 2.5 で自分の line-height を
    /// 確定させてから [`ResolveContext::with_root_line_height`] を組み立てる
    /// (下記 body 参照)。
    ///
    /// # `text-align: match-parent` on the root element (raikiri-spike-l3wg)
    ///
    /// CSS Text 3 §6.1 `#valdef-text-align-match-parent` verbatim continues
    /// past the parent-direction clause with: "Computes to start when
    /// specified on the root element." This is a **different** rule from the
    /// [`Self::finalize`] branch — it does not consult any parent's
    /// `text_align` / `direction` at all, because there is no parent to
    /// consult. [`crate::property::resolve_text_align_match_parent`] only
    /// implements the has-a-parent half; this function implements the
    /// no-parent half locally, matching the same "which entry point runs"
    /// split the `rem` basis already uses below.
    ///
    /// # 引数を取らない理由と、その前提
    ///
    /// 両 phase の基準がいずれも本関数の中で決まるので、呼び出し側が渡し間違える
    /// 余地が無い。**ただしこれは「呼び出し側が本当に親を持たない node にしか
    /// 本関数を使わない」ことが前提**である — 親の computed font-size を捨てて
    /// initial に固定するのが正しいのは §6.1.1 の "if the element has no parent"
    /// が成立するときだけ。[`crate::cascade::resolve_inheritance`] はその invariant
    /// を `debug_assert` で pin している (同関数の `None` arm の comment 参照)。
    /// 同じ「親を持たない」前提が `text-align: match-parent` → `start` にも
    /// 適用される — raikiri のモデルでは「element 祖先が無い」ことを
    /// `root_ctx == None` で判定しており (合成 DOM では複数 element が
    /// 各々この意味で「root」になり得る、`resolve_inheritance` の doc 参照)、
    /// それがそのまま「match-parent の親が無い」の判定基準でもある。
    pub fn finalize_as_root(self) -> ComputedValues {
        // phase 2: 親が無いので initial values 基準。
        //
        // `em` / `rem` の根拠は上記 doc の CSS Values 4 §6.1.1 parent-metrics
        // 条項。**`<percentage>` は §6.1.1 の対象ではない** — 同条項の主語は
        // "the font-relative lengths" であり percentage を含まない。
        // `html { font-size: 150% }` → 24px の根拠は CSS Fonts 4 §2.5
        // <https://www.w3.org/TR/css-fonts-4/#font-size-prop> の
        // "Percentages: refer to parent element's font size" と「親が居ない場合は
        // initial values を基準にする」の**組み合わせによる導出**であって、
        // どちらの § の明文でもない (bd raikiri-spike-zls8 §8.2 spec lens
        // SPEC-6。page 経路の同型の導出は `cascade::resolve_against_inherited`
        // の `FontSize` arm comment に同じ区別で書いてある)。
        //
        // 結果として 3 unit すべて 16px 基準になる。
        let font_size = resolve_font_size(
            self.font_size,
            ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
            &ResolveContext::initial(),
        );
        // CSS Text 3 §6.1: "Computes to start when specified on the root
        // element." — 親の text_align / direction を一切参照しない、この
        // 関数に閉じた特別扱い (上記 doc 節参照)。
        let text_align = match self.text_align {
            TextAlign::MatchParent => TextAlign::Start,
            other => other,
        };
        // phase 2.5 (bd raikiri-spike-vxha): root element には親が無いので
        // `line-height` 自身の値に現れる `lh`/`rlh` の自己参照基準は常に
        // `None` (= "initial values" = `normal`、上記 doc 節)。それ以外の
        // font-relative unit (`em` 等) は自 font-size 基準のまま (`finalize`
        // と同じ `resolve_line_height` 呼び出し形)。
        let line_height = resolve_line_height(
            self.line_height,
            font_size,
            None,
            &ResolveContext::initial(),
        );
        // phase 3 の `ctx`: `rem` の基準は「root element の computed
        // font-size」= 自分、`rlh` の基準も同様「root element の確定済
        // line-height」= 自分 (box property は自己参照条項の対象外、上記
        // doc 節)。`used_line_height_length` は
        // [`crate::cascade::resolve_inheritance`] が子へ配る `child_ctx` と
        // 同じ導出 — 両者の一致は `mod@crate::cascade` の
        // `rlh_on_root_element_matches_child_root_line_height_basis` が pin する。
        let own_line_height = used_line_height_length(line_height, font_size);
        let ctx = ResolveContext::with_root_line_height(font_size, own_line_height);
        self.absolutize_with(font_size, line_height, text_align, &ctx)
    }

    /// phase 3 — 自 node の確定済 computed `font-size` / `line-height` を基準に
    /// 残りを絶対化する。
    ///
    /// `parent` を **意図的に受け取らない** ([`Self::finalize`] doc の担保 1)。
    /// `text_align` は呼び手 ([`Self::finalize`] / [`Self::finalize_as_root`])
    /// が既に `match-parent` を解決した後の値 — 本関数はそれを素通しするだけで、
    /// 自身は解決ロジックを持たない (両呼び手の分岐が異なるため、本関数に
    /// 共通化すると root 判定を関数内に持ち込むことになり、上記「引数を取らない
    /// 理由」の局所性が崩れる)。
    ///
    /// `line_height` も同様に呼び手が phase 2.5 で確定させた**自 node の**
    /// [`ComputedLineHeight`] — 本関数は
    /// それを [`used_line_height_length`] で絶対長へ変換し、
    /// `padding`/`margin`/`border`/`width`/`height` の `lh` 解決基準
    /// (`own_line_height`) として使う (bd raikiri-spike-vxha)。呼び手が
    /// `line_height` を自分で計算する (本関数の内部で
    /// `resolve_line_height` を呼ばない) のは、`padding: 1lh` 等が
    /// **既に確定した**自 node の line-height を必要とし、`font_size` と
    /// 同じく「先に確定させて引数で渡す」形にしないと参照順序を守れない
    /// ためである。
    fn absolutize_with(
        self,
        font_size: ComputedLength,
        line_height: ComputedLineHeight,
        text_align: TextAlign,
        ctx: &ResolveContext,
    ) -> ComputedValues {
        // `padding`/`margin`/`border`/`width`/`height` の `1lh` 解決基準
        // (bd raikiri-spike-vxha) — `rlh` は `ctx.root_line_height` (tree-global)
        // を使うので、本 local はここでしか要らない。
        let own_line_height = used_line_height_length(line_height, font_size);
        ComputedValues {
            color: self.color,
            background_color: self.background_color,
            font_family: self.font_family,
            font_size,
            font_weight: self.font_weight,
            line_height,
            display: self.display,
            counter_reset: self.counter_reset,
            counter_increment: self.counter_increment,
            counter_set: self.counter_set,
            content: self.content,
            string_set: self.string_set,
            running_templates: self.running_templates,
            // 呼び手が既に match-parent を解決した後の値 (関数 doc 参照)。
            text_align,
            // computed value = specified value、相対解決なし ([`Direction`] doc
            // 参照) — 自 node の winner 適用結果をそのまま素通し。
            direction: self.direction,
            padding: self
                .padding
                .map(|l| resolve_length_percentage(l, font_size, own_line_height, ctx)),
            // `margin` は `width`/`height` と型を共有するが、`Lh`/`Rlh`
            // 解決不能時の fallback は違う (`resolve_margin_length_or_auto`
            // doc 参照 — roborev-refine iter 1 Finding A)。
            margin: self
                .margin
                .map(|l| resolve_margin_length_or_auto(l, font_size, own_line_height, ctx)),
            border: self
                .border
                .map(|b| resolve_border(b, font_size, own_line_height, ctx)),
            width: resolve_length_percentage_or_auto(self.width, font_size, own_line_height, ctx),
            height: resolve_length_percentage_or_auto(self.height, font_size, own_line_height, ctx),
            box_sizing: self.box_sizing,
        }
    }
}

/// `border-*` の specified initial value (1 side 分)。
///
/// CSS Backgrounds 3 — width は `medium`
/// (§3.3 "Line Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width> が本文で
/// "The thin, medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." と**規範的に**定める)、style は `none`
/// (§3.2 "Line Patterns: the border-style properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>)、color は
/// `currentcolor` (§3.1 "Line Colors: the border-color properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-color>)。
///
/// **computed 層の initial は本値ではない** — style が `none` なので
/// [`resolve_border`] の gating により width が 0px に潰れる
/// ([`ComputedValues::initial`] 参照)。
///
/// `pub(crate)` なのは page 経路の phase 3 ([`crate::page::cascade_page`]) が
/// `border-*-style` **未宣言**時の gating 基準として `.style` を読むため
/// (bd raikiri-spike-sshp)。CSS Paged Media 3 §6 "Page Properties"
/// <https://www.w3.org/TR/css-page-3/#page-properties> の "both the page context
/// and the margin context have a computed value for every property" により、
/// 未宣言 property の computed value は initial 値であり、`border-*-style` は
/// non-inherited なので継承値ではなくここが唯一の source になる。
/// **「initial の border-style は `none`」を page.rs 側で literal 再掲しない**
/// ための共有である。
pub(crate) const INITIAL_BORDER: Border = Border {
    width: Length::Px(3.0),
    style: BorderStyle::None,
    color: BorderColor::CurrentColor,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::INITIAL_FONT_SIZE_PX;
    use crate::resolve::{
        ComputedBorder, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
        ComputedLineHeight,
    };

    /// `root_font_size` = 16px の共通 context。
    const CTX: ResolveContext = ResolveContext {
        root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
        root_line_height: None,
    };

    /// `finalize` の `parent: &ComputedValues` として渡す、font-size だけ
    /// 差し替えた fixture (raikiri-spike-l3wg — 旧 `PARENT_FS: ComputedLength`
    /// の後継)。`text_align` / `direction` は本 module の phase 2/3 length 系
    /// test では無関係なので initial (`Start` / `Ltr`) のまま。
    fn parent_with_font_size(px: f32) -> ComputedValues {
        ComputedValues {
            font_size: ComputedLength(px),
            ..ComputedValues::initial()
        }
    }

    // -----------------------------------------------------------------
    // initial の 2 表現が一致する (drift 検出)
    // -----------------------------------------------------------------

    /// specified 層の initial を絶対化すると computed 層の initial に一致する。
    ///
    /// 両辺は独立に literal を持つ 2 本の struct literal なので自己参照ではない
    /// — 片方だけを書き換える drift を捕らえる。bd raikiri-spike-jaww が
    /// `resolve.rs` に置いていた drift test (本 task で完全 tautology 化したため
    /// 削除) の後継。
    #[test]
    fn initial_specified_finalizes_to_initial_computed() {
        assert_eq!(
            SpecifiedValues::initial()
                .finalize(&ComputedValues::initial(), &ResolveContext::initial()),
            ComputedValues::initial(),
        );
    }

    /// specified の border initial は `medium` (3px) / `none` / `currentcolor` で、
    /// computed 層では style gating により width が 0px に潰れる
    /// (CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
    /// `none` or `hidden`")。
    #[test]
    fn initial_border_width_is_gated_to_zero_at_computed_layer() {
        assert_eq!(SpecifiedValues::initial().border.top.width, Length::Px(3.0));
        assert_eq!(
            ComputedValues::initial().border.top.width,
            ComputedLength::ZERO
        );
    }

    // -----------------------------------------------------------------
    // inherit_from — inherited / non-inherited の分類
    // -----------------------------------------------------------------

    fn parent_fixture() -> ComputedValues {
        ComputedValues {
            color: CssColor {
                r: 200,
                g: 100,
                b: 50,
                a: 255,
            },
            background_color: CssColor {
                r: 10,
                g: 20,
                b: 30,
                a: 255,
            },
            font_family: Arc::new(vec![Atom::from("sans-serif")]),
            font_size: ComputedLength(24.0),
            font_weight: 700.0,
            line_height: ComputedLineHeight::Number(1.5),
            display: DisplayValue::Block,
            counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
            counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
            counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: vec![RunningTemplate {
                name: SmolStr::new("hdr"),
            }],
            text_align: TextAlign::Center,
            direction: Direction::Rtl,
            padding: Sides::all(ComputedLengthPercentage::Px(7.0)),
            margin: Sides::all(ComputedLengthPercentageOrAuto::Px(12.0)),
            border: Sides::all(ComputedBorder {
                width: ComputedLength(5.0),
                style: BorderStyle::Solid,
                color: BorderColor::Resolved(CssColor::BLACK),
            }),
            width: ComputedLengthPercentageOrAuto::Px(200.0),
            height: ComputedLengthPercentageOrAuto::Px(200.0),
            box_sizing: BoxSizing::BorderBox,
        }
    }

    #[test]
    fn inherit_from_copies_inherited_fields() {
        let parent = parent_fixture();
        let child = SpecifiedValues::inherit_from(&parent);
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_family, parent.font_family);
        // raikiri-spike-no7b (d9y.1/d9y.2 pattern踏襲): `inherit_from` の
        // `parent.font_family.clone()` は Arc bump — deep-clone regression
        // なら ptr_eq が false になる (d9y.1 `Arc::ptr_eq` behavioral-proxy
        // methodology、`mod@crate::cascade` test 群と同型)。
        assert!(Arc::ptr_eq(&child.font_family, &parent.font_family));
        assert_eq!(child.font_weight, 700.0);
        // CSS Text 3 §6.1: text-align は inherited。
        assert_eq!(child.text_align, TextAlign::Center);
        // CSS Writing Modes 4 §2.1: direction は inherited (raikiri-spike-l3wg)。
        assert_eq!(child.direction, Direction::Rtl);
        // computed → specified の lift (px 表現)。
        assert_eq!(child.font_size, Length::Px(24.0));
        assert_eq!(child.line_height, LineHeight::Number(1.5));
    }

    #[test]
    fn inherit_from_leaves_non_inherited_fields_at_initial() {
        let parent = parent_fixture();
        let child = SpecifiedValues::inherit_from(&parent);
        let initial = SpecifiedValues::initial();
        assert_eq!(child.background_color, CssColor::TRANSPARENT);
        assert_eq!(child.display, DisplayValue::Inline);
        assert!(child.counter_reset.is_empty());
        assert!(child.counter_increment.is_empty());
        assert!(child.counter_set.is_empty());
        assert!(child.content.is_empty());
        assert!(child.string_set.is_empty());
        assert!(child.running_templates.is_empty());
        assert_eq!(child.padding, initial.padding);
        assert_eq!(child.margin, initial.margin);
        assert_eq!(child.border, initial.border);
        assert_eq!(child.width, LengthOrAuto::Auto);
        assert_eq!(child.height, LengthOrAuto::Auto);
        assert_eq!(child.box_sizing, BoxSizing::ContentBox);
    }

    /// `line-height: 150%` を親が宣言していた場合、親の computed は
    /// `Length(ComputedLength(px))` であり、子は**その length をそのまま**継承する
    /// (CSS Inline 3 §5.1 — percentage は宣言要素で絶対化される)。
    #[test]
    fn inherit_from_lifts_computed_line_height_length_without_re_resolving() {
        let parent = ComputedValues {
            line_height: ComputedLineHeight::Length(ComputedLength(30.0)),
            ..ComputedValues::initial()
        };
        let child = SpecifiedValues::inherit_from(&parent);
        assert_eq!(child.line_height, LineHeight::Length(Length::Px(30.0)));
        // 子の font-size が 10px でも 15px にはならない。
        let computed = child.finalize(&parent_with_font_size(10.0), &CTX);
        assert_eq!(
            computed.line_height,
            ComputedLineHeight::Length(ComputedLength(30.0))
        );
    }

    // -----------------------------------------------------------------
    // finalize — phase 2 / phase 3 の基準
    // -----------------------------------------------------------------

    /// phase 2 の `em` は **親** の font-size 基準、phase 3 の `em` は
    /// **自 node の (phase 2 で確定した)** font-size 基準
    /// (CSS Values 4 §6.1.1)。両者を取り違えると padding が 32px になる。
    #[test]
    fn finalize_uses_parent_font_size_for_font_size_and_own_for_the_rest() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Em(2.0); // 親 16px → 32px
        sv.padding = Sides::all(Length::Em(1.0)); // 自 32px → 32px
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(32.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(32.0));
    }

    /// bd raikiri-spike-2x8 で追加した `ex` も `em` と同じ parent/own 非対称を
    /// 持つ (unknown-metric fallback `0.5em`、[`Length::Ex`] doc)。数値は
    /// [`finalize_uses_parent_font_size_for_font_size_and_own_for_the_rest`]
    /// と揃える (`32px` / `32px`) — multiplier を変えて `ex` の `0.5` 係数を
    /// 通しても同じ基準規則になることを示す。padding 側に **親** (16px) を
    /// 誤って使うと `16 * 0.5 * 2 = 16px` になり、`32px` にならないため
    /// parent/own の取り違えを検出できる。
    #[test]
    fn finalize_resolves_ex_against_parent_for_font_size_and_own_for_padding() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Ex(4.0); // 親 16px 基準 → 0.5 * 4 * 16 = 32px
        sv.padding = Sides::all(Length::Ex(2.0)); // 自 (phase 2 で確定した) 32px 基準 → 0.5 * 2 * 32 = 32px
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(32.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(32.0));
    }

    /// `padding: 1lh` needs the **already-resolved own** line-height as its
    /// basis (bd raikiri-spike-vxha) — this is exactly the phase-3 reordering
    /// the issue's §1 describes: `absolutize_with` must capture `line_height`
    /// into a local *before* resolving `padding`/`margin`/`border`/`width`/
    /// `height`, or this would be unable to read it at all.
    #[test]
    fn finalize_resolves_lh_against_own_line_height_for_padding() {
        let mut sv = SpecifiedValues::initial();
        sv.line_height = LineHeight::Number(2.0); // 自 font-size (16px, inherited) 基準 → used 32px
        sv.padding = Sides::all(Length::Lh(1.5)); // 1.5 * 32 = 48px
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(48.0));
    }

    /// When the own line-height is unresolvable (`normal`, the initial value
    /// — the common case, not an edge case), `1lh` falls back to padding's
    /// own spec initial `0` rather than a fabricated length (cleanroom: see
    /// `crate::resolve::resolve_length_percentage` doc for why).
    #[test]
    fn finalize_resolves_lh_falls_back_to_zero_when_line_height_normal() {
        let mut sv = SpecifiedValues::initial(); // line_height stays `normal`
        sv.padding = Sides::all(Length::Lh(1.5));
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
    }

    /// `<percentage>` は property ごとに扱いが違う: `font-size` は length に
    /// なり、`padding` / `margin` / `width` / `height` は computed 層に
    /// percentage のまま残る (CSS Values 4 §5.5.1 + CSS Box 3 の各 propdef)。
    #[test]
    fn finalize_keeps_box_percentages_and_resolves_font_size_percentage() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Percent(150.0);
        sv.padding = Sides::all(Length::Percent(25.0));
        sv.margin = Sides::all(LengthOrAuto::Length(Length::Percent(10.0)));
        sv.width = LengthOrAuto::Length(Length::Percent(50.0));
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(24.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Percent(25.0));
        assert_eq!(
            cv.margin.bottom,
            ComputedLengthPercentageOrAuto::Percent(10.0)
        );
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Percent(50.0));
    }

    /// `rem` は phase 2 / phase 3 のどちらでも `ctx.root_font_size` 基準
    /// (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>)。
    #[test]
    fn finalize_resolves_rem_against_context_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(20.0));
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Rem(2.0);
        sv.margin = Sides::all(LengthOrAuto::Length(Length::Rem(0.5)));
        let cv = sv.finalize(&parent_with_font_size(64.0), &ctx);
        assert_eq!(cv.font_size, ComputedLength(40.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    // -----------------------------------------------------------------
    // finalize — text-align: match-parent (CSS Text 3 §6.1、raikiri-spike-l3wg)
    // -----------------------------------------------------------------

    #[test]
    fn finalize_resolves_match_parent_against_parent_text_align_and_direction() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        let parent = ComputedValues {
            text_align: TextAlign::Start,
            direction: Direction::Rtl,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        // Start + Rtl → Right (CSS Text 3 §6.1 verbatim table).
        assert_eq!(cv.text_align, TextAlign::Right);
    }

    /// **The test that pins the whole design of this module's `match-parent`
    /// handling**: a node that declares *both* `direction: rtl` and
    /// `text-align: match-parent` must still resolve against the *parent's*
    /// direction, not its own. If `finalize` (or a future refactor) ever
    /// starts reading `self.direction` instead of `parent.direction` for this
    /// resolution, this is the test that catches it — every other test in
    /// this module has `self.direction == parent.direction` and would stay
    /// green.
    ///
    /// Spec citation: CSS Text 3 §6.1 `#valdef-text-align-match-parent`
    /// says "interpreted against **the parent's** direction value" — not the
    /// element's own. See [`crate::property::resolve_text_align_match_parent`]
    /// doc for why this can't be resolved in `cascade::apply_value` (the
    /// same-node winner-order hazard between the `direction` and `text-align`
    /// `PropertyKey` slots).
    #[test]
    fn finalize_match_parent_uses_parent_direction_not_own_direction_winner() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        // Own winner for `direction` already applied to the staging value —
        // simulates `direction: rtl` being cascaded on *this* node.
        sv.direction = Direction::Rtl;

        // Parent disagrees: Ltr.
        let parent = ComputedValues {
            text_align: TextAlign::Start,
            direction: Direction::Ltr,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        // Must resolve against the *parent's* Ltr (→ Left), not the node's
        // own Rtl (which would give Right).
        assert_eq!(cv.text_align, TextAlign::Left);
        // The node's own `direction` winner is unaffected — it is a wholly
        // separate property and still flows through to the child's computed
        // value normally.
        assert_eq!(cv.direction, Direction::Rtl);
    }

    #[test]
    fn finalize_copies_non_start_end_parent_text_align_verbatim() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        let parent = ComputedValues {
            text_align: TextAlign::Center,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        assert_eq!(cv.text_align, TextAlign::Center);
    }

    #[test]
    fn finalize_leaves_non_match_parent_text_align_untouched() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::Center;
        let parent = ComputedValues {
            text_align: TextAlign::Start,
            direction: Direction::Rtl,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        assert_eq!(cv.text_align, TextAlign::Center);
    }

    /// CSS Text 3 §6.1 verbatim: "Computes to start when specified on the
    /// root element." — the parent-direction table does **not** apply here,
    /// even if the node itself declares a `direction`.
    #[test]
    fn finalize_as_root_resolves_match_parent_to_start() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        sv.direction = Direction::Rtl;
        assert_eq!(sv.finalize_as_root().text_align, TextAlign::Start);
    }

    #[test]
    fn finalize_as_root_leaves_non_match_parent_text_align_untouched() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::Right;
        assert_eq!(sv.finalize_as_root().text_align, TextAlign::Right);
    }

    /// `direction` itself is a plain inherited pass-through — no
    /// `match-parent`-style resolution, symmetric with [`TextAlign::Center`]
    /// et al.
    #[test]
    fn finalize_passes_direction_through_unchanged() {
        let mut sv = SpecifiedValues::initial();
        sv.direction = Direction::Rtl;
        let cv = sv.clone().finalize(&ComputedValues::initial(), &CTX);
        assert_eq!(cv.direction, Direction::Rtl);
        assert_eq!(sv.finalize_as_root().direction, Direction::Rtl);
    }

    /// root element では `rem` の基準が phase 2 と phase 3 で異なる
    /// ([`SpecifiedValues::finalize_as_root`] の doc に spec verbatim)。
    ///
    /// - `font-size: 2rem` → **32px** (initial 16px 基準 — font-\* property 上の
    ///   自己参照 unit なので parent-metrics 条項が発火する)
    /// - `padding: 2rem` → **40px** (自 font-size 20px 基準 — box property は
    ///   条項の対象外で `rem` は素の定義「root element の computed font-size」)
    #[test]
    fn finalize_as_root_uses_initial_for_font_size_and_own_for_box_properties() {
        let mut fs_case = SpecifiedValues::initial();
        fs_case.font_size = Length::Rem(2.0);
        assert_eq!(fs_case.finalize_as_root().font_size, ComputedLength(32.0));

        let mut box_case = SpecifiedValues::initial();
        box_case.font_size = Length::Px(20.0);
        box_case.padding = Sides::all(Length::Rem(2.0));
        let cv = box_case.finalize_as_root();
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(40.0));
    }

    /// root element の `font-size: Nem` も親が無いので initial 16px 基準。
    #[test]
    fn finalize_as_root_resolves_em_font_size_against_initial() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Em(1.5);
        assert_eq!(sv.finalize_as_root().font_size, ComputedLength(24.0));
    }

    /// root element の **box property** の `1rlh` は自分の確定済 line-height
    /// を基準にする (bd raikiri-spike-vxha — `rem_on_root_element_box_property_uses_own_font_size`
    /// の `rem` と同じ非対称の `rlh` 版。self-reference 条項の対象は
    /// `line-height` 自身の値だけで、`padding` はその対象外)。
    /// `finalize_as_root` は own line-height を phase 2.5 で確定させてから
    /// `ResolveContext::with_root_line_height` を組み立てる — この test は
    /// その配線がここまで届くことを直接 pin する。
    #[test]
    fn finalize_as_root_resolves_rlh_using_own_line_height_basis() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Px(20.0);
        sv.line_height = LineHeight::Number(2.0); // own font-size 20px → used 40px
        sv.padding = Sides::all(Length::Rlh(1.5)); // 1.5 * 40 = 60px
        let cv = sv.finalize_as_root();
        assert_eq!(cv.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(60.0));
    }

    /// root element には親が無いので、`line-height` 自身の値としての
    /// `1lh`/`1rlh` (自己参照) は常に「initial values」= `normal` 基準に
    /// 帰着し、常に unresolved になる (CSS Values 4 §6.1.1 "if the element
    /// has no parent" — `finalize_as_root` doc 参照)。上の test と対で、
    /// 「box property の rlh は自分の line-height を使う」「line-height 自身の
    /// lh/rlh は self-reference で常に normal」の 2 つの非対称を区別する。
    #[test]
    fn finalize_as_root_line_height_self_reference_is_always_normal() {
        let mut sv = SpecifiedValues::initial();
        sv.line_height = LineHeight::Length(Length::Lh(1.0));
        assert_eq!(
            sv.finalize_as_root().line_height,
            ComputedLineHeight::Normal
        );

        let mut sv_rlh = SpecifiedValues::initial();
        sv_rlh.line_height = LineHeight::Length(Length::Rlh(1.0));
        assert_eq!(
            sv_rlh.finalize_as_root().line_height,
            ComputedLineHeight::Normal
        );
    }

    /// 全 4 side が独立に絶対化される (`Sides::map` が side を取り違えない)。
    #[test]
    fn finalize_absolutizes_each_side_independently() {
        let mut sv = SpecifiedValues::initial();
        sv.padding = Sides {
            top: Length::Px(1.0),
            right: Length::Em(1.0),
            bottom: Length::Pt(3.0),
            left: Length::Percent(5.0),
        };
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(1.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(16.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(4.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Percent(5.0));
    }
}
