---
title: raikiri rebuild — Streaming HTML/CSS Rendering Engine for fulgur — 設計
status: Draft
date: 2026-07-13
author: Mitsuru Hayasaka (@mitsuru)
related:
  - "raikiri 既存設計: `raikiri/docs/design/2026-07-10-streaming-layout-engine-design.md`"
  - "fulgur `crates/fulgur/src/blitz_adapter.rs` (7,114 行の隔離ファサード)"
  - "blitz upstream (DioxusLabs): 現行の依存元"
---

# raikiri rebuild — Streaming HTML/CSS Rendering Engine for fulgur — 設計

## 1. 背景

fulgur (サーバサイド HTML/CSS → PDF 変換、wkhtmltopdf 代替) は現在
`blitz-html` / `blitz-dom` / `blitz-traits` に依存しており、fulgur 側
`blitz_adapter.rs` (7,114 行) で隔離している。

既存の raikiri は「fulgur を print-first の pipeline に組み替えるための
streaming layout engine」として設計され、M5 (2026-07-12) まで実装が進んで
いる (16,000+ 行、`raikiri-html` / `raikiri-dom` / `raikiri` / `raikiri-wpt` /
`raikiri-vrt`)。blitz は browser (screen-first) 前提の設計で、paged media 中心
の fulgur とは方向性が根本的に異なる。

しかし M5 進行中に以下の設計上の行き詰まりが顕在化した:

1. **stylo と自前 GCPM parser の 2 パス CSS 問題** — stylo が @page / counter /
   string-set / running / target-* を `Ignored` で握り潰すため、fulgur/raikiri は
   同じ stylesheet を cssparser で舐め直す。cascade order・specificity・source
   map が 2 系統で分断
2. **5 stage pipeline の境界と実装の乖離** — stage 3 cascade と stage 4 box tree、
   stage 5 PageStream の境界が実装で守れず、GCPM state (counter/string/running)
   が暗黙 mutation で複数 stage を跨ぐ
3. **DocumentPass / 非同期 resolver のセキュリティ層が重すぎる** —
   sanitize-before-resolve の 4 sub-stage (Stage 1 → 1.5 → 1.75 → 5) 分割が
   pipeline を複雑化。attribute-level sanitize bypass 問題も未解決
4. **PageStream × break policy × widows-orphans の合成が困難** — flat state
   machine で 3 者を絡めた結果、テスト容易性と拡張性が失われた
5. **GCPM state 伝搬 (counter / string-set / named string) が場当たり的** —
   cascade → layout → PageStream で状態が暗黙に流れ、bug 特定困難
6. **target-* の Form XObject slot emit 方式が Consumer 依存すぎ** — fulgur+flpdf
   前提の PDF 特化設計で、他 Consumer への使い回しが効かない

本 spec は、既存 raikiri の **設計哲学 (static / streaming / composable) と
WPT-first 検証方針を継承しつつ、ゼロから作り直す** ための設計を規定する。

作業場所: `raikiri-spike/` (独立 repository)。完成後 raikiri 本体への統合を判断。

## 2. Design Goals / Non-Goals

### Goals

- **fulgur を print-first pipeline に組み替える** — screen-first 前提の
  blitz-* に代わる HTML/CSS rendering backend を提供
- **static / streaming / pipeline-composable の 3 原則を継承**
- **CSS を single-parse し、GCPM を first-class で扱う** (2 パス問題の根絶)
- **streaming first-class**: 出力 API は原則 `PageStream` (per-page emit)
- **2 pass batch mode を first-class に共存させる**: 小〜中規模ドキュメントの
  フル spec compliance (target-* 正確化、`counters()` nested scope、backward
  selector、unlimited widow/orphan lookahead)
- **per-page で PageBox が変わる混合サイズ PDF を native 対応**
- **同期 ReplacedResolver**: raikiri コアに async runtime を持ち込まない
- **WPT 準拠を第一級目標**: blitz baseline oracle として活用、blitz が pass
  する全 test を pass しつつ、blitz の spec drop 箇所を native fix
- **byte-identical layout output** (決定論)

### Non-Goals

- 全 CSS 仕様の網羅 — interactive 系 (`:hover`, `:focus`, animation, transition)
  は明示的に非対応
- HTML5 form controls の interactive behavior (`<input>` は静的表示のみ)
- Post-layout mutation / runtime DOM API (再パースで対応)
- JavaScript 実行
- Accessibility tree の**構築** — hint (`bookmark_hints`, `heading_structure`,
  `structural_hints`) を渡すのみ、structure tree の組立ては Consumer 側
- 縦書き / ルビ / JIS X 4051 相当の日本語組版 (MVP は横書き、型体系は
  writing-mode / ruby 拡張可能に閉じ込める)
- Templating (fulgur が MiniJinja を持つ、Consumer 責任)
- PDF 出力 (fulgur が krilla で直接処理、raikiri は `PageFragment` を渡すのみ)

## 3. 依存 crate の選定

### 継承する dep

- `html5ever` (HTML parse)
- `cssparser` (CSS tokenization)
- `selectors` (Servo 抽出、selector matching)
- `taffy` (block / flex / grid layout)
- `parley` (text layout / shaping)
- `anyrender` + `peniko` + `kurbo` (2D drawing abstraction、blitz と共通)

### 削除する dep

