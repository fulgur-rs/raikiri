# Milestone Plan gap breakdown proposal

**Date**: 2026-07-20
**Trigger**: 0vv umbrella epic (`raikiri-spike-0vv`) verification pass completion — sibling task pair `raikiri-spike-0vv.1` (12 category A/B/C/D/E/F/G/K/L/O/Q/U) + `raikiri-spike-0vv.2` (12 category H/I/J/M/N/P/R/S/T/V/W/X + §3 findings) が現 code state (2026-07-20) 対する verification と Status column landing を完了、次 planner が使う **child epic breakdown proposal (or spec revision PR outline)** を draft する
**Source**: audit doc `docs/superpowers/specs/2026-07-19-milestone-gap-audit.md` + Verification 2026-07-20 pass (bd raikiri-spike-0vv.1 + raikiri-spike-0vv.2)
**Audience**: 次 planner (child epic 起票判断) + design doc maintainer (spec revision PR 判断)
**Author scope**: Draft only — 実 bd 起票 / design doc PR は次 planner の judgment。本 doc は observation summary + 2 optional path + recommend

---

## Executive summary

**Verified item distribution** (grand total 198 items × 24 categories):

| Status | Count | Percent | Notes |
|---|---|---|---|
| full | 21 | 10.6% | M1.4a UA CSS 10 element + M5 GCPM static side (counter-*/content/string-set/running-templates) + Category O cascade skeleton + Category P cssparser 依存 resilience 側 + Category X namespace preservation |
| partial | 15 | 7.6% | Font resolution (D1-D4)、data preservation without CSS access (R7)、resolved-value stage (S1)、@page selector 5/6 (W2)、SVG/MathML namespace-only (X1/X2)、Cascade origin 2-of-3 tier (O2)、CSS Color transparent (C6)、Q1 partial block bundle、Q3 M3 defer、A9 selector-list partial、CSS parse resilience partial (P2/P3 cssparser inherent) |
| stub | 1 | 0.5% | @page rule shell 全体 landed だが descriptor 個別 silent drop (W1) |
| missing | 161 | 81.3% | ほとんどが `parse_value` (property.rs:510-572) match arm 未追加、または `PseudoClass` / `PseudoElem` / at-rule dispatch の未実装 |

**Non-Goal-adjacent items** (design doc §2 で明示排除 or Non-Goal umbrella 挙動一致): C8 CMYK、M9 `:link` / `:visited`、M10 `:target`、Q12 `<script>` / `<noscript>`、T2 vertical writing、T6 ruby、V2 3D transform、L5 `::selection`、L6 `::placeholder` — 計 9 item。**本 breakdown proposal では child epic mapping から除外**。実 addressable gap は 198 - 9 = 189 item (~150 が missing、以下の Option A で 6 primary child epic + Epic 7 optional に mapping)。

**Since-audit code drift**: Sprint 10 hardening (d9y series + 1ll + q3f + 8yu + t19 + s8w) は既存 M5 GCPM / raikiri-vrt / raikiri-html scope 内の SEC / safe I/O 強化に集中、Category A-X の property / selector coverage には 0 additions。sibling 0vv.1 の Category K verification が判明した design-doc-vs-code drift (RuleTree の 5 field 未実装、future field comment のみ) は audit doc の記述を code state で **過大評価** していた形で発見 = 次 planner の Option B (spec revision) の対象。

---

## Option A: Child epic per category cluster

24 category 全 item (Non-Goal 除外後 ~150 addressable missing) を **6 primary child epic + Epic 7 optional (N/S 収容)** に mapping、次 planner が sprint 単位で pour できる shape。各 epic に priority + dep ordering + expected task count を明示。

### Cluster mapping table

