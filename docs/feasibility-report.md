---
title: raikiri M0 feasibility report
status: Final
date: 2026-07-14
author: Mitsuru Hayasaka (@mitsuru)
related:
  - "設計仕様書: `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` §M0"
  - "beads issues: raikiri-spike-nzv.5 〜 raikiri-spike-nzv.11 (すべて closed)"
  - "後続 gate: raikiri-spike-nzv.13 m0-readiness-gate (本 doc が判定 material)"
---

# raikiri M0 feasibility report

## 0. この文書の位置付け

設計仕様書 §M0「Production 生存 artifacts」に列挙された M0 spike 群
(`raikiri-spike-nzv.5` 〜 `raikiri-spike-nzv.11`) の検証結果を集約する
production doc。M0 終了後 M1 以降も残り、M1 実装時の型定義・依存表・
アーキテクチャ判断の一次資料となる。

**判定分業**: 本 doc は「M0 outcome 判定の material」を提供する。M0 の
formal な close 判定・M1 unblock signal は `raikiri-spike-nzv.13`
(m0-readiness-gate) が発行する。この分業は設計仕様書 §M0「M1 依存の precise
定義」に定められている(single `bd close raikiri-spike-m0` では M1 は自動
open しない)。

**証拠原則**: 全 finding は commit hash + test 名 + file path で参照可能な
形で記録する。推測記述は避け、実際に compile / test / run した artifact のみ
を引用する。

---

## 1. Executive summary

### 1.1 検証結果 (7/7)

| # | Spike | Verdict | 補足 |
|---|---|---|---|
| nzv.5  | parley FontContext Send + Sync 適合性 | **OK** | 静的 assert + runtime thread test |
| nzv.6  | taffy Block/Flex/Grid + 並列可能性 | **OK** | `Style: !Send` under `calc` の caveat あり |
| nzv.7  | anyrender_vello_cpu byte-identical raster | **OK** | rayon 1v4 targeted test は M1 defer |
| nzv.8  | selectors standalone (stylo 非依存) | **OK** | 一度 NEEDS_DESIGN_CHANGE → encapsulation で OK |
| nzv.9  | cssparser @page / @counter-style / @font-face 拡張機構 | **OK** | 設計変更不要 |
| nzv.10 | selectors + cssparser 版整合 (API-level) | **OK** | 単一 `cssparser::Parser` 共有可能 |
| nzv.11 | anyrender PaintScene adapter compile-spike | **OK** | 予想 (bridge trait 必要) を **refute** |

### 1.2 M0 outcome

設計仕様書 §M0 outcome の 2 分岐:

1. 全 7 検証項目で "OK" → M0 epic close、**M1 unblock**
2. 1 つでも "NEEDS_DESIGN_CHANGE" → 別 epic `raikiri-spike-m0-revision` 起動

→ **分岐 1**(全 OK)。ただし nzv.8 は初回 NEEDS_DESIGN_CHANGE → follow-up
で OK に格上げした経緯があり、詳細は §2.4 に記録する。

### 1.3 Follow-up 項目 (M1/M2 対応、M0 blocker では無い)

1. **taffy `Style: !Send`** under `calc` feature — raikiri-dom の LayoutBuffer
   が M1 で「calc pointer は同一 buffer 内に留まる」 invariant を documented
   `unsafe impl Send` で表現するか、no-calc-across-thread 制約で表現するかを
   選択(§5.1)。
2. **anyrender_vello_cpu rayon 1v4 targeted test** — 本 spike は default
   multi-thread config での byte-identical を確認済。thread count を pin した
   比較は `vello_cpu` を直接 dev-dep に加えるか upstream に thread-count knob を
   足すかの二択で M1 実施(§5.2)。
3. **raikiri-feasibility crate の disposal** — M1 開始時に `crates/raikiri-
   feasibility/` を削除 or `examples/` へ移動する(§5.3)。

---

## 2. Per-spike findings

各 subsection は次の構造で記述する:
- **Summary**: 1 文で verdict と outcome
- **Verdict**: OK / NEEDS_DESIGN_CHANGE / NEEDS_INVESTIGATION / BLOCKED
- **Evidence**: commit hash、テスト名、file path
- **API surface (M1 material)**: §3 に集約する型定義への引用
- **Next actions**: M1 実装時に検討する事項

### 2.1 nzv.5 parley FontContext Send + Sync 適合性

**Summary**: parley 0.10 の `FontContext` および `LayoutContext<T>` は静的に
Send + Sync が保証される。設計仕様書 §12.8 T1 の「rayon thread count 1–16
baseline」前提が型レベルで unblock。

**Verdict**: OK

**Evidence**:
- Commit: `0fecca5` — `spike(parley_send_sync): parley FontContext Send + Sync (raikiri-spike-nzv.5)`
- File: `crates/raikiri-feasibility/src/parley_send_sync.rs`
- 静的 assert (compile-time):
  ```rust
  const fn assert_send_sync<T: Send + Sync>() {}
  const _ASSERT_FONT_CONTEXT_SEND_SYNC: () =
      assert_send_sync::<parley::FontContext>();
  const _ASSERT_LAYOUT_CONTEXT_SEND_SYNC: () =
      assert_send_sync::<parley::LayoutContext<[u8; 4]>>();
  const _ASSERT_LAYOUT_CONTEXT_PENIKO_SEND_SYNC: () =
      assert_send_sync::<parley::LayoutContext<peniko::Brush>>();
  ```