- `stylo` — 非公開 API 問題、cache infra デッドウェイト、@page 未対応、動的挙動
  前提の SharedRwLock。cascade / ComputedValues / 型体系を自前化する
- `blitz-*` (`blitz-html`, `blitz-dom`, `blitz-traits`) — screen-first 前提の
  設計で paged media 中心の fulgur とは方向性が異なるため、print-first の
  backend を別途用意する

### dev-only dep

- `anyrender_vello_cpu` (VRT ラスタライザ、multithreading feature)
- `tiny-skia` (PNG diff、fallback ラスタライザ)
- `insta` (snapshot testing)
- `blitz-html` / `blitz-dom` (oracle として比較用)

## 4. Workspace Layout

blitz workspace 構成を参考に、責務を明確に分離した 12 crate 構成。

```
raikiri-spike/
├── Cargo.toml (workspace)
├── crates/
│   ├── raikiri-traits/       # 共有 trait 定義
│   │                            RenderSink, NetworkProvider, ReplacedResolver,
│   │                            Dom/Element/Node, RenderMode, LookaheadConfig
│   ├── raikiri-html/         # html5ever wrapper + RaikiriTreeSink
│   │                            Consumer が wrap して sanitize/inject/rewrite
│   ├── raikiri-css/          # cssparser + selectors + 統一 RuleTree
│   │                            + cascade + ComputedValues (stylo リプレース領域)
│   ├── raikiri-dom/          # DOM データモデル (Node/Element/Attribute)
│   ├── raikiri-gcpm/         # @page/counter/string-set/running/target-* IR
│   │                            + PageContext + TargetRegistry
│   ├── raikiri-layout/       # taffy + parley + LayoutBuffer + PageStream (private)
│   │                            + render() free function
│   ├── raikiri-paint/        # PageFragment → anyrender::PaintScene walker
│   │                            (VRT や debug output 用途)
│   ├── raikiri-net/          # NoOpProvider (default) / SandboxedProvider (future)
│   ├── raikiri/              # umbrella crate: primary Consumer API
│   │                            全 sub-crate re-export + html_to_png helper
│   ├── raikiri-blitz-compat/ # blitz 互換 layer (M6 で追加)
│   │                            fulgur migration 支援、型 shape 互換
│   ├── raikiri-wpt/          # dev-only: WPT runner + blitz oracle
│   └── raikiri-vrt/          # dev-only: anyrender_vello_cpu wrapper + PNG diff
├── wpt/                       # W3C web-platform-tests (submodule)
├── expectations/              # blitz-baseline.txt + raikiri-current.txt
├── docs/superpowers/specs/    # 本 doc を含む
└── examples/
```

### 依存 DAG

```
raikiri-traits (foundation)
   ▲
   │
   ├── raikiri-html ─┐
   ├── raikiri-css ──┼──▶ raikiri-dom ──▶ raikiri-gcpm ──▶ raikiri-layout ──▶ raikiri-paint
   ├── raikiri-net ──┘                                            │              │
   └─────────────────────────────────────────────────────────────┼──────────────┘
                                                                  │
                                                          raikiri (umbrella)
                                                                  ▲
                                                                  │
                                                     raikiri-blitz-compat
                                                                  │
                                                     raikiri-vrt (dev)
                                                     raikiri-wpt (dev)
```

相互依存なし。DAG。

### crate 責務詳細

#### `raikiri-traits`

foundation crate。他 crate が共有する trait/型定義を集約。実装は持たず、他 crate
への依存も最小。

```rust
pub trait RenderSink: Send {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()>;
    fn finalize(self: Box<Self>) -> std::io::Result<()>;
}

pub trait ReplacedResolver {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<IntrinsicBox, ResolverError>;
}

pub trait NetworkProvider {
    fn fetch(&self, url: &Url) -> Result<Bytes, NetworkError>;
}

pub enum RenderMode {
    Streaming { lookahead: LookaheadConfig },
    Batch { max_document_pages: Option<u32>, target_convergence: TargetConvergence },
}

pub struct LookaheadConfig {
    pub widow_line_buffer: usize,
    pub orphan_line_buffer: usize,
    pub break_avoid_max_subtree_blocks: usize,
    pub target_mode: TargetMode,
    pub allow_cross_size_lookahead: bool,
}

pub enum TargetConvergence {
    TwoPass,
    NPassConverge { max_iterations: u32 },
}

pub trait Dom { /* ... */ }
pub trait Element<'a> { /* ... */ }
pub trait Node<'a> { /* ... */ }
```

**`DocumentPass` / `DomTransform` は含めない** — Consumer は
`RaikiriTreeSink` を wrap する pattern を使用 (5.2 節参照)。

#### `raikiri-html`

html5ever wrapper。`RaikiriTreeSink` が Dom を構築。Consumer は
`html5ever::tree_builder::TreeSink` として wrap 可能 (sanitize / inject / rewrite)。

エントリポイントの `Document` は薄い wrapper。

```rust
pub fn parse_html<R: std::io::Read>(input: R, options: &ParseOptions) 
    -> Result<Document, ParseError>;

pub fn parse_html_with_sink<R, S>(input: R, sink: S, options: &ParseOptions) 
    -> Result<Document, ParseError>
where R: std::io::Read, S: html5ever::tree_builder::TreeSink;

pub fn iter_replaced_elements(doc: &Document) 
    -> impl Iterator<Item = ReplacedElementRef<'_>>;

pub struct ParseOptions<'a> {
    pub extra_stylesheets: &'a [&'a str],
    pub network: Option<&'a dyn NetworkProvider>,
    pub base_url: Option<Url>,
}
```

