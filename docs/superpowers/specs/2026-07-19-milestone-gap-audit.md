# Milestone Plan gap audit (CSS/HTML feature coverage)

**Date**: 2026-07-19
**Trigger**: Sprint 7 style session drain 中の user observation ("Selector 拡張 + layout property + length unit + display value が計画に含まれていない、他にもたくさんある気がする、一度整理したい")
**Source doc audited**: `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` (~4,118 line)
**Auditor**: raikiri-spike explore subagent (very thorough mode)、Sprint 7 planner session dispatch
**Purpose**: CSS/HTML feature category で Milestone Plan (M0-M8) に explicit timeline がなく、Non-Goals (§2) にも Open Questions (§15) にも記載がない gap を網羅的に inventory 化

---

**Verification pass 2026-07-20** — raikiri-spike-0vv.1

- Verified by: Mitsuru Hayasaka
- Verified at: worktree-0vv1 branched from commit `68b1aab` (main HEAD 2026-07-20)
- Coverage: 12 category = A / B / C / D / E / F / G / K / L / O / Q / U (~122 items)
- Method: grep + code read against `crates/raikiri-{style,html,dom}/` on 2026-07-20 code state。各 category table に Status column を追加し (`full` / `partial` / `stub` / `regressed` / `missing`)、category header 直下に Verification 2026-07-20 subsection (distribution + top gaps + since-audit changes) を挿入
- Sibling: raikiri-spike-0vv.2 が 12 category (H/I/J/M/N/P/R/S/T/V/W/X) + 17 surprising findings + child breakdown proposal を担当、この line 直後に自身の Verified by/at を append する

---

**Scope constraint**: bd 起票 shape の判断・spec revision proposal 作成は本 doc の scope 外 (次アクション、retro-facilitator 判断 or planner-side 判断)。本 doc は observation の source of truth。

## Executive summary

**Total gap item count**: ~130 item across 24 categories
**High conflict with primary goal**: ~90 item (契約書・レポート・招待状・証明書・目次付き技術書・混合サイズ PDF の render に必須)
**Medium conflict**: ~25 item
**Low priority / internal / spec-source gap**: ~15 item

**Most alarming findings** (surprising findings section より抜粋):

1. **3a**: Author style の CSS pipeline は M1 "cascade minimal" 以外に explicit milestone がない
2. **3b**: M1.4a Non-goals が "M2 で必要になったら足す" と書いてあるが M2 task list に対応 task がない (margin/padding が例)
3. **3c**: `RuleTree` struct に `font_face_rules`/`media_rules`/`supports_rules`/`import_rules`/`counter_style_rules` field はあるが evaluate milestone がゼロ
4. **3d**: print-first を強調する design doc なのに **`@media print` handling が完全に missing** (RuleTree field はあるが evaluate なし → 組版 fixture が silent 無視される risk)
5. **3e**: `::before` / `::after` selector 実装なしに GCPM `content: string()/counter()` は M5 first-class (spec 上 `content` は pseudo-element 経由が normal) — **矛盾**
6. **3f**: `<a>` hyperlink default underline / color が M1-M8 で担保されない (契約書・技術書で必須なのに)
7. **3g**: List rendering (`<ul>`/`<ol>`/`<li>`) の milestone assignment が完全欠落 (目次付き技術書が primary goal reference なのに)
8. **3h**: **`!important` の cascade handling への言及がゼロ**
9. **3i**: `calc()` の実装は L324 で M4 に暗黙 imply されるが M4 tasks list に該当 task がない
10. **3o**: "組版品質を primary goal" と言った瞬間に "組版に必要な CSS の union" が定義される責任が生じるが、そのカバレッジ table が本 doc に空欄

## Category 別 inventory (24 category)

### Category A: Selector 拡張 (12 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 1 / partial 1 / stub 0 / regressed 0 / missing 10。実装は M1.4 seed から進展無し。`raikiri-style/src/ruletree.rs:76-78` の `is_type_or_universal_only` gate が type + universal 以外 (class / id / attribute / combinator / pseudo-class) を持つ selector を無条件 drop する現行仕様が確定 (comment: "class/id/attr/combinator selector は m1.4 では drop")。`raikiri-style/src/lib.rs:131-135` の `PseudoClass` enum は `Hover` / `Active` の 2 variant のみ受理し L4 pseudo は `parse_non_ts_pseudo_class` で `UnsupportedPseudoClassOrElement` err に落とす。

**Top gap**: (1) **A5-A8 combinator の一括 missing** — 目次付き技術書 (primary goal reference) では `.chapter h2` / `ol > li` のような combinator が事実上必須、fixture 群を書く時点で block になる。(2) **A1 class selector** — 招待状 / 契約書 fixture 側 template を書く author が `.invoice-header` / `.signature` を書けない、Author style 経路がほぼ空機能。

**Since-audit changes**: Sprint 10 hardening (d9y.1/d9y.2/d9y.6/8yu/4kw etc) は selector shape に触れず、`1ll` は `parse_string_set` trailing-comma strict 化で property 側 change、Category A は 2026-07-19 → 2026-07-20 で code drift 無し。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| A1 | Class selector `.foo` | mention なし | milestone 未割当 | missing | `crates/raikiri-style/src/ruletree.rs:76-78` (`is_type_or_universal_only` gate が drop) |
| A2 | ID selector `#foo` | mention なし | 同 | missing | 同上 |
| A3 | Attribute selector `[type=text]` / `[href^="https"]` | mention なし | 同 | missing | 同上 |
| A4 | Type / universal selector `*` (Author 側) | L3325 で UA CSS 用の type selector 部分 mention | UA CSS = M1、Author 側 timeline 不明 | full | `crates/raikiri-style/src/ruletree.rs:265-269` (`Component::LocalName` + `ExplicitUniversalType` accept)、`cascade.rs:189-232` (`match_by_tag`) |
| A5 | Combinator: descendant (space) | mention なし | milestone 未割当 | missing | ruletree.rs:76-78 drop |
| A6 | Combinator: child `>` | mention なし | 同 | missing | 同上 |
| A7 | Combinator: adjacent sibling `+` | mention なし | 同 | missing | 同上 |
| A8 | Combinator: general sibling `~` | mention なし | 同 | missing | 同上 |
| A9 | Selector list `,` (compound) | mention なし | 同 | partial | `SelectorList::parse` は L4 で受理するが type+universal 混合 (`p, div`) のみ live、class 混じり (`.a, .b`) は per-selector 判定で全 list ごと drop (`ruletree.rs:259-275`) |
| A10 | L4 `:is()` / `:where()` / `:not()` | L857, L1870 "採用、優先度低" | milestone 割当なし | missing | `lib.rs:131-135` PseudoClass enum が Hover/Active のみ、`parse_non_ts_pseudo_class` (lib.rs:201-218) は他 pseudo-class を `UnsupportedPseudoClassOrElement` err に落とす |
| A11 | L4 backward `:has()` / `:nth-last-child()` / `:blank` | L858-860, L1871-1873 "優先度低" | 同 | missing | 同上 (lib.rs:201-218) |
| A12 | `:where()` の 0-specificity 挙動 | mention なし | 同 | missing | :where 自体未実装 (A10 参照) |

Non-Goals 干渉: なし (§2 Non-Goals は interactive selector のみ列挙 = `:hover` `:focus` `:link` `:visited` `:target` `:enabled` `:checked`)。**Verification note**: `:hover` / `:active` は lib.rs:131-135 で PseudoClass::Hover/Active として parse 受理されるが、`is_type_or_universal_only` gate で ruletree 段階で drop されるため cascade に届かない — parse 受理 vs cascade 参加が非対称 (Non-Goals との重複、実害 0)。

### Category B: Length units (9 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 1 / partial 0 / stub 0 / regressed 0 / missing 8。`crates/raikiri-style/src/property.rs:84-89` の `Length` enum は `Px(f32)` 単一 variant、`parse_font_size` (property.rs:666-676) が `Token::Dimension { unit: "px", .. }` のみ受理し、非-px 単位・percent・calc / min / max / clamp function は silent drop。

