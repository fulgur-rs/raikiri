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

**Verification pass 2026-07-20** — raikiri-spike-0vv.2

- Verified by: Mitsuru Hayasaka
- Verified at: worktree-0vv2 branched from commit `f171ddb` (main HEAD 2026-07-20、post-0vv.1 merge)
- Coverage: 12 category = H / I / J / M / N / P / R / S / T / V / W / X (~76 items) + §3 17 surprising findings (3a-3q) の re-verify + `docs/superpowers/specs/2026-07-20-milestone-gap-breakdown.md` (Option A / B 併記 + recommend) の draft
- Method: sibling 0vv.1 と同 (grep + code read against `crates/raikiri-{style,html,dom,traits}/` on 2026-07-20 code state、5 状態分類 + evidence)。§3 findings は "code-feature landed → resolved / plan artifact → still-holds" を分類 heuristic として採用 (sibling 0vv.1 の 3h/O3 precedent と一致)

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

#### Verification 2026-07-20

**Status distribution**: full 2 / partial 0 / stub 0 / regressed 0 / missing 6。M5 GCPM (raikiri-spike-m5.4) が `position` property に対して `static` + `running(<custom-ident>)` の 2 variant のみ受理する `PositionValue` enum (`crates/raikiri-style/src/property.rs:325-337`) を land、`parse_position` (property.rs:1168-1191) が `static` / `running(name)` を返し他 keyword (`relative` / `absolute` / `fixed` / `sticky`) は silent None → declaration drop。H6 `running(name)` は M5 first-class として full (property.rs:2333-2393 に 6 verification test)、H1 `static` は spec baseline / initial value として full (initial は `ComputedValues::initial` で running_templates empty、`static` keyword を明示指定した rule も accept)、H2-H5 + H7-H8 は全て silent drop。

**Top gap**: (1) **H7 top/right/bottom/left missing** — inset 系 property 単体が silent drop、`position: relative; top: 10px` を書いても position 側で drop → offset property は arm 無しで double drop。(2) **H2-H5 non-static position missing** — printed media で使用頻度は低いが、`position: relative` の containing block establishing use や `position: absolute` の float 代替は組版でも需要あり、M5 で `running()` の側だけ landed して残 keyword が silent drop する分岐が固定化 (spec upstream ではこれらは同 property の value ですが code path が split している)。

**Since-audit changes**: 無し。property.rs の `PositionValue` は m5.4 landed の M5 scope 定義そのままで 2026-07-19 → 2026-07-20 の drift 無し (d9y series は Content/StringSet の Arc wrap のみで position 側は touch 無し)。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| H1 | `position: static` (initial value) | mention なし | default 暗黙 | full | `crates/raikiri-style/src/property.rs:1168-1176` (`parse_position` `static` ident branch → `PositionValue::Static`)、`computed.rs:150-152` initial `running_templates: Vec::new()` (position initial は `static`) |
| H2 | `position: relative` | mention なし | milestone 未割当 | missing | `PositionValue` enum (property.rs:325-337) には `Static` + `Running(SmolStr)` のみ、`parse_position` (property.rs:1168-1191) は `relative` ident を silent None に落として declaration drop |
| H3 | `position: absolute` | mention なし | 同 | missing | 同上 (property.rs:1168-1191) |
| H4 | `position: fixed` | mention なし | 同 (paged media では PDF に不向き) | missing | 同上 |
| H5 | `position: sticky` | mention なし | 同 | missing | 同上 |
| H6 | `position: running(name)` | L849, L1895, L2001, L2003, L3448 | **GCPM 系として M5 明示** | full | `property.rs:1178-1191` (`parse_position` `running(<custom-ident>)` branch)、`computed.rs:109-123` `running_templates: Vec<RunningTemplate>` seed、`cascade.rs:732-810` wire-through tests |
| H7 | `top` / `right` / `bottom` / `left` | mention なし | milestone 未割当 | missing | `parse_value` (property.rs:510-572) match arm 無し (offset 系 property は 1 つも受理せず) |
| H8 | `z-index` | mention なし | 同 | missing | 同上 (`parse_value` に `z-index` arm 無し) |

### Category I: Background / gradient (10 items、Medium-High conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 10。**全 10 item MISSING**。`parse_value` (property.rs:510-572) は `background-*` および gradient 系 function を 1 つも受理せず、Author CSS で書いても declaration が silent drop する。I10 `url()` は M5 GCPM `content: url(...)` / `target-*(url("#anchor"), ...)` 内部での `parse_target_url` (property.rs:1101-1141) が landed だが一般 property value としての `url()` (background-image / list-style-image 等) は match arm 無しで silent drop、M5 scope 限定の狭 surface として存在。

**Top gap**: (1) **I1 background-image + I8 linear-gradient() missing** — 招待状 / 証明書 / 契約書 の色付き block / decorated header / 背景 pattern 全てが素朴 flow に降格 (paint 側の背景 fill 経路が cascade を経由しない)、primary goal reference (invoice-en 等) が組版品質に到達不能。(2) **I10 url() 一般 missing** — `<img src>` は raikiri-html の attribute として保持されるが CSS `url()` value 一般は M5 GCPM ロード用 (parse_target_url) 以外で解釈経路が無く、`background-image: url(...)` / `content: url(...)` — CSS Values の url token — が一部 M5 scope でしか届かない。

**Since-audit changes**: 無し。background / gradient 系 property は 2026-07-19 → 2026-07-20 で 0 additions、`parse_value` match arm も unchanged。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| I1 | `background-image` | L494 `ResourceKind::Image` context のみ | 実装 milestone なし | missing | `crates/raikiri-style/src/property.rs:510-572` (`parse_value` に `background-image` arm 無し)、silent drop |
| I2 | `background-repeat` | mention なし | milestone 未割当 | missing | 同上 |
| I3 | `background-size` | mention なし | 同 | missing | 同上 |
| I4 | `background-position` | mention なし | 同 | missing | 同上 |
| I5 | `background-attachment` | mention なし | 同 | missing | 同上 |
| I6 | `background-clip` / `background-origin` | mention なし | 同 | missing | 同上 |
| I7 | `background` shorthand | mention なし | 同 | missing | 同上 (shorthand explosion 経路も無し) |
| I8 | `linear-gradient()` | mention なし | 同 | missing | property.rs:510-572 `parse_value` は gradient function を認識しない、`Token::Function` 分岐 (rgb/rgba 側にのみ存在) と分離 |
| I9 | `radial-gradient()` / `conic-gradient()` / `repeating-*-gradient()` | mention なし | 同 | missing | 同上 (gradient function 全般 未受理) |
| I10 | `url()` value 一般 | L494 の context 定義のみ | 同 | missing | M5 scope 限定の `parse_target_url` (property.rs:1101-1141) が `content: target-*` 内部で url token を受けるのみ、一般 CSS property value としての `url(...)` は arm 無し (background-image / list-style-image / cursor 等はいずれも I1-I9 と同 drop) |

### Category J: CSS custom properties / variables (6 items、Medium conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 6。**全 6 item MISSING**。`parse_value` (property.rs:510-572) は `--<ident>` prefix の custom property declaration を dispatch する arm を持たず、`var(...)` / `env()` / `calc()` / `min()` / `max()` / `clamp()` function も 1 つも受理しない。CSS Custom Properties L1 の substitution 機構 (declared-value / cascaded-value / computed-value / used-value の 4-stage transformation) は raikiri-style layer に完全に未接続で、consumer CSS が `--brand-color: red; color: var(--brand-color)` を書いても declaration ごと drop する。