#### `raikiri-css`

**stylo リプレース領域**。cssparser + selectors を base に、cascade /
specificity / ComputedValues 型体系を自前実装。

- `cssparser` の 1 pass で **通常 rule と @page 系 at-rule を統一 RuleTree に集約**
- selector matching は `selectors` crate に委ねる
- ComputedValues は paged media / GCPM 対応を含む自前型体系
- `content` プロパティは `Vec<ContentValueItem>` として resolved value (実行時解決)

Selectors 対応範囲:
- L3 base、interactive 系 (`:hover`, `:focus`, `:link`, `:visited`, `:target`,
  `:enabled`, `:checked`) は parse は通すが常に false (fail-open)
- L4 streaming-safe (`:is()`, `:where()`, `:not()`) は採用、優先度低
- L4 backward-reference (`:has()`, `:nth-last-child()`, `:blank`) は Batch mode
  のみ有効化

#### `raikiri-dom`

DOM データモデル。Node / Element / Attribute の型と tree walk API。
`raikiri-traits::Dom` を実装。

#### `raikiri-gcpm`

GCPM (Generated Content for Paged Media) の IR 明示ノード化と PageContext 管理。

- `GcpmDirective` enum で counter-increment/reset/set、string-set、
  register-running、register-target を明示化
- `PageContext` は Phase B が所有する明示 mutable state
- `TargetRegistry` で target-* placeholder emit と resolve を管理
- @page rule cascade は raikiri-css と協調 (statelessness を維持)

#### `raikiri-layout`

taffy + parley 統合。**Phase B の主体**。

- `LayoutBuffer`: widow/orphan/break-inside lookahead の独立ユニット
- `PageStream`: crate-private state machine (per-page emit)
- `render()` free function: Consumer からの driver
- Per-page 内部の rayon 並列化 (16 margin box slots、paragraph text shape、
  multi-column)

```rust
pub fn render(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    mode: RenderMode,
    sink: &mut dyn RenderSink,
) -> std::io::Result<()>;
```

#### `raikiri-paint`

`PageFragment` → `anyrender::PaintScene` の walker。VRT や debug output 用途。

- Consumer が独自 walker を書く場合 (fulgur → krilla direct) は使わない
- PDF backend は anyrender に押し込まず、fulgur が独立に `PageFragment` walk

```rust
pub fn paint_to_scene(
    page: &PageFragment,
    scene: &mut impl anyrender::PaintScene,
    scale: f64,
    options: PaintOptions,
);

pub struct PaintOptions {
    pub target_slot_rendering: TargetSlotRendering,
    pub link_visualization: LinkVisualization,
    pub debug_overlay: bool,
}
```

#### `raikiri-net`

`NetworkProvider` 実装群。

- `NoOpNetworkProvider`: fulgur デフォルト、全 fetch を reject
- `SandboxedNetworkProvider` (future): URL allowlist、size/time limit、MIME
  whitelist

#### `raikiri` (umbrella)

**primary Consumer API**。fulgur は `use raikiri::*` のみで足りる。

```rust
pub use raikiri_html::{Document, parse_html, ParseOptions, iter_replaced_elements};
pub use raikiri_layout::{render, PageDefaults};
pub use raikiri_traits::{
    RenderSink, ReplacedResolver, NetworkProvider,
    RenderMode, LookaheadConfig, TargetConvergence,
};
pub use raikiri_paint;  // sub-module として

// dogfooding helper (VRT や examples 用途)
pub fn html_to_png(html: &str) -> Result<Vec<u8>, RenderError>;
```

#### `raikiri-blitz-compat` (M6 で追加)

fulgur (および他 blitz Consumer) の raikiri migration 支援。**型 shape 互換のみ、
振る舞いは raikiri をそのまま露出** (方針 B)。

```rust
pub mod blitz_html { pub struct HtmlDocument { /* raikiri::Document を wrap */ } }
pub mod blitz_dom { pub struct DocumentConfig { /* ... */ } pub struct Node<'a> { /* ... */ } }
pub mod blitz_traits {
    pub mod net { pub trait NetProvider { /* NetworkProvider にブリッジ */ } }
    pub mod shell { pub enum ColorScheme { Light, Dark } pub struct Viewport { /* ... */ } }
}
```

blitz crate 自体には**依存しない** (バージョン地獄回避)。blitz の型 shape を
re-implement。挙動差分は `BEHAVIOR_DIFF.md` に列挙。

#### `raikiri-wpt` (dev-only)

WPT runner。W3C web-platform-tests submodule を実行、`expectations/` と比較。
blitz を dev-dep として使い、baseline oracle として "blitz pass = raikiri
必須 pass" を検証。

#### `raikiri-vrt` (dev-only)

`anyrender_vello_cpu` wrapper + PNG diff。100 行以下の thin wrapper。

```rust
pub fn rasterize_page(page: &PageFragment, dpi: f32) -> Result<Pixmap, VrtError>;
pub fn compare_png(a: &Pixmap, b: &Pixmap, tolerance: f32) -> DiffResult;
pub fn save_diff_visualization(diff: &DiffResult, path: &Path);
```