**Top gap**: (1) **B1 em / B2 rem missing** — CSS typography の relative sizing の基礎、`font-size: 1.2em` が silent drop されるため多 tier font-size 階層が effect 0 で意図しない layout が確定。(2) **B6 絶対単位 (pt / mm 等) missing** — 契約書 / 組版で用紙寸法基準の length は必須、`margin: 20mm` が margin 未実装と併せて double-block。

**Since-audit changes**: 無し。property.rs は Sprint 9 で d9y.1/d9y.2 の Arc wrap 化を受けた程度で length 拡張は無く、Category B は 2026-07-19 → 2026-07-20 で drift 無し。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| B1 | `em` | mention なし | milestone 未割当 | missing | `crates/raikiri-style/src/property.rs:666-676` (`parse_font_size` unit=="px" only)、rule.rs:135-147 test `font-size: 1em` → drop |
| B2 | `rem` | mention なし | 同 | missing | 同上 |
| B3 | Percent `%` | mention なし | 同 | missing | 同上 (property.rs:669 Token::Percentage arm 無し) |
| B4 | `vw` / `vh` / `vmin` / `vmax` | mention なし | 同 (paged media で viewport 単位の意味論定義もない) | missing | 同上 |
| B5 | `ex` / `ch` / `ic` / `lh` / `rlh` | mention なし | 同 | missing | 同上 |
| B6 | `in` / `cm` / `mm` / `pt` / `pc` / `Q` | mention なし | 契約書・組版で必須の絶対単位、milestone 未割当 | missing | 同上 |
| B7 | `px` の resolved-value 意味論 | L2902 内部型 f32 mention のみ | 唯一の CSS length と暗黙、明文化なし | full | `property.rs:84-89` `Length::Px(f32)`、`parse_font_size` accept |
| B8 | `calc()` | L316-328 で invariant / L324 で M4 に暗黙 imply | M4 tasks list に該当 task なし (surprising finding 3i) | missing | property.rs:666-676 は Token::Dimension のみ、Token::Function("calc") 分岐 無し |
| B9 | `min()` / `max()` / `clamp()` | mention なし | milestone 未割当 | missing | 同上 (function token 分岐 無し) |

### Category C: Color values (9 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 4 / partial 1 / stub 0 / regressed 0 / missing 4。`crates/raikiri-style/src/property.rs:581-621` の `parse_color` は 3 branch (Hash/IDHash → `parse_hash_color`、Ident → `parse_named_color`、Function `rgb`/`rgba` → `parse_rgb_function`) を持ち、cssparser の 140+ named color set をそのまま享受。C6 `transparent` は cssparser 側で named color `(0,0,0)` を返すが property.rs:595 が `a: 255` を hardcode するため alpha が失われ opaque black になる (partial、実質誤挙動)。C5 currentColor / C7 L4 color function は match arm 無しで silent drop。

**Top gap**: (1) **C5 currentColor missing** — `border-color: currentColor` 等 hyperlink / icon fill での常用構文が silent drop、契約書 hyperlink 表示 (surprising 3f) の派生 block。(2) **C6 transparent silent-broken** — 名前は accept されるが alpha 情報が消える (property.rs:595 hardcode)、cascade prompts で `background: transparent` を書くと `background: black` として resolve される recipe: SEC-adjacent silent semantic corruption。

**Since-audit changes**: 無し。property.rs の parse_color は 2026-07-19 → 2026-07-20 で touch 無し (d9y series は Content/StringSet の Arc wrap のみ)。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| C1 | Hex color `#rgb` / `#rrggbb` / `#rrggbbaa` | L3287 で `color:red` M1 fixture のみ | milestone 未割当 | full | `crates/raikiri-style/src/property.rs:584-591` (`parse_hash_color` via cssparser、alpha も `clamp_unit_f32`) |
| C2 | `rgb()` / `rgba()` | mention なし | 同 | full | `property.rs:597-621` (`parse_rgb_function`、integer channel + optional alpha) |
| C3 | `hsl()` / `hsla()` | mention なし | 同 | missing | `parse_color` (property.rs:581-604) match arm 無し (`Token::Function` gate は `rgb`/`rgba` のみ) |
| C4 | CSS Named colors (140+) | L3287 で red 実装 imply、full set の timeline なし | partial imply (red は M1) | full | `property.rs:593-596` (`parse_named_color` via cssparser、140+ 全 set) |
| C5 | `currentColor` | mention なし | milestone 未割当 | missing | `parse_named_color` は currentcolor を named color として持たない (cssparser API)、`parse_color` の Ident arm で silent None → drop |
| C6 | `transparent` | mention なし | 同 | partial | cssparser の `parse_named_color` は `transparent` を `(0,0,0)` として ok を返すが property.rs:595 が `a: 255` を hardcode するため alpha 情報が失われ opaque black になる (silent semantic drift) |
| C7 | CSS Color L4: `color()` / `oklab()` / `oklch()` / `lab()` / `lch()` | mention なし | 同 | missing | property.rs:597-604 の Function gate に該当 name 無し |
| C8 | CMYK / ICC color | L2741 "fulgur" 責任 | **Non-Goal 明示** | missing | Non-Goal (raikiri scope 外、fulgur 責任) |
| C9 | sRGB profile assumption | L2741 "raikiri は sRGB 前提" | 明記 | full | `CssColor` (property.rs:65-71) は u8 RGBA、sRGB profile 暗黙 |

### Category D: Font properties (12 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 4 / stub 0 / regressed 0 / missing 8。M1.4 の 4 property (`color` / `font-family` / `font-size` / `font-weight`) のうち D1-D4 (font-*) が partial 状態 — parse は入るが expressivity が非常に限定。`property.rs:636-664` (`parse_font_family`) は Vec<Atom> として保持するが system fallback keyword (serif / sans-serif / monospace) の意味論解釈は raikiri-style scope 外、実 font resolution は raikiri-dom / raikiri-paint 側 (parley 統合 M3+)。`parse_font_weight` (property.rs:678-686) は integer 100-900 のみ、`normal` / `bold` keyword を silent drop する。

**Top gap**: (1) **D7 line-height missing** — 組版 layout の baseline 決定に不可欠、行間指定不能で all-16px font-size fixture でも実際に何 px 行送りかが暗黙定数依存。(2) **D9 @font-face missing** — Custom typeface load 経路が完全欠落、契約書 / 招待状で serif/sans-serif fallback しか使えず商業組版品質に到達不能。

**Since-audit changes**: 無し。d9y.1 / q3f は Content/StringSet の Arc wrap declare のみで font 系 property 拡張は 2026-07-19 → 2026-07-20 で touch 無し。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| D1 | `font-family` (system fallback、`serif`/`sans-serif`/`monospace`) | L3345-3346 で UA `<pre>`/`<code>` の monospace は M3 | UA CSS で monospace family は M3、Author 側 timeline は不明 | partial | `property.rs:636-664` (`parse_font_family` は list of Atom を stored、system fallback keyword は Atom として保持されるが実 resolution は M3+ 責務) |
| D2 | Custom `font-family` (specific typeface) | L138 parley 統合 = M3、L3384 `font-fallback` task | M3 implied だが specific typeface 選択の spec なし | partial | 同上 (name は stored、実 font load は下流 raikiri-dom fonts.rs、d9y.4 で bounded read 実装) |
| D3 | `font-size` (keyword/em/rem/%/pt) | mention なし | milestone 未割当 | partial | `property.rs:666-676` (`parse_font_size` は px のみ、keyword / em / rem / % / pt は Category B の length unit gap により silent drop) |
| D4 | `font-weight` | mention なし | 同 | partial | `property.rs:678-686` (`parse_font_weight` integer 100-900 only、`normal` / `bold` keyword は Token::Number 分岐で silent drop) |
| D5 | `font-style` (normal/italic/oblique) | mention なし | 同 | missing | `parse_value` (property.rs:510-572) match に arm 無し |
| D6 | `font-variant` (small-caps 等) | mention なし | 同 | missing | 同上 |
| D7 | `line-height` | mention なし | 組版最重要属性の 1 つ、milestone 未割当 | missing | 同上 |
| D8 | `font` shorthand | mention なし | 同 | missing | 同上 |
| D9 | `@font-face` | L3217 M0 feasibility、L495 `ResourceKind::Font` | fetch 契約は M4、実際の handling milestone なし | missing | `ruletree.rs:213-217` 他 at-rule silent drop、`add_stylesheet` は `@page` のみ retention (rbo scaffolding) |
| D10 | `font-display` (swap/block/fallback) | mention なし | @font-face に付随、未割当 | missing | @font-face 未実装 (D9)、descriptor 段階に至らない |
| D11 | `font-feature-settings` / `font-variant-*` | mention なし | 組版で欲しい OpenType feature 制御、未割当 | missing | `parse_value` match に arm 無し |
| D12 | `font-language-override` | mention なし | 同 | missing | 同上 |