**Top gap**: (1) **J1 --foo declaration + J2 var() missing 併発** — CSS variables はデザインシステム構築の基礎、招待状 / 契約書 / 技術書の全 fixture author が palette / typography token を 1 元管理できず、Q1 (block element UA CSS) / D1-D2 (font-family) の gap と組み合わさって "hardcode 以外の Author style pattern が事実上不可能"。(2) **J5 calc() propagate** — Category B B8 参照。`padding: calc(1em + 2px)` 相当が silent drop、margin/padding 側 (F2/F3) の missing とも重畳する。

**Since-audit changes**: 無し。custom properties / variables / math functions は 2026-07-19 → 2026-07-20 で 0 additions。d9y series (Content/StringSet/counter-* Arc wrap) は既存 property の memory shape 変更のみで custom property 経路とは orthogonal。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| J1 | Custom property declaration `--foo: value` | mention なし | milestone 未割当 | missing | `parse_value` (property.rs:510-572) match arm は listed property のみ、`--`-prefix ident の受理経路 無し |
| J2 | `var(...)` reference | mention なし | 同 | missing | `Token::Function("var")` 分岐 無し、`parse_font_size` (property.rs:666-676) 等 property 側 parser も `Token::Function` 受理は rgb/rgba/running/target-*/counter/counters/string/attr 限定 |
| J3 | `@property` (CSS Registered Custom Properties) | mention なし | 同 | missing | `ruletree.rs:213-217` at-rule silent drop、`RuleTree` struct に `property_rules` field 無し |
| J4 | `env()` (env variables) | mention なし | 同 | missing | `parse_value` に env function 分岐 無し |
| J5 | `calc()` (Category B B8) | L316-328, L3217 | B8 参照 | missing | Category B B8 参照 (`parse_font_size` 等 length parser は `Token::Dimension` のみ、`Token::Function("calc")` 分岐 無し) |
| J6 | `min()` / `max()` / `clamp()` (Category B B9) | mention なし | B9 参照 | missing | Category B B9 参照 (同 property side に function 分岐 無し) |

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

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 10。**全 10 item MISSING**。`crates/raikiri-style/src/lib.rs:131-135` の `PseudoClass` enum は `Hover` / `Active` の 2 variant のみ (Category A の Non-Goals 干渉 note 参照 — parse 受理はされるが cascade 段階では type+universal only gate で drop)、`parse_non_ts_pseudo_class` (lib.rs:201-218) は他 pseudo-class name を全て `SelectorParseErrorKind::UnsupportedPseudoClassOrElement` err に落として selector list 全体を drop する (per-selector 側 A9 参照)。M9/M10 は Non-Goal 明示だが code 側で explicit "reject" declaration は無く一般 unsupported と同挙動。

**Top gap**: (1) **M3 :first-child + M5 :nth-child() missing** — 招待状 / 目次付き技術書 で自然な pattern (`.section > p:first-child { font-weight: bold }`)、章タイトル / 引用 / 表 zebra strip の Author expressivity が壊滅的。(2) **M7 :lang() missing** — 組版で日本語 / 英語混在 layout での font-family 分岐 (`:lang(ja) { font-family: serif-ja }`) が使えず、Category R R8 (lang attribute) と組み合わさって "言語ごと font 切替" が結局 achievable 不能 (D1-D2 partial とも重畳)。

**Since-audit changes**: 無し。PseudoClass enum は M0 seed 以来 Hover/Active 固定、Sprint 10 hardening (d9y / 1ll / q3f / 8yu 等) は selector shape に触れず、Category M は 2026-07-19 → 2026-07-20 で 0 additions。M9/M10 の fail-closed 挙動は "特別 declaration 無し / 一般 unsupported と同" で L4 spec の :link / :visited privacy fail-closed 意図と挙動一致 (code path が明示 exclusion 無しで achieve)。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| M1 | `:root` | mention なし | milestone 未割当 | missing | `crates/raikiri-style/src/lib.rs:201-218` (`parse_non_ts_pseudo_class` は `:hover` / `:active` のみ Ok、他は UnsupportedPseudoClassOrElement err → selector list 全 drop) |
| M2 | `:empty` | mention なし | 同 | missing | 同上 |
| M3 | `:first-child` / `:last-child` / `:only-child` | mention なし | 同 | missing | 同上 (`:first-child` 等の structural pseudo-class parse も selectors crate の `parse_one_simple_selector` 経路で受理されない、下流の match runtime も unimplemented) |
| M4 | `:first-of-type` / `:last-of-type` / `:only-of-type` / `:nth-of-type()` | mention なし | 同 | missing | 同上 |
| M5 | `:nth-child()` | mention なし | 同 | missing | 同上 (functional pseudo-class 分岐 未実装) |
| M6 | `:not()` (Category A A10) | L857, L1870 | A10 参照 | missing | Category A A10 参照 (`:not(...)` は functional pseudo で `parse_non_ts_pseudo_class` を経由せず、selectors crate 側 `Component::Negation` 生成には至らず未実装) |
| M7 | `:lang()` | mention なし | 組版で必要、milestone 未割当 | missing | `parse_non_ts_pseudo_class` (lib.rs:201-218) `:lang(...)` 受理 arm 無し、`RaikiriSelectorImpl` (lib.rs:180-190) には lang attr matching context 無し |
| M8 | `:dir()` | mention なし | RTL 組版で必要、未割当 | missing | 同上 (:dir arm 無し) |
| M9 | `:link` / `:visited` | L854, L1867 fail-closed | **Non-Goal 明示** | missing | `lib.rs:201-218` に :link / :visited 受理 arm 無し、UnsupportedPseudoClassOrElement err (Non-Goal 明示との整合挙動、explicit reject declaration 無し) |
| M10 | `:target` | L854 fail-closed | 同 | missing | 同上 (:target arm 無し、Non-Goal 明示と挙動一致) |

### Category N: CSS wide keywords (5 items、Medium conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 5。**全 5 item MISSING at cascade level**。CSS wide keyword (`inherit` / `initial` / `unset` / `revert` / `revert-layer`) は `<custom-ident>` 系 property parser (property.rs:759-765, 963-, 978-) が defensively 予約語として "reject" するだけで、declaration keyword としての cascade-level 実装 (親から値を forcibly 引き継ぐ / initial 値に強制 reset / cascade layer を revert する 3 種挙動) は皆無。M5 で `counter-*` / `content` / `string-set` / `position` は landed だが、これら property に対して `color: inherit` / `font-size: initial` を declaration として書いても property parser 側で受理する arm が無く silent drop。

**Top gap**: (1) **N1 inherit + N2 initial missing** — cascade の spec compliance level を測る意味で最基礎の 2 keyword、"font-size: inherit" のような明示的継承 reset が declaration 側で書けない (inheritance 自体は `ComputedValues::inherit_from` が親コピーで実現するが、cascade winner が `inherit` keyword の case に対する fast-path が code 側で認識されない)。(2) **N5 all: <keyword> missing** — shorthand 全 property reset の meta 手段が使えない、Author 側の "reset scope" 制御が block。

**Since-audit changes**: 無し。CSS wide keyword handling は 2026-07-19 → 2026-07-20 で 0 additions。property parser 内 defensive rejection (property.rs:759-765 `is_reserved_ident_for_counter_name` 等) は M5 landed 時点の state で unchanged。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| N1 | `inherit` keyword | mention なし | cascade の一部として implicit だが explicit parse handling なし | missing | `parse_value` (property.rs:510-572) 各 property 側 parser は `<inherit>` を declaration keyword として認識せず silent drop、`inherit_from` (computed.rs:168-201) は "cascade winner 無し + non-inherited" のみ hit する fallback、"declaration が明示的に `inherit` を書いた case" の fast-path 無し |
| N2 | `initial` keyword | mention なし | 同 | missing | 同上 (property parser に `initial` keyword arm 無し、`ComputedValues::initial` は cascade miss 時の baseline としてのみ使用) |
| N3 | `unset` keyword | mention なし | 同 | missing | 同上 |
| N4 | `revert` / `revert-layer` keyword | mention なし | 同 | missing | 同上 (`revert-layer` は @layer 未実装 (K10) と併せて double gap) |
| N5 | `all: <keyword>` shorthand | mention なし | 同 | missing | `parse_value` に `all` arm 無し、shorthand explosion 経路も未実装 (F7 border shorthand と同 gap 構造) |

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