| Epic # | Proposed bd stub name | Category cluster | Addressable items | Priority | Depends on | Expected tasks | Rationale (code-cohesion) |
|---|---|---|---|---|---|---|---|
| 1 | raikiri-spike-`<new>`-author-basics | B (length units), C (color 残 C3/C5/C7 = hsl/currentColor/L4 color functions), D (font-*), E (text-*), F (box model), G (display), H (position H2-H5/H7-H8 = author flow position + inset + z-index) | ~56 | **P1** | — (foundation) | 32-42 | 全 category が `parse_value` (property.rs:510-572) match arm 拡張 + `ComputedValues` field 拡張 + taffy Style translation の 3 phase を共有、code cohesion 高、Author style parse-and-apply の primary seed。位置 property (H) は Author flow の一部として box model と同 landing 場所 |
| 2 | raikiri-spike-`<new>`-selector-expansion | A (selectors 12), M (non-interactive pseudo-class 8、M9/M10 除外), O O4 (:where() 0-specificity、A10 と同 landing) | ~21 | **P1** | — (parallel to Epic 1) | 15-20 | `ruletree.rs:76-78` `is_type_or_universal_only` gate 撤去 + `PseudoClass` enum 拡張 + selectors crate matching path の 3 phase、選択肢を parser-side に集中、Epic 1 と file-orthogonal。O4 (:where() の 0-specificity) は A10/A12 landing と 同 sprint で cascade specificity 側 adjust する形 |
| 3 | raikiri-spike-`<new>`-html-surface | Q (HTML elements UA CSS 13、Q12 Non-Goal 除外), R (attribute promotion 8、R4 = Q11 defer), O O2 (User origin 3-tier 昇格判断) | ~21 | **P1** | Epic 1 (font/text/box model)、Epic 2 (descendant selector) | 12-18 | UA CSS bundle expansion (`raikiri-html/src/ua/minimal.css` 拡張) + presentational hint mapping (raikiri-dom side attribute→computed) の 2 layer、fixture author が最初に触る surface。O2 (User origin as 3rd tier) は Consumer surface に extra_stylesheets vs 専用 user origin の judgment を伴い、HTML surface epic 側で decision |
| 4 | raikiri-spike-`<new>`-generated-content-and-lists | K (@-rules non-paged 9、K5 Non-Goal-adjacent 除外), L (pseudo-elements 4、L5/L6 Non-Goal-adjacent 除外), U (list styling 6) | ~19 | **P2** | Epic 2 (selector grammar) | 15-20 | `PseudoElem` enum populate (`::before` / `::after` / `::first-letter` / `::marker`) + at-rule dispatch (media / import / font-face / supports / counter-style / layer) + list-style-* property arm。**surprising 3e/3g/3d の cardinal contradiction を close する epic** |
| 5 | raikiri-spike-`<new>`-css-variables-and-math | J (custom properties 6) + B B8/B9 calc/min/max/clamp (propagate) | ~8 | **P2** | Epic 1 (length parser stable) | 8-12 | `--<ident>` declaration 経路 + `var()` / `env()` / `calc()` substitution stage を parse_value 前段に追加、@property registered custom properties は M4+ 相当 |
| 6 | raikiri-spike-`<new>`-paged-completion-and-media | W (@page 内 property 5), T (writing-mode 4、T2/T6 Non-Goal 除外), I (background/gradient 10), V (transform/opacity/filter 5、V2 Non-Goal 除外), X (SVG/MathML rendering 2、X3 full) | ~26 | **P2** | Epic 1 (length + display) + paint side coordination | 20-25 | @page descriptor (size/marks/bleed) parser + break-* property parse + writing-mode parse-and-ignore + background-image / linear-gradient + opacity + SVG raster bridge (raikiri-paint 側との協調)。**surprising 3d/3k の "print-first" invariant を close する epic** |
| 7 (opt.) | raikiri-spike-`<new>`-css-wide-keywords-and-value-stages | N (CSS wide keywords 5), S (resolved value stages 2 open) | ~7 | P3 | Epic 1-6 の多数 property landed 後 | 5-8 | Cascade wide keyword (`inherit` / `initial` / `unset` / `revert` / `revert-layer` / `all`) の fast-path + used/actual stage segregation。Spec compliance level bump、他 epic の landing 度合いに応じて優先度が上がる |