### Category E: Text properties (16 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 1 / partial 0 / stub 0 / regressed 0 / missing 15。E1 (color) のみ Category C 経由で full、E2-E16 の text-* / white-space / vertical-align 系は `parse_value` (property.rs:510-572) match arm に一切存在せず、Author CSS で書いても全 declaration が silent drop する。組版属性としては layout-critical だが M1.4 の 4 property scope 外。

**Top gap**: (1) **E2 text-align missing** — 招待状 / 契約書レイアウトで `text-align: center` `text-align: right` が普遍的、Non-Goals 干渉無しで真に primary goal 直撃。(2) **E3 text-decoration missing** — `<a>` の underline default (surprising 3f) を UA CSS 側で書けない、Author 側の overrides 経路も無い、hyperlink 表示 unsupported の double-block。

**Since-audit changes**: 無し。text 系 property は 2026-07-19 → 2026-07-20 で 0 additions。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| E1 | `color` (詳細) | L3287 fixture のみ | Category C 参照 | full | Category C1-C4 全 arm、`property.rs:581-621` |
| E2 | `text-align` (left/right/center/justify/start/end) | mention なし | 契約書・招待状で必須、未割当 | missing | `parse_value` (property.rs:510-572) match arm 無し |
| E3 | `text-decoration` (underline/line-through) | mention なし | hyperlink 表示で必須、未割当 | missing | 同上 |
| E4 | `text-decoration-line`/`-style`/`-color`/`-thickness` | mention なし | 同 | missing | 同上 |
| E5 | `text-indent` | mention なし | 未割当 | missing | 同上 |
| E6 | `text-transform` (uppercase/lowercase/capitalize) | mention なし | 同 | missing | 同上 |
| E7 | `letter-spacing` | mention なし | 同 | missing | 同上 |
| E8 | `word-spacing` | mention なし | 同 | missing | 同上 |
| E9 | `white-space` (`pre` / `nowrap` / `pre-wrap` / `pre-line`) | L3307, L3345-3346 UA `<pre>`/`<code>` M3 | M3 UA CSS のみ、Author 全 value set 不明 | missing | 同上、`crates/raikiri-html/src/ua/minimal.css` にも `<pre>`/`<code>` 定義無し (M3 defer 状態) |
| E10 | `word-break` / `overflow-wrap` (word-wrap) | mention なし | 未割当 | missing | `parse_value` match arm 無し |
| E11 | `hyphens` | mention なし | 同 | missing | 同上 |
| E12 | `tab-size` | mention なし | 同 | missing | 同上 |
| E13 | `vertical-align` | mention なし | inline layout で必須、未割当 | missing | 同上 |
| E14 | `text-shadow` | mention なし | 同 | missing | 同上 |
| E15 | `direction` / `unicode-bidi` | M3 BiDi task に暗黙包含? | M3 に BiDi task はあるが CSS property の timeline なし | missing | 同上 |
| E16 | `text-orientation` | mention なし | writing-mode 関連 (Category T) | missing | 同上 |

### Category F: Box model (12 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 12。**全 12 item MISSING**。`parse_value` (property.rs:510-572) は box-model 系 property を 1 つも受理せず、`margin` / `padding` / `border` / `width` / `background-color` を含む Author CSS declaration は全て silent drop。M2 で pagestream / multi-page 系 task に集中、M2 Non-goal の "必要になったら足す" (surprising 3b) 位置に依然 pin 状態。

**Top gap**: (1) **F2 margin / F3 padding missing** — layout の骨格を成す 2 property が M1.4 → M2 → 現在まで gap のまま、taffy 統合 (M2) が margin なしで意味を成さない (auto width の "margin: 0 auto" が動かず組版 layout 不成立)。(2) **F1 background-color missing** — 招待状 / 証明書の色付き block や、警告 boxed 内容が全て素朴 flow に降格する (paint 側の背景 fill 経路が無い)。

**Since-audit changes**: 無し。box-model 系 property は 2026-07-19 → 2026-07-20 で 0 additions。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| F1 | `background-color` | mention なし | User observation で言及、milestone 未割当 | missing | `parse_value` (property.rs:510-572) match arm 無し |
| F2 | `margin` (short / long-hand) | L3306, L3344 "M2 で必要になったら足す" | **M2 tasks に対応 task なし** (surprising 3b) | missing | 同上、rule.rs:134-149 test `margin: 10px` → drop |
| F3 | `padding` | 同上 | 同 | missing | 同上 |
| F4 | `width` / `height` (Author) | mention なし | milestone 未割当 | missing | 同上 |
| F5 | `min-width` / `max-width` / `min-height` / `max-height` | mention なし | 同 | missing | 同上 |
| F6 | `border` (all sub-properties) | mention なし | 契約書・証明書で必須、未割当 | missing | 同上 |
| F7 | `border-radius` | mention なし | 同 (証明書 / モダン layout で頻出) | missing | 同上 |
| F8 | `box-shadow` | mention なし | 同 | missing | 同上 |
| F9 | `outline` | mention なし | 同 | missing | 同上 |
| F10 | `overflow` (visible/hidden/scroll/auto/clip) | L2666 `OverflowClipping` は container overflow fallback、CSS property と無関係 | Author `overflow` property の timeline なし | missing | `parse_value` match arm 無し (design doc L2666 の `OverflowClipping` は layout 側 utility、CSS property 経路 未接続) |
| F11 | `box-sizing` | mention なし | 組版で border/padding 込みの width 計算に不可欠、未割当 | missing | `parse_value` match arm 無し |
| F12 | `aspect-ratio` | mention なし | 未割当 | missing | 同上 |

### Category G: Display / formatting context (10 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 2 / partial 0 / stub 0 / regressed 0 / missing 8。`property.rs:305-309` の `DisplayValue` enum は `Block` / `Inline` の 2 variant のみ、`parse_display` (property.rs:693-700) は他 keyword (`inline-block` / `none` / `flex` / `grid` / `table*` / `list-item` / `contents`) を silent drop。`crates/raikiri-html/src/ua/minimal.css` は `html` / `body` / `div` / `p` / `h1-h6` の 10 element に `display: block` を宣言、`inline` は spec default (`ComputedValues::initial` L135 で `DisplayValue::Inline`)。

**Top gap**: (1) **G4 display: none missing** — UA CSS で `<head>` / `<script>` / `<style>` に `display: none` を宣言する慣行が使えない (raikiri-paint は d9y.5/s8w で per-element skip predicate を持つが cascade-independent、Author `display: none` override 経路 無し)。(2) **G8 display: list-item missing** — `<ul>`/`<ol>`/`<li>` (Q2、surprising 3g) の marker 生成の basis で、目次付き技術書 primary goal reference が事実上 unsupported。