- Runtime test: `parley_send_sync::tests::font_context_and_layout_context_are_send_sync`
  が `parley::FontContext::new()` を `std::thread::spawn` で thread 境界を越えて
  join させて挙動を追検証。1 test passed.

**API surface (M1 material)**:
- `parley::FontContext: Send + Sync` — `Arc<Mutex<FontContext>>` 共有パターン、
  per-worker clone パターン、いずれも type 上は成立。選択は M1 の contention
  benchmark 次第。

**Next actions**:
- M1 raikiri-paint / raikiri-dom で FontContext の共有戦略を確定する際、
  「thread-safety は type level で確定済み、選択は perf 由来」と扱えばよい。

---

### 2.2 nzv.6 taffy Block/Flex/Grid + 並列可能性

**Summary**: taffy 0.12 は Block/Flex/Grid の 3 layout mode で non-degenerate
size を計算可能、独立 tree を 4-way rayon parallel で並列 layout 可能。ただし
`calc` feature 有効時 `Style: !Send` になる制約を発見。

**Verdict**: OK(caveat あり、M1 対応)

**Evidence**:
- Commit: `c7dc2ff` — `spike(taffy_layout_modes): taffy block/flex/grid + column-count parallel (raikiri-spike-nzv.6)`
- File: `crates/raikiri-feasibility/src/taffy_layout_modes.rs`
- 検証内容:
  1. Workspace の taffy features は `[block_layout, flexbox, grid, content_size, calc, std]`
     で `taffy_tree` feature は OFF(stylo_taffy / blitz-dom と一致)。従って
     `taffy::TaffyTree` は不可、`LayoutPartialTree` trait 群を自前 tree に
     実装する必要がある — 設計仕様書 §4 の raikiri-dom 期待と整合。
  2. Spike は `SpikeTree` (`Vec<Node>` arena、`NodeId=usize`) を実装、
     `TraversePartialTree` / `CacheTree` / `LayoutPartialTree` +
     `LayoutBlockContainer` / `LayoutFlexboxContainer` / `LayoutGridContainer`
     の 6 trait を impl。3 mode dispatch:
     ```rust
     match display {
         Display::Block => compute_block_layout(tree, node_id, inputs, None),
         Display::Flex  => compute_flexbox_layout(tree, node_id, inputs),
         Display::Grid  => compute_grid_layout(tree, node_id, inputs),
         Display::None  => LayoutOutput::HIDDEN,
     }
     ```
  3. **重要な発見**: `calc` feature 有効時、`Dimension` (via `CompactLength`)
     が `*const ()` opaque calc-arena pointer を保持しうる → `Style: !Send`
     (auto-trait 由来)。Spike は `unsafe impl Send for SpikeTree {}` を
     `#[allow(unsafe_code)]` + discipline (`Dimension::length` のみ使用、
     calc pointer を持たない invariant) で正当化。
- Runtime tests (2 passed):
  - `taffy_layout_modes::tests::all_three_display_modes_produce_non_degenerate_size`
  - `taffy_layout_modes::tests::independent_trees_lay_out_in_parallel`
    (N=4, Block/Flex/Grid を rotating)

**API surface (M1 material)**:
- raikiri-dom は `LayoutBuffer` (仮称) を独自 arena として実装し、
  `LayoutPartialTree` + 3 コンテナ trait を impl する。SpikeTree の shape が
  そのまま起草の雛形になる。
- `LayoutBuffer` の Send 表明方法は 2 選択肢:
  - **(A)** `unsafe impl Send for LayoutBuffer {}` を documented invariant で
    正当化(「calc pointers live inside the same buffer」)
  - **(B)** No-calc-across-thread invariant(calc を含む subtree は同一
    thread 内でのみ layout)

**Next actions**:
- M1 raikiri-dom task で LayoutBuffer 型 API を確定する前に、(A) / (B) の
  選択と根拠を design doc §4 に追記する。M2 column-count shard-per-column の
  parallel pass 前に確定させる必要がある。

---

### 2.3 nzv.7 anyrender_vello_cpu byte-identical raster

**Summary**: anyrender_vello_cpu 0.14 (multithreading feature on) は同一
platform で byte-identical raster 出力を提供。設計仕様書 §12.8 の VRT/T1
pixel-exact baseline 前提が unblock。

**Verdict**: OK(rayon 1v4 targeted test は M1 defer)

**Evidence**:
- Commit: `6a932d9` — `spike(anyrender_byte_identical): anyrender_vello_cpu byte-identical raster (raikiri-spike-nzv.7)`
- File: `crates/raikiri-feasibility/src/anyrender_byte_identical.rs`
- 4 runtime tests (all passed):
  - `render_produces_expected_buffer_shape` — 100×100 canvas, 中央 pixel が
    transparent-black でないことで rasterize 動作を確認
  - `same_renderer_reset_between_renders_is_byte_identical` — 単一
    `VelloCpuImageRenderer`、render → `reset()` → render で出力バイト一致
  - `two_independent_renderers_are_byte_identical` — 独立に構築した 2 つの
    renderer で 100×100 solid red rect が byte-identical(VRT/T1 が根拠にする
    cross-invocation property)
  - `tiny_skia_reference_is_byte_identical` — tiny-skia::Pixmap を reference
    として同 scene を鎖状 sanity anchor