## 5. Pipeline Architecture

### 5.1 Phase A / Phase B の 2 段分割

既存 raikiri の 5 stage を、明示的な 2 phase に凝縮。境界は `Document` 型で
完全隔離、Phase B は `&Document` (read-only) で consume。

```
Phase A: Document build (batch, 純関数的)
─────────────────────────────────────────────
  html_input (impl Read)
    │
    ▼
  html5ever chunk streaming → RaikiriTreeSink (Consumer wrap 可)
    │
    ▼
  Dom
    │
    ├─▶ RuleTree: cssparser 1-pass で 通常 rule + GCPM at-rule を統一 tree に集約
    │
    ├─▶ cascade: selectors::matching + 自前 cascade
    │             出力は Vec<ComputedValues> と Vec<GcpmDirective>
    │
    └─▶ Document = { dom, computed, rule_tree, gcpm_directives, running_templates,
                     seed_page_context }
             ↑ Phase A の produce、Phase B の read-only 入力

Phase B: Page emission (streaming)
─────────────────────────────────────────────
  render(doc, defaults, resolver, mode, sink):
    for page in PageStream::new(doc, defaults, mode) {
        sink.accept_page(page?)?;
    }

  PageStream 内部:
    LayoutBuffer       (widows/orphans/break-avoid lookahead 明示型)
    PageContext        (counter tree + string 4-snapshot + running + target)
    ResolverPool       (sync resolver 呼び出し、Consumer 責任で並列化)
    DocumentCursor     (body flow の walk 位置)
    
    per page loop:
      1. cursor から次 block を buffer に取り込む
      2. taffy で block layout / parley で inline layout
      3. break policy 適用 (LayoutBuffer.decide_break())
      4. PageContext に directive 反映 (counter increment, string set, etc.)
      5. @page rule を per-page で resolve (:first / :left / :right / :nth / named)
      6. PageBox を per-page で決定 (defaults + @page override)
      7. 16 margin box slots layout (rayon 並列可能)
      8. PageFragment 組み立てて yield
      9. 確定領域の layout state を drop
```

### 5.2 Consumer 介入ポイント (TreeSink wrap)

`DocumentPass` / `DomTransform` のような独立 trait は raikiri-traits に**入れない**。
Consumer が Dom に介入したい場合は、`RaikiriTreeSink` を wrap する:

```rust
// Consumer 側
struct SanitizingSink<S> {
    inner: S,
    policy: SanitizePolicy,
}

impl<S: TreeSink> TreeSink for SanitizingSink<S> {
    fn create_element(&mut self, name: QualName, attrs: Vec<Attribute>) -> S::Handle {
        if self.policy.is_disallowed(&name) { return dummy_node(); }
        let sanitized_attrs = self.policy.rewrite_attrs(attrs);
        self.inner.create_element(name, sanitized_attrs)
    }
    // ...
}

let sink = SanitizingSink { inner: RaikiriTreeSink::new(), policy };
let doc = raikiri::parse_html_with_sink(html, sink, &options)?;
```

fulgur 現行の 6 個の `DomPass` (`CaptionRestructurePass`, `RunningElementPass`,
`StringSetPass`, `CounterPass`, `InjectCssPass`, `BookmarkPass`) は、new
raikiri では以下に集約:

- **`CaptionRestructurePass`** → raikiri-layout に統合 (native caption-side)
- **`RunningElementPass`** → raikiri-gcpm 内部
- **`StringSetPass`** → raikiri-gcpm 内部
- **`CounterPass`** → raikiri-css cascade + raikiri-gcpm 内部
- **`InjectCssPass`** → `ParseOptions::extra_stylesheets` に降格 (trait 不要)
- **`BookmarkPass`** → Consumer 側 post-cascade utility (raikiri は heading hint
  のみ提供)

### 5.3 API 面のサマリ

```rust
// fulgur (Consumer) の想定 use
use raikiri::{
    Document, RenderSink, ReplacedResolver, NetworkProvider,
    parse_html, render, PageDefaults, RenderMode, LookaheadConfig,
};

let options = ParseOptions {
    extra_stylesheets: &[fulgur_style],
    network: Some(&NoOpNetworkProvider),
    base_url: Some(base),
};
let doc = raikiri::parse_html(html_input, &options)?;
raikiri::render(
    &doc,
    PageDefaults::a4(),
    &FulgurResolver::new(font_data, images),
    RenderMode::Streaming { lookahead: Default::default() },
    &mut fulgur_pdf_sink,
)?;
```

### 5.4 並列化ポリシー

**Streaming の per-page 順序保証を破らない範囲で** rayon を per-page 内部で使用。

- **推奨並列化**:
  - 16 margin box slot layout (完全独立)
  - 同一ページ内の paragraph text shape (parley、`Sync` 確認要)
  - Multi-column の各 column (`column-count` > 1)
  - Consumer 側 replaced content decode (Consumer 責任)
- **Non-goal**: cross-page 並列 layout、speculative layout
- **決定論**: `IndexedParallelIterator` で順序保持、byte-identical output goal
  を守る

## 6. CSS Handling (raikiri-css)

### 6.1 統一 RuleTree

cssparser の 1 pass で全 stylesheet を統一 tree に集約:

```rust
pub struct RuleTree {
    style_rules: Vec<StyleRule>,           // 通常 selector + declarations
    page_rules: Vec<PageRule>,             // @page + margin boxes
    font_face_rules: Vec<FontFaceRule>,
    counter_style_rules: Vec<CounterStyleRule>,
    media_rules: Vec<MediaRule>,
    supports_rules: Vec<SupportsRule>,
    import_rules: Vec<ImportRule>,         // head 内のみ、body 内は無視
}
```

**重要**: stylo が非公開にしていた @page / counter / string-set / running / target-*
系の rule を **同一 tree の first-class citizen** として扱う。cascade 側と GCPM
解決側の 2 つの consumer が同じ tree を index する。

### 6.2 cascade

selectors crate に selector matching を委ね、cascade / specificity / inheritance
は自前:

```rust
pub struct CascadeResult {
    pub computed: Vec<Arc<ComputedValues>>,        // node_id -> computed style
    pub gcpm_directives: Vec<Arc<Vec<GcpmDirective>>>, // node_id -> directives
    pub running_templates: Vec<RunningTemplate>,   // position: running() された subtree
}
```

Selectors 対応:
- L3 base、interactive 系 (parse は通すが常に false、fail-open)
- L4 streaming-safe (`:is()`, `:where()`, `:not()`) 採用、優先度低
- L4 backward (`:has()`, `:nth-last-child()`, `:blank`) は Batch mode 専用

inline `<style>`:
- **MVP**: `<head>` 内のみ (body 内 `<style>` は無視)
- **後の拡張**: "出現位置以降のみ有効" (streaming と自然に合う)
- **Batch mode**: 全 `<style>` が document 全体で有効 (フル spec compliance)

## 7. GCPM IR 明示ノード化 (raikiri-gcpm)

### 7.1 GcpmDirective

**Producing directive** (cascade で生成、Phase B の walk で PageContext を更新):

```rust
pub enum GcpmDirective {
    CounterIncrement { name: Symbol, delta: i32 },
    CounterReset     { name: Symbol, value: i32 },
    CounterSet       { name: Symbol, value: i32 },
    StringSet { name: Symbol, source: ContentSource },
    RegisterRunning { name: Symbol, template_id: RunningTemplateId },
    RegisterTarget { fragment_id: Symbol },
}
```

**Consuming directive** (`content` プロパティの値、Phase B の paint 時に resolve):

```rust
pub enum ContentValueItem {
    Literal(String),
    Counter { name: Symbol, style: CounterStyle },
    Counters { name: Symbol, separator: String, style: CounterStyle },
    String { name: Symbol, fetch: StringFetchMode },
    Element { name: Symbol },
    Content { part: ContentPart },
    Attr { name: Symbol },
    TargetCounter { url: Url, name: Symbol, style: CounterStyle },
    TargetCounters { url: Url, name: Symbol, sep: String, style: CounterStyle },
    TargetText { url: Url, part: ContentPart },
}
```

### 7.2 PageContext

Phase B が所有する mutable page-local state:

```rust
pub struct PageContext {
    counters: HashMap<Symbol, CounterStack>,   // stack で nested scope 対応
    strings: HashMap<Symbol, NamedStringState>, // start/first/last/first-except の 4 snapshot
    running: HashMap<Symbol, RunningTemplateId>,
    targets: TargetRegistry,
    page_index: u32,
    page_name: Option<Symbol>,
}

pub struct NamedStringState {
    on_page_start: Option<String>,
    on_page_first_use: Option<String>,
    on_page_last_use: Option<String>,
    running: Option<String>,
}

pub struct TargetRegistry {
    resolved: HashMap<Symbol, TargetInfo>,
    pending_slots: Vec<TargetSlot>,
}
```

### 7.3 Running element templates

`position: running(name)` された subtree は body flow から除去、`RunningTemplate`
として保持。@page margin box の `content: element(name)` で参照。

- 静的 content のみのテンプレート: 1 回 layout してキャッシュ、per-page で座標
  変換のみ
- 動的 content (counter / string) を含む: pre-cascade で `is_content_dynamic`
  flag、per-page で subtree を再 layout

### 7.4 target-* の 2 モード

**SinglePass モード (Streaming デフォルト、streaming 保持)**

- 未解決の target-* に到達したら `TargetSlot` を pending_slots に登録
- `ResolvedContent::TargetSlot(slot_id)` として PageFragment に emit
- Consumer (fulgur → krilla) が全ページ emit 後に patch (Form XObject slot 方式)

**TwoPass モード (Batch mode で使用、streaming 犠牲)**

- Pass 1: 全ページ layout、TargetRegistry 構築
- Pass 2: registry を PageContext に埋め込んで通常の render

**NPassConverge モード (opt-in、target 値が layout に影響する場合)**

- 収束するまで反復。実装は後回し、`max_iterations: 5` 程度で妥協

## 8. Streaming vs Batch (RenderMode)

### 8.1 モード比較

