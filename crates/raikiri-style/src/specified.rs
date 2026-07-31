//! Cascade winner の **staging 表現** ([`SpecifiedValues`]) と、そこから
//! [`ComputedValues`] への絶対化 (phase 2 + phase 3)。
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
    Border, BorderColor, BorderStyle, BoxSizing, ContentComponent, CssColor, DisplayValue, Length,
    LengthOrAuto, LineHeight, Sides, TextAlign, empty_content_list, empty_counter_entries,
    empty_string_set_entries,
};
use crate::resolve::{
    ComputedLength, ResolveContext, lift_font_size, lift_line_height, resolve_border,
    resolve_font_size, resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_line_height,
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
/// | **既に computed-equivalent** (絶対化する length を含まない) | `color` / `background_color` / `font_family` / `font_weight` / `display` / `counter_*` / `content` / `string_set` / `running_templates` / `text_align` / `box_sizing` |
///
/// `font_weight` が後者にいるのは load-bearing な事実である —
/// `bolder` / `lighter` は [`crate::cascade::apply_value`] が**この struct へ書き込む
/// 時点で**親の computed weight に対して解決するので、`u16` で保持される
/// (下記「D5 invariant」節)。
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
/// `apply_value` 中で唯一の read-modify-write で、書き込み前の
/// `self.font_weight` が**親の computed font-weight である**ことに依拠して
/// `bolder` / `lighter` を解決する。[`Self::initial`] から seed すると
/// `bolder` が常に 400 起点になり、**compile error にも既存 test の失敗にも
/// ならずに**壊れる (bd raikiri-spike-i5bs §8.2 debt lens D5)。
/// pin: `bolder_resolves_against_parent_computed_weight_through_staging`。
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
    pub font_family: Vec<Atom>,
    /// `font-size` の **specified** value。phase 2 ([`resolve_font_size`]) で
    /// **親の** computed font-size を基準に絶対化される。
    pub font_size: Length,
    /// [`ComputedValues::font_weight`] の staging。**既に computed-equivalent**
    /// — 上記 D5 invariant を参照。
    pub font_weight: u16,
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
    pub text_align: TextAlign,
    /// `padding` の **specified** value。phase 3
    /// ([`resolve_length_percentage`]) で絶対化される (percentage は素通し)。
    pub padding: Sides<Length>,
    /// `margin` の **specified** value。phase 3
    /// ([`resolve_length_percentage_or_auto`]) で絶対化される。
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
            font_family: vec![Atom::from("serif")],
            // CSS Fonts 4 §2.5: initial は `medium` (本実装では 16px)。
            font_size: Length::Px(crate::computed::INITIAL_FONT_SIZE_PX),
            font_weight: 400,
            line_height: LineHeight::Normal,
            display: DisplayValue::Inline,
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            text_align: TextAlign::Start,
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
        // で移設したもの)。
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
            text_align: parent.text_align,
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
    /// `parent_font_size` は親要素の computed font-size、`ctx.root_font_size` は
    /// root element の computed font-size。root element 自身には
    /// [`Self::finalize_as_root`] を使うこと (`rem` の基準が違う)。
    ///
    /// # phase 順序の担保 (bd raikiri-spike-i5bs §8.2 quality lens F2 への回答)
    ///
    /// phase 2 と phase 3 は基準が違う (親の font-size vs. 自 node の font-size)
    /// にもかかわらず両方 [`ComputedLength`] なので、型検査だけでは取り違えを
    /// 防げない。本実装が実際に担保するのは次の 3 点であり、それ以上ではない:
    ///
    /// 1. **phase 3 の本体は `parent_font_size` を名前として持たない** — 後半は
    ///    private な `Self::absolutize_with` に閉じており、その signature に
    ///    `parent_font_size` が無いので、**その body の中では**取り違えが書けない。
    ///
    ///    **本関数の body については同じことが言えない** — `parent_font_size` と
    ///    `font_size` はどちらも [`ComputedLength`] として同一 scope に居るので
    ///    `self.absolutize_with(parent_font_size, ctx)` は compile する。
    ///    「取り違えは書けない」という capability claim は本関数には成り立たない
    ///    (bd raikiri-spike-zls8 §8.2 quality lens Q6 で訂正。前例は
    ///    bd raikiri-spike-i5bs の F2)。
    /// 2. **cascade pipeline から見た絶対化の入口は本関数と
    ///    [`Self::finalize_as_root`] の 2 つだけ** なので、phase 2 → phase 3 の
    ///    順序と基準の受け渡しは各 2 行に局所化されている。
    ///    **call site を実際に守っているのはこの局所性であって claim 1 ではない。**
    /// 3. **root / 非 root の `rem` 基準の違いが entry point の名前になっている**
    ///    ので、呼び出し側は `ResolveContext` を組み立てる判断をしない。
    ///
    /// 逆に担保**していない**こと: [`crate::resolve`] の絶対化関数群は個別に
    /// public なので、本関数を経由せず誤った基準で呼ぶ code は依然として書ける。
    /// `OwnFontSize` / `ParentFontSize` newtype による型 level の enforcement は
    /// 採らなかった — 守る距離が各関数の 2 行しかない一方、public 関数 8 本と
    /// その doctest の signature churn を伴うため。
    pub fn finalize(
        self,
        parent_font_size: ComputedLength,
        ctx: &ResolveContext,
    ) -> ComputedValues {
        // phase 2: font-size を **親基準** で絶対化する (CSS Values 4 §6.1.1
        // parent-metrics 条項)。
        let font_size = resolve_font_size(self.font_size, parent_font_size, ctx);
        // phase 3: 残りを **自 node の** font-size 基準で絶対化する。
        self.absolutize_with(font_size, ctx)
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
    /// `line-height` も phase 3 側 (自 font-size 基準) で正しい — 同 §の
    /// "The other font-relative lengths continue to resolve against the
    /// element's own metrics when used in line-height." に一致する。parent
    /// metrics 側に倒れるのは `lh` / `rlh` だけで、両 unit は未実装
    /// (bd raikiri-spike-2x8)。
    ///
    /// # 引数を取らない理由と、その前提
    ///
    /// 両 phase の基準がいずれも本関数の中で決まるので、呼び出し側が渡し間違える
    /// 余地が無い。**ただしこれは「呼び出し側が本当に親を持たない node にしか
    /// 本関数を使わない」ことが前提**である — 親の computed font-size を捨てて
    /// initial に固定するのが正しいのは §6.1.1 の "if the element has no parent"
    /// が成立するときだけ。[`crate::cascade::resolve_inheritance`] はその invariant
    /// を `debug_assert` で pin している (同関数の `None` arm の comment 参照)。
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
        // phase 3: `rem` の基準は「root element の computed font-size」= 自分。
        self.absolutize_with(font_size, &ResolveContext::new(font_size))
    }

    /// phase 3 — 自 node の確定済 computed `font-size` を基準に残りを絶対化する。
    ///
    /// `parent_font_size` を **意図的に受け取らない** ([`Self::finalize`] doc の
    /// 担保 1)。
    fn absolutize_with(self, font_size: ComputedLength, ctx: &ResolveContext) -> ComputedValues {
        ComputedValues {
            color: self.color,
            background_color: self.background_color,
            font_family: self.font_family,
            font_size,
            font_weight: self.font_weight,
            line_height: resolve_line_height(self.line_height, font_size, ctx),
            display: self.display,
            counter_reset: self.counter_reset,
            counter_increment: self.counter_increment,
            counter_set: self.counter_set,
            content: self.content,
            string_set: self.string_set,
            running_templates: self.running_templates,
            text_align: self.text_align,
            padding: self
                .padding
                .map(|l| resolve_length_percentage(l, font_size, ctx)),
            margin: self
                .margin
                .map(|l| resolve_length_percentage_or_auto(l, font_size, ctx)),
            border: self.border.map(|b| resolve_border(b, font_size, ctx)),
            width: resolve_length_percentage_or_auto(self.width, font_size, ctx),
            height: resolve_length_percentage_or_auto(self.height, font_size, ctx),
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
    };

    const PARENT_FS: ComputedLength = ComputedLength(INITIAL_FONT_SIZE_PX);

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
            SpecifiedValues::initial().finalize(PARENT_FS, &ResolveContext::initial()),
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
            font_family: vec![Atom::from("sans-serif")],
            font_size: ComputedLength(24.0),
            font_weight: 700,
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
        assert_eq!(child.font_weight, 700);
        // CSS Text 3 §6.1: text-align は inherited。
        assert_eq!(child.text_align, TextAlign::Center);
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
        let computed = child.finalize(ComputedLength(10.0), &CTX);
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
        let cv = sv.finalize(ComputedLength(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(32.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(32.0));
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
        let cv = sv.finalize(ComputedLength(16.0), &CTX);
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
        let cv = sv.finalize(ComputedLength(64.0), &ctx);
        assert_eq!(cv.font_size, ComputedLength(40.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
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
        let cv = sv.finalize(ComputedLength(16.0), &CTX);
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(1.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(16.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(4.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Percent(5.0));
    }
}