- Scene: 100×100 canvas、`Rect(10,10,90,90)` を `css::RED` で fill、text/font
  無し(rasterizer に対する最小 deterministic 入力)

**API surface (M1 material)**:
- `anyrender::ImageRenderer` trait は raikiri-vrt が VRT harness として直接
  driving する形で M0 seed は不要(anyrender_vello_cpu が既に impl 提供)。
- **注意**: `anyrender_vello_cpu` / `tiny-skia` / `insta` は
  `raikiri-feasibility/Cargo.toml` で `[dev-dependencies]` に配置。spike
  module 内では `#[cfg(test)]` gate で import する必要がある — non-test の
  `use anyrender_vello_cpu::...` は `cargo check --all-targets` で失敗する。
  M1 raikiri-vrt が `[dependencies]` に格上げする際は shim なしで参照可能。

**Next actions**:
- **rayon 1v4 targeted test 追加** (M1):
  - **選択肢 (a)**: `raikiri-feasibility` (もしくは M1 の raikiri-vrt) に
    `vello_cpu` を直接 dev-dep に追加、`RenderSettings::num_threads` で thread
    count を pin して比較。
  - **選択肢 (b)**: `anyrender_vello_cpu` に `VelloCpuImageRenderer::new_with_threads`
    shim を upstream PR で足す。
  - どちらも M1 kickoff の blocker では無い(default multi-thread config で
    既に byte-identical を確認済)。

---

### 2.4 nzv.8 selectors standalone (stylo 非依存) 使用可能性

**Summary**: selectors 0.39 は stylo 非依存で使用可能。初回 spike で
`precomputed_hash::PrecomputedHash` trait bound の抜けを発見したため一度
NEEDS_DESIGN_CHANGE と判定したが、follow-up で `precomputed-hash` を
raikiri-style crate (stylo-equivalent) 内に encapsulate することで OK に格上げ。

**Verdict**: OK(NEEDS_DESIGN_CHANGE → OK の遷移。詳細下記)

**Verdict 遷移の記録**:

1. **初回 spike** (commit `9effd83`): raikiri-feasibility 内で `SelectorImpl`
   を実装しようとしたところ、`Identifier` / `LocalName` / `NamespaceUrl` が
   `precomputed_hash::PrecomputedHash` を trait bound として要求。
   `precomputed-hash` は `selectors` の transitive dep だが re-export されて
   おらず、また `[workspace.dependencies]` にも未登録。
   full impl を `#[cfg(any())]` gate で退避し verdict = NEEDS_DESIGN_CHANGE。
2. **Follow-up** (commits `4a5d2c6` + `bd04140`): 「`precomputed-hash` は
   stylo-side implementation detail (SelectorImpl の内部要求)、raikiri-style
   crate が raikiri architecture における stylo-equivalent」という設計判断に
   基づき、`precomputed-hash = "0.1"` を **raikiri-style の直接 dep のみ** に
   追加、`[workspace.dependencies]` には追加しない encapsulation を採用。
   raikiri-style に `RaikiriSelectorImpl` seed を実装、raikiri-feasibility の
   spike は raikiri-style の public API 経由で `.btn:hover` を parse する形に
   refactor。verdict = OK に確定。

**Evidence**:
- Commits (order):
  - `9effd83` (original NEEDS_DESIGN_CHANGE)
  - `4a5d2c6` — `feat(raikiri-style): seed SelectorImpl with encapsulated precomputed-hash`
  - `bd04140` — `spike(selectors_standalone): switch to raikiri_style::parse_selector_list (nzv.8)`
- Files:
  - `crates/raikiri-style/src/lib.rs` (M0 seed、以降 M1 で拡張)
  - `crates/raikiri-style/Cargo.toml` (`precomputed-hash = "0.1"` に crate-local
    dep コメント付き)
  - `crates/raikiri-feasibility/src/selectors_standalone.rs` (refactor 後 ~75 行)
- Runtime tests (all passed):
  - raikiri-style: `atom_precomputed_hash_is_deterministic`,
    `atom_tocss_roundtrip`, `parse_dot_btn_hover_roundtrip` (round-trip 完全一致),
    `parse_multi_selector_list` (4 passed)
  - raikiri-feasibility: `selectors_standalone::tests::{btn_hover_roundtrips_via_raikiri_style, multi_selector_list_parses, unknown_pseudo_class_is_reported_as_error}` (3 passed)

**Encapsulation rationale**:
- workspace root Cargo.toml を「対外的な dep surface」として stylo-side
  detail を漏らさない。
- Memory: `raikiri-implementation-independence` の「Stylo は blitz から見える
  surface のみ参照」原則の dependency graph 側での表現。他 raikiri crate は
  precomputed-hash に触れられない ─ 意図せず stylo-shaped internal に到達する
  経路を dependency 段階で塞ぐ。