#### Verification 2026-07-20

**Status distribution**: full 3 / partial 2 / stub 0 / regressed 0 / missing 0。CSS parse resilience は raikiri-style code + cssparser 依存の合作で **audit の "推測、明示なし" は現状すべて挙動側で cover 済み**。P1 (invalid declaration skip) は `DeclParser::parse_value` (rule.rs:57-73) が `parse_value` の None を `input.new_custom_error(())` に mapping し、`RuleBodyParser::flatten` (rule.rs:47) が Err を skip、pinning test `rule.rs:134-149 drops_invalid_property_and_value` が挙動を fix (margin drop / em drop / red keep)。P4 (unknown at-rule skip) は `StyleRuleParser::parse_prelude` (ruletree.rs:213-217) が `@page` 以外の at-rule に対して `Err(input.new_custom_error(()))` を返す構造で cssparser 側の rule-list iteration が silently skip する挙動を pin。P5 (RenderError::Cascade variant) は `crates/raikiri-style/src/error.rs:16-26` の `CascadeError::Internal { message }` として landed、raikiri-traits 側で `RenderError::Cascade(CascadeError)` に re-export。P2 / P3 は cssparser tokenizer の inherent behavior に依存する部分で raikiri-side test が無く partial 判定 — CSS Syntax L3 の CDO/CDC / bad-string / bad-url token は cssparser が消費するが raikiri-style 側は explicit branch や regression test を持たない。

**Top gap**: (1) **P2 CDO/CDC / P3 bad-token pin test 欠落** — cssparser dependency の inherent 挙動なので実害 0 に近いが、cssparser upgrade 時の regression detect 面で raikiri-side pin test が無いのが gap (`rule.rs:134-179` は property level の resilience のみ)、raikiri-vrt scenario で `<script>` 内 CSS-like コメントや bad-url を fixture に混ぜる regression harness も未整備。P5 の error variant 定義は landed だが、`CascadeError::Internal` variant のみで parse resilience 由来の terminal error (parse hard failure vs recoverable drop の segregation) は明示的 variant を持たない (comment 上は "silently drop され error にならない" が spec 準拠 stance)。

**Since-audit changes**: 無し。rule.rs / ruletree.rs / error.rs の parse resilience path は 2026-07-19 → 2026-07-20 で touch 無し。1ll (parse_string_set trailing-comma strict 化) は property level の内部 resilience 強化で category P の domain には含まれず。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| P1 | Invalid declaration skip | mention なし | cssparser default 挙動と推測、明示なし | full | `crates/raikiri-style/src/rule.rs:46-49` (`parse_declaration_block` の `RuleBodyParser::flatten`)、`rule.rs:57-73` `DeclParser::parse_value` が None → Err に mapping、pinning test `rule.rs:134-149 drops_invalid_property_and_value` (margin / 1em drop、red keep) と `rule.rs:167-172 rejects_trailing_garbage_after_value` |
| P2 | CDO (`<!--`) / CDC (`-->`) handling | mention なし | 同 | partial | cssparser tokenizer inherent (raikiri-style 内 explicit branch 無し、regression pin test 無し)。CSS Syntax L3 §4 tokenizer が top-level `<!--` / `-->` を消費する挙動を暗黙依存 |
| P3 | bad-string / bad-url token 処理 | mention なし | 同 | partial | 同上 (cssparser tokenizer inherent、raikiri-side pin test 無し)。CSS Syntax L3 §4.3.14 bad-string-token / §4.3.15 bad-url-token の rule termination 挙動を暗黙依存 |
| P4 | Unknown at-rule の skip | mention なし | 同 | full | `crates/raikiri-style/src/ruletree.rs:213-217` `StyleRuleParser::parse_prelude` は `@page` 以外の at-rule name を `Err(input.new_custom_error(()))` に落とし cssparser 側 iterator が silent skip、test `ruletree.rs:759-770` が `@media print { p { ... } }` が drop されることを pin |
| P5 | CSS 1 pass error の `RenderError::Cascade` variant | L3277 M1 task `css-cascade-basic (Cascade error)` | M1 で error taxonomy 一部言及、parse resilience 詳細なし | full | `crates/raikiri-style/src/error.rs:16-26` `CascadeError::Internal { message }` variant landed、`raikiri-traits` 側で `RenderError::Cascade(CascadeError)` 経由 re-export (comment at error.rs:1-6)。**CSS spec 準拠で invalid rule / value は silently drop され error にならない stance** (comment at error.rs:8-11)、Internal variant は raikiri-style 内 fail-hard を明示選択した場合のみ populate |

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

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 1 / stub 0 / regressed 0 / missing 8。html5ever が generic attribute list を parse し raikiri-html sink (`crates/raikiri-html/src/sink.rs:435-442`、null-namespace attr → raikiri-dom `set_element_attributes`) が保持するので DOM level では `data-*` を含む attribute 全般が preserve される (`raikiri-html/src/lib.rs:764-775` data-x preservation test)、しかし **CSS 昇格 / presentational hint mapping は 1 つも実装されておらず**、`<img width="100">` / `<a href>` / `lang="ja"` 等の HTML attribute が cascade 段階に何も寄与しない。R6 `<title>` は DOM に text node として存在するが Consumer 向け抽出 API 無し、R7 `data-*` は attribute として保持されるが CSS attribute selector (Category A A3 missing) 未実装で access 経路無し。

**Top gap**: (1) **R1 <img width/height> missing** — `ReplacedResolver::resolve` は intrinsic size を返す (raikiri-traits/src/resolver.rs:17-)、taffy_impl.rs:145-160 の compute_leaf_layout は computed style の known width/height を優先し fallback で text/parley intrinsic を採用、しかし **HTML `<img width="100" height="100">` attribute を computed style に昇格するコードが皆無** — HTML LS §14.4.4 "Attributes for embedded content and images" が指示する presentational hint mapping が effect 0、fixture 側 `<img width>` は silent ignore。(2) **R8 lang attribute missing** — 組版で `<html lang="ja">` / `<span lang="en">` から font selection に影響を与える経路が完全欠落 (`:lang()` pseudo (M7) と R8 attribute の double gap)、混合 script fixture が M3 parley 統合以降でも lang 判定 hint を得られない。