**Since-audit changes**: 無し。DisplayValue enum は 2026-07-19 → 2026-07-20 で touch 無し。d9y.5 (raikiri-paint inert HTML skip) は cascade-independent defense-in-depth で display property とは orthogonal。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| G1 | `display: block` | L3300, L3325 M1 UA CSS bundle 対象 | **M1 UA として明示** | full | `property.rs:307` DisplayValue::Block、`crates/raikiri-html/src/ua/minimal.css:19-34` (10 element declared) |
| G2 | `display: inline` | mention なし (default 暗黙) | 他要素の inline default timeline なし | full | `property.rs:308` DisplayValue::Inline、`computed.rs:135` initial value |
| G3 | `display: inline-block` | mention なし | milestone 未割当 | missing | `parse_display` (property.rs:693-700) match arm 無し (silent drop) |
| G4 | `display: none` | mention なし | 同、visibility handling も未明示 | missing | 同上 (d9y.5/s8w の raikiri-paint skip predicate は cascade-independent defense、`display: none` 経路 未接続) |
| G5 | `display: flex` | L64 M7 acceptance "multi-page flex/grid"、L137 taffy 統合、L3214 M0 feasibility | **単一ページ内 flex の timeline 不明** (M3 inline? M4? M7?) | missing | `parse_display` match arm 無し |
| G6 | `display: grid` | 同上 | 同 | missing | 同上 |
| G7 | `display: table` / `table-row` / `table-cell` 等 | L3347 "M6+" | M6a-M6e / M7 / M8 のどれかは未指定 | missing | 同上 |
| G8 | `display: list-item` | mention なし | milestone 未割当 | missing | 同上 |
| G9 | `display: contents` | mention なし | 同 | missing | 同上 |
| G10 | `visibility` (visible/hidden/collapse) | mention なし | 同 | missing | `parse_value` (property.rs:510-572) match に `visibility` arm 無し |

### Category H: Position properties (8 items、Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| H1 | `position: static` (initial value) | mention なし | default 暗黙 |
| H2 | `position: relative` | mention なし | milestone 未割当 |
| H3 | `position: absolute` | mention なし | 同 |
| H4 | `position: fixed` | mention なし | 同 (paged media では PDF に不向き) |
| H5 | `position: sticky` | mention なし | 同 |
| H6 | `position: running(name)` | L849, L1895, L2001, L2003, L3448 | **GCPM 系として M5 明示** |
| H7 | `top` / `right` / `bottom` / `left` | mention なし | milestone 未割当 |
| H8 | `z-index` | mention なし | 同 |

### Category I: Background / gradient (10 items、Medium-High conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| I1 | `background-image` | L494 `ResourceKind::Image` context のみ | 実装 milestone なし |
| I2 | `background-repeat` | mention なし | milestone 未割当 |
| I3 | `background-size` | mention なし | 同 |
| I4 | `background-position` | mention なし | 同 |
| I5 | `background-attachment` | mention なし | 同 |
| I6 | `background-clip` / `background-origin` | mention なし | 同 |
| I7 | `background` shorthand | mention なし | 同 |
| I8 | `linear-gradient()` | mention なし | 同 |
| I9 | `radial-gradient()` / `conic-gradient()` / `repeating-*-gradient()` | mention なし | 同 |
| I10 | `url()` value 一般 | L494 の context 定義のみ | 同 |

### Category J: CSS custom properties / variables (6 items、Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| J1 | Custom property declaration `--foo: value` | mention なし | milestone 未割当 |
| J2 | `var(...)` reference | mention なし | 同 |
| J3 | `@property` (CSS Registered Custom Properties) | mention なし | 同 |
| J4 | `env()` (env variables) | mention なし | 同 |
| J5 | `calc()` (Category B B8) | L316-328, L3217 | B8 参照 |
| J6 | `min()` / `max()` / `clamp()` (Category B B9) | mention なし | B9 参照 |

### Category K: @-rules (paged media 以外、10 items、High for K1/K3/K4)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 10。**全 10 item MISSING**。`ruletree.rs:213-217` の `StyleRuleParser::parse_prelude` は `@page` 以外の at-rule を `Err(input.new_custom_error(()))` で cssparser に silent drop させる。`RuleTree` struct (`ruletree.rs:34-41`) は現状 `style_rules` + `page_rules` の 2 field のみ、design doc L1841-1846 が imply する `font_face_rules` / `counter_style_rules` / `media_rules` / `supports_rules` / `import_rules` の 5 field は code 側で "future field" comment (ruletree.rs:31-33) のまま未実装 — **design doc drift**: audit の "3c" (surprising finding) が指す構造は現時点 code に存在せず、audit が code state を過大評価していた形。

**Top gap**: (1) **K1 @media print missing** — print-first engine の primary invariant `@media print { ... }` が silent drop する現状は surprising 3d の cardinal case、組版 fixture 側が print-only rule を書いても cascade に届かない。(2) **K3 @import missing** — external stylesheet 経路が完全欠落、fixture 群を書く時点で 1 stylesheet-per-document に強制され modularity 不成立。

**Since-audit changes**: 無し。at-rule handling は 2026-07-19 → 2026-07-20 で touch 無し。`rbo` scaffolding (@page 保持) は audit 前 Sprint。**Design doc-vs-code drift**: audit doc の "struct field 定義のみ" は 2026-07-19 時点で既に code 側の future field comment に基づく optimistic reading だった (実 field 無し) — verification note: code は結果として "field 定義すら無い" が、 next planner の judgment 呼ぶための reference には audit doc の記述 (design doc 参照) を優先。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| K1 | `@media` queries (`screen` / `print` / `all` 判別) | L1843 `media_rules: Vec<MediaRule>` struct field | **struct field 定義のみで evaluate milestone なし、print-first で critical** (surprising 3d) | missing | `ruletree.rs:213-217` silent drop、`RuleTree` struct に `media_rules` field 未実装 (`ruletree.rs:31-41` "future field" comment のみ)、test `ruletree.rs:759-770` で `@media print` の p rule が drop されることが pin されている |
| K2 | `@media` feature evaluation (min-width, orientation, etc.) | mention なし | 同 | missing | 同上 |
| K3 | `@import` | L1845 field、L492 resource, L486 depth cap、L3218 M0 feasibility | struct field + resource 契約はあるが actual fetch → parse → merge milestone なし | missing | `ruletree.rs:213-217` silent drop、`RuleTree` struct に `import_rules` field 未実装 |
| K4 | `@font-face` | Category D D9 | 同 | missing | Category D D9 参照、同 silent drop |
| K5 | `@keyframes` (parse-and-ignore) | mention なし。animation は L104 Non-Goal だが keyframes parse skip の明示なし | Non-Goal umbrella と推測、明示なし | missing | `ruletree.rs:213-217` silent drop (Non-Goal umbrella と一致挙動、ただし明示 declaration 無し) |
| K6 | `@supports` | L1844 struct field | struct field 定義のみ、evaluate milestone なし | missing | `ruletree.rs:213-217` silent drop、`RuleTree` struct に `supports_rules` field 未実装 |
| K7 | `@counter-style` | L1842 struct field、L3217 M0 feasibility | field + feasibility はあるが GCPM から参照する milestone なし。**M5 GCPM tasks にも counter-style task なし** | missing | `ruletree.rs:213-217` silent drop、`RuleTree` struct に `counter_style_rules` field 未実装。M5 で `counter-*` property (property.rs:401-415) は landed だが `@counter-style` at-rule 側は defer |
| K8 | `@charset` | mention なし | milestone 未割当 | missing | 同上 (silent drop) |
| K9 | `@namespace` | mention なし | 同 | missing | 同上 |
| K10 | `@layer` | L2330 に mention 1 回のみ | milestone 未割当 | missing | 同上 |