**API surface (M1 material)** — §3 で raikiri-style seed を集約。

**Next actions**:
- **完了**(このタスク自体で修正済)。M1 では raikiri-style の seed を拡張し、
  実 cascade / matching / rule tree を実装する。SelectorImpl の associated
  type は既に確定しているので M1 での書き下ろし作業は逐次的にできる。

---

### 2.5 nzv.9 cssparser @page / @counter-style / @font-face 拡張機構

**Summary**: cssparser 0.37 の `AtRuleParser` trait は @page / @counter-style /
@font-face の 3 種 custom at-rule dispatch をクリーンにサポート。設計仕様書
§4 CSS pipeline の paged media 中核が feasibility 確認済。

**Verdict**: OK

**Evidence**:
- Commit: `04821b4` — `spike(cssparser_at_rules): cssparser custom at-rules (raikiri-spike-nzv.9)`
- File: `crates/raikiri-feasibility/src/cssparser_at_rules.rs`
- Trait shape (cssparser 0.37):
  ```rust
  pub trait AtRuleParser<'i> {
      type Prelude;
      type AtRule;
      type Error: 'i;
      fn parse_prelude<'t>(&mut self, name: CowRcStr<'i>,
          input: &mut Parser<'i, 't>)
          -> Result<Self::Prelude, ParseError<'i, Self::Error>>;
      fn parse_block<'t>(&mut self, prelude: Self::Prelude,
          start: &ParserState, input: &mut Parser<'i, 't>)
          -> Result<Self::AtRule, ParseError<'i, Self::Error>>;
      fn rule_without_block(&mut self, prelude, start)
          -> Result<Self::AtRule, ()>;
  }
  ```
- Spike の SpikeParser は `parse_prelude` で `name.eq_ignore_ascii_case("page")` /
  `"counter-style"` / `"font-face"` に対して `AtRuleKind` enum を返し、
  `parse_block` で `input.slice_from(input.position())` により body slice を
  capture(discard も可)。
- Runtime tests (2 passed):
  - `dispatches_all_three_at_rules` — `@page {} @counter-style thumbs {...}
    @font-face {...}` を parse し 3 rule が source 順で `AtRuleKind::Page` /
    `CounterStyle` / `FontFace` に dispatch されることを確認
  - `unknown_at_rule_is_skipped` — `Err(input.new_custom_error(SpikeError::UnknownAtRule))`
    を parse_prelude で返せば `@media` などが cleanly skip され、後続 `@page`
    に影響しないことを確認

**API surface (M1 material)**:
- M1 raikiri-style の RuleTree parser は SpikeParser pattern を踏襲。
  `SpikeError` → 実 `CssParseError` に置き換え、`AtRuleKind` を実 rule 型に
  拡張。@page inner grammar (page-margin box) と @counter-style descriptor
  list は M1〜M2 で個別に足す。

**Next actions**:
- M1 raikiri-style で本 spike の shape を promote。QualifiedRuleParser 側は
  設計仕様書 §4.2 の unified RuleTree を建てる際に併せて実装。

---

### 2.6 nzv.10 selectors + cssparser 版整合 (API-level)

**Summary**: selectors 0.39 と cssparser 0.37 は API surface でも共通の
`cssparser::Parser<'i,'t>` / `ParserInput<'i>` を共有し、単一 parser instance
で selector 部と declaration block 部の両方を drive できる。nzv.4 の
"single-version resolution" 結論が API-level にも拡張される。

**Verdict**: OK

**Evidence**:
- Commit: `591f460` — `spike(selectors_cssparser_compat): selectors + cssparser compat (API-level) (raikiri-spike-nzv.10)`
- File: `crates/raikiri-feasibility/src/selectors_cssparser_compat.rs`
- Type-level 共有証明:
  ```rust
  fn _selectors_uses_cssparser_parser<Impl, P>()
  where
      Impl: SelectorImpl,
      for<'i> P: SelectorParser<'i, Impl = Impl>,
  {
      let _fp = SelectorList::<Impl>::parse::<P>;
      let _pr: ParseRelative = ParseRelative::No;
  }
  ```
- 単一 `cssparser::Parser` を分割駆動する pattern(runtime evidence):
  ```rust
  let mut input_store = ParserInput::new(css);
  let mut input = CssParser::new(&mut input_store);
  let prelude_start = input.position();
  input.parse_until_before(Delimiter::CurlyBracketBlock, |inner| { ... })?;
  let selector_text = input.slice_from(prelude_start).trim().to_string();
  input.expect_curly_bracket_block()?;
  input.parse_nested_block(|inner| {
      RuleBodyParser::new(inner, &mut parser).collect ... })
  ```
- Runtime tests (3 passed):
  - `parses_simple_rule_via_shared_parser`
  - `parses_multi_decl_rule_with_combinator`
  - `parses_rule_without_trailing_semicolon`

**方針変更 note** (nzv.8 follow-up との整合):