| 仕様 | Streaming mode | Batch (TwoPass) mode |
|---|---|---|
| `target-counter/text/page` | placeholder slot、Consumer patch | ✓ 正確 |
| `counters(name, sep)` の nested scope | 現在 scope のみ | ✓ document 全体 |
| `:has()`, `:nth-last-child()` | 非対応 (fail-open) | ✓ 完全対応 |
| Sibling `~`, `+` の後方参照 | forward-only | ✓ 完全対応 |
| Body 内 inline `<style>` | 位置依存 or 非対応 | ✓ document 全体で有効 |
| Widow/orphan lookahead | N 行 buffer 制限 | ✓ 制限なし |
| Break-inside: avoid の大 subtree | pre-scan 制限 | ✓ 完全 lookahead |
| target-* が layout に影響する場合 | 収束保証なし | ✓ N-pass converge で保証 |
| Memory | O(DOM + window + 1 page) | O(DOM + all pages) |

### 8.2 mode 選択 API

```rust
pub enum RenderMode {
    Streaming { lookahead: LookaheadConfig },
    Batch {
        max_document_pages: Option<u32>,
        target_convergence: TargetConvergence,
    },
}
```

Consumer (fulgur) は用途で切り替え可能。**RenderSink API は mode 非依存**、
`accept_page` の逐次呼び出しは Streaming/Batch 両方で同じ shape。

### 8.3 fulgur の移行選択肢

- **既存の 2-pass + PDF Form XObject を維持**: raikiri は Streaming mode を使う、
  target-* placeholder を fulgur が PDF-level で patch
- **raikiri の Batch mode に移行**: fulgur の 2-pass ロジックを raikiri Batch に
  置換、`:has()` などフル spec compliance も自動

## 9. Per-page PageBox

PDF spec は per-page で `/MediaBox` を独立に持てる。CSS Paged Media は
`@page :first`, `@page :left`, `@page landscape-wide` などで per-page 変化を
サポート。

- `render()` は `PageDefaults` を受け取る (単一 PageBox ではない)
- `PageStream` 内部で per-page に @page rule 解決 + `page:` property + defaults
  を cascade
- `PageFragment.page_box` が実効ページ寸法を保持
- Consumer は `page_box.media` を `/MediaBox` に、`page_box.bleed` を
  `/BleedBox` に

```rust
pub struct PageBox {
    pub media: Size,
    pub content: Rect,
    pub margins: Margins,
    pub orientation: Orientation,
    pub bleed: Option<Rect>,
    pub crop_marks: bool,
    pub page_name: Option<String>,
    pub page_index: u32,
}
```

## 10. Resolver Design (sync)

`ReplacedResolver` は **同期 trait**。raikiri コアに async/tokio 依存を持たない。

```rust
pub trait ReplacedResolver {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<IntrinsicBox, ResolverError>;
}

pub struct ResolverRequest<'a> {
    pub node_id: NodeId,
    pub kind: ReplacedKind,
    pub attrs: &'a [Attr<'a>],
    pub base_url: Option<&'a Url>,
}
```

Consumer 責任で並列化・非同期・キャッシュを実装:

```rust
struct FulgurResolver {
    tokio_rt: tokio::runtime::Handle,
    image_cache: parking_lot::Mutex<HashMap<Url, Arc<Bytes>>>,
}

impl ReplacedResolver for FulgurResolver {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<IntrinsicBox, ResolverError> {
        // Consumer が tokio::block_on + rayon で並列 decode、等
    }
}
```

**副次効果**: attribute-level sanitize bypass 問題が自然消滅 (resolver は呼ばれた
瞬間に現在の attrs を参照するので、pre-sanitize の URL を hold しない)。

## 11. Paint Architecture (anyrender 採用)

### 11.1 `PageFragment` を共有 abstraction に

raikiri-paint は `anyrender::PaintScene` に push する walker のみ提供。**PDF backend
は anyrender に押し込まず、fulgur が `PageFragment` を直接 walk して krilla を叩く**。

```
        PageFragment (rich, 座標+style+content+GCPM metadata 全部)
                            │
              ┌─────────────┴─────────────┐
              ▼                            ▼
   raikiri-paint::paint_to_scene    fulgur が独自に walk
   (anyrender::PaintScene に push)  (krilla を直接叩く)
              │                            │
              ├─▶ VRT (vello_cpu → PNG)     │
              ├─▶ SVG debug output           │
              └─▶ record + replay             ▼
                                        PDF (Form XObject slot,
                                         Tagged PDF, annotation,
                                         bookmark, font subset)
```

### 11.2 `PageFragment` の中身

```rust
pub struct PageFragment {
    pub page_index: u32,
    pub page_box: PageBox,
    pub break_reason: BreakReason,
    pub body: BodyFragment,
    pub margin_boxes: [Option<MarginBoxFragment>; 16],
    
    // walker が使う metadata
    pub target_slots: Vec<TargetSlot>,
    pub bookmark_hints: Vec<BookmarkHint>,
    pub link_annotations: Vec<LinkAnnotation>,
    pub heading_structure: Vec<HeadingHint>,
    pub structural_hints: Vec<StructuralHint>,
}

pub struct BodyFragment { pub boxes: Vec<PaintedBox> }

pub struct PaintedBox {
    pub rect: Rect,
    pub transform: Option<Affine>,
    pub clip: Option<PathRepr>,
    pub content: BoxContent,
    pub link: Option<u32>,
    pub tag: Option<StructuralHint>,
    pub style: BoxStyle,
}
```

### 11.3 責務分割