### Category L: Pseudo-elements (6 items、High for L1/L2/L4)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 6。**全 6 item MISSING**。`lib.rs:158-169` の `PseudoElem` enum は **empty variant** (`pub enum PseudoElem {}`) — 何も受理せず、selectors crate の PseudoElement trait requirement を "存在するが受理無し" で満たしている。M5 で `content` / `string-set` / `counter-*` の GCPM static-side は landed (property.rs:401-458) が、pseudo-element selector 経路 (`p::before`) の grammar は未接続で、`content: counter(chapter)` を書いても appropriate host element 無しで cascade 待機状態。

**Top gap**: (1) **L1 ::before / ::after missing** — surprising 3e の cardinal case、M5 で `content` property は fully landed だが pseudo-element selector 経由の generated content 経路が閉じており、GCPM directive がユーザー documented pattern (`p::before { content: counter(chapter) }`) で使えない (structural contradiction 継続)。(2) **L4 ::marker missing** — `<ol>`/`<ul>` (Q2、U5) の marker 生成の spec-official 経路、目次付き技術書 primary goal reference と直接衝突。

**Since-audit changes**: 無し。PseudoElem は M0 seed 以来 empty enum のまま、d9y series は Content payload の Arc wrap のみで pseudo-element selector 経路は 2026-07-19 → 2026-07-20 で 0 additions。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| L1 | `::before` / `::after` | mention なし。GCPM `content` (L845, L850, L1896, L1926) と密接だが selector 自体の timeline なし | **milestone 未割当、M5 GCPM の contradiction** (surprising 3e) | missing | `crates/raikiri-style/src/lib.rs:158-169` (`PseudoElem` は empty variant enum、receiving arm 無し) |
| L2 | `::first-letter` (drop cap) | mention なし | milestone 未割当。招待状 / 技術書 で使う | missing | 同上 |
| L3 | `::first-line` | mention なし | 同 | missing | 同上 |
| L4 | `::marker` (list marker generated content) | mention なし | milestone 未割当。`<ol>`/`<ul>` 出力に必須 | missing | 同上 |
| L5 | `::selection` | mention なし | Non-Goal interactive umbrella と推測、明示なし | missing | 同上 (Non-Goal interactive umbrella と挙動一致、明示 declaration 無し) |
| L6 | `::placeholder` | mention なし | 同 (form control が Non-Goal L106 傘下) | missing | 同上 |

### Category M: Pseudo-classes (non-paged, non-interactive、10 items、Medium-High)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| M1 | `:root` | mention なし | milestone 未割当 |
| M2 | `:empty` | mention なし | 同 |
| M3 | `:first-child` / `:last-child` / `:only-child` | mention なし | 同 |
| M4 | `:first-of-type` / `:last-of-type` / `:only-of-type` / `:nth-of-type()` | mention なし | 同 |
| M5 | `:nth-child()` | mention なし | 同 |
| M6 | `:not()` (Category A A10) | L857, L1870 | A10 参照 |
| M7 | `:lang()` | mention なし | 組版で必要、milestone 未割当 |
| M8 | `:dir()` | mention なし | RTL 組版で必要、未割当 |
| M9 | `:link` / `:visited` | L854, L1867 fail-closed | **Non-Goal 明示** |
| M10 | `:target` | L854 fail-closed | 同 |

### Category N: CSS wide keywords (5 items、Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| N1 | `inherit` keyword | mention なし | cascade の一部として implicit だが explicit parse handling なし |
| N2 | `initial` keyword | mention なし | 同 |
| N3 | `unset` keyword | mention なし | 同 |
| N4 | `revert` / `revert-layer` keyword | mention なし | 同 |
| N5 | `all: <keyword>` shorthand | mention なし | 同 |

### Category O: Cascade specificity / origin (5 items、High for O1/O3)

#### Verification 2026-07-20

**Status distribution**: full 3 / partial 1 / stub 0 / regressed 0 / missing 1。Category O は本 audit の 12 category 中で最も implementation-ready state — cascade の骨格 (O1 specificity / O2 origin / O3 !important / O5 source order tie-break) は M1.4a scope で全 land 済み。**surprising 3h の "!important mention ゼロ" は audit 時点で design doc への言及がゼロだったが、code 側では既に land 済** — audit の spec source を code state で kick して gap を close する好例。

**Top gap**: (1) **O4 :where() 0-specificity missing** — A10 の :where() selector 実装 未着手により propagate、Selector L4 support の front-half が block。(2) **O2 partial** — cascade_rank (`cascade.rs:111-118`) は UA + Author の 2 段のみ、User origin は M1.4a Non-Goal で Consumer が extra_stylesheets 経由で Author として渡す設計 (L3348-3349) — 3-tier full を要求する場合 gap 残る。

**Since-audit changes**: O3 !important は audit 時点で "code に landed だが design doc 未言及" の状態、audit が code state 検査を経由していれば 3h finding は既に closed 判定可能。code 側 (`crates/raikiri-style/src/cascade.rs:111-118` の cascade_rank + `rule.rs:63-72` の parse_important) は 2026-07-19 → 2026-07-20 で touch 無し (stability confirmed)。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| O1 | Selector specificity 計算 (4-tuple) | L840, L1855 "自前実装" | 実装するとは書いてあるが spec-conformant testing の明示 task なし | full | `crates/raikiri-style/src/cascade.rs:86-99` (`Specificity = u32` alias、selectors crate の 32-bit packed specificity 直接使用)、`match_by_tag` の best-of loop (cascade.rs:216-232) |
| O2 | Cascade origin order (UA < User < Author) | L3328-3332 M1.4a 明示 | M1.4a 明示、User origin は Consumer 側 (L3348-3349) | partial | `cascade.rs:111-118` (`cascade_rank` UA / Author 2 段のみ handle、User origin は M1.4a Non-Goal — Consumer が extra_stylesheets 経由で Author 化する設計) |
| O3 | `!important` flag | mention なし | **完全 mention ゼロ、cascade で important reversal も未言及** (surprising 3h) | full | `crates/raikiri-style/src/rule.rs:20-21` (`Declaration.important: bool`)、`rule.rs:64` `input.try_parse(cssparser::parse_important)`、`cascade.rs:111-118` origin reversal (UA important > Author important > Author normal > UA normal)。**audit 時点の "mention ゼロ" は design doc 側の言及ゼロで、code 側は既に landed** |
| O4 | `:where()` の 0-specificity 挙動 (A12) | mention なし | A12 参照 | missing | A10 :where() 未実装のため propagate、`lib.rs:201-218` PseudoClass に :where arm 無し |
| O5 | Author style order (source order tie-break) | L30 背景言及、L1855 implementation 自前 | 対応は暗黙、明示 task なし | full | `rule.rs:37-38` (`StyleRule.source_order: u32`)、`cascade.rs:99` CascadedDecl tuple の 5 番目 field で tie-break |

### Category P: CSS parse resilience / error recovery (5 items、Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| P1 | Invalid declaration skip | mention なし | cssparser default 挙動と推測、明示なし |
| P2 | CDO (`<!--`) / CDC (`-->`) handling | mention なし | 同 |
| P3 | bad-string / bad-url token 処理 | mention なし | 同 |
| P4 | Unknown at-rule の skip | mention なし | 同 |
| P5 | CSS 1 pass error の `RenderError::Cascade` variant | L3277 M1 task `css-cascade-basic (Cascade error)` | M1 で error taxonomy 一部言及、parse resilience 詳細なし |

### Category Q: HTML elements (UA CSS 対象、15 items、High for Q1/Q2/Q5/Q7-Q9/Q13/Q15)

#### Verification 2026-07-20

**Status distribution**: full 2 / partial 1 / stub 0 / regressed 0 / missing 12。`crates/raikiri-html/src/ua/minimal.css` は依然 M1.4a scope の 10 element (html / body / div / p / h1-h6) に `display: block` を宣言するのみ、article / section / nav / aside / header / footer / main / figure / figcaption / blockquote / hr / ul / ol / li / a / img / span / em / strong / br / hr / pre / code / table / *table-*、input / textarea / label 等の HTML LS §14 rendering に対応する UA CSS は未 bundle。**Q13/Q14 <style> は position-independent** — `walk_style_elements` (ruletree.rs:124-183) が iterative DFS で any position の `<style>` element を text 収集。