**Category coverage completeness**: 24 category × 198 items 全てが Epic 1-7 mapping 対象 (7 epic tuple = A + B + C + D + E + F + G + H + I + J + K + L + M + N + O + P + Q + R + S + T + U + V + W + X)。**Category P (parse resilience)** は 5/5 が full or cssparser-inherent partial (addressable missing 0) のため **child epic mapping 不要 = 明示的 omit**。P2/P3 の cssparser upgrade regression pin test 追加は debt/quality 分類で Epic 1-6 の landing sprint 内に組み込む形 (dedicated epic 不要)。Non-Goal 除外 (9 items: C8 / L5 / L6 / M9 / M10 / Q12 / T2 / T6 / V2) を差し引いた **実 addressable missing ~150 items が Epic 1-6 core にほぼ全収束**、Epic 7 は additive で他 epic 完了度合いに応じて起票判断。

### Dep DAG

```
Epic 1 (author-basics) ──┬──▶ Epic 3 (html-surface) ─────┐
                         │                                ├──▶ Epic 6 (paged-completion)
Epic 2 (selector-expansion) ──▶ Epic 4 (generated-content) ─┘
                         │
                         └──▶ Epic 5 (css-variables-and-math)
                                             │
                                             ▼
                                    Epic 7 (wide-keywords、opt.)
```

Epic 1 + Epic 2 は完全 parallel (parse_value 側 vs selector 側で file-orthogonal)、Epic 3 は Epic 1 (font/text/box model の parse-apply cycle) + Epic 2 (descendant selector for `.chapter h2` 等) の両方に依存、Epic 4 は Epic 2 (`::before` / `::after` の selector grammar) 依存、Epic 5 は Epic 1 依存 (calc の length semantics 前提)、Epic 6 は Epic 1 依存 + raikiri-paint 側 coordination。

### Sprint pour granularity guidance

各 Epic は次 planner が **1-3 sprint に分割 pour** することを想定 (~5-15 tasks / sprint × 2-3 sprint):
- Epic 1: 3 sprint (length + display で 1、font/text で 1、box model で 1)
- Epic 2: 1 sprint (class/id/attribute/combinator を 1 batch)
- Epic 3: 2 sprint (UA CSS bundle expansion で 1、attribute promotion で 1)
- Epic 4: 2 sprint (pseudo-element grammar で 1、at-rule dispatch + list で 1)
- Epic 5: 1 sprint (var/calc/env の unified substitution pass)
- Epic 6: 2 sprint (@page descriptor + break-* で 1、writing-mode + background + opacity + SVG bridge で 1)

Total ~11 sprint (Epic 1-6 core)、Sprint 11-22 相当。

---

## Option B: Spec revision PR (design doc §13 Milestone Plan 直接改訂)

`docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` (~4,118 line) の §13 Milestone Plan (L3185-L3618) に対して、以下 5 insertion site で task 追加または section 新設を提案。各 site に 3-5 line draft task text + 3-5 line trade-off を提示。

### Insertion site 1: M1.4b "Author CSS parse-and-apply seed" 新設

**Insertion location**: L3350 直後 (M1.4a Non-goals close の直後、M2 header L3351 前)

**Draft**:
```markdown
#### M1.4b: Author CSS parse-and-apply seed (0vv 対応)

**背景**: audit doc 2026-07-19 の Category F (box model) 全 12 item MISSING、
Category B (length units) B1-B7 MISSING、Category E (text-*) 15 item MISSING が
M2 "必要になったら足す" (M1.4a Non-goals) の pin 状態を長期化させている。
Reference fixture (契約書 / 招待状 / 目次付き技術書) を fixture-driven に landing
するには margin / padding / border / background-color / text-align / line-height
の 6 property を最低限 M2 内で受理する必要がある。

**Tasks**:
- author-box-model-parse (margin / padding / border-* / background-color の
  parse_value arm 追加 + ComputedValues field 追加、physical のみ)
- author-text-basics-parse (text-align / text-decoration / line-height の
  parse_value arm 追加)
- author-length-em-percent (em / rem / % / pt の Length variant 拡張、
  parse_font_size 汎用化)
- author-basics-vrt-fixtures (margin/padding が反映されることを VRT で pin)

**Acceptance**: `<p style="margin: 20px; padding: 10px; background-color: #eee">`
が block box として render、`text-align: center` の p が centered layout、
`font-size: 1.2em` が inherited size × 1.2 で resolve される。
```