| 責務 | 担当 |
|---|---|
| HTML parse、cascade、layout | raikiri |
| 座標つき PageFragment 生成 | raikiri-layout |
| raster / VRT 出力 | raikiri-paint + raikiri-vrt (anyrender) |
| PDF 出力 | **fulgur が独自 walker + krilla** |
| target-* placeholder 認識 | raikiri emit、Consumer 解決 |
| target-* Form XObject 生成 | fulgur |
| Font subsetting | fulgur (krilla feature) |
| Tagged PDF structure tree | fulgur (raikiri は hint 提供) |
| PDF annotation | fulgur |
| CMYK / ICC color | fulgur (raikiri は sRGB 前提) |

## 12. Testing Strategy

### 12.1 3 層構造

```
Layer 3: WPT reftest via VRT (E2E, M1 から primary)
  ├─ raikiri-wpt runner
  ├─ raikiri-vrt (anyrender_vello_cpu で ラスタライズ + PNG diff)
  └─ blitz baseline oracle (dev-dep として blitz-html/blitz-dom)

Layer 2: Integration test (crate 境界)
  ├─ raikiri (umbrella) の end-to-end fixture (raster)
  └─ Streaming / Batch mode-independence test

Layer 1: Unit test + optional structural dump
  ├─ 各 crate の unit test
  └─ Structural dump は debug aid + GCPM state unit test のみ
```

### 12.2 VRT primary from M1

raikiri-paint 早期整備により、M1 から raster reftest が primary。既存 raikiri の
structural dump は "raikiri-paint が無かったため" のワークアラウンドだった。new
raikiri では debug aid に降格。

### 12.3 決定論テスト

- Byte-identical output: 同じ入力を N 回 render、全て一致
- Cross-thread determinism: rayon 並列版が sequential 版と一致

### 12.4 Blitz baseline oracle

dev-dep として blitz を使い、"blitz pass = raikiri 必須 pass" を verify。
blitz が pass する WPT を raikiri が fail したら regression。

### 12.5 Mode-independence test

target-* / :has() / counters() 未使用のドキュメントでは Streaming と Batch の
出力が byte-identical であるべき。fast path (Streaming) と correct path (Batch)
の drift 早期検出。

### 12.6 CI 頻度

| テスト | 頻度 | 時間 | 遮断性 |
|---|---|---|---|
| Unit + Integration | PR ごと | 数秒〜数分 | 必須 |
| VRT reftest | PR ごと | 15〜30 分 | 必須 |
| Byte-identical | PR ごと | 数十秒 | 必須 |
| Mode-independence | PR ごと | 数分 | 必須 |
| WPT full sweep | nightly | 1〜2 時間 | 記録のみ |
| Blitz baseline oracle | nightly + weekly | 数時間 | regression detection |
| PDF reftest (via fulgur) | nightly (M6 以降) | 1〜2 時間 | 記録のみ |

## 13. Milestone Plan

### M1: skeleton + hello world + VRT primary

全 crate を skeleton として存在させ、end-to-end pipeline が最短経路で動くこと。

- 対象: `<html><body><p style="color:red">Hi</p></body></html>` → A4 単一
  ページ PNG → reference と VRT で一致
- 全 crate の Cargo.toml、trait 定義、最小型
- `raikiri::html_to_png()` の dogfooding helper 動作
- CI で unit test + VRT 実行
- **Non-goals**: pagination、@page、GCPM、Batch mode、ReplacedResolver 実使用、
  break policy、per-page PageBox、L4 selector

### M2: Streaming pagination + LayoutBuffer skeleton

複数ページ縦流し、break-before/break-after 基本。

### M3: Inline layout 本実装

parley 統合、BiDi、font selection、多国籍テキストの raster。

### M4: @page rule cascade + margin box + GCPM state 導入

@page 規則 cascade、margin box slot layout、PageContext 実装。

### M5: GCPM directive full + running element + Batch mode skeleton

counter-increment/reset/set、string-set 4-snapshot、running element template。

### M6: target-* SinglePass + Batch mode 完成 + fulgur integration + raikiri-blitz-compat

- target-counter/text/page の placeholder emit と Batch mode resolve
- fulgur adapter (raikiri を fulgur から使う adapter)
- **raikiri-blitz-compat crate 初版** (方針 B: 型 shape 互換、振る舞い raikiri)

### M7: Break policy 完成

widow/orphan、break-inside: avoid、break-before/after: page(name)。

### M8: 決定論保証 + per-page PageBox + WPT full sweep + PDF reftest 定常化

byte-identical CI 常時 verify、混合サイズ PDF、WPT 広範囲実行、fulgur PDF reftest
nightly。

### milestone 依存 DAG

```
M1 skeleton + VRT
     │
     ├─▶ M2 pagination ─▶ M3 inline ─▶ M4 @page + GCPM state
     │                                       │
     │                                       ▼
     │                                  M5 GCPM directive full
     │                                       │
     │                                       ▼
     │                                  M6 target-* + Batch + fulgur + blitz-compat
     │                                       │
     └───────────────────────────────────────▶ M7 break policy
                                             │
                                             ▼
                                        M8 決定論 + mixed-size + PDF
```

### サイズ感 (LoC 目安)