**Top gap**: (1) **Q1 additional block elements missing** — HTML LS §14.3 Sections の article / section / nav / aside / header / footer / main / hgroup 8 element、および blockquote / figure / figcaption / hr が silent inline 化する。article-based document (Non-Goals 干渉無) が block flow 想定 fixture を書いても displayed as inline (surprising 3j 再確認)。(2) **Q5 <a> hyperlink UA CSS missing** — 契約書 / 技術書で hyperlink の default underline + color が消え、link 表示 silent broken (surprising 3f)。

**Since-audit changes**: **s8w (2026-07-20 merge)** が raikiri-dom Node に `is_non_rendered_html_element` predicate を拡張し、`datalist` / `noembed` / `noframes` / `rp` を HTML LS §15.3.1 hidden elements 4 arm 追加。これは cascade-independent defense-in-depth (`display: none` 相当挙動を Node level で fixation) で、Q12 の `<noscript>` に近い Non-Goal 挙動を強化する方向。UA CSS bundle (Q1-Q11) 側の element cover は 2026-07-19 → 2026-07-20 で 0 additions、`raikiri-html/src/ua/minimal.css` は 10 element のまま。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| Q1 | `display: block` 要素の完全リスト | L3325 で `html, body, div, p, h1-h6` の 10 要素のみ M1 bundle | **`<article>`, `<section>`, `<aside>`, `<nav>`, `<header>`, `<footer>`, `<main>`, `<figure>`, `<figcaption>`, `<blockquote>`, `<hr>` 等の block 要素 timeline なし** | partial | `crates/raikiri-html/src/ua/minimal.css:19-34` 10 element (html/body/div/p/h1-h6) のみ、HTML LS §14.3 の残 8 block element + blockquote / hr は未 declare |
| Q2 | `<ul>` / `<ol>` / `<li>` list rendering | mention なし | milestone 未割当 (surprising 3g)。**目次付き技術書で必須** | missing | `crates/raikiri-html/src/ua/minimal.css` に `ul` / `ol` / `li` selector 無し |
| Q3 | `<pre>` / `<code>` UA CSS | L3306, L3345-3346 M3 | **M3 に明示 defer** | missing | 同上、M3 defer per M1.4a Non-goals |
| Q4 | `<table>` / `<tr>` / `<td>` / `<th>` / `<thead>` / `<tbody>` / `<caption>` UA CSS | L3347 "M6+" | M6+ のどれかは未指定 | missing | 同上、M6+ defer per M1.4a Non-goals |
| Q5 | `<a>` (hyperlink) UA CSS + href default styling | mention なし | milestone 未割当 (surprising 3f)。**組版で必須** | missing | 同上、`<a>` UA rule 無し。text-decoration / color property 自体も未実装 (E1 / E3) |
| Q6 | `<img>` implicit dimensions + replaced element handling | L391 `ReplacedResolver` + L1495 running template pre-resolve M5 | replaced resolve は M5、attribute-based sizing の CSS 昇格 timeline なし | missing | `crates/raikiri-html/src/ua/minimal.css` に `img` selector 無し、attribute-based sizing 経路 未実装 |
| Q7 | `<span>`/`<em>`/`<strong>`/`<b>`/`<i>`/`<u>`/`<sub>`/`<sup>`/`<small>`/`<mark>` inline UA CSS | mention なし | milestone 未割当 | missing | 同上 (inline element の default display は spec initial `inline` で正しく落ちるが、`font-style: italic` / `font-weight: bold` などの UA styling は未実装 = font-style Missing D5 も blocker) |
| Q8 | `<br>` (line break) | mention なし | 同 | missing | UA CSS bundle に `br` styling 無し (実挙動は raikiri-dom / raikiri-paint 側の line break handling 依存) |
| Q9 | `<hr>` | mention なし | 同 | missing | UA CSS bundle 無し |
| Q10 | `<label>` / `<legend>` / `<fieldset>` | mention なし | 同 | missing | 同上、UA CSS bundle 無し |
| Q11 | `<input>` / `<button>` / `<select>` / `<textarea>` UA CSS (静的表示のみ) | L106, L3306, L3342-3343 "M4+ で必要になった時に追加" | **M4+ で "必要になった時" と書いてあり、具体的 M は不明** | missing | 同上 (M4+ defer 継続) |
| Q12 | `<script>` / `<noscript>` | L108 JavaScript 実行 Non-Goal | **Non-Goal 明示** | missing | Non-Goal。`raikiri-dom` `is_non_rendered_html_element` predicate (d9y.5/s8w で datalist/noembed/noframes/rp 追加拡張、bd raikiri-spike-s8w) で `<script>` / `<noscript>` 系の paint skip は landed |
| Q13 | `<style>` (`<head>` 内) | L1876 M1 scope | M1 明示 | full | `crates/raikiri-style/src/ruletree.rs:124-183` (`walk_style_elements` は iterative DFS、`<style>` element text を Author stylesheet として集約) |
| Q14 | `<style>` (`<body>` 内) | L1877 "後の拡張" | milestone 未割当、"後の拡張" とだけあり | full | 同上 (`walk_and_collect` の DFS は position-independent、`<head>` / `<body>` どこでも `<style>` element を検出) — **audit の "後の拡張" 記述は既に自動で cover 済み** |
| Q15 | `<link rel="stylesheet">` | L493 `ResourceKind::ExternalStylesheet` context | ResourceKind 定義のみ、fetch → parse → cascade 統合 milestone なし | missing | `raikiri-html/src/parse.rs` / `raikiri-html/src/sink.rs` に `<link rel="stylesheet">` の fetch → parse → RuleTree 統合 hook 無し (ResourceKind 定義のみ) |

### Category R: HTML attributes → CSS 昇格 / replaced element geometry (9 items、High for R1/R8)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| R1 | `<img width="100" height="100">` implicit dimension | mention なし | ReplacedResolver は intrinsic size 返す (L403) が HTML attribute width/height overriding の spec なし |
| R2 | `<a href>` hyperlink default color/underline | mention なし | Q5 と重複 |
| R3 | `<img alt>` accessibility hint | L109 accessibility Non-Goal 説明。alt text mention なし | milestone 未割当 |
| R4 | `<input type=...>` type-based rendering | Q11 = M4+ | Q11 参照 |
| R5 | `<meta http-equiv>` / `<meta charset>` | mention なし | milestone 未割当 |
| R6 | `<title>` extraction (PDF metadata 用) | mention なし | Consumer 責任と推測できるが明示なし |
| R7 | `data-*` attribute | mention なし | 同 |
| R8 | `lang` attribute (font selection への影響) | mention なし | 組版で日本語判定に必須、milestone 未割当 |
| R9 | `dir` attribute (RTL) | mention なし | BiDi task M3 に attribute-level 言及なし |

### Category S: CSS resolved value vs computed value distinction (3 items、Low priority)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| S1 | CSS spec §Values の resolved/used/actual 段階 | L845 で `content` を "resolved value (実行時解決)" 一箇所 | 全 property の 4-stage semantics 言及なし |
| S2 | `resolveNoParent` vs `getComputedStyle` 相当の distinction | mention なし | milestone 未割当 |
| S3 | `ComputedValues` の具体的 field list | L840, L844, L860 で type name あり | 型の存在 mention のみ、具体 property 列挙なし (surprising 3m) |

### Category T: Writing mode / logical properties (6 items、Low-Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| T1 | `writing-mode: horizontal-tb` | L111-112 縦書き Non-Goal、L3002 "parse 準拠" | Non-Goal 明示 (縦書き) だが横書きの明示 M なし |
| T2 | `writing-mode: vertical-rl` / `vertical-lr` | 同上 | Non-Goal 明示 |
| T3 | `writing-mode` property の parse (parse-and-ignore) | L3002 "parse 準拠" | 具体的 milestone なし |
| T4 | `direction` (E15) | mention なし | E15 参照 |
| T5 | Logical properties (`margin-inline-start` 等) | mention なし | milestone 未割当 |
| T6 | `ruby-position` / `ruby-align` | L111 ルビ Non-Goal | Non-Goal 明示 |