本 issue の元の `next_actions` は「(b) `precomputed-hash = "0.1"` を workspace
dep に追加」を提案していたが、nzv.8 の解決で raikiri-style 内 encapsulation を
採用したため、workspace dep 追加は **不採用**。方針変更は本 issue の comment
にも記録済(`raikiri-spike-nzv.10` の comment 参照)。

**API surface (M1 material)**:
- `parse_style_rule` の骨格(prelude 部 → `{`まで → nested block)は M1
  raikiri-style の style-rule parser のリファレンス plumbing として使える。
- 単一 `cssparser::Parser` を SelectorList::parse と RuleBodyParser の両方に
  渡す pattern を採用可能。

**Next actions**:
- M1 raikiri-style で spike の parse_style_rule shape を promote。
  verbatim な selector_text capture は `SelectorList::parse` の直接呼び出しに
  置き換える(SelectorImpl は raikiri-style seed の RaikiriSelectorImpl を使用)。

---

### 2.7 nzv.11 anyrender PaintScene adapter compile-spike

**Summary**: blitz-paint 0.3.0-beta.1 が `&mut impl anyrender::PaintScene` を
**直接** 消費するため、設計仕様書 §M0 review 4 で懸念された「bridge trait
必要」の前提は refute される。raikiri の paint 層は passthrough newtype 程度で
十分。

**Verdict**: OK(§M0 review 4 の mitigation を撤回可能)

**Evidence**:
- Commit: `43b357b` — `spike(paintscene_adapter): anyrender PaintScene adapter compile-spike (raikiri-spike-nzv.11)`
- File: `crates/raikiri-feasibility/src/paintscene_adapter.rs`
- Vendored blitz-paint 0.3.0-beta.1 の該当箇所:
  ```text
  blitz-paint/src/lib.rs:42
      pub fn paint_scene(scene: &mut impl PaintScene, doc: &mut BaseDocument, ...)
  blitz-paint/src/render.rs:113
      pub fn paint_scene(&self, scene: &mut impl PaintScene)
  blitz-paint/src/{text.rs, layers.rs, debug_overlay.rs}: 各 helper が
      &mut impl PaintScene を受ける
  ```
- Spike deliverable:
  ```rust
  pub struct RaikiriPaintAdapter<S: PaintScene> { pub inner: S }
  // anyrender::RenderContext / PaintScene を delegating impl
  const _ASSERT_ADAPTER_IS_PAINTSCENE: fn() = || {
      fn takes_paint_scene<T: PaintScene>() {}
      takes_paint_scene::<RaikiriPaintAdapter<Scene>>();
  };
  ```
- Runtime tests (3 passed):
  - `adapter_forwards_clip_layer_push_pop`
  - `adapter_forwards_fill_to_inner`
  - `adapter_satisfies_paintscene_bound`
- 発生した `unsafe`、`mem::transmute`、lifetime gymnastics: **無し**。

**API surface (M1 material)**:
- raikiri-paint は「anyrender::PaintScene に対して blitz-paint::paint_scene を
  呼ぶ」だけ。newtype adapter は instrumentation / paged-media routing 用の
  hook が必要な場合のみで、bridge trait は不要。

**Design doc implications**:
- 設計仕様書 §M0 review 4「anyrender PaintScene 適合面 (blitz-paint と
  shape 一致するか、adapter compile-spike で verify)」の懸念は解消。§4 の
  raikiri-paint 型定義は「passthrough / instrumentation wrapper」で書き下ろせる。

**Next actions**:
- M1 raikiri-paint task で「direct anyrender sink」設計を採用。設計仕様書
  §M0 review 4 の mitigation 追記(bridge trait scaffolding)を撤回する PR を
  M1 kickoff 前に投げる(§13 spec drift protocol に従う)。

---

## 3. API surface (§4 型定義 material)

M0 spike から確定した型を、設計仕様書 §4 の型定義起草時に参照する material と
して集約する。

### 3.1 raikiri-style seed types (from nzv.8)

`crates/raikiri-style/src/lib.rs` にて確定:

```rust
// Atom: SmolStr newtype with stable u32 precomputed hash.
pub struct Atom(pub SmolStr);
impl PrecomputedHash for Atom { /* FxHasher-based */ }
impl ToCss for Atom { /* via cssparser::serialize_identifier */ }

// AttrValue: string-attribute values used in [attr=...] selectors.
pub struct AttrValue(pub SmolStr);

// PseudoClass: M0 seed = { Hover, Active } のみ。M1 で拡張。
pub enum PseudoClass { Hover, Active }
impl NonTSPseudoClass for PseudoClass { /* ... */ }

// PseudoElement: M0 は空 enum(未定)。M1 で ::before / ::after 等を追加。
pub enum PseudoElem {}

// SelectorImpl: 全 associated type を確定。
pub struct RaikiriSelectorImpl;
impl SelectorImpl for RaikiriSelectorImpl {
    type ExtraMatchingData<'a> = PhantomData<&'a ()>;
    type AttrValue = AttrValue;
    type Identifier = Atom;
    type LocalName = Atom;
    type NamespaceUrl = Atom;
    type NamespacePrefix = Atom;
    type BorrowedNamespaceUrl = Atom;
    type BorrowedLocalName = Atom;
    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElem;
}

// Public parse helper.
pub fn parse_selector_list(input: &str)
    -> Result<SelectorList<RaikiriSelectorImpl>, String>;
```