**Since-audit changes**: **s8w merge** (2026-07-20 merge、bd raikiri-spike-s8w) が raikiri-dom Node に `is_non_rendered_html_element` predicate を拡張 (datalist / noembed / noframes / rp 追加) するが、これは cascade-independent defense-in-depth で HTML attribute → CSS mapping とは orthogonal domain。R1-R9 の attribute promotion 経路は 2026-07-19 → 2026-07-20 で 0 additions、`raikiri-html/src/ua/minimal.css` (10 element) 側も unchanged。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| R1 | `<img width="100" height="100">` implicit dimension | mention なし | ReplacedResolver は intrinsic size 返す (L403) が HTML attribute width/height overriding の spec なし | missing | `crates/raikiri-dom/src/taffy_impl.rs:145-160` compute_leaf_layout は `known.width` (computed style 由来) を優先、`<img width>` attribute → computed style 昇格経路 無し (`raikiri-dom/src/document.rs` に presentational hint mapping API 無し) |
| R2 | `<a href>` hyperlink default color/underline | mention なし | Q5 と重複 | missing | Category Q Q5 参照 (`raikiri-html/src/ua/minimal.css` に `a` selector 無し、text-decoration property 自体も未実装 E3) |
| R3 | `<img alt>` accessibility hint | L109 accessibility Non-Goal 説明。alt text mention なし | milestone 未割当 | missing | attribute 自体は `set_element_attributes` で保持されるが accessibility tree / alt text 露出 API 無し (Non-Goal L109 umbrella と挙動一致) |
| R4 | `<input type=...>` type-based rendering | Q11 = M4+ | Q11 参照 | missing | Category Q Q11 参照 (form 系 UA CSS M4+ defer)、type-based rendering 経路 未実装 |
| R5 | `<meta http-equiv>` / `<meta charset>` | mention なし | milestone 未割当 | missing | `crates/raikiri-html/src/parse.rs:19` は `read_to_string` で UTF-8 前提 (encoding_rs 導入は M2+ 予定 comment)、`<meta charset>` から encoding 変換する経路 無し |
| R6 | `<title>` extraction (PDF metadata 用) | mention なし | Consumer 責任と推測できるが明示なし | missing | `raikiri-html` / `raikiri-dom` に `title` element 抽出 API 無し (Consumer が DOM walk するしかない、fulgur 側でも未整備) |
| R7 | `data-*` attribute | mention なし | 同 | partial | attribute 自体は保持される (`crates/raikiri-html/src/lib.rs:764-775` test `data-x=42` preservation)、しかし CSS attribute selector 未実装 (Category A A3) なので Author CSS からの access 経路無し = "DOM に存在するが CSS ノー touch" の partial |
| R8 | `lang` attribute (font selection への影響) | mention なし | 組版で日本語判定に必須、milestone 未割当 | missing | attribute 自体は保持されるが cascade / font selection / :lang() pseudo (M7) いずれも lang attr 参照経路 無し |
| R9 | `dir` attribute (RTL) | mention なし | BiDi task M3 に attribute-level 言及なし | missing | 同上 (dir attribute → CSS direction property 昇格経路 未実装、BiDi M3 task list も attribute-level 言及無し) |

### Category S: CSS resolved value vs computed value distinction (3 items、Low priority)

#### Verification 2026-07-20

**Status distribution**: full 1 / partial 1 / stub 0 / regressed 0 / missing 1。S3 は audit 時点で "型名 mention のみ、具体列挙なし" と judge されていたが、**現 code state では `crates/raikiri-style/src/computed.rs:41-124` の `ComputedValues` struct が 11 concrete field を named + 各 field に doc comment (inherited / non-inherited + initial value + spec section 引用) 付き で landed** — audit 時点の doc-side gap は現在 code 側で明示的に answer 済み (audit の "3m surprising finding" が指す "minimal 定義かどうかの choice" は minimal + incremental が chosen、11 fields = 4 typography + 1 display + 3 counter + content + string-set + running-templates)。S1 は依然 audit note のとおり "content" 側のみ resolved 中間表現として明示され、length / color / display 等の resolved vs used vs actual segregation は code に反映無し (`Length::Px(f32)` は "resolved" と "used" が同一 shape、`ComputedValues::font_size: Length` は resolved value stage で完了)。S2 は該当 API (`getComputedStyle` 相当 / resolveNoParent) が code 側で design 段階以前で unimplemented、Consumer は `resolve_document_styles` (`crates/raikiri-style/src/cascade.rs`) の全 node computed vec を直接 index するのみ。

**Top gap**: (1) **S1 partial** — CSS spec §Values の 4-stage (specified → computed → used → actual) 中、raikiri は現状 "specified (parse)" → "computed (cascade + inheritance walk)" の 2 段のみ実装、"used" (layout resolution 後の px 値) は taffy が抱え、"actual" (device pixel snap 後) は raikiri-paint の raster 段で implied だが型 boundary 無し。 Stage segregation は code shape 上不明瞭で、Consumer が "computed vs used" を区別する必要がある高度な case (SVG stroke-dasharray / paged media で page-relative unit) で silent conflation の risk (実際 3m と絡む、audit の "resolved value 一箇所のみ" の指摘は code 側で継続)。(2) **S2 missing** — getComputedStyle 相当は Non-Goal ですらないが、Consumer 側 needs (fulgur の debug view) の有無を M0-M8 で査定した記録が code / doc 双方に無い。

**Since-audit changes**: 無し。ComputedValues struct 自体は M5 で counter-* / content / string-set / running-templates 4 field 追加された歴史 (raikiri-spike-s85 / m5.1 / m5.3 / m5.4) が Sprint 9 以前で完成、Sprint 10 hardening (d9y) は Arc wrap の内部変更のみで field surface は unchanged。2026-07-19 → 2026-07-20 の drift 0。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| S1 | CSS spec §Values の resolved/used/actual 段階 | L845 で `content` を "resolved value (実行時解決)" 一箇所 | 全 property の 4-stage semantics 言及なし | partial | `crates/raikiri-style/src/property.rs:339-341` doc comment "M1.4 でサポートする property の resolved value" (specified → computed の 2 段のみ実装)、used stage は taffy 側 (`crates/raikiri-dom/src/taffy_impl.rs`)、actual stage (device px snap) は raikiri-paint 側で型 boundary 無し |
| S2 | `resolveNoParent` vs `getComputedStyle` 相当の distinction | mention なし | milestone 未割当 | missing | `crates/raikiri-style/src/cascade.rs` に getComputedStyle 相当の per-node lookup API 無し、Consumer は `resolve_document_styles` の返り値 vec を直接 index するしかない |
| S3 | `ComputedValues` の具体的 field list | L840, L844, L860 で type name あり | 型の存在 mention のみ、具体 property 列挙なし (surprising 3m) | full | `crates/raikiri-style/src/computed.rs:41-124` `ComputedValues` に 11 concrete field (color / font_family / font_size / font_weight / display / counter_reset / counter_increment / counter_set / content / string_set / running_templates) が named + doc comment (inherited / non-inherited + initial value + spec section 引用) 付きで landed、`ComputedValues::initial` (computed.rs:129-153) + `ComputedValues::inherit_from` (168-201) で initial / inherit の explicit table |

### Category T: Writing mode / logical properties (6 items、Low-Medium conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 6。**全 6 item MISSING**。`parse_value` (property.rs:510-572) は `writing-mode` / `direction` / `margin-inline-*` / `padding-inline-*` / `ruby-*` を 1 つも受理せず、Author CSS で書いても declaration が silent drop。T2 (vertical writing) と T6 (ruby) は L111-112 で Non-Goal 明示 = raikiri scope 外、T1 (horizontal-tb) は spec default で implicit だが property parse 経路無しで明示宣言も drop する構造、T3 (parse-and-ignore) は "parse 準拠" と design doc に書かれているが code 側で parse 経路が無いため design intent と code state の drift。T5 logical properties は F2/F3 の physical margin/padding が missing の状態で "logical だけ実装" は spec上 non-sensical、propagate-drop。

**Top gap**: (1) **T1 + T3 writing-mode 全般 missing** — Non-Goal 明示は vertical (T2/T6) のみ、horizontal-tb の spec default declaration や `writing-mode` property の parse-and-ignore は "対応する" と design doc L3002 に書かれているが code 側で `parse_value` match 未接続、Non-Goal でない部分の gap が silent。(2) **T5 logical properties missing** — physical margin/padding (F2/F3) が全滅の状態で logical だけ landing する優先度は低いが、Category R R8 lang attribute (`<html lang="ar">` RTL 判定 use case) と組み合わせで RTL 組版 の "layout inversion" scenario 全体が block。