### Category U: List styling (6 items、High conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 6。**全 6 item MISSING**。`parse_value` (property.rs:510-572) の match arm に `list-style-*` 一切 無し、property 自体 unrecognized で silent drop する。U5 ::marker は Category L L4、U6 @counter-style は K7 で propagate — 両者 blocked。M5 で `counter-reset` / `counter-increment` / `counter-set` (property.rs:401-415) は landed で counter tree の source-of-truth はある一方、`::marker` / `list-style-type` / `@counter-style` の consumer 経路が全 gap。

**Top gap**: (1) **U1 list-style-type missing** — Q2 (`<ul>`/`<ol>`/`<li>`) の UA CSS が無いのと組み合わせで list rendering 完全 unsupported (surprising 3g)、目次付き技術書 primary goal 直撃。(2) **U5 ::marker missing** — L4 経由で pseudo-element selector が閉じており、list marker generated content の spec-official 経路 (`ol li::marker { content: counter(list-item) ") "; }`) が使えない、counter tree の existing infra が list marker から access 不能。

**Since-audit changes**: 無し。list-styling は 2026-07-19 → 2026-07-20 で 0 additions。d9y.2 が counter-* の Arc wrap 化を実施したが list-style property とは orthogonal。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| U1 | `list-style-type` | mention なし | milestone 未割当 | missing | `parse_value` (property.rs:510-572) match arm 無し |
| U2 | `list-style-position` | mention なし | 同 | missing | 同上 |
| U3 | `list-style-image` | mention なし | 同 | missing | 同上 |
| U4 | `list-style` shorthand | mention なし | 同 | missing | 同上 |
| U5 | `marker` generated content (`::marker`) | mention なし | Category L L4 と重複 | missing | Category L L4 参照 (`lib.rs:158-169` PseudoElem empty enum、::marker selector 未接続) |
| U6 | `counter-style` `additive-symbols` / `symbols` | K7 参照 | K7 参照 | missing | Category K K7 参照 (`ruletree.rs:213-217` @counter-style silent drop) |

### Category V: Transform / opacity / filter / masking (6 items、Low-Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| V1 | `transform` 2D (rotate/scale/translate/skew) | mention なし | milestone 未割当 |
| V2 | `transform` 3D | L3004 WPT tracking "低、3D transform は non-goal" | 3D は Non-Goal 近い明示、2D は不明 |
| V3 | `opacity` | mention なし | milestone 未割当 |
| V4 | `filter` | mention なし | 同 |
| V5 | `mask-image` / `clip-path` | mention なし | 同 |
| V6 | `mix-blend-mode` / `isolation` | mention なし | 同 |

### Category W: Paged media 内の @page 内 property (5 items、Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| W1 | `@page { size: A4 }` | L3401 M4 task `page-rule-cascade`、L3433 mixed-size fixture | M4 明示 assigned |
| W2 | `@page :first / :left / :right / :nth-page(n) / :blank / named` | L3401 | M4 明示 |
| W3 | `@page { marks: crop cross }` | L2440 `crop_marks: bool` field only | field 定義のみ、property parse の milestone なし |
| W4 | `@page { bleed: N }` | L2439 `bleed: Option<Rect>` field | 同 |
| W5 | `page-break-before` / `page-break-after` / `page-break-inside` (CSS 2.1 legacy) | mention なし | milestone 未割当、modern spec `break-before` のみ implied |

### Category X: MathML / SVG / other namespace (3 items、Medium conflict)

| # | Missing feature | 該当 line | 状況 |
|---|-----------------|----------|------|
| X1 | SVG parsing / inline `<svg>` | L1495 running template M5 pre-resolve、L496 `ResourceKind::Svg`、L2660 debug output | inline SVG rendering milestone なし。External SVG は resolve context のみ |
| X2 | MathML | L497 `ResourceKind::MathML`、L487 depth cap のみ | External MathML の resolve context 定義のみ、実装 milestone なし |
| X3 | XML namespace handling in HTML parsing | mention なし | html5ever 内蔵と推測、明示なし |

## Surprising findings (17 items、audit 中の highlighted observations)

### 3a. Author style CSS pipeline は M1 "cascade minimal" 以外に explicit milestone がない

L3269 M1 Goals `end-to-end pipeline (parse → cascade minimal → layout single-page → paint → PNG)` の "cascade minimal" が何を意味するか M1.4a UA CSS bundle 以外に定義されていない。M2-M8 の Goals/Tasks (L3351-3618) を追っても Author CSS property (color/font/margin/border) の parse-and-apply 追加 milestone は具体的に無い。paged media 系と GCPM 系のみが具体化。

### 3b. M1.4a Non-goals が "M2 で必要になったら足す" と書いてあるが M2 tasks に対応 task なし

L3344 「margin / padding 系は M2 simple-multi-page で必要になったら足す」に対し M2 tasks (L3360-3365) は `pagestream-state-machine, layoutbuffer-skeleton, break-before-after-forced, streaming-partial-output-tracking, sink-state-transition-tests, accept_page-error-propagation, RenderStatus-integration, reference-fixtures-multi-page` のみ。**margin/padding parse-and-apply の task はどこにもない**。同様に L3345 M3 の `white-space: pre` / `monospace` も M3 tasks (L3384-3386) が `parley-integration, bidi-support, font-fallback, inline-formatting-context, glyph-run-emit, text-multilingual-fixtures` で明示 task なし。

### 3c. RuleTree struct field はあるが evaluate milestone なし

L1841-1846 で `RuleTree` は `font_face_rules`, `counter_style_rules`, `media_rules`, `supports_rules`, `import_rules` を持つ。しかし 5 種 at-rule の実 evaluation (media condition 判定、@import fetch、@font-face load、@counter-style 適用、@supports evaluate) の milestone task は全 M1-M8 で発見できない。**data structure だけ用意して処理 pipeline は空欄**。

### 3d. `@media print` の handling は print-first engine で critical だが completely missing

Design goal L54 "fulgur を print-first pipeline に組み替える"、L23 "browser (screen-first) 前提の blitz と方向性が根本的に異なる" と強調するが、**`@media print` (Author が print-only rule を書く場合) の evaluate milestone がない**。RuleTree に `media_rules: Vec<MediaRule>` (L1843) はあるが、これを "raikiri は print context として evaluate" するロジックは全 M で言及なし。**組版 fixture が `@media print { ... }` に依存していたら silent 無視される可能性**。

### 3e. ::before / ::after selector 実装が無いのに GCPM `content: string()/counter()` は M5 first-class

GCPM `content` property は普通 `::before` / `::after` pseudo-element を経由して generated content を出す (spec: L4 `content` はこれらの pseudo でのみ有効)。しかし design doc は `::before` / `::after` selector を全く言及しない。**M5 で `ContentValueItem` を実装しても、そもそも `p::before { content: counter(chapter) }` の selector が動かないと GCPM 効かない**。この矛盾は Non-Goals にも Open Questions にも書かれていない。

### 3f. `<a>` hyperlink default underline / color が M1-M8 で担保されない

契約書・技術書で `<a href>` は必ず現れるが `<a>` の UA CSS (`color: blue; text-decoration: underline;`) は M1.4a bundle (L3325 = `html, body, div, p, h1-h6` のみ) に入っていない。L3342-3349 M1.4a Non-goals も form 系 / margin/padding / pre/code / table のみ言及、`<a>` は無視。**Reference document `invoice-en` (L2832) は hyperlink 含む可能性大だが link 表示が silent broken リスク**。

### 3g. List rendering (`<ul>`/`<ol>`/`<li>`) の milestone assignment が完全欠落