### 3.2 taffy 直下 trait 実装パターン (from nzv.6)

raikiri-dom が `LayoutBuffer` (仮称) を独自 arena として実装する際、以下 6 種の
taffy trait を impl する。SpikeTree (`crates/raikiri-feasibility/src/taffy_layout_modes.rs`) の
実装が起草の雛形:

```rust
impl TraversePartialTree for LayoutBuffer { ... }
impl CacheTree for LayoutBuffer { ... }
impl LayoutPartialTree for LayoutBuffer { ... }
impl LayoutBlockContainer for LayoutBuffer { ... }
impl LayoutFlexboxContainer for LayoutBuffer { ... }
impl LayoutGridContainer for LayoutBuffer { ... }
```

Dispatch:
```rust
match display {
    Display::Block => compute_block_layout(tree, node_id, inputs, None),
    Display::Flex  => compute_flexbox_layout(tree, node_id, inputs),
    Display::Grid  => compute_grid_layout(tree, node_id, inputs),
    Display::None  => LayoutOutput::HIDDEN,
}
```

### 3.3 cssparser AtRuleParser 実装パターン (from nzv.9)

M1 raikiri-style の RuleTree parser の骨格:

```rust
pub struct RaikiriAtRuleParser { /* state */ }

impl<'i> AtRuleParser<'i> for RaikiriAtRuleParser {
    type Prelude = RaikiriAtRulePrelude;  // 実 AtRule 型に置き換え
    type AtRule  = RaikiriAtRule;
    type Error   = CssParseError;

    fn parse_prelude<'t>(&mut self, name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>) -> Result<Prelude, ParseError<...>>
    {
        if name.eq_ignore_ascii_case("page") { /* ... */ }
        else if name.eq_ignore_ascii_case("counter-style") { /* ... */ }
        else if name.eq_ignore_ascii_case("font-face") { /* ... */ }
        else { Err(input.new_custom_error(CssParseError::UnknownAtRule(name))) }
    }
    fn parse_block<'t>(...) -> Result<AtRule, ...> { /* per-rule body parser */ }
}
```

### 3.4 anyrender PaintScene 直接消費パターン (from nzv.11)

raikiri-paint の main entry:

```rust
// bridge trait を挟まず、blitz-paint と同じ shape (impl PaintScene) をそのまま採用。
pub fn paint_page(
    scene: &mut impl anyrender::PaintScene,
    document: &BaseDocument,
    page: PageIndex,
) { /* ... */ }
```

必要ならば instrumentation wrapper のみを差し込む:

```rust
pub struct Instrumented<S: PaintScene> { inner: S, /* counters, timers */ }
// PaintScene の全 method を inner に委譲、任意で pre/post hook を挿入。
```

### 3.5 encapsulated stylo-side deps (from nzv.8)

`precomputed-hash = "0.1"` は raikiri-style の直接 dep のみ。
`[workspace.dependencies]` には含めない — 設計仕様書 §4 の "external dep 一覧"
に含まれるが、workspace scope でなく **raikiri-style local scope** の但し書きを
追加する必要あり(§4 の implications へ §4.1 参照)。

---

## 4. Design doc implications

### 4.1 §12.8 T1 baseline の確認

以下 2 前提が spike で確認できた:

1. **rayon thread count 1–16 pixel-exact baseline**: parley (nzv.5) の Send +
   Sync + anyrender_vello_cpu (nzv.7) の byte-identical で unblocked。
2. **byte-identical raster on same platform**: anyrender_vello_cpu 0.14 (
   multithreading feature on) の default config で 2 independent renderer が
   byte-identical(nzv.7)。

### 4.2 §M0 review 4 の撤回

nzv.11 refute により「anyrender PaintScene と blitz-paint が別 shape で
bridge trait 必要」の懸念は誤り。設計仕様書 §M0 review 4 の mitigation は M1
kickoff 前に撤回する(§13 spec drift protocol、別 PR)。

### 4.3 §4 外部 dep 表の記述追加

現状の §4 に対する追加事項(実 spec revise は本 doc とは別 task):

1. `precomputed-hash = "0.1"` は raikiri-style の **crate-local dep**、
   workspace scope でない旨の但し書き。理由は encapsulation policy(
   §2.4 参照)。
2. taffy 0.12 の feature 一覧に対する明示: `[block_layout, flexbox, grid,
   content_size, calc, std]` (stylo_taffy / blitz-dom と一致)、`taffy_tree`
   は **OFF**(raikiri-dom が自前 arena で trait 直下実装するため)。
3. `Style: !Send` under `calc` feature の invariant 記述(§5.1 も参照)。
4. `anyrender_vello_cpu` / `tiny-skia` / `insta` は `[dev-dependencies]` 配置、
   raikiri-vrt が M1 で `[dependencies]` に格上げする際 shim 不要。

### 4.4 §13 spec drift 記録

以下は M0 期間中に発生した spec drift。§13 spec drift protocol に従い、本 doc
と別 PR で design doc §13.0 更新を提案する:

- **MSRV**: 1.85 → 1.88(nzv.3 Task 4、parley 0.10 要件、commit `edce05d`)
  → 1.89(blitz oracle dev-deps 揃え、commit `23b5343`)。設計仕様書 §13 M0 の
  「rust-toolchain.toml MSRV = 1.85」記述は現状と乖離。
- **nzv.8 verdict 遷移**: NEEDS_DESIGN_CHANGE → OK(encapsulation)。
- **raikiri-style の M0 seed 化**: 元は「M0 stub、M1 で populate」の予定
  だったが、nzv.8 fix で SelectorImpl seed を M0 内に追加(§2.4)。

---

## 5. Deferred / follow-up items (M1/M2)

M0 blocker では無いが M1 kickoff 直後〜M2 前半で対応が必要な項目。

### 5.1 taffy Style: !Send under calc feature の LayoutBuffer 設計判断

- **What**: `Dimension` (via `CompactLength`) が `calc()` 有効時に `*const ()`
  を持ちうるため `Style: !Send`。raikiri-dom の LayoutBuffer が独自 arena で
  `Style` を含む場合、そのまま `!Send`。
- **Options**:
  - **(A) documented `unsafe impl Send`**: 「calc pointers live inside the
    same LayoutBuffer」invariant を型 doc に記載、`#[allow(unsafe_code)]` +
    justify comment。M1 raikiri-dom crate lint の workspace-wide
    `unsafe_code = "deny"` に対して局所 opt-in。
  - **(B) no-calc-across-thread invariant**: calc を含む subtree は同一
    thread 内でのみ layout。arena 分割前提のため実装的に不自然になる恐れあり。
- **Owner**: M1 raikiri-dom task 実装者
- **Deadline**: M2 column-count shard-per-column の parallel pass 実装前

### 5.2 anyrender_vello_cpu rayon 1v4 targeted test

- **What**: 本 spike は default multi-thread config での byte-identical 確認
  のみ。thread count を pin(1 vs 4)した比較は未実施。
- **Options**:
  - **(a)** `vello_cpu` を直接 dev-dep に追加、`RenderSettings::num_threads` で
    thread count を pin して比較 test を書く(raikiri-feasibility 又は M1 の
    raikiri-vrt)。
  - **(b)** `anyrender_vello_cpu` に `VelloCpuImageRenderer::new_with_threads`
    shim を upstream PR で足す。downstream VRT 実装が thread pin できる knob
    を持てば shim なしで対応可能。
- **Owner**: M1 raikiri-vrt task 実装者
- **Deadline**: M1 mid(VRT infrastructure 立ち上げ時期)

### 5.3 raikiri-feasibility crate の disposal

- **What**: `crates/raikiri-feasibility/` は §M0「Disposable artifacts」
  分類。M1 開始時に削除 or `examples/` へ移動。
- **注意**: nzv.8 follow-up で SelectorImpl seed は raikiri-style に移設済
  なので、raikiri-feasibility 側の `selectors_standalone.rs` を含む 7 module は
  すべて disposable。ただし個別 spike の evidence (test code) は本 doc §2 で
  引用しているため、削除時は本 doc の commit hash 参照が git 履歴に到達可能
  であることを維持する。
- **Owner**: M1 kickoff の最初の PR
- **Deadline**: M1 Task 0(scaffold cleanup)

### 5.4 MSRV / spec drift 追記 (§13)

§4.4 参照。§13 spec drift protocol に従い、M0 期間中の MSRV bump 履歴 +
nzv.8 verdict 遷移 + raikiri-style seed 追加 を design doc §13.0 に反映する
PR を M1 kickoff 前に用意。

---

## 6. M0 outcome verdict

### 6.1 分岐判定

設計仕様書 §M0「M0 outcome の 2 分岐」に照らして本 doc が提示する材料:

- 全 7 検証項目 (nzv.5–.11) が **OK** 判定。
- NEEDS_DESIGN_CHANGE は nzv.8 で一度発生したが、encapsulation 修正で OK に
  格上げ。M0 期間内に解消済。
- BLOCKED / NEEDS_INVESTIGATION は無し。

→ **分岐 1**: 全 OK → M0 epic close、M1 unblock。

### 6.2 判定分業と次段(nzv.13 m0-readiness-gate)

本 doc は M0 outcome の material 提供までを scope とする。formal な judgment
と `raikiri-spike-m0` epic の close、M1 unblock signal の発行は
`raikiri-spike-nzv.13` (m0-readiness-gate) が担当する。

nzv.13 は本 doc §6.1 の分岐判定を評価し、以下いずれかを実行:

- 分岐 1(本 doc の判定): `raikiri-spike-m0` epic を close、M1 unblock。
- 分岐 2(NEEDS_DESIGN_CHANGE 発生時): 別 epic `raikiri-spike-m0-revision` を
  起動、design doc revise + roborev 再レビュー完了後に M1 unblock。

**本 doc が分岐 1 を提示する以上、nzv.13 の判定は「分岐 1、M1 unblock」に
なる想定**。ただし §5 の follow-up 項目 (§5.1–5.4) を nzv.13 の scope に
含めるかどうかの整理は nzv.13 側で決定する。