**Since-audit changes**: 無し。writing-mode / direction / logical properties / ruby は 2026-07-19 → 2026-07-20 で 0 additions。M3 BiDi task list (L3384-3386) は parley-integration / bidi-support / font-fallback で property-level plumbing に触れず、attribute-side (R9 dir) と property-side (T4 direction) の両方が gap。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| T1 | `writing-mode: horizontal-tb` | L111-112 縦書き Non-Goal、L3002 "parse 準拠" | Non-Goal 明示 (縦書き) だが横書きの明示 M なし | missing | `parse_value` (property.rs:510-572) `writing-mode` arm 無し、horizontal-tb は spec default で implicit だが明示 declaration も silent drop |
| T2 | `writing-mode: vertical-rl` / `vertical-lr` | 同上 | Non-Goal 明示 | missing | 同上 (Non-Goal L111-112 と挙動一致、explicit reject declaration 無し) |
| T3 | `writing-mode` property の parse (parse-and-ignore) | L3002 "parse 準拠" | 具体的 milestone なし | missing | 同上 (`parse_value` に writing-mode arm 無し、design doc L3002 "parse 準拠" 記述と code state に drift) |
| T4 | `direction` (E15) | mention なし | E15 参照 | missing | Category E E15 参照 (`parse_value` に direction arm 無し、BiDi task M3 は parley integration が中心で CSS property side に touch 無し) |
| T5 | Logical properties (`margin-inline-start` 等) | mention なし | milestone 未割当 | missing | `parse_value` に margin-inline-* / padding-inline-* / inset-inline-* arm 無し (F2/F3 physical 側も未実装で double gap) |
| T6 | `ruby-position` / `ruby-align` | L111 ルビ Non-Goal | Non-Goal 明示 | missing | `parse_value` に ruby-* arm 無し (Non-Goal L111 と挙動一致、explicit reject 無し) |

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

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 0 / stub 0 / regressed 0 / missing 6。**全 6 item MISSING**。`parse_value` (property.rs:510-572) は transform / opacity / filter / mask / clip-path / mix-blend-mode / isolation を 1 つも受理せず、Author CSS で書いても declaration が silent drop。V2 (3D transform) は L3004 WPT tracking "低" と near-Non-Goal 明示、V1 (2D transform) は明示的 Non-Goal ではないが code 側で受理経路無し。V3 opacity は組版で図表 watermark / 半透明 header 用に use case があるが Vec に単純 f32 field を足すだけの low-effort task が M0-M8 に無い形。

**Top gap**: (1) **V3 opacity missing** — 6 item 中最も primary goal との衝突が高い (証明書 watermark / 招待状 pattern を透過表示する需要)、implementation cost 極小 (f32 field + `parse_number` を parse_value に接続) にも関わらず milestone 未割当 で M8 スケジュール裏書きが無い、"low-hanging fruit" 状態。(2) **V1 2D transform + V4 filter missing** — 図表・graphics 系の compositional 効果が使えず、モダン layout / graphical decoration の Author expressivity が壊滅、fixture-driven 開発でも fixture 側 template を書く際 constraint。

**Since-audit changes**: 無し。transform / opacity / filter / mask 系 property は 2026-07-19 → 2026-07-20 で 0 additions。raikiri-paint の rendering path (`crates/raikiri-paint/src/*.rs`) も transform matrix / alpha channel の適用経路を持たず (tiny-skia の compose API は使用しているが CSS 由来の transform / opacity を computed style から拾う経路無し)。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| V1 | `transform` 2D (rotate/scale/translate/skew) | mention なし | milestone 未割当 | missing | `parse_value` (property.rs:510-572) match arm 無し、`Token::Function` 分岐は rgb/rgba/running/target-*/counter/counters/string/attr のみで `rotate` / `scale` / `translate` / `skew` / `matrix` function 未受理 |
| V2 | `transform` 3D | L3004 WPT tracking "低、3D transform は non-goal" | 3D は Non-Goal 近い明示、2D は不明 | missing | 同上 (`translate3d` / `rotate3d` / `matrix3d` function 未受理、Non-Goal に近い挙動一致) |
| V3 | `opacity` | mention なし | milestone 未割当 | missing | `parse_value` に opacity arm 無し、`ComputedValues` (computed.rs:41-124) に alpha / opacity field 無し |
| V4 | `filter` | mention なし | 同 | missing | 同上 (filter function `blur(...)` / `brightness(...)` 等 未受理) |
| V5 | `mask-image` / `clip-path` | mention なし | 同 | missing | 同上 (mask-image は Category I I1 background-image と同 gap、clip-path function 未受理) |
| V6 | `mix-blend-mode` / `isolation` | mention なし | 同 | missing | 同上 (blend-mode keyword 受理経路無し、raikiri-paint compose 側にも CSS-driven blend mode 適用経路無し) |

### Category W: Paged media 内の @page 内 property (5 items、Medium conflict)

#### Verification 2026-07-20

**Status distribution**: full 0 / partial 1 / stub 1 / regressed 0 / missing 3。`@page` rule 自体は raikiri-spike-jzv / rbo scaffolding で `PageRule` struct (page.rs:157-175) + `PageSelector` / `PageSelectorEntry` / `PagePseudo` (page.rs:88-141) が landed、`RuleTree::add_stylesheet` (ruletree.rs) が `@page` at-rule のみ retention。**しかし @page block 内の descriptor (`size` / `marks` / `bleed`) は M1.4 property parser を再利用しており、property.rs:341 の "認識できない property" umbrella で silent drop する** (page.rs:395 doc comment "e.g. `size`, `margin`, `marks` — M1.4 property parser silently drops these; see `ruletree::tests::page_body_unsupported_property_drops_declaration`")。W2 は `:first` / `:left` / `:right` / `:blank` + named ident の 5 種 selector を PagePseudo enum (page.rs:132-141) が受理、but `:nth-page(n)` は L59-63 の scaffolding decision で intentionally excluded (comment: "no primary source defines … forcing invented behaviour")、5/6 = partial。

**Top gap**: (1) **W1 stub** — @page rule 全体は cascade 前段 (parse + selector matching) まで完成、しかし descriptor 個別 (`size` / `margin` / `marks` / `bleed`) は property parser 再利用の silent drop で挙動 0、M4 task `page-rule-cascade` (L3408) が assign されているが個別 descriptor parser の M4 sub-task 化は未細分化。(2) **W5 page-break-* + break-* missing** — CSS 2.1 legacy と modern `break-before` / `break-after` / `break-inside` の両方が `parse_value` に arm 無し、M2 task `break-before-after-forced` (L3362) が assign されているが property-side parser の landing 単位 task が明示的に無い、M2 scope 実装は "@page break-before" specific で property side は defer。

**Since-audit changes**: 無し。@page rule / PageRule struct / PagePseudo enum / page selector parsing は raikiri-spike-jzv (rbo scaffolding) 以降 stable、Sprint 10 hardening (d9y / 1ll / q3f / 8yu) で page.rs / ruletree.rs に structural change 無し。2026-07-19 → 2026-07-20 で drift 無し。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| W1 | `@page { size: A4 }` | L3401 M4 task `page-rule-cascade`、L3433 mixed-size fixture | M4 明示 assigned | stub | @page rule shell は landed (`crates/raikiri-style/src/page.rs:157-175` `PageRule` struct、`ruletree.rs` `add_stylesheet` @page retention)、しかし `size` descriptor は M1.4 property parser を再利用しており silent drop (page.rs:395 doc comment + `ruletree::tests::page_body_unsupported_property_drops_declaration`)、M4 task `page-rule-cascade` (L3408) が descriptor 個別 parser 側の sub-task 明示無し |
| W2 | `@page :first / :left / :right / :nth-page(n) / :blank / named` | L3401 | M4 明示 | partial | `crates/raikiri-style/src/page.rs:132-141` `PagePseudo` enum は `First` / `Left` / `Right` / `Blank` の 4 variant 受理、`PageSelectorEntry.ident: Option<Atom>` (page.rs:106-124) で named ident (case-sensitive) 受理 = 5/6 実装。しかし **`:nth-page(n)` は page.rs:59-63 の scaffolding note で intentionally excluded** (`no primary source defines … forcing invented behaviour`) — spec 側の primary source 判定待ち |
| W3 | `@page { marks: crop cross }` | L2440 `crop_marks: bool` field only | field 定義のみ、property parse の milestone なし | missing | `parse_value` (property.rs:510-572) `marks` arm 無し、page.rs:395 の "e.g. `size`, `margin`, `marks`" umbrella silent drop、design doc の `crop_marks: bool` field は raikiri-traits `PageBox` / `PageContext` 側の future field 定義 (raikiri-style leaf crate は traits 依存無しで、`PageCascadeResult` の descriptor field 化は M4 defer) |
| W4 | `@page { bleed: N }` | L2439 `bleed: Option<Rect>` field | 同 | missing | 同上 (`bleed` descriptor は silent drop、raikiri-traits `PageBox.bleed` field は future field) |
| W5 | `page-break-before` / `page-break-after` / `page-break-inside` (CSS 2.1 legacy) | mention なし | milestone 未割当、modern spec `break-before` のみ implied | missing | `parse_value` に `page-break-*` / `break-*` arm 無し、M2 task `break-before-after-forced` (L3362) は @page rule ベースの forced break で property-side parser 未 landed |

