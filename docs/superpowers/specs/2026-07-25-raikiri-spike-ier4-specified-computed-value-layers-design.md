# raikiri-style: specified value 層 / computed value 層の分離 (設計判断)

- **bd issue**: raikiri-spike-ier4 (type/decision, architectural)
- **Related (blocked by this)**: raikiri-spike-yqh (line-height `<percentage>`), raikiri-spike-2x8 (Length unit 拡張)
- **bd decision**: raikiri-spike-082k
- **Follow-up (implementation)**: raikiri-spike-i5bs (Phase 1, additive), raikiri-spike-zls8 (Phase 2, atomic swap)
- **Author**: Mitsuru Hayasaka
- **Date**: 2026-07-25
- **Sprint**: Sprint 26 (sprint/style/10)
- **Status**: Decided — Option A (完全な層分離)
- **Amended**: 2026-07-26 (bd raikiri-spike-jaww) — §6.3 の `ResolveContext.root_font_size` を `f32` から `ComputedLength` に変更。型精度のみの変更で、決定内容 (Option A) と意味論は不変。

## 0. 結論 (summary)

**Option A — specified value 層と computed value 層を別型にする。**

- spec は **Option C (現状維持 + 下流 resolve) を排除する**。現行 code は
  `font-size: em` の inheritance chain で **情報を破壊しており**、下流の
  どの consumer もその情報を復元できない (§4.3 に証明)。これは TODO ではなく
  live な correctness bug である。
- spec は A と B のどちらか一方を強制しない (§4.4)。A を選ぶのは
  **engineering 判断**であり、根拠は raikiri-spike-2x8 による負債増幅
  (§4.5) と、defensive fail-quiet を型で不可能にできること (§4.6)。
- `Length` の名前は **specified 層に残す** (parse 側 172 参照 + parse test 群の
  churn 0)。computed 層に新型を導入する。
- **Percent は computed 層に残す** (padding / margin / width / height)。
  **font-size / line-height の Percent のみ computed 時に絶対化する**
  (§5)。これにより raikiri-spike-yqh は Option A 相当 (declaring element の
  computed font-size で resolve) に確定する。
- code 波及は **raikiri-style + raikiri-dom + raikiri (umbrella) の 3 crate**。
  raikiri-paint / raikiri-html / raikiri-traits は manifest 依存はあるが
  `Length` 型 surface を持たない (§7 に実 grep)。

## 1. Cleanroom 宣言

本書の設計結論は **CSS spec (W3C TR) のみ**から derive した。参照した
primary source は §2 に verbatim + anchor-fragment URL で列挙する。

- Stylo 内部実装は**一切参照していない**。
- ier4 issue description が corroboration として挙げた blitz-visible surface
  (`packages/blitz-dom/**` の `style::values::computed::{Length, CSSPixelLength}`
  消費側) は **本書の根拠に用いていない**。本書の全 rationale は §2 の spec 引用と
  §3 の実 code read から閉じている。
- `wall/cleanroom` escalate 事由なし。

## 2. Primary sources (2026-07-25 に実取得、verbatim)

取得方法: `https://www.w3.org/TR/css-values-4/` 等を実 fetch し、HTML から
tag を除去した plain text 上で該当文を抽出。anchor id は同 HTML の
`<h* id="...">` から実確認した (推測 anchor なし)。

### 2.1 CSS Values 4 §6 Distance Units — <https://www.w3.org/TR/css-values-4/#lengths>

> The specified value of a length (specified length) is represented by its
> quantity and its unit. The computed value of a length (computed length) is
> the specified length resolved to an absolute length, and its unit is not
> distinguished: it can be represented by any absolute length unit (but will
> be serialized using its canonical unit, px).

**読み**: computed 層の length は「絶対長」であり、単位は**値としては**区別
されない。ただしこれは *値* についての規定であって *表現* の規定ではない
(「any absolute length unit で表現してよい」)。表現上の制約は serialization
のみ (canonical unit = px)。**この文だけでは Option A と Option B を判別できない**
— §4.4 でこの点を明示する。

### 2.2 CSS Values 4 §6.2 Absolute Lengths — <https://www.w3.org/TR/css-values-4/#absolute-lengths>

> All of the absolute length units are compatible, and px is their canonical
> unit.

換算: `1in = 96px`, `1pt = 1/72in` → `1pt = 4/3 px` (現行 `layout.rs:338` の
`v * 4.0 / 3.0` と一致)。

### 2.3 CSS Values 4 §6.1.1 Font-relative Lengths — <https://www.w3.org/TR/css-values-4/#font-relative-lengths>