### 6.3 M0 → M1 遷移時の handoff 事項

nzv.13 が M0 close を決めた後、M1 kickoff PR で以下を先行対応する想定:

- §5.3: raikiri-feasibility crate の disposal
- §4.2: 設計仕様書 §M0 review 4 の mitigation 撤回
- §4.3: 設計仕様書 §4 外部 dep 表への追記
- §5.4: 設計仕様書 §13.0 の spec drift 記録
- §5.1: raikiri-dom LayoutBuffer 設計判断(M1 Task 序盤)
- §5.2: raikiri-vrt thread count pin test(M1 mid)

---

## Appendix A. Verification commands

M0 期間中に spike が使った cargo command 一覧(workspace 全体で `cargo check
--workspace --all-targets` が pass、`cargo test -p raikiri-feasibility` は 各
module ごとに個別 pass):

```bash
# nzv.4 - dependency resolution
cargo metadata --frozen --format-version=1     # exit 0
cargo tree --workspace --edges normal -d       # duplicates: bitflags/png 系のみ
cargo tree -p selectors -p cssparser --edges normal  # single-version confirm
cargo check --workspace --all-targets          # exit 0

# 各 spike (raikiri-feasibility/src/<module>.rs)
cargo check -p raikiri-feasibility --all-targets
cargo test  -p raikiri-feasibility --lib parley_send_sync::           # nzv.5, 1 passed
cargo test  -p raikiri-feasibility --lib taffy_layout_modes::          # nzv.6, 2 passed
cargo test  -p raikiri-feasibility --lib anyrender_byte_identical::    # nzv.7, 4 passed
cargo test  -p raikiri-feasibility --lib selectors_standalone::        # nzv.8, 3 passed (最終形)
cargo test  -p raikiri-feasibility --lib cssparser_at_rules::          # nzv.9, 2 passed
cargo test  -p raikiri-feasibility --lib selectors_cssparser_compat::  # nzv.10, 3 passed
cargo test  -p raikiri-feasibility --lib paintscene_adapter::          # nzv.11, 3 passed

# raikiri-style seed (nzv.8 follow-up で追加)
cargo test  -p raikiri-style --lib             # 4 passed
```

---

## Appendix B. Cargo.lock snapshot / dep versions

### B.1 Toolchain

- `rustc 1.89.0 (29483883e 2025-08-04)`
- `cargo 1.89.0 (c24e10642 2025-06-23)`
- `rust-toolchain.toml` channel = `"1.89.0"`, profile = minimal, components =
  `[rustfmt, clippy]`

### B.2 主要 external deps (from `[workspace.dependencies]`)

| Crate | Version | 備考 |
|---|---|---|
| cssparser | 0.37 | Servo family |
| selectors | 0.39 | Servo family、内部 dep として cssparser 0.37 |
| html5ever | 0.39 | Servo family |
| markup5ever | 0.39 | Servo family |
| taffy | 0.12 | features = `[block_layout, flexbox, grid, content_size, calc, std]`、`taffy_tree` は OFF |
| parley | 0.10 | features = `[std]`(no default) |
| anyrender | 0.11 | Linebender family |
| peniko | 0.6 | Linebender family |
| kurbo | 0.13 | Linebender family |
| smol_str | 0.3 | |
| rustc-hash | 2.1 | |
| bytes | 1 | |
| url | 2.5 | |
| anyrender_vello_cpu | 0.14 | dev-only、features = `[multithreading]` |
| tiny-skia | 0.11 | dev-only |
| insta | 1 | dev-only |
| blitz-{traits,html,dom,paint} | `=0.3.0-beta.1` | dev-only oracle |

### B.3 raikiri-style crate-local dep (encapsulated)

| Crate | Version | Location |
|---|---|---|
| precomputed-hash | 0.1 | `crates/raikiri-style/Cargo.toml` の直接 dep のみ |

理由: §2.4 参照。`[workspace.dependencies]` には置かない。

### B.4 Cargo.lock frozen state

- 初期 freeze (nzv.4): SHA-256
  `9665c35a8f61d6cf7c310ab9d77878c9ae79d1c677f4bbafed42b35bd336b123`
  (workspace root、69,226 bytes、commit `c6a7552`)。
- 更新履歴:
  - `88d63746` — raikiri-feasibility crate scaffold 追加(nzv.5–.11 workflow
    setup)、Cargo.lock は追加 dep 分だけ増加。
  - `bd04140` — raikiri-style に precomputed-hash / smol_str / rustc-hash 追加
    + raikiri-feasibility に raikiri-style path dep 追加。Cargo.lock は 4 行
    増加のみ(外部 crate version 変更無し)。
- Duplicates in `cargo tree -d`(nzv.4 verify で記録):
  - `bitflags v1.3.2` / `v2.13.0`(dev-only chain: tiny-skia vs vello_common
    経由)
  - `png v0.17.16` / `v0.18.1`(同上)
  - production crate DAG (raikiri-{traits,style,net,dom,html,paint,blitz-compat})
    単体では duplicate 無し。M0 spike 対象外の transient として合意済。