### Category X: MathML / SVG / other namespace (3 items、Medium conflict)

#### Verification 2026-07-20

**Status distribution**: full 1 / partial 2 / stub 0 / regressed 0 / missing 0。X3 XML namespace handling は html5ever が natively cover (`crates/raikiri-html/src/sink.rs:435` `set_element_namespace` を経由して DOM node に namespace URI が保持される、test `raikiri-html/src/lib.rs:791-803` `parse_wires_svg_namespace_uri` で SVG child への propagation を pin) — audit の "推測、明示なし" は現在 code + test で fully documented。X1 SVG / X2 MathML は namespace URI 保持と MathML annotation-xml integration point 判定 (sink.rs:307-334) が landed する一方、**SVG-specific rendering / MathML-specific layout は raikiri-paint / raikiri-dom 側で実装 0** — HTML foreign content parsing 経路は完成しているが visual output に至らない、DOM 上に "unknown element" として存在するのみ。

**Top gap**: (1) **X1 SVG rendering missing** — 契約書 / 証明書 の署名図 / logo / icon 系需要が silent skip (SVG element は DOM に存在するが raikiri-paint が rendering path を持たない)、External SVG (resolve context `ResourceKind::Svg`) は resolve API 側の shell だけあって Consumer 側 SVG rasterizer 統合の spec が無い。(2) **X2 MathML rendering missing** — 技術書 / 論文組版で数式表示が silent skip、annotation-xml integration point 判定は完成 (HTML LS §12.2.6.5 準拠) だが数式 layout / rendering は 0。

**Since-audit changes**: 無し。SVG / MathML namespace 保持経路は raikiri-html sink refactor で早期 landed、`sink.rs:307-334` の annotation-xml integration point は現 form で 2026-07-19 → 2026-07-20 で touch 無し。s8w (2026-07-20 merge) で `is_non_rendered_html_element` 拡張は datalist / noembed / noframes / rp に留まり、SVG / MathML element は "非-hidden = 存在するが rendering path 無し" のまま。

| # | Missing feature | 該当 line | 状況 | Status | Evidence |
|---|-----------------|----------|------|--------|----------|
| X1 | SVG parsing / inline `<svg>` | L1495 running template M5 pre-resolve、L496 `ResourceKind::Svg`、L2660 debug output | inline SVG rendering milestone なし。External SVG は resolve context のみ | partial | `crates/raikiri-html/src/lib.rs:791-803` test `parse_wires_svg_namespace_uri` (DOM node に `http://www.w3.org/2000/svg` namespace URI 保持、child element へ propagation)、sink.rs:435 `set_element_namespace` 経由。しかし raikiri-paint (`crates/raikiri-paint/src/*.rs`) に SVG element の rendering path 無し、visual output 0 |
| X2 | MathML | L497 `ResourceKind::MathML`、L487 depth cap のみ | External MathML の resolve context 定義のみ、実装 milestone なし | partial | `crates/raikiri-html/src/sink.rs:307-334` `is_mathml_annotation_xml_integration_point` (HTML LS §12.2.6.5 準拠 encoding attribute value 判定)、test `lib.rs:872-948` で annotation-xml integration point 挙動を pin。しかし MathML element の rendering / layout path は raikiri-paint / raikiri-dom 側で未実装、visual output 0 |
| X3 | XML namespace handling in HTML parsing | mention なし | html5ever 内蔵と推測、明示なし | full | html5ever tokenizer が SVG / MathML foreign content parsing を natively support、`raikiri-html/src/sink.rs:435` で null-ns 以外の namespace URI を保持、test `lib.rs:779-803` で HTML / SVG namespace URI wiring を pin (audit 時点の "推測、明示なし" は現在 code + test で fully documented) |

## Surprising findings (17 items、audit 中の highlighted observations)

### 3a. Author style CSS pipeline は M1 "cascade minimal" 以外に explicit milestone がない

L3269 M1 Goals `end-to-end pipeline (parse → cascade minimal → layout single-page → paint → PNG)` の "cascade minimal" が何を意味するか M1.4a UA CSS bundle 以外に定義されていない。M2-M8 の Goals/Tasks (L3351-3618) を追っても Author CSS property (color/font/margin/border) の parse-and-apply 追加 milestone は具体的に無い。paged media 系と GCPM 系のみが具体化。

**Re-verify 2026-07-20**: **still-holds**。Sprint 7-10 で d9y series (Content/StringSet/counter-* Arc wrap) / 1ll (parse_string_set strict 化) / q3f (SEC hardening) / 8yu (raikiri-vrt safe read) / t19 (safe_open +1-probe defense) が landed したが、いずれも既存 M5 GCPM scope 内の hardening で M2 以降の Author CSS property expansion task の新規追加は無い。Design doc §13 の M2-M8 Tasks 一覧 (L3351-3618) は 2026-07-19 → 2026-07-20 で unchanged (`docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` の M-section headings は verify 済み)。Plan/milestone artifact なので code 側 landing では close せず、design doc revision (breakdown proposal Option B の対象) が必要。

### 3b. M1.4a Non-goals が "M2 で必要になったら足す" と書いてあるが M2 tasks に対応 task なし

L3344 「margin / padding 系は M2 simple-multi-page で必要になったら足す」に対し M2 tasks (L3360-3365) は `pagestream-state-machine, layoutbuffer-skeleton, break-before-after-forced, streaming-partial-output-tracking, sink-state-transition-tests, accept_page-error-propagation, RenderStatus-integration, reference-fixtures-multi-page` のみ。**margin/padding parse-and-apply の task はどこにもない**。同様に L3345 M3 の `white-space: pre` / `monospace` も M3 tasks (L3384-3386) が `parley-integration, bidi-support, font-fallback, inline-formatting-context, glyph-run-emit, text-multilingual-fixtures` で明示 task なし。

**Re-verify 2026-07-20**: **still-holds**。Category F verification (sibling 0vv.1) は全 12 item MISSING を確認 — margin/padding/border/width/height/background-color/box-sizing 等 box-model property は 2026-07-19 → 2026-07-20 で `parse_value` (property.rs:510-572) に arm 追加 0。Category E verification (sibling 0vv.1) も white-space (E9) を含む 15 text property MISSING を確認、M3 tasks list への追加 0。設計 doc の "M2 で必要になったら足す" 記述と M2 tasks の分離は unchanged、Plan/milestone artifact なので code landing で close せず。

### 3c. RuleTree struct field はあるが evaluate milestone なし