L3325 M1 UA bundle に含まれず、L3342-3349 M1.4a Non-goals にも `<ul>`/`<ol>`/`<li>` の言及なし、M2-M8 tasks にも list 系 task なし。**「目次付き技術書」(L90) が primary goal reference の 1 つなのに list marker 実装 milestone がない**。`::marker` pseudo-element も同じく欠落 (Category L L4)。

### 3h. `!important` の cascade handling がどこにも触れられていない

L840, L1855 "cascade / specificity / inheritance は自前実装" とあるが **`!important` (CSS Cascading L4 §6.3、origin と一緒に cascade order の major factor) の言及がゼロ**。M1 の `css-cascade-basic` task (L3277) がこれをどこまで含むかも不明。

### 3i. calc() 実装は M4 に暗黙 imply だが M4 tasks に無い

L316-328 で `calc feature invariant` を丁寧に議論し L324 で「M4 sandboxed resolver で CSS calc() 実装時に calc arena を raikiri-dom 側に配置」と一箇所書いてある。**しかし M4 tasks list (L3407-3420) に `css-calc-parse` や `calc-arena-implementation` に相当する task が無い**。roborev で明示化するか、実際は M2 (L319) が正しいのか不明。

### 3j. HTML5 default 挙動 (block/inline の要素分類全体) の spec source が明示されていない

L3336-3339 で "参照 OK: CSS 2.1 App.D、HTML LS §14 Rendering、各 CSS module Sample style sheet" と bundle の spec source は決めているが **M1 bundle が 10 要素で止まっている理由 (`html, body, div, p, h1-h6`) の "残り" をどう埋めるかの計画がない**。HTML LS §14.3 Sections だけでも `article, section, nav, aside, header, footer, main, hgroup` の 8 block 要素があり、これらは silent inline 化される。

### 3k. `@media print` と `@page` は spec 上別次元だが設計で混同されていないか

L1843 に `media_rules`、L1840 に `page_rules` と分離した field はある。しかし `@media print { p { color: black } }` が `@page` の context と別扱いなことの evaluate ロジック / cascade merging の spec は不明。M4 で `@page` cascade を実装するが、`@media print` condition の evaluate is out of scope。**"print-first" と言いつつ `@media print { }` を evaluate しないのは矛盾**。

### 3l. Named color の Set は本 doc で完全に定義されていない

L3287 で M1 hello-world が `color:red` を要求するので少なくとも 1 個の named color parse は M1 に含まれる必要がある。しかし **CSS Color L4 の 140+ named colors 全 set をいつ完備するかは未定**。partial impl で red だけ通す設計なのか、全 set を M1 で入れるのかも不明。

### 3m. `ComputedValues` の具体的 field enumeration が無い

L840, L844, L1860 で `ComputedValues` type 名の言及はあるが **「どの CSS property を computed values に含めるか」の列挙が本 doc 内に無い**。stylo リプレースだけど stylo の huge property table 相当を作るのか、必要 property のみ minimal に定義するのかも不明。Sizing / positioning / typography のどれを include するかの choice が M1 の "cascade minimal" の中身になるはずだが表現なし。

### 3n. User origin CSS の位置付けが inconsistent

L3328 "UA < User < Author" と L4 spec を参照し、しかし L3348-3349 で "User origin (~/.config/raikiri/user.css 等) は Consumer が `extra_stylesheets` に流す方式で吸収 (M1 では専用 origin を作らない)"。**"M1 では作らない" と書いてあるので M2+ で作る可能性を残しているが、それがどの M か / Future Work か Open Question かは記載なし**。

### 3o. Non-Goal L104 の "全 CSS 仕様の網羅" は逆に "何を網羅する予定か" が読み取れない

Non-Goals は排除リスト、Goals は organizational principle。**"どこまで対応するかの CSS property カバレッジ table" が本 doc に存在しない**。fixture-driven (T1 reference documents) と言いつつ "fixture が要求する CSS の union" の明示的 list もない。**"組版品質を primary goal" と言った瞬間に "組版に必要な CSS の union" が定義される責任があるが、それが空欄**。

### 3p. WPT tracking category (L2995-3007) が実装 category を暗黙 imply しているが milestone task と reconcile されていない

`css/css-page/` (高)、`css/css-fragmentation/` (高)、`css/selectors/` (高)、`css/css-text/` (中)、`css/css-values/` (中) が tracking 対象。**`css/css-values/` は length units / calc / var 等の spec cover、`css/css-text/` は text-align / white-space 等の property cover。tracking 表と milestone task の対応関係が無い**。

### 3q. `page:` property の CSS side の milestone が不明

L2454 の page name 遷移表で "次 block に `page: X`" と CSS `page` property を前提としている (Fragmentation L3)。**しかし CSS `page` property の parse-and-store milestone が明示 task にない**。M4 @page rule cascade tasks (L3408) は "@page rule" の cascade で、"要素 selector が `page: X` を宣言する property" とは別。

## Priority summary

**High-priority gap (primary goal 直接衝突)**: ~90 item
- Category A (Selector 拡張、12 item)
- Category B (Length units、9 item)
- Category C (Color values、C1-C7 の 7 item)
- Category D (Font properties、D3-D8, D10-D12 の 9 item)
- Category E (Text properties、E2-E8, E10-E14 の 12 item)
- Category F (Box model、F1, F4-F12 の 10 item)
- Category G (Display / formatting context、G2-G6, G8-G10 の 8 item)
- Category K (@-rules 非-paged、K1-K3, K6-K7, K10 の 6 item)
- Category L (Pseudo-elements、L1-L4 の 4 item)
- Category M (Non-interactive pseudo-classes、M1-M5, M7-M8 の 7 item)
- Category O (Cascade、O1, O3 の 2 item)
- Category Q (HTML elements、Q1, Q2, Q5, Q7-Q9, Q14, Q15 の 8 item)
- Category R (HTML attributes、R1, R8 の 2 item)
- Category U (List styling、U1-U6 の 6 item)

**Medium-priority gap**: ~25 item
- Category H (Position、H2-H5, H7-H8)
- Category I (Background/gradient、10 item)
- Category J (Custom properties、J1-J4)
- Category V (Transform/opacity/filter、V1, V3-V6)
- Category W (Paged media 内 property、W3-W5)
- Category X (SVG/MathML inline、X1-X3)

**Low-priority / internal / spec-source gap**: ~15 item
- Category N (CSS wide keywords、5 item)
- Category P (Parse resilience、5 item)
- Category S (Value stage semantics、3 item)
- Category T (Writing-mode parse-and-ignore、T3, T5)

**Non-Goal 明示 (対比のため列挙)**:
- Interactive selectors (`:hover`/`:focus`/`:link`/`:visited`/`:target`/`:enabled`/`:checked`) — fail-closed
- Animation / transition
- Form control interactive behavior
- JavaScript 実行
- Accessibility tree 構築 (hint のみ)
- 縦書き / ルビ / JIS X 4051
- CMYK / ICC color
- 3D transform (tracking 低)

## 次アクション candidate

本 doc は observation の source of truth (audit の結果)。実 action は本 doc を primary reference として:

- **Option A**: Single umbrella epic (raikiri-spike-<new>) を bd に起票、description に本 doc への reference + High priority category を sub-clause 列挙、children breakdown は retro-facilitator or 次 planner に defer
- **Option B**: category cluster 別に epic を複数起票 (例: "Selector 拡張 epic"、"Font/text properties epic"、"Box model epic"、"HTML elements coverage epic"、"@-rules non-paged epic")、cross-scope で portfolio 全体に散らす
- **Option C**: design doc revision PR を先に書く (M1.5 or M2.5 or M9 milestone を新設 or 各 M に基盤 CSS task を挿入)、bd 起票は revision 後 planner が per-M で行う
- **Option D**: 本 doc を retro-facilitator に primary reference として引き渡す (次 sprint 完了時の retro で spec revision proposal 検討)、planner-side 起票なし