**Trade-off**: M1.4a "M2 で必要になったら足す" 記述との整合を M1.4b 新設で明確化、既存 M1.4a scope (10 element UA CSS + 4 property) を破壊せず incremental extension として位置付ける。既存 M2 tasks (pagination) には影響無し、Author CSS の landing を M1 内に閉じ込めることで hello-world VRT の scope creep を回避。

### Insertion site 2: M2 tasks list 拡張

**Insertion location**: L3365 (M2 tasks list の最終 item `reference-fixtures-multi-page` 直後)

**Draft**:
```markdown
- **css-break-property-parse** (0vv 対応): `page-break-before` / `page-break-after`
  / `page-break-inside` (CSS 2.1 legacy) + `break-before` / `break-after` /
  `break-inside` (modern) の parse_value arm 追加、@page 側 break との連携
```

**Trade-off**: 既存 `break-before-after-forced` (L3362) task は @page rule 側の forced break、property side (要素 selector の `break-*`) は task が明示的に無く gap (audit 3q の派生)。1 task 追加で Category W W5 close、M2 scope 内で自然。

### Insertion site 3: M3 tasks list 拡張

**Insertion location**: L3386 (M3 tasks list の最終 item `text-multilingual-fixtures` 直後)

**Draft**:
```markdown
- **css-white-space-parse** (0vv 対応): `white-space` (`normal` / `pre` / `nowrap`
  / `pre-wrap` / `pre-line`) の parse_value arm + ComputedValues field
- **css-writing-mode-parse-ignore** (0vv 対応): `writing-mode` (`horizontal-tb`
  のみ受理、`vertical-*` は parse-and-ignore で silent drop 明示) + `direction` の
  parse_value arm。Non-Goal L111-112 (縦書き) との整合明示
- **ua-css-pre-code-monospace** (M1.4a M3 defer 対応): `raikiri-html/src/ua/
  minimal.css` に `pre, code { white-space: pre; font-family: monospace; }` 追加
```

**Trade-off**: M1.4a M3 defer 明示項目 (`<pre>` / `<code>` の white-space + monospace) が M3 tasks list に task 化されていない (audit 3b の派生)。3 task 追加で Category E E9 (white-space) + Category T T1/T3 (writing-mode parse-and-ignore) + Category Q Q3 (`<pre>` / `<code>` UA CSS) の 3-fold close。

### Insertion site 4: M4 tasks list 拡張

**Insertion location**: L3411 (M4 tasks list の `pagecontext-seed-builder` 直後、Layer 2 security block 前)

**Draft**:
```markdown
- **at-page-descriptor-parse** (0vv 対応): `@page { size: <length>{1,2} |
  <page-size> || [portrait | landscape] }` の descriptor 個別 parser (現状は
  M1.4 property parser を再利用して silent drop)、`marks` / `bleed` は同 site
- **page-property-parse-store** (surprising 3q 対応): 要素 selector の
  `page: <custom-ident>` property parse + ComputedValues field 追加、
  `page-name-transition-table` (既存 task) の入力側を確立
- **css-calc-parse** (surprising 3i 対応): `parse_length_value` に
  `Token::Function("calc")` 分岐追加、L316-328 の calc invariant を M4 で
  明示 landing (現状 L324 "M4 sandboxed resolver で" と暗黙 imply のみ)
- **at-media-print-evaluate** (surprising 3d 対応): `@media print { ... }` の
  condition evaluation を print context として fixed true に固定、`RuleTree`
  struct に `media_rules` field 追加 (audit doc の "field 定義のみ" premise を
  code state と align)
```