L1841-1846 で `RuleTree` は `font_face_rules`, `counter_style_rules`, `media_rules`, `supports_rules`, `import_rules` を持つ。しかし 5 種 at-rule の実 evaluation (media condition 判定、@import fetch、@font-face load、@counter-style 適用、@supports evaluate) の milestone task は全 M1-M8 で発見できない。**data structure だけ用意して処理 pipeline は空欄**。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 の Category K verification が判明した design-doc-vs-code drift — 現 `crates/raikiri-style/src/ruletree.rs:31-41` の `RuleTree` struct は `style_rules` + `page_rules` の 2 field のみで `font_face_rules` / `media_rules` / `supports_rules` / `import_rules` / `counter_style_rules` の 5 field は **未実装** (future field comment のみ)。at-rule evaluation path (fetch / condition / load / apply / evaluate) は 5 種 全て path 無し (`ruletree.rs:213-217` `StyleRuleParser::parse_prelude` が `@page` 以外を silent drop)。gap は premise-correction (audit doc は "field 定義のみ" と code state を過大評価していた) 経由で "field も evaluate も両方無い" と rephrase されたが、**evaluate 側 gap は 1 ミリも解消していない** = still-holds。次 planner が breakdown proposal で "@-rules non-paged epic" を切る際、struct field 拡張 + evaluate 実装の 2 phase として切る必要あり。

### 3d. `@media print` の handling は print-first engine で critical だが completely missing

Design goal L54 "fulgur を print-first pipeline に組み替える"、L23 "browser (screen-first) 前提の blitz と方向性が根本的に異なる" と強調するが、**`@media print` (Author が print-only rule を書く場合) の evaluate milestone がない**。RuleTree に `media_rules: Vec<MediaRule>` (L1843) はあるが、これを "raikiri は print context として evaluate" するロジックは全 M で言及なし。**組版 fixture が `@media print { ... }` に依存していたら silent 無視される可能性**。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 K1 verification が pinning test `ruletree.rs:759-770` で `@media print { p { ... } }` の p rule が silent drop されることを confirm、`@media` handling は 2026-07-19 → 2026-07-20 で 0 additions。design doc + code の両側で gap 継続、print-first engine の primary invariant として cardinal case。

### 3e. ::before / ::after selector 実装が無いのに GCPM `content: string()/counter()` は M5 first-class

GCPM `content` property は普通 `::before` / `::after` pseudo-element を経由して generated content を出す (spec: L4 `content` はこれらの pseudo でのみ有効)。しかし design doc は `::before` / `::after` selector を全く言及しない。**M5 で `ContentValueItem` を実装しても、そもそも `p::before { content: counter(chapter) }` の selector が動かないと GCPM 効かない**。この矛盾は Non-Goals にも Open Questions にも書かれていない。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 の Category L verification が `crates/raikiri-style/src/lib.rs:158-169` `PseudoElem` **empty variant enum** (`pub enum PseudoElem {}`) を confirm、`::before` / `::after` selector 受理経路は M0 seed 以来 landing 無し。M5 で `content` / `string-set` / `counter-*` の GCPM static-side は fully landed (property.rs:401-458) だが、pseudo-element selector 経由の generated content consumer 側が閉じており、`p::before { content: counter(chapter) }` のユーザー documented pattern が selector 段で drop する構造は unchanged。Non-Goals にも Open Questions にも記述追加無し = design doc / code の両側で cardinal contradiction が継続。

### 3f. `<a>` hyperlink default underline / color が M1-M8 で担保されない

契約書・技術書で `<a href>` は必ず現れるが `<a>` の UA CSS (`color: blue; text-decoration: underline;`) は M1.4a bundle (L3325 = `html, body, div, p, h1-h6` のみ) に入っていない。L3342-3349 M1.4a Non-goals も form 系 / margin/padding / pre/code / table のみ言及、`<a>` は無視。**Reference document `invoice-en` (L2832) は hyperlink 含む可能性大だが link 表示が silent broken リスク**。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 の Category Q Q5 verification が `crates/raikiri-html/src/ua/minimal.css` に `a` selector が **依然無い** ことを confirm (10 element = html/body/div/p/h1-h6 のまま unchanged)、Category E E3 verification が `text-decoration` property が `parse_value` match arm 皆無 = double gap 継続。M2-M8 tasks list への `<a>` UA CSS 追加も 2026-07-19 → 2026-07-20 で 0 additions、design doc / code の両側で hyperlink 表示 silent broken risk 継続。

### 3g. List rendering (`<ul>`/`<ol>`/`<li>`) の milestone assignment が完全欠落

L3325 M1 UA bundle に含まれず、L3342-3349 M1.4a Non-goals にも `<ul>`/`<ol>`/`<li>` の言及なし、M2-M8 tasks にも list 系 task なし。**「目次付き技術書」(L90) が primary goal reference の 1 つなのに list marker 実装 milestone がない**。`::marker` pseudo-element も同じく欠落 (Category L L4)。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 の Category Q Q2 (`<ul>`/`<ol>`/`<li>` UA CSS 無し) + Category U 全 6 item MISSING (list-style-* / marker / counter-style) + Category L L4 `::marker` missing の trio が三重に confirm、`counter-*` property の source-of-truth (counter-tree) は M5 で landed (property.rs:401-415) しているが consumer 経路が全 gap。目次付き技術書 primary goal reference は依然 unsupported、design doc / code の両側で gap 継続。

### 3h. `!important` の cascade handling がどこにも触れられていない

L840, L1855 "cascade / specificity / inheritance は自前実装" とあるが **`!important` (CSS Cascading L4 §6.3、origin と一緒に cascade order の major factor) の言及がゼロ**。M1 の `css-cascade-basic` task (L3277) がこれをどこまで含むかも不明。

**Re-verify 2026-07-20**: **resolved**。sibling 0vv.1 の Category O O3 verification が `!important` の完全実装を confirm — `crates/raikiri-style/src/rule.rs:20-21` `Declaration.important: bool` field、`rule.rs:64` の `cssparser::parse_important` invocation、`cascade.rs:111-118` の origin reversal (UA important > Author important > Author normal > UA normal、CSS Cascading L4 §6.3 準拠)。audit 時点 "design doc mention ゼロ" だったが code 側は M1.4a scope で fully landed、audit doc verification が code state を kick して gap を close する好例。次 planner の Option B (design doc revision) では M1 tasks list の `css-cascade-basic` に `!important reversal` を明示追記するだけで close 可能。

### 3i. calc() 実装は M4 に暗黙 imply だが M4 tasks に無い

L316-328 で `calc feature invariant` を丁寧に議論し L324 で「M4 sandboxed resolver で CSS calc() 実装時に calc arena を raikiri-dom 側に配置」と一箇所書いてある。**しかし M4 tasks list (L3407-3420) に `css-calc-parse` や `calc-arena-implementation` に相当する task が無い**。roborev で明示化するか、実際は M2 (L319) が正しいのか不明。

**Re-verify 2026-07-20**: **still-holds**。Category B B8 verification (sibling 0vv.1) が `parse_font_size` 等 length parser (property.rs:666-676) に `Token::Function("calc")` 分岐無しを confirm、calc() 実装は M2 / M4 いずれの tasks list にも task 追加 0。Plan/milestone artifact なので code landing で close せず、次 planner が design doc revision で M2 または M4 tasks list に `css-calc-parse` を明示追加する判断が必要。

### 3j. HTML5 default 挙動 (block/inline の要素分類全体) の spec source が明示されていない

L3336-3339 で "参照 OK: CSS 2.1 App.D、HTML LS §14 Rendering、各 CSS module Sample style sheet" と bundle の spec source は決めているが **M1 bundle が 10 要素で止まっている理由 (`html, body, div, p, h1-h6`) の "残り" をどう埋めるかの計画がない**。HTML LS §14.3 Sections だけでも `article, section, nav, aside, header, footer, main, hgroup` の 8 block 要素があり、これらは silent inline 化される。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 の Category Q Q1 verification が `crates/raikiri-html/src/ua/minimal.css` の 10 element (html/body/div/p/h1-h6) unchanged を confirm、HTML LS §14.3 の残 8 block element (article/section/nav/aside/header/footer/main/hgroup) + blockquote / figure / figcaption / hr は依然 silent inline 化。M2-M8 tasks list への追加 0、design doc の "残り" 補完計画無し継続。s8w (2026-07-20 merge) で `is_non_rendered_html_element` に datalist/noembed/noframes/rp が追加されたが、これは "hidden element" 側 (Non-Goal umbrella)、block element bundle expansion とは orthogonal。