`em` (<https://www.w3.org/TR/css-values-4/#em>):

> Equal to the computed value of the font-size property of the element on
> which it is used.

`rem` (<https://www.w3.org/TR/css-values-4/#rem>):

> Equal to the computed value of the em unit on the root element.

同 §6.1.1 の self-reference 回避条項:

> When used in the value of any font-\* property on the element they refer to,
> the font-relative lengths resolve against the computed metrics of the parent
> element—or against the computed metrics corresponding to the initial values
> of the font and line-height properties, if the element has no parent.

**読み**: `em` は 2 通りの参照先を持つ。`font-size` 以外の property では
**自要素の computed font-size**、`font-size` property 自身では
**親要素の computed font-size** (親がなければ initial value = 16px)。
`rem` は root element 上の `em`、すなわち root element の computed font-size。
root element の `font-size: 1rem` は上記 parent-metrics 条項に従い
initial value 基準になる (**derived、verbatim 引用ではない** — CSS Values 4
には rem の root 自己参照を明示する一文が見当たらなかったため、§6.1.1 の
parent-metrics 文から導出した)。

### 2.4 CSS Values 4 §5.5 Percentages — <https://www.w3.org/TR/css-values-4/#percentages>

> Percentage values are always relative to another quantity, for example a
> length. Each property that allows percentages also defines the quantity to
> which the percentage refers. This quantity can be a value of another property
> for the same element, the value of a property for an ancestor element, a
> measurement of the formatting context (e.g., the width of a containing
> block), or something else.

### 2.5 CSS Values 4 §5.5.1 Computation and Combination of `<percentage>` — <https://www.w3.org/TR/css-values-4/#combine-percentages>

> Unless otherwise specified (such as in font-size, which computes its
> `<percentage>` values to `<length>`), the computed value of a percentage is
> the specified percentage.

**読み**: **percentage は原則 computed 層に percentage のまま残る。** これが
「computed 層 = px 単一表現」という素朴なモデルを否定する決定的な一文であり、
computed 層の型は property ごとに `<length>` と `<length-percentage>` に
分かれる (§5)。

### 2.5b CSS Values 4 §5.6 / §5.6.1 Mixing Percentages and Dimensions — <https://www.w3.org/TR/css-values-4/#mixed-percentages> / <https://www.w3.org/TR/css-values-4/#combine-mixed>

§5.6 は `<length-percentage>` を **percentage-dimension mix** として定義する:

> In cases where a `<percentage>` can represent the same quantity as a dimension
> in the same component value position, and can therefore be combined with them
> in a calc() expression, the following convenience notations may be used in the
> property grammar: `<length-percentage>` Equivalent to \[ `<length>` |
> `<percentage>` \], where the `<percentage>` will resolve to a `<length>`.

§5.6.1 (<https://www.w3.org/TR/css-values-4/#combine-mixed>):

> The computed value of a percentage-dimension mix is defined as
> - a computed dimension if the percentage component is zero or is defined
>   specifically to compute to a dimension value
> - a computed percentage if the dimension component is zero
> - **a computed calc() expression otherwise**

**読み**: computed 層の `<length-percentage>` は **px / percentage / calc() の
3 形態**を取りうる。§2.5 は「percentage は percentage のまま computed される」と
言っているだけで、**型を 2 択に閉じてはいない**。

`calc()` は Epic 5 (css-variables-and-math) scope であり本 decision の
non-goal だが、**computed 層の型設計に将来 `Calc` variant が加わることは
spec から確定している**。§6.1 の `#[non_exhaustive]` 判断はこの事実を
踏まえた explicit trade として扱う (§4.6 / §6.1)。

なお `ComputedLength` (font-size / border-width) はこの影響を受けない —
`font-size: calc(1em + 2px)` は computed 時に完全に `<length>` へ解決される
ため、px scalar 表現のままでよい。**根拠は §5.6.1 ではない** — 同 section が
規定するのは *percentage 成分と dimension 成分の混合* であり、`1em + 2px` は
dimension 同士の加算なので当該例を支えない。正しい根拠は §2.1 (CSS Values 4 §6
<https://www.w3.org/TR/css-values-4/#lengths>「computed length は絶対長へ
resolve される」) と math function の simplification 規則 (計算可能な成分は
computed 時に畳まれる)。露出は
`ComputedLengthPercentage` / `ComputedLengthPercentageOrAuto` に限られる。

### 2.6 CSS Cascade 5 §4 Value Processing — <https://www.w3.org/TR/css-cascade-5/#value-stages>

§4.4 Computed Values (<https://www.w3.org/TR/css-cascade-5/#computed>):

> The computed value is the result of resolving the specified value as defined
> in the “Computed Value” line of the property definition table, generally
> absolutizing it in preparation for inheritance.
>
> Note: The computed value is the value that is transferred from parent to
> child during inheritance.

§7.2 Inheritance (<https://www.w3.org/TR/css-cascade-5/#inheriting>):

> The inherited value of a property on an element is the computed value of the
> property on the element’s parent element. For the root element, which has no
> parent element, the inherited value is the initial value of the property.

§4.5 Used Values (<https://www.w3.org/TR/css-cascade-5/#used>):

> The used value is the result of taking the computed value and completing any
> remaining calculations to make it the absolute theoretical value used in the
> formatting of the document.

**読み**: 「inheritance は computed value を運ぶ」が **normative**。したがって
inheritance の時点で em/rem は既に絶対化されていなければならない。これが §4.3 の
Option C 排除論の骨子。

### 2.7 CSS Inline 3 §5.1 line-height — <https://www.w3.org/TR/css-inline-3/#propdef-line-height>

property definition table (verbatim):

> Value: normal | `<number [0,∞]>` | `<length-percentage [0,∞]>`
> Initial: normal
> Inherited: yes
> Percentages: computed relative to 1em
> Computed value: the specified keyword, a number, or a computed `<length>` value

**読み**: line-height の computed value に **percentage は存在しない**。
`<percentage>` は computed 時に自要素の computed font-size に対して絶対化される
(`Percentages: computed relative to 1em` + `1em` = §2.3 より自要素 computed
font-size)。`<number>` は computed 層でも number のまま (spec 上の
load-bearing な distinction — 子は number を inherit して自分の font-size に掛ける)。

### 2.8 CSS Box 3 margin-top / padding-top — <https://www.w3.org/TR/css-box-3/#propdef-margin-top> / <https://www.w3.org/TR/css-box-3/#propdef-padding-top>

> margin-top — Percentages: refer to logical width of containing block
> margin-top — Computed value: the keyword auto or a computed `<length-percentage>` value
> padding-top — Percentages: refer to logical width of containing block
> padding-top — Computed value: a computed `<length-percentage>` value

**読み**: margin / padding の percentage は **computed 層に残る** (§2.5 の原則どおり)。
containing block width への解決は **used value 層** (§2.6 §4.5) — つまり taffy の責務。

## 3. 現状 (実 read、2026-07-25 時点の worktree-ier4 HEAD)

### 3.1 単一 `Length` 型が 2 層を兼ねている

`crates/raikiri-style/src/property.rs:216` の `Length` は
`Px | Em | Rem | Percent | Pt` の 5 variant + `#[non_exhaustive]`。

- **parse 出力**: `parse_length_value` (`property.rs:1841`) が
  `px / em / rem / pt / %` を対応 variant に写す。
- **computed field 型**: `crates/raikiri-style/src/computed.rs:62` の
  `pub font_size: Length`、同 `:178 padding: Sides<Length>`、
  `:199 margin: Sides<LengthOrAuto>`、`:268 width` / `:295 height`、
  `:245 border: Sides<Border>` (Border.width: Length)。
- **接続点**: `cascade.rs:342` `PropertyValue::FontSize(s) => target.font_size = s`
  — parse 結果が変換なしで `ComputedValues` に代入される。

### 3.2 cascade.rs の walk 構造 (受入条件 3 の実確認)

`crates/raikiri-style/src/cascade.rs:239` `resolve_inheritance`:

- **tree walk を持つ**。explicit `Vec` stack による top-down iterative DFS
  (`:246` `let mut stack: Vec<(StyleNodeId, ComputedValues)> = vec![(id, parent_computed.clone())];`)。
  各 stack entry が **親の `ComputedValues` を明示的に運んでいる** (`:293`
  `stack.push((child_id, computed.clone()))`)。
  → **em の resolve に必要な「親の computed font-size」は既に walk に存在する。**
  新たな tree 走査も、`StyleDom` trait への追加も不要。
- **root font-size (`rem` の参照値) は保持していない**。
  `grep -n "root_font\|root_size\|RootFont" crates/raikiri-style/src/cascade.rs`
  → **0 件** (実行済、空出力)。
- walk の起点は `dom.root_id()` (`cascade.rs:62`, `:77`)。
  `crates/raikiri-style/src/style_dom.rs:88` の contract:
  `- root_id() returns the Document node (usually StyleNodeId(0)).`
  → **root_id は root *element* ではなく Document node**。`rem` の参照値は
  walk が最初に到達した `StyleNodeKind::Element` の computed font-size として
  捕捉し、既存の stack に相乗りさせる必要がある。
- 初期 parent は `ComputedValues::initial()` (`cascade.rs:78`)、
  `computed.rs:331` `font_size: Length::Px(16.0)` — §2.3 の
  「親がなければ initial value」条項に対応する値が既に存在する。
- `pub fn cascade<D: StyleDom>(dom: &D, rule_tree: &RuleTree)` (`cascade.rs:58`)
  の **signature 変更は不要** — root font-size context は関数内部で完結する。
  → **`wall/traits` の crossing なし。**

### 3.3 apply_value の適用順序が非決定的 (実 read で発見)

`cascade.rs:272-277`:

```
if let Some(candidates) = cascaded.get(&id) {
    let winners = pick_winners(candidates);
    for value in winners.into_values() {
        apply_value(value, &mut computed);
    }
}
```

`pick_winners` (`cascade.rs:299`) は `HashMap<PropertyKey, PropertyValue>` を返し、
`into_values()` の iteration order は **非決定的**。したがって
「`apply_value` の中で em を resolve する」設計は成立しない —
`padding: 2em` の winner が `font-size: 20px` の winner より先に適用される
可能性があるため。

→ **絶対化 (computed value 化) は、その node の全 winner を適用し終えた後の
独立した phase でなければならない。** これは §6 の設計に直結する。

### 3.4 下流の defensive fail-quiet

`crates/raikiri-dom/src/layout.rs`:

- `:335` `length_to_taffy_length_percentage` — `:342` `Length::Em(_) | Length::Rem(_) => LengthPercentage::length(0.0)`
- `:355` `length_or_auto_to_taffy_dimension` — `:363` 同上
- `:377` `length_or_auto_to_taffy_lpa` — `:386` 同上
- `:330` `/// TODO: font-size context を cascade で resolve 済にして em/rem を実 px 値へ。`
- `:344` / `:364` / `:388` — `#[non_exhaustive]` catch-all `_ => length(0.0)` (fail-quiet)
- `:453-463` `preshape_text` — `Length::Px(v) => v` 以外は `warn!` + fallback

`Length::` の出現は layout.rs 内 **23 箇所** (`grep -c "Length::"` 実行値)。

## 4. Option 評価

### 4.1 Option A — 完全な層分離

`Length` (specified、authored unit 保持) と computed 層の新型群を別型にする。
`ComputedValues` の length 系 field を computed 型に置き換える。

| 観点 (spec) | 評価 |
|---|---|
| CSS Values 4 §6 computed = 絶対長 <https://www.w3.org/TR/css-values-4/#lengths> | **満たす**。computed 型の length 成分が px scalar のみになる。 |
| CSS Values 4 §5.5.1 percentage は computed に残る <https://www.w3.org/TR/css-values-4/#combine-percentages> | **満たす**。`<length-percentage>` を取る property 用に Percent variant を持つ別型を用意する (§5)。 |
| CSS Values 4 §5.6.1 calc() mix <https://www.w3.org/TR/css-values-4/#combine-mixed> | **将来 variant 追加が要る**。Epic 5 で `Calc` variant を足す coordinated breaking change になる (§4.6 で trade として明示)。 |
| CSS Cascade 5 §7.2 inheritance は computed を運ぶ <https://www.w3.org/TR/css-cascade-5/#inheriting> | **満たす**。inherit されるのは computed 型のみ。 |
| CSS Inline 3 §5.1 line-height computed に % なし <https://www.w3.org/TR/css-inline-3/#propdef-line-height> | **型で表現できる**。`ComputedLineHeight` は Percent variant を持たない。 |
| 下流の fail-quiet | **型レベルで消える**。Em/Rem arm も catch-all arm も書けなくなる。 |
| cost | `Length` public 型が絡む破壊的変更 (§7)。 |

### 4.2 Option B — 単層 + computed 時 px 正規化

`Length` 1 型のまま、`apply_value` 到達後に絶対単位 / Em / Rem を `Px` に正規化する。

| 観点 (spec) | 評価 |
|---|---|
| CSS Values 4 §6 <https://www.w3.org/TR/css-values-4/#lengths> | **満たす**。値としては正しい computed length になる (§4.4 のとおり spec は表現を規定しない)。 |
| CSS Values 4 §5.5.1 <https://www.w3.org/TR/css-values-4/#combine-percentages> | **満たす** (Percent variant を残すだけ)。 |
| CSS Values 4 §5.6.1 <https://www.w3.org/TR/css-values-4/#combine-mixed> | **満たす**。Epic 5 の `Calc` variant は共有 enum に足せば済み、`#[non_exhaustive]` のため下流は **source 互換** (既存 match に新 arm を書かずに済む。再 compile 自体は dep 変更により当然発生する) — ただしそれは §4.5 の fail-quiet と表裏。 |
| CSS Cascade 5 §7.2 <https://www.w3.org/TR/css-cascade-5/#inheriting> | **満たす**。 |
| CSS Inline 3 §5.1 <https://www.w3.org/TR/css-inline-3/#propdef-line-height> | **型では表現できない**。`ComputedValues.line_height` に
`LineHeight::Length(Length::Percent(..))` を代入する code は依然 compile が通る。 |
| 下流の fail-quiet | **残る**。`layout.rs:342/363/386` の `Em(_) | Rem(_) => 0.0` arm は
`Length` が Em/Rem variant を持ち続ける限り match の網羅性のために必要。
到達不能になるだけで、削除できない (= 「本当に到達しないか」は型ではなく
invariant comment が保証する)。 |
| cost | public 型分割なし。破壊的変更は最小。 |

### 4.3 Option C — 現状維持 + 下流 resolve — **spec が排除する**

各 consumer が `ComputedValues` を読む時点で em/rem を resolve する。

**Option C は CSS Cascade 5 §7.2 (§2.6) に違反し、かつ下流では原理的に修復不能。**

証明 (実 code に基づく):

1. `computed.rs:409` `font_size: parent.font_size` — `inherit_from` は親の
   `font_size` field を**そのまま** copy する。
2. `cascade.rs:342` `PropertyValue::FontSize(s) => target.font_size = s` —
   自 node に `font-size` の winner があれば parse 結果をそのまま代入する。
3. したがって以下 2 ケースの `ComputedValues.font_size` は
   **どちらも `Length::Em(1.5)` になり、区別がつかない**:

   - (i) `<div style="font-size:1.5em"><span style="font-size:1.5em">` の span
     — spec 上 `16 × 1.5 × 1.5 = 36px` (§2.3 の compounding)
   - (ii) `<div style="font-size:1.5em"><span>` の span
     — spec 上 `16 × 1.5 = 24px` (inherit、compounding なし)

4. 下流 consumer は「この `Em(1.5)` が自 node の declaration 由来か、親からの
   inherit 由来か」を `ComputedValues` から知る術がない。**cascade の時点で
   情報が破壊されている。** どんな下流 resolve pass を書いてもこの 2 ケースを
   分離できない。

これは「未実装の TODO」ではなく **live な correctness bug** である
(現状は `layout.rs:342` の defensive 0.0 で全 em が 0px に潰れるため
表面化していないだけで、`layout.rs` を「正しく」直すと (i)/(ii) の誤差が出る)。

Option C は **spec 準拠が到達不能**なので棄却する。

### 4.4 A と B の判別に spec は使えない — 明示

§2.1 は「computed length の単位は区別されない、**任意の**絶対単位で表現してよい」
と述べている。これは *値* の規定であり、実装の *表現* を A に強制しない。
Option B (単一 enum + 正規化) も spec-correct な値を produce する。

したがって本書は **「spec が C を排除する → A と B の選択は engineering 判断」**
という構造で結論を出す。「spec が A を要求する」とは主張しない。

### 4.5 A を B より優先する理由 (1) — raikiri-spike-2x8 の負債増幅

raikiri-spike-2x8 は `vw/vh/ch/ex/cm/mm/in/pc/Q/cap/rcap/ic/ric/lh/rlh` の
**15 unit** を追加する。

- **Option B の場合**: 15 unit すべてが共有 `Length` enum の variant になる。
  `Length` は `#[non_exhaustive]` (`property.rs:215`) なので下流は
  再 compile を強制されず、`layout.rs:344/364/388` の catch-all
  `_ => length(0.0)` が**黙って新 unit を 0px に潰す**。すなわち
  §3.4 の fail-quiet debt がそのまま 15 unit 分増える。「正規化を必ず通す」
  invariant は型ではなく人間の規律で保つことになる。
  (これは §4.2 表の「§5.6.1 を満たす」と表裏の関係にある — B は新 variant を
  下流に伝えずに足せるが、それは下流が黙って落とすことと同義である。)
- **Option A の場合**: 15 unit は specified 層 (`Length`) にのみ追加され、
  絶対化関数の match を 15 arm 増やすだけで閉じる。computed 型は不変なので
  **下流 crate は 1 行も変わらない**。

2x8 は本 decision の**後**に、**specified 層のみを触る task** として再 scope する
(§8)。

### 4.6 A を B より優先する理由 (2) — fail-quiet を型で不可能にする

Option A では `layout.rs` の 3 bridge が以下になる:

- `padding` bridge: `ComputedLengthPercentage` の `Px | Percent` **2 arm 網羅**、
  catch-all 不要 → `:342` の Em/Rem arm と `:344` の `_ => 0.0` が**削除**される。
- `width/height/margin` bridge: `Px | Percent | Auto` の **3 arm 網羅** → 同上。
- `preshape_text` (`:453-463`): `font_size` が px scalar になるため
  match 自体が消え、`warn!` fallback も消える。

すなわち `ier4` description の負債 1 (defensive-0.0 3 箇所 + TODO 1 箇所) が
**型検査で保証された形で** 解消する。Option B ではこれらの arm は
「到達しないはずだが書かねばならない」状態で残り続ける。

#### 4.6.1 網羅 match を取るための trade — `#[non_exhaustive]` を付けない判断

上記の「catch-all 不要」は、`ComputedLengthPercentage` /
`ComputedLengthPercentageOrAuto` に `#[non_exhaustive]` を**付けない**ことで
初めて成立する。これは **spec が variant 数を 2 に閉じているからではない** —
§2.5b のとおり CSS Values 4 §5.6.1 は computed `<length-percentage>` に
`calc()` 形態を認めており、Epic 5 (css-variables-and-math) で `Calc` variant が
**確実に増える**。

したがってこれは **explicit trade** として記録する:

- **得るもの**: 今すぐ網羅 match が書け、`layout.rs` の defensive `_ => 0.0` を
  削除できる (§4.6)。fail-quiet の class が型検査で閉じる。
- **払うもの**: Epic 5 で `Calc` variant を追加する際、
  raikiri-style / raikiri-dom / raikiri の 3 crate を跨ぐ
  **coordinated breaking change** が 1 回発生する。
- **なぜこの trade を取るか**: `calc()` の追加は Epic 5 という**予定された
  1 回の作業**であり、その時点で下流に「新形態が来た」ことを compile error で
  強制通知できる方が、`#[non_exhaustive]` にして黙って 0px に落とすより安全。
  §4.5 の 15 unit と違い、`Calc` は下流が**必ず対応すべき**形態である。

**`ComputedLength` (font-size / border-width) はこの trade の対象外** —
§2.5b のとおり `font-size: calc(1em + 2px)` は computed 時に完全な `<length>` に
解決される (根拠は §6 の「computed length は絶対長へ resolve」+ math function の
simplification。§5.6.1 は percentage×dimension の混合を扱う section なので
この例の根拠にはならない) ため、f32 newtype 表現は calc() 後も不変。

### 4.7 決定

**Option A を採用する。**

## 5. Percent の扱い — computed か used か (property 別)

§2.5 の原則「computed value of a percentage is the specified percentage」に、
property definition table の `Computed value:` 行が上書きをかける形で決まる。

| property | spec 行 | Percent は computed に残るか | 参照値 | 解決層 |
|---|---|---|---|---|
| `font-size` | §2.5 括弧内「font-size, which computes its `<percentage>` values to `<length>`」 | **残らない** | 親の computed font-size | computed |
| `line-height` | §2.7 `Computed value: … a computed <length> value` / `Percentages: computed relative to 1em` | **残らない** | **自要素の** computed font-size | computed |
| `padding-*` | §2.8 `Computed value: a computed <length-percentage> value` | **残る** | containing block の logical width | **used** |
| `margin-*` | §2.8 `Computed value: the keyword auto or a computed <length-percentage> value` | **残る** | containing block の logical width | **used** |
| `width` / `height` | CSS Sizing 3、Box 3 と同型 (`<length-percentage>`) | **残る** | containing block | **used** |
| `border-*-width` | grammar が `<percentage>` を取らない (`computed.rs:216` に既記) | **そもそも流入しない** | — | computed |

**帰結**: computed 層には **2 系統の length 表現**が必要である。

1. `<length>` のみを取る property 用 — px scalar 単一表現
   (calc() 後も不変、§2.5b)
2. `<length-percentage>` を取る property 用 — px か percentage
   (+ Epic 5 で calc()、§2.5b / §4.6.1)

「computed 層 = px 単一」という素朴なモデルは §2.5 に反するので採らない。

**used value 層 (containing block % の解決) は taffy に委譲する** —
`layout.rs` の bridge が `Percent(p) => LengthPercentage::percent(p / 100.0)`
で taffy に渡す現行方針は §2.6 §4.5 の used value の定義に一致しており、
本 decision で変更しない。

## 6. 採用設計 (実装 task への指示)

### 6.1 型 inventory

`Length` の**名前は specified 層に残す**。理由: raikiri-style 内の
`Length::` 参照は 172 箇所 (`property.rs` 内 grep 実行値) で、その大半は
parser と parse test。`SpecifiedLength` へ rename すると diff が 2 倍になり、
本 decision の本質 (層の分離) と無関係な churn になる。

| `ComputedValues` field | 現行型 (`computed.rs`) | computed 層の型 | 根拠 |
|---|---|---|---|
| `font_size` | `Length` (:62) | `ComputedLength` (px newtype) | §2.5 (% → length) |
| `line_height` | `LineHeight` (:74) | `ComputedLineHeight { Normal, Number(f32), Length(ComputedLength) }` | §2.7 |
| `padding` | `Sides<Length>` (:178) | `Sides<ComputedLengthPercentage>` | §2.8 |
| `margin` | `Sides<LengthOrAuto>` (:199) | `Sides<ComputedLengthPercentageOrAuto>` | §2.8 |
| `border` | `Sides<Border>` (:245) | `Sides<ComputedBorder>` (`width: ComputedLength`) | border-width に % なし |
| `width` / `height` | `LengthOrAuto` (:268/:295) | `ComputedLengthPercentageOrAuto` | §2.8 と同型 |

新規 public 型 (raikiri-style):

- `ComputedLength(f32)` — px。`#[non_exhaustive]` にしない。単一 field の
  newtype であり、§2.1 (単位は区別されない / computed length は絶対長へ
  resolve される) + math function の simplification により `font-size:
  calc(1em + 2px)` も computed 時に完全な `<length>` へ解決されるため、
  **calc() 導入後も表現が変わらない**。
- `ComputedLengthPercentage { Px(f32), Percent(f32) }` — §2.5。
  **`#[non_exhaustive]` にしない**。ただしこれは
  「spec が 2 択に閉じているから」**ではない** — §2.5b のとおり
  CSS Values 4 §5.6.1 は computed `<length-percentage>` に calc() 形態を
  認めており、Epic 5 で `Calc` variant が加わる。網羅 match を今得るための
  **explicit trade** であり、その代償 (3 crate coordinated breaking change) を
  §4.6.1 に記録した。**実装時の doc comment にはこの trade をそのまま書くこと**
  (「spec が 2 択に閉じている」と書かないこと — それは誤り)。
- `ComputedLengthPercentageOrAuto { Px(f32), Percent(f32), Auto }` — 同上 + `auto`。
  同じ trade が適用される。
- `ComputedLineHeight`、`ComputedBorder`。

`LineHeight` / `Border` / `LengthOrAuto` は specified 層の型として残す
(parse 出力)。

### 6.2 絶対化 phase を独立させる (§3.3 の帰結)

`resolve_inheritance` の per-node 処理を 3 phase に分ける:

```
phase 1 (order-independent): specified value を staging
    SpecifiedValues を親の ComputedValues から seed し、
    その node の全 cascade winner を apply する。
    → HashMap の非決定的 iteration order は無害になる。

phase 2: font-size を絶対化
    §2.3 の parent-metrics 条項に従い **親の computed font-size** を基準にする。
    親がなければ initial (16px)。
    root element ならここで root_font_size context を確定させる。

phase 3: 残り全 length を絶対化
    §2.3 に従い **自 node の (phase 2 で確定した) computed font-size** を
    em の基準、root_font_size を rem の基準にする。
    line-height の Percent はここで自 font-size に対して絶対化する (§2.7)。
    padding / margin / width / height の Percent はここでは触らない (§5)。
```

**staging 構造について**: 型レベルの保証 (§4.6) を得るには
`ComputedValues` とは別の staging 表現が要る。推奨は `SpecifiedValues` struct
(length 系 field のみ specified 型、他 field は `ComputedValues` と同型)。
inherited property の seed は「親の computed 値を specified 表現に lift する」
(`ComputedLength(px) → Length::Px(px)`) で行う — §2.1 が computed length を
「任意の絶対単位で表現してよい」としており、px として表現するのは値の恒等変換
なので lossless。lift 後に phase 3 の絶対化を通しても `Px` は不動点なので
二重適用の危険はない。

代替 (`Values<L>` の generic 化) も実装可能だが、length 以外の ~15 field を
不要に型パラメータ越しにするため推奨しない。最終形は実装 task の裁量とするが、
**「絶対化が winner 適用とは別 phase であること」は本 decision の拘束事項**とする。

**staging の実際の負荷は小さい (実 read)**: `computed.rs:389-391` の doc が
列挙する inherited property は color / font-family / font-size / font-weight /
text-align / line-height であり、このうち **length 系は `font_size` と
`line_height` の 2 つだけ**。padding / margin / border / width / height は
すべて non-inherited で initial value から seed される (`inherit_from` の
`:443` / `:446` / `:451` が実際に initial を書いている)。すなわち
「computed → specified の lift」を要するのは **2 field のみ**であり、
~20 field の parallel struct を作る必要は必ずしもない。

lift の losslessness は 2 field 両方で確認済:
`ComputedLineHeight::Length(ComputedLength(30))` →
`LineHeight::Length(Length::Px(30))` → phase 3 → `ComputedLength(30)`。
すなわち **font-size がより小さい子は 30px をそのまま継承し、
percentage を再解決しない** — これが CSS Inline 3 §5.1 の要求する挙動
(percentage は宣言要素で絶対化され、子はその length を継承する)。
`Number(1.5)` は素通しして子自身の font-size に掛かる。
**この「Px が絶対化の不動点である」性質が lift を成立させているので、
実装時に「再 resolve するべきでは」と直してはならない。**

### 6.3 rem context (§3.2 の帰結)

`resolve_inheritance` の stack entry を
`(StyleNodeId, ComputedValues, ResolveContext)` に拡張し、
`ResolveContext { root_font_size: ComputedLength }` を運ぶ
(bd raikiri-spike-jaww で `f32` → `ComputedLength` に変更 — 本 field が保持するのは
§6.1 の computed `<length>` そのものであり、絶対化関数の戻り値をそのまま格納できる
形に揃えた。consumer 0 の Phase 1 完了直後に確定させたもので、意味論は不変)。

- 初期値は `ComputedValues::initial().font_size` 相当 (16px)。
- walk が最初に `StyleNodeKind::Element` に到達した node
  (= root element) の phase 2 完了時に `root_font_size` を確定して以降の
  child push に伝播する。`root_id()` は Document node なので (§3.2)、
  Document node 自身では確定させない。
- root element 自身の `font-size: Nrem` は §2.3 parent-metrics 条項により
  initial (16px) 基準 — 上記の初期値がそのまま使われるので追加分岐は不要。

`StyleDom` trait / `pub fn cascade` の signature 変更は不要 (§3.2)。

## 7. 波及範囲 (受入条件 4)

### 7.1 manifest 依存 (kebab-case grep)

`grep -rn "raikiri-style" --include=Cargo.toml .` の実出力より、
raikiri-style に依存する crate は **5 つ**:

| Cargo.toml | 行 |
|---|---|
| `crates/raikiri-dom/Cargo.toml` | `13: raikiri-style  = { workspace = true }` |
| `crates/raikiri-paint/Cargo.toml` | `12: raikiri-style  = { workspace = true }` |
| `crates/raikiri-traits/Cargo.toml` | `13: raikiri-style = { workspace = true }` |
| `crates/raikiri-html/Cargo.toml` | `20: raikiri-style  = { workspace = true }` |
| `crates/raikiri/Cargo.toml` | `11: raikiri-style  = { workspace = true }` |

(+ workspace root `Cargo.toml:5` member / `:24` workspace dependency)

### 7.2 `Length` の crate 外実参照 (snake_case / 型名 grep)

`grep -rn "\bLength\b" --include=*.rs crates/ | grep -v "^crates/raikiri-style/"`
の実出力を精査した結果 (planner の見積りを 1 点訂正):

| crate | `Length` 型の実参照 | 備考 |
|---|---|---|
| `raikiri-dom` | **あり** — `src/layout.rs:20` (import) + `Length::` 23 箇所 (`grep -c "Length::"` 実行値)。3 bridge helper (`:335` / `:355` / `:377`)、`used_border_width` (`:231`)、`preshape_text` (`:458`)、test 群 | **本命の波及先** |
| `raikiri` (umbrella) | **あり** — `src/lib.rs:128` re-export 一覧に `Length`、`tests/build_cascaded.rs:173` import、`:196` `let _font_size: Length = computed.font_size;` | **public surface + 型 assertion test** |
| `raikiri-paint` | **なし** — `src/lib.rs:192` は `Length::Px(0.0)` を含む**コメント文**のみ。`grep -n "Length" crates/raikiri-paint/src/*.rs` から comment 行を除くと **0 件** | ier4 description の「`raikiri-paint/src/lib.rs:192`」は型参照ではない (**訂正**) |
| `raikiri-html` | **なし** — `grep -n "\bLength\b" crates/raikiri-html/src/*.rs` → 0 件 | 再 compile のみ |
| `raikiri-traits` | **なし** — hit は `src/lib.rs:416` / `src/policy.rs:30` の `"Content-Length"` 文字列のみ。`src/page.rs:22` の raikiri-style import は `ContentComponent / ContentPart / ContentTextKeyword / CounterStyle / StringFetchMode` で Length を含まない | **`wall/traits` crossing なし** |

**結論**: manifest 上は 5 crate + umbrella が dependent だが、
**code 変更が必要なのは `raikiri-style` (owner) + `raikiri-dom` + `raikiri` の
3 crate**。残る 3 crate は再 compile のみで通る見込み。

### 7.3 Wall assessment

| wall | 判定 | 根拠 (実 read) |
|---|---|---|
| `wall/umbrella` | **crossing する** | `crates/raikiri/src/lib.rs:128` の `pub use raikiri_style::{… Length …}` に computed 型を追加する必要。`crates/raikiri/tests/build_cascaded.rs:196` の `let _font_size: Length = computed.font_size;` は **compile error になる** (breaking public API change) |
| `wall/dom-paint` | **crossing する (dom 側のみ)** | `crates/raikiri-dom/src/layout.rs` の 3 bridge + `preshape_text` + 23 `Length::` 参照。paint 側は §7.2 のとおり型参照 0 なので実質 dom 単独 |
| `wall/traits` | **crossing しない** | §7.2 のとおり raikiri-traits は `Length` を参照しない。`pub fn cascade` signature も不変 (§3.2) |
| `wall/cleanroom` | **crossing しない** | §1 |

## 8. 従属関係 (受入条件 5)

### 8.1 raikiri-spike-yqh (line-height `<percentage>` computed-value resolution)

**本 decision に従属し、Option A (declaring element の computed font-size で
resolve) に確定する。**

- §2.7 の property definition table が `Computed value: … a computed <length> value`
  と `Percentages: computed relative to 1em` を規定しており、
  §2.3 より `1em` = **自要素の computed font-size**。したがって
  「declaring element の computed font-size で resolve」が唯一の spec 準拠解。
- yqh が実装不可能だった理由 (`ComputedValues.font_size` が Em/Rem を持ちうる)
  は、本 decision の §6.1 (`font_size: ComputedLength`) + §6.2 phase 2 で解消する。
- 実装位置は §6.2 の **phase 3** (`ComputedLineHeight::Length` への絶対化)。
  すなわち **yqh は本 decision の実装 task に吸収される**性質のもので、
  独立 task として残すなら「phase 3 の line-height % arm + test」に再 scope する。

### 8.2 raikiri-spike-2x8 (Length unit 拡張)

**本 decision に従属し、実装順序が「本 decision の後」に確定する。**

- §4.5 のとおり、層分離**前**に 15 unit を足すと fail-quiet debt が 15 倍になり、
  かつ層分離時の migration 対象も 15 倍になる。
- 層分離**後**なら 2x8 は **specified 層 (`Length` enum) + 絶対化関数の match arm
  のみ**を触る task になり、下流 crate は不変。
- したがって 2x8 は「specified-layer only」に再 scope し、
  実装 task (§9) の後段に並べる。

## 9. 後続 task (受入条件 6)

本 decision の実装は 2 段に分ける。分割線は
**「単独で `cargo build --workspace` が通るか」**で引く — cargo workspace は
一括 compile されるため、`ComputedValues` の field 型を変えた瞬間に
`raikiri-dom` / `raikiri` が壊れる。したがって field 型の置換と下流 migration は
**同一 commit でなければならない**。

- **Phase 1 (`raikiri-spike-i5bs`)** — **additive、raikiri-style 内で完結、単独 merge 可**。
  computed 型群 (`ComputedLength` / `ComputedLengthPercentage` /
  `ComputedLengthPercentageOrAuto` / `ComputedLineHeight` / `ComputedBorder`) の導入、
  `SpecifiedValues` staging struct、`ResolveContext { root_font_size }`、
  絶対化関数群 (§6.2 phase 2 / phase 3 のロジック) + unit test。
  **`ComputedValues` の field 型は変えない** — この段階では新型は unit test からのみ
  exercise される。壁 crossing なし。
- **Phase 2 (`raikiri-spike-zls8`)** — **atomic swap、3 crate 同時**。
  `ComputedValues` field 型の置換、`resolve_inheritance` への 3-phase 組み込みと
  root font-size threading、`layout.rs` の 3 bridge を網羅 match に書き換え
  (defensive 0.0 と `:330` TODO を削除)、`preshape_text` の `warn!` fallback 削除、
  umbrella re-export (`raikiri/src/lib.rs:128`) と
  `raikiri/tests/build_cascaded.rs:196` の型 assertion 更新。
  §7.3 のとおり `wall/dom-paint` + `wall/umbrella` を跨ぐ。

**Non-goals** (本 decision / 実装 task の scope 外):

- `calc()` / `var()` (Epic 5 css-variables-and-math)
- viewport-relative unit の viewport size context (paint scope、下流)
- used value 層の自前実装 (containing block % は taffy 委譲を継続、§5)

## 10. 受入条件の充足対応表

| # | 受入条件 | 本書の該当箇所 |
|---|---|---|
| 1 | 3 Option を CSS Values 4 の section cite 付きで評価 (anchor-fragment URL) | §2 (全 anchor を実 HTML の `id=` から検証)、§4.1 / §4.2 / §4.3 |
| 2 | 推奨 Option 1 つ + spec から derive した rationale | §4.3 (spec が C を排除)、§4.4 (A/B は spec で判別不能と明示)、§4.5 / §4.6 (engineering 根拠)、§4.7 |
| 3 | Em/Rem resolve context の所在を cascade.rs 実コードで確認 | §3.2 (walk の stack 構造、`root_font` grep 0 件、`root_id` = Document node)、§3.3 (apply 順序) |
| 4 | 波及範囲 (5 crate の Cargo.toml + `Length` の crate 外実参照) | §7.1 (kebab-case manifest grep)、§7.2 (snake_case / 型名 grep、paint の訂正含む)、§7.3 |
| 5 | yqh / 2x8 の従属関係 | §8.1 / §8.2 |
| 6 | bd decision 1 本 + 後続実装 task | §9 + bd decision `raikiri-spike-082k` + task `raikiri-spike-i5bs` / `raikiri-spike-zls8` |