**Trade-off**: 4 task 追加は M4 の scope creep 懸念 (round 5 review Task #4 で "M4 が incrementally reviewable な粒度に収まる" 表明済み) と拮抗。しかし @page descriptor / page property / calc / @media print は 4 者いずれも "M4 に暗黙 imply" 状態 (L3401 / L2454 / L324 / L1843) で明示化が gap の中核 (audit surprising 3i/3q/3k/3d の 4 finding を単発 close)。分割案として `at-page-descriptor-parse` + `page-property-parse-store` のみ M4 に landing し、`css-calc-parse` + `at-media-print-evaluate` は Insertion site 5 で M9 新設 side に送る 2-batch 判断も可。

### Insertion site 5: M9 (or Post-M8) 新設 "Selector L4 + coverage completion"

**Insertion location**: L3618 直後 (M8 close の直後、"milestone 依存 DAG" L3619 前)

**Draft**:
```markdown
### M9: Selector L4 + coverage completion (0vv 対応)

**背景**: audit doc 2026-07-19 で inventory 化した 24 category 130 item のうち、
M1-M8 で明示 milestone を持たない ~90 item (Non-Goal 除外後) の landing scope。
"組版品質を primary goal" と declare した以上 CSS coverage の union を明示する
責任 (audit surprising 3o) と、Selector L4 (:is/:where/:not/:has) の base level
support を確立する scope。

**Goals**:
- Selector 拡張 (class / id / attribute / combinator + Selector L4)
- CSS variables + `calc()` / `min()` / `max()` / `clamp()` の unified substitution
- HTML element UA CSS bundle expansion (HTML LS §14.3 の残 block element 8 個 +
  hyperlink / list / hr / blockquote)
- @-rules non-paged (@media / @import / @font-face / @supports / @counter-style /
  @layer) の evaluation pipeline
- Pseudo-elements (::before / ::after / ::first-letter / ::marker) の generated
  content selector grammar
- List styling (list-style-* + ::marker)
- Attribute promotion (<img width/height> + lang / dir attribute)
- Writing-mode parse-and-ignore + logical properties (partial、subject to
  physical margin/padding landed)

**Non-goals** (M9 scope 外、Future Work):
- Vertical writing (Non-Goal §2)
- Ruby (Non-Goal §2)
- 3D transform (Non-Goal §2 相当)
- CMYK / ICC color (Non-Goal §2、fulgur 責任)
- Interactive selectors (:link / :visited / :target 等、Non-Goal §2 fail-closed)
- Form control interactive behavior (Non-Goal §2)

**Reference fixtures**:
- `tests/reference/selector-l4-basic/` (:is/:where/:not sample)
- `tests/reference/list-with-marker/` (目次付き技術書 primary goal reference の
  base line、surprising 3g close)
- `tests/reference/hyperlink-invoice/` (契約書 hyperlink 表示、surprising 3f
  close)
- `tests/reference/generated-content-chapter-title/` (::before で chapter number、
  surprising 3e close)

**Acceptance criteria**:
- audit doc 2026-07-19 の High-priority ~90 item のうち scope 内 ~80 item が
  full or partial landing
- CSS property coverage table (design doc §7 新設) が code state と align
- Surprising findings 3e/3f/3g/3h/3l/3m が resolved 判定 (audit doc §3 の
  re-verify pass で確認)
```

**Trade-off**: M9 新設は M0-M8 の "hello-world → 決定論 stress" の progression と並置、Coverage completion phase として位置付け。M1-M8 の Non-goals (M1.4a "M2 で") 記述との整合を M9 で回収する形。M9 の estimate は不確定 (Option A の Epic 1-6 core とほぼ overlap で ~11 sprint 相当) だが、design doc 側で "M9 phase" の commitment を明示すれば bd 起票側 (Option A) と reconciled。M9 を細分割 (M9a/M9b/...) する 2 段判断も可。

---

## Recommend

**Preferred**: **Option A (child epic per category cluster)** を primary、**Option B (spec revision PR) を Option A の insertion site 1 + 3 の 2 site に限定して同時実施** の hybrid strategy。

**Rationale (2-3 line)**:
1. Option A は sprint pour granularity が明確 (6 Epic × ~2 sprint = 11 sprint) で **bd instantiation の judgment cost が低い** — 次 planner が 1 Epic ずつ pour し、実装度合いに応じて Epic 内 sprint 数を後付けで調整可能。逐次 landing で design doc revision の要否を code state から逆算できる。
2. Option B の insertion site 1 (M1.4b Author CSS parse-and-apply seed) + insertion site 3 (M3 white-space / writing-mode / `<pre>` UA CSS) の 2 site は "M1.4a Non-goals 記述と M2/M3 tasks list の整合が現状 broken" (audit 3b 派生) を **単発 close** できる小 site で、Option A の Epic 1 / Epic 3 と重ならず、design doc revision cost が小さい。両 site を Option A dispatch 前 sprint に landing すれば、Epic 1 / Epic 3 の scope 定義が design doc に anchored される。
3. Option B の insertion site 4 (M4 4 task 追加) + site 5 (M9 新設) は **Option A で code 側 landing が進んでから逆算で design doc に反映するのが safe** — surprising 3i/3d/3k 系の close は code state proof (Epic 6 landing 後) を premise にした doc revision の方が spec/code drift risk が低い。

**Option C rejection (do-nothing = 現 design doc + Non-Goal を保持) の rationale (1-2 line)**: audit doc 2026-07-19 の 24 category 130 item + verification 2026-07-20 の 198 item concrete inventory + surprising findings 3e/3f/3g の primary goal reference (契約書 / 目次付き技術書) 直撃 gap が明示化された以上、"組版品質を primary goal" declare (design doc §1) と bd portfolio state の整合を取らずに Non-Goal 傘下に維持することは spec ownership の内部矛盾を継続させる。特に surprising 3e (::before/::after 未実装 vs M5 GCPM content first-class) は Non-Goals にも Open Questions にも書かれておらず、design doc の explicit stance (Non-Goal or Future Work or Open Question) が必要。Option C は "gap をゼロと宣言する / gap は不変と宣言する" の 2 choice しかなく、audit + verification 結果と reconcile 不可。

---

## 次 planner の action checklist

1. 本 doc を `bd decision` に record (rationale + preferred option + Option C rejection)
2. Option A の Epic 1 (`raikiri-spike-<new>-author-basics`) を bd 起票、Sprint 11 relase target で分割 pour 開始
3. Option A の Epic 2 (`raikiri-spike-<new>-selector-expansion`) を parallel 起票
4. Option B の insertion site 1 (M1.4b 新設) + site 3 (M3 task 拡張) を design doc PR として draft
5. Epic 6 (paged-completion) の landing 度合いに応じて Option B insertion site 4 + 5 を後付けで PR 化
6. audit doc の "Verification pass 2026-07-21" 相当を実施する 3-4 sprint 後 checkpoint を bd に記録 (child epic の landing 進捗計測)

---

## Cross-references

- Audit doc: `docs/superpowers/specs/2026-07-19-milestone-gap-audit.md` (verify pass 2026-07-20、24 category status distribution + §3 findings re-verify)
- Design doc: `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` §13 Milestone Plan (L3185-L3618)
- Umbrella epic: `bd raikiri-spike-0vv`
- Verification tasks: `bd raikiri-spike-0vv.1` (A/B/C/D/E/F/G/K/L/O/Q/U) + `bd raikiri-spike-0vv.2` (H/I/J/M/N/P/R/S/T/V/W/X + §3 findings + 本 breakdown proposal)