### 3k. `@media print` と `@page` は spec 上別次元だが設計で混同されていないか

L1843 に `media_rules`、L1840 に `page_rules` と分離した field はある。しかし `@media print { p { color: black } }` が `@page` の context と別扱いなことの evaluate ロジック / cascade merging の spec は不明。M4 で `@page` cascade を実装するが、`@media print` condition の evaluate is out of scope。**"print-first" と言いつつ `@media print { }` を evaluate しないのは矛盾**。

**Re-verify 2026-07-20**: **still-holds**。3d + 3c と連携する finding。sibling 0vv.1 K1 verification が `@media print { p { ... } }` の p rule silent drop を pinning test で confirm、@page cascade は M4 tasks (L3407-3411) で `page-rule-cascade` / `page-name-transition-table` / `per-page-pagebox-resolver` として明示化されている一方、`@media` evaluation task は M1-M8 全 milestone に task 追加 0。design doc / code の両側で `@media print` handling の gap 継続、"print-first" invariant vs `@media print` unevaluated の矛盾は 2026-07-19 → 2026-07-20 で解消せず。

### 3l. Named color の Set は本 doc で完全に定義されていない

L3287 で M1 hello-world が `color:red` を要求するので少なくとも 1 個の named color parse は M1 に含まれる必要がある。しかし **CSS Color L4 の 140+ named colors 全 set をいつ完備するかは未定**。partial impl で red だけ通す設計なのか、全 set を M1 で入れるのかも不明。

**Re-verify 2026-07-20**: **resolved**。sibling 0vv.1 の Category C C4 verification が `crates/raikiri-style/src/property.rs:593-596` `parse_named_color` の cssparser API 経由の 140+ 全 set 受理を confirm — M1 で "全 set 一括 landing" が chosen された state (partial impl for red only ではない)。audit doc の "M1 で 1 個だけか全 set か未定" は現 code で "全 set" 側で resolved。次 planner の Option B (design doc revision) では M1 tasks の `css-cascade-basic` に "CSS Color L4 named colors 全 set (via cssparser)" を明示追記すれば doc 側も align。

### 3m. `ComputedValues` の具体的 field enumeration が無い

L840, L844, L1860 で `ComputedValues` type 名の言及はあるが **「どの CSS property を computed values に含めるか」の列挙が本 doc 内に無い**。stylo リプレースだけど stylo の huge property table 相当を作るのか、必要 property のみ minimal に定義するのかも不明。Sizing / positioning / typography のどれを includeするかの choice が M1 の "cascade minimal" の中身になるはずだが表現なし。

**Re-verify 2026-07-20**: **resolved**。本 task Category S S3 verification (`crates/raikiri-style/src/computed.rs:41-124`) が `ComputedValues` struct に 11 concrete field (color / font_family / font_size / font_weight / display / counter_reset / counter_increment / counter_set / content / string_set / running_templates) が named + 各 field に doc comment (inherited / non-inherited + initial value + spec section 引用) 付きで landed していることを confirm。audit doc "minimal 定義か huge property table か choice 不明" は code 側で "minimal + incremental (M1.4 4 fields → M1.4a display + M5 counter/content/string-set/running-templates)" 側で resolved。3h / 3l と同じ論理 (code landing = resolved)。次 planner の Option B では design doc §7 (data structure section) に code の 11 field 列挙 + `#[non_exhaustive]` incremental 拡張 policy を明示追記すれば doc-side も align。

### 3n. User origin CSS の位置付けが inconsistent

L3328 "UA < User < Author" と L4 spec を参照し、しかし L3348-3349 で "User origin (~/.config/raikiri/user.css 等) は Consumer が `extra_stylesheets` に流す方式で吸収 (M1 では専用 origin を作らない)"。**"M1 では作らない" と書いてあるので M2+ で作る可能性を残しているが、それがどの M か / Future Work か Open Question かは記載なし**。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 の Category O O2 verification が `crates/raikiri-style/src/cascade.rs:111-118` `cascade_rank` の "UA + Author 2 段のみ handle、User origin は Consumer が extra_stylesheets 経由で Author 化" 側の design 実装を confirm — M1.4a scope の設計判断そのままで、M2+ での User origin 昇格 milestone は 2026-07-19 → 2026-07-20 で追加無し。design doc 側は "M1 では作らない" のみで M2+ の位置付けは Future Work / Open Question どちらとも書かれず継続、Plan/milestone artifact なので code landing で close せず。

### 3o. Non-Goal L104 の "全 CSS 仕様の網羅" は逆に "何を網羅する予定か" が読み取れない

Non-Goals は排除リスト、Goals は organizational principle。**"どこまで対応するかの CSS property カバレッジ table" が本 doc に存在しない**。fixture-driven (T1 reference documents) と言いつつ "fixture が要求する CSS の union" の明示的 list もない。**"組版品質を primary goal" と言った瞬間に "組版に必要な CSS の union" が定義される責任があるが、それが空欄**。

**Re-verify 2026-07-20**: **still-holds**。本 audit doc 自体 (2026-07-19-milestone-gap-audit.md) が partial answer を提供 — 24 category × ~130 item の gap inventory を作成した状態 — だが、design doc §2 Non-Goals / §15 Open Questions への feedback loop は未 landed、design doc 側は依然 CSS property coverage table を持たない。次 planner の Option B (design doc revision) の主要意義の一つがこの table 追加、Plan/milestone artifact なので code landing で close せず。

### 3p. WPT tracking category (L2995-3007) が実装 category を暗黙 imply しているが milestone task と reconcile されていない

`css/css-page/` (高)、`css/css-fragmentation/` (高)、`css/selectors/` (高)、`css/css-text/` (中)、`css/css-values/` (中) が tracking 対象。**`css/css-values/` は length units / calc / var 等の spec cover、`css/css-text/` は text-align / white-space 等の property cover。tracking 表と milestone task の対応関係が無い**。

**Re-verify 2026-07-20**: **still-holds**。sibling 0vv.1 の Category A (selectors) / B (values / length units) / E (text) verification が該当 WPT category が cover するべき property の大多数を MISSING と confirm — WPT tracking 表と実 code coverage / milestone task の三者間 reconciliation は 2026-07-19 → 2026-07-20 で 0 progress。design doc §14 (WPT tracking) と §13 (Milestone Plan) の紐付け明示化が次 planner の Option B の焦点、Plan/milestone artifact なので code landing で close せず。

### 3q. `page:` property の CSS side の milestone が不明

L2454 の page name 遷移表で "次 block に `page: X`" と CSS `page` property を前提としている (Fragmentation L3)。**しかし CSS `page` property の parse-and-store milestone が明示 task にない**。M4 @page rule cascade tasks (L3408) は "@page rule" の cascade で、"要素 selector が `page: X` を宣言する property" とは別。

**Re-verify 2026-07-20**: **still-holds**。`parse_value` (property.rs:510-572) に `page` property arm 無し、Author style で `.chapter { page: cover }` を書いても declaration が silent drop。M4 tasks (L3408) の `page-rule-cascade` / `page-name-transition-table` は @page **rule** 側の実装で、要素 selector 側の `page:` **property** parse-and-store は 2026-07-19 → 2026-07-20 で task 追加 0。Plan/milestone artifact なので code landing で close せず、次 planner が Option B で M4 tasks に `page-property-parse-store` を追加する判断が必要。

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