| Milestone | 実装量目安 | 難易度 |
|---|---|---|
| M1 skeleton | ~2000 行 | 中 |
| M2 pagination | ~1500 行 | 中 |
| M3 inline | ~3000 行 | 高 |
| M4 @page + GCPM 骨格 | ~2500 行 | 高 |
| M5 GCPM 完成 | ~3000 行 | 高 |
| M6 target-* + Batch + fulgur + compat | ~5000 行 (compat ~1000 含) | 極高 |
| M7 break policy | ~2500 行 | 高 |
| M8 決定論 + WPT | ~1500 行 + fixture | 中 |
| **合計** | ~21,000 行 (既存 raikiri 16k + 5k 増) | — |

### beads 構成

```
raikiri-spike-m1 (epic)
├─ workspace-setup (task)
├─ traits-definition (task)
├─ html-parse-basic (task)
├─ css-cascade-basic (task)
├─ dom-model (task)
├─ layout-single-page (task)
├─ paint-basic (task)
├─ vrt-tiny-skia (task)
├─ wpt-harness-skeleton (task)
├─ umbrella-facade (task)
├─ ci-setup (task)
├─ determinism-test (task)
└─ hello-world-vrt (task, 上記全てに依存)
```

M2〜M8 も同様に epic + task 分解、`bd dep add` で依存 DAG 明示。

## 14. Consumer Contract (fulgur 視点)

### 14.1 依存 crate

fulgur は `raikiri` umbrella crate のみに依存 (Style B):

```toml
[dependencies]
raikiri = "0.1"
```

sub-crate 直接依存も可能 (advanced):

```toml
[dependencies]
raikiri = "0.1"
raikiri-html = "0.1"  # 例: TreeSink 独自 wrap のため
raikiri-traits = "0.1"
```

### 14.2 fulgur 側の実装フロー

```rust
use raikiri::{
    parse_html, render, PageDefaults, RenderMode, LookaheadConfig,
    RenderSink, ReplacedResolver, NetworkProvider,
    ParseOptions, Document,
};

// 1. HTML parse (template engine で展開後の HTML)
let doc = parse_html(html_input, &ParseOptions {
    extra_stylesheets: &fulgur_stylesheets,
    network: Some(&NoOpNetworkProvider),
    base_url: Some(base),
})?;

// 2. render で PageFragment を per-page で受け取る
render(
    &doc,
    PageDefaults::from_cli(cli_size, cli_orientation),
    &FulgurResolver::new(font_data, images),
    match doc_size {
        n if n < 100 => RenderMode::Batch { max_document_pages: Some(n), 
                        target_convergence: TwoPass },
        _ => RenderMode::Streaming { lookahead: Default::default() },
    },
    &mut FulgurPdfSink::new(krilla_doc),
)?;

// 3. FulgurPdfSink が PageFragment を walk して krilla に落とす
struct FulgurPdfSink { krilla_doc: krilla::Document, /* ... */ }
impl RenderSink for FulgurPdfSink {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        // krilla API を direct に叩く
        // page.target_slots を Form XObject slot として書く
        // page.heading_structure を outline tree に反映
        // ...
    }
    fn finalize(self: Box<Self>) -> std::io::Result<()> {
        // font subset、trailer、target-* patch
    }
}
```

### 14.3 migration path

Step 1: 現状 (blitz 直接依存)
Step 2: `raikiri-blitz-compat` 経由 (import path 差し替えで raikiri 化)
Step 3: `raikiri` direct に段階的移行 (DomPass 削減)
Step 4: 完全 raikiri 化 (blitz_adapter.rs 削除)

## 15. Open Questions / Future Work

- `parley` の `FontContext` の `Sync` 適合性 (rayon 並列 shape の前提)
- `taffy` の並列 layout 対応可否 (multi-column の各 column 並列)
- 縦書き / ルビ / JIS X 4051 相当の日本語組版拡張タイミング
- `SandboxedNetworkProvider` の spec 詳細 (URL allowlist、size cap、MIME
  whitelist の具体的な interface)
- `NPassConverge` の収束条件の詳細 (target-* の layout influence 判定)
- `raikiri-blitz-compat` の sunset タイミング (fulgur 完全 migration 後)
- 将来的な `raikiri-paint-pdf` (anyrender::PaintScene 実装、krilla base) の
  raikiri 側追加 vs. fulgur 側維持
- WASM 対応 (`raikiri-vrt` は wasm でも動くべきか)

## 16. 用語集

- **Consumer**: raikiri を使う側 (fulgur、将来の EPUB reader、他)
- **RenderSink**: Consumer が実装する trait、`PageFragment` を受け取る
- **PageStream**: crate-private state machine、per-page `PageFragment` を emit
- **PageFragment**: 1 ページの layout 完了済み representation (座標 + style +
  content + GCPM metadata)
- **PageContext**: Phase B が所有する mutable page-local state (counter tree +
  string 4-snapshot + running + target)
- **PageBox**: per-page の物理寸法 + margin + orientation
- **GcpmDirective**: cascade で生成される GCPM 命令 (Producing / Consuming)
- **RuleTree**: cssparser で 1 pass parse された統一 rule 表現 (通常 rule +
  GCPM at-rule)
- **RaikiriTreeSink**: html5ever `TreeSink` 実装、Consumer が wrap 可能
- **LayoutBuffer**: widow/orphan/break-avoid lookahead の独立ユニット
- **TargetRegistry**: target-* の fragment_id → 実効ページ情報の map
- **TargetSlot**: target-* placeholder、SinglePass モードで PageFragment に emit
- **RunningTemplate**: `position: running(name)` された subtree の template 保持
